//! Frozen characterisation of `take_keys` dispatch (slice takekeys, #300),
//! authored by navigator takekeys-nav (Meta/muse-wonder-full-1.0-contributor)
//! on base b9cd091b before any refactor. Oracle: docs/app.md `## Keys` /
//! `## Leaving` where they state a binding (marked DOC); everywhere else the
//! current base output, labelled a behaviour pin (marked PIN). The S-numbers
//! are the per-key steps of .local/takekeys/plan.md sec. 2 as critiqued in
//! .local/takekeys/nav-critique.md. Groups: G1 pre-loop/read-press, G2
//! newline + pre-fleet, G3 hold, G4 spent/modal, G5 disarm, G6 selection,
//! G7 mode chain + batch order. Bug-shaped behaviour is pinned, never fixed.
#![allow(clippy::expect_used, reason = "bounded acceptance fixtures")]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::{Position, Rect};

use super::loader::{Answer, Reader};
use super::select::{Panel, Region, Selection};
use super::tests::{Root, entry, session};
use super::{App, Arm, Armed, Held, Keep, Press, Queued, Stage, take_keys};
use crate::console::input::{Input, Key, Mouse, MouseKind, Reading};
use crate::digest::Status;
use crate::listing::World;
use crate::theme::FleetOrder;
use crate::time::Timestamp;

fn cold() -> App {
    App::new(Some("api".into()), None, None, None)
}

/// A fleeted app on a scratch root holding running `api` (home) and `web`.
fn loaded(tag: &str) -> (Root, App) {
    let root = Root::new(tag);
    session(&root, "web", "");
    let mut app = cold();
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
    assert!(app.fleeted, "fixture fleeted");
    assert_eq!(app.model.selected(), Some("api"));
    (root, app)
}

/// Start writing the selected session; the instant just after entry began.
fn writing(app: &mut App) -> Instant {
    app.take(Reading::Owner, Instant::now());
    app.write(true);
    assert!(app.composing(), "writing started");
    Instant::now()
}

fn drafted(app: &App) -> String {
    app.shown_input().map(Input::draft).unwrap_or_default()
}

fn framed(app: &mut App) {
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
    app.frame(&mut buf);
}

fn keep() -> Keep {
    Keep {
        view: super::handover::View {
            session: "api".to_owned(),
            id: "id".to_owned(),
            focus: None,
            sidebar: None,
            list: None,
            pages: 0,
            rows: 0,
            list_top: None,
            body_top: 0,
        },
        at: 0,
    }
}

fn queued() -> Queued {
    Queued {
        key: ("api".to_owned(), "id".to_owned()),
        entry: 0,
        raw: Vec::new(),
        seat: "lead".to_owned(),
        body: "x".to_owned(),
        about: String::new(),
        note: None,
    }
}

fn at(base: Instant, ms: u64) -> Instant {
    base + Duration::from_millis(ms)
}

// ---------------------------------------------------------------- G1: P0/S1

#[test]
fn empty_batch_is_some_false_and_leaves_keep_and_touched() {
    let (_root, mut app) = loaded("tk-empty");
    (app.keep, app.touched) = (Some(keep()), false);
    assert_eq!(take_keys(&mut app, Vec::new()), Some(false));
    assert!(app.keep.is_some(), "no keys, no handover forfeited");
    assert!(!app.touched, "no keys, nothing touched");
}

#[test]
fn nonempty_batch_clears_keep_even_when_the_key_is_dropped() {
    let mut app = cold();
    (app.keep, app.touched) = (Some(keep()), false);
    let batch = vec![(Key::Text(b"v".to_vec()), Instant::now())];
    assert_eq!(take_keys(&mut app, batch), Some(true), "PIN: early path");
    assert!(app.keep.is_none() && app.touched, "P0 ran before the drop");
}

#[test]
fn nonmouse_key_clears_read_press_even_when_spent() {
    let (_root, mut app) = loaded("tk-press-spent");
    let base = Instant::now();
    app.read_press = Some((Press::Web, Position::new(1, 1), app.model.epoch()));
    app.cut = Some(at(base, 10));
    let got = take_keys(&mut app, vec![(Key::Down, base)]);
    assert_eq!(got, Some(false), "PIN: a spent key adds no redraw");
    assert!(app.read_press.is_none(), "S1 ran before the spent skip");
    assert_eq!(
        app.model.selected(),
        Some("api"),
        "the spent key never acted"
    );
}

