//! Built-in syntax highlighting for the file reader (#274): a tiny, bounded
//! mapper from a line of source to coloured runs, for the reader's raw file
//! view and for the tagged fences of its Markdown view.
//!
//! PURE and INERT. A [`Scanner`] reads one line at a time and answers runs
//! (token, byte range) that TILE the line exactly, so the text is byte for byte
//! what it was and only its style differs. Nothing here reads, writes or
//! spawns. It does not parse: a language is a short table of keywords, quotes
//! and comment openers, one pass per line, no backtracking, and a line it
//! cannot make sense of comes back as plain runs. A multi-line construct (a
//! block comment, a triple-quoted or backtick string) is carried to the next
//! line of the same scanner and APPROXIMATED at the edges, never hiding text:
//! the raw reader recovers the state from at most [`LOOKBACK_LINES`] lines
//! above the first drawn row, a fence starts afresh and ends with its fence.
//!
//! Colour is TEXT here, so every drawn colour is a palette role the text bar
//! (4.5:1, `src/l2_model/theme.rs` tests) already pins on its real ground: the reader's
//! `ink` for a file, the `panel` of a fence. No new colour; a role that cannot
//! clear it on a ground wears the text colour and a modifier instead
//! ([`style`]).

use std::ops::Range;

use ratatui_core::style::{Modifier, Style};
use ratatui_core::text::{Line, Span};

use super::draw::Paint;
use super::reader::{self, Body, Flow, LINE_CHARS, View};
use crate::board::terminal_text;
use crate::console::wrap::expand_tabs;

/// The most lines above the first drawn row the raw reader reads to recover a
/// multi-line state: a block comment opened further up is not seen.
pub const LOOKBACK_LINES: usize = 500;
/// The most bytes that look-back reads.
pub const LOOKBACK_BYTES: usize = 65_536;
/// The most runs one Markdown layout or one raw-reader window colours, over
/// all its fences or lines: the rest is drawn plain.
pub const MAX_RUNS: usize = 200_000;

/// Whether a fence that may still colour `left` runs may colour `runs` more.
pub(super) fn within(left: usize, runs: usize) -> bool {
    runs <= left
}

/// What a stretch of a line is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Token {
    /// Anything else: drawn as it is today.
    #[default]
    Plain,
    /// A keyword, or a literal word such as `true`.
    Keyword,
    /// A quoted string, its prefix and quotes included.
    Str,
    /// A number, its suffix included.
    Number,
    /// A comment, to its end.
    Comment,
    /// A key of a config file: TOML, YAML, JSON.
    Key,
}

/// The languages the mapper knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    Python,
    /// sh, bash, zsh and fish share one table.
    Shell,
    Toml,
    Json,
    Yaml,
    /// JavaScript and TypeScript share one table.
    Script,
}

/// One run: its token and its byte range in the scanned line.
pub type Run = (Token, Range<usize>);

/// The language a file extension names (case-insensitive), if any. An extension
/// alone decides: a name with a leading dot only is no extension.
#[must_use]
pub fn lang_of_path(path: &str) -> Option<Lang> {
    let ext = std::path::Path::new(path).extension()?.to_str()?;
    lang_named(ext, false)
}

/// The language a fence's info string names (its first word, case-insensitive),
/// if any.
#[must_use]
pub fn lang_of_tag(info: &str) -> Option<Lang> {
    let word = info
        .split(|ch: char| ch.is_whitespace() || ch == ',' || ch == '{')
        .find(|word| !word.is_empty())?;
    lang_named(word, true)
}

fn lang_named(name: &str, tag: bool) -> Option<Lang> {
    let name = name.to_ascii_lowercase();
    Some(match name.as_str() {
        "rs" => Lang::Rust,
        "py" => Lang::Python,
        "sh" | "bash" | "zsh" | "fish" => Lang::Shell,
        "toml" => Lang::Toml,
        "json" => Lang::Json,
        "yaml" | "yml" => Lang::Yaml,
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "mts" | "cts" | "tsx" => Lang::Script,
        "rust" if tag => Lang::Rust,
        "python" if tag => Lang::Python,
        "shell" if tag => Lang::Shell,
        "javascript" | "typescript" if tag => Lang::Script,
        _ => return None,
    })
}

/// What a scanner carries from one line to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Code,
    /// Inside `/* ... */`.
    Block,
    /// Inside a triple-quoted string; the quote byte.
    Triple(u8),
    /// Inside a backtick string.
    Template,
}

/// One language's reading of consecutive lines.
#[derive(Debug, Clone)]
pub struct Scanner {
    lang: Lang,
    state: State,
}

impl Scanner {
    /// A scanner at the start of a text.
    #[must_use]
    pub fn new(lang: Lang) -> Self {
        Self {
            lang,
            state: State::Code,
        }
    }

    /// The runs of `text`, the next line.
    #[must_use]
    pub fn line(&mut self, text: &str) -> Vec<Run> {
        let mut runs = Vec::new();
        self.scan(text, &mut runs);
        runs
    }

    /// [`Self::line`] into `runs`, which it appends to. Only the first
    /// [`LINE_CHARS`] scalars are read: the rest is one plain run, still drawn.
    pub fn scan(&mut self, text: &str, runs: &mut Vec<Run>) {
        let first = runs.len();
        let end = text
            .char_indices()
            .nth(LINE_CHARS)
            .map_or(text.len(), |(at, _)| at);
        let head = &text[..end];
        let fresh = self.state == State::Code;
        let mut at = self.resume(head, runs);
        if fresh && at == 0 {
            at = self.start_key(head, runs).unwrap_or(0);
        }
        while at < head.len() {
            at = self.step(head, at, runs);
        }
        emit(runs, Token::Plain, end, text.len());
        bind_marks(text, runs, first);
    }

    /// Finish the construct the last line left open: where this line goes on.
    fn resume(&mut self, text: &str, runs: &mut Vec<Run>) -> usize {
        let bytes = text.as_bytes();
        let (found, token) = match self.state {
            State::Code => return 0,
            State::Block => (find_block_end(bytes, 0), Token::Comment),
            State::Triple(quote) => (triple_end(bytes, 0, quote), Token::Str),
            State::Template => (string_end(bytes, 0, b'`', true), Token::Str),
        };
        if let Some(end) = found {
            emit(runs, token, 0, end);
            self.state = State::Code;
            end
        } else {
            emit(runs, token, 0, bytes.len());
            bytes.len()
        }
    }

    /// One token at `at`; where the next begins. Always moves on.
    fn step(&mut self, text: &str, at: usize, runs: &mut Vec<Run>) -> usize {
        self.comment(text, at, runs)
            .or_else(|| self.quoted(text, at, runs))
            .or_else(|| self.word(text, at, runs))
            .or_else(|| number(text, at, runs))
            .unwrap_or_else(|| {
                emit(runs, Token::Plain, at, at + 1);
                at + 1
            })
    }

    fn comment(&mut self, text: &str, at: usize, runs: &mut Vec<Run>) -> Option<usize> {
        let bytes = text.as_bytes();
        if self.line_comment_at(bytes, at) {
            emit(runs, Token::Comment, at, bytes.len());
            return Some(bytes.len());
        }
        let block = matches!(self.lang, Lang::Rust | Lang::Script);
        if !(block && bytes[at..].starts_with(b"/*")) {
            return None;
        }
        let end = find_block_end(bytes, at + 2).unwrap_or_else(|| {
            self.state = State::Block;
            bytes.len()
        });
        emit(runs, Token::Comment, at, end);
        Some(end)
    }

    fn line_comment_at(&self, bytes: &[u8], at: usize) -> bool {
        let before = at.checked_sub(1).map_or(b' ', |back| bytes[back]);
        match self.lang {
            Lang::Rust | Lang::Script => bytes[at..].starts_with(b"//"),
            Lang::Python | Lang::Toml => bytes[at] == b'#',
            Lang::Shell => {
                bytes[at] == b'#'
                    && (before.is_ascii_whitespace() || matches!(before, b';' | b'|' | b'&' | b'('))
            }
            Lang::Yaml => bytes[at] == b'#' && before.is_ascii_whitespace(),
            Lang::Json => false,
        }
    }

    /// A string, or a Rust character, starting at `at`.
    fn quoted(&mut self, text: &str, at: usize, runs: &mut Vec<Run>) -> Option<usize> {
        let bytes = text.as_bytes();
        match bytes[at] {
            b'"' => Some(self.string(text, at, at, runs)),
            b'\'' if self.lang == Lang::Rust => Some(rust_char(bytes, at, runs)),
            b'\'' if self.lang != Lang::Json => Some(self.string(text, at, at, runs)),
            b'`' if self.lang == Lang::Script => Some(self.string(text, at, at, runs)),
            _ => None,
        }
    }

