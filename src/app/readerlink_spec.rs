//! Acceptance cases: a file link whose target a browser opens (HTML, SVG, a
//! raster image, a PDF) is NOT read by the reader. The reader panel says what
//! the file is and the one action that opens it — the terminal's own link,
//! with the modifier the Links hint names — instead of raw markup or a
//! `binary file (NUL byte)` refusal.
//!
//! Oracle: the requirement (human, 2026-11-06: a clear "open in browser" note
//! instead of raw HTML/SVG source or a PNG refusal; the existing external-open
//! path, the shortcut as verified) and the platform literal docs/app.md marks
//! verified (`Shift+Cmd+click to open link` on macOS, Ghostty 1.3.1) — never
//! the code under test.
//!
//! The note replaces the read: the classification is by extension, before the
//! file door, so no ask reaches the loader (ae reads nothing, starts nothing).
//! Everything else keeps today's behaviour: Markdown, plain text, other
//! extensions and the refusals still go to the loader exactly as before.

use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::fleet::{Counts, Fleet, Line2, Row};
use super::loader::{Answer, ReadAsk, Request};
use super::model::Model;
use super::reader::Body;
use super::{App, take_keys};
use crate::console::input;
use crate::console::lane::{Item, Kind, Lane};
use crate::digest::{SessionEntry, Status};
use crate::theme::Mark;
use crate::time::Timestamp;

const MICROS: i64 = 1_759_500_000_000_000;
const WIDTH: u16 = 160;
const HEIGHT: u16 = 32;

/// The hint the Links section documents as verified for this platform:
/// Ghostty on macOS, `Shift+Cmd+click` (docs/app.md §Links). Written out
/// here, not read from the code under test.
#[cfg(target_os = "macos")]
const OPEN: &str = "Shift+Cmd+click to open link";
#[cfg(not(target_os = "macos"))]
const OPEN: &str = "Ctrl+click or Shift+click to open link";

// ---------------------------------------------------------------------------
// Rig
// ---------------------------------------------------------------------------

fn fleet() -> Fleet {
    Fleet {
        rows: vec![Row {
            name: "api".to_owned(),
            index: 1,
            mark: Mark::Working,
            needy: false,
            unanswered: None,
            counts: Counts::Unknown,
            line2: Line2::NoGoal,
            home: true,
        }],
        home: Some("api".to_owned()),
    }
}

/// An app on session `api` (work dir `/w`, home `/h`) whose one marked turn
/// says `body`.
fn marked_app(body: &str) -> (App, Receiver<Request>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = App::new(Some("api".to_owned()), None, None, Some(tx));
    (app.fleeted, app.loading) = (true, false);
    let fleet = fleet();
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
        items: vec![Item {
            micros: MICROS,
            kind: Kind::Said {
                who: "lead".to_owned(),
            },
            body: body.to_owned(),
            record: None,
        }],
        coverage: vec![],
    });
    let key = app.keys()[0];
    app.model.set_turn(Some(key));
    (app, rx)
}

fn pressed(app: &mut App, bytes: &[u8]) {
    let taken = take_keys(
        app,
        vec![(input::Key::Text(bytes.to_vec()), Instant::now())],
    );
    assert!(taken.is_some(), "{bytes:?} never quits");
}

fn click(app: &mut App, (x, y): (u16, u16)) {
    for kind in [input::MouseKind::Click, input::MouseKind::Release] {
        let mouse = input::Mouse {
            kind,
            column: x,
            row: y,
        };
        let taken = take_keys(app, vec![(input::Key::Mouse(mouse), Instant::now())]);
        assert!(taken.is_some(), "a mouse report never quits");
    }
}

fn frame_of(app: &mut App) -> Buffer {
    let mut cells = Buffer::empty(Rect::new(0, 0, WIDTH, HEIGHT));
    app.frame(&mut cells);
    cells
}

