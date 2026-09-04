//! The `goal` helper: `goal <text>`, `goal --clear`, `goal` and `goal --help`.
//!
//! What the frozen `helper_goal_main` does, kept exactly: the text is made one
//! printable line (newlines and tabs to spaces, every other control byte
//! dropped), written as `goal=<text>` into the session's `meta` under
//! `meta.lock`, and announced with a `goal` event whose summary is the text;
//! `--clear` removes the key and announces `goal cleared`. The no-argument
//! READ (P2.4) prints the FIRST `goal=` record's value or `(no goal set)` —
//! `ae_meta_get`'s `grep | head -1 | cut` — and `--help`/`-h` print the usage
//! and take the usage exit.
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
    /// `goal` — show the current goal.
    Show,
    /// `goal --help` / `goal -h` — the usage text and the usage exit, as the
    /// frozen body answers them (anything after the flag is ignored, as it
    /// only ever looked at `$1`).
    Help,
    /// `goal <text>` / `goal --clear`.
    Write(Write),
}

/// A change to the goal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Write {
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
/// [`Usage`] for `--clear` with company, or for text that is empty once made
/// printable.
pub fn parse(tail: &[String]) -> Result<Command, Usage> {
    match tail {
        [] => Ok(Command::Show),
        [flag, ..] if flag == "--help" || flag == "-h" => Ok(Command::Help),
        [flag] if flag == "--clear" => Ok(Command::Write(Write::Clear)),
        [flag, ..] if flag == "--clear" => Err(Usage),
        words => {
            let text = printable(&words.join(" "));
            if text.is_empty() {
                Err(Usage)
            } else {
                Ok(Command::Write(Write::Set(text)))
            }
        }
    }
}

/// The stdout of `goal` with no arguments, for the meta at `dir`: the first
/// `goal=` record's value, or `(no goal set)` when there is none, it is empty,
/// or there is no meta file at all — `ae_meta_get`'s `2>/dev/null || true`
/// makes an absent file an empty answer.
///
/// # Errors
///
/// A meta that exists but could not be read. The frozen body hides that
/// behind the same `|| true` and prints `(no goal set)`; this says what
/// happened instead, because "no goal" and "could not look" are different
/// answers.
pub fn show(dir: &Path) -> io::Result<Vec<u8>> {
    let text = match meta::read_bytes(dir) {
        Ok(text) => text,
        Err(why) if why.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(why) => return Err(why),
    };
    Ok(shown(meta::first_value(&text, KEY)))
}

/// The line `show` prints for a value: the bytes plus a newline, or
/// `(no goal set)` for none or empty — `[[ -n "$current" ]]`.
///
/// ```
/// use ae::goal::shown;
///
/// assert_eq!(shown(Some(b"ship it")), b"ship it\n");
/// assert_eq!(shown(Some(b"")), b"(no goal set)\n");
/// assert_eq!(shown(None), b"(no goal set)\n");
/// ```
#[must_use]
pub fn shown(value: Option<&[u8]>) -> Vec<u8> {
    match value {
        Some(value) if !value.is_empty() => {
            let mut out = value.to_vec();
            out.push(b'\n');
            out
        }
        _ => b"(no goal set)\n".to_vec(),
    }
}

/// One printable line: `tr '\n\t' ' ' | tr -d '[:cntrl:]'` — newline and tab
/// become spaces first, then every remaining C0 control and DEL is dropped.
#[must_use]
pub fn printable(text: &str) -> String {
    text.chars()
        .map(|c| if c == '\n' || c == '\t' { ' ' } else { c })
        .filter(|c| !c.is_ascii_control())
        .collect()
}

/// Why the goal was not (fully) recorded, or not read.
#[derive(Debug)]
pub enum Failure {
    /// `goal` could not read a meta that exists.
    Read(io::Error),
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
            Self::Read(why) => format!("ae: goal not read: could not read session meta: {why}"),
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

/// Apply `write` to the session at `dir` for `viewer`, and return the
/// success line for stdout — only once both writes are down.
///
/// # Errors
///
/// [`Failure`] — see its variants.
pub fn run(dir: &Path, viewer: &Viewer, write: &Write, now: Timestamp) -> Result<String, Failure> {
    let actor = if viewer.is_known() {
        viewer.display.as_str()
    } else {
        "human"
    };
    let (value, summary, line) = match write {
        Write::Set(text) => (
            Some(text.as_str()),
            text.as_str(),
            format!("Goal set: {text}\n"),
        ),
        Write::Clear => (None, "goal cleared", "Goal cleared.\n".to_owned()),
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
    use super::{Command, Usage, Write, parse, printable, show};

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn argv_reads_as_the_helper_reads_it() {
        assert_eq!(
            parse(&words(&["--clear"])),
            Ok(Command::Write(Write::Clear))
        );
        assert_eq!(parse(&words(&["--clear", "x"])), Err(Usage));
        assert_eq!(parse(&[]), Ok(Command::Show), "nothing to set is a read");
        assert_eq!(parse(&words(&["--help"])), Ok(Command::Help));
        assert_eq!(
            parse(&words(&["-h", "ignored"])),
            Ok(Command::Help),
            "the frozen case looks at $1 only"
        );
        assert_eq!(
            parse(&words(&["ship", "it"])),
            Ok(Command::Write(Write::Set("ship it".to_owned())))
        );
        assert_eq!(
            parse(&words(&["\u{7}"])),
            Err(Usage),
            "nothing printable is no goal"
        );
    }

    #[test]
    fn show_prints_the_first_record_or_no_goal_and_reports_an_unreadable_meta() {
        let dir = std::path::PathBuf::from(format!("/tmp/ae-goal-show-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            show(&dir).unwrap(),
            b"(no goal set)\n",
            "no meta at all is no goal, as the frozen grep's || true makes it"
        );
        std::fs::write(
            dir.join("meta"),
            b"mode=local\ngoal=first=kept\r\ngoal=second\n",
        )
        .unwrap();
        assert_eq!(
            show(&dir).unwrap(),
            b"first=kept\r\n",
            "head -1, cut -d= -f2-, bytes verbatim"
        );
        std::fs::write(dir.join("meta"), b"mode=local\ngoal=\n").unwrap();
        assert_eq!(
            show(&dir).unwrap(),
            b"(no goal set)\n",
            "an empty value is no goal"
        );
        std::fs::remove_file(dir.join("meta")).unwrap();
        std::fs::create_dir_all(dir.join("meta")).unwrap();
        assert!(
            show(&dir).is_err(),
            "a meta that exists but cannot be read is reported, not read as no goal"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn each_failure_names_what_is_known_about_the_meta() {
        use super::Failure;
        let why = || std::io::Error::other("disk");
        assert!(Failure::Read(why()).message().contains("goal not read"));
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
