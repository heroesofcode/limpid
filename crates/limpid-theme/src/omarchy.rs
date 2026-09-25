//! Reading the active Omarchy theme.
//!
//! The palette lives in one file — `colors.toml` inside the staged theme —
//! but reading it correctly means reproducing the alias-and-derive cascade
//! that `omarchy-theme-color` applies, because a theme is allowed to define
//! only part of the palette and let the rest be worked out. Third-party
//! generated themes routinely do.
//!
//! Two things are easy to get wrong and are handled here deliberately.
//!
//! The staged theme is resolved through `~/.local/state/omarchy/current/theme`
//! and not through the theme's *source* directory. `omarchy theme dir <name>`
//! points at the source, which may not contain a `colors.toml` at all.
//!
//! The staged directory is replaced wholesale on every theme change — the
//! sequence is `rm -rf` then `mv` — so a watch on the file or on that
//! directory dies the first time the theme changes. See [`crate::watch`].

use std::collections::HashMap;
use std::path::PathBuf;

use crate::color::Color;
use crate::palette::{Mode, Palette};

/// Where Omarchy keeps the state of the active theme.
#[derive(Debug, Clone)]
pub struct Locations {
    /// `~/.local/state/omarchy/current`.
    pub current: PathBuf,
}

impl Locations {
    /// The standard location for the running user.
    pub fn standard() -> Self {
        let state = dirs::state_dir()
            .or_else(|| dirs::home_dir().map(|home| home.join(".local/state")))
            .unwrap_or_else(|| PathBuf::from("/nonexistent"));
        Self {
            current: state.join("omarchy/current"),
        }
    }

    /// Locations rooted at an arbitrary directory, for tests.
    pub fn under(current: impl Into<PathBuf>) -> Self {
        Self {
            current: current.into(),
        }
    }

    /// The staged theme directory, which is what every consumer reads.
    pub fn theme(&self) -> PathBuf {
        self.current.join("theme")
    }

    /// The palette file.
    pub fn colors(&self) -> PathBuf {
        self.theme().join("colors.toml")
    }

    /// The file holding the active theme's name, in kebab case.
    pub fn name_file(&self) -> PathBuf {
        self.current.join("theme.name")
    }

    /// The active theme's name, if one is staged.
    pub fn theme_name(&self) -> Option<String> {
        let name = std::fs::read_to_string(self.name_file()).ok()?;
        let name = name.trim();
        (!name.is_empty()).then(|| name.to_owned())
    }

    /// Read and resolve the active palette.
    ///
    /// `None` when there is no staged theme, or when its palette lacks even a
    /// usable background — in which case following it would look worse than
    /// not following it.
    pub fn load(&self) -> Option<Palette> {
        let source = std::fs::read_to_string(self.colors()).ok()?;
        // Predates the `mode` key; no shipped theme uses it any more, but a
        // theme installed years ago might.
        let light_marker = self.theme().join("light.mode").exists();
        resolve(&source, light_marker)
    }
}

/// Whether this looks like an Omarchy system at all.
///
/// Checked against `/etc/os-release` rather than `$OMARCHY_PATH`, which comes
/// from a shell profile and is therefore absent when the application is
/// launched from a desktop entry.
pub fn is_omarchy() -> bool {
    std::fs::read_to_string("/etc/os-release")
        .is_ok_and(|release| release.lines().any(|line| line.trim() == "ID=omarchy"))
}

