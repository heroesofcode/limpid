//! What Limpid removed, when, and where it went — and putting it back.

use std::path::Path;

use iced::widget::text::Wrapping;
use iced::widget::{Column, Space, button, column, container, row, text};
use iced::{Alignment, Element, Length};

use limpid_core::config::contract;
use limpid_core::history::{Entry, Read, Run};
use limpid_core::restore::{self, Fate, Report};
use limpid_core::size::human;
use limpid_theme::Palette;

use crate::app::{Message, Restore, State};
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
        page = page.push(card(palette, metrics, run, read, home, state));
        if let Some(banner) = alongside(palette, metrics, run.at, home, state) {
            page = page.push(banner);
        }
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
fn card<'a>(
    palette: Palette,
    metrics: Metrics,
    run: &'a Run,
    read: &'a Read,
    home: &Path,
    state: &'a State,
) -> Element<'a, Message> {
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
        list = list.push(entry_row(
            palette,
            entry,
            home,
            mixed,
            restored_note(entry, read),
        ));
    }
    if run.entries.len() > ENTRIES_SHOWN {
        list = list.push(
            text(format!("and {} more", run.entries.len() - ENTRIES_SHOWN))
                .size(ty::CAPTION)
                .style(style::secondary(palette)),
        );
    }
    for item in &run.restored {
        list = list.push(line(
            palette,
            true,
            format!("put back {}", contract(&item.original, home)),
        ));
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

    // Offered on the run, not on each file: the confirmation lists every
    // path, and a run is the unit the history is written in. Hidden on
    // this run while its confirmation, result or failure is up, so there
    // are not two ways to start the same restore. Hidden on every run
    // while a check or an apply is in flight: a second Put back would
    // replace the one whose result has not arrived.
    let pending = !run.pending(read).is_empty();
    let in_flight = state.restore().is_some_and(Restore::in_flight);
    let engaged = matches!(
        state.restore(),
        Some(
            Restore::Confirming(Report { at, .. })
                | Restore::Done(Report { at, .. })
                | Restore::Failed { at, .. }
        ) if *at == run.at
    );
    if pending && !in_flight && !engaged {
        body = body.push(Space::new().height(Length::Fixed(ty::GAP_TIGHT)));
        body = body.push(
            button(text("Put back").size(ty::BODY_SMALL))
                .style(style::quiet_button(palette))
                .padding([8, 14])
                .on_press(Message::AskToRestore(run.at)),
        );
    }

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
    note: Option<&str>,
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
    if let Some(note) = note {
        under.push_str(" \u{b7} ");
        under.push_str(note);
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

/// How much of an entry has already been put back, if any of it has.
fn restored_note(entry: &Entry, read: &Read) -> Option<&'static str> {
    if entry.trashed.is_empty() {
        return None;
    }
    let restored = entry
        .trashed
        .iter()
        .filter(|item| read.was_restored(item))
        .count();
    if restored == 0 {
        None
    } else if restored == entry.trashed.len() {
        Some("put back")
    } else {
        Some("partly put back")
    }
}

/// The confirmation or the result for this run, directly under its card.
///
/// Under the card rather than at the top of the page: the button that
/// opened it may be a long way down a list, and a tiling window does not
/// scroll there for us.
fn alongside<'a>(
    palette: Palette,
    metrics: Metrics,
    at: u64,
    home: &Path,
    state: &'a State,
) -> Option<Element<'a, Message>> {
    let phase = state.restore()?;
    let same = match phase {
        Restore::Checking { at: run }
        | Restore::Applying { at: run }
        | Restore::Failed { at: run, .. } => *run == at,
        Restore::Confirming(report) | Restore::Done(report) => report.at == at,
    };
    if !same {
        return None;
    }
    Some(match phase {
        Restore::Checking { .. } => status(palette, metrics, "Checking the trash\u{2026}"),
        Restore::Applying { .. } => status(palette, metrics, "Putting it back\u{2026}"),
        Restore::Failed { why, .. } => view::failed(palette, metrics, why),
        Restore::Confirming(report) => confirmation(palette, metrics, report, home),
        Restore::Done(report) => outcome(palette, metrics, report, home),
    })
}

fn status<'a>(palette: Palette, metrics: Metrics, said: &'a str) -> Element<'a, Message> {
    container(
        text(said)
            .size(ty::BODY_SMALL)
            .style(style::secondary(palette))
            .width(Length::Fill),
    )
    .style(style::card(palette))
    .padding(metrics.card)
    .width(Length::Fill)
    .into()
}

