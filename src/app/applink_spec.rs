//! Frozen acceptance cases: applink L1, app side — the lane's links, the
//! backend's OSC 8, app.rs's draw flow and the production base seam. Independent
//! navigator applink-nav (Z.AI GLM 5.3 Flash), frozen against driver applink's
//! plan r2 and lead's spec-r1 review. Driver installs byte-identically, wired
//! from src/app.rs via `#[cfg(test)] #[path = "app/applink_spec.rs"] mod
//! applink_spec;`. The scanner and the chat live beside their owners in
//! `src/console/applink_chat_spec.rs`.
//!
//! Level 1 NOW: a web URL or a repo-style path in a lane body becomes a real
//! terminal hyperlink (OSC 8) in `ae app`'s chat column, so a terminal's
//! Cmd/Ctrl-click opens it. The app lane wraps with `link::wrapped`
//! (`Show.base`), carries the links next to its rows (`Tail::links`: one
//! `(cell offset from the row's left, cell width, url)` per link per row), and
//! draw.rs turns them into `Layout::links: Vec<Run>` (one run per link per
//! row, clipped to the room). `app::backend::Ansi` — the ONE writer of
//! escapes — wraps exactly those cells: the open after the run's first cell's
//! CUP and SGR, before its symbol; the close before any cell outside the run,
//! before any CUP jump and at the frame end. app.rs's `draw_frame` runs the
//! frame and hands the runs over; it is byte-equivalent to `Terminal::draw`
//! when no runs are given.
//!
//! Bytes: an OSC 8 open is `ESC ] 8 ; id=<n> ; <url> ESC \` (ST terminates),
//! an OSC 8 close is exactly `ESC ] 8 ; ; ESC \`. `<n>` is the 1-based ordinal
//! of the distinct URL by first run, row-major, within one frame: the two rows
//! of a URL split by a wrap share the id, different URLs get different ids.
//! Every emitted URL passes `link::is_safe_url`: printable ASCII only, no
//! control, no whitespace, no non-ASCII — the emitters' last gate.
//!
//! Pinned universally: the visible text is unchanged (OSC 8 is zero-width and
//! rides outside the wrap measure), wrapping and fold counts do not move, and
//! no body — however hostile — can put a control byte into an emitted URL: a
//! control ends every token and permits no new one, and the URL
//! percent-encodes what it may not carry raw.
//!
//! The app's base is filled in production, not by hand: `overview::of` builds
//! `Overview.base` from the selected entry's recorded work dir and the
//! caller's home, `Show.base` reads it from the screen's overview, and the
//! draw records runs from what the lane wrapped with it. A cell is forced into
//! the diff when the URL the frame gives it differs from the previous frame's
//! (none included) — so a link that moves or is retargeted is re-written, and
//! one that is dropped is re-written unlinked.
//!
//! Named residuals (accepted here, not closed): no filesystem check at draw
//! time — a shape-qualified path links whether or not it exists, and a click
//! opens whatever the text names (the terminal decides); `..` is kept,
//! un-normalised; tmux forwards a link only when the user's own
//! `terminal-features` knows their terminal supports hyperlinks — ae never
//! writes that server option, and a tmux that strips the sequence leaves the
//! visible text unchanged. Out of scope for r1: the Settings About tab's links
//! stay plain text; the `ae doctor` hyperlinks row is a follow-up.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use ratatui_core::backend::Backend as _;
use ratatui_core::buffer::{Buffer, Cell};
use ratatui_core::layout::{Position, Rect};
use ratatui_core::terminal::Terminal;

use super::backend::{Ansi, Run};
use super::draw::{Composer, Paint, Screen, Wait};
use super::lane::{Show, tail_with};
use crate::app::fleet::Fleet;
use crate::app::model::Model;
use crate::console::lane::{Item, Kind, Lane};
use crate::console::link::Base;
use crate::console::view::Style;
use crate::digest::{SessionEntry, Status};
use crate::theme::Look;
use crate::time::Timestamp;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn said(body: &str) -> Item {
    Item {
        micros: 1_759_500_000_000_000,
        kind: Kind::Said {
            who: "lead".to_owned(),
        },
        body: body.to_owned(),
        record: None,
    }
}

