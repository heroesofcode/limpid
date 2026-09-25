//! The boundary between the application and root.
//!
//! **No path crosses this boundary.** The helper takes a fixed set of named
//! operations with bounded parameters, never a list of files, so the worst a
//! compromised or buggy application can ask for is an operation that was
//! going to be offered anyway. There is no request that means "remove this
//! path", which means there is no request that can be made to mean
//! "remove that one".
//!
//! Where a supported tool exists, the operation runs it rather than
//! reimplementing it. `paccache` knows what a safe retention policy is and is
//! maintained alongside pacman; a hand-rolled version of it in here would be
//! a second, worse opinion on a question that has a right answer.

use std::fmt;

/// Something the helper can be asked to do.
///
/// Every variant is a whole policy, not a step. A caller cannot compose
/// these into something none of them means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "operation", rename_all = "kebab-case")]
pub enum Operation {
    /// Trim the pacman package cache, keeping the newest `keep` versions of
    /// each installed package and nothing at all for uninstalled ones.
    TrimPackageCache {
        /// Versions of each installed package to keep.
        keep: u8,
    },
    /// Discard journal entries older than `days`.
    VacuumJournal {
        /// How much history to keep.
        days: u16,
    },
    /// Remove stored core dumps.
    RemoveCoredumps,
    /// Remove Docker build cache, dangling images and stopped containers.
    ///
    /// Never volumes. A volume can be the only copy of a development
    /// database, and no amount of space is worth guessing that it is not.
    PruneDocker,
}

/// Why an operation was not acceptable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Invalid {
    /// A parameter was outside the range the operation permits.
    #[error("{operation}: {reason}")]
    Parameter {
        /// Which operation.
        operation: &'static str,
        /// What was wrong with it.
        reason: String,
    },
}

/// The fewest package versions worth keeping.
///
/// One is the installed version itself; keeping only that leaves no
/// downgrade, which on a rolling release is a real recovery path.
pub const MINIMUM_KEEP: u8 = 1;
/// The most versions it is worth offering to keep.
pub const MAXIMUM_KEEP: u8 = 10;
/// The least journal history worth keeping.
///
/// Below two weeks the journal stops being able to answer "what changed
/// before this started happening", which is most of what it is for.
pub const MINIMUM_DAYS: u16 = 7;
/// The most history the helper will be asked to keep.
pub const MAXIMUM_DAYS: u16 = 3650;

impl Operation {
    /// A short name, for logs and errors.
    pub fn name(self) -> &'static str {
        match self {
            Self::TrimPackageCache { .. } => "trim-package-cache",
            Self::VacuumJournal { .. } => "vacuum-journal",
            Self::RemoveCoredumps => "remove-coredumps",
            Self::PruneDocker => "prune-docker",
        }
    }

    /// What this will do, in a sentence, for the confirmation.
    pub fn describe(self) -> String {
        match self {
            Self::TrimPackageCache { keep } => format!(
                "Keep the {keep} newest version{} of each installed package and \
                 discard the rest",
                if keep == 1 { "" } else { "s" },
            ),
            Self::VacuumJournal { days } => {
                format!("Discard system log entries older than {days} days")
            }
            Self::RemoveCoredumps => "Remove stored crash dumps".to_owned(),
            Self::PruneDocker => {
                "Remove Docker build cache, dangling images and stopped containers, \
                 leaving volumes alone"
                    .to_owned()
            }
        }
    }

    /// Check the parameters before anything is run with them.
    ///
    /// Called by the caller *and* by the helper. The helper's copy is the one
    /// that matters; the caller's is only there to fail earlier.
    pub fn validate(self) -> Result<(), Invalid> {
        match self {
            Self::TrimPackageCache { keep } if !(MINIMUM_KEEP..=MAXIMUM_KEEP).contains(&keep) => {
                Err(Invalid::Parameter {
                    operation: self.name(),
                    reason: format!(
                        "keep must be between {MINIMUM_KEEP} and {MAXIMUM_KEEP}, not {keep}"
                    ),
                })
            }
            Self::VacuumJournal { days } if !(MINIMUM_DAYS..=MAXIMUM_DAYS).contains(&days) => {
                Err(Invalid::Parameter {
                    operation: self.name(),
                    reason: format!(
                        "days must be between {MINIMUM_DAYS} and {MAXIMUM_DAYS}, not {days}"
                    ),
                })
            }
            _ => Ok(()),
        }
    }
}

