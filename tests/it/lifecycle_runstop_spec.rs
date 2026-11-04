//! Frozen phase-1 characterisation of `lifecycle::run_stop` on c1f4549f.
//!
//! Oracle: docs/reference/commands.md's stop guarantees where stated; exact
//! grammar/mode precedence below is labelled BASELINE BEHAVIOUR, not a new
//! design requirement. Invoke `_stop` directly: public `stop all` has an
//! earlier dispatcher. Every server and directory belongs to the parent rig.

use super::*;

fn words(tail: &[&str]) -> Vec<String> {
    tail.iter().map(|word| (*word).to_owned()).collect()
}

fn run(rig: &Rig, tail: &[String], inside: bool) -> (Option<i32>, String, String) {
    let mut argv = vec!["_stop"];
    argv.extend(tail.iter().map(String::as_str));
    if inside {
        argv.extend(["--pane", &rig.pane]);
        rig.run_inside(&argv)
    } else {
        rig.run(&argv)
    }
}

fn refused(rig: &Rig, tail: &[String], inside: bool, code: i32, err: &str) {
    let before = guard_spec::tree(&rig.dir);
    let got = run(rig, tail, inside);
    assert_eq!(got, (Some(code), String::new(), err.to_owned()), "{tail:?}");
    assert_eq!(
        guard_spec::tree(&rig.dir),
        before,
        "{tail:?}: no state writes"
    );
    assert!(rig.session_is_live(), "{tail:?}: original session survives");
}

const SELF_ALL: &str =
    "Error: --self names the session you are in — it cannot be combined with 'all'.\n";
const EXPECT_PUBLIC: &str =
    "Error: --expect-* describes one supervised stop and belongs with --supervise.\n";

/// BASELINE BEHAVIOUR: expectations are lifted before general argv, pane
/// pairs before expectations, ordinary argv refusals left to right.
#[test]
fn runstop_grammar_refusal_precedence_table() {
    let rig = Rig::new("rsgrammar");
    let unknown = "Error: unknown flag '--bogus'. Use -y/--yes or --self.\n";
    let extra =
        "Error: unexpected extra argument 'extra' — _stop takes one session name (or 'all').\n";
    let bad_name = "ae: 'bad/name' is not a usable session name.\n";
    let cases = [
        (words(&["--bogus", &rig.name, "extra"]), 2, unknown),
        (words(&[&rig.name, "extra", "--bogus"]), 2, extra),
        (
            words(&["--bogus", "--expect-uuid", UUID]),
            2,
            "Error: an --expect-* identity is incomplete; all ten fields travel together.\n",
        ),
        (
            words(&["--bogus", "--expect-uuid", UUID, "--expect-uuid", UUID]),
            2,
            "Error: --expect-uuid may be given only once.\n",
        ),
        (
            words(&["bad/name", "--expect-uuid"]),
            2,
            "Error: --expect-uuid requires a value.\n",
        ),
        (words(&["bad/name", "--pane", "--bogus"]), 1, bad_name),
        (words(&["bad/name", "--pane", "--expect-uuid"]), 1, bad_name),
        (words(&["bad/name", "--pane"]), 1, bad_name),
        (words(&["bad/name", "--yes", "-y", "--yes"]), 1, bad_name),
    ];
    for (tail, code, err) in cases {
        refused(&rig, &tail, false, code, err);
    }
}

/// BASELINE BEHAVIOUR: missing target is answered before mode dispatch and
/// before rejecting an otherwise complete expectation on a public form.
#[test]
fn runstop_missing_target_precedes_modes_and_public_expectation() {
    let rig = Rig::new("rsnotarget");
    let ordinary = stop_gap("no usable tmux socket marker ($TMUX is unset or names no socket)");
    let self_gap = "Error: --self with no session name needs a pane ae can resolve (--pane <id>); this one is not an ae agent pane.\n";
    for mode in [None, Some("--supervise"), Some("--handoff")] {
        let mut tail: Vec<String> = mode.into_iter().map(ToOwned::to_owned).collect();
        refused(&rig, &tail, false, 2, &ordinary);
        tail.push("--self".to_owned());
        refused(&rig, &tail, false, 1, self_gap);
    }
    let tail = expectation(&rig, now() + 60, ("", ""));
    refused(&rig, &tail, false, 2, &ordinary);
}

/// BASELINE BEHAVIOUR: self+all wins over supervise; a complete expectation
/// without supervise is rejected before handoff, self-stop or name checking.
#[test]
fn runstop_mode_refusal_precedence_table() {
    let rig = Rig::new("rsmodes");
    for extra in [
        &[][..],
        &["--supervise"][..],
        &["--handoff"][..],
        &["--supervise", "--handoff"][..],
    ] {
        let mut tail = words(&["all", "--self"]);
        tail.extend(words(extra));
        tail.extend(expectation(&rig, now() + 60, ("", "")));
        refused(&rig, &tail, false, 2, SELF_ALL);
    }
    for extra in [&[][..], &["--handoff"][..], &["--self"][..]] {
        let mut tail = words(&["bad/name"]);
        tail.extend(words(extra));
        tail.extend(expectation(&rig, now() + 60, ("", "")));
        refused(&rig, &tail, false, 2, EXPECT_PUBLIC);
    }
    for extra in [
        &[][..],
        &["--handoff"][..],
        &["--self"][..],
        &["--handoff", "--self", "-y"][..],
    ] {
        let mut tail = words(&["bad/name", "--supervise"]);
        tail.extend(words(extra));
        refused(
            &rig,
            &tail,
            false,
            1,
            "Error: 'bad/name' is not a usable session name — nothing was stopped, state preserved.\n",
        );
    }
}

