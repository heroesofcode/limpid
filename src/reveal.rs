//! Handing a file to the rest of the desktop.
//!
//! Two ways, tried in order. `org.freedesktop.FileManager1` opens the folder
//! *and selects the file*, which is what someone looking at a 4 GB entry
//! actually wants; Nautilus, Dolphin, Thunar and Nemo all implement it.
//! Where nothing does, `xdg-open` on the parent directory is the fallback —
//! it always works, including inside a Flatpak, where the runtime's
//! `xdg-open` routes through the portal.

use std::path::Path;

/// Show a file in the desktop's file manager.
///
/// Best effort. There is no useful way to report "no file manager is
/// installed" to someone who just clicked a small button, and nothing is
/// lost by the attempt.
pub fn in_file_manager(path: &Path) {
    if show_and_select(path).is_ok() {
        return;
    }

    let Some(parent) = path.parent() else {
        return;
    };

    if let Err(error) = std::process::Command::new("xdg-open").arg(parent).spawn() {
        tracing::debug!(%error, "could not open the containing folder");
    }
}

/// Ask the file manager to reveal the file with it selected.
fn show_and_select(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let uri = format!("file://{}", path.display());

    let connection = zbus::blocking::Connection::session()?;
    connection.call_method(
        Some("org.freedesktop.FileManager1"),
        "/org/freedesktop/FileManager1",
        Some("org.freedesktop.FileManager1"),
        "ShowItems",
        // The second argument is a startup id, which we do not have.
        &(vec![uri], ""),
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_with_no_parent_does_not_panic() {
        // Root has no containing folder. Nothing to do, and nothing to
        // crash over.
        in_file_manager(Path::new("/"));
    }

    #[test]
    fn revealing_without_a_session_bus_falls_through_quietly() {
        // Runs in CI, where there is neither a bus nor a file manager. The
        // point is that it returns.
        in_file_manager(Path::new("/tmp"));
    }
}
