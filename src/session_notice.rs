//! Session notices: the session watchdog tells a running seat that its session
//! was renamed or its goal changed, in text built from the CURRENT meta.
//!
//! `rename` (actor `ae:rename`) and `goal` records arm a notice; the pure fold
//! [`due`] reads the journal in ORDER per seat. The daemon triggers the leg
//! ([`run`]) through the send helper. The leg folds again under the target
//! lock, journals the attempt BEFORE it reads the meta the text comes from, and
//! journals the outcome on the attempt's ref: a later source re-arms the seat,
//! an earlier one is already in the text. Nothing reaches the pane from argv.
use std::io::{self, Write};
use std::path::Path;

use crate::deliver::{Delivered, Failure};
use crate::events::{Event, RoutingMember};
use crate::time::Timestamp;

/// Selects the leg in the send helper; the trigger journals nothing.
pub const TRIGGER_ACTION: &str = "session-notice-due";
/// One attempt, journaled before the text is built: the consumed point.
pub const ATTEMPT_ACTION: &str = "session-notice-attempt";
/// The attempt landed (`[unconfirmed]` when the submit was not proven).
pub const TOLD_ACTION: &str = "session-notice";
/// The attempt failed after it was composed.
pub const FAILED_ACTION: &str = "session-notice-failed";
/// The third failure since its source: nothing more until a newer source.
pub const GAVE_UP_ACTION: &str = "session-notice-gave-up";
/// The rename source record's action.
pub const RENAME_ACTION: &str = "rename";
/// The rename source record's actor.
pub const RENAME_ACTOR: &str = "ae:rename";
const MAX_FAILURES: usize = 3;
/// The leg's quiet-gate wait: short, because the next cycle looks again.
const DEFER: std::time::Duration = std::time::Duration::from_secs(2);
/// What a rename or a goal change prints when the watchdog runs.
pub const TOLD_LINE: &str = "Running seats are told by the session watchdog within one cycle (events-tail: session-notice).";

/// What a notice tells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Name and paths changed; the text carries the goal too.
    Rename,
    /// The goal changed.
    Goal,
}

impl Kind {
    const fn word(self) -> &'static str {
        match self {
            Self::Rename => "rename",
            Self::Goal => "goal",
        }
    }
}

/// The facts the fold judges one seat by, all from one meta read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seat {
    /// The roster name: a `spawn` record's target.
    pub name: String,
    /// The slot: a `reseat`/`relaunch` record's `target_slot`.
    pub slot: String,
    /// This session's name.
    pub session: String,
    /// This session's meta `session_id`, or empty.
    pub session_id: String,
    /// `launch_id.<slot>`, `-` when absent: the incarnation an attempt names.
    pub launch_id: String,
    /// `launch_time.<slot>`, read only when no launch record bounds the seat.
    pub launch_time: Option<i64>,
    /// Main, or the colead of a lead pair: the seats a goal change reaches.
    pub standing: bool,
}

impl Seat {
    /// The seat `slot` holds, from `bytes` and `meta`, their parse.
    #[must_use]
    pub fn read(meta: &crate::meta::Meta, bytes: &[u8], slot: &str, session: &str) -> Self {
        let value = |key: &str| crate::lifecycle::meta_value(bytes, key);
        let launch_id = value(&format!("launch_id.{slot}"));
        let entry = meta.roster().iter().find(|entry| entry.slot == slot);
        Self {
            name: entry.map(|entry| entry.name.clone()).unwrap_or_default(),
            slot: slot.to_owned(),
            session: session.to_owned(),
            session_id: meta.session_id().unwrap_or_default().to_owned(),
            launch_id: if launch_id.is_empty() {
                "-".to_owned()
            } else {
                launch_id
            },
            launch_time: value(&format!("launch_time.{slot}")).parse().ok(),
            standing: crate::watchdog_daemon::in_lead_pair(slot, value("layout") == "lead-pair"),
        }
    }
}

/// A notice owed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Due {
    /// Rename when one is owed, else goal.
    pub kind: Kind,
    /// Failed attempts of `kind` since its newest source.
    pub failures: usize,
    /// The old names of the renames owed, oldest first, grammar-proven.
    pub old: Vec<String>,
    /// The third failed attempt's ref while its `gave-up` is not journaled.
    pub give_up: Option<String>,
}

/// One counted attempt: this seat, this incarnation, after its boundary.
/// `told`: `Some(true)` told, `Some(false)` failed, `None` no ending yet.
struct Attempt {
    at: usize,
    kind: Kind,
    told: Option<bool>,
}

/// Whether `event` launched `seat` into fresh context: a spawn names its
/// display, a reseat or relaunch its slot.
fn launched(event: &Event, seat: &Seat) -> bool {
    match event.action.as_str() {
        "spawn" => event.target.as_deref() == Some(seat.name.as_str()),
        "reseat" | "relaunch" => event.target_slot == RoutingMember::Value(seat.slot.clone()),
        _ => false,
    }
}

/// Whether a setter keyed `slot` + `id` is `seat`: both proven and equal.
/// Anything less excludes nobody.
fn is_seat(slot: &str, id: &str, seat: &Seat) -> bool {
    !id.is_empty() && slot == seat.slot && id == seat.session_id
}

/// The old name a rename record's `<old> -> <new>` summary names.
fn old_name(event: &Event) -> Option<String> {
    let (old, _) = event.summary.as_deref()?.split_once(" -> ")?;
    crate::session_launch::name::is_session_name(old).then(|| old.to_owned())
}

