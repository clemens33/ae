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

use std::fmt::Write as _;
use std::io::Write as _;
use std::time::{Duration, Instant};

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
    limited_on(tag, "claude", switch, to)
}

/// The same seat on the profile `fake-<on>`, which the map row is keyed by.
fn limited_on(tag: &str, on: &str, switch: &str, to: &str) -> (Rig, String) {
    let rig = Rig::new(tag);
    configure_from(&rig, switch, &format!("fake-{on}"), to);
    rig.seat_rows("spawned.0", "scout", on, "claude");
    let pane = rig.new_pane("spawned.0", "scout");
    rig.start(&pane, "spawned.0", "claude");
    rig.mark_limited(&pane);
    journal(&rig, "limit", None);
    journal(&rig, ATTEMPT_ACTION, Some(KEY));
    (rig, pane)
}

/// Rewrite the global config's auto reseat switch and its one map row.
fn configure(rig: &Rig, switch: &str, to: &str) {
    configure_from(rig, switch, "fake-claude", to);
}

fn configure_from(rig: &Rig, switch: &str, from: &str, to: &str) {
    let path = rig.scratch.join("config");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let base = text.split("auto_reseat = ").next().unwrap_or_default();
    assert!(
        std::fs::write(
            &path,
            format!("{base}auto_reseat = {switch}\n[auto_reseat]\n{from} = {to}\n"),
        )
        .is_ok(),
        "a config"
    );
}

/// Keep a no-room fixture's declared candidates as its whole family universe.
fn remove_unused_profiles(rig: &Rig, profiles: &[&str]) {
    let path = rig.scratch.join("config");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|why| panic!("fixture config: {why}"));
    let mut in_profiles = false;
    let mut kept = String::new();
    for line in text.lines() {
        if line.starts_with('[') {
            in_profiles = line == "[profiles]";
        }
        let unused = in_profiles
            && profiles
                .iter()
                .any(|profile| line.starts_with(&format!("{profile} =")));
        if !unused {
            assert!(writeln!(kept, "{line}").is_ok(), "fixture config line");
        }
    }
    assert!(
        std::fs::write(path, kept).is_ok(),
        "fixture candidate universe"
    );
}

/// The claude account at the scratch's `home`, as its own quota cache reports
/// it NOW: its one window spent.
fn spend(rig: &Rig, home: &str) {
    spend_at(rig, home, 100);
}

/// The same account with its one window at `percent`.
fn spend_at(rig: &Rig, home: &str, percent: u8) {
    let dir = rig.scratch.join(home);
    let now = Timestamp::now().epoch();
    let cache = format!(
        "{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":{},\"utilization\":{{\"limits\":[{{\"kind\":\"session\",\"group\":\"session\",\"percent\":{percent},\"resets_at\":\"{}\",\"scope\":null}}]}}}}}}\n",
        now * 1_000,
        Timestamp::from_epoch(now + 7_200),
    );
    assert!(std::fs::create_dir_all(&dir).is_ok(), "an account home");
    assert!(
        std::fs::write(dir.join(".claude.json"), cache).is_ok(),
        "a spent account"
    );
}

/// Replace the meta's `key` row with `value`. REPLACED, never appended: ae
/// reads a key that appears twice as no value at all.
fn rebind(rig: &Rig, key: &str, value: &str) {
    let path = rig.dir.join("meta");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let prefix = format!("{key}=");
    assert!(
        text.lines().any(|line| line.starts_with(&prefix)),
        "a {key} row: {text}"
    );
    let text: String = text
        .lines()
        .map(|line| {
            if line.starts_with(&prefix) {
                format!("{prefix}{value}\n")
            } else {
                format!("{line}\n")
            }
        })
        .collect();
    assert!(std::fs::write(&path, text).is_ok(), "the {key} row");
}

/// Append one watchdog record addressed to the seat, stamped [`KEY`].
fn journal(rig: &Rig, action: &str, reference: Option<&str>) {
    journal_at(rig, KEY, action, reference);
}

