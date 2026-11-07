//! S5 view/read/reset acceptance; requirements are the oracle, not glance internals.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned acceptance fixture IO"
)]
use super::disk::{self, Dir, Files, Glance, Op, Payload, Read, Reset, Shown, ThisProgress};
use super::s5_fixture::{Fixture, S, T, birth, oracle};
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt as _;
use std::time::{Duration, Instant};

fn ready(glance: Glance) -> crate::stats::View {
    match glance {
        Glance::Ready { view, .. } => view,
        other => panic!("expected published view, got {other:?}"),
    }
}
struct ReadOnly {
    files: Files,
    allowed: Vec<String>,
    reads: Vec<(String, u64)>,
}
impl Dir for ReadOnly {
    fn read(&mut self, name: &str, cap: u64) -> Read {
        assert!(
            self.allowed.iter().any(|allowed| allowed == name),
            "forbidden content read {name}"
        );
        let answer = self.files.read(name, cap);
        let bytes = match &answer {
            Read::Bytes(bytes) => bytes.len() as u64,
            _ => 0,
        };
        self.reads.push((name.to_owned(), bytes));
        answer
    }
    fn apply(&mut self, op: &Op) -> io::Result<()> {
        panic!("tab must write nothing: {op:?}");
    }
    fn list(&mut self, name: &str) -> io::Result<Vec<String>> {
        panic!("tab must list nothing: {name}");
    }
}

#[test]
fn s5_glance_reads_only_bounded_views_and_matches_independent_fold() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, &birth(S, 1));
    root.build();
    let expected = oracle(&dir, "alpha", Some(S));
    let mut read = ReadOnly {
        files: Files::at(&root.0),
        allowed: vec![
            "stats/epoch".into(),
            "stats/view".into(),
            "stats/intent".into(),
            format!("stats/session/{S}.view"),
        ],
        reads: Vec::new(),
    };
    let _ = disk::take_payload_names();
    let glance = disk::glance(&mut read, Some(S));
    let observed = disk::take_payload_names();
    assert_eq!(
        observed, read.reads,
        "observer calibrated against actual returned bytes and every payload read"
    );
    let Shown::Ready {
        view,
        epoch,
        r#gen,
        updating,
        listed,
        pending,
    } = disk::shown(&mut Files::at(&root.0), Some(S))
    else {
        panic!("old independent projection");
    };
    let Glance::Ready {
        epoch: e,
        r#gen: g,
        updating: u,
        all,
        ..
    } = &glance
    else {
        panic!("new Ready projection");
    };
    assert_eq!(
        (*e, *g, *u, all.listed, all.pending),
        (epoch, r#gen, updating, listed, pending)
    );
    assert_eq!(ready(glance), expected);
    for name in &read.allowed {
        assert_eq!(
            read.reads.iter().filter(|(read, _)| read == name).count(),
            1,
            "one stable open reads {name} exactly once"
        );
    }
    assert_eq!(
        view, expected,
        "additive facade preserves old Ready semantics"
    );
}

#[test]
fn s5_glance_pending_event_already_has_numbers_even_when_no_source_is_complete() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, b"");
    let line = String::from_utf8(birth(S, 1)).expect("fixture UTF8");
    assert!(
        crate::store::open(&dir)
            .append_event_counted(&line)
            .expect("counted append")
            .counted()
    );
    let Glance::Ready {
        view, all, this, ..
    } = disk::glance(&mut Files::at(&root.0), Some(S))
    else {
        panic!("event-time publish is already a published view");
    };
    assert_eq!(view.all.total, 1);
    assert_eq!((all.listed, all.pending), (1, 1));
    assert!(matches!(this, ThisProgress::Ready(p) if (p.listed, p.pending) == (1, 1)));
}

#[test]
fn s5_session_absence_damage_and_no_uuid_do_not_destroy_all_scope() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, &birth(S, 1));
    root.build();
    let expected = oracle(&dir, "alpha", Some(S)).all;
    for scope in [Some(T), None, Some("not-a-canonical-uuid")] {
        let Glance::Ready { view, this, .. } = disk::glance(&mut Files::at(&root.0), scope) else {
            panic!("ALL remains available");
        };
        assert_eq!(view.all, expected);
        if scope == Some(T) {
            assert!(matches!(this, ThisProgress::Unregistered));
        } else {
            assert!(matches!(this, ThisProgress::NoUuid));
        }
    }
    let path = root.0.join(format!("stats/session/{S}.view"));
    let valid = fs::read(&path).expect("valid session view");
    for missing in [true, false] {
        if missing {
            fs::remove_file(&path).expect("missing session view");
        } else {
            fs::write(&path, b"broken session view\n").expect("damaged session view");
        }
        let Glance::Ready { view, this, .. } = disk::glance(&mut Files::at(&root.0), Some(S))
        else {
            panic!("session damage must stay confined to that scope");
        };
        assert_eq!(view.all, expected);
        assert!(matches!(this, ThisProgress::Damaged(_)));
        fs::write(&path, &valid).expect("restore valid session view");
    }
}

