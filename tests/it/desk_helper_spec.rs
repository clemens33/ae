//! Dispatcher authority and input through the actual session helper symlink.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "the acceptance tests own their private file and tmux fixtures"
)]

use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::process::{Output, Stdio};
use std::time::Duration;

use super::cli::{Runner, bounded, helper, mkfifo};
use super::o2guard_p2_spec::{Seat, World};

const DECIDE: &[u8] = b"DECIDE\nD7 | Choose the river plan | https://example.test/decision\n";
const RUNNING: &[u8] = b"RUNNING\nR8 | Verify the next release\n";
const WAIT: Duration = Duration::from_secs(12);

fn install_test_link(dir: &Path) {
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ae"), dir.join("desk"))
        .expect("private desk helper link");
}

fn dispatcher(world: &World) -> Seat {
    let overlay = world.root.join("orchestrator.config");
    fs::write(
        &overlay,
        include_str!("../../contrib/aeorchestrator/orchestrator.config"),
    )
    .expect("real dispatcher overlay");
    let seat = world.session(
        "desk-seat",
        "boss",
        &format!("meta_agent=true\nlocal_config={}\n", overlay.display()),
    );
    let path = seat.dir.join("meta");
    let meta = fs::read_to_string(&path).expect("recorded seat");
    fs::write(&path, format!("{meta}main_pane={}\n", seat.pane)).expect("record own main pane");
    install_test_link(&seat.dir);
    seat
}

