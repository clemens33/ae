//! Publication and audit acceptance for the real session store.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "this acceptance test owns and inspects its scratch files"
)]

use std::fs;
use std::path::PathBuf;

use super::{SetOutcome, read, set};
use crate::desk::parse_input;
use crate::events::Event;
use crate::time::Timestamp;

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let id = format!(
            "ae-desk-store-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        );
        let path = std::env::temp_dir().join(id);
        fs::create_dir(&path).expect("private state directory");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn change_replaces_the_record_once_noop_preserves_bytes_and_the_audit_only_has_ids() {
    let dir = Scratch::new();
    let path = dir.0.join("desk-record");
    let ledger = dir.0.join("events.jsonl");
    assert!(matches!(read(&dir.0), Ok(None)));
    let first = parse_input(b"DECIDE\nD17 | publish the eastern plan\nD7 | first draft\n")
        .expect("first input");
    let now = Timestamp::from_epoch(1_795_164_012);
    assert!(matches!(
        set(&dir.0, first.clone(), now, "boss"),
        Ok(SetOutcome::Changed { total: 2, .. })
    ));
    let initial = read(&dir.0).ok().flatten().expect("published board");
    assert_eq!(initial.updated, now);
    assert_eq!(initial.items, first);
    let snapshot = fs::read(&path).expect("published bytes");
    let journal = fs::read_to_string(&ledger).expect("audit event");

    let later = Timestamp::from_epoch(now.epoch() + 20);
    assert!(matches!(
        set(&dir.0, first, later, "boss"),
        Ok(SetOutcome::Unchanged)
    ));
    assert_eq!(fs::read(&path).expect("unchanged bytes"), snapshot);
    assert_eq!(
        fs::read_to_string(&ledger).expect("unchanged journal"),
        journal
    );

    let next = parse_input(b"DECIDE\nD7 | first draft | https://example.test/decision\nRUNNING\nR2 | validate release\n")
        .expect("replacement input");
    assert!(matches!(
        set(&dir.0, next.clone(), now, "boss"),
        Ok(SetOutcome::Changed { total: 2, .. })
    ));
    let current = read(&dir.0).ok().flatten().expect("current board");
    assert_eq!(current.items, next);
    assert!(current.updated.epoch() > initial.updated.epoch());
    let published = fs::read_to_string(&path).expect("current record");
    assert_eq!(published.matches("ae-desk 1\n").count(), 1);
    assert!(!published.contains("D17"));
    let journal = fs::read_to_string(&ledger).expect("final audit events");
    assert_eq!(journal.lines().count(), 2);
    let events: Vec<_> = journal
        .lines()
        .map(|line| Event::parse_line(line).expect("audit event"))
        .collect();
    assert!(
        events
            .iter()
            .all(|event| event.action == "desk" && event.actor == "boss")
    );
    let change = events[1].summary.as_deref().expect("change summary");
    for part in ["+R2", "-D17", "~D7"] {
        assert!(change.contains(part), "{change}");
    }
    for text in [
        "eastern plan",
        "first draft",
        "example.test",
        "validate release",
    ] {
        assert!(!journal.contains(text), "audit exposes {text:?}");
    }
}

#[test]
fn a_damaged_board_is_distinct_from_no_board() {
    let dir = Scratch::new();
    let path = dir.0.join("desk-record");
    fs::write(&path, "ae-desk 1\nupdated unknown\n").expect("damaged record");
    assert!(read(&dir.0).is_err());
    fs::remove_file(&path).expect("replace damaged record with a nonregular node");
    fs::create_dir(&path).expect("not a file");
    assert!(read(&dir.0).is_err());
    fs::remove_dir(&path).expect("operator removal");
    assert!(matches!(read(&dir.0), Ok(None)));
}

#[test]
fn replacing_three_boards_in_one_clock_second_keeps_the_latest_time() {
    let dir = Scratch::new();
    let now = Timestamp::parse("2026-11-20T08:40:59Z").expect("fixed clock");
    let mut previous = now.epoch();
    for (index, text) in ["First decision", "Second decision", "Third decision"]
        .iter()
        .enumerate()
    {
        let input = format!("DECIDE\nD1 | {text}\n");
        let items = parse_input(input.as_bytes()).expect("valid replacement input");
        assert!(matches!(
            set(&dir.0, items, now, "boss"),
            Ok(SetOutcome::Changed { total: 1, .. })
        ));
        let persisted = read(&dir.0)
            .expect("canonical record")
            .expect("replacement");
        let updated = persisted.updated.epoch();
        assert!(
            updated >= previous,
            "revision {} moves the clock backwards",
            index + 1
        );
        assert!(
            updated <= now.epoch() + 1,
            "timestamp must stay near the core clock"
        );
        assert_eq!(persisted.items[0].text, *text);
        previous = updated;
    }
}
