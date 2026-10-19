//! Frozen acceptance cases: appread L2, app side — the reader pure half,
//! the file door, the browse model, the draw and the App glue that joins
//! them. Independent navigator appread-nav (Meta Muse Wonder Full 1.0
//! contributor), frozen against driver appread's plan-r3. Driver installs
//! byte-identically, wired from src/app.rs via `#[cfg(test)]
//! #[path = "app/appread_spec.rs"] mod appread_spec;`. The link side is
//! `src/console/appread_link_spec.rs`, the loader side
//! `src/app/loader/read_spec.rs`. Spec-r3 adds the seat-click close pin
//! and the moved browse-decode rows in `tests/it/app_spec.rs` (the
//! existing oracle for the decode table the new keys extend); spec-r4
//! is the RED-run compile fix only (named lifetimes on a view builder).
//!
//! What the reader is: in `ae app` browse, `r` opens the FIRST file link
//! of the MARKED turn (the turn `y` copies) in a read-only column over
//! the chat lane, read once by the loader through `app::filedoor`, jumped
//! to its `:line`; `Esc` returns to the lane exactly as it was. Web
//! links never open (the terminal's OSC 8 has them). Nothing is written,
//! typed or journaled — pinned as behaviour, not just said here (lead
//! IMPORTANT-1, spec-r2): the loader spec proves the read leaves a
//! journal, a draft and the target byte-identical, and the App round
//! trip below proves no store-bound surface is ever entered (in this
//! app a journal record travels only as a queued/flying ask and a
//! `console.draft` write only under an entered writer, so those fields
//! standing empty after open/scroll/close is the same fact one boundary
//! earlier).
//!
//! Surface compiled against (plan-r3 §7 as amended by nav A1-A10):
//! `reader::{MAX_BYTES, MAX_LINES, LINE_CHARS, BINARY_PROBE,
//! Body::{Text{lines,size}, Gap}, decode, Flow, flow, top_for, window,
//! View (pub fields), status(&View, visible)}` — widths are TEXT columns,
//! the gutter excluded; `Body` is `Clone + PartialEq` like peek's.
//! `filedoor::{Roots, Read, read}` with the refusal precedence of plan
//! §3. `model::{Key::{Read, Top, Bottom}, Act::Read}` and the overlay
//! methods `open_reader/close_reader/reader_open/reader_top ->
//! Option<usize>/reader_generation -> Option<u64>/set_reader_page_rows/
//! clamp_reader/reader_wheel/set_reader_top`: `reader_wheel(up=true)`
//! moves toward the file's start (top shrinks). `draw::draw_reader` over
//! a bare buffer is the column: row 0 the `Read ·` header, gutters on
//! text, no gutter on Gap/loading rows, `Bounds{visible, max_back =
//! max_top}`.
//!
//! Named residuals (not pinned here): a sidebar session press closing
//! the reader, drag-select copying from it, the `reader` read-gate key,
//! and the FIFO-under-race case (no way to make a FIFO in the suite
//! without a new Command): nav verifies those at batch review instead.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::Modifier;

use super::draw::draw_reader;
use super::fleet::{Counts, Facts, Fleet, Line2, Row};
use super::loader::{Answer, Request};
use super::model::{Act, Key, Model, browse_keys};
use super::reader::{self, Body, View};
use super::{App, filedoor, take_keys};
use crate::console::input;
use crate::console::lane::{Item, Kind, Lane};
use crate::console::link::Base;
use crate::digest::{SessionEntry, Status};
use crate::theme::Mark;
use crate::time::Timestamp;

// ---------------------------------------------------------------------------
// reader::decode — the pure bytes-to-body step
// ---------------------------------------------------------------------------

fn text_lines(body: &Body) -> &[String] {
    let Body::Text { lines, .. } = body else {
        panic!("not text: {body:?}");
    };
    lines
}

#[test]
fn decode_pins_the_named_caps() {
    assert_eq!(reader::MAX_BYTES, 1_048_576);
    assert_eq!(reader::MAX_LINES, 50_000);
    assert_eq!(reader::LINE_CHARS, 2000);
    assert_eq!(reader::BINARY_PROBE, 8192);
}

#[test]
fn decode_splits_trims_cr_and_carries_the_size_it_was_given() {
    let body = reader::decode(b"a\nb\n", 999);
    assert_eq!(
        body,
        Body::Text {
            lines: vec!["a".to_owned(), "b".to_owned()],
            size: 999,
        },
    );
    let body = reader::decode(b"a\r\nb\r\r\nc\rd\r\n", 0);
    assert_eq!(text_lines(&body), ["a", "b", "c\rd"]);
    let body = reader::decode(b"a\n\nb\n   \n\t\n\n", 0);
    assert_eq!(text_lines(&body), ["a", "", "b"]);
    let body = reader::decode(b"no newline at end", 0);
    assert_eq!(text_lines(&body), ["no newline at end"]);
}

#[test]
fn decode_empty_and_all_blank_bytes_are_empty_text_not_a_gap() {
    for bytes in [&b""[..], b"\n\n", b"  \n\t\n"] {
        assert_eq!(
            reader::decode(bytes, 3),
            Body::Text {
                lines: Vec::new(),
                size: 3,
            },
            "{bytes:?}",
        );
    }
}

#[test]
fn decode_is_lossy_and_keeps_other_controls_for_the_painter() {
    let body = reader::decode(b"caf\xe9 \xffx\n", 0);
    assert_eq!(text_lines(&body), ["caf\u{fffd} \u{fffd}x"]);
    let body = reader::decode(b"a\x1b[31mb\x07", 0);
    assert_eq!(
        text_lines(&body),
        ["a\x1b[31mb\x07"],
        "neutralising is the painter's job, never the decoder's",
    );
}

#[test]
fn decode_refuses_a_nul_inside_the_probe_and_only_there() {
    let refused = Body::Gap("binary file (NUL byte)".to_owned());
    assert_eq!(reader::decode(b"\0", 0), refused);
    let mut at_edge = vec![b'a'; reader::BINARY_PROBE];
    let last = at_edge.len() - 1;
    at_edge[last] = 0;
    assert_eq!(reader::decode(&at_edge, 0), refused, "index 8191 refuses");
    let mut past = vec![b'a'; reader::BINARY_PROBE];
    past.extend_from_slice(b"\n\0x");
    let body = reader::decode(&past, 0);
    let lines = text_lines(&body);
    assert_eq!(lines.len(), 2, "index 8192 and beyond is displayable text");
    assert_eq!(lines[1], "\0x");
}

