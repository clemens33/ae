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
    let mut out = head(desk, width, zone);
    let width = room(width);
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

/// The `BOARD  updated …` line as rows `width` cells wide: the one head both
/// the flat board and the app's tabbed column carry.
#[must_use]
pub fn head(desk: &Desk, width: u16, zone: Option<&str>) -> Vec<DeskRow> {
    let mut out = Vec::new();
    let header = format!("BOARD  updated {}", clock(desk.updated, zone));
    push_wrapped(
        &mut out,
        RowKind::Header,
        &header,
        (0, HANG),
        room(width),
        None,
    );
    out
}

/// The items of `section` as rows `width` cells wide, in id order with each
/// item's links under it and no heading: the tab names the section. An empty
/// section is the one notice row `no items`.
#[must_use]
pub fn body(desk: &Desk, section: Section, width: u16) -> Vec<DeskRow> {
    let width = room(width);
    let mut out = Vec::new();
    for item in desk.items.iter().filter(|item| item.id.section == section) {
        push_item(&mut out, item, width);
    }
    if out.is_empty() {
        push_wrapped(
            &mut out,
            RowKind::Notice,
            "no items",
            (0, HANG),
            width,
            None,
        );
    }
    out
}

/// How much of a section's name its tab shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Form {
    /// `DECIDE 3`.
    Full,
    /// `DEC 3`.
    Short,
    /// `D3`: the letter its item ids start with.
    Letter,
}

/// What stands between two tabs, the wider first: a bar takes the first gap
/// that lets its widest form fit.
const GAPS: [&str; 2] = [" · ", "·"];

/// The items `desk` holds under `section`.
#[must_use]
pub fn count(desk: &Desk, section: Section) -> usize {
    desk.items
        .iter()
        .filter(|item| item.id.section == section)
        .count()
}

fn tab_text(section: Section, count: usize, form: Form) -> String {
    let word = match (form, section) {
        (Form::Letter, _) => return format!("{}{count}", section.letter()),
        (Form::Full, Section::Decide) => "DECIDE",
        (Form::Full, Section::Coming) => "COMING",
        (Form::Full, Section::Running) => "RUNNING",
        (Form::Full, Section::Waiting) => "WAITING",
        (Form::Full, Section::Links) => "LINKS",
        (Form::Short, Section::Decide) => "DEC",
        (Form::Short, Section::Coming) => "COM",
        (Form::Short, Section::Running) => "RUN",
        (Form::Short, Section::Waiting) => "WAIT",
        (Form::Short, Section::Links) => "LINK",
    };
    format!("{word} {count}")
}

/// One tab of the bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    /// The section it shows.
    pub section: Section,
    /// Its label, a section word and the item count.
    pub text: String,
    /// The cells from the bar's left edge to the label's first cell.
    pub offset: u16,
}

/// The one-row tab bar: a tab per section in board order, and what stands
/// between two of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabBar {
    pub tabs: Vec<Tab>,
    pub gap: &'static str,
}

impl TabBar {
    /// The cells the bar spans, its last label's end.
    #[must_use]
    pub fn width(&self) -> usize {
        self.tabs.last().map_or(0, |tab| {
            usize::from(tab.offset).saturating_add(tab.text.chars().count())
        })
    }

    fn of(desk: &Desk, form: Form, gap: &'static str) -> Self {
        let mut at = 0usize;
        let tabs = Section::ALL
            .map(|section| {
                let text = tab_text(section, count(desk, section), form);
                let offset = u16::try_from(at).unwrap_or(u16::MAX);
                at += text.chars().count() + gap.chars().count();
                Tab {
                    section,
                    text,
                    offset,
                }
            })
            .to_vec();
        Self { tabs, gap }
    }
}

