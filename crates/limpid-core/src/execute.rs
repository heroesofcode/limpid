//! Carrying out a plan.
//!
//! Two habits make this safe rather than careful-sounding. Every path goes
//! past the [`Guard`] immediately before it is acted on, not when the plan
//! was built — the world can change in between. And the default is a dry
//! run, so the destructive path is one a caller has to ask for by name.
//!
//! Directories are emptied rather than removed. Applications expect their
//! cache directory to exist and quietly misbehave when it does not, and an
//! empty directory costs nothing.

use std::path::{Path, PathBuf};

use crate::config::Exclusions;
use crate::guard::{Guard, Refusal};
use crate::paths::Roots;
use crate::plan::{Disposal, Item, Plan};
use crate::size::Size;
use crate::walk::{self, WalkOptions};

/// Something that could not be done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// Something had a file open inside a directory that had to be idle.
    InUse {
        /// What the item was called.
        item: String,
        /// The process holding it.
        holder: String,
    },
    /// The user excluded it. Reported rather than silently skipped, because
    /// it only reaches the executor through a plan built before the
    /// exclusion, and the person who pressed the button should hear that
    /// part of it was not done.
    Excluded(PathBuf),
    /// The guard would not allow it.
    Refused(Refusal),
    /// The filesystem would not allow it.
    Failed {
        /// What could not be removed.
        path: PathBuf,
        /// What the operating system said.
        reason: String,
    },
}

impl std::fmt::Display for Problem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InUse { item, holder } => write!(
                formatter,
                "{item}: {holder} still has files open there. Close it and scan again.",
            ),
            Self::Excluded(path) => {
                write!(
                    formatter,
                    "{} is excluded, and was left alone",
                    path.display()
                )
            }
            Self::Refused(refusal) => write!(formatter, "{refusal}"),
            Self::Failed { path, reason } => write!(formatter, "{}: {reason}", path.display()),
        }
    }
}

/// What a run did, or would have done.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    /// Whether anything was actually changed.
    pub applied: bool,
    /// Space reclaimed, or that would be.
    pub reclaimed: Size,
    /// Files removed, or that would be.
    pub files: u64,
    /// What could not be done.
    pub problems: Vec<Problem>,
}

impl Outcome {
    /// Whether everything in the plan succeeded.
    pub fn is_clean(&self) -> bool {
        self.problems.is_empty()
    }

    /// Fold another outcome into this one.
    fn absorb(&mut self, other: Self) {
        self.reclaimed += other.reclaimed;
        self.files += other.files;
        self.problems.extend(other.problems);
    }
}

/// Runs plans.
pub struct Executor {
    guard: Guard,
    walk: WalkOptions,
    apply: bool,
    exclusions: Exclusions,
}

impl Executor {
    /// An executor that will not change anything.
    pub fn dry_run(roots: &Roots) -> Self {
        Self {
            guard: Guard::new(roots),
            walk: WalkOptions::default(),
            apply: false,
            exclusions: Exclusions::default(),
        }
    }

    /// An executor that will.
    ///
    /// Spelled out at the call site on purpose; there is no boolean to get
    /// the wrong way round.
    pub fn applying(roots: &Roots) -> Self {
        Self {
            guard: Guard::new(roots),
            walk: WalkOptions::default(),
            apply: true,
            exclusions: Exclusions::default(),
        }
    }

    /// Refuse, and preserve, everything the user has excluded.
    ///
    /// The scanners already leave excluded paths out, so this is the second
    /// line: a plan built from a selection made before an exclusion was
    /// added still cannot remove what was excluded. Hiding something the
    /// user asked to keep is a courtesy; this is the guarantee.
    #[must_use]
    pub fn with_exclusions(mut self, exclusions: Exclusions) -> Self {
        self.walk.skip = exclusions.clone();
        self.exclusions = exclusions;
        self
    }

    /// Whether this executor changes anything.
    pub fn is_applying(&self) -> bool {
        self.apply
    }

    /// Carry out a plan.
    pub fn run(&self, plan: &Plan) -> Outcome {
        let mut outcome = Outcome {
            applied: self.apply,
            ..Outcome::default()
        };

        for item in &plan.items {
            outcome.absorb(self.run_item(item));
        }

        outcome
    }

