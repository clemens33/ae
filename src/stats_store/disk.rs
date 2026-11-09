//! The maintained Statistics store on disk, under `<root>/stats/`: the one
//! writer of its payload and of `<root>/stats.lock`.
//!
//! Every decision here is a function from what a [`Dir`] holds (and, for a
//! publication, a fresh look at the journal) to a [`Plan`]: a list of file
//! operations applied in order. The product applies a plan whole on [`Files`]
//! under the root lock; tests and the fuzzer apply any prefix of it, which is
//! exactly a crash after that operation, and recover.
//!
//! A publication stages every file it rewrites, publishes `stats/intent`
//! naming the groups it touches, renames the primary group's evidence (the
//! commit point: the source's own evidence is its truth), then the other
//! groups' evidence, their session views and the root view, and removes the
//! intent. [`recovery`] finishes or undoes whatever a crash left: an
//! interrupted repair swap first, then leftover temps, then an intent, rebuilt
//! from the primary group's evidence as it stands; a primary staged but not
//! committed keeps its temp until the intent is gone. Every file a plan writes
//! is synced before the intent publishes; directories are not.
//!
//! A [`repair`] moves the payload aside whole for an empty one in the epoch
//! after the highest one a surviving document proves, and no prefix of it
//! removes that proof before the active epoch file carries it; the first build
//! is staged the same way, so a payload never exists without its epoch file.
//! A retired payload proving an epoch at or above the active one is damage
//! (amendment 17). A file that cannot be read is an IO error, never damage:
//! nothing is repaired over it.
//!
//! A source with nothing new is remembered by its journal's identity, length
//! and change times (amendment 16): while they stay the same, its next look
//! reads no byte. A change reads the witness again: the bytes just before the
//! cursor. A same-length rewrite further back than the witness is not seen.
//!
//! Lock order: a journal lock, then `stats.lock`, never the reverse. Nothing
//! here takes a journal lock; journals are read unlocked and witnessed.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::doc::{self, Group, Intent, Key, Own, Provenance, Root, SessionView, WITNESS};
use super::{Digest, Error, Evidence, LINE_LIMIT, Origin};
use crate::archive::is_canonical_uuid;
use crate::stats::{self, SourceGap, Tally, ThisSession, View};

/// The largest root view read or written.
pub const ROOT_VIEW_CAP: u64 = 1 << 20;
/// The largest session view read or written.
pub const SESSION_VIEW_CAP: u64 = 1 << 20;
/// The largest group evidence read or written: today's retained budget.
pub const EVIDENCE_CAP: u64 = 64 << 20;
/// The largest intent read or written.
pub const INTENT_CAP: u64 = 64 << 10;
/// The largest epoch read or written.
pub const EPOCH_CAP: u64 = 32;

const PAYLOAD: &str = "stats";
const STAGING: &str = "stats.new";
const RETIRED: &str = "stats.old.";
const LOCK: &str = "stats.lock";
const EPOCH: &str = "stats/epoch";
const INTENT: &str = "stats/intent";
const ROOT: &str = "stats/view";
const SESSIONS: &str = "stats/session";
const TEMP: &str = ".tmp";

fn evidence_name(key: &Key) -> String {
    format!("{SESSIONS}/{}.evidence", doc::stem(key))
}

fn view_name(key: &Key) -> String {
    format!("{SESSIONS}/{}.view", doc::stem(key))
}

fn temp(name: &str) -> String {
    format!("{name}{TEMP}")
}

fn retired_name(epoch: u64) -> String {
    format!("{RETIRED}{epoch}")
}

/// The epoch a retired payload's name carries.
fn retired(name: &str) -> Option<u64> {
    let digits = name.strip_prefix(RETIRED)?;
    let canonical = !digits.is_empty()
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'));
    canonical.then(|| digits.parse().ok()).flatten()
}

// --- files ------------------------------------------------------------------

/// One capped read of a payload file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Read {
    Absent,
    Bytes(Vec<u8>),
    /// Longer than the cap: never handed to a parser.
    Oversized,
    /// A file that is there and could not be read: an IO error, not damage.
    Unreadable(String),
    /// A node no payload file has the shape of: a directory, a link.
    Invalid(String),
}

/// One file operation, named relative to the root: `stats/view`,
/// `stats/session/<key>.evidence`, `stats.new/epoch`, `stats.old.<E>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// Create a file that must not exist yet, with these bytes.
    Write {
        name: String,
        bytes: Vec<u8>,
    },
    /// Rename, replacing a file at `to`.
    Rename {
        from: String,
        to: String,
    },
    /// Remove a file.
    Remove(String),
    MakeDir(String),
    /// Remove a directory and everything under it.
    RemoveTree(String),
}

/// Where the payload lives.
pub trait Dir {
    /// `name` read whole, or [`Read::Oversized`] past `cap` bytes.
    fn read(&mut self, name: &str, cap: u64) -> Read;
    /// # Errors
    ///
    /// The operation's own failure, as the filesystem reports it.
    fn apply(&mut self, op: &Op) -> io::Result<()>;
    /// The names directly under `dir` (`""` is the root), sorted; none for a
    /// directory that is not there.
    ///
    /// # Errors
    ///
    /// A directory that cannot be listed.
    fn list(&mut self, dir: &str) -> io::Result<Vec<String>>;
}

/// File operations in the order they must happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub ops: Vec<Op>,
    generation: Option<u64>,
}

impl Plan {
    pub(crate) const fn of(ops: Vec<Op>) -> Self {
        Self {
            ops,
            generation: None,
        }
    }

    /// Apply `ops[..upto]`; `upto` past the end is the whole plan. Stopping
    /// short is a crash after that operation.
    ///
    /// # Errors
    ///
    /// The first operation that fails; those before it stay applied.
    pub fn apply(&self, dir: &mut dyn Dir, upto: usize) -> io::Result<()> {
        for op in self.ops.iter().take(upto) {
            dir.apply(op)?;
        }
        Ok(())
    }

    /// The generation a publication plan makes.
    #[must_use]
    pub const fn generation(&self) -> Option<u64> {
        self.generation
    }
}

/// The payload as real files under a root: the product's [`Dir`]. Reads go
/// through the capped file door; writes create new files only, mode 0600.
///
/// A writer that holds `stats.lock` never reads a payload file's content or
/// lists a directory in the steady state (ruling 19, option A): it first runs
/// its whole decision WARM, without the lock, recording what each read and
/// listing found with the status of what it opened; under the lock, HELD, a
/// read or listing is answered from that record only while a status read of
/// the node still matches it, else the attempt is STALE: nothing is opened,
/// nothing is written, and the writer tries again from a new warm. Only the
/// replay of an owed intent and a repair read under the lock (the EXCEPTION).
#[derive(Debug, Clone)]
pub struct Files {
    root: PathBuf,
    snap: Snap,
}

/// How [`Files`] reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Mode {
    /// Plain reads: readers, and writers outside a transaction.
    #[default]
    Open,
    /// Reads recorded, no write.
    Warm,
    /// Answers from the record while a status read matches it.
    Held,
    /// Reads under the lock in a replay or a repair, recorded.
    Exception,
}

/// What a warm read of one name found.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Found {
    Absent,
    /// A regular file's bytes, with the status of the handle they came from.
    Bytes(crate::store::Identity, Vec<u8>),
    /// A regular file opened and refused (over its cap, a read failed).
    Opened(crate::store::Identity, Read),
    /// What the node's own status said: not a regular file, or no status;
    /// with the proof of that status: the full status of the node the warm
    /// lstat classified, or that lstat's error.
    Shape(DirProof, Read),
}

/// One real capped read: its answer, the identity of the handle it read,
/// and the node status that classified the path (or that status's error).
struct Warmed {
    read: Read,
    opened: Option<crate::store::Identity>,
    node: Result<std::fs::Metadata, io::ErrorKind>,
}

/// What a warm listing of one directory found.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Listed {
    Absent,
    Names(DirStatus, Vec<String>),
    /// A listing that failed, with what the directory's own status said
    /// before it: served again while that proof still holds.
    Failed(DirProof, Failure),
}

/// What a directory's own status (lstat) said: the proof a recorded
/// listing failure is checked against under the lock.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DirProof {
    /// A directory, with its status.
    Status(DirStatus),
    /// A node that is no directory, by its own full status (its mode
    /// carries its type), whatever kind it is.
    Other(DirStatus),
    /// The status itself failed.
    Error(io::ErrorKind, String),
}

impl DirProof {
    fn of(path: &Path) -> Self {
        match crate::store::node_meta(path) {
            Ok(meta) => Self::of_meta(&meta),
            Err(why) => Self::Error(why.kind(), why.to_string()),
        }
    }

    fn of_meta(meta: &std::fs::Metadata) -> Self {
        DirStatus::of(meta).map_or_else(|| Self::Other(DirStatus::node(meta)), Self::Status)
    }
}

/// A listing's error, kept so the same error can be answered again.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Failure {
    kind: io::ErrorKind,
    os: Option<i32>,
    text: String,
}

impl Failure {
    fn of(why: &io::Error) -> Self {
        Self {
            kind: why.kind(),
            os: why.raw_os_error(),
            text: why.to_string(),
        }
    }

    fn error(&self) -> io::Error {
        self.os.map_or_else(
            || io::Error::new(self.kind, self.text.clone()),
            io::Error::from_raw_os_error,
        )
    }
}

/// A directory's status: an entry added, removed or renamed moves its times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DirStatus {
    dev: u64,
    ino: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
    mode: u32,
}

impl DirStatus {
    fn of(meta: &std::fs::Metadata) -> Option<Self> {
        meta.is_dir().then(|| Self::node(meta))
    }

    /// The same status fields of a node of any kind (lstat: a symlink is
    /// its own node).
    fn node(meta: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt as _;
        Self {
            dev: meta.dev(),
            ino: meta.ino(),
            mtime: (meta.mtime(), meta.mtime_nsec()),
            ctime: (meta.ctime(), meta.ctime_nsec()),
            mode: meta.mode(),
        }
    }
}

/// One transaction's record of what its warm found.
#[derive(Debug, Clone, Default)]
struct Snap {
    mode: Mode,
    files: BTreeMap<String, Found>,
    dirs: BTreeMap<String, Listed>,
    /// Bytes this attempt wrote, by name, until renamed.
    written: BTreeMap<String, Vec<u8>>,
    /// Bytes recorded: file contents.
    recorded: u64,
    /// A held answer did not match: this attempt writes nothing.
    stale: bool,
    /// A held attempt applied an operation.
    wrote: bool,
}

fn stale() -> io::Error {
    io::Error::other("a dependency changed since it was read")
}

impl Files {
    #[must_use]
    pub fn at(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            snap: Snap::default(),
        }
    }

    /// The path of a payload name: plain components only, never out of the
    /// root.
    fn path(&self, name: &str) -> io::Result<PathBuf> {
        if name.is_empty() {
            return Ok(self.root.clone());
        }
        let plain = name
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..");
        if !plain {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{name:?} is not a payload name"),
            ));
        }
        Ok(self.root.join(name))
    }

    /// Start a warm: a fresh record, reads recorded, writes refused.
    pub(super) fn warm(&mut self) {
        self.snap = Snap {
            mode: Mode::Warm,
            ..Snap::default()
        };
    }

    /// Hold: answer from the record.
    pub(super) fn hold(&mut self) {
        self.snap.mode = Mode::Held;
        self.snap.stale = false;
        self.snap.wrote = false;
        self.snap.written.clear();
    }

    /// Back to plain reads, the record dropped.
    pub(super) fn open(&mut self) {
        self.snap = Snap::default();
    }

    /// Bytes the warm recorded.
    pub(super) const fn recorded(&self) -> u64 {
        self.snap.recorded
    }

    /// Whether a held answer did not match its record.
    pub(super) const fn stale(&self) -> bool {
        self.snap.stale
    }

    /// Whether this held attempt applied an operation.
    pub(super) const fn wrote(&self) -> bool {
        self.snap.wrote
    }

    /// Run `body` as the exception: under the lock, reads open and are
    /// recorded. In any other mode than held, `body` runs as it is.
    pub(super) fn exception<T>(&mut self, body: impl FnOnce(&mut Self) -> T) -> T {
        if self.snap.mode != Mode::Held {
            return body(self);
        }
        self.snap.mode = Mode::Exception;
        let done = body(self);
        self.snap.mode = Mode::Held;
        done
    }

    fn mark_stale(&mut self) {
        self.snap.stale = true;
    }

    /// A real capped read, with what it opened.
    #[cfg_attr(
        not(test),
        allow(
            unused_variables,
            reason = "the exception count reaches only the tests"
        )
    )]
    fn read_now(path: &Path, cap: u64, exception: bool) -> Warmed {
        let seen = crate::store::read_capped_identified(path, cap);
        #[cfg(test)]
        if seen.opened.is_some() {
            observe::payload(true, exception);
        }
        let read = match seen.read {
            Ok(crate::store::SourceRead::Absent) => Read::Absent,
            Ok(crate::store::SourceRead::Ready(bytes)) => Read::Bytes(bytes),
            Ok(crate::store::SourceRead::Invalid(why)) => Read::Invalid(why),
            Ok(crate::store::SourceRead::Unreadable(why)) => Read::Unreadable(why),
            Err(crate::store::Oversized) => Read::Oversized,
        };
        Warmed {
            read,
            opened: seen.opened,
            node: seen.node,
        }
    }

    /// What a warm read found, recorded.
    fn record(&mut self, name: &str, seen: &Warmed) {
        let read = &seen.read;
        // The proof of a shape answer is the lstat that classified the path.
        let proof = match (&seen.node, read) {
            (Ok(meta), _) => DirProof::of_meta(meta),
            (Err(kind), Read::Unreadable(text)) => DirProof::Error(*kind, text.clone()),
            (Err(kind), _) => DirProof::Error(*kind, String::new()),
        };
        let found = match (read, seen.opened) {
            (Read::Absent, _) => Found::Absent,
            (Read::Bytes(bytes), Some(identity)) => {
                let width = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                self.snap.recorded = self.snap.recorded.saturating_add(width);
                Found::Bytes(identity, bytes.clone())
            }
            (other, Some(identity)) => Found::Opened(identity, other.clone()),
            // Bytes with no handle status prove nothing: never served.
            (Read::Bytes(_), None) => return,
            // A regular node that would not open (permissions): the same
            // answer while the status that classified it stays the same.
            (Read::Unreadable(_), None) => match seen
                .node
                .as_ref()
                .ok()
                .filter(|meta| crate::store::nonregular_leg(meta.file_type()).is_none())
                .and_then(crate::store::Identity::from_meta)
            {
                Some(identity) => Found::Opened(identity, read.clone()),
                None => Found::Shape(proof, read.clone()),
            },
            (other, None) => Found::Shape(proof, other.clone()),
        };
        self.snap.files.insert(name.to_owned(), found);
    }

    /// Under the lock: the recorded answer for `name` while its status says
    /// the same, else stale. Never opens.
    fn held_read(&mut self, path: &Path, name: &str, cap: u64) -> Read {
        let now = match crate::store::node_meta(path) {
            Ok(meta) => match crate::store::nonregular_leg(meta.file_type()) {
                Some(what) => {
                    Found::Shape(DirProof::of_meta(&meta), Read::Invalid(what.to_owned()))
                }
                None => match crate::store::Identity::from_meta(&meta) {
                    Some(identity) => Found::Opened(identity, Read::Absent),
                    None => Found::Shape(
                        DirProof::of_meta(&meta),
                        Read::Unreadable("no status".to_owned()),
                    ),
                },
            },
            Err(why) if why.kind() == io::ErrorKind::NotFound => Found::Absent,
            Err(why) => {
                let text = why.to_string();
                Found::Shape(
                    DirProof::Error(why.kind(), text.clone()),
                    Read::Unreadable(text),
                )
            }
        };
        let answer = match (self.snap.files.get(name), &now) {
            (Some(Found::Absent), Found::Absent) => Some(Read::Absent),
            // The same answer while the classifying status proves the same
            // node (or the same lstat error).
            (Some(Found::Shape(was_proof, was)), Found::Shape(is_proof, is))
                if was_proof == is_proof && was == is =>
            {
                Some(was.clone())
            }
            (Some(Found::Bytes(was, bytes)), Found::Opened(is, _)) if was == is => {
                Some(if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > cap {
                    Read::Oversized
                } else {
                    Read::Bytes(bytes.clone())
                })
            }
            (Some(Found::Opened(was, read)), Found::Opened(is, _)) if was == is => {
                Some(read.clone())
            }
            _ => None,
        };
        answer.unwrap_or_else(|| {
            self.mark_stale();
            Read::Unreadable("a dependency changed since it was read".to_owned())
        })
    }

    /// A real listing, with the directory's status taken before it.
    fn list_now(&mut self, dir: &str) -> (io::Result<Vec<String>>, Listed) {
        let path = match self.path(dir) {
            Ok(path) => path,
            Err(why) => {
                let failed = Listed::Failed(
                    DirProof::Error(why.kind(), why.to_string()),
                    Failure::of(&why),
                );
                return (Err(why), failed);
            }
        };
        let proof = DirProof::of(&path);
        #[cfg(test)]
        {
            observe::listed(self.snap.mode == Mode::Exception);
        }
        let paths = match crate::archive::read_entries(&path) {
            Ok(paths) => paths,
            Err(why) if why.kind() == io::ErrorKind::NotFound => {
                return (Ok(Vec::new()), Listed::Absent);
            }
            Err(why) => {
                let failed = Listed::Failed(proof, Failure::of(&why));
                return (Err(why), failed);
            }
        };
        let mut names: Vec<String> = paths
            .iter()
            .filter_map(|path| path.file_name()?.to_str().map(ToOwned::to_owned))
            .collect();
        names.sort();
        // Listed, but its status proves no directory (a symlink to one, or
        // a status read that failed): held, that is an error, never names.
        let listed = match proof {
            DirProof::Status(status) => Listed::Names(status, names.clone()),
            other => Listed::Failed(other, Failure::of(&io::Error::other("no directory status"))),
        };
        (Ok(names), listed)
    }

    /// Under the lock: the recorded listing of `dir` while its status says
    /// the same, else stale. Never lists.
    fn held_list(&mut self, dir: &str) -> io::Result<Vec<String>> {
        let path = self.path(dir)?;
        let now = DirProof::of(&path);
        let answer = match (self.snap.dirs.get(dir), &now) {
            (Some(Listed::Absent), DirProof::Error(kind, _))
                if *kind == io::ErrorKind::NotFound =>
            {
                Some(Ok(Vec::new()))
            }
            (Some(Listed::Names(was, names)), DirProof::Status(is)) if is == was => {
                Some(Ok(names.clone()))
            }
            // The same failure while the directory's status says the same.
            (Some(Listed::Failed(was, failure)), is) if was == is => Some(Err(failure.error())),
            _ => None,
        };
        answer.unwrap_or_else(|| {
            self.mark_stale();
            Err(stale())
        })
    }

    /// After an operation applied under the lock: the record follows it, so
    /// the rest of this attempt reads its own writes by status alone.
    fn follow(&mut self, op: &Op) {
        match op {
            Op::Write { name, bytes } => {
                self.snap.written.insert(name.clone(), bytes.clone());
                self.restat_file(name, Some(bytes.clone()));
                self.entered(name);
            }
            Op::Rename { from, to } => {
                let bytes =
                    self.snap
                        .written
                        .remove(from)
                        .or_else(|| match self.snap.files.get(from) {
                            Some(Found::Bytes(_, bytes)) => Some(bytes.clone()),
                            _ => None,
                        });
                self.snap.files.insert(from.clone(), Found::Absent);
                self.moved(from, to);
                if self.snap.dirs.contains_key(to) {
                    self.restat_dir(to);
                } else {
                    self.restat_file(to, bytes);
                }
                self.left(from);
                self.entered(to);
            }
            Op::Remove(name) => {
                self.snap.written.remove(name);
                self.snap.files.insert(name.clone(), Found::Absent);
                self.left(name);
            }
            Op::MakeDir(name) => {
                self.snap
                    .dirs
                    .insert(name.clone(), Listed::Names(DirStatus::zero(), Vec::new()));
                self.restat_dir(name);
                self.entered(name);
            }
            Op::RemoveTree(name) => {
                let inside = format!("{name}/");
                self.snap.files.retain(|key, _| !key.starts_with(&inside));
                self.snap.dirs.retain(|key, _| !key.starts_with(&inside));
                self.snap.files.insert(name.clone(), Found::Absent);
                self.snap.dirs.insert(name.clone(), Listed::Absent);
                self.left(name);
            }
        }
    }

    /// Records under `from` now under `to`: a directory renamed.
    fn moved(&mut self, from: &str, to: &str) {
        let inside = format!("{from}/");
        let target = format!("{to}/");
        let files: Vec<(String, Found)> = self
            .snap
            .files
            .iter()
            .filter(|(key, _)| key.starts_with(&inside))
            .map(|(key, found)| (key.clone(), found.clone()))
            .collect();
        self.snap.files.retain(|key, _| !key.starts_with(&target));
        for (key, found) in files {
            self.snap.files.remove(&key);
            let rest = key.get(inside.len()..).unwrap_or_default();
            self.snap.files.insert(format!("{target}{rest}"), found);
        }
        let dirs: Vec<(String, Listed)> = self
            .snap
            .dirs
            .iter()
            .filter(|(key, _)| *key == from || key.starts_with(&inside))
            .map(|(key, listed)| (key.clone(), listed.clone()))
            .collect();
        self.snap
            .dirs
            .retain(|key, _| key != to && !key.starts_with(&target));
        for (key, listed) in dirs {
            self.snap.dirs.remove(&key);
            let renamed = if key == from {
                to.to_owned()
            } else {
                format!("{target}{}", key.get(inside.len()..).unwrap_or_default())
            };
            self.snap.dirs.insert(renamed, listed);
        }
        if self.snap.dirs.contains_key(to) {
            self.snap.dirs.insert(from.to_owned(), Listed::Absent);
        }
    }

    /// The status of `name` written by this attempt, with its bytes.
    fn restat_file(&mut self, name: &str, bytes: Option<Vec<u8>>) {
        let found = self
            .path(name)
            .ok()
            .and_then(|path| crate::store::node_meta(&path).ok())
            .and_then(|meta| crate::store::Identity::from_meta(&meta))
            .zip(bytes)
            .map(|(identity, bytes)| Found::Bytes(identity, bytes));
        match found {
            Some(found) => self.snap.files.insert(name.to_owned(), found),
            None => self.snap.files.remove(name),
        };
    }

    /// The status of a directory this attempt changed, its names kept.
    fn restat_dir(&mut self, dir: &str) {
        let status = self
            .path(dir)
            .ok()
            .and_then(|path| crate::store::node_meta(&path).ok())
            .and_then(|meta| DirStatus::of(&meta));
        let names = match self.snap.dirs.get(dir) {
            Some(Listed::Names(_, names)) => Some(names.clone()),
            _ => None,
        };
        match (status, names) {
            (Some(status), Some(names)) => {
                self.snap
                    .dirs
                    .insert(dir.to_owned(), Listed::Names(status, names));
            }
            _ => {
                self.snap.dirs.remove(dir);
            }
        }
    }

    /// `name` now in its directory's recorded listing.
    fn entered(&mut self, name: &str) {
        self.relisted(name, true);
    }

    /// `name` no longer in its directory's recorded listing.
    fn left(&mut self, name: &str) {
        self.relisted(name, false);
    }

    fn relisted(&mut self, name: &str, present: bool) {
        let (dir, leaf) = name.rsplit_once('/').unwrap_or(("", name));
        if let Some(Listed::Names(_, names)) = self.snap.dirs.get_mut(dir) {
            names.retain(|have| have != leaf);
            if present {
                names.push(leaf.to_owned());
                names.sort();
            }
        }
        if matches!(self.snap.dirs.get(dir), Some(Listed::Absent)) && present {
            self.snap.dirs.remove(dir);
        }
        self.restat_dir(dir);
    }
}

