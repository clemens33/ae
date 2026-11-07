//! Frozen S4 listing proof: a cached refusal still needs the same node identity.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned fixture IO and contract assertions"
)]

use super::disk::{self, Attempt, Dir, FileJournals, Files, Taken};
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
fn s4_replaced_nonregular_listing_dependency_retries_before_cached_refusal() {
    let root = Fixture::new();
    fs::create_dir(root.0.join("target")).expect("directory target");
    symlink("target", root.0.join("link")).expect("initial symlink dependency");
    let old = fs::symlink_metadata(root.0.join("link")).expect("initial node identity");
    let old_identity = (old.dev(), old.ino());
    assert!(
        Files::at(&root.0).list("link").is_ok(),
        "plain listing may follow the link; the held writer refuses its unproved directory status"
    );
    disk::clear_before_lock();
    let _ = disk::take_payload_reads();
    let _ = disk::take_journal_reads();
    let _ = disk::take_file_events();
    let _ = disk::take_attempts();
    let _ = disk::take_holds();
    let _controls = Controls;
    disk::before_lock(move |root, attempt| {
        if attempt == 1 {
            symlink("target", root.join("replacement")).expect("different symlink, same target");
            fs::rename(root.join("replacement"), root.join("link"))
                .expect("replace the warmed node");
            let current = fs::symlink_metadata(root.join("link")).expect("new identity");
            assert_ne!((current.dev(), current.ino()), old_identity);
        }
    });
    let mut held = Duration::ZERO;
    let result = disk::transact(
        &root.0,
        &mut Files::at(&root.0),
        &mut FileJournals::at(&root.0),
        Duration::ZERO,
        &mut held,
        |files, _| files.list("link"),
    );
    assert!(
        matches!(result, Taken::Done(Err(_))),
        "the current non-directory payload node remains an IO refusal"
    );
    assert_eq!(
        disk::take_attempts(),
        vec![Attempt::Stale, Attempt::Decided]
    );
    assert_eq!(disk::take_holds().len(), 2);
    let reads = disk::take_payload_reads();
    assert_eq!(reads.opened, 0);
    assert_eq!(reads.opened_under_lock, 0);
    assert_eq!(reads.listed_under_lock, 0);
    assert_eq!(reads.opened_under_lock_exception, 0);
    assert_eq!(reads.listed_under_lock_exception, 0);
    assert_eq!(disk::take_journal_reads().total, 0);
    assert!(
        disk::take_file_events().is_empty(),
        "neither refusal applies an operation"
    );
}
