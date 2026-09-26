//! Where the space went.

use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Length};

use limpid_core::analyse::{Breakdown, Entry};
use limpid_core::size::human;
use limpid_theme::Palette;

use crate::app::{Message, Storage, placeholder};
use crate::layout::Metrics;
use crate::style;
use crate::typography as ty;
use crate::widget::treemap::Treemap;

/// Draw the storage page.
pub fn view<'a>(palette: Palette, metrics: Metrics, storage: &'a Storage) -> Element<'a, Message> {
    if storage.working {
        let where_ = storage
            .current()
            .and_then(|path| path.file_name())
            .map(|name| format!("Measuring {}\u{2026}", name.to_string_lossy()))
            .unwrap_or_else(|| "Measuring\u{2026}".to_owned());
        return placeholder(palette, where_);
    }

    let Some(survey) = &storage.survey else {
        return placeholder(palette, "Nothing measured yet.");
    };

    let mut body = column![breadcrumb(palette, metrics, storage)]
        .spacing(metrics.gap)
        .width(Length::Fill);

    if survey.breakdown.is_empty() {
        return body
            .push(
                container(
                    text("This directory is empty.")
                        .size(ty::BODY)
                        .style(style::secondary(palette)),
                )
                .style(style::card(palette))
                .padding(metrics.card)
                .width(Length::Fill),
            )
            .into();
    }

    body = body.push(map(palette, metrics, &survey.breakdown));
    body = body.push(children(palette, metrics, &survey.breakdown));

    if !survey.largest.is_empty() {
        body = body.push(largest(palette, metrics, &survey.largest));
    }

    if survey.breakdown.unreadable > 0 {
        body = body.push(
            container(
                text(format!(
                    "{} paths could not be read, so these figures are a lower bound.",
                    survey.breakdown.unreadable,
                ))
                .size(ty::CAPTION)
                .style(style::secondary(palette))
                .width(Length::Fill),
            )
            .style(style::well(palette))
            .padding(metrics.gap)
            .width(Length::Fill),
        );
    }

    body.into()
}

/// The path from the starting directory to this one, each part clickable.
///
/// Trimmed from the left when there is no room: the directory you are in
/// matters more than the one you started from, and an ellipsis says the rest
/// is still there.
fn breadcrumb<'a>(
    palette: Palette,
    metrics: Metrics,
    storage: &'a Storage,
) -> Element<'a, Message> {
    // Roughly what a crumb costs, in pixels, at this type size.
    const CRUMB: f32 = 110.0;

    let total = storage.trail.len();
    let room = ((metrics.content / CRUMB).floor() as usize).max(1);
    let first = total.saturating_sub(room);

    let mut trail = row![].spacing(4).align_y(Alignment::Center);

    if first > 0 {
        trail = trail.push(
            button(text("\u{2026}").size(ty::BODY_SMALL))
                .style(style::nav_button(palette, false))
                .padding([4, 8])
                .on_press(Message::Ascend(0)),
        );
    }

    for (depth, path) in storage.trail.iter().enumerate().skip(first) {
        if depth > first {
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

        if depth + 1 == total {
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

    // Wrapped as well as trimmed: two crumbs can still be long enough to
    // need a second line.
    trail.wrap().into()
}

/// The treemap itself.
fn map<'a>(palette: Palette, metrics: Metrics, breakdown: &Breakdown) -> Element<'a, Message> {
    let total = breakdown.total().on_disk;

    container(
        column![
            row![
                text("Largest first, by area")
                    .size(ty::BODY_SMALL)
                    .style(style::secondary(palette))
                    .width(Length::Fill),
                text(human(total))
                    .size(ty::SUBTITLE)
                    .style(style::body(palette)),
            ]
            .align_y(Alignment::Center),
            Treemap::new(palette, breakdown.children.clone(), Message::Descend)
                .view(metrics.treemap()),
        ]
        .spacing(ty::GAP_TIGHT),
    )
    .style(style::card(palette))
    .padding(metrics.card)
    .width(Length::Fill)
    .into()
}

/// The same level as a list, which the treemap cannot show for small items.
fn children<'a>(palette: Palette, metrics: Metrics, breakdown: &Breakdown) -> Element<'a, Message> {
    let total = breakdown.total().on_disk;
    let mut rows = column![].spacing(2).width(Length::Fill);

    for (index, child) in breakdown.children.iter().take(12).enumerate() {
        let name = format!("{}{}", child.name, if child.is_dir { "/" } else { "" });
        let share = text(format!("{:.0}%", child.share_of(total) * 100.0))
            .size(ty::CAPTION)
            .style(style::secondary(palette));
        let size = text(human(child.size.on_disk))
            .size(ty::BODY_SMALL)
            .style(style::body(palette));

        // The percentage is the first thing to go: the bar in the treemap
        // above already says the same thing, and the size does not.
        let line: Element<'a, Message> = if metrics.two_columns() {
            row![
                text(name)
                    .size(ty::BODY_SMALL)
                    .style(style::body(palette))
                    .width(Length::Fill),
                share.width(Length::Fixed(44.0)).align_x(Alignment::End),
                size.width(Length::Fixed(76.0)).align_x(Alignment::End),
            ]
            .spacing(ty::GAP_TIGHT)
            .align_y(Alignment::Center)
            .into()
        } else {
            row![
                text(name)
                    .size(ty::BODY_SMALL)
                    .style(style::body(palette))
                    .width(Length::Fill),
                size,
            ]
            .spacing(ty::GAP_TIGHT)
            .align_y(Alignment::Center)
            .into()
        };

        let entry: Element<'a, Message> = if child.is_dir {
            button(line)
                .style(style::nav_button(palette, false))
                .padding([7, 10])
                .width(Length::Fill)
                .on_press(Message::Descend(index))
                .into()
        } else {
            container(line).padding([7, 10]).width(Length::Fill).into()
        };

        rows = rows.push(entry);
    }

    container(rows)
        .style(style::card(palette))
        .padding(metrics.gap)
        .width(Length::Fill)
        .into()
}

/// The largest individual files anywhere below here.
fn largest<'a>(palette: Palette, metrics: Metrics, entries: &[Entry]) -> Element<'a, Message> {
    let mut rows = column![].spacing(6).width(Length::Fill);

    for entry in entries {
        // The parent directory, not the whole path: at 300 px a full path
        // wraps to four lines and says less than the last component of it.
        let where_ = entry
            .path
            .parent()
            .map(|parent| parent.display().to_string())
            .unwrap_or_default();

        let name = column![
            text(entry.name.clone())
                .size(ty::BODY_SMALL)
                .style(style::body(palette)),
            text(where_)
                .size(ty::CAPTION)
                .style(style::secondary(palette))
                .width(Length::Fill),
        ]
        .spacing(2)
        .width(Length::Fill);

        let size = text(human(entry.size.on_disk))
            .size(ty::BODY_SMALL)
            .style(style::body(palette));

        rows = rows.push(
            row![name, size]
                .spacing(ty::GAP_TIGHT)
                .align_y(Alignment::Center),
        );
    }

    let _ = metrics;

    container(
        column![
            text("Largest files")
                .size(ty::TITLE)
                .style(style::heading(palette)),
            text("Anywhere below this directory, not just directly in it.")
                .size(ty::BODY_SMALL)
                .style(style::secondary(palette))
                .width(Length::Fill),
            Space::new().height(Length::Fixed(ty::GAP_TIGHT)),
            rows,
        ]
        .spacing(2)
        .width(Length::Fill),
    )
    .style(style::card(palette))
    .padding(metrics.card)
    .width(Length::Fill)
    .into()
}
