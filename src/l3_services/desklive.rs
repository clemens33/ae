//! The board's live part: what ae itself knows of the fleet right now, beside
//! the board the orchestrator writes.
//!
//! The live part is computed, never stored: no file, no second writer of the
//! board record. [`collect`] takes one fleet read (the same read `ae list`
//! takes); [`from_reads`] turns its typed facts into a [`Live`] snapshot and
//! reads no clock and no file; [`lines`] renders it. Each reader process keeps
//! a [`Cache`], so the fleet is read at most once a minute per viewer.
//!
//! A session whose record cannot be read is `unavailable`, never a zero, and
//! makes the whole snapshot `partial`.

use crate::digest::{SessionEntry, Status};
use crate::session::PendingRequest;
use crate::time::Timestamp;

/// The seconds a snapshot is reused before the fleet is read again.
pub const CACHE_SECS: i64 = 60;

/// The label of the per-session count of non-main seats declared `working`.
pub const WORKERS_LABEL: &str = "running workers";
/// The label of the per-session count of open asks whose target is a human.
pub const ASKS_LABEL: &str = "open asks to you";

/// The most characters of a declared reason a row shows.
const REASON_CHARS: usize = 96;

/// One roster seat as the live part shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveSeat {
    /// The seat's name.
    pub name: String,
    /// Whether it holds the session's `main` slot.
    pub main: bool,
    /// Its declared state: `working`, `done`, `waiting-user`,
    /// `waiting-agent`, `blocked`, `-` for none, `unknown` when the world
    /// could not establish it.
    pub state: String,
    /// When it declared that state, when the declaration carries a stamp.
    pub since: Option<Timestamp>,
    /// The reason it declared with, empty when it gave none.
    pub reason: String,
}

/// One open ask whose target is a human sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanAsk {
    /// The request id.
    pub id: String,
    /// Who asked.
    pub from: String,
    /// When it was sent.
    pub sent_at: Timestamp,
}

/// What a readable session shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionBody {
    /// Every roster seat, waits first, then working, then the rest; roster
    /// order within each group.
    pub seats: Vec<LiveSeat>,
    /// Open asks to the human, oldest first.
    pub asks: Vec<HumanAsk>,
}

impl SessionBody {
    /// The non-main seats declared `working`.
    #[must_use]
    pub fn running_workers(&self) -> usize {
        self.seats
            .iter()
            .filter(|seat| !seat.main && seat.state == "working")
            .count()
    }
}

/// One running (or not provably stopped) session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveSession {
    /// The session's name.
    pub name: String,
    /// The state directory of the record this session was read from.
    pub dir: Option<std::path::PathBuf>,
    /// The session's own uuid, lower-case, when its meta names one.
    pub uuid: Option<String>,
    /// Its seats and asks, or why they cannot be shown.
    pub body: Result<SessionBody, String>,
}

/// The live snapshot of the fleet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Live {
    /// When the fleet was read.
    pub as_of: Timestamp,
    /// The sessions: `own` first, then by name.
    pub sessions: Vec<LiveSession>,
    /// How many fleet sources failed to enumerate.
    pub losses: usize,
}

impl Live {
    /// Whether anything the snapshot should hold is missing.
    #[must_use]
    pub fn partial(&self) -> bool {
        self.losses > 0 || self.sessions.iter().any(|session| session.body.is_err())
    }
}

/// One session's typed inputs: its world entry and the open requests its
/// journal folded, `None` when the journal could not be read.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    /// The world's entry.
    pub entry: &'a SessionEntry,
    /// The state directory of the record the entry was read from.
    pub dir: Option<&'a std::path::Path>,
    /// The request fold's pending rows, or why there are none.
    pub pending: Result<&'a [PendingRequest], &'a str>,
}

/// The order a state's seats stand in: waits first.
fn rank(state: &str) -> u8 {
    match state {
        "waiting-user" => 0,
        "blocked" => 1,
        "waiting-agent" => 2,
        "working" => 3,
        "done" => 4,
        _ => 5,
    }
}

