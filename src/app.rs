//! Application state and the top-level view.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use iced::widget::{Space, column, container, responsive, row, scrollable, text};
use iced::{Element, Length, Subscription, Task};

use limpid_core::analyse::{self, Survey};
use limpid_core::catalog::{self, Context};
use limpid_core::execute::{Executor, Outcome};
use limpid_core::model::{Scan, Target};
use limpid_core::paths::Roots;
use limpid_core::plan::{Disposal, Plan, Selection};
use limpid_core::privileged::{Report, Request, Runner};
use limpid_core::size::Size;
use limpid_core::walk::WalkOptions;
use limpid_theme::{Palette, Source, Theme as LimpidTheme};

use crate::layout::Metrics;
use crate::style;
use crate::typography as ty;
use crate::view;

/// Which screen is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// What was found, and how much of it there is.
    Overview,
    /// Where the space went, whatever it is.
    Storage,
    /// Where the palette comes from, and what Limpid is.
    Settings,
}

impl Page {
    /// Every page, in navigation order.
    pub const ALL: [Self; 3] = [Self::Overview, Self::Storage, Self::Settings];

    /// The label in the sidebar, which is also the page heading.
    pub fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Storage => "Storage",
            Self::Settings => "Settings",
        }
    }

    /// The line under the heading.
    pub fn subtitle(self) -> &'static str {
        match self {
            Self::Overview => "What is taking up space, and what is safe to let go of",
            Self::Storage => "Where the space went, with no opinion about whether it should have",
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

/// Which finding a checkbox belongs to: the category's position, then the
/// target's within it. Names are not unique across categories, and indices
/// are stable for as long as a given scan is on screen.
pub type TargetId = (usize, usize);

/// What a clean did, both halves.
#[derive(Debug, Clone, Default)]
pub struct Cleaned {
    /// What this process removed.
    pub outcome: Outcome,
    /// What the helper did, if it was asked. The error is already a
    /// sentence, because it has to survive crossing a task boundary.
    pub elevated: Option<Result<Report, String>>,
}

/// How many of the largest files to list per level.
const LARGEST_FILES: usize = 8;

/// The storage page's own state.
#[derive(Default)]
pub struct Storage {
    /// The path from the starting directory down to the one on screen, which
    /// is also the breadcrumb.
    pub trail: Vec<PathBuf>,
    /// What the current level holds.
    pub survey: Option<Survey>,
    /// Whether a walk is under way.
    pub working: bool,
    /// Files the user has ticked, by path.
    ///
    /// By path and not by row index: a walk that finishes late replaces the
    /// list underneath, and an index would then point at a different file.
    /// The generation counter stops the common case; this makes the wrong
    /// thing unrepresentable rather than merely unlikely.
    pub selected: BTreeSet<PathBuf>,
    /// Whether the permanent-deletion confirmation is up.
    pub confirming_delete: bool,
    /// Whether a removal is under way.
    pub removing: bool,
    /// What the last removal from this page did.
    pub outcome: Option<Outcome>,
    /// Which walk the result on screen belongs to.
    ///
    /// Walks take seconds and finish out of order, so a slow one started
    /// earlier can land after a fast one started later. Without this the
    /// older result wins and the screen shows one directory's contents under
    /// another's name — which is merely wrong today, and deletes the wrong
    /// file as soon as a row can be ticked.
    pub generation: u64,
}

impl Storage {
    /// The directory being shown.
    pub fn current(&self) -> Option<&PathBuf> {
        self.trail.last()
    }

    /// Whether a path is ticked.
    pub fn is_selected(&self, path: &std::path::Path) -> bool {
        self.selected.contains(path)
    }

    /// The ticked files, with the sizes measured for them.
    ///
    /// Read back out of the survey on screen rather than remembered, so a
    /// selection can only ever name something currently visible.
    fn chosen(&self) -> Vec<(PathBuf, Size)> {
        let Some(survey) = &self.survey else {
            return Vec::new();
        };

        survey
            .breakdown
            .children
            .iter()
            .chain(&survey.largest)
            .filter(|entry| !entry.is_dir && self.selected.contains(&entry.path))
            .map(|entry| (entry.path.clone(), entry.size))
            // The same file can appear in both lists.
            .collect::<std::collections::BTreeMap<_, _>>()
            .into_iter()
            .collect()
    }

    /// What acting on the current selection would do.
    pub fn plan(&self, disposal: Disposal) -> Plan {
        Plan::from_chosen(self.chosen(), disposal)
    }
}

/// Everything the window shows.
pub struct State {
    /// The colours in force, and where they came from.
    theme: LimpidTheme,
    /// The visible page.
    page: Page,
    /// The scan.
    progress: Progress,
    /// What the user has ticked.
    selected: BTreeSet<TargetId>,
    /// Whether the confirmation is up.
    confirming: bool,
    /// Whether a clean is under way.
    cleaning: bool,
    /// What the last clean did.
    outcome: Option<Cleaned>,
    /// The storage page.
    storage: Storage,
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
    /// A finding was ticked or unticked.
    Toggle(TargetId),
    /// Tick everything that regenerates itself with no consequence.
    SelectSafe,
    /// Untick everything.
    SelectNone,
    /// The clean button was pressed; show what is about to happen.
    AskToClean,
    /// The confirmation was dismissed.
    Cancel,
    /// The confirmation was accepted.
    Clean,
    /// The clean finished.
    Cleaned(Box<Cleaned>),
    /// Look at a directory on the storage page.
    Explore(PathBuf),
    /// A directory finished being measured, for the walk of this generation.
    Explored(u64, Box<Survey>),
    /// A tile on the treemap was clicked.
    Descend(usize),
    /// A breadcrumb was clicked; go back to that depth.
    Ascend(usize),
    /// A file on the storage page was ticked or unticked.
    ToggleFile(PathBuf),
    /// Untick everything on the storage page.
    ClearSelection,
    /// Move the ticked files to the trash. Reversible, so it acts directly.
    TrashSelected,
    /// Ask before removing the ticked files outright.
    AskToDelete,
    /// The permanent-deletion confirmation was dismissed.
    CancelDelete,
    /// Remove the ticked files outright.
    DeleteSelected,
    /// A removal from the storage page finished.
    SelectionRemoved(Box<Outcome>),
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
            selected: BTreeSet::new(),
            confirming: false,
            cleaning: false,
            outcome: None,
            storage: Storage::default(),
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

    /// Whether a finding is ticked.
    pub fn is_selected(&self, id: TargetId) -> bool {
        self.selected.contains(&id)
    }

    /// Whether the confirmation is up.
    pub fn is_confirming(&self) -> bool {
        self.confirming
    }

    /// Whether a clean is under way.
    pub fn is_cleaning(&self) -> bool {
        self.cleaning
    }

    /// What the last clean did, if there was one.
    pub fn outcome(&self) -> Option<&Cleaned> {
        self.outcome.as_ref()
    }

    /// The storage page's state.
    pub fn storage(&self) -> &Storage {
        &self.storage
    }

    /// How the scan is going.
    pub fn progress(&self) -> &Progress {
        &self.progress
    }

    /// The scan on screen, if there is one.
    fn scan(&self) -> Option<&Scan> {
        match &self.progress {
            Progress::Done(scan) => Some(scan),
            _ => None,
        }
    }

    /// The findings the user has ticked.
    pub fn chosen(&self) -> Vec<&Target> {
        let Some(scan) = self.scan() else {
            return Vec::new();
        };
        self.selected
            .iter()
            .filter_map(|&(category, target)| scan.categories.get(category)?.targets.get(target))
            .collect()
    }

    /// What acting on the current selection would do.
    pub fn plan(&self) -> Plan {
        Plan::from_targets(self.chosen())
    }

    /// Act on the storage page's selection.
    fn remove_selection(&mut self, disposal: Disposal) -> Task<Message> {
        let plan = self.storage.plan(disposal);
        if plan.is_empty() {
            self.storage.confirming_delete = false;
            return Task::none();
        }

        self.storage.confirming_delete = false;
        self.storage.removing = true;
        self.storage.outcome = None;

        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    Executor::applying(&Roots::from_env()).run(&plan)
                })
                .await
                .unwrap_or_default()
            },
            |outcome| Message::SelectionRemoved(Box::new(outcome)),
        )
    }

    /// Tick everything that regenerates itself with no consequence.
    ///
    /// The starting point after every scan: it is the selection a careful
    /// person would arrive at, and it is never the dangerous one.
    fn select_safe(&mut self) {
        self.selected.clear();
        let Some(scan) = self.scan() else {
            return;
        };
        let safe = Selection::SAFE;
        self.selected = scan
            .categories
            .iter()
            .enumerate()
            .flat_map(|(c, category)| {
                category
                    .targets
                    .iter()
                    .enumerate()
                    .filter(|(_, target)| safe.includes(target) && !target.size.is_zero())
                    .map(move |(t, _)| (c, t))
            })
            .collect();
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
                // Measuring a whole home directory takes seconds, so it
                // happens when the page is first opened rather than at start
                // up, and only once.
                if page == Page::Storage && self.storage.trail.is_empty() {
                    return Task::done(Message::Explore(Roots::from_env().home));
                }
                Task::none()
            }
            Message::Explore(path) => {
                // Cleared on every move. A selection that survived
                // navigation would put files the user cannot see on screen
                // into the next confirmation.
                if self.storage.trail.last() != Some(&path) {
                    self.storage.trail.push(path.clone());
                    self.storage.selected.clear();
                }
                self.storage.confirming_delete = false;
                self.storage.working = true;
                self.storage.survey = None;
                self.storage.generation += 1;
                let generation = self.storage.generation;
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            analyse::survey(&path, LARGEST_FILES, &WalkOptions::default())
                                .unwrap_or_default()
                        })
                        .await
                        .unwrap_or_default()
                    },
                    move |survey| Message::Explored(generation, Box::new(survey)),
                )
            }
            Message::ToggleFile(path) => {
                if !self.storage.selected.remove(&path) {
                    self.storage.selected.insert(path);
                }
                Task::none()
            }
            Message::ClearSelection => {
                self.storage.selected.clear();
                self.storage.confirming_delete = false;
                Task::none()
            }
            Message::AskToDelete => {
                self.storage.confirming_delete = !self.storage.plan(Disposal::Delete).is_empty();
                Task::none()
            }
            Message::CancelDelete => {
                self.storage.confirming_delete = false;
                Task::none()
            }
            Message::TrashSelected => self.remove_selection(Disposal::Trash),
            Message::DeleteSelected => self.remove_selection(Disposal::Delete),
            Message::SelectionRemoved(outcome) => {
                self.storage.outcome = Some(*outcome);
                self.storage.selected.clear();
                self.storage.removing = false;
                // Measure again rather than adjust: the figures on screen
                // are a measurement, and after a removal they are stale.
                match self.storage.trail.last().cloned() {
                    Some(path) => Task::done(Message::Explore(path)),
                    None => Task::none(),
                }
            }
            Message::Explored(generation, survey) => {
                // A result from a walk that has been superseded is dropped.
                // It describes a directory nobody is looking at any more,
                // and every index in it would point at the wrong row.
                if generation != self.storage.generation {
                    return Task::none();
                }
                self.storage.working = false;
                self.storage.survey = Some(*survey);
                Task::none()
            }
            Message::Descend(index) => {
                let Some(survey) = &self.storage.survey else {
                    return Task::none();
                };
                let Some(child) = survey.breakdown.children.get(index) else {
                    return Task::none();
                };
                // A file has nothing inside it to show.
                if !child.is_dir {
                    return Task::none();
                }
                Task::done(Message::Explore(child.path.clone()))
            }
            Message::Ascend(depth) => {
                if depth + 1 >= self.storage.trail.len() {
                    return Task::none();
                }
                // Trimming here rather than letting Explore rebuild it keeps
                // the breadcrumb correct for the frame that renders while
                // the walk is still running.
                self.storage.trail.truncate(depth + 1);
                Task::done(Message::Explore(self.storage.trail[depth].clone()))
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
                self.cleaning = false;
                self.select_safe();
                Task::none()
            }
            Message::Toggle(id) => {
                if !self.selected.remove(&id) {
                    self.selected.insert(id);
                }
                Task::none()
            }
            Message::SelectSafe => {
                self.select_safe();
                Task::none()
            }
            Message::SelectNone => {
                self.selected.clear();
                Task::none()
            }
            Message::AskToClean => {
                // Nothing selected is not a question worth asking.
                self.confirming = !self.plan().is_empty();
                Task::none()
            }
            Message::Cancel => {
                self.confirming = false;
                Task::none()
            }
            Message::Clean => {
                let plan = self.plan();
                if plan.is_empty() {
                    self.confirming = false;
                    return Task::none();
                }
                self.confirming = false;
                self.cleaning = true;
                self.outcome = None;
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            let outcome = Executor::applying(&Roots::from_env()).run(&plan);

                            // Asked for second, and only when there is
                            // something to ask about: an authentication
                            // prompt for nothing would be its own kind of
                            // rude.
                            let elevated = plan.needs_elevation().then(|| {
                                Runner::new()
                                    .run(&Request {
                                        operations: plan.operations.clone(),
                                    })
                                    .map_err(|error| error.to_string())
                            });

                            Cleaned { outcome, elevated }
                        })
                        .await
                        .unwrap_or_default()
                    },
                    |cleaned| Message::Cleaned(Box::new(cleaned)),
                )
            }
            Message::Cleaned(cleaned) => {
                self.outcome = Some(*cleaned);
                // Rescan rather than adjust the numbers in place: what was
                // actually reclaimed is a measurement, not an assumption.
                Task::done(Message::StartScan)
            }
            Message::PaletteChanged(palette) => {
                self.theme.palette = *palette;
                Task::none()
            }
        }
    }

    /// Draw the window.
    ///
    /// Wrapped in `responsive` because the whole shape of the interface
    /// depends on how much room there is — in a tiling window manager the
    /// window does not choose its own size and can be handed a quarter of a
    /// small screen. Everything below takes the resulting [`Metrics`] rather
    /// than assuming a width.
    pub fn view(&self) -> Element<'_, Message> {
        let palette = self.palette();

        responsive(move |size| {
            let metrics = Metrics::of(size);

            let body = match self.page {
                Page::Overview => view::overview::view(palette, metrics, self),
                Page::Storage => view::storage::view(palette, metrics, self.storage()),
                Page::Settings => view::settings::view(palette, metrics, &self.theme),
            };

            let header = column![
                text(self.page.title())
                    .size(metrics.heading())
                    .style(style::heading(palette)),
                text(self.page.subtitle())
                    .size(ty::BODY_SMALL)
                    .style(style::secondary(palette))
                    .width(Length::Fill),
            ]
            // Fill, not the default Shrink. A `Fill` text inside a `Shrink`
            // column resolves to the text's natural width, so the whole
            // chain from here down has to be Fill or nothing wraps — it
            // just runs off the edge.
            .spacing(2)
            .width(Length::Fill);

            let content = column![header, body]
                .spacing(metrics.gap)
                .width(Length::Fill);

            let scrolled = scrollable(
                container(content)
                    .padding(metrics.margin)
                    .width(Length::Fill),
            )
            .style(style::scroller(palette))
            .height(Length::Fill);

            // Beside the content when there is room for it, above when there
            // is not. Both containers are Fill on purpose: `row!` and
            // `column!` are Shrink by default, and a Shrink ancestor turns
            // every Fill below it into "as wide as the content wants".
            let shell: Element<'_, Message> = if metrics.sidebar {
                row![
                    view::sidebar::column(palette, self.page, self.source()),
                    container(scrolled).width(Length::Fill).height(Length::Fill),
                ]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
            } else {
                column![
                    view::sidebar::tabs(palette, metrics, self.page),
                    container(scrolled).width(Length::Fill).height(Length::Fill),
                ]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
            };

            container(shell)
                .style(style::root(palette))
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        })
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

