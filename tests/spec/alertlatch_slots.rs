// Supplemental acceptance for the lead's slot-routing ruling. The original
// frozen alertlatch and deadnotify files remain unchanged.

#[allow(
    clippy::disallowed_methods,
    reason = "owned two-slot acceptance fixture"
)]
fn als_rig() -> (DnRig, String) {
    let rig = DnRig::new("human", false);
    std::fs::write(
        crate::store::open(&rig.dir).meta_path(),
        "session=current\nsession_id=11111111-1111-4111-8111-111111111111\n\
         mode=local\nseat.spawned.0=worker\nagent_bin.spawned.0=sleep\n\
         launch_id.spawned.0=worker-first\nseat.spawned.1=worker\n\
         agent_bin.spawned.1=sleep\nlaunch_id.spawned.1=parent-first\n",
    )
    .expect("owned two-slot roster");
    let other = al_return(&rig, true);
    rig.tmux
        .must(&["set-option", "-p", "-t", &other, "@ae_slot", "spawned.1"]);
    // Keep the owned session observable after both roster panes disappear.
    rig.tmux.must(&[
        "new-window",
        "-d",
        "-t",
        "=current:",
        "/bin/sh",
        "-c",
        "sleep 600; :",
    ]);
    let roster = rig.cycle().roster;
    assert_eq!(roster.len(), 2);
    assert!(roster.iter().all(|entry| entry.name == "worker"));
    assert_ne!(roster[0].slot, roster[1].slot);
    (rig, other)
}

fn als_alerts(rig: &DnRig, slot: &str) -> usize {
    read_events(&rig.dir)
        .iter()
        .filter(|event| {
            event.action == "alert"
                && event.target.as_deref() == Some("worker")
                && event.target_slot.value() == Some(slot)
                && event.target_session.value() == Some("current")
        })
        .count()
}

fn als_independent_deaths(missing: bool) {
    let (rig, other) = als_rig();
    if missing {
        rig.tmux.must(&["kill-pane", "-t", &rig.tmux.pane]);
        rig.tmux.must(&["kill-pane", "-t", &other]);
    } else {
        rig.shell();
        rig.tmux
            .must(&["respawn-pane", "-k", "-t", &other, "/bin/sh"]);
        settle_shell(&rig, &other);
    }
    assert_eq!(al_pass(&rig, &mut Carry::new()), 2, "two distinct deaths");
    assert_eq!(al_alerts(&rig), 2);
    assert_eq!(als_alerts(&rig, "spawned.0"), 1);
    assert_eq!(als_alerts(&rig, "spawned.1"), 1);
    assert_eq!(
        al_pass(&rig, &mut Carry::new()),
        0,
        "restart repeats neither"
    );

    // A positive recovery closes only slot 0, although both seats display worker.
    let returned = if missing {
        al_return(&rig, true)
    } else {
        rig.revive();
        rig.tmux.pane.clone()
    };
    assert_eq!(al_pass(&rig, &mut Carry::new()), 0, "one seat returns");
    let action = if missing { "pane-back" } else { "dead-cleared" };
    let events = read_events(&rig.dir);
    let closers: Vec<_> = events
        .iter()
        .filter(|event| event.action == action)
        .collect();
    assert_eq!(closers.len(), 1);
    assert_eq!(closers[0].target_slot.value(), Some("spawned.0"));
    assert_eq!(closers[0].target_session.value(), Some("current"));
    assert_eq!(al_pass(&rig, &mut Carry::new()), 0, "slot 1 stays latched");
    if missing {
        assert_eq!(al_actions(&rig, "dead-cleared"), 0);
        rig.tmux.must(&["kill-pane", "-t", &returned]);
    } else {
        rig.shell();
    }
    assert_eq!(
        al_pass(&rig, &mut Carry::new()),
        1,
        "only slot 0 dies again"
    );
    assert_eq!(al_alerts(&rig), 3);
    assert_eq!(als_alerts(&rig, "spawned.0"), 2);
    assert_eq!(als_alerts(&rig, "spawned.1"), 1);
    assert_eq!(al_pass(&rig, &mut Carry::new()), 0);
}

#[test]
fn alertlatch_slots_missing_deaths_and_returns_are_independent_with_one_display_name() {
    als_independent_deaths(true);
}

#[test]
fn alertlatch_slots_shell_deaths_and_returns_are_independent_with_one_display_name() {
    als_independent_deaths(false);
}

#[test]
fn alertlatch_slots_legacy_name_alert_and_routed_start_keep_the_other_slot_latched() {
    let (rig, other) = als_rig();
    rig.tmux.must(&["kill-pane", "-t", &rig.tmux.pane]);
    rig.tmux.must(&["kill-pane", "-t", &other]);
    // Actual old line shape: without a routing key the display name is all
    // the reader has, so this historical alert initially latches both seats.
    crate::store::open(&rig.dir)
        .append_event(concat!(
            "{\"ts\":\"2026-11-15T00:00:01Z\",\"actor\":\"watchdog\",",
            "\"action\":\"alert\",\"target\":\"worker\",",
            "\"summary\":\"pane missing — agent no longer visible in session\"}\n",
        ))
        .expect("legacy name-only alert");
    let old = read_events(&rig.dir);
    let alert = old
        .iter()
        .find(|event| event.action == "alert")
        .expect("old alert");
    assert!(alert.target_slot.value().is_none());
    assert_eq!(al_pass(&rig, &mut Carry::new()), 0, "legacy name fallback");
    let seat = crate::effort::DiscardSeat {
        session: "current",
        session_id: "11111111-1111-4111-8111-111111111111",
        slot: "spawned.1",
        agent: "worker",
    };
    let line = crate::effort::line(
        crate::time::Timestamp::now(),
        &seat,
        "conversation-resumed",
        "spawned.1:parent-first:42 - current-id",
        "only the second seat starts again",
    );
    crate::store::open(&rig.dir)
        .append_event(&format!("{line}\n"))
        .expect("owned routed start");
    assert_eq!(
        al_pass(&rig, &mut Carry::new()),
        1,
        "only the addressed seat rearms"
    );
    assert_eq!(al_alerts(&rig), 2, "old alert plus one new death");
    assert_eq!(als_alerts(&rig, "spawned.0"), 0);
    assert_eq!(als_alerts(&rig, "spawned.1"), 1);
    assert_eq!(
        al_pass(&rig, &mut Carry::new()),
        0,
        "mixed journal survives restart"
    );
}
