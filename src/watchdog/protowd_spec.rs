//! Acceptance for the wait-challenge diet and the sleep-aware wait clocks
//! (protocol quick wins I14 and I15), frozen with protowd-nav: D1-D9, S1-S9.

use super::{
    CycleClock, HOST_SUSPENDED_ACTION, NUDGE_TAIL, Suspended, Suspensions, WaitProgress,
    WaitReading, WaitState, host_suspended, raw_nudge, suspended_summary, wait_goal_owed,
    wait_reading, wait_rung_spacing, waiting_agent_ceiling_secs, waiting_agent_escalated,
    waiting_agent_escalated_at,
};
use crate::events::Event;
use crate::time::Timestamp;
use crate::watchdog_daemon::{WaitCheck, done_challenge_text, wait_challenge_text};
use std::path::Path;
use std::time::{Duration, Instant};

const CADENCE: u64 = 300;

fn row(at: i64, actor: &str, action: &str, reference: Option<&str>, target: Option<&str>) -> Event {
    let mut event =
        Event::parse_line(r#"{"ts":"2026-11-11T00:00:00Z","actor":"seat","action":"state"}"#)
            .expect("independent event fixture");
    event.ts = Timestamp::from_epoch(at);
    event.actor = actor.to_owned();
    event.action = action.to_owned();
    event.reference = reference.map(str::to_owned);
    event.target = target.map(str::to_owned);
    event
}

/// The seat's own `waiting-agent` declaration, optionally `--on` a target.
fn wait(at: i64, on: Option<&str>) -> Event {
    let mut event = row(at, "seat", "state", Some("waiting-agent"), None);
    event.summary = Some(format!("waiting at {at}"));
    event.wait_on = on.map(str::to_owned);
    event
}

fn challenge(at: i64, number: u8) -> Event {
    let mut event = row(
        at,
        "watchdog",
        "wait-challenge",
        Some("launch-1"),
        Some("seat"),
    );
    event.summary = Some(format!("waiting-agent confirmation {number}/2"));
    event
}

fn suspended(at: i64, secs: u64) -> Event {
    let mut event = row(at, "watchdog", HOST_SUSPENDED_ACTION, None, None);
    event.summary = Some(suspended_summary(Suspended {
        secs,
        wall_secs: secs + 60,
        mono_secs: 60,
    }));
    event
}

fn read(events: &[Event], now: i64) -> WaitReading {
    wait_reading(
        events,
        "demo",
        "main",
        "seat",
        Some("launch-1"),
        Timestamp::from_epoch(now),
        CADENCE,
        2,
        WaitState::WaitingAgent,
    )
}

fn due(reading: &WaitReading) -> bool {
    matches!(reading.progress, WaitProgress::ChallengeDue { .. })
}

/// Challenge at `at`, credited proof ten seconds later, pushed in order.
fn answer(events: &mut Vec<Event>, at: i64, number: u8, on: Option<&str>) -> i64 {
    events.push(challenge(at, number));
    events.push(wait(at + 10, on));
    at + 10
}

// ── D1 ────────────────────────────────────────────────────────────────────

#[test]
fn d1_backoff_rungs_advance_only_on_credited_unchanged() {
    assert_eq!(
        (0..=5)
            .map(|rung| wait_rung_spacing(CADENCE, rung))
            .collect::<Vec<_>>(),
        [300, 600, 1200, 2400, 2400, 2400],
        "the schedule: cadence x min(2^rung, 8), capped"
    );
    assert_eq!(
        wait_rung_spacing(u64::MAX, 3),
        u64::MAX,
        "overflow keeps the cadence"
    );

    let mut events = vec![wait(0, Some("lead"))];
    assert_eq!(read(&events, 299).rung, 0);
    assert!(!due(&read(&events, 299)) && due(&read(&events, 300)));
    let mut anchor = answer(&mut events, 300, 1, Some("lead"));
    for (rung, spacing) in [(1, 600), (2, 1200), (3, 2400), (3, 2400)] {
        let before = read(&events, anchor + spacing - 1);
        assert_eq!(
            before.rung, rung,
            "rung after the credited proof at {anchor}"
        );
        assert!(!due(&before), "rung {rung} is not due before {spacing}s");
        assert!(
            due(&read(&events, anchor + spacing)),
            "rung {rung} is due at {spacing}s"
        );
        anchor = answer(&mut events, anchor + spacing, 1, Some("lead"));
    }

    // A proactive re-declaration (no challenge outstanding) resets to rung 0.
    events.push(wait(anchor + 30, Some("lead")));
    let reset = read(&events, anchor + 30 + 299);
    assert_eq!(reset.rung, 0);
    assert!(due(&read(&events, anchor + 30 + 300)));
}

// ── D2 ────────────────────────────────────────────────────────────────────

/// A journal two credited proofs deep (rung 2), anchored at the returned epoch.
fn rung_two() -> (Vec<Event>, i64) {
    let mut events = vec![wait(0, Some("lead"))];
    answer(&mut events, 300, 1, Some("lead"));
    let anchor = answer(&mut events, 910, 2, Some("lead"));
    assert_eq!(read(&events, anchor + 1).rung, 2, "fixture is at rung 2");
    (events, anchor)
}

#[test]
fn d2_changed_declaration_or_news_resets_backoff() {
    // A credited proof that changes the --on target resets to rung 0.
    let (mut events, anchor) = rung_two();
    let anchor = answer(&mut events, anchor + 1200, 1, Some("colead"));
    assert_eq!(
        read(&events, anchor + 1).rung,
        0,
        "a changed target is a changed wait"
    );
    assert!(due(&read(&events, anchor + 300)));

    // News the wait outlives (a peer's ask) resets.
    let (mut events, anchor) = rung_two();
    events.push(row(anchor + 50, "peer", "ask", Some("r1"), Some("seat")));
    assert_eq!(read(&events, anchor + 60).rung, 0);
    assert!(due(&read(&events, anchor + 300)));

    // Another state supersedes the episode.
    let (mut events, anchor) = rung_two();
    let mut working = row(anchor + 50, "seat", "state", Some("working"), None);
    working.summary = Some("back at it".to_owned());
    events.push(working);
    assert!(matches!(
        read(&events, anchor + 60).progress,
        WaitProgress::None
    ));

    // Information (a peer's send) keeps the rung: #225 documents it.
    let (mut events, anchor) = rung_two();
    events.push(row(anchor + 50, "peer", "send", None, Some("seat")));
    let informed = read(&events, anchor + 1199);
    assert_eq!(informed.rung, 2);
    assert!(!due(&informed));

    // A reason-text edit on a credited proof keeps climbing.
    let (mut events, anchor) = rung_two();
    events.push(challenge(anchor + 1200, 1));
    let mut edited = wait(anchor + 1210, Some("lead"));
    edited.summary = Some("a wholly different reason".to_owned());
    events.push(edited);
    assert_eq!(read(&events, anchor + 1211).rung, 3);
}

// ── D3 (fold half; the daemon half is in watchdog_daemon/protowd_spec.rs) ─

#[test]
fn d3_ceiling_scales_with_the_credited_rung_only() {
    assert_eq!(
        (0..=4)
            .map(|rung| waiting_agent_ceiling_secs(CADENCE, rung))
            .collect::<Vec<_>>(),
        [1200, 1200, 1500, 2700, 2700]
    );
    assert!(!waiting_agent_escalated_at(1199, CADENCE, 0));
    assert!(waiting_agent_escalated_at(1200, CADENCE, 0));
    assert!(!waiting_agent_escalated_at(2699, CADENCE, 3));
    assert!(waiting_agent_escalated_at(2700, CADENCE, 3));
    assert_eq!(
        waiting_agent_escalated(1200, CADENCE),
        waiting_agent_escalated_at(1200, CADENCE, 0),
        "the rung-0 form is today's pin"
    );
    assert_eq!(
        waiting_agent_ceiling_secs(0, 0),
        1200,
        "zero scales from the default"
    );
}

// ── D4 ────────────────────────────────────────────────────────────────────

const META: &str = "/Users/ckriech/.ae/sessions/aedev/";

fn check(state: WaitState, escalated: bool, target: Option<&str>) -> WaitCheck<'_> {
    WaitCheck {
        age: 720,
        confirmations: 0,
        required: 2,
        state,
        escalated,
        target,
    }
}