fn lane_of(body: &str) -> Lane {
    Lane {
        items: vec![said(body)],
        coverage: vec![],
    }
}

fn base(dir: &str) -> Base {
    Base {
        dir: Some(dir.to_owned()),
        home: Some("/h".to_owned()),
    }
}

/// `output` without the escapes ae writes on purpose: every SGR and every OSC
/// 8 pair is removed, so what remains must be plain visible text.
fn stripped(output: &str) -> String {
    let mut out = String::new();
    let mut chars = output.chars();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            Some(']') => loop {
                match chars.next() {
                    Some('\x07') | None => break,
                    Some('\x1b') => {
                        if chars.next() == Some('\\') {
                            break;
                        }
                    }
                    Some(_) => {}
                }
            },
            _ => out.push('\u{fffd}'),
        }
    }
    out
}

/// An expectation pair as the lane answers it: owned strings.
fn pair(visible: &str, url: &str) -> (String, String) {
    (visible.to_owned(), url.to_owned())
}

/// The cells of `text` from cell `start`, `width` cells long: the slice a
/// row-link's cell offset and width name, measured as the pane draws it
/// (ratatui width, not the `event_text::cell` upper bound).
fn cells_slice(text: &str, start: usize, width: usize) -> String {
    let mut out = String::new();
    let mut cells = 0usize;
    for ch in text.chars() {
        let wide = ratatui_core::text::Span::raw(ch.to_string()).width();
        if cells >= start && cells + wide <= start + width {
            out.push(ch);
        }
        cells += wide;
    }
    out
}

// ---------------------------------------------------------------------------
// the production base seam: overview::of fills the base, the lane reads it
// ---------------------------------------------------------------------------

