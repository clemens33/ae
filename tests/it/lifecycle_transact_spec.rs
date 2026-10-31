//! Frozen acceptance (#248 P1b-2 TRANSACT, Batch B): an ordinary End holds
//! every cooperating writer out from its verified stop through the canonical
//! unregister, refuses at once on a busy session-scoped guard, converges an
//! unchanged direct or supervised retry after a publish whose teardown failed,
//! appends nothing after a valid strict publish, and on true drift deletes
//! nothing, keeps the archive exact and names the bounded manual recovery.
//!
//! Oracle: `plan-p1b2.md` r3 lines 7-8 and 14-26, `p1b2-locks-r3.md` with its
//! frozen r2 table, lead ACK 0d071dfb. Real `_end` and `_end --handoff`, a
//! private tmux socket, hermetic git. The two `supervised` cases replace the
//! P1b-1 pin `retain_a_supervised_retry_refuses_on_its_own_end_request_and_
//! leaves_the_archive_exact` (plan line 5).
//!
//! Text contract: a drift refusal names the archive, the live dir and the
//! configured sessions root, and says `outside` and `fresh name`
//! (case-insensitive); a no-origin End says `no origin remote to push to`
//! beside the kept workdir; a post-publish failure tells every attached client
//! the session name, the archive path and the live dir.

use super::super::cli::git_in;
use super::super::support::try_observe;
use super::guard_spec::{
    CURSOR_LOCK, INSTANCE, MUTATION, Tree, end, half_ended, hold_for, keepalive, obstruct_teardown,
    put, tree,
};
use super::retain_spec::managed;
use super::*;

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

fn both_paths(rig: &Rig, err: &str, what: &str) {
    assert!(
        err.contains(&rig.archive().display().to_string())
            && err.contains(&rig.dir.display().to_string()),
        "{what}: the refusal names the archive and the live dir: {err}"
    );
}

fn claim(rig: &Rig) -> PathBuf {
    rig.home.join("archive").join(format!(".publishing.{UUID}"))
}

/// A first End refused on a busy guard: nothing published, payload kept, and
/// once the guard is free the same End completes.
fn busy(i: usize, lock: &str) {
    let rig = Rig::new(&format!("tz{i}"));
    keepalive(&rig);
    put(&rig.dir, "notes/a.txt", b"payload");
    put(&rig.dir, "memo.tsv", b"x\tremember\n");
    let path = rig.dir.join(lock);
    let held = ae::store::lock(&path, Duration::ZERO).expect("a cooperating holder");
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(1), "{lock}: stdout: {out}\nstderr: {err}");
    assert!(
        err.contains(&path.display().to_string()),
        "{lock}: the refusal names the busy guard: {err}"
    );
    assert!(!exists(&rig.archive()), "{lock}: nothing published");
    assert!(!exists(&claim(&rig)), "{lock}: no claim left");
    assert_eq!(
        std::fs::read(rig.dir.join("notes/a.txt")).expect("kept"),
        b"payload"
    );
    assert_eq!(
        std::fs::read(rig.dir.join("memo.tsv")).expect("kept"),
        b"x\tremember\n"
    );
    assert!(exists(&rig.dir.join("meta")), "{lock}: still registered");
    drop(held);
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(0), "{lock}: once free: {out}{err}");
    assert!(!exists(&rig.dir), "{lock}: the retry completed");
}

#[test]
fn transact_end_refuses_on_a_busy_app_writer_lease() {
    busy(0, ".console-writer.lock");
}

#[test]
fn transact_end_refuses_on_a_busy_console_admission() {
    busy(1, ".console-admission.lock");
}

#[test]
fn transact_end_refuses_on_a_busy_goal_setter() {
    busy(2, ".goal-setter.lock");
}

#[test]
fn transact_end_refuses_on_a_busy_draft_mutation_guard() {
    busy(3, MUTATION);
}

#[test]
fn transact_end_refuses_on_a_busy_cursor_checkpoint() {
    busy(4, CURSOR_LOCK);
}