#[test]
fn decode_caps_one_line_at_two_thousand_scalars_ellipsis_inside() {
    let at = "é".repeat(reader::LINE_CHARS);
    let body = reader::decode(at.as_bytes(), 0);
    assert_eq!(text_lines(&body), [at.as_str()]);
    let over = "é".repeat(reader::LINE_CHARS + 1);
    let body = reader::decode(over.as_bytes(), 0);
    let kept = &text_lines(&body)[0];
    assert_eq!(kept.chars().count(), reader::LINE_CHARS);
    assert!(kept.ends_with('…'));
    assert!(kept.starts_with(&"é".repeat(reader::LINE_CHARS - 1)));
}

#[test]
fn decode_refuses_past_max_lines_kept_count_and_passes_at_it() {
    let at = "x\n".repeat(reader::MAX_LINES);
    let body = reader::decode(at.as_bytes(), 0);
    assert_eq!(text_lines(&body).len(), reader::MAX_LINES);
    let over = "x\n".repeat(reader::MAX_LINES + 1);
    assert_eq!(
        reader::decode(over.as_bytes(), 0),
        Body::Gap(format!(
            "too many lines: {} (limit 50000)",
            reader::MAX_LINES + 1
        )),
    );
    // The kept count: trailing blanks dropped before the cap is judged.
    let blank_tail = format!("{}\n\n", "x\n".repeat(reader::MAX_LINES - 1));
    let body = reader::decode(blank_tail.as_bytes(), 0);
    assert_eq!(text_lines(&body).len(), reader::MAX_LINES - 1);
}

// ---------------------------------------------------------------------------
// reader flow / window / top_for / status — the pure geometry
// ---------------------------------------------------------------------------

/// The oracle display-row count of one line at a text width, from the
/// wrap the chat already uses (its one-cell bar is not drawn, hence +1).
fn oracle_rows(lines: &[String], width: usize) -> Vec<usize> {
    lines
        .iter()
        .map(|line| {
            crate::console::wrap::wrap(&crate::board::terminal_text(line), width + 1, 0).len()
        })
        .collect()
}

#[test]
fn flow_counts_rows_and_names_each_lines_first_row() {
    let lines: Vec<String> = ["a", "bb", ""].iter().map(|s| (*s).to_owned()).collect();
    let flow = reader::flow(&lines, 10);
    assert_eq!(flow.rows(), 3);
    assert_eq!((flow.row_of(1), flow.row_of(2), flow.row_of(3)), (0, 1, 2));
    assert_eq!(flow.row_of(0), 0, "line 0 is the first line");
    assert_eq!(flow.row_of(99), 2, "past EOF is the last line's row");

    let mixed: Vec<String> = [
        "short",
        "a much longer line that wraps at ten columns more than twice",
        "\ttabbed and indented",
        "üñïçödé scalars",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    let flow = reader::flow(&mixed, 10);
    let oracle = oracle_rows(&mixed, 10);
    assert_eq!(flow.rows(), oracle.iter().sum::<usize>());
    let mut first = 0;
    for (index, rows) in oracle.iter().enumerate() {
        assert_eq!(flow.row_of(index + 1), first, "line {}", index + 1);
        first += rows;
    }
    assert_eq!(
        flow.row_of(mixed.len() + 5),
        first - *oracle.last().expect("rows")
    );
}

#[test]
fn flow_is_total_width_zero_is_width_one_and_empty_is_empty() {
    let lines: Vec<String> = ["abc", "de"].iter().map(|s| (*s).to_owned()).collect();
    let (zero, one) = (reader::flow(&lines, 0), reader::flow(&lines, 1));
    assert_eq!(zero.rows(), one.rows());
    assert_eq!(zero.row_of(2), one.row_of(2));
    let empty = reader::flow(&[], 10);
    assert_eq!((empty.rows(), empty.row_of(0), empty.row_of(9)), (0, 0, 0));
}

#[test]
fn window_clamps_the_top_against_the_last_full_page() {
    assert_eq!(reader::window(10, 4, 0), (0, 6));
    assert_eq!(reader::window(10, 4, 3), (3, 6));
    assert_eq!(reader::window(10, 4, 99), (6, 6));
    assert_eq!(reader::window(3, 4, 2), (0, 0));
    assert_eq!(reader::window(0, 4, 0), (0, 0));
    assert_eq!(reader::window(10, 0, 3), (3, 10));
}

#[test]
fn top_for_keeps_context_above_and_never_loses_the_target() {
    assert_eq!(reader::top_for(0, 10, 50), 0);
    assert_eq!(reader::top_for(2, 10, 50), 0);
    assert_eq!(reader::top_for(10, 10, 50), 7, "three rows of context");
    assert_eq!(reader::top_for(10, 3, 50), 8, "the target stays visible");
    assert_eq!(reader::top_for(10, 1, 50), 10);
    assert_eq!(
        reader::top_for(10, 0, 50),
        10,
        "visible counts as at least one"
    );
    assert_eq!(reader::top_for(10, 10, 5), 5, "never past the last page");
}

fn view<'a>(
    body: Option<&'a Body>,
    flow: Option<&'a reader::Flow>,
    top: usize,
    links: usize,
) -> View<'a> {
    View {
        path: "/w/a.rs",
        line: None,
        body,
        flow,
        top,
        links,
    }
}