impl fmt::Display for Operation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// What the application asks the helper to do.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Request {
    /// The operations, in order.
    pub operations: Vec<Operation>,
}

impl Request {
    /// Check every operation.
    pub fn validate(&self) -> Result<(), Invalid> {
        self.operations
            .iter()
            .try_for_each(|operation| operation.validate())
    }
}

/// How one operation went.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Completed {
    /// Which operation.
    pub operation: Operation,
    /// Whether it succeeded.
    pub succeeded: bool,
    /// What it said, trimmed to something printable.
    pub detail: String,
}

/// What the helper did.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Report {
    /// One entry per operation attempted.
    pub completed: Vec<Completed>,
}

impl Report {
    /// Whether everything worked.
    pub fn is_clean(&self) -> bool {
        self.completed.iter().all(|entry| entry.succeeded)
    }
}

/// Where the helper is installed.
pub const INSTALLED_HELPER: &str = "/usr/lib/limpid/limpid-helper";
/// Environment variable that points at a helper somewhere else, for
/// development. Ignored by the helper itself, which has no say in where it
/// is; it only changes which binary the *caller* asks polkit to run, and
/// polkit will refuse an unregistered one.
pub const HELPER_OVERRIDE: &str = "LIMPID_HELPER";

/// Asking the helper to do something.
#[derive(Debug, Clone)]
pub struct Runner {
    helper: std::path::PathBuf,
    elevate: bool,
}

/// Why a privileged run did not happen.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// The request was not acceptable, so it was never sent.
    #[error(transparent)]
    Invalid(#[from] Invalid),
    /// The helper could not be started.
    #[error("could not start the helper: {0}")]
    Unstartable(String),
    /// The user dismissed the authentication dialogue.
    #[error("authorisation was declined")]
    Declined,
    /// The helper ran but said something unintelligible.
    #[error("the helper's reply could not be read: {0}")]
    Unreadable(String),
}

impl Runner {
    /// A runner that asks polkit to elevate.
    pub fn new() -> Self {
        Self {
            helper: Self::locate(),
            elevate: true,
        }
    }

    /// A runner that invokes the helper directly, for when this process is
    /// already root, and for tests.
    pub fn direct(helper: impl Into<std::path::PathBuf>) -> Self {
        Self {
            helper: helper.into(),
            elevate: false,
        }
    }

    /// The helper binary this runner will invoke.
    pub fn helper(&self) -> &std::path::Path {
        &self.helper
    }

    /// Where the helper is.
    fn locate() -> std::path::PathBuf {
        if let Some(override_path) = std::env::var_os(HELPER_OVERRIDE) {
            return override_path.into();
        }
        std::path::PathBuf::from(INSTALLED_HELPER)
    }

    /// Carry out a request, elevating if this runner was built to.
    ///
    /// The request is validated here before anything is spawned, so an
    /// impossible request never raises an authentication prompt. The helper
    /// validates it again, and that copy is the one that matters.
    pub fn run(&self, request: &Request) -> Result<Report, RunError> {
        use std::io::Write;
        use std::process::{Command, Stdio};

        request.validate()?;

        let mut command = if self.elevate {
            let mut command = Command::new("pkexec");
            command.arg(&self.helper);
            command
        } else {
            Command::new(&self.helper)
        };

        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| RunError::Unstartable(error.to_string()))?;

        let encoded = serde_json::to_string(request)
            .map_err(|error| RunError::Unstartable(error.to_string()))?;
        child
            .stdin
            .take()
            .ok_or_else(|| RunError::Unstartable("no stdin".to_owned()))?
            .write_all(encoded.as_bytes())
            .map_err(|error| RunError::Unstartable(error.to_string()))?;

        let output = child
            .wait_with_output()
            .map_err(|error| RunError::Unstartable(error.to_string()))?;

        // pkexec exits 126 when the user dismisses the dialogue and 127 when
        // it cannot be shown at all. Neither is a failure of the request,
        // and neither should be reported as one.
        if matches!(output.status.code(), Some(126 | 127)) && self.elevate {
            return Err(RunError::Declined);
        }

        serde_json::from_slice(&output.stdout).map_err(|error| {
            let said = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            RunError::Unreadable(if said.is_empty() {
                error.to_string()
            } else {
                said
            })
        })
    }
}

