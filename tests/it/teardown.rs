//! `ae _end-local-teardown` / `_end-nonlocal-teardown` — session-state teardown on
//! the built binary, black-box.
//!
//! ROOT AUTHORITY (B1): the core derives the sessions/worktrees roots from the
//! ENVIRONMENT (`AE_HOME`), never from the operand — so every test sets `AE_HOME`
//! to its scratch root and builds the session under `<AE_HOME>/sessions`. The
//! outside-configured-root control proves that authority is load-bearing: point
//! `AE_HOME` elsewhere and the same structurally-valid session is refused, intact.
//!
//! Opposed controls throughout: a valid teardown removes exactly its resources and
//! leaves no tombstone; every refusal leaves the session (and any workdir, root,
//! or standing tombstone) exactly in place. The core never follows a link, never
//! deletes what it cannot prove is the configured session's, and never touches
//! origin.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "fixtures build real session/worktree dirs and read/tamper them on disk; the capability boundary is about what PRODUCT code may reach"
)]

use std::path::{Path, PathBuf};
use std::time::Duration;

/// A temp dir removed on drop, whose root doubles as `AE_HOME`.
struct Scratch(PathBuf);
impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ae-teardown-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Self(dir)
    }
    fn ae_home(&self) -> &Path {
        &self.0
    }
    fn sessions(&self) -> PathBuf {
        self.0.join("sessions")
    }
    fn worktrees(&self) -> PathBuf {
        self.0.join("worktrees")
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Build `<sessions>/<name>` with a `local`-shaped meta (`session`/`mode` only).
fn session_dir(sessions: &Path, name: &str, meta_session: &str, mode: &str) -> PathBuf {
    let dir = sessions.join(name);
    std::fs::create_dir_all(dir.join("messages")).expect("mkdir session");
    std::fs::write(
        dir.join("meta"),
        format!("session={meta_session}\nmode={mode}\norigin=/o\nwork_dir=/w\n"),
    )
    .expect("meta");
    dir
}

/// Build `<sessions>/<name>` with a nonlocal meta recording `mode`, `origin` and
/// the exact `work_dir` bytes.
fn nonlocal_session_dir(
    sessions: &Path,
    name: &str,
    mode: &str,
    origin: &str,
    work_dir: &str,
) -> PathBuf {
    let dir = sessions.join(name);
    std::fs::create_dir_all(dir.join("messages")).expect("mkdir session");
    std::fs::write(
        dir.join("meta"),
        format!("session={name}\nmode={mode}\norigin={origin}\nwork_dir={work_dir}\n"),
    )
    .expect("meta");
    dir
}

fn run_local(ae_home: &Path, dir: &Path) -> std::process::Output {
    invoke(ae_home, &["_end-local-teardown"], dir, false)
}

fn run_nonlocal(ae_home: &Path, dir: &Path, preserve: bool) -> std::process::Output {
    invoke(ae_home, &["_end-nonlocal-teardown"], dir, preserve)
}

fn invoke(ae_home: &Path, sub: &[&str], dir: &Path, preserve: bool) -> std::process::Output {
    let mut cmd = crate::cli::ae();
    cmd.env("AE_HOME", ae_home);
    cmd.args(sub).arg(dir);
    if preserve {
        cmd.arg("--preserve");
    }
    crate::cli::bounded(
        cmd.stdout(std::process::Stdio::piped())
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

// ─────────────────────────── local teardown (P3.5, retrofitted) ──────────────

#[test]
fn a_local_session_is_removed_and_no_tombstone_is_left() {
    let s = Scratch::new("ok");
    let dir = session_dir(&s.sessions(), "demo", "demo", "local");
    let sib = session_dir(&s.sessions(), "other", "other", "local");
    let out = run_local(s.ae_home(), &dir);
    assert_eq!(
        out.status.code(),
        Some(0),
        "local teardown succeeds: {}",
        stderr(&out)
    );
    assert!(out.stdout.is_empty(), "success is silent");
    assert!(!dir.exists(), "the session dir is gone");
    assert!(
        !s.sessions().join(".ending.demo").exists(),
        "no tombstone left"
    );
    assert!(s.sessions().exists(), "the sessions root survives");
    assert!(sib.exists(), "a sibling session is untouched");
}

#[test]
fn a_session_outside_the_configured_root_is_refused() {
    // LOAD-BEARING control for B1: AE_HOME points at one tree, the session lives in
    // another. The dir is structurally valid — real dir, valid name, byte-exact
    // local meta — so ONLY the configured-root authority refuses it. Remove that
    // check and this goes red (the outside-root dir would be deleted).
    let home = Scratch::new("authhome");
    std::fs::create_dir_all(home.sessions()).expect("configured sessions root");
    let elsewhere = Scratch::new("authelse");
    let dir = session_dir(&elsewhere.sessions(), "demo", "demo", "local");
    let out = run_local(home.ae_home(), &dir);
    assert_eq!(
        out.status.code(),
        Some(1),
        "an outside-root session refuses"
    );
    assert!(
        stderr(&out).contains("outside the configured sessions root"),
        "names the authority: {}",
        stderr(&out)
    );
    assert!(dir.exists(), "the outside-root dir is NOT deleted");
}

#[test]
fn a_nonlocal_mode_is_refused_by_the_local_entry() {
    let s = Scratch::new("git");
    let dir = session_dir(&s.sessions(), "demo", "demo", "git");
    let out = run_local(s.ae_home(), &dir);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("does not prove mode 'local'"));
    assert!(dir.exists());
}

#[test]
fn a_grammar_invalid_name_is_refused() {
    let s = Scratch::new("legacy");
    let dir = session_dir(&s.sessions(), "legacy.name", "legacy.name", "local");
    let out = run_local(s.ae_home(), &dir);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("not a grammar-valid session name"));
    assert!(dir.exists());
}

#[test]
fn a_symlinked_session_dir_is_refused() {
    let s = Scratch::new("symdir");
    std::fs::create_dir_all(s.sessions()).unwrap();
    let real = session_dir(&s.sessions(), "realone", "realone", "local");
    let link = s.sessions().join("demo");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let out = run_local(s.ae_home(), &link);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("not a real session directory"));
    assert!(real.exists(), "the link target is untouched");
    assert!(
        std::fs::symlink_metadata(&link).is_ok(),
        "the symlink itself remains"
    );
}

