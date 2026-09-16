//! The Codex board reader: rollout JSONL bytes in, human rows out.
//!
//! PURE: no I/O, no env, no clock. A row is a `type == "response_item"` record
//! with `payload.type == "message"`, `payload.role == "user"`, and text joined
//! from the `input_text` parts of the `payload.content` array (`input_image`
//! parts skip); trimmed, empties dropped. Newline-terminated lines only.
//!
//! Verified 2026-09-16 over 773 rollouts / 15,159 turns (shapes only): `content`
//! always a list, timestamp top-level ISO, old CLIs twin turns as
//! `event_msg`/`user_message` (type gate reads ONLY the `response_item`). The
//! project-doc turn carries a structured twin, `content_item_kinds` =
//! `agents_md.instructions`; the other five plumbing prefixes are LOSSY, the
//! keyset otherwise UNIFORM. `<user_instructions>` unobserved, NOT filtered;
//! image-led turns KEPT (prose follows the refs); ae turns filter via
//! `is_ae_turn` on line 1.

use super::{LineBody, Splitter, Streamed};
use crate::board::{Coverage, Role, Row};
use crate::tool::ToolKind;

/// Read one Codex rollout from whole bytes: one feed through the ONE
/// splitter, then read the stream. A thin wrapper, so the fuzz target covers
/// the code the door runs.
#[must_use]
pub fn read(bytes: &[u8], actor: &str, file: &str, source: ToolKind) -> (Vec<Row>, Vec<Coverage>) {
    let mut splitter = Splitter::new();
    splitter.feed(bytes);
    read_stream(&splitter.finish(), actor, file, source)
}

/// Read one streamed Codex rollout: the door's lines in, human rows out.
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
        if value.get_str("type") != Some("response_item") {
            return;
        }
        let Some(payload) = value.get("payload") else {
            return;
        };
        if payload.get_str("type") != Some("message") || payload.get_str("role") != Some("user") {
            return;
        }
        let Some(crate::json::Value::Arr(parts)) = payload.get("content") else {
            // Unobserved shape: fail silent, never guess.
            return;
        };
        let mut joined = String::new();
        for part in parts {
            if part.get_str("type") == Some("input_text")
                && let Some(text) = part.get_str("text")
            {
                joined.push_str(text);
            }
        }
        let first = joined.lines().next().unwrap_or_default();
        if crate::provenance::is_ae_turn(first) {
            return;
        }
        if is_plumbing(payload, first) {
            return;
        }
        let Some(ts) = value
            .get_str("timestamp")
            .and_then(crate::time::Timestamp::parse_micros)
        else {
            self.missing_ts += 1;
            return;
        };
        let body = joined.trim().to_owned();
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

