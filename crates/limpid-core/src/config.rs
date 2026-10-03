//! What Limpid remembers between runs.
//!
//! One file, `$XDG_CONFIG_HOME/limpid/config.toml`, and a promise about it:
//! it is safe to edit by hand. Three things follow from that promise, and
//! each is the kind of thing that is easy to get wrong.
//!
//! **Saving must not destroy what the person wrote.** The file is edited
//! with `toml_edit`, which keeps comments, ordering and unknown keys, and
//! only the keys Limpid owns are touched. A config file that loses its
//! comments the first time the application saves it is not safe to edit by
//! hand, whatever the documentation says.
//!
//! **A file that cannot be understood must not be overwritten.** If it fails
//! to parse, or was written by a newer Limpid, it is read as defaults and
//! left alone. Overwriting it would silently throw away whatever the person
//! was in the middle of doing.
//!
//! **One bad value costs only itself.** Each key is read and checked on its
//! own, so a typo in one setting produces a warning and a default for that
//! setting, not a refusal to start.

use std::path::{Path, PathBuf};

use crate::paths::Roots;
use crate::privileged::{MAXIMUM_DAYS, MAXIMUM_KEEP, MINIMUM_DAYS, MINIMUM_KEEP};

/// Where the file lives, relative to the config directory.
pub const FILE: &str = "limpid/config.toml";

/// The format version this build writes, and the newest it can read.
///
/// Bumped only when the meaning of an existing key changes. A file with a
/// higher version was written by a newer Limpid; it is read as far as it
/// can be and never overwritten, because writing it back would downgrade it.
pub const VERSION: i64 = 1;

/// Written when there is no file yet. Comments here are the documentation
/// most people will ever read, so they say what each setting costs.
const TEMPLATE: &str = r#"# Limpid configuration. Limpid edits this file, and so can you: comments
# and anything it does not recognise are left alone when it saves.

version = 1

[policy]
# Versions of each installed package kept when the pacman cache is trimmed.
# Keeping at least one older version is what makes a downgrade possible after
# a bad update. 1 to 10.
keep_package_versions = 3

# Days of system journal kept when it is trimmed. Below a week the journal
# stops being able to answer "what changed before this started". 7 to 3650.
keep_journal_days = 14

[exclude]
# Paths Limpid never offers to remove, and everything under them. A leading
# ~/ means your home directory, which keeps this file portable between
# machines.
paths = []
"#;

/// The numbers that decide how much a trim keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Package versions kept per installed package.
    pub keep_package_versions: u8,
    /// Days of journal kept.
    pub keep_journal_days: u16,
}

impl Default for Policy {
    fn default() -> Self {
        // paccache's own default, and about the shortest journal window that
        // still answers "what changed before this broke".
        Self {
            keep_package_versions: 3,
            keep_journal_days: 14,
        }
    }
}

/// Paths Limpid never offers, and never removes.
///
/// An exclusion covers its path and everything below it. It is enforced in
/// two places on purpose: scanners leave excluded paths out, so they are
/// never *offered*, and the executor refuses them, so a plan built from a
/// stale selection cannot remove them either. Hiding something the user
/// asked to keep is a courtesy; refusing to delete it is the guarantee.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exclusions {
    /// Absolute, sorted, without duplicates.
    paths: Vec<PathBuf>,
}

impl Exclusions {
    /// Build a set from absolute paths.
    pub fn new(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut set = Self::default();
        for path in paths {
            set.add(path);
        }
        set
    }

    /// Whether `path` is excluded — it is an exclusion, or lies under one.
    ///
    /// Compared by component, so excluding `~/.cache` does not exclude
    /// `~/.cache-backup`.
    pub fn covers(&self, path: &Path) -> bool {
        self.paths.iter().any(|excluded| path.starts_with(excluded))
    }

    /// Whether an exclusion lies strictly inside `path`.
    ///
    /// When it does, `path` cannot be removed wholesale: whatever is excluded
    /// inside would go with it. The executor descends instead.
    pub fn inside(&self, path: &Path) -> bool {
        self.paths
            .iter()
            .any(|excluded| excluded != path && excluded.starts_with(path))
    }

    /// Add a path. Returns whether it was new.
    ///
    /// Relative paths are refused rather than guessed at: relative to what
    /// would depend on where Limpid happened to be started.
    pub fn add(&mut self, path: PathBuf) -> bool {
        if !path.is_absolute() || self.paths.contains(&path) {
            return false;
        }
        self.paths.push(path);
        self.paths.sort();
        true
    }

