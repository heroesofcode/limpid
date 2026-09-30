//! Application state and the top-level view.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

use iced::widget::{Space, column, container, responsive, row, scrollable, text};
use iced::{Element, Length, Subscription, Task};

use limpid_core::analyse::{self, Survey};
use limpid_core::catalog::{self, Context};
use limpid_core::config::{Config, Exclusions, Store};
use limpid_core::execute::{Executor, Outcome};
use limpid_core::model::{Category, Kind, Scan, Target};
use limpid_core::paths::Roots;
use limpid_core::plan::{Disposal, Magnitude, Plan, Selection};
use limpid_core::privileged::{
    MAXIMUM_DAYS, MAXIMUM_KEEP, MINIMUM_DAYS, MINIMUM_KEEP, Report, Request, Runner,
};
use limpid_core::size::Size;
use limpid_core::volume::{self, Capacity};
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
            Self::Settings => "What Limpid keeps, what it leaves alone, and how it looks",
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

/// What was just excluded, kept so the notice can offer to put it back.
#[derive(Debug, Clone, Default)]
pub struct Excluded {
    /// What the person ticked, in their words rather than as paths.
    pub names: Vec<String>,
    /// What was added to the exclusions. Only what was actually added, so
    /// undoing never removes an exclusion that was there before.
    pub paths: Vec<PathBuf>,
}

impl Excluded {
    /// "Brave cache", "Brave cache and Thumbnails", "Brave cache and 2 more".
    pub fn describe(&self) -> String {
        match self.names.as_slice() {
            [] => "Nothing".to_owned(),
            [one] => one.clone(),
            [one, two] => format!("{one} and {two}"),
            [one, rest @ ..] => format!("{one} and {} more", rest.len()),
        }
    }
}

/// A setting on the settings page that is changed a step at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    /// Versions of each package the pacman cache keeps.
    PackageVersions,
    /// Days of system journal kept.
    JournalDays,
}

/// Which way to move a [`Setting`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Keep less.
    Less,
    /// Keep more.
    More,
}

/// The journal windows the stepper moves between.
///
/// Steps rather than single days: a ten-year range one day at a time is a
/// setting nobody would reach. A value typed into the file between two of
/// these is kept as written, and the next press moves to the neighbour.
pub const JOURNAL_STEPS: [u16; 7] = [7, 14, 30, 60, 90, 180, 365];

impl Setting {
    /// The value this setting would take one step from `config`, or `None`
    /// at the end of the range.
    pub fn step(self, config: &Config, direction: Direction) -> Option<Config> {
        let mut next = config.clone();
        match self {
            Self::PackageVersions => {
                let now = config.policy.keep_package_versions;
                next.policy.keep_package_versions = match direction {
                    Direction::Less if now > MINIMUM_KEEP => now - 1,
                    Direction::More if now < MAXIMUM_KEEP => now + 1,
                    _ => return None,
                };
            }
            Self::JournalDays => {
                let now = config.policy.keep_journal_days;
                let step = match direction {
                    Direction::Less => JOURNAL_STEPS.iter().rev().find(|&&days| days < now),
                    Direction::More => JOURNAL_STEPS.iter().find(|&&days| days > now),
                };
                next.policy.keep_journal_days = step
                    .copied()
                    .filter(|days| (MINIMUM_DAYS..=MAXIMUM_DAYS).contains(days))?;
            }
        }
        Some(next)
    }
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
    /// The disk the files are on, read when the confirmation opened, so a
    /// large deletion can be measured against it.
    pub capacity: Option<Capacity>,
    /// Whether the person has said they read the list of a deletion too
    /// large to go through on the usual click.
    pub checked_large: bool,
    /// Whether a removal is under way.
    pub removing: bool,
    /// What the last removal from this page did.
    pub outcome: Option<Outcome>,
    /// What was just excluded from this page.
    pub excluded: Option<Excluded>,
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

    /// Whether the permanent deletion on offer is too large for the usual
    /// click.
    pub fn magnitude(&self) -> Option<Magnitude> {
        self.plan(Disposal::Delete).magnitude(self.capacity)
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
    /// Whether the person has said they read the list of a clean too large
    /// to go through on the usual click.
    checked_large: bool,
    /// Whether a clean is under way.
    cleaning: bool,
    /// What the last clean did.
    outcome: Option<Cleaned>,
    /// The storage page.
    storage: Storage,
    /// Where to look. Fixed at start-up, so every part of the window agrees
    /// about which home directory and which config file it means.
    roots: Roots,
    /// The config file, as last read or written.
    config: Store,
    /// Why the last change to the config file did not stick.
    config_error: Option<String>,
    /// What was just excluded from the overview.
    excluded: Option<Excluded>,
    /// The path being typed on the settings page.
    draft: String,
    /// Which scan the next result must belong to.
    ///
    /// Changing an exclusion rescans in the background, and two changes in
    /// quick succession start two scans that can finish in either order.
    /// Without this the older one could land last and put back something
    /// that has since been excluded.
    scans: u64,
}

/// Everything that can happen.
#[derive(Debug, Clone)]
pub enum Message {
    /// A sidebar entry was chosen.
    Navigate(Page),
    /// The scan button was pressed.
    StartScan,
    /// A scan finished. Dropped unless it is the latest one started.
    ScanFinished(u64, Box<Scan>),
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
    /// Nothing happened worth reacting to.
    Nothing,
    /// Show a file in the desktop's file manager.
    Reveal(PathBuf),
    /// Put a file's path on the clipboard.
    CopyPath(PathBuf),
    /// Never offer the ticked findings again.
    ExcludeSelected,
    /// Never offer or remove the ticked files on the storage page.
    ExcludeFiles,
    /// Take these back out of the exclusions.
    Include(Vec<PathBuf>),
    /// The "I have read the list" box on a large clean.
    CheckLarge(bool),
    /// The same box on a large permanent deletion.
    CheckLargeDelete(bool),
    /// The path being typed on the settings page changed.
    DraftChanged(String),
    /// Exclude the path typed on the settings page.
    AddDraft,
    /// Move a setting one step.
    Adjust(Setting, Direction),
}

impl State {
    /// Build the initial state and kick off the first scan.
    ///
    /// Scanning immediately is the right default: the question the user
    /// opened the window to ask is always the same one.
    pub fn boot() -> (Self, Task<Message>) {
        Self::boot_in(Roots::from_env())
    }

