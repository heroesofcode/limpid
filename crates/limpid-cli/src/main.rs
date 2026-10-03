//! Headless front-end for the Limpid engine.
//!
//! Exists for two reasons: scripting, and giving the test suite something to
//! drive that is not a window. Everything the GUI can discover, this can
//! print.

#![forbid(unsafe_code)]

use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{Parser, Subcommand};
use limpid_core::analyse::{self, Survey};
use limpid_core::catalog::{self, Context};
use limpid_core::config::{self, Store};
use limpid_core::execute::{Executor, Outcome, Problem};
use limpid_core::model::{Risk, Scan};
use limpid_core::paths::{ROOT_OVERRIDE, Roots};
use limpid_core::plan::{Magnitude, Plan, Selection};
use limpid_core::privileged::{Report, Request, RunError, Runner};
use limpid_core::size::human;

#[derive(Parser)]
#[command(
    name = "limpid-cli",
    version,
    about = "Scan a Linux system for reclaimable space"
)]
struct Cli {
    /// Treat this directory as the filesystem root, for testing against a
    /// fixture instead of the running machine. Also settable as $LIMPID_ROOT.
    #[arg(long, value_name = "DIR", global = true, env = ROOT_OVERRIDE)]
    root: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

/// Risk levels, as a command-line value.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum RiskArg {
    /// Only what regenerates itself with no consequence.
    Safe,
    /// Also what costs a re-download or a slow first launch.
    Review,
    /// Also what could lose something. Rarely what you want.
    Sensitive,
}

