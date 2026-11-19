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
fn standing(root: &Root, full: bool, look: &Look) -> (App, Reader) {
    let (mut app, reader) = orchestrated(root, full);
    app.home = None;
    let read = (Some(*look), Some("+0200".to_owned()));
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
        let (mut app, _reader) = standing(&root, false, &look);
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
            let (mut app, _reader) = standing(&root, false, &look);
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
            let (mut app, _reader) = standing(&root, false, &look);
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
        let (mut app, _reader) = standing(&root, false, &look);
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
    let (mut app, _reader) = standing(&root, false, &Look::DEFAULT);
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
            let (mut app, _reader) = standing(&root, full, &look);
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
            let (mut app, _reader) = standing(&root, full, &look);
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

/// An undrawn look (`theme = off`) paints no colour in either column: the
/// panel's border is as plain as the sidebar's rule, idle and grabbed.
#[test]
fn an_undrawn_look_leaves_the_panel_border_as_plain_as_the_sidebar_rule() {
    let look = Look {
        drawn: false,
        ..Look::DEFAULT
    };
    let shot = |edge: Option<Edge>, tag: &str| {
        let root = Root::new(&format!("o8look-undrawn-{tag}"));
        let (mut app, _reader) = standing(&root, false, &look);
        if let Some(edge) = edge {
            app.model.grab(Drag {
                edge,
                from: 0,
                at: 0,
            });
        }
        frame(WIDTH, HEIGHT, &mut app)
    };
    let (idle, sidebar, panel) = (
        shot(None, "idle"),
        shot(Some(Edge::Sidebar), "sidebar"),
        shot(Some(Edge::Panel), "panel"),
    );
    let (rule, border) = (rule_of(&idle), border_of(&idle));
    for y in 2..HEIGHT - 5 {
        assert_eq!(
            worn(&idle, border, y),
            worn(&idle, rule, y),
            "idle, row {y}"
        );
        assert_eq!(
            worn(&panel, border, y),
            worn(&sidebar, rule, y),
            "grabbed, row {y}"
        );
    }
}

/// A wheel notch over the sidebar's list under a full panel scrolls the list
/// and leaves the panel full: the notch is the sidebar's, not the panel's.
#[test]
fn a_wheel_notch_over_the_sidebar_list_under_a_full_panel_scrolls_the_list() {
    let mut app = app(None);
    app.fleet = Fleet {
        rows: (0..30)
            .map(|at| row(&format!("n{at:02}"), at + 1, false))
            .collect(),
        home: None,
    };
    app.model = Model::new(&app.fleet);
    opened(&mut app, true);
    let buf = frame(WIDTH, HEIGHT, &mut app);
    let rule = rule_without_panel();
    // The first card of the list: its number and the row it stands on.
    let first = |buf: &Buffer| -> Option<(u16, u8)> {
        (1..HEIGHT).find_map(|y| {
            let line: String = (0..rule).map(|x| buf[(x, y)].symbol()).collect();
            let number = line
                .split_whitespace()
                .find_map(|word| word.strip_prefix('n')?.parse().ok())?;
            Some((y, number))
        })
    };
    let (y, top) = first(&buf).expect("a card in the sidebar list");
    let notch = Mouse {
        kind: MouseKind::WheelDown,
        column: 2,
        row: y,
    };
    assert!(app.mouse(notch), "the notch scrolls the list");
    let after = frame(WIDTH, HEIGHT, &mut app);
    assert_eq!(
        first(&after).map(|card| card.1),
        Some(top + 1),
        "by one session"
    );
    assert!(
        app.orch.panel.as_ref().is_some_and(|panel| panel.full),
        "still full"
    );
}

/// What a drag of the sidebar's rule left behind: the width it asked for, the
/// rule and the panel's left edge in the frame drawn after it, and whether a
/// drag still stood after the release.
struct Dragged {
    asked: Option<u16>,
    rule: u16,
    edge: Option<u16>,
    held: bool,
}

/// Press the sidebar's rule at `rule` in a frame `width` wide and move the
/// pointer `by` columns, one cell at a time with a redraw after each event as
/// the live loop does, then let go one cell right of the rule the last frame
/// drew: over the panel when one covers the main column there.
fn drag_rule(app: &mut App, width: u16, rule: u16, by: i32) -> Dragged {
    let mouse = |kind, column: u16| Mouse {
        kind,
        column,
        row: 20,
    };
    let _ = app.mouse(mouse(MouseKind::Click, rule));
    assert_eq!(app.model.drag().map(|drag| drag.edge), Some(Edge::Sidebar));
    let step = if by < 0 { -1 } else { 1 };
    let mut column = rule;
    for _ in 0..by.abs() {
        column = u16::try_from(i32::from(column) + step).expect("a frame column");
        let _ = app.mouse(mouse(MouseKind::Drag, column));
        let _ = frame(width, HEIGHT, app);
    }
    let over = rule_of(&frame(width, HEIGHT, app)) + 1;
    let _ = app.mouse(mouse(MouseKind::Release, over));
    let buf = frame(width, HEIGHT, app);
    Dragged {
        asked: app.model.split().sidebar,
        rule: rule_of(&buf),
        edge: app.layout.panel.map(|cells| cells.x),
        held: app.model.drag().is_some(),
    }
}

/// A drag that grabbed the sidebar's rule moves it the same way with no panel,
/// beside a side panel and under a full one, each way: the full panel covers
/// the main column the pointer moves through, so it must take neither the
/// motion nor the release. The drawn rule follows the pointer and a full
/// panel keeps its left edge one cell right of the rule.
#[test]
fn a_drag_of_the_sidebar_rule_moves_it_under_a_full_panel_as_it_does_beside_a_side_panel() {
    let rule = rule_without_panel();
    let mut wrong = Vec::new();
    for panel in [None, Some(false), Some(true)] {
        for by in [6, -6] {
            let mut app = app(None);
            two(&mut app);
            if let Some(full) = panel {
                opened(&mut app, full);
            }
            let _ = frame(WIDTH, HEIGHT, &mut app);
            let want = u16::try_from(i32::from(rule) + by).expect("a frame column");
            let left = drag_rule(&mut app, WIDTH, rule, by);
            let mut expect = |ok: bool, what: &str| {
                if !ok {
                    wrong.push(format!("panel {panel:?}, by {by}: {what}"));
                }
            };
            expect(left.asked == Some(want), &format!("asked {:?}", left.asked));
            expect(
                left.rule == want,
                &format!("drew the rule at {}", left.rule),
            );
            expect(!left.held, "the release left the drag held");
            if panel == Some(true) {
                expect(
                    left.edge == Some(want + 1),
                    &format!("panel at {:?}", left.edge),
                );
            }
        }
    }
    assert!(wrong.is_empty(), "want the rule at {rule} +- 6: {wrong:#?}");
}

/// Under a full panel the dragged rule keeps its documented bounds: the
/// sidebar at least 30 cells and the chat at least 40, so the rule stands in
/// column 30 to `width - 41`, however far the pointer goes. 160 columns start
/// the rule in column 44, a narrower pane in column 34.
#[test]
fn a_drag_of_the_sidebar_rule_under_a_full_panel_stops_at_its_bounds() {
    for (width, start, by, want) in [
        (160, 44, 115, 119),
        (160, 44, -44, 30),
        (100, 34, 60, 59),
        (100, 34, -34, 30),
    ] {
        let mut app = app(None);
        two(&mut app);
        opened(&mut app, true);
        let _ = frame(width, HEIGHT, &mut app);
        let left = drag_rule(&mut app, width, start, by);
        let at = format!("width {width}, by {by}");
        assert_eq!(left.asked, Some(want), "asked, {at}");
        assert_eq!(left.rule, want, "drawn, {at}");
        assert_eq!(left.edge, Some(want + 1), "the panel's edge, {at}");
    }
}

/// A drag grabbed under a full panel waits out a frame that draws no sidebar
/// and follows the pointer again once the rule is back: the motion under the
/// small frame moves nothing and the drag stays held.
#[test]
fn a_drag_under_a_full_panel_waits_out_a_frame_without_a_sidebar() {
    let rule = rule_without_panel();
    let mut app = app(None);
    two(&mut app);
    opened(&mut app, true);
    let _ = frame(WIDTH, HEIGHT, &mut app);
    let mouse = |kind, column: u16| Mouse {
        kind,
        column,
        row: 20,
    };
    let _ = app.mouse(mouse(MouseKind::Click, rule));
    let _ = frame(80, 19, &mut app);
    assert!(!app.layout.shows(Edge::Sidebar), "no sidebar at 80x19");
    assert!(
        !app.mouse(mouse(MouseKind::Drag, 50)),
        "the motion has no rule to move"
    );
    assert_eq!(app.model.split().sidebar, None, "the width stays unasked");
    assert!(app.model.drag().is_some(), "the drag is still held");
    let _ = frame(WIDTH, HEIGHT, &mut app);
    let _ = app.mouse(mouse(MouseKind::Drag, rule + 3));
    assert_eq!(app.model.split().sidebar, Some(rule + 3), "followed again");
}

/// Beside a side panel the panel keeps the motion that lands on it, as it
/// always did: a sidebar or list drag that crosses to the panel's cells asks
/// for no size and the side panel stays drawn, while the release over it still
/// ends the drag.
#[test]
fn a_drag_that_crosses_a_side_panel_is_still_the_panels() {
    let rule = rule_without_panel();
    let over = 125;
    let mouse = |kind, column: u16, row: u16| Mouse { kind, column, row };
    let mut app = app(None);
    two(&mut app);
    opened(&mut app, false);
    let _ = frame(WIDTH, HEIGHT, &mut app);
    let border = app.layout.panel.map(|cells| cells.x).expect("a side panel");
    assert!(
        rule < border && border < over,
        "the border {border} is crossed"
    );
    let _ = app.mouse(mouse(MouseKind::Click, rule, 20));
    let _ = app.mouse(mouse(MouseKind::Drag, over, 20));
    let _ = app.mouse(mouse(MouseKind::Release, over, 20));
    let _ = frame(WIDTH, HEIGHT, &mut app);
    assert_eq!(app.model.split().sidebar, None, "no sidebar width asked");
    assert!(app.model.drag().is_none(), "the release ends the drag");
    assert_eq!(app.layout.panel.map(|cells| cells.x), Some(border), "side");
    let first = (0..HEIGHT)
        .find(|y| frame(WIDTH, HEIGHT, &mut app)[(2, *y)].symbol() == "─")
        .expect("the list rule");
    let _ = app.mouse(mouse(MouseKind::Click, 2, first));
    let _ = app.mouse(mouse(MouseKind::Drag, over, first + 3));
    let _ = app.mouse(mouse(MouseKind::Release, over, first + 3));
    assert_eq!(app.model.split().list, None, "no list height asked");
    assert!(app.model.drag().is_none(), "the release ends the drag");
}

/// A drag of the session list's rule follows the pointer's row while the
/// pointer is over a full panel: the panel covers the main column beside the
/// sidebar, and the motion there still belongs to the grabbed rule.
#[test]
fn a_drag_of_the_list_rule_follows_the_pointer_over_a_full_panel() {
    let mut app = app(None);
    two(&mut app);
    opened(&mut app, true);
    let list_rule = |buf: &Buffer| (0..HEIGHT).find(|y| buf[(2, *y)].symbol() == "─");
    let first = list_rule(&frame(WIDTH, HEIGHT, &mut app)).expect("the list rule");
    let over = rule_without_panel() + 6;
    let mouse = |kind, column, row| Mouse { kind, column, row };
    let _ = app.mouse(mouse(MouseKind::Click, 2, first));
    assert_eq!(app.model.drag().map(|drag| drag.edge), Some(Edge::List));
    let _ = app.mouse(mouse(MouseKind::Drag, over, first + 3));
    let _ = app.mouse(mouse(MouseKind::Release, over, first + 3));
    assert!(app.model.drag().is_none(), "the release ends the drag");
    assert_eq!(
        list_rule(&frame(WIDTH, HEIGHT, &mut app)),
        Some(first + 3),
        "the rule followed the pointer's row over the panel"
    );
}

/// How a route to a session under a full panel ended.
struct Routed {
    selected: Option<String>,
    full: bool,
    keys_on_panel: bool,
    redraw: bool,
}

/// Run `act` under a full panel over the sessions `web` and `ops` with
/// `start` selected, or over no session at all with `None`: the selection, the
/// panel's mode and focus after it, and whether the route owed a redraw. `web`
/// is the model's own first selection and `ops` is selected by place, so no
/// wheeled list window is pinned: a pin would redraw a key that names no row.
fn routed(start: Option<&str>, act: impl Fn(&mut App) -> bool) -> Routed {
    let mut app = app(None);
    if let Some(name) = start {
        two(&mut app);
        if name == "ops" {
            let _ = app.model.select_at(&app.fleet, 1);
        }
        assert_eq!(app.model.selected(), Some(name));
    }
    opened(&mut app, true);
    let _ = frame(WIDTH, HEIGHT, &mut app);
    assert_eq!(app.model.list_top(), None, "no pinned list window");
    let redraw = act(&mut app);
    let _ = frame(WIDTH, HEIGHT, &mut app);
    Routed {
        selected: app.model.selected().map(str::to_owned),
        full: app.orch.panel.as_ref().is_some_and(|panel| panel.full),
        keys_on_panel: app.panel_focused(),
        redraw,
    }
}

/// A press on the sidebar row `name` as the frame drew it; with `leave` the
/// session has left the fleet since, so the press is a historical one.
fn press_row(name: &'static str, leave: bool) -> impl Fn(&mut App) -> bool {
    let rule = rule_without_panel();
    move |app: &mut App| {
        let buf = frame(WIDTH, HEIGHT, app);
        let (x, y) = (0..HEIGHT)
            .find_map(|y| {
                cell_of(&buf, name, y, 0)
                    .filter(|x| *x < rule)
                    .map(|x| (x, y))
            })
            .expect("the row in the sidebar");
        if leave {
            app.fleet.rows.retain(|row| row.name != name);
        }
        app.mouse(Mouse {
            kind: MouseKind::Click,
            column: x,
            row: y,
        })
    }
}

/// The keys `key` as the loop takes them: whether the route owed a redraw.
fn typed(key: Key) -> impl Fn(&mut App) -> bool {
    move |app: &mut App| take_keys(app, vec![(key.clone(), Instant::now())]).unwrap_or(false)
}

/// One route to a session under a full panel and what it must leave: the
/// start, the selection after it, whether the panel turned side and whether
/// the route owed a redraw, `None` where it does not matter.
struct Case<'a> {
    route: &'a str,
    start: Option<&'a str>,
    act: Box<dyn Fn(&mut App) -> bool>,
    selected: Option<&'a str>,
    side: bool,
    redraw: Option<bool>,
}