#[test]
fn overview_fills_the_link_base_from_the_entry_and_the_caller_home() {
    let now = Timestamp::from_epoch(1_759_500_600);
    let mut entry = SessionEntry::new("s1", Status::Running);
    entry.work_dir = Some("/w".to_owned());
    let overview = crate::app::overview::of(&entry, None, Some(Path::new("/h")), now);
    assert_eq!(
        overview.base,
        Base {
            dir: Some("/w".to_owned()),
            home: Some("/h".to_owned()),
        },
    );
    let bare = SessionEntry::new("s1", Status::Running);
    let none = crate::app::overview::of(&bare, None, Some(Path::new("/h")), now);
    assert_eq!(
        none.base,
        Base {
            dir: None,
            home: Some("/h".to_owned()),
        },
        "no recorded work dir, no base dir",
    );
    // Through the lane with that base: a relative path does not link, an
    // absolute and a home path still do.
    let owned = none.base;
    let show = Show {
        clock: &Style::PLAIN,
        main: "lead",
        open: None,
        sent: &[],
        icons: true,
        base: &owned,
    };
    let tail = tail_with(
        &lane_of("docs/x.md and /abs/a.md and ~/z.md"),
        400,
        Paint::of(None),
        &show,
        100,
        None,
    );
    let linked: Vec<(String, String)> = (0..tail.rows.len())
        .flat_map(|at| {
            let text: String = tail.rows[at]
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect();
            tail.links[at]
                .iter()
                .map(|(offset, cells, url)| (cells_slice(&text, *offset, *cells), url.clone()))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        linked,
        [
            pair("/abs/a.md", "file:///abs/a.md"),
            pair("~/z.md", "file:///h/z.md"),
        ],
    );
}

// ---------------------------------------------------------------------------
// the app lane: Tail carries the links next to its rows, underlined when drawn
// ---------------------------------------------------------------------------

fn shown(
    body: &str,
    width: usize,
    the_base: &Base,
    look: Option<&Look>,
    open: Option<&[bool]>,
) -> super::lane::Tail {
    let show = Show {
        clock: &Style::PLAIN,
        main: "lead",
        open,
        sent: &[],
        icons: true,
        base: the_base,
    };
    tail_with(&lane_of(body), width, Paint::of(look), &show, 100, None)
}

fn row_links(tail: &super::lane::Tail, at: usize) -> Vec<(String, String)> {
    let text: String = tail.rows[at]
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    tail.links[at]
        .iter()
        .map(|(offset, cells, url)| (cells_slice(&text, *offset, *cells), url.clone()))
        .collect()
}

fn all_links(tail: &super::lane::Tail) -> Vec<(String, String)> {
    (0..tail.rows.len())
        .flat_map(|at| row_links(tail, at))
        .collect()
}

#[test]
fn an_app_lane_row_carries_its_links_as_cell_runs_and_absolute_urls() {
    let tail = shown(
        "see https://example.com/x and src/app.rs:12",
        400,
        &base("/w"),
        None,
        None,
    );
    assert_eq!(tail.links.len(), tail.rows.len(), "links parallel the rows");
    assert_eq!(
        all_links(&tail),
        [
            pair("https://example.com/x", "https://example.com/x"),
            pair("src/app.rs:12", "file:///w/src/app.rs"),
        ],
    );
}

#[test]
fn an_app_lane_link_split_by_a_wrap_rides_both_rows_with_one_url() {
    let tail = shown(
        "check https://example.invalid/abcdefghijklmnopqrst exactly",
        25,
        &base("/w"),
        None,
        None,
    );
    let whole = "https://example.invalid/abcdefghijklmnopqrst";
    let mut parts = Vec::new();
    for (visible, url) in all_links(&tail) {
        assert_eq!(url, whole);
        parts.push(visible);
    }
    assert_eq!(parts.concat(), whole, "the rows' parts spell the one URL");
}

#[test]
fn an_app_card_evidence_row_carries_its_link() {
    let lane = Lane {
        items: vec![Item {
            micros: 1_759_500_000_000_000,
            kind: Kind::Card {
                seat: "lead".to_owned(),
            },
            body: "Pick: vertical wins | horizontal waits.\n\ndetails: .local/x.md".to_owned(),
            record: None,
        }],
        coverage: vec![],
    };
    let owned = base("/w");
    let show = Show {
        clock: &Style::PLAIN,
        main: "lead",
        open: None,
        sent: &[],
        icons: true,
        base: &owned,
    };
    let tail = tail_with(&lane, 400, Paint::of(None), &show, 100, None);
    assert_eq!(
        all_links(&tail),
        [pair(".local/x.md", "file:///w/.local/x.md")],
    );
}

#[test]
fn an_app_link_is_underlined_only_when_a_look_is_drawn() {
    let body = "see https://example.com/x now";
    let link_span = |tail: &super::lane::Tail| -> Option<bool> {
        tail.rows.iter().find_map(|row| {
            row.spans.iter().find_map(|span| {
                (span.content.as_ref() == "https://example.com/x").then(|| {
                    span.style
                        .add_modifier
                        .contains(ratatui_core::style::Modifier::UNDERLINED)
                })
            })
        })
    };
    assert_eq!(
        link_span(&shown(body, 400, &base("/w"), Some(&Look::DEFAULT), None)),
        Some(true),
        "dressed: the link underlines",
    );
    let bare = shown(body, 400, &base("/w"), None, None);
    assert_eq!(
        link_span(&bare),
        Some(false),
        "no look: the link keeps today's styling"
    );
    assert_eq!(all_links(&bare).len(), 1, "the links are recorded anyway");
}

#[test]
fn mark_rows_and_a_fold_move_no_link_cell() {
    let tail = shown(
        "see https://example.com/x and tail line",
        400,
        &base("/w"),
        None,
        None,
    );
    let span = 0..tail.rows.len();
    let mut rows = tail.rows[span].to_vec();
    super::lane::mark_rows(&mut rows, Paint::of(None), true);
    let text = |row: &ratatui_core::text::Line<'_>| -> String {
        row.spans.iter().map(|span| span.content.as_ref()).collect()
    };
    for (offset, cells, url) in &tail.links[1] {
        let marked = cells_slice(&text(&rows[1]), *offset, *cells);
        assert_eq!(marked, "https://example.com/x", "{url:?}");
    }
    // A folded turn: `open: Some(&[false])` really folds, and the fold row
    // links nothing.
    let folded = shown(
        &(1..=31)
            .map(|at| format!("l{at}"))
            .collect::<Vec<_>>()
            .join("\n"),
        40,
        &base("/w"),
        None,
        Some(&[false]),
    );
    assert_eq!(
        folded.links.len(),
        folded.rows.len(),
        "links parallel the folded rows"
    );
    assert!(
        folded.links.last().is_some_and(Vec::is_empty),
        "the fold row links nothing"
    );
}

