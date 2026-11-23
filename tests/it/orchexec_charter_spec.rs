//! A watchdog refreshes the running dispatcher when its own seat is done.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "the acceptance fixture owns its watcher, journal and private tmux"
)]

use super::app_writer_spec::Rig;
use super::cli::{OwnedChild, ae};
use super::o2guard_p2_spec::World;
use super::relay_chat_spec::words;
use super::watchdog_step::Stepper;
use std::fs;
use std::mem::ManuallyDrop;
use std::path::Path;
use std::process::Stdio;

const CHARTER_HEAD: &str = "CHARTER UPDATE (ae ";
const MARKER: &str = "You are the owner's chief of staff across the ae fleet";
const OWNER_LINE: &str = "The owner's fixture sentence exists only in this on-disk overlay.";
const CURRENT_OVERLAY: &str = include_str!("../../contrib/aeorchestrator/orchestrator.config");

#[test]
fn the_released_orchestrator_prompt_carries_the_chief_of_staff_charter() {
    let config = include_str!("../../contrib/aeorchestrator/orchestrator.config");
    let charter = include_str!("../../contrib/aeorchestrator/CHARTER.md");
    let (_, prompt) = config.split_once("[prompt]\n").expect("template prompt");
    assert!(prompt.contains(MARKER));
    assert!(charter.to_ascii_lowercase().contains("chief of staff"));
}

fn prepare_old_launch(rig: &Rig, global: &Path, meta: &Path) {
    fs::write(global, "[workspace]\nquota = off\nauto_upgrade = off\n")
        .expect("no external tools in the watchdog cycle");
    let local = rig.root.join("orchestrator.config");
    let (prompt, end) = CURRENT_OVERLAY
        .rsplit_once("\n\"\"\"")
        .expect("current prompt delimiter");
    fs::write(&local, format!("{prompt}\n{OWNER_LINE}\n\"\"\"{end}"))
        .expect("a compatible overlay with the current charter marker");
    let current = fs::read_to_string(meta).expect("live fixture meta");
    let solo = current
        .lines()
        .filter(|line| !line.contains(".worker.0="))
        .collect::<Vec<_>>()
        .join("\n")
        .replace("layout=lead-pair", "layout=vertical");
    fs::write(
        meta,
        format!(
            "{solo}\nmeta_agent=true\nmain_pane={}\nlocal_config={}\nconfig={}\n\
         charter.main=2\ncharter_launch.main=tok-older-launch\n",
            rig.tool.pane,
            local.display(),
            global.display()
        ),
    )
    .expect("the current launch has only an older launch's charter stamp");
}

fn start_watchdog(rig: &Rig, global: &Path) -> (OwnedChild, Stepper) {
    let channel = "orchexec-charter-step";
    let pane = rig.tmux(&[
        "new-window",
        "-d",
        "-t",
        &rig.home,
        "-P",
        "-F",
        "#{pane_id}",
        "sleep",
        "60",
    ]);
    let pane = pane.trim();
    rig.tmux(&["set-option", "-p", "-t", pane, "@ae_agent", "_watchdog"]);
    let watchdog = ae()
        .arg(ae::cli::WATCHDOG_RUN)
        .arg(&rig.tool.dir)
        .args([
            "--interval",
            "1",
            "--step-channel",
            channel,
            "--idle-nudge-secs",
            "0",
            "--tg-supervise-secs",
            "0",
        ])
        .env("HOME", &rig.root)
        .env("AE_HOME", &rig.root)
        .env("CONFIG_FILE", global)
        .env("TMUX_TMPDIR", &rig.root)
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", &rig.socket)
        .env("TMUX", format!("{},0,0", rig.socket.display()))
        .env("TMUX_PANE", pane)
        .env_remove("AE_SENDER_OVERRIDE")
        .stdout(Stdio::null())
        .stderr(
            fs::File::create(rig.root.join("charter-watchdog.err")).expect("watchdog diagnostics"),
        )
        .spawn()
        .expect("start the real stepped watchdog");
    (watchdog, Stepper::new(&rig.socket, channel))
}

fn assert_charter_notice(dir: &Path, receipt: &str) {
    assert_eq!(receipt.matches(CHARTER_HEAD).count(), 1, "{receipt:?}");
    let line = receipt
        .lines()
        .find(|line| line.contains(CHARTER_HEAD))
        .expect("a visible charter heading");
    let (_, notice) = line.split_once(" LONG BODY ").expect("long charter notice");
    let (_, body) = notice
        .split_once(" B in your session dir: messages/")
        .expect("the notice names a saved body in its own session");
    let (file, _) = body
        .split_once(" — ")
        .expect("the notice delimits the path");
    assert!(!file.contains('/'), "expected one filename: {file:?}");
    assert!(
        Path::new(file).extension().is_some_and(|ext| ext == "txt"),
        "expected a saved text body: {file:?}"
    );
    let instructions = fs::read_to_string(dir.join("messages").join(file))
        .expect("the saved body named by the submitted notice");
    assert!(instructions.contains(CHARTER_HEAD));
    assert!(instructions.contains(MARKER));
    assert!(instructions.contains(OWNER_LINE));
}

