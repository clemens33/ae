//! Text selection in `ae app`: which drawn cells a drag covers, the text those
//! cells spell, and the overlay that shows them.
//!
//! PURE. It reads a frame's cells and writes only a style modifier; no escape
//! leaves this module, no I/O. The frame records WHERE a drag may start
//! ([`Map`]); this module turns two cells inside one such region into rows.

use std::hash::{DefaultHasher, Hash as _, Hasher as _};

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::{Position, Rect};
use ratatui_core::style::Modifier;
use ratatui_core::text::Span;

use super::backend;

/// The panels a drag can select in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    /// The chat's turns, between its header rule and the composer.
    Lane,
    /// The peek panel's body.
    Peek,
    /// The sidebar's session list.
    List,
    /// The sidebar's tab body.
    Body,
}

/// One panel's selectable cells in a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub panel: Panel,
    pub area: Rect,
}

/// What a frame lets a drag select: its regions, and the chrome cells inside
/// them (a marked turn's bars) that read as blanks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Map {
    pub regions: Vec<Region>,
    pub blanks: Vec<Rect>,
}

impl Map {
    /// The region holding `at`.
    #[must_use]
    pub fn region_at(&self, at: Position) -> Option<&Region> {
        self.regions.iter().find(|region| region.area.contains(at))
    }

    /// The region of `panel`.
    pub(crate) fn region(&self, panel: Panel) -> Option<&Region> {
        self.regions.iter().find(|region| region.panel == panel)
    }
}

/// A drag inside one region: where it began, where it is, and the fingerprint
/// of the region when it began.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    panel: Panel,
    area: Rect,
    fp: u64,
    anchor: Position,
    head: Position,
    moved: bool,
}

impl Selection {
    /// A selection that has not moved, anchored at `at` clamped into `region`.
    #[must_use]
    pub fn start(region: &Region, at: Position, fp: u64) -> Self {
        let at = clamp(region.area, at);
        Self {
            panel: region.panel,
            area: region.area,
            fp,
            anchor: at,
            head: at,
            moved: false,
        }
    }

    /// Move the head to `at`: a cell outside the region pins to its edge, above
    /// to its first cell, below to the end of its last row. Whether the head
    /// changed. `moved` latches on the first head other than the anchor.
    pub fn extend(&mut self, at: Position) -> bool {
        let head = reach(self.area, at);
        self.moved |= head != self.anchor;
        std::mem::replace(&mut self.head, head) != head
    }

    #[must_use]
    pub fn panel(&self) -> Panel {
        self.panel
    }

    #[must_use]
    pub fn area(&self) -> Rect {
        self.area
    }

    #[must_use]
    pub fn anchor(&self) -> Position {
        self.anchor
    }

    #[must_use]
    pub fn head(&self) -> Position {
        self.head
    }

    #[must_use]
    pub fn moved(&self) -> bool {
        self.moved
    }

    #[must_use]
    pub fn fp(&self) -> u64 {
        self.fp
    }

    /// Each selected row as `(y, first column, last column)`, reading order.
    fn rows(&self) -> Vec<(u16, u16, u16)> {
        let (from, to) = if (self.anchor.y, self.anchor.x) <= (self.head.y, self.head.x) {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        };
        let last = self.area.right().saturating_sub(1);
        (from.y..=to.y)
            .map(|y| {
                let first = if y == from.y { from.x } else { self.area.x };
                let end = if y == to.y { to.x } else { last };
                (y, first, end)
            })
            .collect()
    }
}

/// `at` inside `area`, cell by cell.
fn clamp(area: Rect, at: Position) -> Position {
    Position::new(
        at.x.clamp(area.x, area.right().saturating_sub(1).max(area.x)),
        at.y.clamp(area.y, area.bottom().saturating_sub(1).max(area.y)),
    )
}

/// Where a head at `at` lands: inside the region as it is, above it on the
/// first cell, below it at the end of the last row.
fn reach(area: Rect, at: Position) -> Position {
    let last = Position::new(
        area.right().saturating_sub(1).max(area.x),
        area.bottom().saturating_sub(1).max(area.y),
    );
    if at.y < area.y {
        area.as_position()
    } else if at.y > last.y {
        last
    } else {
        clamp(area, at)
    }
}

