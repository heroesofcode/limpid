//! Headless front-end for the Limpid engine.

#![forbid(unsafe_code)]

use clap::Parser;

#[derive(Parser)]
#[command(name = "limpid-cli", version, about = "Scan and clean a Linux system")]
struct Cli {}

fn main() -> anyhow::Result<()> {
    let _cli = Cli::parse();
    println!("limpid {}", limpid_core::VERSION);
    Ok(())
}