#[test]
fn status_names_loading_refusal_and_the_drawn_rows() {
    let lines: Vec<String> = (1..=30).map(|n| format!("L{n}")).collect();
    let body = Body::Text { lines, size: 0 };
    let Body::Text { lines: held, .. } = &body else {
        unreachable!()
    };
    let flow = reader::flow(held, 80);
    assert_eq!(reader::status(&view(None, None, 0, 1), 10), "loading");
    let gap = Body::Gap("why".to_owned());
    assert_eq!(
        reader::status(&view(Some(&gap), None, 0, 1), 10),
        "not read",
    );
    let text = |top: usize, links: usize| view(Some(&body), Some(&flow), top, links);
    assert_eq!(reader::status(&text(0, 1), 10), "rows 1-10/30");
    assert_eq!(reader::status(&text(5, 1), 10), "rows 6-15/30");
    assert_eq!(reader::status(&text(25, 1), 10), "rows 21-30/30");
    assert_eq!(
        reader::status(&text(0, 3), 10),
        "rows 1-10/30 · first of 3 links",
    );
    assert_eq!(
        reader::status(&view(None, None, 0, 3), 10),
        "loading",
        "the links note rides the rows form only",
    );
    let no_flow = view(Some(&body), None, 0, 1);
    assert_eq!(
        reader::status(&no_flow, 2),
        "rows 1-2/30",
        "no flow yet: one row a line",
    );
    let empty = Body::Text {
        lines: Vec::new(),
        size: 0,
    };
    let view = View {
        path: "/w/a.rs",
        line: None,
        body: Some(&empty),
        flow: None,
        top: 0,
        links: 1,
    };
    assert_eq!(reader::status(&view, 10), "rows 0-0/0");
}

// ---------------------------------------------------------------------------
// filedoor — the named read door against a real temp filesystem
// ---------------------------------------------------------------------------

fn root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae-appread-door-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the fixture root is made");
    dir
}

fn roots(dir: Option<&Path>, home: Option<&Path>) -> filedoor::Roots {
    filedoor::Roots {
        dir: dir.map(Path::to_path_buf),
        home: home.map(Path::to_path_buf),
    }
}

fn refused(read: &filedoor::Read) -> &str {
    let filedoor::Read::Refused(why) = read else {
        panic!("not refused: {read:?}");
    };
    why
}

