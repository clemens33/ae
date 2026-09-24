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
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The event container's file name, kept across every flip.
pub const EVENTS: &str = "events.jsonl";

/// The memo container.
pub const MEMO: &str = "memo.tsv";

/// The session's own record — roster, mode, origin, goal.
pub const META: &str = "meta";

/// The LAUNCH-ATTEMPT stamp: when ae last tried to put this session on a tmux
/// server.
///
/// Written BEFORE the create it is about, under the lifecycle lock, by every
/// launch, resume and spawn — so an attempt that then died between the create
/// and the meta publication has still left the moment behind. That is what
/// makes it the evidence [`crate::tmux::classify_absence`] can weigh against
/// the host's boot time: nothing else ae writes is both universal (every tool,
/// every seat) and earlier than the tmux session it describes.
pub const LAUNCH_ATTEMPT: &str = ".launch-attempt";

/// The most a launch-attempt stamp may be: one epoch and a newline. A larger
/// file at that name is damage, and reading it would let whoever planted it
/// size an allocation on the resume and listing paths.
pub const LAUNCH_ATTEMPT_CAP: u64 = 64;

/// What a launch-attempt stamp's BYTES say — the pure half of
/// [`SessionStore::launch_attempt`], and the one the fuzz lane drives.
///
/// The stamp is MANDATORY and carries no sentinel: it is written only when a
/// launch is about to happen, so the only thing it may say is a strictly
/// positive moment. A present stamp spelling anything else is damage.
///
/// ```
/// use ae::store::parse_launch_attempt;
/// use ae::tmux::Evidence;
/// assert_eq!(parse_launch_attempt(b"1789105855\n"), Evidence::At(1_789_105_855));
/// assert_eq!(parse_launch_attempt(b""), Evidence::Unreadable);
/// assert_eq!(parse_launch_attempt(b"\xff\xfe"), Evidence::Unreadable);
/// assert_eq!(parse_launch_attempt(b"0\n"), Evidence::Unreadable);
/// assert_eq!(parse_launch_attempt(b"-1\n"), Evidence::Unreadable);
/// ```
#[must_use]
pub fn parse_launch_attempt(body: &[u8]) -> crate::tmux::Evidence {
    use crate::tmux::Evidence;
    if body.len() as u64 > LAUNCH_ATTEMPT_CAP {
        return Evidence::Unreadable;
    }
    match std::str::from_utf8(body) {
        Ok(text) => Evidence::claim(text),
        Err(_) => Evidence::Unreadable,
    }
}

/// What a file's lock is called: its own name plus this. Appending it by hand
/// is how two writers end up on two different locks.
pub const LOCK_SUFFIX: &str = ".lock";

/// How long a locked append waits for the lock.
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

/// One session file's classification — four states, never a bool.
///
/// Mirrors `tmux::Evidence` and `session::MetaRead`: a node that is not there,
/// a node that is there and readable, a node OBSERVED in a shape this reader
/// refuses before opening, and a regular node that exists and could not be
/// read. The difference between the last two is the whole reason this is not a
/// bool: an unreadable container is damage, and rendering it as an empty one is
/// the failure this type exists to prevent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceRead {
    /// Positive `NotFound`: nothing is at this path.
    Absent,
    /// A regular file, read whole.
    Ready(Vec<u8>),
    /// A node OBSERVED non-regular — directory, FIFO, socket, symlink, device.
    /// Rejected before any open: never followed, never blocking. Honest
    /// residual: a concurrent replacement between this observation and a read
    /// is not atomic, so the claim is "observed non-regular rejected before
    /// open", never atomicity.
    Invalid(String),
    /// A regular file that exists and could not be read.
    Unreadable(String),
}

