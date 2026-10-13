//! Frozen livepeek P1: brief-livepeek.md R1-R5, lead copy ruling 061139Z.
//! Real App frame/input path; loader cadence pinned in `loader/peek_spec.rs`.
#![allow(
    clippy::expect_used,
    clippy::disallowed_methods,
    reason = "owned spec fixtures"
)]

use super::tests::{ID, Root, session};
use super::*;
use ratatui_core::layout::Rect;

fn sample() -> (Root, App, Receiver<Request>) {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tag = format!(
        "livepeek-{}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let root = Root::new(&tag);
    let (tx, rx) = mpsc::channel();
    let mut app = App::new(Some("api".into()), Some(ServerId::Ambient), None, Some(tx));
    app.fleeted = true;
    app.loading = false;
    app.fleet = fleet::Fleet {
        home: Some("api".into()),
        rows: Vec::new(),
    };
    for (at, name) in ["api", "web"].into_iter().enumerate() {
        let dir = session(&root, name, "");
        std::fs::write(dir.join("events.jsonl"), "").expect("owned journal");
        app.dirs.insert(name.into(), dir);
        app.ids.insert(name.into(), ID.into());
        app.world
            .sessions
            .push(SessionEntry::new(name, Status::Running));
        app.fleet.rows.push(fleet::Row {
            name: name.into(),
            index: at + 1,
            mark: theme::Mark::Working,
            needy: false,
            counts: fleet::Counts::Unknown,
            line2: fleet::Line2::NoGoal,
            home: at == 0,
        });
        let seats: Vec<SeatRef> = [("main", "lead"), ("worker.0", "colead")]
            .into_iter()
            .map(|(slot, name)| SeatRef {
                slot: slot.into(),
                name: name.into(),
            })
            .collect();
        app.facts.insert(
            name.into(),
            fleet::Facts::Seats {
                id: "$1".into(),
                agents: seats
                    .iter()
                    .map(|seat| tmux::PickerAgent {
                        name: seat.name.clone(),
                        profile: "sol61x".into(),
                        state: "working".into(),
                        pane: "%7".into(),
                        client: String::new(),
                        model: String::new(),
                        effort: String::new(),
                        drift: false,
                    })
                    .collect(),
            },
        );
        let lane = Rc::new(Lane {
            coverage: Vec::new(),
            items: vec![Item {
                micros: 1,
                kind: Kind::Said { who: "lead".into() },
                body: format!("CHAT-{name}"),
                record: None,
            }],
        });
        app.shown.insert(
            name.into(),
            Shown {
                id: ID.into(),
                seq: 1,
                lane,
                roster: Some(seats),
            },
        );
    }
    app.model = model::Model::new(&app.fleet);
    app.model.set_focus(Some("colead".into()));
    app.show();
    (root, app, rx)
}

fn frame(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
    app.frame(&mut buf);
    buf
}

fn text(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn cell(app: &App, buf: &Buffer, label: &str, seat: bool) -> Mouse {
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            let at = Mouse {
                kind: MouseKind::Click,
                column: x,
                row: y,
            };
            let correct = match app.layout.hit(at) {
                Some(draw::Hit::Seat(name)) if seat => name == label,
                Some(draw::Hit::Session(name)) if !seat => name == label,
                _ => false,
            };
            let drawn: String = (x..buf.area.width).map(|c| buf[(c, y)].symbol()).collect();
            if correct && drawn.starts_with(label) {
                return at;
            }
        }
    }
    panic!("drawn clickable label {label} seat={seat}");
}

fn key(app: &mut App, key: Key) {
    assert!(
        take_keys(app, vec![(key, Instant::now())]).is_some(),
        "input keeps app alive"
    );
}

fn ask(rx: &Receiver<Request>) -> Box<PeekAsk> {
    rx.try_iter()
        .find_map(|r| {
            if let Request::Peek(Some(ask)) = r {
                Some(ask)
            } else {
                None
            }
        })
        .expect("open asks loader to capture")
}

fn capture(app: &mut App, generation: u64, rows: &[&str]) {
    app.answer(Answer::Peek {
        generation,
        at: Instant::now(),
        body: Body::Lines(rows.iter().map(|s| (*s).into()).collect()),
    });
}