impl DirStatus {
    const fn zero() -> Self {
        Self {
            dev: 0,
            ino: 0,
            mtime: (0, 0),
            ctime: (0, 0),
            mode: 0,
        }
    }
}

impl Files {
    /// One read in this `Files`' mode, before the test observer sees it.
    fn read_unobserved(&mut self, name: &str, cap: u64) -> Read {
        let path = match self.path(name) {
            Ok(path) => path,
            Err(why) => return Read::Unreadable(why.to_string()),
        };
        match self.snap.mode {
            Mode::Open => Self::read_now(&path, cap, false).read,
            Mode::Held => self.held_read(&path, name, cap),
            Mode::Warm | Mode::Exception => {
                let warmed = Self::read_now(&path, cap, self.snap.mode == Mode::Exception);
                self.record(name, &warmed);
                warmed.read
            }
        }
    }
}

impl Dir for Files {
    fn read(&mut self, name: &str, cap: u64) -> Read {
        // A test's injected IO fault, taken only by a read under the lock in
        // the exception: a warm's discarded run never consumes it.
        #[cfg(test)]
        {
            if self.snap.mode == Mode::Exception && observe::read_faulted(name) {
                observe::NAMES.with(|names| names.borrow_mut().push((name.to_owned(), 0)));
                return Read::Unreadable("injected payload read IO".to_owned());
            }
        }
        let read = self.read_unobserved(name, cap);
        #[cfg(test)]
        {
            let bytes = match &read {
                Read::Bytes(bytes) => u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                _ => 0,
            };
            observe::NAMES.with(|names| names.borrow_mut().push((name.to_owned(), bytes)));
        }
        read
    }

    fn apply(&mut self, op: &Op) -> io::Result<()> {
        use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
        match self.snap.mode {
            // A warm writes nothing: what follows reads the files as they
            // are; the held run follows its own writes.
            Mode::Warm => return Ok(()),
            Mode::Held | Mode::Exception if self.snap.stale => return Err(stale()),
            _ => {}
        }
        match op {
            Op::Write { name, bytes } => {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(self.path(name)?)?;
                #[cfg(test)]
                {
                    observe::file(FileEvent::Created(name.clone()));
                }
                file.write_all(bytes)?;
                synced(&file, name)
            }
            Op::Rename { from, to } => std::fs::rename(self.path(from)?, self.path(to)?),
            Op::Remove(name) => std::fs::remove_file(self.path(name)?),
            Op::MakeDir(name) => std::fs::DirBuilder::new()
                .mode(0o700)
                .create(self.path(name)?),
            Op::RemoveTree(name) => std::fs::remove_dir_all(self.path(name)?),
        }?;
        #[cfg(test)]
        {
            observe::applied(op);
        }
        if matches!(self.snap.mode, Mode::Held | Mode::Exception) {
            self.snap.wrote = true;
            self.follow(op);
        }
        Ok(())
    }

    fn list(&mut self, dir: &str) -> io::Result<Vec<String>> {
        match self.snap.mode {
            Mode::Open => self.list_now(dir).0,
            Mode::Held => self.held_list(dir),
            Mode::Warm | Mode::Exception => {
                let (names, listed) = self.list_now(dir);
                self.snap.dirs.insert(dir.to_owned(), listed);
                names
            }
        }
    }
}

/// Make a written file's bytes durable before any plan step names it: every
/// staged write is synced before the intent publishes. Directories are not
/// synced; a file cut short still reads as damage by its end record.
#[cfg_attr(
    not(test),
    allow(
        unused_variables,
        reason = "the name and time are observed only by tests"
    )
)]
fn synced(file: &std::fs::File, name: &str) -> io::Result<()> {
    #[cfg(test)]
    {
        if observe::faulted(name) {
            return Err(io::Error::other(format!("{name}: injected sync failure")));
        }
    }
    let started = std::time::Instant::now();
    file.sync_all()?;
    #[cfg(test)]
    {
        observe::file(FileEvent::Synced {
            name: name.to_owned(),
            took: started.elapsed(),
        });
    }
    Ok(())
}

/// What `Files` did on this thread, in order (`cfg(test)` only): see
/// [`take_file_events`].
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileEvent {
    /// After the exclusive create of this payload name.
    Created(String),
    /// After the real `sync_all` of this name returned, and how long it took.
    Synced {
        name: String,
        took: Duration,
    },
    Renamed {
        from: String,
        to: String,
    },
    Removed(String),
    MadeDir(String),
    RemovedTree(String),
}

/// What `Files` did on this thread since the last call, in order.
#[cfg(test)]
#[must_use]
pub fn take_file_events() -> Vec<FileEvent> {
    observe::FILES.with(std::cell::RefCell::take)
}

/// Make the next sync of `name` on this thread fail, before it would be
/// recorded as [`FileEvent::Synced`].
#[cfg(test)]
pub fn fail_next_sync(name: &str) {
    observe::SYNC_FAULT.with(|fault| *fault.borrow_mut() = Some(name.to_owned()));
}

/// Make the next [`Files`] read of payload `name` on this thread under the
/// lock in an exception answer an IO error, once. Warm, held and open reads
/// never take it.
#[cfg(test)]
pub fn fail_next_exception_read(name: &str) {
    observe::READ_FAULT.with(|fault| *fault.borrow_mut() = Some(name.to_owned()));
}

/// Journal bytes [`FileJournals`] returned on this thread (`cfg(test)` only).
#[cfg(test)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JournalReads {
    pub total: u64,
    /// Of `total`, those read while this thread held a [`Payload`] lock.
    pub under_lock: u64,
}

/// The journal bytes read on this thread since the last call.
#[cfg(test)]
#[must_use]
pub fn take_journal_reads() -> JournalReads {
    observe::JOURNALS.with(std::cell::Cell::take)
}

/// The test observers' state: per thread, so parallel tests never mix.
#[cfg(test)]
mod observe {
    use std::cell::{Cell, RefCell};

    use super::{Attempt, FileEvent, Hold, JournalReads, Op, PayloadReads};

    /// A spec's hook between a warm and its lock request.
    pub(super) type BeforeLock = Box<dyn FnMut(&std::path::Path, u32)>;

    thread_local! {
        pub(super) static FILES: RefCell<Vec<FileEvent>> = const { RefCell::new(Vec::new()) };
        /// Each `Files` read: the payload name and the bytes it returned.
        pub(super) static NAMES: RefCell<Vec<(String, u64)>> = const { RefCell::new(Vec::new()) };
        pub(super) static SYNC_FAULT: RefCell<Option<String>> = const { RefCell::new(None) };
        pub(super) static READ_FAULT: RefCell<Option<String>> = const { RefCell::new(None) };
        pub(super) static JOURNALS: Cell<JournalReads> = const {
            Cell::new(JournalReads { total: 0, under_lock: 0 })
        };
        /// Payload locks this thread holds.
        pub(super) static LOCKS: Cell<u64> = const { Cell::new(0) };
        pub(super) static PAYLOAD: Cell<PayloadReads> = const {
            Cell::new(PayloadReads {
                opened: 0,
                opened_under_lock: 0,
                opened_under_lock_exception: 0,
                listed: 0,
                listed_under_lock: 0,
                listed_under_lock_exception: 0,
            })
        };
        pub(super) static ATTEMPTS: RefCell<Vec<Attempt>> = const { RefCell::new(Vec::new()) };
        pub(super) static CAP: Cell<Option<u64>> = const { Cell::new(None) };
        pub(super) static HOLDS: RefCell<Vec<Hold>> = const { RefCell::new(Vec::new()) };
        pub(super) static BEFORE: RefCell<Option<BeforeLock>> = const { RefCell::new(None) };
    }

    pub(super) fn file(event: FileEvent) {
        FILES.with(|events| events.borrow_mut().push(event));
    }

    /// Whether the sync of `name` is the one a test made fail: taken once.
    pub(super) fn faulted(name: &str) -> bool {
        SYNC_FAULT.with(|fault| {
            let mut fault = fault.borrow_mut();
            let hit = fault.as_deref() == Some(name);
            if hit {
                *fault = None;
            }
            hit
        })
    }

    /// Whether this exception read of `name` is the one a test made fail:
    /// taken once.
    pub(super) fn read_faulted(name: &str) -> bool {
        READ_FAULT.with(|fault| {
            let mut fault = fault.borrow_mut();
            let hit = fault.as_deref() == Some(name);
            if hit {
                *fault = None;
            }
            hit
        })
    }

    /// Every operation but a write, which records its create itself.
    pub(super) fn applied(op: &Op) {
        let event = match op {
            Op::Write { .. } => return,
            Op::Rename { from, to } => FileEvent::Renamed {
                from: from.clone(),
                to: to.clone(),
            },
            Op::Remove(name) => FileEvent::Removed(name.clone()),
            Op::MakeDir(name) => FileEvent::MadeDir(name.clone()),
            Op::RemoveTree(name) => FileEvent::RemovedTree(name.clone()),
        };
        file(event);
    }

    /// A payload content open by `Files`; `exception` while it reads under
    /// the lock in a replay or a repair.
    pub(super) fn payload(opened: bool, exception: bool) {
        let locked = LOCKS.with(Cell::get) > 0;
        PAYLOAD.with(|reads| {
            let mut now = reads.get();
            if opened {
                now.opened += 1;
                if locked && exception {
                    now.opened_under_lock_exception += 1;
                } else if locked {
                    now.opened_under_lock += 1;
                }
            }
            reads.set(now);
        });
    }

    /// A directory listing by `Files`.
    pub(super) fn listed(exception: bool) {
        let locked = LOCKS.with(Cell::get) > 0;
        PAYLOAD.with(|reads| {
            let mut now = reads.get();
            now.listed += 1;
            if locked && exception {
                now.listed_under_lock_exception += 1;
            } else if locked {
                now.listed_under_lock += 1;
            }
            reads.set(now);
        });
    }

    pub(super) fn hold(hold: Hold) {
        HOLDS.with(|holds| holds.borrow_mut().push(hold));
    }

    pub(super) fn attempt(attempt: Attempt) {
        ATTEMPTS.with(|attempts| attempts.borrow_mut().push(attempt));
    }

    /// The snapshot cap in force on this thread.
    pub(super) fn cap() -> Option<u64> {
        CAP.with(Cell::get)
    }

    /// Run the spec's hook, if one is installed.
    pub(super) fn before(root: &std::path::Path, attempt: u32) {
        let hook = BEFORE.with(|held| held.borrow_mut().take());
        if let Some(mut hook) = hook {
            hook(root, attempt);
            BEFORE.with(|held| {
                let mut held = held.borrow_mut();
                if held.is_none() {
                    *held = Some(hook);
                }
            });
        }
    }

    pub(super) fn journal(bytes: usize) {
        let width = u64::try_from(bytes).unwrap_or(u64::MAX);
        let locked = LOCKS.with(Cell::get) > 0;
        JOURNALS.with(|reads| {
            let mut now = reads.get();
            now.total = now.total.saturating_add(width);
            if locked {
                now.under_lock = now.under_lock.saturating_add(width);
            }
            reads.set(now);
        });
    }

    pub(super) fn locked(by: i8) {
        LOCKS.with(|locks| {
            let now = locks.get();
            locks.set(if by > 0 {
                now.saturating_add(1)
            } else {
                now.saturating_sub(1)
            });
        });
    }
}

/// The payload in memory: test and fuzz stores. Behaves as [`Files`] does on
/// a POSIX filesystem: an exclusive create of an existing name, a rename onto
/// a file, a remove of a missing name.
#[derive(Debug, Clone, Default)]
pub struct Memory {
    files: BTreeMap<String, Vec<u8>>,
    dirs: BTreeSet<String>,
}

fn parent(name: &str) -> &str {
    name.rsplit_once('/').map_or("", |(parent, _)| parent)
}

fn under(name: &str, dir: &str) -> bool {
    name.strip_prefix(dir)
        .is_some_and(|rest| rest.starts_with('/'))
}

fn missing(name: &str) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("{name}: not found"))
}

impl Memory {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Put `bytes` at `name` whatever is there, its directories made.
    pub fn plant(&mut self, name: &str, bytes: &[u8]) {
        let mut dir = parent(name);
        while !dir.is_empty() {
            self.dirs.insert(dir.to_owned());
            dir = parent(dir);
        }
        self.files.insert(name.to_owned(), bytes.to_vec());
    }

    #[must_use]
    pub fn file(&self, name: &str) -> Option<&[u8]> {
        self.files.get(name).map(Vec::as_slice)
    }

    /// Every file's name.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.files.keys().cloned().collect()
    }

    fn has_dir(&self, dir: &str) -> bool {
        dir.is_empty() || self.dirs.contains(dir)
    }

    fn exists(&self, name: &str) -> bool {
        self.files.contains_key(name) || self.dirs.contains(name)
    }

    fn rename(&mut self, from: &str, to: &str) -> io::Result<()> {
        if !self.has_dir(parent(to)) {
            return Err(missing(to));
        }
        if self.files.contains_key(from) {
            if self.dirs.contains(to) {
                return Err(io::Error::other(format!("{to}: is a directory")));
            }
            let bytes = self.files.remove(from).unwrap_or_default();
            self.files.insert(to.to_owned(), bytes);
            return Ok(());
        }
        if !self.dirs.contains(from) {
            return Err(missing(from));
        }
        if self.files.contains_key(to) || under(to, from) {
            return Err(io::Error::other(format!("{to}: cannot replace")));
        }
        if self.dirs.contains(to) {
            let empty = !self.files.keys().any(|name| under(name, to))
                && !self.dirs.iter().any(|name| under(name, to));
            if !empty {
                return Err(io::Error::other(format!("{to}: directory not empty")));
            }
        }
        let moved = |name: &str| format!("{to}{}", &name[from.len()..]);
        let dirs: Vec<String> = self
            .dirs
            .iter()
            .filter(|name| *name == from || under(name, from))
            .cloned()
            .collect();
        for dir in dirs {
            self.dirs.remove(&dir);
            self.dirs.insert(moved(&dir));
        }
        let files: Vec<String> = self
            .files
            .keys()
            .filter(|name| under(name, from))
            .cloned()
            .collect();
        for name in files {
            let bytes = self.files.remove(&name).unwrap_or_default();
            self.files.insert(moved(&name), bytes);
        }
        Ok(())
    }
}

