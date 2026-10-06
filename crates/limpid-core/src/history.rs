//! What Limpid removed, when, and where it went.
//!
//! One file, `$XDG_STATE_HOME/limpid/history.jsonl`, with a line for every
//! run that removed anything. It is what lets the trash be somewhere Limpid
//! can give things back from, rather than only somewhere it sends them.
//!
//! **The executor writes it, not whoever called the executor.** An applying
//! executor records its own runs, so no front-end can remove something and
//! forget to say so. A dry run records nothing, because it removed nothing.
//!
//! **One line per run, appended, never rewritten.** A crash in the middle of
//! a write costs at most the line being written, and a line that cannot be
//! read costs only itself. Rewriting the file to add a run would put every
//! earlier run at risk for the sake of the newest.
//!
//! **What went to the trash is written down as the trash wrote it down.** The
//! trash renames whatever arrives under a name already taken there, and
//! resolves the folder it came from, so the path that was ticked is not
//! enough to find it again. Each thing that went is recorded with the
//! trash's own record of it.

use std::collections::HashSet;
use std::ffi::OsString;
use std::io::{Read as _, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::paths::Roots;
use crate::plan::Disposal;
use crate::privileged::{Completed, Report};
use crate::size::Size;

/// Where the file lives, relative to the state directory.
///
/// State rather than config or data: the XDG spec's own example of state is
/// "actions history", and it is not something a person edits or backs up
/// to carry to another machine.
pub const FILE: &str = "limpid/history.jsonl";

/// The format this build writes, and the newest it can read.
///
/// A line with a higher version was written by a newer Limpid. It is left
/// in the file and not shown, rather than shown wrong.
pub const VERSION: u32 = 1;

/// One run that removed something.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Run {
    /// The format this line was written in.
    pub version: u32,
    /// When it started, in milliseconds since the Unix epoch. Also what
    /// tells one run from another.
    pub at: u64,
    /// What came out of each path in the plan that anything came out of.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<Entry>,
    /// What the privileged helper did.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<Completed>,
    /// What could not be done, as it was reported at the time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<String>,
}

impl Run {
    /// Everything that came out, together.
    pub fn reclaimed(&self) -> Size {
        self.entries.iter().map(|entry| entry.size).sum()
    }

    /// Files that came out, together.
    pub fn files(&self) -> u64 {
        self.entries.iter().map(|entry| entry.files).sum()
    }

    /// When it happened, in local time, the way a person would write it.
    pub fn when(&self) -> String {
        use chrono::TimeZone as _;

        i64::try_from(self.at)
            .ok()
            .and_then(|at| chrono::Local.timestamp_millis_opt(at).single())
            .map_or_else(
                || "at an unknown time".to_owned(),
                |at| at.format("%-d %b %Y, %H:%M").to_string(),
            )
    }

    /// How much, in how many files, and which way it went: the line a run
    /// is listed under.
    pub fn summary(&self) -> String {
        if self.entries.is_empty() {
            return "by the privileged helper".to_owned();
        }
        let files = match self.files() {
            1 => "1 file".to_owned(),
            files => format!("{files} files"),
        };
        format!(
            "{} in {files}, {}",
            crate::size::human(self.reclaimed().on_disk),
            self.disposal().map_or(
                "some moved to the trash and some removed permanently",
                Disposal::describe
            ),
        )
    }

    /// Whether every entry went the same way, and which way that was.
    pub fn disposal(&self) -> Option<Disposal> {
        let first = self.entries.first()?.disposal;
        self.entries
            .iter()
            .all(|entry| entry.disposal == first)
            .then_some(first)
    }
}

/// What came out of one path in a plan.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    /// The name shown when it was agreed to.
    pub name: String,
    /// The path in the plan: a directory that was emptied, or what went
    /// whole.
    #[serde(with = "exact")]
    pub path: PathBuf,
    /// How it went.
    pub disposal: Disposal,
    /// What came out.
    pub size: Size,
    /// How many files that was.
    pub files: u64,
    /// The trash's record of each thing that went there. Empty when it was
    /// deleted, and also when the trash could not be read back afterwards:
    /// it went, and the file manager can still restore it, but Limpid
    /// cannot say where it is.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trashed: Vec<Trashed>,
}