#[test]
fn a_running_dispatcher_gets_one_charter_turn_for_its_current_launch_when_done() {
    let rig = Rig::new("execcharter");
    super::orchrename_charter_spec::turn_fixture(&rig);
    let global = rig.root.join("config");
    let meta = rig.tool.dir.join("meta");
    prepare_old_launch(&rig, &global, &meta);
    let world = ManuallyDrop::new(World {
        root: rig.root.clone(),
        sock: rig.socket.clone(),
    });
    let scope = format!("@{}", rig.home);
    let declare = |state| {
        let result = world.run(
            Some(&rig.tool.pane),
            &[
                &scope,
                "state",
                state,
                "The fixture marks the dispatcher through its ordinary state helper.",
            ],
            &[],
        );
        assert_eq!(result.0, Some(0), "{}", words(&result));
    };
    declare("working");
    let receipt = || fs::read_to_string(rig.root.join("received")).expect("actual pane receipt");
    let (_watchdog, mut stepper) = start_watchdog(&rig, &global);
    stepper.ready();
    assert!(!receipt().contains(CHARTER_HEAD));
    assert!(
        fs::read_to_string(&meta)
            .expect("running meta")
            .contains("charter_launch.main=tok-older-launch\n")
    );

    declare("done");
    let draft = "A real unfinished private draft blocks delivery.";
    rig.raw(&rig.tool.pane, draft);
    rig.wait(&rig.tool.pane, "occupied composer", |screen| {
        screen.contains(draft)
    });
    stepper.step();
    assert!(!receipt().contains(CHARTER_HEAD));
    let undelivered = fs::read_to_string(&meta).expect("meta after refused delivery");
    assert!(undelivered.contains("charter_launch.main=tok-older-launch\n"));
    assert!(!undelivered.contains("charter_launch.main=tok-rig\n"));
    assert!(
        rig.screen(&rig.tool.pane).contains(draft),
        "the watchdog preserved the draft"
    );
    rig.key(&rig.tool.pane, "C-u");
    rig.wait(&rig.tool.pane, "cleared composer", |screen| {
        !screen.contains(draft)
    });
    stepper.step();
    let delivered = receipt();
    assert_charter_notice(&rig.tool.dir, &delivered);
    let current = fs::read_to_string(&meta).expect("updated live meta");
    assert!(current.lines().any(|line| line == "charter.main=3"));
    assert!(
        current
            .lines()
            .any(|line| line == "charter_launch.main=tok-rig")
    );
    assert!(!current.contains("charter_launch.main=tok-older-launch"));
    stepper.steps(2);
    assert_eq!(receipt().matches(CHARTER_HEAD).count(), 1);
}

#[test]
fn orchwake_continuation_gets_its_own_cycle_and_leaves_the_charter_owed() {
    let rig = Rig::new("orchwake-charter");
    super::orchrename_charter_spec::turn_fixture(&rig);
    let global = rig.root.join("config");
    let meta = rig.tool.dir.join("meta");
    prepare_old_launch(&rig, &global, &meta);
    let launch = "11111111-1111-4111-8111-111111111111";
    let initial = fs::read_to_string(&meta).expect("fixture meta");
    fs::write(
        &meta,
        format!(
            "{}sweep_sec=0\nreseat_continuation.main=owed:kept:{launch}\n",
            initial.replace(
                "launch_id.main=tok-rig",
                &format!("launch_id.main={launch}")
            )
        ),
    )
    .expect("fixture holds an owed continuation and an obsolete charter stamp");
    let send = rig.tool.dir.join("send");
    if !send.exists() {
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ae"), &send).expect("real session send");
    }
    let world = ManuallyDrop::new(World {
        root: rig.root.clone(),
        sock: rig.socket.clone(),
    });
    let scope = format!("@{}", rig.home);
    let done = world.run(
        Some(&rig.tool.pane),
        &[&scope, "state", "done", "The seat is ready."],
        &[],
    );
    assert_eq!(done.0, Some(0), "{}", words(&done));
    let receipt = || fs::read_to_string(rig.root.join("received")).expect("actual pane receipt");

    let (_watchdog, mut stepper) = start_watchdog(&rig, &global);
    stepper.ready();
    let first = receipt();
    assert_eq!(
        first.matches("Restore your state first").count(),
        1,
        "{first:?}\n{}",
        fs::read_to_string(rig.root.join("charter-watchdog.err")).expect("watchdog diagnostics")
    );
    assert!(!first.contains(CHARTER_HEAD), "{first:?}");
    let after_first = fs::read_to_string(&meta).expect("first cycle metadata");
    assert!(after_first.contains("charter_launch.main=tok-older-launch\n"));
    assert!(!after_first.contains("reseat_continuation.main="));

    stepper.step();
    let second = receipt();
    assert_charter_notice(&rig.tool.dir, &second);
    assert_eq!(
        second.matches("Restore your state first").count(),
        1,
        "{second:?}"
    );
    assert!(
        fs::read_to_string(&meta)
            .expect("second cycle metadata")
            .contains(&format!("charter_launch.main={launch}\n"))
    );
}
