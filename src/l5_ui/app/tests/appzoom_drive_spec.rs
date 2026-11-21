//! The zoom's keyboard rules the frozen acceptance leaves open: the keys that
//! need a card or a seat only the sidebar draws, `X` through the seat view and
//! the reader, and the keys a zoomed panel hands back to the main column.

use super::*;
use crate::console::input::Keys;

const CHAT: &str = "appzoom-drive-chat";
const OTHER: &str = "drive-witness";

fn paint(app: &mut App, width: u16) -> Buffer {
    let mut cells = Buffer::empty(Rect::new(0, 0, width, 45));
    app.frame(&mut cells);
    app.frame_shown(Instant::now());
    cells
}

fn bytes(app: &mut App, value: &[u8]) {
    let at = Instant::now();
    assert!(take_keys(app, Keys::app().feed(value, at)).is_some());
}

fn zoomed_panel(app: &App) -> bool {
    app.orch.panel.as_ref().is_some_and(|panel| panel.full)
}

fn hint(app: &App) -> String {
    app.flash
        .as_ref()
        .map(|(line, _)| line.clone())
        .unwrap_or_default()
}

/// An app homed on `api` beside a second running session, one turn of chat.
fn chat(root: &Root) -> App {
    session(root, OTHER, "");
    let (mut app, reader) = housed(root);
    refold(
        &mut app,
        &reader,
        root,
        &[("api", Status::Running), (OTHER, Status::Running)],
    );
    app.lane = Rc::new(Lane {
        items: vec![said(CHAT, 0)],
        coverage: Vec::new(),
    });
    app.loading = false;
    app
}

#[test]
fn s_and_m_keep_their_visible_card_guard_while_zoom_hides_the_card() {
    let root = Root::new("drive-card");
    let mut app = chat(&root);
    paint(&mut app, 160);
    bytes(&mut app, b"m");
    assert!(app.modal.is_some(), "the drawn card opens its menu");
    paint(&mut app, 160);
    let _ = take_keys(&mut app, vec![(Key::Escape, Instant::now())]);
    assert!(app.modal.is_none());

    bytes(&mut app, b"X");
    paint(&mut app, 160);
    assert!(app.layout.cards.is_empty(), "zoom draws no card to act on");
    for key in [b"s", b"m"] {
        app.flash = None;
        bytes(&mut app, key);
        assert!(app.modal.is_none(), "{key:?} opens nothing in zoom");
        assert_eq!(hint(&app), "menu not opened: no card is on screen for it");
        paint(&mut app, 160);
    }
    assert!(
        app.model.zoom(),
        "the refusal leaves the zoom and the selection"
    );
    assert_eq!(app.model.selected(), Some("api"));
}

#[test]
fn v_in_zoom_opens_no_seat_view_of_a_seat_the_frame_did_not_draw() {
    let root = Root::new("drive-peek-key");
    let mut app = chat(&root);
    bytes(&mut app, b"X");
    paint(&mut app, 160);
    app.flash = None;
    bytes(&mut app, b"v");
    assert!(!app.model.peek_open());
    assert!(hint(&app).starts_with("refused: peek"), "{}", hint(&app));
    assert!(app.model.zoom());
}

#[test]
fn x_reaches_the_zoom_through_the_seat_view_and_the_reader() {
    let root = Root::new("drive-overlay");
    let mut app = chat(&root);
    paint(&mut app, 160);
    app.model.open_peek(1);
    bytes(&mut app, b"X");
    assert!(app.model.zoom() && app.model.peek_open(), "zoom, view kept");
    let zoomed = paint(&mut app, 160);
    assert!(!text(&zoomed).contains(OTHER), "the sidebar is not drawn");
    bytes(&mut app, b"X");
    assert!(
        !app.model.zoom() && app.model.peek_open(),
        "unzoom, view kept"
    );
    app.model.close_peek();

    app.model.open_reader(1);
    bytes(&mut app, b"X");
    assert!(app.model.zoom() && app.model.reader_open());
    bytes(&mut app, b"X");
    assert!(!app.model.zoom() && app.model.reader_open());
}

