//! The `memo` helper: `add`, `read` and `tail`.
//!
//! `add` writes one TSV record
//! `<ts>\t<author>\t<topic>\t<text>` appended to `memo.tsv` under
//! `memo.tsv.lock` — carriage returns dropped, tabs and newlines to spaces in
//! both topic and text, an empty topic is `general` — then a `memo` event
//! (`ref` = topic, `summary` = text) on the container. Nothing on stdout.
//!
//! Two files, two locks, two transactions. A failure between them
//! is reported as exactly that: recorded in `memo.tsv`, event not emitted.
//!
//! `read [--topic <topic>]` and `tail [n]` render ([`render`]) the whole file or
//! its last `n` records ([`crate::event_text::last_records`] — what `tail -n`
//! counts). A regular-file gate decides first: no `memo.tsv`, or anything but a
//! regular file in its place — a directory, a FIFO — is empty output at 0. The
//! gate comes BEFORE the open, because a FIFO opened without it blocks the
//! reader for good. A regular file that then cannot be read is reported at 1.
use std::io;
use std::path::Path;

use crate::requests::Viewer;
use crate::state;
use crate::store;
use crate::time::Timestamp;

/// The usage text.
pub const USAGE: &str =
    "Usage: memo add [--topic <topic>] <text> | memo read [--topic <topic>] | memo tail [n]\n";

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
/// a count too large for `usize` is every record, which is what GNU `tail`
/// gives).
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
        let file = store::MEMO;
        match self {
            Self::Read(why) => format!("ae: memo not read: could not read {file}: {why}"),
            Self::Tsv(why) => format!("ae: memo not recorded: could not append to {file}: {why}"),
            Self::Event(why) => {
                format!("ae: memo recorded in {file} but its event was not emitted: {why}")
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
    let store = store::open(dir);
    store
        .append_memo(record(now, author, add).as_bytes())
        .map_err(|why| Failure::Tsv(why.into()))?;
    let event = state::event_line(
        now,
        author,
        "memo",
        &add.topic,
        &state::summary_of(&add.text),
    );
    store
        .append_event(&event)
        .map_err(|why| Failure::Event(why.into()))
}

/// One `memo.tsv` record: the four fields, BORROWED from the container.
///
/// # This is the file's one grammar, and `memo.tsv` is hand-editable
///
/// A memo file is persisted state a human can open in an editor, so it is
/// hostile input and every reader of it has to be total. Keeping ONE parser is
/// what makes that a property of the file rather than a property each caller
/// re-argues: [`render`] and [`crate::brief::topic_lines`] both come through
/// here, so neither can drift into a second dialect and only this function has
/// to be proven.
///
/// Total by construction: [`slice::split`] over a single byte, four `next()`
/// calls that return `None` rather than index, and no arithmetic. No byte
/// sequence reaches a panic — `no_byte_sequence_makes_the_record_parser_panic`
/// exercises it directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record<'a> {
    /// The first field: when the record was written.
    pub ts: &'a [u8],
    /// The second: who wrote it.
    pub author: &'a [u8],
    /// The third: the topic it was filed under.
    pub topic: &'a [u8],
    /// The fourth: the text. A record with MORE fields keeps only this one.
    pub text: &'a [u8],
}

impl<'a> Record<'a> {
    /// Read one `\t`-separated record, or `None` when it carries fewer than
    /// four fields.
    ///
    /// awk `-F '\t'` with `NF >= 4`, which is what the helper this replaced
    /// ran: a record with MORE fields keeps its fourth and drops the rest, and a
    /// record whose fourth is EMPTY is still a record.
    ///
    /// ```
    /// use ae::memo::Record;
    ///
    /// let full = Record::parse(b"t1\tcl:lead\tgeneral\tplain").unwrap();
    /// assert_eq!(full.topic, b"general");
    /// assert_eq!(full.text, b"plain");
    /// // A fifth field is dropped, an empty fourth is kept, three is not a record.
    /// assert_eq!(Record::parse(b"a\tb\tc\td\te").unwrap().text, b"d");
    /// assert_eq!(Record::parse(b"a\tb\tc\t").unwrap().text, b"");
    /// assert_eq!(Record::parse(b"a\tb\tc"), None);
    /// assert_eq!(Record::parse(b""), None);
    /// ```
    #[must_use]
    pub fn parse(record: &'a [u8]) -> Option<Self> {
        let mut fields = record.split(|byte| *byte == b'\t');
        Some(Self {
            ts: fields.next()?,
            author: fields.next()?,
            topic: fields.next()?,
            text: fields.next()?,
        })
    }
}

/// Every record in a `memo.tsv` container, in file order.
///
/// `\n`-separated, and a line that is not a record is SKIPPED rather than
/// repaired — a half-written append at the end of the file costs its own line
/// and nothing before it.
///
/// ```
/// use ae::memo::records;
///
/// let file = b"t1\tcl:lead\tgeneral\tplain\nnot-a-record\nt2\tcl:lead\tp2\ttopical\n";
/// let topics: Vec<&[u8]> = records(file).map(|record| record.topic).collect();
/// assert_eq!(topics, [b"general".as_slice(), b"p2".as_slice()]);
/// ```
pub fn records(container: &[u8]) -> impl Iterator<Item = Record<'_>> {
    container
        .split(|byte| *byte == b'\n')
        .filter_map(Record::parse)
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
    for record in records(container) {
        if topic.is_some_and(|wanted| record.topic != wanted.as_bytes()) {
            continue;
        }
        out.extend_from_slice(record.ts);
        out.extend_from_slice(" — ".as_bytes());
        out.extend_from_slice(record.author);
        if record.topic != DEFAULT_TOPIC.as_bytes() {
            out.extend_from_slice(b" [");
            out.extend_from_slice(record.topic);
            out.push(b']');
        }
        out.push(b'\n');
        out.extend_from_slice(record.text);
        out.extend_from_slice(b"\n\n");
    }
    out
}

