//! Independent launch characterisation at 719c5d23; exact outputs are behaviour pins.
use super::*;

fn require_tmux() {
    let scratch = OwnedScratch::root("lsp", "required");
    assert!(tmux_present(&scratch), "launchsplit pins require tmux");
}

fn stopped(rig: &Rig, session: &str, mode: &str) {
    let result = rig.launch(&[mode, session]);
    assert_eq!(result.0, Some(0), "{result:?}");
    rig.wait_for_pane_command(session, "main", "sleep");
    assert!(rig.tmux(&["kill-session", "-t", &format!("={session}")]).0);
}

#[test]
fn launchsplit_refusal_precedence_distinguishes_fresh_and_recorded_solo_launches() {
    require_tmux();
    for resumed in [false, true] {
        for fault in ["quota", "idle", "roster", "copy", "quota-roster"] {
            let rig = Rig::idle("lsp-precedence");
            rig.without_chat();
            let session = "lspPrecedence";
            if resumed {
                stopped(&rig, session, "--local");
                let meta = rig.meta(session);
                let unpinned = meta
                    .split_inclusive('\n')
                    .filter(|line| {
                        !line.starts_with("quota_every_secs=")
                            && !line.starts_with("idle_nudge_secs=")
                    })
                    .collect::<String>();
                std::fs::write(rig.dir(session).join("meta"), unpinned)
                    .expect("legacy record without cadence rows");
            }
            let work = rig.home.join("worktrees").join(session);
            std::fs::create_dir_all(&work).expect("conflicting preserved work");
            std::fs::write(work.join("payload"), b"preserved work").expect("work payload");
            let bad_roster = fault == "roster"
                || fault == "quota-roster"
                || (!resumed && matches!(fault, "quota" | "idle"));
            let mut config = IDLE_CONFIG.to_owned();
            if bad_roster {
                config = config.replace("main = lead", "main = missing");
            }
            if matches!(fault, "quota" | "quota-roster") {
                config.push_str("quota_every_secs = broken\n");
            }
            if matches!(fault, "quota" | "idle") {
                config.push_str("idle_nudge_secs = broken\n");
            }
            std::fs::write(&rig.config, config).expect("competing configuration faults");
            let meta_before = std::fs::read(rig.dir(session).join("meta")).ok();
            let events_before = std::fs::read(rig.dir(session).join("events.jsonl")).ok();
            let result = rig.launch(&["--local", session]);
            let (code, expected) = if resumed && bad_roster {
                // Baseline behaviour pin: the solo preflight precedes cadence,
                // and its message names the recorded main even though the
                // configured selected main is the one missing from the roster.
                (
                    1,
                    "Error: recorded solo main 'lead' is not in [roster] of the current config.\n"
                        .to_owned(),
                )
            } else {
                match fault {
                "quota" | "quota-roster" => (2, "Error: [workspace] quota_every_secs must be an unsigned integer in seconds; got 'broken'.\n".to_owned()),
                "idle" => (2, "Error: [workspace] idle_nudge_secs must be an unsigned integer in seconds; got 'broken'.\n".to_owned()),
                "roster" => (1, "Error: the workspace roster is not launchable:\n  - workspace.main: 'missing' is not bound to a profile in [roster].\n".to_owned()),
                "copy" => (1, format!("Error: stopped session '{session}' exists as worktree/copy. End it first (ae end {session}), or use a different name.\n")),
                _ => unreachable!(),
            }
            };
            assert_eq!(
                result.0,
                Some(code),
                "resumed={resumed}, fault={fault}: {result:?}"
            );
            assert_eq!(result.1, "", "refusal before working-copy output");
            assert_eq!(result.2, expected, "resumed={resumed}, fault={fault}");
            assert_eq!(
                std::fs::read(work.join("payload")).expect("work kept"),
                b"preserved work"
            );
            assert_eq!(
                std::fs::read(rig.dir(session).join("meta")).ok(),
                meta_before
            );
            assert_eq!(
                std::fs::read(rig.dir(session).join("events.jsonl")).ok(),
                events_before
            );
            assert!(!rig.sessions().contains(&session.to_owned()));
            if !resumed {
                assert!(!rig.dir(session).exists(), "refusal must not claim state");
                assert!(
                    !rig.home
                        .join(format!("sessions/.lifecycle.{session}.lock"))
                        .exists()
                );
            }
        }
    }
}