/// Exactly what would go back where, and what would be refused.
fn confirmation<'a>(
    palette: Palette,
    metrics: Metrics,
    report: &'a Report,
    home: &Path,
) -> Element<'a, Message> {
    let ready = report
        .attempts
        .iter()
        .filter(|attempt| matches!(attempt.fate, Fate::PutBack))
        .count();
    let title = match ready {
        0 => "Nothing to put back".to_owned(),
        1 => "Put this back?".to_owned(),
        count => format!("Put {count} back?"),
    };

    let mut lines = column![].spacing(ty::GAP_TIGHT).width(Length::Fill);
    for attempt in &report.attempts {
        lines = lines.push(attempt_line(palette, attempt, home, false));
    }

    let stacked = !metrics.buttons_inline();
    let cancel = view::action("Cancel", stacked)
        .style(style::quiet_button(palette))
        .padding([10, 18])
        .on_press(Message::CancelRestore);
    let put_back = view::action("Put back", stacked)
        .style(style::primary_button(palette))
        .padding([10, 22])
        .on_press_maybe(report.puts_back().then_some(Message::ConfirmRestore));
    // Cancel stays first either way, so the one that moves files is never
    // where the safe one was a moment ago.
    let actions: Element<'a, Message> = if metrics.buttons_inline() {
        row![Space::new().width(Length::Fill), cancel, put_back]
            .spacing(ty::GAP_TIGHT)
            .into()
    } else {
        column![cancel, put_back]
            .spacing(ty::GAP_TIGHT)
            .width(Length::Fill)
            .into()
    };

    container(
        column![
            text(title)
                .size(ty::TITLE)
                .style(style::heading(palette))
                .width(Length::Fill),
            text(
                "Checked again when you confirm. Anything that is occupied, or that \
                 is no longer the file that was sent, stays in the trash.",
            )
            .size(ty::BODY_SMALL)
            .style(style::secondary(palette))
            .width(Length::Fill),
            Space::new().height(Length::Fixed(ty::GAP_TIGHT)),
            lines,
            Space::new().height(Length::Fixed(ty::GAP)),
            actions,
        ]
        .spacing(4)
        .width(Length::Fill),
    )
    .style(style::card(palette))
    .padding(metrics.card)
    .width(Length::Fill)
    .into()
}

/// What was put back, and what was refused.
fn outcome<'a>(
    palette: Palette,
    metrics: Metrics,
    report: &'a Report,
    home: &Path,
) -> Element<'a, Message> {
    let title = if report.puts_back() {
        "Put back"
    } else {
        "Nothing put back"
    };
    let mut lines = column![].spacing(ty::GAP_TIGHT).width(Length::Fill);
    for attempt in &report.attempts {
        lines = lines.push(attempt_line(palette, attempt, home, true));
    }
    let mut body = column![
        text(title)
            .size(ty::TITLE)
            .style(style::heading(palette))
            .width(Length::Fill),
        lines,
    ]
    .spacing(ty::GAP_TIGHT)
    .width(Length::Fill);

    for problem in &report.problems {
        body = body.push(
            text(problem.clone())
                .size(ty::CAPTION)
                .style(style::tinted(palette.orange))
                .wrapping(Wrapping::WordOrGlyph)
                .width(Length::Fill),
        );
    }
    if let limpid_core::history::Recorded::Failed(why) = &report.recorded {
        body = body.push(
            text(format!(
                "This was done, but not written to the history: {why}"
            ))
            .size(ty::CAPTION)
            .style(style::tinted(palette.orange))
            .wrapping(Wrapping::WordOrGlyph)
            .width(Length::Fill),
        );
    }

    container(body)
        .style(style::card(palette))
        .padding(metrics.card)
        .width(Length::Fill)
        .into()
}

/// One file in a confirmation or a result: its name, and where it goes.
fn attempt_line<'a>(
    palette: Palette,
    attempt: &'a restore::Attempt,
    home: &Path,
    done: bool,
) -> Element<'a, Message> {
    let back = contract(&attempt.trashed.original, home);
    let (detail, refused) = match &attempt.fate {
        Fate::PutBack if done => (format!("back at {back}"), false),
        Fate::PutBack => (format!("back to {back}"), false),
        Fate::Refused(why) if done => {
            (format!("stayed in the trash: {}", why.describe(home)), true)
        }
        Fate::Refused(why) => (format!("will not: {}", why.describe(home)), true),
    };
    // Converted to elements here. Each style function returns its own
    // opaque type, so the two branches cannot share a `text` value.
    let detail: Element<'a, Message> = if refused {
        text(detail)
            .size(ty::CAPTION)
            .style(style::tinted(palette.red))
            .wrapping(Wrapping::WordOrGlyph)
            .width(Length::Fill)
            .into()
    } else {
        text(detail)
            .size(ty::CAPTION)
            .style(style::secondary(palette))
            .wrapping(Wrapping::WordOrGlyph)
            .width(Length::Fill)
            .into()
    };
    column![
        text(attempt.name.as_str())
            .size(ty::BODY_SMALL)
            .style(style::body(palette))
            .wrapping(Wrapping::WordOrGlyph)
            .width(Length::Fill),
        detail,
    ]
    .spacing(2)
    .width(Length::Fill)
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