/// What the journal owes `seat` at `now`, or `None`. `bound` is how long an
/// attempt with no ending stays in flight (two daemon intervals).
#[must_use]
pub fn due(events: &[Event], seat: &Seat, now: i64, bound: i64) -> Option<Due> {
    let boundary = events.iter().rposition(|event| launched(event, seat));
    let after = |at: usize, line: Option<usize>| line.is_none_or(|line| at > line);
    // No launch record: only a source STRICTLY newer than the launch time.
    let fresh = |at: usize| match boundary {
        Some(line) => at > line,
        None => seat
            .launch_time
            .is_none_or(|time| events[at].ts.epoch() > time),
    };
    let slot = RoutingMember::Value(seat.slot.clone());
    let attempts: Vec<Attempt> = events
        .iter()
        .enumerate()
        .filter(|(at, event)| {
            event.action == ATTEMPT_ACTION && after(*at, boundary) && event.target_slot == slot
        })
        .filter_map(|(at, event)| {
            let (word, launch) = event.summary.as_deref()?.split_once(' ')?;
            let kind = [Kind::Rename, Kind::Goal]
                .into_iter()
                .find(|kind| kind.word() == word)?;
            let reference = event.reference.as_deref()?;
            let told = events[at + 1..]
                .iter()
                .filter(|end| end.reference.as_deref() == Some(reference))
                .find_map(|end| match end.action.as_str() {
                    TOLD_ACTION => Some(true),
                    FAILED_ACTION => Some(false),
                    _ => None,
                });
            (launch == seat.launch_id).then_some(Attempt { at, kind, told })
        })
        .collect();
    if attempts.last().is_some_and(|last| {
        last.told.is_none() && events[last.at].ts.epoch().saturating_add(bound) > now
    }) {
        return None;
    }
    let told = |kinds: &[Kind]| {
        attempts
            .iter()
            .filter(|attempt| attempt.told == Some(true) && kinds.contains(&attempt.kind))
            .map(|attempt| attempt.at)
            .max()
    };
    let sources = |action: &str, floor: Option<usize>| -> Vec<usize> {
        (0..events.len())
            .filter(|at| {
                let event = &events[*at];
                event.action == action
                    && (action != RENAME_ACTION || event.actor == RENAME_ACTOR)
                    && after(*at, floor)
                    && fresh(*at)
            })
            .collect()
    };
    let renames = sources(RENAME_ACTION, boundary.max(told(&[Kind::Rename])));
    let (kind, source, old) = if let Some(&source) = renames.last() {
        let old = renames.iter().filter_map(|at| old_name(&events[*at]));
        (Kind::Rename, source, old.collect::<Vec<_>>())
    } else {
        let goals = sources("goal", boundary.max(told(&[Kind::Rename, Kind::Goal])));
        let source = *goals.last()?;
        let setter = &events[source];
        let own = match (&setter.actor_slot, &setter.actor_session_id) {
            (RoutingMember::Value(slot), RoutingMember::Value(id)) => is_seat(slot, id, seat),
            _ => false,
        };
        if !seat.standing || own {
            return None;
        }
        (Kind::Goal, source, Vec::new())
    };
    let failed: Vec<usize> = attempts
        .iter()
        .filter(|attempt| {
            attempt.kind == kind && attempt.at > source && attempt.told == Some(false)
        })
        .map(|attempt| attempt.at)
        .collect();
    // The ceiling stays owed, as a booking and never a paste, until its record lands.
    let give_up = failed
        .get(MAX_FAILURES - 1)
        .and_then(|at| events[*at].reference.clone());
    let booked = |third: &String| {
        events
            .iter()
            .any(|event| event.action == GAVE_UP_ACTION && event.reference.as_ref() == Some(third))
    };
    (!give_up.as_ref().is_some_and(booked)).then_some(Due {
        kind,
        failures: failed.len(),
        old,
        give_up,
    })
}

/// The rename notice: the chain, the new address and paths, what no longer
/// works, the re-read, the new reply spelling and the goal. It names no old helper
/// DIRECTORY — only the short forms that stopped working.
#[must_use]
pub fn rename_text(old: &[String], name: &str, dir: &Path, goal: &str) -> String {
    let dir = dir.display();
    // Each old name once, in chain order; a rename back to one still works.
    let gone: Vec<&str> = (0..old.len())
        .filter(|at| old[*at] != name && !old[..*at].contains(&old[*at]))
        .map(|at| old[at].as_str())
        .collect();
    let chain = if old.is_empty() {
        format!("session renamed to {name}.")
    } else {
        format!("session renamed: {} -> {name}.", old.join(" -> "))
    };
    let gone = if gone.is_empty() {
        "Old ae @<session> forms and old helper paths".to_owned()
    } else {
        format!("ae @{} and the old helper paths", gone.join(", ae @"))
    };
    let goal = if goal.is_empty() { "none" } else { goal };
    format!(
        "{chain} {gone} no longer work.\nhelpers now: ae @{name} <helper>, or {dir}/<helper>.\npending requests survive: a reply command quoted earlier now reads ae @{name} reply <same id>.\nre-read {dir}/workspace.md now: roster and paths.\ngoal: {goal}"
    )
}

