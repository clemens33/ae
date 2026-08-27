//! The opaque event-text layer the generated `requests` and `events-tail`
//! helpers read their bytes through — SC-211d, SC-211n, SC-1306d, SC-1306e.
//!
//! # Why this is not [`crate::events`]
//!
//! [`crate::events`] is the TYPED reader. It parses a record, enforces
//! SC-510a–f, and under SC-520 skips a malformed complete line and marks the
//! session degraded. The two helper read surfaces do none of that. They take a
//! line, search it for `"key":"`, and keep whatever follows up to the first
//! unescaped quote — so a line that is not JSON at all still yields fields, and
//! a duplicate key silently resolves to its first occurrence. The 206 opaque P1
//! rows compare those bytes, so the extraction they were captured through is
//! its own module rather than a mode of the typed one.
//!
//! # Bytes, not `String`
//!
//! Every function here works on `[u8]`. The helpers are shell pipelines: `read`
//! frames on `\n` and nothing else, and a value that is not valid UTF-8 is
//! copied through verbatim. Decoding first would make an undecodable event log
//! render differently from the frozen capture — and the escape set the walker
//! recognises (`\`, `n`, `t`, `r`, `"`) is ASCII, which cannot occur as a UTF-8
//! continuation byte, so a byte walk and a character walk agree on every
//! sequence that decodes at all.
//!
//! # Framing is per surface, and the two surfaces do not agree
//!
//! MEASURED (bash 5.3.15, BSD userland, 2026-08-24), not derived:
//!
//! - `requests` reads through `_ae_tac`, i.e. `tac` or BSD `tail -r`. Neither
//!   adds a newline, so a container whose last record is unterminated emits that
//!   remainder FIRST, immediately followed by the previous line — the reader
//!   sees the two GLUED into one line. Probe: `a\nb\n{partial` through
//!   `tail -r` into `while IFS= read -r` yields `{partialb` then `a`.
//! - `events-tail` reads through `tail -n 30 -f`, forward, where the same
//!   unterminated remainder is simply not yet a line and `read` never yields it.
//!
//! [`reversed`] and [`last_records`] produce those two byte streams; both are
//! then framed by the one [`read_lines`], because in the pipeline both are the
//! same `while IFS= read -r line` loop.

use std::borrow::Cow;

/// The event container's filename under a session meta directory.
///
/// One spelling for both surfaces. DR-001 defers the written multi-generation
/// layout, and these two surfaces are the bash-era readers: the frozen helpers
/// both name `${META_DIR}/events.jsonl` literally, so inventing a pattern here
/// would invent the half of the DR that is deliberately unwritten.
pub const CONTAINER: &str = "events.jsonl";

/// The container's bytes, or none at all.
///
/// **A DOOR** in the sense `clippy.toml` means, and the only one either helper
/// surface needs: both of them read the same `events.jsonl` the same quiet way,
/// so the read lives with the framing rather than once per surface.
///
/// Quiet on every failure, which is the frozen behavior and not a tolerance
/// choice. The request sensor guards with `[[ -f "$file" ]] || return 0` and
/// then reads through `_ae_tac "$file" 2>/dev/null || true`; `events-tail` reads
/// through `tail … 2>/dev/null`; the state read guards the same way. An absent
/// container, an unreadable one and anything that is not a regular file in
/// its place are therefore indistinguishable at these surfaces: no diagnostic,
/// no rows, `rc=0`.
///
/// **The `-f` gate is applied HERE, before anything is opened**, and it is not
/// decoration: a FIFO in the container's place blocks whoever opens it for a
/// reader that never comes, and an unconditional read left the core hanging
/// with no stdout, no stderr and no exit where the frozen bodies answered
/// empty at 0 (found in review, reproduced). The gate follows symlinks, as
/// `-f` does.
///
/// That is SC-519's quiet-empty direction and deliberately NOT SC-509b's
/// degraded direction, which belongs to [`crate::events`]: a degradation has to
/// be publishable somewhere, and neither of these surfaces has a field to
/// publish it in.
#[must_use]
pub fn read_container(path: &std::path::Path) -> Vec<u8> {
    if !container_exists(path) {
        return Vec::new();
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the opaque event-container read shared by the helper \
                  read surfaces, and by `send` for a delivery's recovery \
                  record — see clippy.toml"
    )]
    let body = std::fs::read(path);
    body.unwrap_or_default()
}

