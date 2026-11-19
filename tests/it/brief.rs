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
    let dir = super::cli::OwnedScratch::root("brief", tag).keep();
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

    declare(
        &dir,
        "lead",
        "waiting-user",
        "which layout do you want",
        720,
    );
    declare(
        &dir,
        "scribe",
        "waiting-user",
        "worker-only layout request",
        600,
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
    let active_age = lines
        .first()
        .and_then(|line| {
            line.split(" · ")
                .find_map(|part| part.strip_prefix("active "))
        })
        .unwrap_or_default();
    assert!(
        active_age.ends_with("s ago"),
        "fixture events were written seconds ago: {stdout}"
    );

    let expected = format!(
        "brf1 · unknown · attn:waiting-user · ae 2026.9.5 · created - · started - · active {active_age} · s1-brief · ~/work\n\
         \x20 goal: ship S1 of #113\n\
         \x20 topics:\n\
         \x20   decision    {}    human         gate once per merge, release after both land\n\
         \x20   parking     {}    human         resume here: the renderer is half written\n\
         \x20 agents:\n\
         \x20   lead          cl            waiting-user  12m   \"which layout do you want\"\n\
         \x20   scribe        cx            waiting-user  10m   \"worker-only layout request\"\n\
         \x20 needs you:\n\
         \x20   lead          waiting-user  12m   which layout do you want\n\
         \x20   scribe        waiting-user  10m   worker-only layout request\n",
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
    assert!(
        stdout.contains("    lead          cl            -"),
        "{stdout}"
    );
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
fn an_open_ask_stays_out_of_needs_you_and_counts_on_the_working_row() {
    // END TO END over the request sensor, not just a planted `Need`: the open
    // request becomes overview metadata while staying out of the human queue.
    let root = scratch("asks");
    let dir = plant(&root, "brf5", &root.join("work"));
    declare(&dir, "lead", "working", "review the strip", 180);
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
    assert!(open.contains("  needs you: none recorded\n"), "{open}");
    assert!(!open.contains("orchestrator first"), "{open}");

    let (_, world) = ae::current_world(&root);
    let entry = world
        .sessions
        .iter()
        .find(|entry| entry.name == "brf5")
        .expect("the planted session is discovered");
    let mut card = ae::brief::card_for(entry, &dir, Some(&root), false, world.now, None);
    card.status = "running";
    let overview = ae::overview::render(&[card], "orchestrator");
    assert!(
        overview.contains("brf5          lead          ship S1 of #113 (1 open ask)"),
        "{overview}"
    );
    assert!(!overview.contains("orchestrator first"), "{overview}");

    // THE OPPOSED HALF: a reply closes the count, and no stale body appears.
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

    let (_, world) = ae::current_world(&root);
    let entry = world
        .sessions
        .iter()
        .find(|entry| entry.name == "brf5")
        .expect("the planted session is discovered");
    let mut card = ae::brief::card_for(entry, &dir, Some(&root), false, world.now, None);
    card.status = "running";
    let overview = ae::overview::render(&[card], "orchestrator");
    assert!(!overview.contains("open ask"), "{overview}");
}

#[test]
fn a_retired_request_is_absent_from_the_brief_needs() {
    // A retire closes by routing-key seat, not by request id. The brief has the
    // session name from its `SessionEntry`, so it must give that same identity
    // to the owner rather than use the session-less compatibility sensor.
    let root = scratch("retired-request");
    let dir = plant(&root, "brf6", &root.join("work"));
    event(
        &dir,
        r#"{"ts":"2026-08-20T16:12:55Z","actor":"lead","action":"ask","target":"scribe","ref":"ae-7","actor_slot":"main","actor_session":"brf6","target_slot":"spawned.0","target_session":"brf6","summary":"does the retire close this?"}"#,
    );
    event(
        &dir,
        r#"{"ts":"2026-08-20T16:13:00Z","actor":"lead","action":"retire","target":"scribe","ref":"0199c0de-1234-4890-abcd-ef0123456789","target_slot":"spawned.0"}"#,
    );

    let (_, world) = ae::current_world(&root);
    let entry = world
        .sessions
        .iter()
        .find(|entry| entry.name == "brf6")
        .expect("the planted session is discovered");
    let mut card = ae::brief::card_for(entry, &dir, Some(&root), false, world.now, None);
    card.status = "running";

    assert_eq!(
        card.open_ask_count(),
        0,
        "retire closes the request: {card:?}"
    );
    assert!(
        !card
            .needs
            .iter()
            .any(|need| matches!(need, ae::brief::Need::Unanswered { .. })),
        "a retire-closed request must not survive in needs: {card:?}"
    );
    let overview = ae::overview::render(&[card], "orchestrator");
    assert!(!overview.contains("open ask"), "{overview}");
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
    let dir = super::cli::OwnedScratch::root("brief", "own").keep();
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
        .env("TMUX", format!("{},1,0", socket.display()))
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
        .env("TMUX", format!("{},1,0", socket.display()))
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

// ── `--seat`: ONE seat's seed pack ──────────────────────────────────────────
//
// The pack is what a successor continuing this seat on another tool is given,
// so the arm below asserts the two things that make it worth pasting: the
// parking note arrives WHOLE (the card clips it to one line, and that clip is
// exactly what the drill measured as the costly loss), and a pending request
// arrives with the exact command that answers it.

/// Plant a second seat's memo, declaration, spawn record and open request, in
/// the shapes the product writes them. SYNTHETIC throughout: no live session is
/// read, and no real record is quoted.
fn plant_seat_facts(dir: &Path, parking: &str) {
    let ts = ago(0);
    let spawn = format!(
        r#"{{"ts":"{ts}","actor":"lead","action":"spawn","target":"scribe","summary":"go"}}"#
    );
    let ask = format!(
        r#"{{"ts":"{ts}","actor":"scribe","action":"ask","target":"lead","ref":"ae-20260918T120000Z-0123abcd","actor_slot":"spawned.0","actor_session":"brf9","target_slot":"main","target_session":"brf9","summary":"which cap applies"}}"#
    );
    let existing = fs::read_to_string(dir.join("events.jsonl")).unwrap_or_default();
    assert!(
        fs::write(
            dir.join("events.jsonl"),
            format!("{existing}{spawn}\n{ask}\n")
        )
        .is_ok(),
        "a planted ledger"
    );
    memo(dir, "parking", parking);
}

#[test]
fn a_seat_pack_carries_the_parking_note_whole_and_the_exact_reply_command() {
    let root = scratch("seat");
    let work = root.join("work");
    let dir = plant(&root, "brf9", &work);
    // Long enough that the CARD would clip it: the pack must not.
    let parking = "resume here: (a) land the renderer, (b) wire the flag, \
                   (c) write the integration pins, (d) update the commands page \
                   and only then ask the lead to merge";
    plant_seat_facts(&dir, parking);
    declare(&dir, "lead", "working", "driving the seed pack slice", 120);

    let (code, stdout, stderr) = run(&root, &["brief", "brf9", "--seat", "lead"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains(parking),
        "the parking body is whole: {stdout}"
    );
    // The command names the session's own helper dir, this seat as the replying
    // identity, and the label the request's KIND carries.
    let reply = format!(
        "{}/reply --as \"lead\" \"ae-20260918T120000Z-0123abcd\" \"<your reply>\"",
        dir.display()
    );
    assert!(stdout.contains(&reply), "{stdout}");
    assert!(
        stdout.contains("## 6. owned spawns\n  scribe"),
        "the seat owns what it spawned: {stdout}"
    );
    assert!(
        stdout.contains("state: working") && stdout.contains("driving the seed pack slice"),
        "{stdout}"
    );
    // The closing block is last, and it tells the successor the rest is DATA.
    assert!(
        stdout.ends_with("4. Continue at the parking note in section 4.\n"),
        "{stdout}"
    );
    // The card path is UNTOUCHED by the flag's existence.
    let (code, cards, _) = run(&root, &["brief", "brf9"]);
    assert_eq!(code, Some(0));
    assert!(cards.starts_with("brf9 · "), "{cards}");
    assert!(!cards.contains("# seed pack"), "{cards}");
}

#[test]
fn a_seat_pack_names_the_tracked_paths_a_real_work_tree_changed() {
    let root = scratch("seatgit");
    let work = root.join("work");
    assert!(fs::create_dir_all(&work).is_ok(), "the work dir");
    super::cli::git_in(&work, &["init", "-q"]);
    plant(&root, "brf11", &work);
    for name in ["kept.txt", "edited.txt"] {
        assert!(fs::write(work.join(name), "one\n").is_ok(), "{name}");
    }
    super::cli::git_in(&work, &["add", "kept.txt", "edited.txt"]);
    super::cli::git_in(&work, &["commit", "-q", "-m", "seed"]);
    assert!(
        fs::write(work.join("edited.txt"), "two\n").is_ok(),
        "an edit"
    );
    assert!(
        fs::write(work.join("new.txt"), "untracked\n").is_ok(),
        "a new file"
    );

    let (code, stdout, stderr) = run(&root, &["brief", "brf11", "--seat", "lead"]);

    assert_eq!(code, Some(0), "stderr: {stderr}");
    // Only the TRACKED change is listed: the untracked file is not the
    // successor's to commit, and the untouched one is not news.
    assert!(
        stdout.contains(
            "dirty: yes\nlatest tag: none\nchanged, not committed:\n  - edited.txt\n\
             recent commits:\n  - seed\n"
        ),
        "{stdout}"
    );
}

#[test]
fn a_seat_pack_refuses_the_flags_that_contradict_it_and_names_an_unknown_seat() {
    let root = scratch("seatrefuse");
    let work = root.join("work");
    let dir = plant(&root, "brf10", &work);
    plant_seat_facts(&dir, "resume here: nothing yet");

    // A pack is ONE seat of ONE session, so the fleet flag and the window flag
    // are usage errors beside it — in EITHER order, so neither is a last-wins.
    for tail in [
        vec!["brief", "brf10", "--seat", "lead", "--all"],
        vec!["brief", "--all", "--seat", "lead"],
        vec!["brief", "brf10", "--seat", "lead", "--since", "2h"],
        vec!["brief", "brf10", "--since", "2h", "--seat", "lead"],
        vec!["brief", "brf10", "--seat"],
    ] {
        let (code, stdout, stderr) = run(&root, &tail);
        assert_eq!(code, Some(2), "{tail:?}: {stderr}");
        assert!(stdout.is_empty(), "{tail:?}: {stdout}");
        assert!(stderr.contains("Usage: ae brief"), "{tail:?}: {stderr}");
    }

    // A seat that is not on the roster is named, and so is the roster — a
    // successor that mistyped a name can see what it should have typed.
    let (code, stdout, stderr) = run(&root, &["brief", "brf10", "--seat", "nobody"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(
        stderr.contains("ae brief: brf10 has no seat named nobody; its roster is lead, scribe"),
        "{stderr}"
    );

    // No session to pack, and no fleet to fall back to: the card path widens to
    // every running session here, and the pack path must NOT.
    let (code, stdout, stderr) = run(&root, &["brief", "--seat", "lead"]);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.contains("--seat needs one session"), "{stderr}");
}

#[test]
fn a_seat_pack_reads_a_stopped_session_and_still_names_what_it_owns() {
    // The whole point of the surface: the seat whose tool died is the one that
    // needs a pack, and its session is often no longer running. Nothing here
    // asks tmux anything — the planted server socket does not exist.
    let root = scratch("seatstopped");
    let work = root.join("work");
    let dir = plant(&root, "brf11", &work);
    plant_seat_facts(&dir, "resume here: the successor picks this up");

    let (code, stdout, stderr) = run(&root, &["brief", "brf11", "--seat", "lead"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("# seed pack — brf11 / lead"), "{stdout}");
    // Stop is not retire: a seat still owns what it spawned.
    assert!(
        stdout.contains("## 6. owned spawns\n  scribe"),
        "a stopped session keeps its ownership: {stdout}"
    );
    assert!(
        stdout.contains("resume here: the successor picks this up"),
        "{stdout}"
    );
    // Monitor panes are not seats and never reach the roster.
    assert!(!stdout.contains("_watchdog"), "{stdout}");
    assert!(!stdout.contains("_events"), "{stdout}");
}

/// One synthetic Claude user turn. Plain fixture prose; nothing is copied out
/// of a real transcript.
fn user_turn(ts: &str, body: &str) -> String {
    format!(
        r#"{{"type":"user","timestamp":"{ts}","message":{{"role":"user","content":"{body}"}}}}"#
    )
}

/// One synthetic Claude assistant turn, carrying a thinking part and a tool
/// call BESIDE its text: the pack may never carry either.
fn assistant_turn(ts: &str, text: &str) -> String {
    format!(
        r#"{{"type":"assistant","timestamp":"{ts}","message":{{"role":"assistant","content":[{{"type":"thinking","thinking":"never carried"}},{{"type":"tool_use","name":"Bash","input":{{"command":"never carried either"}}}},{{"type":"text","text":"{text}"}}]}}}}"#
    )
}

#[test]
fn a_seat_pack_carries_the_seats_own_last_turns_and_names_a_read_it_could_not_make() {
    // THE wiring pin: the one place the pack's turns section meets a real
    // reader. SYNTHETIC records throughout — hand-written shapes, never a line
    // copied out of anyone's transcript.
    let root = scratch("seatturns");
    let work = root.join("work");
    let dir = plant(&root, "brf12", &work);
    let store = root.join("claude");
    let id = "0199c0de-1234-4890-abcd-ef0123456789";
    let project = store.join("projects").join("work");
    assert!(fs::create_dir_all(&project).is_ok(), "a project dir");
    let lines = [
        user_turn(&ago(600), "the first thing the human asked"),
        assistant_turn(&ago(540), "what the seat answered"),
        user_turn(&ago(480), "⟦ae:msg from lead⟧ ae wrote this one"),
        user_turn(&ago(300), "the last thing the human asked"),
    ];
    assert!(
        fs::write(
            project.join(format!("{id}.jsonl")),
            format!("{}\n", lines.join("\n"))
        )
        .is_ok(),
        "a synthetic transcript"
    );
    // The three roster rows the board locator reads, appended to the meta
    // `plant` wrote: without them the seat records no tool at all.
    let meta = dir.join("meta");
    let planted = fs::read_to_string(&meta).unwrap_or_default();
    assert!(
        fs::write(
            &meta,
            format!(
                "{planted}harness_session.main={id}\nagent_bin.main=claude\nconfig_home.main={}\n",
                store.display()
            )
        )
        .is_ok(),
        "the seat's own store"
    );

    let (code, stdout, stderr) = run(&root, &["brief", "brf12", "--seat", "lead"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains("newest six turns, oldest first"),
        "{stdout}"
    );
    assert!(
        stdout.contains("--- human  10m\nthe first thing the human asked\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("--- assistant  9m\nwhat the seat answered\n"),
        "both halves of the exchange: {stdout}"
    );
    assert!(
        stdout.contains("--- human  5m\nthe last thing the human asked\n"),
        "oldest first: {stdout}"
    );
    // ae's own injected turn is not the seat's own words.
    assert!(!stdout.contains("ae wrote this one"), "{stdout}");
    // Words only: the reader never reads a thinking block or a tool call, so
    // neither can reach a successor through the pack.
    assert!(!stdout.contains("never carried"), "{stdout}");

    // The OTHER seat of the same session records no tool, so its pack says WHY
    // it carries nothing rather than reading as a seat that said nothing.
    let (code, stdout, stderr) = run(&root, &["brief", "brf12", "--seat", "scribe"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains("## last turns\nincomplete: unknown tool: out of scope\n"),
        "{stdout}"
    );
    assert!(!stdout.contains("none recorded\n\n## footnote"), "{stdout}");

    // A seat whose reader REFUSES says so in that reader's own words: the
    // coverage reason is the board's, verbatim. OpenCode's is the export leg,
    // and this seat records no conversation at all — which is also why the
    // pin runs no CLI: the id is answered before the leg would shell out.
    let meta = dir.join("meta");
    let named = fs::read_to_string(&meta).unwrap_or_default();
    assert!(
        fs::write(&meta, format!("{named}agent_bin.spawned.0=opencode\n")).is_ok(),
        "a seat whose tool is read through an export"
    );
    let (code, stdout, stderr) = run(&root, &["brief", "brf12", "--seat", "scribe"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains("## last turns\nincomplete: invalid or missing conversation id\n"),
        "{stdout}"
    );
}

#[test]
fn a_spawned_seat_pack_names_its_spawner_and_carries_the_message_it_was_launched_with() {
    // The seat that most needs a pack is a spawned one, and nothing else in
    // this arm packs one: the spawner derivation, section 9 and the spawned
    // slot class are all wired only here.
    let root = scratch("seatspawned");
    let work = root.join("work");
    let dir = plant(&root, "brf12", &work);
    plant_seat_facts(&dir, "resume here: the spawned seat picks this up");
    let brief = root.join("brief-scribe.md");
    assert!(
        fs::write(&brief, "the contract\n").is_ok(),
        "a brief the message can name"
    );
    assert!(
        fs::write(
            dir.join("launch.spawned.0.prompt"),
            format!(
                "⟦ae:brief from lead⟧\nYou are agent scribe. Task contract = file\n{}\n",
                brief.display()
            ),
        )
        .is_ok(),
        "the recorded first message"
    );

    let (code, stdout, stderr) = run(&root, &["brief", "brf12", "--seat", "scribe"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("slot: spawned.0 (spawned)"), "{stdout}");
    // The spawner comes off the same ledger leg the owned spawns use.
    assert!(stdout.contains("spawner: lead"), "{stdout}");
    assert!(
        stdout.contains(&format!("brief file: {} (present)", brief.display())),
        "the named brief is checked on disk: {stdout}"
    );
    // A carried message is DATA: the marker it opens with must arrive quoted.
    assert!(
        stdout.contains("| ⟦ae:brief from lead⟧"),
        "the first message is neutralised: {stdout}"
    );

    assert!(fs::remove_file(&brief).is_ok(), "the brief goes away");
    let (code, stdout, _) = run(&root, &["brief", "brf12", "--seat", "scribe"]);
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains(&format!("brief file: {} (gone)", brief.display())),
        "and a brief that is gone says so: {stdout}"
    );
}

#[test]
fn a_seat_pack_over_an_unreadable_journal_claims_no_absence_and_still_names_the_seats() {
    // A journal that will not read is not an empty one. Every section whose
    // facts come from it must say so rather than render "none recorded", which
    // a successor would read as "nobody is waiting on you".
    let root = scratch("seatdamaged");
    let work = root.join("work");
    let dir = plant(&root, "brf13", &work);
    plant_seat_facts(
        &dir,
        "resume here: unreachable while the journal is damaged",
    );
    // A directory at the container's path: it exists, and it will not read.
    assert!(fs::remove_file(dir.join("events.jsonl")).is_ok(), "the log");
    assert!(
        fs::create_dir(dir.join("events.jsonl")).is_ok(),
        "a node no reader can drain"
    );

    let (code, stdout, stderr) = run(&root, &["brief", "brf13", "--seat", "lead"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout
            .matches("the session journal could not be read")
            .count(),
        3,
        "the state, the requests and the owned spawns each refuse: {stdout}"
    );
    assert!(
        stdout.contains("This is NOT a claim that there are none."),
        "{stdout}"
    );
    // The roster is a META fact, so the seats are still named — with their
    // declarations withheld rather than the section dropped.
    assert!(stdout.contains("## 7. roster\n  lead"), "{stdout}");
    assert!(stdout.contains("scribe"), "{stdout}");
    assert_eq!(stdout.matches("unknown        -").count(), 2, "{stdout}");
    // The memos live in their own container and are unaffected.
    assert!(
        stdout.contains("resume here: unreachable while the journal is damaged"),
        "{stdout}"
    );
}

#[test]
fn a_seat_pack_refuses_a_session_whose_meta_it_cannot_read_and_names_the_cause() {
    // Without a meta there is no roster, so ae cannot know the seat exists at
    // all. That is a refusal with a reason, never a pack built from guesses.
    let root = scratch("seatnometa");
    let work = root.join("work");
    let dir = plant(&root, "brf14", &work);
    plant_seat_facts(&dir, "resume here: never reached");
    assert!(fs::remove_file(dir.join("meta")).is_ok(), "the meta goes");

    let (code, stdout, stderr) = run(&root, &["brief", "brf14", "--seat", "lead"]);
    assert_eq!(code, Some(1), "a refusal, not a pack: {stdout}");
    assert!(stdout.is_empty(), "nothing is printed: {stdout}");
    assert!(
        stderr.contains("meta is absent") && stderr.contains("which seats it has"),
        "the cause is named: {stderr}"
    );
}

#[test]
fn the_seat_pack_carries_agy_replies_beside_human_turns() {
    // No pack change: the transcript leg feeds observe_seat_turns through
    // observe_generation, so the binary proves the interaction.
    let root = scratch("agy-pack");
    let dir = plant(&root, "ship", &root.join("work"));
    let id = "0199c0de-ffff-4890-abcd-ef0123456789";
    let meta = fs::read_to_string(dir.join("meta")).expect("meta");
    fs::write(
        dir.join("meta"),
        format!("{meta}harness_session.main={id}\nagent_bin.main=agy\n"),
    )
    .expect("agy rows");
    let store = root.join(".gemini/antigravity-cli");
    fs::create_dir_all(&store).expect("agy dir");
    fs::write(
        store.join("history.jsonl"),
        format!(
            "{{\"display\":\"pack human words\",\"timestamp\":1789549200500,\"conversationId\":\"{id}\",\"workspace\":\"/work\"}}\n"
        ),
    )
    .expect("history");
    let logs = store.join("brain").join(id).join(".system_generated/logs");
    fs::create_dir_all(&logs).expect("brain dir");
    fs::write(
        logs.join("transcript_full.jsonl"),
        "{\"step_index\":1,\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"status\":\"DONE\",\"created_at\":\"2026-09-16T09:00:00Z\",\"content\":\"pack reply words\"}\n",
    )
    .expect("transcript");
    let (code, stdout, stderr) = run(&root, &["brief", "ship", "--seat", "lead"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("## last turns"), "{stdout}");
    assert!(stdout.contains("--- human"), "{stdout}");
    assert!(stdout.contains("pack human words"), "{stdout}");
    assert!(stdout.contains("--- assistant"), "{stdout}");
    assert!(stdout.contains("pack reply words"), "{stdout}");
    let _ = fs::remove_dir_all(&root);
}

// ── `--full`: the card with nothing clipped, and the declaration view tokens ─
//
// S0 of the approved orchestrator plan (`.local/orchrole/plan.md`, r10, section 7 and
// appendix C). The
// default card clips a goal, a memo line and an agent reason to fit half a
// terminal; `--full` prints every one whole, and for each seat whose latest
// declaration is `waiting-user` or `blocked` it prints a DISPLAY-ONLY view
// token of that seat's current slot and latest readable declaration. The lead's
// ruling (2026-11-19) bounds what a test may pin: the token changes when the
// declared state or reason changes, and promises nothing for two identical
// declarations in one second. It is not an authority binding; the later
// ruling slice writes its own launch-bound declaration ids and refuses it.
//
// The fixtures write state the way the product does: the main seat declares
// `waiting-user` (a spawned seat may not), a spawned seat declares `blocked`,
// and every timestamp is whole seconds with no event id, as `state` emits.

const FULL_UUID: &str = "5b1c7a0e-2d4f-4c3a-9e8b-1f6d0a3b7c92";
const LAUNCH_LEAD: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const LAUNCH_SCRIBE: &str = "aaaaaaaa-0000-4000-8000-000000000002";
const LAUNCH_COLEAD: &str = "aaaaaaaa-0000-4000-8000-000000000003";
/// The main seat's harness conversation id: a DIFFERENT value from the session uuid, so a
/// test can tell the two apart (the list JSON shortens this one to its first 8 characters).
const HARNESS_ID: &str = "e795c9e9-1b2c-4d3e-8f40-5a6b7c8d9e0f";

/// A decision the main seat asks of the human, in the shape `state` requires.
const LEAD_ASK: &str = "Which token shape should S0 print: 1. one slash-joined line, easy to quote | 2. a multi-line block, easier to read (recommend 1 because a script can pass it as one argument; details: memo decision)";
/// What the default card's `needs you:` wraps [`LEAD_ASK`] to (width 100).
const LEAD_ASK_WRAPPED: [&str; 3] = [
    "Which token shape should S0 print: 1. one slash-joined line,",
    "easy to quote | 2. a multi-line block, easier to read (recommend 1 because a script can pass it",
    "as one argument; details: memo decision)",
];
/// A spawned seat's blocker.
const SCRIBE_BLOCK: &str = "blocked on the hp lane claim that layout-p3 holds since 09:12Z; it unblocks when that claim releases; tried a one-file filter, refused at load1 9; details: .local/orch-s0/blocked.md";
const SCRIBE_BLOCK_WRAPPED: [&str; 3] = [
    "blocked on the hp lane claim that layout-p3 holds since",
    "09:12Z; it unblocks when that claim releases; tried a one-file filter, refused at load1 9;",
    "details: .local/orch-s0/blocked.md",
];

/// The planted session with the identity rows a view token needs: a session
/// uuid, a launch id per slot, and a third seat (`worker.0`, named `colead`).
fn plant_full(root: &Path, name: &str, work_dir: &Path) -> PathBuf {
    let dir = plant(root, name, work_dir);
    let mut meta = fs::read_to_string(dir.join("meta")).expect("the planted meta");
    let rows = format!(
        "session_id={FULL_UUID}\nharness_session.main={HARNESS_ID}\nlaunch_id.main={LAUNCH_LEAD}\nlaunch_id.spawned.0={LAUNCH_SCRIBE}\n\
         seat.worker.0=colead\nprofile.worker.0=cx\nlaunch_id.worker.0={LAUNCH_COLEAD}\n"
    );
    meta.push_str(&rows);
    assert!(fs::write(dir.join("meta"), meta).is_ok(), "the full meta");
    dir
}

/// One `state` line at an explicit instant, in the emitter's own shape.
fn state_line(ts: &str, actor: &str, value: &str, reason: &str) -> String {
    let at = ae::time::Timestamp::parse(ts).expect("a whole-second instant");
    ae::state::event_line(at, actor, "state", value, reason)
}

/// The view token the docs define:
/// `view1/<session uuid>/<slot>/<launch id>/<ts>/<digest>`, the digest being
/// the first 16 hex characters of SHA-256 over the declaration's actor,
/// action (`state`), ref, summary and ts, each followed by one NUL byte.
fn view(slot: &str, launch: &str, ts: &str, owner: &str, value: &str, reason: &str) -> String {
    let mut input = Vec::new();
    for part in [owner, "state", value, reason, ts] {
        input.extend_from_slice(part.as_bytes());
        input.push(0);
    }
    let digest = ae::install::sha256_hex(&input);
    format!("view1/{FULL_UUID}/{slot}/{launch}/{ts}/{}", &digest[..16])
}

/// What the default card does to a long field: whole when it fits, else its
/// first `max - 1` characters and an ellipsis.
fn clipped(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(max - 1).collect();
    cut.push('…');
    cut
}

/// The long fixture's header: its newest event is 300 s old, so the card is `active 5m ago`.
fn long_header(name: &str) -> String {
    format!(
        "{name} · unknown · attn:waiting-user · ae 2026.9.5 · created - · started - · active 5m ago · s1-brief · ~/work"
    )
}

struct Long {
    dir: PathBuf,
    goal: String,
    decision: String,
    parking: String,
    ts: [String; 4],
}

/// The fixture every whole-field test shares: a 400-character goal, two memo
/// topics each with a SUPERSEDED older record and a long newest one, and four
/// declarations: colead blocked then working, lead `waiting-user`, scribe
/// `blocked`. Every text is single-spaced, so cleaning changes nothing.
fn plant_long(root: &Path, name: &str) -> Long {
    let dir = plant_full(root, name, &root.join("work"));
    let goal = format!(
        "{} END-OF-GOAL",
        ["goal words that run well past the card width"; 8].join(" ")
    );
    let meta = fs::read_to_string(dir.join("meta")).expect("the planted meta");
    assert!(
        fs::write(
            dir.join("meta"),
            meta.replace("goal=ship S1 of #113", &format!("goal={goal}"))
        )
        .is_ok(),
        "the long goal"
    );
    let decision = format!(
        "{} END-OF-DECISION",
        ["route the review through ae and gate once per merge"; 6].join(" ")
    );
    let parking = format!(
        "{} END-OF-PARKING",
        ["resume here: the tokens are specified and the tests next"; 4].join(" ")
    );
    // File order is time order: a later record SUPERSEDES the earlier one of its topic.
    assert!(
        fs::write(
            dir.join("memo.tsv"),
            format!(
                "{}\thuman\tdecision\tOLDER-DECISION-SUPERSEDED\n\
                 {}\thuman\tparking\tOLDER-PARKING-SUPERSEDED\n\
                 {}\thuman\tdecision\t{decision}\n\
                 {}\thuman\tparking\t{parking}\n",
                ago(9_000),
                ago(20_000),
                ago(1_500),
                ago(4_000)
            )
        )
        .is_ok(),
        "the long memos"
    );
    let ts = [ago(3_000), ago(720), ago(600), ago(300)];
    let journal = [
        state_line(
            &ts[0],
            "colead",
            "blocked",
            "waiting for the spec freeze to be signed off by the reviewer",
        ),
        state_line(&ts[1], "lead", "waiting-user", LEAD_ASK),
        state_line(&ts[2], "scribe", "blocked", SCRIBE_BLOCK),
        state_line(
            &ts[3],
            "colead",
            "working",
            "reviewing the S0 tests against the plan",
        ),
    ]
    .concat();
    assert!(
        fs::write(dir.join("events.jsonl"), journal).is_ok(),
        "the journal"
    );
    Long {
        dir,
        goal,
        decision,
        parking,
        ts,
    }
}

#[test]
fn full_prints_every_field_the_default_card_clips_whole() {
    let root = scratch("full-whole");
    let long = plant_long(&root, "brf20");

    let (code, stdout, stderr) = run(&root, &["brief", "brf20", "--full"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let expected = format!(
        "{header}\n\
         \x20 uuid: {FULL_UUID}\n\
         \x20 goal: {goal}\n\
         \x20 topics:\n\
         \x20   decision    25m   human         {decision}\n\
         \x20   parking     1h    human         {parking}\n\
         \x20 agents:\n\
         \x20   lead          cl            waiting-user  12m   \"{ask}\"\n\
         \x20   scribe        cx            blocked       10m   \"{block}\"\n\
         \x20   colead        cx            working       5m    \"reviewing the S0 tests against the plan\"\n\
         \x20 needs you:\n\
         \x20   lead          waiting-user  12m   {ask}\n\
         \x20   scribe        blocked       10m   {block}\n\
         \x20 declarations:\n\
         \x20   lead          waiting-user  {lead_view}\n\
         \x20   scribe        blocked       {scribe_view}\n",
        header = long_header("brf20"),
        goal = long.goal,
        decision = long.decision,
        parking = long.parking,
        ask = LEAD_ASK,
        block = SCRIBE_BLOCK,
        lead_view = view(
            "main",
            LAUNCH_LEAD,
            &long.ts[1],
            "lead",
            "waiting-user",
            LEAD_ASK
        ),
        scribe_view = view(
            "spawned.0",
            LAUNCH_SCRIBE,
            &long.ts[2],
            "scribe",
            "blocked",
            SCRIBE_BLOCK
        ),
    );
    assert_eq!(stdout, expected, "stderr: {stderr}");
    // The session uuid, not the seat's harness conversation id.
    assert!(!stdout.contains("e795c9e9"), "{stdout}");
    // A superseded record stays superseded under --full.
    assert!(!stdout.contains("SUPERSEDED"), "{stdout}");
    // The view token is ONE whitespace-free word.
    for row in stdout.lines().filter(|line| line.contains("view1/")) {
        let last = row.split_whitespace().last().unwrap_or_default();
        assert!(last.starts_with("view1/"), "{row}");
    }
}

#[test]
fn the_default_card_stays_clipped_and_prints_no_declaration_section() {
    // The byte pin: this fixture's DEFAULT card, as the shipped binary printed it
    // before `--full` existed. A clipped goal, memo line and agent reason, a
    // wrapped need, and no new section.
    let root = scratch("default-pin");
    let long = plant_long(&root, "brf21");

    let (code, stdout, stderr) = run(&root, &["brief", "brf21"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let expected = format!(
        "{header}\n\
         \x20 goal: {goal}\n\
         \x20 topics:\n\
         \x20   decision    25m   human         {decision}\n\
         \x20   parking     1h    human         {parking}\n\
         \x20 agents:\n\
         \x20   lead          cl            waiting-user  12m   \"{ask}\"\n\
         \x20   scribe        cx            blocked       10m   \"{block}\"\n\
         \x20   colead        cx            working       5m    \"reviewing the S0 tests against the plan\"\n\
         \x20 needs you:\n\
         \x20   lead          waiting-user  12m   {ask1}\n\
         \x20   {ask2}\n\
         \x20   {ask3}\n\
         \x20   scribe        blocked       10m   {block1}\n\
         \x20   {block2}\n\
         \x20   {block3}\n",
        header = long_header("brf21"),
        goal = clipped(&long.goal, 92),
        decision = clipped(&long.decision, 64),
        parking = clipped(&long.parking, 64),
        ask = clipped(LEAD_ASK, 46),
        block = clipped(SCRIBE_BLOCK, 46),
        ask1 = LEAD_ASK_WRAPPED[0],
        ask2 = LEAD_ASK_WRAPPED[1],
        ask3 = LEAD_ASK_WRAPPED[2],
        block1 = SCRIBE_BLOCK_WRAPPED[0],
        block2 = SCRIBE_BLOCK_WRAPPED[1],
        block3 = SCRIBE_BLOCK_WRAPPED[2],
    );
    assert_eq!(stdout, expected, "stderr: {stderr}");
    assert!(!stdout.contains("declarations"), "{stdout}");
    assert!(!stdout.contains("view1/"), "{stdout}");
}

#[test]
fn the_view_token_changes_when_the_declared_state_or_reason_changes() {
    // The ONLY distinctness the token promises. Two identical declarations in
    // the same second are NOT promised to differ: `state` writes no event id.
    let root = scratch("token-changes");
    let dir = plant_full(&root, "brf22", &root.join("work"));
    let ts = ago(120);
    let other_reason =
        "a different question, asked once more, with the options spelled out for the human";
    let view_of = |value: &str, reason: &str| view("main", LAUNCH_LEAD, &ts, "lead", value, reason);

    let read = |value: &str, reason: &str| {
        assert!(
            fs::write(
                dir.join("events.jsonl"),
                state_line(&ts, "lead", value, reason)
            )
            .is_ok(),
            "a declaration"
        );
        let (code, stdout, stderr) = run(&root, &["brief", "brf22", "--full"]);
        assert_eq!(code, Some(0), "stderr: {stderr}");
        stdout
    };
    let first = read("waiting-user", LEAD_ASK);
    let reworded = read("waiting-user", other_reason);
    let escalated = read("blocked", LEAD_ASK);
    for (stdout, value, reason) in [
        (&first, "waiting-user", LEAD_ASK),
        (&reworded, "waiting-user", other_reason),
        (&escalated, "blocked", LEAD_ASK),
    ] {
        let row = format!("    lead          {value:<14}{}\n", view_of(value, reason));
        assert!(stdout.contains(&row), "{row}in {stdout}");
    }
    let tokens: Vec<&str> = [&first, &reworded, &escalated]
        .iter()
        .filter_map(|stdout| stdout.lines().find(|row| row.contains("view1/")))
        .filter_map(|row| row.split_whitespace().last())
        .collect();
    assert_eq!(tokens.len(), 3);
    assert!(
        tokens[0] != tokens[1] && tokens[0] != tokens[2] && tokens[1] != tokens[2],
        "a different state or reason is a different view: {tokens:?}"
    );

    // A seat that moved on has no row.
    assert!(
        fs::write(
            dir.join("events.jsonl"),
            format!(
                "{}{}",
                state_line(&ts, "lead", "waiting-user", LEAD_ASK),
                state_line(&ago(30), "lead", "working", "on it")
            )
        )
        .is_ok(),
        "moved on"
    );
    let (_, moved, _) = run(&root, &["brief", "brf22", "--full"]);
    assert!(!moved.contains("view1/"), "{moved}");
    assert!(moved.contains("  declarations: none waiting\n"), "{moved}");
}

#[test]
fn the_view_token_names_the_slots_current_launch_id_at_read_time() {
    // `state` rows carry no launch id, so a view cannot say which incarnation
    // declared. It names the slot as it stands when ae reads, and says so.
    let root = scratch("token-launch");
    let long = plant_long(&root, "brf23");
    let lead = |launch: &str| {
        view(
            "main",
            launch,
            &long.ts[1],
            "lead",
            "waiting-user",
            LEAD_ASK,
        )
    };
    let (_, before, _) = run(&root, &["brief", "brf23", "--full"]);
    assert!(before.contains(&lead(LAUNCH_LEAD)), "{before}");

    let relaunched = "aaaaaaaa-0000-4000-8000-0000000000ff";
    let meta = fs::read_to_string(long.dir.join("meta")).expect("the meta");
    assert!(
        fs::write(long.dir.join("meta"), meta.replace(LAUNCH_LEAD, relaunched)).is_ok(),
        "a new launch id"
    );
    let (_, after, _) = run(&root, &["brief", "brf23", "--full"]);
    assert!(after.contains(&lead(relaunched)), "{after}");
    assert!(!after.contains(&lead(LAUNCH_LEAD)), "{after}");
    // The other seat's view is untouched.
    let scribe = view(
        "spawned.0",
        LAUNCH_SCRIBE,
        &long.ts[2],
        "scribe",
        "blocked",
        SCRIBE_BLOCK,
    );
    assert!(after.contains(&scribe), "{after}");
}

#[test]
fn a_view_ae_cannot_form_says_why_and_the_other_seats_keep_theirs() {
    let root = scratch("token-unavailable");
    let long = plant_long(&root, "brf24");
    // Scribe's slot has no launch id on record.
    let meta = fs::read_to_string(long.dir.join("meta")).expect("the meta");
    let without = meta.replace(&format!("launch_id.spawned.0={LAUNCH_SCRIBE}\n"), "");
    assert_ne!(meta, without);
    assert!(
        fs::write(long.dir.join("meta"), without).is_ok(),
        "no scribe launch id"
    );

    let (code, stdout, stderr) = run(&root, &["brief", "brf24", "--full"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains("\n    scribe        blocked       unavailable (no launch_id recorded for slot spawned.0)\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!(
            "\n    lead          waiting-user  {}\n",
            view(
                "main",
                LAUNCH_LEAD,
                &long.ts[1],
                "lead",
                "waiting-user",
                LEAD_ASK
            )
        )),
        "{stdout}"
    );

    // A session whose meta names no session id has a view for nobody.
    let nameless = fs::read_to_string(long.dir.join("meta"))
        .expect("the meta")
        .replace(&format!("session_id={FULL_UUID}\n"), "");
    assert!(
        fs::write(long.dir.join("meta"), nameless).is_ok(),
        "no session id"
    );
    let (_, stdout, _) = run(&root, &["brief", "brf24", "--full"]);
    assert!(!stdout.contains("view1/"), "{stdout}");
    assert!(
        stdout.contains("\n    lead          waiting-user  unavailable (no session_id recorded)\n"),
        "{stdout}"
    );
}

#[test]
fn an_unreadable_source_is_labelled_never_dropped() {
    // A directory where the file belongs: unreadable, not absent, and no
    // permission bits to portably fake. ONE source at a time, so a renderer that
    // labels both sections because either source failed cannot pass.
    let root = scratch("full-unreadable");
    let ts = ago(600);
    let known = |name: &str| {
        let dir = plant_full(&root, name, &root.join(format!("work-{name}")));
        assert!(
            fs::write(
                dir.join("memo.tsv"),
                format!("{}\thuman\tdecision\tKNOWN-MEMO\n", ago(90))
            )
            .is_ok(),
            "a readable memo"
        );
        assert!(
            fs::write(
                dir.join("events.jsonl"),
                state_line(&ts, "lead", "waiting-user", LEAD_ASK)
            )
            .is_ok(),
            "a readable journal"
        );
        dir
    };
    let swap_for_directory = |dir: &Path, file: &str| {
        assert!(fs::remove_file(dir.join(file)).is_ok(), "{file} removed");
        assert!(
            fs::create_dir(dir.join(file)).is_ok(),
            "{file} as a directory"
        );
    };
    let lead_view = view("main", LAUNCH_LEAD, &ts, "lead", "waiting-user", LEAD_ASK);

    // The journal alone is unreadable: the declarations say so, the memo still reads.
    let dir = known("brf26a");
    swap_for_directory(&dir, "events.jsonl");
    let (code, stdout, stderr) = run(&root, &["brief", "brf26a", "--full"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains("  declarations: unreadable (a directory)\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("  topics:\n    decision    1m    human         KNOWN-MEMO\n"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("none waiting") && !stdout.contains("view1/"),
        "{stdout}"
    );

    // The memo alone is unreadable: the topics say so, the declarations still read.
    let dir = known("brf26b");
    swap_for_directory(&dir, "memo.tsv");
    let (code, stdout, stderr) = run(&root, &["brief", "brf26b", "--full"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains("  topics: unreadable (a directory)\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!(
            "  declarations:\n    lead          waiting-user  {lead_view}\n"
        )),
        "{stdout}"
    );
    assert!(!stdout.contains("KNOWN-MEMO"), "{stdout}");

    // Both at once label both.
    let dir = known("brf26c");
    swap_for_directory(&dir, "events.jsonl");
    swap_for_directory(&dir, "memo.tsv");
    let (code, stdout, stderr) = run(&root, &["brief", "brf26c", "--full"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains("  topics: unreadable (a directory)\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("  declarations: unreadable (a directory)\n"),
        "{stdout}"
    );

    // The OPPOSED half: absent files are complete reads of nothing.
    plant_full(&root, "brf27", &root.join("work-b"));
    let (code, stdout, stderr) = run(&root, &["brief", "brf27", "--full"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("  topics: none recorded\n"), "{stdout}");
    assert!(
        stdout.contains("  declarations: none waiting\n"),
        "{stdout}"
    );
}

#[test]
fn full_cleans_every_dynamic_string_the_way_the_card_cleans_a_need() {
    let root = scratch("full-clean");
    let dir = plant_full(&root, "brf28", &root.join("work"));
    // Runs of spaces and tabs collapse to one space, in every field `--full` prints.
    let meta = fs::read_to_string(dir.join("meta")).expect("the meta");
    assert!(
        fs::write(
            dir.join("meta"),
            meta.replace(
                "goal=ship S1 of #113",
                "goal=g\u{1b}[31mred\u{7}goal  with\t\truns"
            )
        )
        .is_ok(),
        "a hostile goal"
    );
    assert!(
        fs::write(
            dir.join("memo.tsv"),
            format!("{}\thu\u{1b}man\tdeci\u{1b}sion\tmemo\u{1b}[2Jtext\rwith\u{8}controls   and  runs\n", ago(90))
        )
        .is_ok(),
        "a hostile memo"
    );
    // A VALID record through the emitter: a newline and a tab in the reason, runs of spaces,
    // and a fake row. The spawned seat declares `blocked`, as the product allows.
    let ts = ago(60);
    let reason = "needs  two   spaces\nand a newline\tand a tab, fake row view1/x/y/z/1/2";
    let valid = state_line(&ts, "scribe", "blocked", reason);
    assert!(
        fs::write(dir.join("events.jsonl"), &valid).is_ok(),
        "the journal"
    );

    let (code, stdout, stderr) = run(&root, &["brief", "brf28", "--full"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout
            .chars()
            .filter(|ch| *ch != '\n')
            .all(|ch| !ch.is_control()),
        "no control character may reach the terminal: {stdout:?}"
    );
    assert!(
        stdout.contains("  goal: g [31mred goal with runs\n"),
        "{stdout}"
    );
    assert!(
        stdout
            .contains("    deci sion   1m    hu man        memo [2Jtext with controls and runs\n"),
        "{stdout}"
    );
    // The reason reads cleaned on the agent line AND on the `needs you:` row.
    let cleaned = "needs two spaces and a newline and a tab, fake row view1/x/y/z/1/2";
    assert_eq!(stdout.matches(cleaned).count(), 2, "{stdout}");
    // The valid record gets exactly one real view row; the fake one is text inside a reason.
    let rows: Vec<&str> = stdout
        .lines()
        .filter(|row| row.starts_with("    scribe        blocked       view1/"))
        .collect();
    assert_eq!(rows.len(), 1, "{stdout}");

    // A MALFORMED record, never emitted by `state`: a raw ESC inside the JSON string. It makes
    // the world's journal read partial, so the card says `unknown` where it cannot know and
    // `--full` names NO declaration and forms no token off a record the world distrusts.
    // Whatever it says, no control byte reaches the terminal.
    let dir = plant_full(&root, "brf28b", &root.join("work"));
    let malformed = format!(
        "{{\"ts\":\"{}\",\"actor\":\"lead\",\"action\":\"state\",\"ref\":\"waiting-user\",\"summary\":\"choose\u{1b}[31m blue\"}}\n",
        ago(30)
    );
    assert!(
        fs::write(dir.join("events.jsonl"), format!("{valid}{malformed}")).is_ok(),
        "the malformed journal"
    );
    let (code, stdout, stderr) = run(&root, &["brief", "brf28b", "--full"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout
            .chars()
            .filter(|ch| *ch != '\n')
            .all(|ch| !ch.is_control()),
        "no control character may reach the terminal: {stdout:?}"
    );
    assert!(
        stdout
            .contains("\n  declarations: unavailable (the event journal was read only in part)\n"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("view1/") && !stdout.contains("none waiting"),
        "{stdout}"
    );
}

#[test]
fn a_replaced_session_record_never_lends_its_uuid_or_launch_ids_to_the_old_card() {
    // The world read FULL_UUID. Before `--full` reads the meta again for the roster slots and
    // launch ids, the record on disk is replaced by another session's. The card keeps the
    // identity of the read that built it and withholds every view join.
    let root = scratch("full-replaced");
    let long = plant_long(&root, "brf32");
    let (_, world) = ae::current_world(&root);
    let entry = world
        .sessions
        .iter()
        .find(|entry| entry.name == "brf32")
        .expect("the planted session is discovered");
    let read = || ae::brief::full_card_for(entry, &long.dir, Some(&root), false, world.now, None);

    // The opposed half: an unchanged record joins, both seats get a view.
    let joined = read();
    let ae::brief::Declarations::Seats(seats) = &joined.declarations else {
        panic!(
            "a readable journal names its seats: {:?}",
            joined.declarations
        );
    };
    assert_eq!(seats.len(), 2, "{seats:?}");
    assert!(seats.iter().all(|seat| seat.view.is_ok()), "{seats:?}");

    assert!(joined.topics_note.is_none(), "{:?}", joined.topics_note);
    assert_eq!(joined.card.topics.len(), 2, "{:?}", joined.card.topics);

    // The record is replaced by another session's: its uuid, its memo, and its own reasons
    // for the very same states.
    let other = "11111111-2222-4333-8444-555555555555";
    let meta = fs::read_to_string(long.dir.join("meta")).expect("the meta");
    assert!(
        fs::write(long.dir.join("meta"), meta.replace(FULL_UUID, other)).is_ok(),
        "a replaced record"
    );
    assert!(
        fs::write(
            long.dir.join("memo.tsv"),
            format!("{}\thuman\tdecision\tREPLACED-MEMO\n", ago(30))
        )
        .is_ok(),
        "a replaced memo"
    );
    let journal = [
        state_line(&ago(40), "lead", "waiting-user", "REPLACED-LEAD-REASON"),
        state_line(&ago(35), "scribe", "blocked", "REPLACED-SCRIBE-REASON"),
    ]
    .concat();
    assert!(
        fs::write(long.dir.join("events.jsonl"), journal).is_ok(),
        "a replaced journal"
    );
    let replaced = read();
    assert_eq!(
        replaced.uuid, FULL_UUID,
        "the uuid of the read that built the card"
    );
    let ae::brief::Declarations::Seats(seats) = &replaced.declarations else {
        panic!("the journal still reads: {:?}", replaced.declarations);
    };
    assert_eq!(seats.len(), 2, "{seats:?}");
    for seat in seats {
        assert_eq!(
            seat.view,
            Err("session record changed since it was read".to_owned()),
            "{seat:?}"
        );
    }
    // Withheld, and said so: no memo of the replaced session, no reason of it either. The
    // seats keep the words of the read that built the card.
    let changed = "session record changed since it was read";
    assert_eq!(
        replaced.topics_note.as_deref(),
        Some(format!("unavailable ({changed})").as_str())
    );
    assert!(
        replaced.card.topics.is_empty(),
        "{:?}",
        replaced.card.topics
    );
    let rendered = ae::brief::render_full(&[replaced]);
    assert!(
        rendered.contains(&format!("\n  topics: unavailable ({changed})\n")),
        "{rendered}"
    );
    assert!(
        !rendered.contains("REPLACED") && !rendered.contains(other),
        "{rendered}"
    );
    assert!(
        rendered.contains(LEAD_ASK) && rendered.contains(SCRIBE_BLOCK),
        "{rendered}"
    );
    assert!(!rendered.contains("view1/"), "{rendered}");
    assert!(
        rendered.contains(&format!("\n  uuid: {FULL_UUID}\n")),
        "{rendered}"
    );
}

#[test]
fn rows_appended_to_the_journal_after_the_world_read_never_reach_the_full_card() {
    // The world read the journal once and was exact. A malformed same-state row for `lead`
    // and a readable NEWER state for `scribe` land afterwards. The card, its reasons and its
    // tokens stay the world's: no later read lends a reason, and a newer state is not shown
    // against a header that never saw it.
    let root = scratch("full-appended");
    let long = plant_long(&root, "brf33");
    let (_, world) = ae::current_world(&root);
    let entry = world
        .sessions
        .iter()
        .find(|entry| entry.name == "brf33")
        .expect("the planted session is discovered");
    let read = || ae::brief::full_card_for(entry, &long.dir, Some(&root), false, world.now, None);
    let before = ae::brief::render_full(&[read()]);

    let malformed = format!(
        "{{\"ts\":\"{}\",\"actor\":\"lead\",\"action\":\"state\",\"ref\":\"waiting-user\",\"summary\":\"APPENDED-MALFORMED\u{1b}[31m\"}}\n",
        ago(20)
    );
    let newer = state_line(&ago(10), "scribe", "working", "APPENDED-NEWER-STATE");
    let journal = fs::read(long.dir.join("events.jsonl")).expect("the journal");
    let mut grown = journal;
    grown.extend_from_slice(format!("{malformed}{newer}").as_bytes());
    assert!(
        fs::write(long.dir.join("events.jsonl"), grown).is_ok(),
        "appended rows"
    );

    let after = ae::brief::render_full(&[read()]);
    assert_eq!(before, after, "the card is the world's, not the file's");
    assert!(!after.contains("APPENDED"), "{after}");
    assert!(after.contains("  declarations:\n    lead"), "{after}");
    assert!(
        after.contains(&view(
            "spawned.0",
            LAUNCH_SCRIBE,
            &long.ts[2],
            "scribe",
            "blocked",
            SCRIBE_BLOCK
        )),
        "{after}"
    );
}

#[test]
fn full_reads_running_and_stopped_sessions_and_names_their_uuid() {
    // A REAL tmux server that answers: `alive` has a tmux session and the
    // ownership marker, so it is `running`; `gone` has a durable record and no
    // tmux session, so it is `stopped`, not `unknown`. A short scratch:
    // `sun_path` is 104 bytes on macOS.
    let dir = super::cli::OwnedScratch::root("brief", "stopped").keep();
    assert!(
        fs::create_dir_all(dir.join("sessions")).is_ok(),
        "a scratch"
    );
    if !tmux_present(&dir) {
        let _ = fs::remove_dir_all(&dir);
        panic!("tmux is not runnable here, so running and stopped sessions cannot be proven");
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
    assert!(
        tmux(&["new-session", "-d", "-s", "alive"]).0,
        "a live session"
    );
    assert!(
        tmux(&["set-environment", "-t", "alive", "AE_SESSION", "alive"]).0,
        "the marker"
    );
    // Both records name the live server; each has its OWN uuid.
    let on_live_server = |session: &Path, uuid: &str| {
        let meta = fs::read_to_string(session.join("meta")).expect("the meta");
        let live = meta
            .lines()
            .map(|row| {
                if row.starts_with("tmux_server=") {
                    format!("tmux_server={}", socket.display())
                } else if row.starts_with("session_id=") {
                    format!("session_id={uuid}")
                } else {
                    row.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        assert!(
            fs::write(session.join("meta"), live).is_ok(),
            "the meta on the live server"
        );
    };
    let running_uuid = "0a1b2c3d-4e5f-4a6b-8c7d-8e9f0a1b2c3d";
    let alive = plant_full(&dir, "alive", &dir.join("work-alive"));
    on_live_server(&alive, running_uuid);
    let planted = plant_long(&dir, "gone");
    on_live_server(&planted.dir, FULL_UUID);

    let brief_full = |name: &str| {
        let out = ae()
            .env("HOME", &dir)
            .env("AE_HOME", &dir)
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &socket)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .args(["brief", name, "--full"])
            .output()
            .expect("the ae binary should run");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert_eq!(out.status.code(), Some(0), "{name}: {stderr}");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };

    let running = brief_full("alive");
    assert!(running.starts_with("alive · running"), "{running}");
    assert_eq!(
        running.lines().nth(1),
        Some(format!("  uuid: {running_uuid}").as_str()),
        "{running}"
    );

    let stopped = brief_full("gone");
    assert!(stopped.starts_with("gone · stopped"), "{stopped}");
    assert_eq!(
        stopped.lines().nth(1),
        Some(format!("  uuid: {FULL_UUID}").as_str()),
        "{stopped}"
    );
    assert!(
        stopped.contains(&view(
            "main",
            LAUNCH_LEAD,
            &planted.ts[1],
            "lead",
            "waiting-user",
            LEAD_ASK
        )),
        "{stopped}"
    );
    assert!(
        stopped.contains("END-OF-GOAL") && stopped.contains("END-OF-DECISION"),
        "{stopped}"
    );
}

/// Every file under `root`, by relative path, with its bytes.
fn snapshot(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(here) = todo.pop() {
        for entry in fs::read_dir(&here).expect("a readable fixture dir") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                todo.push(path);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .expect("under the root")
                    .to_path_buf();
                out.insert(rel, fs::read(&path).unwrap_or_default());
            }
        }
    }
    out
}

#[test]
fn full_writes_nothing() {
    let root = scratch("full-readonly");
    plant_long(&root, "brf29");
    let before = snapshot(&root);
    let (code, stdout, stderr) = run(&root, &["brief", "brf29", "--full"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains("view1/"),
        "the read must have happened: {stdout}"
    );
    let after = snapshot(&root);
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>(),
        "a file appeared or vanished"
    );
    for (path, bytes) in &before {
        assert_eq!(Some(bytes), after.get(path), "{} changed", path.display());
    }
}

#[test]
fn full_asks_git_for_dirt_without_refreshing_the_work_trees_index() {
    // A tracked file whose mtime moved but whose bytes did not makes a plain `git status` refresh
    // its stat cache and REWRITE `.git/index`: a write the read-only `--full` must never cause.
    let root = scratch("full-git-index");
    let work = root.join("work");
    assert!(fs::create_dir_all(&work).is_ok(), "the work dir");
    super::cli::git_in(&work, &["init", "-q"]);
    plant_full(&root, "brf34", &work);
    for name in ["kept.txt", "edited.txt"] {
        assert!(fs::write(work.join(name), "one\n").is_ok(), "{name}");
    }
    super::cli::git_in(&work, &["add", "kept.txt", "edited.txt"]);
    super::cli::git_in(&work, &["commit", "-q", "-m", "seed"]);
    assert!(
        fs::write(work.join("edited.txt"), "two\n").is_ok(),
        "a tracked edit, so the card has dirt to find"
    );
    let index = work.join(".git").join("index");
    let age = |secs: u64| {
        let file = fs::OpenOptions::new()
            .write(true)
            .open(work.join("kept.txt"))
            .expect("the tracked file opens");
        let times = fs::FileTimes::new()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs));
        assert!(file.set_times(times).is_ok(), "an aged mtime");
    };

    // The opposed half: the fixture is sharp. A plain status over the aged file rewrites the index.
    age(1_000_000_000);
    let before_plain = fs::read(&index).expect("the index");
    super::cli::git_in(&work, &["status", "--porcelain", "--untracked-files=no"]);
    assert_ne!(
        fs::read(&index).expect("the index"),
        before_plain,
        "a plain git status refreshes the stat cache, so this fixture can see a write"
    );

    // Aged again, to an mtime the refreshed index does not know: `--full` must leave it alone.
    age(1_100_000_000);
    let before = fs::read(&index).expect("the index");
    let (code, stdout, stderr) = run(&root, &["brief", "brf34", "--full"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout
            .lines()
            .next()
            .is_some_and(|header| header.contains(" · s1-brief* · ")),
        "the probe ran and found the tracked edit: {stdout}"
    );
    assert_eq!(
        fs::read(&index).expect("the index"),
        before,
        "`ae brief --full` rewrote the work tree's index"
    );
}

#[test]
fn full_takes_the_default_targets_and_refuses_a_seed_pack() {
    let root = scratch("full-argv");
    plant_long(&root, "zlong");
    plant_full(&root, "aquiet", &root.join("work-q"));

    // The fleet, most actionable first, each card whole; a session nobody waits on says so.
    let (code, all, stderr) = run(&root, &["brief", "--all", "--full"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let headers: Vec<&str> = all
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with(' '))
        .collect();
    assert_eq!(headers.len(), 2, "{all}");
    assert!(headers[0].starts_with("zlong · unknown · attn:"), "{all}");
    assert!(headers[1].starts_with("aquiet · unknown"), "{all}");
    assert!(
        all.contains("END-OF-GOAL") && all.contains("view1/"),
        "{all}"
    );
    assert!(all.contains("  declarations: none waiting\n"), "{all}");

    // Bare, and beside --since, in either order.
    for tail in [
        vec!["brief", "--full"],
        vec!["brief", "--full", "--since", "2h"],
        vec!["brief", "zlong", "--since", "2h", "--full"],
    ] {
        let (code, stdout, stderr) = run(&root, &tail);
        assert_eq!(code, Some(0), "{tail:?}: {stderr}");
        assert!(stdout.contains("declarations:"), "{tail:?}: {stdout}");
    }

    // A seed pack is not a card: refused, in either order, and the usage names the flag.
    for tail in [
        vec!["brief", "zlong", "--full", "--seat", "lead"],
        vec!["brief", "zlong", "--seat", "lead", "--full"],
    ] {
        let (code, stdout, stderr) = run(&root, &tail);
        assert_eq!(code, Some(2), "{tail:?}: {stderr}");
        assert!(stdout.is_empty(), "{tail:?}: {stdout}");
        assert!(stderr.contains("--full"), "{tail:?}: {stderr}");
    }
    // The default usage text names it too.
    let (_, _, usage) = run(&root, &["brief", "--frobnicate"]);
    assert!(usage.contains("[--full]"), "{usage}");
}

// ── the full ae session uuid (plan r10, S0 identity item) ───────────────────
//
// The list JSON carries only a shortened HARNESS conversation id per seat. The
// session's own uuid, the one label, rename and bound resume will take, is the
// meta's `session_id`: printed whole, lower-case, or the word `unknown` when
// no usable one is recorded.

#[test]
fn the_full_brief_carries_the_session_uuid_even_when_nobody_is_waiting() {
    let root = scratch("full-uuid");
    plant_full(&root, "brf30", &root.join("work"));

    let (code, stdout, stderr) = run(&root, &["brief", "brf30", "--full"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let mut lines = stdout.lines();
    assert!(
        lines
            .next()
            .is_some_and(|header| header.starts_with("brf30 · ")),
        "{stdout}"
    );
    assert_eq!(
        lines.next(),
        Some(format!("  uuid: {FULL_UUID}").as_str()),
        "{stdout}"
    );
    assert!(
        stdout.contains("  declarations: none waiting\n"),
        "{stdout}"
    );
    // The seat's harness conversation id is a different thing and never stands in for it.
    assert!(!stdout.contains("e795c9e9"), "{stdout}");

    // The default card is the card it always was.
    let (_, plain, _) = run(&root, &["brief", "brf30"]);
    assert!(
        !plain.contains("uuid") && !plain.contains(FULL_UUID),
        "{plain}"
    );
}

#[test]
fn a_session_with_no_usable_meta_uuid_reads_unknown_never_a_guess() {
    let root = scratch("full-uuid-unknown");
    for (name, edit) in [
        ("brf31", Edit::Drop),
        ("brf32", Edit::Replace("not-a-uuid")),
        ("brf33", Edit::Twice),
    ] {
        let dir = plant_full(&root, name, &root.join("work"));
        let meta = fs::read_to_string(dir.join("meta")).expect("the meta");
        let row = format!("session_id={FULL_UUID}\n");
        let edited = match edit {
            Edit::Drop => meta.replace(&row, ""),
            Edit::Replace(value) => meta.replace(&row, &format!("session_id={value}\n")),
            // Two rows are an unknown identity, never "the first one".
            Edit::Twice => format!("{meta}session_id=11111111-2222-4333-8444-555555555555\n"),
        };
        assert_ne!(meta, edited, "{name}");
        assert!(
            fs::write(dir.join("meta"), edited).is_ok(),
            "the edited meta"
        );

        let (code, stdout, stderr) = run(&root, &["brief", name, "--full"]);
        assert_eq!(code, Some(0), "{name}: {stderr}");
        assert_eq!(
            stdout.lines().nth(1),
            Some("  uuid: unknown"),
            "{name}: {stdout}"
        );
        assert!(
            !stdout.contains(FULL_UUID) && !stdout.contains("5555"),
            "{name}: {stdout}"
        );
    }
}

enum Edit {
    Drop,
    Replace(&'static str),
    Twice,
}

/// A planted session retargeted at the live tmux `socket`, its `session_id` row replaced by
/// `uuid_rows` (none at all when empty).
fn plant_on_server(dir: &Path, name: &str, socket: &Path, uuid_rows: &str) {
    let session = plant_full(dir, name, &dir.join(format!("work-{name}")));
    let meta = fs::read_to_string(session.join("meta")).expect("the meta");
    let rows: Vec<String> = meta
        .lines()
        .filter_map(|row| {
            if row.starts_with("tmux_server=") {
                Some(format!("tmux_server={}", socket.display()))
            } else if row.starts_with("session_id=") {
                (!uuid_rows.is_empty()).then(|| uuid_rows.to_owned())
            } else {
                Some(row.to_owned())
            }
        })
        .collect();
    assert!(
        fs::write(session.join("meta"), rows.join("\n") + "\n").is_ok(),
        "the meta of {name}"
    );
}

#[test]
fn list_all_json_carries_the_full_session_uuid_on_running_and_stopped_rows() {
    // A REAL tmux server: `alive` is a proven running row, `gone` a proven
    // stopped one (the server answers and has no such session).
    let dir = super::cli::OwnedScratch::root("brief", "listuuid").keep();
    assert!(
        fs::create_dir_all(dir.join("sessions")).is_ok(),
        "a scratch"
    );
    if !tmux_present(&dir) {
        let _ = fs::remove_dir_all(&dir);
        panic!("tmux is not runnable here, so running and stopped rows cannot be proven");
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
    assert!(
        tmux(&["new-session", "-d", "-s", "alive"]).0,
        "a live session"
    );
    assert!(
        tmux(&["set-environment", "-t", "alive", "AE_SESSION", "alive"]).0,
        "the marker"
    );

    let uuid_of = |name: &str| match name {
        "alive" => "0a1b2c3d-4e5f-4a6b-8c7d-8e9f0a1b2c3d",
        _ => "9f8e7d6c-5b4a-4c3d-8e2f-1a0b9c8d7e6f",
    };
    for name in ["alive", "gone", "nouuid", "twouuids"] {
        let uuid_rows = match name {
            "nouuid" => String::new(),
            "twouuids" => format!(
                "session_id={}\nsession_id={}",
                uuid_of(name),
                uuid_of("alive")
            ),
            _ => format!("session_id={}", uuid_of(name)),
        };
        plant_on_server(&dir, name, &socket, &uuid_rows);
    }

    let out = ae()
        .env("HOME", &dir)
        .env("AE_HOME", &dir)
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", &socket)
        .env("AE_NO_AUTOSTART", "1")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .args(["list", "--all", "--json"])
        .output()
        .expect("the ae binary should run");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    let document = ae::json::parse(&stdout).expect("the digest is JSON");
    let ae::json::Value::Arr(sessions) = document.get("sessions").expect("sessions[]") else {
        panic!("sessions is an array: {stdout}");
    };
    let row = |name: &str| {
        sessions
            .iter()
            .find(|session| session.get_str("name") == Some(name))
            .unwrap_or_else(|| panic!("{name} is listed: {stdout}"))
    };
    // Proven rows, full uuids.
    assert_eq!(row("alive").get_str("status"), Some("running"), "{stdout}");
    assert_eq!(
        row("alive").get_str("uuid"),
        Some(uuid_of("alive")),
        "{stdout}"
    );
    assert_eq!(row("gone").get_str("status"), Some("stopped"), "{stdout}");
    assert_eq!(
        row("gone").get_str("uuid"),
        Some(uuid_of("gone")),
        "{stdout}"
    );
    // The seat's harness id is its own, shortened, member; the uuid is not it.
    let seat = match row("alive").get("agents") {
        Some(ae::json::Value::Arr(agents)) => agents.first().expect("a seat"),
        other => panic!("agents[]: {other:?}"),
    };
    assert_eq!(seat.get_str("session_id"), Some("e795c9e9"), "{stdout}");
    // No usable uuid, or two claims: explicitly `unknown`.
    assert_eq!(row("nouuid").get_str("uuid"), Some("unknown"), "{stdout}");
    assert_eq!(row("twouuids").get_str("uuid"), Some("unknown"), "{stdout}");
}
