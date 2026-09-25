//! Turning a Limpid palette into things Iced can draw with.
//!
//! Iced's own `Theme` carries six colours; Limpid's palette carries twenty,
//! and the difference is most of the design. So the palette is kept in the
//! application state and the style functions here close over a copy of it,
//! rather than being derived from whatever theme Iced thinks is active.
//! `Palette` is `Copy`, which makes that cheap enough to do per widget.

use iced::border::Radius;
use iced::widget::{button, checkbox, container, rule, scrollable, text};
use iced::{Background, Border, Color as IcedColor, Shadow, Theme, Vector};

use limpid_theme::{Color, Mode, Palette};

/// Corner radius of a card.
pub const CARD_RADIUS: f32 = 14.0;
/// Corner radius of a control.
pub const CONTROL_RADIUS: f32 = 9.0;

/// Convert a Limpid colour into an Iced one.
pub fn to_iced(color: Color) -> IcedColor {
    let [r, g, b] = color.to_f32();
    IcedColor { r, g, b, a: 1.0 }
}

/// Convert a Limpid colour into an Iced one at partial opacity.
pub fn faded(color: Color, alpha: f32) -> IcedColor {
    IcedColor {
        a: alpha,
        ..to_iced(color)
    }
}

/// Build the Iced theme, which sets the window background and the defaults
/// for anything not styled explicitly.
pub fn theme(palette: Palette) -> Theme {
    Theme::custom(
        "Limpid".to_owned(),
        iced::theme::Palette {
            background: to_iced(palette.background),
            text: to_iced(palette.foreground),
            primary: to_iced(palette.accent),
            success: to_iced(palette.green),
            warning: to_iced(palette.yellow),
            danger: to_iced(palette.red),
        },
    )
}

/// The window background, and the root everything else sits on.
pub fn root(palette: Palette) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(to_iced(palette.background))),
        text_color: Some(to_iced(palette.foreground)),
        ..container::Style::default()
    }
}

/// The navigation column.
pub fn sidebar(palette: Palette) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(to_iced(palette.dark_background))),
        text_color: Some(to_iced(palette.foreground)),
        border: Border {
            // A hairline rather than a full border: the only edge that needs
            // to read is the one against the content area.
            color: to_iced(palette.muted),
            width: 0.0,
            radius: Radius::default(),
        },
        ..container::Style::default()
    }
}

/// A raised panel holding one group of information.
pub fn card(palette: Palette) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(to_iced(palette.lighter_background))),
        text_color: Some(to_iced(palette.foreground)),
        border: Border {
            color: faded(palette.muted, 0.35),
            width: 1.0,
            radius: Radius::from(CARD_RADIUS),
        },
        shadow: card_shadow(palette),
        ..container::Style::default()
    }
}

/// Shadows only read on a light background; on a dark one they turn cards
/// into smudges, so dark themes get separation from the border instead.
fn card_shadow(palette: Palette) -> Shadow {
    match palette.mode {
        Mode::Light => Shadow {
            color: IcedColor {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.07,
            },
            offset: Vector::new(0.0, 2.0),
            blur_radius: 12.0,
        },
        Mode::Dark => Shadow::default(),
    }
}

/// A sunken well, for a value read against its surroundings.
pub fn well(palette: Palette) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(to_iced(palette.dark_background))),
        text_color: Some(to_iced(palette.foreground)),
        border: Border {
            color: IcedColor::TRANSPARENT,
            width: 0.0,
            radius: Radius::from(10.0),
        },
        ..container::Style::default()
    }
}

/// A small coloured label, used for risk levels.
pub fn badge(palette: Palette, tint: Color) -> impl Fn(&Theme) -> container::Style {
    let _ = palette;
    move |_| container::Style {
        background: Some(Background::Color(faded(tint, 0.16))),
        text_color: Some(to_iced(tint)),
        border: Border {
            color: faded(tint, 0.35),
            width: 1.0,
            radius: Radius::from(6.0),
        },
        ..container::Style::default()
    }
}

