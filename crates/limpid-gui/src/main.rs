//! The Limpid desktop application.

#![forbid(unsafe_code)]

fn main() -> anyhow::Result<()> {
    println!("limpid {}", limpid_core::VERSION);
    Ok(())
}