fn opened(app: &mut App, rx: &Receiver<Request>) -> (Buffer, u64, u16) {
    let before = frame(app, 160, 45);
    let left = app
        .layout
        .map
        .regions
        .iter()
        .find(|r| r.panel == select::Panel::Lane)
        .expect("chat column exists")
        .area
        .x;
    let at = cell(app, &before, "colead", true);
    assert!(app.mouse(at));
    assert!(app.model.peek_open(), "agent click opens seat view");
    let requested = ask(rx);
    assert_eq!(
        (requested.session.as_str(), requested.uuid.as_str()),
        ("api", ID)
    );
    assert_eq!(
        (requested.seat.slot.as_str(), requested.seat.name.as_str()),
        ("worker.0", "colead")
    );
    (before, requested.generation, left)
}

#[test]
fn livepeek_agent_click_replaces_only_chat_and_keeps_drawn_sidebar_targets() {
    let (_root, mut app, rx) = sample();
    let (before, generation, left) = opened(&mut app, &rx);
    capture(&mut app, generation, &["VIEW-colead"]);
    let after = frame(&mut app, 160, 45);
    for y in 0..44 {
        for x in 0..left.saturating_sub(1) {
            assert_eq!(
                before[(x, y)].symbol(),
                after[(x, y)].symbol(),
                "sidebar cell {x},{y}"
            );
        }
    }
    assert!(text(&after).contains("VIEW-colead") && !text(&after).contains("CHAT-api"));
    let region = app
        .layout
        .map
        .regions
        .iter()
        .find(|r| r.panel == select::Panel::Peek)
        .expect("seat body selectable");
    assert!(
        region.area.x >= left && region.area.right() <= 160,
        "view is inside chat column"
    );
    assert!(
        !app.layout
            .map
            .regions
            .iter()
            .any(|r| r.panel == select::Panel::Lane)
    );
    let _ = cell(&app, &after, "api", false);
    let _ = cell(&app, &after, "web", false);
    let _ = cell(&app, &after, "lead", true);
    assert!(text(&after).contains("captured 0s ago") && text(&after).contains("Esc chat"));
}

#[test]
fn livepeek_session_click_and_escape_return_to_chat_and_cancel_capture_once() {
    for escape in [false, true] {
        let (_root, mut app, rx) = sample();
        let (_, generation, _) = opened(&mut app, &rx);
        capture(&mut app, generation, &["VIEW-colead"]);
        let buf = frame(&mut app, 160, 45);
        if escape {
            key(&mut app, Key::Escape);
        } else {
            app.mouse(cell(&app, &buf, "api", false));
        }
        assert!(!app.model.peek_open(), "session click/Esc closes view");
        assert_eq!(
            rx.try_iter()
                .filter(|r| matches!(r, Request::Peek(None)))
                .count(),
            1
        );
        capture(&mut app, generation, &["LATE-CAPTURE"]);
        let chat = text(&frame(&mut app, 160, 45));
        assert!(chat.contains("CHAT-api") && !chat.contains("LATE-CAPTURE"));
    }
}

#[test]
fn livepeek_switch_session_starts_at_chat_and_never_revives_an_old_capture() {
    let (_root, mut app, rx) = sample();
    let (_, generation, _) = opened(&mut app, &rx);
    let buf = frame(&mut app, 160, 45);
    app.mouse(cell(&app, &buf, "web", false));
    assert_eq!(app.model.selected(), Some("web"));
    assert!(!app.model.peek_open(), "switch starts at chat");
    assert_eq!(
        rx.try_iter()
            .filter(|r| matches!(r, Request::Peek(None)))
            .count(),
        1
    );
    capture(&mut app, generation, &["OLD-API"]);
    let buf = frame(&mut app, 160, 45);
    assert!(text(&buf).contains("CHAT-web") && !text(&buf).contains("OLD-API"));
    app.mouse(cell(&app, &buf, "api", false));
    assert!(!app.model.peek_open() && text(&frame(&mut app, 160, 45)).contains("CHAT-api"));
}

