//! Frozen characterisation pins for the two journal folds
//! `watchdog::done_progress` and `watchdog::wait_progress`
//! (slice progress, #300), written before any extraction.
//!
//! Oracle, per pin: `oracle: doc` where `docs/internals/watchdog.md` or the
//! contract states the rule; otherwise the fold's output on base `83b518a2`,
//! labelled `behaviour pin`. Pins marked `bug-shaped` are reported for #300
//! and pinned as-is, never fixed here. G-numbers: `.local/progress/plan.md`
//! section 6 (G1-G10) and the nav critique (G11-G19).
//!
//! G13 migrated under lead ruling ae-20261107T042841Z-4a9facd4: an ask resets
//! done; information preserves its episode and failed-attempt budget.
//!
//! Pure calls over parsed journal lines: no filesystem, tmux or harness.
//! Cadence 60 and required 2 unless a pin says otherwise; times are seconds
//! from one fixed origin, appended in slice order (append order rules).

#![allow(clippy::expect_used, reason = "frozen fixture records and timestamps")]

use ae::events::Event;
use ae::time::Timestamp;
use ae::watchdog::{DoneProgress, WaitProgress, WaitState};

const SESSION: &str = "s";
const SLOT: &str = "main";
const AGENT: &str = "a";
const OWN: &str = r#","actor_slot":"main","actor_session":"s""#;
const TO: &str = r#","target":"a","target_slot":"main","target_session":"s""#;

fn at(secs: i64) -> Timestamp {
    Timestamp::from_epoch(
        Timestamp::parse("2026-08-29T04:00:00Z")
            .expect("fixture origin")
            .epoch()
            + secs,
    )
}

fn ev(secs: i64, actor: &str, action: &str, extra: &str) -> Event {
    Event::parse_line(&format!(
        r#"{{"ts":"{}","actor":"{actor}","action":"{action}"{extra}}}"#,
        at(secs)
    ))
    .expect("fixture event")
}

