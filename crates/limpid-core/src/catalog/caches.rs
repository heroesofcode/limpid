//! Application caches under `$XDG_CACHE_HOME`.
//!
//! Deliberately an allowlist. Plenty of applications keep non-regenerable
//! state under `~/.cache` in defiance of the spec, so "delete everything not
//! known to be precious" is the wrong default — the cost of being wrong is
//! someone's session token, and the reward is a few megabytes.

use super::Context;
use crate::model::{Category, Kind, Risk, Target};

/// A cache directory Limpid knows the cost of clearing.
struct Known {
    /// Directory name directly under the cache root.
    directory: &'static str,
    /// Name to show.
    name: &'static str,
    /// What clearing it costs.
    detail: &'static str,
    /// How careful to be.
    risk: Risk,
}

/// Caches that regenerate with no consequence worth mentioning.
const KNOWN: &[Known] = &[
    Known {
        directory: "thumbnails",
        name: "Thumbnails",
        detail: "Previews for files you have browsed. Regenerated on demand.",
        risk: Risk::Safe,
    },
    Known {
        directory: "nvim",
        name: "Neovim",
        detail: "Shada state and language-server logs.",
        risk: Risk::Safe,
    },
    Known {
        directory: "quickshell",
        name: "Quickshell",
        detail: "Cached assets for the desktop shell.",
        risk: Risk::Safe,
    },
    Known {
        directory: "omarchy",
        name: "Omarchy",
        detail: "Theme previews and background thumbnails, rebuilt on next theme change.",
        risk: Risk::Safe,
    },
    Known {
        directory: "gh",
        name: "GitHub CLI",
        detail: "Cached API responses.",
        risk: Risk::Safe,
    },
    Known {
        directory: "deno",
        name: "Deno",
        detail: "Downloaded modules, re-fetched on next run.",
        risk: Risk::Safe,
    },
    Known {
        directory: "opencode",
        name: "opencode",
        detail: "Cached assets, rebuilt on next run.",
        risk: Risk::Safe,
    },
    // Everything below costs something real. It is still worth offering —
    // debuginfod alone is often the largest item in the whole directory — but
    // not without saying what the next launch will feel like.
    Known {
        directory: "debuginfod_client",
        name: "Debug symbols",
        detail: "Symbols downloaded for debugging crashes. Clearing means the next \
                 backtrace re-downloads them, often hundreds of megabytes.",
        risk: Risk::Review,
    },
    Known {
        directory: "mesa_shader_cache",
        name: "Shader cache",
        detail: "Compiled GPU shaders. Clearing makes the first launch of every \
                 graphical application slower while they recompile.",
        risk: Risk::Review,
    },
    Known {
        directory: "mesa_shader_cache_db",
        name: "Shader cache (database)",
        detail: "Compiled GPU shaders, newer on-disk format.",
        risk: Risk::Review,
    },
    Known {
        directory: "fontconfig",
        name: "Font cache",
        detail: "Font index. Rebuilt automatically, but the first application to \
                 start afterwards waits for it.",
        risk: Risk::Review,
    },
];

/// Measure the known cache directories.
pub fn scan(context: &Context) -> Category {
    let mut category = Category::new(
        "Application caches",
        "Data applications keep so they need not fetch or compute it again.",
    );

    for known in KNOWN {
        category.targets.push(
            Target::new(known.name, Kind::Cache, known.risk)
                .detail(known.detail)
                .path(context.roots.cache(known.directory)),
        );
    }

    context.measure_all(category)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Roots;

    #[test]
    fn a_home_with_no_caches_yields_nothing_measurable() {
        let fixture = tempfile::tempdir().unwrap();
        let context = Context::with_roots(Roots::under(fixture.path()));

        let category = scan(&context);

        assert!(category.targets.iter().all(|target| target.size.is_zero()));
    }

    #[test]
    fn a_populated_cache_directory_is_measured() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let thumbnails = roots.cache("thumbnails/normal");
        std::fs::create_dir_all(&thumbnails).unwrap();
        std::fs::write(thumbnails.join("a.png"), vec![0u8; 5000]).unwrap();

        let category = scan(&Context::with_roots(roots));
        let target = category
            .targets
            .iter()
            .find(|target| target.name == "Thumbnails")
            .unwrap();

        assert_eq!(target.size.apparent, 5000);
        assert_eq!(target.files, 1);
    }

    #[test]
    fn caches_with_a_real_cost_are_marked_for_review() {
        let fixture = tempfile::tempdir().unwrap();
        let category = scan(&Context::with_roots(Roots::under(fixture.path())));

        let symbols = category
            .targets
            .iter()
            .find(|target| target.name == "Debug symbols")
            .unwrap();
        assert_eq!(symbols.risk, Risk::Review);

        let thumbnails = category
            .targets
            .iter()
            .find(|target| target.name == "Thumbnails")
            .unwrap();
        assert_eq!(thumbnails.risk, Risk::Safe);
    }

    #[test]
    fn nothing_here_needs_elevation() {
        let fixture = tempfile::tempdir().unwrap();
        let category = scan(&Context::with_roots(Roots::under(fixture.path())));

        assert!(category.targets.iter().all(|target| !target.requires_root));
    }
}
