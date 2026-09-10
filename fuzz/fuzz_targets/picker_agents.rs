#![no_main]

use libfuzzer_sys::fuzz_target;

// The watchdog-owned tmux option is hand-editable persisted state. Lossy UTF-8
// still reaches the parser, whose ASCII contract must reject every replacement.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = std::hint::black_box(ae::tmux::parse_picker_agents(&text, 2_000_000_000, 60));
});
