//! Frozen acceptance cases: appread L2, loader side — the one-shot read
//! job. Independent navigator appread-nav (Meta Muse Wonder Full 1.0
//! contributor), frozen against driver appread's plan-r3. Driver installs
//! byte-identically, wired from src/app/loader.rs via
//! `#[cfg(test)] mod read_spec;`, beside `peek_spec`. The link side is
//! `src/console/appread_link_spec.rs`, the app side
//! `src/app/appread_spec.rs`.
//!
//! The read is ONE SHOT, unlike the peek cadence: a taken ask is due at
//! once, `Reader::read` answers it under its generation, and the ask is
//! gone — a later `next` never re-reads. `Request::Read(None)` drops an
//! unread ask; a second ask replaces an unread first. The answer body is
//! the file decoded, or ONE uniformly-prefixed refusal: every Gap the
//! loader builds reads `refused: read {abs path}: {why}`, whether the
//! door or the decoder refused. Job priority against peek/settings is
//! deliberately not pinned (the panels are unreachable together).
//! The `reader` read-gate key is exercised nowhere here: with the
//! environment unset the gate is a no-op, which is what these cases need.
//! `ReadAsk` is `Clone`, like `PeekAsk`: `next` hands the job a clone.

use std::path::PathBuf;
use std::time::Instant;

use super::{Answer, Job, ReadAsk, Reader, Request};
use crate::app::filedoor::{self, Roots};
use crate::app::reader::{self, Body};
use crate::console::link::Base;

/// A fresh owned directory under the process temp dir.
fn root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae-appread-load-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the fixture root is made");
    dir
}

/// A reader past its first fleet read, so `next` reaches the read job.
fn reader() -> Reader {
    let mut reader = Reader::new(PathBuf::from("/nonexistent"), None, None);
    reader.fleet_at = Some(Instant::now());
    reader
}

#[allow(
    clippy::unnecessary_box_returns,
    reason = "the request carries the ask boxed, as the loader's Request does"
)]
fn ask(generation: u64, path: String, dir: &str) -> Box<ReadAsk> {
    Box::new(ReadAsk {
        generation,
        path,
        line: Some(2),
        links: 1,
        base: Base {
            dir: Some(dir.to_owned()),
            home: None,
        },
    })
}