/// Something in the trash, as the trash recorded it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Trashed {
    /// The `.trashinfo` file the trash keeps for it.
    #[serde(with = "exact")]
    pub info: PathBuf,
    /// Where it came from, as the trash wrote it down.
    #[serde(with = "exact")]
    pub original: PathBuf,
    /// When the trash says it arrived, in seconds since the Unix epoch.
    pub deleted: i64,
}

/// Whether a run made it into the history.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Recorded {
    /// There was nothing to record: a dry run, or nothing was removed.
    #[default]
    Nothing,
    /// Written down, under this run's `at`.
    Run(u64),
    /// Done, but not written down. Said, because a history with a gap in it
    /// that nobody was told about is worse than no history.
    Failed(String),
}

/// What reading the history found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Read {
    /// Every run, newest first.
    pub runs: Vec<Run>,
    /// Lines that could not be understood: damaged, or written by a newer
    /// Limpid. Left in the file as they are, and not shown.
    pub skipped: usize,
}

/// The history file.
#[derive(Debug, Clone)]
pub struct History {
    path: PathBuf,
}

impl History {
    /// The history for these roots.
    pub fn at(roots: &Roots) -> Self {
        Self {
            path: roots.state.join(FILE),
        }
    }

    /// Where the file is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read every run.
    ///
    /// No file is an empty history. A line that cannot be read is counted
    /// and passed over, never a reason to show none of the others.
    pub fn read(&self) -> std::io::Result<Read> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Read::default());
            }
            Err(error) => return Err(error),
        };

        let mut read = Read::default();
        for line in bytes.split(|&byte| byte == b'\n') {
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            match serde_json::from_slice::<Run>(line) {
                Ok(run) if run.version <= VERSION => read.runs.push(run),
                _ => read.skipped += 1,
            }
        }
        read.runs.reverse();
        Ok(read)
    }

    /// Add a run to the end.
    ///
    /// Synced before returning: this is written straight after something
    /// was removed, and a record that a crash could take back is not one.
    pub fn append(&self, run: &Run) -> std::io::Result<()> {
        let mut line = serde_json::to_vec(run).map_err(std::io::Error::other)?;
        line.push(b'\n');

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(&self.path)?;

        // A last line without its newline is one a crash cut short. Written
        // straight after it, this line would join it and be lost with it.
        if unterminated(&mut file)? {
            line.insert(0, b'\n');
        }

        // One write, so another Limpid appending at the same moment cannot
        // land in the middle of this line.
        file.write_all(&line)?;
        file.sync_data()
    }

    /// Write down what an executor removed.
    pub(crate) fn record(
        &self,
        at: u64,
        removed: &[crate::execute::Removed],
        problems: &[crate::execute::Problem],
        before: Option<&HashSet<OsString>>,
    ) -> Recorded {
        let sent: Vec<PathBuf> = removed
            .iter()
            .flat_map(|removed| removed.trashed.iter().cloned())
            .collect();
        let mut found = match before {
            Some(before) if !sent.is_empty() => locate(before, &sent),
            _ => Vec::new(),
        }
        .into_iter();

        let entries = removed
            .iter()
            .map(|removed| Entry {
                name: removed.name.clone(),
                path: removed.path.clone(),
                disposal: removed.disposal,
                size: removed.size,
                files: removed.files,
                trashed: removed
                    .trashed
                    .iter()
                    .filter_map(|_| found.next().flatten())
                    .collect(),
            })
            .collect();

        let run = Run {
            version: VERSION,
            at,
            entries,
            operations: Vec::new(),
            problems: problems.iter().map(ToString::to_string).collect(),
        };
        match self.append(&run) {
            Ok(()) => Recorded::Run(at),
            Err(error) => Recorded::Failed(format!(
                "could not write to {}: {error}",
                self.path.display()
            )),
        }
    }

    /// Write down what the privileged helper did, if it did anything.
    pub(crate) fn record_operations(&self, at: u64, report: &Report) -> Recorded {
        if !report.completed.iter().any(|done| done.succeeded) {
            return Recorded::Nothing;
        }
        let run = Run {
            version: VERSION,
            at,
            entries: Vec::new(),
            operations: report.completed.clone(),
            problems: Vec::new(),
        };
        match self.append(&run) {
            Ok(()) => Recorded::Run(at),
            Err(error) => Recorded::Failed(format!(
                "could not write to {}: {error}",
                self.path.display()
            )),
        }
    }
}

