//! Which saved sessions were RUNNING when their recorded tmux server died, for
//! bare `ae` to restore. Facts are what ae already keeps (launch stamp, stop
//! ledger) plus the watchdog's mtime-only beat; nothing persisted is parsed
//! here — the ledger arrives through the shared event reader.

use std::collections::BTreeMap;

use crate::events::Event;
use crate::inventory::{DurableRecord, Layout, Roots};
use crate::lifecycle::{
    ALREADY_STOPPED_SUMMARY, STOP_REQUEST_ACTION, STOP_RESULT_ACTION, STOPPED_SUMMARY,
};
use crate::meta::{Selector, ServerSelector};
use crate::session::SessionRead;
use crate::tmux::Evidence;

/// How far behind its cohort's newest beat a session's beat may sit. Fixed: one
/// crash silences a fleet within a cycle; a longer window resurrects the abandoned.
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

/// A stop-result that says the session is gone. A FAILED one is not.
#[must_use]
pub fn is_clean_stop(event: &Event) -> bool {
    event.action == STOP_RESULT_ACTION
        && event.summary.as_deref().is_some_and(|summary| {
            summary == ALREADY_STOPPED_SUMMARY || summary.starts_with(STOPPED_SUMMARY)
        })
}

/// Whether a raw ledger line is a clean stop, which resume retention keeps.
#[must_use]
pub fn pins_clean_stop(line: &str) -> bool {
    line.contains(STOP_RESULT_ACTION)
        && Event::parse_line(line).is_ok_and(|event| is_clean_stop(&event))
}

/// What `read` says about a stop since the launch at `launched`. A clean stop,
/// or a stop request nothing has answered since, reads as stopped; a request a
/// FAILED result answered changed nothing. A ledger not read whole — a refused
/// line may have been the stop — is damaged, never clear.
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
/// CANDIDATES are sessions not live, launched, with a readable ledger showing no
/// stop since, and a beat at or after that launch (an older one is a previous
/// run's). A candidate restores within [`WINDOW_SECS`] of the newest candidate
/// beat on its recorded server; live sessions are no candidates, so one the
/// human already resumed cannot push the window past the crash's sessions.
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

/// The saved sessions under `roots` to restore, by name; `live` are never.
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
