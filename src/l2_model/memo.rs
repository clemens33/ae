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
//! A bare `read` is the bounded [`overview`]: the newest record of each of the
//! newest topics, with a hint to the full read. `read --all` and
//! `read --topic <topic>` render ([`render`]) the whole file or one topic of it,
//! and `tail [n]` its last `n` records ([`crate::event_text::last_records`] —
//! what `tail -n` counts). A regular-file gate decides first: no `memo.tsv`, or anything but a
//! regular file in its place — a directory, a FIFO — is empty output at 0. The
//! gate comes BEFORE the open, because a FIFO opened without it blocks the
//! reader for good. A regular file that then cannot be read is reported at 1.
use std::collections::HashMap;
use std::io;
use std::path::Path;

use crate::requests::Viewer;
use crate::state;
use crate::store;
use crate::time::Timestamp;
use crate::tracked::{self, EventFields};

/// The usage text.
pub const USAGE: &str = "Usage: memo add [--topic <topic>] <text> | memo read [--all | --topic <topic>] | memo tail [n]\n";

/// The topic when none is given or the given one is empty.
pub const DEFAULT_TOPIC: &str = "general";

/// `tail` with no count — `${1:-10}`.
pub const DEFAULT_TAIL: usize = 10;

/// How many topics a bare `memo read` shows: the newest ones.
pub const OVERVIEW_TOPICS: usize = 20;

/// How many characters of one record's text a bare `memo read` shows.
pub const OVERVIEW_TEXT_CHARS: usize = 1_000;

/// What the caller asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `memo add [--topic <topic>] <text…>`.
    Add(Add),
    /// `memo`, `memo read [--all | --topic <topic>]`, `memo tail [n]`.
    View(View),
}

/// A read of the memo file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    /// The newest record of each of the newest [`OVERVIEW_TOPICS`] topics.
    Overview,
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
        return Ok(Command::View(View::Overview));
    };
    match command.as_str() {
        "add" => parse_add(rest).map(Command::Add),
        "read" => match rest {
            [] => Ok(Command::View(View::Overview)),
            [flag] if flag == "--all" => Ok(Command::View(View::All(None))),
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
    run_observed(dir, viewer, add, now, || tracked::observe_caller(dir))
}

/// The writer body. Tests inject the observation so a durable-cut pin does not
/// need a live tmux.
pub(crate) fn run_observed(
    dir: &Path,
    viewer: &Viewer,
    add: &Add,
    now: Timestamp,
    observe: impl FnMut() -> Result<tracked::IdentityTriple, tracked::CorrelationGap>,
) -> Result<(), Failure> {
    let author = if viewer.is_known() {
        viewer.display.as_str()
    } else {
        "human"
    };
    let store = store::open(dir);
    let (tsv, outcome) = tracked::across_cut_with(observe, || {
        store.append_memo(record(now, author, add).as_bytes())
    });
    tsv.map_err(|why| Failure::Tsv(why.into()))?;
    let summary = state::summary_of(&add.text);
    let fields = tracked::stamp_caller(
        &EventFields::new(
            now, author, "memo", "", &add.topic, "", "", "", "", &summary, "",
        ),
        &outcome,
    );
    let event = tracked::event_line(&fields);
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
        push_record(&mut out, &record, record.text);
    }
    out
}

/// One record as `memo read` prints it, with `text` in place of its own.
fn push_record(out: &mut Vec<u8>, record: &Record<'_>, text: &[u8]) {
    out.extend_from_slice(record.ts);
    out.extend_from_slice(" — ".as_bytes());
    out.extend_from_slice(record.author);
    if record.topic != DEFAULT_TOPIC.as_bytes() {
        out.extend_from_slice(b" [");
        out.extend_from_slice(record.topic);
        out.push(b']');
    }
    out.push(b'\n');
    out.extend_from_slice(text);
    out.extend_from_slice(b"\n\n");
}

/// The bounded read a bare `memo read` gives: the newest record of each of the
/// newest [`OVERVIEW_TOPICS`] topics, in file order, each text cut at
/// [`OVERVIEW_TEXT_CHARS`] characters (the last one the ellipsis).
///
/// A hint line follows when the output is less than the file — a record
/// superseded on its topic, a topic past the cap or a cut text — and names the
/// counts and both ways to read more. An overview that hides nothing is the full
/// render, byte for byte, and carries no hint.
///
/// Total over hostile bytes: it only walks [`records`], so a line that is not a
/// record is skipped as everywhere else.
#[must_use]
pub fn overview(container: &[u8]) -> Vec<u8> {
    let records: Vec<Record<'_>> = records(container).collect();
    let mut newest: HashMap<&[u8], usize> = HashMap::new();
    for (at, record) in records.iter().enumerate() {
        newest.insert(record.topic, at);
    }
    let topics = newest.len();
    let mut kept: Vec<usize> = newest.into_values().collect();
    kept.sort_unstable();
    let shown = &kept[topics.saturating_sub(OVERVIEW_TOPICS)..];
    let mut out = Vec::new();
    let mut cut = false;
    for &at in shown {
        let record = &records[at];
        let text = clipped(record.text);
        cut |= text != record.text;
        push_record(&mut out, record, &text);
    }
    if shown.len() < records.len() || cut {
        let cut_note = if cut {
            format!("; long texts are cut at {OVERVIEW_TEXT_CHARS} characters")
        } else {
            String::new()
        };
        out.extend_from_slice(
            format!(
                "(memo read shows the newest record of {} of {topics} topics, {} records in all{cut_note}. \
                 Every record: memo read --all. One topic's history: memo read --topic <topic>.)\n",
                shown.len(),
                records.len()
            )
            .as_bytes(),
        );
    }
    out
}

