//! The side panel's own divider after the board column is gone: `<` and `>`
//! step the right-hand column four cells within the frame's floors and do
//! nothing where no column stands on the right, and the rule draws a grip on
//! its middle rows. These are the panel legs of the retired board suites, the
//! mouse drag of the border being `tests/it/o8_panel_spec.rs`'s.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns scratch state roots and terminal buffers"
)]

use ratatui_core::style::{Color, Style};

use super::orchhead_spec::{HEIGHT, Mix, WIDE, bytes, paint, rig};
use super::*;
use crate::console::input::Keys;

/// A frame width whose chat column is 80 cells: the side panel's floor is
/// also its ceiling.
const PINNED: u16 = 115;
/// The grip's rows: five, centred on the pane's middle row.
const GRIP: std::ops::RangeInclusive<u16> = (HEIGHT / 2 - 2)..=(HEIGHT / 2 + 2);

fn fixture(tag: &str) -> (Root, App) {
    let (root, mut app) = rig(tag, Mix::Exact);
    let _ = paint(&mut app, WIDE, HEIGHT);
    (root, app)
}

/// Where the sidebar and panel rules stand on the pane's last row.
fn rules_at_bottom(cells: &Buffer) -> Vec<u16> {
    let y = cells.area.height.saturating_sub(1);
    (0..cells.area.width)
        .filter(|&x| cells[(x, y)].symbol() == "│")
        .collect()
}

fn rules(app: &mut App) -> Vec<u16> {
    rules_at_bottom(&paint(app, WIDE, HEIGHT))
}

/// The panel's width, its border included: the pane's end less its rule.
fn panel_width(app: &mut App) -> u16 {
    WIDE - rules(app).last().copied().expect("the panel's rule")
}

/// The keys as the loop takes them from the human: the view's change is judged around them.
fn human(app: &mut App, raw: &[u8]) {
    let at = Instant::now();
    assert!(app.human(Keys::app().feed(raw, at)).is_some());
}

fn send_key(app: &mut App, key: Key) {
    let at = Instant::now();
    assert!(take_keys(app, vec![(key, at)]).is_some());
}

/// `>` and `<` step the panel four cells, as many as typed in one read, and
/// stop at its floor of 40 and at the main column's.
#[test]
fn the_angle_keys_step_the_side_panel_by_four_cells_within_its_clamps() {
    for focus in ["panel", "main"] {
        let (_root, mut app) = fixture("panelgrip-steps");
        if focus == "main" {
            app.focus_column(false);
            assert!(!app.panel_focused());
        }
        assert_eq!(
            rules(&mut app).len(),
            2,
            "the sidebar's rule and the panel's, {focus}"
        );
        let start = panel_width(&mut app);
        bytes(&mut app, b">");
        assert_eq!(panel_width(&mut app), start + 4, "> grows by four, {focus}");
        bytes(&mut app, b"<<");
        assert_eq!(
            panel_width(&mut app),
            start - 4,
            "two typed together, {focus}"
        );
        for _ in 0..40 {
            bytes(&mut app, b"<");
        }
        assert_eq!(panel_width(&mut app), 40, "the panel's floor, {focus}");
        for _ in 0..40 {
            bytes(&mut app, b">");
        }
        // 200 - 45 = 155 cells for both columns, 40 kept for the main one.
        assert_eq!(
            panel_width(&mut app),
            115,
            "the main column's floor, {focus}"
        );
    }
}

/// While writing the keys are text: nothing steps and the draft takes them.
#[test]
fn the_angle_keys_type_while_writing() {
    for focus in ["panel", "main"] {
        let (_root, mut app) = fixture("panelgrip-writing");
        if focus == "main" {
            app.focus_column(false);
        }
        let start = panel_width(&mut app);
        bytes(&mut app, b"i");
        assert!(app.composing(), "{focus}");
        bytes(&mut app, b"<>");
        assert_eq!(
            app.writer_input()
                .map(crate::console::input::Input::draft)
                .as_deref(),
            Some("<>"),
            "the draft holds both keys, {focus}"
        );
        assert_eq!(
            panel_width(&mut app),
            start,
            "no step while writing, {focus}"
        );
        send_key(&mut app, Key::Escape);
        bytes(&mut app, b">");
        assert_eq!(
            panel_width(&mut app),
            start + 4,
            "browsing again steps, {focus}"
        );
    }
}

/// With no column on the right the keys are not bound to anything.
#[test]
fn the_angle_keys_do_nothing_with_no_column_on_the_right() {
    let (_root, mut app) = fixture("panelgrip-none");
    bytes(&mut app, b"O");
    assert_eq!(rules(&mut app).len(), 1, "only the sidebar's rule stands");
    let split = app.model.split();
    human(&mut app, b"<>><");
    assert_eq!(app.model.split(), split, "no size moved");
    assert!(!app.view_dirty, "and the view is not due a write");
}

/// With the main column zoomed no column stands on its right: the keys step
/// nothing and leave the view as it was.
#[test]
fn the_angle_keys_do_nothing_while_the_main_column_is_zoomed() {
    let (_root, mut app) = fixture("panelgrip-zoomed");
    app.focus_column(false);
    bytes(&mut app, b"X");
    let _ = paint(&mut app, WIDE, HEIGHT);
    app.view_dirty = false;
    let split = app.model.split();
    human(&mut app, b"<>");
    assert_eq!(app.model.split(), split, "no size moved");
    assert!(!app.view_dirty, "and the view is not due a write");
}

