//! Independent acceptance for #315: information preserves an established proof hold.

use super::{
    Accounting, Activity, Effect, HarnessObservation, Knobs, Observation, PaneState, ProofFrame,
    account_with_activity,
};
use crate::events::Event;
use crate::harness_state::HarnessState;
use crate::procs::Descendancy;
use crate::time::Timestamp;
use crate::watchdog::{
    DoneProgress, QuietKind, WaitProgress, WaitState, done_progress_proof, wait_progress_proof,
};

#[derive(Clone, Copy, Debug)]
enum State {
    Waiting,
    Blocked,
    Done,
}

impl State {
    fn name(self) -> &'static str {
        match self {
            Self::Waiting => "waiting-agent",
            Self::Blocked => "blocked",
            Self::Done => "done",
        }
    }

    fn quiet(self) -> QuietKind {
        match self {
            Self::Waiting => QuietKind::WaitingAgent,
            Self::Blocked => QuietKind::Blocked,
            Self::Done => QuietKind::Done,
        }
    }
}

fn row(at: i64, actor: &str, action: &str, reference: Option<&str>, target: Option<&str>) -> Event {
    let mut event =
        Event::parse_line(r#"{"ts":"2026-11-07T00:00:00Z","actor":"seat","action":"state"}"#)
            .expect("a parsed journal fixture");
    event.ts = Timestamp::from_epoch(at);
    event.actor = actor.to_owned();
    event.action = action.to_owned();
    event.reference = reference.map(str::to_owned);
    event.target = target.map(str::to_owned);
    event
}

fn declaration(at: i64, state: State) -> Event {
    row(at, "seat", "state", Some(state.name()), None)
}

fn information(at: i64, kind: usize) -> Event {
    match kind {
        0 => row(at, "peer", "send", None, Some("seat")),
        1 => row(at, "seat", "send", None, Some("peer")),
        2 => row(at, "seat", "memo", None, None),
        3 => row(at, "seat", "reply", Some("peer-request"), Some("peer")),
        _ => panic!("the four information cases are explicit"),
    }
}

fn journal(state: State, credited: bool) -> (Vec<Event>, i64) {
    let mut events = vec![declaration(1000, state)];
    if credited || matches!(state, State::Done) {
        let mut challenge = row(
            1300,
            "watchdog",
            if matches!(state, State::Done) {
                "done-challenge"
            } else {
                "wait-challenge"
            },
            Some("launch-315"),
            Some("seat"),
        );
        challenge.summary = Some(if matches!(state, State::Done) {
            "confirmation 1/2".to_owned()
        } else {
            format!("{} confirmation 1/2", state.name())
        });
        events.push(challenge);
        events.push(declaration(1310, state));
        (events, 1310)
    } else {
        (events, 1000)
    }
}

fn observation(now: i64, state: State) -> Observation {
    Observation {
        now_epoch: now,
        hash: 7,
        harness: HarnessObservation {
            frame: HarnessState::Idle,
            human_draft: false,
            durable_stale: false,
            declaration: None,
        },
        identity: 1,
        is_dead: false,
        throttle: None,
        limit_notice: None,
        limit_reset: None,
        limit_woke_journal: None,
        auto_continue: false,
        capture_ok: true,
        human_prompt: None,
        throttle_quota: None,
        quiet: Some(state.quiet()),
        ended_wait: None,
        done_progress: DoneProgress::None,
        wait_progress: WaitProgress::None,
        wait_rung: 0,
        wait_ends_elsewhere: false,
        target_moved: None,
        descendancy: Descendancy::Present,
        launch_attempt: crate::tmux::Evidence::Silent,
        outstanding_alert: None,
        unreachable: false,
        failed_elsewhere: false,
        prompt_named: false,
        start: None,
        last_actor_event_age_secs: 10_000,
        declared_age_secs: 0,
        suspended: crate::watchdog::Suspensions::default(),
        sweep: None,
        own_work: crate::session::OwnWork::default(),
        auto_in_flight: None,
        auto_deadline: None,
    }
}

fn cycle(
    prior: &PaneState,
    events: &[Event],
    state: State,
    mut seen: Observation,
    painting: Option<i64>,
) -> Accounting {
    let now = Timestamp::from_epoch(seen.now_epoch);
    let proof = match state {
        State::Done => {
            let (progress, proof) = done_progress_proof(
                events,
                "demo",
                "main",
                "seat",
                Some("launch-315"),
                now,
                300,
                2,
            );
            seen.done_progress = progress;
            proof
        }
        State::Waiting | State::Blocked => {
            let (progress, proof) = wait_progress_proof(
                events,
                "demo",
                "main",
                "seat",
                Some("launch-315"),
                now,
                300,
                2,
                if matches!(state, State::Waiting) {
                    WaitState::WaitingAgent
                } else {
                    WaitState::Blocked
                },
            );
            seen.wait_progress = progress;
            proof
        }
    };
    seen.declared_age_secs = proof.proof.map_or(0, |at| {
        u64::try_from(seen.now_epoch - at.epoch()).expect("a nonnegative declaration age")
    });
    account_with_activity(
        prior,
        &seen,
        &Knobs::default(),
        None,
        &Activity {
            proof: Some(proof),
            painting,
            ..Activity::default()
        },
    )
}

fn established(events: &[Event], state: State, at: i64) -> PaneState {
    // A running watchdog sees a new proof, then a successful non-Busy read
    // one full interval later. No settled knowledge is planted in the fixture.
    let initial = PaneState {
        identity: Some(1),
        proof_frame: Some(ProofFrame::default()),
        ..PaneState::default()
    };
    let fresh = cycle(&initial, events, state, observation(at, state), None);
    let stable = cycle(
        &fresh.next,
        events,
        state,
        observation(at + 60, state),
        None,
    );
    let frame = stable.next.proof_frame.expect("the proof was observed");
    assert_eq!(frame.baseline, Some(7));
    assert!(frame.settled && !frame.moved);
    stable.next
}

fn assert_challenge(booked: &Accounting, state: State, challenge: bool) {
    let pushed = booked.effects.iter().any(|effect| match state {
        State::Done => matches!(effect, Effect::DoneChallenge { .. }),
        State::Waiting | State::Blocked => matches!(effect, Effect::WaitChallenge { .. }),
    });
    assert_eq!(pushed, challenge, "{state:?}: {:?}", booked.effects);
    if matches!(state, State::Done) {
        let held = booked
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::DoneHeld { .. }));
        assert_eq!(
            held, !challenge,
            "last done confirmation: {:?}",
            booked.effects
        );
    }
}