/// `text` as an overview shows it: whole when it fits [`OVERVIEW_TEXT_CHARS`]
/// characters, else its first characters and an ellipsis in exactly that many.
/// Bytes are kept as they are unless a cut is made, which is the only time the
/// text is read as characters.
fn clipped(text: &[u8]) -> Vec<u8> {
    if text.len() <= OVERVIEW_TEXT_CHARS {
        return text.to_vec();
    }
    let chars = String::from_utf8_lossy(text);
    if chars.chars().count() <= OVERVIEW_TEXT_CHARS {
        return text.to_vec();
    }
    let mut kept: String = chars.chars().take(OVERVIEW_TEXT_CHARS - 1).collect();
    kept.push('…');
    kept.into_bytes()
}

/// The stdout of a [`View`] over `container`.
///
/// Pure: the bytes come from [`crate::store::SessionStore::memo_bytes`], which
/// owns the `[[ -f ]]` gate and the read. An empty container renders as no
/// output, which is also what the gate's quiet answer produces.
#[must_use]
pub fn view(container: &[u8], view: &View) -> Vec<u8> {
    match view {
        View::Overview => overview(container),
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
        Add, Command, DEFAULT_TAIL, DEFAULT_TOPIC, OVERVIEW_TEXT_CHARS, OVERVIEW_TOPICS, Record,
        Usage, View, one_line, parse, record, records, render, view,
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
            let _ = view(&container, &View::Overview);
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
        let overview = Ok(Command::View(View::Overview));
        assert_eq!(parse(&[]), overview, "no word is a bare read");
        assert_eq!(parse(&words(&["read"])), overview, "a bare read is bounded");
        assert_eq!(
            parse(&words(&["read", "--all"])),
            all(None),
            "the full file is an explicit word"
        );
        assert_eq!(
            parse(&words(&["read", "--all", "x"])),
            Err(Usage),
            "--all takes nothing after it"
        );
        assert_eq!(
            parse(&words(&["read", "--all", "--topic", "p2"])),
            Err(Usage),
            "--all and --topic do not combine"
        );
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

    fn filed(stamp: &str, topic: &str, text: &str) -> String {
        format!("{stamp}\tcl:lead\t{topic}\t{text}\n")
    }

    fn overview(container: &str) -> String {
        String::from_utf8(view(container.as_bytes(), &View::Overview)).expect("utf8 out")
    }

    #[test]
    fn an_overview_keeps_the_newest_record_of_each_topic_in_file_order_and_hints() {
        let container = [
            filed("t0", "a", "a-old"),
            filed("t1", "b", "b-only"),
            filed("t2", "a", "a-new"),
        ]
        .concat();
        assert_eq!(
            overview(&container),
            "t1 — cl:lead [b]\nb-only\n\nt2 — cl:lead [a]\na-new\n\n\
             (memo read shows the newest record of 2 of 2 topics, 3 records in all. \
             Every record: memo read --all. One topic's history: memo read --topic <topic>.)\n",
            "a superseded record is hidden, so the hint names the full read"
        );
    }

    #[test]
    fn an_overview_that_hides_nothing_is_the_full_read_without_a_hint() {
        // One record per topic, short texts, under the topic cap: the overview
        // IS the file, so the bytes are the full render's and nothing is hinted.
        let container = [
            filed("t0", "a", "one"),
            filed("t1", DEFAULT_TOPIC, "two"),
            filed("t2", "c", ""),
        ]
        .concat();
        assert_eq!(
            view(container.as_bytes(), &View::Overview),
            view(container.as_bytes(), &View::All(None))
        );
        assert!(!overview(&container).contains("memo read"));
    }

    #[test]
    fn an_overview_shows_only_the_newest_topics_and_says_how_many_it_left() {
        let topics = |count: usize| -> String {
            (0..count)
                .map(|n| filed(&format!("t{n:03}"), &format!("topic{n:03}"), "x"))
                .collect()
        };
        // Exactly the cap: every topic shows and the output is the file.
        let at_cap = topics(OVERVIEW_TOPICS);
        assert!(overview(&at_cap).contains("topic000"));
        assert!(
            !overview(&at_cap).contains("memo read"),
            "nothing is hidden"
        );
        // One over: the OLDEST topic goes, the newest stay, and the hint counts.
        let over = topics(OVERVIEW_TOPICS + 1);
        let shown = overview(&over);
        assert!(!shown.contains("topic000"), "the oldest topic is cut");
        assert!(shown.contains("topic001"));
        assert!(shown.contains(&format!("topic{OVERVIEW_TOPICS:03}")));
        assert!(
            shown.contains(&format!(
                "the newest record of {OVERVIEW_TOPICS} of {} topics, {} records in all",
                OVERVIEW_TOPICS + 1,
                OVERVIEW_TOPICS + 1
            )),
            "{shown}"
        );
        assert_eq!(
            shown.matches(" — cl:lead").count(),
            OVERVIEW_TOPICS,
            "exactly the cap is rendered"
        );
    }

    #[test]
    fn an_overview_cuts_a_long_text_at_the_cap_and_says_so() {
        let exact = "x".repeat(OVERVIEW_TEXT_CHARS);
        let whole = overview(&filed("t0", "a", &exact));
        assert!(whole.contains(&exact), "a text at the cap is whole");
        assert!(!whole.contains("memo read"), "and nothing is hinted");

        let long = "y".repeat(OVERVIEW_TEXT_CHARS + 1);
        let cut = overview(&filed("t0", "a", &long));
        let expected = format!("{}…", "y".repeat(OVERVIEW_TEXT_CHARS - 1));
        assert!(
            cut.contains(&format!("\n{expected}\n\n")),
            "the cut text is the cap including its ellipsis: {cut}"
        );
        assert!(!cut.contains(&long));
        assert!(
            cut.contains(&format!(
                "long texts are cut at {OVERVIEW_TEXT_CHARS} characters"
            )),
            "{cut}"
        );

        // Two-byte characters: the cut counts characters, never bytes, and
        // never splits one.
        let wide = "é".repeat(OVERVIEW_TEXT_CHARS + 1);
        let cut = overview(&filed("t0", "a", &wide));
        assert!(cut.contains(&format!("{}…", "é".repeat(OVERVIEW_TEXT_CHARS - 1))));
        let narrow = "é".repeat(OVERVIEW_TEXT_CHARS);
        assert!(
            overview(&filed("t0", "a", &narrow)).contains(&narrow),
            "a text of cap characters that is over the cap in bytes is whole"
        );
    }

    #[test]
    fn an_overview_of_nothing_is_nothing() {
        assert_eq!(view(b"", &View::Overview), b"");
        assert_eq!(
            view(b"not\ta record\nshort\tline\n", &View::Overview),
            b"",
            "lines that are not records leave nothing to show or to hint"
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

    #[test]
    fn memo_event_bytes_carry_caller_facts_only_when_the_cut_holds() {
        use crate::requests::Viewer;
        use crate::tracked::IdentityTriple;
        let dir = std::env::temp_dir().join(format!("ae-memo-cut.{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ts = Timestamp::parse("2026-08-27T08:00:00Z").unwrap();
        let add = Add {
            topic: "goal".to_owned(),
            text: "checkpoint".to_owned(),
        };
        let viewer = Viewer {
            slot: "main".to_owned(),
            session: "s".to_owned(),
            display: "cl:lead".to_owned(),
            session_id: String::new(),
        };
        let held = IdentityTriple {
            server: "/tmp/ae".to_owned(),
            pane: "%3".to_owned(),
            session_uuid: "1b4e28ba-2fa1-11d2-883f-0016d3cc4321".to_owned(),
        };
        crate::tracked::clear_observe();
        crate::tracked::queue_observe(Ok(held.clone()));
        crate::tracked::queue_observe(Ok(held.clone()));
        super::run(&dir, &viewer, &add, ts).expect("memo writes");
        let events = std::fs::read_to_string(dir.join("events.jsonl")).expect("events");
        assert!(events.contains(r#""action":"memo""#));
        assert!(events.contains(r#""caller_server":"/tmp/ae""#));
        assert!(events.contains(r#""caller_pane":"%3""#));
        std::fs::write(dir.join("events.jsonl"), b"").unwrap();
        crate::tracked::clear_observe();
        crate::tracked::queue_observe(Ok(held.clone()));
        crate::tracked::queue_observe(Ok(IdentityTriple {
            pane: "%4".to_owned(),
            ..held.clone()
        }));
        super::run(&dir, &viewer, &add, ts).expect("memo writes");
        let events = std::fs::read_to_string(dir.join("events.jsonl")).expect("events");
        assert!(
            !events.contains("caller_server"),
            "changed identity writes no correlated memo event"
        );
        assert!(
            events.contains(r#""identity_gap":"session identity changed""#),
            "a changed identity is named on the durable record: {events}"
        );
        crate::tracked::clear_observe();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
