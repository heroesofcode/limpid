//! Palette resolution for Limpid.
//!
//! Limpid follows the desktop's theme when there is one to follow and looks
//! like itself when there is not. There are three sources, tried in order:
//!
//! 1. **Omarchy.** The full palette, read from the staged theme, with live
//!    updates when the theme changes.
//! 2. **The freedesktop appearance setting.** Light or dark only, but it is
//!    what every other desktop exposes, and it also changes live.
//! 3. **Built in.** Limpid's own palette.
//!
//! Everything downstream sees a [`Palette`] and cannot tell which of the
//! three produced it.
//!
//! ```no_run
//! let theme = limpid_theme::Theme::detect();
//! println!("{} via {}", theme.palette.background, theme.source.describe());
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod appearance;
pub mod color;
pub mod omarchy;
pub mod palette;
pub mod watch;

pub use color::Color;
pub use palette::{Mode, Palette};

/// Where the active palette came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// An Omarchy theme, named if the name could be read.
    Omarchy(Option<String>),
    /// The desktop's light/dark preference, with Limpid's own colours.
    SystemAppearance,
    /// Nothing to follow.
    Builtin,
}

impl Source {
    /// A phrase for the about screen or a log line.
    pub fn describe(&self) -> String {
        match self {
            Self::Omarchy(Some(name)) => format!("the Omarchy theme {name}"),
            Self::Omarchy(None) => "the active Omarchy theme".to_owned(),
            Self::SystemAppearance => "the desktop appearance setting".to_owned(),
            Self::Builtin => "Limpid's own palette".to_owned(),
        }
    }

    /// Whether this source pushes updates of its own.
    pub fn is_live(&self) -> bool {
        !matches!(self, Self::Builtin)
    }
}

/// The palette in force, and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    /// The colours to draw with.
    pub palette: Palette,
    /// What produced them.
    pub source: Source,
}

impl Theme {
    /// Work out which palette to use on this machine.
    pub fn detect() -> Self {
        Self::detect_with(&omarchy::Locations::standard())
    }

    /// Work out which palette to use, given where to look for Omarchy.
    pub fn detect_with(locations: &omarchy::Locations) -> Self {
        if let Some(palette) = locations.load() {
            return Self {
                palette,
                source: Source::Omarchy(locations.theme_name()),
            };
        }

        match appearance::preferred_mode() {
            Some(mode) => Self {
                palette: Palette::builtin(mode),
                source: Source::SystemAppearance,
            },
            None => Self {
                palette: Palette::default(),
                source: Source::Builtin,
            },
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            palette: Palette::default(),
            source: Source::Builtin,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_staged_omarchy_theme_wins() {
        let fixture = tempfile::tempdir().unwrap();
        let locations = omarchy::Locations::under(fixture.path().join("current"));
        std::fs::create_dir_all(locations.theme()).unwrap();
        std::fs::write(
            locations.colors(),
            "mode = \"dark\"\nbackground = \"#1a1b26\"\n",
        )
        .unwrap();
        std::fs::write(locations.name_file(), "tokyo-night\n").unwrap();

        let theme = Theme::detect_with(&locations);

        assert_eq!(
            theme.source,
            Source::Omarchy(Some("tokyo-night".to_owned()))
        );
        assert_eq!(theme.palette.background.to_string(), "#1a1b26");
        assert!(theme.source.is_live());
    }

    #[test]
    fn without_omarchy_the_palette_is_one_of_our_own() {
        let fixture = tempfile::tempdir().unwrap();
        let locations = omarchy::Locations::under(fixture.path().join("current"));

        let theme = Theme::detect_with(&locations);

        assert_ne!(theme.source, Source::Omarchy(None));
        // Whichever of the two remaining sources applied, the colours are the
        // built-in ones for that mode.
        assert_eq!(theme.palette, Palette::builtin(theme.palette.mode));
    }

    #[test]
    fn each_source_describes_itself_readably() {
        assert_eq!(
            Source::Omarchy(Some("tokyo-night".to_owned())).describe(),
            "the Omarchy theme tokyo-night"
        );
        assert!(!Source::Builtin.is_live());
    }
}