#[test]
fn s5_missing_root_is_building_and_corrupt_unsupported_oversized_root_is_named() {
    let root = Fixture::new();
    assert!(matches!(
        disk::glance(&mut Files::at(&root.0), Some(S)),
        Glance::Building
    ));
    root.live("alpha", S, &birth(S, 1));
    root.build();
    let path = root.0.join("stats/view");
    let valid = fs::read(&path).expect("valid root view");
    fs::remove_file(&path).expect("missing root view");
    assert!(matches!(
        disk::glance(&mut Files::at(&root.0), Some(S)),
        Glance::Building
    ));
    for bytes in [
        b"broken root\n".to_vec(),
        String::from_utf8(valid.clone())
            .expect("UTF8 root")
            .replacen("\"stats_root\":1", "\"stats_root\":999", 1)
            .into_bytes(),
        vec![
            b'x';
            usize::try_from(disk::ROOT_VIEW_CAP).expect("root cap fits the fixture address space")
                + 1
        ],
    ] {
        assert_ne!(
            bytes, valid,
            "each fixture must actually change the subject"
        );
        fs::write(&path, bytes).expect("bad root view");
        let Glance::Refused(why) = disk::glance(&mut Files::at(&root.0), Some(S)) else {
            panic!("named root refusal");
        };
        assert!(
            why.describe().contains("stats/view"),
            "refusal must name the damaged document: {why:?}"
        );
    }
}

#[test]
fn s5_reset_preserves_lock_inode_and_journals_and_rebuilds_fold_equal() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, &birth(S, 1));
    root.build();
    let expected = oracle(&dir, "alpha", Some(S));
    let journal = fs::read(dir.join("events.jsonl")).expect("authoritative journal");
    let lock = fs::metadata(root.0.join("stats.lock")).expect("permanent root lock");
    let Glance::Ready { epoch, .. } = disk::glance(&mut Files::at(&root.0), Some(S)) else {
        panic!("built view");
    };
    let result = disk::reset(&root.0, Duration::ZERO).expect("reset acquired lock");
    assert!(matches!(result, Reset::Rebuilt { epoch: next } if next > epoch));
    let after = fs::metadata(root.0.join("stats.lock")).expect("lock never removed");
    assert_eq!((after.dev(), after.ino()), (lock.dev(), lock.ino()));
    assert_eq!(
        fs::read(dir.join("events.jsonl")).expect("journal survives reset"),
        journal
    );
    assert!(matches!(
        disk::glance(&mut Files::at(&root.0), Some(S)),
        Glance::Building
    ));
    assert!(
        !root.0.join(format!("stats/session/{S}.evidence")).exists(),
        "derived evidence removed"
    );
    root.build();
    assert_eq!(
        ready(disk::glance(&mut Files::at(&root.0), Some(S))),
        expected
    );
}

#[test]
fn s5_busy_reset_keeps_payload_lock_and_journal_bytes_and_returns_promptly() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, &birth(S, 1));
    root.build();
    let paths = [
        root.0.join("stats/epoch"),
        root.0.join("stats/view"),
        root.0.join(format!("stats/session/{S}.view")),
        root.0.join(format!("stats/session/{S}.evidence")),
        root.0.join("stats.lock"),
        dir.join("events.jsonl"),
    ];
    let before: Vec<_> = paths
        .iter()
        .map(|path| {
            let m = fs::metadata(path).expect("before metadata");
            (fs::read(path).expect("before bytes"), m.dev(), m.ino())
        })
        .collect();
    let held = Payload::at(&root.0)
        .lock(Duration::ZERO)
        .expect("competing writer");
    let started = Instant::now();
    let answer = disk::reset(&root.0, Duration::from_millis(25)).expect("busy is named, not IO");
    assert_eq!(answer, Reset::Busy);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "bounded lock wait with host slack"
    );
    for (path, expected) in paths.iter().zip(before) {
        let m = fs::metadata(path).expect("after metadata");
        assert_eq!(
            (fs::read(path).expect("after bytes"), m.dev(), m.ino()),
            expected,
            "busy must change nothing at {}",
            path.display()
        );
    }
    drop(held);
}
