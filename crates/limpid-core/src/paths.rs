//! Where to look.
//!
//! Every path a scanner uses is resolved through [`Roots`], so the whole
//! engine can be pointed at a fixture directory. Without that, testing a
//! cleaner means either mocking the filesystem or letting the suite loose on
//! the developer's real home directory.

use std::path::{Path, PathBuf};

/// The directory prefixes every scan is resolved against.
#[derive(Debug, Clone)]
pub struct Roots {
    /// The user's home directory.
    pub home: PathBuf,
    /// `$XDG_CACHE_HOME`, usually `~/.cache`.
    pub cache: PathBuf,
    /// `$XDG_CONFIG_HOME`, usually `~/.config`.
    pub config: PathBuf,
    /// `$XDG_DATA_HOME`, usually `~/.local/share`.
    pub data: PathBuf,
    /// `$XDG_STATE_HOME`, usually `~/.local/state`.
    pub state: PathBuf,
    /// What to treat as `/`. Only ever anything else under test.
    pub system: PathBuf,
}

/// Environment variable that relocates every root, for tests and dry runs
/// against a captured filesystem.
pub const ROOT_OVERRIDE: &str = "LIMPID_ROOT";

impl Roots {
    /// Resolve roots from the environment.
    ///
    /// When `LIMPID_ROOT` is set, the home directory becomes `$LIMPID_ROOT/home`
    /// and the system root becomes `$LIMPID_ROOT/system`, with the XDG
    /// directories derived from the former. `XDG_*` variables are deliberately
    /// ignored in that mode so a fixture is not perturbed by the developer's
    /// own environment.
    pub fn from_env() -> Self {
        match std::env::var_os(ROOT_OVERRIDE) {
            Some(prefix) => Self::under(Path::new(&prefix)),
            None => Self::real(),
        }
    }

    /// Roots for the machine Limpid is actually running on.
    pub fn real() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        Self {
            cache: dirs::cache_dir().unwrap_or_else(|| home.join(".cache")),
            config: dirs::config_dir().unwrap_or_else(|| home.join(".config")),
            data: dirs::data_dir().unwrap_or_else(|| home.join(".local/share")),
            state: dirs::state_dir().unwrap_or_else(|| home.join(".local/state")),
            system: PathBuf::from("/"),
            home,
        }
    }

    /// Roots rehomed under `prefix`, for tests.
    pub fn under(prefix: &Path) -> Self {
        let home = prefix.join("home");
        Self {
            cache: home.join(".cache"),
            config: home.join(".config"),
            data: home.join(".local/share"),
            state: home.join(".local/state"),
            system: prefix.join("system"),
            home,
        }
    }

    /// A path inside the cache directory.
    pub fn cache(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.cache.join(relative)
    }

    /// A path inside the config directory.
    pub fn config(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.config.join(relative)
    }

    /// A path inside the data directory.
    pub fn data(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.data.join(relative)
    }

    /// A path inside the home directory.
    pub fn home(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.home.join(relative)
    }

    /// An absolute system path, rehomed when running against a fixture.
    ///
    /// Takes the path as it would be written on a real machine — leading
    /// slash and all — so call sites read like the documentation they came
    /// from: `roots.system("var/cache/pacman/pkg")`.
    pub fn system(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.system.join(
            relative
                .as_ref()
                .strip_prefix("/")
                .unwrap_or(relative.as_ref()),
        )
    }

    /// Whether these roots point at a fixture rather than the real machine.
    pub fn is_sandboxed(&self) -> bool {
        self.system != Path::new("/")
    }
}

impl Default for Roots {
    fn default() -> Self {
        Self::from_env()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prefix_rehomes_every_root() {
        let roots = Roots::under(Path::new("/fixture"));

        assert_eq!(roots.home, Path::new("/fixture/home"));
        assert_eq!(roots.cache, Path::new("/fixture/home/.cache"));
        assert_eq!(roots.data, Path::new("/fixture/home/.local/share"));
        assert_eq!(roots.system, Path::new("/fixture/system"));
        assert!(roots.is_sandboxed());
    }

    #[test]
    fn system_paths_are_rehomed_with_or_without_a_leading_slash() {
        let roots = Roots::under(Path::new("/fixture"));

        assert_eq!(
            roots.system("/var/cache/pacman/pkg"),
            roots.system("var/cache/pacman/pkg")
        );
        assert_eq!(
            roots.system("/var/log"),
            Path::new("/fixture/system/var/log")
        );
    }

    #[test]
    fn real_roots_are_not_sandboxed() {
        assert!(!Roots::real().is_sandboxed());
    }
}
