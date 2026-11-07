//! Frozen namespace independence: archive failure does not hide live disappearances.
#![allow(clippy::expect_used, reason = "acceptance fixtures and assertions")]
#![allow(clippy::duplicate_mod, reason = "shared frozen fold oracle")]
#[path = "s3_cases.rs"]
mod cases;
use super::disk::{self, Census, Files, SourceSpec};
use super::maint::{Maintainer, Outcome};
use crate::stats::SourceGap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ae-s3-namespace-completeness-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("private namespace fixture");
        Self(path)
    }

    fn live(&self, name: &str, session: &str, number: u32) -> cases::Feed {
        let id = format!("live/{name}");
        let meta = format!("session={name}\nsession_id={session}\n").into_bytes();
        let path = self.0.join("sessions").join(name);
        fs::create_dir_all(&path).expect("session fixture");
        fs::write(path.join("meta"), &meta).expect("session metadata");
        fs::write(path.join("memo.tsv"), b"").expect("memo fixture");
        let mut feed = cases::Feed::live(&id, Some(session), cases::birth(session, number));
        fs::write(path.join("events.jsonl"), &feed.bytes).expect("session journal");
        feed.spec = SourceSpec::Live {
            id,
            label: name.to_owned(),
            meta,
        };
        feed.active = true;
        feed.cursor = u64::try_from(feed.bytes.len()).expect("fixture cursor");
        feed
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn drain(maintainer: &mut Maintainer) -> usize {
    let mut removed_b = 0;
    for _ in 0..32 {
        let step = maintainer.step();
        assert!(
            !matches!(step.outcome, Outcome::Refused(_)),
            "archive failure is represented as a source gap: {step:?}"
        );
        if matches!(step.outcome, Outcome::Removed { .. })
            && step.source.as_deref() == Some("live/b")
        {
            removed_b += 1;
        }
        if step.outcome == Outcome::Idle {
            return removed_b;
        }
    }
    panic!("namespace fixture never settled");
}

#[test]
fn s3_complete_live_listing_removes_vanished_source_while_archive_enumeration_fails() {
    let root = Root::new();
    let first = root.live("a", cases::S, 1);
    let mut second = root.live("b", cases::T, 2);
    fs::write(root.0.join("archive"), b"not a directory").expect("archive enumeration fails");
    let mut failed = cases::Feed::live("archive", None, Vec::new());
    failed.spec = SourceSpec::Failed {
        id: "archive".to_owned(),
        label: "archive".to_owned(),
        session: None,
        gap: SourceGap::Unreadable,
    };
    failed.active = true;
    let mut maintainer = Maintainer::new(&root.0, cases::LIMITS, Duration::from_millis(100));
    drain(&mut maintainer);
    cases::check(
        &mut Files::at(&root.0),
        &[first.clone(), second.clone(), failed.clone()],
    );

    // The sessions directory still enumerates completely. Only b vanished;
    // the independent archive namespace remains failed throughout.
    fs::rename(root.0.join("sessions/b"), root.0.join("saved-b"))
        .expect("b vanishes from complete live namespace");
    let removed = drain(&mut maintainer);
    second.active = false;
    cases::check(&mut Files::at(&root.0), &[first, second, failed]);
    assert_eq!(
        removed, 1,
        "complete live listing schedules b's removal even while archive listing fails"
    );
    let census =
        disk::census(&mut Files::at(&root.0), &Census::default()).expect("remaining source census");
    assert!(census.sources.contains_key("live/a"));
    assert!(census.sources.contains_key("archive"));
    assert!(!census.sources.contains_key("live/b"));
}