/// An app with a pane handoff pending, a read press and one held key.
fn pane_app() -> (App, Instant) {
    use crate::console::needs::SeatRef;
    let mut app = App::new(None, None, None, None);
    app.fleeted = true;
    app.pane_view = Some(super::paneview::Ask {
        session: "demo".to_owned(),
        uuid: "drawn-incarnation".to_owned(),
        dir: "/unused/takekeys-spec".into(),
        seat: SeatRef {
            slot: "main".to_owned(),
            name: "lead".to_owned(),
        },
        label: "lead".to_owned(),
    });
    let base = Instant::now();
    app.deferred.push((Key::Text(b"a".to_vec()), base));
    app.read_press = Some((Press::Web, Position::new(1, 1), app.model.epoch()));
    (app, base)
}

#[test]
fn pane_view_text_key_clears_read_press_and_holds_nothing() {
    let (mut app, base) = pane_app();
    let batch = vec![(Key::Text(b"q".to_vec()), at(base, 1))];
    assert_eq!(take_keys(&mut app, batch), Some(true), "PIN: S2 return");
    assert!(app.read_press.is_none(), "S1 ran before the S2 return");
    assert!(app.deferred.is_empty(), "S2 clears keys held for the seat");
    assert!(app.pane_view.is_some() && app.armed.is_none());
}

#[test]
fn pane_view_mouse_key_keeps_read_press_and_holds_nothing() {
    let (mut app, base) = pane_app();
    let click = Key::Mouse(Mouse {
        kind: MouseKind::Click,
        column: 1,
        row: 1,
    });
    assert_eq!(take_keys(&mut app, vec![(click, at(base, 1))]), Some(true));
    assert!(app.read_press.is_some(), "PIN: S1 skips Mouse keys");
    assert!(app.deferred.is_empty(), "S2 clears keys held for the seat");
}

// ------------------------------------------------------------- G2: S3/S4

#[test]
fn newline_while_browsing_acts_as_enter_and_starts_writing() {
    let (_root, mut app) = loaded("tk-newline-enter");
    let got = take_keys(&mut app, vec![(Key::Newline, Instant::now())]);
    assert_eq!(got, Some(true));
    assert!(app.composing(), "DOC: Enter writes the selected session");
}

#[test]
fn newline_while_composing_is_a_line_break_not_a_send() {
    let (_root, mut app) = loaded("tk-newline-break");
    let base = writing(&mut app);
    let keys = vec![(Key::Text(b"hi".to_vec()), at(base, 1))];
    assert_eq!(take_keys(&mut app, keys), Some(true));
    assert_eq!(
        take_keys(&mut app, vec![(Key::Newline, at(base, 2))]),
        Some(true)
    );
    let keys = vec![(Key::Text(b"x".to_vec()), at(base, 3))];
    assert_eq!(take_keys(&mut app, keys), Some(true));
    assert!(app.composing(), "DOC: ^J puts a line break in the draft");
    // Input::draft() is the one-line inert spelling: the break reads as a
    // space there, with the x after it and nothing sent.
    assert_eq!(
        drafted(&app),
        "hi x",
        "PIN: the break sits between hi and x"
    );
    assert!(app.queued.is_empty(), "nothing was sent");
}

#[test]
fn newline_held_behind_a_queued_ask_is_stored_as_enter() {
    let (_root, mut app) = loaded("tk-newline-held");
    app.queued.push(queued());
    let base = Instant::now();
    let batch = vec![(Key::Newline, base), (Key::Text(b"j".to_vec()), base)];
    assert_eq!(take_keys(&mut app, batch), Some(true));
    assert_eq!(
        app.deferred,
        [(Key::Enter, base), (Key::Text(b"j".to_vec()), base)],
        "PIN: S3 rewrites before S5 stores the key"
    );
}