    /// Remove a path. Returns whether it was there.
    pub fn remove(&mut self, path: &Path) -> bool {
        let before = self.paths.len();
        self.paths.retain(|excluded| excluded != path);
        self.paths.len() != before
    }

    /// The excluded paths, sorted.
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Whether nothing is excluded.
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

/// Everything Limpid remembers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// How much a trim keeps.
    pub policy: Policy,
    /// What is never offered.
    pub exclusions: Exclusions,
}

/// Why a save did not happen.
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    /// The file on disk could not be understood, so it is left untouched.
    #[error(
        "{} could not be read, so Limpid will not overwrite it; fix it or remove it",
        .0.display()
    )]
    Protected(PathBuf),
    /// The filesystem refused.
    #[error("could not write {}: {error}", path.display())]
    Io {
        /// The file that could not be written.
        path: PathBuf,
        /// What the filesystem said.
        error: std::io::Error,
    },
}

/// The config file, as read, and able to write itself back.
#[derive(Debug, Clone)]
pub struct Store {
    /// The settings in force.
    pub config: Config,
    /// Things worth telling the user about what was read. Empty in the
    /// ordinary case.
    pub warnings: Vec<String>,
    path: PathBuf,
    home: PathBuf,
    /// The file as it was read, so a save can change only the keys Limpid
    /// owns. `None` when there was no file.
    document: Option<toml_edit::DocumentMut>,
    /// What the settings were when last read or written, so a save touches
    /// only the keys that actually changed. Without it, saving an exclusion
    /// would also rewrite a policy value the person wrote — including an
    /// invalid one they have not got round to fixing, silently replacing it
    /// with the default it was being read as.
    saved: Config,
    writable: bool,
}

impl Store {
    /// Read the config file for these roots. Never fails: a missing or
    /// unreadable file produces defaults, and a warning when it was
    /// unreadable.
    pub fn open(roots: &Roots) -> Self {
        let path = roots.config(FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::from_text(&text, path, roots.home.clone()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self {
                config: Config::default(),
                warnings: Vec::new(),
                path,
                home: roots.home.clone(),
                document: None,
                saved: Config::default(),
                writable: true,
            },
            Err(error) => Self {
                config: Config::default(),
                warnings: vec![format!("could not read {}: {error}", path.display())],
                path,
                home: roots.home.clone(),
                document: None,
                saved: Config::default(),
                writable: false,
            },
        }
    }

    /// Interpret the text of a config file.
    fn from_text(text: &str, path: PathBuf, home: PathBuf) -> Self {
        let document = match text.parse::<toml_edit::DocumentMut>() {
            Ok(document) => document,
            Err(error) => {
                return Self {
                    config: Config::default(),
                    warnings: vec![format!(
                        "{} is not valid TOML, so the defaults are in use and the \
                         file is left untouched: {}",
                        path.display(),
                        error.to_string().trim(),
                    )],
                    path,
                    home,
                    document: None,
                    saved: Config::default(),
                    writable: false,
                };
            }
        };

        let mut warnings = Vec::new();
        let mut writable = true;

        match document.get("version") {
            None => {}
            Some(item) => match item.as_integer() {
                Some(VERSION) => {}
                Some(newer) if newer > VERSION => {
                    warnings.push(format!(
                        "{} was written by a newer Limpid (format {newer}); it is read \
                         as far as this version understands it, and not overwritten",
                        path.display(),
                    ));
                    writable = false;
                }
                _ => {
                    warnings.push(format!(
                        "{}: `version` is not a format this Limpid knows; the file is \
                         left untouched",
                        path.display(),
                    ));
                    writable = false;
                }
            },
        }

        let config = Config {
            policy: read_policy(&document, &mut warnings),
            exclusions: read_exclusions(&document, &home, &mut warnings),
        };

        Self {
            saved: config.clone(),
            config,
            warnings,
            path,
            home,
            document: Some(document),
            writable,
        }
    }

    /// Where the file is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether a save would be allowed.
    pub fn is_writable(&self) -> bool {
        self.writable
    }

    /// A path the way the file writes it: `~/...` under home.
    pub fn display(&self, path: &Path) -> String {
        contract(path, &self.home)
    }

    /// Read a path the way the file accepts it. See [`expand`].
    pub fn expand(&self, text: &str) -> Option<PathBuf> {
        expand(text, &self.home)
    }

    /// Write the settings back.
    ///
    /// Only the keys that changed since the file was read are written;
    /// everything else — comments, order, keys this version does not know,
    /// and settings the person wrote and nobody has touched since — stays as
    /// it was. The new file is written beside the old one and renamed over
    /// it, so a crash part-way through leaves the old file rather than half
    /// of a new one.
    pub fn save(&mut self) -> Result<(), SaveError> {
        if !self.writable {
            return Err(SaveError::Protected(self.path.clone()));
        }

        let fresh = self.document.is_none();
        if !fresh && self.config == self.saved {
            return Ok(());
        }

        let mut document = match self.document.take() {
            Some(document) => document,
            None => TEMPLATE
                .parse::<toml_edit::DocumentMut>()
                .expect("the built-in template is valid TOML"),
        };

        document["version"] = toml_edit::value(VERSION);

        // A new file gets every value; an existing one only what changed.
        let (now, before) = (self.config.policy, self.saved.policy);
        if fresh || now.keep_package_versions != before.keep_package_versions {
            set_policy_key(
                &mut document,
                "keep_package_versions",
                i64::from(now.keep_package_versions),
            );
        }
        if fresh || now.keep_journal_days != before.keep_journal_days {
            set_policy_key(
                &mut document,
                "keep_journal_days",
                i64::from(now.keep_journal_days),
            );
        }
        if fresh || self.config.exclusions != self.saved.exclusions {
            write_exclusions(&mut document, &self.config.exclusions, &self.home);
        }

        let text = document.to_string();
        self.document = Some(document);

        let parent = self.path.parent().unwrap_or(Path::new("/"));
        let staged = self.path.with_extension("toml.new");
        let written = std::fs::create_dir_all(parent)
            .and_then(|()| std::fs::write(&staged, text))
            .and_then(|()| std::fs::rename(&staged, &self.path));

        if let Err(error) = written {
            #[expect(
                clippy::disallowed_methods,
                reason = "removes only the staging file this function just \
                          wrote, so a failed save leaves nothing behind"
            )]
            let _ = std::fs::remove_file(&staged);
            return Err(SaveError::Io {
                path: self.path.clone(),
                error,
            });
        }

