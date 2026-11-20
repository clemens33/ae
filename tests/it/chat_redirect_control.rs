//! Exercise redirect and stale-destination refusal through the real console.

use std::fs;
use std::mem::ManuallyDrop;

use ae::events::Event;

use super::super::o2guard_p2_spec::{Seat, World, ledger};
use super::Rig;

#[test]
fn alt_o_delivers_only_to_the_proven_orchestrator_and_restores_a_refused_draft() {
    let rig = Rig::new("chatredirect", 148, 38);
    let role = running_role(&rig);
    rig.lane_reply("running role has joined");
    rig.wait("the follow pass after the role starts", |screen| {
        screen.contains("running role has joined")
    });
    let source = rig.tool.events();

    // Observe that the role started after the console opened. Clear this
    // line to make the actual redirect use a fresh draft.
    rig.paste("ask orchestrator");
    rig.key("Enter");
    rig.wait("running name and redirect are known", |screen| {
        screen.lines().any(|row| {
            row.contains("orchestrator")
                && row.contains(&rig.name)
                && row.contains("Esc")
                && row.contains("Alt+o")
        })
    });
    rig.key("C-u");
    rig.paste("@colead choose D44");
    rig.key("Enter");
    rig.wait("board-ID warning offers the route", |screen| {
        screen
            .lines()
            .any(|row| row.contains("D44") && row.contains("Alt+o"))
    });
    assert_eq!(rig.tool.events(), source);
    assert!(ledger(&role.dir).is_empty());

    rig.key("M-o");
    // The raw receipt pane records the text but cannot prove its own submit.
    rig.wait("redirect reports the raw seat's outcome", |screen| {
        screen
            .lines()
            .any(|row| row.contains("orchestrator") && row.contains("uncertain"))
    });
    let input = fs::read_to_string(&role.receipt).expect("role receipt");
    assert!(
        input.contains("choose D44"),
        "the body reached the role: {input:?}"
    );
    assert!(!input.contains("@colead"), "the source prefix was stripped");
    assert_eq!(rig.tool.events(), source, "the source did not take the ask");
    assert!(rig.screen().lines().any(|row| row.trim_end() == "to lead>"));
    let delivered = ledger(&role.dir);
    let asks: Vec<_> = delivered
        .lines()
        .filter_map(|row| Event::parse_line(row).ok())
        .filter(|event| event.action == "ask")
        .collect();
    assert_eq!(asks.len(), 1);
    assert_eq!(asks[0].actor, "console:local");
    assert_eq!(asks[0].target.as_deref(), Some("boss"));
    assert_eq!(asks[0].summary.as_deref(), Some("choose D44"));

    rig.paste("@colead choose C3 instead");
    rig.key("Enter");
    rig.wait("second draft has a fresh offer", |screen| {
        screen
            .lines()
            .any(|row| row.contains("C3") && row.contains("Alt+o"))
    });
    rig.tmux(&[
        "set-option",
        "-t",
        "orchestrator",
        "@ae_session_uuid",
        "0199c0de-4444-4890-abcd-ef0123456789",
    ]);
    rig.key("M-o");
    rig.wait("stale role refused and source draft restored", |screen| {
        screen
            .lines()
            .any(|row| row.contains("refused") && row.contains("orchestrator"))
            && screen
                .lines()
                .any(|row| row.trim_end() == "to lead> @colead choose C3 instead")
    });
    assert_eq!(rig.tool.events(), source);
    assert_eq!(ledger(&role.dir), delivered);
    assert!(
        !fs::read_to_string(&role.receipt)
            .expect("role receipt")
            .contains("C3")
    );

    rig.tmux(&["kill-session", "-t", "orchestrator"]);
    rig.lane_reply("the role has stopped");
    rig.wait("the follow pass after the role stops", |screen| {
        screen.contains("the role has stopped")
    });
    rig.key("C-u");
    rig.paste("ask orchestrator later");
    rig.submit("ask orchestrator later");
}

fn running_role(rig: &Rig) -> Seat {
    // Rig already owns and will clean this server and root.
    let world = ManuallyDrop::new(World {
        root: rig.root.clone(),
        sock: rig.socket.clone(),
    });
    let overlay = rig.root.join("orchestrator.config");
    fs::write(&overlay, "[workspace]\norchestrator = true\n").expect("role overlay");
    let role = world.session(
        "orchestrator",
        "boss",
        &format!(
            "mode=local\nmeta_agent=true\nlocal_config={}\n",
            overlay.display()
        ),
    );
    rig.tmux(&[
        "set-environment",
        "-t",
        "orchestrator",
        "AE_SESSION",
        "orchestrator",
    ]);
    ae::watchdog_glue::touch_beat(&role.dir).expect("orchestrator running");
    rig.tmux(&[
        "set-option",
        "-t",
        "orchestrator",
        "@ae_main_pane",
        &role.pane,
    ]);
    role
}
