#![no_main]

use libfuzzer_sys::fuzz_target;

// The vendor block record is hand-editable persisted state on every spawn,
// launch and `ae quota` path: a human, a crashed write or another tool can leave
// any bytes at `<state root>/vendorblock/<client>.rec`, and a record that parses
// refuses the next seat on that client scope until `ae quota clear`. The parse
// is bounded (4 KiB) and strict, so oversize and malformed input must be
// refused, never read partway, and no input may panic.
//
// The second leg is the WRITER's contract: anything the parser accepts must
// render back to bytes the parser accepts identically, and every surface that
// words the record (cell, quota line, refusal) must hold for it. A record that
// parsed once and then reads differently would be a block that changes between
// restarts.
fuzz_target!(|data: &[u8]| {
    let parsed = std::hint::black_box(ae::vendorblock::parse(data));
    if let Ok(record) = parsed {
        let rendered = ae::vendorblock::render(&record);
        let again = ae::vendorblock::parse(rendered.as_bytes());
        assert_eq!(
            Ok(record.clone()),
            again,
            "a parsed record must render back to itself"
        );
        assert!(
            record.file_name().is_some(),
            "a parsed scope names one record file"
        );
        assert!(
            record
                .provider
                .as_deref()
                .is_none_or(ae::vendorblock::is_provider),
            "a parsed provider names one upstream provider"
        );
        let refusal = record.refusal();
        assert!(
            refusal.contains(&record.clear_command()),
            "a refusal names its clear command"
        );
        assert!(record.quota_line().contains(&record.cell()));
        assert!(
            !refusal.chars().any(char::is_control),
            "a refusal is one clean line"
        );
        assert!(
            ae::vendorblock::Target::parse(&record.stem()).is_some(),
            "the word a refusal names is a word the clear command takes"
        );
    }
});
