//! Links in lane text: what in a body becomes a terminal hyperlink.
//!
//! PURE. No I/O, no world read: the caller hands in where a relative path
//! resolves ([`Base`]) and gets back byte ranges of the text it scanned. The
//! chat ([`super::view::Style`]) and `ae app` ([`crate::app`]'s lane) both draw
//! what this finds, so one grammar serves both.
//!
//! A link is a web URL (`http://`, `https://`) or a repo-style path: the
//! prefixes `.local/`, `docs/`, `src/`, `tests/` (against the session's work
//! dir), `~/` (against the home) and an absolute `/…` of at least two
//! segments, each with an optional `:line` and `:line:col` that stay in the
//! visible text and out of the URL. A file URL has an EMPTY host. No
//! filesystem is asked, so a shape-qualified path links whether or not it
//! exists; `..` is kept as written.
//!
//! The text it is given is the text that is drawn, already neutralised by
//! `board::terminal_text`; the scan is still safe on raw text, because a
//! control character ends every token and permits no new one, and every URL it
//! emits is printable ASCII ([`is_safe_url`]): what a URL may not carry raw is
//! percent-encoded, and a candidate that cannot be made safe is dropped.

use super::wrap::wrap_in;

/// Where a path that is not absolute resolves. A `dir` or `home` that is not
/// an absolute path names no base, so nothing resolves against it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Base {
    /// The session's recorded work dir: where `src/…` and its kin resolve.
    pub dir: Option<String>,
    /// The viewer's home: where `~/…` resolves.
    pub home: Option<String>,
}

/// A link in a text: a byte range at char boundaries, and the URL it opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub start: usize,
    pub end: usize,
    pub url: String,
}

/// One row of a wrapped text: what [`wrap_in`] makes of it, and the links that
/// fall in its piece, as byte ranges of the piece.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub gap: usize,
    pub piece: String,
    pub links: Vec<Link>,
}

/// Whether `url` may ride an OSC 8 sequence: printable ASCII only, so no
/// control, no space and no non-ASCII byte can end the sequence early or start
/// another. The emitters' last gate.
#[must_use]
pub fn is_safe_url(url: &str) -> bool {
    !url.is_empty() && url.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

/// The hint row line for a plain click on a link ae does not open itself: the
/// terminal opens it only with a modifier, because ae or tmux holds the mouse.
/// Shift bypasses that capture; the platform's link modifier then opens the
/// link. Fixed at compile time; nothing is read from the environment.
#[must_use]
pub fn open_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "Shift+Cmd+click to open link"
    } else {
        "Ctrl+click or Shift+click to open link"
    }
}

/// The characters, besides whitespace, a token may start after.
const OPENERS: &str = "([{<\"'`*=";

/// The relative path prefixes that name a file of the session's work dir.
const RELATIVE: [&str; 4] = [".local/", "docs/", "src/", "tests/"];

/// Whether a token may start right after `prev`.
fn starts(prev: Option<char>) -> bool {
    prev.is_none_or(|ch| ch.is_whitespace() || OPENERS.contains(ch))
}

/// A file link in a text, for a reader that opens it: the byte range as
/// [`Link`] has it (the `:N[:M]` suffix inside `end`), the absolute path the
/// link resolves to as it is written (not percent-encoded, `..` kept) and the
/// numbers of that suffix. A number too long to be one is `None`, alone: the
/// other keeps its value and the suffix stays inside `end`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRef {
    pub start: usize,
    pub end: usize,
    pub path: String,
    pub line: Option<usize>,
    pub col: Option<usize>,
}

/// Where a file link points, before its URL is made.
struct Target {
    path: String,
    line: Option<usize>,
    col: Option<usize>,
}

/// Every link in `text`, in order and disjoint. Total: any text scans.
#[must_use]
pub fn scan(text: &str, base: &Base) -> Vec<Link> {
    walk(text, base).into_iter().map(|(link, _)| link).collect()
}

/// The file links of `text`: exactly the file subset of [`scan`], same ranges
/// and order, each with its decoded target. Web links never appear. Total.
#[must_use]
pub fn files(text: &str, base: &Base) -> Vec<FileRef> {
    walk(text, base)
        .into_iter()
        .filter_map(|(link, target)| target.map(|target| file_ref(&link, &target)))
        .collect()
}

fn file_ref(link: &Link, target: &Target) -> FileRef {
    FileRef {
        start: link.start,
        end: link.end,
        path: target.path.clone(),
        line: target.line,
        col: target.col,
    }
}

/// A wrapped row and its file occurrences: each pair names the index in
/// `row.links` and the ORIGINAL source range/target, including any line suffix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub row: Row,
    pub files: Vec<(usize, FileRef)>,
}