/// One drawn symbol and the cells it covers.
struct Unit {
    x: u16,
    width: u16,
    symbol: String,
    masked: bool,
}

/// The symbols of row `y` inside `area`, left to right. A wide symbol is one
/// unit over all its cells (ratatui leaves the cells after it as blanks); a
/// masked cell reads as a blank and a control symbol is a blank, as the
/// backend would write it.
fn units(buf: &Buffer, area: Rect, y: u16, blanks: &[Rect]) -> Vec<Unit> {
    let mut row = Vec::new();
    let mut x = area.x;
    while x < area.right() {
        let masked = blanks
            .iter()
            .any(|blank| blank.contains(Position::new(x, y)));
        let symbol = match buf.cell((x, y)) {
            Some(cell) if !masked => backend::clean(cell.symbol()).to_owned(),
            _ => " ".to_owned(),
        };
        let width = u16::try_from(Span::raw(symbol.as_str()).width())
            .unwrap_or(1)
            .max(1);
        row.push(Unit {
            x,
            width,
            symbol,
            masked,
        });
        x = x.saturating_add(width);
    }
    row
}

/// Whether `unit` covers any column of `first..=last`.
fn covers(unit: &Unit, first: u16, last: u16) -> bool {
    unit.x <= last && unit.x.saturating_add(unit.width) > first
}

/// A fingerprint of the symbols in `area` (clipped to `buf`): styles do not
/// count and masked cells read as blanks, so a mark or a highlight alone never
/// changes it, and a moved or changed row always does.
#[must_use]
pub fn fingerprint(buf: &Buffer, area: Rect, blanks: &[Rect]) -> u64 {
    let area = area.intersection(buf.area);
    let mut hasher = DefaultHasher::new();
    (area.x, area.y, area.width, area.height).hash(&mut hasher);
    for y in area.y..area.bottom() {
        for unit in units(buf, area, y, blanks) {
            unit.symbol.hash(&mut hasher);
        }
        '\n'.hash(&mut hasher);
    }
    hasher.finish()
}

/// What the selected cells spell: each row from its first selected symbol to
/// its last, trailing blanks trimmed, rows joined by a newline.
#[must_use]
pub fn text(buf: &Buffer, selection: &Selection, blanks: &[Rect]) -> String {
    let area = selection.area.intersection(buf.area);
    let rows: Vec<String> = (selection.rows().into_iter())
        .map(|(y, first, last)| {
            let spelled: String = (units(buf, area, y, blanks).iter())
                .filter(|unit| covers(unit, first, last))
                .map(|unit| unit.symbol.as_str())
                .collect();
            spelled.trim_end().to_owned()
        })
        .collect();
    rows.join("\n")
}

