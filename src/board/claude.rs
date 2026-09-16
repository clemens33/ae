//! The Claude Code board reader: transcript JSONL bytes in, human rows out.
//!
//! PURE: no I/O, no env, no clock. The caller locates the file and hands over
//! its bytes; this module never sees a path. Behaviour is ported from the jq
//! reference (`.local/distill-probe.jq`), never its text:
//!
//! * only newline-terminated lines are records; a torn tail is reported, never
//!   trusted;
//! * a row is a `type == "user"` record with STRING `message.content` whose
//!   first line is bare of any ae marker and is not Claude's own plumbing;
//! * `<system-reminder>` spans are stripped, the body trimmed, empties dropped.
//!
//! Plumbing filter, verified 2026-09-16 against a 15,435-line local transcript
//! (567 string user turns; shapes only, content never quoted):
//!
//! * `isCompactSummary == true` filters alone — 11/11 continuation summaries
//!   carry it, zero other string user turns do. EXACT.
//! * `<local-command-caveat>` requires the field AND the first-line prefix:
//!   all 28 caveat records carry `isMeta == true`, but 4 other `isMeta` records
//!   match no prefix and are KEPT — exclusion needs proof, so the field alone
//!   would over-drop. FIELD-CONFIRMED.
//! * `<task-notification>` requires `promptSource == "system"` AND the prefix:
//!   2/2 carry it, 4 other `promptSource == "system"` records match no prefix
//!   and are KEPT. FIELD-CONFIRMED.
//! * `<command-name>` and `<local-command-stdout>` have no structured twin in
//!   any observed record — first-line prefix only. LOSSY: a human turn opening
//!   with that exact prefix is dropped, pinned by a collision test below.
//! * `[Request interrupted` and `Your claude.ai usage limit has reset` appear
//!   in NO local transcript; both are TAKEN from the jq reference, first-line
//!   prefix only, LOSSY with collision tests.

use super::{LINE_CAP, Line, LineBody, Streamed};
use crate::board::{Coverage, Role, Row};
use crate::tool::ToolKind;

/// Read one Claude transcript from whole bytes: split into lines exactly as
/// the door would stream them, then read the stream. A thin wrapper, so the
/// fuzz target covers the code the door runs.
#[must_use]
pub fn read(bytes: &[u8], actor: &str, file: &str, source: ToolKind) -> (Vec<Row>, Vec<Coverage>) {
    let mut lines = Vec::new();
    let mut start = 0usize;
    let mut torn = false;
    while start < bytes.len() {
        let Some(relative) = bytes[start..].iter().position(|byte| *byte == b'\n') else {
            torn = !bytes[start..].is_empty();
            break;
        };
        let end = start + relative;
        lines.push(Line {
            offset: start as u64,
            body: LineBody::Full(bytes[start..end].to_vec()),
        });
        start = end + 1;
    }
    read_stream(&Streamed { lines, torn }, actor, file, source)
}