#[test]
fn a_zoomed_panel_hands_the_keys_and_the_selection_back_to_the_zoomed_chat() {
    let root = Root::new("drive-hand-back");
    let mut app = sender(&root, Left::SideMain);
    paint(&mut app, 160);
    bytes(&mut app, b"X");
    paint(&mut app, 160);
    bytes(&mut app, b"O");
    paint(&mut app, 160);
    assert!(app.panel_focused() && zoomed_panel(&app));
    bytes(&mut app, b"1");
    paint(&mut app, 160);
    assert!(
        !app.panel_focused() && !zoomed_panel(&app),
        "the panel turns side and the main column takes the keys"
    );
    assert!(app.model.zoom(), "the main column keeps its own zoom");
    assert!(
        app.layout.panel.is_none(),
        "the zoomed main covers the panel"
    );
}

#[test]
fn f_with_the_main_column_zoomed_gives_the_panel_its_own_zoom() {
    let root = Root::new("drive-f");
    let mut app = sender(&root, Left::SideMain);
    paint(&mut app, 160);
    bytes(&mut app, b"X");
    paint(&mut app, 160);
    bytes(&mut app, b"F");
    let panel = paint(&mut app, 160);
    assert!(app.panel_focused() && zoomed_panel(&app));
    assert!(app.layout.panel.is_some(), "the panel fills the frame");
    assert!(!text(&panel).contains("X unzoom"), "{}", text(&panel));
    bytes(&mut app, b"F");
    paint(&mut app, 160);
    assert!(!zoomed_panel(&app), "the panel is side by side again");
    assert!(
        app.model.zoom(),
        "the main column keeps its own zoom stored"
    );
    assert!(
        app.layout.panel.is_some() && app.panel_focused(),
        "the focused side panel shows the split"
    );
    let _ = take_keys(&mut app, vec![(Key::Escape, Instant::now())]);
    let closed = paint(&mut app, 160);
    assert!(app.orch.panel.is_none());
    assert!(
        !row_text(&closed, 0).contains('│') && row_text(&closed, 0).contains("X unzoom"),
        "closing the panel shows the stored main zoom: {}",
        row_text(&closed, 0)
    );
}

fn row_text(cells: &Buffer, y: u16) -> String {
    (0..cells.area.width)
        .map(|x| cells[(x, y)].symbol())
        .collect()
}

fn locate(cells: &Buffer, needle: &str) -> (u16, u16) {
    let width = u16::try_from(needle.len()).expect("short needle");
    for y in 0..cells.area.height {
        for x in 0..=cells.area.width.saturating_sub(width) {
            let candidate: String = (x..x + width).map(|x| cells[(x, y)].symbol()).collect();
            if candidate == needle {
                return (x, y);
            }
        }
    }
    panic!("{needle:?} is absent:\n{}", text(cells));
}

fn mouse_at(app: &mut App, kind: MouseKind, (column, row): (u16, u16), at: Instant) {
    assert!(take_keys(app, vec![(Key::Mouse(Mouse { kind, column, row }), at)]).is_some());
}

#[test]
fn a_panel_zoom_change_by_f_ends_a_selection_even_with_the_main_zoom_off() {
    let root = Root::new("drive-epoch");
    let mut app = sender(&root, Left::SideMain);
    paint(&mut app, 160);
    assert!(!app.model.zoom());
    let before = app.model.epoch();
    bytes(&mut app, b"F");
    assert_ne!(app.model.epoch(), before, "the key itself ends a selection");
    let mid = app.model.epoch();
    paint(&mut app, 160);
    bytes(&mut app, b"F");
    assert_ne!(app.model.epoch(), mid, "and so does the way back");
}

#[test]
fn the_label_on_a_split_frame_zooms_even_with_the_main_zoom_stored() {
    let root = Root::new("drive-label");
    let mut app = sender(&root, Left::SideMain);
    paint(&mut app, 160);
    bytes(&mut app, b"X");
    paint(&mut app, 160);
    bytes(&mut app, b"F");
    paint(&mut app, 160);
    bytes(&mut app, b"F");
    let split = paint(&mut app, 160);
    assert!(app.model.zoom() && app.panel_focused() && app.layout.panel.is_some());
    assert!(
        row_text(&split, 0).contains("X zoom"),
        "{}",
        row_text(&split, 0)
    );
    let at = locate(&split, "X zoom");
    mouse_at(&mut app, MouseKind::Click, at, Instant::now());
    mouse_at(&mut app, MouseKind::Release, at, Instant::now());
    let zoomed = paint(&mut app, 160);
    assert!(app.model.zoom() && !app.panel_focused());
    assert!(
        !row_text(&zoomed, 0).contains('│'),
        "{}",
        row_text(&zoomed, 0)
    );
    assert!(row_text(&zoomed, 0).contains("X unzoom"));
}

