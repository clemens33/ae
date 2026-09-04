//! The `memo` helper: `add`, `read` and `tail`.
//!
//! What the frozen `helper_memo_main add` does, kept exactly: one TSV record
//! `<ts>\t<author>\t<topic>\t<text>` appended to `memo.tsv` under
//! `memo.tsv.lock` — carriage returns dropped, tabs and newlines to spaces in
//! both topic and text, an empty topic is `general` — then a `memo` event
//! (`ref` = topic, `summary` = text) on the container. Nothing on stdout.
//!
//! Two files, two locks, two transactions — as in bash. A failure between them
//! is reported as exactly that: recorded in `memo.tsv`, event not emitted.
//!
//! `read [--topic <topic>]` and `tail [n]` (P2.4) are `helper_memo_render`'s
//! awk program, byte for byte ([`render`]), over the whole file or its last
//! `n` records ([`crate::event_text::last_records`] — what `tail -n` counts).
//! `[[ -f "$MEMO_FILE" ]] || exit 0` is applied as written: no `memo.tsv`, or
//! anything but a regular file in its place — a directory, a FIFO — is empty
//! output at 0. The gate comes BEFORE the open, because a FIFO opened without
//! it blocks the reader for good (found in review). A regular file that then
//! cannot be read is reported at 1, where the frozen body printed awk's
//! complaint.
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

/// `tail` with no count — `${1:-10}`.
pub const DEFAULT_TAIL: usize = 10;

/// What the caller asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `memo add [--topic <topic>] <text…>`.
    Add(Add),
    /// `memo`, `memo read [--topic <topic>]`, `memo tail [n]`.
    View(View),
}

/// A read of the memo file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    /// Every record, or only those whose topic is the given one.
    All(Option<String>),
    /// The last `n` records, unfiltered.
    Tail(usize),
}

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

/// Parse the argv after the meta directory, the way `helper_memo_main` does:
/// no word is `read`; `read` takes nothing or exactly `--topic <topic>`;
/// `tail` takes nothing or one all-digit count (an empty word is the default,
/// as `${1:-10}` reads it; a count too large for `usize` is every record,
/// which is what GNU `tail` gives — BSD `tail` refuses it, so the frozen body
/// already answered that one two ways).
///
/// # Errors
///
/// [`Usage`] for any other shape — including `add` without a text, or with a
/// `--topic` missing either half.
pub fn parse(tail: &[String]) -> Result<Command, Usage> {
    let Some((command, rest)) = tail.split_first() else {
        return Ok(Command::View(View::All(None)));
    };
    match command.as_str() {
        "add" => parse_add(rest).map(Command::Add),
        "read" => match rest {
            [] => Ok(Command::View(View::All(None))),
            [flag, topic] if flag == "--topic" => Ok(Command::View(View::All(
                Some(topic.clone()).filter(|topic| !topic.is_empty()),
            ))),
            _ => Err(Usage),
        },
        "tail" => match rest {
            [] => Ok(Command::View(View::Tail(DEFAULT_TAIL))),
            [count] if count.is_empty() => Ok(Command::View(View::Tail(DEFAULT_TAIL))),
            [count] if count.bytes().all(|byte| byte.is_ascii_digit()) => Ok(Command::View(
                View::Tail(count.parse().unwrap_or(usize::MAX)),
            )),
            _ => Err(Usage),
        },
        _ => Err(Usage),
    }
}

/// The argv after `add`: `[--topic <topic>] <text…>`.
fn parse_add(rest: &[String]) -> Result<Add, Usage> {
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

/// Why the memo was not (fully) recorded, or not read.
#[derive(Debug)]
pub enum Failure {
    /// `read`/`tail` could not read a memo file that exists.
    Read(io::Error),
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
            Self::Read(why) => format!("ae: memo not read: could not read {FILE}: {why}"),
            Self::Tsv(why) => format!("ae: memo not recorded: could not append to {FILE}: {why}"),
            Self::Event(why) => {
                format!("ae: memo recorded in {FILE} but its event was not emitted: {why}")
            }
        }
    }
}

/// Record `add` for `viewer` in the session at `dir`.
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

