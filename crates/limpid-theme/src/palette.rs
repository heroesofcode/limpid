//! The set of colours Limpid draws with.
//!
//! The field names are Omarchy's, so that following a theme is a copy rather
//! than a translation. Off Omarchy the same struct is filled from a built-in
//! palette, and nothing downstream can tell the difference.

use crate::color::Color;

/// Whether a palette is light or dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Dark background, light text.
    Dark,
    /// Light background, dark text.
    Light,
}

impl Mode {
    /// The mode implied by a background colour.
    ///
    /// Omarchy's last-resort guess: the sum of the channels against 382.
    pub fn of_background(background: Color) -> Self {
        if background.channel_sum() > 382 {
            Self::Light
        } else {
            Self::Dark
        }
    }

    /// Parse the `mode` or `theme_type` value from a theme file.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}

/// Every colour the interface needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// Whether this is a light or dark palette.
    pub mode: Mode,

    /// The one colour used for emphasis: primary actions, the gauge arc.
    pub accent: Color,
    /// Background behind a selected row.
    pub selection: Color,
    /// Borders and separators.
    pub muted: Color,

    /// Window background.
    pub background: Color,
    /// One step back from the window background, for wells and sunken areas.
    pub dark_background: Color,
    /// Two steps back.
    pub darker_background: Color,
    /// One step forward, for cards sitting on the window background.
    pub lighter_background: Color,

    /// Body text.
    pub foreground: Color,
    /// De-emphasised text.
    pub dark_foreground: Color,
    /// Slightly de-emphasised text.
    pub light_foreground: Color,
    /// Headings and emphasis.
    pub bright_foreground: Color,

    /// Danger, and the "sensitive" risk level.
    pub red: Color,
    /// Caution, and the "review" risk level.
    pub yellow: Color,
    /// A secondary warning hue.
    pub orange: Color,
    /// Success, and the "safe" risk level.
    pub green: Color,
    /// An informational hue.
    pub cyan: Color,
    /// An informational hue.
    pub blue: Color,
    /// An informational hue.
    pub magenta: Color,
    /// A muted warm hue, used in charts.
    pub brown: Color,

    /// Brighter red, for hover and emphasis.
    pub bright_red: Color,
    /// Brighter yellow.
    pub bright_yellow: Color,
    /// Brighter green.
    pub bright_green: Color,
    /// Brighter cyan.
    pub bright_cyan: Color,
    /// Brighter blue.
    pub bright_blue: Color,
    /// Brighter magenta.
    pub bright_magenta: Color,
}

impl Palette {
    /// The palette used when there is no system theme to follow.
    ///
    /// Not a copy of any shipped Omarchy theme: Limpid should look like
    /// itself when it is not somewhere with an opinion.
    pub const fn dark() -> Self {
        Self {
            mode: Mode::Dark,
            accent: Color::rgb(0x6a, 0xb8, 0xd8),
            selection: Color::rgb(0x2a, 0x30, 0x38),
            muted: Color::rgb(0x3c, 0x44, 0x4e),
            background: Color::rgb(0x14, 0x17, 0x1c),
            dark_background: Color::rgb(0x0f, 0x12, 0x16),
            darker_background: Color::rgb(0x0a, 0x0c, 0x0f),
            lighter_background: Color::rgb(0x1d, 0x22, 0x29),
            foreground: Color::rgb(0xc7, 0xcf, 0xda),
            dark_foreground: Color::rgb(0x71, 0x7d, 0x8c),
            light_foreground: Color::rgb(0xa8, 0xb3, 0xc1),
            bright_foreground: Color::rgb(0xe8, 0xed, 0xf3),
            red: Color::rgb(0xe0, 0x70, 0x7c),
            yellow: Color::rgb(0xd8, 0xaa, 0x60),
            orange: Color::rgb(0xdc, 0x8b, 0x62),
            green: Color::rgb(0x7f, 0xbf, 0x8a),
            cyan: Color::rgb(0x6c, 0xc0, 0xbc),
            blue: Color::rgb(0x6a, 0xb8, 0xd8),
            magenta: Color::rgb(0xb0, 0x92, 0xd4),
            brown: Color::rgb(0x8a, 0x6a, 0x55),
            bright_red: Color::rgb(0xe6, 0x8d, 0x96),
            bright_yellow: Color::rgb(0xe0, 0xbb, 0x80),
            bright_green: Color::rgb(0x99, 0xcc, 0xa1),
            bright_cyan: Color::rgb(0x89, 0xcd, 0xc9),
            bright_blue: Color::rgb(0x88, 0xc6, 0xdf),
            bright_magenta: Color::rgb(0xc0, 0xa8, 0xdd),
        }
    }