#[test]
fn a_press_made_before_the_zoom_moves_from_one_column_to_the_other_acts_on_nothing() {
    let root = Root::new("drive-cut");
    let mut app = sender(&root, Left::SideMain);
    paint(&mut app, 160);
    bytes(&mut app, b"X");
    let chat = paint(&mut app, 160);
    let made = Instant::now();
    bytes(&mut app, b"O");
    paint(&mut app, 160);
    assert!(app.panel_focused() && zoomed_panel(&app));
    app.flash = None;
    let at = locate(&chat, "X unzoom");
    mouse_at(&mut app, MouseKind::Click, at, made);
    assert_eq!(hint(&app), "click not taken: its frame is gone");
    assert!(
        app.model.zoom() && zoomed_panel(&app),
        "the stale press toggled nothing"
    );
}

#[test]
fn the_zoom_cut_still_refuses_after_the_history_bound_dropped_an_older_frame() {
    let root = Root::new("drive-cut-monotonic");
    let mut app = chat(&root);
    for _ in 0..(crate::app::HISTORY + 2) {
        paint(&mut app, 160);
    }
    let card = locate(&paint(&mut app, 160), OTHER);
    let made = Instant::now();
    bytes(&mut app, b"X");
    paint(&mut app, 160);
    assert!(
        app.lost.is_some_and(|lost| lost < made),
        "control: the bound dropped a frame older than the press, which refuses nothing"
    );
    app.flash = None;
    mouse_at(&mut app, MouseKind::Click, card, made);
    assert_eq!(hint(&app), "click not taken: its frame is gone");
    assert_eq!(app.model.selected(), Some("api"), "the old card is not hit");
}

#[test]
fn a_press_decoded_in_the_read_of_the_zoom_key_names_no_card_of_the_old_frame() {
    for (byte, kind, panel) in [
        (b'X', MouseKind::Click, false),
        (b'X', MouseKind::Menu, false),
        (b'F', MouseKind::Menu, true),
    ] {
        let root = Root::new("drive-same-read");
        let mut app = chat(&root);
        if panel {
            app.open_panel(false);
            app.focus_column(false);
        }
        let (column, row) = locate(&paint(&mut app, 160), OTHER);
        let made = Instant::now();
        let read = vec![
            (Key::Text(vec![byte]), made),
            (Key::Mouse(Mouse { kind, column, row }), made),
        ];
        let _ = take_keys(&mut app, read);
        assert_eq!(app.model.selected(), Some("api"), "{byte:?} {kind:?}");
        assert!(app.modal.is_none(), "{byte:?} {kind:?} opened a menu");
    }
}

#[test]
fn a_press_at_the_replaced_layout_or_before_the_frame_is_shown_names_no_old_card() {
    let root = Root::new("drive-window");
    let mut app = chat(&root);
    let card = locate(&paint(&mut app, 160), OTHER);
    bytes(&mut app, b"X");
    paint(&mut app, 160);
    let replaced = app.history.last().expect("the replaced layout").0;
    mouse_at(&mut app, MouseKind::Menu, card, replaced);
    assert!(app.modal.is_none(), "the press at the replaced layout");

    // The way back, shown; then a second zoom frame, drawn but not yet shown:
    // input read after it is drawn and before it is out was made on the frame
    // still on the screen.
    bytes(&mut app, b"X");
    let card = locate(&paint(&mut app, 160), OTHER);
    bytes(&mut app, b"X");
    let mut cells = Buffer::empty(Rect::new(0, 0, 160, 45));
    app.frame(&mut cells);
    let between = Instant::now();
    app.frame_shown(Instant::now());
    mouse_at(&mut app, MouseKind::Menu, card, between);
    assert!(
        app.modal.is_none(),
        "the press made before the frame was out"
    );
    app.flash = None;
    let after = Instant::now();
    mouse_at(&mut app, MouseKind::Click, (3, 5), after);
    assert_eq!(hint(&app), "", "a press after it is taken: {}", hint(&app));
}
