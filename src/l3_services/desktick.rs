//! The watchdog's desk-stale tick: whether the orchestrator's manual desk
//! record has fallen behind a relevant fleet change, and the claim that marks
//! that change as told.
//!
//! The tick never writes the desk. A change is relevant when a seat turns
//! `waiting-user`, `blocked` or `done`, or a request to a human sink opens or
//! closes. The orchestrator main's own `done`, its overview acknowledgement,
//! is not one. A change counts once: a claimed tick stores, per session, the
//! position just past the newest change it represents, and only records past
//! that position can count again. [`changed`] and [`settle`] read no clock and no file;
//! [`read`] reads the desk and judges the journals one fleet read held.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::digest::Status;
use crate::events::{Cursor, Event, Identity, RefMeaning, RoutingMember, SessionKey};
use crate::time::Timestamp;
use crate::watchdog_state::DeskTick;

/// The journal action of a standalone desk-stale turn.
pub const ACTION: &str = crate::watchdog::DESK_TICK_ACTION;

/// The journal summary of a standalone desk-stale turn.
pub const SUMMARY: &str = "desk record behind the fleet";

/// The one line the orchestrator reads, alone or before an overview trailer.
pub const LINE: &str = "Desk stale: a seat or a request to the human changed after the desk's \
                        last update. Refresh the desk record when it suits you.";

/// A desk this old or younger, in seconds, is never stale.
pub const STALE_AFTER_SECS: i64 = 300;

/// The declared states whose arrival is a relevant change.
const STATES: [&str; 3] = ["waiting-user", "blocked", "done"];

/// One session's journal as the fleet read held it, with each record's real
/// source position and the position the last claimed tick consumed.
#[derive(Debug, Clone, Copy)]
pub struct Journal<'a> {
    /// The session the journal belongs to.
    pub key: SessionKey<'a>,
    /// Every record, in stream order.
    pub events: &'a [Event],
    /// The cursor just past each record of `events`, in the same order.
    pub ends: &'a [Cursor],
    /// The position the last claimed tick consumed, if any.
    pub mark: Option<Cursor>,
}

impl Journal<'_> {
    /// Whether the record at `at` lies past the consumed position. A record
    /// without a known position never does.
    fn past_mark(&self, at: usize) -> bool {
        let Some(end) = self.ends.get(at) else {
            return false;
        };
        self.mark
            .is_none_or(|mark| (end.generation, end.offset) > (mark.generation, mark.offset))
    }
}

/// A stale desk found this cycle, with the positions a claim would consume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stale {
    claim: BTreeMap<String, Cursor>,
    /// The desk record's `updated` line the change was judged against.
    updated: Timestamp,
}

impl Stale {
    /// Whether the desk record read now is still the one judged stale and is
    /// still old enough at `now`: the record is rechecked before the paste.
    #[must_use]
    pub(crate) fn desk_unchanged(&self, desk: Option<&crate::desk::Desk>, now: i64) -> bool {
        desk.is_some_and(|desk| desk.updated == self.updated && old(desk.updated, now))
    }

    /// The checkpoint a tick submitted at `now` stores before its submit.
    #[must_use]
    pub(crate) fn claim(&self, now: i64) -> DeskTick {
        DeskTick {
            last_at: Some(now),
            cursor: self.claim.clone(),
        }
    }
}

/// What one cycle's observation booked for its delivery: the stale change,
/// the stored claim it was judged against and the overview delivery time the
/// spacing decision read. The delivery claims only while both are unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Booking {
    /// The change and the claim that would consume it.
    pub stale: Stale,
    /// The stored desk claim the observation read.
    pub prior: DeskTick,
    /// The stored latest overview delivery the observation read.
    pub overview_last: Option<i64>,
}

/// What the claim does after the send helper answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settle {
    /// The turn landed, or may have: the change counts as told.
    Keep,
    /// The helper proved it refused before any paste: the change stays due.
    Release,
}

/// Fold the helper's answer into the claim. Only a typed pre-paste refusal,
/// a failing exit whose output carries [`crate::send::RETRYABLE_MARKER`] on
/// its own line, releases it. Success, any other failure and an unknown exit
/// keep it, because a turn that may have landed must not be sent twice.
#[must_use]
pub fn settle(code: Option<i32>, stdout: &str) -> Settle {
    let refused = code.is_some_and(|code| code != 0)
        && stdout
            .lines()
            .any(|line| line == crate::send::RETRYABLE_MARKER);
    if refused {
        Settle::Release
    } else {
        Settle::Keep
    }
}

/// Whether a desk last updated at `updated` is old enough to be stale at
/// `now`: strictly more than [`STALE_AFTER_SECS`], which also excludes a
/// future stamp.
#[must_use]
pub fn old(updated: Timestamp, now: i64) -> bool {
    now.saturating_sub(updated.epoch()) > STALE_AFTER_SECS
}

/// The newest relevant change in `journal` past its consumed position and
/// strictly newer than `updated`, as an index into its events. `ack` names
/// the orchestrator main's agent when `journal` is the orchestrator's own
/// session: that seat's `done` is its overview acknowledgement, never a change.
#[must_use]
pub fn changed(journal: &Journal<'_>, updated: Timestamp, ack: Option<&str>) -> Option<usize> {
    state_edge(journal, updated, ack).max(request_edge(journal, updated))
}

fn counts(journal: &Journal<'_>, at: usize, updated: Timestamp) -> bool {
    journal.past_mark(at)
        && journal
            .events
            .get(at)
            .is_some_and(|event| event.ts > updated)
}

