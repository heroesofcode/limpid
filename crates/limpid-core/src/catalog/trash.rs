//! The freedesktop trash.
//!
//! Emptying the trash is the one action where the user has already said yes
//! once. That makes it low-risk but never automatic: it is also the only
//! undo they have for everything else they deleted this month.

use super::Context;
use crate::model::{Category, Kind, Risk, Target};
use crate::paths::Roots;

/// The home trash, unmeasured.
///
/// The one definition of what emptying the trash removes. The overview
/// measures it through [`scan`]; the storage view measures the same paths to
/// say how much is sitting there, and empties it through this same target,
/// so the figure shown and what goes cannot drift apart.
///
/// Per-mount trash directories (`$topdir/.Trash-$uid`) are not covered yet;
/// finding them means walking the mount table, which belongs with the
/// executor that would have to empty them coherently.
pub fn target(roots: &Roots) -> Target {
    Target::new("Home trash", Kind::Trash, Risk::Review)
        .detail(
            "Deleted files still recoverable from the trash. Emptying it is the \
             point of no return for all of them.",
        )
        // Both halves: the files themselves and the .trashinfo metadata that
        // a file manager needs to show them. Removing one without the other
        // leaves the trash view broken.
        .path(roots.data("Trash/files"))
        .path(roots.data("Trash/info"))
}

/// Measure the home trash.
pub fn scan(context: &Context) -> Category {
    let mut category = Category::new(
        "Trash",
        "Files you have already deleted but not yet discarded.",
    );
    category.targets.push(target(&context.roots));
    context.measure_all(category)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Roots;

    #[test]
    fn an_absent_trash_directory_measures_to_nothing() {
        let fixture = tempfile::tempdir().unwrap();
        let category = scan(&Context::with_roots(Roots::under(fixture.path())));

        assert!(category.targets[0].size.is_zero());
    }

    #[test]
    fn both_the_files_and_their_metadata_are_counted() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        std::fs::create_dir_all(roots.data("Trash/files")).unwrap();
        std::fs::create_dir_all(roots.data("Trash/info")).unwrap();
        std::fs::write(roots.data("Trash/files/report.pdf"), vec![0u8; 3000]).unwrap();
        std::fs::write(
            roots.data("Trash/info/report.pdf.trashinfo"),
            vec![0u8; 100],
        )
        .unwrap();

        let category = scan(&Context::with_roots(roots));

        assert_eq!(category.targets[0].size.apparent, 3100);
        assert_eq!(category.targets[0].files, 2);
    }

    #[test]
    fn emptying_through_the_target_takes_files_and_records_and_keeps_the_trash() {
        // The path the storage page's "Empty trash" takes, end to end.
        use crate::execute::Executor;
        use crate::plan::Plan;

        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let files = roots.data("Trash/files");
        let info = roots.data("Trash/info");
        std::fs::create_dir_all(files.join("course/week-01")).unwrap();
        std::fs::create_dir_all(&info).unwrap();
        std::fs::write(files.join("ubuntu.iso"), vec![0u8; 40_000]).unwrap();
        std::fs::write(files.join("course/week-01/lecture.mp4"), vec![0u8; 9_000]).unwrap();
        std::fs::write(info.join("ubuntu.iso.trashinfo"), b"[Trash Info]\n").unwrap();
        std::fs::write(info.join("course.trashinfo"), b"[Trash Info]\n").unwrap();

        let target = Context::with_roots(roots.clone()).measure(target(&roots));
        let outcome = Executor::applying(&roots).run(&Plan::from_targets([&target]));

        assert!(outcome.is_clean(), "{:?}", outcome.problems);
        assert_eq!(outcome.reclaimed.apparent, target.size.apparent);
        // Emptied, not removed: a file manager expects both directories.
        assert!(files.is_dir() && info.is_dir());
        assert_eq!(std::fs::read_dir(&files).unwrap().count(), 0);
        assert_eq!(std::fs::read_dir(&info).unwrap().count(), 0);
    }
}