#[test]
fn a_mislabelled_directory_is_refused() {
    let s = Scratch::new("mislabel");
    let dir = session_dir(&s.sessions(), "demo", "someone-else", "local");
    let out = run_local(s.ae_home(), &dir);
    assert_eq!(out.status.code(), Some(1));
    let e = stderr(&out);
    assert!(
        e.contains("does not prove session 'demo'") && e.contains("someone-else"),
        "{e}"
    );
    assert!(dir.exists());
}

#[test]
fn a_missing_meta_is_refused() {
    let s = Scratch::new("nometa");
    let dir = session_dir(&s.sessions(), "demo", "demo", "local");
    std::fs::remove_file(dir.join("meta")).unwrap();
    let out = run_local(s.ae_home(), &dir);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("no readable meta"));
    assert!(dir.exists());
}

#[test]
fn a_standing_tombstone_is_refused_and_left_exactly_as_found() {
    let s = Scratch::new("standing");
    let dir = session_dir(&s.sessions(), "demo", "demo", "local");
    let tomb = s.sessions().join(".ending.demo");
    std::fs::create_dir_all(tomb.join("stuff")).unwrap();
    std::fs::write(tomb.join("stuff").join("f"), b"old state").unwrap();
    let out = run_local(s.ae_home(), &dir);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("is standing"));
    assert!(dir.exists(), "the live dir is untouched");
    assert!(
        tomb.join("stuff").join("f").exists(),
        "the tombstone is left as found"
    );
}

