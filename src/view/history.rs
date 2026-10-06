//! What Limpid removed, when, and where it went.

use std::path::Path;

use iced::widget::text::Wrapping;
use iced::widget::{Column, column, container, row, text};
use iced::{Alignment, Element, Length};

use limpid_core::config::contract;
use limpid_core::history::{Entry, Read, Run};
use limpid_core::size::human;
use limpid_theme::Palette;

use crate::app::{Message, State};
use crate::layout::Metrics;
use crate::style;
use crate::typography as ty;
use crate::view;

/// Runs drawn before the rest are counted instead. Hundreds of cards make a
/// page nobody reads to the end and a frame that is slow to lay out; the
/// file keeps every one, and the command line lists them all.
const RUNS_SHOWN: usize = 50;

/// Entries listed under a run before the rest are counted instead.
const ENTRIES_SHOWN: usize = 6;

/// Draw the history page.
pub fn view<'a>(palette: Palette, metrics: Metrics, state: &'a State) -> Element<'a, Message> {
    let home = state.home();
    let page = column![].spacing(metrics.gap).width(Length::Fill);

    let read = match state.history() {
        None => {
            return page
                .push(
                    text("Reading the history\u{2026}")
                        .size(ty::BODY_SMALL)
                        .style(style::secondary(palette)),
                )
                .into();
        }
        Some(Err(why)) => return page.push(view::failed(palette, metrics, why)).into(),
        Some(Ok(read)) => read,
    };

    let mut page = page;
    if read.runs.is_empty() {
        page = page.push(nothing_yet(palette, metrics));
    }
    for run in read.runs.iter().take(RUNS_SHOWN) {
        page = page.push(card(palette, metrics, run, home));
    }
    page.push(footer(palette, read, &state.history_path(), home))
        .into()
}

/// What the page says before there is anything on it.
fn nothing_yet<'a>(palette: Palette, metrics: Metrics) -> Element<'a, Message> {
    container(
        column![
            text("Nothing removed yet")
                .size(ty::TITLE)
                .style(style::heading(palette)),
            text(
                "Each time Limpid removes something, it is written down here: what it \
                 was, how large, whether it went to the trash or was removed for good, \
                 and when.",
            )
            .size(ty::BODY_SMALL)
            .style(style::secondary(palette))
            .width(Length::Fill),
        ]
        .spacing(2)
        .width(Length::Fill),
    )
    .style(style::card(palette))
    .padding(metrics.card)
    .width(Length::Fill)
    .into()
}

/// One run: when, how much and which way, then what.
fn card<'a>(palette: Palette, metrics: Metrics, run: &'a Run, home: &Path) -> Element<'a, Message> {
    let mut body = column![
        text(run.when())
            .size(ty::SUBTITLE)
            .style(style::heading(palette)),
        text(run.summary())
            .size(ty::BODY_SMALL)
            .style(style::secondary(palette))
            .width(Length::Fill),
    ]
    .spacing(2)
    .width(Length::Fill);

    let mut list = Column::new().spacing(ty::GAP_TIGHT).width(Length::Fill);
    // Which way each one went is only worth saying when they did not all
    // go the same way; otherwise the summary has said it.
    let mixed = run.disposal().is_none();
    for entry in run.entries.iter().take(ENTRIES_SHOWN) {
        list = list.push(entry_row(palette, entry, home, mixed));
    }
    if run.entries.len() > ENTRIES_SHOWN {
        list = list.push(
            text(format!("and {} more", run.entries.len() - ENTRIES_SHOWN))
                .size(ty::CAPTION)
                .style(style::secondary(palette)),
        );
    }
    for done in &run.operations {
        list = list.push(line(
            palette,
            done.succeeded,
            format!("{}: {}", done.operation.describe(), done.detail),
        ));
    }
    for problem in &run.problems {
        list = list.push(line(palette, false, problem.clone()));
    }

    body = body.push(iced::widget::Space::new().height(Length::Fixed(ty::GAP_TIGHT)));
    body = body.push(list);

    container(body)
        .style(style::card(palette))
        .padding(metrics.card)
        .width(Length::Fill)
        .into()
}

/// One path in a run: its name and size, and where it was.
fn entry_row<'a>(
    palette: Palette,
    entry: &'a Entry,
    home: &Path,
    mixed: bool,
) -> Element<'a, Message> {
    // The name takes what the size leaves, and breaks anywhere: a file name
    // is one long token as often as not.
    let top = row![
        text(entry.name.as_str())
            .size(ty::BODY_SMALL)
            .style(style::body(palette))
            .wrapping(Wrapping::WordOrGlyph)
            .width(Length::Fill),
        text(human(entry.size.on_disk))
            .size(ty::BODY_SMALL)
            .style(style::body(palette)),
    ]
    .spacing(ty::GAP_TIGHT)
    .align_y(Alignment::Start)
    .width(Length::Fill);

    let said = column![top].spacing(2).width(Length::Fill);

    // Where it was. For something chosen by name the name is already on
    // the line above, so the folder it was in is what is left to say.
    let named = entry
        .path
        .file_name()
        .is_some_and(|name| name.to_string_lossy() == entry.name);
    let mut under = match entry.path.parent() {
        Some(folder) if named => format!("in {}", contract(folder, home)),
        _ => contract(&entry.path, home),
    };
    if mixed {
        under.push_str(" \u{b7} ");
        under.push_str(entry.disposal.describe());
    }

    said.push(
        text(under)
            .size(ty::CAPTION)
            .style(style::secondary(palette))
            .wrapping(Wrapping::WordOrGlyph)
            .width(Length::Fill),
    )
    .into()
}

/// A line with a mark before it saying whether it worked.
fn line<'a>(palette: Palette, good: bool, said: String) -> Element<'a, Message> {
    row![
        text(if good { "\u{2713}" } else { "\u{2717}" })
            .size(ty::BODY_SMALL)
            .style(style::tinted(if good {
                palette.green
            } else {
                palette.red
            })),
        text(said)
            .size(ty::BODY_SMALL)
            .style(style::body(palette))
            .wrapping(Wrapping::WordOrGlyph)
            .width(Length::Fill),
    ]
    .spacing(ty::GAP_TIGHT)
    .width(Length::Fill)
    .into()
}

/// Where the history is kept, and what of it is not shown.
fn footer<'a>(palette: Palette, read: &Read, path: &Path, home: &Path) -> Element<'a, Message> {
    let mut said = column![
        text(format!(
            "Kept in {}. Limpid adds to it and never rewrites it.",
            contract(path, home),
        ))
        .size(ty::CAPTION)
        .style(style::secondary(palette))
        .wrapping(Wrapping::WordOrGlyph)
        .width(Length::Fill),
    ]
    .spacing(4)
    .width(Length::Fill);

    if read.runs.len() > RUNS_SHOWN {
        said = said.push(
            text(format!(
                "{} older runs are in the file and not drawn here; `limpid-cli history \
                 --last 0` lists every one.",
                read.runs.len() - RUNS_SHOWN,
            ))
            .size(ty::CAPTION)
            .style(style::secondary(palette))
            .width(Length::Fill),
        );
    }
    if read.skipped > 0 {
        said = said.push(
            text(format!(
                "{} could not be read, and {} not shown. The file is left as it is.",
                if read.skipped == 1 {
                    "One line".to_owned()
                } else {
                    format!("{} lines", read.skipped)
                },
                if read.skipped == 1 { "is" } else { "are" },
            ))
            .size(ty::CAPTION)
            .style(style::tinted(palette.orange))
            .width(Length::Fill),
        );
    }

    said.into()
}
