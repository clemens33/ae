//! Frozen S4 own-write addendum: a held decision reads its applied operations.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned fixture IO and contract assertions"
)]

use super::disk::{self, Attempt, Dir, FileJournals, Files, Op, Read, Taken};
use super::s4_behavior::Fixture;
use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;

fn reset() {
    disk::clear_before_lock();
    let _ = disk::take_payload_reads();
    let _ = disk::take_journal_reads();
    let _ = disk::take_file_events();
    let _ = disk::take_attempts();
    let _ = disk::take_holds();
}

fn transact<T>(root: &Path, mut body: impl FnMut(&mut Files) -> T) -> Taken<T> {
    let mut held = Duration::ZERO;
    disk::transact(
        root,
        &mut Files::at(root),
        &mut FileJournals::at(root),
        Duration::ZERO,
        &mut held,
        |files, _| body(files),
    )
}

fn one_applied_hold() {
    assert_eq!(disk::take_attempts(), vec![Attempt::Applied]);
    assert_eq!(
        disk::take_holds().len(),
        1,
        "own operations require no stale retry"
    );
    let reads = disk::take_payload_reads();
    assert_eq!(reads.opened_under_lock, 0);
    assert_eq!(reads.listed_under_lock, 0);
    assert_eq!(reads.opened_under_lock_exception, 0);
    assert_eq!(reads.listed_under_lock_exception, 0);
    assert_eq!(disk::take_journal_reads().total, 0);
    assert!(
        !disk::take_file_events().is_empty(),
        "the real operation applied"
    );
}

#[test]
fn s4_rename_keeps_cached_source_bytes_and_updates_the_destination_listing() {
    let root = Fixture::new();
    fs::write(root.0.join("old"), b"payload").expect("cached source file");
    reset();
    let result = transact(&root.0, |files| -> io::Result<_> {
        let _ = files.read("old", 64);
        let _ = files.read("new", 64);
        let _ = files.list("")?;
        files.apply(&Op::Rename {
            from: "old".to_owned(),
            to: "new".to_owned(),
        })?;
        Ok((
            files.read("new", 64),
            files.read("old", 64),
            files.list("")?,
        ))
    });
    match result {
        Taken::Done(Ok((new, old, names))) => {
            assert_eq!(new, Read::Bytes(b"payload".to_vec()));
            assert_eq!(old, Read::Absent);
            assert!(names.iter().any(|name| name == "new"));
            assert!(!names.iter().any(|name| name == "old"));
        }
        other => panic!("the held decision must read its actual file rename: {other:?}"),
    }
    one_applied_hold();
    assert_eq!(
        fs::read(root.0.join("new")).expect("renamed payload"),
        b"payload"
    );
    assert!(!root.0.join("old").exists());
}

#[test]
fn s4_directory_rename_moves_cached_descendant_bytes_and_listings() {
    let root = Fixture::new();
    fs::create_dir_all(root.0.join("from/inner")).expect("source directory tree");
    fs::write(root.0.join("from/inner/item"), b"payload").expect("source descendant");
    fs::create_dir(root.0.join("to")).expect("empty rename destination");
    reset();
    let result = transact(&root.0, |files| -> io::Result<_> {
        let _ = files.read("from/inner/item", 64);
        let _ = files.read("to/inner/item", 64);
        let _ = files.list("from")?;
        let _ = files.list("from/inner")?;
        let _ = files.list("to")?;
        let _ = files.list("to/inner")?;
        files.apply(&Op::Rename {
            from: "from".to_owned(),
            to: "to".to_owned(),
        })?;
        Ok((
            files.read("to/inner/item", 64),
            files.list("to")?,
            files.list("to/inner")?,
            files.list("from")?,
        ))
    });
    match result {
        Taken::Done(Ok((bytes, to, inner, from))) => {
            assert_eq!(bytes, Read::Bytes(b"payload".to_vec()));
            assert_eq!(to, vec!["inner".to_owned()]);
            assert_eq!(inner, vec!["item".to_owned()]);
            assert!(from.is_empty());
        }
        other => panic!("the held decision must read its actual directory rename: {other:?}"),
    }
    one_applied_hold();
    assert_eq!(
        fs::read(root.0.join("to/inner/item")).expect("moved payload"),
        b"payload"
    );
    assert!(!root.0.join("from").exists());
}

#[test]
fn s4_remove_tree_preserves_unrelated_cached_bytes_and_listings() {
    let root = Fixture::new();
    fs::create_dir_all(root.0.join("gone/child")).expect("tree to remove");
    fs::write(root.0.join("gone/child/item"), b"removed").expect("removed descendant");
    fs::create_dir(root.0.join("keep")).expect("unrelated directory");
    fs::write(root.0.join("keep/item"), b"kept").expect("unrelated payload");
    reset();
    let result = transact(&root.0, |files| -> io::Result<_> {
        let _ = files.read("keep/item", 64);
        let _ = files.read("gone/child/item", 64);
        let _ = files.list("keep")?;
        let _ = files.list("gone")?;
        let _ = files.list("gone/child")?;
        files.apply(&Op::RemoveTree("gone".to_owned()))?;
        Ok((
            files.read("keep/item", 64),
            files.list("keep")?,
            files.read("gone", 64),
            files.list("gone")?,
        ))
    });
    match result {
        Taken::Done(Ok((bytes, keep, gone, gone_names))) => {
            assert_eq!(bytes, Read::Bytes(b"kept".to_vec()));
            assert_eq!(keep, vec!["item".to_owned()]);
            assert_eq!(gone, Read::Absent);
            assert!(gone_names.is_empty());
        }
        other => panic!("removing one tree must preserve unrelated cached answers: {other:?}"),
    }
    one_applied_hold();
    assert_eq!(
        fs::read(root.0.join("keep/item")).expect("unrelated payload kept"),
        b"kept"
    );
    assert!(!root.0.join("gone").exists());
}
