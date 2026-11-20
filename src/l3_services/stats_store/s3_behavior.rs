//! Frozen S3 runtime acceptance. Oracle: the requirements and the original stats fold.
//! This module intentionally compiles against the S2 API at d4d30154.
#![allow(clippy::expect_used, reason = "acceptance assertions and fixture IO")]
#![allow(clippy::too_many_lines, reason = "explicit lifecycle interleavings")]

use super::disk::{self, Files, Limits, Payload, Shown};
use super::maint::{Maintainer, Outcome};
use crate::stats::{self, Source, SourceGap, ThisSession};
use std::fs;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const S: &str = "11111111-1111-4111-8111-111111111111";
const T: &str = "22222222-2222-4222-8222-222222222222";

pub(super) struct Root(pub(super) PathBuf);
impl Root {
    pub(super) fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ae-s3-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("exclusive private fixture root");
        Self(path)
    }

    fn live(&self, name: &str, uuid: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join("sessions").join(name);
        fs::create_dir_all(&path).expect("live fixture");
        put(&path.join("meta"), meta(name, uuid).as_bytes());
        put(&path.join("memo.tsv"), b"");
        put(&path.join("events.jsonl"), bytes);
        path
    }

    pub(super) fn archive(&self, uuid: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join("archive").join(uuid);
        directory(&path.join("messages"));
        directory(&path.join("retained/nested"));
        put(&path.join("retained/nested/state"), b"kept");
        put(&path.join("messages/one.txt"), b"private body");
        put(&path.join("meta"), archive_meta(uuid).as_bytes());
        put(&path.join("digest.md"), digest(uuid).as_bytes());
        put(&path.join("memo.tsv"), b"");
        put(&path.join("events.jsonl"), bytes);
        assert_valid(&path, uuid);
        path
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
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn directory(path: &Path) {
    fs::create_dir_all(path).expect("fixture directories");
    let mut next = Some(path);
    while let Some(node) = next {
        fs::set_permissions(node, fs::Permissions::from_mode(0o700)).expect("directory mode");
        if node.file_name().is_some_and(|name| name == "archive") {
            break;
        }
        next = node.parent();
    }
}

fn put(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).expect("fixture file");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("file mode");
}

fn meta(name: &str, uuid: &str) -> String {
    format!("session={name}\nsession_id={uuid}\n")
}

fn archive_meta(uuid: &str) -> String {
    format!(
        "archive_id={uuid}\narchive_version=1\nsource_session=alpha\nsource_session_id={uuid}\nhandover_count=0\nmemo_topic_count=0\npending_request_count=0\n"
    )
}

fn digest(uuid: &str) -> String {
    format!(
        "- Archive ID: {uuid}\n## Handover (0)\n## Memo topics (0)\n## Unresolved requests (0)\n"
    )
}

pub(super) fn assert_valid(path: &Path, uuid: &str) {
    crate::archive::validated_journal(path, uuid, &mut |node| fs::read(node))
        .expect("the fixture is independently a valid archive");
}

fn birth(uuid: &str, number: u32) -> Vec<u8> {
    format!("{{\"ts\":\"2026-10-01T10:00:00Z\",\"actor\":\"lead\",\"action\":\"worker-birth\",\"target\":\"scout\",\"target_slot\":\"spawned.0\",\"target_session_id\":\"{uuid}\",\"birth_v\":1,\"birth\":\"aaaaaaaa-aaaa-4aaa-8aaa-{number:012x}\",\"birth_tool\":\"codex\",\"birth_profile\":\"preset\",\"birth_model\":\"sol\",\"birth_effort\":\"high\",\"summary\":\"PRIVATE-S3\"}}\n").into_bytes()
}

fn live(root: &Root, name: &str) -> Source {
    let path = root.0.join("sessions").join(name);
    Source::live(
        name,
        &fs::read(path.join("meta")).expect("expected live meta"),
        fs::read(path.join("events.jsonl")).map_err(|_| SourceGap::Missing),
    )
}

fn archive(root: &Root, uuid: &str) -> Source {
    Source::archive(
        uuid,
        Ok(
            fs::read(root.0.join("archive").join(uuid).join("events.jsonl"))
                .expect("expected archive journal"),
        ),
    )
}