#[test]
fn the_door_reads_a_file_under_either_root_and_names_its_size() {
    let work = root("work");
    let home = root("home");
    let deep = work.join("sub/dir");
    std::fs::create_dir_all(&deep).expect("dirs are made");
    let file = deep.join("a.txt");
    std::fs::write(&file, b"hi").expect("the fixture file is written");
    let filedoor::Read::Bytes { bytes, size } =
        filedoor::read(&file, &roots(Some(&work), None), 100)
    else {
        panic!("not bytes");
    };
    assert_eq!((bytes.as_slice(), size), (b"hi".as_slice(), 2));
    let homed = home.join("b.txt");
    std::fs::write(&homed, b"ho").expect("the fixture file is written");
    assert!(matches!(
        filedoor::read(&homed, &roots(Some(&work), Some(&home)), 100),
        filedoor::Read::Bytes { .. }
    ));
    let _ = std::fs::remove_dir_all(&work);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn the_door_refusals_come_in_their_pinned_order_and_words() {
    let work = root("order");
    let file = work.join("a.txt");
    std::fs::write(&file, b"12345678901").expect("the fixture file is written");
    // 1. No base — before the target is ever looked at.
    assert_eq!(
        refused(&filedoor::read(&file, &roots(None, None), 100)),
        "no base to read against",
    );
    let gone = work.join("no-such-dir");
    assert_eq!(
        refused(&filedoor::read(&file, &roots(Some(&gone), None), 100)),
        "no base to read against",
        "a root that cannot canonicalize is no root",
    );
    // 2. Missing target, then other io errors by kind.
    assert_eq!(
        refused(&filedoor::read(
            &work.join("gone.txt"),
            &roots(Some(&work), None),
            100
        )),
        "not found",
    );
    assert_eq!(
        refused(&filedoor::read(
            &file.join("child"),
            &roots(Some(&work), None),
            100
        )),
        "unreadable: not a directory",
    );
    // 4. The root itself is inside containment but no regular file.
    assert_eq!(
        refused(&filedoor::read(&work, &roots(Some(&work), None), 100)),
        "not a regular file",
    );
    // 5. Size, by the fstat number and the caller's limit.
    assert_eq!(
        refused(&filedoor::read(&file, &roots(Some(&work), None), 10)),
        "too large: 11 bytes (limit 10)",
    );
    assert!(matches!(
        filedoor::read(&file, &roots(Some(&work), None), 11),
        filedoor::Read::Bytes { .. }
    ));
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn the_door_refuses_dotdot_and_symlink_escapes_but_follows_in_tree_links() {
    let work = root("tree");
    let outside = root("outside");
    let secret = outside.join("secret.txt");
    std::fs::write(&secret, b"s").expect("the fixture file is written");
    let inner = work.join("sub");
    std::fs::create_dir_all(&inner).expect("dirs are made");
    // A `..` path out of the work dir into the sibling root.
    let escape = inner
        .join("../..")
        .join(outside.file_name().expect("the fixture root has a name"));
    let escape = escape.join("secret.txt");
    assert_eq!(
        refused(&filedoor::read(&escape, &roots(Some(&work), None), 100)),
        "outside the session dir and home",
    );
    // Plainly outside, no traversal at all.
    assert_eq!(
        refused(&filedoor::read(&secret, &roots(Some(&work), None), 100)),
        "outside the session dir and home",
    );
    let link_out = work.join("link-out");
    std::os::unix::fs::symlink(&secret, &link_out).expect("the link is made");
    assert_eq!(
        refused(&filedoor::read(&link_out, &roots(Some(&work), None), 100)),
        "outside the session dir and home",
        "a symlink is judged by where it lands",
    );
    let real = inner.join("real.txt");
    std::fs::write(&real, b"r").expect("the fixture file is written");
    let link_in = work.join("link-in");
    std::os::unix::fs::symlink(&real, &link_in).expect("the link is made");
    assert!(matches!(
        filedoor::read(&link_in, &roots(Some(&work), None), 100),
        filedoor::Read::Bytes { .. }
    ));
    let link_dir = work.join("link-dir");
    std::os::unix::fs::symlink(&inner, &link_dir).expect("the link is made");
    assert_eq!(
        refused(&filedoor::read(&link_dir, &roots(Some(&work), None), 100)),
        "not a regular file",
    );
    let _ = std::fs::remove_dir_all(&work);
    let _ = std::fs::remove_dir_all(&outside);
}

// ---------------------------------------------------------------------------
// model — keys, the reader overlay, epoch and preservation
// ---------------------------------------------------------------------------

fn fleet(names: &[&str]) -> Fleet {
    let row = |(at, name): (usize, &&str)| Row {
        name: (*name).to_owned(),
        index: at + 1,
        mark: Mark::Working,
        needy: false,
        counts: Counts::Unknown,
        line2: Line2::NoGoal,
        home: *name == "api",
    };
    Fleet {
        rows: names.iter().enumerate().map(row).collect(),
        home: Some("api".to_owned()),
    }
}

#[test]
fn the_new_letters_decode_and_home_end_are_their_kin() {
    let text = |bytes: &[u8]| browse_keys(&input::Key::Text(bytes.to_vec()));
    assert_eq!(text(b"r"), [Key::Read]);
    assert_eq!(text(b"g"), [Key::Top]);
    assert_eq!(text(b"G"), [Key::Bottom]);
    assert_eq!(browse_keys(&input::Key::Home), [Key::Top]);
    assert_eq!(browse_keys(&input::Key::End), [Key::Bottom]);
}

#[test]
fn read_in_browse_asks_regardless_of_writing_and_the_ends_are_inert() {
    let both = fleet(&["api", "web"]);
    let mut model = Model::new(&both);
    assert_eq!(model.key(Key::Read, &both, false), Act::Read);
    assert_eq!(model.key(Key::Read, &both, true), Act::Read);
    assert!(!model.reader_open(), "the Act asks; the App opens");
    let before = model.clone();
    assert_eq!(model.key(Key::Top, &both, false), Act::None);
    assert_eq!(model.key(Key::Bottom, &both, false), Act::None);
    assert_eq!(model, before, "browse is bit-identical after g/G");
}

#[test]
fn the_new_keys_are_swallowed_while_settings_or_peek_are_open() {
    let both = fleet(&["api", "web"]);
    let mut model = Model::new(&both);
    model.open_settings(1);
    for key in [Key::Read, Key::Top, Key::Bottom] {
        let before = model.clone();
        assert_eq!(model.key(key, &both, false), Act::None, "{key:?}");
        assert_eq!(model, before);
        assert!(model.settings_open());
    }
    model.close_settings();
    model.open_peek(1);
    let _ = model.key(Key::Up, &both, false);
    for key in [Key::Read, Key::Top, Key::Bottom] {
        assert_eq!(model.key(key, &both, false), Act::None, "{key:?}");
        assert!(model.peek_open());
        assert_eq!(model.peek_back(), Some(1), "the peek did not move");
    }
}

#[test]
fn the_reader_overlay_scrolls_clamps_and_closes_with_peeks_shape() {
    let both = fleet(&["api", "web"]);
    let mut model = Model::new(&both);
    let epoch = model.epoch();
    model.close_reader();
    assert_eq!(model.epoch(), epoch, "a close when closed is nothing");
    model.open_reader(7);
    assert!(model.reader_open());
    assert_eq!(model.reader_generation(), Some(7));
    assert_eq!(model.reader_top(), Some(0));
    assert_eq!(model.epoch(), epoch + 1, "exactly one bump on open");
    model.set_reader_page_rows(10);
    let key = |model: &mut Model, key: Key| model.key(key, &both, false);
    assert_eq!(key(&mut model, Key::Down), Act::Redraw);
    assert_eq!(model.reader_top(), Some(1));
    assert_eq!(key(&mut model, Key::Down), Act::Redraw);
    assert_eq!(key(&mut model, Key::Up), Act::Redraw);
    assert_eq!(model.reader_top(), Some(1));
    model.reader_wheel(false);
    assert_eq!(model.reader_top(), Some(4), "wheel down reads on");
    model.reader_wheel(true);
    assert_eq!(model.reader_top(), Some(1), "wheel up goes back");
    assert_eq!(key(&mut model, Key::PageDown), Act::Redraw);
    assert_eq!(model.reader_top(), Some(11));
    assert_eq!(key(&mut model, Key::PageUp), Act::Redraw);
    assert_eq!(model.reader_top(), Some(1));
    assert_eq!(key(&mut model, Key::Bottom), Act::Redraw);
    assert_eq!(model.reader_top(), Some(usize::MAX), "the frame clamps");
    model.clamp_reader(42);
    assert_eq!(model.reader_top(), Some(42));
    assert_eq!(key(&mut model, Key::Top), Act::Redraw);
    assert_eq!(model.reader_top(), Some(0));
    model.set_reader_top(9);
    assert_eq!(model.reader_top(), Some(9));
    assert_eq!(model.epoch(), epoch + 1, "no scroll or plumbing bumped");
    for swallowed in [Key::Digit(1), Key::Copy, Key::Peek, Key::Tab] {
        assert_eq!(
            key(&mut model, swallowed),
            Act::None,
            "{swallowed:?} dies in the reader",
        );
        assert_eq!(model.reader_top(), Some(9));
    }
    assert_eq!(key(&mut model, Key::Read), Act::Redraw, "r closes too");
    assert!(!model.reader_open());
    assert_eq!(model.reader_top(), None);
    assert_eq!(model.reader_generation(), None);
    assert_eq!(model.epoch(), epoch + 2, "exactly one bump on close");
    model.open_reader(8);
    assert_eq!(model.key(Key::Quit, &both, false), Act::Redraw, "q closes");
    assert!(!model.reader_open());
    model.open_reader(9);
    assert_eq!(model.key(Key::Esc, &both, false), Act::Redraw, "Esc closes");
    assert!(!model.reader_open());
}

#[test]
fn an_open_scroll_close_round_trip_leaves_the_lane_state_alone() {
    let both = fleet(&["api", "web"]);
    let mut model = Model::new(&both);
    let _ = model.key(Key::Digit(2), &both, false);
    let _ = model.key(Key::PageUp, &both, false);
    model.set_turn(Some(7));
    model.set_focus(Some("colead".to_owned()));
    let before = (
        model.selected().map(str::to_owned),
        model.turn(),
        model.focus().map(str::to_owned),
        model.pages(),
        model.scroll_rows(10),
    );
    model.open_reader(3);
    model.set_reader_page_rows(10);
    for _ in 0..5 {
        let _ = model.key(Key::Down, &both, false);
    }
    model.close_reader();
    let after = (
        model.selected().map(str::to_owned),
        model.turn(),
        model.focus().map(str::to_owned),
        model.pages(),
        model.scroll_rows(10),
    );
    assert_eq!(before, after);
    // A restart or a lost selection keeps the open reader, like peek.
    model.open_reader(4);
    model.set_reader_top(5);
    model.restart(&both);
    assert!(model.reader_open());
    assert_eq!(
        (model.reader_generation(), model.reader_top()),
        (Some(4), Some(5)),
    );
    model.reconcile(&fleet(&["api"]));
    assert!(model.reader_open());
    assert_eq!(model.reader_generation(), Some(4));
}

// ---------------------------------------------------------------------------
// draw::draw_reader — the column over a bare buffer
// ---------------------------------------------------------------------------

fn buf(width: u16, height: u16) -> Buffer {
    Buffer::empty(Rect::new(0, 0, width, height))
}

fn row_text(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width)
        .map(|x| buf[(x, y)].symbol())
        .collect::<String>()
        .trim_end()
        .to_owned()
}

fn all_text(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| row_text(buf, y))
        .collect::<Vec<_>>()
        .join("\n")
}

