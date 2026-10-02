//! Independent acceptance: a pane reply belongs to its nearest user turn in
//! source order. The oracle is the human ruling, not transcript timestamps.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixture setup crosses the filesystem boundary the product observes"
)]

use std::fs;
use std::io::Write as _;
use std::path::PathBuf;

use ae::board::{self, Inputs, Replies, Role, follow::Follow};
use ae::usage::SessionInput;

const SESSION: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const CLAUDE: &str = "0199c0de-1234-4890-abcd-ef0123456789";
const CODEX: &str = "01a08046-1974-7352-ade3-81a786200795";
const OTHER: &str = "0199c0de-1234-4890-abcd-ef0123456790";
const MICROS: i64 = 1_789_549_200_000_000;

fn escaped(body: &str) -> String {
    let mut text = String::new();
    ae::json::escape_into(body, &mut text);
    text
}

fn stamp(minute: u8) -> String {
    format!("2026-09-16T09:{minute:02}:00Z")
}

fn user(tool: &str, minute: u8, body: &str) -> String {
    let (ts, body) = (stamp(minute), escaped(body));
    match tool {
        "claude" => format!(
            r#"{{"type":"user","timestamp":"{ts}","message":{{"role":"user","content":"{body}"}}}}"#
        ),
        "codex" => format!(
            r#"{{"timestamp":"{ts}","type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"{body}"}}]}}}}"#
        ),
        "muse" => format!(
            r#"{{"recorded_at":{},"payload_type":"runtime.user_intent.accepted","payload":{{"refill_blocks":[{{"kind":"text","text":"stub"}}],"model_messages":[{{"content":[{{"kind":"text","text":"{body}"}}]}}]}}}}"#,
            MICROS + i64::from(minute) * 60_000_000
        ),
        "grok" => grok(minute, "user_message_chunk", Some(&body)),
        _ => panic!("unsupported fixture tool"),
    }
}

fn assistant(tool: &str, minute: u8, body: &str) -> String {
    let (ts, body) = (stamp(minute), escaped(body));
    match tool {
        "claude" => format!(
            r#"{{"type":"assistant","timestamp":"{ts}","message":{{"role":"assistant","content":[{{"type":"text","text":"{body}"}}]}}}}"#
        ),
        "codex" => format!(
            r#"{{"timestamp":"{ts}","type":"response_item","payload":{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"{body}"}}]}}}}"#
        ),
        "muse" => format!(
            r#"{{"recorded_at":{},"payload_type":"runtime.session","payload":{{"event":{{"kind":"assistant_message_committed","message_id":"m{minute}","response_id":"r","provider_item_id":"p","text":"{body}"}}}}}}"#,
            MICROS + i64::from(minute) * 60_000_000
        ),
        "grok" => grok(minute, "agent_message_chunk", Some(&body)),
        _ => panic!("unsupported fixture tool"),
    }
}

