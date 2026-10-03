//! Where the space went.

use iced::widget::text::Wrapping;
use iced::widget::{Space, button, checkbox, column, container, row, text};
use iced::{Alignment, Element, Length};

use limpid_core::analyse::{Breakdown, Entry};
use limpid_core::config::Exclusions;
use limpid_core::execute::Outcome;
use limpid_core::plan::Disposal;
use limpid_core::size::human;
use limpid_core::walk::Usage;
use limpid_theme::Palette;

use crate::app::{Message, Removal, State, Storage, placeholder};
use crate::layout::Metrics;
use crate::style;
use crate::typography as ty;
use crate::view::{self, Choice, action, chip, tick_spacer};
use crate::widget::treemap::Treemap;

/// Draw the storage page.
pub fn view<'a>(palette: Palette, metrics: Metrics, state: &'a State) -> Element<'a, Message> {
    let storage = state.storage();
    let exclusions = &state.config().config.exclusions;
    if storage.removing {
        return placeholder(palette, "Removing\u{2026}");
    }

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

    if let Some(excluded) = &storage.excluded {
        body = body.push(view::excluded(palette, metrics, excluded));
    }
    if let Some(why) = state.config_error() {
        body = body.push(view::failed(palette, metrics, why));
    }

    if let Some((disposal, outcome)) = &storage.outcome {
        body = body.push(result(palette, metrics, *disposal, outcome));
    }

    let selecting = !storage.plan(Disposal::Trash).is_empty();
    if storage.confirming_empty {
        body = body.push(empty_confirmation(palette, metrics, state));
    } else if !survey.trash.is_empty() && !selecting && !storage.confirming_delete {
        // Not beside a selection: one strip offering actions at a time. It is
        // what is on screen right after something is moved to the trash,
        // because trashing clears the selection.
        body = body.push(trash_bar(palette, metrics, &survey.trash));
    }

    if storage.confirming_delete {
        body = body.push(confirmation(palette, metrics, state));
    } else if selecting {
        // The plan rather than the ticks: a ticked file that has gone from
        // the disk since leaves a tick and nothing to act on, and a bar
        // saying "0 selected" offers buttons that do nothing.
        body = body.push(action_bar(
            palette,
            metrics,
            storage,
            state.config().is_writable(),
        ));
    }

    body = body.push(map(palette, metrics, &survey.breakdown));
    body = body.push(children(
        palette,
        metrics,
        storage,
        exclusions,
        &survey.breakdown,
    ));

    if !survey.largest.is_empty() {
        body = body.push(largest(
            palette,
            metrics,
            storage,
            exclusions,
            &survey.largest,
        ));
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

/// The strip that appears once something is ticked.
///
/// Only when there is a selection: an empty action bar on every visit is a
/// permanent reminder of a thing you are not doing.
fn action_bar<'a>(
    palette: Palette,
    metrics: Metrics,
    storage: &Storage,
    writable: bool,
) -> Element<'a, Message> {
    let plan = storage.plan(Disposal::Trash);
    let folders = storage.has_folders();
    let summary = format!(
        "{} selected, {}{}",
        plan.items.len(),
        human(plan.expected().on_disk),
        // Said where the choice is made, rather than discovered as a
        // greyed-out button with no reason given.
        if folders {
            ". Folders only go to the trash for now"
        } else {
            ""
        },
    );

    let mut quiet: Vec<Choice<'a>> = vec![("Clear", Some(Message::ClearSelection))];

    // One file is something to look at as well as to act on. These two are
    // also on each row of the largest-files list, but only where there is
    // room for them there; here they are reachable at any width.
    if let [only] = plan.items.as_slice()
        && let Some(path) = only.paths.first()
    {
        quiet.push(("Show in folder", Some(Message::Reveal(path.clone()))));
        quiet.push(("Copy path", Some(Message::CopyPath(path.clone()))));
    }

    quiet.push(("Exclude", writable.then_some(Message::ExcludeFiles)));
    quiet.push(("Delete", (!folders).then_some(Message::AskToDelete)));

    // Trash is the primary action because it is the reversible one, and it
    // acts directly for the same reason. A confirmation whose consequence is
    // "you can undo this from your file manager" only teaches people to
    // dismiss confirmations.
    view::selection_bar(
        palette,
        metrics,
        summary,
        quiet,
        ("Move to trash", Some(Message::TrashSelected)),
    )
}