#[test]
fn launchsplit_tmux_creation_failure_removes_only_state_this_attempt_created() {
    require_tmux();
    for mode in ["--local", "--copy", "--worktree"] {
        for kind in ["absent", "unrecorded", "resume"] {
            let rig = Rig::idle("lsp-rollback");
            rig.without_chat();
            let session = "lspRollback";
            std::fs::write(rig.project.join("payload"), b"working bytes").expect("origin payload");
            if mode == "--worktree" {
                git_in(&rig.project, &["init", "-q"]);
                git_in(&rig.project, &["add", "payload"]);
                git_in(&rig.project, &["commit", "-qm", "base"]);
            }
            if kind == "resume" {
                stopped(&rig, session, mode);
            }
            if kind != "absent" {
                std::fs::create_dir_all(rig.dir(session)).expect("pre-existing state");
                std::fs::write(rig.dir(session).join("sentinel"), b"old state")
                    .expect("pre-existing unrelated state");
            }
            let before = std::fs::read(rig.dir(session).join("meta")).ok();
            let events = std::fs::read(rig.dir(session).join("events.jsonl")).ok();
            // The socket's DIRECTORY is a regular file, so connect(2) fails with
            // ENOTDIR and tmux gives up before it tries to start a server, on
            // every supported tmux. A regular file AT the socket path would not
            // do: tmux 3.6 on Linux replaces it and starts, tmux 3.7b on macOS
            // refuses.
            let blocked = rig.scratch.join("blocked-socket");
            std::fs::write(&blocked, b"not a socket").expect("private blocked destination");
            let socket = blocked.join("tmux.sock");
            let mut command = rig.launch_command_with_server(
                "socket",
                &socket.display().to_string(),
                &[mode, session],
            );
            // The blocked socket cannot answer the separate version gate. The
            // established version seam clears it; creation still calls real tmux.
            let log = launchx_trace_command(&mut command, &rig.scratch);
            command.env("AE_LAUNCHX_VERSION", "3.5");
            let output = command.output().expect("creation failure after floor");
            let result = (
                output.status.code(),
                String::from_utf8_lossy(&output.stdout).into_owned(),
                String::from_utf8_lossy(&output.stderr).into_owned(),
            );
            let trace = launchx_trace_text(&log, &rig.scratch, false);
            assert!(
                trace.contains("\tnew-session\t"),
                "never reached creation: {trace}"
            );
            assert_eq!(result.0, Some(1), "mode={mode}, kind={kind}: {result:?}");
            let suffix = if kind == "absent" {
                "ae: session state removed — this launch created it; nothing left to reattach to.\n"
                    .to_owned()
            } else {
                format!(
                    "ae: session state KEPT — it predates this attempt; retry with 'ae {session}'.\n"
                )
            };
            assert_eq!(
                result.2,
                format!("Error: could not create tmux session '{session}'\n{suffix}"),
                "mode={mode}, kind={kind}"
            );
            let work = if mode == "--local" {
                rig.project.clone()
            } else {
                rig.home.join("worktrees").join(session)
            };
            let expected_out = if kind == "resume" {
                format!("Resuming session {session} (dir: {})...\n", work.display())
            } else {
                match mode {
                    "--local" => String::new(),
                    "--copy" => "Creating full copy...\nWorking copy ready.\n".to_owned(),
                    "--worktree" => "Creating git worktree...\nWorking copy ready.\n".to_owned(),
                    _ => unreachable!(),
                }
            };
            assert_eq!(result.1, expected_out, "mode={mode}, kind={kind}");
            assert_eq!(
                std::fs::read(work.join("payload")).expect("working data survives later failure"),
                b"working bytes"
            );
            assert_eq!(std::fs::read(rig.dir(session).join("meta")).ok(), before);
            assert_eq!(
                std::fs::read(rig.dir(session).join("events.jsonl")).ok(),
                events
            );
            assert_eq!(rig.dir(session).exists(), kind != "absent");
            if kind != "absent" {
                assert_eq!(
                    std::fs::read(rig.dir(session).join("sentinel")).expect("old state kept"),
                    b"old state"
                );
            }
            assert_eq!(
                std::fs::read(&blocked).expect("destination file kept"),
                b"not a socket"
            );
        }
    }
}

#[test]
fn launchsplit_running_override_refuses_without_restamping_or_republishing() {
    require_tmux();
    let rig = Rig::idle("lsp-running-override");
    rig.without_chat();
    let session = "lspRunning";
    let created = rig.launch(&["--local", session]);
    assert_eq!(created.0, Some(0), "{created:?}");
    rig.wait_for_pane_command(session, "main", "sleep");
    let before = std::fs::read(rig.dir(session).join("meta")).expect("settled meta");
    let events = std::fs::read(rig.dir(session).join("events.jsonl")).expect("launch journal");
    let panes = rig.panes(session);
    let result = rig.launch(&["--local", session, "--lead", "idle"]);
    assert_eq!(result.0, Some(2), "{result:?}");
    assert_eq!(result.1, "");
    assert_eq!(
        result.2,
        format!("Error: session '{session}' is running; stop it before changing a seat profile.\n")
    );
    assert_eq!(rig.panes(session), panes);
    assert_eq!(
        std::fs::read(rig.dir(session).join("meta")).expect("kept meta"),
        before
    );
    assert_eq!(
        std::fs::read(rig.dir(session).join("events.jsonl")).expect("kept journal"),
        events
    );
}
