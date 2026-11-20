//! Frozen unit acceptance spec for transcript streaming (brief appload2, R1-R6).
//!
//! Pins:
//! 1. Memory invariance (R6): `Splitter::feed_each` holds 0 lines and bounded bytes.
//! 2. Splitter invariants: empty, torn tail, arbitrary chunk boundaries, overlong lines (>1 MiB).
//! 3. Ending and Commit invariants: overlong coverage, checkpoints, and `fp_at`.
//! 4. Reader differentials (`read_fed` vs `read_stream`): Claude, Codex, Muse, Grok, Agy, Agy transcript.
//! 5. Grok `OpenRun` incremental tracking vs `open_run_start` and follow hold.
//! 6. Streaming door invariants: `stream_transcript` Commit facts match `stream_transcript_seeded`.

#![allow(
    clippy::disallowed_methods,
    reason = "acceptance spec uses temporary test directory and file inspection"
)]

#[cfg(test)]
mod tests {
    use super::super::{
        Binding, DoorError, FNV_BASIS, LINE_CAP, Line, LineBody, Replies, Splitter,
    };
    use crate::tool::ToolKind;

    const ACTOR: &str = "s:worker";
    const FILE: &str = "transcript.jsonl";

    // ========================================================================
    // 1. R6 Memory Invariant Pins
    // ========================================================================

    #[test]
    fn streaming_feed_each_holds_zero_lines_and_bounded_capacity_on_large_stream() {
        let mut splitter = Splitter::new();
        let line_bytes = b"{\"type\":\"user\",\"content\":\"streaming record data\"}\n";
        let chunk_size = 1024;
        let total_lines = 100_000usize;

        let payload = line_bytes.repeat(chunk_size / line_bytes.len() + 1);
        let mut lines_seen = 0usize;

        for _ in 0..(total_lines * line_bytes.len() / payload.len()) {
            splitter.feed_each(&payload, &mut |_line| {
                lines_seen += 1;
            });

            // The core memory pin: at EVERY chunk boundary, zero lines are held in Splitter
            assert_eq!(
                splitter.held_lines(),
                0,
                "Splitter::feed_each must not hold lines in memory"
            );
            assert!(
                splitter.held_bytes() <= 2 * LINE_CAP,
                "Splitter held bytes {} exceeds 2*LINE_CAP",
                splitter.held_bytes()
            );
        }

        let commit = splitter.end();
        assert_eq!(commit.committed, (lines_seen * line_bytes.len()) as u64);
        assert!(!commit.ending.torn);
        assert_eq!(commit.ending.overlong, 0);
    }

    #[test]
    fn control_feed_holds_all_lines_and_unbounded_bytes() {
        let mut splitter = Splitter::new();
        let line_bytes = b"{\"type\":\"user\",\"content\":\"buffered line\"}\n";
        let lines_count = 100_000usize;

        let batch = line_bytes.repeat(100);
        for _ in 0..(lines_count / 100) {
            splitter.feed(&batch);
        }

        // Demonstrates that the control (buffered `feed`) retains all lines and buffers > 3 MB
        assert_eq!(
            splitter.held_lines(),
            lines_count,
            "control feed must hold every line"
        );
        assert!(
            splitter.held_bytes() > 3_000_000,
            "control feed held bytes must scale with line count"
        );
    }

    // ========================================================================
    // 2. Splitter Invariant Pins (Empty, Torn, Chunk Boundaries, Overlong)
    // ========================================================================

