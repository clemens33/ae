// Frozen unreach r1. Append verbatim to src/autotrust.rs.
#[cfg(test)]
mod unreach_clock_spec {
    use super::*;
    use crate::json::Value;

    const HINT: &str = "k keep & next · esc dismiss & close · ⌃o copy · ←/→ warning · ↓ scroll";
    const START: &str = "main:start-one:7";

    fn event(at: i64, action: &str, summary: &str, reference: &str) -> Event {
        Event::parse_line(
            &Value::Obj(vec![
                (
                    "ts".into(),
                    Value::str(Timestamp::from_epoch(at).to_string()),
                ),
                ("actor".into(), Value::str("watchdog")),
                ("action".into(), Value::str(action)),
                ("target".into(), Value::str("lead")),
                ("target_slot".into(), Value::str("main")),
                ("target_session".into(), Value::str("s")),
                ("summary".into(), Value::str(summary)),
                ("ref".into(), Value::str(reference)),
            ])
            .render(),
        )
        .expect("synthetic panel journal")
    }

    fn panel(at: i64) -> Event {
        event(
            at,
            "human-prompt",
            &format!("Warnings · 1 of 1 · Startup — press: {HINT}"),
            START,
        )
    }

    fn due(events: &[Event], now: i64, input: Option<i64>) -> Due {
        dismiss_due_at(events, "s", "main", "lead", START, now, input)
    }

    #[test]
    fn unreach_both_quiet_gates_use_sixty_seconds_and_fail_closed_on_clock_skew() {
        assert_eq!(DISMISS_QUIET_SECS, 60, "lead a16ac237 contract pin");
        let events = vec![panel(900)];
        for (now, input, expected) in [
            (899, None, Due::No),
            (900, None, Due::No),
            (959, None, Due::No),
            (960, None, Due::Yes),
            (1_000, Some(941), Due::No),
            (1_000, Some(940), Due::Yes),
            (1_000, Some(1_001), Due::No),
            (1_000, Some(1_000), Due::No),
            (959, Some(800), Due::No),
        ] {
            assert_eq!(
                due(&events, now, input),
                expected,
                "now={now}, input={input:?}"
            );
        }
        assert_eq!(due(&[], 1_000, None), Due::No, "latch must be bound");
    }

    #[test]
    fn unreach_oldest_panel_sighting_survives_rebooking_and_a_new_daemon() {
        let events = vec![panel(900), panel(990)];
        assert_eq!(
            due(&events, 960, None),
            Due::Yes,
            "oldest durable sighting owns age"
        );
        // No PaneState participates: a new daemon computes the same answer.
        let reread = vec![panel(900), panel(990)];
        assert_eq!(due(&reread, 960, None), Due::Yes);
    }

    #[test]
    fn unreach_only_this_panel_run_can_supply_first_sight() {
        for reset in [
            event(990, "human-prompt-cleared", "gone", ""),
            event(
                990,
                "human-prompt",
                "Quick safety check: — press: Enter to confirm",
                "",
            ),
            event(990, "alert", "dead pane", ""),
        ] {
            let mut events = vec![panel(900), reset];
            assert_eq!(
                due(&events, 1_000, None),
                Due::No,
                "a closed run is not owed"
            );
            events.push(panel(995));
            assert_eq!(
                due(&events, 1_000, None),
                Due::No,
                "new panel gets a full quiet age"
            );
            assert_eq!(due(&events, 1_055, None), Due::Yes);
        }
        for reference in [Some("main:older-start:7"), None] {
            let mut other = panel(990);
            other.reference = reference.map(str::to_owned);
            let events = vec![panel(900), other, panel(995)];
            assert_eq!(
                due(&events, 1_000, None),
                Due::No,
                "foreign/unproven start resets old first sight"
            );
            assert_eq!(due(&events, 1_055, None), Due::Yes);
        }
        let mut old_start = panel(900);
        old_start.reference = Some("main:older-start:7".into());
        assert_eq!(
            due(&[old_start], 1_000, None),
            Due::No,
            "older tool start cannot age a panel"
        );
        let mut unproven = panel(900);
        unproven.reference = None;
        assert_eq!(
            due(&[unproven], 1_000, None),
            Due::No,
            "unproven sightings stay named only"
        );
        let mut foreign = panel(900);
        foreign.target_slot = crate::events::RoutingMember::Value("worker".into());
        assert_eq!(
            due(&[foreign], 1_000, None),
            Due::No,
            "another seat cannot arm it"
        );
        let mut foreign = panel(900);
        foreign.target_session = crate::events::RoutingMember::Value("other".into());
        assert_eq!(
            due(&[foreign], 1_000, None),
            Due::No,
            "another session cannot arm it"
        );
    }

    #[test]
    fn unreach_reservation_spends_one_panel_episode_even_when_it_failed() {
        let mut events = vec![panel(900)];
        events.push(event(
            961,
            "auto-trust-reserved",
            "trust spent earlier",
            START,
        ));
        assert_eq!(
            due(&events, 1_000, None),
            Due::Yes,
            "trust and panel budgets are independent"
        );
        events.push(event(
            962,
            "auto-dismiss-reserved",
            "reserved",
            "main:start-one:7@900",
        ));
        events.push(event(
            963,
            "auto-dismiss-failed",
            "key refused",
            "main:start-one:7@900",
        ));
        assert_eq!(
            due(&events, 1_000, None),
            Due::No,
            "reservation alone spends the panel"
        );
        assert_eq!(
            dismiss_due_at(
                &events,
                "s",
                "main",
                "lead",
                "main:start-one:8",
                1_000,
                None
            ),
            Due::No,
            "new pid needs its own first sight before any answer"
        );
        let mut new_pid = panel(1_001);
        new_pid.reference = Some("main:start-one:8".into());
        let new_run = vec![panel(900), new_pid];
        assert_eq!(
            dismiss_due_at(
                &new_run,
                "s",
                "main",
                "lead",
                "main:start-one:8",
                1_060,
                None
            ),
            Due::No
        );
        assert_eq!(
            dismiss_due_at(
                &new_run,
                "s",
                "main",
                "lead",
                "main:start-one:8",
                1_061,
                None
            ),
            Due::Yes
        );
        events.push(event(1_001, "human-prompt-cleared", "gone", ""));
        events.push(panel(1_002));
        assert_eq!(due(&events, 1_061, None), Due::No);
        assert_eq!(
            due(&events, 1_062, None),
            Due::Yes,
            "clear then reopen is a new panel episode"
        );
    }
}