/// The seat's own `state` declaration.
fn decl(secs: i64, state: &str, summary: &str) -> Event {
    ev(
        secs,
        AGENT,
        "state",
        &format!(r#"{OWN},"ref":"{state}","summary":"{summary}""#),
    )
}

/// A watchdog record addressed to the seat; `reference` is the launch ref.
fn wd(secs: i64, action: &str, reference: Option<&str>, summary: Option<&str>) -> Event {
    let reference = reference.map_or_else(String::new, |id| format!(r#","ref":"{id}""#));
    let summary = summary.map_or_else(String::new, |text| format!(r#","summary":"{text}""#));
    ev(
        secs,
        "watchdog",
        action,
        &format!("{TO}{reference}{summary}"),
    )
}

fn dchal(secs: i64, reference: &str) -> Event {
    wd(secs, "done-challenge", Some(reference), None)
}

fn wchal(secs: i64, reference: &str, state_word: &str) -> Event {
    wd(
        secs,
        "wait-challenge",
        Some(reference),
        Some(&format!("{state_word} confirmation 1/2")),
    )
}

fn done_as(
    events: &[Event],
    now: i64,
    cadence: u64,
    required: u8,
    launch: Option<&str>,
) -> DoneProgress {
    ae::watchdog::done_progress(
        events,
        SESSION,
        SLOT,
        AGENT,
        launch,
        at(now),
        cadence,
        required,
    )
}

fn done(events: &[Event], now: i64) -> DoneProgress {
    done_as(events, now, 60, 2, Some("L1"))
}

fn wait_as(
    events: &[Event],
    now: i64,
    cadence: u64,
    required: u8,
    launch: Option<&str>,
    state: WaitState,
) -> WaitProgress {
    ae::watchdog::wait_progress(
        events,
        SESSION,
        SLOT,
        AGENT,
        launch,
        at(now),
        cadence,
        required,
        state,
    )
}

fn wait(events: &[Event], now: i64, state: WaitState) -> WaitProgress {
    wait_as(events, now, 60, 2, Some("L1"), state)
}

fn d_prov(confirmations: u8) -> DoneProgress {
    DoneProgress::Provisional {
        confirmations,
        required: 2,
    }
}

fn d_due(confirmations: u8, age: u64, attempts: u32) -> DoneProgress {
    DoneProgress::ChallengeDue {
        confirmations,
        required: 2,
        done_age_secs: age,
        attempts,
    }
}

fn w_prov(confirmations: u8) -> WaitProgress {
    WaitProgress::Provisional {
        confirmations,
        required: 2,
    }
}

fn w_due(confirmations: u8, age: u64, attempts: u32) -> WaitProgress {
    WaitProgress::ChallengeDue {
        confirmations,
        required: 2,
        wait_age_secs: age,
        attempts,
    }
}

/// G1 (behaviour pin, bug-shaped): a proactive distinct `done` with no
/// outstanding challenge does not move the anchor: age runs from the FIRST
/// done. The wait fold does the opposite (its own unit pins).
#[test]
fn progress_g1_done_proactive_redeclaration_keeps_the_first_anchor() {
    let events = [decl(0, "done", "first"), decl(30, "done", "again")];
    assert_eq!(done(&events, 59), d_prov(0));
    assert_eq!(done(&events, 60), d_due(0, 60, 0));
}

/// G2a (behaviour pin): an identical `(ts, summary)` done inside a challenged
/// episode is the dual-emit twin shape and earns no credit.
#[test]
fn progress_g2_identical_done_repeat_is_deduped_inside_a_challenge() {
    let events = [
        decl(0, "done", "same"),
        dchal(10, "L1"),
        decl(0, "done", "same"),
    ];
    assert_eq!(
        done(&events, 20),
        DoneProgress::Challenged {
            confirmations: 0,
            required: 2
        }
    );
    assert_eq!(
        done(&events, 70),
        DoneProgress::Lapsed {
            confirmations: 0,
            required: 2
        }
    );
}

/// G2b (oracle: doc, done dual-emit): the bare `action=done` twin of a
/// `state done`, same ts and summary, opens no second credit.
#[test]
fn progress_g2_dual_emit_twin_is_deduped() {
    let twin = ev(0, AGENT, "done", &format!(r#"{OWN},"summary":"shipped""#));
    let events = [decl(0, "done", "shipped"), twin];
    assert_eq!(done(&events, 60), d_due(0, 60, 0));
}

/// G2c (behaviour pin): dedup compares against the LAST done signature only;
/// an older signature repeated after a different proof counts again.
#[test]
fn progress_g2_dedup_remembers_only_the_last_done() {
    let events = [
        decl(0, "done", "x"),
        dchal(10, "L1"),
        decl(20, "done", "y"),
        dchal(30, "L1"),
        decl(0, "done", "x"),
    ];
    assert_eq!(done(&events, 40), DoneProgress::Confirmed);
}

/// G2d (behaviour pin): a boundary that ends the episode also clears the
/// dedup signature, so the identical done after it opens a fresh episode.
#[test]
fn progress_g2_a_boundary_clears_the_dedup_signature() {
    let peer = ev(10, "peer", "send", TO);
    let events = [decl(0, "done", "x"), peer, decl(0, "done", "x")];
    assert_eq!(done(&events, 30), d_prov(0));
}

/// G3 (behaviour pin, bug-shaped): a launch-matching `delivery-abandoned`
/// counts as a done attempt whatever its summary says, while the wait fold
/// requires its own state's challenge words.
#[test]
fn progress_g3_abandoned_summary_matching_is_asymmetric() {
    let foreign = "refused: busy; blocked confirmation 1/2";
    let events = [
        decl(0, "done", "x"),
        wd(10, "delivery-abandoned", Some("L1"), Some(foreign)),
    ];
    assert_eq!(done(&events, 60), d_due(0, 60, 1));
    let garbage = [
        decl(0, "done", "x"),
        wd(10, "delivery-abandoned", Some("L1"), Some("whatever")),
    ];
    assert_eq!(done(&garbage, 60), d_due(0, 60, 1));
    for words in ["confirmation 1/2", "waiting-agent confirmation 1/2"] {
        let summary = format!("refused: busy; {words}");
        let events = [
            decl(0, "blocked", "dep"),
            wd(10, "delivery-abandoned", Some("L1"), Some(&summary)),
        ];
        assert_eq!(
            wait(&events, 60, WaitState::Blocked),
            w_due(0, 60, 0),
            "{words}"
        );
    }
}

/// G4 (behaviour pin): a second delivered challenge keeps the FIRST
/// outstanding timestamp; the lapse runs from it in both folds.
#[test]
fn progress_g4_second_delivered_challenge_keeps_the_first_timestamp() {
    let events = [decl(0, "done", "x"), dchal(60, "L1"), dchal(90, "L1")];
    assert_eq!(
        done(&events, 119),
        DoneProgress::Challenged {
            confirmations: 0,
            required: 2
        }
    );
    assert_eq!(
        done(&events, 120),
        DoneProgress::Lapsed {
            confirmations: 0,
            required: 2
        }
    );
    let events = [
        decl(0, "blocked", "dep"),
        wchal(60, "L1", "blocked"),
        wchal(90, "L1", "blocked"),
    ];
    assert_eq!(
        wait(&events, 120, WaitState::Blocked),
        WaitProgress::Lapsed {
            confirmations: 0,
            required: 2
        }
    );
}

/// G5 (behaviour pin): every challenge footprint before any declaration is
/// a no-op: no credit, no pre-counted attempts, no armed challenge.
#[test]
fn progress_g5_footprints_before_any_declaration_are_noops() {
    let events = [
        dchal(0, "L1"),
        wd(
            10,
            "done-challenge",
            Some("L1"),
            Some("[unconfirmed] confirmation 1/2"),
        ),
        wd(20, "delivery-abandoned", Some("L1"), Some("refused: busy")),
        decl(30, "done", "late"),
    ];
    assert_eq!(done(&events, 89), d_prov(0));
    assert_eq!(done(&events, 90), d_due(0, 60, 0));
    let events = [
        wchal(0, "L1", "blocked"),
        wd(
            10,
            "wait-challenge",
            Some("L1"),
            Some("[unconfirmed] blocked confirmation 1/2"),
        ),
        wd(
            20,
            "delivery-abandoned",
            Some("L1"),
            Some("refused: busy; blocked confirmation 1/2"),
        ),
        decl(30, "blocked", "late"),
    ];
    assert_eq!(wait(&events, 89, WaitState::Blocked), w_prov(0));
    assert_eq!(wait(&events, 90, WaitState::Blocked), w_due(0, 60, 0));
}

/// G6 (oracle: doc): `confirmations >= required` outranks an outstanding
/// challenge, and zero knobs with NO episode stay `None` (with an episode,
/// `Confirmed` is pinned by the fold's own unit tests).
#[test]
fn progress_g6_confirmed_outranks_outstanding_and_zero_knobs_need_an_episode() {
    let events = [
        decl(0, "done", "x"),
        dchal(10, "L1"),
        decl(20, "done", "proof"),
        dchal(30, "L1"),
    ];
    assert_eq!(
        done_as(&events, 40, 60, 1, Some("L1")),
        DoneProgress::Confirmed
    );
    assert_eq!(
        done_as(&events, 1000, 60, 1, Some("L1")),
        DoneProgress::Confirmed
    );
    assert_eq!(done_as(&[], 1000, 60, 0, Some("L1")), DoneProgress::None);
    assert_eq!(done_as(&[], 1000, 0, 2, Some("L1")), DoneProgress::None);
    let peer_only = [ev(0, "peer", "send", TO)];
    assert_eq!(
        done_as(&peer_only, 1000, 60, 0, Some("L1")),
        DoneProgress::None
    );
}

/// Knob placement is divergent (oracle: doc, disabled knobs): the wait fold
/// returns `None` on a zero knob BEFORE the fold, so even an open, challenged
/// or credited episode reads `None`; the same shapes in the done fold, whose
/// knob check sits after the fold, read `Confirmed`.
#[test]
fn progress_zero_knobs_open_the_folds_in_opposite_places() {
    let blocked = WaitState::Blocked;
    let open = [decl(0, "blocked", "dep")];
    let challenged = [decl(0, "blocked", "dep"), wchal(10, "L1", "blocked")];
    let credited = [
        decl(0, "blocked", "dep"),
        wchal(10, "L1", "blocked"),
        decl(20, "blocked", "still"),
    ];
    for events in [&open[..], &challenged[..], &credited[..]] {
        assert_eq!(
            wait_as(events, 1000, 60, 0, Some("L1"), blocked),
            WaitProgress::None
        );
        assert_eq!(
            wait_as(events, 1000, 0, 2, Some("L1"), blocked),
            WaitProgress::None
        );
    }
    let open = [decl(0, "done", "x")];
    let challenged = [decl(0, "done", "x"), dchal(10, "L1")];
    for events in [&open[..], &challenged[..]] {
        assert_eq!(
            done_as(events, 1000, 60, 0, Some("L1")),
            DoneProgress::Confirmed
        );
        assert_eq!(
            done_as(events, 1000, 0, 2, Some("L1")),
            DoneProgress::Confirmed
        );
    }
}

/// G7 (behaviour pin): a `now` earlier than the anchor or the challenge
/// clamps the age to zero in both folds, never negative, never due.
#[test]
fn progress_g7_now_before_the_anchor_clamps_age_to_zero() {
    let events = [decl(100, "done", "future")];
    assert_eq!(done(&events, 50), d_prov(0));
    let events = [decl(100, "done", "future"), dchal(110, "L1")];
    assert_eq!(
        done(&events, 105),
        DoneProgress::Challenged {
            confirmations: 0,
            required: 2
        }
    );
    let events = [decl(100, "blocked", "future")];
    assert_eq!(wait(&events, 0, WaitState::Blocked), w_prov(0));
    let events = [decl(100, "blocked", "future"), wchal(110, "L1", "blocked")];
    assert_eq!(
        wait(&events, 105, WaitState::Blocked),
        WaitProgress::Challenged {
            confirmations: 0,
            required: 2
        }
    );
}

/// G8 (oracle: doc #225 spacing): a credited wait proof KEEPS the failed
/// attempt count and earns two-cadence spacing; the re-arming Nth proof
/// resets attempts to zero.
#[test]
fn progress_g8_credit_keeps_attempts_rearm_resets_them() {
    let unconfirmed = wd(
        10,
        "wait-challenge",
        Some("L1"),
        Some("[unconfirmed] blocked confirmation 1/2"),
    );
    let credited = [
        decl(0, "blocked", "dep"),
        unconfirmed.clone(),
        wchal(20, "L1", "blocked"),
        decl(30, "blocked", "still"),
    ];
    assert_eq!(wait(&credited, 149, WaitState::Blocked), w_prov(1));
    assert_eq!(wait(&credited, 150, WaitState::Blocked), w_due(1, 120, 1));
    let rearmed = [
        decl(0, "blocked", "dep"),
        unconfirmed,
        wchal(20, "L1", "blocked"),
        decl(30, "blocked", "still"),
        wchal(160, "L1", "blocked"),
        decl(170, "blocked", "still"),
    ];
    assert_eq!(wait(&rearmed, 289, WaitState::Blocked), w_prov(0));
    assert_eq!(wait(&rearmed, 290, WaitState::Blocked), w_due(0, 120, 0));
}

/// G8/I4 (behaviour pin): the OTHER wait state's challenge is skipped before
/// any launch check, so it preserves earned spacing even with a wrong ref.
#[test]
fn progress_g8_other_state_challenge_preserves_earned_spacing() {
    for reference in ["L1", "L2"] {
        let events = [
            decl(0, "blocked", "dep"),
            wchal(10, "L1", "blocked"),
            decl(20, "blocked", "still"),
            wchal(30, reference, "waiting-agent"),
        ];
        assert_eq!(
            wait(&events, 100, WaitState::Blocked),
            w_prov(1),
            "ref {reference}"
        );
        assert_eq!(
            wait(&events, 140, WaitState::Blocked),
            w_due(1, 120, 0),
            "ref {reference}"
        );
    }
}

/// An own ask/review record for G9/G12.
fn own_request(secs: i64, action: &str, reference: &str) -> Event {
    ev(
        secs,
        AGENT,
        action,
        &format!(r#"{OWN},"ref":"{reference}","target":"peer""#),
    )
}

fn reply_to(secs: i64, reference: &str) -> Event {
    ev(
        secs,
        "peer",
        "reply",
        &format!(r#"{TO},"ref":"{reference}""#),
    )
}

/// G9 (oracle: doc, `ends_quiet`): an own ask sent BEFORE the declaration
/// still feeds `own_requests`; its reply ends `waiting-agent` alone.
#[test]
fn progress_g9_own_request_before_the_wait_still_counts() {
    let waited = [
        own_request(0, "ask", "r1"),
        decl(10, "waiting-agent", "on peer"),
        reply_to(20, "r1"),
    ];
    assert_eq!(
        wait(&waited, 80, WaitState::WaitingAgent),
        WaitProgress::None
    );
    let blocked = [
        own_request(0, "review", "r1"),
        decl(10, "blocked", "dep"),
        reply_to(20, "r1"),
    ];
    assert_eq!(wait(&blocked, 70, WaitState::Blocked), w_due(0, 60, 0));
    let unknown_ref = [
        own_request(0, "ask", "r1"),
        decl(10, "waiting-agent", "on peer"),
        reply_to(20, "r9"),
    ];
    assert_eq!(
        wait(&unknown_ref, 70, WaitState::WaitingAgent),
        w_due(0, 60, 0)
    );
    let asked_mid_wait = [
        decl(0, "waiting-agent", "on peer"),
        own_request(10, "ask", "r2"),
        reply_to(20, "r2"),
    ];
    assert_eq!(
        wait(&asked_mid_wait, 80, WaitState::WaitingAgent),
        WaitProgress::None
    );
}

/// G10 (behaviour pin): a launch-mismatched done-challenge resets the
/// episode AND clears the dedup signature.
#[test]
fn progress_g10_mismatched_challenge_clears_episode_and_signature() {
    let events = [decl(0, "done", "x"), dchal(10, "L2")];
    assert_eq!(done(&events, 60), DoneProgress::None);
    let events = [decl(0, "done", "x"), dchal(10, "L2"), decl(0, "done", "x")];
    assert_eq!(done(&events, 30), d_prov(0));
}

/// G11 (behaviour pin): the done fold never parses a challenge summary, so
/// garbage words still arm it; the wait fold treats a summary naming no
/// state as not-a-challenge and keeps earned spacing through it.
#[test]
fn progress_g11_garbage_challenge_summary_splits_the_folds() {
    let events = [
        decl(0, "done", "x"),
        wd(10, "done-challenge", Some("L1"), Some("garbage words")),
    ];
    assert_eq!(
        done(&events, 20),
        DoneProgress::Challenged {
            confirmations: 0,
            required: 2
        }
    );
    for summary in [Some("garbage"), Some("[unconfirmed] garbage"), None] {
        let events = [
            decl(0, "blocked", "dep"),
            wd(10, "wait-challenge", Some("L1"), summary),
        ];
        assert_eq!(
            wait(&events, 60, WaitState::Blocked),
            w_due(0, 60, 0),
            "{summary:?}"
        );
    }
    let earned = [
        decl(0, "blocked", "dep"),
        wchal(10, "L1", "blocked"),
        decl(20, "blocked", "still"),
        wd(30, "wait-challenge", Some("L1"), Some("garbage")),
    ];
    assert_eq!(wait(&earned, 100, WaitState::Blocked), w_prov(1));
    assert_eq!(wait(&earned, 140, WaitState::Blocked), w_due(1, 120, 0));
}

/// G12 (behaviour pin): `own_requests` is prefix state in append order; a
/// reply appended BEFORE its ask cannot end the wait.
#[test]
fn progress_g12_a_reply_before_its_ask_does_not_end_the_wait() {
    let reply_first = [
        reply_to(0, "r1"),
        own_request(10, "ask", "r1"),
        decl(20, "waiting-agent", "on peer"),
    ];
    assert_eq!(
        wait(&reply_first, 80, WaitState::WaitingAgent),
        w_due(0, 60, 0)
    );
    let mid_wait = [
        decl(0, "waiting-agent", "on peer"),
        reply_to(10, "r1"),
        own_request(20, "ask", "r1"),
    ];
    assert_eq!(
        wait(&mid_wait, 60, WaitState::WaitingAgent),
        w_due(0, 60, 0)
    );
}

/// G13 (behaviour pin): failed attempts never carry across a boundary
/// reset into the next episode, in either fold.
#[test]
fn progress_g13_attempts_do_not_carry_across_a_reset() {
    let unconfirmed = |secs: i64| {
        wd(
            secs,
            "done-challenge",
            Some("L1"),
            Some("[unconfirmed] confirmation 1/2"),
        )
    };
    let events = [
        decl(0, "done", "x"),
        unconfirmed(10),
        unconfirmed(20),
        // #225 D4: an actionable ask is the boundary; a send is information.
        ev(30, "peer", "ask", &format!(r#"{TO},"ref":"new-work""#)),
        decl(40, "done", "new"),
    ];
    assert_eq!(done(&events, 100), d_due(0, 60, 0));
    let events = [
        decl(0, "blocked", "dep"),
        wd(
            10,
            "wait-challenge",
            Some("L1"),
            Some("[unconfirmed] blocked confirmation 1/2"),
        ),
        ev(20, "telegram:1", "send", TO),
        decl(30, "blocked", "new"),
    ];
    assert_eq!(wait(&events, 90, WaitState::Blocked), w_due(0, 60, 0));
}

/// #225 D4: information keeps the existing done episode and failed-delivery
/// budget. A proactive declaration still leaves its original anchor standing.
#[test]
fn progress_g13_information_keeps_the_current_attempt_budget() {
    let events = [
        decl(0, "done", "x"),
        wd(
            10,
            "done-challenge",
            Some("L1"),
            Some("[unconfirmed] confirmation 1/2"),
        ),
        wd(
            20,
            "done-challenge",
            Some("L1"),
            Some("[unconfirmed] confirmation 1/2"),
        ),
        ev(30, "peer", "send", TO),
        decl(40, "done", "new"),
    ];
    assert_eq!(done(&events, 100), d_due(0, 100, 2));
}

/// G14 (oracle: doc, dual-emit legacy `done`): a bare `action=done` record
/// opens and credits a done episode, and supersedes a wait episode.
#[test]
fn progress_g14_bare_done_action_is_a_done_declaration() {
    let bare = |secs: i64, summary: &str| {
        ev(
            secs,
            AGENT,
            "done",
            &format!(r#"{OWN},"summary":"{summary}""#),
        )
    };
    assert_eq!(done(&[bare(0, "shipped")], 30), d_prov(0));
    let credited = [bare(0, "shipped"), dchal(10, "L1"), bare(20, "proof")];
    assert_eq!(done(&credited, 80), d_due(1, 60, 0));
    let superseded = [decl(0, "blocked", "dep"), bare(10, "shipped")];
    assert_eq!(
        wait(&superseded, 80, WaitState::Blocked),
        WaitProgress::None
    );
}

/// G15 (behaviour pin): the launch check runs before the outstanding check,
/// so a mismatched second challenge (unconfirmed included) resets even a
/// challenged episode, in both folds.
#[test]
fn progress_g15_mismatched_second_challenge_resets_a_challenged_episode() {
    let events = [decl(0, "done", "x"), dchal(10, "L1"), dchal(20, "L2")];
    assert_eq!(done(&events, 30), DoneProgress::None);
    let events = [
        decl(0, "done", "x"),
        wd(
            10,
            "done-challenge",
            Some("L2"),
            Some("[unconfirmed] confirmation 1/2"),
        ),
    ];
    assert_eq!(done(&events, 60), DoneProgress::None);
    let events = [
        decl(0, "blocked", "dep"),
        wchal(10, "L1", "blocked"),
        wchal(20, "L2", "blocked"),
    ];
    assert_eq!(wait(&events, 30, WaitState::Blocked), WaitProgress::None);
}

/// G16 (oracle: doc, routing-key relevance): a challenge or declaration of
/// ANOTHER seat is neither own nor addressed and cannot touch the episode.
#[test]
fn progress_g16_foreign_seat_records_are_irrelevant() {
    let foreign_done = ev(
        10,
        "watchdog",
        "done-challenge",
        r#","target":"b","target_slot":"spawned.0","target_session":"s","ref":"L1""#,
    );
    let events = [decl(0, "done", "x"), foreign_done];
    assert_eq!(done(&events, 60), d_due(0, 60, 0));
    let foreign_wait = ev(
        10,
        "watchdog",
        "wait-challenge",
        r#","target":"b","target_slot":"spawned.0","target_session":"s","ref":"L1","summary":"blocked confirmation 1/2""#,
    );
    let events = [decl(0, "blocked", "dep"), foreign_wait];
    assert_eq!(wait(&events, 60, WaitState::Blocked), w_due(0, 60, 0));
}

/// G17 (behaviour pin): a challenge action carried by a NON-watchdog actor
/// is no challenge at all: it ends `done` as ordinary news and clears a
/// wait's earned spacing without ending it.
#[test]
fn progress_g17_a_spoofed_challenge_is_ordinary_news() {
    let spoofed = ev(10, "peer", "done-challenge", TO);
    let events = [decl(0, "done", "x"), spoofed];
    assert_eq!(done(&events, 60), DoneProgress::None);
    let events = [
        decl(0, "blocked", "dep"),
        wchal(10, "L1", "blocked"),
        decl(20, "blocked", "still"),
        ev(30, "peer", "wait-challenge", TO),
    ];
    assert_eq!(wait(&events, 79, WaitState::Blocked), w_prov(1));
    assert_eq!(wait(&events, 80, WaitState::Blocked), w_due(1, 60, 0));
}

/// G18 (oracle: doc #148): a challenge refused BEFORE its paste is an
/// `[unconfirmed]` attempt of that challenge on the done side too: it
/// counts toward attempts and arms nothing.
#[test]
fn progress_g18_refused_pre_paste_done_challenge_counts_and_arms_nothing() {
    let refused = wd(
        10,
        "done-challenge",
        Some("L1"),
        Some("[unconfirmed] confirmation 1/2; refused pre-paste: dead pane"),
    );
    let events = [decl(0, "done", "x"), refused];
    assert_eq!(done(&events, 30), d_prov(0));
    assert_eq!(done(&events, 60), d_due(0, 60, 1));
}

/// Crossed challenges reset the other fold with ANY ref: the crossed path
/// never reaches a launch check (extends the folds' own crossed pins).
#[test]
fn progress_crossed_challenge_resets_with_any_ref() {
    let events = [decl(0, "done", "x"), wchal(10, "L2", "blocked")];
    assert_eq!(done(&events, 60), DoneProgress::None);
    let events = [decl(0, "blocked", "dep"), dchal(10, "L2")];
    assert_eq!(wait(&events, 60, WaitState::Blocked), WaitProgress::None);
}

/// Without a launch witness, a ref-less abandoned record of ANY delivery
/// counts as a done attempt (the documented degraded mode), while with a
/// witness the same ref-less record does not match and counts nothing.
#[test]
fn progress_refless_abandoned_counts_only_without_a_launch_witness() {
    let events = [
        decl(0, "done", "x"),
        wd(10, "delivery-abandoned", None, Some("refused: busy")),
    ];
    assert_eq!(done_as(&events, 60, 60, 2, None), d_due(0, 60, 1));
    assert_eq!(done(&events, 60), d_due(0, 60, 0));
}

/// G19 (behaviour pin, bug-shaped): the birth family never ends a wait
/// (`ends_quiet`), yet the spacing rule reads news as any relevant
/// NON-watchdog record: the seat's own `worker-birth` certificate therefore
/// clears earned spacing, while the watchdog's `worker-fact` does not.
/// (`session-id-replaced` never addresses the seat at all: its writer sets
/// a target slot with no session, half a routing key, so it is irrelevant
/// and cannot touch spacing.)
#[test]
fn progress_g19_birth_family_and_replaced_records_split_on_spacing() {
    let proof = vec![
        decl(0, "blocked", "dep"),
        wchal(10, "L1", "blocked"),
        decl(20, "blocked", "still"),
    ];
    let own_birth = ev(
        30,
        AGENT,
        "worker-birth",
        &format!(r#"{OWN},"target":"scout","target_slot":"spawned.0""#),
    );
    let mut events = proof.clone();
    events.push(own_birth);
    assert_eq!(wait(&events, 80, WaitState::Blocked), w_due(1, 60, 0));
    let watchdog_fact = ev(30, "watchdog", "worker-fact", TO);
    let mut events = proof.clone();
    events.push(watchdog_fact);
    assert_eq!(wait(&events, 100, WaitState::Blocked), w_prov(1));
    assert_eq!(wait(&events, 140, WaitState::Blocked), w_due(1, 120, 0));
    let replaced = ev(
        30,
        "ae",
        "session-id-replaced",
        r#","target":"a","target_slot":"main","summary":"main: recorded old replaced by new""#,
    );
    let mut events = proof;
    events.push(replaced);
    assert_eq!(wait(&events, 100, WaitState::Blocked), w_prov(1));
    assert_eq!(wait(&events, 140, WaitState::Blocked), w_due(1, 120, 0));
}
