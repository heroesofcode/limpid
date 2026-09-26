//! A treemap of one directory level.
//!
//! Area is proportional to occupied size, so the eye goes to the biggest
//! thing without reading a number. Clicking a tile descends into it, which
//! is the whole interaction: the question "where did the space go" is
//! answered by repeating it a few times.

use iced::widget::canvas::{self, Geometry, Path, Stroke, Text};
use iced::{Element, Length, Point, Rectangle, Renderer, Size as IcedSize, Theme, mouse};

use limpid_core::analyse::Entry;
use limpid_core::size::human;
use limpid_theme::{Color, Palette};

use crate::style::{faded, to_iced};

/// Gap between tiles.
const GUTTER: f32 = 2.0;
/// A tile smaller than this in either direction gets no label.
const LABEL_MINIMUM: f32 = 54.0;

/// A tile, once laid out.
#[derive(Debug, Clone)]
struct Tile {
    bounds: Rectangle,
    label: String,
    detail: String,
    tint: Color,
    index: usize,
}

/// The treemap.
pub struct Treemap<Message> {
    palette: Palette,
    entries: Vec<Entry>,
    total: u64,
    on_select: Box<dyn Fn(usize) -> Message>,
}

impl<Message: Clone + 'static> Treemap<Message> {
    /// Build a treemap over one level.
    pub fn new(
        palette: Palette,
        entries: Vec<Entry>,
        on_select: impl Fn(usize) -> Message + 'static,
    ) -> Self {
        let total = entries.iter().map(|entry| entry.size.on_disk).sum();
        Self {
            palette,
            entries,
            total,
            on_select: Box::new(on_select),
        }
    }

    /// The treemap as an element of a given height.
    pub fn view<'a>(self, height: f32) -> Element<'a, Message>
    where
        Message: 'a,
    {
        canvas::Canvas::new(self)
            .width(Length::Fill)
            .height(Length::Fixed(height))
            .into()
    }

    /// Lay the tiles out inside `bounds`.
    fn layout(&self, bounds: Rectangle) -> Vec<Tile> {
        let weights: Vec<f64> = self
            .entries
            .iter()
            .map(|entry| entry.size.on_disk as f64)
            .collect();
        let rectangles = squarify(&weights, bounds);

        rectangles
            .into_iter()
            .enumerate()
            .filter(|(_, rectangle)| rectangle.width > 1.0 && rectangle.height > 1.0)
            .map(|(index, rectangle)| {
                let entry = &self.entries[index];
                Tile {
                    bounds: rectangle,
                    label: entry.name.clone(),
                    detail: human(entry.size.on_disk),
                    tint: hue(self.palette, index),
                    index,
                }
            })
            .collect()
    }
}

/// The hue a tile gets, cycled so neighbours differ.
fn hue(palette: Palette, index: usize) -> Color {
    let series = [
        palette.accent,
        palette.cyan,
        palette.magenta,
        palette.green,
        palette.blue,
        palette.orange,
    ];
    series[index % series.len()]
}

