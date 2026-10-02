//! Independent mid-turn oracle: Claude's absorbed-remove record represents
//! one human line; queue bookkeeping and attachments leave reply windows alone.
//! All transcript text is hand-written dummy prose, never copied from a store.

use std::path::PathBuf;

use ae::board::follow::Follow;
use ae::board::{self, Inputs, Observation, Replies, Role, Splitter};
use ae::tool::ToolKind;
use ae::usage::SessionInput;

use super::board::{claude_roster, plant_session, plant_transcript, rig};

const ID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const TS: &str = "2026-10-02T12:00:02.000Z";
const FIXTURE: &str = include_str!("../fixtures/board/claude-midturn.jsonl");

struct Transcript {
    root: PathBuf,
    store: PathBuf,
    sessions: Vec<SessionInput>,
}

impl Transcript {
    fn new(tag: &str, lines: &[String]) -> Self {
        let root = rig(tag);
        let store = root.join("store");
        plant_transcript(&store, "dummy", ID, lines);
        let path = plant_session(&root, "s", &claude_roster("main", "lead", ID, &store));
        Self {
            root,
            store,
            sessions: vec![SessionInput {
                name: "s".to_owned(),
                path,
            }],
        }
    }

    fn inputs(&self, replies: Replies) -> Inputs<'_> {
        Inputs {
            home: Some(&self.root),
            sessions: &self.sessions,
            replies,
        }
    }

    fn replace(&self, lines: &[String]) {
        plant_transcript(&self.store, "dummy", ID, lines);
    }

    fn observe(&self, replies: Replies) -> Observation {
        board::observe(&self.inputs(replies), None)
    }
}

fn fixture() -> Vec<String> {
    FIXTURE.lines().map(str::to_owned).collect()
}

// `content` is a JSON value literal, including quotes for string content.
fn absorbed(content: &str) -> String {
    format!(
        r#"{{"type":"queue-operation","operation":"remove","reason":"absorbed_mid_turn","content":{content},"timestamp":"{TS}"}}"#
    )
}

fn user(content: &str) -> String {
    format!(r#"{{"type":"user","message":{{"content":{content}}},"timestamp":"{TS}"}}"#)
}

fn answer(body: &str) -> String {
    format!(
        r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"{body}"}}]}},"timestamp":"2026-10-02T12:00:04.000Z"}}"#
    )
}

fn labels(observed: &Observation) -> Vec<String> {
    observed
        .rows
        .iter()
        .map(|row| {
            let role = match row.role {
                Role::Human => 'H',
                Role::Assistant => 'A',
                Role::Boundary => 'B',
            };
            format!("{role}:{}", row.body)
        })
        .collect()
}

#[test]
fn chat_midturn_absorbed_remove_opens_reply_window_once() {
    let transcript = Transcript::new("midturn-absorbed", &fixture());
    let observed = transcript.observe(Replies::ToHuman);
    assert!(observed.coverage.is_empty(), "{:?}", observed.coverage);
    assert_eq!(
        labels(&observed),
        ["H:Dummy mid-turn question.", "A:Dummy mid-turn answer."]
    );
    let row = &observed.rows[0];
    assert_eq!(row.ts, 1_790_942_402_000_000, "absorption timestamp");
    assert_eq!(row.actor, "s:lead");
    assert_eq!(row.source, ToolKind::Claude);
    assert_eq!(row.generation, 0);
    assert_eq!(
        row.offset,
        (FIXTURE.lines().next().unwrap_or_default().len() + 1) as u64,
        "identity belongs to absorbed-remove, not enqueue"
    );
    assert_eq!(
        labels(&transcript.observe(Replies::Off)),
        ["H:Dummy mid-turn question."]
    );
    assert_eq!(labels(&transcript.observe(Replies::All)), labels(&observed));
}

