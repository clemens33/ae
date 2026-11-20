//! Frozen lifecycle lock pacing: a busy removal is deferred and retried.
#![allow(clippy::expect_used, reason = "acceptance fixtures and assertions")]
#![allow(clippy::duplicate_mod, reason = "shared frozen fold oracle")]
#[path = "s3_cases.rs"]
mod cases;
use super::disk::{Files, Payload, SourceSpec};
use super::maint::{self, Maintainer, Outcome};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ae-s3-chore-busy-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("private chore fixture");
        Self(path)
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
            "unlocked upkeep remains healthy: {step:?}"
        );
        if step.outcome == Outcome::Idle {
            return;
        }
    }
    panic!("busy chore fixture never settled");
}

#[test]
fn s3_vanished_removal_uses_deferred_busy_pacing_then_retries_after_unlock() {
    let root = Root::new();
    let meta = format!("session=a\nsession_id={}\n", cases::S).into_bytes();
    let path = root.0.join("sessions/a");
    fs::create_dir_all(&path).expect("live source fixture");
    fs::write(path.join("meta"), &meta).expect("session metadata");
    fs::write(path.join("memo.tsv"), b"").expect("memo fixture");
    let mut feed = cases::Feed::live("live/a", Some(cases::S), cases::birth(cases::S, 1));
    fs::write(path.join("events.jsonl"), &feed.bytes).expect("session journal");
    feed.spec = SourceSpec::Live {
        id: "live/a".to_owned(),
        label: "a".to_owned(),
        meta,
    };
    feed.active = true;
    feed.cursor = u64::try_from(feed.bytes.len()).expect("fixture cursor");
    let mut maintainer = Maintainer::new(&root.0, cases::LIMITS, Duration::ZERO);
    drain(&mut maintainer);
    cases::check(&mut Files::at(&root.0), std::slice::from_ref(&feed));
    fs::rename(&path, root.0.join("saved-a")).expect("source vanishes from complete listing");

    let payload = Payload::at(&root.0);
    let lock = payload
        .lock(Duration::ZERO)
        .expect("another writer holds the real stats lock");
    let deferred = maintainer.step();
    assert_eq!(deferred.source.as_deref(), Some("live/a"));
    assert_eq!(
        deferred.outcome,
        Outcome::Deferred,
        "busy removal is not an IO failure"
    );
    assert_eq!(deferred.read, 0);
    assert_eq!(
        maint::pause(&deferred),
        deferred.active.max(Duration::from_millis(250)),
        "a busy chore uses the busy retry pace"
    );
    cases::check(&mut Files::at(&root.0), std::slice::from_ref(&feed));
    drop(lock);

    let removed = maintainer.step();
    assert_eq!(removed.source.as_deref(), Some("live/a"));
    assert!(
        matches!(removed.outcome, Outcome::Removed { .. }),
        "deferred chore remains queued: {removed:?}"
    );
    feed.active = false;
    cases::check(&mut Files::at(&root.0), std::slice::from_ref(&feed));
    drain(&mut maintainer);
    cases::check(&mut Files::at(&root.0), &[feed]);
}
