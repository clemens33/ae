//! Frozen acceptance (#263 option 1 REBOOTKEEP): `ae reboot` reuses End's
//! retained-file inventory and transactional retention, so seed, undelivered
//! briefs, console draft, launch prompts and the other non-archived regular
//! files End keeps survive a reboot inertly, with `reason=reboot`, and an
//! unchanged retry reuses that archive while drift, an uncapturable node, a
//! busy End activity guard or an incomplete recovery point refuse before
//! teardown and leave the live session intact.
//!
//! Oracle: `.local/brief-fix7.md` rebootkeep (human option 1) plus the lead
//! clarifications (the recovery-step diagnostic names what `ae reboot` runs
//! next; existing incomplete archives fail closed; the existing End activity
//! locks are shared through final comparison and teardown, no second
//! inventory). Real `_compact` on a private tmux socket, digest-only plus
//! exec-plan so the boundary is crossed without a relaunch. Text contract: a
//! drift refusal names the archive, the live dir and the sessions root and
//! says `outside` and `fresh name` (case-insensitive); every refusal says what
//! the human runs next.

use super::guard_spec::{CURSOR_LOCK, MUTATION, Tree, keepalive, obstruct_teardown, put, tree};
use super::*;

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|why| panic!("{}: {why}", dir.display()))
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// `_compact -f --digest-only --exec-plan <plan>`: the whole reboot without a
/// handover wait and without a relaunch, crossing the boundary only.
fn reboot(rig: &Rig, plan: &Path) -> (Option<i32>, String, String) {
    rig.run(&[
        "_compact",
        rig.name.as_str(),
        "-f",
        "--digest-only",
        "--exec-plan",
        plan.to_str().expect("plan path"),
    ])
}

fn claim(rig: &Rig) -> PathBuf {
    rig.home.join("archive").join(format!(".publishing.{UUID}"))
}

