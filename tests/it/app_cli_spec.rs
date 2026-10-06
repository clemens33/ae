//! sideapp stage-B spec, part 1: CLI and integration pins for `ae app`.
//!
//! Oracles: the brief (R1/R2), the plan's §2 refusal line, and the existing
//! owners (`entry::ROUTED_VERBS`, `config::chat_window`, the `ae chat` lane).
//! RED on base by assertion, except the chat-bytes guard, which is GREEN on
//! base and pins R1 through every phase.
//!
//! Split note (window-0 argv): `toggle::command` is private, so this file pins
//! the decision it consumes — `chat = app` on the existing key, local over
//! global. The argv construction itself is the driver's F4 unit pin over
//! `command()` under `chat = app`; the freeze review checks both.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns isolated journal and config fixtures"
)]

use super::board::{claude_roster, plant_session, plant_transcript};
use super::cli::ae;

const LEAD_ID: &str = "0199c0de-1234-4890-abcd-ef0123456789";
const COLEAD_ID: &str = "0199c0de-1234-4890-abcd-ef0123456790";

/// `app` is a routed verb: no new session takes the name, and `ae app` never
/// reaches a session. RED on base: `app` is launchable there.
#[test]
fn app_verb_is_routed() {
    assert!(
        ae::entry::ROUTED_VERBS.contains(&"app"),
        "ROUTED_VERBS must reserve app"
    );
    assert!(
        ae::entry::name_is_routed_verb("app"),
        "app must refuse as a session name"
    );
    assert!(
        !ae::entry::name_is_routed_verb("appx"),
        "the reservation is the exact word"
    );
}

/// The usage names the verb with its session argument. RED on base.
#[test]
fn app_help_names_the_verb() {
    let out = ae().arg("help").output().expect("the ae binary should run");
    assert_eq!(out.status.code(), Some(0), "help exits 0");
    let stdout = String::from_utf8(out.stdout).expect("help UTF-8");
    assert!(
        stdout.contains("ae app [session]"),
        "usage names ae app: {stdout:?}"
    );
}

/// Without a terminal on either stream the app refuses with one line (plan
/// §2). The stream follows ae's error convention — stdout stays empty for a
/// machine caller, the diagnosis goes to stderr (cf. `sc_022`). RED on base:
/// `app` launches there instead of refusing.
#[test]
fn app_without_a_terminal_refuses_with_one_line() {
    for tail in [&["app"][..], &["app", "lane"][..]] {
        let out = ae().args(tail).output().expect("the ae binary should run");
        assert_eq!(
            out.status.code(),
            Some(1),
            "refusal exits 1: {tail:?} {:?}",
            out.status
        );
        assert!(
            out.stdout.is_empty(),
            "stdout stays empty: {:?}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert_eq!(
            String::from_utf8(out.stderr).expect("stderr UTF-8"),
            "ae app draws a terminal UI; use ae chat to print the lane\n",
            "the one refusal line: {tail:?}"
        );
    }
}

/// `chat = app` is a third value on the existing key (D4): the value the
/// window-0 argv builder consumes. Local wins over global, as the key reads
/// today. Pinned through `Debug` so RED lands on the assertion, not on naming
/// a variant base does not have. RED on base: `app` parses as Unusable.
#[test]
fn app_chat_key_takes_app_value() {
    assert_eq!(
        format!("{:?}", ae::config::chat_window(Some("app"))),
        "App",
        "the bare value"
    );
    let scratch = super::cli::OwnedScratch::root("app-cli", "chat-key");
    let root = scratch.path();
    let global = root.join("global.config");
    let local = root.join("local.config");
    std::fs::write(&global, "[workspace]\nchat = off\n").expect("global config");
    std::fs::write(&local, "[workspace]\nchat = app\n").expect("local config");
    assert_eq!(
        format!(
            "{:?}",
            ae::config::workspace_chat_window(Some(&global), Some(&local))
        ),
        "App",
        "local app wins over global off"
    );
    assert_eq!(
        format!(
            "{:?}",
            ae::config::workspace_chat_window(Some(&local), None)
        ),
        "App",
        "a global app reads too"
    );
}

/// R1 guard: the chat's one-shot lane bytes on a fixture, frozen on the
/// rebase tip (chat-needs-you c860c2b6). The lane compares byte-exact; the
/// needs section below it pins seat count and body rows byte-exact and the
/// wall-clock stamp by shape only (`HH:MM:SS`), following the
/// `split_needs_snapshot` precedent — the clock cannot freeze.
#[test]
fn app_chat_lane_bytes_unchanged_on_fixture() {
    let scratch = super::cli::OwnedScratch::root("app-cli", "chat-bytes");
    let root = scratch.path();
    let store = root.join("claude");
    let roster = format!(
        "session_id=0199c0de-aaaa-4890-abcd-ef0123456789\nlayout=lead-pair\n{}{}",
        claude_roster("main", "lead", LEAD_ID, &store),
        claude_roster("worker.0", "colead", COLEAD_ID, &store),
    );
    let dir = plant_session(root, "lane", &roster);
    for id in [LEAD_ID, COLEAD_ID] {
        plant_transcript(&store, "work", id, &[]);
    }
    std::fs::write(
        dir.join("events.jsonl"),
        include_str!("../fixtures/console/app-chat-events.jsonl"),
    )
    .expect("fixture journal");
    let out = ae()
        .env("HOME", root)
        .env("AE_HOME", root)
        .env_remove("TMUX_PANE")
        .args(["chat", "lane"])
        .output()
        .expect("the ae binary should run");
    assert!(
        out.status.success(),
        "chat exit: {:?}\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("chat UTF-8");
    let Some((lane, section)) = stdout.split_once("-- needs you: ") else {
        panic!("the needs section follows the lane:\n{stdout}");
    };
    assert_eq!(
        lane,
        include_str!("../fixtures/console/app-chat-lane-plain.txt"),
        "R1: lane bytes frozen on the rebase tip"
    );
    let (header, body) = section.split_once('\n').expect("section has a body");
    let clock = header
        .strip_prefix("0 seats · as of ")
        .expect("fixture seat count + snapshot time");
    let bytes = clock.as_bytes();
    assert!(
        bytes.len() == 8 && bytes[2] == b':' && bytes[5] == b':',
        "the stamp is a wall clock by shape only: {clock:?}"
    );
    assert_eq!(
        body, "  unverified: watchdog off · 2 seats: lead, colead\n",
        "the section body rows are deterministic"
    );
}