#[test]
fn prefleet_settings_overlay_takes_the_full_path_not_early() {
    let mut app = cold();
    app.model.open_settings(1);
    assert!(app.model.settings_open());
    let batch = vec![(Key::Text(b"j".to_vec()), Instant::now())];
    assert!(take_keys(&mut app, batch).is_some());
    assert!(
        app.early.is_empty(),
        "PIN: overlay keys never queue for replay"
    );
    assert!(app.model.settings_open(), "the key went to the overlay");
}

// ----------------------------------------------------------------- G3: S5

#[test]
fn held_keys_append_after_existing_deferred_then_rest_in_order() {
    let (_root, mut app) = loaded("tk-hold-order");
    let base = Instant::now();
    app.deferred.push((Key::Text(b"a".to_vec()), base));
    let batch = vec![
        (Key::Text(b"b".to_vec()), at(base, 1)),
        (Key::Down, at(base, 2)),
    ];
    assert_eq!(take_keys(&mut app, batch), Some(true));
    assert_eq!(
        app.deferred,
        [
            (Key::Text(b"a".to_vec()), base),
            (Key::Text(b"b".to_vec()), at(base, 1)),
            (Key::Down, at(base, 2)),
        ],
        "PIN: hold order is old, current, rest, origins kept"
    );
    assert_eq!(app.model.selected(), Some("api"), "held keys never acted");
}

#[test]
fn held_unguarded_interrupt_quits_with_nothing_queued_and_no_text() {
    let (_root, mut app) = loaded("tk-hold-quit");
    let base = Instant::now();
    app.deferred.push((Key::Text(b"a".to_vec()), base));
    let got = take_keys(&mut app, vec![(Key::Interrupt, at(base, 1))]);
    assert_eq!(got, None, "PIN: a held ^C still quits at once");
    assert_eq!(
        app.deferred,
        [
            (Key::Text(b"a".to_vec()), base),
            (Key::Interrupt, at(base, 1))
        ],
        "PIN: the key is stored before the quit decision reads it"
    );
}

#[test]
fn held_interrupt_behind_a_queued_ask_never_quits() {
    let (_root, mut app) = loaded("tk-hold-queued");
    app.queued.push(queued());
    let base = Instant::now();
    let got = take_keys(&mut app, vec![(Key::Interrupt, base)]);
    assert_eq!(got, Some(true), "PIN: a quit never loses a queued ask");
    assert_eq!(app.deferred, [(Key::Interrupt, base)]);
}

#[test]
fn held_interrupt_read_before_a_clear_reads_guarded_and_holds() {
    let (_root, mut app) = loaded("tk-hold-preclear");
    let base = Instant::now();
    app.cleared_at = Some(at(base, 60_000));
    app.deferred.push((Key::Text(b"a".to_vec()), base));
    let got = take_keys(&mut app, vec![(Key::Interrupt, at(base, 1))]);
    assert_eq!(
        got,
        Some(true),
        "PIN: each held ^C is guarded at its own stored origin, saturating"
    );
}

#[test]
fn held_interrupt_after_the_guard_expires_quits() {
    let (_root, mut app) = loaded("tk-hold-expired");
    let base = Instant::now();
    app.cleared_at = Some(base);
    app.deferred.push((Key::Text(b"a".to_vec()), base));
    let got = take_keys(&mut app, vec![(Key::Interrupt, at(base, 3_000))]);
    assert_eq!(got, None, "DOC: the clear guards ^C for 2 seconds only");
}

#[test]
fn held_interrupt_with_draft_text_holds_and_keeps_the_draft() {
    let (_root, mut app) = loaded("tk-hold-text");
    let base = writing(&mut app);
    let keys = vec![(Key::Text(b"hi".to_vec()), at(base, 1))];
    assert_eq!(take_keys(&mut app, keys), Some(true));
    app.deferred.push((Key::Text(b"a".to_vec()), at(base, 2)));
    let got = take_keys(&mut app, vec![(Key::Interrupt, at(base, 3))]);
    assert_eq!(got, Some(true), "DOC: ^C over text never quits");
    assert_eq!(drafted(&app), "hi", "a held ^C does not clear either");
}

// ------------------------------------------------------------- G4: S6/S7

