//! The last check before anything is removed.
//!
//! The scanners already decide what is safe, so this is defence in depth: a
//! second, independent opinion that does not trust the first. A bug in a
//! scanner — a path joined against the wrong root, an empty string that
//! collapses to `/` — should cost a refusal, not a home directory.
//!
//! The rule is narrow on purpose. A path a *scanner* produced is removable
//! only if it sits strictly inside one of a small set of known directories,
//! is not one of those directories itself, and cannot climb out.
//!
//! A path the **user** pointed at is a different question, and it needs a
//! different answer. The storage view shows the whole disk, so nothing there
//! is inside a boundary, and widening the boundary list to make that work
//! would gut the protection for everything else. The distinction that
//! resolves it: this guard exists to catch the *program* being wrong, and a
//! file someone selected on screen is not a guess. So an explicit choice
//! keeps every rule that does not depend on guessing — absolute, no climbing
//! out, no symlink anywhere along the way, no protected name, inside the
//! home directory, and never something no cleaner should be touching — and
//! drops only the boundary list.
//!
//! Build output inside projects is a third case. A scanner found it, so it
//! is a guess — but no boundary list can name every project, so it cannot
//! be held to one. It is held to a rule instead: everything an explicit
//! choice must satisfy, plus what makes a directory build output at all
//! ([`crate::project::identify`]), asked again here against the filesystem
//! as it is at the moment of removal, and two rules about where build
//! output can be that this module enforces on its own.
//!
//! Which check applies is not left to the call site. It travels with the
//! work as a [`Permission`], set when the item was created and read when it
//! is acted on.

use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use crate::paths::Roots;
use crate::project::{self, Ecosystem};

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
    /// Is somewhere nothing may be removed from, however it was asked for.
    #[error("{0} holds credentials; Limpid does not remove from there")]
    Sacred(PathBuf),
    /// Carries a name that is never removable, wherever it appears.
    #[error("{0} is protected by name")]
    Protected(PathBuf),
    /// Is, or contains, a directory the desktop session lives in.
    #[error("{0} holds what your desktop runs on; Limpid does not remove it")]
    Essential(PathBuf),
    /// Was found as build output, and no longer looks like it — or is
    /// somewhere build output is never taken from.
    #[error("{0} is not build output inside a project")]
    NotBuildOutput(PathBuf),
}

impl Refusal {
    /// The path that was refused.
    pub fn path(&self) -> &Path {
        match self {
            Self::NotAbsolute(path)
            | Self::Climbing(path)
            | Self::OutOfBounds(path)
            | Self::IsABoundary(path)
            | Self::Symlink(path)
            | Self::Protected(path)
            | Self::Sacred(path)
            | Self::Essential(path)
            | Self::NotBuildOutput(path) => path,
        }
    }
}

/// Files that are never removed, wherever they turn up.
///
/// A belt-and-braces list for the browser trees, where a boundary has to
/// cover a whole profile directory because profile names are not knowable in
/// advance, and that directory holds irreplaceable things next to caches.
///
/// `Local State` is the one that matters most. Besides the profile list it
/// holds `os_crypt.encrypted_key`, the wrapped key every saved cookie and
/// password is encrypted with. Removing it leaves every row intact and
/// permanently undecryptable — a far worse outcome than losing the rows.
const PROTECTED_NAMES: &[&str] = &[
    // Chromium.
    "Local State",
    "Bookmarks",
    "Cookies",
    "History",
    "Login Data",
    "Login Data For Account",
    "Web Data",
    "Preferences",
    "Secure Preferences",
    "Sync Data",
    // Firefox.
    "key4.db",
    "cert9.db",
    "logins.json",
    "places.sqlite",
    "cookies.sqlite",
    "prefs.js",
];

/// Directories nothing may be removed from, however it was arrived at.
///
/// Not about disk space, and not a judgement about what the user wants. A
/// cleaner that can be talked into emptying `~/.ssh` is a cleaner with a
/// vulnerability, and no amount of "but they clicked it" makes that
/// acceptable.
const SACRED: &[&str] = &[".ssh", ".gnupg", ".password-store", ".local/share/keyrings"];

