//! Privileged helper for Limpid.
//!
//! This binary runs as root through polkit. It reads a plan on stdin and
//! refuses anything outside its compiled-in allowlist; keeping it small is the
//! point, so scanning and policy live in `limpid-core`, not here.

#![forbid(unsafe_code)]

fn main() -> anyhow::Result<()> {
    anyhow::bail!("limpid-helper takes a plan on stdin; none was given")
}