    /// Start up looking at these roots. The tests use this, so that nothing
    /// they do can read or write the developer's own config file.
    fn boot_in(roots: Roots) -> (Self, Task<Message>) {
        let state = Self {
            theme: LimpidTheme::detect(),
            page: Page::Overview,
            progress: Progress::Idle,
            selected: BTreeSet::new(),
            confirming: false,
            checked_large: false,
            cleaning: false,
            outcome: None,
            storage: Storage::default(),
            config: Store::open(&roots),
            roots,
            config_error: None,
            excluded: None,
            draft: String::new(),
            scans: 0,
        };
        (state, Task::done(Message::StartScan))
    }

    /// The colours in force.
    pub fn palette(&self) -> Palette {
        self.theme.palette
    }

    /// The colours in force, and where they came from.
    pub fn theme(&self) -> &LimpidTheme {
        &self.theme
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

    /// The config file, as last read or written.
    pub fn config(&self) -> &Store {
        &self.config
    }

    /// Why the last change to the config file did not stick.
    pub fn config_error(&self) -> Option<&str> {
        self.config_error.as_deref()
    }

    /// What was just excluded from the overview.
    pub fn excluded(&self) -> Option<&Excluded> {
        self.excluded.as_ref()
    }

    /// The path being typed on the settings page.
    pub fn draft(&self) -> &str {
        &self.draft
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

    /// Whether the clean on offer is too large for the usual click.
    pub fn magnitude(&self) -> Option<Magnitude> {
        self.plan().magnitude(self.scan()?.capacity)
    }

    /// Whether the person has said they read the list of a large clean.
    pub fn checked_large(&self) -> bool {
        self.checked_large
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
        self.storage.excluded = None;
        let roots = self.roots.clone();

        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    Executor::applying(&roots)
                        .with_exclusions(exclusions(&roots))
                        .run(&plan)
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
                // Notices belong to the page they were raised on and the
                // moment they were raised; carried to another page they
                // would describe something no longer on screen.
                self.excluded = None;
                self.storage.excluded = None;
                self.config_error = None;
                // Read again, so an edit made by hand while the window was
                // open is what the page shows.
                if page == Page::Settings {
                    self.config = Store::open(&self.roots);
                }
                // Measuring a whole home directory takes seconds, so it
                // happens when the page is first opened rather than at start
                // up, and only once.
                if page == Page::Storage && self.storage.trail.is_empty() {
                    return Task::done(Message::Explore(self.roots.home.clone()));
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
                self.storage.excluded = None;
                self.storage.working = true;
                self.storage.survey = None;
                self.storage.generation += 1;
                let generation = self.storage.generation;
                let roots = self.roots.clone();
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            let options = WalkOptions {
                                skip: exclusions(&roots),
                                ..WalkOptions::default()
                            };
                            analyse::survey(&path, LARGEST_FILES, &options).unwrap_or_default()
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
                // Asked every time it opens: a tick given for one list is
                // not a tick for the next.
                self.storage.checked_large = false;
                self.storage.capacity = volume::capacity(&self.roots.home);
                Task::none()
            }
            Message::CancelDelete => {
                self.storage.confirming_delete = false;
                self.storage.checked_large = false;
                Task::none()
            }
            Message::CheckLargeDelete(checked) => {
                self.storage.checked_large = checked;
                Task::none()
            }
            Message::TrashSelected => self.remove_selection(Disposal::Trash),
            Message::DeleteSelected => {
                // Refused here and not only by a greyed-out button: the
                // button is how it is usually asked for, not the only way.
                if self.storage.magnitude().is_some() && !self.storage.checked_large {
                    return Task::none();
                }
                self.storage.checked_large = false;
                self.remove_selection(Disposal::Delete)
            }
            Message::Reveal(path) => {
                // Off the frame loop: talking to a file manager that has to
                // be started cold is not instant, and the answer is not
                // needed for anything.
                Task::perform(
                    async move {
                        let _ = tokio::task::spawn_blocking(move || {
                            crate::reveal::in_file_manager(&path);
                        })
                        .await;
                    },
                    |()| Message::Nothing,
                )
            }
            Message::CopyPath(path) => iced::clipboard::write(path.display().to_string()),
            Message::Nothing => Task::none(),
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
                self.excluded = None;
                self.start_scan()
            }
            Message::ScanFinished(generation, scan) => {
                if generation != self.scans {
                    return Task::none();
                }
                self.cleaning = false;
                // A rescan behind a scan already on screen keeps the ticks;
                // a scan the person asked for starts from the safe set, as
                // the first one did.
                match &self.progress {
                    Progress::Done(old) => {
                        self.selected = carry(&self.selected, old, &scan);
                        self.progress = Progress::Done(scan);
                    }
                    _ => {
                        self.progress = Progress::Done(scan);
                        self.select_safe();
                    }
                }
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
                self.checked_large = false;
                Task::none()
            }
            Message::Cancel => {
                self.confirming = false;
                self.checked_large = false;
                Task::none()
            }
            Message::CheckLarge(checked) => {
                self.checked_large = checked;
                Task::none()
            }
            Message::Clean => {
                let plan = self.plan();
                if plan.is_empty() {
                    self.confirming = false;
                    return Task::none();
                }
                // Refused here and not only by a greyed-out button, for the
                // same reason the executor re-checks what the scan checked.
                if self.magnitude().is_some() && !self.checked_large {
                    return Task::none();
                }
                self.confirming = false;
                self.checked_large = false;
                self.cleaning = true;
                self.outcome = None;
                self.excluded = None;
                let roots = self.roots.clone();
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            let outcome = Executor::applying(&roots)
                                .with_exclusions(exclusions(&roots))
                                .run(&plan);

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
            Message::ExcludeSelected => {
                let chosen: Vec<(String, Vec<PathBuf>)> = self
                    .chosen()
                    .into_iter()
                    .map(|target| (target.name.clone(), target.paths.clone()))
                    .collect();
                let (names, paths): (Vec<_>, Vec<_>) = chosen.into_iter().unzip();
                let (added, task) = self.exclude(paths.into_iter().flatten().collect());
                self.excluded = (!added.is_empty()).then_some(Excluded {
                    names,
                    paths: added,
                });
                task
            }
            Message::ExcludeFiles => {
                let paths: Vec<PathBuf> = self
                    .storage
                    .chosen()
                    .into_iter()
                    .map(|(path, _)| path)
                    .collect();
                let names = paths
                    .iter()
                    .map(|path| {
                        path.file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.display().to_string())
                    })
                    .collect();
                let (added, task) = self.exclude(paths);
                self.storage.excluded = (!added.is_empty()).then_some(Excluded {
                    names,
                    paths: added,
                });
                task
            }
            Message::Include(paths) => {
                let changed = self.change_config(|config| {
                    // Not short-circuiting: every one of them has to go.
                    paths
                        .iter()
                        .fold(false, |any, path| config.exclusions.remove(path) | any)
                });
                // The offer to undo has been answered, whichever page it
                // was made on.
                let answered = |notice: &Option<Excluded>| {
                    notice
                        .as_ref()
                        .is_some_and(|notice| notice.paths.iter().any(|p| paths.contains(p)))
                };
                if answered(&self.excluded) {
                    self.excluded = None;
                }
                if answered(&self.storage.excluded) {
                    self.storage.excluded = None;
                }
                if changed {
                    self.exclusions_changed()
                } else {
                    Task::none()
                }
            }
            Message::DraftChanged(draft) => {
                self.draft = draft;
                Task::none()
            }
            Message::AddDraft => {
                let typed = self.draft.trim().to_owned();
                if typed.is_empty() {
                    return Task::none();
                }
                let Some(path) = self.config.expand(&typed) else {
                    self.config_error = Some(format!(
                        "\u{201c}{typed}\u{201d} is not a full path. Start it with / or ~/, \
                         so it means the same thing wherever Limpid is started from."
                    ));
                    return Task::none();
                };
                if self.config.config.exclusions.covers(&path) {
                    self.config_error = Some(format!(
                        "{} is already excluded.",
                        self.config.display(&path)
                    ));
                    return Task::none();
                }
                let (added, task) = self.exclude(vec![path]);
                if !added.is_empty() {
                    self.draft.clear();
                }
                task
            }
            Message::Adjust(setting, direction) => {
                let changed = self.change_config(|config| match setting.step(config, direction) {
                    Some(next) => {
                        *config = next;
                        true
                    }
                    None => false,
                });
                // The figures do not change, but what the overview says it
                // will do does: "keeps the newest 3" has to become 2.
                if changed { self.rescan() } else { Task::none() }
            }
        }
    }

