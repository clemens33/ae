//! The board as rows of text: the one renderer for the app's right column and
//! for the chat's `/board`.
//!
//! The rows are plain text with the links they carry. A link is shown as the
//! destination the orchestrator wrote (a web URL, or an absolute path) and
//! targets the whole destination (the URL as written, or the `file://` URL of
//! the path), also on every fragment of a link that wraps. Every shown field
//! passes the one terminal-output gate, so a control character in a hostile
//! record never reaches a terminal as an escape.

use std::ops::Range;
use std::path::Path;

use crate::board::terminal_text;
use crate::desk::{Desk, Item, Section};
use crate::time::Timestamp;

use super::link::encode_path;
use super::view::Zone;
use super::wrap::split_row;

/// What a row is, which fixes how a surface dresses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// The `BOARD  updated …` line.
    Header,
    /// A section heading.
    Heading,
    /// An item's text, with its id.
    Item,
    /// One link of an item, on its own row.
    Link,
    /// The marker that rows were cut for height.
    More,
    /// A line about the board itself: none is set, or it cannot be shown.
    Notice,
}

/// The part of a row a link covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowLink {
    /// The byte range of the covered text in the row's `text`.
    pub range: Range<usize>,
    /// The whole destination: a web URL as written, or the `file://` URL of
    /// the path. The same on every fragment of a wrapped link.
    pub url: String,
}

/// One row of the board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeskRow {
    /// What the row is.
    pub kind: RowKind,
    /// The row's text, indent included.
    pub text: String,
    /// The links inside the text.
    pub links: Vec<RowLink>,
}

/// The cells an item's link rows are indented by.
const LINK_INDENT: usize = 2;
/// The cells a wrapped header or heading continues by.
const HANG: usize = 2;

/// Where a row break may stand: `width` cells, or none for `u16::MAX`.
fn room(width: u16) -> usize {
    if width == u16::MAX {
        usize::MAX
    } else {
        usize::from(width)
    }
}

/// `text` as rows of `kind` at most `width` cells wide: the first `lead`
/// cells in, every following row `hang` cells in. `url` is what every row's
/// text targets, when the text is a link.
fn push_wrapped(
    out: &mut Vec<DeskRow>,
    kind: RowKind,
    text: &str,
    (lead, hang): (usize, usize),
    width: usize,
    url: Option<&str>,
) {
    let mut rest = text;
    let mut indent = lead;
    loop {
        let indent_now = indent.min(width.saturating_sub(1));
        let (piece, tail) = split_row(rest, width.saturating_sub(indent_now));
        let mut row = " ".repeat(indent_now);
        let start = row.len();
        row.push_str(piece);
        let links = url
            .map(|url| {
                vec![RowLink {
                    range: start..row.len(),
                    url: url.to_owned(),
                }]
            })
            .unwrap_or_default();
        out.push(DeskRow {
            kind,
            text: row,
            links,
        });
        if tail.is_empty() {
            return;
        }
        rest = tail;
        indent = hang;
    }
}

/// `HH:MM:SS` of `at` and the zone's name: the viewer's zone when `zone` is
/// the `[+-]HHMM` answer tmux gave, else UTC.
fn clock(at: Timestamp, zone: Option<&str>) -> String {
    let (shift, label) = zone
        .and_then(Zone::parse)
        .map_or((0, "UTC".to_owned()), |zone| (zone.secs, zone.label));
    let stamp = Timestamp::from_epoch(at.epoch().saturating_add(shift)).to_string();
    let time = stamp.get(11..19).unwrap_or("--:--:--");
    format!("{time} {label}")
}

/// What a link shows, and where it points: a web URL is both; an absolute
/// path shows as itself and points at its `file://` URL.
fn destination(link: &str) -> (String, String) {
    if link.starts_with('/') {
        (terminal_text(link), encode_path(link))
    } else {
        (terminal_text(link), link.to_owned())
    }
}

fn push_item(out: &mut Vec<DeskRow>, item: &Item, width: usize) {
    let id = item.id.to_string();
    let text = format!("{id} {}", terminal_text(&item.text));
    push_wrapped(out, RowKind::Item, &text, (0, id.len() + 1), width, None);
    for link in &item.links {
        let (shown, url) = destination(link);
        push_wrapped(
            out,
            RowKind::Link,
            &shown,
            (LINK_INDENT, LINK_INDENT),
            width,
            Some(&url),
        );
    }
}