impl Dir for Memory {
    fn read(&mut self, name: &str, cap: u64) -> Read {
        match self.files.get(name) {
            Some(bytes) if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > cap => Read::Oversized,
            Some(bytes) => Read::Bytes(bytes.clone()),
            None if self.dirs.contains(name) => Read::Invalid(format!("{name}: a directory")),
            None => Read::Absent,
        }
    }

    fn apply(&mut self, op: &Op) -> io::Result<()> {
        match op {
            Op::Write { name, bytes } => {
                if !self.has_dir(parent(name)) {
                    return Err(missing(name));
                }
                if self.exists(name) {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!("{name}: exists"),
                    ));
                }
                self.files.insert(name.clone(), bytes.clone());
                Ok(())
            }
            Op::Rename { from, to } => self.rename(from, to),
            Op::Remove(name) => {
                if self.dirs.contains(name) {
                    return Err(io::Error::other(format!("{name}: is a directory")));
                }
                self.files
                    .remove(name)
                    .map(|_| ())
                    .ok_or_else(|| missing(name))
            }
            Op::MakeDir(name) => {
                if self.exists(name) {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!("{name}: exists"),
                    ));
                }
                if !self.has_dir(parent(name)) {
                    return Err(missing(name));
                }
                self.dirs.insert(name.clone());
                Ok(())
            }
            Op::RemoveTree(name) => {
                if !self.dirs.contains(name) {
                    return Err(missing(name));
                }
                self.dirs.retain(|dir| dir != name && !under(dir, name));
                self.files.retain(|file, _| !under(file, name));
                Ok(())
            }
        }
    }

    fn list(&mut self, dir: &str) -> io::Result<Vec<String>> {
        if !self.has_dir(dir) {
            return Ok(Vec::new());
        }
        let child = |name: &String| {
            (parent(name) == dir).then(|| {
                name.rsplit_once('/')
                    .map_or(name.as_str(), |(_, leaf)| leaf)
                    .to_owned()
            })
        };
        let mut names: Vec<String> = self
            .files
            .keys()
            .filter_map(child)
            .chain(self.dirs.iter().filter_map(child))
            .collect();
        names.sort();
        names.dedup();
        Ok(names)
    }
}

// --- journals ---------------------------------------------------------------

/// A bounded look at a journal: its identity, length and change times, and
/// the bytes read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub dev: u64,
    pub ino: u64,
    pub len: u64,
    /// Modification and status-change times, seconds and nanoseconds.
    pub changed: Changed,
    pub bytes: Vec<u8>,
}

/// A journal's modification and status-change times, seconds and
/// nanoseconds: an in-place rewrite moves the status change.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Changed {
    pub mtime: (i64, i64),
    pub ctime: (i64, i64),
}

impl Window {
    fn identity(&self) -> Identity {
        Identity {
            dev: self.dev,
            ino: self.ino,
            len: self.len,
            changed: self.changed,
        }
    }
}

/// What a quiet journal looked like (amendment 16): the same file, the same
/// length and the same change times read nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Identity {
    dev: u64,
    ino: u64,
    len: u64,
    changed: Changed,
}

/// Where sources' journals are read.
pub trait Journal {
    /// At most `max` bytes of `source`'s journal from `from`.
    ///
    /// # Errors
    ///
    /// The source's gap when its journal cannot be read.
    fn window(&mut self, source: &str, from: u64, max: u64) -> Result<Window, SourceGap>;

    /// Where `source` stands in the state root now, asked under the lock
    /// before a publication: [`Standing::Unknown`], no check, unless the
    /// journals know.
    fn standing(&mut self, _source: &str) -> Standing {
        Standing::Unknown
    }
}

/// What a source's place in the state root says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// Its directory is there.
    Present,
    /// It is not: removed, purged or renamed away.
    Gone,
    /// An archive under a publisher's or a purge's claim.
    Claimed,
    /// Not known: nothing is checked.
    Unknown,
}

/// Journals as real files: the product's [`Journal`]. `live/<name>` is
/// `<root>/sessions/<name>/events.jsonl` and `archive/<uuid>` the archived
/// journal, unless [`FileJournals::track`] named another path.
#[derive(Debug, Clone)]
pub struct FileJournals {
    root: PathBuf,
    paths: BTreeMap<String, PathBuf>,
    snap: Windows,
}

/// One transaction's journal windows, as [`Files`] records payload reads: a
/// held window is the recorded one while the journal's status says the same.
#[derive(Debug, Clone, Default)]
struct Windows {
    mode: Mode,
    seen: BTreeMap<(String, u64, u64), Window>,
    recorded: u64,
    stale: bool,
}

impl FileJournals {
    #[must_use]
    pub fn at(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            paths: BTreeMap::new(),
            snap: Windows::default(),
        }
    }

    /// Start a warm: windows read and recorded.
    pub(super) fn warm(&mut self) {
        self.snap = Windows {
            mode: Mode::Warm,
            ..Windows::default()
        };
    }

    /// Hold: windows answered from the record.
    pub(super) fn hold(&mut self) {
        self.snap.mode = Mode::Held;
        self.snap.stale = false;
    }

    /// Back to plain reads, the record dropped.
    pub(super) fn open(&mut self) {
        self.snap = Windows::default();
    }

    pub(super) const fn recorded(&self) -> u64 {
        self.snap.recorded
    }

    pub(super) const fn stale(&self) -> bool {
        self.snap.stale
    }

    /// Read `source` from `journal`.
    pub fn track(&mut self, source: &str, journal: PathBuf) {
        self.paths.insert(source.to_owned(), journal);
    }

    fn path(&self, source: &str) -> Option<PathBuf> {
        if let Some(path) = self.paths.get(source) {
            return Some(path.clone());
        }
        let plain =
            |name: &str| !name.is_empty() && name != "." && name != ".." && !name.contains('/');
        if let Some(name) = source.strip_prefix("live/").filter(|name| plain(name)) {
            return Some(
                self.root
                    .join("sessions")
                    .join(name)
                    .join(crate::store::EVENTS),
            );
        }
        let uuid = source
            .strip_prefix("archive/")
            .filter(|uuid| is_canonical_uuid(uuid))?;
        Some(
            self.root
                .join("archive")
                .join(uuid)
                .join(crate::store::EVENTS),
        )
    }
}

impl Journal for FileJournals {
    /// `live/<name>` by its meta file, `archive/<uuid>` by its claim marker
    /// and its directory, each by its own metadata; anything else, or a look
    /// that failed, is unknown.
    fn standing(&mut self, source: &str) -> Standing {
        let plain =
            |name: &&str| !name.is_empty() && *name != "." && *name != ".." && !name.contains('/');
        if let Some(name) = source.strip_prefix("live/").filter(plain) {
            let meta = self
                .root
                .join("sessions")
                .join(name)
                .join(crate::store::META);
            return match crate::store::source_presence(&meta) {
                crate::store::SourcePresence::Absent => Standing::Gone,
                crate::store::SourcePresence::Regular => Standing::Present,
                crate::store::SourcePresence::Irregular(_) => Standing::Unknown,
            };
        }
        if let Some(uuid) = source
            .strip_prefix("archive/")
            .filter(|uuid| is_canonical_uuid(uuid))
        {
            return match crate::archive::standing(&self.root, uuid) {
                Some((_, true)) => Standing::Claimed,
                Some((true, false)) => Standing::Present,
                Some((false, false)) => Standing::Gone,
                None => Standing::Unknown,
            };
        }
        Standing::Unknown
    }

    fn window(&mut self, source: &str, from: u64, max: u64) -> Result<Window, SourceGap> {
        let path = self.path(source).ok_or(SourceGap::Missing)?;
        let key = (source.to_owned(), from, max);
        if self.snap.mode == Mode::Held {
            // The window the warm read, while the journal's status is the
            // status of the handle it was read from; else stale, unread.
            let now = crate::store::followed_identity(&path).map(crate::store::Identity::status);
            if let Some(window) = self.snap.seen.get(&key).filter(|window| {
                now == Some((
                    window.dev,
                    window.ino,
                    window.len,
                    window.changed.mtime,
                    window.changed.ctime,
                ))
            }) {
                return Ok(window.clone());
            }
            self.snap.stale = true;
            return Err(SourceGap::Unreadable);
        }
        let read = crate::usage::journal_window(&path, from, max)?;
        #[cfg(test)]
        {
            observe::journal(read.bytes.len());
        }
        let window = Window {
            dev: read.dev,
            ino: read.ino,
            len: read.len,
            changed: Changed {
                mtime: read.mtime,
                ctime: read.ctime,
            },
            bytes: read.bytes,
        };
        if self.snap.mode == Mode::Warm {
            let width = u64::try_from(window.bytes.len()).unwrap_or(u64::MAX);
            self.snap.recorded = self.snap.recorded.saturating_add(width);
            self.snap.seen.insert(key, window.clone());
        }
        Ok(window)
    }
}

/// Journals in memory: test and fuzz stores. Every [`MemoryJournals::set`]
/// is a write: it moves the journal's change times, as a write to a file does.
#[derive(Debug, Clone, Default)]
pub struct MemoryJournals {
    journals: BTreeMap<String, (u64, u64, Vec<u8>, i64)>,
    writes: i64,
    standings: BTreeMap<String, Standing>,
}

impl MemoryJournals {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `source`'s journal is now `bytes`, in the file `(dev, ino)`.
    pub fn set(&mut self, source: &str, dev: u64, ino: u64, bytes: &[u8]) {
        self.writes = self.writes.saturating_add(1);
        self.journals
            .insert(source.to_owned(), (dev, ino, bytes.to_vec(), self.writes));
    }

    /// `source` now stands so; an unset source answers
    /// [`Standing::Unknown`].
    pub fn stand(&mut self, source: &str, standing: Standing) {
        self.standings.insert(source.to_owned(), standing);
    }
}

impl Journal for MemoryJournals {
    fn standing(&mut self, source: &str) -> Standing {
        self.standings
            .get(source)
            .copied()
            .unwrap_or(Standing::Unknown)
    }

    fn window(&mut self, source: &str, from: u64, max: u64) -> Result<Window, SourceGap> {
        let (dev, ino, bytes, write) = self.journals.get(source).ok_or(SourceGap::Missing)?;
        let len = bytes.len();
        let start = usize::try_from(from).unwrap_or(usize::MAX).min(len);
        let end = start
            .saturating_add(usize::try_from(max).unwrap_or(usize::MAX))
            .min(len);
        Ok(Window {
            dev: *dev,
            ino: *ino,
            len: u64::try_from(len).unwrap_or(u64::MAX),
            changed: Changed {
                mtime: (*write, 0),
                ctime: (*write, 0),
            },
            bytes: bytes.get(start..end).unwrap_or_default().to_vec(),
        })
    }
}

// --- sources and prepared reads ----------------------------------------------

/// One source as the maintainer lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceSpec {
    /// A live session: its label and raw meta.
    Live {
        id: String,
        label: String,
        meta: Vec<u8>,
    },
    /// A kept archive of session `uuid`.
    Archive { id: String, uuid: String },
    /// A source that gave no journal.
    Failed {
        id: String,
        label: String,
        session: Option<String>,
        gap: SourceGap,
    },
}

impl SourceSpec {
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Live { id, .. } | Self::Archive { id, .. } | Self::Failed { id, .. } => id,
        }
    }

    /// The group the source belongs to now.
    pub(super) fn key(&self) -> Key {
        self.origin().0
    }

    /// The group the source belongs to, and its origin when registered.
    fn origin(&self) -> (Key, Origin) {
        match self {
            Self::Live { label, meta, .. } => match stats::identity(meta) {
                Some((session, seats)) => (
                    session.clone(),
                    Origin::Live {
                        label: label.clone(),
                        session,
                        seats,
                    },
                ),
                None => (
                    None,
                    Origin::Unreadable {
                        label: label.clone(),
                    },
                ),
            },
            Self::Archive { uuid, .. } => (
                is_canonical_uuid(uuid).then(|| uuid.clone()),
                Origin::Archive { id: uuid.clone() },
            ),
            Self::Failed {
                label,
                session,
                gap,
                ..
            } => {
                let session = session.clone().filter(|id| is_canonical_uuid(id));
                (
                    session.clone(),
                    Origin::Failed {
                        label: label.clone(),
                        session,
                        gap: *gap,
                    },
                )
            }
        }
    }
}

/// How much one read takes: new journal bytes per step, and the longest line
/// counted (LF excluded).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub chunk: u64,
    pub line: usize,
}

/// The product's limits.
pub const LIMITS: Limits = Limits {
    chunk: 4 << 20,
    line: LINE_LIMIT,
};

/// Lifecycle inputs to one preparation, from the maintainer's round.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Life {
    /// The archive tree hash its walk-validate-walk just proved valid (A3);
    /// `None`: not judged, the persisted one is kept.
    pub fingerprint: Option<String>,
    /// Same-group source ids whose evidence a read from 0 may copy, tried in
    /// order.
    pub donors: Vec<String>,
}

/// What prepared work does beyond reading its journal.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Shift {
    /// A read, a registration or a reset (S2).
    Read,
    /// A counted source keeps its lines and cursor; its origin (when given)
    /// and its tree hash are replaced.
    Refresh(Option<Origin>),
    /// A read from 0 copies `donor`'s evidence up to the donor's cursor,
    /// while the donor still has `stamp`.
    Rebind { donor: String, stamp: (u64, u64) },
}

/// What a prepared read assumed of the payload.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Base {
    epoch: u64,
    /// The source's `(incarnation, cursor)`, or `None` when unregistered.
    stamp: Option<(u64, u64)>,
}

impl Base {
    /// The source's durable cursor: 0 when unregistered.
    fn cursor(&self) -> u64 {
        self.stamp.map_or(0, |(_, cursor)| cursor)
    }
}

/// A source registered, or replaced from its start.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Fresh {
    origin: Origin,
    reset: bool,
}

/// What a prepared read saw of its journal: checked again under the lock.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Witness {
    dev: u64,
    ino: u64,
    /// The journal's length.
    seen: u64,
    /// Where the read ended.
    read: u64,
    /// The hash of the at most [`WITNESS`] bytes ending at the new cursor.
    hash: String,
}

impl Witness {
    /// Whether `source`'s journal still holds what was read up to `end`.
    fn holds(&self, journal: &mut dyn Journal, source: &str, end: u64) -> bool {
        let back = end.min(WITNESS);
        journal
            .window(source, end.saturating_sub(back), back)
            .is_ok_and(|now| {
                (now.dev, now.ino) == (self.dev, self.ino)
                    && now.len >= end
                    && u64::try_from(now.bytes.len()) == Ok(back)
                    && doc::hash(&now.bytes) == self.hash
            })
    }
}

/// Work prepared outside the lock: one read of one source, judged, and what
/// it assumed. [`publish`] checks the assumptions again under the lock.
#[derive(Debug)]
pub struct Prepared {
    source: String,
    key: Key,
    base: Base,
    fresh: Option<Fresh>,
    start: u64,
    /// `None` for a source registered with no journal.
    digest: Option<Digest>,
    skipping: bool,
    witness: Option<Witness>,
    shift: Shift,
    /// The archive tree hash to keep with the source.
    fp: Option<String>,
}

/// A line longer than one read, held in memory between reads; never written.
#[derive(Debug)]
pub struct Held {
    source: String,
    base: Base,
    fresh: Option<Fresh>,
    cursor: u64,
    /// The witnessed bytes before the cursor.
    tail: Vec<u8>,
    /// The bytes from the cursor read so far: no LF among them.
    data: Vec<u8>,
    dev: u64,
    ino: u64,
}

impl Held {
    /// The bytes held: from the durable cursor to the read end.
    #[must_use]
    pub fn span(&self) -> (u64, u64) {
        let held = u64::try_from(self.data.len()).unwrap_or(u64::MAX);
        (self.cursor, self.cursor.saturating_add(held))
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// What a read of a source that found nothing new saw: the payload's hold on
/// the source, and its journal's identity, length and change times.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tail {
    base: Base,
    seen: Identity,
}

impl Tail {
    /// The durable cursor, and the journal's length past it.
    #[must_use]
    pub fn span(&self) -> (u64, u64) {
        (self.base.cursor(), self.seen.len)
    }
}

/// What one read of a source found, and what it cost.
#[derive(Debug)]
pub struct Observed {
    pub next: Next,
    /// New journal bytes read.
    pub read: u64,
    /// Whether any journal content was read, a witness included.
    pub touched: bool,
    /// The source's durable cursor in the payload.
    pub cursor: u64,
    /// What to tell the next read of a source that found nothing new.
    pub tail: Option<Tail>,
}

/// What preparing a source found.
#[derive(Debug)]
pub enum Next {
    Work(Prepared),
    /// A line that does not end within this read: read on from here.
    Holding(Held),
    Nothing,
}

/// One journal read in progress for one source.
struct Reader<'a> {
    journal: &'a mut dyn Journal,
    id: &'a str,
    key: &'a Key,
    base: &'a Base,
    limits: Limits,
    archive: bool,
    /// New journal bytes read so far.
    read: u64,
    /// Whether any journal content was read, a witness included.
    touched: bool,
    /// The journal as it was when the read found nothing new.
    quiet: Option<Identity>,
}

