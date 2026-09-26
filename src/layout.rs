//! How much room there is, and what to do with it.
//!
//! Limpid is expected to live in a tiling window manager, where it does not
//! choose its own size and can be handed a quarter of a small screen without
//! warning. So nothing in the interface has a fixed size that only works at
//! one width: every view takes a [`Metrics`] and asks it.
//!
//! The breakpoints are where something actually stops fitting, measured
//! rather than picked from a list of phone widths.

use crate::typography as ty;

/// How much horizontal room there is, in bands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Width {
    /// A quarter of a laptop screen. The sidebar is gone and rows stack.
    Tiny,
    /// Half a screen. No sidebar, but rows still have two columns.
    Narrow,
    /// Room for the sidebar beside the content.
    Wide,
}

/// Below this the sidebar costs more than it gives: at 520 px it is 42% of
/// the window, and what is left cannot hold a card and its figures.
///
/// Set so that the moment the sidebar appears, what remains is still enough
/// for the ring and its figures side by side. A threshold that left the hero
/// stacked *because* the sidebar had just taken 216 px would be worse than
/// having no sidebar.
const SIDEBAR_NEEDS: f32 = 760.0;
/// Below this a row of label-and-value has to become two lines.
const TWO_COLUMNS_NEED: f32 = 460.0;
/// What a readable column of figures beside the ring costs.
const FIGURES_NEED: f32 = 300.0;
/// What a row of two secondary buttons and a primary one costs.
const BUTTON_ROW_NEEDS: f32 = 300.0;

/// Everything the views need to know about the room they have.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// Which band the window is in.
    pub width: Width,
    /// Full window width.
    pub window: f32,
    /// Width the content area actually gets, sidebar already taken out.
    pub content: f32,
    /// Window height.
    pub height: f32,
    /// Padding around the page.
    pub margin: f32,
    /// Gap between cards.
    pub gap: f32,
    /// Padding inside a card.
    pub card: f32,
    /// Diameter of the gauge.
    pub gauge: f32,
    /// Whether the navigation is a column beside the content or a row above.
    pub sidebar: bool,
}

impl Metrics {
    /// Work out the metrics for a window of this size.
    pub fn of(window: iced::Size) -> Self {
        let sidebar = window.width >= SIDEBAR_NEEDS;
        let width = if window.width < 520.0 {
            Width::Tiny
        } else if window.width < SIDEBAR_NEEDS {
            Width::Narrow
        } else {
            Width::Wide
        };

        let margin = match width {
            Width::Tiny => ty::STEP * 1.5,
            Width::Narrow => ty::STEP * 2.5,
            Width::Wide => ty::STEP * 4.0,
        };

        let content =
            (window.width - if sidebar { ty::SIDEBAR_WIDTH } else { 0.0 } - margin * 2.0).max(1.0);

        Self {
            width,
            window: window.width,
            content,
            height: window.height,
            margin,
            gap: match width {
                Width::Tiny => ty::STEP * 1.5,
                _ => ty::STEP * 2.0,
            },
            card: match width {
                Width::Tiny => ty::STEP * 1.75,
                Width::Narrow => ty::STEP * 2.5,
                Width::Wide => ty::STEP * 3.0,
            },
            // Scaled against the room actually left over, not the window, and
            // capped so it never eats a short window whole.
            // 0.34 rather than a larger share so that the ring shrinking
            // never costs the figures their place beside it.
            gauge: (content * 0.34)
                .clamp(96.0, 208.0)
                .min(window.height * 0.32)
                .max(96.0),
            sidebar,
        }
    }

    /// Whether a label and its value can sit on one line.
    pub fn two_columns(&self) -> bool {
        self.content >= TWO_COLUMNS_NEED
    }

    /// Whether a row of buttons fits without the last one running off the
    /// edge.
    ///
    /// Below this the primary action takes a line of its own, at full
    /// width. A clipped button is not a smaller button — it is one the user
    /// cannot tell is there.
    pub fn buttons_inline(&self) -> bool {
        self.content >= BUTTON_ROW_NEEDS
    }

    /// Whether the gauge and the figures beside it both fit.
    ///
    /// Asks for the gauge plus a readable column of figures, rather than a
    /// fixed breakpoint, so it stays right when the gauge scales.
    pub fn hero_side_by_side(&self) -> bool {
        self.content >= self.gauge + FIGURES_NEED
    }

    /// How many swatches fit on one row.
    pub fn swatches_per_row(&self, swatch: f32, gap: f32) -> usize {
        (((self.content + gap) / (swatch + gap)).floor() as usize).max(1)
    }

    /// The type size for the one big number, which cannot stay at 30 px when
    /// the card holding it is 260 px wide.
    pub fn display(&self) -> f32 {
        match self.width {
            Width::Tiny => ty::TITLE,
            Width::Narrow => 24.0,
            Width::Wide => ty::DISPLAY,
        }
    }