/// Documented target inference: bare and explicitly own-named stops share
/// the self confirmation path, rather than stopping synchronously.
#[test]
fn runstop_bare_and_explicit_own_targets_both_require_self_confirmation() {
    let rig = Rig::new("rsown");
    for tail in [Vec::new(), words(&[&rig.name])] {
        refused(
            &rig,
            &tail,
            true,
            1,
            "Error: nobody attached to confirm; pass -y.\n",
        );
    }
}

/// BASELINE BEHAVIOUR B4: `all` is also a valid existing session name, but
/// bare target inference sets self before the self/all conflict check.
#[test]
fn runstop_bare_inside_session_named_all_keeps_its_existing_refusal() {
    let rig = AmbientRig::new("rsnamedall");
    let dir = rig.plant("all", "");
    let (_, socket) = rig.tmux(&["display-message", "-p", "#{socket_path}"]);
    let meta = dir.join("meta");
    let text = std::fs::read_to_string(&meta).expect("fixture meta");
    std::fs::write(
        meta,
        format!(
            "{text}tmux_server_kind=socket\ntmux_server={}\n",
            socket.trim()
        ),
    )
    .expect("record the private server");
    let pane = first_pane(
        |args| rig.tmux(args),
        &["-t", "=all"],
        Instant::now() + SETUP_WAIT,
    );
    let before = guard_spec::tree(&dir);
    for extra in [&[][..], &["--supervise"][..], &["--handoff"][..]] {
        let mut argv = vec!["_stop"];
        argv.extend_from_slice(extra);
        argv.extend(["--pane", pane.as_str()]);
        let got = rig.run_inside(&pane, &argv);
        assert_eq!(got, (Some(2), String::new(), SELF_ALL.to_owned()));
        assert_eq!(guard_spec::tree(&dir), before);
        assert_eq!(rig.sessions(), ["all"]);
    }
}

/// BASELINE BEHAVIOUR: a failed caller lookup prevents inference, but is not
/// a write refusal when a singular other-session target was given explicitly.
#[test]
fn runstop_unread_caller_only_refuses_an_inferred_target() {
    let rig = Rig::new("rsunread");
    guard_spec::keepalive(&rig);
    ae::migrate::session(&rig.dir).expect("current fixture meta before snapshot");
    let before = guard_spec::tree(&rig.dir);
    let caller = Some((rig.sock.as_path(), "%9999"));
    let got = rig.run_from(&["_stop", "--pane", "%9999"], caller);
    assert_eq!(
        got,
        (
            Some(2),
            String::new(),
            stop_gap("tmux did not report pane \"%9999\""),
        )
    );
    assert_eq!(guard_spec::tree(&rig.dir), before);
    assert!(rig.session_is_live());

    let (code, out, err) = rig.run_from(&["_stop", &rig.name, "--pane", "%9999"], caller);
    assert_eq!(code, Some(0), "out={out} err={err}");
    assert!(!rig.session_is_live());
    assert_eq!(
        std::fs::read(rig.dir.join("meta")).expect("kept meta"),
        before[Path::new("meta")].2
    );
    let events = std::fs::read_to_string(rig.dir.join("events.jsonl")).expect("stop events");
    let actions: Vec<_> = events
        .lines()
        .map(|line| {
            ae::events::Event::parse_line(line)
                .expect("product event")
                .action
        })
        .collect();
    assert_eq!(actions, ["stop-request", "stop-result"]);
}

/// BASELINE BEHAVIOUR of direct `_stop all`: an empty fleet is success even
/// without confirmation; a nonempty fleet without a terminal refuses exactly.
#[test]
fn runstop_public_all_empty_and_no_terminal_table() {
    let empty = AmbientRig::new("rsempty");
    assert_eq!(
        empty.run(&["_stop", "all"]),
        (
            Some(0),
            "No running ae sessions.\n".to_owned(),
            String::new()
        )
    );
    let rig = Rig::new("rsallprompt");
    refused(
        &rig,
        &words(&["all"]),
        false,
        1,
        "Error: 'stop all' stops every running ae session (1), and there is no terminal to confirm on.\n  Re-run with -y: ae stop all -y\n  Nothing was stopped.\n",
    );
}

/// Documented fleet guarantee: refuse unknown ownership, continue stopping
/// known targets, preserve state, and count failures rather than last status.
#[test]
fn runstop_public_all_counts_each_failure_and_continues_to_the_live_target() {
    let rig = Rig::new("rsallfail");
    guard_spec::keepalive(&rig);
    ae::migrate::session(&rig.dir).expect("current fixture meta before snapshot");
    for name in ["aaa-unowned", "bbb-unowned"] {
        let dir = rig.home.join("sessions").join(name);
        std::fs::create_dir_all(&dir).expect("unowned record directory");
        std::fs::write(
            dir.join("meta"),
            format!("schema=2\nsession={name}\ntmux_server_kind=ambiguous\ntmux_server=unused\n"),
        )
        .expect("unowned server record");
    }
    let before = std::fs::read(rig.dir.join("meta")).expect("original meta");
    let (code, out, err) = rig.run(&["_stop", "all", "--yes"]);
    assert_eq!(code, Some(1), "out={out} err={err}");
    assert!(
        err.ends_with("2 session(s) failed to stop. See errors above.\n"),
        "{err}"
    );
    for name in ["aaa-unowned", "bbb-unowned"] {
        assert!(
            err.contains(&format!("session '{name}' has no positive server record")),
            "{err}"
        );
        assert!(rig.home.join("sessions").join(name).join("meta").is_file());
    }
    assert!(
        !rig.session_is_live(),
        "the later healthy target was stopped"
    );
    assert_eq!(
        std::fs::read(rig.dir.join("meta")).expect("preserved meta"),
        before
    );
    assert!(rig.tmux(&["has-session", "-t", "=keepalive"]).0);
}
