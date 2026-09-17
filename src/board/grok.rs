//! The Grok board reader: updates.jsonl bytes in, human rows — and with
//! `--assistant` the model's joined replies — out.
//!
//! PURE: no I/O, no env, no clock. A row is a `method == "session/update"`
//! record with `params.update.sessionUpdate == "user_message_chunk"` and
//! `params.update.content.type == "text"`; the text is `content.text`.
//! Trimmed, empties dropped. Newline-terminated lines only.
//!
//! With `--assistant`, `agent_message_chunk` records are DELTAS: their
//! `content.text` accumulates in file order across ONE turn — the next
//! `user_message_chunk`, a `turn_completed`, or EOF ends it — and the joined
//! body is one row at the run's FIRST chunk (its ts and offset). Thought
//! chunks, tool calls and every other kind are never read. NAMED LIMITATION:
//! a message boundary inside one turn fuses, the store carries no separator.
//!
//! Verified 2026-09-16 over 17 session dirs (shapes only): every chunk is
//! its own row — NO joining (73 user chunks vs 72 turns). Time prefers
//! `_meta.agentTimestampMs` millis, else top-level `timestamp` secs; ae turns
//! filter via `is_ae_turn` on line 1. No plumbing prefixes: none known.

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

/// Read one streamed Grok updates file: the door's lines in, human rows —
/// and with `--assistant` the model's joined replies — out.
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
        run: None,
        rows: Vec::new(),
        coverage: Vec::new(),
        missing_ts: 0,
    };
    for line in &streamed.lines {
        if let LineBody::Full(bytes) = &line.body {
            sink.push_line(bytes, line.offset);
        }
    }
    // EOF ends the last turn: the one-shot prints the open run as-is. The
    // follow's HOLD is the consumer's (`super::hold_at`), never the reader's.
    sink.flush_run();
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
    /// `--assistant`: the run path is live. Off, an agent chunk is
    /// classified and dropped exactly as before the flag existed.
    assistant: bool,
    /// The turn's open assistant run, joined across chunk lines.
    run: Option<Run>,
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
        let Some((kind, value)) = split_line(line) else {
            return;
        };
        match kind {
            // Every user chunk is a turn boundary, even one the human path
            // then skips as non-text: the run it ends still closes here.
            Turn::Human => {
                self.flush_run();
                self.push_human(&value, offset);
            }
            Turn::Agent if self.assistant => self.push_agent(&value, offset),
            Turn::Close => self.flush_run(),
            _ => {}
        }
    }

    /// One `user_message_chunk`: the human path, ae markers filtered,
    /// empties dropped. Stamps come from `grok_ts`, shared with the run path.
    fn push_human(&mut self, value: &crate::json::Value, offset: u64) {
        // `split_line` proved the method, the params and the kind; the
        // content gate stays here, where the body is read.
        let Some(update) = value.get("params").and_then(|params| params.get("update")) else {
            return;
        };
        let Some(content) = update.get("content") else {
            return;
        };
        if content.get_str("type") != Some("text") {
            return;
        }
        let Some(text) = content.get_str("text") else {
            return;
        };
        let first = text.lines().next().unwrap_or_default();
        if crate::provenance::is_ae_turn(first) {
            return;
        }
        let Some(ts) = grok_ts(value, update) else {
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

    /// One agent TEXT chunk with `--assistant` on: join its text onto the open
    /// run, or open one bound to this chunk's ts and offset. An unstamped
    /// chunk counts into the timestamp coverage and contributes nothing — a
    /// run whose first chunk has no stamp never opens. No marker
    /// classification: a reply may legitimately quote one.
    fn push_agent(&mut self, value: &crate::json::Value, offset: u64) {
        let Some(update) = value.get("params").and_then(|params| params.get("update")) else {
            return;
        };
        let Some(text) = update
            .get("content")
            .and_then(|content| content.get_str("text"))
        else {
            return;
        };
        let Some(ts) = grok_ts(value, update) else {
            self.missing_ts += 1;
            return;
        };
        match self.run.as_mut() {
            Some(run) => run.body.push_str(text),
            None => {
                self.run = Some(Run {
                    ts,
                    offset,
                    body: text.to_owned(),
                });
            }
        }
    }

    /// Close the open run, if any: one row at the FIRST chunk's ts and
    /// offset, the joined body trimmed, empties dropped.
    fn flush_run(&mut self) {
        let Some(run) = self.run.take() else {
            return;
        };
        let body = run.body.trim().to_owned();
        if body.is_empty() {
            return;
        }
        self.rows.push(Row {
            ts: run.ts,
            actor: self.actor.to_owned(),
            role: Role::Assistant,
            body,
            source: self.source,
            file: self.file.to_owned(),
            offset: run.offset,
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

/// One line's role in the turn stream: the reader and the follow hold share
/// this one classifier, so a run opens and closes the same way in both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Turn {
    /// A `user_message_chunk`: a human turn, and a run boundary.
    Human,
    /// An `agent_message_chunk` with text content: joins the open run.
    Agent,
    /// A `turn_completed`: closes the open run.
    Close,
    /// Not a record, or a kind neither path reads.
    Other,
}

/// Parse one line into its turn role plus the parsed record. `None` is not a
/// `session/update` record at all — skipped silently by every consumer.
fn split_line(line: &[u8]) -> Option<(Turn, crate::json::Value)> {
    let text = str::from_utf8(line).ok()?;
    let value = crate::json::parse(text).ok()?;
    if value.get_str("method") != Some("session/update") {
        return None;
    }
    let update = value.get("params")?.get("update")?;
    let kind = match update.get_str("sessionUpdate") {
        Some("user_message_chunk") => Turn::Human,
        Some("turn_completed") => Turn::Close,
        Some("agent_message_chunk") => {
            let text_chunk = update.get("content").is_some_and(|content| {
                content.get_str("type") == Some("text") && content.get_str("text").is_some()
            });
            if text_chunk { Turn::Agent } else { Turn::Other }
        }
        _ => Turn::Other,
    };
    Some((kind, value))
}

/// One turn's in-progress assistant run: the FIRST chunk's identity plus the
/// text joined so far.
struct Run {
    ts: i64,
    offset: u64,
    body: String,
}

/// The open run's first-chunk absolute offset, when the stream ends mid-turn:
/// agent text chunks after the last boundary with no `turn_completed` yet.
/// The follow's commit point holds here so the next poll re-reads from there
/// and joins the whole turn. PURE — and conservative: the reader drops an
/// unstamped first chunk while this holds at it, so a hostile run re-reads
/// rather than fragments.
#[must_use]
pub fn open_run_start(streamed: &Streamed) -> Option<u64> {
    let mut open: Option<u64> = None;
    for line in &streamed.lines {
        let LineBody::Full(bytes) = &line.body else {
            continue;
        };
        match split_line(bytes).map(|(kind, _)| kind) {
            Some(Turn::Agent) => {
                open.get_or_insert(line.offset);
            }
            Some(Turn::Human | Turn::Close) => open = None,
            _ => {}
        }
    }
    open
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

    /// A text chunk of any kind: `top` carries the secs prefix, or nothing.
    fn rec(kind: &str, text: &str, top: &str) -> String {
        format!(
            r#"{{{top}"method":"session/update","params":{{"sessionId":"s","update":{{"sessionUpdate":"{kind}","content":{{"type":"text","text":"{text}"}}}}}}}}"#,
            text = esc(text)
        )
    }

    fn read_lines(lines: &[&str]) -> (Vec<crate::board::Row>, Vec<crate::board::Coverage>) {
        let mut bytes = lines.join("\n").into_bytes();
        bytes.push(b'\n');
        read(&bytes, ACTOR, FILE, crate::tool::ToolKind::Grok)
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
            crate::tool::ToolKind::Grok,
        )
    }

    /// A stamped agent text chunk; `top` empty leaves it unstamped.
    fn agent(text: &str, top: &str) -> String {
        rec("agent_message_chunk", text, top)
    }

    fn stamped() -> String {
        format!(r#""timestamp":{SECS},"#)
    }

    /// A boundary or inert record of `kind`: no content, never a row.
    fn bare(kind: &str) -> String {
        format!(
            r#"{{"timestamp":{SECS},"method":"session/update","params":{{"sessionId":"s","update":{{"sessionUpdate":"{kind}"}}}}}}"#
        )
    }

    #[test]
    fn the_user_chunk_yields_one_row_millis_first_secs_fallback() {
        let (rows, coverage) = read_lines(&[&user("plain human words")]);
        assert!(coverage.is_empty());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, "plain human words");
        assert_eq!(rows[0].ts, MILLIS * 1000);
        assert_eq!(rows[0].actor, ACTOR);
        assert_eq!(rows[0].file, FILE);
        assert_eq!(rows[0].offset, 0);
        let top = format!(r#""timestamp":{SECS},"#);
        let (rows, coverage) = read_lines(&[&rec("user_message_chunk", "secs words", &top)]);
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
            let top = format!(r#""timestamp":{SECS},"#);
            let (rows, coverage) = read_lines(&[&rec(kind, "not human", &top)]);
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
            rec("user_message_chunk", "no ts", ""),
            user("has ts"),
            r#"{"timestamp":"not-a-time","method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"bad ts"}}}}"#,
        );
        let (rows, coverage) = read(bytes.as_bytes(), ACTOR, FILE, crate::tool::ToolKind::Grok);
        assert_eq!(rows.len(), 1);
        assert_eq!(coverage.len(), 1);
        assert_eq!(coverage[0].reason, "2 records without a timestamp");
    }

    #[test]
    fn agent_chunks_join_to_one_row_at_the_first_chunk() {
        let top = stamped();
        let human = user("human words");
        let reply = agent("synthetic reply", &top);
        let close = bare("turn_completed");
        let lines = [human.as_str(), reply.as_str(), close.as_str()];
        let (rows, coverage) = read_lines_with(&lines, true);
        assert!(coverage.is_empty() && rows.len() == 2);
        assert_eq!(rows[1].body, "synthetic reply");
        assert_eq!(rows[1].ts, SECS * 1_000_000);
        assert_eq!(rows[1].offset, human.len() as u64 + 1);
        assert_eq!(rows[1].role, crate::board::Role::Assistant);
        // The same turn split by tool and thought records still joins to one
        // row — and a thought rendered as text would corrupt this body.
        let first = agent("syn", &top);
        let tool = bare("tool_call");
        let thought = rec("agent_thought_chunk", "hidden", &top);
        let second = agent("thetic", &top);
        let lines = [
            first.as_str(),
            tool.as_str(),
            thought.as_str(),
            second.as_str(),
            close.as_str(),
        ];
        let (rows, coverage) = read_lines_with(&lines, true);
        assert!(coverage.is_empty() && rows.len() == 1);
        assert_eq!(rows[0].body, "synthetic");
        assert_eq!(rows[0].offset, 0, "the FIRST chunk binds");
    }

    #[test]
    fn a_user_chunk_between_turns_never_merges() {
        let top = stamped();
        let one = agent("first", &top);
        let human = user("human words");
        let two = agent("second", &top);
        let close = bare("turn_completed");
        let lines = [one.as_str(), human.as_str(), two.as_str(), close.as_str()];
        let (rows, coverage) = read_lines_with(&lines, true);
        assert!(coverage.is_empty());
        let bodies: Vec<&str> = rows.iter().map(|row| row.body.as_str()).collect();
        assert_eq!(bodies, ["first", "human words", "second"]);
        assert_eq!(rows[2].offset, (one.len() + 1 + human.len() + 1) as u64);
        let (rows, _) = read_lines_with(&lines, false);
        assert_eq!(rows.len(), 1, "flag off: the human row alone");
        assert_eq!(rows[0].body, "human words");
    }

    #[test]
    fn an_open_run_emits_at_eof_while_open_run_start_names_its_first_chunk() {
        // Reader-level: the one-shot prints the open run as-is; the HOLD is
        // the follow consumer's, which the follow test pins.
        let top = stamped();
        let human = user("human words");
        let first = agent("syn", &top);
        let second = agent("thetic", &top);
        let mut bytes = [human.as_str(), first.as_str(), second.as_str()]
            .join("\n")
            .into_bytes();
        bytes.push(b'\n');
        let mut splitter = super::Splitter::new();
        splitter.feed(&bytes);
        let streamed = splitter.finish().with_assistant(true);
        let at = (human.len() + 1) as u64;
        assert_eq!(super::open_run_start(&streamed), Some(at));
        let (rows, _) = super::read_stream(&streamed, ACTOR, FILE, crate::tool::ToolKind::Grok);
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[1].body.as_str(), rows[1].offset), ("synthetic", at));
        let open = [human.as_str(), first.as_str(), second.as_str()].join("\n");
        let bytes = format!("{open}\n{}\n", bare("turn_completed"));
        let mut splitter = super::Splitter::new();
        splitter.feed(bytes.as_bytes());
        assert_eq!(super::open_run_start(&splitter.finish()), None);
    }

    #[test]
    fn thought_empty_and_unstamped_chunks_earn_no_text() {
        let top = stamped();
        let thought = rec("agent_thought_chunk", "hidden", &top);
        let (rows, coverage) = read_lines_with(&[thought.as_str()], true);
        assert!(rows.is_empty() && coverage.is_empty());
        let blank = agent("   \n  ", &top);
        let close = bare("turn_completed");
        let (rows, coverage) = read_lines_with(&[blank.as_str(), close.as_str()], true);
        assert!(rows.is_empty() && coverage.is_empty(), "empty drops");
        // Unstamped chunks count into the timestamp coverage and contribute
        // nothing: no stamped first chunk, no run.
        let bare_chunk = agent("no ts", "");
        let (rows, coverage) = read_lines_with(&[bare_chunk.as_str(), close.as_str()], true);
        assert!(rows.is_empty());
        assert_eq!(coverage[0].reason, "1 record without a timestamp");
        let a = agent("a", &top);
        let b = agent("b", "");
        let c = agent("c", &top);
        let lines = [a.as_str(), b.as_str(), c.as_str(), close.as_str()];
        let (rows, coverage) = read_lines_with(&lines, true);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, "ac");
        assert_eq!(coverage[0].reason, "1 record without a timestamp");
    }
}
