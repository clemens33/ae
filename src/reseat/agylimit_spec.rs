//! Independent phase-2 acceptance spec; constructed frames, not live limit captures.
use super::{HarnessState, StopPolicy, ToolKind, limit_proven, may_stop};

const ROW: &str = "⚠ Individual quota reached. Please upgrade your subscription to increase your limits. Resets in 162h20m6s.";
const ERROR: &str = "Error ID: 00000000-0000-0000-0000-000000000000-000";
const EMPTY: &str = include_str!("../../tests/fixtures/agy-composer/agy-composed-frame-80x24.txt");
const WIDE: &str = include_str!("../../tests/fixtures/agy-composer/agy-composed-frame.txt");
const DRAFT: &str = include_str!("../../tests/fixtures/agy-composer/agy-draft-frame-80x24.txt");
const WRAPPED: &str =
    include_str!("../../tests/fixtures/agy-composer/agy-wrapped-draft-frame-80x24.txt");
const TRUST: &str = include_str!("../../tests/fixtures/agy-composer/agy-trust-modal-frame.txt");

/// Keep the measured composer and put the reported limit block immediately
/// above its upper rule. This changes no frozen fixture bytes on disk.
fn frame(composer: &str, block: &str) -> String {
    let fence = composer
        .lines()
        .find(|row| row.starts_with("────"))
        .expect("a measured upper rule");
    composer.replacen(fence, &format!("{block}\n\n{fence}"), 1)
}

fn limited(composer: &str) -> String {
    frame(composer, &format!("{ROW}\n{ERROR}"))
}

fn proves(capture: &str) -> bool {
    limit_proven(capture, "agy", ToolKind::Agy)
}

#[test]
fn agylimit_stop_proof_requires_limit_and_empty_measured_composer() {
    for composer in [EMPTY, WIDE] {
        assert!(
            proves(&limited(composer)),
            "limit + measured empty composer"
        );
        assert!(!proves(composer), "composer alone is no limit stop");
        assert_eq!(
            crate::harness_state::classify(composer, ToolKind::Agy),
            HarnessState::Unknown
        );
        assert!(crate::harness_state::read_frame(composer, ToolKind::Agy).is_none());
    }
    for capture in [
        ROW.to_owned(),
        format!("{ROW}\n{ERROR}"),
        limited(DRAFT),
        limited(WRAPPED),
        format!("{ROW}\n\n{TRUST}"),
    ] {
        assert!(!proves(&capture), "unproven composer: {capture}");
    }
    assert!(
        !ToolKind::Agy.background_tasks_read(),
        "#304 remains Muse-only"
    );
}

#[test]
fn agylimit_work_after_the_limit_and_broken_composers_refuse_stop() {
    let capture = limited(EMPTY);
    for changed in [
        capture.replace("\n>\n", "\n> human draft\n"),
        capture.replace("\n>\n", "\n>\n  continued draft\n"),
        capture.replace("\n>\n", "\n>\nunknown closing rule\n"),
        capture.replace("? for shortcuts", "unknown footer"),
        capture.replace(
            &format!("{ERROR}\n"),
            &format!("{ERROR}\nwork started after the limit\n"),
        ),
        capture.replace(ROW, &format!("  {ROW}")),
        capture.replace(ROW, "⚠ a different notice"),
        capture.replace(&format!("{ERROR}\n"), ""),
        capture.replace(ERROR, " Error ID: indented"),
        capture.replace("limits. Resets", "limits.\nResets"),
    ] {
        assert!(!proves(&changed), "must refuse changed frame: {changed}");
    }
}

#[test]
fn agylimit_stop_needs_both_readings_and_plain_reseat_stays_unknown() {
    let stopped = limited(EMPTY);
    for (first, second, expected) in [
        (stopped.as_str(), stopped.as_str(), true),
        (stopped.as_str(), EMPTY, false),
        (EMPTY, stopped.as_str(), false),
        (stopped.as_str(), DRAFT, false),
        (DRAFT, stopped.as_str(), false),
        (EMPTY, EMPTY, false),
    ] {
        let state = |capture: &str| {
            (
                crate::harness_state::classify(capture, ToolKind::Agy),
                proves(capture),
            )
        };
        assert_eq!(
            may_stop(state(first), state(second), StopPolicy::Proven),
            expected
        );
    }
}