impl Reader<'_> {
    fn window(&mut self, from: u64, max: u64) -> Result<Window, SourceGap> {
        let window = self.journal.window(self.id, from, max)?;
        self.touched |= !window.bytes.is_empty();
        Ok(window)
    }

    fn count(&mut self, bytes: &[u8]) {
        let width = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        self.read = self.read.saturating_add(width);
    }

    /// Read from the start: a registration, or a reset replacing the source.
    fn fresh(&mut self, fresh: Fresh) -> Result<Next, Refused> {
        let window = match self.window(0, self.limits.chunk) {
            Ok(window) => window,
            Err(gap) if !fresh.reset => {
                let origin = match fresh.origin {
                    Origin::Live { label, session, .. } => Origin::Failed {
                        label,
                        session,
                        gap,
                    },
                    Origin::Archive { id } => Origin::Failed {
                        session: is_canonical_uuid(&id).then(|| id.clone()),
                        label: id,
                        gap,
                    },
                    Origin::Unreadable { label } => Origin::Failed {
                        label,
                        session: None,
                        gap,
                    },
                    failed @ Origin::Failed { .. } => failed,
                };
                return Ok(self.registered(origin));
            }
            Err(_) => return Ok(Next::Nothing),
        };
        self.count(&window.bytes);
        self.frame(
            Some(fresh),
            0,
            Vec::new(),
            window.bytes.clone(),
            &window,
            false,
        )
    }

    /// A read from 0: a donor's evidence copied when one holds, else the
    /// journal read.
    fn fresh_life(
        &mut self,
        fresh: Fresh,
        donors: &[String],
        view: Option<&SessionView>,
    ) -> Result<Next, Refused> {
        if let Some(next) = self.rebound(&fresh, donors, view) {
            return Ok(next);
        }
        self.fresh(fresh)
    }

    /// A source with no journal, registered as it is.
    fn registered(&self, origin: Origin) -> Next {
        Next::Work(Prepared {
            source: self.id.to_owned(),
            key: self.key.clone(),
            base: self.base.clone(),
            fresh: Some(Fresh {
                origin,
                reset: false,
            }),
            start: 0,
            digest: None,
            skipping: false,
            witness: None,
            shift: Shift::Read,
            fp: None,
        })
    }

    /// A source registered again as it now is: its evidence replaced.
    /// A failed source: registered failed when it has no row, nothing when
    /// its failed row already names this gap and origin, else registered
    /// failed again in place of what its row holds.
    fn failed(&self, row: Option<&doc::SourceRow>, gap: SourceGap, origin: Origin) -> Next {
        let org = doc::origin_hash(&origin);
        match row {
            None => self.registered(origin),
            Some(row)
                if row.failed
                    && row.gap == Some(gap)
                    && row.org.as_deref() == Some(org.as_str()) =>
            {
                Next::Nothing
            }
            Some(_) => self.replaced(origin),
        }
    }

    fn replaced(&self, origin: Origin) -> Next {
        let mut next = self.registered(origin);
        if let Next::Work(work) = &mut next
            && let Some(fresh) = &mut work.fresh
        {
            fresh.reset = true;
        }
        next
    }

    /// A read from 0 that copies the first donor whose consumed bytes this
    /// journal holds: its length reaches the donor's cursor and the bytes
    /// before it hash as the donor's witness. `None` when no donor holds.
    fn rebound(
        &mut self,
        fresh: &Fresh,
        donors: &[String],
        view: Option<&SessionView>,
    ) -> Option<Next> {
        let view = view?;
        let own = self.id.to_owned();
        for donor in donors.iter().filter(|donor| **donor != own) {
            let Some(row) = view
                .rows
                .get(donor)
                .filter(|row| !row.failed && row.cursor > 0)
            else {
                continue;
            };
            let cursor = row.cursor;
            let back = cursor.min(WITNESS);
            let Ok(tail) = self.window(cursor.saturating_sub(back), back) else {
                continue;
            };
            let holds = tail.len >= cursor
                && u64::try_from(tail.bytes.len()) == Ok(back)
                && doc::hash(&tail.bytes) == row.provenance.hash;
            if !holds {
                continue;
            }
            return Some(Next::Work(Prepared {
                source: self.id.to_owned(),
                key: self.key.clone(),
                base: self.base.clone(),
                fresh: Some(fresh.clone()),
                start: 0,
                digest: None,
                skipping: row.provenance.skipping,
                witness: Some(Witness {
                    dev: tail.dev,
                    ino: tail.ino,
                    seen: tail.len,
                    read: cursor,
                    hash: row.provenance.hash.clone(),
                }),
                shift: Shift::Rebind {
                    donor: donor.clone(),
                    stamp: (row.provenance.incarnation, cursor),
                },
                fp: None,
            }));
        }
        None
    }

    /// Read on from a registered source's cursor; `None` when the journal is
    /// no longer the one read, which resets it.
    fn continued(&mut self, row: &doc::SourceRow) -> Result<Option<Next>, Refused> {
        let cursor = row.cursor;
        let mark = &row.provenance;
        let back = cursor.min(WITNESS);
        let Ok(tail) = self.window(cursor.saturating_sub(back), back) else {
            return Ok(Some(Next::Nothing));
        };
        let same = (tail.dev, tail.ino) == (mark.dev, mark.ino)
            && tail.len >= cursor
            && u64::try_from(tail.bytes.len()) == Ok(back)
            && doc::hash(&tail.bytes) == mark.hash;
        if !same {
            return Ok(None);
        }
        if tail.len == mark.seen && mark.read == mark.seen {
            self.quiet = Some(tail.identity());
            return Ok(Some(Next::Nothing));
        }
        let Ok(window) = self.window(cursor, self.limits.chunk) else {
            return Ok(Some(Next::Nothing));
        };
        if (window.dev, window.ino) != (tail.dev, tail.ino) || window.len < cursor {
            return Ok(None);
        }
        self.count(&window.bytes);
        let data = window.bytes.clone();
        self.frame(None, cursor, tail.bytes, data, &window, mark.skipping)
            .map(Some)
    }

    /// Read on past a held line; `None` when the journal changed under it.
    fn resume(&mut self, held: Held) -> Result<Option<Next>, Refused> {
        let (cursor, from) = held.span();
        let Ok(window) = self.window(from, self.limits.chunk) else {
            return Ok(None);
        };
        if (window.dev, window.ino) != (held.dev, held.ino) || window.len < from {
            return Ok(None);
        }
        self.count(&window.bytes);
        let mut data = held.data;
        data.extend_from_slice(&window.bytes);
        self.frame(held.fresh, cursor, held.tail, data, &window, false)
            .map(Some)
    }

    /// Frame `data`, read from `cursor` with `tail` the bytes just before it.
    fn frame(
        &mut self,
        fresh: Option<Fresh>,
        cursor: u64,
        tail: Vec<u8>,
        data: Vec<u8>,
        window: &Window,
        skipping: bool,
    ) -> Result<Next, Refused> {
        let width = u64::try_from(data.len()).unwrap_or(u64::MAX);
        let read_end = cursor.saturating_add(width);
        let eof = read_end >= window.len;
        let final_line = self.archive && eof;
        let (id, key) = (self.id, self.key);
        let session = key.as_deref();
        let line = self.limits.line;
        let judged = |at: u64, bytes: &[u8]| {
            Digest::of(session, at, bytes, final_line, line)
                .map_err(|why| Refused::Io(format!("{id}: {why:?}")))
        };
        let (digest, still) = if skipping {
            match data.iter().position(|byte| *byte == b'\n') {
                Some(lf) => {
                    let past = cursor.saturating_add(u64::try_from(lf + 1).unwrap_or(u64::MAX));
                    let rest = data.get(lf + 1..).unwrap_or_default();
                    (judged(past, rest)?.widened(cursor), false)
                }
                None if data.is_empty() => {
                    self.quiet = Some(window.identity());
                    return Ok(Next::Nothing);
                }
                None => (Digest::passed(cursor, read_end), true),
            }
        } else if data.contains(&b'\n') || (final_line && !data.is_empty()) {
            (judged(cursor, &data)?, false)
        } else if eof {
            if fresh.is_none() {
                self.quiet = Some(window.identity());
                return Ok(Next::Nothing);
            }
            (Digest::passed(cursor, cursor), false)
        } else if data.len() > line {
            (Digest::over_long(cursor, read_end), true)
        } else {
            return Ok(Next::Holding(Held {
                source: self.id.to_owned(),
                base: self.base.clone(),
                fresh,
                cursor,
                tail,
                data,
                dev: window.dev,
                ino: window.ino,
            }));
        };
        let end = digest.end();
        let base = cursor.saturating_sub(u64::try_from(tail.len()).unwrap_or(u64::MAX));
        let from = end.saturating_sub(WITNESS).max(base);
        let mut seen = tail;
        seen.extend_from_slice(&data);
        let witnessed = usize::try_from(from - base)
            .ok()
            .zip(usize::try_from(end - base).ok())
            .and_then(|(lo, hi)| seen.get(lo..hi))
            .ok_or_else(|| Refused::Io(format!("{}: witness out of the read", self.id)))?;
        Ok(Next::Work(Prepared {
            source: self.id.to_owned(),
            key: self.key.clone(),
            base: self.base.clone(),
            fresh,
            start: cursor,
            digest: Some(digest),
            skipping: still,
            witness: Some(Witness {
                dev: window.dev,
                ino: window.ino,
                seen: window.len,
                read: read_end,
                hash: doc::hash(witnessed),
            }),
            shift: Shift::Read,
            fp: None,
        }))
    }
}

impl Prepared {
    /// Outside the lock: the next read of `source`, from its row in `dir` and
    /// its journal. `held` is the line a previous call held for it.
    ///
    /// # Errors
    ///
    /// A payload file over its cap or damaged.
    pub fn read(
        dir: &mut dyn Dir,
        journal: &mut dyn Journal,
        source: &SourceSpec,
        limits: Limits,
        held: Option<Held>,
    ) -> Result<Next, Refused> {
        Self::read_counted(dir, journal, source, limits, held).map(|(next, _)| next)
    }

    /// [`Prepared::read`], with the new journal bytes it read.
    ///
    /// # Errors
    ///
    /// As [`Prepared::read`].
    pub fn read_counted(
        dir: &mut dyn Dir,
        journal: &mut dyn Journal,
        source: &SourceSpec,
        limits: Limits,
        held: Option<Held>,
    ) -> Result<(Next, u64), Refused> {
        Self::read_observed(dir, journal, source, limits, held, None)
            .map(|seen| (seen.next, seen.read))
    }

    /// [`Prepared::read_counted`], told the [`Tail`] the last read of this
    /// source that found nothing new saw: when the payload still holds the
    /// source as it did and its journal has the same identity, length and
    /// change times (amendment 16), the journal's status is all that is looked
    /// at, no byte of it read. A read that finds nothing new returns the tail
    /// to tell the next one, and says whether it read journal content.
    ///
    /// # Errors
    ///
    /// As [`Prepared::read`].
    pub fn read_observed(
        dir: &mut dyn Dir,
        journal: &mut dyn Journal,
        source: &SourceSpec,
        limits: Limits,
        held: Option<Held>,
        quiet: Option<&Tail>,
    ) -> Result<Observed, Refused> {
        Self::read_life(dir, journal, source, limits, held, quiet, &Life::default())
    }

    /// [`Prepared::read_observed`] with the round's lifecycle inputs. Before
    /// any journal read: a failed source whose row is counted or names another
    /// gap or origin is registered failed again (its evidence replaced); a
    /// counted source whose origin or proven tree hash moved is refreshed, its
    /// lines and cursor kept. A read from 0 first tries `life.donors`.
    ///
    /// # Errors
    ///
    /// As [`Prepared::read`].
    pub fn read_life(
        dir: &mut dyn Dir,
        journal: &mut dyn Journal,
        source: &SourceSpec,
        limits: Limits,
        held: Option<Held>,
        quiet: Option<&Tail>,
        life: &Life,
    ) -> Result<Observed, Refused> {
        let epoch = read_epoch(dir)?.unwrap_or(0);
        let (key, origin) = source.origin();
        let view = load_view(dir, epoch, &key)?;
        let row = view
            .as_ref()
            .and_then(|view| view.rows.get(source.id()).cloned());
        let base = Base {
            epoch,
            stamp: row
                .as_ref()
                .map(|row| (row.provenance.incarnation, row.cursor)),
        };
        let mut reader = Reader {
            journal,
            id: source.id(),
            key: &key,
            base: &base,
            limits,
            archive: matches!(source, SourceSpec::Archive { .. }),
            read: 0,
            touched: false,
            quiet: None,
        };
        let cursor = base.cursor();
        let seen = |next: Next, reader: &Reader<'_>, tail: Option<Tail>| Observed {
            next,
            read: reader.read,
            touched: reader.touched,
            cursor,
            tail,
        };
        if let SourceSpec::Failed { gap, .. } = source {
            let next = reader.failed(row.as_ref(), *gap, origin);
            return Ok(seen(next, &reader, None));
        }
        let fingerprint = life
            .fingerprint
            .clone()
            .filter(|_| matches!(source, SourceSpec::Archive { .. }));
        if held.is_none()
            && let Some(work) = Self::refreshed(
                source,
                &key,
                &base,
                row.as_ref(),
                &origin,
                fingerprint.as_ref(),
            )
        {
            return Ok(seen(Next::Work(work), &reader, None));
        }
        let stamped = |mut next: Next| {
            if let Next::Work(work) = &mut next
                && work.fp.is_none()
            {
                work.fp.clone_from(&fingerprint);
            }
            next
        };
        if held.is_none()
            && let Some(quiet) = quiet.filter(|quiet| quiet.base == base)
            && reader
                .window(quiet.seen.len, 0)
                .is_ok_and(|now| now.identity() == quiet.seen)
        {
            return Ok(seen(Next::Nothing, &reader, Some(quiet.clone())));
        }
        if let Some(held) = held.filter(|held| held.source == source.id() && held.base == base)
            && let Some(next) = reader.resume(held)?
        {
            return Ok(seen(stamped(next), &reader, None));
        }
        let fresh = |reset: bool| Fresh {
            origin: origin.clone(),
            reset,
        };
        let next = match &row {
            Some(row) if !row.failed => match reader.continued(row)? {
                Some(next) => next,
                None => reader.fresh_life(fresh(true), &life.donors, view.as_ref())?,
            },
            Some(_) => reader.fresh_life(fresh(true), &life.donors, view.as_ref())?,
            None => reader.fresh_life(fresh(false), &life.donors, view.as_ref())?,
        };
        let next = stamped(next);
        let tail = match (&next, reader.quiet) {
            (Next::Nothing, Some(quiet)) => Some(Tail {
                base: base.clone(),
                seen: quiet,
            }),
            _ => None,
        };
        Ok(seen(next, &reader, tail))
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// A counted source whose origin, or whose proven archive tree hash,
    /// moved: the work that stores the new one and keeps its lines and
    /// cursor, with no journal read.
    fn refreshed(
        source: &SourceSpec,
        key: &Key,
        base: &Base,
        row: Option<&doc::SourceRow>,
        origin: &Origin,
        fingerprint: Option<&String>,
    ) -> Option<Self> {
        let row = row.filter(|row| !row.failed)?;
        let org = doc::origin_hash(origin);
        let moved = (row.org.as_deref() != Some(org.as_str())).then(|| origin.clone());
        let fp = fingerprint
            .filter(|fp| row.provenance.fp.as_ref() != Some(*fp))
            .cloned();
        (moved.is_some() || fp.is_some()).then(|| Self {
            source: source.id().to_owned(),
            key: key.clone(),
            base: base.clone(),
            fresh: None,
            start: row.cursor,
            digest: None,
            skipping: row.provenance.skipping,
            witness: None,
            shift: Shift::Refresh(moved),
            fp,
        })
    }

    /// The donor whose evidence this work copies.
    #[must_use]
    pub fn donor(&self) -> Option<&str> {
        match &self.shift {
            Shift::Rebind { donor, .. } => Some(donor),
            Shift::Read | Shift::Refresh(_) => None,
        }
    }

    /// The durable cursor before and after this work applies.
    #[must_use]
    pub fn span(&self) -> (u64, u64) {
        if let Shift::Rebind { .. } = self.shift {
            return (0, self.witness.as_ref().map_or(0, |seen| seen.read));
        }
        let end = self.digest.as_ref().map_or(self.start, Digest::end);
        (self.start, end)
    }

    /// After this work published at `generation`: what to tell the next read
    /// of the source so that it looks at the journal's status alone, when the
    /// work consumed the journal to its end and the journal is still the one
    /// read, with no byte more. `prior` is what the read before this work was
    /// told; a refresh, which reads nothing, keeps its journal identity.
    pub(super) fn settled(
        &self,
        generation: u64,
        journal: &mut dyn Journal,
        prior: Option<&Tail>,
    ) -> Option<Tail> {
        let renewed = self.fresh.is_some() || !matches!(self.shift, Shift::Read);
        let incarnation = if renewed {
            generation
        } else {
            self.base.stamp?.0
        };
        let end = self.span().1;
        let base = Base {
            epoch: self.base.epoch,
            stamp: Some((incarnation, end)),
        };
        let Some(witness) = &self.witness else {
            return match self.shift {
                Shift::Refresh(_) => {
                    prior
                        .filter(|prior| prior.base == self.base)
                        .map(|prior| Tail {
                            base,
                            seen: prior.seen,
                        })
                }
                Shift::Read | Shift::Rebind { .. } => None,
            };
        };
        if end != witness.seen || self.skipping {
            return None;
        }
        let now = journal.window(&self.source, end, 0).ok()?;
        ((now.dev, now.ino, now.len) == (witness.dev, witness.ino, end)).then(|| Tail {
            base,
            seen: now.identity(),
        })
    }

    /// Whether this work replaces the source from its start.
    #[must_use]
    pub fn is_reset(&self) -> bool {
        self.fresh.as_ref().is_some_and(|fresh| fresh.reset)
    }

    /// Whether the journal held more than this work reads.
    #[must_use]
    pub fn more(&self) -> bool {
        self.witness
            .as_ref()
            .is_some_and(|seen| self.span().1 < seen.seen)
    }
}

// --- decisions ----------------------------------------------------------------

/// Why prepared work was dropped, nothing written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// The payload was repaired since.
    Epoch,
    /// The source's stamp moved since: another publication won.
    Stamp,
    /// The journal no longer holds what was read.
    Witness,
    /// Another group's row of the root's ownership index lists the source.
    Owned,
    /// Under the lock the source is gone, or an archive that would count is
    /// claimed.
    Gone,
    /// Event time: the source's row is a failed registration.
    Failed,
    /// Event time: the row's journal device and inode differ from what the
    /// append saw, or the journal is now shorter than the row's cursor.
    Journal,
    /// Event time: the source's meta could not be read, or is not text
    /// (`stats::identity` gives nothing): the birth would not be counted. A
    /// text meta without a session id is not this: it registers in the
    /// id-less group.
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Publish(Plan),
    Discard(Why),
}

/// Why nothing was decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// A payload file over its cap: never parsed.
    OverCap {
        file: String,
        cap: u64,
    },
    Damaged {
        file: String,
        reason: String,
    },
    Io(String),
    UnknownSource(String),
    /// A crash left work [`recovery`] must finish first.
    RecoveryOwed,
    /// An event line that is not one certificate or linked `spawn`.
    NotAnEvent,
    /// A payload no surviving evidence proves the epoch of: a repair would
    /// have to guess one.
    EpochUnproven,
    /// One warm recorded more than the snapshot cap: refused before the lock,
    /// nothing taken, nothing written.
    Snapshot {
        recorded: u64,
        cap: u64,
    },
    /// Every held attempt found a dependency changed since its warm: nothing
    /// written.
    Contended,
}