/// Show the selection: every selected cell up to each row's last ink has its
/// reversed-video flipped (so already reversed text shows as its inverse), a
/// wide symbol on all its cells and a masked cell not at all.
pub fn highlight(buf: &mut Buffer, selection: &Selection, blanks: &[Rect]) {
    let area = selection.area.intersection(buf.area);
    for (y, first, last) in selection.rows() {
        let row = units(buf, area, y, blanks);
        let inside: Vec<&Unit> = (row.iter())
            .filter(|unit| covers(unit, first, last))
            .collect();
        let ink = inside
            .iter()
            .rposition(|unit| !unit.symbol.trim().is_empty());
        let Some(ink) = ink else {
            continue;
        };
        for unit in inside.iter().take(ink + 1).filter(|unit| !unit.masked) {
            for x in unit.x..unit.x.saturating_add(unit.width) {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.modifier.toggle(Modifier::REVERSED);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Oracles: a terminal's own selection (stream order, trailing blanks
    //! cut, a wide character taken whole) and the brief's R1-R3.

    use ratatui_core::buffer::Buffer;
    use ratatui_core::layout::{Position, Rect};
    use ratatui_core::style::{Modifier, Style};

    use super::{Map, Panel, Region, Selection, fingerprint, highlight, text};

    fn drawn(rows: &[&str]) -> Buffer {
        let width = rows
            .iter()
            .map(|row| row.chars().count())
            .max()
            .unwrap_or(1)
            * 2;
        let high = u16::try_from(rows.len()).expect("small");
        let mut buf = Buffer::empty(Rect::new(0, 0, u16::try_from(width).expect("small"), high));
        for (y, row) in (0..).zip(rows) {
            buf.set_string(0, y, row, Style::new());
        }
        buf
    }

    fn pick(buf: &Buffer, from: (u16, u16), to: (u16, u16)) -> Selection {
        let region = Region {
            panel: Panel::Lane,
            area: buf.area,
        };
        let mut sel = Selection::start(&region, Position::new(from.0, from.1), 0);
        sel.extend(Position::new(to.0, to.1));
        sel
    }

    fn reversed(buf: &Buffer, y: u16) -> Vec<u16> {
        (0..buf.area.width)
            .filter(|x| buf[(*x, y)].modifier.contains(Modifier::REVERSED))
            .collect()
    }

    #[test]
    fn rows_read_in_order_each_cut_at_its_last_ink() {
        let buf = drawn(&["alpha beta", "gamma   ", "delta"]);
        let forward = pick(&buf, (6, 0), (2, 2));
        assert_eq!(text(&buf, &forward, &[]), "beta\ngamma\ndel");
        let backward = pick(&buf, (2, 2), (6, 0));
        assert_eq!(text(&buf, &backward, &[]), "beta\ngamma\ndel", "either way");
        assert_eq!(text(&buf, &pick(&buf, (1, 0), (3, 0)), &[]), "lph");
        assert_eq!(text(&buf, &pick(&buf, (3, 0), (1, 0)), &[]), "lph");
    }

    #[test]
    fn blank_rows_stay_and_the_text_ends_without_a_newline() {
        let buf = drawn(&["a", "", "b  "]);
        assert_eq!(text(&buf, &pick(&buf, (0, 0), (5, 2)), &[]), "a\n\nb");
        assert_eq!(text(&buf, &pick(&buf, (0, 1), (3, 1)), &[]), "");
    }

    #[test]
    fn a_wide_symbol_is_taken_whole_from_either_of_its_cells() {
        let buf = drawn(&["你好x"]);
        for (from, to, want) in [
            (0, 0, "你"),
            (1, 1, "你"),
            (1, 2, "你好"),
            (3, 4, "好x"),
            (4, 4, "x"),
        ] {
            let sel = pick(&buf, (from, 0), (to, 0));
            assert_eq!(text(&buf, &sel, &[]), want, "{from}..={to}");
        }
    }

    #[test]
    fn a_masked_cell_and_a_control_symbol_read_as_blanks() {
        let mut buf = drawn(&["|ab", "a?b"]);
        buf[(1, 1)].set_symbol("\u{7}");
        let mask = [Rect::new(0, 0, 1, 1)];
        assert_eq!(text(&buf, &pick(&buf, (0, 0), (2, 0)), &mask), " ab");
        assert_eq!(text(&buf, &pick(&buf, (0, 1), (2, 1)), &mask), "a b");
        assert_eq!(text(&buf, &pick(&buf, (0, 0), (2, 0)), &[]), "|ab");
    }

    #[test]
    fn a_head_outside_the_region_pins_to_its_edge() {
        let region = Region {
            panel: Panel::List,
            area: Rect::new(2, 2, 4, 3),
        };
        let mut sel = Selection::start(&region, Position::new(3, 3), 7);
        assert_eq!(
            (sel.anchor(), sel.moved(), sel.fp()),
            (Position::new(3, 3), false, 7)
        );
        for (to, head) in [
            ((0, 9), (5, 4)),
            ((9, 9), (5, 4)),
            ((9, 0), (2, 2)),
            ((0, 3), (2, 3)),
            ((9, 3), (5, 3)),
            ((4, 4), (4, 4)),
        ] {
            sel.extend(Position::new(to.0, to.1));
            assert_eq!(sel.head(), Position::new(head.0, head.1), "{to:?}");
        }
        let outside = Selection::start(&region, Position::new(0, 0), 0);
        assert_eq!(outside.anchor(), Position::new(2, 2), "a press clamps in");
        assert_eq!((sel.panel(), sel.area()), (Panel::List, region.area));
    }

    #[test]
    fn moving_latches_on_the_first_head_past_the_anchor() {
        let region = Region {
            panel: Panel::Peek,
            area: Rect::new(0, 0, 9, 3),
        };
        let mut sel = Selection::start(&region, Position::new(4, 1), 0);
        assert!(
            !sel.extend(Position::new(4, 1)),
            "the same cell is no change"
        );
        assert!(!sel.moved());
        assert!(sel.extend(Position::new(5, 1)));
        assert!(sel.moved());
        assert!(
            sel.extend(Position::new(4, 1)),
            "back on the anchor is a change"
        );
        assert!(sel.moved(), "and it stays a drag");
    }

    #[test]
    fn the_overlay_flips_reversed_on_the_copied_cells_only() {
        let mut buf = drawn(&["ab  cd  ", "        ", "ef"]);
        buf[(0, 0)].modifier.insert(Modifier::REVERSED);
        let sel = pick(&buf, (0, 0), (7, 2));
        highlight(&mut buf, &sel, &[]);
        assert_eq!(
            reversed(&buf, 0),
            vec![1, 2, 3, 4, 5],
            "a reversed cell shows inverse; the tail stays"
        );
        assert_eq!(
            reversed(&buf, 1),
            Vec::<u16>::new(),
            "a blank row has no ink"
        );
        assert_eq!(reversed(&buf, 2), vec![0, 1]);
        highlight(&mut buf, &sel, &[]);
        assert_eq!(reversed(&buf, 0), vec![0], "twice is the original");
    }

    #[test]
    fn the_overlay_skips_a_masked_cell_and_covers_a_wide_symbols_cells() {
        let mut buf = drawn(&["|你x"]);
        let sel = pick(&buf, (0, 0), (5, 0));
        highlight(&mut buf, &sel, &[Rect::new(0, 0, 1, 1)]);
        assert_eq!(
            reversed(&buf, 0),
            vec![1, 2, 3],
            "no bar; both cells of 你; x"
        );
    }

    #[test]
    fn a_fingerprint_sees_symbols_and_the_area_and_nothing_else() {
        let mut buf = drawn(&["|ab", "cd"]);
        let area = Rect::new(0, 0, 3, 2);
        let mask = [Rect::new(0, 0, 1, 1)];
        let base = fingerprint(&buf, area, &mask);
        buf[(1, 0)].modifier.insert(Modifier::BOLD);
        assert_eq!(fingerprint(&buf, area, &mask), base, "a style is not text");
        let plain = drawn(&[" ab", "cd"]);
        assert_eq!(
            fingerprint(&plain, area, &mask),
            base,
            "a masked bar is a blank"
        );
        assert_ne!(
            fingerprint(&buf, area, &[]),
            base,
            "unmasked it is a symbol"
        );
        buf[(2, 1)].set_symbol("x");
        assert_ne!(fingerprint(&buf, area, &mask), base, "a changed symbol");
        let shifted = drawn(&["|ab", " cd"]);
        assert_ne!(fingerprint(&shifted, area, &mask), base, "a moved row");
        assert_ne!(
            fingerprint(&plain, Rect::new(0, 0, 3, 1), &mask),
            base,
            "another area"
        );
    }

    #[test]
    fn a_fingerprint_ignores_the_cells_just_outside_its_area() {
        let mut buf = drawn(&["abcd", "efgh"]);
        let area = Rect::new(0, 0, 2, 2);
        let base = fingerprint(&buf, area, &[]);
        buf[(2, 0)].set_symbol("Z");
        buf[(2, 1)].set_symbol("Y");
        assert_eq!(fingerprint(&buf, area, &[]), base, "beside the area");
        buf[(1, 1)].set_symbol("X");
        assert_ne!(fingerprint(&buf, area, &[]), base, "inside the area");
    }

    #[test]
    fn a_map_names_the_region_under_a_cell() {
        let map = Map {
            regions: vec![
                Region {
                    panel: Panel::List,
                    area: Rect::new(0, 0, 4, 4),
                },
                Region {
                    panel: Panel::Body,
                    area: Rect::new(0, 4, 4, 4),
                },
            ],
            blanks: Vec::new(),
        };
        assert_eq!(
            map.region_at(Position::new(1, 5)).map(|r| r.panel),
            Some(Panel::Body)
        );
        assert_eq!(map.region_at(Position::new(4, 1)), None);
        assert_eq!(map.region(Panel::List).map(|r| r.area.height), Some(4));
        assert_eq!(map.region(Panel::Lane), None);
    }
}
