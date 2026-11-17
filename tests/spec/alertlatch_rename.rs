// Session rename preserves the seat incarnation and its open death episode.
// Exercise the directory move, meta rewrite, real rename record and tmux
// rename; a fresh Carry models the monitor process rebound by ae rename.

fn alr_pass(rig: &DnRig) -> usize {
    let mut cycle = rig.cycle();
    cycle.session = "renamed";
    let (result, calls) =
        crate::transport::spawns_during(|| cycle.run(&mut Carry::new(), &mut Vec::new()));
    result.expect("real cycle in renamed session");
    calls
        .iter()
        .filter(|args| {
            args.iter().any(|arg| arg == "display-message")
                && args.last().is_some_and(|text| {
                    text.contains("[ae watchdog] ")
                        && (text.contains("is DEAD") || text.contains("is MISSING"))
                })
        })
        .count()
}

fn alr_command(rig: &DnRig, live: bool) {
    let previous = rig
        .tmux
        .must(&["display-message", "-p", "-t", &rig.tmux.pane, "#{pane_pid}"]);
    let previous: u32 = previous.trim().parse().expect("owned old pane PID");
    let mut args = vec!["respawn-pane", "-k", "-t", &rig.tmux.pane, "/bin/sh"];
    if live {
        args.extend(["-c", "sleep 600; :"]);
    }
    rig.tmux.must(&args);
    let end = std::time::Instant::now() + Duration::from_mins(1);
    loop {
        let panes = crate::transport::observe_watch_panes(&rig.tmux.server, "renamed")
            .expect("owned renamed session");
        let pane = panes.iter().find(|pane| pane.pane_id == rig.tmux.pane);
        if let Some(pane) = pane
            && let Some(pid) = pane.pane_pid
            && pid != previous
            && crate::watchdog::command_is_shell(pane.current_command.trim())
            && let Some(table) = crate::procs::snapshot()
            && table.iter().any(|row| row.pid == pid)
            && crate::procs::has_descendant_named(&table, pid, "sleep") == live
        {
            return;
        }
        assert!(
            std::time::Instant::now() < end,
            "renamed fixture did not settle: {pane:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[allow(clippy::disallowed_methods, reason = "owned session rename fixture")]
fn alertlatch_session_rename_preserves_death_then_recovery_rearms() {
    let mut rig = DnRig::new("human", true);
    rig.shell();
    assert_eq!(als_pass(&rig, &mut Carry::new()), 1, "initial death");
    assert_eq!(al_alerts(&rig), 1);
    let session_id = crate::meta::session_id_in(&rig.dir);
    assert_eq!(session_id, "11111111-1111-4111-8111-111111111111");

    // These are the rename publication operations, without launching monitor
    // subprocesses: the next real Cycle starts with the rebound fresh Carry.
    rig.tmux
        .must(&["rename-session", "-t", "=current", "renamed"]);
    let moved = rig.tmux.path("renamed-record");
    std::fs::rename(&rig.dir, &moved).expect("move owned session directory");
    rig.dir = moved;
    rig.helper = SendHelper(rig.dir.join("send"));
    crate::meta::rewrite(&rig.dir, "session", Some("renamed")).expect("publish renamed meta");
    crate::store::open(&rig.dir)
        .append_event(&crate::state::event_line(
            crate::time::Timestamp::now(),
            crate::session_notice::RENAME_ACTOR,
            crate::session_notice::RENAME_ACTION,
            "",
            "current -> renamed",
        ))
        .expect("production rename record");
    assert_eq!(crate::meta::session_id_in(&rig.dir), session_id);
    assert_eq!(
        alr_pass(&rig),
        0,
        "rename and restart preserve the same death"
    );
    assert_eq!(alr_pass(&rig), 0, "another restart stays silent");
    assert_eq!(al_alerts(&rig), 1);
    assert_eq!(al_actions(&rig, "dead-cleared"), 0);

    alr_command(&rig, true);
    assert_eq!(alr_pass(&rig), 0, "positive harness recovery");
    assert_eq!(al_actions(&rig, "dead-cleared"), 1);
    assert_eq!(alr_pass(&rig), 0, "recovery clears once across restarts");
    assert_eq!(al_actions(&rig, "dead-cleared"), 1);
    alr_command(&rig, false);
    assert_eq!(alr_pass(&rig), 1, "new death after recovery alerts once");
    assert_eq!(alr_pass(&rig), 0, "new death survives another restart");
    assert_eq!(al_alerts(&rig), 2);

    // The existing stable-routing field pins the same incarnation on both
    // sides of the rename; expected identity comes from the fixture's meta.
    for event in read_events(&rig.dir)
        .into_iter()
        .filter(|event| matches!(event.action.as_str(), "alert" | "dead-cleared"))
    {
        assert_eq!(event.target_slot.value(), Some("spawned.0"));
        assert_eq!(event.target_session_id.value(), Some(session_id.as_str()));
    }
}
