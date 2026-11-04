//! Supplemental #304 B acceptance, frozen before the decision seam exists.
//! The main IT spec supplies behavioural RED; this file has compile RED on
//! base because `long_limit_stop` is new. Lead explicitly approved that gap.
use super::*;

const NOW: i64 = 1_800_000_000;
const TITLE: &str = "background evidence watch";
const WATCHING: &str = include_str!("../../tests/fixtures/harness-state/muse-limit-watching.txt");
const PLAIN: &str = include_str!("../../tests/fixtures/harness-state/muse-limit-plain.txt");

fn frame(reset: i64) -> String {
    format!(
        "◆ model failed: API error 429 [request_id=00000000-0000-0000-0000-000000000000]:\n  Subscription quota exhausted. Your usage window resets at\n  {}. (rate_limit_error)\n\n{WATCHING}",
        Timestamp::from_epoch(reset)
    )
}

#[test]
fn exact_twelve_hour_reset_and_ten_minute_grace_are_inclusive() {
    let capture = frame(NOW + 43_200);
    let proof = long_limit_stop(&[&capture, &capture, &capture], NOW - 600, NOW)
        .unwrap_or_else(|gap| panic!("exact boundary must prove B: {gap:?}"));
    assert_eq!(proof.reset_at, NOW + 43_200);
    assert_eq!(proof.tasks, [TITLE.to_owned()]);
}

#[test]
fn one_second_short_of_twelve_hours_refuses() {
    let capture = frame(NOW + 43_199);
    assert_eq!(
        long_limit_stop(&[&capture, &capture], NOW - 600, NOW).err(),
        Some(LongLimitGap::Short)
    );
}

#[test]
fn one_second_short_of_ten_minutes_refuses() {
    let capture = frame(NOW + 43_200);
    assert_eq!(
        long_limit_stop(&[&capture, &capture], NOW - 599, NOW).err(),
        Some(LongLimitGap::Grace)
    );
}

#[test]
fn a_future_episode_key_never_earns_grace() {
    let capture = frame(NOW + 43_200);
    assert_eq!(
        long_limit_stop(&[&capture, &capture], NOW + 1, NOW).err(),
        Some(LongLimitGap::Grace)
    );
}

#[test]
fn stop_boundary_rechecks_the_remaining_reset_horizon() {
    let capture = frame(NOW + 43_200);
    assert!(long_limit_stop(&[&capture, &capture], NOW - 1200, NOW).is_ok());
    assert_eq!(
        long_limit_stop(&[&capture, &capture, &capture], NOW - 1200, NOW + 1).err(),
        Some(LongLimitGap::Short)
    );
}

#[test]
fn third_capture_with_a_human_draft_refuses() {
    let capture = frame(NOW + 43_200);
    let third = capture.replace("\n❯\n", "\n❯ human draft\n");
    assert!(long_limit_stop(&[&capture, &capture], NOW - 600, NOW).is_ok());
    assert_eq!(
        long_limit_stop(&[&capture, &capture, &third], NOW - 600, NOW).err(),
        Some(LongLimitGap::NotBackgroundOnly)
    );
}

#[test]
fn third_capture_with_new_transcript_activity_refuses() {
    let capture = frame(NOW + 43_200);
    let third = capture.replace(
        "── Voice input",
        "◆ Working on a newer turn\n── Voice input",
    );
    assert_eq!(
        long_limit_stop(&[&capture, &capture, &third], NOW - 600, NOW).err(),
        Some(LongLimitGap::NotBackgroundOnly)
    );
}

#[test]
fn third_capture_with_a_different_reset_refuses() {
    let capture = frame(NOW + 43_200);
    let third = frame(NOW + 86_400);
    assert_eq!(
        long_limit_stop(&[&capture, &capture, &third], NOW - 600, NOW).err(),
        Some(LongLimitGap::ResetDiffers)
    );
}

#[test]
fn missing_reset_in_any_capture_refuses() {
    let capture = frame(NOW + 43_200);
    for captures in [
        vec![WATCHING, capture.as_str()],
        vec![capture.as_str(), WATCHING],
        vec![capture.as_str(), capture.as_str(), WATCHING],
    ] {
        assert_eq!(
            long_limit_stop(&captures, NOW - 600, NOW).err(),
            Some(LongLimitGap::NoReset)
        );
    }
}

#[test]
fn empty_or_mixed_plain_task_evidence_never_uses_b() {
    assert_eq!(
        long_limit_stop(&[], NOW - 600, NOW).err(),
        Some(LongLimitGap::NotBackgroundOnly)
    );
    let capture = frame(NOW + 43_200);
    for captures in [vec![capture.as_str(), PLAIN], vec![PLAIN, capture.as_str()]] {
        assert_eq!(
            long_limit_stop(&captures, NOW - 600, NOW).err(),
            Some(LongLimitGap::NotBackgroundOnly)
        );
    }
}

#[test]
fn any_changed_task_title_refuses_including_the_last_capture() {
    let capture = frame(NOW + 43_200);
    let changed = capture.replace(TITLE, "different human work");
    for captures in [
        vec![changed.as_str(), capture.as_str()],
        vec![capture.as_str(), changed.as_str()],
        vec![capture.as_str(), capture.as_str(), changed.as_str()],
    ] {
        assert_eq!(
            long_limit_stop(&captures, NOW - 600, NOW).err(),
            Some(LongLimitGap::TaskChanged)
        );
    }
}

#[test]
fn task_glyph_status_and_elapsed_are_not_task_identity() {
    let capture = frame(NOW + 43_200);
    let second = capture
        .replace("└ ◇", "└ ◆")
        .replace("watching", "running")
        .replace("22m 01s", "22m 02s");
    let third = capture.replace("└ ◇", "└ ◈").replace("22m 01s", "23m 01s");
    let proof = long_limit_stop(&[&capture, &second, &third], NOW - 600, NOW)
        .unwrap_or_else(|gap| panic!("same title through animated furniture: {gap:?}"));
    assert_eq!(proof.tasks, [TITLE.to_owned()]);
}

#[test]
fn task_count_and_order_must_stand_in_all_readings() {
    let first = frame(NOW + 43_200).replace(
        "  muse-wonder-full",
        "└ ◆ second task  running 1s\n  muse-wonder-full",
    );
    let one = frame(NOW + 43_200);
    let reordered = first
        .replace(TITLE, "SWAP")
        .replace("second task", TITLE)
        .replace("SWAP", "second task");
    let proof = long_limit_stop(&[&first, &first], NOW - 600, NOW)
        .unwrap_or_else(|gap| panic!("stable multiple tasks: {gap:?}"));
    assert_eq!(proof.tasks, [TITLE.to_owned(), "second task".to_owned()]);
    for captures in [
        vec![first.as_str(), one.as_str()],
        vec![first.as_str(), reordered.as_str()],
    ] {
        assert_eq!(
            long_limit_stop(&captures, NOW - 600, NOW).err(),
            Some(LongLimitGap::TaskChanged)
        );
    }
}