        self.saved = self.config.clone();
        Ok(())
    }
}

/// Read `[policy]`, falling back per key.
fn read_policy(document: &toml_edit::DocumentMut, warnings: &mut Vec<String>) -> Policy {
    let defaults = Policy::default();
    let Some(table) = document.get("policy").and_then(|item| item.as_table_like()) else {
        return defaults;
    };

    // A misspelt key is ignored by any TOML reader, which means the setting
    // the person meant to change silently stays at its default. Worth a
    // warning here, because these two settings decide how much is removed.
    for (key, _) in table.iter() {
        if key != "keep_package_versions" && key != "keep_journal_days" {
            warnings.push(format!(
                "[policy] has an unknown setting `{key}`, which is ignored"
            ));
        }
    }

    Policy {
        keep_package_versions: read_bounded(
            table.get("keep_package_versions"),
            "keep_package_versions",
            i64::from(MINIMUM_KEEP),
            i64::from(MAXIMUM_KEEP),
            i64::from(defaults.keep_package_versions),
            warnings,
        ) as u8,
        keep_journal_days: read_bounded(
            table.get("keep_journal_days"),
            "keep_journal_days",
            i64::from(MINIMUM_DAYS),
            i64::from(MAXIMUM_DAYS),
            i64::from(defaults.keep_journal_days),
            warnings,
        ) as u16,
    }
}

/// Read one integer setting, keeping it within the bounds the privileged
/// helper enforces.
///
/// The bounds are the helper's own. A config file cannot produce an
/// operation the helper would refuse, so an out-of-range value is caught
/// here, where it can be explained, rather than as a failed clean.
fn read_bounded(
    item: Option<&toml_edit::Item>,
    name: &str,
    minimum: i64,
    maximum: i64,
    default: i64,
    warnings: &mut Vec<String>,
) -> i64 {
    let Some(item) = item else {
        return default;
    };
    match item.as_integer() {
        Some(value) if (minimum..=maximum).contains(&value) => value,
        Some(value) => {
            warnings.push(format!(
                "`{name} = {value}` is outside {minimum}..={maximum}; using {default}"
            ));
            default
        }
        None => {
            warnings.push(format!(
                "`{name}` should be a whole number; using {default}"
            ));
            default
        }
    }
}

