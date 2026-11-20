//! Driver regressions for the orchestrator executor at the public binary entry
//! on private tmux: the audit rows carry the seat's one observation and the
//! target's uuid, an unproven pane gets no helper route, a two-mode create is
//! refused, and owner input waives only the busy declarations.

#![allow(clippy::expect_used, reason = "fixture setup and effect checks")]

use super::super::o2guard_p2_spec::{Seat, World, uuid_of};
use super::super::relay_chat_spec::{Out, Rig as ChatRig, events, words};
use super::*;
use ae::telegram::{
    Echoes,
    bridge::SharedChat,
    inbound::{Origin, Submit, Submitted},
    routing::Target,
};
use std::fs;
use std::io::Write as _;
use std::mem::ManuallyDrop;

const DISPATCHER: &str = "orchestrator";
const TARGET: &str = "drv-target";
/// A pane-less sender name a peer passes; from a pane of the seat's session
/// it carries no power the pane lacks.
const OVERRIDE: &[(&str, &str)] = &[("AE_SENDER_OVERRIDE", "fixture-peer")];

/// One helper route: its name, the argv head before the address and the body,
/// and the extra environment.
type Route<'a> = (&'a str, Vec<&'a str>, &'a [(&'a str, &'a str)]);

fn fixture(tag: &str) -> (Rig, Seat) {
    let rig = Rig::idle(tag);
    fs::write(
        &rig.config,
        format!("{IDLE_CONFIG}chat = off\nauto_upgrade = off\nquota = off\n"),
    )
    .expect("private inert configuration");
    let world = ManuallyDrop::new(World {
        root: rig.home.clone(),
        sock: rig.sock.clone(),
    });
    let overlay = rig.home.join("orchestrator.config");
    fs::create_dir_all(&rig.home).expect("private state root");
    fs::write(
        &overlay,
        include_str!("../../contrib/aeorchestrator/orchestrator.config"),
    )
    .expect("current dispatcher overlay");
    let seat = world.session(
        DISPATCHER,
        "boss",
        &format!("meta_agent=true\nlocal_config={}\n", overlay.display()),
    );
    let path = seat.dir.join("meta");
    let meta = fs::read_to_string(&path).expect("dispatcher meta");
    fs::write(path, format!("{meta}main_pane={}\n", seat.pane)).expect("main pane stamp");
    (rig, seat)
}

