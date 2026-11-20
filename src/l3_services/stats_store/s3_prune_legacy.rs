//! Frozen legacy root: a counted unindexed row does not prove an empty group.
#![allow(clippy::expect_used, reason = "acceptance fixtures and assertions")]
#![allow(clippy::duplicate_mod, reason = "shared frozen fold oracle")]
#[path = "s3_cases.rs"]
mod cases;
use super::disk::{self, Dir, Life, Memory, MemoryJournals, Op, Read, Shown};

fn bytes(dir: &mut Memory, path: &str, cap: u64) -> Vec<u8> {
    match dir.read(path, cap) {
        Read::Bytes(bytes) => bytes,
        other => panic!("fixture document {path}: {other:?}"),
    }
}

#[test]
fn s3_prune_refuses_a_legacy_counted_root_row_over_an_empty_group() {
    let mut dir = Memory::new();
    let mut journals = MemoryJournals::new();
    let mut feed = cases::Feed::live("live/only", Some(cases::S), cases::birth(cases::S, 1));
    journals.set(feed.id(), 1, feed.ino, &feed.bytes);
    let work = cases::work(&mut dir, &mut journals, &feed.spec, &Life::default());
    let publication = cases::plan(disk::publish(&mut dir, &mut journals, &work).expect("source"));
    cases::complete(&mut dir, &publication);
    feed.active = true;
    feed.cursor = u64::try_from(feed.bytes.len()).expect("fixture length");
    cases::check(&mut dir, std::slice::from_ref(&feed));
    let view_path = format!("stats/session/{}.view", cases::S);
    let evidence_path = format!("stats/session/{}.evidence", cases::S);
    let original_view = bytes(&mut dir, &view_path, disk::SESSION_VIEW_CAP);
    let original_evidence = bytes(&mut dir, &evidence_path, disk::EVIDENCE_CAP);
    let original_root = bytes(&mut dir, "stats/view", disk::ROOT_VIEW_CAP);

    // Create a genuine empty session document through removal, stopping
    // after intent clearance but before leftover cleanup, as in amendment 7.
    let removal = cases::plan(
        disk::remove(&mut dir, Some(cases::S), feed.id(), None).expect("remove source"),
    );
    let cleanup = removal
        .ops
        .iter()
        .position(|op| matches!(op, Op::Remove(path) if path == &view_path))
        .expect("empty-view cleanup");
    removal
        .apply(&mut dir, cleanup)
        .expect("empty group after commit");
    assert!(
        disk::recovery(&mut dir)
            .expect("intent cleared")
            .ops
            .is_empty()
    );
    let mut empty_feed = feed.clone();
    empty_feed.active = false;
    cases::check(&mut dir, &[empty_feed]);

    // Restore the counted root from the earlier publication, removing only
    // its optional ownership index: sources 1, pending 0, ids absent.
    let root_text = String::from_utf8(original_root.clone()).expect("root UTF-8");
    let index = ",\"ids\":[\"live/only\"]";
    assert_eq!(root_text.matches(index).count(), 1);
    assert!(root_text.contains("\"sources\":1,\"pending\":0"));
    let legacy = root_text.replace(index, "");
    dir.plant("stats/view", legacy.as_bytes());
    assert!(
        matches!(
            disk::shown(&mut dir, None),
            Shown::Ready {
                listed: 1,
                pending: 0,
                ..
            }
        ),
        "optional ids absence is accepted input, not a parser rejection"
    );
    let empty_view = bytes(&mut dir, &view_path, disk::SESSION_VIEW_CAP);
    let empty_evidence = bytes(&mut dir, &evidence_path, disk::EVIDENCE_CAP);

    let outcome = disk::prune(&mut dir, Some(cases::S));
    assert!(
        matches!(&outcome, Err(disk::Refused::Damaged { file, .. }) if file == &view_path),
        "a listed source with an empty group requires repair, never row removal: {outcome:?}"
    );
    assert_eq!(
        bytes(&mut dir, "stats/view", disk::ROOT_VIEW_CAP),
        legacy.as_bytes(),
        "legacy counted root bytes stay exact on refusal"
    );
    assert_eq!(
        bytes(&mut dir, &view_path, disk::SESSION_VIEW_CAP),
        empty_view
    );
    assert_eq!(
        bytes(&mut dir, &evidence_path, disk::EVIDENCE_CAP),
        empty_evidence
    );

    // Restore the deliberate disagreement and verify the original full fold.
    dir.plant("stats/view", &original_root);
    dir.plant(&view_path, &original_view);
    dir.plant(&evidence_path, &original_evidence);
    cases::check(&mut dir, &[feed]);
}
