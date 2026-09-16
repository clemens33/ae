//! `ae board` over synthetic Claude transcripts — scope, coverage, rows.
//!
//! Fixtures copy `tests/it/usage.rs`: a scratch store with
//! `.claude/projects/<slug>/<id>.jsonl` SYNTHETIC records (hand-written shapes,
//! never copied transcript text) and a session dir with the meta rows
//! `harness_session.<slot>`, `agent_bin.<slot>`, `config_home.<slot>`. Most
//! pins drive `board::observe`/`board::render` directly; the exit codes ride
//! the shipped binary like `tests/it/brief.rs`.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixture setup crosses the filesystem boundary the product observes"
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use ae::board::{self, Inputs};
use ae::usage::SessionInput;

use super::cli::ae;

const SCOPE: &str = "scope: current conversations only (phase 1b) — a seat that resumed keeps only its current transcript";

const CLAUDE_ID: &str = "0199c0de-1234-4890-abcd-ef0123456789";
const OTHER_ID: &str = "0199c0de-1234-4890-abcd-ef0123456790";

fn rig(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("ae-board-it-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    root
}

/// One synthetic Claude user turn. Bodies are plain fixture prose; the helper
/// escapes what JSON strings forbid.
fn user(ts: &str, body: &str) -> String {
    let escaped = body.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        r#"{{"type":"user","timestamp":"{ts}","message":{{"role":"user","content":"{escaped}"}}}}"#
    )
}

/// Plant `sessions/<name>/meta` carrying exactly these roster rows.
fn plant_session(root: &Path, name: &str, roster: &str) -> PathBuf {
    let dir = root.join("sessions").join(name);
    std::fs::create_dir_all(&dir).expect("session dir");
    std::fs::write(dir.join("meta"), format!("schema=2\n{roster}")).expect("meta");
    dir
}

/// Plant `<store>/projects/<slug>/<id>.jsonl` with these newline-terminated lines.
fn plant_transcript(store: &Path, slug: &str, id: &str, lines: &[String]) -> PathBuf {
    let dir = store.join("projects").join(slug);
    std::fs::create_dir_all(&dir).expect("project dir");
    let path = dir.join(format!("{id}.jsonl"));
    let mut body = lines.join("\n");
    if !lines.is_empty() {
        body.push('\n');
    }
    std::fs::write(&path, body).expect("transcript");
    path
}

fn claude_roster(slot: &str, seat: &str, id: &str, store: &Path) -> String {
    format!(
        "seat.{slot}={seat}\nharness_session.{slot}={id}\nagent_bin.{slot}=claude\nconfig_home.{slot}={}\n",
        store.display()
    )
}

fn observe(root: &Path, names: &[&str], since: Option<i64>) -> board::Observation {
    let inputs: Vec<SessionInput> = names
        .iter()
        .map(|name| SessionInput {
            name: (*name).to_owned(),
            path: root.join("sessions").join(name),
        })
        .collect();
    board::observe(
        &Inputs {
            home: Some(root),
            sessions: &inputs,
        },
        since,
    )
}

