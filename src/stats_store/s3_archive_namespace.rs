//! Frozen lifecycle: a healed archive namespace removes its failed placeholder.
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
            "ae-s3-archive-namespace-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("private namespace fixture");
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn drain(maintainer: &mut Maintainer) -> bool {
    let mut removed = false;
    for _ in 0..32 {
        let step = maintainer.step();
        assert!(
            !matches!(step.outcome, Outcome::Refused(_)),
            "namespace upkeep must remain healthy: {step:?}"
        );
        removed |= matches!(step.outcome, Outcome::Removed { .. });
        if step.outcome == Outcome::Idle {
            return removed;
        }
    }
    panic!("namespace fixture never settled");
}

#[test]
fn s3_healed_archive_namespace_removes_its_failed_source_placeholder() {
    let root = Root::new();
    fs::create_dir(root.0.join("sessions")).expect("complete empty live namespace");
    fs::write(root.0.join("archive"), b"not a directory").expect("archive enumeration fails");
    let mut maintainer = Maintainer::new(&root.0, cases::LIMITS, Duration::from_millis(100));
    drain(&mut maintainer);
    let mut failed = cases::Feed::live("archive", None, Vec::new());
    failed.spec = SourceSpec::Failed {
        id: "archive".to_owned(),
        label: "archive".to_owned(),
        session: None,
        gap: SourceGap::Unreadable,
    };
    failed.active = true;
    cases::check(&mut Files::at(&root.0), std::slice::from_ref(&failed));
    let before =
        disk::census(&mut Files::at(&root.0), &Census::default()).expect("failed namespace census");
    assert!(
        before.sources.contains_key("archive"),
        "failure placeholder actually published"
    );

    // A complete successful listing now proves the placeholder vanished.
    fs::remove_file(root.0.join("archive")).expect("remove fixture obstruction");
    fs::create_dir(root.0.join("archive")).expect("healed empty archive namespace");
    assert!(
        drain(&mut maintainer),
        "upkeep removes the healed namespace placeholder"
    );
    failed.active = false;
    cases::check(&mut Files::at(&root.0), &[failed]);
    let after = disk::census(&mut Files::at(&root.0), &Census::default()).expect("healed census");
    assert!(
        !after.sources.contains_key("archive"),
        "no stale failed archive row after healing"
    );
}
