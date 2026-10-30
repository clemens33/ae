//! The card menu's read-only items: `Activity…` and `Memos…` ask the reader
//! for rows, the menu waits for them, and a dialog over those rows scrolls and
//! closes. Child of `app.rs::tests`; the end-to-end cases are
//! `tests/it/app_menu_activity_memos_spec.rs`.

use std::sync::mpsc::Receiver;

use super::*;
use crate::app::loader::{RecordsAsk, Request};
use crate::app::menu::{Asked, Card, Dialog, Item, Menu, Modal, Records, Step};
use crate::session_menu::CellRow;

fn keyed(app: &mut App, bytes: &[u8], at: Instant) {
    assert!(take_keys(app, vec![(Key::Text(bytes.to_vec()), at)]).is_some());
}

fn shown(app: &mut App, at: Instant) -> String {
    let screen = framed(app);
    app.frame_shown(at);
    screen
}

/// An app on a running `api` whose menu is open and on screen, with a channel
/// for what it asks the reader. Keys made after `opened` are fresh.
fn open(tag: &str) -> (Root, App, Reader, Receiver<Request>, Instant) {
    let root = Root::new(tag);
    let (mut app, reader) = housed(&root);
    app.answer(Answer::Look {
        name: "api".to_owned(),
        look: Some(crate::theme::Look::read("off", "darcula", "off", "off")),
        zone: None,
    });
    framed(&mut app);
    let (tx, asks) = std::sync::mpsc::channel();
    app.ask = Some(tx);
    keyed(&mut app, b"m", Instant::now());
    let opened = Instant::now();
    assert!(shown(&mut app, opened).contains("a  Activity."));
    (root, app, reader, asks, opened)
}

fn asked(asks: &Receiver<Request>) -> Vec<Option<Box<RecordsAsk>>> {
    asks.try_iter()
        .filter_map(|request| match request {
            Request::Records(ask) => Some(ask),
            _ => None,
        })
        .collect()
}

fn cells(age: &str, actor: &str, kind: &str, text: &str) -> CellRow {
    CellRow::Cells([age, actor, kind, text].map(str::to_owned).to_vec())
}

fn dialog(app: &App) -> bool {
    matches!(app.modal, Some(Modal::Records(_)))
}

fn later(at: Instant, ms: u64) -> Instant {
    at + Duration::from_millis(ms)
}

#[test]
fn every_session_card_offers_both_read_only_items_and_the_orchestrator_none() {
    let card = |name: &str, status| Card::new(name, ID, Some(ID), Some(status), None);
    assert_eq!(
        card("api", Status::Running).items(),
        [Item::Stop, Item::Activity, Item::Memos]
    );
    assert_eq!(
        card("api", Status::Stopped).items(),
        [Item::Start, Item::Activity, Item::Memos]
    );
    let lead = card(crate::orchestrator::ORCHESTRATOR_SESSION, Status::Running);
    assert!(lead.items().is_empty());
    assert_eq!(Item::Activity.dialog(), Some(Dialog::Activity));
    assert_eq!(Item::Memos.dialog(), Some(Dialog::Memos));
    assert_eq!((Item::Start.dialog(), Item::Stop.dialog()), (None, None));
}

