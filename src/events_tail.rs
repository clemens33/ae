//! The `events-tail` read surface — SC-211n, SC-1306e.
//!
//! A monitor pane: a cyan banner, then the last thirty records of the session's
//! event container formatted one line each, then the same formatting applied to
//! every record appended afterwards. It never finishes on its own.
//!
//! # The frozen argv, and the successor spelling
//!
//! ```text
//! <AE_HOME>/sessions/<name>/events-tail  ->  ae _events-tail <AE_HOME>/sessions/<name>
//! ```
//!
//! Same underscore reasoning as [`crate::requests`]: `_validate_session_name`
//! forbids a leading `_`, so the spelling cannot shadow a session name. The
//! session label in the banner is the meta directory's own basename
//! (`${META_DIR##*/}`), so it comes from the path and is never a second
//! argument.
//!
//! # What the 38 corpus rows can and cannot pin
//!
//! Every one of them was captured by starting the helper, waiting four seconds
//! and killing it: `rc=143`, `bounded=4s=yes`. Two consequences, and they point
//! opposite ways.
//!
//! **stdout is fully determined and this module reproduces it.** Nothing
//! appended during those four seconds, so each capture is exactly
//! [`banner`] + [`replay`] over the frozen container.
//!
//! **stderr is not reproducible by anything that is not GNU bash, and the
//! corpus demonstrates that against itself.** The captured stderr is bash's own
//! job-control notification for the pipeline it was killed in — `Terminated: 15`
//! followed by the SOURCE TEXT of `tail -n 30 -f … | while IFS= read -r line`.
//! It is not a diagnostic `ae` ever wrote, and it is not stable: 37 of the 38
//! rows carry one byte string (149 B) while `arms/A1/c09-dupkey-unknown-ro`
//! carries the same message truncated after its first line (99 B). Byte-parity
//! with an unstable capture would be a requirement to reproduce a race.
//!
//! This surface therefore writes NOTHING to stderr. Not a normalisation and not
//! a claim of parity: a follow that is killed has nothing to report, and
//! inventing a farewell diagnostic would put bytes in the successor that no row
//! asks for. The 38 stderr comparisons are expected to score
//! FAIL-pending-ruling until the seats rule (lead ruling on F1, 2026-08-24); the
//! fixed comparison projection has no open-choice row admitting a span in
//! opaque-surface stderr.
//!
//! # SC-1306e
//!
//! The replay is a snapshot cut: the container is read once, and records that
//! arrive after that read are the FOLLOW's business, not the replay's. A record
//! appended while the replay is being written is therefore shown once, by the
//! follow, and not twice.

use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use crate::event_text::{
    CONTAINER, char_count, char_prefix, char_slice, container_exists, event_line, extract,
    last_records, pad_left_aligned, read_container, read_lines,
};

/// How many records the replay shows — the frozen `tail -n 30`.
pub const REPLAY_RECORDS: usize = 30;

/// How long a summary may be before it is cut, in CHARACTERS.
const SUMMARY_LIMIT: usize = 60;

/// What a cut summary keeps, in CHARACTERS, before the ellipsis.
const SUMMARY_KEPT: usize = 57;

/// The banner, byte for byte, for the session named `label`.
///
/// The three box lines have FIXED rule widths — the label is interpolated into
/// the top line without re-padding it — so a longer session name makes a longer
/// first line rather than a re-drawn box. That is the frozen behavior, and the
/// `tg11` capture (a four-character label) shows it directly.
///
/// The surrounding SGR pair is part of the banner: `ESC[1;36m` opens before the
/// box and `ESC[0m` plus a newline closes after it.
///
/// ```
/// let banner = ae::events_tail::banner(b"tg1");
/// assert!(banner.starts_with(b"\x1b[1;36m"));
/// assert!(banner.ends_with(b"\x1b[0m\n"));
/// assert_eq!(banner.iter().filter(|byte| **byte == b'\n').count(), 4);
/// ```
#[must_use]
pub fn banner(label: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"\x1b[1;36m");
    out.extend_from_slice("╭─ ae events — session: ".as_bytes());
    out.extend_from_slice(label);
    out.push(b' ');
    for _ in 0..31 {
        out.extend_from_slice("─".as_bytes());
    }
    out.extend_from_slice("╮\n".as_bytes());
    out.extend_from_slice(
        "│ date+time (UTC)  action   actor                  → target                 summary\n"
            .as_bytes(),
    );
    out.extend_from_slice("╰".as_bytes());
    for _ in 0..74 {
        out.extend_from_slice("─".as_bytes());
    }
    out.extend_from_slice("╯\n".as_bytes());
    out.extend_from_slice(b"\x1b[0m\n");
    out
}

