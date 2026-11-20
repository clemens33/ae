//! Frozen prune authority: nonempty evidence survives missing materializations.
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

fn exercise(fragment_only: bool, root_absent: bool) {
    let mut dir = Memory::new();
    let mut journals = MemoryJournals::new();
    let mut feeds = cases::seed(&mut dir, &mut journals);
    let key = if fragment_only {
        let removal = cases::plan(
            disk::remove(&mut dir, Some(cases::T), "live/b", None).expect("remove T's own source"),
        );
        cases::complete(&mut dir, &removal);
        feeds[1].active = false;
        cases::T
    } else {
        cases::S
    };
    cases::check(&mut dir, &feeds);
    let view_path = format!("stats/session/{key}.view");
    let evidence_path = format!("stats/session/{key}.evidence");
    let view = bytes(&mut dir, &view_path, disk::SESSION_VIEW_CAP);
    let evidence = bytes(&mut dir, &evidence_path, disk::EVIDENCE_CAP);
    let root = bytes(&mut dir, "stats/view", disk::ROOT_VIEW_CAP);
    dir.apply(&Op::Remove(view_path.clone()))
        .expect("lose session materialization");
    if root_absent {
        dir.apply(&Op::Remove("stats/view".to_owned()))
            .expect("lose root materialization");
    }
    let root_before = dir.read("stats/view", disk::ROOT_VIEW_CAP);

    // A missing view does not turn surviving source/fragment evidence into
    // an empty group. Prune must neither repair the store nor erase evidence.
    let plan = disk::prune(&mut dir, Some(key)).expect("nonempty evidence makes prune a no-op");
    assert!(
        plan.ops.is_empty(),
        "a group holding a source or fragment cannot be pruned: {plan:?}"
    );
    cases::complete(&mut dir, &plan);
    assert_eq!(
        bytes(&mut dir, &evidence_path, disk::EVIDENCE_CAP),
        evidence,
        "prune preserves authoritative evidence bytes"
    );
    assert_eq!(
        dir.read("stats/view", disk::ROOT_VIEW_CAP),
        root_before,
        "prune preserves the root state"
    );
    assert!(matches!(
        dir.read(&view_path, disk::SESSION_VIEW_CAP),
        Read::Absent
    ));

    // Restore only the deliberately removed materializations, then prove
    // every original count and fragment remains using the independent oracle.
    dir.plant(&view_path, &view);
    if root_absent {
        dir.plant("stats/view", &root);
    }
    cases::check(&mut dir, &feeds);
}

#[test]
fn s3_prune_keeps_own_source_evidence_when_only_its_view_is_missing() {
    exercise(false, false);
}

#[test]
fn s3_prune_keeps_own_source_evidence_when_view_and_root_are_missing() {
    exercise(false, true);
}

#[test]
fn s3_prune_keeps_fragment_only_evidence_when_only_its_view_is_missing() {
    exercise(true, false);
}

#[test]
fn s3_prune_keeps_fragment_only_evidence_when_view_and_root_are_missing() {
    exercise(true, true);
}
