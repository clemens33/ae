//! The zoom is inside ae's application frame. Tmux's status and options
//! keep the values the private server owned before the toggle.

use super::*;
use ae::theme::FLEET_STRIP_OPTION;

fn options(rig: &Rig, pane: &str) -> [String; 5] {
    [
        rig.tmux(&["show-options", "-s"]),
        rig.tmux(&["show-options", "-g"]),
        rig.tmux(&["show-options", "-t", &rig.name]),
        rig.tmux(&["show-options", "-w", "-t", pane]),
        rig.tmux(&["show-options", "-p", "-t", pane]),
    ]
}

#[test]
fn zoom_draws_inside_the_app_and_leaves_tmux_options_unchanged() {
    let rig = Rig::new("appzoom", false);
    for (name, value) in [
        (FLEET_STRIP_OPTION, "zoom-fixture-fleet-strip"),
        ("status-left", "zoom-fixture-status"),
    ] {
        rig.tmux(&["set-option", "-t", &rig.name, name, value]);
    }
    let pane = rig.open_app();
    rig.wait(&pane, WAIT, "the real app draws its sidebar", |screen| {
        painted(screen, &rig.name)
    });
    let before = options(&rig, &pane);
    assert!(before[2].contains("zoom-fixture-fleet-strip"));
    assert!(before[2].contains("zoom-fixture-status"));

    rig.tmux(&["send-keys", "-t", &pane, "-l", "--", "X"]);
    rig.wait(&pane, WAIT, "X zooms the real app frame", |screen| {
        screen.lines().next().is_some_and(|header| {
            header.contains(&rig.name) && header.contains("X unzoom") && !header.contains('│')
        })
    });
    assert_eq!(options(&rig, &pane), before, "while zoomed");

    rig.tmux(&["send-keys", "-t", &pane, "-l", "--", "X"]);
    rig.wait(&pane, WAIT, "X restores the sidebar", |screen| {
        painted(screen, &rig.name)
    });
    assert_eq!(options(&rig, &pane), before, "after leaving zoom");
}
