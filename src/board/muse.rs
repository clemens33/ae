//! The Muse board reader: session.jsonl bytes in, human rows out.
//!
//! PURE: no I/O, no env, no clock. A row is a
//! `payload_type == "runtime.user_intent.accepted"` record; the body joins the
//! `kind == "text"` parts of `payload.model_messages[0].content` in order;
//! trimmed, empties dropped. Verified 2026-09-16 over 51 logs / 198 accepted
//! records (shapes only): entry [0] is the turn (`content` alone, NO role
//! field), `recorded_at` int micros native, `payload.text` on no record.
//! `refill_blocks` is NEVER read — a display stub (27 chars, 79/198) unmarked
//! on ae turns (61/198), so it would truncate turns and bypass the ae filter.
//! The `materialized` twin, the `runtime.session` stream, `retained_frame` and
//! `record_json` lines stay silent. `semantic_kind` (`{kind: "chat"}`) and
//! `surface` (`"main"`) are single values: no rule. `is_ae_turn` on line 1.

use super::{LineBody, Splitter, Streamed};
use crate::board::{Coverage, Role, Row};
use crate::tool::ToolKind;

/// Whole-bytes wrapper over the ONE splitter, so the fuzz target covers the
/// code the door runs.
#[must_use]
pub fn read(bytes: &[u8], actor: &str, file: &str, source: ToolKind) -> (Vec<Row>, Vec<Coverage>) {
    let mut splitter = Splitter::new();
    splitter.feed(bytes);
    read_stream(&splitter.finish(), actor, file, source)
}

