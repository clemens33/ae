//! Frozen issue 314 acceptance tests, authored independently before the fix.
//! Install as `src/app/early_settings_spec.rs` and wire from src/app.rs under cfg(test).
#![allow(clippy::expect_used, reason = "bounded acceptance fixtures")]

use std::time::Instant;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::loader::{Answer, Reader, Request};
use super::model::SettingsTab;
use super::tests::{Root, entry};
use super::{App, take_keys};
use crate::console::input::Key;
use crate::digest::Status;
use crate::listing::World;
use crate::theme::FleetOrder;
use crate::time::Timestamp;

fn cold() -> App {
    App::new(None, None, None, None)
}

fn text(app: &mut App, bytes: &[u8]) {
    assert_eq!(
        take_keys(app, vec![(Key::Text(bytes.to_vec()), Instant::now())]),
        Some(true)
    );
}

fn load(app: &mut App, tag: &str) {
    let root = Root::new(tag);
    let dirs = ["api", "web"]
        .into_iter()
        .map(|name| (name.to_owned(), root.0.join("sessions").join(name)))
        .collect();
    let world = World::new(
        Timestamp::now(),
        ["api", "web"]
            .into_iter()
            .map(|name| entry(name, Status::Running, None))
            .collect(),
    );
    let read = Reader::new(root.0.clone(), None, None).fold(
        dirs,
        world,
        None,
        &FleetOrder::EMPTY,
        Timestamp::now(),
    );
    app.answer(Answer::Fleet(read));
}

fn tab(app: &App) -> SettingsTab {
    app.model.settings().expect("Settings is open").tab
}

fn frame(app: &mut App) -> String {
    let mut buffer = Buffer::empty(Rect::new(0, 0, 160, 45));
    app.frame(&mut buffer);
    buffer
        .content
        .iter()
        .map(ratatui_core::buffer::Cell::symbol)
        .collect()
}

#[test]
fn comma_and_question_paint_settings_on_the_next_frame_before_the_fleet() {
    for (key, wanted) in [(b',', SettingsTab::Quota), (b'?', SettingsTab::Keys)] {
        let mut app = cold();
        assert!(!frame(&mut app).contains("Settings"));
        text(&mut app, &[key]);
        assert_eq!(tab(&app), wanted);
        assert!(!app.fleeted, "the first fleet read has not arrived");
        assert!(app.early.is_empty());
        assert!(app.note().is_none(), "the accepted key has no drop notice");
        let shown = frame(&mut app);
        assert!(shown.contains("Settings"), "{shown}");
    }
}

#[test]
fn an_immediate_opener_hands_the_same_text_runs_tail_to_settings() {
    for bytes in [b",5".as_slice(), b"?1"] {
        let mut app = cold();
        text(&mut app, bytes);
        let wanted = if bytes[0] == b',' {
            SettingsTab::Keys
        } else {
            SettingsTab::Quota
        };
        assert_eq!(tab(&app), wanted);
        assert!(app.note().is_none());
        assert!(app.early.is_empty());
    }
}

#[test]
fn navigation_then_opener_replays_in_order_in_one_or_several_reads() {
    for (index, chunks) in [
        vec![b"2,".as_slice()],
        vec![b"2".as_slice(), b","],
        vec![b"2?".as_slice()],
        vec![b"2".as_slice(), b"?"],
    ]
    .into_iter()
    .enumerate()
    {
        let mut app = cold();
        let wanted = if chunks.last().expect("a chunk").last() == Some(&b'?') {
            SettingsTab::Keys
        } else {
            SettingsTab::Quota
        };
        for chunk in chunks {
            text(&mut app, chunk);
        }
        assert!(
            !app.model.settings_open(),
            "navigation precedes the overlay"
        );
        assert!(app.note().is_none(), "a queued opener is not dropped");
        load(&mut app, &format!("issue314-order-{index}"));
        assert_eq!(app.model.selected(), Some("web"));
        assert_eq!(tab(&app), wanted, "2 selected a session, never a tab");
        assert!(app.early.is_empty());
    }
}

