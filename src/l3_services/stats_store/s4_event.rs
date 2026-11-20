//! S4 event-time acceptance. Frozen S4 v1; navigator revision and lead gate required for changes.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned fixture IO and contract assertions"
)]

use super::disk::{
    self, Attempt, Counted, Dir, Files, Read, Refused, Seen, Shown, SourceSpec, Why,
};
use super::s4_behavior::{Fixture, S, T, birth, drain, source};
use crate::stats::{self, ThisSession};
use std::fs;
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;
use std::time::{Duration, Instant};

fn spec(dir: &Path, name: &str) -> SourceSpec {
    SourceSpec::Live {
        id: format!("live/{name}"),
        label: name.to_owned(),
        meta: fs::read(dir.join("meta")).expect("source meta"),
    }
}

fn seen(dir: &Path) -> Seen {
    let meta = fs::metadata(dir.join("events.jsonl")).expect("journal identity");
    Seen {
        dev: meta.dev(),
        ino: meta.ino(),
        len: meta.len(),
    }
}

fn bytes(files: &mut Files) -> Vec<(String, Vec<u8>)> {
    let mut names = vec!["stats/epoch".to_owned(), "stats/view".to_owned()];
    names.extend(
        files
            .list("stats/session")
            .expect("session file names")
            .into_iter()
            .map(|name| format!("stats/session/{name}")),
    );
    names
        .into_iter()
        .filter_map(|name| match files.read(&name, disk::EVIDENCE_CAP) {
            Read::Bytes(bytes) => Some((name, bytes)),
            Read::Absent => None,
            other => panic!("valid fixture payload: {other:?}"),
        })
        .collect()
}

fn no_reads() {
    assert_eq!(
        disk::take_journal_reads().total,
        0,
        "event publication pays no history debt"
    );
    let reads = disk::take_payload_reads();
    assert_eq!(reads.opened_under_lock, 0, "steady-state payload opens");
    assert_eq!(reads.listed_under_lock, 0, "steady-state listings");
    assert_eq!(reads.opened_under_lock_exception, 0, "clean fixture");
    assert_eq!(reads.listed_under_lock_exception, 0, "clean fixture");
}

fn line(uuid: &str, n: u32) -> String {
    String::from_utf8(birth(uuid, n)).expect("fixture JSON")
}

#[test]
fn s4_counted_append_cold_registers_pending_and_survives_a_new_reader_without_journal_reads() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, b"");
    let _ = disk::take_journal_reads();
    let _ = disk::take_payload_reads();
    let result = crate::store::open(&dir)
        .append_event_counted(&line(S, 1))
        .expect("journal append succeeds");
    assert!(matches!(result, Counted::Published { .. }), "{result:?}");
    no_reads();
    let journal = fs::read(dir.join("events.jsonl")).expect("actual stamped journal");
    let parsed = crate::json::parse(std::str::from_utf8(&journal).expect("JSON").trim_end())
        .expect("one JSON line");
    assert_eq!(
        parsed.get("schema_version"),
        Some(&crate::json::Value::Num(1)),
        "counted append preserves the stamp"
    );
    let mark = seen(&dir);
    let evidence =
        fs::read_to_string(root.0.join(format!("stats/session/{S}.evidence"))).expect("evidence");
    let (group, _) = super::doc::parse_group(&evidence).expect("existing persisted format");
    let own = group.own.get("live/alpha").expect("registered source");
    assert_eq!(
        own.evidence.cursor, 0,
        "event must not advance the sequential cursor"
    );
    assert_eq!(
        (own.provenance.dev, own.provenance.ino, own.provenance.seen),
        (mark.dev, mark.ino, mark.len)
    );
    fs::rename(dir.join("events.jsonl"), root.0.join("unavailable-journal"))
        .expect("history unavailable");
    let meta = fs::read(dir.join("meta")).expect("oracle meta");
    for scope in [None, Some(S), Some(T)] {
        let expected = stats::fold(
            vec![stats::Source::live("alpha", &meta, Ok(journal.clone()))],
            scope,
        );
        let Shown::Ready {
            view,
            pending,
            updating,
            ..
        } = disk::shown(&mut Files::at(&root.0), scope)
        else {
            panic!("durable cold event view");
        };
        assert!(!updating);
        assert_eq!(pending, 1, "history remains explicitly pending");
        assert_eq!(view, expected, "counted event already in every scope");
        assert!(view.all.balanced());
        if let ThisSession::Ready(this) = view.this {
            assert!(this.balanced());
        }
    }
    assert_eq!(disk::take_journal_reads().total, 0);
    fs::rename(root.0.join("unavailable-journal"), dir.join("events.jsonl"))
        .expect("restore source for catch-up");
    drain(&mut root.maintainer());
    root.check(|| vec![source(&dir, "alpha")]);
}

