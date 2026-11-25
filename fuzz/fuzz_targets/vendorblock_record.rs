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
//
// The third leg is the journal ref grammar (quarantine2): a watchdog notice
// receipt or drawn-block event names `<identity>` or `<identity>#<slot>` in
// the session journal, hostile persisted text that `notice_target` cuts into
// the identity and the record stem before any record is read. A ref it takes
// starts with its identity, the identity holds no `#`, and the stem is a word
// `ae quota clear` takes; a parsed record's own identity, with or without a
// slot, names that record's stem.
fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        if let Some((identity, target)) = ae::vendorblock::notice_target(text) {
            assert!(text.starts_with(identity), "the identity is the ref's head");
            assert!(!identity.contains('#'), "the identity stops at the first #");
            let stem = identity.split_once('@').map(|(stem, _)| stem);
            assert_eq!(
                stem.and_then(ae::vendorblock::Target::parse),
                Some(target),
                "the stem before the first @ names the target"
            );
        }
    }
    let parsed = std::hint::black_box(ae::vendorblock::parse(data));
    if let Ok(record) = parsed {
        let identity = record.identity();
        let stem = record.stem();
        for reference in [identity.clone(), format!("{identity}#main")] {
            assert_eq!(
                ae::vendorblock::notice_target(&reference),
                Some((
                    identity.as_str(),
                    ae::vendorblock::Target::parse(&stem).expect("stem")
                )),
                "a record's own identity names its stem"
            );
        }
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
