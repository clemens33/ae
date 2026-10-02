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

/// Where each unit of `bytes` ends, 0 first: a character, or one byte that is
/// not UTF-8.
fn unit_ends(bytes: &[u8]) -> Vec<usize> {
    let (mut ends, mut from) = (vec![0], 0);
    while from < bytes.len() {
        let (upto, bad) = match std::str::from_utf8(&bytes[from..]) {
            Ok(_) => (bytes.len(), false),
            Err(why) => (from + why.valid_up_to(), true),
        };
        let valid = std::str::from_utf8(&bytes[from..upto]).unwrap_or_default();
        ends.extend(
            valid
                .char_indices()
                .map(|(at, ch)| from + at + ch.len_utf8()),
        );
        from = upto + usize::from(bad);
        if bad {
            ends.push(from);
        }
    }
    ends
}

/// The lines Enter enters when an owning console that restored `kept` is sent
/// `stream`, written out here from the key table and the editing rules without
/// `Keys` or `Composer`: text goes in at the cursor, which the arrows, Home,
/// End, Delete, Backspace and ^U move or erase; a paste is literal to its end.
fn entered(kept: &[u8], stream: &[u8]) -> Vec<Vec<u8>> {
    const CLOSE: &[u8] = b"\x1b[201~";
    let (mut draft, mut cursor) = (kept.to_vec(), kept.len());
    let (mut lines, mut pasting, mut at) = (Vec::new(), false, 0);
    while at < stream.len() {
        let rest = &stream[at..];
        if pasting {
            let close = rest.windows(CLOSE.len()).position(|w| w == CLOSE);
            let held = (1..CLOSE.len())
                .rev()
                .find(|&n| rest.ends_with(&CLOSE[..n]));
            let text = &rest[..close.unwrap_or(rest.len() - held.unwrap_or(0))];
            draft.splice(cursor..cursor, text.iter().copied());
            cursor += text.len();
            let Some(n) = close else { break };
            (pasting, at) = (false, at + n + CLOSE.len());
            continue;
        }
        at += 1;
        let key: &[u8] = if rest[0] == 0x1b {
            let done = |n: usize| match &rest[..n] {
                [_, b'[' | b'O'] => false,
                [_, b'[', .., last] => (0x40..=0x7e).contains(last),
                _ => true,
            };
            let Some(len) = (2..=17.min(rest.len())).find(|&n| n == 17 || done(n)) else {
                break;
            };
            at += len - 1;
            match &rest[..len] {
                b"\x1b[200~" => {
                    pasting = true;
                    continue;
                }
                seq @ (b"\x1b[D" | b"\x1bOD" | b"\x1b[C" | b"\x1bOC" | b"\x1b[3~") => seq,
                b"\x1b[H" | b"\x1bOH" | b"\x1b[1~" | b"\x1b[7~" => b"\x01",
                b"\x1b[F" | b"\x1bOF" | b"\x1b[4~" | b"\x1b[8~" => b"\x05",
                _ => continue,
            }
        } else {
            &rest[..1]
        };
        // Text goes in at the byte offset, whatever it completes; any other key
        // first moves the cursor on to the end of the character it sits inside.
        let at_text = cursor;
        let ends = unit_ends(&draft);
        cursor = ends
            .iter()
            .find(|&&end| end >= cursor)
            .copied()
            .unwrap_or(cursor);
        let before = ends.iter().rev().find(|&&end| end < cursor).copied();
        let after = ends.iter().find(|&&end| end > cursor).copied();
        match key {
            b"\x1b[D" | b"\x1bOD" => cursor = before.unwrap_or(cursor),
            b"\x1b[C" | b"\x1bOC" => cursor = after.unwrap_or(cursor),
            b"\x1b[3~" => {
                draft.drain(cursor..after.unwrap_or(cursor));
            }
            b"\x7f" | b"\x08" => {
                let from = before.unwrap_or(cursor);
                draft.drain(from..cursor);
                cursor = from;
            }
            b"\x01" => cursor = 0,
            b"\x05" => cursor = draft.len(),
            b"\x15" => (draft, cursor) = (Vec::new(), 0),
            b"\r" | b"\n" if !draft.is_empty() => {
                lines.push(std::mem::take(&mut draft));
                cursor = 0;
            }
            b"\r" | b"\n" => {}
            text => {
                draft.splice(at_text..at_text, text.iter().copied());
                cursor = at_text + text.len();
            }
        }
    }
    lines
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
    // The same stream, closed with a paste end and Enter, enters exactly the lines
    // an independent copy of the key table and editor says, one effect each.
    let full = [stream, b"\x1b[201~\r"].concat();
    if kept.len() + full.len() <= input::CAP {
        let (all, _, _) = effects(kept, &full, chunk, at, size);
        let lines = entered(kept, &full);
        let after = &all[usize::from(!kept.is_empty())..];
        assert_eq!(after.len(), lines.len(), "one effect per entered line");
        for (effect, line) in after.iter().zip(&lines) {
            if let Effect::Ask { raw, .. } = effect {
                assert_eq!(raw, line);
            }
        }
    }
    if std::str::from_utf8(stream).is_err() {
        let refused = input::command(stream, &pair);
        assert!(matches!(refused, input::Command::Refused(_)));
    }
});