#[test]
fn s4_event_ahead_of_prepared_chunk_is_merged_and_not_counted_twice() {
    let root = Fixture::new();
    let first = birth(S, 1);
    let dir = root.live("alpha", S, &first);
    let mut maintainer = root.maintainer();
    drain(&mut maintainer);
    fs::write(dir.join("events.jsonl"), [first, birth(S, 2)].concat()).expect("second record");
    let old = maintainer.prepare().expect("old prepared chunk");
    assert!(
        crate::store::open(&dir)
            .append_event_counted(&line(S, 3))
            .expect("append")
            .counted()
    );
    let step = maintainer.commit(old);
    assert!(
        matches!(step.outcome, super::maint::Outcome::Published { .. }),
        "old chunk must merge current evidence: {step:?}"
    );
    let Shown::Ready { view, .. } = disk::shown(&mut Files::at(&root.0), Some(S)) else {
        panic!("merged view");
    };
    assert_eq!(view.all.total, 3, "future event survives old chunk");
    drain(&mut maintainer);
    root.check(|| vec![source(&dir, "alpha")]);
}

#[test]
fn s4_repeated_event_is_kept_and_generation_stays_unchanged() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, b"");
    crate::store::open(&dir)
        .append_event_counted(&line(S, 1))
        .expect("first append");
    let before = bytes(&mut Files::at(&root.0));
    let stamped = fs::read(dir.join("events.jsonl")).expect("actual event");
    let result = disk::counted(
        &root.0,
        &spec(&dir, "alpha"),
        0,
        &stamped[..stamped.len() - 1],
        seen(&dir),
        Duration::ZERO,
    );
    assert_eq!(result, Counted::Kept);
    assert_eq!(
        bytes(&mut Files::at(&root.0)),
        before,
        "idempotent insert rewrites nothing"
    );
}

fn skipped_transition(change: impl FnOnce(&Fixture, &Path), expected: Why) {
    let root = Fixture::new();
    let dir = root.live("alpha", S, &birth(S, 1));
    drain(&mut root.maintainer());
    change(&root, &dir);
    let before = bytes(&mut Files::at(&root.0));
    let offset = seen(&dir).len;
    crate::store::open(&dir)
        .append_event(&line(S, 2))
        .expect("journal authoritative append");
    let stamped = crate::store::stamp_event_records(&line(S, 2)).into_owned();
    let result = disk::counted(
        &root.0,
        &spec(&dir, "alpha"),
        offset,
        stamped.trim_end_matches('\n').as_bytes(),
        seen(&dir),
        Duration::ZERO,
    );
    assert_eq!(result, Counted::Skipped(expected));
    assert!(!result.counted());
    assert!(
        result.reason().is_some(),
        "the spawn can name every non-success"
    );
    assert_eq!(
        bytes(&mut Files::at(&root.0)),
        before,
        "an unsafe source transition writes nothing"
    );
    drain(&mut root.maintainer());
    root.check(|| vec![source(&dir, "alpha")]);
}

#[test]
fn s4_replaced_journal_incarnation_is_skipped_until_maintenance() {
    skipped_transition(
        |_, dir| {
            fs::write(dir.join("new-journal"), birth(S, 9)).expect("new inode");
            fs::rename(dir.join("new-journal"), dir.join("events.jsonl"))
                .expect("journal replacement");
        },
        Why::Journal,
    );
}

#[test]
fn s4_uuid_changed_source_is_skipped_without_cross_group_ownership() {
    skipped_transition(
        |_, dir| {
            fs::write(dir.join("meta"), format!("session=alpha\nsession_id={T}\n"))
                .expect("changed UUID")
        },
        Why::Owned,
    );
}

#[test]
fn s4_shrunk_same_inode_is_skipped_before_cursor_rules() {
    let root = Fixture::new();
    let dir = root.live(
        "alpha",
        S,
        &[birth(S, 1), birth(S, 2), birth(S, 3)].concat(),
    );
    drain(&mut root.maintainer());
    let before = bytes(&mut Files::at(&root.0));
    fs::write(dir.join("events.jsonl"), b"").expect("same-inode truncation");
    crate::store::open(&dir)
        .append_event(&line(S, 4))
        .expect("new birth");
    let event = crate::store::stamp_event_records(&line(S, 4)).into_owned();
    assert_eq!(
        disk::counted(
            &root.0,
            &spec(&dir, "alpha"),
            0,
            event.trim_end_matches('\n').as_bytes(),
            seen(&dir),
            Duration::ZERO
        ),
        Counted::Skipped(Why::Journal)
    );
    assert_eq!(bytes(&mut Files::at(&root.0)), before);
    drain(&mut root.maintainer());
    root.check(|| vec![source(&dir, "alpha")]);
}

