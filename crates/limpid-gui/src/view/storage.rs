//! Where the space went.

use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Length};

use limpid_core::analyse::{Breakdown, Entry};
use limpid_core::size::human;
use limpid_theme::Palette;

use crate::app::{Message, Storage, placeholder};
use crate::style;
use crate::typography as ty;
use crate::widget::treemap::Treemap;

/// Height of the treemap.
const MAP_HEIGHT: f32 = 320.0;

/// Draw the storage page.
pub fn view<'a>(palette: Palette, storage: &'a Storage) -> Element<'a, Message> {
    if storage.working {
        let where_ = storage
            .current()
            .map(|path| format!("Measuring {}\u{2026}", path.display()))
            .unwrap_or_else(|| "Measuring\u{2026}".to_owned());
        return placeholder(palette, where_);
    }

    let Some(survey) = &storage.survey else {
        return placeholder(palette, "Nothing measured yet.");
    };

    let mut body = column![breadcrumb(palette, storage)].spacing(ty::GAP);

    if survey.breakdown.is_empty() {
        body = body.push(
            container(
                text("This directory is empty.")
                    .size(ty::BODY)
                    .style(style::secondary(palette)),
            )
            .style(style::card(palette))
            .padding(ty::GAP_WIDE)
            .width(Length::Fill),
        );
        return body.into();
    }

    body = body.push(map(palette, &survey.breakdown));
    body = body.push(children(palette, &survey.breakdown));

    if !survey.largest.is_empty() {
        body = body.push(largest(palette, &survey.largest));
    }

    if survey.breakdown.unreadable > 0 {
        body = body.push(
            container(
                text(format!(
                    "{} paths could not be read, so these figures are a lower bound.",
                    survey.breakdown.unreadable,
                ))
                .size(ty::CAPTION)
                .style(style::secondary(palette)),
            )
            .style(style::well(palette))
            .padding(ty::GAP)
            .width(Length::Fill),
        );
    }

    body.into()
}

/// The path from the starting directory to this one, each part clickable.
fn breadcrumb<'a>(palette: Palette, storage: &'a Storage) -> Element<'a, Message> {
    let mut trail = row![].spacing(4).align_y(Alignment::Center);

    for (depth, path) in storage.trail.iter().enumerate() {
        if depth > 0 {
            trail = trail.push(
                text("\u{203a}")
                    .size(ty::BODY_SMALL)
                    .style(style::secondary(palette)),
            );
        }

        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let last = depth + 1 == storage.trail.len();

        if last {
            trail = trail.push(
                container(
                    text(name)
                        .size(ty::BODY_SMALL)
                        .style(style::heading(palette)),
                )
                .padding([4, 8]),
            );
        } else {
            trail = trail.push(
                button(text(name).size(ty::BODY_SMALL))
                    .style(style::nav_button(palette, false))
                    .padding([4, 8])
                    .on_press(Message::Ascend(depth)),
            );
        }
    }

    trail.into()
}

/// The treemap itself.
fn map<'a>(palette: Palette, breakdown: &Breakdown) -> Element<'a, Message> {
    let total = breakdown.total().on_disk;

    container(
        column![
            row![
                text("Largest first, by area")
                    .size(ty::BODY_SMALL)
                    .style(style::secondary(palette)),
                Space::new().width(Length::Fill),
                text(human(total))
                    .size(ty::SUBTITLE)
                    .style(style::body(palette)),
            ]
            .align_y(Alignment::Center),
            Treemap::new(palette, breakdown.children.clone(), Message::Descend).view(MAP_HEIGHT),
        ]
        .spacing(ty::GAP_TIGHT),
    )
    .style(style::card(palette))
    .padding(ty::GAP_WIDE)
    .width(Length::Fill)
    .into()
}

/// The same level as a list, which the treemap cannot show for small items.
fn children<'a>(palette: Palette, breakdown: &Breakdown) -> Element<'a, Message> {
    let total = breakdown.total().on_disk;
    let mut rows = column![].spacing(2);

    for (index, child) in breakdown.children.iter().take(12).enumerate() {
        let name = format!("{}{}", child.name, if child.is_dir { "/" } else { "" });
        let line = row![
            text(name)
                .size(ty::BODY_SMALL)
                .style(style::body(palette))
                .width(Length::Fill),
            text(format!("{:.0}%", child.share_of(total) * 100.0))
                .size(ty::CAPTION)
                .style(style::secondary(palette))
                .width(Length::Fixed(44.0))
                .align_x(Alignment::End),
            text(human(child.size.on_disk))
                .size(ty::BODY_SMALL)
                .style(style::body(palette))
                .width(Length::Fixed(80.0))
                .align_x(Alignment::End),
        ]
        .spacing(ty::GAP_TIGHT)
        .align_y(Alignment::Center);

        let entry: Element<'a, Message> = if child.is_dir {
            button(line)
                .style(style::nav_button(palette, false))
                .padding([7, 12])
                .width(Length::Fill)
                .on_press(Message::Descend(index))
                .into()
        } else {
            container(line).padding([7, 12]).width(Length::Fill).into()
        };

        rows = rows.push(entry);
    }

    container(rows)
        .style(style::card(palette))
        .padding(ty::GAP)
        .width(Length::Fill)
        .into()
}

/// The largest individual files anywhere below here.
fn largest<'a>(palette: Palette, entries: &[Entry]) -> Element<'a, Message> {
    let mut rows = column![].spacing(6);

    for entry in entries {
        rows = rows.push(
            row![
                column![
                    text(entry.name.clone())
                        .size(ty::BODY_SMALL)
                        .style(style::body(palette)),
                    text(entry.path.display().to_string())
                        .size(ty::CAPTION)
                        .style(style::secondary(palette)),
                ]
                .spacing(2)
                .width(Length::Fill),
                text(human(entry.size.on_disk))
                    .size(ty::BODY_SMALL)
                    .style(style::body(palette)),
            ]
            .spacing(ty::GAP)
            .align_y(Alignment::Center),
        );
    }

    container(
        column![
            text("Largest files")
                .size(ty::TITLE)
                .style(style::heading(palette)),
            text("Anywhere below this directory, not just directly in it.")
                .size(ty::BODY_SMALL)
                .style(style::secondary(palette)),
            Space::new().height(Length::Fixed(ty::GAP_TIGHT)),
            rows,
        ]
        .spacing(2),
    )
    .style(style::card(palette))
    .padding(ty::GAP_WIDE)
    .width(Length::Fill)
    .into()
}
