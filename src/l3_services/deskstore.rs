//! The board record in a session directory: read it and replace it.
//!
//! The pure grammar is [`crate::desk`]. This module owns the file
//! `desk-record` beside the session's other records. A reader never blocks and
//! never follows a link: the read goes through the capped, kind-checking read
//! of [`crate::store`], so a FIFO, a symlink, a directory or an oversize file
//! is refused by name. A set replaces the whole record under the
//! `desk-record.lock` writer lock, publishes it whole or not at all, and
//! journals exactly one `desk` event for a change: the ids it touched, never an
//! item's text. A set that changes nothing writes nothing and journals
//! nothing.

use std::fmt;
use std::io;
use std::path::Path;

use crate::desk::{self as record, Delta, Desk, Item, ParseError, RECORD_FILE, STORED_CAP};
use crate::relay::{Grant, Observed};
use crate::store::{self, SourceRead};
use crate::time::Timestamp;
use crate::tracked::{EventFields, event_line};

#[cfg(test)]
#[path = "desk/desk_admission_spec.rs"]
mod desk_admission_spec;
#[cfg(test)]
#[path = "desk/desk_empty_input_spec.rs"]
mod desk_empty_input_spec;
#[cfg(test)]
#[path = "desk/desk_store_repair_spec.rs"]
mod desk_store_repair_spec;
#[cfg(test)]
#[path = "desk/desk_store_spec.rs"]
mod desk_store_spec;
#[cfg(test)]
#[path = "desk/desk_sync_spec.rs"]
mod desk_sync_spec;
#[cfg(test)]
#[path = "desk/render_tests.rs"]
mod render_tests;
#[cfg(test)]
#[path = "desk/update_tests.rs"]
mod update_tests;

/// Why the record could not be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// Something other than a regular file stands at the record's path.
    NotRegular,
    /// The file is longer than the stored cap.
    TooLarge,
    /// The file exists and could not be read.
    Io(String),
    /// The file is no canonical record.
    Parse(ParseError),
}

impl fmt::Display for ReadError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRegular => out.write_str("not a regular file"),
            Self::TooLarge => write!(out, "larger than {STORED_CAP} bytes"),
            Self::Io(why) => write!(out, "unreadable: {why}"),
            Self::Parse(why) => write!(out, "{why}"),
        }
    }
}

/// The record in `dir`, or `None` when there is none. The file is opened only
/// when it is a regular file no longer than the stored cap.
///
/// # Errors
///
/// [`ReadError`]: a node of another kind, a file over the cap, a file that
/// could not be read, or text that is no canonical record. A damaged record
/// is never `None`: "no board" and "a damaged board" are different answers.
pub fn read(dir: &Path) -> Result<Option<Desk>, ReadError> {
    match store::read_capped_in(dir, RECORD_FILE, STORED_CAP as u64) {
        Err(store::Oversized) => Err(ReadError::TooLarge),
        Ok(SourceRead::Absent) => Ok(None),
        Ok(SourceRead::Invalid(_)) => Err(ReadError::NotRegular),
        Ok(SourceRead::Unreadable(why)) => Err(ReadError::Io(why)),
        Ok(SourceRead::Ready(bytes)) => record::parse(&bytes).map(Some).map_err(ReadError::Parse),
    }
}

/// What a set did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetOutcome {
    /// The new items equal the stored ones: nothing was written or journaled.
    Unchanged,
    /// The record was replaced.
    Changed {
        /// What differs from the record it replaced.
        delta: Delta,
        /// The items the new record holds.
        total: usize,
    },
}

/// Why a set did not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetError {
    /// The writer lock could not be taken.
    Lock(String),
    /// What stands at the record's path cannot be replaced: not a regular
    /// file, or unreadable. A damaged or oversize regular file is replaced.
    Blocked(ReadError),
    /// The new record was not published: the old one stands.
    Write(String),
    /// The record is published, and its `desk` event is not.
    Event(String),
    /// The record is published, but its directory entry could not be synced and
    /// no `desk` event was written.
    Unsynced(String),
    /// The caller is no longer the orchestrator seat that was proved.
    Admission(String),
}

