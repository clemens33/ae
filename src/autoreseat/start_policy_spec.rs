//! Frozen restart policy: quota urgency keeps delivery and human-input guards.
use super::{
    Decision, Episode, Frame, HoldReason, Outcome, Pane, Trigger, Waits, Window, decide,
    decide_with_waits, exhausted, quiet_pair, relieved, start_fact, urgent, urgent_frame,
};
use crate::quota::{Status, WindowKey};
use crate::time::Timestamp;

const NOW: i64 = 10_000;

// Sanitized measured Claude Code 2.1.316 resume. The reply can be longer than
// the visible pane; no transcript marker is required by the empty-box proof.
const REAL_RESUMED: &str = "Claude Code v2.1.316\n⏺ ok\n\n\
tmux detected - scroll with PgUp/PgDn\n\
────────────────────────────────────────────────────────────\n\
❯ \n\
────────────────────────────────────────────────────────────\n\
  🧠 Haiku 5.5 (medium)  📁 project  🌿 branch  ctx 5%  $0.01\n\
  ⏵⏵ bypass permissions on (shift+tab to cycle)\n";

#[test]
fn quiet_start_proof_requires_two_identical_complete_empty_nonbusy_frames() {
    use crate::harness_state::{HarnessState, classify};
    use crate::tool::ToolKind;

    assert_eq!(
        classify(REAL_RESUMED, ToolKind::Claude),
        HarnessState::Unknown
    );
    assert!(quiet_pair(Some(REAL_RESUMED), Some(REAL_RESUMED), "claude"));
    let scrolled = REAL_RESUMED.replace("⏺ ok", "reply continued from above the pane");
    assert!(quiet_pair(Some(&scrolled), Some(&scrolled), "claude"));
    for (first, second) in [
        (None, None),
        (None, Some(REAL_RESUMED)),
        (Some(REAL_RESUMED), None),
    ] {
        assert!(
            !quiet_pair(first, second, "claude"),
            "an absent read proves nothing"
        );
    }
    let changed = REAL_RESUMED.replace("tmux detected", "tmux focus-events off");
    assert!(!quiet_pair(Some(REAL_RESUMED), Some(&changed), "claude"));
    assert!(!quiet_pair(Some(&changed), Some(REAL_RESUMED), "claude"));
    let missing_footer = REAL_RESUMED.replace(
        "🧠 Haiku 5.5 (medium)  📁 project  🌿 branch  ctx 5%  $0.01",
        "missing status footer",
    );
    assert_ne!(
        missing_footer, REAL_RESUMED,
        "footer mutation must take effect"
    );
    for rejected in [
        String::new(),
        "unrecognized text without an input box".to_owned(),
        REAL_RESUMED.replace("❯ ", "❯ human draft"),
        REAL_RESUMED.replace(
            "tmux detected - scroll with PgUp/PgDn",
            "✻ Working… (esc to interrupt)",
        ),
        missing_footer,
        include_str!("../../tests/fixtures/claude-trust/claude-trust-modal-80x24.txt").to_owned(),
    ] {
        assert!(
            !quiet_pair(Some(&rejected), Some(&rejected), "claude"),
            "{rejected}"
        );
    }
    assert!(!quiet_pair(
        Some(REAL_RESUMED),
        Some(REAL_RESUMED),
        "unsupported"
    ));
}

#[test]
fn quiet_proof_promotes_only_unproven_and_does_not_change_the_frame_classifier() {
    assert_eq!(urgent_frame(Frame::Unproven, true), Frame::Clear);
    assert_eq!(urgent_frame(Frame::Unproven, false), Frame::Unproven);
    for frame in [Frame::Clear, Frame::Busy, Frame::Draft, Frame::Unread] {
        assert_eq!(urgent_frame(frame, true), frame);
        assert_eq!(urgent_frame(frame, false), frame);
    }
}

fn window(used: f64, at: i64, status: Status) -> Window {
    Window {
        judged: used,
        critical: used >= 95.0,
        long: false,
        observed_at: at,
        resets_at: Some(NOW + 3_600),
        status,
        label: "session 5h".to_owned(),
    }
}

fn key(bucket: &str) -> WindowKey {
    WindowKey {
        bucket: bucket.to_owned(),
        qualifier: None,
        minutes: Some(300),
    }
}