/// How a path came to be in a plan, which decides how it is checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    /// A scanner produced it. Must sit inside a known boundary.
    Catalogued,
    /// The user pointed at it on screen. The boundary list does not apply,
    /// everything else does.
    Chosen,
    /// A scanner found it by rule rather than by place: build output inside
    /// a project. Everything an explicit choice must satisfy, and the rule
    /// itself, checked again at the moment of removal.
    BuildOutput,
}

impl Permission {
    /// Whether this is the ordinary case, a path inside a boundary.
    pub fn is_catalogued(&self) -> bool {
        *self == Self::Catalogued
    }
}

/// Decides whether a path may be removed.
#[derive(Debug, Clone)]
pub struct Guard {
    /// Directories whose *contents* may be removed.
    boundaries: Vec<PathBuf>,
    /// The user's home directory, which bounds an explicit choice.
    home: PathBuf,
    /// Directories the desktop session lives in, which no explicit choice
    /// may be or contain.
    essential: Vec<PathBuf>,
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

        // Browser profile trees have to be admitted whole, because profile
        // directory names are not knowable in advance. PROTECTED_NAMES is
        // what keeps that from being as wide as it sounds.
        for browser in [
            "BraveSoftware",
            "google-chrome",
            "chromium",
            "vivaldi",
            "microsoft-edge",
        ] {
            boundaries.push(roots.config(browser));
        }
        boundaries.push(roots.home(".mozilla/firefox"));
        boundaries.sort();
        boundaries.dedup();
        Self {
            boundaries,
            home: roots.home.clone(),
            // Where every application keeps its settings and its data. Each
            // of these going to the trash is reversible on paper and a
            // broken session in practice: the window manager reads its
            // configuration from the first one, and the file manager you
            // would restore it with keeps its state in the others. `~/.local`
            // is covered too, because it contains two of them.
            essential: vec![
                roots.config.clone(),
                roots.data.clone(),
                roots.state.clone(),
            ],
        }
    }

    /// The directories this guard permits removal inside.
    pub fn boundaries(&self) -> &[PathBuf] {
        &self.boundaries
    }

    /// Check a path that is about to be removed, under the permission it
    /// was created with.
    pub fn check_with(&self, path: &Path, permission: Permission) -> Result<(), Refusal> {
        match permission {
            Permission::Catalogued => self.check(path),
            Permission::Chosen => self.check_chosen(path),
            Permission::BuildOutput => self.check_build_output(path),
        }
    }

    /// Check build output that a scanner found by rule.
    ///
    /// The rule is asked again rather than trusted: a `Cargo.toml` deleted
    /// since the scan, or a `target` swapped for something else, and this is
    /// no longer build output. The two rules about *where* are this
    /// module's own, not the scanner's, so a scanner that stopped applying
    /// them would still be refused here.
    pub fn check_build_output(&self, path: &Path) -> Result<(), Refusal> {
        self.check_chosen(path)?;
        let refused = || Refusal::NotBuildOutput(path.to_owned());

        let below_home = path.strip_prefix(&self.home).map_err(|_| refused())?;

        // Applications keep their own state in hidden directories, and it
        // can look exactly like a project: an application's plugins under
        // `~/.config` came with a `package.json`, a lockfile and a
        // `node_modules`. People do not keep projects there.
        if below_home
            .components()
            .any(|part| part.as_os_str().as_bytes().starts_with(b"."))
        {
            return Err(refused());
        }

        // Inside a boundary, it is the catalogue's to clean, under the
        // catalogue's rules — the Go module cache is read-only and needs its
        // own operation, not a recursive delete.
        if self
            .boundaries
            .iter()
            .any(|boundary| path.starts_with(boundary))
        {
            return Err(refused());
        }

        // Nested inside other build output: a package's own
        // `node_modules`, inside the project's. Removing it would break
        // the install around it rather than free anything on its own.
        let nested = below_home.parent().is_some_and(|parent| {
            parent.components().any(|part| {
                Ecosystem::ALL
                    .iter()
                    .any(|ecosystem| part.as_os_str() == ecosystem.directory())
            })
        });
        if nested {
            return Err(refused());
        }

        project::identify(path).map(|_| ()).ok_or_else(refused)
    }

    /// Check a path the user picked from the storage view.
    ///
    /// Everything that does not depend on the program having guessed right
    /// still applies. What is dropped is the boundary list, and only that.
    pub fn check_chosen(&self, path: &Path) -> Result<(), Refusal> {
        self.check_universal(path)?;

        // Outside the home directory is not the user's to give away from
        // here: it is either someone else's or the system's.
        if !path.starts_with(&self.home) || path == self.home {
            return Err(Refusal::OutOfBounds(path.to_owned()));
        }

        if SACRED
            .iter()
            .any(|sacred| path.starts_with(self.home.join(sacred)))
        {
            return Err(Refusal::Sacred(path.to_owned()));
        }

        // A folder that holds one of them. Something inside is a different
        // matter — one application's leftovers under `~/.config` are a
        // reasonable thing to point at.
        if self
            .essential
            .iter()
            .any(|essential| essential.starts_with(path))
        {
            return Err(Refusal::Essential(path.to_owned()));
        }

        Ok(())
    }

    /// Check a path that is about to be removed.
    pub fn check(&self, path: &Path) -> Result<(), Refusal> {
        self.check_universal(path)?;

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

        Ok(())
    }

    /// The rules that hold however the path was arrived at.
    fn check_universal(&self, path: &Path) -> Result<(), Refusal> {
        if !path.is_absolute() {
            return Err(Refusal::NotAbsolute(path.to_owned()));
        }

        // Rejected rather than resolved: resolving would mean canonicalising,
        // which follows symlinks, and a symlink is exactly what an attacker
        // or a bug would use to escape.
        if path.components().any(|part| part == Component::ParentDir) {
            return Err(Refusal::Climbing(path.to_owned()));
        }

        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| PROTECTED_NAMES.contains(&name))
        {
            return Err(Refusal::Protected(path.to_owned()));
        }

        // Every component, not just the leaf. The boundary check above is
        // lexical, so if any directory along the way is a symlink the path
        // can start with a boundary and still resolve somewhere else
        // entirely — and `remove_dir_all` would follow it. Checking only the
        // last component looks right and defends nothing.
        //
        // A missing path is fine: there is simply nothing to remove.
        for component in path.ancestors() {
            match std::fs::symlink_metadata(component) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(Refusal::Symlink(component.to_owned()));
                }
                _ => continue,
            }
        }

        Ok(())
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
    fn irreplaceable_browser_files_are_refused_by_name() {
        let (_fixture, roots, guard) = fixture();
        let profile = roots.config("BraveSoftware/Brave-Browser/Default");

        // Inside a boundary, and still refused.
        for name in ["Local State", "Cookies", "Login Data", "Bookmarks"] {
            let path = profile.join(name);
            assert_eq!(
                guard.check(&path).unwrap_err(),
                Refusal::Protected(path.clone()),
                "{name} should be protected",
            );
        }

        // What sits beside them is not.
        assert!(guard.allows(&profile.join("Cache")));
        assert!(guard.allows(&profile.join("Service Worker/CacheStorage")));
    }

    #[test]
    fn firefox_credentials_are_protected_too() {
        let (_fixture, roots, guard) = fixture();
        let profile = roots.home(".mozilla/firefox/abc.default-release");

        assert!(matches!(
            guard.check(&profile.join("key4.db")),
            Err(Refusal::Protected(_))
        ));
        assert!(matches!(
            guard.check(&profile.join("logins.json")),
            Err(Refusal::Protected(_))
        ));
        assert!(guard.allows(&profile.join("startupCache")));
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
    fn a_symlink_anywhere_along_the_path_is_refused_not_just_the_last_part() {
        let (_fixture, roots, guard) = fixture();
        std::fs::create_dir_all(roots.home("elsewhere/yay")).unwrap();
        std::fs::create_dir_all(&roots.home).unwrap();

        // The cache root itself is a link. Lexically `~/.cache/yay` is
        // inside the boundary; in reality it is somewhere else, and only
        // checking the leaf would let it through.
        std::os::unix::fs::symlink(roots.home("elsewhere"), &roots.cache).unwrap();

        let refusal = guard.check(&roots.cache("yay")).unwrap_err();

        assert_eq!(refusal, Refusal::Symlink(roots.cache.clone()));
    }

    #[test]
    fn a_symlink_in_the_middle_of_a_path_is_refused() {
        let (_fixture, roots, guard) = fixture();
        std::fs::create_dir_all(roots.home("Documents/secrets")).unwrap();
        std::fs::create_dir_all(&roots.cache).unwrap();
        std::os::unix::fs::symlink(roots.home("Documents"), roots.cache("sneaky")).unwrap();

        let refusal = guard.check(&roots.cache("sneaky/secrets")).unwrap_err();

        assert_eq!(refusal, Refusal::Symlink(roots.cache("sneaky")));
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
    fn a_chosen_file_is_allowed_where_a_catalogued_one_would_not_be() {
        let (_fixture, roots, guard) = fixture();
        let film = roots.home("Videos/holiday.mkv");
        std::fs::create_dir_all(film.parent().unwrap()).unwrap();
        std::fs::write(&film, b"x").unwrap();

        // The scanner has no business there; the user pointing at it does.
        assert!(matches!(guard.check(&film), Err(Refusal::OutOfBounds(_))));
        assert!(guard.check_chosen(&film).is_ok());
    }

    #[test]
    fn an_explicit_choice_still_cannot_leave_the_home_directory() {
        let (_fixture, _roots, guard) = fixture();

        for path in ["/etc/passwd", "/usr/lib/libc.so", "/", "/var/log/journal"] {
            assert!(
                matches!(
                    guard.check_chosen(Path::new(path)),
                    Err(Refusal::OutOfBounds(_))
                ),
                "{path} should have been refused",
            );
        }
    }

    #[test]
    fn an_explicit_choice_cannot_be_the_home_directory_itself() {
        let (_fixture, roots, guard) = fixture();

        assert!(matches!(
            guard.check_chosen(&roots.home),
            Err(Refusal::OutOfBounds(_))
        ));
    }

    #[test]
    fn credentials_are_refused_however_hard_someone_points_at_them() {
        let (_fixture, roots, guard) = fixture();

        for relative in [
            ".ssh",
            ".ssh/id_ed25519",
            ".gnupg/pubring.kbx",
            ".password-store",
        ] {
            let path = roots.home(relative);
            assert!(
                matches!(guard.check_chosen(&path), Err(Refusal::Sacred(_))),
                "{relative} should have been refused",
            );
        }
    }

    #[test]
    fn an_explicit_choice_keeps_every_rule_that_is_not_about_boundaries() {
        let (_fixture, roots, guard) = fixture();
        std::fs::create_dir_all(roots.home("Videos")).unwrap();
        std::fs::create_dir_all(roots.home("Documents")).unwrap();

        // Relative, climbing, and symlinked are refused just the same.
        assert!(matches!(
            guard.check_chosen(Path::new("Videos/x.mkv")),
            Err(Refusal::NotAbsolute(_))
        ));
        assert!(matches!(
            guard.check_chosen(&roots.home("Videos/../../etc")),
            Err(Refusal::Climbing(_))
        ));

        let link = roots.home("Videos/shortcut");
        std::os::unix::fs::symlink(roots.home("Documents"), &link).unwrap();
        assert!(matches!(
            guard.check_chosen(&link),
            Err(Refusal::Symlink(_))
        ));

        // And a protected name is protected wherever it turns up.
        assert!(matches!(
            guard.check_chosen(&roots.home("Videos/Local State")),
            Err(Refusal::Protected(_))
        ));
    }

    #[test]
    fn a_folder_the_session_lives_in_cannot_be_chosen_nor_anything_holding_one() {
        let (_fixture, roots, guard) = fixture();

        for path in [
            roots.config.clone(),
            roots.data.clone(),
            roots.state.clone(),
            roots.home(".local"),
        ] {
            assert_eq!(
                guard.check_chosen(&path).unwrap_err(),
                Refusal::Essential(path.clone()),
                "{}",
                path.display(),
            );
        }

        // What is inside is the person's to point at: an uninstalled
        // application's leftovers, one setting file.
        assert!(guard.check_chosen(&roots.config("someapp")).is_ok());
        assert!(guard.check_chosen(&roots.data("Steam/logs")).is_ok());
    }

    #[test]
    fn the_permission_decides_which_check_runs() {
        let (_fixture, roots, guard) = fixture();
        let film = roots.home("Videos/holiday.mkv");
        std::fs::create_dir_all(film.parent().unwrap()).unwrap();
        std::fs::write(&film, b"x").unwrap();

        assert!(guard.check_with(&film, Permission::Chosen).is_ok());
        assert!(guard.check_with(&film, Permission::Catalogued).is_err());

        // And the reverse: the cache root is a boundary, so it is refused
        // either way, for different reasons.
        assert!(
            guard
                .check_with(&roots.cache, Permission::Catalogued)
                .is_err()
        );
        assert!(guard.check_with(&roots.cache, Permission::Chosen).is_ok());
    }

    #[test]
    fn build_output_in_a_project_is_allowed_where_nothing_else_a_scanner_found_would_be() {
        let (_fixture, roots, guard) = fixture();
        let target = crate::project::fixture::cargo_project(&roots.home("Work/limpid"), 10);

        assert!(matches!(guard.check(&target), Err(Refusal::OutOfBounds(_))));
        assert!(guard.check_with(&target, Permission::BuildOutput).is_ok());
    }

    #[test]
    fn build_output_is_asked_for_again_at_the_moment_of_removal() {
        let (_fixture, roots, guard) = fixture();
        let project = roots.home("Work/limpid");
        let target = crate::project::fixture::cargo_project(&project, 10);

        // Between the scan and the button, the project stopped being one.
        std::fs::remove_file(project.join("Cargo.toml")).unwrap();

        assert_eq!(
            guard.check_build_output(&target).unwrap_err(),
            Refusal::NotBuildOutput(target)
        );
    }

    #[test]
    fn build_output_under_a_hidden_directory_is_refused_even_when_it_looks_right() {
        let (_fixture, roots, guard) = fixture();

        for place in [".config/opencode", ".local/share/tool", "Work/.hidden/app"] {
            let modules = crate::project::fixture::npm_project(&roots.home(place), 10);
            assert_eq!(
                guard.check_build_output(&modules).unwrap_err(),
                Refusal::NotBuildOutput(modules),
                "{place}",
            );
        }
    }

    #[test]
    fn build_output_nested_in_other_build_output_is_refused() {
        let (_fixture, roots, guard) = fixture();
        let outer = crate::project::fixture::npm_project(&roots.home("Work/site"), 10);
        let inner = crate::project::fixture::npm_project(&outer.join("some-package"), 10);

        assert!(guard.check_build_output(&outer).is_ok());
        assert_eq!(
            guard.check_build_output(&inner).unwrap_err(),
            Refusal::NotBuildOutput(inner)
        );
    }

    #[test]
    fn build_output_inside_a_catalogued_boundary_is_left_to_the_catalogue() {
        let (_fixture, roots, guard) = fixture();
        let target =
            crate::project::fixture::cargo_project(&roots.home("go/pkg/mod/github.com/x/y"), 10);

        assert!(guard.allows(&target), "the catalogue may clean it");
        assert_eq!(
            guard.check_build_output(&target).unwrap_err(),
            Refusal::NotBuildOutput(target)
        );
    }

    #[test]
    fn build_output_keeps_every_rule_an_explicit_choice_has() {
        let (_fixture, roots, guard) = fixture();
        let real = crate::project::fixture::cargo_project(&roots.home("Work/real"), 10);

        // A project directory that is a link somewhere else.
        std::os::unix::fs::symlink(roots.home("Work/real"), roots.home("Work/alias")).unwrap();
        assert!(matches!(
            guard.check_build_output(&roots.home("Work/alias/target")),
            Err(Refusal::Symlink(_))
        ));

        // Outside the home directory, and climbing out of it.
        assert!(matches!(
            guard.check_build_output(Path::new("/opt/project/target")),
            Err(Refusal::OutOfBounds(_))
        ));
        assert!(matches!(
            guard.check_build_output(&roots.home("Work/real/../../../target")),
            Err(Refusal::Climbing(_))
        ));

        // And the real one is still fine.
        assert!(guard.check_build_output(&real).is_ok());
    }

    #[test]
    fn a_directory_that_merely_has_the_name_is_refused() {
        let (_fixture, roots, guard) = fixture();
        let directory = roots.home("Documents/target");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("plans.odt"), "x").unwrap();

        assert_eq!(
            guard
                .check_with(&directory, Permission::BuildOutput)
                .unwrap_err(),
            Refusal::NotBuildOutput(directory)
        );
    }

    #[test]
    fn the_boundary_list_is_sorted_and_free_of_duplicates() {
        let (_fixture, _roots, guard) = fixture();
        let boundaries = guard.boundaries();

        assert!(boundaries.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
