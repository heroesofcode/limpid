//! The screens.

use iced::widget::{Row, Space, button, checkbox, column, container, row, text};
use iced::{Alignment, Element, Length};

use limpid_core::plan::Magnitude;
use limpid_theme::{Color, Palette};

use crate::app::{Excluded, Message};
use crate::layout::{self, Metrics};
use crate::style;
use crate::typography as ty;

pub mod overview;
pub mod settings;
pub mod sidebar;
pub mod storage;

/// A button whose label is centred only when the button spans the width.
///
/// A `Fill` label inside a button sitting in a row makes the *button* take
/// the row's slack, which is right when it is the only thing on its line and
/// wrong when it is not.
pub fn action<'a>(label: &'a str, full_width: bool) -> button::Button<'a, Message> {
    let label = text(label).size(ty::BODY);
    if full_width {
        button(label.width(Length::Fill).center()).width(Length::Fill)
    } else {
        button(label)
    }
}

/// The element a row uses when nothing there can be ticked, so the text
/// still lines up with the rows that can.
pub fn tick_spacer<'a>() -> Element<'a, Message> {
    iced::widget::Space::new().width(Length::Fixed(22.0)).into()
}

/// A small coloured label.
pub fn chip<'a>(palette: Palette, label: &'a str, tint: Color) -> Element<'a, Message> {
    container(text(label).size(ty::CAPTION))
        .style(style::badge(palette, tint))
        .padding([1, 7])
        .into()
}

/// A button on a selection bar, and what pressing it does. `None` draws it
/// disabled rather than leaving it out, so the bar does not rearrange
/// itself under the pointer as the selection changes.
pub type Choice<'a> = (&'a str, Option<Message>);

/// What a quiet button's padding costs, both sides and the border.
const QUIET_FRAME: f32 = 2.0 * 14.0 + 2.0;
/// The same for the primary button.
const PRIMARY_FRAME: f32 = 2.0 * 20.0;
/// What the summary needs to stay readable beside the buttons: about
/// "12 selected, 999.9 MiB" on one line.
const SUMMARY_NEEDS: f32 = 170.0;

/// The strip that says what is selected and offers what can be done with it.
///
/// Arranged by what its buttons actually cost at this size rather than by a
/// band, because the bar grows: one file selected on the storage page offers
/// two actions a larger selection does not, and a fixed breakpoint measured
/// for the short bar would clip the long one.
///
/// - Everything on one line, when the summary still gets a readable column.
/// - The summary above, and the quiet actions in one line beside the
///   primary one.
/// - The summary, the quiet actions wrapping, and the primary action on a
///   line of its own at full width. Not wrapping beside it: a stack of
///   quiet buttons with the primary one floating next to it reads as two
///   unrelated things, and leaves the primary one the sliver.
pub fn selection_bar<'a>(
    palette: Palette,
    metrics: Metrics,
    summary: String,
    quiet: Vec<Choice<'a>>,
    primary: Choice<'a>,
) -> Element<'a, Message> {
    let cost = |label: &str| layout::text_width(label, ty::BODY_SMALL) + QUIET_FRAME;
    let quiet_total = quiet.iter().map(|(label, _)| cost(label)).sum::<f32>()
        + ty::GAP_TIGHT * quiet.len().saturating_sub(1) as f32;
    let primary_cost = layout::text_width(primary.0, ty::BODY) + PRIMARY_FRAME;
    // The bar's own padding.
    let frame = 2.0 * metrics.gap;

    let one_line =
        metrics.fits(frame + SUMMARY_NEEDS + quiet_total + ty::GAP_TIGHT * 3.0 + primary_cost);
    let beside = metrics.fits(frame + quiet_total + ty::GAP_TIGHT * 2.0 + primary_cost);

    let quiet_button = |(label, message): Choice<'a>| {
        button(text(label).size(ty::BODY_SMALL))
            .style(style::quiet_button(palette))
            .padding([8, 14])
            .on_press_maybe(message)
    };
    let primary_button = |full_width: bool| {
        action(primary.0, full_width)
            .style(style::primary_button(palette))
            .padding([10, 20])
            .on_press_maybe(primary.1.clone())
    };
    let label = text(summary).size(ty::BODY).style(style::body(palette));

    let inner: Element<'a, Message> = if one_line {
        let mut line = row![label.width(Length::Fill)]
            .spacing(ty::GAP_TIGHT)
            .align_y(Alignment::Center);
        for choice in quiet {
            line = line.push(quiet_button(choice));
        }
        line.push(Space::new().width(Length::Fixed(ty::GAP_TIGHT)))
            .push(primary_button(false))
            .into()
    } else {
        // Wrapping, so however many there are, none of them is pushed past
        // the edge — including beside the primary one, where the estimate
        // of what they cost is what put them, and an estimate can be short.
        let actions = Row::with_children(quiet.into_iter().map(|c| quiet_button(c).into()))
            .spacing(ty::GAP_TIGHT)
            .width(Length::Fill)
            .wrap()
            .vertical_spacing(ty::GAP_TIGHT);

        if beside {
            column![
                label.width(Length::Fill),
                row![actions, primary_button(false)]
                    .spacing(ty::GAP_TIGHT)
                    .align_y(Alignment::Center),
            ]
            .spacing(ty::GAP_TIGHT)
            .into()
        } else {
            column![label.width(Length::Fill), actions, primary_button(true)]
                .spacing(ty::GAP_TIGHT)
                .width(Length::Fill)
                .into()
        }
    };

    container(inner)
        .style(style::card(palette))
        .padding(metrics.gap)
        .width(Length::Fill)
        .into()
}

