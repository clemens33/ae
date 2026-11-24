#![no_main]

use libfuzzer_sys::fuzz_target;

// The orchestrator seat's queued reseat request is hostile persisted state on
// the WATCHDOG's path: a human, a crashed write or another tool can leave any
// bytes at `reseat-request`, and the watchdog reads it every cycle to decide
// whether to stop the orchestrator's tool and move it. The parse is capped and
// clock-free, so oversize input must be refused rather than parsed, and no
// input may panic.
//
// The second leg is the WRITER's contract: the grammar has one spelling per
// request, so bytes that parse must be exactly the bytes `render` writes for
// what they parsed to, and that rendering must parse back to the same request.
fuzz_target!(|data: &[u8]| {
    let parsed = std::hint::black_box(ae::reseat::request::parse(data));
    if let Ok(request) = parsed {
        assert!(u64::try_from(data.len()).is_ok_and(|len| len <= ae::reseat::request::CAP));
        let rendered = ae::reseat::request::render(&request);
        assert_eq!(rendered.as_bytes(), data, "a request has one spelling");
        assert_eq!(
            ae::reseat::request::parse(rendered.as_bytes()),
            Ok(request),
            "a rendered request must parse back to itself"
        );
    }
});
