//! Frozen acceptance (#248 P1b-1 RETAIN): an ordinary End captures EVERY
//! non-registration node of the session directory as an inert copy, refuses a
//! node it cannot capture, keeps legacy state paths and any managed workdir it
//! cannot prove lossless, and a direct unchanged retry reuses its own archive
//! while every drift refuses and leaves that archive exact.
//!
//! Oracle: `p1b-inventory-r3.md` (the fate table), `plan-p1b-r3.md` 5-27 and the
//! lead rulings bfbe06ce (R-I8 helper links, R-REC fail-closed retry, R-I3 kept
//! workdir and its launch refusal text), colead b5e2aa25 I1-I4/N1. Real
//! `_end`, private tmux socket, hermetic git.

use std::collections::BTreeMap;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{PermissionsExt, symlink};

use super::*;

/// Relative path -> (kind, permission bits, bytes or link text).
type Tree = BTreeMap<PathBuf, (char, u32, Vec<u8>)>;

/// Every node under `root`, by lstat; a link is never followed.
fn tree(root: &Path) -> Tree {
    let mut out = Tree::new();
    let mut todo = vec![PathBuf::new()];
    while let Some(rel) = todo.pop() {
        let here = root.join(&rel);
        let entries =
            std::fs::read_dir(&here).unwrap_or_else(|why| panic!("{}: {why}", here.display()));
        for entry in entries {
            let path = rel.join(entry.expect("an entry").file_name());
            let full = root.join(&path);
            let meta = std::fs::symlink_metadata(&full).expect("lstat");
            let mode = meta.permissions().mode() & 0o7777;
            let row = if meta.file_type().is_symlink() {
                let text = std::fs::read_link(&full).expect("readlink");
                ('l', mode, text.into_os_string().into_vec())
            } else if meta.is_dir() {
                todo.push(path.clone());
                ('d', mode, Vec::new())
            } else if meta.is_file() {
                let bytes =
                    std::fs::read(&full).unwrap_or_else(|why| panic!("{}: {why}", full.display()));
                ('f', mode, bytes)
            } else {
                ('o', mode, Vec::new())
            };
            out.insert(path, row);
        }
    }
    out
}

/// The inert tree a capture must produce: real 0700 dirs, plain 0600 files.
fn inert(files: &[(&str, &[u8])], dirs: &[&str]) -> Tree {
    let mut want = Tree::new();
    let mut dir_rows = |rel: &Path| {
        for dir in rel.ancestors().filter(|d| !d.as_os_str().is_empty()) {
            want.insert(dir.to_path_buf(), ('d', 0o700, Vec::new()));
        }
    };
    for (rel, _) in files {
        dir_rows(Path::new(rel).parent().unwrap_or(Path::new("")));
    }
    for rel in dirs {
        dir_rows(Path::new(rel));
    }
    for (rel, bytes) in files {
        want.insert(PathBuf::from(rel), ('f', 0o600, bytes.to_vec()));
    }
    want
}

fn put(dir: &Path, rel: &str, bytes: &[u8]) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("parent dirs");
    std::fs::write(&path, bytes).unwrap_or_else(|why| panic!("{rel}: {why}"));
}

fn chmod(path: &Path, mode: u32) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
}

fn end(rig: &Rig) -> (Option<i32>, String, String) {
    rig.run_with_env(
        &[
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ],
        &["_end", "-f", &rig.name],
    )
}

/// A second session keeps the isolated server answering once the target stops.
fn keepalive(rig: &Rig) {
    assert!(
        rig.tmux(&["new-session", "-d", "-s", "keepalive", "sh"]).0,
        "a second session keeps the server answering"
    );
}