/// What was just excluded, with the way back.
///
/// Excluding is not destructive, so it acts at once rather than asking
/// first — and this is why that is safe: the undo is right there, in the
/// place the person was looking when they did it.
pub fn excluded<'a>(
    palette: Palette,
    metrics: Metrics,
    excluded: &Excluded,
) -> Element<'a, Message> {
    let them = if excluded.names.len() == 1 {
        "it"
    } else {
        "them"
    };
    let said = format!(
        "Excluded {}. Limpid will leave {them} alone from now on; Settings lists \
         everything excluded.",
        excluded.describe(),
    );

    let undo = button(text("Undo").size(ty::BODY_SMALL))
        .style(style::quiet_button(palette))
        .padding([6, 14])
        .on_press(Message::Include(excluded.paths.clone()));

    let words = row![
        text("\u{2713}")
            .size(ty::BODY_SMALL)
            .style(style::tinted(palette.green)),
        text(said)
            .size(ty::BODY_SMALL)
            .style(style::body(palette))
            .wrapping(text::Wrapping::WordOrGlyph)
            .width(Length::Fill),
    ]
    .spacing(ty::GAP_TIGHT)
    .width(Length::Fill);

    let inner: Element<'a, Message> = if metrics.two_columns() {
        row![words, undo]
            .spacing(ty::GAP)
            .align_y(Alignment::Center)
            .into()
    } else {
        column![words, undo].spacing(ty::GAP_TIGHT).into()
    };

    container(inner)
        .style(style::well(palette))
        .padding(metrics.gap)
        .width(Length::Fill)
        .into()
}

/// The extra step a large plan needs: what makes it large, and a box to say
/// the list above was read.
///
/// A box rather than a second dialog. The list it asks about is right
/// above it, and a dialog would cover the very thing it asks about.
pub fn large<'a>(
    palette: Palette,
    metrics: Metrics,
    magnitude: &Magnitude,
    checked: bool,
    on_toggle: fn(bool) -> Message,
) -> Element<'a, Message> {
    container(
        column![
            text(magnitude.describe())
                .size(ty::BODY_SMALL)
                .style(style::tinted(palette.orange))
                .width(Length::Fill),
            checkbox(checked)
                .label("I have read the list")
                .size(16)
                .text_size(ty::BODY_SMALL)
                .style(style::tick(palette))
                .on_toggle(on_toggle),
        ]
        .spacing(ty::GAP_TIGHT)
        .width(Length::Fill),
    )
    .style(style::well(palette))
    .padding(metrics.gap)
    .width(Length::Fill)
    .into()
}

/// A change to the settings that did not stick, said where it was tried.
pub fn failed<'a>(palette: Palette, metrics: Metrics, why: &str) -> Element<'a, Message> {
    container(
        row![
            text("\u{2717}")
                .size(ty::BODY_SMALL)
                .style(style::tinted(palette.red)),
            text(why.to_owned())
                .size(ty::BODY_SMALL)
                .style(style::body(palette))
                .wrapping(text::Wrapping::WordOrGlyph)
                .width(Length::Fill),
        ]
        .spacing(ty::GAP_TIGHT),
    )
    .style(style::well(palette))
    .padding(metrics.gap)
    .width(Length::Fill)
    .into()
}
