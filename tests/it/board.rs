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

const SCOPE: &str = "scope: current conversations plus each seat's recorded predecessors (up to 4, newest first) — nothing is inferred from time";

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

/// One synthetic Claude assistant turn; `content` is the parts array.
fn claude_assistant(ts: &str, content: &str) -> String {
    format!(
        r#"{{"type":"assistant","timestamp":"{ts}","message":{{"role":"assistant","content":{content}}}}}"#
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
    observe_with(root, names, since, false)
}

fn observe_with(
    root: &Path,
    names: &[&str],
    since: Option<i64>,
    assistant: bool,
) -> board::Observation {
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
            assistant,
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
    let text = board::render(&observe(&root, &["one"], None), false, None);
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
    assert_eq!(lines[row], "## 09:00:00 one:lead");
    assert_eq!(lines[row - 1], "# 2026-09-16 UTC", "divider above: {text}");
    assert_eq!(lines[row + 1], "  plain human words", "body is indented");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn json_empty_board_is_scope_alone() {
    let root = rig("empty-json");
    let rendered = board::render(&observe(&root, &[], None), true, None);
    let lines: Vec<&str> = rendered.lines().collect();
    assert_eq!(lines.len(), 1, "scope alone on an empty board: {rendered}");
    let value = ae::json::parse(lines[0]).expect("the scope line parses");
    assert_eq!(value.get_str("kind"), Some("scope"));
    assert_eq!(
        value.get_str("scope"),
        Some("current-and-recorded-predecessors")
    );
    assert_eq!(value.get_str("phase"), Some("8b"));
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
            // Codex, Grok, Muse, agy and opencode read now: no id, so the
            // read is attempted and covered (for opencode the export leg
            // refuses the id before any spawn).
            "invalid or missing conversation id",
            "invalid or missing conversation id",
            "invalid or missing conversation id",
            "invalid or missing conversation id",
            "invalid or missing conversation id",
            "gemini: out of scope",
        ]
    );
    let text = board::render(&observation, false, None);
    assert_eq!(text.lines().count(), 7, "scope plus six coverage rows");
    let json = board::render(&observation, true, None);
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
fn ae_injected_turns_are_hidden_and_counted_per_seat() {
    let root = rig("hidden");
    let store = root.join("claude");
    // The live store writes a leading space before the marker: that shape
    // slips past the reader's own line-1 check, so the board must hide it.
    plant_transcript(
        &store,
        "work",
        CLAUDE_ID,
        &[
            user("2026-09-16T09:00:00Z", "human one"),
            user(
                "2026-09-16T09:01:00Z",
                &format!(" {}", ae::provenance::peer("worker")),
            ),
            user("2026-09-16T09:02:00Z", "human two"),
            user(
                "2026-09-16T09:03:00Z",
                &format!(" {}", ae::provenance::ctx()),
            ),
        ],
    );
    plant_transcript(
        &store,
        "work",
        PRIOR_NEW_ID,
        &[user(
            "2026-09-15T09:00:00Z",
            &format!(" {}", ae::provenance::brief("lead")),
        )],
    );
    plant_session(
        &root,
        "one",
        &format!(
            "{}harness_session_prior.main={PRIOR_NEW_ID}\n",
            claude_roster("main", "lead", CLAUDE_ID, &store)
        ),
    );
    let observation = observe(&root, &["one"], None);
    let bodies: Vec<&str> = observation
        .rows
        .iter()
        .map(|row| row.body.as_str())
        .collect();
    assert_eq!(bodies, ["human one", "human two"]);
    assert_eq!(
        (observation.hidden.len(), observation.hidden[0].count),
        (1, 3),
        "the predecessor's hidden turn counts under the same actor"
    );
    let text = board::render(&observation, false, None);
    assert_eq!(
        text.lines().nth(1),
        Some("hidden: one:lead — 3 ae-injected turns"),
        "{text}"
    );
    let json = board::render(&observation, true, None);
    assert!(
        json.contains("{\"kind\":\"hidden\",\"actor\":\"one:lead\",\"count\":3}\n"),
        "{json}"
    );
    // `--since` (2026-09-16T00:00:00Z) applies before counting.
    let after = observe(&root, &["one"], Some(1_789_516_800_000_000));
    assert_eq!((after.hidden.len(), after.hidden[0].count), (1, 2));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_codex_passive_launch_turn_is_hidden_not_rendered() {
    let root = rig("hidden-codex");
    let store = root.join("codex");
    let session = root.join("sessions").join("ship");
    let prompt = ae::launch::initial_prompt_for(ae::tool::ToolKind::Codex, &session, "main");
    let body = prompt.split_once('\n').expect("marker plus body").1;
    plant_rollout(
        &store,
        &[
            codex_user("2026-09-16T09:00:00Z", body),
            codex_user("2026-09-16T09:01:00Z", "codex human words"),
        ],
    );
    plant_session(
        &root,
        "ship",
        &codex_roster("main", "lead", CODEX_ID, &store),
    );
    let observation = observe(&root, &["ship"], None);
    assert_eq!(
        observation.hidden,
        [board::Hidden {
            actor: "ship:lead".to_owned(),
            count: 1,
        }]
    );
    assert_eq!(observation.rows.len(), 1);
    assert_eq!(observation.rows[0].body, "codex human words");
    let text = board::render(&observation, false, None);
    assert!(
        text.contains("hidden: ship:lead — 1 ae-injected turns"),
        "{text}"
    );
    assert!(!text.contains("_register-sid"), "gone: {text}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_board_respells_no_provenance_marker() {
    let source = include_str!("../../src/board.rs");
    assert!(
        !source.contains("⟦ae:"),
        "the marker spellings live in src/provenance.rs alone"
    );
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
    let rendered = board::render(&observe(&root, &["one"], None), true, None);
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
        stdout.contains("# 2026-09-16 UTC\n## 09:00:00 torn:lead\n  kept words\n"),
        "stdout: {stdout}"
    );
    assert!(
        !stdout.contains("torn words"),
        "the torn tail is never trusted: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn binary_lines_clips_a_multi_line_text_body() {
    let root = rig("bin-lines");
    let store = root.join("claude");
    let record = "{\"type\":\"user\",\"timestamp\":\"2026-09-16T09:00:00Z\",\"message\":{\"role\":\"user\",\"content\":\"first\\nsecond\\nthird\\nfourth\"}}";
    plant_transcript(&store, "work", CLAUDE_ID, &[record.to_owned()]);
    plant_session(
        &root,
        "clip",
        &claude_roster("main", "lead", CLAUDE_ID, &store),
    );
    let (code, stdout, stderr) = run(&root, &["board", "clip", "--lines", "2"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains("  first\n  second\n  … +2 lines\n"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("third"),
        "dropped lines never print: {stdout}"
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

/// One synthetic Codex assistant message; `content` is the parts array.
fn codex_assistant(ts: &str, content: &str) -> String {
    format!(
        r#"{{"timestamp":"{ts}","type":"response_item","payload":{{"type":"message","role":"assistant","content":{content}}}}}"#
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
    let text = board::render(&observation, false, None);
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
    let text = board::render(&observation, false, None);
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
    let text = board::render(&observation, false, None);
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

const AGY_ID: &str = "0199c0de-ffff-4890-abcd-ef0123456789";
const AGY_OTHER: &str = "0199c0de-0000-4890-abcd-ef0123456790";

/// One synthetic agy history record: `{id}` absent when no conversation id.
fn agy_record(id: Option<&str>, body: &str, millis: i64) -> String {
    let id = id.map_or(String::new(), |id| format!(r#""conversationId":"{id}","#));
    format!(r#"{{"display":"{body}","timestamp":{millis},{id}"workspace":"/work"}}"#)
}

fn agy_roster(slot: &str, seat: &str, id: &str) -> String {
    format!("seat.{slot}={seat}\nharness_session.{slot}={id}\nagent_bin.{slot}=agy\n")
}

#[test]
fn an_agy_seat_reads_only_its_own_conversation() {
    // ONE history file per home carries every agy conversation, so the seat's
    // captured id alone separates its turns from a sibling's.
    let root = rig("agy-rows");
    let dir = root.join(".gemini/antigravity-cli");
    std::fs::create_dir_all(&dir).expect("agy dir");
    let lines = [
        agy_record(Some(AGY_ID), "agy human words", 1_789_549_200_500),
        agy_record(Some(AGY_OTHER), "another seat's words", 1_789_549_200_500),
    ];
    std::fs::write(dir.join("history.jsonl"), lines.join("\n") + "\n").expect("history");
    plant_session(&root, "ship", &agy_roster("main", "lead", AGY_ID));
    let observation = observe(&root, &["ship"], None);
    assert!(observation.coverage.is_empty(), "no phase row after a read");
    assert_eq!(observation.rows.len(), 1, "the foreign record stays silent");
    assert_eq!(observation.rows[0].body, "agy human words");
    assert_eq!(observation.rows[0].ts, 1_789_549_200_500_000);
    assert_eq!(observation.rows[0].actor, "ship:lead");
    let text = board::render(&observation, false, None);
    assert!(!text.contains("phase 5"), "the phase row is gone: {text}");
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
        "{\"kind\":\"scope\",\"scope\":\"current-and-recorded-predecessors\",\"phase\":\"8b\"}\n"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn follow_parses_anywhere_in_the_tail_and_is_advertised() {
    let args = board::parse(&["--follow".to_owned()]).expect("--follow parses");
    assert!(args.follow);
    assert!(!args.json);
    let args = board::parse(&[
        "day".to_owned(),
        "--since".to_owned(),
        "2026-09-16T09:00:00Z".to_owned(),
        "--follow".to_owned(),
        "--json".to_owned(),
    ])
    .expect("a mixed tail parses");
    assert!(args.follow && args.json, "follow composes with --json");
    assert_eq!(args.sessions, ["day"]);
    assert!(board::USAGE.contains("--follow"), "usage names it");
    assert!(board::USAGE.contains("--lines <n>"), "usage names the clip");
    assert!(
        ae::entry::HELP.contains(
            "ae board [session…] [--since <ts>] [--json] [--follow] [--lines <n>] [--assistant]"
        ),
        "help carries the pinned synopsis"
    );
    assert!(
        ae::entry::HELP.contains("--follow keeps printing"),
        "help says what it does"
    );
    assert!(
        ae::entry::HELP
            .contains("--assistant adds the model's replies (text only; off by default)"),
        "help names the flag"
    );
}

#[test]
fn assistant_seats_render_rows_and_roles_only_behind_the_flag() {
    let root = rig("assistant-flag");
    let claude_store = root.join("claude");
    plant_transcript(
        &claude_store,
        "work",
        CLAUDE_ID,
        &[
            user("2026-09-16T09:00:00Z", "human words"),
            claude_assistant(
                "2026-09-16T09:00:01Z",
                r#"[{"type":"thinking","thinking":"hidden"},{"type":"text","text":"synthetic reply"}]"#,
            ),
        ],
    );
    let codex_store = root.join("codex");
    plant_rollout(
        &codex_store,
        &[
            codex_user("2026-09-16T09:00:02Z", "codex human words"),
            codex_assistant(
                "2026-09-16T09:00:03Z",
                r#"[{"type":"output_text","text":"codex synthetic reply"}]"#,
            ),
        ],
    );
    plant_session(
        &root,
        "one",
        &format!(
            "{}{}",
            claude_roster("main", "lead", CLAUDE_ID, &claude_store),
            codex_roster("worker.0", "colead", CODEX_ID, &codex_store),
        ),
    );
    let off = observe(&root, &["one"], None);
    let bodies: Vec<&str> = off.rows.iter().map(|row| row.body.as_str()).collect();
    assert_eq!(bodies, ["human words", "codex human words"], "human only");
    assert!(
        !board::render(&off, false, None).contains("assistant"),
        "off"
    );
    assert!(
        !board::render(&off, true, None).contains("\"role\":\"assistant\""),
        "off"
    );
    let on = observe_with(&root, &["one"], None, true);
    let roles: Vec<board::Role> = on.rows.iter().map(|row| row.role).collect();
    assert_eq!(
        roles,
        [
            board::Role::Human,
            board::Role::Assistant,
            board::Role::Human,
            board::Role::Assistant,
        ]
    );
    let text = board::render(&on, false, None);
    for header in [
        "## 09:00:01 one:lead · assistant\n  synthetic reply\n",
        "## 09:00:03 one:colead · assistant\n  codex synthetic reply\n",
    ] {
        assert!(text.contains(header), "{text}");
    }
    let json = board::render(&on, true, None);
    assert_eq!(json.matches("\"role\":\"assistant\"").count(), 2, "{json}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_flag_off_board_is_byte_identical_to_the_human_only_store() {
    // Two roots, ONE session name: the assistant records exist only in the
    // first, and with the flag off the two boards must print the same bytes.
    let replied = rig("identical-replied");
    let plain = rig("identical-plain");
    for (root, with_replies) in [(&replied, true), (&plain, false)] {
        let store = root.join("claude");
        let mut lines = vec![user("2026-09-16T09:00:00Z", "human words")];
        if with_replies {
            lines.push(claude_assistant(
                "2026-09-16T09:00:01Z",
                r#"[{"type":"text","text":"synthetic reply"}]"#,
            ));
        }
        plant_transcript(&store, "work", CLAUDE_ID, &lines);
        plant_session(
            root,
            "same",
            &claude_roster("main", "lead", CLAUDE_ID, &store),
        );
    }
    let with_reply = board::render(&observe(&replied, &["same"], None), false, None);
    let human_only = board::render(&observe(&plain, &["same"], None), false, None);
    assert_eq!(with_reply, human_only, "the flag-off stream is unchanged");
    let _ = std::fs::remove_dir_all(&replied);
    let _ = std::fs::remove_dir_all(&plain);
}

#[test]
fn the_assistant_flag_parses_with_every_other_flag_and_is_advertised() {
    let words = |items: &[&str]| {
        items
            .iter()
            .map(|word| (*word).to_owned())
            .collect::<Vec<_>>()
    };
    let args = board::parse(&words(&[
        "day",
        "--since",
        "2026-09-16T09:00:00Z",
        "--follow",
        "--json",
        "--assistant",
    ]))
    .expect("the flag composes");
    assert!(args.assistant && args.follow && args.json);
    let clipped =
        board::parse(&words(&["--lines", "3", "--assistant"])).expect("composes with --lines");
    assert_eq!(clipped.lines, Some(3));
    assert!(clipped.assistant);
    assert!(board::USAGE.contains("--assistant"), "usage names it");
}

#[test]
fn binary_assistant_flag_reaches_the_reader() {
    let root = rig("bin-assistant");
    let store = root.join("claude");
    plant_transcript(
        &store,
        "work",
        CLAUDE_ID,
        &[
            user("2026-09-16T09:00:00Z", "human words"),
            claude_assistant(
                "2026-09-16T09:00:01Z",
                r#"[{"type":"text","text":"synthetic reply"}]"#,
            ),
        ],
    );
    plant_session(
        &root,
        "one",
        &claude_roster("main", "lead", CLAUDE_ID, &store),
    );
    let (code, stdout, stderr) = run(&root, &["board", "one"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(!stdout.contains("assistant"), "off: {stdout}");
    let (code, stdout, stderr) = run(&root, &["board", "one", "--assistant"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stdout.contains("one:lead · assistant\n  synthetic reply\n"),
        "{stdout}"
    );
    let (code, stdout, stderr) = run(&root, &["board", "one", "--assistant", "--json"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("\"role\":\"assistant\""), "{stdout}");
    let _ = std::fs::remove_dir_all(&root);
}

/// One synthetic grok agent text chunk; `{b}` is the delta.
const GROK_AGENT: &str = r#"{"timestamp":1789549200,"method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"{b}"}}}}"#;

/// One content-free grok record; `{k}` is the kind (a boundary or inert).
const GROK_BARE: &str = r#"{"timestamp":1789549200,"method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"{k}"}}}"#;

/// One synthetic Muse committed reply; `{b}` is the whole text.
const MUSE_COMM: &str = r#"{"recorded_at":1789565338436454,"payload_type":"runtime.session","payload":{"event":{"kind":"assistant_message_committed","message_id":"m","response_id":"r","provider_item_id":"p","text":"{b}"}}}"#;

fn grok_agent(body: &str) -> String {
    GROK_AGENT.replace("{b}", &body.replace('"', "\\\"").replace('\n', "\\n"))
}

fn grok_bare(kind: &str) -> String {
    GROK_BARE.replace("{k}", kind)
}

fn muse_comm(body: &str) -> String {
    MUSE_COMM.replace("{b}", body)
}

/// Plant the 7b session: grok (two turns, the first split by a tool record),
/// muse and agy seats beside their human rows; replies only when asked.
fn plant_7b(root: &Path, with_replies: bool) {
    let mut grok = vec![grok_user("grok human words")];
    if with_replies {
        grok.extend([
            grok_agent("syn"),
            grok_bare("tool_call"),
            grok_agent("thetic reply"),
            grok_bare("turn_completed"),
            grok_agent("second turn"),
            grok_bare("turn_completed"),
        ]);
    }
    plant_grok(root, "work", GROK_ID, &grok);
    let mut muse = vec![muse_user("muse human words")];
    if with_replies {
        muse.push(muse_comm("muse synthetic reply"));
    }
    plant_muse(root, "2026/09/16", MUSE_ID, &muse);
    let dir = root.join(".gemini/antigravity-cli");
    std::fs::create_dir_all(&dir).expect("agy dir");
    std::fs::write(
        dir.join("history.jsonl"),
        agy_record(Some(AGY_ID), "agy human words", 1_789_549_201_500) + "\n",
    )
    .expect("history");
    plant_session(
        root,
        "one",
        &format!(
            "{}{}{}",
            grok_roster("main", "lead", GROK_ID),
            muse_roster("worker.0", "colead", MUSE_ID),
            agy_roster("worker.1", "scout", AGY_ID),
        ),
    );
}

#[test]
fn grok_muse_and_agy_replies_render_only_behind_the_flag() {
    let root = rig("assistant-7b");
    plant_7b(&root, true);
    let off = observe(&root, &["one"], None);
    let bodies: Vec<&str> = off.rows.iter().map(|row| row.body.as_str()).collect();
    assert_eq!(
        bodies,
        ["grok human words", "agy human words", "muse human words"]
    );
    assert!(
        !board::render(&off, false, None).contains("assistant"),
        "off"
    );
    let on = observe_with(&root, &["one"], None, true);
    assert_eq!(
        on.rows
            .iter()
            .filter(|row| row.role == board::Role::Assistant)
            .count(),
        3,
        "two grok turns plus the muse reply"
    );
    let text = board::render(&on, false, None);
    for header in [
        "one:lead · assistant\n  synthetic reply\n",
        "one:lead · assistant\n  second turn\n",
        "one:colead · assistant\n  muse synthetic reply\n",
        "coverage incomplete: one:scout — agy: no assistant records (history carries prompts only)",
    ] {
        assert!(text.contains(header), "{text}");
    }
    let json = board::render(&on, true, None);
    assert_eq!(json.matches("\"role\":\"assistant\"").count(), 3, "{json}");
    // Flag off, the replied store prints exactly the human-only rendering.
    let plain = rig("assistant-7b-plain");
    plant_7b(&plain, false);
    assert_eq!(
        board::render(&observe(&root, &["one"], None), false, None),
        board::render(&observe(&plain, &["one"], None), false, None)
    );
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&plain);
}

const PRIOR_OLD_ID: &str = "0199c0de-6666-4890-abcd-ef0123456789";
const PRIOR_NEW_ID: &str = "0199c0de-7777-4890-abcd-ef0123456789";

#[test]
fn predecessors_read_newest_first_with_coverage_for_the_missing() {
    let root = rig("priors");
    let store = root.join("claude");
    plant_transcript(
        &store,
        "work",
        CLAUDE_ID,
        &[user("2026-09-16T09:00:00Z", "current words")],
    );
    plant_transcript(
        &store,
        "work",
        PRIOR_NEW_ID,
        &[user("2026-09-15T09:00:00Z", "prior words")],
    );
    plant_session(
        &root,
        "one",
        &format!(
            "{}harness_session_prior.main={PRIOR_OLD_ID},{PRIOR_NEW_ID}\n",
            claude_roster("main", "lead", CLAUDE_ID, &store)
        ),
    );
    let observation = observe(&root, &["one"], None);
    let generations: Vec<u8> = observation.rows.iter().map(|row| row.generation).collect();
    assert_eq!(generations, [1, 0], "oldest first across generations");
    assert_eq!(observation.coverage.len(), 1);
    assert_eq!(
        observation.coverage[0].reason,
        "predecessor 2: transcript not found"
    );
    let text = board::render(&observation, false, None);
    assert_eq!(text.lines().next(), Some(SCOPE));
    assert!(
        text.contains("## 09:00:00 one:lead · prior 1\n  prior words\n"),
        "{text}"
    );
    assert!(
        text.contains("predecessor 2: transcript not found"),
        "{text}"
    );
    let json = board::render(&observation, true, None);
    let lines: Vec<&str> = json.lines().collect();
    assert_eq!(lines.len(), 4, "scope, coverage, two rows: {json}");
    for (line, expected) in lines[2..].iter().zip([1, 0]) {
        let value = ae::json::parse(line).expect("row parses");
        assert_eq!(
            value.get("generation"),
            Some(&ae::json::Value::Num(expected))
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_predecessor_is_read_in_the_store_of_the_tool_its_tag_names() {
    // THE RESEAT CASE. This seat runs muse now and ran claude before, so its
    // predecessor lives in a store the slot no longer names. The tag is what
    // makes that conversation readable: the locator and the reader are chosen
    // by the ELEMENT's tool, not the slot's.
    let root = rig("prior-tags");
    plant_muse(&root, "2026/09/16", MUSE_ID, &[muse_user("current words")]);
    plant_transcript(
        &root.join(".claude"),
        "work",
        PRIOR_NEW_ID,
        &[user("2026-09-15T09:00:00Z", "predecessor words")],
    );
    plant_session(
        &root,
        "tagged",
        &format!(
            "{}harness_session_prior.main=claude:{PRIOR_NEW_ID}\n",
            muse_roster("main", "lead", MUSE_ID)
        ),
    );
    let observation = observe(&root, &["tagged"], None);
    assert_eq!(
        observation.coverage,
        [],
        "a tagged predecessor was not located: {:?}",
        observation.coverage
    );
    let read: Vec<(u8, &str)> = observation
        .rows
        .iter()
        .map(|row| (row.generation, row.body.as_str()))
        .collect();
    assert_eq!(read, [(1, "predecessor words"), (0, "current words")]);

    // THE CONTRAST, and the reason the tag exists: the same id left UNTAGGED is
    // looked for in the store the slot names now, where it is not, and the read
    // says so instead of inventing it.
    plant_session(
        &root,
        "legacy",
        &format!(
            "{}harness_session_prior.main={PRIOR_NEW_ID}\n",
            muse_roster("main", "lead", MUSE_ID)
        ),
    );
    let legacy = observe(&root, &["legacy"], None);
    assert_eq!(
        legacy.coverage.len(),
        1,
        "the untagged predecessor read: {legacy:?}"
    );
    assert_eq!(
        legacy.coverage[0].reason,
        "predecessor 1: transcript not found"
    );
    assert_eq!(legacy.rows.len(), 1, "only the current conversation reads");

    // AN UNKNOWN TAG DEGRADES, IT NEVER REFUSES. The grammar admits any
    // basename, so a wrapper binary keeps working — and the cost of that is a
    // tag naming a tool ae has no reader for. That one becomes a coverage row
    // exactly as an unsupported CURRENT tool does, and the seat's own
    // conversation still reads.
    plant_session(
        &root,
        "unknown",
        &format!(
            "{}harness_session_prior.main=nope:{PRIOR_NEW_ID}\n",
            muse_roster("main", "lead", MUSE_ID)
        ),
    );
    let unknown = observe(&root, &["unknown"], None);
    assert_eq!(
        unknown.coverage.len(),
        1,
        "an unknown tag is one coverage row: {unknown:?}"
    );
    assert_eq!(
        unknown.coverage[0].reason,
        "predecessor 1: unknown tool: out of scope"
    );
    assert_eq!(
        unknown.rows.len(),
        1,
        "the seat's own conversation still reads"
    );

    // THE TAG PICKS THE READER, NOT JUST THE STORE. OpenCode is read through
    // its export leg rather than a file, so an opencode-tagged element must
    // reach THAT reader — proved here by the claude transcript planted above
    // sitting untouched at this very id: a dispatch on the slot's tool would
    // read it, and this one refuses in the export leg's own words instead. It
    // never shells out, because that id is not an opencode name.
    plant_session(
        &root,
        "exported",
        &format!(
            "{}harness_session_prior.main=opencode:{PRIOR_NEW_ID}\n",
            muse_roster("main", "lead", MUSE_ID)
        ),
    );
    let exported = observe(&root, &["exported"], None);
    assert_eq!(
        exported.coverage.len(),
        1,
        "the export leg covers its own refusal: {exported:?}"
    );
    assert_eq!(
        exported.coverage[0].reason,
        "predecessor 1: invalid or missing conversation id"
    );
    assert_eq!(
        exported.rows.len(),
        1,
        "the planted claude transcript was read through the wrong tool: {exported:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_first_follow_pass_is_the_plain_board() {
    // The loop itself sleeps on a clock and never returns, so it is not driven
    // here: the unit tests own the arms and the driver shares this exact read
    // path. What this pins is that `--follow` changes nothing about the pass it
    // shares with the plain board — same args, same rows, same bytes.
    let root = rig("follow-first");
    let store = root.join("claude");
    plant_transcript(
        &store,
        "work",
        CLAUDE_ID,
        &[user("2026-09-16T09:00:00.500Z", "plain human words")],
    );
    plant_transcript(
        &store,
        "work",
        PRIOR_NEW_ID,
        &[user("2026-09-15T09:00:00Z", "prior words")],
    );
    plant_session(
        &root,
        "one",
        &format!(
            "{}harness_session_prior.main={PRIOR_NEW_ID}\n",
            claude_roster("main", "lead", CLAUDE_ID, &store)
        ),
    );
    let plain = board::parse(&[]).expect("plain parses");
    let follow = board::parse(&["--follow".to_owned()]).expect("follow parses");
    assert!(follow.follow && !plain.follow);
    let plain = board::render(
        &observe(&root, &["one"], plain.since_micros),
        plain.json,
        None,
    );
    let followed = board::render(
        &observe(&root, &["one"], follow.since_micros),
        follow.json,
        None,
    );
    assert_eq!(plain, followed);
    assert!(followed.contains("# 2026-09-16 UTC\n## 09:00:00 one:lead"));
    assert!(followed.contains("· prior 1"), "predecessors ride along");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn dividers_mark_utc_days_and_follow_polls_carry_the_day() {
    let two = rig("dividers-two");
    let store = two.join("claude");
    plant_transcript(
        &store,
        "work",
        CLAUDE_ID,
        &[
            user("2026-09-16T23:00:00Z", "late words"),
            user("2026-09-17T00:00:00Z", "next day words"),
        ],
    );
    plant_session(
        &two,
        "one",
        &claude_roster("main", "lead", CLAUDE_ID, &store),
    );
    let text = board::render(&observe(&two, &["one"], None), false, None);
    assert_eq!(text.matches("# 2026-09-16 UTC\n").count(), 1, "{text}");
    assert_eq!(text.matches("# 2026-09-17 UTC\n").count(), 1, "{text}");
    let _ = std::fs::remove_dir_all(&two);

    let root = rig("dividers");
    let store = root.join("claude");
    let first = user("2026-09-16T09:00:00Z", "same day one");
    let second = user("2026-09-16T10:00:00Z", "same day two");
    let path = plant_transcript(&store, "work", CLAUDE_ID, &[first, second]);
    plant_session(
        &root,
        "one",
        &claude_roster("main", "lead", CLAUDE_ID, &store),
    );
    let text = board::render(&observe(&root, &["one"], None), false, None);
    assert_eq!(text.matches("# 2026-09-16 UTC\n").count(), 1, "{text}");
    assert!(text.contains("## 09:00:00 one:lead"), "{text}");

    // The follow, driven exactly as the binary drives it: the first pass IS
    // the one-shot board, then polls append only what is new.
    let sessions = vec![SessionInput {
        name: "one".to_owned(),
        path: root.join("sessions").join("one"),
    }];
    let inputs = Inputs {
        home: Some(root.as_path()),
        sessions: &sessions,
        assistant: false,
    };
    let first_pass = board::observe(&inputs, None);
    let mut follow = board::follow::Follow::seeded(&first_pass.seeds, &first_pass.coverage, None);
    let mut body = std::fs::read_to_string(&path).expect("transcript");
    let _ = writeln!(body, "{}", user("2026-09-16T11:00:00Z", "same day three"));
    std::fs::write(&path, &body).expect("append");
    let batch = board::follow_poll(&inputs, &mut follow);
    assert_eq!(batch.rows.len(), 1);
    let text = board::render_batch(&batch, false, None);
    assert!(!text.contains("# 2026-"), "same day, no divider: {text}");
    assert!(text.contains("## 11:00:00 one:lead"), "{text}");
    let _ = writeln!(body, "{}", user("2026-09-17T09:00:00Z", "new day"));
    std::fs::write(&path, &body).expect("append");
    let batch = board::follow_poll(&inputs, &mut follow);
    let text = board::render_batch(&batch, false, None);
    assert!(
        text.contains("# 2026-09-17 UTC\n## 09:00:00 one:lead"),
        "{text}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// OpenCode: the export leg over the shipped binary
// ---------------------------------------------------------------------------

const OC_SID: &str = "ses_00000000000000000000000000";

/// One synthetic `opencode export` document; `messages` are hand-written
/// message records, never copied transcript text.
fn export_doc(id: &str, messages: &[String]) -> String {
    format!(
        r#"{{"info":{{"id":"{id}","slug":"probe","directory":"/probe"}},"messages":[{}]}}"#,
        messages.join(",")
    )
}

fn oc_message(id: &str, role: &str, created: &str, parts: &[String]) -> String {
    format!(
        r#"{{"info":{{"id":"{id}","sessionID":"{OC_SID}","role":"{role}","time":{{"created":{created}}}}},"parts":[{}]}}"#,
        parts.join(",")
    )
}

fn oc_text(text: &str) -> String {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    format!(r#"{{"type":"text","text":"{escaped}"}}"#)
}

fn oc_reasoning(text: &str) -> String {
    format!(r#"{{"type":"reasoning","text":"{text}","time":{{"start":1,"end":2}}}}"#)
}

fn oc_roster(slot: &str, seat: &str, id: &str) -> String {
    format!("seat.{slot}={seat}\nharness_session.{slot}={id}\nagent_bin.{slot}=opencode\n")
}

/// Plant `<root>/bin/opencode`, executable, with this exact script body.
fn fake_opencode(root: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).expect("fake bin dir");
    let path = bin.join("opencode");
    std::fs::write(&path, script).expect("fake opencode");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("exec bit");
}

/// An `opencode` that answers every import with one export document.
fn exporting(export: &str) -> String {
    format!("#!/bin/sh\ncat <<'JSON'\n{export}\nJSON\n")
}

/// Run the shipped binary with a deterministic `PATH`: the rig's own bin first
/// (where the fake `opencode` may live), then the OS binaries so a fake script
/// can still call `cat` or `sleep` — and nothing else, so neither a fake nor a
/// real `opencode` can hide on the runner's inherited PATH.
fn run_with_path(root: &Path, bin: &Path, tail: &[&str]) -> (Option<i32>, String, String) {
    let out = ae()
        .env("HOME", root)
        .env("AE_HOME", root)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
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

/// The fake bin dir of a rig whose `opencode` is planted.
fn fake_bin(root: &Path) -> PathBuf {
    root.join("bin")
}

/// A PATH directory with no `opencode` anywhere.
fn empty_bin(root: &Path) -> PathBuf {
    let bin = root.join("no-opencode");
    std::fs::create_dir_all(&bin).expect("empty bin dir");
    bin
}

#[test]
fn an_opencode_seat_reads_its_export_through_the_shipped_binary() {
    let root = rig("oc-read");
    let export = export_doc(
        OC_SID,
        &[oc_message(
            "msg_1",
            "user",
            "1789549200500",
            &[oc_text("plain human words")],
        )],
    );
    fake_opencode(&root, &exporting(&export));
    plant_session(&root, "oc", &oc_roster("main", "lead", OC_SID));
    let (code, stdout, stderr) = run_with_path(&root, &fake_bin(&root), &["board", "oc"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        stdout.contains("# 2026-09-16 UTC\n## 09:00:00 oc:lead\n  plain human words\n"),
        "{stdout}"
    );
    assert!(!stdout.contains("coverage incomplete"), "{stdout}");

    let (_, json, _) = run_with_path(&root, &fake_bin(&root), &["board", "oc", "--json"]);
    let row = json
        .lines()
        .find(|line| line.contains("\"kind\":\"row\""))
        .expect("one row line");
    let value = ae::json::parse(row).expect("the row line parses");
    assert_eq!(value.get_str("source"), Some("opencode"));
    assert_eq!(value.get_str("role"), Some("human"));
    assert_eq!(value.get_str("body"), Some("plain human words"));
    let identity = format!("opencode:{OC_SID}#msg_1");
    assert_eq!(value.get_str("file"), Some(identity.as_str()));

    // `--since` after the turn filters it out; before it, the row stands.
    let (_, later, _) = run_with_path(
        &root,
        &fake_bin(&root),
        &["board", "oc", "--since", "2026-09-16T09:00:01Z"],
    );
    assert!(!later.contains("plain human words"), "{later}");
    let (_, earlier, _) = run_with_path(
        &root,
        &fake_bin(&root),
        &["board", "oc", "--since", "2026-09-16T08:59:59Z"],
    );
    assert!(earlier.contains("plain human words"), "{earlier}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_opencode_reply_reads_only_with_the_flag_and_never_reasoning() {
    let root = rig("oc-assistant");
    let export = export_doc(
        OC_SID,
        &[
            oc_message("msg_1", "user", "1789549200500", &[oc_text("human asks")]),
            oc_message(
                "msg_2",
                "assistant",
                "1789549201500",
                &[oc_reasoning("hidden chain"), oc_text("visible reply")],
            ),
        ],
    );
    fake_opencode(&root, &exporting(&export));
    plant_session(&root, "oc", &oc_roster("main", "lead", OC_SID));
    let (_, off, _) = run_with_path(&root, &fake_bin(&root), &["board", "oc"]);
    assert!(!off.contains("assistant"), "{off}");
    assert!(
        !off.contains("visible reply") && !off.contains("hidden chain"),
        "{off}"
    );
    let (_, on, _) = run_with_path(&root, &fake_bin(&root), &["board", "oc", "--assistant"]);
    assert!(
        on.contains("## 09:00:01 oc:lead · assistant\n  visible reply\n"),
        "{on}"
    );
    assert!(
        !on.contains("hidden chain"),
        "reasoning text never reaches the board: {on}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_ae_marked_opencode_turn_is_hidden_and_counted() {
    let root = rig("oc-hidden");
    let export = export_doc(
        OC_SID,
        &[
            oc_message(
                "msg_1",
                "user",
                "1789549200500",
                &[oc_text("⟦ae:msg from lead⟧\ninjected words")],
            ),
            oc_message(
                "msg_2",
                "user",
                "1789549201500",
                &[oc_text("the human's own words")],
            ),
        ],
    );
    fake_opencode(&root, &exporting(&export));
    plant_session(&root, "oc", &oc_roster("main", "lead", OC_SID));
    let (_, stdout, _) = run_with_path(&root, &fake_bin(&root), &["board", "oc"]);
    assert!(
        stdout.contains("hidden: oc:lead — 1 ae-injected turns"),
        "{stdout}"
    );
    assert!(!stdout.contains("injected words"), "{stdout}");
    assert!(stdout.contains("the human's own words"), "{stdout}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_failed_opencode_export_covers_that_seat_and_the_others_still_print() {
    let root = rig("oc-failed");
    let store = root.join("claude");
    plant_transcript(
        &store,
        "work",
        CLAUDE_ID,
        &[user("2026-09-16T09:00:00.500Z", "claude words")],
    );
    plant_session(
        &root,
        "fleet",
        &format!(
            "{}{}",
            oc_roster("main", "oc", OC_SID),
            claude_roster("spawned.0", "cc", CLAUDE_ID, &store)
        ),
    );

    // No `opencode` on PATH at all: the spawn itself fails.
    let (code, stdout, _) = run_with_path(&root, &empty_bin(&root), &["board", "fleet"]);
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("coverage incomplete: fleet:oc — export failed"),
        "{stdout}"
    );
    assert!(
        stdout.contains("## 09:00:00 fleet:cc\n  claude words"),
        "{stdout}"
    );

    // And a binary that answers nonzero is the same covered seat.
    fake_opencode(&root, "#!/bin/sh\nexit 3\n");
    let (_, stdout, _) = run_with_path(&root, &fake_bin(&root), &["board", "fleet"]);
    assert!(
        stdout.contains("coverage incomplete: fleet:oc — export failed"),
        "{stdout}"
    );
    assert!(
        stdout.contains("## 09:00:00 fleet:cc\n  claude words"),
        "{stdout}"
    );

    // A success exit with no document at all is a document fault, not a
    // process fault: the seat reads as unreadable, still without touching the
    // other seat.
    fake_opencode(&root, "#!/bin/sh\nexit 0\n");
    let (_, stdout, _) = run_with_path(&root, &fake_bin(&root), &["board", "fleet"]);
    assert!(
        stdout.contains("coverage incomplete: fleet:oc — export unreadable"),
        "{stdout}"
    );
    assert!(
        stdout.contains("## 09:00:00 fleet:cc\n  claude words"),
        "{stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_refused_session_id_never_reaches_the_opencode_argv() {
    let root = rig("oc-grammar");
    let marker = root.join("spawned-argv");
    fake_opencode(
        &root,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\nexit 1\n",
            marker.display()
        ),
    );
    // Two refusals: an id that is not the measured grammar, and no id at all.
    plant_session(
        &root,
        "fleet",
        "seat.main=oc\nharness_session.main=--continue\nagent_bin.main=opencode\nseat.spawned.0=oc2\nagent_bin.spawned.0=opencode\n",
    );
    let (code, stdout, _) = run_with_path(&root, &fake_bin(&root), &["board", "fleet"]);
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout.matches("invalid or missing conversation id").count(),
        2,
        "{stdout}"
    );
    assert!(
        !marker.exists(),
        "an invalid id must not mint an argv: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_slow_opencode_export_never_clips_another_seats_rows() {
    let root = rig("oc-slow");
    let store = root.join("claude");
    plant_transcript(
        &store,
        "work",
        CLAUDE_ID,
        &[user("2026-09-16T08:00:00.000Z", "claude words")],
    );
    let export = export_doc(
        OC_SID,
        &[oc_message(
            "msg_1",
            "user",
            "1789549200500",
            &[oc_text("oc words")],
        )],
    );
    fake_opencode(
        &root,
        &format!("#!/bin/sh\nsleep 3\n{}", exporting(&export)),
    );
    plant_session(
        &root,
        "fleet",
        &format!(
            "{}{}",
            oc_roster("main", "oc", OC_SID),
            claude_roster("spawned.0", "cc", CLAUDE_ID, &store)
        ),
    );
    let (code, stdout, _) = run_with_path(&root, &fake_bin(&root), &["board", "fleet"]);
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("## 08:00:00 fleet:cc\n  claude words"),
        "a slow export delays the sequential board but never clips another seat: {stdout}"
    );
    assert!(
        stdout.contains("## 09:00:00 fleet:oc\n  oc words"),
        "and its own rows print whole once it answers: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_opencode_predecessor_reads_through_its_own_export() {
    let root = rig("oc-prior");
    let prior_id = "ses_11111111111111111111111111";
    let current = export_doc(
        OC_SID,
        &[oc_message(
            "msg_now",
            "user",
            "1789549201500",
            &[oc_text("current words")],
        )],
    );
    let prior = export_doc(
        prior_id,
        &[oc_message(
            "msg_old",
            "user",
            "1789549200500",
            &[oc_text("prior words")],
        )],
    );
    // One fake that answers each generation's own id: the export leg runs
    // once per generation, so a prior conversation needs no special path.
    fake_opencode(
        &root,
        &format!(
            "#!/bin/sh\ncase \"$2\" in\n  {OC_SID}) cat <<'JSON'\n{current}\nJSON\n  ;;\n  {prior_id}) cat <<'JSON'\n{prior}\nJSON\n  ;;\nesac\n"
        ),
    );
    plant_session(
        &root,
        "oc",
        &format!(
            "seat.main=lead\nharness_session.main={OC_SID}\nharness_session_prior.main={prior_id}\nagent_bin.main=opencode\n"
        ),
    );
    let (code, stdout, stderr) = run_with_path(&root, &fake_bin(&root), &["board", "oc"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        stdout.contains("## 09:00:00 oc:lead · prior 1\n  prior words"),
        "{stdout}"
    );
    assert!(
        stdout.contains("## 09:00:01 oc:lead\n  current words"),
        "{stdout}"
    );
    let (_, json, _) = run_with_path(&root, &fake_bin(&root), &["board", "oc", "--json"]);
    assert!(json.contains("\"generation\":1"), "{json}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_opencode_export_cap_is_the_documented_sixteen_mib() {
    // The VALUE, not the constant: an input built as `EXPORT_CAP + 1` moves
    // with the constant and can never see the constant move (a 16 MiB -> 4 MiB
    // edit survives it). At exactly 16 MiB the document is inside the cap and
    // fails to PARSE; one byte more is refused as oversize. The doc phrase is
    // pinned beside the value so the two cannot drift apart silently.
    let at_cap = vec![b'x'; 16 * 1024 * 1024];
    let (_, coverage) = ae::board::opencode::read(
        &at_cap,
        OC_SID,
        "fleet:oc",
        ae::tool::ToolKind::OpenCode,
        false,
    );
    assert_eq!(
        coverage[0].reason, "export unreadable",
        "16 MiB itself is inside the cap"
    );
    let over_cap = vec![b'x'; 16 * 1024 * 1024 + 1];
    let (rows, coverage) = ae::board::opencode::read(
        &over_cap,
        OC_SID,
        "fleet:oc",
        ae::tool::ToolKind::OpenCode,
        false,
    );
    assert!(rows.is_empty());
    assert_eq!(coverage[0].reason, "export exceeds the read budget");
    let docs = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/board.md"))
        .expect("docs/board.md is readable");
    // Wrap-proof: the sentence is compared over normalized whitespace, so a
    // rewrap cannot silently disconnect the doc from the value.
    let flat = docs.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains("A whole export over 16 MiB is refused"),
        "docs/board.md names the same cap"
    );
}

#[test]
fn a_planted_control_laden_transcript_reaches_no_raw_control_byte() {
    // The JSON TEXT carries the escapes; the reader parses them into real
    // control bytes in the body, and the shipped binary must not print them.
    let root = rig("boardctl");
    let store = root.join("claude");
    let line = "{\"type\":\"user\",\"timestamp\":\"2026-09-16T09:00:00.500Z\",\"message\":{\"role\":\"user\",\"content\":\"A\\u001b[2J B\\u0007 C\\u0000 D\\u000d E\\u007f F\\u009b\"}}".to_owned();
    plant_transcript(&store, "work", CLAUDE_ID, &[line]);
    plant_session(
        &root,
        "one",
        &claude_roster("main", "lead", CLAUDE_ID, &store),
    );
    let (code, stdout, stderr) = run(&root, &["board", "one"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        !stdout
            .chars()
            .any(|ch| ch.is_control() && ch != '\n' && ch != '\t'),
        "the board's stdout carries no raw control: {stdout:?}"
    );
    assert!(stdout.contains('\u{FFFD}'), "{stdout:?}");
    assert!(stdout.contains("[2J"), "visible text kept: {stdout:?}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_export_larger_than_a_pipe_buffer_reads_whole_through_the_shipped_binary() {
    // This pin claims only what it can see: a document well past a pipe buffer
    // is read end to end (its LAST row prints) and the ROW_CAP walk stays
    // intact. It is NOT the truncation regression pin — a shell fake drains
    // its pipe normally, so it passes with or without the file capture. The
    // doors lexical guard `the_opencode_leg_captures_through_a_file_never_a_pipe`
    // is that pin.
    let root = rig("oc-large");
    let messages: Vec<String> = (0..700)
        .map(|n| {
            oc_message(
                &format!("msg_{n}"),
                "user",
                &(1_789_549_200_500_i64 + n).to_string(),
                &[oc_text(&format!("turn {n}"))],
            )
        })
        .collect();
    let export = export_doc(OC_SID, &messages);
    assert!(
        export.len() > 64 * 1024,
        "the fixture must exceed a pipe buffer: {} bytes",
        export.len()
    );
    fake_opencode(&root, &exporting(&export));
    plant_session(&root, "oc", &oc_roster("main", "lead", OC_SID));
    let (code, stdout, stderr) = run_with_path(&root, &fake_bin(&root), &["board", "oc"]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        stdout.contains("  turn 699\n"),
        "the document's end was read whole"
    );
    assert!(!stdout.contains("coverage incomplete"), "{stdout}");
    let _ = std::fs::remove_dir_all(&root);
}