/// `helper_memo_render`, byte for byte: for every `\n`-separated record with
/// at least four tab-separated fields — awk `-F '\t'` and `NF >= 4`, so a
/// record with MORE fields renders only its fourth, and a record whose fourth
/// is empty still renders — and, when `topic` is given, a third field equal
/// to it: `<ts> — <author>`, then ` [<topic>]` unless the topic is
/// [`DEFAULT_TOPIC`], a newline, the fourth field, two newlines. Bytes are
/// copied as they are; a carriage return inside a field stays inside it.
///
/// ```
/// use ae::memo::render;
///
/// let file = b"t1\tcl:lead\tgeneral\tplain\nt2\tcl:lead\tp2\ttopical\nshort\tline\n";
/// assert_eq!(
///     render(file, None),
///     "t1 — cl:lead\nplain\n\nt2 — cl:lead [p2]\ntopical\n\n".as_bytes()
/// );
/// assert_eq!(render(file, Some("p2")), "t2 — cl:lead [p2]\ntopical\n\n".as_bytes());
/// ```
#[must_use]
pub fn render(container: &[u8], topic: Option<&str>) -> Vec<u8> {
    let mut out = Vec::new();
    for record in container.split(|byte| *byte == b'\n') {
        let fields: Vec<&[u8]> = record.split(|byte| *byte == b'\t').collect();
        let [ts, author, record_topic, text, ..] = fields.as_slice() else {
            continue;
        };
        if topic.is_some_and(|wanted| *record_topic != wanted.as_bytes()) {
            continue;
        }
        out.extend_from_slice(ts);
        out.extend_from_slice(" — ".as_bytes());
        out.extend_from_slice(author);
        if *record_topic != DEFAULT_TOPIC.as_bytes() {
            out.extend_from_slice(b" [");
            out.extend_from_slice(record_topic);
            out.push(b']');
        }
        out.push(b'\n');
        out.extend_from_slice(text);
        out.extend_from_slice(b"\n\n");
    }
    out
}

/// The stdout of a [`View`] over the memo file at `dir`.
///
/// # Errors
///
/// A regular memo file that could not be read. Anything `[[ -f ]]` rejects —
/// absent, a directory, a FIFO, a socket — is empty output, not an error, and
/// is never opened.
pub fn read(dir: &Path, view: &View) -> io::Result<Vec<u8>> {
    let path = dir.join(FILE);
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the frozen `[[ -f \"$MEMO_FILE\" ]]` gate, before the memo file is opened — see clippy.toml"
    )]
    let regular = path.is_file();
    if !regular {
        return Ok(Vec::new());
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the memo file read behind `memo read` and `memo tail` — see clippy.toml"
    )]
    let container = std::fs::read(&path)?;
    Ok(match view {
        View::All(topic) => render(&container, topic.as_deref()),
        View::Tail(count) => render(&crate::event_text::last_records(&container, *count), None),
    })
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "tests read back what the door wrote and check their own fixtures; the boundary is on product code — see clippy.toml"
)]
mod tests {
    use super::{Add, Command, DEFAULT_TAIL, Usage, View, one_line, parse, read, record, render};
    use crate::time::Timestamp;
    use std::os::unix::fs::PermissionsExt;

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    fn added(topic: &str, text: &str) -> Command {
        Command::Add(Add {
            topic: topic.to_owned(),
            text: text.to_owned(),
        })
    }

    #[test]
    fn argv_reads_as_the_helper_reads_it() {
        assert_eq!(
            parse(&words(&["add", "two", "words"])),
            Ok(added("general", "two words"))
        );
        assert_eq!(
            parse(&words(&["add", "--topic", "p2", "note"])),
            Ok(added("p2", "note"))
        );
        assert_eq!(
            parse(&words(&["add", "--topic", "p2"])),
            Err(Usage),
            "no text"
        );
        assert_eq!(parse(&words(&["add", "--topic"])), Err(Usage));
        assert_eq!(parse(&words(&["add"])), Err(Usage));
        assert_eq!(
            parse(&words(&["add", "--topic", "", "x"])),
            Ok(added("general", "x")),
            "an empty topic is the default"
        );

        let all = |topic: Option<&str>| Ok(Command::View(View::All(topic.map(ToOwned::to_owned))));
        assert_eq!(parse(&[]), all(None), "no word is read");
        assert_eq!(parse(&words(&["read"])), all(None));
        assert_eq!(parse(&words(&["read", "--topic", "p2"])), all(Some("p2")));
        assert_eq!(
            parse(&words(&["read", "--topic", ""])),
            all(None),
            "an empty filter filters nothing"
        );
        assert_eq!(parse(&words(&["read", "--topic"])), Err(Usage));
        assert_eq!(parse(&words(&["read", "x"])), Err(Usage));
        assert_eq!(
            parse(&words(&["read", "--topic", "p2", "x"])),
            Err(Usage),
            "exactly two words after read"
        );

        let tail = |count: usize| Ok(Command::View(View::Tail(count)));
        assert_eq!(parse(&words(&["tail"])), tail(DEFAULT_TAIL));
        assert_eq!(
            parse(&words(&["tail", ""])),
            tail(DEFAULT_TAIL),
            "${{1:-10}}"
        );
        assert_eq!(parse(&words(&["tail", "3"])), tail(3));
        assert_eq!(parse(&words(&["tail", "0"])), tail(0));
        assert_eq!(
            parse(&words(&["tail", "99999999999999999999999"])),
            tail(usize::MAX)
        );
        assert_eq!(parse(&words(&["tail", "-1"])), Err(Usage));
        assert_eq!(parse(&words(&["tail", "3", "4"])), Err(Usage));
        assert_eq!(parse(&words(&["show"])), Err(Usage));
    }

