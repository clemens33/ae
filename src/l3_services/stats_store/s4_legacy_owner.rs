//! Frozen S4 legacy ownership: missing root indexes do not permit a second owner.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned fixture IO and contract assertions"
)]

use super::disk::{self, Counted, Why};
use super::doc;
use super::s4_behavior::{Fixture, S, T, birth, drain, source};
use std::fs;

#[test]
fn s4_legacy_unindexed_root_still_refuses_a_source_owned_by_another_group() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, &birth(S, 1));
    drain(&mut root.maintainer());
    let primary_path = root.0.join(format!("stats/session/{S}.evidence"));
    let primary = fs::read(&primary_path).expect("existing owner's evidence");
    let text = fs::read_to_string(root.0.join("stats/view")).expect("root projection");
    let mut legacy = doc::parse_root(&text).expect("valid projection fixture");
    assert!(!legacy.rows.is_empty());
    for row in legacy.rows.values_mut() {
        row.ids = None;
    }
    let legacy_text = doc::render_root(&legacy);
    assert!(
        !legacy_text.contains("\"ids\""),
        "old format omits the ownership index"
    );
    fs::write(root.0.join("stats/view"), &legacy_text).expect("valid legacy-format root");
    fs::write(dir.join("meta"), format!("session=alpha\nsession_id={T}\n"))
        .expect("same source now asks to belong to a different group");
    let _ = disk::take_payload_reads();
    let _ = disk::take_journal_reads();
    let _ = disk::take_file_events();
    let line = String::from_utf8(birth(S, 2)).expect("complete LF-terminated birth");
    let result = crate::store::open(&dir)
        .append_event_counted(&line)
        .expect("the journal append succeeds");
    assert_eq!(
        result,
        Counted::Skipped(Why::Owned),
        "an absent root index is not proof that an existing source is unowned"
    );
    assert!(result.reason().is_some());
    assert!(
        disk::take_file_events().is_empty(),
        "no registration or migration on a skipped event"
    );
    assert_eq!(disk::take_journal_reads().total, 0);
    let reads = disk::take_payload_reads();
    assert_eq!(reads.opened_under_lock, 0);
    assert_eq!(reads.listed_under_lock, 0);
    assert_eq!(reads.opened_under_lock_exception, 0);
    assert_eq!(reads.listed_under_lock_exception, 0);
    assert_eq!(
        fs::read(&primary_path).expect("original owner kept"),
        primary
    );
    assert_eq!(
        fs::read_to_string(root.0.join("stats/view")).expect("root kept"),
        legacy_text
    );
    assert!(
        !root.0.join(format!("stats/session/{T}.evidence")).exists(),
        "no second owning group created"
    );
    let journal = fs::read(dir.join("events.jsonl")).expect("both durable births");
    assert_eq!(journal.iter().filter(|byte| **byte == b'\n').count(), 2);
    drain(&mut root.maintainer());
    root.check(|| vec![source(&dir, "alpha")]);
}
