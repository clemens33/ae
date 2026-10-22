// Frozen unreach addendum a3. Include at EOF of tests/it/honestwords_spec.rs.
#[cfg(test)]
mod unreach_agreement_a3 {
    use super::*;
    use ae::attention::Reason;

    #[test]
    fn unreach_panel_naming_and_clear_preserve_one_failure_reason_and_word_until_delivery() {
        let targeted = |action: &str, summary: &str, age: i64| {
            Event::parse_line(
                &Value::Obj(vec![
                    (
                        "ts".into(),
                        Value::str(Timestamp::from_epoch(NOW - age).to_string()),
                    ),
                    ("actor".into(), Value::str("watchdog")),
                    ("action".into(), Value::str(action)),
                    ("target".into(), Value::str("lead")),
                    ("target_slot".into(), Value::str("main")),
                    ("target_session".into(), Value::str("one")),
                    ("target_session_id".into(), Value::str(UUID)),
                    ("summary".into(), Value::str(summary)),
                ])
                .render(),
            )
            .expect("synthetic routed watchdog record")
        };
        let mut events = vec![
            declaration("done", "review complete", 720, "lead"),
            targeted(
                "alert",
                "nudge unreachable/occupied — 3 undelivered attempts (idle 5m)",
                600,
            ),
        ];
        for (action, summary) in [
            (
                "human-prompt",
                "Warnings · 1 of 1 · Startup — press: k keep & next · esc dismiss & close · ⌃o copy · ←/→ warning · ↓ scroll",
            ),
            (
                "human-prompt-cleared",
                "the human-only prompt is gone from the pane",
            ),
        ] {
            events.push(targeted(action, summary, 10));
            let snapshot = entry(Some(events.clone()));
            assert_eq!(snapshot.attention, Some(Reason::Stale), "{action}");
            assert_eq!(snapshot.agents[0].reason, Some(Reason::Stale), "{action}");
            let text = ae::listing::table_at(&[&snapshot], now());
            assert!(text.contains("attn:unreachable"), "{action}: {text}");
            let seat = text
                .lines()
                .find(|row| row.trim_start().starts_with("lead "))
                .expect("lead seat row");
            assert!(seat.contains("unreachable"), "{action}: {seat}");
        }
        events.push(targeted("alert-cleared", "reachable again", 1));
        let recovered = entry(Some(events));
        assert_eq!(recovered.attention, None);
        assert_eq!(recovered.agents[0].reason, None);
        assert!(
            !ae::listing::table_at(&[&recovered], now()).contains("unreachable"),
            "a landed-delivery clear ends both the reason and the word"
        );
    }
}
