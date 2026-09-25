//! The freedesktop trash.
//!
//! Emptying the trash is the one action where the user has already said yes
//! once. That makes it low-risk but never automatic: it is also the only
//! undo they have for everything else they deleted this month.

use super::Context;
use crate::model::{Category, Kind, Risk, Target};

/// Measure the home trash.
///
/// Per-mount trash directories (`$topdir/.Trash-$uid`) are not covered yet;
/// finding them means walking the mount table, which belongs with the
/// executor that would have to empty them coherently.
pub fn scan(context: &Context) -> Category {
    let mut category = Category::new(
        "Trash",
        "Files you have already deleted but not yet discarded.",
    );

    category.targets.push(
        Target::new("Home trash", Kind::Trash, Risk::Review)
            .detail(
                "Deleted files still recoverable from the trash. Emptying it is the \
                 point of no return for all of them.",
            )
            // Both halves: the files themselves and the .trashinfo metadata
            // that a file manager needs to show them. Removing one without the
            // other leaves the trash view broken.
            .path(context.roots.data("Trash/files"))
            .path(context.roots.data("Trash/info")),
    );

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
}
