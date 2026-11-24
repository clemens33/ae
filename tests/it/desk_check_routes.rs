//! Driver-owned real-route cases of `desk check`, beside the frozen
//! acceptance in `desk_check_spec`: the same private fleet and the same helper
//! routes, for the damage and the layouts that module does not reach.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "the fixture runs the real helper and owns an isolated fleet"
)]

use std::fs;
use std::io::Write;
use std::process::Stdio;
use std::time::Duration;

use super::cli::{Runner, ae, bounded, helper};
use super::o2guard_p2_spec::uuid_of;
use super::relay_chat_spec::{Out, Rig, words};

const SECTIONS: [&str; 4] = [
    "unboarded",
    "boarded (hint)",
    "known closed",
    "unresolved coverage",
];

fn rig(tag: &str) -> Rig {
    let rig = Rig::dispatcher(tag);
    for session in ["orch", rig.owner.home.as_str()] {
        rig.owner
            .tmux(&["set-environment", "-t", session, "AE_SESSION", "1"]);
    }
    rig
}

fn run(rig: &Rig, mut command: Runner, pane: Option<&str>, args: &[&str]) -> Out {
    let root = &rig.world.root;
    let socket = &rig.world.sock;
    command
        .env("HOME", root)
        .env("AE_HOME", root)
        .env("CONFIG_FILE", root.join("config"))
        .env("TMUX_TMPDIR", root)
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", socket)
        .current_dir(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(pane) = pane {
        command
            .env("TMUX", format!("{},0,0", socket.display()))
            .env("TMUX_PANE", pane);
    }
    let child = command.spawn().expect("run ae in the private fleet");
    let out =
        bounded(child, Duration::from_secs(12)).expect("helper returns within the fixture bound");
    (
        out.status.code(),
        String::from_utf8(out.stdout).expect("stdout is text"),
        String::from_utf8(out.stderr).expect("stderr is text"),
    )
}

fn report(rig: &Rig) -> Out {
    run(rig, ae(), None, &["@orch", "desk", "check"])
}

fn success(out: &Out, code: i32) {
    assert_eq!(out.0, Some(code), "{}", words(out));
}

fn section<'a>(out: &'a Out, name: &str) -> Vec<&'a str> {
    let mut lines = out.1.lines();
    assert!(
        lines
            .by_ref()
            .any(|line| line.trim().to_ascii_lowercase().starts_with(name)),
        "no {name} section in {}",
        words(out)
    );
    lines
        .take_while(|line| {
            let line = line.trim().to_ascii_lowercase();
            !SECTIONS.iter().any(|heading| line.starts_with(heading))
        })
        .collect()
}

fn row<'a>(out: &'a Out, name: &str, id: &str) -> &'a str {
    section(out, name)
        .into_iter()
        .find(|line| line.contains(id))
        .unwrap_or_else(|| panic!("missing {id} in {name}: {}", words(out)))
}

fn board(rig: &Rig, item: &str) {
    fs::write(
        rig.world.root.join("board.txt"),
        format!("RUNNING\n{item}\n"),
    )
    .expect("provide board in its ordinary input grammar");
    let out = run(
        rig,
        ae(),
        Some(&rig.caller.pane),
        &["@orch", "desk", "set", "board.txt"],
    );
    success(&out, 0);
}

#[test]
fn a_damaged_unrelated_origin_line_keeps_the_readable_relay_claim_and_names_the_damage() {
    let rig = rig("dckpart");
    let id = rig.ask("This relay stays readable beside a damaged line.");
    let journal = rig.caller.dir.join("events.jsonl");
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&journal)
        .expect("the orchestrator journal");
    file.write_all(b"{\"ts\": this line is damaged\n")
        .expect("an unrelated damaged line");
    drop(file);
    let out = report(&rig);
    success(&out, 1);
    let relay = row(&out, "unboarded", &id);
    assert!(
        relay.contains("origin journal partly unreadable"),
        "{relay}"
    );
    let gap = section(&out, "unresolved coverage")
        .into_iter()
        .find(|line| line.contains("orch") && line.contains("partly unreadable"))
        .unwrap_or_else(|| panic!("the damaged session has its one gap: {}", words(&out)));
    assert!(gap.contains("readable claims are kept"), "{gap}");
    assert!(
        section(&out, "unresolved coverage")
            .iter()
            .all(|line| !line.contains("seats of orch")),
        "the journal loss is said once, by the session's line: {}",
        words(&out)
    );
}

#[test]
fn a_lost_fleet_source_is_unresolved_coverage_through_the_real_route() {
    let rig = rig("dcklost");
    let id = rig.ask("This relay is read beside a lost fleet source.");
    let blocked = rig.world.root.join("worktrees").join("opaque-checkout");
    fs::create_dir_all(&blocked).expect("a worktree checkout");
    fs::write(blocked.join(".ae"), "not a directory").expect("a state root that cannot list");
    let out = report(&rig);
    success(&out, 1);
    row(&out, "unboarded", &id);
    let gap = row(&out, "unresolved coverage", "would not enumerate");
    assert!(
        gap.contains("opaque-checkout"),
        "the gap names the source: {gap}"
    );
}