    /// The string whose text begins at `start` and whose opening quote is at
    /// `quote_at`; where it ends, or the line's end when it does not.
    fn string(&mut self, text: &str, start: usize, quote_at: usize, runs: &mut Vec<Run>) -> usize {
        let bytes = text.as_bytes();
        let quote = bytes[quote_at];
        let triple_lang = matches!(self.lang, Lang::Python | Lang::Toml);
        if triple_lang && quote != b'`' && bytes[quote_at..].starts_with(&[quote; 3]) {
            let end = triple_end(bytes, quote_at + 3, quote).unwrap_or_else(|| {
                self.state = State::Triple(quote);
                bytes.len()
            });
            emit(runs, Token::Str, start, end);
            return end;
        }
        let escapes = quote != b'\'' || matches!(self.lang, Lang::Python | Lang::Script);
        let closed = string_end(bytes, quote_at + 1, quote, escapes);
        if closed.is_none() && quote == b'`' {
            self.state = State::Template;
        }
        let end = closed.unwrap_or(bytes.len());
        let key = self.lang == Lang::Json && closed.is_some() && colon_follows(bytes, end);
        emit(runs, if key { Token::Key } else { Token::Str }, start, end);
        end
    }

    /// An identifier at `at`: a keyword, a prefixed string or plain.
    fn word(&mut self, text: &str, at: usize, runs: &mut Vec<Run>) -> Option<usize> {
        let bytes = text.as_bytes();
        if !is_ident(bytes[at]) || bytes[at].is_ascii_digit() {
            return None;
        }
        let end = at
            + bytes[at..]
                .iter()
                .position(|byte| !is_ident(*byte))
                .unwrap_or(bytes.len() - at);
        let word = &text[at..end];
        if let Some(next) = self.prefixed(text, word, at, end, runs) {
            return Some(next);
        }
        let before = at.checked_sub(1).map_or(b' ', |back| bytes[back]);
        let lifetime = self.lang == Lang::Rust && before == b'\'';
        let token = if !lifetime && !matches!(before, b'.' | b'$') && is_keyword(self.lang, word) {
            Token::Keyword
        } else {
            Token::Plain
        };
        emit(runs, token, at, end);
        Some(end)
    }

    /// `r"x"`, `b"x"`, `f"x"` and kin: the prefix belongs to the string.
    fn prefixed(
        &mut self,
        text: &str,
        word: &str,
        at: usize,
        end: usize,
        runs: &mut Vec<Run>,
    ) -> Option<usize> {
        let bytes = text.as_bytes();
        let next = bytes.get(end).copied()?;
        match self.lang {
            Lang::Python
                if word.len() <= 2
                    && word.bytes().all(|byte| b"rRbBuUfF".contains(&byte))
                    && matches!(next, b'"' | b'\'') =>
            {
                Some(self.string(text, at, end, runs))
            }
            Lang::Rust if word == "b" && next == b'"' => Some(self.string(text, at, end, runs)),
            Lang::Rust if matches!(word, "r" | "br") && matches!(next, b'"' | b'#') => {
                raw_string(bytes, at, end, runs)
            }
            _ => None,
        }
    }

    /// A TOML or YAML key at the start of a line, and a TOML table header:
    /// the runs up to the end of the key; where the line goes on from.
    fn start_key(&self, text: &str, runs: &mut Vec<Run>) -> Option<usize> {
        let bytes = text.as_bytes();
        let from = skip_blanks(bytes, 0);
        let (key, end) = match self.lang {
            Lang::Toml => toml_key(bytes, from)?,
            Lang::Yaml => yaml_key(bytes, from)?,
            _ => return None,
        };
        emit(runs, Token::Plain, 0, key.start);
        emit(runs, Token::Key, key.start, key.end);
        Some(end)
    }
}

/// Add `token` over `from..to`, joining the run before it when it is the same
/// token and adjacent. An empty range adds nothing.
fn emit(runs: &mut Vec<Run>, token: Token, from: usize, to: usize) {
    if from >= to {
        return;
    }
    if let Some((last, range)) = runs.last_mut()
        && *last == token
        && range.end == from
    {
        range.end = to;
        return;
    }
    runs.push((token, from..to));
}

/// Move every run start that falls inside a grapheme to the grapheme's end,
/// from `runs[from..]` on: the scalars of the cluster belong to the run before
/// it. The text of a line is drawn a span at a time, and a span that began with
/// a combining or spacing mark would not draw it with the scalar before it as
/// the plain draw does (a zero-width grapheme standing alone takes no cell, a
/// spacing mark takes a cell of its own). The boundaries are the drawing
/// library's own (`Span::styled_graphemes`), so a cut is never made where it
/// would not cut.
fn bind_marks(text: &str, runs: &mut Vec<Run>, from: usize) {
    if text.is_ascii() {
        return;
    }
    let base = text.as_ptr().addr();
    let starts: Vec<usize> = Span::raw(text)
        .styled_graphemes(Style::default())
        .map(|grapheme| grapheme.symbol.as_ptr().addr() - base)
        .collect();
    let mut at = from + 1;
    while at < runs.len() {
        let range = runs[at].1.clone();
        if starts.binary_search(&range.start).is_ok() {
            at += 1;
            continue;
        }
        let next = starts
            .get(starts.partition_point(|start| *start < range.start))
            .copied()
            .unwrap_or(text.len());
        let carried = next.min(range.end) - range.start;
        runs[at - 1].1.end += carried;
        if carried < range.len() {
            runs[at].1.start += carried;
            at += 1;
        } else if runs
            .get(at + 1)
            .is_some_and(|following| following.0 == runs[at - 1].0)
        {
            runs[at - 1].1.end = runs[at + 1].1.end;
            runs.drain(at..=at + 1);
        } else {
            runs.remove(at);
        }
    }
}

/// A byte an identifier or a number is made of. Every byte of a non-ASCII
/// scalar counts, so a run never ends inside one.
fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

/// A number at `at`, when no identifier runs into it: its digits, letters of
/// suffix and `_`, and a `.` between digits.
fn number(text: &str, at: usize, runs: &mut Vec<Run>) -> Option<usize> {
    let bytes = text.as_bytes();
    if !bytes[at].is_ascii_digit() || at.checked_sub(1).is_some_and(|back| is_ident(bytes[back])) {
        return None;
    }
    let mut end = at + 1;
    while end < bytes.len() {
        let dot_digit = bytes[end] == b'.' && bytes.get(end + 1).is_some_and(u8::is_ascii_digit);
        if !(is_ident(bytes[end]) || dot_digit) {
            break;
        }
        end += 1;
    }
    emit(runs, Token::Number, at, end);
    Some(end)
}

/// Where the first unescaped `quote` after `from` closes: the index after it.
fn string_end(bytes: &[u8], from: usize, quote: u8, escapes: bool) -> Option<usize> {
    let mut at = from;
    while at < bytes.len() {
        if escapes && bytes[at] == b'\\' {
            at += 2;
        } else if bytes[at] == quote {
            return Some(at + 1);
        } else {
            at += 1;
        }
    }
    None
}

/// Where the triple quote `quote` closes after `from`: the index after it.
fn triple_end(bytes: &[u8], from: usize, quote: u8) -> Option<usize> {
    let mut at = from;
    while at < bytes.len() {
        if bytes[at] == b'\\' {
            at += 2;
        } else if bytes[at..].starts_with(&[quote; 3]) {
            return Some(at + 3);
        } else {
            at += 1;
        }
    }
    None
}

/// Where a block comment closes after `from`: the index after its `*/`.
fn find_block_end(bytes: &[u8], from: usize) -> Option<usize> {
    let rest = bytes.get(from..)?;
    rest.windows(2)
        .position(|pair| pair == b"*/")
        .map(|found| from + found + 2)
}

/// Whether a `:` follows `at` after blanks: what makes a JSON string a key.
fn colon_follows(bytes: &[u8], at: usize) -> bool {
    bytes.get(skip_blanks(bytes, at)) == Some(&b':')
}

fn skip_blanks(bytes: &[u8], from: usize) -> usize {
    let rest = bytes.get(from..).unwrap_or_default();
    from + rest
        .iter()
        .position(|byte| *byte != b' ' && *byte != b'\t')
        .unwrap_or(rest.len())
}

/// A Rust `'` at `at`: a character literal (`'a'`, `'\n'`, `'é'`) drawn as a
/// string, or a lifetime or label, whose `'` is plain. The index after the
/// consumed bytes.
fn rust_char(bytes: &[u8], at: usize, runs: &mut Vec<Run>) -> usize {
    let after = at + 1;
    let literal = match bytes.get(after) {
        Some(b'\\') => bytes
            .get(after + 2..(after + 12).min(bytes.len())) // past the escaped char
            .and_then(|rest| rest.iter().position(|byte| *byte == b'\''))
            .map(|found| after + 2 + found + 1),
        Some(first) if *first != b'\'' => {
            let width = match first {
                0xF0..=0xFF => 4,
                0xE0..=0xEF => 3,
                0xC0..=0xDF => 2,
                _ => 1,
            };
            (bytes.get(after + width) == Some(&b'\'')).then_some(after + width + 1)
        }
        _ => None,
    };
    if let Some(end) = literal {
        emit(runs, Token::Str, at, end);
        end
    } else {
        emit(runs, Token::Plain, at, after);
        after
    }
}