#[test]
fn d4_diet_text_names_target_and_age_and_strips() {
    assert_eq!(
        META.len(),
        34,
        "the aedev path the byte budget is pinned on"
    );
    let meta = Path::new(META.trim_end_matches('/'));
    for (state, target, word) in [
        (
            WaitState::WaitingAgent,
            Some("lead"),
            "waiting-agent on lead",
        ),
        (WaitState::WaitingAgent, None, "waiting-agent"),
        (WaitState::Blocked, None, "blocked"),
    ] {
        let plain = wait_challenge_text(None, meta, &check(state, false, target));
        assert!(plain.len() <= 250, "{} B: {plain}", plain.len());
        assert!(!plain.contains('\n'), "one line: {plain}");
        assert!(
            plain.contains(word),
            "names the raw state and target: {plain}"
        );
        assert!(plain.contains("12m"), "names the age: {plain}");
        assert!(plain.contains("1/2"), "names the confirmation: {plain}");
        assert!(
            plain.contains("Re-declare"),
            "the re-declare affordance: {plain}"
        );
        assert!(plain.ends_with(NUDGE_TAIL), "{plain}");
        assert!(!plain.contains("Session goal:"), "{plain}");
        assert!(raw_nudge(&plain), "the footprint filter strips it: {plain}");

        let escalated = wait_challenge_text(None, meta, &check(state, true, target));
        assert!(escalated.len() <= 290, "{} B: {escalated}", escalated.len());
        assert!(escalated.contains("past the ceiling"), "{escalated}");
        assert!(raw_nudge(&escalated), "{escalated}");

        let goal = wait_challenge_text(
            Some("ship the quick wins"),
            meta,
            &check(state, false, target),
        );
        assert!(
            goal.starts_with("Session goal: ship the quick wins. "),
            "{goal}"
        );
        assert!(raw_nudge(&goal), "{goal}");
        let long = "g".repeat(200);
        let clipped = wait_challenge_text(Some(&long), meta, &check(state, false, target));
        assert!(
            raw_nudge(&clipped),
            "a clipped goal still strips: {clipped}"
        );
    }
    let blocked = wait_challenge_text(
        None,
        Path::new("/s"),
        &check(WaitState::Blocked, false, None),
    );
    assert!(
        blocked.contains("blocker"),
        "blocked asks for its blocker: {blocked}"
    );
}

