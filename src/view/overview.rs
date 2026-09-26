//! What was found, and how much of it there is.
//!
//! One number dominates, because there is only one question. Everything else
//! on the page explains where that number came from — and gets out of the
//! way first when the window is small.

use iced::widget::{Space, button, checkbox, column, container, row, text};
use iced::{Alignment, Element, Length};

use limpid_core::model::{Category, Kind, Risk, Scan, Target};
use limpid_core::size::human;
use limpid_theme::{Color, Palette};

use crate::app::{Cleaned, Message, Progress, State, TargetId, placeholder};
use crate::layout::Metrics;
use crate::style;
use crate::typography as ty;
use crate::widget::gauge::{Bar, Gauge};

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
pub fn view<'a>(palette: Palette, metrics: Metrics, state: &'a State) -> Element<'a, Message> {
    if state.is_cleaning() {
        return placeholder(palette, "Cleaning\u{2026}");
    }
    match state.progress() {
        Progress::Idle => placeholder(palette, "Ready to look."),
        Progress::Running => placeholder(palette, "Looking through your disk\u{2026}"),
        Progress::Done(scan) => found(palette, metrics, state, scan),
    }
}

/// The page once there is something to show.
fn found<'a>(
    palette: Palette,
    metrics: Metrics,
    state: &'a State,
    scan: &'a Scan,
) -> Element<'a, Message> {
    let mut body = column![hero(palette, metrics, scan)]
        .spacing(metrics.gap)
        .width(Length::Fill);

    if let Some(cleaned) = state.outcome() {
        body = body.push(result(palette, metrics, cleaned));
    }

    if state.is_confirming() {
        body = body.push(confirmation(palette, metrics, state));
    } else {
        body = body.push(action_bar(palette, metrics, state));
    }

    let largest = scan
        .categories
        .first()
        .map_or(0, |category| category.size().on_disk);
    for (index, category) in scan.categories.iter().enumerate() {
        body = body.push(category_card(
            palette,
            metrics,
            state,
            index,
            category,
            largest,
            hue(palette, index),
        ));
    }

    for caveat in &scan.caveats {
        body = body.push(note(palette, metrics, caveat));
    }

    body.into()
}

/// The card that carries the answer.
///
/// The ring shows what is ready to go as a share of what was found, and the
/// number inside it is that same figure — a ring whose fill and whose label
/// describe different quantities is worse than no ring.
fn hero<'a>(palette: Palette, metrics: Metrics, scan: &'a Scan) -> Element<'a, Message> {
    let ring = gauge(palette, metrics, scan);
    let figures = facts(palette, metrics, scan);

    let inner: Element<'a, Message> = if metrics.hero_side_by_side() {
        row![ring, figures]
            .spacing(metrics.card)
            .align_y(Alignment::Center)
            .into()
    } else {
        column![container(ring).center_x(Length::Fill), figures]
            .spacing(metrics.gap)
            .into()
    };

    container(inner)
        .style(style::card(palette))
        .padding(metrics.card)
        .width(Length::Fill)
        .into()
}

/// The ring, filled with the share of the find that is ready to go.
fn gauge<'a>(palette: Palette, metrics: Metrics, scan: &Scan) -> Element<'a, Message> {
    let found = scan.size().on_disk;
    let ready = scan.reclaimable_unprivileged().on_disk;
    let fraction = if found == 0 {
        0.0
    } else {
        ready as f32 / found as f32
    };

    Gauge::new(palette, fraction, human(ready), "ready").view(metrics.gauge)
}

