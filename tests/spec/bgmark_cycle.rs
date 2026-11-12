// Include in watchdog_daemon.rs's existing cfg(test) mod tests.
// Contract: absent or negative task proof never changes a quiet seat to Background.
mod bgmark_cycle_acceptance {
    use super::*;
    use crate::time::Timestamp;
    use std::path::Path;
    use std::time::{Duration, Instant};

    fn cycle_without_external_actions<'a>(
        rig: &'a crate::transport::quietwd_spec::Rig,
        meta: &'a Path,
        helper: &'a SendHelper,
        binary: Option<&str>,
    ) -> Cycle<'a> {
        let mut seat = entry("main", "p", "lead");
        seat.binary = binary.map(str::to_owned);
        Cycle {
            knobs: Knobs {
                quota_aware: false,
                quota_every_secs: 0,
                idle_nudge_secs: 0,
                tg_supervise_secs: 0,
                ..Knobs::default()
            },
            meta_dir: meta,
            helper,
            server: &rig.server,
            session: "current",
            goal: None,
            roster: vec![seat],
            local_config: None,
            lead_pair: false,
            fleet_order: crate::theme::FleetOrder::EMPTY,
            auto: crate::autoreseat::settings(None),
            auto_trust: crate::config::AutoTrust::Off,
            meta_agent: false,
            launch_ids: Vec::new(),
            started: None,
            settled: None,
        }
    }

    #[test]
    fn bgmark_real_cycle_without_task_proof_keeps_done_and_an_empty_task_cell() {
        let rig = crate::transport::quietwd_spec::Rig::new("bgmark-no-task");
        rig.must(&["set-option", "-p", "-t", &rig.pane, "@ae_slot", "main"]);
        for (key, value) in [
            ("@ae_icons", "on"),
            ("@ae_palette", "darcula"),
            ("@ae_look", "on"),
            ("@ae_motion", "off"),
        ] {
            rig.must(&["set-option", "-t", "=current:", key, value]);
        }
        // The real fixture pane runs sleep, with no child job. Wait for its exec,
        // so an intermediate startup shell cannot manufacture a dead verdict.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let command = rig.must(&[
                "display-message",
                "-p",
                "-t",
                &rig.pane,
                "#{pane_current_command}",
            ]);
            if command.trim() == "sleep" {
                break;
            }
            assert!(Instant::now() < deadline, "sleep did not start: {command}");
            std::thread::sleep(Duration::from_millis(10));
        }
        let pid: u32 = rig
            .must(&["display-message", "-p", "-t", &rig.pane, "#{pane_pid}"])
            .trim()
            .parse()
            .expect("real fixture pane pid");
        let meta = rig.path("meta");
        std::fs::create_dir(&meta).expect("private metadata directory");
        let helper = SendHelper::for_session(&meta);
        for (binary, expected_reading) in [(Some("sleep"), Some(false)), (None, None)] {
            let recorded = binary.map_or(String::new(), |bin| format!("agent_bin.main={bin}\n"));
            std::fs::write(
                meta.join("meta"),
                format!("mode=local\nsession=current\nseat.main=lead\nprofile.main=p\n{recorded}"),
            )
            .expect("matching recorded seat");
            std::fs::write(
                meta.join("events.jsonl"),
                format!(
                    "{{\"ts\":\"{}\",\"actor\":\"p:lead\",\"action\":\"state\",\"ref\":\"done\",\"summary\":\"finished\",\"actor_slot\":\"main\",\"actor_session\":\"current\"}}\n",
                    Timestamp::now()
                ),
            )
            .expect("real quiet declaration");
            let before = crate::procs::stat_snapshot();
            assert_eq!(
                binary.and_then(|bin| crate::procs::task_running(before.as_deref(), pid, bin)),
                expected_reading,
                "fixture must exercise the negative or unknown task reading"
            );
            rig.must(&[
                "set-option",
                "-p",
                "-t",
                &rig.pane,
                "@ae_pane_state",
                "opposed",
            ]);
            rig.must(&["set-option", "-t", "=current:", "@ae_agents", "opposed"]);
            let mut carry = Carry::new();
            carry.pending = super::super::PendingCycle::new(
                crate::transport::observe_session_identity(&rig.server, "current"),
            );
            let mut err = Vec::new();
            cycle_without_external_actions(&rig, &meta, &helper, binary)
                .run(&mut carry, &mut err)
                .expect("real watchdog cycle");
            super::super::flush_cycle(&rig.server, &mut carry);
            let state = rig.option("-p", &rig.pane, "@ae_pane_state");
            let fact = rig.option("", "=current:", "@ae_agents");
            assert!(
                state.contains("✓#[default] finished"),
                "{binary:?}: {state}; {fact}"
            );
            assert!(!state.contains('◉'), "unproven job: {state}");
            assert!(
                fact.starts_with("v3;"),
                "real cycle must publish v3: {fact}"
            );
            let row = fact.split(';').nth(3).expect("published seat");
            assert_eq!(row.split(':').count(), 9, "v3 row: {row}");
            assert_eq!(row.split(':').nth(8), Some(""), "unproven job: {fact}");
            let parsed = crate::tmux::parse_picker_agents(&fact, Timestamp::now().epoch())
                .expect("real cycle fact parses");
            assert_eq!(parsed.len(), 1);
            assert_eq!(parsed[0].state, "done");
            assert!(!parsed[0].task, "unproven job: {fact}");
        }
    }
}
