//! Finding browsers, their profiles, and whether they are running.
//!
//! Two things about browsers make them different from every other target.
//!
//! Their data lives in two trees, not one. A Chromium profile keeps its HTTP
//! cache under `$XDG_CACHE_HOME` and a further pile of caches — service
//! workers, extension archives, GPU state — inside `$XDG_CONFIG_HOME`
//! alongside the bookmarks. Cleaning only the first leaves hundreds of
//! megabytes behind, which is what most tools do.
//!
//! And they must be closed. Chromium holds long-lived write connections to
//! its SQLite databases; removing a file underneath one does not free the
//! space until the last descriptor closes, and it can leave the database in a
//! state Chromium responds to by discarding the whole thing. Limpid refuses
//! rather than warns.

use std::path::{Path, PathBuf};

use crate::paths::Roots;

/// Which browser lineage an installation belongs to, which decides where
/// everything is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// Chrome, Chromium, Brave, Vivaldi, Edge.
    Chromium,
    /// Firefox and its relatives.
    Firefox,
}

/// A browser found on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installation {
    /// What to call it.
    pub name: &'static str,
    /// Which lineage it belongs to.
    pub family: Family,
    /// The profile tree, under the config directory.
    pub config: PathBuf,
    /// The cache tree, under the cache directory.
    pub cache: PathBuf,
}

/// One profile within a browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// The directory name, e.g. `Default`.
    pub name: String,
    /// Where its data lives.
    pub config: PathBuf,
    /// Where its cache lives.
    pub cache: PathBuf,
}

/// Browsers Limpid knows the layout of, as `(name, family, config, cache)`
/// relative to the config and cache roots.
const KNOWN: &[(&str, Family, &str, &str)] = &[
    (
        "Brave",
        Family::Chromium,
        "BraveSoftware/Brave-Browser",
        "BraveSoftware/Brave-Browser",
    ),
    ("Chrome", Family::Chromium, "google-chrome", "google-chrome"),
    ("Chromium", Family::Chromium, "chromium", "chromium"),
    ("Vivaldi", Family::Chromium, "vivaldi", "vivaldi"),
    ("Edge", Family::Chromium, "microsoft-edge", "microsoft-edge"),
];

impl Installation {
    /// Every browser present under these roots.
    ///
    /// "Present" means it has a profile. A bare config directory is not
    /// enough: installing Chrome and never opening it leaves a directory
    /// holding nothing but a native-messaging manifest, and reporting that
    /// as a browser with nothing to clean is just noise.
    pub fn all(roots: &Roots) -> Vec<Self> {
        let mut found: Vec<Self> = KNOWN
            .iter()
            .map(|&(name, family, config, cache)| Self {
                name,
                family,
                config: roots.config(config),
                cache: roots.cache(cache),
            })
            .collect();

        found.push(Self {
            name: "Firefox",
            family: Family::Firefox,
            config: roots.home(".mozilla/firefox"),
            cache: roots.cache("mozilla/firefox"),
        });

        found.retain(|installation| !installation.profiles().is_empty());
        found
    }

    /// The profiles this browser has.
    pub fn profiles(&self) -> Vec<Profile> {
        match self.family {
            Family::Chromium => self.chromium_profiles(),
            Family::Firefox => self.firefox_profiles(),
        }
    }

    /// Chromium records its profiles in `Local State`.
    ///
    /// That file is read, never written and never removed: besides the
    /// profile list it holds `os_crypt.encrypted_key`, the wrapped key every
    /// saved cookie and password is encrypted with. Deleting it leaves the
    /// rows intact and permanently undecryptable.
    fn chromium_profiles(&self) -> Vec<Profile> {
        let names = self.chromium_profile_names();
        names
            .into_iter()
            .filter(|name| self.config.join(name).is_dir())
            .map(|name| Profile {
                config: self.config.join(&name),
                cache: self.cache.join(&name),
                name,
            })
            .collect()
    }

