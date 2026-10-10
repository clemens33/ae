//! The `goal` helper: `goal <text>`, `goal --clear`, `goal` and `goal --help`.
//!
//! The text is made one
//! printable line (newlines and tabs to spaces, every other control byte
//! dropped), written as `goal=<text>` into the session's `meta` under
//! `meta.lock`, and announced with a `goal` event whose summary is the text;
//! `--clear` removes the key and announces `goal cleared`. The no-argument
//! READ prints the FIRST `goal=` record's value or `(no goal set)`, and
//! `--help`/`-h` print the usage
//! and take the usage exit.
//!
//! Two storage transactions, deliberately NOT one: the meta rewrite (a locked
//! replace of a whole small file) and the event append (a locked append to the
//! container) have different boundaries, and forcing them under one lock
//! would couple every `ae list` reader of meta to the event log. A failure
//! between them is therefore possible and is reported EXACTLY: "written to
//! meta but its event was not emitted" — loud, non-zero, and never the
//! success line. A separate outer lock, `.goal-setter.lock`, serializes WRITERS
//! across both, from the unchanged check through the event, so the newest
//! `goal` record names the published goal and its setter; readers take neither.
use std::io;
use std::path::Path;

use crate::meta;
use crate::requests::Viewer;
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::store;
use crate::time::Timestamp;
use crate::tracked::EventFields;

/// The usage text.
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
/// A meta that exists but could not be read. It is REPORTED rather than
/// rendered as `(no goal set)`, because "no goal" and "could not look" are
/// different answers.
pub fn show(dir: &Path) -> io::Result<Vec<u8>> {
    Ok(shown(store::open(dir).goal()?.as_deref()))
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
/// success line for stdout — only once both writes are down — and whether the
/// goal changed. An unchanged goal writes nothing and journals nothing.
///
/// The record names the setter's slot, session and session id when its pane
/// was proven, so the session notice spares the seat that set it.
///
/// # Errors
///
/// [`Failure`] — see its variants.
pub fn run(
    dir: &Path,
    viewer: &Viewer,
    write: &Write,
    now: Timestamp,
) -> Result<(String, bool), Failure> {
    let actor = if viewer.is_known() {
        viewer.display.as_str()
    } else {
        "human"
    };
    // One setter at a time from the check through the record: the newest
    // record always names the published goal and its setter.
    let session = store::open(dir);
    let _setter = session.goal_setter().map_err(Failure::Meta)?;
    let current = session.goal().map_err(Failure::Read)?;
    let current = current.as_deref().unwrap_or_default();
    let (value, summary, line) = match write {
        Write::Set(text) if current == text.as_bytes() => {
            return Ok((format!("Goal unchanged: {text}\n"), false));
        }
        Write::Clear if current.is_empty() => {
            return Ok(("Goal unchanged: (no goal set)\n".to_owned(), false));
        }
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
    let fields = EventFields {
        actor_session_id: &viewer.session_id,
        ..EventFields::new(
            now,
            actor,
            "goal",
            "",
            "",
            &viewer.slot,
            &viewer.session,
            "",
            "",
            summary,
            "",
        )
    };
    session
        .append_event(&crate::tracked::event_line(&fields))
        .map_err(|why| Failure::Event(why.into()))?;
    Ok((line, true))
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

    /// Two setters race while the journal is held: the second waits for the
    /// first's WHOLE operation, so the newest record names the published goal
    /// and its setter, and an equal update after a pending one stays a no-op.
    #[test]
    fn competing_setters_publish_and_record_in_one_order() {
        use std::time::{Duration, Instant};
        let root = std::env::temp_dir().join(format!("ae-goal-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let id = "11111111-2222-3333-4444-555555555555";
        let meta = format!(
            "session=s\nsession_id={id}\nlayout=lead-pair\nseat.main=lead\nseat.worker.0=colead\ngoal=initial\n"
        );
        std::fs::write(root.join("meta"), meta).unwrap();
        let change = |slot: &'static str, name: &'static str, text: &'static str| {
            let dir = root.clone();
            std::thread::spawn(move || {
                let viewer = super::Viewer {
                    slot: slot.to_owned(),
                    session: "s".to_owned(),
                    display: name.to_owned(),
                    session_id: id.to_owned(),
                };
                let changed = super::run(
                    &dir,
                    &viewer,
                    &Write::Set(text.to_owned()),
                    super::Timestamp::now(),
                );
                changed.unwrap().1
            })
        };
        let goal = || {
            super::store::open(&root)
                .goal()
                .unwrap()
                .unwrap_or_default()
        };
        let published = |text: &[u8], budget: Duration| {
            let until = Instant::now() + budget;
            while goal() != text {
                if Instant::now() > until {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            true
        };
        let journal = super::store::lock_path(&super::store::open(&root).events_path());
        let held = super::store::lock(&journal, Duration::ZERO).unwrap();
        let left = change("main", "lead", "left");
        assert!(
            published(b"left", Duration::from_secs(3)),
            "the first setter published"
        );
        let right = change("worker.0", "colead", "right");
        let raced = published(b"right", Duration::from_millis(300));
        drop(held);
        assert!(
            left.join().unwrap() && right.join().unwrap(),
            "both changed the goal"
        );
        assert!(
            !raced,
            "a second setter published while the first's record was pending"
        );
        let events = crate::watchdog_daemon::read_events(&root);
        let bytes = crate::meta::read_bytes(&root).unwrap();
        let meta = crate::meta::Meta::parse(&String::from_utf8_lossy(&bytes));
        let due = |slot: &str| {
            let seat = crate::session_notice::Seat::read(&meta, &bytes, slot, "s");
            crate::session_notice::due(&events, &seat, super::Timestamp::now().epoch(), 120)
                .is_some()
        };
        assert_eq!(goal(), b"right");
        assert_eq!(
            events.last().and_then(|event| event.summary.as_deref()),
            Some("right")
        );
        assert!(
            due("main") && !due("worker.0"),
            "the peer is owed, the setter spared"
        );
        let held = super::store::lock(&journal, Duration::ZERO).unwrap();
        let first = change("main", "lead", "same");
        assert!(published(b"same", Duration::from_secs(3)));
        let second = change("worker.0", "colead", "same");
        std::thread::sleep(Duration::from_millis(50));
        drop(held);
        assert_eq!(
            (first.join().unwrap(), second.join().unwrap()),
            (true, false)
        );
        let records = crate::watchdog_daemon::read_events(&root);
        assert_eq!(
            records.len(),
            events.len() + 1,
            "an equal update journals nothing"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
