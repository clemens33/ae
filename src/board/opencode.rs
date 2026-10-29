//! The `OpenCode` board reader: one `opencode session export` JSON document in, rows out.
//!
//! PURE: no I/O, no env, no clock. `OpenCode` keeps its conversations in `SQLite`,
//! which ae never opens, so the board reads them through the CLI's own export
//! instead: ONE JSON document (`{info, messages:[{id, time, type, …}]}`), never
//! JSONL — no splitter, no byte offsets. `OpenCode` 2 ONLY: V1's `export` command
//! and its `info.role` / `parts` shape are not read (measured 2.2.1, 2026-10-29,
//! shapes only). Message ids are `msg_` + an opaque run, stable across exports
//! and a resume; `time.created` is epoch MILLISECONDS. Reading rules:
//! `type == "user"` is a human turn whose body is the message's `text`; with
//! `--assistant`, `type == "assistant"` is a reply whose body is the `content`
//! parts of `type == "text"`, joined in order (a reasoning part is never read).
//! Bodies are trimmed, empties dropped. Any other STRING type (`idle`,
//! `agent-switched`, …) is skipped silently; a message with no string `type` is
//! damage and counted. One row per message whose body survives. Row identity is
//! the message's own id, because the export is regenerated per read:
//! `file = "opencode:<session>#<message>"`, `offset = 0`. The ae-turn filter is
//! the board's ONE `hidden` pass, after this reader — this reader emits every
//! human row and never hides one itself.

use super::{Coverage, Role, Row};
use crate::json::Value;
use crate::tool::ToolKind;

/// A whole export larger than this is refused before parsing. The measured
/// probe session exported 0.5 MiB at 16 messages; 16 MiB matches the quota
/// scan's own byte budget and keeps the one-document parse bounded. The cap
/// bounds the PARSE, the row walk and the door's read back (it hands the door
/// this ceiling and the door returns at most one byte more). The DISK spill is
/// unbounded by nature: the child writes its whole stdout into the scratch
/// file before any cap is consulted, so an `opencode` printing gigabytes fills
/// that filesystem first — no rlimit exists without unsafe or a dependency.
pub(crate) const EXPORT_CAP: usize = 16 * 1024 * 1024;

/// At most this many rows come out of one export; the messages left unread by
/// the stop are named in the coverage line.
pub(crate) const ROW_CAP: usize = 4_096;

