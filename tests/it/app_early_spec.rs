//! Frozen P2: ruling R5/A1/A2(9), lead notice/cap and input-precedence pins.
#![allow(clippy::expect_used, reason = "bounded acceptance fixtures")]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use super::loader::{Answer, Reader};
use super::model::Tab;
use super::tests::{Root, entry, session};
use super::{App, EARLY_CAP, EARLY_NOTICE, take_keys};
use crate::console::input::{Key, Mouse, MouseKind};
use crate::digest::Status;
use crate::listing::World;
use crate::theme::FleetOrder;
use crate::time::Timestamp;

const NOTICE: &str = "input before sessions loaded was dropped";

fn cold() -> App {
    App::new(Some("api".into()), None, None, None)
}

fn loaded(app: &mut App, tag: &str) {
    let root = Root::new(tag);
    session(&root, "web", "");
    let dirs: BTreeMap<_, _> = ["api", "web"]
        .into_iter()
        .map(|n| (n.to_owned(), root.0.join("sessions").join(n)))
        .collect();
    let world = World::new(
        Timestamp::now(),
        ["api", "web"]
            .into_iter()
            .map(|n| entry(n, Status::Running, None))
            .collect(),
    );
    let read = Reader::new(root.0.clone(), Some("api".into()), None).fold(
        dirs,
        world,
        None,
        &FleetOrder::EMPTY,
        Timestamp::now(),
    );
    app.answer(Answer::Fleet(read));
}

#[test]
fn pre_fleet_digit_selects_after_the_first_fleet_read() {
    let mut app = cold();
    let _ = take_keys(&mut app, vec![(Key::Text(b"2".to_vec()), Instant::now())]);
    assert_eq!(app.model.selected(), Some("api"));
    loaded(&mut app, "early-digit");
    assert_eq!(
        app.model.selected(),
        Some("web"),
        "digit is replayed, not lost"
    );
    assert!(app.early.is_empty());
}

#[test]
fn navigation_has_no_pre_fleet_mutation_and_replays_in_byte_order() {
    let mut app = cold();
    let at = Instant::now();
    let _ = take_keys(
        &mut app,
        vec![
            (Key::Tab, at),
            (Key::PageUp, at),
            (Key::Text(b"21".to_vec()), at),
        ],
    );
    assert_eq!(
        (app.model.tab(), app.model.pages()),
        (Tab::Overview, 0),
        "cold model never navigates"
    );
    loaded(&mut app, "early-order");
    assert_eq!(
        (app.model.selected(), app.model.tab(), app.model.pages()),
        (Some("api"), Tab::Agents, 0)
    );
    assert!(app.early.is_empty());
}

#[test]
fn peek_settings_and_other_actions_are_dropped_once_and_never_replayed() {
    let mut app = cold();
    let at = Instant::now();
    let _ = take_keys(
        &mut app,
        vec![
            (Key::Text(b"vs?iynpo[]".to_vec()), at),
            (Key::Enter, at),
            (Key::Pasted(b"draft".to_vec()), at),
        ],
    );
    assert_eq!(
        app.note().as_deref(),
        Some(NOTICE),
        "one visible drop notice"
    );
    let flash = app.flash.clone();
    let _ = take_keys(
        &mut app,
        vec![(Key::Text(b"sv".to_vec()), at + Duration::from_secs(1))],
    );
    assert_eq!(app.flash, flash, "repeat drops never refresh the notice");
    loaded(&mut app, "early-actions");
    assert!(!app.model.settings_open() && !app.model.peek_open() && !app.composing());
    assert!(app.queued.is_empty() && app.deferred.is_empty() && app.early.is_empty());
    assert_eq!(app.note().as_deref(), Some(NOTICE));
    assert_eq!(EARLY_NOTICE, NOTICE);
}

#[test]
fn all_pre_fleet_mouse_kinds_are_dropped_under_the_same_notice() {
    for kind in [
        MouseKind::Click,
        MouseKind::Release,
        MouseKind::Drag,
        MouseKind::WheelUp,
        MouseKind::WheelDown,
    ] {
        let mut app = cold();
        let before = app.model.clone();
        let _ = take_keys(
            &mut app,
            vec![(
                Key::Mouse(Mouse {
                    column: 1,
                    row: 1,
                    kind,
                }),
                Instant::now(),
            )],
        );
        assert_eq!(app.note().as_deref(), Some(NOTICE), "mouse drop is visible");
        assert_eq!(app.model, before, "mouse never mutates the cold model");
        loaded(&mut app, "early-mouse");
        assert_eq!(
            (app.model.selected(), app.model.tab(), app.model.pages()),
            (Some("api"), Tab::Overview, 0)
        );
        assert!(app.early.is_empty() && app.deferred.is_empty());
    }
}

#[test]
fn every_allowed_navigation_key_is_queued_with_its_original_read_instant() {
    let mut app = cold();
    let at = Instant::now()
        .checked_sub(Duration::from_secs(6))
        .expect("old read time");
    app.last_input = Some(at);
    let mut events = vec![(Key::Text(b"123456789!jk".to_vec()), at)];
    let arrows = [
        Key::Up,
        Key::Down,
        Key::PageUp,
        Key::PageDown,
        Key::Tab,
        Key::Escape,
    ];
    events.extend(arrows.iter().cloned().map(|k| (k, at)));
    let _ = take_keys(&mut app, events);
    let mut want: Vec<_> = b"123456789!jk"
        .iter()
        .map(|b| (Key::Text(vec![*b]), at))
        .collect();
    want.extend(arrows.into_iter().map(|k| (k, at)));
    assert_eq!(
        app.early, want,
        "only the approved navigation subset is retained"
    );
    loaded(&mut app, "early-times");
    assert_eq!(
        app.last_input,
        Some(at),
        "replay never fabricates a fresh read"
    );
    assert!(
        app.blocker(Instant::now()).is_none(),
        "old input cannot renew RecentInput"
    );
}

#[test]
fn overflow_evicts_oldest_and_retains_only_the_newest_64_navigation_keys() {
    let mut app = cold();
    let at = Instant::now();
    let mut events = vec![(Key::Text(b"2".to_vec()), at)];
    events.extend((0..64).map(|_| (Key::PageUp, at)));
    let _ = take_keys(&mut app, events);
    assert_eq!(app.early.len(), 64, "queue is bounded");
    assert_eq!(
        app.early.first(),
        Some(&(Key::PageUp, at)),
        "oldest digit evicted"
    );
    assert_eq!(app.note().as_deref(), Some(NOTICE));
    loaded(&mut app, "early-overflow");
    assert_eq!((app.model.selected(), app.model.pages()), (Some("api"), 64));
    assert_eq!(EARLY_CAP, 64);
}

#[test]
fn quit_interrupt_and_double_q_keep_acting_before_the_first_read() {
    let at = Instant::now();
    assert!(take_keys(&mut cold(), vec![(Key::Interrupt, at)]).is_none());
    let mut app = cold();
    assert!(take_keys(&mut app, vec![(Key::Text(b"q".to_vec()), at)]).is_some());
    assert!(app.note().is_some_and(|n| n.contains("q again to quit")));
    assert!(
        take_keys(
            &mut app,
            vec![(Key::Text(b"q".to_vec()), at + Duration::from_millis(1))]
        )
        .is_none()
    );
    let mut app = cold();
    let _ = take_keys(&mut app, vec![(Key::Text(b"q2".to_vec()), at)]);
    assert!(
        app.armed.is_none(),
        "queued navigation still disarms a prior q"
    );
}
