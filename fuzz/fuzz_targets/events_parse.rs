#![no_main]

use libfuzzer_sys::fuzz_target;

// events.jsonl: hand-editable hostile persisted state. `parse_line` is the
// production door — `EventLog::drain` drives it once per line, and the
// session/requests/telegram/usage readers call it directly. `from_json` has
// no other production caller, so the same target drives it with a Value the
// real JSON parser produced from the same bytes — never a hand-built tree.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = std::hint::black_box(ae::events::Event::parse_line(&text));
    if let Ok(value) = ae::json::parse(&text) {
        let _ = std::hint::black_box(ae::events::Event::from_json(&value));
    }
});
