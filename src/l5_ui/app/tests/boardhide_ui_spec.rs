//! More acceptance for board visibility: the header click, the unreadable
//! fault cue and the key bar's own word.

use super::boardfull_spec::{PANEL_TURN, bytes, locate, paint, row_text, sample_desk, setup_board};
use super::*;
use crate::console::deskview::DeskView;

const WIDE: u16 = 200;
const HEIGHT: u16 = 35;
const DECISION: &str = "Check release choice";
const BOARD: &[u8] = b"DECIDE\nD1 | Check release choice\nD2 | Confirm scope\n";

fn click(app: &mut App, (column, row): (u16, u16)) {
    for kind in [MouseKind::Click, MouseKind::Release] {
        let press = Key::Mouse(Mouse { kind, column, row });
        assert!(take_keys(app, vec![(press, Instant::now())]).is_some());
    }
}

#[test]
fn the_header_word_hides_and_shows_the_board_by_click() {
    for full in [false, true] {
        let (_root, mut app) = setup_board("boardhide-click", Some(sample_desk(BOARD)));
        if full {
            bytes(&mut app, b"F");
        }
        let shown = paint(&mut app, WIDE, HEIGHT);
        let word = locate(&shown, "b hide").expect("the hide word is in the header");
        assert_eq!(word.1, 0, "full={full}");
        click(&mut app, word);
        let hidden = paint(&mut app, WIDE, HEIGHT);
        assert!(!text(&hidden).contains(DECISION), "full={full}");
        let word = locate(&hidden, "b show").expect("the show word is in the header");
        assert_eq!(word.1, 0, "full={full}");
        click(&mut app, word);
        assert!(text(&paint(&mut app, WIDE, HEIGHT)).contains(DECISION));
    }
}

#[test]
fn hiding_an_unreadable_board_keeps_its_fault_in_the_header() {
    let unreadable = DeskView::Unreadable("cut".to_owned());
    let (_root, mut app) = setup_board("boardhide-fault", Some(unreadable));
    let shown = paint(&mut app, WIDE, HEIGHT);
    assert!(text(&shown).contains("unreadable: cut"));
    bytes(&mut app, b"b");
    let hidden = paint(&mut app, WIDE, HEIGHT);
    assert!(!text(&hidden).contains("unreadable: cut"));
    assert!(row_text(&hidden, 0).contains("desk unreadable"));
    assert!(row_text(&hidden, 0).contains("b show"));
}

#[test]
fn the_key_bar_names_the_key_only_while_browsing_a_board() {
    let (_root, mut app) = setup_board("boardhide-bar", Some(sample_desk(BOARD)));
    let last = |cells: &Buffer| row_text(cells, cells.area.height - 1);
    assert!(last(&paint(&mut app, WIDE, HEIGHT)).contains("? keys · b hide"));
    bytes(&mut app, b"b");
    assert!(last(&paint(&mut app, WIDE, HEIGHT)).contains("? keys · b show"));
    bytes(&mut app, b"b");

    // While a draft is written the keys are the composer's, and `b` is a letter.
    bytes(&mut app, b"i");
    let writing = last(&paint(&mut app, WIDE, HEIGHT));
    assert!(writing.contains("Esc browses"));
    assert!(!writing.contains("b show") && !writing.contains("b hide"));

    // With the draft kept, the longer hint stands only where it fits whole;
    // else the plain one the hint row had before stays.
    bytes(&mut app, b"hello");
    assert!(take_keys(&mut app, vec![(Key::Escape, Instant::now())]).is_some());
    let kept = "draft kept · Enter writes · ? keys";
    let (mut with, mut without) = (0, 0);
    for width in 100..=WIDE {
        let row = last(&paint(&mut app, width, HEIGHT));
        if row.contains(&format!("{kept} · b hide")) {
            with += 1;
        } else if row.contains(kept) {
            without += 1;
            assert!(!row.contains("b hide"), "{row:?}");
        }
    }
    assert!(with > 0 && without > 0, "with={with} without={without}");
    let (_root, mut bare) = setup_board("boardhide-bar-none", Some(DeskView::Absent));
    assert!(!last(&paint(&mut bare, WIDE, HEIGHT)).contains("b hide"));
}

#[test]
fn a_stale_header_word_asks_for_the_visibility_it_named_not_a_flip() {
    // The frame drew `b hide`. The read that holds `b` also holds a press on
    // that word: the board is already hidden, so the press changes nothing.
    for (width, full) in [(120, false), (60, true), (WIDE, false), (WIDE, true)] {
        let (_root, mut app) = setup_board("boardhide-stale-word", Some(sample_desk(BOARD)));
        if full {
            bytes(&mut app, b"F");
        }
        let shown = paint(&mut app, width, HEIGHT);
        let word = locate(&shown, "b hide").expect("the hide word is in the header");
        let origin = Instant::now();
        let press = Key::Mouse(Mouse {
            kind: MouseKind::Click,
            column: word.0,
            row: word.1,
        });
        let read = vec![(Key::Text(b"b".to_vec()), origin), (press, origin)];
        assert!(take_keys(&mut app, read).is_some());
        let after = paint(&mut app, width, HEIGHT);
        let header = row_text(&after, 0);
        assert!(header.contains("b show"), "{width} {full}: {header:?}");
        assert!(!header.contains("b hide"), "{width} {full}: {header:?}");
    }
}

