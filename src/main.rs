//! The Limpid desktop application.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod app;
mod layout;
mod style;
mod typography;
mod view;
mod widget;

use app::State;

/// Starting size, for the case where the window manager lets the window
/// choose. Wide enough for the ring and its figures side by side, and short
/// enough to open whole on a 1280x720 laptop.
const WINDOW: iced::Size = iced::Size::new(1020.0, 660.0);
/// The smallest the interface still works at.
///
/// A tiling compositor honours this, which means a floor set too high makes
/// the window overflow its tile rather than fit it. Four columns on a 1152
/// pixel screen is 288 each, so the floor is below that and the layout is
/// expected to cope rather than the window to refuse.
const MINIMUM: iced::Size = iced::Size::new(240.0, 240.0);

/// Wayland app id, which has to match the desktop entry's file name for the
/// compositor to associate the window with it. Without one the window has no
/// class at all, so it gets no icon and no window rules.
const APP_ID: &str = "org.limpid.Limpid";

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "limpid=info,warn".into()),
        )
        .init();

    iced::application(State::boot, State::update, State::view)
        .title(State::title)
        .theme(State::iced_theme)
        .subscription(State::subscription)
        .window(iced::window::Settings {
            size: WINDOW,
            min_size: Some(MINIMUM),
            platform_specific: iced::window::settings::PlatformSpecific {
                application_id: APP_ID.to_owned(),
                ..Default::default()
            },
            ..iced::window::Settings::default()
        })
        .run()
}
