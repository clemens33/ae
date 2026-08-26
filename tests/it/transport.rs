//! The real transport, against real tmux servers.
//!
//! `src/transport.rs`'s unit tests pin what can be proven without one: that the
//! exec can succeed at all, that a program which cannot be spawned is a FAILED
//! run rather than an empty one, and that an unaddressable socket never reaches
//! the wire. What is here needs a server that actually answers — because the
//! fact this slice added is that ae's answer now comes from one.
//!
//! # What each arm is for
//!
//! SC-017k grants `running`/`stopped` only to a SUCCESSFUL query; SC-017l sends
//! every failure to `unknown`. Those two rows are only distinguishable with BOTH
//! arms present:
//!
//! * a server that ANSWERS, so `running` and `stopped` are reachable at all —
//!   without it, "every session is unknown" passes every unknown assertion in
//!   the suite while ae has silently stopped being able to look;
//! * a server that does NOT answer, so a transport which mistook silence for
//!   absence would be caught — that transport reports `stopped`, and `stopped`
//!   is ae asserting a session is gone on the strength of a question that got no
//!   answer.
//!
//! Neither is worth much alone. The pair is the test.
//!
//! Everything runs through `ae::current_world` — the function `ae list` itself
//! calls — rather than through a hand-assembled inventory, so what is observed
//! is the route the product takes.

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

use super::phase2::{run_tmux, tmux_present};

/// A short-lived scratch directory, short enough to hold a socket path.
///
/// `sun_path` is 104 bytes on macOS and the usual temp dir eats most of it, so
/// this is `/tmp` directly — the same reason `phase2.rs`'s real-server arms do
/// it. Nextest gives every test its own process, so the pid keeps two of these
/// from colliding.
fn scratch(tag: &str) -> PathBuf {
    let dir = PathBuf::from(format!("/tmp/ae-tr-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    assert!(fs::create_dir_all(&dir).is_ok(), "a short scratch dir");
    dir
}

/// A durable session record naming `socket` as its server.
///
/// A POSITIVE selector, which is load-bearing: SC-405l normalizes a missing one
/// to `missing`, and the classifier then never queries anything at all — so a
/// fixture without it cannot tell a working transport from a broken one.
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
    // set of answers. ONE server, ONE query, THREE candidates recorded on it,
    // asserted PER NAME. A transport returning the correct statuses attached to
    // the wrong sessions dies here by name.
    //
    // WHY `ali` IS THE WHOLE POINT AND NOT A FOURTH VARIATION. tmux's `-t`
    // PREFIX-MATCHES: `has-session -t ali` SUCCEEDS when only `alive` exists,
    // and reading that as "ali is running" is not a neighbour of issue #105, it
    // IS #105. ae never asks tmux whether a name exists — `enumerate` reads a
    // marker only for names `list-sessions` already returned, and the exact
    // match happens on ae's side in `liveness` — so this is expected to hold.
    // That expectation was traced by READING before this arm existed, and a
    // mechanism verified by reading is not a mechanism under test.
    //
    // IT HAD TO BE TESTED HERE. A fake backend cannot exhibit a prefix sibling,
    // so no amount of classifier testing reaches it; this slice is the first
    // code where the real adapter can get it wrong. And the earlier fixture
    // population could not have caught it at any size: alive, gone, unowned,
    // mismatched, owned, marked, bare — no name a prefix of another, so the
    // defect was excluded BY VOCABULARY rather than by assertion, and adding
    // arms over those names would have raised confidence without raising
    // coverage.
    let scratch = scratch("live");
    require_tmux(&scratch);
    let socket = scratch.join("t.sock");
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
        "SC-017o: an entitled server that answered is not a failed source, so the \
         snapshot no longer claims it could not look everywhere"
    );
}

#[test]
fn sc_017l_a_socket_that_is_not_a_server_is_unknown_and_never_stopped() {
    // THE ARM THAT CATCHES A TRANSPORT WHICH TREATS FAILURE AS ABSENCE. Empty
    // output from a failed query and empty output from a live server with no
    // sessions are the same bytes; only the exit status tells them apart. A
    // transport that dropped it would report every session here `stopped` — ae
    // asserting they are gone on the strength of a question that got no answer.
    //
    // Two shapes, because "not a server" has more than one: nothing at the path
    // at all, and something at the path that is not a socket. Both are failures
    // and neither is absence.
    //
    // The opposed control is the test above: it proves this same route CAN say
    // `running` and `stopped`, so `unknown` here is a decision rather than the
    // only answer the code is capable of.
    let scratch = scratch("dead");
    // SELF-STANDING, DELIBERATELY. Without this, a machine with no tmux still
    // passes — but by the SPAWN-FAILURE path, not the completed-non-zero one
    // this arm names. Both are failures and both yield `unknown`, so the test
    // stays green while silently testing a different mechanism than its name
    // claims. A test that degrades into a neighbouring case without saying so
    // is worse than one that is skipped loudly.
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
        "SC-017o: an entitled server ae could not enumerate is a loss the snapshot reports"
    );
}

