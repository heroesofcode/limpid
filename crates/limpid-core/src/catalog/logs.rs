//! Logs and crash dumps.
//!
//! Both are cheap to delete and occasionally priceless to have kept. The
//! journal is sized by policy rather than by accident — systemd caps it at
//! 10% of the filesystem by default — so a large journal is not a fault, and
//! the honest fix is usually a smaller cap rather than a one-off vacuum.

use super::Context;
use crate::model::{Category, Kind, Risk, Target};

/// Measure the journal and the coredump store.
pub fn scan(context: &Context) -> Category {
    let roots = &context.roots;
    let mut category = Category::new(
        "Logs and crash dumps",
        "What the system recorded about its own past.",
    );

    category.targets.push(
        Target::new("System journal", Kind::Log, Risk::Review)
            .detail(
                "Everything systemd has logged. Trimming it is safe for the running \
                 system and costs you the history you would need to debug whatever \
                 breaks next.",
            )
            .path(roots.system("/var/log/journal"))
            .requires_root(),
    );

    category.targets.push(
        Target::new("Crash dumps", Kind::Coredump, Risk::Review)
            .detail(
                "Compressed core dumps from crashed programs. systemd already expires \
                 these on a timer; removing one means no backtrace for that crash.",
            )
            .path(roots.system("/var/lib/systemd/coredump"))
            .requires_root(),
    );

    context.measure_all(category)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Roots;

    #[test]
    fn the_journal_is_measured_and_needs_elevation() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let journal = roots.system("/var/log/journal/abcdef");
        std::fs::create_dir_all(&journal).unwrap();
        std::fs::write(journal.join("system.journal"), vec![0u8; 12_000]).unwrap();

        let category = scan(&Context::with_roots(roots));
        let target = category
            .targets
            .iter()
            .find(|t| t.name == "System journal")
            .unwrap();

        assert_eq!(target.size.apparent, 12_000);
        assert!(target.requires_root);
    }

    #[test]
    fn crash_dumps_are_measured() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let dumps = roots.system("/var/lib/systemd/coredump");
        std::fs::create_dir_all(&dumps).unwrap();
        std::fs::write(dumps.join("core.magick.1000.zst"), vec![0u8; 2400]).unwrap();

        let category = scan(&Context::with_roots(roots));
        let target = category
            .targets
            .iter()
            .find(|t| t.name == "Crash dumps")
            .unwrap();

        assert_eq!(target.size.apparent, 2400);
        assert_eq!(target.files, 1);
    }

    #[test]
    fn neither_is_ever_offered_as_a_safe_default() {
        let fixture = tempfile::tempdir().unwrap();
        let category = scan(&Context::with_roots(Roots::under(fixture.path())));

        assert!(
            category
                .targets
                .iter()
                .all(|target| target.risk != Risk::Safe)
        );
    }
}
