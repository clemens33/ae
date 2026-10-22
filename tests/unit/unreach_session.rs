// Frozen unreach r1. Included at EOF of src/session.rs, under cfg(test).
#[cfg(test)]
mod unreach_fold_spec {
    use super::*;
    use crate::json::Value;

    const WORDS: &str = "nudge unreachable/occupied — 3 undelivered attempts (idle 5m)";

    fn event(at: i64, actor: &str, action: &str, summary: &str) -> Event {
        let mut fields = vec![
            (
                "ts".into(),
                Value::str(Timestamp::from_epoch(at).to_string()),
            ),
            ("actor".into(), Value::str(actor)),
            ("action".into(), Value::str(action)),
            ("summary".into(), Value::str(summary)),
        ];
        if actor == "lead" {
            fields.extend([
                ("actor_slot".into(), Value::str("main")),
                ("actor_session".into(), Value::str("s")),
            ]);
        } else {
            fields.extend([
                ("target".into(), Value::str("lead")),
                ("target_slot".into(), Value::str("main")),
                ("target_session".into(), Value::str("s")),
            ]);
        }
        Event::parse_line(&Value::Obj(fields).render()).expect("synthetic failure fold")
    }

    #[test]
    fn unreach_one_durable_record_survives_restart_until_same_seat_recovery() {
        let mut events = vec![event(900, "watchdog", "alert", WORDS)];
        for at in 901..906 {
            events.push(event(at, "watchdog", "watchdog-start", "restarted"));
            assert!(unreachable_standing(&events, "s", "main", "lead", None));
            assert_eq!(
                alert_reason_in(&events, "s", "main", "lead"),
                Some(Reason::Stale)
            );
        }
        assert!(
            !unreachable_standing(&events, "s", "main", "lead", Some(901)),
            "new tool launch clears old alert"
        );
        let mut foreign = event(906, "lead", "state", "working");
        foreign.actor_slot = crate::events::RoutingMember::Value("worker".into());
        events.push(foreign);
        assert!(
            unreachable_standing(&events, "s", "main", "lead", None),
            "another seat cannot recover this one"
        );
        let mut own = events.clone();
        own.push(event(907, "lead", "state", "working"));
        assert!(!unreachable_standing(&own, "s", "main", "lead", None));
        events.push(event(907, "watchdog", "alert-cleared", "reachable again"));
        assert!(!unreachable_standing(&events, "s", "main", "lead", None));
    }

    #[test]
    fn unreach_classifier_does_not_relabel_stale_or_other_seats() {
        for summary in [
            "max nudges reached (idle 30m), needs attention",
            "not sweeping — stale overview",
            "dead pane",
        ] {
            assert!(
                !unreachable_standing(
                    &[event(900, "watchdog", "alert", summary)],
                    "s",
                    "main",
                    "lead",
                    None
                ),
                "{summary}"
            );
        }
        let event = event(900, "watchdog", "alert", WORDS);
        assert!(!unreachable_standing(
            std::slice::from_ref(&event),
            "other",
            "main",
            "lead",
            None
        ));
        assert!(!unreachable_standing(&[event], "s", "worker", "lead", None));
    }
}
