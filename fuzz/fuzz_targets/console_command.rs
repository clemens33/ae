#![no_main]

use ae::console::input::{self, Effect, Input, Reading, Screen, Size, View};
use ae::console::submit::Draft;
use libfuzzer_sys::fuzz_target;
use std::time::Instant;

type Taken = (Vec<Effect>, Option<String>, Option<View>);

/// App read boundaries preserve keys and origins, coalescing literal runs.
fn app_keys(stream: &[u8], chunk: usize, at: Instant) -> Vec<(input::Key, Instant)> {
    let mut decoder = input::Keys::app();
    let mut keys = Vec::new();
    let decoded = stream
        .chunks(chunk)
        .flat_map(|read| decoder.feed(read, at))
        .collect::<Vec<_>>();
    for (key, origin) in decoded
        .into_iter()
        .chain(decoder.idle(at + input::ESC_IDLE))
    {
        if let Some((previous, stamp)) = keys.last_mut() {
            let run = match (previous, &key) {
                (input::Key::Text(old), input::Key::Text(new))
                | (input::Key::Pasted(old), input::Key::Pasted(new))
                    if *stamp == origin =>
                {
                    Some((old, new))
                }
                _ => None,
            };
            if let Some((old, new)) = run {
                old.extend_from_slice(new);
                continue;
            }
        }
        keys.push((key, origin));
    }
    keys
}

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