impl Refused {
    /// One line for a human.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::OverCap { file, cap } => format!("{file} is over its cap of {cap} bytes"),
            Self::Damaged { file, reason } => format!("{file} is damaged: {reason}"),
            Self::Io(why) => why.clone(),
            Self::UnknownSource(id) => format!("no source {id}"),
            Self::RecoveryOwed => "an interrupted update is not recovered".to_owned(),
            Self::NotAnEvent => "not a certificate or linked spawn line".to_owned(),
            Self::EpochUnproven => "nothing left proves the store's epoch".to_owned(),
            Self::Snapshot { recorded, cap } => {
                format!("a store snapshot of {recorded} bytes is over its cap of {cap} bytes")
            }
            Self::Contended => "the statistics store was busy".to_owned(),
        }
    }

    /// The file and the reason of a payload a writer must repair.
    #[must_use]
    pub fn damage(&self) -> Option<(String, String)> {
        match self {
            Self::OverCap { file, cap } => {
                Some((file.clone(), format!("over its cap of {cap} bytes")))
            }
            Self::Damaged { file, reason } => Some((file.clone(), reason.clone())),
            _ => None,
        }
    }
}

/// What the store shows, read without the lock.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    clippy::large_enum_variant,
    reason = "read once per refresh and matched at once; boxing the view buys nothing"
)]
pub enum Shown {
    /// No materialized answer yet.
    Building,
    Ready {
        view: View,
        epoch: u64,
        r#gen: u64,
        /// A publication is in flight or a crash left one.
        updating: bool,
        /// Sources whose reading has not reached what was seen of the journal.
        pending: u64,
        listed: u64,
    },
    Refused(Refused),
}

// --- the tab's read and reset (S5) ---------------------------------------------

/// How long a reset waits for `stats.lock`.
pub const RESET_WAIT: Duration = Duration::from_secs(1);

/// One scope's sources: those the store has registered, and of them those
/// whose reading has not reached what was seen of the journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Progress {
    pub listed: u64,
    pub pending: u64,
}

/// What the store holds for the session the tab was opened on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThisProgress {
    /// No canonical session id was asked for.
    NoUuid,
    /// The root has no row for the session yet.
    Unregistered,
    /// The session's view could not be read in agreement with the root.
    Damaged(Refused),
    Ready(Progress),
}

/// What the Statistics tab shows, read without the lock: [`Shown`] with
/// each scope's sources, and a session's damage kept to its own scope.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    clippy::large_enum_variant,
    reason = "read once per refresh and matched at once; boxing the view buys nothing"
)]
pub enum Glance {
    /// No published answer to show.
    Building,
    Ready {
        view: View,
        epoch: u64,
        r#gen: u64,
        /// A publication is in flight or a crash left one.
        updating: bool,
        all: Progress,
        this: ThisProgress,
    },
    Refused(Refused),
}

/// What a reset did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reset {
    /// The payload replaced by an empty one in this epoch.
    Rebuilt { epoch: u64 },
    /// Nothing to replace.
    Empty,
    /// The lock was held elsewhere past the wait: nothing touched.
    Busy,
}

/// The tab's answer, read without the lock from the epoch, the root view,
/// the intent and the session's view alone: never a journal, a meta or an
/// archive. A damaged or missing root refuses the answer; a session's own
/// trouble stays in [`ThisProgress`], the root's counts still shown.
#[must_use]
pub fn glance(dir: &mut dyn Dir, this: Option<&str>) -> Glance {
    glance_of(dir, this).unwrap_or_else(Glance::Refused)
}

/// What the store holds for one canonical session, read beside a root.
enum Session {
    Unregistered,
    Damaged(Refused),
    Ready(Box<stats::Scoped>, Progress),
}

/// [`shown_of`]'s reads, each scope kept apart: a session view of another
/// generation than its row is read again with the root, at most [`READS`]
/// times, and still differing it is that session's damage alone.
fn glance_of(dir: &mut dyn Dir, this: Option<&str>) -> Result<Glance, Refused> {
    let key: Option<Key> = this
        .filter(|id| is_canonical_uuid(id))
        .map(|id| Some(id.to_owned()));
    let mut first = None;
    let mut moved = false;
    let mut differing = None;
    for _ in 0..READS {
        let Some(epoch) = read_epoch(dir)? else {
            return Ok(Glance::Building);
        };
        let Some(root) = load_root(dir, epoch)? else {
            return Ok(Glance::Building);
        };
        moved |= *first.get_or_insert(root.generation) != root.generation;
        let updating = !matches!(dir.read(INTENT, INTENT_CAP), Read::Absent);
        let session = match key.as_ref().map(|key| session_of(dir, epoch, &root, key)) {
            None => None,
            Some(Ok(session)) => Some(session),
            Some(Err(name)) => {
                differing = Some((epoch, root, updating, name));
                continue;
            }
        };
        return Ok(ready(epoch, &root, updating, session));
    }
    let Some((epoch, root, updating, name)) = differing else {
        return Ok(Glance::Building);
    };
    let damaged = Session::Damaged(wrong(&name, "generation differs from the root"));
    Ok(ready(epoch, &root, updating || moved, Some(damaged)))
}

/// Session `key`'s counts beside `root`, the id-less group's included; the
/// view's name when its generation is not the one the root's row records (a
/// publication between the two reads, or damage).
fn session_of(dir: &mut dyn Dir, epoch: u64, root: &Root, key: &Key) -> Result<Session, String> {
    let Some(row) = root.rows.get(key) else {
        return Ok(Session::Unregistered);
    };
    let name = view_name(key);
    match load_view(dir, epoch, key) {
        Ok(Some(view)) if view.generation == row.generation => {
            let mut scope = root.covering.clone();
            scope.add(&view.tally);
            let progress = Progress {
                listed: row.listed,
                pending: row.pending,
            };
            Ok(Session::Ready(Box::new(scope.scoped()), progress))
        }
        Ok(Some(_)) => Err(name),
        Ok(None) => Ok(Session::Damaged(wrong(&name, "missing"))),
        Err(refused) => Ok(Session::Damaged(refused)),
    }
}

/// The answer of `root` read at `epoch`, with `session` when one was asked.
fn ready(epoch: u64, root: &Root, updating: bool, session: Option<Session>) -> Glance {
    let (this, progress) = match session {
        None => (ThisSession::NoUuid, ThisProgress::NoUuid),
        Some(Session::Ready(scoped, progress)) => {
            (ThisSession::Ready(scoped), ThisProgress::Ready(progress))
        }
        Some(Session::Unregistered) => (ThisSession::NoUuid, ThisProgress::Unregistered),
        Some(Session::Damaged(refused)) => (ThisSession::NoUuid, ThisProgress::Damaged(refused)),
    };
    Glance::Ready {
        view: View {
            all: root.all.scoped(),
            this,
        },
        epoch,
        r#gen: root.generation,
        updating,
        all: Progress {
            listed: root.listed(),
            pending: root.pending(),
        },
        this: progress,
    }
}

/// The `r` of the Statistics tab: under `stats.lock` taken within `wait`,
/// recovery applied whole, then the payload replaced by an empty one in the
/// next epoch ([`repair`]'s swap, reading under the lock as a repair may),
/// so the upkeep rebuilds it from the journals. The lock and the journals
/// are never touched. A lock held past `wait`, or every attempt stale, is
/// [`Reset::Busy`] with nothing written.
///
/// # Errors
///
/// An IO error, recovery or the repair refused, or a warm over its cap.
pub fn reset(root: &Path, wait: Duration) -> Result<Reset, Refused> {
    let mut files = Files::at(root);
    let applied = |files: &mut Files, plan: &Plan| {
        plan.apply(files, plan.ops.len())
            .map_err(|why| Refused::Io(why.to_string()))
    };
    let mut held = Duration::ZERO;
    let taken = transact(
        root,
        &mut files,
        &mut FileJournals::at(root),
        wait,
        &mut held,
        |files, _| {
            // The F1 exception: a reset is a repair, which reads under the lock.
            files.exception(|files| {
                match recovery(files) {
                    Ok(owed) => applied(files, &owed)?,
                    // Damage recovery cannot get past: the repair takes it over.
                    Err(refused) if refused.damage().is_some() => {}
                    Err(refused) => return Err(refused),
                }
                let plan = repair(files)?;
                if plan.ops.is_empty() {
                    return Ok(Reset::Empty);
                }
                applied(files, &plan)?;
                let epoch = read_epoch(files)?.ok_or_else(|| wrong(EPOCH, "missing"))?;
                Ok(Reset::Rebuilt { epoch })
            })
        },
    );
    match taken {
        Taken::Done(outcome) => outcome,
        Taken::Unlocked(why) if why.kind() == io::ErrorKind::WouldBlock => Ok(Reset::Busy),
        Taken::Contended => Ok(Reset::Busy),
        Taken::Unlocked(why) => Err(Refused::Io(format!("{LOCK}: {why}"))),
        Taken::Over { recorded, cap } => Err(Refused::Snapshot { recorded, cap }),
    }
}

/// How one write to the tools subtree `<root>/stats/tools/` ended (#288 P3).
#[derive(Debug)]
pub enum Tools<T> {
    /// Decided under `stats.lock` in the active `epoch`: the plan the
    /// builder made, if any, was applied whole.
    Done { epoch: u64, value: T },
    /// No active epoch: S5 has not built the payload, or a swap is under way.
    /// Tools never makes the payload or its epoch.
    NoEpoch,
    /// The lock was held past the wait, or every attempt was stale: nothing
    /// was written.
    Busy,
    /// An IO error, a damaged epoch, or a warm over its cap.
    Failed(String),
}

/// The one writer entry of the tools subtree (#288 P3, owner condition S5):
/// `build` reads through the payload [`Dir`] and returns the plan to apply
/// with its result; it runs warm without the lock, then held under
/// `stats.lock` taken within `wait`, as every writer here does (ruling 19).
/// The active epoch is read under the lock first and handed to `build`, so a
/// plan never lands in a payload it did not see. `tools::store` builds the
/// plans; this module stays the one executor of writes under `<root>/stats/`.
pub fn tools<T>(
    root: &Path,
    wait: Duration,
    mut build: impl FnMut(&mut dyn Dir, u64) -> (Option<Plan>, T),
) -> Tools<T> {
    let mut files = Files::at(root);
    let mut held = Duration::ZERO;
    let taken = transact(
        root,
        &mut files,
        &mut FileJournals::at(root),
        wait,
        &mut held,
        |files, _| {
            let epoch = match read_epoch(files) {
                Ok(Some(epoch)) => epoch,
                Ok(None) => return Ok(None),
                Err(refused) => return Err(refused.describe()),
            };
            let (plan, value) = build(files, epoch);
            if let Some(plan) = plan {
                plan.apply(files, plan.ops.len())
                    .map_err(|why| why.to_string())?;
            }
            Ok(Some((epoch, value)))
        },
    );
    match taken {
        Taken::Done(Ok(Some((epoch, value)))) => Tools::Done { epoch, value },
        Taken::Done(Ok(None)) => Tools::NoEpoch,
        Taken::Done(Err(why)) => Tools::Failed(why),
        Taken::Unlocked(why) if why.kind() == io::ErrorKind::WouldBlock => Tools::Busy,
        Taken::Contended => Tools::Busy,
        Taken::Unlocked(why) => Tools::Failed(format!("{LOCK}: {why}")),
        Taken::Over { recorded, cap } => {
            Tools::Failed(format!("warm recorded {recorded} bytes over the cap {cap}"))
        }
    }
}

/// The active epoch as a reader sees it, without the lock: `None` before
/// S5's first build. The tools reader compares it each round, so a reset or
/// a repair that moved the payload aside drops what it held.
///
/// # Errors
///
/// A damaged or unreadable epoch, described.
pub fn active_epoch(dir: &mut dyn Dir) -> Result<Option<u64>, String> {
    read_epoch(dir).map_err(|refused| refused.describe())
}

/// Every [`Files`] read on this thread since the last call: the payload
/// name and the content bytes it returned (0 for any answer but bytes).
#[cfg(test)]
#[must_use]
pub fn take_payload_names() -> Vec<(String, u64)> {
    observe::NAMES.with(std::cell::RefCell::take)
}

// --- event time (S4) ------------------------------------------------------------

/// The journal as a spawn's append left it: the appended handle's status,
/// taken under the journal lock after the write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seen {
    pub dev: u64,
    pub ino: u64,
    pub len: u64,
}

/// What one event-time count did. `Published` and `Kept` are the successes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Counted {
    /// The record, and for an absent source its registration, published.
    Published { r#gen: u64 },
    /// Nothing to write: kept already, or the ordered read of this same
    /// journal passed it.
    Kept,
    /// `stats.lock` not taken before the call's one deadline.
    Busy,
    /// Every held attempt was stale.
    Contended,
    /// A state only the maintainer resolves.
    Skipped(Why),
    /// Damage, an IO error, over the snapshot cap, or not an event line.
    Refused(Refused),
}

impl Counted {
    /// Whether the birth is in the store now.
    #[must_use]
    pub const fn counted(&self) -> bool {
        matches!(self, Self::Published { .. } | Self::Kept)
    }

    /// Why the birth is in the journal only, for one line on stderr; `None`
    /// for a success.
    #[must_use]
    pub fn reason(&self) -> Option<String> {
        match self {
            Self::Published { .. } | Self::Kept => None,
            Self::Busy => Some("the statistics store is locked".to_owned()),
            Self::Contended => Some(Refused::Contended.describe()),
            Self::Skipped(Why::Owned) => {
                Some("another session's statistics own this source".to_owned())
            }
            Self::Skipped(Why::Failed) => {
                Some("the statistics hold this source as unreadable".to_owned())
            }
            Self::Skipped(Why::Journal) => {
                Some("the journal changed since the statistics read it".to_owned())
            }
            Self::Skipped(Why::Unreadable) => Some("the session meta is unreadable".to_owned()),
            Self::Skipped(why) => Some(format!("{why:?}").to_lowercase()),
            Self::Refused(refused) => Some(refused.describe()),
        }
    }
}

/// The `stats.lock` poll budget of one counted call: one deadline across
/// every attempt.
pub const EVENT_WAIT: Duration = Duration::from_millis(200);
/// Held attempts per taker call.
pub const ATTEMPTS: u32 = 3;
/// Bytes one warm may record before it is refused.
pub const SNAPSHOT_CAP: u64 = 256 << 20;

/// Under the stats lock: the event-time rules for `spec`'s certificate or
/// linked `spawn` `line` at journal `offset`, the append having seen `seen`.
///
/// # Errors
///
/// Owed recovery, a line that is no such record, or a payload file over its
/// cap or damaged.
pub fn event_counted(
    dir: &mut dyn Dir,
    spec: &SourceSpec,
    offset: u64,
    line: &[u8],
    seen: Seen,
) -> Result<Decision, Refused> {
    owed(dir)?;
    let SourceSpec::Live { id: source, .. } = spec else {
        return Err(Refused::UnknownSource(spec.id().to_owned()));
    };
    let (key, origin) = spec.origin();
    // Not a certificate or linked `spawn` of this source: nothing, first.
    let kept = Evidence::new(origin.clone())
        .take_event(offset, line, LINE_LIMIT)
        .map_err(|_| Refused::NotAnEvent)?;
    if matches!(origin, Origin::Unreadable { .. }) {
        return Ok(Decision::Discard(Why::Unreadable));
    }
    let epoch = read_epoch(dir)?.unwrap_or(0);
    let mut root = load_root(dir, epoch)?;
    // The ownership proof below reads the index: made first when an earlier
    // format did not write it.
    if let Some(generation) = root
        .as_ref()
        .filter(|root| !root.indexed())
        .map(|root| root.generation)
    {
        root = Some(resummed(dir, epoch, generation)?);
    }
    let generation = root
        .as_ref()
        .map_or(0, |root| root.generation)
        .checked_add(1)
        .ok_or_else(|| Refused::Io("generation".to_owned()))?;
    let mut primary = load_group(dir, epoch, &key)?.unwrap_or_else(|| Group::new(key.clone()));
    let mut touched = BTreeSet::new();
    match primary.own.get_mut(source) {
        None => {
            if root
                .as_ref()
                .is_some_and(|root| root.owner(source).is_some())
            {
                return Ok(Decision::Discard(Why::Owned));
            }
            // Registered pending: the maintainer reads it from 0 and the
            // empty witness holds on this same journal.
            let Some(kept) = kept else {
                return Ok(Decision::Publish(Plan::of(Vec::new())));
            };
            let mut own = Own {
                evidence: Evidence::new(origin),
                provenance: Provenance {
                    seen: seen.len,
                    dev: seen.dev,
                    ino: seen.ino,
                    ..Provenance::none(generation)
                },
            };
            own.evidence.keep(offset, kept);
            touched.extend(own.evidence.groups());
            primary.own.insert(source.clone(), own);
        }
        Some(own) => {
            match &own.evidence.origin {
                Origin::Failed { .. } => return Ok(Decision::Discard(Why::Failed)),
                Origin::Unreadable { .. } => return Ok(Decision::Discard(Why::Unreadable)),
                Origin::Live { .. } | Origin::Archive { .. } => {}
            }
            if (own.provenance.dev, own.provenance.ino) != (seen.dev, seen.ino)
                || seen.len < own.evidence.cursor
            {
                return Ok(Decision::Discard(Why::Journal));
            }
            // Below the cursor the ordered read of this same journal passed
            // it; an equal record is kept already.
            let Some(kept) = own
                .evidence
                .take_event(offset, line, LINE_LIMIT)
                .map_err(|_| Refused::NotAnEvent)?
            else {
                return Ok(Decision::Publish(Plan::of(Vec::new())));
            };
            touched.extend(own.evidence.groups());
            own.evidence.keep(offset, kept);
            touched.extend(own.evidence.groups());
        }
    }
    touched.insert(key);
    commit(dir, epoch, root, primary, touched, generation).map(Decision::Publish)
}