// ---------------------------------------------------------------------------
// the app backend: OSC 8 around exactly the link cells
// ---------------------------------------------------------------------------

fn cell(symbol: &str) -> Cell {
    let mut cell = Cell::default();
    cell.set_symbol(symbol);
    cell
}

fn cells_of(text: &str) -> Vec<(u16, u16, Cell)> {
    text.chars()
        .enumerate()
        .map(|(at, ch)| {
            (
                u16::try_from(at).expect("an index fits u16"),
                0,
                cell(&ch.to_string()),
            )
        })
        .collect()
}

fn drawn_with(cells: &[(u16, u16, Cell)], runs: &[Run]) -> String {
    let mut out = Vec::new();
    let mut ansi = Ansi::new(&mut out, (40, 4));
    ansi.links(runs.to_vec());
    ansi.draw(cells.iter().map(|(x, y, cell)| (*x, *y, cell)))
        .expect("a Vec takes every byte");
    ansi.flush().expect("a Vec takes every byte");
    String::from_utf8(out).expect("the backend writes UTF-8")
}

fn drawn_plain(cells: &[(u16, u16, Cell)]) -> String {
    let mut out = Vec::new();
    let mut ansi = Ansi::new(&mut out, (40, 4));
    ansi.draw(cells.iter().map(|(x, y, cell)| (*x, *y, cell)))
        .expect("a Vec takes every byte");
    String::from_utf8(out).expect("the backend writes UTF-8")
}

#[test]
fn the_backend_wraps_a_link_run_in_osc8_around_exactly_its_cells() {
    let url = "https://example.com";
    let out = drawn_with(
        &cells_of(url),
        &[Run {
            x: 0,
            y: 0,
            width: u16::try_from(url.len()).expect("an ascii url fits u16"),
            url: url.to_owned(),
        }],
    );
    assert_eq!(
        out,
        format!("\x1b[1;1H\x1b[0m\x1b]8;id=1;{url}\x1b\\{url}\x1b]8;;\x1b\\\x1b[0m"),
        "{out:?}"
    );
}

#[test]
fn the_backend_closes_a_link_before_a_non_link_cell() {
    let out = drawn_with(
        &cells_of("hi x"),
        &[Run {
            x: 0,
            y: 0,
            width: 2,
            url: "https://a.b".to_owned(),
        }],
    );
    assert_eq!(
        out,
        format!("\x1b[1;1H\x1b[0m\x1b]8;id=1;https://a.b\x1b\\hi\x1b]8;;\x1b\\ x\x1b[0m"),
        "{out:?}"
    );
}

#[test]
fn the_backend_closes_a_link_before_a_cursor_move() {
    let mut cells = cells_of("hi");
    cells.push((7, 0, cell("y")));
    let out = drawn_with(
        &cells,
        &[Run {
            x: 0,
            y: 0,
            width: 2,
            url: "https://a.b".to_owned(),
        }],
    );
    assert_eq!(
        out,
        format!("\x1b[1;1H\x1b[0m\x1b]8;id=1;https://a.b\x1b\\hi\x1b]8;;\x1b\\\x1b[1;8Hy\x1b[0m"),
        "{out:?}"
    );
}

