//! Build output inside projects.
//!
//! The one kind of finding whose location nobody can list in advance:
//! projects live wherever their authors put them. So instead of a place
//! there is a rule. A directory is build output when the tool that made it
//! left a mark in two places — a manifest *beside* it, saying this is a
//! project of that kind, and evidence *inside* it, saying the tool wrote it.
//!
//! Two signals from two directions, because either alone has been wrong on
//! a real machine. A directory named `target` inside a crate's unpacked
//! sources was source code, with no manifest beside it. And a
//! `node_modules` with a `package.json` and a lockfile beside it was not a
//! project at all but an application's own installation, under `~/.config`
//! — which is why [`discover`] never looks inside a hidden directory, and
//! why the guard refuses one independently.
//!
//! [`identify`] is also what the guard asks again at the moment of removal,
//! against the filesystem as it is then.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use crate::config::Exclusions;

/// How deep below the home directory a project is looked for.
///
/// `~/Work/org/repository/crates/member` is five levels down. Twelve leaves
/// room for monorepos while keeping a pathological tree from turning a scan
/// into minutes.
const MAX_DEPTH: usize = 12;

/// Directories below home that are not projects, though nothing about their
/// names says so.
///
/// The Go module cache is stored read-only and has its own target; snap
/// keeps each application's data under `~/snap`, visibly, where a
/// `node_modules` would be the application's and not the person's.
const NOT_PROJECTS: &[&str] = &["go/pkg", "snap"];

/// The first line of a `CACHEDIR.TAG`, from the Cache Directory Tagging
/// Specification. A directory carrying it has declared itself regenerable.
const CACHEDIR_SIGNATURE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

/// Lockfiles that make a `package.json` a project with installed packages.
const NODE_LOCKFILES: &[&str] = &[
    "package-lock.json",
    "npm-shrinkwrap.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lock",
    "bun.lockb",
];

/// Files a package manager leaves inside `node_modules` when it installs.
///
/// pnpm's `.modules.yaml` is deliberately absent. pnpm fills `node_modules`
/// with hardlinks into one store shared by every project on the machine, so
/// removing it frees almost nothing — and the figure measured for it would
/// promise the full size. A package manager that leaves none of these is
/// not offered at all: better nothing than a guess.
const NODE_INSTALL_STATE: &[&str] = &[
    // npm 7 and later.
    ".package-lock.json",
    // Yarn 1.
    ".yarn-integrity",
    // Yarn 2 and later, with the node-modules linker.
    ".yarn-state.yml",
];

/// What made a build output directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Ecosystem {
    /// A Rust project's `target/`.
    Cargo,
    /// A JavaScript project's `node_modules/`.
    Node,
}

impl Ecosystem {
    /// Every ecosystem Limpid recognises.
    pub const ALL: [Self; 2] = [Self::Cargo, Self::Node];

    /// The name its build output directory has.
    pub fn directory(self) -> &'static str {
        match self {
            Self::Cargo => "target",
            Self::Node => "node_modules",
        }
    }

    /// What the directory holds, in words.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Cargo => "Rust build output",
            Self::Node => "Installed Node packages",
        }
    }

    /// What brings it back, in words.
    pub fn restored_by(self) -> &'static str {
        match self {
            Self::Cargo => "the next build starts from scratch",
            Self::Node => "the next install downloads them again",
        }
    }

    /// Whether `project` holds this ecosystem's manifest.
    fn has_manifest(self, project: &Path) -> bool {
        match self {
            Self::Cargo => is_file(&project.join("Cargo.toml")),
            // A `package.json` alone is not enough: plenty of tools keep one
            // for metadata. A lockfile says packages were installed.
            Self::Node => {
                is_file(&project.join("package.json"))
                    && NODE_LOCKFILES
                        .iter()
                        .any(|lockfile| is_file(&project.join(lockfile)))
            }
        }
    }

    /// Whether `directory` shows this ecosystem's tool wrote it.
    fn wrote(self, directory: &Path) -> bool {
        match self {
            // Cargo tags every target directory it creates, and has written
            // `.rustc_info.json` into it for longer than that.
            Self::Cargo => {
                has_cachedir_tag(directory) || is_file(&directory.join(".rustc_info.json"))
            }
            Self::Node => NODE_INSTALL_STATE
                .iter()
                .any(|state| is_file(&directory.join(state))),
        }
    }
}

