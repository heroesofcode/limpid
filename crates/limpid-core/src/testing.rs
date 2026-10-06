//! What the tests share.

use std::path::{Path, PathBuf};

use crate::paths::Roots;

/// Set to the fixture in a child process whose trash is that fixture's.
const TRASH_FIXTURE: &str = "LIMPID_TEST_TRASH_FIXTURE";

/// Run the test called `name` in a child process whose trash is a fixture's.
///
/// The trash crate finds the trash through `XDG_DATA_HOME` and nothing
/// else, so a test that really trashes something has to run where that
/// points at a fixture. Changing it here would change it for every test
/// running alongside; a child process has its own.
pub fn in_a_fixture_trash(name: &str) {
    let fixture = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--include-ignored", "--nocapture"])
        .env(TRASH_FIXTURE, fixture.path())
        .env("XDG_DATA_HOME", Roots::under(fixture.path()).data)
        .output()
        .unwrap();

    let said = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{said}");
    assert!(said.contains("1 passed"), "{said}");
}

/// The roots whose trash this process has, when it is a child started by
/// [`in_a_fixture_trash`]; `None` in an ordinary test run.
///
/// Whatever started it, a test that gets roots from here does not get the
/// real trash: the trash it would use has to be the fixture's.
pub fn fixture_trash() -> Option<Roots> {
    let fixture = std::env::var_os(TRASH_FIXTURE)?;
    let roots = Roots::under(Path::new(&fixture));
    assert_eq!(
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        Some(roots.data.clone()),
        "a test meant for a fixture's trash, about to use another one",
    );
    Some(roots)
}

/// A file of `bytes` bytes, and the folders above it.
pub fn write(path: &Path, bytes: usize) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, vec![b'x'; bytes]).unwrap();
}