/// The stdout of a [`View`] over `container`.
///
/// Pure: the bytes come from [`crate::store::SessionStore::memo_bytes`], which
/// owns the `[[ -f ]]` gate and the read. An empty container renders as no
/// output, which is also what the gate's quiet answer produces.
#[must_use]
pub fn view(container: &[u8], view: &View) -> Vec<u8> {
    match view {
        View::All(topic) => render(container, topic.as_deref()),
        View::Tail(count) => render(&crate::event_text::last_records(container, *count), None),
    }
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "tests read back what the door wrote and check their own fixtures; the boundary is on product code — see clippy.toml"
)]
mod tests {
    use super::{
        Add, Command, DEFAULT_TAIL, Record, Usage, View, one_line, parse, record, records, render,
        view,
    };
    use crate::time::Timestamp;

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    fn added(topic: &str, text: &str) -> Command {
        Command::Add(Add {
            topic: topic.to_owned(),
            text: text.to_owned(),
        })
    }

    /// THE PARSER'S ONE CONTRACT, and the reason it is the only one.
    ///
    /// `memo.tsv` is persisted state a human can open in an editor, so every
    /// byte sequence has to yield a list of records rather than a panic. Both
    /// readers come through here, so proving it once proves it for `memo read`
    /// and for `ae brief` alike.
    #[test]
    fn no_byte_sequence_makes_the_record_parser_panic() {
        let mut adversarial: Vec<Vec<u8>> = vec![
            Vec::new(),
            b"\n".to_vec(),
            b"\t\t\t".to_vec(),
            b"\t\t\t\n\t\t\t".to_vec(),
            b"a\tb\tc".to_vec(),
            b"a\tb\tc\td\te\tf".to_vec(),
            b"\xff\xfe\tauthor\ttopic\ttext".to_vec(),
            "\u{feff}\thuman\t\u{202e}\t\u{0}text"
                .to_string()
                .into_bytes(),
            b"9223372036854775807\thuman\tt\tx".to_vec(),
            b"-9223372036854775808\thuman\tt\tx".to_vec(),
            format!(
                "{}\t{}\t{}\t{}",
                "9".repeat(400),
                "a".repeat(400),
                "t".repeat(400),
                "x".repeat(400)
            )
            .into_bytes(),
            b"1970-01-01T00:00:00Z\thuman\tt\tx".to_vec(),
        ];
        // A reproducible byte soup: every value 0..=255 in shifting positions,
        // so tabs, newlines, control bytes and broken UTF-8 land everywhere.
        let mut seed = 0x2026_0906_u64;
        for _ in 0..512 {
            let mut record = Vec::new();
            for _ in 0..64 {
                seed = seed
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                #[allow(clippy::cast_possible_truncation, reason = "a byte is what this wants")]
                record.push((seed >> 33) as u8);
            }
            adversarial.push(record);
        }
        for container in adversarial {
            for record in records(&container) {
                // The INVARIANT beyond "did not panic": four fields, every one
                // of them a slice OF THE INPUT rather than something invented.
                let within = |field: &[u8]| {
                    field.is_empty() || container.windows(field.len()).any(|window| window == field)
                };
                assert!(within(record.ts), "ts is not from the container");
                assert!(within(record.author), "author is not from the container");
                assert!(within(record.topic), "topic is not from the container");
                assert!(within(record.text), "text is not from the container");
            }
            // And the renderer over the same bytes, since it is the other
            // consumer and shares the parser's totality by construction.
            let _ = render(&container, None);
            let _ = render(&container, Some("general"));
        }
    }

    #[test]
    fn a_record_needs_four_fields_and_keeps_only_the_fourth() {
        assert_eq!(Record::parse(b"a\tb\tc"), None, "three is not a record");
        let five = Record::parse(b"a\tb\tc\td\te").expect("five is");
        assert_eq!(five.text, b"d", "the fifth field is dropped, not appended");
        let empty_text = Record::parse(b"a\tb\tc\t").expect("an empty fourth is a record");
        assert_eq!(empty_text.text, b"");
        assert_eq!(
            records(b"a\tb\tc\td\nbad\ne\tf\tg\th").count(),
            2,
            "a line that is not a record costs its own line and nothing else"
        );
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

    // The fixture and both renderings, record by record: a
    // three-field line is skipped, a five-field line renders its fourth field
    // only, an empty line is skipped, an empty text renders as an empty line,
    // a carriage return stays inside the text, and an unterminated last record
    // still renders.
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
    fn a_view_renders_the_whole_container_or_its_tail() {
        // The gate and the read are the store's (see its `memo_bytes` tests);
        // what is left here is pure, so the fixture IS the file.
        assert_eq!(view(b"", &View::All(None)), b"", "nothing to show");
        assert_eq!(view(b"", &View::Tail(2)), b"");
        assert_eq!(view(FIXTURE, &View::All(None)), RENDERED_ALL.as_bytes());
        assert_eq!(
            view(FIXTURE, &View::All(Some("p2".to_owned()))),
            RENDERED_P2.as_bytes()
        );
        assert_eq!(
            view(FIXTURE, &View::Tail(2)),
            RENDERED_TAIL_2.as_bytes(),
            "tail -n 2 counts the unterminated record"
        );
        assert_eq!(view(FIXTURE, &View::Tail(0)), b"");
        assert_eq!(
            view(FIXTURE, &View::Tail(usize::MAX)),
            RENDERED_ALL.as_bytes()
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
