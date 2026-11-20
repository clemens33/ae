//! The seeded orchestrator prompt teaches the writer the board contract.

use super::{SEAT_TEMPLATE, prompt_rows};

#[test]
fn the_seeded_prompt_teaches_the_board_command_sections_ids_and_links() {
    let prompt = prompt_rows(SEAT_TEMPLATE)
        .pop()
        .expect("the instructions that a seat launch uses")
        .value;
    let board = prompt
        .split_once("\n## Board\n")
        .expect("the board guidance in the winning instructions")
        .1;
    let (board, _) = board
        .split_once("\n## Boundaries\n")
        .expect("the board guidance precedes the boundaries");
    for rule in [
        "desk set",
        "DECIDE",
        "COMING",
        "RUNNING",
        "WAITING ON OTHERS",
        "LINKS",
        "desk set REFUSED",
        "desk set FAILED AFTER WRITE",
        "desk set UNCONFIRMED",
    ] {
        assert!(
            board.contains(rule),
            "missing from the seeded prompt: {rule}"
        );
    }
    assert!(
        ["D1", "C1", "R1", "W1", "L1"]
            .into_iter()
            .all(|id| board.contains(id)),
        "the prompt supplies a valid example of each section's ID"
    );
    let words = board.to_lowercase();
    assert!(words.contains("replace") && words.contains("stable"));
    assert!(words.contains("link") && words.contains("absolute path"));
}