/// Which non-regular leg a node's OWN file type is, in the one spelling every
/// gate in this module refuses by. `None` is returned EXACTLY for a regular
/// file — the only shape these gates open, because a FIFO write-open BLOCKS
/// until a reader appears and an open with `create(true)` FOLLOWS a symlink.
/// The proceed arm is the positive `is_file` test, never the FALLTHROUGH: a
/// node whose kind nobody enumerated is refused as "an unrecognized node"
/// rather than opened, because a fallback that proceeds is how a node no gate
/// classified still gets opened.
fn nonregular_leg(kind: std::fs::FileType) -> Option<&'static str> {
    use std::os::unix::fs::FileTypeExt as _;
    if kind.is_file() {
        None
    } else if kind.is_symlink() {
        Some("a symlink")
    } else if kind.is_dir() {
        Some("a directory")
    } else if kind.is_fifo() {
        Some("a fifo")
    } else if kind.is_socket() {
        Some("a socket")
    } else if kind.is_block_device() || kind.is_char_device() {
        Some("a device")
    } else {
        Some("an unrecognized node")
    }
}

/// Classify the node at `path` without ever opening a non-regular one.
#[must_use]
pub fn read_source(path: &Path) -> SourceRead {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: classifies the node itself WITHOUT following a link, before any open — see clippy.toml"
    )]
    let observed = std::fs::symlink_metadata(path);
    let shape = match observed {
        Ok(meta) => meta,
        Err(why) if why.kind() == io::ErrorKind::NotFound => return SourceRead::Absent,
        Err(why) => return SourceRead::Unreadable(why.to_string()),
    };
    if let Some(what) = nonregular_leg(shape.file_type()) {
        return SourceRead::Invalid(what.to_owned());
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the classified read of a session source a human asked about — see clippy.toml"
    )]
    let read = std::fs::read(path);
    match read {
        Ok(bytes) => SourceRead::Ready(bytes),
        Err(why) if why.kind() == io::ErrorKind::NotFound => SourceRead::Absent,
        Err(why) => SourceRead::Unreadable(why.to_string()),
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

    /// The launch-attempt stamp's path.
    #[must_use]
    pub fn launch_attempt_path(&self) -> PathBuf {
        self.dir.join(LAUNCH_ATTEMPT)
    }

    /// The staged sibling [`Self::stamp_launch_attempt`] publishes from.
    fn launch_attempt_temp(&self) -> PathBuf {
        self.dir
            .join(format!("{LAUNCH_ATTEMPT}.tmp.{}", std::process::id()))
    }

    /// Record `epoch` as this session's newest launch attempt, DURABLY.
    ///
    /// Temp, `fsync`, rename — the shape [`crate::run`]'s start marker uses,
    /// for the same reason: the caller is about to do something it cannot take
    /// back, and a stamp that is still in a page cache when the machine stops
    /// is a stamp that was never written. A caller that cannot write this must
    /// REFUSE to create the tmux session, so the error is returned rather than
    /// swallowed.
    ///
    /// # Errors
    ///
    /// A temp name that is already taken — which is never overwritten, because
    /// what is behind it may not be this session's — or the write, the `fsync`
    /// or the rename, whichever failed.
    pub fn stamp_launch_attempt(&self, epoch: i64) -> io::Result<()> {
        let path = self.launch_attempt_path();
        let temp = self.launch_attempt_temp();
        // EXCLUSIVE, and that is not a detail. The name is predictable and it
        // sits in session state a human edits, so `File::create` would FOLLOW a
        // link planted there and truncate whatever it points at — a file
        // outside this session entirely. `create_new` refuses a name that is
        // taken, whatever is behind it, and refusing is the right answer: a
        // stamp that cannot be written already means the launch is refused.
        let created = OpenOptions::new().write(true).create_new(true).open(&temp);
        let mut file = match created {
            Ok(file) => file,
            Err(why) => {
                // NOT ours, so NOT ours to remove. Say where it is instead.
                return Err(io::Error::new(
                    why.kind(),
                    format!(
                        "{}: {why} — nothing was overwritten; remove that file if it is stale",
                        temp.display()
                    ),
                ));
            }
        };
        // From here the temp is one this process made, which is what makes the
        // cleanup below safe.
        let publish = file
            .write_all(format!("{epoch}\n").as_bytes())
            .and_then(|()| file.sync_all())
            .and_then(|()| std::fs::rename(&temp, &path));
        if let Err(why) = publish {
            let _ = std::fs::remove_file(&temp);
            return Err(why);
        }
        Ok(())
    }

    /// When ae last tried to launch into this session.
    ///
    /// The epoch is the file's CONTENT — the moment ae deliberately recorded,
    /// which no copy, restore or archive rewrites. Three answers, and the
    /// difference between the last two is the whole reason this is not an
    /// `Option`: a stamp that is NOT THERE is a session older than the stamp
    /// and says nothing, while a stamp that is there and unreadable is DAMAGE
    /// and must refuse the proof rather than step aside for an older row.
    ///
    /// Hostile persisted state, so the read is BOUNDED before it happens: the
    /// file holds one epoch and a newline, and anything larger is damage rather
    /// than an allocation every resume and every listing has to pay for.
    #[must_use]
    pub fn launch_attempt(&self) -> crate::tmux::Evidence {
        use crate::tmux::Evidence;
        let path = self.launch_attempt_path();
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: the launch-attempt stamp is classified without following a link to it — see `LAUNCH_ATTEMPT`"
        )]
        let probe = std::fs::symlink_metadata(&path);
        let meta = match probe {
            Ok(meta) => meta,
            // The ONE benign absence.
            Err(why) if why.kind() == io::ErrorKind::NotFound => return Evidence::Silent,
            Err(_) => return Evidence::Unreadable,
        };
        if !meta.is_file() || meta.len() > LAUNCH_ATTEMPT_CAP {
            return Evidence::Unreadable;
        }
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: the stamp's own epoch is the fact ae wrote — see `LAUNCH_ATTEMPT`"
        )]
        let opened = File::open(&path);
        let Ok(file) = opened else {
            return Evidence::Unreadable;
        };
        let mut body = Vec::new();
        // The cap AGAIN, on the read itself: the size above was a different
        // moment, and this one is what actually allocates.
        if io::Read::read_to_end(&mut io::Read::take(file, LAUNCH_ATTEMPT_CAP + 1), &mut body)
            .is_err()
        {
            return Evidence::Unreadable;
        }
        parse_launch_attempt(&body)
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

    /// The event container classified for a human read — the noisy variant
    /// [`Self::container`] deliberately is not.
    #[must_use]
    pub fn events_source(&self) -> SourceRead {
        read_source(&self.events_path())
    }

    /// The memo container classified for a human read.
    #[must_use]
    pub fn memo_source(&self) -> SourceRead {
        read_source(&self.memo_path())
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

/// `O_NONBLOCK | O_NOFOLLOW` for the stamp hold's open, spelled per target
/// because ae carries no `libc`: a FIFO swapped in is opened without waiting
/// for a writer and then refused, a link is refused instead of followed.
/// macOS aarch64: SDK `sys/fcntl.h` `0x0004`, `0x0100`. Linux `x86_64`, glibc and
/// musl alike: `asm-generic/fcntl.h` `0o4000`, `0o400000`.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const HOLD_FLAGS: Option<i32> = Some(0x0004 | 0x0100);
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const HOLD_FLAGS: Option<i32> = Some(0o4000 | 0o400_000);
/// Any other target: no spelled pair, so the hold is unavailable there.
#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "x86_64")
)))]
const HOLD_FLAGS: Option<i32> = None;