/// A Rust raw string whose prefix is `at..end` (`r` or `br`): `r"..."` and
/// `r#"..."#`, on this line. `None` when no `"` follows the hashes, which
/// leaves the prefix an identifier.
fn raw_string(bytes: &[u8], at: usize, end: usize, runs: &mut Vec<Run>) -> Option<usize> {
    let hashes = bytes[end..]
        .iter()
        .take_while(|byte| **byte == b'#')
        .count();
    let open = end + hashes;
    if bytes.get(open) != Some(&b'"') {
        return None;
    }
    let mut closing = vec![b'"'];
    closing.resize(hashes + 1, b'#');
    let close = bytes
        .get(open + 1..)
        .and_then(|rest| rest.windows(closing.len()).position(|win| win == closing))
        .map_or(bytes.len(), |found| open + 1 + found + closing.len());
    emit(runs, Token::Str, at, close);
    Some(close)
}

/// A TOML key at `from`, bare or quoted, that an `=` follows, or the inside of
/// a `[table]` header. The key's range and where the line goes on from.
fn toml_key(bytes: &[u8], from: usize) -> Option<(Range<usize>, usize)> {
    if bytes.get(from) == Some(&b'[') {
        let open = from + 1 + usize::from(bytes.get(from + 1) == Some(&b'['));
        let close = open + bytes.get(open..)?.iter().position(|byte| *byte == b']')?;
        return (close > open).then_some((open..close, close));
    }
    let first = *bytes.get(from)?;
    let end = if matches!(first, b'"' | b'\'') {
        string_end(bytes, from + 1, first, first == b'"')?
    } else {
        from + bytes[from..]
            .iter()
            .take_while(|byte| is_ident(**byte) || matches!(**byte, b'-' | b'.'))
            .count()
    };
    (end > from && bytes.get(skip_blanks(bytes, end)) == Some(&b'=')).then_some((from..end, end))
}

/// A YAML mapping key at `from` (after any `- ` markers), bare or quoted,
/// that a `:` ends: the key's range and where the line goes on from.
fn yaml_key(bytes: &[u8], from: usize) -> Option<(Range<usize>, usize)> {
    let colon =
        |at: usize| bytes.get(at) == Some(&b':') && matches!(bytes.get(at + 1), None | Some(b' '));
    let mut at = from;
    while bytes.get(at) == Some(&b'-') && matches!(bytes.get(at + 1), None | Some(b' ')) {
        at = skip_blanks(bytes, at + 1);
    }
    let first = *bytes.get(at)?;
    if b"#{[|>&*!%@`?".contains(&first) {
        return None;
    }
    let end = if matches!(first, b'"' | b'\'') {
        let end = string_end(bytes, at + 1, first, first == b'"')?;
        colon(skip_blanks(bytes, end)).then_some(end)?
    } else {
        let mut scan = at;
        loop {
            let byte = *bytes.get(scan)?;
            if byte == b'#' && scan > at && bytes[scan - 1] == b' ' {
                return None;
            }
            if colon(scan) {
                break;
            }
            scan += 1;
        }
        let key = &bytes[at..scan];
        at + key.len() - key.iter().rev().take_while(|byte| **byte == b' ').count()
    };
    (end > at).then_some((at..end, end))
}

/// Rust's keywords, with `true` and `false`.
const RUST: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while",
];
/// Python's keywords and its three constants.
const PYTHON: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];
/// The control words of sh, bash, zsh and fish together.
const SHELL: &[&str] = &[
    "if", "then", "elif", "else", "fi", "for", "while", "until", "do", "done", "case", "esac",
    "in", "function", "select", "return", "break", "continue", "end", "begin", "switch",
];
/// The literals of TOML and JSON.
const LITERALS: &[&str] = &["true", "false", "null"];
/// The literals of YAML, in the spellings it accepts.
const YAML: &[&str] = &[
    "true", "false", "null", "True", "False", "Null", "TRUE", "FALSE", "NULL",
];
/// The keywords of JavaScript and TypeScript together.
const SCRIPT: &[&str] = &[
    "abstract",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "declare",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "namespace",
    "new",
    "null",
    "of",
    "private",
    "protected",
    "public",
    "readonly",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

/// Whether `word` is a keyword of `lang`: one short table each.
fn is_keyword(lang: Lang, word: &str) -> bool {
    let table = match lang {
        Lang::Rust => RUST,
        Lang::Python => PYTHON,
        Lang::Shell => SHELL,
        Lang::Toml | Lang::Json => LITERALS,
        Lang::Yaml => YAML,
        Lang::Script => SCRIPT,
    };
    table.contains(&word)
}

/// The ground a token is drawn on: the reader's `ink` for a file, the `panel`
/// of a fence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ground {
    Ink,
    Panel,
}

/// The style a token wears on `ground`, none under an undrawn look.
///
/// The roles are the ones the 4.5:1 text bar pins on that ground in every
/// palette (`theme.rs` tests): on `ink` `waiting_agent`, `working`, `title` and
/// `text`; on `panel` `waiting_agent`, `title` and `text`. A comment and a key,
/// which have no such hue left, wear the text colour with a modifier.
#[must_use]
pub(super) fn style(token: Token, paint: Paint, ground: Ground) -> Style {
    if !paint.drawn() {
        return Style::new();
    }
    match (token, ground) {
        (Token::Plain, _) => paint.fg(|palette| palette.text),
        (Token::Keyword, _) => paint
            .fg(|palette| palette.waiting_agent)
            .add_modifier(Modifier::BOLD),
        (Token::Str, Ground::Ink) => paint.fg(|palette| palette.working),
        (Token::Str, Ground::Panel) | (Token::Number, Ground::Ink) => {
            paint.fg(|palette| palette.title)
        }
        (Token::Number, Ground::Panel) => paint
            .fg(|palette| palette.title)
            .add_modifier(Modifier::BOLD),
        (Token::Comment, _) => paint
            .fg(|palette| palette.text)
            .add_modifier(Modifier::ITALIC),
        (Token::Key, _) => paint
            .fg(|palette| palette.text)
            .add_modifier(Modifier::BOLD),
    }
}

/// `base`, a fence's code style, worn over by what `token` is: nothing for a
/// plain run.
#[must_use]
pub(super) fn fenced(base: Style, token: Token, paint: Paint) -> Style {
    if token == Token::Plain {
        return base;
    }
    base.patch(style(token, paint, Ground::Panel))
}

/// The raw reader's colouring of the rows it draws: the scanner is read up to
/// the first drawn line (at most [`LOOKBACK_LINES`] lines and
/// [`LOOKBACK_BYTES`] bytes back) and then follows the rows down.
pub struct Reading<'a> {
    lines: &'a [String],
    flow: &'a Flow,
    paint: Paint,
    scanner: Scanner,
    /// The next line the scanner has not read.
    next: usize,
    /// The runs the window may still colour; `None` once they are spent.
    left: Option<usize>,
    /// The newest line read: its index, its tab-flat text and its runs.
    held: Option<(usize, String, Vec<Run>)>,
}

impl<'a> Reading<'a> {
    /// The colouring of `view`'s file when its path names a language, its body
    /// is text and the look is drawn; `top` is the first drawn display row.
    #[must_use]
    pub(super) fn open(view: &View<'a>, flow: &'a Flow, top: usize, paint: Paint) -> Option<Self> {
        if !paint.drawn() {
            return None;
        }
        let lang = lang_of_path(view.path)?;
        let Some(Body::Text { lines, .. }) = view.body else {
            return None;
        };
        let (first, ..) = flow.display(top)?;
        let (mut from, mut bytes) = (first.min(lines.len()), 0);
        while from > 0 && first - from < LOOKBACK_LINES {
            bytes += lines.get(from - 1).map_or(0, String::len);
            if bytes > LOOKBACK_BYTES {
                break;
            }
            from -= 1;
        }
        let mut scanner = Scanner::new(lang);
        let mut sink = Vec::new();
        let mut left = Some(MAX_RUNS);
        for line in lines.get(from..first).unwrap_or_default() {
            sink.clear();
            scanner.scan(line, &mut sink);
            left = left.and_then(|left| left.checked_sub(sink.len()));
        }
        Some(Self {
            lines,
            flow,
            paint,
            scanner,
            next: first,
            left,
            held: None,
        })
    }

