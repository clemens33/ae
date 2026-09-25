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
                Some("refused: no usable candidate: fake-claude-a (exhausted)"),
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
        chats[0].summary.as_deref().is_some_and(|line| line
            .starts_with("auto reseat: scout not moved — refused: no usable candidate: ghost")),
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
        Some("failed: no outcome recorded")
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
