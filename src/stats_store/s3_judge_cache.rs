//! Frozen archive validation reuse for failed trees and persisted counted fingerprints.
#![allow(clippy::expect_used, reason = "acceptance fixtures and assertions")]
#![allow(clippy::duplicate_mod, reason = "shared frozen fold oracle")]
#[path = "s3_cases.rs"]
mod cases;
use super::disk::{self, Census, Files, SourceSpec};
use super::maint::{Maintainer, Outcome, take_validations};
use crate::stats::SourceGap;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ae-s3-judge-cache-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("exclusive private cache fixture");
        Self(path)
    }

    fn archive(&self, uuid: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join("archive").join(uuid);
        fs::create_dir_all(path.join("messages")).expect("archive fixture");
        for node in [self.0.join("archive"), path.clone(), path.join("messages")] {
            fs::set_permissions(node, fs::Permissions::from_mode(0o700)).expect("directory mode");
        }
        write(&path.join("messages/one.txt"), b"kept message");
        write(&path.join("meta"), format!("archive_id={uuid}\narchive_version=1\nsource_session=alpha\nsource_session_id={uuid}\nhandover_count=0\nmemo_topic_count=0\npending_request_count=0\n").as_bytes());
        write(&path.join("digest.md"), format!("- Archive ID: {uuid}\n## Handover (0)\n## Memo topics (0)\n## Unresolved requests (0)\n").as_bytes());
        write(&path.join("memo.tsv"), b"");
        write(&path.join("events.jsonl"), bytes);
        crate::archive::validated_journal(&path, uuid, &mut |node| fs::read(node))
            .expect("independently valid archive fixture");
        path
    }

    fn maintainer(&self) -> Maintainer {
        Maintainer::new(&self.0, cases::LIMITS, Duration::from_millis(100))
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).expect("archive fixture file");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("file mode");
}

fn drain(maintainer: &mut Maintainer) {
    for _ in 0..32 {
        let step = maintainer.step();
        assert!(!matches!(step.outcome, Outcome::Refused(_)), "{step:?}");
        if step.outcome == Outcome::Idle {
            return;
        }
    }
    panic!("archive cache fixture never settled");
}

fn feed(bytes: Vec<u8>) -> cases::Feed {
    let mut feed = cases::Feed::live(&format!("archive/{}", cases::S), Some(cases::S), bytes);
    feed.spec = SourceSpec::Archive {
        id: format!("archive/{}", cases::S),
        uuid: cases::S.to_owned(),
    };
    feed.active = true;
    feed.cursor = u64::try_from(feed.bytes.len()).expect("fixture cursor");
    feed
}

#[test]
fn s3_unchanged_invalid_archive_reuses_its_in_memory_validation_verdict() {
    let root = Root::new();
    let bytes = cases::birth(cases::T, 1);
    let path = root.archive(cases::S, &bytes);
    fs::set_permissions(
        path.join("messages/one.txt"),
        fs::Permissions::from_mode(0o700),
    )
    .expect("invalid archive member mode");
    assert!(
        crate::archive::validated_journal(&path, cases::S, &mut |node| fs::read(node)).is_err()
    );
    let identity =
        crate::archive::tree_identity(&path).expect("invalid tree still has an identity");
    let mut failed = feed(bytes);
    failed.spec = SourceSpec::Failed {
        id: format!("archive/{}", cases::S),
        label: cases::S.to_owned(),
        session: Some(cases::S.to_owned()),
        gap: SourceGap::Invalid,
    };
    failed.cursor = 0;
    let mut maintainer = root.maintainer();
    let _ = take_validations();
    drain(&mut maintainer);
    assert_eq!(take_validations(), vec![cases::S.to_owned()]);
    cases::check(&mut Files::at(&root.0), &[failed.clone()]);

    drain(&mut maintainer);
    assert!(
        take_validations().is_empty(),
        "a stable Invalid verdict is reused even though no counted row stores its fingerprint"
    );
    assert_eq!(
        crate::archive::tree_identity(&path).expect("unchanged invalid tree"),
        identity
    );
    cases::check(&mut Files::at(&root.0), &[failed]);
}

#[test]
fn s3_new_maintainer_reuses_a_valid_archive_fingerprint_from_its_counted_row() {
    let root = Root::new();
    let bytes = [cases::birth(cases::S, 1), cases::birth(cases::T, 2)].concat();
    let path = root.archive(cases::S, &bytes);
    let expected = feed(bytes);
    let mut first = root.maintainer();
    let _ = take_validations();
    drain(&mut first);
    assert_eq!(take_validations(), vec![cases::S.to_owned()]);
    cases::check(&mut Files::at(&root.0), &[expected.clone()]);
    let census =
        disk::census(&mut Files::at(&root.0), &Census::default()).expect("counted archive");
    let entry = census
        .sources
        .get(&format!("archive/{}", cases::S))
        .expect("archive row");
    assert!(entry.counted);
    assert_eq!(
        entry.fp.as_deref(),
        Some(
            crate::archive::tree_identity(&path)
                .expect("valid tree identity")
                .as_str()
        )
    );
    drop(first);

    let mut next = root.maintainer();
    drain(&mut next);
    assert!(
        take_validations().is_empty(),
        "a new maintainer trusts the counted row fingerprint for this unchanged valid tree"
    );
    cases::check(&mut Files::at(&root.0), &[expected]);
}
