//! The last check before anything is removed.
//!
//! The scanners already decide what is safe, so this is defence in depth: a
//! second, independent opinion that does not trust the first. A bug in a
//! scanner — a path joined against the wrong root, an empty string that
//! collapses to `/` — should cost a refusal, not a home directory.
//!
//! The rule is narrow on purpose. A path is removable only if it sits
//! strictly inside one of a small set of known directories, is not one of
//! those directories itself, and cannot climb out.

use std::path::{Component, Path, PathBuf};

use crate::paths::Roots;

/// Why a path may not be removed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// Not an absolute path.
    #[error("{0} is not an absolute path")]
    NotAbsolute(PathBuf),
    /// Contains `..`, which could climb out of an allowed directory.
    #[error("{0} contains a parent-directory component")]
    Climbing(PathBuf),
    /// Not inside anything Limpid is allowed to touch.
    #[error("{0} is outside every directory Limpid may remove from")]
    OutOfBounds(PathBuf),
    /// Is one of the directories that bound what may be removed.
    #[error("{0} is a directory Limpid removes from, not one it removes")]
    IsABoundary(PathBuf),
    /// Is a symlink, which would act on something elsewhere.
    #[error("{0} is a symlink")]
    Symlink(PathBuf),
}

impl Refusal {
    /// The path that was refused.
    pub fn path(&self) -> &Path {
        match self {
            Self::NotAbsolute(path)
            | Self::Climbing(path)
            | Self::OutOfBounds(path)
            | Self::IsABoundary(path)
            | Self::Symlink(path) => path,
        }
    }
}

/// Decides whether a path may be removed.
#[derive(Debug, Clone)]
pub struct Guard {
    /// Directories whose *contents* may be removed.
    boundaries: Vec<PathBuf>,
}

impl Guard {
    /// Build the guard for a set of roots.
    ///
    /// The list is written out rather than derived from what the scanners
    /// declare, so that adding a scanner cannot widen it by accident. A new
    /// place to clean has to be granted here, deliberately, as well.
    pub fn new(roots: &Roots) -> Self {
        let mut boundaries = vec![
            roots.cache.clone(),
            roots.data.join("Trash"),
            roots.home(".npm"),
            roots.home(".cargo/registry"),
            roots.home(".yarn"),
            roots.home("go/pkg/mod"),
            roots.home(".var/app"),
            roots.system("/var/cache/pacman/pkg"),
            roots.system("/var/log/journal"),
            roots.system("/var/lib/systemd/coredump"),
        ];
        boundaries.sort();
        boundaries.dedup();
        Self { boundaries }
    }

    /// The directories this guard permits removal inside.
    pub fn boundaries(&self) -> &[PathBuf] {
        &self.boundaries
    }

