//! Distill S1 acceptance tests that run on the unmodified board API.

use std::fs;

use ae::board::{self, Coverage, Hidden, Inputs, Observation, Replies};
use ae::usage::SessionInput;

use super::{claude_roster, plant_session, plant_transcript};
use crate::cli::OwnedScratch;

pub(super) const CURRENT: &str = "0199c0de-1234-4890-abcd-ef0123456789";
pub(super) const DISPATCH: &str =
    "/compact checkpoint ae-20260923T190939Z-0a5d2b92 saved; compact now, then re-read `ae brief`";

pub(super) struct Fixture {
    root: OwnedScratch,
    session: SessionInput,
}

impl Fixture {
    pub(super) fn new(tag: &str, bytes: &[u8]) -> Self {
        let root = OwnedScratch::root("bds", tag);
        let store = root.join("claude");
        let path = plant_transcript(&store, "work", CURRENT, &[]);
        fs::write(path, bytes).expect("Claude transcript fixture");
        let path = plant_session(
            root.path(),
            "distill-spec",
            &claude_roster("main", "lead", CURRENT, &store),
        );
        Self {
            root,
            session: SessionInput {
                name: "distill-spec".to_owned(),
                path,
            },
        }
    }

    pub(super) fn inputs(&self, replies: Replies) -> Inputs<'_> {
        Inputs {
            home: Some(self.root.path()),
            sessions: std::slice::from_ref(&self.session),
            replies,
        }
    }

    fn observe(&self, replies: Replies) -> Observation {
        board::observe(&self.inputs(replies), None)
    }
}

pub(super) fn user(second: u8, body: &str, meta: bool) -> String {
    let mut escaped = String::new();
    ae::json::escape_into(body, &mut escaped);
    user_record(second, &format!("\"{escaped}\""), meta)
}

pub(super) fn user_record(second: u8, content: &str, meta: bool) -> String {
    format!(
        r#"{{"type":"user","timestamp":"2026-09-23T19:09:{second:02}Z","isMeta":{meta},"message":{{"role":"user","content":{content}}}}}"#
    )
}

fn assistant(second: u8, body: &str) -> String {
    format!(
        r#"{{"type":"assistant","timestamp":"2026-09-23T19:09:{second:02}Z","message":{{"role":"assistant","content":[{{"type":"text","text":"{body}"}}]}}}}"#
    )
}

pub(super) fn transcript(lines: &[String]) -> Vec<u8> {
    let mut joined = lines.join("\n");
    joined.push('\n');
    joined.into_bytes()
}

pub(super) fn bodies(obs: &Observation) -> Vec<&str> {
    obs.rows.iter().map(|row| row.body.as_str()).collect()
}

#[test]
fn usage_notices_drop_only_when_the_meta_field_confirms_them() {
    const AT_LIMIT: &str = "[Usage limit reached; example grace note]";
    const RESET: &str = "[Earlier usage-limit notes no longer apply. Example resume note.]";
    let fixture = Fixture::new(
        "usage",
        &transcript(&[
            user(1, AT_LIMIT, true),
            user(2, RESET, true),
            user(3, AT_LIMIT, false),
            user(4, RESET, false),
            user(5, "typed text tagged isMeta with neither prefix", true),
        ]),
    );
    assert_eq!(
        bodies(&fixture.observe(Replies::Off)),
        [
            AT_LIMIT,
            RESET,
            "typed text tagged isMeta with neither prefix"
        ]
    );
}

#[test]
fn bash_stdout_drops_and_bash_input_and_interior_mentions_stay() {
    const BASH_INPUT: &str = "<bash-input>printf example</bash-input>";
    const INTERIOR: &str = "Quoted output follows.\n<bash-stdout>example</bash-stdout>";
    let fake_output = format!("<bash-stdout>\nghp_{}\n</bash-stdout>", "A0".repeat(18));
    let fixture = Fixture::new(
        "bash",
        &transcript(&[
            user(1, &fake_output, false),
            user(2, BASH_INPUT, false),
            user(3, INTERIOR, false),
        ]),
    );
    assert_eq!(
        bodies(&fixture.observe(Replies::Off)),
        [BASH_INPUT, INTERIOR]
    );
}