    /// The type size for a page heading.
    pub fn heading(&self) -> f32 {
        match self.width {
            Width::Tiny => ty::TITLE,
            _ => ty::HEADING,
        }
    }

    /// Height of the treemap: tall enough to read, never more than half a
    /// short window.
    pub fn treemap(&self) -> f32 {
        (self.height * 0.42).clamp(140.0, 340.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(width: f32, height: f32) -> Metrics {
        Metrics::of(iced::Size::new(width, height))
    }

    #[test]
    fn a_tiled_quarter_screen_loses_the_sidebar() {
        // A quarter of this laptop's 1152 logical pixels.
        let metrics = at(288.0, 700.0);

        assert_eq!(metrics.width, Width::Tiny);
        assert!(!metrics.sidebar);
        assert!(!metrics.two_columns());
        assert!(!metrics.hero_side_by_side());
    }

    #[test]
    fn a_tiled_half_screen_keeps_two_columns_but_not_the_sidebar() {
        let metrics = at(576.0, 700.0);

        assert_eq!(metrics.width, Width::Narrow);
        assert!(!metrics.sidebar);
        assert!(metrics.two_columns());
    }

    #[test]
    fn a_full_window_gets_everything() {
        let metrics = at(1020.0, 660.0);

        assert_eq!(metrics.width, Width::Wide);
        assert!(metrics.sidebar);
        assert!(metrics.two_columns());
        assert!(metrics.hero_side_by_side());
    }

    #[test]
    fn the_content_width_accounts_for_the_sidebar_and_the_margins() {
        let wide = at(1020.0, 660.0);
        assert_eq!(wide.content, 1020.0 - ty::SIDEBAR_WIDTH - wide.margin * 2.0);

        let narrow = at(600.0, 660.0);
        assert_eq!(narrow.content, 600.0 - narrow.margin * 2.0);
    }

    #[test]
    fn the_gauge_never_outgrows_a_short_window() {
        // A wide but very short window: the ring has to give way, not the
        // rest of the card.
        let letterbox = at(1200.0, 320.0);

        assert!(letterbox.gauge <= 320.0 * 0.32 + 0.01 || letterbox.gauge == 96.0);
        assert!(letterbox.gauge >= 96.0);
    }

    #[test]
    fn the_gauge_never_shrinks_below_something_readable() {
        for (width, height) in [(200.0, 200.0), (320.0, 240.0), (1.0, 1.0)] {
            assert!(at(width, height).gauge >= 96.0, "{width}x{height}");
        }
    }

    #[test]
    fn margins_shrink_before_the_content_does() {
        assert!(at(288.0, 700.0).margin < at(1020.0, 660.0).margin);
        // And there is still some, so text never touches the frame.
        assert!(at(288.0, 700.0).margin >= 8.0);
    }

    #[test]
    fn the_buttons_stop_sharing_a_line_before_one_would_be_clipped() {
        // Three buttons need about 300 px; a quarter-screen window has
        // fewer than 220 to give.
        assert!(!at(240.0, 690.0).buttons_inline());
        assert!(at(576.0, 640.0).buttons_inline());
        assert!(at(1020.0, 660.0).buttons_inline());
    }

    #[test]
    fn swatches_wrap_rather_than_overflow() {
        // Seven swatches at 72 px plus gaps need about 560 px.
        assert!(at(1020.0, 660.0).swatches_per_row(72.0, 8.0) >= 7);
        assert!(at(400.0, 660.0).swatches_per_row(72.0, 8.0) < 7);
        // And never zero, however little room there is.
        assert_eq!(at(40.0, 100.0).swatches_per_row(72.0, 8.0), 1);
    }

    #[test]
    fn the_hero_splits_when_the_ring_and_the_figures_both_fit() {
        // The threshold follows the gauge rather than being a fixed width,
        // so it stays true as the ring scales.
        for width in [320.0, 520.0, 700.0, 1020.0, 1600.0] {
            let metrics = at(width, 700.0);
            assert_eq!(
                metrics.hero_side_by_side(),
                metrics.content >= metrics.gauge + FIGURES_NEED,
                "at {width}",
            );
        }
    }

    #[test]
    fn the_sidebar_never_appears_at_the_cost_of_splitting_the_hero() {
        // The two thresholds have to agree: gaining a sidebar and losing the
        // side-by-side hero in the same pixel would read as a bug.
        for width in [700.0, 740.0, 760.0, 780.0, 820.0, 1020.0, 1600.0] {
            let metrics = at(width, 660.0);
            if metrics.sidebar {
                assert!(
                    metrics.hero_side_by_side(),
                    "at {width} the sidebar appeared but the hero stacked",
                );
            }
        }
    }

    #[test]
    fn a_degenerate_size_does_not_produce_a_negative_width() {
        let metrics = at(0.0, 0.0);

        assert!(metrics.content >= 1.0);
        assert!(metrics.gauge > 0.0);
        assert!(metrics.treemap() > 0.0);
    }
}
