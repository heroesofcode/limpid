//! The screens.

use iced::widget::{button, text};
use iced::{Element, Length};

use crate::app::Message;
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
