//! `ae _end-local-teardown` — local-mode canonical session-state removal (P3.5),
//! black-box against the built binary.
//!
//! Opposed controls: the happy path removes the session dir and leaves NO
//! tombstone; every refusal path leaves the session (and any standing tombstone)
//! exactly in place. The core never follows a link, never deletes a directory it
//! cannot prove is this local session's, and never touches a sibling session or
//! the sessions root itself.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "fixtures build real session dirs and read/tamper them on disk; the capability boundary is about what PRODUCT code may reach"
)]

use std::path::{Path, PathBuf};
use std::time::Duration;

/// A temp dir removed on drop, even while a test panics.
struct Scratch(PathBuf);
impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ae-teardown-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Self(dir)
    }
    /// The sessions root `<scratch>/sessions` — the parent the core derives.
    fn sessions(&self) -> PathBuf {
        self.0.join("sessions")
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Build `<sessions>/<name>` with a meta recording `session=<meta_session>` and
/// `mode=<mode>`. Returns the session directory.
fn session_dir(sessions: &Path, name: &str, meta_session: &str, mode: &str) -> PathBuf {
    let dir = sessions.join(name);
    std::fs::create_dir_all(dir.join("messages")).expect("mkdir session");
    std::fs::write(
        dir.join("meta"),
        format!("session={meta_session}\nmode={mode}\norigin=/some/origin\nwork_dir=/some/wd\n"),
    )
    .expect("meta");
    std::fs::write(dir.join("memo.tsv"), "x\n").expect("memo");
    dir
}

/// Run `_end-local-teardown <session-dir>`.
fn teardown(dir: &Path) -> std::process::Output {
    crate::cli::bounded(
        crate::cli::ae()
            .arg("_end-local-teardown")
            .arg(dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn teardown"),
        Duration::from_secs(10),
    )
    .expect("teardown returned")
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn a_local_session_is_removed_and_no_tombstone_is_left() {
    let scratch = Scratch::new("ok");
    let dir = session_dir(&scratch.sessions(), "demo", "demo", "local");
    // A sibling session that must NEVER be touched.
    let sib = session_dir(&scratch.sessions(), "other", "other", "local");

    let out = teardown(&dir);
    assert_eq!(
        out.status.code(),
        Some(0),
        "local teardown succeeds: {}",
        stderr(&out)
    );
    assert!(out.stdout.is_empty(), "success is silent");
    assert!(!dir.exists(), "the session dir is gone");
    assert!(
        !scratch.sessions().join(".ending.demo").exists(),
        "no tombstone is left after a clean teardown"
    );
    assert!(scratch.sessions().exists(), "the sessions root survives");
    assert!(sib.exists(), "a sibling session is untouched");
}

#[test]
fn a_nonlocal_mode_is_refused() {
    let scratch = Scratch::new("git");
    let dir = session_dir(&scratch.sessions(), "demo", "demo", "git");
    let out = teardown(&dir);
    assert_eq!(out.status.code(), Some(1), "a non-local session refuses");
    assert!(
        stderr(&out).contains("does not prove mode 'local'"),
        "names the mode"
    );
    assert!(dir.exists(), "the non-local session is untouched");
}

#[test]
fn a_grammar_invalid_name_is_refused() {
    let scratch = Scratch::new("legacy");
    // A legacy name that fails the fresh grammar (a dot in the body): it must stay
    // on the bash path, so the core refuses it rather than acting.
    let dir = session_dir(&scratch.sessions(), "legacy.name", "legacy.name", "local");
    let out = teardown(&dir);
    assert_eq!(out.status.code(), Some(1), "a legacy name refuses");
    assert!(
        stderr(&out).contains("not a grammar-valid session name"),
        "names the grammar boundary"
    );
    assert!(dir.exists(), "the legacy session is left for the bash path");
}

#[test]
fn a_symlinked_session_dir_is_refused() {
    let scratch = Scratch::new("symdir");
    let real = session_dir(&scratch.sessions(), "realone", "realone", "local");
    // The path handed in is a SYMLINK to a real session — never followed or deleted.
    let link = scratch.sessions().join("demo");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let out = teardown(&link);
    assert_eq!(out.status.code(), Some(1), "a symlinked dir refuses");
    assert!(
        stderr(&out).contains("not a real session directory"),
        "refuses to follow the link"
    );
    assert!(real.exists(), "the link target session is untouched");
    assert!(
        std::fs::symlink_metadata(&link).is_ok(),
        "the symlink itself was not removed"
    );
}

#[test]
fn a_mislabelled_directory_is_refused() {
    let scratch = Scratch::new("mislabel");
    // The dir is named 'demo' but its meta records a different session name.
    let dir = session_dir(&scratch.sessions(), "demo", "someone-else", "local");
    let out = teardown(&dir);
    assert_eq!(out.status.code(), Some(1), "a mislabelled dir refuses");
    let e = stderr(&out);
    assert!(
        e.contains("does not prove session 'demo'") && e.contains("someone-else"),
        "names the exact-match failure and what was recorded: {e}"
    );
    assert!(dir.exists(), "the mislabelled dir is untouched");
}

#[test]
fn a_missing_meta_is_refused() {
    let scratch = Scratch::new("nometa");
    let dir = session_dir(&scratch.sessions(), "demo", "demo", "local");
    std::fs::remove_file(dir.join("meta")).unwrap();
    let out = teardown(&dir);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a session it cannot identify refuses"
    );
    assert!(stderr(&out).contains("no readable meta"), "names why");
    assert!(dir.exists(), "the unidentifiable dir is untouched");
}

#[test]
fn a_standing_tombstone_is_refused_and_left_exactly_as_found() {
    let scratch = Scratch::new("standing");
    let dir = session_dir(&scratch.sessions(), "demo", "demo", "local");
    // A previous crashed teardown left a tombstone: refuse and NEVER overwrite it.
    let tomb = scratch.sessions().join(".ending.demo");
    std::fs::create_dir_all(tomb.join("stuff")).unwrap();
    std::fs::write(tomb.join("stuff").join("f"), b"old state").unwrap();

    let out = teardown(&dir);
    assert_eq!(out.status.code(), Some(1), "a standing tombstone refuses");
    assert!(
        stderr(&out).contains("is standing"),
        "names the standing tombstone"
    );
    assert!(dir.exists(), "the live session dir is untouched");
    assert!(
        tomb.join("stuff").join("f").exists(),
        "the standing tombstone is left exactly as found, never overwritten"
    );
}

#[test]
fn an_absent_dir_is_refused() {
    let scratch = Scratch::new("absent");
    std::fs::create_dir_all(scratch.sessions()).unwrap();
    let dir = scratch.sessions().join("ghost");
    let out = teardown(&dir);
    assert_eq!(out.status.code(), Some(1), "an absent session refuses");
    assert!(stderr(&out).contains("already absent"), "names the absence");
}

#[test]
fn a_trailing_whitespace_session_is_refused() {
    let scratch = Scratch::new("wsname");
    // The dir is 'demo' but its meta records `session=demo ` (trailing space). A
    // destructive op must NOT normalize the byte-mismatch into a match.
    let dir = session_dir(&scratch.sessions(), "demo", "demo ", "local");
    let out = teardown(&dir);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a non-exact session value refuses"
    );
    assert!(
        stderr(&out).contains("does not prove session 'demo'"),
        "names the byte-exact failure"
    );
    assert!(dir.exists(), "the unproven dir is untouched");
}

#[test]
fn a_trailing_whitespace_mode_is_refused() {
    let scratch = Scratch::new("wsmode");
    // meta records `mode=local\t` (trailing tab) — not byte-exactly `local`.
    let dir = session_dir(&scratch.sessions(), "demo", "demo", "local\t");
    let out = teardown(&dir);
    assert_eq!(out.status.code(), Some(1), "a non-exact mode value refuses");
    assert!(
        stderr(&out).contains("does not prove mode 'local'"),
        "names the byte-exact failure"
    );
    assert!(dir.exists(), "the unproven dir is untouched");
}

#[test]
fn a_symlinked_sessions_root_is_refused() {
    let scratch = Scratch::new("rootsym");
    // The sessions ROOT is a symlink to a real dir elsewhere: teardown of a session
    // reached THROUGH it must be refused, never delete the link target.
    let host = scratch.0.join("host");
    let real = session_dir(&host, "demo", "demo", "local");
    let link = scratch.0.join("sessions");
    std::os::unix::fs::symlink(&host, &link).unwrap();
    let out = teardown(&link.join("demo"));
    assert_eq!(
        out.status.code(),
        Some(1),
        "a symlinked sessions root refuses"
    );
    let e = stderr(&out);
    assert!(
        e.contains("sessions root") && e.contains("not a real directory"),
        "refuses to act through a symlinked root: {e}"
    );
    assert!(
        real.exists(),
        "the target under the real host is untouched (never deleted through the link)"
    );
}

#[test]
fn a_traversal_component_is_refused() {
    let scratch = Scratch::new("traverse");
    let dir = session_dir(&scratch.sessions(), "demo", "demo", "local");
    // A `..` in the operand could pass the basename grammar while resolving elsewhere.
    let out = teardown(&scratch.sessions().join("demo").join("..").join("demo"));
    assert_eq!(out.status.code(), Some(1), "a '..' traversal refuses");
    assert!(stderr(&out).contains("traversal"), "names the traversal");
    assert!(dir.exists(), "the real session is untouched");
}

#[test]
fn a_symlinked_meta_is_refused() {
    let scratch = Scratch::new("symmeta");
    let dir = scratch.sessions().join("demo");
    std::fs::create_dir_all(dir.join("messages")).unwrap();
    // The identity evidence is a SYMLINK pointing OUTSIDE the session dir at an
    // attacker-controlled file whose bytes would pass the byte-exact identity check.
    // The core must not prove identity through a followed link.
    let external = scratch.0.join("evil-meta");
    std::fs::write(&external, "session=demo\nmode=local\n").unwrap();
    std::os::unix::fs::symlink(&external, dir.join("meta")).unwrap();
    let out = teardown(&dir);
    assert_eq!(out.status.code(), Some(1), "a symlinked meta refuses");
    assert!(
        stderr(&out).contains("not a plain file"),
        "refuses to prove identity through a link"
    );
    assert!(dir.exists(), "the session dir is untouched");
    assert!(
        external.exists(),
        "the external forged-identity file is untouched"
    );
}

#[test]
fn a_fifo_meta_is_refused_and_never_blocks() {
    let scratch = Scratch::new("fifometa");
    let dir = scratch.sessions().join("demo");
    std::fs::create_dir_all(dir.join("messages")).unwrap();
    // A FIFO at the meta path would make `fs::read` block forever, hanging `ae end`
    // under the lifecycle lock. The core must classify and refuse it, never read it.
    let meta = dir.join("meta");
    crate::cli::mkfifo(&meta);
    // `teardown()` is bounded (10s): a regression that reads the FIFO times out and
    // returns no exit code rather than `Some(1)`, so this asserts both refusal and
    // that nothing blocked.
    let out = teardown(&dir);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a FIFO meta refuses and never blocks"
    );
    assert!(
        stderr(&out).contains("not a plain file"),
        "refuses the FIFO node"
    );
    assert!(dir.exists(), "the session dir is untouched");
}
