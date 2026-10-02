//! Privileged helper for Limpid.
//!
//! Runs as root through polkit. It reads a [`Request`] on stdin, refuses
//! anything it does not recognise, carries out the operations, and writes a
//! [`Report`] on stdout.
//!
//! It is deliberately small, and the design is what keeps it small: **no
//! path crosses the boundary.** The request is a fixed set of named
//! operations with bounded parameters, so there is nothing here that parses
//! a filename, resolves a symlink, or decides whether a directory is safe.
//! Those are the parts that go wrong, and they all live in the unprivileged
//! side where being wrong is survivable.
//!
//! Where a supported tool exists it is run rather than reimplemented.
//! `paccache` is maintained alongside pacman and knows what a correct
//! retention policy is; a version of it in here would be a second, worse
//! opinion.

#![forbid(unsafe_code)]

use std::io::{self, Read, Write};
use std::path::Path;
use std::process::Command;

use limpid_core::privileged::{Completed, Operation, Report, Request};

/// Where core dumps are kept. Fixed, not taken from the request.
const COREDUMP_DIRECTORY: &str = "/var/lib/systemd/coredump";

fn main() -> std::process::ExitCode {
    let mut input = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut input) {
        return fail(&format!("could not read the request: {error}"));
    }

    let request: Request = match serde_json::from_str(&input) {
        Ok(request) => request,
        Err(error) => return fail(&format!("could not understand the request: {error}")),
    };

    // Validated here as well as by the caller. The caller's check is a
    // courtesy; this one is the one that counts, because this is the side
    // running as root.
    if let Err(error) = request.validate() {
        return fail(&format!("refused: {error}"));
    }

    let report = Report {
        completed: request.operations.into_iter().map(perform).collect(),
    };

    let encoded = match serde_json::to_string(&report) {
        Ok(encoded) => encoded,
        Err(error) => return fail(&format!("could not encode the report: {error}")),
    };
    if let Err(error) = writeln!(io::stdout(), "{encoded}") {
        return fail(&format!("could not write the report: {error}"));
    }

    if report.is_clean() {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}

/// Report a problem that stopped the helper before it could do anything.
fn fail(message: &str) -> std::process::ExitCode {
    eprintln!("limpid-helper: {message}");
    std::process::ExitCode::FAILURE
}

/// Carry out one operation.
fn perform(operation: Operation) -> Completed {
    let outcome = match operation {
        Operation::TrimPackageCache { keep } => trim_package_cache(keep),
        Operation::VacuumJournal { days } => {
            run("journalctl", &[&format!("--vacuum-time={days}d")])
        }
        Operation::RemoveCoredumps => remove_coredumps(),
        Operation::PruneDocker => {
            // No --volumes, ever. A volume can be the only copy of a
            // development database.
            run("docker", &["system", "prune", "--force"])
        }
    };

    match outcome {
        Ok(detail) => Completed {
            operation,
            succeeded: true,
            detail,
        },
        Err(detail) => Completed {
            operation,
            succeeded: false,
            detail,
        },
    }
}

/// Trim the package cache in the two passes `paccache` documents.
fn trim_package_cache(keep: u8) -> Result<String, String> {
    // Installed packages keep their newest few versions, which is what makes
    // a downgrade possible. Uninstalled ones keep nothing, because there is
    // nothing to downgrade to.
    let installed = run("paccache", &["--remove", &format!("--keep={keep}")])?;
    let uninstalled = run("paccache", &["--remove", "--uninstalled", "--keep=0"])?;

    Ok(format!("{}\n{}", installed.trim(), uninstalled.trim())
        .trim()
        .to_owned())
}

/// Remove the files in the coredump directory.
///
/// The one operation that touches files directly, and the path is a
/// constant: nothing the caller sends can influence it.
fn remove_coredumps() -> Result<String, String> {
    remove_files_in(Path::new(COREDUMP_DIRECTORY))
}

