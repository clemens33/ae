//! A live session's files, and the only writes to them.
//!
//! The path/lock invariant is structural. Write ownership lives in this
//! facade and is defended by a conservative source tripwire rather than
//! convention:
//!
//! * **one spelling.** A session file's name, and the `.lock` beside it, are
//!   written HERE and nowhere else. A second spelling of a lock is how one
//!   mutual exclusion silently becomes two, so `tests/it/doors.rs` trips when
//!   production code names one of these files in one of its guarded forms.
//! * **one locked append, one retention transaction.**
//!   [`SessionStore::append_event`] and [`SessionStore::append_memo`] are the
//!   only appenders of a session file, and [`SessionStore::retain_events`] is
//!   the only replacement of the event container. The primitives under them
//!   are private, so the owned append and replacement paths stay explicit.
//!   Each append is one transaction: take `<file>.lock`, append, `fdatasync`,
//!   and on any failure cut the file back to the length it had. A caller is
//!   told "recorded" only once the bytes are durable, which is what lets
//!   [`crate::telegram`] read the ledger by offset between the explicit
//!   resume-time retention replacements.
//!
//! [`open`] does no IO. It is a directory, so holding a store commits a caller
//! to nothing and costs nothing.
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The event container's file name, kept across every flip.
pub const EVENTS: &str = "events.jsonl";

/// The memo container.
pub const MEMO: &str = "memo.tsv";

/// The session's own record — roster, mode, origin, goal.
pub const META: &str = "meta";

/// What a file's lock is called: its own name plus this. Appending it by hand
/// is how two writers end up on two different locks.
pub const LOCK_SUFFIX: &str = ".lock";

/// How long a locked append waits for the lock — `flock -w 5`.
pub const LOCK_WAIT: Duration = Duration::from_secs(5);

/// How often the lock is retried while waiting.
const LOCK_POLL: Duration = Duration::from_millis(20);

/// The lock file beside `path`.
#[must_use]
pub fn lock_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(LOCK_SUFFIX);
    PathBuf::from(name)
}

/// Why a locked append did not happen: which step, on which path.
#[derive(Debug)]
pub enum Error {
    /// The lock file could not be opened or the lock was not acquired within
    /// [`LOCK_WAIT`].
    Lock(String, io::Error),
    /// The append itself failed (and was rolled back where it could be).
    Append(String, io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lock(path, cause) => write!(
                out,
                "could not lock {path} within {}s: {cause}",
                LOCK_WAIT.as_secs()
            ),
            Self::Append(path, cause) => write!(out, "could not append to {path}: {cause}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<Error> for io::Error {
    fn from(why: Error) -> Self {
        let kind = match &why {
            Error::Lock(_, cause) | Error::Append(_, cause) => cause.kind(),
        };
        Self::new(kind, why.to_string())
    }
}

/// One session's files, addressed by its meta directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStore {
    dir: PathBuf,
}

/// The store for the session whose meta directory is `dir`. No IO.
#[must_use]
pub fn open(dir: &Path) -> SessionStore {
    SessionStore {
        dir: dir.to_owned(),
    }
}

impl SessionStore {
    /// The meta directory this store was opened on.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The event container's path.
    #[must_use]
    pub fn events_path(&self) -> PathBuf {
        self.dir.join(EVENTS)
    }

    /// The memo container's path.
    #[must_use]
    pub fn memo_path(&self) -> PathBuf {
        self.dir.join(MEMO)
    }

    /// The meta file's path.
    #[must_use]
    pub fn meta_path(&self) -> PathBuf {
        self.dir.join(META)
    }

    /// The meta file's lock, derived from its name like every other.
    #[must_use]
    pub fn meta_lock(&self) -> PathBuf {
        lock_path(&self.meta_path())
    }

    /// The session's goal — the FIRST `goal=` record in meta, which is what
    /// `ae_meta_get`'s `grep | head -1 | cut` reads and what the helper prints.
    ///
    /// `None` is a session nobody has given a goal, and so is a session with no
    /// meta at all: not having been asked is not a failure to look.
    ///
    /// # Errors
    ///
    /// A meta file that exists and could not be read — reported, never rendered
    /// as "no goal", because those are different answers.
    pub fn goal(&self) -> io::Result<Option<Vec<u8>>> {
        let text = match crate::meta::read_bytes(&self.dir) {
            Ok(text) => text,
            Err(why) if why.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(why) => return Err(why),
        };
        Ok(crate::meta::first_value(&text, crate::goal::KEY).map(<[u8]>::to_vec))
    }

