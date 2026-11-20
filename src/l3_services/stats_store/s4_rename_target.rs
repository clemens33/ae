//! Frozen S4 rename: old target names cannot acquire the source identity.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned fixture IO and contract assertions"
)]

use super::disk::{self, Dir, FileJournals, Files, Op, Read, Taken};
use super::s4_behavior::Fixture;
use std::fs;
use std::time::Duration;

struct Controls;
impl Drop for Controls {
    fn drop(&mut self) {
        disk::clear_before_lock();
        disk::set_snapshot_cap(None);
    }
}

#[test]
fn s4_renamed_directory_never_inherits_the_old_empty_target_listing() {
    let root = Fixture::new();
    fs::create_dir(root.0.join("from")).expect("source directory");
    fs::write(root.0.join("from/child"), b"data").expect("source child");
    fs::create_dir(root.0.join("to")).expect("existing empty target");
    let _controls = Controls;
    disk::clear_before_lock();
    disk::set_snapshot_cap(None);
    let _ = disk::take_payload_reads();
    let _ = disk::take_journal_reads();
    let _ = disk::take_file_events();
    let _ = disk::take_attempts();
    let _ = disk::take_holds();
    let answer = disk::transact(
        &root.0,
        &mut Files::at(&root.0),
        &mut FileJournals::at(&root.0),
        Duration::ZERO,
        &mut Duration::default(),
        |files, _| {
            // Cache the old target listing, but never list the source root.
            let _ = files.list("to")?;
            match files.read("from/child", 4) {
                Read::Bytes(bytes) => {
                    assert_eq!(bytes, b"data");
                    files.apply(&Op::Rename {
                        from: "from".to_owned(),
                        to: "to".to_owned(),
                    })?;
                }
                Read::Absent => {} // A safe retry sees the completed rename.
                other => panic!("source must be present or already moved: {other:?}"),
            }
            files.list("to")
        },
    );
    match answer {
        Taken::Done(Ok(names)) => assert_eq!(names, vec!["child".to_owned()]),
        other => panic!("answer must name the actual moved child: {other:?}"),
    }
    assert!(!root.0.join("from").exists());
    assert_eq!(
        fs::read(root.0.join("to/child")).expect("moved child"),
        b"data"
    );
    let reads = disk::take_payload_reads();
    assert_eq!(reads.opened_under_lock, 0);
    assert_eq!(reads.listed_under_lock, 0);
    assert_eq!(reads.opened_under_lock_exception, 0);
    assert_eq!(reads.listed_under_lock_exception, 0);
    assert_eq!(disk::take_journal_reads().total, 0);
}
