// Independent alertlatch acceptance. Included after deadnotify.rs so the
// real Cycle and the owned tmux fixture, including its external helper, agree.
// Expected counts come from the one-alert-per-death ruling, not the fold.

fn al_pass(rig: &DnRig, carry: &mut Carry) -> usize {
    let (result, calls) =
        crate::transport::spawns_during(|| rig.cycle().run(carry, &mut Vec::new()));
    result.expect("real watchdog cycle");
    calls
        .iter()
        .filter(|args| {
            args.iter().any(|arg| arg == "display-message")
                && args.last().is_some_and(|text| {
                    text.contains("[ae watchdog] worker ")
                        && (text.contains("is DEAD") || text.contains("is MISSING"))
                })
        })
        .count()
}

fn al_alerts(rig: &DnRig) -> usize {
    read_events(&rig.dir)
        .iter()
        .filter(|event| {
            event.target.as_deref() == Some("worker")
                && event.action == "alert"
                && matches!(
                    event.summary.as_deref(),
                    Some(
                        "agent process dead — dropped to shell"
                            | "pane missing — agent no longer visible in session"
                    )
                )
        })
        .count()
}

fn al_actions(rig: &DnRig, action: &str) -> usize {
    read_events(&rig.dir)
        .iter()
        .filter(|event| event.action == action && event.target.as_deref() == Some("worker"))
        .count()
}

fn al_assert_open(rig: &DnRig) {
    assert!(
        crate::session::alerts_since_in(
            &read_events(&rig.dir),
            "current",
            "spawned.0",
            "worker",
            None,
        )
        .winner()
        .is_some(),
        "a restart must not retract an outstanding human alert"
    );
}

fn al_return(rig: &DnRig, harness: bool) -> String {
    let pane = rig.tmux.must(&[
        "new-window",
        "-d",
        "-t",
        "=current:",
        "-P",
        "-F",
        "#{pane_id}",
        "/bin/sh",
        "-c",
        if harness {
            "sleep 600; :"
        } else {
            "exec /bin/sh"
        },
    ]);
    let pane = pane.trim().to_owned();
    rig.tmux
        .must(&["set-option", "-p", "-t", &pane, "@ae_agent", "worker"]);
    rig.tmux
        .must(&["set-option", "-p", "-t", &pane, "@ae_slot", "spawned.0"]);
    if harness {
        DnRig::wait_harness(&rig.tmux, &pane);
    } else {
        settle_shell(rig, &pane);
    }
    pane
}

#[test]
fn alertlatch_real_cycle_restart_preserves_both_human_alert_legs_without_a_spawner_notice() {
    for missing in [false, true] {
        // A human spawner receives no dead-notice. Its absence must not weaken
        // the independently owed human alert or its durable suppression.
        let rig = DnRig::new("human", true);
        if missing {
            rig.tmux.must(&["kill-pane", "-t", &rig.tmux.pane]);
        } else {
            rig.shell();
        }
        assert_eq!(al_pass(&rig, &mut Carry::new()), 1, "first death");
        assert_eq!(al_alerts(&rig), 1);
        for _ in 0..2 {
            let mut restarted = Carry::new();
            assert_eq!(al_pass(&rig, &mut restarted), 0, "same death after restart");
            assert_eq!(al_pass(&rig, &mut restarted), 0, "later sweep stays silent");
            assert_eq!(al_alerts(&rig), 1);
            assert_eq!(al_actions(&rig, "dead-cleared"), 0, "no false recovery");
            al_assert_open(&rig);
        }
        assert_eq!(rig.notices(), 0, "no spawner notice can act as the latch");
    }
}