#[test]
fn a_taken_ask_is_due_at_once_answered_once_and_then_gone() {
    let dir = root("once");
    let file = dir.join("a.txt");
    std::fs::write(&file, b"a\nb\n").expect("the fixture file is written");
    let path = file.to_string_lossy().into_owned();
    let dir_word = dir.to_string_lossy().into_owned();
    let mut reader = reader();
    let asked = ask(7, path, &dir_word);
    reader.take(Request::Read(Some(asked.clone())));
    assert!(
        matches!(reader.next(Instant::now()), Job::Read(_)),
        "due at once",
    );
    let Answer::Read { generation, body } = reader.read(&asked) else {
        panic!("a read answers a read");
    };
    assert_eq!(generation, 7);
    assert_eq!(
        body,
        Body::Text {
            lines: vec!["a".to_owned(), "b".to_owned()],
            size: 4,
        },
    );
    assert!(
        !matches!(reader.next(Instant::now()), Job::Read(_)),
        "one shot: a later next never re-reads",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn every_gap_wears_the_one_refusal_prefix_door_or_decoder_alike() {
    let dir = root("gap");
    let dir_word = dir.to_string_lossy().into_owned();
    let missing = dir.join("gone.txt").to_string_lossy().into_owned();
    let mut reader = reader();
    let asked = ask(3, missing.clone(), &dir_word);
    reader.take(Request::Read(Some(asked.clone())));
    let Answer::Read { generation, body } = reader.read(&asked) else {
        panic!("a read answers a read");
    };
    assert_eq!(generation, 3);
    assert_eq!(
        body,
        Body::Gap(format!("refused: read {missing}: not found")),
    );
    let binary = dir.join("bin.dat");
    std::fs::write(&binary, b"a\0b").expect("the fixture file is written");
    let binary = binary.to_string_lossy().into_owned();
    let asked = ask(4, binary.clone(), &dir_word);
    reader.take(Request::Read(Some(asked.clone())));
    let Answer::Read { body, .. } = reader.read(&asked) else {
        panic!("a read answers a read");
    };
    assert_eq!(
        body,
        Body::Gap(format!("refused: read {binary}: binary file (NUL byte)")),
    );
    // Outside every root: the same prefix, the door's own words.
    let other = root("gap-other");
    let far = other.join("a.txt");
    std::fs::write(&far, b"x\n").expect("the fixture file is written");
    let far = far.to_string_lossy().into_owned();
    let asked = ask(5, far.clone(), &dir_word);
    reader.take(Request::Read(Some(asked.clone())));
    let Answer::Read { body, .. } = reader.read(&asked) else {
        panic!("a read answers a read");
    };
    assert_eq!(
        body,
        Body::Gap(format!(
            "refused: read {far}: outside the session dir and home"
        )),
    );
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&other);
}

#[test]
fn a_second_ask_replaces_an_unread_first_and_none_drops_it() {
    let mut reader = reader();
    let dir = root("replace");
    let file = dir.join("a.txt");
    std::fs::write(&file, b"x\n").expect("the fixture file is written");
    let path = file.to_string_lossy().into_owned();
    let dir_word = dir.to_string_lossy().into_owned();
    reader.take(Request::Read(Some(ask(1, path.clone(), &dir_word))));
    let second = ask(2, path, &dir_word);
    reader.take(Request::Read(Some(second.clone())));
    assert!(matches!(reader.next(Instant::now()), Job::Read(_)));
    let Answer::Read { generation, .. } = reader.read(&second) else {
        panic!("a read answers a read");
    };
    assert_eq!(generation, 2, "the newest ask is the one read");
    reader.take(Request::Read(Some(ask(
        3,
        "/nonexistent/a.txt".to_owned(),
        &dir_word,
    ))));
    reader.take(Request::Read(None));
    assert!(
        !matches!(reader.next(Instant::now()), Job::Read(_)),
        "a close drops the unread ask",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The read half of the write-nothing invariant (lead IMPORTANT-1): the
/// files standing in the directory read from — a journal, a draft and the
/// read target itself — are byte-identical after the read, read back
/// through the door whose own byte fidelity the door cases pin against
/// what this spec wrote. The App half is beside `reading_app`.
#[test]
fn a_read_leaves_the_journal_the_draft_and_the_target_byte_identical() {
    let dir = root("untouched");
    let written: [(&str, &[u8]); 3] = [
        ("events.jsonl", b"{\"kind\":\"said\",\"body\":\"hi\"}\n"),
        ("console.draft", b"half a thought, unsaved"),
        ("target.txt", b"alpha\nbeta\n"),
    ];
    for (name, bytes) in written {
        std::fs::write(dir.join(name), bytes).expect("the fixture file is written");
    }
    let dir_word = dir.to_string_lossy().into_owned();
    let target = dir.join("target.txt").to_string_lossy().into_owned();
    let mut reader = reader();
    let asked = ask(6, target, &dir_word);
    reader.take(Request::Read(Some(asked.clone())));
    let Answer::Read { body, .. } = reader.read(&asked) else {
        panic!("a read answers a read");
    };
    assert_eq!(
        body,
        Body::Text {
            lines: vec!["alpha".to_owned(), "beta".to_owned()],
            size: 11,
        },
    );
    let roots = Roots {
        dir: Some(dir.clone()),
        home: None,
    };
    for (name, bytes) in written {
        let filedoor::Read::Bytes { bytes: back, size } =
            filedoor::read(&dir.join(name), &roots, reader::MAX_BYTES)
        else {
            panic!("{name} reads back");
        };
        assert_eq!(back.as_slice(), bytes, "{name} is byte-identical");
        assert_eq!(
            usize::try_from(size).expect("a test file fits usize"),
            bytes.len(),
            "{name} kept its size"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
