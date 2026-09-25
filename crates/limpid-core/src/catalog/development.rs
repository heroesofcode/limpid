//! Toolchain caches.
//!
//! These are usually the easiest large win on a developer's machine: every
//! byte here was downloaded once and can be downloaded again. The exceptions
//! are worth knowing. The cargo registry index is slow enough to rebuild that
//! it is kept; the pnpm store is hardlinked into every `node_modules` on the
//! machine, so removing it wholesale breaks projects that look untouched.

use super::Context;
use crate::model::{Category, Kind, Risk, Target};

/// Measure toolchain and package-manager caches.
pub fn scan(context: &Context) -> Category {
    let roots = &context.roots;
    let mut category = Category::new(
        "Development",
        "Downloaded dependencies and compiler output, all rebuildable.",
    );

    category.targets.push(
        Target::new("npm cache", Kind::Cache, Risk::Safe)
            .detail("Packages npm has downloaded. Re-fetched on the next install.")
            .path(roots.home(".npm/_cacache")),
    );

    category.targets.push(
        Target::new("Yarn cache", Kind::Cache, Risk::Safe)
            .detail("Packages Yarn has downloaded.")
            .path(roots.cache("yarn"))
            .path(roots.home(".yarn/berry/cache")),
    );

    // Deliberately only the two halves that are pure download cache. The
    // registry index is excluded: removing it makes the next build re-fetch
    // the whole crate index, which is minutes of waiting for a modest win.
    category.targets.push(
        Target::new("Cargo registry cache", Kind::Cache, Risk::Safe)
            .detail(
                "Downloaded crate archives and their unpacked sources. The registry \
                 index is left alone, because rebuilding it is slow.",
            )
            .path(roots.home(".cargo/registry/cache"))
            .path(roots.home(".cargo/registry/src")),
    );

    category.targets.push(
        Target::new("pip cache", Kind::Cache, Risk::Safe)
            .detail("Wheels and archives pip has downloaded.")
            .path(roots.cache("pip")),
    );

    category.targets.push(
        Target::new("Go build cache", Kind::Cache, Risk::Safe)
            .detail("Compiled package archives, rebuilt on demand.")
            .path(roots.cache("go-build")),
    );

    // The module cache is mode 0444 by design, so a plain recursive delete
    // fails part-way through and leaves it half-removed. It has to go through
    // `go clean -modcache`, which is why it is a separate, flagged target.
    category.targets.push(
        Target::new("Go module cache", Kind::Cache, Risk::Review)
            .detail(
                "Downloaded Go modules. Stored read-only, so this one has to be \
                 removed with go clean rather than deleted directly.",
            )
            .path(roots.home("go/pkg/mod")),
    );

    context.measure_all(category)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Roots;

    fn write(roots: &Roots, relative: &str, bytes: usize) {
        let path = roots.home.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, vec![0u8; bytes]).unwrap();
    }

    #[test]
    fn the_npm_cache_is_measured() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        write(&roots, ".npm/_cacache/index-v5/aa/blob", 7000);

        let category = scan(&Context::with_roots(roots));
        let target = category
            .targets
            .iter()
            .find(|t| t.name == "npm cache")
            .unwrap();

        assert_eq!(target.size.apparent, 7000);
    }

    #[test]
    fn the_cargo_registry_index_is_left_out_of_the_measurement() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        write(&roots, ".cargo/registry/cache/crate.crate", 1000);
        write(&roots, ".cargo/registry/index/huge-index-file", 500_000);

        let category = scan(&Context::with_roots(roots));
        let target = category
            .targets
            .iter()
            .find(|t| t.name == "Cargo registry cache")
            .unwrap();

        assert_eq!(target.size.apparent, 1000);
    }

    #[test]
    fn a_target_spanning_two_directories_sums_them() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        write(&roots, ".cache/yarn/v6/a", 100);
        write(&roots, ".yarn/berry/cache/b.zip", 200);

        let category = scan(&Context::with_roots(roots));
        let target = category
            .targets
            .iter()
            .find(|t| t.name == "Yarn cache")
            .unwrap();

        assert_eq!(target.size.apparent, 300);
    }

    #[test]
    fn the_read_only_go_module_cache_is_flagged_for_review() {
        let fixture = tempfile::tempdir().unwrap();
        let category = scan(&Context::with_roots(Roots::under(fixture.path())));
        let target = category
            .targets
            .iter()
            .find(|t| t.name == "Go module cache")
            .unwrap();

        assert_eq!(target.risk, Risk::Review);
    }
}