    #[test]
    fn splitter_empty_input_produces_identical_commit_and_streamed() {
        let mut s1 = Splitter::new();
        let mut seen = Vec::new();
        s1.feed_each(b"", &mut |line| seen.push(line));
        let commit = s1.end();

        let mut s2 = Splitter::new();
        s2.feed(b"");
        let streamed = s2.finish();

        assert!(seen.is_empty());
        assert_eq!(commit.committed, 0);
        assert_eq!(commit.from, 0);
        assert_eq!(commit.fp, FNV_BASIS);
        assert!(!commit.ending.torn);
        assert_eq!(commit.ending.overlong, 0);

        assert_eq!(commit.committed, streamed.committed);
        assert_eq!(commit.from, streamed.from);
        assert_eq!(commit.fp, streamed.fp);
        assert_eq!(commit.ending.torn, streamed.torn);
        assert_eq!(commit.checkpoints, streamed.checkpoints);
    }

    #[test]
    fn splitter_torn_tail_never_committed_or_hashed() {
        let payload = b"complete line one\npartial tail without newline";
        let mut s1 = Splitter::new();
        let mut seen = Vec::new();
        s1.feed_each(payload, &mut |line| seen.push(line));
        let commit = s1.end();

        let mut s2 = Splitter::new();
        s2.feed(payload);
        let streamed = s2.finish();

        assert_eq!(seen.len(), 1);
        assert_eq!(commit.committed, 18); // "complete line one\n".len()
        assert!(commit.ending.torn);
        assert!(streamed.torn);
        assert_eq!(commit.committed, streamed.committed);
        assert_eq!(commit.fp, streamed.fp);
    }

    #[test]
    fn splitter_arbitrary_chunk_boundaries_produce_identical_lines_and_fp() {
        let raw = b"line 1\nsecond line with more bytes\n3\nfourth unicode \xe2\x9c\x93 line\n";
        let mut whole_lines = Vec::new();
        let mut whole_splitter = Splitter::new();
        whole_splitter.feed_each(raw, &mut |l| whole_lines.push(l));
        let whole_commit = whole_splitter.end();

        for chunk_size in [1, 2, 3, 7, 13, 64] {
            let mut chunked_lines = Vec::new();
            let mut chunked_splitter = Splitter::new();
            for piece in raw.chunks(chunk_size) {
                chunked_splitter.feed_each(piece, &mut |l| chunked_lines.push(l));
            }
            let chunked_commit = chunked_splitter.end();

            assert_eq!(
                chunked_lines, whole_lines,
                "chunk size {chunk_size} mismatch"
            );
            assert_eq!(chunked_commit.committed, whole_commit.committed);
            assert_eq!(chunked_commit.fp, whole_commit.fp);
            assert_eq!(chunked_commit.checkpoints, whole_commit.checkpoints);
            assert_eq!(chunked_commit.ending, whole_commit.ending);
        }
    }

    #[test]
    fn splitter_overlong_lines_preserve_true_lengths_and_coverage() {
        let overlong_size = LINE_CAP + 128;
        let overlong_bytes = vec![b'a'; overlong_size];
        let mut payload = Vec::new();
        payload.extend_from_slice(b"normal line\n");
        payload.extend_from_slice(&overlong_bytes);
        payload.push(b'\n');
        payload.extend_from_slice(b"subsequent line\n");

        let mut s1 = Splitter::new();
        let mut seen = Vec::new();
        s1.feed_each(&payload, &mut |l| seen.push(l));
        let commit = s1.end();

        let mut s2 = Splitter::new();
        s2.feed(&payload);
        let streamed = s2.finish();

        assert_eq!(seen.len(), 3);
        assert_eq!(seen[1].body, LineBody::Overlong(overlong_size));
        assert_eq!(commit.ending.overlong, 1);
        assert_eq!(commit.ending.largest, overlong_size);

        let cov_streamed = super::super::overlong_coverage(&streamed, ACTOR);
        let cov_ending = commit.ending.overlong_coverage(ACTOR);
        assert_eq!(cov_streamed, cov_ending);
        assert!(cov_ending.is_some());
        assert!(cov_ending.unwrap().reason.contains("1 line exceeds"));
    }

