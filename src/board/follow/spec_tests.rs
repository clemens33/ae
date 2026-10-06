//! Frozen recovery pins: successful identical bytes replace read-door doubt
//! with the parser evidence those bytes actually carry.

#[cfg(test)]
mod tests {
    use super::super::{Follow, Loaded, Located, Plan, Snapshot};
    use crate::board::{Observation, Splitter};
    use crate::tool::ToolKind;
    use std::time::{Duration, SystemTime};

    const ACTOR: &str = "quiet:lead";

    fn located(len: usize, tick: u64) -> Located {
        Located {
            identity: (1, 7),
            len: len as u64,
            mtime: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(tick)),
        }
    }

    fn snapshot(
        seen: Located,
        streamed: Option<Result<crate::board::Streamed, &'static str>>,
    ) -> Snapshot {
        Snapshot {
            actor: ACTOR.to_owned(),
            located: Ok(Loaded {
                file: "synthetic#1:7".to_owned(),
                source: ToolKind::Claude,
                observed: seen,
            }),
            streamed,
            passive: None,
        }
    }

    fn read(follow: &mut Follow, bytes: &str, tick: u64) -> Observation {
        let seen = located(bytes.len(), tick);
        let streamed = match follow.plan(ACTOR, &seen) {
            Plan::Hold => None,
            Plan::Read(from) => {
                // Every successful read in these scenarios starts at zero;
                // the synthetic append refuses before any bytes are supplied.
                assert_eq!(from, 0, "first sight or verification of restored bytes");
                let mut splitter = Splitter::new();
                splitter.feed(bytes.as_bytes());
                Some(Ok(splitter.finish()))
            }
        };
        follow.step(vec![snapshot(seen, streamed)])
    }

    fn failed_append(follow: &mut Follow, original_len: usize) {
        let failure = located(original_len + 1, 2);
        let refused = follow.step(vec![snapshot(failure, Some(Err("transcript unreadable")))]);
        assert!(
            refused
                .coverage
                .iter()
                .any(|gap| gap.reason == "transcript unreadable")
        );
        assert!(
            follow
                .standing()
                .iter()
                .any(|gap| gap.reason == "transcript unreadable")
        );
    }

    fn first() -> String {
        "{\"type\":\"user\",\"timestamp\":\"2026-10-03T18:00:00Z\",\"message\":{\"role\":\"user\",\"content\":\"FIRST\"}}\n".to_owned()
    }

    #[test]
    fn quiet_verified_touch_after_failed_append_clears_read_doubt() {
        let bytes = first();
        let mut follow = Follow::seeded(&[], &[], None);
        let initial = read(&mut follow, &bytes, 1);
        assert_eq!(initial.rows.len(), 1);
        assert!(initial.coverage.is_empty());
        failed_append(&mut follow, bytes.len());
        let recovered = read(&mut follow, &bytes, 3);
        assert!(
            recovered.rows.is_empty(),
            "verified recovery is a touch, not a replay"
        );
        assert!(
            recovered
                .coverage
                .iter()
                .all(|gap| !gap.reason.contains("transcript rewritten")),
            "verified recovery must not report a rewrite: {:?}",
            recovered.coverage
        );
        assert!(
            follow.standing().is_empty(),
            "a successful verified read clears the earlier read-door failure"
        );
    }

    #[test]
    fn quiet_verified_touch_after_failed_append_restores_content_doubt() {
        let bytes = format!("{}{}\n", first(), "x".repeat(1_048_576 + 32));
        let mut follow = Follow::seeded(&[], &[], None);
        let initial = read(&mut follow, &bytes, 1);
        assert_eq!(initial.rows.len(), 1);
        assert!(
            initial
                .coverage
                .iter()
                .any(|gap| gap.reason.contains("1 MiB cap"))
        );
        failed_append(&mut follow, bytes.len());
        let recovered = read(&mut follow, &bytes, 3);
        assert!(
            recovered.rows.is_empty(),
            "verified recovery is a touch, not a replay"
        );
        assert!(
            recovered
                .coverage
                .iter()
                .all(|gap| !gap.reason.contains("transcript rewritten")),
            "verified recovery must not report a rewrite: {:?}",
            recovered.coverage
        );
        let standing = follow.standing();
        assert_eq!(standing.len(), 1, "the content warning still stands");
        assert!(
            standing[0].reason.contains("1 MiB cap"),
            "identical bytes restore their parser warning, not the resolved read failure: {standing:?}"
        );
    }

    #[test]
    fn quiet_verified_touch_remembers_mtime_so_the_next_snapshot_holds() {
        let bytes = first();
        let mut follow = Follow::seeded(&[], &[], None);
        let initial = read(&mut follow, &bytes, 1);
        assert_eq!(initial.rows.len(), 1);
        assert!(initial.coverage.is_empty());
        let touched = read(&mut follow, &bytes, 2);
        assert!(
            touched.rows.is_empty() && touched.coverage.is_empty(),
            "verified identical bytes are a quiet touch"
        );
        assert_eq!(
            follow.plan(ACTOR, &located(bytes.len(), 2)),
            Plan::Hold,
            "the verified mtime is remembered so unchanged bytes are not reread"
        );
    }

    #[test]
    fn quiet_splitter_at_preserves_absolute_offsets_for_a_complete_line() {
        let bytes = first();
        let base = 37;
        let mut splitter = Splitter::at(base);
        splitter.feed(bytes.as_bytes());
        let streamed = splitter.finish();
        assert_eq!(streamed.lines.len(), 1);
        assert_eq!(
            streamed.lines[0].offset, base,
            "line offsets start at the declared absolute base"
        );
        assert_eq!(
            streamed.committed,
            base + bytes.len() as u64,
            "the commit point remains absolute in the file"
        );
        assert!(!streamed.torn);
    }
}