/// Remove the regular files directly inside `directory`, and nothing else.
///
/// No recursion, so a directory planted there leads nowhere. And symlinks
/// are **skipped** — neither followed nor removed. `DirEntry::metadata`
/// does not traverse a link, so a link reports itself as a link, and
/// `is_file` is false for it. Split out from [`remove_coredumps`] only so
/// that this is tested rather than asserted: the project roadmap once
/// claimed the opposite, from a reading of an older comment here.
#[expect(
    clippy::disallowed_methods,
    reason = "the helper's only removal: regular files directly inside a \
              directory fixed at compile time, with links skipped"
)]
fn remove_files_in(directory: &Path) -> Result<String, String> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok("nothing stored".to_owned());
        }
        Err(error) => return Err(format!("{}: {error}", directory.display())),
    };

    let mut removed = 0_u32;
    let mut failed = Vec::new();

    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        match std::fs::remove_file(entry.path()) {
            Ok(()) => removed += 1,
            Err(error) => failed.push(format!("{}: {error}", entry.path().display())),
        }
    }

    if failed.is_empty() {
        Ok(format!(
            "removed {removed} dump{}",
            if removed == 1 { "" } else { "s" }
        ))
    } else {
        Err(failed.join("; "))
    }
}

/// Run a command and collect what it said.
#[expect(
    clippy::disallowed_methods,
    reason = "runs only the programs `perform` names as constants, with the \
              environment cleared and PATH fixed before anything starts"
)]
fn run(program: &str, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(arguments)
        .env_clear()
        // A minimal PATH rather than the caller's: this runs as root, and
        // inheriting an environment across a privilege boundary is how a
        // helper ends up running someone else's `paccache`.
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("LC_ALL", "C")
        .output()
        .map_err(|error| format!("could not run {program}: {error}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();

    if output.status.success() {
        Ok(if stdout.is_empty() { stderr } else { stdout })
    } else {
        Err(if stderr.is_empty() { stdout } else { stderr })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_regular_files_directly_inside_are_removed() {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("core.a.zst"), b"x").unwrap();
        std::fs::write(directory.path().join("core.b.zst"), b"x").unwrap();
        std::fs::create_dir(directory.path().join("nested")).unwrap();
        std::fs::write(directory.path().join("nested/core.c.zst"), b"x").unwrap();
        std::fs::write(outside.path().join("precious"), b"x").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("precious"),
            directory.path().join("core.link.zst"),
        )
        .unwrap();

        let said = remove_files_in(directory.path()).unwrap();

        assert_eq!(said, "removed 2 dumps");
        // No recursion into a directory planted there.
        assert!(directory.path().join("nested/core.c.zst").exists());
        // The link is skipped: not removed, and not followed to its target.
        assert!(directory.path().join("core.link.zst").exists());
        assert!(outside.path().join("precious").exists());
    }

    #[test]
    fn a_directory_that_does_not_exist_has_nothing_stored() {
        assert_eq!(
            remove_files_in(Path::new("/definitely/not/here")).unwrap(),
            "nothing stored"
        );
    }

    #[test]
    fn a_command_that_does_not_exist_is_an_error_rather_than_a_panic() {
        let result = run("limpid-no-such-program", &[]);

        assert!(result.unwrap_err().contains("could not run"));
    }

    #[test]
    fn a_failing_command_reports_what_it_said() {
        let result = run("false", &[]);

        assert!(result.is_err());
    }

    #[test]
    fn a_successful_command_reports_its_output() {
        assert_eq!(run("echo", &["trimmed"]).unwrap(), "trimmed");
    }

    #[test]
    fn the_environment_does_not_cross_the_boundary() {
        // A helper that inherited PATH could be made to run the caller's
        // paccache instead of the system one, as root.
        let path = run("sh", &["-c", "echo \"$PATH\""]).unwrap();
        assert_eq!(path, "/usr/bin:/bin:/usr/sbin:/sbin");

        // And nothing else comes across either: HOME is always set in the
        // environment this test runs in, and must not be in that one.
        assert!(
            std::env::var_os("HOME").is_some(),
            "the test itself has a HOME"
        );
        let home = run("sh", &["-c", "echo \"${HOME:-absent}\""]).unwrap();
        assert_eq!(home, "absent");
    }

    #[test]
    fn output_is_read_from_stderr_when_a_command_says_nothing_on_stdout() {
        let said = run("sh", &["-c", "echo complaint >&2"]).unwrap();

        assert_eq!(said, "complaint");
    }
}