#[test]
fn s4_already_consumed_same_journal_event_is_kept_without_a_write() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, &birth(S, 1));
    drain(&mut root.maintainer());
    let before = bytes(&mut Files::at(&root.0));
    let record = fs::read(dir.join("events.jsonl")).expect("consumed record");
    assert_eq!(
        disk::counted(
            &root.0,
            &spec(&dir, "alpha"),
            0,
            &record[..record.len() - 1],
            seen(&dir),
            Duration::ZERO
        ),
        Counted::Kept
    );
    assert_eq!(bytes(&mut Files::at(&root.0)), before);
}

#[test]
fn s4_journal_append_failure_never_publishes_statistics() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, b"");
    fs::remove_file(dir.join("events.jsonl")).expect("remove journal");
    fs::create_dir(dir.join("events.jsonl")).expect("nonregular append target");
    assert!(
        crate::store::open(&dir)
            .append_event_counted(&line(S, 1))
            .is_err()
    );
    assert!(
        !root.0.join("stats").exists(),
        "no statistics from a failed journal append"
    );
}

#[test]
fn s4_held_root_lock_returns_busy_within_one_poll_budget_and_preserves_the_journal() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, b"");
    let held = disk::Payload::at(&root.0)
        .lock(Duration::ZERO)
        .expect("test holds stats.lock");
    let started = Instant::now();
    let result = crate::store::open(&dir)
        .append_event_counted(&line(S, 1))
        .expect("journal append is independent");
    assert_eq!(result, Counted::Busy);
    assert!(
        started.elapsed() < Duration::from_millis(2000),
        "200 ms poll plus empirical host and append slack"
    );
    assert!(result.reason().is_some());
    assert!(!root.0.join("stats").exists());
    drop(held);
    drain(&mut root.maintainer());
    root.check(|| vec![source(&dir, "alpha")]);
}

#[test]
fn s4_event_retry_exhaustion_writes_nothing_and_returns_contended() {
    // Navigator revision (320 ruling B): the product keeps rule r2.4, so
    // the deadline is shared by lock wait and warm starts. The test pins
    // exhaustion only, so it holds a budget no load can pass: the stall
    // below is inside the budget because the budget widened, and without
    // the hold the same stall yields Busy (see the sibling below).
    let _held_wait = disk::hold_event_wait(Duration::from_secs(3600));
    let root = Fixture::new();
    let dir = root.live("alpha", S, &birth(S, 1));
    drain(&mut root.maintainer());
    let before = bytes(&mut Files::at(&root.0));
    let _ = disk::take_attempts();
    let _ = disk::take_file_events();
    let _ = disk::take_journal_reads();
    let _ = disk::take_payload_reads();
    disk::before_lock(|root, attempt| {
        if attempt == 2 {
            // A load stall: the second attempt's warm starts inside the
            // deadline (the deadline starts at attempt 1's lock request)
            // and outlasts the event wait only if the budget is narrow.
            std::thread::sleep(disk::EVENT_WAIT + Duration::from_millis(50));
        }
        assert!(
            disk::take_file_events().is_empty(),
            "stale event attempt applies nothing"
        );
        fs::write(
            root.join("replacement"),
            fs::read(root.join("stats/view")).expect("view"),
        )
        .expect("fresh inode");
        fs::rename(root.join("replacement"), root.join("stats/view"))
            .expect("replaced each attempt");
    });
    let result = crate::store::open(&dir)
        .append_event_counted(&line(S, 2))
        .expect("journal append succeeds");
    disk::clear_before_lock();
    assert_eq!(result, Counted::Contended);
    assert_eq!(disk::take_attempts(), vec![Attempt::Stale; 3]);
    assert_eq!(bytes(&mut Files::at(&root.0)), before);
    assert!(disk::take_file_events().is_empty());
    no_reads();
    drain(&mut root.maintainer());
    root.check(|| vec![source(&dir, "alpha")]);
}

