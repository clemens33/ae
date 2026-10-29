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
        // `schema_version` is descriptive metadata: reading it is total, and
        // the member's presence, absence or value changes no field of the
        // record and no error the record earns.
        let _ = std::hint::black_box(ae::events::schema_version(&value));
        if let ae::json::Value::Obj(fields) = &value {
            let bare = ae::json::Value::Obj(
                fields
                    .iter()
                    .filter(|(key, _)| key != "schema_version")
                    .cloned()
                    .collect(),
            );
            assert_eq!(
                ae::events::Event::from_json(&value),
                ae::events::Event::from_json(&bare)
            );
        }
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
    // `relay --status` folds one claim, then its opening, from these rows;
    // the first row's id and origin drive both.
    let events: Vec<_> = text
        .lines()
        .filter_map(|line| ae::events::Event::parse_line(line).ok())
        .collect();
    if let Some(first) = events.first()
        && let (Some(id), Some(origin)) = (&first.reference, first.actor_session_id.value())
        && let Ok(claim) = ae::relay::claimed(&events, id, origin)
        && let Ok(Some(at)) = ae::relay::opening(&events, id, origin, &claim)
    {
        assert_eq!(events[at].reference.as_ref(), Some(id));
    }
    // `_run`'s conversation choices: a decoded record spells its own `ref`
    // back. The currency fold takes its seat, baseline, live start and
    // completeness from the hostile rows too: any ref's first token, or none.
    for event in &events {
        if let Some(Ok(record)) = ae::conversation::decode(event) {
            let spelled = ae::conversation::reference(&record);
            assert_eq!(event.reference.as_deref(), Some(spelled.as_str()));
        }
    }
    let keys: Vec<&str> = events
        .iter()
        .filter_map(|e| e.reference.as_deref()?.split(' ').next())
        .collect();
    let pick = |n: usize| keys.get(n % (keys.len() + 1)).copied();
    let bits = data.iter().fold(0usize, |h, b| {
        h.wrapping_mul(31).wrapping_add(usize::from(*b))
    });
    let mut seat = pick(bits >> 8).unwrap_or("main").split(':');
    let (slot, launch) = (seat.next().unwrap_or(""), seat.next().unwrap_or(""));
    let launched = [
        None,
        Some(Err(())),
        pick(bits >> 2).map(Ok),
        pick(bits >> 4).map(Ok),
    ][(bits >> 6) % 4];
    let live = pick(bits >> 4);
    let verdict = ae::conversation::current(&events, bits % 2 == 0, slot, launch, launched, live);
    if let ae::conversation::Verdict::Current(record) = verdict {
        assert!(bits % 2 == 0 && Some(record.start_key.as_str()) == live);
    }
});