/// The confirmation for removing outright rather than trashing.
fn confirmation<'a>(palette: Palette, metrics: Metrics, state: &State) -> Element<'a, Message> {
    let storage = state.storage();
    let plan = storage.plan(Disposal::Delete);

    let mut lines = column![].spacing(6).width(Length::Fill);
    for item in &plan.items {
        // Where each one is as well as what it is called: two files called
        // `backup.tar` are two different questions.
        let folder = item
            .paths
            .first()
            .and_then(|path| path.parent())
            .map(|parent| state.config().display(parent))
            .unwrap_or_default();
        lines = lines.push(
            column![
                row![
                    text(item.name.clone())
                        .size(ty::BODY_SMALL)
                        .style(style::body(palette))
                        .wrapping(Wrapping::WordOrGlyph)
                        .width(Length::Fill),
                    text(human(item.expected.on_disk))
                        .size(ty::BODY_SMALL)
                        .style(style::secondary(palette)),
                ]
                .spacing(ty::GAP_TIGHT)
                .width(Length::Fill),
                text(folder)
                    .size(ty::CAPTION)
                    .style(style::secondary(palette))
                    .wrapping(Wrapping::WordOrGlyph)
                    .width(Length::Fill),
            ]
            .spacing(2)
            .width(Length::Fill),
        );
    }

    let stacked = !metrics.buttons_inline();
    let cancel = action("Cancel", stacked)
        .style(style::quiet_button(palette))
        .padding([10, 18])
        .on_press(Message::CancelDelete);
    let large = storage.magnitude();
    let allowed = large.is_none() || storage.checked_large;
    let remove = action("Delete permanently", stacked)
        .style(style::danger_button(palette))
        .padding([10, 20])
        .on_press_maybe(allowed.then_some(Message::DeleteSelected));

    let actions: Element<'a, Message> = if stacked {
        column![cancel, remove]
            .spacing(ty::GAP_TIGHT)
            .width(Length::Fill)
            .into()
    } else {
        row![Space::new().width(Length::Fill), cancel, remove]
            .spacing(ty::GAP_TIGHT)
            .into()
    };

    let mut body = column![
        text(format!(
            "Delete {} permanently?",
            human(plan.expected().on_disk)
        ))
        .size(ty::TITLE)
        .style(style::heading(palette))
        .width(Length::Fill),
        text(
            "These do not go to the trash and cannot be recovered. Moving them to \
             the trash instead frees the same space once you empty it.",
        )
        .size(ty::BODY_SMALL)
        .style(style::secondary(palette))
        .width(Length::Fill),
        Space::new().height(Length::Fixed(ty::GAP_TIGHT)),
        lines,
    ]
    .spacing(4)
    .width(Length::Fill);

    // Between the list and the way to act on it.
    if let Some(magnitude) = &large {
        body = body.push(Space::new().height(Length::Fixed(ty::GAP_TIGHT)));
        body = body.push(view::large(
            palette,
            metrics,
            magnitude,
            storage.checked_large,
            Message::CheckLargeDelete,
        ));
    }

    container(
        body.push(Space::new().height(Length::Fixed(ty::GAP)))
            .push(actions),
    )
    .style(style::card(palette))
    .padding(metrics.card)
    .width(Length::Fill)
    .into()
}

/// What is sitting in the trash, and the way to actually free it.
///
/// Moving something to the trash keeps it on the same disk, so on its own it
/// frees nothing. Without this, trashing the largest file looked like a button
/// that did nothing: the space stayed where it was, and the file was still
/// there to be found.
fn trash_bar<'a>(palette: Palette, metrics: Metrics, trash: &Usage) -> Element<'a, Message> {
    let summary = format!(
        "{} in the trash, across {} files. It frees nothing until the trash is \
         emptied.",
        human(trash.size.on_disk),
        trash.files,
    );
    view::selection_bar(
        palette,
        metrics,
        summary,
        Vec::new(),
        ("Empty trash", Some(Message::AskToEmptyTrash)),
    )
}