/// A build output directory, and the project it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildOutput {
    /// What made it.
    pub ecosystem: Ecosystem,
    /// The directory holding the manifest.
    pub project: PathBuf,
    /// The build output itself.
    pub directory: PathBuf,
}

/// Whether `directory` is build output: named for an ecosystem, with that
/// ecosystem's manifest beside it and its tool's evidence inside it.
///
/// Says nothing about *where* the directory is. That is a separate question
/// with separate answers in [`discover`] and in the guard.
pub fn identify(directory: &Path) -> Option<BuildOutput> {
    let name = directory.file_name()?.to_str()?;
    let ecosystem = Ecosystem::ALL
        .into_iter()
        .find(|ecosystem| ecosystem.directory() == name)?;

    // A real directory, not a link to one: the link's target is somewhere
    // this rule knows nothing about.
    let metadata = std::fs::symlink_metadata(directory).ok()?;
    if !metadata.is_dir() {
        return None;
    }

    let project = directory.parent()?;
    if !ecosystem.has_manifest(project) || !ecosystem.wrote(directory) {
        return None;
    }

    Some(BuildOutput {
        ecosystem,
        project: project.to_owned(),
        directory: directory.to_owned(),
    })
}

/// Find build output under `home`.
///
/// Never looks inside a hidden directory. That is where applications keep
/// their own state — `~/.config`, `~/.local`, `~/.vscode`, `~/.cargo` — and
/// a `node_modules` there is an application's installation, not a project
/// someone forgot. It also skips `.git` and `.snapshots` without needing to
/// name them.
///
/// Does not descend into what it finds, or into any `node_modules`: what is
/// inside belongs to the directory above, and removing a nested one would
/// break the install around it.
pub fn discover(home: &Path, skip: &Exclusions) -> Vec<BuildOutput> {
    if !home.is_dir() {
        return Vec::new();
    }

    let not_projects: Vec<PathBuf> = NOT_PROJECTS.iter().map(|path| home.join(path)).collect();
    let skip = skip.clone();
    let found = Mutex::new(Vec::new());

    let mut builder = ignore::WalkBuilder::new(home);
    builder
        .standard_filters(false)
        .hidden(false)
        .follow_links(false)
        .same_file_system(true)
        .max_depth(Some(MAX_DEPTH))
        .filter_entry(move |entry| {
            // Only directories can hold projects, and the file type comes
            // from the directory listing, so this costs no extra call.
            if !entry.file_type().is_some_and(|kind| kind.is_dir()) {
                return false;
            }
            if entry.depth() == 0 {
                return true;
            }
            let hidden = entry
                .file_name()
                .to_str()
                .is_none_or(|name| name.starts_with('.'));
            !hidden
                && !skip.covers(entry.path())
                && !not_projects.iter().any(|path| entry.path() == path)
        });

    builder.build_parallel().run(|| {
        let found = &found;
        Box::new(move |result| {
            let Ok(entry) = result else {
                return ignore::WalkState::Continue;
            };
            if let Some(output) = identify(entry.path()) {
                found
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(output);
                return ignore::WalkState::Skip;
            }
            if entry.file_name() == "node_modules" {
                return ignore::WalkState::Skip;
            }
            ignore::WalkState::Continue
        })
    });

    let mut found = found
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    found.sort_by(|a, b| a.directory.cmp(&b.directory));
    found
}

