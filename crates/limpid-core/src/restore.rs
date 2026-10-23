//! Putting back what a run sent to the trash.
//!
//! The history records each trashed file the way the trash recorded it: the
//! `.trashinfo`, the path it came from, and when. Those three together are
//! the only thing a restore will move. Any one of them differing means the
//! trash now holds something else, and something else is left where it is.
//!
//! **Nothing is overwritten.** The original path is checked immediately
//! before the move, and the move itself refuses to replace whatever is
//! there, so a file that appears in the gap stays. A symlink along the path
//! is a refusal too: renaming through it would write somewhere other than
//! the path that was read.
//!
//! **A dry run is the default.** [`Restorer::dry_run`] and
//! [`Restorer::applying`] are separate constructors, so the one that moves
//! files has to be asked for by name. Only an applying restore writes a
//! line to the history, and it writes a new line: the one that sent the
//! file is never rewritten.

use std::ffi::OsStr;
use std::io::ErrorKind;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use crate::history::{self, History, Read, Recorded, Run, Trashed};
use crate::paths::Roots;

/// Why one thing was not put back.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// A later run has already put this record back.
    #[error("it has already been put back")]
    AlreadyRestored,
    /// The trash no longer has this record.
    #[error("it is no longer in the trash")]
    Gone,
    /// Something in the trash shares the name or the record, and is not it.
    #[error("the trash now holds something else under the same name")]
    Replaced,
    /// The original path exists, so putting this back would replace it.
    #[error("{path} is occupied")]
    Occupied {
        /// The path that is taken.
        path: PathBuf,
    },
    /// The original path is not inside the home directory.
    #[error("{0} is outside the home directory")]
    OutsideHome(PathBuf),
    /// A symlink is on the path, which a rename would follow.
    #[error("{0} is a symlink")]
    Symlink(PathBuf),
    /// The path contains `..`.
    #[error("{0} contains a parent-directory component")]
    Climbing(PathBuf),
    /// The path is not absolute.
    #[error("{0} is not an absolute path")]
    NotAbsolute(PathBuf),
    /// The directory it would return to is missing, or is not a directory.
    #[error("the folder {0} is not there to put it back into")]
    NoParent(PathBuf),
    /// The copy in the trash is not inside the home directory, so it is not
    /// moved. A trash on another disk can sit outside the home, and a
    /// restore does not reach out of it.
    #[error("{0} is outside the home trash")]
    NotInHomeTrash(PathBuf),
    /// The trash could not be listed, so there is nothing honest to match.
    #[error("the trash could not be read")]
    Unlisted,
    /// The filesystem refused the move.
    #[error("{path}: {reason}")]
    Failed {
        /// The path the operation was about.
        path: PathBuf,
        /// What the operating system said.
        reason: String,
    },
}

impl Refusal {
    /// The reason, with paths under `home` written as `~/`.
    pub fn describe(&self, home: &Path) -> String {
        self.text(|path| crate::config::contract(path, home))
    }

    fn text(&self, show: impl Fn(&Path) -> String) -> String {
        match self {
            Self::AlreadyRestored => "it has already been put back".to_owned(),
            Self::Gone => "it is no longer in the trash".to_owned(),
            Self::Replaced => "the trash now holds something else under the same name".to_owned(),
            Self::Occupied { path } => format!("{} is occupied", show(path)),
            Self::OutsideHome(path) => {
                format!("{} is outside the home directory", show(path))
            }
            Self::Symlink(path) => format!("{} is a symlink", show(path)),
            Self::Climbing(path) => {
                format!("{} contains a parent-directory component", show(path))
            }
            Self::NotAbsolute(path) => format!("{} is not an absolute path", show(path)),
            Self::NoParent(path) => {
                format!("the folder {} is not there to put it back into", show(path))
            }
            Self::NotInHomeTrash(path) => {
                format!("{} is outside the home trash", show(path))
            }
            Self::Unlisted => "the trash could not be read".to_owned(),
            Self::Failed { path, reason } => format!("{}: {reason}", show(path)),
        }
    }
}

/// What happened to one trashed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fate {
    /// Put back, or — when the restore was a dry run — would be.
    PutBack,
    /// Left where it is, for this reason.
    Refused(Refusal),
}

/// One file a restore considered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    /// The name it was shown under when it was sent.
    pub name: String,
    /// The trash's record of it.
    pub trashed: Trashed,
    /// What this restore did with it, or would.
    pub fate: Fate,
}

/// What a restore did, or would have done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Whether this was asked to move files. A dry run is false even when
    /// every file would go back.
    pub applied: bool,
    /// The run that sent them, by its `at`.
    pub at: u64,
    /// When that run happened, the way the history page writes it.
    pub when: String,
    /// One entry per thing that run sent to the trash, in the order it went.
    pub attempts: Vec<Attempt>,
    /// Whether the restore was written down. A dry run records nothing,
    /// because it moved nothing.
    pub recorded: Recorded,
    /// Trouble that did not stop a file coming back: the file is home, and
    /// the trash's record of it could not be removed.
    pub problems: Vec<String>,
}