fn text_body(lines: &[&str]) -> Body {
    Body::Text {
        lines: lines.iter().map(|line| (*line).to_owned()).collect(),
        size: 0,
    }
}

/// Gutter columns for `count` lines: the digits, right-aligned, plus one.
fn gutter(count: usize) -> usize {
    count.to_string().len() + 1
}

#[test]
fn draw_paints_header_gutter_and_the_bold_target_and_bounds() {
    let body = text_body(&["one", "two", "three"]);
    let Body::Text { lines, .. } = &body else {
        unreachable!()
    };
    let flow = reader::flow(lines, 40 - gutter(3));
    let view = View {
        path: "/w/src/a.rs",
        line: Some(2),
        body: Some(&body),
        flow: Some(&flow),
        top: 0,
        links: 1,
    };
    let mut cells = buf(40, 10);
    let bounds = draw_reader(&view, None, &mut cells);
    assert_eq!((bounds.visible, bounds.max_back), (9, 0));
    assert!(
        row_text(&cells, 0).starts_with("Read · /w/src/a.rs:2 · rows 1-3/3"),
        "{}",
        row_text(&cells, 0)
    );
    assert_eq!(row_text(&cells, 1), "1 one");
    assert_eq!(row_text(&cells, 2), "2 two");
    assert!(
        cells[(0, 2)].modifier.contains(Modifier::BOLD),
        "the target line's gutter number is bold",
    );
    assert!(!cells[(0, 1)].modifier.contains(Modifier::BOLD));
    assert!(!cells[(0, 3)].modifier.contains(Modifier::BOLD));
}

#[test]
fn draw_windows_from_the_top_and_clamps_past_the_last_page() {
    let owned: Vec<String> = (1..=30).map(|n| format!("L{n:02}")).collect();
    let body = Body::Text {
        lines: owned,
        size: 0,
    };
    let Body::Text { lines, .. } = &body else {
        unreachable!()
    };
    let flow = reader::flow(lines, 40 - gutter(30));
    let at = |top: usize| View {
        path: "/w/a.rs",
        line: None,
        body: Some(&body),
        flow: Some(&flow),
        top,
        links: 1,
    };
    let mut cells = buf(40, 6);
    let bounds = draw_reader(&at(10), None, &mut cells);
    assert_eq!((bounds.visible, bounds.max_back), (5, 25));
    assert!(row_text(&cells, 0).starts_with("Read · /w/a.rs · rows 11-15/30"));
    assert_eq!(row_text(&cells, 1), "11 L11");
    assert_eq!(row_text(&cells, 5), "15 L15");
    let mut cells = buf(40, 6);
    let _ = draw_reader(&at(999), None, &mut cells);
    assert_eq!(row_text(&cells, 1), "26 L26", "a top past the end clamps");
    // Three-digit gutter: right-aligned numbers, one blank, then text.
    let wide: Vec<String> = (1..=120).map(|n| format!("W{n}")).collect();
    let body = Body::Text {
        lines: wide,
        size: 0,
    };
    let Body::Text { lines, .. } = &body else {
        unreachable!()
    };
    let flow = reader::flow(lines, 40 - gutter(120));
    let view = View {
        path: "/w/a.rs",
        line: None,
        body: Some(&body),
        flow: Some(&flow),
        top: 4,
        links: 1,
    };
    let mut cells = buf(40, 6);
    let _ = draw_reader(&view, None, &mut cells);
    assert_eq!(row_text(&cells, 1), "  5 W5");
}

#[test]
fn draw_wrap_continuations_reconstruct_the_line_and_match_flow() {
    let line = "a".repeat(30);
    let body = Body::Text {
        lines: vec![line.clone(), "next".to_owned()],
        size: 0,
    };
    let Body::Text { lines, .. } = &body else {
        unreachable!()
    };
    let flow = reader::flow(lines, 12 - gutter(2));
    let view = View {
        path: "/w/a.rs",
        line: None,
        body: Some(&body),
        flow: Some(&flow),
        top: 0,
        links: 1,
    };
    let mut cells = buf(12, 20);
    let bounds = draw_reader(&view, None, &mut cells);
    let g = gutter(2);
    let body_rows: Vec<String> = (1..20)
        .map(|y| row_text(&cells, y))
        .take_while(|row| !row.is_empty())
        .collect();
    assert_eq!(body_rows.len(), flow.rows(), "draw and flow agree");
    let rebuilt: String = body_rows[..body_rows.len() - 1]
        .iter()
        .map(|row| row[g.min(row.len())..].trim_start().to_owned())
        .collect();
    assert_eq!(rebuilt, line, "no scalar is lost to the wrap");
    assert_eq!(body_rows.last().map(String::as_str), Some("2 next"));
    assert!(
        body_rows[1].starts_with("  "),
        "a continuation row's gutter is blank: {:?}",
        body_rows[1],
    );
    assert_eq!(bounds.max_back, flow.rows().saturating_sub(19));
}

