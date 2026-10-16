#![no_main]

use ae::json::Value;
use libfuzzer_sys::fuzz_target;

// A Codex rollout line is vendor-written state that also carries every tool
// output the conversation ever printed, and a tool output can quote another
// seat's launch token. The proof that a rollout belongs to a launch must hold
// for a response_item developer message and nothing else: never panic, and
// whenever it says yes the real JSON parser must see exactly that shape with
// the marker inside a content item's text.
const MARKER: &str = "AE_CODEX_LAUNCH_ID=fuzz-token";

fuzz_target!(|data: &[u8]| {
    let Ok(line) = std::str::from_utf8(data) else {
        return;
    };
    let proved = std::hint::black_box(ae::session_launch::developer_record_carries(line, MARKER));
    // An empty marker is a caller bug and must never prove anything.
    assert!(!ae::session_launch::developer_record_carries(line, ""));
    if !proved {
        return;
    }
    let value = ae::json::parse(line).expect("a proven line is one JSON value");
    assert_eq!(value.get_str("type"), Some("response_item"));
    let payload = value.get("payload").expect("a proven line has a payload");
    assert_eq!(payload.get_str("type"), Some("message"));
    assert_eq!(payload.get_str("role"), Some("developer"));
    let Some(Value::Arr(content)) = payload.get("content") else {
        panic!("a proven line has a content list");
    };
    assert!(
        content.iter().any(|item| item
            .get_str("text")
            .is_some_and(|text| text.contains(MARKER))),
        "the marker sits in a content item's text"
    );
});