/// The numbers beside the ring.
fn facts<'a>(palette: Palette, metrics: Metrics, scan: &'a Scan) -> Element<'a, Message> {
    let found = scan.size().on_disk;
    let ready = scan.reclaimable_unprivileged().on_disk;
    let elevated = found.saturating_sub(ready);

    let mut facts = column![
        text("Ready to reclaim")
            .size(ty::BODY_SMALL)
            .style(style::secondary(palette)),
        text(human(ready))
            .size(metrics.display())
            .style(style::heading(palette)),
        Space::new().height(Length::Fixed(ty::GAP)),
        stat(palette, metrics, "Found in total", human(found)),
        stat(palette, metrics, "Behind elevation", human(elevated)),
        stat(
            palette,
            metrics,
            "Groups",
            scan.categories.len().to_string()
        ),
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
        .push(Space::new().height(Length::Fixed(metrics.gap)))
        .push(
            action("Scan again", !metrics.two_columns())
                .style(style::primary_button(palette))
                .padding([10, 20])
                .on_press(Message::StartScan),
        )
        .into()
}

/// A label on the left, a value on the right — or stacked when a line cannot
/// hold both without the label wrapping to one word.
fn stat<'a>(
    palette: Palette,
    metrics: Metrics,
    label: &'a str,
    value: String,
) -> Element<'a, Message> {
    let name = text(label)
        .size(ty::BODY_SMALL)
        .style(style::secondary(palette));
    let figure = text(value).size(ty::BODY_SMALL).style(style::body(palette));

    if metrics.two_columns() {
        row![name.width(Length::Fill), figure].into()
    } else {
        row![name, Space::new().width(Length::Fill), figure].into()
    }
}

/// The strip that says what is selected and offers to act on it.
fn action_bar<'a>(palette: Palette, metrics: Metrics, state: &'a State) -> Element<'a, Message> {
    let plan = state.plan();
    let count = plan.items.len() + plan.operations.len();
    let summary = if count == 0 {
        "Nothing selected".to_owned()
    } else {
        format!(
            "{count} selected, {} to reclaim",
            human(plan.expected().on_disk)
        )
    };

    let stacked = !metrics.two_columns() && !metrics.buttons_inline();
    let mut clean = action("Clean", stacked)
        .style(style::primary_button(palette))
        .padding([10, 22]);
    if count > 0 {
        clean = clean.on_press(Message::AskToClean);
    }

    let picks = row![
        button(text("Safe only").size(ty::BODY_SMALL))
            .style(style::quiet_button(palette))
            .padding([8, 14])
            .on_press(Message::SelectSafe),
        button(text("None").size(ty::BODY_SMALL))
            .style(style::quiet_button(palette))
            .padding([8, 14])
            .on_press(Message::SelectNone),
    ]
    .spacing(ty::GAP_TIGHT)
    .align_y(Alignment::Center);

    let label = text(summary).size(ty::BODY).style(style::body(palette));

    // Three tiers, because a clipped button is not a smaller button — it is
    // one the user cannot tell is there.
    let inner: Element<'a, Message> = if metrics.two_columns() {
        row![
            label.width(Length::Fill),
            picks,
            Space::new().width(Length::Fixed(ty::GAP_TIGHT)),
            clean,
        ]
        .spacing(ty::GAP_TIGHT)
        .align_y(Alignment::Center)
        .into()
    } else if metrics.buttons_inline() {
        column![
            label,
            row![picks, Space::new().width(Length::Fill), clean]
                .spacing(ty::GAP_TIGHT)
                .align_y(Alignment::Center),
        ]
        .spacing(ty::GAP_TIGHT)
        .into()
    } else {
        // The primary action takes the whole width rather than the sliver
        // left over by the two beside it.
        column![label, picks, clean]
            .spacing(ty::GAP_TIGHT)
            .width(Length::Fill)
            .into()
    };

    container(inner)
        .style(style::card(palette))
        .padding(metrics.gap)
        .width(Length::Fill)
        .into()
}