/// The event taker: `event_counted` for `spec` under `stats.lock`, warm then
/// held, at most [`ATTEMPTS`] times within one poll deadline of `wait`; no
/// warm starts once that deadline has passed ([`Counted::Busy`]).
/// Never reads a journal; never repairs (the maintainer does).
#[must_use]
pub fn counted(
    root: &Path,
    spec: &SourceSpec,
    offset: u64,
    line: &[u8],
    seen: Seen,
    wait: Duration,
) -> Counted {
    let mut held = Duration::ZERO;
    let taken = attempted(
        root,
        &mut Files::at(root),
        &mut FileJournals::at(root),
        wait,
        &mut held,
        Rewarm::WithinDeadline,
        |files, _| {
            let decided = recovery(files)
                .and_then(|plan| {
                    plan.apply(files, plan.ops.len())
                        .map_err(|why| Refused::Io(why.to_string()))
                })
                .and_then(|()| event_counted(files, spec, offset, line, seen))?;
            match decided {
                Decision::Publish(plan) => {
                    plan.apply(files, plan.ops.len())
                        .map_err(|why| Refused::Io(why.to_string()))?;
                    Ok(plan
                        .generation()
                        .map_or(Counted::Kept, |r#gen| Counted::Published { r#gen }))
                }
                Decision::Discard(why) => Ok(Counted::Skipped(why)),
            }
        },
    );
    match taken {
        Taken::Done(Ok(counted)) => counted,
        Taken::Done(Err(refused)) => Counted::Refused(refused),
        Taken::Unlocked(why) if why.kind() == io::ErrorKind::WouldBlock => Counted::Busy,
        Taken::Unlocked(why) => Counted::Refused(Refused::Io(format!("{LOCK}: {why}"))),
        Taken::Contended => Counted::Contended,
        Taken::Over { recorded, cap } => Counted::Refused(Refused::Snapshot { recorded, cap }),
    }
}

/// Payload reads `Files` made on this thread (`cfg(test)` only).
#[cfg(test)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PayloadReads {
    /// Content opens by `Files::read`, in every mode.
    pub opened: u64,
    /// Of `opened`, under a steady-state hold of the lock.
    pub opened_under_lock: u64,
    /// Of `opened`, under the lock in an owed-intent replay or a repair.
    pub opened_under_lock_exception: u64,
    /// Directory listings by `Files::list`.
    pub listed: u64,
    pub listed_under_lock: u64,
    pub listed_under_lock_exception: u64,
}

/// The payload reads since the last call.
#[cfg(test)]
#[must_use]
pub fn take_payload_reads() -> PayloadReads {
    observe::PAYLOAD.with(std::cell::Cell::take)
}

/// One lock-taking attempt of a taker (`cfg(test)` only).
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attempt {
    /// The lock was not taken before the deadline.
    Busy,
    /// A dependency had changed: released, nothing written.
    Stale,
    /// Decided with nothing to apply.
    Decided,
    /// A plan applied.
    Applied,
}

/// The attempts since the last call, in order.
#[cfg(test)]
#[must_use]
pub fn take_attempts() -> Vec<Attempt> {
    observe::ATTEMPTS.with(std::cell::RefCell::take)
}

/// Override [`SNAPSHOT_CAP`] on this thread; `None` restores it.
#[cfg(test)]
pub fn set_snapshot_cap(cap: Option<u64>) {
    observe::CAP.with(|held| held.set(cap));
}

/// Install `hook`, called by every taker after each warm and before that
/// attempt's lock request with the root and the attempt number, until
/// [`clear_before_lock`].
#[cfg(test)]
pub fn before_lock(hook: impl FnMut(&Path, u32) + 'static) {
    observe::BEFORE.with(|held| *held.borrow_mut() = Some(Box::new(hook)));
}

/// Remove the hook [`before_lock`] installed.
#[cfg(test)]
pub fn clear_before_lock() {
    observe::BEFORE.with(|held| *held.borrow_mut() = None);
}

// --- the transaction (ruling 19, option A) --------------------------------------

/// How one taker call ended.
#[derive(Debug)]
pub(super) enum Taken<T> {
    /// Decided under the lock (whatever `T` says).
    Done(T),
    /// The lock was not taken: held past the deadline (`WouldBlock`), or the
    /// lock path refused.
    Unlocked(io::Error),
    /// Every attempt was stale.
    Contended,
    /// The warm recorded more than the snapshot cap.
    Over { recorded: u64, cap: u64 },
}

/// Whether `recorded` bytes pass `cap`: the one comparison the product and
/// the tests' lowered cap share.
const fn over_budget(recorded: u64, cap: u64) -> bool {
    recorded > cap
}

/// The snapshot cap in force.
fn snapshot_cap() -> u64 {
    #[cfg(test)]
    {
        if let Some(cap) = observe::cap() {
            return cap;
        }
    }
    SNAPSHOT_CAP
}

/// Run `run` as a writer holding `stats.lock` must (ruling 19): warm, without
/// the lock, then take the lock within what is left of ONE deadline of `wait`
/// and run it held; a stale attempt released and tried again from a new warm,
/// at most [`ATTEMPTS`] times. `held` gains each attempt's hold, from the
/// lock taken to the lock released.
pub(super) fn transact<T>(
    root: &Path,
    files: &mut Files,
    journals: &mut FileJournals,
    wait: Duration,
    held: &mut Duration,
    run: impl FnMut(&mut Files, &mut FileJournals) -> T,
) -> Taken<T> {
    attempted(root, files, journals, wait, held, Rewarm::Always, run)
}

/// Whether a stale attempt warms again once the call's deadline has passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rewarm {
    /// Every attempt warms (the maintainer and purge: a deferral costs a round).
    Always,
    /// No warm starts after the deadline: the call gives up as busy (an
    /// event-time count, whose journal stays authoritative; lead ruling r2.4).
    WithinDeadline,
}

/// [`transact`], with the re-warm rule `rewarm`.
fn attempted<T>(
    root: &Path,
    files: &mut Files,
    journals: &mut FileJournals,
    wait: Duration,
    held: &mut Duration,
    rewarm: Rewarm,
    mut run: impl FnMut(&mut Files, &mut FileJournals) -> T,
) -> Taken<T> {
    let mut deadline: Option<std::time::Instant> = None;
    #[cfg_attr(
        not(test),
        allow(
            unused_variables,
            reason = "the attempt number and the times reach only the tests' observers"
        )
    )]
    for attempt in 1..=ATTEMPTS {
        if rewarm == Rewarm::WithinDeadline
            && deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline)
        {
            #[cfg(test)]
            {
                observe::attempt(Attempt::Busy);
            }
            return Taken::Unlocked(io::Error::new(
                io::ErrorKind::WouldBlock,
                "the deadline passed before another warm",
            ));
        }
        let warmed = std::time::Instant::now();
        let mut rewarmed = false;
        loop {
            files.warm();
            journals.warm();
            drop(run(files, journals));
            let recorded = files.recorded().saturating_add(journals.recorded());
            let cap = snapshot_cap();
            if over_budget(recorded, cap) {
                files.open();
                journals.open();
                return Taken::Over { recorded, cap };
            }
            // A root's first taker makes the lock file, which changes the
            // root listing this warm recorded: made unlocked, warmed again,
            // once.
            if rewarmed || !Payload::at(root).made() {
                break;
            }
            rewarmed = true;
        }
        let warm = warmed.elapsed();
        #[cfg(test)]
        {
            observe::before(root, attempt);
        }
        let now = std::time::Instant::now();
        let left = deadline
            .get_or_insert_with(|| now + wait)
            .saturating_duration_since(now);
        let lock = match Payload::at(root).lock(left) {
            Ok(lock) => lock,
            Err(why) => {
                #[cfg(test)]
                {
                    observe::attempt(Attempt::Busy);
                }
                files.open();
                journals.open();
                return Taken::Unlocked(why);
            }
        };
        let taken = lock.acquired();
        files.hold();
        journals.hold();
        let done = run(files, journals);
        let stale = files.stale() || journals.stale();
        let wrote = files.wrote();
        drop(lock);
        let span = taken.elapsed();
        *held = held.saturating_add(span);
        #[cfg(test)]
        {
            observe::hold(Hold {
                waited: taken.saturating_duration_since(now),
                warm,
                held: span,
            });
        }
        files.open();
        journals.open();
        if stale {
            #[cfg(test)]
            {
                observe::attempt(Attempt::Stale);
            }
            continue;
        }
        #[cfg(test)]
        {
            observe::attempt(if wrote {
                Attempt::Applied
            } else {
                Attempt::Decided
            });
        }
        return Taken::Done(done);
    }
    Taken::Contended
}

/// One lock-holding attempt's times (`cfg(test)` only, for the profile).
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hold {
    /// From the lock requested to the lock taken.
    pub waited: Duration,
    /// The warm before it.
    pub warm: Duration,
    /// From the lock taken to the lock released.
    pub held: Duration,
}

/// The holds since the last call.
#[cfg(test)]
#[must_use]
pub fn take_holds() -> Vec<Hold> {
    observe::HOLDS.with(std::cell::RefCell::take)
}

// --- reads --------------------------------------------------------------------

fn fetch(dir: &mut dyn Dir, name: &str, cap: u64) -> Result<Option<Vec<u8>>, Refused> {
    match dir.read(name, cap) {
        Read::Absent => Ok(None),
        Read::Bytes(bytes) => Ok(Some(bytes)),
        Read::Oversized => Err(Refused::OverCap {
            file: name.to_owned(),
            cap,
        }),
        Read::Invalid(why) => Err(Refused::Damaged {
            file: name.to_owned(),
            reason: why,
        }),
        Read::Unreadable(why) => Err(Refused::Io(format!("{name}: {why}"))),
    }
}

fn damaged(name: &str) -> impl Fn(Error) -> Refused + '_ {
    move |why| Refused::Damaged {
        file: name.to_owned(),
        reason: match why {
            Error::Damaged(reason) => reason.to_owned(),
            other => format!("{other:?}"),
        },
    }
}

fn utf8<'a>(name: &str, bytes: &'a [u8]) -> Result<&'a str, Refused> {
    std::str::from_utf8(bytes).map_err(|_| Refused::Damaged {
        file: name.to_owned(),
        reason: "not UTF-8".to_owned(),
    })
}

fn wrong(name: &str, reason: &str) -> Refused {
    Refused::Damaged {
        file: name.to_owned(),
        reason: reason.to_owned(),
    }
}

fn list(dir: &mut dyn Dir, name: &str) -> Result<Vec<String>, Refused> {
    dir.list(name).map_err(|why| {
        Refused::Io(format!(
            "{}: {why}",
            if name.is_empty() { "." } else { name }
        ))
    })
}

/// Whether the payload directory is there.
fn built(dir: &mut dyn Dir) -> Result<bool, Refused> {
    Ok(list(dir, "")?.iter().any(|name| name == PAYLOAD))
}

/// The payload's epoch, `None` only while there is no payload: a payload with
/// no epoch file is damaged, never epoch 0.
fn read_epoch(dir: &mut dyn Dir) -> Result<Option<u64>, Refused> {
    match fetch(dir, EPOCH, EPOCH_CAP)? {
        Some(bytes) => doc::parse_epoch(&bytes).map(Some).map_err(damaged(EPOCH)),
        None if built(dir)? => Err(wrong(EPOCH, "missing")),
        None => Ok(None),
    }
}

/// What a document proves, when it can: damaged or over its cap it proves
/// nothing, but a file that cannot be read is an error, never passed over.
fn proof(
    dir: &mut dyn Dir,
    name: &str,
    cap: u64,
    epoch_of: impl Fn(&[u8]) -> Option<u64>,
) -> Result<Option<u64>, Refused> {
    match fetch(dir, name, cap) {
        Ok(bytes) => Ok(bytes.as_deref().and_then(epoch_of)),
        Err(refused) if refused.damage().is_some() => Ok(None),
        Err(refused) => Err(refused),
    }
}

fn epoch_text(bytes: &[u8]) -> Option<u64> {
    doc::parse_epoch(bytes).ok()
}

/// Whether the retired payload `stats.old.<epoch>` carries the epoch its
/// name says: only then does it count.
fn retired_valid(dir: &mut dyn Dir, epoch: u64) -> Result<bool, Refused> {
    let name = format!("{}/epoch", retired_name(epoch));
    Ok(proof(dir, &name, EPOCH_CAP, epoch_text)? == Some(epoch))
}

/// The highest epoch anything left proves: the epoch file, the epoch field of
/// a readable root or session view, and each retired payload whose content
/// carries its name's epoch. `None` when nothing proves one.
fn authority(dir: &mut dyn Dir) -> Result<Option<u64>, Refused> {
    let mut proven = vec![proof(dir, EPOCH, EPOCH_CAP, epoch_text)?];
    proven.push(proof(dir, ROOT, ROOT_VIEW_CAP, |bytes| {
        let text = std::str::from_utf8(bytes).ok()?;
        doc::parse_root(text).ok().map(|root| root.epoch)
    })?);
    for name in list(dir, SESSIONS)? {
        if name.strip_suffix(".view").and_then(doc::key_of).is_none() {
            continue;
        }
        proven.push(proof(
            dir,
            &format!("{SESSIONS}/{name}"),
            SESSION_VIEW_CAP,
            |bytes| {
                let text = std::str::from_utf8(bytes).ok()?;
                doc::parse_session(text).ok().map(|view| view.epoch)
            },
        )?);
    }
    for old in list(dir, "")?.iter().filter_map(|name| retired(name)) {
        if retired_valid(dir, old)? {
            proven.push(Some(old));
        }
    }
    Ok(proven.into_iter().flatten().max())
}

fn load_root(dir: &mut dyn Dir, epoch: u64) -> Result<Option<Root>, Refused> {
    let Some(bytes) = fetch(dir, ROOT, ROOT_VIEW_CAP)? else {
        return Ok(None);
    };
    let root = doc::parse_root(utf8(ROOT, &bytes)?).map_err(damaged(ROOT))?;
    if root.epoch != epoch {
        return Err(wrong(ROOT, "epoch"));
    }
    Ok(Some(root))
}

fn load_view(dir: &mut dyn Dir, epoch: u64, key: &Key) -> Result<Option<SessionView>, Refused> {
    let name = view_name(key);
    let Some(bytes) = fetch(dir, &name, SESSION_VIEW_CAP)? else {
        return Ok(None);
    };
    let view = doc::parse_session(utf8(&name, &bytes)?).map_err(damaged(&name))?;
    if view.epoch != epoch || view.key != *key {
        return Err(wrong(&name, "epoch or group"));
    }
    Ok(Some(view))
}

fn load_group(dir: &mut dyn Dir, epoch: u64, key: &Key) -> Result<Option<Group>, Refused> {
    let name = evidence_name(key);
    let Some(bytes) = fetch(dir, &name, EVIDENCE_CAP)? else {
        return Ok(None);
    };
    let (group, written) = doc::parse_group(utf8(&name, &bytes)?).map_err(damaged(&name))?;
    if written != epoch || group.key != *key {
        return Err(wrong(&name, "epoch or group"));
    }
    Ok(Some(group))
}

/// Every session view but those of `skip`.
fn all_views(
    dir: &mut dyn Dir,
    epoch: u64,
    skip: &BTreeSet<Key>,
) -> Result<BTreeMap<Key, SessionView>, Refused> {
    let mut views = BTreeMap::new();
    for name in list(dir, SESSIONS)? {
        let Some(key) = name.strip_suffix(".view").and_then(doc::key_of) else {
            continue;
        };
        if skip.contains(&key) {
            continue;
        }
        let view = load_view(dir, epoch, &key)?
            .ok_or_else(|| wrong(&view_name(&key), "vanished while read"))?;
        views.insert(key, view);
    }
    Ok(views)
}

/// One source the store holds, as the session view of its group shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Entry {
    pub(super) key: Key,
    /// Its incarnation and cursor: what a removal of it is checked against.
    pub(super) stamp: (u64, u64),
    pub(super) counted: bool,
    /// The archive tree hash a validation proved, when one was stored.
    pub(super) fp: Option<String>,
}

/// What the store holds, read without the lock: every source with its
/// group, and the groups whose files hold no source and no count, which a
/// crash after a removal's commit may have left.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Census {
    pub(super) sources: BTreeMap<String, Entry>,
    pub(super) empty: Vec<Key>,
    /// The epoch read, and each indexed group's generation with the sources
    /// its view held then: a group whose root row still names that
    /// generation is not read again.
    epoch: Option<u64>,
    groups: BTreeMap<Key, (u64, Vec<(String, Entry)>)>,
}

/// The maintainer's census of the store. Only the root, the directory
/// listing and the session views whose generation moved since `prior` are
/// read: O(changed groups) per round.
///
/// # Errors
///
/// A payload file over its cap or damaged, or an IO error.
pub(super) fn census(dir: &mut dyn Dir, prior: &Census) -> Result<Census, Refused> {
    let mut census = Census::default();
    let Some(epoch) = read_epoch(dir)? else {
        return Ok(census);
    };
    census.epoch = Some(epoch);
    let rows: BTreeMap<Key, u64> = load_root(dir, epoch)?
        .map(|root| {
            root.rows
                .into_iter()
                .map(|(key, row)| (key, row.generation))
                .collect()
        })
        .unwrap_or_default();
    let mut keys: BTreeSet<Key> = rows.keys().cloned().collect();
    for name in list(dir, SESSIONS)? {
        if let Some(key) = name
            .strip_suffix(".view")
            .or_else(|| name.strip_suffix(".evidence"))
            .and_then(doc::key_of)
        {
            keys.insert(key);
        }
    }
    for key in keys {
        let generation = rows.get(&key).copied();
        let kept = prior
            .groups
            .get(&key)
            .filter(|(seen, _)| prior.epoch == Some(epoch) && generation == Some(*seen));
        let entries = if let Some((_, entries)) = kept {
            entries.clone()
        } else {
            let view = load_view(dir, epoch, &key)?;
            if view
                .as_ref()
                .is_none_or(|view| view.rows.is_empty() && view.tally == Tally::default())
            {
                census.empty.push(key.clone());
                continue;
            }
            view.iter()
                .flat_map(|view| &view.rows)
                .map(|(id, row)| {
                    let entry = Entry {
                        key: key.clone(),
                        stamp: (row.provenance.incarnation, row.cursor),
                        counted: !row.failed,
                        fp: row.provenance.fp.clone(),
                    };
                    (id.clone(), entry)
                })
                .collect()
        };
        census.sources.extend(
            entries
                .iter()
                .map(|(id, entry)| (id.clone(), entry.clone())),
        );
        if let Some(generation) = generation {
            census.groups.insert(key, (generation, entries));
        }
    }
    Ok(census)
}

fn intent_of(dir: &mut dyn Dir) -> Result<Option<Intent>, Refused> {
    fetch(dir, INTENT, INTENT_CAP)?
        .map(|bytes| doc::parse_intent(utf8(INTENT, &bytes)?).map_err(damaged(INTENT)))
        .transpose()
}