#[test]
fn sc_017l_a_session_the_server_reports_without_ae_s_marker_is_unknown() {
    // PRESENT BUT NOT PROVABLY AE'S. The query SUCCEEDED and the exact name came
    // back, so `stopped` is off the table — the session is demonstrably there.
    // What is missing is ownership evidence, and SC-017l names both ways it can
    // be missing: no marker at all, and a marker naming something else. Neither
    // supports `running`, and guessing either way is the direction that asserts.
    let scratch = scratch("owner");
    require_tmux(&scratch);
    let socket = scratch.join("t.sock");
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
    // is the whole claim it makes. This session's identity was already settled
    // by the exact name match against its own recorded server, so re-deriving
    // it from the variable added nothing — and demanding it made `running`
    // unreachable for every real session, which is what shipped.
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
    // `Discovery::enumerate` promises every session the server reports WITH the
    // marker that server holds for it — two queries per name, not one, because
    // the enumeration cannot see a session's own environment.
    let scratch = scratch("port");
    require_tmux(&scratch);
    let socket = scratch.join("t.sock");
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

// ---- SC-017p/SC-017q: the PURE pane half, against a real server -----------
//
// THIS IS NOT THE TRANSPORT'S TEST, and the distinction is deliberate. The
// product transport has no pane query and must not grow one this slice: no
// ratified row defines "positively recognizes its agent process as live", so
// there is no liveness verdict to wire a pane observation to, and a seam built
// toward an unratified predicate is a decision rather than preparation.
//
// What IS ratified is the derivation and the reading, so that is what these
// prove — through the harness's pinned door, exactly as `phase2.rs`'s criterion
// 20 proves the session half. The value is that tmux's real output is checked
// against what `src/tmux.rs` assumes, rather than against what its author
// believed while writing the parser.

/// Give the pane at `target` the `@ae_slot` marker `slot`.
///
/// `target` is a PANE ID (`%N`), never an index — see [`split`].
fn mark_pane(socket: &Path, scratch: &Path, target: &str, slot: &str) {
    let server = ServerId::Selected(Selector::Socket(socket.to_path_buf()));
    let mut args = ae::tmux::server_args(&server);
    args.extend(["set-option", "-p", "-t", target, "@ae_slot", slot].map(ToOwned::to_owned));
    let (marked, _) = run_tmux(&args, scratch);
    assert!(marked, "marking pane {target} must succeed");
}

/// Add a pane to `session`, optionally running `command` in it.
///
/// Returns the new pane's ID (`%N`), asked of tmux rather than assumed.
///
/// PANE INDICES ARE NOT IDENTIFIERS, measured twice on the way here. First: a
/// split inserts relative to the ACTIVE pane, so creation order is not index
/// order, and hard-coded indices marked the wrong pane. Then: two consecutive
/// `split-window -d` calls BOTH reported index 1, because inserting renumbers
/// the panes after it — so even an index captured at creation is stale by the
/// next split. `pane_id` is unique and stable for the pane's lifetime; index is
/// a position, and positions move.
///
/// Both bugs were invisible as fixture bugs: the first surfaced as an assertion
/// about an exited pane reading a live one, which looks exactly like a product
/// defect in the field this slice exists to add.
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
///
/// NIT 1, and it was MY OWN LESSON surviving in my own fixture: after teaching
/// `split` to return a pane id because indices are not identifiers, this test
/// still reached the first pane as `marked.0`. That spelling is correct only
/// while nothing has been inserted before it — which is exactly the assumption
/// two measured bugs already broke. Asked of tmux, and asserted to be the only
/// pane, so it cannot silently become the wrong one.
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
///
/// BOUNDED POLLING, NOT A SLEEP. The pane's process exits on its own schedule;
/// a fixed delay is a guess at how long that takes, and the failure mode of a
/// guess that is too short is a flaky test that blames the product.
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

/// Turn on `remain-on-exit`, so a pane whose process exits is RETAINED.
///
/// Neither ae nor the successor sets this; SC-017s records it as
/// operator-configurable. It is what makes an exited pane observable at all,
/// and therefore what makes #109 reproducible instead of theoretical.
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
    //
    // The parser's correctness turns on tmux's actual bytes, not on what its
    // author believed while writing it. Three facts are checked against a live
    // server rather than a fixture: an unmarked pane arrives as an EMPTY MIDDLE
    // FIELD (not a dropped line), a marker SET to the empty string is
    // indistinguishable from unset, and — the one that matters — AN EXITED PANE
    // REPORTS A NON-SHELL COMMAND.
    //
    // That last one is #109. A `remain-on-exit` pane whose process has exited
    // keeps reporting the exited process's command, so `pane_current_command`
    // ALONE says a dead agent is alive. Only `pane_dead` separates them, and
    // this asserts the read carries it.
    let scratch = scratch("panes");
    require_tmux(&scratch);
    let socket = scratch.join("t.sock");
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

    // THE #109 ARM. The exited pane's command is NOT in SC-017s's shell set, so
    // the command field alone would read alive; `pane_dead` is the only thing
    // that says otherwise, and it survived the read.
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