/// Why the launch stamp could not be held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StampGap {
    /// This target has no spelled flag pair.
    Unavailable,
    /// No stamp at the initial `lstat`: nothing to hold. It proves nothing about
    /// the session.
    Absent,
    /// Not a regular file, unreadable, replaced before the open, or not a
    /// positive epoch.
    Unreadable,
}

/// The launch stamp's node as `lstat` saw it: a regular file, by identity.
#[derive(Debug)]
pub struct StampNode {
    path: PathBuf,
    dev: u64,
    ino: u64,
}

/// A held launch stamp. The open file keeps its inode from being reused while
/// the hold lives, so [`StampHold::matches`] can compare identities.
#[derive(Debug)]
pub struct StampHold {
    node: StampNode,
    _file: File,
}

impl SessionStore {
    /// Step 1 of the stamp hold: `lstat` the stamp and keep its identity.
    ///
    /// # Errors
    ///
    /// [`StampGap::Absent`] when the lstat finds no stamp; [`StampGap::Unreadable`]
    /// when it is unreadable or not a regular file.
    pub fn stamp_node(&self) -> Result<StampNode, StampGap> {
        let path = self.launch_attempt_path();
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: the stamp's own node, never followed — see `LAUNCH_ATTEMPT`"
        )]
        let probe = std::fs::symlink_metadata(&path);
        match probe {
            Ok(meta) if meta.is_file() => Ok(StampNode {
                dev: meta.dev(),
                ino: meta.ino(),
                path,
            }),
            // The ONE benign absence, as `launch_attempt` reads it.
            Err(why) if why.kind() == io::ErrorKind::NotFound => Err(StampGap::Absent),
            _ => Err(StampGap::Unreadable),
        }
    }
}

