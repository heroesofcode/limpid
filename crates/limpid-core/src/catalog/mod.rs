//! The catalogue of things worth looking for.
//!
//! Each module here contributes one [`Category`]. Scanners only ever measure;
//! the knowledge of *how* to remove something lives with the executor, so
//! that adding a target cannot accidentally add a way to delete it.

use crate::model::{Category, Scan, Target};
use crate::paths::Roots;
use crate::walk::{WalkOptions, measure_all};

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
}

impl Context {
    /// A context for the machine Limpid is running on.
    pub fn new() -> Self {
        Self {
            roots: Roots::from_env(),
            walk: WalkOptions::default(),
        }
    }

    /// A context pointed at a fixture.
    pub fn with_roots(roots: Roots) -> Self {
        Self {
            roots,
            walk: WalkOptions::default(),
        }
    }

    /// Measure every path a target covers and record the result.
    ///
    /// Measuring here rather than in each scanner keeps hardlink accounting
    /// and error handling in one place, and means a scanner is a list of
    /// paths and an explanation — which is all it should be.
    pub fn measure(&self, target: Target) -> Target {
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
            caches::scan(context),
            packages::scan(context),
            development::scan(context),
            logs::scan(context),
            trash::scan(context),
        ],
        caveats: crate::volume::caveats(&context.roots),
    };
    scan.prune();
    scan
}
