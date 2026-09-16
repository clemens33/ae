//! The Grok board reader: updates.jsonl bytes in, human rows out.
//!
//! PURE: no I/O, no env, no clock. A row is a `method == "session/update"`
//! record with `params.update.sessionUpdate == "user_message_chunk"` and
//! `params.update.content.type == "text"`; the text is `content.text`.
//! Trimmed, empties dropped. Newline-terminated lines only.
//!
//! Verified 2026-09-16 over 17 session dirs (shapes only): every
//! `user_message_chunk` is its own row — NO chunk joining (73 user chunks vs
//! 72 turns: a user turn is ONE chunk in practice). Time prefers
//! `params.update._meta.agentTimestampMs` (millis to micros), else the
//! top-level `timestamp` secs; ae turns filter via `is_ae_turn` on line 1
//! (ae's grok context rides positional `[PROMPT]` and carries the `ctx`
//! marker). No plumbing prefixes known — none filtered, none invented.

use super::{LineBody, Splitter, Streamed};
use crate::board::{Coverage, Role, Row};
use crate::tool::ToolKind;

/// Read one Grok updates file from whole bytes: one feed through the ONE
/// splitter, then read the stream. A thin wrapper, so the fuzz target covers
/// the code the door runs.
#[must_use]
pub fn read(bytes: &[u8], actor: &str, file: &str, source: ToolKind) -> (Vec<Row>, Vec<Coverage>) {
    let mut splitter = Splitter::new();
    splitter.feed(bytes);
    read_stream(&splitter.finish(), actor, file, source)
}

/// Read one streamed Grok updates file: the door's lines in, human rows out.
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
        if value.get_str("method") != Some("session/update") {
            return;
        }
        let Some(params) = value.get("params") else {
            return;
        };
        let Some(update) = params.get("update") else {
            return;
        };
        if update.get_str("sessionUpdate") != Some("user_message_chunk") {
            return;
        }
        let Some(content) = update.get("content") else {
            return;
        };
        if content.get_str("type") != Some("text") {
            return;
        };
        let Some(text) = content.get_str("text") else {
            return;
        };
        let first = text.lines().next().unwrap_or_default();
        if crate::provenance::is_ae_turn(first) {
            return;
        }
        let Some(ts) = grok_ts(&value, update) else {
            self.missing_ts += 1;
            return;
        };
        let body = text.trim().to_owned();
        if body.is_empty() {
            return;
        }
        self.rows.push(Row {
            ts,
            actor: self.actor.to_owned(),
            role: Role::Human,
            body,
            source: self.source,
            file: self.file.to_owned(),
            offset,
        });
    }
}

