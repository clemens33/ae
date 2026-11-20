//! Frozen acceptance of slice composercopy, the app side.
//!
//! Author composercopy-rev (Anthropic), frozen against main 201244e57 and the
//! interface sent to the driver on 2026-11-20. The driver installs this file
//! byte-identically as `src/l5_ui/app/tests/composercopy_app_spec.rs`, wired by
//! `mod composercopy_app_spec;` among the other `mod` lines of
//! `src/l5_ui/app/tests.rs`; it uses that module's fixtures through `super::*`.
//!
//! Oracles: the owner wish (select draft text and copy it while still
//! typing), the brief's objective 1 and 2, docs/app.md "Selecting text" (the
//! drawn cells, row by row, cut at the last character, a head outside the
//! region pinned to its edge, Esc with a selection only clears it), and the
//! copy answer `ship` gives outside tmux (`sent N lines to the terminal
//! clipboard - unconfirmed`), which these fixtures reach because they run with
//! no tmux server. Nothing here mocks the selection or the copy.

use super::*;
use ratatui_core::style::Modifier;

const UNTMUXED: &str = "to the terminal clipboard - unconfirmed";

fn shown(app: &mut App) -> Buffer {
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
    app.frame(&mut buf);
    buf
}

fn row_text(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

/// The one cell where `needle` starts; the fixture keeps it unique on screen.
fn cell(buf: &Buffer, needle: &str) -> (u16, u16) {
    let hits: Vec<(u16, u16)> = (0..buf.area.height)
        .filter_map(|y| {
            let row = row_text(buf, y);
            row.find(needle).map(|byte| {
                let x = u16::try_from(row[..byte].chars().count()).expect("column");
                (x, y)
            })
        })
        .collect();
    assert_eq!(hits.len(), 1, "{needle:?} drawn once: {hits:?}");
    hits[0]
}

fn press(app: &mut App, kind: MouseKind, at: (u16, u16)) {
    let _ = app.mouse(Mouse {
        kind,
        column: at.0,
        row: at.1,
    });
}

fn key(app: &mut App, key: Key) {
    let _ = take_keys(app, vec![(key, Instant::now())]);
}

/// `root`'s home, writing a draft of `lines`, framed once.
fn drafting(root: &Root, lines: &[&str]) -> App {
    let (mut app, _reader) = housed(root);
    let _ = writing(&mut app);
    for (at, line) in lines.iter().enumerate() {
        if at > 0 {
            key(&mut app, Key::Newline);
        }
        key(&mut app, Key::Text(line.as_bytes().to_vec()));
    }
    let _ = shown(&mut app);
    app
}

/// A drag from `from` to `to`, its release there, the frame after it and the
/// shipping that frame owes. The selection is checked to be the composer's,
/// and nothing reaches the clipboard before the shipping. The frame the drag
/// drew is returned.
fn drag_copy(app: &mut App, from: (u16, u16), to: (u16, u16)) -> Buffer {
    press(app, MouseKind::Click, from);
    press(app, MouseKind::Drag, to);
    let lit = shown(app);
    let held = app.select.as_ref().expect("the drag holds a selection");
    assert_eq!(held.sel.panel(), select::Panel::Composer, "the composer's");
    press(app, MouseKind::Release, to);
    let _ = shown(app);
    assert_eq!(app.clip, None, "nothing is copied before the shipping");
    assert!(app.ship_staged(), "the release ships once, after its frame");
    assert!(!app.ship_staged(), "and only once");
    lit
}

fn flash(app: &App) -> String {
    app.flash
        .as_ref()
        .map(|(line, _)| line.clone())
        .unwrap_or_default()
}

/// A1: Alt+y while writing copies the whole draft, says so the way `y` does,
/// and sends nothing: the draft, the writing and the queue stay.
#[test]
fn alt_y_copies_the_whole_draft_while_writing() {
    let root = Root::new("cc-alty");
    let mut app = drafting(&root, &["qzfirst line", "  qzsecond"]);
    let before = drafted(&app);
    key(&mut app, Key::CopyDraft);
    assert_eq!(app.clip.as_deref(), Some("qzfirst line\n  qzsecond"));
    assert_eq!(flash(&app), format!("sent 2 lines {UNTMUXED}"));
    assert_eq!(drafted(&app), before, "the draft is untouched");
    assert!(app.composing(), "writing stays on");
    assert!(app.queued.is_empty(), "nothing is sent");
    key(&mut app, Key::Text(b"!".to_vec()));
    assert_eq!(drafted(&app), format!("{before}!"), "typing goes on");
}

/// A1b: Alt+y on an empty draft copies nothing and says why.
#[test]
fn alt_y_on_an_empty_draft_says_nothing_to_copy() {
    let root = Root::new("cc-empty");
    let mut app = drafting(&root, &[]);
    key(&mut app, Key::CopyDraft);
    assert_eq!(flash(&app), "nothing to copy: the draft is empty");
    assert_eq!(app.clip, None);
    assert!(app.composing() && app.queued.is_empty());
}

/// A2: a drag over draft cells copies exactly the lit cells and changes
/// nothing of the draft, the cursor or the writing.
#[test]
fn a_drag_in_the_composer_copies_the_lit_draft_cells() {
    let root = Root::new("cc-drag");
    let mut app = drafting(&root, &["qzhello world", "qzsecond"]);
    let before = (drafted(&app), app.layout.cursor);
    let plain = shown(&mut app);
    let start = cell(&plain, "qzhello");
    let lit = drag_copy(&mut app, start, (start.0 + 6, start.1));
    let flipped: Vec<u16> = (0..lit.area.width)
        .filter(|&x| {
            let (was, is) = (plain[(x, start.1)].modifier, lit[(x, start.1)].modifier);
            was.contains(Modifier::REVERSED) != is.contains(Modifier::REVERSED)
        })
        .collect();
    assert_eq!(
        flipped,
        (start.0..=start.0 + 6).collect::<Vec<_>>(),
        "exactly the seven dragged cells light up, the space after them not"
    );
    let other = (0..lit.area.width)
        .any(|x| plain[(x, start.1 + 1)].modifier != lit[(x, start.1 + 1)].modifier);
    assert!(!other, "the row under the drag is not lit");
    assert_eq!(app.clip.as_deref(), Some("qzhello"));
    assert_eq!(flash(&app), format!("sent 1 line {UNTMUXED}"));
    let _ = shown(&mut app);
    assert_eq!(
        (drafted(&app), app.layout.cursor),
        before,
        "draft and cursor"
    );
    assert!(app.composing() && app.queued.is_empty());
}

/// A3: dragged up past the address row, the head pins to the first draft
/// cell: the copy is the draft rows, never the address row above them.
#[test]
fn a_drag_up_past_the_address_row_stops_at_the_first_draft_row() {
    let root = Root::new("cc-up");
    let mut app = drafting(&root, &["qzhello world", "qzsecond"]);
    let buf = shown(&mut app);
    let (first, second) = (cell(&buf, "qzhello"), cell(&buf, "qzsecond"));
    assert_eq!(second.1, first.1 + 1, "one row each");
    let end = (second.0 + 7, second.1);
    let _ = drag_copy(&mut app, end, (first.0 + 3, first.1 - 1));
    assert_eq!(app.clip.as_deref(), Some("qzhello world\nqzsecond"));
}

/// A4: a press on the address row starts no composer selection and a drag
/// from it copies nothing; the draft stays.
#[test]
fn the_address_row_is_not_selectable() {
    let root = Root::new("cc-addr");
    let mut app = drafting(&root, &["qzonly"]);
    let before = drafted(&app);
    let draft = cell(&shown(&mut app), "qzonly");
    let address = (draft.0 + 1, draft.1 - 1);
    press(&mut app, MouseKind::Click, address);
    press(&mut app, MouseKind::Drag, (address.0 + 8, address.1));
    let _ = shown(&mut app);
    assert!(
        !(app.select.as_ref()).is_some_and(|held| held.sel.panel() == select::Panel::Composer),
        "no composer selection from the address row"
    );
    press(&mut app, MouseKind::Release, (address.0 + 8, address.1));
    let _ = shown(&mut app);
    let _ = app.ship_staged();
    assert_eq!(app.clip, None, "nothing is copied");
    assert_eq!(drafted(&app), before);
}

/// A5: typing while a selection is held clears it and still edits; an edit
/// mid-drag clears it before anything ships.
#[test]
fn an_edit_clears_the_selection_and_still_types() {
    let root = Root::new("cc-edit");
    let mut app = drafting(&root, &["qzhello"]);
    let start = cell(&shown(&mut app), "qzhello");
    let _ = drag_copy(&mut app, start, (start.0 + 3, start.1));
    key(&mut app, Key::Text(b"Z".to_vec()));
    assert!(app.select.is_none(), "an edit clears the selection");
    assert_eq!(drafted(&app), "qzhelloZ");

    let root = Root::new("cc-middrag");
    let mut app = drafting(&root, &["qzhello"]);
    let start = cell(&shown(&mut app), "qzhello");
    press(&mut app, MouseKind::Click, start);
    press(&mut app, MouseKind::Drag, (start.0 + 3, start.1));
    let _ = shown(&mut app);
    key(&mut app, Key::Text(b"Z".to_vec()));
    assert!(app.select.is_none(), "an edit mid-drag clears it");
    press(&mut app, MouseKind::Release, (start.0 + 3, start.1));
    let _ = shown(&mut app);
    assert!(!app.ship_staged(), "nothing ships");
    assert_eq!(app.clip, None);
    assert_eq!(drafted(&app), "qzhelloZ");
}

/// A6: Esc with a composer selection only clears it; the next Esc goes back
/// to browsing and keeps the draft.
#[test]
fn esc_clears_the_selection_first_then_leaves_writing() {
    let root = Root::new("cc-esc");
    let mut app = drafting(&root, &["qzhello"]);
    let start = cell(&shown(&mut app), "qzhello");
    let _ = drag_copy(&mut app, start, (start.0 + 3, start.1));
    key(&mut app, Key::Escape);
    assert!(
        app.select.is_none() && app.composing(),
        "only the selection"
    );
    key(&mut app, Key::Escape);
    assert!(!app.composing(), "then browsing");
    assert_eq!(drafted(&app), "qzhello", "the draft is kept");
}

/// A7: the elision row the composer draws is not draft text and reads as a
/// blank; a draft line that merely spells such a marker is text and copies.
#[test]
fn an_elision_row_reads_blank_and_a_typed_marker_copies() {
    let root = Root::new("cc-marker");
    let lines: Vec<String> = (1..=14).map(|n| format!("qzrow{n:02}")).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut app = drafting(&root, &refs);
    let buf = shown(&mut app);
    let marker = cell(&buf, "lines above");
    let left = cell(&buf, "qzrow14").0;
    let below: String = (left..buf.area.width)
        .map(|x| buf[(x, marker.1 + 1)].symbol())
        .collect();
    let next = below.trim_end().to_owned();
    assert!(next.starts_with("qzrow"), "a draft row under it: {below:?}");
    let end = left + u16::try_from(next.chars().count()).expect("short") - 1;
    let _ = drag_copy(&mut app, (left, marker.1), (end, marker.1 + 1));
    assert_eq!(
        app.clip,
        Some(format!("\n{next}")),
        "the elision row is one blank row, then the draft row under it"
    );

    let root = Root::new("cc-literal");
    let mut app = drafting(&root, &["… +2 lines above"]);
    let start = cell(&shown(&mut app), "… +2 lines above");
    let _ = drag_copy(&mut app, start, (start.0 + 15, start.1));
    assert_eq!(app.clip.as_deref(), Some("… +2 lines above"));
}