    /// Carry out one item.
    fn run_item(&self, item: &Item) -> Outcome {
        let mut outcome = Outcome::default();

        // Checked here rather than trusting what the scan found. A browser
        // closed when the list was drawn may be open by the time someone
        // reads it and presses the button, and removing a live profile does
        // not free the space — it can make the browser discard the whole
        // database rather than the part that was asked for.
        if let Some(directory) = &item.requires_idle {
            if let Some(holder) = crate::browser::holder_of(directory) {
                outcome.problems.push(Problem::InUse {
                    item: item.name.clone(),
                    holder: format!("{} ({})", holder.name, holder.pid),
                });
                return outcome;
            }
        }

        for path in &item.paths {
            if self.exclusions.covers(path) {
                outcome.problems.push(Problem::Excluded(path.clone()));
                continue;
            }

            // Checked here, against the filesystem as it is now, rather than
            // when the plan was assembled — and under the permission the
            // item was created with, so a scanner's path can never be given
            // the looser check meant for something the user pointed at.
            if let Err(refusal) = self.guard.check_with(path, item.permission) {
                outcome.problems.push(Problem::Refused(refusal));
                continue;
            }

            if !path.exists() {
                continue;
            }

            outcome.absorb(self.empty(path, item.disposal));
        }

        outcome
    }

