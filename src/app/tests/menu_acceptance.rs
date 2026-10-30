//! Independent acceptance: painted identity and original input times.
//! Install as `src/app/tests/menu_acceptance.rs`, child of `app.rs::tests`.
//! `frame_shown` is the agreed real post-terminal.draw owner; no test shim.

use super::*;

fn shown(app: &mut App, at: Instant) -> String {
    let text = framed(app);
    app.frame_shown(at);
    text
}

fn keyed(app: &mut App, bytes: &[u8], at: Instant) {
    assert!(take_keys(app, vec![(Key::Text(bytes.to_vec()), at)]).is_some());
}

fn noticed(app: &App, word: &str) -> bool {
    app.notices.iter().any(|(_, item)| item.body.contains(word))
}

fn menu(text: &str) -> bool {
    text.lines()
        .any(|line| line.contains("s  Stop") || line.contains("s  Start"))
}

fn right(app: &mut App, name: &str, at: Instant) {
    let screen = framed(app);
    let row = screen
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            line.chars()
                .take(30)
                .collect::<String>()
                .split_whitespace()
                .any(|word| word == name)
                .then_some(row)
        })
        .expect("painted named card in fixed 160x45 sidebar");
    let mut keys = crate::console::input::Keys::app();
    let decoded = keys.feed(format!("\x1b[<2;9;{}M", row + 1).as_bytes(), at);
    assert_eq!(
        decoded.len(),
        1,
        "one right press, not text or an empty decode"
    );
    assert!(take_keys(app, decoded).is_some());
}

fn fixture(tag: &str) -> (Root, App, Reader) {
    let root = Root::new(tag);
    let (mut app, reader) = housed(&root);
    app.answer(Answer::Look {
        name: "api".to_owned(),
        look: Some(crate::theme::Look::read("off", "darcula", "off", "off")),
        zone: None,
    });
    framed(&mut app);
    (root, app, reader)
}

#[test]
fn keyboard_menu_is_for_painted_selection_and_has_only_this_release_actions() {
    let (_root, mut app, _reader) = fixture("menu-keys");
    let selection = app.model.selected().map(str::to_owned);
    keyed(&mut app, b"m", Instant::now());
    let screen = shown(&mut app, Instant::now());
    assert!(menu(&screen), "keyboard menu absent: {screen}");
    assert!(
        screen.contains("api"),
        "question/menu must name painted target"
    );
    for future in ["Pin", "Unpin", "Resume", "End…", "End..."] {
        assert!(
            !screen.contains(future),
            "future placeholder {future}: {screen}"
        );
    }
    assert_eq!(app.model.selected().map(str::to_owned), selection);
    assert!(app.queued.is_empty(), "opening cannot submit a draft");
}

#[test]
fn unseen_menu_input_is_consumed_before_compose_settings_copy_or_open() {
    let (_root, mut app, _reader) = fixture("menu-blind");
    let at = Instant::now();
    keyed(&mut app, b"mosy", at);
    assert!(take_keys(&mut app, vec![(Key::Enter, at)]).is_some());
    let screen = shown(&mut app, Instant::now());
    assert!(menu(&screen), "batch left the unseen menu: {screen}");
    assert!(!app.composing());
    assert!(!app.model.settings_open());
    assert!(app.queued.is_empty());
    assert!(
        app.notices.is_empty(),
        "an unseen start or stop must never reach an owner"
    );
}

#[test]
fn first_visible_frame_is_strict_and_repaint_does_not_renew_it() {
    let (_root, mut app, _reader) = fixture("menu-visible");
    keyed(&mut app, b"m", Instant::now());
    let first = Instant::now();
    assert!(menu(&shown(&mut app, first)));
    keyed(&mut app, b"s", first);
    assert!(app.notices.is_empty(), "equal-origin input is not fresh");
    let fresh = first + Duration::from_millis(1);
    let _ = shown(&mut app, first + Duration::from_millis(2));
    keyed(&mut app, b"s", fresh);
    assert_eq!(
        app.notices.len(),
        1,
        "repaint must not starve a fresh press"
    );
    assert!(app.notices[0].1.body.contains("names no tmux server"));
}

