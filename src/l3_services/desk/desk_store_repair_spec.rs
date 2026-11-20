//! The regular-file corruption recovery path through the real session store.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "the acceptance test owns and inspects its scratch files"
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
        let name = format!(
            "ae-desk-repair-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        );
        let path = std::env::temp_dir().join(name);
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
fn set_replaces_the_damaged_record_and_audits_the_replacement() {
    let dir = Scratch::new();
    let path = dir.0.join("desk-record");
    fs::write(
        &path,
        "ae-desk 1\nupdated damaged\nD8\tDamaged predecessor\n",
    )
    .expect("damaged record");
    assert!(read(&dir.0).is_err());

    let replacement = parse_input(b"RUNNING\nR7 | A valid replacement\n").expect("valid input");
    let now = Timestamp::from_epoch(1_795_164_012);
    assert!(matches!(
        set(&dir.0, replacement.clone(), now, "boss"),
        Ok(SetOutcome::Changed { total: 1, .. })
    ));
    let actual = read(&dir.0)
        .expect("record is repaired")
        .expect("board exists");
    assert_eq!(actual.items, replacement);
    assert_eq!(actual.updated, now);
    let bytes = fs::read_to_string(&path).expect("new bytes");
    assert!(!bytes.contains("Damaged predecessor"));

    let journal = fs::read_to_string(dir.0.join("events.jsonl")).expect("journal");
    let mut lines = journal.lines();
    let event = Event::parse_line(lines.next().expect("event")).expect("parse event");
    assert_eq!(
        (event.action.as_str(), event.actor.as_str()),
        ("desk", "boss")
    );
    assert!(lines.next().is_none());
    assert!(!journal.contains("A valid replacement"));
}

#[test]
fn read_refuses_nonregular_records_and_distinguishes_the_cap_from_parse_failure() {
    use super::ReadError;
    use std::os::unix::fs::symlink;

    const STORED_CAP: usize = 4096 + 256;

    let dir = Scratch::new();
    let path = dir.0.join("desk-record");
    let ordinary = dir.0.join("regular");
    let valid = "ae-desk 1\nupdated 2026-11-20T08:40:12Z\nD1\tA valid record\n";
    fs::write(&ordinary, valid).expect("valid record for the symlink target");
    symlink(&ordinary, &path).expect("symlink at board path");
    assert!(matches!(read(&dir.0), Err(ReadError::NotRegular)));
    fs::remove_file(&path).expect("remove symlink");

    fs::create_dir(&path).expect("directory at board path");
    assert!(matches!(read(&dir.0), Err(ReadError::NotRegular)));
    fs::remove_dir(&path).expect("remove directory");

    let mut bytes = valid.as_bytes().to_vec();
    bytes.resize(STORED_CAP, b'x');
    fs::write(&path, &bytes).expect("file at the stored cap with invalid content");
    assert!(matches!(read(&dir.0), Err(ReadError::Parse(_))));
    bytes.push(b'x');
    fs::write(&path, &bytes).expect("file one byte above the stored cap");
    assert!(matches!(read(&dir.0), Err(ReadError::TooLarge)));

    fs::write(&path, valid).expect("restore a regular valid record");
    assert!(matches!(read(&dir.0), Ok(Some(_))));
}