fn episode(trigger: Trigger) -> Episode {
    Episode::opened_by(
        Timestamp::from_epoch(NOW - 1),
        trigger,
        "headroom 100% session 5h".to_owned(),
    )
}

fn pane() -> Pane {
    Pane {
        frame: Frame::Clear,
        human_prompt: false,
        client_input: None,
    }
}

#[test]
fn completed_current_start_is_required_and_future_or_old_markers_do_not_count() {
    assert_eq!(
        start_fact(Some(NOW - 20), Some(NOW - 10), NOW),
        Some(NOW - 10)
    );
    for (started, settled) in [
        (None, None),
        (Some(NOW - 20), None),
        (None, Some(NOW - 10)),
        (Some(NOW - 20), Some(NOW - 21)),
        (Some(NOW - 20), Some(NOW + 1)),
        (Some(0), Some(NOW - 10)),
        (Some(-1), Some(NOW - 10)),
    ] {
        assert_eq!(
            start_fact(started, settled, NOW),
            None,
            "started={started:?} settled={settled:?}"
        );
    }
    assert!(urgent(Some(NOW), NOW, true, false));
    assert!(urgent(Some(NOW), NOW + 360, true, false));
    assert!(!urgent(Some(NOW), NOW - 1, true, false));
    assert!(!urgent(Some(NOW), NOW + 361, true, false));
    assert!(!urgent(None, NOW, true, false));
    assert!(!urgent(Some(NOW), NOW, false, false));
    assert!(
        !urgent(Some(NOW), NOW, true, true),
        "armed brief keeps its incarnation"
    );
}

#[test]
fn fresh_effective_exhaustion_is_required_even_for_short_windows() {
    assert!(
        exhausted(
            &[(key("session"), window(100.0, NOW, Status::Fresh))],
            false
        )
        .is_some()
    );
    assert!(exhausted(&[(key("session"), window(99.9, NOW, Status::Fresh))], false).is_none());
    for status in [
        Status::Stale,
        Status::Unknown,
        Status::ReadError,
        Status::Unsupported,
        Status::Truncated,
    ] {
        assert!(
            exhausted(&[(key("session"), window(100.0, NOW, status))], false).is_none(),
            "{status:?}"
        );
    }
    assert!(exhausted(&[], false).is_none());
    assert!(
        exhausted(&[(key("session"), window(3.0, NOW, Status::Fresh))], true).is_some(),
        "fresh proven account spend cap outranks window usage"
    );
}

#[test]
fn newest_same_window_controls_and_equal_stamp_conflict_is_ambiguous() {
    for reverse in [false, true] {
        for (old, new, expected) in [(100.0, 10.0, false), (10.0, 100.0, true)] {
            let mut rows = vec![
                (key("session"), window(old, NOW - 10, Status::Fresh)),
                (key("session"), window(new, NOW, Status::Fresh)),
            ];
            if reverse {
                rows.reverse();
            }
            assert_eq!(
                exhausted(&rows, false).is_some(),
                expected,
                "newest observed evidence controls independently of iteration order"
            );
        }
        let mut conflict = vec![
            (key("session"), window(100.0, NOW, Status::Fresh)),
            (key("session"), window(10.0, NOW, Status::Fresh)),
        ];
        if reverse {
            conflict.reverse();
        }
        assert!(
            exhausted(&conflict, false).is_none(),
            "equal stamp opposed readings have no winner"
        );
    }
    let independent = vec![
        (key("session"), window(100.0, NOW - 10, Status::Fresh)),
        (key("other-window"), window(10.0, NOW, Status::Fresh)),
    ];
    assert!(
        exhausted(&independent, false).is_some(),
        "healthy independent window cannot erase an exhausted constraint"
    );
}

