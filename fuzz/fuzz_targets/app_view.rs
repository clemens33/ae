#![no_main]

use libfuzzer_sys::fuzz_target;

// The `@ae_app_view` option can be set by any client on the server. Lossy
// UTF-8 still reaches the parser, which accepts only text it could have
// written itself, plus the one view token an older core wrote and an empty
// trailing panel width.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Some(handover) = ae::app::handover::parse(&text) {
        let written = handover.render();
        // The older core's `overview` view token reads as `agents`, and an
        // empty fifteenth field, the panel width, reads as the 14-field line
        // without it; every other accepted text renders back to itself, and
        // what was written reads back as the same value.
        let empty_panel =
            text.split('|').count() == 15 && text.strip_suffix('|') == Some(written.as_str());
        let legacy =
            text.starts_with("v1|") && (text.split('|').nth(6) == Some("overview") || empty_panel);
        assert!(written == text || legacy);
        assert_eq!(ae::app::handover::parse(&written), Some(handover.clone()));
        let _ = std::hint::black_box(handover.admits("%7", "2026.10.99", 1_760_259_000));
    }
});