#[test]
fn the_exact_guided_dispatch_drops_and_counts_but_near_matches_stay_trimmed() {
    let review_id = DISPATCH.replacen("ae-2026", "review-2026", 1);
    let invalid_id = DISPATCH.replacen("0a5d2b92", "0a5d2b9X", 1);
    let extra_prose = format!("{DISPATCH}\nThen check the fixture.");
    let with_trailing_space = format!("{DISPATCH} \n");
    let with_leading_space = format!(" {DISPATCH}");
    let with_reminder = format!("{DISPATCH}<system-reminder>fixture</system-reminder>");
    let fixture = Fixture::new(
        "dispatch",
        &transcript(&[
            user(1, DISPATCH, false),
            user(2, "/compact", false),
            user(3, "/compact a request", false),
            user(4, "/compact checkpoint x saved", false),
            user(5, &extra_prose, false),
            user(6, &review_id, false),
            user(7, &invalid_id, false),
            user(8, &with_trailing_space, false),
            user(9, &with_leading_space, false),
            user(10, &with_reminder, false),
        ]),
    );
    let obs = fixture.observe(Replies::Off);
    assert_eq!(
        bodies(&obs),
        [
            "/compact",
            "/compact a request",
            "/compact checkpoint x saved",
            extra_prose.as_str(),
            review_id.as_str(),
            invalid_id.as_str(),
            DISPATCH,
            DISPATCH,
            DISPATCH,
        ]
    );
    assert_eq!(
        obs.hidden,
        [Hidden {
            actor: "distill-spec:lead".to_owned(),
            count: 1,
        }]
    );
}

#[test]
fn the_dispatch_closes_a_window_of_replies_to_the_user() {
    let fixture = Fixture::new(
        "dispatch-window",
        &transcript(&[
            user(1, "first question", false),
            assistant(2, "first answer"),
            user(3, DISPATCH, false),
            assistant(4, "dispatch followup"),
            user(5, "next question", false),
            assistant(6, "next answer"),
        ]),
    );
    assert_eq!(
        bodies(&fixture.observe(Replies::ToHuman)),
        [
            "first question",
            "first answer",
            "next question",
            "next answer"
        ]
    );
    assert_eq!(
        bodies(&fixture.observe(Replies::All)),
        [
            "first question",
            "first answer",
            "dispatch followup",
            "next question",
            "next answer",
        ]
    );
}

#[test]
fn this_slice_preserves_grok_board_rows_with_the_same_dispatch_text() {
    const GROK: &str = "0199c0de-3000-4890-abcd-ef0123456789";
    let root = OwnedScratch::root("bds", "grok-dispatch");
    let store = root.join(".grok/sessions/default").join(GROK);
    fs::create_dir_all(&store).expect("Grok transcript dir");
    fs::write(
        store.join("updates.jsonl"),
        format!(
            "{{\"timestamp\":1789549800,\"method\":\"session/update\",\"params\":{{\"sessionId\":\"s\",\"update\":{{\"sessionUpdate\":\"user_message_chunk\",\"content\":{{\"type\":\"text\",\"text\":\"{DISPATCH}\"}}}}}}}}\n"
        ),
    )
    .expect("Grok transcript");
    let sessions = [SessionInput {
        name: "grok-scope".to_owned(),
        path: plant_session(
            root.path(),
            "grok-scope",
            &format!("seat.main=lead\nagent_bin.main=grok\nharness_session.main={GROK}\n"),
        ),
    }];
    let obs = board::observe(
        &Inputs {
            home: Some(root.path()),
            sessions: &sessions,
            replies: Replies::Off,
        },
        None,
    );
    assert_eq!(bodies(&obs), [DISPATCH]);
    assert!(obs.hidden.is_empty());
}

