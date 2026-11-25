//! Panel links: a link the open orchestrator panel draws reaches the terminal
//! as the same hyperlink run a chat-column link is, in every geometry the
//! panel stands in, and no link the main column drew under the panel stays.
//!
//! Oracle: docs/app.md "Links": a web URL or an absolute path in a turn is a
//! terminal hyperlink, and the backend wraps exactly the cells of
//! `app.layout.links` (the frame the run loop hands it) in OSC 8. The panel is
//! drawn by the chat's own renderer, so its turns are links as the chat
//! column's are. A hyperlink on a cell the panel covers would send the human
//! to a URL the cell does not say.

use super::*;
use crate::app::backend::Run;

const PANEL_URL: &str = "https://example.com/panel-proof";
const PANEL_FILE: &str = "/tmp/panel-proof.md";
const CHAT_URL: &str = "https://example.com/chat-proof";

/// The lane of the orchestrator panel: one turn per link.
fn panel_lane() -> Answer {
    let items = [PANEL_URL, PANEL_FILE]
        .into_iter()
        .enumerate()
        .map(|(n, link)| said(&format!("see {link} now"), i64::try_from(n).expect("small")))
        .collect();
    Answer::View(ViewRead {
        name: ORCH.to_owned(),
        id: OID.to_owned(),
        seq: 2,
        lane: Lane {
            items,
            coverage: Vec::new(),
        },
        needs: None,
        roster: None,
    })
}

/// An app with the panel as `left`, the panel's lane and the main chat's
/// lane each holding links.
fn linked(root: &Root, left: Left) -> App {
    let mut app = sender(root, left);
    app.answer(panel_lane());
    let body = format!("open {CHAT_URL} here");
    app.lane = Rc::new(Lane {
        items: vec![said(&body, 0)],
        coverage: Vec::new(),
    });
    app.loading = false;
    app
}

fn paint(app: &mut App, width: u16) -> Buffer {
    let mut cells = Buffer::empty(Rect::new(0, 0, width, 45));
    app.frame(&mut cells);
    app.frame_shown(Instant::now());
    cells
}

/// Where the ASCII `needle` is drawn, first row then first column; `None`
/// when no cell row holds it.
fn find(cells: &Buffer, needle: &str) -> Option<(u16, u16)> {
    let width = u16::try_from(needle.len()).expect("short needle");
    (0..cells.area.height).find_map(|y| {
        (0..=cells.area.width.saturating_sub(width)).find_map(|x| {
            let at: String = (x..x + width).map(|x| cells[(x, y)].symbol()).collect();
            (at == needle).then_some((x, y))
        })
    })
}

/// The cells `run` wraps.
fn under(cells: &Buffer, run: &Run) -> String {
    (run.x..run.x + run.width)
        .map(|x| cells[(x, run.y)].symbol())
        .collect()
}

/// The run the frame hands the backend for `link` drawn as `url`, asserted
/// to wrap exactly the cells the link is drawn in.
fn assert_linked(app: &App, cells: &Buffer, link: &str, url: &str) {
    let at = find(cells, link).unwrap_or_else(|| panic!("{link:?} is not drawn:\n{}", text(cells)));
    let run = app.layout.links.iter().find(|run| run.url == url);
    let run = run.unwrap_or_else(|| panic!("no run for {url:?} in {:?}", app.layout.links));
    assert_eq!((run.x, run.y), at, "the run starts where {link:?} is drawn");
    assert_eq!(under(cells, run), link, "the run wraps exactly the link");
}

/// No run the frame hands the backend lies on a cell of the panel unless it
/// is one of the panel's own links.
fn assert_none_under_panel(app: &App, cells: &Buffer) {
    let panel = app.layout.panel.expect("the panel is drawn");
    for run in &app.layout.links {
        let inside = Rect::new(run.x, run.y, run.width, 1).intersects(panel);
        let own = run.url == PANEL_URL || run.url == "file:///tmp/panel-proof.md";
        assert!(
            !inside || own,
            "{:?} wraps {:?}, a cell the panel covers",
            run.url,
            under(cells, run)
        );
    }
}

#[test]
fn a_link_in_the_side_panel_is_a_terminal_link_and_the_chat_keeps_its_own() {
    let root = Root::new("panellinks-side");
    let mut app = linked(&root, Left::Side);
    let cells = paint(&mut app, 160);
    assert!(app.layout.panel.is_some(), "the side panel stands");
    assert_linked(&app, &cells, PANEL_URL, PANEL_URL);
    assert_linked(&app, &cells, PANEL_FILE, "file:///tmp/panel-proof.md");
    assert_linked(&app, &cells, CHAT_URL, CHAT_URL);
    assert_none_under_panel(&app, &cells);
}

#[test]
fn a_link_in_the_full_panel_is_a_terminal_link_and_no_chat_link_shows_through() {
    let root = Root::new("panellinks-full");
    let mut app = linked(&root, Left::Full);
    let cells = paint(&mut app, 160);
    assert!(
        find(&cells, CHAT_URL).is_none(),
        "the panel covers the chat"
    );
    assert_linked(&app, &cells, PANEL_URL, PANEL_URL);
    assert_linked(&app, &cells, PANEL_FILE, "file:///tmp/panel-proof.md");
    assert!(
        app.layout.links.iter().all(|run| run.url != CHAT_URL),
        "a covered chat link is no run: {:?}",
        app.layout.links
    );
    assert_none_under_panel(&app, &cells);
}

