//! Frozen app acceptance from R1-R7 + lead GO 2026-10-12 15:09Z.
#![allow(clippy::expect_used, clippy::disallowed_methods, reason = "fixture")]
use super::{
    App, fleet,
    loader::{Answer, ViewRead},
    model, take_keys,
    tests::{ID, Root},
};
use crate::{
    console::{
        self,
        input::{Key, Mouse, MouseKind},
        lane::{Item, Kind, Lane},
        submit::Outcome,
    },
    digest::{SessionEntry, Status},
    events::Event,
    theme::Mark,
};
use ratatui_core::{buffer::Buffer, layout::Rect};
use std::{rc::Rc, time::Instant};

fn turn(body: &str, record: usize) -> Item {
    Item {
        micros: 0,
        kind: Kind::Said { who: "lead".into() },
        body: body.into(),
        record: Some(record),
    }
}
fn lines(n: usize) -> String {
    (1..=n)
        .map(|n| format!("line-{n:02}"))
        .collect::<Vec<_>>()
        .join("\n")
}
fn lane(items: Vec<Item>) -> Rc<Lane> {
    Rc::new(Lane {
        items,
        coverage: vec![],
    })
}
fn app(body: &str) -> App {
    let mut app = App::new(None, None, None, None);
    app.fleet = fleet::Fleet {
        home: None,
        rows: ["api", "two"]
            .into_iter()
            .enumerate()
            .map(|(i, n)| fleet::Row {
                name: n.into(),
                index: i + 1,
                mark: Mark::Working,
                needy: false,
                counts: fleet::Counts::Unknown,
                line2: fleet::Line2::NoGoal,
                home: false,
            })
            .collect(),
    };
    app.model = model::Model::new(&app.fleet);
    app.world.sessions = ["api", "two"]
        .into_iter()
        .map(|n| SessionEntry::new(n, Status::Stopped))
        .collect();
    app.ids.insert("api".into(), ID.into());
    app.ids.insert("two".into(), ID.into());
    (app.fleeted, app.loading) = (true, false);
    app.lane = lane(vec![turn(body, 0)]);
    app
}
fn frame(app: &mut App) -> (Buffer, String) {
    let mut buf = Buffer::empty(Rect::new(0, 0, 160, 45));
    app.frame(&mut buf);
    let text = (3..38)
        .map(|y| {
            (47..158)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n");
    (buf, text)
}
fn press(app: &mut App, key: Key) {
    assert!(take_keys(app, vec![(key, Instant::now())]).is_some());
}
fn click(app: &mut App, needle: &str) {
    let (buf, _) = frame(app);
    let row = (3..38)
        .find(|&y| {
            (47..158)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .contains(needle)
        })
        .expect("fold row visible");
    press(
        app,
        Key::Mouse(Mouse {
            kind: MouseKind::Click,
            column: 49,
            row,
        }),
    );
}

#[test]
fn app_shows_ten_rows_whole_and_eleven_as_first_eight_plus_three_more() {
    for n in [8, 9, 10, 11, 12] {
        let (_, shown) = frame(&mut app(&lines(n)));
        assert!(shown.contains("line-08"));
        assert_eq!(shown.contains("line-09"), (9..=10).contains(&n));
        assert_eq!(shown.contains(&format!("... {} more lines", n - 8)), n > 10);
    }
    let body = "x".repeat(108 * 11);
    let (_, shown) = frame(&mut app(&body));
    assert!(
        shown.contains("... 3 more lines"),
        "R2 rendered wrap rows: {shown}"
    );
}

#[test]
fn short_app_preserves_blank_rows_tab_spacing_and_trailing_newline() {
    let (_, shown) = frame(&mut app("one\n\n\ttwo\n"));
    assert!(shown.contains("  one\n\n       two\n"));
}

#[test]
fn click_and_marked_enter_toggle_and_keep_state_through_reread_and_new_turns() {
    let mut app = app(&lines(12));
    click(&mut app, "... 4 more lines");
    assert!(frame(&mut app).1.contains("line-12"));
    app.lane = lane(vec![turn(&lines(13), 0), turn("new turn", 1)]);
    assert!(frame(&mut app).1.contains("line-13"));
    press(&mut app, Key::Text(b"[".to_vec()));
    press(&mut app, Key::Text(b"[".to_vec()));
    let marked = app.model.turn();
    press(&mut app, Key::Enter);
    assert_eq!(app.model.turn(), marked, "toggle preserves mark");
    assert!(!frame(&mut app).1.contains("line-09"));
    press(&mut app, Key::Enter);
    assert!(frame(&mut app).1.contains("line-13"));
    press(&mut app, Key::Text(b"2".to_vec()));
    app.lane = lane(vec![turn(&lines(13), 0)]);
    assert!(!frame(&mut app).1.contains("line-09"));
    app.ids.insert("two".into(), ID.replace("1234", "bbbb"));
    click(&mut app, "... 5 more lines");
    app.ids.insert("two".into(), ID.into());
    assert!(!frame(&mut app).1.contains("line-09"));
    assert!(!frame(&mut self::app(&lines(13))).1.contains("line-09"));
}

#[test]
fn a_toggle_preserves_a_composed_draft_and_enter_submits_it() {
    let root = Root::new("longpaste-draft");
    let mut app = app(&lines(12));
    app.world.sessions[0].status = Status::Running;
    app.dirs.insert("api".into(), root.0.join("sessions/api"));
    app.pairs.insert(
        "api".into(),
        Ok(vec![console::lane::Seat {
            slot: "main".into(),
            name: "lead".into(),
            profile: None,
        }]),
    );
    app.answer(Answer::View(ViewRead {
        name: "api".into(),
        id: ID.into(),
        seq: 1,
        lane: Lane {
            items: vec![turn(&lines(12), 0)],
            coverage: vec![],
        },
        needs: None,
        roster: None,
    }));
    app.read_only.clear();
    let _ = frame(&mut app);
    press(&mut app, Key::Text(b"[".to_vec()));
    app.write(true);
    assert!(app.composing());
    press(&mut app, Key::Text(b"keep this draft".to_vec()));
    click(&mut app, "... 4 more lines");
    assert_eq!(
        app.shown_input().expect("composer").draft(),
        "keep this draft"
    );
    assert!(app.queued.is_empty(), "click writes nothing");
    press(&mut app, Key::Enter);
    assert_eq!(app.queued.len(), 1, "Enter submits draft before any toggle");
    assert_eq!(app.queued[0].body, "keep this draft");
    assert!(frame(&mut app).1.contains("line-12"));
}

#[test]
fn successful_outcomes_live_in_the_ask_header_and_failures_keep_visible_lines() {
    let ask = Event::parse_line(r#"{"ts":"1970-01-01T00:00:00Z","actor":"console:local","action":"ask","target":"lead","ref":"q","summary":"asked body"}"#).expect("ask");
    let lane = console::lane::fold(
        "api",
        &[],
        &[ask],
        &crate::board::Observation::default(),
        0,
        &|_| console::lane::Body::Whole("asked body".into()),
    );
    for kept in [None, Some("the draft is kept (fixture)".to_owned())] {
        let mut app = app("unused");
        app.dirs.insert("api".into(), "fixture".into());
        app.answer(Answer::View(ViewRead {
            name: "api".into(),
            id: ID.into(),
            seq: 1,
            lane: lane.clone(),
            needs: None,
            roster: None,
        }));
        let notices = app.settle(
            "q".into(),
            "lead",
            "api",
            Outcome::Sent("q".into(), kept.clone()),
        );
        for notice in notices {
            app.notice("api", notice);
        }
        app.show();
        let (_, text) = frame(&mut app);
        assert!(text.contains("to lead · sent"), "R7 header: {text}");
        assert_eq!(app.notices.len(), usize::from(kept.is_some()));
        if kept.is_some() {
            assert!(text.contains("the draft is kept (fixture)"));
        }
        assert!(
            app.settle(
                "bad".into(),
                "lead",
                "api",
                Outcome::Unknown("bad".into(), "why".into())
            )[0]
            .contains("no record")
        );
        for outcome in [
            Outcome::Uncertain("bad".into()),
            Outcome::NotDelivered("bad".into()),
        ] {
            let _ = app.settle("bad".into(), "lead", "api", outcome);
            assert!(app.note().is_some(), "R7 alert survives");
        }
    }
}

#[test]
fn decision_options_are_split_before_folding_and_full_toggle_keeps_last_option() {
    let body = (1..=12)
        .map(|n| format!("choice-{n:02}"))
        .collect::<Vec<_>>()
        .join(" | ");
    let mut app = app("unused");
    let mut card = turn(&body, 0);
    card.kind = Kind::Card {
        seat: "lead".into(),
    };
    app.lane = lane(vec![card]);
    let shown = frame(&mut app).1;
    assert!(shown.contains("choice-08") && !shown.contains("choice-09"));
    click(&mut app, "... 4 more lines");
    assert!(frame(&mut app).1.contains("choice-12"));
}