/// The board as rows `width` cells wide, `u16::MAX` meaning no wrapping: the
/// header, then each section that holds items with its heading, its items in
/// id order and each item's links on rows of their own. `zone` is the tmux
/// answer for the viewer's clock (`+0200`); `None` or anything else reads UTC.
#[must_use]
pub fn rows(desk: &Desk, width: u16, zone: Option<&str>) -> Vec<DeskRow> {
    let width = room(width);
    let mut out = Vec::new();
    let header = format!("BOARD  updated {}", clock(desk.updated, zone));
    push_wrapped(&mut out, RowKind::Header, &header, (0, HANG), width, None);
    for section in Section::ALL {
        let mut items = desk.items.iter().filter(|item| item.id.section == section);
        let Some(first) = items.next() else {
            continue;
        };
        push_wrapped(
            &mut out,
            RowKind::Heading,
            section.heading(),
            (0, HANG),
            width,
            None,
        );
        push_item(&mut out, first, width);
        for item in items {
            push_item(&mut out, item, width);
        }
    }
    out
}

/// `rows` cut to `height`: the rows that fit, the last one the marker
/// `+N more: /board` naming how many rows went. Nothing is cut when they all
/// fit, and nothing is left at height 0.
#[must_use]
pub fn fit(mut rows: Vec<DeskRow>, height: u16) -> Vec<DeskRow> {
    let height = usize::from(height);
    if rows.len() <= height {
        return rows;
    }
    if height == 0 {
        return Vec::new();
    }
    let keep = height - 1;
    let omitted = rows.len() - keep;
    rows.truncate(keep);
    rows.push(DeskRow {
        kind: RowKind::More,
        text: format!("+{omitted} more: /board"),
        links: Vec::new(),
    });
    rows
}

/// `text` as notice rows `width` cells wide.
fn notice(text: &str, width: u16) -> Vec<DeskRow> {
    let mut out = Vec::new();
    push_wrapped(
        &mut out,
        RowKind::Notice,
        text,
        (0, HANG),
        room(width),
        None,
    );
    out
}

/// What the app's board column shows for `view`, `width` cells wide: the rows
/// of the board, or a header with the one line that says there is none or why
/// it cannot be shown. `None` is a record not read yet: no rows.
#[must_use]
pub fn column(view: Option<&DeskView>, width: u16, zone: Option<&str>) -> Vec<DeskRow> {
    match view {
        None => Vec::new(),
        Some(DeskView::Shown(desk)) => rows(desk, width, zone),
        Some(DeskView::Absent) => {
            let mut out = notice("BOARD", width);
            out.extend(notice("no board set", width));
            out
        }
        Some(DeskView::Unreadable(why)) => notice(&format!("desk unreadable: {why}"), width),
    }
}

/// The word that stands in the panel header while the board column does not:
/// `desk N` for the N items in DECIDE of a board that is set (also `desk 0`),
/// or `desk unreadable`. `None` when no board is set.
#[must_use]
pub fn cue(view: &DeskView) -> Option<String> {
    match view {
        DeskView::Absent => None,
        DeskView::Unreadable(_) => Some("desk unreadable".to_owned()),
        DeskView::Shown(desk) => {
            let waiting = desk
                .items
                .iter()
                .filter(|item| item.id.section == Section::Decide)
                .count();
            Some(format!("desk {waiting}"))
        }
    }
}

/// The word that stands in the panel header while the human hid the board:
/// [`cue`], except that a board with nothing under DECIDE says nothing, so a
/// word there always means a decision waits, or that the record is unreadable.
#[must_use]
pub fn hidden_cue(view: &DeskView) -> Option<String> {
    match view {
        DeskView::Shown(desk)
            if !desk
                .items
                .iter()
                .any(|item| item.id.section == Section::Decide) =>
        {
            None
        }
        _ => cue(view),
    }
}

/// What reading the board of a session found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeskView {
    /// No record is set.
    Absent,
    /// A record stands that cannot be shown, and why.
    Unreadable(String),
    /// The record.
    Shown(Desk),
}

impl DeskView {
    /// The record in the session directory `dir`, read now.
    #[must_use]
    pub fn read(dir: &Path) -> Self {
        match crate::deskstore::read(dir) {
            Ok(None) => Self::Absent,
            Ok(Some(desk)) => Self::Shown(desk),
            Err(why) => Self::Unreadable(terminal_text(&why.to_string())),
        }
    }
}

/// The lines `/board` prints for `view`, each ending in a line break: the
/// whole board unwrapped, or the one line that says there is none or why it
/// cannot be shown. `zone` is the tmux answer for the viewer's clock.
#[must_use]
pub fn board_text(view: &DeskView, zone: Option<&str>) -> String {
    match view {
        DeskView::Absent => "no board set\n".to_owned(),
        DeskView::Unreadable(why) => format!("board unreadable: {why}\n"),
        DeskView::Shown(desk) => {
            rows(desk, u16::MAX, zone)
                .iter()
                .fold(String::new(), |mut text, row| {
                    text.push_str(&row.text);
                    text.push('\n');
                    text
                })
        }
    }
}