/// The goal notice.
#[must_use]
pub fn goal_text(goal: &str) -> String {
    if goal.is_empty() {
        "session goal cleared.".to_owned()
    } else {
        format!("session goal now: {goal}")
    }
}

/// The line a caller prints when running seats were NOT told.
#[must_use]
pub fn untold_line(why: &str, seats: usize) -> String {
    format!("{why}: {seats} running seats NOT told — relaunch them or tell them.")
}

/// The line a changed goal prints, `None` when only its setter stands.
#[must_use]
pub fn after_goal(dir: &Path, setter: &crate::requests::Viewer) -> Option<String> {
    let bytes = crate::meta::read_bytes(dir).ok()?;
    let meta = crate::meta::Meta::parse(&String::from_utf8_lossy(&bytes));
    let seats = meta
        .roster()
        .iter()
        .map(|entry| Seat::read(&meta, &bytes, &entry.slot, ""))
        .filter(|seat| seat.standing && !is_seat(&setter.slot, &setter.session_id, seat))
        .count();
    let watched = crate::session_launch::watchdog_enabled_for_session(dir);
    (seats > 0).then(|| {
        if watched {
            TOLD_LINE.to_owned()
        } else {
            untold_line("watchdog off", seats)
        }
    })
}

/// The seat `slot` holds and the fold over a FRESH journal read.
fn judge(dir: &Path, slot: &str, session: &str, bound: i64) -> (Seat, Option<Due>) {
    let bytes = crate::meta::read_bytes(dir).unwrap_or_default();
    let meta = crate::meta::Meta::parse(&String::from_utf8_lossy(&bytes));
    let seat = Seat::read(&meta, &bytes, slot, session);
    let events = crate::watchdog_daemon::read_events(dir);
    let owed = due(&events, &seat, Timestamp::now().epoch(), bound);
    (seat, owed)
}

/// Append one leg record for `seat`.
fn record(
    dir: &Path,
    action: &str,
    seat: &Seat,
    reference: &str,
    summary: &str,
    body_file: &str,
) -> Result<(), crate::store::Error> {
    let fields = crate::tracked::EventFields {
        target_session_id: &seat.session_id,
        ..crate::tracked::EventFields::new(
            Timestamp::now(),
            crate::watchdog::WATCHDOG_ACTOR,
            action,
            &seat.name,
            reference,
            "",
            "",
            &seat.slot,
            &seat.session,
            summary,
            body_file,
        )
    };
    crate::store::open(dir).append_event(&crate::tracked::event_line(&fields))
}

/// Whether the seat's own frame proves an idle box. Only a tool whose frame
/// the classifier reads is asked; the quiet gate judged the rest.
fn idle(server: &crate::inventory::ServerId, pane: &str, dir: &Path, slot: &str) -> bool {
    use crate::harness_state::{HarnessState, classify, observable};
    let tool = crate::tool::ToolKind::from_binary_name(&crate::deliver::recorded_binary(dir, slot));
    !observable(tool)
        || crate::transport::capture_pane(server, pane)
            .is_some_and(|capture| classify(&capture, tool) == HarnessState::Idle)
}

/// The leg: tell `target` what is due, if anything. `interval` is the
/// daemon's cycle in seconds (`_AE_EVENT_SUMMARY`), the in-flight bound's unit.
///
/// # Errors
///
/// Only a failure to write `err`.
pub fn run(
    dir: &Path,
    target: &str,
    own_session: &str,
    interval: Option<&str>,
    err: &mut impl Write,
) -> io::Result<u8> {
    use crate::state::EXIT_FAILED;
    let (resolved, server) = match crate::tracked::resolve_on(target, own_session, dir) {
        Ok(resolved) => resolved,
        Err(why) => {
            writeln!(err, "{}", why.message())?;
            return Ok(EXIT_FAILED);
        }
    };
    if resolved.slot.is_empty() || !(resolved.session.is_empty() || resolved.session == own_session)
    {
        writeln!(
            err,
            "ae: {TRIGGER_ACTION} refused — {target} is no seat here"
        )?;
        return Ok(EXIT_FAILED);
    }
    let interval = interval
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(crate::watchdog_daemon::DEFAULT_INTERVAL_SECS.cast_signed());
    let bound = interval.saturating_mul(2);
    let Some(pre) = judge(dir, &resolved.slot, own_session, bound).1 else {
        return Ok(0);
    };
    if pre.give_up.is_some() {
        if let Some(why) = give_up(dir, &resolved.slot, &resolved.pane, own_session, bound) {
            writeln!(err, "ae: {GAVE_UP_ACTION} {target} not journaled: {why}")?;
        }
        return Ok(EXIT_FAILED);
    }
    let reference = crate::tracked::request_id("sn", Timestamp::now(), entropy());
    let request = crate::deliver::Request {
        dir,
        server: &server,
        pane: &resolved.pane,
        logged_target: target,
        target_session: own_session,
        pane_slot: &resolved.slot,
        own_session,
        action: TOLD_ACTION,
        reference: &reference,
        actor: crate::watchdog::WATCHDOG_ACTOR,
        body: "",
        shape: crate::deliver::Shape::Context,
        defer: DEFER,
        composed: crate::tool::Composed::NONE,
    };
    let mut attempted = None;
    let mut fault = None;
    let compose = || {
        if !idle(&server, &resolved.pane, dir, &resolved.slot) {
            return None;
        }
        let goal = || crate::store::open(dir).goal();
        match attempt(dir, &resolved.slot, own_session, &reference, bound, goal) {
            Ok(Some((seat, owed, text))) => {
                attempted = Some((seat, owed.kind, text.as_ref().err().copied()));
                text.ok()
            }
            Ok(None) => None,
            Err(why) => {
                fault = Some(why);
                None
            }
        }
    };
    let outcome = crate::deliver::deliver_composed(&request, compose, err)?;
    if let Some(why) = fault {
        writeln!(
            err,
            "ae: {ATTEMPT_ACTION} for {target} not journaled: {why}"
        )?;
    }
    let Some((seat, kind, unbuilt)) = attempted else {
        return Ok(EXIT_FAILED);
    };
    // Attempted and nothing composed: the text was never built, nothing pasted.
    let outcome = outcome.ok_or(unbuilt.unwrap_or("refused"));
    finish(dir, &seat, kind, &reference, outcome, err)
}