/// Read `[exclude] paths`, skipping entries that cannot be used.
fn read_exclusions(
    document: &toml_edit::DocumentMut,
    home: &Path,
    warnings: &mut Vec<String>,
) -> Exclusions {
    let Some(paths) = document
        .get("exclude")
        .and_then(|item| item.as_table_like())
        .and_then(|table| table.get("paths"))
    else {
        return Exclusions::default();
    };

    let Some(array) = paths.as_array() else {
        warnings.push("[exclude] `paths` should be a list of paths".to_owned());
        return Exclusions::default();
    };

    let mut exclusions = Exclusions::default();
    for value in array.iter() {
        let Some(text) = value.as_str() else {
            warnings.push(format!(
                "[exclude] `{}` is not a path, and is ignored",
                value.to_string().trim()
            ));
            continue;
        };
        match expand(text, home) {
            Some(path) => {
                exclusions.add(path);
            }
            None => warnings.push(format!(
                "[exclude] `{text}` is relative, and is ignored; start it with / or ~/"
            )),
        }
    }
    exclusions
}

/// Turn a path as a person would write it into an absolute one.
///
/// `~` and `~/…` mean the home directory; anything else must already be
/// absolute. Used for the config file, where a relative path would depend on
/// where Limpid happened to be started, and so is refused.
pub fn expand(text: &str, home: &Path) -> Option<PathBuf> {
    if text == "~" {
        return Some(home.to_owned());
    }
    if let Some(rest) = text.strip_prefix("~/") {
        return Some(home.join(rest));
    }
    let path = PathBuf::from(text);
    path.is_absolute().then_some(path)
}

