// Frozen unreach addendum a2. Include at EOF of tests/it/autotrust_spec.rs.
#[cfg(test)]
mod unreach_guard_a2 {
    use super::*;
    use ae::json::Value;
    use ae::time::Timestamp;

    #[test]
    fn unreach_model_body_movement_with_unchanged_title_and_hint_holds_escape() {
        let model = include_str!("../fixtures/codex-warning/model-80.txt");
        let alternate = model.replace("gpt-6-luna", "gpt-6-sol");
        assert_ne!(model, alternate);
        for frame in [model, alternate.as_str()] {
            let prompt = ae::watchdog::human_prompt_class(frame, "codex")
                .expect("measured model body with bounded model slots");
            assert_eq!(prompt.question, "Warnings · 2 of 2 · Startup");
            assert!(prompt.dismiss, "both bodies are eligible individually");
        }

        let mut rig = Rig::new("ur-body-moves", "codex", "normal");
        rig.repaint(model);
        let reference = format!("main:start-one:{}", text(rig.root.join("ready")).trim());
        let record = Value::Obj(vec![
            (
                "ts".into(),
                Value::str(Timestamp::from_epoch(Timestamp::now().epoch() - 120).to_string()),
            ),
            ("actor".into(), Value::str("watchdog")),
            ("action".into(), Value::str("human-prompt")),
            ("target".into(), Value::str("lead")),
            ("target_slot".into(), Value::str("main")),
            ("target_session".into(), Value::str("s")),
            ("target_session_id".into(), Value::str(UUID)),
            ("ref".into(), Value::str(reference)),
            (
                "summary".into(),
                Value::str(
                    "Warnings · 2 of 2 · Startup — press: k keep & next · esc dismiss & close · ⌃o copy · ←/→ warning · ↓ scroll",
                ),
            ),
        ]);
        write(
            rig.dir.join("events.jsonl"),
            text(rig.dir.join("events.jsonl")) + &record.render() + "\n",
        );
        write(
            rig.root.join("guard-lock"),
            rig.root
                .join("sessions/.locks")
                .join(format!("send-lock-{}", rig.pane.replace('%', "_")))
                .display()
                .to_string(),
        );
        write(rig.root.join("frame"), model);
        write(rig.root.join("alternate-frame"), alternate);
        write(rig.root.join("wrapper"), "panel-moves");
        rig.start();
        std::thread::sleep(Duration::from_secs(6));
        rig.stop();
        assert!(
            text(rig.root.join("locked-captures")).lines().count() >= 2,
            "moving bodies were actually observed under the send lock"
        );
        assert!(rig.wire().is_empty(), "body movement sends zero keys");
        assert_eq!(rig.count("auto-dismiss-reserved"), 0, "pre-key hold");
        assert_eq!(rig.count("auto-dismiss"), 0);
    }
}