#[test]
fn the_keys_choose_the_dialogs_and_a_waiting_menu_takes_no_second_item() {
    let typed = |bytes: &[u8]| Key::Text(bytes.to_vec());
    let card = Card::new("api", ID, Some(ID), Some(Status::Running), None);
    let mut menu = Modal::Menu(Menu::new(card));
    assert_eq!(menu.key(&typed(b"a")), Step::Act(Item::Activity));
    assert_eq!(menu.key(&typed(b"o")), Step::Act(Item::Memos));
    assert_eq!(menu.key(&Key::Down), Step::Moved);
    assert_eq!(menu.key(&Key::Enter), Step::Act(Item::Activity));
    assert_eq!(menu.key(&Key::Down), Step::Moved);
    assert_eq!(menu.key(&Key::Enter), Step::Act(Item::Memos));
    let Modal::Menu(waiting) = &mut menu else {
        unreachable!("built as a menu")
    };
    waiting.reading = Some(Asked {
        generation: 1,
        dialog: Dialog::Memos,
    });
    for key in [typed(b"s"), typed(b"a"), typed(b"o"), Key::Enter] {
        assert_eq!(menu.key(&key), Step::Kept, "{key:?}");
    }
    assert_eq!(menu.key(&typed(b"q")), Step::Close);
    assert_eq!(menu.key(&Key::Escape), Step::Close);
    // The orchestrator's menu has no items, so no key reaches one.
    let lead = Card::new(
        crate::orchestrator::ORCHESTRATOR_SESSION,
        ID,
        Some(ID),
        Some(Status::Running),
        None,
    );
    let mut lone = Modal::Menu(Menu::new(lead));
    assert_eq!(lone.key(&typed(b"a")), Step::Kept);
    assert_eq!(lone.key(&typed(b"o")), Step::Kept);
}

#[test]
fn a_dialog_item_asks_the_reader_once_and_the_menu_stays_until_the_rows_land() {
    let (root, mut app, _reader, asks, opened) = open("records-ask");
    keyed(&mut app, b"a", later(opened, 1));
    let mut sent = asked(&asks);
    assert_eq!(sent.len(), 1, "one ask");
    let ask = sent.remove(0).expect("an ask, not a cancel");
    assert_eq!(ask.dialog, Dialog::Activity);
    assert_eq!(ask.session, "api");
    assert_eq!(ask.uuid, ID);
    assert_eq!(ask.dir, root.0.join("sessions").join("api"));
    assert!(ask.live, "a running card reads the live identity");
    assert!(
        matches!(&app.modal, Some(Modal::Menu(menu)) if menu.reading.map(|r| r.generation) == Some(ask.generation)),
        "the menu waits"
    );
    assert!(framed(&mut app).contains("s  Stop"), "and stays drawn");
    // A second pick, by key or by click, while the first is out: nothing.
    keyed(&mut app, b"o", later(opened, 2));
    assert!(asked(&asks).is_empty());
    assert!(app.notices.is_empty(), "a read writes no notice");

    let rows = vec![cells(">7d", "lead", "goal", "ROW_ONE")];
    app.answer(Answer::Records {
        generation: ask.generation,
        rows,
    });
    assert!(dialog(&app), "the answer opens the dialog");
    let screen = framed(&mut app);
    assert!(
        screen.contains("Activity · api") && screen.contains("ROW_ONE"),
        "{screen}"
    );
    assert!(!screen.contains("s  Stop"), "the menu is gone: {screen}");
    // The next ask is a new generation.
    app.modal = None;
    keyed(&mut app, b"m", Instant::now());
    let again = Instant::now();
    shown(&mut app, again);
    keyed(&mut app, b"o", later(again, 1));
    let next = asked(&asks).remove(0).expect("an ask");
    assert_eq!(next.dialog, Dialog::Memos);
    assert!(next.generation > ask.generation);
}

#[test]
fn rows_for_a_closed_menu_or_an_older_ask_are_dropped() {
    let (_root, mut app, _reader, asks, opened) = open("records-stale");
    keyed(&mut app, b"a", later(opened, 1));
    let ask = asked(&asks).remove(0).expect("an ask");
    let rows = || vec![cells(">7d", "lead", "goal", "STALE_ROW")];
    // Another generation: the menu keeps waiting.
    app.answer(Answer::Records {
        generation: ask.generation + 1,
        rows: rows(),
    });
    assert!(matches!(&app.modal, Some(Modal::Menu(_))), "not its answer");
    // Esc abandons the read and tells the reader.
    assert!(take_keys(&mut app, vec![(Key::Escape, later(opened, 3))]).is_some());
    assert!(app.modal.is_none());
    let told = asked(&asks);
    assert!(
        matches!(told.as_slice(), [None]),
        "the reader is told to drop it"
    );
    app.answer(Answer::Records {
        generation: ask.generation,
        rows: rows(),
    });
    assert!(app.modal.is_none(), "a late answer opens nothing");
    assert!(!framed(&mut app).contains("STALE_ROW"));
}

