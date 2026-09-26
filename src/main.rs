//! The Limpid desktop application.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod app;
mod style;
mod typography;
mod view;
mod widget;

use app::State;

/// Starting size. Wide enough for the ring and its figures side by side, and
/// short enough to open whole on a 1280x720 laptop — which is the smallest
/// screen this is likely to meet, and smaller than it looks once a bar and
/// window gaps are taken out.
const WINDOW: iced::Size = iced::Size::new(1020.0, 660.0);
/// Below this the sidebar and a card cannot coexist.
const MINIMUM: iced::Size = iced::Size::new(680.0, 460.0);

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
