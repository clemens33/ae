//! #296 follow-up 1 residual: relaunch and snapshot-less `_run` must use the
//! recorded global authority for a labeled spawned seat. Oracle: lead ruling
//! 2026-11-17. A named refusal is allowed; running a project-shadowed client is not.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "independent acceptance owns private configs, tools and tmux panes"
)]

use std::process::Stdio;
use std::time::{Duration, Instant};

use super::clientfollow_resume_spec::{Rig, Shape, success};

#[derive(Clone, Copy)]
enum Entry {
    Relaunch,
    Run,
}

fn parked(tag: &str, labeled: bool, main: bool) -> (Rig, String, String) {
    let rig = Rig::new(tag, Shape::Flagless);
    if main {
        let config = std::fs::read_to_string(&rig.config).expect("global identity");
        assert!(
            config.contains("lead = idle\n"),
            "the fixture names its main profile"
        );
        std::fs::write(&rig.config, config.replace("lead = idle\n", "lead = p\n"))
            .expect("main starts on a bare profile");
    }
    rig.launch();
    let slot = if main { "main" } else { "spawned.0" };
    if !main {
        success(&rig.spawn("helper", if labeled { "p@carry" } else { "p" }));
    }
    rig.wait_exec(
        if labeled { "b" } else { "a" },
        if labeled { "store-b" } else { "store-a" },
    );
    let pane = rig.pane(slot).expect("the actual seat pane");
    assert!(
        rig.tmux(&["respawn-pane", "-k", "-t", &pane, "exec /bin/sh"])
            .0,
        "park only this fixture's tool at its shell"
    );
    super::support::pane_command(
        |args| rig.tmux(args),
        &pane,
        Instant::now() + super::support::PANE_WAIT,
        super::support::is_shell,
    );
    // Keep the adapter and account identical. Only the executable differs,
    // so reapplying the recorded account cannot conceal the wrong binary.
    std::fs::write(
        rig.project.join(".ae/config"),
        format!(
            "[clients]\n{} = {} config_home={}\n",
            if labeled { "carry" } else { "default" },
            rig.binary("c", "claude").display(),
            rig.scratch
                .join(if labeled { "store-b" } else { "store-a" })
                .display(),
        ),
    )
    .expect("project executable shadow");
    std::fs::write(&rig.receipt, "").expect("restart-only execution receipts");
    (rig, pane, slot.to_owned())
}

fn named_refusal(stderr: &str) {
    assert!(
        stderr.contains("carry") && stderr.contains("Error:"),
        "a refusal identifies the recorded client and its reason: {stderr}"
    );
}

fn restart(entry: Entry, labeled: bool, main: bool) {
    let tag = match (entry, labeled, main) {
        (Entry::Relaunch, true, _) => "pin-relaunch-label",
        (Entry::Run, true, _) => "pin-run-label",
        (Entry::Relaunch, false, false) => "pin-relaunch-bare",
        (Entry::Run, false, false) => "pin-run-bare",
        (Entry::Relaunch, false, true) => "pin-relaunch-main",
        (Entry::Run, false, true) => "pin-run-main",
    };
    let (rig, pane, slot) = parked(tag, labeled, main);
    let before = rig.meta();
    let mut refused = false;
    // A spawned child holds stdin open for the external fake. OwnedChild
    // terminates and reaps precisely that process when this test finishes.
    let mut running = None;
    match entry {
        Entry::Relaunch => {
            let out = rig
                .command()
                .args([
                    "@cfsession",
                    "relaunch",
                    if main { "lead" } else { "helper" },
                ])
                .output()
                .expect("real public relaunch");
            if !out.status.success() {
                assert!(labeled, "bare route must remain usable: {out:?}");
                named_refusal(&String::from_utf8_lossy(&out.stderr));
                refused = true;
            }
        }
        Entry::Run => {
            let mut child = rig
                .command()
                .arg("_run")
                .arg(rig.dir())
                .arg(&slot)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .expect("real snapshot-less run");
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if child.try_wait().expect("run child status").is_some() {
                    let out = child.wait_with_output().expect("run refusal output");
                    assert!(labeled && !out.status.success(), "{out:?}");
                    named_refusal(&String::from_utf8_lossy(&out.stderr));
                    refused = true;
                    break;
                }
                if !rig.execs().is_empty() {
                    running = Some(child);
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "run neither executed nor refused"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    if refused {
        assert!(rig.execs().is_empty(), "refusal starts no binary");
        assert_eq!(rig.meta(), before, "refusal keeps the recorded identity");
        let (ok, command) = rig.tmux(&[
            "display-message",
            "-p",
            "-t",
            &pane,
            "#{pane_current_command}",
        ]);
        assert!(ok && super::support::is_shell(command.trim()), "{command}");
    } else {
        let side = if labeled { "b" } else { "c" };
        rig.wait_exec(side, if labeled { "store-b" } else { "store-a" });
        let receipt = rig.execs();
        if labeled {
            assert!(
                !receipt.contains(&format!("EXEC={} ", rig.binary("c", "claude").display())),
                "a labeled spawned seat must never execute the project-shadowed client: {receipt}"
            );
            assert_eq!(rig.row("client.spawned.0").as_deref(), Some("carry"));
        }
    }
    drop(running);
}

#[test]
fn labeled_spawned_relaunch_never_executes_the_project_client() {
    restart(Entry::Relaunch, true, false);
}

#[test]
fn labeled_spawned_snapshotless_run_never_executes_the_project_client() {
    restart(Entry::Run, true, false);
}

#[test]
fn unlabeled_spawned_relaunch_keeps_layered_resolution() {
    restart(Entry::Relaunch, false, false);
}

#[test]
fn unlabeled_spawned_snapshotless_run_keeps_layered_resolution() {
    restart(Entry::Run, false, false);
}

#[test]
fn bare_main_relaunch_keeps_layered_resolution() {
    restart(Entry::Relaunch, false, true);
}

#[test]
fn bare_main_snapshotless_run_keeps_layered_resolution() {
    restart(Entry::Run, false, true);
}