    /// Start a scan, tagged so that only the latest one lands.
    ///
    /// The walk is IO-bound and long enough to be felt, so it goes to a
    /// blocking thread rather than stalling the frame loop.
    fn start_scan(&mut self) -> Task<Message> {
        self.scans += 1;
        let generation = self.scans;
        let roots = self.roots.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let config = Store::open(&roots).config;
                    catalog::scan(&Context::with_config(roots, config))
                })
                .await
                .unwrap_or_default()
            },
            move |scan| Message::ScanFinished(generation, Box::new(scan)),
        )
    }

    /// Scan again behind what is on screen, keeping the ticks.
    ///
    /// Behind rather than instead of: the results stay up while the new ones
    /// are measured, so an exclusion does not blank the page. Nothing to do
    /// before the first scan, which will read the new settings anyway.
    fn rescan(&mut self) -> Task<Message> {
        match self.progress {
            Progress::Idle => Task::none(),
            // Superseded rather than left to finish: it read the settings
            // before they changed.
            Progress::Running | Progress::Done(_) => self.start_scan(),
        }
    }

    /// Read the file as it is now, change it, and write it back.
    ///
    /// Read fresh rather than from the copy held since start-up, so an edit
    /// made by hand while the window is open is built on rather than
    /// overwritten. Returns whether anything was saved.
    fn change_config(&mut self, change: impl FnOnce(&mut Config) -> bool) -> bool {
        let mut store = Store::open(&self.roots);
        if !change(&mut store.config) {
            self.config = store;
            return false;
        }
        match store.save() {
            Ok(()) => {
                self.config = store;
                self.config_error = None;
                true
            }
            Err(error) => {
                // What is on disk, not what failed to get there: showing the
                // change as made would be showing a setting nothing honours.
                self.config = Store::open(&self.roots);
                self.config_error = Some(format!("The change was not saved: {error}"));
                false
            }
        }
    }

    /// Add these to the exclusions, and bring what is on screen into line.
    ///
    /// Returns what was actually added. A path already covered adds
    /// nothing, and undoing must not take away an exclusion that was there
    /// before.
    fn exclude(&mut self, paths: Vec<PathBuf>) -> (Vec<PathBuf>, Task<Message>) {
        let mut added = Vec::new();
        let saved = self.change_config(|config| {
            for path in paths {
                if !config.exclusions.covers(&path) && config.exclusions.add(path.clone()) {
                    added.push(path);
                }
            }
            !added.is_empty()
        });
        if !saved {
            return (Vec::new(), Task::none());
        }
        (added, self.exclusions_changed())
    }

    /// Make everything on screen agree with the exclusions as they now are.
    fn exclusions_changed(&mut self) -> Task<Message> {
        let exclusions = self.config.config.exclusions.clone();

        // The storage page is corrected in place. Its lists stay as they
        // were measured, with what is now excluded marked and unticked, so
        // an undo puts things back exactly; the next walk leaves excluded
        // files out of the largest list.
        self.storage
            .selected
            .retain(|path| !exclusions.covers(path));
        if self.storage.plan(Disposal::Delete).is_empty() {
            self.storage.confirming_delete = false;
            self.storage.checked_large = false;
        }

        // Unticked now rather than when the rescan lands: in between, the
        // action bar would count what was just excluded and Clean would ask
        // about it. A finding only partly excluded keeps its tick; the
        // rescan will say how much of it is left.
        if let Some(scan) = self.scan() {
            let excluded = |&(c, t): &TargetId| {
                scan.categories
                    .get(c)
                    .and_then(|category| category.targets.get(t))
                    .is_some_and(|target| {
                        !target.paths.is_empty()
                            && target.paths.iter().all(|path| exclusions.covers(path))
                    })
            };
            let kept = self.selected.iter().filter(|id| !excluded(id)).copied();
            self.selected = kept.collect();
        }

        // A confirmation listing what is about to be removed must not
        // change while it is being read.
        self.confirming = false;
        self.checked_large = false;

        self.rescan()
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
                Page::Storage => view::storage::view(palette, metrics, self),
                Page::Settings => view::settings::view(palette, metrics, self),
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

/// The ticks on one scan, moved onto the next.
///
/// Matched by category and target name, and only where that pair names
/// exactly one target in both scans: a tick that could belong to either of
/// two rows lands on neither. Indices cannot be carried, because excluding
/// one finding moves every one after it.
fn carry(selected: &BTreeSet<TargetId>, old: &Scan, new: &Scan) -> BTreeSet<TargetId> {
    fn index(scan: &Scan) -> BTreeMap<(&str, &str), Vec<TargetId>> {
        let mut index: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for (c, category) in scan.categories.iter().enumerate() {
            for (t, target) in category.targets.iter().enumerate() {
                index
                    .entry((category.name.as_str(), target.name.as_str()))
                    .or_default()
                    .push((c, t));
            }
        }
        index
    }

    let key = |scan: &'_ Scan, (c, t): TargetId| -> Option<(String, String)> {
        let category: &Category = scan.categories.get(c)?;
        Some((category.name.clone(), category.targets.get(t)?.name.clone()))
    };

    let before = index(old);
    let after = index(new);

    selected
        .iter()
        .filter_map(|&id| key(old, id))
        .filter(|(c, t)| {
            before
                .get(&(c.as_str(), t.as_str()))
                .is_some_and(|ids| ids.len() == 1)
        })
        .filter_map(
            |(c, t)| match after.get(&(c.as_str(), t.as_str()))?.as_slice() {
                [id] => Some(*id),
                _ => None,
            },
        )
        // What can no longer be ticked — now in use, or now empty — loses
        // its tick rather than carrying one the view would not draw.
        .filter(|&(c, t)| selectable(&new.categories[c].targets[t]))
        .collect()
}