#[test]
fn a_link_in_the_zoomed_panel_is_a_terminal_link_and_no_chat_link_shows_through() {
    let root = Root::new("panellinks-zoom");
    let mut app = linked(&root, Left::Side);
    let at = Instant::now();
    assert!(take_keys(&mut app, crate::console::input::Keys::app().feed(b"F", at)).is_some());
    let cells = paint(&mut app, 160);
    assert!(
        !row_has_rule(&cells),
        "the sidebar rule is gone: the panel is zoomed over the whole frame"
    );
    assert!(
        find(&cells, CHAT_URL).is_none(),
        "the panel covers the chat"
    );
    assert_linked(&app, &cells, PANEL_URL, PANEL_URL);
    assert_linked(&app, &cells, PANEL_FILE, "file:///tmp/panel-proof.md");
    assert!(
        app.layout.links.iter().all(|run| run.url != CHAT_URL),
        "a covered chat link is no run: {:?}",
        app.layout.links
    );
    assert_none_under_panel(&app, &cells);
}

fn row_has_rule(cells: &Buffer) -> bool {
    (0..cells.area.width).any(|x| cells[(x, 0)].symbol() == "│")
}

#[test]
fn a_link_in_the_narrow_fallback_panel_is_a_terminal_link_and_no_chat_link_shows_through() {
    let root = Root::new("panellinks-narrow");
    let mut app = linked(&root, Left::Side);
    let cells = paint(&mut app, 90);
    assert!(
        text(&cells).contains("side needs"),
        "the side panel fell back"
    );
    assert!(
        find(&cells, CHAT_URL).is_none(),
        "the panel covers the chat"
    );
    assert_linked(&app, &cells, PANEL_URL, PANEL_URL);
    assert_linked(&app, &cells, PANEL_FILE, "file:///tmp/panel-proof.md");
    assert!(
        app.layout.links.iter().all(|run| run.url != CHAT_URL),
        "a covered chat link is no run: {:?}",
        app.layout.links
    );
    assert_none_under_panel(&app, &cells);
}

#[test]
fn the_settings_overlay_covers_every_link_the_chat_and_the_panel_drew() {
    let root = Root::new("panellinks-settings");
    let mut app = linked(&root, Left::Side);
    let _ = paint(&mut app, 160);
    assert!(!app.layout.links.is_empty(), "the links stand before");
    app.open_settings();
    let cells_settings = paint(&mut app, 160);
    assert!(app.model.settings_open(), "the overlay is open");
    assert!(
        find(&cells_settings, CHAT_URL).is_none() && find(&cells_settings, PANEL_URL).is_none(),
        "the overlay covers the links:\n{}",
        text(&cells_settings)
    );
    assert!(
        app.layout.links.is_empty(),
        "no run wraps a cell the overlay painted: {:?}\n{}",
        app.layout.links,
        text(&cells_settings)
    );
}

#[test]
fn a_dialog_over_the_panel_and_the_chat_leaves_no_link_on_its_cells() {
    use crate::app::menu::{Dialog, Modal, Records};
    use crate::session_menu::CellRow;

    let root = Root::new("panellinks-modal");
    let mut app = linked(&root, Left::Side);
    let wide = "wide ".repeat(16);
    let rows = (1..=40)
        .map(|n| {
            let text = format!("{wide}{n:02}");
            let cells = [">7d", "lead", "goal", text.as_str()];
            CellRow::Cells(cells.map(str::to_owned).to_vec())
        })
        .collect();
    app.modal = Some(Modal::Records(Records::new(Dialog::Activity, "api", rows)));
    let cells = paint(&mut app, 160);
    assert!(app.layout.shows_modal(), "the dialog is painted");
    for link in [PANEL_URL, PANEL_FILE, CHAT_URL] {
        assert!(
            find(&cells, link).is_none(),
            "the dialog covers {link}:\n{}",
            text(&cells)
        );
    }
    for run in &app.layout.links {
        let said = under(&cells, run);
        assert!(
            run.url.contains(said.trim()),
            "{:?} wraps {said:?}, a cell the dialog painted: {:?}",
            run.url,
            app.layout.links
        );
    }
}

#[test]
fn a_dialog_keeps_of_each_link_only_the_cells_it_leaves() {
    use crate::app::draw::{Hit, Layout};

    let run = |x, y, width| Run {
        x,
        y,
        width,
        url: "https://example.com/u".to_owned(),
    };
    let mut layout = Layout::default();
    layout.links.extend([
        run(2, 3, 20),
        run(2, 4, 4),
        run(30, 3, 5),
        run(10, 5, 6),
        run(6, 4, 6),
        run(9, 3, 3),
        run(8, 4, 6),
    ]);
    let buf = Buffer::empty(Rect::new(0, 0, 80, 10));
    layout.overlay(&buf, Rect::new(8, 3, 6, 2), Hit::Modal);
    assert_eq!(
        layout.links,
        vec![
            run(2, 3, 6),
            run(14, 3, 8),
            run(2, 4, 4),
            run(30, 3, 5),
            run(10, 5, 6),
            run(6, 4, 2),
        ],
        "a cut run keeps its left and right cells, a covered one goes, the rest stand"
    );
    layout.overlay(&buf, Rect::new(2, 4, 2, 1), Hit::Item(0));
    assert_eq!(
        layout.links.len(),
        6,
        "only a dialog takes cells from links"
    );
}