#[test]
fn a_helper_in_a_linked_worktree_checks_the_whole_fleet_with_its_own_board() {
    let rig = rig("dcknest");
    let id = rig.ask("This relay belongs to the global fleet.");
    board(&rig, "R1 | A board item of the orchestrator only");
    let nested = rig
        .world
        .root
        .join("worktrees")
        .join("linked-checkout")
        .join(".ae")
        .join("linked");
    fs::create_dir_all(&nested).expect("a linked worktree record");
    let uuid = uuid_of("linked");
    fs::write(
        nested.join("meta"),
        format!("session=linked\nsession_id={uuid}\nseat.main=lead\nlaunch_id.main=tok-linked\n"),
    )
    .expect("the linked record's meta");
    let link = nested.join("desk");
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ae"), &link).expect("a private desk link");
    let out = run(&rig, helper(&link), None, &["check"]);
    success(&out, 1);
    assert_eq!(
        out.1.lines().next(),
        Some("board: absent"),
        "the board is the linked record's own, not the orchestrator's: {}",
        words(&out)
    );
    row(&out, "unboarded", &id);
    let sessions = out
        .1
        .lines()
        .find(|line| line.starts_with("desk check"))
        .expect("the report header");
    assert!(
        !sessions.contains("sessions 0 read, 0 stopped"),
        "the fleet is the whole AE_HOME: {sessions}"
    );
}

#[test]
fn a_stopped_dispatcher_keeps_its_relay_and_labels_it_through_the_real_route() {
    let rig = rig("dckstop");
    let id = rig.ask("The dispatcher stops while this relay is open.");
    let (stopped, why) = rig.world.tmux("kill-session -t", &["orch"]);
    assert!(stopped, "stop the dispatcher: {why}");
    let nested = rig
        .world
        .root
        .join("worktrees")
        .join("viewer-checkout")
        .join(".ae")
        .join("viewer");
    fs::create_dir_all(&nested).expect("a viewing record");
    let uuid = uuid_of("viewer");
    fs::write(
        nested.join("meta"),
        format!("session=viewer\nsession_id={uuid}\nseat.main=lead\nlaunch_id.main=tok-viewer\n"),
    )
    .expect("the viewing record's meta");
    let link = nested.join("desk");
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ae"), &link).expect("a private desk link");
    let out = run(&rig, helper(&link), None, &["check"]);
    success(&out, 1);
    let relay = row(&out, "unboarded", &id);
    assert!(relay.ends_with("origin stopped"), "{relay}");
}

fn excludes(out: &Out, name: &str, id: &str) {
    assert!(
        section(out, name)
            .into_iter()
            .all(|line| !line.contains(id)),
        "unexpected {id} in {name}: {}",
        words(out)
    );
}

fn waiting(rig: &Rig) {
    let route = format!("@{}", rig.owner.home);
    let out = run(
        rig,
        ae(),
        Some(&rig.owner.tool.pane),
        &[
            &route,
            "state",
            "waiting-user",
            "The private fixture needs the owner's input before continuing with the next verification step.",
        ],
    );
    success(&out, 0);
}

/// Give the target seat the completed launch baseline a live seat records,
/// ten seconds before now.
fn baseline(rig: &Rig) {
    let meta_path = rig.owner.tool.dir.join("meta");
    let meta = fs::read_to_string(&meta_path).expect("seat metadata");
    let started = ae::time::Timestamp::now().epoch() - 10;
    fs::write(&meta_path, format!("{meta}launch_time.main={started}\n"))
        .expect("a live seat has a completed launch baseline");
}

/// Append to the target's journal a declaration that another incarnation of
/// the same session name made: the same name and slot, another session id.
fn foreign_declaration(rig: &Rig, value: &str) {
    let body = ae::state::event_body(
        ae::time::Timestamp::now(),
        "lead",
        &ae::state::Declaration {
            value: value.to_owned(),
            reason: "an earlier incarnation under the same session name speaks for its own seat"
                .to_owned(),
            on: None,
        },
    );
    let line = body.lines().next().expect("a record");
    let routed = format!(
        "{},\"actor_slot\":\"main\",\"actor_session\":\"{}\",\"actor_session_id\":\"{}\"}}\n",
        line.trim_end_matches('}'),
        rig.owner.home,
        uuid_of("another-incarnation"),
    );
    let mut journal = fs::OpenOptions::new()
        .append(true)
        .open(rig.owner.tool.dir.join("events.jsonl"))
        .expect("the target journal");
    journal
        .write_all(routed.as_bytes())
        .expect("a foreign declaration");
}

#[test]
fn a_seats_own_wait_stays_listed_beside_a_newer_row_of_another_incarnation() {
    let rig = rig("dckwown");
    baseline(&rig);
    waiting(&rig);
    foreign_declaration(&rig, "working");
    let seat = format!("{}:lead", rig.owner.home);
    let out = report(&rig);
    success(&out, 1);
    let wait = row(&out, "unboarded", &seat);
    assert!(wait.contains("waiting-user since"), "{wait}");
    excludes(&out, "unresolved coverage", &seat);
}

#[test]
fn a_waiting_row_of_another_incarnation_is_coverage_and_never_a_wait() {
    let rig = rig("dckwfor");
    baseline(&rig);
    foreign_declaration(&rig, "waiting-user");
    let seat = format!("{}:lead", rig.owner.home);
    let out = report(&rig);
    success(&out, 0);
    excludes(&out, "unboarded", &seat);
    let gap = row(&out, "unresolved coverage", &seat);
    assert!(
        gap.contains("no declaration of the seat under its session id"),
        "{gap}"
    );
}