    /// Check a path that is about to be removed.
    pub fn check(&self, path: &Path) -> Result<(), Refusal> {
        if !path.is_absolute() {
            return Err(Refusal::NotAbsolute(path.to_owned()));
        }

        // Rejected rather than resolved: resolving would mean canonicalising,
        // which follows symlinks, and a symlink is exactly what an attacker
        // or a bug would use to escape.
        if path.components().any(|part| part == Component::ParentDir) {
            return Err(Refusal::Climbing(path.to_owned()));
        }

        if self.boundaries.iter().any(|boundary| boundary == path) {
            return Err(Refusal::IsABoundary(path.to_owned()));
        }

        if !self
            .boundaries
            .iter()
            .any(|boundary| path.starts_with(boundary))
        {
            return Err(Refusal::OutOfBounds(path.to_owned()));
        }

        // `symlink_metadata` does not follow, so this sees the link itself.
        // A missing path is fine: there is simply nothing to remove.
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Err(Refusal::Symlink(path.to_owned()))
            }
            _ => Ok(()),
        }
    }

    /// Whether a path would be accepted.
    pub fn allows(&self, path: &Path) -> bool {
        self.check(path).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Roots, Guard) {
        let directory = tempfile::tempdir().unwrap();
        let roots = Roots::under(directory.path());
        let guard = Guard::new(&roots);
        (directory, roots, guard)
    }

    #[test]
    fn a_path_inside_a_boundary_is_allowed() {
        let (_fixture, roots, guard) = fixture();

        assert!(guard.allows(&roots.cache("thumbnails")));
        assert!(guard.allows(&roots.cache("yay/brave-bin/src")));
        assert!(guard.allows(&roots.system("/var/cache/pacman/pkg/linux.pkg.tar.zst")));
    }

    #[test]
    fn the_boundaries_themselves_are_refused() {
        let (_fixture, roots, guard) = fixture();

        // Removing ~/.cache wholesale is exactly the mistake this exists to
        // stop, and it is one component away from something legitimate.
        assert_eq!(
            guard.check(&roots.cache).unwrap_err(),
            Refusal::IsABoundary(roots.cache)
        );
    }

    #[test]
    fn everything_outside_the_boundaries_is_refused() {
        let (_fixture, roots, guard) = fixture();

        for path in [
            PathBuf::from("/"),
            PathBuf::from("/etc"),
            PathBuf::from("/usr/lib"),
            PathBuf::from("/home"),
            roots.home.clone(),
            roots.config.clone(),
            roots.home("Documents"),
            roots.home(".ssh/id_ed25519"),
            roots.data.clone(),
            roots.system("/var/log"),
            roots.system("/boot"),
            roots.system("/.snapshots/1"),
        ] {
            assert!(
                matches!(
                    guard.check(&path),
                    Err(Refusal::OutOfBounds(_) | Refusal::IsABoundary(_))
                ),
                "{} should have been refused",
                path.display(),
            );
        }
    }

    #[test]
    fn a_relative_path_is_refused() {
        let (_fixture, _roots, guard) = fixture();

        assert!(matches!(
            guard.check(Path::new(".cache/thumbnails")),
            Err(Refusal::NotAbsolute(_))
        ));
    }

    #[test]
    fn climbing_out_with_a_parent_component_is_refused() {
        let (_fixture, roots, guard) = fixture();

        // Lexically this is inside the cache; resolved it is the home
        // directory. Refused before anyone has to work out which.
        let escape = roots.cache("../../..");
        assert!(matches!(guard.check(&escape), Err(Refusal::Climbing(_))));

        let subtle = roots.cache("yay/../../.ssh");
        assert!(matches!(guard.check(&subtle), Err(Refusal::Climbing(_))));
    }

    #[test]
    fn a_symlink_is_refused_even_inside_a_boundary() {
        let (_fixture, roots, guard) = fixture();
        std::fs::create_dir_all(&roots.cache).unwrap();
        std::fs::create_dir_all(roots.home("Documents")).unwrap();

        let link = roots.cache("looks-like-a-cache");
        std::os::unix::fs::symlink(roots.home("Documents"), &link).unwrap();

        assert_eq!(guard.check(&link).unwrap_err(), Refusal::Symlink(link));
    }

    #[test]
    fn a_path_that_does_not_exist_is_allowed_because_there_is_nothing_to_do() {
        let (_fixture, roots, guard) = fixture();

        assert!(guard.allows(&roots.cache("never-existed")));
    }

    #[test]
    fn a_boundary_prefix_does_not_admit_a_sibling_with_the_same_start() {
        let (_fixture, roots, guard) = fixture();

        // `.cache-backup` starts with the same characters as `.cache` but is
        // a different directory; `starts_with` on paths compares components,
        // which is why this is refused.
        let sibling = roots.home(".cache-backup/important");
        assert!(matches!(
            guard.check(&sibling),
            Err(Refusal::OutOfBounds(_))
        ));
    }

    #[test]
    fn the_boundary_list_is_sorted_and_free_of_duplicates() {
        let (_fixture, _roots, guard) = fixture();
        let boundaries = guard.boundaries();

        assert!(boundaries.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
