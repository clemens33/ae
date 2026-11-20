//! Seatguard acceptance through the real binary and one private tmux socket.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixture setup and independent before/after state checks"
)]

use super::super::o2guard_p2_spec::{Seat, World, uuid_of};
use super::super::relay_chat_spec::{Out, events, words};
use super::*;
use ae::events::Event;
use std::fs;
use std::mem::ManuallyDrop;

const SOURCE: &str = "assistant";
const TARGET: &str = "roster";
const FRESH: &str = "sg-new";

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
    let version = format!("{}={}\n", ae::migrate::KEY, ae::migrate::CURRENT);
    let caller = world.session(SOURCE, "lead", &version);
    (rig, caller)
}

fn from_pane(cmd: &mut Runner, rig: &Rig, pane: &str) {
    cmd.env("TMUX", format!("{},0,0", rig.sock.display()))
        .env("TMUX_PANE", pane);
}

fn collect(mut cmd: Runner) -> Out {
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .expect("private executable returned");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn run(rig: &Rig, pane: Option<&str>, argv: &[&str]) -> Out {
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
        .args(argv);
    if let Some(pane) = pane {
        from_pane(&mut cmd, rig, pane);
    }
    collect(cmd)
}

fn succeeds(out: &Out) {
    assert_eq!(out.0, Some(0), "{}", words(out));
}

fn refuses_cross(out: &Out, target: &str) {
    let shown = words(out);
    assert_eq!(out.0, Some(2), "{shown}");
    let said = out.2.to_ascii_lowercase();
    assert!(said.contains("refus") && said.contains(target), "{shown}");
    assert!(
        said.contains("owner") || said.contains("orchestrator"),
        "{shown}"
    );
}

fn is_live(rig: &Rig, name: &str) -> bool {
    rig.sessions().iter().any(|live| live == name)
}

fn target_uuid(rig: &Rig, name: &str) -> String {
    let out = run(rig, None, &["list", "--all", "--json"]);
    succeeds(&out);
    let value = ae::json::parse(&out.1).expect("fleet JSON");
    let Some(ae::json::Value::Arr(rows)) = value.get("sessions") else {
        panic!("missing session census: {out:?}");
    };
    rows.iter()
        .find(|row| row.get_str("name") == Some(name))
        .and_then(|row| row.get_str("uuid"))
        .filter(|id| id.len() == 36)
        .expect("canonical target uuid")
        .to_owned()
}

fn stopped_target(rig: &Rig) -> String {
    succeeds(&run(rig, None, &[TARGET, "--local", "--no-attach"]));
    rig.wait_for_pane_command(TARGET, "main", "sleep");
    let uuid = target_uuid(rig, TARGET);
    succeeds(&run(rig, None, &["stop", TARGET]));
    assert!(!is_live(rig, TARGET), "owner stopped the target");
    uuid
}

#[test]
fn bare_roster_invocation_from_another_seat_leaves_roster_stopped() {
    let (rig, caller) = fixture("sg-roster");
    let uuid = stopped_target(&rig);
    let meta = rig.meta(TARGET);
    let before = events(&rig.dir(TARGET));

    let out = run(&rig, Some(&caller.pane), &[TARGET]);
    refuses_cross(&out, TARGET);
    assert!(out.2.contains("ae roster"), "{}", words(&out));

    assert!(!is_live(&rig, TARGET), "roster stayed stopped");
    assert!(is_live(&rig, SOURCE), "caller stayed live");
    assert_eq!(rig.meta(TARGET), meta, "target meta stayed intact");
    assert_eq!(events(&rig.dir(TARGET)), before, "no target journal write");

    let rows = events(&caller.dir);
    let refusals = rows
        .iter()
        .filter(|row| row.action == "launch-refused")
        .collect::<Vec<_>>();
    let [refusal] = refusals.as_slice() else {
        panic!("one refusal in the calling session: {rows:?}");
    };
    assert_eq!(refusal.actor, "lead", "{refusal:?}");
    assert_eq!(refusal.target.as_deref(), Some(TARGET), "{refusal:?}");
    assert_eq!(refusal.actor_slot.value(), Some("main"), "{refusal:?}");
    assert_eq!(refusal.actor_session.value(), Some(SOURCE), "{refusal:?}");
    assert_eq!(refusal.target_session.value(), Some(TARGET), "{refusal:?}");
    assert_eq!(
        refusal.target_session_uuid.as_deref(),
        Some(uuid.as_str()),
        "{refusal:?}"
    );
    assert_eq!(refusal.caller_pane.as_deref(), Some(caller.pane.as_str()));
    assert_eq!(
        refusal.caller_session_uuid.as_deref(),
        Some(uuid_of(SOURCE).as_str())
    );
}

#[test]
fn a_partial_seat_still_refuses_when_its_own_journal_cannot_be_proved() {
    let (rig, caller) = fixture("sg-partial");
    assert!(
        rig.tmux(&["set-option", "-p", "-u", "-t", &caller.pane, "@ae_slot"])
            .0,
        "removed only the slot stamp"
    );
    refuses_cross(
        &run(&rig, Some(&caller.pane), &[FRESH, "--no-attach"]),
        FRESH,
    );
    let before = events(&caller.dir);
    assert_eq!(before.len(), 1, "{before:?}");
    assert_eq!(before[0].actor, "lead", "{before:?}");
    assert_eq!(before[0].action, "launch-refused", "{before:?}");

    let meta = rig.meta(SOURCE);
    fs::write(
        caller.dir.join("meta"),
        format!("{meta}session_id={}\n", uuid_of(SOURCE)),
    )
    .expect("caller record now has two session ids");

    let out = run(&rig, Some(&caller.pane), &[FRESH, "--no-attach"]);
    refuses_cross(&out, FRESH);
    assert!(
        out.2.contains("refusal could not be journaled"),
        "{}",
        words(&out)
    );
    assert_eq!(events(&caller.dir), before, "no append to damaged record");
    assert!(!is_live(&rig, FRESH) && !rig.dir(FRESH).exists());
}

#[test]
fn dispatcher_start_is_attributed_in_the_target_and_dispatch_journals() {
    let (rig, _) = fixture("sg-dispatch");
    let uuid = stopped_target(&rig);
    let world = ManuallyDrop::new(World {
        root: rig.home.clone(),
        sock: rig.sock.clone(),
    });
    let overlay = rig.home.join("orchestrator.config");
    fs::write(
        &overlay,
        include_str!("../../contrib/aeorchestrator/orchestrator.config"),
    )
    .expect("current dispatcher overlay");
    let seat = world.session(
        "orchestrator",
        "boss",
        &format!("meta_agent=true\nlocal_config={}\n", overlay.display()),
    );
    let meta = rig.meta("orchestrator");
    fs::write(
        seat.dir.join("meta"),
        format!("{meta}main_pane={}\n", seat.pane),
    )
    .expect("recorded dispatcher main");

    succeeds(&run(&rig, Some(&seat.pane), &[TARGET, "--no-attach"]));
    assert!(is_live(&rig, TARGET));
    let journal = events(&seat.dir);
    let request = journal
        .iter()
        .find(|row| row.action == "dispatch-request")
        .expect("S1 dispatcher request");
    assert!(request.reference.as_ref().is_some_and(|id| !id.is_empty()));
    assert!(
        journal
            .iter()
            .any(|row| { row.action == "dispatch-result" && row.reference == request.reference })
    );
    let rows = starts(&rig, TARGET);
    assert_eq!(rows.len(), 2, "{rows:?}");
    let resumed = &rows[1];
    names_start(resumed, TARGET, &uuid, "resumed");
    assert_eq!(resumed.actor, "ae/orchestrator", "{resumed:?}");
    assert_eq!(
        resumed.caller_pane.as_deref(),
        Some(seat.pane.as_str()),
        "{resumed:?}"
    );
    assert_eq!(resumed.reference, request.reference, "{resumed:?}");
}

#[test]
fn explicit_bound_and_served_resume_apply_the_same_seat_guard() {
    let (rig, caller) = fixture("sg-resume");
    let uuid = stopped_target(&rig);
    let meta = rig.meta(TARGET);
    let before = events(&rig.dir(TARGET));
    let bound = [TARGET, "--resume", "--no-attach", "--expect-uuid", &uuid];

    refuses_cross(
        &run(
            &rig,
            Some(&caller.pane),
            &[TARGET, "--resume", "--no-attach"],
        ),
        TARGET,
    );
    refuses_cross(&run(&rig, Some(&caller.pane), &bound), TARGET);

    let mut served = rig.launch_command_no_autostart(&[TARGET, "--resume"]);
    served.env("AE_HOME", &rig.home);
    from_pane(&mut served, &rig, &caller.pane);
    refuses_cross(&collect(served), TARGET);

    assert!(!is_live(&rig, TARGET), "roster stayed stopped");
    assert_eq!(rig.meta(TARGET), meta, "target meta stayed intact");
    assert_eq!(events(&rig.dir(TARGET)), before, "no target journal write");

    succeeds(&run(&rig, None, &bound));
    assert!(is_live(&rig, TARGET), "the owner's bound resume still runs");
}

#[test]
fn an_ordinary_seat_cannot_create_another_session() {
    let (rig, caller) = fixture("sg-create");
    let dir = rig.project.to_str().expect("private project path");
    let out = run(
        &rig,
        Some(&caller.pane),
        &[FRESH, "--dir", dir, "--copy", "--no-attach"],
    );
    refuses_cross(&out, FRESH);
    assert!(!is_live(&rig, FRESH), "no tmux session");
    assert!(!rig.dir(FRESH).exists(), "no state directory");
    assert!(
        !rig.home.join("worktrees").join(FRESH).exists(),
        "no working copy"
    );
}

#[test]
fn a_rename_alias_of_the_callers_session_is_a_distinct_create_target() {
    const ALIAS: &str = "sg-source-old";
    let (rig, caller) = fixture("sg-alias");
    let alias = rig.dir(ALIAS);
    std::os::unix::fs::symlink(SOURCE, &alias).expect("private rename alias");
    let before = rig.meta(SOURCE);

    refuses_cross(
        &run(&rig, Some(&caller.pane), &[ALIAS, "--no-attach"]),
        ALIAS,
    );
    assert_eq!(
        fs::read_link(alias).expect("alias was not reclaimed"),
        Path::new(SOURCE)
    );
    assert!(!is_live(&rig, ALIAS), "no session for the alias name");
    assert!(is_live(&rig, SOURCE), "the alias destination stayed live");
    assert_eq!(rig.meta(SOURCE), before);
}

#[test]
fn an_ordinary_seat_can_reattach_its_own_session_only() {
    let (rig, caller) = fixture("sg-live");
    succeeds(&run(&rig, None, &[TARGET, "--local", "--no-attach"]));
    let target = events(&rig.dir(TARGET));

    refuses_cross(
        &run(&rig, Some(&caller.pane), &[TARGET, "--no-attach"]),
        TARGET,
    );
    assert_eq!(events(&rig.dir(TARGET)), target);
    assert!(is_live(&rig, TARGET), "the running target remains live");

    succeeds(&run(&rig, Some(&caller.pane), &[SOURCE, "--no-attach"]));
    assert!(
        starts(&rig, SOURCE).is_empty(),
        "own running session was not started"
    );

    let different_uuid = uuid_of("sg-different");
    let (ok, _) = rig.tmux(&[
        "set-option",
        "-t",
        &caller.pane,
        "@ae_session_uuid",
        &different_uuid,
    ]);
    assert!(ok, "replace only the live UUID");
    let (ok, live_uuid) = rig.tmux(&["show-option", "-v", "-t", &caller.pane, "@ae_session_uuid"]);
    assert!(ok, "read the changed UUID");
    assert_eq!(live_uuid.trim(), different_uuid);
    let meta = rig.meta(SOURCE);
    let journal = events(&caller.dir);

    refuses_cross(
        &run(&rig, Some(&caller.pane), &[SOURCE, "--no-attach"]),
        SOURCE,
    );
    assert!(is_live(&rig, SOURCE), "running session stayed live");
    assert_eq!(rig.meta(SOURCE), meta);
    assert_eq!(
        events(&caller.dir),
        journal,
        "the caller no longer correlates"
    );
}

fn starts(rig: &Rig, name: &str) -> Vec<Event> {
    events(&rig.dir(name))
        .into_iter()
        .filter(|row| row.action == "session-start")
        .collect()
}

fn names_start(row: &Event, target: &str, uuid: &str, how: &str) {
    assert_eq!(row.target.as_deref(), Some(target), "{row:?}");
    assert_eq!(row.target_session.value(), Some(target), "{row:?}");
    assert_eq!(row.target_session_uuid.as_deref(), Some(uuid), "{row:?}");
    assert!(
        row.summary.as_deref().is_some_and(|s| s.starts_with(how)),
        "{row:?}"
    );
}

#[test]
fn owner_create_and_resume_record_the_actual_actor_and_caller_pane() {
    let (rig, _) = fixture("sg-owner");
    let uuid = stopped_target(&rig);
    let created = starts(&rig, TARGET);
    assert_eq!(created.len(), 1, "{created:?}");
    names_start(&created[0], TARGET, &uuid, "created");
    assert_eq!(created[0].actor, "human");
    assert_eq!(created[0].caller_pane, None, "{created:?}");
    assert_eq!(created[0].caller_server, None, "{created:?}");
    assert_eq!(created[0].caller_session_uuid, None, "{created:?}");

    let (ok, pane) = rig.tmux(&[
        "new-window",
        "-d",
        "-t",
        "assistant:",
        "-P",
        "-F",
        "#{pane_id}",
        "exec sleep 300",
    ]);
    let pane = pane.trim();
    assert!(
        ok && pane.starts_with('%'),
        "private unstamped pane: {pane}"
    );
    succeeds(&run(&rig, Some(pane), &[TARGET, "--resume", "--no-attach"]));
    assert!(is_live(&rig, TARGET));

    let started = starts(&rig, TARGET);
    assert_eq!(started.len(), 2, "{started:?}");
    assert_eq!(started[0], created[0]);
    let resumed = &started[1];
    names_start(resumed, TARGET, &uuid, "resumed");
    assert_eq!(resumed.actor, "ae:launch", "{resumed:?}");
    assert_eq!(resumed.caller_pane.as_deref(), Some(pane), "{resumed:?}");
    assert_eq!(
        resumed.caller_session_uuid.as_deref(),
        Some(uuid_of(SOURCE).as_str())
    );
    assert_eq!(
        resumed
            .caller_server
            .as_deref()
            .and_then(|s| fs::canonicalize(s).ok()),
        Some(fs::canonicalize(&rig.sock).expect("private socket")),
        "{resumed:?}"
    );

    succeeds(&run(&rig, Some(pane), &[TARGET, "--no-attach"]));
    assert_eq!(starts(&rig, TARGET), started, "reattach adds no row");
}

#[test]
fn an_observed_vacant_pane_is_recorded_even_without_a_session_uuid() {
    let (rig, _) = fixture("sg-vacant");
    let (ok, pane) = rig.tmux(&[
        "new-session",
        "-d",
        "-s",
        "plain-shell",
        "-P",
        "-F",
        "#{pane_id}",
        "exec sleep 300",
    ]);
    let pane = pane.trim();
    assert!(ok && pane.starts_with('%'), "private vacant pane: {pane}");

    succeeds(&run(&rig, Some(pane), &[TARGET, "--local", "--no-attach"]));
    let rows = starts(&rig, TARGET);
    let [started] = rows.as_slice() else {
        panic!("one start event: {rows:?}");
    };
    assert_eq!(started.actor, "ae:launch", "{started:?}");
    assert_eq!(started.caller_pane.as_deref(), Some(pane), "{started:?}");
    assert_eq!(started.caller_session_uuid, None, "{started:?}");
}

#[test]
fn a_start_with_unwritable_attribution_does_not_leave_the_session_running() {
    let (rig, _) = fixture("sg-journal");
    let _ = stopped_target(&rig);
    let ledger = rig.dir(TARGET).join("events.jsonl");
    let saved = rig.dir(TARGET).join("saved-events.jsonl");
    fs::rename(&ledger, &saved).expect("preserve existing events");
    fs::create_dir(&ledger).expect("journal append now fails");
    let before = fs::read(&saved).expect("saved events");

    let out = run(&rig, None, &[TARGET, "--resume", "--no-attach"]);
    let shown = words(&out);
    assert_eq!(out.0, Some(1), "{shown}");
    assert!(
        out.2.contains("journal") || out.2.contains("events.jsonl"),
        "{shown}"
    );
    assert!(!is_live(&rig, TARGET), "unaudited resume left no session");
    assert_eq!(fs::read(&saved).expect("saved events"), before);
}