impl Report {
    /// Whether anything was put back, or would be.
    pub fn puts_back(&self) -> bool {
        self.attempts
            .iter()
            .any(|attempt| matches!(attempt.fate, Fate::PutBack))
    }
}

/// Why a restore could not even be considered.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The history file could not be read.
    #[error("cannot read {history}: {source}")]
    Unreadable {
        /// Where the history was looked for.
        history: PathBuf,
        /// What the operating system said.
        #[source]
        source: std::io::Error,
    },
    /// No line in the history has this `at`.
    #[error("there is no run recorded at {0}")]
    NoSuchRun(u64),
    /// Two lines share that `at`, so either one could be meant.
    #[error("more than one run is recorded at {0}")]
    Ambiguous(u64),
    /// The run removed things, and none of them went to the trash.
    #[error("nothing in that run went to the trash, so there is nothing to put back")]
    NothingTrashed,
}

/// Puts back what one run sent to the trash.
pub struct Restorer {
    roots: Roots,
    history: History,
    /// Private on purpose. The constructors are the only way to set it, so
    /// a caller cannot pass the wrong boolean.
    apply: bool,
}

impl Restorer {
    /// A restore that reports what it would put back and changes nothing.
    pub fn dry_run(roots: &Roots) -> Self {
        Self::new(roots, false)
    }

    /// A restore that moves files, and appends a line to the history when
    /// it managed to.
    ///
    /// Spelled out at the call site on purpose. There is no boolean to get
    /// the wrong way round, and the history comes with it, so a file cannot
    /// come back without a record that it did.
    pub fn applying(roots: &Roots) -> Self {
        Self::new(roots, true)
    }

    fn new(roots: &Roots, apply: bool) -> Self {
        Self {
            history: History::at(roots),
            roots: roots.clone(),
            apply,
        }
    }

    /// Whether this restore moves files.
    pub fn is_applying(&self) -> bool {
        self.apply
    }

    /// Put back everything `at` sent to the trash, or say what would be.
    ///
    /// Each record is matched against the trash as it is at that moment,
    /// not as it was when the confirmation was drawn. A record that no
    /// longer matches, or an original path that is taken, is refused on
    /// its own; the rest of the run still goes back.
    pub fn run(&self, at: u64) -> Result<Report, Error> {
        let read = self.load()?;
        let runs: Vec<&Run> = read.runs.iter().filter(|run| run.at == at).collect();
        let run = match runs.as_slice() {
            [run] => *run,
            [] => return Err(Error::NoSuchRun(at)),
            _ => return Err(Error::Ambiguous(at)),
        };
        let when = run.when();
        let records: Vec<(String, Trashed)> = run
            .entries
            .iter()
            .flat_map(|entry| {
                entry
                    .trashed
                    .iter()
                    .cloned()
                    .map(|trashed| (label(entry, &trashed), trashed))
            })
            .collect();
        if records.is_empty() {
            return Err(Error::NothingTrashed);
        }

        let mut attempts = Vec::with_capacity(records.len());
        let mut restored = Vec::new();
        let mut problems = Vec::new();
        for (name, record) in records {
            // Read again per file. Another Limpid can append a restore
            // between two of them, and the check has to see that line.
            let current = match self.history.read() {
                Ok(current) => current,
                Err(error) => {
                    attempts.push(Attempt {
                        name,
                        trashed: record.clone(),
                        fate: Fate::Refused(Refusal::Failed {
                            path: record.original.clone(),
                            reason: error.to_string(),
                        }),
                    });
                    continue;
                }
            };
            match self.judge(&current, &record) {
                Ok(located) if self.apply => match move_back(&located) {
                    Ok(leftover) => {
                        if let Some(leftover) = leftover {
                            problems.push(leftover);
                        }
                        restored.push(record.clone());
                        attempts.push(Attempt {
                            name,
                            trashed: record,
                            fate: Fate::PutBack,
                        });
                    }
                    Err(refusal) => attempts.push(Attempt {
                        name,
                        trashed: record,
                        fate: Fate::Refused(refusal),
                    }),
                },
                Ok(_) => attempts.push(Attempt {
                    name,
                    trashed: record,
                    fate: Fate::PutBack,
                }),
                Err(refusal) => attempts.push(Attempt {
                    name,
                    trashed: record,
                    fate: Fate::Refused(refusal),
                }),
            }
        }

        let recorded = if self.apply {
            self.write_down(&restored, &attempts, &problems)
        } else {
            Recorded::Nothing
        };

        Ok(Report {
            applied: self.apply,
            at,
            when,
            attempts,
            recorded,
            problems,
        })
    }

