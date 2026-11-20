//! The orchestrator's board: one replaced record per orchestrator session.
//!
//! The orchestrator keeps a short list of what the human must decide, what is
//! coming, what runs, what waits on others and which links matter. The core
//! holds exactly one current record of it, never a history: a set replaces the
//! whole record. This module is the pure part: the input grammar the
//! orchestrator writes, the canonical stored text, the diff and the bounds.
//! Reading and writing the file is `l3_services::deskstore`; showing the record is
//! `console::deskview` and the Telegram pin.
//!
//! INPUT, one item per line, UTF-8 with LF line ends, empty lines ignored:
//!
//! ```text
//! DECIDE
//! D1 | Ship the release now? | https://example.test/pull/12 | /abs/path/note.md
//! RUNNING
//! R2 | Write the draft
//! ```
//!
//! Headings are exactly `DECIDE`, `COMING`, `RUNNING`, `WAITING ON OTHERS`
//! and `LINKS`, each at most once and in that order. An item is `ID | text`
//! with up to three ` | link` parts. The ID is the section letter (`D`, `C`,
//! `R`, `W`, `L`) and a number 1 to 99 without a leading zero. The orchestrator
//! owns the ids: an item that stands keeps its id, so a change is visible as a
//! diff. A link is an `http(s)` URL of printable ASCII with a host, or an
//! absolute path of at least two segments without `.` or `..` segments.
//! Anything else refuses the whole input with the line and the reason: nothing
//! is truncated, repaired or half applied.
//!
//! STORED, the one canonical spelling the core writes and the readers parse:
//!
//! ```text
//! ae-desk 1
//! updated 2026-11-20T08:40:12Z
//! D1<TAB>text<TAB>link<TAB>link
//! ```
//!
//! Items stand in section order, then by ascending number. `updated` is the
//! core's clock at the change, never the input's. `encode(parse(b)) == b` for
//! every `b` that parses: the fuzz target `desk_record` holds that.

use std::collections::BTreeMap;
use std::fmt;

use crate::time::Timestamp;

/// The most bytes a set may carry.
pub const INPUT_CAP: usize = 4096;
/// The most bytes a stored record may hold: the input cap and room for the
/// two header lines. The stored text of an item is never longer than its
/// input line, so an accepted input always fits.
pub const STORED_CAP: usize = INPUT_CAP + 256;
/// The most items a board holds.
pub const MAX_ITEMS: usize = 16;
/// The most characters in an item's text.
pub const MAX_TEXT: usize = 160;
/// The most links an item carries.
pub const MAX_LINKS: usize = 3;
/// The most bytes in one link.
pub const MAX_LINK: usize = 300;
/// The journal action of a board change.
pub const ACTION: &str = "desk";
/// The record's file name in a session directory. The session helper `desk`
/// is a link of that other name, so the record cannot share it.
pub const RECORD_FILE: &str = "desk-record";

#[cfg(test)]
#[path = "desk/input_tests.rs"]
mod input_tests;

const MAGIC: &str = "ae-desk 1";
const UPDATED: &str = "updated ";

const _: () = assert!(MAGIC.len() + 1 + UPDATED.len() + 20 < STORED_CAP - INPUT_CAP);

/// The five parts of a board, in the order they are written and shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Section {
    /// What the human must decide.
    Decide,
    /// What comes next.
    Coming,
    /// What runs now.
    Running,
    /// What waits on others.
    Waiting,
    /// The links that matter.
    Links,
}

impl Section {
    /// Every section, in board order.
    pub const ALL: [Self; 5] = [
        Self::Decide,
        Self::Coming,
        Self::Running,
        Self::Waiting,
        Self::Links,
    ];

    /// The letter its ids start with.
    #[must_use]
    pub const fn letter(self) -> char {
        match self {
            Self::Decide => 'D',
            Self::Coming => 'C',
            Self::Running => 'R',
            Self::Waiting => 'W',
            Self::Links => 'L',
        }
    }