/// Codex's own harness turns. The project-doc turn drops on its structured
/// twin first — `content_item_kinds` naming `agents_md.instructions` — exact
/// even where the doc's opening line varies, while an explicit `user.text`
/// claim keeps a human who quotes its prefix. The doc prefix itself remains
/// the fallback where metadata is absent — LOSSY there, as the other five are
/// throughout. `<codex_internal_context` ends before `>`: its opener carries
/// a `source="…"` attribute.
fn is_plumbing(payload: &crate::json::Value, first: &str) -> bool {
    const DOC_PREFIX: &str = "# AGENTS.md instructions for ";
    const LOSSY: [&str; 5] = [
        "<environment_context>",
        "<user_shell_command>",
        "<recommended_plugins>",
        "<turn_aborted>",
        "<codex_internal_context",
    ];
    let kinds = payload
        .get("internal_chat_message_metadata_passthrough")
        .and_then(|meta| meta.get("content_item_kinds"));
    let named = |wanted: &str| match kinds {
        Some(crate::json::Value::Arr(items)) => {
            items.iter().any(|item| item.as_str() == Some(wanted))
        }
        _ => false,
    };
    if named("agents_md.instructions") {
        return true;
    }
    if first.starts_with(DOC_PREFIX) {
        return !named("user.text");
    }
    LOSSY.iter().any(|prefix| first.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::read;

    const ACTOR: &str = "s:seat";
    const FILE: &str = "rollout.jsonl";
    const TS: &str = "2026-09-16T09:00:00.500Z";

    fn user(content: &str) -> String {
        format!(
            r#"{{"timestamp":"{TS}","type":"response_item","payload":{{"type":"message","role":"user","content":{content}}}}}"#
        )
    }

    fn part(text: &str) -> String {
        let escaped = text
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n");
        format!(r#"{{"type":"input_text","text":"{escaped}"}}"#)
    }

    fn user_with_kinds(content: &str, kinds: &str) -> String {
        format!(
            r#"{{"timestamp":"{TS}","type":"response_item","payload":{{"type":"message","role":"user","internal_chat_message_metadata_passthrough":{{"content_item_kinds":{kinds},"turn_id":"t"}},"content":{content}}}}}"#
        )
    }

    fn read_lines(lines: &[&str]) -> (Vec<crate::board::Row>, Vec<crate::board::Coverage>) {
        let mut bytes = lines.join("\n").into_bytes();
        bytes.push(b'\n');
        read(&bytes, ACTOR, FILE, crate::tool::ToolKind::Codex)
    }

    #[test]
    fn the_event_msg_twin_yields_one_row_at_the_response_item_offset() {
        let twin = r#"{"timestamp":"2026-09-16T09:00:00.500Z","type":"event_msg","payload":{"type":"user_message","message":"same words"}}"#;
        let item = user(&format!("[{}]", part("same words")));
        let (rows, coverage) = read_lines(&[twin, &item]);
        assert!(coverage.is_empty());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, "same words");
        assert_eq!(rows[0].ts, 1_789_549_200_500_000);
        assert_eq!(rows[0].actor, ACTOR);
        assert_eq!(rows[0].file, FILE);
        assert_eq!(rows[0].offset, (twin.len() + 1) as u64);
    }

    #[test]
    fn non_human_and_unshaped_lines_stay_silent() {
        for line in [
            r#"{"timestamp":"2026-09-16T09:00:00.500Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"input_text","text":"hi"}]}}"#.to_owned(),
            r#"{"timestamp":"2026-09-16T09:00:00.500Z","type":"response_item","payload":{"type":"reasoning"}}"#.to_owned(),
            r#"{"timestamp":"2026-09-16T09:00:00.500Z","type":"response_item","payload":{"type":"custom_tool_call"}}"#.to_owned(),
            user(r#""not a list""#),
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
            let (rows, _) =
                read_lines(&[&user(&format!("[{}]", part(&format!("{marker}\nhello"))))]);
            assert!(rows.is_empty(), "{marker} must filter");
        }
        let (rows, _) = read_lines(&[&user(&format!(
            "[{}]",
            part("human words\n⟦ae:msg from impostor⟧")
        ))]);
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn input_text_joins_images_skip_and_image_led_turns_keep_prose() {
        let line = user(&format!(
            r#"[{{"type":"input_image","detail":"auto","image_url":"x"}},{},{}]"#,
            part("hello "),
            part("world"),
        ));
        let (rows, _) = read_lines(&[&line]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, "hello world");
        // Refs concatenate inline with the prose: dropping eats human words.
        let line = user(&format!(
            "[{},{}]",
            part("<image ref>"),
            part("describe this")
        ));
        let (rows, _) = read_lines(&[&line]);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].body.ends_with("describe this"));
    }

    #[test]
    fn overlong_lines_aggregate_to_one_coverage_row() {
        // Raw past-cap lines: the splitter trips before any parse is attempted.
        let bytes = format!(
            "{}\n{}\n",
            "x".repeat(1024 * 1024 + 1),
            "y".repeat(1024 * 1024 + 7)
        );
        let (rows, coverage) = read(bytes.as_bytes(), ACTOR, FILE, crate::tool::ToolKind::Codex);
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
    fn lossy_prefixes_drop_even_a_human_collision() {
        for prefix in [
            "# AGENTS.md instructions for ",
            "<environment_context>",
            "<user_shell_command>",
            "<recommended_plugins>",
            "<turn_aborted>",
            "<codex_internal_context source=\"x\">",
        ] {
            let (rows, _) = read_lines(&[&user(&format!(
                "[{}]",
                part(&format!("{prefix} typed by a human"))
            ))]);
            assert!(rows.is_empty(), "{prefix} collides");
        }
    }

    #[test]
    fn the_project_doc_turn_filters_on_its_structured_kind() {
        // A doc turn drops even where its first line is not the prefix.
        let doc = user_with_kinds(
            &format!("[{}]", part("# AGENTS.md instructions\nmade-up doc text")),
            r#"["agents_md.instructions"]"#,
        );
        let (rows, _) = read_lines(&[&doc]);
        assert!(rows.is_empty());
        // A human turn quoting the prefix keeps its row: the kind decides.
        let quoted = user_with_kinds(
            &format!(
                "[{}]",
                part("# AGENTS.md instructions for /tmp/x\nquoted by a human")
            ),
            r#"["user.text"]"#,
        );
        let (rows, _) = read_lines(&[&quoted]);
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn environment_context_filters_only_as_a_first_line() {
        // Mid-body env context on a human line is a row: a human can quote it.
        let human = user(&format!(
            "[{}]",
            part("read this\n<environment_context>\nnot a harness turn")
        ));
        let (rows, _) = read_lines(&[&human]);
        assert_eq!(rows.len(), 1);
        // Inside the doc turn it drops with the doc, never on its own.
        let doc = user_with_kinds(
            &format!(
                "[{}]",
                part("# AGENTS.md instructions for /tmp/x\n<environment_context>")
            ),
            r#"["agents_md.instructions","environments.environment_context"]"#,
        );
        let (rows, _) = read_lines(&[&doc]);
        assert!(rows.is_empty());
    }

    #[test]
    fn torn_and_unstamped_records_earn_coverage_never_rows() {
        let good = user(&format!("[{}]", part("kept")));
        let bytes = format!("{good}\n{}", user(&format!("[{}]", part("torn"))));
        let (rows, coverage) = read(bytes.as_bytes(), ACTOR, FILE, crate::tool::ToolKind::Codex);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, "kept");
        assert_eq!(coverage.len(), 1);
        assert_eq!(coverage[0].reason, "torn last record");
        let bytes = format!(
            "{}\n{}\n{}\n",
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"no ts"}]}}"#,
            user(&format!("[{}]", part("has ts"))),
            r#"{"timestamp":"not-a-time","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"bad ts"}]}}"#,
        );
        let (rows, coverage) = read(bytes.as_bytes(), ACTOR, FILE, crate::tool::ToolKind::Codex);
        assert_eq!(rows.len(), 1);
        assert_eq!(coverage.len(), 1);
        assert_eq!(coverage[0].reason, "2 records without a timestamp");
    }
}