// ── D5 ────────────────────────────────────────────────────────────────────

fn goal(at: i64) -> Event {
    let mut event = row(at, "lead", "goal", None, None);
    event.summary = Some("new goal".to_owned());
    event
}

fn owed(events: &[Event], agent: &str, launch: &str) -> bool {
    wait_goal_owed(events, "demo", "main", agent, Some(launch))
}

#[test]
fn d5_goal_quoted_only_when_changed_for_that_seat() {
    let mut events = vec![goal(0), wait(10, None)];
    assert!(
        owed(&events, "seat", "launch-1"),
        "the first challenge of a launch quotes it"
    );
    events.push(challenge(310, 1));
    assert!(
        !owed(&events, "seat", "launch-1"),
        "seen once, then omitted"
    );
    assert!(
        owed(&events, "other", "launch-1"),
        "another seat has not seen it"
    );
    assert!(
        owed(&events, "seat", "launch-2"),
        "a new launch has not seen it"
    );
    events.push(goal(400));
    assert!(
        owed(&events, "seat", "launch-1"),
        "a goal change is owed again"
    );
    events.push(challenge(700, 1));
    assert!(!owed(&events, "seat", "launch-1"));
    let mut unconfirmed = challenge(800, 1);
    unconfirmed.summary = Some(crate::tracked::unconfirmed_summary(
        "waiting-agent confirmation 1/2",
    ));
    events.push(goal(750));
    events.push(unconfirmed);
    assert!(
        owed(&events, "seat", "launch-1"),
        "an unconfirmed paste is not seen"
    );
}

// ── D6 ────────────────────────────────────────────────────────────────────

#[test]
fn d6_journal_derived_backoff_survives_restart() {
    let mut events = vec![wait(0, None)];
    answer(&mut events, 300, 1, None);
    let anchor = answer(&mut events, 910, 2, None);
    // A pure fold over the journal: no PaneState or carry is consulted.
    let reading = read(&events, anchor + 600);
    assert_eq!(reading.rung, 2);
    assert!(!due(&reading), "rung 2 waits 1200s, not today's 600s");
}

// ── D7 ────────────────────────────────────────────────────────────────────

#[test]
fn d7_lapsed_and_attempts_unchanged() {
    let (mut events, anchor) = rung_two();
    let at = anchor + 1200;
    events.push(challenge(at, 1));
    assert!(matches!(
        read(&events, at + 299).progress,
        WaitProgress::Challenged { .. }
    ));
    assert!(
        matches!(
            read(&events, at + 300).progress,
            WaitProgress::Lapsed { .. }
        ),
        "an unanswered challenge lapses after one cadence, whatever the rung"
    );
    let (mut events, anchor) = rung_two();
    for _ in 0..3 {
        let mut abandoned = row(
            anchor + 5,
            "watchdog",
            "delivery-abandoned",
            Some("launch-1"),
            Some("seat"),
        );
        abandoned.summary = Some("busy; waiting-agent confirmation 1/2".to_owned());
        events.push(abandoned);
    }
    assert!(matches!(
        read(&events, anchor + 1200).progress,
        WaitProgress::ChallengeDue { attempts: 3, .. }
    ));
}

