//! S5 refusal and publication interleaving requirements, exposed by mutation review.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned acceptance fixture IO"
)]

use super::disk::{self, Dir, Files, Glance, Op, Payload, Read, Refused, Reset, ThisProgress};
use super::doc;
use super::s5_fixture::{Fixture, S, birth, oracle};
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn mismatched_session(root: &Path) -> doc::Root {
    let published =
        doc::parse_root(&fs::read_to_string(root.join("stats/view")).expect("root view"))
            .expect("valid root fixture");
    let path = root.join(format!("stats/session/{S}.view"));
    let mut view = doc::parse_session(&fs::read_to_string(&path).expect("session view"))
        .expect("valid session fixture");
    let row = published
        .rows
        .get(&Some(S.to_owned()))
        .expect("registered row");
    assert_eq!(view.generation, row.generation, "calibrated matching store");
    view.generation = row
        .generation
        .checked_add(1)
        .expect("fixture generation fits");
    fs::write(&path, doc::render_session(&view)).expect("valid view of another generation");
    let changed = doc::parse_session(&fs::read_to_string(path).expect("changed view"))
        .expect("mismatch is generation, not parse damage");
    assert_ne!(changed.generation, row.generation);
    published
}

fn assert_mismatch(glance: Glance, expected: &crate::stats::Scoped, updating: bool) {
    let Glance::Ready {
        view,
        this,
        updating: actual,
        ..
    } = glance
    else {
        panic!("root remains readable despite its mismatched session view");
    };
    assert_eq!(
        &view.all, expected,
        "ALL remains the independent journal fold"
    );
    assert_eq!(
        actual, updating,
        "publication movement/intent must be honest"
    );
    assert!(matches!(view.this, crate::stats::ThisSession::NoUuid));
    let ThisProgress::Damaged(Refused::Damaged { file, reason }) = this else {
        panic!("a persistently mismatched session must be unavailable");
    };
    assert_eq!(file, format!("stats/session/{S}.view"));
    assert_eq!(reason, "generation differs from the root");
}

#[test]
fn s5_generation_mismatch_is_scope_damage_after_bounded_retries_and_reports_intent() {
    for intent in [false, true] {
        let root = Fixture::new();
        let dir = root.live("alpha", S, &birth(S, 1));
        root.build();
        let expected = oracle(&dir, "alpha", Some(S)).all;
        mismatched_session(&root.0);
        if intent {
            // Glance reports presence only; recovery is not being exercised here.
            fs::write(root.0.join("stats/intent"), b"publication in flight")
                .expect("intent presence");
        }
        let _ = disk::take_payload_names();
        assert_mismatch(
            disk::glance(&mut Files::at(&root.0), Some(S)),
            &expected,
            intent,
        );
        let reads = disk::take_payload_names();
        for name in ["stats/view".to_owned(), format!("stats/session/{S}.view")] {
            assert_eq!(
                reads.iter().filter(|(read, _)| *read == name).count(),
                3,
                "persistent mismatch gets the bounded three attempts for {name}"
            );
        }
    }
}

struct MovingRoot {
    files: Files,
    path: PathBuf,
    published: doc::Root,
    reads: u64,
}
impl Dir for MovingRoot {
    fn read(&mut self, name: &str, cap: u64) -> Read {
        if name == "stats/view" {
            // Real persisted documents change at a deterministic read boundary.
            // Counts and the session row stay fixed; only the root publication moves.
            let mut next = self.published.clone();
            next.generation = next
                .generation
                .checked_add(self.reads)
                .expect("fixture generation fits");
            fs::write(&self.path, doc::render_root(&next)).expect("publish next root");
            self.reads += 1;
        }
        self.files.read(name, cap)
    }
    fn apply(&mut self, op: &Op) -> io::Result<()> {
        panic!("glance must write nothing: {op:?}");
    }
    fn list(&mut self, name: &str) -> io::Result<Vec<String>> {
        panic!("published glance must list nothing: {name}");
    }
}

#[test]
fn s5_generation_mismatch_marks_a_moving_root_updating_without_an_intent() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, &birth(S, 1));
    root.build();
    let expected = oracle(&dir, "alpha", Some(S)).all;
    let published = mismatched_session(&root.0);
    let mut moving = MovingRoot {
        files: Files::at(&root.0),
        path: root.0.join("stats/view"),
        published,
        reads: 0,
    };
    assert_mismatch(disk::glance(&mut moving, Some(S)), &expected, true);
    assert_eq!(moving.reads, 3, "moving publication retries remain bounded");
}

