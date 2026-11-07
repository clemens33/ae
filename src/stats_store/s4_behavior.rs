//! S4 baseline-compatible behavior acceptance. Frozen S4 v1; navigator revision and lead gate required for changes.
//! Oracle: ruling 19 and the original stats::fold, not the writer implementation.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "private fixture IO"
)]

use super::disk::{self, Files, Limits, Shown};
use super::maint::{Maintainer, Outcome};
use crate::stats::{self, Source, ThisSession};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub(super) const S: &str = "11111111-1111-4111-8111-111111111111";
pub(super) const T: &str = "22222222-2222-4222-8222-222222222222";

pub(super) struct Fixture(pub(super) PathBuf);

impl Fixture {
    pub(super) fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "ae-s4-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("exclusive private fixture root");
        Self(root)
    }

    pub(super) fn live(&self, name: &str, uuid: &str, bytes: &[u8]) -> PathBuf {
        let dir = self.0.join("sessions").join(name);
        fs::create_dir_all(&dir).expect("fixture session");
        fs::write(
            dir.join("meta"),
            format!("session={name}\nsession_id={uuid}\n"),
        )
        .expect("fixture meta");
        fs::write(dir.join("events.jsonl"), bytes).expect("fixture journal");
        dir
    }

    pub(super) fn maintainer(&self) -> Maintainer {
        Maintainer::new(
            &self.0,
            Limits {
                chunk: 4 << 20,
                line: 4 << 20,
            },
            Duration::from_millis(100),
        )
    }

    pub(super) fn archive(&self, uuid: &str, bytes: &[u8]) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let archive = self.0.join("archive");
        let path = archive.join(uuid);
        fs::create_dir_all(path.join("messages")).expect("archive fixture directories");
        for dir in [&archive, &path, &path.join("messages")] {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).expect("private directory");
        }
        let meta = format!(
            "archive_id={uuid}\narchive_version=1\nsource_session=alpha\nsource_session_id={uuid}\nhandover_count=0\nmemo_topic_count=0\npending_request_count=0\n"
        );
        let digest = format!(
            "- Archive ID: {uuid}\n## Handover (0)\n## Memo topics (0)\n## Unresolved requests (0)\n"
        );
        for (name, bytes) in [
            ("meta", meta.as_bytes()),
            ("digest.md", digest.as_bytes()),
            ("memo.tsv", b"".as_slice()),
            ("events.jsonl", bytes),
        ] {
            fs::write(path.join(name), bytes).expect("archive fixture file");
            fs::set_permissions(path.join(name), fs::Permissions::from_mode(0o600))
                .expect("private file");
        }
        crate::archive::validated_journal(&path, uuid, &mut |node| fs::read(node))
            .expect("independently validated archive fixture");
        path
    }

    pub(super) fn check(&self, sources: impl Fn() -> Vec<Source>) {
        for scope in [
            None,
            Some(S),
            Some(T),
            Some("33333333-3333-4333-8333-333333333333"),
        ] {
            let expected = stats::fold(sources(), scope);
            let Shown::Ready {
                view,
                pending,
                updating,
                ..
            } = disk::shown(&mut Files::at(&self.0), scope)
            else {
                panic!("caught-up store must answer scope {scope:?}");
            };
            assert!(!updating, "publication finished");
            assert_eq!(pending, 0, "fixture caught up");
            assert_eq!(view, expected, "every fold field at scope {scope:?}");
            assert!(view.all.balanced());
            if let ThisSession::Ready(this) = view.this {
                assert!(this.balanced());
            }
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn birth(uuid: &str, n: u32) -> Vec<u8> {
    format!("{{\"ts\":\"2026-10-01T10:00:00Z\",\"actor\":\"lead\",\"action\":\"worker-birth\",\"target\":\"scout\",\"target_slot\":\"spawned.0\",\"target_session_id\":\"{uuid}\",\"birth_v\":1,\"birth\":\"aaaaaaaa-aaaa-4aaa-8aaa-{n:012x}\",\"birth_tool\":\"codex\",\"birth_profile\":\"preset\",\"birth_model\":\"sol\",\"birth_effort\":\"high\"}}\n").into_bytes()
}

pub(super) fn source(dir: &Path, name: &str) -> Source {
    Source::live(
        name,
        &fs::read(dir.join("meta")).expect("oracle meta"),
        Ok(fs::read(dir.join("events.jsonl")).expect("oracle journal")),
    )
}

pub(super) fn drain(maintainer: &mut Maintainer) {
    for _ in 0..256 {
        let step = maintainer.step();
        assert!(!matches!(step.outcome, Outcome::Refused(_)), "{step:?}");
        if step.outcome == Outcome::Idle {
            return;
        }
    }
    panic!("bounded fixture did not catch up");
}

#[test]
fn s4_first_registration_reads_no_journal_content_under_root_lock() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, &birth(S, 1));
    let mut maintainer = root.maintainer();
    let work = maintainer
        .prepare()
        .expect("initial registration prepared unlocked");
    let _ = disk::take_journal_reads();
    let step = maintainer.commit(work);
    assert!(
        matches!(step.outcome, Outcome::Published { .. }),
        "{step:?}"
    );
    let reads = disk::take_journal_reads();
    assert_eq!(
        reads.under_lock, 0,
        "ruling 19 moves witness content opens outside stats.lock"
    );
    drain(&mut maintainer);
    root.check(|| vec![source(&dir, "alpha")]);
}

#[test]
fn s4_incremental_commit_reads_no_journal_content_under_root_lock() {
    let root = Fixture::new();
    let old = birth(S, 1);
    let dir = root.live("alpha", S, &old);
    let mut maintainer = root.maintainer();
    drain(&mut maintainer);
    fs::write(dir.join("events.jsonl"), [old, birth(S, 2)].concat())
        .expect("fixture append retaining prefix");
    let work = maintainer
        .prepare()
        .expect("incremental work prepared unlocked");
    let _ = disk::take_journal_reads();
    let step = maintainer.commit(work);
    assert!(
        matches!(step.outcome, Outcome::Published { .. }),
        "{step:?}"
    );
    let reads = disk::take_journal_reads();
    assert_eq!(
        reads.under_lock, 0,
        "ruling 19 moves witness content opens outside stats.lock"
    );
    drain(&mut maintainer);
    root.check(|| vec![source(&dir, "alpha")]);
}

#[test]
fn s4_readable_same_length_rewrite_after_preparation_is_not_published() {
    let root = Fixture::new();
    let dir = root.live("alpha", S, &birth(S, 1));
    let mut maintainer = root.maintainer();
    let work = maintainer.prepare().expect("initial work");
    let old = fs::read(dir.join("events.jsonl")).expect("old bytes");
    let replacement = birth(S, 2);
    assert_eq!(
        old.len(),
        replacement.len(),
        "opposed readable value at identical length"
    );
    fs::write(dir.join("events.jsonl"), replacement).expect("same inode rewrite");
    let step = maintainer.commit(work);
    assert!(
        matches!(step.outcome, Outcome::Discarded(_)),
        "stale read must discard: {step:?}"
    );
    drain(&mut maintainer);
    root.check(|| vec![source(&dir, "alpha")]);
}