    /// Whether the event container exists yet — the wait in `events-tail`,
    /// which exists because a fresh session has no container until its first
    /// event.
    #[must_use]
    pub fn has_container(&self) -> bool {
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: the lazily-created event container's existence test — see clippy.toml"
        )]
        let present = self.events_path().is_file();
        present
    }

    /// The event container's bytes, or none at all.
    ///
    /// The QUIET read, and deliberately not [`Self::memo_bytes`]'s louder one:
    /// anything that is not a readable regular file — absent, a directory, a
    /// FIFO, a regular file this process may not open — is no bytes and no
    /// complaint. That is the frozen `2>/dev/null` answer every event reader
    /// was built on, and a reader that started reporting it would turn a
    /// missing container into a failed `requests` table.
    #[must_use]
    pub fn container(&self) -> Vec<u8> {
        if !self.has_container() {
            return Vec::new();
        }
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: the opaque event-container read shared by every read surface — see clippy.toml"
        )]
        let body = std::fs::read(self.events_path());
        body.unwrap_or_default()
    }

    /// The memo container's bytes.
    ///
    /// The `[[ -f ]]` gate comes BEFORE the open and is the whole difference
    /// between the two quiet answers and the loud one: absent, a directory, a
    /// FIFO or a socket is no bytes at all, and is never opened — a FIFO opened
    /// without the gate blocks the reader for good. Only a REGULAR file that
    /// then cannot be read is an error.
    ///
    /// # Errors
    ///
    /// A regular memo file that exists and could not be read.
    pub fn memo_bytes(&self) -> io::Result<Vec<u8>> {
        let path = self.memo_path();
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: the `[[ -f \"$MEMO_FILE\" ]]` gate, before the memo file is opened — see clippy.toml"
        )]
        let regular = path.is_file();
        if !regular {
            return Ok(Vec::new());
        }
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: the memo file read behind `memo read` and `memo tail` — see clippy.toml"
        )]
        let bytes = std::fs::read(&path)?;
        Ok(bytes)
    }

    /// The memo container's bytes, or none at all — the QUIET read of the same
    /// file [`Self::memo_bytes`] reads loudly.
    ///
    /// The compaction handover watches the memo file's LENGTH across a wait, so
    /// a file it cannot read has to answer "no growth yet" and keep waiting; a
    /// mid-flight compaction must not fail on a transient read. The `memo`
    /// helper wants the opposite, because a memo file that exists and cannot be
    /// read is the one thing worth saying out loud rather than rendering as an
    /// empty session memory. Same file, two callers, two answers — spelled out
    /// here so the difference stays a decision.
    #[must_use]
    pub fn memo_bytes_or_empty(&self) -> Vec<u8> {
        self.memo_bytes().unwrap_or_default()
    }

    /// Append one event line to the container under its lock. This is the only
    /// append path for the event ledger.
    ///
    /// # Errors
    ///
    /// [`Error`] — which step failed, on which path.
    pub fn append_event(&self, line: &str) -> Result<(), Error> {
        append_locked(&self.events_path(), line.as_bytes())
    }

    /// Append one memo record to `memo.tsv` under its lock.
    ///
    /// # Errors
    ///
    /// [`Error`] — which step failed, on which path.
    pub fn append_memo(&self, record: &[u8]) -> Result<(), Error> {
        append_locked(&self.memo_path(), record)
    }

    /// Cap the event container to its newest `keep` lines on resume.
    ///
    /// The lock is held from the read through the staged sibling's rename, so
    /// an appender cannot land bytes between the snapshot and replacement. A
    /// failed lock, read, write or rename leaves the original container alone
    /// and is deliberately ignored: resume retention has always been a best
    /// effort step.
    pub fn retain_events(&self, keep: usize) {
        let path = self.events_path();
        let Ok(_held) = lock(&lock_path(&path), LOCK_WAIT) else {
            return;
        };
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: the resume-time event-log retention reads the log it is about to trim — see clippy.toml"
        )]
        let read = std::fs::read_to_string(&path);
        let Ok(text) = read else {
            return;
        };
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() <= keep {
            return;
        }
        let mut retained = String::new();
        for line in &lines[lines.len() - keep..] {
            retained.push_str(line);
            retained.push('\n');
        }
        let temp = self.events_trim_path();
        if std::fs::write(&temp, retained).is_ok() && std::fs::rename(&temp, &path).is_ok() {
            return;
        }
        let _ = std::fs::remove_file(&temp);
    }

    /// The staged sibling used by [`Self::retain_events`].
    fn events_trim_path(&self) -> PathBuf {
        self.dir
            .join(format!("{EVENTS}.trim.{}", std::process::id()))
    }
}

