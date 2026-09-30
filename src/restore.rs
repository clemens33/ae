//! Which saved sessions were RUNNING when their recorded tmux server died.
//!
//! Bare `ae` restores those. The predicate reads facts ae already keeps — the
//! launch stamp, the stop ledger — plus ONE mtime-only fact the watchdog adds,
//! [`crate::watchdog_glue::beat_modified`]. Nothing persisted is parsed here:
//! the ledger comes through the event reader every consumer shares.

use std::collections::BTreeMap;

use crate::events::Event;
use crate::inventory::{DurableRecord, Layout, Roots};
use crate::lifecycle::{
    ALREADY_STOPPED_SUMMARY, STOP_REQUEST_ACTION, STOP_RESULT_ACTION, STOPPED_SUMMARY,
};
use crate::meta::{Selector, ServerSelector};
use crate::session::SessionRead;
use crate::tmux::Evidence;

/// How far behind the newest beat of its cohort a session's own beat may sit.
/// Fixed: the sessions one crash takes down stop beating within one cycle of
/// each other, and a longer window would resurrect a session abandoned earlier
/// the same hour.
pub const WINDOW_SECS: i64 = 15 * 60;

/// What a session's stop ledger says since its current launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ledger {
    /// No stop since the launch, or one that failed.
    Clear,
    /// A clean stop, or a stop request nobody has answered — fail closed.
    Stopped,
    /// The ledger could not be read whole, so what it said is unknown.
    Damaged,
}

/// Everything the predicate needs to know about one session.
#[derive(Debug, Clone)]
pub struct Fact {
    pub name: String,
    pub server: ServerSelector,
    /// Already running on the server it was recorded on.
    pub live: bool,
    pub launched: Evidence,
    pub ledger: Ledger,
    pub beat: Evidence,
}

/// The verdict on one session, the reason a skipped one is skipped included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Restore,
    Live,
    NoServer,
    Unlaunched,
    Stopped,
    Damaged,
    NoBeat,
    Abandoned,
}

/// A stop-result that says the session is gone: `stopped: …` or `already
/// stopped`. A FAILED stop, and anything else, is not — the session may well
/// still be running.
#[must_use]
pub fn is_clean_stop(event: &Event) -> bool {
    event.action == STOP_RESULT_ACTION
        && event.summary.as_deref().is_some_and(|summary| {
            summary == ALREADY_STOPPED_SUMMARY || summary.starts_with(STOPPED_SUMMARY)
        })
}

/// Whether a raw ledger line is the kind resume retention must never drop: the
/// newest clean stop. It asks the reader every consumer shares; a line the
/// reader refuses pins nothing.
#[must_use]
pub fn pins_clean_stop(line: &str) -> bool {
    line.contains(STOP_RESULT_ACTION)
        && Event::parse_line(line).is_ok_and(|event| is_clean_stop(&event))
}

/// What `read` says about a stop since the launch at `launched`.
///
/// A clean stop at or after the launch, or a stop request nothing has answered
/// since (a stop in flight, or one that died), reads as stopped: doubt skips.
/// A request answered by a FAILED result changed nothing. A ledger that could
/// not be read whole — or held a line the reader refused, which may have been
/// the stop — is damaged, never clear.
#[must_use]
pub fn ledger(read: Option<&SessionRead>, launched: i64) -> Ledger {
    let Some(read) = read.filter(|read| read.skipped.is_empty()) else {
        return Ledger::Damaged;
    };
    let since = |event: &&Event| event.ts.epoch() >= launched;
    if read.events.iter().filter(since).any(is_clean_stop) {
        return Ledger::Stopped;
    }
    let asked = read
        .events
        .iter()
        .filter(since)
        .filter(|event| event.action == STOP_REQUEST_ACTION)
        .map(|event| event.ts.epoch())
        .max();
    let unanswered = asked.is_some_and(|at| {
        !read
            .events
            .iter()
            .any(|event| event.action == STOP_RESULT_ACTION && event.ts.epoch() >= at)
    });
    if unanswered {
        Ledger::Stopped
    } else {
        Ledger::Clear
    }
}

/// One verdict per fact, in order.
///
/// The CANDIDATES are the sessions that are not live, were launched, have a
/// readable ledger with no stop since, and beat at or after that launch (an
/// older beat belongs to a previous run, which says nothing about this one).
/// A candidate restores when its beat is within [`WINDOW_SECS`] of the newest
/// candidate beat on the same recorded server. Live, stopped and damaged
/// sessions are not candidates, so a session the human already resumed — whose
/// beat is fresh — cannot push the window past the ones the crash took.
#[must_use]
pub fn judge(facts: &[Fact]) -> Vec<Decision> {
    let gate = |fact: &Fact| -> Result<(Selector, i64), Decision> {
        let ServerSelector::Positive(server) = &fact.server else {
            return Err(Decision::NoServer);
        };
        if fact.live {
            return Err(Decision::Live);
        }
        let Evidence::At(launched) = fact.launched else {
            return Err(Decision::Unlaunched);
        };
        match fact.ledger {
            Ledger::Damaged => return Err(Decision::Damaged),
            Ledger::Stopped => return Err(Decision::Stopped),
            Ledger::Clear => {}
        }
        match fact.beat {
            Evidence::At(beat) if beat >= launched => Ok((server.clone(), beat)),
            _ => Err(Decision::NoBeat),
        }
    };
    let gated: Vec<_> = facts.iter().map(gate).collect();
    let mut newest: BTreeMap<&Selector, i64> = BTreeMap::new();
    for (server, beat) in gated.iter().flatten() {
        let slot = newest.entry(server).or_insert(*beat);
        *slot = (*slot).max(*beat);
    }
    gated
        .iter()
        .map(|gated| match gated {
            Err(why) => *why,
            Ok((server, beat)) => match newest.get(server) {
                Some(top) if top - beat <= WINDOW_SECS => Decision::Restore,
                _ => Decision::Abandoned,
            },
        })
        .collect()
}

/// The facts of one saved session, read from what ae already keeps.
fn fact_of(record: &DurableRecord, live: bool) -> Fact {
    let launched = crate::store::open(&record.path).launch_attempt();
    let epoch = match launched {
        Evidence::At(at) => at,
        _ => 0,
    };
    Fact {
        name: record.name.clone(),
        server: record.server.clone(),
        live,
        launched,
        ledger: ledger(record.snapshot.events.as_ref(), epoch),
        beat: crate::watchdog_glue::beat_modified(&record.path),
    }
}

/// The saved sessions under `roots` that bare `ae` should restore, by name.
/// `live` names the sessions already running, which are never restored.
#[must_use]
pub fn restorable(roots: &Roots, live: &[String]) -> Vec<String> {
    let facts: Vec<Fact> = crate::inventory::durable_records(roots)
        .records
        .iter()
        .filter(|record| record.layout == Layout::Canonical)
        .map(|record| fact_of(record, live.contains(&record.name)))
        .collect();
    let mut names: Vec<String> = facts
        .iter()
        .zip(judge(&facts))
        .filter(|(_, decision)| *decision == Decision::Restore)
        .map(|(fact, _)| fact.name.clone())
        .collect();
    names.sort();
    names
}