/// A standing tombstone makes the teardown refuse AFTER the archive is published.
fn obstruct_teardown(rig: &Rig) -> PathBuf {
    let tomb = rig
        .home
        .join("sessions")
        .join(format!(".ending.{}", rig.name));
    std::fs::write(&tomb, b"obstruction").expect("a standing tombstone");
    tomb
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|why| panic!("{}: {why}", dir.display()))
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Every name the inventory files as RETAINED, by content.
const RETAINED: &[(&str, &[u8])] = &[
    ("meta.v1", b"session=older\n"),
    ("meta.tmp.4242", b"session=staged\n"),
    ("console.draft", b"a legacy draft"),
    ("seed.lead.md", b"# seed\n"),
    ("undelivered.lead.txt", b"a brief"),
    ("launch.main.prompt", b"a launch prompt"),
    ("opencode.main.json", b"{}"),
    ("opencode.main.md", b"# instructions\n"),
    ("telegram-outbound.cursor", b"42\n"),
    ("telegram-outbound.cursor.tmp.4242", b"43\n"),
    ("send", b"a regular file is not the helper\n"),
    ("_send-deliver", b"#!/bin/sh\nexit 0\n"),
    (".hidden", b"h"),
    ("empty", b""),
    ("foo.lock", b"not ae's lock"),
    ("foo.tmp", b"t"),
    ("foo.tmp.4242", b"u"),
    ("scripts/run.sh", b"#!/bin/sh\necho hi\n"),
    ("deep/a/b/c/d/e/f.bin", &[0, 255, 1, 254, 10]),
    ("messages/shot.png", &[137, 80, 78, 71, 13, 10, 26, 10, 0]),
    ("messages/note.md", b"not a request body"),
    ("messages/nested/deep/x.txt", b"a nested body"),
];
const RETAINED_DIRS: &[&str] = &["emptydir", "messages/emptydir", "deep/a/b/c/d/e/g"];

/// Every regular name the inventory files as REGISTRATION-ONLY.
const REGISTRATION: &[&str] = &[
    "workspace.md",
    "codex.main.sid",
    "launch.main.started",
    "meta.lock",
    "memo.tsv.lock",
    "events.jsonl.lock",
    ".console-admission.lock",
    ".console-writer.lock",
    ".pane-view.lock",
    ".goal-setter.lock",
    ".watchdog.start.lock",
    ".watchdog-beat",
    "meta-agent-state.json",
    "seatcompact.lock",
    "auto-reseat.main.lock",
    ".send.tmp.4242",
    ".workspace.md.tmp.4242",
    ".launch-attempt.tmp.4242",
];

const ARCHIVE_TOP: &[&str] = &[
    "meta",
    "digest.md",
    "memo.tsv",
    "events.jsonl",
    "messages",
    "harness.tsv",
    "retained",
];

#[test]
fn retain_end_captures_every_node_inert_and_drops_only_registration() {
    let rig = Rig::new("rt1");
    let dir = &rig.dir;
    for (rel, bytes) in RETAINED {
        put(dir, rel, bytes);
    }
    for rel in RETAINED_DIRS {
        std::fs::create_dir_all(dir.join(rel)).expect("an empty dir");
    }
    chmod(&dir.join("_send-deliver"), 0o700);
    chmod(&dir.join("scripts/run.sh"), 0o755);
    chmod(&dir.join("foo.tmp"), 0o444);
    let draft = format!("console.draft.{UUID}");
    put(dir, &draft, b"a session draft");
    put(dir, "messages/a.txt", b"a request body");
    for rel in REGISTRATION {
        put(dir, rel, b"");
    }
    put(dir, &format!("console.draft.{UUID}.lock"), b"");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("a clock")
        .as_secs();
    put(dir, ".launch-attempt", format!("{now}\n").as_bytes());
    let mut want = inert(RETAINED, RETAINED_DIRS);
    want.insert(
        PathBuf::from(&draft),
        ('f', 0o600, b"a session draft".to_vec()),
    );
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt;
        let raw = std::ffi::OsStr::from_bytes(b"raw\xffname");
        std::fs::write(dir.join(raw), [7, 8, 9]).expect("a non-UTF-8 name");
        want.insert(PathBuf::from(raw), ('f', 0o600, vec![7, 8, 9]));
    }

    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(!exists(dir), "the live state is gone");
    let archive = rig.archive();
    assert_eq!(
        tree(&archive.join("retained")),
        want,
        "retained/ holds exactly every non-registration node, inert, by its own relative name"
    );
    let top = names(&archive);
    assert!(top.iter().any(|n| n == "retained"), "{top:?}");
    for name in &top {
        assert!(
            ARCHIVE_TOP.contains(&name.as_str()),
            "unexpected top-level {name}"
        );
    }
    assert_eq!(
        names(&archive.join("messages")),
        ["a.txt"],
        "only direct regular *.txt bodies keep the existing messages/ path"
    );
    assert_eq!(
        std::fs::read(archive.join("messages/a.txt")).expect("the body"),
        b"a request body"
    );
    let whole = tree(&archive);
    for (path, (kind, mode, _)) in &whole {
        assert!(
            (*kind == 'd' && *mode == 0o700) || (*kind == 'f' && *mode == 0o600),
            "inert archive node {}: {kind} {mode:o}",
            path.display()
        );
    }
    let (code, out, err) = rig.run_with_env(
        &[(
            "CONFIG_FILE",
            rig.home.join("config").to_str().expect("utf-8"),
        )],
        &["list", "--ended"],
    );
    assert_eq!(code, Some(0), "{err}");
    assert!(
        out.contains(UUID),
        "the retained archive validates: {out}\n{err}"
    );
}