    /// The light counterpart of [`Palette::dark`].
    pub const fn light() -> Self {
        Self {
            mode: Mode::Light,
            accent: Color::rgb(0x27, 0x6d, 0x93),
            selection: Color::rgb(0xdc, 0xe3, 0xea),
            muted: Color::rgb(0xbe, 0xc7, 0xd1),
            background: Color::rgb(0xfa, 0xfb, 0xfc),
            dark_background: Color::rgb(0xee, 0xf1, 0xf4),
            darker_background: Color::rgb(0xe2, 0xe6, 0xeb),
            lighter_background: Color::rgb(0xff, 0xff, 0xff),
            foreground: Color::rgb(0x33, 0x3c, 0x47),
            dark_foreground: Color::rgb(0x77, 0x82, 0x90),
            light_foreground: Color::rgb(0x4c, 0x56, 0x62),
            bright_foreground: Color::rgb(0x1b, 0x22, 0x2b),
            red: Color::rgb(0xbc, 0x36, 0x44),
            yellow: Color::rgb(0x9a, 0x6b, 0x16),
            orange: Color::rgb(0xb2, 0x53, 0x25),
            green: Color::rgb(0x2f, 0x7d, 0x45),
            cyan: Color::rgb(0x1c, 0x72, 0x74),
            blue: Color::rgb(0x27, 0x6d, 0x93),
            magenta: Color::rgb(0x8a, 0x44, 0xa8),
            brown: Color::rgb(0x6c, 0x4a, 0x32),
            bright_red: Color::rgb(0xd4, 0x46, 0x53),
            bright_yellow: Color::rgb(0xb0, 0x7d, 0x1e),
            bright_green: Color::rgb(0x38, 0x92, 0x50),
            bright_cyan: Color::rgb(0x22, 0x85, 0x87),
            bright_blue: Color::rgb(0x2e, 0x80, 0xab),
            bright_magenta: Color::rgb(0xa0, 0x51, 0xc2),
        }
    }

    /// The built-in palette matching `mode`.
    pub const fn builtin(mode: Mode) -> Self {
        match mode {
            Mode::Dark => Self::dark(),
            Mode::Light => Self::light(),
        }
    }

    /// Text that stays readable on top of `background`.
    ///
    /// Used for labels drawn over a filled shape — a chart segment, a badge —
    /// where the fill comes from the palette and the text has to work over
    /// whatever the theme chose.
    pub fn on(&self, background: Color) -> Color {
        let contrast = |text: Color| {
            let (lighter, darker) = if text.luminance() > background.luminance() {
                (text.luminance(), background.luminance())
            } else {
                (background.luminance(), text.luminance())
            };
            (lighter + 0.05) / (darker + 0.05)
        };

        if contrast(self.bright_foreground) >= contrast(self.darker_background) {
            self.bright_foreground
        } else {
            self.darker_background
        }
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::dark()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_parses_the_values_themes_actually_use() {
        assert_eq!(Mode::parse("dark"), Some(Mode::Dark));
        assert_eq!(Mode::parse("Light"), Some(Mode::Light));
        assert_eq!(Mode::parse(" dark "), Some(Mode::Dark));
        assert_eq!(Mode::parse("sepia"), None);
    }

    #[test]
    fn the_background_guess_agrees_with_the_shipped_themes() {
        assert_eq!(
            Mode::of_background(Color::rgb(0x1a, 0x1b, 0x26)),
            Mode::Dark
        );
        assert_eq!(
            Mode::of_background(Color::rgb(0xef, 0xf1, 0xf5)),
            Mode::Light
        );
        assert_eq!(Mode::of_background(Color::BLACK), Mode::Dark);
        assert_eq!(Mode::of_background(Color::WHITE), Mode::Light);
    }

    #[test]
    fn both_builtin_palettes_declare_the_mode_they_look_like() {
        assert_eq!(Palette::dark().mode, Mode::Dark);
        assert_eq!(Mode::of_background(Palette::dark().background), Mode::Dark);
        assert_eq!(Palette::light().mode, Mode::Light);
        assert_eq!(
            Mode::of_background(Palette::light().background),
            Mode::Light
        );
    }

    #[test]
    fn text_over_a_fill_picks_the_more_readable_of_the_two_extremes() {
        let palette = Palette::dark();

        // Over a light accent the dark extreme wins, and vice versa.
        assert_eq!(palette.on(Color::WHITE), palette.darker_background);
        assert_eq!(palette.on(Color::BLACK), palette.bright_foreground);
    }
}