#[test]
fn transact_source_locks_keep_their_bounded_wait() {
    let rig = Rig::new("tq1");
    keepalive(&rig);
    put(&rig.dir, "notes/a.txt", b"payload");
    let held = ae::store::lock(&rig.dir.join("memo.tsv.lock"), Duration::ZERO).expect("held");
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(1), "a memo writer past the wait: {out}{err}");
    assert!(!exists(&rig.archive()), "nothing published");
    assert_eq!(
        std::fs::read(rig.dir.join("notes/a.txt")).expect("kept"),
        b"payload"
    );
    drop(held);
    inside_the_wait(&rig, "meta.lock");
    // Each source wait gets its own End, so no wait can hide another.
    let other = Rig::new("tq1m");
    keepalive(&other);
    inside_the_wait(&other, "memo.tsv.lock");
}

/// One source lock held 1.5 s, inside the 5 s wait: End waits and completes.
fn inside_the_wait(rig: &Rig, name: &str) {
    let holder = hold_for(&rig.dir.join(name), Duration::from_millis(1500));
    let (code, out, err) = end(rig);
    holder.join().expect("the holder");
    assert_eq!(code, Some(0), "{name} held inside the wait: {out}{err}");
    assert!(!exists(&rig.dir), "{name}: the session is unregistered");
}

#[test]
fn transact_neither_a_held_view_nor_an_idle_draft_lifetime_blocks_end() {
    let rig = Rig::new("tv1");
    let view = ae::store::open(&rig.dir)
        .pane_view()
        .expect("a view holds its lock");
    let idle = ae::store::DraftRecord::open(&rig.dir, INSTANCE).expect("an idle app");
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(!exists(&rig.dir), "the live state is gone");
    let retained = rig.archive().join("retained");
    for lock in [
        ".pane-view.lock".to_owned(),
        format!("console.draft.{INSTANCE}.lock"),
    ] {
        assert!(
            !exists(&retained.join(&lock)),
            "{lock} is registration-only"
        );
    }
    drop(view);
    // A same-name session comes back; the stale record writes nothing there.
    put(
        &rig.dir,
        "meta",
        format!(
            "session={}\nsession_id=44444444-4444-4444-4444-444444444444\n",
            rig.name
        )
        .as_bytes(),
    );
    let replacement = tree(&rig.dir);
    assert!(idle.publish(b"late").is_err(), "a stale app refuses");
    kept_all(
        &replacement,
        &rig.dir,
        "no byte or node from the old incarnation",
    );
}

