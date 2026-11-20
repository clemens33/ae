//! Frozen donor eligibility: listed live siblings cannot supply each other's history.
#![allow(clippy::expect_used, reason = "acceptance fixtures and assertions")]
#![allow(clippy::duplicate_mod, reason = "shared frozen fold oracle")]
#[path = "s3_cases.rs"]
mod cases;
use super::disk::{self, Census, Files, SourceSpec};
use super::maint::{Maintainer, Outcome};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ae-s3-listed-donors-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("private donor fixture");
        Self(path)
    }

    fn live(&self, name: &str, bytes: Vec<u8>) -> cases::Feed {
        let id = format!("live/{name}");
        let meta = format!("session={name}\nsession_id={}\n", cases::S).into_bytes();
        let path = self.0.join("sessions").join(name);
        fs::create_dir_all(&path).expect("live session fixture");
        fs::write(path.join("meta"), &meta).expect("session metadata");
        fs::write(path.join("memo.tsv"), b"").expect("memo fixture");
        fs::write(path.join("events.jsonl"), &bytes).expect("journal bytes");
        let mut feed = cases::Feed::live(&id, Some(cases::S), bytes);
        feed.spec = SourceSpec::Live {
            id,
            label: name.to_owned(),
            meta,
        };
        feed.active = true;
        feed.cursor = u64::try_from(feed.bytes.len()).expect("complete fixture cursor");
        feed
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn drain(maintainer: &mut Maintainer) {
    for _ in 0..32 {
        let step = maintainer.step();
        assert!(
            !matches!(step.outcome, Outcome::Refused(_)),
            "donor fixture upkeep remains healthy: {step:?}"
        );
        if step.outcome == Outcome::Idle {
            return;
        }
    }
    panic!("listed donor fixture never settled");
}

#[test]
fn s3_listed_live_siblings_with_matching_witnesses_keep_distinct_early_evidence() {
    let root = Root::new();
    let common = b"{\"ts\":\"2026-10-01T10:00:00Z\",\"actor\":\"lead\",\"action\":\"memo\",\"summary\":\"padding\"}\n".repeat(128);
    assert!(
        common.len() > 4096,
        "common suffix exceeds the witness window"
    );
    let first_birth = cases::birth(cases::T, 1);
    let second_birth = cases::birth(cases::S, 2);
    assert_eq!(first_birth.len(), second_birth.len());
    assert_ne!(first_birth, second_birth);
    let first = root.live("a", [first_birth, common.clone()].concat());
    let mut maintainer = Maintainer::new(&root.0, cases::LIMITS, Duration::from_millis(100));
    drain(&mut maintainer);
    cases::check(&mut Files::at(&root.0), std::slice::from_ref(&first));

    // Keep a listed while adding another live source with the same group,
    // equal length and witness suffix, but a different early certificate.
    let second = root.live("c", [second_birth, common].concat());
    assert_eq!(first.bytes.len(), second.bytes.len());
    let start = first.bytes.len() - 4096;
    assert_eq!(&first.bytes[start..], &second.bytes[start..]);
    drain(&mut maintainer);
    cases::check(&mut Files::at(&root.0), &[first, second]);
    let census =
        disk::census(&mut Files::at(&root.0), &Census::default()).expect("two listed siblings");
    assert_eq!(census.sources.len(), 2);
    for id in ["live/a", "live/c"] {
        let entry = census
            .sources
            .get(id)
            .expect("listed source remains counted");
        assert!(entry.counted);
        assert_eq!(entry.key.as_deref(), Some(cases::S));
    }
}
