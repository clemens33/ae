//! Distill S1 acceptance tests for the new board observation options.

#![allow(
    clippy::disallowed_methods,
    reason = "the allow-set fixture reads its isolated process export receipt"
)]

use std::fs;
use std::path::{Path, PathBuf};

use ae::board::{self, Inputs, Replies, Scope, TextMode};
use ae::tool::ToolKind;
use ae::usage::SessionInput;

use super::distill_spec::{CURRENT, DISPATCH, Fixture, bodies, transcript, user, user_record};
use super::{claude_roster, fake_opencode, plant_session, plant_transcript};
use crate::cli::{OwnedScratch, helper};

#[test]
fn raw_claude_read_keeps_whitespace_after_stripping_only_reminders() {
    let raw = " \tKeep leading\nline two \t\n";
    let with_reminder =
        " \tKeep leading<system-reminder>fixture-only</system-reminder>\nline two \t\n";
    let dispatch_with_space = format!("{DISPATCH} \n");
    let fixture = Fixture::new(
        "raw",
        &transcript(&[
            user(1, with_reminder, false),
            user(2, DISPATCH, false),
            user(3, &dispatch_with_space, false),
            r#"{"type":"queue-operation","operation":"remove","reason":"absorbed_mid_turn","timestamp":"2026-09-23T19:09:04Z","content":"  absorbed text\t"}"#.to_owned(),
            user_record(
                5,
                r#"[{"type":"text","text":"  First chunk"},{"type":"text","text":"Last chunk\t " }]"#,
                false,
            ),
            user(
                6,
                "<pasted_content id=\"fixture\">\n⟦ae:msg from peer⟧ hidden\n</pasted_content id=\"fixture\">",
                false,
            ),
        ]),
    );
    let inputs = fixture.inputs(Replies::Off);
    let default = board::observe(&inputs, None);
    let scope = Scope {
        text: TextMode::Raw,
        tools: None,
    };
    let raw_obs = board::observe_with(&inputs, None, None, &|_| true, &scope);
    assert_eq!(
        bodies(&default),
        [
            "Keep leading\nline two",
            DISPATCH,
            "absorbed text",
            "First chunk\nLast chunk"
        ]
    );
    assert_eq!(
        bodies(&raw_obs),
        [
            raw,
            dispatch_with_space.as_str(),
            "  absorbed text\t",
            "  First chunk\nLast chunk\t "
        ]
    );
    assert_eq!(raw_obs.dispatches.len(), 1);
    assert_eq!(raw_obs.dispatches[0].count, 1);
    assert_eq!(raw_obs.hidden.len(), 1);
    assert_eq!(raw_obs.hidden[0].count, 2);

    let after_dispatch = ae::time::Timestamp::parse_micros("2026-09-23T19:09:03Z");
    assert!(after_dispatch.is_some());
    let recent = board::observe_with(&inputs, after_dispatch, None, &|_| true, &scope);
    assert!(recent.dispatches.is_empty());
    assert_eq!(recent.hidden.len(), 1);
    assert_eq!(recent.hidden[0].count, 1);
}

const OC_CURRENT: &str = "ses_00000000000000000000000000";
const OC_PRIOR: &str = "ses_00000000000000000000000001";
const CLAUDE_LEGACY: &str = "0199c0de-1234-4890-abcd-ef0123456790";
const CLAUDE_TAGGED: &str = "0199c0de-1234-4890-abcd-ef0123456791";
const CODEX_CURRENT: &str = "01a08046-1974-7352-ade3-81a786200795";
const CODEX_LEGACY: &str = "01a08046-1974-7352-ade3-81a786200796";