/// Parse a `colors.toml` and resolve it into a full palette.
///
/// `light_marker` is whether a `light.mode` file sits beside it.
pub fn resolve(source: &str, light_marker: bool) -> Option<Palette> {
    let mut keys = read_keys(source);

    alias_legacy_short_names(&mut keys);
    alias_ansi_names(&mut keys);

    // Without a background there is nothing to derive the rest from.
    let background = colour(&keys, "background")?;
    let foreground = colour(&keys, "foreground").unwrap_or_else(|| {
        // A theme this incomplete is unusual; picking readable text beats
        // refusing to render.
        match Mode::of_background(background) {
            Mode::Dark => Color::WHITE,
            Mode::Light => Color::BLACK,
        }
    });

    let mode = resolve_mode(&keys, light_marker, background);

    // Order matters: orange feeds brown, and each base hue feeds its bright
    // variant, exactly as in the reference implementation.
    let red = colour(&keys, "red").unwrap_or(Color::rgb(0xd0, 0x50, 0x60));
    let yellow = colour(&keys, "yellow").unwrap_or(Color::rgb(0xd0, 0xa0, 0x50));
    let green = colour(&keys, "green").unwrap_or(Color::rgb(0x60, 0xb0, 0x70));
    let cyan = colour(&keys, "cyan").unwrap_or(Color::rgb(0x50, 0xb0, 0xb0));
    let blue = colour(&keys, "blue").unwrap_or(Color::rgb(0x60, 0x90, 0xd0));
    let magenta = colour(&keys, "magenta").unwrap_or(Color::rgb(0xa0, 0x80, 0xd0));
    let orange = colour(&keys, "orange").unwrap_or(yellow);
    let brown = colour(&keys, "brown").unwrap_or_else(|| orange.mix(Color::BLACK, 0.5));

    let dark_foreground = colour(&keys, "dark_foreground").unwrap_or(foreground);
    let bright_foreground = colour(&keys, "bright_foreground").unwrap_or(foreground);

    Some(Palette {
        mode,
        // Omarchy's cascade has no rule for accent, because every shipped
        // theme defines it. Blue is the closest thing to a neutral default.
        accent: colour(&keys, "accent").unwrap_or(blue),
        selection: colour(&keys, "selection")
            .or_else(|| colour(&keys, "selection_background"))
            .unwrap_or(background),
        muted: colour(&keys, "muted").unwrap_or(dark_foreground),

        background,
        dark_background: colour(&keys, "dark_background")
            .unwrap_or_else(|| background.mix(Color::BLACK, 0.25)),
        darker_background: colour(&keys, "darker_background")
            .unwrap_or_else(|| background.mix(Color::BLACK, 0.5)),
        lighter_background: colour(&keys, "lighter_background").unwrap_or(background),

        foreground,
        dark_foreground,
        light_foreground: colour(&keys, "light_foreground").unwrap_or(foreground),
        bright_foreground,

        red,
        yellow,
        orange,
        green,
        cyan,
        blue,
        magenta,
        brown,

        bright_red: brighten(&keys, "bright_red", red),
        bright_yellow: brighten(&keys, "bright_yellow", yellow),
        bright_green: brighten(&keys, "bright_green", green),
        bright_cyan: brighten(&keys, "bright_cyan", cyan),
        bright_blue: brighten(&keys, "bright_blue", blue),
        bright_magenta: brighten(&keys, "bright_magenta", magenta),
    })
}

/// Read the top-level string keys out of a theme file.
///
/// Anything that is not a top-level string is ignored rather than rejected:
/// a few themes carry Hyprland gradient tables, and one unusable key should
/// not cost the whole palette.
fn read_keys(source: &str) -> HashMap<String, String> {
    let Ok(table) = source.parse::<toml::Table>() else {
        return HashMap::new();
    };
    table
        .into_iter()
        .filter_map(|(key, value)| match value {
            toml::Value::String(text) => Some((key.to_ascii_lowercase(), text)),
            _ => None,
        })
        .collect()
}

/// Themes written before the semantic names used `bg`, `fg` and friends.
fn alias_legacy_short_names(keys: &mut HashMap<String, String>) {
    const PAIRS: &[(&str, &str)] = &[
        ("background", "bg"),
        ("dark_background", "dark_bg"),
        ("darker_background", "darker_bg"),
        ("lighter_background", "lighter_bg"),
        ("foreground", "fg"),
        ("dark_foreground", "dark_fg"),
        ("light_foreground", "light_fg"),
        ("bright_foreground", "bright_fg"),
    ];
    for (canonical, legacy) in PAIRS {
        alias(keys, canonical, legacy);
    }
}