/// Whether the container exists yet — the frozen `[[ ! -f "$EVENTS_FILE" ]]`
/// wait in `events-tail`, which exists because a fresh session has no container
/// until its first event (SC-519).
#[must_use]
pub fn container_exists(path: &std::path::Path) -> bool {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the lazily-created event container's existence test — \
                  see clippy.toml"
    )]
    let present = path.is_file();
    present
}

/// The records `tac` and `tail` count: every byte up to and including a `\n`,
/// plus a final unterminated remainder when the container does not end in one.
///
/// A record is NOT a line. `tail -n 30` over 31 terminated lines followed by an
/// unterminated remainder emits from line 3 onward — 29 terminated records plus
/// the remainder — because the remainder is one of the thirty (measured).
fn records(bytes: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            out.push(&bytes[start..=index]);
            start = index + 1;
        }
    }
    if start < bytes.len() {
        out.push(&bytes[start..]);
    }
    out
}

/// The byte stream `_ae_tac` produces — records in reverse order, concatenated.
///
/// The glue is the point. Reversing RECORDS rather than lines reproduces
/// `tail -r`'s refusal to invent a newline for an unterminated tail, so the
/// remainder lands first and runs into what was the line before it. Modelling
/// this as "lines, reversed" would silently repair a torn container that the
/// frozen helper reads corrupted.
///
/// ```
/// use ae::event_text::{read_lines, reversed};
///
/// let stream = reversed(b"a\nb\n{partial");
/// assert_eq!(stream, b"{partialb\na\n");
/// assert_eq!(read_lines(&stream), [b"{partialb".as_slice(), b"a".as_slice()]);
/// ```
#[must_use]
pub fn reversed(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    for record in records(bytes).into_iter().rev() {
        out.extend_from_slice(record);
    }
    out
}

/// The byte stream `tail -n <count>` produces — the LAST `count` records.
///
/// ```
/// use ae::event_text::last_records;
///
/// assert_eq!(last_records(b"a\nb\nc\n", 2), b"b\nc\n");
/// assert_eq!(last_records(b"a\nb\n", 9), b"a\nb\n");
/// ```
#[must_use]
pub fn last_records(bytes: &[u8], count: usize) -> Vec<u8> {
    let all = records(bytes);
    let start = all.len().saturating_sub(count);
    let mut out = Vec::new();
    for record in &all[start..] {
        out.extend_from_slice(record);
    }
    out
}

/// The lines a `while IFS= read -r line` loop yields: complete lines only.
///
/// `read` returns non-zero at end-of-input without a delimiter, so the loop
/// body never runs for a trailing remainder. Dropping it here is that fact, not
/// a tolerance choice.
#[must_use]
pub fn read_lines(bytes: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            out.push(&bytes[start..index]);
            start = index + 1;
        }
    }
    out
}

/// A line both helpers accept as an event: nonempty and `{`-prefixed.
///
/// `[[ "$line" == \{* ]] || continue` in both. Nothing here validates JSON —
/// that is exactly the difference from [`crate::events`].
#[must_use]
pub fn event_line(line: &[u8]) -> Option<&[u8]> {
    line.first().filter(|byte| **byte == b'{').map(|_| line)
}