#[test]
fn spent_key_is_skipped_and_a_later_key_in_the_batch_still_acts() {
    let (_root, mut app) = loaded("tk-spent-batch");
    let base = Instant::now();
    app.cut = Some(base);
    let batch = vec![(Key::Down, base), (Key::Down, at(base, 1))];
    assert_eq!(take_keys(&mut app, batch), Some(true));
    assert_eq!(
        app.model.selected(),
        Some("web"),
        "PIN: origin == cut is spent, the next key acts"
    );
}

/// Open the session menu on the painted selection and show that frame.
fn menu_shown(app: &mut App, base: Instant) {
    framed(app);
    let batch = vec![(Key::Text(b"m".to_vec()), base)];
    assert_eq!(take_keys(app, batch), Some(true));
    assert!(app.modal.is_some(), "fixture: menu opened");
    framed(app);
    app.frame_shown(at(base, 1));
}

#[test]
fn spent_escape_does_not_step_the_open_menu() {
    let (_root, mut app) = loaded("tk-spent-modal");
    let base = Instant::now();
    menu_shown(&mut app, base);
    app.cut = Some(at(base, 2));
    let got = take_keys(&mut app, vec![(Key::Escape, at(base, 2))]);
    assert_eq!(got, Some(false), "PIN: spent adds no redraw before modal");
    assert!(
        app.modal.is_some(),
        "PIN: S6 runs before the modal sees keys"
    );
    let got = take_keys(&mut app, vec![(Key::Escape, at(base, 3))]);
    assert_eq!(got, Some(true));
    assert!(app.modal.is_none(), "a fresh Escape closes the menu");
}

#[test]
fn open_menu_takes_text_before_browse_moves_the_selection() {
    let (_root, mut app) = loaded("tk-modal-first");
    let base = Instant::now();
    menu_shown(&mut app, base);
    let batch = vec![(Key::Text(b"j".to_vec()), at(base, 2))];
    assert!(take_keys(&mut app, batch).is_some());
    assert!(app.modal.is_some(), "the menu took the key and stayed open");
    assert_eq!(
        app.model.selected(),
        Some("api"),
        "PIN: browse never saw the j under the modal"
    );
}

#[test]
fn queued_ask_holds_a_key_before_the_open_menu_sees_it() {
    let (_root, mut app) = loaded("tk-hold-modal");
    let base = Instant::now();
    menu_shown(&mut app, base);
    app.queued.push(queued());
    let got = take_keys(&mut app, vec![(Key::Escape, at(base, 2))]);
    assert_eq!(got, Some(true), "PIN: S5 returns before S7");
    assert!(app.modal.is_some(), "the held Escape never closed the menu");
    assert_eq!(app.deferred, [(Key::Escape, at(base, 2))]);
}

// ----------------------------------------------------------------- G5: S8

#[test]
fn noop_navigation_key_disarms_and_the_disarm_alone_redraws() {
    let (_root, mut app) = loaded("tk-disarm-nav");
    let base = Instant::now();
    app.armed = Some(Armed {
        at: base,
        what: Arm::Quit,
    });
    let got = take_keys(&mut app, vec![(Key::Left, at(base, 1))]);
    assert_eq!(got, Some(true), "PIN: redraw is the disarm alone");
    assert!(app.armed.is_none(), "DOC: any other key disarms");
    assert_eq!(app.model.selected(), Some("api"));
}

#[test]
fn text_while_composing_disarms_and_lands_in_the_draft() {
    let (_root, mut app) = loaded("tk-disarm-compose");
    let base = writing(&mut app);
    app.armed = Some(Armed {
        at: base,
        what: Arm::Quit,
    });
    let keys = vec![(Key::Text(b"x".to_vec()), at(base, 1))];
    assert_eq!(take_keys(&mut app, keys), Some(true));
    assert!(app.armed.is_none(), "PIN: composing is not browsing at S8");
    assert_eq!(drafted(&app), "x");
}

#[test]
fn text_under_peek_keeps_the_armed_press() {
    let (_root, mut app) = loaded("tk-disarm-peek");
    app.model.open_peek(1);
    assert!(app.model.peek_open());
    let base = Instant::now();
    app.armed = Some(Armed {
        at: base,
        what: Arm::Quit,
    });
    let keys = vec![(Key::Text(b"j".to_vec()), at(base, 1))];
    assert!(take_keys(&mut app, keys).is_some());
    assert!(
        app.armed.is_some(),
        "PIN (quirk): peek counts as browsing at S8, settings does not"
    );
    assert!(
        app.model.peek_open(),
        "the key scrolled the panel, not browse"
    );
}

