//! Frozen acceptance for boardgrip: the divider of the board and of the side
//! panel is easy to take. A press on the rule or on the plain cell beside it
//! drags it, `<` and `>` step the right-hand column four cells, and the rule
//! draws a grip on its middle rows.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "frozen acceptance owns scratch state roots and terminal buffers"
)]

use ratatui_core::style::{Color, Style};

use super::boardfull_spec::{
    bytes, locate, paint, rules_at_bottom, sample_desk, send_key, setup_board,
};
use super::*;
use crate::app::draw::Hit;
use crate::console::input::Keys;

const WIDE: u16 = 200;
const HEIGHT: u16 = 35;
/// A frame width whose chat column is 80 cells (115 less the sidebar's 35): the side panel's floor is also its ceiling.
const PINNED: u16 = 115;
const BOARD: &[u8] = b"DECIDE\nD1 | Check release choice\nCOMING\nC1 | Review tomorrow\n";
/// The grip's rows: five, centred on the pane's middle row.
const GRIP: std::ops::RangeInclusive<u16> = (HEIGHT / 2 - 2)..=(HEIGHT / 2 + 2);

fn fixture(tag: &str) -> (Root, App) {
    let (root, mut app) = setup_board(tag, Some(sample_desk(BOARD)));
    let _ = paint(&mut app, WIDE, HEIGHT);
    (root, app)
}

/// Where the sidebar, panel and board rules stand on the pane's last row.
fn rules(app: &mut App) -> Vec<u16> {
    rules_at_bottom(&paint(app, WIDE, HEIGHT))
}

/// The board's width, its border included: the pane's end less its rule.
fn board_width(app: &mut App) -> u16 {
    WIDE - rules(app).last().copied().expect("the board's rule")
}

/// The keys as the loop takes them from the human: the view's change is judged around them.
fn human(app: &mut App, raw: &[u8]) {
    let at = Instant::now();
    assert!(app.human(Keys::app().feed(raw, at)).is_some());
}

fn event(app: &mut App, kind: MouseKind, (column, row): (u16, u16)) {
    let at = Instant::now();
    assert!(take_keys(app, vec![(Key::Mouse(Mouse { kind, column, row }), at)]).is_some());
}

/// Press at `from`, move to column `to` on the same row, let go.
fn drag_from(app: &mut App, from: (u16, u16), to: u16) {
    event(app, MouseKind::Click, from);
    event(app, MouseKind::Drag, (to, from.1));
    event(app, MouseKind::Release, (to, from.1));
}

/// A press on the rule, or on the cell either side of it, on every kind of
/// row, takes the rule: it follows the pointer by the distance it moved.
#[test]
fn a_press_on_a_rule_or_the_cell_beside_it_drags_the_panel_and_the_board_borders() {
    let (_root, mut app) = fixture("boardgrip-zone");
    assert_eq!(rules(&mut app), [44, 105, 164]);
    let mut flip = 1_i32;
    for index in [2_usize, 1] {
        for beside in [-1_i32, 0, 1] {
            for row in [0_u16, 1, 5, HEIGHT / 2, HEIGHT - 3, HEIGHT - 2] {
                let before = rules(&mut app);
                let rule = i32::from(before[index]);
                let press = u16::try_from(rule + beside).expect("a column");
                // Left widens the column, right narrows it; alternate so the
                // clamps never meet.
                let to = u16::try_from(i32::from(press) - 3 * flip).expect("a column");
                drag_from(&mut app, (press, row), to);
                let after = rules(&mut app);
                assert_eq!(
                    i32::from(after[index]),
                    rule - 3 * flip,
                    "press {beside:+} of rule {index} on row {row}: {before:?} -> {after:?}"
                );
                flip = -flip;
            }
        }
    }
}