fn command(world: &World, dir: &Path, pane: Option<&str>, args: &[&str]) -> Runner {
    let mut command = helper(&dir.join("desk"));
    command
        .env("HOME", &world.root)
        .env("AE_HOME", &world.root)
        .env("CONFIG_FILE", world.root.join("config"))
        .env("TMUX_TMPDIR", &world.root)
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", &world.sock)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("AE_SENDER_OVERRIDE")
        .current_dir(&world.root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(pane) = pane {
        command
            .env("TMUX", format!("{},0,0", world.sock.display()))
            .env("TMUX_PANE", pane);
    }
    command
}

fn run(world: &World, seat: &Seat, pane: Option<&str>, args: &[&str]) -> Output {
    let child = command(world, &seat.dir, pane, args)
        .spawn()
        .expect("real desk helper");
    bounded(child, WAIT).expect("the desk helper must return within the fixture bound")
}

fn stdin_set(world: &World, seat: &Seat, args: &[&str], input: &[u8]) -> Output {
    let mut child = command(world, &seat.dir, Some(&seat.pane), args)
        .stdin(Stdio::piped())
        .spawn()
        .expect("stdin form of the real desk helper");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(input)
        .expect("pass the entire replacement and close the pipe");
    bounded(child, WAIT).expect("stdin case must return")
}

fn success(out: &Output) -> String {
    assert!(
        out.status.success(),
        "status {:?}: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout.clone()).expect("helper stdout")
}

fn refusal(out: &Output, expected: &str) {
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(out.stdout.is_empty());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("desk set REFUSED") && err.contains(expected),
        "{err}"
    );
}

fn record_and_audit(dir: &Path) -> [Option<Vec<u8>>; 2] {
    ["desk-record", "events.jsonl"].map(|name| match fs::read(dir.join(name)) {
        Ok(bytes) => Some(bytes),
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => None,
        Err(why) => panic!("cannot read fixture {name}: {why}"),
    })
}

#[test]
fn its_real_dispatcher_can_set_the_board_and_others_can_show_it() {
    let world = World::new("desk-write");
    let seat = dispatcher(&world);
    let link = seat.dir.join("desk");
    let target = fs::read_link(&link).expect("real helper link");
    assert!(!seat.dir.join("desk-record").exists());
    fs::write(world.root.join("board.txt"), DECIDE).expect("regular input file");

    let first = run(&world, &seat, Some(&seat.pane), &["set", "board.txt"]);
    let confirmation = success(&first);
    assert!(confirmation.contains("desk set:") && confirmation.contains("1 item"));
    let bytes = fs::read_to_string(seat.dir.join("desk-record")).expect("published board record");
    ae::desk::parse(bytes.as_bytes()).expect("core wrote the canonical grammar");
    assert_eq!(
        bytes.lines().skip(2).collect::<Vec<_>>(),
        ["D7\tChoose the river plan\thttps://example.test/decision"]
    );
    let first_state = record_and_audit(&seat.dir);
    let show = run(&world, &seat, None, &["show"]);
    let shown = success(&show);
    assert_eq!(
        shown.lines().collect::<Vec<_>>(),
        [
            "DECIDE",
            "D7 | Choose the river plan | https://example.test/decision"
        ]
    );
    assert_eq!(success(&run(&world, &seat, None, &[])), shown);
    assert_eq!(record_and_audit(&seat.dir), first_state);
    let same = run(&world, &seat, Some(&seat.pane), &["set", "board.txt"]);
    assert!(success(&same).contains("unchanged"));
    assert_eq!(record_and_audit(&seat.dir), first_state);

    success(&stdin_set(&world, &seat, &["set", "-"], RUNNING));
    let bytes = fs::read_to_string(seat.dir.join("desk-record")).expect("replacement record");
    ae::desk::parse(bytes.as_bytes()).expect("canonical replaced board");
    assert_eq!(
        bytes.lines().skip(2).collect::<Vec<_>>(),
        ["R8\tVerify the next release"]
    );
    let second_state = record_and_audit(&seat.dir);
    assert!(success(&stdin_set(&world, &seat, &["set"], RUNNING)).contains("unchanged"));
    assert_eq!(record_and_audit(&seat.dir), second_state);
    assert_eq!(fs::read_link(link).expect("preserved helper link"), target);

    let journal = fs::read_to_string(seat.dir.join("events.jsonl")).expect("desk audit");
    let events: Vec<_> = journal
        .lines()
        .map(|line| ae::events::Event::parse_line(line).expect("audit JSON"))
        .filter(|event| event.action == "desk")
        .collect();
    assert_eq!(events.len(), 2);
    assert!(
        events
            .iter()
            .all(|event| event.actor == "boss" && event.reference.is_none())
    );
    assert!(!journal.contains("Choose the river plan") && !journal.contains("example.test"));

    let usage = success(&run(&world, &seat, None, &["--help"]));
    assert!(usage.contains("desk set") && usage.contains("desk show"));
    assert!(ae::shim::lookup("desk").is_some());
}

#[test]
fn helper_proof_refuses_foreign_and_stale_callers_without_record_or_audit_writes() {
    let world = World::new("desk-proof");
    let seat = dispatcher(&world);
    let ordinary = world.session("desk-other", "other", "");
    install_test_link(&ordinary.dir);
    fs::write(world.root.join("board.txt"), DECIDE).expect("regular input file");
    fs::write(world.root.join("next.txt"), RUNNING).expect("second valid input");
    success(&run(&world, &seat, Some(&seat.pane), &["set", "board.txt"]));
    let before = record_and_audit(&seat.dir);
    let ordinary_before = record_and_audit(&ordinary.dir);
    let set = ["set", "next.txt"];

    refusal(
        &run(&world, &ordinary, Some(&seat.pane), &set),
        "the desk helper of another session",
    );
    refusal(
        &run(&world, &ordinary, Some(&ordinary.pane), &set),
        "not an orchestrator",
    );
    assert_eq!(
        success(&run(&world, &ordinary, None, &["show"])).trim(),
        "no board set"
    );
    assert_eq!(record_and_audit(&ordinary.dir), ordinary_before);
    assert!(
        !ordinary.dir.join("desk-record.lock").exists(),
        "the foreign-session and role refusals precede the writer lock"
    );
    assert_eq!(record_and_audit(&seat.dir), before);
    refusal(&run(&world, &seat, None, &set), "caller");

    world.stamp(&seat.pane, "worker.0", "boss");
    refusal(&run(&world, &seat, Some(&seat.pane), &set), "main seat");
    world.stamp(&seat.pane, "main", "other");
    refusal(
        &run(&world, &seat, Some(&seat.pane), &set),
        "records as main",
    );
    world.stamp(&seat.pane, "main", "boss");

    let path = seat.dir.join("meta");
    let valid = fs::read_to_string(&path).expect("valid proof metadata");
    let without_launch: String = valid
        .split_inclusive('\n')
        .filter(|line| !line.starts_with("launch_id.main="))
        .collect();
    let cases = [
        (
            valid.replace(&format!("main_pane={}", seat.pane), "main_pane=%99999"),
            "recorded main pane",
        ),
        (without_launch, "usable launch id"),
        (
            valid.replace("meta_agent=true\n", ""),
            "not an orchestrator",
        ),
        (
            valid.replace("orchestrator.config", "ordinary.config"),
            "seat overlay",
        ),
    ];
    for (metadata, why) in cases {
        fs::write(&path, metadata).expect("change one proof fact");
        refusal(&run(&world, &seat, Some(&seat.pane), &set), why);
        assert_eq!(record_and_audit(&seat.dir), before);
    }
    fs::write(&path, valid).expect("restore proof metadata");
    assert_eq!(record_and_audit(&seat.dir), before);
    success(&run(&world, &seat, Some(&seat.pane), &set));
}

#[test]
fn a_fifo_or_oversize_helper_input_leaves_the_existing_board_and_audit_unchanged() {
    let world = World::new("desk-file");
    let seat = dispatcher(&world);
    fs::write(world.root.join("board.txt"), DECIDE).expect("regular input file");
    success(&run(&world, &seat, Some(&seat.pane), &["set", "board.txt"]));
    let before = record_and_audit(&seat.dir);
    mkfifo(&world.root.join("pipe"));
    fs::write(world.root.join("large"), vec![b' '; 4097]).expect("over the input cap");
    for name in ["pipe", "large"] {
        let out = run(&world, &seat, Some(&seat.pane), &["set", name]);
        assert!(!out.status.success(), "{name} must fail");
        assert!(out.stdout.is_empty() && !out.stderr.is_empty(), "{out:?}");
        assert_eq!(record_and_audit(&seat.dir), before, "{name} changed state");
    }
}

#[test]
fn a_failed_audit_reports_that_the_new_board_was_already_published() {
    let world = World::new("desk-audit");
    let seat = dispatcher(&world);
    let audit = seat.dir.join("events.jsonl");
    fs::create_dir(&audit).expect("block the journal append in the private fixture");
    fs::write(world.root.join("board.txt"), DECIDE).expect("regular input file");

    let out = run(&world, &seat, Some(&seat.pane), &["set", "board.txt"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(out.stdout.is_empty());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("desk set FAILED AFTER WRITE")
            && err.contains("the board was replaced")
            && err.contains("journal event was not recorded")
            && !err.contains("REFUSED"),
        "{err}"
    );
    let record = fs::read_to_string(seat.dir.join("desk-record")).expect("published despite audit");
    assert_eq!(
        record.lines().skip(2).collect::<Vec<_>>(),
        ["D7\tChoose the river plan\thttps://example.test/decision"]
    );
    assert_eq!(
        fs::read_dir(audit)
            .expect("the journal obstacle remains")
            .count(),
        0
    );
}

#[test]
fn the_real_helper_shows_an_explicitly_empty_board_in_reusable_input_form() {
    let world = World::new("desk-empty-show");
    let seat = dispatcher(&world);
    assert!(success(&run(&world, &seat, None, &["show"])).contains("no board set"));
    success(&stdin_set(&world, &seat, &["set"], b"DECIDE\n"));
    let published = record_and_audit(&seat.dir);
    let shown = success(&run(&world, &seat, None, &["show"]));
    let items = ae::desk::parse_input(shown.as_bytes()).expect("show returns valid set input");
    assert!(items.is_empty());
    assert!(success(&stdin_set(&world, &seat, &["set"], shown.as_bytes())).contains("unchanged"));
    assert_eq!(record_and_audit(&seat.dir), published);
}
