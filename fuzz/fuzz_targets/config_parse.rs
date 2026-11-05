#![no_main]

use libfuzzer_sys::fuzz_target;

// The identity v2 INI parser, over one whole config text. `read_identity` reads
// the file and hands the parser a `str`, so lossy conversion puts every input
// byte in play as the one thing the parser can see. The auto reseat settings
// read the same global text, through their own entry, and so does the strict
// effort switch. The approved-pair list is hand-edited global text read through
// `section_entries`, so its pure entry takes the same bytes.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = std::hint::black_box(ae::config::parse_identity(&text));
    let reseat = ae::autoreseat::settings_in(std::path::Path::new("config"), &text);
    for (lead_pair, slot) in [
        (true, "main"),
        (true, "worker.0"),
        (false, "main"),
        (true, "worker.1"),
    ] {
        let view = reseat.seat_view(lead_pair, slot);
        let _ = std::hint::black_box((view.headroom_at, view.headroom_key()));
    }
    let _ = std::hint::black_box(reseat);
    let _ = std::hint::black_box(ae::config::auto_effort_in(
        std::path::Path::new("config"),
        &text,
    ));
    let pairs = ae::pairs::parse_in(std::path::Path::new("config"), &text);
    let _ = std::hint::black_box(pairs.judge("opus55x", "cc-mic"));
});