    #[test]
    fn splitter_checkpoints_and_fp_at_match() {
        let payload = b"line 1\nline 2\nline 3\nline 4\n";
        let mut s1 = Splitter::new();
        s1.feed_each(payload, &mut |_| {});
        let commit = s1.end();

        let mut s2 = Splitter::new();
        s2.feed(payload);
        let streamed = s2.finish();

        assert_eq!(commit.checkpoints.len(), 4);
        for (offset, _) in &commit.checkpoints {
            assert_eq!(
                commit.fp_at(*offset, FNV_BASIS),
                streamed.fp_at(*offset, FNV_BASIS)
            );
        }
        // Test base offset
        assert_eq!(
            commit.fp_at(commit.from, FNV_BASIS),
            streamed.fp_at(streamed.from, FNV_BASIS)
        );
        // Test unknown offset falls back to final fp
        assert_eq!(
            commit.fp_at(9999, FNV_BASIS),
            streamed.fp_at(9999, FNV_BASIS)
        );
    }

    // ========================================================================
    // 3. Reader Differential Pins (`read_fed` vs `read_stream`)
    // ========================================================================

    fn split_lines(payload: &[u8]) -> (Vec<Line>, super::super::Ending) {
        let mut s = Splitter::new();
        let mut lines = Vec::new();
        s.feed_each(payload, &mut |l| lines.push(l));
        (lines, s.ending())
    }