/// Whether a crash left work [`recovery`] would do: a staged or retired
/// payload, an intent, a temp. It lists names only and reads no file, so it
/// is cheap enough to ask without the lock before every idle round.
///
/// # Errors
///
/// A directory that cannot be listed.
pub fn owes(dir: &mut dyn Dir) -> Result<bool, Refused> {
    let top = list(dir, "")?;
    if top
        .iter()
        .any(|name| name == STAGING || retired(name).is_some())
    {
        return Ok(true);
    }
    if !top.iter().any(|name| name == PAYLOAD) {
        return Ok(false);
    }
    let inside = list(dir, PAYLOAD)?;
    if inside
        .iter()
        .any(|name| name == "intent" || name.ends_with(TEMP))
    {
        return Ok(true);
    }
    Ok(list(dir, SESSIONS)?.iter().any(|name| name.ends_with(TEMP)))
}

/// The payload reads a preparation of `source` makes, and nothing of its
/// journal: the epoch, then its group's session view.
///
/// # Errors
///
/// What those reads meet: damage, a file over its cap, an IO error.
pub fn verify(dir: &mut dyn Dir, source: &SourceSpec) -> Result<(), Refused> {
    let epoch = read_epoch(dir)?.unwrap_or(0);
    let (key, _) = source.origin();
    load_view(dir, epoch, &key).map(drop)
}

/// Refused with [`Refused::RecoveryOwed`] while a crash left work behind.
fn owed(dir: &mut dyn Dir) -> Result<(), Refused> {
    if owes(dir)? {
        return Err(Refused::RecoveryOwed);
    }
    Ok(())
}

// --- writing --------------------------------------------------------------------

/// Stage `files`, publish `intent`, then rename the files in their order and
/// clear the intent. `first` names the files that must exist before staging.
fn staged(first: Vec<Op>, files: Vec<(String, String)>, intent: &Intent) -> Plan {
    let mut ops = first;
    for (name, text) in &files {
        ops.push(Op::Write {
            name: temp(name),
            bytes: text.clone().into_bytes(),
        });
    }
    ops.push(Op::Write {
        name: temp(INTENT),
        bytes: doc::render_intent(intent).into_bytes(),
    });
    ops.push(Op::Rename {
        from: temp(INTENT),
        to: INTENT.to_owned(),
    });
    for (name, _) in files {
        ops.push(Op::Rename {
            from: temp(&name),
            to: name,
        });
    }
    ops.push(Op::Remove(INTENT.to_owned()));
    Plan {
        ops,
        generation: Some(intent.generation),
    }
}

/// A file's text, refused when it would pass the cap its readers hold it to.
fn capped(name: String, text: String, cap: u64) -> Result<(String, String), Refused> {
    if u64::try_from(text.len()).unwrap_or(u64::MAX) > cap {
        return Err(Refused::Io(format!(
            "{name} would pass its cap of {cap} bytes"
        )));
    }
    Ok((name, text))
}

/// The root after `views` replace their groups': by difference when the root
/// and the views it was made from agree, else added up again from every view.
/// A missing root is no baseline: the other groups' views are still there.
fn rooted(
    dir: &mut dyn Dir,
    epoch: u64,
    root: Option<Root>,
    views: &[SessionView],
    generation: u64,
) -> Result<Root, Refused> {
    let mut all = root.as_ref().map(|root| root.all.clone());
    let mut root = root.unwrap_or_else(|| Root::empty(epoch));
    for view in views {
        let old = load_view(dir, epoch, &view.key)?;
        let agreed = root.rows.get(&view.key).map(|row| row.generation)
            == old.as_ref().map(|old| old.generation);
        all = all.filter(|_| agreed).and_then(|mut all| {
            if let Some(old) = &old {
                all.sub(&old.tally)?;
            }
            all.checked_add(&view.tally)?;
            Some(all)
        });
    }
    if let Some(all) = all {
        root.all = all;
        for view in views {
            root.rows.insert(view.key.clone(), doc::RootRow::of(view));
            if view.key.is_none() {
                root.covering = view.tally.covering();
            }
        }
        root.all.set_range(root.range());
    } else {
        let mine: BTreeSet<Key> = views.iter().map(|view| view.key.clone()).collect();
        let mut every = all_views(dir, epoch, &mine)?;
        for view in views {
            every.insert(view.key.clone(), view.clone());
        }
        root = Root::sum(epoch, generation, every.values());
    }
    root.epoch = epoch;
    root.generation = generation;
    Ok(root)
}

/// The plan that publishes `primary` and the groups `touched` with it.
fn commit(
    dir: &mut dyn Dir,
    epoch: u64,
    root: Option<Root>,
    primary: Group,
    touched: BTreeSet<Key>,
    generation: u64,
) -> Result<Plan, Refused> {
    let mut groups = Vec::new();
    for key in &touched {
        if *key != primary.key {
            let mut group = load_group(dir, epoch, key)?.unwrap_or_else(|| Group::new(key.clone()));
            group.refragment(&primary);
            groups.push(group);
        }
    }
    groups.insert(0, primary);
    let views: Vec<SessionView> = groups
        .iter()
        .map(|group| SessionView::of(group, epoch, generation))
        .collect();
    let mut root = rooted(dir, epoch, root, &views, generation)?;
    // A group left with no source and no fragment goes: its row now, its
    // files once the intent is gone, so recovery never meets it missing.
    let empty: Vec<Key> = groups
        .iter()
        .filter(|group| group.own.is_empty() && group.fragments.is_empty())
        .map(|group| group.key.clone())
        .collect();
    if !empty.is_empty() {
        for key in &empty {
            root.rows.remove(key);
            if key.is_none() {
                root.covering = Tally::default();
            }
        }
        root.all.set_range(root.range());
    }
    let mut first = Vec::new();
    if built(dir)? {
        if !list(dir, PAYLOAD)?.iter().any(|name| name == "session") {
            first.push(Op::MakeDir(SESSIONS.to_owned()));
        }
    } else {
        // The first build is staged like a repair, so a payload never exists
        // without its epoch file.
        first.extend([
            Op::MakeDir(STAGING.to_owned()),
            Op::Write {
                name: format!("{STAGING}/epoch"),
                bytes: doc::render_epoch(epoch).into_bytes(),
            },
            Op::MakeDir(format!("{STAGING}/session")),
            Op::Rename {
                from: STAGING.to_owned(),
                to: PAYLOAD.to_owned(),
            },
        ]);
    }
    let mut files = Vec::new();
    for group in &groups {
        files.push(capped(
            evidence_name(&group.key),
            doc::render_group(group, epoch),
            EVIDENCE_CAP,
        )?);
    }
    for view in &views {
        files.push(capped(
            view_name(&view.key),
            doc::render_session(view),
            SESSION_VIEW_CAP,
        )?);
    }
    files.push(capped(
        ROOT.to_owned(),
        doc::render_root(&root),
        ROOT_VIEW_CAP,
    )?);
    let intent = Intent {
        epoch,
        generation,
        primary: groups
            .first()
            .map(|group| group.key.clone())
            .unwrap_or_default(),
        touched,
    };
    let mut plan = staged(first, files, &intent);
    for key in &empty {
        plan.ops.push(Op::Remove(view_name(key)));
        plan.ops.push(Op::Remove(evidence_name(key)));
    }
    Ok(plan)
}

/// Under the stats lock, after [`recovery`]: the plan that publishes `work`,
/// or why it is dropped. A source's stamp and its journal are checked again
/// here; a source's evidence takes the read at its cursor, keeping the
/// certificates written ahead of it.
///
/// # Errors
///
/// Owed recovery, or a payload file over its cap or damaged.
pub fn publish(
    dir: &mut dyn Dir,
    journal: &mut dyn Journal,
    work: &Prepared,
) -> Result<Decision, Refused> {
    owed(dir)?;
    let epoch = read_epoch(dir)?.unwrap_or(0);
    if epoch != work.base.epoch {
        return Ok(Decision::Discard(Why::Epoch));
    }
    let mut root = load_root(dir, epoch)?;
    // An index an earlier format did not write is made from the views first:
    // the ownership proof below reads it.
    if let Some(generation) = root
        .as_ref()
        .filter(|root| !root.indexed())
        .map(|root| root.generation)
    {
        root = Some(resummed(dir, epoch, generation)?);
    }
    let mut primary =
        load_group(dir, epoch, &work.key)?.unwrap_or_else(|| Group::new(work.key.clone()));
    let stamp = primary
        .own
        .get(&work.source)
        .map(|own| (own.provenance.incarnation, own.evidence.cursor));
    if stamp != work.base.stamp {
        return Ok(Decision::Discard(Why::Stamp));
    }
    if work.base.stamp.is_none()
        && root
            .as_ref()
            .is_some_and(|root| root.owner(&work.source).is_some())
    {
        return Ok(Decision::Discard(Why::Owned));
    }
    let failed = matches!(
        work.fresh.as_ref().map(|fresh| &fresh.origin),
        Some(Origin::Failed { .. })
    );
    match journal.standing(&work.source) {
        Standing::Gone => return Ok(Decision::Discard(Why::Gone)),
        Standing::Claimed if !failed => return Ok(Decision::Discard(Why::Gone)),
        Standing::Claimed | Standing::Present | Standing::Unknown => {}
    }
    let copied = match &work.shift {
        Shift::Rebind { donor, stamp } => {
            let Some(own) = primary.own.get(donor).filter(|own| {
                (own.provenance.incarnation, own.evidence.cursor) == *stamp
                    && !matches!(own.evidence.origin, Origin::Failed { .. })
            }) else {
                return Ok(Decision::Discard(Why::Stamp));
            };
            Some(own.evidence.clone())
        }
        Shift::Read | Shift::Refresh(_) => None,
    };
    if let Some(witness) = &work.witness
        && !witness.holds(journal, &work.source, work.span().1)
    {
        return Ok(Decision::Discard(Why::Witness));
    }
    let generation = root
        .as_ref()
        .map_or(0, |root| root.generation)
        .checked_add(1)
        .ok_or_else(|| Refused::Io("generation".to_owned()))?;
    let mut touched: BTreeSet<Key> = primary
        .own
        .get(&work.source)
        .map(|own| own.evidence.groups())
        .unwrap_or_default();
    if let Some(fresh) = &work.fresh {
        primary.own.insert(
            work.source.clone(),
            Own {
                evidence: Evidence::new(fresh.origin.clone()),
                provenance: Provenance::none(generation),
            },
        );
    }
    let own = primary
        .own
        .get_mut(&work.source)
        .ok_or_else(|| Refused::UnknownSource(work.source.clone()))?;
    shifted(own, work, generation, copied)?;
    touched.extend(own.evidence.groups());
    touched.insert(work.key.clone());
    commit(dir, epoch, root, primary, touched, generation).map(Decision::Publish)
}

/// `work` applied to its source's record `own`, at `generation`: a read's
/// lines absorbed, a refresh's origin and incarnation, or a rebind's copy of
/// the donor's evidence (`copied`) up to its cursor.
fn shifted(
    own: &mut Own,
    work: &Prepared,
    generation: u64,
    copied: Option<Evidence>,
) -> Result<(), Refused> {
    match &work.shift {
        Shift::Read => {
            if let Some(digest) = &work.digest {
                own.evidence.absorb(digest.clone());
            }
            if let Some(seen) = &work.witness {
                own.provenance = Provenance {
                    incarnation: own.provenance.incarnation,
                    seen: seen.seen,
                    read: seen.read,
                    dev: seen.dev,
                    ino: seen.ino,
                    hash: seen.hash.clone(),
                    skipping: work.skipping,
                    fp: own.provenance.fp.clone(),
                };
            }
        }
        Shift::Refresh(origin) => {
            if let Some(origin) = origin {
                own.evidence.origin = origin.clone();
            }
            own.provenance.incarnation = generation;
        }
        Shift::Rebind { .. } => {
            let (Some(mut evidence), Some(seen)) = (copied, work.witness.as_ref()) else {
                return Err(Refused::UnknownSource(work.source.clone()));
            };
            evidence.origin = own.evidence.origin.clone();
            let cursor = evidence.cursor;
            drop(evidence.lines.split_off(&cursor));
            for offsets in evidence.named.values_mut() {
                offsets.retain(|offset| *offset < cursor);
            }
            evidence.named.retain(|_, offsets| !offsets.is_empty());
            own.evidence = evidence;
            own.provenance = Provenance {
                incarnation: generation,
                seen: seen.seen,
                read: seen.read,
                dev: seen.dev,
                ino: seen.ino,
                hash: seen.hash.clone(),
                skipping: work.skipping,
                fp: None,
            };
        }
    }
    if let Some(fp) = &work.fp {
        own.provenance.fp = Some(fp.clone());
    }
    Ok(())
}

/// The root added up again from every session view: an index an earlier
/// format did not write, made.
fn resummed(dir: &mut dyn Dir, epoch: u64, generation: u64) -> Result<Root, Refused> {
    let views = all_views(dir, epoch, &BTreeSet::new())?;
    Ok(Root::sum(epoch, generation, views.values()))
}

/// Under the stats lock, after [`recovery`]: the plan that drops `source`
/// from `group` and everything it counted, its records about other groups
/// included. `base = Some(stamp)` drops it only while it has that stamp;
/// `None` whatever its stamp (a purge). A source not there is an empty plan.
/// A group the removal leaves with no source and no fragment is pruned in the
/// same plan, after the intent is gone.
///
/// # Errors
///
/// Owed recovery, or a payload file over its cap or damaged.
pub fn remove(
    dir: &mut dyn Dir,
    group: Option<&str>,
    source: &str,
    base: Option<(u64, u64)>,
) -> Result<Decision, Refused> {
    owed(dir)?;
    let nothing = || Ok(Decision::Publish(Plan::of(Vec::new())));
    if group.is_some_and(|id| !is_canonical_uuid(id)) {
        return nothing();
    }
    let key: Key = group.map(ToOwned::to_owned);
    let Some(epoch) = read_epoch(dir)? else {
        return nothing();
    };
    let root = load_root(dir, epoch)?;
    let Some(mut primary) = load_group(dir, epoch, &key)? else {
        return nothing();
    };
    let Some(own) = primary.own.get(source) else {
        return nothing();
    };
    if base.is_some_and(|base| base != (own.provenance.incarnation, own.evidence.cursor)) {
        return Ok(Decision::Discard(Why::Stamp));
    }
    let mut touched = own.evidence.groups();
    touched.insert(key);
    primary.own.remove(source);
    let generation = root
        .as_ref()
        .map_or(0, |root| root.generation)
        .checked_add(1)
        .ok_or_else(|| Refused::Io("generation".to_owned()))?;
    commit(dir, epoch, root, primary, touched, generation).map(Decision::Publish)
}

/// Under the stats lock, after [`recovery`]: an empty group's leftover files
/// removed, the root's row first: what a crash after a removal's commit left.
/// A group with a source or a fragment, or none at all, is an empty plan.
///
/// # Errors
///
/// Owed recovery, or a payload file over its cap or damaged.
pub fn prune(dir: &mut dyn Dir, group: Option<&str>) -> Result<Plan, Refused> {
    owed(dir)?;
    let nothing = || Ok(Plan::of(Vec::new()));
    if group.is_some_and(|id| !is_canonical_uuid(id)) {
        return nothing();
    }
    let key: Key = group.map(ToOwned::to_owned);
    let Some(epoch) = read_epoch(dir)? else {
        return nothing();
    };
    let evidence = load_group(dir, epoch, &key)?;
    if evidence
        .as_ref()
        .is_some_and(|group| !group.own.is_empty() || !group.fragments.is_empty())
    {
        return nothing();
    }
    let view = load_view(dir, epoch, &key)?;
    if view
        .as_ref()
        .is_some_and(|view| !view.rows.is_empty() || view.tally != Tally::default())
    {
        return nothing();
    }
    let mut ops = Vec::new();
    let root = load_root(dir, epoch)?.filter(|root| root.rows.contains_key(&key));
    // A missing document is no proof of an empty group. A root row is
    // removed only before its view, so a row whose view is gone may still
    // count fragments the root adds up, and a row that lists sources over an
    // empty view disagrees with it: both are damage, the root kept for repair.
    if root
        .as_ref()
        .and_then(|root| root.rows.get(&key))
        .is_some_and(|row| {
            view.is_none()
                || row.listed > 0
                || row.pending > 0
                || row.ids.as_ref().is_some_and(|ids| !ids.is_empty())
        })
    {
        return Err(wrong(&view_name(&key), "missing for a counted group"));
    }
    if let Some(mut root) = root {
        root.rows.remove(&key);
        if key.is_none() {
            root.covering = Tally::default();
        }
        root.all.set_range(root.range());
        root.generation = root
            .generation
            .checked_add(1)
            .ok_or_else(|| Refused::Io("generation".to_owned()))?;
        let (name, text) = capped(ROOT.to_owned(), doc::render_root(&root), ROOT_VIEW_CAP)?;
        ops.push(Op::Write {
            name: temp(&name),
            bytes: text.into_bytes(),
        });
        ops.push(Op::Rename {
            from: temp(&name),
            to: name,
        });
    }
    if view.is_some() {
        ops.push(Op::Remove(view_name(&key)));
    }
    if evidence.is_some() {
        ops.push(Op::Remove(evidence_name(&key)));
    }
    Ok(Plan::of(ops))
}

/// How long `_archive-purge` waits for `stats.lock`. The hold after it is
/// not bounded.
pub const PURGE_WAIT: Duration = Duration::from_secs(2);

/// What a purge did to the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Purged {
    /// There is no payload.
    Absent,
    /// The archive's source is gone from it, or was never in it.
    Removed,
    /// Damage met: the payload was repaired, empty, in a later epoch.
    Reset { file: String, reason: String },
}

/// `_archive-purge`'s store step for archive `aid` under the state root
/// `root`: under `stats.lock` (within `wait`), recovery applied whole, then
/// the archive's source removed and its group's empty leftovers pruned. Damage met on the way repairs the payload
/// instead, once, so nothing names the archive after it either way.
///
/// # Errors
///
/// The lock not taken within `wait`, an IO error, or a repair refused.
pub fn purged(root: &Path, aid: &str, wait: Duration) -> Result<Purged, Refused> {
    let mut files = Files::at(root);
    let any = list(&mut files, "")?
        .iter()
        .any(|name| name == PAYLOAD || name == STAGING || retired(name).is_some());
    if !any {
        return Ok(Purged::Absent);
    }
    let applied = |files: &mut Files, plan: &Plan| {
        plan.apply(files, plan.ops.len())
            .map_err(|why| Refused::Io(why.to_string()))
    };
    let source = format!("archive/{aid}");
    let mut held = Duration::ZERO;
    let taken = transact(
        root,
        &mut files,
        &mut FileJournals::at(root),
        wait,
        &mut held,
        |files, _| {
            let removed = recovery(files)
                .and_then(|plan| applied(files, &plan))
                .and_then(|()| remove(files, Some(aid), &source, None))
                .and_then(|decision| match decision {
                    Decision::Publish(plan) => applied(files, &plan),
                    Decision::Discard(_) => Ok(()),
                })
                // A removal recovered from a crash, or one whose prune a
                // crash cut short, leaves the archive's empty group naming
                // it: pruned too, whether or not the source was still there.
                .and_then(|()| prune(files, Some(aid)))
                .and_then(|plan| applied(files, &plan));
            match removed {
                Ok(()) => Ok(Purged::Removed),
                Err(refused) => {
                    let Some((file, reason)) = refused.damage() else {
                        return Err(refused);
                    };
                    // The F1 exception: a repair reads under the lock.
                    files.exception(|files| {
                        let plan = repair(files)?;
                        applied(files, &plan)
                    })?;
                    Ok(Purged::Reset { file, reason })
                }
            }
        },
    );
    match taken {
        Taken::Done(outcome) => outcome,
        Taken::Unlocked(why) => Err(Refused::Io(format!("{LOCK}: {why}"))),
        Taken::Contended => Err(Refused::Contended),
        Taken::Over { recorded, cap } => Err(Refused::Snapshot { recorded, cap }),
    }
}