    fn load(&self) -> Result<Read, Error> {
        self.history.read().map_err(|source| Error::Unreadable {
            history: self.history.path().to_owned(),
            source,
        })
    }

    /// Match one record against the trash, then against the path it would
    /// return to. Both are as of this call.
    fn judge(&self, read: &Read, record: &Trashed) -> Result<Located, Refusal> {
        if read.was_restored(record) {
            return Err(Refusal::AlreadyRestored);
        }
        let listed = trash::os_limited::list().map_err(|_| Refusal::Unlisted)?;
        // Every trash the crate can see, including ones on other disks.
        // Only a record that lives inside the home is eligible: a restore
        // does not move a file that sits outside it, and a test whose home
        // is a fixture therefore cannot see the real trash.
        let visible: Vec<trash::TrashItem> = listed
            .iter()
            .filter(|item| Path::new(&item.id).starts_with(&self.roots.home))
            .cloned()
            .collect();
        match identify(record, &visible) {
            Match::Exact(item) => screen(&self.roots, record, item),
            Match::Replaced => Err(Refusal::Replaced),
            Match::Gone if listed.iter().any(|item| agrees(record, item)) => {
                Err(Refusal::NotInHomeTrash(record.info.clone()))
            }
            Match::Gone => Err(Refusal::Gone),
        }
    }

    fn write_down(&self, restored: &[Trashed], attempts: &[Attempt], extra: &[String]) -> Recorded {
        if restored.is_empty() {
            return Recorded::Nothing;
        }
        let mut problems = extra.to_vec();
        for attempt in attempts {
            if let Fate::Refused(why) = &attempt.fate {
                // Written as the page shows it, with `~` for home, so the
                // line reads the same as the confirmation did.
                problems.push(format!(
                    "{}: {}",
                    attempt.name,
                    why.describe(&self.roots.home)
                ));
            }
        }
        let run = Run {
            version: history::VERSION,
            at: history::now(),
            entries: Vec::new(),
            operations: Vec::new(),
            problems,
            restored: restored.to_vec(),
        };
        match self.history.append(&run) {
            Ok(()) => Recorded::Run(run.at),
            Err(error) => Recorded::Failed(format!(
                "could not write to {}: {error}",
                self.history.path().display()
            )),
        }
    }
}

/// The name a confirmation shows. A folder that went whole keeps the name
/// it was ticked under; a file among several is named by where it returns.
fn label(entry: &crate::history::Entry, trashed: &Trashed) -> String {
    if entry.trashed.len() == 1 && entry.path == trashed.original {
        return entry.name.clone();
    }
    trashed
        .original
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| entry.name.clone())
}

/// Where the three paths a move needs have been checked to be.
struct Located {
    info: PathBuf,
    payload: PathBuf,
    original: PathBuf,
}

/// How a record relates to the trash entries it was shown.
enum Match<'a> {
    /// The `.trashinfo`, the origin and the deletion time all agree.
    Exact(&'a trash::TrashItem),
    /// Something shares the name or part of the record, and is not it.
    Replaced,
    /// Nothing in the trash is this, or wearing its name.
    Gone,
}

/// Whether the trash's own fields are the recorded ones.
///
/// All three have to agree. The trash reuses a name once the previous file
/// has left, so a matching `.trashinfo` path with a different time is a
/// different file that arrived later.
fn agrees(record: &Trashed, item: &trash::TrashItem) -> bool {
    Path::new(&item.id) == record.info
        && item.original_path() == record.original
        && item.time_deleted == record.deleted
}

fn identify<'a>(record: &Trashed, items: &'a [trash::TrashItem]) -> Match<'a> {
    if let Some(item) = items.iter().find(|item| agrees(record, item)) {
        return Match::Exact(item);
    }
    let name = record.original.file_name();
    let same = items.iter().any(|item| {
        Path::new(&item.id) == record.info
            || item.original_path() == record.original
            || name.is_some_and(|name| item.name == name)
    });
    if same { Match::Replaced } else { Match::Gone }
}

