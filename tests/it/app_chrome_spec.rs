// Frozen R1-R4/R3b: literal budgets, visible cells and reducer clicks.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::{App, app as empty_app, draw, one_row, row as fleet_row};
use crate::app::model;
use crate::console::input::{Mouse, MouseKind};
use crate::digest::{SessionEntry, Status};
use crate::theme::Look;

fn app() -> App {
    let mut app = empty_app(Some("api"));
    app.read_only.clear();
    app.fleet = one_row(Some("api"));
    app.model = model::Model::new(&app.fleet);
    app.world
        .sessions
        .push(SessionEntry::new("api", Status::Running));
    let look = Look::read("off", "", "off", "off");
    app.looks.insert("api".into(), (Some(look), None));
    app
}

fn frame(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
    app.frame(&mut buf);
    buf
}

fn row(buf: &Buffer, y: u16, left: u16) -> String {
    (left..buf.area.width)
        .map(|x| buf[(x, y)].symbol())
        .collect()
}

fn click(x: u16, y: u16) -> Mouse {
    Mouse {
        kind: MouseKind::Click,
        column: x,
        row: y,
    }
}

#[test]
fn needy_arrows_survive_without_the_lone_key_marker() {
    let mut app = app();
    app.fleet.rows = (0..30)
        .map(|at| {
            let mut row = fleet_row(&format!("s{at}"), at + 1, false);
            row.needy = at == 29;
            row
        })
        .collect();
    app.fleet.home = Some("s0".into());
    app.model = model::Model::new(&app.fleet);
    let buf = frame(&mut app, 160, 43);
    let attention: String = (0..44).map(|x| buf[(x, 0)].symbol()).collect();
    assert!(attention.contains('↓'), "hidden needy row keeps its arrow");
    assert!(
        attention.trim_end().ends_with('↓'),
        "lone key marker removed: nothing follows the arrow"
    );
}

#[test]
fn browse_hint_version_and_gear_share_the_composer_hint_row() {
    let mut app = app();
    let buf = frame(&mut app, 160, 43);
    assert!(row(&buf, 42, 47).starts_with("Enter writes · ? keys"));
    assert!(row(&buf, 42, 47).ends_with(&format!("{} *", crate::version_line())));
    assert_eq!(buf[(44, 42)].symbol(), "│", "sidebar keeps the freed row");
}

#[test]
fn fixed_size_browse_lane_gains_three_rows() {
    for (width, height, rows) in [(100, 28, 21), (160, 43, 36)] {
        let mut app = app();
        frame(&mut app, width, height);
        assert_eq!(app.layout.page_rows, rows, "old budgets 18/33 plus three");
    }
}

#[test]
fn gear_and_its_leading_blank_open_settings_from_the_hint_row() {
    for x in [158, 159] {
        let mut app = app();
        frame(&mut app, 160, 43);
        assert!(matches!(
            app.layout.hit(click(x, 42)),
            Some(draw::Hit::Settings)
        ));
        assert!(app.mouse(click(x, 42)));
        assert!(app.model.settings_open());
        let buf = frame(&mut app, 160, 43);
        assert!(row(&buf, 42, 0).starts_with("  settings   Esc close"));
    }
}

#[test]
fn peek_footer_keeps_copy_note_and_version_without_gear_or_target() {
    let mut app = app();
    app.model.open_peek(0);
    app.flash = Some(("copied 2 lines".into(), std::time::Instant::now()));
    let buf = frame(&mut app, 160, 43);
    assert!(row(&buf, 42, 0).starts_with("  viewing   copied 2 lines"));
    assert!(row(&buf, 42, 0).ends_with(&crate::version_line()));
    assert!(!row(&buf, 42, 0).contains('│'), "footer clears the sidebar");
    assert!(app.layout.hit(click(159, 42)).is_none());
}

#[test]
fn narrowing_drops_version_then_gear_without_dead_settings_targets() {
    let tail = format!("{} *", crate::version_line());
    let full = 113 - 2 - tail.chars().count();
    for (length, wanted, gear) in [
        (full, tail.as_str(), true),
        (full + 1, "*", true),
        (111, "", false),
    ] {
        let mut app = app();
        app.held = Some("fixture hold".into());
        app.flash = Some(("n".repeat(length), std::time::Instant::now()));
        let buf = frame(&mut app, 160, 43);
        assert_eq!(row(&buf, 42, 47).trim_start_matches('n').trim(), wanted);
        assert_eq!(
            matches!(app.layout.hit(click(159, 42)), Some(draw::Hit::Settings)),
            gear
        );
    }
}

#[test]
fn sidebar_fallback_keeps_header_and_warning() {
    let mut app = app();
    let buf = frame(&mut app, 89, 19);
    assert!(row(&buf, 0, 2).starts_with("api"));
    assert!(row(&buf, 1, 0).contains("sidebar needs 90x20"));
}
