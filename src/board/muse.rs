//! The Muse board reader: session.jsonl bytes in, human rows — and with
//! `--assistant` the model's committed replies — out.
//!
//! PURE: no I/O, no env, no clock. A row is a
//! `payload_type == "runtime.user_intent.accepted"` record; the body joins the
//! `kind == "text"` parts of `payload.model_messages[0].content` in order;
//! trimmed, empties dropped. With `--assistant`, a `payload_type ==
//! "runtime.session"` record whose `payload.event.kind ==
//! "assistant_message_committed"` yields one row with `payload.event.text`
//! whole. Reasoning, `output` chunks and session frames are never read.
//! Verified 2026-09-16 over 51 logs / 198 accepted
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
        assistant: streamed.assistant,
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
    /// `--assistant`: the reply path is live. Off, a session record is
    /// classified and dropped exactly as before the flag existed.
    assistant: bool,
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
        match value.get_str("payload_type") {
            Some("runtime.user_intent.accepted") => self.push_human(&value, offset),
            Some("runtime.session") if self.assistant => self.push_assistant(&value, offset),
            _ => {}
        }
    }

    /// One accepted intent: the human path, markers filtered, empties dropped.
    fn push_human(&mut self, value: &crate::json::Value, offset: u64) {
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

    /// One committed assistant message with `--assistant` on: the event's
    /// whole text, one record one row. Every other `runtime.session` event —
    /// reasoning, `output` chunks, frames — is never read, and the reply path
    /// classifies no markers: a model may legitimately quote one.
    fn push_assistant(&mut self, value: &crate::json::Value, offset: u64) {
        let Some(event) = value
            .get("payload")
            .and_then(|payload| payload.get("event"))
        else {
            return;
        };
        if event.get_str("kind") != Some("assistant_message_committed") {
            return;
        }
        let Some(text) = event.get_str("text") else {
            return;
        };
        let Some(crate::json::Value::Num(micros)) = value.get("recorded_at") else {
            self.missing_ts += 1;
            return;
        };
        let body = text.trim().to_owned();
        if body.is_empty() {
            return;
        }
        self.rows.push(Row {
            ts: *micros,
            actor: self.actor.to_owned(),
            role: Role::Assistant,
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
    // One accepted turn: {c} is the content parts, {m} the stamp. The BARE
    // stub refill proves the body comes from the model text, never the refill.
    const ACC: &str = r#"{"recorded_at":{m},"payload_type":"runtime.user_intent.accepted","payload":{"refill_blocks":[{"kind":"text","text":"stub"}],"model_messages":[{"content":[{c}]}]}}"#;
    const TWIN: &str = r#"{"recorded_at":1789565338436454,"payload_type":"runtime.user_intent.materialized","payload":{"intent_id":"i","outcome":"ok"}}"#;

    fn acc(text: &str, micros: &str) -> String {
        let esc = text.replace('\n', "\\n");
        ACC.replace("{m}", micros)
            .replace("{c}", &format!("{{\"kind\":\"text\",\"text\":\"{esc}\"}}"))
    }

    fn read_lines(lines: &[&str]) -> (Vec<crate::board::Row>, Vec<crate::board::Coverage>) {
        let mut bytes = lines.join("\n").into_bytes();
        bytes.push(b'\n');
        read(&bytes, ACTOR, FILE, crate::tool::ToolKind::Muse)
    }

    /// Read these lines through the flag: the one-shot `read` stays the
    /// off-path, so the reply tests bind [`super::read_stream`] directly.
    fn read_lines_with(
        lines: &[&str],
        assistant: bool,
    ) -> (Vec<crate::board::Row>, Vec<crate::board::Coverage>) {
        let mut bytes = lines.join("\n").into_bytes();
        bytes.push(b'\n');
        let mut splitter = super::Splitter::new();
        splitter.feed(&bytes);
        super::read_stream(
            &splitter.finish().with_assistant(assistant),
            ACTOR,
            FILE,
            crate::tool::ToolKind::Muse,
        )
    }

    /// One synthetic `runtime.session` event record; `body` is the event.
    fn event(body: &str) -> String {
        format!(
            r#"{{"recorded_at":{MICROS},"payload_type":"runtime.session","payload":{{"event":{body}}}}}"#
        )
    }

    /// One committed assistant message; `micros` raw (non-numeric unstamps).
    fn comm(text: &str, micros: &str) -> String {
        format!(
            r#"{{"recorded_at":{micros},"payload_type":"runtime.session","payload":{{"event":{{"kind":"assistant_message_committed","message_id":"m","response_id":"r","provider_item_id":"p","text":"{text}"}}}}}}"#
        )
    }

    #[test]
    fn the_accepted_turn_yields_one_row_with_native_micros() {
        let (rows, coverage) = read_lines(&[&acc("plain human words", &MICROS.to_string())]);
        assert!(coverage.is_empty() && rows.len() == 1 && rows[0].ts == MICROS);
        assert_eq!(rows[0].body, "plain human words");
        assert_eq!(rows[0].offset, 0);
        assert_eq!(rows[0].actor, ACTOR);
        assert_eq!(rows[0].file, FILE);
    }

    #[test]
    fn the_materialized_twin_yields_no_second_row() {
        let turn = acc("human words", &MICROS.to_string());
        let (rows, coverage) = read_lines(&[&turn, TWIN]);
        assert!(coverage.is_empty() && rows.len() == 1 && rows[0].offset == 0);
        // Either order: the pair yields ONE row at the accepted offset.
        let (rows, _) = read_lines(&[TWIN, &turn]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, "human words");
        assert_eq!(rows[0].offset, TWIN.len() as u64 + 1);
    }

    #[test]
    fn model_stream_and_frame_lines_stay_silent() {
        let session = r#"{"recorded_at":1789565338436454,"payload_type":"runtime.session","payload":{"event":{"role":"user","text":"model words"}}}"#;
        let (rows, coverage) = read_lines(&[
            session,
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
            let (rows, _) = read_lines(&[&acc(&format!("{marker}\nhello"), &MICROS.to_string())]);
            assert!(rows.is_empty(), "{marker} must filter");
        }
        let mid = acc("mid\n⟦ae:msg from impostor⟧", &MICROS.to_string());
        let (rows, _) = read_lines(&[&mid]);
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn text_parts_join_in_order_and_image_parts_skip() {
        let parts = r#"{"kind":"text","text":"first "},{"kind":"image","url":"x"},{"kind":"text","text":"second"}"#;
        let ms = MICROS.to_string();
        let line = ACC.replace("{m}", &ms).replace("{c}", parts);
        let (rows, coverage) = read_lines(&[&line]);
        assert!(coverage.is_empty() && rows.len() == 1);
        assert_eq!(rows[0].body, "first second");
    }

    #[test]
    fn hostile_lines_earn_coverage_never_rows() {
        let big = |n: usize| "x".repeat(n);
        let bytes = format!("{}\n{}\n", big(1024 * 1024 + 1), big(1024 * 1024 + 7));
        let (rows, coverage) = read(bytes.as_bytes(), ACTOR, FILE, crate::tool::ToolKind::Muse);
        assert!(rows.is_empty() && coverage.len() == 1);
        let want = format!(
            "2 lines exceed the 1 MiB cap (largest {} bytes)",
            1024 * 1024 + 7
        );
        assert_eq!(coverage[0].reason, want);
        let good = acc("kept", &MICROS.to_string());
        let torn = format!("{good}\n{}", acc("torn", &MICROS.to_string()));
        let (rows, coverage) = read(torn.as_bytes(), ACTOR, FILE, crate::tool::ToolKind::Muse);
        assert_eq!((rows.len(), rows[0].body.as_str()), (1, "kept"));
        assert_eq!(
            (coverage.len(), coverage[0].reason.as_str()),
            (1, "torn last record")
        );
        let no = acc("no ts", "null");
        let has = acc("has ts", &MICROS.to_string());
        let bad = acc("bad ts", r#""yesterday""#);
        let bytes = format!("{no}\n{has}\n{bad}\n");
        let (rows, coverage) = read(bytes.as_bytes(), ACTOR, FILE, crate::tool::ToolKind::Muse);
        assert_eq!((rows.len(), rows[0].body.as_str()), (1, "has ts"));
        assert_eq!(
            (coverage.len(), coverage[0].reason.as_str()),
            (1, "2 records without a timestamp")
        );
    }

    #[test]
    fn a_committed_message_is_one_row_only_with_the_flag() {
        let human = acc("plain human words", &MICROS.to_string());
        let reply = comm("synthetic reply", &MICROS.to_string());
        let lines = [human.as_str(), reply.as_str()];
        let (rows, coverage) = read_lines_with(&lines, true);
        assert!(coverage.is_empty() && rows.len() == 2);
        assert_eq!(rows[1].body, "synthetic reply");
        assert_eq!(rows[1].ts, MICROS);
        assert_eq!(rows[1].offset, human.len() as u64 + 1);
        assert_eq!(rows[1].role, crate::board::Role::Assistant);
        let (rows, _) = read_lines_with(&lines, false);
        assert_eq!(rows.len(), 1, "flag off: the human row alone");
    }

    #[test]
    fn reasoning_output_and_frame_events_stay_silent() {
        let lines = [
            event(r#"{"kind":"reasoning_summary_committed","text":"hidden"}"#),
            event(r#"{"kind":"reasoning_committed","text":"hidden"}"#),
            event(r#"{"kind":"output","chunk":"hidden"}"#),
            event(r#"{"role":"user","text":"model words"}"#),
        ];
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let (rows, coverage) = read_lines_with(&refs, true);
        assert!(rows.is_empty() && coverage.is_empty());
    }

    #[test]
    fn an_empty_reply_drops_and_an_unstamped_one_counts() {
        let blank = comm("   ", &MICROS.to_string());
        let (rows, coverage) = read_lines_with(&[blank.as_str()], true);
        assert!(rows.is_empty() && coverage.is_empty(), "empty drops");
        let bare = comm("no ts", "null");
        let (rows, coverage) = read_lines_with(&[bare.as_str()], true);
        assert!(rows.is_empty());
        assert_eq!(coverage[0].reason, "1 record without a timestamp");
    }
}
