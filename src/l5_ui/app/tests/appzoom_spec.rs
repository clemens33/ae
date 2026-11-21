//! Appzoom acceptance: exercise decoded terminal input, the drawn cells, and
//! both restart carriers without naming the zoom state or wire representation.

use super::*;
use crate::console::input::Keys;

const CHAT: &str = "appzoom-chat-proof";
const OTHER: &str = "sidebar-witness";
const LINK: &str = "/tmp/appzoom-proof.html";
const ZOOM: &[u8] = b"X";
const CHAT_ENTER: &str = "X zoom";
const CHAT_EXIT: &str = "X unzoom";

fn paint(app: &mut App, width: u16) -> Buffer {
    let mut cells = Buffer::empty(Rect::new(0, 0, width, 45));
    app.frame(&mut cells);
    app.frame_shown(Instant::now());
    cells
}

fn row_text(cells: &Buffer, y: u16) -> String {
    (0..cells.area.width)
        .map(|x| cells[(x, y)].symbol())
        .collect()
}

/// Locate ASCII text by cell, including where the frame has wide glyphs.
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

fn bytes(app: &mut App, value: &[u8]) {
    let at = Instant::now();
    assert!(take_keys(app, Keys::app().feed(value, at)).is_some());
}

fn event(app: &mut App, kind: MouseKind, at: (u16, u16)) {
    event_at(app, kind, at, Instant::now());
}

fn event_at(app: &mut App, kind: MouseKind, (column, row): (u16, u16), at: Instant) {
    assert!(take_keys(app, vec![(Key::Mouse(Mouse { kind, column, row }), at)]).is_some());
}

fn click(app: &mut App, at: (u16, u16)) {
    event(app, MouseKind::Click, at);
    event(app, MouseKind::Release, at);
}

fn side(app: &App, cells: &Buffer) {
    let header = row_text(cells, 0);
    assert!(header.contains('│'), "drawn sidebar rule");
    assert!(fleet_summary(&header), "drawn fleet summary: {header}");
    assert!(
        app.layout.sessions().any(|name| name == "api"),
        "drawn sidebar has its home target"
    );
}

fn zoom(app: &App, cells: &Buffer) {
    let header = row_text(cells, 0);
    assert!(!header.contains('│'), "the sidebar rule is gone: {header}");
    assert!(
        !fleet_summary(&header),
        "the fleet summary is gone: {header}"
    );
    assert!(
        app.layout.sessions().next().is_none(),
        "the hidden fleet has no session hit targets"
    );
}

fn fleet_summary(header: &str) -> bool {
    header.contains("Agt ") || header.contains("Agents ") || header.contains("◈ ")
}

fn chat(root: &Root, lines: &[&str]) -> App {
    session(root, OTHER, "");
    let (mut app, reader) = housed(root);
    refold(
        &mut app,
        &reader,
        root,
        &[("api", Status::Running), (OTHER, Status::Running)],
    );
    supply_turns(&mut app, lines.iter().copied());
    app
}

fn supply_turns<'a>(app: &mut App, lines: impl Iterator<Item = &'a str>) {
    let items = lines
        .enumerate()
        .map(|(n, line)| said(line, i64::try_from(n).expect("small")))
        .collect();
    app.lane = Rc::new(Lane {
        items,
        coverage: Vec::new(),
    });
    app.loading = false;
}

#[test]
fn the_chat_shortcut_and_drawn_header_button_toggle_one_full_frame() {
    let root = Root::new("zoom-chat");
    let mut app = chat(&root, &[CHAT]);
    let before = paint(&mut app, 160);
    side(&app, &before);
    assert!(text(&before).contains(OTHER));
    let split_x = locate(&before, CHAT).0;

    bytes(&mut app, ZOOM);
    let full = paint(&mut app, 160);
    zoom(&app, &full);
    assert!(row_text(&full, 0).contains("api"));
    assert!(!text(&full).contains(OTHER));
    assert!(
        locate(&full, CHAT).0 < split_x,
        "chat uses the sidebar cells"
    );

    let leave = locate(&full, CHAT_EXIT);
    assert_eq!(leave.1, 0, "the exit is in the header");
    click(&mut app, leave);
    let restored = paint(&mut app, 160);
    side(&app, &restored);
    assert_eq!(locate(&restored, CHAT).0, split_x);

    let enter = locate(&restored, CHAT_ENTER);
    assert_eq!(enter.1, 0, "the entrance is in the header");
    click(&mut app, enter);
    let full = paint(&mut app, 160);
    zoom(&app, &full);
    bytes(&mut app, ZOOM);
    let normal = paint(&mut app, 160);
    side(&app, &normal);
}

