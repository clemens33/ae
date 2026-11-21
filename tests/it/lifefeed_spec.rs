//! Fleet lifecycle feed acceptance through public commands and private tmux.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "the integration fixture owns its state, tmux socket, and child processes"
)]

use super::orchrename_rename_spec::{declare, dispatcher, live, refuses, succeeds};
use super::relay_chat_spec::words;
use super::renametell_routing_spec::{CEILING, Rig};
use std::fs::{self, File};
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

const ORCH: &str = "orchestrator";
const KEEP: &str = "feed-keep";
const LIVE_RENAME: &str = "feed-live-new";
const MOVED: &str = "feed-moved";
const PURGE: &str = "feed-purge";
const TARGET: &str = "feed-target";

fn launch(rig: &Rig, name: &str, caller: Option<&str>) -> String {
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
        caller,
    ));
    assert!(live(rig, name), "{name} launched on the private server");
    let meta = fs::read_to_string(rig.dir(name).join("meta")).expect("real launch record");
    meta.lines()
        .find_map(|line| line.strip_prefix("session_id="))
        .expect("real launch assigned an identity")
        .to_owned()
}

fn chat(rig: &Rig, name: &str) -> String {
    let out = rig.run(&["chat", name], None);
    succeeds(&out);
    out.1
}

fn has_clock(line: &str) -> bool {
    let Some((hour, minute)) = line.split_once(':') else {
        return false;
    };
    hour.len() == 2
        && minute.len() == 2
        && hour.parse::<u8>().is_ok_and(|hour| hour < 24)
        && minute.parse::<u8>().is_ok_and(|minute| minute < 60)
}

fn lifecycle_rows<'a>(text: &'a str, event: &str) -> Vec<&'a str> {
    let prefix = format!("{event} by ");
    text.lines()
        .filter_map(|line| line.split_once(' '))
        .filter(|(clock, body)| has_clock(clock) && body.starts_with(&prefix))
        .map(|(_, body)| body)
        .collect()
}

fn one<'a>(text: &'a str, event: &str) -> &'a str {
    let rows = lifecycle_rows(text, event);
    let [row] = rows.as_slice() else {
        panic!("expected exactly one {event} row, got {rows:?}:\n{text}");
    };
    row
}

fn checked_end(rig: &Rig, name: &str, id: &str, retention: &str, reason: &str) {
    // Use the documented noninteractive owner form against a private fixture.
    succeeds(&rig.run(&["end", name, retention, "-f", "--reason", reason], None));
    assert!(!live(rig, name));
    assert!(!rig.dir(name).exists());
    assert_eq!(
        rig.root.join("archive").join(id).is_dir(),
        retention == "--keep-history",
    );
}

#[test]
fn owner_lifecycle_is_attributed_and_survives_a_rename_and_kept_end() {
    let rig = Rig::new("lf-owner");
    dispatcher(&rig);
    let id = launch(&rig, KEEP, None);
    let initial = chat(&rig, ORCH);
    assert_eq!(
        one(&initial, "feed-keep created"),
        "feed-keep created by owner"
    );
    let ordinary = chat(&rig, KEEP);
    assert!(
        lifecycle_rows(&ordinary, "feed-keep created").is_empty(),
        "{ordinary}"
    );

    refuses(&rig.run(&["stop", KEEP, "--reason"], None));
    assert!(live(&rig, KEEP));
    let long = format!("Check\tthe\nledger {} beyond-the-cap", "é".repeat(2_000));
    succeeds(&rig.run(&["stop", KEEP, "--reason", &long], None));
    assert!(!live(&rig, KEEP));
    let stopped = chat(&rig, ORCH);
    let row = one(&stopped, "feed-keep stopped");
    assert!(
        row.starts_with("feed-keep stopped by owner — Check the ledger é"),
        "{row}"
    );
    assert!(!stopped.contains("beyond-the-cap"), "{stopped}");
    let journal = fs::read_to_string(rig.dir(KEEP).join("events.jsonl")).expect("stop journal");
    assert!(journal.contains("Check the ledger é"), "{journal}");
    assert!(
        !journal.contains("beyond-the-cap"),
        "reason must be capped at the writer"
    );

    succeeds(&rig.helper(
        KEEP,
        "memo",
        &[
            "add",
            "--topic",
            "parking",
            "resume here: compare the two ledgers",
        ],
        None,
    ));
    succeeds(&rig.run(&[KEEP, "--resume", "--no-attach"], None));
    assert!(live(&rig, KEEP));
    let resumed = chat(&rig, ORCH);
    let row = one(&resumed, "feed-keep resumed");
    assert!(
        row.starts_with("feed-keep resumed by owner — parked:"),
        "{row}"
    );
    assert!(row.contains("compare the two ledgers"), "{row}");

    succeeds(&rig.run(&["rename", KEEP, LIVE_RENAME], None));
    assert!(live(&rig, LIVE_RENAME) && !live(&rig, KEEP));
    succeeds(&rig.run(&["stop", LIVE_RENAME], None));
    assert!(!live(&rig, LIVE_RENAME));
    let already = rig.run(&["stop", LIVE_RENAME], None);
    assert_eq!(already.0, Some(1), "{}", words(&already));
    assert!(
        words(&already).contains("not running"),
        "{}",
        words(&already)
    );
    succeeds(&rig.run(&["rename", LIVE_RENAME, MOVED], None));
    assert!(!rig.dir(LIVE_RENAME).exists() && rig.dir(MOVED).is_dir());
    succeeds(&rig.run(&["rename", LIVE_RENAME, MOVED], None));
    checked_end(&rig, MOVED, &id, "--keep-history", "Work accepted");

    let final_chat = chat(&rig, ORCH);
    assert_eq!(
        one(&final_chat, "feed-live-new renamed (was feed-keep)"),
        "feed-live-new renamed (was feed-keep) by owner"
    );
    assert_eq!(
        one(&final_chat, "feed-moved renamed (was feed-live-new)"),
        "feed-moved renamed (was feed-live-new) by owner"
    );
    assert_eq!(
        one(&final_chat, "feed-moved ended"),
        "feed-moved ended by owner — Work accepted"
    );
    assert_eq!(
        one(&final_chat, "feed-live-new stopped"),
        "feed-live-new stopped by owner"
    );
}

