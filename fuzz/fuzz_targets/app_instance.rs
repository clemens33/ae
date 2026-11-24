#![no_main]

use std::time::Duration;

use libfuzzer_sys::fuzz_target;

// `app-instance`, `app-takeover` and `app-takeover-ack` are hand-editable
// files under the state root. A crashed write, a human or another tool can
// leave any bytes there. Each parser is bounded and accepts only the one
// spelling `render_*` writes, so a value that parses must render back to the
// bytes it was given, and no input may panic.
fuzz_target!(|data: &[u8]| {
    if let Some(parsed) = ae::app::instance::parse_instance(data) {
        let rendered = ae::app::instance::render_instance(&parsed);
        assert_eq!(
            rendered.as_deref(),
            Some(data),
            "a parsed instance must render back to its bytes"
        );
        let _ = std::hint::black_box(ae::app::instance::judge(
            &ae::app::instance::Record::Parsed(parsed),
            ae::app::instance::Hold::Held,
            ae::app::instance::Liveness::Unknown,
            &ae::app::instance::Caller {
                in_tmux: true,
                ae_owned: true,
                server: ae::app::instance::Server::Name("ae".to_owned()),
            },
        ));
    }
    if let Some(parsed) = ae::app::instance::parse_request(data) {
        let rendered = ae::app::instance::render_request(&parsed);
        assert_eq!(rendered.as_deref(), Some(data));
        let _ = std::hint::black_box(ae::app::instance::targets_me(
            &parsed,
            parsed.target_pid,
            &parsed.target_identity,
        ));
    }
    if let Some(parsed) = ae::app::instance::parse_ack(data) {
        let rendered = ae::app::instance::render_ack(&parsed);
        assert_eq!(rendered.as_deref(), Some(data));
        let _ = std::hint::black_box(ae::app::instance::phase(
            Some(&parsed),
            &parsed.nonce,
            Duration::from_secs(5),
        ));
    }
});
