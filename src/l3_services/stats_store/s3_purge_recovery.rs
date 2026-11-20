//! Frozen regression: successful purge finishes removal recovery and leftover pruning.
#![allow(clippy::expect_used, reason = "contract fixtures and crash assertions")]
use super::disk::{
    self, Decision, Files, MemoryJournals, Next, Op, Plan, Prepared, Shown, SourceSpec,
};
use crate::stats::{self, ThisSession};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
const S: &str = "11111111-1111-4111-8111-111111111111";
const T: &str = "22222222-2222-4222-8222-222222222222";
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ae-s3-purge-recovery-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("private store fixture");
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn birth(uuid: &str, n: u32) -> Vec<u8> {
    format!("{{\"ts\":\"2026-10-01T10:00:00Z\",\"actor\":\"lead\",\"action\":\"worker-birth\",\"target\":\"scout\",\"target_slot\":\"spawned.0\",\"target_session_id\":\"{uuid}\",\"birth_v\":1,\"birth\":\"aaaaaaaa-aaaa-4aaa-8aaa-{n:012x}\",\"birth_tool\":\"codex\",\"birth_profile\":\"preset\",\"birth_model\":\"sol\",\"birth_effort\":\"high\",\"summary\":\"S3\"}}\n").into_bytes()
}
fn plan(decision: Decision) -> Plan {
    match decision {
        Decision::Publish(plan) => plan,
        other => panic!("publication: {other:?}"),
    }
}
fn removal(root: &Root) -> Plan {
    let mut files = Files::at(&root.0);
    let mut journals = MemoryJournals::new();
    let source = SourceSpec::Archive {
        id: format!("archive/{S}"),
        uuid: S.to_owned(),
    };
    journals.set(source.id(), 1, 33, &[birth(S, 1), birth(T, 2)].concat());
    let Next::Work(work) = Prepared::read(&mut files, &mut journals, &source, disk::LIMITS, None)
        .expect("archive registration")
    else {
        panic!("registration work");
    };
    let initial =
        plan(disk::publish(&mut files, &mut journals, &work).expect("archive publication"));
    initial
        .apply(&mut files, initial.ops.len())
        .expect("count own and foreign records");
    plan(disk::remove(&mut files, Some(S), source.id(), None).expect("removal plan"))
}
fn no_uuid(path: &Path) {
    for entry in fs::read_dir(path).expect("store tree") {
        let entry = entry.expect("store entry");
        assert!(
            !entry.file_name().to_string_lossy().contains(S),
            "successful purge leaves a UUID filename: {}",
            entry.path().display()
        );
        if entry.file_type().expect("type").is_dir() {
            no_uuid(&entry.path());
        } else {
            let bytes = fs::read(entry.path()).expect("store bytes");
            assert!(
                !bytes.windows(S.len()).any(|w| w == S.as_bytes()),
                "successful purge leaves UUID bytes: {}",
                entry.path().display()
            );
        }
    }
}
#[test]
fn s3_purge_recovers_each_committed_removal_prefix_and_prunes_uuid() {
    let probe = Root::new();
    let sample = removal(&probe);
    let committed = sample
        .ops
        .iter()
        .position(
            |op| matches!(op, Op::Rename {to, ..} if to == &format!("stats/session/{S}.evidence")),
        )
        .expect("primary commit boundary")
        + 1;
    for cut in committed..=sample.ops.len() {
        let root = Root::new();
        let removal = removal(&root);
        removal
            .apply(&mut Files::at(&root.0), cut)
            .expect("committed removal crash prefix");
        disk::purged(&root.0, S, Duration::from_millis(100))
            .expect("synchronous store-only re-purge");
        let mut files = Files::at(&root.0);
        for scope in [None, Some(S), Some(T)] {
            let Shown::Ready { view, .. } = disk::shown(&mut files, scope) else {
                panic!("completed purge answer at cut {cut}");
            };
            assert_eq!(
                view,
                stats::fold(Vec::new(), scope),
                "empty oracle at cut {cut}, scope {scope:?}"
            );
            assert!(view.all.balanced());
            if let ThisSession::Ready(this) = view.this {
                assert!(this.balanced());
            }
        }
        let Shown::Ready { view, .. } = disk::resum(&mut files).expect("independent sum") else {
            panic!("completed root sum");
        };
        assert_eq!(view.all, stats::fold(Vec::new(), None).all);
        no_uuid(&root.0.join("stats"));
        assert!(
            disk::recovery(&mut files)
                .expect("fully recovered")
                .ops
                .is_empty()
        );
    }
}