#[test]
fn draw_gap_loading_and_empty_have_no_gutter_and_hostile_bytes_die() {
    let gap = Body::Gap("refused: read /w/x: not found".to_owned());
    let view = View {
        path: "/w/x",
        line: None,
        body: Some(&gap),
        flow: None,
        top: 0,
        links: 1,
    };
    let mut cells = buf(48, 6);
    let bounds = draw_reader(&view, None, &mut cells);
    assert!(row_text(&cells, 0).starts_with("Read · /w/x · not read"));
    assert_eq!(row_text(&cells, 1), "refused: read /w/x: not found");
    assert_eq!(bounds.max_back, 0);
    let loading = View {
        path: "/w/x",
        line: Some(4),
        body: None,
        flow: None,
        top: 0,
        links: 1,
    };
    let mut cells = buf(48, 6);
    let _ = draw_reader(&loading, None, &mut cells);
    assert!(row_text(&cells, 0).starts_with("Read · /w/x:4 · loading"));
    assert_eq!(row_text(&cells, 1), "loading");
    let hostile = Body::Text {
        lines: vec![
            "ok \u{1b}]52;c;AAAA\u{7} end".to_owned(),
            "tab\there".to_owned(),
        ],
        size: 0,
    };
    let Body::Text { lines, .. } = &hostile else {
        unreachable!()
    };
    let flow = reader::flow(lines, 46);
    let view = View {
        path: "/w/a\u{1b}b.rs",
        line: None,
        body: Some(&hostile),
        flow: Some(&flow),
        top: 0,
        links: 1,
    };
    let mut cells = buf(48, 6);
    let _ = draw_reader(&view, None, &mut cells);
    let drawn = all_text(&cells);
    assert!(
        !drawn.contains('\u{1b}') && !drawn.contains('\u{7}'),
        "{drawn:?}"
    );
    assert!(drawn.contains("ok") && drawn.contains('\u{fffd}'));
    assert!(row_text(&cells, 0).starts_with("Read · /w/a\u{fffd}b.rs"));
}

