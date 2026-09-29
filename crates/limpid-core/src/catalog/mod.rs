//! The catalogue of things worth looking for.
//!
//! Each module here contributes one [`Category`]. Scanners only ever measure;
//! the knowledge of *how* to remove something lives with the executor, so
//! that adding a target cannot accidentally add a way to delete it.

use crate::config::{Config, Store};
use crate::model::{Category, Scan, Target};
use crate::paths::Roots;
use crate::walk::{WalkOptions, measure_all};

pub mod browsers;
pub mod caches;
pub mod development;
pub mod logs;
pub mod packages;
pub mod trash;

/// Everything a scanner needs to know.
pub struct Context {
    /// Where to look.
    pub roots: Roots,
    /// How to measure.
    pub walk: WalkOptions,
    /// What the user has told Limpid: how much to keep, and what to leave
    /// alone.
    pub config: Config,
}

impl Context {
    /// A context for the machine Limpid is running on, with the user's
    /// config file read from disk.
    ///
    /// Anything wrong with that file is dropped here; a caller that wants
    /// to show it should open a [`Store`] itself and use
    /// [`Context::with_config`].
    pub fn new() -> Self {
        let roots = Roots::from_env();
        let config = Store::open(&roots).config;
        Self::with_config(roots, config)
    }

    /// A context pointed at a fixture, with default settings.
    pub fn with_roots(roots: Roots) -> Self {
        Self::with_config(roots, Config::default())
    }

    /// A context with explicit settings.
    pub fn with_config(roots: Roots, config: Config) -> Self {
        Self {
            roots,
            walk: WalkOptions {
                skip: config.exclusions.clone(),
                ..WalkOptions::default()
            },
            config,
        }
    }

    /// Measure every path a target covers and record the result.
    ///
    /// Measuring here rather than in each scanner keeps hardlink accounting
    /// and error handling in one place, and means a scanner is a list of
    /// paths and an explanation — which is all it should be.
    ///
    /// Excluded paths are dropped before measuring rather than after, and the
    /// walk skips anything excluded inside the paths that remain — so the
    /// figure is what would actually go.
    pub fn measure(&self, mut target: Target) -> Target {
        target
            .paths
            .retain(|path| !self.config.exclusions.covers(path));

        match measure_all(&target.paths, &self.walk) {
            Ok(usage) => target.measured(usage.size, usage.files),
            Err(error) => {
                tracing::debug!(target = %target.name, %error, "could not measure target");
                target
            }
        }
    }

    /// Measure every target in a category.
    pub fn measure_all(&self, mut category: Category) -> Category {
        category.targets = category
            .targets
            .into_iter()
            .map(|target| self.measure(target))
            .collect();
        category
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

/// Run every scanner and assemble the result.
pub fn scan(context: &Context) -> Scan {
    let mut scan = Scan {
        categories: vec![
            browsers::scan(context),
            caches::scan(context),
            packages::scan(context),
            development::scan(context),
            logs::scan(context),
            trash::scan(context),
        ],
        capacity: crate::volume::capacity(&context.roots.home),
        caveats: crate::volume::caveats(&context.roots),
    };

    // Measured targets have already lost their excluded paths. This catches
    // the rest — findings that are listed rather than measured, such as the
    // `.pacnew` files — and drops any target left with nothing to point at.
    for category in &mut scan.categories {
        for target in &mut category.targets {
            target
                .paths
                .retain(|path| !context.config.exclusions.covers(path));
        }
        category.targets.retain(|target| !target.paths.is_empty());
    }

    scan.prune();
    scan
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::config::Exclusions;

    fn write(path: &Path, bytes: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![0u8; bytes]).unwrap();
    }

    fn excluding(roots: Roots, paths: impl IntoIterator<Item = PathBuf>) -> Context {
        let config = Config {
            exclusions: Exclusions::new(paths),
            ..Config::default()
        };
        Context::with_config(roots, config)
    }

    fn find<'a>(scan: &'a Scan, name: &str) -> Option<&'a Target> {
        scan.categories
            .iter()
            .flat_map(|category| &category.targets)
            .find(|target| target.name == name)
    }

    #[test]
    fn an_excluded_target_is_not_offered_at_all() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        write(&roots.cache("debuginfod_client/a/debuginfo"), 20_000);
        write(&roots.cache("nvim/log"), 1_000);

        let context = excluding(roots.clone(), [roots.cache("debuginfod_client")]);
        let scan = scan(&context);

        assert!(find(&scan, "Debug symbols").is_none());
        // And nothing else went with it.
        assert!(find(&scan, "Neovim").is_some());
    }

    #[test]
    fn a_partly_excluded_target_reports_only_what_would_go() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        write(&roots.cache("yay/brave-bin/src.tar"), 90_000);
        write(&roots.cache("yay/other/stale.tar"), 10_000);

        let context = excluding(roots.clone(), [roots.cache("yay/brave-bin")]);
        let scan = scan(&context);
        let yay = find(&scan, "AUR build cache (yay)").unwrap();

        // Still offered, but the figure leaves out the part that will stay.
        assert_eq!(yay.size.apparent, 10_000);
    }

    #[test]
    fn an_excluded_privileged_target_asks_the_helper_for_nothing() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let pkg = roots.system("/var/cache/pacman/pkg");
        write(&pkg.join("linux.pkg.tar.zst"), 50_000);

        let context = excluding(roots.clone(), [pkg]);
        let scan = scan(&context);

        assert!(find(&scan, "pacman package cache").is_none());
        let plan = crate::plan::Plan::from_targets(
            scan.categories
                .iter()
                .flat_map(|category| &category.targets),
        );
        assert!(plan.operations.is_empty());
    }

    #[test]
    fn an_excluded_finding_that_is_listed_rather_than_measured_is_dropped_too() {
        // `.pacnew` files never go through measurement, so they need the
        // second pass. One excluded, one not.
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let etc = roots.system("/etc");
        write(&etc.join("pacman.conf.pacnew"), 10);
        write(&etc.join("locale.gen.pacnew"), 10);

        let context = excluding(roots.clone(), [etc.join("locale.gen.pacnew")]);
        let scan = scan(&context);
        let merges = find(&scan, "Unmerged configuration").unwrap();

        assert_eq!(merges.paths, [etc.join("pacman.conf.pacnew")]);
    }

    #[test]
    fn with_nothing_excluded_the_scan_is_unchanged() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        write(&roots.cache("debuginfod_client/a"), 20_000);

        let plain = scan(&Context::with_roots(roots.clone()));
        let configured = scan(&Context::with_config(roots, Config::default()));

        assert_eq!(plain.size(), configured.size());
    }
}
