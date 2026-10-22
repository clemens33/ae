// Frozen unreach addendum a1. Include at EOF of tests/it/autotrust_spec.rs.
mod unreach_recovery_a1 {
    use super::*;
    use ae::json::Value;
    use ae::time::Timestamp;

    const HINT: &str = "k keep & next · esc dismiss & close · ⌃o copy · ←/→ warning · ↓ scroll";

    fn append(rig: &Rig, at: i64, actor: &str, action: &str, summary: &str, reference: &str) {
        let mut fields = vec![
            (
                "ts".into(),
                Value::str(Timestamp::from_epoch(at).to_string()),
            ),
            ("actor".into(), Value::str(actor)),
            ("action".into(), Value::str(action)),
            ("summary".into(), Value::str(summary)),
            ("ref".into(), Value::str(reference)),
        ];
        if actor == "lead" {
            fields.extend([
                ("actor_slot".into(), Value::str("main")),
                ("actor_session".into(), Value::str("s")),
                ("actor_session_id".into(), Value::str(UUID)),
            ]);
        } else {
            fields.extend([
                ("target".into(), Value::str("lead")),
                ("target_slot".into(), Value::str("main")),
                ("target_session".into(), Value::str("s")),
                ("target_session_id".into(), Value::str(UUID)),
            ]);
        }
        write(
            rig.dir.join("events.jsonl"),
            text(rig.dir.join("events.jsonl")) + &Value::Obj(fields).render() + "\n",
        );
    }

    fn start(rig: &mut Rig) {
        let command = rig
            .command(ae::cli::WATCHDOG_RUN)
            .arg("--interval")
            .arg("1")
            .arg("--quota-every-secs")
            .arg("0")
            .arg("--idle-nudge-secs")
            .arg("1")
            .arg("--sweep-secs")
            .arg("0")
            .arg("--tg-supervise-secs")
            .arg("0");
        let root = rig.root.clone();
        rig.daemon = Some(std::thread::spawn(move || {
            let _ = raw::run(
                &command,
                &root,
                &root.join("watch.out"),
                &root.join("watch.err"),
            );
        }));
    }

    fn roster(rig: &Rig) -> Vec<ae::tmux::PickerAgent> {
        let value = rig.tmux(&["show-option", "-qv", "-t", "s", "@ae_agents"]);
        ae::tmux::parse_picker_agents(value.trim(), Timestamp::now().epoch()).unwrap_or_default()
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one real daemon episode proves alert, successful recovery delivery, and all published clears"
    )]
    fn unreach_landed_recovery_challenge_clears_episode_and_roster_and_list() {
        for (state, automatic) in [("done", false), ("blocked", true)] {
            let mut rig = Rig::new(&format!("ur-recover-{state}"), "codex", "normal");
            write(
                rig.root.join("config"),
                text(rig.root.join("config")).replace("auto_trust = on", "auto_trust = off"),
            );
            rig.replace_meta("idle_nudge_secs", "1");
            rig.replace_meta("done_confirmations", "2");
            let now = Timestamp::now().epoch();
            let start_ref = format!("main:start-one:{}", text(rig.root.join("ready")).trim());
            append(
                &rig,
                now - 400,
                "watchdog",
                "human-prompt",
                &format!("Warnings · 1 of 1 · Startup — press: {HINT}"),
                &start_ref,
            );
            append(
                &rig,
                now - 300,
                "lead",
                "state",
                "unreach recovery acceptance",
                state,
            );
            let action = if state == "done" {
                "done-challenge"
            } else {
                "wait-challenge"
            };
            let summary = if state == "done" {
                "[unconfirmed] confirmation 1/2; refused pre-paste: composer unreadable"
            } else {
                "[unconfirmed] blocked confirmation 1/2; refused pre-paste: composer unreadable"
            };
            for age in [240, 180, 120] {
                append(&rig, now - age, "watchdog", action, summary, "start-one");
            }
            start(&mut rig);
            wait_for("durable unreachable published", || {
                roster(&rig)
                    .iter()
                    .any(|a| a.name == "lead" && a.state == "unreachable")
            });
            assert!(
                rig.wire().is_empty(),
                "unreadable panel gets no recovery paste"
            );
            if automatic {
                write(
                    rig.root.join("config"),
                    text(rig.root.join("config")).replace("auto_trust = off", "auto_trust = on"),
                );
                wait_for("ae dismisses aged panel", || rig.count("auto-dismiss") == 1);
            } else {
                // Human Escape is supplied through the typed product transport
                // to this fixture's private socket, never a live session.
                let server = ae::inventory::ServerId::Selected(ae::meta::Selector::Socket(
                    rig.root.join("sock"),
                ));
                assert!(ae::transport::send_key(
                    &server,
                    &rig.pane,
                    ae::tmux::Key::Escape
                ));
            }
            wait_for("recovery delivery and clear", || {
                let events = rig.events();
                events.iter().any(|e| {
                    e.action == action
                        && !e
                            .summary
                            .as_deref()
                            .unwrap_or_default()
                            .starts_with("[unconfirmed]")
                }) && events.iter().any(|e| {
                    e.action == "alert-cleared"
                        && e.summary
                            .as_deref()
                            .is_some_and(|s| s.contains("reachable again"))
                })
            });
            wait_for("roster leaves unreachable", || {
                roster(&rig)
                    .iter()
                    .any(|a| a.name == "lead" && a.state != "unreachable")
            });
            rig.stop();
            let events = rig.events();
            let failure = events
                .iter()
                .position(|e| {
                    e.action == "alert"
                        && e.summary
                            .as_deref()
                            .is_some_and(|s| s.starts_with("nudge unreachable/occupied"))
                })
                .expect("original failure episode");
            let landed = events
                .iter()
                .position(|e| {
                    e.action == action
                        && !e
                            .summary
                            .as_deref()
                            .unwrap_or_default()
                            .starts_with("[unconfirmed]")
                })
                .expect("actual helper verified recovery delivery");
            let clears = events
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    e.action == "alert-cleared"
                        && e.summary
                            .as_deref()
                            .is_some_and(|s| s.contains("reachable again"))
                })
                .collect::<Vec<_>>();
            assert_eq!(clears.len(), 1, "one durable recovery clear");
            assert!(
                failure < landed && landed < clears[0].0,
                "delivery, then clear"
            );
            assert!(
                events[failure + 1..].iter().all(|e| e.actor != "lead"),
                "a newer own record did not manufacture recovery"
            );
            assert_eq!(clears[0].1.actor, "watchdog");
            assert!(
                ae::session::alert_reason_in(&events, "s", "main", "lead").is_none(),
                "standing failure is gone"
            );
            assert!(
                text(rig.root.join("submitted")).contains("confirmation"),
                "recovery challenge reached actual fake-tool stdin"
            );
            assert_eq!(rig.count("auto-dismiss"), usize::from(automatic));
            let (code, out, err) = rig.invoke(rig.environment().arg("list"), "recovered-list");
            assert_eq!(code, 0, "{err}");
            assert!(
                !out.contains("attn:unreachable"),
                "list clears same failure: {out}"
            );
            assert!(!out.contains("unreachable"), "seat word cleared: {out}");
            assert!(
                roster(&rig)
                    .iter()
                    .all(|a| a.name != "lead" || a.state != "unreachable")
            );
        }
    }
}
