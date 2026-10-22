// Frozen unreach addendum a5. Include at EOF of honestwords_spec.rs.
#[cfg(test)]
mod unreach_interrupt_a5 {
    use super::*;
    use ae::attention::Reason;

    #[test]
    fn unreach_confirmed_message_interrupt_ends_failure_but_bare_keys_do_not() {
        let targeted = |action: &str, summary: &str, body_file: &str, age: i64| {
            Event::parse_line(
                &Value::Obj(vec![
                    (
                        "ts".into(),
                        Value::str(Timestamp::from_epoch(NOW - age).to_string()),
                    ),
                    (
                        "actor".into(),
                        Value::str(if action == "alert" {
                            "watchdog"
                        } else {
                            "worker"
                        }),
                    ),
                    ("action".into(), Value::str(action)),
                    ("target".into(), Value::str("lead")),
                    ("target_slot".into(), Value::str("main")),
                    ("target_session".into(), Value::str("one")),
                    ("target_session_id".into(), Value::str(UUID)),
                    ("summary".into(), Value::str(summary)),
                    ("body_file".into(), Value::str(body_file)),
                ])
                .render(),
            )
            .expect("synthetic routed helper record")
        };
        for (action, summary, body_file, recovered) in [
            (
                "interrupt",
                "please finish the review",
                "/messages/confirmed-interrupt.txt",
                true,
            ),
            ("interrupt", "", "", false),
            (
                "interrupt",
                "[unconfirmed] please finish the review",
                "/messages/unconfirmed-interrupt.txt",
                false,
            ),
            (
                "delivery-failed",
                "UNCONFIRMED notice; interrupt submit proof failed",
                "",
                false,
            ),
        ] {
            let events = vec![
                declaration("done", "review complete", 720, "lead"),
                targeted(
                    "alert",
                    "nudge unreachable/occupied — 3 undelivered attempts (idle 5m)",
                    "",
                    600,
                ),
                targeted(action, summary, body_file, 1),
            ];
            let snapshot = entry(Some(events));
            let expected = (!recovered).then_some(Reason::Stale);
            assert_eq!(snapshot.attention, expected, "{action}: {summary}");
            assert_eq!(snapshot.agents[0].reason, expected, "{action}: {summary}");
            let rendered = ae::listing::table_at(&[&snapshot], now());
            assert_eq!(
                rendered.contains("unreachable"),
                !recovered,
                "{action}: {summary}: {rendered}"
            );
        }
    }
}
