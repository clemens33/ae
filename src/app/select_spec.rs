//! Frozen private-controller acceptance. No transport mock; the live spec proves buffer effects.
#![allow(clippy::expect_used, reason = "pure bounded frame fixtures")]
use super::{App, Stage, fleet, loader::Answer, model, peek, take_keys};
use crate::console::{
    input::{Key, Mouse, MouseKind},
    lane::{Item, Kind, Lane},
};
use crate::digest::{SessionEntry, Status};
use crate::theme::Mark;
use ratatui_core::{buffer::Buffer, layout::Rect};
use std::rc::Rc;
use std::time::Instant;

fn item(body: &str) -> Item {
    Item {
        micros: 1,
        kind: Kind::Said {
            who: "lead".to_owned(),
        },
        body: body.to_owned(),
        record: None,
    }
}
fn app() -> App {
    let mut app = App::new(Some("one".to_owned()), None, None, None);
    app.fleet = fleet::Fleet {
        home: Some("one".to_owned()),
        rows: ["one", "two"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| fleet::Row {
                name: name.to_owned(),
                index: index + 1,
                mark: Mark::Working,
                needy: false,
                counts: fleet::Counts::Unknown,
                line2: fleet::Line2::NoGoal,
                home: index == 0,
            })
            .collect(),
    };
    app.model = model::Model::new(&app.fleet);
    app.world.sessions = vec![
        SessionEntry::new("one", Status::Running),
        SessionEntry::new("two", Status::Running),
    ];
    (app.fleeted, app.loading) = (true, false);
    app.lane = Rc::new(Lane {
        items: vec![item("alphaZ")],
        coverage: vec![],
    });
    app
}
fn frame(app: &mut App) -> Buffer {
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
    app.frame(&mut buf);
    buf
}
fn at(buf: &Buffer, needle: &str) -> (u16, u16) {
    (3..buf.area.height)
        .find_map(|y| {
            let row = (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>();
            row.find(needle).map(|byte| {
                (
                    u16::try_from(row[..byte].chars().count()).expect("column"),
                    y,
                )
            })
        })
        .expect("fixture drawn")
}
fn mouse(app: &mut App, kind: MouseKind, at: (u16, u16)) {
    let _ = app.mouse(Mouse {
        kind,
        column: at.0,
        row: at.1,
    });
}
fn drag(app: &mut App, word: &str) -> (u16, u16) {
    let before = frame(app);
    let start = at(&before, word);
    mouse(app, MouseKind::Click, start);
    let end = (start.0 + 2, start.1);
    mouse(app, MouseKind::Drag, end);
    let shown = frame(app);
    assert!(app.select.is_some(), "R1 controller holds a live drag");
    let changed: Vec<_> = (0..shown.area.width)
        .filter(|&x| before[(x, start.1)].modifier != shown[(x, start.1)].modifier)
        .collect();
    assert_eq!(
        changed,
        (start.0..=end.0).collect::<Vec<_>>(),
        "R1 exact overlay"
    );
    end
}
fn no_ship(app: &mut App) {
    assert!(!app.ship_staged(), "R5 no ship");
    assert!(
        !app.flash.as_ref().is_some_and(
            |(line, _)| line.starts_with("copied ") || line.starts_with("copy failed:")
        ),
        "no copy attempt/result"
    );
}

#[test]
fn list_and_body_ship_their_cells_and_list_clamps_before_lane() {
    for mode in 0..3 {
        let mut app = app();
        app.fleet.rows[0].home = false;
        app.fleet.rows[0].counts = fleet::Counts::Marks(vec![]);
        app.overview.goal = Some(("BODY_ALPHA".to_owned(), None));
        let mut end = drag(&mut app, if mode == 1 { "BODY_ALPHA" } else { "one" });
        if mode == 2 {
            end.0 = 150;
            mouse(&mut app, MouseKind::Drag, end);
            let _ = frame(&mut app);
        }
        mouse(&mut app, MouseKind::Release, end);
        let _ = frame(&mut app);
        let held = app.select.as_ref().expect("R3 live panel");
        let panel = if mode == 1 {
            super::select::Panel::Body
        } else {
            super::select::Panel::List
        };
        assert_eq!(held.sel.panel(), panel, "R3 origin panel");
        let wanted = if mode == 1 { "BOD" } else { "one" };
        assert!(
            matches!(&held.stage, Stage::Ready(text) if text == wanted),
            "R3 panel bytes"
        );
    }
}

#[test]
fn release_stages_exact_final_cells_only_after_a_frame() {
    let mut app = app();
    let end = drag(&mut app, "alphaZ");
    mouse(&mut app, MouseKind::Release, (end.0 + 2, end.1));
    no_ship(&mut app);
    let _ = frame(&mut app);
    let held = app.select.as_ref().expect("valid highlighted selection");
    assert!(
        matches!(&held.stage,Stage::Ready(text) if text=="alpha"),
        "R2 final frame defines exactly alpha"
    );
    assert!(app.ship_staged(), "one attempt after the frame");
    assert!(!app.ship_staged(), "R2 a release ships once");
}

#[test]
fn lane_growth_and_resize_clear_mid_drag_and_ship_nothing() {
    for resize in [false, true] {
        let mut app = app();
        let end = drag(&mut app, "alphaZ");
        if resize {
            app.frame(&mut Buffer::empty(Rect::new(0, 0, 160, 44)));
        } else {
            app.lane = Rc::new(Lane {
                items: vec![item("alphaZ"), item("replacement")],
                coverage: vec![],
            });
            let _ = frame(&mut app);
        }
        assert!(app.select.is_none(), "R5 lane/geometry changed");
        mouse(&mut app, MouseKind::Release, end);
        let _ = frame(&mut app);
        no_ship(&mut app);
    }
}

#[test]
fn session_tab_and_modal_round_trips_cancel_even_before_a_frame() {
    for change in 0..4 {
        let mut app = app();
        let end = drag(&mut app, "alphaZ");
        mouse(&mut app, MouseKind::Release, end);
        match change {
            0 => {
                let _ = app.model.key(model::Key::Digit(2), &app.fleet, false, true);
                let _ = app.model.key(model::Key::Digit(1), &app.fleet, false, true);
            }
            1 => {
                let _ = app.model.show_tab(model::Tab::Agents);
                let _ = app.model.show_tab(model::Tab::Overview);
            }
            2 => {
                app.model.open_settings(17);
                app.model.close_settings();
            }
            _ => {
                app.model.open_peek(17);
                app.model.close_peek();
            }
        }
        let _ = frame(&mut app);
        assert!(app.select.is_none(), "R6 transition {change}");
        no_ship(&mut app);
    }
}

#[test]
fn release_then_escape_or_new_press_in_one_batch_never_ships() {
    for escape in [false, true] {
        let mut app = app();
        let end = drag(&mut app, "alphaZ");
        mouse(&mut app, MouseKind::Release, end);
        if escape {
            let _ = take_keys(&mut app, vec![(Key::Escape, Instant::now())]);
        } else {
            mouse(&mut app, MouseKind::Click, (47, 1));
        }
        let _ = frame(&mut app);
        assert!(app.select.is_none(), "R6 later event");
        no_ship(&mut app);
    }
}

#[test]
fn stale_and_lost_frame_presses_clear_without_rearming() {
    for lost in [false, true] {
        let old = Instant::now();
        let mut app = app();
        let end = drag(&mut app, "alphaZ");
        if lost {
            app.lost = Some(Instant::now());
        }
        let _ = app.mouse_made(
            Mouse {
                kind: MouseKind::Click,
                column: end.0,
                row: end.1,
            },
            old,
        );
        assert!(app.select.is_none(), "R6 stale/lost press");
        no_ship(&mut app);
    }
}

fn peeking() -> App {
    let mut app = app();
    app.model.open_peek(17);
    app.peek_sync();
    app.peek.shown = Some(peek::Body::Lines(vec!["alphaZ".to_owned()]));
    app
}
#[test]
fn selection_escape_is_consumed_and_a_second_escape_closes_peek() {
    let mut app = peeking();
    let _ = drag(&mut app, "alphaZ");
    let _ = take_keys(&mut app, vec![(Key::Escape, Instant::now())]);
    assert!(app.select.is_none());
    assert!(app.model.peek_open(), "R6 first Esc only clears selection");
    let _ = take_keys(&mut app, vec![(Key::Escape, Instant::now())]);
    assert!(!app.model.peek_open(), "R6 second Esc closes");
}

#[test]
fn new_peek_gap_cancels_but_success_waits_only_until_ship() {
    for gap in [false, true] {
        let mut app = peeking();
        let end = drag(&mut app, "alphaZ");
        let body = if gap {
            peek::Body::Gap("capture refused".to_owned())
        } else {
            peek::Body::Lines(vec!["newest".to_owned()])
        };
        app.answer(Answer::Peek {
            generation: 17,
            at: Instant::now(),
            body,
        });
        let _ = frame(&mut app);
        if gap {
            assert!(app.select.is_none());
            mouse(&mut app, MouseKind::Release, end);
            let _ = frame(&mut app);
            no_ship(&mut app);
        } else {
            assert!(
                matches!(&app.peek.shown,Some(peek::Body::Lines(lines)) if lines==&["alphaZ"]),
                "R5 successful new Lines remain held during drag"
            );
            mouse(&mut app, MouseKind::Release, end);
            let _ = frame(&mut app);
            assert!(
                matches!(&app.select.as_ref().expect("same text").stage,Stage::Ready(text) if text=="alp"),
                "copy originally highlighted text"
            );
            assert!(app.ship_staged());
            let _ = frame(&mut app);
            assert!(
                matches!(&app.peek.shown,Some(peek::Body::Lines(lines)) if lines==&["newest"]),
                "bound: next frame adopts newest after ship"
            );
        }
    }
}
