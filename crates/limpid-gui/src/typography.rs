//! The type scale.
//!
//! Taken from Omarchy's own `shell.toml`, so that Limpid's text sits at the
//! same sizes as the bar and the popups it shares a screen with. Off Omarchy
//! the same scale is used; it is a reasonable one regardless of where it
//! came from.

/// Smallest text: units, timestamps, footnotes.
pub const CAPTION: f32 = 11.0;
/// Secondary body text.
pub const BODY_SMALL: f32 = 12.0;
/// Body text.
pub const BODY: f32 = 13.0;
/// Slightly emphasised body text.
pub const SUBTITLE: f32 = 14.0;
/// A section title.
pub const TITLE: f32 = 16.0;
/// A page heading.
pub const HEADING: f32 = 20.0;
/// The one number that carries a screen.
pub const DISPLAY: f32 = 30.0;

/// Standard spacing step, in pixels. Everything is a multiple of this.
pub const STEP: f32 = 8.0;

/// Gap inside a group of related controls.
pub const GAP_TIGHT: f32 = STEP;
/// Gap between related blocks.
pub const GAP: f32 = STEP * 2.0;
/// Gap between sections.
pub const GAP_WIDE: f32 = STEP * 3.0;
/// Page margin.
pub const MARGIN: f32 = STEP * 4.0;

/// Width of the navigation column.
pub const SIDEBAR_WIDTH: f32 = 216.0;
