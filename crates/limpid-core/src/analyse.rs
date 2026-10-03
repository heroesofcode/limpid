//! Where the space actually went.
//!
//! Separate from the catalogue, and asking a different question. The
//! catalogue knows what a directory *means* — this is a cache, that is a
//! package archive — and can only find what it was told to look for. This
//! knows nothing and finds everything, which is what you need when the space
//! went somewhere nobody thought to write a scanner for.

use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::config::Exclusions;
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
    /// Everything directly inside it, largest first. A file sitting directly
    /// in the root is a child like any directory, so this accounts for all
    /// of it.
    pub children: Vec<Entry>,
    /// Paths that could not be read.
    pub unreadable: u64,
}

impl Breakdown {
    /// Everything this level accounts for.
    pub fn total(&self) -> Size {
        self.children.iter().map(|child| child.size).sum()
    }

    /// Whether there is anything here.
    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
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
    //
    // Exclusions are deliberately *not* applied here. This is accounting,
    // and a directory total that silently leaves out what the user excluded
    // would make the treemap lie about where the space is. Exclusions stop
    // Limpid *offering* things — see `largest_files` — not counting them.
    let per_child = WalkOptions {
        threads: Some(1),
        skip: Exclusions::default(),
        ..options.clone()
    };

    let measured: Vec<(Option<Entry>, u64)> = entries
        .par_iter()
        .map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();

