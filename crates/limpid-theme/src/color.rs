//! A colour, and the two operations the Omarchy palette cascade needs.

use std::fmt;
use std::str::FromStr;

/// An opaque 24-bit colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
}

impl Color {
    /// Build a colour from its channels.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Black.
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    /// White.
    pub const WHITE: Self = Self::rgb(255, 255, 255);

    /// Linear blend towards `other`, where `amount` runs 0.0 to 1.0.
    ///
    /// This mirrors the mix in `omarchy-theme-color`, which is what generates
    /// the derived shades a theme leaves out. Blending in sRGB rather than
    /// linear light is wrong in the colour-science sense and right here,
    /// because matching the reference implementation matters more than being
    /// perceptually correct — a Limpid window sitting beside a themed
    /// terminal has to agree with it.
    pub fn mix(self, other: Self, amount: f32) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        let blend = |from: u8, to: u8| {
            (f32::from(from) * (1.0 - amount) + f32::from(to) * amount + 0.5) as u8
        };
        Self::rgb(
            blend(self.r, other.r),
            blend(self.g, other.g),
            blend(self.b, other.b),
        )
    }

    /// Sum of the channels, which is the test Omarchy uses to guess whether a
    /// theme is light when it does not say so. The threshold is 382.
    pub fn channel_sum(self) -> u16 {
        u16::from(self.r) + u16::from(self.g) + u16::from(self.b)
    }

    /// Relative luminance, for choosing readable text over this colour.
    pub fn luminance(self) -> f32 {
        let channel = |value: u8| {
            let normalised = f32::from(value) / 255.0;
            if normalised <= 0.040_45 {
                normalised / 12.92
            } else {
                ((normalised + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(self.r) + 0.7152 * channel(self.g) + 0.0722 * channel(self.b)
    }

    /// The channels as floats in 0.0..=1.0, which is what every GPU-backed
    /// toolkit wants.
    pub fn to_f32(self) -> [f32; 3] {
        [
            f32::from(self.r) / 255.0,
            f32::from(self.g) / 255.0,
            f32::from(self.b) / 255.0,
        ]
    }
}

/// A value in a theme file that was not a colour Limpid could use.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a hex colour: {0}")]
pub struct NotAColor(String);

impl FromStr for Color {
    type Err = NotAColor;

    /// Parse `#rrggbb` or `#rgb`, in either case.
    ///
    /// Omarchy's own validator also admits `rgb()`, `rgba()`, gradient angles
    /// and bare words, which appear in the handful of Hyprland-specific keys.
    /// Those are rejected here rather than guessed at; the cascade treats a
    /// rejected value as absent and derives a replacement.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let digits = text.trim().strip_prefix('#').unwrap_or(text.trim());

        let parse = |slice: &str| u8::from_str_radix(slice, 16).ok();
        let double = |slice: &str| u8::from_str_radix(slice, 16).ok().map(|v| v * 17);

        let parsed = match digits.len() {
            6 => (
                parse(&digits[0..2]),
                parse(&digits[2..4]),
                parse(&digits[4..6]),
            ),
            3 => (
                double(&digits[0..1]),
                double(&digits[1..2]),
                double(&digits[2..3]),
            ),
            _ => (None, None, None),
        };

        match parsed {
            (Some(r), Some(g), Some(b)) => Ok(Self::rgb(r, g, b)),
            _ => Err(NotAColor(text.to_owned())),
        }
    }
}

impl fmt::Display for Color {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_digit_hex_parses_in_either_case() {
        // flexoki-light ships uppercase, everything else lowercase.
        assert_eq!(
            "#1a1b26".parse::<Color>().unwrap(),
            Color::rgb(0x1a, 0x1b, 0x26)
        );
        assert_eq!(
            "#FFFCF0".parse::<Color>().unwrap(),
            Color::rgb(0xff, 0xfc, 0xf0)
        );
    }

    #[test]
    fn three_digit_hex_expands_each_digit() {
        assert_eq!(
            "#abc".parse::<Color>().unwrap(),
            Color::rgb(0xaa, 0xbb, 0xcc)
        );
    }

    #[test]
    fn the_leading_hash_and_surrounding_space_are_optional() {
        assert_eq!(
            "  1a1b26  ".parse::<Color>().unwrap(),
            Color::rgb(0x1a, 0x1b, 0x26)
        );
    }

    #[test]
    fn hyprland_gradient_syntax_is_rejected_rather_than_guessed_at() {
        assert!(
            "rgba(1a1b26ff) rgba(7aa2f7ff) 45deg"
                .parse::<Color>()
                .is_err()
        );
        assert!("-45deg".parse::<Color>().is_err());
        assert!("".parse::<Color>().is_err());
        assert!("#12345".parse::<Color>().is_err());
    }

    #[test]
    fn mixing_matches_the_reference_shell_implementation() {
        // Each expected value was produced by running the awk in
        // omarchy-theme-color's mix_color, so these pin Limpid to the
        // palette a themed terminal beside it will be using.
        let cases = [
            // (start, end, amount, expected) — the derivations the cascade
            // actually performs when a theme leaves a key out.
            ("#f7768e", "#ffffff", 0.2, "#f991a5"), // bright_red
            ("#1a1b26", "#000000", 0.5, "#0d0e13"), // darker_background
            ("#1a1b26", "#000000", 0.25, "#14141d"), // dark_background
            ("#d84e2b", "#000000", 0.5, "#6c2716"), // brown
            ("#d20f39", "#ffffff", 0.2, "#db3f61"), // bright_red, light theme
        ];

        for (start, end, amount, expected) in cases {
            let mixed = start
                .parse::<Color>()
                .unwrap()
                .mix(end.parse().unwrap(), amount);
            assert_eq!(mixed.to_string(), expected, "mix({start}, {end}, {amount})");
        }
    }

    #[test]
    fn mixing_by_zero_or_one_returns_an_endpoint() {
        let a = Color::rgb(10, 20, 30);
        let b = Color::rgb(200, 210, 220);
        assert_eq!(a.mix(b, 0.0), a);
        assert_eq!(a.mix(b, 1.0), b);
        assert_eq!(a.mix(b, -5.0), a);
        assert_eq!(a.mix(b, 5.0), b);
    }

    #[test]
    fn the_channel_sum_threshold_separates_the_shipped_light_and_dark_themes() {
        // catppuccin-latte's background, which is light.
        assert!("#eff1f5".parse::<Color>().unwrap().channel_sum() > 382);
        // tokyo-night's, which is not.
        assert!("#1a1b26".parse::<Color>().unwrap().channel_sum() <= 382);
    }

    #[test]
    fn display_round_trips_through_parsing() {
        let colour = Color::rgb(0x7a, 0xa2, 0xf7);
        assert_eq!(colour.to_string(), "#7aa2f7");
        assert_eq!(colour.to_string().parse::<Color>().unwrap(), colour);
    }
}