#[test]
fn chat_midturn_idle_enqueue_dequeue_user_has_one_human_row() {
    let lines = [
        fixture()[0].clone(),
        r#"{"type":"queue-operation","operation":"dequeue","content":"Dummy mid-turn question."}"#
            .to_owned(),
        user(r#""Dummy mid-turn question.""#),
        answer("Dummy idle answer."),
    ];
    let transcript = Transcript::new("midturn-idle", &lines);
    assert_eq!(
        labels(&transcript.observe(Replies::ToHuman)),
        ["H:Dummy mid-turn question.", "A:Dummy idle answer."]
    );
    let enqueue_only = Transcript::new("midturn-enqueue-only", &lines[..1]);
    assert!(enqueue_only.observe(Replies::Off).rows.is_empty());
}

#[test]
fn chat_midturn_agent_markers_and_wrappers_close_reply_window() {
    for (index, body) in [
        r#""⟦ae:msg from lead⟧\nDummy peer message.""#,
        r#""⟦ae:brief from lead⟧\nDummy brief.""#,
        r#""⟦ae:ctx⟧\nDummy context.""#,
        r#""⟦ae:interrupt from lead⟧\nDummy interruption.""#,
        r#""<pasted_content id=\"dummy\">\n⟦ae:msg from lead⟧\nDummy wrapped message.\n</pasted_content id=\"dummy\">""#,
    ].iter().enumerate() {
        let lines = [user(r#""Dummy earlier question.""#), answer("Dummy earlier answer."), absorbed(body), answer("Dummy hidden answer.")];
        let transcript = Transcript::new(&format!("midturn-marker-{index}"), &lines);
        assert_eq!(labels(&transcript.observe(Replies::ToHuman)), ["H:Dummy earlier question.", "A:Dummy earlier answer."], "{body}");
    }
    let transcript = Transcript::new(
        "midturn-quoted-marker",
        &[
            absorbed(r#""Dummy human first line.\n⟦ae:msg from lead⟧""#),
            answer("Dummy visible answer."),
        ],
    );
    assert_eq!(
        labels(&transcript.observe(Replies::ToHuman)),
        [
            "H:Dummy human first line.\n⟦ae:msg from lead⟧",
            "A:Dummy visible answer."
        ]
    );
}

#[test]
fn chat_midturn_queue_plumbing_uses_prefix_user_keeps_field_confirmation() {
    let task = r#""<task-notification>Dummy harness task.</task-notification>""#;
    let lines = [
        user(r#""Dummy earlier question.""#),
        absorbed(task),
        answer("Dummy hidden task answer."),
        user(task),
        answer("Dummy human answer."),
    ];
    let transcript = Transcript::new("midturn-task-prefix", &lines);
    assert_eq!(
        labels(&transcript.observe(Replies::ToHuman)),
        [
            "H:Dummy earlier question.",
            "H:<task-notification>Dummy harness task.</task-notification>",
            "A:Dummy human answer."
        ]
    );

    let lines = [
        absorbed(r#""  Dummy question.<system-reminder>Dummy reminder.</system-reminder>  ""#),
        answer("Dummy stripped answer."),
    ];
    let transcript = Transcript::new("midturn-strip-reminder", &lines);
    assert_eq!(
        labels(&transcript.observe(Replies::ToHuman)),
        ["H:Dummy question.", "A:Dummy stripped answer."]
    );
}

#[test]
fn chat_midturn_unreadable_absorbed_turn_closes_but_bookkeeping_is_neutral() {
    for (index, record) in [
        absorbed("null"),
        absorbed("[]"),
        r#"{"type":"queue-operation","operation":"remove","reason":"absorbed_mid_turn","timestamp":"2026-10-02T12:00:02.000Z"}"#.to_owned(),
        absorbed(r#""   ""#),
        absorbed(r#""<system-reminder>Dummy reminder only.</system-reminder>""#),
        absorbed(r#""[Request interrupted by dummy user]""#),
    ].iter().enumerate() {
        let lines = [user(r#""Dummy earlier question.""#), record.clone(), answer("Dummy hidden answer.")];
        let transcript = Transcript::new(&format!("midturn-unreadable-{index}"), &lines);
        assert_eq!(labels(&transcript.observe(Replies::ToHuman)), ["H:Dummy earlier question."], "{record}");
        assert_eq!(labels(&transcript.observe(Replies::All)), ["H:Dummy earlier question.", "A:Dummy hidden answer."]);
    }
    let mut lines = vec![user(r#""Dummy earlier question.""#)];
    lines.extend([
        r#"{"type":"queue-operation","operation":"enqueue","content":"⟦ae:msg from lead⟧\nDummy pending message."}"#.to_owned(),
        r#"{"type":"queue-operation","operation":"dequeue","content":null}"#.to_owned(),
        r#"{"type":"queue-operation","operation":"remove","reason":"other","content":"Dummy unrelated removal."}"#.to_owned(),
        r#"{"type":"queue-operation","operation":"other"}"#.to_owned(),
        fixture()[2].clone(),
        fixture()[3].clone(),
        fixture()[4].clone(),
        r#"{"type":"summary"}"#.to_owned(),
        answer("Dummy visible answer."),
        r#"{"typo":"user"}"#.to_owned(),
        answer("Dummy hidden answer."),
    ]);
    let transcript = Transcript::new("midturn-neutral-records", &lines);
    assert_eq!(
        labels(&transcript.observe(Replies::ToHuman)),
        ["H:Dummy earlier question.", "A:Dummy visible answer."]
    );
}

#[test]
fn chat_midturn_missing_absorption_timestamp_reports_coverage_and_closes() {
    for (index, stamp) in ["", r#", "timestamp":"invalid""#].iter().enumerate() {
        let record = format!(
            r#"{{"type":"queue-operation","operation":"remove","reason":"absorbed_mid_turn","content":"Dummy unstamped question."{stamp}}}"#
        );
        let transcript = Transcript::new(
            &format!("midturn-timestamp-{index}"),
            &[
                user(r#""Dummy earlier question.""#),
                record,
                answer("Dummy hidden answer."),
            ],
        );
        let observed = transcript.observe(Replies::ToHuman);
        assert_eq!(labels(&observed), ["H:Dummy earlier question."]);
        assert_eq!(observed.coverage.len(), 1);
        assert_eq!(observed.coverage[0].actor, "s:lead");
        assert_eq!(observed.coverage[0].reason, "1 record without a timestamp");
    }
}

#[test]
fn chat_midturn_follow_absorption_and_answer_in_separate_polls_agree_with_one_shot() {
    let mut lines = fixture();
    let transcript = Transcript::new("midturn-follow", &lines[..1]);
    let first = transcript.observe(Replies::ToHuman);
    assert!(first.rows.is_empty());
    assert!(first.coverage.is_empty());
    let mut follow = Follow::seeded(&first.seeds, &first.coverage, None);
    transcript.replace(&lines[..2]);
    let human = board::follow_poll(&transcript.inputs(Replies::ToHuman), &mut follow);
    assert_eq!(labels(&human), ["H:Dummy mid-turn question."]);
    transcript.replace(&lines[..5]);
    let neutral = board::follow_poll(&transcript.inputs(Replies::ToHuman), &mut follow);
    assert!(neutral.rows.is_empty());
    transcript.replace(&lines);
    let reply = board::follow_poll(&transcript.inputs(Replies::ToHuman), &mut follow);
    assert_eq!(labels(&reply), ["A:Dummy mid-turn answer."]);
    let streamed: Vec<_> = [first, human, neutral, reply]
        .iter()
        .flat_map(labels)
        .collect();
    assert_eq!(streamed, labels(&transcript.observe(Replies::ToHuman)));
    lines.push(absorbed(r#""⟦ae:msg from lead⟧\nDummy peer message.""#));
    transcript.replace(&lines);
    assert!(
        board::follow_poll(&transcript.inputs(Replies::ToHuman), &mut follow)
            .rows
            .is_empty()
    );
    lines.push(answer("Dummy hidden later answer."));
    transcript.replace(&lines);
    assert!(
        board::follow_poll(&transcript.inputs(Replies::ToHuman), &mut follow)
            .rows
            .is_empty()
    );
}

#[test]
fn chat_midturn_public_reader_accepts_fixture_in_small_byte_chunks() {
    let direct = board::claude::read(
        FIXTURE.as_bytes(),
        "s:lead",
        "dummy.jsonl",
        ToolKind::Claude,
    );
    let mut splitter = Splitter::new();
    for chunk in FIXTURE.as_bytes().chunks(7) {
        splitter.feed(chunk);
    }
    let streamed = board::claude::read_stream(
        &splitter.finish(),
        "s:lead",
        "dummy.jsonl",
        ToolKind::Claude,
    );
    assert_eq!(streamed, direct);
    assert!(direct.1.is_empty());
    assert_eq!(direct.0.len(), 1);
    assert_eq!(direct.0[0].body, "Dummy mid-turn question.");
}
