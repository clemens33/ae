//! S6b R3: only positive directory absence permits the app to say Ended.
//! Drive the reader that takes the real Ending request and reads the world;
//! a supplied `gone` boolean would mock away the behaviour being judged.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "frozen reader fixtures inspect only their owned scratch state"
)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use super::{Answer, EndingAsk, Reader, Request};

const UUID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "ae-s6b-reader-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn pending(&self) -> (Reader, PathBuf) {
        let dir = self.0.join("sessions/alpha");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("meta"), format!("session_id={UUID}\n")).unwrap();
        fs::write(dir.join("memory.md"), "S6B_PRESERVED\n").unwrap();
        let mut reader = Reader::new(self.0.clone(), None, None);
        reader.take(Request::Ending(Box::new(EndingAsk {
            name: "alpha".to_owned(),
            dir: dir.clone(),
            uuid: UUID.to_owned(),
            position: 0,
            asked: Instant::now(),
        })));
        (reader, dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_file_replacing_the_directory_does_not_mean_ended() {
    let root = Scratch::new();
    let (mut reader, dir) = root.pending();
    let saved = root.0.join("preserved");
    fs::rename(&dir, &saved).unwrap();
    fs::write(&dir, "replacement node\n").unwrap();
    assert!(fs::metadata(&dir).unwrap().is_file());
    assert!(
        reader.ended().is_empty(),
        "a replaced record supplied an outcome"
    );
    assert!(reader.ended().is_empty(), "a discarded watch told twice");
    assert_eq!(
        fs::read(saved.join("memory.md")).unwrap(),
        b"S6B_PRESERVED\n"
    );
    assert_eq!(fs::read(dir).unwrap(), b"replacement node\n");
}

#[test]
fn a_stat_error_is_not_positive_absence_or_an_ended_outcome() {
    let root = Scratch::new();
    let (mut reader, dir) = root.pending();
    let sessions = root.0.join("sessions");
    let saved = root.0.join("preserved-sessions");
    fs::rename(&sessions, &saved).unwrap();
    fs::write(&sessions, "blocked ancestor\n").unwrap();
    let error = fs::metadata(&dir).expect_err("the ancestor is a file");
    assert_eq!(error.kind(), std::io::ErrorKind::NotADirectory);
    assert!(
        reader.ended().is_empty(),
        "an unreadable record supplied an outcome"
    );
    assert_eq!(
        fs::read(saved.join("alpha/memory.md")).unwrap(),
        b"S6B_PRESERVED\n"
    );
}

#[test]
fn positively_missing_directory_reports_ended_once() {
    let root = Scratch::new();
    let (mut reader, dir) = root.pending();
    fs::rename(&dir, root.0.join("preserved")).unwrap();
    assert_eq!(
        fs::metadata(&dir).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    let answers = reader.ended();
    let [Answer::Ended { name, line }] = answers.as_slice() else {
        panic!("positive absence must supply exactly one Ended answer");
    };
    assert_eq!(name, "alpha");
    assert_eq!(line, "Ended alpha");
    assert!(reader.ended().is_empty(), "completed watch told twice");
}