    /// The heading line that opens it in an input and in every display.
    #[must_use]
    pub const fn heading(self) -> &'static str {
        match self {
            Self::Decide => "DECIDE",
            Self::Coming => "COMING",
            Self::Running => "RUNNING",
            Self::Waiting => "WAITING ON OTHERS",
            Self::Links => "LINKS",
        }
    }

    fn of_letter(letter: char) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|section| section.letter() == letter)
    }

    fn of_heading(line: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|section| section.heading() == line)
    }
}

/// An item's id: its section and a number from 1 to 99. Displays as `D1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id {
    /// The section the item stands in.
    pub section: Section,
    /// The number, 1 to 99.
    pub number: u8,
}

impl fmt::Display for Id {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "{}{}", self.section.letter(), self.number)
    }
}

/// One line of the board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The orchestrator's id for it.
    pub id: Id,
    /// What it says, 1 to 160 characters without control characters or `|`.
    pub text: String,
    /// Its links: `http(s)` URLs and absolute paths, at most three.
    pub links: Vec<String>,
}

/// The whole record: when it changed last and its items in canonical order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Desk {
    /// The core's clock when the items last changed.
    pub updated: Timestamp,
    /// Section order, then ascending number.
    pub items: Vec<Item>,
}

/// Why a text is no board: the 1-based line, 0 for the whole text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// The offending line, 1-based, or 0 when the whole text is at fault.
    pub line: usize,
    /// The reason, as a clause.
    pub why: &'static str,
}

impl fmt::Display for ParseError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            out.write_str(self.why)
        } else {
            write!(out, "line {}: {}", self.line, self.why)
        }
    }
}

const fn fail(line: usize, why: &'static str) -> ParseError {
    ParseError { line, why }
}

/// What changed between two item lists, each in id order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Delta {
    /// Ids only in the new list.
    pub added: Vec<Id>,
    /// Ids only in the old list.
    pub removed: Vec<Id>,
    /// Ids in both whose text or links differ.
    pub changed: Vec<Id>,
}

impl Delta {
    /// Whether nothing changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

/// `id` read as `D1`: a section letter and a number 1 to 99 without a
/// leading zero.
pub(crate) fn parse_id(token: &str) -> Option<Id> {
    let mut chars = token.chars();
    let section = Section::of_letter(chars.next()?)?;
    let digits = chars.as_str();
    if digits.is_empty()
        || digits.len() > 2
        || digits.starts_with('0')
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let number: u8 = digits.parse().ok()?;
    Some(Id { section, number })
}

fn check_text(text: &str) -> Result<(), &'static str> {
    if text.is_empty() {
        return Err("the text is empty");
    }
    if text.chars().count() > MAX_TEXT {
        return Err("the text is longer than 160 characters");
    }
    if text.chars().any(char::is_control) {
        return Err("the text holds a control character");
    }
    if text.contains('|') {
        return Err("the text holds a pipe");
    }
    Ok(())
}

fn check_link(link: &str) -> Result<(), &'static str> {
    if link.len() > MAX_LINK {
        return Err("a link is longer than 300 bytes");
    }
    if link.trim() != link {
        return Err("a link has surrounding whitespace");
    }
    if let Some(rest) = link
        .strip_prefix("https://")
        .or_else(|| link.strip_prefix("http://"))
    {
        if !link.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) {
            return Err("a web link holds a character outside printable ASCII");
        }
        let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
        if host.is_empty() {
            return Err("a web link has no host");
        }
        return Ok(());
    }
    if link.starts_with('/') {
        if link.chars().any(char::is_control) {
            return Err("a path holds a control character");
        }
        let mut named = 0_usize;
        for segment in link.split('/').filter(|segment| !segment.is_empty()) {
            if segment == "." || segment == ".." {
                return Err("a path holds a dot segment");
            }
            named += 1;
        }
        if named < 2 {
            return Err("a path needs at least two segments");
        }
        return Ok(());
    }
    Err("a link is neither an http(s) URL nor an absolute path")
}