#[test]
fn information_in_the_last_interval_preserves_a_due_wait_hold() {
    for state in [State::Waiting, State::Blocked] {
        for kind in 0..4 {
            for information_at in [1241, 1270, 1299, 1300] {
                let (mut events, at) = journal(state, false);
                let prior = established(&events, state, at);
                events.push(information(information_at, kind));
                let booked = cycle(&prior, &events, state, observation(1300, state), None);
                assert_challenge(&booked, state, false);
            }
        }
    }
}

#[test]
fn repeated_information_holds_each_due_cycle_but_never_renews_the_wait_ceiling() {
    for state in [State::Waiting, State::Blocked] {
        let (mut events, at) = journal(state, false);
        let mut prior = established(&events, state, at);
        for now in [1300, 1330, 1360, 1600, 1899, 1900] {
            events.push(information(now - 1, 2));
            let booked = cycle(&prior, &events, state, observation(now, state), None);
            assert_challenge(&booked, state, now == 1900);
            prior = booked.next;
        }
    }
}

#[test]
fn an_information_record_preserves_the_credited_wait_spacing_and_hold() {
    for state in [State::Waiting, State::Blocked] {
        let (mut events, at) = journal(state, true);
        let prior = established(&events, state, at);
        events.push(information(1880, 0));
        let before_due = cycle(&prior, &events, state, observation(1909, state), None);
        assert_challenge(&before_due, state, false);
        let due = cycle(
            &before_due.next,
            &events,
            state,
            observation(1910, state),
            None,
        );
        assert_challenge(&due, state, false);
        let expired = cycle(&due.next, &events, state, observation(2210, state), None);
        assert_challenge(&expired, state, true);
    }
}

#[test]
fn information_preserves_done_held_after_a_real_answered_challenge() {
    for kind in 0..4 {
        let state = State::Done;
        let (mut events, at) = journal(state, true);
        let prior = established(&events, state, at);
        events.push(information(1580, kind));
        let booked = cycle(&prior, &events, state, observation(1610, state), None);
        assert_challenge(&booked, state, false);
    }
}

#[test]
fn busy_at_the_end_of_the_new_settle_window_ends_both_holds() {
    for state in [State::Waiting, State::Blocked, State::Done] {
        let (mut events, at) = journal(state, false);
        let prior = established(&events, state, at);
        let due = at + 300;
        events.push(information(due - 30, 1));
        let resettling = cycle(&prior, &events, state, observation(due, state), None);
        let mut busy = observation(due + 30, state);
        busy.harness.frame = HarnessState::Busy;
        let booked = cycle(&resettling.next, &events, state, busy, None);
        assert_challenge(&booked, state, true);
        events.push(information(due + 31, 2));
        let later = cycle(
            &booked.next,
            &events,
            state,
            observation(due + 60, state),
            None,
        );
        assert_challenge(&later, state, true);
    }
}