/// The zone ends where a click target starts: the board's `COMING` tab label
/// two cells right of its rule takes the click and moves no rule, while the
/// plain cell right beside the rule on the same row still takes the rule.
#[test]
fn a_click_target_beside_the_zone_keeps_its_click() {
    let (_root, mut app) = fixture("boardgrip-target");
    let cells = paint(&mut app, WIDE, HEIGHT);
    let rule = rules_at_bottom(&cells)[2];
    // `COMING` is the second label of the tab bar.
    let (x, y) = locate(&cells, "COM").expect("the COMING tab label");
    assert!(
        x > rule + 1,
        "the label starts beyond the zone: {x} vs {rule}"
    );
    assert_ne!(app.model.desk_tab(), crate::desk::Section::Coming);
    let before = rules(&mut app);
    drag_from(&mut app, (x, y), x - 6);
    assert_eq!(
        app.model.desk_tab(),
        crate::desk::Section::Coming,
        "the click"
    );
    assert_eq!(rules(&mut app), before, "a label press drags nothing");
    drag_from(&mut app, (rule + 1, y), rule - 2);
    assert_eq!(
        rules(&mut app)[2],
        rule - 3,
        "the plain cell beside the rule, on the label's row, takes the rule"
    );
}

/// A click target inside the zone keeps its click: a session row of the sidebar
/// reaches the cell left of the sidebar's rule, and a press there selects it.
#[test]
fn a_click_target_inside_the_zone_keeps_its_click_and_takes_no_rule() {
    let (_root, mut app) = fixture("boardgrip-inside");
    let cells = paint(&mut app, WIDE, HEIGHT);
    let rule = rules_at_bottom(&cells)[0];
    let row = (0..HEIGHT)
        .find(|row| {
            let at = Mouse {
                kind: MouseKind::Click,
                column: rule - 1,
                row: *row,
            };
            matches!(app.layout.hit(at), Some(Hit::Session(_)))
        })
        .expect("a session row reaches the cell beside the sidebar's rule");
    drag_from(&mut app, (rule - 1, row), rule - 12);
    assert_eq!(
        rules(&mut app)[0],
        rule,
        "the press selected, it dragged nothing"
    );
}

/// In F full the main column lies hidden under the panel and the board: its
/// composer and turn rows are no targets there, so a press beside the board's
/// rule takes it on every row.
#[test]
fn a_press_beside_the_board_rule_takes_it_in_full_panel_mode_on_every_row() {
    let (_root, mut app) = fixture("boardgrip-full");
    bytes(&mut app, b"F");
    assert_eq!(rules(&mut app).len(), 1, "the board's rule alone stands");
    let mut flip = 1_i32;
    for beside in [-1_i32, 0, 1] {
        for row in 0..HEIGHT {
            let before = rules(&mut app);
            let rule = i32::from(before[0]);
            let press = u16::try_from(rule + beside).expect("a column");
            let to = u16::try_from(i32::from(press) - 3 * flip).expect("a column");
            drag_from(&mut app, (press, row), to);
            let after = rules(&mut app);
            assert_eq!(
                i32::from(after[0]),
                rule - 3 * flip,
                "press {beside:+} of the rule on row {row}: {before:?} -> {after:?}"
            );
            flip = -flip;
        }
    }
}

/// `>` and `<` in F full: the board keeps its floor of 24 and leaves the
/// panel 40 cells, the sidebar being hidden.
#[test]
fn the_angle_keys_step_the_board_in_full_panel_mode() {
    let (_root, mut app) = fixture("boardgrip-full-keys");
    bytes(&mut app, b"F");
    assert_eq!(board_width(&mut app), 36);
    bytes(&mut app, b">");
    assert_eq!(board_width(&mut app), 40);
    for _ in 0..40 {
        bytes(&mut app, b">");
    }
    assert_eq!(board_width(&mut app), WIDE - 40, "the panel keeps 40");
    for _ in 0..40 {
        bytes(&mut app, b"<");
    }
    assert_eq!(board_width(&mut app), 24, "the board's floor");
}

/// With the main column zoomed no column stands on its right: the keys step
/// nothing and leave the view as it was.
#[test]
fn the_angle_keys_do_nothing_while_the_main_column_is_zoomed() {
    let (_root, mut app) = fixture("boardgrip-zoomed");
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
    let (_root, mut app) = fixture("boardgrip-floor");
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
    let (_root, mut app) = fixture("boardgrip-pinned");
    bytes(&mut app, b"b");
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
    let (_root, mut app) = fixture("boardgrip-report");
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
    let (_root, mut app) = fixture("boardgrip-settings");
    bytes(&mut app, b",");
    assert!(app.model.settings_open());
    bytes(&mut app, b"<>>");
    send_key(&mut app, Key::Escape);
    assert!(!app.model.settings_open());
    assert_eq!(board_width(&mut app), 36, "no step under Settings");
}

