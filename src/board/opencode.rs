//! The `OpenCode` board reader: one `opencode export` JSON document in, rows out.
//!
//! PURE: no I/O, no env, no clock. `OpenCode` keeps its conversations in `SQLite`,
//! which ae never opens, so the board reads them through the CLI's own export
//! instead: ONE JSON document (`{info, messages:[{info, parts:[…]}]}`), never
//! JSONL — no splitter, no byte offsets. Measured shape (opencode 1.18.31,
//! 2026-09-18, shapes only): message ids are `msg_` + an opaque run, immutable
//! and stable across two exports; `time.created` is epoch MILLISECONDS; `text`
//! parts carry the prose, and a `reasoning` part carries a `text` field TOO —
//! never read. Reading rules: `role == "user"` is a human turn; with
//! `--assistant`, `role == "assistant"` is a reply; a body is the message's
//! `text` parts joined in order, trimmed, empties dropped. One row per message
//! whose body survives. Row identity is the message's own id, because the
//! export is regenerated per read: `file = "opencode:<session>#<message>"`,
//! `offset = 0`. The ae-turn filter is the board's ONE `hidden` pass, after
//! this reader — this reader emits every human row and never hides one itself.

use super::{Coverage, Role, Row};
use crate::json::Value;
use crate::tool::ToolKind;

/// A whole export larger than this is refused before parsing. The measured
/// probe session exported 0.5 MiB at 16 messages; 16 MiB matches the quota
/// scan's own byte budget and keeps the one-document parse bounded.
pub(crate) const EXPORT_CAP: usize = 16 * 1024 * 1024;

/// At most this many rows come out of one export; the messages left unread by
/// the stop are named in the coverage line.
pub(crate) const ROW_CAP: usize = 4_096;

/// Read one whole `opencode export` document: the document's bytes, the
/// conversation id the caller asked for, and the seat's row identity triple.
/// Genuine human turns come back; with `assistant` the model's replies ride
/// beside them, `text` parts only. Every refusal is a [`Coverage`] with a
/// reason, never a silent empty board.
#[must_use]
pub fn read(
    export: &[u8],
    requested_id: &str,
    actor: &str,
    source: ToolKind,
    assistant: bool,
) -> (Vec<Row>, Vec<Coverage>) {
    let mut sink = Sink {
        actor,
        source,
        assistant,
        session: requested_id,
        rows: Vec::new(),
        coverage: Vec::new(),
        missing_id: 0,
        missing_ts: 0,
    };
    if export.len() > EXPORT_CAP {
        sink.cover("export exceeds the read budget");
        return (sink.rows, sink.coverage);
    }
    let Ok(text) = str::from_utf8(export) else {
        sink.cover("export unreadable");
        return (sink.rows, sink.coverage);
    };
    let Ok(document) = crate::json::parse(text) else {
        sink.cover("export unreadable");
        return (sink.rows, sink.coverage);
    };
    let Some(info) = document.get("info") else {
        sink.cover("export unreadable");
        return (sink.rows, sink.coverage);
    };
    match info.get_str("id") {
        Some(id) if id == requested_id => {}
        Some("") | None => {
            sink.cover("export unreadable");
            return (sink.rows, sink.coverage);
        }
        Some(_) => {
            sink.cover("export names another session");
            return (sink.rows, sink.coverage);
        }
    }
    let Some(Value::Arr(messages)) = document.get("messages") else {
        sink.cover("export unreadable");
        return (sink.rows, sink.coverage);
    };
    sink.push_messages(messages);
    (sink.rows, sink.coverage)
}

/// One export's in-progress read.
struct Sink<'a> {
    actor: &'a str,
    source: ToolKind,
    /// `--assistant`: the reply path is live. Off, an assistant message is
    /// classified and dropped exactly as before the flag existed.
    assistant: bool,
    /// The requested conversation id: the `file` identity's first half, and
    /// the id the document's own `info.id` must match.
    session: &'a str,
    rows: Vec<Row>,
    coverage: Vec<Coverage>,
    missing_id: u64,
    missing_ts: u64,
}

