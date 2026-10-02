#![no_main]

use ae::console::input::{self, Effect, Input, Reading};
use ae::console::submit::Draft;
use libfuzzer_sys::fuzz_target;
use std::time::Instant;

/// Every effect one owning console takes from `stream`, read `chunk` bytes at
/// a time, all at one stamp, after `kept` was restored as its draft.
fn effects(kept: &[u8], stream: &[u8], chunk: usize, at: Instant) -> (Vec<Effect>, Option<String>) {
    let mut input = Input::new(vec!["lead".to_owned(), "colead".to_owned()]);
    let _ = input.tick(Reading::Owner, at);
    let draft = if kept.is_empty() {
        Draft::Nothing
    } else {
        Draft::Kept(kept.to_vec())
    };
    let mut taken = input.restore(draft);
    assert!(
        taken
            .iter()
            .all(|effect| matches!(effect, Effect::Print(_))),
        "a restored draft only prints: {taken:?}"
    );
    taken.extend(stream.chunks(chunk).flat_map(|read| input.chunk(read, at)));
    (taken, input.line())
}

fuzz_target!(|data: &[u8]| {
    // The first byte sizes the reads, 1..=256; the second sizes the kept draft
    // restored before any key, taken from the front of the rest; what is left
    // is what the terminal sent. A read boundary must never change a key, a
    // restored draft only prints, and nothing the console draws or prints may
    // carry a byte the terminal would act on.
    let (chunk, rest) = match data.split_first() {
        Some((size, rest)) => (usize::from(*size) % 256 + 1, rest),
        None => (1, data),
    };
    let (kept, stream) = match rest.split_first() {
        Some((size, rest)) => rest.split_at(usize::from(*size).min(rest.len())),
        None => (rest, rest),
    };
    let at = Instant::now();
    let (whole, line) = effects(kept, stream, stream.len().max(1), at);
    assert_eq!(
        effects(kept, stream, chunk, at),
        (whole.clone(), line.clone())
    );
    let pair = ["lead".to_owned(), "colead".to_owned()];
    let mut speaker = "lead".to_owned();
    for effect in &whole {
        match effect {
            Effect::Ask { raw, seat, body } => {
                assert!(raw.len() <= input::CAP && pair.contains(seat));
                // The grammar again, written out here: an `@seat ` line asks
                // that seat and becomes the speaker; any other line asks the
                // speaker, which stays.
                let text = std::str::from_utf8(raw).expect("an ask is UTF-8");
                let want = match text.strip_prefix('@') {
                    Some(routed) => {
                        let cut = routed.find(char::is_whitespace);
                        let (seat, body) = routed.split_at(cut.unwrap_or(routed.len()));
                        let body = body.chars().next().map_or("", |sp| &body[sp.len_utf8()..]);
                        speaker = seat.to_owned();
                        (seat.to_owned(), body.to_owned())
                    }
                    None => {
                        assert!(!text.starts_with('/'), "a bare slash line is never an ask");
                        (speaker.clone(), text.to_owned())
                    }
                };
                assert_eq!((seat.clone(), body.clone()), want);
                assert!(!body.trim().is_empty(), "nothing to ask is never an ask");
            }
            Effect::Print(said) => {
                let restored =
                    said.starts_with("Kept line, maybe already sent: check lead, colead");
                assert!(restored || said.starts_with("refused: "), "{said:?}");
            }
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
