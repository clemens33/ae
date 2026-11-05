//! Independent #296 preservation pin: attaching to an existing live session
//! keeps its running admitted seat; stopped restoration re-proves its store.
//! Oracle: frozen plan-r3 section 5, contract.md running-pair invariant and
//! the existing public running-session preservation acceptance test.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "independent acceptance owns private files and tmux fixtures"
)]

use super::clientfollow_resume_spec::{Rig, Shape, success};

const KEYS: [&str; 6] = [
    "seat.spawned.0",
    "profile.spawned.0",
    "client.spawned.0",
    "config_home.spawned.0",
    "config_home_base.spawned.0",
    "agent_bin.spawned.0",
];

fn identity(rig: &Rig) -> [Option<String>; 6] {
    KEYS.map(|key| rig.row(key))
}

fn resume_with_marker(rig: &Rig) -> std::process::Output {
    let socket = rig.row("tmux_server").expect("recorded private socket");
    rig.command()
        .arg(ae::cli::LAUNCH)
        .arg("--home")
        .arg(&rig.home)
        .arg("--cwd")
        .arg(&rig.project)
        .arg("--global")
        .arg(&rig.config)
        .args(["--server-kind", "socket", "--server"])
        .arg(socket)
        .args([
            "--no-attach",
            "--test-pre-lock-marker",
            "--",
            "--local",
            "cfsession",
        ])
        .output()
        .expect("real flagless launch through the public launch door")
}

#[allow(
    clippy::too_many_lines,
    reason = "opposing real running/stopped routes share one established seat fixture"
)]
fn changed_store(running: bool) {
    let rig = Rig::new(
        if running {
            "reattach-live"
        } else {
            "reattach-stopped"
        },
        Shape::Flagless,
    );
    rig.launch();
    success(&rig.spawn("helper", "p@carry"));
    rig.wait_exec("b", "store-b");
    assert_eq!(rig.row("client.spawned.0").as_deref(), Some("carry"));
    assert_eq!(rig.row("profile.spawned.0").as_deref(), Some("p"));
    let store_b =
        std::fs::canonicalize(rig.scratch.join("store-b")).expect("established account store");
    assert_eq!(
        rig.row("config_home.spawned.0"),
        Some(store_b.display().to_string()),
        "the admitted running seat actually retained store B"
    );
    let before_identity = identity(&rig);
    let pane = rig.pane("spawned.0").expect("live admitted spawned pane");
    let (ok, live_before) = rig.tmux(&[
        "display-message",
        "-p",
        "-t",
        &pane,
        "#{session_id}|#{pane_id}|#{pane_pid}|#{pane_dead}",
    ]);
    assert!(
        ok && live_before.trim_end().ends_with("|0"),
        "{live_before}"
    );
    let initial_receipt = rig.execs();
    assert!(
        !initial_receipt.is_empty(),
        "established external harness receipt"
    );

    if !running {
        assert!(
            rig.tmux(&["kill-session", "-t", "=cfsession"]).0,
            "stop only this fixture's private session"
        );
        std::fs::write(&rig.receipt, "").expect("resume-only execution receipts");
    }
    // Valid identity, existing directories, unchanged harness/binary/profile.
    // Only carry's explicit account changes B -> A; no missing config or syntax
    // error can cause an earlier unrelated current-config refusal.
    rig.write_config(&rig.identity("b", "store-a", "claude"), "", false);
    let before_meta = rig.meta();
    let marker = rig.home.join(ae::session_launch::TEST_PRE_LOCK_MARKER);
    assert!(!marker.exists(), "fresh real lifecycle-boundary witness");
    let out = resume_with_marker(&rig);

    if running {
        assert!(
            out.status.success(),
            "live reattach must preserve the running admitted seat despite a future store change: {out:?}"
        );
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("Session 'cfsession' is running."),
            "completed live reattach response: {out:?}"
        );
        assert!(marker.is_file(), "actual launch passed preflight");
        assert_eq!(rig.pane("spawned.0").as_deref(), Some(pane.as_str()));
        let (ok, live_after) = rig.tmux(&[
            "display-message",
            "-p",
            "-t",
            &pane,
            "#{session_id}|#{pane_id}|#{pane_pid}|#{pane_dead}",
        ]);
        assert!(ok, "same private live pane remains observable");
        assert_eq!(
            live_after, live_before,
            "reattach must not rebuild or respawn the seat"
        );
        assert_eq!(
            identity(&rig),
            before_identity,
            "running identity stays whole"
        );
        assert_eq!(
            rig.execs(),
            initial_receipt,
            "reattach starts no replacement harness"
        );
    } else {
        assert!(
            !out.status.success(),
            "stopped restore must reject the changed store: {out:?}"
        );
        assert!(
            String::from_utf8_lossy(&out.stderr)
                .contains("a client override cannot move a retained conversation"),
            "stopped control reaches the retained-store refusal: {out:?}"
        );
        assert!(
            !marker.exists(),
            "stopped refusal precedes the lifecycle boundary"
        );
        assert_eq!(
            rig.meta(),
            before_meta,
            "stopped preflight leaves recorded meta unchanged"
        );
        assert_eq!(
            identity(&rig),
            before_identity,
            "stopped identity stays whole"
        );
        assert!(
            !rig.tmux(&["has-session", "-t", "=cfsession"]).0,
            "stopped refusal creates no session"
        );
        assert!(
            rig.execs().is_empty(),
            "stopped refusal executes no harness"
        );
    }
}

#[test]
fn live_labeled_spawn_reattaches_after_global_store_change() {
    changed_store(true);
}

#[test]
fn stopped_labeled_spawn_refuses_the_same_global_store_change() {
    changed_store(false);
}