impl Sink<'_> {
    fn cover(&mut self, reason: &str) {
        self.coverage.push(Coverage {
            actor: self.actor.to_owned(),
            reason: reason.to_owned(),
        });
    }

    /// Every message, in document order, until the row cap stops the walk.
    fn push_messages(&mut self, messages: &[Value]) {
        let mut unread = 0;
        for (index, message) in messages.iter().enumerate() {
            if self.rows.len() >= ROW_CAP {
                unread = messages.len() - index;
                break;
            }
            self.push_message(message);
        }
        if self.missing_id > 0 {
            let (noun, verb) = if self.missing_id == 1 {
                ("message", "lacks")
            } else {
                ("messages", "lack")
            };
            self.cover(&format!("{} {noun} {verb} an id", self.missing_id));
        }
        if self.missing_ts > 0 {
            let noun = if self.missing_ts == 1 {
                "message"
            } else {
                "messages"
            };
            self.cover(&format!("{} {noun} without a timestamp", self.missing_ts));
        }
        if unread > 0 {
            let noun = if unread == 1 { "message" } else { "messages" };
            self.cover(&format!(
                "export exceeds the read budget — {unread} {noun} unread"
            ));
        }
    }

    /// One message: classify by `info.role`, name it by `info.id`, stamp it
    /// from `info.time.created` (millis), and join its `text` parts. A message
    /// this read would not emit is skipped silently; one it would emit but
    /// cannot name or stamp is counted.
    fn push_message(&mut self, message: &Value) {
        let Some(info) = message.get("info") else {
            self.missing_id += 1;
            return;
        };
        let role = match info.get_str("role") {
            Some("user") => Role::Human,
            Some("assistant") if self.assistant => Role::Assistant,
            _ => return,
        };
        let Some(id) = info.get_str("id").filter(|id| !id.is_empty()) else {
            self.missing_id += 1;
            return;
        };
        let Some(ms) = created_millis(info) else {
            self.missing_ts += 1;
            return;
        };
        let body = join_text_parts(message).trim().to_owned();
        if body.is_empty() {
            return;
        }
        self.rows.push(Row {
            ts: ms,
            actor: self.actor.to_owned(),
            role,
            body,
            source: self.source,
            file: format!("opencode:{}#{id}", self.session),
            offset: 0,
            generation: 0,
        });
    }
}

/// `info.time.created` in micros: the export's native integer MILLISECONDS
/// scaled to the board's clock. A missing, non-integer or unspellable moment
/// is damage and counts, exactly like an absent timestamp elsewhere.
fn created_millis(info: &Value) -> Option<i64> {
    let Value::Num(ms) = info.get("time")?.get("created")? else {
        return None;
    };
    ms.checked_mul(1_000)
}

/// The body: every `type == "text"` part's `text`, in order, and nothing else.
/// `reasoning` parts carry a `text` field too and are NEVER read; `tool` and
/// `step-*` parts are never read.
fn join_text_parts(message: &Value) -> String {
    let Some(Value::Arr(parts)) = message.get("parts") else {
        return String::new();
    };
    let mut joined = String::new();
    for part in parts {
        if part.get_str("type") == Some("text")
            && let Some(text) = part.get_str("text")
        {
            joined.push_str(text);
        }
    }
    joined
}

#[cfg(test)]
mod tests {
    use super::{EXPORT_CAP, ROW_CAP, read};
    use crate::board::{Coverage, Role, Row, collect};
    use crate::tool::ToolKind;

    const ACTOR: &str = "aedev:ocseat";
    const SID: &str = "ses_00000000000000000000000000";
    const MS: i64 = 1_789_549_200_500;

