//! Independent baseline 719c5d23 pin for launch's lifecycle-lock refusal.
use super::*;

#[test]
fn launchsplit_held_resume_lock_refuses_with_retry_text_and_keeps_saved_state() {
    let scratch = OwnedScratch::root("lsp", "lock-required");
    assert!(tmux_present(&scratch), "lock-busy pin requires real tmux");
    let rig = Rig::idle("lsp-lock-busy");
    rig.without_chat();
    let session = "lspLockBusy";
    let created = rig.launch(&["--local", session]);
    assert_eq!(created.0, Some(0), "{created:?}");
    rig.wait_for_pane_command(session, "main", "sleep");
    assert!(rig.tmux(&["kill-session", "-t", &format!("={session}")]).0);
    let dir = rig.dir(session);
    let meta = std::fs::read(dir.join("meta")).expect("saved metadata");
    let journal = std::fs::read(dir.join("events.jsonl")).expect("saved journal");
    let attempt = std::fs::read(dir.join(".launch-attempt")).expect("saved attempt");
    let held = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(rig.home.join(format!("sessions/.lifecycle.{session}.lock")))
        .expect("existing canonical lifecycle lock");
    held.try_lock()
        .expect("hold the same kernel lock as launch");
    let refused = rig.launch(&["--local", session]);
    drop(held);
    assert_eq!(refused.0, Some(1), "{refused:?}");
    assert_eq!(refused.1, "", "lock refusal has no working-copy output");
    assert_eq!(
        refused.2,
        format!(
            "Error: another lifecycle operation is in progress for '{session}' — retry shortly.\n"
        )
    );
    assert_eq!(
        std::fs::read(dir.join("meta")).expect("kept metadata"),
        meta
    );
    assert_eq!(
        std::fs::read(dir.join("events.jsonl")).expect("kept journal"),
        journal
    );
    assert_eq!(
        std::fs::read(dir.join(".launch-attempt")).expect("kept attempt"),
        attempt
    );
    assert!(
        !rig.sessions().contains(&session.to_owned()),
        "refusal created tmux state"
    );
}
