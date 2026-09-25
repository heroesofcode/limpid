//! Browser caches.
//!
//! Usually the largest single win on a desktop machine, and the one most
//! easily got wrong. See [`crate::browser`] for why the data lives in two
//! trees and why an open browser is a refusal rather than a warning.

use super::Context;
use crate::browser::{Family, Installation, Profile, holder_of};
use crate::model::{Category, Kind, Risk, Target};

/// Measure every browser's caches.
pub fn scan(context: &Context) -> Category {
    let mut category = Category::new(
        "Browsers",
        "Pages, scripts and assets kept so a site loads faster the second time.",
    );

    for installation in Installation::all(&context.roots) {
        // One check per browser rather than per profile: walking every
        // process's descriptors is not cheap, and a browser with one profile
        // open is closed for all of them as far as this is concerned.
        let blocked = holder_of(&installation.config).map(|holder| {
            format!(
                "{} is running as {} ({}). Close it first — removing these while it \
                 is open would not free the space, and can make it discard the whole \
                 profile database rather than the part you asked for.",
                installation.name, holder.name, holder.pid,
            )
        });

        let profiles = installation.profiles();
        let many = profiles.len() > 1;

        let mut found: Vec<Target> = profiles
            .iter()
            .flat_map(|profile| targets_for(&installation, profile, many))
            .collect();
        if let Some(target) = shared_target(&installation) {
            found.push(target);
        }

        for target in found {
            category.targets.push(match &blocked {
                Some(reason) => target.blocked(reason.clone()),
                None => target,
            });
        }
    }

    context.measure_all(category)
}

/// The targets one profile contributes.
fn targets_for(
    installation: &Installation,
    profile: &Profile,
    name_the_profile: bool,
) -> Vec<Target> {
    let label = |what: &str| {
        if name_the_profile {
            format!("{} — {what} ({})", installation.name, profile.name)
        } else {
            format!("{} — {what}", installation.name)
        }
    };

    match installation.family {
        Family::Chromium => chromium_targets(&label, profile),
        Family::Firefox => firefox_targets(&label, profile),
    }
}

/// Chromium keeps its caches in both trees; these cover both.
fn chromium_targets(label: &dyn Fn(&str) -> String, profile: &Profile) -> Vec<Target> {
    vec![
        Target::new(label("web cache"), Kind::Cache, Risk::Safe)
            .detail(
                "Pages, images and scripts kept so a site loads faster next time. \
                 Usually the largest single thing on a desktop machine.",
            )
            .path(profile.cache.join("Cache/Cache_Data")),
        Target::new(label("compiled scripts"), Kind::Cache, Risk::Safe)
            .detail(
                "Javascript and WebAssembly compiled ahead of time. Sites recompile \
                 them on the next visit.",
            )
            .path(profile.cache.join("Code Cache")),
        Target::new(label("graphics cache"), Kind::Cache, Risk::Safe)
            .detail("Compiled shaders and GPU state.")
            .path(profile.cache.join("GPUCache"))
            .path(profile.config.join("GPUCache"))
            .path(profile.config.join("DawnGraphiteCache"))
            .path(profile.config.join("DawnWebGPUCache"))
            .path(profile.config.join("GrShaderCache")),
        // This one lives entirely in the config tree, which is why tools
        // that clean only ~/.cache leave it behind.
        Target::new(label("offline site data"), Kind::Cache, Risk::Review)
            .detail(
                "What service workers have stored so their sites work offline. \
                 Removing it logs you out of nothing, but an offline-capable site \
                 will have to download itself again.",
            )
            .path(profile.config.join("Service Worker/CacheStorage"))
            .path(profile.config.join("Service Worker/ScriptCache")),
        Target::new(label("filter lists"), Kind::Cache, Risk::Safe)
            .detail("Compiled ad-blocking rules, rebuilt from the subscribed lists.")
            .path(profile.config.join("adblock_cache")),
    ]
}

/// Firefox splits its profile and cache the same way, with different names.
fn firefox_targets(label: &dyn Fn(&str) -> String, profile: &Profile) -> Vec<Target> {
    vec![
        Target::new(label("web cache"), Kind::Cache, Risk::Safe)
            .detail("Pages, images and scripts kept so a site loads faster next time.")
            .path(profile.cache.join("cache2")),
        Target::new(label("startup cache"), Kind::Cache, Risk::Safe)
            .detail("Precomputed startup data, rebuilt on the next launch.")
            .path(profile.cache.join("startupCache")),
    ]
}