/// A solid block of one palette colour, for showing the palette itself.
pub fn swatch(palette: Palette, colour: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(to_iced(colour))),
        // Background and surface swatches are near the card they sit on, so
        // without an outline they would read as holes rather than as colours.
        border: Border {
            color: faded(palette.muted, 0.6),
            width: 1.0,
            radius: Radius::from(8.0),
        },
        ..container::Style::default()
    }
}

/// The primary action.
pub fn primary_button(palette: Palette) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let base = match status {
            button::Status::Hovered => palette.accent.mix(Color::WHITE, 0.12),
            button::Status::Pressed => palette.accent.mix(Color::BLACK, 0.12),
            _ => palette.accent,
        };
        button::Style {
            background: Some(Background::Color(match status {
                button::Status::Disabled => faded(palette.accent, 0.35),
                _ => to_iced(base),
            })),
            text_color: to_iced(palette.on(base)),
            border: Border {
                color: IcedColor::TRANSPARENT,
                width: 0.0,
                radius: Radius::from(CONTROL_RADIUS),
            },
            ..button::Style::default()
        }
    }
}

/// A secondary action: outlined, so it never competes with the primary one.
pub fn quiet_button(palette: Palette) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let (background, border) = match status {
            button::Status::Hovered => (faded(palette.selection, 1.0), faded(palette.accent, 0.5)),
            button::Status::Pressed => (faded(palette.selection, 1.0), to_iced(palette.accent)),
            _ => (IcedColor::TRANSPARENT, faded(palette.muted, 0.5)),
        };
        button::Style {
            background: Some(Background::Color(background)),
            text_color: to_iced(palette.foreground),
            border: Border {
                color: border,
                width: 1.0,
                radius: Radius::from(CONTROL_RADIUS),
            },
            ..button::Style::default()
        }
    }
}

/// The button that actually removes things.
///
/// Red, and only ever on the confirmation — a destructive action should not
/// be reachable in one click from the screen you land on.
pub fn danger_button(palette: Palette) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let base = match status {
            button::Status::Hovered => palette.red.mix(Color::WHITE, 0.12),
            button::Status::Pressed => palette.red.mix(Color::BLACK, 0.12),
            _ => palette.red,
        };
        button::Style {
            background: Some(Background::Color(to_iced(base))),
            text_color: to_iced(palette.on(base)),
            border: Border {
                color: IcedColor::TRANSPARENT,
                width: 0.0,
                radius: Radius::from(CONTROL_RADIUS),
            },
            ..button::Style::default()
        }
    }
}

/// A checkbox.
pub fn tick(palette: Palette) -> impl Fn(&Theme, checkbox::Status) -> checkbox::Style {
    move |_, status| {
        let checked = matches!(
            status,
            checkbox::Status::Active { is_checked: true }
                | checkbox::Status::Hovered { is_checked: true }
                | checkbox::Status::Disabled { is_checked: true }
        );
        let hovered = matches!(status, checkbox::Status::Hovered { .. });

        checkbox::Style {
            background: Background::Color(if checked {
                to_iced(palette.accent)
            } else {
                faded(palette.muted, if hovered { 0.35 } else { 0.18 })
            }),
            icon_color: to_iced(palette.on(palette.accent)),
            border: Border {
                color: if checked {
                    to_iced(palette.accent)
                } else {
                    faded(palette.muted, 0.6)
                },
                width: 1.0,
                radius: Radius::from(5.0),
            },
            text_color: None,
        }
    }
}

/// A navigation entry. `selected` is drawn as a filled pill.
pub fn nav_button(
    palette: Palette,
    selected: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let background = if selected {
            Some(Background::Color(faded(palette.accent, 0.16)))
        } else {
            match status {
                button::Status::Hovered | button::Status::Pressed => {
                    Some(Background::Color(faded(palette.muted, 0.18)))
                }
                _ => None,
            }
        };
        button::Style {
            background,
            text_color: if selected {
                to_iced(palette.accent)
            } else {
                to_iced(palette.dark_foreground)
            },
            border: Border {
                color: IcedColor::TRANSPARENT,
                width: 0.0,
                radius: Radius::from(CONTROL_RADIUS),
            },
            ..button::Style::default()
        }
    }
}

