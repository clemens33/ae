//! The Antigravity (`agy`) board reader: history.jsonl bytes in, human rows out.
//!
//! PURE: no I/O, no env, no clock. ONE file per home is shared by every agy
//! conversation, so attribution is BY CONVERSATION ID ALONE: a record whose
//! `conversationId` differs from the seat's captured id is skipped silently —
//! it belongs to another conversation, or to a pre-field CLI that can never
//! match. `workspace` is NEVER consulted: two agy seats in one working
//! directory would cross-wire, exactly the hazard the capture module documents.
//!
//! A record is one prompt the human typed: `display` is its text, `timestamp`
//! is integer MILLIS native (→ micros ×1000), and a `"type":"slash_command"`
//! record is KEPT — the human typed it. No assistant text lives in this store:
//! with `--assistant` the reader covers that fact once per read, unless the
//! caller already produced assistant rows from the transcript leg (the
//! `assistant_rows_found` bit), and silent (documented) with the flag off.
//! A follow poll whose first pass read the replies covers that once-read
//! instead (the `assistant_read_once` bit, bound by the agy follow arm).
//! ae's context rides `-i` as a USER turn, so the ctx turn IS a history record:
//! line 1 carries the ae marker → `is_ae_turn` drops it.
//!
//! Verified 2026-09-17 over the local store (shapes and counts only): 152
//! records, 126 with a `conversationId`, 5 `slash_command`, every `timestamp`
//! an integral non-negative millis value. The per-conversation `.db` files are
//! deferred and never opened here.

use super::{LineBody, Splitter, Streamed};
use crate::board::{Coverage, Role, Row};
use crate::tool::ToolKind;

/// Read one agy history from whole bytes: one feed through the ONE splitter,
/// then read the stream bound to `seat_id`. A thin wrapper, so the fuzz target
/// covers the code the door runs.
#[must_use]
pub fn read(
    bytes: &[u8],
    actor: &str,
    file: &str,
    source: ToolKind,
    seat_id: &str,
) -> (Vec<Row>, Vec<Coverage>) {
    let mut splitter = Splitter::new();
    splitter.feed(bytes);
    read_stream(&splitter.finish().for_seat(seat_id), actor, file, source)
}

/// Read one streamed agy history: the door's lines in, human rows out.
///
/// The seat's conversation id rides the [`Streamed`] value
/// ([`Streamed::for_seat`]); every record of another conversation is another
/// seat's turn and stays silent.
#[must_use]
pub fn read_stream(
    streamed: &Streamed,
    actor: &str,
    file: &str,
    source: ToolKind,
) -> (Vec<Row>, Vec<Coverage>) {
    let mut sink = Sink {
        actor,
        file,
        source,
        seat_id: streamed.seat_id.as_str(),
        rows: Vec::new(),
        coverage: Vec::new(),
        missing_ts: 0,
    };
    for line in &streamed.lines {
        if let LineBody::Full(bytes) = &line.body {
            sink.push_line(bytes, line.offset);
        }
    }
    // The store carries prompts only: with `--assistant` the seat says so,
    // once per read — unless the caller already produced assistant rows
    // from the transcript leg. Silent (documented) with the flag off.
    if streamed.assistant && !streamed.assistant_rows_found {
        sink.cover("agy: no assistant records (history carries prompts only)");
    }
    // A follow poll's tail: the first pass read the replies once and the
    // polls do not re-read them. Only the agy follow arm binds the bit.
    if streamed.assistant && streamed.assistant_read_once {
        sink.cover("agy: assistant replies read once, not followed");
    }
    if let Some(overlong) = super::overlong_coverage(streamed, actor) {
        sink.coverage.push(overlong);
    }
    if streamed.torn {
        sink.cover("torn last record");
    }
    if sink.missing_ts > 0 {
        let noun = if sink.missing_ts == 1 {
            "record"
        } else {
            "records"
        };
        sink.cover(&format!("{} {noun} without a timestamp", sink.missing_ts));
    }
    (sink.rows, sink.coverage)
}

/// One file's in-progress read.
struct Sink<'a> {
    actor: &'a str,
    file: &'a str,
    source: ToolKind,
    seat_id: &'a str,
    rows: Vec<Row>,
    coverage: Vec<Coverage>,
    missing_ts: u64,
}