/// Check the destination and the copy in the trash, without moving either.
fn screen(roots: &Roots, record: &Trashed, item: &trash::TrashItem) -> Result<Located, Refusal> {
    // `identify` already required this. Asked again because the move uses
    // these paths and nothing else, and a mismatch here would put a file
    // back somewhere the record does not name.
    if !agrees(record, item) {
        return Err(Refusal::Replaced);
    }
    let original = record.original.clone();
    lexical(&original)?;
    if !original.starts_with(&roots.home) || original == roots.home {
        return Err(Refusal::OutsideHome(original));
    }
    // Parents only. The leaf is supposed to be absent, and a symlink
    // sitting on the leaf is something occupying the path, which is a
    // different refusal from a symlink that would redirect the write.
    real_ancestors(&original)?;
    let parent = original
        .parent()
        .ok_or_else(|| Refusal::NoParent(original.clone()))?;
    match std::fs::symlink_metadata(parent) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(Refusal::NoParent(parent.to_owned())),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(Refusal::NoParent(parent.to_owned()));
        }
        Err(error) => {
            return Err(Refusal::Failed {
                path: parent.to_owned(),
                reason: error.to_string(),
            });
        }
    }
    match std::fs::symlink_metadata(&original) {
        Ok(_) => return Err(Refusal::Occupied { path: original }),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(Refusal::Failed {
                path: original,
                reason: error.to_string(),
            });
        }
    }

    let info = PathBuf::from(&item.id);
    if !info.starts_with(&roots.home) {
        return Err(Refusal::NotInHomeTrash(info));
    }
    lexical(&info)?;
    real_ancestors(&info)?;
    match std::fs::symlink_metadata(&info) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(Refusal::Symlink(info));
        }
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Err(Refusal::NotInHomeTrash(info)),
        Err(error) if error.kind() == ErrorKind::NotFound => return Err(Refusal::Gone),
        Err(error) => {
            return Err(Refusal::Failed {
                path: info,
                reason: error.to_string(),
            });
        }
    }

    let payload = payload_of(&info)?;
    if !payload.starts_with(&roots.home) {
        return Err(Refusal::NotInHomeTrash(payload));
    }
    lexical(&payload)?;
    real_ancestors(&payload)?;
    match std::fs::symlink_metadata(&payload) {
        // A symlink here is the thing that was trashed, and it is moved as
        // a link. Following it would put back whatever it points at.
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => return Err(Refusal::Gone),
        Err(error) => {
            return Err(Refusal::Failed {
                path: payload,
                reason: error.to_string(),
            });
        }
    }

    Ok(Located {
        info,
        payload,
        original,
    })
}

/// Absolute, and with no `..` to resolve. Resolving would follow a symlink.
fn lexical(path: &Path) -> Result<(), Refusal> {
    if !path.is_absolute() {
        return Err(Refusal::NotAbsolute(path.to_owned()));
    }
    if path.components().any(|part| part == Component::ParentDir) {
        return Err(Refusal::Climbing(path.to_owned()));
    }
    Ok(())
}

/// Every directory above `path` exists and is not a symlink.
fn real_ancestors(path: &Path) -> Result<(), Refusal> {
    for ancestor in path.ancestors().skip(1) {
        match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(Refusal::Symlink(ancestor.to_owned()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(Refusal::NoParent(ancestor.to_owned()));
            }
            Err(error) => {
                return Err(Refusal::Failed {
                    path: ancestor.to_owned(),
                    reason: error.to_string(),
                });
            }
        }
    }
    Ok(())
}

/// The file the `.trashinfo` names, beside it in `files/`.
///
/// The stem is one component. The trash names `film.mkv.trashinfo` after
/// the file it holds, and `film.2.mkv.trashinfo` when that name was taken;
/// either stem is the name under `files/`, and neither is allowed to be a
/// path.
fn payload_of(info: &Path) -> Result<PathBuf, Refusal> {
    let stem = info
        .file_stem()
        .filter(|stem| !stem.is_empty() && *stem != OsStr::new(".") && *stem != OsStr::new(".."))
        .filter(|stem| !stem.as_bytes().contains(&b'/'))
        .ok_or_else(|| Refusal::NotInHomeTrash(info.to_owned()))?;
    let info_dir = info
        .parent()
        .filter(|dir| dir.file_name() == Some(OsStr::new("info")))
        .ok_or_else(|| Refusal::NotInHomeTrash(info.to_owned()))?;
    let trash_dir = info_dir
        .parent()
        .ok_or_else(|| Refusal::NotInHomeTrash(info.to_owned()))?;
    Ok(trash_dir.join("files").join(stem))
}

