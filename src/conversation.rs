//! Which conversation one tool start CHOSE: `_run` journals one record per
//! start before the exec, its `ref` the grammar
//! `<slot>:<launch_id>:<pid> <prior|-> <id|pending>`. A record is a decision,
//! never proof of context; [`current`] says whether it describes the running
//! start. The summary is display only and nothing here reads it.

use std::path::Path;

use crate::events::Event;
use crate::launch::{PENDING, id_probeable};
use crate::launch_cmd::Resolved;
use crate::tool::ToolKind;

/// What the tool's own store says about a recorded id. `Unreadable` is never
/// proof of absence; `NoId` is an absent, empty, `pending` or ungrammatical
/// row; `Unprobed` a tool with no probe or a store that cannot be located.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    Found,
    NotFound,
    Unreadable,
    NoId,
    Unprobed,
}

/// Why a resume passed on its recorded conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    NoId,
    NotFound,
    Unreadable,
}

/// What one tool start chose: an exact resume the probe proved or could not
/// check, a first start, or a resume that started fresh instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    Resumed,
    Unproven,
    New,
    Lost(Cause),
}

/// One decoded record: `start_key` is `<slot>:<launch_id>:<pid>`, `prior` the
/// id a loss passed over, `id` the one launched or `pending`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub start: Start,
    pub start_key: String,
    pub prior: Option<String>,
    pub id: String,
}

/// A record that names a conversation action but breaks its grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Damage;

/// Why [`current`] will not name the running start's choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    Unrecorded,
    Incomplete,
    Damaged,
    NoLiveStart,
    Stale,
    LaunchedMismatch,
    LaunchedDamaged,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Current(Record),
    Unknown(Why),
}

impl Cause {
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::NoId => "no-id",
            Self::NotFound => "not-found",
            Self::Unreadable => "unreadable",
        }
    }
}

impl Start {
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Resumed => "resumed",
            Self::Unproven => "unproven",
            Self::New => "new",
            Self::Lost(_) => "lost",
        }
    }
}

impl Verdict {
    /// The `ae list` cell: silence is a proven current resume and nothing else.
    #[must_use]
    pub fn cell(&self) -> Option<String> {
        let word = match self {
            Self::Current(record) => match record.start {
                Start::Resumed => return None,
                Start::Lost(cause) => format!("lost({})", cause.word()),
                start => start.word().to_owned(),
            },
            Self::Unknown(Why::Unrecorded) => "unrecorded".to_owned(),
            Self::Unknown(_) => "unknown".to_owned(),
        };
        Some(format!(" · conv:{word}"))
    }

    /// Why the running start's choice is unknown, `None` when it is current.
    #[must_use]
    pub const fn why(&self) -> Option<&'static str> {
        let Self::Unknown(why) = self else {
            return None;
        };
        Some(match why {
            Why::Unrecorded => "unrecorded",
            Why::Incomplete => "incomplete",
            Why::Damaged => "damaged",
            Why::NoLiveStart => "no-live-start",
            Why::Stale => "stale",
            Why::LaunchedMismatch => "launched-mismatch",
            Why::LaunchedDamaged => "launched-damaged",
            Why::Conflict => "conflict",
        })
    }
}

const ACTIONS: [(Start, &str); 6] = [
    (Start::Resumed, "conversation-resumed"),
    (Start::Unproven, "conversation-unproven"),
    (Start::New, "conversation-new"),
    (Start::Lost(Cause::NoId), "conversation-lost-no-id"),
    (Start::Lost(Cause::NotFound), "conversation-lost-not-found"),
    (
        Start::Lost(Cause::Unreadable),
        "conversation-lost-unreadable",
    ),
];

/// The tool's own store probe: [`crate::run::probe_in`], the one owner.
#[must_use]
pub fn probe(tool: ToolKind, id: &str, config_home: &Resolved, work_dir: Option<&Path>) -> Probe {
    crate::run::probe_in(tool, id, config_home, work_dir)
}

#[must_use]
pub fn action(start: Start) -> &'static str {
    ACTIONS
        .iter()
        .find(|(kind, _)| *kind == start)
        .map_or("", |(_, action)| action)
}

#[must_use]
pub fn reference(record: &Record) -> String {
    let prior = record.prior.as_deref().unwrap_or("-");
    format!("{} {prior} {}", record.start_key, record.id)
}

/// `None` for any other action; `Damage` for a conversation action whose
/// `ref` breaks the grammar or its kind's rules.
#[must_use]
pub fn decode(event: &Event) -> Option<Result<Record, Damage>> {
    let (start, _) = ACTIONS.iter().find(|(_, action)| *action == event.action)?;
    let reference = event.reference.as_deref().unwrap_or_default();
    let Ok([key, prior, id]) = <[&str; 3]>::try_from(reference.split(' ').collect::<Vec<_>>())
    else {
        return Some(Err(Damage));
    };
    let prior = (prior != "-").then_some(prior);
    let rules = match start {
        Start::Resumed | Start::Unproven => prior.is_none() && id != PENDING,
        Start::New | Start::Lost(Cause::NoId) => prior.is_none(),
        Start::Lost(_) => prior.is_some(),
    };
    let grammar =
        start_key(key) && prior.is_none_or(id_probeable) && (id == PENDING || id_probeable(id));
    let record = Record {
        start: *start,
        start_key: key.to_owned(),
        prior: prior.map(ToOwned::to_owned),
        id: id.to_owned(),
    };
    Some(if rules && grammar {
        Ok(record)
    } else {
        Err(Damage)
    })
}