/// The confirmation for emptying the trash.
///
/// Asked, unlike moving something there: this is the point of no return for
/// everything in it, and some of that was put there by other programs and
/// may be something the person still means to restore.
fn empty_confirmation<'a>(
    palette: Palette,
    metrics: Metrics,
    state: &State,
) -> Element<'a, Message> {
    let storage = state.storage();
    let trash = storage
        .survey
        .as_ref()
        .map(|survey| survey.trash.clone())
        .unwrap_or_default();

    let stacked = !metrics.buttons_inline();
    let cancel = action("Cancel", stacked)
        .style(style::quiet_button(palette))
        .padding([10, 18])
        .on_press(Message::CancelEmptyTrash);
    let large = state.trash_magnitude();
    let allowed = large.is_none() || storage.checked_large;
    let empty = action("Empty trash", stacked)
        .style(style::danger_button(palette))
        .padding([10, 20])
        .on_press_maybe(allowed.then_some(Message::EmptyTrash));

    let actions: Element<'a, Message> = if stacked {
        column![cancel, empty]
            .spacing(ty::GAP_TIGHT)
            .width(Length::Fill)
            .into()
    } else {
        row![Space::new().width(Length::Fill), cancel, empty]
            .spacing(ty::GAP_TIGHT)
            .into()
    };

    let mut body = column![
        text(format!("Empty the trash? {}", human(trash.size.on_disk)))
            .size(ty::TITLE)
            .style(style::heading(palette))
            .width(Length::Fill),
        text(format!(
            "All {} files in the trash are deleted permanently, including anything \
             other applications put there. Restore whatever you want to keep from \
             your file manager first.",
            trash.files,
        ))
        .size(ty::BODY_SMALL)
        .style(style::secondary(palette))
        .width(Length::Fill),
    ]
    .spacing(4)
    .width(Length::Fill);

    if let Some(magnitude) = &large {
        body = body.push(Space::new().height(Length::Fixed(ty::GAP_TIGHT)));
        body = body.push(view::large(
            palette,
            metrics,
            magnitude,
            storage.checked_large,
            Message::CheckLargeDelete,
        ));
    }

    container(
        body.push(Space::new().height(Length::Fixed(ty::GAP)))
            .push(actions),
    )
    .style(style::card(palette))
    .padding(metrics.card)
    .width(Length::Fill)
    .into()
}

/// What the last removal from this page did.
fn result<'a>(
    palette: Palette,
    metrics: Metrics,
    removal: Removal,
    outcome: &Outcome,
) -> Element<'a, Message> {
    let line = |good: bool, said: String| {
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
                .width(Length::Fill),
        ]
        .spacing(ty::GAP_TIGHT)
    };

    // The trash is a rename on the same disk, so nothing is freed until it
    // is emptied. Saying "removed" there would be saying the space came back
    // when it has not.
    let said = match removal {
        Removal::Trashed => format!(
            "Moved {} to the trash, across {} files. The space comes back once the \
             trash is emptied.",
            human(outcome.reclaimed.on_disk),
            outcome.files,
        ),
        Removal::Deleted => format!(
            "Removed {} across {} files.",
            human(outcome.reclaimed.on_disk),
            outcome.files
        ),
        Removal::Emptied => format!(
            "Emptied the trash: {} freed, across {} files.",
            human(outcome.reclaimed.on_disk),
            outcome.files
        ),
    };
    let mut body = column![line(true, said)].spacing(6).width(Length::Fill);

    for problem in &outcome.problems {
        body = body.push(line(false, problem.to_string()));
    }

    container(body)
        .style(style::well(palette))
        .padding(metrics.gap)
        .width(Length::Fill)
        .into()
}

/// The two per-file actions, for windows with room to show them.
///
/// Only on the largest-files list. In the list of what is directly inside
/// this directory, "open the containing folder" would open the directory
/// already on screen.
///
/// Hidden below the two-column band. At that width the name and the size
/// are already competing for room, and a 16-pixel target in a 240-pixel
/// column is a poor one even when it fits.
fn file_actions<'a>(
    palette: Palette,
    metrics: Metrics,
    entry: &Entry,
) -> Option<Element<'a, Message>> {
    if !metrics.two_columns() {
        return None;
    }

    // No tooltips. Iced has them, but they need an overlay layer that
    // fights the scrollable these sit inside, and both glyphs are
    // conventional enough to stand alone.
    let small = |glyph: &'static str, message: Message| {
        button(text(glyph).size(ty::BODY_SMALL).center())
            .style(style::quiet_button(palette))
            .padding([4, 8])
            .width(Length::Fixed(30.0))
            .on_press(message)
    };

    Some(
        row![
            small("\u{2197}", Message::Reveal(entry.path.clone())),
            small("\u{29c9}", Message::CopyPath(entry.path.clone())),
        ]
        .spacing(4)
        .into(),
    )
}

/// A checkbox for a file or a folder, or the space one would take.
///
/// Folders can be ticked, and go to the trash whole; deleting one outright
/// waits for a history of what was removed. Anything excluded gets the
/// space instead — the executor would refuse it, and a tick that can only
/// produce a refusal is a question with one answer.
fn tick<'a>(
    palette: Palette,
    storage: &Storage,
    exclusions: &Exclusions,
    entry: &Entry,
) -> Element<'a, Message> {
    if exclusions.covers(&entry.path) {
        return tick_spacer();
    }

    let path = entry.path.clone();
    checkbox(storage.is_selected(&path))
        .size(16)
        .style(style::tick(palette))
        .on_toggle(move |_| Message::ToggleFile(path.clone()))
        .into()
}

