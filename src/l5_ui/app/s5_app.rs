//! S5 app lifecycle, request routing and generation barriers.
#![allow(
    clippy::expect_used,
    reason = "acceptance assertions and channel setup"
)]
use super::loader::{Answer, Request, Wake};
use super::maintainer::Upkeep;
use super::settings::{ResetLine, StatsScope};
use super::{App, take_keys};
use crate::console::input;
use crate::stats::{self, Source};
use crate::stats_fixture as f;
use crate::stats_store::disk::{Glance, Progress, Reset, ThisProgress};
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

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

// Only terminal/process effects are substituted. The app, turn, reset queue,
// worker, disk reset and completion below are the real implementations.
struct NoOutside;
impl super::Outside for NoOutside {
    fn suspend(&mut self) {
        panic!("reset handover must not suspend the terminal");
    }
    fn exec(&mut self, _argv: &[String]) -> std::io::Error {
        panic!("reset handover must not exec");
    }
    fn resume(&mut self) -> Result<(), String> {
        panic!("reset handover must not resume the terminal");
    }
    fn clear(&mut self) -> std::io::Result<()> {
        panic!("reset handover must not clear the terminal");
    }
}

fn actual_reset_completion(
    app: &mut App,
    upkeep: &mut Upkeep,
    rx: &Receiver<Wake>,
    generation: u64,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        upkeep.tend();
        if let Ok(Wake::StatsReset {
            generation: got,
            outcome,
        }) = rx.recv_timeout(Duration::from_millis(10))
        {
            assert_eq!(
                got, generation,
                "the actual worker retains the asking generation"
            );
            assert!(matches!(outcome, Ok(Reset::Rebuilt { .. })), "{outcome:?}");
            app.reset_landed(got, outcome);
            return;
        }
    }
    panic!("the actual worker must finish the handed reset");
}

#[test]
fn s5_app_step_held_reset_draws_waiting_and_runs_after_resume() {
    use crate::stats_store::s5_fixture::{Fixture, S, birth};
    let root = Fixture::new();
    root.live("alpha", S, &birth(S, 1));
    root.build();
    let (tx, rx) = mpsc::channel();
    let (mut upkeep, hold) = Upkeep::start_holding(root.0.clone(), tx);
    assert!(hold.entered(Duration::from_secs(3)));
    upkeep.pause(Duration::ZERO);
    let mut app = App::new(None, None, None, None);
    app.fleeted = true;
    app.open_settings();
    key(&mut app, "7");
    frame(&mut app);
    let generation = app.model.settings().expect("open").generation;
    key(&mut app, "r");
    let mut sink = Vec::new();
    let turn =
        super::step(&mut app, &mut NoOutside, &mut upkeep, &[], &mut sink).expect("real app turn");
    assert!(
        matches!(turn, super::Step::Stay(true)),
        "Waiting needs a redraw"
    );
    assert_eq!(upkeep.owed_reset(), Some(generation));
    assert_eq!(
        app.take_reset(),
        None,
        "the app hands the request only once"
    );
    assert_eq!(
        app.settings_bodies.stats.store.reset,
        Some(ResetLine::Waiting)
    );
    let shown = frame(&mut app);
    assert!(
        shown.contains("rebuild waits: statistics upkeep is not running"),
        "{shown}"
    );
    assert!(
        matches!(
            super::step(&mut app, &mut NoOutside, &mut upkeep, &[], &mut sink)
                .expect("following app turn"),
            super::Step::Stay(false)
        ),
        "an already handed request does not repeatedly demand a redraw"
    );
    assert_eq!(upkeep.owed_reset(), Some(generation));
    drop(hold);
    upkeep.resume();
    actual_reset_completion(&mut app, &mut upkeep, &rx, generation);
    assert_eq!(upkeep.owed_reset(), None);
    assert_eq!(
        app.settings_bodies.stats.store.reset,
        Some(ResetLine::Rebuilt)
    );
    upkeep.stop(Duration::from_secs(1));
}

#[test]
fn s5_app_step_sent_reset_stays_requested_without_waiting() {
    use crate::stats_store::s5_fixture::{Fixture, S, birth};
    let root = Fixture::new();
    root.live("alpha", S, &birth(S, 1));
    root.build();
    let (tx, rx) = mpsc::channel();
    let (mut upkeep, hold) = Upkeep::start_holding(root.0.clone(), tx);
    assert!(hold.entered(Duration::from_secs(3)));
    let mut app = App::new(None, None, None, None);
    app.fleeted = true;
    app.open_settings();
    key(&mut app, "7");
    frame(&mut app);
    let generation = app.model.settings().expect("open").generation;
    key(&mut app, "r");
    let mut sink = Vec::new();
    let turn =
        super::step(&mut app, &mut NoOutside, &mut upkeep, &[], &mut sink).expect("real app turn");
    assert!(
        matches!(turn, super::Step::Stay(false)),
        "a sent reset is not held"
    );
    assert_eq!(upkeep.owed_reset(), None);
    assert_eq!(
        app.take_reset(),
        None,
        "the app hands the request only once"
    );
    assert_eq!(
        app.settings_bodies.stats.store.reset,
        Some(ResetLine::Requested)
    );
    assert!(!frame(&mut app).contains("rebuild waits:"));
    drop(hold);
    actual_reset_completion(&mut app, &mut upkeep, &rx, generation);
    assert_eq!(
        app.settings_bodies.stats.store.reset,
        Some(ResetLine::Rebuilt)
    );
    upkeep.stop(Duration::from_secs(1));
}
