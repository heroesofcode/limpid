//! Application state and the top-level view.

use std::time::Duration;

use iced::widget::{Space, column, container, row, scrollable, text};
use iced::{Element, Length, Subscription, Task};

use limpid_core::catalog::{self, Context};
use limpid_core::model::Scan;
use limpid_theme::{Palette, Source, Theme as LimpidTheme};

use crate::style;
use crate::typography as ty;
use crate::view;

/// Which screen is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// What was found, and how much of it there is.
    Overview,
    /// Where the palette comes from, and what Limpid is.
    Settings,
}

impl Page {
    /// Every page, in navigation order.
    pub const ALL: [Self; 2] = [Self::Overview, Self::Settings];

    /// The label in the sidebar, which is also the page heading.
    pub fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Settings => "Settings",
        }
    }

    /// The line under the heading.
    pub fn subtitle(self) -> &'static str {
        match self {
            Self::Overview => "What is taking up space, and what is safe to let go of",
            Self::Settings => "Where Limpid gets its colours, and what it is",
        }
    }
}

/// How the scan is going.
#[derive(Debug, Clone, Default)]
pub enum Progress {
    /// Not started.
    #[default]
    Idle,
    /// Walking the disk.
    Running,
    /// Finished, with what it found.
    Done(Box<Scan>),
}

/// Everything the window shows.
pub struct State {
    /// The colours in force, and where they came from.
    theme: LimpidTheme,
    /// The visible page.
    page: Page,
    /// The scan.
    progress: Progress,
}

/// Everything that can happen.
#[derive(Debug, Clone)]
pub enum Message {
    /// A sidebar entry was chosen.
    Navigate(Page),
    /// The scan button was pressed.
    StartScan,
    /// The scan finished.
    ScanFinished(Box<Scan>),
    /// The desktop theme changed underneath us.
    PaletteChanged(Box<Palette>),
}

impl State {
    /// Build the initial state and kick off the first scan.
    ///
    /// Scanning immediately is the right default: the question the user
    /// opened the window to ask is always the same one.
    pub fn boot() -> (Self, Task<Message>) {
        let state = Self {
            theme: LimpidTheme::detect(),
            page: Page::Overview,
            progress: Progress::Idle,
        };
        (state, Task::done(Message::StartScan))
    }

    /// The colours in force.
    pub fn palette(&self) -> Palette {
        self.theme.palette
    }

    /// Where the colours came from.
    pub fn source(&self) -> &Source {
        &self.theme.source
    }

    /// The Iced theme, which sets the window background.
    pub fn iced_theme(&self) -> iced::Theme {
        style::theme(self.palette())
    }

    /// The window title.
    pub fn title(&self) -> String {
        "Limpid".to_owned()
    }

    /// Handle a message.
    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Navigate(page) => {
                self.page = page;
                Task::none()
            }
            Message::StartScan => {
                if matches!(self.progress, Progress::Running) {
                    return Task::none();
                }
                self.progress = Progress::Running;
                // The walk is IO-bound and long enough to be felt, so it goes
                // to a blocking thread rather than stalling the frame loop.
                Task::perform(
                    async {
                        tokio::task::spawn_blocking(|| catalog::scan(&Context::new()))
                            .await
                            .unwrap_or_default()
                    },
                    |scan| Message::ScanFinished(Box::new(scan)),
                )
            }
            Message::ScanFinished(scan) => {
                self.progress = Progress::Done(scan);
                Task::none()
            }
            Message::PaletteChanged(palette) => {
                self.theme.palette = *palette;
                Task::none()
            }
        }
    }

    /// Draw the window.
    pub fn view(&self) -> Element<'_, Message> {
        let palette = self.palette();

        let body = match self.page {
            Page::Overview => view::overview::view(palette, &self.progress),
            Page::Settings => view::settings::view(palette, &self.theme),
        };

        let header = column![
            text(self.page.title())
                .size(ty::HEADING)
                .style(style::heading(palette)),
            text(self.page.subtitle())
                .size(ty::BODY_SMALL)
                .style(style::secondary(palette)),
        ]
        .spacing(2);

        let content = column![header, body].spacing(ty::GAP_WIDE);

        let scrolled = scrollable(container(content).padding(ty::MARGIN).width(Length::Fill))
            .style(style::scroller(palette))
            .height(Length::Fill);

        container(row![
            view::sidebar::view(palette, self.page, self.source()),
            container(scrolled).width(Length::Fill).height(Length::Fill),
        ])
        .style(style::root(palette))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    /// Follow the desktop theme for as long as the window is open.
    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::run(palette_changes)
    }
}