/// Book the ceiling once under the target lock, on the third failure's ref:
/// no attempt, nothing pasted. `Some(why)`: the record was not written.
fn give_up(dir: &Path, slot: &str, pane: &str, session: &str, bound: i64) -> Option<String> {
    let _held = crate::deliver::lock_target(dir, pane, std::time::Duration::ZERO)?;
    let (seat, owed) = judge(dir, slot, session, bound);
    let (kind, third) = owed.and_then(|owed| Some((owed.kind, owed.give_up?)))?;
    let launch = format!("{} {}", kind.word(), seat.launch_id);
    let written = record(dir, GAVE_UP_ACTION, &seat, &third, &launch, "");
    written.err().map(|why| why.to_string())
}

/// A journaled attempt: the seat, what it owed, its text or why none.
type Attempted = (Seat, Due, Result<String, &'static str>);

/// Under the target lock: a fresh fold, the attempt journaled, THEN the goal
/// read (a later source re-arms). `Err`: no attempt; the text's `Err`: no text.
fn attempt(
    dir: &Path,
    slot: &str,
    session: &str,
    reference: &str,
    bound: i64,
    goal: impl FnOnce() -> io::Result<Option<Vec<u8>>>,
) -> Result<Option<Attempted>, crate::store::Error> {
    let (seat, owed) = judge(dir, slot, session, bound);
    let Some(owed) = owed.filter(|owed| owed.give_up.is_none()) else {
        return Ok(None);
    };
    let launch = format!("{} {}", owed.kind.word(), seat.launch_id);
    record(dir, ATTEMPT_ACTION, &seat, reference, &launch, "")?;
    // An unreadable meta is never "no goal": the attempt fails on its ref.
    let text = goal().map_err(|_| "meta-unread").map(|goal| {
        let goal = crate::goal::printable(&String::from_utf8_lossy(&goal.unwrap_or_default()));
        match owed.kind {
            Kind::Rename => rename_text(&owed.old, session, dir, &goal),
            Kind::Goal => goal_text(&goal),
        }
    });
    Ok(Some((seat, owed, text)))
}

/// Journal what the attempt came to; `Err(why)`: nothing was composed after it.
fn finish(
    dir: &Path,
    seat: &Seat,
    kind: Kind,
    reference: &str,
    outcome: Result<Result<Delivered, Failure>, &str>,
    err: &mut impl Write,
) -> io::Result<u8> {
    let kind = kind.word();
    let unconfirmed = format!("{}{kind}", crate::tracked::UNCONFIRMED_SUMMARY_PREFIX);
    let failed =
        |why: &str, body_file: &str| (FAILED_ACTION, format!("{kind} {why}"), body_file.to_owned());
    let (action, summary, body_file) = match outcome {
        Ok(Ok(landed)) if landed.verification.unverifiable_marker().is_none() => {
            (TOLD_ACTION, kind.to_owned(), landed.body_file)
        }
        // The box may hold the text or it may have landed: told, never twice.
        Ok(Ok(Delivered { body_file, .. }) | Err(Failure::Unconfirmed { body_file, .. })) => {
            (TOLD_ACTION, unconfirmed, body_file)
        }
        Ok(Err(failure)) => failed(
            match &failure {
                Failure::Storage => "storage",
                Failure::Abandoned { .. } => "busy",
                Failure::Paste { .. } => "paste",
                _ => "refused",
            },
            failure.body_file(),
        ),
        Err(why) => failed(why, ""),
    };
    if let Err(why) = record(dir, action, seat, reference, &summary, &body_file) {
        writeln!(err, "ae: {action} for {} not journaled: {why}", seat.name)?;
    }
    Ok(if action == TOLD_ACTION {
        0
    } else {
        crate::state::EXIT_FAILED
    })
}

/// Per-call entropy for the attempt ref.
fn entropy() -> u64 {
    use std::hash::{BuildHasher as _, RandomState};
    RandomState::new().hash_one((std::process::id(), Timestamp::now().epoch()))
}

#[cfg(test)]
mod tests {
    use super::{Due, Kind, Seat, attempt, due, finish, give_up, goal_text, rename_text};
    use crate::events::Event;

    const NOW: i64 = 1_791_000_000;

    /// A fold case: why, the journal, the expected kind and failures.
    type Case<'a> = (&'a str, Vec<(&'a str, i64, &'a str)>, Option<(Kind, usize)>);

    fn seat() -> Seat {
        Seat {
            name: "lead".to_owned(),
            slot: "main".to_owned(),
            session: "s".to_owned(),
            session_id: "A".to_owned(),
            launch_id: "tok".to_owned(),
            launch_time: Some(0),
            standing: true,
        }
    }

    /// One journal line: `action` at `NOW - age`, `extra` JSON members appended.
    fn line(action: &str, age: i64, extra: &str) -> String {
        let ts = crate::time::Timestamp::from_epoch(NOW - age);
        let actor = if action == "rename" { "ae:rename" } else { "x" };
        format!(r#"{{"ts":"{ts}","actor":"{actor}","action":"{action}"{extra}}}"#)
    }

    fn parse(lines: &[String]) -> Vec<Event> {
        lines
            .iter()
            .map(|line| Event::parse_line(line).unwrap())
            .collect()
    }

    fn journal(lines: &[(&str, i64, &str)]) -> Vec<Event> {
        let lines: Vec<String> = lines.iter().map(|(a, age, e)| line(a, *age, e)).collect();
        parse(&lines)
    }

    fn attempt_json(kind: &str, token: &str, reference: &str) -> String {
        format!(r#","target_slot":"main","ref":"{reference}","summary":"{kind} {token}""#)
    }

    /// A goal, then three failed goal attempts `f0`..`f2` of this incarnation.
    fn failed_thrice() -> Vec<String> {
        let mut lines = vec![line("goal", 50, "")];
        for index in 0..3 {
            let reference = format!("f{index}");
            let attempted = attempt_json("goal", "tok", &reference);
            lines.push(line("session-notice-attempt", 40, &attempted));
            lines.push(line("session-notice-failed", 40, &ended(&reference)));
        }
        lines
    }

    fn ended(reference: &str) -> String {
        format!(r#","ref":"{reference}""#)
    }

    fn kind(events: &[Event], seat: &Seat) -> Option<(Kind, usize)> {
        due(events, seat, NOW, 120).map(|Due { kind, failures, .. }| (kind, failures))
    }

    #[test]
    fn the_fold_owes_the_newest_source_after_the_seats_told_point() {
        let s = seat();
        let a1 = attempt_json("goal", "tok", "r1");
        let r1 = ended("r1");
        let rn = attempt_json("rename", "tok", "r2");
        let r2 = ended("r2");
        let foreign = attempt_json("rename", "old-tok", "r3");
        let r3 = ended("r3");
        let cases: Vec<Case<'_>> = vec![
            ("nothing", vec![], None),
            ("rename", vec![("rename", 5, "")], Some((Kind::Rename, 0))),
            ("goal", vec![("goal", 5, "")], Some((Kind::Goal, 0))),
            (
                "goal told",
                vec![
                    ("goal", 5, ""),
                    ("session-notice-attempt", 4, &a1),
                    ("session-notice", 4, &r1),
                ],
                None,
            ),
            (
                "goal told never clears a rename",
                vec![
                    ("rename", 5, ""),
                    ("session-notice-attempt", 4, &a1),
                    ("session-notice", 4, &r1),
                ],
                Some((Kind::Rename, 0)),
            ),
            (
                "rename told clears an earlier goal",
                vec![
                    ("goal", 6, ""),
                    ("rename", 5, ""),
                    ("session-notice-attempt", 4, &rn),
                    ("session-notice", 4, &r2),
                ],
                None,
            ),
            (
                "a source after the attempt re-arms",
                vec![
                    ("goal", 6, ""),
                    ("session-notice-attempt", 5, &a1),
                    ("goal", 4, ""),
                    ("session-notice", 3, &r1),
                ],
                Some((Kind::Goal, 0)),
            ),
            (
                "another incarnation's told point is not this seat's",
                vec![
                    ("rename", 5, ""),
                    ("session-notice-attempt", 4, &foreign),
                    ("session-notice", 4, &r3),
                ],
                Some((Kind::Rename, 0)),
            ),
            (
                "fresh bare attempt is in flight",
                vec![("goal", 9, ""), ("session-notice-attempt", 5, &a1)],
                None,
            ),
            (
                "expired bare attempt is retried",
                vec![("goal", 900, ""), ("session-notice-attempt", 800, &a1)],
                Some((Kind::Goal, 0)),
            ),
            (
                "a failure re-arms and is counted",
                vec![
                    ("goal", 5, ""),
                    ("session-notice-attempt", 4, &a1),
                    ("session-notice-failed", 4, &r1),
                ],
                Some((Kind::Goal, 1)),
            ),
        ];
        for (why, lines, expected) in cases {
            assert_eq!(kind(&journal(&lines), &s), expected, "{why}");
        }
    }

    #[test]
    fn launch_boundaries_setters_and_standing() {
        let s = seat();
        let spawn = r#","target":"lead""#;
        let other = r#","target":"colead""#;
        let reseat = r#","target_slot":"main""#;
        assert_eq!(
            kind(&journal(&[("rename", 5, ""), ("spawn", 5, spawn)]), &s),
            None
        );
        assert!(kind(&journal(&[("rename", 5, ""), ("spawn", 5, other)]), &s).is_some());
        assert_eq!(
            kind(&journal(&[("rename", 5, ""), ("relaunch", 5, reseat)]), &s),
            None
        );
        assert!(kind(&journal(&[("reseat", 5, reseat), ("rename", 5, "")]), &s).is_some());
        let legacy = Seat {
            launch_time: Some(NOW - 5),
            ..seat()
        };
        assert_eq!(
            kind(&journal(&[("rename", 5, "")]), &legacy),
            None,
            "same second"
        );
        assert!(kind(&journal(&[("rename", 4, "")]), &legacy).is_some());
        let own = r#","actor_slot":"main","actor_session_id":"A""#;
        let peer = r#","actor_slot":"main","actor_session_id":"B""#;
        let unproven = r#","actor_slot":"main""#;
        assert_eq!(kind(&journal(&[("goal", 5, own)]), &s), None);
        assert!(kind(&journal(&[("goal", 5, peer)]), &s).is_some());
        assert!(kind(&journal(&[("goal", 5, unproven)]), &s).is_some());
        let spawned = Seat {
            standing: false,
            ..seat()
        };
        assert_eq!(kind(&journal(&[("goal", 5, "")]), &spawned), None);
        assert!(kind(&journal(&[("rename", 5, "")]), &spawned).is_some());
    }

    #[test]
    fn the_texts_name_the_new_address_and_never_an_old_directory() {
        let told = attempt_json("rename", "tok", "r1");
        let end = ended("r1");
        let mut lines = vec![
            ("rename", 9, r#","summary":"old -> middle""#),
            ("rename", 8, r#","summary":"bad/name -> x""#),
            ("rename", 7, r#","summary":"middle -> newest""#),
        ];
        let old = due(&journal(&lines), &seat(), NOW, 120).unwrap().old;
        assert_eq!(old, ["old", "middle"]);
        lines.splice(
            1..1,
            [
                ("session-notice-attempt", 9, told.as_str()),
                ("session-notice", 9, end.as_str()),
            ],
        );
        let owed = due(&journal(&lines), &seat(), NOW, 120).unwrap();
        assert_eq!(owed.old, ["middle"], "a told rename leaves the chain");
        let dir = std::path::Path::new("/st/sessions/newest");
        let text = rename_text(&old, "newest", dir, "ship");
        for fact in [
            "old -> middle -> newest",
            "ae @old, ae @middle",
            "no longer work",
            "ae @newest",
            "re-read /st/sessions/newest/workspace.md now",
            "quoted earlier now reads ae @newest reply <same id>",
            "goal: ship",
        ] {
            assert!(text.contains(fact), "{fact}: {text}");
        }
        assert!(!text.contains("/st/sessions/old") && !text.contains("/st/sessions/middle"));
        let back = rename_text(&["a".to_owned(), "b".to_owned()], "a", dir, "");
        assert!(
            back.contains("ae @b and") && !back.contains("ae @a,"),
            "{back}"
        );
        assert!(back.contains("goal: none"));
        assert_eq!(goal_text(""), "session goal cleared.");
        assert_eq!(goal_text("g"), "session goal now: g");
    }

    /// A scratch session `s` with one lead-pair main seat and `lines` journaled;
    /// its parent holds the send locks.
    fn session(tag: &str, lines: &[String]) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("ae-notice-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("s");
        std::fs::create_dir_all(&dir).unwrap();
        let meta = "session=s\nsession_id=11111111-2222-3333-4444-555555555555\nlayout=lead-pair\nseat.main=lead\nagent_bin.main=claude\nlaunch_id.main=tok\nlaunch_time.main=0\n";
        std::fs::write(dir.join("meta"), meta).unwrap();
        for line in lines {
            crate::store::open(&dir)
                .append_event(&format!("{line}\n"))
                .unwrap();
        }
        dir
    }

    #[test]
    fn a_missing_gave_up_stays_owed_and_names_the_third_failed_ref() {
        let mut lines = failed_thrice();
        let owed = due(&parse(&lines), &seat(), NOW, 120).unwrap();
        assert_eq!((owed.failures, owed.give_up.as_deref()), (3, Some("f2")));
        lines.push(line("session-notice-gave-up", 30, &ended("f2")));
        assert_eq!(due(&parse(&lines), &seat(), NOW, 120), None, "booked once");
        lines.push(line("goal", 1, ""));
        let rearmed = kind(&parse(&lines), &seat());
        assert_eq!(rearmed, Some((Kind::Goal, 0)), "a newer source re-arms");
    }

    #[test]
    fn a_goal_read_error_after_the_attempt_is_never_a_no_goal_text() {
        let dir = session("unread", &[line("goal", 0, r#","summary":"g""#)]);
        let unread = || Err(std::io::Error::other("unreadable meta"));
        let attempted = attempt(&dir, "main", "s", "sn-1", 120, unread).unwrap();
        let (seat, owed, text) = attempted.unwrap();
        assert_eq!(owed.kind, Kind::Goal);
        assert_eq!(
            text,
            Err("meta-unread"),
            "a failed read is not a cleared goal"
        );
        let mut err = Vec::new();
        let code = finish(&dir, &seat, owed.kind, "sn-1", Err("meta-unread"), &mut err).unwrap();
        assert_eq!(code, crate::state::EXIT_FAILED);
        let events = crate::watchdog_daemon::read_events(&dir);
        let on_ref: Vec<(&str, Option<&str>)> = events
            .iter()
            .filter(|event| event.reference.as_deref() == Some("sn-1"))
            .map(|event| (event.action.as_str(), event.summary.as_deref()))
            .collect();
        let expected = [
            ("session-notice-attempt", Some("goal tok")),
            ("session-notice-failed", Some("goal meta-unread")),
        ];
        assert_eq!(on_ref, expected, "a failed terminal on the attempt's ref");
        let again = due(&events, &seat, crate::time::Timestamp::now().epoch(), 120);
        assert_eq!(
            again.map(|owed| owed.failures),
            Some(1),
            "owed again, not told"
        );
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn the_ceiling_is_booked_once_on_the_third_ref_and_never_attempted() {
        let dir = session("ceiling", &failed_thrice());
        let goal = || Ok(Some(b"g".to_vec()));
        let before = attempt(&dir, "main", "s", "sn-4", 120, goal).unwrap();
        assert!(
            before.is_none(),
            "no fourth attempt while the booking is owed"
        );
        assert_eq!(give_up(&dir, "main", "%9", "s", 120), None);
        assert_eq!(give_up(&dir, "main", "%9", "s", 120), None, "nothing left");
        let events = crate::watchdog_daemon::read_events(&dir);
        let count = |action: &str| events.iter().filter(|event| event.action == action).count();
        assert_eq!(
            (
                count("session-notice-attempt"),
                count("session-notice-gave-up")
            ),
            (3, 1)
        );
        let booked = events.last().unwrap();
        assert_eq!(
            (booked.reference.as_deref(), booked.summary.as_deref()),
            (Some("f2"), Some("goal tok"))
        );
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    /// Contract: "in flight: newest counted attempt with no terminal and
    /// ts + 2*interval > now -> not due" — strict, so an attempt exactly the
    /// bound old is owed again.
    #[test]
    fn an_attempt_exactly_the_bound_old_is_no_longer_in_flight() {
        let a1 = attempt_json("goal", "tok", "r1");
        let at_bound = journal(&[("goal", 200, ""), ("session-notice-attempt", 120, &a1)]);
        assert_eq!(kind(&at_bound, &seat()), Some((Kind::Goal, 0)));
        let inside = journal(&[("goal", 200, ""), ("session-notice-attempt", 119, &a1)]);
        assert_eq!(kind(&inside, &seat()), None);
    }

    /// Contract: the gave-up record carries the attempt's `target_slot` and
    /// `<kind> <launch_id>` summary, and a newer source is due after it — the
    /// booking is never read back as an attempt in flight.
    #[test]
    fn a_booked_ceiling_is_no_attempt_and_a_newer_source_is_owed_at_once() {
        let mut lines = failed_thrice();
        let booked = attempt_json("goal", "tok", "f2");
        lines.push(line("session-notice-gave-up", 30, &booked));
        lines.push(line("goal", 1, ""));
        assert_eq!(kind(&parse(&lines), &seat()), Some((Kind::Goal, 0)));
    }

    /// The journal is hostile input: one that opens with an attempt is folded
    /// like any other, never a panic.
    #[test]
    fn a_journal_that_opens_with_an_attempt_is_folded() {
        let a1 = attempt_json("goal", "tok", "r1");
        let opened = journal(&[("session-notice-attempt", 900, &a1), ("goal", 5, "")]);
        assert_eq!(kind(&opened, &seat()), Some((Kind::Goal, 0)));
    }

    /// Contract: every leg record names the seat by target, `target_slot`,
    /// `target_session` and `target_session_id`.
    #[test]
    fn every_leg_record_names_the_seat_by_all_four_target_fields() {
        use crate::events::RoutingMember;
        let dir = session("fields", &[line("goal", 0, r#","summary":"g""#)]);
        let goal = || Ok(Some(b"g".to_vec()));
        let (seat, owed, _) = attempt(&dir, "main", "s", "sn-1", 120, goal)
            .unwrap()
            .unwrap();
        let mut err = Vec::new();
        finish(&dir, &seat, owed.kind, "sn-1", Err("refused"), &mut err).unwrap();
        let events = crate::watchdog_daemon::read_events(&dir);
        let leg: Vec<&Event> = events
            .iter()
            .filter(|event| event.reference.as_deref() == Some("sn-1"))
            .collect();
        assert_eq!(leg.len(), 2);
        let value = |text: &str| RoutingMember::Value(text.to_owned());
        for event in leg {
            assert_eq!(event.target.as_deref(), Some("lead"), "{}", event.action);
            assert_eq!(event.target_slot, value("main"), "{}", event.action);
            assert_eq!(event.target_session, value("s"), "{}", event.action);
            assert_eq!(
                event.target_session_id,
                value("11111111-2222-3333-4444-555555555555"),
                "{}",
                event.action
            );
        }
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    /// docs/internals/events.md: told is `<kind>` (verified) or `[unconfirmed] <kind>`;
    /// failed is `<kind> storage|busy|paste|refused`; exit 0 only when told.
    #[test]
    fn each_ending_is_journaled_by_its_documented_word() {
        use crate::deliver::{DeferHeld, Delivered, DeliveryVerification, Failure, Unverifiable};
        let dir = session("endings", &[]);
        let landed = |verification| {
            Ok(Ok(Delivered {
                body_file: String::new(),
                framed: String::new(),
                verification,
            }))
        };
        let refused = |failure| Ok(Err(failure));
        let paste = Failure::Paste {
            body_file: String::new(),
        };
        let busy = Failure::Abandoned {
            held: DeferHeld::ComposerOccupied,
        };
        let cases = [
            (
                landed(DeliveryVerification::Verified),
                "session-notice",
                "goal",
                0,
            ),
            (
                landed(DeliveryVerification::Unverifiable(Unverifiable::Unmodelled)),
                "session-notice",
                "[unconfirmed] goal",
                0,
            ),
            (
                refused(Failure::Storage),
                "session-notice-failed",
                "goal storage",
                1,
            ),
            (refused(busy), "session-notice-failed", "goal busy", 1),
            (refused(paste), "session-notice-failed", "goal paste", 1),
            (
                refused(Failure::Lock),
                "session-notice-failed",
                "goal refused",
                1,
            ),
        ];
        for (at, (outcome, action, summary, code)) in cases.into_iter().enumerate() {
            let reference = format!("sn-{at}");
            let mut err = Vec::new();
            let exit = finish(&dir, &seat(), Kind::Goal, &reference, outcome, &mut err).unwrap();
            let events = crate::watchdog_daemon::read_events(&dir);
            let ended = events.last().unwrap();
            assert_eq!(
                (ended.action.as_str(), ended.summary.as_deref(), exit),
                (action, Some(summary), code),
                "{reference}"
            );
        }
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    /// Contract: the attempt ref's nonce is unique — terminals match their
    /// attempt by ref ONLY, so two refs minted in one second differ.
    #[test]
    fn two_refs_minted_in_one_second_differ() {
        let now = crate::time::Timestamp::now();
        let first = crate::tracked::request_id("sn", now, super::entropy());
        let second = crate::tracked::request_id("sn", now, super::entropy());
        assert_ne!(first, second);
    }

    /// R4 + fn doc: a changed goal prints the told line while a standing seat
    /// other than its PROVEN setter runs, says NOT told for the count of them
    /// with the watchdog off, and prints nothing when only the setter stands;
    /// a worker never stands, and an unproven or foreign setter excludes nobody.
    #[test]
    fn a_changed_goal_prints_a_line_only_for_another_standing_seat() {
        let id = "11111111-2222-3333-4444-555555555555";
        let line = |tag: &str, setter_id: &str, rows: &str| {
            let setter = crate::requests::Viewer {
                slot: "main".to_owned(),
                session: "s".to_owned(),
                display: "lead".to_owned(),
                session_id: setter_id.to_owned(),
            };
            let dir = session(tag, &[]);
            let meta = format!("session=s\nsession_id={id}\nseat.main=lead\n{rows}");
            std::fs::write(dir.join("meta"), meta).unwrap();
            let said = super::after_goal(&dir, &setter);
            let _ = std::fs::remove_dir_all(dir.parent().unwrap());
            said
        };
        let told = Some(super::TOLD_LINE.to_owned());
        let pair = "layout=lead-pair\nseat.worker.0=colead\nseat.worker.1=scribe\n";
        assert_eq!(line("goal-pair", id, &format!("{pair}watchdog=on\n")), told);
        let off = line("goal-off", id, &format!("{pair}watchdog=off\n"));
        assert_eq!(off, Some(super::untold_line("watchdog off", 1)));
        assert_eq!(
            line("goal-solo", id, "seat.worker.0=scribe\nwatchdog=on\n"),
            None
        );
        assert_eq!(line("goal-unproven", "", "watchdog=on\n"), told);
        let foreign = "66666666-7777-8888-9999-000000000000";
        assert_eq!(line("goal-foreign", foreign, "watchdog=on\n"), told);
    }

    /// Docs (events.md, session notices) + run doc: the leg tells a seat of
    /// THIS session what is due, if anything; an attempt with no ending stays
    /// in flight for two daemon intervals, and an interval that is not a
    /// positive count of seconds is the daemon's default, never a zero bound.
    #[test]
    fn the_leg_takes_only_its_own_seat_and_never_a_non_positive_interval() {
        let now = crate::time::Timestamp::now().epoch();
        let at = |action: &str, age: i64, extra: &str| {
            let ts = crate::time::Timestamp::from_epoch(now - age);
            format!(r#"{{"ts":"{ts}","actor":"x","action":"{action}"{extra}}}"#)
        };
        // Three failed goal attempts owe the ceiling; a bare attempt 30 s old
        // holds it in flight under the default bound.
        let mut lines = vec![at("goal", 300, "")];
        for index in 0..3 {
            let reference = format!("f{index}");
            lines.push(at(
                "session-notice-attempt",
                200,
                &attempt_json("goal", "tok", &reference),
            ));
            lines.push(at("session-notice-failed", 200, &ended(&reference)));
        }
        lines.push(at(
            "session-notice-attempt",
            30,
            &attempt_json("goal", "tok", "f3"),
        ));
        for (pane_session, interval, code) in [("s", "0", 0), ("", "-5", 0), ("other", "60", 1)] {
            let dir = session("leg-own", &lines);
            let resolved = crate::tracked::Resolved {
                pane: "%1".to_owned(),
                agent: "lead".to_owned(),
                slot: "main".to_owned(),
                session: pane_session.to_owned(),
            };
            crate::tracked::set_test_resolve(resolved, crate::inventory::ServerId::Ambient);
            let mut err = Vec::new();
            let exit = super::run(&dir, "lead", "s", Some(interval), &mut err).unwrap();
            let events = crate::watchdog_daemon::read_events(&dir);
            let gave_up = events
                .iter()
                .any(|event| event.action == "session-notice-gave-up");
            let said = String::from_utf8_lossy(&err);
            assert_eq!(
                (exit, gave_up),
                (code, false),
                "{pane_session} {interval}: {said}"
            );
            let _ = std::fs::remove_dir_all(dir.parent().unwrap());
        }
    }
}