    fn text_part(text: &str) -> String {
        let escaped = text
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n");
        format!(r#"{{"type":"text","text":"{escaped}"}}"#)
    }

    fn reasoning_part(text: &str) -> String {
        let escaped = text
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n");
        format!(
            r#"{{"type":"reasoning","text":"{escaped}","time":{{"start":1,"end":2}},"id":"prt_1","sessionID":"{SID}","messageID":"msg_1"}}"#
        )
    }

    fn tool_part() -> String {
        format!(
            r#"{{"type":"tool","tool":"bash","callID":"call_1","state":{{"status":"completed","input":{{"command":"secret"}},"output":"tool output never read"}},"id":"prt_2","sessionID":"{SID}","messageID":"msg_1"}}"#
        )
    }

    fn step_part(kind: &str) -> String {
        format!(
            r#"{{"type":"{kind}","snapshot":"snap","id":"prt_3","sessionID":"{SID}","messageID":"msg_1"}}"#
        )
    }

    /// One message with a raw `parts` array.
    fn message(id: &str, role: &str, created: &str, parts: &[String]) -> String {
        format!(
            r#"{{"info":{{"id":"{id}","sessionID":"{SID}","role":"{role}","time":{{"created":{created}}}}},"parts":[{}]}}"#,
            parts.join(",")
        )
    }

    /// A user message whose body is one text part.
    fn user(id: &str, created: &str, text: &str) -> String {
        message(id, "user", created, &[text_part(text)])
    }

    fn document(messages: &[String]) -> String {
        format!(
            r#"{{"info":{{"id":"{SID}","slug":"probe","directory":"/probe"}},"messages":[{}]}}"#,
            messages.join(",")
        )
    }

    fn read_doc(export: &str, assistant: bool) -> (Vec<Row>, Vec<Coverage>) {
        read(export.as_bytes(), SID, ACTOR, ToolKind::OpenCode, assistant)
    }

    fn reasons(coverage: &[Coverage]) -> Vec<&str> {
        coverage.iter().map(|item| item.reason.as_str()).collect()
    }

    #[test]
    fn the_user_turn_joins_its_text_parts_and_scales_millis_to_micros() {
        let line = message(
            "msg_human",
            "user",
            &MS.to_string(),
            &[
                text_part("first "),
                reasoning_part("chain of thought"),
                tool_part(),
                step_part("step-start"),
                text_part("second"),
            ],
        );
        let (rows, coverage) = read_doc(&document(&[line]), false);
        assert!(coverage.is_empty());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, "first second", "text parts only, in order");
        assert_eq!(rows[0].ts, MS * 1_000, "millis become micros");
        assert_eq!(rows[0].role, Role::Human);
        assert_eq!(rows[0].actor, ACTOR);
        assert_eq!(rows[0].source, ToolKind::OpenCode);
        assert_eq!(rows[0].file, format!("opencode:{SID}#msg_human"));
        assert_eq!(rows[0].offset, 0);
        assert_eq!(rows[0].generation, 0);
    }