/// The value of the FIRST flat `"key":"…"` member, or empty when absent.
///
/// One function for two frozen bodies. `_event_json_str` (used by `requests`
/// through `_lib`) and `helper_events_tail_extract_json_str` are a hand-copy
/// pair — the frozen source says so at the definition site — and they agree
/// down to the fast path, so a second Rust extractor would be the drift the
/// frozen comment is complaining about.
///
/// The unescape set is `ae_emit_event`'s and only that: `\n` and `\t` become one
/// SPACE, `\r` is dropped entirely, `\"` and `\\` unescape, and any other
/// escape is kept as BOTH its characters. There is no `\uXXXX` handling, because
/// the emitter never writes one.
///
/// ```
/// use ae::event_text::extract;
///
/// let line = br#"{"ref":"first","summary":"a\nb\tc\\d\"e","ref":"second"}"#;
/// assert_eq!(extract(line, "ref"), b"first");
/// assert_eq!(extract(line, "summary"), br#"a b c\d"e"#);
/// assert_eq!(extract(line, "absent"), b"");
/// ```
#[must_use]
pub fn extract(line: &[u8], key: &str) -> Vec<u8> {
    member(line, key).value().unwrap_or_default().to_vec()
}

/// A flat string member's three states — SC-511b's, read off opaque text.
///
/// [`extract`] answers with bytes and so cannot tell an ABSENT key from one
/// that is present and EMPTY. The frozen `requests` matcher never needed to:
/// it tested `-n` and both look the same to that. **The RULED matcher does**,
/// because SC-511b/SC-405j make those two different identities — both members
/// absent falls back to the display name, while a member present and empty is
/// a writer that meant to route and did not say where, which identifies
/// nobody. Collapsing them would silently make an `Unassociated` side compare
/// as a `Display` side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Member<'a> {
    /// The key is not in the record.
    Absent,
    /// The key is present and its value is empty.
    Empty,
    /// The key carries a nonempty value.
    ///
    /// Borrowed on the fast path and owned only when the value carried an
    /// escape, so the common case costs no copy while an unescaped value still
    /// has somewhere to live.
    Value(Cow<'a, [u8]>),
}

impl Member<'_> {
    /// The bytes, or `None` when the key is absent. `Empty` answers `Some(&[])`.
    #[must_use]
    pub fn value(&self) -> Option<&[u8]> {
        match self {
            Self::Absent => None,
            Self::Empty => Some(&[]),
            Self::Value(bytes) => Some(bytes),
        }
    }
}

/// Read one flat `"key":"…"` member, keeping absent and empty apart.
///
/// This is [`extract`]'s own body; `extract` is the byte-only view of it. The
/// unescaping and the first-occurrence rule are identical, because they have to
/// be: one reader, two views, so a value cannot mean one thing to a matcher and
/// another to a formatter.
///
/// A member whose value is present but empty AFTER unescaping — `"\r"`, say,
/// which the emitter's escape set drops entirely — answers [`Member::Empty`],
/// because what the identity rule compares is the value, not the spelling.
///
/// ```
/// use ae::event_text::{Member, member};
///
/// let line = br#"{"actor_slot":"","actor_session":"s"}"#;
/// assert_eq!(member(line, "actor_slot"), Member::Empty);
/// assert_eq!(member(line, "target_slot"), Member::Absent);
/// assert_eq!(member(line, "actor_session").value(), Some(b"s".as_slice()));
///
/// // Empty and Absent both have no bytes, and are NOT the same identity.
/// assert_eq!(member(line, "actor_slot").value(), Some(b"".as_slice()));
/// assert_eq!(member(line, "target_slot").value(), None);
/// ```
#[must_use]
pub fn member<'a>(line: &'a [u8], key: &str) -> Member<'a> {
    let mut needle = Vec::with_capacity(key.len() + 4);
    needle.push(b'"');
    needle.extend_from_slice(key.as_bytes());
    needle.extend_from_slice(b"\":\"");
    let Some(offset) = find(line, &needle) else {
        return Member::Absent;
    };
    let rest = &line[offset + needle.len()..];
    // The frozen fast path: when nothing before the first quote is a backslash,
    // that quote is the real terminator and there is nothing to unescape.
    let head = match find(rest, b"\"") {
        Some(end) => &rest[..end],
        None => rest,
    };
    let resolved = if head.contains(&b'\\') {
        Cow::Owned(unescape(rest))
    } else {
        Cow::Borrowed(head)
    };
    // A value that RESOLVES to nothing is `Empty`, however it was spelled: the
    // identity rule compares values, not spellings, and `"\r"` unescapes away
    // entirely under the emitter's own escape set.
    if resolved.is_empty() {
        Member::Empty
    } else {
        Member::Value(resolved)
    }
}