#[test]
fn structured_user_text_stays_and_tool_results_and_meta_turns_never_print() {
    let fixture = Fixture::new(
        "structured",
        &transcript(&[
            user_record(
                1,
                r#"[{"type":"text","text":"Look at this."},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"ZmFrZQ=="}},{"type":"text","text":"Second line."}]"#,
                false,
            ),
            user_record(2, r#"[{"type":"text","text":"Meta note"}]"#, true),
            user_record(
                3,
                r#"[{"type":"tool_result","tool_use_id":"toolu_fixture","content":"tool-only output"}]"#,
                false,
            ),
            user_record(
                4,
                r#"[{"type":"text","text":"mixed text"},{"type":"tool_result","tool_use_id":"toolu_fixture","content":"mixed output"}]"#,
                false,
            ),
            user_record(
                5,
                r#"[{"type":"text","text":"[Request interrupted by user]"}]"#,
                false,
            ),
            user_record(
                6,
                r#"[{"type":"text","text":"⟦ae:msg from peer⟧ synthetic inject"}]"#,
                false,
            ),
            user_record(
                7,
                r#"[{"type":"text","text":"Output with unread parts"},{"type":"other","text":"not a supported part"},{"type":"text","text":42}]"#,
                false,
            ),
        ]),
    );
    let obs = fixture.observe(Replies::Off);
    assert_eq!(
        bodies(&obs),
        [
            "Look at this.\n[image]\nSecond line.",
            "Output with unread parts"
        ]
    );
    assert_eq!(
        obs.coverage,
        [Coverage {
            actor: "distill-spec:lead".to_owned(),
            reason: "2 structured parts not read".to_owned(),
        }]
    );
}

#[test]
fn malformed_complete_lines_count_without_losing_adjacent_valid_records() {
    let mut bytes = transcript(&[user(1, "first", false)]);
    bytes.extend_from_slice(b"\xff\n{bad JSON\n{\"type\":\"system\"}\n");
    bytes.extend(transcript(&[user(2, "second", false)]));
    let fixture = Fixture::new("malformed", &bytes);
    for replies in [Replies::Off, Replies::All, Replies::ToHuman] {
        let obs = fixture.observe(replies);
        assert_eq!(bodies(&obs), ["first", "second"]);
        assert_eq!(
            obs.coverage,
            [Coverage {
                actor: "distill-spec:lead".to_owned(),
                reason: "2 lines not JSON".to_owned(),
            }]
        );
    }
}

#[test]
fn claude_image_row_is_read_without_opening_the_reply_window() {
    let fixture = Fixture::new(
        "image-window",
        &transcript(&[
            user(1, "first question", false),
            assistant(2, "first answer"),
            user_record(
                3,
                r#"[{"type":"text","text":"image question"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAAA"}}]"#,
                false,
            ),
            assistant(4, "image answer"),
            user(5, "next question", false),
            assistant(6, "next answer"),
        ]),
    );
    let raw = board::observe_with(
        &fixture.inputs(Replies::Off),
        None,
        None,
        &|_| true,
        &board::Scope {
            text: board::TextMode::Raw,
            tools: None,
        },
    );
    assert_eq!(
        bodies(&raw),
        ["first question", "image question\n[image]", "next question"]
    );

    let replies = |mode| {
        fixture
            .observe(mode)
            .rows
            .into_iter()
            .filter(|row| row.role == board::Role::Assistant)
            .map(|row| row.body)
            .collect::<Vec<_>>()
    };
    assert_eq!(replies(Replies::ToHuman), ["first answer", "next answer"]);
    assert_eq!(
        replies(Replies::All),
        ["first answer", "image answer", "next answer"]
    );
}
