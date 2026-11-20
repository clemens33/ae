//! The waiting-user charter refresh requires a live, idle composer.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "the acceptance fixture owns its private tmux, fake composer and journal"
)]

use super::app_writer_spec::Rig;
use super::cli::{OwnedChild, ae};
use super::o2guard_p2_spec::World;
use super::relay_chat_spec::{Rig as Relay, events, words};
use super::watchdog_step::Stepper;
use ae::harness_state::{HarnessState, classify};
use ae::tool::ToolKind;
use std::fs;
use std::mem::ManuallyDrop;
use std::path::Path;
use std::process::Stdio;

const HEAD: &str = "CHARTER UPDATE (ae ";
const MARKER: &str = "You are the owner's chief of staff across the ae fleet";
const OVERLAY_LINE: &str = "The waiting-user fixture sentence comes from the running seat overlay.";
const RELEASE_100: &str = include_str!("../fixtures/orchestrator/v2026.11.100.config");
const BUSY: &str = "• Working (3s · esc to interrupt)";
const IDLE: &str = "• Fixture finished";
const PLACEHOLDER: &str = "Ask Codex to do anything";
const FOOTER: &str = "  gpt-6-sol high · ~/x";

pub(super) fn turn_fixture(rig: &Rig) {
    // Use the delivery fixture's submit recorder and style-delimited composer.
    // Give it the measured Codex chrome so a normal running turn can have
    // exactly the same empty box as an idle turn. No fake ae verdict is set.
    let script = rig.root.join("faketui.pl");
    let stock = fs::read_to_string(&script).expect("stock delivery fixture");
    let banner = r#"print "fake tui transcript\r\n";"#;
    let draw_turn = format!(
        "print (-e \"$control.busy\" ? \"{BUSY}\\r\\n\" : \"{IDLE}\\r\\n\");\
         \n$content = \"\\e[2m{PLACEHOLDER}\\e[0m\" if $content eq '';"
    );
    assert!(stock.contains(banner));
    assert!(stock.contains("fake-model  ~/x"));
    let measured = stock
        .replacen(banner, &draw_turn, 1)
        .replace("  fake-model  ~/x", FOOTER);
    fs::write(&script, measured).expect("draw measured frames in the private fake TUI");
    let command = format!(
        "exec perl {} {} {} 400 codex 0 '' {}",
        script.display(),
        rig.root.join("received").display(),
        rig.root.join("enters").display(),
        rig.root.join("control").display(),
    );
    rig.tmux(&["respawn-pane", "-k", "-t", &rig.tool.pane, &command]);
    rig.wait(&rig.tool.pane, "real idle frame", |screen| {
        screen.contains(IDLE) && classify(screen, ToolKind::Codex) == HarnessState::Idle
    });
}

fn prepare_launch(rig: &Rig, global: &Path, meta: &Path) {
    fs::write(global, "[workspace]\nquota = off\nauto_upgrade = off\n")
        .expect("no external tools in the cycle");
    let overlay = rig.root.join("orchestrator.config");
    let current = include_str!("../../contrib/aeorchestrator/orchestrator.config");
    let (prompt, end) = current.rsplit_once("\n\"\"\"").expect("prompt delimiter");
    fs::write(&overlay, format!("{prompt}\n{OVERLAY_LINE}\n\"\"\"{end}"))
        .expect("the current local charter plus one owner line");
    let prior = fs::read_to_string(meta).expect("live fixture meta");
    let solo = prior
        .lines()
        .filter(|line| !line.contains(".worker.0="))
        .collect::<Vec<_>>()
        .join("\n")
        .replace("layout=lead-pair", "layout=vertical");
    fs::write(
        meta,
        format!(
            "{solo}\nmeta_agent=true\nmain_pane={}\nlocal_config={}\nconfig={}\n\
             charter.main=1\ncharter_launch.main=tok-rig\n",
            rig.tool.pane,
            overlay.display(),
            global.display()
        ),
    )
    .expect("the current launch has an older charter on the same launch token");
}

fn start_watchdog(rig: &Rig, global: &Path) -> (OwnedChild, Stepper) {
    let channel = "orchrename-charter-step";
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
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("AE_SENDER_OVERRIDE")
        .stdout(Stdio::null())
        .stderr(
            fs::File::create(rig.root.join("charter-watchdog.err")).expect("watchdog diagnostics"),
        )
        .spawn()
        .expect("start the stepped watchdog");
    (watchdog, Stepper::new(&rig.socket, channel))
}

fn declare(rig: &Rig, state: &str) {
    let world = ManuallyDrop::new(World {
        root: rig.root.clone(),
        sock: rig.socket.clone(),
    });
    let scope = format!("@{}", rig.home);
    let mut args = vec![scope.as_str(), "state", state];
    if state == "waiting-agent" {
        args.extend(["--on", "fixture-peer"]);
    }
    args.push(
        "The fixture changes the main declaration before a stepped watchdog cycle. \
         1. Keep watching | 2. Stop the fixture (recommend 1 to exercise delivery).",
    );
    let out = world.run(Some(&rig.tool.pane), &args, &[]);
    assert_eq!(out.0, Some(0), "{}", words(&out));
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).expect("fixture file")
}

fn delivery_rows(rig: &Rig) -> usize {
    events(&rig.tool.dir)
        .iter()
        .filter(|row| {
            row.action == "charter-update"
                && row.summary.as_deref() == Some("charter update delivered")
        })
        .count()
}