/// Placeholder shown while work is under way and before the first result.
pub fn placeholder<'a>(
    palette: Palette,
    message: impl text::IntoFragment<'a>,
) -> Element<'a, Message> {
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

    /// A scan with one of each interesting shape.
    fn sample_scan() -> Scan {
        use limpid_core::model::{Category, Kind, Risk};
        use limpid_core::size::Size;

        let sized =
            |name: &str, kind, risk| Target::new(name, kind, risk).measured(Size::new(100, 100), 1);

        let mut category = Category::new("Caches", "");
        category.targets = vec![
            sized("safe", Kind::Cache, Risk::Safe),
            sized("review", Kind::Cache, Risk::Review),
            sized("privileged", Kind::Cache, Risk::Safe).requires_root(),
            Target::new("empty", Kind::Cache, Risk::Safe),
            Target::new("pacnew", Kind::Attention, Risk::Sensitive),
        ];

        Scan {
            categories: vec![category],
            ..Scan::default()
        }
    }

    fn state_with_scan() -> State {
        let (mut state, _) = State::boot();
        let _ = state.update(Message::ScanFinished(Box::new(sample_scan())));
        state
    }

    #[test]
    fn a_scan_arrives_with_only_the_uncontroversial_items_ticked() {
        let state = state_with_scan();

        assert!(state.is_selected((0, 0)), "the safe one should be ticked");
        assert!(!state.is_selected((0, 1)), "review needs a decision");
        assert!(!state.is_selected((0, 2)), "this process cannot act on it");
        assert!(!state.is_selected((0, 3)), "nothing to reclaim");
        assert!(!state.is_selected((0, 4)), "not reclaimable space at all");
    }

    #[test]
    fn ticking_is_a_toggle() {
        let mut state = state_with_scan();

        let _ = state.update(Message::Toggle((0, 1)));
        assert!(state.is_selected((0, 1)));

        let _ = state.update(Message::Toggle((0, 1)));
        assert!(!state.is_selected((0, 1)));
    }

    #[test]
    fn the_plan_follows_the_selection() {
        let mut state = state_with_scan();
        assert_eq!(state.plan().items.len(), 1);

        let _ = state.update(Message::Toggle((0, 1)));
        assert_eq!(state.plan().items.len(), 2);
        assert_eq!(state.plan().expected().on_disk, 200);

        let _ = state.update(Message::SelectNone);
        assert!(state.plan().is_empty());
    }

    #[test]
    fn a_selection_that_would_do_nothing_does_not_raise_a_confirmation() {
        let mut state = state_with_scan();
        let _ = state.update(Message::SelectNone);

        let _ = state.update(Message::AskToClean);

        assert!(!state.is_confirming());
    }

    #[test]
    fn confirming_can_be_backed_out_of() {
        let mut state = state_with_scan();

        let _ = state.update(Message::AskToClean);
        assert!(state.is_confirming());

        let _ = state.update(Message::Cancel);
        assert!(!state.is_confirming());
        assert!(!state.is_cleaning());
    }

    #[test]
    fn accepting_the_confirmation_closes_it_and_starts_work() {
        let mut state = state_with_scan();
        let _ = state.update(Message::AskToClean);

        let task = state.update(Message::Clean);

        assert!(!state.is_confirming());
        assert!(state.is_cleaning());
        drop(task);
    }

    #[test]
    fn accepting_with_nothing_chosen_starts_nothing() {
        let mut state = state_with_scan();
        let _ = state.update(Message::SelectNone);

        let task = state.update(Message::Clean);

        assert!(!state.is_cleaning());
        drop(task);
    }

    #[test]
    fn selecting_safe_again_restores_the_starting_point() {
        let mut state = state_with_scan();
        let _ = state.update(Message::Toggle((0, 1)));
        let _ = state.update(Message::Toggle((0, 0)));

        let _ = state.update(Message::SelectSafe);

        assert!(state.is_selected((0, 0)));
        assert!(!state.is_selected((0, 1)));
    }

    #[test]
    fn a_privileged_finding_can_be_ticked_and_becomes_an_operation() {
        use limpid_core::model::{Category, Kind, Risk};
        use limpid_core::privileged::Operation;
        use limpid_core::size::Size;

        let mut category = Category::new("Package manager", "");
        category.targets = vec![
            Target::new("pacman", Kind::PackageCache, Risk::Review)
                .measured(Size::new(900, 900), 1)
                .by_operation(Operation::TrimPackageCache { keep: 3 }),
        ];
        let scan = Scan {
            categories: vec![category],
            ..Scan::default()
        };

        let (mut state, _) = State::boot();
        let _ = state.update(Message::ScanFinished(Box::new(scan)));

        // Not ticked by default: it costs a password prompt.
        assert!(!state.is_selected((0, 0)));
        assert!(state.plan().is_empty());

        let _ = state.update(Message::Toggle((0, 0)));
        let plan = state.plan();

        assert!(plan.needs_elevation());
        assert_eq!(
            plan.operations,
            vec![Operation::TrimPackageCache { keep: 3 }]
        );
        // And it is a named operation, not a path handed across.
        assert!(plan.items.is_empty());
    }

    #[test]
    fn a_finished_clean_is_reported_and_triggers_a_fresh_measurement() {
        let mut state = state_with_scan();

        let _ = state.update(Message::Cleaned(Box::default()));

        assert!(state.outcome().is_some());
        // The figure shown afterwards is measured again rather than assumed.
        let _ = state.update(Message::ScanFinished(Box::new(sample_scan())));
        assert!(!state.is_cleaning());
    }

    #[test]
    fn opening_the_storage_page_starts_a_measurement_once() {
        let (mut state, _) = State::boot();

        let first = state.update(Message::Navigate(Page::Storage));
        assert_eq!(state.page, Page::Storage);
        drop(first);

        // Simulate the walk that the task would have started.
        let _ = state.update(Message::Explore(PathBuf::from("/tmp")));
        let _ = state.update(Message::Explored(
            state.storage().generation,
            Box::default(),
        ));
        assert_eq!(state.storage().trail.len(), 1);

        // Coming back later does not measure again.
        let _ = state.update(Message::Navigate(Page::Overview));
        let _ = state.update(Message::Navigate(Page::Storage));
        assert_eq!(state.storage().trail.len(), 1);
    }

    #[test]
    fn descending_into_something_that_is_not_a_directory_goes_nowhere() {
        use limpid_core::analyse::{Breakdown, Entry, Survey};
        use limpid_core::size::Size;

        let (mut state, _) = State::boot();
        let _ = state.update(Message::Explore(PathBuf::from("/tmp")));
        let generation = state.storage().generation;
        let _ = state.update(Message::Explored(
            generation,
            Box::new(Survey {
                breakdown: Breakdown {
                    children: vec![Entry {
                        path: "/tmp/film.mkv".into(),
                        name: "film.mkv".into(),
                        size: Size::new(10, 10),
                        files: 1,
                        is_dir: false,
                    }],
                    ..Breakdown::default()
                },
                largest: Vec::new(),
            }),
        ));

        let _ = state.update(Message::Descend(0));
        let _ = state.update(Message::Descend(99));

        assert_eq!(state.storage().trail, vec![PathBuf::from("/tmp")]);
    }

    /// A survey holding one directory and one file, both selectable-ish.
    fn survey_with_a_file() -> limpid_core::analyse::Survey {
        use limpid_core::analyse::{Breakdown, Entry, Survey};
        use limpid_core::size::Size;

        let entry = |name: &str, is_dir: bool, bytes: u64| Entry {
            path: PathBuf::from(format!("/home/x/{name}")),
            name: name.to_owned(),
            size: Size::new(bytes, bytes),
            files: 1,
            is_dir,
        };

        Survey {
            breakdown: Breakdown {
                children: vec![entry("Videos", true, 900), entry("big.iso", false, 4096)],
                ..Breakdown::default()
            },
            largest: vec![entry("big.iso", false, 4096)],
        }
    }

    fn state_on_storage() -> State {
        let (mut state, _) = State::boot();
        let _ = state.update(Message::Explore(PathBuf::from("/home/x")));
        let generation = state.storage().generation;
        let _ = state.update(Message::Explored(
            generation,
            Box::new(survey_with_a_file()),
        ));
        state
    }

    #[test]
    fn ticking_a_file_on_the_storage_page_is_a_toggle() {
        let mut state = state_on_storage();
        let file = PathBuf::from("/home/x/big.iso");

        let _ = state.update(Message::ToggleFile(file.clone()));
        assert!(state.storage().is_selected(&file));

        let _ = state.update(Message::ToggleFile(file.clone()));
        assert!(!state.storage().is_selected(&file));
    }

    #[test]
    fn a_selection_becomes_a_chosen_plan_that_goes_to_the_trash() {
        use limpid_core::guard::Permission;

        let mut state = state_on_storage();
        let _ = state.update(Message::ToggleFile(PathBuf::from("/home/x/big.iso")));

        let plan = state.storage().plan(Disposal::Trash);

        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].permission, Permission::Chosen);
        assert_eq!(plan.items[0].disposal, Disposal::Trash);
        // The same file is in both the children and the largest list; it
        // must be counted once.
        assert_eq!(plan.expected().on_disk, 4096);
    }

    #[test]
    fn a_selection_cannot_name_a_file_the_user_has_navigated_away_from() {
        let mut state = state_on_storage();
        let _ = state.update(Message::ToggleFile(PathBuf::from("/home/x/big.iso")));
        assert!(!state.storage().plan(Disposal::Trash).is_empty());

        let _ = state.update(Message::Explore(PathBuf::from("/home/x/Videos")));

        assert!(state.storage().selected.is_empty());
        assert!(state.storage().plan(Disposal::Trash).is_empty());
    }

    #[test]
    fn a_selection_that_would_delete_nothing_raises_no_confirmation() {
        let mut state = state_on_storage();

        let _ = state.update(Message::AskToDelete);

        assert!(!state.storage().confirming_delete);
    }

    #[test]
    fn the_permanent_deletion_confirmation_can_be_backed_out_of() {
        let mut state = state_on_storage();
        let _ = state.update(Message::ToggleFile(PathBuf::from("/home/x/big.iso")));

        let _ = state.update(Message::AskToDelete);
        assert!(state.storage().confirming_delete);

        let _ = state.update(Message::CancelDelete);
        assert!(!state.storage().confirming_delete);
        assert!(!state.storage().removing);
    }

    #[test]
    fn trashing_acts_directly_because_it_is_reversible() {
        let mut state = state_on_storage();
        let _ = state.update(Message::ToggleFile(PathBuf::from("/home/x/big.iso")));

        let task = state.update(Message::TrashSelected);

        assert!(state.storage().removing);
        assert!(!state.storage().confirming_delete);
        drop(task);
    }

    #[test]
    fn a_finished_removal_clears_the_selection_and_measures_again() {
        let mut state = state_on_storage();
        let _ = state.update(Message::ToggleFile(PathBuf::from("/home/x/big.iso")));
        let _ = state.update(Message::TrashSelected);

        let task = state.update(Message::SelectionRemoved(Box::default()));

        assert!(state.storage().selected.is_empty());
        assert!(state.storage().outcome.is_some());
        assert!(!state.storage().removing);
        drop(task);
    }

    #[test]
    fn a_directory_is_never_part_of_a_selection_plan() {
        // Directories have no checkbox, and nothing else may put one in.
        let mut state = state_on_storage();
        let _ = state.update(Message::ToggleFile(PathBuf::from("/home/x/Videos")));

        assert!(state.storage().plan(Disposal::Trash).is_empty());
    }

    #[test]
    fn a_result_from_a_superseded_walk_is_dropped() {
        use limpid_core::analyse::{Breakdown, Entry, Survey};
        use limpid_core::size::Size;

        let named = |name: &str| Survey {
            breakdown: Breakdown {
                children: vec![Entry {
                    path: format!("/a/{name}").into(),
                    name: name.to_owned(),
                    size: Size::new(10, 10),
                    files: 1,
                    is_dir: true,
                }],
                ..Breakdown::default()
            },
            largest: Vec::new(),
        };

        let (mut state, _) = State::boot();
        let _ = state.update(Message::Explore(PathBuf::from("/a")));
        let slow = state.storage().generation;
        let _ = state.update(Message::Explore(PathBuf::from("/a/b")));
        let quick = state.storage().generation;

        // The second walk finishes first, as a smaller directory would.
        let _ = state.update(Message::Explored(quick, Box::new(named("from-b"))));
        // Then the first one lands, describing a directory nobody is on.
        let _ = state.update(Message::Explored(slow, Box::new(named("from-a"))));

        let showing = &state.storage().survey.as_ref().unwrap().breakdown.children[0].name;
        assert_eq!(
            showing, "from-b",
            "the superseded walk overwrote the current one"
        );
    }

    #[test]
    fn a_breadcrumb_click_trims_the_trail_back_to_that_depth() {
        let (mut state, _) = State::boot();
        for path in ["/a", "/a/b", "/a/b/c"] {
            let _ = state.update(Message::Explore(PathBuf::from(path)));
        }
        assert_eq!(state.storage().trail.len(), 3);

        let _ = state.update(Message::Ascend(0));

        assert_eq!(state.storage().trail, vec![PathBuf::from("/a")]);
    }

    #[test]
    fn clicking_the_breadcrumb_you_are_already_on_does_nothing() {
        let (mut state, _) = State::boot();
        let _ = state.update(Message::Explore(PathBuf::from("/a")));

        let _ = state.update(Message::Ascend(0));

        assert_eq!(state.storage().trail, vec![PathBuf::from("/a")]);
    }

    #[test]
    fn a_finished_scan_is_kept() {
        let (mut state, _) = State::boot();

        let _ = state.update(Message::ScanFinished(Box::default()));

        assert!(matches!(state.progress, Progress::Done(_)));
    }
}
