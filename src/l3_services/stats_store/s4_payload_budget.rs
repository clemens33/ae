//! Frozen S4 payload-budget addendum: payload bytes alone can refuse a transaction.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned fixture IO and contract assertions"
)]

use super::disk::{self, Counted, Refused, Seen, SourceSpec};
use super::s4_behavior::{Fixture, S, birth, drain, source};
use std::fs;
use std::os::unix::fs::MetadataExt as _;
use std::time::Duration;

struct Controls;
impl Drop for Controls {
    fn drop(&mut self) {
        disk::set_snapshot_cap(None);
    }
}

#[test]
fn s4_payload_bytes_alone_refuse_over_budget_before_a_hold_or_write() {
    let root = Fixture::new();
    let first = birth(S, 1);
    let dir = root.live("alpha", S, &first);
    drain(&mut root.maintainer());
    let before = fs::read(root.0.join("stats/view")).expect("caught-up view");
    assert!(
        before.len() > 1,
        "the payload alone exceeds the lowered cap"
    );
    let second = birth(S, 2);
    fs::write(dir.join("events.jsonl"), [&first[..], &second[..]].concat())
        .expect("append one uncounted birth to the same journal");
    let meta = fs::metadata(dir.join("events.jsonl")).expect("journal identity");
    let seen = Seen {
        dev: meta.dev(),
        ino: meta.ino(),
        len: meta.len(),
    };
    let spec = SourceSpec::Live {
        id: "live/alpha".to_owned(),
        label: "alpha".to_owned(),
        meta: fs::read(dir.join("meta")).expect("source meta"),
    };
    disk::set_snapshot_cap(None);
    let _ = disk::take_payload_reads();
    let _ = disk::take_journal_reads();
    let _ = disk::take_file_events();
    let _ = disk::take_attempts();
    let _ = disk::take_holds();
    let controls = Controls;
    disk::set_snapshot_cap(Some(1));

    let result = disk::counted(
        &root.0,
        &spec,
        first.len() as u64,
        &second[..second.len() - 1],
        seen,
        Duration::ZERO,
    );
    match result {
        Counted::Refused(Refused::Snapshot { recorded, cap }) => {
            assert_eq!(cap, 1);
            assert!(
                recorded >= before.len() as u64,
                "the complete warm includes at least the existing view payload"
            );
        }
        other => panic!("payload bytes alone must exceed the cap: {other:?}"),
    }
    assert_eq!(
        disk::take_journal_reads().total,
        0,
        "no journal-window bytes"
    );
    let reads = disk::take_payload_reads();
    assert!(reads.opened > 0, "real payloads were warmed");
    assert_eq!(reads.opened_under_lock, 0);
    assert_eq!(reads.listed_under_lock, 0);
    assert_eq!(reads.opened_under_lock_exception, 0);
    assert_eq!(reads.listed_under_lock_exception, 0);
    assert!(disk::take_attempts().is_empty(), "no lock-taking attempt");
    assert!(disk::take_holds().is_empty(), "no actual hold");
    assert!(disk::take_file_events().is_empty(), "no applied operation");
    assert_eq!(
        fs::read(root.0.join("stats/view")).expect("durable view"),
        before,
        "budget refusal keeps the existing view"
    );

    drop(controls);
    let control = disk::counted(
        &root.0,
        &spec,
        first.len() as u64,
        &second[..second.len() - 1],
        seen,
        Duration::ZERO,
    );
    assert!(
        matches!(control, Counted::Published { .. }),
        "same input publishes once the ordinary budget is restored: {control:?}"
    );
    assert_eq!(disk::take_journal_reads().total, 0);
    drain(&mut root.maintainer());
    root.check(|| vec![source(&dir, "alpha")]);
}
