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

use crate::guard::{Guard, Refusal};
use crate::paths::Roots;
use crate::plan::{Disposal, Item, Plan};
use crate::size::Size;
use crate::walk::{self, WalkOptions};

/// Something that could not be done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
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
}

impl Executor {
    /// An executor that will not change anything.
    pub fn dry_run(roots: &Roots) -> Self {
        Self {
            guard: Guard::new(roots),
            walk: WalkOptions::default(),
            apply: false,
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
        }
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

        for path in &item.paths {
            // Checked here, against the filesystem as it is now, rather than
            // when the plan was assembled.
            if let Err(refusal) = self.guard.check(path) {
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
            outcome.absorb(self.remove(&entry.path(), disposal));
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