#[test]
fn allow_set_refuses_generations_before_running_the_export_door() {
    let root = OwnedScratch::root("bds", "allow-set");
    let store = root.join("store");
    for (id, second, body) in [
        (CURRENT, 3, "current Claude"),
        (CLAUDE_LEGACY, 1, "untagged Claude predecessor"),
        (CLAUDE_TAGGED, 2, "tagged Claude predecessor"),
    ] {
        plant_transcript(&store, "work", id, &[user(second, body, false)]);
    }
    for (id, body) in [
        (CODEX_CURRENT, "current Codex"),
        (CODEX_LEGACY, "untagged Codex predecessor"),
    ] {
        plant_codex(&store, id, body);
    }
    plant_session(
        root.path(),
        "allow-set",
        &format!(
            "{}harness_session_prior.main={CLAUDE_LEGACY},opencode:{OC_PRIOR}\n\
             seat.worker.0=codex-seat\nagent_bin.worker.0=codex\nconfig_home.worker.0={}\n\
             harness_session.worker.0={CODEX_CURRENT}\n\
             harness_session_prior.worker.0={CODEX_LEGACY},claude:{CLAUDE_TAGGED}\n\
             seat.spawned.0=opencode-seat\nagent_bin.spawned.0=opencode\n\
             harness_session.spawned.0={OC_CURRENT}\n",
            claude_roster("main", "lead", CURRENT, &store),
            store.display(),
        ),
    );
    fake_opencode(
        root.path(),
        "#!/bin/sh\nprintf '%s\\n' \"$3\" >> \"$0.calls\"\nexit 23\n",
    );
    let calls = root.join("bin/opencode.calls");
    run_allow_set_child(root.path(), "filtered");
    assert!(!calls.exists(), "a refused OpenCode generation ran");

    // Same fixture and PATH, with no allow-set: both export paths must run.
    // This proves the negative control did not win by hiding the executable.
    run_allow_set_child(root.path(), "default");
    let observed = fs::read_to_string(calls).expect("unfiltered export receipts");
    let mut ids: Vec<_> = observed.lines().collect();
    ids.sort_unstable();
    assert_eq!(ids, [OC_CURRENT, OC_PRIOR]);
}

fn plant_codex(store: &Path, id: &str, body: &str) {
    let dir = store.join("sessions/2026/09/08");
    fs::create_dir_all(&dir).expect("Codex dir");
    fs::write(
        dir.join(format!("rollout-2026-09-23T19-00-00-{id}.jsonl")),
        format!(
            "{{\"type\":\"response_item\",\"timestamp\":\"2026-09-23T19:09:04Z\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"{body}\"}}]}}}}\n"
        ),
    )
    .expect("Codex transcript");
}

fn run_allow_set_child(root: &Path, mode: &str) {
    let child = helper(&std::env::current_exe().expect("the integration test binary"))
        .args([
            "--exact",
            "board::distill_scope_spec::allow_set_child",
            "--ignored",
            "--nocapture",
        ])
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", root.join("bin").display()),
        )
        .env("AE_DISTILL_ACCEPT_ROOT", root)
        .env("AE_DISTILL_ACCEPT_MODE", mode)
        .output()
        .expect("isolated allow-set child");
    assert!(child.status.success(), "{mode}: {child:?}");
    let stdout = String::from_utf8_lossy(&child.stdout);
    assert!(stdout.contains("running 1 test"), "{mode}: {stdout}");
    assert!(stdout.contains("1 passed"), "{mode}: {stdout}");
}

#[test]
#[ignore = "invoked by allow_set_refuses_generations_before_running_the_export_door"]
fn allow_set_child() {
    let root = PathBuf::from(std::env::var_os("AE_DISTILL_ACCEPT_ROOT").expect("fixture root"));
    let sessions = [SessionInput {
        name: "allow-set".to_owned(),
        path: root.join("sessions/allow-set"),
    }];
    let inputs = Inputs {
        home: Some(&root),
        sessions: &sessions,
        replies: Replies::Off,
    };
    if std::env::var("AE_DISTILL_ACCEPT_MODE").as_deref() == Ok("default") {
        let obs = board::observe(&inputs, None);
        let got = bodies(&obs);
        assert!(got.contains(&"current Codex"), "{obs:?}");
        assert!(got.contains(&"untagged Codex predecessor"), "{obs:?}");
        return;
    }
    let obs = board::observe_with(
        &inputs,
        None,
        None,
        &|_| true,
        &Scope {
            text: TextMode::Trimmed,
            tools: Some(&[ToolKind::Claude]),
        },
    );
    let got: Vec<_> = obs
        .rows
        .iter()
        .map(|row| (row.actor.as_str(), row.generation, row.body.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            ("allow-set:lead", 2, "untagged Claude predecessor"),
            ("allow-set:codex-seat", 1, "tagged Claude predecessor"),
            ("allow-set:lead", 0, "current Claude"),
        ]
    );
    let mut coverage: Vec<_> = obs
        .coverage
        .iter()
        .map(|item| (item.actor.as_str(), item.reason.as_str()))
        .collect();
    coverage.sort_unstable();
    assert_eq!(
        coverage,
        [
            ("allow-set:codex-seat", "not read: codex"),
            ("allow-set:codex-seat", "predecessor 2: not read: codex"),
            ("allow-set:lead", "predecessor 1: not read: opencode"),
            ("allow-set:opencode-seat", "not read: opencode"),
        ]
    );
}