/// Write a path back, as `~/…` when it is under the home directory.
///
/// So that the file can be carried to another machine, or another user
/// name, with the rest of someone's dotfiles.
pub fn contract(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Set one policy key, keeping the table, its comments, and its other keys.
fn set_policy_key(document: &mut toml_edit::DocumentMut, key: &str, value: i64) {
    if !document.contains_table("policy") {
        document["policy"] = toml_edit::table();
    }
    document["policy"][key] = toml_edit::value(value);
}

/// Set the exclusion list, keeping the table and its comments.
fn write_exclusions(document: &mut toml_edit::DocumentMut, exclusions: &Exclusions, home: &Path) {
    if !document.contains_table("exclude") {
        document["exclude"] = toml_edit::table();
    }

    let mut array = toml_edit::Array::new();
    for path in exclusions.paths() {
        array.push(contract(path, home));
    }
    // One per line once there is more than one: this is the part of the
    // file people read, and diff, and a single long line is neither.
    if array.len() > 1 {
        for value in array.iter_mut() {
            value.decor_mut().set_prefix("\n    ");
        }
        array.set_trailing("\n");
        array.set_trailing_comma(true);
    }
    document["exclude"]["paths"] = toml_edit::value(array);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Roots) {
        let directory = tempfile::tempdir().unwrap();
        let roots = Roots::under(directory.path());
        (directory, roots)
    }

    fn write(roots: &Roots, text: &str) {
        let path = roots.config(FILE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn read(roots: &Roots) -> String {
        std::fs::read_to_string(roots.config(FILE)).unwrap()
    }

    #[test]
    fn no_file_means_defaults_and_nothing_to_report() {
        let (_fixture, roots) = fixture();

        let store = Store::open(&roots);

        assert_eq!(store.config, Config::default());
        assert!(store.warnings.is_empty());
        assert!(store.is_writable());
    }

    #[test]
    fn a_valid_file_is_read() {
        let (_fixture, roots) = fixture();
        write(
            &roots,
            "version = 1\n[policy]\nkeep_package_versions = 5\nkeep_journal_days = 30\n\
             [exclude]\npaths = [\"~/.cache/debuginfod_client\", \"/opt/keep\"]\n",
        );

        let store = Store::open(&roots);

        assert!(store.warnings.is_empty(), "{:?}", store.warnings);
        assert_eq!(store.config.policy.keep_package_versions, 5);
        assert_eq!(store.config.policy.keep_journal_days, 30);
        assert!(
            store
                .config
                .exclusions
                .covers(&roots.cache("debuginfod_client"))
        );
        assert!(store.config.exclusions.covers(Path::new("/opt/keep/file")));
    }

    #[test]
    fn one_bad_value_costs_only_that_setting() {
        let (_fixture, roots) = fixture();
        write(
            &roots,
            "[policy]\nkeep_package_versions = 0\nkeep_journal_days = 30\n",
        );

        let store = Store::open(&roots);

        // Zero would leave no downgrade path. Refused, explained, defaulted.
        assert_eq!(store.config.policy.keep_package_versions, 3);
        assert_eq!(store.config.policy.keep_journal_days, 30);
        assert_eq!(store.warnings.len(), 1);
        assert!(store.warnings[0].contains("keep_package_versions"));
        // A bad value is not a file that cannot be understood.
        assert!(store.is_writable());
    }

    #[test]
    fn a_policy_value_outside_what_the_helper_accepts_never_gets_through() {
        let (_fixture, roots) = fixture();
        write(
            &roots,
            "[policy]\nkeep_journal_days = 1\nkeep_package_versions = 200\n",
        );

        let policy = Store::open(&roots).config.policy;

        assert!((MINIMUM_DAYS..=MAXIMUM_DAYS).contains(&policy.keep_journal_days));
        assert!((MINIMUM_KEEP..=MAXIMUM_KEEP).contains(&policy.keep_package_versions));
    }

    #[test]
    fn a_misspelt_setting_is_reported_rather_than_silently_ignored() {
        let (_fixture, roots) = fixture();
        write(&roots, "[policy]\nkeep_pacakge_versions = 5\n");

        let store = Store::open(&roots);

        assert_eq!(store.config.policy.keep_package_versions, 3);
        assert!(
            store.warnings[0].contains("keep_pacakge_versions"),
            "{:?}",
            store.warnings
        );
    }

    #[test]
    fn a_relative_exclusion_is_skipped_and_reported() {
        let (_fixture, roots) = fixture();
        write(&roots, "[exclude]\npaths = [\"Videos\", \"~/Music\"]\n");

        let store = Store::open(&roots);

        assert_eq!(store.config.exclusions.paths(), [roots.home("Music")]);
        assert!(store.warnings[0].contains("Videos"));
    }

    #[test]
    fn a_file_that_is_not_toml_is_read_as_defaults_and_never_overwritten() {
        let (_fixture, roots) = fixture();
        write(&roots, "this is [not toml at all\n");

        let mut store = Store::open(&roots);

        assert_eq!(store.config, Config::default());
        assert!(!store.is_writable());
        assert!(store.warnings[0].contains("not valid TOML"));

        store.config.policy.keep_journal_days = 30;
        assert!(matches!(store.save(), Err(SaveError::Protected(_))));
        // Whatever the person was halfway through writing is still there.
        assert_eq!(read(&roots), "this is [not toml at all\n");
    }

    #[test]
    fn a_file_from_a_newer_limpid_is_read_but_never_downgraded() {
        let (_fixture, roots) = fixture();
        write(&roots, "version = 2\n[policy]\nkeep_journal_days = 30\n");

        let mut store = Store::open(&roots);

        assert_eq!(store.config.policy.keep_journal_days, 30);
        assert!(!store.is_writable());
        assert!(matches!(store.save(), Err(SaveError::Protected(_))));
    }

    #[test]
    fn the_first_save_writes_the_commented_template() {
        let (_fixture, roots) = fixture();
        let mut store = Store::open(&roots);

        store.save().unwrap();

        let text = read(&roots);
        assert!(text.contains("# Limpid configuration."));
        assert!(text.contains("keep_package_versions = 3"));
        // And it reads back as what was saved.
        assert_eq!(Store::open(&roots).config, Config::default());
    }

    #[test]
    fn saving_keeps_the_comments_and_keys_the_person_wrote() {
        let (_fixture, roots) = fixture();
        write(
            &roots,
            "# my notes about why\nversion = 1\n\n[policy]\n# three is plenty\n\
             keep_package_versions = 3\n\n[mine]\nsomething = \"else\"\n",
        );
        let mut store = Store::open(&roots);

        store.config.policy.keep_package_versions = 5;
        store.config.exclusions.add(roots.home("Videos"));
        store.save().unwrap();

        let text = read(&roots);
        assert!(text.contains("# my notes about why"), "{text}");
        assert!(text.contains("# three is plenty"), "{text}");
        assert!(text.contains("[mine]"), "{text}");
        assert!(text.contains("keep_package_versions = 5"), "{text}");
        assert!(text.contains("\"~/Videos\""), "{text}");
    }

    #[test]
    fn saving_one_setting_leaves_the_others_exactly_as_written() {
        // Found by hand: the person's `keep_package_versions = 0` is invalid
        // and read as 3. Adding an exclusion — nothing to do with policy —
        // used to write 3 over their 0 without being asked.
        let (_fixture, roots) = fixture();
        write(
            &roots,
            "version = 1\n[policy]\nkeep_package_versions = 0\nkeep_jornal_days = 30\n",
        );
        let mut store = Store::open(&roots);
        assert_eq!(store.config.policy.keep_package_versions, 3);

        store.config.exclusions.add(roots.home("Videos"));
        store.save().unwrap();

        let text = read(&roots);
        assert!(text.contains("keep_package_versions = 0"), "{text}");
        // The misspelt key is theirs too, and stays for them to fix.
        assert!(text.contains("keep_jornal_days = 30"), "{text}");
        assert!(text.contains("\"~/Videos\""), "{text}");
    }

    #[test]
    fn a_setting_the_person_changes_is_written_even_over_an_invalid_one() {
        let (_fixture, roots) = fixture();
        write(&roots, "version = 1\n[policy]\nkeep_package_versions = 0\n");
        let mut store = Store::open(&roots);

        store.config.policy.keep_package_versions = 5;
        store.save().unwrap();

        assert!(read(&roots).contains("keep_package_versions = 5"));
    }

    #[test]
    fn saving_with_nothing_changed_does_not_touch_the_file() {
        let (_fixture, roots) = fixture();
        write(&roots, "version = 1\n# a comment written last\n");
        let before = std::fs::metadata(roots.config(FILE))
            .unwrap()
            .modified()
            .unwrap();
        let mut store = Store::open(&roots);

        std::thread::sleep(std::time::Duration::from_millis(20));
        store.save().unwrap();

        let after = std::fs::metadata(roots.config(FILE))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn paths_under_home_are_written_portably() {
        let (_fixture, roots) = fixture();
        let mut store = Store::open(&roots);
        store
            .config
            .exclusions
            .add(roots.cache("debuginfod_client"));
        store.config.exclusions.add(PathBuf::from("/opt/keep"));

        store.save().unwrap();

        let text = read(&roots);
        assert!(text.contains("\"~/.cache/debuginfod_client\""), "{text}");
        assert!(text.contains("\"/opt/keep\""), "{text}");
        // And a machine with a different home reads them as its own.
        assert_eq!(
            Store::open(&roots).config.exclusions,
            store.config.exclusions
        );
    }

    #[test]
    fn a_save_leaves_no_staging_file_behind() {
        let (_fixture, roots) = fixture();
        let mut store = Store::open(&roots);

        store.save().unwrap();

        assert!(!roots.config(FILE).with_extension("toml.new").exists());
    }

    #[test]
    fn an_exclusion_covers_its_subtree_and_nothing_beside_it() {
        let exclusions = Exclusions::new([PathBuf::from("/home/x/.cache")]);

        assert!(exclusions.covers(Path::new("/home/x/.cache")));
        assert!(exclusions.covers(Path::new("/home/x/.cache/yay/brave-bin")));
        // Same leading characters, different directory.
        assert!(!exclusions.covers(Path::new("/home/x/.cache-backup")));
        assert!(!exclusions.covers(Path::new("/home/x")));
    }

    #[test]
    fn an_exclusion_inside_a_directory_is_seen_from_that_directory() {
        let exclusions = Exclusions::new([PathBuf::from("/home/x/.cache/yay/brave-bin")]);

        assert!(exclusions.inside(Path::new("/home/x/.cache/yay")));
        assert!(exclusions.inside(Path::new("/home/x/.cache")));
        // Not strictly inside itself, and not inside a sibling.
        assert!(!exclusions.inside(Path::new("/home/x/.cache/yay/brave-bin")));
        assert!(!exclusions.inside(Path::new("/home/x/.cache/nvim")));
    }

    #[test]
    fn exclusions_refuse_relative_paths_and_duplicates() {
        let mut exclusions = Exclusions::default();

        assert!(exclusions.add(PathBuf::from("/a")));
        assert!(!exclusions.add(PathBuf::from("/a")));
        assert!(!exclusions.add(PathBuf::from("relative")));
        assert_eq!(exclusions.paths().len(), 1);

        assert!(exclusions.remove(Path::new("/a")));
        assert!(!exclusions.remove(Path::new("/a")));
        assert!(exclusions.is_empty());
    }
}
