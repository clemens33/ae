//! Frozen S4 held-shape addendum: errors retain their class and changed shapes retry.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned fixture IO and contract assertions"
)]

use super::disk::{self, Attempt, Dir, FileJournals, Files, Read, Taken};
use super::s4_behavior::Fixture;
use std::fs;
use std::os::unix::fs::symlink;
use std::time::Duration;

struct Controls;
impl Drop for Controls {
    fn drop(&mut self) {
        disk::clear_before_lock();
    }
}

fn reset() -> Controls {
    disk::clear_before_lock();
    let _ = disk::take_payload_reads();
    let _ = disk::take_journal_reads();
    let _ = disk::take_file_events();
    let _ = disk::take_attempts();
    let _ = disk::take_holds();
    Controls
}

fn no_content_or_operations() {
    let reads = disk::take_payload_reads();
    assert_eq!(
        reads.opened, 0,
        "only a nonregular node or an error was met"
    );
    assert_eq!(reads.opened_under_lock, 0);
    assert_eq!(reads.listed_under_lock, 0);
    assert_eq!(reads.opened_under_lock_exception, 0);
    assert_eq!(reads.listed_under_lock_exception, 0);
    assert_eq!(disk::take_journal_reads().total, 0);
    assert!(
        disk::take_file_events().is_empty(),
        "read decision writes nothing"
    );
}

#[test]
fn s4_stable_not_a_directory_error_is_unreadable_without_stale_retries() {
    let root = Fixture::new();
    fs::write(root.0.join("blocked"), b"a regular parent").expect("non-directory parent");
    let _controls = reset();
    let mut held = Duration::ZERO;
    let result = disk::transact(
        &root.0,
        &mut Files::at(&root.0),
        &mut FileJournals::at(&root.0),
        Duration::ZERO,
        &mut held,
        |files, _| files.read("blocked/child", 1024),
    );
    assert!(
        matches!(result, Taken::Done(Read::Unreadable(_))),
        "a stable non-NotFound status error is a decided unreadable dependency"
    );
    assert_eq!(disk::take_attempts(), vec![Attempt::Decided]);
    assert_eq!(
        disk::take_holds().len(),
        1,
        "no stale retry for a stable error"
    );
    no_content_or_operations();
    assert_eq!(
        fs::read(root.0.join("blocked")).expect("parent kept"),
        b"a regular parent"
    );
}

#[test]
fn s4_changed_nonregular_shape_retries_and_reports_the_current_shape() {
    let root = Fixture::new();
    symlink("missing-target", root.0.join("shape")).expect("symlink dependency");
    let _controls = reset();
    disk::before_lock(|root, attempt| {
        if attempt == 1 {
            fs::remove_file(root.join("shape")).expect("replace the warmed symlink");
            fs::create_dir(root.join("shape")).expect("different nonregular shape");
        }
    });
    let mut held = Duration::ZERO;
    let result = disk::transact(
        &root.0,
        &mut Files::at(&root.0),
        &mut FileJournals::at(&root.0),
        Duration::ZERO,
        &mut held,
        |files, _| files.read("shape", 1024),
    );
    assert!(
        matches!(result, Taken::Done(Read::Invalid(ref reason)) if reason == "a directory"),
        "the new warm must describe the current directory rather than the old symlink"
    );
    assert_eq!(
        disk::take_attempts(),
        vec![Attempt::Stale, Attempt::Decided]
    );
    assert_eq!(
        disk::take_holds().len(),
        2,
        "changed dependency is retried once"
    );
    no_content_or_operations();
    assert!(
        fs::symlink_metadata(root.0.join("shape"))
            .expect("current node")
            .is_dir()
    );
}
