//! `_auto-reseat`: the leg that moves ONE seat stuck on its vendor usage limit,
//! against a REAL tmux server.
//!
//! It runs on [`super::seat_relaunch::Rig`], the fixture `ae reseat` is pinned
//! on, because the leg moves the seat through that one operation. What the leg
//! decided is read back out of the journal it writes, record by record.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::io::Write as _;

use ae::autoreseat::{
    ATTEMPT_ACTION, DONE_ACTION, Decision, FAILED_ACTION, Frame, HELD_ACTION, Outcome, Pane,
    REFUSED_ACTION, decide, episode,
};
use ae::events::Event;
use ae::time::Timestamp;

use super::seat_relaunch::Rig;

/// The `ts` of the episode's first `limit` record: the key the leg is handed.
const KEY: &str = "2026-09-25T10:00:00Z";

/// The grammar a usage refusal names, which an unknown verb never prints.
const USAGE: &str = "_auto-reseat <dir> <slot> <key>";

/// A spawned claude seat on its usage limit over an empty box, with the
/// watchdog's `limit` record and one attempt at [`KEY`] in its journal — what
/// the watchdog leaves behind when it hands the seat to the leg. The global
/// config is switched `switch` and maps `fake-claude` to `to`.
fn limited(tag: &str, switch: &str, to: &str) -> (Rig, String) {
    let rig = Rig::new(tag);
    configure(&rig, switch, to);
    rig.seat_rows("spawned.0", "scout", "claude", "claude");
    let pane = rig.new_pane("spawned.0", "scout");
    rig.start(&pane, "spawned.0", "claude");
    rig.mark_limited(&pane);
    journal(&rig, "limit", None);
    journal(&rig, ATTEMPT_ACTION, Some(KEY));
    (rig, pane)
}

/// Rewrite the global config's auto reseat switch and its one map row.
fn configure(rig: &Rig, switch: &str, to: &str) {
    let path = rig.scratch.join("config");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let base = text.split("auto_reseat = ").next().unwrap_or_default();
    assert!(
        std::fs::write(
            &path,
            format!("{base}auto_reseat = {switch}\n[auto_reseat]\nfake-claude = {to}\n"),
        )
        .is_ok(),
        "a config"
    );
}

/// Append one watchdog record addressed to the seat, stamped [`KEY`].
fn journal(rig: &Rig, action: &str, reference: Option<&str>) {
    let reference = reference
        .map(|value| format!(r#","ref":"{value}""#))
        .unwrap_or_default();
    let line = format!(
        r#"{{"ts":"{KEY}","actor":"watchdog","action":"{action}","target":"scout","target_slot":"spawned.0","target_session":"{}"{reference}}}"#,
        rig.session
    );
    let appended = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(rig.dir.join("events.jsonl"))
        .and_then(|mut file| writeln!(file, "{line}"));
    assert!(appended.is_ok(), "a journal record");
}

fn records(rig: &Rig) -> Vec<Event> {
    rig.events()
        .lines()
        .filter_map(|line| Event::parse_line(line).ok())
        .collect()
}

fn with_action<'e>(events: &'e [Event], action: &str) -> Vec<&'e Event> {
    events
        .iter()
        .filter(|event| event.action == action)
        .collect()
}

/// Run the leg on the seat at `slot`, handed `key`.
fn leg(rig: &Rig, slot: &str, key: &str) -> (Option<i32>, String, String) {
    rig.run("_auto-reseat", &[slot, key])
}

/// The one outcome record the leg journaled for the seat at `slot`, which the
/// watchdog owns and which the seat's episode fold reads.
fn outcome_at(rig: &Rig, action: &str, slot: &str) -> Event {
    let events = records(rig);
    let found: Vec<&Event> = with_action(&events, action)
        .into_iter()
        .filter(|event| ae::watchdog::event_is_addressed_to(event, &rig.session, slot, "scout"))
        .collect();
    assert_eq!(found.len(), 1, "one {action} for {slot}: {}", rig.events());
    assert_eq!(found[0].actor, "watchdog", "{:?}", found[0]);
    found[0].clone()
}

fn one_outcome(rig: &Rig, action: &str) -> Event {
    outcome_at(rig, action, "spawned.0")
}