#[test]
fn a_hash_change_after_the_retaken_baseline_ends_both_holds() {
    for state in [State::Waiting, State::Blocked, State::Done] {
        let (mut events, at) = journal(state, false);
        let prior = established(&events, state, at);
        let due = at + 300;
        events.push(information(due - 30, 0));
        let resettling = cycle(&prior, &events, state, observation(due, state), None);
        let mut retake = observation(due + 30, state);
        retake.hash = 8;
        let baseline = cycle(&resettling.next, &events, state, retake, None);
        let mut changed = observation(due + 90, state);
        changed.hash = 9;
        let moved = cycle(&baseline.next, &events, state, changed.clone(), None);
        assert_challenge(&moved, state, true);
        events.push(information(due + 91, 2));
        changed.now_epoch = due + 120;
        let later = cycle(&moved.next, &events, state, changed, None);
        assert_challenge(&later, state, true);
    }
}

#[test]
fn information_cannot_forget_motion_observed_before_the_settle_point_moves() {
    for state in [State::Waiting, State::Blocked, State::Done] {
        let (mut events, at) = journal(state, false);
        let prior = established(&events, state, at);
        let mut changed = observation(at + 120, state);
        changed.hash = 8;
        let moved = cycle(&prior, &events, state, changed.clone(), None);
        events.push(information(at + 280, 3));
        changed.now_epoch = at + 300;
        let due = cycle(&moved.next, &events, state, changed, None);
        assert_challenge(&due, state, true);
    }
}

#[test]
fn a_failed_capture_after_information_does_not_hold_and_does_not_destroy_prior_knowledge() {
    for state in [State::Waiting, State::Blocked, State::Done] {
        let (mut events, at) = journal(state, false);
        let prior = established(&events, state, at);
        let due = at + 300;
        events.push(information(due - 30, 2));
        let resettling = cycle(&prior, &events, state, observation(due, state), None);
        let mut failed = observation(due + 1, state);
        failed.capture_ok = false;
        let unread = cycle(&resettling.next, &events, state, failed, None);
        assert_challenge(&unread, state, true);
        assert_eq!(unread.next.proof_frame, resettling.next.proof_frame);
        let recovered = cycle(
            &unread.next,
            &events,
            state,
            observation(due + 2, state),
            None,
        );
        assert_challenge(&recovered, state, false);
    }
}

#[test]
fn information_before_any_clean_baseline_cannot_create_a_hold() {
    for state in [State::Waiting, State::Blocked, State::Done] {
        let (mut events, at) = journal(state, false);
        let initial = PaneState {
            identity: Some(1),
            proof_frame: Some(ProofFrame::default()),
            ..PaneState::default()
        };
        let fresh = cycle(&initial, &events, state, observation(at, state), None);
        events.push(information(at + 280, 2));
        let due = cycle(
            &fresh.next,
            &events,
            state,
            observation(at + 300, state),
            None,
        );
        assert_challenge(&due, state, true);
    }
}

#[test]
fn a_restart_cannot_recover_a_hold_from_information_alone() {
    for state in [State::Waiting, State::Blocked, State::Done] {
        let (mut events, at) = journal(state, false);
        let prior = PaneState {
            identity: Some(1),
            ..PaneState::default()
        };
        events.push(information(at + 280, 0));
        let due = cycle(&prior, &events, state, observation(at + 300, state), None);
        assert_challenge(&due, state, true);
        let later = cycle(
            &due.next,
            &events,
            state,
            observation(at + 360, state),
            None,
        );
        assert_challenge(&later, state, true);
    }
}

#[test]
fn new_watchdog_painting_keeps_an_established_hold_during_resettling() {
    for state in [State::Waiting, State::Blocked, State::Done] {
        let (events, at) = journal(state, false);
        let prior = established(&events, state, at);
        let booked = cycle(
            &prior,
            &events,
            state,
            observation(at + 300, state),
            Some(at + 280),
        );
        assert_challenge(&booked, state, false);
    }
}

#[test]
fn information_never_replaces_the_first_real_done_challenge() {
    let state = State::Done;
    let mut events = vec![declaration(1000, state)];
    let prior = established(&events, state, 1000);
    events.push(information(1280, 2));
    let due = cycle(&prior, &events, state, observation(1300, state), None);
    assert_challenge(&due, state, true);
}

#[test]
fn actionable_news_during_resettling_still_ends_the_blocked_hold() {
    for action in ["ask", "review"] {
        let state = State::Blocked;
        let (mut events, at) = journal(state, false);
        let prior = established(&events, state, at);
        events.push(information(1270, 2));
        let resettling = cycle(&prior, &events, state, observation(1300, state), None);
        events.push(row(1301, "peer", action, Some("news"), Some("seat")));
        let due = cycle(
            &resettling.next,
            &events,
            state,
            observation(1302, state),
            None,
        );
        assert_challenge(&due, state, true);
    }
}