/// When anyone last did something to this project, as far as the
/// filesystem can tell.
///
/// The newest of several signals, because each one alone misses a case that
/// matters. Commits and checkouts show in git's reflog, but a week of
/// uncommitted work leaves git untouched; that shows in the project's own
/// directory and the ones directly inside it, which an editor's save
/// renames into. A build — including the one an editor runs in the
/// background when a project is opened — shows in the build output. Erring
/// recent is the safe direction: it makes something look in use, never
/// abandoned.
pub fn last_touched(output: &BuildOutput) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;
    let mut consider = |path: &Path| {
        if let Ok(modified) = std::fs::symlink_metadata(path).and_then(|m| m.modified()) {
            newest = Some(newest.map_or(modified, |seen| seen.max(modified)));
        }
    };

    for directory in [&output.project, &output.directory] {
        consider(directory);
        if let Ok(entries) = std::fs::read_dir(directory) {
            for entry in entries.flatten() {
                consider(&entry.path());
            }
        }
    }

    if let Some(git) = git_directory(&output.project) {
        consider(&git.join("logs/HEAD"));
        consider(&git.join("HEAD"));
        consider(&git.join("index"));
    }

    newest
}

/// The git directory for a project, which may be above it — a crate inside
/// a workspace inside a repository.
///
/// `.git` is a directory in an ordinary clone and a file in a worktree or a
/// submodule, where it names the real one.
fn git_directory(project: &Path) -> Option<PathBuf> {
    for directory in project.ancestors() {
        let dot_git = directory.join(".git");
        let Ok(metadata) = std::fs::symlink_metadata(&dot_git) else {
            continue;
        };
        if metadata.is_dir() {
            return Some(dot_git);
        }
        if metadata.is_file() {
            let text = std::fs::read_to_string(&dot_git).ok()?;
            let named = text.strip_prefix("gitdir:")?.trim();
            return Some(directory.join(named));
        }
    }
    None
}

/// A regular file, not a link to one.
fn is_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file())
}

/// Whether `directory` carries a `CACHEDIR.TAG` with the standard signature.
///
/// The signature and not just the name: the specification exists so that
/// backup tools can trust it, and a file that merely has the name has not
/// declared anything.
fn has_cachedir_tag(directory: &Path) -> bool {
    let tag = directory.join("CACHEDIR.TAG");
    if !is_file(&tag) {
        return false;
    }
    let Ok(file) = std::fs::File::open(&tag) else {
        return false;
    };
    let mut start = Vec::with_capacity(CACHEDIR_SIGNATURE.len());
    file.take(CACHEDIR_SIGNATURE.len() as u64)
        .read_to_end(&mut start)
        .is_ok_and(|_| start == CACHEDIR_SIGNATURE)
}

#[cfg(test)]
pub(crate) mod fixture {
    //! Projects as the tools actually leave them, for this module's tests
    //! and the guard's and the scanner's.

    use std::path::{Path, PathBuf};

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// A Rust project at `project`, built, with `bytes` of output.
    pub fn cargo_project(project: &Path, bytes: usize) -> PathBuf {
        write(&project.join("Cargo.toml"), "[package]\nname = \"x\"\n");
        let target = project.join("target");
        write(
            &target.join("CACHEDIR.TAG"),
            "Signature: 8a477f597d28d172789f06886806bc55\n# This file is a cache directory tag.\n",
        );
        std::fs::create_dir_all(target.join("debug/deps")).unwrap();
        std::fs::write(target.join("debug/deps/libx.rlib"), vec![0u8; bytes]).unwrap();
        target
    }

    /// Set when something was last modified, directories included.
    pub fn set_modified(path: &Path, when: std::time::SystemTime) {
        std::fs::File::open(path)
            .unwrap()
            .set_modified(when)
            .unwrap();
    }

    /// A Node project at `project`, installed with npm.
    pub fn npm_project(project: &Path, bytes: usize) -> PathBuf {
        write(&project.join("package.json"), "{}");
        write(&project.join("package-lock.json"), "{}");
        let modules = project.join("node_modules");
        write(&modules.join(".package-lock.json"), "{}");
        std::fs::create_dir_all(modules.join("left-pad")).unwrap();
        std::fs::write(modules.join("left-pad/index.js"), vec![b'x'; bytes]).unwrap();
        modules
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{cargo_project, npm_project, set_modified};
    use super::*;

    fn home() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        (directory, home)
    }