#[test]
fn retain_helper_named_links_are_registration_only_and_retained_is_created_empty() {
    let rig = Rig::new("rt2");
    let dir = &rig.dir;
    let live_target = rig.home.join("core-bin");
    std::fs::write(&live_target, b"the target of a live link").expect("a link target");
    for (i, helper) in ae::shim::HELPERS.iter().enumerate() {
        let target = if i == 0 {
            live_target.clone()
        } else {
            PathBuf::from("/nonexistent/ae-wt/gone/target/debug/ae")
        };
        symlink(&target, dir.join(helper.name)).expect("a helper link");
    }
    symlink("/nonexistent/old/core", dir.join("_send-deliver")).expect("a retired helper link");
    let versioned = rig.home.join("versions").join("2026.1.1").join("ae");
    symlink(&versioned, dir.join("oldhelper")).expect("an older helper link");
    let normalized = rig.home.join("versions/x/../2026.1.1/ae");
    symlink(&normalized, dir.join("oldhelper2")).expect("a lexically in-root link");

    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(!exists(dir), "the live state is gone");
    let retained = rig.archive().join("retained");
    let meta = std::fs::symlink_metadata(&retained).expect("retained/ exists even when empty");
    assert!(meta.is_dir(), "retained/ is a real directory");
    assert_eq!(meta.permissions().mode() & 0o7777, 0o700);
    assert!(
        tree(&retained).is_empty(),
        "a helper-named or (lexically normalized) versioned link is registration"
    );
    assert_eq!(
        std::fs::read(&live_target).expect("the link target survives"),
        b"the target of a live link",
        "removing a link never touches its target"
    );
}

/// End refuses `shape` before it captures anything, naming the node.
fn refuses(i: usize, shape: &str) {
    let rig = Rig::new(&format!("rf{i}"));
    put(&rig.dir, "notes/keep.txt", b"keep me");
    let mut unowned = None;
    let node = match shape {
        "nested-link" => {
            let node = rig.dir.join("messages/nested/ln");
            std::fs::create_dir_all(rig.dir.join("messages/nested")).expect("dirs");
            symlink("/etc/hosts", &node).expect("a nested link");
            node
        }
        "unknown-link" => {
            let node = rig.dir.join("weird");
            symlink(rig.home.join("elsewhere"), &node).expect("an unknown link");
            node
        }
        "escaped-link" | "lookalike-link" => {
            let rel = if shape == "escaped-link" {
                "versions/../outside/ae"
            } else {
                "versions-lookalike/2026.1.1/ae"
            };
            let target = rig.home.join(rel);
            let physical = rel.trim_start_matches("versions/../");
            let known = rig.home.join(physical);
            std::fs::create_dir_all(rig.home.join("versions")).expect("a versions dir");
            put(&rig.home, physical, b"unowned");
            assert_eq!(
                std::fs::read(&target).expect("the link text resolves before End"),
                b"unowned"
            );
            let node = rig.dir.join("oldhelper");
            symlink(&target, &node).expect("an out-of-root link");
            unowned = Some(known);
            node
        }
        "socket" => {
            let node = rig.dir.join("sock");
            // Bound in a short owned root (room proven by OwnedScratch::root).
            let scratch = OwnedScratch::root("sock", &i.to_string());
            let short = scratch.path().join("s");
            drop(std::os::unix::net::UnixListener::bind(&short).expect("a socket"));
            std::fs::rename(&short, &node).expect("the socket in place");
            node
        }
        _ => {
            let node = rig.dir.join("locked");
            std::fs::write(&node, b"secret").expect("a file");
            chmod(&node, 0o600);
            node
        }
    };
    let before = tree(&rig.dir);
    if shape == "unreadable" {
        chmod(&node, 0o000);
        if std::fs::read(&node).is_ok() {
            eprintln!("SKIPPED: a privileged runner reads a mode-000 file; not exercised");
            chmod(&node, 0o600);
            return;
        }
    }
    let (code, out, err) = end(&rig);
    if shape == "unreadable" {
        // Restore access so the literal bytes and plain kind can be compared.
        let _ = std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o600));
    }
    assert_eq!(code, Some(1), "{shape}: stdout: {out}\nstderr: {err}");
    assert!(
        err.contains(&node.display().to_string()) && err.contains("remove it, then ae end"),
        "{shape}: the refusal names the node and the remedy: {err}"
    );
    let after = tree(&rig.dir);
    for (path, row) in &before {
        assert_eq!(
            after.get(path),
            Some(row),
            "{shape}: {} kept",
            path.display()
        );
    }
    if let Some(known) = unowned {
        let meta = std::fs::symlink_metadata(&known).expect("the link target is kept");
        assert!(meta.is_file(), "{shape}: the target stays a plain file");
        assert_eq!(
            std::fs::read(&known).expect("the target reads"),
            b"unowned",
            "{shape}: a link target is never touched"
        );
    }
    assert!(!exists(&rig.archive()), "{shape}: no archive");
    let root = rig.home.join("archive");
    let claims = if root.is_dir() {
        names(&root)
    } else {
        Vec::new()
    };
    assert!(
        claims.iter().all(|n| !n.starts_with(".publishing.")),
        "{shape}: no claim left: {claims:?}"
    );
}