/// One formatted line for one event line, or `None` for a line that is not one.
///
/// `printf '%s  %-8s %-22s → %-22s %s\n'` when the record carries a target and
/// `'%s  %-8s %-22s   %-22s %s\n'` when it does not — note that the arrow's
/// three-byte glyph is replaced by ONE space, so the no-target line is two bytes
/// narrower before its summary and the columns do not actually line up. That
/// asymmetry is in the frozen format strings and in every capture; it is
/// reproduced, not corrected.
///
/// The timestamp is cut to `MM-DDTHH:MM:SS` — fourteen CHARACTERS from character
/// offset five — so a day rollover stays visible in a monitor pane while the
/// full ISO value stays in the container for audit.
///
/// ```
/// use ae::events_tail::format_event;
///
/// let line = br#"{"ts":"2026-08-20T16:12:52Z","actor":"a:lead","action":"state","summary":"working"}"#;
/// let rendered = format_event(line).expect("an event line");
/// assert!(rendered.starts_with(b"08-20T16:12:52  state    a:lead"));
/// assert!(rendered.ends_with(b" working\n"));
/// assert_eq!(format_event(b"Terminated: 15"), None);
/// ```
#[must_use]
pub fn format_event(line: &[u8]) -> Option<Vec<u8>> {
    let line = event_line(line)?;
    let timestamp = extract(line, "ts");
    let actor = extract(line, "actor");
    let action = extract(line, "action");
    let target = extract(line, "target");
    let summary = extract(line, "summary");

    let mut out = Vec::new();
    out.extend_from_slice(char_slice(&timestamp, 5, 14));
    out.extend_from_slice(b"  ");
    pad_left_aligned(&mut out, &action, 8);
    out.push(b' ');
    pad_left_aligned(&mut out, &actor, 22);
    out.push(b' ');
    if target.is_empty() {
        // The no-target branch's THREE spaces, one of which stands where the
        // separating space would be: `%-22s   %-22s` against `%-22s → %-22s`.
        out.extend_from_slice(b"  ");
    } else {
        out.extend_from_slice("→ ".as_bytes());
    }
    pad_left_aligned(&mut out, &target, 22);
    out.push(b' ');
    out.extend_from_slice(&cut_summary(&summary));
    out.push(b'\n');
    Some(out)
}

/// `if ((${#summary} > 60)); then summary="${summary:0:57}..."; fi`.
///
/// CHARACTERS on both counts, and the frozen `G11/escapes` capture proves it
/// without reference to any probe: a 62-character, 66-byte summary is cut after
/// `and ` — the 57th CHARACTER. A byte cut would have landed five bytes earlier.
fn cut_summary(summary: &[u8]) -> Vec<u8> {
    if char_count(summary) <= SUMMARY_LIMIT {
        return summary.to_vec();
    }
    let mut out = char_prefix(summary, SUMMARY_KEPT).to_vec();
    out.extend_from_slice(b"...");
    out
}

/// The replay: the last [`REPLAY_RECORDS`] records of `container`, formatted.
///
/// `tail -n 30` counts RECORDS, and an unterminated remainder is one of them
/// even though the reader never yields it — so a torn container replays
/// twenty-nine lines, not thirty. Both halves of that are
/// [`crate::event_text`]'s measured framing, and both G8 captures show the
/// remainder absent from the output.
#[must_use]
pub fn replay(container: &[u8]) -> Vec<u8> {
    let window = last_records(container, REPLAY_RECORDS);
    let mut out = Vec::new();
    for line in read_lines(&window) {
        if let Some(rendered) = format_event(line) {
            out.extend_from_slice(&rendered);
        }
    }
    out
}

/// How long the frozen helper sleeps between checks for a container that does
/// not exist yet, and how long this one waits between reads once it does.
pub const POLL: Duration = Duration::from_secs(1);

