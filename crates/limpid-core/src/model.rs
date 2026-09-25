//! What a scan produces.
//!
//! A [`Scan`] holds [`Category`]s, each holding [`Target`]s. A target is a
//! proposal, never an action: it says what was found, how big it is, and how
//! dangerous it would be to remove. Deciding and acting happen elsewhere.

use std::path::PathBuf;

use crate::size::Size;

/// What kind of thing a target is, which is what decides how it is cleaned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// Regenerated on demand. The cost of removing it is time, not data.
    Cache,
    /// Log output. Removing it costs you the ability to debug the past.
    Log,
    /// Already discarded by the user; this is the second confirmation.
    Trash,
    /// Compiler and package-manager output, rebuildable from source.
    BuildArtifact,
    /// A downloaded package or source tree that can be fetched again.
    PackageCache,
    /// A crash dump.
    Coredump,
    /// Not reclaimable space at all — something the user has to resolve.
    Attention,
}

impl Kind {
    /// A short label for display.
    pub fn label(self) -> &'static str {
        match self {
            Self::Cache => "cache",
            Self::Log => "log",
            Self::Trash => "trash",
            Self::BuildArtifact => "build artifact",
            Self::PackageCache => "package cache",
            Self::Coredump => "coredump",
            Self::Attention => "needs attention",
        }
    }
}

/// How much care removing a target needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Risk {
    /// Regenerates itself with no user-visible consequence.
    Safe,
    /// Costs something real but recoverable — a slow first launch, a
    /// re-download, a lost undo. Worth reading before agreeing to.
    Review,
    /// Can lose data or break the system. Never cleaned without an explicit,
    /// per-item decision.
    Sensitive,
}

impl Risk {
    /// A short label for display.
    pub fn label(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Review => "review",
            Self::Sensitive => "sensitive",
        }
    }
}

/// One thing that could be cleaned.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Target {
    /// Human-readable name, e.g. "Brave — HTTP cache".
    pub name: String,
    /// One line on what this is and what removing it costs.
    pub detail: String,
    /// The paths this target covers. Usually one, sometimes several when a
    /// single concept is split across trees — browser caches in particular
    /// live in both `~/.cache` and `~/.config`.
    pub paths: Vec<PathBuf>,
    /// Measured size, with each inode counted once across all the paths.
    pub size: Size,
    /// Number of files covered.
    pub files: u64,
    /// What this is.
    pub kind: Kind,
    /// How careful to be.
    pub risk: Risk,
    /// Whether removing it needs privileges the app does not have.
    pub requires_root: bool,
}

impl Target {
    /// Start describing a target. Size is filled in by measurement.
    pub fn new(name: impl Into<String>, kind: Kind, risk: Risk) -> Self {
        Self {
            name: name.into(),
            detail: String::new(),
            paths: Vec::new(),
            size: Size::ZERO,
            files: 0,
            kind,
            risk,
            requires_root: false,
        }
    }

    /// Attach the explanation shown next to the name.
    #[must_use]
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    /// Add a path this target covers.
    #[must_use]
    pub fn path(mut self, path: impl Into<PathBuf>) -> Self {
        self.paths.push(path.into());
        self
    }

    /// Mark this as needing elevation.
    #[must_use]
    pub fn requires_root(mut self) -> Self {
        self.requires_root = true;
        self
    }

    /// Record measured usage.
    #[must_use]
    pub fn measured(mut self, size: Size, files: u64) -> Self {
        self.size = size;
        self.files = files;
        self
    }

    /// Whether this target is worth showing.
    ///
    /// Targets that need attention are always worth showing — they are not
    /// measured in bytes.
    pub fn is_interesting(&self) -> bool {
        self.kind == Kind::Attention || !self.size.is_zero()
    }
}

/// A group of related targets, which is also the unit the UI lists.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Category {
    /// Group name, e.g. "Package manager".
    pub name: String,
    /// What this group is about.
    pub detail: String,
    /// The findings.
    pub targets: Vec<Target>,
}