/// A stream of palettes, one per theme change.
///
/// The watch is blocking, so it lives on its own thread and pushes into the
/// stream; the async side then parks forever, because returning would end the
/// subscription and Iced would not restart it.
fn palette_changes() -> impl iced::futures::Stream<Item = Message> {
    iced::stream::channel(
        4,
        |output: iced::futures::channel::mpsc::Sender<Message>| async move {
            let locations = limpid_theme::omarchy::Locations::standard();

            match limpid_theme::watch::watch(locations) {
                Ok(watcher) => {
                    let mut output = output;
                    let spawned = std::thread::Builder::new()
                        .name("limpid-theme-bridge".to_owned())
                        .spawn(move || {
                            while let Some(palette) = watcher.next_blocking() {
                                if output
                                    .try_send(Message::PaletteChanged(Box::new(palette)))
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        });
                    if let Err(error) = spawned {
                        tracing::warn!(%error, "could not start the theme watch");
                    }
                }
                Err(error) => {
                    // Ordinary off Omarchy: there is no state directory to watch.
                    tracing::debug!(%error, "not following a system theme");
                }
            }

            // Parking rather than returning: an ended stream is a subscription
            // Iced will not bring back.
            loop {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        },
    )
}

/// Placeholder shown while the scan runs and before the first result.
pub fn placeholder<'a>(palette: Palette, message: &'a str) -> Element<'a, Message> {
    container(
        column![
            Space::new().height(Length::Fixed(ty::GAP_WIDE)),
            text(message)
                .size(ty::BODY)
                .style(style::secondary(palette)),
        ]
        .spacing(ty::GAP),
    )
    .center_x(Length::Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_has_a_title_and_a_subtitle() {
        for page in Page::ALL {
            assert!(!page.title().is_empty());
            assert!(!page.subtitle().is_empty());
        }
    }

    #[test]
    fn navigating_changes_the_page_without_disturbing_the_scan() {
        let (mut state, _) = State::boot();
        state.progress = Progress::Running;

        let _ = state.update(Message::Navigate(Page::Settings));

        assert_eq!(state.page, Page::Settings);
        assert!(matches!(state.progress, Progress::Running));
    }

    #[test]
    fn a_second_scan_request_while_one_is_running_is_ignored() {
        let (mut state, _) = State::boot();
        state.progress = Progress::Running;

        let task = state.update(Message::StartScan);

        // Task has no public emptiness predicate, so the observable effect is
        // that the state is untouched.
        assert!(matches!(state.progress, Progress::Running));
        drop(task);
    }

    #[test]
    fn a_theme_change_repaints_with_the_new_palette() {
        let (mut state, _) = State::boot();
        let light = Palette::light();

        let _ = state.update(Message::PaletteChanged(Box::new(light)));

        assert_eq!(state.palette(), light);
        assert_eq!(
            state.iced_theme().palette().background,
            style::to_iced(light.background)
        );
    }

    #[test]
    fn a_finished_scan_is_kept() {
        let (mut state, _) = State::boot();

        let _ = state.update(Message::ScanFinished(Box::default()));

        assert!(matches!(state.progress, Progress::Done(_)));
    }
}
