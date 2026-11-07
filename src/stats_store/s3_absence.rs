//! Frozen absence regressions: absent archive namespaces still purge; missing counted docs never prove empty.
#![allow(clippy::expect_used, reason = "acceptance fixtures and assertions")]
use super::disk::{
    self, Decision, Dir, Files, Memory, MemoryJournals, Next, Op, Prepared, Read, Shown, SourceSpec,
};
use crate::stats::{self, Source, ThisSession};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
const S: &str = "11111111-1111-4111-8111-111111111111";
const T: &str = "22222222-2222-4222-8222-222222222222";
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ae-s3-absence-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("private fixture root");
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn populate(dir: &mut dyn Dir) {
    let source = SourceSpec::Archive {
        id: format!("archive/{S}"),
        uuid: S.to_owned(),
    };
    let bytes = format!("{{\"ts\":\"2026-10-01T10:00:00Z\",\"actor\":\"lead\",\"action\":\"worker-birth\",\"target\":\"scout\",\"target_slot\":\"spawned.0\",\"target_session_id\":\"{S}\",\"birth_v\":1,\"birth\":\"aaaaaaaa-aaaa-4aaa-8aaa-000000000001\",\"birth_tool\":\"codex\",\"birth_profile\":\"preset\",\"birth_model\":\"sol\",\"birth_effort\":\"high\",\"summary\":\"S3\"}}\n").into_bytes();
    let mut journals = MemoryJournals::new();
    journals.set(source.id(), 1, 99, &bytes);
    let Next::Work(work) = Prepared::read(dir, &mut journals, &source, disk::LIMITS, None)
        .expect("archive preparation")
    else {
        panic!("registration work");
    };
    let Decision::Publish(plan) =
        disk::publish(dir, &mut journals, &work).expect("archive publication")
    else {
        panic!("publication plan");
    };
    plan.apply(dir, plan.ops.len())
        .expect("seed persisted counted archive");
    for scope in [None, Some(S), Some(T)] {
        let Shown::Ready { view, .. } = disk::shown(dir, scope) else {
            panic!("initial counted answer");
        };
        assert_eq!(
            view,
            stats::fold(vec![Source::archive(S, Ok(bytes.clone()))], scope)
        );
        assert_eq!(view.all.sources_total, 1);
    }
}
fn no_uuid(path: &Path) {
    for entry in fs::read_dir(path).expect("store directory") {
        let entry = entry.expect("entry");
        assert!(
            !entry.file_name().to_string_lossy().contains(S),
            "UUID filename remains"
        );
        if entry.file_type().expect("type").is_dir() {
            no_uuid(&entry.path());
        } else {
            let bytes = fs::read(entry.path()).expect("store bytes");
            assert!(
                !bytes.windows(S.len()).any(|window| window == S.as_bytes()),
                "UUID bytes remain"
            );
        }
    }
}
#[test]
fn s3_purge_absent_archive_namespace_still_removes_persisted_stats() {
    let root = Root::new();
    populate(&mut Files::at(&root.0));
    assert!(
        !root.0.join("archive").exists(),
        "the whole archive namespace is already gone"
    );
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = crate::archive::purge::run(
        &root.0.join("sessions/alpha"),
        S,
        "alpha",
        "",
        &mut out,
        &mut err,
    )
    .expect("same purge command");
    assert_eq!(
        code,
        0,
        "store-only purge: {}",
        String::from_utf8_lossy(&err)
    );
    let mut files = Files::at(&root.0);
    for scope in [None, Some(S), Some(T)] {
        let Shown::Ready { view, .. } = disk::shown(&mut files, scope) else {
            panic!("synchronously cleared store");
        };
        assert_eq!(
            view,
            stats::fold(Vec::new(), scope),
            "absence of archive root does not bypass store purge"
        );
        assert!(view.all.balanced());
        if let ThisSession::Ready(this) = view.this {
            assert!(this.balanced());
        }
    }
    let Shown::Ready { view, .. } = disk::resum(&mut files).expect("independent root sum") else {
        panic!("ready sum");
    };
    assert_eq!(view.all, stats::fold(Vec::new(), None).all);
    no_uuid(&root.0.join("stats"));
}
#[test]
fn s3_prune_missing_counted_documents_refuses_without_rewriting_root() {
    let mut dir = Memory::new();
    populate(&mut dir);
    let before = dir.read("stats/view", disk::ROOT_VIEW_CAP);
    assert!(matches!(before, Read::Bytes(_)));
    dir.apply(&Op::Remove(format!("stats/session/{S}.view")))
        .expect("lost session view");
    dir.apply(&Op::Remove(format!("stats/session/{S}.evidence")))
        .expect("lost primary evidence");
    let outcome = disk::prune(&mut dir, Some(S));
    assert!(
        matches!(outcome, Err(disk::Refused::Damaged { .. })),
        "a root with a counted source is not an empty leftover group: {outcome:?}"
    );
    assert_eq!(
        dir.read("stats/view", disk::ROOT_VIEW_CAP),
        before,
        "refusal preserves the indexed counts and ownership for repair"
    );
}