/// Nothing of the seat changed: its tool runs, its profile stands, and no
/// reseat was recorded or seeded.
fn untouched(rig: &Rig, pane: &str) {
    assert!(rig.tool_pid(pane, "claude").is_some(), "still running");
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-claude");
    assert!(
        with_action(&records(rig), "reseat").is_empty(),
        "{}",
        rig.events()
    );
    assert!(!rig.dir.join("seed.scout.md").exists(), "no seed");
}

/// The leg exits 1 on the seat, says `why`, journals nothing and moves nothing.
fn silent(rig: &Rig, pane: &str, why: &str) {
    let before = rig.events();
    let (code, out, err) = leg(rig, "spawned.0", KEY);
    assert_eq!(code, Some(1), "out={out} err={err}");
    assert!(err.contains(why), "{err}");
    assert_eq!(rig.events(), before, "journaled nothing");
    untouched(rig, pane);
}

#[test]
fn a_bad_argv_reads_nothing_and_a_stale_or_closed_attempt_moves_nothing() {
    let (rig, pane) = limited("legstale", "on", "fake-opencode");
    let before = rig.events();
    for tail in [
        &["spawned.0"][..],
        &["spawned.0", KEY, "extra"],
        &["main.x", KEY],
        &["spawned.0", "yesterday"],
    ] {
        let (code, out, err) = rig.run("_auto-reseat", tail);
        assert_eq!(code, Some(2), "{tail:?} out={out} err={err}");
        assert!(err.contains(USAGE), "{tail:?}: {err}");
        assert_eq!(rig.events(), before, "{tail:?} journaled nothing");
    }
    let elsewhere = rig.scratch.join("elsewhere").join(&rig.session);
    assert!(std::fs::create_dir_all(&elsewhere).is_ok());
    let (code, out, err) = rig.run_top(
        &rig.main_pane.clone(),
        &[
            "_auto-reseat",
            &elsewhere.display().to_string(),
            "spawned.0",
            KEY,
        ],
    );
    assert_eq!(code, Some(2), "a dir outside the root: out={out} err={err}");
    assert!(err.contains(USAGE), "{err}");
    assert_eq!(rig.events(), before);

    // The episode the watchdog handed over is no longer the seat's: the leg
    // closes the attempt it was handed, under the key it was handed.
    let stale = "2026-09-25T09:00:00Z";
    let (code, out, err) = leg(&rig, "spawned.0", stale);
    assert_eq!(code, Some(1), "out={out} err={err}");
    let refused = one_outcome(&rig, REFUSED_ACTION);
    assert_eq!(refused.reference.as_deref(), Some(stale), "{refused:?}");
    untouched(&rig, &pane);

    // An attempt the watchdog already closed, booked failed past its bound, is
    // not the leg's to act on, and nothing is left for it to close — asked
    // before the switch, which is off here.
    journal(&rig, FAILED_ACTION, Some(KEY));
    configure(&rig, "off", "fake-opencode");
    silent(&rig, &pane, "already ended");
}

#[test]
fn a_seat_gone_never_limited_or_busy_is_not_moved() {
    let (rig, pane) = limited("leggone", "on", "fake-opencode");

    // No seat at the slot: the refusal is routed by slot and session.
    let (code, out, err) = leg(&rig, "spawned.7", KEY);
    assert_eq!(code, Some(1), "out={out} err={err}");
    let gone = outcome_at(&rig, REFUSED_ACTION, "spawned.7");
    assert_eq!(gone.reference.as_deref(), Some(KEY), "{gone:?}");

    // A seat whose journal holds no limit episode at all.
    rig.seat_rows("spawned.1", "quiet", "claude", "claude");
    let (code, out, err) = leg(&rig, "spawned.1", KEY);
    assert_eq!(code, Some(1), "out={out} err={err}");
    let quiet = outcome_at(&rig, REFUSED_ACTION, "spawned.1");
    assert_eq!(quiet.reference.as_deref(), Some(KEY), "{quiet:?}");

    // A running turn is a hold, never a refusal: the episode keeps its retry.
    rig.mark_busy(&pane);
    let (code, out, err) = leg(&rig, "spawned.0", KEY);
    assert_eq!(code, Some(1), "out={out} err={err}");
    let held = one_outcome(&rig, HELD_ACTION);
    assert_eq!(held.reference.as_deref(), Some(KEY), "{held:?}");
    assert!(
        held.summary
            .as_deref()
            .is_some_and(|summary| summary.contains("BUSY")),
        "{held:?}"
    );
    untouched(&rig, &pane);
}