/// Whether a finding can be ticked.
///
/// A privileged target is selectable when the helper knows an operation for
/// it; one that needs root and names no operation cannot be cleaned by
/// anything, so offering a tick would be a lie.
pub fn selectable(target: &Target) -> bool {
    !target.size.is_zero()
        && target.blocked.is_none()
        && target.kind != Kind::Attention
        && (target.is_actionable() || target.privileged.is_some())
}

/// What the person asked never to be offered or removed, read from the file
/// as it is now.
///
/// Read at the moment of acting rather than once at start-up, so an
/// exclusion added by hand while the window is open is honoured by the next
/// removal — the same reason `Guard` checks the filesystem as it is then.
fn exclusions(roots: &Roots) -> Exclusions {
    Store::open(roots).config.exclusions
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window pointed at an empty fixture, so nothing a test does can
    /// read or write the developer's own config file. The directory lives
    /// as long as the returned guard.
    fn boot() -> (State, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (state, _) = State::boot_in(Roots::under(dir.path()));
        (state, dir)
    }

    #[test]
    fn every_page_has_a_title_and_a_subtitle() {
        for page in Page::ALL {
            assert!(!page.title().is_empty());
            assert!(!page.subtitle().is_empty());
        }
    }

    #[test]
    fn navigating_changes_the_page_without_disturbing_the_scan() {
        let (mut state, _dir) = boot();
        state.progress = Progress::Running;

        let _ = state.update(Message::Navigate(Page::Settings));

        assert_eq!(state.page, Page::Settings);
        assert!(matches!(state.progress, Progress::Running));
    }

    #[test]
    fn a_second_scan_request_while_one_is_running_is_ignored() {
        let (mut state, _dir) = boot();
        state.progress = Progress::Running;

        let task = state.update(Message::StartScan);

        // Task has no public emptiness predicate, so the observable effect is
        // that the state is untouched.
        assert!(matches!(state.progress, Progress::Running));
        drop(task);
    }

    #[test]
    fn a_theme_change_repaints_with_the_new_palette() {
        let (mut state, _dir) = boot();
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

    fn state_with_scan() -> (State, tempfile::TempDir) {
        let (mut state, dir) = boot();
        let _ = state.update(Message::ScanFinished(state.scans, Box::new(sample_scan())));
        (state, dir)
    }

    #[test]
    fn a_scan_arrives_with_only_the_uncontroversial_items_ticked() {
        let (state, _dir) = state_with_scan();

        assert!(state.is_selected((0, 0)), "the safe one should be ticked");
        assert!(!state.is_selected((0, 1)), "review needs a decision");
        assert!(!state.is_selected((0, 2)), "this process cannot act on it");
        assert!(!state.is_selected((0, 3)), "nothing to reclaim");
        assert!(!state.is_selected((0, 4)), "not reclaimable space at all");
    }

    #[test]
    fn ticking_is_a_toggle() {
        let (mut state, _dir) = state_with_scan();

        let _ = state.update(Message::Toggle((0, 1)));
        assert!(state.is_selected((0, 1)));

        let _ = state.update(Message::Toggle((0, 1)));
        assert!(!state.is_selected((0, 1)));
    }

    #[test]
    fn the_plan_follows_the_selection() {
        let (mut state, _dir) = state_with_scan();
        assert_eq!(state.plan().items.len(), 1);

        let _ = state.update(Message::Toggle((0, 1)));
        assert_eq!(state.plan().items.len(), 2);
        assert_eq!(state.plan().expected().on_disk, 200);

        let _ = state.update(Message::SelectNone);
        assert!(state.plan().is_empty());
    }

    #[test]
    fn a_selection_that_would_do_nothing_does_not_raise_a_confirmation() {
        let (mut state, _dir) = state_with_scan();
        let _ = state.update(Message::SelectNone);

        let _ = state.update(Message::AskToClean);

        assert!(!state.is_confirming());
    }

    #[test]
    fn confirming_can_be_backed_out_of() {
        let (mut state, _dir) = state_with_scan();

        let _ = state.update(Message::AskToClean);
        assert!(state.is_confirming());

        let _ = state.update(Message::Cancel);
        assert!(!state.is_confirming());
        assert!(!state.is_cleaning());
    }

    #[test]
    fn accepting_the_confirmation_closes_it_and_starts_work() {
        let (mut state, _dir) = state_with_scan();
        let _ = state.update(Message::AskToClean);

        let task = state.update(Message::Clean);

        assert!(!state.is_confirming());
        assert!(state.is_cleaning());
        drop(task);
    }

    #[test]
    fn accepting_with_nothing_chosen_starts_nothing() {
        let (mut state, _dir) = state_with_scan();
        let _ = state.update(Message::SelectNone);

        let task = state.update(Message::Clean);

        assert!(!state.is_cleaning());
        drop(task);
    }

    #[test]
    fn selecting_safe_again_restores_the_starting_point() {
        let (mut state, _dir) = state_with_scan();
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

        let (mut state, _dir) = boot();
        let _ = state.update(Message::ScanFinished(state.scans, Box::new(scan)));

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
        let (mut state, _dir) = state_with_scan();

        let _ = state.update(Message::Cleaned(Box::default()));

        assert!(state.outcome().is_some());
        // The figure shown afterwards is measured again rather than assumed.
        let _ = state.update(Message::ScanFinished(state.scans, Box::new(sample_scan())));
        assert!(!state.is_cleaning());
    }

    #[test]
    fn opening_the_storage_page_starts_a_measurement_once() {
        let (mut state, _dir) = boot();

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

        let (mut state, _dir) = boot();
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

    fn state_on_storage() -> (State, tempfile::TempDir) {
        let (mut state, dir) = boot();
        let _ = state.update(Message::Explore(PathBuf::from("/home/x")));
        let generation = state.storage().generation;
        let _ = state.update(Message::Explored(
            generation,
            Box::new(survey_with_a_file()),
        ));
        (state, dir)
    }

    #[test]
    fn ticking_a_file_on_the_storage_page_is_a_toggle() {
        let (mut state, _dir) = state_on_storage();
        let file = PathBuf::from("/home/x/big.iso");

        let _ = state.update(Message::ToggleFile(file.clone()));
        assert!(state.storage().is_selected(&file));

        let _ = state.update(Message::ToggleFile(file.clone()));
        assert!(!state.storage().is_selected(&file));
    }

    #[test]
    fn a_selection_becomes_a_chosen_plan_that_goes_to_the_trash() {
        use limpid_core::guard::Permission;

        let (mut state, _dir) = state_on_storage();
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
        let (mut state, _dir) = state_on_storage();
        let _ = state.update(Message::ToggleFile(PathBuf::from("/home/x/big.iso")));
        assert!(!state.storage().plan(Disposal::Trash).is_empty());

        let _ = state.update(Message::Explore(PathBuf::from("/home/x/Videos")));

        assert!(state.storage().selected.is_empty());
        assert!(state.storage().plan(Disposal::Trash).is_empty());
    }

    #[test]
    fn a_selection_that_would_delete_nothing_raises_no_confirmation() {
        let (mut state, _dir) = state_on_storage();

        let _ = state.update(Message::AskToDelete);

        assert!(!state.storage().confirming_delete);
    }

    #[test]
    fn the_permanent_deletion_confirmation_can_be_backed_out_of() {
        let (mut state, _dir) = state_on_storage();
        let _ = state.update(Message::ToggleFile(PathBuf::from("/home/x/big.iso")));

        let _ = state.update(Message::AskToDelete);
        assert!(state.storage().confirming_delete);

        let _ = state.update(Message::CancelDelete);
        assert!(!state.storage().confirming_delete);
        assert!(!state.storage().removing);
    }

    #[test]
    fn trashing_acts_directly_because_it_is_reversible() {
        let (mut state, _dir) = state_on_storage();
        let _ = state.update(Message::ToggleFile(PathBuf::from("/home/x/big.iso")));

        let task = state.update(Message::TrashSelected);

        assert!(state.storage().removing);
        assert!(!state.storage().confirming_delete);
        drop(task);
    }

    #[test]
    fn a_finished_removal_clears_the_selection_and_measures_again() {
        let (mut state, _dir) = state_on_storage();
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
        let (mut state, _dir) = state_on_storage();
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

        let (mut state, _dir) = boot();
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
        let (mut state, _dir) = boot();
        for path in ["/a", "/a/b", "/a/b/c"] {
            let _ = state.update(Message::Explore(PathBuf::from(path)));
        }
        assert_eq!(state.storage().trail.len(), 3);

        let _ = state.update(Message::Ascend(0));

        assert_eq!(state.storage().trail, vec![PathBuf::from("/a")]);
    }

    #[test]
    fn clicking_the_breadcrumb_you_are_already_on_does_nothing() {
        let (mut state, _dir) = boot();
        let _ = state.update(Message::Explore(PathBuf::from("/a")));

        let _ = state.update(Message::Ascend(0));

        assert_eq!(state.storage().trail, vec![PathBuf::from("/a")]);
    }

    #[test]
    fn a_finished_scan_is_kept() {
        let (mut state, _dir) = boot();

        let _ = state.update(Message::ScanFinished(state.scans, Box::default()));

        assert!(matches!(state.progress, Progress::Done(_)));
    }

    /// A scan whose findings have real paths inside the fixture.
    fn scan_with_paths(roots: &Roots) -> Scan {
        use limpid_core::model::{Category, Kind, Risk};
        use limpid_core::size::Size;

        let at = |name: &str, path: &str| {
            Target::new(name, Kind::Cache, Risk::Safe)
                .path(roots.home.join(path))
                .measured(Size::new(100, 100), 1)
        };

        let mut category = Category::new("Caches", "");
        category.targets = vec![
            at("Thumbnails", ".cache/thumbnails"),
            at("Fonts", ".cache/fontconfig"),
            at("Pip", ".cache/pip"),
        ];

        Scan {
            categories: vec![category],
            ..Scan::default()
        }
    }

    fn on_disk(roots: &Roots) -> Store {
        Store::open(roots)
    }

    #[test]
    fn excluding_the_ticked_findings_writes_them_to_the_file_and_offers_an_undo() {
        let (mut state, _dir) = boot();
        let roots = state.roots.clone();
        let _ = state.update(Message::ScanFinished(
            state.scans,
            Box::new(scan_with_paths(&roots)),
        ));
        let _ = state.update(Message::SelectNone);
        let _ = state.update(Message::Toggle((0, 0)));

        let _ = state.update(Message::ExcludeSelected);

        let thumbnails = roots.home.join(".cache/thumbnails");
        assert!(on_disk(&roots).config.exclusions.covers(&thumbnails));
        let excluded = state.excluded().expect("a notice");
        assert_eq!(excluded.names, ["Thumbnails"]);
        assert_eq!(excluded.paths, [thumbnails]);
        // Unticked at once, not when the rescan lands: in between, the bar
        // would count it and Clean would ask about it.
        assert!(!state.is_selected((0, 0)));
        assert!(state.plan().is_empty());
    }

    #[test]
    fn undoing_takes_back_only_what_the_exclusion_added() {
        let (mut state, _dir) = boot();
        let roots = state.roots.clone();
        let cache = roots.home.join(".cache");
        let mut store = on_disk(&roots);
        store
            .config
            .exclusions
            .add(roots.home.join(".cache/fontconfig"));
        store.save().unwrap();

        let _ = state.update(Message::ScanFinished(
            state.scans,
            Box::new(scan_with_paths(&roots)),
        ));
        let _ = state.update(Message::SelectNone);
        let _ = state.update(Message::Toggle((0, 0)));
        let _ = state.update(Message::Toggle((0, 1)));
        let _ = state.update(Message::ExcludeSelected);

        // Fonts was already excluded, so only Thumbnails was added.
        let added = state.excluded().expect("a notice").paths.clone();
        assert_eq!(added, [cache.join("thumbnails")]);

        let _ = state.update(Message::Include(added));

        let exclusions = on_disk(&roots).config.exclusions;
        assert!(!exclusions.covers(&cache.join("thumbnails")));
        assert!(
            exclusions.covers(&cache.join("fontconfig")),
            "an undo must not remove an exclusion that was there before"
        );
        assert!(state.excluded().is_none(), "the offer has been answered");
    }

    #[test]
    fn a_rescan_keeps_the_ticks_even_when_the_rows_move() {
        let (state, _dir) = boot();
        let old = scan_with_paths(&state.roots);
        let mut new = old.clone();
        // Excluding the first finding moves the other two up one.
        new.categories[0].targets.remove(0);

        let ticked = BTreeSet::from([(0, 0), (0, 2)]);
        let carried = carry(&ticked, &old, &new);

        // Thumbnails is gone; Pip was at 2 and is now at 1.
        assert_eq!(carried, BTreeSet::from([(0, 1)]));
    }

    #[test]
    fn a_tick_that_could_belong_to_two_rows_lands_on_neither() {
        let (state, _dir) = boot();
        let old = scan_with_paths(&state.roots);
        let mut new = old.clone();
        new.categories[0].targets[1].name = "Pip".to_owned();

        let carried = carry(&BTreeSet::from([(0, 2)]), &old, &new);

        assert!(carried.is_empty());
    }

    #[test]
    fn a_tick_is_not_carried_onto_something_that_is_now_in_use() {
        let (state, _dir) = boot();
        let old = scan_with_paths(&state.roots);
        let mut new = old.clone();
        new.categories[0].targets[0].blocked = Some("Close it first".to_owned());

        let carried = carry(&BTreeSet::from([(0, 0)]), &old, &new);

        assert!(carried.is_empty());
    }

    #[test]
    fn a_rescan_behind_the_results_keeps_the_ticks_but_a_fresh_scan_does_not() {
        let (mut state, _dir) = boot();
        let roots = state.roots.clone();
        let _ = state.update(Message::ScanFinished(
            state.scans,
            Box::new(scan_with_paths(&roots)),
        ));
        let _ = state.update(Message::SelectNone);
        let _ = state.update(Message::Toggle((0, 2)));

        // Behind the results: the scan on screen stays up, the tick stays.
        let _ = state.rescan();
        let _ = state.update(Message::ScanFinished(
            state.scans,
            Box::new(scan_with_paths(&roots)),
        ));
        assert_eq!(state.selected, BTreeSet::from([(0, 2)]));

        // Asked for: starts again from the safe set.
        let _ = state.update(Message::StartScan);
        let _ = state.update(Message::ScanFinished(
            state.scans,
            Box::new(scan_with_paths(&roots)),
        ));
        assert_eq!(state.selected, BTreeSet::from([(0, 0), (0, 1), (0, 2)]));
    }

    #[test]
    fn a_scan_that_read_the_settings_before_they_changed_is_dropped() {
        let (mut state, _dir) = boot();
        let roots = state.roots.clone();
        let _ = state.update(Message::ScanFinished(
            state.scans,
            Box::new(scan_with_paths(&roots)),
        ));

        let stale = state.scans;
        let _ = state.rescan();
        let _ = state.update(Message::ScanFinished(stale, Box::default()));

        let Progress::Done(scan) = &state.progress else {
            panic!("the scan on screen should stay");
        };
        assert_eq!(scan.categories[0].targets.len(), 3);
    }

    #[test]
    fn excluding_from_the_storage_page_unticks_the_files_and_marks_them() {
        let (mut state, _dir) = state_on_storage();
        let file = PathBuf::from("/home/x/big.iso");
        let _ = state.update(Message::ToggleFile(file.clone()));

        let _ = state.update(Message::ExcludeFiles);

        assert!(state.storage.selected.is_empty());
        assert!(on_disk(&state.roots).config.exclusions.covers(&file));
        assert_eq!(
            state.storage.excluded.as_ref().expect("a notice").names,
            ["big.iso"]
        );
        // Still listed, so an undo puts it back exactly where it was.
        let survey = state.storage.survey.as_ref().unwrap();
        assert!(survey.largest.iter().any(|entry| entry.path == file));
        // And no longer something a plan can be made from.
        assert!(state.storage.plan(Disposal::Trash).is_empty());
    }

    #[test]
    fn a_setting_moves_a_step_at_a_time_and_stops_at_its_ends() {
        let mut config = Config::default();
        let versions = |config: &Config, direction| {
            Setting::PackageVersions
                .step(config, direction)
                .map(|next| next.policy.keep_package_versions)
        };
        let days = |config: &Config, direction| {
            Setting::JournalDays
                .step(config, direction)
                .map(|next| next.policy.keep_journal_days)
        };

        assert_eq!(versions(&config, Direction::Less), Some(2));
        config.policy.keep_package_versions = MINIMUM_KEEP;
        assert_eq!(versions(&config, Direction::Less), None);
        config.policy.keep_package_versions = MAXIMUM_KEEP;
        assert_eq!(versions(&config, Direction::More), None);

        assert_eq!(days(&config, Direction::More), Some(30));
        // A value typed into the file between two steps moves to its
        // neighbours rather than jumping to an end.
        config.policy.keep_journal_days = 20;
        assert_eq!(days(&config, Direction::Less), Some(14));
        assert_eq!(days(&config, Direction::More), Some(30));
        config.policy.keep_journal_days = MINIMUM_DAYS;
        assert_eq!(days(&config, Direction::Less), None);
        config.policy.keep_journal_days = 400;
        assert_eq!(days(&config, Direction::More), None);
        assert_eq!(days(&config, Direction::Less), Some(365));
    }

    #[test]
    fn adjusting_a_setting_builds_on_an_edit_made_by_hand_while_the_window_was_open() {
        let (mut state, _dir) = boot();
        let path = state.config().path().to_owned();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "version = 1\n\n[policy]\n# mine\nkeep_journal_days = 30\n",
        )
        .unwrap();

        let _ = state.update(Message::Adjust(Setting::PackageVersions, Direction::Less));

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("# mine"), "{written}");
        assert!(written.contains("keep_journal_days = 30"), "{written}");
        assert_eq!(on_disk(&state.roots).config.policy.keep_package_versions, 2);
    }

    #[test]
    fn a_file_that_cannot_be_read_is_never_changed_from_the_window() {
        let (mut state, _dir) = boot();
        let path = state.config().path().to_owned();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "this is [not toml").unwrap();

        let _ = state.update(Message::Adjust(Setting::PackageVersions, Direction::Less));

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "this is [not toml");
        assert!(state.config_error().is_some());
        assert!(!state.config().is_writable());
    }

    #[test]
    fn a_typed_path_is_excluded_and_the_field_cleared() {
        let (mut state, _dir) = boot();

        let _ = state.update(Message::DraftChanged("  ~/Projects/keep  ".to_owned()));
        let _ = state.update(Message::AddDraft);

        let keep = state.roots.home.join("Projects/keep");
        assert!(on_disk(&state.roots).config.exclusions.covers(&keep));
        assert!(state.draft().is_empty());
        assert!(state.config_error().is_none());
    }

    #[test]
    fn a_typed_path_that_is_not_a_full_path_is_refused_with_a_reason() {
        let (mut state, _dir) = boot();

        let _ = state.update(Message::DraftChanged("Projects/keep".to_owned()));
        let _ = state.update(Message::AddDraft);

        assert!(on_disk(&state.roots).config.exclusions.is_empty());
        assert!(
            state
                .config_error()
                .is_some_and(|why| why.contains("full path"))
        );
        // Kept, so the person can fix it rather than type it again.
        assert_eq!(state.draft(), "Projects/keep");
    }

    #[test]
    fn opening_the_settings_page_reads_the_file_again() {
        let (mut state, _dir) = boot();
        let path = state.config().path().to_owned();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[policy]\nkeep_package_versions = 5\n").unwrap();

        let _ = state.update(Message::Navigate(Page::Settings));

        assert_eq!(state.config().config.policy.keep_package_versions, 5);
    }

    const GIB: u64 = 1 << 30;

    /// A disk with this much in use, and as much again free.
    fn disk(used: u64) -> Capacity {
        Capacity {
            total: used * 2,
            available: used,
        }
    }

    /// A scan whose one finding is most of a small disk.
    fn state_with_a_large_scan() -> (State, tempfile::TempDir) {
        use limpid_core::model::{Category, Kind, Risk};

        let (mut state, dir) = boot();
        let mut category = Category::new("Projects", "");
        category.targets = vec![
            Target::new("~/Work/old/target", Kind::BuildArtifact, Risk::Safe)
                .path(state.roots.home.join("Work/old/target"))
                .measured(Size::new(3 * GIB, 3 * GIB), 1),
        ];
        let scan = Scan {
            categories: vec![category],
            capacity: Some(disk(4 * GIB)),
            ..Scan::default()
        };
        let _ = state.update(Message::ScanFinished(state.scans, Box::new(scan)));
        (state, dir)
    }

    #[test]
    fn a_clean_that_is_most_of_the_disk_waits_for_the_list_to_be_read() {
        let (mut state, _dir) = state_with_a_large_scan();
        let _ = state.update(Message::AskToClean);
        assert!(state.magnitude().is_some());

        // The button is greyed out, and the message is refused as well.
        let _ = state.update(Message::Clean);
        assert!(state.is_confirming(), "still asking");
        assert!(!state.is_cleaning());

        let _ = state.update(Message::CheckLarge(true));
        let _ = state.update(Message::Clean);
        assert!(state.is_cleaning());
    }

    #[test]
    fn saying_the_list_was_read_counts_for_one_list_only() {
        let (mut state, _dir) = state_with_a_large_scan();
        let _ = state.update(Message::AskToClean);
        let _ = state.update(Message::CheckLarge(true));
        let _ = state.update(Message::Cancel);

        let _ = state.update(Message::AskToClean);

        assert!(!state.checked_large());
    }

    #[test]
    fn an_ordinary_clean_needs_no_extra_step() {
        let (mut state, _dir) = state_with_scan();
        let _ = state.update(Message::AskToClean);

        assert!(state.magnitude().is_none());
        let _ = state.update(Message::Clean);
        assert!(state.is_cleaning());
    }

    #[test]
    fn a_large_permanent_deletion_waits_for_the_list_to_be_read() {
        let (mut state, _dir) = state_on_storage();
        let file = PathBuf::from("/home/x/big.iso");
        if let Some(survey) = &mut state.storage.survey {
            for entry in survey
                .breakdown
                .children
                .iter_mut()
                .chain(&mut survey.largest)
            {
                if entry.path == file {
                    entry.size = Size::new(3 * GIB, 3 * GIB);
                }
            }
        }
        let _ = state.update(Message::ToggleFile(file));
        let _ = state.update(Message::AskToDelete);
        // Read from the machine the test runs on, which says nothing about
        // this; set to a disk the file is most of.
        state.storage.capacity = Some(disk(4 * GIB));

        let _ = state.update(Message::DeleteSelected);
        assert!(!state.storage.removing, "refused until the list is read");
        assert!(state.storage.confirming_delete);

        let _ = state.update(Message::CheckLargeDelete(true));
        let _ = state.update(Message::DeleteSelected);
        assert!(state.storage.removing);
    }
}