/// The per-character walk both frozen bodies fall through to.
fn unescape(rest: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rest.len());
    let mut index = 0;
    while index < rest.len() {
        let byte = rest[index];
        if byte == b'\\' {
            match rest.get(index + 1) {
                Some(b'n' | b't') => out.push(b' '),
                Some(b'r') => {}
                Some(b'"') => out.push(b'"'),
                // `\\` unescapes to one backslash, and so does a TRAILING lone
                // backslash — bash reads `${rest:$((i+1)):1}` past the end as
                // the empty string, whose `case` falls to `*)` and appends `$c`
                // followed by nothing. Two different reasons, one byte.
                Some(b'\\') | None => out.push(b'\\'),
                Some(other) => {
                    out.push(b'\\');
                    out.push(*other);
                }
            }
            index += 2;
        } else if byte == b'"' {
            break;
        } else {
            out.push(byte);
            index += 1;
        }
    }
    out
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// `${#value}` — bash's parameter length, which is CHARACTERS under a UTF-8
/// locale and not bytes.
///
/// MEASURED both ways, and the two disagree, so neither may be assumed: this
/// counts characters (`${#s}` of `⟦abc⟧` is 5), while
/// [`pad_left_aligned`] pads to BYTES. The frozen `events-tail` capture for
/// `G11/escapes` decides it independently of any probe — a 62-character,
/// 66-byte summary truncates after `and `, which only a character count and a
/// character slice produce.
///
/// A byte that cannot start a UTF-8 sequence counts as one character and
/// advances one byte, so the count is total. No fixture in the corpus carries
/// undecodable event text, so that extension is the honest generalisation of
/// the measured behavior and not itself measured.
#[must_use]
pub fn char_count(bytes: &[u8]) -> usize {
    char_starts(bytes).count()
}

/// `${value:0:len}` — a prefix of `len` CHARACTERS.
#[must_use]
pub fn char_prefix(bytes: &[u8], len: usize) -> &[u8] {
    match char_starts(bytes).nth(len) {
        Some(offset) => &bytes[..offset],
        None => bytes,
    }
}

/// `${value:start:len}` — `len` CHARACTERS from character offset `start`.
///
/// Bash yields the empty string when `start` is past the end and a short slice
/// when fewer than `len` characters remain; both fall out of the clamping here.
///
/// ```
/// use ae::event_text::char_slice;
///
/// // The frozen `short_ts` cut: 14 characters from offset 5.
/// assert_eq!(char_slice(b"2026-08-20T16:12:52Z", 5, 14), b"08-20T16:12:52");
/// assert_eq!(char_slice(b"short", 9, 14), b"");
/// ```
#[must_use]
pub fn char_slice(bytes: &[u8], start: usize, len: usize) -> &[u8] {
    let Some(from) = char_starts(bytes).nth(start) else {
        return &[];
    };
    let tail = &bytes[from..];
    char_prefix(tail, len)
}

/// The byte offset at which each character starts.
///
/// `nth(n)` is therefore both "where character `n` begins" and "where the first
/// `n` characters end", which is what makes one iterator serve the count and
/// both slices. `None` from `nth(n)` means there are at most `n` characters —
/// the whole value — and that is exactly bash's short-slice behavior.
fn char_starts(bytes: &[u8]) -> impl Iterator<Item = usize> + '_ {
    let mut index = 0;
    std::iter::from_fn(move || {
        if index >= bytes.len() {
            return None;
        }
        let here = index;
        // `min` keeps a truncated multibyte lead from stepping past the end.
        index += utf8_width(bytes[here]).min(bytes.len() - here);
        Some(here)
    })
}

/// The length of the UTF-8 sequence a lead byte opens; `1` for any byte that
/// opens none, which is how bash's own indexing advances over invalid input.
fn utf8_width(lead: u8) -> usize {
    match lead {
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        // ASCII and every byte that opens no sequence at all, together: one
        // byte is one step, which is how bash's own indexing walks past invalid
        // input rather than stalling on it.
        _ => 1,
    }
}