#[test]
fn retain_end_refuses_a_nested_link_and_deletes_nothing() {
    refuses(0, "nested-link");
}

#[test]
fn retain_end_refuses_an_unknown_direct_link_and_deletes_nothing() {
    refuses(1, "unknown-link");
}

#[test]
fn retain_end_refuses_a_socket_and_deletes_nothing() {
    refuses(2, "socket");
}

#[test]
fn retain_end_refuses_an_unreadable_file_and_keeps_its_bytes() {
    refuses(3, "unreadable");
}

#[test]
fn retain_end_refuses_an_old_link_escaping_versions_by_dot_dot() {
    refuses(4, "escaped-link");
}

#[test]
fn retain_end_refuses_an_old_link_under_a_versions_prefix_lookalike() {
    refuses(5, "lookalike-link");
}

#[test]
fn retain_end_leaves_legacy_state_paths_in_place_and_names_them() {
    let rig = Rig::new("rt3");
    let nested = rig.home.join(".ae").join(&rig.name);
    let worktree = rig
        .home
        .join("worktrees")
        .join(&rig.name)
        .join(".ae")
        .join(&rig.name);
    put(&nested, "keep.txt", b"origin legacy");
    put(&worktree, "keep.txt", b"worktree legacy");
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(!exists(&rig.dir), "the live state is gone");
    for (path, bytes) in [
        (&nested, &b"origin legacy"[..]),
        (&worktree, &b"worktree legacy"[..]),
    ] {
        assert_eq!(
            std::fs::read(path.join("keep.txt")).ok().as_deref(),
            Some(bytes),
            "{} is left in place",
            path.display()
        );
        let both = format!("{out}{err}");
        assert!(
            both.contains(&path.display().to_string()),
            "End names the kept legacy path {}: {both}",
            path.display()
        );
    }
}

#[test]
fn retain_list_ended_accepts_only_an_inert_retained_tree() {
    let rig = Rig::new("rv1");
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    let retained = rig.archive().join("retained");
    std::fs::create_dir_all(retained.join("notes")).expect("a retained dir");
    chmod(&retained, 0o700);
    chmod(&retained.join("notes"), 0o700);
    let file = retained.join("notes/a.txt");
    std::fs::write(&file, b"inert").expect("a retained file");
    chmod(&file, 0o600);
    let config = rig.home.join("config");
    let listed = |why: &str, want: bool| {
        let env = [("CONFIG_FILE", config.to_str().expect("utf-8"))];
        let (code, out, err) = rig.run_with_env(&env, &["list", "--ended"]);
        assert_eq!(code, Some(0), "{why}: {err}");
        assert_eq!(out.contains(UUID), want, "{why}: {out}\n{err}");
    };
    listed("an inert retained tree", true);
    chmod(&file, 0o700);
    listed("an executable retained file", false);
    chmod(&file, 0o600);
    chmod(&retained.join("notes"), 0o755);
    listed("a 0755 retained dir", false);
    chmod(&retained.join("notes"), 0o700);
    symlink("/etc/hosts", retained.join("ln")).expect("a link");
    listed("a link inside retained/", false);
    std::fs::remove_file(retained.join("ln")).expect("the link removed");
    listed("the restored inert tree", true);
}