#[test]
fn a_panel_enters_the_same_zoom_via_f_or_its_header_and_returns_to_side() {
    let root = Root::new("zoom-panel");
    let mut app = sender(&root, Left::Side);
    supply_turns(&mut app, [CHAT].into_iter());
    let before = paint(&mut app, 160);
    side(&app, &before);
    assert!(
        text(&before).contains(CHAT),
        "chat visible beside the panel"
    );
    assert!(
        !turns(&panel_rows(&mut app)).is_empty(),
        "panel has its own turns"
    );

    bytes(&mut app, b"F");
    let full = paint(&mut app, 160);
    zoom(&app, &full);
    let header = row_text(&full, 0);
    assert!(header.contains(ORCH) && header.contains("F side"));
    assert!(!text(&full).contains(CHAT), "panel covers the main chat");
    let scrolled = turns(&panel_rows(&mut app));
    let turn = scrolled.first().expect("panel keeps its lane");
    let at = locate(&full, turn);
    let rule = (0..before.area.width)
        .find(|&x| before[(x, 0)].symbol() == "│")
        .expect("visible sidebar rule");
    assert!(at.0 < rule, "the panel uses the former sidebar cells");
    event(&mut app, MouseKind::WheelDown, at);
    assert_ne!(turns(&panel_rows(&mut app)), scrolled, "the panel scrolls");

    let full = paint(&mut app, 160);
    click(&mut app, locate(&full, "F side"));
    let restored = paint(&mut app, 160);
    side(&app, &restored);
    assert!(text(&restored).contains(CHAT));

    click(&mut app, locate(&restored, "F full"));
    let full = paint(&mut app, 160);
    zoom(&app, &full);
    bytes(&mut app, ZOOM);
    let restored = paint(&mut app, 160);
    side(&app, &restored);
    assert!(text(&restored).contains(CHAT));

    bytes(&mut app, ZOOM);
    let full = paint(&mut app, 160);
    zoom(&app, &full);
    bytes(&mut app, b"F");
    let restored = paint(&mut app, 160);
    side(&app, &restored);
    assert!(text(&restored).contains(CHAT));
}

#[test]
fn the_hidden_sidebar_has_no_click_or_menu_targets_and_its_cells_scroll_chat() {
    let root = Root::new("zoom-input");
    let rows: Vec<String> = (0..80).map(|n| format!("zoom-turn-{n:02}")).collect();
    let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
    let mut app = chat(&root, &refs);
    let before = paint(&mut app, 160);
    let stale = locate(&before, OTHER);
    let rule = (0..before.area.width)
        .find(|&x| before[(x, 0)].symbol() == "│")
        .expect("visible sidebar rule");
    event(&mut app, MouseKind::Menu, stale);
    assert!(app.modal.is_some(), "the visible card has a menu");
    let _ = paint(&mut app, 160);
    let _ = take_keys(&mut app, vec![(Key::Escape, Instant::now())]);
    assert!(app.modal.is_none());
    let before_zoom = Instant::now();

    bytes(&mut app, ZOOM);
    let full = paint(&mut app, 160);
    zoom(&app, &full);
    event_at(&mut app, MouseKind::Menu, stale, before_zoom);
    assert!(
        app.modal.is_none(),
        "a delayed pre-zoom right press is stale"
    );
    click(&mut app, stale);
    event(&mut app, MouseKind::Menu, stale);
    assert_eq!(app.model.selected(), Some("api"));
    assert!(app.modal.is_none(), "the hidden card cannot open its menu");

    let newest = locate(&full, "zoom-turn-79");
    assert!(newest.0 < rule, "the turn is in former sidebar cells");
    event(&mut app, MouseKind::WheelUp, newest);
    let scrolled = paint(&mut app, 160);
    assert!(
        !text(&scrolled).contains("zoom-turn-79"),
        "chat scrolled up"
    );
    event(&mut app, MouseKind::WheelDown, newest);
    let back = paint(&mut app, 160);
    assert!(text(&back).contains("zoom-turn-79"), "chat scrolled back");
}

