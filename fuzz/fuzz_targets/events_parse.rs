#![no_main]

use libfuzzer_sys::fuzz_target;

// events.jsonl: hand-editable hostile persisted state. `parse_line` is the
// production door — `EventLog::drain` drives it once per line, and the
// session/requests/telegram/usage readers call it directly. `from_json` has
// no other production caller, so the same target drives it with a Value the
// real JSON parser produced from the same bytes — never a hand-built tree.
//
// `wait_on` is lenient: whatever the bytes, a target survives only on a
// `waiting-agent` state record and only when the target grammar admits it.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Ok(event) = std::hint::black_box(ae::events::Event::parse_line(&text))
        && let Some(on) = &event.wait_on
    {
        assert!(event.action == "state" && event.reference.as_deref() == Some("waiting-agent"));
        assert!(ae::wait_on::Target::parse(on, "").is_some());
        let _ = std::hint::black_box(ae::wait_on::Target::parse(on, "mac"));
    }
    if let Ok(value) = ae::json::parse(&text) {
        let _ = std::hint::black_box(ae::events::Event::from_json(&value));
    }
    // The request reader retains strict opening facts from these same
    // hostile records, including optional launch keys and damaged rows.
    let _ = std::hint::black_box(ae::requests::states(data));
    if let Some(line) = text.lines().next()
        && let Ok(event) = ae::events::Event::parse_line(line)
        && let Some(id) = event.reference
    {
        let pair = [data, line.as_bytes()];
        let _ = std::hint::black_box(ae::requests::bound_opening(pair, &id, None));
        let repair = text.lines().last().map(str::as_bytes);
        let _ = std::hint::black_box(ae::requests::bound_opening(pair, &id, repair));
    }
});
