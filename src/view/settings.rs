//! Where the colours come from, and what Limpid is.

use iced::widget::{column, container, row, text};
use iced::{Element, Length};

use limpid_theme::{Palette, Theme as LimpidTheme};

use crate::app::Message;
use crate::layout::Metrics;
use crate::style;
use crate::typography as ty;

/// Draw the settings page.
pub fn view<'a>(palette: Palette, metrics: Metrics, theme: &LimpidTheme) -> Element<'a, Message> {
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

    column![theme_card, about]
        .spacing(metrics.gap)
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
                .style(style::secondary(palette)),
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
