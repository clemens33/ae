#![no_main]

use libfuzzer_sys::fuzz_target;

// The `@ae_app_view` option can be set by any client on the server. Lossy
// UTF-8 still reaches the parser, which accepts only text it could have
// written itself.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Some(handover) = ae::app::handover::parse(&text) {
        assert_eq!(handover.render(), text);
        let _ = std::hint::black_box(handover.admits("%7", "2026.10.99", 1_760_259_000));
    }
});