/// Print the opening, then follow the container until the process is signalled.
///
/// The frozen helper waits in `while [[ ! -f "$EVENTS_FILE" ]]; do sleep 1; done`
/// before printing anything but the banner, because a fresh session has no
/// container until its first event — SC-519's quiet direction, as a wait rather
/// than as an error. That wait is unbounded there and is unbounded here.
///
/// The follow re-reads from the byte offset it stopped at and emits only whole
/// records, so a record caught mid-write is shown once, whole, on a later poll —
/// never as two half lines. `tail -f` gets that from `read` blocking on the
/// delimiter; this gets it from [`crate::event_text::read_lines`].
///
/// **This function does not return.** It has no completion condition, exactly as
/// the surface has none; the caller is a process whose lifetime IS the follow.
///
/// # Errors
///
/// Returns the [`io::Error`] from writing to `out` — a closed pane is the way
/// this ends when it is not a signal.
pub fn follow(dir: &Path, out: &mut impl Write) -> io::Result<std::convert::Infallible> {
    let label = dir
        .file_name()
        .map_or_else(Vec::new, |name| name.as_encoded_bytes().to_vec());
    out.write_all(&banner(&label))?;
    out.flush()?;

    let container = dir.join(CONTAINER);
    while !container_exists(&container) {
        std::thread::sleep(POLL);
    }

    // `None` is "the replay window has not been taken yet", which is NOT the
    // same as offset zero: a container that exists but is still empty has
    // offset zero forever, and conflating the two re-entered the replay branch
    // on every poll.
    let mut consumed: Option<usize> = None;
    loop {
        let body = read_container(&container);
        match consumed {
            None => {
                // The first read IS the replay window; everything after it
                // belongs to the follow. Consuming the whole body here is what
                // keeps the two from showing one record twice.
                out.write_all(&replay(&body))?;
                out.flush()?;
                consumed = Some(complete_len(&body));
            }
            // Truncated or replaced beneath us. `tail -f` reports the shrink and
            // starts over; there is nothing to gain from guessing an offset into
            // a container that is no longer the one we were reading.
            Some(offset) if body.len() < offset => consumed = None,
            Some(offset) => {
                let fresh = &body[offset..];
                if !fresh.is_empty() {
                    for line in read_lines(fresh) {
                        if let Some(rendered) = format_event(line) {
                            out.write_all(&rendered)?;
                        }
                    }
                    out.flush()?;
                    consumed = Some(offset + complete_len(fresh));
                }
            }
        }
        std::thread::sleep(POLL);
    }
}

/// The length of `bytes` up to and including its last `\n` — the offset a reader
/// that only consumes whole records stops at.
fn complete_len(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1)
}

#[cfg(test)]
mod tests {
    use super::{
        CONTAINER, REPLAY_RECORDS, banner, complete_len, cut_summary, format_event, read_container,
        replay,
    };
    use std::fs;
    use std::path::PathBuf;

    fn text(bytes: &[u8]) -> String {
        String::from_utf8(bytes.to_vec()).expect("test fixtures are utf-8")
    }