impl Category {
    /// An empty category.
    pub fn new(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            detail: detail.into(),
            targets: Vec::new(),
        }
    }

    /// Total size across every target here.
    pub fn size(&self) -> Size {
        self.targets.iter().map(|target| target.size).sum()
    }

    /// Total size across targets at or below `risk` that need no elevation.
    pub fn size_at_most(&self, risk: Risk) -> Size {
        self.targets
            .iter()
            .filter(|target| target.risk <= risk && !target.requires_root)
            .map(|target| target.size)
            .sum()
    }
}

/// Everything one run found.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Scan {
    /// The groups, in the order they should be shown.
    pub categories: Vec<Category>,
    /// Caveats that apply to the whole scan, such as snapshots pinning
    /// extents so that removing files frees nothing until they rotate out.
    pub caveats: Vec<String>,
}

impl Scan {
    /// Total size across every category.
    pub fn size(&self) -> Size {
        self.categories.iter().map(Category::size).sum()
    }

    /// Total size that could be reclaimed without elevation and without
    /// touching anything marked sensitive.
    pub fn reclaimable_unprivileged(&self) -> Size {
        self.categories
            .iter()
            .map(|category| category.size_at_most(Risk::Review))
            .sum()
    }

    /// Drop categories that found nothing, so the UI has no empty sections.
    pub fn prune(&mut self) {
        for category in &mut self.categories {
            category.targets.retain(Target::is_interesting);
        }
        self.categories
            .retain(|category| !category.targets.is_empty());
        // Biggest first: it is the order every question about disk space is
        // really asking about.
        self.categories
            .sort_by_key(|category| std::cmp::Reverse(category.size().on_disk));
        for category in &mut self.categories {
            category
                .targets
                .sort_by_key(|target| std::cmp::Reverse(target.size.on_disk));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(name: &str, on_disk: u64, risk: Risk) -> Target {
        Target::new(name, Kind::Cache, risk).measured(Size::new(on_disk, on_disk), 1)
    }

    #[test]
    fn a_category_sums_its_targets() {
        let mut category = Category::new("Caches", "");
        category.targets.push(target("a", 100, Risk::Safe));
        category.targets.push(target("b", 200, Risk::Safe));

        assert_eq!(category.size().on_disk, 300);
    }

    #[test]
    fn sensitive_and_privileged_targets_are_left_out_of_the_reclaimable_total() {
        let mut category = Category::new("Mixed", "");
        category.targets.push(target("safe", 100, Risk::Safe));
        category.targets.push(target("review", 200, Risk::Review));
        category
            .targets
            .push(target("sensitive", 400, Risk::Sensitive));
        category
            .targets
            .push(target("needs root", 800, Risk::Safe).requires_root());

        let scan = Scan {
            categories: vec![category],
            caveats: Vec::new(),
        };

        assert_eq!(scan.size().on_disk, 1500);
        assert_eq!(scan.reclaimable_unprivileged().on_disk, 300);
    }

    #[test]
    fn pruning_drops_empty_targets_and_categories() {
        let mut empty = Category::new("Nothing", "");
        empty.targets.push(target("zero", 0, Risk::Safe));

        let mut full = Category::new("Something", "");
        full.targets.push(target("real", 10, Risk::Safe));

        let mut scan = Scan {
            categories: vec![empty, full],
            caveats: Vec::new(),
        };
        scan.prune();

        assert_eq!(scan.categories.len(), 1);
        assert_eq!(scan.categories[0].name, "Something");
    }

    #[test]
    fn pruning_keeps_zero_sized_attention_items() {
        let mut category = Category::new("Config", "");
        category.targets.push(Target::new(
            "pacnew files",
            Kind::Attention,
            Risk::Sensitive,
        ));

        let mut scan = Scan {
            categories: vec![category],
            caveats: Vec::new(),
        };
        scan.prune();

        assert_eq!(scan.categories.len(), 1);
    }

    #[test]
    fn pruning_orders_by_size_descending() {
        let mut small = Category::new("Small", "");
        small.targets.push(target("s", 10, Risk::Safe));
        let mut large = Category::new("Large", "");
        large.targets.push(target("l1", 100, Risk::Safe));
        large.targets.push(target("l2", 500, Risk::Safe));

        let mut scan = Scan {
            categories: vec![small, large],
            caveats: Vec::new(),
        };
        scan.prune();

        assert_eq!(scan.categories[0].name, "Large");
        assert_eq!(scan.categories[0].targets[0].name, "l2");
    }
}