/// Every node of `before` is still there, byte for byte; anything new is an
/// empty `*.lock` a refused attempt may leave.
fn kept_all(before: &Tree, dir: &Path, what: &str) {
    let after = tree(dir);
    for (path, row) in before {
        assert_eq!(
            after.get(path),
            Some(row),
            "{what}: {} kept",
            path.display()
        );
    }
    for (path, (kind, _, bytes)) in &after {
        let name = path.to_string_lossy();
        assert!(
            before.contains_key(path)
                || (name.ends_with(".lock") && *kind == 'f' && bytes.is_empty()),
            "{what}: unexpected new node {name}"
        );
    }
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

/// The four stdout contract lines, in order, and nothing else.
fn boundary(rig: &Rig, out: &str) {
    assert_eq!(
        out.lines().count(),
        4,
        "reboot stdout is exactly the boundary: {out:?}"
    );
    assert!(
        out.starts_with(&format!("Archived {UUID}\n")),
        "the boundary opens with the archive id: {out:?}"
    );
    assert!(
        out.contains(&format!("Archive: {}\n", rig.archive().display())),
        "the boundary names the archive: {out:?}"
    );
    assert!(
        out.contains(&format!("Digest: {}/digest.md\n", rig.archive().display())),
        "the boundary names the digest: {out:?}"
    );
    assert!(
        out.contains("--from"),
        "the boundary names recovery: {out:?}"
    );
}

/// The issue-named files plus a deep node, an empty dir and both messages
/// paths. Registration-only names are never in this set.
const KEPT: &[(&str, &[u8])] = &[
    ("seed.lead.md", b"# seed\n"),
    ("undelivered.lead.txt", b"a brief"),
    ("launch.main.prompt", b"a launch prompt"),
    ("notes/a.txt", b"retained once"),
    ("deep/a/b/c/d.bin", &[0, 255, 1, 254, 10]),
    ("messages/shot.png", &[137, 80, 78, 71, 13, 10, 26, 10, 0]),
    ("messages/note.md", b"not a request body"),
];
const KEPT_DIRS: &[&str] = &["emptydir"];
const REGISTRATION: &[&str] = &[
    "workspace.md",
    "codex.main.sid",
    "launch.main.started",
    "meta.lock",
    "memo.tsv.lock",
    "events.jsonl.lock",
];

#[test]
fn reboot_keeps_every_retained_file_as_an_inert_reboot_archive() {
    let rig = Rig::new("rk1");
    let dir = &rig.dir;
    for (rel, bytes) in KEPT {
        put(dir, rel, bytes);
    }
    for rel in KEPT_DIRS {
        std::fs::create_dir_all(dir.join(rel)).expect("an empty dir");
    }
    let draft = format!("console.draft.{UUID}");
    put(dir, &draft, b"a session draft");
    put(dir, "messages/a.txt", b"a request body");
    for rel in REGISTRATION {
        put(dir, rel, b"");
    }
    let mut want = inert(KEPT, KEPT_DIRS);
    want.insert(
        PathBuf::from(&draft),
        ('f', 0o600, b"a session draft".to_vec()),
    );

    let plan = rig.home.join("plan");
    let (code, out, err) = reboot(&rig, &plan);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    boundary(&rig, &out);
    assert!(!exists(dir), "the live state is gone");
    assert!(!rig.session_is_live(), "the recorded tmux session stopped");
    let archive = rig.archive();
    let retained = archive.join("retained");
    assert!(retained.is_dir(), "a reboot archive carries retained/");
    assert_eq!(
        tree(&retained),
        want,
        "retained/ holds exactly every non-registration node, inert, by its own relative name"
    );
    for name in names(&archive) {
        assert!(
            [
                "meta",
                "digest.md",
                "memo.tsv",
                "events.jsonl",
                "messages",
                "harness.tsv",
                "retained"
            ]
            .contains(&name.as_str()),
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
    let meta = std::fs::read_to_string(archive.join("meta")).expect("archive meta");
    assert!(
        meta.contains("reason=reboot"),
        "the reboot archive keeps its reason: {meta}"
    );
    assert!(
        meta.contains(&format!("source_session={}\n", rig.name)),
        "the archive names its source: {meta}"
    );
    assert!(!exists(&claim(&rig)), "no claim left");
    let config = rig.home.join("config");
    let (code, out, err) = rig.run_with_env(
        &[("CONFIG_FILE", config.to_str().expect("utf-8"))],
        &["list", "--ended"],
    );
    assert_eq!(code, Some(0), "{err}");
    assert!(out.contains(UUID), "the archive validates: {out}\n{err}");
    let (code, _, err) = rig.run(&[
        "_archive-from-preflight",
        rig.home.join("archive").to_str().expect("archive root"),
        UUID,
    ]);
    assert_eq!(code, Some(0), "the archive can be inherited: {err}");
}

#[test]
fn reboot_retained_tree_matches_end_retained_tree() {
    let rebooted = Rig::new("rk2a");
    for (rel, bytes) in KEPT {
        put(&rebooted.dir, rel, bytes);
    }
    let draft = format!("console.draft.{UUID}");
    put(&rebooted.dir, &draft, b"a session draft");
    put(&rebooted.dir, "messages/a.txt", b"a request body");
    let plan = rebooted.home.join("plan");
    let (code, out, err) = reboot(&rebooted, &plan);
    assert_eq!(code, Some(0), "reboot: stdout: {out}\nstderr: {err}");

    let archived = Rig::new("rk2b");
    for (rel, bytes) in KEPT {
        put(&archived.dir, rel, bytes);
    }
    put(&archived.dir, &draft, b"a session draft");
    put(&archived.dir, "messages/a.txt", b"a request body");
    let (code, out, err) = super::guard_spec::end(&archived);
    assert_eq!(code, Some(0), "end: stdout: {out}\nstderr: {err}");

    assert!(
        rebooted.archive().join("retained").is_dir(),
        "a reboot archive carries retained/"
    );
    let reboot_retained = tree(&rebooted.archive().join("retained"));
    let end_retained = tree(&archived.archive().join("retained"));
    assert_eq!(
        reboot_retained, end_retained,
        "reboot reuses End's inventory: the same files land the same way"
    );
    assert_eq!(
        reboot_retained.get(Path::new("seed.lead.md")),
        Some(&('f', 0o600, b"# seed\n".to_vec())),
        "the seed the issue names survives a reboot"
    );
}

#[test]
fn reboot_refuses_a_node_it_cannot_capture_and_leaves_the_session_usable() {
    use std::os::unix::fs::symlink;
    let rig = Rig::new("rk3");
    put(&rig.dir, "notes/keep.txt", b"keep me");
    let node = rig.dir.join("weird");
    symlink(rig.home.join("elsewhere"), &node).expect("an unknown link");
    let before = tree(&rig.dir);

    let plan = rig.home.join("plan");
    let (code, out, err) = reboot(&rig, &plan);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert!(out.is_empty(), "nothing crossed the boundary: {out:?}");
    assert!(
        err.contains(&node.display().to_string()),
        "the refusal names the uncapturable node: {err}"
    );
    assert!(
        err.contains("ae reboot"),
        "the refusal names the step the human runs next: {err}"
    );
    assert!(!exists(&rig.archive()), "no archive");
    assert!(!exists(&claim(&rig)), "no claim left");
    kept_all(&before, &rig.dir, "the uncapturable node keeps the session");
    assert!(exists(&rig.dir.join("meta")), "still registered");
}

/// One busy End activity guard refuses the reboot before teardown.
fn busy(i: usize, lock: &str) {
    let rig = Rig::new(&format!("rk4-{i}"));
    keepalive(&rig);
    put(&rig.dir, "notes/a.txt", b"payload");
    put(&rig.dir, "seed.lead.md", b"# seed\n");
    let path = rig.dir.join(lock);
    let held = ae::store::lock(&path, Duration::ZERO).expect("a cooperating holder");
    let plan = rig.home.join("plan");
    let (code, out, err) = reboot(&rig, &plan);
    assert_eq!(code, Some(1), "{lock}: stdout: {out}\nstderr: {err}");
    assert!(out.is_empty(), "{lock}: nothing crossed the boundary");
    assert!(
        err.contains(&path.display().to_string()),
        "{lock}: the refusal names the busy guard: {err}"
    );
    assert!(
        err.contains("ae reboot"),
        "{lock}: the refusal names the step the human runs next: {err}"
    );
    assert!(!exists(&rig.archive()), "{lock}: nothing published");
    assert!(!exists(&claim(&rig)), "{lock}: no claim left");
    assert_eq!(
        std::fs::read(rig.dir.join("notes/a.txt")).expect("kept"),
        b"payload"
    );
    assert_eq!(
        std::fs::read(rig.dir.join("seed.lead.md")).expect("kept"),
        b"# seed\n"
    );
    assert!(exists(&rig.dir.join("meta")), "{lock}: still registered");
    drop(held);
}

#[test]
fn reboot_refuses_on_a_busy_console_writer() {
    busy(0, ".console-writer.lock");
}

#[test]
fn reboot_refuses_on_a_busy_console_admission() {
    busy(1, ".console-admission.lock");
}

#[test]
fn reboot_refuses_on_a_busy_goal_setter() {
    busy(2, ".goal-setter.lock");
}

#[test]
fn reboot_refuses_on_a_busy_draft_mutation_guard() {
    busy(3, MUTATION);
}

#[test]
fn reboot_refuses_on_a_busy_cursor_checkpoint() {
    busy(4, CURSOR_LOCK);
}

/// Publish the reboot archive, let the teardown refuse, return the archive
/// bytes. The session is stopped; the live dir is kept.
fn half_rebooted(rig: &Rig) -> Tree {
    keepalive(rig);
    put(&rig.dir, "notes/a.txt", b"retained once");
    put(&rig.dir, "seed.lead.md", b"# seed\n");
    put(&rig.dir, "memo.tsv", b"x\tremember\n");
    put(&rig.dir, "messages/a.txt", b"a body");
    let tomb = obstruct_teardown(rig);
    let plan = rig.home.join("plan1");
    let (code, out, err) = reboot(rig, &plan);
    assert_eq!(code, Some(1), "the teardown refused: {out}{err}");
    assert!(out.is_empty(), "nothing crossed the boundary");
    assert!(exists(&rig.dir.join("meta")), "the registration is kept");
    let first = tree(&rig.archive());
    assert_eq!(
        first.get(Path::new("retained/notes/a.txt")),
        Some(&('f', 0o600, b"retained once".to_vec())),
        "the first attempt published retained/"
    );
    assert_eq!(
        first.get(Path::new("retained/seed.lead.md")),
        Some(&('f', 0o600, b"# seed\n".to_vec())),
        "the first attempt published the seed the issue names"
    );
    std::fs::remove_file(&tomb).expect("the obstruction cleared");
    // A later archived_at is a comparison substitution, never drift.
    std::thread::sleep(Duration::from_millis(1100));
    first
}

#[test]
fn reboot_unchanged_retry_reuses_its_archive_without_rewriting_it() {
    let rig = Rig::new("rk5");
    let first = half_rebooted(&rig);
    let plan = rig.home.join("plan2");
    let (code, out, err) = reboot(&rig, &plan);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    boundary(&rig, &out);
    assert_eq!(
        tree(&rig.archive()),
        first,
        "the archive is reused, never rewritten"
    );
    assert!(!exists(&rig.dir), "the retry finished the teardown");
    assert!(!exists(&claim(&rig)), "no claim left");
}

#[test]
fn reboot_retry_after_retained_drift_refuses_and_keeps_both_exact() {
    let rig = Rig::new("rk6");
    let first = half_rebooted(&rig);
    put(&rig.dir, "notes/a.txt", b"retained twice");
    let live = tree(&rig.dir);
    let plan = rig.home.join("plan2");
    let (code, out, err) = reboot(&rig, &plan);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert!(out.is_empty(), "nothing crossed the boundary");
    assert!(
        err.contains(&rig.archive().display().to_string())
            && err.contains(&rig.dir.display().to_string()),
        "the refusal names the archive and the live dir: {err}"
    );
    let lower = err.to_lowercase();
    assert!(
        lower.contains("outside") && lower.contains("fresh name"),
        "the refusal names the bounded recovery: {err}"
    );
    assert!(
        err.contains("ae reboot"),
        "the refusal names the step the human runs next: {err}"
    );
    assert_eq!(tree(&rig.archive()), first, "the archive is exact");
    kept_all(&live, &rig.dir, "retained drift keeps the live session");
    assert!(!exists(&claim(&rig)), "no claim left");
}

#[test]
fn reboot_teardown_refuses_an_incomplete_recovery_point_and_keeps_the_live_session() {
    let rig = Rig::new("rk7");
    keepalive(&rig);
    put(&rig.dir, "notes/a.txt", b"retained once");
    put(&rig.dir, "seed.lead.md", b"# seed\n");
    put(&rig.dir, "memo.tsv", b"x\tremember\n");
    put(&rig.dir, "messages/a.txt", b"a body");
    let tomb = obstruct_teardown(&rig);
    let plan = rig.home.join("plan1");
    let (code, out, err) = reboot(&rig, &plan);
    assert_eq!(code, Some(1), "the teardown refused: {out}{err}");
    std::fs::remove_file(&tomb).expect("the obstruction cleared");
    // The published archive becomes an old incomplete recovery point.
    let _ = std::fs::remove_dir_all(rig.archive().join("retained"));
    let live = tree(&rig.dir);
    let (code, tuple_out, err) =
        rig.run(&["_compact-freeze", rig.dir.to_str().expect("session dir")]);
    assert_eq!(code, Some(0), "frozen for the second stage: {err}");
    let tuple = tuple_out.trim_end().to_owned();
    assert!(!tuple.is_empty(), "a frozen tuple is emitted");
    let (code, out, err) = rig.run(&[
        "_compact-teardown",
        rig.dir.to_str().expect("session dir"),
        tuple.as_str(),
    ]);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert!(out.is_empty(), "nothing crossed the boundary");
    assert!(
        err.contains(&rig.archive().display().to_string())
            && err.contains(&rig.dir.display().to_string()),
        "the refusal names the archive and the live dir: {err}"
    );
    assert!(
        err.contains("ae reboot"),
        "the refusal names the step the human runs next: {err}"
    );
    kept_all(
        &live,
        &rig.dir,
        "the incomplete archive keeps the live session",
    );
}

/// A regular file the copy cannot read fails during staging, after the
/// inventory classified it: the refusal names the node and the next step,
/// publishes nothing and keeps every byte.
#[test]
fn reboot_refuses_an_unreadable_retained_file_and_keeps_its_bytes() {
    use std::os::unix::fs::PermissionsExt;
    let rig = Rig::new("rk8");
    put(&rig.dir, "notes/locked.txt", b"secret");
    let node = rig.dir.join("notes/locked.txt");
    std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o600)).expect("a live file");
    let before = tree(&rig.dir);
    std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o000)).expect("unreadable");
    if std::fs::read(&node).is_ok() {
        eprintln!("SKIPPED: a privileged runner reads a mode-000 file; not exercised");
        let _ = std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o600));
        return;
    }

    let plan = rig.home.join("plan");
    let (code, out, err) = reboot(&rig, &plan);
    let _ = std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o600));
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert!(out.is_empty(), "nothing crossed the boundary");
    assert!(
        err.contains(&node.display().to_string()),
        "the refusal names the unreadable node: {err}"
    );
    assert!(
        err.contains("ae reboot"),
        "the refusal names the step the human runs next: {err}"
    );
    assert!(!exists(&rig.archive()), "no archive");
    assert!(!exists(&claim(&rig)), "no claim left");
    kept_all(&before, &rig.dir, "the unreadable file keeps the session");
    assert!(exists(&rig.dir.join("meta")), "still registered");
}

