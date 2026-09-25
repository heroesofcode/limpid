//! Parallel directory measurement.
//!
//! Built on `ignore::WalkBuilder` — ripgrep's walker — rather than `jwalk`,
//! which was archived in August 2026. All of ignore's filtering is switched
//! off: a cleaner that respected `.gitignore` would silently skip the very
//! build artefacts it exists to find.

use std::collections::HashSet;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::size::Size;

/// Directory names that are never descended into, wherever they appear.
///
/// `.snapshots` is the important one: btrfs snapshots share extents with the
/// live tree, so walking into them counts the same bytes many times over and
/// inflates the total into nonsense.
const NEVER_DESCEND: &[&str] = &[".snapshots", ".zfs"];

/// How to measure a directory.
#[derive(Debug, Clone)]
pub struct WalkOptions {
    /// Stay on the filesystem the root lives on.
    ///
    /// On by default. Omarchy puts `/var/log` and `/var/cache/pacman/pkg` on
    /// their own btrfs subvolumes, so crossing boundaries by accident is easy.
    pub same_filesystem: bool,
    /// Worker threads. `None` lets the walker choose from the CPU count.
    pub threads: Option<usize>,
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self {
            same_filesystem: true,
            threads: None,
        }
    }
}

/// What a walk found.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct Usage {
    /// Total size, with each inode counted once.
    pub size: Size,
    /// Number of regular files counted.
    pub files: u64,
    /// Number of directories visited.
    pub directories: u64,
    /// Entries skipped because they were a second link to a counted inode.
    pub hardlinks_skipped: u64,
    /// Paths that could not be read, usually for want of permission.
    pub unreadable: u64,
}

impl Usage {
    /// Whether anything at all was counted.
    pub fn is_empty(&self) -> bool {
        self.files == 0 && self.size.is_zero()
    }
}

/// Measure `root` recursively.
///
/// Missing roots are not an error — most of what Limpid looks for is absent on
/// any given machine — and come back as an empty [`Usage`]. Symlinks are
/// counted as links, never followed, so a symlink loop cannot hang the walk
/// and a link out of the tree cannot drag unrelated bytes in.
pub fn measure(root: &Path, options: &WalkOptions) -> io::Result<Usage> {
    if !root.exists() {
        return Ok(Usage::default());
    }

    let root_device = std::fs::symlink_metadata(root)?.dev();

    let apparent = AtomicU64::new(0);
    let on_disk = AtomicU64::new(0);
    let files = AtomicU64::new(0);
    let directories = AtomicU64::new(0);
    let hardlinks_skipped = AtomicU64::new(0);
    let unreadable = AtomicU64::new(0);

    // Only files with more than one link ever touch this, so the lock is
    // uncontended on the overwhelming majority of entries.
    let seen_inodes: Mutex<HashSet<(u64, u64)>> = Mutex::new(HashSet::new());

    let mut builder = ignore::WalkBuilder::new(root);
    builder
        .standard_filters(false)
        .hidden(false)
        .follow_links(false)
        .same_file_system(options.same_filesystem)
        .filter_entry(|entry| {
            !entry
                .file_name()
                .to_str()
                .is_some_and(|name| NEVER_DESCEND.contains(&name))
        });
    if let Some(threads) = options.threads {
        builder.threads(threads);
    }

    builder.build_parallel().run(|| {
        Box::new(|result| {
            let entry = match result {
                Ok(entry) => entry,
                Err(_) => {
                    unreadable.fetch_add(1, Ordering::Relaxed);
                    return ignore::WalkState::Continue;
                }
            };

            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(_) => {
                    unreadable.fetch_add(1, Ordering::Relaxed);
                    return ignore::WalkState::Continue;
                }
            };

            // `same_file_system` governs descent, but the root's own device
            // still has to be checked against mount points that appear as
            // plain entries.
            if options.same_filesystem && metadata.dev() != root_device {
                return ignore::WalkState::Skip;
            }

            if metadata.is_dir() {
                directories.fetch_add(1, Ordering::Relaxed);
                return ignore::WalkState::Continue;
            }

            // Symlinks, sockets, fifos and device nodes occupy no data blocks
            // worth reclaiming, and a symlink's reported length is just the
            // length of the path it holds. Counting them is noise.
            if !metadata.is_file() {
                return ignore::WalkState::Continue;
            }

            // A hardlinked file occupies its blocks once however many names
            // point at it, so only the first name encountered may count them.
            if metadata.nlink() > 1 {
                let inode = (metadata.dev(), metadata.ino());
                let first_sighting = seen_inodes
                    .lock()
                    .expect("inode set is only ever locked for an insert")
                    .insert(inode);
                if !first_sighting {
                    hardlinks_skipped.fetch_add(1, Ordering::Relaxed);
                    return ignore::WalkState::Continue;
                }
            }

            let size = Size::of(&metadata);
            apparent.fetch_add(size.apparent, Ordering::Relaxed);
            on_disk.fetch_add(size.on_disk, Ordering::Relaxed);
            files.fetch_add(1, Ordering::Relaxed);

            ignore::WalkState::Continue
        })
    });

    Ok(Usage {
        size: Size::new(apparent.into_inner(), on_disk.into_inner()),
        files: files.into_inner(),
        // The root itself is visited as a directory; it is not a finding.
        directories: directories.into_inner().saturating_sub(1),
        hardlinks_skipped: hardlinks_skipped.into_inner(),
        unreadable: unreadable.into_inner(),
    })
}