fn whole(cells: &Buffer) -> String {
    (0..cells.area.height)
        .map(|y| {
            (0..cells.area.width)
                .map(|x| cells[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The drawn frame, every row's trailing blanks trimmed.
fn shown(app: &mut App) -> String {
    let cells = frame_of(app);
    whole(&cells)
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The reader header the next frame draws.
#[track_caller]
fn header(app: &mut App) -> String {
    let frame = shown(app);
    frame
        .lines()
        .find_map(|line| line.find("Read · ").map(|at| line[at..].to_owned()))
        .unwrap_or_else(|| panic!("no reader header:\n{frame}"))
}

/// The first cell of the first drawn occurrence of `link` in the lane.
#[track_caller]
fn cell_of(app: &mut App, link: &str) -> (u16, u16) {
    let cells = frame_of(app);
    let lane = app.layout.lane_area().expect("the lane is drawn");
    for y in lane.y..lane.y + lane.height {
        let row: String = (lane.x..lane.x + lane.width)
            .map(|x| cells[(x, y)].symbol())
            .collect();
        if let Some(at) = row.find(link) {
            let x = u16::try_from(row[..at].chars().count()).expect("column");
            return (lane.x + x, y);
        }
    }
    panic!("{link} not drawn in the lane:\n{}", whole(&cells));
}

/// What the app asked of its loader since the last call: the reads, and the
/// number of closes (`Read(None)`). Any other request fails the case by name.
#[track_caller]
fn effects(rx: &Receiver<Request>) -> (Vec<ReadAsk>, usize) {
    let (mut reads, mut closes) = (Vec::new(), 0);
    while let Ok(request) = rx.try_recv() {
        match request {
            Request::Read(Some(ask)) => reads.push(*ask),
            Request::Read(None) => closes += 1,
            Request::Writing(_) => panic!("a file link changed the writing"),
            Request::Focus(focus) => panic!("a file link moved the focus to {focus:?}"),
            _ => panic!("a file link asked the loader for something other than a read"),
        }
    }
    (reads, closes)
}

fn text_answer(app: &mut App, generation: u64, src: &str) {
    app.answer(Answer::Read {
        generation,
        body: Body::Text {
            lines: src.lines().map(str::to_owned).collect(),
            size: 0,
        },
    });
}

/// `r` on the marked turn, from a fresh rig saying `body`.
fn opened_by_key(body: &str) -> (App, Receiver<Request>) {
    let (mut app, rx) = marked_app(body);
    pressed(&mut app, b"r");
    (app, rx)
}

/// The note a visual file's reader panel must carry, for the label `kind`.
#[track_caller]
fn assert_note(frame: &str, kind: &str) {
    assert!(
        frame.contains(kind),
        "the note names the kind {kind:?}:\n{frame}"
    );
    assert!(
        frame.contains(OPEN),
        "the note carries the verified shortcut {OPEN:?}:\n{frame}"
    );
    assert!(
        frame.contains("browser"),
        "the note says it opens in the browser:\n{frame}"
    );
}

// ---------------------------------------------------------------------------
// The invariant: no read, a note naming the kind and the open action
// ---------------------------------------------------------------------------

/// (extension as written in the link, label the note names). The list is the
/// files a browser opens natively whose reader answer today is either raw
/// markup (HTML, SVG) or a binary refusal (images, PDF).
const KINDS: [(&str, &str); 13] = [
    ("html", "HTML page"),
    ("htm", "HTML page"),
    ("xhtml", "HTML page"),
    ("svg", "SVG image"),
    ("png", "image"),
    ("jpg", "image"),
    ("jpeg", "image"),
    ("gif", "image"),
    ("webp", "image"),
    ("avif", "image"),
    ("bmp", "image"),
    ("ico", "image"),
    ("pdf", "PDF document"),
];

#[test]
fn a_click_on_a_visual_file_link_asks_the_loader_for_nothing_and_says_how_to_open_it() {
    for (ext, kind) in KINDS {
        let link = format!("docs/page.{ext}");
        let (mut app, rx) = marked_app(&format!("see {link} end"));
        let at = cell_of(&mut app, &link);
        click(&mut app, at);
        let (reads, _) = effects(&rx);
        assert!(
            reads.is_empty(),
            "{ext}: the file is never read: {} ask(s)",
            reads.len()
        );
        assert!(app.model.reader_open(), "{ext}: the reader opens");
        let head = header(&mut app);
        assert!(
            head.starts_with(&format!("Read · /w/{link} · not read")),
            "{ext}: {head}"
        );
        let frame = shown(&mut app);
        assert_note(&frame, kind);
        assert!(
            !frame.contains("loading"),
            "{ext}: nothing is awaited:\n{frame}"
        );
    }
}

#[test]
fn r_on_a_visual_file_link_asks_the_loader_for_nothing_and_says_how_to_open_it() {
    for (ext, kind) in KINDS {
        let link = format!("docs/page.{ext}");
        let (mut app, rx) = opened_by_key(&format!("see {link} end"));
        let (reads, _) = effects(&rx);
        assert!(
            reads.is_empty(),
            "{ext}: the file is never read: {} ask(s)",
            reads.len()
        );
        assert!(app.model.reader_open(), "{ext}: the reader opens");
        assert_note(&shown(&mut app), kind);
    }
}

#[test]
fn the_extension_is_matched_without_regard_to_case() {
    for link in [
        "docs/PAGE.HTML",
        "docs/Shot.PNG",
        "docs/a.JpEg",
        "docs/B.Pdf",
    ] {
        let (mut app, rx) = opened_by_key(&format!("see {link} end"));
        let (reads, _) = effects(&rx);
        assert!(
            reads.is_empty(),
            "{link}: never read: {} ask(s)",
            reads.len()
        );
        assert!(
            shown(&mut app).contains(OPEN),
            "{link}: the note carries the shortcut"
        );
    }
}

#[test]
fn a_line_suffix_does_not_hide_the_extension() {
    let (mut app, rx) = opened_by_key("see docs/page.html:12:3 end");
    let (reads, _) = effects(&rx);
    assert!(reads.is_empty(), "never read: {} ask(s)", reads.len());
    let head = header(&mut app);
    assert!(
        head.starts_with("Read · /w/docs/page.html:12 · not read"),
        "{head}"
    );
    assert_note(&shown(&mut app), "HTML page");
}

#[test]
fn the_note_is_the_whole_answer_no_source_no_refusal_no_gutter() {
    let (mut app, _rx) = opened_by_key("see docs/page.svg end");
    let frame = shown(&mut app);
    assert!(!frame.contains("refused"), "no refusal:\n{frame}");
    assert!(!frame.contains("binary file"), "no binary text:\n{frame}");
    assert!(!frame.contains("rows "), "no row count:\n{frame}");
    let head = header(&mut app);
    assert!(head.contains(" · not read"), "{head}");
}

#[test]
fn the_note_survives_a_narrow_chat_column() {
    let (mut app, _rx) = opened_by_key("see docs/page.pdf end");
    let mut cells = Buffer::empty(Rect::new(0, 0, 70, HEIGHT));
    app.frame(&mut cells);
    let frame = whole(&cells);
    assert!(frame.contains("PDF document"), "{frame}");
    // The shortcut is wrapped across rows at this width; every word is drawn.
    for word in OPEN.split(' ') {
        assert!(frame.contains(word), "{word:?} is drawn:\n{frame}");
    }
}

// ---------------------------------------------------------------------------
// n / p reach a visual link: same note, no raw dump; reads resume around it
// ---------------------------------------------------------------------------

#[test]
fn n_and_p_step_through_a_visual_link_without_reading_it() {
    let (mut app, rx) = opened_by_key("a src/a.rs b docs/page.html c docs/shot.png d src/b.rs e");
    let (reads, _) = effects(&rx);
    assert_eq!(reads.len(), 1, "the first link is a source file");
    assert_eq!(reads[0].path, "/w/src/a.rs");
    text_answer(&mut app, reads[0].generation, "fn a() {}");

    pressed(&mut app, b"n");
    let (reads, _) = effects(&rx);
    assert!(reads.is_empty(), "n onto the HTML link reads nothing");
    let head = header(&mut app);
    assert!(
        head.starts_with("Read · /w/docs/page.html · not read"),
        "{head}"
    );
    assert!(head.ends_with(" · 2 of 4 links"), "{head}");
    let frame = shown(&mut app);
    assert_note(&frame, "HTML page");
    assert!(
        !frame.contains("fn a()"),
        "the earlier file is gone:\n{frame}"
    );

    pressed(&mut app, b"n");
    let (reads, _) = effects(&rx);
    assert!(reads.is_empty(), "n onto the image link reads nothing");
    let head = header(&mut app);
    assert!(head.ends_with(" · 3 of 4 links"), "{head}");
    assert_note(&shown(&mut app), "image");

    pressed(&mut app, b"n");
    let (reads, _) = effects(&rx);
    assert_eq!(reads.len(), 1, "the next source file is read again");
    assert_eq!(reads[0].path, "/w/src/b.rs");
    text_answer(&mut app, reads[0].generation, "fn b() {}");
    assert!(shown(&mut app).contains("fn b()"));

    pressed(&mut app, b"p");
    let (reads, _) = effects(&rx);
    assert!(reads.is_empty(), "p back onto the image link reads nothing");
    assert_note(&shown(&mut app), "image");
}

#[test]
fn stepping_onto_a_visual_link_cancels_the_unread_ask_and_drops_a_late_answer() {
    let (mut app, rx) = opened_by_key("a src/a.rs b docs/page.html c");
    let (reads, _) = effects(&rx);
    let stale = reads[0].generation;

    pressed(&mut app, b"n");
    let (reads, closes) = effects(&rx);
    assert!(reads.is_empty(), "no ask for the visual file");
    assert_eq!(
        closes, 1,
        "the unread ask of the file left behind is dropped"
    );

    // The earlier file's answer lands late: it must not replace the note.
    text_answer(&mut app, stale, "fn late() {}");
    let frame = shown(&mut app);
    assert!(
        !frame.contains("fn late()"),
        "a stale answer is dropped:\n{frame}"
    );
    assert_note(&frame, "HTML page");
}

#[test]
fn a_visual_link_in_a_reader_opened_from_another_turn_still_notes_not_reads() {
    let (mut app, rx) = marked_app("only docs/page.html here");
    pressed(&mut app, b"r");
    let (reads, _) = effects(&rx);
    assert!(reads.is_empty());
    assert!(app.model.reader_open());
    // One link: the header names no place.
    let head = header(&mut app);
    assert!(!head.contains(" links"), "{head}");
    // Esc returns to the lane as for any reader.
    let taken = take_keys(&mut app, vec![(input::Key::Escape, Instant::now())]);
    assert!(taken.is_some());
    assert!(!app.model.reader_open(), "Esc closes the note");
}

// ---------------------------------------------------------------------------
// Controls: everything else is read exactly as before
// ---------------------------------------------------------------------------

#[test]
fn files_the_reader_shows_are_still_asked_for() {
    for link in [
        "docs/notes.md",
        "docs/notes.txt",
        "src/main.rs",
        "docs/data.json",
        "docs/Makefile",
        // An extension that merely contains or ends a visual one, or a
        // visual-looking directory or stem, is not a visual file.
        "docs/page.htmlx",
        "docs/image.png.txt",
        "docs/pdf",
        "docs/a.pngx",
        "docs/page.html.d/readme.txt",
    ] {
        let (mut app, rx) = opened_by_key(&format!("see {link} end"));
        let (reads, _) = effects(&rx);
        assert_eq!(reads.len(), 1, "{link}: one read is asked for");
        assert_eq!(reads[0].path, format!("/w/{link}"));
        assert!(app.model.reader_open());
        let frame = shown(&mut app);
        assert!(!frame.contains(OPEN), "{link}: no open note:\n{frame}");
        assert!(
            header(&mut app).ends_with("loading"),
            "{link}: awaits its answer"
        );
    }
}

#[test]
fn a_binary_file_the_note_does_not_cover_keeps_its_refusal() {
    let (mut app, rx) = opened_by_key("see docs/blob.bin end");
    let (reads, _) = effects(&rx);
    assert_eq!(reads.len(), 1, "an unlisted binary is still read");
    app.answer(Answer::Read {
        generation: reads[0].generation,
        body: Body::Gap("refused: read /w/docs/blob.bin: binary file (NUL byte)".to_owned()),
    });
    let frame = shown(&mut app);
    assert!(frame.contains("binary file (NUL byte)"), "{frame}");
    assert!(!frame.contains(OPEN), "{frame}");
}

// ---------------------------------------------------------------------------
// The pure classifier and the words
// ---------------------------------------------------------------------------

#[test]
fn the_classifier_names_each_kind_and_ignores_everything_else() {
    use super::reader::visual;
    for (ext, kind) in KINDS {
        assert_eq!(visual(&format!("/w/docs/x.{ext}")), Some(kind), "{ext}");
        assert_eq!(
            visual(&format!("/w/docs/x.{}", ext.to_ascii_uppercase())),
            Some(kind),
            "{ext} upper"
        );
    }
    for path in [
        "/w/docs/x.md",
        "/w/docs/x.txt",
        "/w/docs/x.htmlx",
        "/w/docs/x.html.txt",
        "/w/docs/html",
        "/w/docs/.html",
        "/w/docs/x.html/readme",
        "/w/docs/x.",
        "/w/docs/x",
        "",
    ] {
        assert_eq!(visual(path), None, "{path:?}");
    }
}

#[test]
fn the_words_name_the_kind_the_shortcut_and_the_browser() {
    use super::reader::visual_note;
    for kind in ["HTML page", "SVG image", "image", "PDF document"] {
        let note = visual_note(kind);
        assert!(note.starts_with(&format!("{kind}: ")), "{note}");
        assert!(note.contains(OPEN), "{note}");
        assert!(note.contains("Esc"), "the way back to the link: {note}");
        assert!(note.contains("browser"), "{note}");
        assert!(!note.contains('\n'), "one line, the panel wraps it: {note}");
    }
}