/// The seat incarnation a declaration belongs to, by the routing rules every
/// reader shares ([`Event::actor_identity`]): a routed or stable identity
/// counts only when it names a slot of this journal's session, and then with
/// its launch; a display name stands for itself unless ae reserves it. A
/// partial routing key names nobody.
fn seat_of(event: &Event, session: SessionKey<'_>) -> Option<String> {
    let slot = match event.actor_identity() {
        Identity::Display(name) => {
            return (!crate::config::is_reserved_actor(name)).then(|| format!("actor\u{1f}{name}"));
        }
        identity @ (Identity::Routed { slot, .. } | Identity::Stable { slot, .. }) => {
            if !identity.matches(session.seat(slot)) {
                return None;
            }
            slot
        }
        Identity::Unassociated => return None,
    };
    let launch = match &event.actor_launch_id {
        RoutingMember::Value(launch) => launch.as_str(),
        RoutingMember::Absent => "",
        RoutingMember::Invalid => return None,
    };
    Some(format!("slot\u{1f}{slot}\u{1f}{launch}"))
}

fn state_edge(journal: &Journal<'_>, updated: Timestamp, ack: Option<&str>) -> Option<usize> {
    let mut seats: BTreeMap<String, &str> = BTreeMap::new();
    let mut newest = None;
    for (at, event) in journal.events.iter().enumerate() {
        let (Some(state), Some(seat)) = (event.declared_state(), seat_of(event, journal.key))
        else {
            continue;
        };
        let previous = seats.insert(seat, state);
        if !counts(journal, at, updated) || !STATES.contains(&state) || previous == Some(state) {
            continue;
        }
        let acknowledgement = state == "done"
            && ack.is_some_and(|agent| {
                crate::watchdog::event_is_actor_in(
                    event,
                    journal.key,
                    crate::watchdog::MAIN_SLOT,
                    agent,
                )
            });
        if !acknowledgement {
            newest = Some(at);
        }
    }
    newest
}

fn request_edge(journal: &Journal<'_>, updated: Timestamp) -> Option<usize> {
    let human = |request: &Event| {
        request
            .target
            .as_deref()
            .is_some_and(crate::tracked::is_human_sink)
    };
    let opened = journal
        .events
        .iter()
        .enumerate()
        .filter(|(at, event)| {
            matches!(event.action.as_str(), "ask" | "review")
                && matches!(event.ref_meaning(), RefMeaning::RequestId(_))
                && human(event)
                && counts(journal, *at, updated)
        })
        .map(|(at, _)| at)
        .max();
    let closed = crate::session::request_closures(journal.events, journal.key)
        .into_iter()
        .filter(|(at, request)| human(request) && counts(journal, *at, updated))
        .map(|(at, _)| at)
        .max();
    opened.max(closed)
}

/// The key a session's consumed position is stored under: its canonical
/// uuid, which a rename keeps, and for a legacy record without one its exact
/// durable directory, so two id-less namesakes never share a position.
fn key_text(dir: &Path, uuid: Option<&str>) -> String {
    match uuid.filter(|uuid| !uuid.is_empty()) {
        Some(uuid) => uuid.to_owned(),
        None => format!("path\u{1f}{}", dir.display()),
    }
}

/// Each journal the fleet read held, by its classified record's state
/// directory, with the real source position of each of its events.
pub type Positions = BTreeMap<PathBuf, Vec<Cursor>>;

/// Judge the orchestrator's desk in `own_dir` against the journal of every
/// running session in `read`, the one classified fleet read this cycle took,
/// with `positions` its reader recorded. `ack` applies only to the record
/// that is `own_dir` itself, by exact durable directory or by the same
/// session uuid, never to a namesake. `None` when the desk is absent,
/// damaged, not old enough, or no running session holds a relevant change
/// past `prior`'s positions. The claim starts from `prior`: a session whose
/// record, journal or positions are missing keeps its stored position, and a
/// session with a change moves to the newest change the tick represents.
#[must_use]
pub(crate) fn read(
    read: &crate::fleet_read::WorldRead,
    positions: &Positions,
    (own_dir, ack): (&Path, Option<&str>),
    prior: &DeskTick,
    now: i64,
) -> Option<Stale> {
    let desk = crate::deskstore::read(own_dir).ok().flatten()?;
    if !old(desk.updated, now) {
        return None;
    }
    let classified = &read.snapshot.sessions;
    let entries = &read.world.sessions;
    if classified.len() != entries.len() {
        return None;
    }
    let own_uuid = crate::meta::session_id_in(own_dir);
    let mut claim = prior.cursor.clone();
    let mut stale = false;
    for (entry, classified) in entries.iter().zip(classified) {
        if entry.status != Status::Running {
            continue;
        }
        let Some(record) = classified.candidate.durable.as_ref() else {
            continue;
        };
        let (Some(journal), Some(ends)) =
            (record.snapshot.events.as_ref(), positions.get(&record.path))
        else {
            continue;
        };
        if ends.len() != journal.events.len() {
            continue;
        }
        let uuid = entry.session_uuid.as_deref();
        let stored = key_text(&record.path, uuid);
        let journal = Journal {
            key: SessionKey::new(&entry.name, uuid.unwrap_or_default()),
            events: &journal.events,
            ends,
            mark: prior.cursor.get(&stored).copied(),
        };
        let ack = ack.filter(|_| {
            record.path == own_dir
                || (!own_uuid.is_empty()
                    && uuid.is_some_and(|uuid| uuid.eq_ignore_ascii_case(&own_uuid)))
        });
        if let Some(at) = changed(&journal, desk.updated, ack)
            && let Some(end) = ends.get(at)
        {
            stale = true;
            claim.insert(stored, *end);
        }
    }
    stale.then_some(Stale {
        claim,
        updated: desk.updated,
    })
}

#[cfg(test)]
#[path = "desktick/desktick_tests.rs"]
mod tests;
