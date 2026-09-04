//! `ae _archive-preview` git facts against REAL temporary repositories (P3.2).
//!
//! The preview derives `Final commit`, `Commit range` and `Commit count` by
//! running `git` for a non-local session. These black-box tests build real
//! repos and assert the digest's git lines, covering the shapes the frozen
//! `_ar_git_head`/`_ar_git_range` handle: ordinary history, detached HEAD, a
//! linked-worktree `.git` pointer, a non-ancestor base, a missing repo, and —
//! the point of the OS-native argv door — a work dir or base that would be an
//! injection if a shell were ever involved.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "fixtures build real repos and sessions on disk; the capability boundary \
              is about what PRODUCT code may reach"
)]

use std::path::{Path, PathBuf};
use std::time::Duration;

/// A temp dir removed on drop (Drop runs while unwinding, so a panicking test
/// still cleans up).
struct Scratch(PathBuf);
impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ae-gitfacts-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Self(dir)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A repo of `n` empty commits; returns its path and every commit sha oldest
/// first.
fn repo_with_commits(scratch: &Scratch, name: &str, n: usize) -> (PathBuf, Vec<String>) {
    let repo = scratch.path().join(name);
    std::fs::create_dir_all(&repo).expect("mkdir repo");
    crate::cli::git_in(&repo, &["init", "-q"]);
    let mut shas = Vec::new();
    for i in 0..n {
        crate::cli::git_in(
            &repo,
            &["commit", "-q", "--allow-empty", "-m", &format!("c{i}")],
        );
        shas.push(crate::cli::git_in(&repo, &["rev-parse", "HEAD"]));
    }
    (repo, shas)
}

/// Write a session `meta` and return its dir.
fn session(scratch: &Scratch, name: &str, mode: &str, work_dir: &str, git_base: &str) -> PathBuf {
    let dir = scratch.path().join(format!("session-{name}"));
    std::fs::create_dir_all(&dir).expect("mkdir session");
    let meta = format!(
        "session={name}\n\
         session_id=e795c9e9-1111-2222-3333-444455556666\n\
         session_id_origin=session\n\
         mode={mode}\n\
         work_dir={work_dir}\n\
         git_base_commit={git_base}\n\
         agent.main=cl:lead:x\n\
         agent_bin.main=claude\n"
    );
    std::fs::write(dir.join("meta"), meta).expect("write meta");
    std::fs::write(dir.join("events.jsonl"), b"").expect("write events");
    dir
}