/// Read one streamed Muse session file: the door's lines in, human rows out.
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
        rows: Vec::new(),
        coverage: Vec::new(),
        missing_ts: 0,
    };
    for line in &streamed.lines {
        if let LineBody::Full(bytes) = &line.body {
            sink.push_line(bytes, line.offset);
        }
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
        if value.get_str("payload_type") != Some("runtime.user_intent.accepted") {
            return;
        };
        let Some(payload) = value.get("payload") else {
            return;
        };
        let Some(crate::json::Value::Arr(messages)) = payload.get("model_messages") else {
            return;
        };
        // No role field on this store, so entry [0]: the probe shape above.
        let Some(turn) = messages.first() else {
            return;
        };
        let Some(crate::json::Value::Arr(parts)) = turn.get("content") else {
            return;
        };
        let mut joined = String::new();
        for part in parts {
            if part.get_str("kind") == Some("text")
                && let Some(text) = part.get_str("text")
            {
                joined.push_str(text);
            }
        }
        let first = joined.lines().next().unwrap_or_default();
        if crate::provenance::is_ae_turn(first) {
            return;
        }
        let Some(crate::json::Value::Num(micros)) = value.get("recorded_at") else {
            self.missing_ts += 1;
            return;
        };
        let body = joined.trim().to_owned();
        if body.is_empty() {
            return;
        }
        self.rows.push(Row {
            ts: *micros,
            actor: self.actor.to_owned(),
            role: Role::Human,
            body,
            source: self.source,
            file: self.file.to_owned(),
            offset,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::read;

    const ACTOR: &str = "s:seat";
    const FILE: &str = "session.jsonl";
    const MICROS: i64 = 1_789_565_338_436_454;

    fn esc(text: &str) -> String {
        text.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    }

    fn txt(text: &str) -> String {
        format!(r#"{{"kind":"text","text":"{}"}}"#, esc(text))
    }

    /// One accepted turn: these content parts plus a BARE stub refill, so
    /// every test proves the body comes from the model text, never the refill.
    fn acc(content: &str, micros: &str) -> String {
        format!(
            r#"{{"recorded_at":{micros},"payload_type":"runtime.user_intent.accepted","payload":{{"refill_blocks":[{{"kind":"text","text":"stub"}}],"model_messages":[{{"content":[{content}]}}]}}}}"#
        )
    }

    /// The accepted record's materialized twin: same intent, no text of its own.
    fn twin() -> String {
        format!(
            r#"{{"recorded_at":{MICROS},"payload_type":"runtime.user_intent.materialized","payload":{{"intent_id":"i","outcome":"ok"}}}}"#
        )
    }

    fn read_lines(lines: &[&str]) -> (Vec<crate::board::Row>, Vec<crate::board::Coverage>) {
        let mut bytes = lines.join("\n").into_bytes();
        bytes.push(b'\n');
        read(&bytes, ACTOR, FILE, crate::tool::ToolKind::Muse)
    }

    #[test]
    fn the_accepted_turn_yields_one_row_with_native_micros() {
        let (rows, coverage) = read_lines(&[&acc(&txt("plain human words"), &MICROS.to_string())]);
        assert!(coverage.is_empty());
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].body.as_str(), rows[0].ts),
            ("plain human words", MICROS)
        );
        assert_eq!(
            (
                rows[0].actor.as_str(),
                rows[0].file.as_str(),
                rows[0].offset
            ),
            (ACTOR, FILE, 0)
        );
    }

    #[test]
    fn the_materialized_twin_yields_no_second_row() {
        let turn = acc(&txt("human words"), &MICROS.to_string());
        let (rows, coverage) = read_lines(&[&turn, &twin()]);
        assert!(coverage.is_empty() && rows.len() == 1 && rows[0].offset == 0);
        // Either order: the pair yields ONE row at the accepted offset.
        let (rows, _) = read_lines(&[&twin(), &turn]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, "human words");
        assert_eq!(rows[0].offset, twin().len() as u64 + 1);
    }

    #[test]
    fn model_stream_and_frame_lines_stay_silent() {
        let session = format!(
            r#"{{"recorded_at":{MICROS},"payload_type":"runtime.session","payload":{{"event":{{"role":"user","text":"model words"}}}}}}"#
        );
        let (rows, coverage) = read_lines(&[
            &session,
            r#"{"retained_frame":{"id":"r"}}"#,
            r#"{"record_json":"{\"a\":1}"}"#,
        ]);
        assert!(rows.is_empty() && coverage.is_empty());
    }

    #[test]
    fn ae_markers_filter_on_line_one_only() {
        for marker in [
            "⟦ae:msg from lead⟧",
            "⟦ae:ctx⟧",
            "⟦ae:brief from lead⟧",
            "⟦ae:interrupt from lead⟧",
        ] {
            let line = acc(&txt(&format!("{marker}\nhello")), &MICROS.to_string());
            let (rows, _) = read_lines(&[&line]);
            assert!(rows.is_empty(), "{marker} must filter");
        }
        let line = acc(
            &txt("human words\n⟦ae:msg from impostor⟧"),
            &MICROS.to_string(),
        );
        let (rows, _) = read_lines(&[&line]);
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn text_parts_join_in_order_and_image_parts_skip() {
        let line = acc(
            r#"{"kind":"text","text":"first "},{"kind":"image","url":"x"},{"kind":"text","text":"second"}"#,
            &MICROS.to_string(),
        );
        let (rows, coverage) = read_lines(&[&line]);
        assert!(coverage.is_empty());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, "first second");
    }

    #[test]
    fn overlong_lines_aggregate_to_one_coverage_row() {
        let bytes = format!(
            "{}\n{}\n",
            "x".repeat(1024 * 1024 + 1),
            "y".repeat(1024 * 1024 + 7)
        );
        let (rows, coverage) = read(bytes.as_bytes(), ACTOR, FILE, crate::tool::ToolKind::Muse);
        assert!(rows.is_empty() && coverage.len() == 1);
        assert_eq!(
            coverage[0].reason,
            format!(
                "2 lines exceed the 1 MiB cap (largest {} bytes)",
                1024 * 1024 + 7
            )
        );
    }

    #[test]
    fn torn_and_unstamped_records_earn_coverage_never_rows() {
        let good = acc(&txt("kept"), &MICROS.to_string());
        let (rows, coverage) = read(
            format!("{good}\n{}", acc(&txt("torn"), &MICROS.to_string())).as_bytes(),
            ACTOR,
            FILE,
            crate::tool::ToolKind::Muse,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, "kept");
        assert_eq!(
            (coverage.len(), coverage[0].reason.as_str()),
            (1, "torn last record")
        );
        let bytes = format!(
            "{}\n{}\n{}\n",
            acc(&txt("no ts"), "null"),
            acc(&txt("has ts"), &MICROS.to_string()),
            acc(&txt("bad ts"), r#""yesterday""#)
        );
        let (rows, coverage) = read(bytes.as_bytes(), ACTOR, FILE, crate::tool::ToolKind::Muse);
        assert_eq!((rows.len(), rows[0].body.as_str()), (1, "has ts"));
        assert_eq!(
            (coverage.len(), coverage[0].reason.as_str()),
            (1, "2 records without a timestamp")
        );
    }
}
