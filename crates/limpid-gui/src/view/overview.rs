//! What was found, and how much of it there is.
//!
//! One number dominates, because there is only one question. Everything else
//! on the page explains where that number came from.

use iced::widget::{Space, button, checkbox, column, container, responsive, row, text};
use iced::{Alignment, Element, Length};

use limpid_core::execute::Outcome;
use limpid_core::model::{Category, Risk, Scan, Target};
use limpid_core::size::human;
use limpid_theme::{Color, Palette};

use crate::app::{Message, Progress, State, TargetId, placeholder};
use crate::style;
use crate::typography as ty;
use crate::widget::gauge::{Bar, Gauge};

/// Diameter of the gauge.
const GAUGE_SIZE: f32 = 208.0;

/// Hues cycled through the category cards, so that two groups next to each
/// other are told apart by more than their position. Deliberately excludes
/// red and yellow, which are reserved for risk.
fn hue(palette: Palette, index: usize) -> Color {
    let series = [
        palette.accent,
        palette.cyan,
        palette.magenta,
        palette.green,
        palette.blue,
    ];
    series[index % series.len()]
}

/// Draw the overview.
pub fn view<'a>(palette: Palette, state: &'a State) -> Element<'a, Message> {
    if state.is_cleaning() {
        return placeholder(palette, "Cleaning\u{2026}");
    }
    match state.progress() {
        Progress::Idle => placeholder(palette, "Ready to look."),
        Progress::Running => placeholder(palette, "Looking through your disk\u{2026}"),
        Progress::Done(scan) => found(palette, state, scan),
    }
}

/// The page once there is something to show.
fn found<'a>(palette: Palette, state: &'a State, scan: &'a Scan) -> Element<'a, Message> {
    let mut body = column![hero(palette, scan)].spacing(ty::GAP);

    if let Some(outcome) = state.outcome() {
        body = body.push(result(palette, outcome));
    }

    if state.is_confirming() {
        body = body.push(confirmation(palette, state));
    } else {
        body = body.push(action_bar(palette, state));
    }

    let largest = scan
        .categories
        .first()
        .map_or(0, |category| category.size().on_disk);
    for (index, category) in scan.categories.iter().enumerate() {
        body = body.push(category_card(
            palette,
            state,
            index,
            category,
            largest,
            hue(palette, index),
        ));
    }

    for caveat in &scan.caveats {
        body = body.push(note(palette, caveat));
    }

    body.into()
}

/// The card that carries the answer.
///
/// The ring shows what is ready to go as a share of what was found, and the
/// number inside it is that same figure — a ring whose fill and whose label
/// describe different quantities is worse than no ring.
fn hero<'a>(palette: Palette, scan: &'a Scan) -> Element<'a, Message> {
    // Below this width the ring and the facts cannot sit side by side
    // without the text wrapping to one word a line, so they stack instead.
    const SIDE_BY_SIDE: f32 = 520.0;

    let layout = responsive(move |size| {
        let ring = gauge(palette, scan);
        if size.width < SIDE_BY_SIDE {
            column![container(ring).center_x(Length::Fill), facts(palette, scan)]
                .spacing(ty::GAP_WIDE)
                .into()
        } else {
            row![ring, facts(palette, scan)]
                .spacing(ty::GAP_WIDE + ty::GAP)
                .align_y(Alignment::Center)
                .into()
        }
    });

    container(layout)
        .style(style::card(palette))
        .padding(ty::GAP_WIDE)
        .width(Length::Fill)
        .into()
}

/// The ring, filled with the share of the find that is ready to go.
fn gauge<'a>(palette: Palette, scan: &Scan) -> Element<'a, Message> {
    let found = scan.size().on_disk;
    let ready = scan.reclaimable_unprivileged().on_disk;
    let fraction = if found == 0 {
        0.0
    } else {
        ready as f32 / found as f32
    };

    Gauge::new(palette, fraction, human(ready), "ready").view(GAUGE_SIZE)
}

/// The numbers beside the ring.
fn facts<'a>(palette: Palette, scan: &'a Scan) -> Element<'a, Message> {
    let found = scan.size().on_disk;
    let ready = scan.reclaimable_unprivileged().on_disk;
    let elevated = found.saturating_sub(ready);

    let mut facts = column![
        text("Ready to reclaim")
            .size(ty::BODY_SMALL)
            .style(style::secondary(palette)),
        text(human(ready))
            .size(ty::DISPLAY)
            .style(style::heading(palette)),
        Space::new().height(Length::Fixed(ty::GAP)),
        stat(palette, "Found in total", human(found)),
        stat(palette, "Behind elevation", human(elevated)),
        stat(palette, "Groups", scan.categories.len().to_string()),
    ]
    .spacing(3)
    .width(Length::Fill);

    if let Some(capacity) = scan.capacity {
        facts = facts.push(Space::new().height(Length::Fixed(ty::GAP)));
        facts = facts.push(
            column![
                Bar::new(palette, capacity.fraction_used(), palette.muted).view(6.0),
                text(format!(
                    "{} of {} used on this disk",
                    human(capacity.used()),
                    human(capacity.total),
                ))
                .size(ty::CAPTION)
                .style(style::secondary(palette)),
            ]
            .spacing(6),
        );
    }

    facts
        .push(Space::new().height(Length::Fixed(ty::GAP_WIDE)))
        .push(
            button(text("Scan again").size(ty::BODY))
                .style(style::primary_button(palette))
                .padding([10, 20])
                .on_press(Message::StartScan),
        )
        .into()
}

