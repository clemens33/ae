//! The board rename is visible before its directory sync and audit append.

#![allow(
    clippy::disallowed_methods,
    reason = "the acceptance fixture owns its record and journal paths"
)]

use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{SetError, SetOutcome, render_set, set_with_sync};
use crate::desk::parse_input;
use crate::store;
use crate::time::Timestamp;

const FIRST: &[u8] = b"ae-desk 1\nupdated 2026-11-20T08:40:12Z\nD1\tFirst board\n";
const AUDIT: &[u8] = b"{\"ts\":\"2026-11-20T08:40:12Z\",\"actor\":\"boss\",\"action\":\"desk\",\"summary\":\"+D1\"}\n";

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_sync_failure_preserves_the_visible_replacement_without_an_audit_or_refusal() {
    let root = Path::new("/tmp").join(format!(
        "ae-desk-sync-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    fs::create_dir(&root).expect("private board directory");
    let dir = Scratch(root);
    let path = dir.0.join("desk-record");
    let journal = dir.0.join("events.jsonl");
    let lock = store::lock_path(&path);
    fs::write(&path, FIRST).expect("initial board");
    fs::write(&journal, AUDIT).expect("initial audit");
    let now = Timestamp::parse("2026-11-20T08:41:12Z").expect("different update time");

    let same = parse_input(b"DECIDE\nD1 | First board\n").expect("unchanged input");
    let same = set_with_sync(&dir.0, same, now, "boss", |_| {
        panic!("an unchanged board never reaches directory sync")
    });
    assert!(matches!(&same, Ok(SetOutcome::Unchanged)), "{same:?}");
    assert_eq!(fs::read(&path).expect("unchanged board"), FIRST);
    assert_eq!(fs::read(&journal).expect("unchanged audit"), AUDIT);

    let next = parse_input(b"RUNNING\nR8 | Ready for review\n").expect("valid replacement");
    let expected = b"ae-desk 1\nupdated 2026-11-20T08:41:12Z\nR8\tReady for review\n";
    let result = set_with_sync(&dir.0, next, now, "boss", |observed| {
        assert_eq!(observed, dir.0.as_path());
        let held = store::lock(&lock, Duration::ZERO).expect_err("writer holds the lock at sync");
        assert_eq!(held.kind(), ErrorKind::WouldBlock);
        assert_eq!(
            fs::read(&path).expect("rename is visible before sync"),
            expected
        );
        assert_eq!(fs::read(&journal).expect("audit is still untouched"), AUDIT);
        Err(io::Error::other("fixture directory sync failed"))
    });
    assert!(matches!(&result, Err(SetError::Unsynced(_))), "{result:?}");
    assert_eq!(
        fs::read(&path).expect("replacement remains visible"),
        expected
    );
    assert_eq!(
        fs::read(&journal).expect("failed sync skips the audit"),
        AUDIT
    );

    let outcome = render_set(&result, 1);
    assert_eq!(outcome.code, 2);
    assert!(outcome.stdout.is_empty());
    assert!(
        outcome.stderr.starts_with("desk set UNCONFIRMED:")
            && outcome.stderr.contains("the board was replaced")
            && outcome.stderr.contains("may not survive a crash")
            && outcome.stderr.contains("no journal event was written")
            && outcome.stderr.contains("fixture directory sync failed"),
        "{}",
        outcome.stderr
    );
}