#[test]
fn a_refused_purge_reports_no_end_and_a_successful_purge_survives_deletion() {
    let rig = Rig::new("lf-purge");
    dispatcher(&rig);
    let id = launch(&rig, PURGE, None);
    // An outside process with no terminal cannot confirm this purge.
    let denied = rig.run(
        &[
            "end",
            PURGE,
            "--purge-history",
            "--reason",
            "Await confirmation",
        ],
        None,
    );
    refuses(&denied);
    assert!(
        words(&denied).contains("terminal"),
        "the request must reach confirmation: {denied:?}"
    );
    assert!(live(&rig, PURGE) && rig.dir(PURGE).is_dir());
    let before = chat(&rig, ORCH);
    assert!(
        lifecycle_rows(&before, "feed-purge ended").is_empty(),
        "{before}"
    );
    checked_end(&rig, PURGE, &id, "--purge-history", "Remove obsolete work");
    let after = chat(&rig, ORCH);
    assert_eq!(
        one(&after, "feed-purge ended"),
        "feed-purge ended by owner — Remove obsolete work"
    );
}

fn marker(rig: &Rig, text: &str) {
    let line = ae::state::event_line(ae::time::Timestamp::now(), "ae:relay", "chat", "", text);
    rig.append(ORCH, &line);
}

