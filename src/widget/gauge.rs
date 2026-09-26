//! The reclaimable-space gauge.
//!
//! One number, large, with a ring around it showing how that number relates
//! to the disk it came from. It is the only thing on the overview that should
//! read from across a room, so everything else defers to it.

use std::f32::consts::PI;

use iced::mouse;
use iced::widget::canvas as canvas_widget;
use iced::widget::canvas::{self, Cache, Geometry, Path, Stroke, Text};
use iced::{Element, Length, Point, Rectangle, Renderer, Size, Theme, Vector};

use limpid_theme::Palette;

use crate::style::{faded, to_iced};

/// Where the ring starts, in radians. Twelve o'clock.
const START: f32 = -PI / 2.0;
/// Thickness of the ring.
const THICKNESS: f32 = 14.0;
/// Padding between the ring and the edge of the widget.
const INSET: f32 = 6.0;

/// Bring a caller's fraction into 0.0..=1.0.
///
/// A ratio computed from a scan is one division away from a zero total, so
/// this has to survive NaN as well as being out of range. NaN means "no
/// answer", which is an empty bar; infinity means "more than everything",
/// which is a full one.
fn clamp_fraction(fraction: f32) -> f32 {
    if fraction.is_nan() {
        0.0
    } else {
        fraction.clamp(0.0, 1.0)
    }
}

/// A ring with a value in the middle.
pub struct Gauge {
    palette: Palette,
    /// How much of the ring to fill, 0.0 to 1.0.
    fraction: f32,
    /// The large text in the centre.
    value: String,
    /// The small text under it.
    caption: String,
    cache: Cache,
}

impl Gauge {
    /// Build a gauge.
    ///
    /// `fraction` is clamped, so a caller that divides by a zero total gets a
    /// sensible empty ring rather than a panic or a NaN sweep.
    pub fn new(
        palette: Palette,
        fraction: f32,
        value: impl Into<String>,
        caption: impl Into<String>,
    ) -> Self {
        Self {
            palette,
            fraction: clamp_fraction(fraction),
            value: value.into(),
            caption: caption.into(),
            cache: Cache::new(),
        }
    }

    /// The gauge as an element, sized square.
    pub fn view<'a, Message: 'a>(self, side: f32) -> Element<'a, Message> {
        canvas_widget::Canvas::new(self)
            .width(Length::Fixed(side))
            .height(Length::Fixed(side))
            .into()
    }
}

impl<Message> canvas::Program<Message> for Gauge {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            let centre = Point::new(frame.width() / 2.0, frame.height() / 2.0);
            let radius = (frame.width().min(frame.height()) / 2.0) - INSET - THICKNESS / 2.0;

            if radius <= 0.0 {
                return;
            }

            // The track. Faint enough to read as a groove rather than as a
            // second value.
            frame.stroke(
                &Path::circle(centre, radius),
                Stroke::default()
                    .with_color(faded(self.palette.muted, 0.25))
                    .with_width(THICKNESS),
            );

            if self.fraction > 0.0 {
                let sweep = 2.0 * PI * self.fraction;
                let arc = Path::new(|builder| {
                    builder.arc(canvas::path::Arc {
                        center: centre,
                        radius,
                        start_angle: iced::Radians(START),
                        end_angle: iced::Radians(START + sweep),
                    });
                });
                frame.stroke(
                    &arc,
                    Stroke::default()
                        .with_color(to_iced(self.palette.accent))
                        .with_width(THICKNESS)
                        .with_line_cap(canvas::LineCap::Round),
                );
            }

            // The value, sized against the ring so it stays inside it at any
            // widget size.
            let value_size = radius * 0.32;
            frame.fill_text(Text {
                content: self.value.clone(),
                position: centre - Vector::new(0.0, value_size * 0.66),
                color: to_iced(self.palette.bright_foreground),
                size: value_size.into(),
                align_x: iced::alignment::Horizontal::Center.into(),
                align_y: iced::alignment::Vertical::Top,
                ..Text::default()
            });

            let caption_size = (radius * 0.14).max(10.0);
            frame.fill_text(Text {
                content: self.caption.clone(),
                position: centre + Vector::new(0.0, value_size * 0.48),
                color: to_iced(self.palette.dark_foreground),
                size: caption_size.into(),
                align_x: iced::alignment::Horizontal::Center.into(),
                align_y: iced::alignment::Vertical::Top,
                ..Text::default()
            });
        });

        vec![geometry]
    }
}

/// A slim horizontal bar, for the per-category proportions.
pub struct Bar {
    palette: Palette,
    fraction: f32,
    tint: limpid_theme::Color,
    cache: Cache,
}

impl Bar {
    /// Build a bar.
    pub fn new(palette: Palette, fraction: f32, tint: limpid_theme::Color) -> Self {
        Self {
            palette,
            fraction: clamp_fraction(fraction),
            tint,
            cache: Cache::new(),
        }
    }

    /// The bar as an element.
    pub fn view<'a, Message: 'a>(self, height: f32) -> Element<'a, Message> {
        canvas_widget::Canvas::new(self)
            .width(Length::Fill)
            .height(Length::Fixed(height))
            .into()
    }
}

impl<Message> canvas::Program<Message> for Bar {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            let height = frame.height();
            let radius = height / 2.0;

            frame.fill(
                &Path::rounded_rectangle(
                    Point::ORIGIN,
                    Size::new(frame.width(), height),
                    radius.into(),
                ),
                faded(self.palette.muted, 0.22),
            );

            // A sliver of colour still reads as "some", where a hairline
            // would read as "none".
            let filled =
                (frame.width() * self.fraction).max(if self.fraction > 0.0 { height } else { 0.0 });

            if filled > 0.0 {
                frame.fill(
                    &Path::rounded_rectangle(
                        Point::ORIGIN,
                        Size::new(filled, height),
                        radius.into(),
                    ),
                    to_iced(self.tint),
                );
            }
        });

        vec![geometry]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fraction_outside_the_range_is_clamped() {
        let palette = Palette::dark();

        assert_eq!(Gauge::new(palette, 1.7, "", "").fraction, 1.0);
        assert_eq!(Gauge::new(palette, -0.4, "", "").fraction, 0.0);
    }

    #[test]
    fn a_division_by_zero_produces_an_empty_ring_rather_than_a_nan() {
        let palette = Palette::dark();

        // 0/0 is what a scan that found nothing divides out to. Computed
        // rather than written, because the compiler rejects the literal.
        let found = 0.0_f32;
        assert_eq!(Gauge::new(palette, found / found, "", "").fraction, 0.0);
        assert_eq!(Bar::new(palette, f32::NAN, palette.accent).fraction, 0.0);
    }

    #[test]
    fn an_infinite_fraction_fills_rather_than_empties() {
        let palette = Palette::dark();

        assert_eq!(
            Bar::new(palette, f32::INFINITY, palette.accent).fraction,
            1.0
        );
        assert_eq!(Gauge::new(palette, f32::NEG_INFINITY, "", "").fraction, 0.0);
    }
}