impl SetError {
    /// The one-line reason for stderr.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Lock(why) => format!("the board is locked by another writer: {why}"),
            Self::Blocked(why) => format!("the stored board cannot be replaced: {why}"),
            Self::Write(why) => format!("the board was not written: {why}"),
            Self::Event(why) => {
                format!("the board was written but its event was not recorded: {why}")
            }
            Self::Unsynced(why) => format!("the board was written but not synced: {why}"),
            Self::Admission(why) => why.clone(),
        }
    }
}

impl fmt::Display for SetError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&self.message())
    }
}

/// Replace the record in `dir` with `items` when they differ from what stands,
/// journaling one `desk` event for `actor`.
///
/// The stored `updated` is `now`, or one second on when the replaced record
/// carries that second or the next, so a change shows as a later time and the
/// shown time never goes back.
///
/// # Errors
///
/// [`SetError`]: see its variants. A damaged record is replaced, never
/// refused: that is the repair path.
pub fn set(
    dir: &Path,
    items: Vec<Item>,
    now: Timestamp,
    actor: &str,
) -> Result<SetOutcome, SetError> {
    set_with_sync(dir, items, now, actor, sync_dir)
}

/// [`set`] with the directory sync injected: the one place a test can fail the
/// sync without a broken filesystem. `sync` runs at most once, with the
/// session directory and the writer lock held, after the new record is
/// visible and before the `desk` event.
pub(crate) fn set_with_sync(
    dir: &Path,
    items: Vec<Item>,
    now: Timestamp,
    actor: &str,
    sync: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<SetOutcome, SetError> {
    let held = lock_writer(dir)?;
    publish_locked(dir, items, now, actor, sync, &held)
}

/// The writer lock of one session's board, held until it is dropped. A
/// [`publish_locked`] takes it by reference, so no publish can run without it
/// and no path takes the lock twice.
struct DeskLock {
    _file: std::fs::File,
}

fn lock_writer(dir: &Path) -> Result<DeskLock, SetError> {
    let lock = store::lock_path(&dir.join(RECORD_FILE));
    store::lock(&lock, store::LOCK_WAIT)
        .map(|file| DeskLock { _file: file })
        .map_err(|why| SetError::Lock(why.to_string()))
}

/// Make the entry of a renamed record durable: fsync the session directory.
fn sync_dir(dir: &Path) -> io::Result<()> {
    std::fs::OpenOptions::new()
        .read(true)
        .open(dir)
        .and_then(|directory| directory.sync_all())
}

/// The time a change is stamped with: `now`, except that a change replacing a
/// record stamped with `now` or one second ahead of it is stamped one second
/// ahead, so the first change in a second shows as later and a burst of changes
/// in that second never shows an earlier time than the one before. A replaced
/// time further ahead of the clock is not trusted: it would carry a future
/// date into the shown time. The tick is kept only while it is still a
/// canonical time.
fn next_update(now: Timestamp, replaced: Option<Timestamp>) -> Timestamp {
    let ahead = replaced.map(|at| at.epoch().saturating_sub(now.epoch()));
    if !matches!(ahead, Some(0 | 1)) {
        return now;
    }
    let tick = Timestamp::from_epoch(now.epoch().saturating_add(1));
    if Timestamp::parse(&tick.to_string()) == Some(tick) {
        tick
    } else {
        now
    }
}

/// [`set`] with the writer lock held by the caller: stage and sync the record,
/// rename it, sync the directory, then append the `desk` event. A failure
/// before the rename leaves the old record; a failed directory sync reports the
/// new record as published but unconfirmed and writes no event, so no event can
/// outlive a record that reverts on a crash.
fn publish_locked(
    dir: &Path,
    items: Vec<Item>,
    now: Timestamp,
    actor: &str,
    sync: impl FnOnce(&Path) -> io::Result<()>,
    _held: &DeskLock,
) -> Result<SetOutcome, SetError> {
    // Only a record that parses can equal the new items: a missing or
    // damaged one is replaced, an empty board included.
    let stored = match read(dir) {
        Ok(found) => found,
        Err(ReadError::Parse(_) | ReadError::TooLarge) => None,
        Err(blocked) => return Err(SetError::Blocked(blocked)),
    };
    let before: &[Item] = stored.as_ref().map_or(&[], |desk| desk.items.as_slice());
    let delta = record::diff(before, &items);
    if stored.is_some() && delta.is_empty() {
        return Ok(SetOutcome::Unchanged);
    }
    let updated = next_update(now, stored.as_ref().map(|desk| desk.updated));
    let total = items.len();
    let desk = Desk { updated, items };
    store::publish_private_in(dir, RECORD_FILE, &record::encode(&desk))
        .map_err(|why| SetError::Write(why.to_string()))?;
    sync(dir).map_err(|why| SetError::Unsynced(why.to_string()))?;
    let summary = record::summary(&delta, total);
    let fields = EventFields::new(
        now,
        actor,
        record::ACTION,
        "",
        "",
        "",
        "",
        "",
        "",
        &summary,
        "",
    );
    store::open(dir)
        .append_event(&event_line(&fields))
        .map_err(|why| SetError::Event(why.to_string()))?;
    Ok(SetOutcome::Changed { delta, total })
}

/// The grant of one reading of the calling pane for the board writer: the
/// helper of the caller's own session, held by that session's orchestrator
/// seat in its recorded main pane. The session is compared before the role, so
/// an ordinary caller never meets a pane fact.
fn writer_grant(dir: &Path, observed: Option<&Observed>) -> Result<Grant, &'static str> {
    let grant = Grant::of_observed(dir, observed)?;
    if grant.session_dir() != dir {
        return Err(
            "this is the desk helper of another session: run the helper of the session you are in",
        );
    }
    grant.dispatcher()?;
    Ok(grant)
}