/// Standalone teardown re-proves the retained bytes, not only their presence:
/// a live retained file changed after publishing refuses the teardown and
/// keeps both trees exact.
#[test]
fn reboot_teardown_refuses_a_retained_file_that_changed_after_publishing() {
    let rig = Rig::new("rk9");
    let first = half_rebooted(&rig);
    put(&rig.dir, "notes/a.txt", b"retained twice");
    let live = tree(&rig.dir);
    let (code, tuple_out, err) =
        rig.run(&["_compact-freeze", rig.dir.to_str().expect("session dir")]);
    assert_eq!(code, Some(0), "frozen for the second stage: {err}");
    let tuple = tuple_out.trim_end().to_owned();
    let (code, out, err) = rig.run(&[
        "_compact-teardown",
        rig.dir.to_str().expect("session dir"),
        tuple.as_str(),
    ]);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert!(out.is_empty(), "nothing crossed the boundary");
    assert!(
        err.contains(&rig.archive().display().to_string())
            && err.contains(&rig.dir.display().to_string()),
        "the refusal names the archive and the live dir: {err}"
    );
    let lower = err.to_lowercase();
    assert!(
        lower.contains("outside") && lower.contains("fresh name"),
        "the refusal names the bounded recovery: {err}"
    );
    assert!(
        err.contains("ae reboot"),
        "the refusal names the step the human runs next: {err}"
    );
    assert_eq!(tree(&rig.archive()), first, "the archive is exact");
    kept_all(&live, &rig.dir, "teardown drift keeps the live session");
}

