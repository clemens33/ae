#![no_main]

use libfuzzer_sys::fuzz_target;

// A decision card's reason is an agent's `waiting-user` declaration as the
// journal holds it: hostile persisted text a human, a crashed write or another
// tool can leave as any bytes. The chat reads the journal lossily, so the
// layout is driven over `from_utf8_lossy` text, and it must lay out anything
// without a panic.
//
// The second leg is the layout's promise: every line is a slice of the reason,
// in its order, so the words it draws are the reason's words, the standalone
// `|` option separators aside.
fuzz_target!(|data: &[u8]| {
    let reason = String::from_utf8_lossy(data);
    let parts = std::hint::black_box(ae::console::card::layout(&reason));
    let words = |text: &str| -> Vec<String> {
        text.split_whitespace()
            .filter(|word| *word != "|")
            .map(str::to_owned)
            .collect()
    };
    let drawn: Vec<&str> = parts.iter().map(|part| part.text).collect();
    assert_eq!(
        words(&drawn.join(" ")),
        words(&reason),
        "the layout draws the reason's words"
    );
});