/// Run the shipped binary over `root` with no pane identity.
fn run(root: &Path, tail: &[&str]) -> (Option<i32>, String, String) {
    let out = ae()
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

#[test]
fn text_scope_line_is_first_and_coverage_precedes_rows() {
    let root = rig("scope-first");
    let store = root.join("claude");
    plant_transcript(
        &store,
        "work",
        CLAUDE_ID,
        &[user("2026-09-16T09:00:00.500Z", "plain human words")],
    );
    plant_session(
        &root,
        "one",
        &format!(
            "{}seat.worker.0=colead\nagent_bin.worker.0=codex\n",
            claude_roster("main", "lead", CLAUDE_ID, &store)
        ),
    );
    let text = board::render(&observe(&root, &["one"], None), false);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], SCOPE, "the scope statement is line 1: {text}");
    let coverage = lines
        .iter()
        .position(|line| line.starts_with("coverage incomplete: "))
        .expect("a coverage row");
    let row = lines
        .iter()
        .position(|line| line.starts_with("## "))
        .expect("a body row");
    assert!(coverage < row, "coverage precedes body rows: {text}");
    assert_eq!(
        lines[coverage],
        "coverage incomplete: one:colead — invalid or missing conversation id"
    );
    assert_eq!(lines[row], "## 2026-09-16T09:00:00.500000Z one:lead");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn json_empty_board_is_scope_alone() {
    let root = rig("empty-json");
    let rendered = board::render(&observe(&root, &[], None), true);
    let lines: Vec<&str> = rendered.lines().collect();
    assert_eq!(lines.len(), 1, "scope alone on an empty board: {rendered}");
    let value = ae::json::parse(lines[0]).expect("the scope line parses");
    assert_eq!(value.get_str("kind"), Some("scope"));
    assert_eq!(value.get_str("scope"), Some("current-conversations"));
    assert_eq!(value.get_str("phase"), Some("1b"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn non_claude_seats_name_their_phase_in_both_modes() {
    let root = rig("phases");
    let mut roster = String::new();
    for (slot, seat, bin) in [
        ("main", "lead", "codex"),
        ("spawned.0", "a", "grok"),
        ("spawned.1", "b", "muse"),
        ("spawned.2", "c", "agy"),
        ("spawned.3", "d", "opencode"),
        ("spawned.4", "e", "gemini"),
    ] {
        let _ = writeln!(roster, "seat.{slot}={seat}\nagent_bin.{slot}={bin}");
    }
    plant_session(&root, "fleet", &roster);
    let observation = observe(&root, &["fleet"], None);
    assert!(observation.rows.is_empty());
    let reasons: Vec<&str> = observation
        .coverage
        .iter()
        .map(|item| item.reason.as_str())
        .collect();
    assert_eq!(
        reasons,
        [
            // Codex, Grok and Muse read now: no id, so the read is attempted and covered.
            "invalid or missing conversation id",
            "invalid or missing conversation id",
            "invalid or missing conversation id",
            "agy: phase 5",
            "opencode: ruling pending",
            "gemini: out of scope",
        ]
    );
    let text = board::render(&observation, false);
    assert_eq!(text.lines().count(), 7, "scope plus six coverage rows");
    let json = board::render(&observation, true);
    let lines: Vec<&str> = json.lines().collect();
    assert_eq!(lines.len(), 7);
    for (line, reason) in lines[1..].iter().zip(reasons) {
        let value = ae::json::parse(line).expect("every JSON line parses");
        assert_eq!(value.get_str("kind"), Some("coverage"));
        assert_eq!(value.get_str("reason"), Some(reason));
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn symlinked_transcript_is_covered_never_followed() {
    let root = rig("symlink");
    std::fs::create_dir_all(&root).expect("rig root");
    let store = root.join("claude");
    let target = root.join("real.jsonl");
    std::fs::write(
        &target,
        format!(
            "{}\n",
            user("2026-09-16T09:00:00.500Z", "words behind a link")
        ),
    )
    .expect("target");
    let dir = store.join("projects").join("work");
    std::fs::create_dir_all(&dir).expect("project dir");
    std::os::unix::fs::symlink(&target, dir.join(format!("{CLAUDE_ID}.jsonl"))).expect("link");
    plant_session(
        &root,
        "linked",
        &claude_roster("main", "lead", CLAUDE_ID, &store),
    );
    let observation = observe(&root, &["linked"], None);
    assert!(
        observation.rows.is_empty(),
        "a link is never followed into rows"
    );
    assert_eq!(observation.coverage.len(), 1);
    assert_eq!(
        observation.coverage[0].reason,
        "transcript is not a regular file"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_subagents_transcript_never_yields_a_row() {
    let root = rig("subagents");
    let store = root.join("claude");
    plant_transcript(
        &store,
        "work",
        CLAUDE_ID,
        &[user("2026-09-16T09:00:00.500Z", "parent words")],
    );
    // The sidechain carries the parent's prompt as a `user` turn — exactly
    // what would masquerade as the human if the board read it.
    let side = store
        .join("projects")
        .join("work")
        .join(CLAUDE_ID)
        .join("subagents");
    std::fs::create_dir_all(&side).expect("subagents dir");
    std::fs::write(
        side.join("agent-x.jsonl"),
        format!("{}\n", user("2026-09-16T09:01:00.500Z", "sidechain words")),
    )
    .expect("sidechain");
    plant_session(
        &root,
        "sub",
        &claude_roster("main", "lead", CLAUDE_ID, &store),
    );
    let observation = observe(&root, &["sub"], None);
    assert!(observation.coverage.is_empty());
    assert_eq!(observation.rows.len(), 1);
    assert_eq!(observation.rows[0].body, "parent words");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn two_sessions_interleave_by_ts() {
    let root = rig("interleave");
    let store = root.join("claude");
    plant_transcript(
        &store,
        "early",
        CLAUDE_ID,
        &[user("2026-09-16T10:00:00Z", "later words")],
    );
    plant_transcript(
        &store,
        "late",
        OTHER_ID,
        &[user("2026-09-16T09:00:00Z", "earlier words")],
    );
    plant_session(
        &root,
        "early",
        &claude_roster("main", "lead", CLAUDE_ID, &store),
    );
    plant_session(
        &root,
        "late",
        &claude_roster("main", "lead", OTHER_ID, &store),
    );
    // Caller order is late-first; the board sorts oldest-first regardless.
    let observation = observe(&root, &["late", "early"], None);
    let actors: Vec<&str> = observation
        .rows
        .iter()
        .map(|row| row.actor.as_str())
        .collect();
    assert_eq!(actors, ["late:lead", "early:lead"]);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn since_keeps_the_equal_timestamp_row() {
    let root = rig("since");
    let store = root.join("claude");
    plant_transcript(
        &store,
        "work",
        CLAUDE_ID,
        &[
            user("2026-09-16T08:59:59Z", "too early"),
            user("2026-09-16T09:00:00Z", "exactly since"),
            user("2026-09-16T09:00:01Z", "after"),
        ],
    );
    plant_session(
        &root,
        "day",
        &claude_roster("main", "lead", CLAUDE_ID, &store),
    );
    let args = board::parse(&["--since".to_owned(), "2026-09-16T09:00:00Z".to_owned()])
        .expect("the strict grammar parses");
    let observation = observe(&root, &["day"], args.since_micros);
    let bodies: Vec<&str> = observation
        .rows
        .iter()
        .map(|row| row.body.as_str())
        .collect();
    assert_eq!(bodies, ["exactly since", "after"]);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn bad_since_is_a_usage_error_with_the_usage_text() {
    let words = |items: &[&str]| {
        items
            .iter()
            .map(|word| (*word).to_owned())
            .collect::<Vec<_>>()
    };
    for tail in [
        words(&["--since", "yesterday"]),
        words(&["--since"]),
        words(&["--frobnicate"]),
        words(&["aedev", "aedev"]),
    ] {
        let rendered = board::parse(&tail)
            .expect_err("this argv must not parse")
            .render();
        assert!(
            rendered.starts_with("ae board: unexpected "),
            "the token first: {rendered}"
        );
        assert!(
            rendered.contains("Usage: ae board "),
            "then the usage text: {rendered}"
        );
    }
}

#[test]
fn json_lines_each_parse_with_their_kind_first() {
    let root = rig("json-kinds");
    let store = root.join("claude");
    plant_transcript(
        &store,
        "work",
        CLAUDE_ID,
        &[user(
            "2026-09-16T09:00:00.500Z",
            "quoted \"body\" stays valid",
        )],
    );
    plant_session(
        &root,
        "one",
        &format!(
            "{}seat.worker.0=colead\nagent_bin.worker.0=codex\n",
            claude_roster("main", "lead", CLAUDE_ID, &store)
        ),
    );
    let rendered = board::render(&observe(&root, &["one"], None), true);
    let lines: Vec<&str> = rendered.lines().collect();
    assert_eq!(lines.len(), 3);
    let mut kinds = Vec::new();
    for line in &lines {
        let value = ae::json::parse(line).expect("every JSON line parses");
        kinds.push(
            value
                .get_str("kind")
                .expect("every line has a kind")
                .to_owned(),
        );
    }
    assert_eq!(kinds, ["scope", "coverage", "row"]);
    let row = ae::json::parse(lines[2]).expect("the row line");
    assert_eq!(
        row.get("ts"),
        Some(&ae::json::Value::Num(1_789_549_200_500_000))
    );
    assert_eq!(row.get_str("actor"), Some("one:lead"));
    assert_eq!(row.get_str("role"), Some("human"));
    assert_eq!(row.get_str("body"), Some("quoted \"body\" stays valid"));
    assert_eq!(row.get_str("source"), Some("claude"));
    assert!(
        row.get_str("file")
            .is_some_and(|file| file.contains(".jsonl#"))
    );
    assert_eq!(row.get("offset"), Some(&ae::json::Value::Num(0)));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn binary_bad_since_exits_2_with_usage() {
    let root = rig("bin-since");
    std::fs::create_dir_all(root.join("sessions")).expect("sessions dir");
    let (code, stdout, stderr) = run(&root, &["board", "--since", "yesterday"]);
    assert_eq!(code, Some(2), "stderr: {stderr}");
    assert!(stdout.is_empty());
    assert!(
        stderr.starts_with("ae board: unexpected "),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("Usage: ae board "), "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn binary_unknown_session_exits_1_with_one_line() {
    let root = rig("bin-unknown");
    std::fs::create_dir_all(root.join("sessions")).expect("sessions dir");
    let (code, stdout, stderr) = run(&root, &["board", "nosuch"]);
    assert_eq!(code, Some(1), "stderr: {stderr}");
    assert!(stdout.is_empty());
    assert_eq!(stderr, "ae board: no session named nosuch\n");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn door_overlong_line_names_its_true_length_and_offsets_survive() {
    let root = rig("overlong");
    let store = root.join("claude");
    let overhead = user("2026-09-16T09:00:00Z", "").len();
    let big = "x".repeat(1024 * 1024 + 1 - overhead);
    let first = user("2026-09-16T09:00:00Z", &big);
    assert_eq!(first.len(), 1024 * 1024 + 1);
    plant_transcript(
        &store,
        "work",
        CLAUDE_ID,
        &[first, user("2026-09-16T09:01:00Z", "after words")],
    );
    plant_session(
        &root,
        "big",
        &claude_roster("main", "lead", CLAUDE_ID, &store),
    );
    let observation = observe(&root, &["big"], None);
    assert_eq!(observation.coverage.len(), 1);
    assert_eq!(
        observation.coverage[0].reason,
        format!(
            "1 line exceeds the 1 MiB cap (largest {} bytes)",
            1024 * 1024 + 1
        )
    );
    assert_eq!(observation.rows.len(), 1);
    assert_eq!(observation.rows[0].body, "after words");
    assert_eq!(observation.rows[0].offset, (1024 * 1024 + 2) as u64);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn binary_torn_tail_keeps_earlier_rows_and_covers_the_tail() {
    let root = rig("bin-torn");
    let store = root.join("claude");
    let dir = store.join("projects").join("work");
    std::fs::create_dir_all(&dir).expect("project dir");
    std::fs::write(
        dir.join(format!("{CLAUDE_ID}.jsonl")),
        format!(
            "{}\n{}",
            user("2026-09-16T09:00:00Z", "kept words"),
            user("2026-09-16T09:01:00Z", "torn words")
        ),
    )
    .expect("torn transcript");
    plant_session(
        &root,
        "torn",
        &claude_roster("main", "lead", CLAUDE_ID, &store),
    );
    let (code, stdout, stderr) = run(&root, &["board", "torn"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains("coverage incomplete: torn:lead — torn last record"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("## 2026-09-16T09:00:00.000000Z torn:lead\nkept words\n"),
        "stdout: {stdout}"
    );
    assert!(
        !stdout.contains("torn words"),
        "the torn tail is never trusted: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

const CODEX_ID: &str = "01a08046-1974-7352-ade3-81a786200795";

/// One synthetic Codex user turn. Bodies are plain fixture prose.
fn codex_user(ts: &str, body: &str) -> String {
    let escaped = body
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    format!(
        r#"{{"timestamp":"{ts}","type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"{escaped}"}}]}}}}"#
    )
}

fn codex_roster(slot: &str, seat: &str, id: &str, store: &Path) -> String {
    format!(
        "seat.{slot}={seat}\nharness_session.{slot}={id}\nagent_bin.{slot}=codex\nconfig_home.{slot}={}\n",
        store.display()
    )
}

/// Plant `<store>/sessions/2026/09/08/rollout-…-{CODEX_ID}.jsonl`: the id's
/// embedded time picks the day, as in `tests/it/usage.rs`.
fn plant_rollout(store: &Path, lines: &[String]) {
    let dir = store.join("sessions/2026/09/08");
    std::fs::create_dir_all(&dir).expect("rollout day dir");
    let mut body = lines.join("\n");
    if !lines.is_empty() {
        body.push('\n');
    }
    let path = dir.join(format!("rollout-2026-09-08T09-00-00-{CODEX_ID}.jsonl"));
    std::fs::write(path, body).expect("rollout");
}

#[test]
fn a_codex_seat_renders_rows_and_names_no_phase() {
    let root = rig("codex-rows");
    let store = root.join("codex");
    plant_rollout(
        &store,
        &[
            codex_user("2026-09-16T09:00:00.500Z", "codex human words"),
            codex_user("2026-09-16T09:01:00Z", "⟦ae:msg from lead⟧\nnot human"),
            codex_user("2026-09-16T09:02:00Z", "<environment_context>\nnoise"),
        ],
    );
    plant_session(
        &root,
        "ship",
        &codex_roster("main", "lead", CODEX_ID, &store),
    );
    let observation = observe(&root, &["ship"], None);
    assert!(observation.coverage.is_empty(), "no phase row after a read");
    assert_eq!(observation.rows.len(), 1);
    assert_eq!(observation.rows[0].body, "codex human words");
    let text = board::render(&observation, false);
    assert!(!text.contains("phase 2"), "the phase row is gone: {text}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn claude_and_codex_seats_interleave_by_ts() {
    let root = rig("codex-interleave");
    let claude_store = root.join("claude");
    plant_transcript(
        &claude_store,
        "work",
        CLAUDE_ID,
        &[user("2026-09-16T10:00:00Z", "claude later words")],
    );
    let codex_store = root.join("codex");
    plant_rollout(
        &codex_store,
        &[codex_user("2026-09-16T09:00:00Z", "codex earlier words")],
    );
    plant_session(
        &root,
        "mixed",
        &format!(
            "{}{}",
            claude_roster("main", "lead", CLAUDE_ID, &claude_store),
            codex_roster("worker.0", "colead", CODEX_ID, &codex_store),
        ),
    );
    let observation = observe(&root, &["mixed"], None);
    assert!(observation.coverage.is_empty());
    let actors: Vec<&str> = observation
        .rows
        .iter()
        .map(|row| row.actor.as_str())
        .collect();
    assert_eq!(actors, ["mixed:colead", "mixed:lead"]);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_missing_codex_rollout_is_a_coverage_row() {
    let root = rig("codex-missing");
    let store = root.join("codex");
    std::fs::create_dir_all(store.join("sessions/2026/09/08")).expect("day dir");
    plant_session(
        &root,
        "gone",
        &codex_roster("main", "lead", CODEX_ID, &store),
    );
    let observation = observe(&root, &["gone"], None);
    assert!(observation.rows.is_empty());
    assert_eq!(observation.coverage.len(), 1);
    assert_eq!(observation.coverage[0].reason, "rollout not found");
    let _ = std::fs::remove_dir_all(&root);
}

const GROK_ID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const GROK_ABSENT: &str = "0199c0de-bbbb-4890-abcd-ef0123456789";
const GROK_DUP: &str = "0199c0de-cccc-4890-abcd-ef0123456789";

/// One synthetic Grok user chunk, stamped both ways. Bodies are plain prose.
fn grok_user(body: &str) -> String {
    let escaped = body.replace('"', "\\\"").replace('\n', "\\n");
    format!(
        r#"{{"timestamp":1789549200,"method":"session/update","params":{{"sessionId":"s","update":{{"sessionUpdate":"user_message_chunk","_meta":{{"agentTimestampMs":1789549200500}},"content":{{"type":"text","text":"{escaped}"}}}}}}}}"#
    )
}

fn grok_roster(slot: &str, seat: &str, id: &str) -> String {
    format!("seat.{slot}={seat}\nharness_session.{slot}={id}\nagent_bin.{slot}=grok\n")
}

/// Plant `.grok/sessions/<cwd>/<id>/updates.jsonl` with these lines.
fn plant_grok(root: &Path, cwd: &str, id: &str, lines: &[String]) {
    let dir = root.join(".grok/sessions").join(cwd).join(id);
    std::fs::create_dir_all(&dir).expect("grok dir");
    std::fs::write(dir.join("updates.jsonl"), lines.join("\n") + "\n").expect("updates");
}

#[test]
fn a_grok_seat_renders_rows_and_names_no_phase() {
    let root = rig("grok-rows");
    plant_grok(
        &root,
        "work",
        GROK_ID,
        &[
            grok_user("grok human words"),
            grok_user("⟦ae:msg from lead⟧\nnot human"),
        ],
    );
    plant_session(&root, "ship", &grok_roster("main", "lead", GROK_ID));
    let observation = observe(&root, &["ship"], None);
    assert!(observation.coverage.is_empty(), "no phase row after a read");
    assert_eq!(observation.rows.len(), 1);
    assert_eq!(observation.rows[0].body, "grok human words");
    let text = board::render(&observation, false);
    assert!(!text.contains("phase 3a"), "the phase row is gone: {text}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn grok_locate_failures_are_coverage_rows() {
    let root = rig("grok-locate");
    plant_session(&root, "linked", &grok_roster("main", "lead", GROK_ID));
    let words = grok_user("words behind a link") + "\n";
    std::fs::write(root.join("real.jsonl"), words).expect("target");
    let dir = root.join(".grok/sessions").join("work").join(GROK_ID);
    std::fs::create_dir_all(&dir).expect("grok dir");
    std::os::unix::fs::symlink(root.join("real.jsonl"), dir.join("updates.jsonl")).expect("link");
    plant_session(&root, "gone", &grok_roster("main", "lead", GROK_ABSENT));
    plant_grok(&root, "one", GROK_DUP, &[grok_user("first")]);
    plant_grok(&root, "two", GROK_DUP, &[grok_user("second")]);
    plant_session(&root, "dup", &grok_roster("main", "lead", GROK_DUP));
    let observation = observe(&root, &["linked", "gone", "dup"], None);
    assert!(observation.rows.is_empty(), "locate failures yield no rows");
    let reasons: Vec<&str> = observation
        .coverage
        .iter()
        .map(|item| item.reason.as_str())
        .collect();
    assert_eq!(
        reasons,
        [
            "transcript is not a regular file",
            "transcript not found",
            "conversation id is not unique",
        ]
    );
    let _ = std::fs::remove_dir_all(&root);
}

const MUSE_ID: &str = "0199c0de-dddd-4890-abcd-ef0123456789";
const MUSE_ABSENT: &str = "0199c0de-eeee-4890-abcd-ef0123456789";
const MUSE_ENV: &str = r#"{"recorded_at":1789565338436454,"payload_type":"runtime.user_intent.accepted","payload":{"refill_blocks":[{"kind":"text","text":"stub"}],"model_messages":[{"content":[{"kind":"text","text":"{b}"}]}]}}"#;

/// One synthetic Muse turn: model text plus a bare stub refill.
fn muse_user(body: &str) -> String {
    MUSE_ENV.replace("{b}", &body.replace('\n', "\\n"))
}

fn muse_roster(slot: &str, seat: &str, id: &str) -> String {
    format!("seat.{slot}={seat}\nharness_session.{slot}={id}\nagent_bin.{slot}=muse\n")
}

/// Plant `.local/share/muse/sessions/<day>/<id>/session.jsonl` with these lines.
fn plant_muse(root: &Path, day: &str, id: &str, lines: &[String]) {
    let dir = root.join(".local/share/muse/sessions").join(day).join(id);
    std::fs::create_dir_all(&dir).expect("muse dir");
    std::fs::write(dir.join("session.jsonl"), lines.join("\n") + "\n").expect("session");
}

#[test]
fn a_muse_seat_renders_rows_and_names_no_phase() {
    let root = rig("muse-rows");
    let one = muse_user("muse human words");
    let two = muse_user("⟦ae:ctx⟧\nnot human");
    plant_muse(&root, "2026/09/16", MUSE_ID, &[one, two]);
    plant_session(&root, "ship", &muse_roster("main", "lead", MUSE_ID));
    let observation = observe(&root, &["ship"], None);
    assert!(observation.coverage.is_empty(), "no phase row after a read");
    assert_eq!(observation.rows.len(), 1);
    assert_eq!(observation.rows[0].body, "muse human words");
    let text = board::render(&observation, false);
    assert!(!text.contains("phase 3b"), "the phase row is gone: {text}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn muse_locate_failures_are_coverage_rows() {
    let root = rig("muse-locate");
    plant_session(&root, "linked", &muse_roster("main", "lead", MUSE_ID));
    std::fs::write(root.join("real.jsonl"), muse_user("linked words") + "\n").expect("target");
    let dir = root.join(format!(".local/share/muse/sessions/2026/09/16/{MUSE_ID}"));
    std::fs::create_dir_all(&dir).expect("muse dir");
    std::os::unix::fs::symlink(root.join("real.jsonl"), dir.join("session.jsonl")).expect("link");
    plant_session(&root, "gone", &muse_roster("main", "lead", MUSE_ABSENT));
    let observation = observe(&root, &["linked", "gone"], None);
    assert!(observation.rows.is_empty(), "locate failures yield no rows");
    assert_eq!(observation.coverage.len(), 2);
    assert_eq!(
        observation.coverage[0].reason,
        "transcript is not a regular file"
    );
    assert_eq!(observation.coverage[1].reason, "transcript not found");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn binary_empty_board_prints_scope_only() {
    let root = rig("bin-empty");
    std::fs::create_dir_all(root.join("sessions")).expect("sessions dir");
    let (code, stdout, stderr) = run(&root, &["board"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, format!("{SCOPE}\n"));
    let (code, stdout, stderr) = run(&root, &["board", "--json"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        "{\"kind\":\"scope\",\"scope\":\"current-conversations\",\"phase\":\"1b\"}\n"
    );
    let _ = std::fs::remove_dir_all(&root);
}