#[test]
fn an_open_overlay_takes_b_so_the_key_bar_stops_naming_it() {
    let (_root, mut app) = setup_board("boardhide-overlay", Some(sample_desk(BOARD)));
    let last = |cells: &Buffer| row_text(cells, cells.area.height - 1);
    assert!(last(&paint(&mut app, WIDE, HEIGHT)).contains("b hide"));
    bytes(&mut app, b"?");
    let keys_tab = paint(&mut app, WIDE, HEIGHT);
    assert!(text(&keys_tab).contains("hide / show the orchestrator board"));
    let bar = last(&keys_tab);
    assert!(
        !bar.contains("b hide") && !bar.contains("b show"),
        "{bar:?}"
    );
}

#[test]
fn a_peek_or_a_reader_takes_b_as_the_settings_do() {
    for reader in [false, true] {
        let (_root, mut app) = setup_board("boardhide-peek", Some(sample_desk(BOARD)));
        let last = |cells: &Buffer| row_text(cells, cells.area.height - 1);
        assert!(last(&paint(&mut app, WIDE, HEIGHT)).contains("b hide"));
        if reader {
            app.model.open_reader(1);
        } else {
            app.model.open_peek(1);
        }
        let bar = last(&paint(&mut app, WIDE, HEIGHT));
        assert!(
            !bar.contains("b hide") && !bar.contains("b show"),
            "reader={reader}: {bar:?}"
        );
    }
}

const GONE: &str = "click not taken: its frame is gone";

#[test]
fn a_press_in_the_read_that_hides_the_board_is_refused_before_any_frame() {
    for full in [false, true] {
        let (_root, mut app) = setup_board("boardhide-same-read", Some(sample_desk(BOARD)));
        if full {
            bytes(&mut app, b"F");
        }
        let shown = paint(&mut app, WIDE, HEIGHT);
        let (column, row) = locate(&shown, DECISION).expect("a decision on the board");
        let origin = Instant::now();
        let press = Key::Mouse(Mouse {
            kind: MouseKind::Click,
            column,
            row,
        });
        app.flash = None;
        let read = vec![(Key::Text(b"b".to_vec()), origin), (press, origin)];
        assert!(take_keys(&mut app, read).is_some());
        let line = app.flash.as_ref().map(|(line, _)| line.as_str());
        assert_eq!(line, Some(GONE), "full={full}");
    }
}

#[test]
fn a_press_made_where_the_board_returns_is_refused_not_aimed_at_the_old_frame() {
    let (_root, mut app) = setup_board("boardhide-returns", Some(sample_desk(BOARD)));
    bytes(&mut app, b"b");
    let hidden = paint(&mut app, WIDE, HEIGHT);
    let (column, row) = locate(&hidden, "F full").expect("the mode word on the wide panel");
    let origin = Instant::now();
    bytes(&mut app, b"b");
    let _ = paint(&mut app, WIDE, HEIGHT);
    app.flash = None;
    let press = Key::Mouse(Mouse {
        kind: MouseKind::Click,
        column,
        row,
    });
    assert!(take_keys(&mut app, vec![(press, origin)]).is_some());
    let line = app.flash.as_ref().map(|(line, _)| line.as_str());
    assert_eq!(line, Some(GONE));
    let after = row_text(&paint(&mut app, WIDE, HEIGHT), 0);
    assert!(
        after.contains("F full") && !after.contains("F side"),
        "{after:?}"
    );
}

#[test]
fn b_asks_for_a_redraw_exactly_when_it_changes_what_shows() {
    let (_root, mut app) = setup_board("boardhide-redraw", Some(sample_desk(BOARD)));
    let _ = paint(&mut app, WIDE, HEIGHT);
    let press = |app: &mut App| take_keys(app, vec![(Key::Text(b"b".to_vec()), Instant::now())]);
    assert_eq!(press(&mut app), Some(true), "hides");
    assert_eq!(press(&mut app), Some(true), "shows");
    let (_root, mut bare) = setup_board("boardhide-redraw-none", Some(DeskView::Absent));
    let _ = paint(&mut bare, WIDE, HEIGHT);
    assert_eq!(press(&mut bare), Some(false), "no board, nothing to change");
}

#[test]
fn a_panel_the_zoomed_main_column_covers_does_not_hand_the_keys_to_its_board() {
    let (_root, mut app) = setup_board("boardhide-covered", Some(sample_desk(BOARD)));
    let _ = paint(&mut app, WIDE, HEIGHT);
    app.focus_column(false);
    bytes(&mut app, b"X");
    let covered = paint(&mut app, WIDE, HEIGHT);
    assert!(!text(&covered).contains(PANEL_TURN) && !text(&covered).contains(DECISION));
    bytes(&mut app, b"B");
    assert!(!app.desk_focused());
    assert!(!app.panel_focused());
}
