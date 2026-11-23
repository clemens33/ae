#![no_main]

use ae::monitor::State;
use libfuzzer_sys::fuzz_target;

// The watchdog checkpoint `meta-agent-state.json`: hostile persisted state the
// watchdog reads on every orchestrator cycle and the manual `_monitor sweep`
// rewrites. A reader that trusts a document keeps every field it understood,
// so the document it renders from that reading reads back to the same state:
// no field, the desk-stale tick's stamp and per-session positions included,
// is gained, lost or changed by one more write.
fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Some(state) = std::hint::black_box(State::parse(text)) {
        let rendered = state.render();
        assert_eq!(
            State::parse(&rendered).as_ref(),
            Some(&state),
            "a trusted checkpoint renders into one that reads back the same"
        );
        assert_eq!(state.render(), rendered, "rendering is deterministic");
    }
});