    /// Remove everything inside `directory`, leaving the directory itself.
    fn empty(&self, directory: &Path, disposal: Disposal) -> Outcome {
        let mut outcome = Outcome::default();

        let entries = match std::fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) => {
                // A plain file rather than a directory: act on it directly.
                if error.kind() == std::io::ErrorKind::NotADirectory {
                    return self.remove(directory, disposal);
                }
                outcome.problems.push(Problem::Failed {
                    path: directory.to_owned(),
                    reason: error.to_string(),
                });
                return outcome;
            }
        };

        for entry in entries.flatten() {
            outcome.absorb(self.clear(&entry.path(), disposal));
        }

        outcome
    }

    /// Remove `path`, unless something inside it is excluded.
    ///
    /// Emptying a directory removes each entry wholesale, which would take
    /// an excluded subdirectory with it. So an entry with an exclusion
    /// somewhere below is descended into instead, level by level, and only
    /// what is not excluded goes. An entry that is itself excluded is left
    /// without comment: the user asked to clean the directory around it and
    /// to keep this, and both are being honoured.
    fn clear(&self, path: &Path, disposal: Disposal) -> Outcome {
        if self.exclusions.covers(path) {
            return Outcome::default();
        }

        if !self.exclusions.inside(path) {
            return self.remove(path, disposal);
        }

        // Only a real directory is descended into. A symlink is removed as
        // a link, which cannot touch what it points at — and what is excluded
        // "inside" it lexically lives somewhere else entirely.
        let is_directory = std::fs::symlink_metadata(path)
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false);
        if !is_directory {
            return self.remove(path, disposal);
        }

        let mut outcome = Outcome::default();
        match std::fs::read_dir(path) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    outcome.absorb(self.clear(&entry.path(), disposal));
                }
            }
            Err(error) => outcome.problems.push(Problem::Failed {
                path: path.to_owned(),
                reason: error.to_string(),
            }),
        }
        outcome
    }

    /// Remove one entry, counting what it was worth.
    fn remove(&self, path: &Path, disposal: Disposal) -> Outcome {
        let mut outcome = Outcome::default();

        let Ok(metadata) = std::fs::symlink_metadata(path) else {
            return outcome;
        };

        // Measured before removal, because afterwards there is nothing to
        // measure. A directory is walked; anything else is its own size.
        let (size, files) = if metadata.is_dir() {
            match walk::measure(path, &self.walk) {
                Ok(usage) => (usage.size, usage.files),
                Err(_) => (Size::ZERO, 0),
            }
        } else {
            (Size::of(&metadata), 1)
        };

        if !self.apply {
            outcome.reclaimed = size;
            outcome.files = files;
            return outcome;
        }

        let result = match disposal {
            // The trash crate documents a thread-safety caveat on Linux, so
            // every call is made from this one thread.
            Disposal::Trash => trash::delete(path).map_err(|error| error.to_string()),
            Disposal::Delete if metadata.is_dir() => {
                // `remove_dir_all` opens with O_NOFOLLOW, so a symlink
                // planted mid-tree cannot redirect it.
                std::fs::remove_dir_all(path).map_err(|error| error.to_string())
            }
            Disposal::Delete => std::fs::remove_file(path).map_err(|error| error.to_string()),
        };

        match result {
            Ok(()) => {
                outcome.reclaimed = size;
                outcome.files = files;
            }
            Err(reason) => {
                outcome.problems.push(Problem::Failed {
                    path: path.to_owned(),
                    reason,
                });
            }
        }

        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Kind, Risk, Target};

    fn write(path: &Path, bytes: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    /// A cache with two files and a nested directory, inside a guard
    /// boundary.
    fn populated_cache() -> (tempfile::TempDir, Roots, PathBuf) {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let cache = roots.cache("thumbnails");
        write(&cache.join("a.png"), 1000);
        write(&cache.join("b.png"), 2000);
        write(&cache.join("large/c.png"), 4000);
        (fixture, roots, cache)
    }

    fn plan_for(name: &str, path: &Path, kind: Kind) -> Plan {
        Plan::from_targets(&[Target::new(name, kind, Risk::Safe).path(path)])
    }

    #[test]
    fn a_dry_run_reports_what_it_would_take_and_takes_nothing() {
        let (_fixture, roots, cache) = populated_cache();
        let plan = plan_for("thumbnails", &cache, Kind::Cache);

        let outcome = Executor::dry_run(&roots).run(&plan);

        assert!(!outcome.applied);
        assert_eq!(outcome.reclaimed.apparent, 7000);
        assert_eq!(outcome.files, 3);
        assert!(outcome.is_clean());
        assert!(cache.join("a.png").exists());
        assert!(cache.join("large/c.png").exists());
    }

    #[test]
    fn applying_empties_the_directory_but_keeps_it() {
        let (_fixture, roots, cache) = populated_cache();
        let plan = plan_for("thumbnails", &cache, Kind::Cache);

        let outcome = Executor::applying(&roots).run(&plan);

        assert!(outcome.applied);
        assert_eq!(outcome.reclaimed.apparent, 7000);
        assert!(outcome.is_clean());
        // The directory survives: applications expect their cache directory
        // to be there and misbehave quietly when it is not.
        assert!(cache.is_dir());
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0);
    }

    #[test]
    fn a_dry_run_and_a_real_run_agree_on_the_figure() {
        let (_fixture, roots, cache) = populated_cache();
        let plan = plan_for("thumbnails", &cache, Kind::Cache);

        let predicted = Executor::dry_run(&roots).run(&plan);
        let actual = Executor::applying(&roots).run(&plan);

        assert_eq!(predicted.reclaimed, actual.reclaimed);
        assert_eq!(predicted.files, actual.files);
    }

    #[test]
    fn a_path_outside_the_boundaries_is_refused_at_the_last_moment() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let precious = roots.home("Documents");
        write(&precious.join("thesis.txt"), 5000);

        // A plan that should never have been built; the executor is the
        // second opinion that catches it anyway.
        let plan = plan_for("oops", &precious, Kind::Cache);
        let outcome = Executor::applying(&roots).run(&plan);

        assert!(!outcome.is_clean());
        assert!(matches!(
            outcome.problems[0],
            Problem::Refused(Refusal::OutOfBounds(_))
        ));
        assert_eq!(outcome.reclaimed, Size::ZERO);
        assert!(precious.join("thesis.txt").exists());
    }

    #[test]
    fn emptying_the_cache_root_itself_is_refused() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        write(&roots.cache("something/keep.bin"), 100);

        let plan = plan_for("everything", &roots.cache, Kind::Cache);
        let outcome = Executor::applying(&roots).run(&plan);

        assert!(matches!(
            outcome.problems[0],
            Problem::Refused(Refusal::IsABoundary(_))
        ));
        assert!(roots.cache("something/keep.bin").exists());
    }

    #[test]
    fn a_symlink_planted_in_a_cache_is_not_followed_out() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let outside = roots.home("Documents");
        write(&outside.join("thesis.txt"), 5000);

        let cache = roots.cache("thumbnails");
        std::fs::create_dir_all(&cache).unwrap();
        write(&cache.join("real.png"), 100);
        std::os::unix::fs::symlink(&outside, cache.join("escape")).unwrap();

        let outcome = Executor::applying(&roots).run(&plan_for("t", &cache, Kind::Cache));

        assert!(outcome.is_clean());
        // The link went; what it pointed at did not.
        assert!(!cache.join("escape").exists());
        assert!(outside.join("thesis.txt").exists());
    }

    #[test]
    fn a_missing_path_is_not_a_problem() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());

        let outcome =
            Executor::applying(&roots).run(&plan_for("gone", &roots.cache("nope"), Kind::Cache));

        assert!(outcome.is_clean());
        assert_eq!(outcome.files, 0);
    }

    #[test]
    fn a_single_file_target_is_removed_rather_than_emptied() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let file = roots.cache("yay/completion.cache");
        write(&file, 2600);

        let outcome = Executor::applying(&roots).run(&plan_for("c", &file, Kind::Cache));

        assert!(outcome.is_clean());
        assert_eq!(outcome.reclaimed.apparent, 2600);
        assert!(!file.exists());
    }

    #[test]
    fn an_item_whose_directory_is_in_use_is_refused_at_the_last_moment() {
        // The scan said it was closed; by the time the button was pressed it
        // was not. This is the check the scan-time one cannot make.
        let (_fixture, roots, cache) = populated_cache();
        let plan = Plan::from_targets(&[Target::new("Brave — web cache", Kind::Cache, Risk::Safe)
            .path(&cache)
            .requires_idle(&cache)]);

        let _open = std::fs::File::open(cache.join("a.png")).unwrap();
        let outcome = Executor::applying(&roots).run(&plan);

        assert!(matches!(outcome.problems[0], Problem::InUse { .. }));
        assert_eq!(outcome.reclaimed, Size::ZERO);
        assert!(cache.join("a.png").exists());
    }

    #[test]
    fn an_idle_directory_is_cleaned_normally() {
        let (_fixture, roots, cache) = populated_cache();
        let plan = Plan::from_targets(&[Target::new("Brave — web cache", Kind::Cache, Risk::Safe)
            .path(&cache)
            .requires_idle(&cache)]);

        let outcome = Executor::applying(&roots).run(&plan);

        assert!(outcome.is_clean());
        assert_eq!(outcome.reclaimed.apparent, 7000);
    }

    #[test]
    fn a_dry_run_still_refuses_an_item_that_is_in_use() {
        // Otherwise the preview promises bytes the real run will not deliver.
        let (_fixture, roots, cache) = populated_cache();
        let plan = Plan::from_targets(&[Target::new("Brave", Kind::Cache, Risk::Safe)
            .path(&cache)
            .requires_idle(&cache)]);

        let _open = std::fs::File::open(cache.join("a.png")).unwrap();
        let outcome = Executor::dry_run(&roots).run(&plan);

        assert!(matches!(outcome.problems[0], Problem::InUse { .. }));
        assert_eq!(outcome.reclaimed, Size::ZERO);
    }

    #[test]
    fn a_chosen_file_outside_every_boundary_is_removed() {
        // The whole point of the second permission: the storage view shows
        // the disk, and nothing there is inside a boundary.
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let film = roots.home("Videos/holiday.mkv");
        write(&film, 4096);

        let plan = Plan::from_chosen(
            [(film.clone(), Size::new(4096, 4096))],
            crate::plan::Disposal::Delete,
        );
        let outcome = Executor::applying(&roots).run(&plan);

        assert!(outcome.is_clean(), "{:?}", outcome.problems);
        assert!(!film.exists());
    }

    #[test]
    fn a_chosen_path_is_still_refused_where_no_choice_should_reach() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let key = roots.home(".ssh/id_ed25519");
        write(&key, 400);

        let plan = Plan::from_chosen(
            [(key.clone(), Size::new(400, 400))],
            crate::plan::Disposal::Delete,
        );
        let outcome = Executor::applying(&roots).run(&plan);

        assert!(matches!(
            outcome.problems[0],
            Problem::Refused(Refusal::Sacred(_))
        ));
        assert!(key.exists());
    }

    #[test]
    fn a_catalogued_item_does_not_get_the_looser_check() {
        // Same path, same executor; only the permission differs.
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let film = roots.home("Videos/holiday.mkv");
        write(&film, 4096);

        let plan =
            Plan::from_targets(&[Target::new("somehow", Kind::Cache, Risk::Safe).path(&film)]);
        let outcome = Executor::applying(&roots).run(&plan);

        assert!(matches!(
            outcome.problems[0],
            Problem::Refused(Refusal::OutOfBounds(_))
        ));
        assert!(film.exists());
    }

    #[test]
    fn an_item_naming_an_excluded_path_is_refused_and_left_alone() {
        // A plan built before the exclusion was added: the scanner would not
        // offer it now, and the executor must not remove it either.
        let (_fixture, roots, cache) = populated_cache();
        let plan = plan_for("thumbnails", &cache, Kind::Cache);

        let outcome = Executor::applying(&roots)
            .with_exclusions(Exclusions::new([cache.clone()]))
            .run(&plan);

        assert!(matches!(&outcome.problems[..], [Problem::Excluded(path)] if path == &cache));
        assert_eq!(outcome.reclaimed, Size::ZERO);
        assert!(cache.join("a.png").exists());
    }

    #[test]
    fn an_exclusion_inside_a_directory_being_emptied_survives_it() {
        let (_fixture, roots, cache) = populated_cache();
        let keep = cache.join("large");

        let outcome = Executor::applying(&roots)
            .with_exclusions(Exclusions::new([keep.clone()]))
            .run(&plan_for("thumbnails", &cache, Kind::Cache));

        // Emptying removes each entry wholesale; the excluded one had to be
        // stepped around rather than swept up with the rest.
        assert!(outcome.is_clean(), "{:?}", outcome.problems);
        assert!(keep.join("c.png").exists());
        assert!(!cache.join("a.png").exists());
        assert!(!cache.join("b.png").exists());
        assert_eq!(outcome.reclaimed.apparent, 3000);
    }

    #[test]
    fn a_deeply_nested_exclusion_keeps_only_itself() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let cache = roots.cache("yay");
        write(&cache.join("brave-bin/src/brave.tar"), 900);
        write(&cache.join("brave-bin/pkg/brave.pkg"), 400);
        write(&cache.join("other/stale.tar"), 100);
        let keep = cache.join("brave-bin/src");

        let outcome = Executor::applying(&roots)
            .with_exclusions(Exclusions::new([keep.clone()]))
            .run(&plan_for("yay", &cache, Kind::BuildArtifact));

        assert!(outcome.is_clean(), "{:?}", outcome.problems);
        // Kept: the excluded directory and the one above it, which cannot
        // go while it holds something excluded.
        assert!(keep.join("brave.tar").exists());
        // Gone: every sibling at every level on the way down.
        assert!(!cache.join("brave-bin/pkg").exists());
        assert!(!cache.join("other").exists());
        assert_eq!(outcome.reclaimed.apparent, 500);
    }

    #[test]
    fn a_dry_run_and_a_real_run_agree_when_something_is_excluded() {
        let (_fixture, roots, cache) = populated_cache();
        let exclusions = Exclusions::new([cache.join("large")]);
        let plan = plan_for("thumbnails", &cache, Kind::Cache);

        let predicted = Executor::dry_run(&roots)
            .with_exclusions(exclusions.clone())
            .run(&plan);
        let actual = Executor::applying(&roots)
            .with_exclusions(exclusions)
            .run(&plan);

        assert_eq!(predicted.reclaimed, actual.reclaimed);
        assert_eq!(predicted.files, actual.files);
    }

    #[test]
    fn a_symlink_whose_lexical_inside_is_excluded_is_removed_as_a_link_only() {
        // The exclusion names a path "inside" the link, which really lives
        // wherever the link points. Removing the link cannot reach it.
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let elsewhere = roots.home("Documents");
        write(&elsewhere.join("keep.txt"), 50);
        let cache = roots.cache("thumbnails");
        write(&cache.join("a.png"), 100);
        std::os::unix::fs::symlink(&elsewhere, cache.join("link")).unwrap();

        let outcome = Executor::applying(&roots)
            .with_exclusions(Exclusions::new([cache.join("link/keep.txt")]))
            .run(&plan_for("thumbnails", &cache, Kind::Cache));

        assert!(outcome.is_clean(), "{:?}", outcome.problems);
        assert!(!cache.join("link").exists());
        assert!(elsewhere.join("keep.txt").exists());
    }

    #[test]
    fn a_chosen_file_that_is_excluded_is_refused() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let film = roots.home("Videos/keep.mkv");
        write(&film, 4096);

        let plan = Plan::from_chosen(
            [(film.clone(), Size::new(4096, 4096))],
            crate::plan::Disposal::Delete,
        );
        let outcome = Executor::applying(&roots)
            .with_exclusions(Exclusions::new([film.clone()]))
            .run(&plan);

        assert!(matches!(outcome.problems[0], Problem::Excluded(_)));
        assert!(film.exists());
    }

    #[test]
    fn an_empty_plan_does_nothing_and_says_so() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());

        let outcome = Executor::applying(&roots).run(&Plan::new());

        assert!(outcome.is_clean());
        assert_eq!(outcome.reclaimed, Size::ZERO);
    }

    #[test]
    fn one_refused_path_does_not_stop_the_rest_of_the_plan() {
        let (_fixture, roots, cache) = populated_cache();
        let plan = Plan::from_targets(&[
            Target::new("outside", Kind::Cache, Risk::Safe).path(roots.home("Documents")),
            Target::new("thumbnails", Kind::Cache, Risk::Safe).path(&cache),
        ]);

        let outcome = Executor::applying(&roots).run(&plan);

        assert_eq!(outcome.problems.len(), 1);
        assert_eq!(outcome.reclaimed.apparent, 7000);
    }
}
