//! Frozen S4 own-write listings retain every unaffected sibling.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned fixture IO and independent contract assertions"
)]

use super::disk::{self, Dir, FileJournals, Files, Op, Taken};
use super::s4_behavior::Fixture;
use std::fs;
use std::time::Duration;

#[test]
fn s4_own_write_listings_keep_siblings_after_an_entry_and_a_removal() {
    let root = Fixture::new();
    fs::create_dir(root.0.join("dir")).expect("owned directory");
    fs::write(root.0.join("dir/a"), b"a").expect("first old name");
    fs::write(root.0.join("dir/b"), b"b").expect("unaffected sibling");
    let _ = disk::take_payload_reads();
    let _ = disk::take_journal_reads();
    let _ = disk::take_file_events();
    let mut held = Duration::ZERO;
    let answer = disk::transact(
        &root.0,
        &mut Files::at(&root.0),
        &mut FileJournals::at(&root.0),
        Duration::ZERO,
        &mut held,
        |files, _| {
            let _ = files.list("dir")?;
            files.apply(&Op::Write {
                name: "dir/c".to_owned(),
                bytes: b"c".to_vec(),
            })?;
            let entered = files.list("dir")?;
            files.apply(&Op::Remove("dir/a".to_owned()))?;
            let left = files.list("dir")?;
            Ok::<_, std::io::Error>((entered, left))
        },
    );
    match answer {
        Taken::Done(Ok((entered, left))) => {
            assert_eq!(entered, vec!["a", "b", "c"]);
            assert_eq!(left, vec!["b", "c"]);
        }
        other => panic!("returned listings must preserve unaffected siblings: {other:?}"),
    }
    assert!(!root.0.join("dir/a").exists());
    assert_eq!(fs::read(root.0.join("dir/b")).expect("kept sibling"), b"b");
    assert_eq!(fs::read(root.0.join("dir/c")).expect("new entry"), b"c");
    let reads = disk::take_payload_reads();
    assert_eq!(reads.opened_under_lock, 0);
    assert_eq!(reads.listed_under_lock, 0);
    assert_eq!(reads.opened_under_lock_exception, 0);
    assert_eq!(reads.listed_under_lock_exception, 0);
    assert_eq!(disk::take_journal_reads().total, 0);
}
