//! Unit cases for the orchestrator panel's look and its full stand: the
//! panel wears the sidebar's own ink for its border, grab handle and header,
//! and the full panel covers the main column alone. Every oracle is a cell
//! of the same rendered frame (the sidebar's rule or the main column's
//! header) or of the frame of a peer app, never a colour typed here. A
//! child of `app::tests`, driven by hand without tmux.

use super::o7app_units::{at as frame, border_of, cell_of, opened, orchestrated, rule_of};
use super::*;
use crate::theme::{Look, Palette};
use ratatui_core::style::{Color, Modifier};

const WIDTH: u16 = 160;
const HEIGHT: u16 = 45;
const ORCH: &str = crate::orchestrator::ORCHESTRATOR_SESSION;

/// One look for each palette the app draws.
fn looks() -> [Look; 3] {
    [Palette::DARCULA, Palette::NEUTRAL, Palette::WARM].map(|palette| Look {
        palette,
        ..Look::DEFAULT
    })
}

/// What a cell wears.
fn worn(buf: &Buffer, x: u16, y: u16) -> (Color, Color, Modifier) {
    let cell = &buf[(x, y)];
    (cell.fg, cell.bg, cell.modifier)
}

/// A standalone `ae app`: no home session, `look` and the viewer's zone read
/// for the selected `api`, the panel open on a running orchestrator. The
/// orchestrator has no look entry of its own, as in a live fleet.
fn standing(root: &Root, full: bool, look: Look) -> (App, Reader) {
    let (mut app, reader) = orchestrated(root, full);
    app.home = None;
    let read = (Some(look), Some("+0200".to_owned()));
    app.looks.insert("api".to_owned(), read);
    assert_eq!(app.model.selected(), Some("api"));
    (app, reader)
}

/// Where the sidebar's rule stands in a frame of this size with no panel
/// open, the geometry no panel covers.
fn rule_without_panel() -> u16 {
    let mut closed = app(None);
    two(&mut closed);
    rule_of(&frame(WIDTH, HEIGHT, &mut closed))
}

/// The panel's border is drawn in the ink of the sidebar's rule, in every
/// palette, never in the terminal's default foreground.
#[test]
fn the_side_panels_border_wears_the_sidebar_rules_ink() {
    for look in looks() {
        let name = look.palette.name;
        let root = Root::new(&format!("o8look-border-{name}"));
        let (mut app, _reader) = standing(&root, false, look);
        let buf = frame(WIDTH, HEIGHT, &mut app);
        let (rule, border) = (rule_of(&buf), border_of(&buf));
        assert!(rule < border, "{name}: the rule stands left of the border");
        let idle = worn(&buf, rule, 3);
        assert_ne!(
            idle.0,
            Color::Reset,
            "{name}: the oracle wears a palette ink"
        );
        for y in 0..HEIGHT {
            assert_eq!(buf[(border, y)].symbol(), "│", "{name}: row {y}");
            assert_eq!(
                worn(&buf, border, y),
                worn(&buf, rule, y),
                "{name}: row {y}\n{}",
                text(&buf)
            );
        }
    }
}

/// A grabbed panel border wears the grabbed sidebar rule's ink, and that ink
/// is not the idle one.
#[test]
fn a_grabbed_panel_border_wears_the_ink_a_grabbed_sidebar_rule_does() {
    for look in looks() {
        let name = look.palette.name;
        let grab = |edge: Edge, tag: &str| {
            let root = Root::new(&format!("o8look-grab-{tag}-{name}"));
            let (mut app, _reader) = standing(&root, false, look);
            let idle = frame(WIDTH, HEIGHT, &mut app);
            app.model.grab(Drag {
                edge,
                from: 0,
                at: 0,
            });
            (idle, frame(WIDTH, HEIGHT, &mut app))
        };
        let (idle, sidebar) = grab(Edge::Sidebar, "sidebar");
        let (_, panel) = grab(Edge::Panel, "panel");
        let (rule, border) = (rule_of(&idle), border_of(&idle));
        for y in 2..HEIGHT - 5 {
            let grabbed = worn(&sidebar, rule, y);
            assert_ne!(grabbed, worn(&idle, rule, y), "{name}: row {y} is lit");
            assert_eq!(worn(&panel, border, y), grabbed, "{name}: row {y}");
        }
    }
}