/// A caller proved to be this session's orchestrator seat by ONE reading of
/// its pane. The helper takes it before it reads any input and hands it to
/// [`publish_admitted`], which proves the seat again under the writer lock and
/// refuses unless that is the same seat, so a launch that changes while the
/// input is read cannot publish the request made under the old one.
pub(crate) struct Admitted(Grant);

/// Prove the caller is this session's orchestrator seat, taking no lock and
/// reading no record: the helper asks it before it reads any input.
///
/// # Errors
///
/// [`SetError::Admission`] with the reason.
pub(crate) fn admit_caller(
    dir: &Path,
    observe: &mut impl FnMut() -> Option<Observed>,
) -> Result<Admitted, SetError> {
    writer_grant(dir, observe().as_ref())
        .map(Admitted)
        .map_err(|why| SetError::Admission(why.to_owned()))
}

/// [`set`] for the helper: under the writer lock the caller is proved again,
/// the fresh proof must be the seat `first` proved, and the fresh proof names
/// the actor of the `desk` event.
///
/// # Errors
///
/// [`SetError`]: [`SetError::Admission`] when the fresh proof fails or the seat
/// is another since `first`, else as [`set`].
pub(crate) fn publish_admitted(
    dir: &Path,
    first: &Admitted,
    observe: &mut impl FnMut() -> Option<Observed>,
    items: Vec<Item>,
    now: Timestamp,
) -> Result<SetOutcome, SetError> {
    publish_admitted_with_sync(dir, first, observe, items, now, sync_dir)
}