fn grok(minute: u8, kind: &str, escaped_body: Option<&str>) -> String {
    let content = escaped_body.map_or_else(String::new, |body| {
        format!(r#", "content":{{"type":"text","text":"{body}"}}"#)
    });
    format!(
        r#"{{"timestamp":1789549200,"method":"session/update","params":{{"sessionId":"s","update":{{"sessionUpdate":"{kind}","_meta":{{"agentTimestampMs":{}}}{content}}}}}}}"#,
        MICROS / 1000 + i64::from(minute) * 60_000
    )
}

struct Rig {
    root: PathBuf,
    transcript: PathBuf,
    session: PathBuf,
}

impl Rig {
    fn new(tag: &str, tool: &str, records: &[String]) -> Self {
        let root = super::board::rig(&format!("reply-spec-{tag}"));
        let store = root.join(tool);
        let id = if tool == "codex" { CODEX } else { CLAUDE };
        let transcript = match tool {
            "claude" => store.join("projects/work").join(format!("{id}.jsonl")),
            "codex" => store
                .join("sessions/2026/09/08")
                .join(format!("rollout-2026-09-08T09-00-00-{id}.jsonl")),
            "muse" => root
                .join(".local/share/muse/sessions/2026/09/16")
                .join(id)
                .join("session.jsonl"),
            "grok" => root
                .join(".grok/sessions/work")
                .join(id)
                .join("updates.jsonl"),
            _ => panic!("unsupported fixture tool"),
        };
        fs::create_dir_all(transcript.parent().expect("fixture parent")).expect("fixture store");
        fs::write(&transcript, records.join("\n") + "\n").expect("fixture transcript");
        let config = if matches!(tool, "claude" | "codex") {
            format!("config_home.main={}\n", store.display())
        } else {
            String::new()
        };
        let roster = format!(
            "session_id={SESSION}\nlayout=lead-pair\nseat.main=lead\nagent_bin.main={tool}\nharness_session.main={id}\n{config}"
        );
        let session = super::board::plant_session(&root, "one", &roster);
        Self {
            root,
            transcript,
            session,
        }
    }

    fn run(&self, verb: &str, flags: &[&str]) -> String {
        let mut runner = super::cli::ae();
        runner
            .env("HOME", &self.root)
            .env("AE_HOME", &self.root)
            .env("CONFIG_FILE", self.root.join("config"))
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.root.join("bin").display()),
            )
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .args([verb, "one"])
            .args(flags);
        let child = runner.spawn().expect("fixture command");
        let output = super::cli::bounded(child, std::time::Duration::from_secs(30))
            .expect("fixture command exits within 30 seconds");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("UTF-8 fixture output")
    }

    fn append(&self, bytes: &[u8]) {
        fs::OpenOptions::new()
            .append(true)
            .open(&self.transcript)
            .expect("append fixture")
            .write_all(bytes)
            .expect("fixture tail");
    }

    fn observe(&self) -> board::Observation {
        let sessions = [SessionInput {
            name: "one".to_owned(),
            path: self.session.clone(),
        }];
        board::observe_selected(
            &Inputs {
                home: Some(&self.root),
                sessions: &sessions,
                replies: Replies::ToHuman,
            },
            None,
            None,
            &|_| true,
        )
    }

    fn poll(&self, follow: &mut Follow) -> board::Observation {
        let sessions = [SessionInput {
            name: "one".to_owned(),
            path: self.session.clone(),
        }];
        board::follow_poll_selected(
            &Inputs {
                home: Some(&self.root),
                sessions: &sessions,
                replies: Replies::ToHuman,
            },
            follow,
            &|_| true,
        )
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn bodies(seen: &board::Observation) -> Vec<&str> {
    seen.rows.iter().map(|row| row.body.as_str()).collect()
}

fn boundary_case(tag: &str, boundary: &str) {
    let rig = Rig::new(
        tag,
        "claude",
        &[
            user("claude", 0, "pane question"),
            assistant("claude", 1, "eligible before boundary"),
            boundary.to_owned(),
            assistant("claude", 3, "reply to non-human turn"),
            user("claude", 4, "new pane question"),
            assistant("claude", 5, "eligible after boundary"),
        ],
    );
    let all = rig.run("console", &["--all"]);
    assert!(
        all.contains("eligible before boundary")
            && all.contains("reply to non-human turn")
            && all.contains("eligible after boundary"),
        "fixture readiness: {all}"
    );
    let seen = rig.run("console", &[]);
    assert!(
        seen.contains("eligible before boundary") && seen.contains("eligible after boundary"),
        "pane replies must show: {seen}"
    );
    assert!(
        !seen.contains("reply to non-human turn"),
        "non-human user turn closes window: {seen}"
    );
}

#[test]
fn claude_ae_marker_closes_the_window() {
    boundary_case(
        "marker",
        &user("claude", 2, "⟦ae:msg from worker⟧\npeer question"),
    );
}

#[test]
fn console_local_ask_closes_the_pane_reply_window() {
    boundary_case(
        "console-local",
        &user("claude", 2, "⟦ae:msg from console:local⟧\nchat ask"),
    );
}

#[test]
fn claude_wrapped_ae_paste_closes_the_window() {
    boundary_case(
        "wrapped",
        &user(
            "claude",
            2,
            "<pasted_content id=\"x\">\n⟦ae:ctx⟧\ncontext\n</pasted_content id=\"x\">",
        ),
    );
}

#[test]
fn claude_task_notification_closes_the_window() {
    let record = user(
        "claude",
        2,
        "<task-notification>task done</task-notification>",
    )
    .replacen(
        "\"type\":\"user\"",
        "\"type\":\"user\",\"promptSource\":\"system\"",
        1,
    );
    boundary_case("task", &record);
}

#[test]
fn claude_compact_summary_closes_the_window() {
    let record = user("claude", 2, "old conversation summary").replacen(
        "\"type\":\"user\"",
        "\"type\":\"user\",\"isCompactSummary\":true",
        1,
    );
    boundary_case("compact", &record);
}

#[test]
fn claude_local_command_caveat_closes_the_window() {
    let record = user(
        "claude",
        2,
        "<local-command-caveat>command output</local-command-caveat>",
    )
    .replacen("\"type\":\"user\"", "\"type\":\"user\",\"isMeta\":true", 1);
    boundary_case("caveat", &record);
}

#[test]
fn claude_unstamped_user_closes_the_window() {
    boundary_case(
        "unstamped",
        r#"{"type":"user","message":{"role":"user","content":"unknown time"}}"#,
    );
}

#[test]
fn claude_unclassifiable_user_array_closes_the_window() {
    boundary_case(
        "array",
        r#"{"type":"user","timestamp":"2026-09-16T09:02:00Z","message":{"role":"user","content":[{"type":"text","text":"unclassified user"}]}}"#,
    );
}

#[test]
fn claude_image_bearing_user_array_closes_the_window() {
    boundary_case(
        "image-array",
        r#"{"type":"user","timestamp":"2026-09-16T09:02:00Z","message":{"role":"user","content":[{"type":"text","text":"image question"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAAA"}}]}}"#,
    );
}

#[test]
fn malformed_record_closes_the_window() {
    boundary_case("malformed", "{unclassifiable-record");
}

#[test]
fn non_utf8_record_closes_the_window() {
    let rig = Rig::new(
        "bytes",
        "claude",
        &[
            user("claude", 0, "pane question"),
            assistant("claude", 1, "eligible before damage"),
        ],
    );
    rig.append(&[0xff, b'\n']);
    rig.append((assistant("claude", 3, "reply after damaged turn") + "\n").as_bytes());
    let all = rig.run("console", &["--all"]);
    assert!(
        all.contains("eligible before damage") && all.contains("reply after damaged turn"),
        "fixture readiness: {all}"
    );
    let seen = rig.run("console", &[]);
    assert!(
        seen.contains("eligible before damage"),
        "human reply: {seen}"
    );
    assert!(
        !seen.contains("reply after damaged turn"),
        "damage closes window: {seen}"
    );
}

#[test]
fn overlong_record_closes_the_window() {
    boundary_case("overlong", &"x".repeat(1024 * 1024 + 1));
}

#[test]
fn tool_result_calls_and_thinking_are_not_user_turns() {
    let rig = Rig::new("tools", "claude", &[
        user("claude", 0, "pane question"),
        r#"{"type":"assistant","timestamp":"2026-09-16T09:01:00Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hidden thought"},{"type":"tool_use","id":"call-1","name":"fixture","input":{}},{"type":"text","text":"before tool"}]}}"#.to_owned(),
        r#"{"type":"user","timestamp":"2026-09-16T09:02:00Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"call-1","content":"hidden tool result"}]}}"#.to_owned(),
        r#"{"type":"assistant","timestamp":"2026-09-16T09:03:00Z","message":{"role":"assistant","content":[{"type":"text","text":"after tool"},{"type":"thinking","thinking":"hidden second thought"},{"type":"text","text":"second text part"}]}}"#.to_owned(),
    ]);
    let all = rig.run("console", &["--all"]);
    assert!(
        all.contains("before tool") && all.contains("after toolsecond text part"),
        "fixture readiness: {all}"
    );
    let seen = rig.run("console", &[]);
    assert_eq!(
        seen, all,
        "every assistant text part here answers the human"
    );
    assert!(
        !seen.contains("hidden thought")
            && !seen.contains("hidden tool result")
            && !seen.contains("hidden second thought")
    );
}

#[test]
fn windows_follow_source_order_even_when_timestamps_reverse_and_humans_repeat() {
    let rig = Rig::new(
        "order",
        "claude",
        &[
            user("claude", 8, "first human"),
            assistant("claude", 1, "answer first human"),
            user("claude", 7, "second human"),
            assistant("claude", 2, "answer second human"),
            user("claude", 0, "⟦ae:brief from lead⟧\npeer brief"),
            assistant("claude", 9, "answer peer brief"),
            user("claude", 6, "third human"),
            assistant("claude", 3, "answer third human"),
        ],
    );
    let all = rig.run("console", &["--all"]);
    assert!(
        all.contains("answer peer brief") && all.contains("answer third human"),
        "fixture readiness: {all}"
    );
    let seen = rig.run("console", &[]);
    for body in [
        "answer first human",
        "answer second human",
        "answer third human",
    ] {
        assert!(seen.contains(body), "source-order human window: {seen}");
    }
    assert!(
        !seen.contains("answer peer brief"),
        "timestamp sort must not reopen window: {seen}"
    );
}

fn tool_window(tool: &str) {
    let mut records = vec![
        user(tool, 0, "pane question"),
        assistant(tool, 1, "human answer"),
        user(tool, 2, "⟦ae:ctx⟧\nlaunch context"),
        assistant(tool, 3, "context answer"),
        user(tool, 4, "next pane question"),
        assistant(tool, 5, "next human answer"),
    ];
    if tool == "grok" {
        records.push(grok(6, "turn_completed", None));
    }
    let rig = Rig::new(tool, tool, &records);
    let all = rig.run("console", &["--all"]);
    let board = rig.run("board", &["--assistant"]);
    for body in ["human answer", "context answer", "next human answer"] {
        assert!(
            all.contains(body) && board.contains(body),
            "fixture readiness and unchanged all modes: {all}\n{board}"
        );
    }
    let seen = rig.run("console", &[]);
    assert!(
        seen.contains("human answer") && seen.contains("next human answer"),
        "pane replies: {seen}"
    );
    assert!(
        !seen.contains("context answer"),
        "ae turn closes window: {seen}"
    );
}

#[test]
fn codex_filters_replies_by_nearest_user_turn() {
    tool_window("codex");
}

#[test]
fn muse_filters_replies_by_nearest_user_turn() {
    tool_window("muse");
}

#[test]
fn grok_filters_replies_by_nearest_user_turn() {
    tool_window("grok");
}

#[test]
fn codex_plumbing_closes_but_function_results_do_not() {
    let rig = Rig::new("codex-plumbing", "codex", &[
        user("codex", 0, "pane question"),
        r#"{"timestamp":"2026-09-16T09:01:00Z","type":"response_item","payload":{"type":"function_call_output","call_id":"c","output":"hidden tool result"}}"#.to_owned(),
        assistant("codex", 2, "answer after tool result"),
        user("codex", 3, "<turn_aborted>interrupted turn</turn_aborted>"),
        assistant("codex", 4, "answer after interruption"),
    ]);
    let all = rig.run("console", &["--all"]);
    assert!(
        all.contains("answer after tool result") && all.contains("answer after interruption"),
        "fixture readiness: {all}"
    );
    let seen = rig.run("console", &[]);
    assert!(
        seen.contains("answer after tool result"),
        "tool results keep window: {seen}"
    );
    assert!(
        !seen.contains("answer after interruption"),
        "plumbing closes window: {seen}"
    );
}

#[test]
fn codex_passive_launch_turn_closes_the_window() {
    let rig = Rig::new(
        "passive",
        "codex",
        &[
            user("codex", 0, "pane question"),
            assistant("codex", 1, "human reply before passive turn"),
        ],
    );
    let prompt = ae::launch::initial_prompt_for(
        ae::tool::ToolKind::from_binary_name("codex"),
        &rig.session,
        "main",
    );
    let (_, passive) = prompt
        .split_once('\n')
        .expect("fixture passive launch body");
    rig.append(
        (user("codex", 2, passive)
            + "\n"
            + &assistant("codex", 3, "reply to passive launch")
            + "\n")
            .as_bytes(),
    );
    let all = rig.run("console", &["--all"]);
    assert!(
        all.contains("human reply before passive turn") && all.contains("reply to passive launch"),
        "fixture readiness: {all}"
    );
    let seen = rig.run("console", &[]);
    assert!(
        seen.contains("human reply before passive turn"),
        "pane reply shows: {seen}"
    );
    assert!(
        !seen.contains("reply to passive launch"),
        "passive launch closes window: {seen}"
    );
}

#[test]
fn a_human_window_never_crosses_seats_or_generations() {
    let rig = Rig::new(
        "isolation",
        "claude",
        &[
            assistant("claude", 0, "orphan current reply"),
            user("claude", 1, "current human"),
            assistant("claude", 2, "current human answer"),
        ],
    );
    super::board::plant_transcript(
        &rig.root.join("claude"),
        "work",
        OTHER,
        &[
            assistant("claude", 3, "orphan prior reply"),
            user("claude", 4, "prior human"),
            assistant("claude", 5, "prior human answer"),
        ],
    );
    let meta = fs::read_to_string(rig.session.join("meta")).expect("fixture meta");
    let colead = "0199c0de-1234-4890-abcd-ef0123456791";
    super::board::plant_transcript(
        &rig.root.join("claude"),
        "work",
        colead,
        &[
            assistant("claude", 6, "orphan other seat reply"),
            user("claude", 7, "colead human"),
            assistant("claude", 8, "colead human answer"),
        ],
    );
    let meta = format!(
        "{meta}harness_session_prior.main=claude:{OTHER}\n{}",
        super::board::claude_roster("worker.0", "colead", colead, &rig.root.join("claude"))
    );
    fs::write(rig.session.join("meta"), meta).expect("fixture predecessor and second seat");
    let all = rig.run("console", &["--all"]);
    for body in [
        "orphan current reply",
        "orphan prior reply",
        "orphan other seat reply",
        "current human answer",
        "prior human answer",
        "colead human answer",
    ] {
        assert!(all.contains(body), "fixture readiness: {all}");
    }
    let seen = rig.run("console", &[]);
    for body in [
        "current human answer",
        "prior human answer",
        "colead human answer",
    ] {
        assert!(seen.contains(body), "same-generation pane reply: {seen}");
    }
    for body in [
        "orphan current reply",
        "orphan prior reply",
        "orphan other seat reply",
    ] {
        assert!(!seen.contains(body), "new transcript starts closed: {seen}");
    }
}

#[test]
fn follow_carries_the_window_across_polls_and_commits_partial_rows_once() {
    let rig = Rig::new("follow", "claude", &[user("claude", 0, "pane question")]);
    let first = rig.observe();
    assert!(
        first.coverage.is_empty(),
        "fixture readiness: {:?}",
        first.coverage
    );
    assert_eq!(bodies(&first), ["pane question"]);
    let mut follow = Follow::seeded(&first.seeds, &first.coverage, None);
    rig.append((assistant("claude", 1, "first streaming row") + "\n").as_bytes());
    assert_eq!(bodies(&rig.poll(&mut follow)), ["first streaming row"]);
    assert!(
        rig.poll(&mut follow).rows.is_empty(),
        "a repeated poll emits no row twice"
    );
    let tail = assistant("claude", 2, "second streaming row");
    let split = tail.len() / 2;
    rig.append(&tail.as_bytes()[..split]);
    let torn = rig.poll(&mut follow);
    assert!(torn.rows.is_empty(), "partial record must not print");
    assert!(
        torn.coverage.iter().any(|gap| gap.reason.contains("torn")),
        "partial tail is named"
    );
    rig.append(&tail.as_bytes()[split..]);
    rig.append(b"\n");
    assert_eq!(bodies(&rig.poll(&mut follow)), ["second streaming row"]);
    assert!(rig.poll(&mut follow).rows.is_empty());
    rig.append((user("claude", 3, "⟦ae:msg from lead⟧\npeer turn") + "\n").as_bytes());
    assert!(rig.poll(&mut follow).rows.is_empty());
    rig.append((assistant("claude", 4, "late peer reply") + "\n").as_bytes());
    assert!(
        rig.poll(&mut follow).rows.is_empty(),
        "ended window cannot leak a later reply"
    );
    rig.append(
        (user("claude", 5, "new human") + "\n" + &assistant("claude", 6, "new human reply") + "\n")
            .as_bytes(),
    );
    let resumed = rig.poll(&mut follow);
    assert_eq!(bodies(&resumed), ["new human", "new human reply"]);
    assert_eq!(
        resumed.rows.iter().map(|row| row.role).collect::<Vec<_>>(),
        [Role::Human, Role::Assistant]
    );
}

#[test]
fn follow_rescan_starts_closed_after_transcript_replacement() {
    let rig = Rig::new(
        "rescan",
        "claude",
        &[user("claude", 0, &"old human ".repeat(300))],
    );
    let first = rig.observe();
    assert!(
        first.coverage.is_empty(),
        "fixture readiness: {:?}",
        first.coverage
    );
    let mut follow = Follow::seeded(&first.seeds, &first.coverage, None);
    let replacement = [
        assistant("claude", 1, "orphan replacement reply"),
        user("claude", 2, "replacement human"),
        assistant("claude", 3, "replacement human reply"),
    ];
    fs::write(&rig.transcript, replacement.join("\n") + "\n").expect("shorter replacement fixture");
    let seen = rig.poll(&mut follow);
    assert!(
        seen.coverage
            .iter()
            .any(|gap| gap.reason.contains("rescanned")),
        "rescan is named: {:?}",
        seen.coverage
    );
    assert_eq!(
        bodies(&seen),
        ["replacement human", "replacement human reply"],
        "a replaced transcript inherits no human window"
    );
}

#[test]
fn opencode_replies_fail_closed_until_export_order_is_proven() {
    let rig = Rig::new(
        "opencode",
        "claude",
        &[
            user("claude", 0, "classified pane question"),
            assistant("claude", 1, "classified human reply"),
        ],
    );
    let id = "ses_00000000000000000000000000";
    let messages = [
        format!(
            r#"{{"info":{{"id":"msg_user","sessionID":"{id}","role":"user","time":{{"created":1789549200000}}}},"parts":[{{"type":"text","text":"opencode pane question"}}]}}"#
        ),
        format!(
            r#"{{"info":{{"id":"msg_reply","sessionID":"{id}","role":"assistant","time":{{"created":1789549260000}}}},"parts":[{{"type":"text","text":"unclassified opencode reply"}}]}}"#
        ),
    ];
    let export = super::board::export_doc(id, &messages);
    super::board::fake_opencode(&rig.root, &super::board::exporting(&export));
    let meta = fs::read_to_string(rig.session.join("meta")).expect("fixture meta");
    fs::write(rig.session.join("meta"), format!("{meta}seat.worker.0=colead\nagent_bin.worker.0=opencode\nharness_session.worker.0={id}\n"))
        .expect("fixture opencode seat");
    let all = rig.run("console", &["--all"]);
    let board = rig.run("board", &["--assistant"]);
    assert!(
        all.contains("unclassified opencode reply")
            && board.contains("unclassified opencode reply"),
        "fixture readiness and unchanged all modes: {all}\n{board}"
    );
    let seen = rig.run("console", &[]);
    assert!(
        seen.contains("classified human reply"),
        "supported pane replies show: {seen}"
    );
    assert!(
        seen.contains("opencode pane question"),
        "opencode human rows remain: {seen}"
    );
    assert!(
        !seen.contains("unclassified opencode reply"),
        "unproven export order fails closed: {seen}"
    );
}

#[test]
fn agy_replies_fail_closed_without_a_single_classifiable_transcript() {
    let rig = Rig::new(
        "agy",
        "claude",
        &[
            user("claude", 0, "classified pane question"),
            assistant("claude", 1, "classified human reply"),
        ],
    );
    let id = "0199c0de-ffff-4890-abcd-ef0123456789";
    let store = rig.root.join(".gemini/antigravity-cli");
    fs::create_dir_all(&store).expect("fixture agy store");
    fs::write(
        store.join("history.jsonl"),
        super::board::agy_record(Some(id), "agy pane question", MICROS / 1000) + "\n",
    )
    .expect("fixture agy history");
    super::board::plant_agy_transcript(
        &rig.root,
        id,
        "transcript.jsonl",
        &[super::board::agy_tr(1, "unclassified agy reply")],
    );
    let meta = fs::read_to_string(rig.session.join("meta")).expect("fixture meta");
    fs::write(
        rig.session.join("meta"),
        meta + &super::board::agy_roster("worker.0", "colead", id),
    )
    .expect("fixture agy seat");
    let all = rig.run("console", &["--all"]);
    let board = rig.run("board", &["--assistant"]);
    assert!(
        all.contains("unclassified agy reply") && board.contains("unclassified agy reply"),
        "fixture readiness and unchanged all modes: {all}\n{board}"
    );
    let seen = rig.run("console", &[]);
    assert!(
        seen.contains("classified human reply"),
        "supported pane replies show: {seen}"
    );
    assert!(
        seen.contains("agy pane question"),
        "agy human rows remain: {seen}"
    );
    assert!(
        !seen.contains("unclassified agy reply"),
        "separate uncorrelated stores fail closed: {seen}"
    );
    assert!(
        seen.contains("agy: no assistant records"),
        "missing classified replies are named: {seen}"
    );
}
