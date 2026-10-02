//! What Limpid keeps, what it leaves alone, and how it looks.

use iced::widget::text::Wrapping;
use iced::widget::{Space, button, column, container, row, text, text_input};
use iced::{Alignment, Element, Length};

use limpid_core::config::Store;
use limpid_theme::Palette;

use crate::app::{Direction, Message, Setting, State};
use crate::layout::Metrics;
use crate::style;
use crate::typography as ty;
use crate::view;

/// Draw the settings page.
pub fn view<'a>(palette: Palette, metrics: Metrics, state: &'a State) -> Element<'a, Message> {
    let theme = state.theme();
    let store = state.config();
    let swatches = [
        ("Accent", palette.accent),
        ("Background", palette.background),
        ("Surface", palette.lighter_background),
        ("Text", palette.foreground),
        ("Safe", palette.green),
        ("Review", palette.yellow),
        ("Sensitive", palette.red),
    ];

    // Wide enough for "Sensitive", so the labels never collide.
    const SWATCH: f32 = 72.0;

    // Seven of these need about 560 px. Rather than let them overflow or
    // squash, they wrap into as many rows as the window has room for.
    let per_row = metrics.swatches_per_row(SWATCH, ty::GAP_TIGHT);
    let mut strip = column![].spacing(ty::GAP_TIGHT);

    for chunk in swatches.chunks(per_row) {
        let mut line = row![].spacing(ty::GAP_TIGHT);
        for (label, colour) in chunk {
            line = line.push(
                column![
                    container(iced::widget::Space::new().height(Length::Fixed(44.0)))
                        .style(style::swatch(palette, *colour))
                        .width(Length::Fixed(SWATCH)),
                    text(*label)
                        .size(ty::CAPTION)
                        .style(style::secondary(palette))
                        .width(Length::Fixed(SWATCH))
                        .center(),
                ]
                .spacing(6),
            );
        }
        strip = strip.push(line);
    }

    let theme_card = card(
        palette,
        metrics,
        "Appearance",
        theme.source.describe(),
        column![
            strip,
            text(if theme.source.is_live() {
                "Limpid repaints when the desktop theme changes."
            } else {
                "There is no desktop theme to follow, so Limpid uses its own."
            })
            .size(ty::BODY_SMALL)
            .style(style::secondary(palette))
            .width(Length::Fill),
        ]
        .spacing(ty::GAP)
        .width(Length::Fill)
        .into(),
    );

    let about = card(
        palette,
        metrics,
        "About",
        format!("Version {}", limpid_core::VERSION),
        text(
            "Limpid finds reclaimable space and removes it carefully. Scanning never \
             changes anything, and system-level cleaning runs in a separate helper \
             that is never given a path.",
        )
        .size(ty::BODY_SMALL)
        .style(style::secondary(palette))
        .width(Length::Fill)
        .into(),
    );

    let mut page = column![].spacing(metrics.gap).width(Length::Fill);

    // Above everything, because it explains why the controls below may not
    // be doing anything.
    if !store.warnings.is_empty() || !store.is_writable() {
        page = page.push(file_problems(palette, metrics, store));
    }
    if let Some(why) = state.config_error() {
        page = page.push(view::failed(palette, metrics, why));
    }

    page.push(cleaning(palette, metrics, store))
        .push(exclusions(palette, metrics, store, state.draft()))
        .push(theme_card)
        .push(about)
        .into()
}

/// How much Limpid leaves behind when it trims system files.
fn cleaning<'a>(palette: Palette, metrics: Metrics, store: &Store) -> Element<'a, Message> {
    let policy = &store.config.policy;
    let versions = policy.keep_package_versions;
    let days = policy.keep_journal_days;

    let body = column![
        setting(
            palette,
            metrics,
            store,
            Setting::PackageVersions,
            "Package versions kept",
            "Older versions stay in the pacman cache so a bad update can be \
             rolled back. Keeping fewer frees more.",
            if versions == 1 {
                "1 version".to_owned()
            } else {
                format!("{versions} versions")
            },
        ),
        setting(
            palette,
            metrics,
            store,
            Setting::JournalDays,
            "System journal kept",
            "How far back the system log can answer \u{201c}what changed before \
             this started?\u{201d}",
            if days == 1 {
                "1 day".to_owned()
            } else {
                format!("{days} days")
            },
        ),
    ]
    .spacing(ty::GAP)
    .width(Length::Fill);

    card(
        palette,
        metrics,
        "Cleaning",
        "Used the next time these are cleaned. Both need your password.",
        body.into(),
    )
}

/// One setting with its explanation and a stepper.
fn setting<'a>(
    palette: Palette,
    metrics: Metrics,
    store: &Store,
    which: Setting,
    title: &'a str,
    explanation: &'a str,
    value: String,
) -> Element<'a, Message> {
    let writable = store.is_writable();
    let config = &store.config;
    let step = |direction: Direction, glyph: &'static str| {
        // Disabled at the ends of the range rather than silently doing
        // nothing, so the edge is visible before it is pressed.
        let possible = writable && which.step(config, direction).is_some();
        button(text(glyph).size(ty::BODY).center())
            .style(style::quiet_button(palette))
            .padding([4, 0])
            .width(Length::Fixed(34.0))
            .on_press_maybe(possible.then_some(Message::Adjust(which, direction)))
    };

    let stepper = row![
        step(Direction::Less, "\u{2212}"),
        // Wide enough for "10 versions" and "365 days", so the buttons do
        // not move as the value changes.
        text(value)
            .size(ty::BODY_SMALL)
            .style(style::body(palette))
            .width(Length::Fixed(84.0))
            .center(),
        step(Direction::More, "+"),
    ]
    .spacing(4)
    .align_y(Alignment::Center);

    let words = column![
        text(title).size(ty::BODY).style(style::body(palette)),
        text(explanation)
            .size(ty::CAPTION)
            .style(style::secondary(palette))
            .width(Length::Fill),
    ]
    .spacing(2)
    .width(Length::Fill);

    if metrics.two_columns() {
        row![words, stepper]
            .spacing(ty::GAP)
            .align_y(Alignment::Center)
            .into()
    } else {
        column![words, stepper].spacing(ty::GAP_TIGHT).into()
    }
}

