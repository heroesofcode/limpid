//! What the user agreed to.
//!
//! A scan produces proposals; a plan is the subset someone said yes to. The
//! separation matters because the executor takes a plan and nothing else —
//! there is no path from "found something" to "removed it" that does not go
//! through an explicit selection.

use std::path::PathBuf;

use crate::guard::Permission;
use crate::model::{Kind, Risk, Target};
use crate::privileged::Operation;
use crate::size::Size;
use crate::volume::Capacity;

/// Below this, no plan is large enough to question. On a nearly empty disk
/// an ordinary cache is a big share of very little, and asking about it
/// would teach people to tick the box without reading.
const LARGE_FLOOR: u64 = 1 << 30;

/// Above this share of everything stored on the disk, a plan is looked at
/// twice.
///
/// Not because removing that much is wrong — a quarter of a developer's disk
/// can honestly be one project's build output — but because a scanner bug
/// that turns a cache into a home directory looks exactly like this, and the
/// guard should not be the only thing that notices.
const LARGE_SHARE: f64 = 0.25;

/// A plan too large to go through on the usual single confirmation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Magnitude {
    /// What the plan expects to reclaim.
    pub expected: Size,
    /// That, as a share of the space in use on the disk. `None` when the
    /// disk could not be measured.
    pub share: Option<f64>,
}

impl Magnitude {
    /// A sentence saying how large, for the confirmation.
    pub fn describe(&self) -> String {
        match self.share {
            Some(share) => format!(
                "This removes {:.0}% of everything stored on this disk, which is more \
                 than a cleaner usually finds.",
                share * 100.0,
            ),
            None => "The size of this disk could not be read, so there is nothing to \
                     compare this with."
                .to_owned(),
        }
    }
}

/// How a target should be got rid of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    /// A directory that must have nothing open inside it when this runs.
    pub requires_idle: Option<PathBuf>,
    /// How this came to be here, which decides how the guard checks it.
    ///
    /// Carried rather than decided at the point of removal, so there is no
    /// call site that can pick the looser check for something a scanner
    /// produced.
    pub permission: Permission,
    /// What the scan measured, for comparison against what actually happened.
    pub expected: Size,
}

/// Everything to be done, in one go.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Plan {
    /// The items this process removes itself.
    pub items: Vec<Item>,
    /// Operations the privileged helper is asked to carry out.
    ///
    /// Kept apart from the items because they are a different kind of
    /// thing: an item is a set of paths, an operation is a named policy with
    /// no paths in it at all.
    pub operations: Vec<Operation>,
    /// What the operations are expected to reclaim.
    pub operations_expected: Size,
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
        let mut plan = Self::default();

        for target in targets {
            if target.blocked.is_some() || target.kind == Kind::Attention {
                continue;
            }

            match target.privileged {
                Some(operation) => {
                    plan.operations.push(operation);
                    plan.operations_expected += target.size;
                }
                None if !target.requires_root => plan.items.push(Item {
                    name: target.name.clone(),
                    paths: target.paths.clone(),
                    disposal: Disposal::for_kind(target.kind),
                    requires_idle: target.requires_idle.clone(),
                    // The scanner's, not decided here: what makes build
                    // output safe to remove is a rule, and only the scanner
                    // that applied it knows it did.
                    permission: target.permission,
                    expected: target.size,
                }),
                // Needs elevation but nothing knows how to do it. Dropped
                // rather than attempted, which would only produce a
                // permission error per file.
                None => {}
            }
        }

        plan
    }

    /// Build a plan from files the user selected on screen.
    ///
    /// Separate from [`Plan::from_targets`] on purpose. These paths were
    /// never produced by a scanner and are not inside any boundary; what
    /// makes them safe is that a person pointed at each one. That fact is
    /// recorded here as [`Permission::Chosen`] and read again by the
    /// executor, so it cannot be lost in between.
    ///
    /// The default disposal is the trash. This is the user's own data, not
    /// a regenerable cache, and on the same filesystem trashing it is a
    /// rename — instant even for a file of several gigabytes.
    pub fn from_chosen(
        chosen: impl IntoIterator<Item = (PathBuf, Size)>,
        disposal: Disposal,
    ) -> Self {
        let items = chosen
            .into_iter()
            .map(|(path, expected)| Item {
                name: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string()),
                paths: vec![path],
                disposal,
                requires_idle: None,
                permission: Permission::Chosen,
                expected,
            })
            .collect();

        Self {
            items,
            ..Self::default()
        }
    }

    /// Total size the plan expects to reclaim, both halves together.
    pub fn expected(&self) -> Size {
        self.items.iter().map(|item| item.expected).sum::<Size>() + self.operations_expected
    }

    /// Whether there is anything to do.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.operations.is_empty()
    }

    /// Whether any of this needs the helper.
    pub fn needs_elevation(&self) -> bool {
        !self.operations.is_empty()
    }

    /// Whether this plan is too large to go through on the usual single
    /// confirmation, measured against the disk it is on.
    ///
    /// Everything counts, the privileged half too: the helper's operations
    /// are bounded, but what they were measured to free is still a claim
    /// about this disk. A disk that could not be measured is not a reason to
    /// wave a large plan through — anything over the floor is questioned.
    pub fn magnitude(&self, capacity: Option<Capacity>) -> Option<Magnitude> {
        let expected = self.expected();
        if expected.on_disk < LARGE_FLOOR {
            return None;
        }

        let share = capacity
            .filter(|capacity| capacity.used() > 0)
            .map(|capacity| expected.on_disk as f64 / capacity.used() as f64);

        match share {
            Some(share) if share <= LARGE_SHARE => None,
            share => Some(Magnitude { expected, share }),
        }
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
            && target.blocked.is_none()
            && target.risk <= self.up_to
            && (self.include_privileged || !target.requires_root)
            // A privileged target with no operation cannot be cleaned by
            // anything, so offering it would be a lie.
            && (!target.requires_root || target.privileged.is_some())
    }
}