impl StampNode {
    /// Step 2: open the SAME node without following a link or waiting on a
    /// FIFO, prove it is still that regular file, and read a positive epoch.
    ///
    /// # Errors
    ///
    /// [`StampGap::Unavailable`] on a target with no flag pair; otherwise
    /// [`StampGap::Unreadable`]: the open refused, a replacement, or bytes
    /// that are not a positive epoch.
    pub fn open(self) -> Result<StampHold, StampGap> {
        let flags = HOLD_FLAGS.ok_or(StampGap::Unavailable)?;
        let opened = OpenOptions::new()
            .read(true)
            .custom_flags(flags)
            .open(&self.path);
        let mut file = opened.map_err(|_| StampGap::Unreadable)?;
        let meta = file.metadata().map_err(|_| StampGap::Unreadable)?;
        if !meta.is_file() || (meta.dev(), meta.ino()) != (self.dev, self.ino) {
            return Err(StampGap::Unreadable);
        }
        let mut body = Vec::new();
        io::Read::read_to_end(
            &mut io::Read::take(&mut file, LAUNCH_ATTEMPT_CAP + 1),
            &mut body,
        )
        .map_err(|_| StampGap::Unreadable)?;
        match parse_launch_attempt(&body) {
            crate::tmux::Evidence::At(_) => Ok(StampHold {
                node: self,
                _file: file,
            }),
            _ => Err(StampGap::Unreadable),
        }
    }
}

impl StampHold {
    /// Whether the stamp path still names the held regular file. `lstat`, so a
    /// link swapped in over it does not match, and a restamp is a new inode.
    #[must_use]
    pub fn matches(&self) -> bool {
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: the stamp's own node, never followed — see `LAUNCH_ATTEMPT`"
        )]
        let probe = std::fs::symlink_metadata(&self.node.path);
        probe.is_ok_and(|meta| {
            meta.is_file() && (meta.dev(), meta.ino()) == (self.node.dev, self.node.ino)
        })
    }
}

/// Append `bytes` to `path` under `<path>.lock`: the lock is the file's own
/// `.lock` sibling, taken as the exclusive advisory lock bounded by
/// [`LOCK_WAIT`] and held through the append.
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

/// Refuse a lock path whose OWN node is not absent or a regular file, BEFORE
/// the open — the one gate every [`lock`] caller inherits.
///
/// `open(2)` is where both defects live: a write-open on a FIFO BLOCKS until a
/// reader appears, so no caller's `wait` would ever be reached, and
/// `create(true).append(true)` FOLLOWS a symlink, taking the lock wherever the
/// link points — and creating the target when the link dangles. [`lock`] never
/// writes a byte to that File, so "appended through" would overstate the
/// damage; "opened and locked through" is what the gate prevents. The classification is `symlink_metadata`, so the
/// node's own shape is read without following it; absent is the create case the
/// caller asked for and proceeds, a regular file proceeds, every other leg is
/// refused by name.
///
/// The TOCTOU residual is REAL and named, never papered over: a node can be
/// swapped between this observation and the open below, so the check is not
/// atomic. ae's threat model is cooperative agents on one host, and a lock path
/// lives under ae's own state directory — whoever can win that race can already
/// write that state directly. The atomic form is `OpenOptionsExt::custom_flags`
/// with `O_NOFOLLOW` and `O_NONBLOCK`; it needs the raw platform flag
/// integers, which differ between macOS and Linux and which ae, carrying no
/// `libc`, cannot assert at compile time. `HOLD_FLAGS` spells that pair for the
/// stamp hold; reusing it here is the upgrade path if the threat model changes.
fn refuse_nonregular_lock_path(path: &Path) -> io::Result<()> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: classifies the lock node itself WITHOUT following a link, before any open — see clippy.toml"
    )]
    let observed = std::fs::symlink_metadata(path);
    let kind = match observed {
        Ok(meta) => meta.file_type(),
        Err(why) if why.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(why) => return Err(why),
    };
    let Some(what) = nonregular_leg(kind) else {
        return Ok(());
    };
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "{}: the lock path is {what} — refused before any open",
            path.display()
        ),
    ))
}