#[derive(Debug, PartialEq, Eq)]
struct Node {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
    dev: u64,
    ino: u64,
}
fn snapshot(path: &Path) -> Vec<Node> {
    let meta = fs::metadata(path).expect("fixture node metadata");
    let mut nodes = vec![Node {
        path: path.to_path_buf(),
        bytes: meta
            .is_file()
            .then(|| fs::read(path).expect("fixture node bytes")),
        dev: meta.dev(),
        ino: meta.ino(),
    }];
    if meta.is_dir() {
        let mut children: Vec<_> = fs::read_dir(path)
            .expect("fixture directory")
            .map(|entry| entry.expect("fixture child").path())
            .collect();
        children.sort();
        for child in children {
            nodes.extend(snapshot(&child));
        }
    }
    nodes
}

#[test]
fn s5_reset_takes_over_recovery_damage_and_preserves_authoritative_journal() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, &birth(S, 1));
    root.build();
    let expected = oracle(&dir, "alpha", Some(S));
    let journal = fs::read(dir.join("events.jsonl")).expect("authoritative journal");
    let lock = fs::metadata(root.0.join("stats.lock")).expect("permanent lock");
    let Glance::Ready { epoch, .. } = disk::glance(&mut Files::at(&root.0), Some(S)) else {
        panic!("calibrated published store");
    };
    fs::write(root.0.join("stats/epoch"), b"broken epoch\n").expect("damage recovery input");
    assert!(
        matches!(
            disk::recovery(&mut Files::at(&root.0)),
            Err(Refused::Damaged { .. })
        ),
        "recovery itself cannot get past this damage"
    );
    assert!(
        matches!(disk::reset(&root.0, Duration::ZERO), Ok(Reset::Rebuilt { epoch: next }) if next > epoch)
    );
    let after = fs::metadata(root.0.join("stats.lock")).expect("same permanent lock");
    assert_eq!((after.dev(), after.ino()), (lock.dev(), lock.ino()));
    assert_eq!(
        fs::read(dir.join("events.jsonl")).expect("journal kept"),
        journal
    );
    assert!(matches!(
        disk::glance(&mut Files::at(&root.0), Some(S)),
        Glance::Building
    ));
    root.build();
    let Glance::Ready { view, .. } = disk::glance(&mut Files::at(&root.0), Some(S)) else {
        panic!("real upkeep rebuilds after takeover");
    };
    assert_eq!(view, expected);
}

#[test]
fn s5_reset_non_wouldblock_lock_path_error_is_named_io_and_changes_nothing() {
    let root = Fixture::new();
    root.live("alpha", S, &birth(S, 1));
    root.build();
    fs::remove_file(root.0.join("stats.lock")).expect("fixture replaces its own lock");
    fs::create_dir(root.0.join("stats.lock")).expect("lock pathname cannot be opened as a file");
    let why = Payload::at(&root.0)
        .lock(Duration::ZERO)
        .expect_err("calibrated lock-path error");
    assert_ne!(why.kind(), io::ErrorKind::WouldBlock);
    let before = snapshot(&root.0);
    let Err(Refused::Io(why)) = disk::reset(&root.0, Duration::ZERO) else {
        panic!("a lock-path IO error must not be called Busy");
    };
    assert!(why.contains("stats.lock"), "name the failing lock: {why}");
    assert_eq!(
        snapshot(&root.0),
        before,
        "IO refusal changes no bytes or inodes"
    );
}

#[test]
fn s5_reset_transient_held_read_io_refuses_without_writes_then_retry_succeeds() {
    let root = Fixture::new();
    root.live("alpha", S, &birth(S, 1));
    root.build();
    let before = snapshot(&root.0);
    let _ = disk::take_file_events();
    let _ = disk::take_payload_names();
    disk::fail_next_exception_read("stats/epoch");
    let Err(Refused::Io(why)) = disk::reset(&root.0, Duration::ZERO) else {
        panic!("even a one-shot external IO error must refuse, not authorize repair");
    };
    assert!(
        why.contains("stats/epoch") && why.contains("injected payload read IO"),
        "{why}"
    );
    assert!(
        disk::take_payload_names()
            .iter()
            .any(|(name, bytes)| name == "stats/epoch" && *bytes == 0),
        "fault must reach the actual held exception read after the warm"
    );
    assert!(
        disk::take_file_events().is_empty(),
        "IO refusal performs zero writes"
    );
    assert_eq!(snapshot(&root.0), before, "all bytes and inodes are kept");
    assert!(
        matches!(
            disk::reset(&root.0, Duration::ZERO),
            Ok(Reset::Rebuilt { .. })
        ),
        "fault is one-shot: an explicit later retry can reset normally"
    );
}
