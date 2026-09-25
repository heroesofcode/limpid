//! What the user agreed to.
//!
//! A scan produces proposals; a plan is the subset someone said yes to. The
//! separation matters because the executor takes a plan and nothing else —
//! there is no path from "found something" to "removed it" that does not go
//! through an explicit selection.

use std::path::PathBuf;

use crate::model::{Kind, Risk, Target};
use crate::size::Size;

/// How a target should be got rid of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Disposal {
    /// Removed outright.
    ///
    /// The right answer for anything regenerable. Sending a cache to the
    /// trash would move the bytes to another directory on the same
    /// filesystem and free nothing at all, while filling the one place the
    /// user looks to recover things.
    Delete,
    /// Moved to the freedesktop trash, where it can be got back.
    ///
    /// For anything that a person, rather than a program, would have to
    /// recreate.
    Trash,
}

impl Disposal {
    /// The disposal a kind of finding gets unless told otherwise.
    pub fn for_kind(kind: Kind) -> Self {
        match kind {
            // All regenerable, and all large enough that trashing them would
            // defeat the purpose.
            Kind::Cache | Kind::Log | Kind::BuildArtifact | Kind::PackageCache | Kind::Coredump => {
                Self::Delete
            }
            // Trashing the trash is not a thing.
            Kind::Trash => Self::Delete,
            // Never actually removed; present so the match stays total.
            Kind::Attention => Self::Trash,
        }
    }

    /// A phrase for a confirmation prompt.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Delete => "removed permanently",
            Self::Trash => "moved to the trash",
        }
    }
}

/// One thing to be done.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Item {
    /// The name shown to the user when they agreed.
    pub name: String,
    /// Directories whose contents go, or files that go.
    pub paths: Vec<PathBuf>,
    /// How it goes.
    pub disposal: Disposal,
    /// What the scan measured, for comparison against what actually happened.
    pub expected: Size,
}

/// Everything to be done, in one go.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Plan {
    /// The items, in the order they will be acted on.
    pub items: Vec<Item>,
}

impl Plan {
    /// An empty plan.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a plan from chosen targets.
    ///
    /// Targets that need attention are dropped rather than refused: they are
    /// not reclaimable space and there is nothing for the executor to do with
    /// them. Anything needing elevation is dropped too — that belongs to the
    /// privileged helper, and letting it reach this executor would only
    /// produce a permission error per file.
    pub fn from_targets<'a>(targets: impl IntoIterator<Item = &'a Target>) -> Self {
        let items = targets
            .into_iter()
            .filter(|target| target.kind != Kind::Attention && !target.requires_root)
            .map(|target| Item {
                name: target.name.clone(),
                paths: target.paths.clone(),
                disposal: Disposal::for_kind(target.kind),
                expected: target.size,
            })
            .collect();
        Self { items }
    }

    /// Total size the plan expects to reclaim.
    pub fn expected(&self) -> Size {
        self.items.iter().map(|item| item.expected).sum()
    }

    /// Whether there is anything to do.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Whether anything in the plan is removed rather than trashed.
    ///
    /// Drives the wording of the confirmation: "removed permanently" needs a
    /// firmer yes than "moved to the trash".
    pub fn has_permanent_deletions(&self) -> bool {
        self.items
            .iter()
            .any(|item| item.disposal == Disposal::Delete)
    }
}

/// Which findings to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    /// The highest risk level to include.
    pub up_to: Risk,
    /// Whether to include findings that need elevation.
    pub include_privileged: bool,
}

impl Selection {
    /// Everything that regenerates itself with no consequence.
    pub const SAFE: Self = Self {
        up_to: Risk::Safe,
        include_privileged: false,
    };

    /// Whether a target is in this selection.
    pub fn includes(&self, target: &Target) -> bool {
        target.kind != Kind::Attention
            && target.risk <= self.up_to
            && (self.include_privileged || !target.requires_root)
    }
}

impl Default for Selection {
    fn default() -> Self {
        Self::SAFE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(name: &str, kind: Kind, risk: Risk) -> Target {
        Target::new(name, kind, risk)
            .path("/tmp/x")
            .measured(Size::new(100, 100), 1)
    }

    #[test]
    fn regenerable_findings_are_deleted_rather_than_trashed() {
        // Trashing a cache moves the bytes to another directory on the same
        // filesystem and frees nothing.
        for kind in [
            Kind::Cache,
            Kind::Log,
            Kind::BuildArtifact,
            Kind::PackageCache,
        ] {
            assert_eq!(Disposal::for_kind(kind), Disposal::Delete, "{kind:?}");
        }
    }

    #[test]
    fn a_plan_leaves_out_what_the_executor_cannot_act_on() {
        let targets = vec![
            target("cache", Kind::Cache, Risk::Safe),
            target("pacnew", Kind::Attention, Risk::Sensitive),
            target("journal", Kind::Log, Risk::Review).requires_root(),
        ];

        let plan = Plan::from_targets(&targets);

        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].name, "cache");
    }

    #[test]
    fn a_plan_totals_what_it_expects_to_reclaim() {
        let targets = vec![
            target("a", Kind::Cache, Risk::Safe),
            target("b", Kind::Cache, Risk::Safe),
        ];

        assert_eq!(Plan::from_targets(&targets).expected().on_disk, 200);
    }

    #[test]
    fn an_empty_plan_is_empty() {
        assert!(Plan::new().is_empty());
        assert!(Plan::from_targets(&[target("p", Kind::Attention, Risk::Safe)]).is_empty());
    }

    #[test]
    fn the_default_selection_is_the_cautious_one() {
        let selection = Selection::default();

        assert!(selection.includes(&target("safe", Kind::Cache, Risk::Safe)));
        assert!(!selection.includes(&target("review", Kind::Cache, Risk::Review)));
        assert!(!selection.includes(&target("sensitive", Kind::Cache, Risk::Sensitive)));
        assert!(!selection.includes(&target("root", Kind::Cache, Risk::Safe).requires_root()));
    }

    #[test]
    fn a_wider_selection_takes_more_but_never_takes_attention_items() {
        let selection = Selection {
            up_to: Risk::Sensitive,
            include_privileged: true,
        };

        assert!(selection.includes(&target("root", Kind::Log, Risk::Review).requires_root()));
        assert!(!selection.includes(&target("pacnew", Kind::Attention, Risk::Sensitive)));
    }
}