#[test]
fn a_seat_on_its_limit_moves_to_its_declared_candidate_and_the_watchdog_owns_every_record() {
    let (rig, pane) = limited("legmove", "on", "fake-opencode");

    let (code, out, err) = leg(&rig, "spawned.0", KEY);

    assert_eq!(
        code,
        Some(0),
        "out={out} err={err}\nframe was:\n{}",
        rig.capture(&pane)
    );
    assert!(rig.tool_pid(&pane, "opencode").is_some(), "moved in place");
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-opencode");
    let events = records(&rig);
    let reseats = with_action(&events, "reseat");
    assert_eq!(reseats.len(), 2, "the stop and the move: {}", rig.events());
    assert!(
        reseats.iter().all(|event| event.actor == "watchdog"),
        "{reseats:?}"
    );
    let done = one_outcome(&rig, DONE_ACTION);
    assert_eq!(done.reference.as_deref(), Some("fake-claude"), "{done:?}");
    assert_eq!(
        done.summary.as_deref(),
        Some("to fake-opencode, seeded"),
        "{done:?}"
    );
    let received = rig.received();
    assert!(
        received.contains("⟦ae:ctx⟧") && received.contains("## 10. successor instructions"),
        "the successor was handed the seed: {received}"
    );
    assert_eq!(
        episode(&events, &rig.session, "spawned.0", "scout").map(|found| found.terminal),
        Some(Some(Outcome::Done))
    );
}

#[test]
fn a_seat_with_no_usable_candidate_is_refused_where_it_stands() {
    let (rig, pane) = limited("legnone", "on", "ghost");

    let (code, out, err) = leg(&rig, "spawned.0", KEY);

    assert_eq!(code, Some(1), "out={out} err={err}");
    let refused = one_outcome(&rig, REFUSED_ACTION);
    assert_eq!(refused.reference.as_deref(), Some(KEY), "{refused:?}");
    assert_eq!(
        refused.summary.as_deref(),
        Some("refused: no usable candidate: ghost (not configured here)"),
        "the skip is named with its reason: {refused:?}"
    );
    untouched(&rig, &pane);
}

#[test]
fn a_switch_turned_off_between_the_legs_holds_and_the_retry_moves_once_it_is_on() {
    let (rig, pane) = limited("legflip", "off", "fake-opencode");

    let (code, out, err) = leg(&rig, "spawned.0", KEY);

    assert_eq!(code, Some(1), "out={out} err={err}");
    let held = one_outcome(&rig, HELD_ACTION);
    assert_eq!(held.reference.as_deref(), Some(KEY), "{held:?}");
    assert!(
        held.summary
            .as_deref()
            .is_some_and(|summary| summary.starts_with("held: ")),
        "{held:?}"
    );
    untouched(&rig, &pane);

    // The hold closes the attempt without ending the episode, so the fold
    // still owes the seat its one retry.
    let key = Timestamp::parse(KEY).map_or(0, Timestamp::epoch);
    let clear = Pane {
        frame: Frame::Clear,
        human_prompt: false,
        client_input: None,
    };
    let found = episode(&records(&rig), &rig.session, "spawned.0", "scout");
    assert_eq!(
        decide(found.as_ref(), 0, &clear, key + 1),
        Decision::Attempt,
        "{found:?}"
    );

    // The leg moves only under an attempt the watchdog opened, so no call can
    // move the seat ahead of the watchdog's own decision — asked before the
    // switch, off or on.
    silent(&rig, &pane, "no open attempt");
    configure(&rig, "on", "fake-opencode");
    silent(&rig, &pane, "no open attempt");

    journal(&rig, ATTEMPT_ACTION, Some(KEY));
    let (code, out, err) = leg(&rig, "spawned.0", KEY);
    assert_eq!(
        code,
        Some(0),
        "out={out} err={err}\nframe was:\n{}",
        rig.capture(&pane)
    );
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-opencode");
    one_outcome(&rig, DONE_ACTION);
}