/// The seats of `entry`, from the world alone: the state the world read and
/// the words and stamp of the very declaration it read it from
/// ([`crate::brief`]'s world-only agent lines), waits first.
fn seats(entry: &SessionEntry, as_of: Timestamp) -> Vec<LiveSeat> {
    let mut seats: Vec<LiveSeat> = crate::brief::agent_lines_from_world(entry, as_of)
        .into_iter()
        .map(|line| LiveSeat {
            main: entry.main_agent.as_deref() == Some(line.name.as_str()),
            since: line
                .age_secs
                .map(|age| Timestamp::from_epoch(as_of.epoch().saturating_sub(age))),
            name: line.name,
            state: line.state,
            reason: line.reason,
        })
        .collect();
    seats.sort_by_key(|seat| rank(&seat.state));
    seats
}

/// The open asks among `pending` whose target is a human sink, oldest first.
fn asks(pending: &[PendingRequest]) -> Vec<HumanAsk> {
    let mut asks: Vec<HumanAsk> = pending
        .iter()
        .filter(|request| {
            request
                .target
                .as_deref()
                .is_some_and(crate::tracked::is_human_sink)
        })
        .map(|request| HumanAsk {
            id: request.id.clone(),
            from: request.actor.clone(),
            sent_at: request.sent_at,
        })
        .collect();
    asks.sort_by_key(|ask| ask.sent_at.epoch());
    asks
}

/// The live snapshot of `inputs` as of `as_of`. Stopped sessions are left
/// out; a session whose liveness is unknown, whose record lost data or whose
/// journal could not be read is `unavailable` with the reason. `own` (the
/// orchestrator's session) comes first, the rest by name.
#[must_use]
pub fn from_reads(inputs: &[Input<'_>], own: &str, as_of: Timestamp, losses: usize) -> Live {
    let mut sessions: Vec<LiveSession> = inputs
        .iter()
        .filter(|input| input.entry.status != Status::Stopped)
        .map(|input| {
            let entry = input.entry;
            let body = if entry.status == Status::Unknown {
                Err("liveness unknown".to_owned())
            } else if entry.degraded {
                Err("record partly unreadable".to_owned())
            } else {
                match input.pending {
                    Err(why) => Err(format!("journal unreadable: {why}")),
                    Ok(pending) => Ok(SessionBody {
                        seats: seats(entry, as_of),
                        asks: asks(pending),
                    }),
                }
            };
            LiveSession {
                name: entry.name.clone(),
                dir: input.dir.map(std::path::Path::to_path_buf),
                uuid: entry.session_uuid.clone(),
                body,
            }
        })
        .collect();
    sessions.sort_by(|left, right| {
        (left.name != own)
            .cmp(&(right.name != own))
            .then_with(|| left.name.as_bytes().cmp(right.name.as_bytes()))
    });
    Live {
        as_of,
        sessions,
        losses,
    }
}

/// How a row tells time: an age (`4m`, for a surface read at once) or the
/// clock time it started (`since 07:58`, for a surface edited rarely).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Elapsed {
    /// `brief::age` of the time since, as of the snapshot.
    Age,
    /// `since HH:MM` UTC.
    Since,
}

fn elapsed(at: Option<Timestamp>, as_of: Timestamp, style: Elapsed) -> String {
    match (style, at) {
        (_, None) => "-".to_owned(),
        (Elapsed::Age, Some(at)) => crate::brief::age(Some(at.seconds_until(as_of))),
        (Elapsed::Since, Some(at)) => {
            let stamp = at.to_string();
            format!("since {}", stamp.get(11..16).unwrap_or("--:--"))
        }
    }
}

fn clip(text: &str) -> String {
    let clean = crate::board::terminal_text(text);
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.chars().count() <= REASON_CHARS {
        clean
    } else {
        let mut cut: String = clean.chars().take(REASON_CHARS - 1).collect();
        cut.push('…');
        cut
    }
}

/// The heading of the live part: `LIVE  as of <clock>`, and ` (partial)`
/// when something is missing. `clock` is the caller's rendering of `as_of`.
#[must_use]
pub fn header(live: &Live, clock: &str) -> String {
    let partial = if live.partial() { " (partial)" } else { "" };
    format!("LIVE  as of {clock}{partial}")
}

