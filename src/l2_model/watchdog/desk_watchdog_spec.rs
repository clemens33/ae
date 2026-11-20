//! Publishing the board preserves the dispatcher seat's done declaration.

use super::{
    QuietKind, declaration_current, done_progress_proof, ends_quiet, latest_relevant_event,
    quiet_reason,
};
use crate::events::Event;
use crate::time::Timestamp;

#[test]
fn a_board_audit_is_own_information_and_keeps_the_done_declaration_current() {
    let declaration = Event::parse_line(
        r#"{"ts":"2026-11-20T08:40:12Z","actor":"boss","action":"state","ref":"done"}"#,
    )
    .expect("dispatcher declaration");
    let board = Event::parse_line(
        r#"{"ts":"2026-11-20T08:41:12Z","actor":"boss","action":"desk","summary":"+D7 (1 items)"}"#,
    )
    .expect("publication audit");
    let reported = board.ts;
    let declared = declaration.ts;
    assert!(!ends_quiet("done", &board, true, &[]));
    assert!(ends_quiet("done", &board, false, &[]));
    let events = [declaration, board];
    let current = latest_relevant_event(&events, "orchestrator", "main", "boss")
        .expect("shared current declaration");
    assert!(declaration_current(&current));
    assert_eq!(quiet_reason(&current), Some(QuietKind::Done));

    let (_, proof) = done_progress_proof(
        &events,
        "orchestrator",
        "main",
        "boss",
        Some("launch-1"),
        Timestamp::from_epoch(reported.epoch() + 60),
        300,
        2,
    );
    assert!(proof.unchanged);
    assert_eq!(proof.proof, Some(declared));
    assert_eq!(proof.settle, Some(reported));
}