#[test]
fn two_adjacent_links_close_reopen_and_take_distinct_ids() {
    let out = drawn_with(
        &cells_of("abcdef"),
        &[
            Run {
                x: 0,
                y: 0,
                width: 3,
                url: "https://a.b".to_owned(),
            },
            Run {
                x: 3,
                y: 0,
                width: 3,
                url: "https://c.d".to_owned(),
            },
        ],
    );
    assert_eq!(
        out,
        "\x1b[1;1H\x1b[0m\x1b]8;id=1;https://a.b\x1b\\abc\x1b]8;;\x1b\\\x1b]8;id=2;https://c.d\x1b\\def\x1b]8;;\x1b\\\x1b[0m",
        "{out:?}"
    );
}

#[test]
fn two_rows_of_one_url_share_the_id_of_the_first_run() {
    let mut cells = cells_of("abc");
    cells.extend("def".chars().enumerate().map(|(at, ch)| {
        (
            u16::try_from(at).expect("an index fits u16"),
            1,
            cell(&ch.to_string()),
        )
    }));
    let out = drawn_with(
        &cells,
        &[
            Run {
                x: 0,
                y: 0,
                width: 3,
                url: "https://a.b".to_owned(),
            },
            Run {
                x: 0,
                y: 1,
                width: 3,
                url: "https://a.b".to_owned(),
            },
        ],
    );
    assert_eq!(
        out.matches("\x1b]8;id=1;https://a.b\x1b\\").count(),
        2,
        "both rows open the same id: {out:?}"
    );
    assert_eq!(out.matches("\x1b]8;;\x1b\\").count(), 2, "{out:?}");
}

#[test]
fn a_zero_width_run_and_an_unsafe_url_never_reach_the_stream() {
    let url = "https://example.com";
    let plain = drawn_plain(&cells_of(url));
    let zero = drawn_with(
        &cells_of(url),
        &[Run {
            x: 0,
            y: 0,
            width: 0,
            url: url.to_owned(),
        }],
    );
    assert_eq!(zero, plain, "a zero-width run writes nothing");
    let unsafe_out = drawn_with(
        &cells_of(url),
        &[Run {
            x: 0,
            y: 0,
            width: u16::try_from(url.len()).expect("an ascii url fits u16"),
            url: "https://exa\x07mple.com".to_owned(),
        }],
    );
    assert_eq!(
        unsafe_out, plain,
        "an unsafe url is dropped, never repaired"
    );
}

#[test]
fn the_backend_wraps_only_the_cells_the_frame_drew() {
    let url = "https://example.com";
    let out = drawn_with(
        &cells_of(&url[..6]),
        &[Run {
            x: 0,
            y: 0,
            width: u16::try_from(url.len()).expect("an ascii url fits u16"),
            url: url.to_owned(),
        }],
    );
    assert_eq!(
        out,
        format!("\x1b[1;1H\x1b[0m\x1b]8;id=1;{url}\x1b\\https:\x1b]8;;\x1b\\\x1b[0m"),
        "{out:?}"
    );
}

#[test]
fn the_links_table_answers_the_cells_the_diff_must_force() {
    let mut out = Vec::new();
    let mut ansi = Ansi::new(&mut out, (40, 4));
    let fresh = ansi.links(vec![Run {
        x: 0,
        y: 0,
        width: 2,
        url: "https://a.b".to_owned(),
    }]);
    assert_eq!(
        fresh,
        vec![Position::new(0, 0), Position::new(1, 0)],
        "every cell of a run is forced"
    );
    let same = ansi.links(vec![Run {
        x: 0,
        y: 0,
        width: 2,
        url: "https://a.b".to_owned(),
    }]);
    assert_eq!(same, vec![Position::new(0, 0), Position::new(1, 0)]);
    let gone = ansi.links(vec![]);
    assert_eq!(
        gone,
        vec![Position::new(0, 0), Position::new(1, 0)],
        "the previous table's cells stay forced once unlinked"
    );
    // A retargeted cell (same place, other url) must be forced too, or the
    // terminal keeps the old link on it forever: its symbol is unchanged and
    // the diff would skip it.
    let moved = ansi.links(vec![Run {
        x: 0,
        y: 0,
        width: 2,
        url: "https://c.d".to_owned(),
    }]);
    assert_eq!(moved, vec![Position::new(0, 0), Position::new(1, 0)]);
}