/// Run a retry End while the fixture, as an outer journal appender, holds
/// `events.jsonl.lock`. Proven phase: End created `.console-writer.lock` (its
/// first hold) and then holds `memo.tsv.lock` (taken just before the journal);
/// `during` runs there, the journal is released `held` later. Returns End's
/// result and the instant the journal was released.
fn paused_at_the_journal(
    rig: &Rig,
    held: Duration,
    during: impl FnOnce(),
) -> (
    (Option<i32>, String, String),
    std::time::Instant,
    std::time::Instant,
) {
    let writer = rig.dir.join(".console-writer.lock");
    let _ = std::fs::remove_file(&writer);
    let memo = rig.dir.join("memo.tsv.lock");
    let journal = ae::store::lock(&rig.dir.join("events.jsonl.lock"), Duration::ZERO)
        .expect("the outer appender holds the journal");
    std::thread::scope(|scope| {
        let ending = scope.spawn(|| {
            let ended = end(rig);
            (ended, std::time::Instant::now())
        });
        let mut entered = false;
        for _ in 0..1200 {
            if exists(&writer) {
                entered = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(entered, "End entered its holds while the journal was held");
        let mut holds_memo = false;
        for _ in 0..1000 {
            if ae::store::lock(&memo, Duration::ZERO).is_err() {
                holds_memo = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(holds_memo, "End holds the memo, waiting on the journal");
        during();
        std::thread::sleep(held);
        let released = std::time::Instant::now();
        drop(journal);
        let (ended, finished) = ending.join().expect("the End");
        (ended, released, finished)
    })
}

/// lead 3b: an outer appender holding the journal while End sits inside its
/// holds; End waits its bounded wait and completes after the release.
#[test]
fn transact_an_outer_journal_appender_mid_transaction_neither_deadlocks_nor_appends() {
    let rig = Rig::new("tj1");
    let first = half_ended(&rig);
    let ((code, out, err), released, finished) =
        paused_at_the_journal(&rig, Duration::from_millis(1500), || {});
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(
        finished > released,
        "End finished only after the outer appender let go"
    );
    assert!(!exists(&rig.dir), "the retry unregistered");
    assert_eq!(tree(&rig.archive()), first, "the archive is exact");
}

/// A real cooperating writer (the stopped session's own `_memo` helper)
/// queued on the memo while End holds it: it fails loudly and lands nowhere.
#[test]
fn transact_a_post_stop_memo_writer_queued_on_the_holds_lands_nowhere() {
    let rig = Rig::new("tw1");
    let first = half_ended(&rig);
    let dir = rig.dir.display().to_string();
    let late = std::thread::scope(|scope| {
        let mut helper = None;
        let ((code, out, err), _, _) = paused_at_the_journal(&rig, Duration::from_secs(2), || {
            helper = Some(scope.spawn(|| rig.run(&["_memo", &dir, "add", "a late memo"])));
        });
        assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
        helper
            .expect("the helper started")
            .join()
            .expect("the helper")
    });
    assert_ne!(late.0, Some(0), "the late writer fails loudly: {late:?}");
    assert!(!exists(&rig.dir), "the late writer recreated nothing");
    assert_eq!(tree(&rig.archive()), first, "nothing landed in the archive");
}

#[test]
fn transact_a_pushed_retry_with_its_workdir_already_removed_only_unregisters() {
    let (rig, wt, proj) = managed("tp1");
    let first = half_ended(&rig);
    assert!(!exists(&wt), "the lossless workdir went first");
    // A retry that fetched or pushed would fail here.
    git_in(
        &proj,
        &["remote", "set-url", "origin", "/nonexistent/ae-origin.git"],
    );
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(!exists(&rig.dir), "canonical unregister");
    assert_eq!(tree(&rig.archive()), first, "the archive is exact");
}

/// A pushed managed session whose workdir is kept (ignored output).
fn kept_pushed(tag: &str, later: bool) {
    let (rig, wt, _) = managed(tag);
    put(&wt, "target/out.bin", b"ignored output");
    let first = half_ended(&rig);
    assert!(
        exists(&wt.join("target/out.bin")),
        "the unprovable workdir is kept"
    );
    if later {
        put(&wt, "later.txt", b"later work");
        git_in(&wt, &["add", "-A"]);
        git_in(&wt, &["commit", "-q", "-m", "later"]);
    }
    let live = tree(&rig.dir);
    let trace = rig.home.join("git-trace2.jsonl");
    let (code, out, err) = rig.run_with_env(
        &[
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_TRACE2_EVENT", trace.to_str().expect("utf-8")),
        ],
        &["_end", "-f", &rig.name],
    );
    // Every git child End started, read-only proof allowed.
    let started: Vec<String> = std::fs::read_to_string(&trace)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.contains("\"event\":\"cmd_name\""))
        .filter_map(|l| l.split("\"name\":\"").nth(1)?.split('"').next())
        .map(str::to_owned)
        .collect();
    if later {
        assert_eq!(code, Some(1), "changed git facts: {out}{err}");
        both_paths(&rig, &err, "later commit");
        kept_all(&live, &rig.dir, "later commit");
        assert!(!started.is_empty(), "the witness sees End's git children");
    } else {
        assert_eq!(code, Some(0), "no repeat git operation: {out}{err}");
        assert!(!exists(&rig.dir), "the unchanged retry converged");
        for forbidden in [
            "commit",
            "commit-tree",
            "fetch",
            "fetch-pack",
            "pull",
            "push",
        ] {
            assert!(
                !started.iter().any(|c| c == forbidden),
                "no repeat {forbidden}: {started:?}"
            );
        }
        for network in ["send-pack", "upload-pack"] {
            assert!(
                !started.iter().any(|c| c == network),
                "{network}: {started:?}"
            );
        }
    }
    assert_eq!(tree(&rig.archive()), first, "the archive is exact");
    assert_eq!(
        std::fs::read(wt.join("target/out.bin")).expect("kept"),
        b"ignored output"
    );
}

#[test]
fn transact_an_unchanged_retry_over_a_kept_pushed_workdir_runs_no_git() {
    kept_pushed("tk1", false);
}

#[test]
fn transact_a_retry_after_a_later_commit_in_a_kept_pushed_workdir_refuses() {
    kept_pushed("tk2", true);
}

#[test]
fn transact_a_no_origin_retry_whose_kept_workdir_was_removed_refuses() {
    let (rig, wt, proj) = managed("tn1");
    git_in(&proj, &["remote", "remove", "origin"]);
    let first = half_ended(&rig);
    assert!(exists(&wt), "a no-origin workdir is kept");
    std::fs::remove_dir_all(&wt).expect("a human removed it");
    let live = tree(&rig.dir);
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert!(err.contains(&wt.display().to_string()), "{err}");
    assert_eq!(tree(&rig.archive()), first, "the archive is exact");
    kept_all(&live, &rig.dir, "removed kept workdir");
}

#[test]
fn transact_a_no_origin_end_says_why_its_workdir_is_kept() {
    let (rig, wt, proj) = managed("to1");
    git_in(&proj, &["remote", "remove", "origin"]);
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(
        out.contains("no origin remote to push to") && out.contains(&wt.display().to_string()),
        "the reason and the kept path: {out}"
    );
    assert!(exists(&wt), "kept");
}

#[test]
fn transact_a_standing_claim_refuses_and_is_never_stolen() {
    let rig = Rig::new("tc1");
    keepalive(&rig);
    put(&rig.dir, "notes/a.txt", b"payload");
    let standing = claim(&rig);
    put(&standing, "partial", b"another publisher");
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    assert!(err.contains(&standing.display().to_string()), "{err}");
    assert!(!exists(&rig.archive()), "nothing published");
    assert_eq!(
        std::fs::read(standing.join("partial")).expect("never stolen"),
        b"another publisher"
    );
    assert_eq!(
        std::fs::read(rig.dir.join("notes/a.txt")).expect("kept"),
        b"payload"
    );
}

#[test]
fn transact_a_retry_whose_sources_change_after_its_early_check_refuses() {
    let rig = Rig::new("tf1");
    let first = half_ended(&rig);
    // Registration-only; its creation marks End entering its holds.
    let writer = rig.dir.join(".console-writer.lock");
    let _ = std::fs::remove_file(&writer);
    let held = ae::store::lock(&rig.dir.join("memo.tsv.lock"), Duration::ZERO).expect("held");
    let (code, out, err) = std::thread::scope(|scope| {
        let ending = scope.spawn(|| end(&rig));
        let mut entered = false;
        for _ in 0..1200 {
            if exists(&writer) {
                // The queued memo writer lands its row, then lets go.
                put(&rig.dir, "memo.tsv", b"x\tremember\ny\tqueued\n");
                entered = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(held);
        let ended = ending.join().expect("the End");
        assert!(
            entered,
            "the early inspection never waits on a queued source lock: {ended:?}"
        );
        ended
    });
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    both_paths(&rig, &err, "the held re-decision saw the drift");
    assert_eq!(tree(&rig.archive()), first, "no fabricated second attempt");
    assert!(exists(&rig.dir.join("notes/a.txt")), "nothing deleted");
    assert!(exists(&rig.dir.join("meta")), "still registered");
}

#[test]
fn transact_a_drift_refusal_names_the_recovery_outside_sessions() {
    let rig = Rig::new("td1");
    let first = half_ended(&rig);
    put(&rig.dir, "memo.tsv", b"x\tremember\ny\tlater\n");
    let live = tree(&rig.dir);
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(1), "stdout: {out}\nstderr: {err}");
    both_paths(&rig, &err, "drift");
    let lower = err.to_lowercase();
    assert!(
        err.contains(&rig.home.join("sessions").display().to_string())
            && lower.contains("outside")
            && lower.contains("fresh name"),
        "move the stopped dir outside the sessions root, restart under a fresh name: {err}"
    );
    assert_eq!(tree(&rig.archive()), first, "the archive is exact");
    kept_all(&live, &rig.dir, "drift");
}

/// A wide client on the keepalive session: its pane shows every message.
fn watcher(rig: &Rig) -> String {
    assert!(rig.tmux(&["set-option", "-g", "display-time", "10000"]).0);
    let attach = format!(
        "env -u TMUX tmux -S {} attach -t =keepalive",
        rig.sock.display()
    );
    let start = [
        "new-session",
        "-d",
        "-s",
        "watch",
        "-x",
        "1000",
        "-y",
        "5",
        attach.as_str(),
    ];
    assert!(rig.tmux(&start).0, "the watching client starts");
    // One 5 s budget for both reads. The client is the witness: the watch
    // pane exists from `new-session`, and existing is not attached.
    let deadline = Instant::now() + Duration::from_secs(5);
    try_observe(
        |args| rig.tmux(args),
        &["list-clients", "-F", "#{session_name}"],
        deadline,
        |clients| clients.lines().any(|s| s == "keepalive"),
    )
    .unwrap_or_else(|why| panic!("no client attached to keepalive: {why}"));
    first_pane(|args| rig.tmux(args), &["-t", "=watch"], deadline)
}

fn noticed(rig: &Rig, pane: &str, needles: &[&str]) -> bool {
    for _ in 0..600 {
        let (_, screen) = rig.tmux(&["capture-pane", "-p", "-t", pane]);
        if screen
            .lines()
            .any(|l| needles.iter().all(|n| l.contains(n)))
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

fn handoff(rig: &Rig) {
    let (code, out, err) = rig.run(&["_end", "--handoff", &rig.name]);
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
}

#[test]
fn transact_a_supervised_end_whose_teardown_fails_after_publishing_appends_nothing() {
    let rig = Rig::new("ts1");
    keepalive(&rig);
    let pane = watcher(&rig);
    put(&rig.dir, "notes/a.txt", b"retained once");
    let tomb = obstruct_teardown(&rig);
    handoff(&rig);
    let (archive, dir) = (rig.archive(), rig.dir.display().to_string());
    let shown = archive.display().to_string();
    assert!(
        noticed(&rig, &pane, &[&rig.name, &shown, &dir]),
        "the attached human is told the session, archive and live dir"
    );
    std::thread::sleep(Duration::from_secs(2));
    let events = std::fs::read_to_string(rig.dir.join("events.jsonl")).unwrap_or_default();
    assert!(!events.contains("\"action\":\"end-result\""), "{events}");
    assert!(exists(&rig.dir.join("meta")), "still registered");
    let first = tree(&archive);
    std::fs::remove_file(&tomb).expect("the obstruction cleared");
    std::thread::sleep(Duration::from_millis(1100));
    let (code, out, err) = end(&rig);
    assert_eq!(code, Some(0), "the direct retry converges: {out}{err}");
    assert!(!exists(&rig.dir));
    assert_eq!(tree(&archive), first, "the archive is exact");
}

#[test]
fn transact_an_unchanged_supervised_retry_converges_and_appends_nothing() {
    let rig = Rig::new("ts2");
    let first = half_ended(&rig);
    handoff(&rig);
    for _ in 0..400 {
        if !exists(&rig.dir) {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(!exists(&rig.dir), "the supervised retry converged");
    assert_eq!(
        tree(&rig.archive()),
        first,
        "the first request stays archived and nothing was appended"
    );
}

#[test]
fn transact_a_drifted_supervised_retry_appends_and_deletes_nothing_and_says_so() {
    let rig = Rig::new("ts3");
    let first = half_ended(&rig);
    let pane = watcher(&rig);
    put(&rig.dir, "memo.tsv", b"x\tremember\ny\tlater\n");
    let live = tree(&rig.dir);
    handoff(&rig);
    let (archive, dir) = (
        rig.archive().display().to_string(),
        rig.dir.display().to_string(),
    );
    assert!(
        noticed(&rig, &pane, &[&archive, &dir]),
        "the attached human is told both paths"
    );
    std::thread::sleep(Duration::from_secs(2));
    kept_all(&live, &rig.dir, "supervised drift");
    assert_eq!(tree(&rig.archive()), first, "the archive is exact");
    let config = rig.home.join("config");
    let (code, out, err) = rig.run_with_env(
        &[("CONFIG_FILE", config.to_str().expect("utf-8"))],
        &["list", "--all"],
    );
    assert_eq!(code, Some(0), "{err}");
    assert!(out.contains(&rig.name), "still listed: {out}");
}