#[test]
fn an_absent_dir_is_refused() {
    let s = Scratch::new("absent");
    std::fs::create_dir_all(s.sessions()).unwrap();
    let out = run_local(s.ae_home(), &s.sessions().join("ghost"));
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("already absent"));
}

#[test]
fn a_trailing_whitespace_session_is_refused() {
    let s = Scratch::new("wsname");
    let dir = session_dir(&s.sessions(), "demo", "demo ", "local");
    let out = run_local(s.ae_home(), &dir);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("does not prove session 'demo'"));
    assert!(dir.exists());
}

#[test]
fn a_trailing_whitespace_mode_is_refused() {
    let s = Scratch::new("wsmode");
    let dir = session_dir(&s.sessions(), "demo", "demo", "local\t");
    let out = run_local(s.ae_home(), &dir);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("does not prove mode 'local'"));
    assert!(dir.exists());
}

#[test]
fn a_symlinked_sessions_root_is_refused() {
    // The CONFIGURED sessions root is a symlink: refused regardless of the operand,
    // and the link target is never deleted.
    let s = Scratch::new("rootsym");
    let host = s.0.join("host-sessions");
    let real = session_dir(&host, "demo", "demo", "local");
    std::os::unix::fs::symlink(&host, s.sessions()).unwrap();
    let out = run_local(s.ae_home(), &s.sessions().join("demo"));
    assert_eq!(out.status.code(), Some(1));
    let e = stderr(&out);
    assert!(
        e.contains("sessions root") && e.contains("not a real directory"),
        "{e}"
    );
    assert!(real.exists(), "the link target is untouched");
}

#[test]
fn a_traversal_component_is_refused() {
    let s = Scratch::new("traverse");
    let dir = session_dir(&s.sessions(), "demo", "demo", "local");
    let out = run_local(
        s.ae_home(),
        &s.sessions().join("demo").join("..").join("demo"),
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("traversal"));
    assert!(dir.exists());
}

#[test]
fn a_symlinked_meta_is_refused() {
    let s = Scratch::new("symmeta");
    let dir = s.sessions().join("demo");
    std::fs::create_dir_all(dir.join("messages")).unwrap();
    let external = s.0.join("evil-meta");
    std::fs::write(&external, "session=demo\nmode=local\n").unwrap();
    std::os::unix::fs::symlink(&external, dir.join("meta")).unwrap();
    let out = run_local(s.ae_home(), &dir);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("not a plain file"));
    assert!(dir.exists(), "the session dir is untouched");
    assert!(
        external.exists(),
        "the external forged-identity file is untouched"
    );
}

#[test]
fn a_fifo_meta_is_refused_and_never_blocks() {
    let s = Scratch::new("fifometa");
    let dir = s.sessions().join("demo");
    std::fs::create_dir_all(dir.join("messages")).unwrap();
    crate::cli::mkfifo(&dir.join("meta"));
    let out = run_local(s.ae_home(), &dir);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a FIFO meta refuses and never blocks"
    );
    assert!(stderr(&out).contains("not a plain file"));
    assert!(dir.exists());
}

// ─────────────────────────── nonlocal teardown (P3.6) ────────────────────────

/// A `full`/`git` session plus its managed workdir under `<AE_HOME>/worktrees`,
/// with `meta.work_dir` pointing byte-exact at the managed child.
fn nonlocal_with_workdir(s: &Scratch, name: &str, mode: &str, origin: &str) -> (PathBuf, PathBuf) {
    let managed = s.worktrees().join(name);
    std::fs::create_dir_all(&managed).expect("managed workdir");
    std::fs::write(managed.join("file"), b"work").unwrap();
    let dir = nonlocal_session_dir(
        &s.sessions(),
        name,
        mode,
        origin,
        managed.to_str().expect("utf8 workdir"),
    );
    (dir, managed)
}