#[test]
fn interrupt_never_disarms_at_s8_and_clears_the_draft() {
    let (_root, mut app) = loaded("tk-disarm-interrupt");
    let base = writing(&mut app);
    let keys = vec![(Key::Text(b"hi".to_vec()), at(base, 1))];
    assert_eq!(take_keys(&mut app, keys), Some(true));
    app.armed = Some(Armed {
        at: at(base, 2),
        what: Arm::Quit,
    });
    let got = take_keys(&mut app, vec![(Key::Interrupt, at(base, 3))]);
    assert_eq!(got, Some(true), "DOC: ^C over text clears, never quits");
    assert_eq!(drafted(&app), "");
    assert!(
        app.armed
            .as_ref()
            .is_some_and(|armed| matches!(armed.what, Arm::Quit)),
        "PIN: S8 exempts Interrupt; the clear does not disarm"
    );
}

// ------------------------------------------------------------ G6: S9/S10

fn held_selection(app: &App, moved: bool) -> Held {
    let region = Region {
        panel: Panel::Lane,
        area: Rect::new(0, 0, 10, 10),
    };
    let mut sel = Selection::start(&region, Position::new(1, 1), 0);
    if moved {
        sel.extend(Position::new(4, 1));
        assert!(sel.moved(), "fixture: selection moved");
    }
    Held {
        sel,
        epoch: app.model.epoch(),
        toggled: false,
        before: None,
        down: false,
        stage: Stage::None,
        touched: Instant::now(),
    }
}

#[test]
fn escape_with_a_moved_selection_only_clears_it_and_writing_survives() {
    let (_root, mut app) = loaded("tk-sel-moved");
    let base = writing(&mut app);
    app.select = Some(held_selection(&app, true));
    let got = take_keys(&mut app, vec![(Key::Escape, at(base, 1))]);
    assert_eq!(got, Some(true));
    assert!(app.select.is_none(), "DOC: Esc ends a selection");
    assert!(
        app.composing(),
        "PIN: that Escape never reached the composer"
    );
}

#[test]
fn escape_with_an_unmoved_selection_consumes_it_and_still_dispatches() {
    let (_root, mut app) = loaded("tk-sel-unmoved");
    let base = writing(&mut app);
    let keys = vec![(Key::Text(b"hi".to_vec()), at(base, 1))];
    assert_eq!(take_keys(&mut app, keys), Some(true));
    app.select = Some(held_selection(&app, false));
    let got = take_keys(&mut app, vec![(Key::Escape, at(base, 2))]);
    assert_eq!(got, Some(true));
    assert!(app.select.is_none(), "PIN: S10 take() ran for this Escape");
    assert!(!app.composing(), "DOC: Esc goes back to browsing");
    assert_eq!(drafted(&app), "hi", "DOC: Esc keeps the draft");
}

#[test]
fn nonescape_key_leaves_a_current_selection_in_place() {
    let (_root, mut app) = loaded("tk-sel-kept");
    app.select = Some(held_selection(&app, false));
    let got = take_keys(&mut app, vec![(Key::Left, Instant::now())]);
    assert_eq!(got, Some(false), "PIN: Left alone owes no frame");
    assert!(
        app.select.is_some(),
        "PIN: S10 take() is Escape-conditional only"
    );
}

// ------------------------------------------------- G7: S11 chain + batch

#[test]
fn interrupt_under_settings_with_a_draft_clears_and_never_quits() {
    let (_root, mut app) = loaded("tk-settings-clear");
    let base = writing(&mut app);
    let keys = vec![(Key::Text(b"hi".to_vec()), at(base, 1))];
    assert_eq!(take_keys(&mut app, keys), Some(true));
    app.open_settings();
    assert!(app.model.settings_open());
    let got = take_keys(&mut app, vec![(Key::Interrupt, at(base, 2))]);
    assert_eq!(
        got,
        Some(true),
        "DOC: ^C over text, also under Settings, clears and never quits"
    );
    assert_eq!(drafted(&app), "");
    assert!(app.model.settings_open(), "the overlay never saw the ^C");
}