/// Move the payload back and drop the `.trashinfo`.
///
/// The directories are opened with `O_NOFOLLOW`, and the rename will not
/// replace an existing name. A symlink swapped in as the parent fails the
/// open; a file that appears at the destination fails the rename. Neither
/// is overwritten.
///
/// `Ok(Some(_))` means the file is back and the trash's record of it is
/// still there. That is still a success: the file is home, and saying
/// otherwise would invite a second attempt at a path that is now taken.
fn move_back(located: &Located) -> Result<Option<String>, Refusal> {
    let from_dir = open_directory(
        located
            .payload
            .parent()
            .ok_or_else(|| Refusal::NotInHomeTrash(located.payload.clone()))?,
    )?;
    let to_dir = open_directory(
        located
            .original
            .parent()
            .ok_or_else(|| Refusal::NoParent(located.original.clone()))?,
    )?;
    let from_name = located
        .payload
        .file_name()
        .ok_or_else(|| Refusal::NotInHomeTrash(located.payload.clone()))?;
    let to_name = located
        .original
        .file_name()
        .ok_or_else(|| Refusal::NoParent(located.original.clone()))?;

    match rustix::fs::renameat_with(
        &from_dir,
        from_name,
        &to_dir,
        to_name,
        rustix::fs::RenameFlags::NOREPLACE,
    ) {
        Ok(()) => {}
        Err(error) if error == rustix::io::Errno::EXIST => {
            return Err(Refusal::Occupied {
                path: located.original.clone(),
            });
        }
        Err(error) if error == rustix::io::Errno::NOENT => return Err(Refusal::Gone),
        Err(error) if error == rustix::io::Errno::LOOP => {
            return Err(Refusal::Symlink(located.original.clone()));
        }
        Err(error) => {
            return Err(Refusal::Failed {
                path: located.original.clone(),
                reason: error.to_string(),
            });
        }
    }
    drop(from_dir);
    drop(to_dir);

    match std::fs::symlink_metadata(&located.info) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Ok(Some(format!(
                "{} was put back, and its .trashinfo was left because it is a symlink",
                located.original.display(),
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Ok(Some(format!(
                "{} was put back, and its .trashinfo could not be read: {error}",
                located.original.display(),
            )));
        }
    }

    // The payload has already moved. This removes the trash's record of it,
    // and only that file, at the path matched against the home trash a
    // moment ago. `unlink` removes the name; a symlink here was refused
    // above, so this cannot delete whatever a link points at.
    #[expect(
        clippy::disallowed_methods,
        reason = "the .trashinfo is removed only after the file it describes has been moved back, and only that one file, whose path was matched against the home trash at this moment"
    )]
    if let Err(error) = std::fs::remove_file(&located.info) {
        return Ok(Some(format!(
            "{} was put back, and its .trashinfo could not be removed: {error}",
            located.original.display(),
        )));
    }
    Ok(None)
}

