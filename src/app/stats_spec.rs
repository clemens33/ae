//! #287 slice2: real App wire, once-per-open reads, scope and generation gates.
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::loader::{Answer, Request};
use super::model::{Act, Key, Model, SettingsTab};
use super::settings::StatsScope;
use super::{App, take_keys};
use crate::app::fleet::Fleet;
use crate::console::input;
use crate::stats::{self, Source, View};
use crate::stats_fixture as f;

fn view() -> View {
    let (s, t) = f::matrix();
    stats::fold(
        vec![
            Source::live("S", format!("session_id={}\n", f::S).as_bytes(), Ok(s)),
            Source::live("T", format!("session_id={}\n", f::T).as_bytes(), Ok(t)),
        ],
        Some(f::S),
    )
}
fn frame(app: &mut App) -> String {
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 60));
    app.frame(&mut buf);
    (0_u16..60)
        .map(|y| {
            (0_u16..160)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn key(app: &mut App, word: &str) {
    let _ = take_keys(
        app,
        vec![(input::Key::Text(word.as_bytes().to_vec()), Instant::now())],
    );
}
fn requests(rx: &Receiver<Request>) -> Vec<u64> {
    rx.try_iter()
        .filter_map(|request| match request {
            Request::Stats { generation } => Some(generation),
            _ => None,
        })
        .collect()
}
fn answer(app: &mut App, generation: u64) {
    app.answer(Answer::Stats {
        generation,
        view: Box::new(view()),
    });
}

#[test]
fn digit_seven_and_tab_cycle_route_statistics_and_m_is_local_to_that_tab() {
    let fleet = Fleet::default();
    let mut model = Model::new(&fleet);
    model.open_settings(7);
    assert_eq!(model.key(Key::Digit(6), &fleet, false), Act::Redraw);
    assert_eq!(model.settings().expect("open").tab, SettingsTab::Accounts);
    assert_eq!(model.key(Key::Tab, &fleet, false), Act::Redraw);
    assert_eq!(model.settings().expect("open").tab, SettingsTab::Statistics);
    assert_eq!(model.key(Key::Mode, &fleet, false), Act::StatsScope);
    assert_eq!(model.key(Key::Tab, &fleet, false), Act::Redraw);
    assert_eq!(model.settings().expect("open").tab, SettingsTab::Quota);
    assert_eq!(model.key(Key::Mode, &fleet, false), Act::None);
    model.key(Key::Digit(7), &fleet, false);
    assert_eq!(model.settings().expect("open").tab, SettingsTab::Statistics);
}

#[test]
fn first_statistics_frame_asks_once_scope_toggle_and_quota_frames_do_not_read() {
    let (tx, rx) = mpsc::channel();
    let mut app = App::new(None, None, None, Some(tx));
    app.fleeted = true;
    app.open_settings();
    let generation = app.model.settings().expect("open").generation;
    frame(&mut app);
    assert!(
        requests(&rx).is_empty(),
        "opening Quota must not scan statistics"
    );
    key(&mut app, "7");
    assert!(frame(&mut app).contains("loading"));
    assert_eq!(requests(&rx), vec![generation]);
    frame(&mut app);
    app.ask_stats();
    assert!(requests(&rx).is_empty(), "pending scan is asked once");
    answer(&mut app, generation);
    let ready = frame(&mut app);
    assert!(ready.contains("Recorded worker spawns: 12"), "{ready}");
    assert_eq!(app.settings_bodies.stats.scope, StatsScope::All);
    let held = app.settings_bodies.stats.view.clone();
    key(&mut app, "m");
    assert_eq!(app.settings_bodies.stats.scope, StatsScope::ThisSession);
    let current = frame(&mut app);
    assert!(current.contains("Recorded worker spawns: 10"), "{current}");
    assert!(current.contains("Configured model: 5 / 10"), "{current}");
    assert_eq!(
        app.settings_bodies.stats.view, held,
        "toggle selects a precomputed scope"
    );
    key(&mut app, "m");
    assert!(frame(&mut app).contains("Recorded worker spawns: 12"));
    key(&mut app, "1");
    key(&mut app, "m");
    assert_eq!(
        app.settings_bodies.stats.scope,
        StatsScope::All,
        "m on Quota does not toggle stats"
    );
    frame(&mut app);
    key(&mut app, "7");
    frame(&mut app);
    assert!(
        requests(&rx).is_empty(),
        "scope, quota repaint, and revisiting tab do not reread"
    );
}

#[test]
fn stale_answers_cannot_paint_closed_or_reopened_settings_and_reopen_does_not_accumulate() {
    let (tx, rx) = mpsc::channel();
    let mut app = App::new(None, None, None, Some(tx));
    app.fleeted = true;
    app.open_settings();
    let first = app.model.settings().expect("first").generation;
    key(&mut app, "7");
    frame(&mut app);
    assert_eq!(requests(&rx), vec![first]);
    app.close_settings();
    let before = app.settings_bodies.stats.clone();
    answer(&mut app, first);
    assert_eq!(app.settings_bodies.stats, before, "closed answer discarded");
    app.open_settings();
    let second = app.model.settings().expect("second").generation;
    assert_ne!(second, first);
    assert!(
        app.settings_bodies.stats.view.is_none(),
        "new open has no earlier result"
    );
    assert_eq!(app.settings_bodies.stats.scope, StatsScope::All);
    answer(&mut app, first);
    assert!(
        app.settings_bodies.stats.view.is_none(),
        "old delayed answer discarded after reopen"
    );
    key(&mut app, "7");
    frame(&mut app);
    assert_eq!(requests(&rx), vec![second]);
    answer(&mut app, second);
    let ready = frame(&mut app);
    assert!(
        ready.contains("Recorded worker spawns: 12"),
        "same input remains twelve: {ready}"
    );
    assert!(!ready.contains(f::PRIVATE));
    assert!(requests(&rx).is_empty());
}
