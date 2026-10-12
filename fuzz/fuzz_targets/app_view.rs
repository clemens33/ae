#![no_main]

use libfuzzer_sys::fuzz_target;

// The `@ae_app_view` option can be set by any client on the server. Lossy
// UTF-8 still reaches the parser, which accepts only text it could have
// written itself, plus the one view token an older core wrote.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Some(handover) = ae::app::handover::parse(&text) {
        let written = handover.render();
        // The older core's `overview` view token reads as `agents`; every
        // other accepted text renders back to itself, and what was written
        // reads back as the same value.
        assert!(written == text || text.split('|').nth(6) == Some("overview"));
        assert_eq!(ae::app::handover::parse(&written), Some(handover.clone()));
        let _ = std::hint::black_box(handover.admits("%7", "2026.10.99", 1_760_259_000));
    }
});
