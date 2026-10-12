//! Frozen P1 App seam: A3 incomplete reads, incarnation and input precedence.
#![allow(
    clippy::expect_used,
    clippy::disallowed_methods,
    reason = "owned acceptance fixtures"
)]

use std::collections::BTreeMap;
use std::time::Instant;

use super::handover::{Handover, View};
use super::loader::{Answer, FleetRead, Reader};
use super::model::{Drag, Edge, Key as Browse};
use super::tests::{ID, Root, entry, session};
use super::{App, take_keys};
use crate::console::input::{Key, Mouse, MouseKind};
use crate::digest::Status;
use crate::listing::World;
use crate::theme::FleetOrder;
use crate::time::Timestamp;

const OTHER: &str = "6f1c2b7e-0d3a-4c58-9a2e-5b1d7c9e0a11";

fn saved() -> View {
    View {
        session: "web".into(),
        id: ID.into(),
        focus: Some("colead".into()),
        sidebar: Some(31),
        list: Some(9),
        pages: 2,
        rows: 3,
        list_top: Some(1),
        body_top: 4,
    }
}

fn read(root: &Root, names: &[&str], scanned: bool) -> FleetRead {
    let dirs: BTreeMap<_, _> = names
        .iter()
        .map(|n| ((*n).to_owned(), root.0.join("sessions").join(n)))
        .collect();
    let world = World::new(
        Timestamp::now(),
        names
            .iter()
            .map(|n| entry(n, Status::Running, None))
            .collect(),
    );
    let mut read = Reader::new(root.0.clone(), Some("api".into()), None).fold(
        dirs,
        world,
        None,
        &FleetOrder::EMPTY,
        Timestamp::now(),
    );
    read.scanned = scanned;
    read
}

fn pending() -> (Root, App) {
    let root = Root::new("view-pending");
    session(&root, "web", "");
    let mut app = App::new(Some("api".into()), None, None, None);
    app.answer(Answer::Handover {
        view: saved(),
        at: Timestamp::now().epoch(),
    });
    (root, app)
}

fn restored(app: &App) {
    assert_eq!(
        app.model.selected(),
        Some("web"),
        "identity-keyed selection restored"
    );
    assert_eq!(app.model.focus(), Some("colead"));
    assert_eq!(
        (app.model.split().sidebar, app.model.split().list),
        (Some(31), Some(9))
    );
    assert_eq!(app.model.scroll_rows(10), 23);
    assert_eq!((app.model.list_top(), app.model.body_top()), (Some(1), 4));
    assert!(app.keep.is_none(), "applied once");
}

#[test]
fn matching_complete_read_applies_all_preferences_together() {
    let (root, mut app) = pending();
    app.answer(Answer::Fleet(read(&root, &["api", "web"], true)));
    restored(&app);
    app.model.key(Browse::Digit(1), &app.fleet, false);
    app.answer(Answer::Fleet(read(&root, &["api", "web"], true)));
    assert_eq!(
        app.model.selected(),
        Some("api"),
        "consumed restore never reapplies"
    );
}

#[test]
fn incomplete_omission_or_unreadable_uuid_defers_then_a_later_match_applies() {
    for missing in [true, false] {
        let (root, mut app) = pending();
        let mut first = read(
            &root,
            if missing { &["api"] } else { &["api", "web"] },
            false,
        );
        if !missing {
            first.ids.insert("web".into(), String::new());
        }
        app.answer(Answer::Fleet(first));
        assert!(app.keep.is_some(), "incomplete absence proves nothing");
        assert_eq!(app.model.selected(), Some("api"));
        app.answer(Answer::Fleet(read(&root, &["api", "web"], true)));
        restored(&app);
    }
}

#[test]
fn complete_absence_unreadable_or_replaced_incarnation_discards_all_preferences() {
    for case in 0..4 {
        let (root, mut app) = pending();
        assert!(
            app.keep.is_some(),
            "valid handover waits for a deciding fleet read"
        );
        let mut next = read(
            &root,
            if case == 0 { &["api"] } else { &["api", "web"] },
            case != 3,
        );
        if case == 1 {
            next.ids.insert("web".into(), String::new());
        }
        if case >= 2 {
            next.ids.insert("web".into(), OTHER.into());
        }
        app.answer(Answer::Fleet(next));
        assert!(app.keep.is_none(), "case {case}: decisive read discards");
        assert_eq!(app.model.selected(), Some("api"));
        assert_eq!((app.model.focus(), app.model.scroll_rows(10)), (None, 0));
        assert_eq!(
            (
                app.model.split().sidebar,
                app.model.list_top(),
                app.model.body_top()
            ),
            (None, None, 0)
        );
        app.answer(Answer::Fleet(read(&root, &["api", "web"], true)));
        assert_eq!(
            app.model.selected(),
            Some("api"),
            "later matching read cannot revive it"
        );
    }
}

