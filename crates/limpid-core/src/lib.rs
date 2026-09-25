//! Scanning and cleaning engine for Limpid.
//!
//! Nothing in this crate touches the filesystem destructively on its own: a
//! scan produces [`model::Target`]s, the caller assembles them into a plan,
//! and only an executor acts on that plan.
//!
//! # Measuring
//!
//! Sizes are reported two ways — see [`size::Size`]. The one that matters is
//! `on_disk`, taken from `st_blocks`, because on a compressed filesystem it
//! is the only number that predicts what `df` will say afterwards.
//!
//! # Testing
//!
//! Every path goes through [`paths::Roots`], so setting `LIMPID_ROOT` points
//! the whole engine at a fixture directory. The test suite never reads the
//! developer's real home.
//!
//! ```no_run
//! let context = limpid_core::catalog::Context::new();
//! let scan = limpid_core::catalog::scan(&context);
//! println!("{} reclaimable", scan.reclaimable_unprivileged());
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod catalog;
pub mod model;
pub mod paths;
pub mod size;
pub mod volume;
pub mod walk;

/// Crate version, surfaced by both the CLI and the GUI.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