impl<Message: Clone + 'static> canvas::Program<Message> for Treemap<Message> {
    /// The tile under the pointer, if any.
    type State = Option<usize>;

    fn update(
        &self,
        state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let Some(position) = cursor.position_in(bounds) else {
            // Leaving the canvas has to clear the highlight, or a tile stays
            // lit after the pointer is somewhere else entirely.
            return state.take().map(|_| canvas::Action::request_redraw());
        };

        let hovered = self
            .layout(Rectangle::with_size(bounds.size()))
            .into_iter()
            .find(|tile| tile.bounds.contains(position))
            .map(|tile| tile.index);

        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                hovered.map(|index| canvas::Action::publish((self.on_select)(index)).and_capture())
            }
            _ if *state != hovered => {
                *state = hovered;
                Some(canvas::Action::request_redraw())
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        // Not cached against `state`, because the highlight changes with the
        // pointer and a cached frame would never show it.
        let mut frame = canvas::Frame::new(renderer, bounds.size());

        if self.total == 0 || self.entries.is_empty() {
            return vec![frame.into_geometry()];
        }

        for tile in self.layout(Rectangle::with_size(bounds.size())) {
            let hovered = *state == Some(tile.index);
            let inner = Rectangle {
                x: tile.bounds.x + GUTTER / 2.0,
                y: tile.bounds.y + GUTTER / 2.0,
                width: (tile.bounds.width - GUTTER).max(1.0),
                height: (tile.bounds.height - GUTTER).max(1.0),
            };

            let path = Path::rounded_rectangle(
                Point::new(inner.x, inner.y),
                IcedSize::new(inner.width, inner.height),
                4.0.into(),
            );
            frame.fill(&path, faded(tile.tint, if hovered { 0.55 } else { 0.32 }));
            frame.stroke(
                &path,
                Stroke::default()
                    .with_color(faded(tile.tint, if hovered { 1.0 } else { 0.6 }))
                    .with_width(1.0),
            );

            if inner.width < LABEL_MINIMUM || inner.height < 30.0 {
                continue;
            }

            frame.fill_text(Text {
                content: tile.label,
                position: Point::new(inner.x + 8.0, inner.y + 6.0),
                color: to_iced(self.palette.bright_foreground),
                size: 12.0.into(),
                max_width: inner.width - 16.0,
                ..Text::default()
            });
            frame.fill_text(Text {
                content: tile.detail,
                position: Point::new(inner.x + 8.0, inner.y + 22.0),
                color: to_iced(self.palette.dark_foreground),
                size: 11.0.into(),
                max_width: inner.width - 16.0,
                ..Text::default()
            });
        }

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.is_some() {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

/// Lay weights out as rectangles filling `bounds`, keeping each close to
/// square.
///
/// The squarified algorithm of Bruls, Huizing and van Wijk: take items
/// largest first, keep adding to the current row while doing so improves the
/// worst aspect ratio in it, then commit the row and carry on in what is
/// left. Rectangles come back in the order the weights were given, not the
/// order they were placed, so a caller can index straight into its own list.
fn squarify(weights: &[f64], bounds: Rectangle) -> Vec<Rectangle> {
    let mut placed = vec![Rectangle::default(); weights.len()];

    let total: f64 = weights.iter().sum();
    if total <= 0.0 || bounds.width <= 0.0 || bounds.height <= 0.0 {
        return placed;
    }

    // Largest first is what makes the result look deliberate rather than
    // shuffled.
    let mut order: Vec<usize> = (0..weights.len()).collect();
    order.sort_by(|&a, &b| weights[b].total_cmp(&weights[a]));

    // Everything is scaled into area up front, so a row's length is just a
    // division rather than a proportion of a proportion.
    let scale = f64::from(bounds.width) * f64::from(bounds.height) / total;
    let areas: Vec<f64> = order.iter().map(|&index| weights[index] * scale).collect();

    let mut free = bounds;
    let mut start = 0;

    while start < areas.len() {
        let short = f64::from(free.width.min(free.height));
        if short <= 0.0 {
            break;
        }

        // Grow the row while the worst aspect ratio in it keeps improving.
        let mut end = start + 1;
        let mut sum = areas[start];
        let mut worst = aspect(short, sum, areas[start], areas[start]);

        while end < areas.len() {
            let grown = sum + areas[end];
            let candidate = aspect(short, grown, areas[start], areas[end]);
            if candidate > worst {
                break;
            }
            sum = grown;
            worst = candidate;
            end += 1;
        }

        let thickness = (sum / short) as f32;
        let horizontal = free.width >= free.height;
        let mut offset = 0.0_f32;

        for (position, &area) in areas[start..end].iter().enumerate() {
            let length = (area / sum * short.max(f64::EPSILON)) as f32;
            let rectangle = if horizontal {
                Rectangle {
                    x: free.x,
                    y: free.y + offset,
                    width: thickness,
                    height: length,
                }
            } else {
                Rectangle {
                    x: free.x + offset,
                    y: free.y,
                    width: length,
                    height: thickness,
                }
            };
            placed[order[start + position]] = rectangle;
            offset += length;
        }

        if horizontal {
            free.x += thickness;
            free.width -= thickness;
        } else {
            free.y += thickness;
            free.height -= thickness;
        }

        start = end;
    }

    placed
}

/// The worst aspect ratio in a row of the given total area along `short`.
fn aspect(short: f64, row: f64, largest: f64, smallest: f64) -> f64 {
    if row <= 0.0 || short <= 0.0 {
        return f64::MAX;
    }
    let squared = short * short;
    let row_squared = row * row;
    let wide = squared * largest / row_squared;
    let tall = row_squared / (squared * smallest.max(f64::EPSILON));
    wide.max(tall)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rectangle = Rectangle {
        x: 0.0,
        y: 0.0,
        width: 400.0,
        height: 300.0,
    };

    fn coverage(rectangles: &[Rectangle]) -> f32 {
        rectangles.iter().map(|r| r.width * r.height).sum()
    }

    #[test]
    fn the_tiles_fill_the_area_they_were_given() {
        let placed = squarify(&[50.0, 30.0, 12.0, 5.0, 3.0], AREA);

        let expected = AREA.width * AREA.height;
        assert!(
            (coverage(&placed) - expected).abs() < expected * 0.001,
            "covered {} of {expected}",
            coverage(&placed),
        );
    }

    #[test]
    fn area_is_proportional_to_weight() {
        let placed = squarify(&[75.0, 25.0], AREA);

        let first = placed[0].width * placed[0].height;
        let second = placed[1].width * placed[1].height;

        assert!((first / second - 3.0).abs() < 0.01, "{first} vs {second}");
    }

    #[test]
    fn rectangles_come_back_in_the_order_the_weights_were_given() {
        // Smallest first on the way in; the biggest tile must still be the
        // one at the index its weight came from.
        let placed = squarify(&[1.0, 99.0], AREA);

        assert!(placed[1].width * placed[1].height > placed[0].width * placed[0].height);
    }

    #[test]
    fn tiles_are_kept_roughly_square_rather_than_slivers() {
        let placed = squarify(&[25.0, 25.0, 25.0, 25.0], AREA);

        for rectangle in &placed {
            let ratio =
                (rectangle.width / rectangle.height).max(rectangle.height / rectangle.width);
            assert!(ratio < 3.0, "aspect ratio {ratio} is a sliver");
        }
    }

    #[test]
    fn no_tiles_overlap() {
        let placed = squarify(&[40.0, 30.0, 20.0, 7.0, 3.0], AREA);

        for (i, a) in placed.iter().enumerate() {
            for b in placed.iter().skip(i + 1) {
                let separated = a.x + a.width <= b.x + 0.01
                    || b.x + b.width <= a.x + 0.01
                    || a.y + a.height <= b.y + 0.01
                    || b.y + b.height <= a.y + 0.01;
                assert!(separated, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn degenerate_input_lays_out_nothing_rather_than_panicking() {
        assert!(squarify(&[], AREA).is_empty());
        assert_eq!(squarify(&[0.0, 0.0], AREA).len(), 2);
        assert_eq!(
            squarify(
                &[1.0],
                Rectangle {
                    width: 0.0,
                    height: 0.0,
                    ..AREA
                }
            )
            .len(),
            1
        );
    }

    #[test]
    fn a_single_weight_takes_the_whole_area() {
        let placed = squarify(&[42.0], AREA);

        assert!((placed[0].width - AREA.width).abs() < 0.01);
        assert!((placed[0].height - AREA.height).abs() < 0.01);
    }
}