/// Read one whole `opencode session export` document: the document's bytes, the
/// conversation id the caller asked for, and the seat's row identity triple.
/// Genuine human turns come back; with `assistant` the model's replies ride
/// beside them, `text` parts only. Every refusal is a [`Coverage`] with a
/// reason, never a silent empty board.
///
/// The non-UTF-8 refusal below is a UNIT- and FUZZ-only arm: the ONE product
/// caller hands this function the existing door's
/// `String::from_utf8_lossy` output, so a child printing invalid bytes arrives
/// already carrying U+FFFD and prints as an ordinary row (measured 2026-09-18,
/// reviewer). The arm stays because the pure reader must define its own
/// behavior on bytes, and the door's lossy copy is the named residual.
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
        missing_type: 0,
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
    missing_type: u64,
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
        if self.missing_type > 0 {
            let (noun, verb) = if self.missing_type == 1 {
                ("message", "lacks")
            } else {
                ("messages", "lack")
            };
            self.cover(&format!("{} {noun} {verb} a type", self.missing_type));
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

    /// One message: classify by `type`, name it by `id`, stamp it from
    /// `time.created` (millis) and take its text. A message this read would not
    /// emit is skipped silently; one it would emit but cannot name or stamp is
    /// counted, and so is one with no string `type` to classify it by.
    fn push_message(&mut self, message: &Value) {
        let role = match message.get_str("type") {
            Some("user") => Role::Human,
            Some("assistant") if self.assistant => Role::Assistant,
            Some(_) => return,
            None => {
                self.missing_type += 1;
                return;
            }
        };
        let Some(id) = message.get_str("id").filter(|id| !id.is_empty()) else {
            self.missing_id += 1;
            return;
        };
        let Some(micros) = created_micros(message) else {
            self.missing_ts += 1;
            return;
        };
        let body = message_text(message, role).trim().to_owned();
        if body.is_empty() {
            return;
        }
        self.rows.push(Row {
            ts: micros,
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

/// `time.created` in micros: the export's native integer MILLISECONDS scaled to
/// the board's clock. A missing, non-integer or unspellable moment is damage and
/// counts, exactly like an absent timestamp elsewhere.
fn created_micros(message: &Value) -> Option<i64> {
    let Value::Num(ms) = message.get("time")?.get("created")? else {
        return None;
    };
    ms.checked_mul(1_000)
}

/// The body: a user message's `text`, or an assistant message's `content` parts
/// with `type == "text"`, in order, and nothing else. Any other part (reasoning,
/// tool) is NEVER read, and neither role reads the other's field.
fn message_text(message: &Value, role: Role) -> String {
    if role == Role::Human {
        return message.get_str("text").unwrap_or_default().to_owned();
    }
    let Some(Value::Arr(parts)) = message.get("content") else {
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

    /// One V2 assistant message; user bodies are separate top-level text.
    fn message(id: &str, role: &str, created: &str, parts: &[String]) -> String {
        format!(
            r#"{{"id":"{id}","type":"{role}","time":{{"created":{created}}},"content":[{}]}}"#,
            parts.join(",")
        )
    }

    fn user(id: &str, created: &str, text: &str) -> String {
        let mut escaped = String::new();
        crate::json::escape_into(text, &mut escaped);
        format!(
            r#"{{"id":"{id}","type":"user","time":{{"created":{created}}},"text":"{escaped}"}}"#
        )
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
    fn the_user_turn_reads_text_and_scales_millis_to_micros() {
        let line = user("msg_human", &MS.to_string(), "  first second  ");
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
                text_part("visible "),
                text_part("reply"),
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
        let no_info = r#"{"text":"lost"}"#;
        let no_id = r#"{"type":"user","time":{"created":1789549200500},"text":"lost"}"#;
        let no_ts = user("msg_nots", "null", "lost");
        let float_ts = user("msg_float", "1.5", "lost");
        let huge_ts = user("msg_huge", &i64::MAX.to_string(), "lost");
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
            [
                "1 message lacks an id",
                "1 message lacks a type",
                "3 messages without a timestamp"
            ]
        );
    }

    #[test]
    fn the_row_cap_stops_the_walk_and_names_what_is_unread() {
        let lines: Vec<String> = (0..ROW_CAP + 3)
            .map(|n| user(&format!("msg_{n}"), &MS.to_string(), "words"))
            .collect();
        let mut damaged = vec![
            r#"{"type":"user"}"#.to_owned(),
            r#"{"type":null}"#.to_owned(),
            r#"{"type":"user","id":"msg_nots"}"#.to_owned(),
        ];
        damaged.extend(lines);
        let (rows, coverage) = read_doc(&document(&damaged), false);
        assert_eq!(rows.len(), ROW_CAP);
        assert_eq!(
            reasons(&coverage),
            [
                "1 message lacks an id",
                "1 message lacks a type",
                "1 message without a timestamp",
                "export exceeds the read budget — 3 messages unread",
            ]
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
    #[test]
    fn v2_completed_and_resumed_exports_keep_six_stable_text_turns() {
        // R3 oracle: 1+1 completed, 3+3 resumed, two shared ids.
        let first = [
            user("msg_h1", "1789549200500", "question one"),
            message(
                "msg_a1",
                "assistant",
                "1789549201500",
                &[text_part("answer one")],
            ),
            r#"{"type":"idle"}"#.to_owned(),
        ];
        let mut resumed = first.to_vec();
        resumed.extend([
            r#"{"type":"agent-switched"}"#.to_owned(),
            r#"{"type":"model-switched"}"#.to_owned(),
            user("msg_h2", "1789549202500", "question two"),
            message(
                "msg_a2",
                "assistant",
                "1789549203500",
                &[text_part("answer two")],
            ),
            r#"{"type":"idle"}"#.to_owned(),
            user("msg_h3", "1789549204500", "question three"),
            message(
                "msg_a3",
                "assistant",
                "1789549205500",
                &[text_part("answer three")],
            ),
            r#"{"type":"idle"}"#.to_owned(),
        ]);
        for (messages, count) in [(&first[..], 2), (&resumed[..], 6)] {
            for assistant in [false, true] {
                let (rows, coverage) = read_doc(&document(messages), assistant);
                assert!(coverage.is_empty(), "{coverage:?}");
                assert_eq!(rows.len(), if assistant { count } else { count / 2 });
            }
        }
        let (mut old, _) = read_doc(&document(&first), true);
        let (new, _) = read_doc(&document(&resumed), true);
        old.extend(new);
        let rows = collect(old);
        let bodies: Vec<_> = rows.iter().map(|row| row.body.as_str()).collect();
        assert_eq!(
            bodies,
            [
                "question one",
                "answer one",
                "question two",
                "answer two",
                "question three",
                "answer three"
            ]
        );
        for (row, id) in rows
            .iter()
            .zip(["msg_h1", "msg_a1", "msg_h2", "msg_a2", "msg_h3", "msg_a3"])
        {
            assert_eq!(row.file, format!("opencode:{SID}#{id}"));
        }
    }

    #[test]
    fn damaged_types_are_counted_but_valid_unknown_types_are_silent() {
        let bad = [r"{}", r#"{"type":null}"#, r#"{"type":7}"#, "false", "[]"];
        for raw in bad {
            let (rows, coverage) = read_doc(&document(&[raw.to_owned()]), true);
            assert!(rows.is_empty());
            assert_eq!(reasons(&coverage), ["1 message lacks a type"], "{raw}");
        }
        let all = bad.map(str::to_owned);
        let (_, coverage) = read_doc(&document(&all), false);
        assert_eq!(reasons(&coverage), ["5 messages lack a type"]);
        let unknown = ["idle", "agent-switched", "model-switched", "future-event"]
            .map(|kind| format!(r#"{{"type":"{kind}","text":"never read"}}"#));
        let (rows, coverage) = read_doc(&document(&unknown), true);
        assert!(rows.is_empty() && coverage.is_empty());
        let disabled = r#"{"type":"assistant"}"#.to_owned();
        let (rows, coverage) = read_doc(&document(&[disabled]), false);
        assert!(rows.is_empty() && coverage.is_empty());
    }

    #[test]
    fn v2_uses_only_the_role_specific_text_location() {
        let cases = [
            (
                r#""type":"user","text":"kept user","content":[{"type":"text","text":"decoy"}]"#,
                "kept user",
            ),
            (
                r#""type":"assistant","text":"decoy","content":[{"type":"text","text":"kept reply"}]"#,
                "kept reply",
            ),
            (
                r#""type":"assistant","content":[{"type":"text","text":9},{"type":"reasoning","text":"secret"},{"type":"text","text":"kept"}]"#,
                "kept",
            ),
            (
                r#""type":"user","text":9,"content":[{"type":"text","text":"decoy"}]"#,
                "",
            ),
            (r#""type":"assistant","text":"decoy","content":{}"#, ""),
            (
                r#""type":"assistant","content":[],"error":{"message":"never a reply"}"#,
                "",
            ),
            (
                r#""type":"assistant","content":[{"type":"reasoning","text":"secret"}]"#,
                "",
            ),
        ];
        for (fields, want) in cases {
            let raw = format!(r#"{{"id":"msg_body","time":{{"created":1789549200500}},{fields}}}"#);
            let (rows, coverage) = read_doc(&document(&[raw]), true);
            assert!(coverage.is_empty(), "{fields}: {coverage:?}");
            let bodies: Vec<_> = rows.iter().map(|row| row.body.as_str()).collect();
            let expected: Vec<_> = if want.is_empty() { vec![] } else { vec![want] };
            assert_eq!(bodies, expected, "{fields}");
        }
        let old = r#"{"info":{"id":"msg_v1","role":"user","time":{"created":1789549200500}},"parts":[{"type":"text","text":"unsupported"}]}"#;
        let (rows, coverage) = read_doc(&document(&[old.to_owned()]), true);
        assert!(rows.is_empty());
        assert_eq!(reasons(&coverage), ["1 message lacks a type"]);
    }
}