/// The one walk both [`scan`] and [`files`] make: each link with the target of
/// a file link, `None` for a web one.
fn walk(text: &str, base: &Base) -> Vec<(Link, Option<Target>)> {
    let mut found = Vec::new();
    let (mut prev, mut at) = (None, 0);
    while let Some(ch) = text.get(at..).and_then(|rest| rest.chars().next()) {
        if starts(prev)
            && let Some((link, target)) = candidate(text, at, base)
        {
            prev = text
                .get(..link.end)
                .and_then(|head| head.chars().next_back());
            at = link.end;
            found.push((link, target));
            continue;
        }
        prev = Some(ch);
        at += ch.len_utf8();
    }
    found
}

/// The link starting at byte `at` of `text`, if one does, and its target when
/// it is a file link.
fn candidate(text: &str, at: usize, base: &Base) -> Option<(Link, Option<Target>)> {
    let rest = text.get(at..)?;
    let (len, url, target) = if let Some((len, url)) = web(rest) {
        (len, url, None)
    } else {
        let (len, url, target) = file(rest, base)?;
        (len, url, Some(target))
    };
    is_safe_url(&url).then_some((
        Link {
            start: at,
            end: at + len,
            url,
        },
        target,
    ))
}

/// The web URL `rest` starts with: its byte length and its URL.
fn web(rest: &str) -> Option<(usize, String)> {
    let scheme = ["https://", "http://"].into_iter().find(|scheme| {
        (rest.get(..scheme.len())).is_some_and(|head| head.eq_ignore_ascii_case(scheme))
    })?;
    let stop = rest
        .find(|ch: char| ch.is_whitespace() || ch.is_control() || matches!(ch, '<' | '>' | '"'))
        .unwrap_or(rest.len());
    let mut token = rest.get(..stop)?;
    // Counted once and kept as the tail is stripped, so a long run of
    // closers costs one pass.
    let count = |pair: [char; 2]| {
        let opens = token.matches(pair[0]).count();
        (opens, token.matches(pair[1]).count())
    };
    let mut brackets = [count(['(', ')']), count(['[', ']']), count(['{', '}'])];
    while let Some(last) = token.chars().next_back() {
        let closing = [')', ']', '}'].iter().position(|closer| *closer == last);
        let strip = match closing {
            Some(at) => brackets
                .get(at)
                .is_some_and(|(opens, closes)| closes > opens),
            None => matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | '\''),
        };
        if !strip {
            break;
        }
        if let Some((_, closes)) = closing.and_then(|at| brackets.get_mut(at)) {
            *closes -= 1;
        }
        token = token.get(..token.len() - last.len_utf8())?;
    }
    (token.len() > scheme.len()).then(|| (token.len(), encode_web(token)))
}

/// `token` as a URL: what may not ride raw is `%XX`, a `%` that is not
/// already an escape is `%25`.
fn encode_web(token: &str) -> String {
    let bytes = token.as_bytes();
    let mut out = String::with_capacity(token.len());
    for (at, byte) in bytes.iter().copied().enumerate() {
        let escape =
            (bytes.get(at + 1..at + 3)).is_some_and(|pair| pair.iter().all(u8::is_ascii_hexdigit));
        match byte {
            b'%' if escape => out.push('%'),
            b'%' | b'"' | b'<' | b'>' | b'\\' | b'^' | b'`' | b'{' | b'|' | b'}' => {
                escaped(&mut out, byte);
            }
            0x21..=0x7e => out.push(char::from(byte)),
            _ => escaped(&mut out, byte),
        }
    }
    out
}

/// `%XX` for `byte`, upper-hex.
fn escaped(out: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.push('%');
    out.push(char::from(HEX[usize::from(byte >> 4)]));
    out.push(char::from(HEX[usize::from(byte & 15)]));
}

/// Whether `ch` may stand in a path.
fn path_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '.' | '_' | '-' | '/' | '+' | '@' | '~' | '#' | '%')
}

/// Whether `text` holds a character that is not a slash.
fn has_name(text: &str) -> bool {
    text.chars().any(|ch| ch != '/')
}

/// The path `rest` starts with, its `:line[:col]` kept in the length: its
/// byte length, its `file://` URL and its target.
fn file(rest: &str, base: &Base) -> Option<(usize, String, Target)> {
    let run = rest.find(|ch: char| !path_char(ch)).unwrap_or(rest.len());
    let path = rest.get(..run)?.trim_end_matches('.');
    let absolute = if let Some(after) = path.strip_prefix("~/") {
        let home = base.home.as_deref().filter(|home| home.starts_with('/'))?;
        has_name(after).then(|| format!("{}/{after}", home.trim_end_matches('/')))?
    } else if path.starts_with('/') {
        let named = path.split('/').filter(|segment| !segment.is_empty());
        (named.count() >= 2).then(|| path.to_owned())?
    } else {
        let prefix = RELATIVE.iter().find(|prefix| path.starts_with(**prefix))?;
        let dir = base.dir.as_deref().filter(|dir| dir.starts_with('/'))?;
        has_name(path.get(prefix.len()..)?)
            .then(|| format!("{}/{path}", dir.trim_end_matches('/')))?
    };
    let (suffix, line, col) = line_suffix(rest.get(path.len()..)?);
    let url = encode_path(&absolute);
    Some((
        path.len() + suffix,
        url,
        Target {
            path: absolute,
            line,
            col,
        },
    ))
}