fn case<'a>(
    route: &'a str,
    start: Option<&'a str>,
    act: impl Fn(&mut App) -> bool + 'static,
    selected: Option<&'a str>,
    side: bool,
    redraw: Option<bool>,
) -> Case<'a> {
    Case {
        route,
        start,
        act: Box::new(act),
        selected,
        side,
        redraw,
    }
}

/// Every way to name a session under a full panel shows it: a press on its
/// row, also the one already selected, and `j`, `k`, the arrows and `1`-`9`
/// that name a row select it, turn the panel side, give the main column the
/// keys and owe a redraw. A key that names no row (a digit past the list, a
/// step off either end), a press on a session that left the fleet, the
/// panel's own keys and a list key with no session leave the panel full.
#[test]
fn every_route_to_a_session_under_a_full_panel_selects_it_and_turns_the_panel_side() {
    let text = |byte: &[u8]| typed(Key::Text(byte.to_vec()));
    let (web, ops) = (Some("web"), Some("ops"));
    let (yes, no) = (Some(true), Some(false));
    let cases = [
        case("press ops", web, press_row("ops", false), ops, true, yes),
        case(
            "press web, the selected",
            web,
            press_row("web", false),
            web,
            true,
            yes,
        ),
        case("j", web, text(b"j"), ops, true, yes),
        case("Down", web, typed(Key::Down), ops, true, yes),
        case("2", web, text(b"2"), ops, true, yes),
        case("k", ops, text(b"k"), web, true, yes),
        case("Up", ops, typed(Key::Up), web, true, yes),
        case("1 on web, the selected", web, text(b"1"), web, true, yes),
        case("2 on ops, the selected", ops, text(b"2"), ops, true, yes),
        case("k at the first row", web, text(b"k"), web, false, no),
        case("Up at the first row", web, typed(Key::Up), web, false, no),
        case("j at the last row", ops, text(b"j"), ops, false, no),
        case(
            "Down at the last row",
            ops,
            typed(Key::Down),
            ops,
            false,
            no,
        ),
        case("9 past the list", web, text(b"9"), web, false, no),
        case(
            "press on a departed row",
            web,
            press_row("ops", true),
            web,
            false,
            no,
        ),
        case("j with no session", None, text(b"j"), None, false, no),
        case("Enter", web, typed(Key::Enter), web, false, None),
        case("Tab", web, typed(Key::Tab), web, false, None),
        case("n", web, text(b"n"), web, false, None),
    ];
    let mut wrong = Vec::new();
    for case in &cases {
        let left = routed(case.start, &case.act);
        let ok = left.selected.as_deref() == case.selected
            && left.full != case.side
            && left.keys_on_panel != case.side
            && case.redraw.is_none_or(|owed| left.redraw == owed);
        if !ok {
            wrong.push(format!(
                "{}: selected {:?}, full {}, panel keys {}, redraw {}",
                case.route, left.selected, left.full, left.keys_on_panel, left.redraw
            ));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}