/// A label on the left, a value on the right.
fn stat<'a>(palette: Palette, label: &'a str, value: String) -> Element<'a, Message> {
    row![
        text(label)
            .size(ty::BODY_SMALL)
            .style(style::secondary(palette))
            .width(Length::Fill),
        text(value).size(ty::BODY_SMALL).style(style::body(palette)),
    ]
    .into()
}

/// The strip that says what is selected and offers to act on it.
fn action_bar<'a>(palette: Palette, state: &'a State) -> Element<'a, Message> {
    let plan = state.plan();
    let count = plan.items.len();
    let summary = if count == 0 {
        "Nothing selected".to_owned()
    } else {
        format!(
            "{count} selected, {} to reclaim",
            human(plan.expected().on_disk)
        )
    };

    let mut clean = button(text("Clean").size(ty::BODY))
        .style(style::primary_button(palette))
        .padding([10, 22]);
    if count > 0 {
        clean = clean.on_press(Message::AskToClean);
    }

    container(
        row![
            text(summary)
                .size(ty::BODY)
                .style(style::body(palette))
                .width(Length::Fill),
            button(text("Safe only").size(ty::BODY_SMALL))
                .style(style::quiet_button(palette))
                .padding([8, 14])
                .on_press(Message::SelectSafe),
            button(text("None").size(ty::BODY_SMALL))
                .style(style::quiet_button(palette))
                .padding([8, 14])
                .on_press(Message::SelectNone),
            clean,
        ]
        .spacing(ty::GAP_TIGHT)
        .align_y(Alignment::Center),
    )
    .style(style::card(palette))
    .padding(ty::GAP)
    .width(Length::Fill)
    .into()
}

/// The last chance to say no.
///
/// A destructive action should not be one click away from the screen you
/// land on, and the wording has to be honest about whether it can be undone.
fn confirmation<'a>(palette: Palette, state: &'a State) -> Element<'a, Message> {
    let plan = state.plan();

    let mut lines = column![].spacing(4);
    for item in &plan.items {
        lines = lines.push(row![
            text(item.name.clone())
                .size(ty::BODY_SMALL)
                .style(style::body(palette))
                .width(Length::Fill),
            text(human(item.expected.on_disk))
                .size(ty::BODY_SMALL)
                .style(style::secondary(palette)),
        ]);
    }

    let warning = if plan.has_permanent_deletions() {
        "This cannot be undone. Caches are removed outright rather than sent to the \
         trash, because moving them there would not free any space."
    } else {
        "Everything here goes to the trash and can be put back."
    };

    container(
        column![
            text(format!("Remove {}?", human(plan.expected().on_disk)))
                .size(ty::TITLE)
                .style(style::heading(palette)),
            text(warning)
                .size(ty::BODY_SMALL)
                .style(style::secondary(palette)),
            Space::new().height(Length::Fixed(ty::GAP_TIGHT)),
            lines,
            Space::new().height(Length::Fixed(ty::GAP)),
            row![
                Space::new().width(Length::Fill),
                button(text("Cancel").size(ty::BODY))
                    .style(style::quiet_button(palette))
                    .padding([10, 18])
                    .on_press(Message::Cancel),
                button(text("Remove").size(ty::BODY))
                    .style(style::danger_button(palette))
                    .padding([10, 22])
                    .on_press(Message::Clean),
            ]
            .spacing(ty::GAP_TIGHT),
        ]
        .spacing(4),
    )
    .style(style::card(palette))
    .padding(ty::GAP_WIDE)
    .width(Length::Fill)
    .into()
}

/// What the last clean did.
fn result<'a>(palette: Palette, outcome: &'a Outcome) -> Element<'a, Message> {
    let headline = format!(
        "Reclaimed {} across {} files.",
        human(outcome.reclaimed.on_disk),
        outcome.files
    );

    let mut body = column![
        row![
            text("\u{2713}")
                .size(ty::BODY)
                .style(style::tinted(palette.green)),
            text(headline).size(ty::BODY).style(style::body(palette)),
        ]
        .spacing(ty::GAP_TIGHT),
    ]
    .spacing(6);

    for problem in &outcome.problems {
        body = body.push(
            row![
                text("\u{2717}")
                    .size(ty::BODY_SMALL)
                    .style(style::tinted(palette.red)),
                text(problem.to_string())
                    .size(ty::CAPTION)
                    .style(style::secondary(palette)),
            ]
            .spacing(ty::GAP_TIGHT),
        );
    }

    container(body)
        .style(style::well(palette))
        .padding(ty::GAP)
        .width(Length::Fill)
        .into()
}