/// `>` and `<` step the board by four cells, as many as typed in one read,
/// and stop at the board's floor of 24 and at the chat columns' floors.
#[test]
fn the_angle_keys_step_the_board_by_four_cells_within_its_clamps() {
    for focus in ["panel", "board", "main"] {
        let (_root, mut app) = fixture("boardgrip-keys");
        match focus {
            "board" => bytes(&mut app, b"B"),
            "main" => {
                app.focus_column(false);
                assert!(!app.panel_focused() && !app.desk_focused());
            }
            _ => {}
        }
        assert_eq!(board_width(&mut app), 36, "the default, {focus}");
        bytes(&mut app, b">");
        assert_eq!(board_width(&mut app), 40, "> grows by four, {focus}");
        bytes(&mut app, b"<<");
        assert_eq!(board_width(&mut app), 32, "two typed together, {focus}");
        for _ in 0..9 {
            bytes(&mut app, b"<");
        }
        assert_eq!(board_width(&mut app), 24, "the board's floor, {focus}");
        bytes(&mut app, b"<");
        assert_eq!(board_width(&mut app), 24, "no step below it, {focus}");
        // Both chat columns keep 40 beside the sidebar's 45: 200 - 125.
        for _ in 0..30 {
            bytes(&mut app, b">");
        }
        assert_eq!(
            board_width(&mut app),
            75,
            "the chat columns' floors, {focus}"
        );
        assert_eq!(app.desk_focused(), focus == "board", "the keys stay put");
    }
}

/// A step is a change the human made: the view is due a write and a restart
/// carries the width, like a dragged one.
#[test]
fn a_stepped_board_is_saved_with_the_view_like_a_dragged_one() {
    let (_root, mut app) = fixture("boardgrip-saved");
    assert!(!app.view_dirty);
    human(&mut app, b">");
    assert!(app.view_dirty, "the step is the human's change");
    let held = app
        .handover("%1", crate::VERSION, 100)
        .expect("a handover")
        .render();
    assert!(held.ends_with("|desk=40"), "{held}");
    human(&mut app, b"<<");
    let held = app
        .handover("%1", crate::VERSION, 100)
        .expect("a handover")
        .render();
    assert!(held.ends_with("|desk=32"), "{held}");
    let parsed = crate::app::handover::parse(&held).expect("the wire parses");
    assert_eq!(parsed.render(), held, "and renders back the same");
}

/// While writing the keys are text: nothing steps and the draft takes them.
#[test]
fn the_angle_keys_type_while_writing() {
    for focus in ["panel", "main"] {
        let (_root, mut app) = fixture("boardgrip-writing");
        if focus == "main" {
            app.focus_column(false);
        }
        bytes(&mut app, b"i");
        assert!(app.composing(), "{focus}");
        bytes(&mut app, b"<>");
        assert_eq!(
            app.writer_input().map(Input::draft).as_deref(),
            Some("<>"),
            "the draft holds both keys, {focus}"
        );
        assert_eq!(board_width(&mut app), 36, "no step while writing, {focus}");
        send_key(&mut app, Key::Escape);
        bytes(&mut app, b">");
        assert_eq!(board_width(&mut app), 40, "browsing again steps, {focus}");
    }
}

/// With the board hidden, the side panel is the right-hand column and the
/// keys step it, to its floor of 40 and the main column's.
#[test]
fn the_angle_keys_step_the_side_panel_when_no_board_stands() {
    let (_root, mut app) = fixture("boardgrip-panel");
    bytes(&mut app, b"b");
    let width = |app: &mut App| WIDE - rules(app).last().copied().expect("the panel's rule");
    assert_eq!(
        rules(&mut app).len(),
        2,
        "the sidebar's rule and the panel's"
    );
    let start = width(&mut app);
    bytes(&mut app, b">");
    assert_eq!(width(&mut app), start + 4);
    bytes(&mut app, b"<<");
    assert_eq!(width(&mut app), start - 4);
    for _ in 0..40 {
        bytes(&mut app, b"<");
    }
    assert_eq!(width(&mut app), 40, "the panel's floor");
    for _ in 0..40 {
        bytes(&mut app, b">");
    }
    // 200 - 45 = 155 cells for both columns, 40 kept for the main one.
    assert_eq!(width(&mut app), 115, "the main column's floor");
}

