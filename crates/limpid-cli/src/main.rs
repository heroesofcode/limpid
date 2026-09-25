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
use limpid_core::catalog::{self, Context};
use limpid_core::model::{Risk, Scan};
use limpid_core::paths::{ROOT_OVERRIDE, Roots};
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

#[derive(Subcommand)]
enum Command {
    /// Look for reclaimable space without changing anything.
    Scan {
        /// Print machine-readable output.
        #[arg(long)]
        json: bool,
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
            caveats: Vec::new(),
        }
    }

    fn render(scan: &Scan) -> String {
        let mut buffer = Vec::new();
        report(&mut buffer, scan, false).unwrap();
        String::from_utf8(buffer).unwrap()
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
