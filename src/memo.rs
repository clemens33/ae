//! The `memo add` WRITE path.
//!
//! What the frozen `helper_memo_main add` does, kept exactly: one TSV record
//! `<ts>\t<author>\t<topic>\t<text>` appended to `memo.tsv` under
//! `memo.tsv.lock` — carriage returns dropped, tabs and newlines to spaces in
//! both topic and text, an empty topic is `general` — then a `memo` event
//! (`ref` = topic, `summary` = text) on the container. Nothing on stdout.
//! `read` and `tail` stay on the bash body.
//!
//! Two files, two locks, two transactions — as in bash. A failure between them
//! is reported as exactly that: recorded in `memo.tsv`, event not emitted.
use std::io;
use std::path::Path;

use crate::requests::Viewer;
use crate::state;
use crate::time::Timestamp;

/// The frozen `helper_memo_usage` text.
pub const USAGE: &str =
    "Usage: memo add [--topic <topic>] <text> | memo read [--topic <topic>] | memo tail [n]\n";

/// The memo file, inside the session directory.
pub const FILE: &str = "memo.tsv";

/// The topic when none is given or the given one is empty.
pub const DEFAULT_TOPIC: &str = "general";

/// A parsed `add`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Add {
    /// The topic, sanitised, never empty.
    pub topic: String,
    /// The text, sanitised.
    pub text: String,
}

/// A refused argv: [`USAGE`] to stderr, exit [`state::EXIT_USAGE`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage;

/// Parse the argv after the meta directory: `add [--topic <topic>] <text…>`.
///
/// # Errors
///
/// [`Usage`] for anything but `add`, a `--topic` without both a topic and a
/// text, or no text.
pub fn parse(tail: &[String]) -> Result<Add, Usage> {
    let [command, rest @ ..] = tail else {
        return Err(Usage);
    };
    if command != "add" {
        return Err(Usage);
    }
    let (topic, words) = match rest {
        [flag, topic, words @ ..] if flag == "--topic" => (topic.as_str(), words),
        [flag] | [flag, _] if flag == "--topic" => return Err(Usage),
        words => (DEFAULT_TOPIC, words),
    };
    if words.is_empty() {
        return Err(Usage);
    }
    let topic = one_line(topic);
    Ok(Add {
        topic: if topic.is_empty() {
            DEFAULT_TOPIC.to_owned()
        } else {
            topic
        },
        text: one_line(&words.join(" ")),
    })
}

/// The helper's field sanitiser: `\r` dropped, `\t` and `\n` to spaces.
#[must_use]
pub fn one_line(text: &str) -> String {
    text.chars()
        .filter(|c| *c != '\r')
        .map(|c| if c == '\t' || c == '\n' { ' ' } else { c })
        .collect()
}

/// One memo record, `\n` included.
#[must_use]
pub fn record(ts: Timestamp, author: &str, add: &Add) -> String {
    format!("{ts}\t{author}\t{}\t{}\n", add.topic, add.text)
}

/// Why the memo was not (fully) recorded. Both are [`state::EXIT_FAILED`].
#[derive(Debug)]
pub enum Failure {
    /// The TSV append failed — nothing recorded, nothing announced.
    Tsv(io::Error),
    /// The TSV record landed but the event append failed.
    Event(io::Error),
}

impl Failure {
    /// The stderr line.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Tsv(why) => format!("ae: memo not recorded: could not append to {FILE}: {why}"),
            Self::Event(why) => {
                format!("ae: memo recorded in {FILE} but its event was not emitted: {why}")
            }
        }
    }
}

/// Record `add` for `viewer` in the session at `dir`.
///
/// The author is the pane's display ref, or `human` when the caller has none —
/// a memo typed at a shell is a human's note, and that is what the frozen
/// helper writes too.
///
/// # Errors
///
/// [`Failure`] — see its variants.
pub fn run(dir: &Path, viewer: &Viewer, add: &Add, now: Timestamp) -> Result<(), Failure> {
    let author = if viewer.is_known() {
        viewer.display.as_str()
    } else {
        "human"
    };
    state::append_locked(&dir.join(FILE), record(now, author, add).as_bytes())
        .map_err(|why| Failure::Tsv(why.into()))?;
    let event = state::event_line(
        now,
        author,
        "memo",
        &add.topic,
        &state::summary_of(&add.text),
    );
    state::emit(dir, &event).map_err(Failure::Event)
}

#[cfg(test)]
mod tests {
    use super::{Add, Usage, one_line, parse, record};
    use crate::time::Timestamp;

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn argv_reads_as_the_helper_reads_it() {
        assert_eq!(
            parse(&words(&["add", "two", "words"])),
            Ok(Add {
                topic: "general".to_owned(),
                text: "two words".to_owned()
            })
        );
        assert_eq!(
            parse(&words(&["add", "--topic", "p2", "note"])),
            Ok(Add {
                topic: "p2".to_owned(),
                text: "note".to_owned()
            })
        );
        assert_eq!(
            parse(&words(&["add", "--topic", "p2"])),
            Err(Usage),
            "no text"
        );
        assert_eq!(parse(&words(&["add", "--topic"])), Err(Usage));
        assert_eq!(parse(&words(&["add"])), Err(Usage));
        assert_eq!(parse(&words(&["read"])), Err(Usage), "only add is a write");
        assert_eq!(parse(&[]), Err(Usage));
        assert_eq!(
            parse(&words(&["add", "--topic", "", "x"])).unwrap().topic,
            "general",
            "an empty topic is the default"
        );
    }

    #[test]
    fn fields_are_one_line_and_the_record_is_tab_separated() {
        assert_eq!(one_line("a\r\nb\tc"), "a b c");
        let ts = Timestamp::parse("2026-08-27T08:00:00Z").unwrap();
        let add = Add {
            topic: "p2".to_owned(),
            text: "the note".to_owned(),
        };
        assert_eq!(
            record(ts, "cl:lead", &add),
            "2026-08-27T08:00:00Z\tcl:lead\tp2\tthe note\n"
        );
    }
}