/// While the publisher stages under its claim, the writer holds must still be
/// held: a probe inside the claim window must find them busy. The retained
/// tree is large on purpose, so staging lasts long enough to observe.
#[test]
fn reboot_keeps_writer_holds_through_staging() {
    let rig = Rig::new("rk10");
    const BIG: [u8; 16384] = [0x5a; 16384];
    for i in 0..300usize {
        put(&rig.dir, &format!("big/f{i}.bin"), &BIG);
    }
    put(&rig.dir, "seed.lead.md", b"# seed\n");

    let plan = rig.home.join("plan");
    let mut reboot = ae();
    reboot
        .env("AE_HOME", &rig.home)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .args([
            "_compact",
            rig.name.as_str(),
            "-f",
            "--digest-only",
            "--exec-plan",
            plan.to_str().expect("plan path"),
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = reboot.spawn().expect("the reboot starts");

    let claim = claim(&rig);
    let writer = rig.dir.join(".console-writer.lock");
    let mutation = rig.dir.join(MUTATION);
    let mut claimed = false;
    let mut busy = false;
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll the reboot") {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "the reboot did not finish"
        );
        if claim.exists() {
            claimed = true;
            if ae::store::lock(&writer, Duration::ZERO).is_err()
                && ae::store::lock(&mutation, Duration::ZERO).is_err()
            {
                busy = true;
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(status.code(), Some(0), "the reboot crossed the boundary");
    assert!(
        claimed,
        "the claim was observed: the fixture is not too fast"
    );
    assert!(busy, "activity holds are kept while the publisher stages");
}
