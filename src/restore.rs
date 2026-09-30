//! Which saved sessions were RUNNING when their recorded tmux server died, for
//! bare `ae` to restore. Facts are what ae already keeps (launch stamp, stop
//! ledger) plus the watchdog's mtime-only beat; nothing persisted is parsed
//! here — the ledger arrives through the shared event reader.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use crate::entry::Preamble;
use crate::events::Event;
use crate::inventory::{DurableRecord, Layout, Roots, ServerId};
use crate::lifecycle::{
    ALREADY_STOPPED_SUMMARY, STOP_REQUEST_ACTION, STOP_RESULT_ACTION, STOPPED_SUMMARY,
};
use crate::meta::{Meta, Selector, ServerSelector};
use crate::session::SessionRead;
use crate::tmux::{Evidence, StopProbe};

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
    Unproven,
}

/// What the recorded server says about a session that passed every other gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proof {
    Absent,
    Present,
    Unknown,
}

/// What a restore carries from the scan to the lifecycle lock: the session's
/// own facts and the cohort's window edge, re-read there and never recomputed.
#[derive(Debug, Clone)]
pub struct Guard {
    pub launched: i64,
    pub cutoff: i64,
    pub server: Selector,
}

impl Guard {
    /// Why a restore must not go on, read afresh from `dir` under the lock.
    #[must_use]
    pub fn refuse(&self, dir: &Path) -> Option<&'static str> {
        let Ok(bytes) = crate::meta::read_bytes(dir) else {
            return Some("saved record unreadable");
        };
        let server = Meta::parse(&String::from_utf8_lossy(&bytes)).server_selector();
        if server != ServerSelector::Positive(self.server.clone()) {
            return Some("recorded server changed since the scan");
        }
        if crate::store::open(dir).launch_attempt() != Evidence::At(self.launched) {
            return Some("launched again since the scan");
        }
        match ledger(SessionRead::open(dir).ok().as_ref(), self.launched) {
            Ledger::Clear => {}
            Ledger::Stopped => return Some("stopped through ae since its launch"),
            Ledger::Damaged => return Some("stop ledger unreadable"),
        }
        match crate::watchdog_glue::beat_modified(dir) {
            Evidence::At(beat) if beat >= self.launched && beat >= self.cutoff => None,
            _ => Some("watchdog beat missing or outside the crashed cohort window"),
        }
    }
}

/// One saved session's scan result: the verdict, the reason a proof gap gives,
/// and — for a restore — the guard the lock will re-check.
#[derive(Debug, Clone)]
pub struct Verdict {
    pub name: String,
    pub decision: Decision,
    pub why: Option<String>,
    pub guard: Option<Guard>,
}