/// Under the stats lock: the plan that adds a certificate or linked `spawn`
/// `line` (LF excluded) of `source` in group `group`, written at journal
/// `offset` ahead of the ordered read. Nothing to do is an empty plan.
///
/// # Errors
///
/// Owed recovery, an unknown or failed source, a line that is no such
/// record, or a payload file over its cap or damaged.
pub fn event(
    dir: &mut dyn Dir,
    group: Option<&str>,
    source: &str,
    offset: u64,
    line: &[u8],
) -> Result<Decision, Refused> {
    owed(dir)?;
    let key: Key = match group {
        Some(id) if !is_canonical_uuid(id) => {
            return Err(Refused::UnknownSource(source.to_owned()));
        }
        group => group.map(ToOwned::to_owned),
    };
    let epoch = read_epoch(dir)?.unwrap_or(0);
    let root = load_root(dir, epoch)?;
    let mut primary =
        load_group(dir, epoch, &key)?.ok_or_else(|| Refused::UnknownSource(source.to_owned()))?;
    let own = primary
        .own
        .get_mut(source)
        .filter(|own| !matches!(own.evidence.origin, Origin::Failed { .. }))
        .ok_or_else(|| Refused::UnknownSource(source.to_owned()))?;
    let kept = own
        .evidence
        .take_event(offset, line, LINE_LIMIT)
        .map_err(|_| Refused::NotAnEvent)?;
    let Some(kept) = kept else {
        return Ok(Decision::Publish(Plan::of(Vec::new())));
    };
    let mut touched = own.evidence.groups();
    own.evidence.keep(offset, kept);
    touched.extend(own.evidence.groups());
    touched.insert(key);
    let generation = root
        .as_ref()
        .map_or(0, |root| root.generation)
        .checked_add(1)
        .ok_or_else(|| Refused::Io("generation".to_owned()))?;
    commit(dir, epoch, root, primary, touched, generation).map(Decision::Publish)
}

/// Under the stats lock: finish or undo what a crash left. An interrupted
/// swap first: the staged payload when it carries the epoch after the newest
/// retired one, or when there is none (a first build), else that retired
/// payload. Then leftover temps, and an intent rebuilt from its primary
/// group's evidence as it stands; a primary staged but never committed keeps
/// its temp until the intent is gone. Applied whole, a second call plans
/// nothing; any prefix of it is a crash a later call still recovers.
///
/// # Errors
///
/// A payload file over its cap or damaged, an active epoch at or below a
/// retired payload that proves its own (amendment 17), a retired payload whose
/// content is not the epoch its name says, or a file or directory that cannot
/// be read.
pub fn recovery(dir: &mut dyn Dir) -> Result<Plan, Refused> {
    let top = list(dir, "")?;
    let olds: BTreeSet<u64> = top.iter().filter_map(|name| retired(name)).collect();
    let staging = top.iter().any(|name| name == STAGING);
    if !top.iter().any(|name| name == PAYLOAD) {
        return swap_back(dir, staging, &olds).map(Plan::of);
    }
    let bytes = fetch(dir, EPOCH, EPOCH_CAP)?.ok_or_else(|| wrong(EPOCH, "missing"))?;
    let epoch = doc::parse_epoch(&bytes).map_err(damaged(EPOCH))?;
    // Amendment 17: a retired payload proving an epoch at or above the active
    // one means the active epoch went back. Removing it would lose that proof.
    for old in olds.iter().filter(|old| **old >= epoch) {
        if retired_valid(dir, *old)? {
            let name = retired_name(*old);
            return Err(wrong(
                EPOCH,
                &format!("epoch {epoch} is not above the retired epoch {old} in {name}"),
            ));
        }
    }
    let mut ops = Vec::new();
    if staging {
        ops.push(Op::RemoveTree(STAGING.to_owned()));
    }
    for old in &olds {
        ops.push(Op::RemoveTree(retired_name(*old)));
    }
    for name in list(dir, PAYLOAD)? {
        if name.ends_with(TEMP) {
            ops.push(Op::Remove(format!("{PAYLOAD}/{name}")));
        }
    }
    let sessions = list(dir, SESSIONS)?;
    let intent = intent_of(dir)?;
    // The primary's staged evidence is the proof that it never committed: it
    // goes only after the intent, so a crash between the two finds either both
    // or a temp alone.
    let staged = intent
        .as_ref()
        .map(|intent| temp(&evidence_name(&intent.primary)))
        .filter(|staged| {
            sessions
                .iter()
                .any(|name| format!("{SESSIONS}/{name}") == *staged)
        });
    for name in &sessions {
        let name = format!("{SESSIONS}/{name}");
        if name.ends_with(TEMP) && staged.as_ref() != Some(&name) {
            ops.push(Op::Remove(name));
        }
    }
    let Some(intent) = intent else {
        return Ok(Plan::of(ops));
    };
    if intent.epoch != epoch {
        return Err(wrong(INTENT, "epoch"));
    }
    ops.extend(replay(dir, epoch, &intent, staged.is_some())?);
    ops.extend(staged.map(Op::Remove));
    Ok(Plan::of(ops))
}

/// With no payload: finish an interrupted swap when the staged payload
/// carries the epoch after the newest retired one, or there is no retired one
/// (amendment 14: a first build), else put that retired payload back. A
/// retired payload counts only when its content carries its name's epoch;
/// every other one goes.
fn swap_back(dir: &mut dyn Dir, staging: bool, olds: &BTreeSet<u64>) -> Result<Vec<Op>, Refused> {
    let mut valid = BTreeSet::new();
    for old in olds {
        if retired_valid(dir, *old)? {
            valid.insert(*old);
        }
    }
    let newest = valid.last().copied();
    if newest.is_none()
        && let Some(bad) = olds.first()
    {
        return Err(wrong(
            &format!("{}/epoch", retired_name(*bad)),
            "differs from its name",
        ));
    }
    let staged = if staging {
        proof(dir, &format!("{STAGING}/epoch"), EPOCH_CAP, epoch_text)?
    } else {
        None
    };
    let finish = match (staged, newest) {
        (Some(staged), Some(newest)) => newest.checked_add(1) == Some(staged),
        (Some(_), None) => true,
        (None, _) => false,
    };
    let mut ops = Vec::new();
    let restored = if finish {
        ops.push(Op::Rename {
            from: STAGING.to_owned(),
            to: PAYLOAD.to_owned(),
        });
        None
    } else {
        if staging {
            ops.push(Op::RemoveTree(STAGING.to_owned()));
        }
        newest.inspect(|newest| {
            ops.push(Op::Rename {
                from: retired_name(*newest),
                to: PAYLOAD.to_owned(),
            });
        })
    };
    for old in olds.iter().filter(|old| Some(**old) != restored) {
        ops.push(Op::RemoveTree(retired_name(*old)));
    }
    Ok(ops)
}

/// The writes that finish `intent`: each touched group's fragments taken
/// again from the primary's evidence, their views, the root, then the
/// intent's removal. A primary still `staged` was not committed: it stands as
/// it was. One neither there nor staged is damage: the commit point is gone.
fn replay(
    dir: &mut dyn Dir,
    epoch: u64,
    intent: &Intent,
    staged: bool,
) -> Result<Vec<Op>, Refused> {
    let primary = match load_group(dir, epoch, &intent.primary)? {
        Some(group) => group,
        None if staged => Group::new(intent.primary.clone()),
        None => {
            return Err(wrong(
                &evidence_name(&intent.primary),
                "missing, though the intent names it",
            ));
        }
    };
    let mut rewritten = Vec::new();
    let mut views = Vec::new();
    for key in &intent.touched {
        if *key == primary.key {
            views.push(SessionView::of(&primary, epoch, intent.generation));
        } else {
            let mut group = load_group(dir, epoch, key)?.unwrap_or_else(|| Group::new(key.clone()));
            group.refragment(&primary);
            views.push(SessionView::of(&group, epoch, intent.generation));
            rewritten.push(group);
        }
    }
    let mut every = all_views(dir, epoch, &intent.touched)?;
    for view in &views {
        every.insert(view.key.clone(), view.clone());
    }
    let root = Root::sum(epoch, intent.generation, every.values());
    let mut files = Vec::new();
    for group in &rewritten {
        files.push(capped(
            evidence_name(&group.key),
            doc::render_group(group, epoch),
            EVIDENCE_CAP,
        )?);
    }
    for view in &views {
        files.push(capped(
            view_name(&view.key),
            doc::render_session(view),
            SESSION_VIEW_CAP,
        )?);
    }
    files.push(capped(
        ROOT.to_owned(),
        doc::render_root(&root),
        ROOT_VIEW_CAP,
    )?);
    let mut ops = Vec::new();
    for (name, text) in &files {
        ops.push(Op::Write {
            name: temp(name),
            bytes: text.clone().into_bytes(),
        });
    }
    for (name, _) in files {
        ops.push(Op::Rename {
            from: temp(&name),
            to: name,
        });
    }
    ops.push(Op::Remove(INTENT.to_owned()));
    Ok(ops)
}

/// Under the stats lock: the plan that replaces the payload with an empty one
/// in the epoch after the highest one anything left proves, so every source
/// is read again from its journal. The old payload is moved aside whole, its
/// epoch file first made to say the epoch its retired name carries, then
/// removed.
///
/// [`recovery`] decides first (amendment 15): work it can still finish is
/// owed; damage it cannot get past is taken over; an IO error refuses, every
/// file kept. A takeover removes what proves nothing first: the intent, every
/// temp, staging and the retired payloads below the proven epoch. Then the
/// active epoch file is made to say the proven epoch, and only then does the
/// retired payload carrying it go, so every prefix keeps the proof. A store
/// never built has nothing to repair.
///
/// # Errors
///
/// Recovery still owed, an IO error, or a payload no surviving evidence
/// proves the epoch of ([`Refused::EpochUnproven`]).
pub fn repair(dir: &mut dyn Dir) -> Result<Plan, Refused> {
    let takeover = match recovery(dir) {
        Ok(plan) if plan.ops.is_empty() => false,
        Ok(_) => return Err(Refused::RecoveryOwed),
        Err(refused) if refused.damage().is_some() => true,
        Err(refused) => return Err(refused),
    };
    let top = list(dir, "")?;
    let payload = top.iter().any(|name| name == PAYLOAD);
    let debris: Vec<&String> = top
        .iter()
        .filter(|name| *name == STAGING || retired(name).is_some())
        .collect();
    if !payload && debris.is_empty() {
        return Ok(Plan::of(Vec::new()));
    }
    let proven = authority(dir)?.ok_or(Refused::EpochUnproven)?;
    let next = proven
        .checked_add(1)
        .ok_or_else(|| Refused::Io("epoch".to_owned()))?;
    let old = retired_name(proven);
    let mut ops = Vec::new();
    let mut proof = None;
    if takeover {
        for name in debris {
            if *name == old {
                proof = Some(Op::RemoveTree(old.clone()));
            } else {
                ops.push(Op::RemoveTree(name.clone()));
            }
        }
        if payload {
            for name in list(dir, PAYLOAD)? {
                if name == "intent" || name.ends_with(TEMP) {
                    ops.push(Op::Remove(format!("{PAYLOAD}/{name}")));
                }
            }
            for name in list(dir, SESSIONS)? {
                if name.ends_with(TEMP) {
                    ops.push(Op::Remove(format!("{SESSIONS}/{name}")));
                }
            }
        }
    }
    if payload {
        let current = dir.read(EPOCH, EPOCH_CAP);
        let says = match &current {
            Read::Bytes(bytes) => epoch_text(bytes),
            _ => None,
        };
        if says != Some(proven) {
            if matches!(current, Read::Invalid(_)) {
                ops.push(Op::RemoveTree(EPOCH.to_owned()));
            }
            ops.push(Op::Write {
                name: temp(EPOCH),
                bytes: doc::render_epoch(proven).into_bytes(),
            });
            ops.push(Op::Rename {
                from: temp(EPOCH),
                to: EPOCH.to_owned(),
            });
        }
    }
    ops.extend(proof);
    ops.push(Op::MakeDir(STAGING.to_owned()));
    ops.push(Op::Write {
        name: format!("{STAGING}/epoch"),
        bytes: doc::render_epoch(next).into_bytes(),
    });
    if payload {
        ops.push(Op::Rename {
            from: PAYLOAD.to_owned(),
            to: old.clone(),
        });
    }
    ops.push(Op::Rename {
        from: STAGING.to_owned(),
        to: PAYLOAD.to_owned(),
    });
    if payload {
        ops.push(Op::RemoveTree(old));
    }
    Ok(Plan::of(ops))
}

/// How many times a reader takes the root and a session view again when a
/// publication moved between the two reads.
const READS: usize = 3;

/// The answer both scopes show, read without the lock: the root's counts and,
/// for a session, its group's beside them, both of one committed state.
pub fn shown(dir: &mut dyn Dir, this: Option<&str>) -> Shown {
    shown_of(dir, this).unwrap_or_else(Shown::Refused)
}

/// A session view is of the root read before it when its generation is the
/// one the root's row records: a publication committed between the two reads
/// makes them differ, and both are read again. Still differing after
/// [`READS`] reads, the answer is [`Shown::Building`] while publications move
/// or an intent is there, else the view is damaged.
fn shown_of(dir: &mut dyn Dir, this: Option<&str>) -> Result<Shown, Refused> {
    let mut first = None;
    let mut moved = false;
    let mut differs = String::new();
    for _ in 0..READS {
        let Some(epoch) = read_epoch(dir)? else {
            return Ok(Shown::Building);
        };
        let Some(root) = load_root(dir, epoch)? else {
            return Ok(Shown::Building);
        };
        moved |= *first.get_or_insert(root.generation) != root.generation;
        let updating = !matches!(dir.read(INTENT, INTENT_CAP), Read::Absent);
        moved |= updating;
        let this = match this {
            None => ThisSession::NoUuid,
            Some(id) => {
                let mut scope = root.covering.clone();
                let key = Some(id.to_owned());
                if is_canonical_uuid(id)
                    && let Some(row) = root.rows.get(&key)
                {
                    let name = view_name(&key);
                    let view =
                        load_view(dir, epoch, &key)?.ok_or_else(|| wrong(&name, "missing"))?;
                    if view.generation != row.generation {
                        differs = name;
                        continue;
                    }
                    scope.add(&view.tally);
                }
                ThisSession::Ready(Box::new(scope.scoped()))
            }
        };
        return Ok(Shown::Ready {
            view: View {
                all: root.all.scoped(),
                this,
            },
            epoch,
            r#gen: root.generation,
            updating,
            pending: root.pending(),
            listed: root.listed(),
        });
    }
    if moved {
        return Ok(Shown::Building);
    }
    Err(wrong(&differs, "generation differs from the root"))
}

/// An answer added up from every session view alone, never the persisted
/// root: what the root must equal.
///
/// # Errors
///
/// A payload file over its cap or damaged, or a directory that cannot be
/// listed.
pub fn resum(dir: &mut dyn Dir) -> Result<Shown, Refused> {
    let Some(epoch) = read_epoch(dir)? else {
        return Ok(Shown::Building);
    };
    let views = all_views(dir, epoch, &BTreeSet::new())?;
    if views.is_empty() && load_root(dir, epoch)?.is_none() {
        return Ok(Shown::Building);
    }
    let generation = views
        .values()
        .map(|view| view.generation)
        .max()
        .unwrap_or_default();
    let root = Root::sum(epoch, generation, views.values());
    let updating = !matches!(dir.read(INTENT, INTENT_CAP), Read::Absent);
    Ok(Shown::Ready {
        view: View {
            all: root.all.scoped(),
            this: ThisSession::NoUuid,
        },
        epoch,
        r#gen: generation,
        updating,
        pending: root.pending(),
        listed: root.listed(),
    })
}

// --- the lock -------------------------------------------------------------------

/// The payload's lock, `<root>/stats.lock`: outside the payload, so a repair
/// swap never moves it, and never removed.
#[derive(Debug, Clone)]
pub struct Payload {
    root: PathBuf,
}

impl Payload {
    #[must_use]
    pub fn at(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    /// Take the lock within `wait`. Locking recovers nothing by itself: the
    /// holder applies [`recovery`] first.
    ///
    /// # Errors
    ///
    /// The lock held elsewhere past `wait` ([`io::ErrorKind::WouldBlock`]), or
    /// a lock path that cannot be opened.
    pub fn lock(&self, wait: Duration) -> io::Result<Locked> {
        let (held, acquired) = crate::store::lock_timed(&self.root.join(LOCK), wait)?;
        #[cfg(test)]
        {
            observe::locked(1);
        }
        Ok(Locked {
            _held: held,
            acquired,
            root: self.root.clone(),
        })
    }

    /// Make the lock file when it is absent, without locking it: `true` when
    /// it was made now (the root directory changed under a warm).
    fn made(&self) -> bool {
        let path = self.root.join(LOCK);
        crate::store::node_meta(&path).is_err_and(|why| why.kind() == io::ErrorKind::NotFound)
            && crate::store::lock_file(&path).is_ok()
    }
}

/// The lock held; dropping it releases.
#[derive(Debug)]
pub struct Locked {
    _held: std::fs::File,
    /// When `flock` was acquired, before the door's identity proof.
    acquired: std::time::Instant,
    root: PathBuf,
}

impl Locked {
    /// The instant the lock was acquired: a hold is timed from it.
    #[must_use]
    pub const fn acquired(&self) -> std::time::Instant {
        self.acquired
    }
}

#[cfg(test)]
impl Drop for Locked {
    fn drop(&mut self) {
        observe::locked(-1);
    }
}

impl Locked {
    /// The payload this lock guards.
    #[must_use]
    pub fn files(&self) -> Files {
        Files::at(&self.root)
    }
}
