//! `ae _archive-purge` — the `--purge-history` archive deletion (P3.4), black-box
//! against the built binary.
//!
//! Purge is a PRIVACY promise, so these are opposed controls: the success path
//! proves the bytes are gone AND no claim is left behind, and every refusal path
//! proves the archive is STILL THERE afterwards — purge never `rm -rf`s a tree it
//! cannot prove is provably this session's. Each archive is a REAL one the
//! publisher wrote and the shared validator (`src/archive/store.rs`) accepts; a
//! hand-built stub would only prove purge deletes whatever sits at the path.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "fixtures build real sessions, publish real archives, and read/tamper the tree on disk; the capability boundary is about what PRODUCT code may reach"
)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

const AID: &str = "e795c9e9-1111-2222-3333-444455556666";
const AT: &str = "2026-08-28T10:00:00Z";

/// A temp dir removed on drop, even while a test panics.
struct Scratch(PathBuf);
impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ae-purge-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Self(dir)
    }
    /// `<AE_HOME>` — the session lives under it, the archive root beside it.
    fn home(&self) -> &Path {
        &self.0
    }
    /// The archive root `<AE_HOME>/archive`, the way `archive_root_of` derives it.
    fn archive_root(&self) -> PathBuf {
        self.0.join("archive")
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Build a session `<home>/sessions/<name>` (`session_id=AID`,
/// `source_session=name`) with the core files and the archive root beside it —
/// but do NOT publish.
fn build_session(home: &Path, name: &str) -> PathBuf {
    let dir = home.join("sessions").join(name);
    std::fs::create_dir_all(dir.join("messages")).expect("mkdir session");
    std::fs::create_dir_all(home.join("archive")).expect("mkdir archive root");
    std::fs::write(
        dir.join("meta"),
        format!(
            "session={name}\n\
             session_id={AID}\n\
             session_id_origin=session\n\
             mode=worktree\n\
             origin=/some/origin\n\
             layout=vertical\n\
             ae_version=0.2.13\n\
             parent_archive_id=-\n\
             agent.main=cl:lead:sid-xyz\n\
             agent_bin.main=claude\n"
        ),
    )
    .expect("write meta");
    std::fs::write(dir.join("memo.tsv"), "ts1\tcl:lead\thandover\tpicking up\n").expect("memo");
    std::fs::write(
        dir.join("events.jsonl"),
        "{\"ts\":\"2026-08-01T00:00:00Z\",\"actor\":\"cl:lead\",\"action\":\"ask\",\"ref\":\"r1\",\"summary\":\"a question\",\"body_file\":\"messages/r1.ask.txt\"}\n",
    )
    .expect("events");
    std::fs::write(dir.join("messages").join("r1.ask.txt"), "the body\n").expect("message");
    dir
}

/// Publish the session at `dir` into a real archive at `<home>/archive/<AID>`.
fn publish(dir: &Path) {
    let out = crate::cli::bounded(
        crate::cli::ae()
            .arg("_archive-publish")
            .arg(dir)
            .arg("not-managed")
            .arg("-")
            .arg("-")
            .arg("-")
            .arg(AT)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn publish"),
        Duration::from_secs(10),
    )
    .expect("publish returned");
    assert_eq!(
        out.status.code(),
        Some(0),
        "setup publish rc: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A session dir whose archive is already published — the common starting point.
fn published(home: &Path, name: &str) -> PathBuf {
    let dir = build_session(home, name);
    publish(&dir);
    dir
}

/// Run `_archive-purge <dir> <aid> <source-session> <parent-id>`.
fn purge(dir: &Path, aid: &str, source: &str, parent: &str) -> std::process::Output {
    crate::cli::bounded(
        crate::cli::ae()
            .arg("_archive-purge")
            .arg(dir)
            .arg(aid)
            .arg(source)
            .arg(parent)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn purge"),
        Duration::from_secs(10),
    )
    .expect("purge returned")
}

#[test]
fn a_provably_owned_archive_is_purged_and_no_claim_is_left() {
    let scratch = Scratch::new("owned");
    let dir = published(scratch.home(), "demo");
    let target = scratch.archive_root().join(AID);
    assert!(target.exists(), "the archive exists before purge");

    let out = purge(&dir, AID, "demo", "-");
    assert_eq!(
        out.status.code(),
        Some(0),
        "an owned archive purges: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The removed target is printed for the bash consumer (no trailing newline).
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        stdout,
        target.to_string_lossy(),
        "stdout is the removed target"
    );
    assert!(!target.exists(), "the archive bytes are gone");
    assert!(
        !scratch
            .archive_root()
            .join(format!(".publishing.{AID}"))
            .exists(),
        "the purge claim was removed"
    );
}

#[test]
fn purging_the_parent_this_session_launched_from_is_refused() {
    let scratch = Scratch::new("parent");
    let dir = published(scratch.home(), "demo");
    // Parent-id == the id being purged: refused before the root is even touched.
    let out = purge(&dir, AID, "demo", AID);
    assert_eq!(out.status.code(), Some(1), "purging the parent refuses");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("parent archive"),
        "names why"
    );
    assert!(
        scratch.archive_root().join(AID).exists(),
        "the archive is intact"
    );
}

#[test]
fn a_wrong_source_session_is_refused_and_the_archive_survives() {
    let scratch = Scratch::new("wrongsrc");
    let dir = published(scratch.home(), "demo");
    // The archive records source_session=demo; this end claims another name.
    let out = purge(&dir, AID, "someone-else", "-");
    assert_eq!(out.status.code(), Some(1), "a non-owner refuses");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not provably this session's"),
        "names the ownership failure"
    );
    assert!(
        scratch.archive_root().join(AID).exists(),
        "the archive that is not ours is untouched"
    );
}

#[test]
fn an_empty_source_session_is_not_a_wildcard() {
    let scratch = Scratch::new("emptysrc");
    let dir = published(scratch.home(), "demo");
    // An empty source is never a match — it must not delete an owned archive.
    let out = purge(&dir, AID, "", "-");
    assert_eq!(out.status.code(), Some(1), "an empty owner refuses");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not provably this session's"),
        "names the ownership failure"
    );
    assert!(
        scratch.archive_root().join(AID).exists(),
        "the archive is intact"
    );
}

#[test]
fn a_standing_foreign_claim_is_refused_and_never_guess_cleaned() {
    let scratch = Scratch::new("standing");
    let dir = published(scratch.home(), "demo");
    // Someone else holds the claim (or a crash left it): purge must lose the mkdir,
    // refuse, and leave the foreign claim exactly as found.
    let claim = scratch.archive_root().join(format!(".publishing.{AID}"));
    std::fs::create_dir_all(&claim).unwrap();
    std::fs::write(claim.join("marker"), b"someone else's").unwrap();

    let out = purge(&dir, AID, "demo", "-");
    assert_eq!(out.status.code(), Some(1), "a standing claim refuses");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("is standing"),
        "names the standing claim"
    );
    assert!(
        claim.join("marker").exists(),
        "the foreign claim was left exactly as found, not guess-cleaned"
    );
    assert!(
        scratch.archive_root().join(AID).exists(),
        "the archive behind the claim is untouched"
    );
}