/// The panel's header names wear the main column's: the column that holds
/// the keys has its name in the title ink, the other one dim.
#[test]
fn the_panel_header_wears_the_main_columns_name_ink() {
    for look in looks() {
        let name = look.palette.name;
        let shot = |keys_on_panel: bool| {
            let root = Root::new(&format!("o8look-head-{keys_on_panel}-{name}"));
            let (mut app, _reader) = standing(&root, false, look);
            app.focus_column(keys_on_panel);
            frame(WIDTH, HEIGHT, &mut app)
        };
        let (panel_keys, main_keys) = (shot(true), shot(false));
        let border = border_of(&panel_keys);
        let panel = cell_of(&panel_keys, "orchestrator", 0, border).expect("the panel's name");
        let main = cell_of(&panel_keys, "api", 0, 0).expect("the main column's name");
        assert_eq!(
            worn(&panel_keys, panel, 0),
            worn(&main_keys, main, 0),
            "{name}: the focused name"
        );
        assert_eq!(
            worn(&main_keys, panel, 0),
            worn(&panel_keys, main, 0),
            "{name}: the other name"
        );
        assert_ne!(
            worn(&panel_keys, panel, 0),
            worn(&main_keys, panel, 0),
            "{name}: focus moves the ink"
        );
    }
}

/// No cell of the panel is left in the terminal's default colours where the
/// main column's cells are not: the ground and every ink come from the look.
#[test]
fn the_panel_leaves_no_cell_in_the_terminals_default_colours() {
    for look in looks() {
        let name = look.palette.name;
        let root = Root::new(&format!("o8look-bare-{name}"));
        let (mut app, _reader) = standing(&root, false, look);
        let buf = frame(WIDTH, HEIGHT, &mut app);
        let (rule, border) = (rule_of(&buf), border_of(&buf));
        let bare = |from: u16, to: u16| {
            let cells = (0..HEIGHT).flat_map(|y| (from..to).map(move |x| (x, y)));
            let cells: Vec<_> = cells.collect();
            let ground = cells
                .iter()
                .filter(|(x, y)| buf[(*x, *y)].bg == Color::Reset);
            let ink = cells
                .iter()
                .filter(|(x, y)| buf[(*x, *y)].fg == Color::Reset && buf[(*x, *y)].symbol() != " ");
            (ground.count(), ink.count())
        };
        assert_eq!(
            bare(border, WIDTH),
            bare(rule + 1, border),
            "{name}: the panel's bare cells (ground, ink) against the main column's"
        );
    }
}

