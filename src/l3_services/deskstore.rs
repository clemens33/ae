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
use std::path::Path;

use crate::desk::{self as record, Delta, Desk, Item, ParseError, RECORD_FILE, STORED_CAP};
use crate::store::{self, SourceRead};
use crate::time::Timestamp;
use crate::tracked::{EventFields, event_line};

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
    let lock = store::lock_path(&dir.join(RECORD_FILE));
    let _writer =
        store::lock(&lock, store::LOCK_WAIT).map_err(|why| SetError::Lock(why.to_string()))?;
    publish_locked(dir, items, now, actor)
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

/// [`set`] with the writer lock held by the caller.
fn publish_locked(
    dir: &Path,
    items: Vec<Item>,
    now: Timestamp,
    actor: &str,
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
    let summary = record::summary(&delta, total);
    let fields = EventFields::new(now, actor, "desk", "", "", "", "", "", "", &summary, "");
    store::open(dir)
        .append_event(&event_line(&fields))
        .map_err(|why| SetError::Event(why.to_string()))?;
    Ok(SetOutcome::Changed { delta, total })
}
