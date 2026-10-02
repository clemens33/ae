#![no_main]

use ae::console::input::{self, Effect, Input, Reading, Screen, Size, View};
use ae::console::submit::Draft;
use libfuzzer_sys::fuzz_target;
use std::time::Instant;

type Taken = (Vec<Effect>, Option<String>, Option<View>);

/// Every effect one owning console takes from `stream`, read `chunk` bytes at
/// a time, all at one stamp, after `kept` was restored as its draft; then its
/// composer on one line and laid out for `size`.
fn effects(kept: &[u8], stream: &[u8], chunk: usize, at: Instant, size: Size) -> Taken {
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
    let (line, view) = (input.line(), input.view(size));
    (taken, line, view)
}

/// Panics unless `out` is printable text, line breaks and the few sequences
/// the composer draws with: autowrap and bracketed paste switches, relative
/// moves up or down, clear, and the cursor save and restore.
fn assert_drawn(out: &str) {
    let mut rest = out;
    while let Some(ch) = rest.chars().next() {
        let after = &rest[ch.len_utf8()..];
        rest = if ch != '\x1b' {
            assert!(!ch.is_control() || ch == '\n' || ch == '\r', "{out:?}");
            after
        } else if let Some(tail) = after.strip_prefix(['7', '8']) {
            tail
        } else {
            let body = after.strip_prefix('[').expect("an escape draws");
            let fixed = ["?7l", "?7h", "?2004h", "?2004l", "J", "K"];
            if let Some(tail) = fixed.iter().find_map(|seq| body.strip_prefix(seq)) {
                tail
            } else {
                let digits = body.bytes().take_while(u8::is_ascii_digit).count();
                assert!(digits > 0, "{out:?}");
                body[digits..].strip_prefix(['A', 'B']).expect("a move")
            }
        };
    }
}

/// Panics unless `view` fits `size`: within the row cap, no row wider than the
/// pane (a non-ASCII character counted two cells), none with a control, and
/// the cursor on a row that begins with the text before it.
fn assert_view(view: &View, size: Size) {
    let cap = input::ROWS_MAX.min(size.height.saturating_sub(1).max(1));
    assert!(view.rows.len() <= cap && view.cursor_row < view.rows.len());
    for row in &view.rows {
        let cells: usize = row
            .chars()
            .map(|ch| if ch.is_ascii() { 1 } else { 2 })
            .sum();
        assert!(
            cells <= size.width && !row.chars().any(char::is_control),
            "{row:?}"
        );
    }
    assert!(view.rows[view.cursor_row].starts_with(&view.before));
}

fuzz_target!(|data: &[u8]| {
    // The first byte sizes the reads, 1..=256; the second sizes the kept draft
    // restored before any key, taken from the front of the rest; the third
    // sizes the pane, 1..=120 wide and 1..=24 high; what is left is what the
    // terminal sent. A read boundary must never change a key or the layout, a
    // restored draft only prints, no row is wider than its pane, and nothing
    // the console draws or prints may carry a byte the terminal would act on.
    let (chunk, rest) = match data.split_first() {
        Some((size, rest)) => (usize::from(*size) % 256 + 1, rest),
        None => (1, data),
    };
    let (kept_len, rest) = rest.split_first().map_or((0, rest), |(n, rest)| (*n, rest));
    let (shape, rest) = rest
        .split_first()
        .map_or((79, rest), |(n, rest)| (*n, rest));
    let size = Size {
        width: usize::from(shape) % 120 + 1,
        height: usize::from(shape) / 5 % 24 + 1,
    };
    let (kept, stream) = rest.split_at(usize::from(kept_len).min(rest.len()));
    let at = Instant::now();
    let (whole, line, view) = effects(kept, stream, stream.len().max(1), at, size);
    assert_eq!(
        effects(kept, stream, chunk, at, size),
        (whole.clone(), line.clone(), view.clone())
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
    let line = line.expect("an owner has a composer line");
    assert!(!line.chars().any(|ch| ch.is_control()), "{line:?}");
    let view = view.expect("an owner has a composer");
    assert_view(&view, size);
    let mut screen = Screen::default();
    let first = input::paint(&mut screen, "", &whole, Some(&view), size, None);
    let settled = screen.settle();
    assert!(
        !settled.is_empty(),
        "a drawn composer settles with a line break"
    );
    let drawn = input::paint(&mut screen, "", &[], Some(&view), size, None);
    let again = input::paint(&mut screen, "lane\n", &[], Some(&view), size, None);
    let gone = input::paint(&mut screen, "", &[], None, size, None);
    for text in [&first, &settled, &drawn, &again, &gone, &screen.settle()] {
        assert_drawn(text);
    }
    if std::str::from_utf8(stream).is_err() {
        let refused = input::command(stream, &pair);
        assert!(matches!(refused, input::Command::Refused(_)));
    }
});