#[test]
fn an_empty_table_after_no_table_writes_exactly_todays_bytes() {
    let url = "https://example.com";
    let out = drawn_with(&cells_of(url), &[]);
    assert_eq!(out, drawn_plain(&cells_of(url)), "{out:?}");
    assert!(!out.contains("\x1b]8;"), "{out:?}");
}

#[test]
fn the_links_change_no_byte_of_the_symbols_or_the_cursor_logic() {
    let url = "https://example.com";
    let cells = cells_of(url);
    let plain = drawn_plain(&cells);
    let linked = drawn_with(
        &cells,
        &[Run {
            x: 0,
            y: 0,
            width: u16::try_from(url.len()).expect("an ascii url fits u16"),
            url: url.to_owned(),
        }],
    );
    assert_eq!(
        stripped(&linked),
        stripped(&plain),
        "the symbols are untouched"
    );
    let mut out = Vec::new();
    let mut ansi = Ansi::new(&mut out, (40, 4));
    ansi.set_cursor_position((4, 2)).expect("the move writes");
    let moved = String::from_utf8(out).expect("the backend writes UTF-8");
    assert_eq!(moved, "\x1b[3;5H");
    assert!(!moved.contains("\x1b]8;"));
}

// ---------------------------------------------------------------------------
// app.rs's draw_frame: byte-equivalent to Terminal::draw, runs aside
// ---------------------------------------------------------------------------

/// A writer the test reads while the `Terminal` holds the backend.
#[derive(Clone, Default)]
struct Sink(Rc<RefCell<Vec<u8>>>);

impl std::io::Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A deterministic frame: `step` says what it draws and where the cursor
/// stands. Step 4 resizes and draws a wide glyph.
fn fill(buf: &mut Buffer, step: usize) -> Option<Position> {
    let area = buf.area;
    for y in 0..area.height {
        for x in 0..area.width {
            buf[(x, y)].reset();
        }
    }
    let word = match step {
        0 | 1 => "first frame",
        2 => "changed frame",
        3 => "another frame",
        4 => "wide 你 row",
        _ => "after clear",
    };
    for (at, ch) in word.chars().enumerate() {
        let (x, y) = (
            u16::try_from(at).expect("an index fits u16"),
            u16::try_from(at).expect("an index fits u16") % 3,
        );
        if usize::from(x) < usize::from(area.width) && usize::from(y) < usize::from(area.height) {
            buf[(x, y)].set_symbol(&ch.to_string());
        }
    }
    match step {
        0 | 1 | 3 | 5 => Some(Position::new(2, 4)),
        _ => None,
    }
}

fn streamed(reference: bool) -> String {
    let sink = Sink::default();
    let mut terminal =
        Terminal::new(Ansi::new(sink.clone(), (40, 12))).expect("the terminal builds");
    for step in [0usize, 1, 2, 3, 4, 5] {
        if step == 4 {
            terminal.backend_mut().resize((44, 12));
        }
        if step == 5 {
            terminal.clear().expect("the clear writes");
        }
        if reference {
            terminal
                .draw(|frame| {
                    if let Some(at) = fill(frame.buffer_mut(), step) {
                        frame.set_cursor_position(at);
                    }
                })
                .expect("the reference draws");
        } else {
            super::draw_frame(&mut terminal, |buf| (fill(buf, step), Vec::new()))
                .expect("draw_frame draws");
        }
    }
    let bytes = sink.0.borrow().clone();
    String::from_utf8(bytes).expect("the backend writes UTF-8")
}

#[test]
fn draw_frame_without_runs_writes_exactly_what_terminal_draw_writes() {
    assert_eq!(streamed(true), streamed(false), "the flows are one flow");
}

