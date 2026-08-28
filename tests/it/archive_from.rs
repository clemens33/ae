//! `ae _archive-from-preflight` — the read-only `--from` inheritance preflight
//! (P3.4), black-box against the built binary.
//!
//! The preflight proves an archive is a real, validated, not-mid-flight ae
//! archive and hands Bash exactly the tuple it consumes — `aid\thandover\t
//! pending`, no trailing newline — or refuses with a named `Error:` and rc 1. It
//! shares ONE validator/root/claim implementation with publish and purge
//! (`src/archive/store.rs`), so these opposed controls also guard that the shared
//! trust rules refuse what they must. Every case runs against a REAL archive the
//! publisher itself wrote: a hand-built tree would only prove the preflight
//! accepts whatever sits at the path — which is the behaviour that would be wrong.

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
        let dir = std::env::temp_dir().join(format!("ae-from-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Self(dir)
    }
    /// `<AE_HOME>` — the session lives under it, the archive root beside it.
    fn home(&self) -> &Path {
        &self.0
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Build a session `<home>/sessions/<name>` (`session_id=AID`) and PUBLISH it, so
/// every from-preflight below runs against a real, validator-approved archive at
/// `<home>/archive/<AID>`. Returns the archive root `<home>/archive`.
fn published_archive(home: &Path, name: &str) -> PathBuf {
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

    let out = crate::cli::bounded(
        crate::cli::ae()
            .arg("_archive-publish")
            .arg(&dir)
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
    home.join("archive")
}

/// Run `_archive-from-preflight <root> <raw-uuid>`.
fn preflight(root: &Path, raw_uuid: &str) -> std::process::Output {
    crate::cli::bounded(
        crate::cli::ae()
            .arg("_archive-from-preflight")
            .arg(root)
            .arg(raw_uuid)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn preflight"),
        Duration::from_secs(10),
    )
    .expect("preflight returned")
}

#[test]
fn a_real_archive_preflights_and_prints_the_frozen_tuple() {
    let scratch = Scratch::new("ok");
    let root = published_archive(scratch.home(), "demo");
    let out = preflight(&root, AID);
    assert_eq!(
        out.status.code(),
        Some(0),
        "a valid archive preflights: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Exactly three tab fields, NO trailing newline (bash captures it with `$(...)`
    // and splits on the tabs); field 0 is the canonical id and the counts parse.
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(!stdout.ends_with('\n'), "no trailing newline: {stdout:?}");
    let fields: Vec<&str> = stdout.split('\t').collect();
    assert_eq!(fields.len(), 3, "id + two counts: {stdout:?}");
    assert_eq!(fields[0], AID, "field 0 is the canonical id");
    assert!(
        fields[1].parse::<u64>().is_ok(),
        "handover count parses: {:?}",
        fields[1]
    );
    assert!(
        fields[2].parse::<u64>().is_ok(),
        "pending count parses: {:?}",
        fields[2]
    );
}

#[test]
fn a_non_uuid_is_refused() {
    let scratch = Scratch::new("nonuuid");
    let root = published_archive(scratch.home(), "demo");
    let out = preflight(&root, "not-a-uuid");
    assert_eq!(out.status.code(), Some(1), "a non-UUID refuses");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("is not one"),
        "names the bad id"
    );
    assert!(out.stdout.is_empty(), "no tuple on a refusal");
}

#[test]
fn an_absent_archive_is_refused() {
    let scratch = Scratch::new("absent");
    let root = published_archive(scratch.home(), "demo");
    // A well-formed id with no archive directory under an otherwise real root.
    let out = preflight(&root, "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
    assert_eq!(out.status.code(), Some(1), "an absent archive refuses");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no archive"),
        "names the missing archive"
    );
}

#[test]
fn a_live_claim_refuses_inheriting_mid_flight() {
    let scratch = Scratch::new("claim");
    let root = published_archive(scratch.home(), "demo");
    // A standing `.publishing.<AID>` means this exact id is being published or
    // purged right now — inheriting frozen counts that are about to change refuses.
    std::fs::create_dir_all(root.join(format!(".publishing.{AID}"))).unwrap();
    let out = preflight(&root, AID);
    assert_eq!(out.status.code(), Some(1), "a mid-flight id refuses");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("being published or purged"),
        "names the in-flight state"
    );
}

#[test]
fn a_symlinked_root_is_refused() {
    let scratch = Scratch::new("symroot");
    // A real archive exists; the preflight is pointed at a SYMLINK standing in for
    // the root — the way a lineage pointer would reach outside the archive ae owns.
    let real = published_archive(scratch.home(), "demo");
    let link = scratch.home().join("archive-link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let out = preflight(&link, AID);
    assert_eq!(out.status.code(), Some(1), "a symlinked root refuses");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("is a symlink"),
        "refuses to act through the link"
    );
}

#[test]
fn a_tampered_archive_does_not_validate() {
    let scratch = Scratch::new("tamper");
    let root = published_archive(scratch.home(), "demo");
    // A file inside the archive gains an executable bit: it stays readable (so it
    // passes the meta/digest readable gate) but the shared validator refuses it.
    let victim = root.join(AID).join("memo.tsv");
    std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o700)).unwrap();
    let out = preflight(&root, AID);
    let _ = std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o600));
    assert_eq!(out.status.code(), Some(1), "a tampered archive refuses");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("did not validate"),
        "refuses to inherit from an unvalidatable tree"
    );
}

#[test]
fn an_extra_top_level_entry_does_not_validate() {
    let scratch = Scratch::new("extra");
    let root = published_archive(scratch.home(), "demo");
    // An unrecognised top-level entry (here a directory) must fail validation — the
    // shared exact-root whitelist rejects anything outside the known archive set,
    // so lineage is never inherited from a tree with unvalidated contents.
    std::fs::create_dir_all(root.join(AID).join("extra-stuff")).unwrap();
    let out = preflight(&root, AID);
    assert_eq!(out.status.code(), Some(1), "an extra entry refuses");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("did not validate"),
        "refuses to inherit from a tree with an unrecognised entry"
    );
}