#[test]
fn menu_dismissal_does_not_leak_enter_into_a_suspended_draft() {
    let (_root, mut app, _reader) = fixture("menu-draft");
    let entered = writing(&mut app);
    app.compose(Key::Text("a界 draft".as_bytes().to_vec()), entered)
        .expect("draft");
    app.compose(Key::Left, Instant::now()).expect("cursor left");
    let before = framed(&mut app);
    let draft = drafted(&app);
    let cursor = app.layout.cursor;
    right(&mut app, "api", Instant::now());
    assert!(menu(&shown(&mut app, Instant::now())));
    let batch = Instant::now();
    assert!(take_keys(&mut app, vec![(Key::Escape, batch), (Key::Enter, batch)]).is_some());
    let after = framed(&mut app);
    assert_eq!(drafted(&app), draft);
    assert_eq!(app.layout.cursor, cursor);
    assert!(app.composing(), "suspended mode must be restored");
    assert!(
        app.queued.is_empty(),
        "same-read Enter cannot submit after dismissal"
    );
    assert!(before.contains("draft") && after.contains("draft"));
}

#[test]
fn held_reason_survives_right_menu_and_consumed_cancel() {
    let (_root, mut app, _reader) = fixture("menu-held");
    app.held = Some("fixture owner refused this draft".to_owned());
    let held = app.held.clone();
    right(&mut app, "api", Instant::now());
    assert!(menu(&shown(&mut app, Instant::now())));
    assert!(take_keys(&mut app, vec![(Key::Escape, Instant::now())]).is_some());
    assert_eq!(app.held, held, "Esc belonged to the overlay, not Held");
    assert!(app.queued.is_empty());
}

#[test]
fn outside_and_second_right_press_close_without_changing_selection() {
    for second_right in [false, true] {
        let (_root, mut app, _reader) = fixture(if second_right {
            "menu-right-close"
        } else {
            "menu-outside"
        });
        let selection = app.model.selected().map(str::to_owned);
        keyed(&mut app, b"m", Instant::now());
        assert!(menu(&shown(&mut app, Instant::now())));
        if second_right {
            right(&mut app, "api", Instant::now());
        } else {
            let click = Mouse {
                kind: MouseKind::Click,
                column: 159,
                row: 40,
            };
            assert!(take_keys(&mut app, vec![(Key::Mouse(click), Instant::now())]).is_some());
        }
        assert!(!menu(&framed(&mut app)));
        assert_eq!(app.model.selected().map(str::to_owned), selection);
        assert!(app.queued.is_empty() && app.notices.is_empty());
    }
}

#[test]
fn modal_paste_wheel_and_ctrl_c_are_consumed() {
    let (_root, mut app, _reader) = fixture("menu-swallow");
    keyed(&mut app, b"m", Instant::now());
    assert!(menu(&shown(&mut app, Instant::now())));
    let scroll = app.model.scroll_rows(app.layout.page_rows);
    let wheel = Mouse {
        kind: MouseKind::WheelUp,
        column: 70,
        row: 20,
    };
    let at = Instant::now();
    assert!(
        take_keys(
            &mut app,
            vec![(Key::Pasted(b"sy\r".to_vec()), at), (Key::Mouse(wheel), at)]
        )
        .is_some()
    );
    assert_eq!(app.model.scroll_rows(app.layout.page_rows), scroll);
    assert!(menu(&framed(&mut app)) && app.queued.is_empty());
    assert!(
        take_keys(&mut app, vec![(Key::Interrupt, Instant::now())]).is_some(),
        "Ctrl-C dismisses the modal"
    );
    assert!(!menu(&framed(&mut app)));
}

#[test]
fn menu_blocks_idle_upgrade_until_closed() {
    let (_root, mut app, _reader) = fixture("menu-restart");
    app.last_input = None;
    assert!(
        app.blocker(Instant::now()).is_none(),
        "fixture is otherwise idle"
    );
    keyed(&mut app, b"m", Instant::now());
    assert!(
        app.blocker(Instant::now()).is_some(),
        "menu is restart-sensitive view state"
    );
    let _ = shown(&mut app, Instant::now());
    assert!(take_keys(&mut app, vec![(Key::Escape, Instant::now())]).is_some());
    assert!(app.blocker(Instant::now()).is_none());
}

