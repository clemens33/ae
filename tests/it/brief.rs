//! `ae brief`, black-box, against a planted state root.
//!
//! The card's whole claim is that it carries the REASONS across: what the
//! session is for, what was last decided under each memo topic, and who is
//! waiting on the human. So the arm here writes those facts the way the product
//! writes them — the memos through the shipped `memo` surface, the declaration
//! as the event the `state` helper appends — and then asserts the rendered card
//! whole, byte for byte. A per-field assertion would pass while the layout that
//! makes a card readable at a glance fell apart.
//!
//! No tmux and no live session: the planted server socket does not exist, so
//! liveness is `unknown` and every fact in the card comes off disk.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "fixtures build and inspect real directories with expect on the fixture \
              I/O; the capability boundary is about what PRODUCT code may reach"
)]

use std::fs;
use std::path::{Path, PathBuf};

use super::cli::ae;
use super::parity::Invocation;
use super::parity::capture::raw;
use super::phase2::{run_tmux, tmux_present};

/// A scratch state root, per-test.
fn scratch(tag: &str) -> PathBuf {
    let dir = PathBuf::from(format!("/tmp/ae-brief-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    assert!(
        fs::create_dir_all(dir.join("sessions")).is_ok(),
        "a scratch state root"
    );
    dir
}

/// A durable session ae will discover for itself, recording a server that does
/// not answer — so the fixture proves the card, never a live query.
fn plant(root: &Path, name: &str, work_dir: &Path) -> PathBuf {
    let dir = root.join("sessions").join(name);
    let server = root.join("no-server.sock");
    assert!(
        !server.exists(),
        "nothing may answer at {}",
        server.display()
    );
    assert!(fs::create_dir_all(&dir).is_ok(), "the session dir");
    assert!(
        fs::create_dir_all(work_dir.join(".git")).is_ok(),
        "the work dir"
    );
    // `session::branch_at` reads `.git/HEAD` itself rather than running git, so
    // the branch segment is a filesystem fact this fixture can state exactly.
    assert!(
        fs::write(
            work_dir.join(".git").join("HEAD"),
            "ref: refs/heads/s1-brief\n"
        )
        .is_ok(),
        "a HEAD to name the branch"
    );
    let meta = format!(
        "mode=local\nmeta_version=2\nsession={name}\nae_version=2026.9.5\nwork_dir={}\ngoal=ship S1 of #113\n\
         seat.main=lead\nprofile.main=cl\nseat.spawned.0=scribe\nprofile.spawned.0=cx\n\
         tmux_server_kind=socket\ntmux_server={}\n",
        work_dir.display(),
        server.display()
    );
    assert!(fs::write(dir.join("meta"), meta).is_ok(), "a planted meta");
    dir
}

/// Run the shipped binary over `root` with no pane identity.
fn run(root: &Path, tail: &[&str]) -> (Option<i32>, String, String) {
    let out = ae()
        // HOME too: the card shortens a work dir under it, and this fixture's
        // work dirs live in the state root.
        .env("HOME", root)
        .env("AE_HOME", root)
        .env_remove("TMUX_PANE")
        .args(tail)
        .output()
        .expect("the ae binary should run");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// One memo, written through the surface the product ships.
fn memo(dir: &Path, topic: &str, text: &str) {
    let out = ae()
        .env_remove("TMUX_PANE")
        .arg(ae::cli::MEMO)
        .arg(dir)
        .args(["add", "--topic", topic, text])
        .output()
        .expect("the ae binary should run");
    assert_eq!(out.status.code(), Some(0), "memo add {topic}");
}

/// An ISO-8601 instant `age_secs` in the past — the spelling every emitter uses.
fn ago(age_secs: i64) -> String {
    ae::time::Timestamp::from_epoch(ae::time::Timestamp::now().epoch() - age_secs).to_string()
}

/// Append one event, in [`ae::state::event_line`]'s own shape.
fn declare(dir: &Path, actor: &str, value: &str, reason: &str, age_secs: i64) {
    let ts = ae::time::Timestamp::from_epoch(ae::time::Timestamp::now().epoch() - age_secs);
    let line = ae::state::event_line(ts, actor, "state", value, reason);
    let existing = fs::read_to_string(dir.join("events.jsonl")).unwrap_or_default();
    assert!(
        fs::write(dir.join("events.jsonl"), existing + &line).is_ok(),
        "a planted declaration"
    );
}

#[test]
fn the_card_carries_the_goal_the_latest_record_per_topic_and_who_is_waiting() {
    let root = scratch("card");
    let work = root.join("work");
    let dir = plant(&root, "brf1", &work);

    // Two topics, and a SECOND record under one of them: the card must show the
    // later one, because a checkpoint supersedes the checkpoint before it.
    memo(&dir, "decision", "route the review through ae, never a CLI");
    memo(&dir, "parking", "resume here: the renderer is half written");
    memo(
        &dir,
        "decision",
        "gate once per merge, release after both land",
    );

    declare(&dir, "lead", "working", "wiring the dispatch", 180);
    declare(
        &dir,
        "scribe",
        "waiting-user",
        "which layout do you want",
        720,
    );

    let (code, stdout, stderr) = run(&root, &["brief", "brf1"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");

    // The memos were written seconds ago, so their ages are the only field this
    // fixture cannot state exactly; every other byte is asserted.
    let lines: Vec<&str> = stdout.lines().collect();
    let ages: Vec<String> = lines
        .iter()
        .filter(|line| line.starts_with("    decision") || line.starts_with("    parking"))
        .map(|line| {
            line.split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_owned()
        })
        .collect();
    assert_eq!(ages.len(), 2, "one line per topic: {stdout}");
    for age in &ages {
        assert!(
            age.ends_with('s'),
            "a memo written by this fixture is seconds old, not {age}: {stdout}"
        );
    }

    let expected = format!(
        "brf1 · unknown · attn:waiting-user · ae 2026.9.5 · s1-brief · ~/work\n\
         \x20 goal: ship S1 of #113\n\
         \x20 topics:\n\
         \x20   decision    {}    human         gate once per merge, release after both land\n\
         \x20   parking     {}    human         resume here: the renderer is half written\n\
         \x20 agents:\n\
         \x20   lead          working       3m    \"wiring the dispatch\"\n\
         \x20   scribe        waiting-user  12m   \"which layout do you want\"\n\
         \x20 needs you:\n\
         \x20   scribe        waiting-user  12m   which layout do you want\n",
        ages[0], ages[1],
    );
    assert_eq!(stdout, expected, "stderr: {stderr}");
}

#[test]
fn since_drops_the_older_topic_and_keeps_the_newer_one() {
    let root = scratch("since");
    let work = root.join("work");
    let dir = plant(&root, "brf2", &work);
    // Written directly: `--since` is about RECORD AGE, and the memo surface can
    // only write records dated now.
    let older = format!("{}\thuman\tparking\told checkpoint\n", ago(7_200));
    let newer = format!("{}\thuman\tdecision\tfresh call\n", ago(60));
    assert!(
        fs::write(dir.join("memo.tsv"), older + &newer).is_ok(),
        "a planted memo file"
    );

    let (code, all, _) = run(&root, &["brief", "brf2"]);
    assert_eq!(code, Some(0));
    assert!(
        all.contains("old checkpoint") && all.contains("fresh call"),
        "{all}"
    );

    let (code, recent, stderr) = run(&root, &["brief", "brf2", "--since", "30m"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(recent.contains("fresh call"), "{recent}");
    assert!(!recent.contains("old checkpoint"), "{recent}");
}

#[test]
fn a_session_with_nothing_recorded_says_so_in_every_section() {
    let root = scratch("empty");
    let work = root.join("work");
    plant(&root, "brf3", &work);

    let (code, stdout, stderr) = run(&root, &["brief", "brf3"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("  topics: none recorded\n"), "{stdout}");
    assert!(stdout.contains("  needs you: none recorded\n"), "{stdout}");
    // The roster is not empty, so `agents:` is not a "none recorded" section:
    // an agent that has declared nothing renders `-`, which is a different fact.
    assert!(stdout.contains("  agents:\n"), "{stdout}");
    assert!(stdout.contains("    lead          -"), "{stdout}");
}

#[test]
fn a_name_nobody_recorded_is_refused_and_an_unknown_flag_is_a_usage_error() {
    let root = scratch("refusals");
    plant(&root, "brf4", &root.join("work"));

    // THE INVENTORY HERE IS INCOMPLETE BY CONSTRUCTION — the planted server does
    // not answer — so ae may NOT claim the name is gone. It says what it
    // actually knows, and warns that the enumeration lost a source first.
    // `a_bare_brief_inside_a_session_cards_that_session_and_no_other` proves the
    // other half, where a live server makes absence provable.
    let (code, stdout, stderr) = run(&root, &["brief", "nope"]);
    assert_eq!(code, Some(1), "stderr: {stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.contains("inventory incomplete"), "{stderr}");
    assert!(
        stderr.contains("nope is in no session source ae could read"),
        "{stderr}"
    );
    assert!(!stderr.contains("no session named"), "{stderr}");

    for tail in [
        vec!["brief", "--frobnicate"],
        vec!["brief", "--since"],
        vec!["brief", "--since", "soon"],
        vec!["brief", "one", "two"],
    ] {
        let (code, stdout, stderr) = run(&root, &tail);
        assert_eq!(code, Some(2), "{tail:?}: {stderr}");
        assert!(stdout.is_empty(), "{tail:?}: {stdout}");
        assert!(stderr.contains("Usage: ae brief"), "{tail:?}: {stderr}");
    }
}

#[test]
fn all_cards_the_fleet_most_actionable_first() {
    let root = scratch("all");
    let quiet = plant(&root, "aquiet", &root.join("work-a"));
    let stuck = plant(&root, "zstuck", &root.join("work-z"));
    declare(&quiet, "lead", "working", "on it", 60);
    declare(&stuck, "lead", "blocked", "the runner is down", 60);

    let (code, stdout, stderr) = run(&root, &["brief", "--all"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let headers: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with(' '))
        .collect();
    assert_eq!(headers.len(), 2, "{stdout}");
    assert!(
        headers[0].starts_with("zstuck · unknown · attn:blocked"),
        "the blocked session leads, name order notwithstanding: {stdout}"
    );
    assert!(headers[1].starts_with("aquiet · unknown · ae"), "{stdout}");
}

#[test]
fn a_state_root_with_no_sessions_says_so_and_still_exits_zero() {
    let root = scratch("bare");
    let (code, stdout, stderr) = run(&root, &["brief", "--all"]);
    // Nothing to brief is a fact about the fleet, not a failure of the command.
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    assert_eq!(stderr, ae::brief::NOTHING);
}

/// One event, appended in the emitter's own shape.
fn event(dir: &Path, line: &str) {
    let existing = fs::read_to_string(dir.join("events.jsonl")).unwrap_or_default();
    assert!(
        fs::write(dir.join("events.jsonl"), existing + line + "\n").is_ok(),
        "a planted event"
    );
}

#[test]
fn an_open_ask_reaches_needs_you_and_leaves_it_when_it_is_answered() {
    // END TO END over the request sensor, not the renderer: the section has to
    // fill from a real ledger and EMPTY again when the ledger closes the row.
    // Asserting only the rendered `Need` would pass with the sensor unwired.
    let root = scratch("asks");
    let dir = plant(&root, "brf5", &root.join("work"));
    event(
        &dir,
        &format!(
            "{{\"ts\":\"{}\",\"actor\":\"lead\",\"action\":\"ask\",\"target\":\"colead\",\
             \"ref\":\"ae-7\",\"summary\":\"does the strip pin orchestrator first?\"}}",
            ago(2_460)
        ),
    );

    let (code, open, stderr) = run(&root, &["brief", "brf5"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        open.contains(
            "    ask           lead → colead 41m   does the strip pin orchestrator first?\n"
        ),
        "{open}"
    );

    // THE OPPOSED HALF: a reply closes the row, and the section goes back to
    // claiming nothing rather than keeping a stale question on screen.
    event(
        &dir,
        &format!(
            "{{\"ts\":\"{}\",\"actor\":\"colead\",\"action\":\"reply\",\"target\":\"lead\",\
             \"ref\":\"ae-7\",\"summary\":\"yes, first and never reordered\"}}",
            ago(60)
        ),
    );
    let (code, closed, stderr) = run(&root, &["brief", "brf5"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(!closed.contains("ae-7"), "{closed}");
    assert!(!closed.contains("orchestrator first"), "{closed}");
    assert!(closed.contains("  needs you: none recorded\n"), "{closed}");
}

#[test]
fn a_bare_brief_outside_tmux_falls_back_to_the_whole_fleet() {
    // The DEFAULT invocation, and its first failure branch: no $TMUX_PANE means
    // no caller to name, and the spec's fallback is the fleet — never nothing.
    let root = scratch("bare-out");
    plant(&root, "brfa", &root.join("work-a"));
    plant(&root, "brfb", &root.join("work-b"));

    let (code, stdout, stderr) = run(&root, &["brief"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let headers: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with(' '))
        .collect();
    assert_eq!(headers.len(), 2, "both sessions carded: {stdout}");
}

#[test]
fn a_bare_brief_whose_pane_no_server_answers_for_falls_back_too() {
    // The SECOND failure branch, and the one that matters for safety: a pane id
    // inherited from some other server must not be resolved against this one —
    // pane ids are small per-server integers, so a guess would card a stranger.
    let root = scratch("bare-stale");
    plant(&root, "brfa", &root.join("work-a"));
    plant(&root, "brfb", &root.join("work-b"));

    let out = ae()
        .env("HOME", &root)
        .env("AE_HOME", &root)
        .env("TMUX_PANE", "%99")
        .args(["brief"])
        .output()
        .expect("the ae binary should run");
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let headers = stdout
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with(' '))
        .count();
    assert_eq!(headers, 2, "an unanswerable pane names nobody: {stdout}");
}

/// Kill the arm's server and remove its scratch, WHATEVER ended the arm.
struct Cleanup {
    socket: PathBuf,
    scratch: PathBuf,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let out = self.scratch.join("cleanup-out");
        let err = self.scratch.join("cleanup-err");
        let invocation = Invocation::new("tmux")
            .arg("-S")
            .arg(&self.socket)
            .arg("kill-server");
        let _ = raw::run(&invocation, &self.scratch, &out, &err);
        let _ = fs::remove_dir_all(&self.scratch);
    }
}

#[test]
fn a_bare_brief_inside_a_session_cards_that_session_and_no_other() {
    // THE DEFAULT PATH, against a REAL server. A fixture that faked the pane
    // lookup would prove the fallback and nothing about the resolution that
    // makes `ae brief` worth typing bare.
    //
    // A short scratch: `sun_path` is 104 bytes on macOS and the usual temp dir
    // eats most of it.
    let dir = PathBuf::from(format!("/tmp/ae-brf-own-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    assert!(
        fs::create_dir_all(dir.join("sessions")).is_ok(),
        "a scratch"
    );
    if !tmux_present(&dir) {
        let _ = fs::remove_dir_all(&dir);
        panic!(
            "tmux is not runnable here, so the caller-resolution half of `ae brief` cannot be \
             proven; install tmux or run this suite where one exists"
        );
    }
    let socket = dir.join("t.sock");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: dir.clone(),
    };
    let tmux = |words: &[&str]| {
        let mut args = vec!["-S".to_owned(), socket.display().to_string()];
        args.extend(words.iter().map(|word| (*word).to_owned()));
        run_tmux(&args, &dir)
    };
    for name in ["mine", "other"] {
        assert!(tmux(&["new-session", "-d", "-s", name]).0, "a session");
        // The ae-ownership marker liveness reads: without it a live session is
        // `unknown`, and this arm would prove resolution against a status the
        // product would never show.
        assert!(
            tmux(&["set-environment", "-t", name, "AE_SESSION", name]).0,
            "the ownership marker"
        );
    }
    let (listed, panes) = tmux(&["list-panes", "-t", "mine", "-F", "#{pane_id}"]);
    assert!(listed, "the panes");
    let pane = panes.lines().next().unwrap_or_default().to_owned();
    assert!(pane.starts_with('%'), "a pane id: {panes}");

    for name in ["mine", "other"] {
        let session = dir.join("sessions").join(name);
        assert!(fs::create_dir_all(&session).is_ok(), "a session dir");
        let meta = format!(
            "mode=local\nmeta_version=2\nsession={name}\nseat.main=lead\nprofile.main=cl\n\
             tmux_server_kind=socket\ntmux_server={}\n",
            socket.display()
        );
        assert!(fs::write(session.join("meta"), meta).is_ok(), "a meta");
    }

    let out = ae()
        .env("HOME", &dir)
        .env("AE_HOME", &dir)
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", &socket)
        .env("TMUX_PANE", &pane)
        .arg("brief")
        .output()
        .expect("the ae binary should run");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    let headers: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with(' '))
        .collect();
    assert_eq!(headers.len(), 1, "one card, the caller's own: {stdout}");
    assert!(headers[0].starts_with("mine · running"), "{stdout}");
    assert!(
        !stdout.contains("other"),
        "the sibling was carded: {stdout}"
    );

    // AND THE COMPLETE-INVENTORY HALF of the absence claim: this server answers,
    // so ae may say a name is gone — and says it in those words.
    let out = ae()
        .env("HOME", &dir)
        .env("AE_HOME", &dir)
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", &socket)
        .env("TMUX_PANE", &pane)
        .args(["brief", "nope"])
        .output()
        .expect("the ae binary should run");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("ae brief: no session named nope"),
        "{stderr}"
    );
    assert!(!stderr.contains("inventory incomplete"), "{stderr}");
}