/// Take the exclusive advisory lock on `path`, retrying for up to `wait`.
///
/// The path's own node is classified before it is opened; anything but an
/// absent or regular node is refused by name. `wait` bounds only the poll for a
/// lock another writer holds — a refused path never reaches the open, let alone
/// the poll.
///
/// # Errors
///
/// The lock path being anything but absent or a regular file, the lock file not
/// being openable, or the lock still held at `wait`.
pub fn lock(path: &Path, wait: Duration) -> io::Result<File> {
    refuse_nonregular_lock_path(path)?;
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
    use super::{EVENTS, LOCK_WAIT, MEMO, META, Sink, StampGap, commit, lock, lock_path, open};
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
    fn a_stamp_never_writes_through_a_name_it_did_not_create() {
        // I-1. The temp name is PREDICTABLE, and it sits in session state a
        // human edits. `File::create` would follow a link planted there and
        // truncate whatever it points at — a file outside this session — and
        // the error cleanup would then unlink a name this process never owned.
        let dir = scratch("exclusive");
        let store = open(&dir);
        let victim = dir.join("someone-elses-file");
        let bytes = b"THIS MUST SURVIVE\n";

        for (shape, plant) in [
            ("a regular file", false),
            ("a symlink to a file outside the session", true),
        ] {
            std::fs::write(&victim, bytes).unwrap();
            let temp = store.launch_attempt_temp();
            let _ = std::fs::remove_file(&temp);
            let _ = std::fs::remove_file(store.launch_attempt_path());
            if plant {
                std::os::unix::fs::symlink(&victim, &temp).unwrap();
            } else {
                std::fs::write(&temp, bytes).unwrap();
            }

            let refused = store.stamp_launch_attempt(1_789_105_855);
            assert!(refused.is_err(), "{shape}: the stamp must refuse");
            assert_eq!(
                std::fs::read(&victim).unwrap(),
                bytes,
                "{shape}: the foreign target was written through"
            );
            assert!(
                !store.launch_attempt_path().exists(),
                "{shape}: a refused stamp published itself anyway"
            );
            assert!(
                std::fs::symlink_metadata(&temp).is_ok(),
                "{shape}: the collision was removed, and it was never ours to remove"
            );
            if !plant {
                // The collision is the victim here: a regular file already at
                // the temp name is someone else's too, and refusing it is only
                // half the promise if its bytes moved.
                assert_eq!(
                    std::fs::read(&temp).unwrap(),
                    bytes,
                    "{shape}: the file already at the temp name was written through"
                );
            }
            let _ = std::fs::remove_file(&temp);
        }
        let _ = std::fs::remove_dir_all(&dir);
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

    /// A bool could not carry this: a node that is NOT THERE and a node that
    /// exists but is a directory, a socket or a link must read differently, and
    /// none of the non-regular shapes may ever be opened — a FIFO open without
    /// a gate blocks the reader for good.
    #[test]
    fn a_nonregular_events_node_is_invalid_not_absent_and_is_never_opened() {
        use super::SourceRead;
        let dir = scratch("source");
        let store = open(&dir);
        let events = store.events_path();
        assert_eq!(store.events_source(), SourceRead::Absent, "no node yet");
        std::fs::write(&events, b"{\"a\":1}\n").unwrap();
        assert_eq!(
            store.events_source(),
            SourceRead::Ready(b"{\"a\":1}\n".to_vec()),
            "a regular file is read whole"
        );
        std::fs::remove_file(&events).unwrap();
        std::fs::create_dir_all(&events).unwrap();
        assert!(
            matches!(store.events_source(), SourceRead::Invalid(reason) if reason.contains("directory")),
            "a directory is rejected before any open"
        );
        std::fs::remove_dir_all(&events).unwrap();
        let socket = std::os::unix::net::UnixListener::bind(&events).unwrap();
        assert!(
            matches!(store.events_source(), SourceRead::Invalid(reason) if reason.contains("socket")),
            "a socket is rejected before any open"
        );
        drop(socket);
        std::fs::remove_file(&events).unwrap();
        // A SYMLINK, even when it points at a readable regular file, is
        // classified by its own node: nothing is followed.
        let target = dir.join("outside");
        std::fs::write(&target, b"secret\n").unwrap();
        std::os::unix::fs::symlink(&target, &events).unwrap();
        assert!(
            matches!(store.events_source(), SourceRead::Invalid(reason) if reason.contains("symlink")),
            "a symlink is never followed"
        );
        std::fs::remove_file(&events).unwrap();
        // A REGULAR file that exists and cannot be read is the one reported case.
        std::fs::write(&events, b"x").unwrap();
        std::fs::set_permissions(&events, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&events).is_err() {
            assert!(
                matches!(store.events_source(), SourceRead::Unreadable(_)),
                "a regular file that cannot be read is UNREADABLE, not absent"
            );
        }
        std::fs::set_permissions(&events, std::fs::Permissions::from_mode(0o644)).unwrap();
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
        // The real path uses the real bound: 5s.
        assert_eq!(LOCK_WAIT, Duration::from_secs(5));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The lock path's OWN node decides, before any open: a link is refused and
    /// its target is never created or locked through, a directory, a socket
    /// and a DEVICE are refused by name, and absent and regular are the two
    /// shapes that proceed.
    ///
    /// The refusals are asserted by the WHOLE message — the leg, the path and
    /// "refused before any open" — because a bare substring like "directory"
    /// also matches the kernel's own `EISDIR`, which is the very answer this
    /// gate exists to pre-empt. A `contains` there would stay green with the
    /// gate deleted.
    #[test]
    fn a_nonregular_lock_path_is_refused_by_name_before_any_open() {
        let dir = scratch("lockpath");
        let refuse = |path: &Path, leg: &str| {
            let refused = lock(path, Duration::ZERO)
                .expect_err(&format!("a {leg} lock path must refuse before any open"));
            assert_eq!(
                refused.to_string(),
                format!(
                    "{}: the lock path is {leg} — refused before any open",
                    path.display()
                ),
                "the refusal names the leg and the path, and comes from the gate"
            );
        };

        let link_target = dir.join("outside");
        std::fs::write(&link_target, b"THIS MUST SURVIVE\n").unwrap();
        let link = dir.join("existing.lock");
        std::os::unix::fs::symlink(&link_target, &link).unwrap();
        refuse(&link, "a symlink");
        assert_eq!(
            std::fs::read(&link_target).unwrap(),
            b"THIS MUST SURVIVE\n",
            "the gate was defeated: the lock was taken through the symlink"
        );

        let absent_target = dir.join("never-created");
        let dangling = dir.join("dangling.lock");
        std::os::unix::fs::symlink(&absent_target, &dangling).unwrap();
        refuse(&dangling, "a symlink");
        assert!(
            !absent_target.exists(),
            "the gate was defeated: the link target was created through the link"
        );

        let as_dir = dir.join("dir.lock");
        std::fs::create_dir_all(&as_dir).unwrap();
        refuse(&as_dir, "a directory");

        let as_socket = dir.join("socket.lock");
        let listener = std::os::unix::net::UnixListener::bind(&as_socket).unwrap();
        refuse(&as_socket, "a socket");
        drop(listener);

        // A CHARACTER DEVICE every host this suite runs on carries. Its open
        // with `create(true).append(true)` SUCCEEDS, so the gate is the only
        // thing standing between a lock path and writing into a device node.
        refuse(Path::new("/dev/null"), "a device");

        let fresh = dir.join("fresh.lock");
        assert!(!fresh.exists(), "the fixture starts absent");
        let held = lock(&fresh, Duration::ZERO).expect("an absent lock path is created and locked");
        assert!(fresh.is_file(), "the lock file was created");
        drop(held);
        let held = lock(&fresh, Duration::ZERO).expect("a regular lock file locks again");
        drop(held);

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

    #[test]
    fn the_stamp_hold_refuses_what_its_lstat_did_not_see() {
        let dir = scratch("hold-open");
        let store = open(&dir);
        let path = store.launch_attempt_path();
        let aside = dir.join("aside");
        // A regular replacement with the SAME bytes: identity, not content.
        store.stamp_launch_attempt(1_789_105_855).unwrap();
        let node = store.stamp_node().unwrap();
        store.stamp_launch_attempt(1_789_105_855).unwrap();
        assert_eq!(
            node.open().err(),
            Some(StampGap::Unreadable),
            "a replacement"
        );
        // A link to the SAME inode: only O_NOFOLLOW refuses it.
        let node = store.stamp_node().unwrap();
        std::fs::rename(&path, &aside).unwrap();
        std::os::unix::fs::symlink(&aside, &path).unwrap();
        assert_eq!(node.open().err(), Some(StampGap::Unreadable), "a link");
        std::fs::remove_file(&path).unwrap();
        // No stamp at the lstat is nothing to hold; a link or a directory is no
        // node at all.
        assert_eq!(store.stamp_node().err(), Some(StampGap::Absent), "absent");
        std::os::unix::fs::symlink(&aside, &path).unwrap();
        assert_eq!(
            store.stamp_node().err(),
            Some(StampGap::Unreadable),
            "a link"
        );
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert_eq!(
            store.stamp_node().err(),
            Some(StampGap::Unreadable),
            "a directory"
        );
        std::fs::remove_dir(&path).unwrap();
        // Present at the lstat, gone by the open: damage, never an absence.
        store.stamp_launch_attempt(1_789_105_855).unwrap();
        let node = store.stamp_node().unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            node.open().err(),
            Some(StampGap::Unreadable),
            "gone by the open"
        );
        // A parent that is not a directory: damage, never an absence.
        let file = dir.join("not-a-dir");
        std::fs::write(&file, b"").unwrap();
        assert_eq!(
            open(&file).stamp_node().err(),
            Some(StampGap::Unreadable),
            "ENOTDIR"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_stamp_hold_takes_only_a_positive_epoch() {
        let dir = scratch("hold-bytes");
        let store = open(&dir);
        for body in [&b"0\n"[..], b"-5\n", &[b'1'; 65], b"\xff\xfe", b"soon\n"] {
            std::fs::write(store.launch_attempt_path(), body).unwrap();
            let node = store.stamp_node().unwrap();
            assert_eq!(node.open().err(), Some(StampGap::Unreadable), "{body:?}");
        }
        let padded = format!("{:<65}", 1_789_105_855);
        std::fs::write(store.launch_attempt_path(), &padded).unwrap();
        let node = store.stamp_node().unwrap();
        assert_eq!(node.open().err(), Some(StampGap::Unreadable), "{padded:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_held_stamp_matches_until_its_path_names_another_node() {
        let dir = scratch("hold-matches");
        let store = open(&dir);
        let path = store.launch_attempt_path();
        let aside = dir.join("aside");
        let acquire = |store: &super::SessionStore| {
            store.stamp_launch_attempt(1_789_105_855).unwrap();
            store.stamp_node().unwrap().open().unwrap()
        };
        let held = acquire(&store);
        for _ in 0..3 {
            assert!(held.matches(), "untouched");
        }
        store.stamp_launch_attempt(1_789_105_855).unwrap();
        assert!(!held.matches(), "a restamp with the same epoch");
        let held = acquire(&store);
        std::fs::rename(&path, &aside).unwrap();
        std::os::unix::fs::symlink(&aside, &path).unwrap();
        assert!(!held.matches(), "a link to the same inode");
        std::fs::remove_file(&path).unwrap();
        let held = acquire(&store);
        std::fs::remove_file(&path).unwrap();
        assert!(!held.matches(), "unlinked");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
