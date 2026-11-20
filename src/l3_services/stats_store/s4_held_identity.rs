//! Frozen S4 held-identity addendum: each regular payload validates its own identity.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned fixture IO and contract assertions"
)]

use super::disk::{self, Attempt, Dir, FileJournals, Files, Read, Taken};
use super::s4_behavior::Fixture;
use std::fs;
use std::os::unix::fs::MetadataExt as _;
use std::time::Duration;

struct Controls;
impl Drop for Controls {
    fn drop(&mut self) {
        disk::clear_before_lock();
    }
}

fn changed_regular_payload(cap: u64, old: &[u8], new: &[u8]) {
    let root = Fixture::new();
    fs::write(root.0.join("payload"), old).expect("initial regular payload");
    let old_meta = fs::metadata(root.0.join("payload")).expect("old identity");
    let old_identity = (old_meta.dev(), old_meta.ino());
    let new_bytes = new.to_vec();
    disk::clear_before_lock();
    let _ = disk::take_payload_reads();
    let _ = disk::take_journal_reads();
    let _ = disk::take_file_events();
    let _ = disk::take_attempts();
    let _ = disk::take_holds();
    let _controls = Controls;
    disk::before_lock(move |root, attempt| {
        if attempt == 1 {
            fs::write(root.join("replacement"), &new_bytes).expect("replacement regular payload");
            fs::rename(root.join("replacement"), root.join("payload"))
                .expect("replace only the decision dependency");
            let current = fs::metadata(root.join("payload")).expect("new identity");
            assert_ne!(
                (current.dev(), current.ino()),
                old_identity,
                "inode replacement requires no timestamp or sleep assumption"
            );
        }
    });
    let mut held = Duration::ZERO;
    let result = disk::transact(
        &root.0,
        &mut Files::at(&root.0),
        &mut FileJournals::at(&root.0),
        Duration::ZERO,
        &mut held,
        |files, _| files.read("payload", cap),
    );
    match result {
        Taken::Done(Read::Bytes(bytes)) => assert_eq!(bytes, new, "serve the replacement bytes"),
        other => panic!("the retry must decide on the new in-cap payload: {other:?}"),
    }
    assert_eq!(
        disk::take_attempts(),
        vec![Attempt::Stale, Attempt::Decided]
    );
    assert_eq!(
        disk::take_holds().len(),
        2,
        "one stale hold and one decided hold"
    );
    let reads = disk::take_payload_reads();
    assert!(
        reads.opened >= 2,
        "each attempt warms a real regular payload"
    );
    assert_eq!(reads.opened_under_lock, 0);
    assert_eq!(reads.listed_under_lock, 0);
    assert_eq!(reads.opened_under_lock_exception, 0);
    assert_eq!(reads.listed_under_lock_exception, 0);
    assert_eq!(disk::take_journal_reads().total, 0);
    assert!(
        disk::take_file_events().is_empty(),
        "read decision applies no operation"
    );
    assert_eq!(
        fs::read(root.0.join("payload")).expect("replacement kept"),
        new
    );
}

#[test]
fn s4_cached_payload_bytes_validate_their_own_identity_before_serving() {
    changed_regular_payload(4, b"old!", b"new!");
}

#[test]
fn s4_cached_oversized_payload_validates_its_own_identity_before_refusing() {
    changed_regular_payload(1, b"long", b"x");
}