/// The time now, the way a run is stamped.
pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// Whether a file ends part-way through a line.
fn unterminated(file: &mut std::fs::File) -> std::io::Result<bool> {
    if file.metadata()?.len() == 0 {
        return Ok(false);
    }
    file.seek(SeekFrom::End(-1))?;
    let mut last = [0];
    file.read_exact(&mut last)?;
    Ok(last[0] != b'\n')
}

/// Everything in the trash now, by the trash's own name for it. `None` when
/// the trash cannot be listed, which only costs the ability to find what is
/// about to be sent there.
pub(crate) fn in_the_trash() -> Option<HashSet<OsString>> {
    trash::os_limited::list()
        .ok()
        .map(|items| items.into_iter().map(|item| item.id).collect())
}

/// Find each of `sent` among what arrived in the trash since `before`.
fn locate(before: &HashSet<OsString>, sent: &[PathBuf]) -> Vec<Option<Trashed>> {
    match trash::os_limited::list() {
        Ok(now) => identify(before, now, sent),
        Err(_) => vec![None; sent.len()],
    }
}

/// Match what was sent to the trash with what arrived there.
///
/// Only what arrived counts, so something of the same name trashed last
/// week is never mistaken for it. Among the arrivals, the name decides; the
/// trash keeps the original name even when it has to rename the file. When
/// two arrivals share a name — two `notes.txt` from different folders — the
/// folder decides, and when even that is ambiguous nothing is claimed: a
/// record that cannot find something is honest, one that points at the
/// wrong thing would put it back in the wrong place.
fn identify(
    before: &HashSet<OsString>,
    now: Vec<trash::TrashItem>,
    sent: &[PathBuf],
) -> Vec<Option<Trashed>> {
    let mut arrived: Vec<trash::TrashItem> = now
        .into_iter()
        .filter(|item| !before.contains(&item.id))
        .collect();

    sent.iter()
        .map(|path| {
            let name = path.file_name()?;
            let named: Vec<usize> = arrived
                .iter()
                .enumerate()
                .filter(|(_, item)| item.name == name)
                .map(|(index, _)| index)
                .collect();
            let index = match named.as_slice() {
                [only] => *only,
                several => {
                    let mut same = several
                        .iter()
                        .copied()
                        .filter(|&index| arrived[index].original_path() == *path);
                    match (same.next(), same.next()) {
                        (Some(index), None) => index,
                        _ => return None,
                    }
                }
            };
            let item = arrived.swap_remove(index);
            Some(Trashed {
                original: item.original_path(),
                info: PathBuf::from(item.id),
                deleted: item.time_deleted,
            })
        })
        .collect()
}

/// A path written exactly.
///
/// A Linux file name is bytes, not text, and serde refuses a path that is
/// not UTF-8. One such name would cost the record of the whole run it was
/// in, so a path that is text is written as text, and one that is not as
/// its bytes.
mod exact {
    use std::ffi::OsString;
    use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
    use std::path::{Path, PathBuf};

