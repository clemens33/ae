//! Control characters are refused and an explicit empty first board is saved.

#![allow(
    clippy::disallowed_methods,
    reason = "the acceptance test reads a record and audit in its owned fixture"
)]

use std::fs;
use std::path::PathBuf;

use super::{SetOutcome, read, set};
use crate::desk::parse_input;
use crate::events::Event;
use crate::time::Timestamp;

#[test]
fn text_trimming_does_not_discard_controls() {
    let spaces = b"DECIDE\nD1 |   ordinary spaces   \n";
    let items = parse_input(spaces).expect("ordinary surrounding spaces are permitted");
    assert_eq!(items[0].text, "ordinary spaces");

    for input in [
        b"DECIDE\nD1 | \tleading tab\n".as_slice(),
        b"DECIDE\nD1 | trailing tab\t\n",
        b"DECIDE\nD1 | trailing carriage return\r\n",
    ] {
        let why = parse_input(input).expect_err("a control character invalidates the update");
        assert_eq!(why.line, 2, "bad input: {input:?}");
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let name = format!(
            "ae-desk-empty-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        );
        let dir = std::env::temp_dir().join(name);
        fs::create_dir(&dir).expect("private desk directory");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn an_explicit_empty_first_board_is_published_and_the_next_set_is_unchanged() {
    let dir = Scratch::new();
    let record = dir.0.join("desk-record");
    let audit = dir.0.join("events.jsonl");
    let now = Timestamp::parse("2026-11-20T08:40:12Z").expect("fixture time");
    let later = Timestamp::from_epoch(now.epoch() + 20);
    let expected = b"ae-desk 1\nupdated 2026-11-20T08:40:12Z\n";
    let items = parse_input(b"DECIDE\nCOMING\nRUNNING\nWAITING ON OTHERS\nLINKS\n")
        .expect("an explicitly empty board");
    assert!(items.is_empty());
    assert!(matches!(read(&dir.0), Ok(None)));
    assert!(matches!(
        set(&dir.0, items.clone(), now, "boss"),
        Ok(SetOutcome::Changed { total: 0, .. })
    ));
    assert_eq!(
        fs::read(&record)
            .expect("published empty record")
            .as_slice(),
        expected
    );
    assert!(matches!(read(&dir.0), Ok(Some(board)) if board.items.is_empty()));

    let journal = fs::read_to_string(&audit).expect("audit of the first set");
    let mut events = journal.lines();
    let entry = Event::parse_line(events.next().expect("one audit event")).expect("audit JSON");
    assert_eq!(entry.actor, "boss");
    assert_eq!(entry.action, "desk");
    assert!(entry.reference.is_none());
    assert_eq!(entry.summary.as_deref(), Some("(0 items)"));
    assert!(events.next().is_none());

    assert!(matches!(
        set(&dir.0, items, later, "boss"),
        Ok(SetOutcome::Unchanged)
    ));
    assert_eq!(
        fs::read(&record)
            .expect("unchanged empty record")
            .as_slice(),
        expected
    );
    assert_eq!(fs::read_to_string(audit).expect("unchanged audit"), journal);
}