/// Below the size floor the frame draws a note and no column: the keys step
/// nothing there either, so the view keeps the sizes it had.
#[test]
fn the_angle_keys_do_nothing_below_the_size_floor() {
    let (_root, mut app) = fixture("panelgrip-floor");
    let _ = paint(&mut app, 120, 5);
    app.view_dirty = false;
    let split = app.model.split();
    human(&mut app, b"<>");
    assert_eq!(app.model.split(), split, "no size moved");
    assert!(!app.view_dirty, "and the view is not due a write");
}

/// A step the clamp cancels is no change at all: with the main column and the
/// side panel both at their floor, an 80-cell chat has one width for the
/// panel, and a key that asks another leaves the saved size unset, the view
/// clean and the loop told nothing moved.
#[test]
fn a_step_the_clamp_cancels_leaves_the_size_unset() {
    let (_root, mut app) = fixture("panelgrip-pinned");
    let cells = paint(&mut app, PINNED, HEIGHT);
    let panel = rules_at_bottom(&cells)
        .last()
        .copied()
        .expect("the panel's rule");
    assert_eq!(PINNED - panel, 40, "the panel keeps its floor, and no more");
    app.view_dirty = false;
    let split = app.model.split();
    let at = Instant::now();
    assert_eq!(app.human(Keys::app().feed(b">", at)), Some(false));
    assert_eq!(app.human(Keys::app().feed(b"<", at)), Some(false));
    assert_eq!(app.model.split(), split, "no size was kept");
    assert!(!app.view_dirty, "and the view is not due a write");
}

/// A key that moves a size tells the loop a change happened, so the frame is
/// painted again at once; one the clamp leaves as it was tells it nothing.
#[test]
fn a_key_that_moves_a_size_reports_the_change() {
    let (_root, mut app) = fixture("panelgrip-report");
    let at = Instant::now();
    assert_eq!(app.human(Keys::app().feed(b">", at)), Some(true));
    assert_eq!(app.human(Keys::app().feed(b"<", at)), Some(true));
    for _ in 0..40 {
        human(&mut app, b"<");
    }
    assert_eq!(app.human(Keys::app().feed(b"<", at)), Some(false));
}

/// With Settings open the keys are Settings': nothing steps.
#[test]
fn the_angle_keys_step_nothing_while_settings_is_open() {
    let (_root, mut app) = fixture("panelgrip-settings");
    let start = panel_width(&mut app);
    bytes(&mut app, b",");
    assert!(app.model.settings_open());
    bytes(&mut app, b"<>>");
    send_key(&mut app, Key::Escape);
    assert!(!app.model.settings_open());
    assert_eq!(panel_width(&mut app), start, "no step under Settings");
}

/// A step is a change the human made: the view is due a write and a restart
/// carries the width, like a dragged one.
#[test]
fn a_stepped_panel_is_saved_with_the_view_like_a_dragged_one() {
    let (_root, mut app) = fixture("panelgrip-saved");
    let start = panel_width(&mut app);
    assert!(!app.view_dirty);
    human(&mut app, b">");
    assert!(app.view_dirty, "the step is the human's change");
    let held = app.handover("%1", crate::VERSION, 100).expect("a handover");
    assert_eq!(held.view.panel, Some(start + 4));
    human(&mut app, b"<<");
    let held = app.handover("%1", crate::VERSION, 100).expect("a handover");
    assert_eq!(held.view.panel, Some(start - 4));
    let line = held.render();
    let parsed = crate::app::handover::parse(&line).expect("the wire parses");
    assert_eq!(parsed.render(), line, "and renders back the same");
}

/// The panel's rule draws a heavy grip on its five middle rows, in the rule's
/// own colour; every other row keeps the thin line, and the sidebar's rule
/// keeps its plain line.
#[test]
fn the_panel_rule_draws_a_grip_on_its_middle_rows_in_the_rule_colour() {
    let (_root, mut app) = fixture("panelgrip-grip");
    let look = crate::theme::Look::DEFAULT;
    app.looks.insert("api".to_owned(), (Some(look), None));
    let cells = paint(&mut app, WIDE, HEIGHT);
    let rules = rules_at_bottom(&cells);
    let [sidebar, column] = rules[..] else {
        panic!("the sidebar's rule and the panel's: {rules:?}");
    };
    let thin = cells[(column, 3)].style();
    assert_ne!(thin, Style::default(), "the rule has its colour");
    for row in 1..HEIGHT - 1 {
        let cell = &cells[(column, row)];
        let want = if GRIP.contains(&row) { "┃" } else { "│" };
        assert_eq!(cell.symbol(), want, "column {column}, row {row}");
        assert_eq!(cell.style(), thin, "column {column}, row {row}: its colour");
    }
    assert!(
        (1..HEIGHT - 1).all(|row| cells[(sidebar, row)].symbol() != "┃"),
        "the sidebar's rule keeps its plain line"
    );
}

/// `theme = off` paints no colour, and the grip is still there as a glyph.
#[test]
fn the_grip_is_a_plain_glyph_with_the_theme_off() {
    let (_root, mut app) = fixture("panelgrip-off");
    assert!(app.dressed().0.is_none(), "the fixture paints no colour");
    let cells = paint(&mut app, WIDE, HEIGHT);
    let column = *rules_at_bottom(&cells).last().expect("the panel's rule");
    assert_eq!(cells[(column, HEIGHT / 2)].symbol(), "┃");
    assert_eq!(cells[(column, 3)].symbol(), "│");
    let grip = cells[(column, HEIGHT / 2)].style();
    assert_eq!(grip, cells[(column, 3)].style(), "the line's own style");
    assert_eq!(grip.fg, Some(Color::Reset), "no colour");
}
