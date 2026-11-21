//! The fleet feed names the agent seat that ran a bound stop, as it names the
//! seat that ran an unbound one; the orchestrator's own bound stop is its own.
//! A detached stop names its asker by a pane tmux is asked about again, never
//! by a word it is told.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "the integration fixture owns its state, tmux socket, and child processes"
)]

use super::orchrename_rename_spec::{declare, dispatcher, live, refuses, succeeds};
use super::relay_chat_spec::words;
use super::renametell_routing_spec::{CEILING, Rig};
use std::fs;
use std::time::{Duration, Instant};

fn launch(rig: &Rig, name: &str) -> String {
    let work = rig.scratch.join(format!("project-{name}"));
    fs::create_dir(&work).expect("private local workspace");
    succeeds(&rig.run(
        &[
            name,
            "--dir",
            work.to_str().expect("project path"),
            "--local",
            "--no-attach",
        ],
        None,
    ));
    assert!(live(rig, name), "{name} launched on the private server");
    let meta = fs::read_to_string(rig.dir(name).join("meta")).expect("real launch record");
    meta.lines()
        .find_map(|line| line.strip_prefix("session_id="))
        .expect("real launch assigned an identity")
        .to_owned()
}

fn panes(rig: &Rig, name: &str) -> Vec<String> {
    rig.tmux(&["list-panes", "-s", "-t", name, "-F", "#{pane_id}"])
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_bound_stop_from_an_ordinary_seat_is_shown_as_that_seat() {
    let rig = Rig::new("lf-bound");
    dispatcher(&rig);
    launch(&rig, "feed-caller");
    let target = launch(&rig, "feed-bound");
    let caller = panes(&rig, "feed-caller")
        .into_iter()
        .next()
        .expect("a caller pane");
    let display = rig.tmux(&["display-message", "-p", "-t", &caller, "#{@ae_agent}"]);
    let display = display.trim();
    assert!(!display.is_empty(), "the caller pane has its actor stamp");
    for pane in panes(&rig, "feed-bound") {
        declare(&rig, "feed-bound", &pane, "done");
    }
    let out = rig.run(
        &[
            "stop",
            "feed-bound",
            "--expect-uuid",
            &target,
            "--reason",
            "Bound from a seat",
        ],
        Some(&caller),
    );
    succeeds(&out);
    assert!(!live(&rig, "feed-bound"), "{}", words(&out));
    let chat = rig.run(&["chat", "orchestrator"], None);
    succeeds(&chat);
    let row = chat
        .1
        .lines()
        .find(|line| line.contains("feed-bound stopped by"))
        .unwrap_or_else(|| panic!("a stop row in:\n{}", chat.1));
    assert!(
        row.ends_with(&format!(
            "feed-bound stopped by {display} — Bound from a seat"
        )),
        "{row}"
    );
}

fn stop_row(rig: &Rig, session: &str) -> String {
    // The row is published after the kill and can wait on the fleet lock.
    let prefix = format!("{session} stopped by ");
    let deadline = Instant::now() + CEILING;
    loop {
        let chat = rig.run(&["chat", "orchestrator"], None);
        succeeds(&chat);
        if let Some(body) = chat.1.lines().find_map(|line| {
            line.split_once(' ')
                .map(|(_, body)| body)
                .filter(|body| body.starts_with(&prefix))
        }) {
            return body.to_owned();
        }
        assert!(
            Instant::now() < deadline,
            "no {session} stop row in:\n{}",
            chat.1
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn gone(rig: &Rig, name: &str) {
    let deadline = Instant::now() + CEILING;
    while live(rig, name) {
        assert!(Instant::now() < deadline, "{name} was not stopped");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn asker(rig: &Rig) -> (String, String) {
    launch(rig, "feed-asker");
    let pane = panes(rig, "feed-asker")
        .into_iter()
        .next()
        .expect("the asker pane");
    let display = rig.tmux(&["display-message", "-p", "-t", &pane, "#{@ae_agent}"]);
    let display = display.trim().to_owned();
    assert!(!display.is_empty(), "the asker pane has its actor stamp");
    (pane, display)
}

// A supervisor the asking seat starts out of its own pane inherits that pane
// and reads it as every command does: the seat that asked is the actor, with
// no word of any argv to believe.
#[test]
fn a_seats_own_detached_stop_keeps_the_seat_that_asked() {
    let rig = Rig::new("lf-self");
    dispatcher(&rig);
    let (pane, display) = asker(&rig);
    launch(&rig, "feed-self");
    let own = panes(&rig, "feed-self")
        .into_iter()
        .next()
        .expect("its own pane");
    let own_display = rig.tmux(&["display-message", "-p", "-t", &own, "#{@ae_agent}"]);
    succeeds(&rig.run(
        &["stop", "--self", "-y", "--reason", "By itself"],
        Some(&own),
    ));
    gone(&rig, "feed-self");
    assert_eq!(
        stop_row(&rig, "feed-self"),
        format!("feed-self stopped by {} — By itself", own_display.trim())
    );
    assert!(live(&rig, "feed-asker"), "{display} at {pane} stays");
}

// The internal `--handoff` word proves no owner: a seat that is proved at the
// entry stays that seat in it, and a caller with no seat is the owner.
#[test]
fn a_handoff_keeps_a_proved_seat_and_reads_no_seat_as_the_owner() {
    let rig = Rig::new("lf-handoff");
    dispatcher(&rig);
    let (pane, display) = asker(&rig);
    launch(&rig, "feed-by-seat");
    launch(&rig, "feed-by-owner");
    succeeds(&rig.run(
        &[
            "_stop",
            "--handoff",
            "feed-by-seat",
            "--reason",
            "Typed by a seat",
        ],
        Some(&pane),
    ));
    gone(&rig, "feed-by-seat");
    assert_eq!(
        stop_row(&rig, "feed-by-seat"),
        format!("feed-by-seat stopped by {display} — Typed by a seat")
    );
    succeeds(&rig.run(
        &["_stop", "--handoff", "feed-by-owner", "--reason", "No seat"],
        None,
    ));
    gone(&rig, "feed-by-owner");
    assert_eq!(
        stop_row(&rig, "feed-by-owner"),
        "feed-by-owner stopped by owner — No seat"
    );
}

// An expectation word proves no owner either: a seat that passes one is
// refused by the expectation's own proof and stops nothing.
#[test]
fn a_seat_that_passes_an_expectation_is_no_owner() {
    let rig = Rig::new("lf-expect");
    dispatcher(&rig);
    let (pane, _) = asker(&rig);
    launch(&rig, "feed-expect");
    refuses(&rig.run(
        &[
            "_stop",
            "--supervise",
            "feed-expect",
            "--expect-session-id",
            "$1",
            "--reason",
            "Forged",
        ],
        Some(&pane),
    ));
    assert!(live(&rig, "feed-expect"));
    let chat = rig.run(&["chat", "orchestrator"], None);
    succeeds(&chat);
    assert!(
        !chat.1.contains("feed-expect stopped by"),
        "a refused stop writes no row:\n{}",
        chat.1
    );
}

// A seat's own end, started from its pane, keeps the seat as its actor.
#[test]
fn a_seats_own_end_keeps_the_seat_that_asked() {
    let rig = Rig::new("lf-own-end");
    dispatcher(&rig);
    launch(&rig, "feed-own");
    let own = panes(&rig, "feed-own")
        .into_iter()
        .next()
        .expect("its own pane");
    let display = rig.tmux(&["display-message", "-p", "-t", &own, "#{@ae_agent}"]);
    succeeds(&rig.run(
        &["end", "-f", "--keep-history", "--reason", "Own end"],
        Some(&own),
    ));
    gone(&rig, "feed-own");
    let deadline = Instant::now() + CEILING;
    loop {
        let chat = rig.run(&["chat", "orchestrator"], None);
        succeeds(&chat);
        if let Some(row) = chat
            .1
            .lines()
            .find(|line| line.contains("feed-own ended by"))
        {
            assert!(
                row.ends_with(&format!("feed-own ended by {} — Own end", display.trim())),
                "{row}"
            );
            break;
        }
        assert!(Instant::now() < deadline, "no end row:\n{}", chat.1);
        std::thread::sleep(Duration::from_millis(100));
    }
}

// No word of the argv is an identity: neither spelling is a flag of any stop.
#[test]
fn no_argv_word_names_who_asked() {
    let rig = Rig::new("lf-word");
    dispatcher(&rig);
    let (pane, _) = asker(&rig);
    launch(&rig, "feed-word");
    for flag in ["--actor=lead", "--actor-pane=%1"] {
        refuses(&rig.run(
            &[
                "_stop",
                "--supervise",
                "feed-word",
                flag,
                "--reason",
                "By its word",
            ],
            Some(&pane),
        ));
        refuses(&rig.run(&["stop", "feed-word", flag], Some(&pane)));
        assert!(live(&rig, "feed-word"), "{flag} stops nothing");
    }
}