/// Append `bytes` to `path` under `<path>.lock`: the lock is the file's own
/// `.lock` sibling, taken with `flock -w 5` and held through the append.
///
/// PRIVATE on purpose. Every session-ledger append goes through one of the two
/// methods above; the explicit retention replacement is the only other
/// producer-owned mutation.
fn append_locked(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let lock_path = lock_path(path);
    let _held = lock(&lock_path, LOCK_WAIT)
        .map_err(|why| Error::Lock(lock_path.display().to_string(), why))?;
    append(path, bytes).map_err(|why| Error::Append(path.display().to_string(), why))
}

/// Take the exclusive advisory lock on `path`, retrying for up to `wait`.
///
/// # Errors
///
/// The lock file not being openable, or the lock still held at `wait`.
pub fn lock(path: &Path, wait: Duration) -> io::Result<File> {
    let file = OpenOptions::new().append(true).create(true).open(path)?;
    let started = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) => {
                if started.elapsed() >= wait {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "another writer holds the lock",
                    ));
                }
                std::thread::sleep(LOCK_POLL);
            }
            Err(TryLockError::Error(why)) => return Err(why),
        }
    }
}

/// Append `bytes` to `path`, creating it, as one transaction under the lock
/// the caller holds.
fn append(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().append(true).create(true).open(path)?;
    commit(&mut file, bytes)
}

/// What a transactional append needs from its container.
trait Sink {
    /// The current length — the point to roll back to.
    fn len(&mut self) -> io::Result<u64>;
    /// Write all of `bytes`, possibly failing after a prefix.
    fn put(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// Make what was written durable — `fdatasync`.
    fn sync(&mut self) -> io::Result<()>;
    /// Cut the container back to `len`.
    fn truncate(&mut self, len: u64) -> io::Result<()>;
}

impl Sink for File {
    fn len(&mut self) -> io::Result<u64> {
        self.metadata().map(|meta| meta.len())
    }
    fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.write_all(bytes)
    }
    fn sync(&mut self) -> io::Result<()> {
        self.sync_data()
    }
    fn truncate(&mut self, len: u64) -> io::Result<()> {
        self.set_len(len)
    }
}