impl Default for Runner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeping_no_package_versions_is_refused() {
        // Zero would leave no downgrade path, which on a rolling release is
        // a real way to recover from a bad update.
        assert!(Operation::TrimPackageCache { keep: 0 }.validate().is_err());
        assert!(Operation::TrimPackageCache { keep: 1 }.validate().is_ok());
        assert!(Operation::TrimPackageCache { keep: 3 }.validate().is_ok());
        assert!(
            Operation::TrimPackageCache { keep: 200 }
                .validate()
                .is_err()
        );
    }

    #[test]
    fn vacuuming_the_journal_to_nothing_is_refused() {
        assert!(Operation::VacuumJournal { days: 0 }.validate().is_err());
        assert!(Operation::VacuumJournal { days: 1 }.validate().is_err());
        assert!(Operation::VacuumJournal { days: 7 }.validate().is_ok());
        assert!(Operation::VacuumJournal { days: 30 }.validate().is_ok());
    }

    #[test]
    fn a_request_is_only_as_valid_as_its_worst_operation() {
        let request = Request {
            operations: vec![
                Operation::RemoveCoredumps,
                Operation::TrimPackageCache { keep: 0 },
            ],
        };

        assert!(request.validate().is_err());
    }

    #[test]
    fn operations_survive_the_round_trip_the_helper_reads_them_through() {
        let request = Request {
            operations: vec![
                Operation::TrimPackageCache { keep: 3 },
                Operation::VacuumJournal { days: 14 },
                Operation::RemoveCoredumps,
                Operation::PruneDocker,
            ],
        };

        let encoded = serde_json::to_string(&request).unwrap();
        let decoded: Request = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, request);
    }

    #[test]
    fn a_request_cannot_carry_a_path() {
        // The point of the whole boundary: there is no field to put one in,
        // so an encoded request that mentions a path decodes to nothing.
        let hostile = r#"{"operations":[{"operation":"remove-coredumps","path":"/etc"}]}"#;
        let decoded: Request = serde_json::from_str(hostile).unwrap();

        assert_eq!(decoded.operations, vec![Operation::RemoveCoredumps]);
    }

    #[test]
    fn an_unknown_operation_does_not_decode() {
        let hostile = r#"{"operations":[{"operation":"run-anything"}]}"#;

        assert!(serde_json::from_str::<Request>(hostile).is_err());
    }

    #[test]
    fn every_operation_can_say_what_it_will_do() {
        for operation in [
            Operation::TrimPackageCache { keep: 3 },
            Operation::VacuumJournal { days: 14 },
            Operation::RemoveCoredumps,
            Operation::PruneDocker,
        ] {
            assert!(!operation.describe().is_empty());
            assert!(!operation.name().is_empty());
        }
    }

    #[test]
    fn an_invalid_request_never_reaches_the_helper() {
        // No prompt, no process: the failure happens before anything is
        // spawned, which is why the nonexistent helper path does not matter.
        let runner = Runner::direct("/definitely/not/a/helper");
        let request = Request {
            operations: vec![Operation::TrimPackageCache { keep: 0 }],
        };

        assert!(matches!(
            runner.run(&request).unwrap_err(),
            RunError::Invalid(_)
        ));
    }

    #[test]
    fn a_missing_helper_is_reported_rather_than_panicking() {
        let runner = Runner::direct("/definitely/not/a/helper");
        let request = Request {
            operations: vec![Operation::RemoveCoredumps],
        };

        assert!(matches!(
            runner.run(&request).unwrap_err(),
            RunError::Unstartable(_)
        ));
    }

    #[test]
    fn the_helper_location_can_be_overridden_for_development() {
        // Not for the helper's benefit: polkit will refuse to elevate a
        // binary it has no policy for, so this only helps when running the
        // helper directly.
        assert_eq!(
            Runner::direct("/tmp/helper").helper(),
            std::path::Path::new("/tmp/helper")
        );
    }

    #[test]
    fn a_report_is_clean_only_if_everything_worked() {
        let entry = |succeeded| Completed {
            operation: Operation::RemoveCoredumps,
            succeeded,
            detail: String::new(),
        };

        assert!(
            Report {
                completed: vec![entry(true)]
            }
            .is_clean()
        );
        assert!(
            !Report {
                completed: vec![entry(true), entry(false)]
            }
            .is_clean()
        );
        assert!(Report::default().is_clean());
    }
}