fn check_links(links: &[String]) -> Result<(), &'static str> {
    if links.len() > MAX_LINKS {
        return Err("an item holds more than three links");
    }
    links.iter().try_for_each(|link| check_link(link))
}

/// An input read as the items it holds, in canonical order.
///
/// # Errors
///
/// The first line that breaks the grammar, with its reason: nothing is
/// partially applied.
pub fn parse_input(bytes: &[u8]) -> Result<Vec<Item>, ParseError> {
    if bytes.len() > INPUT_CAP {
        return Err(fail(0, "the input is longer than 4096 bytes"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| fail(0, "the input is not UTF-8"))?;
    if text.trim().is_empty() {
        return Err(fail(0, "the input is empty"));
    }
    let mut items: Vec<Item> = Vec::new();
    let mut heading: Option<Section> = None;
    let mut opened: Option<Section> = None;
    for (index, line) in text.split('\n').enumerate() {
        let number = index + 1;
        if line.is_empty() {
            continue;
        }
        if let Some(section) = Section::of_heading(line) {
            if opened.is_some_and(|last| section <= last) {
                return Err(fail(number, "a heading is repeated or out of order"));
            }
            opened = Some(section);
            heading = Some(section);
            continue;
        }
        let mut parts = line.split(" | ");
        let Some(id) = parts.next().and_then(parse_id) else {
            return Err(fail(number, "neither a heading nor an item id"));
        };
        let Some(section) = heading else {
            return Err(fail(number, "an item stands before its heading"));
        };
        if id.section != section {
            return Err(fail(number, "the id does not belong to the heading"));
        }
        let Some(text) = parts.next() else {
            return Err(fail(number, "an item needs ` | ` and a text"));
        };
        // A control character is refused where it stands: trimming first
        // would quietly drop a boundary tab or carriage return.
        if text.chars().any(char::is_control) {
            return Err(fail(number, "the text holds a control character"));
        }
        let text = text.trim();
        check_text(text).map_err(|why| fail(number, why))?;
        let links: Vec<String> = parts.map(str::to_owned).collect();
        check_links(&links).map_err(|why| fail(number, why))?;
        if items.iter().any(|item| item.id == id) {
            return Err(fail(number, "the id is used twice"));
        }
        if items.len() == MAX_ITEMS {
            return Err(fail(number, "the board holds more than 16 items"));
        }
        items.push(Item {
            id,
            text: text.to_owned(),
            links,
        });
    }
    items.sort_by_key(|item| item.id);
    Ok(items)
}

fn parse_stored_item(line: &str) -> Result<Item, &'static str> {
    let mut fields = line.split('\t');
    let Some(id) = fields.next().and_then(parse_id) else {
        return Err("not an item id");
    };
    let Some(text) = fields.next() else {
        return Err("an item needs a text");
    };
    check_text(text)?;
    if text.trim() != text {
        return Err("the text has surrounding whitespace");
    }
    let links: Vec<String> = fields.map(str::to_owned).collect();
    check_links(&links)?;
    Ok(Item {
        id,
        text: text.to_owned(),
        links,
    })
}

/// A stored record read back whole.
///
/// # Errors
///
/// The first line that is not the canonical spelling, with its reason.
pub fn parse(bytes: &[u8]) -> Result<Desk, ParseError> {
    if bytes.len() > STORED_CAP {
        return Err(fail(0, "the record is longer than the stored cap"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| fail(0, "the record is not UTF-8"))?;
    let Some(body) = text.strip_suffix('\n') else {
        return Err(fail(0, "the record ends in a torn line"));
    };
    let mut lines = body.split('\n');
    if lines.next() != Some(MAGIC) {
        return Err(fail(1, "not an ae-desk 1 record"));
    }
    let updated = lines
        .next()
        .and_then(|line| line.strip_prefix(UPDATED))
        .and_then(|stamp| Timestamp::parse(stamp).filter(|at| at.to_string() == stamp))
        .ok_or_else(|| fail(2, "the update time is missing or malformed"))?;
    let mut items: Vec<Item> = Vec::new();
    for (index, line) in lines.enumerate() {
        let number = index + 3;
        let item = parse_stored_item(line).map_err(|why| fail(number, why))?;
        if items.last().is_some_and(|last| last.id >= item.id) {
            return Err(fail(number, "the ids are repeated or out of order"));
        }
        if items.len() == MAX_ITEMS {
            return Err(fail(number, "the board holds more than 16 items"));
        }
        items.push(item);
    }
    Ok(Desk { updated, items })
}

/// The canonical stored spelling of `desk`. Its items stand in canonical
/// order, as [`parse_input`] and [`parse`] return them.
#[must_use]
pub fn encode(desk: &Desk) -> Vec<u8> {
    let mut out = format!("{MAGIC}\n{UPDATED}{}\n", desk.updated);
    for item in &desk.items {
        out.push_str(&item.id.to_string());
        out.push('\t');
        out.push_str(&item.text);
        for link in &item.links {
            out.push('\t');
            out.push_str(link);
        }
        out.push('\n');
    }
    out.into_bytes()
}

/// What changed from `old` to `new`, by id.
#[must_use]
pub fn diff(old: &[Item], new: &[Item]) -> Delta {
    let old: BTreeMap<Id, &Item> = old.iter().map(|item| (item.id, item)).collect();
    let new: BTreeMap<Id, &Item> = new.iter().map(|item| (item.id, item)).collect();
    let mut delta = Delta::default();
    for (id, item) in &new {
        match old.get(id) {
            None => delta.added.push(*id),
            Some(before) if before != item => delta.changed.push(*id),
            Some(_) => {}
        }
    }
    delta.removed = old
        .keys()
        .filter(|id| !new.contains_key(id))
        .copied()
        .collect();
    delta
}

/// The ids a change touched, never an item's text, as `+D3 -D1 ~R2`.
#[must_use]
pub fn change_ids(delta: &Delta) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.extend(delta.added.iter().map(|id| format!("+{id}")));
    parts.extend(delta.removed.iter().map(|id| format!("-{id}")));
    parts.extend(delta.changed.iter().map(|id| format!("~{id}")));
    parts.join(" ")
}

/// The audit words of a change: its ids and the item count, as
/// `+D3 -D1 ~R2 (12 items)`.
#[must_use]
pub fn summary(delta: &Delta, total: usize) -> String {
    let noun = if total == 1 { "item" } else { "items" };
    let count = format!("({total} {noun})");
    let ids = change_ids(delta);
    if ids.is_empty() {
        count
    } else {
        format!("{ids} {count}")
    }
}

/// `items` in the INPUT spelling: each section's heading once, then one
/// `<id> | <text> | <link>` line per item, so the text parses back to the same
/// items. No items print the first heading alone. The items stand in canonical order, as [`parse_input`] returns them.
#[must_use]
pub fn render_input(items: &[Item]) -> String {
    if items.is_empty() {
        // An empty board is a set board: one bare heading is legal input, and
        // empty text is not.
        return format!("{}\n", Section::ALL[0].heading());
    }
    let mut out = String::new();
    let mut open: Option<Section> = None;
    for item in items {
        if open != Some(item.id.section) {
            out.push_str(item.id.section.heading());
            out.push('\n');
            open = Some(item.id.section);
        }
        out.push_str(&item.id.to_string());
        out.push_str(" | ");
        out.push_str(&item.text);
        for link in &item.links {
            out.push_str(" | ");
            out.push_str(link);
        }
        out.push('\n');
    }
    out
}
