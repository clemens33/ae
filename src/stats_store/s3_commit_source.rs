//! Frozen split API: another preparation never supplies this work's bookkeeping.
#![allow(clippy::expect_used, reason = "acceptance fixtures and assertions")]
#![allow(clippy::duplicate_mod, reason = "shared frozen fold oracle")]
#[path = "s3_cases.rs"]
mod cases;
use super::disk::{Files, SourceSpec};
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
            "ae-s3-commit-source-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("private split API fixture");
        Self(path)
    }

    fn live(&self, name: &str, session: &str, bytes: Vec<u8>) -> cases::Feed {
        let id = format!("live/{name}");
        let meta = format!("session={name}\nsession_id={session}\n").into_bytes();
        let path = self.0.join("sessions").join(name);
        fs::create_dir_all(&path).expect("session fixture");
        fs::write(path.join("meta"), &meta).expect("session metadata");
        fs::write(path.join("memo.tsv"), b"").expect("memo fixture");
        fs::write(path.join("events.jsonl"), &bytes).expect("session journal");
        let mut feed = cases::Feed::live(&id, Some(session), bytes);
        feed.spec = SourceSpec::Live {
            id,
            label: name.to_owned(),
            meta,
        };
        feed
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn s3_commit_never_attributes_a_later_sources_preparation_to_older_work() {
    let root = Root::new();
    let mut first = root.live("a", cases::S, cases::birth(cases::S, 1));
    let mut second = root.live(
        "b",
        cases::T,
        [cases::birth(cases::T, 2), cases::birth(cases::T, 3)].concat(),
    );
    let mut maintainer = Maintainer::new(&root.0, cases::LIMITS, Duration::from_millis(100));
    let older = maintainer.prepare().expect("prepare a");
    assert_eq!(older.source(), "live/a");
    let later = maintainer.prepare().expect("prepare b before committing a");
    assert_eq!(later.source(), "live/b");
    assert!(later.span().1 > 0, "b actually read journal bytes");

    let committed = maintainer.commit(older);
    assert_eq!(committed.source.as_deref(), Some("live/a"));
    assert!(
        matches!(committed.outcome, Outcome::Published { .. }),
        "older work remains publishable: {committed:?}"
    );
    assert_eq!(
        committed.read, 0,
        "a has no matching cached preparation; b's read bytes must never be attributed to it"
    );
    first.active = true;
    first.cursor = u64::try_from(first.bytes.len()).expect("a cursor");
    cases::check(&mut Files::at(&root.0), &[first.clone(), second.clone()]);

    let committed = maintainer.commit(later);
    assert_eq!(committed.source.as_deref(), Some("live/b"));
    assert!(
        matches!(committed.outcome, Outcome::Published { .. }),
        "b remains independently publishable: {committed:?}"
    );
    second.active = true;
    second.cursor = u64::try_from(second.bytes.len()).expect("b cursor");
    cases::check(&mut Files::at(&root.0), &[first.clone(), second.clone()]);
    for _ in 0..32 {
        let step = maintainer.step();
        assert!(
            !matches!(step.outcome, Outcome::Refused(_)),
            "later rounds remain healthy: {step:?}"
        );
        if step.outcome == Outcome::Idle {
            cases::check(&mut Files::at(&root.0), &[first, second]);
            return;
        }
    }
    panic!("split API fixture never settled");
}
