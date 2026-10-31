//! One honest source view for the text guards.
//!
//! A text guard asks "does production code spell X". It is only as honest as
//! what it was shown, so this module owns what a guard may be shown: every
//! production byte of every `src/` file, or a failure naming the file and line
//! the view could not classify. Unknown never reads as test code.
//!
//! The view of one file ([`classify`]) keeps the source's length and newlines:
//! comments and every `#[cfg(test)]` item are replaced by spaces, so an offset
//! or a line number in the view is the source's own. String and character
//! literals stay verbatim, because the live-file spelling guards read them.
//! The view of the tree ([`product_of`]) follows `mod` registrations from the
//! crate roots by rustc's own file rule, so a file is test-only because it is
//! registered by test code alone, never because of what it is called.
//!
//! BOUNDED SYNTAX. This is a lexer plus a finite table of gated item shapes
//! calibrated against the registrations that exist in `src/`, not a Rust
//! parser. A shape outside the table is a [`Failure`] at its `#[cfg(test)]`
//! line; adding a shape is a review, never a fallback. What the table reads:
//!
//! | after `#[cfg(test)]` (and any attribute run, `pub`, `pub(..)`)  | ends at |
//! |---|---|
//! | `mod NAME;` (a registration) / `mod NAME { .. }` | `;` / the matching `}` |
//! | `fn`, `struct`, `enum`, `union`, `trait` | first depth-0 `{ .. }`, or `;` |
//! | `impl` | first depth-0 `{ .. }` |
//! | `const`, `static`, `type`, `use` | first depth-0 `;` |
//! | `NAME! ( .. )` / `[ .. ]` / `{ .. }` (`include!`, `thread_local!`) | the group, plus `;` for `( )` and `[ ]` |
//! | `if .. { .. }` with `else if` / `else` | the last `}` |
//! | enum variant `PATH`, `PATH( .. )`, `PATH { .. }` | the `,` |
//! | struct field or initialiser `NAME: ..` | the depth-0 `,` |
//! | match arm `PATH [group] => ..` | `{ .. }` plus optional `,`, else the `,` |
//!
//! Attributes: `cfg(test)` gates; `cfg(not(test))` does not; `cfg_attr` is
//! read only when every attribute it adds is `allow`, `expect`, `warn`, `deny`
//! or `forbid`; any other `cfg(..)` that mentions `test` is refused.

#![allow(
    clippy::disallowed_methods,
    reason = "this module is the view's one door to the filesystem: it reads the crate's own \
              source, and the capability boundary is about what PRODUCT code may reach"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

/// A file the view could not read or classify, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failure {
    /// The file, as the view names it (root-relative, `/`-separated).
    pub(crate) file: String,
    /// The source line it could not classify; `0` only for a failed read.
    pub(crate) line: usize,
    /// Why.
    pub(crate) what: String,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.line, self.what)
    }
}

fn fail(file: &str, line: usize, what: impl Into<String>) -> Failure {
    Failure {
        file: file.to_owned(),
        line,
        what: what.into(),
    }
}

/// One inline module a registration sits inside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Nest {
    pub(crate) name: String,
    pub(crate) path_attr: Option<String>,
}

/// One `mod NAME;` of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Registration {
    pub(crate) name: String,
    /// The line of its `mod` keyword.
    pub(crate) line: usize,
    /// The `#[path = ".."]` value, as written.
    pub(crate) path_attr: Option<String>,
    /// Gated by `#[cfg(test)]`, or inside something that is.
    pub(crate) gated: bool,
    /// The inline modules around it, outermost first.
    pub(crate) nesting: Vec<Nest>,
}

/// One file, classified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Classified {
    /// The source with comments and `#[cfg(test)]` items blanked.
    pub(crate) production: String,
    pub(crate) registrations: Vec<Registration>,
}

// ---- the lexer ---------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Region {
    Comment,
    Literal,
}

struct Span {
    start: usize,
    end: usize,
    region: Region,
}

fn line_at(text: &str, at: usize) -> usize {
    1 + text.bytes().take(at).filter(|byte| *byte == b'\n').count()
}

fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// The index after the nested block comment opening at `at`.
fn block_end(b: &[u8], at: usize) -> Option<usize> {
    let (mut depth, mut i) = (1_usize, at + 2);
    while i < b.len() {
        match (b.get(i), b.get(i + 1)) {
            (Some(b'/'), Some(b'*')) => {
                depth += 1;
                i += 2;
            }
            (Some(b'*'), Some(b'/')) => {
                depth -= 1;
                i += 2;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => i += 1,
        }
    }
    None
}

/// The index after the closing `quote` of a literal whose body starts at `from`.
fn quoted_end(b: &[u8], from: usize, quote: u8) -> Option<usize> {
    let mut i = from;
    while let Some(&byte) = b.get(i) {
        if byte == b'\\' {
            i += 2;
        } else if byte == quote {
            return Some(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

/// The number of `#` of a raw string whose `r` is at `at`, if one opens there.
fn raw_open(b: &[u8], at: usize) -> Option<usize> {
    let before = |back: usize| at.checked_sub(back).and_then(|i| b.get(i)).copied();
    let lone_prefix = matches!(before(1), Some(b'b' | b'c')) && !before(2).is_some_and(is_ident);
    if before(1).is_some_and(is_ident) && !lone_prefix {
        return None;
    }
    let hashes = b[at + 1..].iter().take_while(|byte| **byte == b'#').count();
    (b.get(at + 1 + hashes) == Some(&b'"')).then_some(hashes)
}

/// The index after the closing `"` and `hashes` `#` of a raw string body at `from`.
fn raw_end(b: &[u8], from: usize, hashes: usize) -> Option<usize> {
    (from..b.len()).find_map(|i| {
        let closes =
            b.get(i) == Some(&b'"') && (1..=hashes).all(|step| b.get(i + step) == Some(&b'#'));
        closes.then_some(i + 1 + hashes)
    })
}

/// The end of the character literal opening at `at`, or `None` for a lifetime.
fn char_end(text: &str, at: usize) -> Option<usize> {
    let b = text.as_bytes();
    if b.get(at + 1) == Some(&b'\\') {
        let skipped = text.get(at + 2..)?.chars().next()?.len_utf8();
        let mut i = at + 2 + skipped;
        while b
            .get(i)
            .is_some_and(|byte| *byte != b'\'' && *byte != b'\n')
        {
            i += 1;
        }
        return (b.get(i) == Some(&b'\'')).then_some(i + 1);
    }
    let one = text.get(at + 1..)?.chars().next()?;
    let after = at + 1 + one.len_utf8();
    (one != '\'' && one != '\n' && b.get(after) == Some(&b'\'')).then_some(after + 1)
}

/// Every comment and every string or character literal of `text`.
fn spans(file: &str, text: &str) -> Result<Vec<Span>, Failure> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let unterminated = |what: &str, at: usize| fail(file, line_at(text, at), what);
    while let Some(&byte) = b.get(i) {
        let (end, region) = match byte {
            b'/' if b.get(i + 1) == Some(&b'/') => (
                b[i..]
                    .iter()
                    .position(|c| *c == b'\n')
                    .map_or(b.len(), |p| i + p),
                Region::Comment,
            ),
            b'/' if b.get(i + 1) == Some(&b'*') => (
                block_end(b, i).ok_or_else(|| unterminated("unterminated block comment", i))?,
                Region::Comment,
            ),
            b'"' => (
                quoted_end(b, i + 1, b'"')
                    .ok_or_else(|| unterminated("unterminated string literal", i))?,
                Region::Literal,
            ),
            b'r' if raw_open(b, i).is_some() => {
                let hashes = raw_open(b, i).unwrap_or(0);
                (
                    raw_end(b, i + 2 + hashes, hashes)
                        .ok_or_else(|| unterminated("unterminated raw string literal", i))?,
                    Region::Literal,
                )
            }
            b'\'' => {
                let Some(end) = char_end(text, i) else {
                    i += 1;
                    continue;
                };
                (end, Region::Literal)
            }
            _ => {
                i += 1;
                continue;
            }
        };
        out.push(Span {
            start: i,
            end,
            region,
        });
        i = end;
    }
    Ok(out)
}

/// Replace `start..end` with spaces, keeping newlines, so offsets and lines hold.
fn blank_range(bytes: &mut [u8], start: usize, end: usize) {
    for byte in bytes.iter_mut().take(end).skip(start) {
        if *byte != b'\n' {
            *byte = b' ';
        }
    }
}

fn blanked(
    file: &str,
    text: &str,
    spans: &[Span],
    blank: impl Fn(Region) -> bool,
) -> Result<String, Failure> {
    let mut bytes = text.as_bytes().to_vec();
    for span in spans.iter().filter(|span| blank(span.region)) {
        blank_range(&mut bytes, span.start, span.end);
    }
    into_text(file, bytes)
}

fn into_text(file: &str, bytes: Vec<u8>) -> Result<String, Failure> {
    String::from_utf8(bytes).map_err(|_| fail(file, 0, "a blanked range split a character"))
}

/// `text` with every comment (line, doc, nested block) blanked to spaces.
pub(crate) fn without_comments(file: &str, text: &str) -> Result<String, Failure> {
    blanked(file, text, &spans(file, text)?, |region| {
        region == Region::Comment
    })
}

/// `text` with every string and character literal blanked to spaces, quotes
/// included, one space per byte.
pub(crate) fn blank_literals(file: &str, text: &str) -> Result<String, Failure> {
    blanked(file, text, &spans(file, text)?, |region| {
        region == Region::Literal
    })
}

// ---- the classifier ----------------------------------------------------------

enum AttrClass {
    Gate,
    Path(String),
    Plain,
}

/// A run of attributes, `#[..] #[..]`, and what it says about the item after it.
struct Run {
    start: usize,
    end: usize,
    gate: bool,
    /// Where the gate attribute starts: the line every failure about the gated
    /// item names.
    gate_at: usize,
    path: Option<String>,
}

struct Scan<'a> {
    file: &'a str,
    text: &'a str,
    /// `text` with comments and literals blanked: what structure is read from.
    s: &'a [u8],
    cuts: Vec<(usize, usize)>,
    regs: Vec<Registration>,
}

/// Words that start a statement or expression, never a variant, field or arm.
const NOT_AN_ELEMENT: [&str; 9] = [
    "let", "match", "for", "while", "loop", "return", "break", "continue", "move",
];

impl Scan<'_> {
    fn fail(&self, at: usize, what: impl Into<String>) -> Failure {
        fail(self.file, line_at(self.text, at), what)
    }

    fn at(&self, i: usize) -> u8 {
        self.s.get(i).copied().unwrap_or(0)
    }

    fn ws(&self, mut i: usize) -> usize {
        while self.at(i).is_ascii_whitespace() {
            i += 1;
        }
        i
    }

    /// The identifier at `at` past any space, and the index after it.
    fn word(&self, at: usize) -> (&str, usize) {
        let start = self.ws(at);
        let mut end = start;
        while is_ident(self.at(end)) {
            end += 1;
        }
        (self.text.get(start..end).unwrap_or(""), end)
    }

    /// The index after the delimiter group opening at `open`, kinds matched.
    fn group_end(&self, open: usize, limit: usize) -> Option<usize> {
        let mut stack = Vec::new();
        for i in open..limit.min(self.s.len()) {
            match self.at(i) {
                b'(' => stack.push(b')'),
                b'[' => stack.push(b']'),
                b'{' => stack.push(b'}'),
                close @ (b')' | b']' | b'}') => {
                    if stack.pop() != Some(close) {
                        return None;
                    }
                    if stack.is_empty() {
                        return Some(i + 1);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// The first depth-0 byte of `from..limit` in `ends`; groups are skipped.
    fn scan_to(
        &self,
        from: usize,
        limit: usize,
        ends: &[u8],
        forbid: &[u8],
    ) -> Result<usize, &'static str> {
        let mut i = from;
        while i < limit {
            let byte = self.at(i);
            if ends.contains(&byte) {
                return Ok(i);
            }
            let arrow =
                byte == b'>' && matches!(i.checked_sub(1).map(|p| self.at(p)), Some(b'-' | b'='));
            if forbid.contains(&byte) && !arrow {
                return Err(if byte == b';' {
                    "a `;` ends the statement before its `,`"
                } else {
                    "an angle bracket may hold a `,` this view cannot see"
                });
            }
            match byte {
                b'(' | b'[' | b'{' => {
                    i = self.group_end(i, limit).ok_or("an unbalanced delimiter")?;
                    continue;
                }
                b')' | b']' | b'}' => return Err("an unmatched closing delimiter"),
                _ => {}
            }
            i += 1;
        }
        Err("the item never ends")
    }

    /// The position after `pub` and `pub(..)`, if present.
    fn visibility(&self, at: usize) -> usize {
        let (word, end) = self.word(at);
        if word != "pub" {
            return at;
        }
        let open = self.ws(end);
        if self.at(open) == b'(' {
            self.group_end(open, self.s.len()).unwrap_or(open)
        } else {
            end
        }
    }

    /// The attributes starting at `at` (`#[` or `#![`).
    fn run(&self, at: usize) -> Result<Run, Failure> {
        let mut run = Run {
            start: at,
            end: at,
            gate: false,
            gate_at: at,
            path: None,
        };
        let mut j = at;
        loop {
            let inner = self.at(j + 1) == b'!';
            let open = if inner { j + 2 } else { j + 1 };
            let end = self
                .group_end(open, self.s.len())
                .ok_or_else(|| self.fail(j, "an attribute never closes"))?;
            let body = self.s.get(open + 1..end - 1).unwrap_or_default();
            let norm: String = body
                .iter()
                .filter(|byte| !byte.is_ascii_whitespace())
                .map(|byte| char::from(*byte))
                .collect();
            match self.attribute(j, &norm, (open + 1, end - 1))? {
                AttrClass::Gate if inner => {
                    return Err(self.fail(j, "an inner #![cfg(test)] gates the whole file"));
                }
                AttrClass::Gate if !run.gate => (run.gate, run.gate_at) = (true, j),
                AttrClass::Path(path) => run.path = Some(path),
                AttrClass::Gate | AttrClass::Plain => {}
            }
            run.end = end;
            let next = self.ws(end);
            if inner || self.at(next) != b'#' || self.at(next + 1) != b'[' {
                return Ok(run);
            }
            j = next;
        }
    }

    /// What one attribute (whitespace removed, literals blanked) says.
    fn attribute(&self, at: usize, norm: &str, body: (usize, usize)) -> Result<AttrClass, Failure> {
        if norm == "cfg(test)" {
            return Ok(AttrClass::Gate);
        }
        if norm == "cfg(not(test))" {
            return Ok(AttrClass::Plain);
        }
        if let Some(args) = norm
            .strip_prefix("cfg_attr(")
            .and_then(|rest| rest.strip_suffix(')'))
        {
            return self.conditional(at, args);
        }
        if norm.starts_with("cfg(") && words(norm).any(|word| word == "test") {
            return Err(self.fail(at, format!("an unrecognised cfg mentioning test: {norm}")));
        }
        if norm.starts_with("path=") {
            let written = self.text.get(body.0..body.1).unwrap_or("");
            let value = written
                .split('"')
                .nth(1)
                .filter(|value| !value.contains('\\') && !value.starts_with('/'))
                .ok_or_else(|| {
                    self.fail(
                        at,
                        "a #[path] that is not one plain relative string literal",
                    )
                })?;
            return Ok(AttrClass::Path(value.to_owned()));
        }
        Ok(AttrClass::Plain)
    }

    /// `cfg_attr(PREDICATE, ATTR, ..)`: read only when every ATTR is a lint.
    fn conditional(&self, at: usize, args: &str) -> Result<AttrClass, Failure> {
        let mut depth = 0_usize;
        let mut parts = vec![String::new()];
        for ch in args.chars() {
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => {
                    parts.push(String::new());
                    continue;
                }
                _ => {}
            }
            if let Some(last) = parts.last_mut() {
                last.push(ch);
            }
        }
        for part in parts.iter().skip(1) {
            let name: String = part.chars().take_while(|c| is_ident(*c as u8)).collect();
            if !matches!(
                name.as_str(),
                "allow" | "expect" | "warn" | "deny" | "forbid"
            ) {
                return Err(self.fail(
                    at,
                    format!(
                        "a cfg_attr that may change cfg or path (`{name}`); only \
                         allow, expect, warn, deny and forbid are read"
                    ),
                ));
            }
        }
        Ok(AttrClass::Plain)
    }

    /// Walk `from..to`, finding attribute runs and module registrations.
    /// `in_gate` is true inside a `#[cfg(test)]` module body.
    fn region(
        &mut self,
        (from, to): (usize, usize),
        nest: &mut Vec<Nest>,
        in_gate: bool,
    ) -> Result<(), Failure> {
        let mut i = from;
        while i < to {
            let byte = self.at(i);
            let attribute = byte == b'#'
                && (self.at(i + 1) == b'[' || (self.at(i + 1) == b'!' && self.at(i + 2) == b'['));
            if attribute {
                i = self.attributed(i, to, nest, in_gate)?;
            } else if is_ident(byte) {
                let (word, end) = self.word(i);
                let module = if word == "mod" {
                    self.module(end, (i, i), None, in_gate, nest, to)?
                } else {
                    None
                };
                i = module.unwrap_or(end);
            } else {
                i += 1;
            }
        }
        Ok(())
    }

    /// An attribute run at `at`, and the item it gates or decorates.
    fn attributed(
        &mut self,
        at: usize,
        to: usize,
        nest: &mut Vec<Nest>,
        in_gate: bool,
    ) -> Result<usize, Failure> {
        let run = self.run(at)?;
        if self.at(at + 1) == b'!' {
            return Ok(run.end);
        }
        let head = self.visibility(self.ws(run.end));
        let (word, end) = self.word(head);
        let word_at = self.ws(head);
        let gates = run.gate && !in_gate;
        if word == "mod" {
            let blame = if gates { run.gate_at } else { word_at };
            if let Some(after) = self.module(
                end,
                (word_at, blame),
                run.path.clone(),
                in_gate || run.gate,
                nest,
                to,
            )? {
                if gates {
                    self.cuts.push((run.start, after));
                }
                return Ok(after);
            }
        }
        if gates {
            let after = self.gated_item(run.gate_at, head, to)?;
            self.cuts.push((run.start, after));
            return Ok(after);
        }
        Ok(run.end)
    }

    /// `mod NAME;` or `mod NAME { .. }` after the keyword (ending at `kw_end`).
    /// `at` is the keyword and the position a failure blames. Returns the index
    /// after the whole item, or `None` when this `mod` is neither shape.
    fn module(
        &mut self,
        kw_end: usize,
        (kw_at, blame): (usize, usize),
        path_attr: Option<String>,
        gated: bool,
        nest: &mut Vec<Nest>,
        to: usize,
    ) -> Result<Option<usize>, Failure> {
        let (name, end) = self.word(kw_end);
        let name = name.to_owned();
        if name.is_empty() {
            return Ok(None);
        }
        let after = self.ws(end);
        match self.at(after) {
            b';' => {
                self.regs.push(Registration {
                    name,
                    line: line_at(self.text, kw_at),
                    path_attr,
                    gated,
                    nesting: nest.clone(),
                });
                Ok(Some(after + 1))
            }
            b'{' => {
                let close = self
                    .group_end(after, to)
                    .ok_or_else(|| self.fail(blame, "an inline module never closes"))?;
                nest.push(Nest { name, path_attr });
                let walked = self.region((after + 1, close - 1), nest, gated);
                nest.pop();
                walked?;
                Ok(Some(close))
            }
            _ => Ok(None),
        }
    }

    /// The end of the item a `#[cfg(test)]` at `gate_at` gates, starting at `from`.
    fn gated_item(&self, gate_at: usize, from: usize, to: usize) -> Result<usize, Failure> {
        let bad = |why: &str| {
            self.fail(
                gate_at,
                format!("an item after #[cfg(test)] this view does not read: {why}"),
            )
        };
        let mut at = from;
        loop {
            let (word, end) = self.word(at);
            let next = self.word(end).0;
            let qualifier = matches!(word, "unsafe" | "async" | "default" | "extern")
                || (word == "const" && matches!(next, "fn" | "unsafe" | "async" | "extern"));
            if !qualifier {
                break;
            }
            at = end;
        }
        let (word, end) = self.word(at);
        let to_end = |ends: &[u8]| self.scan_to(end, to, ends, b"").map_err(&bad);
        let closed = |open: usize| {
            self.group_end(open, to)
                .ok_or_else(|| bad("an unbalanced delimiter"))
        };
        match word {
            "fn" | "struct" | "enum" | "union" | "trait" => {
                let stop = to_end(b"{;")?;
                if self.at(stop) == b';' {
                    Ok(stop + 1)
                } else {
                    closed(stop)
                }
            }
            "impl" => closed(to_end(b"{")?),
            "const" | "static" | "type" | "use" => Ok(to_end(b";")? + 1),
            "if" => self.gated_if(end, to, &bad),
            "" if self.at(end) == b'{' => closed(end),
            "" => Err(bad("nothing readable follows")),
            _ if self.at(self.ws(end)) == b'!' && self.at(self.ws(end) + 1) != b'=' => {
                let open = self.ws(self.ws(end) + 1);
                if !matches!(self.at(open), b'(' | b'[' | b'{') {
                    return Err(bad("a macro call without a delimited argument"));
                }
                let after = closed(open)?;
                let semi = self.ws(after);
                Ok(if self.at(open) != b'{' && self.at(semi) == b';' {
                    semi + 1
                } else {
                    after
                })
            }
            _ => self.gated_element(at, to, &bad),
        }
    }

    /// `if COND { .. } else if COND { .. } else { .. }`, from after the `if`.
    fn gated_if(
        &self,
        mut at: usize,
        to: usize,
        bad: &dyn Fn(&str) -> Failure,
    ) -> Result<usize, Failure> {
        loop {
            let open = self.scan_to(at, to, b"{", b"").map_err(bad)?;
            let end = self
                .group_end(open, to)
                .ok_or_else(|| bad("an unbalanced block"))?;
            let (word, after) = self.word(end);
            if word != "else" {
                return Ok(end);
            }
            let (next, after_next) = self.word(after);
            if next == "if" {
                at = after_next;
                continue;
            }
            let block = self.ws(after);
            if self.at(block) != b'{' {
                return Err(bad("an `else` that is neither `else if` nor a block"));
            }
            return self
                .group_end(block, to)
                .ok_or_else(|| bad("an unbalanced block"));
        }
    }

    /// An enum variant, a struct field or initialiser, or a match arm.
    fn gated_element(
        &self,
        at: usize,
        to: usize,
        bad: &dyn Fn(&str) -> Failure,
    ) -> Result<usize, Failure> {
        let (first, mut end) = self.word(at);
        if NOT_AN_ELEMENT.contains(&first) || first == "else" {
            return Err(bad("a statement, not a variant, field or arm"));
        }
        let mut segments = 1;
        while self.at(end) == b':' && self.at(end + 1) == b':' {
            let (word, after) = self.word(end + 2);
            if word.is_empty() {
                return Err(bad("a path that stops after `::`"));
            }
            (end, segments) = (after, segments + 1);
        }
        let mut at_tail = self.ws(end);
        let grouped = matches!(self.at(at_tail), b'(' | b'{');
        if grouped {
            let after = self
                .group_end(at_tail, to)
                .ok_or_else(|| bad("an unbalanced delimiter"))?;
            at_tail = self.ws(after);
        }
        let until_comma = |from: usize| {
            self.scan_to(from, to, b",", b";<>")
                .map(|stop| stop + 1)
                .map_err(bad)
        };
        match self.at(at_tail) {
            b',' => Ok(at_tail + 1),
            b':' if !grouped && segments == 1 && self.at(at_tail + 1) != b':' => {
                until_comma(at_tail + 1)
            }
            b'=' if self.at(at_tail + 1) == b'>' => {
                let body = self.ws(at_tail + 2);
                if self.at(body) != b'{' {
                    return until_comma(body);
                }
                let close = self
                    .group_end(body, to)
                    .ok_or_else(|| bad("an unbalanced arm body"))?;
                let comma = self.ws(close);
                Ok(if self.at(comma) == b',' {
                    comma + 1
                } else {
                    close
                })
            }
            _ => Err(bad("not a calibrated variant, field or match arm")),
        }
    }
}

/// The identifiers of `text`.
fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .filter(|word| !word.is_empty())
}

/// Classify one file: its production text and its module registrations, or the
/// file and line the view could not read. `file` only names diagnostics.
pub(crate) fn classify(file: &str, text: &str) -> Result<Classified, Failure> {
    let spans = spans(file, text)?;
    let structural = blanked(file, text, &spans, |_| true)?;
    let mut scan = Scan {
        file,
        text,
        s: structural.as_bytes(),
        cuts: Vec::new(),
        regs: Vec::new(),
    };
    scan.region((0, structural.len()), &mut Vec::new(), false)?;
    let Scan { cuts, regs, .. } = scan;
    let mut bytes = text.as_bytes().to_vec();
    for span in spans.iter().filter(|span| span.region == Region::Comment) {
        blank_range(&mut bytes, span.start, span.end);
    }
    for (start, end) in cuts {
        blank_range(&mut bytes, start, end);
    }
    Ok(Classified {
        production: into_text(file, bytes)?,
        registrations: regs,
    })
}

// ---- file identity -----------------------------------------------------------

/// Where a registration points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// A loaded `src/` file.
    Src(String),
    /// A path outside `src/` (a test file registered with `#[path]`).
    Outside(String),
}

/// `dir/rel` with `.` and `..` resolved lexically.
fn join(dir: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in dir.split('/').chain(rel.split('/')) {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|last| *last != "..") => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// The file `reg` (a registration in `file`) names, by rustc's rule: a module
/// file of a `lib.rs`, `main.rs` or `mod.rs` sits in its directory, any other
/// file `a/b.rs` owns `a/b/`, an inline module adds its name (or its
/// `#[path]`, which also drops the file's own stem), and `#[path]` on a file
/// module is relative to the directory of the registering file.
pub(crate) fn target_of(file: &str, reg: &Registration, known: &[&str]) -> Result<Target, Failure> {
    let (parent, leaf) = file.rsplit_once('/').unwrap_or(("", file));
    let mut dir = parent.to_owned();
    let mut relative = (!matches!(leaf, "lib.rs" | "main.rs" | "mod.rs"))
        .then(|| leaf.trim_end_matches(".rs").to_owned());
    for inline in &reg.nesting {
        if let Some(path) = &inline.path_attr {
            dir = join(&dir, path);
            relative = None;
        } else {
            if let Some(stem) = relative.take() {
                dir = join(&dir, &stem);
            }
            dir = join(&dir, &inline.name);
        }
    }
    let unresolved = |why: String| fail(file, reg.line, why);
    if let Some(path) = &reg.path_attr {
        let target = join(&dir, path);
        if !target.starts_with("src/") {
            return Ok(Target::Outside(target));
        }
        return if known.contains(&target.as_str()) {
            Ok(Target::Src(target))
        } else {
            Err(unresolved(format!(
                "`mod {}` #[path] names {target}, which is not a loaded source file",
                reg.name
            )))
        };
    }
    if let Some(stem) = relative {
        dir = join(&dir, &stem);
    }
    let flat = join(&dir, &format!("{}.rs", reg.name));
    let nested = join(&dir, &format!("{}/mod.rs", reg.name));
    match (
        known.contains(&flat.as_str()),
        known.contains(&nested.as_str()),
    ) {
        (true, false) => Ok(Target::Src(flat)),
        (false, true) => Ok(Target::Src(nested)),
        (true, true) => Err(unresolved(format!(
            "`mod {}` is ambiguous: both {flat} and {nested} exist",
            reg.name
        ))),
        (false, false) => Err(unresolved(format!(
            "`mod {}` names neither {flat} nor {nested}",
            reg.name
        ))),
    }
}

/// A source file's production text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProductFile {
    pub(crate) path: String,
    pub(crate) text: String,
}

/// What the text guards may be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Product {
    /// Every file reachable from a crate root through registrations that are
    /// not gated, sorted by path.
    pub(crate) files: Vec<ProductFile>,
    /// Every other file, each registered by test code alone, sorted.
    pub(crate) test_only: Vec<String>,
}

/// The crate roots a registration chain starts from.
const ROOTS: [&str; 2] = ["src/lib.rs", "src/main.rs"];

/// Classify every file and decide which are production and which test-only.
/// `files` are `(root-relative path, text)`. A file registered by nothing, a
/// registration that names no file and an ungated registration that leaves
/// `src/` are failures: a file the view cannot place never silently drops out.
pub(crate) fn product_of(files: &[(String, String)]) -> Result<Product, Failure> {
    let known: Vec<&str> = files.iter().map(|(path, _)| path.as_str()).collect();
    let mut classified = BTreeMap::new();
    for (path, text) in files {
        classified.insert(path.as_str(), classify(path, text)?);
    }
    let regs = |file: &str| -> Vec<Registration> {
        classified
            .get(file)
            .map(|class| class.registrations.clone())
            .unwrap_or_default()
    };
    let roots: Vec<&str> = ROOTS
        .into_iter()
        .filter(|root| known.contains(root))
        .collect();
    if roots.is_empty() {
        return Err(fail(ROOTS[0], 1, "no crate root among the loaded files"));
    }

    let mut production: BTreeSet<String> = roots.iter().map(|root| (*root).to_owned()).collect();
    let mut queue: Vec<String> = production.iter().cloned().collect();
    while let Some(file) = queue.pop() {
        for reg in regs(&file).iter().filter(|reg| !reg.gated) {
            match target_of(&file, reg, &known)? {
                Target::Src(target) => {
                    if production.insert(target.clone()) {
                        queue.push(target);
                    }
                }
                Target::Outside(target) => {
                    return Err(fail(
                        &file,
                        reg.line,
                        format!(
                            "an ungated `mod {}` registers {target}, outside src/",
                            reg.name
                        ),
                    ));
                }
            }
        }
    }

    // A production file's GATED registrations seed the test-only set; from a
    // test-only file every registration, gated or not, stays test-only.
    let mut test_only = BTreeSet::new();
    let mut frontier: Vec<(String, bool)> = production.iter().map(|f| (f.clone(), true)).collect();
    while let Some((file, gated_only)) = frontier.pop() {
        for reg in regs(&file).iter().filter(|reg| reg.gated || !gated_only) {
            if let Target::Src(target) = target_of(&file, reg, &known)?
                && !production.contains(&target)
                && test_only.insert(target.clone())
            {
                frontier.push((target, false));
            }
        }
    }

    for path in &known {
        if !production.contains(*path) && !test_only.contains(*path) {
            return Err(fail(
                path,
                1,
                "registered by no module reachable from src/lib.rs or src/main.rs, \
                 and by no test-only registration",
            ));
        }
    }
    Ok(Product {
        files: production
            .into_iter()
            .map(|path| ProductFile {
                text: classified
                    .get(path.as_str())
                    .map(|class| class.production.clone())
                    .unwrap_or_default(),
                path,
            })
            .collect(),
        test_only: test_only.into_iter().collect(),
    })
}

// ---- the filesystem door -----------------------------------------------------

fn walk(root: &Path, dir: &str, out: &mut Vec<(String, String)>) -> Result<(), Failure> {
    let entries = fs::read_dir(root.join(dir))
        .map_err(|err| fail(dir, 0, format!("cannot read the directory: {err}")))?;
    for entry in entries {
        let entry = entry.map_err(|err| fail(dir, 0, format!("cannot read an entry: {err}")))?;
        let name = entry.file_name().into_string().map_err(|name| {
            fail(
                dir,
                0,
                format!("a file name that is not UTF-8: {}", name.display()),
            )
        })?;
        let path = format!("{dir}/{name}");
        let kind = entry
            .file_type()
            .map_err(|err| fail(&path, 0, format!("cannot read the file type: {err}")))?;
        if kind.is_symlink() {
            return Err(fail(
                &path,
                0,
                "a symbolic link in a scanned tree is not followed",
            ));
        }
        if kind.is_dir() {
            walk(root, &path, out)?;
        } else if Path::new(&name).extension().is_some_and(|ext| ext == "rs") {
            let text = fs::read_to_string(root.join(&path))
                .map_err(|err| fail(&path, 0, format!("cannot read the file: {err}")))?;
            out.push((path, text));
        }
    }
    Ok(())
}

/// Every `.rs` file under each of `dirs` (relative to `root`), as
/// `(root-relative path, text)`, sorted. A directory, an entry or a file that
/// cannot be read, a name or text that is not UTF-8 and a symbolic link are
/// failures that name the path: nothing is skipped.
pub(crate) fn load(root: &Path, dirs: &[&str]) -> Result<Vec<(String, String)>, Failure> {
    let mut out = Vec::new();
    for dir in dirs {
        walk(root, dir, &mut out)?;
    }
    out.sort();
    Ok(out)
}

/// Every `.rs` file of `src/` and `tests/` of this crate, read once. Panics
/// with the failure's `file:line:` when any of it cannot be read.
pub(crate) fn tree() -> &'static [(String, String)] {
    static TREE: OnceLock<Result<Vec<(String, String)>, Failure>> = OnceLock::new();
    match TREE.get_or_init(|| load(Path::new(env!("CARGO_MANIFEST_DIR")), &["src", "tests"])) {
        Ok(files) => files,
        Err(failure) => panic!("{failure}"),
    }
}

/// The product view of this crate's `src/`, built once. Panics with the
/// failure's `file:line:` when `src/` cannot be read or classified.
pub(crate) fn product() -> &'static Product {
    static PRODUCT: OnceLock<Result<Product, Failure>> = OnceLock::new();
    match PRODUCT.get_or_init(|| {
        let src: Vec<(String, String)> = tree()
            .iter()
            .filter(|(path, _)| path.starts_with("src/"))
            .cloned()
            .collect();
        product_of(&src)
    }) {
        Ok(product) => product,
        Err(failure) => panic!("{failure}"),
    }
}