// ── D8 ────────────────────────────────────────────────────────────────────

#[test]
fn d8_done_path_untouched() {
    assert_eq!(
        done_challenge_text(None, Path::new("/s"), 600, 0, 2),
        "Continue working; do not re-plan or ask unless blocked. Done was declared 10m ago; \
         confirmation 1 of 2. State how each deliverable was verified. Anything unverified: \
         declare working and finish assigned work NOW. Awaiting owner review: invent no scope; \
         do not commit or edit solely for this challenge. Else re-declare done with work and \
         proof, held review included. Self-attestation, not owner approval. Declare state: \
         /s/state <waiting-user|waiting-agent|blocked|done> \"<reason>\""
    );
    // A suspension does not move the done fold's clock (done is out of scope).
    let mut done = row(0, "seat", "state", Some("done"), None);
    done.summary = Some("shipped".to_owned());
    let events = [done, suspended(7000, 6900)];
    assert!(matches!(
        super::done_progress(
            &events,
            "demo",
            "main",
            "seat",
            Some("launch-1"),
            Timestamp::from_epoch(7000),
            CADENCE,
            2
        ),
        super::DoneProgress::ChallengeDue {
            done_age_secs: 7000,
            ..
        }
    ));
}

// ── D9 ────────────────────────────────────────────────────────────────────

#[test]
fn d9_credited_wait_and_a_long_sleep_neither_challenge_nor_climb() {
    let (mut events, anchor) = rung_two();
    // Asleep from anchor+100 to anchor+7300.
    events.push(suspended(anchor + 7300, 7200));
    let woke = read(&events, anchor + 7300);
    assert_eq!(woke.rung, 2, "a suspension is neither a proof nor news");
    assert!(
        matches!(woke.progress, WaitProgress::Provisional { .. }),
        "{woke:?}"
    );
    let suspensions = Suspensions::read(&events);
    assert_eq!(suspensions.awake_secs(anchor, anchor + 7300), 100);
    assert!(!waiting_agent_escalated_at(100, CADENCE, woke.rung));
}

// ── S1, S6, S7: the detector ─────────────────────────────────────────────

fn clock(base: Instant, wall: i64, mono: u64) -> CycleClock {
    CycleClock {
        wall,
        mono: base + Duration::from_secs(mono),
    }
}

#[test]
fn s1_gap_exact_threshold_boundary() {
    let base = Instant::now();
    let prev = clock(base, 1_000, 0);
    assert_eq!(
        host_suspended(prev, clock(base, 1_000 + 60 + 180, 60), 60),
        Some(Suspended {
            secs: 180,
            wall_secs: 240,
            mono_secs: 60
        }),
        "3 x interval is inclusive"
    );
    assert_eq!(
        host_suspended(prev, clock(base, 1_000 + 60 + 179, 60), 60),
        None
    );
    // Two successive wake cycles: two records over disjoint spans, sum exact.
    let first = suspended(5_000, 1_000);
    let second = suspended(7_000, 1_500);
    let both = Suspensions::read(&[first, second]);
    assert_eq!(both.awake_secs(3_000, 7_000), 4_000 - 2_500);
    assert_eq!(both.awake_secs(4_500, 5_000), 500 - 500);
}

#[test]
fn s6_stall_gap_is_not_a_suspension() {
    let base = Instant::now();
    assert_eq!(
        host_suspended(clock(base, 1_000, 0), clock(base, 4_600, 3_600), 60),
        None,
        "both clocks advanced: a stall, not a sleep"
    );
    let text = suspended_summary(Suspended {
        secs: 3570,
        wall_secs: 3600,
        mono_secs: 30,
    });
    assert!(
        !text.contains("verified") && !text.contains("sleep"),
        "{text}"
    );
    assert!(
        text.contains("wall clock +3600s") && text.contains("monotonic clock +30s"),
        "{text}"
    );
}

#[test]
fn s7_wall_monotonic_divergence() {
    let base = Instant::now();
    assert_eq!(
        host_suspended(clock(base, 0, 0), clock(base, 3_600, 30), 60),
        Some(Suspended {
            secs: 3_570,
            wall_secs: 3_600,
            mono_secs: 30
        })
    );
    assert_eq!(
        host_suspended(clock(base, 0, 0), clock(base, 30, 3_600), 60),
        None
    );
    assert_eq!(
        host_suspended(clock(base, 0, 0), clock(base, -100, 30), 60),
        None,
        "wall stepped back"
    );
}

// ── S2, S3, S5 ────────────────────────────────────────────────────────────

