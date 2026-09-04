//! The real transport, against real tmux servers.
//!
//! `src/transport.rs`'s unit tests pin what can be proven without one: that the
//! exec can succeed at all, that a program which cannot be spawned is a FAILED
//! run rather than an empty one, and that an unaddressable socket never reaches
//! the wire. What is here needs a server that actually answers — because the
//! fact this slice added is that ae's answer now comes from one.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::fs;
use std::path::{Path, PathBuf};

use ae::inventory::{DiscoveredSession, Discovery, QueryFailed, ServerId};
use ae::liveness::Snapshot;
use ae::meta::Selector;
use ae::tmux::{interpret_panes, list_panes_args};
use ae::transport::Tmux;

use super::parity::{Invocation, capture::raw};
use super::phase2::{run_tmux, tmux_present};

/// A short-lived scratch directory, short enough to hold a socket path.
fn scratch(tag: &str) -> PathBuf {
    let dir = PathBuf::from(format!("/tmp/ae-tr-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    assert!(fs::create_dir_all(&dir).is_ok(), "a short scratch dir");
    dir
}

/// Kill the test's private tmux server even when an assertion unwinds before
/// the arm's ordinary teardown reaches its explicit kill.
struct Cleanup {
    socket: PathBuf,
    scratch: PathBuf,
}

impl Cleanup {
    fn new(socket: &Path, scratch: &Path) -> Self {
        Self {
            socket: socket.to_owned(),
            scratch: scratch.to_owned(),
        }
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let out = self.scratch.join("cleanup-out");
        let err = self.scratch.join("cleanup-err");
        let invocation = Invocation::new("tmux")
            .arg("-S")
            .arg(&self.socket)
            .arg("kill-server");
        let _ = raw::run(&invocation, &self.scratch, &out, &err);
        let _ = fs::remove_dir_all(&self.scratch);
    }
}

/// A durable session record naming `socket` as its server.
fn plant(root: &Path, name: &str, socket: &Path) {
    let dir = root.join("sessions").join(name);
    let written = fs::create_dir_all(&dir).and_then(|()| {
        fs::write(
            dir.join("meta"),
            format!(
                "mode=local\nagent.main=cl:lead\ntmux_server_kind=socket\ntmux_server={}\n",
                socket.display()
            ),
        )
    });
    assert!(written.is_ok(), "a planted session");
}

/// What the snapshot decided about `name`.
fn status_of(snapshot: &Snapshot, name: &str) -> &'static str {
    snapshot
        .sessions
        .iter()
        .find(|classified| classified.candidate.name == name)
        .map_or_else(
            || panic!("no candidate named {name} in the snapshot"),
            |classified| classified.status.as_str(),
        )
}

/// Start an isolated server on `socket` with `sessions`, each `(name, marker)`.
fn start_server(socket: &Path, scratch: &Path, sessions: &[(&str, Option<&str>)]) {
    let server = ServerId::Selected(Selector::Socket(socket.to_path_buf()));
    for (name, marker) in sessions {
        let mut create = ae::tmux::server_args(&server);
        create.extend(["new-session", "-d", "-s", name].map(ToOwned::to_owned));
        if let Some(marker) = marker {
            create.push("-e".to_owned());
            create.push(format!("AE_SESSION={marker}"));
        }
        let (created, _) = run_tmux(&create, scratch);
        assert!(created, "creating {name} must succeed");
    }
}

/// Kill the server on `socket`, whatever state the test left it in.
fn kill_server(socket: &Path, scratch: &Path) {
    let server = ServerId::Selected(Selector::Socket(socket.to_path_buf()));
    let mut kill = ae::tmux::server_args(&server);
    kill.push("kill-server".to_owned());
    let _ = run_tmux(&kill, scratch);
}

/// Fail loudly when tmux is absent rather than passing without proving anything.
fn require_tmux(scratch: &Path) {
    if !tmux_present(scratch) {
        let _ = fs::remove_dir_all(scratch);
        panic!(
            "tmux is not runnable here, so the transport's answering arm cannot be proven; \
             install tmux or run this suite where one exists"
        );
    }
}

#[test]
fn sc_017k_one_real_query_answers_present_absent_and_prefix_candidates_by_exact_name() {
    // THE ARM THAT MAKES EVERY `unknown` ASSERTION IN THIS SUITE MEAN SOMETHING,
    // and — since the prefix candidate joined it — the arm that proves each
    // answer is bound to the RIGHT candidate rather than merely being the right
    let scratch = scratch("live");
    require_tmux(&scratch);
    let socket = scratch.join("t.sock");
    let _cleanup = Cleanup::new(&socket, &scratch);
    start_server(&socket, &scratch, &[("alive", Some("alive"))]);

    let root = scratch.join("home");
    plant(&root, "alive", &socket);
    plant(&root, "gone", &socket);
    // A PROPER PREFIX of a session that IS there, recorded on that same server.
    plant(&root, "ali", &socket);

    // THE REAL ROUTE: this is the function `ae list` calls.
    let (snapshot, _world) = ae::current_world(&root);
    let alive = status_of(&snapshot, "alive");
    let gone = status_of(&snapshot, "gone");
    let ali = status_of(&snapshot, "ali");
    let complete = snapshot.complete();

    // Tear down BEFORE asserting, so a failure cannot leave a server behind.
    kill_server(&socket, &scratch);
    let _ = fs::remove_dir_all(&scratch);

    assert_eq!(
        alive, "running",
        "a session the server reported, with ae's marker on it"
    );
    assert_eq!(
        gone, "stopped",
        "and the SAME successful query proved this one absent — the only route to `stopped`"
    );
    assert_eq!(
        ali, "stopped",
        "a name that is a PREFIX of a live session is absent, not present: tmux would \
         have matched it to `alive`, and that substitution is issue #105 itself"
    );
    assert!(
        complete,
        "an entitled server that answered is not a failed source, so the \
         snapshot no longer claims it could not look everywhere"
    );
}

#[test]
fn sc_017l_a_socket_that_is_not_a_server_is_unknown_and_never_stopped() {
    // THE ARM THAT CATCHES A TRANSPORT WHICH TREATS FAILURE AS ABSENCE. Empty
    // output from a failed query and empty output from a live server with no
    // sessions are the same bytes; only the exit status tells them apart. A
    let scratch = scratch("dead");
    // SELF-STANDING, DELIBERATELY.
    require_tmux(&scratch);
    let missing = scratch.join("nothing-here.sock");
    let not_a_socket = scratch.join("a-plain-file.sock");
    assert!(
        fs::write(&not_a_socket, b"this is not a tmux server\n").is_ok(),
        "a file where a socket would be"
    );

    let root = scratch.join("home");
    plant(&root, "absent-path", &missing);
    plant(&root, "wrong-kind", &not_a_socket);

    let (snapshot, _world) = ae::current_world(&root);
    let absent_path = status_of(&snapshot, "absent-path");
    let wrong_kind = status_of(&snapshot, "wrong-kind");
    let complete = snapshot.complete();
    let _ = fs::remove_dir_all(&scratch);

    assert_eq!(
        absent_path, "unknown",
        "a socket path with no server behind it answers nothing, and nothing is not absence"
    );
    assert_eq!(
        wrong_kind, "unknown",
        "and neither does a path holding something that is not a server"
    );
    assert!(
        !complete,
        "an entitled server ae could not enumerate is a loss the snapshot reports"
    );
}

#[test]
fn sc_017l_a_session_the_server_reports_without_ae_s_marker_is_unknown() {
    // PRESENT BUT NOT PROVABLY AE'S.
    let scratch = scratch("owner");
    require_tmux(&scratch);
    let socket = scratch.join("t.sock");
    let _cleanup = Cleanup::new(&socket, &scratch);
    start_server(
        &socket,
        &scratch,
        &[
            ("unowned", None),
            ("mismatched", Some("some-other-session")),
            ("owned", Some("owned")),
        ],
    );

    let root = scratch.join("home");
    for name in ["unowned", "mismatched", "owned"] {
        plant(&root, name, &socket);
    }

    let (snapshot, _world) = ae::current_world(&root);
    let unowned = status_of(&snapshot, "unowned");
    let mismatched = status_of(&snapshot, "mismatched");
    let owned = status_of(&snapshot, "owned");

    kill_server(&socket, &scratch);
    let _ = fs::remove_dir_all(&scratch);

    assert_eq!(unowned, "unknown", "no marker is not proof of ownership");
    // A marker whose VALUE is some other string is still ae's tag, and the tag
    // is the whole claim it makes.
    assert_eq!(
        mismatched, "running",
        "the marker tags a session as ae's; it does not name one"
    );
    assert_eq!(
        owned, "running",
        "and the control on the same server still answers, or the other two prove nothing"
    );
}

#[test]
fn the_transport_reports_the_names_and_markers_the_server_holds() {
    // THE PORT'S OWN CONTRACT, asked directly rather than through three phases.
    let scratch = scratch("port");
    require_tmux(&scratch);
    let socket = scratch.join("t.sock");
    let _cleanup = Cleanup::new(&socket, &scratch);
    start_server(
        &socket,
        &scratch,
        &[("marked", Some("marked")), ("bare", None)],
    );

    let server = ServerId::Selected(Selector::Socket(socket.clone()));
    let mut answered = Tmux.enumerate(&server);
    if let Ok(sessions) = &mut answered {
        sessions.sort_by(|a, b| a.name.cmp(&b.name));
    }

    // A server that is not there, on the same transport and in the same test, so
    // the two outcomes are compared rather than merely both asserted.
    let absent = ServerId::Selected(Selector::Socket(scratch.join("no-such.sock")));
    let refused = Tmux.enumerate(&absent);

    kill_server(&socket, &scratch);
    let _ = fs::remove_dir_all(&scratch);

    assert_eq!(
        answered,
        Ok(vec![
            DiscoveredSession {
                name: "bare".to_owned(),
                marker: None,
            },
            DiscoveredSession {
                name: "marked".to_owned(),
                marker: Some("marked".to_owned()),
            },
        ]),
        "the transport reported the server's names, each with its own marker"
    );
    assert_eq!(
        refused,
        Err(QueryFailed),
        "and a server that is not there is a FAILED query, not an empty success"
    );
}

// ---- the PURE pane half, against a real server ---------------------------
//
// THIS IS NOT THE TRANSPORT'S TEST, and the distinction is deliberate. The

/// Give the pane at `target` the `@ae_slot` marker `slot`.
fn mark_pane(socket: &Path, scratch: &Path, target: &str, slot: &str) {
    let server = ServerId::Selected(Selector::Socket(socket.to_path_buf()));
    let mut args = ae::tmux::server_args(&server);
    args.extend(["set-option", "-p", "-t", target, "@ae_slot", slot].map(ToOwned::to_owned));
    let (marked, _) = run_tmux(&args, scratch);
    assert!(marked, "marking pane {target} must succeed");
}

/// Add a pane to `session`, optionally running `command` in it.
fn split(socket: &Path, scratch: &Path, session: &str, command: Option<&str>) -> String {
    let server = ServerId::Selected(Selector::Socket(socket.to_path_buf()));
    let mut args = ae::tmux::server_args(&server);
    args.extend(
        [
            "split-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            session,
        ]
        .map(ToOwned::to_owned),
    );
    if let Some(command) = command {
        args.push(command.to_owned());
    }
    let (made, out) = run_tmux(&args, scratch);
    assert!(made, "splitting {session} must succeed");
    let id = out.trim().to_owned();
    assert!(
        id.starts_with('%'),
        "tmux must report the new pane id, got {out:?}"
    );
    id
}

/// The pane id of a session that has exactly one pane.
fn only_pane_id(socket: &Path, scratch: &Path, session: &str) -> String {
    let server = ServerId::Selected(Selector::Socket(socket.to_path_buf()));
    let mut args = ae::tmux::server_args(&server);
    args.extend(["list-panes", "-s", "-t", session, "-F", "#{pane_id}"].map(ToOwned::to_owned));
    let (ok, out) = run_tmux(&args, scratch);
    assert!(ok, "listing panes of {session} must succeed");
    let ids: Vec<&str> = out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(ids.len(), 1, "expected a single-pane session, got {ids:?}");
    ids[0].to_owned()
}

/// Wait until `session` reports a dead pane, or fail loudly.
fn wait_for_a_dead_pane(socket: &Path, scratch: &Path, session: &str) {
    let server = ServerId::Selected(Selector::Socket(socket.to_path_buf()));
    let mut args = ae::tmux::server_args(&server);
    args.extend(["list-panes", "-s", "-t", session, "-F", "#{pane_dead}"].map(ToOwned::to_owned));
    for _ in 0..100 {
        let (ok, out) = run_tmux(&args, scratch);
        if ok && out.lines().any(|line| line.trim() == "1") {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("no pane ever reported pane_dead=1; the #109 arm cannot be proven");
}

/// Wait until `pane` reports `pane_current_command` == `command` with `pane_dead` == 0,
/// or fail loudly.
fn wait_for_pane_command(socket: &Path, scratch: &Path, pane: &str, command: &str) {
    let server = ServerId::Selected(Selector::Socket(socket.to_path_buf()));
    let mut args = ae::tmux::server_args(&server);
    args.extend(
        [
            "display-message",
            "-p",
            "-t",
            pane,
            "#{pane_dead}\t#{pane_current_command}",
        ]
        .map(ToOwned::to_owned),
    );
    for _ in 0..200 {
        let (ok, out) = run_tmux(&args, scratch);
        if ok && out.trim() == format!("0\t{command}") {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("pane {pane} never reported pane_current_command={command} with pane_dead=0");
}

/// Turn on `remain-on-exit`, so a pane whose process exits is RETAINED.
fn retain_exited_panes(socket: &Path, scratch: &Path) {
    let server = ServerId::Selected(Selector::Socket(socket.to_path_buf()));
    let mut args = ae::tmux::server_args(&server);
    args.extend(["set-option", "-g", "remain-on-exit", "on"].map(ToOwned::to_owned));
    let (set, _) = run_tmux(&args, scratch);
    assert!(set, "remain-on-exit must be settable");
}

#[test]
fn sc_017s_a_real_enumeration_carries_identity_and_both_liveness_conjuncts() {
    // WHAT THIS PROVES AGAINST A REAL SERVER, and why each arm is here.
    let scratch = scratch("panes");
    require_tmux(&scratch);
    let socket = scratch.join("t.sock");
    let _cleanup = Cleanup::new(&socket, &scratch);
    start_server(&socket, &scratch, &[("marked", Some("marked"))]);
    // Captured BEFORE any split, while "the only pane" is a checkable fact.
    let original = only_pane_id(&socket, &scratch, "marked");
    retain_exited_panes(&socket, &scratch);
    let unmarked = split(&socket, &scratch, "marked", None);
    let exited = split(&socket, &scratch, "marked", Some("true"));
    assert_ne!(
        unmarked, exited,
        "the fixture needs two distinct extra panes"
    );
    mark_pane(&socket, &scratch, &original, "main");
    mark_pane(&socket, &scratch, &exited, "gone");
    wait_for_a_dead_pane(&socket, &scratch, "marked");

    let server = ServerId::Selected(Selector::Socket(socket.clone()));
    let (ok, out) = run_tmux(&list_panes_args(&server, "marked"), &scratch);
    let panes = interpret_panes(ok, &out);

    // The platform fact, asked of a real server because no assertion over the
    // parser can establish it: unset and set-to-empty are indistinguishable.
    mark_pane(&socket, &scratch, &unmarked, "");
    let (empty_ok, empty_out) = run_tmux(&list_panes_args(&server, "marked"), &scratch);
    let after_setting_empty = interpret_panes(empty_ok, &empty_out);

    // Prefix hazard and failed query, on the same primitive and the same run.
    let (prefix_ok, prefix_out) = run_tmux(&list_panes_args(&server, "marke"), &scratch);
    let by_prefix = interpret_panes(prefix_ok, &prefix_out);
    let (absent_ok, absent_out) = run_tmux(&list_panes_args(&server, "nosuch"), &scratch);
    let absent = interpret_panes(absent_ok, &absent_out);

    kill_server(&socket, &scratch);
    let _ = fs::remove_dir_all(&scratch);

    let panes = panes.expect("a live session enumerates");
    assert_eq!(
        panes.len(),
        3,
        "three panes, and the unmarked one is one of them"
    );

    // FOUND BY IDENTITY, NEVER BY POSITION — see `split`'s docs for why.
    let by_slot = |slot: &str| {
        panes
            .iter()
            .find(|pane| pane.slot.as_deref() == Some(slot))
            .unwrap_or_else(|| panic!("no pane carries slot {slot}: {out:?}"))
    };
    let main = by_slot("main");
    let gone = by_slot("gone");
    assert_eq!(
        panes.iter().filter(|pane| pane.slot.is_none()).count(),
        1,
        "exactly one pane is present and anonymous: {out:?}"
    );

    // THE #109 ARM.
    assert_eq!(
        gone.dead,
        Some(true),
        "an exited pane reports pane_dead=1: {out:?}"
    );
    assert_eq!(
        gone.command.as_deref(),
        Some("true"),
        "and still reports the EXITED process's command, which is not a shell: {out:?}"
    );
    assert_eq!(
        main.dead,
        Some(false),
        "while a live pane reports pane_dead=0, or the arm above proves nothing"
    );
    // The live panes' command is the machine's login shell and is deliberately
    // NOT asserted by name: it differs across machines, and pinning it would
    // make this test a statement about the developer's shell.
    assert!(
        main.command.is_some(),
        "a live pane reports SOME command: {out:?}"
    );

    assert_eq!(
        after_setting_empty
            .as_ref()
            .map(|panes| panes.iter().filter(|pane| pane.slot.is_none()).count()),
        Ok(1),
        "a marker SET to the empty string reads exactly like an unset one — still \
         exactly one anonymous pane, not two and not none"
    );
    assert_eq!(
        by_prefix.map(|panes| panes.len()),
        Ok(3),
        "tmux answered a PREFIX target with this session's panes — measured, and \
         the reason the caller must supply an exact name"
    );
    assert_eq!(
        absent,
        Err(QueryFailed),
        "a target naming no session is a failed query, never an empty enumeration"
    );
}

/// A durable record with a two-seat roster, so the runtime has slots to answer.
fn plant_roster(root: &Path, name: &str, socket: &Path) {
    let dir = root.join("sessions").join(name);
    let written = fs::create_dir_all(&dir).and_then(|()| {
        fs::write(
            dir.join("meta"),
            format!(
                "mode=local\nseat.main=lead\nprofile.main=cl\nseat.worker.0=hand\n\
                 profile.worker.0=cl\ntmux_server_kind=socket\ntmux_server={}\n",
                socket.display()
            ),
        )
    });
    assert!(written.is_ok(), "a planted session with a roster");
}

/// Publish the watchdog's machine-value branch option onto `session`.
fn publish_branch(socket: &Path, scratch: &Path, session: &str, branch: &str) {
    let server = ServerId::Selected(Selector::Socket(socket.to_path_buf()));
    let mut args = ae::tmux::server_args(&server);
    args.extend(["set-option", "-t", session, "@ae_branch_name", branch].map(ToOwned::to_owned));
    let (set, _) = run_tmux(&args, scratch);
    assert!(set, "publishing the branch option must succeed");
}

#[test]
fn sc_017p_the_list_route_answers_each_seat_from_the_live_panes_and_the_published_branch() {
    // THE SLICE'S OWN ARM. Everything before it proved the SESSION status comes
    // from a real server; nothing proved that the per-agent liveness and the
    // live branch `ae list` prints do. Both were `null` for every agent of every
    let scratch = scratch("runtime");
    require_tmux(&scratch);
    let socket = scratch.join("t.sock");
    let _cleanup = Cleanup::new(&socket, &scratch);
    start_server(&socket, &scratch, &[("live", Some("live"))]);
    // The session's own first pane runs the login SHELL, which the reader puts in
    // the not-alive set — so the alive arm needs a pane running something else.
    let shell = only_pane_id(&socket, &scratch, "live");
    mark_pane(&socket, &scratch, &shell, "spawned.0");
    let agent = split(&socket, &scratch, "live", Some("sleep 60"));
    mark_pane(&socket, &scratch, &agent, "main");
    wait_for_pane_command(&socket, &scratch, &agent, "sleep");
    publish_branch(&socket, &scratch, "live", "feature/runtime");

    let root = scratch.join("home");
    plant_roster(&root, "live", &socket);

    // THE REAL ROUTE: the function `ae list` calls.
    let (_snapshot, world) = ae::current_world(&root);
    let entry = world
        .sessions
        .iter()
        .find(|entry| entry.name == "live")
        .cloned();

    // Tear down BEFORE asserting, so a failure cannot leave a server behind.
    kill_server(&socket, &scratch);
    let _ = fs::remove_dir_all(&scratch);

    let entry = entry.expect("the planted session reaches the world");
    let seat = |reference: &str| {
        entry
            .agents
            .iter()
            .find(|agent| agent.reference == reference)
            .unwrap_or_else(|| panic!("no seat {reference} in {:?}", entry.agents))
    };
    assert_eq!(
        seat("lead").alive,
        Some(true),
        "the marked pane runs a real command, so its seat is alive"
    );
    assert_eq!(
        seat("hand").alive,
        Some(false),
        "no pane carries worker.0 and every pane is identified — a complete          enumeration that excludes the slot"
    );
    assert_eq!(
        seat("hand").reason,
        Some(ae::attention::Reason::Dead),
        "and a vanished pane is the frozen rollup's `dead`"
    );
    assert_eq!(
        entry.attention,
        Some(ae::attention::Reason::Dead),
        "which rolls up to the session marker a human reads as attn:dead"
    );
    assert_eq!(
        entry.branch.as_deref(),
        Some("feature/runtime"),
        "the branch is the watchdog's published option, not a git call against \
         a work tree this fixture does not have"
    );
}