/// The tab bar for `desk` in `room` cells, one row: the widest of the full,
/// short and letter forms that fits, with the wide gap before the tight one.
/// A bar that fits nowhere is the letter form with the tight gap, which the
/// caller clips; the app's narrowest board has room for it.
#[must_use]
pub fn tab_bar(desk: &Desk, room: u16) -> TabBar {
    [Form::Full, Form::Short, Form::Letter]
        .into_iter()
        .flat_map(|form| GAPS.map(|gap| TabBar::of(desk, form, gap)))
        .find(|bar| bar.width() <= usize::from(room))
        .unwrap_or_else(|| TabBar::of(desk, Form::Letter, GAPS[1]))
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

/// The live part as `/board` prints it, each line ending in a line break:
/// `LIVE  as of <clock>`, then a row per session, seat and open ask to the
/// human, with ages. `zone` is the tmux answer for the viewer's clock.
#[must_use]
pub fn live_text(live: &crate::desklive::Live, zone: Option<&str>) -> String {
    let mut text = crate::desklive::header(live, &clock(live.as_of, zone));
    text.push('\n');
    for row in crate::desklive::rows(live, crate::desklive::Elapsed::Age) {
        text.push_str(&row);
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;
    use crate::desk::parse_input;

    fn desk(raw: &str) -> Desk {
        Desk {
            updated: Timestamp::from_epoch(0),
            items: parse_input(raw.as_bytes()).expect("a valid board"),
        }
    }

    /// Counts 2, 1, 1, 0, 1.
    const SPREAD: &str = "DECIDE\nD1 | a\nD2 | b\nCOMING\nC1 | c\nRUNNING\nR1 | d\nLINKS\nL1 | e\n";

    /// The widest form that fits wins, the wide gap before the tight one. The
    /// widths of the five forms for the `SPREAD` counts are 53 and 45 (full),
    /// 39 and 31 (short), 22 and 14 (letter).
    #[test]
    fn the_bar_takes_the_widest_form_and_gap_that_fit() {
        let desk = desk(SPREAD);
        for (room, first, gap, width) in [
            (200, "DECIDE 2", " · ", 53),
            (53, "DECIDE 2", " · ", 53),
            (52, "DECIDE 2", "·", 45),
            (45, "DECIDE 2", "·", 45),
            (44, "DEC 2", " · ", 39),
            (39, "DEC 2", " · ", 39),
            (38, "DEC 2", "·", 31),
            (34, "DEC 2", "·", 31),
            (31, "DEC 2", "·", 31),
            (30, "D2", " · ", 22),
            (22, "D2", " · ", 22),
            (21, "D2", "·", 14),
            (14, "D2", "·", 14),
            (13, "D2", "·", 14),
            (0, "D2", "·", 14),
        ] {
            let bar = tab_bar(&desk, room);
            assert_eq!(bar.tabs[0].text, first, "room {room}");
            assert_eq!(bar.gap, gap, "room {room}");
            assert_eq!(bar.width(), width, "room {room}");
        }
    }

    #[test]
    fn each_form_names_the_five_sections_in_board_order_with_their_counts() {
        let desk = desk(SPREAD);
        let names = |room| -> Vec<String> {
            let bar = tab_bar(&desk, room);
            assert_eq!(
                bar.tabs.iter().map(|tab| tab.section).collect::<Vec<_>>(),
                Section::ALL
            );
            bar.tabs.into_iter().map(|tab| tab.text).collect()
        };
        assert_eq!(
            names(60),
            ["DECIDE 2", "COMING 1", "RUNNING 1", "WAITING 0", "LINKS 1"]
        );
        assert_eq!(names(34), ["DEC 2", "COM 1", "RUN 1", "WAIT 0", "LINK 1"]);
        assert_eq!(names(22), ["D2", "C1", "R1", "W0", "L1"]);
    }

    #[test]
    fn a_label_stands_after_the_labels_and_gaps_before_it() {
        let bar = tab_bar(&desk(SPREAD), 34);
        let at: Vec<u16> = bar.tabs.iter().map(|tab| tab.offset).collect();
        assert_eq!(at, [0, 6, 12, 18, 25]);
        let bar = tab_bar(&desk(SPREAD), 22);
        let at: Vec<u16> = bar.tabs.iter().map(|tab| tab.offset).collect();
        assert_eq!(at, [0, 5, 10, 15, 20]);
    }

    /// Ten decisions make a two-digit count: at the narrowest board's room of
    /// 22 cells the spaced letter form needs 23, so the tight one stands.
    #[test]
    fn a_two_digit_count_drops_the_narrowest_board_to_the_tight_letters() {
        let mut raw = String::from("DECIDE\n");
        for n in 1..=10 {
            let _ = writeln!(raw, "D{n} | item");
        }
        raw.push_str("COMING\nC1 | next\n");
        let desk = desk(&raw);
        let bar = tab_bar(&desk, 22);
        let texts: Vec<&str> = bar.tabs.iter().map(|tab| tab.text.as_str()).collect();
        assert_eq!(texts, ["D10", "C1", "R0", "W0", "L0"]);
        assert_eq!((bar.gap, bar.width()), ("·", 15));
        assert_eq!(count(&desk, Section::Decide), 10);
        assert_eq!(count(&desk, Section::Links), 0);
    }

    #[test]
    fn the_body_holds_only_the_items_of_its_section_without_a_heading() {
        let desk = desk(SPREAD);
        let rows = body(&desk, Section::Decide, 34);
        let texts: Vec<&str> = rows.iter().map(|row| row.text.as_str()).collect();
        assert_eq!(texts, ["D1 a", "D2 b"]);
        assert!(rows.iter().all(|row| row.kind == RowKind::Item));
        let rows = body(&desk, Section::Coming, 34);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].text, "C1 c");
    }

    #[test]
    fn an_empty_section_is_one_notice_row() {
        let rows = body(&desk(SPREAD), Section::Waiting, 34);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].kind, rows[0].text.as_str()),
            (RowKind::Notice, "no items")
        );
    }

    #[test]
    fn the_head_is_the_header_the_flat_board_starts_with() {
        let desk = desk(SPREAD);
        for width in [14, 34, u16::MAX] {
            let head = head(&desk, width, None);
            assert!(head.iter().all(|row| row.kind == RowKind::Header));
            assert_eq!(head.as_slice(), &rows(&desk, width, None)[..head.len()]);
        }
        assert_eq!(
            head(&desk, u16::MAX, None)[0].text,
            "BOARD  updated 00:00:00 UTC"
        );
    }
}