#[test]
fn a_menu_that_waits_on_nothing_closes_without_telling_the_reader() {
    let (_root, mut app, _reader, asks, opened) = open("records-quiet-close");
    assert!(take_keys(&mut app, vec![(Key::Escape, later(opened, 1))]).is_some());
    assert!(app.modal.is_none());
    assert!(asked(&asks).is_empty());
}

#[test]
fn a_card_that_refuses_to_act_still_reads() {
    let (root, mut app, reader, asks, _) = open("records-refused");
    app.modal = None;
    refold(&mut app, &reader, &root, &[("api", Status::Unknown)]);
    app.facts
        .insert("api".to_owned(), fleet::Facts::NotPublished);
    framed(&mut app);
    keyed(&mut app, b"m", Instant::now());
    let opened = Instant::now();
    let screen = shown(&mut app, opened);
    assert!(screen.contains("unknown"), "the card says why: {screen}");
    // Start and Stop still refuse.
    let before = app.noticed;
    keyed(&mut app, b"s", later(opened, 1));
    assert!(app.noticed > before, "the Stop refusal is a fresh notice");
    let (target, item) = app.notices.last().expect("the refusal");
    assert_eq!(target, "api");
    assert!(
        item.body.contains("refused: Stop api") && item.body.contains("nothing done"),
        "{}",
        item.body
    );
    assert!(app.modal.is_none(), "a refused item closes the menu");
    // The read-only item takes no refusal.
    let count = app.notices.len();
    framed(&mut app);
    keyed(&mut app, b"m", Instant::now());
    let reopened = Instant::now();
    let _ = shown(&mut app, reopened);
    keyed(&mut app, b"a", later(reopened, 1));
    assert_eq!(asked(&asks).len(), 1, "a read-only item takes no refusal");
    assert_eq!(app.notices.len(), count, "{:?}", app.notices);
}

#[test]
fn a_record_directory_ae_cannot_read_is_told_not_asked() {
    let (_root, mut app, _reader, asks, opened) = open("records-no-dir");
    app.dirs.clear();
    keyed(&mut app, b"a", later(opened, 1));
    assert!(asked(&asks).is_empty());
    assert!(app.modal.is_none());
    let (target, item) = app.notices.last().expect("the refusal");
    assert_eq!(target, "api");
    assert!(
        item.body.contains("refused: Activity api") && item.body.contains("no record directory"),
        "{}",
        item.body
    );
}

fn rows(count: usize) -> Vec<CellRow> {
    (1..=count)
        .map(|n| cells(">7d", "lead", "goal", &format!("SCROLL_ROW_{n:02}")))
        .collect()
}

fn painted(modal: &Modal, width: u16, height: u16) -> Result<String, String> {
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
    let mut layout = crate::app::draw::Layout::default();
    crate::app::menu::paint(
        modal,
        crate::app::draw::Paint::of(None),
        true,
        &mut buf,
        &mut layout,
    )?;
    assert!(layout.shows_modal(), "a painted dialog is a modal");
    Ok(text(&buf))
}