#[test]
fn alertlatch_real_cycle_shell_then_missing_remains_one_human_death() {
    for restart in [false, true] {
        let rig = DnRig::new("parent", true);
        rig.shell();
        let mut carry = Carry::new();
        assert_eq!(al_pass(&rig, &mut carry), 1);
        rig.tmux.must(&["kill-pane", "-t", &rig.tmux.pane]);
        if restart {
            carry = Carry::new();
        }
        assert_eq!(al_pass(&rig, &mut carry), 0, "pane loss is the same death");
        assert_eq!(al_alerts(&rig), 1);
        assert_eq!(al_actions(&rig, "dead-cleared"), 0);
        assert_eq!(rig.notices(), 1, "spawner notice keeps its episode");
    }
}

#[test]
fn alertlatch_real_cycle_missing_then_shell_is_no_positive_recovery() {
    let rig = DnRig::new("parent", true);
    rig.tmux.must(&["kill-pane", "-t", &rig.tmux.pane]);
    assert_eq!(al_pass(&rig, &mut Carry::new()), 1);
    let _pane = al_return(&rig, false);
    assert_eq!(
        al_pass(&rig, &mut Carry::new()),
        0,
        "a shell proves no harness"
    );
    assert_eq!(al_alerts(&rig), 1);
    assert_eq!(al_actions(&rig, "dead-cleared"), 0);
    assert_eq!(rig.notices(), 1);
    al_assert_open(&rig);
}

#[test]
fn alertlatch_real_cycle_positive_harness_recovery_clears_once_and_rearms_after_restart() {
    let rig = DnRig::new("human", true);
    rig.shell();
    assert_eq!(al_pass(&rig, &mut Carry::new()), 1);
    rig.revive();
    let mut restarted = Carry::new();
    assert_eq!(al_pass(&rig, &mut restarted), 0);
    assert_eq!(al_pass(&rig, &mut restarted), 0);
    assert_eq!(al_actions(&rig, "dead-cleared"), 1, "one actual recovery");
    assert!(
        crate::session::alerts_since_in(
            &read_events(&rig.dir),
            "current",
            "spawned.0",
            "worker",
            None,
        )
        .winner()
        .is_none(),
        "positive shell-leg recovery retracts the human alert"
    );
    rig.shell();
    assert_eq!(
        al_pass(&rig, &mut Carry::new()),
        1,
        "new death alerts again"
    );
    assert_eq!(al_pass(&rig, &mut Carry::new()), 0);
    assert_eq!(al_alerts(&rig), 2);
}

#[test]
fn alertlatch_real_cycle_missing_return_and_redeath_rearms_without_a_spawner_notice() {
    for restart in [false, true] {
        let rig = DnRig::new("human", true);
        rig.tmux.must(&["kill-pane", "-t", &rig.tmux.pane]);
        let mut carry = Carry::new();
        assert_eq!(al_pass(&rig, &mut carry), 1);
        let pane = al_return(&rig, true);
        assert_eq!(al_pass(&rig, &mut carry), 0);
        assert_eq!(
            al_actions(&rig, "pane-back"),
            1,
            "durable return without a notice"
        );
        assert_eq!(al_pass(&rig, &mut carry), 0);
        assert_eq!(
            al_actions(&rig, "pane-back"),
            1,
            "book the return only once"
        );
        assert_eq!(
            al_actions(&rig, "dead-cleared"),
            0,
            "pane return retracts no alert"
        );
        al_assert_open(&rig);
        rig.tmux.must(&["kill-pane", "-t", &pane]);
        if restart {
            carry = Carry::new();
        }
        assert_eq!(al_pass(&rig, &mut carry), 1, "new missing death");
        assert_eq!(al_pass(&rig, &mut Carry::new()), 0);
        assert_eq!(al_alerts(&rig), 2);
        assert_eq!(rig.notices(), 0);
    }
}

