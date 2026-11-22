//! More acceptance for board visibility: the header click, the unreadable
//! fault cue and the key bar's own word.

use super::boardfull_spec::{bytes, locate, paint, row_text, sample_desk, setup_board};
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