#[test]
fn a_full_copy_teardown_removes_both_resources() {
    let s = Scratch::new("full");
    let (dir, managed) = nonlocal_with_workdir(&s, "demo", "full", "/o");
    let out = run_nonlocal(s.ae_home(), &dir, false);
    assert_eq!(
        out.status.code(),
        Some(0),
        "full teardown succeeds: {}",
        stderr(&out)
    );
    assert!(!managed.exists(), "the managed copy is gone");
    assert!(!dir.exists(), "the canonical state is gone");
    assert!(
        !s.worktrees().join(".ending.demo").exists(),
        "no workdir tombstone left"
    );
    assert!(
        !s.sessions().join(".ending.demo").exists(),
        "no session tombstone left"
    );
    assert!(
        s.worktrees().exists() && s.sessions().exists(),
        "both roots survive"
    );
}

#[test]
fn a_preserve_keeps_the_workdir_and_removes_canonical_only() {
    let s = Scratch::new("preserve");
    let (dir, managed) = nonlocal_with_workdir(&s, "demo", "full", "/o");
    let before = std::fs::read(managed.join("file")).unwrap();
    let out = run_nonlocal(s.ae_home(), &dir, true);
    assert_eq!(
        out.status.code(),
        Some(0),
        "preserve succeeds: {}",
        stderr(&out)
    );
    assert!(!dir.exists(), "canonical state is gone");
    assert!(managed.exists(), "the workdir is preserved");
    assert_eq!(
        std::fs::read(managed.join("file")).unwrap(),
        before,
        "byte-for-byte"
    );
}

#[test]
fn a_nonlocal_workdir_mismatch_is_refused() {
    // meta.work_dir does not point at the configured managed child.
    let s = Scratch::new("wdmismatch");
    std::fs::create_dir_all(s.worktrees()).unwrap();
    let dir = nonlocal_session_dir(&s.sessions(), "demo", "full", "/o", "/somewhere/else");
    let out = run_nonlocal(s.ae_home(), &dir, false);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("does not prove work_dir"));
    assert!(dir.exists());
}

#[test]
fn a_nonlocal_wrong_mode_is_refused() {
    let s = Scratch::new("nlwrongmode");
    std::fs::create_dir_all(s.worktrees()).unwrap();
    let managed = s.worktrees().join("demo");
    std::fs::create_dir_all(&managed).unwrap();
    let dir = nonlocal_session_dir(
        &s.sessions(),
        "demo",
        "local",
        "/o",
        managed.to_str().unwrap(),
    );
    let out = run_nonlocal(s.ae_home(), &dir, false);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("does not prove mode 'full' or 'git'"));
    assert!(dir.exists() && managed.exists());
}

#[test]
fn a_symlinked_worktrees_root_is_refused() {
    let s = Scratch::new("wtrootsym");
    std::fs::create_dir_all(s.sessions()).unwrap();
    let host = s.0.join("host-worktrees");
    std::fs::create_dir_all(&host).unwrap();
    std::os::unix::fs::symlink(&host, s.worktrees()).unwrap();
    // work_dir points at the lexical worktrees/demo (through the symlinked root).
    let managed = s.worktrees().join("demo");
    let dir = nonlocal_session_dir(
        &s.sessions(),
        "demo",
        "full",
        "/o",
        managed.to_str().unwrap(),
    );
    let out = run_nonlocal(s.ae_home(), &dir, false);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("worktrees root") && stderr(&out).contains("not a real directory")
    );
    assert!(dir.exists());
}

#[test]
fn a_git_teardown_removes_the_worktree_and_canonical() {
    let s = Scratch::new("gitok");
    let origin = s.0.join("origin");
    std::fs::create_dir_all(&origin).unwrap();
    crate::cli::git_in(&origin, &["init", "-q"]);
    crate::cli::git_in(&origin, &["commit", "-q", "--allow-empty", "-m", "init"]);
    std::fs::create_dir_all(s.worktrees()).unwrap();
    let managed = s.worktrees().join("demo");
    crate::cli::git_in(
        &origin,
        &[
            "worktree",
            "add",
            "--detach",
            managed.to_str().unwrap(),
            "HEAD",
        ],
    );
    let dir = nonlocal_session_dir(
        &s.sessions(),
        "demo",
        "git",
        origin.to_str().unwrap(),
        managed.to_str().unwrap(),
    );
    let out = run_nonlocal(s.ae_home(), &dir, false);
    assert_eq!(
        out.status.code(),
        Some(0),
        "git teardown succeeds: {}",
        stderr(&out)
    );
    assert!(!managed.exists(), "the worktree is removed");
    assert!(!dir.exists(), "canonical state is gone");
    assert!(origin.join(".git").exists(), "origin is untouched");
}