#[test]
fn livepeek_clicking_another_seat_retargets_and_v_uses_the_same_column_surface() {
    let (_root, mut app, rx) = sample();
    let (_, old, left) = opened(&mut app, &rx);
    let buf = frame(&mut app, 160, 45);
    app.mouse(cell(&app, &buf, "lead", true));
    let next = ask(&rx);
    assert_eq!(next.seat.name, "lead");
    assert_ne!(next.generation, old);
    capture(&mut app, next.generation, &["CURRENT-LEAD"]);
    capture(&mut app, old, &["STALE-COLEAD"]);
    let buf = frame(&mut app, 160, 45);
    assert!(text(&buf).contains("CURRENT-LEAD") && !text(&buf).contains("STALE-COLEAD"));
    key(&mut app, Key::Escape);
    let _ = rx.try_iter().count();
    key(&mut app, Key::Text(b"v".to_vec()));
    assert_eq!(ask(&rx).seat.name, "lead");
    let buf = frame(&mut app, 160, 45);
    assert!(
        app.layout
            .map
            .regions
            .iter()
            .any(|r| r.panel == select::Panel::Peek && r.area.x >= left)
    );
    let _ = cell(&app, &buf, "api", false);
}

#[test]
fn livepeek_composer_is_inert_and_preserves_a_draft_from_writing() {
    let (root, mut app, rx) = sample();
    app.pairs.insert(
        "api".into(),
        Ok(vec![
            Seat {
                slot: "main".into(),
                name: "lead".into(),
                profile: None,
            },
            Seat {
                slot: "worker.0".into(),
                name: "colead".into(),
                profile: None,
            },
        ]),
    );
    app.write(true);
    assert!(app.composing(), "fixture actually owns writer lease");
    app.compose(Key::Text(b"KEPT-DRAFT".to_vec()), Instant::now())
        .expect("draft");
    let before = std::fs::read(root.0.join("sessions/api/events.jsonl")).expect("events");
    let _ = rx.try_iter().count();
    let (_, generation, _) = opened(&mut app, &rx);
    capture(&mut app, generation, &["VIEW"]);
    let buf = frame(&mut app, 160, 45);
    assert!(
        !app.composing() && !app.can_compose(),
        "view releases writer and cannot compose"
    );
    let composer: String = text(&buf).lines().rev().skip(1).take(3).collect();
    assert!(
        composer.contains("Esc") && composer.contains("chat"),
        "composer explains return to chat"
    );
    for y in 0..45 {
        for x in 0..160 {
            assert!(!matches!(
                app.layout.hit(Mouse {
                    kind: MouseKind::Click,
                    column: x,
                    row: y
                }),
                Some(draw::Hit::Compose)
            ));
        }
    }
    let _ = rx.try_iter().count();
    for k in [
        Key::Text(b"ihello".to_vec()),
        Key::Pasted(b"payload".to_vec()),
        Key::Enter,
        Key::Newline,
    ] {
        key(&mut app, k);
    }
    assert!(
        app.queued.is_empty() && rx.try_iter().count() == 0,
        "input queues no writes"
    );
    assert_eq!(app.shown_input().expect("kept input").draft(), "KEPT-DRAFT");
    assert_eq!(
        std::fs::read(root.0.join("sessions/api/events.jsonl")).expect("events"),
        before
    );
    key(&mut app, Key::Escape);
    assert!(text(&frame(&mut app, 160, 45)).contains("KEPT-DRAFT"));
    key(&mut app, Key::Enter); // Returning browses; Enter retakes the chat writer lease.
    assert!(
        app.composing(),
        "chat composer retakes its lease after returning"
    );
    key(&mut app, Key::Text(b"X".to_vec()));
    assert_eq!(
        app.shown_input().expect("resumed input").draft(),
        "KEPT-DRAFTX"
    );
    assert_eq!(
        std::fs::read(root.0.join("sessions/api/events.jsonl")).expect("events"),
        before
    );
}