/// Themes generated from an `alacritty.toml` only carry `color0`..`color15`.
fn alias_ansi_names(keys: &mut HashMap<String, String>) {
    const PAIRS: &[(&str, &str)] = &[
        ("background", "color0"),
        ("foreground", "color7"),
        ("red", "color1"),
        ("green", "color2"),
        ("yellow", "color3"),
        ("blue", "color4"),
        ("magenta", "color5"),
        ("cyan", "color6"),
        ("muted", "color8"),
        ("dark_foreground", "color8"),
        ("bright_red", "color9"),
        ("bright_green", "color10"),
        ("bright_yellow", "color11"),
        ("bright_blue", "color12"),
        ("bright_magenta", "color13"),
        ("bright_cyan", "color14"),
        ("bright_foreground", "color15"),
        ("magenta", "purple"),
        ("bright_magenta", "bright_purple"),
    ];
    for (canonical, legacy) in PAIRS {
        alias(keys, canonical, legacy);
    }
}

/// Fill `canonical` from `legacy` when the canonical name is absent.
fn alias(keys: &mut HashMap<String, String>, canonical: &str, legacy: &str) {
    if keys.contains_key(canonical) {
        return;
    }
    if let Some(value) = keys.get(legacy).cloned() {
        keys.insert(canonical.to_owned(), value);
    }
}

/// A parsed colour, or `None` if the key is missing or not a hex colour.
fn colour(keys: &HashMap<String, String>, name: &str) -> Option<Color> {
    keys.get(name)?.parse().ok()
}

/// A bright variant: the theme's own if given, otherwise the base mixed 20%
/// towards white.
fn brighten(keys: &HashMap<String, String>, name: &str, base: Color) -> Color {
    colour(keys, name).unwrap_or_else(|| base.mix(Color::WHITE, 0.2))
}

/// Resolve the light/dark question through all the levels Omarchy uses.
///
/// In order: the `mode` key, the older `theme_type` key, a `light.mode` file
/// beside the palette, and finally the background's own brightness.
fn resolve_mode(keys: &HashMap<String, String>, light_marker: bool, background: Color) -> Mode {
    keys.get("mode")
        .and_then(|value| Mode::parse(value))
        .or_else(|| keys.get("theme_type").and_then(|value| Mode::parse(value)))
        .or(light_marker.then_some(Mode::Light))
        .unwrap_or_else(|| Mode::of_background(background))
}