    // The fixture and both renderings were MEASURED on the frozen awk program
    // (`helper_memo_render`, macOS awk, 2026-08-27), record by record: a
    // three-field line is skipped, a five-field line renders its fourth field
    const FIXTURE: &[u8] = b"2026-01-01T00:00:00Z\tcl:lead\tgeneral\tplain note\n\
2026-01-01T00:00:01Z\tcl:lead\tp2\ttopic note\n\
short\tline\tonly\n\
2026-01-01T00:00:02Z\tcl:lead\tgeneral\tfour\tfive fields\n\
\n\
2026-01-01T00:00:03Z\tcl:lead\tp2\t\n\
2026-01-01T00:00:04Z\tcl:lead\tgeneral\twith cr\r\n\
2026-01-01T00:00:05Z\tcl:lead\tp2\tunterminated tail";

    const RENDERED_ALL: &str = "2026-01-01T00:00:00Z — cl:lead\nplain note\n\n\
2026-01-01T00:00:01Z — cl:lead [p2]\ntopic note\n\n\
2026-01-01T00:00:02Z — cl:lead\nfour\n\n\
2026-01-01T00:00:03Z — cl:lead [p2]\n\n\n\
2026-01-01T00:00:04Z — cl:lead\nwith cr\r\n\n\
2026-01-01T00:00:05Z — cl:lead [p2]\nunterminated tail\n\n";

    const RENDERED_P2: &str = "2026-01-01T00:00:01Z — cl:lead [p2]\ntopic note\n\n\
2026-01-01T00:00:03Z — cl:lead [p2]\n\n\n\
2026-01-01T00:00:05Z — cl:lead [p2]\nunterminated tail\n\n";

    const RENDERED_TAIL_2: &str = "2026-01-01T00:00:04Z — cl:lead\nwith cr\r\n\n\
2026-01-01T00:00:05Z — cl:lead [p2]\nunterminated tail\n\n";

    #[test]
    fn render_is_the_measured_awk_output_byte_for_byte() {
        assert_eq!(render(FIXTURE, None), RENDERED_ALL.as_bytes());
        assert_eq!(render(FIXTURE, Some("p2")), RENDERED_P2.as_bytes());
        assert_eq!(render(FIXTURE, Some("nosuch")), b"");
        assert_eq!(render(b"", None), b"");
    }

    #[test]
    fn read_renders_the_file_or_its_tail_and_says_so_when_it_cannot_read_one() {
        let dir = std::path::PathBuf::from(format!("/tmp/ae-memo-read-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            read(&dir, &View::All(None)).unwrap(),
            b"",
            "no memo file yet is nothing to show"
        );
        assert_eq!(read(&dir, &View::Tail(2)).unwrap(), b"");
        std::fs::write(dir.join("memo.tsv"), FIXTURE).unwrap();
        assert_eq!(
            read(&dir, &View::All(None)).unwrap(),
            RENDERED_ALL.as_bytes()
        );
        assert_eq!(
            read(&dir, &View::All(Some("p2".to_owned()))).unwrap(),
            RENDERED_P2.as_bytes()
        );
        assert_eq!(
            read(&dir, &View::Tail(2)).unwrap(),
            RENDERED_TAIL_2.as_bytes(),
            "tail -n 2 counts the unterminated record"
        );
        assert_eq!(read(&dir, &View::Tail(0)).unwrap(), b"");
        assert_eq!(
            read(&dir, &View::Tail(usize::MAX)).unwrap(),
            RENDERED_ALL.as_bytes()
        );
        // Anything `-f` rejects is the frozen empty answer, never opened: a
        // directory, a socket (bound here from safe std — the FIFO that would
        // BLOCK an ungated open needs mkfifo and is covered black-box).
        std::fs::remove_file(dir.join("memo.tsv")).unwrap();
        std::fs::create_dir_all(dir.join("memo.tsv")).unwrap();
        assert_eq!(read(&dir, &View::All(None)).unwrap(), b"", "a directory");
        std::fs::remove_dir_all(dir.join("memo.tsv")).unwrap();
        let socket = std::os::unix::net::UnixListener::bind(dir.join("memo.tsv")).unwrap();
        assert_eq!(read(&dir, &View::Tail(1)).unwrap(), b"", "a socket");
        drop(socket);
        std::fs::remove_file(dir.join("memo.tsv")).unwrap();
        // A REGULAR file that cannot be read is the one reported case.
        std::fs::write(dir.join("memo.tsv"), FIXTURE).unwrap();
        std::fs::set_permissions(dir.join("memo.tsv"), std::fs::Permissions::from_mode(0o000))
            .unwrap();
        if std::fs::read(dir.join("memo.tsv")).is_err() {
            assert!(
                read(&dir, &View::All(None)).is_err(),
                "a regular memo file that exists but cannot be read is reported"
            );
        }
        std::fs::set_permissions(dir.join("memo.tsv"), std::fs::Permissions::from_mode(0o644))
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
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