#[test]
fn livepeek_refusal_replaces_capture_in_column_and_copy_notes_remain_visible() {
    let (_root, mut app, rx) = sample();
    let (_, generation, left) = opened(&mut app, &rx);
    app.server = None; // Clipboard path outside tmux is staged, with no terminal I/O here.
    capture(&mut app, generation, &["COPY-ME"]);
    let buf = frame(&mut app, 160, 45);
    key(&mut app, Key::Text(b"y".to_vec()));
    assert_eq!(
        app.flash.as_ref().map(|(s, _)| s.as_str()),
        Some("drag to copy text")
    );
    assert!(app.clip.is_none(), "y writes no clipboard");
    assert!(text(&frame(&mut app, 160, 45)).contains("drag to copy text"));
    let region = app
        .layout
        .map
        .regions
        .iter()
        .find(|r| r.panel == select::Panel::Peek)
        .expect("peek")
        .area;
    let at = (region.y..region.bottom())
        .find_map(|y| {
            (region.x..region.right())
                .find(|x| buf[(*x, y)].symbol() == "C")
                .map(|x| (x, y))
        })
        .expect("drawn copy text");
    app.mouse(Mouse {
        kind: MouseKind::Click,
        column: at.0,
        row: at.1,
    });
    app.mouse(Mouse {
        kind: MouseKind::Drag,
        column: at.0 + 6,
        row: at.1,
    });
    app.mouse(Mouse {
        kind: MouseKind::Release,
        column: at.0 + 6,
        row: at.1,
    });
    let _ = frame(&mut app, 160, 45); // The drawn highlight makes its staged copy Ready.
    assert!(app.shipped(), "drag copy uses existing ship path");
    assert_eq!(app.clip.as_deref(), Some("COPY-ME"));
    let copied = text(&frame(&mut app, 160, 45));
    assert!(
        copied.contains("terminal clipboard"),
        "copy outcome visible in view"
    );
    app.answer(Answer::Peek {
        generation,
        at: Instant::now(),
        body: Body::Gap("refused: peek colead: its pane is dead".into()),
    });
    let refused = frame(&mut app, 160, 45);
    assert!(
        text(&refused).contains("refused: peek colead: its pane is dead")
            && !text(&refused).contains("COPY-ME")
    );
    assert!(
        app.layout
            .map
            .regions
            .iter()
            .any(|r| r.panel == select::Panel::Peek && r.area.x >= left)
    );
    let _ = cell(&app, &refused, "api", false);
}

#[test]
fn livepeek_v_wraps_at_column_width_neutralises_text_and_handles_small_frames() {
    let (_root, mut app, rx) = sample();
    let before = frame(&mut app, 100, 25);
    assert!(text(&before).contains("CHAT-api"));
    let left = app
        .layout
        .map
        .regions
        .iter()
        .find(|r| r.panel == select::Panel::Lane)
        .expect("chat column")
        .area
        .x;
    key(&mut app, Key::Text(b"v".to_vec()));
    let generation = ask(&rx).generation;
    let raw = format!("\x1b[2J\x00{}", "wrapword ".repeat(15));
    capture(&mut app, generation, &[&raw]);
    let buf = frame(&mut app, 100, 25);
    let region = app
        .layout
        .map
        .regions
        .iter()
        .find(|r| r.panel == select::Panel::Peek)
        .expect("view body")
        .area;
    assert!(region.x >= left, "v shares the chat-column surface");
    let rows: Vec<String> = (region.y..region.bottom())
        .map(|y| {
            (region.x..region.right())
                .map(|x| buf[(x, y)].symbol())
                .collect()
        })
        .collect();
    let painted = rows.join("\n");
    assert!(painted.chars().all(|ch| ch == '\n' || !ch.is_control()));
    assert!(painted.contains('\u{fffd}'), "hostile controls neutralised");
    assert_eq!(
        painted.matches("wrapword").count(),
        15,
        "wrap loses no text"
    );
    assert!(
        rows.iter().filter(|s| s.contains("wrapword")).count() > 1,
        "wrap uses column width"
    );
    assert!(
        rows.iter()
            .filter(|s| s.contains("wrapword"))
            .skip(1)
            .all(|s| s.starts_with("  wrapword")),
        "continuations hang two cells"
    );
    for (width, height) in [(90, 20), (89, 19), (40, 8), (35, 6), (0, 0)] {
        let buf = frame(&mut app, width, height);
        assert_eq!(buf.area, Rect::new(0, 0, width, height));
        assert!(
            app.layout
                .map
                .regions
                .iter()
                .all(|r| r.area.intersection(buf.area) == r.area)
        );
    }
}
