//! Q1 ruling: existing outbound routing remains; relay adds no egress path.
use super::{Watch, human_facing};
use crate::config::RelayReplies;
use crate::events::Event;
use std::path::Path;

#[test]
fn existing_bridge_forwards_target_chat_and_emitter_echo_but_not_attempts() {
    let watch = Watch {
        pair: vec!["lead".to_owned()],
        since: i64::MIN,
        echoes: std::sync::Arc::default(),
    };
    let cases = [
        ("console:local", "ask", Some("human:chat")),
        ("ae:relay", "chat", Some("ae:relay")),
        ("boss", "relay-attempt", None),
        ("boss", "relay-result", None),
    ];
    for (actor, action, expected) in cases {
        let line = format!(
            r#"{{"ts":"2026-10-25T09:00:00Z","actor":"{actor}","action":"{action}","ref":"relay-id","summary":"owner instruction","relay_origin":"0199c0de-aaaa-4890-abcd-ef0123456789"}}"#
        );
        let event = Event::parse_line(&line).expect("fixture event");
        let routed = human_facing(&event, &watch, Path::new("."));
        assert_eq!(
            routed.as_ref().map(|(who, _)| *who),
            expected,
            "{actor}/{action}"
        );
    }
}

/// What the bridge's own pass makes of one journal line of a session whose reply
/// echoes `replies` sets: the message text, or `None` when the line is not sent.
fn bridged_as(line: &str, replies: RelayReplies) -> Option<String> {
    let mut outbound = super::Outbound::new(Path::new("/nowhere/sessions/orch"), "orch");
    outbound.relay_replies = replies;
    match outbound.wanted(line.as_bytes()) {
        super::Wanted::Send(text) => Some(text),
        super::Wanted::Skip | super::Wanted::Stale => None,
    }
}

/// [`bridged_as`] for an orchestrator whose setting is the default, `off`.
fn bridged(line: &str) -> Option<String> {
    bridged_as(line, RelayReplies::Hidden)
}

fn row(actor: &str, summary: &str) -> String {
    format!(
        r#"{{"ts":"2026-10-25T09:00:00Z","actor":"{actor}","action":"chat","ref":"ae-20261025T090000Z-0000abcd","summary":"{summary}"}}"#
    ) + "\n"
}

#[test]
fn an_orchestrator_bridge_keeps_a_raw_relay_answer_off_the_phone_but_not_the_rest_of_ae() {
    // Production-shaped: the writer is `relay.rs`'s receipt and reply echoes.
    let receipt = row(
        "ae:relay",
        "routed to home: please check it (sent), request ae-20261025T090000Z-0000abcd",
    );
    let reply = row(
        "ae:relay",
        "reply fd5bf0 from lead: Found one: RAW-RELAY-ANSWER",
    );
    let other = row("ae:compact", "reboot: handover complete");
    let watchdog = row("watchdog", "reply fd5bf0 from lead: a watchdog's own words");
    let odd = row(
        "ae:relay",
        "reply not-a-nonce from lead: not an answer echo",
    );

    assert_eq!(bridged(&reply), None, "the raw answer is not forwarded");
    let receipt = bridged(&receipt).expect("the receipt echo still goes");
    assert!(
        receipt.starts_with("[orch] ae:relay\nrouted to home: "),
        "{receipt}"
    );
    assert!(bridged(&other).is_some_and(|text| text.contains("reboot: handover")));
    assert!(bridged(&watchdog).is_some_and(|text| text.contains("a watchdog's own words")));
    assert!(bridged(&odd).is_some_and(|text| text.contains("not an answer echo")));
}

#[test]
fn a_bridge_that_knows_no_role_forwards_a_reply_echo_as_it_always_did() {
    let reply = row(
        "ae:relay",
        "reply fd5bf0 from lead: Found one: RAW-RELAY-ANSWER",
    );
    // The constructor knows no role: only the pass that owns the bridge does.
    let outbound = super::Outbound::new(Path::new("/nowhere/sessions/home"), "home");
    assert_eq!(outbound.relay_replies, RelayReplies::Shown);
    assert!(
        matches!(outbound.wanted(reply.as_bytes()), super::Wanted::Send(text) if text.contains("RAW-RELAY-ANSWER"))
    );
    assert!(
        bridged_as(&reply, RelayReplies::Shown)
            .is_some_and(|text| text.contains("RAW-RELAY-ANSWER")),
        "`on` forwards it"
    );
}