#[test]
fn draw_is_total_on_a_one_row_buffer_and_an_empty_file() {
    let body = text_body(&["one", "two"]);
    let flow = reader::flow(text_lines(&body), 18);
    let view = View {
        path: "/w/a.rs",
        line: None,
        body: Some(&body),
        flow: Some(&flow),
        top: 0,
        links: 1,
    };
    let mut cells = buf(20, 1);
    let bounds = draw_reader(&view, None, &mut cells);
    assert_eq!(bounds.visible, 0);
    assert!(row_text(&cells, 0).starts_with("Read · /w/a.rs"));
    let empty = Body::Text {
        lines: Vec::new(),
        size: 0,
    };
    let empty_flow = reader::flow(&[], 38);
    let view = View {
        path: "/w/a.rs",
        line: None,
        body: Some(&empty),
        flow: Some(&empty_flow),
        top: 0,
        links: 1,
    };
    let mut cells = buf(40, 4);
    let _ = draw_reader(&view, None, &mut cells);
    assert!(row_text(&cells, 0).starts_with("Read · /w/a.rs · rows 0-0/0"));
    assert_eq!(row_text(&cells, 1), "");
    // No reachable target, no bold gutter anywhere.
    for line in [Some(0), Some(99)] {
        let view = View {
            path: "/w/a.rs",
            line,
            body: Some(&body),
            flow: Some(&flow),
            top: 0,
            links: 1,
        };
        let mut cells = buf(20, 5);
        let _ = draw_reader(&view, None, &mut cells);
        for y in 1..5 {
            assert!(!cells[(0, y)].modifier.contains(Modifier::BOLD), "{line:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// App — r on the marked turn, the ask, the answer, the return
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

/// An app on session `api` (work dir `/w`, home `/h`) whose lane holds one
/// turn saying `body`. The turn is NOT marked; marking is each case's act.
fn reading_app(body: &str) -> (App, Receiver<Request>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = App::new(Some("api".to_owned()), None, None, Some(tx));
    (app.fleeted, app.loading) = (true, false);
    let fleet = fleet(&["api"]);
    app.model = Model::new(&fleet);
    app.fleet = fleet;
    let mut entry = SessionEntry::new("api", Status::Running);
    entry.work_dir = Some("/w".to_owned());
    app.world.sessions.push(entry.clone());
    app.overview = super::overview::of(
        &entry,
        None,
        Some(Path::new("/h")),
        Timestamp::from_epoch(1_759_500_600),
    );
    app.lane = Rc::new(Lane {
        items: vec![said(body)],
        coverage: vec![],
    });
    (app, rx)
}

fn marked(app: &mut App) -> u64 {
    let key = app.keys()[0];
    app.model.set_turn(Some(key));
    key
}

fn pressed(app: &mut App, bytes: &[u8]) -> Option<bool> {
    take_keys(
        app,
        vec![(input::Key::Text(bytes.to_vec()), Instant::now())],
    )
}

fn framed(app: &mut App) -> String {
    let mut cells = buf(160, 45);
    app.frame(&mut cells);
    all_text(&cells)
}

fn read_ask(rx: &Receiver<Request>) -> super::loader::ReadAsk {
    let Ok(Request::Read(Some(ask))) = rx.try_recv() else {
        panic!("the open asks for one read");
    };
    *ask
}

#[test]
fn r_opens_the_first_file_link_of_the_marked_turn_and_esc_returns() {
    let (mut app, rx) = reading_app("see https://x.io/a and src/b.rs:50 then docs/c.md");
    let mark = marked(&mut app);
    assert_eq!(pressed(&mut app, b"r"), Some(true));
    assert!(app.model.reader_open());
    let ask = read_ask(&rx);
    assert_eq!(ask.generation, 1);
    assert_eq!(ask.path, "/w/src/b.rs");
    assert_eq!(ask.line, Some(50));
    assert_eq!(ask.links, 2, "both file links of the turn are counted");
    assert_eq!(
        ask.base,
        Base {
            dir: Some("/w".to_owned()),
            home: Some("/h".to_owned()),
        },
    );
    assert!(rx.try_recv().is_err(), "one ask per open");
    let lines: Vec<String> = (1..=100).map(|n| format!("fileline{n:03}")).collect();
    app.answer(Answer::Read {
        generation: 1,
        body: Body::Text { lines, size: 0 },
    });
    let shown = framed(&mut app);
    assert!(shown.contains("Read · /w/src/b.rs:50"), "{shown}");
    assert!(shown.contains("fileline050"), "the jump lands");
    assert!(
        shown.contains("fileline047"),
        "three lines of context above"
    );
    assert!(
        !shown.contains("fileline046"),
        "above the context is scrolled away"
    );
    assert!(!shown.contains("fileline001"));
    assert_eq!(
        take_keys(&mut app, vec![(input::Key::Escape, Instant::now())]),
        Some(true),
        "Esc closes",
    );
    assert!(!app.model.reader_open());
    assert!(matches!(rx.try_recv(), Ok(Request::Read(None))));
    assert!(rx.try_recv().is_err(), "one close per close");
    let back = framed(&mut app);
    assert!(back.contains("src/b.rs:50"), "the lane is back");
    assert!(!back.contains("fileline050"));
    assert_eq!(app.model.turn(), Some(mark), "the mark never moved");
}

#[test]
fn r_without_a_mark_or_a_file_link_says_so_and_asks_nothing() {
    let (mut app, rx) = reading_app("see src/b.rs now");
    let _ = pressed(&mut app, b"r");
    assert!(!app.model.reader_open() && rx.try_recv().is_err());
    assert!(
        framed(&mut app).contains("no turn marked - click one or press [ ]"),
        "copied()'s words",
    );
    app.model.set_turn(Some(9_999_999));
    let _ = pressed(&mut app, b"r");
    assert!(!app.model.reader_open() && rx.try_recv().is_err());
    assert_eq!(app.model.turn(), None, "the stale mark is cleared");
    assert!(framed(&mut app).contains("that turn changed or left the lane - mark it again"),);
    let (mut app, rx) = reading_app("only https://example.com/x here");
    let _ = marked(&mut app);
    let _ = pressed(&mut app, b"r");
    assert!(!app.model.reader_open() && rx.try_recv().is_err());
    assert!(framed(&mut app).contains("no file link in that turn"));
    let (mut app, rx) = reading_app("nothing linked at all");
    let _ = marked(&mut app);
    let _ = pressed(&mut app, b"r");
    assert!(!app.model.reader_open() && rx.try_recv().is_err());
    assert!(framed(&mut app).contains("no file link in that turn"));
}

#[test]
fn generations_mint_per_open_and_a_stale_answer_paints_nothing() {
    let (mut app, rx) = reading_app("open src/b.rs please");
    let _ = marked(&mut app);
    // An answer before any open is dropped.
    app.answer(Answer::Read {
        generation: 1,
        body: Body::Text {
            lines: vec!["BEFOREOPEN".to_owned()],
            size: 0,
        },
    });
    let _ = pressed(&mut app, b"r");
    assert_eq!(read_ask(&rx).generation, 1);
    app.answer(Answer::Read {
        generation: 9,
        body: Body::Text {
            lines: vec!["WRONGGENERATION".to_owned()],
            size: 0,
        },
    });
    assert!(!framed(&mut app).contains("WRONGGENERATION"));
    assert!(!framed(&mut app).contains("BEFOREOPEN"));
    assert_eq!(pressed(&mut app, b"q"), Some(true), "q closes");
    assert!(matches!(rx.try_recv(), Ok(Request::Read(None))));
    let _ = pressed(&mut app, b"r");
    assert_eq!(read_ask(&rx).generation, 2, "a reopen is a new generation");
    app.answer(Answer::Read {
        generation: 1,
        body: Body::Text {
            lines: vec!["STALEFILE".to_owned()],
            size: 0,
        },
    });
    app.answer(Answer::Read {
        generation: 2,
        body: Body::Gap("refused: read /w/src/b.rs: not found".to_owned()),
    });
    let shown = framed(&mut app);
    assert!(shown.contains("refused: read /w/src/b.rs: not found"));
    assert!(!shown.contains("STALEFILE"));
    assert!(app.model.reader_open(), "a refusal stays in the panel");
    assert_eq!(pressed(&mut app, b"r"), Some(true), "r closes again");
    assert!(!app.model.reader_open());
    assert!(matches!(rx.try_recv(), Ok(Request::Read(None))));
    // An answer after the close leaves nothing behind.
    app.answer(Answer::Read {
        generation: 2,
        body: Body::Text {
            lines: vec!["AFTERCLOSE".to_owned()],
            size: 0,
        },
    });
    assert!(!framed(&mut app).contains("AFTERCLOSE"));
}

#[test]
fn the_reader_swallows_browse_keys_whole_until_closed() {
    let (mut app, rx) = reading_app("open src/b.rs please");
    let _ = marked(&mut app);
    let _ = pressed(&mut app, b"r");
    let _ = read_ask(&rx);
    app.answer(Answer::Read {
        generation: 1,
        body: Body::Text {
            lines: vec!["onlyline".to_owned()],
            size: 0,
        },
    });
    let _ = pressed(&mut app, b"i");
    assert!(
        app.model.reader_open(),
        "i does not compose under the reader"
    );
    assert!(!app.composing());
    let _ = pressed(&mut app, b"1");
    assert!(app.model.reader_open());
    assert_eq!(app.model.selected(), Some("api"));
    assert!(framed(&mut app).contains("onlyline"));
    assert!(rx.try_recv().is_err(), "no second ask, no close");
}

/// The App half of the write-nothing invariant (lead IMPORTANT-1): after
/// a full open -> scroll -> close round trip, every surface through
/// which this app could reach a session store stands exactly as it
/// started. The journal is written only by asks the loop flies from
/// `queued`/`flight`; `console.draft` only under an `entered` writer
/// from an incarnation's `inputs` draft; the clipboard and the pane
/// view are the other two ways a key leaves the app. All empty, plus
/// exactly the two loader requests on the channel, means no record and
/// no draft byte can have been produced — the same fact the loader
/// spec proves on disk, seen at the boundary that would write.
#[test]
fn a_reader_round_trip_reaches_no_write_surface() {
    let (mut app, rx) = reading_app("open src/b.rs please");
    let _ = marked(&mut app);
    let notices = app.notices.len();
    let _ = pressed(&mut app, b"r");
    let _ = read_ask(&rx);
    let lines: Vec<String> = (1..=40).map(|n| format!("row{n:02}")).collect();
    app.answer(Answer::Read {
        generation: 1,
        body: Body::Text { lines, size: 0 },
    });
    for bytes in [&b"j"[..], b"G", b"g", b"k"] {
        let _ = pressed(&mut app, bytes);
    }
    let _ = take_keys(
        &mut app,
        vec![
            (input::Key::PageDown, Instant::now()),
            (input::Key::PageUp, Instant::now()),
        ],
    );
    let _ = framed(&mut app);
    assert!(app.model.reader_open(), "still reading after the scrolls");
    let _ = take_keys(&mut app, vec![(input::Key::Escape, Instant::now())]);
    assert!(!app.model.reader_open());
    assert!(matches!(rx.try_recv(), Ok(Request::Read(None))));
    assert!(
        rx.try_recv().is_err(),
        "the loader heard open then close, only"
    );
    assert!(app.queued.is_empty(), "no ask was ever queued to a journal");
    assert!(app.flight.is_none(), "no ask was ever flown");
    assert!(app.deferred.is_empty());
    assert!(app.entered.is_none(), "no writer ever entered");
    assert!(app.writer.is_none());
    assert!(app.held.is_none());
    assert!(
        app.inputs.values().all(|input| input.draft().is_empty()),
        "no draft exists that a loop could persist",
    );
    assert!(app.draft.is_empty());
    assert!(app.clip.is_none(), "nothing was shipped to a clipboard");
    assert!(app.pane_view.is_none(), "no terminal was handed over");
    assert_eq!(app.notices.len(), notices, "ae recorded no line of its own");
    assert!(!app.composing());
}

/// A reading app with drawn seats and no tmux server: session `api`, a
/// lead and a colead on its roster, one lane turn saying `body`. The
/// fixture the app tests' seat-click cases build, outside tmux.
fn bound_reading_app(body: &str) -> (App, Receiver<Request>) {
    use crate::console::needs::SeatRef;

    const ID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = App::new(Some("api".to_owned()), None, None, Some(tx));
    (app.fleeted, app.loading) = (true, false);
    app.dirs.insert("api".to_owned(), PathBuf::from("/tmp/api"));
    app.ids.insert("api".to_owned(), ID.to_owned());
    let mut entry = SessionEntry::new("api", Status::Running);
    entry.work_dir = Some("/w".to_owned());
    app.world.sessions.push(entry);
    let roster: Vec<SeatRef> = [("main", "lead"), ("worker.0", "colead")]
        .iter()
        .map(|(slot, name)| SeatRef {
            slot: (*slot).into(),
            name: (*name).into(),
        })
        .collect();
    app.facts.insert(
        "api".to_owned(),
        Facts::Seats {
            id: ID.to_owned(),
            agents: roster
                .iter()
                .map(|seat| crate::tmux::PickerAgent {
                    name: seat.name.clone(),
                    profile: "sol61x".into(),
                    state: "working".into(),
                    pane: "%7".into(),
                    client: String::new(),
                    model: String::new(),
                    effort: String::new(),
                    drift: false,
                })
                .collect(),
        },
    );
    app.shown.insert(
        "api".to_owned(),
        super::Shown {
            id: ID.to_owned(),
            seq: 1,
            lane: Rc::new(Lane {
                items: vec![said(body)],
                coverage: vec![],
            }),
            roster: Some(roster),
        },
    );
    let fleet = fleet(&["api"]);
    app.model = Model::new(&fleet);
    app.fleet = fleet;
    app.model.set_focus(Some("colead".to_owned()));
    app.show();
    (app, rx)
}

/// The click on the drawn seat target `seat` in `shown`.
fn seat_cell(shown: &str, app: &App, seat: &str) -> super::Mouse {
    let click = |column: usize, row: usize| super::Mouse {
        kind: super::MouseKind::Click,
        column: u16::try_from(column).expect("frame column"),
        row: u16::try_from(row).expect("frame row"),
    };
    shown
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            line.match_indices(seat).find_map(|(byte, _)| {
                let column = line[..byte].chars().count();
                matches!(
                    app.layout.hit(click(column, row)),
                    Some(super::draw::Hit::Seat(name)) if name == seat
                )
                .then_some(click(column, row))
            })
        })
        .unwrap_or_else(|| panic!("drawn seat target {seat}"))
}

#[test]
fn a_seat_click_outside_tmux_closes_the_reader_before_the_pane_view() {
    let (mut app, rx) = bound_reading_app("open src/b.rs please");
    let _ = marked(&mut app);
    let _ = pressed(&mut app, b"r");
    let _ = read_ask(&rx);
    app.answer(Answer::Read {
        generation: 1,
        body: Body::Text {
            lines: vec!["onlyline".to_owned()],
            size: 0,
        },
    });
    let shown = framed(&mut app);
    assert!(app.model.reader_open());
    assert!(shown.contains("Read · /w/src/b.rs"), "{shown}");
    let at = seat_cell(&shown, &app, "colead");
    assert!(
        app.mouse_made(at, Instant::now()),
        "the seat click is taken"
    );
    let ask = app.pane_view.as_ref().expect("the pane view is queued");
    assert_eq!(ask.seat.name, "colead");
    assert!(
        !app.model.reader_open(),
        "the reader closed before the pane view",
    );
    assert!(matches!(rx.try_recv(), Ok(Request::Read(None))));
    assert!(rx.try_recv().is_err(), "exactly one close");
    // The loop takes the ask and hands the terminal over; coming back
    // does not reopen the reader or ask again.
    let _ = app.pane_view.take();
    app.resumed_at = Some(Instant::now());
    let back = framed(&mut app);
    assert!(!app.model.reader_open(), "the return does not reopen it");
    assert!(rx.try_recv().is_err(), "no second read was asked");
    assert!(back.contains("src/b.rs"), "the lane is what came back");
    assert!(!back.contains("Read · /w/src/b.rs"));
}