/// The four git lines of a preview, as `(base, final, range, count)`.
fn git_facts(dir: &Path) -> (String, String, String, String) {
    let child = crate::cli::ae()
        .arg("_archive-preview")
        .arg(dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    let out = crate::cli::bounded(child, Duration::from_secs(10)).expect("preview returned");
    assert_eq!(
        out.status.code(),
        Some(0),
        "preview rc for {}",
        dir.display()
    );
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let field = |label: &str| -> String {
        stdout
            .lines()
            .find_map(|l| l.strip_prefix(&format!("- {label}: ")))
            .unwrap_or_else(|| panic!("no '{label}' line in:\n{stdout}"))
            .to_owned()
    };
    (
        field("Base commit"),
        field("Final commit"),
        field("Commit range"),
        field("Commit count"),
    )
}

#[test]
fn worktree_facts_match_a_real_repo() {
    let scratch = Scratch::new("ordinary");
    let (repo, shas) = repo_with_commits(&scratch, "repo", 3);
    let (base, tip) = (&shas[0], &shas[2]);
    let dir = session(&scratch, "wt", "worktree", &repo.to_string_lossy(), base);
    let (b, f, r, c) = git_facts(&dir);
    assert_eq!(b, *base, "base is the meta value");
    assert_eq!(f, *tip, "final is HEAD");
    assert_eq!(r, format!("{base}..{tip}"), "range");
    assert_eq!(c, "2", "count of base..tip");
}

#[test]
fn copy_mode_also_derives_git_facts() {
    // Copy is non-local too, so it runs git — proving the widened gate reaches
    // more than worktree.
    let scratch = Scratch::new("copy");
    let (repo, shas) = repo_with_commits(&scratch, "repo", 2);
    let dir = session(&scratch, "cp", "copy", &repo.to_string_lossy(), &shas[0]);
    let (_, f, r, c) = git_facts(&dir);
    assert_eq!(f, shas[1], "final is HEAD in copy mode");
    assert_eq!(r, format!("{}..{}", shas[0], shas[1]));
    assert_eq!(c, "1");
}

#[test]
fn a_detached_head_still_reports_its_commit() {
    let scratch = Scratch::new("detached");
    let (repo, shas) = repo_with_commits(&scratch, "repo", 3);
    crate::cli::git_in(&repo, &["checkout", "-q", &shas[1]]); // detach onto c1
    let dir = session(
        &scratch,
        "det",
        "worktree",
        &repo.to_string_lossy(),
        &shas[0],
    );
    let (_, f, _, c) = git_facts(&dir);
    assert_eq!(f, shas[1], "final is the detached commit");
    assert_eq!(c, "1", "base..detached is one commit");
}

#[test]
fn a_linked_worktree_pointer_is_followed() {
    // A `git worktree add` dir has `.git` as a FILE pointing to the real gitdir;
    // the preview must still resolve HEAD through it.
    let scratch = Scratch::new("wtptr");
    let (repo, shas) = repo_with_commits(&scratch, "repo", 2);
    let linked = scratch.path().join("linked");
    crate::cli::git_in(
        &repo,
        &["worktree", "add", "-q", &linked.to_string_lossy(), "HEAD"],
    );
    assert!(
        linked.join(".git").is_file(),
        "the worktree .git is a pointer file"
    );
    let dir = session(
        &scratch,
        "wp",
        "worktree",
        &linked.to_string_lossy(),
        &shas[0],
    );
    let (_, f, _, c) = git_facts(&dir);
    assert_eq!(f, shas[1], "final resolved through the pointer");
    assert_eq!(c, "1");
}

#[test]
fn a_non_ancestor_base_yields_dash_range() {
    // Base is NOT an ancestor of HEAD, so merge-base --is-ancestor fails and the
    // range/count fall to '-', while final is still HEAD.
    let scratch = Scratch::new("nonanc");
    let (repo, shas) = repo_with_commits(&scratch, "repo", 3);
    let orphan = crate::cli::git_in(
        &repo,
        &["commit-tree", "-m", "orphan", &{
            crate::cli::git_in(&repo, &["rev-parse", "HEAD^{tree}"])
        }],
    );
    let dir = session(&scratch, "na", "worktree", &repo.to_string_lossy(), &orphan);
    let (b, f, r, c) = git_facts(&dir);
    assert_eq!(
        b, orphan,
        "base shown verbatim even when unusable for a range"
    );
    assert_eq!(f, shas[2], "final is HEAD");
    assert_eq!(r, "-", "no range for a non-ancestor base");
    assert_eq!(c, "-", "no count for a non-ancestor base");
}

#[test]
fn a_missing_repo_yields_dash_final_and_range() {
    let scratch = Scratch::new("missing");
    let missing = scratch.path().join("no-such-repo");
    let dir = session(
        &scratch,
        "ms",
        "worktree",
        &missing.to_string_lossy(),
        "0123456789abcdef0123456789abcdef01234567",
    );
    let (b, f, r, c) = git_facts(&dir);
    assert_eq!(
        b, "0123456789abcdef0123456789abcdef01234567",
        "base is the meta value"
    );
    assert_eq!(f, "-", "no HEAD for a missing repo");
    assert_eq!(r, "-");
    assert_eq!(c, "-");
}

#[test]
fn an_injected_workdir_or_base_runs_no_shell() {
    // The whole point of OS-native argv: a work dir or base that would execute
    // in a shell is passed to git as one datum, git fails on it, the facts fall
    // to '-', and the side effect NEVER happens.
    let scratch = Scratch::new("inject");
    let canary = scratch.path().join("pwned");
    let hostile_wd = format!("{}/x; touch {}", scratch.path().display(), canary.display());
    let hostile_base = format!("$(touch {})", canary.display());
    let dir = session(&scratch, "inj", "worktree", &hostile_wd, &hostile_base);
    let (b, f, r, c) = git_facts(&dir);
    assert_eq!(
        b, hostile_base,
        "the hostile base is shown verbatim, never executed"
    );
    assert_eq!(f, "-", "git fails on the hostile work dir");
    assert_eq!(r, "-");
    assert_eq!(c, "-");
    assert!(!canary.exists(), "a shell ran: the canary file was created");
}
