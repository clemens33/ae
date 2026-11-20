//! Daemon acceptance for the wait-challenge diet and the sleep-aware clocks
//! (protocol quick wins I14 and I15), frozen with protowd-nav: D3, D7, D9,
//! S2, S4 and S9 through `account`.

use super::{Effect, HarnessObservation, Knobs, Observation, PaneState, Verdict, account};
use crate::events::Event;
use crate::harness_state::HarnessState;
use crate::procs::Descendancy;
use crate::time::Timestamp;
use crate::watchdog::{
    DoneProgress, HOST_SUSPENDED_ACTION, QuietKind, Suspended, Suspensions, WaitProgress,
    suspended_summary,
};

fn seen(now: i64) -> Observation {
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
        quiet: None,
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
        suspended: Suspensions::default(),
        sweep: None,
        own_work: crate::session::OwnWork::default(),
        auto_in_flight: None,
        auto_deadline: None,
    }
}

fn suspension(at: i64, secs: u64) -> Suspensions {
    let mut event =
        Event::parse_line(r#"{"ts":"2026-11-11T00:00:00Z","actor":"watchdog","action":"x"}"#)
            .expect("independent event fixture");
    event.ts = Timestamp::from_epoch(at);
    event.action = HOST_SUSPENDED_ACTION.to_owned();
    event.summary = Some(suspended_summary(Suspended {
        secs,
        wall_secs: secs + 60,
        mono_secs: 60,
    }));
    Suspensions::read(&[event])
}

fn waiting(now: i64, age: u64, rung: u8) -> Observation {
    Observation {
        quiet: Some(QuietKind::WaitingAgent),
        wait_progress: WaitProgress::Provisional {
            confirmations: 1,
            required: 2,
        },
        wait_rung: rung,
        declared_age_secs: age,
        ..seen(now)
    }
}

#[test]
fn d3_ceiling_escalation_follows_the_credited_rung() {
    let knobs = Knobs::default();
    let prior = PaneState::default();
    let quiet = Verdict::Quiet(QuietKind::WaitingAgent);
    let blocked = Verdict::Quiet(QuietKind::Blocked);
    assert_eq!(
        account(&prior, &waiting(5_000, 1_199, 0), &knobs).verdict,
        quiet
    );
    assert_eq!(
        account(&prior, &waiting(5_000, 1_200, 0), &knobs).verdict,
        blocked,
        "rung 0 keeps 1200s"
    );
    assert_eq!(
        account(&prior, &waiting(5_000, 1_200, 3), &knobs).verdict,
        quiet,
        "rung 3 is not blocked at 20m"
    );
    assert_eq!(
        account(&prior, &waiting(5_000, 2_699, 3), &knobs).verdict,
        quiet
    );
    assert_eq!(
        account(&prior, &waiting(5_000, 2_700, 3), &knobs).verdict,
        blocked
    );

    // A due challenge past the rung-3 ceiling names it.
    let mut due = waiting(5_000, 2_700, 3);
    due.wait_progress = WaitProgress::ChallengeDue {
        confirmations: 0,
        required: 2,
        wait_age_secs: 2_700,
        attempts: 0,
    };
    let booked = account(&prior, &due, &knobs);
    assert!(
        booked.effects.iter().any(|effect| matches!(
            effect,
            Effect::WaitChallenge {
                escalated: true,
                ..
            }
        )),
        "{:?}",
        booked.effects
    );

    // The target-moved push is not suppressed by the rung.
    let mut moved = waiting(5_000, 600, 3);
    moved.target_moved = Some(crate::wait_on::Moved {
        class: crate::wait_on::Class::Done,
        ordinal: 1,
        booked: 0,
        attempts: 0,
        current: true,
        at: Timestamp::from_epoch(4_900),
        reason: "landed".to_owned(),
    });
    let told = account(&prior, &moved, &knobs);
    assert!(
        told.effects
            .iter()
            .any(|effect| matches!(effect, Effect::TargetMoved { .. })),
        "{:?}",
        told.effects
    );
}

#[test]
fn d7_a_lapsed_rung_three_wait_resumes_the_nudge_budget_past_its_ceiling() {
    let knobs = Knobs::default();
    let mut lapsed = waiting(5_000, 2_700, 3);
    lapsed.wait_progress = WaitProgress::Lapsed {
        confirmations: 0,
        required: 2,
    };
    let booked = account(&PaneState::default(), &lapsed, &knobs);
    assert_eq!(booked.verdict, Verdict::Quiet(QuietKind::Blocked));
    assert!(
        booked.effects.contains(&Effect::Nudge),
        "{:?}",
        booked.effects
    );
}

#[test]
fn s4_idle_nudge_does_not_burst_after_wake() {
    let knobs = Knobs::default();
    // Idle since 1000; asleep from 1120 to 8320; the wake cycle at 8320.
    let prior = PaneState {
        identity: Some(1),
        idle_since_epoch: Some(1_000),
        ..PaneState::default()
    };
    let woke = Observation {
        suspended: suspension(8_320, 7_200),
        ..seen(8_320)
    };
    let booked = account(&prior, &woke, &knobs);
    assert!(
        !booked.effects.contains(&Effect::Nudge),
        "{:?}",
        booked.effects
    );
    assert_eq!(
        booked.next.idle_since_epoch,
        Some(1_000),
        "the wall record is kept"
    );
    // 180 awake seconds later the ordinary 300s idle nudge falls due.
    let later = Observation {
        suspended: suspension(8_320, 7_200),
        ..seen(8_500)
    };
    assert!(
        account(&booked.next, &later, &knobs)
            .effects
            .contains(&Effect::Nudge)
    );
    // Without the record the same wall age nudges at once: the oracle.
    assert!(
        account(&prior, &seen(8_320), &knobs)
            .effects
            .contains(&Effect::Nudge)
    );
}

#[test]
fn s9_zero_cadence_keeps_the_marker_and_never_nudges_after_wake() {
    let zero = Knobs {
        idle_nudge_secs: 0,
        ..Knobs::default()
    };
    let prior = PaneState::default();
    assert_eq!(
        account(&prior, &waiting(9_000, 1_199, 0), &zero).verdict,
        Verdict::Quiet(QuietKind::WaitingAgent)
    );
    let past = account(&prior, &waiting(9_000, 1_200, 0), &zero);
    assert_eq!(past.verdict, Verdict::Quiet(QuietKind::Blocked));
    assert!(!past.effects.contains(&Effect::Nudge));
}