fn run(rig: &Rig, caller: Option<&str>, args: &[&str]) -> Out {
    let mut cmd = ae();
    cmd.env("HOME", &rig.scratch)
        .env("AE_HOME", &rig.home)
        .env("CONFIG_FILE", &rig.config)
        .env("TMUX_TMPDIR", &rig.scratch)
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", &rig.sock)
        .env("AE_NO_AUTOSTART", "1")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("AE_SENDER_OVERRIDE")
        .current_dir(&rig.project)
        .stdin(std::process::Stdio::null())
        .args(args);
    if let Some(pane) = caller {
        cmd.env("TMUX", format!("{},0,0", rig.sock.display()))
            .env("TMUX_PANE", pane);
    }
    let out = cmd.output().expect("private executable returned");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn succeeds(out: &Out) {
    assert_eq!(out.0, Some(0), "{}", words(out));
}

fn uuid(rig: &Rig, name: &str) -> String {
    let out = run(rig, None, &["list", "--all", "--json"]);
    succeeds(&out);
    let value = ae::json::parse(&out.1).expect("fleet JSON");
    let Some(ae::json::Value::Arr(sessions)) = value.get("sessions") else {
        panic!("missing session census: {out:?}");
    };
    sessions
        .iter()
        .find(|row| row.get_str("name") == Some(name))
        .and_then(|row| row.get_str("uuid"))
        .filter(|id| id.len() == 36)
        .expect("canonical UUID in fleet census")
        .to_owned()
}

/// The seat's observation on one audit row: its pane, its session uuid and
/// the very server socket the fixture runs.
fn observed_by(row: &ae::events::Event, rig: &Rig, seat: &Seat) -> bool {
    let socket = fs::canonicalize(&rig.sock).expect("the private socket");
    row.actor == "ae/orchestrator"
        && row.caller_pane.as_deref() == Some(seat.pane.as_str())
        && row.caller_session_uuid.as_deref() == Some(uuid_of(DISPATCHER).as_str())
        && row
            .caller_server
            .as_deref()
            .and_then(|server| fs::canonicalize(server).ok())
            .is_some_and(|server| server == socket)
}

/// One request row and its result row under one ref, for `verb` on the
/// target: the request carries `before` as the target uuid, the result `after`.
fn audited_pair(rig: &Rig, seat: &Seat, verb: &str, before: Option<&str>, after: Option<&str>) {
    let rows = events(&seat.dir);
    let request = rows
        .iter()
        .find(|row| {
            row.action == "dispatch-request"
                && row.summary.as_deref() == Some(format!("{verb} {TARGET}").as_str())
        })
        .unwrap_or_else(|| panic!("a {verb} request row: {rows:?}"));
    assert!(observed_by(request, rig, seat), "{request:?}");
    assert_eq!(request.target_session.value(), Some(TARGET), "{request:?}");
    assert_eq!(
        request
            .target_session_uuid
            .as_deref()
            .filter(|id| !id.is_empty()),
        before,
        "{request:?}"
    );
    let result = rows
        .iter()
        .find(|row| row.action == "dispatch-result" && row.reference == request.reference)
        .unwrap_or_else(|| panic!("the {verb} result row: {rows:?}"));
    assert!(observed_by(result, rig, seat), "{result:?}");
    assert_eq!(
        result.summary.as_deref(),
        Some(format!("{verb} {TARGET}: ok").as_str())
    );
    assert_eq!(
        result
            .target_session_uuid
            .as_deref()
            .filter(|id| !id.is_empty()),
        after,
        "{result:?}"
    );
}

#[test]
fn the_seats_create_and_bound_resume_rows_carry_its_observation_and_the_target_uuid() {
    let (rig, dispatcher) = fixture("drv-audit");
    let caller = Some(dispatcher.pane.as_str());
    let dir = rig.project.to_str().expect("private project path");
    succeeds(&run(
        &rig,
        caller,
        &[TARGET, "--dir", dir, "--local", "--no-attach"],
    ));
    rig.wait_for_pane_command(TARGET, "main", "sleep");
    let id = uuid(&rig, TARGET);
    audited_pair(&rig, &dispatcher, "create", None, Some(id.as_str()));

    succeeds(&run(&rig, None, &["stop", TARGET]));
    succeeds(&run(
        &rig,
        caller,
        &[TARGET, "--resume", "--no-attach", "--expect-uuid", &id],
    ));
    rig.wait_for_pane_command(TARGET, "main", "sleep");
    audited_pair(
        &rig,
        &dispatcher,
        "resume",
        Some(id.as_str()),
        Some(id.as_str()),
    );
}

/// One more request row and one more `ok` result row of `verb` to `address`
/// in the seat's journal than `before` counted, each carrying the seat's one
/// observation and the target's uuid.
fn helper_rows(rig: &ChatRig, verb: &str, address: &str, uuid: &str) -> (usize, usize) {
    let socket = fs::canonicalize(&rig.world.sock).expect("the private socket");
    let rows = events(&rig.caller.dir);
    let observed = |row: &&ae::events::Event| {
        row.actor == "ae/orchestrator"
            && row.target.as_deref() == Some(address)
            && row.target_session_uuid.as_deref() == Some(uuid)
            && row.caller_pane.as_deref() == Some(rig.caller.pane.as_str())
            && row.caller_session_uuid.as_deref() == Some(uuid_of("orch").as_str())
            && row
                .caller_server
                .as_deref()
                .and_then(|server| fs::canonicalize(server).ok())
                .is_some_and(|server| server == socket)
    };
    let request = format!("{verb} {address}");
    let result = format!("{verb} {address}: ok");
    let count = |action: &str, summary: &str| {
        rows.iter()
            .filter(observed)
            .filter(|row| row.action == action && row.summary.as_deref() == Some(summary))
            .count()
    };
    (
        count("dispatch-request", &request),
        count("dispatch-result", &result),
    )
}

/// The proved seat's helpers reach another session with the explicit
/// `--cross-session` word on the short route and with or without it on the
/// served route, each audited with the seat's observation.
#[test]
fn the_seats_explicit_and_served_helpers_reach_another_session_audited() {
    let rig = ChatRig::dispatcher("drvhelpers");
    let address = format!("{}:lead", rig.owner.home);
    let uuid = rig.owner.tmux(&[
        "display-message",
        "-p",
        "-t",
        &rig.owner.home,
        "#{@ae_session_uuid}",
    ]);
    let uuid = uuid.trim().to_owned();
    assert_eq!(uuid.len(), 36, "private target is UUID-stamped");
    let dir = rig.caller.dir.to_str().expect("seat path").to_owned();
    for (verb, served) in [
        ("send", ae::cli::SEND),
        ("ask", ae::cli::ASK),
        ("review", ae::cli::REVIEW),
        ("interrupt", ae::cli::INTERRUPT),
    ] {
        let routes: [(&str, Vec<&str>); 3] = [
            ("short explicit", vec!["@orch", verb, "--cross-session"]),
            ("served bare", vec![served, dir.as_str()]),
            (
                "served explicit",
                vec![served, dir.as_str(), "--cross-session"],
            ),
        ];
        for (route, head) in routes {
            let body = format!("dispatcher {route} {verb} fixture body");
            let before = helper_rows(&rig, verb, &address, &uuid);
            let argv = [head.as_slice(), &[address.as_str(), body.as_str()]].concat();
            let delivered = rig.world.run(Some(&rig.caller.pane), &argv, &[]);
            assert_eq!(
                delivered.0,
                Some(0),
                "{route} {verb}: {}",
                words(&delivered)
            );
            assert!(
                rig.sent().contains(&body),
                "{route} {verb} did not reach the target pane"
            );
            assert_eq!(
                helper_rows(&rig, verb, &address, &uuid),
                (before.0 + 1, before.1 + 1),
                "{route} {verb} writes one audited request and result"
            );
        }
    }
}

/// A pane of the seat's session that ae cannot prove to be the seat gets
/// neither the seat's powers nor an ordinary pane's: every helper, bare, with
/// `--cross-session` or under a sender override, on the short route and the
/// served one, is refused as unproven, delivers nothing
/// and writes no audit row.
#[test]
fn an_unproven_seat_pane_is_refused_every_helper_route() {
    let rig = ChatRig::dispatcher("drvunproven");
    let path = rig.caller.dir.join("meta");
    let proved = fs::read_to_string(&path).expect("proved dispatcher meta");
    let unproved = proved.replace("launch_id.main=tok-orch\n", "");
    assert_ne!(
        unproved, proved,
        "the fixture removes only the launch proof"
    );
    fs::write(&path, unproved).expect("role, pane and agent facts remain");
    let address = format!("{}:lead", rig.owner.home);
    let dir = rig.caller.dir.to_str().expect("seat path").to_owned();
    for (verb, served) in [
        ("send", ae::cli::SEND),
        ("ask", ae::cli::ASK),
        ("review", ae::cli::REVIEW),
        ("interrupt", ae::cli::INTERRUPT),
    ] {
        let routes: [Route<'_>; 6] = [
            ("short", vec!["@orch", verb], &[]),
            ("served", vec![served, dir.as_str()], &[]),
            (
                "short explicit",
                vec!["@orch", verb, "--cross-session"],
                &[],
            ),
            (
                "served explicit",
                vec![served, dir.as_str(), "--cross-session"],
                &[],
            ),
            (
                "short explicit override",
                vec!["@orch", verb, "--cross-session"],
                OVERRIDE,
            ),
            ("served override", vec![served, dir.as_str()], OVERRIDE),
        ];
        for (route, head, envs) in routes {
            let body = format!("unproven {route} {verb} fixture body");
            let argv = [head.as_slice(), &[address.as_str(), body.as_str()]].concat();
            let refused = rig.world.run(Some(&rig.caller.pane), &argv, envs);
            let said = words(&refused);
            assert_ne!(refused.0, Some(0), "{route} {verb}: {said}");
            assert!(
                said.contains(&format!("ae: {verb} REFUSED"))
                    && said.contains("cannot prove it is the seat"),
                "{route} {verb}: {said}"
            );
            assert!(!rig.sent().contains(&body), "{route} {verb} was delivered");
        }
    }
    assert!(
        !events(&rig.caller.dir)
            .iter()
            .any(|row| row.action.starts_with("dispatch-")),
        "a refused helper writes no audit row"
    );
}

/// The seat's create names one working-copy mode: a line with two is none of
/// its forms, so nothing is created and no audit row is written.
#[test]
fn a_seat_create_with_two_modes_is_refused_before_any_write() {
    let (rig, dispatcher) = fixture("drv-modes");
    let caller = Some(dispatcher.pane.as_str());
    let dir = rig.project.to_str().expect("private project path");
    for pair in [
        ["--local", "--copy"],
        ["--copy", "--worktree"],
        ["--worktree", "--local"],
    ] {
        let argv = [TARGET, "--dir", dir, pair[0], pair[1], "--no-attach"];
        let out = run(&rig, caller, &argv);
        assert!(matches!(out.0, Some(1 | 2)), "{argv:?}: {}", words(&out));
        assert!(
            out.2.contains("ae: launch REFUSED")
                && out
                    .2
                    .contains("more than one of --local, --copy and --worktree"),
            "{argv:?}: {}",
            words(&out)
        );
        assert!(!rig.dir(TARGET).exists(), "{argv:?} created the session");
    }
    assert!(
        !events(&dispatcher.dir)
            .iter()
            .any(|row| row.action.starts_with("dispatch-")),
        "a refused create writes no audit row"
    );
}

/// The owner's words `body` stored in the seat's own chat journal, the way a
/// Telegram request lands: the request id the seat cites as owner input.
fn owner_input(seat: &Seat, rig: &Rig, body: &str, update_id: i64) -> String {
    let chat = SharedChat {
        root: rig.home.clone(),
        echoes: Echoes::default(),
    };
    let target = Target {
        session: DISPATCHER.to_owned(),
        dir: seat.dir.clone(),
        session_id: uuid_of(DISPATCHER),
        pair: vec!["boss".to_owned()],
        seat: Some("boss".to_owned()),
    };
    let origin = Origin {
        update_id,
        date: ae::time::Timestamp::now().epoch(),
        from_id: "42".to_owned(),
    };
    let result = chat.submit(&target, body, &origin);
    assert!(
        matches!(
            result,
            Submitted::Sent { .. } | Submitted::Unconfirmed { .. }
        ),
        "owner ingress: {result:?}"
    );
    origin.request_id()
}

fn declare(rig: &Rig, agent: &str, state: &str) {
    let (ok, panes) = rig.tmux(&[
        "list-panes",
        "-s",
        "-t",
        TARGET,
        "-F",
        "#{@ae_agent}|#{pane_id}",
    ]);
    assert!(ok, "{panes}");
    let pane = panes
        .lines()
        .filter_map(|line| line.split_once('|'))
        .find_map(|(label, pane)| (label == agent).then_some(pane))
        .expect("the declared seat has a live pane");
    let dir = rig.dir(TARGET);
    let out = run(
        rig,
        Some(pane),
        &[
            ae::cli::STATE,
            dir.to_str().expect("state path"),
            state,
            "The driver fixture declares a seat state through the ordinary helper.",
        ],
    );
    succeeds(&out);
}

fn live(rig: &Rig) -> bool {
    rig.sessions().iter().any(|row| row == TARGET)
}

/// Stored owner words waive only the busy declarations G3 names: with an
/// authentic approval, an unrecognized latest value still refuses and leaves
/// the target live, and a `working` seat is stopped.
#[test]
fn owner_input_waives_working_but_never_an_unrecognized_declaration() {
    let (rig, dispatcher) = fixture("drv-waive");
    let caller = Some(dispatcher.pane.as_str());
    let dir = rig.project.to_str().expect("private project path");
    succeeds(&run(
        &rig,
        caller,
        &[TARGET, "--dir", dir, "--local", "--no-attach"],
    ));
    rig.wait_for_pane_command(TARGET, "main", "sleep");
    let id = uuid(&rig, TARGET);
    let stop = |input: &str| {
        run(
            &rig,
            caller,
            &["stop", TARGET, "--expect-uuid", &id, "--owner-input", input],
        )
    };

    declare(&rig, "lead", "done");
    fs::OpenOptions::new()
        .append(true)
        .open(rig.dir(TARGET).join("events.jsonl"))
        .expect("the target's private journal")
        .write_all(concat!(
            r#"{"ts":"2026-11-19T17:00:00Z","actor":"lead","action":"state","ref":"paused","summary":"unknown state"}"#,
            "\n",
        ).as_bytes())
        .expect("a future state value");
    let first = owner_input(&dispatcher, &rig, &format!("stop {TARGET}"), 301);
    let out = stop(&first);
    assert!(matches!(out.0, Some(1 | 2)), "{}", words(&out));
    assert!(
        out.2.contains("lead") && out.2.contains("paused"),
        "{}",
        words(&out)
    );
    assert!(
        live(&rig),
        "an unrecognized declaration keeps the target live"
    );

    declare(&rig, "lead", "working");
    let second = owner_input(&dispatcher, &rig, &format!("stop {TARGET}"), 302);
    succeeds(&stop(&second));
    assert!(!live(&rig), "an approved stop admits a working seat");
}