/// A stopped, recordless session under a pre-grammar name, ended on the
/// human's `--assume-stopped` acknowledgement; `meta` says whether it has one.
fn pregrammar(tag: &str, meta: bool) -> (Scratch, PathBuf, (Option<i32>, String, String)) {
    let scratch = Scratch::new(OwnedScratch::root("lc", tag).keep());
    let home = scratch.home.clone();
    let name = "old.one";
    let dir = home.join("sessions").join(name);
    put(&dir, "notes.txt", b"the only copy");
    if meta {
        let text = format!(
            "session={name}\nsession_id={UUID}\nwork_dir={}\nmode=local\nschema=2\n",
            home.display()
        );
        put(&dir, "meta", text.as_bytes());
    }
    std::fs::create_dir_all(home.join("tmux")).expect("a private tmux dir");
    std::fs::write(home.join("config"), "").expect("a config");
    let mut cmd = ae();
    cmd.env("AE_HOME", &home)
        .env("CONFIG_FILE", home.join("config"))
        .env("TMUX_TMPDIR", home.join("tmux"))
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", home.join("tmux").join("s"))
        .env("AE_NO_AUTOSTART", "1")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .args(["_end", "-f", "--assume-stopped", name]);
    let out = bounded(
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("the ae binary should run"),
        Duration::from_secs(30),
    )
    .expect("the end returned");
    let run = (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    );
    (scratch, dir, run)
}

#[test]
fn retain_a_pregrammar_name_gets_the_same_strict_snapshot() {
    let (scratch, dir, (code, out, err)) = pregrammar("rt4", true);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(!exists(&dir), "the live state is gone");
    let retained = scratch.home.join("archive").join(UUID).join("retained");
    assert_eq!(
        tree(&retained),
        inert(&[("notes.txt", b"the only copy")], &[]),
        "a grammar-invalid name is captured before its cleanup"
    );
}

#[test]
fn retain_a_metaless_session_with_retained_evidence_refuses_and_deletes_nothing() {
    let (scratch, dir, (code, out, err)) = pregrammar("rt5", false);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert!(
        err.contains(&dir.display().to_string()),
        "the refusal names the session dir: {err}"
    );
    assert_eq!(
        std::fs::read(dir.join("notes.txt")).ok().as_deref(),
        Some(&b"the only copy"[..]),
        "nothing was deleted"
    );
    assert!(
        !exists(&scratch.home.join("archive").join(UUID)),
        "no archive"
    );
}

/// Publish the archive, let the teardown refuse, and return the archive bytes.
fn half_ended(rig: &Rig) -> Tree {
    keepalive(rig);
    put(&rig.dir, "notes/a.txt", b"retained once");
    put(&rig.dir, "memo.tsv", b"x\tremember\n");
    put(&rig.dir, "messages/a.txt", b"a body");
    let tomb = obstruct_teardown(rig);
    let (code, out, err) = end(rig);
    assert_eq!(code, Some(1), "the teardown refused: {out}{err}");
    assert!(exists(&rig.dir.join("meta")), "the registration is kept");
    let first = tree(&rig.archive());
    std::fs::remove_file(&tomb).expect("the obstruction cleared");
    // A later archived_at is a comparison substitution, never drift.
    std::thread::sleep(Duration::from_millis(1100));
    first
}

/// The first attempt published the full snapshot, retained/ included.
fn held_the_snapshot(first: &Tree) {
    assert_eq!(
        first.get(Path::new("retained/notes/a.txt")),
        Some(&('f', 0o600, b"retained once".to_vec())),
        "the first attempt published retained/"
    );
}

#[test]
fn retain_a_direct_unchanged_retry_reuses_its_archive_byte_for_byte() {
    let rig = Rig::new("rr1");
    let first = half_ended(&rig);
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert_eq!(
        tree(&rig.archive()),
        first,
        "the archive is reused, never rewritten"
    );
    assert!(!exists(&rig.dir), "the retry finished the teardown");
    held_the_snapshot(&first);
}

