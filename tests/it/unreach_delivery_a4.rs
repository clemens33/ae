// Frozen unreach addendum a4. Include at EOF of honestwords_spec.rs.
#[cfg(test)]
mod unreach_delivery_a4 {
    use super::*;
    use ae::attention::Reason;

    #[test]
    fn unreach_any_landed_helper_delivery_ends_failure_but_unconfirmed_does_not() {
        let targeted = |actor: &str, action: &str, summary: &str, age: i64| {
            Event::parse_line(
                &Value::Obj(vec![
                    (
                        "ts".into(),
                        Value::str(Timestamp::from_epoch(NOW - age).to_string()),
                    ),
                    ("actor".into(), Value::str(actor)),
                    ("action".into(), Value::str(action)),
                    ("target".into(), Value::str("lead")),
                    ("target_slot".into(), Value::str("main")),
                    ("target_session".into(), Value::str("one")),
                    ("target_session_id".into(), Value::str(UUID)),
                    ("summary".into(), Value::str(summary)),
                ])
                .render(),
            )
            .expect("synthetic routed delivery record")
        };
        for action in [
            "send",
            "ask",
            "review",
            "reply",
            "nudge",
            "done-challenge",
            "wait-challenge",
        ] {
            let actor = if action.ends_with("challenge") || action == "nudge" {
                "watchdog"
            } else {
                "worker"
            };
            for confirmed in [false, true] {
                let summary = if confirmed {
                    "the helper delivered this body"
                } else {
                    "[unconfirmed] the helper could not confirm submit"
                };
                let events = vec![
                    declaration("done", "review complete", 720, "lead"),
                    targeted(
                        "watchdog",
                        "alert",
                        "nudge unreachable/occupied — 3 undelivered attempts (idle 5m)",
                        600,
                    ),
                    targeted(actor, action, summary, 1),
                ];
                let snapshot = entry(Some(events));
                let expected = (!confirmed).then_some(Reason::Stale);
                assert_eq!(snapshot.attention, expected, "{action}, {confirmed}");
                assert_eq!(snapshot.agents[0].reason, expected, "{action}, {confirmed}");
                let rendered = ae::listing::table_at(&[&snapshot], now());
                assert_eq!(
                    rendered.contains("unreachable"),
                    !confirmed,
                    "{action}, {confirmed}: {rendered}"
                );
            }
        }
    }
}