    use serde::{Deserialize as _, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
        match path.to_str() {
            Some(text) => serializer.serialize_str(text),
            None => serializer.collect_seq(path.as_os_str().as_bytes()),
        }
    }

    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Written {
        Text(String),
        Bytes(Vec<u8>),
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PathBuf, D::Error> {
        Ok(match Written::deserialize(deserializer)? {
            Written::Text(text) => PathBuf::from(text),
            Written::Bytes(bytes) => PathBuf::from(OsString::from_vec(bytes)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt as _;

    fn run(at: u64, name: &str) -> Run {
        Run {
            version: VERSION,
            at,
            entries: vec![Entry {
                name: name.to_owned(),
                path: PathBuf::from(format!("/home/x/{name}")),
                disposal: Disposal::Trash,
                size: Size::new(4096, 4096),
                files: 1,
                trashed: Vec::new(),
            }],
            operations: Vec::new(),
            problems: Vec::new(),
        }
    }

    fn history() -> (tempfile::TempDir, History) {
        let fixture = tempfile::tempdir().unwrap();
        let history = History::at(&Roots::under(fixture.path()));
        (fixture, history)
    }

    fn item(id: &str, original: &str, deleted: i64) -> trash::TrashItem {
        let original = Path::new(original);
        trash::TrashItem {
            id: OsString::from(id),
            name: original.file_name().unwrap().to_owned(),
            original_parent: original.parent().unwrap().to_owned(),
            time_deleted: deleted,
        }
    }

    #[test]
    fn there_is_no_history_until_something_is_removed() {
        let (_fixture, history) = history();

        assert_eq!(history.read().unwrap(), Read::default());
        assert!(!history.path().exists());
    }

    #[test]
    fn runs_are_read_back_newest_first() {
        let (_fixture, history) = history();
        history.append(&run(1, "first.iso")).unwrap();
        history.append(&run(2, "second.iso")).unwrap();

        let read = history.read().unwrap();

        assert_eq!(read.runs, vec![run(2, "second.iso"), run(1, "first.iso")]);
        assert_eq!(read.skipped, 0);
    }

    #[test]
    fn the_history_lives_with_the_other_state_not_the_settings() {
        let fixture = tempfile::tempdir().unwrap();
        let roots = Roots::under(fixture.path());

        assert_eq!(
            History::at(&roots).path(),
            roots.state.join("limpid/history.jsonl")
        );
    }

    #[test]
    fn a_damaged_line_costs_only_itself() {
        let (_fixture, history) = history();
        history.append(&run(1, "first.iso")).unwrap();
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(history.path())
            .unwrap();
        file.write_all(b"{\"version\":1,\"at\":\n").unwrap();
        history.append(&run(3, "third.iso")).unwrap();

        let read = history.read().unwrap();

        assert_eq!(read.runs, vec![run(3, "third.iso"), run(1, "first.iso")]);
        assert_eq!(read.skipped, 1);
    }

    #[test]
    fn a_line_cut_short_by_a_crash_does_not_take_the_next_one_with_it() {
        let (_fixture, history) = history();
        history.append(&run(1, "first.iso")).unwrap();
        // No newline: the write that was interrupted.
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(history.path())
            .unwrap();
        file.write_all(b"{\"version\":1,\"at\":2,\"entr").unwrap();

        history.append(&run(3, "third.iso")).unwrap();
        let read = history.read().unwrap();

        assert_eq!(read.runs, vec![run(3, "third.iso"), run(1, "first.iso")]);
        assert_eq!(read.skipped, 1);
    }

    #[test]
    fn a_run_from_a_newer_limpid_is_left_alone_and_not_shown_wrong() {
        let (_fixture, history) = history();
        let mut newer = run(2, "newer.iso");
        newer.version = VERSION + 1;
        history.append(&run(1, "first.iso")).unwrap();
        history.append(&newer).unwrap();

        let read = history.read().unwrap();

        assert_eq!(read.runs, vec![run(1, "first.iso")]);
        assert_eq!(read.skipped, 1);
        let text = std::fs::read_to_string(history.path()).unwrap();
        assert!(text.contains("newer.iso"), "{text}");
    }

    #[test]
    fn a_file_name_that_is_not_text_is_kept_exactly() {
        let (_fixture, history) = history();
        let mut odd = run(1, "x");
        // Latin-1 "é", which is not UTF-8 on its own.
        odd.entries[0].path = PathBuf::from(OsString::from_vec(b"/home/x/caf\xe9.txt".to_vec()));
        history.append(&odd).unwrap();

        assert_eq!(history.read().unwrap().runs, vec![odd]);
    }

    #[test]
    fn only_what_arrived_during_the_run_is_claimed() {
        // The same file was trashed once before, from the file manager.
        let before = HashSet::from([OsString::from("/t/info/film.mkv.trashinfo")]);
        let now = vec![
            item("/t/info/film.mkv.trashinfo", "/home/x/film.mkv", 100),
            item("/t/info/film.2.mkv.trashinfo", "/home/x/film.mkv", 200),
        ];

        let found = identify(&before, now, &[PathBuf::from("/home/x/film.mkv")]);

        assert_eq!(
            found,
            vec![Some(Trashed {
                info: PathBuf::from("/t/info/film.2.mkv.trashinfo"),
                original: PathBuf::from("/home/x/film.mkv"),
                deleted: 200,
            })]
        );
    }

    #[test]
    fn two_arrivals_of_the_same_name_are_told_apart_by_where_they_came_from() {
        let now = vec![
            item("/t/info/notes.txt.trashinfo", "/home/x/a/notes.txt", 1),
            item("/t/info/notes.2.txt.trashinfo", "/home/x/b/notes.txt", 1),
        ];
        let sent = [
            PathBuf::from("/home/x/b/notes.txt"),
            PathBuf::from("/home/x/a/notes.txt"),
        ];

        let found = identify(&HashSet::new(), now, &sent);

        assert_eq!(
            found[0].as_ref().unwrap().info,
            Path::new("/t/info/notes.2.txt.trashinfo")
        );
        assert_eq!(
            found[1].as_ref().unwrap().info,
            Path::new("/t/info/notes.txt.trashinfo")
        );
    }

    #[test]
    fn an_arrival_that_cannot_be_told_apart_is_not_claimed() {
        // Same name, and neither came from where this was sent: the trash
        // resolved the folder to somewhere else. Guessing could put one of
        // them back in the other's place.
        let now = vec![
            item("/t/info/notes.txt.trashinfo", "/var/home/x/a/notes.txt", 1),
            item(
                "/t/info/notes.2.txt.trashinfo",
                "/var/home/x/b/notes.txt",
                1,
            ),
        ];

        let found = identify(
            &HashSet::new(),
            now,
            &[PathBuf::from("/home/x/a/notes.txt")],
        );

        assert_eq!(found, vec![None]);
    }

    #[test]
    fn a_single_arrival_is_found_even_where_the_trash_resolved_the_folder() {
        // `/home` a link to `/var/home`: the trash writes down where the file
        // really was, and there is only one thing it can be.
        let now = vec![item(
            "/t/info/film.mkv.trashinfo",
            "/var/home/x/film.mkv",
            1,
        )];

        let found = identify(&HashSet::new(), now, &[PathBuf::from("/home/x/film.mkv")]);

        assert_eq!(
            found[0].as_ref().unwrap().original,
            Path::new("/var/home/x/film.mkv")
        );
    }

    #[test]
    fn a_run_says_when_it_happened_in_words() {
        let when = run(1_759_660_320_000, "x").when();

        assert!(when.contains("2025"), "{when}");
        assert!(when.contains(':'), "{when}");
    }

    #[test]
    fn a_run_that_went_two_ways_does_not_claim_one() {
        let mut mixed = run(1, "a");
        let mut deleted = mixed.entries[0].clone();
        deleted.disposal = Disposal::Delete;
        mixed.entries.push(deleted);

        assert_eq!(run(1, "a").disposal(), Some(Disposal::Trash));
        assert_eq!(mixed.disposal(), None);
        assert!(
            mixed
                .summary()
                .ends_with("some moved to the trash and some removed permanently"),
            "{}",
            mixed.summary()
        );
    }

    #[test]
    fn a_run_is_summed_up_by_how_much_how_many_and_which_way() {
        assert_eq!(
            run(1, "a").summary(),
            "4.00 KiB in 1 file, moved to the trash"
        );

        let helper = Run {
            entries: Vec::new(),
            ..run(1, "a")
        };
        assert_eq!(helper.summary(), "by the privileged helper");
    }

    #[test]
    fn what_the_helper_did_is_recorded_as_it_said_it() {
        let (_fixture, history) = history();
        let report = Report {
            completed: vec![Completed {
                operation: crate::privileged::Operation::TrimPackageCache { keep: 3 },
                succeeded: true,
                detail: "14 packages removed".to_owned(),
            }],
        };

        assert_eq!(history.record_operations(7, &report), Recorded::Run(7));
        let runs = history.read().unwrap().runs;
        assert_eq!(runs[0].operations, report.completed);
        assert!(runs[0].entries.is_empty());
    }

    #[test]
    fn an_operation_run_is_only_recorded_when_something_was_done() {
        let (_fixture, history) = history();
        let failed = Report {
            completed: vec![Completed {
                operation: crate::privileged::Operation::RemoveCoredumps,
                succeeded: false,
                detail: "no".to_owned(),
            }],
        };

        assert_eq!(history.record_operations(1, &failed), Recorded::Nothing);
        assert!(history.read().unwrap().runs.is_empty());
    }
}