/// `printf '%-<width>s'` — left-aligned, padded to `width` BYTES.
///
/// MEASURED (bash 5.3.15, `LC_ALL=en_US.UTF-8`): `printf '%-8s'` of `αβ` — two
/// characters, four bytes — emits eight BYTES, four of them padding. Rust's own
/// `{:<8}` counts characters and would emit ten. An over-long field is never
/// truncated: the frozen `A6` capture carries a 31-character request id in a
/// `%-28s` column with exactly one space after it.
pub fn pad_left_aligned(out: &mut Vec<u8>, field: &[u8], width: usize) {
    out.extend_from_slice(field);
    for _ in field.len()..width {
        out.push(b' ');
    }
}

#[cfg(test)]
mod tests {
    use super::{
        char_count, char_prefix, char_slice, event_line, extract, last_records, pad_left_aligned,
        read_lines, records, reversed,
    };

    fn padded(field: &[u8], width: usize) -> Vec<u8> {
        let mut out = Vec::new();
        pad_left_aligned(&mut out, field, width);
        out
    }

    #[test]
    fn a_record_keeps_its_newline_and_a_remainder_is_still_a_record() {
        assert_eq!(
            records(b"a\nb\n"),
            [b"a\n".as_slice(), b"b\n".as_slice()],
            "two terminated records"
        );
        assert_eq!(
            records(b"a\nb"),
            [b"a\n".as_slice(), b"b".as_slice()],
            "the remainder counts, which is why tail -n 30 shifts by one"
        );
        assert_eq!(records(b""), [] as [&[u8]; 0]);
    }

    #[test]
    fn reversing_records_glues_an_unterminated_tail_onto_the_line_before_it() {
        // The measured `tail -r` behavior, as the property that makes it
        // matter: after reversal the two are ONE line to `read`.
        assert_eq!(reversed(b"a\nb\n{partial"), b"{partialb\na\n");
        assert_eq!(
            read_lines(&reversed(b"a\nb\n{partial")),
            [b"{partialb".as_slice(), b"a".as_slice()]
        );
        // A fully terminated container reverses to the same lines, in order.
        assert_eq!(
            read_lines(&reversed(b"a\nb\nc\n")),
            [b"c".as_slice(), b"b".as_slice(), b"a".as_slice()]
        );
        // A single unterminated record survives reversal and is then dropped by
        // the reader, exactly as `read` drops it.
        assert_eq!(reversed(b"only"), b"only");
        assert_eq!(read_lines(&reversed(b"only")), [] as [&[u8]; 0]);
    }

    #[test]
    fn tail_n_counts_records_so_a_remainder_displaces_a_line() {
        let mut body = Vec::new();
        for line in 1..=31 {
            body.extend_from_slice(format!("L{line}\n").as_bytes());
        }
        body.extend_from_slice(b"PARTIAL");
        let tail = last_records(&body, 30);
        assert!(
            tail.starts_with(b"L3\n"),
            "measured: tail -n 30 starts at L3"
        );
        assert!(tail.ends_with(b"L31\nPARTIAL"));
        assert_eq!(
            read_lines(&tail).len(),
            29,
            "the remainder occupies a slot and is then never yielded"
        );
    }

    #[test]
    fn read_lines_never_yields_an_unterminated_remainder() {
        assert_eq!(read_lines(b"a\nb\n"), [b"a".as_slice(), b"b".as_slice()]);
        assert_eq!(read_lines(b"a\nb"), [b"a".as_slice()]);
        assert_eq!(read_lines(b"only"), [] as [&[u8]; 0]);
        assert_eq!(read_lines(b""), [] as [&[u8]; 0]);
        assert_eq!(
            read_lines(b"\n\n"),
            [b"".as_slice(), b"".as_slice()],
            "an empty line is a line"
        );
    }

