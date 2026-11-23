//! Independent #238 acceptance, frozen before the implementation.
//! Cycle clocks, the real suspension fold, accounting and delivery bookkeeping
//! are composed here. External delivery legs still require a wiring review.

use super::{
    Activity, Effect, HarnessObservation, Knobs, Observation, PaneState, Verdict, account,
    account_settling, record_nudge,
};
use crate::events::Event;
use crate::harness_state::HarnessState;
use crate::procs::Descendancy;
use crate::time::Timestamp;
use crate::watchdog::{
    CycleClock, DoneProgress, QuietKind, Suspensions, SweepObservation, WaitProgress,
    host_suspended, settle,
};
use std::time::{Duration, Instant, UNIX_EPOCH};

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
        declared_age_secs: 400,
        suspended: Suspensions::default(),
        sweep: None,
        own_work: crate::session::OwnWork::default(),
        auto_in_flight: None,
        auto_deadline: None,
    }
}

fn delivery(effect: &Effect) -> bool {
    match effect {
        Effect::Nudge
        | Effect::DoneChallenge { .. }
        | Effect::DoneHeld { .. }
        | Effect::WaitHeld { .. }
        | Effect::WaitChallenge { .. }
        | Effect::TargetMoved { .. }
        | Effect::LimitWake { .. }
        | Effect::SweepNudge
        | Effect::AnswerTrust { .. } => true,
        Effect::ReseatContinuation { owed, .. } => {
            owed.phase == crate::meta::ContinuationPhase::Owed
        }
        Effect::Emit { .. }
        | Effect::Notify(_)
        | Effect::QuotaRefresh
        | Effect::ReconcileWedge
        | Effect::Note(_) => false,
    }
}

fn idle_prior() -> PaneState {
    PaneState {
        identity: Some(1),
        prev_hash: Some(7),
        last_hash_change: Some(1_000),
        idle_since_epoch: Some(1_000),
        nudge_count: 1,
        undelivered_streak: 1,
        ..PaneState::default()
    }
}

#[test]
fn sleepgap_dark_wake_spends_no_budget_and_real_wake_redrives_normally() {
    let knobs = Knobs::default();
    let start = Instant::now();
    let clock = |wall, mono| CycleClock {
        wall,
        mono: start + Duration::from_secs(mono),
    };
    let mut previous = clock(1_400, 0);
    let mut held = 0;
    let mut events = Vec::new();
    let mut state = idle_prior();
    for (wall, mono, expected_hold) in [
        (8_600, 0, true),
        (8_660, 60, true),
        (15_860, 60, true),
        (15_920, 120, true),
        (15_980, 180, false),
    ] {
        let now = clock(wall, mono);
        let gap = host_suspended(previous, now, knobs.interval_secs);
        held = settle(held, gap);
        if let Some(gap) = gap {
            // Journal spelling is an independent input, not the summary writer.
            events.push(Event::parse_line(&format!(
                r#"{{"ts":"{}","actor":"watchdog","action":"host-suspended","summary":"suspended {}s: wall clock +{}s, monotonic clock +{}s"}}"#,
                Timestamp::from_epoch(wall), gap.secs, gap.wall_secs, gap.mono_secs
            )).expect("suspension event"));
        }
        let mut observation = seen(wall);
        observation.suspended = Suspensions::read(&events);
        let booked = account_settling(
            &state,
            &observation,
            &knobs,
            None,
            &Activity::default(),
            held > 0,
        );
        assert_eq!(booked.next.nudge_count, 1, "no blanket wake reset: {wall}");
        assert_eq!(booked.next.undelivered_streak, 1, "no retry spent: {wall}");
        assert_eq!(booked.next.idle_since_epoch, Some(1_000));
        if expected_hold {
            assert!(
                !booked.effects.iter().any(delivery),
                "wall={wall}: {:?}",
                booked.effects
            );
        } else {
            assert_eq!(booked.effects, [Effect::Nudge]);
        }
        assert_eq!(held > 0, expected_hold, "wall={wall}");
        state = booked.next;
        previous = now;
    }
    assert!(record_nudge(&mut state, true, &knobs, "idle 8m").is_empty());
    assert_eq!(state.nudge_count, 2);
    assert_eq!(state.undelivered_streak, 0);
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "independent due-delivery fixtures share one exhaustive hold oracle"
)]
fn sleepgap_every_due_fold_delivery_waits_without_consuming_its_delivery_clock() {
    let knobs = Knobs::default();
    let mut cases = Vec::new();
    cases.push(("idle", idle_prior(), seen(2_000), None));
    let mut unknown = seen(2_000);
    unknown.harness.frame = HarnessState::Unknown;
    cases.push(("unknown frame", idle_prior(), unknown, None));
    let mut done = seen(2_000);
    done.quiet = Some(QuietKind::Done);
    done.done_progress = DoneProgress::ChallengeDue {
        confirmations: 0,
        required: 2,
        done_age_secs: 400,
        attempts: 0,
    };
    cases.push(("done", PaneState::default(), done, None));
    for kind in [QuietKind::WaitingAgent, QuietKind::Blocked] {
        let mut wait = seen(2_000);
        wait.quiet = Some(kind);
        wait.wait_progress = WaitProgress::ChallengeDue {
            confirmations: 0,
            required: 2,
            wait_age_secs: 400,
            attempts: 0,
        };
        cases.push(("wait", PaneState::default(), wait, None));
    }
    let mut moved = seen(2_000);
    moved.quiet = Some(QuietKind::WaitingAgent);
    moved.target_moved = Some(crate::wait_on::Moved {
        class: crate::wait_on::Class::Done,
        ordinal: 1,
        booked: 0,
        attempts: 0,
        current: true,
        at: Timestamp::from_epoch(1_900),
        reason: "finished".to_owned(),
    });
    cases.push(("target moved", PaneState::default(), moved, None));
    let reset = crate::autoreseat::LimitReset {
        at: 1_000,
        short: true,
    };
    let mut limited = seen(2_000);
    limited.throttle = Some(crate::watchdog::Throttle::LimitReached);
    limited.limit_reset = Some(reset);
    let latched = PaneState {
        identity: Some(1),
        limit_since: Some(900),
        limit_reset: Some(reset),
        ..PaneState::default()
    };
    cases.push(("limit wake", latched, limited, None));
    let mut sweep = seen(2_000);
    sweep.sweep = Some(SweepObservation::new(
        UNIX_EPOCH + Duration::from_secs(2_000),
        None,
    ));
    cases.push(("sweep", PaneState::default(), sweep, None));
    let mut trust = seen(2_000);
    trust.human_prompt = Some(crate::watchdog::HumanPrompt {
        kind: crate::watchdog::DialogKind::FolderTrust,
        warnings: Vec::new(),
        question: "Do you trust this folder?".to_owned(),
        keys: "Enter".to_owned(),
        trust: None,
        dismiss: false,
        differs: None,
    });
    cases.push((
        "trust",
        PaneState::default(),
        trust,
        Some(crate::autotrust::Due::Yes),
    ));
    for (name, prior, observation, trust) in cases {
        let ordinary = account_settling(
            &prior,
            &observation,
            &knobs,
            trust.as_ref(),
            &Activity::default(),
            false,
        );
        assert!(
            ordinary.effects.iter().any(delivery),
            "fixture must be due: {name}: {:?}",
            ordinary.effects
        );
        let held = account_settling(
            &prior,
            &observation,
            &knobs,
            trust.as_ref(),
            &Activity::default(),
            true,
        );
        assert!(
            !held.effects.iter().any(delivery),
            "held {name}: {:?}",
            held.effects
        );
        assert_eq!(held.next.sweep.last_sweep, prior.sweep.last_sweep, "{name}");
        assert_eq!(
            held.next.sweep.last_delivered, prior.sweep.last_delivered,
            "{name}"
        );
        assert_eq!(held.next.sweep.fails, prior.sweep.fails, "{name}");
        assert_eq!(
            held.next.limit_wake_attempts, prior.limit_wake_attempts,
            "{name}"
        );
    }
}