/// The visible text a byte stream paints: CUP places, SGR and OSC 8 style,
/// every other char is a glyph that advances by its cells.
fn replay(stream: &str, cols: usize, rows: usize) -> Vec<Vec<char>> {
    let mut grid = vec![vec![' '; cols]; rows];
    let (mut x, mut y) = (0usize, 0usize);
    let mut chars = stream.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\x1b' => match chars.next() {
                Some('[') => {
                    let mut params = String::new();
                    let mut kind = ' ';
                    for next in chars.by_ref() {
                        if next.is_ascii_alphabetic() {
                            kind = next;
                            break;
                        }
                        params.push(next);
                    }
                    if kind == 'H' {
                        let places: Vec<usize> = params
                            .split(';')
                            .map(|part| part.parse::<usize>().unwrap_or(1))
                            .collect();
                        y = places.first().copied().unwrap_or(1).saturating_sub(1);
                        x = places.get(1).copied().unwrap_or(1).saturating_sub(1);
                    }
                }
                Some(']') => loop {
                    match chars.next() {
                        Some('\x07') | None => break,
                        Some('\x1b') => {
                            if chars.next() == Some('\\') {
                                break;
                            }
                        }
                        Some(_) => {}
                    }
                },
                _ => {}
            },
            ch => {
                if y < rows && x < cols {
                    grid[y][x] = ch;
                }
                x += crate::event_text::cell(ch).max(1);
            }
        }
    }
    grid
}

#[test]
fn a_frame_with_runs_differs_from_the_link_less_frame_only_by_the_links() {
    let url = "https://example.com/x";
    let sink = Sink::default();
    let mut terminal =
        Terminal::new(Ansi::new(sink.clone(), (40, 12))).expect("the terminal builds");
    // Frame 1 links cells 0..4 of row 1; frame 2 unlinks them.
    super::draw_frame(&mut terminal, |buf| {
        for (at, ch) in "abcdefgh".chars().enumerate() {
            buf[(u16::try_from(at).expect("an index fits u16"), 1)].set_symbol(&ch.to_string());
        }
        (
            Some(Position::new(0, 3)),
            vec![Run {
                x: 0,
                y: 1,
                width: 4,
                url: url.to_owned(),
            }],
        )
    })
    .expect("the linked frame draws");
    let linked_stream = sink.0.borrow().clone();
    super::draw_frame(&mut terminal, |buf| {
        for (at, ch) in "abcdefgh".chars().enumerate() {
            buf[(u16::try_from(at).expect("an index fits u16"), 1)].set_symbol(&ch.to_string());
        }
        (Some(Position::new(0, 3)), Vec::new())
    })
    .expect("the unlinked frame draws");
    let all = sink.0.borrow().clone();
    let second = String::from_utf8(all[linked_stream.len()..].to_vec()).expect("UTF-8");
    assert!(
        !second.contains("\x1b]8;"),
        "the stale cells are re-written unlinked: {second:?}"
    );
    assert!(
        second.contains("abcd"),
        "the unlinked cells are re-written at all: {second:?}"
    );
    let linked = String::from_utf8(linked_stream).expect("UTF-8");
    assert!(linked.contains("\x1b]8;"), "{linked:?}");
    // The linked stream paints the same glyphs: the pairs are zero-width.
    let replayed = replay(&linked, 40, 12);
    assert_eq!(&replayed[1][..8], &['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h']);
}

// ---------------------------------------------------------------------------
// selection still reads the visible text
// ---------------------------------------------------------------------------

/// This test guards that no escape bytes sit in the SPAN CONTENT a selection
/// reads back (the buffer's cells), not the backend's emission: it builds the
/// buffer by hand from the lane's spans, exactly what `select::text` sees.
#[test]
fn selection_over_a_linked_lane_row_copies_its_visible_text() {
    let body = "see https://example.com/x now";
    let tail = shown(body, 400, &base("/w"), Some(&Look::DEFAULT), None);
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 8));
    for (at, row) in tail.rows.iter().enumerate() {
        let mut x = 0u16;
        for span in &row.spans {
            for ch in span.content.chars() {
                if let Some(cell) = buf.cell_mut((x, u16::try_from(at).expect("an index fits u16")))
                {
                    cell.set_symbol(&ch.to_string());
                }
                x += u16::try_from(crate::event_text::cell(ch)).expect("a cell width fits u16");
            }
        }
    }
    let region = crate::app::select::Region {
        panel: crate::app::select::Panel::Lane,
        area: Rect::new(0, 1, 40, 1),
    };
    let mut selection = crate::app::select::Selection::start(
        &region,
        Position::new(0, 1),
        crate::app::select::fingerprint(&buf, region.area, &[]),
    );
    selection.extend(Position::new(39, 1));
    let copied = crate::app::select::text(&buf, &selection, &[]);
    assert!(copied.contains("https://example.com/x"), "{copied:?}");
    assert!(!copied.contains('\x1b'), "{copied:?}");
    assert!(!copied.contains('\x07'), "{copied:?}");
}