    /// Display row `at` as coloured spans (its hang first), or `None` when the
    /// row is to be drawn as it is: a row it cannot place in its line.
    #[must_use]
    pub fn row(&mut self, at: usize) -> Option<Line<'static>> {
        let (line, _, gap, _) = self.flow.display(at)?;
        if !self.load(line) {
            return None;
        }
        let (_, flat, runs) = self.held.as_ref()?;
        let span = self.flow.span_of(at, flat)?;
        let plain = style(Token::Plain, self.paint, Ground::Ink);
        let mut spans = Vec::new();
        if gap > 0 {
            spans.push(Span::styled(" ".repeat(gap), plain));
        }
        for (token, range) in runs {
            if range.start >= span.end {
                break;
            }
            let (from, to) = (range.start.max(span.start), range.end.min(span.end));
            if from < to {
                let wear = style(*token, self.paint, Ground::Ink);
                spans.push(Span::styled(flat[from..to].to_owned(), wear));
            }
        }
        Some(Line::from(spans))
    }

    /// Read up to line `line`, holding its flat text and runs.
    fn load(&mut self, line: usize) -> bool {
        if self.held.as_ref().is_some_and(|held| held.0 == line) {
            return true;
        }
        if line < self.next {
            return false;
        }
        while self.next <= line {
            let Some(text) = self.lines.get(self.next) else {
                return false;
            };
            let flat = expand_tabs(&terminal_text(text), reader::WRAP_COLUMN);
            let mut runs = Vec::new();
            self.scanner.scan(&flat, &mut runs);
            self.left = self.left.and_then(|left| left.checked_sub(runs.len()));
            if self.next == line && self.left.is_some() {
                self.held = Some((line, flat, runs));
            }
            self.next += 1;
        }
        self.left.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::draw::Paint;
    use crate::app::markdown;
    use crate::theme::{Look, Palette};
    use ratatui_core::buffer::Buffer;
    use ratatui_core::layout::Rect;
    use std::fmt::Write as _;

    /// `text`'s runs, each with the text it covers, plain runs left out.
    fn marked(lang: Lang, text: &str) -> Vec<(Token, String)> {
        Scanner::new(lang)
            .line(text)
            .into_iter()
            .filter(|(token, _)| *token != Token::Plain)
            .map(|(token, range)| (token, text[range].to_owned()))
            .collect()
    }

    fn tile(runs: &[Run], text: &str) {
        let mut at = 0;
        for (_, range) in runs {
            assert_eq!(range.start, at, "{text:?}: a gap or overlap at {at}");
            assert!(range.end > range.start, "{text:?}: an empty run");
            assert!(
                text.is_char_boundary(range.end),
                "{text:?}: cut inside a scalar"
            );
            at = range.end;
        }
        assert_eq!(at, text.len(), "{text:?}: runs stop short");
    }

    #[test]
    fn languages_are_named_by_extension_and_by_tag() {
        for (path, lang) in [
            ("a.rs", Lang::Rust),
            ("/x/a.RS", Lang::Rust),
            ("a.py", Lang::Python),
            ("a.sh", Lang::Shell),
            ("a.bash", Lang::Shell),
            ("a.zsh", Lang::Shell),
            ("a.fish", Lang::Shell),
            ("a.toml", Lang::Toml),
            ("a.json", Lang::Json),
            ("a.yaml", Lang::Yaml),
            ("a.YML", Lang::Yaml),
            ("a.js", Lang::Script),
            ("a.mjs", Lang::Script),
            ("a.tsx", Lang::Script),
        ] {
            assert_eq!(lang_of_path(path), Some(lang), "{path}");
        }
        for path in ["a", ".rs", "a.rust", "a.txt", "a.rs.bak", "a.md", ""] {
            assert_eq!(lang_of_path(path), None, "{path:?}");
        }
        for (tag, lang) in [
            ("rust", Lang::Rust),
            ("Rs extra-info", Lang::Rust),
            ("python,linenos", Lang::Python),
            ("shell", Lang::Shell),
            ("TOML", Lang::Toml),
            ("javascript", Lang::Script),
            ("typescript", Lang::Script),
            ("yml", Lang::Yaml),
        ] {
            assert_eq!(lang_of_tag(tag), Some(lang), "{tag}");
        }
        for tag in ["", " ", "text", "rustc", "{rust}", "diff"] {
            assert_eq!(lang_of_tag(tag), None, "{tag:?}");
        }
    }

    #[test]
    fn rust_tokens() {
        let line =
            "pub fn f<'a>(x: &'a str) -> u32 { let c = 'x'; let n = 0x2A_u8; \"s\\\"t\" } // end";
        let got = marked(Lang::Rust, line);
        let words = |token| -> Vec<&str> {
            got.iter()
                .filter(|(kind, _)| *kind == token)
                .map(|(_, text)| text.as_str())
                .collect()
        };
        assert_eq!(words(Token::Keyword), ["pub", "fn", "let", "let"]);
        assert_eq!(words(Token::Str), ["'x'", "\"s\\\"t\""]);
        assert_eq!(words(Token::Number), ["0x2A_u8"]);
        assert_eq!(words(Token::Comment), ["// end"]);
        // A lifetime is no string and `static` after `'` no keyword.
        assert!(
            marked(Lang::Rust, "fn f(x: &'static str)")
                .iter()
                .all(|(kind, text)| {
                    *kind != Token::Str && (*kind != Token::Keyword || text == "fn")
                })
        );
        assert_eq!(
            marked(Lang::Rust, "let r = r#\"a\"b\"#; let s = b\"x\";"),
            [
                (Token::Keyword, "let".to_owned()),
                (Token::Str, "r#\"a\"b\"#".to_owned()),
                (Token::Keyword, "let".to_owned()),
                (Token::Str, "b\"x\"".to_owned()),
            ]
        );
        assert_eq!(
            marked(Lang::Rust, "x.type + y.fn"),
            [],
            "a keyword after a dot is a field"
        );
    }

    #[test]
    fn a_block_comment_does_not_close_on_the_star_of_its_own_opener() {
        for line in ["/*/ x */ fn", " /*/ x */ fn", "a /*/ x */ fn"] {
            let got = marked(Lang::Rust, line);
            let open = line.find("/*/").unwrap_or(0);
            let close = line.find("x */").map_or(0, |at| at + 4);
            assert_eq!(
                got,
                [
                    (Token::Comment, line[open..close].to_owned()),
                    (Token::Keyword, "fn".to_owned())
                ],
                "{line:?}"
            );
        }
    }

    #[test]
    fn python_tokens() {
        let got = marked(Lang::Python, "def f(): return f\"a{b}\" + 3.5  # n");
        assert_eq!(
            got,
            [
                (Token::Keyword, "def".to_owned()),
                (Token::Keyword, "return".to_owned()),
                (Token::Str, "f\"a{b}\"".to_owned()),
                (Token::Number, "3.5".to_owned()),
                (Token::Comment, "# n".to_owned()),
            ]
        );
        // `#` inside a string is no comment, a word that merely ends in a
        // prefix letter is no string prefix.
        assert_eq!(
            marked(Lang::Python, "x = 'a # b' ; buf'x'"),
            [
                (Token::Str, "'a # b'".to_owned()),
                (Token::Str, "'x'".to_owned())
            ]
        );
    }

    #[test]
    fn shell_tokens() {
        let got = marked(
            Lang::Shell,
            "for f in *; do echo \"$f\" 'x' $# 42; done # c",
        );
        let kinds: Vec<_> = got
            .iter()
            .map(|(kind, text)| (*kind, text.as_str()))
            .collect();
        assert_eq!(
            kinds,
            [
                (Token::Keyword, "for"),
                (Token::Keyword, "in"),
                (Token::Keyword, "do"),
                (Token::Str, "\"$f\""),
                (Token::Str, "'x'"),
                (Token::Number, "42"),
                (Token::Keyword, "done"),
                (Token::Comment, "# c"),
            ]
        );
        assert_eq!(
            marked(Lang::Shell, "echo $if a#b"),
            [],
            "no keyword after $, no # inside a word"
        );
    }

    #[test]
    fn toml_tokens_and_multiline_strings() {
        let mut scanner = Scanner::new(Lang::Toml);
        let lines = [
            "[tool.\"x\"]",
            "name = \"\"\"a",
            "# in",
            "b\"\"\" # out",
            "n = 1_000",
        ];
        let mut seen = Vec::new();
        for line in lines {
            let runs = scanner.line(line);
            tile(&runs, line);
            seen.push(
                runs.iter()
                    .filter(|(token, _)| *token != Token::Plain)
                    .map(|(token, range)| (*token, line[range.clone()].to_owned()))
                    .collect::<Vec<_>>(),
            );
        }
        assert_eq!(seen[0], [(Token::Key, "tool.\"x\"".to_owned())]);
        assert_eq!(
            seen[1],
            [
                (Token::Key, "name".to_owned()),
                (Token::Str, "\"\"\"a".to_owned())
            ]
        );
        assert_eq!(
            seen[2],
            [(Token::Str, "# in".to_owned())],
            "inside a triple string"
        );
        assert_eq!(
            seen[3],
            [
                (Token::Str, "b\"\"\"".to_owned()),
                (Token::Comment, "# out".to_owned())
            ]
        );
        assert_eq!(
            seen[4],
            [
                (Token::Key, "n".to_owned()),
                (Token::Number, "1_000".to_owned())
            ]
        );
    }

    #[test]
    fn json_keys_are_not_strings() {
        assert_eq!(
            marked(Lang::Json, "{ \"a\" : [1, -2.5e3, null], \"b\": \"c\" }"),
            [
                (Token::Key, "\"a\"".to_owned()),
                (Token::Number, "1".to_owned()),
                (Token::Number, "2.5e3".to_owned()),
                (Token::Keyword, "null".to_owned()),
                (Token::Key, "\"b\"".to_owned()),
                (Token::Str, "\"c\"".to_owned()),
            ]
        );
    }

    #[test]
    fn yaml_keys_comments_and_lists() {
        assert_eq!(
            marked(Lang::Yaml, "- name: a b # c"),
            [
                (Token::Key, "name".to_owned()),
                (Token::Comment, "# c".to_owned())
            ]
        );
        assert_eq!(
            marked(Lang::Yaml, "url: http://x.io:80/p"),
            [
                (Token::Key, "url".to_owned()),
                (Token::Number, "80".to_owned())
            ]
        );
        assert_eq!(marked(Lang::Yaml, "---"), []);
        assert_eq!(
            marked(Lang::Yaml, "plain text # not a key"),
            [(Token::Comment, "# not a key".to_owned())]
        );
        assert_eq!(
            marked(Lang::Yaml, "\"q k\": true"),
            [
                (Token::Key, "\"q k\"".to_owned()),
                (Token::Keyword, "true".to_owned())
            ]
        );
    }

    #[test]
    fn script_template_and_block_comments_span_lines() {
        let mut scanner = Scanner::new(Lang::Script);
        let first = scanner.line("const s = `a ${b}");
        let second = scanner.line("still` + /* open");
        let third = scanner.line("still comment */ let x = 1");
        let text = |runs: &[Run], line: &str, token| {
            runs.iter()
                .filter(|(kind, _)| *kind == token)
                .map(|(_, range)| line[range.clone()].to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(text(&first, "const s = `a ${b}", Token::Str), ["`a ${b}"]);
        assert_eq!(text(&second, "still` + /* open", Token::Str), ["still`"]);
        assert_eq!(
            text(&second, "still` + /* open", Token::Comment),
            ["/* open"]
        );
        assert_eq!(
            text(&third, "still comment */ let x = 1", Token::Comment),
            ["still comment */"]
        );
        assert_eq!(
            text(&third, "still comment */ let x = 1", Token::Keyword),
            ["let"]
        );
        assert_eq!(
            text(&third, "still comment */ let x = 1", Token::Number),
            ["1"]
        );
    }

    #[test]
    fn python_triple_string_carries_over_and_ends() {
        let mut scanner = Scanner::new(Lang::Python);
        let lines = ["x = '''a", "def not_code(): pass", "'''; y = 1"];
        let kinds: Vec<Vec<Token>> = lines
            .iter()
            .map(|line| {
                scanner
                    .line(line)
                    .into_iter()
                    .map(|(token, _)| token)
                    .collect()
            })
            .collect();
        assert!(kinds[1] == [Token::Str], "{kinds:?}");
        assert_eq!(kinds[2].first(), Some(&Token::Str));
        assert!(kinds[2].contains(&Token::Number));
    }

    #[test]
    fn runs_tile_every_line_whatever_it_holds() {
        let langs = [
            Lang::Rust,
            Lang::Python,
            Lang::Shell,
            Lang::Toml,
            Lang::Json,
            Lang::Yaml,
            Lang::Script,
        ];
        let long = format!(
            "let s = \"{}\"; // {}",
            "é界".repeat(3000),
            "x".repeat(5000)
        );
        let lines = [
            "",
            "   ",
            "\u{FFFD}]52;c;x\u{FFFD}\u{FFFD}31m \"\u{FFFD}",
            "\"unterminated",
            "'",
            "'''",
            "\"\"\"",
            "/*",
            "`",
            "#",
            "//",
            "r#\"x",
            "'\\",
            "- - a: b: c: \"d\"",
            "[[",
            "[a",
            "0x",
            "1..2",
            "ünï = 1",
            "\"x\"\u{0301};",
            "\"x\"\u{0301}\u{0301}\u{200D}",
            "/* c */\u{0301}\u{0308} 1",
            "\u{0301}\"x\"",
            "\"x\"\u{0903}\u{093E};",
            "/* c */\u{093E} fn",
            long.as_str(),
        ];
        for lang in langs {
            let mut scanner = Scanner::new(lang);
            for line in lines {
                let runs = scanner.line(line);
                tile(&runs, line);
                let base = line.as_ptr().addr();
                let starts: Vec<usize> = Span::raw(line)
                    .styled_graphemes(Style::default())
                    .map(|grapheme| grapheme.symbol.as_ptr().addr() - base)
                    .collect();
                for (_, range) in runs.iter().skip(1) {
                    assert!(
                        line.is_ascii() || starts.contains(&range.start),
                        "{lang:?} {line:?}: a run starts inside a grapheme, at {}",
                        range.start
                    );
                }
            }
        }
    }

    #[test]
    fn a_long_line_is_read_only_to_the_line_cap_and_stays_whole() {
        let line = format!("let s = \"{}\"; TAIL", "a".repeat(LINE_CHARS * 2));
        let runs = Scanner::new(Lang::Rust).line(&line);
        tile(&runs, &line);
        let (token, range) = runs.last().cloned().unwrap_or((Token::Keyword, 0..0));
        assert_eq!(token, Token::Plain, "the tail beyond the cap is plain");
        assert_eq!(range.end, line.len());
        assert!(line[range].ends_with("TAIL"));
        assert_eq!(
            runs.iter()
                .filter(|(kind, _)| *kind == Token::Keyword)
                .count(),
            1
        );
    }

    #[test]
    fn a_state_still_open_at_the_cap_does_not_hide_the_tail() {
        let mut scanner = Scanner::new(Lang::Rust);
        let line = format!("/* {} */ fn", "x".repeat(LINE_CHARS));
        let runs = scanner.line(&line);
        tile(&runs, &line);
        assert!(runs.iter().all(|(token, _)| *token != Token::Keyword));
    }

    #[test]
    fn the_style_table_wears_only_roles_the_text_bar_pins() {
        for palette in [Palette::DARCULA, Palette::NEUTRAL, Palette::WARM] {
            let look = Look {
                palette,
                ..Look::DEFAULT
            };
            let paint = Paint::of(Some(&look));
            let fg = |token, ground| style(token, paint, ground).fg;
            let role = |hex| {
                let [red, green, blue] = crate::theme::rgb(hex);
                Some(ratatui_core::style::Color::Rgb(red, green, blue))
            };
            for ground in [Ground::Ink, Ground::Panel] {
                assert_eq!(fg(Token::Keyword, ground), role(palette.waiting_agent));
                assert_eq!(fg(Token::Comment, ground), role(palette.text));
                assert_eq!(fg(Token::Key, ground), role(palette.text));
            }
            assert_eq!(fg(Token::Str, Ground::Ink), role(palette.working));
            assert_eq!(fg(Token::Number, Ground::Ink), role(palette.title));
            assert_eq!(fg(Token::Str, Ground::Panel), role(palette.title));
            assert_eq!(fg(Token::Number, Ground::Panel), role(palette.title));
            assert_eq!(
                style(Token::Keyword, paint, Ground::Ink).add_modifier,
                Modifier::BOLD
            );
            assert_eq!(
                style(Token::Comment, paint, Ground::Panel).add_modifier,
                Modifier::ITALIC
            );
            assert_eq!(
                style(Token::Number, paint, Ground::Panel).add_modifier,
                Modifier::BOLD
            );
            assert_eq!(
                style(Token::Number, paint, Ground::Ink).add_modifier,
                Modifier::empty()
            );
        }
        let none = Paint::of(None);
        for token in [Token::Keyword, Token::Str, Token::Comment, Token::Key] {
            assert_eq!(style(token, none, Ground::Ink), Style::new());
            assert_eq!(fenced(Style::new(), token, none), Style::new());
        }
    }

    /// Every display row of `lines` at `width`, coloured: its text is what the
    /// plain draw puts there, and a row it cannot place comes back `None`.
    fn painted(path: &str, lines: &[&str], width: usize) -> Vec<Option<String>> {
        let body = Body::Text {
            lines: lines.iter().map(|line| (*line).to_owned()).collect(),
            size: 0,
        };
        let Body::Text { lines, .. } = &body else {
            return Vec::new();
        };
        let flow = reader::flow(lines, width);
        let view = View {
            path,
            line: None,
            body: Some(&body),
            flow: Some(&flow),
            top: 0,
            links: 0,
        };
        let look = Look::DEFAULT;
        let mut reading = Reading::open(&view, &flow, 0, Paint::of(Some(&look)));
        (0..flow.rows())
            .map(|at| {
                let line = reading.as_mut()?.row(at)?;
                let (_, _, gap, piece) = flow.display(at)?;
                let drawn: String = line
                    .spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect();
                assert_eq!(drawn, format!("{}{piece}", " ".repeat(gap)), "row {at}");
                Some(drawn)
            })
            .collect()
    }

    #[test]
    fn wrapped_rows_are_placed_in_their_line_through_tabs_and_trims() {
        let lines = [
            "\tfn  main()  {  let  s = \"aaa  bbb ccc ddd eee fff\";  }",
            "        let x = 1; // some   comment   with   spaces   inside   it",
            "",
            "    ",
            "é界x\t\"y\"\tz  /* c */",
        ];
        for width in 1..48 {
            let rows = painted("/f/a.rs", &lines, width);
            assert!(!rows.is_empty());
            assert!(rows.iter().all(Option::is_some), "width {width}: {rows:?}");
        }
    }

    #[test]
    fn a_path_without_a_language_or_a_body_is_not_read() {
        assert!(
            painted("/f/a.txt", &["let x = 1;"], 40)
                .iter()
                .all(Option::is_none)
        );
        let flow = reader::flow(&["x".to_owned()], 10);
        let gap = Body::Gap("gone".to_owned());
        let view = View {
            path: "/f/a.rs",
            line: None,
            body: Some(&gap),
            flow: Some(&flow),
            top: 0,
            links: 0,
        };
        assert!(Reading::open(&view, &flow, 0, Paint::of(Some(&Look::DEFAULT))).is_none());
        let body = Body::Text {
            lines: vec!["x".to_owned()],
            size: 0,
        };
        let view = View {
            body: Some(&body),
            ..view
        };
        assert!(Reading::open(&view, &flow, 0, Paint::of(None)).is_none());
        assert!(Reading::open(&view, &flow, 99, Paint::of(Some(&Look::DEFAULT))).is_none());
    }

    /// Whether the last of `lines`, read as Rust at width 80, wears a comment.
    fn last_is_comment(lines: &[String]) -> Option<bool> {
        let body = Body::Text {
            lines: lines.to_vec(),
            size: 0,
        };
        let flow = reader::flow(lines, 80);
        let view = View {
            path: "/f/a.rs",
            line: None,
            body: Some(&body),
            flow: Some(&flow),
            top: 0,
            links: 0,
        };
        let look = Look::DEFAULT;
        let at = flow.row_of(lines.len());
        let line = Reading::open(&view, &flow, at, Paint::of(Some(&look)))?.row(at)?;
        Some(
            line.spans
                .iter()
                .any(|span| span.style.add_modifier == Modifier::ITALIC),
        )
    }

    #[test]
    fn the_look_back_recovers_a_block_comment_opened_above_the_first_row() {
        let build = |fillers: usize| -> Vec<String> {
            let mut lines = vec!["/* open".to_owned()];
            lines.extend((0..fillers).map(|n| format!("text {n}")));
            lines.push("fn here() {}".to_owned());
            lines
        };
        assert_eq!(
            last_is_comment(&build(LOOKBACK_LINES - 1)),
            Some(true),
            "the comment is within the look-back"
        );
        assert_eq!(
            last_is_comment(&build(LOOKBACK_LINES)),
            Some(false),
            "a comment past the look-back is not seen"
        );
        let wide = vec![
            "/* open".to_owned(),
            "x".repeat(LOOKBACK_BYTES),
            "fn here() {}".to_owned(),
        ];
        assert_eq!(
            last_is_comment(&wide),
            Some(false),
            "the byte bound holds too"
        );
    }

    /// The symbol of every cell from row `from` below, a row to a list.
    fn symbols(buffer: &Buffer, from: u16, rows: u16) -> Vec<Vec<String>> {
        (from..rows)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_owned())
                    .collect()
            })
            .collect()
    }

    /// `source` drawn as the file `path` in the raw reader, width 80, drawn look.
    fn drawn_file(path: &str, source: &str) -> Buffer {
        let body = reader::decode(source.as_bytes(), 0);
        let Body::Text { lines, .. } = &body else {
            return Buffer::empty(Rect::new(0, 0, 80, 6));
        };
        let flow = reader::flow(lines, 80 - reader::gutter(lines.len()));
        let view = View {
            path,
            line: None,
            body: Some(&body),
            flow: Some(&flow),
            top: 0,
            links: 1,
        };
        let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 6));
        let _ = crate::app::draw::draw_reader(&view, Some(Look::DEFAULT), &mut buffer);
        buffer
    }

    /// `source` in a `~~~` fence of `tag` (none: untagged), drawn as rendered Markdown.
    fn drawn_fence(tag: &str, source: &str) -> Buffer {
        let text = format!("~~~{tag}\n{source}\n~~~\n");
        let lines: Vec<String> = text.lines().map(str::to_owned).collect();
        let rendered = markdown::render(&text, 80);
        let view = markdown::View {
            path: "/f/sample.md",
            line: None,
            lines: &lines,
            shown: markdown::Shown::Rendered(&rendered),
            refusal: None,
            top: 0,
            links: 1,
        };
        let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 6));
        let _ = crate::app::draw::draw_markdown(&view, Some(Look::DEFAULT), &mut buffer);
        buffer
    }

    #[test]
    fn a_combining_mark_after_a_token_is_drawn_as_the_plain_reader_draws_it() {
        // The mark follows a closing quote or a comment end, where a run ends.
        // A span that began with it would lose it (a zero-width grapheme on its
        // own is not drawn), so every drawn symbol must equal the plain draw's.
        let cases = [
            ("rs", "rust", "let s = \"x\"\u{0301};"),
            ("rs", "rust", "/* c */\u{0301} fn"),
            ("rs", "rust", "let c = 'a'\u{0301}; // e\u{0301}"),
            ("py", "python", "s = 'x'\u{0301} + \"y\"\u{0308}"),
            ("sh", "bash", "echo \"x\"\u{0301} $y"),
            ("json", "json", "{\"k\"\u{0301}: \"v\"\u{0301}}"),
            ("toml", "toml", "k = \"v\"\u{0301}"),
            ("js", "js", "const s = `a`\u{0301}; /* c */\u{200D}x"),
            // Spacing marks have a width of their own, but still end no grapheme.
            ("rs", "rust", "let s = \"x\"\u{0903};"),
            ("rs", "rust", "let s = \"x\"\u{093E}; /* c */\u{0903}"),
            ("py", "python", "s = 'x'\u{093E}\u{0903}"),
        ];
        for (ext, tag, source) in cases {
            let named = format!("/f/sample.{ext}");
            let tagged = symbols(&drawn_file(&named, source), 1, 4);
            let plain = symbols(&drawn_file("/f/sample.txt", source), 1, 4);
            assert_eq!(tagged, plain, "a {ext} file: {source:?}");
            let fenced = symbols(&drawn_fence(tag, source), 0, 6);
            let bare = symbols(&drawn_fence("", source), 0, 6);
            assert_eq!(fenced, bare, "a {tag} fence: {source:?}");
        }
    }

    #[test]
    fn a_line_that_opens_with_a_control_and_a_wide_scalar_is_scanned_whole() {
        // A control is no grapheme of the drawing library, so such a line has
        // no grapheme start at byte 0: the first run must not be re-examined.
        for lang in [
            Lang::Rust,
            Lang::Python,
            Lang::Shell,
            Lang::Toml,
            Lang::Json,
            Lang::Yaml,
            Lang::Script,
        ] {
            for line in [
                "\u{1b}é let",
                "\u{7}\u{0301}x \"y\"\u{0301}",
                "\u{1b}",
                // U+0600 prepends to the quote after it: a run starts inside its cluster.
                "\u{1b}\u{0600}\"a\"",
            ] {
                let runs = Scanner::new(lang).line(line);
                tile(&runs, line);
            }
        }
    }

    #[test]
    fn the_look_back_survives_a_line_that_opens_with_a_control() {
        let lines: Vec<String> = ["\u{1b}é /* open", "text", "fn here() {}"]
            .iter()
            .map(|line| (*line).to_owned())
            .collect();
        assert_eq!(last_is_comment(&lines), Some(true));
    }

    #[test]
    fn a_run_a_grapheme_swallows_joins_the_equal_runs_around_it() {
        // The mark run lies wholly inside the cluster of the quote before it,
        // so it goes to the string before it, which then meets the next string.
        let line = "\"a\"\u{0301}\"b\"";
        assert_eq!(marked(Lang::Rust, line), [(Token::Str, line.to_owned())]);
        // Before a run of another kind it joins neither of them: the comment
        // after it stays a comment.
        let line = "\"a\"\u{0301}// c";
        assert_eq!(
            marked(Lang::Rust, line),
            [
                (Token::Str, "\"a\"\u{0301}".to_owned()),
                (Token::Comment, "// c".to_owned())
            ]
        );
        // A run that only starts inside the cluster gives up that part of it.
        let line = "\"a\"\u{0301}fn";
        assert_eq!(
            marked(Lang::Rust, line),
            [(Token::Str, "\"a\"\u{0301}".to_owned())]
        );
    }

    #[test]
    fn the_fence_run_budget_is_met_at_its_edge() {
        assert!(within(5, 5));
        assert!(within(5, 0));
        assert!(within(0, 0));
        assert!(!within(5, 6));
        assert!(!within(0, 1));
    }

    #[test]
    fn the_layout_run_budget_is_shared_by_its_fences() {
        // Each line is 1000 runs (a number, a blank, then 499 of each): the
        // budget of MAX_RUNS pays for MAX_RUNS / 1000 lines of the whole layout,
        // so a second fence gets only what the first left.
        let paid = MAX_RUNS / 1000;
        let (first, second) = (paid * 3 / 4, paid / 2);
        let mut source = String::new();
        for lines in [first, second] {
            source.push_str("```py\n");
            for line in 0..lines {
                let _ = writeln!(source, "{line:03} {}", "1 ".repeat(499));
            }
            source.push_str("```\n\n");
        }
        let rendered = markdown::render(&source, 80);
        assert!(
            rendered.refusal().is_none(),
            "the fixture is within the reader budget"
        );
        let starts: Vec<usize> = (0..rendered.rows())
            .filter(|row| {
                let text = rendered.text(*row).unwrap_or_default().as_bytes();
                text.len() > 3 && text[..3].iter().all(u8::is_ascii_digit) && text[3] == b' '
            })
            .collect();
        assert_eq!(
            starts.len(),
            first + second,
            "one start row per source line"
        );
        for (line, row) in starts.iter().enumerate() {
            let want = if line < paid {
                Token::Number
            } else {
                Token::Plain
            };
            assert_eq!(rendered.marks(*row, 0).syn, want, "line {line}");
        }
    }

    #[test]
    fn a_raw_window_stops_colouring_when_its_runs_use_the_budget_up() {
        let paid = MAX_RUNS / 1000;
        let line = "1 ".repeat(500);
        let lines: Vec<&str> = (0..=paid).map(|_| line.as_str()).collect();
        let rows = painted("/f/a.rs", &lines, 2000);
        assert_eq!(rows.len(), paid + 1, "one row per line");
        for (at, row) in rows.iter().enumerate() {
            assert_eq!(row.is_some(), at < paid, "row {at}");
        }
    }

    #[test]
    fn a_fence_of_a_known_language_marks_its_runs_and_nothing_else() {
        let source = "```rust\nlet a = 1; /* x\nstill */ fn\n```\n\n```text\nlet a\n```\n\nlet\n";
        let rendered = markdown::render(source, 40);
        let syn = |row: usize, byte: usize| rendered.marks(row, byte).syn;
        let text_of = |row| rendered.text(row).unwrap_or_default().to_owned();
        let let_row = (0..rendered.rows())
            .find(|row| text_of(*row).starts_with("let a = 1"))
            .unwrap_or(0);
        assert_eq!(syn(let_row, 0), Token::Keyword);
        assert_eq!(syn(let_row, 8), Token::Number);
        assert_eq!(
            syn(let_row + 1, 0),
            Token::Comment,
            "the block comment goes on"
        );
        assert_eq!(syn(let_row + 1, 9), Token::Keyword);
        for row in 0..rendered.rows() {
            let text = text_of(row);
            if (text.starts_with("let a") && row > let_row + 1) || text == "let" {
                assert_eq!(
                    syn(row, 0),
                    Token::Plain,
                    "{text:?} is no fence of a language"
                );
            }
        }
        // The lane's layout keeps every fence plain, and the text is the same.
        let doc = markdown::Document::parse(source);
        let lane = doc.layout_in(40, &markdown::Budget::LANE, markdown::Opts::default());
        assert_eq!(lane.rows(), rendered.rows());
        for row in 0..lane.rows() {
            assert_eq!(lane.text(row), rendered.text(row));
            assert!(
                lane.runs(row)
                    .iter()
                    .all(|(_, marks)| marks.syn == Token::Plain)
            );
        }
    }

    /// One expected non-plain run of `marked`.
    fn run(token: Token, text: &str) -> (Token, String) {
        (token, text.to_owned())
    }

    /// `text`'s runs, plain ones left out, read by the scanner that has read
    /// the line `before` first.
    fn marked_after(lang: Lang, before: &str, text: &str) -> Vec<(Token, String)> {
        let mut scanner = Scanner::new(lang);
        let _ = scanner.line(before);
        scanner
            .line(text)
            .into_iter()
            .filter(|(token, _)| *token != Token::Plain)
            .map(|(token, range)| (token, text[range].to_owned()))
            .collect()
    }

    /// The raw reader's colouring of `lines` at `width`, from the first row:
    /// `then` gets it and the number of display rows. `None` when it reads nothing.
    fn reading_of<R>(
        path: &str,
        lines: &[&str],
        width: usize,
        then: impl FnOnce(&mut Reading<'_>, usize) -> R,
    ) -> Option<R> {
        let body = Body::Text {
            lines: lines.iter().map(|line| (*line).to_owned()).collect(),
            size: 0,
        };
        let Body::Text { lines, .. } = &body else {
            return None;
        };
        let flow = reader::flow(lines, width);
        let view = View {
            path,
            line: None,
            body: Some(&body),
            flow: Some(&flow),
            top: 0,
            links: 0,
        };
        let look = Look::DEFAULT;
        let mut reading = Reading::open(&view, &flow, 0, Paint::of(Some(&look)))?;
        Some(then(&mut reading, flow.rows()))
    }

    #[test]
    fn a_long_language_name_is_a_fence_tag_and_never_an_extension() {
        for name in ["rust", "python", "shell", "javascript", "typescript"] {
            assert_eq!(lang_of_path(&format!("a.{name}")), None, "{name}");
            assert!(lang_of_tag(name).is_some(), "{name}");
        }
    }

    #[test]
    fn a_language_opens_only_the_quotes_it_has() {
        // A JSON string is double-quoted only (RFC 8259, section 7).
        assert_eq!(marked(Lang::Json, "'x'"), []);
        // A backtick quotes a string in JavaScript and TypeScript alone.
        for lang in [
            Lang::Rust,
            Lang::Python,
            Lang::Shell,
            Lang::Toml,
            Lang::Json,
            Lang::Yaml,
        ] {
            assert_eq!(marked(lang, "`x`"), [], "{lang:?}");
        }
        assert_eq!(marked(Lang::Script, "`x`"), [run(Token::Str, "`x`")]);
    }

    #[test]
    fn only_a_block_a_triple_or_a_template_string_goes_on_to_the_next_line() {
        // A string cut at the line's end, or closed on it, leaves the next
        // line to be read as code.
        for (lang, before, word) in [
            (Lang::Rust, "let s = \"cut", "fn"),
            (Lang::Python, "x = 'cut", "def"),
            (Lang::Python, "x = '''a'''", "def"),
            (Lang::Shell, "echo \"cut", "fi"),
            (Lang::Toml, "k = \"\"\"a\"\"\"", "true"),
            (Lang::Json, "\"cut", "null"),
            (Lang::Yaml, "k: \"cut", "true"),
            (Lang::Script, "const s = \"cut", "let"),
            (Lang::Script, "const s = `a`;", "let"),
        ] {
            assert_eq!(
                marked_after(lang, before, word),
                [run(Token::Keyword, word)],
                "{lang:?} after {before:?}"
            );
        }
    }

    #[test]
    fn a_triple_string_closed_on_its_line_ends_there_and_an_escape_does_not_close_it() {
        assert_eq!(
            marked(Lang::Python, "x = '''a''' + 1"),
            [run(Token::Str, "'''a'''"), run(Token::Number, "1")]
        );
        assert_eq!(
            marked(Lang::Python, "'''a''' 1"),
            [run(Token::Str, "'''a'''"), run(Token::Number, "1")]
        );
        assert_eq!(
            marked(Lang::Toml, "k = \"\"\"a\"\"\" # c"),
            [
                run(Token::Key, "k"),
                run(Token::Str, "\"\"\"a\"\"\""),
                run(Token::Comment, "# c")
            ]
        );
        // The backslash takes the first of the four quotes: the other three close it.
        assert_eq!(
            marked(Lang::Python, "s = '''a\\'''' + 1"),
            [run(Token::Str, "'''a\\''''"), run(Token::Number, "1")]
        );
    }

    #[test]
    fn only_r_br_and_b_prefix_a_rust_string() {
        // Any other word before a quote or a hash is a word of its own.
        for line in ["let s = xr\"a\";", "let s = c\"a\";", "let s = ar#\"a\"#;"] {
            assert_eq!(
                marked(Lang::Rust, line),
                [run(Token::Keyword, "let"), run(Token::Str, "\"a\"")],
                "{line:?}"
            );
        }
        assert_eq!(
            marked(Lang::Rust, "br\"a\" r\"b\""),
            [run(Token::Str, "br\"a\""), run(Token::Str, "r\"b\"")]
        );
    }

    #[test]
    fn equal_neighbours_are_one_run_and_the_runs_of_two_lines_are_kept_apart() {
        assert_eq!(
            Scanner::new(Lang::Rust).line("a + b"),
            [(Token::Plain, 0..5)]
        );
        // `scan` appends: the second line's offsets are its own, so its run
        // does not join the last of the first line.
        let mut scanner = Scanner::new(Lang::Rust);
        let mut runs = Vec::new();
        scanner.scan("a", &mut runs);
        scanner.scan("b", &mut runs);
        assert_eq!(runs, [(Token::Plain, 0..1), (Token::Plain, 0..1)]);
    }

    #[test]
    fn a_dot_belongs_to_a_number_only_between_digits() {
        assert_eq!(
            marked(Lang::Rust, "0..5"),
            [run(Token::Number, "0"), run(Token::Number, "5")]
        );
        assert_eq!(marked(Lang::Rust, "1.foo()"), [run(Token::Number, "1")]);
        assert_eq!(marked(Lang::Rust, "1.5"), [run(Token::Number, "1.5")]);
    }

    #[test]
    fn a_rust_character_literal_is_one_string_whatever_scalar_or_escape_it_holds() {
        // The Rust reference (character literals): one scalar, or one escape.
        for literal in [
            "'a'",
            "'\\n'",
            "'\\\\'",
            "'\\x41'",
            "'\\u{e9}'",
            "'\\u{10FFFF}'",
            "'é'",
            "'界'",
            "'😀'",
        ] {
            assert_eq!(
                marked(Lang::Rust, &format!("{literal} + 1")),
                [run(Token::Str, literal), run(Token::Number, "1")],
                "{literal}"
            );
            // Later in the line, where an offset bound that counts from the
            // line's start cannot reach it.
            assert_eq!(
                marked(Lang::Rust, &format!("let a_long_name = {literal} + 1")),
                [
                    run(Token::Keyword, "let"),
                    run(Token::Str, literal),
                    run(Token::Number, "1")
                ],
                "{literal}"
            );
        }
        // Not a literal: no scalar between the quotes, and no escape runs on
        // so long (the longest is `'\u{10FFFF}'`).
        assert_eq!(marked(Lang::Rust, "'''"), []);
        assert_eq!(marked(Lang::Rust, "x = '\\abcdefghijklmnop'"), []);
    }

    #[test]
    fn toml_keys_are_bare_or_quoted_and_an_equals_sign_follows_them() {
        for (line, key) in [
            ("\"quoted key\" = 1", "\"quoted key\""),
            ("'lit key' = 1", "'lit key'"),
            // A basic string has escapes, a literal string has none (the TOML spec).
            ("\"a\\\"b\" = 1", "\"a\\\"b\""),
            ("'C:\\' = 1", "'C:\\'"),
        ] {
            assert_eq!(
                marked(Lang::Toml, line),
                [run(Token::Key, key), run(Token::Number, "1")],
                "{line:?}"
            );
        }
        // No equals sign, no key.
        assert_eq!(marked(Lang::Toml, "just words"), []);
        assert_eq!(
            marked(Lang::Toml, "  \"alone\""),
            [run(Token::Str, "\"alone\"")]
        );
        // The inside of an array-of-tables header is the key, brackets left out.
        assert_eq!(marked(Lang::Toml, "[[bin]]"), [run(Token::Key, "bin")]);
        assert_eq!(
            marked(Lang::Toml, "  [[bin.x]] # c"),
            [run(Token::Key, "bin.x"), run(Token::Comment, "# c")]
        );
    }

    #[test]
    fn yaml_keys_end_at_their_colon_and_a_comment_needs_a_blank_before_it() {
        // A plain key may hold blanks, and a `#` that no blank precedes.
        assert_eq!(marked(Lang::Yaml, "my key: v"), [run(Token::Key, "my key")]);
        assert_eq!(
            marked(Lang::Yaml, "a#b: 1"),
            [run(Token::Key, "a#b"), run(Token::Number, "1")]
        );
        assert_eq!(marked(Lang::Yaml, "a# b: c"), [run(Token::Key, "a# b")]);
        // A `#` after a blank starts a comment, whatever a colon later says.
        assert_eq!(
            marked(Lang::Yaml, "a # b: c"),
            [run(Token::Comment, "# b: c")]
        );
        assert_eq!(
            marked(Lang::Yaml, "a #b: c"),
            [run(Token::Comment, "#b: c")]
        );
        // The blanks before the colon are not the key's.
        assert_eq!(marked(Lang::Yaml, "key : v"), [run(Token::Key, "key")]);
        // A double-quoted key has escapes, a single-quoted one has none (the YAML spec).
        assert_eq!(
            marked(Lang::Yaml, "\"a\\\"b\": 1"),
            [run(Token::Key, "\"a\\\"b\""), run(Token::Number, "1")]
        );
        assert_eq!(
            marked(Lang::Yaml, "'a\\': 1"),
            [run(Token::Key, "'a\\'"), run(Token::Number, "1")]
        );
    }

    #[test]
    fn the_look_back_reads_exactly_its_byte_bound() {
        let above = |filler: usize| {
            vec![
                "/* open".to_owned(),
                "x".repeat(filler),
                "fn here() {}".to_owned(),
            ]
        };
        // The two lines above hold LOOKBACK_BYTES bytes: all are read.
        assert_eq!(
            last_is_comment(&above(LOOKBACK_BYTES - "/* open".len())),
            Some(true)
        );
        // One more byte and the comment's line is beyond the bound.
        assert_eq!(
            last_is_comment(&above(LOOKBACK_BYTES - "/* open".len() + 1)),
            Some(false)
        );
    }

    #[test]
    fn a_character_literal_of_a_quote_escape_ends_at_its_own_closing_quote() {
        // The Rust reference (character literals): `\'` and `\"` are quote
        // escapes, so each of these is one literal of four bytes. The quote
        // after the backslash is the escaped character, not the closing one.
        for literal in ["'\\''", "'\\\"'"] {
            assert_eq!(
                marked(Lang::Rust, &format!("let q = {literal};")),
                [run(Token::Keyword, "let"), run(Token::Str, literal)],
                "{literal}"
            );
            assert_eq!(
                marked(Lang::Rust, &format!("{literal} + 1")),
                [run(Token::Str, literal), run(Token::Number, "1")],
                "{literal}"
            );
        }
        // No closing quote follows: the quote is plain, as a lifetime's is.
        assert_eq!(marked(Lang::Rust, "x = '\\'a"), []);
    }

    #[test]
    fn a_row_is_never_an_empty_span() {
        let lines = [
            "fn a() { let s = \"aaa bbb ccc\"; } // and a comment that wraps",
            "    let long_name = 12345; // another one over rows",
            "x",
        ];
        for width in 1..40 {
            let rows = reading_of("/f/a.rs", &lines, width, |reading, rows| {
                (0..rows).map(|at| reading.row(at)).collect::<Vec<_>>()
            })
            .unwrap_or_default();
            assert!(!rows.is_empty(), "width {width}");
            for row in rows {
                let Some(line) = row else {
                    panic!("width {width}: a row left plain");
                };
                assert!(
                    line.spans.iter().all(|span| !span.content.is_empty()),
                    "width {width}: {line:?}"
                );
            }
        }
    }

    #[test]
    fn a_row_above_the_lines_already_read_is_left_plain() {
        // The scanner only follows the rows down: line 0 cannot be placed once
        // line 2 is read, whatever text the lines hold.
        let lines = ["let x = 1;"; 3];
        let got = reading_of("/f/a.rs", &lines, 80, |reading, _| {
            (
                reading.row(2).is_some(),
                reading.row(0).is_none(),
                reading.row(2).is_some(),
            )
        });
        assert_eq!(got, Some((true, true, true)));
    }

    #[test]
    fn a_row_past_the_run_budget_stays_plain_each_time_it_is_asked_for() {
        let paid = MAX_RUNS / 1000;
        let line = "1 ".repeat(500);
        let lines: Vec<&str> = (0..=paid).map(|_| line.as_str()).collect();
        let got = reading_of("/f/a.rs", &lines, 2000, |reading, rows| {
            for at in 0..rows {
                let _ = reading.row(at);
            }
            (0..3)
                .map(|_| reading.row(rows - 1).is_none())
                .collect::<Vec<_>>()
        });
        assert_eq!(got, Some(vec![true; 3]));
    }
}