/// The byte length of the `:N` or `:N:M` (digits) `after` starts with, else 0,
/// and the two numbers: each `None` when absent or too long to be a `usize`.
fn line_suffix(after: &str) -> (usize, Option<usize>, Option<usize>) {
    let digits = |text: &str| text.bytes().take_while(u8::is_ascii_digit).count();
    let (mut len, mut numbers) = (0, [None, None]);
    for number in &mut numbers {
        let Some(rest) = after.get(len..).and_then(|rest| rest.strip_prefix(':')) else {
            break;
        };
        match digits(rest) {
            0 => break,
            n => {
                *number = rest.get(..n).and_then(|held| held.parse().ok());
                len += 1 + n;
            }
        }
    }
    let [line, col] = numbers;
    (len, line, col)
}

/// `path` as the path of a `file://` URL: every byte outside
/// `A-Za-z0-9-._~/` is `%XX`.
fn encode_path(path: &str) -> String {
    let mut out = String::from("file://");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(char::from(byte));
        } else {
            escaped(&mut out, byte);
        }
    }
    out
}

/// `text` wrapped as [`wrap_in`] wraps it, each row carrying the links that
/// fall in its piece. A link a wrap cuts is one link per row, the same URL.
///
/// The pieces are [`wrap_in`]'s exactly. A link is placed by its count of
/// non-whitespace characters before it, which the wrap keeps in order (it
/// only spends tabs and trims spaces) and a link, holding no whitespace,
/// cannot straddle.
#[must_use]
pub fn wrapped(text: &str, width: usize, first: usize, bar: usize, base: &Base) -> Vec<Row> {
    placed(text, width, first, bar, base)
        .into_iter()
        .map(|placed| placed.row)
        .collect()
}

/// The same placement as [`wrapped`], retaining each file occurrence across
/// row breaks. Duplicate paths and different line suffixes remain distinct.
#[must_use]
pub fn placed(text: &str, width: usize, first: usize, bar: usize, base: &Base) -> Vec<Placed> {
    let rows = wrap_in(text, width, first, bar);
    let links = walk(text, base);
    if links.is_empty() {
        let unlinked = |(gap, piece)| Placed {
            row: Row {
                gap,
                piece,
                links: Vec::new(),
            },
            files: Vec::new(),
        };
        return rows.into_iter().map(unlinked).collect();
    }
    let letters = |text: Option<&str>| {
        text.map_or(0, |text| {
            text.chars().filter(|ch| !ch.is_whitespace()).count()
        })
    };
    // Each link as the half-open range of non-whitespace characters it spans.
    let (mut seen, mut cursor) = (0, 0);
    let spans: Vec<(usize, usize)> = (links.iter())
        .map(|(link, _)| {
            seen += letters(text.get(cursor..link.start));
            let from = seen;
            seen += letters(text.get(link.start..link.end));
            cursor = link.end;
            (from, seen)
        })
        .collect();
    let (mut consumed, mut live) = (0, 0);
    let mut placed = Vec::with_capacity(rows.len());
    for (gap, piece) in rows {
        let marks: Vec<(usize, usize)> = (piece.char_indices())
            .filter(|(_, ch)| !ch.is_whitespace())
            .map(|(at, ch)| (at, at + ch.len_utf8()))
            .collect();
        let (lo, hi) = (consumed, consumed + marks.len());
        consumed = hi;
        while spans.get(live).is_some_and(|(_, to)| *to <= lo) {
            live += 1;
        }
        let (mut here, mut files) = (Vec::new(), Vec::new());
        for ((from, to), (link, target)) in spans.iter().zip(&links).skip(live) {
            if *from >= hi {
                break;
            }
            let (a, b) = (from.max(&lo) - lo, to.min(&hi) - lo);
            let ends = (b > a)
                .then(|| (marks.get(a), marks.get(b - 1)))
                .and_then(|pair| Some((pair.0?, pair.1?)));
            if let Some((head, tail)) = ends {
                if let Some(target) = target {
                    files.push((here.len(), file_ref(link, target)));
                }
                here.push(Link {
                    start: head.0,
                    end: tail.1,
                    url: link.url.clone(),
                });
            }
        }
        placed.push(Placed {
            row: Row {
                gap,
                piece,
                links: here,
            },
            files,
        });
    }
    placed
}
