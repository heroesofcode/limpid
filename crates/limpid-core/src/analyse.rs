//! Where the space actually went.
//!
//! Separate from the catalogue, and asking a different question. The
//! catalogue knows what a directory *means* — this is a cache, that is a
//! package archive — and can only find what it was told to look for. This
//! knows nothing and finds everything, which is what you need when the space
//! went somewhere nobody thought to write a scanner for.

use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::size::Size;
use crate::walk::{self, WalkOptions};

/// One thing taking up space.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Entry {
    /// Where it is.
    pub path: PathBuf,
    /// The last component, which is what gets shown.
    pub name: String,
    /// How much it takes.
    pub size: Size,
    /// How many files are under it. One, for a file.
    pub files: u64,
    /// Whether it can be descended into.
    pub is_dir: bool,
}

impl Entry {
    /// The share of `total` this accounts for, 0.0 to 1.0.
    pub fn share_of(&self, total: u64) -> f32 {
        if total == 0 {
            0.0
        } else {
            self.size.on_disk as f32 / total as f32
        }
    }
}

/// One level of a directory, with each child measured.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Breakdown {
    /// The directory this describes.
    pub root: PathBuf,
    /// Everything directly inside it, largest first.
    pub children: Vec<Entry>,
    /// Bytes in files sitting directly in the root rather than in a child.
    pub loose: Size,
    /// Paths that could not be read.
    pub unreadable: u64,
}

impl Breakdown {
    /// Everything this level accounts for.
    pub fn total(&self) -> Size {
        self.children.iter().map(|child| child.size).sum::<Size>() + self.loose
    }

    /// Whether there is anything here.
    pub fn is_empty(&self) -> bool {
        self.children.is_empty() && self.loose.is_zero()
    }
}

/// Measure every immediate child of `root`.
///
/// The children are measured in parallel, each with its own recursive walk.
/// One level at a time rather than the whole tree at once: a treemap only
/// ever draws one level, and measuring what is not shown is time the user
/// spends waiting for nothing.
pub fn breakdown(root: &Path, options: &WalkOptions) -> std::io::Result<Breakdown> {
    let entries: Vec<_> = std::fs::read_dir(root)?.flatten().collect();

    // Each child gets its own walk, so the walker's own thread pool would
    // nest inside rayon's. One thread per child walk keeps the total near
    // the core count instead of squaring it.
    let per_child = WalkOptions {
        threads: Some(1),
        ..options.clone()
    };

    let measured: Vec<(Option<Entry>, Size, u64)> = entries
        .par_iter()
        .map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();

            let Ok(metadata) = entry.metadata() else {
                return (None, Size::ZERO, 1);
            };

            if metadata.is_dir() {
                match walk::measure(&path, &per_child) {
                    Ok(usage) => (
                        Some(Entry {
                            path,
                            name,
                            size: usage.size,
                            files: usage.files,
                            is_dir: true,
                        }),
                        Size::ZERO,
                        usage.unreadable,
                    ),
                    Err(_) => (None, Size::ZERO, 1),
                }
            } else if metadata.is_file() {
                // A large file sitting directly in the directory is a
                // finding in its own right, not an anonymous remainder.
                let size = Size::of(&metadata);
                (
                    Some(Entry {
                        path,
                        name,
                        size,
                        files: 1,
                        is_dir: false,
                    }),
                    size,
                    0,
                )
            } else {
                (None, Size::ZERO, 0)
            }
        })
        .collect();

    let mut breakdown = Breakdown {
        root: root.to_owned(),
        ..Breakdown::default()
    };
    for (entry, loose, unreadable) in measured {
        if let Some(entry) = entry {
            breakdown.children.push(entry);
        }
        breakdown.loose += loose;
        breakdown.unreadable += unreadable;
    }

    // The loose total double-counted the files that also became children;
    // they are children, so take it back out.
    breakdown.loose = Size::ZERO;

    breakdown.children.sort_by(|a, b| {
        b.size
            .on_disk
            .cmp(&a.size.on_disk)
            .then_with(|| a.name.cmp(&b.name))
    });

    Ok(breakdown)
}

/// The `limit` largest files anywhere under `root`.
///
/// Answers the other half of "where did it go": not which directory is
/// heavy, but which single file is.
pub fn largest_files(
    root: &Path,
    limit: usize,
    options: &WalkOptions,
) -> std::io::Result<Vec<Entry>> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    if !root.exists() {
        return Ok(Vec::new());
    }

    // A bounded min-heap rather than sorting everything: a home directory
    // holds hundreds of thousands of files and only the top handful matter.
    let heap: std::sync::Mutex<BinaryHeap<Reverse<(u64, PathBuf)>>> =
        std::sync::Mutex::new(BinaryHeap::new());

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
                .is_some_and(|name| name == ".snapshots")
        });
    if let Some(threads) = options.threads {
        builder.threads(threads);
    }

    builder.build_parallel().run(|| {
        Box::new(|result| {
            let Ok(entry) = result else {
                return ignore::WalkState::Continue;
            };
            let Ok(metadata) = entry.metadata() else {
                return ignore::WalkState::Continue;
            };
            if !metadata.is_file() {
                return ignore::WalkState::Continue;
            }

            use std::os::unix::fs::MetadataExt;
            let bytes = metadata.blocks() * 512;

            let mut heap = heap.lock().expect("heap is only locked to push and pop");
            heap.push(Reverse((bytes, entry.into_path())));
            if heap.len() > limit {
                heap.pop();
            }

            ignore::WalkState::Continue
        })
    });

    let heap = heap.into_inner().expect("walk threads have all finished");
    let mut found: Vec<Entry> = heap
        .into_iter()
        .map(|Reverse((bytes, path))| Entry {
            name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            // The apparent size is not carried: for a "largest files" list
            // the occupied size is the one being ranked, and re-statting
            // every candidate to get the other would cost more than it says.
            size: Size::new(bytes, bytes),
            files: 1,
            is_dir: false,
            path,
        })
        .collect();
    // Block rounding makes small files tie on occupied size, so the path
    // breaks the tie and the list is the same on every run.
    found.sort_by(|a, b| {
        b.size
            .on_disk
            .cmp(&a.size.on_disk)
            .then_with(|| a.path.cmp(&b.path))
    });

    Ok(found)
}