fn read_through(path: &Path, marker: &str) -> String {
    let deadline = Instant::now() + CEILING;
    loop {
        let text = fs::read_to_string(path).unwrap_or_default();
        if text.contains(marker) {
            return text;
        }
        assert!(
            Instant::now() < deadline,
            "chat did not draw {marker}:\n{text}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn an_open_chat_follows_other_sessions_and_counts_one_dispatch_once() {
    let rig = Rig::new("lf-follow");
    let caller = dispatcher(&rig);
    let capture = rig.scratch.join("follow.out");
    let _follow = rig
        .command(None)
        .args(["chat", ORCH, "--follow"])
        .stdin(Stdio::null())
        .stdout(File::create(&capture).expect("follow capture"))
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start real chat follow");
    marker(&rig, "feed-barrier-open");
    read_through(&capture, "feed-barrier-open");

    let id = launch(&rig, TARGET, Some(&caller));
    let (main, peer) = target_panes(&rig, TARGET);
    declare(&rig, TARGET, &main, "working");
    let denied = rig.run(
        &[
            "stop",
            TARGET,
            "--expect-uuid",
            &id,
            "--reason",
            "Do not report this refusal",
        ],
        Some(&caller),
    );
    refuses(&denied);
    assert!(words(&denied).contains("working"), "{}", words(&denied));
    assert!(live(&rig, TARGET));
    for pane in [main, peer] {
        declare(&rig, TARGET, &pane, "done");
    }
    succeeds(&rig.run(
        &[
            "stop",
            TARGET,
            "--expect-uuid",
            &id,
            "--reason",
            "Owner requested a pause",
        ],
        Some(&caller),
    ));
    assert!(!live(&rig, TARGET));
    succeeds(&rig.run(
        &[
            "end",
            TARGET,
            "--keep-history",
            "--expect-uuid",
            &id,
            "--reason",
            "Closed by the dispatcher",
        ],
        Some(&caller),
    ));
    assert!(!rig.dir(TARGET).exists());
    assert!(rig.root.join("archive").join(&id).is_dir());
    marker(&rig, "feed-barrier-actions");
    read_through(&capture, "feed-barrier-actions");
    // A subsequent refresh proves that the same journal bytes did not replay.
    marker(&rig, "feed-barrier-quiet");
    let text = read_through(&capture, "feed-barrier-quiet");
    assert_eq!(
        one(&text, "feed-target created"),
        "feed-target created by orchestrator"
    );
    assert_eq!(
        one(&text, "feed-target stopped"),
        "feed-target stopped by orchestrator — Owner requested a pause"
    );
    assert_eq!(
        one(&text, "feed-target ended"),
        "feed-target ended by orchestrator — Closed by the dispatcher"
    );
}

fn target_panes(rig: &Rig, name: &str) -> (String, String) {
    let panes = rig.tmux(&[
        "list-panes",
        "-s",
        "-t",
        name,
        "-F",
        "#{@ae_slot} #{pane_id}",
    ]);
    let slot = |want: &str| {
        panes
            .lines()
            .filter_map(|row| row.split_once(' '))
            .find_map(|(slot, pane)| (slot == want).then(|| pane.to_owned()))
            .expect("launched seat pane")
    };
    (slot("main"), slot("worker.0"))
}

#[test]
fn a_stop_from_an_ordinary_seat_uses_that_seats_actor() {
    let rig = Rig::new("lf-seat");
    dispatcher(&rig);
    launch(&rig, "feed-caller", None);
    launch(&rig, TARGET, None);
    let (pane, _) = target_panes(&rig, "feed-caller");
    let label = rig.tmux(&["display-message", "-p", "-t", &pane, "#{@ae_agent}"]);
    let display = label.trim();
    assert!(!display.is_empty(), "actual pane has its actor stamp");
    succeeds(&rig.run(
        &["stop", TARGET, "--reason", "From the ordinary seat"],
        Some(&pane),
    ));
    assert!(live(&rig, "feed-caller") && !live(&rig, TARGET));
    let text = chat(&rig, ORCH);
    assert_eq!(
        one(&text, "feed-target stopped"),
        format!("feed-target stopped by {display} — From the ordinary seat")
    );
}

#[test]
fn first_open_limits_backlog_and_skips_seat_work() {
    let rig = Rig::new("lf-backlog");
    dispatcher(&rig);
    for index in 0..6 {
        let name = format!("feed-history-{index}");
        launch(&rig, &name, None);
        succeeds(&rig.run(&["stop", &name], None));
        assert!(!live(&rig, &name));
    }
    let reseat = ae::state::event_line(
        ae::time::Timestamp::now(),
        "watchdog",
        "reseat",
        "",
        "seat-only-lifefeed-token",
    );
    rig.append(ORCH, &reseat);
    let text = chat(&rig, ORCH);
    let expected = (1..6)
        .flat_map(|index| {
            ["created", "stopped"]
                .map(move |event| format!("feed-history-{index} {event} by owner"))
        })
        .collect::<Vec<_>>();
    let actual = text
        .lines()
        .filter_map(|line| line.split_once(' '))
        .filter(|(clock, body)| has_clock(clock) && body.starts_with("feed-history-"))
        .map(|(_, body)| body)
        .collect::<Vec<_>>();
    assert_eq!(
        actual, expected,
        "initial backlog must show the latest ten in order: {text}"
    );
    assert!(
        !text.contains("seat-only-lifefeed-token") && !text.contains("orchestrator reseat"),
        "{text}"
    );
}

#[test]
fn simultaneous_lifecycle_writers_both_reach_the_chat_once() {
    let rig = Rig::new("lf-parallel");
    dispatcher(&rig);
    // Independent live sessions give two CLI processes heritable reasons and
    // independent lifecycle locks. They still share the fleet journal.
    let names = ["feed-parallel-a", "feed-parallel-b"];
    for name in names {
        launch(&rig, name, None);
    }
    let writers = names.map(|name| {
        rig.command(None)
            .args(["stop", name, "--reason", name])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn a real lifecycle writer")
    });
    for (name, writer) in names.into_iter().zip(writers) {
        let out = writer.wait_with_output().expect("lifecycle writer exited");
        assert!(
            out.status.success(),
            "concurrent stop of {name} returned {}: {} {}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let text = chat(&rig, ORCH);
    for name in names {
        assert!(!live(&rig, name));
        assert_eq!(
            one(&text, &format!("{name} stopped")),
            format!("{name} stopped by owner — {name}")
        );
    }
}