/// Read the monospace font family fontconfig resolves to.
///
/// Omarchy's own scripts treat fontconfig as authoritative and do not update
/// the GNOME font settings, so reading gsettings here would return a stale
/// answer. Falling back to `None` lets the caller choose its own default.
pub fn monospace_family() -> Option<String> {
    let output = std::process::Command::new("fc-match")
        .args(["monospace", "-f", "%{family}"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let family = String::from_utf8(output.stdout).ok()?;
    // fc-match returns a comma-separated family list; the first is the match.
    let family = family.split(',').next()?.trim();
    (!family.is_empty()).then(|| family.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shipped theme, verbatim, so the cascade is tested against the real
    /// thing rather than against what the cascade expects to see.
    const TOKYO_NIGHT: &str = r##"
mode = "dark"

accent = "#7aa2f7"
selection = "#292e42"
muted = "#414868"

background = "#1a1b26"
dark_background = "#13141c"
darker_background = "#0e0e14"
lighter_background = "#24283b"

foreground = "#a9b1d6"
dark_foreground = "#565f89"
light_foreground = "#b4bee6"
bright_foreground = "#c0caf5"

red = "#f7768e"
yellow = "#e0af68"
orange = "#eb927b"
green = "#9ece6a"
cyan = "#449dab"
blue = "#7aa2f7"
magenta = "#ad8ee6"
brown = "#75493d"

bright_red = "#ff7a93"
bright_yellow = "#ff9e64"
bright_green = "#b9f27c"
bright_cyan = "#0db9d7"
bright_blue = "#7da6ff"
bright_magenta = "#bb9af7"
"##;

    /// A shipped light theme.
    const CATPPUCCIN_LATTE: &str = r##"
mode = "light"
accent = "#1e66f5"
selection = "#ccd0da"
muted = "#acb0be"
background = "#eff1f5"
dark_background = "#e3e4e8"
darker_background = "#d7d8dc"
lighter_background = "#dce0e8"
foreground = "#4c4f69"
dark_foreground = "#9ca0b0"
light_foreground = "#5c5f77"
bright_foreground = "#4c4f69"
red = "#d20f39"
yellow = "#df8e1d"
orange = "#d84e2b"
green = "#40a02b"
cyan = "#179299"
blue = "#1e66f5"
magenta = "#ea76cb"
brown = "#6c2715"
"##;

    #[test]
    fn a_complete_theme_is_read_verbatim() {
        let palette = resolve(TOKYO_NIGHT, false).unwrap();

        assert_eq!(palette.mode, Mode::Dark);
        assert_eq!(palette.background.to_string(), "#1a1b26");
        assert_eq!(palette.accent.to_string(), "#7aa2f7");
        assert_eq!(palette.brown.to_string(), "#75493d");
        // Given explicitly, so not the 20%-towards-white derivation.
        assert_eq!(palette.bright_red.to_string(), "#ff7a93");
    }

    #[test]
    fn a_light_theme_reports_light_even_though_it_is_the_minority() {
        let palette = resolve(CATPPUCCIN_LATTE, false).unwrap();

        assert_eq!(palette.mode, Mode::Light);
        assert_eq!(palette.foreground.to_string(), "#4c4f69");
    }

    #[test]
    fn bright_variants_are_derived_when_a_theme_omits_them() {
        // catppuccin-latte defines no bright_* keys at all.
        let palette = resolve(CATPPUCCIN_LATTE, false).unwrap();

        let red: Color = "#d20f39".parse().unwrap();
        assert_eq!(palette.bright_red, red.mix(Color::WHITE, 0.2));
    }

    #[test]
    fn the_three_themes_missing_orange_and_brown_still_resolve() {
        // last-horizon, solitude and white ship no orange or brown.
        let source = r##"
mode = "light"
background = "#ffffff"
foreground = "#000000"
yellow = "#b58900"
"##;
        let palette = resolve(source, false).unwrap();

        assert_eq!(palette.orange, palette.yellow);
        let yellow: Color = "#b58900".parse().unwrap();
        assert_eq!(palette.brown, yellow.mix(Color::BLACK, 0.5));
    }

    #[test]
    fn a_theme_generated_from_alacritty_resolves_through_the_ansi_aliases() {
        // This is what omarchy-theme-colors-from-alacritty produces: ANSI
        // names only, and crucially no mode key.
        let source = r##"
accent = "#7aa2f7"
selection = "#292e42"
color0 = "#1a1b26"
color1 = "#f7768e"
color2 = "#9ece6a"
color3 = "#e0af68"
color4 = "#7aa2f7"
color5 = "#ad8ee6"
color6 = "#449dab"
color7 = "#a9b1d6"
color8 = "#414868"
color15 = "#c0caf5"
"##;
        let palette = resolve(source, false).unwrap();

        assert_eq!(palette.background.to_string(), "#1a1b26");
        assert_eq!(palette.foreground.to_string(), "#a9b1d6");
        assert_eq!(palette.red.to_string(), "#f7768e");
        assert_eq!(palette.muted.to_string(), "#414868");
        assert_eq!(palette.bright_foreground.to_string(), "#c0caf5");
        // No mode key, so the background's brightness decides.
        assert_eq!(palette.mode, Mode::Dark);
    }

    #[test]
    fn the_legacy_short_names_are_understood() {
        let source = r##"
bg = "#101010"
fg = "#e0e0e0"
lighter_bg = "#202020"
"##;
        let palette = resolve(source, false).unwrap();

        assert_eq!(palette.background.to_string(), "#101010");
        assert_eq!(palette.foreground.to_string(), "#e0e0e0");
        assert_eq!(palette.lighter_background.to_string(), "#202020");
    }

    #[test]
    fn purple_is_accepted_as_a_name_for_magenta() {
        let source = r##"
background = "#101010"
foreground = "#e0e0e0"
purple = "#b48ead"
"##;
        let palette = resolve(source, false).unwrap();

        assert_eq!(palette.magenta.to_string(), "#b48ead");
    }

    #[test]
    fn mode_precedence_runs_key_then_legacy_key_then_marker_then_brightness() {
        let dark_bg = r##"background = "#101010""##;

        // The key wins over everything, even a contradicting background.
        let explicit = format!("mode = \"light\"\n{dark_bg}");
        assert_eq!(resolve(&explicit, false).unwrap().mode, Mode::Light);

        // Then the older spelling.
        let legacy = format!("theme_type = \"light\"\n{dark_bg}");
        assert_eq!(resolve(&legacy, false).unwrap().mode, Mode::Light);

        // Then a light.mode file sitting beside the palette.
        assert_eq!(resolve(dark_bg, true).unwrap().mode, Mode::Light);

        // And only then the background itself.
        assert_eq!(resolve(dark_bg, false).unwrap().mode, Mode::Dark);
    }

    #[test]
    fn uppercase_hex_is_accepted() {
        // flexoki-light ships uppercase.
        let palette = resolve(r##"background = "#FFFCF0""##, false).unwrap();
        assert_eq!(palette.background.to_string(), "#fffcf0");
        assert_eq!(palette.mode, Mode::Light);
    }

    #[test]
    fn a_value_that_is_not_a_colour_is_skipped_rather_than_fatal() {
        // The Hyprland-specific keys a few themes carry hold gradient syntax.
        let source = r##"
background = "#1a1b26"
foreground = "#a9b1d6"
hyprland_active_border = "rgba(7aa2f7ff) rgba(bb9af7ff) 45deg"
accent = "not a colour"
"##;
        let palette = resolve(source, false).unwrap();

        assert_eq!(palette.background.to_string(), "#1a1b26");
        // accent was unusable, so it fell through to blue.
        assert_eq!(palette.accent, palette.blue);
    }

    #[test]
    fn shades_are_derived_from_the_background_when_absent() {
        let palette = resolve(r##"background = "#202020""##, false).unwrap();

        let background: Color = "#202020".parse().unwrap();
        assert_eq!(palette.dark_background, background.mix(Color::BLACK, 0.25));
        assert_eq!(palette.darker_background, background.mix(Color::BLACK, 0.5));
    }

    #[test]
    fn a_palette_with_no_background_is_refused() {
        assert!(resolve("", false).is_none());
        assert!(resolve(r##"accent = "#7aa2f7""##, false).is_none());
        assert!(resolve("this is not toml at all {{{", false).is_none());
    }

    #[test]
    fn a_background_without_a_foreground_still_gets_readable_text() {
        assert_eq!(
            resolve(r##"background = "#000000""##, false)
                .unwrap()
                .foreground,
            Color::WHITE
        );
        assert_eq!(
            resolve(r##"background = "#ffffff""##, false)
                .unwrap()
                .foreground,
            Color::BLACK
        );
    }

    #[test]
    fn the_staged_theme_is_read_from_the_state_directory() {
        let fixture = tempfile::tempdir().unwrap();
        let locations = Locations::under(fixture.path().join("current"));
        std::fs::create_dir_all(locations.theme()).unwrap();
        std::fs::write(locations.colors(), TOKYO_NIGHT).unwrap();
        std::fs::write(locations.name_file(), "tokyo-night\n").unwrap();

        assert_eq!(locations.theme_name().as_deref(), Some("tokyo-night"));
        assert_eq!(locations.load().unwrap().accent.to_string(), "#7aa2f7");
    }

    #[test]
    fn an_absent_staged_theme_loads_nothing() {
        let fixture = tempfile::tempdir().unwrap();
        let locations = Locations::under(fixture.path().join("current"));

        assert!(locations.load().is_none());
        assert!(locations.theme_name().is_none());
    }

    #[test]
    fn a_light_mode_marker_file_is_honoured() {
        let fixture = tempfile::tempdir().unwrap();
        let locations = Locations::under(fixture.path().join("current"));
        std::fs::create_dir_all(locations.theme()).unwrap();
        std::fs::write(locations.colors(), r##"background = "#101010""##).unwrap();
        std::fs::write(locations.theme().join("light.mode"), "").unwrap();

        assert_eq!(locations.load().unwrap().mode, Mode::Light);
    }
}
