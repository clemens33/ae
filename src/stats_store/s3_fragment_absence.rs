//! Frozen regression: foreign counts do not imply an empty ownerless group.
#![allow(clippy::expect_used, reason = "acceptance fixtures and assertions")]
#![allow(clippy::duplicate_mod, reason = "shared frozen fold oracle")]
#[path = "s3_cases.rs"]
mod cases;
use super::disk::{self, Dir, Memory, MemoryJournals, Op, Read};

#[test]
fn s3_prune_missing_fragment_only_documents_refuses_without_rewriting_root() {
    let mut dir = Memory::new();
    let mut journals = MemoryJournals::new();
    let mut feeds = cases::seed(&mut dir, &mut journals);
    // Remove T's own source. S's journal still contributes its explicit
    // birth(T, 2), so T now has counts but no own source or ownership id.
    let removal = cases::plan(
        disk::remove(&mut dir, Some(cases::T), "live/b", None).expect("remove T's own source"),
    );
    cases::complete(&mut dir, &removal);
    feeds[1].active = false;
    cases::check(&mut dir, &feeds);
    let before = dir.read("stats/view", disk::ROOT_VIEW_CAP);
    assert!(matches!(before, Read::Bytes(_)));
    dir.apply(&Op::Remove(format!("stats/session/{}.view", cases::T)))
        .expect("lost fragment-only view");
    dir.apply(&Op::Remove(format!("stats/session/{}.evidence", cases::T)))
        .expect("lost fragment-only evidence");
    let outcome = disk::prune(&mut dir, Some(cases::T));
    assert!(
        matches!(outcome, Err(disk::Refused::Damaged { .. })),
        "foreign counts survive without own sources: missing documents do not prove emptiness: {outcome:?}"
    );
    assert_eq!(
        dir.read("stats/view", disk::ROOT_VIEW_CAP),
        before,
        "refusal preserves the root aggregate and fragment-only row for repair"
    );
}
