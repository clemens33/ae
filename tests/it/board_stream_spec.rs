//! Frozen acceptance spec for transcript streaming (brief appload2, R1-R6).
//!
//! Captures and verifies goldens of `ae::board::observe` and
//! `ae::board::observe_selected` across all five supported harnesses (Claude,
//! Codex, Grok, Muse, Agy) with current and predecessor conversations and all
//! three reply modes (`Replies::Off`, `Replies::All`, `Replies::ToHuman`),
//! plus all 105 tracked seeds across the five splitter fuzz targets.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixture setup across scratch stores"
)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use ae::board::{self, Inputs, Observation, Replies, Role, Row};
use ae::usage::SessionInput;

use super::board::{
    agy_record, agy_roster, agy_tr, claude_roster, plant_agy_transcript, plant_session,
    plant_transcript, rig, user,
};

const CLAUDE_ID_0: &str = "0199c0de-1000-4890-abcd-ef0123456789";
const CLAUDE_ID_1: &str = "0199c0de-1001-4890-abcd-ef0123456789";
const CODEX_ID: &str = "01a08046-1974-7352-ade3-81a786200795";
const GROK_ID: &str = "0199c0de-3000-4890-abcd-ef0123456789";
const MUSE_ID: &str = "0199c0de-4000-4890-abcd-ef0123456789";
const AGY_ID: &str = "0199c0de-5000-4890-abcd-ef0123456789";

const TARGETS: [&str; 5] = [
    "board_agy",
    "board_claude",
    "board_codex",
    "board_grok",
    "board_muse",
];

struct Rig {
    root: PathBuf,
    session_dir: PathBuf,
}

