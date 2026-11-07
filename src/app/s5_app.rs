//! S5 app lifecycle, request routing and generation barriers.
#![allow(
    clippy::expect_used,
    reason = "acceptance assertions and channel setup"
)]
use super::loader::{Answer, Request, Wake};
use super::settings::{ResetLine, StatsScope};
use super::{App, take_keys};
use crate::console::input;
use crate::stats::{self, Source};
use crate::stats_fixture as f;
use crate::stats_store::disk::{Glance, Progress, Reset, ThisProgress};
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

fn published() -> Glance {
    let bytes = f::birth(
        f::S,
        1,
        &f::fields("codex", None, Some("sol"), Some("high")),
    )
    .into_bytes();
    let view = stats::fold(
        vec![Source::live(
            "alpha",
            format!("session_id={}\n", f::S).as_bytes(),
            Ok(bytes),
        )],
        Some(f::S),
    );
    Glance::Ready {
        view,
        epoch: 1,
        r#gen: 1,
        updating: false,
        all: Progress {
            listed: 1,
            pending: 0,
        },
        this: ThisProgress::Ready(Progress {
            listed: 1,
            pending: 0,
        }),
    }
}
fn frame(app: &mut App) -> String {
    let mut buf = Buffer::empty(Rect::new(0, 0, 180, 60));
    app.frame(&mut buf);
    (0..60)
        .map(|y| (0..180).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
fn key(app: &mut App, text: &str) {
    let _ = take_keys(
        app,
        vec![(input::Key::Text(text.as_bytes().to_vec()), Instant::now())],
    );
}
fn asks(rx: &Receiver<Request>) -> Vec<(u64, bool)> {
    rx.try_iter()
        .filter_map(|request| match request {
            Request::Stats { generation, shown } => Some((generation, shown)),
            _ => None,
        })
        .collect()
}
fn land(app: &mut App, generation: u64, glance: Glance) {
    app.answer(Answer::Stats {
        generation,
        glance: Box::new(glance),
    });
}
fn reset_result(app: &mut App, generation: u64, outcome: Reset) {
    let (tx, rx) = mpsc::channel();
    tx.send(Wake::StatsReset {
        generation,
        outcome: Ok(outcome),
    })
    .expect("wake");
    let mut keys = input::Keys::app();
    let mut first = None;
    let _ = super::drain(app, &mut keys, &rx, &mut first);
}

#[test]
fn s5_app_shown_hidden_and_scope_requests_replace_the_once_per_open_contract() {
    let (tx, rx) = mpsc::channel();
    let mut app = App::new(None, None, None, Some(tx));
    app.fleeted = true;
    app.open_settings();
    let generation = app.model.settings().expect("open").generation;
    frame(&mut app);
    assert!(asks(&rx).is_empty(), "Quota does not arm Statistics");
    key(&mut app, "7");
    frame(&mut app);
    assert_eq!(asks(&rx), [(generation, true)]);
    land(&mut app, generation, published());
    assert!(frame(&mut app).contains("Recorded worker spawns: 1"));
    frame(&mut app);
    assert!(asks(&rx).is_empty(), "repaint is not a new arm");
    key(&mut app, "m");
    frame(&mut app);
    assert_eq!(app.settings_bodies.stats.scope, StatsScope::ThisSession);
    assert!(
        asks(&rx).is_empty(),
        "scope is a local selection of the already-read answer"
    );
    key(&mut app, "1");
    frame(&mut app);
    assert_eq!(
        asks(&rx),
        [(generation, false)],
        "hidden drops loader watch"
    );
    key(&mut app, "7");
    frame(&mut app);
    assert_eq!(asks(&rx), [(generation, true)], "returning to tab re-arms");
}

#[test]
fn s5_app_stale_answers_cannot_paint_a_closed_or_reopened_overlay() {
    let (tx, rx) = mpsc::channel();
    let mut app = App::new(None, None, None, Some(tx));
    app.fleeted = true;
    app.open_settings();
    let first = app.model.settings().expect("open").generation;
    key(&mut app, "7");
    frame(&mut app);
    let _ = asks(&rx);
    app.close_settings();
    let closed = app.settings_bodies.stats.clone();
    land(&mut app, first, published());
    assert_eq!(app.settings_bodies.stats, closed);
    app.open_settings();
    let second = app.model.settings().expect("reopened").generation;
    assert_ne!(first, second);
    assert_eq!(app.settings_bodies.stats.scope, StatsScope::All);
    let fresh = app.settings_bodies.stats.clone();
    land(&mut app, first, published());
    assert_eq!(app.settings_bodies.stats, fresh);
    key(&mut app, "7");
    frame(&mut app);
    land(&mut app, second, published());
    assert!(frame(&mut app).contains("Recorded worker spawns: 1"));
}

#[test]
fn s5_app_r_hands_one_reset_only_from_statistics_and_busy_keeps_the_answer() {
    let (tx, rx) = mpsc::channel();
    let mut app = App::new(None, None, None, Some(tx));
    app.fleeted = true;
    app.open_settings();
    let generation = app.model.settings().expect("open").generation;
    key(&mut app, "r");
    assert_eq!(
        app.take_reset(),
        None,
        "other tabs cannot rebuild Statistics"
    );
    key(&mut app, "7");
    frame(&mut app);
    let _ = asks(&rx);
    land(&mut app, generation, published());
    key(&mut app, "r");
    assert_eq!(app.take_reset(), Some(generation));
    assert_eq!(app.take_reset(), None, "handed once");
    assert_eq!(
        app.settings_bodies.stats.store.reset,
        Some(ResetLine::Requested)
    );
    reset_result(&mut app, generation, Reset::Busy);
    let shown = frame(&mut app);
    assert!(
        shown.contains("store is busy; nothing was removed"),
        "{shown}"
    );
    assert!(shown.contains("Recorded worker spawns: 1"));
    land(&mut app, generation, published());
    assert!(
        frame(&mut app).contains("store is busy; nothing was removed"),
        "ordinary refresh must not erase the named reset refusal"
    );
}

#[test]
fn s5_app_reset_completion_rearms_only_its_original_overlay_generation() {
    let (tx, rx) = mpsc::channel();
    let mut app = App::new(None, None, None, Some(tx));
    app.fleeted = true;
    app.open_settings();
    let first = app.model.settings().expect("open").generation;
    key(&mut app, "7");
    frame(&mut app);
    let _ = asks(&rx);
    land(&mut app, first, published());
    key(&mut app, "r");
    let _ = app.take_reset();
    app.close_settings();
    app.open_settings();
    let second = app.model.settings().expect("reopened").generation;
    key(&mut app, "7");
    frame(&mut app);
    let _ = asks(&rx);
    let before = app.settings_bodies.stats.clone();
    reset_result(&mut app, first, Reset::Rebuilt { epoch: 2 });
    assert_eq!(
        app.settings_bodies.stats, before,
        "old completion cannot repaint the new open"
    );
    assert!(
        asks(&rx).is_empty(),
        "old completion cannot arm a replacement watch"
    );
    land(&mut app, second, published());
    key(&mut app, "r");
    let _ = app.take_reset();
    reset_result(&mut app, second, Reset::Rebuilt { epoch: 3 });
    frame(&mut app);
    assert!(
        asks(&rx).contains(&(second, true)),
        "successful reset re-arms an immediate read"
    );
    land(&mut app, second, Glance::Building);
    let shown = frame(&mut app);
    assert!(shown.contains("Statistics are being built: no source read yet"));
    assert!(
        !shown.contains("Recorded worker spawns:"),
        "reset success must not leave old counts as a complete answer"
    );
}