/// Measure several roots and sum them.
///
/// Roots that do not exist contribute nothing, which is the common case.
pub fn measure_all(roots: &[PathBuf], options: &WalkOptions) -> io::Result<Usage> {
    let mut total = Usage::default();
    for root in roots {
        let usage = measure(root, options)?;
        total.size += usage.size;
        total.files += usage.files;
        total.directories += usage.directories;
        total.hardlinks_skipped += usage.hardlinks_skipped;
        total.unreadable += usage.unreadable;
    }
    Ok(total)
}

/// Collect every file under `root` whose name ends in one of `suffixes`.
///
/// Used for findings that are counted rather than measured — `.pacnew` files
/// are a handful of kilobytes but represent configuration merges the user
/// still owes the system.
pub fn find_by_suffix(root: &Path, suffixes: &[&str], options: &WalkOptions) -> Vec<PathBuf> {
    if !root.exists() {
        return Vec::new();
    }

    let found = Mutex::new(Vec::new());

    let mut builder = ignore::WalkBuilder::new(root);
    builder
        .standard_filters(false)
        .hidden(false)
        .follow_links(false)
        .same_file_system(options.same_filesystem);
    if let Some(threads) = options.threads {
        builder.threads(threads);
    }

    builder.build_parallel().run(|| {
        Box::new(|result| {
            let Ok(entry) = result else {
                return ignore::WalkState::Continue;
            };
            let Some(name) = entry.file_name().to_str() else {
                return ignore::WalkState::Continue;
            };
            if suffixes.iter().any(|suffix| name.ends_with(suffix)) {
                found
                    .lock()
                    .expect("match list is only ever locked for a push")
                    .push(entry.into_path());
            }
            ignore::WalkState::Continue
        })
    });

    let mut found = found.into_inner().expect("walk threads have all finished");
    // Parallel traversal gives no useful order; sorting makes output stable
    // between runs, which matters for both tests and diffable reports.
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, bytes: usize) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    #[test]
    fn a_missing_root_is_empty_rather_than_an_error() {
        let usage = measure(Path::new("/definitely/not/here"), &WalkOptions::default()).unwrap();
        assert!(usage.is_empty());
        assert_eq!(usage.files, 0);
    }

    #[test]
    fn files_are_counted_recursively() {
        let root = tempfile::tempdir().unwrap();
        write(&root.path().join("a.txt"), 1000);
        write(&root.path().join("nested/b.txt"), 2000);
        write(&root.path().join("nested/deeper/c.txt"), 3000);

        let usage = measure(root.path(), &WalkOptions::default()).unwrap();

        assert_eq!(usage.files, 3);
        assert_eq!(usage.directories, 2);
        assert_eq!(usage.size.apparent, 6000);
    }

    #[test]
    fn a_hardlinked_inode_is_counted_once() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("original");
        write(&original, 4096);
        fs::hard_link(&original, root.path().join("second-name")).unwrap();
        fs::hard_link(&original, root.path().join("third-name")).unwrap();

        let usage = measure(root.path(), &WalkOptions::default()).unwrap();

        assert_eq!(usage.files, 1);
        assert_eq!(usage.hardlinks_skipped, 2);
        assert_eq!(usage.size.apparent, 4096);
    }

    #[test]
    fn snapshot_directories_are_never_descended() {
        let root = tempfile::tempdir().unwrap();
        write(&root.path().join("live.txt"), 100);
        write(&root.path().join(".snapshots/1/snapshot/live.txt"), 100);

        let usage = measure(root.path(), &WalkOptions::default()).unwrap();

        assert_eq!(usage.files, 1);
        assert_eq!(usage.size.apparent, 100);
    }

    #[test]
    fn symlinks_are_not_followed_out_of_the_tree() {
        let outside = tempfile::tempdir().unwrap();
        write(&outside.path().join("big.bin"), 100_000);

        let root = tempfile::tempdir().unwrap();
        write(&root.path().join("small.txt"), 10);
        std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();

        let usage = measure(root.path(), &WalkOptions::default()).unwrap();

        assert_eq!(usage.files, 1);
        assert_eq!(usage.size.apparent, 10);
    }

    #[test]
    fn hidden_files_are_counted() {
        let root = tempfile::tempdir().unwrap();
        write(&root.path().join(".hidden"), 512);

        let usage = measure(root.path(), &WalkOptions::default()).unwrap();

        assert_eq!(usage.files, 1);
    }

    #[test]
    fn gitignored_files_are_still_counted() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(".gitignore"), "target/\n").unwrap();
        write(&root.path().join("target/debug/artifact.bin"), 8192);

        let usage = measure(root.path(), &WalkOptions::default()).unwrap();

        // The whole point: build output is the target, not something to skip.
        assert!(usage.size.apparent >= 8192);
    }

    #[test]
    fn files_can_be_found_by_suffix_in_a_stable_order() {
        let root = tempfile::tempdir().unwrap();
        write(&root.path().join("etc/pacman.conf"), 10);
        write(&root.path().join("etc/pacman.conf.pacnew"), 10);
        write(&root.path().join("etc/ssh/sshd_config.pacsave"), 10);
        write(&root.path().join("etc/fstab"), 10);

        let found = find_by_suffix(
            root.path(),
            &[".pacnew", ".pacsave"],
            &WalkOptions::default(),
        );

        assert_eq!(found.len(), 2);
        assert!(found[0].ends_with("etc/pacman.conf.pacnew"));
        assert!(found[1].ends_with("etc/ssh/sshd_config.pacsave"));
    }

    #[test]
    fn finding_by_suffix_in_a_missing_root_is_empty() {
        let found = find_by_suffix(Path::new("/nope"), &[".pacnew"], &WalkOptions::default());
        assert!(found.is_empty());
    }

    #[test]
    fn several_roots_sum_together() {
        let first = tempfile::tempdir().unwrap();
        write(&first.path().join("a"), 100);
        let second = tempfile::tempdir().unwrap();
        write(&second.path().join("b"), 200);

        let roots = vec![
            first.path().to_path_buf(),
            second.path().to_path_buf(),
            "/not/here".into(),
        ];
        let usage = measure_all(&roots, &WalkOptions::default()).unwrap();

        assert_eq!(usage.files, 2);
        assert_eq!(usage.size.apparent, 300);
    }
}
