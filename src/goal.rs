//! The `goal` helper's WRITE path: `goal <text>` and `goal --clear`.
//!
//! What the frozen `helper_goal_main` does, kept exactly: the text is made one
//! printable line (newlines and tabs to spaces, every other control byte
//! dropped), written as `goal=<text>` into the session's `meta` under
//! `meta.lock`, and announced with a `goal` event whose summary is the text;
//! `--clear` removes the key and announces `goal cleared`. The no-argument
//! READ and `--help` stay on the bash body.
//!
//! Two storage transactions, deliberately NOT one: the meta rewrite (a locked
//! replace of a whole small file) and the event append (a locked append to the
//! container) have different boundaries, and forcing them under one lock
//! would couple every `ae list` reader of meta to the event log. A failure
//! between them is therefore possible and is reported EXACTLY: "written to
//! meta but its event was not emitted" — loud, non-zero, and never the
//! success line.
use std::io;
use std::path::Path;

use crate::meta;
use crate::requests::Viewer;
use crate::state::{self, EXIT_FAILED, EXIT_USAGE};
use crate::time::Timestamp;

/// The frozen `helper_goal_usage` text.
pub const USAGE: &str = "Usage: goal            # show the session goal\n       goal <text>     # set it (one line)\n       goal --clear    # remove it\n";

/// The meta key.
pub const KEY: &str = "goal";

/// What the caller asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `goal <text>` — the text already made one printable line.
    Set(String),
    /// `goal --clear`.
    Clear,
}

/// A refused argv: [`USAGE`] to stderr, exit [`EXIT_USAGE`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage;

/// Parse the argv after the meta directory.
///
/// # Errors
///
/// [`Usage`] for `--clear` with company, for no text, or for text that is
/// empty once made printable.
pub fn parse(tail: &[String]) -> Result<Command, Usage> {
    match tail {
        [] => Err(Usage),
        [flag] if flag == "--clear" => Ok(Command::Clear),
        [flag, ..] if flag == "--clear" => Err(Usage),
        words => {
            let text = printable(&words.join(" "));
            if text.is_empty() {
                Err(Usage)
            } else {
                Ok(Command::Set(text))
            }
        }
    }
}

/// One printable line: `tr '\n\t' '  ' | tr -d '[:cntrl:]'` — newline and
/// tab become spaces first, then every remaining C0 control and DEL is
/// dropped. The value fans out to meta, the event log, `list` and the
/// watchdog nudge, none of which may carry a raw control byte.
#[must_use]
pub fn printable(text: &str) -> String {
    text.chars()
        .map(|c| if c == '\n' || c == '\t' { ' ' } else { c })
        .filter(|c| !c.is_ascii_control())
        .collect()
}

/// Why the goal was not (fully) recorded. All are [`EXIT_FAILED`].
#[derive(Debug)]
pub enum Failure {
    /// The meta rewrite failed with nothing visible changed — nothing
    /// announced.
    Meta(io::Error),
    /// The new meta is visible but its directory entry could not be synced:
    /// whether it survives a crash is not known, so no event is emitted and
    /// the caller is told exactly that — never "nothing changed".
    MetaUnknown(io::Error),
    /// Meta changed durably but the event append failed.
    Event(io::Error),
}

impl Failure {
    /// The stderr line.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Meta(why) => {
                format!("ae: goal not recorded: could not write session meta: {why}")
            }
            Self::MetaUnknown(why) => format!(
                "ae: goal in an UNKNOWN state: the new meta is visible but could not be published durably, and no event was emitted: {why}"
            ),
            Self::Event(why) => {
                format!("ae: goal written to meta but its event was not emitted: {why}")
            }
        }
    }
}

/// Apply `command` to the session at `dir` for `viewer`, and return the
/// success line for stdout — only once both writes are down.
///
/// The event's actor is the pane's display ref, or `human` when the caller has
/// none: the frozen emitter's own fallback, minus its habit of naming whatever
/// pane the server last touched.
///
/// # Errors
///
/// [`Failure`] — see its variants.
pub fn run(
    dir: &Path,
    viewer: &Viewer,
    command: &Command,
    now: Timestamp,
) -> Result<String, Failure> {
    let actor = if viewer.is_known() {
        viewer.display.as_str()
    } else {
        "human"
    };
    let (value, summary, line) = match command {
        Command::Set(text) => (
            Some(text.as_str()),
            text.as_str(),
            format!("Goal set: {text}\n"),
        ),
        Command::Clear => (None, "goal cleared", "Goal cleared.\n".to_owned()),
    };
    meta::rewrite(dir, KEY, value).map_err(|why| match why {
        meta::RewriteError::NotWritten(cause) => Failure::Meta(cause),
        meta::RewriteError::Unknown(cause) => Failure::MetaUnknown(cause),
    })?;
    let event = state::event_line(now, actor, "goal", "", &state::summary_of(summary));
    state::emit(dir, &event).map_err(Failure::Event)?;
    Ok(line)
}

/// The exit status a [`Usage`] takes.
#[must_use]
pub const fn usage_code() -> u8 {
    EXIT_USAGE
}

/// The exit status a [`Failure`] takes.
#[must_use]
pub const fn failure_code() -> u8 {
    EXIT_FAILED
}

#[cfg(test)]
mod tests {
    use super::{Command, Usage, parse, printable};

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn argv_reads_as_the_helper_reads_it() {
        assert_eq!(parse(&words(&["--clear"])), Ok(Command::Clear));
        assert_eq!(parse(&words(&["--clear", "x"])), Err(Usage));
        assert_eq!(parse(&[]), Err(Usage));
        assert_eq!(
            parse(&words(&["ship", "it"])),
            Ok(Command::Set("ship it".to_owned()))
        );
        assert_eq!(
            parse(&words(&["\u{7}"])),
            Err(Usage),
            "nothing printable is no goal"
        );
    }

    #[test]
    fn each_failure_names_what_is_known_about_the_meta() {
        use super::Failure;
        let why = || std::io::Error::other("disk");
        assert!(Failure::Meta(why()).message().contains("goal not recorded"));
        let unknown = Failure::MetaUnknown(why()).message();
        assert!(
            unknown.contains("UNKNOWN") && unknown.contains("no event was emitted"),
            "{unknown}"
        );
        assert!(
            !unknown.contains("not recorded"),
            "an unknown outcome must not claim nothing changed"
        );
        assert!(
            Failure::Event(why())
                .message()
                .contains("written to meta but its event was not emitted")
        );
    }

    #[test]
    fn the_text_becomes_one_printable_line() {
        assert_eq!(printable("a\nb\tc"), "a b c");
        assert_eq!(
            printable("bell\u{7}esc\u{1b}[0m del\u{7f}"),
            "bellesc[0m del"
        );
        assert_eq!(
            printable("ümlaut — kept"),
            "ümlaut — kept",
            "only ASCII controls go"
        );
    }
}