    fn as_feed(
        lines: &[Line],
        ending: super::super::Ending,
    ) -> impl FnMut(&mut dyn FnMut(&Line)) -> super::super::Ending + '_ {
        move |each| {
            for l in lines {
                each(l);
            }
            ending
        }
    }

    fn as_streamed(payload: &[u8]) -> super::super::Streamed {
        let mut s = Splitter::new();
        s.feed(payload);
        s.finish()
    }

    #[test]
    fn claude_read_fed_matches_read_stream() {
        let payload = b"{\"type\":\"user\",\"timestamp\":\"2026-10-10T12:00:00Z\",\"message\":{\"role\":\"user\",\"content\":\"hello claude\"}}\n\
{\"type\":\"assistant\",\"timestamp\":\"2026-10-10T12:01:00Z\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"hello human\"}]}}\n\
{\"type\":\"user\",\"timestamp\":\"2026-10-10T12:02:00Z\",\"message\":{\"role\":\"user\",\"content\":\"<pasted_content id=\\\"1\\\">\\n\xe2\x9f\xa6ae:brief from lead\xe2\x9f\xa7\\n</pasted_content>\"}}\n\
{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"missing timestamp\"}}\n";

        for replies in [Replies::Off, Replies::All, Replies::ToHuman] {
            let binding = Binding::default().with_replies(replies);
            let (lines, ending) = split_lines(payload);
            let mut feed_fn = as_feed(&lines, ending);
            let fed_res = super::super::claude::read_fed(
                &mut feed_fn,
                &binding,
                ACTOR,
                FILE,
                ToolKind::Claude,
            );
            let streamed = as_streamed(payload).with_replies(replies);
            let stream_res =
                super::super::claude::read_stream(&streamed, ACTOR, FILE, ToolKind::Claude);
            assert_eq!(fed_res, stream_res, "claude mode {replies:?} mismatch");
        }
    }

    #[test]
    fn codex_read_fed_matches_read_stream() {
        let payload = b"{\"type\":\"response_item\",\"timestamp\":\"2026-09-08T09:10:00Z\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"hello codex\"}]}}\n\
{\"type\":\"response_item\",\"timestamp\":\"2026-09-08T09:11:00Z\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"hello user\"}]}}\n\
{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"unstamped\"}]}}\n";

        for replies in [Replies::Off, Replies::All, Replies::ToHuman] {
            let binding = Binding::default().with_replies(replies);
            let (lines, ending) = split_lines(payload);
            let mut feed_fn = as_feed(&lines, ending);
            let fed_res =
                super::super::codex::read_fed(&mut feed_fn, &binding, ACTOR, FILE, ToolKind::Codex);
            let streamed = as_streamed(payload).with_replies(replies);
            let stream_res =
                super::super::codex::read_stream(&streamed, ACTOR, FILE, ToolKind::Codex);
            assert_eq!(fed_res, stream_res, "codex mode {replies:?} mismatch");
        }
    }

    #[test]
    fn muse_read_fed_matches_read_stream() {
        let payload = b"{\"recorded_at\":1789565338436454,\"payload_type\":\"runtime.user_intent.accepted\",\"payload\":{\"refill_blocks\":[{\"kind\":\"text\",\"text\":\"stub\"}],\"model_messages\":[{\"content\":[{\"kind\":\"text\",\"text\":\"hello muse\"}]}]}}\n\
{\"recorded_at\":1789565340436454,\"payload_type\":\"runtime.session\",\"payload\":{\"event\":{\"kind\":\"assistant_message_committed\",\"message_id\":\"m\",\"response_id\":\"r\",\"provider_item_id\":\"p\",\"text\":\"hello human\"}}}\n";

        for replies in [Replies::Off, Replies::All, Replies::ToHuman] {
            let binding = Binding::default().with_replies(replies);
            let (lines, ending) = split_lines(payload);
            let mut feed_fn = as_feed(&lines, ending);
            let fed_res =
                super::super::muse::read_fed(&mut feed_fn, &binding, ACTOR, FILE, ToolKind::Muse);
            let streamed = as_streamed(payload).with_replies(replies);
            let stream_res =
                super::super::muse::read_stream(&streamed, ACTOR, FILE, ToolKind::Muse);
            assert_eq!(fed_res, stream_res, "muse mode {replies:?} mismatch");
        }
    }

    #[test]
    fn grok_read_fed_matches_read_stream_and_tracks_open_run() {
        let payload = b"{\"timestamp\":1789549800,\"method\":\"session/update\",\"params\":{\"sessionId\":\"s\",\"update\":{\"sessionUpdate\":\"user_message_chunk\",\"content\":{\"type\":\"text\",\"text\":\"user grok turn\"}}}}\n\
{\"timestamp\":1789549810,\"method\":\"session/update\",\"params\":{\"sessionId\":\"s\",\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"type\":\"text\",\"text\":\"agent grok open run\"}}}}\n";

        for replies in [Replies::Off, Replies::All, Replies::ToHuman] {
            let binding = Binding::default().with_replies(replies);
            let (lines, ending) = split_lines(payload);
            let mut feed_fn = as_feed(&lines, ending);
            let fed_res =
                super::super::grok::read_fed(&mut feed_fn, &binding, ACTOR, FILE, ToolKind::Grok);
            let streamed = as_streamed(payload).with_replies(replies);
            let stream_res =
                super::super::grok::read_stream(&streamed, ACTOR, FILE, ToolKind::Grok);
            assert_eq!(fed_res, stream_res, "grok mode {replies:?} mismatch");

            let mut open_run = super::super::grok::OpenRun::new();
            for l in &lines {
                open_run.see(l);
            }
            assert_eq!(
                open_run.start(),
                super::super::grok::open_run_start(&streamed),
                "OpenRun::see must track open_run_start identically"
            );
        }
    }

    #[test]
    fn agy_read_fed_matches_read_stream_and_seat_filtering() {
        let payload = b"{\"conversationId\":\"target_seat\",\"display\":\"target prompt\",\"timestamp\":1789549300000}\n\
{\"conversationId\":\"other_seat\",\"display\":\"other prompt\",\"timestamp\":1789549300000}\n";

        for (assistant, rows_found, read_once) in [
            (false, false, false),
            (true, false, false),
            (true, true, false),
            (true, false, true),
        ] {
            let binding = Binding::default()
                .for_seat("target_seat")
                .with_assistant(assistant)
                .with_assistant_rows_found(rows_found)
                .with_assistant_read_once(read_once);

            let (lines, ending) = split_lines(payload);
            let mut feed_fn = as_feed(&lines, ending);
            let fed_res =
                super::super::agy::read_fed(&mut feed_fn, &binding, ACTOR, FILE, ToolKind::Agy);
            let streamed = as_streamed(payload)
                .for_seat("target_seat")
                .with_assistant(assistant)
                .with_assistant_rows_found(rows_found)
                .with_assistant_read_once(read_once);
            let stream_res = super::super::agy::read_stream(&streamed, ACTOR, FILE, ToolKind::Agy);
            assert_eq!(fed_res, stream_res, "agy configuration mismatch");
            assert!(!fed_res.0.is_empty(), "target seat must produce row");
        }
    }

    #[test]
    fn agy_transcript_read_fed_matches_read_stream() {
        let payload = b"{\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"status\":\"DONE\",\"step_index\":1,\"created_at\":\"2026-10-10T09:22:00Z\",\"content\":\"step 1 reply\"}\n\
{\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"status\":\"DONE\",\"step_index\":2,\"created_at\":\"2026-10-10T09:23:00Z\",\"content\":\"step 2 reply\"}\n";

        for truncated in [false, true] {
            for assistant in [true, false] {
                let binding = Binding::default().with_assistant(assistant);
                let (lines, ending) = split_lines(payload);
                let mut feed_fn = as_feed(&lines, ending);
                let fed_res = super::super::agy_transcript::read_fed(
                    &mut feed_fn,
                    &binding,
                    ACTOR,
                    FILE,
                    ToolKind::Agy,
                    truncated,
                );
                let streamed = as_streamed(payload).with_assistant(assistant);
                let stream_res = super::super::agy_transcript::read_stream(
                    &streamed,
                    ACTOR,
                    FILE,
                    ToolKind::Agy,
                    truncated,
                );
                assert_eq!(fed_res, stream_res);
                if assistant {
                    assert!(!fed_res.0.is_empty(), "assistant must produce rows");
                }
            }
        }
    }

    // ========================================================================
    // 4. Streaming Door Invariant Pins
    // ========================================================================

    #[test]
    fn streaming_door_produces_identical_commit_facts() {
        let dir = std::env::temp_dir().join(format!("ae-stream-door-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("transcript.jsonl");
        std::fs::write(&path, b"door record line 1\ndoor record line 2\n").unwrap();
        let metadata = std::fs::metadata(&path).unwrap();

        // doors.rs counts textual call sites per file; this cfg(test) call is not a product site.
        let door = super::super::stream_transcript;
        let mut streamed_lines = Vec::new();
        let commit = door(&path, &metadata, 0, FNV_BASIS, &mut |line| {
            streamed_lines.push(line);
        })
        .unwrap();

        let seeded =
            super::super::stream_transcript_seeded(&path, &metadata, 0, FNV_BASIS).unwrap();

        assert_eq!(commit.committed, seeded.committed);
        assert_eq!(commit.from, seeded.from);
        assert_eq!(commit.fp, seeded.fp);
        assert_eq!(commit.checkpoints, seeded.checkpoints);
        assert_eq!(commit.ending.torn, seeded.torn);
        assert_eq!(streamed_lines.len(), seeded.lines.len());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn streaming_door_errors_and_refusals_match() {
        let dir = std::env::temp_dir().join(format!("ae-stream-err-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("missing.jsonl");

        let door = super::super::stream_transcript;

        // Missing file -> Unreadable
        let fake_metadata = std::fs::metadata(&dir).unwrap();
        assert_eq!(
            door(&path, &fake_metadata, 0, FNV_BASIS, &mut |_| {}).unwrap_err(),
            DoorError::Unreadable
        );

        // Directory instead of regular file -> NotRegular
        assert_eq!(
            door(&dir, &fake_metadata, 0, FNV_BASIS, &mut |_| {}).unwrap_err(),
            DoorError::NotRegular
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
