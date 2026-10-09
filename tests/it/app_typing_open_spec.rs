//! Frozen P2: one o cannot move a client; a second opens the captured seat.
//! Uses appopen's existing private tmux/client fixture and live state oracle.

use super::{HOME, Rig};

#[test]
fn one_o_arms_without_moving_and_other_key_disarms() {
    let rig = Rig::new("typeopenarm");
    let app = rig.app(&rig.socket);
    let viewer = rig.attach(HOME);
    rig.agents(&app);
    rig.focused(&app, "lead");
    let before = rig.viewed(&viewer);
    rig.raw(&app, "o");
    rig.wait_screen(&app, "P2 first o only arms", |s| {
        s.contains("o again to open lead")
    });
    assert_eq!(
        rig.viewed(&viewer),
        before,
        "P2 first o cannot move the client"
    );
    rig.raw(&app, "n");
    rig.focused(&app, "colead");
    rig.wait_screen(&app, "P2 n disarms the old seat", |s| {
        !s.contains("o again to open lead")
    });
    rig.raw(&app, "o");
    rig.wait_screen(&app, "P2 next o arms the new focused seat", |s| {
        s.contains("o again to open colead")
    });
    assert_eq!(rig.viewed(&viewer), before);
    rig.raw(&app, "o");
    rig.opened(&viewer, HOME, &rig.colead);
}

#[test]
fn hello_and_open_never_move_the_viewer_or_send_an_ask() {
    for word in ["hello", "open"] {
        let rig = Rig::new(&format!("typeword{word}"));
        let app = rig.app(&rig.socket);
        let viewer = rig.attach(HOME);
        rig.agents(&app);
        rig.focused(&app, "lead");
        let before = rig.viewed(&viewer);
        let path = rig.scratch.join("sessions").join(HOME).join("events.jsonl");
        let journal = std::fs::read(&path).expect("fixture journal");
        rig.raw(&app, word);
        // s produces a later visible frame, proving the complete word was
        // processed without requiring any new implementation accessor.
        rig.raw(&app, "s");
        rig.wait_screen(&app, "P2 full word processed before Settings", |s| {
            s.lines().next().is_some_and(|row| row.contains("Settings"))
        });
        assert_eq!(
            rig.viewed(&viewer),
            before,
            "P2 typing {word} cannot leave the app"
        );
        assert_eq!(std::fs::read(&path).expect("journal after word"), journal);
    }
}

#[test]
fn armed_open_does_not_turn_a_later_q_into_quit() {
    let rig = Rig::new("typecrossarm");
    let app = rig.app(&rig.socket);
    // Keep the exited frame observable on the base instead of racing a
    // capture against tmux deleting its last pane in the chat window.
    rig.tmux(&["set-option", "-w", "-t", &app, "remain-on-exit", "on"]);
    let viewer = rig.attach(HOME);
    rig.agents(&app);
    rig.focused(&app, "lead");
    let before = rig.viewed(&viewer);
    rig.raw(&app, "oq");
    rig.wait_screen(&app, "P2 q only arms quit after disarming o", |s| {
        s.contains("q again to quit") && !s.contains("o again to open")
    });
    assert_eq!(
        rig.tmux(&["display-message", "-p", "-t", &app, "#{pane_dead}"])
            .trim(),
        "0",
        "P2 oq must keep the app alive"
    );
    assert_eq!(rig.viewed(&viewer), before);
}