#[test]
fn interrupt_under_peek_with_a_draft_clears_and_never_quits() {
    let (_root, mut app) = loaded("tk-peek-clear");
    let base = writing(&mut app);
    let keys = vec![(Key::Text(b"hi".to_vec()), at(base, 1))];
    assert_eq!(take_keys(&mut app, keys), Some(true));
    app.model.open_peek(1);
    assert!(app.model.peek_open());
    let got = take_keys(&mut app, vec![(Key::Interrupt, at(base, 2))]);
    assert_eq!(
        got,
        Some(true),
        "DOC: ^C over text, also under a peek, clears and never quits"
    );
    assert_eq!(drafted(&app), "");
    assert!(app.model.peek_open(), "the panel never saw the ^C");
}

#[test]
fn interrupt_under_settings_without_a_draft_quits() {
    let (_root, mut app) = loaded("tk-settings-quit");
    app.open_settings();
    assert!(app.model.settings_open());
    let got = take_keys(&mut app, vec![(Key::Interrupt, Instant::now())]);
    assert_eq!(got, None, "PIN: chain step 1 misses, settings quits");
}

#[test]
fn settings_overlay_takes_text_before_the_suspended_draft() {
    let (_root, mut app) = loaded("tk-settings-first");
    let base = writing(&mut app);
    let keys = vec![(Key::Text(b"a".to_vec()), at(base, 1))];
    assert_eq!(take_keys(&mut app, keys), Some(true));
    app.open_settings();
    let keys = vec![(Key::Text(b"b".to_vec()), at(base, 2))];
    assert!(take_keys(&mut app, keys).is_some());
    assert_eq!(drafted(&app), "a", "PIN: settings routes before composing");
}

#[test]
fn held_composer_swallows_a_plain_key_with_no_redraw() {
    let (_root, mut app) = loaded("tk-held-swallow");
    app.held = Some("not writing".to_owned());
    let keys = vec![(Key::Text(b"j".to_vec()), Instant::now())];
    assert_eq!(
        take_keys(&mut app, keys),
        Some(false),
        "PIN: retry swallows"
    );
    assert_eq!(app.model.selected(), Some("api"), "browse never saw it");
}

#[test]
fn key_ends_a_drag_before_it_acts_and_the_ending_alone_redraws() {
    let (_root, mut app) = loaded("tk-drag-end");
    app.model.grab(crate::app::model::Drag {
        edge: crate::app::model::Edge::Sidebar,
        from: 10,
        at: 10,
    });
    let got = take_keys(&mut app, vec![(Key::Left, Instant::now())]);
    assert_eq!(got, Some(true), "PIN: redraw is let_go alone (acted false)");
    assert!(app.model.drag().is_none(), "the drag ended first");
}

#[test]
fn mid_batch_enter_then_text_lands_in_the_new_draft() {
    let (_root, mut app) = loaded("tk-batch-enter");
    // Entry is stamped on the real clock during the first key, so both
    // origins sit ahead of it: neither reads as typed-before-writing.
    let base = Instant::now() + Duration::from_secs(30);
    let batch = vec![(Key::Enter, base), (Key::Text(b"b".to_vec()), at(base, 1))];
    assert_eq!(take_keys(&mut app, batch), Some(true));
    assert!(app.composing(), "PIN: mode is re-read for every key");
    assert_eq!(drafted(&app), "b");
}

#[test]
fn mid_batch_quit_abandons_the_rest_of_the_batch() {
    let (_root, mut app) = loaded("tk-batch-quit");
    let base = Instant::now();
    let batch = vec![
        (Key::Text(b"q".to_vec()), base),
        (Key::Text(b"q".to_vec()), at(base, 1)),
        (Key::Text(b"j".to_vec()), at(base, 2)),
    ];
    assert_eq!(take_keys(&mut app, batch), None, "DOC: qq quits");
    assert_eq!(
        app.model.selected(),
        Some("api"),
        "PIN: the j after the quit never acted"
    );
}