/// What is never offered and never removed, and the way to add to it.
fn exclusions<'a>(
    palette: Palette,
    metrics: Metrics,
    store: &Store,
    draft: &str,
) -> Element<'a, Message> {
    let writable = store.is_writable();
    let paths = store.config.exclusions.paths();

    let mut list = column![].spacing(4).width(Length::Fill);

    if paths.is_empty() {
        list = list.push(
            text(
                "Nothing is excluded. Tick something on the Overview or Storage page \
                 and choose Exclude, or type a path below.",
            )
            .size(ty::BODY_SMALL)
            .style(style::secondary(palette))
            .width(Length::Fill),
        );
    }

    for path in paths {
        list = list.push(
            container(
                row![
                    // A path is one long token as often as not; without
                    // glyph wrapping it runs under the button beside it.
                    text(store.display(path))
                        .size(ty::BODY_SMALL)
                        .style(style::body(palette))
                        .wrapping(Wrapping::WordOrGlyph)
                        .width(Length::Fill),
                    button(text("Remove").size(ty::CAPTION))
                        .style(style::quiet_button(palette))
                        .padding([4, 10])
                        .on_press_maybe(writable.then(|| Message::Include(vec![path.clone()]))),
                ]
                .spacing(ty::GAP_TIGHT)
                .align_y(Alignment::Center),
            )
            .style(style::well(palette))
            .padding([6, 10])
            .width(Length::Fill),
        );
    }

    let mut field = text_input("~/path/to/keep", draft)
        .size(ty::BODY_SMALL)
        .padding([8, 10])
        .style(style::field(palette))
        .width(Length::Fill);
    if writable {
        field = field
            .on_input(Message::DraftChanged)
            .on_submit(Message::AddDraft);
    }

    let adding = !draft.trim().is_empty() && writable;
    let add = button(text("Exclude").size(ty::BODY_SMALL))
        .style(style::quiet_button(palette))
        .padding([8, 14])
        .on_press_maybe(adding.then_some(Message::AddDraft));

    let adder: Element<'a, Message> = if metrics.buttons_inline() {
        row![field, add]
            .spacing(ty::GAP_TIGHT)
            .align_y(Alignment::Center)
            .into()
    } else {
        column![field, add].spacing(ty::GAP_TIGHT).into()
    };

    card(
        palette,
        metrics,
        "Excluded",
        "Never offered and never removed, including when they sit inside \
         something Limpid is cleaning. The storage view still counts them.",
        column![
            list,
            Space::new().height(Length::Fixed(ty::GAP_TIGHT)),
            adder,
            text(format!(
                "Kept in {}, which you can also edit by hand.",
                store.display(store.path())
            ))
            .size(ty::CAPTION)
            .style(style::secondary(palette))
            .wrapping(Wrapping::WordOrGlyph)
            .width(Length::Fill),
        ]
        .spacing(ty::GAP_TIGHT)
        .width(Length::Fill)
        .into(),
    )
}

/// What was wrong with the file when it was read.
fn file_problems<'a>(palette: Palette, metrics: Metrics, store: &Store) -> Element<'a, Message> {
    let mut lines = column![].spacing(6).width(Length::Fill);

    if !store.is_writable() {
        lines = lines.push(
            text(
                "Limpid will not change this file until it can read all of it, so the \
                 settings below cannot be changed here. Fix or remove the file and open \
                 this page again.",
            )
            .size(ty::BODY_SMALL)
            .style(style::body(palette))
            .width(Length::Fill),
        );
    }

    for warning in &store.warnings {
        lines = lines.push(
            text(warning.clone())
                .size(ty::CAPTION)
                .style(style::tinted(palette.orange))
                .wrapping(Wrapping::WordOrGlyph)
                .width(Length::Fill),
        );
    }

    container(
        column![
            text("About the settings file")
                .size(ty::TITLE)
                .style(style::heading(palette)),
            lines,
        ]
        .spacing(ty::GAP_TIGHT)
        .width(Length::Fill),
    )
    .style(style::card(palette))
    .padding(metrics.card)
    .width(Length::Fill)
    .into()
}

/// A titled panel.
fn card<'a>(
    palette: Palette,
    metrics: Metrics,
    title: &'a str,
    subtitle: impl text::IntoFragment<'a>,
    body: Element<'a, Message>,
) -> Element<'a, Message> {
    container(
        column![
            text(title).size(ty::TITLE).style(style::heading(palette)),
            text(subtitle)
                .size(ty::BODY_SMALL)
                .style(style::secondary(palette))
                .width(Length::Fill),
            iced::widget::Space::new().height(Length::Fixed(ty::GAP_TIGHT)),
            body,
        ]
        .spacing(2)
        .width(Length::Fill),
    )
    .style(style::card(palette))
    .padding(metrics.card)
    .width(Length::Fill)
    .into()
}
