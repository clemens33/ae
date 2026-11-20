//! Frozen S4 listing-error addendum: stable errors decide, changed proofs retry.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned fixture IO and contract assertions"
)]

use super::disk::{self, Attempt, Dir, FileJournals, Files, Taken};
use super::s4_behavior::Fixture;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

struct Controls;
impl Drop for Controls {
    fn drop(&mut self) {
        disk::clear_before_lock();
    }
}

struct SearchPermission(PathBuf);
impl Drop for SearchPermission {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
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

fn listing(root: &Path, name: &str) -> Taken<io::Result<Vec<String>>> {
    disk::transact(
        root,
        &mut Files::at(root),
        &mut FileJournals::at(root),
        Duration::ZERO,
        &mut Duration::ZERO,
        |files, _| files.list(name),
    )
}

fn same_error(result: Taken<io::Result<Vec<String>>>, expected: &io::Error) {
    match result {
        Taken::Done(Err(why)) => {
            assert_eq!(why.kind(), expected.kind(), "preserve the OS error class");
            assert_eq!(why.to_string(), expected.to_string(), "preserve its text");
        }
        other => panic!("a stable listing failure must decide its named IO error: {other:?}"),
    }
}

fn no_held_reads_or_operations() {
    let reads = disk::take_payload_reads();
    assert_eq!(reads.opened, 0, "no payload content involved");
    assert_eq!(reads.opened_under_lock, 0);
    assert_eq!(reads.listed_under_lock, 0);
    assert_eq!(reads.opened_under_lock_exception, 0);
    assert_eq!(reads.listed_under_lock_exception, 0);
    assert_eq!(disk::take_journal_reads().total, 0);
    assert!(disk::take_file_events().is_empty(), "read-only decision");
}

#[test]
fn s4_plain_listing_preserves_not_a_directory_as_an_error() {
    let root = Fixture::new();
    fs::write(root.0.join("blocked"), b"regular parent").expect("non-directory parent");
    let expected = fs::read_dir(root.0.join("blocked/child"))
        .err()
        .expect("the OS refuses this listing");
    let why = Files::at(&root.0)
        .list("blocked/child")
        .expect_err("an unreadable directory is not an empty directory");
    assert_eq!(why.kind(), expected.kind());
    assert_eq!(why.to_string(), expected.to_string());
}

#[test]
fn s4_stable_listing_failure_decides_the_io_error_in_one_hold() {
    let root = Fixture::new();
    fs::write(root.0.join("blocked"), b"regular parent").expect("non-directory parent");
    let expected = fs::read_dir(root.0.join("blocked/child"))
        .err()
        .expect("the OS refuses this listing");
    let _controls = reset();
    same_error(listing(&root.0, "blocked/child"), &expected);
    assert_eq!(disk::take_attempts(), vec![Attempt::Decided]);
    assert_eq!(disk::take_holds().len(), 1, "no perpetual stale retry");
    no_held_reads_or_operations();
}

#[test]
fn s4_absent_listing_becoming_not_a_directory_retries_before_refusing() {
    let root = Fixture::new();
    fs::create_dir(root.0.join("parent")).expect("initial searchable parent");
    let _controls = reset();
    disk::before_lock(|root, attempt| {
        if attempt == 1 {
            fs::remove_dir(root.join("parent")).expect("remove the empty parent");
            fs::write(root.join("parent"), b"regular parent").expect("new parent shape");
        }
    });
    let result = listing(&root.0, "parent/child");
    let expected = fs::read_dir(root.0.join("parent/child"))
        .err()
        .expect("the current OS listing fails");
    same_error(result, &expected);
    assert_eq!(
        disk::take_attempts(),
        vec![Attempt::Stale, Attempt::Decided]
    );
    assert_eq!(disk::take_holds().len(), 2);
    no_held_reads_or_operations();
}

#[test]
fn s4_listing_permission_change_invalidates_the_cached_failure() {
    let root = Fixture::new();
    let sealed = root.0.join("sealed");
    fs::create_dir(&sealed).expect("directory for a permission failure");
    let _permission = SearchPermission(sealed.clone());
    fs::set_permissions(&sealed, fs::Permissions::from_mode(0o000)).expect("remove permissions");
    match fs::read_dir(&sealed) {
        Ok(_) => {
            eprintln!("SKIP permission fixture: this process bypasses directory permissions");
            return;
        }
        Err(why) => assert_eq!(why.kind(), io::ErrorKind::PermissionDenied),
    }
    let _controls = reset();
    disk::before_lock(|root, attempt| {
        if attempt == 1 {
            fs::set_permissions(root.join("sealed"), fs::Permissions::from_mode(0o700))
                .expect("change the warmed directory mode");
        }
    });
    match listing(&root.0, "sealed") {
        Taken::Done(Ok(names)) => assert!(names.is_empty()),
        other => panic!("the retry must list the now-readable directory: {other:?}"),
    }
    assert_eq!(
        disk::take_attempts(),
        vec![Attempt::Stale, Attempt::Decided]
    );
    assert_eq!(disk::take_holds().len(), 2);
    no_held_reads_or_operations();
}
