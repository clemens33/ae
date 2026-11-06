#![no_main]

use libfuzzer_sys::fuzz_target;

// The identity v2 INI parser, over one whole config text. `read_identity` reads
// the file and hands the parser a `str`, so lossy conversion puts every input
// byte in play as the one thing the parser can see. The auto reseat settings
// read the same global text, through their own entry (the lead floor and its
// `[model_tiers]` rows too), and so does the strict effort switch. The
// approved-pair list is hand-edited global text read through
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
        // The lead floor: every written `[model_tiers]` pin, plus pins no row
        // names, read under both pin rules and judged for two tools, so a row
        // text cannot panic the match or the verdict.
        let pins = reseat.tiers.iter().map(|row| row.pin.as_str()).chain([
            "opus55x",
            "claude-opus-5-5",
            "gpt-5.5",
            "",
        ]);
        for pin in pins {
            for rule in [ae::tool::PinMatch::Exact, ae::tool::PinMatch::FamilyVersion] {
                let _ = std::hint::black_box(reseat.tier_of(pin, rule));
            }
            for tool in [ae::tool::ToolKind::Claude, ae::tool::ToolKind::Codex] {
                let model = ae::autoreseat::Model {
                    pin: Some(pin.to_owned()),
                    tool: Some(tool),
                };
                let _ = std::hint::black_box(view.floor_refuses(&model));
            }
        }
        let _ = std::hint::black_box(view.floor_refuses(&ae::autoreseat::Model::default()));
        let _ = std::hint::black_box(view.lead_seat);
    }
    let _ = std::hint::black_box(reseat);
    let _ = std::hint::black_box(ae::config::auto_effort_in(
        std::path::Path::new("config"),
        &text,
    ));
    let pairs = ae::pairs::parse_in(std::path::Path::new("config"), &text);
    let _ = std::hint::black_box(pairs.judge("opus55x", "cc-mic"));
});