    #[test]
    fn a_built_rust_project_is_recognised() {
        let (_fixture, home) = home();
        let target = cargo_project(&home.join("Work/limpid"), 10);

        let found = identify(&target).expect("build output");

        assert_eq!(found.ecosystem, Ecosystem::Cargo);
        assert_eq!(found.project, home.join("Work/limpid"));
    }

    #[test]
    fn a_directory_named_target_with_no_manifest_beside_it_is_not_build_output() {
        let (_fixture, home) = home();
        // As found in a crate's unpacked sources: `src/target` is code.
        let sources = home.join(".cargo/registry/src/index/cc-1.5.1");
        std::fs::create_dir_all(sources.join("src/target")).unwrap();
        std::fs::write(sources.join("Cargo.toml"), "").unwrap();
        std::fs::write(sources.join("src/target/apple.rs"), "").unwrap();

        assert_eq!(identify(&sources.join("src/target")), None);
    }

    #[test]
    fn a_manifest_is_not_enough_without_the_tools_own_evidence_inside() {
        let (_fixture, home) = home();
        let project = home.join("Work/deploy");
        std::fs::create_dir_all(project.join("target")).unwrap();
        std::fs::write(project.join("Cargo.toml"), "").unwrap();
        // Something a person put there, named `target` for their own reasons.
        std::fs::write(project.join("target/servers.txt"), "prod-1").unwrap();

        assert_eq!(identify(&project.join("target")), None);
    }

    #[test]
    fn a_tag_with_the_right_name_and_the_wrong_signature_proves_nothing() {
        let (_fixture, home) = home();
        let project = home.join("Work/x");
        std::fs::create_dir_all(project.join("target")).unwrap();
        std::fs::write(project.join("Cargo.toml"), "").unwrap();
        std::fs::write(project.join("target/CACHEDIR.TAG"), "not a signature").unwrap();

        assert_eq!(identify(&project.join("target")), None);
    }

    #[test]
    fn node_packages_need_a_lockfile_as_well_as_a_package_json() {
        let (_fixture, home) = home();
        let modules = npm_project(&home.join("Work/site"), 10);
        assert!(identify(&modules).is_some());

        std::fs::remove_file(home.join("Work/site/package-lock.json")).unwrap();
        assert_eq!(identify(&modules), None);
    }

    #[test]
    fn a_pnpm_install_is_not_offered_because_removing_it_frees_almost_nothing() {
        let (_fixture, home) = home();
        let project = home.join("Work/app");
        std::fs::create_dir_all(project.join("node_modules/.pnpm")).unwrap();
        std::fs::write(project.join("package.json"), "{}").unwrap();
        std::fs::write(project.join("pnpm-lock.yaml"), "").unwrap();
        std::fs::write(project.join("node_modules/.modules.yaml"), "").unwrap();

        assert_eq!(identify(&project.join("node_modules")), None);
    }

    #[test]
    fn a_link_to_build_output_is_not_build_output() {
        let (_fixture, home) = home();
        let target = cargo_project(&home.join("Work/real"), 10);
        let elsewhere = home.join("Work/linked");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("Cargo.toml"), "").unwrap();
        std::os::unix::fs::symlink(&target, elsewhere.join("target")).unwrap();

