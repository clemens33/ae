//! Baseline 719c5d23: a running explicit-server record cannot focus a foreign pane.
use super::*;

#[test]
fn launchsplit_running_reattach_clears_a_foreign_main_pane_without_restamping_it() {
    let scratch = OwnedScratch::root("lsp", "repair-required");
    assert!(tmux_present(&scratch), "repair pin requires real tmux");
    let rig = Rig::idle("lsp-foreign-main");
    std::fs::write(&rig.config, lead_config("lead-pair", &["colead"]))
        .expect("two-seat lead-pair config");
    rig.without_chat();
    let session = "lspForeignMain";
    let created = rig.launch(&["--local", session]);
    assert_eq!(created.0, Some(0), "{created:?}");
    rig.wait_for_pane_command(session, "main", "sleep");
    let foreign = bare_session(&rig, &rig.sock, "lspOther");
    let own = rig.panes(session);
    assert!(!own.iter().any(|(pane, _, _)| pane == &foreign));
    let meta = rig.meta(session);
    let old = meta
        .lines()
        .find(|line| line.starts_with("main_pane="))
        .expect("recorded main row");
    let replaced = meta.replace(old, &format!("main_pane={foreign}"));
    assert_ne!(meta, replaced, "foreign pane control must apply");
    std::fs::write(rig.dir(session).join("meta"), &replaced).expect("hostile saved main pane");
    let journal = std::fs::read(rig.dir(session).join("events.jsonl")).expect("settled journal");
    let (ok, width) = rig.tmux(&[
        "show-window-options",
        "-v",
        "-t",
        &foreign,
        "main-pane-width",
    ]);
    assert!(ok, "foreign width read must succeed");
    let (ok, hooks) = rig.tmux(&["show-hooks", "-w", "-t", &foreign]);
    assert!(ok, "foreign hooks read must succeed");
    let result = rig.launch(&["--local", session]);
    assert_eq!(result.0, Some(0), "{result:?}");
    assert!(
        result
            .1
            .starts_with("Session 'lspForeignMain' is running. Attach with: "),
        "{result:?}"
    );
    assert_eq!(result.2, "");
    let (ok, options) = rig.tmux(&["show-options", "-t", &format!("={session}:")]);
    assert!(ok, "own session option read must succeed");
    assert!(
        !options
            .lines()
            .any(|line| line.starts_with(ae::theme::MAIN_PANE_OPTION)),
        "foreign main remains published: {options}"
    );
    let (ok, own_hooks) = rig.tmux(&["show-hooks", "-t", &format!("={session}:")]);
    assert!(ok, "own session hook read must succeed");
    assert!(
        !own_hooks
            .lines()
            .any(|line| line.starts_with("client-session-changed[0] ")),
        "foreign focus hook remains: {own_hooks}"
    );
    let (ok, after_width) = rig.tmux(&[
        "show-window-options",
        "-v",
        "-t",
        &foreign,
        "main-pane-width",
    ]);
    assert!(ok, "foreign width re-read must succeed");
    assert_eq!(
        after_width, width,
        "reattach changed a foreign window width"
    );
    let (ok, after_hooks) = rig.tmux(&["show-hooks", "-w", "-t", &foreign]);
    assert!(ok, "foreign hooks re-read must succeed");
    assert_eq!(after_hooks, hooks, "reattach changed foreign resize policy");
    assert_eq!(rig.panes(session), own, "reattach replaced own panes");
    assert_eq!(rig.meta(session), replaced, "reattach republished metadata");
    assert_eq!(
        std::fs::read(rig.dir(session).join("events.jsonl")).expect("kept journal"),
        journal
    );
}