#[test]
fn rearming_requires_proven_relief_not_equal_time_ambiguity() {
    for reverse in [false, true] {
        for (full_at, healthy_at, expected) in [
            (NOW - 1, NOW, true),
            (NOW, NOW, false),
            (NOW, NOW - 1, false),
        ] {
            let mut rows = vec![
                (key("session"), window(100.0, full_at, Status::Fresh)),
                (key("session"), window(10.0, healthy_at, Status::Fresh)),
            ];
            if reverse {
                rows.reverse();
            }
            assert_eq!(
                relieved(&rows).map(|row| row.judged),
                expected.then_some(10.0),
                "only a strictly newer healthy reading permits rearming"
            );
        }
    }
    assert!(
        relieved(&[
            (key("session"), window(10.0, NOW, Status::Fresh)),
            (key("other-window"), window(100.0, NOW - 1, Status::Fresh)),
        ])
        .is_none(),
        "an independent exhausted window still blocks relief"
    );
    assert!(relieved(&[]).is_none());
    assert!(relieved(&[(key("session"), window(10.0, NOW, Status::Stale))]).is_none());
    assert_eq!(
        relieved(&[(key("session"), window(10.0, NOW, Status::Fresh))]).map(|row| row.judged),
        Some(10.0)
    );
}

#[test]
fn urgent_waits_waive_quota_grace_but_keep_four_seconds_of_client_input_protection() {
    assert_eq!(Waits::urgent().quota, 0);
    assert_eq!(Waits::urgent().input, 4);
    for trigger in [Trigger::Limit, Trigger::Headroom] {
        let found = episode(trigger);
        assert_eq!(
            decide(Some(&found), 600, &pane(), NOW),
            Decision::Wait {
                due_at: NOW - 1 + 600
            }
        );
        assert_eq!(
            decide_with_waits(Some(&found), Waits::urgent(), &pane(), NOW),
            Decision::Attempt
        );
        for (age, expected) in [
            (3, Decision::Hold(HoldReason::ClientInput)),
            (4, Decision::Attempt),
        ] {
            let mut seen = pane();
            seen.client_input = Some(NOW - age);
            assert_eq!(
                decide_with_waits(Some(&found), Waits::urgent(), &seen, NOW),
                expected,
                "trigger={trigger:?} input age={age}"
            );
        }
        let mut future_input = pane();
        future_input.client_input = Some(NOW + 1);
        assert_eq!(
            decide_with_waits(Some(&found), Waits::urgent(), &future_input, NOW),
            Decision::Hold(HoldReason::ClientInput)
        );
    }
}

#[test]
fn urgent_quota_never_weakens_existing_frame_guards_or_terminal_rounds() {
    for trigger in [Trigger::Limit, Trigger::Headroom] {
        let found = episode(trigger);
        for (frame, reason) in [
            (Frame::Unread, HoldReason::Unread),
            (Frame::Busy, HoldReason::Busy),
            (Frame::Draft, HoldReason::Draft),
        ] {
            let mut seen = pane();
            seen.frame = frame;
            assert_eq!(
                decide_with_waits(Some(&found), Waits::urgent(), &seen, NOW),
                Decision::Hold(reason)
            );
        }
        let mut asked = pane();
        asked.human_prompt = true;
        assert_eq!(
            decide_with_waits(Some(&found), Waits::urgent(), &asked, NOW),
            Decision::Hold(HoldReason::HumanPrompt)
        );
        let mut terminal = found;
        terminal.terminal = Some(Outcome::Done);
        assert_eq!(
            decide_with_waits(Some(&terminal), Waits::urgent(), &pane(), NOW),
            Decision::Rest
        );
    }
    let mut unknown = pane();
    unknown.frame = Frame::Unproven;
    assert_eq!(
        decide_with_waits(
            Some(&episode(Trigger::Headroom)),
            Waits::urgent(),
            &unknown,
            NOW
        ),
        Decision::Hold(HoldReason::Unproven)
    );
}

#[test]
fn settlement_metadata_refuses_absent_duplicate_and_unusable_stamps() {
    assert_eq!(
        crate::meta::Meta::parse("launch_settled=10000\n").launch_settled_epoch(),
        Some(10_000)
    );
    for text in [
        "",
        "launch_settled=\n",
        "launch_settled=oops\n",
        "launch_settled=0\n",
        "launch_settled=-1\n",
        "launch_settled=9223372036854775808\n",
        "launch_settled=10000\nlaunch_settled=10001\n",
    ] {
        assert_eq!(
            crate::meta::Meta::parse(text).launch_settled_epoch(),
            None,
            "{text:?}"
        );
    }
}