fn failed(uuid: &str, gap: SourceGap) -> Source {
    Source::failed(uuid, Some(uuid), gap)
}

pub(super) fn drain(maintainer: &mut Maintainer) {
    for _ in 0..256 {
        let step = maintainer.step();
        assert!(
            !matches!(step.outcome, Outcome::Refused(_)),
            "unexpected refusal: {step:?}"
        );
        if matches!(step.outcome, Outcome::Idle) {
            return;
        }
    }
    panic!("bounded fixture did not catch up");
}

fn check(root: &Root, expected: impl Fn() -> Vec<Source>) {
    let mut files = Files::at(&root.0);
    for scope in [None, Some(S), Some(T)] {
        let oracle = stats::fold(expected(), scope);
        let Shown::Ready {
            view,
            updating,
            pending,
            ..
        } = disk::shown(&mut files, scope)
        else {
            panic!("caught-up store must answer scope {scope:?}");
        };
        assert!(!updating, "completed publication");
        assert_eq!(pending, 0, "complete fixture was consumed");
        assert_eq!(view, oracle, "every field in scope {scope:?}");
        assert!(view.all.balanced());
        if let ThisSession::Ready(this) = &view.this {
            assert!(this.balanced());
        }
    }
    let Shown::Ready { view: sum, .. } = disk::resum(&mut files).expect("independent session sum")
    else {
        panic!("session sum must be ready");
    };
    assert_eq!(sum.all, stats::fold(expected(), None).all);
}

fn foreign() -> Vec<u8> {
    [birth(S, 1), birth(T, 2)].concat()
}

#[test]
fn s3_vanished_live_removes_own_and_foreign_fragments() {
    let root = Root::new();
    let path = root.live("alpha", S, &foreign());
    let mut maintainer = root.maintainer();
    drain(&mut maintainer);
    check(&root, || vec![live(&root, "alpha")]);
    fs::remove_dir_all(path).expect("remove live source");
    drain(&mut maintainer);
    check(&root, Vec::new);
}

#[test]
fn s3_vanished_archive_removes_own_and_foreign_fragments() {
    let root = Root::new();
    let path = root.archive(S, &foreign());
    let mut maintainer = root.maintainer();
    drain(&mut maintainer);
    check(&root, || vec![archive(&root, S)]);
    fs::remove_dir_all(path).expect("remove archive source");
    drain(&mut maintainer);
    check(&root, Vec::new);
}

#[test]
fn s3_live_rename_retains_exact_sources_and_scope() {
    let root = Root::new();
    let path = root.live("alpha", S, &foreign());
    let mut maintainer = root.maintainer();
    drain(&mut maintainer);
    fs::rename(path, root.0.join("sessions/beta")).expect("rename source");
    drain(&mut maintainer);
    check(&root, || vec![live(&root, "beta")]);
}

#[test]
fn s3_live_uuid_change_never_counts_the_source_twice() {
    let root = Root::new();
    let bytes = [
        foreign(),
        b"{\"ts\":\"2025-01-01T00:00:00Z\",\"action\":\"spawn\",\"target\":\"legacy\"}\n".to_vec(),
    ]
    .concat();
    let path = root.live("alpha", S, &bytes);
    let mut maintainer = root.maintainer();
    drain(&mut maintainer);
    put(&path.join("meta"), meta("alpha", T).as_bytes());
    for _ in 0..64 {
        let step = maintainer.step();
        if let Shown::Ready { view, .. } = disk::shown(&mut Files::at(&root.0), None) {
            assert!(
                view.all.sources_total <= 1,
                "a source has only one owner at every commit"
            );
        }
        if matches!(step.outcome, Outcome::Idle) {
            break;
        }
    }
    drain(&mut maintainer);
    check(&root, || vec![live(&root, "alpha")]);
}

#[test]
fn s3_quiet_live_roster_change_recounts_without_journal_reads() {
    let root = Root::new();
    let path = root.live("alpha", S, &birth(S, 1));
    let mut maintainer = root.maintainer();
    drain(&mut maintainer);
    put(
        &path.join("meta"),
        format!("{}seat.spawned.9=unborn\n", meta("alpha", S)).as_bytes(),
    );
    let _ = disk::take_journal_reads();
    drain(&mut maintainer);
    assert_eq!(
        disk::take_journal_reads().total,
        0,
        "a roster edit must not replay the journal"
    );
    check(&root, || vec![live(&root, "alpha")]);
}

