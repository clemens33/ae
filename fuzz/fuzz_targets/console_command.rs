#![no_main]

use ae::console::input::{self, Effect, Input, Reading};
use libfuzzer_sys::fuzz_target;
use std::time::Instant;

/// Every effect one owning console takes from `stream`, read `chunk` bytes at
/// a time, all at one stamp.
fn effects(stream: &[u8], chunk: usize, at: Instant) -> (Vec<Effect>, Option<String>) {
    let mut input = Input::new(vec!["lead".to_owned(), "colead".to_owned()]);
    let _ = input.tick(Reading::Owner, at);
    let taken = stream.chunks(chunk).flat_map(|read| input.chunk(read, at));
    (taken.collect(), input.line())
}

fuzz_target!(|data: &[u8]| {
    // The first byte sizes the reads, 1..=256; the rest is what the terminal
    // sent. A read boundary must never change a key, and nothing the console
    // draws or prints may carry a byte the terminal would act on.
    let (chunk, stream) = match data.split_first() {
        Some((size, rest)) => (usize::from(*size) % 256 + 1, rest),
        None => (1, data),
    };
    let at = Instant::now();
    let (whole, line) = effects(stream, stream.len().max(1), at);
    assert_eq!(effects(stream, chunk, at), (whole.clone(), line.clone()));
    let pair = ["lead".to_owned(), "colead".to_owned()];
    for effect in &whole {
        match effect {
            Effect::Ask { raw, seat, body } => {
                assert!(raw.len() <= input::CAP && pair.contains(seat));
                let again = input::command(raw, &pair);
                let want = input::Command::Ask {
                    seat: seat.clone(),
                    body: body.clone(),
                };
                assert_eq!(again, want);
            }
            Effect::Print(said) => assert!(said.starts_with("refused: ")),
            Effect::Close(_) | Effect::Paste(_) => {}
        }
    }
    let drawn = input::paint("", &whole, line.as_deref());
    let known = [input::DRAW_OPEN, input::DRAW_CLOSE, "\r\x1b[K"];
    let bare = known.iter().fold(drawn, |text, seq| text.replace(seq, ""));
    assert!(
        !bare.chars().any(|ch| ch.is_control() && ch != '\n'),
        "{bare:?}"
    );
    if std::str::from_utf8(stream).is_err() {
        let refused = input::command(stream, &pair);
        assert!(matches!(refused, input::Command::Refused(_)));
    }
});
