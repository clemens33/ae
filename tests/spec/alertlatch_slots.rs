// Supplemental acceptance for slot routing and name reuse over time.
// Meta rejects two roster seats with one display name, so a simultaneous
// two-slot missing-leg case cannot occur. An unrostered pane has no known
// binary and reads Unknown, so it cannot provide a second shell death either.

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

fn als_pass(rig: &DnRig, carry: &mut Carry) -> usize {
    let (result, calls) =
        crate::transport::spawns_during(|| rig.cycle().run(carry, &mut Vec::new()));
    result.expect("real watchdog cycle");
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

#[test]
fn alertlatch_slots_real_death_and_recovery_records_carry_slot_and_session() {
    for missing in [false, true] {
        let rig = DnRig::new("human", true);
        if missing {
            rig.tmux.must(&["kill-pane", "-t", &rig.tmux.pane]);
        } else {
            rig.shell();
        }
        assert_eq!(als_pass(&rig, &mut Carry::new()), 1);
        assert_eq!(als_alerts(&rig, "spawned.0"), 1);
        assert_eq!(als_pass(&rig, &mut Carry::new()), 0);
        if missing {
            al_return(&rig, true);
        } else {
            rig.revive();
        }
        assert_eq!(als_pass(&rig, &mut Carry::new()), 0);
        let action = if missing { "pane-back" } else { "dead-cleared" };
        let events = read_events(&rig.dir);
        let closers: Vec<_> = events
            .iter()
            .filter(|event| event.action == action)
            .collect();
        assert_eq!(closers.len(), 1);
        assert_eq!(closers[0].target_slot.value(), Some("spawned.0"));
        assert_eq!(closers[0].target_session.value(), Some("current"));
        assert_eq!(als_pass(&rig, &mut Carry::new()), 0);
        if missing {
            assert_eq!(al_actions(&rig, "dead-cleared"), 0);
        }
    }
}

#[test]
fn alertlatch_slots_other_watchdog_effects_keep_the_keyless_line() {
    let rig = DnRig::new("human", true);
    let observation = seen();
    let on = super::Acting {
        agent: "worker",
        slot: "spawned.0",
        pane: &rig.tmux.pane,
        seen: &observation,
        events: &[],
        overview: None,
    };
    for (action, summary) in [
        ("alert", "throttled persistently"),
        ("throttled", "upstream throttle"),
        ("limit", "vendor usage limit"),
        ("alert-cleared", "limit released"),
    ] {
        rig.cycle()
            .apply(
                &Effect::Emit {
                    action,
                    summary: summary.to_owned(),
                },
                &on,
                &mut PaneState::default(),
                &mut Vec::new(),
            )
            .expect("real effect application and journal writer");
    }
    let events = read_events(&rig.dir);
    let records: Vec<_> = events
        .iter()
        .filter(|event| event.actor == "watchdog")
        .collect();
    assert_eq!(records.len(), 4);
    for event in records {
        assert!(event.target_slot.value().is_none());
        assert!(event.target_session.value().is_none());
    }
}

#[test]
#[allow(clippy::disallowed_methods, reason = "owned rename-state fixture")]
fn alertlatch_slots_name_reused_after_rename_does_not_inherit_another_slots_death() {
    let rig = DnRig::new("human", true);
    rig.shell();
    assert_eq!(als_pass(&rig, &mut Carry::new()), 1);
    let panes = crate::transport::observe_watch_panes(&rig.tmux.server, "current")
        .expect("owned panes before rename");
    let other = panes
        .iter()
        .find(|pane| pane.agent.as_deref() == Some("parent"))
        .expect("live second seat")
        .pane_id
        .clone();
    // A rename changes the names in meta and pane tags, without a spawn,
    // start or other death closer. Keep both distinct slots and binaries.
    let meta_path = crate::store::open(&rig.dir).meta_path();
    let rows = std::fs::read_to_string(&meta_path).expect("owned meta");
    std::fs::write(
        &meta_path,
        rows.replace("seat.spawned.0=worker", "seat.spawned.0=old")
            .replace("seat.spawned.1=parent", "seat.spawned.1=worker"),
    )
    .expect("valid rename result");
    rig.tmux
        .must(&["set-option", "-p", "-t", &rig.tmux.pane, "@ae_agent", "old"]);
    rig.tmux
        .must(&["set-option", "-p", "-t", &other, "@ae_agent", "worker"]);
    crate::store::open(&rig.dir)
        .append_event(&crate::state::event_line(
            crate::time::Timestamp::now(),
            crate::session_notice::RENAME_ACTOR,
            crate::session_notice::RENAME_ACTION,
            "",
            "parent -> worker",
        ))
        .expect("rename record is no death closer");
    let cycle = rig.cycle();
    assert_eq!(cycle.roster.len(), 2);
    assert!(
        cycle
            .roster
            .iter()
            .any(|seat| seat.slot == "spawned.0" && seat.name == "old")
    );
    assert!(
        cycle
            .roster
            .iter()
            .any(|seat| seat.slot == "spawned.1" && seat.name == "worker")
    );
    assert_eq!(als_pass(&rig, &mut Carry::new()), 0, "restart after rename");
    rig.tmux
        .must(&["respawn-pane", "-k", "-t", &other, "/bin/sh"]);
    settle_shell(&rig, &other);
    assert_eq!(
        als_pass(&rig, &mut Carry::new()),
        1,
        "a different slot dies"
    );
    assert_eq!(als_alerts(&rig, "spawned.1"), 1);
    assert_eq!(als_pass(&rig, &mut Carry::new()), 0);
    // Positive recovery of slot 1 must not rearm still-dead slot 0.
    rig.tmux.must(&[
        "respawn-pane",
        "-k",
        "-t",
        &other,
        "/bin/sh",
        "-c",
        "sleep 600; :",
    ]);
    DnRig::wait_harness(&rig.tmux, &other);
    assert_eq!(als_pass(&rig, &mut Carry::new()), 0);
    assert_eq!(als_pass(&rig, &mut Carry::new()), 0);
    rig.tmux
        .must(&["respawn-pane", "-k", "-t", &other, "/bin/sh"]);
    settle_shell(&rig, &other);
    assert_eq!(als_pass(&rig, &mut Carry::new()), 1, "same slot dies anew");
    assert_eq!(als_alerts(&rig, "spawned.0"), 1);
    assert_eq!(als_alerts(&rig, "spawned.1"), 2);
    assert_eq!(als_pass(&rig, &mut Carry::new()), 0);
}

#[test]
fn alertlatch_slots_legacy_name_alert_and_routed_start_share_one_journal() {
    let rig = DnRig::new("human", true);
    rig.shell();
    // The old name-only alert must still latch by name: it cannot prove
    // which historical slot owned the display name. This is a named residual.
    crate::store::open(&rig.dir)
        .append_event(concat!(
            "{\"ts\":\"2026-11-15T00:00:01Z\",\"actor\":\"watchdog\",",
            "\"action\":\"alert\",\"target\":\"worker\",",
            "\"summary\":\"agent process dead — dropped to shell\"}\n",
        ))
        .expect("legacy name-only alert");
    assert_eq!(als_pass(&rig, &mut Carry::new()), 0, "legacy name fallback");
    assert_eq!(al_alerts(&rig), 1);
    let seat = crate::effort::DiscardSeat {
        session: "current",
        session_id: "11111111-1111-4111-8111-111111111111",
        slot: "spawned.0",
        agent: "worker",
    };
    let line = crate::effort::line(
        crate::time::Timestamp::now(),
        &seat,
        "conversation-resumed",
        "spawned.0:worker-first:42 - current-id",
        "the seat starts again",
    );
    crate::store::open(&rig.dir)
        .append_event(&format!("{line}\n"))
        .expect("owned routed start");
    assert_eq!(als_pass(&rig, &mut Carry::new()), 1, "routed start rearms");
    assert_eq!(al_alerts(&rig), 2, "old alert plus new death");
    assert_eq!(als_alerts(&rig, "spawned.0"), 1);
    assert_eq!(
        als_pass(&rig, &mut Carry::new()),
        0,
        "mixed journal survives restart"
    );
}