/// [`effects`] through the app's decoder: the keys it decodes are handed to the
/// input as they come, so a paste arrives as one fragment per read.
fn app_effects(kept: &[u8], stream: &[u8], chunk: usize, at: Instant, size: Size) -> Taken {
    let mut input = Input::new(vec!["lead".to_owned(), "colead".to_owned()]);
    let _ = input.tick(Reading::Owner, at);
    let draft = if kept.is_empty() {
        Draft::Nothing
    } else {
        Draft::Kept(kept.to_vec())
    };
    let mut taken = input.restore(draft);
    let mut decoder = input::Keys::app();
    for read in stream.chunks(chunk) {
        taken.extend(input.keyed(decoder.feed(read, at)));
    }
    taken.extend(input.keyed(decoder.idle(at + input::ESC_IDLE)));
    (taken, input.line(), input.view(size))
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

/// The byte that begins an attachment atom in the model's draft: `0xFF` and
/// then `0x80 + n`, an index into the model's table of wire texts. No outside
/// byte is ever `0xFF` (it enters as U+FFFD), so the pair is never forged.
const ATOM: u8 = 0xFF;

/// Where each unit of `bytes` ends, 0 first: a character, one byte that is
/// not UTF-8, or one whole attachment atom.
fn unit_ends(bytes: &[u8]) -> Vec<usize> {
    let (mut ends, mut from) = (vec![0], 0);
    while from < bytes.len() {
        if bytes[from] == ATOM {
            from += 2;
            ends.push(from);
            continue;
        }
        let stop = bytes[from..]
            .iter()
            .position(|&byte| byte == ATOM)
            .map_or(bytes.len(), |at| from + at);
        let (upto, bad) = match std::str::from_utf8(&bytes[from..stop]) {
            Ok(_) => (stop, false),
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

/// `bytes` from outside the draft: the byte that begins an atom becomes U+FFFD.
fn safe(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    for &byte in bytes {
        if byte == ATOM {
            out.extend_from_slice("\u{fffd}".as_bytes());
        } else {
            out.push(byte);
        }
    }
    out
}

/// The length of the attachment token `text` begins with, written out from
/// the grammar: `[Image #N: "<path>"]` with N 1..=999999 and no leading zero,
/// or `[File: "<path>"]`, the path absolute, at most 4096 bytes, with no `"`
/// and no control character.
fn token_len(text: &str) -> Option<usize> {
    let rest = text.strip_prefix('[')?;
    let body = if let Some(number) = rest.strip_prefix("Image #") {
        let digits = number.bytes().take_while(u8::is_ascii_digit).count();
        if !(1..=6).contains(&digits) || number.starts_with('0') {
            return None;
        }
        number[digits..].strip_prefix(": \"")?
    } else {
        rest.strip_prefix("File: \"")?
    };
    let end = body.find('"')?;
    let path = &body[..end];
    let after = body[end + 1..].strip_prefix(']')?;
    let fits = path.starts_with('/') && path.len() <= 4096 && !path.chars().any(char::is_control);
    fits.then(|| text.len() - after.len())
}

/// A draft's bytes with each well-formed token in `line` turned into one atom,
/// its wire text pushed on `table`; past 32 held it stays text. `line` is
/// outside text, so the atom byte is made safe first.
fn atomize(line: &[u8], table: &mut Vec<Vec<u8>>) -> Vec<u8> {
    let line = safe(line);
    let mut out = Vec::with_capacity(line.len());
    for chunk in line.utf8_chunks() {
        let valid = chunk.valid();
        let mut at = 0;
        while at < valid.len() {
            let len = token_len(&valid[at..]).filter(|_| table.len() < 32);
            if let Some(len) = len {
                let index = u8::try_from(table.len()).unwrap_or(0);
                out.extend_from_slice(&[ATOM, 0x80 + index]);
                table.push(valid.as_bytes()[at..at + len].to_vec());
                at += len;
            } else {
                let ch = valid[at..].chars().next().map_or(1, char::len_utf8);
                out.extend_from_slice(&valid.as_bytes()[at..at + ch]);
                at += ch;
            }
        }
        out.extend_from_slice(chunk.invalid());
    }
    out
}

/// The line a draft is sent as: every atom as its wire text.
fn resolve(draft: &[u8], table: &[Vec<u8>]) -> Vec<u8> {
    let mut line = Vec::with_capacity(draft.len());
    let mut at = 0;
    while at < draft.len() {
        if draft[at] == ATOM {
            line.extend_from_slice(&table[usize::from(draft[at + 1] - 0x80)]);
            at += 2;
        } else {
            line.push(draft[at]);
            at += 1;
        }
    }
    line
}

/// Where the word before the cursor begins in `head`: blanks skipped, then its
/// bytes. A big word (^W) is any run without whitespace; the other
/// (Opt+Backspace) is letters, digits and non-ASCII bytes.
fn word_start(head: &[u8], big: bool) -> usize {
    let word = |byte: &u8| {
        if big {
            !byte.is_ascii_whitespace()
        } else {
            byte.is_ascii_alphanumeric() || !byte.is_ascii()
        }
    };
    let blanks = head.iter().rev().take_while(|byte| !word(byte)).count();
    let kept = &head[..head.len() - blanks];
    // An attachment atom is a word of its own, and no word runs through one.
    let atom_ends = |end: usize| end >= 2 && kept[end - 2] == ATOM;
    if atom_ends(kept.len()) {
        return kept.len() - 2;
    }
    let mut start = kept.len();
    while start > 0 && word(&kept[start - 1]) && !atom_ends(start) {
        start -= 1;
    }
    start
}

/// The lines Enter enters when an owning console that restored `kept` is sent
/// `stream`, written out here from the key table and the editing rules without
/// `Keys` or `Composer`: text goes in at the cursor, which the arrows, Home,
/// End, Delete, Backspace and ^U move or erase; ^W, Opt+Backspace (`ESC DEL`)
/// and ^K take a word or the rest of the line into the slot ^U fills too,
/// which ^Y puts back once and Enter empties; ^V asks for a clipboard image
/// and types nothing; a paste is literal to its end
/// with CR and CRLF folded to LF; ^J and Alt+Enter (`ESC CR`/`ESC LF`) insert
/// a newline like text, and only a typed CR enters. A refused line goes back
/// to the draft, like the put-back, and may enter again on a later CR.
fn entered(kept: &[u8], stream: &[u8]) -> Vec<Vec<u8>> {
    const CLOSE: &[u8] = b"\x1b[201~";
    let mut table = Vec::new();
    let mut draft = atomize(kept, &mut table);
    let mut cursor = draft.len();
    let mut slot: Vec<u8> = Vec::new();
    let (mut lines, mut pasting, mut at) = (Vec::new(), false, 0);
    let mut skip = false;
    while at < stream.len() {
        let rest = &stream[at..];
        if pasting {
            let close = rest.windows(CLOSE.len()).position(|w| w == CLOSE);
            let held = (1..CLOSE.len())
                .rev()
                .find(|&n| rest.ends_with(&CLOSE[..n]));
            let text = &rest[..close.unwrap_or(rest.len() - held.unwrap_or(0))];
            let mut folded = Vec::with_capacity(text.len());
            for &byte in text {
                let eaten = skip && byte == b'\n';
                skip = byte == b'\r';
                if !eaten {
                    folded.push(if byte == b'\r' { b'\n' } else { byte });
                }
            }
            let folded = safe(&folded);
            draft.splice(cursor..cursor, folded.iter().copied());
            cursor += folded.len();
            let Some(n) = close else { break };
            (pasting, at, skip) = (false, at + n + CLOSE.len(), false);
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
                    (pasting, skip) = (true, false);
                    continue;
                }
                b"\x1b\r" | b"\x1b\n" => b"\n",
                b"\x1b\x7f" | b"\x1b\x08" => b"\x1b\x7f",
                seq @ (b"\x1b[D" | b"\x1bOD" | b"\x1b[C" | b"\x1bOC" | b"\x1b[3~") => seq,
                b"\x1b[H" | b"\x1bOH" | b"\x1b[1~" | b"\x1b[7~" => b"\x01",
                b"\x1b[F" | b"\x1bOF" | b"\x1b[4~" | b"\x1b[8~" => b"\x05",
                _ => continue,
            }
        } else {
            &rest[..1]
        };
        // ^V reaches no composer: it asks for an image and moves nothing.
        if key == b"\x16" {
            continue;
        }
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
            b"\x15" => {
                let cleared = std::mem::take(&mut draft);
                if !cleared.is_empty() {
                    slot = cleared;
                }
                cursor = 0;
            }
            b"\x17" | b"\x1b\x7f" => {
                let from = word_start(&draft[..cursor], key == b"\x17");
                if from < cursor {
                    slot = draft.drain(from..cursor).collect();
                    cursor = from;
                }
            }
            b"\x0b" => {
                let rest = &draft[cursor..];
                let len = match rest.iter().position(|&byte| byte == b'\n') {
                    Some(0) => 1,
                    Some(len) => len,
                    None => rest.len(),
                };
                if len > 0 {
                    slot = draft.drain(cursor..cursor + len).collect();
                }
            }
            b"\x19" => {
                if draft.len() + slot.len() <= input::CAP {
                    let back = std::mem::take(&mut slot);
                    draft.splice(cursor..cursor, back.iter().copied());
                    cursor += back.len();
                }
            }
            b"\r" if !draft.is_empty() => {
                let line = resolve(&std::mem::take(&mut draft), &table);
                (slot, table) = (Vec::new(), Vec::new());
                lines.push(line.clone());
                if refused_line(&line) {
                    // The put-back reads the line as a kept draft would be.
                    draft = atomize(&line, &mut table);
                    cursor = draft.len();
                } else {
                    cursor = 0;
                }
            }
            b"\r" => {}
            text => {
                let text = safe(text);
                draft.splice(at_text..at_text, text.iter().copied());
                cursor = at_text + text.len();
            }
        }
    }
    lines
}

/// Whether the command grammar refuses `line`, returning it to the draft:
/// the refused arms of `command_to` for the harness pair, written out here.
fn refused_line(line: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(line) else {
        return true;
    };
    if let Some(routed) = text.strip_prefix('@') {
        let cut = routed.split_once(char::is_whitespace);
        let (seat, body) = cut.unwrap_or((routed, ""));
        return seat.contains(':') || !["lead", "colead"].contains(&seat) || body.trim().is_empty();
    }
    if text.starts_with('/') {
        let words: Vec<_> = text.split_whitespace().collect();
        return match words.as_slice() {
            ["/close"] | ["/close", _] => false,
            ["/open", name] => !ae::config::is_agent_name(name),
            ["/more", n] => n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()),
            _ => true,
        };
    }
    text.trim().is_empty()
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
    assert_eq!(
        app_keys(stream, stream.len().max(1), at),
        app_keys(stream, chunk, at)
    );
    assert!(
        input::Keys::default()
            .feed(stream, at)
            .iter()
            .all(|(key, _)| !matches!(key, input::Key::Mouse(_)))
    );
    let (whole, line, view) = effects(kept, stream, stream.len().max(1), at, size);
    assert_eq!(
        effects(kept, stream, chunk, at, size),
        (whole.clone(), line.clone(), view.clone())
    );
    // The app's decoder hands the input a paste in as many fragments as the
    // terminal read it in; the draft it leaves must be the same for any split.
    assert_eq!(
        app_effects(kept, stream, chunk, at, size),
        app_effects(kept, stream, stream.len().max(1), at, size)
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
            Effect::Close(id) => assert!(
                id.as_deref()
                    .is_none_or(|id| !id.is_empty() && !id.contains(char::is_whitespace)),
                "a named close is one word"
            ),
            Effect::Open(seat) => panic!("no roster was set, so nothing opens: {seat:?}"),
            Effect::More(number) => assert!(
                !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()),
                "a fold number is digits: {number:?}"
            ),
            Effect::ImagePaste => assert!(stream.contains(&0x16), "only ^V asks for an image"),
            Effect::Paste(_) | Effect::Lane(_) | Effect::Styled(_) => {}
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
    // A stray 0xFF grows to three bytes, so the model is held well inside the cap.
    if (kept.len() + full.len()) * 3 <= input::CAP {
        let (all, _, _) = effects(kept, &full, chunk, at, size);
        let lines = entered(kept, &full);
        let after: Vec<_> = (all.iter().skip(usize::from(!kept.is_empty())))
            .filter(|effect| !matches!(effect, Effect::ImagePaste))
            .collect();
        assert_eq!(after.len(), lines.len(), "one effect per entered line");
        for (effect, line) in after.into_iter().zip(&lines) {
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