/// Caches a Chromium browser shares across all its profiles.
///
/// These sit at the top of the config tree, beside `Local State` — which is
/// named one directory away and never touched.
fn shared_target(installation: &Installation) -> Option<Target> {
    if installation.family != Family::Chromium {
        return None;
    }

    let mut target = Target::new(
        format!("{} — downloaded components", installation.name),
        Kind::Cache,
        Risk::Safe,
    )
    .detail(
        "Extension and component archives the browser downloaded, plus crash          reports it has not sent. All re-fetched when needed.",
    );

    for name in [
        "component_crx_cache",
        "extensions_crx_cache",
        "GPUPersistentCache",
        "Crash Reports",
    ] {
        target = target.path(installation.config.join(name));
    }

    Some(target)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::paths::Roots;

    fn brave_fixture() -> (tempfile::TempDir, Roots) {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let config = roots.config("BraveSoftware/Brave-Browser");
        let cache = roots.cache("BraveSoftware/Brave-Browser");

        std::fs::create_dir_all(config.join("Default")).unwrap();
        std::fs::write(
            config.join("Local State"),
            r#"{"profile":{"info_cache":{"Default":{}}}}"#,
        )
        .unwrap();

        let write = |path: PathBuf, bytes: usize| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, vec![0u8; bytes]).unwrap();
        };
        write(cache.join("Default/Cache/Cache_Data/f_000001"), 800_000);
        write(cache.join("Default/Code Cache/js/index"), 200_000);
        // The half that lives in the config tree, which is the half most
        // tools never find.
        write(
            config.join("Default/Service Worker/CacheStorage/blob"),
            120_000,
        );
        write(config.join("Default/adblock_cache/rules.dat"), 11_000);

        (fixture, roots)
    }

    #[test]
    fn both_trees_are_measured() {
        let (_fixture, roots) = brave_fixture();

        let category = scan(&Context::with_roots(roots));
        let total: u64 = category
            .targets
            .iter()
            .map(|target| target.size.apparent)
            .sum();

        // 800k + 200k from ~/.cache, 120k + 11k from ~/.config. A tool that
        // only looked at the cache tree would report 1000000.
        assert_eq!(total, 1_131_000);
    }

    #[test]
    fn the_config_half_is_actually_found() {
        let (_fixture, roots) = brave_fixture();

        let category = scan(&Context::with_roots(roots));
        let offline = category
            .targets
            .iter()
            .find(|target| target.name == "Brave — offline site data")
            .unwrap();

        assert_eq!(offline.size.apparent, 120_000);
    }

    #[test]
    fn profiles_are_named_only_when_there_is_more_than_one() {
        let (_fixture, roots) = brave_fixture();
        let single = scan(&Context::with_roots(roots.clone()));
        assert!(
            single
                .targets
                .iter()
                .any(|target| target.name == "Brave — web cache")
        );

        let config = roots.config("BraveSoftware/Brave-Browser");
        std::fs::create_dir_all(config.join("Profile 1")).unwrap();
        std::fs::write(
            config.join("Local State"),
            r#"{"profile":{"info_cache":{"Default":{},"Profile 1":{}}}}"#,
        )
        .unwrap();

        let several = scan(&Context::with_roots(roots));
        assert!(
            several
                .targets
                .iter()
                .any(|target| target.name == "Brave — web cache (Default)")
        );
    }

    #[test]
    fn a_browser_holding_its_profile_open_blocks_every_one_of_its_targets() {
        let (_fixture, roots) = brave_fixture();
        let config = roots.config("BraveSoftware/Brave-Browser");

        // Stand in for the browser: hold a descriptor inside the profile.
        let _open = std::fs::File::open(config.join("Local State")).unwrap();

        let category = scan(&Context::with_roots(roots));

        assert!(!category.targets.is_empty());
        for target in &category.targets {
            let reason = target.blocked.as_ref().expect("should be blocked");
            assert!(reason.contains("Brave is running"), "{reason}");
        }
        // And none of it counts as available.
        assert_eq!(category.size_at_most(Risk::Sensitive).on_disk, 0);
    }

    #[test]
    fn nothing_is_blocked_when_the_browser_is_closed() {
        let (_fixture, roots) = brave_fixture();

        let category = scan(&Context::with_roots(roots));

        assert!(
            category
                .targets
                .iter()
                .all(|target| target.blocked.is_none())
        );
    }

    #[test]
    fn no_target_ever_names_local_state() {
        let (_fixture, roots) = brave_fixture();

        let category = scan(&Context::with_roots(roots));

        for target in &category.targets {
            for path in &target.paths {
                assert!(!path.ends_with("Local State"), "{}", path.display());
            }
        }
    }

    #[test]
    fn shared_component_caches_are_offered_once_per_browser() {
        let (_fixture, roots) = brave_fixture();
        let config = roots.config("BraveSoftware/Brave-Browser");
        std::fs::create_dir_all(config.join("component_crx_cache")).unwrap();
        std::fs::write(config.join("component_crx_cache/a.crx"), vec![0u8; 35_000]).unwrap();
        std::fs::create_dir_all(config.join("Profile 1")).unwrap();
        std::fs::write(
            config.join("Local State"),
            r#"{"profile":{"info_cache":{"Default":{},"Profile 1":{}}}}"#,
        )
        .unwrap();

        let category = scan(&Context::with_roots(roots));
        let shared: Vec<_> = category
            .targets
            .iter()
            .filter(|target| target.name == "Brave — downloaded components")
            .collect();

        // Two profiles, one shared entry — not one per profile.
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].size.apparent, 35_000);
    }

    #[test]
    fn a_machine_with_no_browsers_produces_nothing() {
        let fixture = tempfile::tempdir().unwrap();
        let category = scan(&Context::with_roots(Roots::under(fixture.path())));

        assert!(category.targets.is_empty());
    }
}