#[test]
fn raw_ambiguous_identity_painted_then_repaired_never_acquires_authority() {
    let (root, mut app, reader) = fixture("menu-painted-sole");
    let path = root.0.join("sessions/api/meta");
    let good =
        String::from_utf8(crate::meta::read_bytes(&root.0.join("sessions/api")).expect("record"))
            .expect("UTF-8 fixture meta");
    std::fs::write(&path, format!("{good}session_id={ID}\n")).expect("duplicate");
    refold(&mut app, &reader, &root, &[("api", Status::Running)]);
    framed(&mut app);
    std::fs::write(&path, good).expect("repair without any new fleet read");
    keyed(&mut app, b"m", Instant::now());
    let screen = shown(&mut app, Instant::now());
    assert!(
        screen.contains("session_id"),
        "painted identity refusal disappeared: {screen}"
    );
    keyed(&mut app, b"s", Instant::now());
    assert!(noticed(&app, "session_id"));
    assert!(
        !noticed(&app, "names no tmux server"),
        "ambiguous painted identity reached Stop"
    );
    assert!(take_keys(&mut app, vec![(Key::Escape, Instant::now())]).is_some());
    refold(&mut app, &reader, &root, &[("api", Status::Running)]);
    framed(&mut app);
    keyed(&mut app, b"m", Instant::now());
    let _ = shown(&mut app, Instant::now());
    keyed(&mut app, b"s", Instant::now());
    assert!(
        noticed(&app, "names no tmux server"),
        "a newly painted sole identity must pass the menu guard"
    );
}

#[test]
fn m_is_literal_in_composer_and_swallowed_by_held() {
    let (_root, mut app, _reader) = fixture("menu-typing-m");
    let entered = writing(&mut app);
    keyed(&mut app, b"m", entered);
    assert_eq!(drafted(&app), "m");
    assert!(!menu(&framed(&mut app)));
    app.write(false);
    app.held = Some("fixture refused owner".to_owned());
    let draft = drafted(&app);
    keyed(&mut app, b"m", Instant::now());
    assert_eq!(drafted(&app), draft);
    assert_eq!(app.held.as_deref(), Some("fixture refused owner"));
    assert!(!menu(&framed(&mut app)));
}

#[test]
fn historical_right_press_never_freezes_current_replacement_uuid() {
    let (root, mut app, reader) = fixture("menu-old-uuid");
    let screen = framed(&mut app);
    let row = screen
        .lines()
        .position(|line| {
            line.chars()
                .take(30)
                .collect::<String>()
                .split_whitespace()
                .any(|word| word == "api")
        })
        .expect("painted api");
    let origin = Instant::now();
    let other = "0199c0de-bbbb-4890-abcd-ef0123456789";
    meta(&root, other, "colead");
    refold(&mut app, &reader, &root, &[("api", Status::Running)]);
    framed(&mut app);
    let mut decoder = crate::console::input::Keys::app();
    let events = decoder.feed(format!("\x1b[<2;9;{}M", row + 1).as_bytes(), origin);
    assert_eq!(events.len(), 1);
    assert!(take_keys(&mut app, events).is_some());
    let _ = shown(&mut app, Instant::now());
    keyed(&mut app, b"s", Instant::now());
    assert!(
        app.notices.iter().any(
            |(_, item)| ["changed", "replaced", "identity", "session_id"]
                .iter()
                .any(|word| item.body.contains(word))
        ),
        "old frame accepted replacement: {:?}",
        app.notices
            .iter()
            .map(|(_, item)| &item.body)
            .collect::<Vec<_>>()
    );
    assert!(
        !noticed(&app, "names no tmux server"),
        "replacement reached Stop with a freshly acquired UUID"
    );
}

#[test]
fn pre_fleet_menu_and_enter_are_discarded_and_never_replayed() {
    let root = Root::new("menu-prefleet");
    let reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
    let mut app = App::new(Some("api".to_owned()), None, None, None);
    framed(&mut app);
    let early = Instant::now();
    keyed(&mut app, b"m", early);
    assert!(take_keys(&mut app, vec![(Key::Enter, early)]).is_some());
    refold(&mut app, &reader, &root, &[("api", Status::Running)]);
    let screen = shown(&mut app, Instant::now());
    assert!(
        !menu(&screen) && !app.composing(),
        "early effect became fresh after fleet: {screen}"
    );
    assert!(app.queued.is_empty());
}

