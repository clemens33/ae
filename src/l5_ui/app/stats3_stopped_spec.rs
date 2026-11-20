//! Frozen #287 slice 3, Q1: the real upkeep stop remains visible in Settings.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "owned fixture IO and bounded upkeep synchronization"
)]

use super::loader::Answer;
use super::maintainer::Upkeep;
use super::settings::StatsScope;
use super::{App, run, take_keys};
use crate::console::input;
use crate::stats_store::disk::{self, Files};
use crate::stats_store::maint::Outcome;
use crate::stats_store::s5_fixture::{Fixture, S, birth, drain};
use ratatui_core::{buffer::Buffer, layout::Rect};
use std::fs;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn frame(app: &mut App) -> String {
    let mut buffer = Buffer::empty(Rect::new(0, 0, 180, 60));
    app.frame(&mut buffer);
    (0..60)
        .map(|y| {
            (0..180)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn key(app: &mut App, key: &[u8]) {
    let _ = take_keys(app, vec![(input::Key::Text(key.to_vec()), Instant::now())]);
}

fn says_stopped(app: &mut App) {
    let text = frame(app);
    for word in ["upkeep stopped", "stale", "restart ae app"] {
        assert!(
            text.contains(word),
            "persistent stop must explain {word}: {text}"
        );
    }
}

#[test]
fn stats3_real_upkeep_stop_survives_flash_expiry_refresh_and_settings_reopen() {
    // Product-valid repeated damage while incomplete discovery keeps the
    // repair episode open. Reuse the source conditions of the S5 contract.
    let root = Fixture::new();
    let mut journal = birth(S, 1);
    let dir = root.live("alpha", S, &journal);
    fs::remove_dir(root.0.join("archive")).expect("replace archive namespace");
    fs::write(root.0.join("archive"), b"not a directory").expect("incomplete discovery");
    let mut upkeep = root.maintainer();
    drain(&mut upkeep);
    fs::write(root.0.join("stats/view"), b"first damage\n").expect("first damage");
    journal.extend(birth(S, 2));
    fs::write(dir.join("events.jsonl"), &journal).expect("trigger first repair");
    assert!((0..32).any(|_| matches!(upkeep.step().outcome, Outcome::Repaired { .. })));
    drain(&mut upkeep);
    fs::write(root.0.join("stats/view"), b"second damage\n").expect("second damage");
    journal.extend(birth(S, 3));
    fs::write(dir.join("events.jsonl"), journal).expect("trigger repeat damage");
    for _ in 0..32 {
        if upkeep.stopped() {
            break;
        }
        let _ = upkeep.step();
    }
    assert!(
        upkeep.stopped(),
        "real maintainer must stop before the UI proof"
    );

    let (sender, wakes) = mpsc::channel();
    let (thread, hold) = Upkeep::start_with_holding(root.0.clone(), sender, upkeep);
    assert!(hold.entered(Duration::from_secs(3)));
    drop(hold);
    let mut first = Some(
        wakes
            .recv_timeout(Duration::from_secs(5))
            .expect("real stop wake"),
    );
    thread.stop(Duration::from_secs(1));
    let mut app = App::new(None, None, None, None);
    app.fleeted = true;
    let mut keys = input::Keys::app();
    assert!(run::drain(&mut app, &mut keys, &wakes, &mut first).is_some());
    app.flash = None; // The transient notice has expired.
    app.open_settings();
    key(&mut app, b"7");
    says_stopped(&mut app);

    let generation = app.model.settings().expect("open settings").generation;
    app.answer(Answer::Stats {
        generation,
        glance: Box::new(disk::glance(&mut Files::at(&root.0), Some(S))),
    });
    app.flash = None;
    says_stopped(&mut app);
    app.settings_bodies.stats.scope = StatsScope::ThisSession;
    says_stopped(&mut app);
    let _ = take_keys(&mut app, vec![(input::Key::Escape, Instant::now())]);
    app.open_settings();
    key(&mut app, b"7");
    app.flash = None;
    says_stopped(&mut app);
}