/// Write `bytes` so that afterwards the container holds either all of them,
/// durably, or none of them.
fn commit(sink: &mut impl Sink, bytes: &[u8]) -> io::Result<()> {
    let before = sink.len()?;
    match sink.put(bytes).and_then(|()| sink.sync()) {
        Ok(()) => Ok(()),
        Err(failed) => match sink.truncate(before).and_then(|()| sink.sync()) {
            Ok(()) => Err(failed),
            Err(rollback) => Err(io::Error::new(
                rollback.kind(),
                format!(
                    "{failed}; and rolling the container back to {before} bytes failed: {rollback} \
                     — the container's state is UNKNOWN"
                ),
            )),
        },
    }
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "tests read back what the door wrote; the boundary is on product code — see clippy.toml"
)]
mod tests {
    use super::{EVENTS, LOCK_WAIT, MEMO, META, Sink, commit, lock, lock_path, open};
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    fn scratch(tag: &str) -> PathBuf {
        let dir = PathBuf::from(format!("/tmp/ae-store-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_store_is_the_one_place_the_file_names_are_spelled() {
        let store = open(Path::new("/sessions/demo"));
        assert_eq!(
            store.events_path(),
            Path::new("/sessions/demo/events.jsonl")
        );
        assert_eq!(store.memo_path(), Path::new("/sessions/demo/memo.tsv"));
        assert_eq!(
            lock_path(&store.events_path()),
            Path::new("/sessions/demo/events.jsonl.lock"),
            "the lock is the file's own name plus .lock — the spelling bash took"
        );
        assert_eq!(
            lock_path(&store.memo_path()).file_name().unwrap(),
            "memo.tsv.lock"
        );
        // The data file and the lock beside it derive from ONE name, so
        // neither can move without the other. Equal strings would not say that:
        // the lock is asserted to BE the derived path, not to look like it.
        assert_eq!(store.meta_path(), Path::new("/sessions/demo/meta"));
        assert_eq!(store.meta_lock(), lock_path(&store.meta_path()));
        assert_eq!(store.meta_lock().file_name().unwrap(), "meta.lock");
        assert_eq!(crate::meta::FILE, META, "one spelling, re-exported");
        assert_eq!((EVENTS, MEMO, META), ("events.jsonl", "memo.tsv", "meta"));
    }

    #[test]
    fn both_appends_go_through_one_locked_transaction() {
        let dir = scratch("append");
        let store = open(&dir);
        store
            .append_event("{\"ts\":\"t1\",\"action\":\"state\"}\n")
            .unwrap();
        store
            .append_event("{\"ts\":\"t2\",\"action\":\"state\"}\n")
            .unwrap();
        store.append_memo(b"t1\tcl:lead\tgeneral\tnote\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(store.events_path())
                .unwrap()
                .lines()
                .count(),
            2,
            "the second event APPENDS; the first is untouched"
        );
        assert_eq!(
            std::fs::read_to_string(store.memo_path()).unwrap(),
            "t1\tcl:lead\tgeneral\tnote\n"
        );
        for path in [store.events_path(), store.memo_path()] {
            assert!(lock_path(&path).exists(), "each file took its own lock");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_memo_gate_answers_quietly_for_anything_that_is_not_a_regular_file() {
        let dir = scratch("memo");
        let store = open(&dir);
        assert_eq!(store.memo_bytes().unwrap(), b"", "no memo file yet");
        std::fs::write(store.memo_path(), b"t1\tcl:lead\tgeneral\tnote\n").unwrap();
        assert_eq!(
            store.memo_bytes().unwrap(),
            b"t1\tcl:lead\tgeneral\tnote\n",
            "a regular file is read whole"
        );
        // Anything that is not a regular file is the empty answer, never
        // opened: a directory, a socket (bound here from safe std — the FIFO
        // that would BLOCK an ungated open needs mkfifo and is covered
        // black-box).
        std::fs::remove_file(store.memo_path()).unwrap();
        std::fs::create_dir_all(store.memo_path()).unwrap();
        assert_eq!(store.memo_bytes().unwrap(), b"", "a directory");
        std::fs::remove_dir_all(store.memo_path()).unwrap();
        let socket = std::os::unix::net::UnixListener::bind(store.memo_path()).unwrap();
        assert_eq!(store.memo_bytes().unwrap(), b"", "a socket");
        drop(socket);
        std::fs::remove_file(store.memo_path()).unwrap();
        // A REGULAR file that cannot be read is the one reported case.
        std::fs::write(store.memo_path(), b"x").unwrap();
        std::fs::set_permissions(store.memo_path(), std::fs::Permissions::from_mode(0o000))
            .unwrap();
        if std::fs::read(store.memo_path()).is_err() {
            assert!(
                store.memo_bytes().is_err(),
                "a regular memo file that exists but cannot be read is reported"
            );
        }
        std::fs::set_permissions(store.memo_path(), std::fs::Permissions::from_mode(0o644))
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_held_lock_is_refused_at_the_bound_with_no_bytes_written() {
        let dir = scratch("held");
        let store = open(&dir);
        let held = lock_path(&store.events_path());
        // Another open file description holding the same flock.
        let holder = lock(&held, Duration::from_millis(10)).unwrap();
        let started = std::time::Instant::now();
        let waited = lock(&held, Duration::from_millis(150));
        assert!(waited.is_err(), "the lock is held");
        assert!(
            started.elapsed() >= Duration::from_millis(150),
            "the bound was honoured"
        );
        drop(holder);
        assert!(lock(&held, Duration::from_millis(10)).is_ok(), "released");
        // The real path uses the real bound: 5s, per flock -w 5.
        assert_eq!(LOCK_WAIT, Duration::from_secs(5));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A container that fails on demand: after `fail_after` bytes of a put, or
    /// at sync, or at truncate.
    #[derive(Default)]
    struct Flaky {
        held: Vec<u8>,
        fail_put_after: Option<usize>,
        /// Per sync call, in order: `true` fails that call.
        fail_syncs: Vec<bool>,
        /// How many syncs were asked for.
        syncs: usize,
        fail_truncate: bool,
        truncated_to: Vec<u64>,
    }

    impl Sink for Flaky {
        fn len(&mut self) -> std::io::Result<u64> {
            Ok(self.held.len() as u64)
        }
        fn put(&mut self, bytes: &[u8]) -> std::io::Result<()> {
            if let Some(prefix) = self.fail_put_after {
                self.held
                    .extend_from_slice(&bytes[..prefix.min(bytes.len())]);
                return Err(std::io::Error::other("disk full after a prefix"));
            }
            self.held.extend_from_slice(bytes);
            Ok(())
        }
        fn sync(&mut self) -> std::io::Result<()> {
            let call = self.syncs;
            self.syncs += 1;
            if self.fail_syncs.get(call).copied().unwrap_or(false) {
                return Err(std::io::Error::other(format!("sync {} failed", call + 1)));
            }
            Ok(())
        }
        fn truncate(&mut self, len: u64) -> std::io::Result<()> {
            self.truncated_to.push(len);
            if self.fail_truncate {
                return Err(std::io::Error::other("truncate failed"));
            }
            self.held.truncate(usize::try_from(len).unwrap());
            Ok(())
        }
    }

    #[test]
    fn a_write_that_fails_after_a_prefix_rolls_the_container_back() {
        let mut sink = Flaky {
            held: b"{\"ts\":\"earlier\"}\n".to_vec(),
            fail_put_after: Some(7),
            ..Flaky::default()
        };
        let before = sink.held.clone();
        let result = commit(&mut sink, b"{\"ts\":\"now\",\"action\":\"state\"}\n");
        assert!(result.is_err());
        assert_eq!(
            sink.held, before,
            "not one byte of the failed record survives"
        );
        assert_eq!(sink.truncated_to, vec![before.len() as u64]);
    }

    #[test]
    fn a_sync_that_fails_after_a_complete_write_rolls_the_container_back_too() {
        // The subtler arm: the bytes are all there, the caller is told "not
        // recorded", and without the rollback the next reader would find a
        // state nobody acknowledged.
        let mut sink = Flaky {
            fail_syncs: vec![true],
            ..Flaky::default()
        };
        let result = commit(&mut sink, b"{\"action\":\"state\"}\n");
        assert_eq!(
            result.unwrap_err().to_string(),
            "sync 1 failed",
            "the write's error"
        );
        assert!(sink.held.is_empty());
        assert_eq!(sink.truncated_to, vec![0]);
        assert_eq!(sink.syncs, 2, "the rollback was synced too");
    }

    #[test]
    fn a_rollback_whose_own_sync_fails_is_reported_as_an_unknown_state() {
        // The body reached durable storage, the sync after it failed, the cut
        // back succeeded in the page cache — and THAT sync failed.
        let mut sink = Flaky {
            fail_syncs: vec![true, true],
            ..Flaky::default()
        };
        let why = commit(&mut sink, b"{\"action\":\"state\"}\n")
            .unwrap_err()
            .to_string();
        assert!(why.contains("sync 1 failed"), "{why}");
        assert!(
            why.contains("rolling the container back to 0 bytes failed: sync 2 failed"),
            "{why}"
        );
        assert!(why.contains("UNKNOWN"), "{why}");
        assert_eq!(sink.syncs, 2);
    }

    #[test]
    fn a_rollback_that_fails_is_what_gets_reported() {
        let mut sink = Flaky {
            fail_put_after: Some(2),
            fail_truncate: true,
            ..Flaky::default()
        };
        let why = commit(&mut sink, b"abcdef").unwrap_err().to_string();
        assert!(why.contains("disk full after a prefix"), "{why}");
        assert!(
            why.contains("rolling the container back to 0 bytes failed"),
            "{why}"
        );
        assert!(why.contains("UNKNOWN"), "{why}");
        assert_eq!(sink.syncs, 0, "a failed truncate is not followed by a sync");
    }

    #[test]
    fn a_successful_commit_never_truncates() {
        let mut sink = Flaky::default();
        commit(&mut sink, b"line\n").unwrap();
        assert_eq!(sink.held, b"line\n");
        assert!(sink.truncated_to.is_empty());
        assert_eq!(sink.syncs, 1, "one sync, for the write");
    }
}