#[test]
fn s3_stale_first_registration_cannot_create_a_second_uuid_owner() {
    let root = Root::new();
    let path = root.live("alpha", S, &foreign());
    let mut old = root.maintainer();
    let work = old.prepare().expect("first registration prepared under S");
    put(&path.join("meta"), meta("alpha", T).as_bytes());
    let mut newer = root.maintainer();
    drain(&mut newer);
    check(&root, || vec![live(&root, "alpha")]);
    let step = old.commit(work);
    assert!(
        !matches!(step.outcome, Outcome::Published { .. }),
        "stale absent-stamp ownership must be refused: {step:?}"
    );
    check(&root, || vec![live(&root, "alpha")]);
}

#[test]
fn s3_stale_failed_registration_cannot_create_a_second_uuid_owner() {
    let root = Root::new();
    let path = root.live("alpha", S, b"");
    fs::remove_file(path.join("events.jsonl")).expect("missing journal");
    let mut old = root.maintainer();
    let work = old
        .prepare()
        .expect("first failed registration has no witness");
    put(&path.join("meta"), meta("alpha", T).as_bytes());
    let mut newer = root.maintainer();
    drain(&mut newer);
    check(&root, || vec![live(&root, "alpha")]);
    let step = old.commit(work);
    assert!(
        !matches!(step.outcome, Outcome::Published { .. }),
        "a witness-free registration still needs exclusive ownership: {step:?}"
    );
    check(&root, || vec![live(&root, "alpha")]);
}

#[test]
fn s3_claim_flip_subtracts_and_restores_archive() {
    let root = Root::new();
    root.archive(S, &foreign());
    let mut maintainer = root.maintainer();
    drain(&mut maintainer);
    check(&root, || vec![archive(&root, S)]);
    let claim = root.0.join("archive").join(format!(".publishing.{S}"));
    fs::create_dir(&claim).expect("archive claim");
    drain(&mut maintainer);
    check(&root, || vec![failed(S, SourceGap::Claimed)]);
    fs::remove_dir(claim).expect("claim cleared");
    drain(&mut maintainer);
    check(&root, || vec![archive(&root, S)]);
}

#[test]
fn s3_claim_after_prepare_cannot_publish_a_counted_archive() {
    let root = Root::new();
    root.archive(S, &birth(S, 1));
    let mut maintainer = root.maintainer();
    let work = maintainer
        .prepare()
        .expect("archive prepared while unclaimed");
    fs::create_dir(root.0.join("archive").join(format!(".publishing.{S}")))
        .expect("claim before commit");
    let step = maintainer.commit(work);
    assert!(
        !matches!(step.outcome, Outcome::Published { .. }),
        "claim must be checked again under the lock: {step:?}"
    );
    drain(&mut maintainer);
    check(&root, || vec![failed(S, SourceGap::Claimed)]);
}

#[derive(Clone, Copy, Debug)]
enum Mutation {
    MessageMode,
    TopMode,
    NestedMode,
    UnknownTop,
    Digest,
    Meta,
}