    #[test]
    fn the_assistant_reply_reads_only_with_the_flag_and_only_text_parts() {
        let line = message(
            "msg_reply",
            "assistant",
            &MS.to_string(),
            &[
                reasoning_part("hidden reasoning words"),
                tool_part(),
                text_part("visible reply"),
                step_part("step-finish"),
            ],
        );
        let export = document(&[line]);
        let (rows, coverage) = read_doc(&export, false);
        assert!(rows.is_empty() && coverage.is_empty(), "flag off: no reply");
        let (rows, coverage) = read_doc(&export, true);
        assert!(coverage.is_empty());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].role, Role::Assistant);
        assert_eq!(rows[0].body, "visible reply");
        assert!(
            !rows[0].body.contains("hidden") && !rows[0].body.contains("tool"),
            "reasoning and tool parts never reach a body: {}",
            rows[0].body
        );
    }

    #[test]
    fn a_marker_on_line_one_is_emitted_for_the_boards_one_hidden_pass() {
        let marked = user("msg_marked", &MS.to_string(), "⟦ae:msg from lead⟧\nhello");
        let buried = user(
            "msg_buried",
            &MS.to_string(),
            "human words\n⟦ae:msg from lead⟧",
        );
        let (rows, coverage) = read_doc(&document(&[marked, buried]), false);
        assert!(coverage.is_empty());
        assert_eq!(rows.len(), 2, "the reader hides nothing itself");
        assert!(rows[0].body.starts_with("⟦ae:msg from lead⟧"));
        assert_eq!(rows[1].body, "human words\n⟦ae:msg from lead⟧");
    }

    #[test]
    fn a_message_without_a_text_part_drops_silently() {
        let only_tools = message(
            "msg_tools",
            "assistant",
            &MS.to_string(),
            &[tool_part(), step_part("step-start")],
        );
        let blank = user("msg_blank", &MS.to_string(), "   ");
        let (rows, coverage) = read_doc(&document(&[only_tools, blank]), true);
        assert!(rows.is_empty() && coverage.is_empty());
    }

    #[test]
    fn malformed_documents_are_unreadable_never_silent_or_panicking() {
        let mismatch = document(&[user("msg_1", &MS.to_string(), "words")])
            .replace(SID, "ses_11111111111111111111111111");
        let cases: Vec<(&[u8], &str)> = vec![
            (b"\xff\xfe{\"info\":{}", "export unreadable"),
            (b"{ not json", "export unreadable"),
            (b"[]", "export unreadable"),
            (br#"{"info":{}}"#, "export unreadable"),
            (br#"{"info":{"id":""},"messages":[]}"#, "export unreadable"),
            (
                br#"{"info":{"id":"ses_00000000000000000000000000"},"messages":{}}"#,
                "export unreadable",
            ),
            (mismatch.as_bytes(), "export names another session"),
        ];
        for (export, want) in cases {
            let (rows, coverage) = read(export, SID, ACTOR, ToolKind::OpenCode, false);
            assert!(rows.is_empty(), "{want}");
            assert_eq!(reasons(&coverage), [want], "{want}");
        }
    }

    #[test]
    fn per_message_damage_is_counted_and_the_rest_still_reads() {
        let no_info = r#"{"parts":[{"type":"text","text":"lost"}]}"#;
        let no_id = r#"{"info":{"role":"user","time":{"created":1789549200500}},"parts":[{"type":"text","text":"lost"}]}"#;
        let no_ts = message("msg_nots", "user", "null", &[text_part("lost")]);
        let float_ts = message("msg_float", "user", "1.5", &[text_part("lost")]);
        let huge_ts = message(
            "msg_huge",
            "user",
            &i64::MAX.to_string(),
            &[text_part("lost")],
        );
        let good = user("msg_good", &MS.to_string(), "kept");
        let (rows, coverage) = read_doc(
            &document(&[
                no_info.to_owned(),
                no_id.to_owned(),
                no_ts,
                float_ts,
                huge_ts,
                good,
            ]),
            false,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, "kept");
        assert_eq!(
            reasons(&coverage),
            ["2 messages lack an id", "3 messages without a timestamp"]
        );
    }

    #[test]
    fn the_row_cap_stops_the_walk_and_names_what_is_unread() {
        let lines: Vec<String> = (0..ROW_CAP + 3)
            .map(|n| user(&format!("msg_{n}"), &MS.to_string(), "words"))
            .collect();
        let (rows, coverage) = read_doc(&document(&lines), false);
        assert_eq!(rows.len(), ROW_CAP);
        assert_eq!(
            reasons(&coverage),
            ["export exceeds the read budget — 3 messages unread"]
        );
    }

    #[test]
    fn an_export_over_the_cap_refuses_before_parsing() {
        let over = vec![b'x'; EXPORT_CAP + 1];
        let (rows, coverage) = read(&over, SID, ACTOR, ToolKind::OpenCode, false);
        assert!(rows.is_empty());
        assert_eq!(reasons(&coverage), ["export exceeds the read budget"]);
    }

    #[test]
    fn two_reads_of_one_export_dedup_to_one_row_each_through_collect() {
        let lines = [
            user("msg_one", &MS.to_string(), "first"),
            user("msg_two", &(MS + 1).to_string(), "second"),
        ];
        let export = document(&lines);
        let (first, _) = read_doc(&export, false);
        let (second, _) = read_doc(&export, false);
        let mut both = first.clone();
        both.extend(second);
        let merged = collect(both);
        assert_eq!(merged.len(), 2, "same ids collapse, offsets never shift");
        assert_eq!(merged[0].body, "first");
        assert_eq!(merged[1].body, "second");
    }

    #[test]
    fn identity_is_the_message_id_never_the_array_index() {
        // The D2 reason: an export is regenerated per read, so identity rides
        // the message's own id. Under an index identity these two reads would
        // mint the SAME (file, offset) for two DIFFERENT messages — the older
        // read's alpha at 0 and the newer read's beta at 0.
        let older = document(&[
            user("msg_a", &MS.to_string(), "alpha"),
            user("msg_b", &(MS + 1).to_string(), "beta"),
        ]);
        let newer = document(&[
            user("msg_b", &(MS + 1).to_string(), "beta"),
            user("msg_a", &MS.to_string(), "alpha"),
        ]);
        let (first, _) = read_doc(&older, false);
        let (second, _) = read_doc(&newer, false);
        let mut both = first;
        both.extend(second);
        let pairs: Vec<(String, String)> = collect(both)
            .into_iter()
            .map(|row| (row.file, row.body))
            .collect();
        assert_eq!(
            pairs,
            [
                (format!("opencode:{SID}#msg_a"), "alpha".to_owned()),
                (format!("opencode:{SID}#msg_b"), "beta".to_owned()),
            ],
            "each id keeps its own body across a reorder"
        );
    }
}