#[test]
fn a_dialog_scrolls_within_the_room_the_frame_drew_and_q_closes() {
    let typed = |bytes: &[u8]| Key::Text(bytes.to_vec());
    let mut modal = Modal::Records(Records::new(Dialog::Activity, "api", rows(30)));
    let first = painted(&modal, 100, 20).expect("fits");
    assert!(first.contains("SCROLL_ROW_01") && first.contains("SCROLL_ROW_14"));
    assert!(
        !first.contains("SCROLL_ROW_15"),
        "14 rows and a scroll line"
    );
    assert!(first.contains("1-14 of 30"), "{first}");
    assert_eq!(modal.key(&Key::Up), Step::Kept, "already at the top");
    assert_eq!(modal.key(&Key::Down), Step::Moved);
    assert_eq!(modal.key(&typed(b"j")), Step::Moved);
    assert_eq!(modal.key(&typed(b"k")), Step::Moved);
    let moved = painted(&modal, 100, 20).expect("fits");
    assert!(moved.contains("2-15 of 30") && !moved.contains("SCROLL_ROW_01"));
    assert_eq!(modal.key(&Key::PageDown), Step::Moved);
    assert!(
        painted(&modal, 100, 20)
            .expect("fits")
            .contains("16-29 of 30")
    );
    assert_eq!(modal.key(&Key::End), Step::Moved);
    let end = painted(&modal, 100, 20).expect("fits");
    assert!(end.contains("17-30 of 30") && end.contains("SCROLL_ROW_30"));
    assert_eq!(modal.key(&Key::Down), Step::Kept, "never past the last row");
    assert_eq!(modal.key(&Key::PageUp), Step::Moved);
    assert_eq!(modal.key(&Key::Home), Step::Moved);
    assert_eq!(modal.key(&Key::Home), Step::Kept);
    assert_eq!(modal.key(&typed(b"x")), Step::Kept);
    assert_eq!(modal.key(&typed(b"q")), Step::Close);
    assert_eq!(modal.key(&Key::Escape), Step::Close);
    assert_eq!(modal.key(&Key::Interrupt), Step::Close);
    // A tall window draws every row and no scroll line.
    let tall = painted(&modal, 100, 45).expect("fits");
    assert!(tall.contains("SCROLL_ROW_30") && !tall.contains(" of 30"));
}

#[test]
fn a_dialog_names_a_gap_and_aligns_like_the_tmux_dialog() {
    let rows = vec![
        cells("3m", "lead", "goal", "short"),
        cells("1h", "colead", "effort-restored", "a longer detail — kept"),
    ];
    let aligned =
        crate::session_menu::aligned_lines(&rows, crate::session_menu::ACTIVITY_FIXED_CAPS, 100);
    let modal = Modal::Records(Records::new(Dialog::Activity, "api", rows));
    let screen = painted(&modal, 100, 20).expect("fits");
    for line in &aligned {
        assert!(screen.contains(line.trim_end()), "{line:?}\n{screen}");
    }
    let gap = Modal::Records(Records::new(
        Dialog::Memos,
        "api",
        vec![CellRow::Gap("memos: none".to_owned())],
    ));
    let screen = painted(&gap, 60, 12).expect("fits");
    assert!(screen.contains("Memos · api") && screen.contains("memos: none"));
}

#[test]
fn drawn_dialog_text_is_neutralised_and_a_small_window_paints_nothing() {
    let rows = vec![CellRow::Gap("gap \u{1b}[2J \u{7}".to_owned())];
    let modal = Modal::Records(Records::new(Dialog::Activity, "api \u{1b}[2J", rows));
    let screen = painted(&modal, 80, 12).expect("fits");
    assert!(!screen.contains('\u{1b}') && !screen.contains('\u{7}'));
    assert!(screen.contains("api \u{FFFD}[2J") && screen.contains("gap \u{FFFD}[2J \u{FFFD}"));
    for (width, height) in [(15, 12), (80, 6)] {
        let why = painted(&modal, width, height).expect_err("too small");
        assert!(why.contains("too small for the Activity dialog"), "{why}");
    }
}

#[test]
fn a_press_outside_the_dialog_or_a_right_press_closes_it() {
    let modal = Modal::Records(Records::new(Dialog::Memos, "api", rows(3)));
    assert_eq!(
        modal.press(false, Some(&crate::app::draw::Hit::Modal)),
        Step::Kept
    );
    assert_eq!(modal.press(false, None), Step::Close);
    assert_eq!(
        modal.press(true, Some(&crate::app::draw::Hit::Modal)),
        Step::Close
    );
}