fn mutation_case(kind: Mutation) {
    let root = Root::new();
    let path = root.archive(S, &foreign());
    let events = fs::read(path.join("events.jsonl")).expect("fixed journal bytes");
    let inode = fs::metadata(path.join("events.jsonl"))
        .expect("fixed journal inode")
        .ino();
    let mut maintainer = root.maintainer();
    drain(&mut maintainer);
    check(&root, || vec![archive(&root, S)]);
    let node = match kind {
        Mutation::MessageMode => path.join("messages/one.txt"),
        Mutation::TopMode => path.join("memo.tsv"),
        Mutation::NestedMode => path.join("retained/nested/state"),
        Mutation::UnknownTop => path.join("unexpected"),
        Mutation::Digest => path.join("digest.md"),
        Mutation::Meta => path.join("meta"),
    };
    let saved = fs::read(&node).ok();
    match kind {
        Mutation::MessageMode | Mutation::TopMode | Mutation::NestedMode => {
            fs::set_permissions(&node, fs::Permissions::from_mode(0o700))
                .expect("invalid executable mode")
        }
        Mutation::UnknownTop => put(&node, b"unknown"),
        Mutation::Digest => put(&node, digest(T).as_bytes()),
        Mutation::Meta => put(&node, archive_meta(T).as_bytes()),
    }
    assert!(
        crate::archive::validated_journal(&path, S, &mut |p| fs::read(p)).is_err(),
        "changed fixture must independently fail validation: {kind:?}"
    );
    drain(&mut maintainer);
    check(&root, || vec![failed(S, SourceGap::Invalid)]);
    assert_eq!(
        fs::read(path.join("events.jsonl")).expect("journal"),
        events
    );
    assert_eq!(
        fs::metadata(path.join("events.jsonl"))
            .expect("journal")
            .ino(),
        inode
    );
    match saved {
        Some(bytes) => put(&node, &bytes),
        None => fs::remove_file(&node).expect("remove unknown entry"),
    }
    assert_valid(&path, S);
    drain(&mut maintainer);
    check(&root, || vec![archive(&root, S)]);
}

#[test]
fn s3_message_chmod_0600_to_0700_revalidates_without_journal_change() {
    mutation_case(Mutation::MessageMode);
}
#[test]
fn s3_top_file_chmod_revalidates_without_journal_change() {
    mutation_case(Mutation::TopMode);
}
#[test]
fn s3_nested_retained_change_revalidates_without_journal_change() {
    mutation_case(Mutation::NestedMode);
}
#[test]
fn s3_added_top_entry_revalidates_without_journal_change() {
    mutation_case(Mutation::UnknownTop);
}
#[test]
fn s3_digest_change_revalidates_without_journal_change() {
    mutation_case(Mutation::Digest);
}
#[test]
fn s3_meta_change_revalidates_without_journal_change() {
    mutation_case(Mutation::Meta);
}

fn purge(root: &Root) -> (u8, String, String) {
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
    .expect("purge command");
    (
        code,
        String::from_utf8(out).expect("stdout"),
        String::from_utf8(err).expect("stderr"),
    )
}

fn no_uuid(path: &Path, uuid: &str) {
    if !path.exists() {
        return;
    }
    for entry in fs::read_dir(path).expect("store evidence") {
        let entry = entry.expect("store entry");
        assert!(
            !entry.file_name().to_string_lossy().contains(uuid),
            "removed UUID remains in a file name"
        );
        if entry.file_type().expect("file type").is_dir() {
            no_uuid(&entry.path(), uuid);
        } else {
            let bytes = fs::read(entry.path()).expect("store bytes");
            assert!(
                !bytes
                    .windows(uuid.len())
                    .any(|window| window == uuid.as_bytes()),
                "removed UUID remains in store bytes at {}",
                entry.path().display()
            );
        }
    }
}

#[test]
fn s3_successful_purge_synchronously_removes_counts_and_uuid() {
    let root = Root::new();
    root.archive(S, &foreign());
    let mut maintainer = root.maintainer();
    drain(&mut maintainer);
    check(&root, || vec![archive(&root, S)]);
    let (code, _, err) = purge(&root);
    assert_eq!(code, 0, "{err}");
    assert!(!root.0.join("archive").join(S).exists());
    check(&root, Vec::new);
    no_uuid(&root.0.join("stats"), S);
}

#[test]
fn s3_purge_lock_failure_is_nonzero_and_same_command_retries_store_only() {
    let root = Root::new();
    root.archive(S, &foreign());
    let mut maintainer = root.maintainer();
    drain(&mut maintainer);
    let payload = Payload::at(&root.0);
    let lock = payload
        .lock(Duration::from_millis(100))
        .expect("hold stats lock");
    let (code, out, err) = purge(&root);
    assert_ne!(code, 0, "store removal failure is not a successful purge");
    assert!(out.is_empty(), "no success output on partial purge");
    assert!(
        err.contains("PURGE INCOMPLETE") && err.contains(S) && err.contains("Re-run"),
        "diagnostic names the remaining store and retry: {err}"
    );
    assert!(
        !root.0.join("archive").join(S).exists(),
        "archive bytes already removed"
    );
    assert!(
        root.0.join(format!("stats/session/{S}.evidence")).exists(),
        "postcommit partial state retains the indexed evidence"
    );
    drop(lock);
    let (code, _, err) = purge(&root);
    assert_eq!(code, 0, "store-only retry: {err}");
    check(&root, Vec::new);
    no_uuid(&root.0.join("stats"), S);
}