/// The panel's clock reads in the zone the main column's does.
#[test]
fn the_panel_clock_reads_in_the_main_columns_zone() {
    let root = Root::new("o8look-zone");
    let (mut app, _reader) = standing(&root, false, Look::DEFAULT);
    app.answer(view(ORCH, ID, 100, "PANELWORDS"));
    app.answer(view("api", ID, 100, "MAINWORDS"));
    let buf = frame(WIDTH, HEIGHT, &mut app);
    let (rule, border) = (rule_of(&buf), border_of(&buf));
    let region = |from: u16, to: u16| -> String {
        (0..HEIGHT)
            .map(|y| (from..to).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let (main, panel) = (region(rule + 1, border), region(border + 1, WIDTH));
    assert!(main.contains("MAINWORDS") && panel.contains("PANELWORDS"));
    // 14:00 UTC, one fixture instant read in the viewer's +0200.
    assert!(main.contains("16:00"), "the oracle reads local: {main}");
    assert!(
        panel.contains("16:00") && !panel.contains("14:00"),
        "{panel}"
    );
}

/// A full panel covers the main column and nothing left of it: the sidebar's
/// cells, rule included, are those of the same frame beside a side panel.
#[test]
fn a_full_panel_leaves_the_sidebar_as_it_is_drawn_beside_a_side_panel() {
    for look in looks() {
        let name = look.palette.name;
        let drawn = |full: bool| {
            let root = Root::new(&format!("o8look-full-{full}-{name}"));
            let (mut app, _reader) = standing(&root, full, look);
            frame(WIDTH, HEIGHT, &mut app)
        };
        let (side, full) = (drawn(false), drawn(true));
        let rule = rule_of(&side);
        for y in 0..HEIGHT {
            for x in 0..=rule {
                assert_eq!(
                    full[(x, y)],
                    side[(x, y)],
                    "{name}: sidebar cell {x},{y}\n{}",
                    text(&full)
                );
            }
        }
        assert_eq!(rule_of(&full), rule, "{name}: the rule stays");
        let left = |buf: &Buffer| -> String {
            (0..HEIGHT)
                .map(|y| (0..rule).map(|x| buf[(x, y)].symbol()).collect::<String>())
                .collect()
        };
        assert!(
            left(&full).contains("api"),
            "{name}: the session list is drawn"
        );
        let mode = cell_of(&full, "F side", 0, 0).expect("the full panel's mode label");
        let panel = cell_of(&full, "orchestrator", 0, 0).expect("the panel's name");
        assert!(
            panel > rule && mode > panel,
            "{name}: the panel starts right of the rule"
        );
    }
}

/// A full panel's ground is the ground of the sidebar's rule cells in the
/// frame beside a side panel: the look's, never the terminal's default.
#[test]
fn a_full_panel_wears_the_ground_of_the_sidebar_rule() {
    for look in looks() {
        let name = look.palette.name;
        let drawn = |full: bool| {
            let root = Root::new(&format!("o8look-fullink-{full}-{name}"));
            let (mut app, _reader) = standing(&root, full, look);
            frame(WIDTH, HEIGHT, &mut app)
        };
        let (side, full) = (drawn(false), drawn(true));
        let rule = rule_of(&side);
        for y in 2..HEIGHT - 5 {
            let ground = worn(&side, rule, y).1;
            assert_ne!(ground, Color::Reset, "{name}: the oracle has a ground");
            assert_eq!(full[(rule + 4, y)].bg, ground, "{name}: ground, row {y}");
        }
    }
}

/// A press on a sidebar row under a full panel acts as in the main column:
/// it selects the row, and the panel turns side so the selection shows, with
/// the keys on the main column, as `!` does.
#[test]
fn a_press_on_a_sidebar_row_under_a_full_panel_selects_it_and_turns_the_panel_side() {
    let mut app = app(None);
    two(&mut app);
    opened(&mut app, true);
    let buf = frame(WIDTH, HEIGHT, &mut app);
    let rule = rule_without_panel();
    let (x, y) = (0..HEIGHT)
        .find_map(|y| {
            cell_of(&buf, "ops", y, 0)
                .filter(|x| *x < rule)
                .map(|x| (x, y))
        })
        .expect("the ops row left of the rule");
    let redraw = app.mouse(Mouse {
        kind: MouseKind::Click,
        column: x,
        row: y,
    });
    assert!(redraw, "the press redraws");
    assert_eq!(app.model.selected(), Some("ops"), "the row is selected");
    let full = app.orch.panel.as_ref().is_some_and(|panel| panel.full);
    assert!(!full, "the panel turned side");
    assert!(!app.panel_focused(), "the main column holds the keys");
}

/// A press on the sidebar's rule grabs it and leaves the panel full, with
/// the keys where they were.
#[test]
fn a_press_on_the_sidebar_rule_under_a_full_panel_grabs_it_and_keeps_the_panel_full() {
    let mut app = app(None);
    two(&mut app);
    opened(&mut app, true);
    let _ = frame(WIDTH, HEIGHT, &mut app);
    let rule = rule_without_panel();
    let _ = app.mouse(Mouse {
        kind: MouseKind::Click,
        column: rule,
        row: 20,
    });
    assert_eq!(app.model.drag().map(|drag| drag.edge), Some(Edge::Sidebar));
    let full = app.orch.panel.as_ref().is_some_and(|panel| panel.full);
    assert!(
        full && app.panel_focused(),
        "full, with the keys on the panel"
    );
}