/// One group of findings.
fn category_card<'a>(
    palette: Palette,
    state: &'a State,
    index: usize,
    category: &'a Category,
    largest: u64,
    hue: Color,
) -> Element<'a, Message> {
    let size = category.size().on_disk;
    let fraction = if largest == 0 {
        0.0
    } else {
        size as f32 / largest as f32
    };

    let header = row![
        container(text("\u{25cf}").size(ty::CAPTION).style(style::tinted(hue)))
            .padding(iced::Padding::default().top(4)),
        column![
            text(category.name.as_str())
                .size(ty::TITLE)
                .style(style::heading(palette)),
            text(category.detail.as_str())
                .size(ty::BODY_SMALL)
                .style(style::secondary(palette)),
        ]
        .spacing(2)
        .width(Length::Fill),
        text(human(size))
            .size(ty::SUBTITLE)
            .style(style::body(palette)),
    ]
    .spacing(ty::GAP_TIGHT)
    .align_y(Alignment::Start);

    let mut rows = column![].spacing(2);
    for (position, target) in category.targets.iter().enumerate() {
        rows = rows.push(target_row(palette, state, (index, position), target));
    }

    container(
        column![
            header,
            Bar::new(palette, fraction, hue).view(5.0),
            Space::new().height(Length::Fixed(4.0)),
            rows,
        ]
        .spacing(ty::GAP_TIGHT),
    )
    .style(style::card(palette))
    .padding(ty::GAP_WIDE)
    .width(Length::Fill)
    .into()
}

/// One finding.
fn target_row<'a>(
    palette: Palette,
    state: &'a State,
    id: TargetId,
    target: &'a Target,
) -> Element<'a, Message> {
    let size = if target.size.is_zero() {
        text("\u{2014}")
            .size(ty::BODY_SMALL)
            .style(style::secondary(palette))
    } else {
        text(human(target.size.on_disk))
            .size(ty::BODY_SMALL)
            .style(style::body(palette))
    };

    let mut labels = row![
        text(target.name.as_str())
            .size(ty::BODY)
            .style(style::body(palette))
    ]
    .spacing(ty::GAP_TIGHT)
    .align_y(Alignment::Center);

    if target.requires_root {
        labels = labels.push(chip(palette, "needs root", palette.dark_foreground));
    }
    if target.blocked.is_some() {
        labels = labels.push(chip(palette, "in use", palette.orange));
    }
    if target.risk != Risk::Safe {
        labels = labels.push(chip(
            palette,
            target.risk.label(),
            risk_colour(palette, target.risk),
        ));
    }

    // Nothing to tick for an item that is only a note, or one this process
    // could not act on even if asked.
    let selectable = target.is_actionable() && !target.size.is_zero();

    let tick: Element<'a, Message> = if selectable {
        checkbox(state.is_selected(id))
            .size(16)
            .style(style::tick(palette))
            .on_toggle(move |_| Message::Toggle(id))
            .into()
    } else {
        Space::new().width(Length::Fixed(22.0)).into()
    };

    container(
        row![
            tick,
            column![
                labels,
                // The reason it cannot be touched displaces the description:
                // "close Brave first" is the only thing worth reading here.
                match &target.blocked {
                    Some(reason) => text(reason.as_str())
                        .size(ty::CAPTION)
                        .style(style::tinted(palette.orange)),
                    None => text(target.detail.as_str())
                        .size(ty::CAPTION)
                        .style(style::secondary(palette)),
                },
            ]
            .spacing(3)
            .width(Length::Fill),
            container(size).align_right(Length::Fixed(88.0)),
        ]
        .align_y(Alignment::Center)
        .spacing(ty::GAP),
    )
    .style(style::well(palette))
    .padding([10, 14])
    .width(Length::Fill)
    .into()
}

/// A small coloured label.
fn chip<'a>(palette: Palette, label: &'a str, tint: Color) -> Element<'a, Message> {
    container(text(label).size(ty::CAPTION))
        .style(style::badge(palette, tint))
        .padding([1, 7])
        .into()
}

/// The colour that carries a risk level. Green is deliberately not used: a
/// safe item wears no chip at all, so green would only ever mean "look here"
/// about something that needs no looking.
fn risk_colour(palette: Palette, risk: Risk) -> Color {
    match risk {
        Risk::Safe => palette.green,
        Risk::Review => palette.yellow,
        Risk::Sensitive => palette.red,
    }
}

/// A caveat about the numbers above.
fn note<'a>(palette: Palette, body: &'a str) -> Element<'a, Message> {
    container(
        row![
            text("\u{24d8}")
                .size(ty::BODY)
                .style(style::tinted(palette.cyan)),
            text(body)
                .size(ty::BODY_SMALL)
                .style(style::secondary(palette)),
        ]
        .spacing(ty::GAP_TIGHT),
    )
    .style(style::well(palette))
    .padding(ty::GAP)
    .width(Length::Fill)
    .into()
}