#[test]
fn s3_prepared_archive_cannot_resurrect_after_purge() {
    let root = Root::new();
    root.archive(S, &birth(S, 1));
    let mut maintainer = root.maintainer();
    let work = maintainer
        .prepare()
        .expect("initial absent-stamp archive registration");
    let (code, _, err) = purge(&root);
    assert_eq!(code, 0, "{err}");
    let step = maintainer.commit(work);
    assert!(
        !matches!(step.outcome, Outcome::Published { .. }),
        "purge invalidates stale registrations: {step:?}"
    );
}

#[test]
fn s3_archive_copy_and_live_disappearance_keep_the_fold() {
    let root = Root::new();
    let bytes = [foreign(), vec![b' '; 16384], b"\n".to_vec()].concat();
    let live_path = root.live("alpha", S, &bytes);
    let mut maintainer = root.maintainer();
    drain(&mut maintainer);
    root.archive(S, &bytes);
    let _ = disk::take_journal_reads();
    drain(&mut maintainer);
    assert!(
        disk::take_journal_reads().total <= 8192,
        "archive transfer copies the donor, with witness reads only"
    );
    check(&root, || vec![live(&root, "alpha"), archive(&root, S)]);
    fs::remove_dir_all(live_path).expect("live source disappears after transfer");
    drain(&mut maintainer);
    check(&root, || vec![archive(&root, S)]);
}

#[test]
fn s3_damaged_store_purge_resets_under_lock_and_can_rebuild() {
    let root = Root::new();
    root.archive(S, &foreign());
    let mut maintainer = root.maintainer();
    drain(&mut maintainer);
    put(&root.0.join("stats/view"), b"damaged\n");
    let (code, _, err) = purge(&root);
    assert_eq!(
        code, 0,
        "damage repair completes the privacy operation: {err}"
    );
    assert!(err.contains("reset"), "reset is reported: {err}");
    assert!(!root.0.join("archive").join(S).exists());
    for entry in fs::read_dir(&root.0).expect("state root") {
        let entry = entry.expect("state entry");
        if entry.file_name().to_string_lossy().starts_with("stats")
            && entry.file_type().expect("type").is_dir()
        {
            no_uuid(&entry.path(), S);
        }
    }
    root.live("new", T, &birth(T, 7));
    drain(&mut root.maintainer());
    check(&root, || vec![live(&root, "new")]);
}

#[test]
fn s3_incomplete_live_listing_cannot_remove_previously_counted_sources() {
    let root = Root::new();
    let bytes = foreign();
    root.live("alpha", S, &bytes);
    let mut maintainer = root.maintainer();
    drain(&mut maintainer);
    fs::rename(root.0.join("sessions"), root.0.join("saved-sessions")).expect("hide namespace");
    put(&root.0.join("sessions"), b"not a directory");
    drain(&mut maintainer);
    for scope in [None, Some(S), Some(T)] {
        let Shown::Ready { view, .. } = disk::shown(&mut Files::at(&root.0), scope) else {
            panic!("retained answer");
        };
        let oracle = stats::fold(
            vec![
                Source::live("alpha", meta("alpha", S).as_bytes(), Ok(bytes.clone())),
                Source::failed("sessions", None, SourceGap::Unreadable),
            ],
            scope,
        );
        assert_eq!(view, oracle, "an incomplete census is not proof of absence");
    }
    fs::remove_file(root.0.join("sessions")).expect("remove bad namespace");
    fs::rename(root.0.join("saved-sessions"), root.0.join("sessions")).expect("restore namespace");
    drain(&mut maintainer);
    check(&root, || vec![live(&root, "alpha")]);
}