#[test]
fn historical_right_press_on_discarded_frame_refuses() {
    let (_root, mut app, _reader) = fixture("menu-lost");
    let origin = Instant::now();
    app.lost = Some(origin);
    let mut decoder = crate::console::input::Keys::app();
    let keys = decoder.feed(b"\x1b[<2;9;6M", origin);
    assert_eq!(keys.len(), 1);
    assert!(take_keys(&mut app, keys).is_some());
    let screen = framed(&mut app);
    assert!(!menu(&screen));
    assert!(
        screen.contains("frame is gone"),
        "lost frame must refuse visibly: {screen}"
    );
}

#[test]
fn orchestrator_has_no_menu_and_row_refusals_never_reach_an_owner() {
    let (root, mut app, reader) = fixture("menu-row-states");
    let name = crate::orchestrator::ORCHESTRATOR_SESSION;
    session(&root, name, "meta_agent=true\n");
    refold(&mut app, &reader, &root, &[(name, Status::Running)]);
    framed(&mut app);
    right(&mut app, name, Instant::now());
    let screen = shown(&mut app, Instant::now());
    assert!(!menu(&screen), "the orchestrator offers nothing: {screen}");
    assert!(screen.contains("nothing to start or stop"), "{screen}");
    for (status, fact, reason) in [
        (Status::Unknown, fleet::Facts::NotPublished, "unknown"),
        (Status::Running, fleet::Facts::OtherServer, "server"),
    ] {
        refold(&mut app, &reader, &root, &[("api", status)]);
        app.facts.insert("api".to_owned(), fact);
        framed(&mut app);
        right(&mut app, "api", Instant::now());
        let screen = shown(&mut app, Instant::now());
        assert!(
            menu(&screen) && screen.contains(reason),
            "{reason}: {screen}"
        );
        let count = app.noticed;
        keyed(&mut app, b"s", Instant::now());
        assert!(app.noticed > count, "activation must add a fresh refusal");
        let (target, item) = app.notices.last().expect("new row refusal");
        assert_eq!(target, "api");
        assert!(item.body.contains(reason));
        assert!(!noticed(&app, "names no tmux server"));
        assert!(take_keys(&mut app, vec![(Key::Escape, Instant::now())]).is_some());
    }
}

#[test]
fn a_stopped_session_offers_start_not_stop() {
    let (root, mut app, reader) = fixture("menu-stopped");
    refold(&mut app, &reader, &root, &[("api", Status::Stopped)]);
    app.facts
        .insert("api".to_owned(), fleet::Facts::NotPublished);
    framed(&mut app);
    right(&mut app, "api", Instant::now());
    let screen = shown(&mut app, Instant::now());
    assert!(screen.contains("s  Start"), "{screen}");
    assert!(!screen.contains("Stop…") && !screen.contains("Stop..."));
    let started = detached(&mut app);
    keyed(&mut app, b"s", Instant::now());
    assert!(
        noticed(&app, "Starting 'api'."),
        "Start works outside tmux: {:?}",
        app.notices
    );
    assert_eq!(started.borrow().len(), 1);
}

#[test]
fn fresh_left_press_on_stop_and_keys_help_use_the_same_action() {
    let (_root, mut app, _reader) = fixture("menu-item-mouse");
    keyed(&mut app, b"m", Instant::now());
    let screen = shown(&mut app, Instant::now());
    let (row, column) = screen
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            line.find("s  Stop")
                .map(|byte| (row, line[..byte].chars().count()))
        })
        .expect("drawn Stop item");
    let click = Mouse {
        kind: MouseKind::Click,
        column: u16::try_from(column).expect("painted column"),
        row: u16::try_from(row).expect("painted row"),
    };
    assert!(take_keys(&mut app, vec![(Key::Mouse(click), Instant::now())]).is_some());
    assert!(noticed(&app, "names no tmux server"), "{:?}", app.notices);
    app.open_settings();
    app.model.show_settings_tab(SettingsTab::Keys);
    assert!(framed(&mut app).lines().any(|line| {
        let text = line.trim_matches([' ', '│']);
        text.starts_with("m ") && text.contains("menu")
    }));
}

