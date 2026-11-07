//! Independent behavioural acceptance for #225 proof holds and #283 child leases.

use super::{
    Activity, Effect, FrameSample, FrameVerdict, HarnessObservation, Knobs, Observation, PaneState,
    ProofFrame, Verdict, account, account_with_activity, done_held_due, proof_frame_step,
    task_deferred, task_lease, wait_held,
};
use crate::harness_state::HarnessState;
use crate::procs::Descendancy;
use crate::time::Timestamp;
use crate::watchdog::{DoneProgress, ProofQuiet, QuietKind, Throttle, WaitProgress};

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
        sweep: None,
        own_work: crate::session::OwnWork::default(),
        auto_in_flight: None,
        auto_deadline: None,
    }
}

fn proof(at: i64) -> ProofQuiet {
    ProofQuiet {
        proof: Some(Timestamp::from_epoch(at)),
        unchanged: true,
        settle: Some(Timestamp::from_epoch(at)),
    }
}

fn stable(at: i64) -> PaneState {
    PaneState {
        identity: Some(1),
        proof_frame: Some(ProofFrame {
            proof: Some(at),
            settle: Some(at),
            baseline: Some(7),
            settled: true,
            moved: false,
        }),
        ..PaneState::default()
    }
}

fn due_wait(age: u64) -> WaitProgress {
    WaitProgress::ChallengeDue {
        confirmations: 0,
        required: 2,
        wait_age_secs: age,
        attempts: 0,
    }
}

fn due_done(confirmations: u8, required: u8) -> DoneProgress {
    DoneProgress::ChallengeDue {
        confirmations,
        required,
        done_age_secs: 300,
        attempts: 0,
    }
}

fn sample(hash: u64) -> FrameSample {
    FrameSample {
        capture_ok: true,
        busy: false,
        hash,
    }
}

#[test]
fn real_motion_stays_sticky_across_informational_settling_until_a_new_proof() {
    let (initial, first) = proof_frame_step(
        Some(ProofFrame::default()),
        Some(1000),
        Some(1000),
        sample(7),
        1000,
        60,
    );
    assert_eq!(first, FrameVerdict::Settling);
    let (baseline, quiet) = proof_frame_step(initial, Some(1000), Some(1000), sample(7), 1060, 60);
    assert_eq!(quiet, FrameVerdict::Unmoved);
    let (motion, changed) = proof_frame_step(baseline, Some(1000), Some(1000), sample(8), 1120, 60);
    assert_eq!(changed, FrameVerdict::Moved);
    let (settling, after_information) =
        proof_frame_step(motion, Some(1000), Some(1130), sample(9), 1140, 60);
    assert_eq!(after_information, FrameVerdict::Moved);
    let (rebased, after_settle) =
        proof_frame_step(settling, Some(1000), Some(1130), sample(9), 1190, 60);
    assert_eq!(
        after_settle,
        FrameVerdict::Moved,
        "P2: taking a new baseline must not forget observed work"
    );
    assert!(rebased.expect("stored proof frame").moved);
    let (new_proof, _) = proof_frame_step(rebased, Some(1200), Some(1200), sample(10), 1200, 60);
    let (_, renewed) = proof_frame_step(new_proof, Some(1200), Some(1200), sample(10), 1260, 60);
    assert_eq!(
        renewed,
        FrameVerdict::Unmoved,
        "a new proof can establish a new quiet condition"
    );
}

#[test]
fn failed_capture_and_restart_do_not_prove_an_unchanged_frame() {
    let prior = stable(1000).proof_frame;
    let (next, verdict) = proof_frame_step(
        prior,
        Some(1000),
        Some(1000),
        FrameSample {
            capture_ok: false,
            busy: false,
            hash: 99,
        },
        1300,
        60,
    );
    assert_ne!(verdict, FrameVerdict::Unmoved);
    assert_eq!(next, prior, "a failed read changes no stored proof frame");
    let (_, restart) = proof_frame_step(None, Some(1000), Some(1000), sample(7), 1300, 60);
    assert_ne!(
        restart,
        FrameVerdict::Unmoved,
        "a restarted daemon cannot know motion since an old proof"
    );
}