impl Rig {
    fn setup(tag: &str) -> Self {
        let root = rig(tag);
        let claude_store = root.join("claude");
        plant_transcript(
            &claude_store,
            "work",
            CLAUDE_ID_0,
            &[
                user("2026-10-10T10:00:00Z", "claude lead gen0 user prompt"),
                r#"{"type":"assistant","timestamp":"2026-10-10T10:01:00Z","message":{"role":"assistant","content":[{"type":"text","text":"claude lead gen0 reply"}]}}"#.to_string(),
                user("2026-10-10T10:02:00Z", "<pasted_content id=\"1\">\n⟦ae:msg from colead⟧ wrapped message\n</pasted_content>"),
            ],
        );
        plant_transcript(
            &claude_store,
            "work",
            CLAUDE_ID_1,
            &[
                user("2026-10-10T08:00:00Z", "claude lead prior user prompt"),
                r#"{"type":"assistant","timestamp":"2026-10-10T08:01:00Z","message":{"role":"assistant","content":[{"type":"text","text":"claude lead prior reply"}]}}"#.to_string(),
            ],
        );

        let codex_store = root.join("codex");
        let codex_dir = codex_store.join("sessions/2026/09/08");
        fs::create_dir_all(&codex_dir).expect("codex dir");
        let codex_path = codex_dir.join(format!("rollout-2026-09-08T09-00-00-{CODEX_ID}.jsonl"));
        fs::write(codex_path, r#"{"type":"response_item","timestamp":"2026-09-08T09:10:00Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"codex colead prompt"}]}}
{"type":"response_item","timestamp":"2026-09-08T09:11:00Z","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"codex colead reply"}]}}
"#).expect("codex rollout");

        let grok_dir = root.join(".grok/sessions/default").join(GROK_ID);
        fs::create_dir_all(&grok_dir).expect("grok dir");
        fs::write(grok_dir.join("updates.jsonl"), r#"{"timestamp":1789549800,"method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"grok user turn"}}}}
{"timestamp":1789549810,"method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"grok open reply chunk"}}}}
"#).expect("grok updates");

        let muse_dir = root
            .join(".local/share/muse/sessions/2026/10/10")
            .join(MUSE_ID);
        fs::create_dir_all(&muse_dir).expect("muse dir");
        fs::write(muse_dir.join("session.jsonl"), r#"{"recorded_at":1789565338436454,"payload_type":"runtime.user_intent.accepted","payload":{"refill_blocks":[{"kind":"text","text":"stub"}],"model_messages":[{"content":[{"kind":"text","text":"muse prompt"}]}]}}
{"recorded_at":1789565340436454,"payload_type":"runtime.session","payload":{"event":{"kind":"assistant_message_committed","message_id":"m","response_id":"r","provider_item_id":"p","text":"muse reply"}}}
"#).expect("muse session");

        let agy_store = root.join(".gemini/antigravity-cli");
        fs::create_dir_all(&agy_store).expect("agy store");
        fs::write(
            agy_store.join("history.jsonl"),
            agy_record(Some(AGY_ID), "agy prompt", 1_789_549_300_000) + "\n",
        )
        .expect("agy history");
        plant_agy_transcript(
            &root,
            AGY_ID,
            "transcript.jsonl",
            &[agy_tr(1, "agy planner reply")],
        );

        let roster = format!(
            "{}\nprior.0.main={CLAUDE_ID_1}\nseat.worker.0=colead\nagent_bin.worker.0=codex\nconfig_home.worker.0={}\nharness_session.worker.0={CODEX_ID}\nseat.spawned.0=grok-worker\nagent_bin.spawned.0=grok\nharness_session.spawned.0={GROK_ID}\nseat.spawned.1=muse-worker\nagent_bin.spawned.1=muse\nharness_session.spawned.1={MUSE_ID}\n{}",
            claude_roster("main", "lead", CLAUDE_ID_0, &claude_store),
            codex_store.display(),
            agy_roster("spawned.2", "agy-worker", AGY_ID)
        );
        let session_dir = plant_session(&root, "s-stream", &roster);
        Self { root, session_dir }
    }

    fn inputs(&self, replies: Replies) -> (Inputs<'_>, Vec<SessionInput>) {
        let sessions = vec![SessionInput {
            name: "s-stream".to_owned(),
            path: self.session_dir.clone(),
        }];
        let inputs = Inputs {
            home: Some(self.root.as_path()),
            sessions: &[],
            replies,
        };
        (inputs, sessions)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct GoldenRow {
    actor: String,
    generation: u8,
    role: String,
    body: String,
    offset: u64,
    ts: i64,
}

impl From<&Row> for GoldenRow {
    fn from(r: &Row) -> Self {
        Self {
            actor: r.actor.clone(),
            generation: r.generation,
            role: match r.role {
                Role::Human => "Human".to_owned(),
                Role::Assistant => "Assistant".to_owned(),
                Role::Boundary => "Boundary".to_owned(),
            },
            body: r.body.clone(),
            offset: r.offset,
            ts: r.ts,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct GoldenSeed {
    actor: String,
    committed: u64,
    fp: u64,
}

fn seed_facts(seed: &board::SeatSeed) -> GoldenSeed {
    let s = format!("{seed:?}");
    let parse = |pat: &str, end: char| {
        s.split(pat)
            .nth(1)
            .and_then(|p| p.split(end).next())
            .unwrap_or("")
            .trim()
    };
    GoldenSeed {
        actor: parse("actor: \"", '"').to_owned(),
        committed: parse("committed: ", ',').parse().unwrap_or(0),
        fp: parse("fp: ", ',').parse().unwrap_or(0),
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

fn format_rows_cov(out: &mut String, rows: &[GoldenRow], cov: &[(String, String)]) {
    out.push_str("    \"rows\": [\n");
    for (i, r) in rows.iter().enumerate() {
        let comma = if i + 1 == rows.len() { "" } else { "," };
        let _ = writeln!(
            out,
            "      {{\"actor\": \"{}\", \"generation\": {}, \"role\": \"{}\", \"body\": \"{}\", \"offset\": {}, \"ts\": {}}}{comma}",
            r.actor,
            r.generation,
            r.role,
            json_escape(&r.body),
            r.offset,
            r.ts
        );
    }
    out.push_str("    ],\n    \"coverage\": [\n");
    for (i, c) in cov.iter().enumerate() {
        let comma = if i + 1 == cov.len() { "" } else { "," };
        let _ = writeln!(
            out,
            "      {{\"actor\": \"{}\", \"reason\": \"{}\"}}{comma}",
            c.0, c.1
        );
    }
    out.push_str("    ]\n");
}

fn golden_snapshot(obs: &Observation) -> (Vec<GoldenRow>, Vec<(String, String)>, Vec<GoldenSeed>) {
    let rows: Vec<GoldenRow> = obs.rows.iter().map(GoldenRow::from).collect();
    let coverage: Vec<(String, String)> = obs
        .coverage
        .iter()
        .map(|c| (c.actor.clone(), c.reason.clone()))
        .collect();
    let seeds: Vec<GoldenSeed> = obs.seeds.iter().map(seed_facts).collect();
    (rows, coverage, seeds)
}

fn format_json_goldens(
    off: &(Vec<GoldenRow>, Vec<(String, String)>, Vec<GoldenSeed>),
    all: &(Vec<GoldenRow>, Vec<(String, String)>, Vec<GoldenSeed>),
    to_human: &(Vec<GoldenRow>, Vec<(String, String)>, Vec<GoldenSeed>),
) -> String {
    let mut out = String::from("{\n");
    for (idx, (name, section)) in [
        ("replies_off", off),
        ("replies_all", all),
        ("replies_to_human", to_human),
    ]
    .iter()
    .enumerate()
    {
        let comma = if idx == 2 { "" } else { ",\n" };
        let _ = writeln!(out, "  \"{name}\": {{\n    \"seeds\": [");
        for (i, s) in section.2.iter().enumerate() {
            let s_comma = if i + 1 == section.2.len() { "" } else { "," };
            let _ = writeln!(
                out,
                "      {{\"actor\": \"{}\", \"committed\": {}, \"fp\": {}}}{s_comma}",
                s.actor, s.committed, s.fp
            );
        }
        out.push_str("    ],\n");
        format_rows_cov(&mut out, &section.0, &section.1);
        let _ = write!(out, "  }}{comma}");
    }
    out.push('\n');
    out
}

#[test]
fn record_or_verify_goldens() {
    let rig = Rig::setup("stream-spec-multi");
    let (mut inputs_off, sessions) = rig.inputs(Replies::Off);
    inputs_off.sessions = &sessions;
    let obs_off = board::observe(&inputs_off, None);

    let (mut inputs_all, sessions) = rig.inputs(Replies::All);
    inputs_all.sessions = &sessions;
    let obs_all = board::observe(&inputs_all, None);

    let (mut inputs_human, sessions) = rig.inputs(Replies::ToHuman);
    inputs_human.sessions = &sessions;
    let obs_human = board::observe(&inputs_human, None);

    let snap_off = golden_snapshot(&obs_off);
    let snap_all = golden_snapshot(&obs_all);
    let snap_human = golden_snapshot(&obs_human);

    let golden_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/it/board_stream_goldens.json");
    if std::env::var("GEN_GOLDENS").is_ok() {
        let json = format_json_goldens(&snap_off, &snap_all, &snap_human);
        fs::write(&golden_path, json).expect("write goldens");
    }

    assert!(
        snap_off.0.iter().all(|r| r.role == "Human"),
        "Replies::Off human only"
    );
    assert!(
        snap_all.0.iter().any(|r| r.role == "Human")
            && snap_all.0.iter().any(|r| r.role == "Assistant"),
        "Replies::All mixed"
    );
    assert_eq!(obs_off.seeds.len(), 5, "all 5 seats seeded");

    let golden = include_str!("board_stream_goldens.json");
    let actual_json = format_json_goldens(&snap_off, &snap_all, &snap_human);
    assert_eq!(
        actual_json, golden,
        "observe outputs match frozen golden baseline"
    );
    let _ = fs::remove_dir_all(&rig.root);
}

#[test]
fn observe_selected_matches_observe_on_multi_harness() {
    let rig = Rig::setup("stream-spec-selected");
    let (mut inputs, sessions) = rig.inputs(Replies::All);
    inputs.sessions = &sessions;

    let unselected = board::observe(&inputs, None);
    let selected = board::observe_selected(&inputs, None, None, &|_| true);
    let snap_unsel = golden_snapshot(&unselected);
    let snap_sel = golden_snapshot(&selected);

    assert_eq!(
        snap_unsel.0, snap_sel.0,
        "rows between observe and observe_selected match"
    );
    assert_eq!(snap_unsel.1, snap_sel.1, "coverage match");
    assert_eq!(snap_unsel.2, snap_sel.2, "seeds match");
    let _ = fs::remove_dir_all(&rig.root);
}

#[test]
fn grok_midturn_seed_holds_and_follow_poll_joins_completed_run() {
    let rig = Rig::setup("stream-spec-grok-follow");
    let (mut inputs, sessions) = rig.inputs(Replies::All);
    inputs.sessions = &sessions;

    let obs = board::observe(&inputs, None);
    let seeds = golden_snapshot(&obs).2;
    let grok_seed = seeds
        .iter()
        .find(|s| s.actor == "s-stream:grok-worker")
        .expect("grok seed");
    let held_offset = grok_seed.committed;
    assert!(held_offset > 0, "grok seed must hold at agent chunk, not 0");

    let mut follow = board::follow::Follow::seeded(&obs.seeds, &obs.coverage, None);
    let grok_path = rig
        .root
        .join(".grok/sessions/default")
        .join(GROK_ID)
        .join("updates.jsonl");
    let mut grok_content = fs::read_to_string(&grok_path).expect("read grok");
    let next_agent = r#"{"timestamp":1789549820,"method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":" and completed"}}}}"#;
    let turn_close = r#"{"timestamp":1789549830,"method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"turn_completed"}}}"#;
    grok_content.push_str(next_agent);
    grok_content.push('\n');
    grok_content.push_str(turn_close);
    grok_content.push('\n');
    fs::write(&grok_path, grok_content).expect("write grok append");

    let batch = board::follow_poll(&inputs, &mut follow);
    let grok_rows: Vec<&Row> = batch
        .rows
        .iter()
        .filter(|r| r.actor == "s-stream:grok-worker")
        .collect();
    assert_eq!(
        grok_rows.len(),
        1,
        "follow_poll must yield exactly ONE joined assistant row"
    );
    let joined = grok_rows[0];
    assert_eq!(joined.role, Role::Assistant);
    assert_eq!(
        joined.offset, held_offset,
        "joined row offset must be at the held offset"
    );
    assert_eq!(
        joined.body, "grok open reply chunk and completed",
        "joined row body combines chunks"
    );
    let _ = fs::remove_dir_all(&rig.root);
}

fn run_seed_grammar(target: &str, data: &[u8]) -> (Vec<GoldenRow>, Vec<(String, String)>) {
    let (chunk, rest) = match data.split_first() {
        Some((size, rest)) => (usize::from(*size) % 256 + 1, rest),
        None => (1, data),
    };
    if target == "board_agy" {
        let (flag, bytes) = match rest.split_first() {
            Some((flag, rest)) => (*flag, rest),
            None => (0, rest),
        };
        let mut splitter = board::Splitter::new();
        for piece in bytes.chunks(chunk) {
            splitter.feed(piece);
        }
        let streamed = splitter
            .finish()
            .for_seat("0199c0de-ffff-4890-abcd-ef0123456789")
            .with_assistant(flag & 1 == 1)
            .with_assistant_rows_found(flag & 8 == 8)
            .with_assistant_read_once(flag & 16 == 16);
        let (rows, coverage) = if flag & 2 == 0 {
            board::agy::read_stream(&streamed, "s:seat", "fuzz.jsonl", ae::tool::ToolKind::Agy)
        } else {
            board::agy_transcript::read_stream(
                &streamed,
                "s:seat",
                "agy:0199c0de-ffff-4890-abcd-ef0123456789",
                ae::tool::ToolKind::Agy,
                flag & 4 == 4,
            )
        };
        (
            rows.iter().map(GoldenRow::from).collect(),
            coverage.into_iter().map(|c| (c.actor, c.reason)).collect(),
        )
    } else {
        let (replies, bytes) = match rest.split_first() {
            Some((flag, rest)) if flag & 4 == 4 => (Replies::ToHuman, rest),
            Some((flag, rest)) if flag & 1 == 1 => (Replies::All, rest),
            Some((_, rest)) => (Replies::Off, rest),
            None => (Replies::Off, rest),
        };
        let mut splitter = board::Splitter::new();
        for piece in bytes.chunks(chunk) {
            splitter.feed(piece);
        }
        let streamed = splitter.finish().with_replies(replies);
        let (reader, kind): (fn(&_, _, _, _) -> _, _) = match target {
            "board_claude" => (board::claude::read_stream, ae::tool::ToolKind::Claude),
            "board_codex" => (board::codex::read_stream, ae::tool::ToolKind::Codex),
            "board_grok" => (board::grok::read_stream, ae::tool::ToolKind::Grok),
            "board_muse" => (board::muse::read_stream, ae::tool::ToolKind::Muse),
            _ => unreachable!(),
        };
        let (rows, coverage) = reader(&streamed, "s:seat", "fuzz.jsonl", kind);
        (
            rows.iter().map(GoldenRow::from).collect(),
            coverage.into_iter().map(|c| (c.actor, c.reason)).collect(),
        )
    }
}

type SeedEntry = (String, Vec<GoldenRow>, Vec<(String, String)>);

fn format_json_seed_goldens(entries: &[SeedEntry]) -> String {
    let mut out = String::from("{\n");
    for (idx, (key, rows, cov)) in entries.iter().enumerate() {
        let comma = if idx + 1 == entries.len() { "" } else { "," };
        let _ = writeln!(out, "  \"{key}\": {{");
        format_rows_cov(&mut out, rows, cov);
        let _ = writeln!(out, "  }}{comma}");
    }
    out.push_str("}\n");
    out
}

#[test]
fn record_or_verify_seed_goldens() {
    let seeds_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/seeds");
    let mut entries = Vec::new();
    for target in TARGETS {
        let target_dir = seeds_root.join(target);
        let mut file_names = Vec::new();
        for entry in fs::read_dir(&target_dir).expect("read target dir") {
            let entry = entry.expect("dir entry");
            if entry.file_type().expect("file type").is_file() {
                file_names.push(entry.file_name());
            }
        }
        file_names.sort();
        for name in file_names {
            let file_name_str = name.to_string_lossy();
            let path = target_dir.join(&name);
            let data = fs::read(&path).expect("read seed file");
            let (rows, coverage) = run_seed_grammar(target, &data);
            entries.push((format!("{target}/{file_name_str}"), rows, coverage));
        }
    }
    assert_eq!(entries.len(), 105, "must cover exactly 105 tracked seeds");
    let golden_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/it/board_stream_seed_goldens.json");
    if std::env::var("GEN_GOLDENS").is_ok() {
        let json = format_json_seed_goldens(&entries);
        fs::write(&golden_path, json).expect("write seed goldens");
    }
    let golden = include_str!("board_stream_seed_goldens.json");
    let actual_json = format_json_seed_goldens(&entries);
    assert_eq!(
        actual_json, golden,
        "seed reader outputs match frozen golden baseline"
    );
}
