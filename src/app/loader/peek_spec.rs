//! Frozen App/Reader spec: S2-S5, B1-B2, I1-I3 and D-B2/D-B3/D-I4/D-I6/D-I7.
//! Child of loader: observes Reader and ancestor App without new visibility.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use super::{Answer, Job, PeekAsk, Reader, Request};
use crate::app::fleet::{Counts, Fleet, Line2, Row};
use crate::app::model::Key;
use crate::app::peek::{Body, PEEK_EVERY};
use crate::app::{App, Drawn, take_keys};
use crate::console::input::{self, Mouse, MouseKind};
use crate::console::needs::SeatRef;
use crate::digest::{SessionEntry, Status};
use crate::inventory::ServerId;
use crate::theme::Mark;

fn framed(app: &mut App) -> String {
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
    app.frame(&mut buf);
    (0..45)
        .map(|y| (0..160).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn sample_ask() -> Box<PeekAsk> {
    Box::new(PeekAsk {
        generation: 1,
        session: "aedev".to_owned(),
        uuid: "0199c0de-cccc-4890-abcd-ef0123456789".to_owned(),
        dir: PathBuf::from("/tmp/aedev"),
        seat: SeatRef {
            slot: "worker.0".to_owned(),
            name: "colead".to_owned(),
        },
        server: ServerId::Ambient,
    })
}

fn answer(app: &mut App, generation: u64, body: Body) {
    app.answer(Answer::Peek {
        generation,
        at: Instant::now(),
        body,
    });
}

#[test]
fn test_s2_paused_answer_held_and_refusal_adopts_immediately() {
    let mut app = App::new(Some("aedev".to_owned()), None, None, None);
    app.fleeted = true;
    app.model.open_peek(1);
    let initial = (0..70).map(|i| format!("INITIAL ROW {i}")).collect();
    answer(&mut app, 1, Body::Lines(initial));
    assert!(framed(&mut app).contains("INITIAL ROW 69"));
    let _ = app.model.key(Key::Up, &app.fleet, false, false);
    let before = framed(&mut app);
    assert_eq!(app.model.peek_back(), Some(1), "real paused viewport");
    answer(&mut app, 1, Body::Lines(vec!["NEWER ROW".to_owned(); 70]));
    let paused = framed(&mut app);
    // Status may truthfully add "newer waiting"; every body cell stays fixed.
    assert_eq!(
        before.lines().skip(3).take(41).collect::<Vec<_>>(),
        paused.lines().skip(3).take(41).collect::<Vec<_>>(),
        "paused body changes no painted cell"
    );
    assert!(paused.contains("paused") && !paused.contains("NEWER ROW"));
    let _ = take_keys(&mut app, vec![(input::Key::Down, Instant::now())]);
    assert_eq!(app.model.peek_back(), Some(0), "back to live");
    assert!(framed(&mut app).contains("NEWER ROW"), "latest adopted");
    let _ = take_keys(&mut app, vec![(input::Key::Up, Instant::now())]);
    assert_eq!(app.model.peek_back(), Some(1));
    answer(
        &mut app,
        1,
        Body::Gap("refused: peek colead: its pane is dead".to_owned()),
    );
    assert_eq!(app.model.peek_back(), Some(0), "refusal resets back");
    let refused = framed(&mut app);
    assert!(refused.contains("refused: peek colead: its pane is dead"));
    assert!(!refused.contains("INITIAL ROW") && !refused.contains("NEWER ROW"));
}

#[test]
fn test_s3_stale_generation_dropped_and_close_sends_none() {
    let (ask_tx, ask_rx) = mpsc::channel();
    let mut app = App::new(Some("aedev".to_owned()), None, None, Some(ask_tx));
    app.fleeted = true;
    app.model.open_peek(2);
    answer(
        &mut app,
        2,
        Body::Lines(vec!["CURRENT GENERATION".to_owned()]),
    );
    assert!(framed(&mut app).contains("CURRENT GENERATION"));
    answer(
        &mut app,
        1,
        Body::Lines(vec!["STALE GENERATION".to_owned()]),
    );
    let screen = framed(&mut app);
    assert!(screen.contains("CURRENT GENERATION") && !screen.contains("STALE GENERATION"));
    for (generation, key) in [
        (3, input::Key::Escape),
        (4, input::Key::Text(vec![b'q'])),
        (5, input::Key::Text(vec![b'v'])),
    ] {
        app.model.open_peek(generation);
        while ask_rx.try_recv().is_ok() {}
        let _ = take_keys(&mut app, vec![(key, Instant::now())]);
        assert!(!app.model.peek_open(), "close generation {generation}");
        assert_eq!(
            ask_rx
                .try_iter()
                .filter(|r| matches!(r, Request::Peek(None)))
                .count(),
            1,
            "close generation {generation} sends exactly one Peek(None)"
        );
    }
}

#[test]
fn test_s4_pre_panel_refusal_sends_no_request() {
    for why in [
        "no seat",
        "stopped",
        "not running inside tmux",
        "no record directory",
    ] {
        let (ask_tx, ask_rx) = mpsc::channel();
        let mut app = App::new(
            Some("aedev".to_owned()),
            (why != "not running inside tmux").then_some(ServerId::Ambient),
            Some("%999".to_owned()),
            Some(ask_tx),
        );
        app.fleeted = true;
        let status = if why == "stopped" {
            Status::Stopped
        } else {
            Status::Running
        };
        app.world.sessions.push(SessionEntry::new("aedev", status));
        app.fleet = Fleet {
            rows: vec![Row {
                name: "aedev".to_owned(),
                index: 1,
                mark: Mark::Working,
                needy: false,
                counts: Counts::Unknown,
                line2: Line2::NoGoal,
                home: true,
            }],
            home: Some("aedev".to_owned()),
        };
        app.model = crate::app::model::Model::new(&app.fleet);
        let ask = sample_ask();
        app.ids.insert(ask.session.clone(), ask.uuid.clone());
        if why != "no record directory" {
            app.dirs.insert(ask.session.clone(), ask.dir.clone());
        }
        app.drawn = Some(Drawn {
            session: ask.session.clone(),
            uuid: ask.uuid.clone(),
            rows: vec![(ask.seat.name.clone(), Some(ask.seat.clone()))],
            focused: (why != "no seat").then_some(ask.seat.name.clone()),
        });
        while ask_rx.try_recv().is_ok() {}
        let _ = take_keys(
            &mut app,
            vec![(input::Key::Text(vec![b'v']), Instant::now())],
        );
        assert!(!app.model.peek_open(), "{why}: no panel");
        assert!(ask_rx.try_recv().is_err(), "{why}: no Request");
        let screen = framed(&mut app);
        assert!(
            screen.contains("peek") && screen.contains(why),
            "{why}: named flash: {screen}"
        );
    }
}

#[test]
fn test_s5_reader_cadence_and_no_duplicate_queue() {
    let mut reader = Reader::new(PathBuf::from("/tmp/ae-reader-cadence"), None, None);
    let t0 = Instant::now();
    reader.take(Request::Peek(Some(sample_ask())));
    assert!(matches!(reader.next(t0), Job::Fleet), "initial fleet first");
    reader.fleet_at = Some(t0);
    assert!(matches!(reader.next(t0), Job::Peek(_)), "first peek due");
    reader.peek = Some((sample_ask(), Some(t0)));
    for millis in [0, 1000, 1999] {
        let job = reader.next(t0 + Duration::from_millis(millis));
        assert!(
            matches!(job, Job::Wait(until) if until == t0 + PEEK_EVERY),
            "no early or duplicate peek at {millis}ms"
        );
    }
    assert!(
        matches!(reader.next(t0 + PEEK_EVERY), Job::Peek(_)),
        "due at 2s"
    );
    reader.take(Request::Peek(None));
    assert!(
        !matches!(reader.next(t0 + Duration::from_secs(10)), Job::Peek(_)),
        "closed clears job"
    );
}

#[test]
fn test_keys_row_click_close_and_app_wheel() {
    let (ask_tx, ask_rx) = mpsc::channel();
    let mut app = App::new(Some("aedev".to_owned()), None, None, Some(ask_tx));
    app.model.open_peek(1);
    answer(&mut app, 1, Body::Lines(vec!["WHEEL ROW".to_owned(); 70]));
    let screen = framed(&mut app);
    let bottom = screen.lines().nth(44).unwrap_or_default();
    assert!(
        bottom.contains("peek") && bottom.contains("Esc close"),
        "peek keys: {bottom}"
    );
    assert!(
        bottom.contains(&crate::version_line()),
        "version kept: {bottom}"
    );
    assert!(
        !bottom.contains('⚙') && !bottom.contains('*'),
        "gear hidden: {bottom}"
    );
    let wheeled = app.mouse(Mouse {
        kind: MouseKind::WheelUp,
        column: 10,
        row: 10,
    });
    assert!(
        wheeled && app.model.peek_back() == Some(3),
        "App wheel older = 3 rows"
    );
    let wheeled = app.mouse(Mouse {
        kind: MouseKind::WheelDown,
        column: 10,
        row: 10,
    });
    assert!(
        wheeled && app.model.peek_back() == Some(0),
        "App wheel newer = 3 rows"
    );
    // Click the title label actually recorded by the panel, rather than dead hint text.
    let title = screen.lines().next().unwrap_or_default();
    let byte = title.find("Esc close").expect("drawn title close label");
    let col = u16::try_from(title[..byte].chars().count()).expect("title column fits");
    while ask_rx.try_recv().is_ok() {}
    assert!(app.mouse(Mouse {
        kind: MouseKind::Click,
        column: col,
        row: 0
    }));
    assert!(!app.model.peek_open(), "drawn close click closes");
    assert_eq!(
        ask_rx
            .try_iter()
            .filter(|r| matches!(r, Request::Peek(None)))
            .count(),
        1,
        "click sends one close"
    );
}