fn assert_held(rig: &Rig) {
    let meta = read(&rig.tool.dir.join("meta"));
    assert!(meta.lines().any(|line| line == "charter.main=1"), "{meta}");
    assert!(!meta.contains("charter_pending.main="), "{meta}");
    assert!(!read(&rig.root.join("received")).contains(HEAD));
    assert!(read(&rig.root.join("enters")).is_empty());
    assert_eq!(delivery_rows(rig), 0);
}

fn assert_delivery(rig: &Rig) {
    let received = read(&rig.root.join("received"));
    assert_eq!(received.matches(HEAD).count(), 1, "{received:?}");
    assert_eq!(read(&rig.root.join("enters")).lines().count(), 1);
    let (_, pointer) = received
        .split_once(" B in your session dir: messages/")
        .expect("the submitted charter turn names the saved overlay body");
    let (filename, _) = pointer.split_once(" — ").expect("saved body filename");
    let file = Path::new(filename);
    assert_eq!(
        file.file_name().and_then(|name| name.to_str()),
        Some(filename)
    );
    let body = read(&rig.tool.dir.join("messages").join(file));
    for text in [HEAD, MARKER, OVERLAY_LINE] {
        assert!(body.contains(text), "saved instructions lack {text:?}");
    }
    let meta = read(&rig.tool.dir.join("meta"));
    let stamp = meta
        .lines()
        .find_map(|line| line.strip_prefix("charter.main="))
        .expect("charter version stamp");
    assert!(matches!(stamp, "2" | "3"), "{meta}");
    assert!(
        meta.lines()
            .any(|line| line == "charter_launch.main=tok-rig")
    );
    assert!(!meta.contains("charter_pending.main="), "{meta}");
    assert_eq!(delivery_rows(rig), 1);
}

#[test]
fn waiting_user_refreshes_once_when_the_same_launch_is_idle_and_empty() {
    let rig = Rig::new("orchrenameidle");
    turn_fixture(&rig);
    let global = rig.root.join("config");
    let meta = rig.tool.dir.join("meta");
    prepare_launch(&rig, &global, &meta);
    let (_watchdog, mut stepper) = start_watchdog(&rig, &global);
    stepper.ready();
    assert_held(&rig);
    for state in ["working", "waiting-agent", "blocked"] {
        declare(&rig, state);
        stepper.step();
        assert_held(&rig);
    }

    declare(&rig, "waiting-user");
    let busy = rig.root.join("control.busy");
    fs::write(&busy, []).expect("hold the fixture turn in progress");
    rig.key(&rig.tool.pane, "C-u");
    rig.wait(
        &rig.tool.pane,
        "working turn with empty composer",
        |screen| {
            screen.contains(BUSY)
                && screen.contains(PLACEHOLDER)
                && classify(screen, ToolKind::Codex) == HarnessState::Busy
        },
    );
    stepper.step();
    assert_held(&rig);
    assert!(rig.screen(&rig.tool.pane).contains(PLACEHOLDER));

    fs::remove_file(busy).expect("end the fixture turn");
    let draft = "This unfinished owner draft must survive the cycle.";
    rig.raw(&rig.tool.pane, draft);
    rig.wait(&rig.tool.pane, "private draft after the turn", |screen| {
        screen.contains(IDLE) && screen.contains(draft)
    });
    stepper.step();
    assert_held(&rig);
    assert!(rig.screen(&rig.tool.pane).contains(draft));
    rig.key(&rig.tool.pane, "C-u");
    rig.wait(
        &rig.tool.pane,
        "now idle with the draft cleared",
        |screen| {
            screen.contains(PLACEHOLDER) && classify(screen, ToolKind::Codex) == HarnessState::Idle
        },
    );

    stepper.step();
    assert_delivery(&rig);
    stepper.steps(2);
    assert_delivery(&rig);
}

#[test]
fn the_released_v2026_11_100_overlay_still_relays_while_waiting_user() {
    let rig = Relay::dispatcher("orchrenamecompat");
    let overlay = rig.world.root.join("orchestrator.config");
    fs::write(&overlay, RELEASE_100).expect("byte-exact previously released overlay");
    let state = rig.world.run(
        Some(&rig.caller.pane),
        &[
            "@orch",
            "state",
            "waiting-user",
            "Which relay fixture should run? 1. Real relay | 2. Print only \
             (recommend 1 to check the installed prompt marker).",
        ],
        &[],
    );
    assert_eq!(state.0, Some(0), "{}", words(&state));
    let text = "This quote still reaches the target with the v2026.11.100 prompt installed.";
    let id = rig.ask(text);
    assert!(rig.sent().contains(text));
    assert!(
        rig.asks()
            .iter()
            .any(|row| row.reference.as_deref() == Some(id.as_str()))
    );
    assert_eq!(read(&overlay), RELEASE_100);
}

#[test]
fn an_unconfirmed_charter_claim_stays_bound_to_the_running_launch() {
    let rig = Rig::new("orchrenamepending");
    turn_fixture(&rig);
    let global = rig.root.join("config");
    let meta = rig.tool.dir.join("meta");
    prepare_launch(&rig, &global, &meta);
    fs::write(
        &meta,
        format!("{}charter_pending.main=tok-rig\n", read(&meta)),
    )
    .expect("a previous watchdog could not confirm the old charter turn");
    declare(&rig, "waiting-user");
    let (_watchdog, mut stepper) = start_watchdog(&rig, &global);
    stepper.ready();
    stepper.steps(2);
    let current = read(&meta);
    assert!(current.lines().any(|line| line == "charter.main=1"));
    assert!(
        current
            .lines()
            .any(|line| line == "charter_pending.main=tok-rig")
    );
    assert!(!read(&rig.root.join("received")).contains(HEAD));
    assert_eq!(delivery_rows(&rig), 0);
}