#[test]
fn an_unchanged_wait_holds_only_before_the_three_cadence_ceiling() {
    for age in [300, 600, 899] {
        assert!(
            wait_held(due_wait(age), proof(1000), FrameVerdict::Unmoved, 300),
            "{age}"
        );
    }
    assert!(!wait_held(
        due_wait(900),
        proof(1000),
        FrameVerdict::Unmoved,
        300
    ));
    for frame in [FrameVerdict::Moved, FrameVerdict::Settling] {
        assert!(!wait_held(due_wait(300), proof(1000), frame, 300));
    }
    let changed = ProofQuiet {
        unchanged: false,
        ..proof(1000)
    };
    assert!(!wait_held(
        due_wait(300),
        changed,
        FrameVerdict::Unmoved,
        300
    ));
}

#[test]
fn accounting_defers_the_wait_push_but_still_pushes_at_expiry_or_changed_evidence() {
    let knobs = Knobs::default();
    let activity = Activity {
        proof: Some(proof(1000)),
        ..Activity::default()
    };
    for (now, hash, capture_ok, must_push) in [
        (1300, 7, true, false),
        (1899, 7, true, false),
        (1900, 7, true, true),
        (1300, 8, true, true),
        (1300, 7, false, true),
    ] {
        let mut observation = seen(now);
        observation.hash = hash;
        observation.capture_ok = capture_ok;
        observation.quiet = Some(QuietKind::WaitingAgent);
        observation.declared_age_secs = u64::try_from(now - 1000).expect("nonnegative age");
        observation.wait_progress = due_wait(observation.declared_age_secs);
        let booked = account_with_activity(&stable(1000), &observation, &knobs, None, &activity);
        assert_eq!(
            booked
                .effects
                .iter()
                .any(|effect| matches!(effect, Effect::WaitChallenge { .. })),
            must_push,
            "now={now}, hash={hash}, capture={capture_ok}"
        );
    }
}

#[test]
fn done_held_never_replaces_the_first_challenge_or_unproved_work() {
    assert!(!done_held_due(
        due_done(0, 2),
        proof(1000),
        FrameVerdict::Unmoved
    ));
    assert!(done_held_due(
        due_done(1, 2),
        proof(1000),
        FrameVerdict::Unmoved
    ));
    assert!(!done_held_due(
        due_done(1, 3),
        proof(1000),
        FrameVerdict::Unmoved
    ));
    for frame in [FrameVerdict::Moved, FrameVerdict::Settling] {
        assert!(!done_held_due(due_done(1, 2), proof(1000), frame));
    }
    assert!(!done_held_due(
        due_done(1, 2),
        ProofQuiet {
            unchanged: false,
            ..proof(1000)
        },
        FrameVerdict::Unmoved
    ));
    let knobs = Knobs::default();
    for confirmations in [0, 1] {
        let mut observation = seen(1300);
        observation.quiet = Some(QuietKind::Done);
        observation.done_progress = due_done(confirmations, 2);
        let booked = account_with_activity(
            &stable(1000),
            &observation,
            &knobs,
            None,
            &Activity {
                proof: Some(proof(1000)),
                ..Activity::default()
            },
        );
        assert_eq!(
            booked
                .effects
                .iter()
                .any(|effect| matches!(effect, Effect::DoneHeld { .. })),
            confirmations == 1
        );
        assert_eq!(
            booked
                .effects
                .iter()
                .any(|effect| matches!(effect, Effect::DoneChallenge { .. })),
            confirmations == 0
        );
    }
}

#[test]
fn child_lease_expires_without_renewal_and_only_a_proven_gap_reopens_it() {
    let knobs = Knobs::default();
    let since = task_lease(None, Some(true), 1000);
    assert_eq!(since, Some(1000));
    assert_eq!(task_lease(since, Some(true), 2000), since);
    assert_eq!(task_lease(since, None, 3000), since);
    assert!(!task_deferred(since, None, 3000, &knobs));
    assert!(task_deferred(since, Some(true), 8199, &knobs));
    assert!(!task_deferred(since, Some(true), 8200, &knobs));
    assert_eq!(
        task_lease(since, Some(true), 9000),
        since,
        "persistent descendants do not renew an expired lease"
    );
    assert_eq!(task_lease(since, Some(false), 9000), None);
    assert_eq!(task_lease(None, Some(true), 9060), Some(9060));
    let disabled = Knobs {
        idle_nudge_secs: 0,
        ..knobs
    };
    assert!(!task_deferred(since, Some(true), 1000, &disabled));
}

