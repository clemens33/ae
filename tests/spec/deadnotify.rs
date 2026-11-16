// Independent #211 acceptance. Included in watchdog_daemon::tests.
// The real Cycle runs against an owned tmux socket. Only the external send
// executable is a fixture: it books the supplied event and succeeds. Its argv
// is observed at the real transport door, so a missing dispatch fails the test.

struct DnRig {
    tmux: crate::transport::quietwd_spec::Rig,
    dir: PathBuf,
    helper: SendHelper,
}

impl DnRig {
    #[allow(clippy::disallowed_methods, reason = "owned acceptance fixture only")]
    fn new(actor: &str, parent: bool) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let tmux = crate::transport::quietwd_spec::Rig::new("deadnotify");
        let dir = tmux.path("record");
        std::fs::create_dir(&dir).expect("owned record");
        let rows = format!(
            "session=current\nsession_id=11111111-1111-4111-8111-111111111111\n\
             mode=local\nseat.spawned.0=worker\nagent_bin.spawned.0=sleep\n\
             launch_id.spawned.0=worker-first\n{}",
            if parent {
                "seat.spawned.1=parent\nagent_bin.spawned.1=sleep\nlaunch_id.spawned.1=parent-first\n"
            } else {
                ""
            }
        );
        std::fs::write(crate::store::open(&dir).meta_path(), rows).expect("owned meta");
        tmux.must(&["set-option", "-p", "-t", &tmux.pane, "@ae_agent", "worker"]);
        tmux.must(&[
            "set-option",
            "-p",
            "-t",
            &tmux.pane,
            "@ae_slot",
            "spawned.0",
        ]);
        if parent {
            let pane = tmux.must(&[
                "new-window",
                "-d",
                "-t",
                "=current:",
                "-P",
                "-F",
                "#{pane_id}",
                "sleep 600",
            ]);
            let pane = pane.trim();
            tmux.must(&["set-option", "-p", "-t", pane, "@ae_agent", "parent"]);
            tmux.must(&["set-option", "-p", "-t", pane, "@ae_slot", "spawned.1"]);
        }
        // Keep the stub external to the behavior under judgment. Its contract
        // matches the helper's supplied action, ref and addressed target.
        let helper_path = dir.join("send");
        std::fs::write(&helper_path, concat!(
            "#!/bin/sh\n",
            "printf '{\"ts\":\"%s\",\"actor\":\"watchdog\",\"action\":\"%s\",\"target\":\"%s\",\"ref\":\"%s\",\"summary\":\"%s\"}\\n' ",
            "\"$(date -u +%Y-%m-%dT%H:%M:%SZ)\" \"$_AE_EVENT_ACTION\" \"$1\" \"$_AE_EVENT_REF\" \"$_AE_EVENT_SUMMARY\" >> \"$(dirname \"$0\")/events.jsonl\"\n"
        )).expect("owned external helper");
        std::fs::set_permissions(&helper_path, std::fs::Permissions::from_mode(0o755))
            .expect("helper mode");
        let fixture = Self {
            tmux,
            dir,
            helper: SendHelper(helper_path),
        };
        fixture.append(&format!(
            "{{\"ts\":\"2026-11-15T00:00:00Z\",\"actor\":\"{actor}\",\"action\":\"spawn\",\"target\":\"worker\",\"birth\":\"worker-first\"}}"
        ));
        fixture
    }

    fn append(&self, line: &str) {
        crate::store::open(&self.dir)
            .append_event(&format!("{line}\n"))
            .expect("owned event");
        assert_eq!(
            read_events(&self.dir).len(),
            1,
            "spawn fixture must parse before the first cycle"
        );
    }

    fn cycle(&self) -> Cycle<'_> {
        let mut cycle = demo_cycle(&self.dir, &self.helper, &self.tmux.server);
        cycle.session = "current";
        cycle.knobs.quota_aware = false;
        cycle.knobs.idle_nudge_secs = 0;
        cycle.meta_agent = false;
        let bytes = crate::meta::read_bytes(&self.dir).expect("owned meta bytes");
        let meta = Meta::parse(&String::from_utf8_lossy(&bytes));
        cycle.roster = meta.roster().to_vec();
        cycle.launch_ids = vec![
            ("spawned.0".into(), "worker-first".into()),
            ("spawned.1".into(), "parent-first".into()),
        ];
        cycle
    }

    fn pass(&self, carry: &mut Carry) -> Vec<Vec<String>> {
        let (result, calls) =
            crate::transport::spawns_during(|| self.cycle().run(carry, &mut Vec::new()));
        result.expect("real watchdog cycle");
        calls
            .into_iter()
            .filter(|args| {
                args.first()
                    .is_some_and(|p| p == &self.helper.path().display().to_string())
            })
            .collect()
    }

    fn command(&self, command: &str, expected: &str) {
        self.tmux
            .must(&["respawn-pane", "-k", "-t", &self.tmux.pane, command]);
        let end = std::time::Instant::now() + Duration::from_mins(1);
        loop {
            let actual = self.tmux.must(&[
                "display-message",
                "-p",
                "-t",
                &self.tmux.pane,
                "#{pane_current_command}",
            ]);
            if actual.trim() == expected {
                return;
            }
            assert!(
                std::time::Instant::now() < end,
                "fixture command did not settle: {actual}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn shell(&self) {
        self.command("/bin/sh", "sh");
    }

    fn revive(&self) {
        // A harness is a descendant of the pane shell, not the pane root.
        self.command("/bin/sh -c 'sleep 600; :'", "sh");
        let end = std::time::Instant::now() + Duration::from_mins(1);
        loop {
            let panes = crate::transport::observe_watch_panes(&self.tmux.server, "current")
                .expect("owned panes");
            let pid = panes
                .iter()
                .find(|p| p.pane_id == self.tmux.pane)
                .and_then(|p| p.pane_pid);
            let table = crate::procs::snapshot();
            if matches!((pid, table.as_deref()), (Some(pid), Some(table)) if crate::procs::has_descendant_named(table, pid, "sleep"))
            {
                return;
            }
            assert!(
                std::time::Instant::now() < end,
                "fixture must prove the harness descendant returned"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn assert_notice(calls: &[Vec<String>], state: &str) {
        assert_eq!(calls.len(), 1, "one dispatch to the spawner: {calls:?}");
        assert_eq!(
            calls[0][1], "parent",
            "the recorded spawner, not the lead by default"
        );
        assert!(
            calls[0][2].contains("worker"),
            "notice names worker: {calls:?}"
        );
        assert!(
            calls[0][2].to_ascii_lowercase().contains(state),
            "notice names observed state {state}: {calls:?}"
        );
    }

    fn notices(&self) -> usize {
        read_events(&self.dir)
            .iter()
            .filter(|e| e.action == "dead-notice")
            .count()
    }
}

#[test]
fn deadnotify_real_cycle_notifies_spawner_once_and_preserves_human_alert() {
    let rig = DnRig::new("parent", true);
    rig.shell();
    let mut carry = Carry::new();
    let calls = rig.pass(&mut carry);
    DnRig::assert_notice(&calls, "shell");
    assert_eq!(rig.notices(), 1);
    assert!(rig.pass(&mut carry).is_empty(), "same death is silent");
    let events = read_events(&rig.dir);
    let alerts: Vec<_> = events
        .iter()
        .filter(|e| e.action == "alert" && e.target.as_deref() == Some("worker"))
        .collect();
    assert_eq!(alerts.len(), 1, "one unchanged human alert");
    assert_eq!(
        alerts[0].summary.as_deref(),
        Some("agent process dead — dropped to shell")
    );
}

#[test]
fn deadnotify_real_cycle_suppresses_non_agent_retired_unknown_and_self_spawners() {
    for (actor, parent) in [
        ("human", true),
        ("retired", false),
        ("unverified", true),
        ("worker", true),
    ] {
        let rig = DnRig::new(actor, parent);
        rig.shell();
        let mut carry = Carry::new();
        assert!(rig.pass(&mut carry).is_empty(), "no recipient for {actor}");
        assert!(
            rig.pass(&mut carry).is_empty(),
            "no later recipient for {actor}"
        );
        assert_eq!(rig.notices(), 0);
    }
}

#[test]
#[allow(clippy::disallowed_methods, reason = "owned launch stamp")]
fn deadnotify_real_cycle_does_not_treat_launch_grace_as_death() {
    let rig = DnRig::new("parent", true);
    rig.shell();
    std::fs::write(
        rig.dir.join(".launch-attempt"),
        format!("{}\n", crate::time::Timestamp::now().epoch()),
    )
    .expect("owned stamp");
    assert!(rig.pass(&mut Carry::new()).is_empty());
    assert_eq!(rig.notices(), 0);
    assert!(!read_events(&rig.dir).iter().any(|e| e.action == "alert"));
}

#[test]
fn deadnotify_accounting_keeps_auto_reseat_non_death_and_human_notification_exact() {
    let mut observation = seen();
    observation.is_dead = true;
    observation.descendancy = Descendancy::Absent;
    observation.auto_in_flight = Some(crate::autoreseat::Trigger::Limit);
    let held = account(&PaneState::default(), &observation, &Knobs::default());
    assert!(!held.next.dead_latched);
    assert!(
        !held
            .effects
            .iter()
            .any(|e| matches!(e, Effect::Notify(s) if s.contains("DEAD")))
    );
    observation.auto_in_flight = None;
    let dead = account(&PaneState::default(), &observation, &Knobs::default());
    assert!(dead.next.dead_latched);
    let notify: Vec<_> = dead
        .effects
        .iter()
        .filter_map(|e| {
            if let Effect::Notify(s) = e {
                Some(s.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(notify, ["is DEAD — process dropped to shell"]);
}

#[test]
fn deadnotify_real_cycle_revived_then_dead_is_a_new_episode() {
    let rig = DnRig::new("parent", true);
    let mut carry = Carry::new();
    rig.shell();
    DnRig::assert_notice(&rig.pass(&mut carry), "shell");
    rig.revive();
    assert!(
        rig.pass(&mut carry).is_empty(),
        "revival alone sends no death notice"
    );
    rig.shell();
    DnRig::assert_notice(&rig.pass(&mut carry), "shell");
    assert!(rig.pass(&mut carry).is_empty());
    assert_eq!(rig.notices(), 2);
}

#[test]
fn deadnotify_real_cycle_restart_does_not_repeat_a_delivered_episode() {
    let rig = DnRig::new("parent", true);
    rig.shell();
    DnRig::assert_notice(&rig.pass(&mut Carry::new()), "shell");
    assert!(
        rig.pass(&mut Carry::new()).is_empty(),
        "journal survives daemon restart"
    );
    assert_eq!(rig.notices(), 1);
}

#[test]
fn deadnotify_real_cycle_settle_preserves_owed_notice_until_first_free_cycle() {
    let rig = DnRig::new("parent", true);
    rig.shell();
    let mut carry = Carry::new();
    carry.clock = Some(crate::watchdog::CycleClock {
        wall: crate::time::Timestamp::now().epoch() - 7_200,
        mono: std::time::Instant::now(),
    });
    assert!(
        rig.pass(&mut carry).is_empty(),
        "suspension cycle holds notice"
    );
    assert!(
        rig.pass(&mut carry).is_empty(),
        "second settle cycle holds notice"
    );
    DnRig::assert_notice(&rig.pass(&mut carry), "shell");
    assert!(rig.pass(&mut carry).is_empty());
    assert_eq!(rig.notices(), 1);
}

#[test]
fn deadnotify_real_cycle_pane_gone_notifies_once() {
    let rig = DnRig::new("parent", true);
    rig.tmux.must(&["kill-pane", "-t", &rig.tmux.pane]);
    let mut carry = Carry::new();
    DnRig::assert_notice(&rig.pass(&mut carry), "pane");
    assert!(rig.pass(&mut carry).is_empty());
    assert!(
        rig.pass(&mut Carry::new()).is_empty(),
        "missing notice restart dedupe"
    );
    assert_eq!(rig.notices(), 1);
}

#[test]
fn deadnotify_real_cycle_shell_then_missing_is_one_episode() {
    let rig = DnRig::new("parent", true);
    rig.shell();
    let mut carry = Carry::new();
    DnRig::assert_notice(&rig.pass(&mut carry), "shell");
    rig.tmux.must(&["kill-pane", "-t", &rig.tmux.pane]);
    assert!(
        rig.pass(&mut carry).is_empty(),
        "pane loss after process death does not create another episode"
    );
    assert_eq!(rig.notices(), 1);
}

#[test]
fn deadnotify_real_cycle_missing_then_live_then_missing_rearms_notice_only() {
    let rig = DnRig::new("parent", true);
    rig.tmux.must(&["kill-pane", "-t", &rig.tmux.pane]);
    let mut carry = Carry::new();
    DnRig::assert_notice(&rig.pass(&mut carry), "pane");
    let pane = rig.tmux.must(&[
        "new-window",
        "-d",
        "-t",
        "=current:",
        "-P",
        "-F",
        "#{pane_id}",
        "sleep 600",
    ]);
    let pane = pane.trim();
    rig.tmux
        .must(&["set-option", "-p", "-t", pane, "@ae_agent", "worker"]);
    rig.tmux
        .must(&["set-option", "-p", "-t", pane, "@ae_slot", "spawned.0"]);
    let before = crate::session::alerts_since_in(
        &read_events(&rig.dir),
        "current",
        "spawned.0",
        "worker",
        None,
    );
    assert!(
        before.winner().is_some(),
        "missing-pane alert is open before return"
    );
    assert!(
        rig.pass(&mut carry).is_empty(),
        "a live return sends no death notice"
    );
    let returned = read_events(&rig.dir);
    assert_eq!(
        crate::session::alerts_since_in(&returned, "current", "spawned.0", "worker", None),
        before,
        "pane return changes no human alert"
    );
    let backs: Vec<_> = returned
        .iter()
        .filter(|e| e.action == "pane-back" && e.target.as_deref() == Some("worker"))
        .collect();
    assert_eq!(backs.len(), 1, "one durable worker return record");
    assert_eq!(
        backs[0].alert_meaning(),
        crate::events::AlertMeaning::Undefined
    );
    // A new daemon must learn the rearm from the journal, not the old carry.
    carry = Carry::new();
    rig.tmux.must(&["kill-pane", "-t", pane]);
    DnRig::assert_notice(&rig.pass(&mut carry), "pane");
    assert!(rig.pass(&mut carry).is_empty());
    assert_eq!(rig.notices(), 2);
    let alerts = read_events(&rig.dir)
        .into_iter()
        .filter(|e| {
            e.action == "alert"
                && e.summary.as_deref() == Some("pane missing — agent no longer visible in session")
        })
        .count();
    assert_eq!(
        alerts, 2,
        "existing missing-pane human alert remains unchanged"
    );
}

// This test makes no new-notice assertion. Its passing main receipt pins the
// existing human behavior independently of the feature's failing acceptance.
#[test]
fn deadnotify_baseline_missing_human_alert_repeats_after_daemon_restart() {
    let rig = DnRig::new("parent", true);
    rig.tmux.must(&["kill-pane", "-t", &rig.tmux.pane]);
    rig.pass(&mut Carry::new());
    rig.pass(&mut Carry::new());
    let alerts = read_events(&rig.dir)
        .into_iter()
        .filter(|e| {
            e.action == "alert"
                && e.summary.as_deref() == Some("pane missing — agent no longer visible in session")
        })
        .count();
    assert_eq!(
        alerts, 2,
        "the unchanged human missing-alert latch is memory only"
    );
}

#[test]
fn deadnotify_real_cycle_settle_holds_missing_notice_too() {
    let rig = DnRig::new("parent", true);
    rig.tmux.must(&["kill-pane", "-t", &rig.tmux.pane]);
    let mut carry = Carry::new();
    carry.clock = Some(crate::watchdog::CycleClock {
        wall: crate::time::Timestamp::now().epoch() - 7_200,
        mono: std::time::Instant::now(),
    });
    assert!(rig.pass(&mut carry).is_empty());
    assert!(rig.pass(&mut carry).is_empty());
    DnRig::assert_notice(&rig.pass(&mut carry), "pane");
    assert!(rig.pass(&mut carry).is_empty());
    assert_eq!(rig.notices(), 1);
}

#[test]
#[allow(clippy::disallowed_methods, reason = "owned external failure oracle")]
fn deadnotify_real_cycle_retries_only_proven_pre_submit_failures_and_stops_at_three() {
    for retryable in [false, true] {
        let rig = DnRig::new("parent", true);
        rig.shell();
        // This external dependency refuses without ever touching a pane. The
        // marker alone is the existing helper contract for a safe retry.
        let script = if retryable {
            "#!/bin/sh\nprintf 'ae-send: retryable-before-submit\\n'\nexit 1\n"
        } else {
            "#!/bin/sh\nexit 1\n"
        };
        std::fs::write(rig.helper.path(), script).expect("owned regular stub");
        let mut carry = Carry::new();
        let counts: Vec<_> = (0..5).map(|_| rig.pass(&mut carry).len()).collect();
        assert_eq!(
            counts,
            if retryable {
                vec![1, 1, 1, 0, 0]
            } else {
                vec![1, 0, 0, 0, 0]
            }
        );
        assert_eq!(
            rig.notices(),
            0,
            "a refusal is never reported as a delivery"
        );
    }
}