/// The same stall at the product default wait: one deadline across warm
/// starts, so attempt 3 never warms and the count is Busy (r2.4) without
/// writing. Pairs with the retry test above: only the budget differs.
#[test]
fn s4_event_retry_expired_stale_attempt_is_busy_at_default_wait() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, &birth(S, 1));
    drain(&mut root.maintainer());
    let before = bytes(&mut Files::at(&root.0));
    let _ = disk::take_attempts();
    let _ = disk::take_file_events();
    let _ = disk::take_journal_reads();
    let _ = disk::take_payload_reads();
    disk::before_lock(|root, attempt| {
        if attempt == 2 {
            std::thread::sleep(disk::EVENT_WAIT + Duration::from_millis(50));
        }
        assert!(
            disk::take_file_events().is_empty(),
            "stale event attempt applies nothing"
        );
        fs::write(
            root.join("replacement"),
            fs::read(root.join("stats/view")).expect("view"),
        )
        .expect("fresh inode");
        fs::rename(root.join("replacement"), root.join("stats/view"))
            .expect("replaced each attempt");
    });
    let result = crate::store::open(&dir)
        .append_event_counted(&line(S, 2))
        .expect("journal append succeeds");
    disk::clear_before_lock();
    assert_eq!(
        result,
        Counted::Busy,
        "expired stale work stays in the journal"
    );
    assert_eq!(
        disk::take_attempts(),
        vec![Attempt::Stale, Attempt::Stale, Attempt::Busy]
    );
    assert_eq!(bytes(&mut Files::at(&root.0)), before);
    assert!(disk::take_file_events().is_empty());
    no_reads();
    drain(&mut root.maintainer());
    root.check(|| vec![source(&dir, "alpha")]);
}

#[test]
fn s4_damage_at_event_time_is_refused_without_repair() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, &birth(S, 1));
    drain(&mut root.maintainer());
    fs::write(root.0.join("stats/view"), b"damaged\n").expect("damage");
    let before = bytes(&mut Files::at(&root.0));
    let result = crate::store::open(&dir)
        .append_event_counted(&line(S, 2))
        .expect("journal append succeeds");
    assert!(
        matches!(result, Counted::Refused(Refused::Damaged { .. })),
        "{result:?}"
    );
    assert_eq!(
        bytes(&mut Files::at(&root.0)),
        before,
        "spawn does not repair damage"
    );
}

#[test]
fn s4_unreadable_source_meta_returns_a_named_non_success_without_registration() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, &birth(S, 1));
    let spec = SourceSpec::Live {
        id: "live/alpha".to_owned(),
        label: "alpha".to_owned(),
        meta: vec![0xff],
    };
    let record = birth(S, 1);
    let result = disk::counted(
        &root.0,
        &spec,
        0,
        &record[..record.len() - 1],
        seen(&dir),
        Duration::ZERO,
    );
    assert_eq!(result, Counted::Skipped(Why::Unreadable));
    assert!(!result.counted());
    assert!(result.reason().is_some());
    assert!(!root.0.join("stats").exists());
}

#[test]
fn s4_failed_persisted_source_is_not_silently_kept_as_a_success() {
    let mut files = disk::Memory::new();
    let mut journals = disk::MemoryJournals::new();
    let failed = SourceSpec::Failed {
        id: "live/alpha".to_owned(),
        label: "alpha".to_owned(),
        session: Some(S.to_owned()),
        gap: stats::SourceGap::Unreadable,
    };
    let disk::Next::Work(work) =
        disk::Prepared::read(&mut files, &mut journals, &failed, disk::LIMITS, None)
            .expect("failed registration")
    else {
        panic!("failure is recorded");
    };
    let disk::Decision::Publish(plan) =
        disk::publish(&mut files, &mut journals, &work).expect("failed source plan")
    else {
        panic!("failure registration publishes");
    };
    plan.apply(&mut files, plan.ops.len())
        .expect("failed source stored");
    let before = files.clone();
    let live = SourceSpec::Live {
        id: "live/alpha".to_owned(),
        label: "alpha".to_owned(),
        meta: format!("session=alpha\nsession_id={S}\n").into_bytes(),
    };
    let record = birth(S, 1);
    let result = disk::event_counted(
        &mut files,
        &live,
        0,
        &record[..record.len() - 1],
        Seen {
            dev: 1,
            ino: 2,
            len: record.len() as u64,
        },
    )
    .expect("named failed-source decision");
    assert_eq!(result, disk::Decision::Discard(Why::Failed));
    assert_eq!(
        files.names(),
        before.names(),
        "failed source cannot add a payload name"
    );
    for name in before.names() {
        assert_eq!(
            files.file(&name),
            before.file(&name),
            "failed source cannot write {name}"
        );
    }
}

#[test]
fn s4_text_meta_without_uuid_registers_the_idless_source_and_keeps_foreign_birth_scope() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, b"");
    fs::write(dir.join("meta"), b"session=alpha\n").expect("valid legacy idless meta");
    let result = crate::store::open(&dir)
        .append_event_counted(&line(S, 1))
        .expect("append");
    assert!(result.counted(), "idless text meta is readable: {result:?}");
    drain(&mut root.maintainer());
    root.check(|| vec![source(&dir, "alpha")]);
}

#[path = "s4_cases.rs"]
mod cases;

#[test]
fn s4_registration_crash_prefix_and_incarnation_cases_match_the_original_fold() {
    for operation in 0..4 {
        for cut in 0..64 {
            cases::exercise(&[operation, cut]);
        }
    }
}