#[test]
fn s2_wait_past_ceiling_after_sleep_stays_quiet() {
    // Declared 240s before a 7200s suspension (300s would be the cadence and
    // so the ordinary due challenge, not a burst).
    let events = vec![wait(0, Some("lead")), suspended(240 + 7200, 7200)];
    let woke = read(&events, 240 + 7200);
    assert!(
        matches!(woke.progress, WaitProgress::Provisional { .. }),
        "{woke:?}"
    );
    let awake = Suspensions::read(&events).awake_secs(0, 240 + 7200);
    assert_eq!(awake, 240);
    assert!(!waiting_agent_escalated_at(awake, CADENCE, 0));
}

#[test]
fn s3_wall_records_stay_wall() {
    let events = vec![wait(0, None), suspended(7_440, 7_200)];
    let before = events.clone();
    let _ = Suspensions::read(&events);
    let _ = read(&events, 7_440);
    assert_eq!(events, before, "no record is rewritten");
    assert_eq!(events[0].ts.epoch(), 0);
}

#[test]
fn s5_durable_sleep_fold() {
    // Suspended after the declaration: subtracted.
    let after = Suspensions::read(&[wait(1_000, None), suspended(9_000, 7_000)]);
    assert_eq!(after.awake_secs(1_000, 9_100), 8_100 - 7_000);
    // Suspended wholly before it: nothing to subtract.
    let before = Suspensions::read(&[suspended(900, 500), wait(1_000, None)]);
    assert_eq!(before.awake_secs(1_000, 1_200), 200);
    // Straddling the declaration: only the part after it.
    let straddle = Suspensions::read(&[suspended(1_100, 300)]);
    assert_eq!(straddle.awake_secs(1_000, 1_200), 200 - 100);
    // Overlapping records never subtract twice and never go negative.
    let doubled = Suspensions::read(&[suspended(2_000, 1_000), suspended(2_000, 1_000)]);
    assert_eq!(doubled.awake_secs(0, 2_000), 1_000);
    assert_eq!(doubled.awake_secs(1_500, 2_000), 0);
}

// ── S8 ────────────────────────────────────────────────────────────────────

#[test]
fn s8_hostile_suspended_parser() {
    let forged = |actor: &str, summary: Option<&str>| {
        let mut event = row(10_000, actor, HOST_SUSPENDED_ACTION, None, None);
        event.summary = summary.map(str::to_owned);
        event
    };
    for summary in [
        None,
        Some(""),
        Some("suspended -5s: wall clock +1s, monotonic clock +1s"),
        Some("suspended 99999999999999999999999s: x"),
        Some("suspended 3600000s: beyond the thirty-day cap"),
        Some("suspended 12 s: spaced"),
        Some("suspended ٣s: non-ascii digit"),
        Some("suspended s:"),
        Some("resumed 600s:"),
    ] {
        let read = Suspensions::read(&[forged("watchdog", summary)]);
        assert_eq!(
            read.awake_secs(0, 10_000),
            10_000,
            "{summary:?} subtracts nothing"
        );
    }
    let foreign = Suspensions::read(&[forged(
        "seat",
        Some("suspended 600s: wall clock +660s, monotonic clock +60s"),
    )]);
    assert_eq!(
        foreign.awake_secs(0, 10_000),
        10_000,
        "only the watchdog records suspensions"
    );
    let real = Suspensions::read(&[forged(
        "watchdog",
        Some("suspended 600s: wall clock +660s, monotonic clock +60s"),
    )]);
    assert_eq!(real.awake_secs(0, 10_000), 9_400);
    assert_eq!(real.awake_secs(10_000, 0), 0, "a clock behind reads zero");
    assert_eq!(
        real.awake_secs(i64::MIN, i64::MAX),
        u64::MAX - 600,
        "saturates"
    );
}

// ── S9 ────────────────────────────────────────────────────────────────────

#[test]
fn s9_zero_interval_keeps_ceiling_from_default() {
    let events = vec![wait(0, None), suspended(7_400, 7_200)];
    let zero = wait_reading(
        &events,
        "demo",
        "main",
        "seat",
        Some("launch-1"),
        Timestamp::from_epoch(7_400),
        0,
        2,
        WaitState::WaitingAgent,
    );
    assert_eq!(
        zero.progress,
        WaitProgress::None,
        "cadence 0 tracks no episode"
    );
    let awake = Suspensions::read(&events).awake_secs(0, 7_400);
    assert_eq!(awake, 200);
    assert!(!waiting_agent_escalated_at(awake, 0, 0));
    assert!(
        waiting_agent_escalated_at(1_200, 0, 0),
        "1200s from the default 300s"
    );
}