            let Ok(metadata) = entry.metadata() else {
                return (None, 1);
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
                        usage.unreadable,
                    ),
                    Err(_) => (None, 1),
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
                    0,
                )
            } else {
                (None, 0)
            }
        })
        .collect();

    let mut breakdown = Breakdown {
        root: root.to_owned(),
        ..Breakdown::default()
    };
    for (entry, unreadable) in measured {
        if let Some(entry) = entry {
            breakdown.children.push(entry);
        }
        breakdown.unreadable += unreadable;
    }

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

    // Unlike `breakdown`, this list honours exclusions. It is a list of
    // things worth acting on, and the point of excluding a file is to stop
    // being shown it — which also leaves room in the list for the next one.
    let skip = options.skip.clone();

    let mut builder = ignore::WalkBuilder::new(root);
    builder
        .standard_filters(false)
        .hidden(false)
        .follow_links(false)
        .same_file_system(options.same_filesystem)
        .filter_entry(move |entry| {
            let snapshots = entry
                .file_name()
                .to_str()
                .is_some_and(|name| name == ".snapshots");
            !snapshots && !skip.covers(entry.path())
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
    /// The largest individual files anywhere under it, the trash's left out.
    pub largest: Vec<Entry>,
    /// What is sitting in the trash: space that looks freed and is not.
    ///
    /// Measured on every survey, wherever it is taken from, because it is the
    /// answer to "I removed it and nothing changed".
    pub trash: walk::Usage,
}

/// Answer both halves of "where did the space go" for one directory.
///
/// `trash` is what emptying the trash removes — the paths of
/// [`crate::catalog::trash::target`], passed in so that what is measured
/// here and what the button empties are the same paths.
///
/// The trash is left out of `largest`. Something moved there is still on the
/// disk, so it would come straight back as the largest file under its new
/// name, as if moving it had done nothing; and a file in the trash is not a
/// candidate to act on one by one — what it waits for is the trash being
/// emptied. So it is measured on its own instead, and the breakdown still
/// counts it: that is accounting, and leaving it out would make the treemap
/// lie about where the space is.
///
/// The walks run side by side rather than one after the other. They read the
/// same directories, so each is largely served from the page cache the others
/// warmed, and the wall time is close to that of one.
pub fn survey(
    root: &Path,
    files: usize,
    options: &WalkOptions,
    trash: &[PathBuf],
) -> std::io::Result<Survey> {
    let mut listing = options.clone();
    for path in trash {
        listing.skip.add(path.clone());
    }

    // The trash is measured with the person's exclusions, unlike the
    // breakdown: the figure is "what emptying it would remove", and the
    // executor leaves anything excluded in there alone.
    let (breakdown, (largest, trashed)) = rayon::join(
        || breakdown(root, options),
        || {
            rayon::join(
                || largest_files(root, files, &listing),
                || walk::measure_all(trash, options),
            )
        },
    );

    Ok(Survey {
        breakdown: breakdown?,
        largest: largest.unwrap_or_default(),
        trash: trashed.unwrap_or_default(),
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

        let survey = survey(root.path(), 2, &WalkOptions::default(), &[]).unwrap();

        assert_eq!(survey.breakdown.children.len(), 3);
        assert_eq!(survey.largest.len(), 2);
    }

    /// A home with one large file just moved to the trash, the way the trash
    /// crate does it: the file under `Trash/files`, its record under
    /// `Trash/info`.
    fn home_with_something_trashed() -> (tempfile::TempDir, Vec<PathBuf>) {
        let home = tempfile::tempdir().unwrap();
        let trash = home.path().join(".local/share/Trash");
        write(&trash.join("files/ubuntu.iso"), 40_000);
        write(&trash.join("info/ubuntu.iso.trashinfo"), 100);
        write(&home.path().join("Documents/notes.pdf"), 2_000);
        let paths = vec![trash.join("files"), trash.join("info")];
        (home, paths)
    }

    #[test]
    fn a_file_moved_to_the_trash_does_not_come_back_as_the_largest() {
        // What it did: trash the largest file, measure again, and find the
        // same file at the top under its trash path, as if nothing had
        // happened.
        let (home, trash) = home_with_something_trashed();

        let survey = survey(home.path(), 5, &WalkOptions::default(), &trash).unwrap();

        let names: Vec<&str> = survey.largest.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["notes.pdf"]);
    }

    #[test]
    fn the_trash_is_measured_on_its_own_with_its_records() {
        let (home, trash) = home_with_something_trashed();

        let survey = survey(home.path(), 5, &WalkOptions::default(), &trash).unwrap();

        // Both halves, as emptying removes both.
        assert_eq!(survey.trash.size.apparent, 40_100);
        assert_eq!(survey.trash.files, 2);
    }

    #[test]
    fn the_treemap_still_counts_what_is_in_the_trash() {
        // It is still on the disk. A breakdown that left it out would show
        // a home smaller than the one the disk is holding.
        let (home, trash) = home_with_something_trashed();

        let survey = survey(home.path(), 5, &WalkOptions::default(), &trash).unwrap();

        assert_eq!(survey.breakdown.total().apparent, 42_100);
    }

    #[test]
    fn a_missing_trash_measures_to_nothing() {
        let root = tree();
        let trash = vec![
            root.path().join(".local/share/Trash/files"),
            root.path().join(".local/share/Trash/info"),
        ];

        let survey = survey(root.path(), 5, &WalkOptions::default(), &trash).unwrap();

        assert!(survey.trash.is_empty());
        assert_eq!(survey.largest.len(), 4);
    }

    #[test]
    fn an_excluded_file_is_left_out_of_the_largest_files() {
        let root = tempfile::tempdir().unwrap();
        write(&root.path().join("keep/game.iso"), 500_000);
        write(&root.path().join("other.bin"), 90_000);
        let options = WalkOptions {
            skip: Exclusions::new([root.path().join("keep/game.iso")]),
            ..WalkOptions::default()
        };

        let found = largest_files(root.path(), 5, &options).unwrap();

        // The one the user asked to stop seeing is gone, and the next one
        // moves up rather than the list coming back short.
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "other.bin");
    }

    #[test]
    fn a_breakdown_still_counts_what_is_excluded() {
        let root = tempfile::tempdir().unwrap();
        write(&root.path().join("keep/game.iso"), 500_000);
        let options = WalkOptions {
            skip: Exclusions::new([root.path().join("keep/game.iso")]),
            ..WalkOptions::default()
        };

        let breakdown = breakdown(root.path(), &options).unwrap();

        // Excluding it stops Limpid offering it, not counting it. Otherwise
        // the treemap would lie about where the space is.
        assert_eq!(breakdown.total().apparent, 500_000);
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
