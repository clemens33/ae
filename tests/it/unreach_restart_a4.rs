// Frozen unreach addendum a4. Include at EOF of autotrust_spec.rs.
#[cfg(test)]
mod unreach_restart_a4 {
    use super::*;
    use ae::json::Value;
    use ae::time::Timestamp;

    fn record(rig: &Rig, action: &str, summary: &str, reference: &str) {
        let line = Value::Obj(vec![
            ("ts".into(), Value::str(Timestamp::now().to_string())),
            ("actor".into(), Value::str("watchdog")),
            ("action".into(), Value::str(action)),
            ("target".into(), Value::str("lead")),
            ("target_slot".into(), Value::str("main")),
            ("target_session".into(), Value::str("s")),
            ("target_session_id".into(), Value::str(UUID)),
            ("ref".into(), Value::str(reference)),
            ("summary".into(), Value::str(summary)),
        ]);
        write(
            rig.dir.join("events.jsonl"),
            text(rig.dir.join("events.jsonl")) + &line.render() + "\n",
        );
    }

    fn no_auto(rig: &Rig) {
        write(
            rig.root.join("config"),
            text(rig.root.join("config")).replace("auto_trust = on", "auto_trust = off"),
        );
    }

    #[test]
    fn unreach_restart_clears_a_removed_panel_even_when_failure_outranks_its_naming() {
        let mut rig = Rig::new("ur-clear-restart", "codex", "normal");
        no_auto(&rig);
        rig.start();
        rig.named();
        rig.stop();
        let start = format!("main:start-one:{}", text(rig.root.join("ready")).trim());
        record(
            &rig,
            "alert",
            "nudge unreachable/occupied — 3 undelivered attempts (idle 5m)",
            &start,
        );
        rig.repaint(include_str!(
            "../fixtures/codex-composer/codex-idle-0.156.1-200x40.esc"
        ));
        rig.start();
        wait_for(
            "fresh daemon clears durable panel naming under Stale",
            || rig.count("human-prompt-cleared") > 0,
        );
        std::thread::sleep(Duration::from_secs(3));
        rig.stop();
        assert_eq!(rig.count("human-prompt-cleared"), 1, "one panel-run end");
        assert_eq!(rig.count("auto-dismiss"), 0);
        assert_eq!(
            ae::session::alert_reason_in(&rig.events(), "s", "main", "lead"),
            Some(ae::attention::Reason::Stale),
            "panel removal alone does not manufacture successful delivery"
        );
    }

    #[test]
    fn unreach_proven_new_tool_start_ends_failure_but_same_or_unknown_start_does_not() {
        for case in ["same", "other", "legacy", "unproven"] {
            let mut rig = Rig::new(&format!("ur-start-{case}"), "codex", "normal");
            no_auto(&rig);
            let current = format!("main:start-one:{}", text(rig.root.join("ready")).trim());
            let reference = match case {
                "same" => current.as_str(),
                "legacy" => "",
                _ => "main:older-launch:1",
            };
            if case == "unproven" {
                rig.replace_meta("launch_id.main", "");
            }
            record(
                &rig,
                "alert",
                "nudge unreachable/occupied — 3 undelivered attempts (idle 5m)",
                reference,
            );
            rig.start();
            rig.named();
            rig.stop();
            let clears = rig
                .events()
                .into_iter()
                .filter(|event| {
                    event.action == "alert-cleared"
                        && event.summary.as_deref().is_some_and(|summary| {
                            summary.starts_with("reachable again")
                                && summary.contains("new tool start")
                        })
                })
                .count();
            assert_eq!(clears, usize::from(case == "other"), "{case}");
            assert!(rig.wire().is_empty(), "start reconciliation sends no key");
        }
    }
}