#[test]
fn alertlatch_real_cycle_seat_owned_activity_clears_once_and_rearms() {
    let rig = DnRig::new("parent", true);
    rig.shell();
    let mut carry = Carry::new();
    assert_eq!(al_pass(&rig, &mut carry), 1);
    let line = format!(
        "{{\"ts\":\"{}\",\"actor\":\"worker\",\"actor_slot\":\"spawned.0\",\
         \"actor_session\":\"current\",\"actor_launch_id\":\"worker-first\",\
         \"action\":\"state\",\"ref\":\"working\",\"summary\":\"seat resumed\"}}\n",
        crate::time::Timestamp::now(),
    );
    crate::store::open(&rig.dir)
        .append_event(&line)
        .expect("owned activity");
    assert_eq!(
        al_pass(&rig, &mut carry),
        1,
        "cleared seat died again before the sweep"
    );
    assert_eq!(al_actions(&rig, "dead-cleared"), 1);
    assert_eq!(al_alerts(&rig), 2);
    assert_eq!(al_pass(&rig, &mut Carry::new()), 0);
    assert_eq!(rig.notices(), 2);
}

#[test]
fn alertlatch_real_cycle_ae_run_start_rearms_human_and_spawner_for_every_start_choice() {
    use crate::conversation::{Cause, Record, Start};
    for start in [
        Start::Resumed,
        Start::Unproven,
        Start::New,
        Start::Lost(Cause::NoId),
        Start::Lost(Cause::NotFound),
        Start::Lost(Cause::Unreadable),
    ] {
        let rig = DnRig::new("parent", true);
        rig.shell();
        assert_eq!(al_pass(&rig, &mut Carry::new()), 1);
        let seat = crate::effort::DiscardSeat {
            session: "current",
            session_id: "11111111-1111-4111-8111-111111111111",
            slot: "spawned.0",
            agent: "worker",
        };
        let record = Record {
            start,
            start_key: "spawned.0:worker-first:42".to_owned(),
            prior: matches!(start, Start::Lost(Cause::NotFound | Cause::Unreadable))
                .then(|| "prior-id".to_owned()),
            id: "current-id".to_owned(),
        };
        // The real _run record writer uses this line owner and grammar.
        let line = crate::effort::line(
            crate::time::Timestamp::now(),
            &seat,
            crate::conversation::action(start),
            &crate::conversation::reference(&record),
            "seat starts again",
        );
        crate::store::open(&rig.dir)
            .append_event(&format!("{line}\n"))
            .expect("owned start record");
        assert_eq!(
            al_pass(&rig, &mut Carry::new()),
            1,
            "new start {start:?} died"
        );
        assert_eq!(al_alerts(&rig), 2);
        assert_eq!(rig.notices(), 2, "new start rearms the spawner too");
        assert_eq!(al_pass(&rig, &mut Carry::new()), 0);
    }
}

#[test]
fn alertlatch_real_cycle_a_non_death_alert_does_not_suppress_the_first_death() {
    let rig = DnRig::new("human", true);
    rig.cycle()
        .emit("alert", "worker", "throttled persistently", &mut Vec::new())
        .expect("existing non-death alert");
    rig.shell();
    assert_eq!(al_pass(&rig, &mut Carry::new()), 1);
    assert_eq!(al_alerts(&rig), 1, "only a death record latches death");
    assert_eq!(al_actions(&rig, "dead-cleared"), 0);
}

#[test]
fn alertlatch_real_cycle_another_seats_ae_run_start_does_not_rearm_the_dead_seat() {
    let rig = DnRig::new("parent", true);
    rig.shell();
    assert_eq!(al_pass(&rig, &mut Carry::new()), 1);
    let seat = crate::effort::DiscardSeat {
        session: "current",
        session_id: "11111111-1111-4111-8111-111111111111",
        slot: "spawned.1",
        agent: "parent",
    };
    let line = crate::effort::line(
        crate::time::Timestamp::now(),
        &seat,
        "conversation-resumed",
        "spawned.1:parent-first:42 - current-id",
        "another seat starts again",
    );
    crate::store::open(&rig.dir)
        .append_event(&format!("{line}\n"))
        .expect("owned other-seat start");
    assert_eq!(al_pass(&rig, &mut Carry::new()), 0);
    assert_eq!(al_alerts(&rig), 1);
    assert_eq!(rig.notices(), 1);
    assert_eq!(al_actions(&rig, "dead-cleared"), 0);
    al_assert_open(&rig);
}
