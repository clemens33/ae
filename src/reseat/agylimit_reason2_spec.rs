//! Refusal-reason supplement, oracle: phase-0 plan section 5 and reseat contract.
//! Synthetic limit cells over measured composers; no live limit-pane claim.
use super::{HarnessState, ToolKind, frame_gap};
use crate::watchdog::{LimitStanding, limit_standing};

const ROW: &str = "⚠ Individual quota reached. Please upgrade your subscription to increase your limits. Resets in 162h20m6s.";
const ERROR: &str = "Error ID: 00000000-0000-0000-0000-000000000000-000";
const EMPTY: &str = include_str!("../../tests/fixtures/agy-composer/agy-composed-frame-80x24.txt");
const DRAFT: &str = include_str!("../../tests/fixtures/agy-composer/agy-draft-frame-80x24.txt");
const WRAPPED: &str =
    include_str!("../../tests/fixtures/agy-composer/agy-wrapped-draft-frame-80x24.txt");

fn frame(composer: &str, block: &str) -> String {
    let fence = composer
        .lines()
        .find(|row| row.starts_with("────"))
        .expect("measured upper rule");
    composer.replacen(fence, &format!("{block}\n\n{fence}"), 1)
}

fn reason(capture: &str) -> Option<String> {
    frame_gap(
        (HarnessState::Unknown, false, Some(capture)),
        "agy",
        ToolKind::Agy,
    )
}

#[test]
fn unexpected_row_in_place_of_error_id_is_quoted_as_row_after_limit() {
    let unexpected = "unexpected status after limit";
    let capture = frame(EMPTY, &format!("{ROW}\n{unexpected}"));
    assert_eq!(
        limit_standing(&capture, "agy"),
        LimitStanding::RowAfterCell(unexpected.to_owned())
    );
    assert_eq!(
        reason(&capture).as_deref(),
        Some("row after the limit row: \"unexpected status after limit\"")
    );
}

#[test]
fn work_abutting_error_id_is_quoted_as_row_after_limit() {
    let work = "work started after the limit";
    let capture = frame(EMPTY, &format!("{ROW}\n{ERROR}\n{work}"));
    assert_eq!(
        limit_standing(&capture, "agy"),
        LimitStanding::RowAfterCell(work.to_owned())
    );
    assert_eq!(
        reason(&capture).as_deref(),
        Some("row after the limit row: \"work started after the limit\"")
    );
}

#[test]
fn draft_wrapped_draft_and_broken_composers_name_no_empty_box() {
    let block = format!("{ROW}\n{ERROR}");
    let valid = frame(EMPTY, &block);
    // The independently measured empty composer is a positive construction
    // control, so the following cases cannot pass through a missing limit head.
    assert_eq!(limit_standing(&valid, "agy"), LimitStanding::Stands);
    for capture in [
        frame(DRAFT, &block),
        frame(WRAPPED, &block),
        valid.replace("? for shortcuts", "unknown footer"),
        valid.replace("\n>\n", "\n>\nunknown closing rule\n"),
    ] {
        assert_eq!(
            limit_standing(&capture, "agy"),
            LimitStanding::NoBox,
            "unproven empty composer: {capture}"
        );
        assert_eq!(
            reason(&capture).as_deref(),
            Some("no empty box under the limit row")
        );
    }
}
