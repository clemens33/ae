//! Frozen regression: a validation whose tree changes inside the sandwich proves no counted archive.
#![allow(clippy::expect_used, reason = "acceptance fixture and assertions")]
use super::disk::{self, Files, Limits, Shown};
use super::maint::{Maintainer, Outcome};
use crate::stats::{self, Source, SourceGap, ThisSession};
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
const S: &str = "11111111-1111-4111-8111-111111111111";
struct Root(PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn write(path: &std::path::Path, bytes: &[u8]) {
    fs::write(path, bytes).expect("fixture file");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("file mode");
}
fn root() -> Root {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = Root(std::env::temp_dir().join(format!(
        "ae-s3-sandwich-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    let archive = root.0.join("archive").join(S);
    fs::create_dir_all(archive.join("messages")).expect("archive fixture");
    for path in [
        root.0.join("archive"),
        archive.clone(),
        archive.join("messages"),
    ] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).expect("directory mode");
    }
    write(&archive.join("messages/one.txt"), b"kept message");
    write(&archive.join("meta"), format!("archive_id={S}\narchive_version=1\nsource_session=alpha\nsource_session_id={S}\nhandover_count=0\nmemo_topic_count=0\npending_request_count=0\n").as_bytes());
    write(
        &archive.join("digest.md"),
        format!(
            "- Archive ID: {S}\n## Handover (0)\n## Memo topics (0)\n## Unresolved requests (0)\n"
        )
        .as_bytes(),
    );
    write(&archive.join("memo.tsv"), b"");
    write(&archive.join("events.jsonl"), format!("{{\"ts\":\"2026-10-01T10:00:00Z\",\"actor\":\"lead\",\"action\":\"worker-birth\",\"target\":\"scout\",\"target_slot\":\"spawned.0\",\"target_session_id\":\"{S}\",\"birth_v\":1,\"birth\":\"aaaaaaaa-aaaa-4aaa-8aaa-000000000001\",\"birth_tool\":\"codex\",\"birth_profile\":\"preset\",\"birth_model\":\"sol\",\"birth_effort\":\"high\",\"summary\":\"S3\"}}\n").as_bytes());
    crate::archive::validated_journal(&archive, S, &mut |p| fs::read(p))
        .expect("initially valid archive");
    root
}
fn check(root: &Root, valid: bool) {
    for scope in [None, Some(S)] {
        let source = if valid {
            Source::archive(
                S,
                Ok(
                    fs::read(root.0.join("archive").join(S).join("events.jsonl"))
                        .expect("fixed journal"),
                ),
            )
        } else {
            Source::failed(S, Some(S), SourceGap::Invalid)
        };
        let expected = stats::fold(vec![source], scope);
        let Shown::Ready { view, .. } = disk::shown(&mut Files::at(&root.0), scope) else {
            panic!("materialized validation verdict");
        };
        assert_eq!(
            view, expected,
            "no counted evidence from an unproved sandwich"
        );
        assert!(view.all.balanced());
        if let ThisSession::Ready(this) = view.this {
            assert!(this.balanced());
        }
    }
}
#[test]
fn s3_changed_tree_between_validation_and_identity_cannot_count() {
    let root = root();
    let archive = root.0.join("archive").join(S);
    let before = fs::read(archive.join("events.jsonl")).expect("fixed journal");
    let mut maintainer = Maintainer::new(
        &root.0,
        Limits {
            chunk: 4096,
            line: 4096,
        },
        Duration::from_millis(100),
    );
    let mut changed = false;
    maintainer.on_validated(move |path| {
        if !changed {
            fs::set_permissions(
                path.join("messages/one.txt"),
                fs::Permissions::from_mode(0o700),
            )
            .expect("invalidate after validation before second identity walk");
            changed = true;
        }
    });
    let work = maintainer
        .prepare()
        .expect("unproved validation produces an explicit Invalid source");
    let step = maintainer.commit(work);
    assert!(
        matches!(step.outcome, Outcome::Published { .. }),
        "invalid source verdict committed: {step:?}"
    );
    assert!(crate::archive::validated_journal(&archive, S, &mut |p| fs::read(p)).is_err());
    assert_eq!(
        fs::read(archive.join("events.jsonl")).expect("journal"),
        before
    );
    check(&root, false);
    fs::set_permissions(
        archive.join("messages/one.txt"),
        fs::Permissions::from_mode(0o600),
    )
    .expect("restore valid node");
    for _ in 0..32 {
        if matches!(maintainer.step().outcome, Outcome::Idle) {
            break;
        }
    }
    check(&root, true);
}
