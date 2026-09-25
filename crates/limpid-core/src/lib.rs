//! Scanning and cleaning engine for Limpid.
//!
//! Nothing in this crate touches the filesystem destructively on its own: a
//! scan produces targets, the caller assembles them into a plan, and only an
//! executor acts on that plan.

#![forbid(unsafe_code)]

/// Crate version, surfaced by both the CLI and the GUI.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