/// A file or directory name, marked when it is excluded.
///
/// Still listed, and still counted: excluding something stops Limpid
/// offering it, not accounting for it. A list that silently dropped it
/// would leave the treemap above showing space nothing below explains.
fn named<'a>(palette: Palette, name: String, excluded: bool) -> Element<'a, Message> {
    let name = text(name)
        .size(ty::BODY_SMALL)
        .style(style::body(palette))
        .wrapping(Wrapping::WordOrGlyph);

    if !excluded {
        return name.width(Length::Fill).into();
    }

    // The name at its own width and the chip after it, on a line that
    // wraps: beside the name when both fit, under it when they do not.
    // With the name set to Fill instead, the chip would keep its width and
    // squeeze a long name into a column three glyphs wide.
    row![name, chip(palette, "excluded", palette.dark_foreground)]
        .spacing(ty::GAP_TIGHT)
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .wrap()
        .vertical_spacing(4)
        .into()
}

/// The same level as a list, which the treemap cannot show for small items.
fn children<'a>(
    palette: Palette,
    metrics: Metrics,
    storage: &Storage,
    exclusions: &Exclusions,
    breakdown: &Breakdown,
) -> Element<'a, Message> {
    let total = breakdown.total().on_disk;
    let mut rows = column![].spacing(2).width(Length::Fill);

    for (index, child) in breakdown.children.iter().take(12).enumerate() {
        let name = named(
            palette,
            format!("{}{}", child.name, if child.is_dir { "/" } else { "" }),
            exclusions.covers(&child.path),
        );
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
                name,
                share.width(Length::Fixed(44.0)).align_x(Alignment::End),
                size.width(Length::Fixed(76.0)).align_x(Alignment::End),
            ]
            .spacing(ty::GAP_TIGHT)
            .align_y(Alignment::Center)
            .into()
        } else {
            row![name, size]
                .spacing(ty::GAP_TIGHT)
                .align_y(Alignment::Center)
                .into()
        };

        // The tick sits outside the button: inside it, the button would
        // swallow the click and descend instead of selecting.
        let body: Element<'a, Message> = if child.is_dir {
            button(line)
                .style(style::nav_button(palette, false))
                .padding([7, 10])
                .width(Length::Fill)
                .on_press(Message::Descend(index))
                .into()
        } else {
            container(line).padding([7, 10]).width(Length::Fill).into()
        };

        rows = rows.push(
            row![tick(palette, storage, exclusions, child), body]
                .spacing(ty::GAP_TIGHT)
                .align_y(Alignment::Center)
                .width(Length::Fill),
        );
    }

    container(rows)
        .style(style::card(palette))
        .padding(metrics.gap)
        .width(Length::Fill)
        .into()
}

/// The largest individual files anywhere below here.
fn largest<'a>(
    palette: Palette,
    metrics: Metrics,
    storage: &Storage,
    exclusions: &Exclusions,
    entries: &[Entry],
) -> Element<'a, Message> {
    let mut rows = column![].spacing(6).width(Length::Fill);

    for entry in entries {
        // The parent directory, not the whole path: at 300 px a full path
        // wraps to four lines and says less than the last component of it.
        let where_ = entry
            .path
            .parent()
            .map(|parent| parent.display().to_string())
            .unwrap_or_default();

        // A file name is one long token as often as not, and the default
        // word-level wrap cannot break it — so it runs past its column and
        // draws straight over the size beside it. Only visible in a narrow
        // window, which is exactly where it matters.
        let name = column![
            named(palette, entry.name.clone(), exclusions.covers(&entry.path)),
            text(where_)
                .size(ty::CAPTION)
                .style(style::secondary(palette))
                .wrapping(Wrapping::WordOrGlyph)
                .width(Length::Fill),
        ]
        .spacing(2)
        .width(Length::Fill);

        let size = text(human(entry.size.on_disk))
            .size(ty::BODY_SMALL)
            .style(style::body(palette));

        // Fill, because `row!` is Shrink by default and a Shrink row
        // resolves its Fill child to the child's natural width — so the
        // name never wraps and draws straight over the size.
        let mut line = row![tick(palette, storage, exclusions, entry), name, size]
            .spacing(ty::GAP_TIGHT)
            .align_y(Alignment::Center)
            .width(Length::Fill);

        if let Some(actions) = file_actions(palette, metrics, entry) {
            line = line.push(actions);
        }

        rows = rows.push(line);
    }

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