#[test]
fn a_git_refusal_retains_both_resources() {
    // origin is a real repo but the managed dir is NOT a registered worktree, so git
    // worktree remove refuses; the managed dir is still a real dir, so BOTH resources
    // are retained (retriable) and nothing is deleted.
    let s = Scratch::new("gitrefuse");
    let origin = s.0.join("origin");
    std::fs::create_dir_all(&origin).unwrap();
    crate::cli::git_in(&origin, &["init", "-q"]);
    crate::cli::git_in(&origin, &["commit", "-q", "--allow-empty", "-m", "init"]);
    let (dir, managed) = nonlocal_with_workdir(&s, "demo", "git", origin.to_str().unwrap());
    let out = run_nonlocal(s.ae_home(), &dir, false);
    assert_eq!(out.status.code(), Some(1), "a git refusal fails loudly");
    let e = stderr(&out);
    assert!(e.contains("git refused") && e.contains("RETAINED"), "{e}");
    assert!(
        managed.exists(),
        "the working tree is retained (no rm -rf fallback)"
    );
    assert!(dir.exists(), "the canonical state is retained");
}

#[test]
fn a_git_managed_symlink_is_refused_before_git_and_leaves_the_external_worktree_intact() {
    // BLOCKER regression: the managed child under worktrees/ is a SYMLINK pointing at
    // a worktree registered OUTSIDE the configured root. `git worktree remove --force`
    // resolves the link and would delete the external worktree at the target. The core
    // must lstat the managed child FIRST and refuse to hand a link to git — the
    // external worktree, its tracked content, the symlink and canonical state all stay.
    let s = Scratch::new("gitmanagedsym");
    let origin = s.0.join("origin");
    std::fs::create_dir_all(&origin).unwrap();
    crate::cli::git_in(&origin, &["init", "-q"]);
    std::fs::write(origin.join("tracked"), b"content").unwrap();
    crate::cli::git_in(&origin, &["add", "tracked"]);
    crate::cli::git_in(&origin, &["commit", "-q", "-m", "init"]);
    // An external worktree, registered in origin, living OUTSIDE the worktrees root.
    let external = s.0.join("external-wt");
    crate::cli::git_in(
        &origin,
        &[
            "worktree",
            "add",
            "--detach",
            external.to_str().unwrap(),
            "HEAD",
        ],
    );
    assert!(
        external.join("tracked").exists(),
        "sanity: the external worktree has its tracked content"
    );
    // worktrees/demo is a SYMLINK to that external worktree; meta.work_dir names the
    // lexical managed child byte-exact (the symlink path itself).
    std::fs::create_dir_all(s.worktrees()).unwrap();
    let managed = s.worktrees().join("demo");
    std::os::unix::fs::symlink(&external, &managed).unwrap();
    let dir = nonlocal_session_dir(
        &s.sessions(),
        "demo",
        "git",
        origin.to_str().unwrap(),
        managed.to_str().unwrap(),
    );
    let out = run_nonlocal(s.ae_home(), &dir, false);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a link managed child is refused"
    );
    let e = stderr(&out);
    assert!(
        e.contains("link or special node") && e.contains("RETAINED"),
        "{e}"
    );
    assert!(
        external.exists() && external.join("tracked").exists(),
        "git never ran: the external worktree and its content are intact"
    );
    assert!(
        std::fs::symlink_metadata(&managed).is_ok_and(|m| m.file_type().is_symlink()),
        "the managed symlink is left in place"
    );
    assert!(dir.exists(), "the canonical state is retained");
}