/// Whether `events` hold a conversation record, damaged or not, whose ref
/// begins with a start of `slot`'s `launch_id`: what makes a process read worth
/// taking.
#[must_use]
pub fn attributable(events: &[Event], slot: &str, launch_id: &str) -> bool {
    let prefix = format!("{slot}:{launch_id}:");
    events.iter().any(|event| {
        decode(event).is_some()
            && event
                .reference
                .as_deref()
                .is_some_and(|raw| raw.starts_with(&prefix))
    })
}

/// `main|worker.N|spawned.N`, a lowercase uuid and a pid with no sign, no
/// leading zero and not 0.
fn start_key(key: &str) -> bool {
    let Ok([slot, launch, pid]) = <[&str; 3]>::try_from(key.split(':').collect::<Vec<_>>()) else {
        return false;
    };
    let numbered = |prefix: &str| {
        slot.strip_prefix(prefix)
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    };
    (slot == "main" || numbered("worker.") || numbered("spawned."))
        && crate::session_launch::capture::is_lowercase_uuid(launch)
        && !pid.starts_with('0')
        && pid.bytes().all(|b| b.is_ascii_digit())
        && pid.parse::<u32>().is_ok()
}

/// The choice of the start running now in `slot`'s `launch_id`. `complete` is
/// false when the journal was not read whole; `launched` is the meta
/// `launched.<slot>` start (`Err` damaged); `live` the start the process table
/// shows. Damage vetoes only a start it could belong to: one naming the live
/// start, or an unparseable start of this seat newer than the newest valid
/// record of the live start.
#[must_use]
pub fn current(
    events: &[Event],
    complete: bool,
    slot: &str,
    launch_id: &str,
    launched: Option<Result<&str, ()>>,
    live: Option<&str>,
) -> Verdict {
    if !complete {
        return Verdict::Unknown(Why::Incomplete);
    }
    let prefix = format!("{slot}:{launch_id}:");
    let (mut valid, mut unplaced, mut live_damage) = (Vec::new(), Vec::new(), false);
    for (at, event) in events.iter().enumerate() {
        match decode(event) {
            Some(Ok(record)) if record.start_key.starts_with(&prefix) => valid.push((at, record)),
            Some(Err(Damage)) => {
                let raw = event.reference.as_deref().unwrap_or_default();
                let key = raw.split(' ').next().unwrap_or_default();
                if start_key(key) {
                    live_damage |= Some(key) == live && key.starts_with(&prefix);
                } else if raw.starts_with(&prefix) {
                    unplaced.push(at);
                }
            }
            _ => {}
        }
    }
    if valid.is_empty() {
        let damaged = live_damage || !unplaced.is_empty();
        return Verdict::Unknown(if damaged {
            Why::Damaged
        } else {
            Why::Unrecorded
        });
    }
    let Some(live) = live else {
        return Verdict::Unknown(Why::NoLiveStart);
    };
    let mine: Vec<_> = valid.iter().filter(|(_, r)| r.start_key == live).collect();
    let Some((newest, record)) = mine.last() else {
        return Verdict::Unknown(if live_damage {
            Why::Damaged
        } else {
            Why::Stale
        });
    };
    let why = if live_damage || unplaced.iter().any(|at| at > newest) {
        Why::Damaged
    } else if mine.iter().any(|(_, other)| other != record) {
        Why::Conflict
    } else {
        match launched {
            Some(Err(())) => Why::LaunchedDamaged,
            Some(Ok(start)) if start != live => Why::LaunchedMismatch,
            _ => return Verdict::Current(record.clone()),
        }
    };
    Verdict::Unknown(why)
}

/// The chat lane's line for a loss, `None` for every other choice.
#[must_use]
pub fn said(record: &Record, who: &str) -> Option<String> {
    let Start::Lost(cause) = record.start else {
        return None;
    };
    let what = record.prior.as_deref().map_or_else(
        || "no conversation id was recorded".to_owned(),
        |prior| format!("{} was not resumed", short(prior)),
    );
    Some(format!(
        "{who} start chose a fresh conversation; {what} ({})",
        cause.word()
    ))
}

/// What `_run` says on stderr about a loss; a launched id is the
/// `--session-id` the fresh start was handed.
#[must_use]
pub fn notice(slot: &str, record: &Record, tool: &str) -> Option<String> {
    let Start::Lost(cause) = record.start else {
        return None;
    };
    let why = match (cause, &record.prior) {
        (Cause::NotFound, Some(prior)) => format!("{} not found in its store", short(prior)),
        (Cause::Unreadable, _) => "its store is unreadable".to_owned(),
        _ => "no conversation id was recorded".to_owned(),
    };
    let mint = (record.id != PENDING).then(|| format!(" (--session-id {})", record.id));
    Some(format!(
        "ae: seat {slot}: no proven {tool} conversation ({why}) — starting a fresh {tool} conversation{}",
        mint.unwrap_or_default()
    ))
}

fn short(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}