/// A Grok record's epoch micros: `_meta.agentTimestampMs` (millis) preferred,
/// else the top-level `timestamp` secs. Non-numeric or absent is no timestamp.
fn grok_ts(value: &crate::json::Value, update: &crate::json::Value) -> Option<i64> {
    if let Some(meta) = update.get("_meta")
        && let Some(crate::json::Value::Num(millis)) = meta.get("agentTimestampMs")
    {
        return Some(millis.saturating_mul(1000));
    }
    if let Some(crate::json::Value::Num(secs)) = value.get("timestamp") {
        return Some(secs.saturating_mul(1_000_000));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::read;

    const ACTOR: &str = "s:seat";
    const FILE: &str = "updates.jsonl";
    const MILLIS: i64 = 1_789_549_200_500;
    const SECS: i64 = 1_789_549_200;

    fn esc(text: &str) -> String {
        text.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    }

    /// A user chunk stamped both ways: millis in `_meta`, secs on top.
    fn user(text: &str) -> String {
        format!(
            r#"{{"timestamp":{SECS},"method":"session/update","params":{{"sessionId":"s","update":{{"sessionUpdate":"user_message_chunk","_meta":{{"agentTimestampMs":{MILLIS}}},"content":{{"type":"text","text":"{text}"}}}}}}}}"#,
            text = esc(text)
        )
    }

    /// A text chunk of any kind: secs on top, no `_meta`.
    fn stamped(kind: &str, text: &str) -> String {
        format!(
            r#"{{"timestamp":{SECS},"method":"session/update","params":{{"sessionId":"s","update":{{"sessionUpdate":"{kind}","content":{{"type":"text","text":"{text}"}}}}}}}}"#,
            text = esc(text)
        )
    }

    /// A text chunk of any kind with no timestamp at all.
    fn unstamped(kind: &str, text: &str) -> String {
        format!(
            r#"{{"method":"session/update","params":{{"sessionId":"s","update":{{"sessionUpdate":"{kind}","content":{{"type":"text","text":"{text}"}}}}}}}}"#,
            text = esc(text)
        )
    }

    fn read_lines(lines: &[&str]) -> (Vec<crate::board::Row>, Vec<crate::board::Coverage>) {
        let mut bytes = lines.join("\n").into_bytes();
        bytes.push(b'\n');
        read(&bytes, ACTOR, FILE, crate::tool::ToolKind::Grok)
    }

    #[test]
    fn the_user_chunk_yields_one_row_at_millis_precision() {
        let (rows, coverage) = read_lines(&[&user("plain human words")]);
        assert!(coverage.is_empty());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, "plain human words");
        assert_eq!(rows[0].ts, MILLIS * 1000);
        assert_eq!(rows[0].actor, ACTOR);
        assert_eq!(rows[0].file, FILE);
        assert_eq!(rows[0].offset, 0);
    }

    #[test]
    fn without_meta_millis_the_secs_field_decides() {
        let (rows, coverage) = read_lines(&[&stamped("user_message_chunk", "secs words")]);
        assert!(coverage.is_empty());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].ts, SECS * 1_000_000);
    }

    #[test]
    fn agent_thought_tool_and_turn_records_stay_silent() {
        for kind in [
            "agent_message_chunk",
            "agent_thought_chunk",
            "tool_call",
            "tool_call_update",
            "turn_completed",
            "task_completed",
            "task_backgrounded",
        ] {
            let (rows, coverage) = read_lines(&[&stamped(kind, "not human")]);
            assert!(rows.is_empty() && coverage.is_empty(), "{kind}");
        }
        for line in [
            format!(
                r#"{{"timestamp":{SECS},"method":"session/update","params":{{"sessionId":"s","update":{{"sessionUpdate":"user_message_chunk","content":{{"type":"image","url":"x"}}}}}}}}"#
            ),
            r#"{"timestamp":1789549200,"method":"session/list","params":{}}"#.to_owned(),
        ] {
            let (rows, coverage) = read_lines(&[&line]);
            assert!(rows.is_empty() && coverage.is_empty(), "{line}");
        }
    }

    #[test]
    fn ae_markers_filter_on_line_one_only() {
        for marker in [
            "⟦ae:msg from lead⟧",
            "⟦ae:ctx⟧",
            "⟦ae:brief from lead⟧",
            "⟦ae:interrupt from lead⟧",
        ] {
            let (rows, _) = read_lines(&[&user(&format!("{marker}\nhello"))]);
            assert!(rows.is_empty(), "{marker} must filter");
        }
        let (rows, _) = read_lines(&[&user("human words\n⟦ae:msg from impostor⟧")]);
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn overlong_lines_aggregate_to_one_coverage_row() {
        let bytes = format!(
            "{}\n{}\n",
            "x".repeat(1024 * 1024 + 1),
            "y".repeat(1024 * 1024 + 7)
        );
        let (rows, coverage) = read(bytes.as_bytes(), ACTOR, FILE, crate::tool::ToolKind::Grok);
        assert!(rows.is_empty());
        assert_eq!(coverage.len(), 1);
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
        let good = user("kept");
        let bytes = format!("{good}\n{}", user("torn"));
        let (rows, coverage) = read(bytes.as_bytes(), ACTOR, FILE, crate::tool::ToolKind::Grok);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, "kept");
        assert_eq!(coverage.len(), 1);
        assert_eq!(coverage[0].reason, "torn last record");
        let bytes = format!(
            "{}\n{}\n{}\n",
            unstamped("user_message_chunk", "no ts"),
            user("has ts"),
            r#"{"timestamp":"not-a-time","method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"bad ts"}}}}"#,
        );
        let (rows, coverage) = read(bytes.as_bytes(), ACTOR, FILE, crate::tool::ToolKind::Grok);
        assert_eq!(rows.len(), 1);
        assert_eq!(coverage.len(), 1);
        assert_eq!(coverage[0].reason, "2 records without a timestamp");
    }
}
