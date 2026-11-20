//! Frozen move reporting: a busy move removes its old owner exactly once.
#![allow(clippy::expect_used, reason = "acceptance fixtures and assertions")]
#![allow(clippy::duplicate_mod, reason = "shared frozen fold oracle")]
#[path = "s3_cases.rs"]
mod cases;
use super::disk::{Files, Payload, SourceSpec};
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
            "ae-s3-move-busy-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("private move fixture");
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
            "move upkeep remains healthy: {step:?}"
        );
        if step.outcome == Outcome::Idle {
            return;
        }
    }
    panic!("move fixture never settled");
}

#[test]
fn s3_busy_uuid_move_reports_exactly_one_removal_before_new_owner_settles() {
    let root = Root::new();
    let old_meta = format!("session=a\nsession_id={}\n", cases::S).into_bytes();
    let path = root.0.join("sessions/a");
    fs::create_dir_all(&path).expect("live source fixture");
    fs::write(path.join("meta"), &old_meta).expect("old source group");
    fs::write(path.join("memo.tsv"), b"").expect("memo fixture");
    let mut feed = cases::Feed::live("live/a", Some(cases::S), cases::birth(cases::S, 1));
    fs::write(path.join("events.jsonl"), &feed.bytes).expect("journal fixture");
    feed.spec = SourceSpec::Live {
        id: "live/a".to_owned(),
        label: "a".to_owned(),
        meta: old_meta,
    };
    feed.active = true;
    feed.cursor = u64::try_from(feed.bytes.len()).expect("complete fixture cursor");
    let mut maintainer = Maintainer::new(&root.0, cases::LIMITS, Duration::ZERO);
    drain(&mut maintainer);
    cases::check(&mut Files::at(&root.0), std::slice::from_ref(&feed));

    let new_meta = format!("session=a\nsession_id={}\n", cases::T).into_bytes();
    fs::write(path.join("meta"), &new_meta).expect("same live source moves S to T");
    let payload = Payload::at(&root.0);
    let lock = payload
        .lock(Duration::ZERO)
        .expect("another writer holds stats lock");
    let deferred = maintainer.step();
    assert_eq!(deferred.source.as_deref(), Some("live/a"));
    assert_eq!(deferred.outcome, Outcome::Deferred);
    cases::check(&mut Files::at(&root.0), std::slice::from_ref(&feed));
    drop(lock);

    let removed = maintainer.step();
    assert_eq!(removed.source.as_deref(), Some("live/a"));
    assert!(
        matches!(removed.outcome, Outcome::Removed { .. }),
        "old owner removed first: {removed:?}"
    );
    feed.active = false;
    cases::check(&mut Files::at(&root.0), std::slice::from_ref(&feed));
    feed.spec = SourceSpec::Live {
        id: "live/a".to_owned(),
        label: "a".to_owned(),
        meta: new_meta,
    };
    let mut removals = 1;
    let mut published = false;
    for _ in 0..32 {
        let step = maintainer.step();
        assert!(
            !matches!(step.outcome, Outcome::Refused(_)),
            "move retry remains healthy: {step:?}"
        );
        if matches!(step.outcome, Outcome::Removed { .. })
            && step.source.as_deref() == Some("live/a")
        {
            removals += 1;
        }
        if matches!(step.outcome, Outcome::Published { .. }) {
            assert_eq!(step.source.as_deref(), Some("live/a"));
            published = true;
            feed.active = true;
        }
        cases::check(&mut Files::at(&root.0), std::slice::from_ref(&feed));
        if step.outcome == Outcome::Idle {
            assert!(published, "new owner actually published");
            assert_eq!(
                removals, 1,
                "one real move cannot emit a second stale Removed report"
            );
            return;
        }
    }
    panic!("busy UUID move never settled");
}
