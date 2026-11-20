//! Frozen S4 complete snapshot budget: exactly the cap fits.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned fixture IO and independent contract assertions"
)]

use super::disk::{self, Attempt, Dir, FileJournals, Files, Journal, Read, Taken};
use super::s4_behavior::{Fixture, S, birth};
use std::fs;
use std::time::Duration;

struct Controls;
impl Drop for Controls {
    fn drop(&mut self) {
        disk::set_snapshot_cap(None);
        disk::clear_before_lock();
    }
}

fn reset_counts() {
    let _ = disk::take_payload_reads();
    let _ = disk::take_journal_reads();
    let _ = disk::take_file_events();
    let _ = disk::take_attempts();
    let _ = disk::take_holds();
}

#[test]
fn s4_complete_snapshot_exactly_at_the_cap_is_accepted() {
    let root = Fixture::new();
    let body = birth(S, 1);
    let live = root.live("alpha", S, &body);
    let payload = b"data";
    fs::write(root.0.join("dependency"), payload).expect("four payload bytes");
    assert!(body.len() >= 32);
    let _controls = Controls;
    disk::clear_before_lock();
    reset_counts();
    // The complete snapshot contains four payload bytes and 32 journal bytes.
    disk::set_snapshot_cap(Some(36));
    let mut held = Duration::ZERO;
    let run = |files: &mut Files, journals: &mut FileJournals| {
        (
            files.read("dependency", 4),
            journals.window("live/alpha", 0, 32),
        )
    };
    let answer = disk::transact(
        &root.0,
        &mut Files::at(&root.0),
        &mut FileJournals::at(&root.0),
        Duration::ZERO,
        &mut held,
        run,
    );
    match answer {
        Taken::Done((Read::Bytes(bytes), Ok(window))) => {
            assert_eq!(bytes, payload);
            assert_eq!(window.bytes, body[..32]);
        }
        other => panic!("exactly 36 recorded bytes must fit cap 36: {other:?}"),
    }
    assert_eq!(disk::take_attempts(), vec![Attempt::Decided]);
    assert_eq!(disk::take_holds().len(), 1);
    assert!(disk::take_file_events().is_empty());
    let reads = disk::take_payload_reads();
    assert!(reads.opened >= 1);
    assert_eq!(reads.opened_under_lock, 0);
    assert_eq!(reads.listed_under_lock, 0);
    assert_eq!(reads.opened_under_lock_exception, 0);
    assert_eq!(reads.listed_under_lock_exception, 0);
    let journals = disk::take_journal_reads();
    assert!(journals.total >= 32);
    assert_eq!(journals.under_lock, 0);

    // One byte below the same complete snapshot refuses before any lock.
    reset_counts();
    disk::set_snapshot_cap(Some(35));
    let refused = disk::transact(
        &root.0,
        &mut Files::at(&root.0),
        &mut FileJournals::at(&root.0),
        Duration::ZERO,
        &mut held,
        run,
    );
    assert!(matches!(
        refused,
        Taken::Over {
            recorded: 36,
            cap: 35
        }
    ));
    assert!(disk::take_attempts().is_empty());
    assert!(disk::take_holds().is_empty());
    assert!(disk::take_file_events().is_empty());
    assert_eq!(disk::take_journal_reads().under_lock, 0);
    assert_eq!(
        fs::read(root.0.join("dependency")).expect("kept payload"),
        payload
    );
    assert_eq!(
        fs::read(live.join("events.jsonl")).expect("kept journal"),
        body
    );
}
