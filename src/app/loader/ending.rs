//! The reader's watch over an End handed to its detached owner. Nothing comes
//! back from that process, so its result is read from what it leaves: the
//! session's directory gone, or an `end-result` failure in its journal. The
//! watch reads on the fleet reads that happen anyway, only while one is
//! pending, and believes only the incarnation it asked about: records written
//! after the journal position the app took just before it let the owner go.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::board::terminal_text;
use crate::event_text;
use crate::store::SourceRead;

/// How long a watch waits before it says there is no result yet. An End that
/// pushes can take longer, so reaching this implies nothing about the End.
pub(super) const NO_RESULT_YET: Duration = Duration::from_mins(2);

/// One End the app started and has not heard of.
#[derive(Debug, Clone)]
pub(in crate::app) struct EndingAsk {
    pub(in crate::app) name: String,
    /// The session's record directory.
    pub(in crate::app) dir: PathBuf,
    /// The `session_id` the human confirmed.
    pub(in crate::app) uuid: String,
    /// The journal's length just before the owner started: only what lies
    /// past it can be this End's.
    pub(in crate::app) position: usize,
    pub(in crate::app) asked: Instant,
}

/// What one look found, in the order the app is told.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Seen {
    /// Nothing yet: keep watching.
    Pending,
    /// The line to tell. The watch is over.
    Told(String),
    /// Another session holds the name now, or the record stopped naming one:
    /// nothing here is this End's to report. The watch is over, silently.
    Replaced,
}

/// The newest failure this End's owner recorded past `position`. A journal
/// now shorter than `position` is not the journal the position was taken in,
/// so nothing in it counts.
fn failure(events: &SourceRead, position: usize) -> Option<String> {
    let SourceRead::Ready(bytes) = events else {
        return None;
    };
    let stream = event_text::reversed(bytes.get(position..)?);
    event_text::read_lines(&stream)
        .into_iter()
        .filter_map(event_text::event_line)
        .filter(|line| event_text::extract(line, "action") == b"end-result")
        .map(|line| event_text::extract(line, "summary"))
        .map(|summary| String::from_utf8_lossy(&summary).into_owned())
        .find(|summary| summary.starts_with("FAILED"))
}

/// One look: `events` was read first, then `gone` and `uuid`, so a directory
/// swapped between the two reads never lends its journal to this End.
pub(super) fn seen(
    ask: &EndingAsk,
    events: &SourceRead,
    gone: bool,
    uuid: Option<&str>,
    waited: Duration,
) -> Seen {
    let name = terminal_text(&ask.name);
    if gone {
        return Seen::Told(format!("Ended {name}"));
    }
    if uuid != Some(ask.uuid.as_str()) {
        return Seen::Replaced;
    }
    if let Some(summary) = failure(events, ask.position) {
        return Seen::Told(format!("End {name}: {}", terminal_text(&summary)));
    }
    if waited >= NO_RESULT_YET {
        return Seen::Told(format!("End of {name}: no result yet; see ae list"));
    }
    Seen::Pending
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use super::{EndingAsk, NO_RESULT_YET, Seen, seen};
    use crate::store::SourceRead;

    const ID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
    const OLD: &str = concat!(
        r#"{"ts":"2026-01-01T00:00:00Z","actor":"human","action":"end-result","#,
        r#""target":"web","summary":"FAILED: an older end"}"#,
        "\n",
    );
    const NEW: &str = concat!(
        r#"{"ts":"2026-01-01T00:00:00Z","actor":"human","action":"end-result","#,
        r#""target":"web","summary":"FAILED: session STOPPED, not ended: x"}"#,
        "\n",
    );

    fn ask(position: usize) -> EndingAsk {
        EndingAsk {
            name: "web".to_owned(),
            dir: PathBuf::from("/nowhere/web"),
            uuid: ID.to_owned(),
            position,
            asked: Instant::now(),
        }
    }

    fn ready(text: &str) -> SourceRead {
        SourceRead::Ready(text.as_bytes().to_vec())
    }

    #[test]
    fn a_failure_at_the_position_is_this_ends_and_one_before_it_is_not() {
        let both = format!("{OLD}{NEW}");
        let told = seen(
            &ask(OLD.len()),
            &ready(&both),
            false,
            Some(ID),
            Duration::ZERO,
        );
        assert_eq!(
            told,
            Seen::Told("End web: FAILED: session STOPPED, not ended: x".to_owned())
        );
        assert_eq!(
            seen(
                &ask(OLD.len()),
                &ready(OLD),
                false,
                Some(ID),
                Duration::ZERO
            ),
            Seen::Pending,
            "the same second's older failure is not this End's"
        );
    }

    #[test]
    fn a_journal_shorter_than_the_position_gives_nothing_and_keeps_watching() {
        assert_eq!(
            seen(
                &ask(OLD.len() + 1),
                &ready(OLD),
                false,
                Some(ID),
                Duration::ZERO
            ),
            Seen::Pending
        );
    }

    #[test]
    fn a_replaced_directory_never_supplies_an_outcome() {
        for uuid in [Some("0199c0de-ffff-4890-abcd-ef0123456789"), None] {
            assert_eq!(
                seen(&ask(0), &ready(NEW), false, uuid, Duration::ZERO),
                Seen::Replaced
            );
        }
    }

    #[test]
    fn a_gone_directory_is_ended_whatever_the_read_before_it_held() {
        assert_eq!(
            seen(&ask(0), &ready(NEW), true, None, Duration::ZERO),
            Seen::Told("Ended web".to_owned())
        );
    }

    #[test]
    fn the_wait_ends_in_exactly_the_no_result_line_and_only_at_its_bound() {
        let quiet = SourceRead::Absent;
        let waited = NO_RESULT_YET.saturating_sub(Duration::from_secs(1));
        assert_eq!(
            seen(&ask(0), &quiet, false, Some(ID), waited),
            Seen::Pending
        );
        assert_eq!(
            seen(&ask(0), &quiet, false, Some(ID), NO_RESULT_YET),
            Seen::Told("End of web: no result yet; see ae list".to_owned())
        );
    }

    #[test]
    fn a_failure_outranks_the_wait_and_other_results_do_not_count() {
        let note = concat!(
            r#"{"ts":"2026-01-01T00:00:00Z","actor":"human","action":"end-result","#,
            r#""target":"web","summary":"clients moved"}"#,
            "\n",
            r#"{"ts":"2026-01-01T00:00:00Z","actor":"human","action":"end-request","#,
            r#""target":"web","summary":"FAILED: not a result"}"#,
            "\n",
        );
        assert_eq!(
            seen(&ask(0), &ready(note), false, Some(ID), Duration::ZERO),
            Seen::Pending
        );
        assert_eq!(
            seen(&ask(0), &ready(NEW), false, Some(ID), NO_RESULT_YET),
            Seen::Told("End web: FAILED: session STOPPED, not ended: x".to_owned())
        );
    }

    #[test]
    fn a_hostile_summary_is_neutralised_before_it_is_told() {
        let line = concat!(
            r#"{"ts":"2026-01-01T00:00:00Z","actor":"human","action":"end-result","#,
            r#""target":"web","summary":"FAILED: \u001b]52;c;x\u0007 bad"}"#,
            "\n",
        );
        let told = seen(&ask(0), &ready(line), false, Some(ID), Duration::ZERO);
        let Seen::Told(line) = told else {
            panic!("a failure is told, got {told:?}");
        };
        assert!(!line.contains('\u{1b}'), "{line:?}");
        assert!(!line.contains('\u{7}'), "{line:?}");
    }
}