impl From<RiskArg> for Risk {
    fn from(argument: RiskArg) -> Self {
        match argument {
            RiskArg::Safe => Self::Safe,
            RiskArg::Review => Self::Review,
            RiskArg::Sensitive => Self::Sensitive,
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Look for reclaimable space without changing anything.
    Scan {
        /// Print machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Remove what a scan found. Reports without changing anything unless
    /// told to apply.
    Clean {
        /// Actually remove things. Without this, nothing is touched.
        #[arg(long)]
        apply: bool,
        /// The highest risk level to include.
        #[arg(long, value_enum, default_value = "safe")]
        risk: RiskArg,
        /// Also do the parts that need root, which asks for authentication.
        #[arg(long)]
        include_root: bool,
        /// Go ahead even though this removes an unusually large share of
        /// the disk. Look at the list without --apply first.
        #[arg(long)]
        accept_large: bool,
    },
    /// Show where the space went, without judging any of it.
    Storage {
        /// The directory to break down. Defaults to the home directory.
        #[arg(long, value_name = "DIR")]
        path: Option<PathBuf>,
        /// How many of the largest individual files to list.
        #[arg(long, default_value_t = 10)]
        files: usize,
    },
    /// Show the settings in force, and where they came from.
    Config,
    /// Stop Limpid offering a path, or everything under it. The path is
    /// still counted where space is being accounted for; it is only never
    /// offered for removal, and never removed.
    Exclude {
        /// Stop excluding these paths instead.
        #[arg(long)]
        remove: bool,
        /// Paths to exclude. Relative paths are taken from the current
        /// directory; `~/` means the home directory.
        #[arg(required = true, value_name = "PATH")]
        paths: Vec<PathBuf>,
    },
    /// Show the palette Limpid would draw with, and where it came from.
    Theme {
        /// Resolve this colors.toml instead of the active theme.
        #[arg(long, value_name = "FILE")]
        file: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .with_writer(io::stderr)
        .init();

    let cli = Cli::parse();

    let roots = match &cli.root {
        Some(root) => Roots::under(root),
        None => Roots::from_env(),
    };

    // Read once, up front. A problem with the file is reported and never
    // fatal: every setting has a default, and refusing to scan because of a
    // typo in a config file would be the wrong way round.
    let mut store = Store::open(&roots);
    // `config` shows them in its own report; everything else hears about
    // them here, once.
    if !matches!(cli.command, Command::Config) {
        for warning in &store.warnings {
            eprintln!("limpid: {warning}");
        }
    }
    let context = Context::with_config(roots, store.config.clone());

    let result = match cli.command {
        Command::Scan { json } => {
            let scan = catalog::scan(&context);
            let colour = io::stdout().is_terminal();
            let mut stdout = io::stdout().lock();
            if json {
                serde_json::to_writer_pretty(&mut stdout, &scan)
                    .map_err(io::Error::from)
                    .and_then(|()| writeln!(stdout))
            } else {
                report(&mut stdout, &scan, colour)
            }
        }
        Command::Clean {
            apply,
            risk,
            include_root,
            accept_large,
        } => {
            let scan = catalog::scan(&context);
            let selection = Selection {
                up_to: risk.into(),
                include_privileged: include_root,
            };
            let plan = Plan::from_targets(
                scan.categories
                    .iter()
                    .flat_map(|category| &category.targets)
                    .filter(|target| selection.includes(target)),
            );

            // Before anything is touched, and before the helper is asked:
            // a refusal after half the plan has run is not a refusal.
            let magnitude = plan.magnitude(scan.capacity);
            if apply && let Some(why) = refusal_to_apply(magnitude, accept_large) {
                eprintln!("limpid: {why}");
                std::process::exit(2);
            }

            let executor = if apply {
                Executor::applying(&context.roots)
            } else {
                Executor::dry_run(&context.roots)
            }
            .with_exclusions(context.config.exclusions.clone());
            let outcome = executor.run(&plan);

            // The privileged half is a separate request to a separate
            // process, and only made when the user asked for it and meant
            // it: a dry run never raises an authentication prompt.
            let elevated = if apply && plan.needs_elevation() {
                Some(Runner::for_roots(&context.roots).run(&Request {
                    operations: plan.operations.clone(),
                }))
            } else {
                None
            };

            let colour = io::stdout().is_terminal();
            let mut stdout = io::stdout().lock();
            report_clean(
                &mut stdout,
                &Shown {
                    plan: &plan,
                    outcome: &outcome,
                    elevated: elevated.as_ref(),
                    magnitude,
                    home: &context.roots.home,
                },
                colour,
            )
        }
        Command::Storage { path, files } => {
            let root = path.unwrap_or_else(|| context.roots.home.clone());
            let colour = io::stdout().is_terminal();
            // The trash's own paths, from the target that empties it, so the
            // figure printed is what emptying would remove.
            let trash = catalog::trash::target(&context.roots).paths;
            match analyse::survey(&root, files, &context.walk, &trash) {
                Ok(survey) => {
                    let mut stdout = io::stdout().lock();
                    report_storage(&mut stdout, &survey, colour)
                }
                Err(error) => {
                    eprintln!("cannot read {}: {error}", root.display());
                    std::process::exit(1);
                }
            }
        }
        Command::Config => {
            let colour = io::stdout().is_terminal();
            let mut stdout = io::stdout().lock();
            show_config(&mut stdout, &store, &context.roots.home, colour)
        }
        Command::Exclude { remove, paths } => {
            if !store.is_writable() {
                eprintln!(
                    "limpid: {} could not be read, so it will not be overwritten; fix it or remove it",
                    store.path().display(),
                );
                std::process::exit(1);
            }

            let home = context.roots.home.clone();
            for path in paths {
                // The shell expands an unquoted ~; a quoted one arrives as
                // written, and means the same thing.
                let absolute = config::expand(&path.to_string_lossy(), &home)
                    .or_else(|| std::path::absolute(&path).ok());
                let Some(absolute) = absolute else {
                    eprintln!("limpid: cannot make sense of {}", path.display());
                    std::process::exit(1);
                };
                let changed = if remove {
                    store.config.exclusions.remove(&absolute)
                } else {
                    store.config.exclusions.add(absolute.clone())
                };
                let verb = match (remove, changed) {
                    (false, true) => "excluded",
                    (false, false) => "already excluded",
                    (true, true) => "no longer excluded",
                    (true, false) => "was not excluded",
                };
                println!("{verb}: {}", absolute.display());
            }

            store
                .save()
                .map_err(|error| io::Error::other(error.to_string()))
        }
        Command::Theme { file } => {
            let theme = match &file {
                Some(path) => {
                    let source = std::fs::read_to_string(path)?;
                    let palette =
                        limpid_theme::omarchy::resolve(&source, false).ok_or_else(|| {
                            anyhow::anyhow!("no usable palette in {}", path.display())
                        })?;
                    limpid_theme::Theme {
                        palette,
                        source: limpid_theme::Source::Omarchy(None),
                    }
                }
                None => limpid_theme::Theme::detect(),
            };
            let colour = io::stdout().is_terminal();
            let mut stdout = io::stdout().lock();
            show_theme(&mut stdout, &theme, colour)
        }
    };

    // Being piped into `head` is not a failure, and printing a backtrace
    // about it is noise.
    match result {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        other => Ok(other?),
    }
}

/// ANSI styling, dropped when the output is not a terminal.
struct Style {
    enabled: bool,
}

impl Style {
    fn paint(&self, code: &str, text: &str) -> String {
        if self.enabled {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }

    fn bold(&self, text: &str) -> String {
        self.paint("1", text)
    }

    fn dim(&self, text: &str) -> String {
        self.paint("2", text)
    }

    /// Colour a risk label the way the GUI will: nothing for safe, yellow for
    /// review, red for sensitive.
    fn risk(&self, risk: Risk) -> String {
        match risk {
            Risk::Safe => String::new(),
            Risk::Review => self.paint("33", " review"),
            Risk::Sensitive => self.paint("31", " sensitive"),
        }
    }
}

/// Print a scan for a human.
fn report(out: &mut impl Write, scan: &Scan, colour: bool) -> io::Result<()> {
    let style = Style { enabled: colour };

    if scan.categories.is_empty() {
        writeln!(out, "Nothing to reclaim.")?;
        return Ok(());
    }

    for category in &scan.categories {
        writeln!(
            out,
            "\n{}  {}",
            style.bold(&category.name),
            style.dim(&human(category.size().on_disk)),
        )?;

        for target in &category.targets {
            let size = if target.size.is_zero() {
                // An attention item has no size; padding keeps the column.
                "        —".to_owned()
            } else {
                format!("{:>9}", human(target.size.on_disk))
            };
            let root = if target.requires_root {
                style.dim(" root")
            } else {
                String::new()
            };

            writeln!(
                out,
                "  {size}  {}{}{}",
                target.name,
                root,
                style.risk(target.risk)
            )?;

            // A blocked finding is listed with its reason rather than
            // quietly dropped: the user can act on "close Brave", and cannot
            // act on a number that went missing without explanation.
            if let Some(reason) = &target.blocked {
                let headline = reason
                    .split_once(". ")
                    .map_or(reason.as_str(), |(head, _)| head);
                writeln!(out, "             {}", style.paint("33", headline))?;
            }
        }
    }

    let total = scan.size();
    let tally = scan.tally();

    writeln!(
        out,
        "\n{}  {}",
        style.bold("Total found"),
        human(total.on_disk)
    )?;
    // Ready is always said, even when it is nothing: that is the answer to
    // the question. The rest only when there is some, and together the four
    // add up to what was found.
    writeln!(
        out,
        "{}  {}",
        style.bold("Ready to reclaim"),
        human(tally.ready.on_disk),
    )?;
    for (label, size) in [
        ("Needs a decision", tally.needs_decision),
        ("In use", tally.in_use),
        ("Needs root", tally.needs_root),
    ] {
        if !size.is_zero() {
            writeln!(out, "{label}  {}", human(size.on_disk))?;
        }
    }

    // The apparent/on-disk gap is worth showing only when it is real, which
    // on a compressed filesystem it usually is.
    if total.apparent > total.on_disk {
        writeln!(
            out,
            "{}",
            style.dim(&format!(
                "Files total {} but occupy {}; the difference is compression.",
                human(total.apparent),
                human(total.on_disk),
            )),
        )?;
    }

    for caveat in &scan.caveats {
        writeln!(out, "\n{}", style.dim(caveat))?;
    }

    Ok(())
}

/// Why an `--apply` run must not go ahead, if it must not.
fn refusal_to_apply(magnitude: Option<Magnitude>, accepted: bool) -> Option<String> {
    let magnitude = magnitude.filter(|_| !accepted)?;
    Some(format!(
        "{} Nothing was changed. Run it without --apply to see the list, and add \
         --accept-large once you have.",
        magnitude.describe(),
    ))
}

/// Everything a clean report says.
struct Shown<'a> {
    plan: &'a Plan,
    outcome: &'a Outcome,
    elevated: Option<&'a Result<Report, RunError>>,
    magnitude: Option<Magnitude>,
    home: &'a Path,
}

/// Print what a clean did, or would do.
fn report_clean(out: &mut impl Write, shown: &Shown, colour: bool) -> io::Result<()> {
    let style = Style { enabled: colour };
    let Shown {
        plan,
        outcome,
        elevated,
        magnitude,
        home,
    } = *shown;

    if plan.is_empty() {
        writeln!(out, "Nothing selected.")?;
        return Ok(());
    }

    for item in &plan.items {
        writeln!(
            out,
            "  {:>9}  {}  {}",
            human(item.expected.on_disk),
            item.name,
            style.dim(item.disposal.describe()),
        )?;
        // The exact list: every directory emptied and every file removed,
        // unless the name already is the one path.
        for path in &item.paths {
            let shown = config::contract(path, home);
            if item.paths.len() == 1 && shown == item.name {
                continue;
            }
            writeln!(out, "  {:>9}    {}", "", style.dim(&shown))?;
        }
    }

    for operation in &plan.operations {
        writeln!(out, "  {:>9}  {}", "", style.dim(&operation.describe()),)?;
    }

    writeln!(out)?;

    match elevated {
        Some(Ok(report)) => {
            for done in &report.completed {
                let mark = if done.succeeded {
                    style.paint("32", "done")
                } else {
                    style.paint("31", "failed")
                };
                writeln!(out, "{mark} {}: {}", done.operation, done.detail)?;
            }
        }
        Some(Err(error)) => {
            writeln!(
                out,
                "{} {error}",
                style.paint("31", "elevated part not done:")
            )?;
        }
        None if plan.needs_elevation() && !outcome.applied => {
            writeln!(
                out,
                "{}",
                style.dim(&format!(
                    "The parts needing root would free about {} more. They are handed \
                     to the privileged helper, which asks for authentication; pass \
                     --include-root --apply to do it.",
                    human(plan.operations_expected.on_disk),
                )),
            )?;
        }
        None => {}
    }

    // A dry run is where a large plan should be read, so it says so here
    // rather than only when --apply refuses.
    if let Some(magnitude) = magnitude.filter(|_| !outcome.applied) {
        writeln!(
            out,
            "{} {}",
            style.paint("33", &magnitude.describe()),
            style.dim("Read the list above; --apply will ask for --accept-large."),
        )?;
    }

    if outcome.applied {
        writeln!(
            out,
            "{} {} in {} files.",
            style.bold("Reclaimed"),
            human(outcome.reclaimed.on_disk),
            outcome.files,
        )?;
    } else {
        writeln!(
            out,
            "{} {} in {} files. Nothing was changed; pass --apply to do it.",
            style.bold("Would reclaim"),
            human(outcome.reclaimed.on_disk),
            outcome.files,
        )?;
    }

    for problem in &outcome.problems {
        let label = match problem {
            Problem::InUse { .. } => "in use",
            Problem::Excluded(_) => "excluded",
            Problem::Refused(_) | Problem::FolderNotDeleted(_) => "refused",
            Problem::Failed { .. } => "failed",
        };
        writeln!(out, "{} {problem}", style.paint("31", label))?;
    }

    Ok(())
}

/// Print a directory breakdown and the largest files under it.
fn report_storage(out: &mut impl Write, survey: &Survey, colour: bool) -> io::Result<()> {
    let style = Style { enabled: colour };
    let breakdown = &survey.breakdown;
    let largest = &survey.largest;
    let total = breakdown.total().on_disk;

    writeln!(
        out,
        "{}  {}",
        style.bold(&breakdown.root.display().to_string()),
        human(total),
    )?;

    if breakdown.is_empty() {
        writeln!(out, "  empty")?;
        return Ok(());
    }

    for child in breakdown.children.iter().take(20) {
        let share = child.share_of(total);
        writeln!(
            out,
            "  {:>9}  {}  {}{}",
            human(child.size.on_disk),
            bar(share, 16),
            child.name,
            if child.is_dir { "/" } else { "" },
        )?;
    }

    if !largest.is_empty() {
        writeln!(out, "\n{}", style.bold("Largest files"))?;
        for entry in largest {
            writeln!(
                out,
                "  {:>9}  {}",
                human(entry.size.on_disk),
                style.dim(&entry.path.display().to_string()),
            )?;
        }
    }

    // Said apart from the list: moving something to the trash keeps it on
    // the disk, and this is where it went.
    if !survey.trash.is_empty() {
        writeln!(
            out,
            "\n{}  {}  {}",
            style.bold("In the trash"),
            human(survey.trash.size.on_disk),
            style.dim(&format!(
                "across {} files, which frees nothing until the trash is emptied",
                survey.trash.files,
            )),
        )?;
    }

    if breakdown.unreadable > 0 {
        writeln!(
            out,
            "\n{}",
            style.dim(&format!(
                "{} paths could not be read, so this is a lower bound.",
                breakdown.unreadable,
            )),
        )?;
    }

    Ok(())
}

/// A proportion, as a bar of block characters.
///
/// Uses the eighth-block characters rather than whole cells. Forcing a whole
/// block for anything non-zero — the obvious alternative — makes a directory
/// holding a thousandth of the total look the same as one holding a
/// sixteenth, which is the one thing the bar exists to distinguish.
fn bar(share: f32, width: usize) -> String {
    const EIGHTHS: [char; 8] = [
        '\u{258f}', '\u{258e}', '\u{258d}', '\u{258c}', '\u{258b}', '\u{258a}', '\u{2589}',
        '\u{2588}',
    ];

    let eighths = (share.clamp(0.0, 1.0) * (width * 8) as f32).round() as usize;
    let full = eighths / 8;
    let remainder = eighths % 8;

    let mut bar = String::with_capacity(width * 3);
    for _ in 0..full.min(width) {
        bar.push('\u{2588}');
    }
    if full < width && remainder > 0 {
        bar.push(EIGHTHS[remainder - 1]);
    }
    let drawn = full.min(width) + usize::from(full < width && remainder > 0);
    for _ in drawn..width {
        bar.push(' ');
    }
    bar
}

/// Print the settings in force.
fn show_config(
    out: &mut impl Write,
    store: &Store,
    home: &std::path::Path,
    colour: bool,
) -> io::Result<()> {
    let style = Style { enabled: colour };
    let shown = |path: &std::path::Path| match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    };

    let state = if !store.path().exists() {
        "not created yet; these are the defaults"
    } else if store.is_writable() {
        "in use"
    } else {
        "read, but will not be overwritten"
    };
    writeln!(
        out,
        "{}  {}",
        style.bold(&shown(store.path())),
        style.dim(state)
    )?;

    let policy = store.config.policy;
    writeln!(out)?;
    writeln!(
        out,
        "  Package versions kept  {}",
        policy.keep_package_versions
    )?;
    writeln!(out, "  Journal days kept      {}", policy.keep_journal_days)?;

    writeln!(out)?;
    let excluded = store.config.exclusions.paths();
    if excluded.is_empty() {
        writeln!(out, "  Nothing excluded")?;
    } else {
        writeln!(out, "  Excluded")?;
        for path in excluded {
            writeln!(out, "    {}", shown(path))?;
        }
    }

    for warning in &store.warnings {
        writeln!(out, "\n{}", style.paint("33", warning))?;
    }

    Ok(())
}

/// Print the resolved palette, with a swatch of each colour.
fn show_theme(out: &mut impl Write, theme: &limpid_theme::Theme, colour: bool) -> io::Result<()> {
    use limpid_theme::Color;

    let style = Style { enabled: colour };
    let palette = &theme.palette;

    writeln!(
        out,
        "{}",
        style.bold(&format!("Following {}", theme.source.describe()))
    )?;
    writeln!(
        out,
        "{}",
        style.dim(&format!(
            "{:?} mode{}",
            palette.mode,
            if theme.source.is_live() {
                ", updates live"
            } else {
                ""
            },
        )),
    )?;

    let swatch = |value: Color| {
        if colour {
            // Two spaces of background is enough to read the hue at a glance.
            format!("\x1b[48;2;{};{};{}m  \x1b[0m", value.r, value.g, value.b)
        } else {
            String::new()
        }
    };

    let entries: [(&str, Color); 20] = [
        ("accent", palette.accent),
        ("background", palette.background),
        ("dark_background", palette.dark_background),
        ("darker_background", palette.darker_background),
        ("lighter_background", palette.lighter_background),
        ("foreground", palette.foreground),
        ("dark_foreground", palette.dark_foreground),
        ("light_foreground", palette.light_foreground),
        ("bright_foreground", palette.bright_foreground),
        ("selection", palette.selection),
        ("muted", palette.muted),
        ("red", palette.red),
        ("yellow", palette.yellow),
        ("orange", palette.orange),
        ("green", palette.green),
        ("cyan", palette.cyan),
        ("blue", palette.blue),
        ("magenta", palette.magenta),
        ("brown", palette.brown),
        ("bright_red", palette.bright_red),
    ];

    writeln!(out)?;
    for (name, value) in entries {
        writeln!(out, "  {} {:<19} {value}", swatch(value), name)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use limpid_core::model::{Category, Kind, Target};
    use limpid_core::size::Size;

    fn scan_with(targets: Vec<Target>) -> Scan {
        let mut category = Category::new("Caches", "");
        category.targets = targets;
        Scan {
            categories: vec![category],
            ..Scan::default()
        }
    }

    fn render_clean(plan: &Plan, outcome: &Outcome) -> String {
        render_clean_with(plan, outcome, None)
    }

    fn render_clean_with(plan: &Plan, outcome: &Outcome, magnitude: Option<Magnitude>) -> String {
        let mut buffer = Vec::new();
        let shown = Shown {
            plan,
            outcome,
            elevated: None,
            magnitude,
            home: Path::new("/home/x"),
        };
        report_clean(&mut buffer, &shown, false).unwrap();
        String::from_utf8(buffer).unwrap()
    }

    fn render(scan: &Scan) -> String {
        let mut buffer = Vec::new();
        report(&mut buffer, scan, false).unwrap();
        String::from_utf8(buffer).unwrap()
    }

    #[test]
    fn a_dry_run_says_the_privileged_half_would_need_authentication() {
        use limpid_core::model::Kind;
        use limpid_core::privileged::Operation;
        use limpid_core::size::Size;

        let plan = Plan::from_targets(&[Target::new("pacman", Kind::PackageCache, Risk::Review)
            .measured(Size::new(100, 100), 1)
            .by_operation(Operation::TrimPackageCache { keep: 3 })]);

        let output = render_clean(&plan, &Outcome::default());

        assert!(output.contains("Keep the 3 newest versions"), "{output}");
        assert!(output.contains("asks for authentication"), "{output}");
    }

    #[test]
    fn a_proportion_bar_is_always_the_width_asked_for() {
        for share in [0.0, 0.001, 0.37, 0.5, 1.0, 5.0, f32::NAN] {
            assert_eq!(bar(share, 8).chars().count(), 8, "share {share}");
        }
    }

    #[test]
    fn a_proportion_bar_distinguishes_small_shares_from_each_other() {
        // Whole blocks alone cannot tell these apart; eighths can.
        assert_eq!(bar(1.0, 8), "████████");
        assert_eq!(bar(0.5, 8), "████    ");
        assert_ne!(bar(0.02, 8), bar(0.10, 8));
        assert_eq!(bar(0.0, 8), "        ");
    }

    #[test]
    fn an_empty_scan_says_so() {
        let output = render(&Scan::default());
        assert_eq!(output.trim(), "Nothing to reclaim.");
    }

    #[test]
    fn the_report_says_what_is_ready_and_why_the_rest_is_not() {
        let scan = scan_with(vec![
            Target::new("user cache", Kind::Cache, Risk::Safe).measured(Size::new(1024, 1024), 1),
            Target::new("a project", Kind::BuildArtifact, Risk::Review)
                .measured(Size::new(2048, 2048), 1),
            Target::new("system cache", Kind::Cache, Risk::Safe)
                .measured(Size::new(3072, 3072), 1)
                .requires_root(),
        ]);

        let output = render(&scan);

        assert!(output.contains("Total found  6.00 KiB"), "{output}");
        assert!(output.contains("Ready to reclaim  1.00 KiB"), "{output}");
        assert!(output.contains("Needs a decision  2.00 KiB"), "{output}");
        assert!(output.contains("Needs root  3.00 KiB"), "{output}");
        // Nothing is open, so that line is not there to be read.
        assert!(!output.contains("In use"), "{output}");
    }

    #[test]
    fn a_dry_run_lists_every_directory_it_would_empty() {
        let plan = Plan::from_targets(&[Target::new("Brave — web cache", Kind::Cache, Risk::Safe)
            .measured(Size::new(100, 100), 1)
            .path("/home/x/.cache/BraveSoftware/Brave-Browser/Default/Cache")
            .path("/home/x/.config/BraveSoftware/Brave-Browser/Default/GPUCache")]);

        let output = render_clean(&plan, &Outcome::default());

        assert!(
            output.contains("~/.cache/BraveSoftware/Brave-Browser/Default/Cache"),
            "{output}"
        );
        assert!(
            output.contains("~/.config/BraveSoftware/Brave-Browser/Default/GPUCache"),
            "{output}"
        );
    }

    #[test]
    fn a_name_that_already_is_the_path_is_not_repeated() {
        let plan = Plan::from_targets(&[Target::new(
            "~/Work/old/target",
            Kind::BuildArtifact,
            Risk::Safe,
        )
        .measured(Size::new(100, 100), 1)
        .path("/home/x/Work/old/target")]);

        let output = render_clean(&plan, &Outcome::default());

        assert_eq!(output.matches("~/Work/old/target").count(), 1, "{output}");
    }

    #[test]
    fn a_large_plan_is_refused_on_apply_until_it_is_accepted() {
        let large = Magnitude {
            expected: Size::new(20 << 30, 20 << 30),
            share: Some(0.4),
        };

        let why = refusal_to_apply(Some(large), false).expect("a refusal");
        assert!(why.contains("40%"), "{why}");
        assert!(why.contains("--accept-large"), "{why}");

        assert_eq!(refusal_to_apply(Some(large), true), None);
        assert_eq!(refusal_to_apply(None, false), None);
    }

    #[test]
    fn a_dry_run_of_a_large_plan_says_so_before_anyone_applies_it() {
        let plan = Plan::from_targets(&[Target::new("x", Kind::Cache, Risk::Safe)
            .measured(Size::new(100, 100), 1)
            .path("/home/x/.cache/x")]);
        let large = Magnitude {
            expected: Size::new(20 << 30, 20 << 30),
            share: Some(0.4),
        };

        let output = render_clean_with(&plan, &Outcome::default(), Some(large));

        assert!(output.contains("40% of everything stored"), "{output}");
        assert!(output.contains("--accept-large"), "{output}");
    }

    #[test]
    fn compression_is_only_mentioned_when_the_sizes_actually_differ() {
        let uncompressed = scan_with(vec![
            Target::new("cache", Kind::Cache, Risk::Safe).measured(Size::new(4096, 4096), 1),
        ]);
        assert!(!render(&uncompressed).contains("compression"));

        let compressed = scan_with(vec![
            Target::new("cache", Kind::Cache, Risk::Safe).measured(Size::new(9000, 4096), 1),
        ]);
        assert!(render(&compressed).contains("compression"));
    }

    #[test]
    fn targets_needing_attention_show_a_dash_rather_than_a_size() {
        let scan = scan_with(vec![Target::new(
            "pacnew",
            Kind::Attention,
            Risk::Sensitive,
        )]);

        let output = render(&scan);

        assert!(output.contains("—  pacnew"), "{output}");
        assert!(output.contains("sensitive"), "{output}");
    }

    #[test]
    fn caveats_are_printed_after_the_totals() {
        let mut scan = scan_with(vec![
            Target::new("cache", Kind::Cache, Risk::Safe).measured(Size::new(10, 10), 1),
        ]);
        scan.caveats
            .push("Snapshots exist on this system.".to_owned());

        let output = render(&scan);
        let totals = output.find("Total found").unwrap();
        let caveat = output.find("Snapshots exist").unwrap();

        assert!(caveat > totals);
    }
}