#[test]
fn the_resized_zoom_maps_composer_drag_and_preserves_plain_keys_and_escape() {
    let root = Root::new("zoom-composer");
    let mut app = chat(&root, &[CHAT]);
    let _ = paint(&mut app, 160);
    bytes(&mut app, ZOOM);
    let _ = paint(&mut app, 160);
    let _ = writing(&mut app);
    bytes(&mut app, b"zoom-copy X F tail");
    let draft = drafted(&app);
    assert_eq!(
        draft, "zoom-copy X F tail",
        "printable keys stay draft text"
    );
    bytes(&mut app, b"\x1by");
    assert_eq!(app.clip.as_deref(), Some(draft.as_str()));
    bytes(&mut app, b"\x1bo");
    assert_eq!(drafted(&app), draft, "Alt+o without a warning is inert");
    let wide = paint(&mut app, 160);
    zoom(&app, &wide);
    let narrow = paint(&mut app, 90);
    zoom(&app, &narrow);
    assert!(
        row_text(&narrow, 0).contains(CHAT_EXIT),
        "exit survives resize"
    );
    let from = locate(&narrow, "zoom-copy");
    let to = (from.0 + 8, from.1);
    event(&mut app, MouseKind::Click, from);
    event(&mut app, MouseKind::Drag, to);
    event(&mut app, MouseKind::Release, to);
    let _ = paint(&mut app, 90);
    assert!(app.ship_staged(), "the painted selection ships");
    assert_eq!(app.clip.as_deref(), Some("zoom-copy"));
    assert_eq!(drafted(&app), draft);
    assert!(app.composing() && app.queued.is_empty());

    let _ = take_keys(&mut app, vec![(Key::Escape, Instant::now())]);
    assert!(app.composing(), "first Esc clears only the selection");
    let full = paint(&mut app, 90);
    click(&mut app, locate(&full, CHAT_EXIT));
    let normal = paint(&mut app, 90);
    side(&app, &normal);
    assert!(app.composing(), "click unzoom keeps writing");
    click(&mut app, locate(&normal, CHAT_ENTER));
    let full = paint(&mut app, 90);
    zoom(&app, &full);
    assert!(app.composing(), "click zoom keeps writing");
    assert_eq!(drafted(&app), draft);

    let _ = take_keys(&mut app, vec![(Key::Escape, Instant::now())]);
    assert!(!app.composing(), "second Esc returns to browsing");
    assert_eq!(drafted(&app), draft);
    let full = paint(&mut app, 90);
    zoom(&app, &full);
    click(&mut app, locate(&full, CHAT_EXIT));
    let normal = paint(&mut app, 90);
    side(&app, &normal);
}

#[test]
fn a_file_link_opens_a_reader_that_can_zoom_and_escape_without_losing_zoom() {
    let root = Root::new("zoom-reader");
    let body = format!("open {LINK} here");
    let mut app = chat(&root, &[&body]);
    let before = paint(&mut app, 160);
    side(&app, &before);

    bytes(&mut app, ZOOM);
    let full = paint(&mut app, 160);
    zoom(&app, &full);
    click(&mut app, locate(&full, LINK));
    assert!(app.model.reader_open(), "the link under the mouse opens");
    let reader = paint(&mut app, 160);
    zoom(&app, &reader);
    assert!(
        text(&reader).contains("HTML page"),
        "the linked file is shown"
    );

    click(&mut app, locate(&reader, CHAT_EXIT));
    let side_reader = paint(&mut app, 160);
    side(&app, &side_reader);
    assert!(app.model.reader_open(), "unzoom leaves the reader open");
    bytes(&mut app, ZOOM);
    let reader_again = paint(&mut app, 160);
    zoom(&app, &reader_again);
    assert!(app.model.reader_open());
    let _ = take_keys(&mut app, vec![(Key::Escape, Instant::now())]);
    assert!(!app.model.reader_open(), "Esc closes the reader");
    let chat_again = paint(&mut app, 160);
    zoom(&app, &chat_again);
    assert!(text(&chat_again).contains(&body));
}

