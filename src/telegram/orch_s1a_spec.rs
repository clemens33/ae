#[test]
fn orch_s1a_names_dispatcher_in_telegram_outbound_and_keeps_legacy_display() {
    let temp = Temp::new("orch-s1a-outbound");
    let ask = |id: &str, text: &str, routing: &str| {
        format!(
            r#"{{"ts":"2026-11-19T11:00:00Z","actor":"console:local","action":"ask","target":"lead","ref":"{id}","summary":"{text}"{routing}}}"#
        )
    };
    let origin = r#","relay_origin":"0199c0de-aaaa-4890-abcd-ef0123456789""#;
    append(
        temp.path(),
        &[
            ask("ae-20261119T110000Z-00000001", "ordinary", ""),
            ask("ae-20261119T110000Z-00000002", "legacy relay", origin),
            ask(
                "ae-20261119T110000Z-00000003",
                "attested relay",
                &format!(r#"{origin},"relay_mode":"attested""#),
            ),
            ask(
                "ae-20261119T110000Z-00000004",
                "bound relay",
                &format!(
                    r#"{origin},"relay_mode":"bound","owner_input":"ae-20261119T100000Z-00000001""#
                ),
            ),
            ask(
                "ae-20261119T110000Z-00000005",
                "interim relay",
                &format!(r#"{origin},"relay_mode":"interim""#),
            ),
            ask(
                "ae-20261119T110000Z-00000006",
                "unknown mode",
                &format!(r#"{origin},"relay_mode":"unknown""#),
            ),
            ask(
                "ae-20261119T110000Z-00000007",
                "no relay origin",
                r#","relay_mode":"attested""#,
            ),
        ],
    );
    let fake = Fake::one(Reply::ok());
    let mut outbound = Outbound::new(temp.path(), "recipient");
    let pass = outbound.pump(&fake.api());
    assert_eq!(pass.failure, None);
    assert_eq!(pass.delivered, 7);
    assert_eq!(
        texts(&fake),
        [
            "[recipient] human:chat\nordinary",
            "[recipient] human:chat\nlegacy relay",
            "[recipient] ae/orchestrator\nattested relay",
            "[recipient] ae/orchestrator\nbound relay",
            "[recipient] ae/orchestrator\ninterim relay",
            "[recipient] ae/orchestrator\nunknown mode",
            "[recipient] human:chat\nno relay origin",
        ]
    );
}