/// Read one streamed Claude transcript: the door's lines in, human rows out.
///
/// The caller supplies `source`: the reader translates bytes, it never decides
/// which tool it reads — production tool literals live in `src/tool.rs` alone
/// (`per_tool_branches_live_only_in_the_adapter_rows`), and the locating glue
/// already classifies the seat before it calls here.
#[must_use]
pub(crate) fn read_stream(
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
        match &line.body {
            LineBody::Full(bytes) => sink.push_line(bytes, line.offset),
            LineBody::Overlong(len) => {
                sink.cover(&format!("line exceeds 1 MiB cap ({len} bytes)"));
            }
        }
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

/// One file's in-progress read: the caller's naming plus the rows, coverage
/// and timestamp-miss count accumulated so far.
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

    /// Attempt one newline-terminated line at byte `offset`.
    fn push_line(&mut self, line: &[u8], offset: u64) {
        if line.len() > LINE_CAP {
            self.cover(&format!("line exceeds 1 MiB cap ({} bytes)", line.len()));
            return;
        }
        // Not UTF-8 or not JSON is not a record — skipped silently, like the usage
        // reader skips malformed lines. Only a line that COULD be a turn and is
        // refused for a stated reason earns coverage.
        let Ok(text) = str::from_utf8(line) else {
            return;
        };
        let Ok(value) = crate::json::parse(text) else {
            return;
        };
        if value.get_str("type") != Some("user") {
            return;
        }
        if value
            .get("isCompactSummary")
            .is_some_and(|flag| *flag == crate::json::Value::Bool(true))
        {
            return;
        }
        let Some(content) = value
            .get("message")
            .and_then(|message| message.get("content"))
        else {
            return;
        };
        let crate::json::Value::Str(content) = content else {
            // Array content is a tool result or structured turn, never prose.
            return;
        };
        let first = content.lines().next().unwrap_or_default();
        if crate::provenance::is_ae_turn(first) {
            return;
        }
        if is_plumbing(&value, first) {
            return;
        }
        let Some(ts) = value
            .get_str("timestamp")
            .and_then(crate::time::Timestamp::parse_micros)
        else {
            self.missing_ts += 1;
            return;
        };
        let body = strip_reminders(content).trim().to_owned();
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

/// Claude's own harness turns, by structured field where one exists.
fn is_plumbing(value: &crate::json::Value, first: &str) -> bool {
    const CAVEAT: &str = "<local-command-caveat>";
    const TASK: &str = "<task-notification>";
    // LOSSY: no structured twin observed — a human turn opening with one of
    // these exact prefixes is dropped. Each carries a collision test.
    const LOSSY: [&str; 4] = [
        "<command-name>",
        "<local-command-stdout>",
        "[Request interrupted",
        "Your claude.ai usage limit has reset",
    ];
    let flagged = |key: &str| {
        value
            .get(key)
            .is_some_and(|flag| *flag == crate::json::Value::Bool(true))
    };
    if flagged("isMeta") && first.starts_with(CAVEAT) {
        return true;
    }
    if value.get_str("promptSource") == Some("system") && first.starts_with(TASK) {
        return true;
    }
    LOSSY.iter().any(|prefix| first.starts_with(prefix))
}

/// Strip every `<system-reminder>…</system-reminder>` span, innermost first so
/// nesting collapses correctly. An unclosed opener is left alone — eating the
/// rest of a human turn on a guess would be the worse error — and an orphan
/// closer is skipped past, so a proper span after it still strips.
fn strip_reminders(body: &str) -> String {
    const OPEN: &str = "<system-reminder>";
    const CLOSE: &str = "</system-reminder>";
    let mut out = body.to_owned();
    let mut cursor = 0;
    while let Some(relative) = out[cursor..].find(CLOSE) {
        let close_at = cursor + relative;
        if let Some(open_at) = out[..close_at].rfind(OPEN) {
            out.replace_range(open_at..close_at + CLOSE.len(), "");
            cursor = open_at;
        } else {
            cursor = close_at + CLOSE.len();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::read;

    const ACTOR: &str = "s:seat";
    const FILE: &str = "transcript.jsonl";

    fn user(content: &str) -> String {
        format!(
            r#"{{"type":"user","timestamp":"2026-09-16T09:00:00.500Z","message":{{"role":"user","content":{content}}}}}"#
        )
    }

    fn read_one(line: &str) -> (Vec<crate::board::Row>, Vec<crate::board::Coverage>) {
        let mut bytes = line.as_bytes().to_vec();
        bytes.push(b'\n');
        read(&bytes, ACTOR, FILE, crate::tool::ToolKind::Claude)
    }

    #[test]
    fn a_bare_human_turn_is_a_row_with_its_byte_offset() {
        let head = user(r#""first turn""#) + "\n";
        let tail = user(r#""second turn""#);
        let bytes = format!("{head}{tail}\n");
        let (rows, coverage) = read(bytes.as_bytes(), ACTOR, FILE, crate::tool::ToolKind::Claude);
        assert!(coverage.is_empty());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].body, "first turn");
        assert_eq!(rows[0].offset, 0);
        assert_eq!(rows[1].body, "second turn");
        assert_eq!(rows[1].offset, head.len() as u64);
        assert_eq!(rows[0].ts, 1_789_549_200_500_000);
        assert_eq!(rows[0].actor, ACTOR);
        assert_eq!(rows[0].file, FILE);
    }

    #[test]
    fn every_ae_marker_on_line_one_filters_the_turn() {
        for marker in [
            "⟦ae:msg from lead⟧",
            "⟦ae:ctx⟧",
            "⟦ae:brief from lead⟧",
            "⟦ae:interrupt from lead⟧",
        ] {
            let (rows, _) = read_one(&user(&format!(r#""{marker}\nhello""#)));
            assert!(rows.is_empty(), "{marker} must filter");
        }
    }

    #[test]
    fn a_marker_past_line_one_is_prose_and_survives() {
        let (rows, _) = read_one(&user(r#""human words\n⟦ae:msg from impostor⟧""#));
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn a_torn_last_record_is_reported_and_never_trusted() {
        let good = user(r#""kept""#) + "\n";
        let bytes = format!("{good}{}", user(r#""torn""#));
        let (rows, coverage) = read(bytes.as_bytes(), ACTOR, FILE, crate::tool::ToolKind::Claude);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, "kept");
        assert_eq!(coverage.len(), 1);
        assert_eq!(coverage[0].reason, "torn last record");
    }

    #[test]
    fn an_empty_tail_after_the_last_newline_is_not_torn() {
        let (rows, coverage) = read_one(&user(r#""whole""#));
        assert_eq!(rows.len(), 1);
        assert!(coverage.is_empty());
        let (rows, coverage) = read(b"", ACTOR, FILE, crate::tool::ToolKind::Claude);
        assert!(rows.is_empty() && coverage.is_empty());
    }

    #[test]
    fn a_line_past_the_cap_is_skipped_with_its_length_named() {
        let big = "x".repeat(1024 * 1024 + 1);
        let line = user(&format!(r#""{big}""#));
        let (rows, coverage) = read_one(&line);
        assert!(rows.is_empty());
        assert_eq!(coverage.len(), 1);
        assert!(coverage[0].reason.starts_with("line exceeds 1 MiB cap ("));
        assert!(coverage[0].reason.contains(&line.len().to_string()));
    }

    #[test]
    fn array_content_is_a_tool_result_and_stays_silent() {
        let (rows, coverage) = read_one(&user(r#"[{"type":"tool_result"}]"#));
        assert!(rows.is_empty() && coverage.is_empty());
    }

    #[test]
    fn non_user_records_malformed_lines_and_non_utf8_stay_silent() {
        let (rows, coverage) = read_one(
            r#"{"type":"assistant","timestamp":"2026-09-16T09:00:00.500Z","message":{"content":"hi"}}"#,
        );
        assert!(rows.is_empty() && coverage.is_empty());
        let (rows, coverage) = read_one(r#"{"type":"user",broken"#);
        assert!(rows.is_empty() && coverage.is_empty());
        let (rows, coverage) = read(b"\xff\xfe\n", ACTOR, FILE, crate::tool::ToolKind::Claude);
        assert!(rows.is_empty() && coverage.is_empty());
    }

    #[test]
    fn turns_without_a_timestamp_count_into_one_coverage_row() {
        let bytes = format!(
            "{}\n{}\n{}\n",
            r#"{"type":"user","message":{"content":"no ts"}}"#,
            user(r#""has ts""#),
            r#"{"type":"user","timestamp":"not-a-time","message":{"content":"bad ts"}}"#,
        );
        let (rows, coverage) = read(bytes.as_bytes(), ACTOR, FILE, crate::tool::ToolKind::Claude);
        assert_eq!(rows.len(), 1);
        assert_eq!(coverage.len(), 1);
        assert_eq!(coverage[0].reason, "2 records without a timestamp");
    }

    #[test]
    fn reminders_strip_including_nested_and_empties_drop() {
        let (rows, _) = read_one(&user(
            r#""before <system-reminder>outer <system-reminder>inner</system-reminder> still outer</system-reminder> after""#,
        ));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, "before  after");
        let (rows, _) = read_one(&user(r#""<system-reminder>only</system-reminder>""#));
        assert!(rows.is_empty());
        // Unclosed: left alone rather than eating the turn.
        let (rows, _) = read_one(&user(r#""kept <system-reminder>oops""#));
        assert_eq!(rows.len(), 1);
        assert!(rows[0].body.contains("<system-reminder>"));
    }

    #[test]
    fn an_orphan_closer_is_skipped_and_a_later_span_still_strips() {
        let (rows, _) = read_one(&user(
            r#""before </system-reminder> middle <system-reminder>gone</system-reminder> after""#,
        ));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, "before </system-reminder> middle  after");
    }

    #[test]
    fn a_compact_summary_filters_on_its_field() {
        let (rows, _) = read_one(
            r#"{"type":"user","isCompactSummary":true,"timestamp":"2026-09-16T09:00:00.500Z","message":{"content":"This session is being continued from a previous conversation"}}"#,
        );
        assert!(rows.is_empty());
    }

    #[test]
    fn caveat_and_task_notification_need_field_and_prefix() {
        let (rows, _) = read_one(
            r#"{"type":"user","isMeta":true,"timestamp":"2026-09-16T09:00:00.500Z","message":{"content":"<local-command-caveat>noise"}}"#,
        );
        assert!(rows.is_empty());
        // The field alone keeps the turn: 4 such records exist locally.
        let (rows, _) = read_one(
            r#"{"type":"user","isMeta":true,"timestamp":"2026-09-16T09:00:00.500Z","message":{"content":"human words"}}"#,
        );
        assert_eq!(rows.len(), 1);
        let (rows, _) = read_one(
            r#"{"type":"user","promptSource":"system","timestamp":"2026-09-16T09:00:00.500Z","message":{"content":"<task-notification>noise"}}"#,
        );
        assert!(rows.is_empty());
        let (rows, _) = read_one(
            r#"{"type":"user","promptSource":"system","timestamp":"2026-09-16T09:00:00.500Z","message":{"content":"human words"}}"#,
        );
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn lossy_prefixes_drop_even_a_human_collision() {
        // The documented loss: no structured twin exists, so the prefix alone
        // decides. If a twin is ever found, this test names the fix.
        for prefix in [
            "<command-name>",
            "<local-command-stdout>",
            "[Request interrupted",
            "Your claude.ai usage limit has reset",
        ] {
            let (rows, _) = read_one(&user(&format!(r#""{prefix} typed by a human""#)));
            assert!(rows.is_empty(), "{prefix} collides");
        }
    }
}