#[test]
fn a_narrow_panel_fallback_is_distinct_from_explicit_zoom() {
    let root = Root::new("zoom-fallback");
    let mut app = sender(&root, Left::Side);
    let narrow = paint(&mut app, 90);
    side(&app, &narrow);
    assert!(row_text(&narrow, 0).contains("F full"));
    assert!(text(&narrow).contains("side needs"));

    bytes(&mut app, b"F");
    let zoomed_narrow = paint(&mut app, 90);
    zoom(&app, &zoomed_narrow);
    assert!(row_text(&zoomed_narrow, 0).contains("F side"));
    let zoomed_wide = paint(&mut app, 160);
    zoom(&app, &zoomed_wide);

    bytes(&mut app, b"F");
    let side_wide = paint(&mut app, 160);
    side(&app, &side_wide);
    assert!(
        app.layout.panel.is_some(),
        "the open panel returns to its side"
    );
}

#[test]
fn opening_the_panel_from_zoom_and_closing_it_restores_the_zoomed_chat() {
    let root = Root::new("zoom-switch");
    let mut app = sender(&root, Left::SideMain);
    app.answer(view("api", ID, 1, CHAT));
    let normal = paint(&mut app, 160);
    side(&app, &normal);
    assert!(app.layout.panel.is_some() && !app.panel_focused());

    bytes(&mut app, ZOOM);
    let chat_full = paint(&mut app, 160);
    zoom(&app, &chat_full);
    assert!(app.layout.panel.is_none() && text(&chat_full).contains(CHAT));

    bytes(&mut app, b"O");
    let panel_full = paint(&mut app, 160);
    zoom(&app, &panel_full);
    assert!(app.panel_focused());
    assert!(row_text(&panel_full, 0).contains("F side"));
    assert!(!text(&panel_full).contains(CHAT));

    let _ = take_keys(&mut app, vec![(Key::Escape, Instant::now())]);
    let chat_again = paint(&mut app, 160);
    zoom(&app, &chat_again);
    assert!(app.orch.panel.is_none(), "Esc closes the panel as before");
    assert!(text(&chat_again).contains(CHAT));
}

#[test]
fn both_restart_carriers_preserve_the_zoomed_surface() {
    for carrier in [Carrier::Exec, Carrier::Option] {
        for (left, shortcut, panel_zoom) in [
            (Left::Closed, ZOOM, false),
            (Left::SideMain, ZOOM, false),
            (Left::Side, b"F".as_slice(), true),
            (Left::SideMain, b"XO".as_slice(), true),
        ] {
            let root = Root::new("zoom-restart");
            let mut before = sender(&root, left);
            let normal = paint(&mut before, 160);
            side(&before, &normal);
            bytes(&mut before, shortcut);
            let (_, keep) = restart(&mut before, &root);
            let mut after = receiver(&root, keep, carrier, loader::Proof::Proven(OID.to_owned()));
            let restored = paint(&mut after, 160);
            zoom(&after, &restored);
            assert!(row_text(&restored, 0).contains(if panel_zoom { "F side" } else { CHAT_EXIT }));
            if shortcut == b"XO" {
                bytes(&mut after, b"O");
                let main = paint(&mut after, 160);
                zoom(&after, &main);
                assert!(row_text(&main, 0).contains(CHAT_EXIT));
            }
            bytes(&mut after, if shortcut == b"XO" { ZOOM } else { shortcut });
            let normal = paint(&mut after, 160);
            side(&after, &normal);
            if left == Left::SideMain && !panel_zoom {
                assert!(after.layout.panel.is_some(), "the side panel was kept");
                assert!(!after.panel_focused(), "the chat keeps the keys");
            }
        }
    }
}
