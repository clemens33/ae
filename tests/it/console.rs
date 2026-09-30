//! `ae console` phase 1 — the read view of a session's lead pair.
//!
//! Fixtures reuse the board's synthetic Claude stores: the console reads
//! transcripts through the board's one seat read, so what it must NOT show
//! (a marked turn, a worker's words) is pinned here.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixture setup crosses the filesystem boundary the product observes"
)]

use ae::board::{self, Inputs};
use ae::usage::SessionInput;

use super::board::{claude_roster, plant_session, plant_transcript, rig, user};

const LEAD_ID: &str = "0199c0de-1234-4890-abcd-ef0123456789";
const SCOUT_ID: &str = "0199c0de-1234-4890-abcd-ef0123456790";

#[test]
fn a_seat_filtered_read_yields_lead_human_turns_and_never_a_marked_or_worker_one() {
    let root = rig("console-filtered");
    let store = root.join("claude");
    let marked = ae::provenance::peer("worker");
    let lines = [
        user("2026-09-16T09:00:00Z", "human one"),
        user("2026-09-16T09:01:00Z", &marked),
    ];
    plant_transcript(&store, "work", LEAD_ID, &lines);
    let lines = [user("2026-09-16T09:04:00Z", "worker words")];
    plant_transcript(&store, "work", SCOUT_ID, &lines);
    let roster = format!(
        "{}{}",
        claude_roster("main", "lead", LEAD_ID, &store),
        claude_roster("worker.1", "scout", SCOUT_ID, &store)
    );
    plant_session(&root, "one", &roster);
    let sessions = [SessionInput {
        name: "one".to_owned(),
        path: root.join("sessions").join("one"),
    }];
    let inputs = Inputs {
        home: Some(&root),
        sessions: &sessions,
        assistant: false,
    };
    let bodies = |keep: &dyn Fn(&ae::meta::RosterEntry) -> bool| -> Vec<String> {
        let seen = board::observe_selected(&inputs, None, None, keep);
        seen.rows.into_iter().map(|row| row.body).collect()
    };
    assert_eq!(bodies(&|entry| entry.slot == "main"), ["human one"]);
    assert_eq!(
        bodies(&|_| true),
        ["human one", "worker words"],
        "the filter alone kept the worker out"
    );
    let _ = std::fs::remove_dir_all(&root);
}

const SESSION_ID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const COLEAD_ID: &str = "0199c0de-1234-4890-abcd-ef0123456791";

fn esc(text: &str) -> String {
    let mut out = String::new();
    ae::json::escape_into(text, &mut out);
    out
}

fn line(ts: &str, actor: &str, action: &str, target: &str, text: &str) -> String {
    let (actor, target, text) = (esc(actor), esc(target), esc(text));
    format!(
        r#"{{"ts":"{ts}","actor":"{actor}","action":"{action}","target":"{target}","ref":"r-1","summary":"{text}"}}"#
    )
}

fn declared(ts: &str, actor: &str, state: &str, reason: &str) -> String {
    let reason = esc(reason);
    format!(
        r#"{{"ts":"{ts}","actor":"{actor}","action":"state","ref":"{state}","summary":"{reason}"}}"#
    )
}

fn human(ts: &str, body: &str) -> String {
    format!(
        r#"{{"type":"user","timestamp":"{ts}","message":{{"role":"user","content":"{}"}}}}"#,
        esc(body)
    )
}