/// [`publish_admitted`] with the directory sync injected, as
/// [`set_with_sync`]: the one guarded publisher the helper and the one-call
/// forms share.
pub(crate) fn publish_admitted_with_sync(
    dir: &Path,
    first: &Admitted,
    observe: &mut impl FnMut() -> Option<Observed>,
    items: Vec<Item>,
    now: Timestamp,
    sync: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<SetOutcome, SetError> {
    let held = lock_writer(dir)?;
    let second = writer_grant(dir, observe().as_ref())
        .map_err(|why| SetError::Admission(format!("the caller is unproven now: {why}")))?;
    if second != first.0 {
        return Err(SetError::Admission(String::from(
            "the seat changed while the board was set",
        )));
    }
    publish_locked(dir, items, now, second.actor(), sync, &held)
}

/// The one-call form of [`admit_caller`] then [`publish_admitted`], for a
/// caller that already holds its input: two readings of the pane, one before
/// the lock and one under it.
///
/// # Errors
///
/// As [`publish_admitted`], or [`SetError::Admission`] when the first proof
/// fails.
#[cfg(test)]
pub(crate) fn admit_set(
    dir: &Path,
    observe: &mut impl FnMut() -> Option<Observed>,
    items: Vec<Item>,
    now: Timestamp,
) -> Result<SetOutcome, SetError> {
    admit_set_with_sync(dir, observe, items, now, sync_dir)
}

/// [`admit_set`] with the directory sync injected.
#[cfg(test)]
pub(crate) fn admit_set_with_sync(
    dir: &Path,
    observe: &mut impl FnMut() -> Option<Observed>,
    items: Vec<Item>,
    now: Timestamp,
    sync: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<SetOutcome, SetError> {
    let first = admit_caller(dir, observe)?;
    publish_admitted_with_sync(dir, &first, observe, items, now, sync)
}

/// What the helper prints and the status it exits with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    /// The exit status: 0 done, 1 nothing changed, 2 the board changed but the
    /// write is not whole.
    pub code: u8,
    /// What goes to stdout.
    pub stdout: String,
    /// What goes to stderr.
    pub stderr: String,
}

fn items_noun(total: usize) -> String {
    format!("{total} {}", if total == 1 { "item" } else { "items" })
}

/// The helper's words for the end of a set of `total` items. Every refusal
/// starts `desk set REFUSED` and changed nothing; the two exit-2 words start
/// `FAILED AFTER WRITE` and `UNCONFIRMED` and say the board DID change.
#[must_use]
pub fn render_set(result: &Result<SetOutcome, SetError>, total: usize) -> Rendered {
    let refused = |why: String| Rendered {
        code: 1,
        stdout: String::new(),
        stderr: format!("desk set REFUSED: {why}\n"),
    };
    match result {
        Ok(SetOutcome::Unchanged) => Rendered {
            code: 0,
            stdout: format!("desk set: unchanged ({})\n", items_noun(total)),
            stderr: String::new(),
        },
        Ok(SetOutcome::Changed { delta, total }) => Rendered {
            code: 0,
            stdout: format!(
                "desk set: {} written ({})\n",
                items_noun(*total),
                record::change_ids(delta)
            ),
            stderr: String::new(),
        },
        Err(error @ (SetError::Lock(_) | SetError::Blocked(_) | SetError::Write(_))) => {
            refused(error.message())
        }
        Err(SetError::Admission(why)) => refused(why.clone()),
        Err(SetError::Event(why)) => Rendered {
            code: 2,
            stdout: String::new(),
            stderr: format!(
                "desk set FAILED AFTER WRITE: the board was replaced ({}) and its journal event was not recorded (the detail says when the journal's state is unknown): {why}\n",
                items_noun(total)
            ),
        },
        Err(SetError::Unsynced(why)) => Rendered {
            code: 2,
            stdout: String::new(),
            stderr: format!(
                "desk set UNCONFIRMED: the board was replaced ({}) but its directory entry could not be synced, so it may not survive a crash, and no journal event was written: {why}\n",
                items_noun(total)
            ),
        },
    }
}