/// The last chance to say no.
///
/// A destructive action should not be one click away from the screen you
/// land on, and the wording has to be honest about whether it can be undone.
fn confirmation<'a>(palette: Palette, metrics: Metrics, state: &'a State) -> Element<'a, Message> {
    let plan = state.plan();

    let mut lines = column![].spacing(4).width(Length::Fill);
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
    for operation in &plan.operations {
        lines = lines.push(row![
            text(operation.describe())
                .size(ty::BODY_SMALL)
                .style(style::body(palette))
                .width(Length::Fill),
            text("needs root")
                .size(ty::CAPTION)
                .style(style::secondary(palette)),
        ]);
    }

    let warning: &str = if plan.has_permanent_deletions() {
        "This cannot be undone. Caches are removed outright rather than sent to the \
         trash, because moving them there would not free any space."
    } else {
        "Everything here goes to the trash and can be put back."
    };

    let stacked = !metrics.buttons_inline();
    let cancel = action("Cancel", stacked)
        .style(style::quiet_button(palette))
        .padding([10, 18])
        .on_press(Message::Cancel);
    let remove = action("Remove", stacked)
        .style(style::danger_button(palette))
        .padding([10, 22])
        .on_press(Message::Clean);

    // Stacked when narrow, and Cancel stays first either way: the
    // destructive one should never be where the safe one was a moment ago.
    let actions: Element<'a, Message> = if metrics.buttons_inline() {
        row![Space::new().width(Length::Fill), cancel, remove]
            .spacing(ty::GAP_TIGHT)
            .into()
    } else {
        column![cancel, remove]
            .spacing(ty::GAP_TIGHT)
            .width(Length::Fill)
            .into()
    };

    let mut body = column![
        text(format!("Remove {}?", human(plan.expected().on_disk)))
            .size(ty::TITLE)
            .style(style::heading(palette))
            .width(Length::Fill),
        text(warning)
            .size(ty::BODY_SMALL)
            .style(style::secondary(palette))
            .width(Length::Fill),
        Space::new().height(Length::Fixed(ty::GAP_TIGHT)),
        lines,
    ]
    .spacing(4)
    .width(Length::Fill);

    if plan.needs_elevation() {
        body = body.push(Space::new().height(Length::Fixed(ty::GAP_TIGHT)));
        body = body.push(
            text(
                "Some of this needs root. Limpid will ask for your password, and the \
                 work is done by a small separate program that only accepts a fixed \
                 list of operations — it is never given a path.",
            )
            .size(ty::CAPTION)
            .style(style::tinted(palette.orange))
            .width(Length::Fill),
        );
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

/// What the last clean did, both halves.
fn result<'a>(palette: Palette, metrics: Metrics, cleaned: &'a Cleaned) -> Element<'a, Message> {
    let outcome = &cleaned.outcome;

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

    let mut body = column![line(
        true,
        format!(
            "Reclaimed {} across {} files.",
            human(outcome.reclaimed.on_disk),
            outcome.files,
        ),
    )]
    .spacing(6);

    for problem in &outcome.problems {
        body = body.push(line(false, problem.to_string()));
    }

    match &cleaned.elevated {
        Some(Ok(report)) => {
            for done in &report.completed {
                body = body.push(line(
                    done.succeeded,
                    format!("{}: {}", done.operation.describe(), done.detail),
                ));
            }
        }
        Some(Err(error)) => {
            body = body.push(line(
                false,
                format!("The parts needing root were not done — {error}"),
            ));
        }
        None => {}
    }

    container(body)
        .style(style::well(palette))
        .padding(metrics.gap)
        .width(Length::Fill)
        .into()
}

/// One group of findings.
fn category_card<'a>(
    palette: Palette,
    metrics: Metrics,
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

    let titles = column![
        text(category.name.as_str())
            .size(ty::TITLE)
            .style(style::heading(palette)),
        text(category.detail.as_str())
            .size(ty::BODY_SMALL)
            .style(style::secondary(palette))
            .width(Length::Fill),
    ]
    .spacing(2)
    .width(Length::Fill);

    let total = text(human(size))
        .size(ty::SUBTITLE)
        .style(style::body(palette));
    let dot = container(text("\u{25cf}").size(ty::CAPTION).style(style::tinted(hue)))
        .padding(iced::Padding::default().top(4));

    // The total moves under the name rather than being pushed off the edge
    // by it.
    let header: Element<'a, Message> = if metrics.two_columns() {
        row![dot, titles, total]
            .spacing(ty::GAP_TIGHT)
            .align_y(Alignment::Start)
            .into()
    } else {
        column![
            row![dot, titles]
                .spacing(ty::GAP_TIGHT)
                .align_y(Alignment::Start),
            total
        ]
        .spacing(4)
        .into()
    };

    let mut rows = column![].spacing(2).width(Length::Fill);
    for (position, target) in category.targets.iter().enumerate() {
        rows = rows.push(target_row(
            palette,
            metrics,
            state,
            (index, position),
            target,
        ));
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
    .padding(metrics.card)
    .width(Length::Fill)
    .into()
}

