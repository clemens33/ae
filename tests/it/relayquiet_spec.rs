//! `[workspace] relay_replies`: the orchestrator's chat hides the raw
//! `reply <nonce> from <seat>: …` echo that `relay --status` journals, shows it
//! again under `relay_replies = on` or `ae chat --all`, and keeps the receipt
//! echo, the orchestrator's own `say` and every ordinary chat as they were.
//! The real relay, a physical private tmux and the real `ae chat` read; the
//! echo is produced by the shipped writer, never by a fixture line.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private state and inspects its effects"
)]

use super::relay_chat_spec::{Rig, echo_count, events, words};
use std::fs;

/// A word that only the answer's own text carries.
const MARK: &str = "RQUIET-ANSWER-MARK";
const SAY: &str = "RQUIET-ORCH-SAY";
/// The coverage row's fixed head: stable whatever the count, so a followed
/// chat prints it once.
const HIDDEN: &str = "relay — answer echoes hidden";
const SHAPED: &str = "RQUIET-SAME-SHAPE";

/// A relayed ask the target answered, from the proven dispatcher the relay now
/// requires, with the answer read once through
/// `relay --status` so the reply echo is in the orchestrator's journal.
fn answered(tag: &str) -> (Rig, String) {
    let r = Rig::dispatcher(tag);
    let id = r.ask("please answer here");
    r.reply(&id, &format!("{MARK} the short answer"));
    let out = r.run(&["--status", &id]);
    assert_eq!(out.0, Some(0), "status: {}", words(&out));
    assert!(out.1.contains(MARK), "status prints the answer: {}", out.1);
    (r, id)
}

fn chat(r: &Rig, session: &str, extra: &[&str]) -> String {
    let argv = [&["chat", session][..], extra].concat();
    let out = r.world.run(None, &argv, &[]);
    assert_eq!(out.0, Some(0), "chat read: {}", words(&out));
    words(&out)
}

fn config(r: &Rig, text: &str) {
    fs::write(r.world.root.join("config"), text).expect("global config");
}

fn relay_replies(value: &str) -> String {
    format!("[workspace]\nrelay_replies = {value}\n")
}

#[test]
fn the_orchestrator_chat_hides_the_raw_reply_line_by_default() {
    let (r, id) = answered("rquiet-default");
    // The journal is untouched: the echo is stored, once, with the answer's start.
    let echoes: Vec<_> = events(&r.caller.dir)
        .into_iter()
        .filter(|e| e.action == "chat" && e.actor == "ae:relay")
        .filter(|e| {
            e.summary
                .as_deref()
                .is_some_and(|s| s.starts_with("reply "))
        })
        .collect();
    assert_eq!(echoes.len(), 1, "one stored reply echo");
    assert!(echoes[0].summary.as_deref().unwrap_or("").contains(MARK));
    assert_eq!(echo_count(&r, &id), 2, "receipt + reply, as before");

    let lane = chat(&r, "orch", &[]);
    assert!(!lane.contains(MARK), "raw reply line hidden: {lane}");
    assert!(lane.contains("routed to"), "receipt echo stays: {lane}");
    assert!(lane.contains(&id), "receipt names the request: {lane}");
    assert!(lane.contains(HIDDEN), "one never-silent row: {lane}");
    assert!(
        lane.contains("relay_replies = on"),
        "row names the switch: {lane}"
    );

    // The answer stays readable where it always was.
    let status = r.run(&["--status", &id]);
    assert_eq!(status.0, Some(0));
    assert!(
        status.1.contains(MARK),
        "status still prints it: {}",
        status.1
    );
}

#[test]
fn chat_all_draws_the_raw_reply_line_again_for_one_run() {
    let (r, _id) = answered("rquiet-all");
    let lane = chat(&r, "orch", &["--all"]);
    assert!(lane.contains(MARK), "--all shows the reply echo: {lane}");
    assert!(lane.contains("routed to"), "{lane}");
    assert!(
        !lane.contains("answer echo"),
        "no hidden row when shown: {lane}"
    );
    // One run only: the next plain read hides it again.
    assert!(!chat(&r, "orch", &[]).contains(MARK));
}

