//! Frozen S4 remaining guards: stale decisions never write; journal bytes count toward the cap.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned fixture IO and contract assertions"
)]

use super::disk::{self, Attempt, Dir, FileJournals, Files, Journal, Op, Taken};
use super::s4_behavior::{Fixture, S, birth};
use std::fs;
use std::time::Duration;

struct Controls;
impl Drop for Controls {
    fn drop(&mut self) {
        disk::clear_before_lock();
        disk::set_snapshot_cap(None);
    }
}

fn reset() -> Controls {
    disk::clear_before_lock();
    disk::set_snapshot_cap(None);
    let _ = disk::take_payload_reads();
    let _ = disk::take_journal_reads();
    let _ = disk::take_file_events();
    let _ = disk::take_attempts();
    let _ = disk::take_holds();
    Controls
}

#[test]
fn s4_ignored_stale_read_cannot_apply_a_later_operation() {
    let root = Fixture::new();
    fs::write(root.0.join("dependency"), b"old").expect("initial read dependency");
    let _controls = reset();
    disk::before_lock(|root, _| {
        fs::write(root.join("replacement"), b"new").expect("replacement dependency");
        fs::rename(root.join("replacement"), root.join("dependency"))
            .expect("invalidate every warm with a new inode");
    });
    let mut held = Duration::ZERO;
    let result = disk::transact(
        &root.0,
        &mut Files::at(&root.0),
        &mut FileJournals::at(&root.0),
        Duration::ZERO,
        &mut held,
        |files, _| {
            let _ = files.read("dependency", 64);
            files.apply(&Op::Write {
                name: "forbidden".to_owned(),
                bytes: b"a stale decision must not write".to_vec(),
            })
        },
    );
    assert!(matches!(result, Taken::Contended));
    assert_eq!(disk::take_attempts(), vec![Attempt::Stale; 3]);
    assert_eq!(disk::take_holds().len(), 3);
    assert!(
        disk::take_file_events().is_empty(),
        "stale apply has no prefix side effect"
    );
    assert!(
        !root.0.join("forbidden").exists(),
        "a dropped read result does not authorize a write"
    );
    let reads = disk::take_payload_reads();
    assert_eq!(reads.opened_under_lock, 0);
    assert_eq!(reads.listed_under_lock, 0);
    assert_eq!(reads.opened_under_lock_exception, 0);
    assert_eq!(reads.listed_under_lock_exception, 0);
    assert_eq!(disk::take_journal_reads().total, 0);
}

#[test]
fn s4_journal_window_bytes_alone_refuse_snapshot_budget_before_the_lock() {
    let root = Fixture::new();
    let body = birth(S, 1);
    let dir = root.live("alpha", S, &body);
    assert!(body.len() >= 32);
    let controls = reset();
    disk::set_snapshot_cap(Some(1));
    let mut held = Duration::ZERO;
    let result = disk::transact(
        &root.0,
        &mut Files::at(&root.0),
        &mut FileJournals::at(&root.0),
        Duration::ZERO,
        &mut held,
        |_, journals| journals.window("live/alpha", 0, 32),
    );
    assert!(
        matches!(result, Taken::Over { recorded, cap: 1 } if recorded >= 32),
        "the actual journal window alone must exceed the one-byte cap"
    );
    assert!(disk::take_attempts().is_empty());
    assert!(disk::take_holds().is_empty());
    assert!(disk::take_file_events().is_empty());
    let reads = disk::take_payload_reads();
    assert_eq!(
        reads.opened, 0,
        "no payload-byte contribution hides missing journal accounting"
    );
    assert_eq!(reads.listed, 0);
    let journals = disk::take_journal_reads();
    assert!(
        journals.total >= 32,
        "the real journal window was read outside the lock"
    );
    assert_eq!(journals.under_lock, 0);
    assert_eq!(
        fs::read(dir.join("events.jsonl")).expect("unchanged journal"),
        body
    );

    drop(controls);
    let control = disk::transact(
        &root.0,
        &mut Files::at(&root.0),
        &mut FileJournals::at(&root.0),
        Duration::ZERO,
        &mut held,
        |_, journals| journals.window("live/alpha", 0, 32),
    );
    match control {
        Taken::Done(Ok(window)) => assert_eq!(window.bytes, body[..32]),
        other => panic!("ordinary budget must serve the same real journal window: {other:?}"),
    }
    assert_eq!(disk::take_journal_reads().under_lock, 0);
    assert!(disk::take_file_events().is_empty());
}