fn captured_question(app: &mut App, deadline: i64) -> crate::lifecycle::StopExpectation {
    use crate::app::menu::{Choice, Modal, Question};
    if app.modal.is_none() {
        keyed(app, b"m", Instant::now());
    }
    let card = match app.modal.take().expect("actual painted menu") {
        Modal::Menu(menu) => menu.card,
        Modal::Question(_) | Modal::Records(_) => {
            panic!("m unexpectedly constructed a question")
        }
    };
    let expect = crate::lifecycle::StopExpectation {
        session_id: "$7".to_owned(),
        uuid: ID.to_owned(),
        server_pid: "401".to_owned(),
        server_start: "1759500000".to_owned(),
        deadline,
        client: "/dev/ttysfixture".to_owned(),
        client_pid: "402".to_owned(),
        server: crate::inventory::ServerId::from_typed_flags("socket", "/fixture/ae.sock")
            .expect("captured server"),
        require_meta_agent: false,
    };
    app.modal = Some(Modal::Question(Question {
        card,
        expect: expect.clone(),
        focus: Choice::Cancel,
        shown_at: None,
    }));
    expect
}

#[test]
fn question_requires_its_own_visible_frame_and_preserves_the_whole_capture() {
    use crate::app::menu::{Choice, Modal};
    let (_root, mut app, _reader) = fixture("question-origin");
    let expect = captured_question(&mut app, Timestamp::now().epoch() + 120);
    let before_draw = Instant::now();
    keyed(&mut app, b"sy", before_draw);
    assert!(take_keys(&mut app, vec![(Key::Enter, before_draw)]).is_some());
    let first = Instant::now();
    assert!(shown(&mut app, first).contains("Cancel"));
    keyed(&mut app, b"sy", first);
    if let Some(Modal::Question(question)) = &app.modal {
        assert_eq!(question.focus, Choice::Cancel);
        assert_eq!(question.expect, expect);
        assert_eq!(question.shown_at, Some(first));
    } else {
        panic!("blind/equal input acted on the question");
    }
    let fresh = first + Duration::from_millis(1);
    shown(&mut app, first + Duration::from_millis(2));
    keyed(&mut app, b"s", fresh);
    if let Some(Modal::Question(question)) = &app.modal {
        assert_eq!(question.focus, Choice::Stop);
        assert_eq!(question.expect, expect);
        assert_eq!(question.shown_at, Some(first));
    } else {
        panic!("fresh focus selection dismissed question");
    }
    assert!(app.notices.is_empty() && app.queued.is_empty());
}

#[test]
fn affirm_does_not_refresh_an_expired_question_deadline() {
    let (_root, mut app, _reader) = fixture("question-expired");
    captured_question(&mut app, Timestamp::now().epoch() - 1);
    shown(&mut app, Instant::now());
    assert!(take_keys(&mut app, vec![(Key::Enter, Instant::now())]).is_some());
    assert!(app.modal.is_none() && app.notices.is_empty() && app.queued.is_empty());
    captured_question(&mut app, Timestamp::now().epoch() - 1);
    shown(&mut app, Instant::now());
    keyed(&mut app, b"y", Instant::now());
    assert!(
        app.notices
            .iter()
            .any(|(_, item)| { item.body.contains("expired") || item.body.contains("stale") })
    );
    assert!(!noticed(&app, "names no tmux server"));
    assert!(app.queued.is_empty());
}

#[test]
fn unfit_modal_closes_without_leaking_already_read_confirm_into_the_draft() {
    for (question, painted) in [(false, true), (true, false), (true, true)] {
        let (_root, mut app, _reader) = fixture(if question {
            "question-unfit"
        } else {
            "menu-unfit"
        });
        let at = writing(&mut app);
        app.compose(Key::Text("keep 界 draft".as_bytes().to_vec()), at)
            .expect("draft");
        app.compose(Key::Left, Instant::now()).expect("cursor");
        let draft = drafted(&app);
        let size = crate::console::input::Size {
            width: 120,
            height: 3,
        };
        let cursor = app.shown_input().expect("input").bare_view(size);
        right(&mut app, "api", Instant::now());
        if question {
            // Supply captured external facts; exercise the real question reducer/renderer.
            captured_question(&mut app, Timestamp::now().epoch() + 120);
        }
        if painted {
            shown(&mut app, Instant::now());
        }
        let pending = Instant::now();
        let mut buf = Buffer::empty(Rect::new(0, 0, 32, 8));
        app.frame(&mut buf);
        app.frame_shown(Instant::now());
        assert!(app.modal.is_none(), "unfit modal retained authority");
        app.deferred = vec![(Key::Enter, pending), (Key::Text(b"sy".to_vec()), pending)];
        let (_wake, wakes) = std::sync::mpsc::channel();
        let mut first = None;
        let mut decoder = crate::console::input::Keys::app();
        assert!(super::super::drain(&mut app, &mut decoder, &wakes, &mut first).is_some());
        assert_eq!(drafted(&app), draft);
        assert_eq!(app.shown_input().expect("input").bare_view(size), cursor);
        assert!(app.composing() && app.queued.is_empty());
        keyed(&mut app, b"z", Instant::now());
        assert!(drafted(&app).contains('z'), "fresh read remained poisoned");
    }
}

