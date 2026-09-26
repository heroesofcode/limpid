//! Navigation, in whichever shape fits.
//!
//! A column beside the content when the window is wide enough to spare
//! 216 px, and a row of pills above it when it is not. Below about 720 px a
//! fixed column is a large fraction of the window, and what remains cannot
//! hold a card and its figures side by side.

use iced::widget::{Space, button, column as col, container, row, rule, scrollable, text};
use iced::{Alignment, Element, Length};

use limpid_theme::{Palette, Source};

use crate::app::{Message, Page};
use crate::layout::{Metrics, Width};
use crate::style;
use crate::typography as ty;

/// The navigation column, for windows with room for it.
pub fn column<'a>(palette: Palette, current: Page, source: &Source) -> Element<'a, Message> {
    let mut items = col![].spacing(2).width(Length::Fill);

    for page in Page::ALL {
        items = items.push(
            button(text(page.title()).size(ty::BODY))
                .style(style::nav_button(palette, page == current))
                .padding([9, 12])
                .width(Length::Fill)
                .on_press(Message::Navigate(page)),
        );
    }

    container(
        col![
            brand(palette, ty::TITLE),
            Space::new().height(Length::Fixed(ty::GAP_WIDE)),
            items,
            Space::new().height(Length::Fill),
            container(rule::horizontal(1.0).style(style::divider(palette))).padding([0, 12]),
            Space::new().height(Length::Fixed(ty::GAP_TIGHT)),
            container(
                text(source.describe())
                    .size(ty::CAPTION)
                    .style(style::secondary(palette)),
            )
            .padding([0, 12]),
        ]
        .spacing(0)
        .padding(ty::GAP),
    )
    .style(style::sidebar(palette))
    .width(Length::Fixed(ty::SIDEBAR_WIDTH))
    .height(Length::Fill)
    .into()
}

/// The navigation row, for windows without.
///
/// Scrolls horizontally rather than wrapping or truncating: at the narrowest
/// sizes three pills still do not fit, and a tab you cannot reach is worse
/// than one you have to scroll to.
pub fn tabs<'a>(palette: Palette, metrics: Metrics, current: Page) -> Element<'a, Message> {
    let mut items = row![].spacing(4).align_y(Alignment::Center);

    for page in Page::ALL {
        items = items.push(
            button(text(page.title()).size(ty::BODY_SMALL))
                .style(style::nav_button(palette, page == current))
                .padding([7, 12])
                .on_press(Message::Navigate(page)),
        );
    }

    // The wordmark is the first thing to go: at the narrowest widths the
    // tabs are what the row is for.
    let bar: Element<'a, Message> = if metrics.width == Width::Tiny {
        items.into()
    } else {
        row![
            brand(palette, ty::BODY),
            Space::new().width(Length::Fixed(ty::GAP)),
            items
        ]
        .align_y(Alignment::Center)
        .into()
    };

    container(
        scrollable(container(bar).padding([ty::GAP_TIGHT, metrics.margin]))
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::new().width(3).scroller_width(3),
            ))
            .style(style::scroller(palette)),
    )
    .style(style::sidebar(palette))
    .width(Length::Fill)
    .into()
}

/// The wordmark: a filled dot in the accent colour, and the name.
///
/// The one place the accent appears without meaning "act on this".
fn brand<'a>(palette: Palette, size: f32) -> Element<'a, Message> {
    row![
        text("\u{25cf}")
            .size(size * 0.7)
            .style(style::tinted(palette.accent)),
        text("Limpid").size(size).style(style::heading(palette)),
    ]
    .spacing(ty::GAP_TIGHT)
    .align_y(Alignment::Center)
    .into()
}