/// With no column on the right the keys are not bound to anything.
#[test]
fn the_angle_keys_do_nothing_with_no_column_on_the_right() {
    let (_root, mut app) = fixture("boardgrip-none");
    bytes(&mut app, b"O");
    assert_eq!(rules(&mut app).len(), 1, "only the sidebar's rule stands");
    let split = app.model.split();
    human(&mut app, b"<>><");
    assert_eq!(app.model.split(), split, "no size moved");
    assert!(!app.view_dirty, "and the view is not due a write");
}

/// The rule of the panel and of the board draws a heavy grip on its five
/// middle rows, in the rule's own colour; every other row keeps the thin line.
#[test]
fn the_panel_and_board_rules_draw_a_grip_on_their_middle_rows_in_the_rule_colour() {
    let (_root, mut app) = fixture("boardgrip-grip");
    let look = crate::theme::Look::DEFAULT;
    app.looks.insert("api".to_owned(), (Some(look), None));
    let cells = paint(&mut app, WIDE, HEIGHT);
    let rules = rules_at_bottom(&cells);
    assert_eq!(rules, [44, 105, 164]);
    for column in [105_u16, 164] {
        let thin = cells[(column, 3)].style();
        assert_ne!(thin, Style::default(), "the rule has its colour");
        for row in 1..HEIGHT - 1 {
            let cell = &cells[(column, row)];
            let want = if GRIP.contains(&row) { "┃" } else { "│" };
            assert_eq!(cell.symbol(), want, "column {column}, row {row}");
            assert_eq!(cell.style(), thin, "column {column}, row {row}: its colour");
        }
    }
    let sidebar = rules[0];
    assert!(
        (1..HEIGHT - 1).all(|row| cells[(sidebar, row)].symbol() != "┃"),
        "the sidebar's rule keeps its plain line"
    );
}

/// `theme = off` paints no colour, and the grip is still there as a glyph.
#[test]
fn the_grip_is_a_plain_glyph_with_the_theme_off() {
    let (_root, mut app) = fixture("boardgrip-off");
    assert!(app.dressed().0.is_none(), "the fixture paints no colour");
    let cells = paint(&mut app, WIDE, HEIGHT);
    for column in [105_u16, 164] {
        assert_eq!(cells[(column, HEIGHT / 2)].symbol(), "┃", "column {column}");
        assert_eq!(cells[(column, 3)].symbol(), "│", "column {column}");
        let grip = cells[(column, HEIGHT / 2)].style();
        assert_eq!(
            grip,
            cells[(column, 3)].style(),
            "column {column}: the line's own style"
        );
        assert_eq!(grip.fg, Some(Color::Reset), "column {column}: no colour");
    }
}

/// A rule held by a drag takes the drag's look over its whole length, grip
/// included.
#[test]
fn a_dragged_rule_keeps_its_grip_and_turns_whole() {
    let (_root, mut app) = fixture("boardgrip-held");
    let look = crate::theme::Look::DEFAULT;
    app.looks.insert("api".to_owned(), (Some(look), None));
    event(&mut app, MouseKind::Click, (164, 5));
    let cells = paint(&mut app, WIDE, HEIGHT);
    let held = cells[(164, 3)].style();
    for row in 1..HEIGHT - 1 {
        let want = if GRIP.contains(&row) { "┃" } else { "│" };
        assert_eq!(cells[(164, row)].symbol(), want, "row {row}");
        assert_eq!(cells[(164, row)].style(), held, "row {row}");
    }
    event(&mut app, MouseKind::Release, (164, 5));
    let cells = paint(&mut app, WIDE, HEIGHT);
    assert_ne!(
        cells[(164, 3)].style(),
        held,
        "let go: the rule's own colour"
    );
}
