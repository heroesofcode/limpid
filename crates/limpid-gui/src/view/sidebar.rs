//! The navigation column.

use iced::widget::{Space, button, column, container, rule, text};
use iced::{Alignment, Element, Length};

use limpid_theme::{Palette, Source};

use crate::app::{Message, Page};
use crate::style;
use crate::typography as ty;

/// Draw the sidebar.
pub fn view<'a>(palette: Palette, current: Page, source: &Source) -> Element<'a, Message> {
    let mut items = column![].spacing(2).width(Length::Fill);

    for page in Page::ALL {
        items = items.push(
            button(text(page.title()).size(ty::BODY))
                .style(style::nav_button(palette, page == current))
                .padding([9, 12])
                .width(Length::Fill)
                .on_press(Message::Navigate(page)),
        );
    }

    // The wordmark carries a filled dot in the accent colour: the one place
    // the accent appears without meaning "act on this".
    let brand = container(
        iced::widget::row![
            text("\u{25cf}")
                .size(ty::BODY_SMALL)
                .style(style::tinted(palette.accent)),
            text("Limpid")
                .size(ty::TITLE)
                .style(style::heading(palette)),
        ]
        .spacing(ty::GAP_TIGHT)
        .align_y(Alignment::Center),
    )
    .padding([0, 12]);

    let footer = container(
        text(source.describe())
            .size(ty::CAPTION)
            .style(style::secondary(palette)),
    )
    .padding([0, 12]);

    container(
        column![
            brand,
            Space::new().height(Length::Fixed(ty::GAP_WIDE)),
            items,
            Space::new().height(Length::Fill),
            container(rule::horizontal(1.0).style(style::divider(palette))).padding([0, 12]),
            Space::new().height(Length::Fixed(ty::GAP_TIGHT)),
            footer,
        ]
        .spacing(0)
        .padding(ty::GAP),
    )
    .style(style::sidebar(palette))
    .width(Length::Fixed(ty::SIDEBAR_WIDTH))
    .height(Length::Fill)
    .into()
}
