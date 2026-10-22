// Frozen unreach addendum a2. Include at EOF of src/autotrust.rs under cfg(test).
#[cfg(test)]
mod unreach_clock_a2 {
    use super::*;
    use crate::json::Value;

    #[test]
    fn unreach_non_panel_naming_with_current_start_cannot_age_a_panel() {
        let start = "main:start-one:7";
        let event = Event::parse_line(
            &Value::Obj(vec![
                (
                    "ts".into(),
                    Value::str(Timestamp::from_epoch(900).to_string()),
                ),
                ("actor".into(), Value::str("watchdog")),
                ("action".into(), Value::str("human-prompt")),
                ("target".into(), Value::str("lead")),
                ("target_slot".into(), Value::str("main")),
                ("target_session".into(), Value::str("s")),
                ("ref".into(), Value::str(start)),
                (
                    "summary".into(),
                    Value::str("Quick safety check: — press: Enter to confirm"),
                ),
            ])
            .render(),
        )
        .expect("synthetic non-panel naming");
        assert_eq!(
            dismiss_due_at(&[event], "s", "main", "lead", start, 1_000, None),
            Due::No,
            "the same tool start is necessary but only a panel naming supplies first sight"
        );
    }
}