/// Rewrite one live meta row (appended when absent); every other row is kept.
fn set_meta(path: &Path, key: &str, value: &str) {
    let text = std::fs::read_to_string(path).expect("a meta");
    let prefix = format!("{key}=");
    let mut rows: Vec<String> = text
        .lines()
        .filter(|l| !l.starts_with(&prefix))
        .map(str::to_owned)
        .collect();
    rows.push(format!("{prefix}{value}"));
    std::fs::write(path, rows.join("\n") + "\n").expect("a rewritten meta");
}

/// A retry after `drift` (one cause, otherwise valid) refuses, names both
/// paths, and changes nothing.
fn drifted(i: usize, drift: &str) {
    use super::super::cli::git_in;
    let (rig, wt) = if drift == "git" {
        // No origin: End keeps the worktree on base and change alike.
        let (rig, wt, proj) = managed(&format!("rd{i}"));
        git_in(&proj, &["remote", "remove", "origin"]);
        (rig, Some(wt))
    } else {
        (Rig::new(&format!("rd{i}")), None)
    };
    let mut first = half_ended(&rig);
    match drift {
        "memo" => put(&rig.dir, "memo.tsv", b"x\tremember\ny\tlater\n"),
        "message" => put(&rig.dir, "messages/b.txt", b"a later body"),
        "retained-changed" => put(&rig.dir, "notes/a.txt", b"retained twice"),
        "retained-added" => put(&rig.dir, "notes/b.txt", b"new"),
        "retained-removed" => std::fs::remove_file(rig.dir.join("notes/a.txt")).expect("gone"),
        // Stable composed meta: source_goal.
        "goal" => set_meta(&rig.dir.join("meta"), "goal", "a later goal"),
        // harness.tsv alone: a current conversation appears.
        "harness" => set_meta(
            &rig.dir.join("meta"),
            "harness_session.main",
            "44444444-4444-4444-4444-444444444444",
        ),
        // Source identity: the archive at this UUID names another session.
        "source" => {
            let meta = rig.archive().join("meta");
            let text = std::fs::read_to_string(&meta).expect("archive meta");
            let from = format!("source_session={}\n", rig.name);
            assert!(text.contains(&from), "{text}");
            let text = text.replace(&from, "source_session=lcelse\n");
            std::fs::write(&meta, text).expect("another source");
            first = tree(&rig.archive());
        }
        // Independently derived git final/range: a later commit.
        _ => {
            let wt = wt.as_ref().expect("a worktree");
            assert!(exists(wt), "the worktree outlived the half end");
            put(wt, "later.txt", b"later work");
            git_in(wt, &["add", "-A"]);
            git_in(wt, &["commit", "-q", "-m", "later"]);
        }
    }
    let live = tree(&rig.dir);
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(1), "{drift}: stdout: {out}\nstderr: {err}");
    assert!(
        err.contains(&rig.archive().display().to_string())
            && err.contains(&rig.dir.display().to_string()),
        "{drift}: the refusal names the archive and the live dir: {err}"
    );
    assert_eq!(tree(&rig.archive()), first, "{drift}: the archive is exact");
    let after = tree(&rig.dir);
    for (path, row) in &live {
        assert_eq!(
            after.get(path),
            Some(row),
            "{drift}: {} kept",
            path.display()
        );
    }
    held_the_snapshot(&first);
}

#[test]
fn retain_a_retry_after_memo_drift_refuses() {
    drifted(0, "memo");
}

#[test]
fn retain_a_retry_after_a_new_message_refuses() {
    drifted(1, "message");
}

#[test]
fn retain_a_retry_after_a_changed_retained_file_refuses() {
    drifted(2, "retained-changed");
}

#[test]
fn retain_a_retry_after_an_added_retained_file_refuses() {
    drifted(3, "retained-added");
}

#[test]
fn retain_a_retry_after_a_removed_retained_file_refuses() {
    drifted(4, "retained-removed");
}

#[test]
fn retain_a_retry_after_a_stable_meta_change_refuses() {
    drifted(5, "goal");
}

#[test]
fn retain_a_retry_after_a_harness_change_refuses() {
    drifted(6, "harness");
}

#[test]
fn retain_a_retry_against_another_sources_archive_refuses() {
    drifted(7, "source");
}

#[test]
fn retain_a_retry_after_a_later_commit_refuses() {
    drifted(8, "git");
}