// ---- Stop and Start outside tmux: no client, so the app's own modal is the
// confirmation and the session's recorded server is the proof -----------------

use std::cell::RefCell;

/// Every argv the app detached, in order; the argv never starts a process.
fn detached(app: &mut App) -> Rc<RefCell<Vec<Vec<String>>>> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let keep = Rc::clone(&seen);
    app.detach = Box::new(move |argv| {
        keep.borrow_mut().push(argv.as_args().to_vec());
        true
    });
    seen
}

/// `root`'s home `api` recorded on tmux server `server`, as a launch records it.
fn recorded_on(root: &Root, server: &str) {
    let path = root.0.join("sessions/api/meta");
    let bytes = crate::meta::read_bytes(&root.0.join("sessions/api")).expect("record");
    let meta = String::from_utf8(bytes).expect("UTF-8 fixture meta");
    let mut kept = String::new();
    for line in meta.lines().filter(|line| !line.starts_with("tmux_server")) {
        kept.push_str(line);
        kept.push('\n');
    }
    let rows = if server.is_empty() {
        String::new()
    } else {
        format!("tmux_server_kind=name\ntmux_server={server}\n")
    };
    std::fs::write(&path, format!("{kept}{rows}")).expect("record");
}

/// The Stop question as `ae app` outside tmux freezes it: nobody attached to
/// answer, so the client and its pid are empty and the rest is the record's.
fn unattended_question(app: &mut App, deadline: i64) {
    use crate::app::menu::{Choice, Modal, Question};
    keyed(app, b"m", Instant::now());
    let card = match app.modal.take().expect("actual painted menu") {
        Modal::Menu(menu) => menu.card,
        Modal::Question(_) | Modal::Records(_) => {
            panic!("m unexpectedly constructed a question")
        }
    };
    let expect = crate::lifecycle::StopExpectation {
        session_id: "$7".to_owned(),
        uuid: ID.to_owned(),
        server_pid: "401".to_owned(),
        server_start: "1759500000".to_owned(),
        deadline,
        client: String::new(),
        client_pid: String::new(),
        server: crate::inventory::ServerId::from_typed_flags("name", "fixture")
            .expect("captured server"),
        require_meta_agent: false,
    };
    app.modal = Some(Modal::Question(Question {
        card,
        expect,
        focus: Choice::Cancel,
        shown_at: None,
    }));
}

fn affirmed(app: &mut App) {
    shown(app, Instant::now());
    keyed(app, b"y", Instant::now());
}

#[test]
fn start_outside_tmux_resumes_the_proven_incarnation_without_attaching() {
    let (root, mut app, reader) = fixture("start-unattended");
    refold(&mut app, &reader, &root, &[("api", Status::Stopped)]);
    app.facts
        .insert("api".to_owned(), fleet::Facts::NotPublished);
    framed(&mut app);
    let started = detached(&mut app);
    right(&mut app, "api", Instant::now());
    shown(&mut app, Instant::now());
    keyed(&mut app, b"s", Instant::now());
    assert!(
        !noticed(&app, "not running inside tmux"),
        "{:?}",
        app.notices
    );
    assert!(noticed(&app, "Starting 'api'."), "{:?}", app.notices);
    // Spelled out, not asked of the minter for the session: exactly the
    // executable (the one `lifecycle`'s own test pins), the session as ONE
    // element and `--no-attach`, so nothing attaches and nothing else rides.
    let own = crate::lifecycle::resume_argv("any").expect("own executable");
    let want = vec![
        own.as_args()[0].clone(),
        "api".to_owned(),
        "--no-attach".to_owned(),
    ];
    assert_eq!(*started.borrow(), vec![want]);
}

