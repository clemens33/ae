//! Q1 ruling: existing outbound routing remains; relay adds no egress path.
use super::{Watch, human_facing};
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
