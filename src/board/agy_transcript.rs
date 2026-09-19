//! The Antigravity transcript reader: per-conversation transcript bytes in,
//! assistant rows out.
//!
//! PURE: no I/O, no env, no clock. Each conversation keeps its own transcript
//! at `brain/<uuid>/.system_generated/logs/transcript_full.jsonl` (falling
//! back to `transcript.jsonl`), so attribution needs no id match — the caller
//! located this seat's file already, and the seat's id rides
//! [`Streamed`](super::Streamed::for_seat) only as the row identity's first
//! half. One record per line; every record carries `step_index`, `source`,
//! `type`, `status` and second-precision `created_at`.
//!
//! Whitelist, per FIELD: `source == "MODEL"` and `type ==
//! "PLANNER_RESPONSE"` and `status == "DONE"` with non-blank `content`
//! renders, and only the `content` field is ever read. `GENERIC` is the
//! UNTYPED tool-result record — 914 of 925 sit immediately after a
//! `tool_calls` record and the same tool lands in its named type and in
//! `GENERIC` alike — so it drops silently with every tool type, and
//! `thinking` / `tool_calls` fields never reach a body even on a rendered
//! record. `USER_INPUT` is the human echo: history.jsonl stays the one human
//! source, so it is ignored here and never double-counted. A `RUNNING` or
//! otherwise non-final planner record is skipped and counted, never rendered.
//!
//! Identity is `(uuid, step)`: `file = "agy:<uuid>"`, `offset = step_index`
//! as a number, so same-second rows sort by numeric step and a rewrite that
//! reorders records keeps every row's name. Byte offsets would not survive:
//! the two sibling files carry the same steps at different lengths. A second
//! record repeating a step is damage, not a row — it is skipped and counted,
//! because two rows sharing one identity would silently delete a reply in
//! [`collect`](super::collect); only emitted rows take an identity, so a
//! record that yields no row never consumes its step. Stamps parse through
//! the one
//! [`Timestamp`](crate::time::Timestamp) spelling; an unparseable one counts.
//!
//! Measured 2026-09-19 over the local store (shapes and counts only): 41
//! conversations, 2940 records, `step_index` unique within every file,
//! planner-DONE-with-content 401. The truncated sibling clips `content` past
//! ~4 K chars and names it in `truncated_fields`; the caller says which
//! sibling it opened, and only the truncated leg counts clipped replies.

use std::collections::HashSet;

use super::{Coverage, LineBody, Role, Row, Streamed};
use crate::tool::ToolKind;

/// Read one streamed agy transcript: the door's lines in, assistant rows out.
///
/// The seat's conversation id rides the [`Streamed`](super::Streamed) value;
/// `truncated` says the caller opened `transcript.jsonl` rather than
/// `transcript_full.jsonl`, and only then does a record naming `content` in
/// `truncated_fields` count as clipped. With `assistant` off the read is
/// empty by construction — the leg never runs, so this arm only keeps the
/// reader honest when driven directly.
#[must_use]
pub fn read_stream(
    streamed: &Streamed,
    actor: &str,
    file: &str,
    source: ToolKind,
    truncated: bool,
) -> (Vec<Row>, Vec<Coverage>) {
    if !streamed.assistant {
        return (Vec::new(), Vec::new());
    }
    let mut sink = Sink {
        actor,
        file,
        source,
        truncated,
        rows: Vec::new(),
        coverage: Vec::new(),
        seen: HashSet::new(),
        not_records: 0,
        missing_step: 0,
        dup_step: 0,
        missing_ts: 0,
        skipped_status: 0,
        clipped: 0,
    };
    for line in &streamed.lines {
        if let LineBody::Full(bytes) = &line.body {
            sink.push_line(bytes);
        }
    }
    // The reader's own verdicts first — doc garbage, then record damage
    // in check order — then the door's.
    if sink.not_records > 0 {
        sink.cover(&counted(
            sink.not_records,
            "line is not a record",
            "lines are not records",
        ));
    }
    if sink.missing_step > 0 {
        sink.cover(&counted(
            sink.missing_step,
            "record lacks a step index",
            "records lack a step index",
        ));
    }
    if sink.dup_step > 0 {
        sink.cover(&counted(
            sink.dup_step,
            "record repeats a step index",
            "records repeat a step index",
        ));
    }
    if sink.missing_ts > 0 {
        sink.cover(&counted(
            sink.missing_ts,
            "record without a timestamp",
            "records without a timestamp",
        ));
    }
    if sink.skipped_status > 0 {
        sink.cover(&counted(
            sink.skipped_status,
            "reply skipped (not final)",
            "replies skipped (not final)",
        ));
    }
    if sink.clipped > 0 {
        sink.cover(&counted(
            sink.clipped,
            "reply clipped by the agy store",
            "replies clipped by the agy store",
        ));
    }
    if let Some(overlong) = super::overlong_coverage(streamed, actor) {
        sink.coverage.push(overlong);
    }
    if streamed.torn {
        sink.cover("torn last record");
    }
    (sink.rows, sink.coverage)
}

