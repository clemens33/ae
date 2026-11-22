//! Frozen acceptance for boardfull: the orchestrator board in `F full`,
//! scrolling with clickable links and directional cues, narrow-frame toggle
//! via `B` / `Esc`, and zero-diff behavior when no board is set.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "frozen acceptance owns scratch state roots and terminal buffers"
)]

use std::fmt::Write as _;

use super::*;
use crate::app::{Terminal, backend, draw_frame};
use crate::console::deskview::DeskView;
use crate::console::input::Keys;
use crate::console::link;
use crate::desk::{Desk, parse_input};

pub(super) const ORCH: &str = crate::orchestrator::ORCHESTRATOR_SESSION;
pub(super) const OID: &str = "0199c0de-2222-4890-abcd-ef0123456789";
pub(super) const MAIN_TURN: &str = "BOARDFULL_MAIN_SENTINEL";
pub(super) const PANEL_TURN: &str = "BOARDFULL_PANEL_SENTINEL";

pub(super) fn paint(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut cells = Buffer::empty(Rect::new(0, 0, width, height));
    app.frame(&mut cells);
    app.frame_shown(Instant::now());
    cells
}

pub(super) fn bytes(app: &mut App, value: &[u8]) {
    let at = Instant::now();
    assert!(take_keys(app, Keys::app().feed(value, at)).is_some());
}

pub(super) fn send_key(app: &mut App, key: Key) {
    let at = Instant::now();
    assert!(take_keys(app, vec![(key, at)]).is_some());
}

fn event(app: &mut App, kind: MouseKind, (column, row): (u16, u16)) {
    let at = Instant::now();
    assert!(take_keys(app, vec![(Key::Mouse(Mouse { kind, column, row }), at)]).is_some());
}

fn click(app: &mut App, at: (u16, u16)) {
    event(app, MouseKind::Click, at);
    event(app, MouseKind::Release, at);
}

fn wheel(app: &mut App, up: bool, at: (u16, u16)) {
    let kind = if up {
        MouseKind::WheelUp
    } else {
        MouseKind::WheelDown
    };
    event(app, kind, at);
}

pub(super) fn drag_border(app: &mut App, from: u16, to: u16) {
    for (kind, column) in [
        (MouseKind::Click, from),
        (MouseKind::Drag, to),
        (MouseKind::Release, to),
    ] {
        event(app, kind, (column, 5));
    }
}

pub(super) fn rules_at_bottom(cells: &Buffer) -> Vec<u16> {
    let y = cells.area.height.saturating_sub(1);
    (0..cells.area.width)
        .filter(|&x| cells[(x, y)].symbol() == "│")
        .collect()
}

pub(super) fn locate(cells: &Buffer, needle: &str) -> Option<(u16, u16)> {
    let width = u16::try_from(needle.len()).expect("short needle");
    (0..cells.area.height).find_map(|y| {
        (0..=cells.area.width.saturating_sub(width)).find_map(|x| {
            let candidate: String = (x..x + width).map(|cx| cells[(cx, y)].symbol()).collect();
            (candidate == needle).then_some((x, y))
        })
    })
}

fn scrolled_link_at(
    app: &App,
    cells: &Buffer,
    url: &str,
    expected_text: &str,
    min_x: u16,
) -> (u16, u16) {
    let runs: Vec<&backend::Run> = app
        .layout
        .links
        .iter()
        .filter(|run| run.url == url)
        .collect();
    assert!(!runs.is_empty(), "missing link run for {url}");
    let mut joined = String::new();
    for run in &runs {
        assert!(run.x >= min_x && run.y < cells.area.height && run.width > 0);
        joined.push_str(under(cells, run).trim());
    }
    assert_eq!(joined, expected_text);
    (runs[0].x, runs[0].y)
}

pub(super) fn row_text(cells: &Buffer, y: u16) -> String {
    (0..cells.area.width)
        .map(|x| cells[(x, y)].symbol())
        .collect()
}