    /// Profile directory names, from `Local State` where possible.
    fn chromium_profile_names(&self) -> Vec<String> {
        let from_state = std::fs::read_to_string(self.config.join("Local State"))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|state| {
                let cache = state.get("profile")?.get("info_cache")?.as_object()?;
                Some(cache.keys().cloned().collect::<Vec<_>>())
            });

        match from_state {
            Some(names) if !names.is_empty() => names,
            // A profile can exist before Local State mentions it, and a
            // corrupt Local State should not hide the whole browser.
            _ => {
                let mut names = vec!["Default".to_owned()];
                if let Ok(entries) = std::fs::read_dir(&self.config) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        if name.starts_with("Profile ") {
                            names.push(name);
                        }
                    }
                }
                names.sort();
                names.dedup();
                names
            }
        }
    }

    /// Firefox records its profiles in `profiles.ini`.
    fn firefox_profiles(&self) -> Vec<Profile> {
        let Ok(text) = std::fs::read_to_string(self.config.join("profiles.ini")) else {
            return Vec::new();
        };

        parse_profiles_ini(&text)
            .into_iter()
            .map(|relative| Profile {
                config: self.config.join(&relative),
                // Firefox mirrors the profile's directory name under the
                // cache root, not the full relative path.
                cache: match Path::new(&relative).file_name() {
                    Some(leaf) => self.cache.join(leaf),
                    None => self.cache.join(&relative),
                },
                name: relative,
            })
            .filter(|profile| profile.config.is_dir())
            .collect()
    }
}

/// Pull the profile paths out of a `profiles.ini`.
///
/// Hand-rolled rather than pulled in: the file is a handful of `Key=Value`
/// lines under `[Section]` headers, and the obvious crates for it carry an
/// LGPL arm in their licence.
fn parse_profiles_ini(text: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut in_profile = false;

    for line in text.lines() {
        let line = line.trim();

        if line.starts_with('[') {
            // Only [ProfileN] sections hold profiles; [Install…] and
            // [General] point at one of them and would duplicate it.
            in_profile = line.starts_with("[Profile");
            continue;
        }

        if !in_profile {
            continue;
        }

        if let Some(value) = line.strip_prefix("Path=") {
            paths.push(value.trim().to_owned());
        }
    }

    paths
}

/// A process holding a file open inside a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    /// Its process id.
    pub pid: u32,
    /// Its command name, as the kernel reports it.
    pub name: String,
}

/// Find a process with a file open inside `directory`.
///
/// Walking `/proc/*/fd` rather than looking for a lock file, because a lock
/// file survives a crash and would report a browser that is not running.
/// Descriptors cannot lie: if one resolves into the profile, something has it
/// open right now. This also catches the renderer and zygote processes, which
/// a check on the main binary's name would miss.
///
/// Returns the first holder found; there is no value in enumerating all of
/// them when the answer either way is "close it".
pub fn holder_of(directory: &Path) -> Option<Holder> {
    let entries = std::fs::read_dir("/proc").ok()?;

    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };

        // Other users' processes are unreadable, which is fine: they cannot
        // have this user's profile open either.
        let Ok(descriptors) = std::fs::read_dir(entry.path().join("fd")) else {
            continue;
        };

        for descriptor in descriptors.flatten() {
            let Ok(target) = std::fs::read_link(descriptor.path()) else {
                continue;
            };
            if target.starts_with(directory) {
                return Some(Holder {
                    pid,
                    name: process_name(pid),
                });
            }
        }
    }

    None
}