/// A lead-pair session `one`: a lead (marked, wrapped and multi-line turns), a
/// colead with a torn last record, a worker that must never show, and a journal
/// with a bridge thread, `say` lines, a superseded and a current card and a
/// standing prompt. Returns the session directory.
fn lead_pair_rig(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let root = rig(tag);
    let store = root.join("claude");
    let wrapped = include_str!("../fixtures/board/claude-wrapped-ae-turn.txt");
    let marked = ae::provenance::peer("worker");
    let lead = [
        user("2026-09-16T09:00:00Z", "human one"),
        human("2026-09-16T09:01:30Z", wrapped),
        human("2026-09-16T09:02:00Z", &marked),
        human("2026-09-16T09:03:00Z", "code:\n\tindented\n  two"),
    ];
    plant_transcript(&store, "work", LEAD_ID, &lead);
    let answer = r#"{"type":"assistant","timestamp":"2026-09-16T09:04:30Z","message":{"role":"assistant","content":[{"type":"text","text":"colead answers"}]}}"#;
    let torn = format!(
        "{}\n{answer}\n{}",
        user("2026-09-16T09:04:00Z", "colead words"),
        user("2026-09-16T09:05:00Z", "torn words")
    );
    let project = store.join("projects").join("work");
    std::fs::write(project.join(format!("{COLEAD_ID}.jsonl")), torn).expect("colead");
    plant_transcript(
        &store,
        "work",
        SCOUT_ID,
        &[user("2026-09-16T09:06:00Z", "worker words")],
    );
    let roster = format!(
        "session_id={SESSION_ID}\nlayout=lead-pair\n{}{}{}",
        claude_roster("main", "lead", LEAD_ID, &store),
        claude_roster("worker.0", "colead", COLEAD_ID, &store),
        claude_roster("worker.1", "scout", SCOUT_ID, &store)
    );
    let dir = plant_session(&root, "one", &roster);
    let hostile = "telegram:9\u{1b}]52;c;AAAA\u{7}";
    let journal = [
        line(
            "2026-09-16T09:10:00Z",
            "telegram:42",
            "send",
            "lead",
            "deploy now?",
        ),
        line(
            "2026-09-16T09:10:30Z",
            hostile,
            "send",
            "colead\u{1b}[2J",
            "hi\rthere",
        ),
        line(
            "2026-09-16T09:11:00Z",
            "lead",
            "reply",
            "telegram:42",
            "deploy started",
        ),
        line(
            "2026-09-16T09:12:00Z",
            "colead",
            "chat",
            "",
            "colead announces",
        ),
        line(
            "2026-09-16T09:12:30Z",
            "watchdog",
            "chat",
            "",
            "moved a seat",
        ),
        line(
            "2026-09-16T09:13:00Z",
            "scout",
            "chat",
            "",
            "a worker speaks",
        ),
        declared(
            "2026-09-16T09:14:00Z",
            "lead",
            "waiting-user",
            "old question",
        ),
        declared("2026-09-16T09:15:00Z", "lead", "working", "answered"),
        declared(
            "2026-09-16T09:16:00Z",
            "colead",
            "waiting-user",
            "pick a name\tfor v2",
        ),
        line(
            "2026-09-16T09:17:00Z",
            "watchdog",
            "human-prompt",
            "colead",
            "Trust this folder?",
        ),
        "not json".to_owned(),
    ];
    std::fs::write(dir.join("events.jsonl"), journal.join("\n") + "\n").expect("journal");
    (root, dir)
}

fn console(root: &std::path::Path, tail: &[&str]) -> (Option<i32>, String, String) {
    let out = super::cli::ae()
        .env("HOME", root)
        .env("AE_HOME", root)
        .env_remove("TMUX_PANE")
        .args(["console"])
        .args(tail)
        .output()
        .expect("the ae binary should run");
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    (out.status.code(), text(&out.stdout), text(&out.stderr))
}

#[test]
fn the_console_prints_the_lane_golden_and_writes_nothing() {
    let (root, dir) = lead_pair_rig("console-golden");
    let before = std::fs::read(dir.join("events.jsonl")).expect("journal");
    let (code, stdout, stderr) = console(&root, &["one"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, include_str!("../fixtures/console/lane.txt"));
    assert_eq!(
        std::fs::read(dir.join("events.jsonl")).expect("journal"),
        before
    );
    assert!(
        !stdout.contains("colead answers"),
        "assistant rows are off by default"
    );
    let (_, all, _) = console(&root, &["one", "--all"]);
    assert!(
        all.contains("## 09:04:30 colead assistant (transcript)\n  colead answers\n"),
        "{all}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_bad_flag_is_a_usage_error_and_an_unknown_session_is_one_line() {
    let root = rig("console-refuse");
    std::fs::create_dir_all(root.join("sessions")).expect("sessions");
    let (code, stdout, stderr) = console(&root, &["--frobnicate"]);
    assert_eq!((code, stdout.as_str()), (Some(2), ""), "{stderr}");
    assert!(stderr.starts_with("ae console: unexpected --frobnicate\nUsage: ae console"));
    let (code, stdout, stderr) = console(&root, &["nosuch"]);
    assert_eq!((code, stdout.as_str()), (Some(1), ""));
    assert_eq!(stderr, "ae console: no session named nosuch\n");
    let hostile = "--x\u{1b}]52;c;AAAA\u{7}\r";
    let (code, _, stderr) = console(&root, &[hostile]);
    assert_eq!(code, Some(2));
    assert!(
        !stderr.chars().any(|ch| ch.is_control() && ch != '\n'),
        "{stderr:?}"
    );
    assert!(
        stderr.contains("--x\u{fffd}]52;c;AAAA\u{fffd}\u{fffd}"),
        "{stderr:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_session_without_a_canonical_id_is_refused_before_anything_is_read() {
    for (tag, id) in [("absent", ""), ("malformed", "session_id=not-a-uuid\n")] {
        let root = rig(&format!("console-id-{tag}"));
        let store = root.join("claude");
        let roster = format!(
            "{id}layout=lead-pair\n{}",
            claude_roster("main", "lead", LEAD_ID, &store)
        );
        plant_session(&root, "one", &roster);
        let (code, stdout, stderr) = console(&root, &["one"]);
        assert_eq!((code, stdout.as_str()), (Some(1), ""), "{tag}: {stderr}");
        assert!(stderr.contains("session_id"), "{tag}: {stderr}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
