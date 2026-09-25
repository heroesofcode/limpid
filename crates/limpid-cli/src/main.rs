//! Headless front-end for the Limpid engine.
//!
//! Exists for two reasons: scripting, and giving the test suite something to
//! drive that is not a window. Everything the GUI can discover, this can
//! print.

#![forbid(unsafe_code)]

use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use limpid_core::analyse::{self, Breakdown, Entry};
use limpid_core::catalog::{self, Context};
use limpid_core::execute::{Executor, Outcome, Problem};
use limpid_core::model::{Risk, Scan};
use limpid_core::paths::{ROOT_OVERRIDE, Roots};
use limpid_core::plan::{Plan, Selection};
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
    let context = Context::with_roots(roots);

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
        Command::Clean { apply, risk } => {
            let scan = catalog::scan(&context);
            let selection = Selection {
                up_to: risk.into(),
                include_privileged: false,
            };
            let plan = Plan::from_targets(
                scan.categories
                    .iter()
                    .flat_map(|category| &category.targets)
                    .filter(|target| selection.includes(target)),
            );

            let executor = if apply {
                Executor::applying(&context.roots)
            } else {
                Executor::dry_run(&context.roots)
            };
            let outcome = executor.run(&plan);

            let colour = io::stdout().is_terminal();
            let mut stdout = io::stdout().lock();
            report_clean(&mut stdout, &plan, &outcome, colour)
        }
        Command::Storage { path, files } => {
            let root = path.unwrap_or_else(|| context.roots.home.clone());
            let colour = io::stdout().is_terminal();
            match analyse::breakdown(&root, &context.walk) {
                Ok(breakdown) => {
                    let largest =
                        analyse::largest_files(&root, files, &context.walk).unwrap_or_default();
                    let mut stdout = io::stdout().lock();
                    report_storage(&mut stdout, &breakdown, &largest, colour)
                }
                Err(error) => {
                    eprintln!("cannot read {}: {error}", root.display());
                    std::process::exit(1);
                }
            }
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
    let unprivileged = scan.reclaimable_unprivileged();

    writeln!(
        out,
        "\n{}  {}",
        style.bold("Total found"),
        human(total.on_disk)
    )?;
    writeln!(
        out,
        "{}  {}",
        style.bold("Reclaimable without elevation"),
        human(unprivileged.on_disk),
    )?;

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

/// Print what a clean did, or would do.
fn report_clean(
    out: &mut impl Write,
    plan: &Plan,
    outcome: &Outcome,
    colour: bool,
) -> io::Result<()> {
    let style = Style { enabled: colour };

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
    }

    writeln!(out)?;
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
            Problem::Refused(_) => "refused",
            Problem::Failed { .. } => "failed",
        };
        writeln!(out, "{} {problem}", style.paint("31", label))?;
    }

    Ok(())
}

/// Print a directory breakdown and the largest files under it.
fn report_storage(
    out: &mut impl Write,
    breakdown: &Breakdown,
    largest: &[Entry],
    colour: bool,
) -> io::Result<()> {
    let style = Style { enabled: colour };
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

    fn render(scan: &Scan) -> String {
        let mut buffer = Vec::new();
        report(&mut buffer, scan, false).unwrap();
        String::from_utf8(buffer).unwrap()
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
    fn the_report_separates_total_from_what_needs_no_elevation() {
        let scan = scan_with(vec![
            Target::new("user cache", Kind::Cache, Risk::Safe).measured(Size::new(1024, 1024), 1),
            Target::new("system cache", Kind::Cache, Risk::Safe)
                .measured(Size::new(3072, 3072), 1)
                .requires_root(),
        ]);

        let output = render(&scan);

        assert!(output.contains("Total found  4.00 KiB"), "{output}");
        assert!(
            output.contains("Reclaimable without elevation  1.00 KiB"),
            "{output}"
        );
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