    fn container(lines: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for line in lines {
            out.extend_from_slice(line.as_bytes());
            out.push(b'\n');
        }
        out
    }

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("ae-eventstail-{}-{tag}", std::process::id()))
                .join(tag);
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("scratch");
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_banner_is_the_frozen_bytes() {
        // Transcribed from the frozen capture
        // arms/A1/c01-healthy-ro/out/events-tail.stdout, not rebuilt from the
        // same loops the code uses.
        let expected = concat!(
            "\u{1b}[1;36m",
            "╭─ ae events — session: tg1 ───────────────────────────────╮\n",
            "│ date+time (UTC)  action   actor                  → target                 summary\n",
            "╰──────────────────────────────────────────────────────────────────────────╯\n",
            "\u{1b}[0m\n",
        );
        assert_eq!(text(&banner(b"tg1")), expected);
    }

    #[test]
    fn a_longer_label_lengthens_the_first_line_and_leaves_the_rules_alone() {
        let short = banner(b"tg1");
        let long = banner(b"tg11");
        assert_eq!(
            long.len(),
            short.len() + 1,
            "the label is interpolated, never padded"
        );
        let rule = |bytes: &[u8]| {
            text(bytes)
                .lines()
                .nth(2)
                .expect("the bottom rule")
                .to_owned()
        };
        assert_eq!(rule(&short), rule(&long));
    }

    #[test]
    fn the_target_and_no_target_lines_are_the_two_frozen_formats() {
        let with = format_event(
            br#"{"ts":"2026-08-20T16:12:54Z","actor":"fake:lead","action":"send","target":"fake:worker","summary":"body"}"#,
        )
        .expect("an event");
        let without = format_event(
            br#"{"ts":"2026-08-20T16:12:52Z","actor":"fake:lead","action":"state","summary":"building the healthy fixture"}"#,
        )
        .expect("an event");
        // Byte offsets measured on the frozen c01 capture: the summary starts at
        // 75 with a target and at 73 without one, because the arrow's glyph is
        // three bytes and its replacement is one space.
        assert_eq!(
            with.windows(4).position(|w| w == b"body"),
            Some(75),
            "{}",
            text(&with)
        );
        assert_eq!(
            without.windows(8).position(|w| w == b"building"),
            Some(73),
            "{}",
            text(&without)
        );
        assert_eq!(&with[48..51], "→".as_bytes());
    }

    #[test]
    fn the_timestamp_is_fourteen_characters_from_offset_five() {
        let rendered = format_event(br#"{"ts":"2026-08-20T16:12:52Z","actor":"a","action":"x"}"#)
            .expect("an event");
        assert!(rendered.starts_with(b"08-20T16:12:52  "));
        // A short or absent timestamp is an empty column, not a panic.
        let short = format_event(br#"{"ts":"2026","actor":"a","action":"x"}"#).expect("an event");
        assert!(short.starts_with(b"  x       "), "{}", text(&short));
        let none = format_event(br#"{"actor":"a","action":"x"}"#).expect("an event");
        assert!(none.starts_with(b"  x       "), "{}", text(&none));
    }

    #[test]
    fn a_long_summary_is_cut_after_fifty_seven_characters() {
        // The G11 capture's own summary, and its own expected cut. 62 characters
        // in 66 bytes: only a CHARACTER count exceeds 60 here and only a
        // CHARACTER slice stops after `and `.
        let summary = "⟦ae:msg from fake:lead⟧ quote class: he said \"hello\" and 'bye'";
        assert_eq!(summary.chars().count(), 62);
        assert_eq!(summary.len(), 66);
        assert_eq!(
            text(&cut_summary(summary.as_bytes())),
            "⟦ae:msg from fake:lead⟧ quote class: he said \"hello\" and ..."
        );
    }

    #[test]
    fn a_summary_at_the_limit_is_left_alone() {
        let sixty = "x".repeat(60);
        assert_eq!(cut_summary(sixty.as_bytes()), sixty.as_bytes());
        let sixty_one = "x".repeat(61);
        assert_eq!(
            text(&cut_summary(sixty_one.as_bytes())),
            format!("{}...", "x".repeat(57)),
            "the boundary is > 60, not >= 60"
        );
        // Sixty multibyte characters are 120 bytes and still under the limit: a
        // byte test would have cut this one.
        let wide = "α".repeat(60);
        assert_eq!(cut_summary(wide.as_bytes()), wide.as_bytes());
    }

    #[test]
    fn a_line_that_is_not_an_event_is_no_line_at_all() {
        assert_eq!(format_event(b""), None);
        assert_eq!(format_event(b"Terminated: 15"), None);
        assert_eq!(format_event(b" {\"ts\":\"x\"}"), None);
        // Unlike the request sensor, this surface does NOT require a ref: every
        // brace-prefixed line is shown, which is why an unknown action appears.
        let unknown =
            format_event(br#"{"ts":"2026-08-20T16:00:00Z","actor":"a","action":"frobnicate"}"#);
        assert!(text(&unknown.expect("shown")).contains("frobnicate"));
    }

    #[test]
    fn the_replay_is_the_last_thirty_records() {
        let mut lines = Vec::new();
        for index in 0..35 {
            lines.push(format!(
                r#"{{"ts":"2026-08-20T16:00:{index:02}Z","actor":"a","action":"n{index}"}}"#
            ));
        }
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let rendered = text(&replay(&container(&refs)));
        assert_eq!(rendered.lines().count(), REPLAY_RECORDS);
        assert!(rendered.contains("n5 "), "the 30th-from-last is n5");
        assert!(!rendered.contains("n4 "), "n4 is one too far back");
        assert!(rendered.contains("n34"));
    }

    #[test]
    fn an_unterminated_remainder_costs_a_replay_slot_and_is_never_shown() {
        let mut body = Vec::new();
        for index in 0..31 {
            body.extend_from_slice(
                format!(
                    r#"{{"ts":"2026-08-20T16:00:{index:02}Z","actor":"a","action":"n{index}"}}"#
                )
                .as_bytes(),
            );
            body.push(b'\n');
        }
        body.extend_from_slice(br#"{"ts":"2026-08-20T16:00:99Z","actor":"a","action":"tor"#);
        let rendered = text(&replay(&body));
        assert_eq!(
            rendered.lines().count(),
            29,
            "the remainder occupies one of the thirty and yields nothing"
        );
        assert!(!rendered.contains("tor"));
        assert!(rendered.contains("n30"));
        assert!(rendered.contains("n2 "));
        assert!(!rendered.contains("n1 "));
    }

    #[test]
    fn an_absent_or_unreadable_container_replays_nothing() {
        // The frozen reader is `tail … 2>/dev/null`, so there is one answer for
        // "no container" and "no permission": no lines, no complaint. The
        // container read is `event_text`'s quiet door; this pins that the
        // replay over what it returns is empty rather than an error.
        let scratch = Scratch::new("nolog");
        let missing = read_container(&scratch.0.join(CONTAINER));
        assert!(missing.is_empty());
        assert!(replay(&missing).is_empty());
        // A directory where the container should be reads the same way.
        fs::create_dir_all(scratch.0.join(CONTAINER)).expect("a directory in its place");
        assert!(replay(&read_container(&scratch.0.join(CONTAINER))).is_empty());
    }

    #[test]
    fn the_consumed_offset_stops_at_the_last_complete_record() {
        assert_eq!(complete_len(b"a\nb\n"), 4);
        assert_eq!(complete_len(b"a\nb"), 2, "the remainder is not consumed");
        assert_eq!(complete_len(b"nope"), 0);
        assert_eq!(complete_len(b""), 0);
    }
}
