//! Noticing that the theme changed.
//!
//! `omarchy-theme-set` stages the new theme in a sibling directory and then
//! does `rm -rf current/theme && mv next-theme current/theme`. That is the
//! whole difficulty: a watch on `colors.toml`, or on `current/theme` itself,
//! follows an inode that stops existing the first time the theme changes, and
//! then silently never fires again. The fix is to watch the *parent*.
//!
//! Omarchy broadcasts nothing over D-Bus for themes, so there is no signal to
//! subscribe to instead.

use std::path::Path;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use notify::{RecursiveMode, Watcher as _};

use crate::omarchy::Locations;
use crate::palette::Palette;

/// How long to wait for the rest of a burst of events before reloading.
///
/// A theme change produces the directory swap and the name-file write back to
/// back; reacting to the first would read a directory the second is still
/// finishing.
const SETTLE: Duration = Duration::from_millis(150);

/// A handle that yields a new palette each time the theme changes.
///
/// Dropping it stops the watch.
pub struct Watcher {
    updates: Receiver<Palette>,
    _inner: notify::RecommendedWatcher,
}

impl Watcher {
    /// The next palette, if one has arrived.
    pub fn try_next(&self) -> Option<Palette> {
        // Drain the queue: only the most recent palette is worth having.
        let mut latest = None;
        while let Ok(palette) = self.updates.try_recv() {
            latest = Some(palette);
        }
        latest
    }

    /// Block until the theme changes, or until `timeout` elapses.
    pub fn next_timeout(&self, timeout: Duration) -> Option<Palette> {
        self.updates.recv_timeout(timeout).ok()
    }
}

/// Watch the Omarchy state directory and report each new palette.
///
/// Fails when the directory does not exist, which is the ordinary case off
/// Omarchy — callers should treat that as "nothing to follow" rather than as
/// an error worth showing.
pub fn watch(locations: Locations) -> notify::Result<Watcher> {
    let (raw_tx, raw_rx) = channel();
    let (palette_tx, palette_rx) = channel();

    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = raw_tx.send(event);
    })?;

    // Non-recursive, and on `current/` rather than on `current/theme/`: the
    // events that matter are a directory being moved into place and a file
    // being rewritten, both of which happen *in* this directory.
    watcher.watch(&locations.current, RecursiveMode::NonRecursive)?;

    std::thread::Builder::new()
        .name("limpid-theme-watch".to_owned())
        .spawn(move || {
            while let Ok(first) = raw_rx.recv() {
                if !is_theme_change(&first) {
                    continue;
                }

                // Swallow the rest of the burst: keep reading until the
                // directory has been quiet for a moment, so the reload sees
                // the finished state rather than a half-applied one.
                while raw_rx.recv_timeout(SETTLE).is_ok() {}

                if let Some(palette) = locations.load() {
                    if palette_tx.send(palette).is_err() {
                        break;
                    }
                }
            }
        })
        .map_err(notify::Error::io)?;

    Ok(Watcher {
        updates: palette_rx,
        _inner: watcher,
    })
}

/// Whether an event touched the staged theme or the name file.
fn is_theme_change(event: &notify::Result<notify::Event>) -> bool {
    let Ok(event) = event else {
        return false;
    };
    event.paths.iter().any(|path| is_theme_path(path))
}

/// Whether a path is one of the two entries a theme change rewrites.
fn is_theme_path(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("theme" | "theme.name")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const TOKYO: &str = r##"
mode = "dark"
background = "#1a1b26"
foreground = "#a9b1d6"
accent = "#7aa2f7"
"##;

    const LATTE: &str = r##"
mode = "light"
background = "#eff1f5"
foreground = "#4c4f69"
accent = "#1e66f5"
"##;

    #[test]
    fn only_the_staged_theme_and_its_name_count_as_changes() {
        assert!(is_theme_path(&PathBuf::from("/s/omarchy/current/theme")));
        assert!(is_theme_path(&PathBuf::from(
            "/s/omarchy/current/theme.name"
        )));

        // The background symlink is rewritten by the same command and is not
        // a palette change.
        assert!(!is_theme_path(&PathBuf::from(
            "/s/omarchy/current/background"
        )));
        assert!(!is_theme_path(&PathBuf::from(
            "/s/omarchy/current/next-theme"
        )));
    }

    #[test]
    fn watching_a_directory_that_does_not_exist_fails_rather_than_hanging() {
        let locations = Locations::under("/definitely/not/here");
        assert!(watch(locations).is_err());
    }

    #[test]
    fn a_theme_swap_produces_the_new_palette() {
        let fixture = tempfile::tempdir().unwrap();
        let current = fixture.path().join("current");
        let locations = Locations::under(&current);

        // Stage an initial theme the way omarchy-theme-set leaves things.
        std::fs::create_dir_all(locations.theme()).unwrap();
        std::fs::write(locations.colors(), TOKYO).unwrap();
        std::fs::write(locations.name_file(), "tokyo-night\n").unwrap();

        let watcher = watch(locations.clone()).unwrap();

        // Reproduce the swap exactly: build a sibling, remove, move in.
        // Watching the theme directory itself would stop working right here.
        let next = current.join("next-theme");
        std::fs::create_dir_all(&next).unwrap();
        std::fs::write(next.join("colors.toml"), LATTE).unwrap();
        std::fs::remove_dir_all(locations.theme()).unwrap();
        std::fs::rename(&next, locations.theme()).unwrap();
        std::fs::write(locations.name_file(), "catppuccin-latte\n").unwrap();

        let palette = watcher
            .next_timeout(Duration::from_secs(10))
            .expect("the swap should have produced a palette");

        assert_eq!(palette.background.to_string(), "#eff1f5");
        assert_eq!(locations.theme_name().as_deref(), Some("catppuccin-latte"));
    }

    #[test]
    fn an_unrelated_write_in_the_directory_produces_nothing() {
        let fixture = tempfile::tempdir().unwrap();
        let current = fixture.path().join("current");
        let locations = Locations::under(&current);
        std::fs::create_dir_all(locations.theme()).unwrap();
        std::fs::write(locations.colors(), TOKYO).unwrap();

        let watcher = watch(locations).unwrap();
        std::fs::write(current.join("background"), "somewhere.png").unwrap();

        assert!(watcher.next_timeout(Duration::from_millis(600)).is_none());
    }
}