impl Default for Selection {
    fn default() -> Self {
        Self::SAFE
    }
}

#[cfg(test)]
mod magnitude {
    use super::*;

    const GIB: u64 = 1 << 30;

    fn plan_of(bytes: u64) -> Plan {
        Plan::from_chosen(
            [(PathBuf::from("/home/x/big"), Size::new(bytes, bytes))],
            Disposal::Delete,
        )
    }

    fn disk(used: u64) -> Option<Capacity> {
        Some(Capacity {
            total: used * 2,
            available: used,
        })
    }

    #[test]
    fn a_plan_that_is_a_large_share_of_the_disk_is_questioned() {
        // One project's build output on this machine: 21.6 of 52 GB in use.
        let large = plan_of(21 * GIB + GIB / 2)
            .magnitude(disk(52 * GIB))
            .unwrap();

        assert!(large.share.is_some_and(|share| share > 0.4));
        assert!(large.describe().contains("41%"), "{}", large.describe());
    }

    #[test]
    fn a_plan_that_is_a_small_share_goes_through() {
        assert_eq!(plan_of(5 * GIB).magnitude(disk(100 * GIB)), None);
    }

    #[test]
    fn nothing_under_the_floor_is_questioned_however_empty_the_disk() {
        // Half of a nearly empty disk, and still only a cache.
        assert_eq!(plan_of(GIB / 2).magnitude(disk(GIB)), None);
    }

    #[test]
    fn a_disk_that_cannot_be_measured_does_not_wave_a_large_plan_through() {
        let unknown = plan_of(2 * GIB).magnitude(None).unwrap();

        assert_eq!(unknown.share, None);
        assert!(plan_of(GIB / 2).magnitude(None).is_none());
    }

    #[test]
    fn the_privileged_half_counts_too() {
        let mut plan = plan_of(GIB / 2);
        plan.operations.push(Operation::VacuumJournal { days: 7 });
        plan.operations_expected = Size::new(30 * GIB, 30 * GIB);

        assert!(plan.magnitude(disk(52 * GIB)).is_some());
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
    fn the_idle_requirement_is_carried_into_the_plan() {
        let target = target("Brave — web cache", Kind::Cache, Risk::Safe)
            .requires_idle("/home/x/.config/BraveSoftware");

        let plan = Plan::from_targets(std::slice::from_ref(&target));

        assert_eq!(
            plan.items[0].requires_idle.as_deref(),
            Some(std::path::Path::new("/home/x/.config/BraveSoftware")),
        );
    }

    #[test]
    fn a_blocked_target_never_reaches_a_plan() {
        let targets = vec![
            target("free", Kind::Cache, Risk::Safe),
            target("open", Kind::Cache, Risk::Safe).blocked("Brave is running"),
        ];

        let plan = Plan::from_targets(&targets);

        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].name, "free");
        assert!(!Selection::SAFE.includes(&targets[1]));
    }

    #[test]
    fn a_chosen_file_carries_its_own_permission_and_goes_to_the_trash() {
        let plan = Plan::from_chosen(
            [(
                PathBuf::from("/home/x/Videos/holiday.mkv"),
                Size::new(4096, 4096),
            )],
            Disposal::Trash,
        );

        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].name, "holiday.mkv");
        assert_eq!(plan.items[0].permission, Permission::Chosen);
        assert_eq!(plan.items[0].disposal, Disposal::Trash);
        assert!(plan.items[0].requires_idle.is_none());
    }

    #[test]
    fn a_catalogued_target_never_becomes_a_chosen_one() {
        let plan = Plan::from_targets(&[target("cache", Kind::Cache, Risk::Safe)]);

        assert_eq!(plan.items[0].permission, Permission::Catalogued);
    }

    #[test]
    fn a_chosen_file_can_be_removed_outright_when_asked_for_explicitly() {
        let plan = Plan::from_chosen(
            [(PathBuf::from("/home/x/big.iso"), Size::new(10, 10))],
            Disposal::Delete,
        );

        assert_eq!(plan.items[0].disposal, Disposal::Delete);
        assert!(plan.has_permanent_deletions());
    }

    #[test]
    fn a_privileged_target_becomes_an_operation_rather_than_an_item() {
        let targets = vec![
            target("cache", Kind::Cache, Risk::Safe),
            target("pacman", Kind::PackageCache, Risk::Review)
                .by_operation(Operation::TrimPackageCache { keep: 3 }),
        ];

        let plan = Plan::from_targets(&targets);

        assert_eq!(plan.items.len(), 1);
        assert_eq!(
            plan.operations,
            vec![Operation::TrimPackageCache { keep: 3 }]
        );
        assert!(plan.needs_elevation());
        // Both halves count towards what the confirmation promises.
        assert_eq!(plan.expected().on_disk, 200);
    }

    #[test]
    fn a_privileged_target_with_no_operation_is_dropped_and_never_offered() {
        let orphan = target("mystery", Kind::Log, Risk::Safe).requires_root();

        assert!(Plan::from_targets(std::slice::from_ref(&orphan)).is_empty());
        let everything = Selection {
            up_to: Risk::Sensitive,
            include_privileged: true,
        };
        assert!(!everything.includes(&orphan));
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

        let journal = target("journal", Kind::Log, Risk::Review)
            .by_operation(Operation::VacuumJournal { days: 14 });
        assert!(selection.includes(&journal));
        assert!(!selection.includes(&target("pacnew", Kind::Attention, Risk::Sensitive)));
    }
}