/// Channel: the detached supervisor journals its inner End's first `Error:`
/// line as the `FAILED:` end-result (`end.rs::run_supervised`), so the drift
/// refusal's both-path line is what that record must carry.
#[test]
fn retain_a_supervised_retry_refuses_on_its_own_end_request_and_leaves_the_archive_exact() {
    let rig = Rig::new("rs1");
    let first = half_ended(&rig);
    let (code, out, err) = rig.run(&["_end", "--handoff", &rig.name]);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    let mut result = None;
    for _ in 0..400 {
        let events = std::fs::read_to_string(rig.dir.join("events.jsonl")).unwrap_or_default();
        let after = events
            .split_once("\"action\":\"end-request\"")
            .map(|(_, t)| t);
        result = after
            .and_then(|t| t.lines().find(|l| l.contains("\"action\":\"end-result\"")))
            .map(str::to_owned);
        if result.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let result = result.expect("the supervisor journaled a result after its end-request");
    assert!(
        result.contains("FAILED:")
            && result.contains(&rig.archive().display().to_string())
            && result.contains(&rig.dir.display().to_string()),
        "the END_REQUEST drift refusal, naming both paths, is the recorded cause: {result}"
    );
    assert!(exists(&rig.dir.join("meta")), "the registration is kept");
    assert_eq!(tree(&rig.archive()), first, "the archive is exact");
}

/// A git-mode session whose managed worktree has an `origin` remote.
fn managed(tag: &str) -> (Rig, PathBuf, PathBuf) {
    use super::super::cli::git_in;
    let rig = Rig::new(tag);
    let remote = rig.home.join("remote.git");
    let proj = rig.home.join("proj");
    std::fs::create_dir_all(&remote).expect("remote");
    std::fs::create_dir_all(&proj).expect("proj");
    git_in(&remote, &["init", "-q", "--bare"]);
    git_in(&proj, &["init", "-q", "-b", "main"]);
    git_in(&proj, &["config", "user.name", "t"]);
    git_in(&proj, &["config", "user.email", "t@t"]);
    put(&proj, "a.txt", b"tracked\n");
    put(&proj, ".gitignore", b"target/\n");
    git_in(&proj, &["add", "-A"]);
    git_in(&proj, &["commit", "-q", "-m", "base"]);
    let remote_text = remote.display().to_string();
    git_in(&proj, &["remote", "add", "origin", &remote_text]);
    git_in(&proj, &["push", "-q", "origin", "main"]);
    let wt = rig.home.join("worktrees").join(&rig.name);
    std::fs::create_dir_all(rig.home.join("worktrees")).expect("worktrees root");
    let wt_text = wt.display().to_string();
    let branch = format!("ae/{}", rig.name);
    git_in(
        &proj,
        &["worktree", "add", "-q", "-b", &branch, &wt_text, "main"],
    );
    let meta = rig.dir.join("meta");
    let home = rig.home.display().to_string();
    let text: String = std::fs::read_to_string(&meta)
        .expect("rig meta")
        .lines()
        .map(|line| match line {
            "mode=local" => "mode=git\n".to_owned(),
            l if l == format!("work_dir={home}") => format!("work_dir={wt_text}\n"),
            l if l == format!("origin={home}") => format!("origin={}\n", proj.display()),
            l => format!("{l}\n"),
        })
        .collect();
    std::fs::write(&meta, text).expect("a git-mode meta");
    (rig, wt, proj)
}

#[test]
fn retain_a_clean_pushed_worktree_is_removed() {
    let (rig, wt, _) = managed("wk0");
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(!exists(&wt), "a provably lossless worktree is removed");
    assert!(!exists(&rig.dir), "the live state is gone");
}

/// A worktree holding `dirt` that a push would not carry is kept and named.
fn kept(i: usize, dirt: &str) {
    use super::super::cli::git_in;
    let (rig, wt, proj) = managed(&format!("wk{i}"));
    let (rel, bytes): (&str, &[u8]) = match dirt {
        "ignored" => ("target/out.bin", &[0, 1, 2]),
        "repo-excluded" => {
            let exclude = proj.join(".git/info/exclude");
            let mut text = std::fs::read_to_string(&exclude).unwrap_or_default();
            text.push_str("scratch.out\n");
            std::fs::write(&exclude, text).expect("a repo-local exclude");
            ("scratch.out", b"local only")
        }
        "mode-masked" => {
            git_in(&proj, &["config", "core.fileMode", "false"]);
            ("a.txt", b"tracked\n")
        }
        "assume-unchanged" | "skip-worktree" => {
            let flag = format!("--{dirt}");
            git_in(&wt, &["update-index", &flag, "a.txt"]);
            ("a.txt", b"edited, hidden from status\n")
        }
        _ => {
            git_in(&proj, &["config", "status.showUntrackedFiles", "no"]);
            ("new.txt", b"untracked")
        }
    };
    put(&wt, rel, bytes);
    if dirt == "mode-masked" {
        chmod(&wt.join("a.txt"), 0o755);
    }
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(0), "{dirt}: stdout: {out}\nstderr: {err}");
    assert!(!exists(&rig.dir), "{dirt}: the live state is gone");
    if dirt == "untracked-hidden" {
        // A status setting must not hide it: committed and pushed, or kept.
        let pushed = git_in(
            &rig.home.join("remote.git"),
            &["log", "--all", "--name-only", "--format="],
        );
        assert!(
            pushed.lines().any(|l| l == "new.txt")
                || std::fs::read(wt.join(rel)).ok().as_deref() == Some(bytes),
            "{dirt}: the untracked file is never lost"
        );
        return;
    }
    assert_eq!(
        std::fs::read(wt.join(rel)).ok().as_deref(),
        Some(bytes),
        "{dirt}: the worktree is kept with its data"
    );
    if dirt == "mode-masked" {
        let mode = std::fs::metadata(wt.join("a.txt"))
            .expect("a.txt")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755, "{dirt}: the masked mode survives");
    }
    assert!(
        format!("{out}{err}").contains(&wt.display().to_string()),
        "{dirt}: End names the kept worktree: {out}{err}"
    );
    if dirt == "ignored" {
        launch_refuses_the_kept_name(&rig, &wt, &proj);
    }
}

