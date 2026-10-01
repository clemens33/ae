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

/// A console reply whose stored body is a FIFO is refused by name, never
/// opened: the one-shot ends and shows the answer's summary.
#[test]
fn a_console_reply_body_that_is_a_fifo_is_refused_never_opened() {
    let (root, dir) = lead_pair_rig("console-fifo");
    let id = "ae-20260916T091000Z-0000abcd";
    let fifo = dir.join(format!("messages/{id}.reply.0000aa.txt"));
    std::fs::create_dir_all(dir.join("messages")).expect("messages");
    super::cli::mkfifo(&fifo);
    let uuid = SESSION_ID;
    let ask = format!(
        r#"{{"ts":"2026-09-16T09:10:00Z","actor":"console:local","action":"ask","target":"lead","ref":"{id}","target_slot":"main","target_session":"one","target_server":"/t","target_pane":"%1","target_session_uuid":"{uuid}","summary":"q"}}"#
    );
    let body = esc(&fifo.display().to_string());
    let reply = format!(
        r#"{{"ts":"2026-09-16T09:11:00Z","actor":"lead","action":"reply","target":"console:local","ref":"{id}","actor_slot":"main","actor_session":"one","caller_server":"/t","caller_pane":"%1","caller_session_uuid":"{uuid}","body_file":"{body}","summary":"the summary"}}"#
    );
    let journal = std::fs::read_to_string(dir.join("events.jsonl")).expect("journal");
    let journal = format!("{}\n{ask}\n{reply}\n", journal.trim_end());
    std::fs::write(dir.join("events.jsonl"), journal).expect("journal");
    let (code, stdout, stderr) = console(&root, &["one"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let refused = format!(
        "lead answers {id} · body refused: a fifo · preview (600-char summary)\n  the summary\n"
    );
    assert!(stdout.contains(&refused), "{stdout}");
    let _ = std::fs::remove_dir_all(&root);
}

/// A console draft that is a FIFO is classified and refused, never opened: no
/// writer ever comes, so an open would wait for good.
#[test]
fn a_console_draft_that_is_a_fifo_is_refused_never_opened() {
    let root = rig("console-draft-fifo");
    super::cli::mkfifo(&root.join(ae::store::CONSOLE_DRAFT));
    let (read, done) = std::sync::mpsc::channel();
    let dir = root.clone();
    std::thread::spawn(move || read.send(ae::store::open(&dir).console_draft()));
    let got = done.recv_timeout(std::time::Duration::from_secs(10));
    let refused = Ok(Ok(ae::store::SourceRead::Invalid("a fifo".to_owned())));
    assert_eq!(got, refused, "classified before any open");
    let _ = std::fs::remove_dir_all(&root);
}

/// Run `runner`, waiting at most 30 s: a console that follows when it
/// should print once fails with a red that ARRIVES, never a stalled lane.
fn bounded_output(runner: &mut super::cli::Runner) -> std::process::Output {
    let child = runner
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("the ae binary should run");
    let limit = std::time::Duration::from_secs(30);
    super::cli::bounded(child, limit).expect("the ae binary did not exit within 30 s")
}

fn console(root: &std::path::Path, tail: &[&str]) -> (Option<i32>, String, String) {
    let mut runner = super::cli::ae();
    runner
        .env("HOME", root)
        .env("AE_HOME", root)
        .env_remove("TMUX_PANE")
        .stdin(std::process::Stdio::null())
        .args(["console"])
        .args(tail);
    let out = bounded_output(&mut runner);
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
fn a_console_asked_for_input_on_no_terminal_says_so_and_only_reads() {
    let (root, dir) = lead_pair_rig("console-no-tty");
    let before = std::fs::read(dir.join("events.jsonl")).expect("journal");
    let (code, stdout, stderr) = console(&root, &["one", "--input"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let off = "input off: stdin is not a terminal; this console only reads\n";
    let golden = include_str!("../fixtures/console/lane.txt");
    assert!(stdout.contains(off), "{stdout}");
    assert_eq!(stdout.replacen(off, "", 1), golden);
    let meta = std::fs::read_to_string(dir.join("meta")).expect("meta");
    let bad = meta.replace("seat.main=lead\n", "seat.main=%3\n");
    std::fs::write(dir.join("meta"), bad).expect("a main seat that is no agent name");
    let (code, stdout, _) = console(&root, &["one", "--input"]);
    let refused = "input off: the main seat is not an agent name; this console only reads\n";
    assert!(code == Some(0) && stdout.contains(refused), "{stdout}");
    let after = std::fs::read(dir.join("events.jsonl")).expect("journal");
    assert_eq!(after, before);
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

/// The key verb reads `[--jump] [--client <name>]` and nothing else: anything
/// more is a usage error (exit 2) before tmux is asked; a well-formed argv goes
/// on to the pane it cannot find on a socket nothing listens at (exit 1).
#[test]
fn the_key_verb_argv_is_a_usage_error_unless_it_is_jump_and_client_alone() {
    let root = rig("console-key-argv");
    let socket = root.join("nothing-listens.sock");
    let cases: [(&[&str], i32); 10] = [
        (&[], 1),
        (&["--jump"], 1),
        (&["--client", "c"], 1),
        (&["--jump", "--client", "c"], 1),
        (&["--frob"], 2),
        (&["--frob", "c"], 2),
        (&["--jump", "--frob"], 2),
        (&["--jump", "--jump"], 2),
        (&["--client"], 2),
        (&["--client", "c", "d"], 2),
    ];
    for (words, code) in cases {
        let mut runner = super::cli::ae();
        runner
            .env("HOME", &root)
            .env("AE_HOME", &root)
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &socket)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .arg("_console")
            .args(words);
        let out = bounded_output(&mut runner);
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert_eq!(out.status.code(), Some(code), "{words:?}: {stderr}");
        let usage = stderr.starts_with("usage: ae _console [--jump] [--client <name>]");
        assert_eq!(usage, code == 2, "{words:?}: {stderr}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// Run a helper ENTRY against `dir` with no pane and no server, and
/// `override_as` as `AE_SENDER_OVERRIDE` when given.
fn helper_run(
    root: &std::path::Path,
    dir: &std::path::Path,
    entry: &str,
    tail: &[&str],
    override_as: Option<&str>,
) -> (Option<i32>, String) {
    let mut runner = super::cli::ae();
    runner
        .env("HOME", root)
        .env("AE_HOME", root)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("AE_SENDER_OVERRIDE")
        .arg(entry)
        .arg(dir)
        .args(tail);
    if let Some(value) = override_as {
        runner.env("AE_SENDER_OVERRIDE", value);
    }
    let out = bounded_output(&mut runner);
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The console sink takes replies only, and no helper may speak AS the
/// console: both are refused as usage, before a pane, a server or the
/// journal is touched.
#[test]
fn nothing_speaks_as_the_console_and_only_a_reply_is_sent_to_it() {
    let (root, dir) = lead_pair_rig("console-sink");
    let before = super::cli::byte_tree(&root);
    for target in ["console:local", "console:x", "console:"] {
        for entry in [ae::cli::SEND, ae::cli::ASK, ae::cli::REVIEW] {
            let (code, err) = helper_run(&root, &dir, entry, &[target, "hello"], None);
            assert_eq!(code, Some(2), "{entry} {target}: {err}");
            assert!(
                err.contains("reply <id>") && err.contains("say") && err.contains("waiting-user"),
                "{entry} {target} names the working channels: {err}"
            );
        }
    }
    let calls: [(&str, &[&str]); 8] = [
        (ae::cli::SEND, &["lead", "hi"]),
        (ae::cli::ASK, &["lead", "q"]),
        (ae::cli::REVIEW, &["lead", "r"]),
        (ae::cli::REPLY, &["ae-20260930T120000Z-0000abcd", "a"]),
        (ae::cli::SAY, &["hi"]),
        (ae::cli::STATE, &["working", "x"]),
        (ae::cli::MEMO, &["read"]),
        (ae::cli::REQUESTS, &["all"]),
    ];
    for value in ["console:local", "console:x"] {
        for (entry, tail) in calls {
            let (code, err) = helper_run(&root, &dir, entry, tail, Some(value));
            assert_eq!(code, Some(2), "{entry} as {value}: {err}");
            assert!(err.contains("AE_SENDER_OVERRIDE"), "{entry}: {err}");
        }
    }
    // The short form is the same ingress as the link and the entry.
    let session = dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let mut runner = super::cli::ae();
    runner
        .env("HOME", &root)
        .env("AE_HOME", &root)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env("AE_SENDER_OVERRIDE", "console:local")
        .args([format!("@{session}").as_str(), "send", "lead", "hi"]);
    let out = bounded_output(&mut runner);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("AE_SENDER_OVERRIDE"), "short form: {err}");
    let (_, err) = helper_run(&root, &dir, "_console", &[], Some("console:local"));
    assert!(err.starts_with("usage: ae _console"), "{err}");
    assert_eq!(super::cli::byte_tree(&root), before, "nothing was written");
    let _ = std::fs::remove_dir_all(&root);
}