/// One level's breakdown and the largest files beneath it.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Survey {
    /// What is directly inside the directory.
    pub breakdown: Breakdown,
    /// The largest individual files anywhere under it.
    pub largest: Vec<Entry>,
}

/// Answer both halves of "where did the space go" for one directory.
///
/// The two walks run side by side rather than one after the other. They read
/// the same directories, so the second is largely served from the page cache
/// the first warmed, and the wall time is close to that of one.
pub fn survey(root: &Path, files: usize, options: &WalkOptions) -> std::io::Result<Survey> {
    let (breakdown, largest) = rayon::join(
        || breakdown(root, options),
        || largest_files(root, files, options),
    );

    Ok(Survey {
        breakdown: breakdown?,
        largest: largest.unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    fn tree() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        write(&root.path().join("big/one.bin"), 5000);
        write(&root.path().join("big/deeper/two.bin"), 3000);
        write(&root.path().join("small/three.bin"), 100);
        write(&root.path().join("loose.bin"), 900);
        root
    }

    #[test]
    fn a_level_is_measured_with_each_child_totalled_recursively() {
        let root = tree();

        let breakdown = breakdown(root.path(), &WalkOptions::default()).unwrap();

        assert_eq!(breakdown.children.len(), 3);
        assert_eq!(breakdown.children[0].name, "big");
        assert_eq!(breakdown.children[0].size.apparent, 8000);
        assert_eq!(breakdown.children[0].files, 2);
        assert!(breakdown.children[0].is_dir);
    }

    #[test]
    fn children_come_back_largest_first() {
        let root = tree();

        let breakdown = breakdown(root.path(), &WalkOptions::default()).unwrap();
        let names: Vec<&str> = breakdown
            .children
            .iter()
            .map(|child| child.name.as_str())
            .collect();

        assert_eq!(names, ["big", "loose.bin", "small"]);
    }

    #[test]
    fn a_loose_file_is_a_child_in_its_own_right() {
        let root = tree();

        let breakdown = breakdown(root.path(), &WalkOptions::default()).unwrap();
        let loose = breakdown
            .children
            .iter()
            .find(|child| child.name == "loose.bin")
            .unwrap();

        assert!(!loose.is_dir);
        assert_eq!(loose.size.apparent, 900);
        // And it is counted once, not once as a child and again as a
        // remainder.
        assert_eq!(breakdown.total().apparent, 9000);
    }

    #[test]
    fn a_share_of_nothing_is_nothing_rather_than_a_nan() {
        let entry = Entry {
            path: "/x".into(),
            name: "x".into(),
            size: Size::ZERO,
            files: 0,
            is_dir: false,
        };

        assert_eq!(entry.share_of(0), 0.0);
    }

    #[test]
    fn the_largest_files_are_ranked_across_the_whole_tree() {
        // Sizes a block apart, because ranking is by occupied size and
        // anything under one block ties with everything else under one.
        let root = tempfile::tempdir().unwrap();
        write(&root.path().join("a/huge.bin"), 200_000);
        write(&root.path().join("a/b/middling.bin"), 90_000);
        write(&root.path().join("tiny.bin"), 100);

        let found = largest_files(root.path(), 2, &WalkOptions::default()).unwrap();

        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "huge.bin");
        assert_eq!(found[1].name, "middling.bin");
    }

    #[test]
    fn asking_for_more_files_than_exist_returns_what_there_is() {
        let root = tree();

        let found = largest_files(root.path(), 100, &WalkOptions::default()).unwrap();

        assert_eq!(found.len(), 4);
    }

    #[test]
    fn a_missing_root_yields_nothing_rather_than_an_error() {
        let missing = Path::new("/definitely/not/here");

        assert!(
            largest_files(missing, 5, &WalkOptions::default())
                .unwrap()
                .is_empty()
        );
        assert!(breakdown(missing, &WalkOptions::default()).is_err());
    }

    #[test]
    fn a_survey_answers_both_halves_at_once() {
        let root = tree();

        let survey = survey(root.path(), 2, &WalkOptions::default()).unwrap();

        assert_eq!(survey.breakdown.children.len(), 3);
        assert_eq!(survey.largest.len(), 2);
    }

    #[test]
    fn snapshots_are_skipped_when_ranking_files_too() {
        let root = tempfile::tempdir().unwrap();
        write(&root.path().join("live.bin"), 100);
        write(&root.path().join(".snapshots/1/snapshot/huge.bin"), 900_000);

        let found = largest_files(root.path(), 5, &WalkOptions::default()).unwrap();

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "live.bin");
    }
}