/// The rows below [`header`]: per session a heading with its two counts, or
/// why it is unavailable; then a row per seat and a row per open ask to the
/// human. Every shown field passes the terminal-output gate.
#[must_use]
pub fn rows(live: &Live, style: Elapsed) -> Vec<String> {
    let mut out = Vec::new();
    for session in &live.sessions {
        let name = crate::board::terminal_text(&session.name);
        match &session.body {
            Err(why) => out.push(format!("{name}  unavailable: {}", clip(why))),
            Ok(body) => {
                out.push(format!(
                    "{name}  {WORKERS_LABEL}: {} · {ASKS_LABEL}: {}",
                    body.running_workers(),
                    body.asks.len()
                ));
                for seat in &body.seats {
                    let mut row = format!(
                        "  {}  {}  {}",
                        crate::board::terminal_text(&seat.name),
                        crate::board::terminal_text(&seat.state),
                        elapsed(seat.since, live.as_of, style)
                    );
                    let reason = clip(&seat.reason);
                    if !reason.is_empty() {
                        row.push_str("  ");
                        row.push_str(&reason);
                    }
                    out.push(row);
                }
                for ask in &body.asks {
                    out.push(format!(
                        "  ask {} from {}  {}",
                        crate::board::terminal_text(&ask.id),
                        crate::board::terminal_text(&ask.from),
                        elapsed(Some(ask.sent_at), live.as_of, style)
                    ));
                }
            }
        }
    }
    out
}

/// A snapshot kept for [`CACHE_SECS`]: each reader process holds one.
#[derive(Debug, Default)]
pub struct Cache {
    held: Option<(i64, Live)>,
}

impl Cache {
    /// A cache already holding `live`, sampled at `at`.
    #[must_use]
    pub const fn holding(at: i64, live: Live) -> Self {
        Self {
            held: Some((at, live)),
        }
    }

    /// The snapshot held now, however old: what a reader that refreshes it
    /// elsewhere publishes.
    #[must_use]
    pub fn held(&self) -> Option<&Live> {
        self.held.as_ref().map(|(_, live)| live)
    }

    /// The held snapshot while it is younger than [`CACHE_SECS`] at `now`,
    /// else the one `read` takes, which is then held.
    pub fn get(&mut self, now: i64, read: impl FnOnce() -> Live) -> &Live {
        let fresh = self
            .held
            .as_ref()
            .is_some_and(|(at, _)| now >= *at && now - *at < CACHE_SECS);
        if !fresh {
            self.held = None;
        }
        &self.held.get_or_insert_with(|| (now, read())).1
    }
}

/// The live snapshot of the fleet under the state root `root`, read now:
/// one fleet read, each world entry paired with the classified record it was
/// built from (the listing keeps one entry per classified candidate, in
/// order), so its request fold and its directory are that record's own.
/// `own` is the orchestrator's session.
#[must_use]
pub fn collect(root: &std::path::Path, own: &str) -> Live {
    from_world(
        &crate::fleet_read::world_read(root, &mut crate::session::RecordSnapshot::read),
        own,
    )
}

/// The live snapshot of one fleet read a caller already holds, so a reader
/// that reads the fleet anyway takes no second census.
#[must_use]
pub(crate) fn from_world(read: &crate::fleet_read::WorldRead, own: &str) -> Live {
    let classified = &read.snapshot.sessions;
    let entries = &read.world.sessions;
    let paired = classified.len() == entries.len();
    let inputs: Vec<Input<'_>> = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let durable = classified
                .get(index)
                .filter(|_| paired)
                .and_then(|classified| classified.candidate.durable.as_ref());
            let pending = match durable {
                _ if !paired => Err("the fleet read did not pair its records"),
                None => Err("no durable record"),
                Some(record) => record
                    .snapshot
                    .events
                    .as_ref()
                    .map(|events| events.pending.as_slice())
                    .ok_or("events not read"),
            };
            Input {
                entry,
                dir: durable.map(|record| record.path.as_path()),
                pending,
            }
        })
        .collect();
    from_reads(&inputs, own, read.world.now, read.world.losses)
}

#[cfg(test)]
#[path = "desklive/desklive_tests.rs"]
mod tests;