#[test]
fn sleepgap_no_gap_keeps_the_pinned_idle_trace_and_backwards_age_clamp() {
    let knobs = Knobs::default();
    let mut state = PaneState::default();
    let mut trace = Vec::new();
    for now in [1_000, 1_060, 1_299, 1_300] {
        let booked = account(&state, &seen(now), &knobs);
        trace.push((booked.verdict, booked.effects.clone()));
        state = booked.next;
    }
    assert_eq!(
        trace,
        [
            (Verdict::Idle, vec![]),
            (Verdict::Idle, vec![]),
            (Verdict::Idle, vec![]),
            (Verdict::Idle, vec![Effect::Nudge]),
        ]
    );
    let backwards = account(&state, &seen(900), &knobs);
    assert_eq!(backwards.verdict, Verdict::Idle);
    assert!(backwards.effects.is_empty());
    assert_eq!(backwards.next.idle_since_epoch, Some(1_000));
}

#[test]
fn sleepgap_held_trust_answer_keeps_the_prompt_naming_at_its_observed_threshold() {
    let knobs = Knobs::default();
    let mut observation = seen(2_000);
    observation.human_prompt = Some(crate::watchdog::HumanPrompt {
        kind: crate::watchdog::DialogKind::FolderTrust,
        warnings: Vec::new(),
        question: "Do you trust this folder?".to_owned(),
        keys: "Enter".to_owned(),
        trust: None,
        dismiss: false,
        differs: None,
    });
    let prior = PaneState {
        identity: Some(1),
        human_prompt_streak: knobs.human_prompt_cycles - 1,
        ..PaneState::default()
    };
    let ordinary = account_settling(
        &prior,
        &observation,
        &knobs,
        Some(&crate::autotrust::Due::Yes),
        &Activity::default(),
        false,
    );
    assert!(
        ordinary
            .effects
            .contains(&Effect::AnswerTrust { name: true })
    );
    let held = account_settling(
        &prior,
        &observation,
        &knobs,
        Some(&crate::autotrust::Due::Yes),
        &Activity::default(),
        true,
    );
    assert!(!held.effects.iter().any(delivery));
    assert_eq!(held.next.human_prompt_streak, knobs.human_prompt_cycles);
    assert!(
        held.effects.iter().any(|effect| matches!(
            effect,
            Effect::Emit {
                action: "human-prompt",
                ..
            }
        )),
        "a held keystroke must not lose the episode's one naming: {:?}",
        held.effects
    );
    let resumed = account_settling(
        &held.next,
        &observation,
        &knobs,
        Some(&crate::autotrust::Due::Yes),
        &Activity::default(),
        false,
    );
    assert!(
        resumed
            .effects
            .contains(&Effect::AnswerTrust { name: false })
    );
}