fn rect_text(cells: &Buffer, rect: Rect) -> String {
    (rect.y..rect.bottom())
        .map(|y| {
            (rect.x..rect.right())
                .map(|x| cells[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn under(cells: &Buffer, run: &backend::Run) -> String {
    (run.x..run.x + run.width)
        .map(|x| cells[(x, run.y)].symbol())
        .collect()
}

fn seat_record(root: &Root) {
    let dir = root.0.join("sessions").join(ORCH);
    std::fs::create_dir_all(&dir).expect("private orchestrator dir");
    let overlay = root.0.join("orchestrator.config");
    std::fs::write(&overlay, "[workspace]\norchestrator = true\n").expect("orchestrator overlay");
    let meta = format!(
        "schema=2\nsession_id={OID}\nlayout=lead-pair\nseat.main=lead\nmeta_agent=true\nlocal_config={}\n",
        overlay.display()
    );
    std::fs::write(dir.join("meta"), meta).expect("orchestrator meta");
}

pub(super) fn setup_board(tag: &str, desk: Option<DeskView>) -> (Root, App) {
    let root = Root::new(tag);
    seat_record(&root);
    let reader = Reader::new(root.0.clone(), Some("api".to_owned()), None);
    let mut app = app(Some("api"));
    refold(
        &mut app,
        &reader,
        &root,
        &[("api", Status::Running), (ORCH, Status::Running)],
    );
    app.lane = Rc::new(Lane {
        items: vec![said(MAIN_TURN, 0)],
        coverage: Vec::new(),
    });
    app.loading = false;
    app.answer(Answer::Orchestrator(loader::Proof::Proven(OID.to_owned())));
    bytes(&mut app, b"O");
    app.answer(Answer::View(ViewRead {
        name: ORCH.to_owned(),
        id: OID.to_owned(),
        seq: 1,
        lane: Lane {
            items: vec![said(PANEL_TURN, 0)],
            coverage: Vec::new(),
        },
        needs: None,
        desk,
        roster: None,
    }));
    (root, app)
}

pub(super) fn sample_desk(raw: &[u8]) -> DeskView {
    DeskView::Shown(Desk {
        updated: Timestamp::now(),
        items: parse_input(raw).expect("valid board"),
    })
}

fn assert_scrolled_osc8(app: &mut App, width: u16, height: u16, urls: &[&str]) {
    let mut ansi_bytes = Vec::new();
    {
        let mut terminal =
            Terminal::new(backend::Ansi::new(&mut ansi_bytes, (width, height))).expect("terminal");
        draw_frame(&mut terminal, |buffer| {
            app.frame(buffer);
            (app.layout.cursor, app.layout.links.clone())
        })
        .expect("ansi frame");
    }
    let ansi = String::from_utf8(ansi_bytes).expect("utf8");
    for url in urls {
        assert!(
            ansi.contains(&format!(";{url}\x1b\\")),
            "missing OSC 8 {url}"
        );
    }
}

#[test]
fn full_panel_shows_and_drags_adjacent_board_when_it_fits() {
    let (_root, mut app) = setup_board(
        "bf-full",
        Some(sample_desk(
            b"DECIDE\nD1 | Decide next step\nCOMING\nC1 | Coming soon\n",
        )),
    );
    bytes(&mut app, b"F");

    for width in [200, 161, 76] {
        let cells = paint(&mut app, width, 45);
        let rendered = text(&cells);
        assert!(
            app.layout.sessions().next().is_none(),
            "{width}: zoomed full panel hides sidebar"
        );
        assert!(!rendered.contains(MAIN_TURN), "{width}: main chat hidden");
        assert!(rendered.contains(PANEL_TURN), "{width}: panel chat visible");
        assert!(rendered.contains("BOARD"), "{width}: board header visible");
        let (dx, _) = locate(&cells, "Decide next step").expect("board item visible");
        assert!(dx >= 40, "{width}: board sits right of 40-cell panel floor");
        let header = row_text(&cells, 0);
        assert!(header.contains("F side"), "{width}: header {header:?}");
        assert!(header.contains("B board"), "{width}: header {header:?}");
    }

    let tight = paint(&mut app, 75, 45);
    assert!(
        !text(&tight).contains("Decide next step"),
        "75 cols leaves no room for 36-col board"
    );
    assert!(
        row_text(&tight, 0).contains("desk 1"),
        "header retains desk count when board column cannot fit"
    );

    let _ = paint(&mut app, 200, 45);
    assert_eq!(rules_at_bottom(&paint(&mut app, 200, 45)), [164]);
    drag_border(&mut app, 164, 140);
    assert_eq!(rules_at_bottom(&paint(&mut app, 200, 45)), [140]);
    drag_border(&mut app, 140, 190);
    assert_eq!(rules_at_bottom(&paint(&mut app, 200, 45)), [176]);
    drag_border(&mut app, 176, 10);
    assert_eq!(rules_at_bottom(&paint(&mut app, 200, 45)), [40]);

    drag_border(&mut app, 40, 176);
    assert!(text(&paint(&mut app, 64, 45)).contains("Decide next step"));
    assert!(!text(&paint(&mut app, 63, 45)).contains("Decide next step"));
}

#[test]
fn board_scrolls_by_wheel_and_keyboard_keeps_links_clickable_and_updates_cue() {
    let mut raw = String::from("RUNNING\nR1 | Top milestone\n");
    for n in 2..=13 {
        let _ = writeln!(
            raw,
            "R{n} | Middle step {n:02} with extra wrapped detail text"
        );
    }
    let web_url = "https://example.test/scrolled-web";
    let file_path = "/tmp/scrolled-file.md";
    let file_url = "file:///tmp/scrolled-file.md";
    let _ = writeln!(raw, "R14 | Bottom target | {web_url} | {file_path}");

    for full in [false, true] {
        let (_root, mut app) = setup_board("bf-scroll", Some(sample_desk(raw.as_bytes())));
        if full {
            bytes(&mut app, b"F");
        }
        let width = if full { 120 } else { 200 };
        let initial = paint(&mut app, width, 20);
        assert!(text(&initial).contains("Top milestone"));
        assert!(!text(&initial).contains("Bottom target"));
        assert!(
            app.layout
                .links
                .iter()
                .all(|run| run.url != web_url && run.url != file_url)
        );
        let cue = row_text(&initial, 19);
        assert!(cue.contains("more") && cue.contains('↓') && !cue.contains('↑'));

        let board_rule = *rules_at_bottom(&initial)
            .last()
            .expect("adjacent board rule");
        let board_x = board_rule + 2;

        wheel(&mut app, false, (board_x, 5));
        let stepped = paint(&mut app, width, 20);
        assert!(!text(&stepped).contains("Top milestone"));
        assert!(row_text(&stepped, 19).contains('↑'));

        for _ in 0..20 {
            wheel(&mut app, false, (board_x, 5));
        }
        let bottom = paint(&mut app, width, 20);
        assert!(text(&bottom).contains("Bottom target"));
        let web_at = scrolled_link_at(&app, &bottom, web_url, web_url, board_rule + 1);
        let _ = scrolled_link_at(&app, &bottom, file_url, file_path, board_rule + 1);

        assert_scrolled_osc8(&mut app, width, 20, &[web_url, file_url]);
        let _ = paint(&mut app, width, 20);
        app.flash = None;
        click(&mut app, web_at);
        assert_eq!(
            app.flash.as_ref().map(|(line, _)| line.as_str()),
            Some(link::open_hint())
        );

        // Clicking inside the board already gave it focus: PgUp scrolls without pressing B.
        send_key(&mut app, Key::PageUp);
        let click_scrolled = paint(&mut app, width, 20);
        assert!(text(&click_scrolled).contains("Top milestone"));
        assert!(!text(&click_scrolled).contains("Bottom target"));

        // Wheel back to bottom, release focus with B (PgUp stays on panel), then re-focus with B.
        for _ in 0..10 {
            wheel(&mut app, false, (board_x, 5));
        }
        assert!(text(&paint(&mut app, width, 20)).contains("Bottom target"));
        bytes(&mut app, b"B");
        send_key(&mut app, Key::PageUp);
        assert!(text(&paint(&mut app, width, 20)).contains("Bottom target"));
        bytes(&mut app, b"B");
        for _ in 0..10 {
            send_key(&mut app, Key::PageUp);
        }
        let back_top = paint(&mut app, width, 20);
        assert!(text(&back_top).contains("Top milestone"));
        assert!(!row_text(&back_top, 19).contains('↑'));

        if full {
            send_key(&mut app, Key::Escape);
        } else {
            bytes(&mut app, b"B");
        }
        assert!(app.orch.panel.is_some());
    }
}

#[test]
fn narrow_frame_toggles_board_over_panel_preserving_geometry_and_chat() {
    let web_url = "https://example.test/narrow-link";
    let mut raw = format!("DECIDE\nD1 | Narrow decision | {web_url}\nRUNNING\n");
    for n in 1..=13 {
        let _ = writeln!(
            raw,
            "R{n} | Narrow item {n:02} with extra wrapped detail text"
        );
    }
    let (_root, mut app) = setup_board("bf-narrow", Some(sample_desk(raw.as_bytes())));

    let before = paint(&mut app, 120, 20);
    let panel_before = app.layout.panel.expect("side panel drawn");
    let rules_before = rules_at_bottom(&before);
    let main_at = locate(&before, MAIN_TURN).expect("main chat visible");
    assert!(text(&before).contains(PANEL_TURN));
    assert!(!text(&before).contains("Narrow decision"));
    let header_before = row_text(&before, 0);
    assert!(header_before.contains("desk 1") && header_before.contains("B board"));

    bytes(&mut app, b"B");
    let replaced = paint(&mut app, 120, 20);
    let panel_after = app.layout.panel.expect("panel rect preserved");
    assert_eq!(panel_after, panel_before);
    assert_eq!(rules_at_bottom(&replaced), rules_before);
    assert_eq!(locate(&replaced, MAIN_TURN), Some(main_at));
    assert!(app.layout.sessions().any(|name| name == "api"));
    assert!(!text(&replaced).contains(PANEL_TURN));
    assert!(rect_text(&replaced, panel_after).contains("Narrow decision"));
    assert!(row_text(&replaced, 0).contains("B/Esc chat"));

    let link_at = scrolled_link_at(&app, &replaced, web_url, web_url, panel_after.x);
    app.flash = None;
    click(&mut app, link_at);
    assert_eq!(
        app.flash.as_ref().map(|(line, _)| line.as_str()),
        Some(link::open_hint())
    );
    click(
        &mut app,
        (panel_after.x + 3, panel_after.bottom().saturating_sub(2)),
    );
    assert!(!app.composing());

    send_key(&mut app, Key::PageDown);
    assert!(!rect_text(&paint(&mut app, 120, 20), panel_after).contains("Narrow decision"));
    send_key(&mut app, Key::PageUp);
    assert!(rect_text(&paint(&mut app, 120, 20), panel_after).contains("Narrow decision"));

    send_key(&mut app, Key::Escape);
    let restored = paint(&mut app, 120, 20);
    assert!(app.orch.panel.is_some());
    assert!(text(&restored).contains(PANEL_TURN));
    assert!(!text(&restored).contains("Narrow decision"));

    bytes(&mut app, b"F");
    let _ = paint(&mut app, 60, 20);
    bytes(&mut app, b"B");
    let full_board = paint(&mut app, 60, 20);
    assert!(text(&full_board).contains("Narrow decision"));
    assert!(row_text(&full_board, 0).contains("B/Esc chat"));
    assert!(row_text(&full_board, 0).contains("F side"));
    bytes(&mut app, b"B");
    let full_chat = paint(&mut app, 60, 20);
    assert!(text(&full_chat).contains(PANEL_TURN));
    assert!(row_text(&full_chat, 0).contains("F side"));

    bytes(&mut app, b"F");
    bytes(&mut app, b"i");
    assert!(app.composing());
    bytes(&mut app, b"Board");
    assert_eq!(
        app.writer_input().map(Input::draft).as_deref(),
        Some("Board")
    );
    send_key(&mut app, Key::Escape);
    assert!(!app.composing());
    let _ = paint(&mut app, 120, 20);
    bytes(&mut app, b"B");
    assert!(text(&paint(&mut app, 120, 20)).contains("Narrow decision"));
    send_key(&mut app, Key::Escape);
    let _ = paint(&mut app, 120, 20);
    bytes(&mut app, b"i");
    assert_eq!(
        app.writer_input().map(Input::draft).as_deref(),
        Some("Board")
    );
    assert!(rect_text(&paint(&mut app, 120, 20), panel_before).contains("Board"));
}

#[test]
fn absent_board_leaves_every_width_target_and_key_unchanged() {
    for (width, full) in [
        (120, false),
        (161, false),
        (200, false),
        (60, true),
        (160, true),
        (200, true),
    ] {
        let (_r1, mut none_app) = setup_board("bf-none", None);
        let (_r2, mut absent_app) = setup_board("bf-absent", Some(DeskView::Absent));
        if full {
            bytes(&mut none_app, b"F");
            bytes(&mut absent_app, b"F");
        }
        let none_buf = paint(&mut none_app, width, 35);
        let absent_buf = paint(&mut absent_app, width, 35);
        assert_eq!(text(&absent_buf), text(&none_buf), "{width}/{full}");
        assert_eq!(absent_app.layout.panel, none_app.layout.panel);
        assert_eq!(absent_app.layout.links, none_app.layout.links);
        assert_eq!(
            absent_app.layout.sessions().collect::<Vec<_>>(),
            none_app.layout.sessions().collect::<Vec<_>>()
        );
        let header = row_text(&absent_buf, 0);
        assert!(
            !header.contains("desk") && !header.contains("B board") && !header.contains("B/Esc")
        );

        let focused_before = absent_app.panel_focused();
        let full_before = absent_app.orch.panel.as_ref().map(|p| p.full);
        absent_app.flash = None;
        bytes(&mut absent_app, b"B");
        assert!(absent_app.flash.is_none());
        assert_eq!(absent_app.panel_focused(), focused_before);
        assert_eq!(absent_app.orch.panel.as_ref().map(|p| p.full), full_before);
        assert_eq!(text(&paint(&mut absent_app, width, 35)), text(&none_buf));
    }
}

#[test]
fn exact_fit_and_bottom_scroll_cue_and_plain_board_click() {
    let mut exact_raw = String::from("RUNNING\n");
    for n in 1..=6 {
        let _ = writeln!(exact_raw, "R{n} | Fit {n}");
    }
    let (_root, mut exact_app) =
        setup_board("bf-exact-fit", Some(sample_desk(exact_raw.as_bytes())));
    let exact = paint(&mut exact_app, 200, 8);
    let board_rect = Rect::new(164, 0, 36, 8);
    let exact_board = rect_text(&exact, board_rect);
    assert!(exact_board.contains("BOARD"));
    assert!(exact_board.contains("Fit 6"));
    assert!(
        !exact_board.contains("more") && !exact_board.contains('↓') && !exact_board.contains('↑'),
        "exact 8-row board shows no overflow cue:\n{exact_board}"
    );

    let mut long_raw = String::from("RUNNING\n");
    for n in 1..=9 {
        let _ = writeln!(long_raw, "R{n} | Row {n:02}");
    }
    long_raw.push_str("R10 | Plain tenth | https://example.test/extra\n");
    let (_root2, mut long_app) =
        setup_board("bf-bottom-cue", Some(sample_desk(long_raw.as_bytes())));
    let _ = paint(&mut long_app, 200, 10);
    for _ in 0..10 {
        wheel(&mut long_app, false, (170, 5));
    }
    let bottom = paint(&mut long_app, 200, 10);
    assert!(text(&bottom).contains("Plain tenth"));
    let cue = row_text(&bottom, 9);
    assert!(
        cue.contains("more") && cue.contains('↑') && !cue.contains('↓'),
        "bottom scroll cue points up only: {cue:?}"
    );

    let plain_at = locate(&bottom, "Plain tenth").expect("plain board text visible");
    long_app.flash = None;
    click(&mut long_app, plain_at);
    assert!(
        long_app.flash.is_none(),
        "clicking plain board text triggers no link hint"
    );
}

#[test]
fn board_wheel_redraw_signal_respects_scroll_bounds() {
    let mut raw = String::from("RUNNING\nR1 | Top milestone\n");
    for n in 2..=13 {
        let _ = writeln!(
            raw,
            "R{n} | Middle step {n:02} with extra wrapped detail text"
        );
    }
    let (_root, mut app) = setup_board("bf-wheel-bounds", Some(sample_desk(raw.as_bytes())));
    let _ = paint(&mut app, 200, 20);
    let wheel_at = |app: &mut App, kind| {
        take_keys(
            app,
            vec![(
                Key::Mouse(Mouse {
                    kind,
                    column: 170,
                    row: 5,
                }),
                Instant::now(),
            )],
        )
    };
    assert_eq!(wheel_at(&mut app, MouseKind::WheelUp), Some(false));
    assert_eq!(wheel_at(&mut app, MouseKind::WheelDown), Some(true));
    let _ = paint(&mut app, 200, 20);
    for _ in 0..20 {
        let _ = wheel_at(&mut app, MouseKind::WheelDown);
    }
    let _ = paint(&mut app, 200, 20);
    assert_eq!(wheel_at(&mut app, MouseKind::WheelDown), Some(false));
    assert_eq!(wheel_at(&mut app, MouseKind::WheelUp), Some(true));
}

#[test]
fn absent_board_b_ignores_unfocused_panel_and_clicking_b_board_exits_panel_composer() {
    let (_r1, mut absent) = setup_board("bf-absent-main", Some(DeskView::Absent));
    let first = paint(&mut absent, 120, 20);
    let main_at = locate(&first, MAIN_TURN).expect("main chat visible");
    click(&mut absent, main_at);
    assert!(absent.orch.panel.is_some() && !absent.panel_focused());
    absent.flash = None;
    let redraw = take_keys(&mut absent, Keys::app().feed(b"B", Instant::now()));
    assert_eq!(redraw, Some(false));
    assert!(!absent.panel_focused());
    assert!(absent.flash.is_none());

    let (_r2, mut comp) = setup_board(
        "bf-click-compose",
        Some(sample_desk(b"DECIDE\nD1 | Header click test\n")),
    );
    let _ = paint(&mut comp, 120, 20);
    bytes(&mut comp, b"i");
    bytes(&mut comp, b"keep");
    assert!(comp.composing());
    let cells = paint(&mut comp, 120, 20);
    let button = locate(&cells, "B board").expect("B board header button");
    click(&mut comp, button);
    assert!(!comp.composing(), "clicking B board leaves composer");
    let replaced = paint(&mut comp, 120, 20);
    assert!(text(&replaced).contains("Header click test"));
    send_key(&mut comp, Key::Escape);
    let _ = paint(&mut comp, 120, 20);
    bytes(&mut comp, b"i");
    assert_eq!(
        comp.writer_input().map(Input::draft).as_deref(),
        Some("keep")
    );
}
