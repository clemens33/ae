//! Orchestrator executor acceptance at the public binary entry on private tmux.

#![allow(clippy::expect_used, reason = "fixture setup and effect checks")]

use super::super::o2guard_p2_spec::{Seat, World, uuid_of};
use super::super::relay_chat_spec::{Out, events, words};
use super::*;
use ae::telegram::{
    Echoes,
    bridge::SharedChat,
    inbound::{Origin, Submit, Submitted},
    routing::Target,
};
use std::fs;
use std::mem::ManuallyDrop;

const DISPATCHER: &str = "orchestrator";
const TARGET: &str = "exec-target";
const OTHER: &str = "exec-other";
const REASON: &str = "The executor acceptance fixture changes the live seat declaration through the ordinary state helper.";

fn fixture(tag: &str) -> (Rig, Seat) {
    let rig = Rig::idle(tag);
    let pair = IDLE_CONFIG
        .replace("lead = idle\n", "lead = idle\ncolead = idle\n")
        .replace("layout = vertical", "layout = lead-pair");
    fs::write(
        &rig.config,
        format!("{pair}workers = colead\nchat = off\nauto_upgrade = off\nquota = off\n"),
    )
    .expect("private inert lead-pair configuration");
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
    .expect("current released dispatcher overlay");
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

fn refused(out: &Out) {
    assert!(matches!(out.0, Some(1 | 2)), "{}", words(out));
}

fn live(rig: &Rig, name: &str) -> bool {
    rig.sessions().iter().any(|row| row == name)
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

fn create(rig: &Rig, caller: Option<&str>, name: &str) -> String {
    let dir = rig.project.to_str().expect("private project path");
    let out = run(rig, caller, &[name, "--dir", dir, "--local", "--no-attach"]);
    succeeds(&out);
    rig.wait_for_pane_command(name, "main", "sleep");
    uuid(rig, name)
}

fn declare(rig: &Rig, name: &str, agent: &str, state: &str) {
    let (ok, panes) = rig.tmux(&[
        "list-panes",
        "-s",
        "-t",
        name,
        "-F",
        "#{@ae_agent}|#{pane_id}",
    ]);
    assert!(ok, "{panes}");
    let pane = panes
        .lines()
        .filter_map(|line| line.split_once('|'))
        .find_map(|(label, pane)| (label == agent).then_some(pane))
        .expect("the declared seat has a live pane");
    let dir = rig.dir(name);
    let mut args = vec![ae::cli::STATE, dir.to_str().expect("state path"), state];
    if state == "waiting-agent" {
        args.extend(["--on", "lead"]);
    }
    args.push(REASON);
    let out = run(rig, Some(pane), &args);
    succeeds(&out);
    assert!(
        out.1.contains(&format!("Marked {agent} {state}")),
        "{}",
        words(&out)
    );
}

#[test]
fn dispatcher_creates_and_resumes_without_attaching() {
    let (rig, dispatcher) = fixture("exec-create");
    let caller = Some(dispatcher.pane.as_str());
    let dir = rig.project.to_str().expect("private project path");
    refused(&run(&rig, caller, &[TARGET, "--dir", dir, "--local"]));
    assert!(!rig.dir(TARGET).exists());
    assert!(!live(&rig, TARGET));
    let id = create(&rig, caller, TARGET);
    fs::write(rig.dir(TARGET).join("exec-sentinel"), b"session memory").expect("sentinel");
    succeeds(&run(&rig, None, &["stop", TARGET]));
    assert!(!live(&rig, TARGET));
    succeeds(&run(&rig, caller, &[TARGET, "--no-attach"]));
    rig.wait_for_pane_command(TARGET, "main", "sleep");
    assert!(live(&rig, TARGET));
    assert_eq!(uuid(&rig, TARGET), id);
    assert_eq!(
        fs::read(rig.dir(TARGET).join("exec-sentinel")).expect("saved memory"),
        b"session memory"
    );
    assert!(live(&rig, DISPATCHER));
    let rows = events(&dispatcher.dir);
    let named = |row: &ae::events::Event| {
        row.actor == "ae/orchestrator"
            && row.target_session_uuid.as_deref() == Some(id.as_str())
            && row.target_session.value() == Some(TARGET)
    };
    let request = rows
        .iter()
        .find(|row| {
            row.action == "dispatch-request"
                && named(row)
                && row.summary.as_deref() == Some(format!("start {TARGET}").as_str())
        })
        .expect("named start has its own request audit with the preexisting UUID");
    assert!(request.reference.is_some());
    assert!(rows.iter().any(|row| row.action == "dispatch-result"
        && named(row)
        && row.reference == request.reference
        && row.summary.as_deref() == Some(format!("start {TARGET}: ok").as_str())));
}

#[test]
fn dispatcher_self_stop_reports_the_exact_owner_command() {
    let (rig, dispatcher) = fixture("exec-self");
    let id = uuid_of(DISPATCHER);
    let meta = dispatcher.dir.join("meta");
    let before = fs::read(&meta).expect("existing dispatcher meta");
    let caller = Some(dispatcher.pane.as_str());
    let command = format!("ae stop {DISPATCHER} -y");
    for args in [
        vec!["stop", DISPATCHER, "--expect-uuid", &id],
        vec!["stop", "--self"],
        vec!["stop"],
    ] {
        let out = run(&rig, caller, &args);
        refused(&out);
        assert!(
            words(&out).contains(&command),
            "missing {command:?}: {}",
            words(&out)
        );
        assert!(live(&rig, DISPATCHER));
        assert_eq!(fs::read(&meta).expect("dispatcher meta remains"), before);
        assert!(!rig.home.join("archive").join(&id).exists());
    }
}

fn ingress(rig: &Rig, seat: &Seat, session: &str, body: &str, update_id: i64) -> String {
    let chat = SharedChat {
        root: rig.home.clone(),
        echoes: Echoes::default(),
    };
    let target = Target {
        session: session.to_owned(),
        dir: seat.dir.clone(),
        session_id: uuid_of(session),
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
    let id = origin.request_id();
    assert!(
        events(&seat.dir).iter().any(|row| row.action == "ask"
            && row.actor == "console:local"
            && row.reference.as_deref() == Some(id.as_str())),
        "the stored owner opening exists"
    );
    id
}

fn owner_input(rig: &Rig, dispatcher: &Seat, body: &str, update_id: i64) -> String {
    ingress(rig, dispatcher, DISPATCHER, body, update_id)
}

#[test]
fn real_owner_words_in_another_session_cannot_authorize_the_dispatcher() {
    let (rig, dispatcher) = fixture("exec-foreign-input");
    let id = create(&rig, None, TARGET);
    declare(&rig, TARGET, "colead", "working");
    let world = ManuallyDrop::new(World {
        root: rig.home.clone(),
        sock: rig.sock.clone(),
    });
    let foreign = world.session(OTHER, "boss", "");
    let body = format!("stop {TARGET}");
    let wrong = ingress(&rig, &foreign, OTHER, &body, 223);
    let stop = |pane, input| {
        run(
            &rig,
            pane,
            &["stop", TARGET, "--expect-uuid", &id, "--owner-input", input],
        )
    };
    let caller = Some(dispatcher.pane.as_str());
    for pane in [None, Some(foreign.pane.as_str()), caller] {
        refused(&stop(pane, &wrong));
        assert!(live(&rig, TARGET));
    }
    let right = owner_input(&rig, &dispatcher, &body, 224);
    let path = dispatcher.dir.join("events.jsonl");
    let genuine = fs::read_to_string(&path).expect("owner journal");
    let opening_actor = "\"actor\":\"console:local\"";
    let opening_ref = format!("\"ref\":\"{right}\"");
    let damaged: String = genuine
        .lines()
        .map(|line| {
            if line.contains("\"action\":\"ask\"") && line.contains(&opening_ref) {
                format!("{}\n", line.replace(opening_actor, "\"actor\":\"a-peer\""))
            } else {
                format!("{line}\n")
            }
        })
        .collect();
    assert_ne!(damaged, genuine, "remove only the stored actor proof");
    fs::write(&path, damaged).expect("simulate an opening with the wrong actor");
    refused(&stop(caller, &right));
    assert!(live(&rig, TARGET));
    fs::write(path, genuine).expect("restore the real owner actor");
    succeeds(&stop(caller, &right));
    assert!(!live(&rig, TARGET));
}

#[test]
fn a_bound_stop_uses_one_real_owner_input_for_the_named_busy_target() {
    let (rig, dispatcher) = fixture("exec-approval");
    let caller = Some(dispatcher.pane.as_str());
    let id = create(&rig, None, TARGET);
    declare(&rig, TARGET, "lead", "done");
    declare(&rig, TARGET, "colead", "waiting-agent");
    let bare = ["stop", TARGET, "--expect-uuid", &id];
    let stop = |named_id: &str, input: &str| {
        run(
            &rig,
            caller,
            &[
                "stop",
                TARGET,
                "--expect-uuid",
                named_id,
                "--owner-input",
                input,
            ],
        )
    };
    let busy = run(&rig, caller, &bare);
    refused(&busy);
    assert!(words(&busy).contains("colead") && words(&busy).contains("waiting-agent"));

    for (update, text) in [
        (217, format!("Please end {TARGET}")),
        (218, format!("Please stop {OTHER}")),
        (219, format!("The stopper for {TARGET} is still working")),
        (220, format!("Please stop {TARGET}-shadow")),
    ] {
        let input = owner_input(&rig, &dispatcher, &text, update);
        refused(&stop(&id, &input));
        assert!(live(&rig, TARGET));
    }
    let input = owner_input(&rig, &dispatcher, &format!("STOP {TARGET}"), 221);
    let wrong_uuid = uuid_of(OTHER);
    assert_ne!(wrong_uuid, id);
    refused(&stop(&wrong_uuid, &input));
    assert!(live(&rig, TARGET));

    refused(&run(
        &rig,
        caller,
        &[
            "stop",
            TARGET,
            "--expect-uuid",
            &id,
            "--owner-input",
            &input,
            "--owner-input",
            &input,
        ],
    ));
    assert!(live(&rig, TARGET));
    assert!(
        !events(&dispatcher.dir)
            .iter()
            .any(|row| row.action == "dispatch-request"
                && row.owner_input.as_deref() == Some(input.as_str())),
        "a refusal before admission must leave the approval unused"
    );

    succeeds(&stop(&id, &input));
    assert!(!live(&rig, TARGET));
    assert_eq!(uuid(&rig, TARGET), id);
    let rows = events(&dispatcher.dir);
    let expected_summary = format!("stop {TARGET}");
    let matches_audit = |row: &ae::events::Event| {
        row.actor == "ae/orchestrator"
            && row.owner_input.as_deref() == Some(input.as_str())
            && row.target_session_id.value() == Some(id.as_str())
            && row.target_session_uuid.as_deref() == Some(id.as_str())
            && row.target_session.value() == Some(TARGET)
            && row.actor_session_id.value() == Some(uuid_of(DISPATCHER).as_str())
            && row.caller_session_uuid.as_deref() == Some(uuid_of(DISPATCHER).as_str())
            && row.actor_slot.value() == Some("main")
            && row.caller_pane.as_deref() == Some(dispatcher.pane.as_str())
    };
    let request = rows
        .iter()
        .find(|row| row.action == "dispatch-request" && matches_audit(row))
        .expect("dispatcher request audit names the target UUID and actual owner input");
    assert_eq!(request.summary.as_deref(), Some(expected_summary.as_str()));
    assert!(request.reference.is_some());
    assert!(
        rows.iter().any(|row| row.action == "dispatch-result"
            && matches_audit(row)
            && row.reference == request.reference
            && row.summary.as_deref() == Some(format!("{expected_summary}: ok").as_str())),
        "dispatcher result audit correlates to the request"
    );
    succeeds(&run(&rig, None, &[TARGET, "--no-attach"]));
    rig.wait_for_pane_command(TARGET, "main", "sleep");
    declare(&rig, TARGET, "colead", "working");
    let replay = stop(&id, &input);
    refused(&replay);
    assert!(
        live(&rig, TARGET),
        "spent owner input stopped a later incarnation of the same UUID"
    );
}

#[test]
fn dispatcher_does_not_create_until_its_request_audit_is_writable() {
    let (rig, dispatcher) = fixture("exec-audit-gap");
    let caller = Some(dispatcher.pane.as_str());
    let event_file = dispatcher.dir.join("events.jsonl");
    assert!(!event_file.exists());
    fs::create_dir(&event_file).expect("simulate a journal that cannot be appended");
    let dir = rig.project.to_str().expect("private project path");
    let out = run(
        &rig,
        caller,
        &[TARGET, "--dir", dir, "--local", "--no-attach"],
    );
    refused(&out);
    assert!(
        words(&out).contains("could not record the dispatch"),
        "{}",
        words(&out)
    );
    assert!(!live(&rig, TARGET));
    assert!(!rig.dir(TARGET).exists());

    fs::remove_dir(&event_file).expect("restore writable journal pathname");
    let id = create(&rig, caller, TARGET);
    let rows = events(&dispatcher.dir);
    let request = rows
        .iter()
        .find(|row| {
            row.action == "dispatch-request"
                && row.actor == "ae/orchestrator"
                && row.target_session.value() == Some(TARGET)
                && row.summary.as_deref() == Some(format!("create {TARGET}").as_str())
        })
        .expect("create is audited before the UUID exists");
    assert!(request.target_session_id.value().is_none());
    assert!(request.target_session_uuid.is_none());
    assert!(request.owner_input.is_none());
    assert!(request.reference.is_some());
    assert!(
        rows.iter().any(|row| {
            row.action == "dispatch-result"
                && row.actor == "ae/orchestrator"
                && row.reference == request.reference
                && row.target_session_uuid.as_deref() == Some(id.as_str())
                && row.summary.as_deref() == Some(format!("create {TARGET}: ok").as_str())
        }),
        "create result records the new canonical UUID"
    );
}

#[test]
fn a_bound_stop_with_a_mismatched_live_uuid_never_uses_approved_owner_input() {
    let (rig, dispatcher) = fixture("exec-stop-stamp");
    let caller = Some(dispatcher.pane.as_str());
    let id = create(&rig, None, TARGET);
    declare(&rig, TARGET, "colead", "working");
    let input = owner_input(&rig, &dispatcher, &format!("stop {TARGET}"), 222);
    let stop = || {
        run(
            &rig,
            caller,
            &[
                "stop",
                TARGET,
                "--expect-uuid",
                &id,
                "--owner-input",
                &input,
            ],
        )
    };
    let scope = format!("={TARGET}:");
    let different = uuid_of(OTHER);
    assert_ne!(id, different);
    assert!(
        rig.tmux(&["set-option", "-t", &scope, "@ae_session_uuid", &different])
            .0
    );
    refused(&stop());
    assert!(live(&rig, TARGET));
    assert!(
        rig.tmux(&["set-option", "-t", &scope, "@ae_session_uuid", &id])
            .0
    );
    succeeds(&stop());
    assert!(!live(&rig, TARGET));
}

#[test]
fn an_unproven_dispatcher_pane_cannot_inherit_ordinary_stop_rights() {
    let (rig, dispatcher) = fixture("exec-stop-proof");
    let id = create(&rig, None, TARGET);
    declare(&rig, TARGET, "colead", "working");
    let caller = Some(dispatcher.pane.as_str());
    let path = dispatcher.dir.join("meta");
    let complete = fs::read_to_string(&path).expect("proved dispatcher meta");
    let missing = complete.replace("launch_id.main=tok-orchestrator\n", "");
    assert_ne!(missing, complete, "fixture removed only the launch proof");
    fs::write(&path, missing).expect("role, pane and agent facts remain");

    let out = run(&rig, caller, &["stop", TARGET, "-y"]);
    refused(&out);
    assert!(
        words(&out).contains("cannot prove it is the seat"),
        "{}",
        words(&out)
    );
    assert!(live(&rig, TARGET));

    succeeds(&run(&rig, None, &["stop", TARGET, "-y"]));
    assert!(
        !live(&rig, TARGET),
        "owner's existing stop works for the same target"
    );
    assert_eq!(uuid(&rig, TARGET), id);
}