#[test]
fn an_absent_target_succeeds_with_no_output_and_no_claim() {
    let scratch = Scratch::new("absent");
    // A real archive ROOT, but no archive for this id (never published): nothing to
    // purge — succeed silently and leave no claim of our own behind.
    let dir = build_session(scratch.home(), "demo");
    assert!(
        !scratch.archive_root().join(AID).exists(),
        "no archive for this id"
    );
    let out = purge(&dir, AID, "demo", "-");
    assert_eq!(
        out.status.code(),
        Some(0),
        "nothing to purge succeeds: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stdout.is_empty(), "no target printed when none existed");
    assert!(
        !scratch
            .archive_root()
            .join(format!(".publishing.{AID}"))
            .exists(),
        "our own claim was cleaned durably"
    );
}

#[test]
fn a_symlinked_root_is_refused() {
    let scratch = Scratch::new("symroot");
    let dir = build_session(scratch.home(), "demo");
    // The archive root is a SYMLINK: acting through it is a hard refusal.
    let root = scratch.archive_root();
    std::fs::remove_dir_all(&root).unwrap();
    let decoy = scratch.home().join("decoy");
    std::fs::create_dir_all(&decoy).unwrap();
    std::os::unix::fs::symlink(&decoy, &root).unwrap();

    let out = purge(&dir, AID, "demo", "-");
    assert_eq!(out.status.code(), Some(1), "a symlinked root refuses");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("is a symlink"),
        "refuses to act through the link"
    );
}

#[test]
fn an_unvalidatable_tree_is_refused_and_left_in_place() {
    let scratch = Scratch::new("unvalid");
    let dir = published(scratch.home(), "demo");
    // A file inside the archive gains an executable bit: purge must prove the tree
    // IS an archive before deleting it — rm -rf is not how you find out what a
    // thing is — so it refuses and leaves it.
    let victim = scratch.archive_root().join(AID).join("memo.tsv");
    std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o700)).unwrap();

    let out = purge(&dir, AID, "demo", "-");
    let _ = std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o600));
    assert_eq!(out.status.code(), Some(1), "an unvalidatable tree refuses");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("does not validate as an ae archive"),
        "names the validation failure"
    );
    assert!(
        scratch.archive_root().join(AID).exists(),
        "the thing it refused to delete is still there"
    );
}

#[test]
fn an_extra_top_level_file_is_refused_and_left_in_place() {
    let scratch = Scratch::new("extrafile");
    let dir = published(scratch.home(), "demo");
    // An UNRECOGNISED top-level file (e.g. a leaked launch script or secret).
    let extra = scratch.archive_root().join(AID).join("launch.main.sh");
    std::fs::write(&extra, b"#!/bin/sh\necho leaked\n").unwrap();

    let out = purge(&dir, AID, "demo", "-");
    assert_eq!(
        out.status.code(),
        Some(1),
        "an extra top-level file refuses"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("does not validate as an ae archive"),
        "names the validation failure"
    );
    assert!(
        extra.exists(),
        "the archive (and its extra file) is left in place"
    );
}

#[test]
fn an_extra_top_level_directory_is_refused_and_left_in_place() {
    let scratch = Scratch::new("extradir");
    let dir = published(scratch.home(), "demo");
    // An UNRECOGNISED top-level directory: same hazard, and recursive removal
    // is exactly what would wipe it.
    let extra = scratch.archive_root().join(AID).join("evil");
    std::fs::create_dir_all(&extra).unwrap();
    std::fs::write(extra.join("payload"), b"do not delete unexamined").unwrap();

    let out = purge(&dir, AID, "demo", "-");
    assert_eq!(
        out.status.code(),
        Some(1),
        "an extra top-level directory refuses"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("does not validate as an ae archive"),
        "names the validation failure"
    );
    assert!(
        extra.join("payload").exists(),
        "the unexamined directory was never recursively deleted"
    );
}