#[test]
fn delayed_handover_retains_write_epoch_and_expires_while_pending() {
    let (root, mut app) = pending();
    app.answer(Answer::Fleet(read(&root, &["api"], false)));
    assert!(
        app.keep.is_some(),
        "incomplete read keeps a pending handover"
    );
    app.keep.as_mut().expect("pending").at = Timestamp::now().epoch() - 61;
    app.answer(Answer::Fleet(read(&root, &["api", "web"], true)));
    assert_eq!(
        app.model.selected(),
        Some("api"),
        "61-second-old view cannot apply"
    );
    assert!(app.keep.is_none());
    app.answer(Answer::Handover {
        view: saved(),
        at: Timestamp::now().epoch() - 61,
    });
    app.answer(Answer::Fleet(read(&root, &["api", "web"], true)));
    assert_eq!(app.model.focus(), None, "receipt cannot renew the epoch");
}

#[test]
fn every_input_cancels_pending_restore_even_before_first_fleet_or_handover_answer() {
    for early in [true, false] {
        for key in [
            Key::Text(b"s".to_vec()),
            Key::Text(b"v".to_vec()),
            Key::Text(b"i".to_vec()),
            Key::Text(b"x".to_vec()),
            Key::Tab,
            Key::Mouse(Mouse {
                column: 1,
                row: 1,
                kind: MouseKind::Click,
            }),
        ] {
            let (root, mut app) = pending();
            if !early {
                app.answer(Answer::Fleet(read(&root, &["api"], false)));
            }
            let _ = take_keys(&mut app, vec![(key, Instant::now())]);
            assert!(
                app.keep.is_none() && app.touched,
                "all input ends pending view"
            );
            app.answer(Answer::Handover {
                view: saved(),
                at: Timestamp::now().epoch(),
            });
            app.answer(Answer::Fleet(read(&root, &["api", "web"], true)));
            assert_eq!(app.model.selected(), Some("api"), "no late yank");
        }
    }
}

#[test]
fn writer_snapshot_preserves_only_view_and_requires_a_readable_selection_uuid() {
    let root = Root::new("view-writer");
    session(&root, "web", "");
    let mut app = App::new(Some("api".into()), None, None, None);
    app.answer(Answer::Fleet(read(&root, &["api", "web"], true)));
    app.model.key(Browse::Digit(2), &app.fleet, false);
    app.model.set_focus(Some("colead".into()));
    app.model.key(Browse::PageUp, &app.fleet, false);
    app.model.wheel_body(false, 20);
    app.model.grab(Drag {
        edge: Edge::Sidebar,
        from: 31,
        at: 31,
    });
    app.model.drag_to(31);
    let want = Handover {
        pane: "%7".into(),
        version: "2026.10.99".into(),
        at: 123,
        view: View {
            sidebar: Some(31),
            list: None,
            pages: 1,
            rows: 0,
            list_top: None,
            body_top: 3,
            ..saved()
        },
    };
    assert_eq!(
        app.handover("%7", "2026.10.99", 123),
        Some(want),
        "pure writer snapshot"
    );
    app.ids.insert("web".into(), String::new());
    assert!(
        app.handover("%7", "2026.10.99", 123).is_none(),
        "unreadable identity not carried"
    );
}

#[test]
fn model_adopt_cannot_select_a_row_missing_from_the_fleet() {
    let root = Root::new("view-model");
    let fleet = read(&root, &["api"], true).fleet;
    let mut model = super::model::Model::new(&fleet);
    model.adopt(&fleet, &saved());
    assert_eq!(model.selected(), Some("api"));
    assert!(
        model.view("").is_none(),
        "missing UUID prevents any snapshot"
    );
    assert_eq!(model.view(ID).expect("snapshot").session, "api");
}