/// One finding.
fn target_row<'a>(
    palette: Palette,
    metrics: Metrics,
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

    // A privileged target is selectable when the helper knows an operation
    // for it; one that needs root and names no operation cannot be cleaned
    // by anything, so offering a tick would be a lie.
    let selectable = !target.size.is_zero()
        && target.blocked.is_none()
        && target.kind != Kind::Attention
        && (target.is_actionable() || target.privileged.is_some());

    let tick: Element<'a, Message> = if selectable {
        checkbox(state.is_selected(id))
            .size(16)
            .style(style::tick(palette))
            .on_toggle(move |_| Message::Toggle(id))
            .into()
    } else {
        Space::new().width(Length::Fixed(22.0)).into()
    };

    // The reason it cannot be touched displaces the description: "close
    // Brave first" is the only thing worth reading there.
    let detail: Element<'a, Message> = match &target.blocked {
        Some(reason) => text(reason.as_str())
            .size(ty::CAPTION)
            .style(style::tinted(palette.orange))
            .width(Length::Fill)
            .into(),
        None => text(target.detail.as_str())
            .size(ty::CAPTION)
            .style(style::secondary(palette))
            .width(Length::Fill)
            .into(),
    };

    // Narrow enough and the size goes under the name instead of into a
    // column that would leave the name two words wide.
    // Wrapped, so a long name plus two chips spills onto a second line
    // instead of pushing the size off the edge.
    let heading = labels.wrap();

    let inner: Element<'a, Message> = if metrics.two_columns() {
        row![
            tick,
            column![heading, detail].spacing(3).width(Length::Fill),
            container(size).align_right(Length::Fixed(80.0)),
        ]
        .align_y(Alignment::Center)
        .spacing(ty::GAP)
        .into()
    } else {
        // Narrow enough and the size goes under the name instead of into a
        // column that would leave the name two words wide.
        row![
            tick,
            column![
                heading,
                row![detail, Space::new().width(Length::Fill), size]
                    .spacing(ty::GAP_TIGHT)
                    .align_y(Alignment::End),
            ]
            .spacing(3)
            .width(Length::Fill),
        ]
        .align_y(Alignment::Start)
        .spacing(ty::GAP_TIGHT)
        .into()
    };

    container(inner)
        .style(style::well(palette))
        .padding([10, 12])
        .width(Length::Fill)
        .into()
}

/// A button whose label is centred only when the button spans the width.
///
/// A `Fill` label inside a button sitting in a row makes the *button* take
/// the row's slack, which is right when it is the only thing on its line and
/// wrong when it is not.
fn action<'a>(label: &'a str, full_width: bool) -> button::Button<'a, Message> {
    let text = text(label).size(ty::BODY);
    if full_width {
        button(text.width(Length::Fill).center()).width(Length::Fill)
    } else {
        button(text)
    }
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
fn note<'a>(palette: Palette, metrics: Metrics, body: &'a str) -> Element<'a, Message> {
    container(
        row![
            text("\u{24d8}")
                .size(ty::BODY)
                .style(style::tinted(palette.cyan)),
            text(body)
                .size(ty::BODY_SMALL)
                .style(style::secondary(palette))
                .width(Length::Fill),
        ]
        .spacing(ty::GAP_TIGHT),
    )
    .style(style::well(palette))
    .padding(metrics.gap)
    .width(Length::Fill)
    .into()
}