#[test]
fn the_global_setting_on_draws_it_and_off_or_anything_else_hides_it() {
    let (r, _id) = answered("rquiet-config");
    config(&r, &relay_replies("on"));
    let shown = chat(&r, "orch", &[]);
    assert!(shown.contains(MARK), "on shows the reply echo: {shown}");
    assert!(!shown.contains("answer echo"), "{shown}");
    for value in ["off", "maybe", "ON"] {
        config(&r, &relay_replies(value));
        let lane = chat(&r, "orch", &[]);
        assert!(!lane.contains(MARK), "{value} hides it: {lane}");
        assert!(lane.contains(HIDDEN), "{value}: {lane}");
    }
    // A present file without the key hides it too; the default arm above is the absent file.
    config(&r, "[workspace]\nchat = on\n");
    assert!(!chat(&r, "orch", &[]).contains(MARK));
}

#[test]
fn the_orchestrators_own_say_stays_beside_the_hidden_echo() {
    let (r, _id) = answered("rquiet-say");
    let dir = r.caller.dir.to_string_lossy();
    let said = r
        .world
        .run(Some(&r.caller.pane), &[ae::cli::SAY, &dir, SAY], &[]);
    assert_eq!(said.0, Some(0), "say: {}", words(&said));
    let lane = chat(&r, "orch", &[]);
    assert!(lane.contains(SAY), "the orchestrator's say shows: {lane}");
    assert!(!lane.contains(MARK), "{lane}");
}

#[test]
fn the_target_session_chat_is_not_touched() {
    let (r, _id) = answered("rquiet-target");
    // The answered session is an ordinary one: its ask and its answer, whole.
    let lane = chat(&r, &r.owner.home, &[]);
    assert!(lane.contains(MARK), "target chat keeps the answer: {lane}");
    assert!(!lane.contains("answer echo"), "no relay row there: {lane}");
}

#[test]
fn an_ordinary_session_keeps_a_reply_shaped_echo_it_may_hold() {
    let (r, _id) = answered("rquiet-ordinary");
    // The scope control: the same writer, action and summary shape in a
    // session that holds no orchestrator role is drawn as it always was.
    let shaped_row = format!(
        r#"{{"ts":"2026-11-19T17:00:00Z","actor":"ae:relay","action":"chat","ref":"ae-20261119T170000Z-0000abcd","summary":"reply fd5bf0 from lead: {SHAPED}"}}"#
    );
    let journal = r.owner.tool.dir.join("events.jsonl");
    let held = fs::read_to_string(&journal).expect("journal");
    fs::write(&journal, format!("{held}{shaped_row}\n")).expect("journal");
    let lane = chat(&r, &r.owner.home, &[]);
    assert!(lane.contains(SHAPED), "an ordinary chat keeps it: {lane}");
    assert!(!lane.contains("answer echo"), "no relay row there: {lane}");
}

#[test]
fn the_orchestrators_real_say_shows_in_its_chat_whatever_relay_replies_says() {
    let (r, _id) = answered("rquiet-say-both");
    let dir = r.caller.dir.to_string_lossy();
    let said = r
        .world
        .run(Some(&r.caller.pane), &[ae::cli::SAY, &dir, SAY], &[]);
    assert_eq!(said.0, Some(0), "say: {}", words(&said));
    for setting in [None, Some("on"), Some("off")] {
        config(&r, &setting.map_or_else(String::new, relay_replies));
        let lane = chat(&r, "orch", &[]);
        assert!(
            lane.contains(SAY),
            "{setting:?}: the seat's say shows: {lane}"
        );
        assert!(
            !lane.contains("from other seats not shown"),
            "{setting:?}: it is the seat's, not another's: {lane}"
        );
    }
}
