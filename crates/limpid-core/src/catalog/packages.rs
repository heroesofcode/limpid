//! Package manager caches.
//!
//! On a rolling distribution the package cache is usually the largest single
//! thing on the disk, and it is also a recovery path: the cached package is
//! how you downgrade after an update breaks something. So the cache is
//! reported in two halves — the versions worth keeping, and the rest.

use super::Context;
use crate::model::{Category, Kind, Risk, Target};
use crate::privileged::Operation;

/// Package versions kept when the cache is trimmed.
///
/// `paccache`'s own default. Three is enough to get back past a bad update
/// without the cache being most of what it was.
const KEEP_VERSIONS: u8 = 3;

/// Measure package manager caches.
pub fn scan(context: &Context) -> Category {
    let roots = &context.roots;
    let mut category = Category::new(
        "Package manager",
        "Downloaded packages and build trees, re-fetchable from the mirrors.",
    );

    category.targets.push(
        Target::new("pacman package cache", Kind::PackageCache, Risk::Review)
            .detail(
                "Every package version pacman has downloaded. Keeping the last few \
                 is what makes a downgrade possible after a bad update, so Limpid \
                 trims rather than empties it.",
            )
            .path(roots.system("/var/cache/pacman/pkg"))
            .by_operation(Operation::TrimPackageCache {
                keep: KEEP_VERSIONS,
            }),
    );

    // yay keeps the upstream tarball and the extracted build tree per package;
    // a single large AUR binary package can account for most of it.
    category.targets.push(
        Target::new("AUR build cache (yay)", Kind::BuildArtifact, Risk::Safe)
            .detail(
                "Sources and build trees for AUR packages. Re-downloaded and rebuilt \
                 the next time one of them updates.",
            )
            .path(roots.cache("yay")),
    );

    category.targets.push(
        Target::new("AUR build cache (paru)", Kind::BuildArtifact, Risk::Safe)
            .detail("Cloned AUR repositories and build output.")
            .path(roots.cache("paru")),
    );

    category.targets.push(
        Target::new("Flatpak per-application caches", Kind::Cache, Risk::Safe)
            .detail("Caches written by sandboxed applications, outside their data.")
            .path(roots.home(".var/app")),
    );

    let mut category = context.measure_all(category);

    // Not space, and deliberately listed anyway: an unmerged .pacnew means
    // the system is running without a change upstream considered necessary,
    // and the symptom shows up weeks later as something unrelated breaking.
    if let Some(target) = pending_config_merges(context) {
        category.targets.push(target);
    }

    category
}

/// Report `.pacnew` and `.pacsave` files left behind by package upgrades.
///
/// Returns `None` when there are none, which is the healthy case.
fn pending_config_merges(context: &Context) -> Option<Target> {
    let etc = context.roots.system("/etc");
    let pending = crate::walk::find_by_suffix(&etc, &[".pacnew", ".pacsave"], &context.walk);

    if pending.is_empty() {
        return None;
    }

    let detail = format!(
        "{} configuration file{} left behind by package upgrades. These are not \
         reclaimable space: merge them with pacdiff. Deleting a .pacnew unmerged \
         means silently running without a change upstream thought necessary.",
        pending.len(),
        if pending.len() == 1 { "" } else { "s" },
    );

    let mut target = Target::new("Unmerged configuration", Kind::Attention, Risk::Sensitive)
        .detail(detail)
        .requires_root();
    target.paths = pending;
    Some(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Roots;

    fn fixture_with(relative: &str, bytes: usize) -> (tempfile::TempDir, Roots) {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let path = roots.home.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, vec![0u8; bytes]).unwrap();
        (fixture, roots)
    }

    #[test]
    fn the_pacman_cache_is_measured_under_the_system_root() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let cache = roots.system("/var/cache/pacman/pkg");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("linux-7.2.3-1.pkg.tar.zst"), vec![0u8; 9000]).unwrap();

        let category = scan(&Context::with_roots(roots));
        let target = category
            .targets
            .iter()
            .find(|target| target.name == "pacman package cache")
            .unwrap();

        assert_eq!(target.size.apparent, 9000);
        assert!(target.requires_root);
    }

    #[test]
    fn the_aur_cache_is_measured_and_needs_no_elevation() {
        let (_fixture, roots) = fixture_with(".cache/yay/brave-bin/brave.tar.zst", 4096);

        let category = scan(&Context::with_roots(roots));
        let target = category
            .targets
            .iter()
            .find(|target| target.name == "AUR build cache (yay)")
            .unwrap();

        assert_eq!(target.size.apparent, 4096);
        assert!(!target.requires_root);
    }

    #[test]
    fn pacnew_files_are_reported_as_work_to_do_not_as_space() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let etc = roots.system("/etc");
        std::fs::create_dir_all(etc.join("ssh")).unwrap();
        std::fs::write(etc.join("pacman.conf.pacnew"), b"x").unwrap();
        std::fs::write(etc.join("ssh/sshd_config.pacsave"), b"x").unwrap();

        let category = scan(&Context::with_roots(roots));
        let target = category
            .targets
            .iter()
            .find(|target| target.name == "Unmerged configuration")
            .unwrap();

        assert_eq!(target.kind, Kind::Attention);
        assert_eq!(target.risk, Risk::Sensitive);
        assert_eq!(target.paths.len(), 2);
        // Never measured: two kilobytes of config are not the point.
        assert!(target.size.is_zero());
        // And it still survives pruning, because it is not about bytes.
        assert!(target.is_interesting());
    }

    #[test]
    fn a_clean_etc_reports_no_pending_merges() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        std::fs::create_dir_all(roots.system("/etc")).unwrap();
        std::fs::write(roots.system("/etc/pacman.conf"), b"x").unwrap();

        let category = scan(&Context::with_roots(roots));

        assert!(
            !category
                .targets
                .iter()
                .any(|t| t.name == "Unmerged configuration")
        );
    }

    #[test]
    fn the_package_cache_is_never_marked_safe() {
        let fixture = tempfile::tempdir().unwrap();
        let category = scan(&Context::with_roots(Roots::under(fixture.path())));
        let target = category
            .targets
            .iter()
            .find(|target| target.name == "pacman package cache")
            .unwrap();

        // Emptying it costs the ability to downgrade, which is a real recovery
        // path on a rolling release.
        assert_ne!(target.risk, Risk::Safe);
    }
}