#[test]
fn child_activity_defers_idle_wording_then_spends_the_normal_budget_at_the_cap() {
    let knobs = Knobs::default();
    let activity = Activity {
        task_running: Some(true),
        ..Activity::default()
    };
    let first = account_with_activity(&PaneState::default(), &seen(1000), &knobs, None, &activity);
    let mut after_declaration = seen(1300);
    after_declaration.hash = 8;
    after_declaration.harness.declaration = Some(44);
    let second = account_with_activity(&first.next, &after_declaration, &knobs, None, &activity);
    assert_eq!(
        second.next.task_since,
        Some(1000),
        "a declaration and output cannot renew the child lease"
    );
    let mut near_cap = after_declaration.clone();
    near_cap.now_epoch = 8199;
    near_cap.hash = 9;
    let quiet = account_with_activity(&second.next, &near_cap, &knobs, None, &activity);
    assert_eq!(quiet.verdict, Verdict::Idle);
    assert!(
        !quiet
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::Nudge))
    );
    assert_eq!(quiet.next.task_since, Some(1000));
    near_cap.now_epoch = 8200;
    let expired = account_with_activity(&quiet.next, &near_cap, &knobs, None, &activity);
    assert!(
        expired
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::Nudge)),
        "a live stuck child eventually gets the ordinary nudge"
    );
}

#[test]
fn child_activity_also_defers_the_unknown_frame_idle_wording() {
    let knobs = Knobs::default();
    let prior = PaneState {
        identity: Some(1),
        prev_hash: Some(7),
        last_hash_change: Some(0),
        ..PaneState::default()
    };
    let mut observation = seen(2000);
    observation.harness.frame = HarnessState::Unknown;
    let ordinary = account(&prior, &observation, &knobs);
    assert!(
        ordinary
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::Nudge)),
        "the control reaches the legacy stale reminder"
    );
    let booked = account_with_activity(
        &prior,
        &observation,
        &knobs,
        None,
        &Activity {
            task_running: Some(true),
            ..Activity::default()
        },
    );
    assert!(
        !booked
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::Nudge))
    );
}

#[test]
fn child_activity_preserves_dead_limit_and_throttle_precedence() {
    let knobs = Knobs::default();
    let activity = Activity {
        task_running: Some(true),
        ..Activity::default()
    };
    let mut dead = seen(1000);
    dead.is_dead = true;
    dead.descendancy = Descendancy::Absent;
    assert_eq!(
        account_with_activity(&PaneState::default(), &dead, &knobs, None, &activity).verdict,
        Verdict::Dead
    );
    let mut limited = seen(1000);
    limited.throttle = Some(Throttle::LimitReached);
    assert_eq!(
        account_with_activity(&PaneState::default(), &limited, &knobs, None, &activity).verdict,
        Verdict::Limit
    );
    limited.throttle = Some(Throttle::Throttled);
    assert_eq!(
        account_with_activity(&PaneState::default(), &limited, &knobs, None, &activity).verdict,
        Verdict::Throttled
    );
    let mut prompted = seen(1000);
    prompted.human_prompt = Some(crate::watchdog::HumanPrompt {
        question: "Do you trust the contents of this project?".to_owned(),
        keys: "enter Confirm".to_owned(),
        trust: None,
        dismiss: false,
    });
    let first = account_with_activity(&PaneState::default(), &prompted, &knobs, None, &activity);
    let second = account_with_activity(&first.next, &prompted, &knobs, None, &activity);
    assert_eq!(second.verdict, Verdict::HumanPrompt);
}

#[test]
fn default_activity_preserves_the_existing_public_accounting_path() {
    let knobs = Knobs::default();
    let prior = PaneState {
        identity: Some(1),
        idle_since_epoch: Some(1000),
        ..PaneState::default()
    };
    let observation = seen(1300);
    let old = account(&prior, &observation, &knobs);
    let explicit = account_with_activity(&prior, &observation, &knobs, None, &Activity::default());
    assert_eq!(old.next, explicit.next);
    assert_eq!(old.effects, explicit.effects);
    assert_eq!(old.verdict, explicit.verdict);
}