#[test]
fn the_tail_after_a_queued_opener_keeps_its_overlay_meaning_across_reads() {
    for (index, chunks) in [
        vec![b"2,5".as_slice()],
        vec![b"2".as_slice(), b",", b"5"],
        vec![b"2?1".as_slice()],
        vec![b"2?".as_slice(), b"1"],
    ]
    .into_iter()
    .enumerate()
    {
        let mut app = cold();
        let wanted = if chunks.last().expect("a chunk").last() == Some(&b'5') {
            SettingsTab::Keys
        } else {
            SettingsTab::Quota
        };
        for chunk in chunks {
            text(&mut app, chunk);
        }
        load(&mut app, &format!("issue314-tail-{index}"));
        assert_eq!(app.model.selected(), Some("web"));
        assert_eq!(tab(&app), wanted);
        assert!(app.note().is_none());
    }
}

#[test]
fn a_non_text_key_after_a_queued_opener_is_owned_by_the_overlay() {
    let mut app = cold();
    text(&mut app, b"2,");
    assert_eq!(
        take_keys(&mut app, vec![(Key::Tab, Instant::now())]),
        Some(true)
    );
    load(&mut app, "issue314-tab-tail");
    assert_eq!(app.model.selected(), Some("web"));
    assert_eq!(tab(&app), SettingsTab::Config);
    assert!(app.note().is_none());
}

#[test]
fn a_non_navigation_letter_after_a_queued_opener_keeps_its_overlay_meaning() {
    use super::settings::StatsScope;

    for (index, chunks) in [vec![b"2,7m".as_slice()], vec![b"2,".as_slice(), b"7", b"m"]]
        .into_iter()
        .enumerate()
    {
        let mut app = cold();
        assert_eq!(app.settings_bodies.stats.scope, StatsScope::All);
        for chunk in chunks {
            text(&mut app, chunk);
        }
        assert_eq!(app.settings_bodies.stats.scope, StatsScope::All);
        load(&mut app, &format!("issue314-scope-tail-{index}"));
        assert_eq!(app.model.selected(), Some("web"));
        assert_eq!(tab(&app), SettingsTab::Statistics);
        assert_eq!(app.settings_bodies.stats.scope, StatsScope::ThisSession);
        assert!(app.note().is_none());
    }
}

#[test]
fn q_after_a_queued_opener_closes_the_overlay_in_order_instead_of_arming_quit() {
    for (index, chunks) in [vec![b"2,q".as_slice()], vec![b"2,".as_slice(), b"q"]]
        .into_iter()
        .enumerate()
    {
        let mut app = cold();
        for chunk in chunks {
            text(&mut app, chunk);
        }
        assert!(app.armed.is_none(), "q belongs to the queued overlay");
        load(&mut app, &format!("issue314-q-tail-{index}"));
        assert_eq!(app.model.selected(), Some("web"));
        assert!(!app.model.settings_open());
        assert!(app.armed.is_none());
        assert!(app.note().is_none());
    }
}

#[test]
fn interrupt_quits_even_when_an_overlay_opener_is_waiting() {
    let mut app = cold();
    text(&mut app, b"2,");
    assert!(take_keys(&mut app, vec![(Key::Interrupt, Instant::now())]).is_none());
}

#[test]
fn first_fleet_focus_reaches_the_loader_after_an_early_open_without_home() {
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut app = App::new(None, None, None, Some(sender));
    text(&mut app, b",");
    assert!(
        receiver
            .try_iter()
            .any(|request| matches!(request, Request::Settings { open: true, .. }))
    );
    load(&mut app, "issue314-focus");
    app.focus();
    assert!(receiver.try_iter().any(|request| matches!(
        request,
        Request::Focus(Some(name)) if name == "api"
    )));
    assert_eq!(tab(&app), SettingsTab::Quota);
}
