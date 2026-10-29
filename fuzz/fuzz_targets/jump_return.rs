#![no_main]

use libfuzzer_sys::fuzz_target;

// The orchestrator jump's return record is a tmux session option: ae writes it,
// but any client on the server can set it to any bytes, and the record's bytes
// are spliced into a tmux format when it is cleared. The parser must refuse
// rather than panic, and its grammar has one spelling per record, so anything
// it accepts must render back to the very text it was given.
fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Some(record) = std::hint::black_box(ae::jump::Record::parse(text)) {
        assert_eq!(
            record.render(),
            text,
            "a parsed record must render back to its text"
        );
    }
});