/// "1 record lacks a step index" / "7 records lack a step index": the one
/// singular/plural spelling this reader's counters share.
fn counted(count: u64, singular: &str, plural: &str) -> String {
    format!("{count} {}", if count == 1 { singular } else { plural })
}

/// Whether the truncated sibling clipped this record's `content`: the field
/// must be an array naming it. Any other shape is not a clip — hostile
/// `truncated_fields` never panics and never counts.
fn content_is_clipped(value: &crate::json::Value) -> bool {
    let Some(crate::json::Value::Arr(items)) = value.get("truncated_fields") else {
        return false;
    };
    items
        .iter()
        .any(|item| matches!(item, crate::json::Value::Str(name) if name == "content"))
}

/// One transcript's in-progress read.
struct Sink<'a> {
    actor: &'a str,
    file: &'a str,
    source: ToolKind,
    truncated: bool,
    rows: Vec<Row>,
    coverage: Vec<Coverage>,
    seen: HashSet<u64>,
    not_records: u64,
    missing_step: u64,
    dup_step: u64,
    missing_ts: u64,
    skipped_status: u64,
    clipped: u64,
}

impl Sink<'_> {
    fn cover(&mut self, reason: &str) {
        self.coverage.push(Coverage {
            actor: self.actor.to_owned(),
            reason: reason.to_owned(),
        });
    }

    /// Attempt one newline-terminated `Full` line: classify silently, count
    /// damage, emit only the whitelisted planner reply. The step identity is
    /// taken last, for an emitted row only — never for damage or a blank.
    fn push_line(&mut self, line: &[u8]) {
        // Not UTF-8, not JSON or not an object is not a record at all —
        // doc-level garbage, never per-record damage.
        let Ok(text) = str::from_utf8(line) else {
            self.not_records += 1;
            return;
        };
        let Ok(value) = crate::json::parse(text) else {
            self.not_records += 1;
            return;
        };
        if !matches!(value, crate::json::Value::Obj(_)) {
            self.not_records += 1;
            return;
        }
        if value.get_str("source") != Some("MODEL") {
            return;
        }
        if value.get_str("type") != Some("PLANNER_RESPONSE") {
            return;
        }
        if value.get_str("status") != Some("DONE") {
            self.skipped_status += 1;
            return;
        }
        let Some(step) = value.get("step_index").and_then(|step| match step {
            crate::json::Value::Num(index) => u64::try_from(*index).ok(),
            _ => None,
        }) else {
            self.missing_step += 1;
            return;
        };
        // A negative, float or oversized literal already counted above;
        // only a genuine `u64` reaches here, so no wrap, no saturating fold.
        let Some(ts) = value
            .get_str("created_at")
            .and_then(crate::time::Timestamp::parse_micros)
        else {
            self.missing_ts += 1;
            return;
        };
        let Some(body) = value
            .get_str("content")
            .map(str::trim)
            .filter(|body| !body.is_empty())
        else {
            return;
        };
        // Only emitted rows take an identity: a record that yields no row
        // must not consume its step, or the real reply at that step would
        // count as a duplicate and drop.
        if !self.seen.insert(step) {
            self.dup_step += 1;
            return;
        }
        if self.truncated && content_is_clipped(&value) {
            self.clipped += 1;
        }
        self.rows.push(Row {
            ts,
            actor: self.actor.to_owned(),
            role: Role::Assistant,
            body: body.to_owned(),
            source: self.source,
            file: self.file.to_owned(),
            offset: step,
            generation: 0,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::read_stream;
    use crate::board::{Role, Splitter, collect};

    const ACTOR: &str = "s:seat";
    const SEAT: &str = "0199c0de-ffff-4890-abcd-ef0123456789";
    const FILE: &str = "agy:0199c0de-ffff-4890-abcd-ef0123456789";
    const STAMP: &str = "2026-09-16T09:00:00Z";
    const MICROS: i64 = 1_789_549_200_000_000;

    fn esc(text: &str) -> String {
        text.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    }

    /// One synthetic transcript record; `extra` carries payload fields such
    /// as `,"content":"…"`, already comma-led, `step`/`stamp` raw literals.
    fn rec(step: &str, source: &str, typ: &str, status: &str, stamp: &str, extra: &str) -> String {
        format!(
            r#"{{"step_index":{step},"source":"{source}","type":"{typ}","status":"{status}","created_at":"{stamp}"{extra}}}"#
        )
    }

    fn content(text: &str) -> String {
        format!(r#","content":"{}""#, esc(text))
    }

    /// One planner reply at the fixed stamp; odd shapes use [`rec`] or the
    /// one-field helpers below, so every call site fits one line.
    fn pr(step: &str, body: &str) -> String {
        rec(
            step,
            "MODEL",
            "PLANNER_RESPONSE",
            "DONE",
            STAMP,
            &content(body),
        )
    }

    fn pr_status(step: &str, status: &str, body: &str) -> String {
        rec(
            step,
            "MODEL",
            "PLANNER_RESPONSE",
            status,
            STAMP,
            &content(body),
        )
    }

    fn pr_stamp(step: &str, stamp: &str, body: &str) -> String {
        rec(
            step,
            "MODEL",
            "PLANNER_RESPONSE",
            "DONE",
            stamp,
            &content(body),
        )
    }

    fn typed(step: &str, source: &str, typ: &str, body: &str) -> String {
        rec(step, source, typ, "DONE", STAMP, &content(body))
    }

    fn read_lines(
        lines: &[&str],
        assistant: bool,
        truncated: bool,
    ) -> (Vec<crate::board::Row>, Vec<crate::board::Coverage>) {
        let mut bytes = lines.join("\n").into_bytes();
        bytes.push(b'\n');
        let mut splitter = Splitter::new();
        splitter.feed(&bytes);
        read_stream(
            &splitter.finish().for_seat(SEAT).with_assistant(assistant),
            ACTOR,
            FILE,
            crate::tool::ToolKind::Agy,
            truncated,
        )
    }

    fn reasons(coverage: &[crate::board::Coverage]) -> Vec<&str> {
        coverage.iter().map(|item| item.reason.as_str()).collect()
    }

    /// Drive one `board_agy` seed exactly the way the fuzz target decodes
    /// it: chunk size, flag word, then the stream through the ONE splitter
    /// into the flagged entry.
    fn drive_seed(seed: &str) -> (Vec<crate::board::Row>, Vec<crate::board::Coverage>) {
        let bytes = seed.as_bytes();
        let (&size, rest) = bytes.split_first().expect("chunk byte");
        let (&flag, stream) = rest.split_first().expect("flag byte");
        let mut splitter = Splitter::new();
        for piece in stream.chunks(usize::from(size) % 256 + 1) {
            splitter.feed(piece);
        }
        let streamed = splitter
            .finish()
            .for_seat(SEAT)
            .with_assistant(flag & 1 == 1)
            .with_assistant_rows_found(flag & 8 == 8)
            .with_assistant_read_once(flag & 16 == 16);
        if flag & 2 == 0 {
            crate::board::agy::read_stream(
                &streamed,
                ACTOR,
                "fuzz.jsonl",
                crate::tool::ToolKind::Agy,
            )
        } else {
            read_stream(
                &streamed,
                ACTOR,
                FILE,
                crate::tool::ToolKind::Agy,
                flag & 4 == 4,
            )
        }
    }

    fn flag_of(seed: &str) -> u8 {
        seed.as_bytes().get(1).copied().expect("flag byte")
    }

    #[test]
    fn the_planner_reply_renders_with_step_identity_and_second_micros() {
        let line = pr("7", "synthetic reply");
        let (rows, coverage) = read_lines(&[&line], true, false);
        assert!(coverage.is_empty());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, "synthetic reply");
        assert_eq!(rows[0].role, Role::Assistant);
        assert_eq!(rows[0].ts, MICROS, "seconds become micros, exactly");
        assert_eq!(rows[0].file, FILE);
        assert_eq!(rows[0].offset, 7, "identity is the step, never bytes");
    }

    #[test]
    fn tool_results_and_system_records_drop_silently() {
        let generic = typed("1", "MODEL", "GENERIC", "tool output");
        let named = typed("2", "MODEL", "RUN_COMMAND", "tool output");
        let system = typed("3", "SYSTEM", "SYSTEM_MESSAGE", "system words");
        let checkpoint = typed("4", "SYSTEM", "CHECKPOINT", "checkpoint words");
        let bare = rec("5", "MODEL", "PLANNER_RESPONSE", "DONE", STAMP, "");
        let empty = pr("6", "   ");
        let lines: Vec<String> = [generic, named, system, checkpoint, bare, empty].to_vec();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let (rows, coverage) = read_lines(&refs, true, false);
        assert!(rows.is_empty(), "no tool output, no system text, no blanks");
        assert!(coverage.is_empty(), "classified drops stay silent");
    }

    #[test]
    fn thinking_and_tool_calls_never_reach_a_body() {
        let line = format!(
            "{}{}",
            pr("9", "visible narration").trim_end_matches('}'),
            r#","thinking":"hidden chain of thought","tool_calls":[{"name":"run_command","args":{}}]}"#,
        );
        let (rows, coverage) = read_lines(&[&line], true, false);
        assert!(coverage.is_empty());
        assert_eq!(rows.len(), 1, "narration beside a call still renders");
        assert_eq!(rows[0].body, "visible narration");
        assert!(
            !rows[0].body.contains("hidden") && !rows[0].body.contains("run_command"),
            "thinking and calls never leak: {}",
            rows[0].body
        );
    }

    #[test]
    fn the_human_echo_and_the_flag_off_read_stay_silent() {
        let human = typed("0", "USER_EXPLICIT", "USER_INPUT", "human words");
        let reply = pr("1", "synthetic reply");
        let (rows, coverage) = read_lines(&[&human, &reply], true, false);
        assert_eq!(rows.len(), 1, "the echo is history's row, never ours");
        assert_eq!(rows[0].body, "synthetic reply");
        assert!(coverage.is_empty());
        let (rows, coverage) = read_lines(&[&human, &reply], false, false);
        assert!(rows.is_empty() && coverage.is_empty(), "flag off: nothing");
    }
    #[test]
    fn per_record_damage_is_counted_and_the_rest_still_reads() {
        let no_step = r#"{"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-09-16T09:00:00Z","content":"lost"}"#.to_owned();
        let contentless_running = rec("11", "MODEL", "PLANNER_RESPONSE", "RUNNING", STAMP, "");
        let owned: Vec<String> = [
            "{ not json".to_owned(),
            "[1,2]".to_owned(),
            no_step,
            pr("-1", "lost"),
            pr("1.5", "lost"),
            pr("9223372036854775808", "lost"),
            pr("3", "first"),
            pr("3", "second"),
            pr_stamp("4", "null", "lost"),
            pr_stamp("5", "yesterday", "lost"),
            pr_status("6", "RUNNING", "lost"),
            pr_status("7", "INVALID", "lost"),
            contentless_running,
            pr("8", "kept"),
            pr("9", "   "),
            pr("9", "second chance"),
            pr_stamp("10", "yesterday", "lost"),
            pr("10", "stamp second chance"),
        ]
        .to_vec();
        let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
        let (rows, coverage) = read_lines(&refs, true, false);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].body, "first");
        assert_eq!(rows[1].body, "kept");
        assert_eq!(rows[2].body, "second chance");
        assert_eq!(rows[3].body, "stamp second chance");
        assert_eq!(
            reasons(&coverage),
            [
                "2 lines are not records",
                "4 records lack a step index",
                "1 record repeats a step index",
                "3 records without a timestamp",
                "3 replies skipped (not final)",
            ]
        );
        // Non-UTF8 bytes ride below `&str`: one hostile line plus a good
        // one, fed as bytes, counts the garbage and keeps the row.
        let mut hostile = b"\xff\xfe\n".to_vec();
        hostile.extend_from_slice(pr("12", "kept").as_bytes());
        hostile.push(b'\n');
        let mut splitter = Splitter::new();
        splitter.feed(&hostile);
        let (rows, coverage) = read_stream(
            &splitter.finish().for_seat(SEAT).with_assistant(true),
            ACTOR,
            FILE,
            crate::tool::ToolKind::Agy,
            false,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, "kept");
        assert_eq!(reasons(&coverage), ["1 line is not a record"]);
        // I3: each hostile step shape counts on its own, never wraps into a
        // colliding offset.
        for bad in ["-1", "1.5", "9223372036854775808"] {
            let line = pr(bad, "lost");
            let (rows, coverage) = read_lines(&[&line], true, false);
            assert!(rows.is_empty(), "step {bad} yields no row");
            assert_eq!(
                reasons(&coverage),
                ["1 record lacks a step index"],
                "step {bad} counts"
            );
        }
    }

    #[test]
    fn clipped_counts_only_on_the_truncated_leg_and_never_panics() {
        let clipped = format!(
            "{}{}",
            pr("1", "cut short").trim_end_matches('}'),
            r#","truncated_fields":["content"]}"#,
        );
        let (rows, coverage) = read_lines(&[&clipped], true, true);
        assert_eq!(rows.len(), 1, "clipped text still renders");
        assert_eq!(rows[0].body, "cut short");
        assert_eq!(reasons(&coverage), ["1 reply clipped by the agy store"]);
        let (rows, coverage) = read_lines(&[&clipped], true, false);
        assert_eq!(rows.len(), 1);
        assert!(coverage.is_empty(), "the full leg never counts clips");
        // N7: hostile `truncated_fields` shapes: counted never, panics never.
        for hostile in [
            r#""truncated_fields":"content""#,
            r#""truncated_fields":[1,2]"#,
            r#""truncated_fields":[["content"]]"#,
            r#""truncated_fields":["thinking"]"#,
        ] {
            let line = format!(
                "{},{} }}",
                pr("2", "whole").trim_end_matches('}'),
                hostile.trim(),
            );
            let line = line.replace(" }}", "}");
            let (rows, coverage) = read_lines(&[&line], true, true);
            assert_eq!(rows.len(), 1, "{hostile} still renders");
            assert!(coverage.is_empty(), "{hostile} never counts");
        }
    }

    #[test]
    fn torn_and_overlong_mirror_the_door() {
        let good = pr("1", "kept");
        let torn = format!("{good}\n{}", pr("2", "torn"));
        // NOTE: `read_lines` terminates its input, so the torn arm is
        // pinned through the splitter directly, unterminated tail and all.
        let mut splitter = Splitter::new();
        splitter.feed(torn.as_bytes());
        let (rows, coverage) = read_stream(
            &splitter.finish().for_seat(SEAT).with_assistant(true),
            ACTOR,
            FILE,
            crate::tool::ToolKind::Agy,
            false,
        );
        assert_eq!((rows.len(), rows[0].body.as_str()), (1, "kept"));
        assert_eq!(reasons(&coverage), ["torn last record"]);

        let big = "x".repeat(1024 * 1024 + 1);
        let (rows, coverage) = read_lines(&[&big], true, false);
        assert!(rows.is_empty());
        assert_eq!(
            reasons(&coverage),
            ["1 line exceeds the 1 MiB cap (largest 1048577 bytes)"]
        );
    }

    #[test]
    fn two_reads_of_reordered_steps_dedup_to_one_row_each() {
        let one = pr("1", "first");
        let two = pr("2", "second");
        let (first, _) = read_lines(&[&one, &two], true, false);
        let (second, _) = read_lines(&[&two, &one], true, false);
        let mut both = first;
        both.extend(second);
        let merged = collect(both);
        assert_eq!(merged.len(), 2, "same steps collapse across a rewrite");
        assert_eq!(merged[0].body, "first");
        assert_eq!(merged[1].body, "second");
    }

    #[test]
    fn board_agy_fuzz_seeds_reach_their_named_transcript_paths() {
        let basic = include_str!("../../fuzz/seeds/board_agy/transcript-pr-basic");
        let non_done = include_str!("../../fuzz/seeds/board_agy/transcript-non-done");
        let bad_steps = include_str!("../../fuzz/seeds/board_agy/transcript-bad-steps");
        let missing_stamp = include_str!("../../fuzz/seeds/board_agy/transcript-missing-stamp");
        let truncated = include_str!("../../fuzz/seeds/board_agy/transcript-truncated");
        let hostile = include_str!("../../fuzz/seeds/board_agy/transcript-hostile-trunc");
        // The pin replicates the target's flag decode, so it also pins the
        // decode itself: a changed mask breaks here, not silently in a lane
        // outside the product graph.
        let target = include_str!("../../fuzz/fuzz_targets/board_agy.rs");
        for mask in [
            "flag & 2 == 0",
            "flag & 4 == 4",
            "flag & 8 == 8",
            "flag & 16 == 16",
        ] {
            assert!(
                target.contains(mask),
                "the pin decodes what the target decodes"
            );
        }
        // Every transcript seed selects the transcript entry; only the one
        // named truncated claims the truncated leg.
        for (name, seed) in [
            ("basic", basic),
            ("non-done", non_done),
            ("bad-steps", bad_steps),
            ("missing-stamp", missing_stamp),
            ("hostile-trunc", hostile),
        ] {
            assert_eq!(flag_of(seed), b'#', "{name} drives the transcript entry");
        }
        assert_eq!(
            flag_of(truncated),
            b'\'',
            "truncated claims the clipped leg"
        );
        let (rows, coverage) = drive_seed(basic);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body, "seed narration beside a call");
        assert!(coverage.is_empty());
        let (rows, coverage) = drive_seed(non_done);
        assert_eq!(rows.len(), 1);
        assert_eq!(reasons(&coverage), ["1 reply skipped (not final)"]);
        let (rows, coverage) = drive_seed(bad_steps);
        assert!(rows.is_empty());
        assert_eq!(reasons(&coverage), ["3 records lack a step index"]);
        let (rows, coverage) = drive_seed(missing_stamp);
        assert!(rows.is_empty());
        assert_eq!(reasons(&coverage), ["1 record without a timestamp"]);
        let (rows, coverage) = drive_seed(truncated);
        assert_eq!(rows.len(), 1);
        assert_eq!(reasons(&coverage), ["1 reply clipped by the agy store"]);
        let (rows, coverage) = drive_seed(hostile);
        assert_eq!(rows.len(), 3, "hostile truncation shapes still render");
        assert!(coverage.is_empty(), "and never count");
    }

    #[test]
    fn same_second_steps_sort_numeric_never_lexical() {
        let two = pr("2", "two");
        let ten = pr("10", "ten");
        let (rows, _) = read_lines(&[&ten, &two], true, false);
        let merged = collect(rows);
        assert_eq!(merged.len(), 2);
        assert_eq!(
            (merged[0].body.as_str(), merged[1].body.as_str()),
            ("two", "ten"),
            "offset 2 sorts before offset 10; `#10 < #2` would scramble"
        );
    }
}