// ---------------------------------------------------------------------------
// the app frame: a drawn link is a tight run over its cells
// ---------------------------------------------------------------------------

#[test]
fn a_drawn_url_is_one_tight_run_over_its_cells() {
    let fleet = Fleet {
        rows: vec![],
        home: Some("s1".to_owned()),
    };
    let model = Model::new(&fleet);
    let mut entry = SessionEntry::new("s1", Status::Running);
    entry.work_dir = Some("/w".to_owned());
    let overview = crate::app::overview::Overview {
        open: vec![],
        dir: None,
        base: base("/w"),
        unanswered: None,
        unanswered_oldest_secs: None,
    };
    let lane = lane_of("see https://example.com/x now");
    let screen = Screen {
        fleet: &fleet,
        model: &model,
        overview: &overview,
        selected: Some(&entry),
        pair: &["lead".to_owned()],
        agents: None,
        lane: &lane,
        composer: Composer::ReadOnly {
            why: "owned elsewhere",
        },
        look: Some(Look::read("on", "darcula", "on", "on")),
        zone: None,
        now: Timestamp::from_epoch(1_759_500_600),
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
    let layout = super::draw::draw_with_layout(&screen, Wait::default(), &mut buf);
    let runs: Vec<&Run> = layout
        .links
        .iter()
        .filter(|run| run.url == "https://example.com/x")
        .collect();
    assert_eq!(runs.len(), 1, "{:?}", layout.links);
    let run = runs[0];
    let under: String = (run.x..run.x + run.width)
        .map(|x| buf[(x, run.y)].symbol())
        .collect();
    assert_eq!(under, "https://example.com/x", "{under:?} at {run:?}");
}

#[test]
fn two_links_on_one_row_are_two_runs_and_nothing_between_them() {
    let fleet = Fleet {
        rows: vec![],
        home: Some("s1".to_owned()),
    };
    let model = Model::new(&fleet);
    let mut entry = SessionEntry::new("s1", Status::Running);
    entry.work_dir = Some("/w".to_owned());
    let overview = crate::app::overview::Overview {
        open: vec![],
        dir: None,
        base: base("/w"),
        unanswered: None,
        unanswered_oldest_secs: None,
    };
    let lane = lane_of("a https://a.dev/x, b /Users/me/y.md and done");
    let screen = Screen {
        fleet: &fleet,
        model: &model,
        overview: &overview,
        selected: Some(&entry),
        pair: &["lead".to_owned()],
        agents: None,
        lane: &lane,
        composer: Composer::ReadOnly {
            why: "owned elsewhere",
        },
        look: Some(Look::read("on", "darcula", "on", "on")),
        zone: None,
        now: Timestamp::from_epoch(1_759_500_600),
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
    let layout = super::draw::draw_with_layout(&screen, Wait::default(), &mut buf);
    let mut runs: Vec<Run> = layout
        .links
        .iter()
        .filter(|run| run.url == "https://a.dev/x" || run.url == "file:///Users/me/y.md")
        .cloned()
        .collect();
    runs.sort_by_key(|run| (run.y, run.x));
    assert_eq!(runs.len(), 2, "{:?}", layout.links);
    let under = |run: &Run| -> String {
        (run.x..run.x + run.width)
            .map(|x| buf[(x, run.y)].symbol())
            .collect()
    };
    assert_eq!(under(&runs[0]), "https://a.dev/x");
    assert_eq!(under(&runs[1]), "/Users/me/y.md");
    assert!(
        runs[0].x + runs[0].width < runs[1].x,
        "a gap stands between: {:?}",
        layout.links
    );
}