    #[test]
    fn only_a_brace_prefixed_line_is_an_event_line() {
        assert_eq!(event_line(b"{\"a\":1}"), Some(b"{\"a\":1}".as_slice()));
        assert_eq!(event_line(b" {\"a\":1}"), None);
        assert_eq!(event_line(b"Terminated"), None);
        assert_eq!(event_line(b""), None);
    }

    #[test]
    fn extract_takes_the_first_occurrence_of_a_duplicated_key() {
        let line = br#"{"ref":"first","ref":"second"}"#;
        assert_eq!(extract(line, "ref"), b"first");
    }

    #[test]
    fn extract_unescapes_exactly_the_emitter_set() {
        let line = br#"{"s":"a\nb\tc\rd\"e\\f\qg"}"#;
        // `\n` and `\t` become ONE space, `\r` is dropped entirely — so `c` and
        // `d` end up adjacent, which the frozen G11 capture shows independently
        // (`cr class: before\rafter` renders as `cr class: beforeafter`). `\"`
        // and `\\` unescape to themselves; `\q` is kept as both characters.
        assert_eq!(extract(line, "s"), br#"a b cd"e\f\qg"#);
    }

    #[test]
    fn extract_is_empty_for_an_absent_key_and_for_a_prefix_of_one() {
        let line = br#"{"summary":"x"}"#;
        assert_eq!(extract(line, "sum"), b"");
        assert_eq!(extract(line, "absent"), b"");
        assert_eq!(extract(line, ""), b"");
    }

    #[test]
    fn extract_survives_a_value_that_never_closes() {
        // `${rest%%\"*}` leaves the whole tail when there is no quote; the walk
        // then ends at the input's end rather than losing the value.
        assert_eq!(extract(br#"{"s":"unterminated"#, "s"), b"unterminated");
        assert_eq!(extract(br#"{"s":"trailing\"#, "s"), b"trailing\\");
    }

    #[test]
    fn extract_copies_undecodable_bytes_through_verbatim() {
        // A byte walk and a character walk agree here only because the escape
        // set is ASCII. Decoding first would lose the value entirely.
        let mut line = Vec::from(&br#"{"s":"a"#[..]);
        line.extend_from_slice(&[0xFF, 0xFE]);
        line.extend_from_slice(br#"b"}"#);
        assert_eq!(extract(&line, "s"), [b'a', 0xFF, 0xFE, b'b']);
    }

    #[test]
    fn length_and_slicing_count_characters() {
        assert_eq!(char_count("⟦abc⟧".as_bytes()), 5, "not the 9 bytes");
        assert_eq!(char_prefix("⟦abc⟧".as_bytes(), 3), "⟦ab".as_bytes());
        assert_eq!(char_prefix(b"ab", 9), b"ab", "a short value is not padded");
        assert_eq!(
            char_slice(b"2026-08-20T16:12:52Z", 5, 14),
            b"08-20T16:12:52"
        );
        assert_eq!(char_slice(b"2026", 5, 14), b"", "past the end is empty");
        assert_eq!(
            char_slice(b"abcdef", 4, 14),
            b"ef",
            "short tail, not an error"
        );
    }

    #[test]
    fn an_undecodable_byte_counts_as_one_character() {
        assert_eq!(char_count(&[0xFF, 0xFE]), 2);
        assert_eq!(char_prefix(&[0xFF, b'a'], 1), &[0xFF]);
        // A truncated multibyte lead does not read past the end.
        assert_eq!(char_count(&[0xE2, 0x86]), 1);
        assert_eq!(char_prefix(&[0xE2, 0x86], 1), &[0xE2, 0x86]);
    }

    #[test]
    fn padding_counts_bytes_and_never_truncates() {
        assert_eq!(padded(b"ab", 8), b"ab      ");
        assert_eq!(
            padded("αβ".as_bytes(), 8),
            "αβ    ".as_bytes(),
            "four bytes of value, four of padding — not two characters of value"
        );
        assert_eq!(padded(b"abcdefgh", 8), b"abcdefgh");
        assert_eq!(
            padded(b"review-20260820T161305Z-dc302d09", 28),
            b"review-20260820T161305Z-dc302d09",
            "the A6 capture's 31-character id overflows its column"
        );
    }
}