/// Body text.
pub fn body(palette: Palette) -> impl Fn(&Theme) -> text::Style {
    move |_| text::Style {
        color: Some(to_iced(palette.foreground)),
    }
}

/// De-emphasised text.
pub fn secondary(palette: Palette) -> impl Fn(&Theme) -> text::Style {
    move |_| text::Style {
        color: Some(to_iced(palette.dark_foreground)),
    }
}

/// A heading.
pub fn heading(palette: Palette) -> impl Fn(&Theme) -> text::Style {
    move |_| text::Style {
        color: Some(to_iced(palette.bright_foreground)),
    }
}

/// Text in an arbitrary palette colour.
pub fn tinted(tint: Color) -> impl Fn(&Theme) -> text::Style {
    move |_| text::Style {
        color: Some(to_iced(tint)),
    }
}

/// A separator.
pub fn divider(palette: Palette) -> impl Fn(&Theme) -> rule::Style {
    move |_| rule::Style {
        color: faded(palette.muted, 0.3),
        radius: Radius::default(),
        fill_mode: rule::FillMode::Full,
        snap: true,
    }
}

/// A scrollbar that stays out of the way until it is used.
pub fn scroller(palette: Palette) -> impl Fn(&Theme, scrollable::Status) -> scrollable::Style {
    move |_, status| {
        let hovered = matches!(
            status,
            scrollable::Status::Hovered { .. } | scrollable::Status::Dragged { .. }
        );
        let rail = scrollable::Rail {
            background: None,
            border: Border::default(),
            scroller: scrollable::Scroller {
                background: Background::Color(faded(
                    palette.muted,
                    if hovered { 0.7 } else { 0.3 },
                )),
                border: Border {
                    radius: Radius::from(4.0),
                    ..Border::default()
                },
            },
        };
        scrollable::Style {
            container: container::Style::default(),
            vertical_rail: rail,
            horizontal_rail: rail,
            gap: None,
            ..scrollable::default(&Theme::Dark, status)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_survive_the_round_trip_into_iced() {
        let converted = to_iced(Color::rgb(0x7a, 0xa2, 0xf7));

        assert!((converted.r - 122.0 / 255.0).abs() < f32::EPSILON);
        assert!((converted.g - 162.0 / 255.0).abs() < f32::EPSILON);
        assert!((converted.b - 247.0 / 255.0).abs() < f32::EPSILON);
        assert_eq!(converted.a, 1.0);
    }

    #[test]
    fn the_iced_theme_takes_its_background_from_the_palette() {
        let palette = Palette::dark();
        let theme = theme(palette);

        assert_eq!(theme.palette().background, to_iced(palette.background));
        assert_eq!(theme.palette().primary, to_iced(palette.accent));
    }

    #[test]
    fn cards_are_only_shadowed_on_light_palettes() {
        // On a dark background a drop shadow reads as a smudge, not as
        // elevation.
        assert_eq!(card_shadow(Palette::dark()), Shadow::default());
        assert_ne!(card_shadow(Palette::light()), Shadow::default());
    }

    #[test]
    fn a_primary_button_labels_itself_readably_over_any_accent() {
        for palette in [Palette::dark(), Palette::light()] {
            let style = primary_button(palette)(&theme(palette), button::Status::Active);
            let label = style.text_color;

            // The label is one of the two palette extremes, chosen for
            // contrast; never a colour that happens to match the fill.
            assert!(
                label == to_iced(palette.bright_foreground)
                    || label == to_iced(palette.darker_background)
            );
        }
    }

    #[test]
    fn a_selected_nav_entry_is_tinted_and_filled() {
        let palette = Palette::dark();
        let theme = theme(palette);

        let selected = nav_button(palette, true)(&theme, button::Status::Active);
        let unselected = nav_button(palette, false)(&theme, button::Status::Active);

        assert_eq!(selected.text_color, to_iced(palette.accent));
        assert!(selected.background.is_some());
        assert!(unselected.background.is_none());
    }
}
