//! Frozen prune authority: a foreign tally survives missing evidence.
#![allow(clippy::expect_used, reason = "acceptance fixtures and assertions")]
#![allow(clippy::duplicate_mod, reason = "shared frozen fold oracle")]
#[path = "s3_cases.rs"]
mod cases;
use super::disk::{self, Dir, Memory, MemoryJournals, Op, Read};

fn bytes(dir: &mut Memory, path: &str, cap: u64) -> Vec<u8> {
    match dir.read(path, cap) {
        Read::Bytes(bytes) => bytes,
        other => panic!("fixture document {path}: {other:?}"),
    }
}

#[test]
fn s3_prune_keeps_a_fragment_only_view_when_its_evidence_is_missing() {
    let mut dir = Memory::new();
    let mut journals = MemoryJournals::new();
    let mut feeds = cases::seed(&mut dir, &mut journals);
    let removal = cases::plan(
        disk::remove(&mut dir, Some(cases::T), "live/b", None).expect("remove T's own source"),
    );
    cases::complete(&mut dir, &removal);
    feeds[1].active = false;
    cases::check(&mut dir, &feeds);
    // The view has no own rows, but S's explicit birth(T, 2) still counts.
    let view_path = format!("stats/session/{}.view", cases::T);
    let evidence_path = format!("stats/session/{}.evidence", cases::T);
    let view = bytes(&mut dir, &view_path, disk::SESSION_VIEW_CAP);
    let evidence = bytes(&mut dir, &evidence_path, disk::EVIDENCE_CAP);
    let root = bytes(&mut dir, "stats/view", disk::ROOT_VIEW_CAP);
    dir.apply(&Op::Remove(evidence_path.clone()))
        .expect("lose only T evidence");

    let plan = disk::prune(&mut dir, Some(cases::T)).expect("view counts make prune a no-op");
    assert!(
        plan.ops.is_empty(),
        "a surviving foreign tally forbids pruning its view and root row: {plan:?}"
    );
    cases::complete(&mut dir, &plan);
    assert_eq!(
        bytes(&mut dir, &view_path, disk::SESSION_VIEW_CAP),
        view,
        "foreign-count view bytes preserved"
    );
    assert_eq!(
        bytes(&mut dir, "stats/view", disk::ROOT_VIEW_CAP),
        root,
        "root bytes preserved"
    );
    assert!(matches!(
        dir.read(&evidence_path, disk::EVIDENCE_CAP),
        Read::Absent
    ));

    // Restoring only the removed evidence must recover the original fold.
    dir.plant(&evidence_path, &evidence);
    cases::check(&mut dir, &feeds);
}