fn open_directory(path: &Path) -> Result<rustix::fd::OwnedFd, Refusal> {
    rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::NOFOLLOW,
        rustix::fs::Mode::empty(),
    )
    .map_err(|error| {
        if error == rustix::io::Errno::LOOP {
            Refusal::Symlink(path.to_owned())
        } else if error == rustix::io::Errno::NOENT || error == rustix::io::Errno::NOTDIR {
            Refusal::NoParent(path.to_owned())
        } else {
            Refusal::Failed {
                path: path.to_owned(),
                reason: error.to_string(),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execute::Executor;
    use crate::plan::{Disposal, Plan};
    use crate::size::Size;
    use std::os::unix::fs::symlink;
    use std::time::Duration;

    fn item(id: &str, original: &str, deleted: i64) -> trash::TrashItem {
        let original = Path::new(original);
        trash::TrashItem {
            id: OsStr::new(id).to_owned(),
            name: original.file_name().unwrap().to_owned(),
            original_parent: original.parent().unwrap().to_owned(),
            time_deleted: deleted,
        }
    }

    fn record(id: &str, original: &str, deleted: i64) -> Trashed {
        Trashed {
            info: PathBuf::from(id),
            original: PathBuf::from(original),
            deleted,
        }
    }

    #[test]
    fn a_record_matches_only_when_the_info_the_origin_and_the_time_all_agree() {
        let items = vec![item("/t/info/film.mkv.trashinfo", "/home/x/film.mkv", 10)];
        let exact = record("/t/info/film.mkv.trashinfo", "/home/x/film.mkv", 10);

        assert!(matches!(identify(&exact, &items), Match::Exact(_)));
        assert!(matches!(
            identify(
                &record("/t/info/film.2.mkv.trashinfo", "/home/x/film.mkv", 10),
                &items
            ),
            Match::Replaced
        ));
        assert!(matches!(
            identify(
                &record("/t/info/film.mkv.trashinfo", "/home/x/other.mkv", 10),
                &items
            ),
            Match::Replaced
        ));
        assert!(matches!(
            identify(
                &record("/t/info/film.mkv.trashinfo", "/home/x/film.mkv", 11),
                &items
            ),
            Match::Replaced
        ));
    }

    #[test]
    fn the_same_name_with_a_different_record_is_not_that_file() {
        // The previous film.mkv left the trash, and a different one arrived
        // under the name the trash had freed.
        let items = vec![item("/t/info/film.mkv.trashinfo", "/home/x/film.mkv", 50)];
        let earlier = record("/t/info/film.mkv.trashinfo", "/home/x/film.mkv", 10);

        assert!(matches!(identify(&earlier, &items), Match::Replaced));
    }

    #[test]
    fn a_record_that_is_simply_absent_is_gone() {
        let items = vec![item("/t/info/notes.txt.trashinfo", "/home/x/notes.txt", 1)];
        let film = record("/t/info/film.mkv.trashinfo", "/home/x/film.mkv", 10);

        assert!(matches!(identify(&film, &items), Match::Gone));
    }

    /// Set on the process [`in_fixture`] runs in.
    const TRASH_FIXTURE: &str = "LIMPID_TEST_TRASH_FIXTURE";

    #[test]
    fn restores_are_tried_against_a_fixture_trash() {
        // The trash crate finds the trash through `XDG_DATA_HOME` and
        // nothing else. The child has its own, pointed at the fixture, so
        // this never lists or moves anything in the real trash. `HOME` is
        // pointed there too, in case a path falls back to it.
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "in_fixture::",
                "--include-ignored",
                "--test-threads=1",
                "--nocapture",
            ])
            .env(TRASH_FIXTURE, fixture.path())
            .env("XDG_DATA_HOME", &roots.data)
            .env("HOME", &roots.home)
            .output()
            .unwrap();

        let said = String::from_utf8_lossy(&output.stdout);
        let err = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{said}\n{err}");
        assert!(said.contains("10 passed"), "{said}\n{err}");
    }

    mod in_fixture {
        use super::*;

        fn roots() -> Roots {
            let fixture = std::env::var_os(TRASH_FIXTURE).expect("set by the parent test");
            let roots = Roots::under(Path::new(&fixture));
            assert_eq!(
                std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
                Some(roots.data.clone()),
                "the trash must be the fixture, never the real one",
            );
            roots
        }

        fn write(path: &Path, bytes: &[u8]) {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }

        /// Send paths to the trash and return the run that did it.
        ///
        /// A short pause keeps two runs from sharing a millisecond, which
        /// is the only thing that tells them apart.
        fn send(roots: &Roots, paths: &[&Path]) -> u64 {
            std::thread::sleep(Duration::from_millis(5));
            let plan = Plan::from_chosen(
                paths
                    .iter()
                    .map(|path| ((*path).to_path_buf(), Size::new(1, 1))),
                Disposal::Trash,
            );
            let outcome = Executor::applying(roots).run(&plan);
            assert!(outcome.is_clean(), "{:?}", outcome.problems);
            let Recorded::Run(at) = outcome.recorded else {
                panic!("recorded, not {:?}", outcome.recorded);
            };
            at
        }

        fn only(roots: &Roots, at: u64) -> Trashed {
            let read = History::at(roots).read().unwrap();
            let matches: Vec<&Run> = read.runs.iter().filter(|run| run.at == at).collect();
            let [run] = matches.as_slice() else {
                panic!("one run at {at}, not {}", matches.len());
            };
            let [entry] = run.entries.as_slice() else {
                panic!("one entry, not {:?}", run.entries);
            };
            let [trashed] = entry.trashed.as_slice() else {
                panic!("one trash record, not {:?}", entry.trashed);
            };
            trashed.clone()
        }

        fn files_of(info: &Path) -> PathBuf {
            payload_of(info).unwrap()
        }

        fn purge(info: &Path) {
            let files = files_of(info);
            let metadata = std::fs::symlink_metadata(&files).unwrap();
            if metadata.is_dir() {
                std::fs::remove_dir_all(files).unwrap();
            } else {
                std::fs::remove_file(files).unwrap();
            }
            std::fs::remove_file(info).unwrap();
        }

        /// Make the live `.trashinfo` claim a different moment, so a record
        /// of the previous one cannot agree with it.
        fn rewind(info: &Path) {
            let text = std::fs::read_to_string(info).unwrap();
            let mut out = String::new();
            for line in text.lines() {
                if let Some(rest) = line.strip_prefix("DeletionDate=") {
                    assert!(!rest.starts_with("1999"), "{line}");
                    out.push_str("DeletionDate=1999-01-01T00:00:00\n");
                } else {
                    out.push_str(line);
                    out.push('\n');
                }
            }
            std::fs::write(info, out).unwrap();
        }

        fn fate(report: &Report) -> &Fate {
            assert_eq!(report.attempts.len(), 1, "{:?}", report.attempts);
            &report.attempts[0].fate
        }

        #[test]
        #[ignore = "run by restores_are_tried_against_a_fixture_trash, in a process whose trash is a fixture"]
        fn restoring_puts_a_trashed_file_back() {
            let roots = roots();
            let film = roots.home("Back/film.mkv");
            write(&film, b"the film");
            let at = send(&roots, &[&film]);
            let record = only(&roots, at);
            assert!(!film.exists());
            assert!(record.info.exists());

            let report = Restorer::applying(&roots).run(at).unwrap();

            assert!(report.applied);
            assert_eq!(fate(&report), &Fate::PutBack);
            assert_eq!(std::fs::read(&film).unwrap(), b"the film");
            assert!(!record.info.exists());
            assert!(!files_of(&record.info).exists());
            let read = History::at(&roots).read().unwrap();
            assert!(read.was_restored(&record));
            assert!(read.runs[0].summary().contains("put 1 back"));
            assert!(matches!(report.recorded, Recorded::Run(_)));
        }

        #[test]
        #[ignore = "run by restores_are_tried_against_a_fixture_trash, in a process whose trash is a fixture"]
        fn restoring_puts_a_trashed_folder_back() {
            let roots = roots();
            let folder = roots.home("Back/old-project");
            write(&folder.join("src/main.rs"), b"fn main() {}");
            write(&folder.join("notes.txt"), b"hi");
            let at = send(&roots, &[&folder]);
            let record = only(&roots, at);
            assert_eq!(record.original, folder);
            assert!(!folder.exists());

            let report = Restorer::applying(&roots).run(at).unwrap();

            assert_eq!(fate(&report), &Fate::PutBack);
            assert_eq!(
                std::fs::read(folder.join("src/main.rs")).unwrap(),
                b"fn main() {}"
            );
            assert_eq!(std::fs::read(folder.join("notes.txt")).unwrap(), b"hi");
            assert!(!files_of(&record.info).exists());
        }

        #[test]
        #[ignore = "run by restores_are_tried_against_a_fixture_trash, in a process whose trash is a fixture"]
        fn an_occupied_original_path_is_left_alone() {
            let roots = roots();
            let notes = roots.home("Back/notes.txt");
            write(&notes, b"original");
            let at = send(&roots, &[&notes]);
            let record = only(&roots, at);
            write(&notes, b"now mine");

            let before = History::at(&roots).read().unwrap().runs.len();
            let report = Restorer::applying(&roots).run(at).unwrap();

            assert_eq!(
                fate(&report),
                &Fate::Refused(Refusal::Occupied {
                    path: notes.clone(),
                })
            );
            assert_eq!(std::fs::read(&notes).unwrap(), b"now mine");
            assert!(record.info.exists());
            assert_eq!(std::fs::read(files_of(&record.info)).unwrap(), b"original");
            // A refusal is not a run. The tests share one history, so what
            // matters is that this one added nothing.
            let read = History::at(&roots).read().unwrap();
            assert_eq!(read.runs.len(), before);
            assert!(!read.was_restored(&record));
        }

        #[test]
        #[ignore = "run by restores_are_tried_against_a_fixture_trash, in a process whose trash is a fixture"]
        fn a_removed_trash_entry_is_refused() {
            let roots = roots();
            let gone = roots.home("Back/gone.bin");
            write(&gone, b"gone");
            let at = send(&roots, &[&gone]);
            let record = only(&roots, at);
            purge(&record.info);

            let report = Restorer::applying(&roots).run(at).unwrap();

            assert_eq!(fate(&report), &Fate::Refused(Refusal::Gone));
            assert!(!gone.exists());
        }

        #[test]
        #[ignore = "run by restores_are_tried_against_a_fixture_trash, in a process whose trash is a fixture"]
        fn a_trash_entry_replaced_under_the_same_name_is_refused() {
            let roots = roots();
            let same = roots.home("Back/same.bin");
            write(&same, b"first");
            let first = send(&roots, &[&same]);
            let earlier = only(&roots, first);
            purge(&earlier.info);
            write(&same, b"second");
            let second = send(&roots, &[&same]);
            let later = only(&roots, second);
            rewind(&later.info);

            let report = Restorer::applying(&roots).run(first).unwrap();

            assert_eq!(fate(&report), &Fate::Refused(Refusal::Replaced));
            assert!(!same.exists());
            assert_eq!(std::fs::read(files_of(&later.info)).unwrap(), b"second");
        }

        #[test]
        #[ignore = "run by restores_are_tried_against_a_fixture_trash, in a process whose trash is a fixture"]
        fn a_restored_entry_cannot_be_restored_twice() {
            let roots = roots();
            let once = roots.home("Back/once.bin");
            write(&once, b"once");
            let at = send(&roots, &[&once]);
            let record = only(&roots, at);
            let saved_info = std::fs::read(&record.info).unwrap();
            let saved_file = std::fs::read(files_of(&record.info)).unwrap();
            let file_in_trash = files_of(&record.info);

            let first = Restorer::applying(&roots).run(at).unwrap();
            assert_eq!(fate(&first), &Fate::PutBack);
            assert_eq!(std::fs::read(&once).unwrap(), b"once");

            // The same record, put back into the trash by hand. Matching
            // the trash alone would move it again; the history says it has
            // already come back, and that is what is believed.
            std::fs::create_dir_all(record.info.parent().unwrap()).unwrap();
            std::fs::create_dir_all(file_in_trash.parent().unwrap()).unwrap();
            std::fs::write(&record.info, &saved_info).unwrap();
            std::fs::write(&file_in_trash, &saved_file).unwrap();

            let second = Restorer::applying(&roots).run(at).unwrap();

            assert_eq!(fate(&second), &Fate::Refused(Refusal::AlreadyRestored));
            assert_eq!(std::fs::read(&once).unwrap(), b"once");
            assert!(record.info.exists());
            assert_eq!(std::fs::read(&file_in_trash).unwrap(), b"once");
        }

        #[test]
        #[ignore = "run by restores_are_tried_against_a_fixture_trash, in a process whose trash is a fixture"]
        fn a_dry_run_puts_nothing_back() {
            let roots = roots();
            let dry = roots.home("Back/dry.bin");
            write(&dry, b"dry");
            let at = send(&roots, &[&dry]);
            let record = only(&roots, at);

            let before = History::at(&roots).read().unwrap().runs.len();
            let report = Restorer::dry_run(&roots).run(at).unwrap();

            assert!(!report.applied);
            assert_eq!(fate(&report), &Fate::PutBack);
            assert_eq!(report.recorded, Recorded::Nothing);
            assert!(!dry.exists());
            assert!(record.info.exists());
            assert_eq!(History::at(&roots).read().unwrap().runs.len(), before);
        }

        #[test]
        #[ignore = "run by restores_are_tried_against_a_fixture_trash, in a process whose trash is a fixture"]
        fn a_symlink_on_the_way_back_is_not_followed() {
            let roots = roots();
            let film = roots.home("Way/film.mkv");
            write(&film, b"film");
            let at = send(&roots, &[&film]);
            let record = only(&roots, at);
            std::fs::remove_dir(film.parent().unwrap()).unwrap();
            let elsewhere = roots.home("elsewhere");
            write(&elsewhere.join("secret"), b"secret");
            symlink(&elsewhere, roots.home("Way")).unwrap();

            let report = Restorer::applying(&roots).run(at).unwrap();

            assert!(matches!(fate(&report), Fate::Refused(Refusal::Symlink(_))));
            assert_eq!(std::fs::read(elsewhere.join("secret")).unwrap(), b"secret");
            assert!(!elsewhere.join("film.mkv").exists());
            assert!(record.info.exists());
            assert_eq!(std::fs::read(files_of(&record.info)).unwrap(), b"film");
        }

        #[test]
        #[ignore = "run by restores_are_tried_against_a_fixture_trash, in a process whose trash is a fixture"]
        fn a_symlink_occupying_the_original_path_is_not_overwritten() {
            let roots = roots();
            let film = roots.home("Leaf/film.mkv");
            write(&film, b"film");
            let at = send(&roots, &[&film]);
            let record = only(&roots, at);
            let secret = roots.home("Leaf/secret");
            write(&secret, b"secret");
            symlink(&secret, &film).unwrap();

            let report = Restorer::applying(&roots).run(at).unwrap();

            assert_eq!(
                fate(&report),
                &Fate::Refused(Refusal::Occupied { path: film.clone() })
            );
            assert_eq!(std::fs::read(&secret).unwrap(), b"secret");
            assert!(film.symlink_metadata().unwrap().file_type().is_symlink());
            assert_eq!(std::fs::read(files_of(&record.info)).unwrap(), b"film");
        }

        #[test]
        #[ignore = "run by restores_are_tried_against_a_fixture_trash, in a process whose trash is a fixture"]
        fn a_path_outside_the_home_directory_is_not_written() {
            let roots = roots();
            // Next to the home, on the same filesystem, so the trash crate
            // puts it in the fixture's home trash rather than a trash on
            // some other disk.
            let outside = roots.home.parent().unwrap().join("outside/secret.txt");
            write(&outside, b"nope");
            trash::delete(&outside).unwrap();
            assert!(!outside.exists());
            let listed = trash::os_limited::list().unwrap();
            let item = listed
                .iter()
                .find(|item| item.original_path() == outside)
                .expect("the file is in the fixture trash");
            let trashed = Trashed {
                info: PathBuf::from(&item.id),
                original: item.original_path(),
                deleted: item.time_deleted,
            };
            let at = 42;
            History::at(&roots)
                .append(&Run {
                    version: history::VERSION,
                    at,
                    entries: vec![crate::history::Entry {
                        name: "secret.txt".to_owned(),
                        path: outside.clone(),
                        disposal: Disposal::Trash,
                        size: Size::new(4, 4),
                        files: 1,
                        trashed: vec![trashed.clone()],
                    }],
                    operations: Vec::new(),
                    problems: Vec::new(),
                    restored: Vec::new(),
                })
                .unwrap();

            let report = Restorer::applying(&roots).run(at).unwrap();

            assert_eq!(
                fate(&report),
                &Fate::Refused(Refusal::OutsideHome(outside.clone()))
            );
            assert!(!outside.exists());
            assert!(trashed.info.exists());
        }
    }
}