/// The command name of a process, or a placeholder.
fn process_name(pid: u32) -> String {
    std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .map(|name| name.trim().to_owned())
        .unwrap_or_else(|_| "a process".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chromium_fixture(name: &str, profiles: &[&str]) -> (tempfile::TempDir, Roots) {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let config = roots.config(name);

        let entries: Vec<String> = profiles
            .iter()
            .map(|profile| format!("\"{profile}\": {{}}"))
            .collect();
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(
            config.join("Local State"),
            format!(
                "{{\"profile\":{{\"info_cache\":{{{}}}}}}}",
                entries.join(",")
            ),
        )
        .unwrap();
        for profile in profiles {
            std::fs::create_dir_all(config.join(profile)).unwrap();
        }

        (fixture, roots)
    }

    #[test]
    fn a_browser_with_no_profile_is_not_reported() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        // Exactly what an installed-but-never-opened Chrome leaves behind.
        std::fs::create_dir_all(roots.config("google-chrome/NativeMessagingHosts")).unwrap();

        assert!(Installation::all(&roots).is_empty());
    }

    #[test]
    fn chromium_profiles_come_from_local_state() {
        let (_fixture, roots) =
            chromium_fixture("BraveSoftware/Brave-Browser", &["Default", "Profile 1"]);

        let installed = Installation::all(&roots);
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].name, "Brave");

        let profiles = installed[0].profiles();
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].name, "Default");
        assert!(
            profiles[0]
                .cache
                .ends_with("BraveSoftware/Brave-Browser/Default")
        );
    }

    #[test]
    fn a_profile_listed_but_absent_is_dropped() {
        let (_fixture, roots) = chromium_fixture("chromium", &["Default", "Profile 9"]);
        std::fs::remove_dir_all(roots.config("chromium/Profile 9")).unwrap();

        assert_eq!(Installation::all(&roots)[0].profiles().len(), 1);
    }

    #[test]
    fn a_corrupt_local_state_does_not_hide_the_browser() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let config = roots.config("chromium");
        std::fs::create_dir_all(config.join("Default")).unwrap();
        std::fs::create_dir_all(config.join("Profile 2")).unwrap();
        std::fs::write(config.join("Local State"), "{ this is not json").unwrap();

        let profiles = Installation::all(&roots)[0].profiles();

        assert_eq!(profiles.len(), 2);
    }

    #[test]
    fn firefox_profiles_come_from_profiles_ini() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let config = roots.home(".mozilla/firefox");
        std::fs::create_dir_all(config.join("abc123.default-release")).unwrap();
        std::fs::write(
            config.join("profiles.ini"),
            "[Install4F96D1932A9F858E]\n\
             Default=abc123.default-release\n\
             \n\
             [Profile0]\n\
             Name=default-release\n\
             IsRelative=1\n\
             Path=abc123.default-release\n\
             Default=1\n\
             \n\
             [General]\n\
             StartWithLastProfile=1\n",
        )
        .unwrap();

        let installed = Installation::all(&roots);
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].family, Family::Firefox);

        let profiles = installed[0].profiles();
        assert_eq!(profiles.len(), 1);
        assert!(
            profiles[0]
                .cache
                .ends_with("mozilla/firefox/abc123.default-release")
        );
    }

    #[test]
    fn only_profile_sections_of_an_ini_are_read() {
        // The [Install…] section repeats a path that [Profile0] already
        // gave; counting both would clean the same profile twice.
        let text = "[Install1]\nDefault=one\n[Profile0]\nPath=one\n[Profile1]\nPath=two\n";

        assert_eq!(parse_profiles_ini(text), vec!["one", "two"]);
    }

    #[test]
    fn an_empty_ini_yields_nothing() {
        assert!(parse_profiles_ini("").is_empty());
        assert!(parse_profiles_ini("[General]\nStartWithLastProfile=1\n").is_empty());
    }

    #[test]
    fn a_directory_nothing_has_open_has_no_holder() {
        let fixture = tempfile::tempdir().unwrap();
        assert_eq!(holder_of(fixture.path()), None);
    }

    #[test]
    fn a_directory_with_an_open_file_reports_this_process() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("Cookies");
        let _open = std::fs::File::create(&path).unwrap();

        let holder = holder_of(fixture.path()).expect("this process holds it open");

        assert_eq!(holder.pid, std::process::id());
        assert!(!holder.name.is_empty());
    }
}