#[test]
fn start_outside_tmux_refuses_an_incarnation_that_replaced_the_drawn_one() {
    let (root, mut app, reader) = fixture("start-replaced");
    refold(&mut app, &reader, &root, &[("api", Status::Stopped)]);
    app.facts
        .insert("api".to_owned(), fleet::Facts::NotPublished);
    framed(&mut app);
    let started = detached(&mut app);
    right(&mut app, "api", Instant::now());
    shown(&mut app, Instant::now());
    meta(&root, &ID.replace("1234", "bbbb"), "colead");
    keyed(&mut app, b"s", Instant::now());
    assert!(noticed(&app, "replaced or changed"), "{:?}", app.notices);
    assert!(noticed(&app, "nothing started"), "{:?}", app.notices);
    assert!(
        started.borrow().is_empty(),
        "a replaced session was started"
    );
}

#[test]
fn stop_outside_tmux_names_the_missing_record_not_a_missing_tmux() {
    let (_root, mut app, _reader) = fixture("stop-unrecorded");
    let started = detached(&mut app);
    keyed(&mut app, b"m", Instant::now());
    shown(&mut app, Instant::now());
    keyed(&mut app, b"s", Instant::now());
    assert!(
        !noticed(&app, "not running inside tmux"),
        "{:?}",
        app.notices
    );
    assert!(noticed(&app, "names no tmux server"), "{:?}", app.notices);
    assert!(noticed(&app, "nothing stopped"), "{:?}", app.notices);
    assert!(started.borrow().is_empty());
    assert!(app.modal.is_none(), "no question was asked");
}

#[test]
fn affirming_an_unattended_stop_hands_the_supervisor_the_identity_it_froze() {
    let (root, mut app, _reader) = fixture("stop-unattended");
    app.root = Some(root.0.clone());
    recorded_on(&root, "fixture");
    let started = detached(&mut app);
    unattended_question(&mut app, Timestamp::now().epoch() + 120);
    affirmed(&mut app);
    assert!(noticed(&app, "Stopping 'api'."), "{:?}", app.notices);
    assert!(!noticed(&app, "not running inside tmux"));
    let argv = started.borrow();
    assert_eq!(argv.len(), 1, "one supervisor");
    let words = &argv[0];
    let after = |flag: &str| {
        let at = words.iter().position(|word| word == flag).expect(flag);
        words[at + 1].clone()
    };
    assert_eq!(&words[1..4], ["_stop", "--supervise", "api"]);
    assert_eq!(after("--expect-client"), "");
    assert_eq!(after("--expect-client-pid"), "");
    assert_eq!(after("--expect-server-kind"), "name");
    assert_eq!(after("--expect-server"), "fixture");
    assert_eq!(after("--expect-uuid"), ID);
}

#[test]
fn an_unattended_stop_is_refused_when_what_it_froze_no_longer_holds() {
    for (tag, deadline, why) in [
        ("elsewhere", 120, "different tmux server"),
        ("replaced", 120, "no longer names the session_id"),
        ("expired", -1, "expired"),
        ("unrecorded", 120, "names no tmux server"),
    ] {
        let (root, mut app, _reader) = fixture(&format!("stop-unattended-{tag}"));
        app.root = Some(root.0.clone());
        recorded_on(&root, "fixture");
        let started = detached(&mut app);
        unattended_question(&mut app, Timestamp::now().epoch() + deadline);
        match tag {
            "elsewhere" => recorded_on(&root, "other"),
            "replaced" => meta(&root, &ID.replace("1234", "bbbb"), "colead"),
            "unrecorded" => recorded_on(&root, ""),
            _ => {}
        }
        affirmed(&mut app);
        assert!(noticed(&app, why), "{tag}: {:?}", app.notices);
        assert!(noticed(&app, "nothing stopped"), "{tag}: {:?}", app.notices);
        assert!(started.borrow().is_empty(), "{tag}: a supervisor started");
    }
}
