//! Diagnostic addendum: existing non-limit refusals and the phase-0 missing-row rule.
//! Oracle: lead A1 preserves non-limit agy routes; plan section 5 assigns a
//! missing Error ID row `NoBox`. Constructed limit frames are not live captures.
use super::{HarnessState, ToolKind, frame_gap, refusal_why};
use crate::watchdog::{LimitStanding, limit_standing};

const ROW: &str = "⚠ Individual quota reached. Please upgrade your subscription to increase your limits. Resets in 162h20m6s.";
const EMPTY: &str = include_str!("../../tests/fixtures/agy-composer/agy-composed-frame-80x24.txt");
const DRAFT: &str = include_str!("../../tests/fixtures/agy-composer/agy-draft-frame-80x24.txt");
const TRUST: &str = include_str!("../../tests/fixtures/agy-composer/agy-trust-modal-frame.txt");
const MUSE: &str = include_str!("../../tests/fixtures/runtime-identity/muse-idle-plain-80x24.txt");

#[test]
fn non_limit_agy_refusals_keep_no_frame_for_composer_draft_and_modal() {
    for capture in [EMPTY, DRAFT, TRUST] {
        let reading = (HarnessState::Unknown, false, Some(capture));
        assert_eq!(
            frame_gap(reading, "agy", ToolKind::Agy).as_deref(),
            Some("no frame"),
            "non-limit agy reading: {capture}"
        );
        assert_eq!(
            refusal_why(reading, reading, "agy", ToolKind::Agy),
            "no frame",
            "the stop refusal must retain the same reason"
        );
    }
}

#[test]
fn muse_non_limit_refusals_keep_no_limit_row() {
    let reading = (HarnessState::Unknown, false, Some(MUSE));
    assert_eq!(limit_standing(MUSE, "muse"), LimitStanding::NoCell);
    assert_eq!(
        frame_gap(reading, "muse", ToolKind::Muse).as_deref(),
        Some("no limit row")
    );
    assert_eq!(
        refusal_why(reading, reading, "muse", ToolKind::Muse),
        "no limit row"
    );
}

#[test]
fn blank_instead_of_error_id_refuses_as_no_box_after_the_found_limit() {
    let fence = EMPTY
        .lines()
        .find(|row| row.starts_with("────"))
        .expect("measured upper rule");
    let capture = EMPTY.replacen(fence, &format!("{ROW}\n\n{fence}"), 1);
    assert_eq!(limit_standing(&capture, "agy"), LimitStanding::NoBox);
    assert_eq!(
        frame_gap(
            (HarnessState::Unknown, false, Some(&capture)),
            "agy",
            ToolKind::Agy,
        )
        .as_deref(),
        Some("no empty box under the limit row")
    );
}

#[test]
fn absent_error_id_after_limit_as_last_ink_refuses_as_no_box() {
    for capture in [ROW.to_owned(), format!("{ROW}\n\n  \n")] {
        assert_eq!(limit_standing(&capture, "agy"), LimitStanding::NoBox);
        assert_eq!(
            frame_gap(
                (HarnessState::Unknown, false, Some(&capture)),
                "agy",
                ToolKind::Agy,
            )
            .as_deref(),
            Some("no empty box under the limit row")
        );
    }
}

#[test]
fn no_agy_limit_head_still_means_no_cell() {
    for capture in [EMPTY, DRAFT, TRUST, "", "\n \n"] {
        assert_eq!(limit_standing(capture, "agy"), LimitStanding::NoCell);
    }
}
