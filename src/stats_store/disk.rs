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
//! from the primary group's evidence as it stands. Every file a plan writes is
//! synced before the intent publishes; directories are not.
//!
//! A [`repair`] moves the payload aside whole for an empty one in the epoch
//! after the highest one a surviving document proves; the first build is
//! staged the same way, so a payload never exists without its epoch file. A
//! file that cannot be read is an IO error, never damage: nothing is repaired
//! over it.
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
use crate::stats::{self, SourceGap, ThisSession, View};

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
    const fn of(ops: Vec<Op>) -> Self {
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
#[derive(Debug, Clone)]
pub struct Files {
    root: PathBuf,
}

impl Files {
    #[must_use]
    pub fn at(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
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
}

impl Dir for Files {
    fn read(&mut self, name: &str, cap: u64) -> Read {
        let path = match self.path(name) {
            Ok(path) => path,
            Err(why) => return Read::Unreadable(why.to_string()),
        };
        match crate::store::read_capped(&path, cap) {
            Ok(crate::store::SourceRead::Absent) => Read::Absent,
            Ok(crate::store::SourceRead::Ready(bytes)) => Read::Bytes(bytes),
            Ok(crate::store::SourceRead::Invalid(why)) => Read::Invalid(why),
            Ok(crate::store::SourceRead::Unreadable(why)) => Read::Unreadable(why),
            Err(crate::store::Oversized) => Read::Oversized,
        }
    }

    fn apply(&mut self, op: &Op) -> io::Result<()> {
        use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
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
        Ok(())
    }

    fn list(&mut self, dir: &str) -> io::Result<Vec<String>> {
        let paths = match crate::archive::read_entries(&self.path(dir)?) {
            Ok(paths) => paths,
            Err(why) if why.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(why) => return Err(why),
        };
        let mut names: Vec<String> = paths
            .iter()
            .filter_map(|path| path.file_name()?.to_str().map(ToOwned::to_owned))
            .collect();
        names.sort();
        Ok(names)
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

    use super::{FileEvent, JournalReads, Op};

    thread_local! {
        pub(super) static FILES: RefCell<Vec<FileEvent>> = const { RefCell::new(Vec::new()) };
        pub(super) static SYNC_FAULT: RefCell<Option<String>> = const { RefCell::new(None) };
        pub(super) static JOURNALS: Cell<JournalReads> = const {
            Cell::new(JournalReads { total: 0, under_lock: 0 })
        };
        /// Payload locks this thread holds.
        pub(super) static LOCKS: Cell<u64> = const { Cell::new(0) };
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

/// A bounded look at a journal: its identity and length, and the bytes read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub dev: u64,
    pub ino: u64,
    pub len: u64,
    pub bytes: Vec<u8>,
}

/// Where sources' journals are read.
pub trait Journal {
    /// At most `max` bytes of `source`'s journal from `from`.
    ///
    /// # Errors
    ///
    /// The source's gap when its journal cannot be read.
    fn window(&mut self, source: &str, from: u64, max: u64) -> Result<Window, SourceGap>;
}

/// Journals as real files: the product's [`Journal`]. `live/<name>` is
/// `<root>/sessions/<name>/events.jsonl` and `archive/<uuid>` the archived
/// journal, unless [`FileJournals::track`] named another path.
#[derive(Debug, Clone)]
pub struct FileJournals {
    root: PathBuf,
    paths: BTreeMap<String, PathBuf>,
}

impl FileJournals {
    #[must_use]
    pub fn at(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            paths: BTreeMap::new(),
        }
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
    fn window(&mut self, source: &str, from: u64, max: u64) -> Result<Window, SourceGap> {
        let path = self.path(source).ok_or(SourceGap::Missing)?;
        let (dev, ino, len, bytes) = crate::usage::journal_window(&path, from, max)?;
        #[cfg(test)]
        {
            observe::journal(bytes.len());
        }
        Ok(Window {
            dev,
            ino,
            len,
            bytes,
        })
    }
}

/// Journals in memory: test and fuzz stores.
#[derive(Debug, Clone, Default)]
pub struct MemoryJournals {
    journals: BTreeMap<String, (u64, u64, Vec<u8>)>,
}

impl MemoryJournals {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `source`'s journal is now `bytes`, in the file `(dev, ino)`.
    pub fn set(&mut self, source: &str, dev: u64, ino: u64, bytes: &[u8]) {
        self.journals
            .insert(source.to_owned(), (dev, ino, bytes.to_vec()));
    }
}

impl Journal for MemoryJournals {
    fn window(&mut self, source: &str, from: u64, max: u64) -> Result<Window, SourceGap> {
        let (dev, ino, bytes) = self.journals.get(source).ok_or(SourceGap::Missing)?;
        let len = bytes.len();
        let start = usize::try_from(from).unwrap_or(usize::MAX).min(len);
        let end = start
            .saturating_add(usize::try_from(max).unwrap_or(usize::MAX))
            .min(len);
        Ok(Window {
            dev: *dev,
            ino: *ino,
            len: u64::try_from(len).unwrap_or(u64::MAX),
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

/// What a prepared read assumed of the payload.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Base {
    epoch: u64,
    /// The source's `(incarnation, cursor)`, or `None` when unregistered.
    stamp: Option<(u64, u64)>,
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
/// the source, and its journal's identity and length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tail {
    base: Base,
    dev: u64,
    ino: u64,
    len: u64,
}

impl Tail {
    /// The durable cursor, and the journal's length past it.
    #[must_use]
    pub fn span(&self) -> (u64, u64) {
        let cursor = self.base.stamp.map_or(0, |(_, cursor)| cursor);
        (cursor, self.len)
    }
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
    /// The journal's identity and length when the read found nothing new.
    quiet: Option<(u64, u64, u64)>,
}

impl Reader<'_> {
    fn window(&mut self, from: u64, max: u64) -> Result<Window, SourceGap> {
        self.journal.window(self.id, from, max)
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
        })
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
            self.quiet = Some((tail.dev, tail.ino, tail.len));
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
                    self.quiet = Some((window.dev, window.ino, window.len));
                    return Ok(Next::Nothing);
                }
                None => (Digest::passed(cursor, read_end), true),
            }
        } else if data.contains(&b'\n') || (final_line && !data.is_empty()) {
            (judged(cursor, &data)?, false)
        } else if eof {
            if fresh.is_none() {
                self.quiet = Some((window.dev, window.ino, window.len));
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
            .map(|(next, read, _)| (next, read))
    }

    /// [`Prepared::read_counted`], told the [`Tail`] the last read of this
    /// source that found nothing new saw: when the payload still holds the
    /// source as it did and its journal has the same identity and length, the
    /// journal's length is all that is looked at, no byte of it read. A read
    /// that finds nothing new returns the tail to tell the next one.
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
    ) -> Result<(Next, u64, Option<Tail>), Refused> {
        let epoch = read_epoch(dir)?.unwrap_or(0);
        let (key, origin) = source.origin();
        let row = load_view(dir, epoch, &key)?.and_then(|view| view.rows.get(source.id()).cloned());
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
            quiet: None,
        };
        if matches!(source, SourceSpec::Failed { .. }) {
            let next = if row.is_some() {
                Next::Nothing
            } else {
                reader.registered(origin)
            };
            return Ok((next, 0, None));
        }
        if held.is_none()
            && let Some(quiet) = quiet.filter(|quiet| quiet.base == base)
            && reader
                .window(quiet.len, 0)
                .is_ok_and(|now| (now.dev, now.ino, now.len) == (quiet.dev, quiet.ino, quiet.len))
        {
            return Ok((Next::Nothing, 0, Some(quiet.clone())));
        }
        if let Some(held) = held.filter(|held| held.source == source.id() && held.base == base)
            && let Some(next) = reader.resume(held)?
        {
            return Ok((next, reader.read, None));
        }
        let next = match &row {
            Some(row) if !row.failed => match reader.continued(row)? {
                Some(next) => next,
                None => reader.fresh(Fresh {
                    origin,
                    reset: true,
                })?,
            },
            Some(_) => reader.fresh(Fresh {
                origin,
                reset: true,
            })?,
            None => reader.fresh(Fresh {
                origin,
                reset: false,
            })?,
        };
        let tail = match (&next, reader.quiet) {
            (Next::Nothing, Some((dev, ino, len))) => Some(Tail {
                base: base.clone(),
                dev,
                ino,
                len,
            }),
            _ => None,
        };
        Ok((next, reader.read, tail))
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The durable cursor before and after this work applies.
    #[must_use]
    pub fn span(&self) -> (u64, u64) {
        let end = self.digest.as_ref().map_or(self.start, Digest::end);
        (self.start, end)
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
    let root = rooted(dir, epoch, root, &views, generation)?;
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
    Ok(staged(first, files, &intent))
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
    let root = load_root(dir, epoch)?;
    let mut primary =
        load_group(dir, epoch, &work.key)?.unwrap_or_else(|| Group::new(work.key.clone()));
    let stamp = primary
        .own
        .get(&work.source)
        .map(|own| (own.provenance.incarnation, own.evidence.cursor));
    if stamp != work.base.stamp {
        return Ok(Decision::Discard(Why::Stamp));
    }
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
        };
    }
    touched.extend(own.evidence.groups());
    touched.insert(work.key.clone());
    commit(dir, epoch, root, primary, touched, generation).map(Decision::Publish)
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
/// group's evidence as it stands. Applied whole, a second call plans nothing.
///
/// # Errors
///
/// A payload file over its cap or damaged, a retired payload whose content is
/// not the epoch its name says, or a file or directory that cannot be read.
pub fn recovery(dir: &mut dyn Dir) -> Result<Plan, Refused> {
    let top = list(dir, "")?;
    let olds: BTreeSet<u64> = top.iter().filter_map(|name| retired(name)).collect();
    let staging = top.iter().any(|name| name == STAGING);
    if !top.iter().any(|name| name == PAYLOAD) {
        return swap_back(dir, staging, &olds).map(Plan::of);
    }
    let bytes = fetch(dir, EPOCH, EPOCH_CAP)?.ok_or_else(|| wrong(EPOCH, "missing"))?;
    let epoch = doc::parse_epoch(&bytes).map_err(damaged(EPOCH))?;
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
    for name in &sessions {
        if name.ends_with(TEMP) {
            ops.push(Op::Remove(format!("{SESSIONS}/{name}")));
        }
    }
    let Some(intent) = intent_of(dir)? else {
        return Ok(Plan::of(ops));
    };
    if intent.epoch != epoch {
        return Err(wrong(INTENT, "epoch"));
    }
    let staged = temp(&evidence_name(&intent.primary));
    let staged = sessions
        .iter()
        .any(|name| format!("{SESSIONS}/{name}") == staged);
    ops.extend(replay(dir, epoch, &intent, staged)?);
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
/// owed; damage it cannot get past is taken over, the intent, every temp and
/// any staged or retired payload removed before the swap; an IO error refuses,
/// every file kept. A store never built has nothing to repair.
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
    let mut ops = Vec::new();
    if takeover {
        ops.extend(debris.into_iter().map(|name| Op::RemoveTree(name.clone())));
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
    let old = retired_name(proven);
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
    if views.is_empty() {
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
        let held = crate::store::lock(&self.root.join(LOCK), wait)?;
        #[cfg(test)]
        {
            observe::locked(1);
        }
        Ok(Locked {
            _held: held,
            root: self.root.clone(),
        })
    }
}

/// The lock held; dropping it releases.
#[derive(Debug)]
pub struct Locked {
    _held: std::fs::File,
    root: PathBuf,
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