/// `names` in restore order: the human's fleet order, then by name.
#[must_use]
pub fn order(mut names: Vec<String>, place: impl Fn(&str) -> usize) -> Vec<String> {
    names.sort_by(|a, b| place(a).cmp(&place(b)).then_with(|| a.cmp(b)));
    names
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

/// What `read` says about a stop since the launch at `launched`. Read in ledger
/// ORDER, because a second is too coarse to order two events inside it: a clean
/// stop, or a request no LATER result answered, reads as stopped. A FAILED
/// result answers the request before it and changes nothing. A ledger not read
/// whole — a refused line may have been the stop — is damaged, never clear.
#[must_use]
pub fn ledger(read: Option<&SessionRead>, launched: i64) -> Ledger {
    let Some(read) = read.filter(|read| read.skipped.is_empty()) else {
        return Ledger::Damaged;
    };
    let mut asked = false;
    for event in read.events.iter().filter(|e| e.ts.epoch() >= launched) {
        if is_clean_stop(event) {
            return Ledger::Stopped;
        }
        match event.action.as_str() {
            STOP_REQUEST_ACTION => asked = true,
            STOP_RESULT_ACTION => asked = false,
            _ => {}
        }
    }
    if asked {
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
pub fn judge(facts: &[Fact], mut prove: impl FnMut(usize) -> Proof) -> Vec<Decision> {
    let mut gate = |index: usize, fact: &Fact| -> Result<(Selector, i64), Decision> {
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
            Evidence::At(beat) if beat >= launched => match prove(index) {
                Proof::Absent => Ok((server.clone(), beat)),
                Proof::Present => Err(Decision::Live),
                Proof::Unknown => Err(Decision::Unproven),
            },
            _ => Err(Decision::NoBeat),
        }
    };
    let gated: Vec<_> = facts
        .iter()
        .enumerate()
        .map(|(index, fact)| gate(index, fact))
        .collect();
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

/// One verdict per saved session under `roots`, in the scan's path order. A name
/// its recorded server lists is live; one it answers without is proven absent;
/// only when that server gave no listing does the launch's own proof decide.
#[must_use]
pub fn restorable(roots: &Roots) -> Vec<Verdict> {
    let scan = crate::inventory::durable_records(roots);
    let records: Vec<&DurableRecord> = scan
        .records
        .iter()
        .filter(|record| record.layout == Layout::Canonical)
        .collect();
    let mut listed: BTreeMap<Selector, Option<Vec<String>>> = BTreeMap::new();
    for record in &records {
        if let ServerSelector::Positive(server) = &record.server {
            listed.entry(server.clone()).or_insert_with(|| {
                crate::transport::session_names(&ServerId::Selected(server.clone()))
            });
        }
    }
    let facts: Vec<Fact> = records
        .iter()
        .map(|record| {
            let names = match &record.server {
                ServerSelector::Positive(server) => listed.get(server).and_then(Option::as_ref),
                _ => None,
            };
            fact_of(
                record,
                names.is_some_and(|names| names.contains(&record.name)),
            )
        })
        .collect();
    let mut gaps: BTreeMap<usize, String> = BTreeMap::new();
    let decisions = judge(&facts, |index| {
        let record = records[index];
        let ServerSelector::Positive(server) = &record.server else {
            return Proof::Unknown;
        };
        if listed.get(server).is_some_and(Option::is_some) {
            return Proof::Absent;
        }
        let recorded = ServerId::Selected(server.clone());
        match crate::session_launch::resume_absence(&recorded, &record.name, &record.path) {
            (StopProbe::Absent, _) => Proof::Absent,
            (StopProbe::Present, _) => Proof::Present,
            (StopProbe::Unknown, why) => {
                gaps.insert(index, why.unwrap_or_default());
                Proof::Unknown
            }
        }
    });
    let mut top: BTreeMap<&Selector, i64> = BTreeMap::new();
    for (fact, decision) in facts.iter().zip(&decisions) {
        if let (Decision::Restore, ServerSelector::Positive(server), Evidence::At(beat)) =
            (decision, &fact.server, fact.beat)
        {
            let slot = top.entry(server).or_insert(beat);
            *slot = (*slot).max(beat);
        }
    }
    let mut verdicts = Vec::with_capacity(facts.len());
    for (index, (fact, decision)) in facts.iter().zip(decisions).enumerate() {
        let guard = match (decision, &fact.server, fact.launched) {
            (Decision::Restore, ServerSelector::Positive(server), Evidence::At(launched)) => {
                top.get(server).map(|top| Guard {
                    launched,
                    cutoff: top - WINDOW_SECS,
                    server: server.clone(),
                })
            }
            _ => None,
        };
        let why = gaps.remove(&index);
        verdicts.push(Verdict {
            name: fact.name.clone(),
            decision,
            why,
            guard,
        });
    }
    verdicts
}

/// A restore launch in flight: its guard, and the skip the lock-time checks
/// decided, which is an outcome and not a failure.
pub struct Attempt {
    pub guard: Guard,
    skipped: Cell<Option<String>>,
}

impl Attempt {
    pub fn skip(&self, why: impl Into<String>) {
        self.skipped.set(Some(why.into()));
    }
}

/// What a restore pass did.
#[derive(Default)]
pub struct Report {
    pub restored: usize,
    pub failed: usize,
}

/// How a proof gap reads in a skip line, at the scan and under the lock alike.
#[must_use]
pub fn unproven(why: Option<&str>) -> String {
    format!(
        "recorded server unproven: {}",
        why.unwrap_or("no reason given")
    )
}

/// The stderr line a session that is not restored gets.
fn skip_line(verdict: &Verdict) -> Option<String> {
    let why = match verdict.decision {
        Decision::Restore | Decision::Live => return None,
        Decision::NoServer => "no recorded tmux server".to_owned(),
        Decision::Unlaunched => "no launch record".to_owned(),
        Decision::Stopped => "stopped through ae".to_owned(),
        Decision::Damaged => "stop ledger unreadable".to_owned(),
        Decision::NoBeat => "no watchdog beat since its last launch".to_owned(),
        Decision::Abandoned => "watchdog beat older than the crashed cohort window".to_owned(),
        Decision::Unproven => unproven(verdict.why.as_deref()),
    };
    Some(format!("ae: skipped {}: {why}", verdict.name))
}

/// Restore every `Restore` verdict, one session at a time, and say so as it
/// goes: the progress line is flushed BEFORE the launch takes its lock.
///
/// # Errors
///
/// Only a failure to write `out` or `err`; a failed launch is counted.
pub fn run(
    preamble: &Preamble,
    verdicts: &[Verdict],
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<Report> {
    let restoring = verdicts.iter().any(|v| v.guard.is_some());
    for verdict in verdicts
        .iter()
        .filter(|v| restoring || v.decision == Decision::Unproven)
    {
        if let Some(line) = skip_line(verdict) {
            writeln!(err, "{line}")?;
        }
    }
    let fleet = crate::fleet_order();
    let due = verdicts.iter().filter(|v| v.guard.is_some());
    let names = order(due.map(|v| v.name.clone()).collect(), |name| {
        fleet.place(name)
    });
    let mut report = Report::default();
    for name in names {
        let Some(guard) = verdicts
            .iter()
            .find(|v| v.name == name)
            .and_then(|v| v.guard.clone())
        else {
            continue;
        };
        let label = crate::tmux_floor::server_label(&ServerId::Selected(guard.server.clone()));
        writeln!(out, "ae: restoring {name} ({label})")?;
        out.flush()?;
        let attempt = Attempt {
            guard,
            skipped: Cell::new(None),
        };
        let (mut launch_out, mut launch_err) = (Vec::new(), Vec::new());
        let code = crate::session_launch::run_restore(
            preamble,
            &name,
            &attempt,
            &mut launch_out,
            &mut launch_err,
        )?;
        match (code, attempt.skipped.take()) {
            (0, None) => {
                writeln!(out, "ae: restored {name} ({label})")?;
                report.restored += 1;
            }
            (0, Some(why)) => writeln!(err, "ae: skipped {name}: {why}")?,
            _ => {
                report.failed += 1;
                writeln!(err, "ae: restore failed {name}:")?;
                for line in String::from_utf8_lossy(&launch_err).lines() {
                    writeln!(err, "  {line}")?;
                }
            }
        }
    }
    Ok(report)
}
