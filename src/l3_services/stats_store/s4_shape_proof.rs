//! Frozen S4 refusal proof: a replacement of the same node class is stale.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned fixture IO and contract assertions"
)]

use super::disk::{self, Attempt, Dir, FileJournals, Files, Read, Taken};
use super::s4_behavior::Fixture;
use std::fs;
use std::os::unix::fs::{MetadataExt as _, symlink};
use std::time::Duration;

struct Controls;
impl Drop for Controls {
    fn drop(&mut self) {
        disk::clear_before_lock();
    }
}

#[test]
fn s4_replaced_same_class_payload_refusal_requires_a_new_warm() {
    let root = Fixture::new();
    symlink("missing", root.0.join("shape")).expect("initial symlink");
    symlink("missing", root.0.join("replacement")).expect("replacement symlink");
    let old = fs::symlink_metadata(root.0.join("shape")).expect("initial identity");
    let new = fs::symlink_metadata(root.0.join("replacement")).expect("replacement identity");
    assert_ne!((old.dev(), old.ino()), (new.dev(), new.ino()));
    let _controls = Controls;
    disk::clear_before_lock();
    let _ = disk::take_payload_reads();
    let _ = disk::take_journal_reads();
    let _ = disk::take_file_events();
    let _ = disk::take_attempts();
    let _ = disk::take_holds();
    disk::before_lock(|root, attempt| {
        if attempt == 1 {
            fs::rename(root.join("replacement"), root.join("shape"))
                .expect("replace the warmed node without changing its class");
        }
    });
    let mut held = Duration::ZERO;
    let answer = disk::transact(
        &root.0,
        &mut Files::at(&root.0),
        &mut FileJournals::at(&root.0),
        Duration::ZERO,
        &mut held,
        |files, _| files.read("shape", 1024),
    );
    assert!(matches!(answer, Taken::Done(Read::Invalid(ref why)) if why == "a symlink"));
    assert_eq!(
        disk::take_attempts(),
        vec![Attempt::Stale, Attempt::Decided]
    );
    assert_eq!(disk::take_holds().len(), 2);
    let reads = disk::take_payload_reads();
    assert_eq!(reads.opened, 0);
    assert_eq!(reads.opened_under_lock, 0);
    assert_eq!(reads.listed, 0);
    assert_eq!(reads.listed_under_lock, 0);
    assert_eq!(reads.opened_under_lock_exception, 0);
    assert_eq!(reads.listed_under_lock_exception, 0);
    assert_eq!(disk::take_journal_reads().total, 0);
    assert!(disk::take_file_events().is_empty());
    let current = fs::symlink_metadata(root.0.join("shape")).expect("current symlink");
    assert_eq!((current.dev(), current.ino()), (new.dev(), new.ino()));
}
