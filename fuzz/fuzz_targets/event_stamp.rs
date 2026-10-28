#![no_main]

use ae::json::Value;
use libfuzzer_sys::fuzz_target;

// The journal's one stamp: `store::stamp_event_records` edits the bytes of a
// body about to be appended, and may be handed anything a hand-edited or
// copied record could hold. The oracle is built from the ORIGINAL body: an
// LF-terminated record that is one JSON object without `schema_version` must
// gain exactly `"schema_version":1` before its closing brace (with a comma
// unless it was empty) and nothing else; every other record, and an
// unterminated tail, must come out byte for byte. Stamping twice stamps once.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let once = ae::store::stamp_event_records(&text);
    assert_eq!(ae::store::stamp_event_records(&once), once);
    let mut expected = String::with_capacity(once.len());
    for record in text.split_inclusive('\n') {
        let eligible = match ae::json::parse(record) {
            Ok(Value::Obj(fields))
                if record.ends_with('\n') && fields.iter().all(|(k, _)| k != "schema_version") =>
            {
                Some(fields.is_empty())
            }
            _ => None,
        };
        let Some(empty) = eligible else {
            expected.push_str(record);
            continue;
        };
        let Some(close) = record.rfind('}') else {
            panic!("an object record ends in a closing brace");
        };
        let comma = if empty { "" } else { "," };
        let stamped = format!(
            "{}{comma}\"schema_version\":1{}",
            &record[..close],
            &record[close..]
        );
        let Ok(value) = ae::json::parse(&stamped) else {
            panic!("a stamped record must still parse");
        };
        assert_eq!(
            ae::events::schema_version(&value),
            ae::events::SchemaVersion::V(1)
        );
        expected.push_str(&stamped);
    }
    assert_eq!(once, expected.as_str());
});