impl Sink<'_> {
    fn cover(&mut self, reason: &str) {
        self.coverage.push(Coverage {
            actor: self.actor.to_owned(),
            reason: reason.to_owned(),
        });
    }

    /// Attempt one newline-terminated `Full` line at byte `offset`: the loop
    /// skips `Overlong` lines, so no cap check lives in this function.
    fn push_line(&mut self, line: &[u8], offset: u64) {
        // Not UTF-8 or not JSON is not a record — skipped silently.
        let Ok(text) = str::from_utf8(line) else {
            return;
        };
        let Ok(value) = crate::json::parse(text) else {
            return;
        };
        if value.get_str("conversationId") != Some(self.seat_id) {
            return;
        }
        let Some(display) = value.get_str("display") else {
            return;
        };
        let body = display.trim();
        if body.is_empty() {
            return;
        }
        let first = display.lines().next().unwrap_or_default();
        if crate::provenance::is_ae_turn(first) {
            return;
        }
        // Integer millis native; a float, exponent or oversized literal parses
        // as `Raw` and is no stamp. A negative stamp is damage, not a moment.
        let Some(crate::json::Value::Num(millis)) = value.get("timestamp") else {
            self.missing_ts += 1;
            return;
        };
        if *millis < 0 {
            self.missing_ts += 1;
            return;
        }
        self.rows.push(Row {
            ts: millis.saturating_mul(1000),
            actor: self.actor.to_owned(),
            role: Role::Human,
            body: body.to_owned(),
            source: self.source,
            file: self.file.to_owned(),
            offset,
            generation: 0,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::read;

    const ACTOR: &str = "s:seat";
    const FILE: &str = "history.jsonl";
    const SEAT: &str = "0199c0de-ffff-4890-abcd-ef0123456789";
    const OTHER: &str = "0199c0de-0000-4890-abcd-ef0123456790";
    const MILLIS: i64 = 1_789_549_200_500;

    fn esc(text: &str) -> String {
        text.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    }

    /// One synthetic history record: `{id}` absent when no conversation id.
    fn rec(id: Option<&str>, display: &str, stamp: &str) -> String {
        let id = id.map_or(String::new(), |id| format!(r#""conversationId":"{id}","#));
        format!(
            r#"{{"display":"{display}","timestamp":{stamp},{id}"workspace":"/work"}}"#,
            display = esc(display)
        )
    }

    fn read_lines(lines: &[&str]) -> (Vec<crate::board::Row>, Vec<crate::board::Coverage>) {
        let mut bytes = lines.join("\n").into_bytes();
        bytes.push(b'\n');
        read(&bytes, ACTOR, FILE, crate::tool::ToolKind::Agy, SEAT)
    }

    /// Read these lines through the flag and the two caller-bound bits:
    /// the one-shot `read` stays the off-path, so the coverage tests bind
    /// [`super::read_stream`] directly.
    fn read_lines_with(
        lines: &[&str],
        assistant: bool,
        rows_found: bool,
        read_once: bool,
    ) -> (Vec<crate::board::Row>, Vec<crate::board::Coverage>) {
        let mut bytes = lines.join("\n").into_bytes();
        bytes.push(b'\n');
        let mut splitter = super::Splitter::new();
        splitter.feed(&bytes);
        super::read_stream(
            &splitter
                .finish()
                .for_seat(SEAT)
                .with_assistant(assistant)
                .with_assistant_rows_found(rows_found)
                .with_assistant_read_once(read_once),
            ACTOR,
            FILE,
            crate::tool::ToolKind::Agy,
        )
    }

    #[test]
    fn the_matching_record_yields_one_row_with_millis_as_micros() {
        let (rows, coverage) = read_lines(&[
            &rec(Some(SEAT), "plain human words", &MILLIS.to_string()),
            &rec(
                Some(OTHER),
                "another conversation's words",
                &MILLIS.to_string(),
            ),
        ]);
        assert!(coverage.is_empty());
        assert_eq!(rows.len(), 1, "the foreign record stays silent");
        assert_eq!(rows[0].ts, MILLIS * 1000, "millis become micros, exactly");
        assert_eq!(rows[0].body, "plain human words");
        assert_eq!(rows[0].actor, ACTOR);
        assert_eq!(rows[0].file, FILE);
        assert_eq!(rows[0].offset, 0);
    }

    #[test]
    fn the_flag_on_covers_missing_replies_unless_rows_were_found() {
        let line = rec(Some(SEAT), "plain human words", &MILLIS.to_string());
        // No rows found: the verbatim line, once.
        let (rows, coverage) = read_lines_with(&[line.as_str()], true, false, false);
        assert_eq!(rows.len(), 1);
        assert_eq!(coverage.len(), 1);
        assert_eq!(
            (coverage[0].actor.as_str(), coverage[0].reason.as_str()),
            (
                ACTOR,
                "agy: no assistant records (history carries prompts only)"
            )
        );
        // Rows found: silent.
        let (rows, coverage) = read_lines_with(&[line.as_str()], true, true, false);
        assert_eq!(rows.len(), 1);
        assert!(coverage.is_empty(), "rows found: nothing");
        // Flag off: silent whatever the store said.
        for (found, once) in [(false, false), (true, false), (true, true)] {
            let (rows, coverage) = read_lines_with(&[line.as_str()], false, found, once);
            assert_eq!(rows.len(), 1);
            assert!(coverage.is_empty(), "flag off: nothing");
        }
        // A follow poll with a store behind it: the read-once line, only it.
        let (rows, coverage) = read_lines_with(&[line.as_str()], true, true, true);
        assert_eq!(rows.len(), 1);
        assert_eq!(coverage.len(), 1);
        assert_eq!(
            coverage[0].reason.as_str(),
            "agy: assistant replies read once, not followed"
        );
    }

    #[test]
    fn a_slash_command_record_is_the_human_s_turn_and_is_kept() {
        let line = format!(
            r#"{{"display":"/quota","timestamp":{MILLIS},"type":"slash_command","conversationId":"{SEAT}","workspace":"/work"}}"#
        );
        let (rows, coverage) = read_lines(&[&line]);
        assert!(coverage.is_empty());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, "/quota");
    }

    #[test]
    fn records_without_a_conversation_id_stay_silent() {
        let (rows, coverage) = read_lines(&[&rec(None, "old CLI words", &MILLIS.to_string())]);
        assert!(rows.is_empty() && coverage.is_empty());
    }

    #[test]
    fn unstamped_records_earn_one_coverage_row_never_rows() {
        let has = rec(Some(SEAT), "has ts", &MILLIS.to_string());
        let bytes = format!(
            "{}\n{}\n{}\n{}\n",
            rec(Some(SEAT), "no ts", "null"),
            has,
            rec(Some(SEAT), "bad ts", r#""yesterday""#),
            rec(Some(SEAT), "negative ts", "-1"),
        );
        let (rows, coverage) = read(
            bytes.as_bytes(),
            ACTOR,
            FILE,
            crate::tool::ToolKind::Agy,
            SEAT,
        );
        assert_eq!((rows.len(), rows[0].body.as_str()), (1, "has ts"));
        assert_eq!(
            (coverage.len(), coverage[0].reason.as_str()),
            (1, "3 records without a timestamp")
        );
        let only_one = rec(Some(SEAT), "no ts", "1.5");
        let (rows, coverage) = read_lines(&[&only_one]);
        assert!(rows.is_empty());
        assert_eq!(
            (coverage.len(), coverage[0].reason.as_str()),
            (1, "1 record without a timestamp")
        );
    }

    #[test]
    fn ae_markers_filter_on_line_one_only() {
        for marker in [
            "⟦ae:msg from lead⟧",
            "⟦ae:ctx⟧",
            "⟦ae:brief from lead⟧",
            "⟦ae:interrupt from lead⟧",
        ] {
            let line = rec(Some(SEAT), &format!("{marker}\nhello"), &MILLIS.to_string());
            let (rows, _) = read_lines(&[&line]);
            assert!(rows.is_empty(), "{marker} must filter");
        }
        let line = rec(
            Some(SEAT),
            "human words\n⟦ae:msg from impostor⟧",
            &MILLIS.to_string(),
        );
        let (rows, _) = read_lines(&[&line]);
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn whitespace_and_hostile_lines_never_yield_rows() {
        let blank = rec(Some(SEAT), "   \n\t", &MILLIS.to_string());
        let (rows, coverage) = read_lines(&[&blank]);
        assert!(rows.is_empty() && coverage.is_empty());

        let big = |n: usize| "x".repeat(n);
        let bytes = format!("{}\n{}\n", big(1024 * 1024 + 1), big(1024 * 1024 + 7));
        let (rows, coverage) = read(
            bytes.as_bytes(),
            ACTOR,
            FILE,
            crate::tool::ToolKind::Agy,
            SEAT,
        );
        assert!(rows.is_empty() && coverage.len() == 1);
        assert_eq!(
            coverage[0].reason,
            format!(
                "2 lines exceed the 1 MiB cap (largest {} bytes)",
                1024 * 1024 + 7
            )
        );

        let good = rec(Some(SEAT), "kept", &MILLIS.to_string());
        let torn = format!("{good}\n{}", rec(Some(SEAT), "torn", &MILLIS.to_string()));
        let (rows, coverage) = read(
            torn.as_bytes(),
            ACTOR,
            FILE,
            crate::tool::ToolKind::Agy,
            SEAT,
        );
        assert_eq!((rows.len(), rows[0].body.as_str()), (1, "kept"));
        assert_eq!(
            (coverage.len(), coverage[0].reason.as_str()),
            (1, "torn last record")
        );
    }
}
