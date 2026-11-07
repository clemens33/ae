//! Frozen additive gap absence: readable failed rows are replaced and backfilled.
#![allow(clippy::expect_used, reason = "acceptance fixtures and assertions")]
#![allow(clippy::duplicate_mod, reason = "shared frozen fold oracle")]
#[path = "s3_cases.rs"]
mod cases;
use super::disk::{self, Dir, Life, Memory, MemoryJournals, Next, Prepared, Read, SourceSpec};
use crate::stats::SourceGap;

fn snapshot(dir: &mut Memory, name: &str) -> Vec<u8> {
    match dir.read(name, disk::SESSION_VIEW_CAP) {
        Read::Bytes(bytes) => bytes,
        other => panic!("fixture view: {other:?}"),
    }
}

#[test]
fn s3_failed_row_with_matching_origin_but_absent_gap_is_replaced() {
    let mut dir = Memory::new();
    let mut journals = MemoryJournals::new();
    let mut feeds = cases::seed(&mut dir, &mut journals);
    let failed = SourceSpec::Failed {
        id: "live/a".to_owned(),
        label: "live/a".to_owned(),
        session: Some(cases::S.to_owned()),
        gap: SourceGap::Missing,
    };
    let first = cases::work(&mut dir, &mut journals, &failed, &Life::default());
    let publication =
        cases::plan(disk::publish(&mut dir, &mut journals, &first).expect("failed row"));
    cases::complete(&mut dir, &publication);
    feeds[0].spec = failed.clone();
    feeds[0].cursor = 0;
    cases::check(&mut dir, &feeds);

    // Exactly the accepted failed-gap-absent shape from frozen s3_keys mode 6:
    // retain org and every other byte, remove only this row's optional gap.
    let path = format!("stats/session/{}.view", cases::S);
    let before = String::from_utf8(snapshot(&mut dir, &path)).expect("generated UTF-8");
    let mut edited = String::new();
    let mut changed = false;
    for line in before.lines() {
        if line.starts_with("{\"row\":\"live/a\",") {
            assert!(line.contains(",\"org\":\""));
            assert!(line.contains(",\"gap\":\"missing\""));
            edited.push_str(&line.replace(",\"gap\":\"missing\"", ""));
            changed = true;
        } else {
            edited.push_str(line);
        }
        edited.push('\n');
    }
    assert!(changed, "failed row edited");
    dir.plant(&path, edited.as_bytes());
    cases::check(&mut dir, &feeds);

    let Next::Work(work) = Prepared::read(&mut dir, &mut journals, &failed, cases::LIMITS, None)
        .expect("accepted optional gap absence")
    else {
        panic!("a failed row without gap must be replaced even when its origin hash matches");
    };
    let publication =
        cases::plan(disk::publish(&mut dir, &mut journals, &work).expect("backfill gap"));
    cases::complete(&mut dir, &publication);
    cases::check(&mut dir, &feeds);
    let after = String::from_utf8(snapshot(&mut dir, &path)).expect("rendered UTF-8");
    assert!(
        after
            .lines()
            .any(|line| line.starts_with("{\"row\":\"live/a\",")
                && line.contains(",\"gap\":\"missing\"")),
        "replacement backfills the missing gap without changing the fold"
    );
}