/// The same, stamped `ts`.
fn journal_at(rig: &Rig, ts: &str, action: &str, reference: Option<&str>) {
    let reference = reference
        .map(|value| format!(r#","ref":"{value}""#))
        .unwrap_or_default();
    let line = format!(
        r#"{{"ts":"{ts}","actor":"watchdog","action":"{action}","target":"scout","target_slot":"spawned.0","target_session":"{}"{reference}}}"#,
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

/// Run the leg on the seat at `slot`, handed `key`, from the lead's pane and
/// with the scratch's own HOME, so no quota it reads is a real account.
fn leg(rig: &Rig, slot: &str, key: &str) -> (Option<i32>, String, String) {
    let out = super::cli::ae()
        .env("TMUX", format!("{},0,0", rig.sock.display()))
        .env("TMUX_PANE", &rig.main_pane)
        .env("AE_HOME", &rig.scratch)
        .env("HOME", rig.scratch.join("home"))
        .env_remove("AE_SENDER_OVERRIDE")
        .arg("_auto-reseat")
        .arg(&rig.dir)
        .args([slot, key])
        .output()
        .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The session's real `send` link, which every notice is delivered through.
fn link_send(rig: &Rig) {
    let send = rig.dir.join("send");
    if !send.exists() {
        assert!(
            std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ae"), &send).is_ok(),
            "the send link"
        );
    }
}

/// The watchdog's target-less `chat` records: what the human's chat was told.
fn said(rig: &Rig) -> Vec<Event> {
    records(rig)
        .into_iter()
        .filter(|event| event.action == "chat")
        .inspect(|event| assert_eq!((event.actor.as_str(), &event.target), ("watchdog", &None)))
        .collect()
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
    assert!(said(&rig).is_empty(), "no open attempt, nothing said");

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
    assert!(said(&rig).is_empty(), "no open attempt, nothing said");

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
    assert_eq!(said(&rig).len(), 1, "the hold is said once");
}

#[test]
fn a_seat_on_its_limit_moves_to_its_declared_candidate_and_the_watchdog_owns_every_record() {
    let (rig, pane) = limited("legmove", "on", "fake-opencode");
    // The lead on the moving seat's OLD tool, so a notice that read another
    // seat's binary as the successor's would say so.
    rebind(&rig, "profile.main", "fake-claude");
    rebind(&rig, "agent_bin.main", "claude");

    let (code, out, err) = leg(&rig, "spawned.0", KEY);

    assert_eq!(
        code,
        Some(0),
        "out={out} err={err}\nframe was:\n{}",
        rig.capture(&pane)
    );
    assert!(rig.tool_pid(&pane, "opencode").is_some(), "moved in place");
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-opencode");
    // The profile moved onto has no row and no family, so a HUMAN choosing it
    // would be nudged. The watchdog's own move prints no such line: the nudge
    // is for the moment of a human's choice.
    assert!(
        !out.contains("warning") && !err.contains("warning"),
        "a watchdog-driven reseat is silent about cover: out={out} err={err}"
    );
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
    let chat = said(&rig).remove(0).summary.unwrap_or_default();
    assert!(
        chat.starts_with(
            "auto reseat: scout moved fake-claude -> fake-opencode (tool claude -> opencode, model "
        ),
        "the successor's own tool: {chat}"
    );
    assert!(
        chat.contains("), seeded; work tree "),
        "a target with no quota reading is not called critical: {chat}"
    );
}

/// A move onto an account already judged critical, and still usable, says so.
/// The seat records no conversation, so the move between the two accounts is
/// seeded rather than carried.
#[test]
fn a_move_onto_a_critical_account_is_said_to_be_one() {
    let (rig, _pane) = limited_on("legcrit", "claude-b", "on", "fake-claude-a");
    spend_at(&rig, "home-a", 97);

    let (code, out, err) = leg(&rig, "spawned.0", KEY);

    assert_eq!(code, Some(0), "out={out} err={err}\n{}", rig.events());
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-claude-a");
    let chat = said(&rig).remove(0).summary.unwrap_or_default();
    assert!(
        chat.starts_with(
            "auto reseat: scout moved fake-claude-b -> fake-claude-a (tool claude -> claude"
        ),
        "{chat}"
    );
    assert!(
        chat.contains("), seeded, target already critical; work tree "),
        "{chat}"
    );
}

/// The colead of a lead-pair session is told of a move; a worker of any other
/// layout is not.
#[test]
fn a_move_is_told_to_the_colead_only_in_a_lead_pair_session() {
    for (tag, layout, told) in [("legpair", "lead-pair", 1), ("legvert", "vertical", 0)] {
        let (rig, _pane) = limited(tag, "on", "fake-opencode");
        link_send(&rig);
        rebind(&rig, "layout", layout);
        rig.seat("worker.0", "colead", "opencode");

        let (code, out, err) = leg(&rig, "spawned.0", KEY);

        assert_eq!(code, Some(0), "{tag}: out={out} err={err}");
        let events = records(&rig);
        let to_colead = with_action(&events, "auto-reseat-notice")
            .into_iter()
            .filter(|event| event.target.as_deref() == Some("colead"))
            .count();
        assert_eq!(to_colead, told, "{tag}: {}", rig.events());
    }
}

/// A move is said once and told to the lead, as the watchdog and after its
/// outcome; the seat that moved is not told of its own move.
#[test]
fn a_move_is_said_once_and_told_to_the_lead_after_its_outcome() {
    let (rig, _pane) = limited("legtell", "on", "fake-opencode");
    link_send(&rig);
    rig.start(&rig.main_pane.clone(), "main", "opencode");

    let (code, out, err) = leg(&rig, "spawned.0", KEY);

    assert_eq!(code, Some(0), "out={out} err={err}");
    let events = records(&rig);
    let tail: Vec<(&str, &str, Option<&str>)> = events
        .iter()
        .skip_while(|event| event.action != DONE_ACTION)
        .map(|event| {
            (
                event.action.as_str(),
                event.actor.as_str(),
                event.target.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        tail,
        [
            (DONE_ACTION, "watchdog", Some("scout")),
            ("chat", "watchdog", None),
            ("auto-reseat-notice", "watchdog", Some("lead")),
        ],
        "{}",
        rig.events()
    );
    let chat = said(&rig).remove(0).summary.unwrap_or_default();
    assert!(
        chat.starts_with(
            "auto reseat: scout moved fake-claude -> fake-opencode (tool claude -> opencode"
        ),
        "{chat}"
    );
    assert!(
        rig.received().contains(&chat),
        "the lead is told the same line: {}",
        rig.received()
    );
}

/// A candidate whose account its own cache reports spent is passed over by
/// name, and the next declared candidate takes the seat. The seat runs on a
/// scratch account, so no move this row could make reaches a real one.
#[test]
fn a_candidate_on_a_spent_account_is_passed_over_for_the_next_declared_one() {
    for (tag, to) in [
        ("legspent", "fake-claude-a, fake-opencode"),
        ("legspentall", "fake-claude-a"),
    ] {
        let (rig, pane) = limited_on(tag, "claude-b", "on", to);
        remove_unused_profiles(&rig, &["fake-claude"]);
        spend(&rig, "home-a");

        let (code, out, err) = leg(&rig, "spawned.0", KEY);

        if to.ends_with("fake-opencode") {
            assert_eq!(code, Some(0), "{tag}: out={out} err={err}");
            assert_eq!(rig.meta_row("profile.spawned.0"), "fake-opencode");
        } else {
            assert_eq!(code, Some(1), "{tag}: out={out} err={err}");
            let refused = one_outcome(&rig, REFUSED_ACTION);
            assert_eq!(
                refused.summary.as_deref(),
                Some("no-candidate: refused: no usable candidate: fake-claude-a (exhausted)"),
                "{refused:?}"
            );
            assert!(rig.tool_pid(&pane, "claude").is_some(), "still running");
            assert_eq!(rig.meta_row("profile.spawned.0"), "fake-claude-b");
        }
    }
}

#[test]
fn a_seat_with_no_usable_candidate_is_refused_where_it_stands() {
    let (rig, pane) = limited("legnone", "on", "ghost");
    remove_unused_profiles(&rig, &["fake-claude-a", "fake-claude-b"]);

    let (code, out, err) = leg(&rig, "spawned.0", KEY);

    assert_eq!(code, Some(1), "out={out} err={err}");
    let refused = one_outcome(&rig, REFUSED_ACTION);
    assert_eq!(refused.reference.as_deref(), Some(KEY), "{refused:?}");
    assert_eq!(
        refused.summary.as_deref(),
        Some("no-candidate: refused: no usable candidate: ghost (not configured here)"),
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
            .is_some_and(|summary| summary.starts_with("ineligible: held: ")),
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

/// How long one live daemon run may take before the pin fails rather than hangs.
const BUDGET: Duration = Duration::from_mins(1);

/// The daemon's line for a trigger that did not start.
const NOT_STARTED: &str = "ae: watchdog: auto reseat of scout not started this cycle";

/// The global config that turns the path on with no grace, so a seat is due at
/// its limit's first sight.
const ON_NOW: &str = "on\nauto_reseat_grace_secs = 0";

/// Run the session's REAL watchdog until `done` holds, delivering through the
/// session's real `send` link. Its HOME is the scratch's own, so nothing it
/// reads is a real account.
fn watch_until(rig: &Rig, done: impl Fn() -> bool) -> bool {
    link_send(rig);
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(rig.scratch.join("daemon-err"));
    let mut runner = super::cli::ae();
    runner
        .arg("_watchdog-run")
        .arg(&rig.dir)
        .args(["--interval", "1", "--stale-secs", "999999"])
        .args(["--tg-supervise-secs", "0"])
        .env("HOME", rig.scratch.join("home"))
        .env("AE_HOME", &rig.scratch)
        .env("CONFIG_FILE", rig.scratch.join("config"))
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("AE_SENDER_OVERRIDE")
        .stdout(std::process::Stdio::null());
    if let Ok(log) = log {
        runner.stderr(log);
    }
    let _daemon = runner
        .spawn()
        .unwrap_or_else(|why| panic!("the daemon should start: {why}"));
    let deadline = Instant::now() + BUDGET;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// What the daemons said on their error stream, for a failure message.
fn daemon_err(rig: &Rig) -> String {
    std::fs::read_to_string(rig.scratch.join("daemon-err")).unwrap_or_default()
}

/// The seat's own records, in journal order.
fn seat_records(rig: &Rig) -> Vec<Event> {
    records(rig)
        .into_iter()
        .filter(|event| {
            ae::watchdog::event_is_addressed_to(event, &rig.session, "spawned.0", "scout")
        })
        .collect()
}

/// How many `action` records the seat has.
fn booked(rig: &Rig, action: &str) -> usize {
    with_action(&seat_records(rig), action).len()
}

/// The seat's records of the auto path, as `(action, ref)`.
fn auto_path(rig: &Rig) -> Vec<(String, String)> {
    seat_records(rig)
        .into_iter()
        .filter(|event| event.action.starts_with(ATTEMPT_ACTION))
        .map(|event| (event.action, event.reference.unwrap_or_default()))
        .collect()
}

/// THE PATH END TO END: the watchdog books the seat's limit and triggers
/// through the one `send`; the trigger opens ONE attempt and starts the leg;
/// the leg moves the seat. The shell the move leaves is no death and no release.
#[test]
fn a_seat_on_its_limit_is_moved_once_by_the_watchdog_with_no_death_between() {
    let rig = Rig::new("automove");
    configure(&rig, ON_NOW, "fake-opencode");
    let pane = rig.seat("spawned.0", "scout", "claude");
    rig.mark_limited(&pane);

    let moved = watch_until(&rig, || booked(&rig, DONE_ACTION) == 1);

    assert!(moved, "no move: {}\n{}", rig.events(), daemon_err(&rig));
    assert!(
        !daemon_err(&rig).contains(NOT_STARTED),
        "{}",
        daemon_err(&rig)
    );
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-opencode");
    assert!(rig.tool_pid(&pane, "opencode").is_some(), "moved in place");
    let ours = seat_records(&rig);
    let at = |action: &str| ours.iter().position(|event| event.action == action);
    let (Some(limit), Some(attempt), Some(done)) =
        (at("limit"), at(ATTEMPT_ACTION), at(DONE_ACTION))
    else {
        panic!("limit, attempt and move: {}", rig.events());
    };
    assert!(limit < attempt && attempt < done, "{}", rig.events());
    assert_eq!(booked(&rig, ATTEMPT_ACTION), 1, "one attempt");
    assert_eq!(ours[attempt].actor, "watchdog");
    assert_eq!(
        ours[attempt].reference.as_deref(),
        Some(ours[limit].ts.to_string().as_str()),
        "the attempt names its episode"
    );
    assert_eq!(
        ours[attempt].summary.as_deref(),
        Some("from fake-claude to fake-opencode")
    );
    let between: Vec<&str> = ours[attempt..done]
        .iter()
        .map(|event| event.action.as_str())
        .collect();
    assert!(
        !between.contains(&"alert") && !between.contains(&"alert-cleared"),
        "{between:?}"
    );
}

/// Attach one real client to the rig's session, from a pane of a second
/// session on the same private server: a notice is drawn only for an attached
/// client, and the server's message log is where the pin reads it back.
fn attach_viewer(rig: &Rig) {
    let attach = format!(
        "env -u TMUX tmux -S '{}' attach-session -t '={}'",
        rig.sock.display(),
        rig.session
    );
    assert!(
        rig.tmux(&[
            "new-session",
            "-d",
            "-s",
            "viewer",
            "-x",
            "200",
            "-y",
            "40",
            &attach
        ])
        .0,
        "a viewer session"
    );
    let target = format!("={}", rig.session);
    let deadline = Instant::now() + BUDGET;
    while Instant::now() < deadline {
        let (_, clients) = rig.tmux(&["list-clients", "-t", &target, "-F", "#{client_tty}"]);
        if !clients.trim().is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("a client should attach to {}", rig.session);
}

/// The watchdog's limit notices the attached client was shown, one line each.
/// The log also holds each `display-message` command; only a shown message
/// counts.
fn limit_notices(rig: &Rig) -> Vec<String> {
    rig.tmux(&["show-messages"])
        .1
        .lines()
        .filter(|line| {
            line.contains(" message: [ae watchdog] ") && line.contains("hit its vendor usage limit")
        })
        .map(str::to_owned)
        .collect()
}

/// The limit's first-sight notice forecasts the move for a seat the path would
/// move, and for that seat alone.
#[test]
fn the_first_sight_limit_notice_forecasts_the_move_of_a_seat_the_path_would_move() {
    let rig = Rig::new("autoforecast");
    configure(&rig, "on", "fake-opencode");
    // The other seat has no declared row or family: effort differs from scout.
    let path = rig.scratch.join("config");
    let text = std::fs::read_to_string(&path).expect("forecast fixture config");
    let distinct = format!(
        "fake-claude-b = \"CLAUDE_CONFIG_DIR={} {} {} --effort high\"",
        rig.scratch.join("home-b").display(),
        rig.scratch.join("tools/claude").display(),
        rig.scratch.join("claude.pl").display(),
    );
    let text = text
        .lines()
        .map(|line| {
            if line.starts_with("fake-claude-b =") {
                distinct.as_str()
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(path, format!("{text}\n")).expect("distinct forecast profile");
    attach_viewer(&rig);
    let scout = rig.seat("spawned.0", "scout", "claude");
    rig.seat_rows("spawned.1", "other", "claude-b", "claude");
    let other = rig.new_pane("spawned.1", "other");
    rig.start(&other, "spawned.1", "claude");
    rig.mark_limited(&scout);

    let shown = watch_until(&rig, || limit_notices(&rig).len() >= 2);

    let notices = limit_notices(&rig);
    assert!(shown, "{notices:?}\n{}\n{}", rig.events(), daemon_err(&rig));
    let of = |agent: &str| -> Vec<&String> {
        notices
            .iter()
            .filter(|line| line.contains(&format!("[ae watchdog] {agent} hit")))
            .collect()
    };
    let (scouts, others) = (of("scout"), of("other"));
    assert_eq!((scouts.len(), others.len()), (1, 1), "{notices:?}");
    assert!(
        scouts[0].ends_with("; ae will move scout to fake-opencode in 10m"),
        "{notices:?}"
    );
    assert!(
        !others[0].contains("ae will move") && !others[0].contains("ae cannot move"),
        "{notices:?}"
    );
}

/// OFF IS INERT: with the switch absent, and then written `off`, the watchdog
/// books and releases a limit exactly as it always has, and nothing of the
/// auto path runs — no record, no lock, no move.
#[test]
fn with_the_switch_absent_or_off_a_limit_is_booked_and_released_as_it_always_was() {
    let rig = Rig::new("autooff");
    let pane = rig.seat("spawned.0", "scout", "claude");
    for (round, switch) in [(1, None), (2, Some("off\nauto_reseat_grace_secs = 0"))] {
        if let Some(switch) = switch {
            configure(&rig, switch, "fake-opencode");
        }
        rig.mark_limited(&pane);
        let since = Instant::now();
        let shown = std::cell::Cell::new(true);
        let released = watch_until(&rig, || {
            if shown.get()
                && booked(&rig, "limit") == round
                && since.elapsed() > Duration::from_secs(4)
            {
                rig.unmark_limited(&pane);
                shown.set(false);
            }
            booked(&rig, "alert-cleared") == round
        });
        assert!(
            released,
            "round {round}: {}\n{}",
            rig.events(),
            daemon_err(&rig)
        );
    }
    assert!(auto_path(&rig).is_empty(), "{}", rig.events());
    assert!(
        !rig.dir.join("auto-reseat.spawned.0.lock").exists(),
        "no writer of the path ran"
    );
    untouched(&rig, &pane);
}

/// A refusal ends the episode's auto path for good: a second daemon on the
/// same journal books the limit again, and attempts and refuses nothing more.
#[test]
fn a_refused_episode_stays_refused_across_a_daemon_restart() {
    let rig = Rig::new("autorefuse");
    configure(&rig, ON_NOW, "ghost");
    remove_unused_profiles(&rig, &["fake-claude-a", "fake-claude-b"]);
    let pane = rig.seat("spawned.0", "scout", "claude");
    rig.mark_limited(&pane);
    let refused = watch_until(&rig, || booked(&rig, REFUSED_ACTION) == 1);
    assert!(refused, "{}\n{}", rig.events(), daemon_err(&rig));

    let since = Instant::now();
    let rebooked = watch_until(&rig, || {
        booked(&rig, "limit") == 2 && since.elapsed() > Duration::from_secs(4)
    });

    assert!(rebooked, "{}\n{}", rig.events(), daemon_err(&rig));
    let key = seat_records(&rig)
        .iter()
        .find(|event| event.action == "limit")
        .map(|event| event.ts.to_string())
        .unwrap_or_default();
    assert_eq!(
        auto_path(&rig),
        [(REFUSED_ACTION.to_owned(), key)],
        "no attempt, one refusal"
    );
    untouched(&rig, &pane);
    let chats = said(&rig);
    assert_eq!(chats.len(), 1, "the refusal is said once: {}", rig.events());
    assert!(
        chats[0]
            .summary
            .as_deref()
            .is_some_and(|line| line.starts_with(
                "auto reseat: no-candidate: scout not moved — refused: no usable candidate: ghost"
            )),
        "{chats:?}"
    );
}

/// An attempt in flight belongs to its leg, even across a daemon restart: the
/// shell its respawn leaves is neither a death nor a release, and nothing is
/// attempted or held beside it.
#[test]
fn an_attempt_in_flight_survives_a_daemon_restart_with_no_death_and_no_repeat() {
    let rig = Rig::new("autoflight");
    configure(&rig, ON_NOW, "fake-opencode");
    let pane = rig.seat("spawned.0", "scout", "claude");
    let key = Timestamp::now().to_string();
    journal_at(&rig, &key, "limit", None);
    journal_at(&rig, &key, ATTEMPT_ACTION, Some(&key));
    rig.kill_tools(&pane);

    let since = Instant::now();
    let watched = watch_until(&rig, || since.elapsed() > Duration::from_secs(5));

    assert!(watched);
    assert_eq!(booked(&rig, "alert"), 0, "no death: {}", rig.events());
    assert_eq!(booked(&rig, "alert-cleared"), 0, "no release");
    assert_eq!(booked(&rig, "limit"), 2, "the limit latched again");
    assert_eq!(auto_path(&rig), [(ATTEMPT_ACTION.to_owned(), key)]);
}

/// A forged trigger does only what the watchdog would do now: a seat not on
/// its limit, or a switch that is off, declines, and nothing is written, sent
/// or moved.
#[test]
fn a_forged_trigger_moves_nothing_the_watchdog_would_not() {
    let rig = Rig::new("autoforge");
    configure(&rig, ON_NOW, "fake-opencode");
    let pane = rig.seat("spawned.0", "scout", "claude");
    let forge = || {
        let at = format!("@{}", rig.session);
        let out = super::cli::ae()
            .args([at.as_str(), "send", "scout", "auto reseat"])
            .env("TMUX", format!("{},0,0", rig.sock.display()))
            .env("TMUX_PANE", &rig.main_pane)
            .env("AE_HOME", &rig.scratch)
            .env("AE_SENDER_OVERRIDE", "watchdog")
            .env("_AE_EVENT_ACTION", ATTEMPT_ACTION)
            .output()
            .expect("the ae binary runs");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let before = rig.events();
    let (code, err) = forge();
    assert_eq!(code, Some(1), "{err}");
    assert_eq!(rig.events(), before, "not on its limit: nothing written");

    rig.mark_limited(&pane);
    journal(&rig, "limit", None);
    configure(&rig, "off", "fake-opencode");
    let before = rig.events();
    let (code, err) = forge();
    assert_eq!(code, Some(1), "{err}");
    assert_eq!(rig.events(), before, "switched off: nothing written");
    assert!(!rig.received().contains("auto reseat"), "nothing sent");
    untouched(&rig, &pane);
}

/// The seat's lock, which every writer of the auto path takes first.
fn hold_seat_lock(rig: &Rig) -> std::fs::File {
    let lock = rig.dir.join("auto-reseat.spawned.0.lock");
    ae::store::lock(&lock, Duration::ZERO)
        .unwrap_or_else(|why| panic!("the test should hold the seat's lock: {why}"))
}

/// The trigger writes nothing while another writer holds the seat, and does
/// exactly what the watchdog would do once it is free: one attempt, one move.
#[test]
fn a_trigger_backs_off_while_another_writer_holds_the_seat() {
    let rig = Rig::new("autotriglock");
    configure(&rig, ON_NOW, "fake-opencode");
    let pane = rig.seat("spawned.0", "scout", "claude");
    rig.mark_limited(&pane);
    journal(&rig, "limit", None);
    let trigger = || {
        let at = format!("@{}", rig.session);
        let out = super::cli::ae()
            .args([at.as_str(), "send", "scout", "auto reseat"])
            .env("AE_HOME", &rig.scratch)
            .env("AE_SENDER_OVERRIDE", "watchdog")
            .env("_AE_EVENT_ACTION", ATTEMPT_ACTION)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .output()
            .expect("the ae binary runs");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let held = hold_seat_lock(&rig);
    let before = rig.events();
    let (code, err) = trigger();
    assert_eq!(code, Some(1), "{err}");
    assert_eq!(rig.events(), before, "held: nothing written");
    untouched(&rig, &pane);

    drop(held);
    let (code, err) = trigger();
    assert_eq!(code, Some(0), "{err}");
    let attempt = one_outcome(&rig, ATTEMPT_ACTION);
    assert_eq!(attempt.reference.as_deref(), Some(KEY), "{attempt:?}");
    assert_eq!(
        attempt.summary.as_deref(),
        Some("from fake-claude to fake-opencode")
    );
    let deadline = Instant::now() + BUDGET;
    while booked(&rig, DONE_ACTION) == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(
        rig.meta_row("profile.spawned.0"),
        "fake-opencode",
        "{}",
        rig.events()
    );
}

/// A daemon writer writes nothing while another writer holds the seat, and
/// closes an attempt past its bound once it is free.
#[test]
fn a_daemon_writer_backs_off_while_another_writer_holds_the_seat() {
    let rig = Rig::new("autodaemonlock");
    configure(&rig, ON_NOW, "fake-opencode");
    let pane = rig.seat("spawned.0", "scout", "claude");
    rig.mark_limited(&pane);
    let key = Timestamp::from_epoch(Timestamp::now().epoch() - 300).to_string();
    let opened = Timestamp::from_epoch(Timestamp::now().epoch() - 200).to_string();
    journal_at(&rig, &key, "limit", None);
    journal_at(&rig, &opened, ATTEMPT_ACTION, Some(&key));

    let held = hold_seat_lock(&rig);
    let since = Instant::now();
    assert!(watch_until(&rig, || since.elapsed() > Duration::from_secs(4)));
    assert_eq!(booked(&rig, FAILED_ACTION), 0, "held: {}", rig.events());

    drop(held);
    let closed = watch_until(&rig, || booked(&rig, FAILED_ACTION) == 1);
    assert!(closed, "{}\n{}", rig.events(), daemon_err(&rig));
    let failed = one_outcome(&rig, FAILED_ACTION);
    assert_eq!(
        failed.reference.as_deref(),
        Some(key.as_str()),
        "{failed:?}"
    );
    assert_eq!(
        failed.summary.as_deref(),
        Some("other: failed: no outcome recorded")
    );
    untouched(&rig, &pane);
}

/// A trigger the seat refuses is said on the daemon's error stream and opens
/// nothing; the next cycle asks again, and the seat moves once it is free.
#[test]
fn a_trigger_the_seat_refuses_is_said_and_asked_again_until_it_moves() {
    let rig = Rig::new("autodaemontrig");
    configure(&rig, ON_NOW, "fake-opencode");
    let pane = rig.seat("spawned.0", "scout", "claude");
    rig.mark_limited(&pane);

    let held = hold_seat_lock(&rig);
    let said = watch_until(&rig, || daemon_err(&rig).contains(NOT_STARTED));
    assert!(said, "{}\n{}", rig.events(), daemon_err(&rig));
    assert_eq!(booked(&rig, ATTEMPT_ACTION), 0, "held: {}", rig.events());
    untouched(&rig, &pane);

    drop(held);
    let moved = watch_until(&rig, || booked(&rig, DONE_ACTION) == 1);
    assert!(moved, "{}\n{}", rig.events(), daemon_err(&rig));
    assert_eq!(booked(&rig, ATTEMPT_ACTION), 1, "one attempt");
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-opencode");
}

// Frozen limitstop acceptance: real 2.1.296 limit furniture, stop safety,
// actionable refusal, and the daemon's previously silent no-candidate path.
const LIMITSTOP_FRAME: &str =
    include_str!("../fixtures/harness-state/claude-limit-2.1.296-180x50.txt");

/// Playback replaces only the external harness; the stop, process proof,
/// journal, watchdog and notice delivery remain the product's real path.
const LIMITSTOP_PLAYBACK: &str = r#"#!/usr/bin/perl
use strict;
use warnings;
system("stty raw -echo 2>/dev/null");
binmode(STDIN, ':raw');
binmode(STDOUT, ':raw');
$| = 1;
my $last = '';
while (1) {
    my $state = -e "__CLEAR__" ? 'clear' : 'limit';
    if ($last ne $state) {
        print "\e[H\e[2J";
        if ($state eq 'clear') {
            print "\r\n" x 50;
            print "Usage restored\r\n";
        } else {
            open(my $frame, '<', "__FRAME__") or die;
            while (my $row = <$frame>) { $row =~ s/\n\z//; print "$row\r\n"; }
            close($frame);
        }
        open(my $drawn, '>', "__DRAWN__.$$") or die;
        print $drawn $state eq 'limit' ? "__MARK__" : "done (0s)";
        close($drawn);
        $last = $state;
    }
    my $ready = '';
    vec($ready, fileno(STDIN), 1) = 1;
    if (select($ready, undef, undef, 0.05) > 0) {
        my $chunk = '';
        sysread(STDIN, $chunk, 4096);
        open(my $received, '>>', "__RECEIVED__") or die;
        print $received $chunk;
        close($received);
    }
}
"#;

fn limitstop_playback(rig: &Rig, tool: &str, frame: &str) -> String {
    limitstop_playback_as(rig, tool, frame, "scout")
}

fn limitstop_playback_as(rig: &Rig, tool: &str, frame: &str, agent: &str) -> String {
    let frame_path = rig.scratch.join("limitstop-frame.txt");
    assert!(std::fs::write(&frame_path, frame).is_ok(), "playback frame");
    // The marker is a string the drawn frame contains: the product's own words
    // where the frame has them, else the menu's title.
    let mark = if frame.contains("Usage limit reached") {
        "Usage limit reached"
    } else {
        "What do you want to do?"
    };
    let body = LIMITSTOP_PLAYBACK
        .replace("__FRAME__", &frame_path.display().to_string())
        .replace("__MARK__", mark)
        .replace(
            "__CLEAR__",
            &rig.scratch.join("limitstop-clear").display().to_string(),
        )
        .replace(
            "__DRAWN__",
            &rig.scratch.join("drawn").display().to_string(),
        )
        .replace(
            "__RECEIVED__",
            &rig.scratch.join("received").display().to_string(),
        );
    assert!(
        std::fs::write(rig.scratch.join(format!("{tool}.pl")), body).is_ok(),
        "playback harness"
    );
    rig.seat_rows("spawned.0", agent, tool, tool);
    let pane = rig.new_pane("spawned.0", agent);
    assert!(
        rig.tmux(&["resize-window", "-t", &pane, "-x", "200", "-y", "60"])
            .0,
        "the entire frozen frame fits"
    );
    rig.start(&pane, "spawned.0", tool);
    pane
}

fn limitstop_attempt(tag: &str, frame: &str) -> (Rig, String) {
    let rig = Rig::new(tag);
    configure(&rig, ON_NOW, "fake-opencode");
    let pane = limitstop_playback(&rig, "claude", frame);
    journal(&rig, "limit", None);
    journal(&rig, ATTEMPT_ACTION, Some(KEY));
    (rig, pane)
}

#[test]
fn limitstop_the_real_limit_frame_is_stopped_by_the_limit_leg() {
    let (rig, pane) = limitstop_attempt("limitstopreal", LIMITSTOP_FRAME);
    let pid = rig.tool_pid(&pane, "claude").expect("the limited tool");
    let (code, out, err) = leg(&rig, "spawned.0", KEY);
    assert_eq!(
        code,
        Some(0),
        "the real terminal limit frame must move under proven-stop policy: out={out} err={err}"
    );
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-opencode");
    assert!(rig.tool_pid(&pane, "claude").is_none(), "old tool stopped");
    assert!(rig.tool_pid(&pane, "opencode").is_some(), "replacement up");
    assert_ne!(rig.tool_pid(&pane, "opencode"), Some(pid));
    assert_eq!(
        one_outcome(&rig, DONE_ACTION).reference.as_deref(),
        Some("fake-claude")
    );
}

/// Claude's `/rate-limit-options` dialog, measured on a live seat at its
/// weekly limit: the box is replaced by the menu, and no `⎿` row is drawn.
const LIMIT_MENU_FRAME: &str =
    include_str!("../fixtures/claude-limit-menu/claude-limit-menu-39col.txt");

/// The menu is the limit over a dialog instead of a box: the leg's stop reads
/// it as the proof, on the real stop path.
#[test]
fn limitmenu_the_measured_menu_is_stopped_by_the_limit_leg() {
    let (rig, pane) = limitstop_attempt("limitmenustop", LIMIT_MENU_FRAME);
    let pid = rig.tool_pid(&pane, "claude").expect("the limited tool");
    assert_eq!(
        ae::harness_state::classify(&rig.capture(&pane), ae::tool::ToolKind::Claude),
        ae::harness_state::HarnessState::Unknown,
        "the dialog draws no box the frame grammar reads"
    );
    let (code, out, err) = leg(&rig, "spawned.0", KEY);
    assert_eq!(code, Some(0), "the menu must move: out={out} err={err}");
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-opencode");
    assert!(rig.tool_pid(&pane, "claude").is_none(), "old tool stopped");
    assert_ne!(rig.tool_pid(&pane, "opencode"), Some(pid));
    assert_eq!(
        one_outcome(&rig, DONE_ACTION).reference.as_deref(),
        Some("fake-claude")
    );
}

/// A running turn drawn above the dialog is a limit still, but no stop proof:
/// the leg refuses and the seat stays.
#[test]
fn limitmenu_a_running_turn_above_the_menu_cannot_be_stopped() {
    let busy = include_str!("../fixtures/harness-state/claude-busy-167x40.txt")
        .lines()
        .find(|row| row.starts_with("✻ Enchanting"))
        .expect("measured busy row");
    let frame = LIMIT_MENU_FRAME.replace("✻ Sautéed for 6s · done Tuesday 3:03 PM", busy);
    let (rig, pane) = limitstop_attempt("limitmenubusy", &frame);
    assert!(
        ae::watchdog::limit_notice(&rig.capture(&pane), "claude").is_some(),
        "the menu is still the limit"
    );
    let (code, out, err) = leg(&rig, "spawned.0", KEY);
    assert_eq!(code, Some(1), "no proof, no stop: out={out} err={err}");
    one_outcome(&rig, REFUSED_ACTION);
    untouched(&rig, &pane);
}

/// THE PATH END TO END on the menu: the watchdog books the limit from the
/// capture ONCE, and the leg moves the seat, with no `⎿` row ever drawn.
#[test]
fn a_seat_on_the_limit_menu_is_booked_once_and_moved_by_the_watchdog() {
    let rig = Rig::new("automenu");
    configure(&rig, ON_NOW, "fake-opencode");
    let pane = limitstop_playback(&rig, "claude", LIMIT_MENU_FRAME);

    let moved = watch_until(&rig, || booked(&rig, DONE_ACTION) == 1);

    assert!(moved, "no move: {}\n{}", rig.events(), daemon_err(&rig));
    assert_eq!(booked(&rig, "limit"), 1, "{}", rig.events());
    assert_eq!(booked(&rig, ATTEMPT_ACTION), 1, "one attempt");
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-opencode");
    assert!(rig.tool_pid(&pane, "opencode").is_some(), "moved in place");
    let limit = seat_records(&rig)
        .into_iter()
        .find(|event| event.action == "limit")
        .and_then(|event| event.summary)
        .unwrap_or_default();
    assert!(
        limit.contains("1. Stop and wait for limit to reset 2. Wait here"),
        "the record quotes the menu: {limit}"
    );
}

#[test]
fn limitstop_busy_wins_even_with_the_real_limit_footer_and_stop_unknown() {
    // The busy row was measured live; Busy AT a limit cannot be reproduced
    // because every limited turn ends immediately. Only this combination is synthetic.
    let busy = include_str!("../fixtures/harness-state/claude-busy-167x40.txt")
        .lines()
        .find(|row| row.starts_with("✻ Enchanting"))
        .expect("measured busy row");
    let frame = LIMITSTOP_FRAME.replace("✻ Baked for 0s · done 10:23 AM", busy);
    let (rig, pane) = limitstop_attempt("limitstopbusy", &frame);
    let before = rig.tool_pid(&pane, "claude");
    assert_eq!(
        ae::harness_state::classify(&rig.capture(&pane), ae::tool::ToolKind::Claude),
        ae::harness_state::HarnessState::Busy,
        "a limit warning cannot hide the current running turn"
    );
    let (code, out, err) = leg(&rig, "spawned.0", KEY);
    assert_eq!(code, Some(1), "out={out} err={err}");
    assert!(err.contains("BUSY"), "busy must refuse: {err}");
    one_outcome(&rig, HELD_ACTION);
    let (code, out, err) = rig.run_top(
        &rig.main_pane,
        &[
            "reseat",
            &rig.session,
            "scout",
            "--using",
            "fake-opencode",
            "--stop-unknown",
        ],
    );
    assert_eq!(code, Some(1), "out={out} err={err}");
    assert!(err.contains("BUSY"), "flag never lifts Busy: {err}");
    assert_eq!(rig.tool_pid(&pane, "claude"), before);
    untouched(&rig, &pane);
}

#[test]
fn limitstop_a_stale_transcript_limit_above_an_unproven_turn_cannot_stop_it() {
    let frame: String = LIMITSTOP_FRAME
        .replace(
            "✻ Baked for 0s · done 10:23 AM",
            "⏺ Running a tool after the earlier limit",
        )
        .lines()
        .filter(|row| {
            !row.trim_start().starts_with("⚠ Usage limit reached")
                && !row.trim_start().starts_with("Continuing automatically at")
        })
        .fold(String::new(), |mut frame, row| {
            frame.push_str(row);
            frame.push('\n');
            frame
        });
    let (rig, pane) = limitstop_attempt("limitstophistory", &frame);
    let before = rig.tool_pid(&pane, "claude");
    let captured = rig.capture(&pane);
    assert!(
        ae::watchdog::limit_notice(&captured, "claude").is_some(),
        "historical vendor row still in window"
    );
    assert_eq!(
        ae::harness_state::classify(&captured, ae::tool::ToolKind::Claude),
        ae::harness_state::HarnessState::Unknown,
        "empty composer does not prove this current turn ended"
    );
    let (code, out, err) = leg(&rig, "spawned.0", KEY);
    assert_eq!(
        code,
        Some(1),
        "a transcript-only limit cannot justify stopping: out={out} err={err}"
    );
    assert_eq!(rig.tool_pid(&pane, "claude"), before, "same tool remains");
    one_outcome(&rig, REFUSED_ACTION);
    untouched(&rig, &pane);
}

#[test]
fn limitstop_an_unreadable_limit_refusal_records_and_tells_the_exact_next_command() {
    let frame = LIMITSTOP_FRAME.replace("❯\u{a0}\n", "❯ draft text\n");
    let (rig, pane) = limitstop_attempt("limitstopnext", &frame);
    link_send(&rig);
    rig.start(&rig.main_pane, "main", "opencode");
    let command = format!(
        "ae reseat {} scout --using fake-opencode --stop-unknown",
        rig.session
    );
    let (code, out, err) = leg(&rig, "spawned.0", KEY);
    assert_eq!(code, Some(1), "draft must refuse: out={out} err={err}");
    untouched(&rig, &pane);
    let refused = one_outcome(&rig, REFUSED_ACTION);
    assert!(
        refused
            .summary
            .as_deref()
            .is_some_and(|line| line.contains(&command)),
        "the refusal record must retain the selected candidate and complete command: {refused:?}"
    );
    let chats = said(&rig);
    assert_eq!(chats.len(), 1, "one human notice: {chats:?}");
    assert!(
        chats[0]
            .summary
            .as_deref()
            .is_some_and(|line| line.contains(&command)),
        "human notice must name that same exact command: {chats:?}"
    );
    let told: Vec<_> = records(&rig)
        .into_iter()
        .filter(|event| {
            event.action == "auto-reseat-notice" && event.target.as_deref() == Some("lead")
        })
        .collect();
    assert_eq!(told.len(), 1, "lead notice delivered once: {told:?}");
    assert!(
        told[0]
            .summary
            .as_deref()
            .is_some_and(|line| line.contains(&command)),
        "delivered notice retains command: {told:?}"
    );
}

#[test]
fn limitstop_max_length_names_keep_the_complete_command_and_precondition() {
    let mut rig = Rig::new("limitstopmax");
    let session = "s".repeat(128);
    let agent = "a".repeat(64);
    let profile = "p".repeat(64);
    assert!(
        rig.tmux(&["rename-session", "-t", &rig.session, &session])
            .0
    );
    let dir = rig.scratch.join("sessions").join(&session);
    assert!(std::fs::rename(&rig.dir, &dir).is_ok());
    rig.dir = dir;
    rig.session = session;
    rebind(&rig, "session", &rig.session);
    configure(&rig, ON_NOW, &profile);
    let config = rig.scratch.join("config");
    let text =
        std::fs::read_to_string(&config).unwrap_or_else(|why| panic!("fixture config: {why}"));
    let value = text
        .lines()
        .find_map(|row| row.strip_prefix("fake-opencode = "))
        .expect("the external replacement command");
    assert!(
        std::fs::write(
            &config,
            text.replace("[profiles]", &format!("[profiles]\n{profile} = {value}"))
        )
        .is_ok()
    );
    let frame = LIMITSTOP_FRAME.replace("❯\u{a0}\n", "❯ draft text\n");
    let pane = limitstop_playback_as(&rig, "claude", &frame, &agent);
    for (action, reference) in [("limit", ""), (ATTEMPT_ACTION, KEY)] {
        let event = format!(
            r#"{{"ts":"{KEY}","actor":"watchdog","action":"{action}","target":"{agent}","target_slot":"spawned.0","target_session":"{}","ref":"{reference}"}}"#,
            rig.session
        );
        let appended = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(rig.dir.join("events.jsonl"))
            .and_then(|mut file| writeln!(file, "{event}"));
        assert!(appended.is_ok(), "max-name episode");
    }
    link_send(&rig);
    rig.start(&rig.main_pane, "main", "opencode");
    let command = format!(
        "ae reseat {} {agent} --using {profile} --stop-unknown",
        rig.session
    );
    let precondition = "once its box is empty and the limit line is its newest row";
    let (code, out, err) = leg(&rig, "spawned.0", KEY);
    assert_eq!(code, Some(1), "draft must refuse: out={out} err={err}");
    untouched(&rig, &pane);
    let events = records(&rig);
    let outcomes = with_action(&events, REFUSED_ACTION);
    assert_eq!(outcomes.len(), 1, "one refused outcome: {}", rig.events());
    let summary = outcomes[0].summary.as_deref().unwrap_or_default();
    assert!(
        summary.contains(&command),
        "max-name record keeps full command: {summary}"
    );
    assert!(
        summary
            .find(precondition)
            .zip(summary.find(&command))
            .is_some_and(|(before, command)| before < command),
        "record states precondition first: {summary}"
    );
    let chats = said(&rig);
    assert_eq!(chats.len(), 1, "one human notice");
    let notice = chats[0].summary.as_deref().unwrap_or_default();
    let next = notice
        .split_once("Next:")
        .map(|(_, next)| next)
        .unwrap_or_default();
    assert!(
        next.contains(&command),
        "max-name notice Next keeps full command: {notice}"
    );
    assert!(
        next.find(precondition)
            .zip(next.find(&command))
            .is_some_and(|(before, command)| before < command),
        "Next states precondition first: {notice}"
    );
}

fn limitstop_no_candidate(tag: &str) -> (Rig, String) {
    let rig = Rig::new(tag);
    configure_from(&rig, ON_NOW, "fake-grok", "fake-opencode");
    // Muse has one shared account. With no sibling and no explicit map,
    // this profile has no candidate; this does not depend on local config.
    assert!(
        std::fs::copy(
            rig.scratch.join("tools/claude"),
            rig.scratch.join("tools/muse")
        )
        .is_ok()
    );
    let config = rig.scratch.join("config");
    let text =
        std::fs::read_to_string(&config).unwrap_or_else(|why| panic!("fixture config: {why}"));
    let row = format!(
        "fake-muse = \"{} {}\"\n[workspace]",
        rig.scratch.join("tools/muse").display(),
        rig.scratch.join("muse.pl").display()
    );
    assert!(std::fs::write(&config, text.replace("[workspace]", &row)).is_ok());
    let frame = "◆ Usage limit reached · /upgrade for increased limits\n\n✻ Fake · done (0s)\n────────────────────────────────────────────────────────────\n❯\n────────────────────────────────────────────────────────────\n  🧠 Muse synthetic status\n  ⏵⏵ accept edits on\n";
    let pane = limitstop_playback(&rig, "muse", frame);
    rebind(&rig, "layout", "lead-pair");
    rig.start(&rig.main_pane, "main", "opencode");
    rig.seat("worker.0", "colead", "opencode");
    rig.seat("spawned.1", "spawner", "opencode");
    let event = format!(
        r#"{{"ts":"{KEY}","actor":"spawner","action":"spawn","target":"scout","target_slot":"spawned.0","target_session":"{}"}}"#,
        rig.session
    );
    let mut events = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(rig.dir.join("events.jsonl"))
        .unwrap_or_else(|why| panic!("fixture journal: {why}"));
    writeln!(events, "{event}").unwrap_or_else(|why| panic!("fixture spawn record: {why}"));
    (rig, pane)
}

fn limitstop_candidate_hold(rig: &Rig) -> Vec<Event> {
    records(rig)
        .into_iter()
        .filter(|event| event.action == HELD_ACTION && event.target.as_deref() == Some("scout"))
        .collect()
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one episode pins the hold, recipient route, restart and configuration recovery"
)]
fn limitstop_an_unmapped_limit_has_one_hold_and_one_notice_then_a_map_still_moves_it() {
    let (rig, pane) = limitstop_no_candidate("limitstopunmapped");
    // Fixed observation time covers multiple daemon cycles even on the old
    // code, where no hold appears. A missing record fails an assertion.
    let started = Instant::now();
    assert!(
        watch_until(&rig, || {
            started.elapsed() >= Duration::from_secs(6)
                && (limitstop_candidate_hold(&rig).is_empty()
                    || records(&rig)
                        .iter()
                        .filter(|event| event.action == "auto-reseat-notice")
                        .count()
                        == 3)
        }),
        "the hold and its recipient deliveries must complete: {}\n{}",
        rig.events(),
        daemon_err(&rig)
    );
    let held = limitstop_candidate_hold(&rig);
    assert_eq!(
        held.len(),
        1,
        "a due unmapped limit must be explained once: {}",
        rig.events()
    );
    assert!(
        held[0]
            .summary
            .as_deref()
            .is_some_and(|line| line.contains("candidate") || line.contains("[auto_reseat]")),
        "named no-candidate reason: {held:?}"
    );
    let limits: Vec<_> = records(&rig)
        .into_iter()
        .filter(|event| event.action == "limit" && event.target.as_deref() == Some("scout"))
        .collect();
    assert_eq!(
        held[0].reference.as_deref(),
        Some(limits[0].ts.to_string().as_str()),
        "held belongs to this episode"
    );
    assert!(rig.tool_pid(&pane, "muse").is_some(), "held tool remains");
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-muse");
    let notices = said(&rig);
    assert_eq!(
        notices.len(),
        1,
        "one human notice per episode: {notices:?}"
    );
    let notice = notices[0].summary.as_deref().unwrap_or_default();
    assert!(
        notice.contains("fake-muse") && notice.contains("[auto_reseat]"),
        "notice says which row to add: {notice}"
    );
    assert!(
        notice.contains(&format!("ae reseat {} scout --using", rig.session)),
        "manual move is actionable: {notice}"
    );
    let targets: Vec<_> = records(&rig)
        .into_iter()
        .filter(|event| event.action == "auto-reseat-notice")
        .map(|event| event.target.unwrap_or_default())
        .collect();
    assert_eq!(
        targets,
        ["lead", "colead", "spawner"],
        "same recipient route as refusal, without scout"
    );
    // Each watch_until starts a fresh daemon; OwnedChild drop kills and waits
    // the preceding daemon. A restart must not repeat the same hold or notice; a held episode
    // remains open so an operator's new row is tried immediately.
    let restarted = Instant::now();
    assert!(watch_until(&rig, || restarted.elapsed() >= Duration::from_secs(4)));
    assert_eq!(limitstop_candidate_hold(&rig).len(), 1, "restart dedupes");
    assert_eq!(said(&rig).len(), 1, "restart does not retell");
    configure_from(&rig, ON_NOW, "fake-muse", "fake-opencode");
    assert!(
        watch_until(&rig, || booked(&rig, DONE_ACTION) == 1),
        "new mapping must recover a held episode: {}\n{}",
        rig.events(),
        daemon_err(&rig)
    );
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-opencode");
    assert!(rig.tool_pid(&pane, "muse").is_none(), "old tool stopped");
    assert!(
        rig.tool_pid(&pane, "opencode").is_some(),
        "replacement running"
    );
    assert_eq!(
        booked(&rig, REFUSED_ACTION),
        0,
        "no terminal refusal blocks recovery"
    );
}

#[test]
fn limitstop_operator_ineligible_unmapped_limits_stay_silent() {
    for (tag, switch) in [
        ("limitstopoff", "off"),
        (
            "limitstopnotlisted",
            "all\nauto_reseat_sessions = elsewhere",
        ),
    ] {
        let (rig, pane) = limitstop_no_candidate(tag);
        configure_from(&rig, switch, "fake-grok", "fake-opencode");
        let started = Instant::now();
        assert!(
            watch_until(&rig, || {
                booked(&rig, "limit") == 1 && started.elapsed() >= Duration::from_secs(4)
            }),
            "limit is latched: {}",
            rig.events()
        );
        assert!(
            limitstop_candidate_hold(&rig).is_empty(),
            "operator choice stays silent: {}",
            rig.events()
        );
        assert!(said(&rig).is_empty(), "no human notice: {}", rig.events());
        assert!(rig.tool_pid(&pane, "muse").is_some(), "same seat remains");
    }
}

#[test]
fn limitstop_an_unmapped_second_episode_gets_a_fresh_hold_and_notice() {
    let (rig, pane) = limitstop_no_candidate("limitstoprearm");
    let phase = std::cell::Cell::new(0);
    let started = Instant::now();
    // Keep ONE daemon observing limit, actual relief, then the second limit:
    // restarting between relief and its read would discard its capture latch.
    assert!(
        watch_until(&rig, || {
            let held = limitstop_candidate_hold(&rig).len();
            let notices = records(&rig)
                .iter()
                .filter(|event| event.action == "auto-reseat-notice")
                .count();
            if phase.get() == 0 && held == 1 && said(&rig).len() == 1 && notices == 3 {
                assert!(std::fs::write(rig.scratch.join("limitstop-clear"), "").is_ok());
                phase.set(1);
            } else if phase.get() == 1 && booked(&rig, "alert-cleared") == 1 {
                assert!(std::fs::remove_file(rig.scratch.join("limitstop-clear")).is_ok());
                phase.set(2);
            }
            started.elapsed() >= Duration::from_secs(6)
                && (held == 0
                    || (phase.get() == 2 && held == 2 && said(&rig).len() == 2 && notices == 6))
        }),
        "two episodes complete: {}",
        rig.events()
    );
    let held = limitstop_candidate_hold(&rig);
    assert_eq!(
        held.len(),
        2,
        "fresh hold after observed relief: {}",
        rig.events()
    );
    assert_eq!(said(&rig).len(), 2, "fresh human notice per episode");
    assert_eq!(
        booked(&rig, "alert-cleared"),
        1,
        "actual relief was observed"
    );
    assert_ne!(held[0].reference, held[1].reference, "fresh episode key");
    assert!(
        rig.tool_pid(&pane, "muse").is_some(),
        "held process remains"
    );
}

// Cause classes: every auto reseat outcome leads with a short class word, so a
// clip of a long message (a path, a profile name) never takes the cause.

/// A record's summary, which must lead with `class` whatever follows it.
fn assert_leads(event: &Event, class: &str) {
    let summary = event.summary.as_deref().unwrap_or_default();
    assert!(
        summary.starts_with(&format!("{class}: ")),
        "the summary leads with its class {class:?}: {summary:?}"
    );
}

/// The human's chat notice carries `class` in its first `within` characters.
fn assert_told_early(rig: &Rig, class: &str, within: usize) {
    let chats = said(rig);
    assert_eq!(chats.len(), 1, "one human notice: {chats:?}");
    let notice = chats[0].summary.as_deref().unwrap_or_default();
    assert!(
        notice.starts_with(&format!("auto reseat: {class}: "))
            && notice.find(class).is_some_and(|at| at < within),
        "the notice leads with {class:?}, within {within} characters: {notice:?}"
    );
}

#[test]
fn class_a_refusal_with_no_candidate_leads_with_its_class_when_the_name_clips_the_message() {
    let long = "g".repeat(150);
    let (rig, pane) = limited("classnone", "on", &long);
    remove_unused_profiles(&rig, &["fake-claude-a", "fake-claude-b"]);

    let (code, out, err) = leg(&rig, "spawned.0", KEY);

    assert_eq!(code, Some(1), "out={out} err={err}");
    assert!(err.len() > 160, "the message is long enough to clip: {err}");
    let refused = one_outcome(&rig, REFUSED_ACTION);
    assert_leads(&refused, "no-candidate");
    assert_told_early(&rig, "no-candidate", 60);
    untouched(&rig, &pane);
}

#[test]
fn class_an_unmapped_hold_leads_with_its_class() {
    // No row for the seat's profile and no sibling to derive: nothing to move to.
    let (rig, pane) = limited("classunmapped", "on", "ghost");
    configure_from(&rig, "on", "fake-other", "ghost");
    remove_unused_profiles(&rig, &["fake-claude-a", "fake-claude-b"]);

    let (code, out, err) = leg(&rig, "spawned.0", KEY);

    assert_eq!(code, Some(1), "out={out} err={err}");
    assert_leads(&one_outcome(&rig, HELD_ACTION), "no-candidate");
    assert_told_early(&rig, "no-candidate", 60);
    untouched(&rig, &pane);
}

#[test]
fn class_a_busy_hold_leads_with_its_class() {
    let (rig, pane) = limited("classbusy", "on", "fake-opencode");
    rig.mark_busy(&pane);

    let (code, out, err) = leg(&rig, "spawned.0", KEY);

    assert_eq!(code, Some(1), "out={out} err={err}");
    assert_leads(&one_outcome(&rig, HELD_ACTION), "busy");
    assert_told_early(&rig, "busy", 60);
    untouched(&rig, &pane);
}

#[test]
fn class_an_unreadable_frame_refusal_leads_with_its_class_and_keeps_the_command() {
    let frame = LIMITSTOP_FRAME.replace("❯\u{a0}\n", "❯ draft text\n");
    let (rig, pane) = limitstop_attempt("classframe", &frame);
    let command = format!(
        "ae reseat {} scout --using fake-opencode --stop-unknown",
        rig.session
    );

    let (code, out, err) = leg(&rig, "spawned.0", KEY);

    assert_eq!(code, Some(1), "out={out} err={err}");
    let refused = one_outcome(&rig, REFUSED_ACTION);
    assert_leads(&refused, "unknown-frame");
    let summary = refused.summary.as_deref().unwrap_or_default();
    assert!(
        summary.contains(&command),
        "the command stays whole: {summary}"
    );
    untouched(&rig, &pane);
}

/// The conversation the carry cases move.
const CARRIED: &str = "22222222-2222-4222-8222-222222222222";

/// A running claude seat on account `a` that records [`CARRIED`] under a
/// working copy with a long name, moving to account `b`: the carry Check reads
/// `source` (a planted transcript, or none) and `target` (what account `b`
/// already holds under the conversation's name) before anything is stopped.
fn carry_attempt(tag: &str, source: Option<&[u8]>, target: Option<&[u8]>) -> (Rig, String) {
    let (rig, pane) = limited_on(tag, "claude-a", "on", "fake-claude-b");
    let scratch = std::fs::canonicalize(&rig.scratch)
        .unwrap_or_else(|why| panic!("the scratch dir should canonicalize: {why}"));
    let work = scratch.join(format!("work-{}", "w".repeat(90)));
    assert!(std::fs::create_dir_all(&work).is_ok(), "a working copy");
    let (from, to) = (scratch.join("home-a"), scratch.join("home-b"));
    let meta = rig.dir.join("meta");
    let text = std::fs::read_to_string(&meta).unwrap_or_default();
    let mut text = text
        .lines()
        .filter(|line| {
            ![
                "work_dir=",
                "harness_session.spawned.0=",
                "capture_floor.spawned.0=",
                "launch_time.spawned.0=",
                "observed_model.spawned.0=",
                "config_home.spawned.0=",
                "config_home_base.spawned.0=",
            ]
            .iter()
            .any(|key| line.starts_with(key))
        })
        .fold(String::new(), |mut kept, line| {
            assert!(writeln!(kept, "{line}").is_ok(), "a meta row");
            kept
        });
    assert!(
        writeln!(
            text,
            "work_dir={}\nharness_session.spawned.0={CARRIED}\ncapture_floor.spawned.0=100\n\
             launch_time.spawned.0=111\nobserved_model.spawned.0=Opus 5\n\
             config_home.spawned.0={}",
            work.display(),
            from.display()
        )
        .is_ok(),
        "the seat's history"
    );
    assert!(std::fs::write(&meta, text).is_ok(), "the seat's meta");
    let key = ae::carry::project_key(&work);
    for (home, bytes) in [(from, source), (to, target)] {
        if let Some(bytes) = bytes {
            let project = home.join("projects").join(&key);
            assert!(std::fs::create_dir_all(&project).is_ok(), "a project dir");
            assert!(
                std::fs::write(project.join(format!("{CARRIED}.jsonl")), bytes).is_ok(),
                "a transcript"
            );
        }
    }
    (rig, pane)
}

#[test]
fn class_a_carry_whose_source_is_missing_leads_with_its_class_when_the_path_clips_the_message() {
    let (rig, pane) = carry_attempt("classsrc", None, None);

    let (code, out, err) = leg(&rig, "spawned.0", KEY);

    assert_eq!(code, Some(1), "out={out} err={err}");
    assert!(err.len() > 160, "the message is long enough to clip: {err}");
    let refused = one_outcome(&rig, REFUSED_ACTION);
    assert_leads(&refused, "carry-source-missing");
    assert!(
        refused
            .summary
            .as_deref()
            .unwrap_or_default()
            .chars()
            .count()
            <= 200,
        "{refused:?}"
    );
    assert_told_early(&rig, "carry-source-missing", 60);
    assert!(rig.tool_pid(&pane, "claude").is_some(), "never stopped");
}

#[test]
fn class_a_carry_whose_target_holds_a_different_file_leads_with_its_class() {
    let (rig, pane) = carry_attempt(
        "classtgt",
        Some(b"{\"type\":\"user\"}\n"),
        Some(b"{\"type\":\"fork\"}\n{\"type\":\"longer\"}\n"),
    );

    let (code, out, err) = leg(&rig, "spawned.0", KEY);

    assert_eq!(code, Some(1), "out={out} err={err}");
    let refused = one_outcome(&rig, REFUSED_ACTION);
    assert_leads(&refused, "carry-target-differs");
    assert_told_early(&rig, "carry-target-differs", 60);
    assert!(rig.tool_pid(&pane, "claude").is_some(), "never stopped");
}