#[test]
fn retain_a_worktree_with_ignored_output_is_kept_and_its_name_refused() {
    kept(1, "ignored");
}

#[test]
fn retain_a_worktree_with_repo_excluded_data_is_kept() {
    kept(2, "repo-excluded");
}

#[test]
fn retain_a_worktree_with_a_config_masked_mode_change_is_kept() {
    kept(3, "mode-masked");
}

#[test]
fn retain_a_worktree_with_an_assume_unchanged_edit_is_kept() {
    kept(4, "assume-unchanged");
}

#[test]
fn retain_a_worktree_with_a_skip_worktree_edit_is_kept() {
    kept(5, "skip-worktree");
}

#[test]
fn retain_an_untracked_file_hidden_by_status_config_is_never_lost() {
    kept(6, "untracked-hidden");
}

#[test]
fn retain_a_same_name_launch_over_a_kept_worktree_names_it_and_the_remedy() {
    let (rig, wt, proj) = managed("wk7");
    keepalive(&rig);
    assert!(rig.tmux(&["kill-session", "-t", &rig.name]).0, "stopped");
    std::fs::remove_dir_all(&rig.dir).expect("the registration is gone");
    put(&wt, "target/out.bin", &[0, 1, 2]);
    launch_refuses_the_kept_name(&rig, &wt, &proj);
}

/// The same-name managed launch refuses the kept worktree with the owner text.
fn launch_refuses_the_kept_name(rig: &Rig, wt: &Path, proj: &Path) {
    let mut cmd = ae();
    cmd.current_dir(proj)
        .env("PWD", proj)
        .env("AE_HOME", &rig.home)
        .env("CONFIG_FILE", rig.home.join("config"))
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", &rig.sock)
        .env("AE_NO_AUTOSTART", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .args([rig.name.as_str(), "--worktree", "--no-attach"]);
    let out = bounded(
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("the ae binary should run"),
        Duration::from_secs(30),
    )
    .expect("the launch returned");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "the launch refuses: {err}");
    assert!(
        err.contains(&wt.display().to_string())
            && err
                .contains("choose a fresh name or remove/relocate the preserved workdir yourself"),
        "the refusal names the kept path and the remedy: {err}"
    );
    assert_eq!(
        std::fs::read(wt.join("target/out.bin")).ok().as_deref(),
        Some(&[0, 1, 2][..]),
        "the refused launch leaves the kept worktree alone"
    );
    assert!(!exists(&rig.dir), "the refused launch registers nothing");
}