        assert_eq!(identify(&elsewhere.join("target")), None);
    }

    #[test]
    fn projects_are_found_wherever_they_are_and_only_once() {
        let (_fixture, home) = home();
        cargo_project(&home.join("Work/limpid"), 10);
        npm_project(&home.join("Projects/site"), 10);
        // A crate inside a workspace, built on its own at some point.
        cargo_project(&home.join("Work/limpid/crates/core"), 10);

        let found: Vec<PathBuf> = discover(&home, &Exclusions::default())
            .into_iter()
            .map(|output| output.directory)
            .collect();

        assert_eq!(
            found,
            [
                home.join("Projects/site/node_modules"),
                home.join("Work/limpid/crates/core/target"),
                home.join("Work/limpid/target"),
            ]
        );
    }

    #[test]
    fn an_applications_own_install_under_a_hidden_directory_is_never_found() {
        let (_fixture, home) = home();
        // As found under ~/.config: package.json, a lockfile, and
        // node_modules — an application's plugins, not a project.
        npm_project(&home.join(".config/opencode"), 10);
        cargo_project(&home.join(".local/share/tool"), 10);

        assert!(discover(&home, &Exclusions::default()).is_empty());
    }

    #[test]
    fn nothing_inside_build_output_is_offered_separately() {
        let (_fixture, home) = home();
        let modules = npm_project(&home.join("Work/site"), 10);
        // A package that ships its own installed dependencies.
        npm_project(&modules.join("some-package"), 10);

        let found = discover(&home, &Exclusions::default());

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].directory, modules);
    }

    #[test]
    fn an_excluded_project_is_not_found() {
        let (_fixture, home) = home();
        cargo_project(&home.join("Work/keep"), 10);
        cargo_project(&home.join("Work/other"), 10);

        let found = discover(&home, &Exclusions::new([home.join("Work/keep")]));

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].project, home.join("Work/other"));
    }

    #[test]
    fn the_go_module_cache_is_not_searched_for_projects() {
        let (_fixture, home) = home();
        cargo_project(&home.join("go/pkg/mod/github.com/x/y@v1"), 10);

        assert!(discover(&home, &Exclusions::default()).is_empty());
    }

    #[test]
    fn a_missing_home_finds_nothing_rather_than_failing() {
        let (_fixture, home) = home();

        assert!(discover(&home.join("absent"), &Exclusions::default()).is_empty());
    }

    #[test]
    fn a_commit_makes_a_project_recent_even_when_nothing_else_moved() {
        let (_fixture, home) = home();
        let project = home.join("Work/old");
        let target = cargo_project(&project, 10);
        let long_ago = SystemTime::now() - std::time::Duration::from_secs(400 * 86_400);
        for path in [
            &project,
            &project.join("Cargo.toml"),
            &target,
            &target.join("CACHEDIR.TAG"),
            &target.join("debug"),
        ] {
            set_modified(path, long_ago);
        }
        let output = identify(&target).unwrap();
        let before = last_touched(&output).unwrap();

        // The repository is a level up, as in a workspace.
        std::fs::create_dir_all(home.join("Work/.git/logs")).unwrap();
        std::fs::write(home.join("Work/.git/logs/HEAD"), "commit").unwrap();

        let after = last_touched(&output).unwrap();
        assert!(before < SystemTime::now() - std::time::Duration::from_secs(300 * 86_400));
        assert!(after > SystemTime::now() - std::time::Duration::from_secs(60));
    }

    #[test]
    fn a_build_or_a_saved_file_makes_a_project_recent_without_a_commit() {
        let (_fixture, home) = home();
        let project = home.join("Work/old");
        let target = cargo_project(&project, 10);
        std::fs::create_dir_all(project.join("src")).unwrap();
        let long_ago = SystemTime::now() - std::time::Duration::from_secs(400 * 86_400);
        let age_everything = || {
            for path in [
                &project,
                &project.join("Cargo.toml"),
                &project.join("src"),
                &target,
                &target.join("CACHEDIR.TAG"),
                &target.join("debug"),
            ] {
                set_modified(path, long_ago);
            }
        };
        let output = identify(&target).unwrap();
        let recently = SystemTime::now() - std::time::Duration::from_secs(60);

        // A build: the compiler writes into the build output.
        age_everything();
        std::fs::write(target.join("debug/x"), "binary").unwrap();
        assert!(last_touched(&output).unwrap() > recently);

        // A save: an editor renames the new file into place.
        age_everything();
        std::fs::write(project.join("src/lib.rs.new"), "").unwrap();
        std::fs::rename(project.join("src/lib.rs.new"), project.join("src/lib.rs")).unwrap();
        assert!(last_touched(&output).unwrap() > recently);
    }

    #[test]
    fn a_worktree_points_at_its_real_git_directory() {
        let (_fixture, home) = home();
        let real = home.join("Work/main/.git/worktrees/feature");
        std::fs::create_dir_all(&real).unwrap();
        let worktree = home.join("Work/feature");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", real.display()),
        )
        .unwrap();

        assert_eq!(git_directory(&worktree), Some(real));
    }
}
