//! Frozen acceptance spec for `ae app` agent screen peek panel.
//! Oracles: brief-apppeek.md P1-P6, lead rulings D1-D9 (D1 key 'v', D2 2s cadence,
//! D3 pause on scroll, D4 plain `Op::CapturePane`, D5 hide gear, D6 read-gate,
//! D7 AGENTS.md, D8 wrap, D9 'q' closes), lead rulings I1-I4 (I1 tests/it
//! location, I2 strip trailing \r only, I3 initial loading state, I4 refusal purges body),
//! lead rulings S1-S5, and plan-apppeek.md sec 10 public surface.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::too_many_lines,
    reason = "acceptance owns private terminal fixtures and reads their effects"
)]

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use ae::app::draw::{Bounds, draw_peek};
use ae::app::fleet::Fleet;
use ae::app::model::{Act, Key, Model, browse_keys};
use ae::app::peek::{
    Body, HISTORY, KEPT_LINES, LINE_CHARS, PEEK_EVERY, STALE_SECS, View, bound, captured, refused,
    status, window,
};
use ae::console::input;
use ae::console::needs::SeatRef;
use ae::console::open::{Refusal, Target, capture_argv};
use ae::inventory::ServerId;
use ae::theme::Look;
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::Color;

fn render(buf: &Buffer) -> String {
    let mut out = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        let _ = writeln!(&mut out);
    }
    out
}

fn sample_view(body: Option<&Body>, age_secs: u64, back: usize, newer_waiting: bool) -> View<'_> {
    View {
        session: "aedev",
        seat: "worker",
        body,
        age_secs,
        back,
        newer_waiting,
    }
}

#[test]
fn test_browse_keys_v_maps_to_peek_and_p_remains_seat_prev() {
    assert_eq!(
        browse_keys(&input::Key::Text(vec![b'v'])),
        vec![Key::Peek],
        "v must decode to Key::Peek in browse mode"
    );
    assert_eq!(
        browse_keys(&input::Key::Text(vec![b'p'])),
        vec![Key::SeatPrev],
        "p must decode to Key::SeatPrev in browse mode"
    );

    let fleet = Fleet::default();
    let mut model = Model::new(&fleet);
    assert_eq!(
        model.key(Key::Peek, &fleet, false, false),
        Act::Peek,
        "Key::Peek in browse mode triggers Act::Peek"
    );
}

#[test]
fn test_model_peek_open_close_keys() {
    let fleet = Fleet::default();
    let mut model = Model::new(&fleet);

    assert!(!model.peek_open(), "initially closed");
    assert_eq!(model.peek_back(), None, "no peek back when closed");

    for (step, key) in [(1, Key::Esc), (2, Key::Quit), (3, Key::Peek)] {
        model.open_peek(step);
        assert!(model.peek_open(), "step {step}: open after open_peek");
        assert_eq!(model.peek_back(), Some(0), "step {step}: back 0 on open");
        assert_eq!(
            model.key(key, &fleet, false, false),
            Act::Redraw,
            "step {step}: key closes peek"
        );
        assert!(!model.peek_open(), "step {step}: closed");
        assert_eq!(model.peek_back(), None);
    }

    model.open_peek(4);
    for key in [
        Key::Digit(1),
        Key::Open,
        Key::Compose,
        Key::Tab,
        Key::SeatNext,
        Key::SeatPrev,
    ] {
        assert_eq!(
            model.key(key, &fleet, true, true),
            Act::None,
            "key swallowed while peek open"
        );
    }
    assert!(model.peek_open(), "remains open after swallowed keys");

    model.close_peek();
    assert!(!model.peek_open(), "closed via close_peek");
}

#[test]
fn test_model_peek_scroll_keys_and_clamping() {
    let fleet = Fleet::default();
    let mut model = Model::new(&fleet);

    model.open_peek(1);
    model.set_peek_page_rows(5);
    model.clamp_peek(20);

    assert_eq!(model.key(Key::Up, &fleet, false, false), Act::Redraw);
    assert_eq!(model.peek_back(), Some(1), "Up increments back");

    assert_eq!(model.key(Key::Down, &fleet, false, false), Act::Redraw);
    assert_eq!(model.peek_back(), Some(0), "Down decrements back");

    assert_eq!(model.key(Key::Down, &fleet, false, false), Act::Redraw);
    assert_eq!(model.peek_back(), Some(0), "Down saturates at 0");

    assert_eq!(model.key(Key::PageUp, &fleet, false, false), Act::Redraw);
    assert_eq!(model.peek_back(), Some(5), "PageUp adds page_rows");

    assert_eq!(model.key(Key::PageDown, &fleet, false, false), Act::Redraw);
    assert_eq!(model.peek_back(), Some(0), "PageDown subtracts page_rows");

    model.peek_wheel(true);
    assert!(
        model.peek_back().unwrap_or(0) > 0,
        "wheel older increments back"
    );

    model.clamp_peek(2);
    assert!(
        model.peek_back().unwrap_or(0) <= 2,
        "clamp_peek bounds scroll"
    );
}

#[test]
fn test_peek_constants_pin() {
    assert_eq!(PEEK_EVERY, Duration::from_secs(2), "D2 2s cadence");
    assert_eq!(HISTORY, 200, "200 lines history capture");
    assert_eq!(KEPT_LINES, 400, "400 lines max kept in buffer");
    assert_eq!(LINE_CHARS, 2000, "2000 max line chars including ellipsis");
    assert_eq!(STALE_SECS, 5, "STALE threshold exactly 5 seconds");
}

#[test]
fn test_peek_window_viewport_math() {
    assert_eq!(window(100, 20, 0), (80, 80), "back 0 shows newest rows");
    assert_eq!(window(100, 20, 10), (70, 80), "back 10 scrolls up 10");
    assert_eq!(window(100, 20, 80), (0, 80), "back max_back shows top");
    assert_eq!(window(100, 20, 999), (0, 80), "overshoot clamps to top");
    assert_eq!(window(10, 20, 0), (0, 0), "rows <= visible has 0 max_back");
    assert_eq!(window(0, 20, 0), (0, 0), "zero rows has 0 max_back");
}

#[test]
fn test_peek_bound_formatting_caps_and_trailing_blanks() {
    let raw = "line 1\r\nline 2\nline 3\n\n\n";
    assert_eq!(
        bound(raw),
        vec!["line 1", "line 2", "line 3"],
        "trailing blank lines dropped and CRLF stripped cleanly"
    );

    let mid_cr = "clean line\rmidline carriage return\r\n";
    assert_eq!(
        bound(mid_cr),
        vec!["clean line\rmidline carriage return"],
        "trailing CR stripped but midline CR preserved for painter neutralization"
    );

    let long = "x".repeat(2500);
    let bounded = bound(&long);
    assert_eq!(bounded.len(), 1);
    assert!(
        bounded[0].chars().count() <= LINE_CHARS,
        "line capped to LINE_CHARS"
    );
    assert!(bounded[0].ends_with('…'), "capped line ends with ellipsis");

    let many = (0..500)
        .map(|i| format!("row {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let bounded_many = bound(&many);
    assert_eq!(bounded_many.len(), KEPT_LINES, "capped at KEPT_LINES");
    assert_eq!(bounded_many.first().map(String::as_str), Some("row 100"));
    assert_eq!(bounded_many.last().map(String::as_str), Some("row 499"));
}

#[test]
fn test_peek_captured_body_variants() {
    let cap_fail = "capture failed: tmux did not return the pane's screen".to_owned();
    assert_eq!(captured(None), Body::Gap(cap_fail.clone()));
    assert_eq!(captured(Some((false, "err"))), Body::Gap(cap_fail));
    assert_eq!(
        captured(Some((true, ""))),
        Body::Gap("(the pane shows nothing)".to_owned())
    );
    assert_eq!(
        captured(Some((true, "   \n\n"))),
        Body::Gap("(the pane shows nothing)".to_owned())
    );
    assert_eq!(
        captured(Some((true, "hello\nworld"))),
        Body::Lines(vec!["hello".to_owned(), "world".to_owned()])
    );
}

#[test]
fn test_peek_refused_body_variants() {
    assert_eq!(
        refused("lead", &Refusal::Dead),
        Body::Gap("refused: peek lead: its pane is dead".to_owned())
    );
    assert_eq!(
        refused("worker.1", &Refusal::OtherAgent),
        Body::Gap("refused: peek worker.1: its pane carries another agent".to_owned())
    );
    assert_eq!(
        refused("worker.2", &Refusal::NoPane),
        Body::Gap("refused: peek worker.2: no pane carries its slot".to_owned())
    );
}

#[test]
fn test_peek_status_text_variants() {
    assert_eq!(
        status(&sample_view(None, 0, 0, false)),
        "loading",
        "initial loading state"
    );

    let gap = Body::Gap("refused".to_owned());
    assert_eq!(
        status(&sample_view(Some(&gap), 0, 0, false)),
        "not captured",
        "gap body status"
    );

    let lines = Body::Lines(vec!["a".to_owned(), "b".to_owned()]);
    assert_eq!(
        status(&sample_view(Some(&lines), 2, 0, false)),
        "captured 2s ago · 2 rows",
        "live status with age and row count"
    );
    assert_eq!(
        status(&sample_view(Some(&lines), STALE_SECS, 0, false)),
        "STALE · captured 5s ago · 2 rows",
        "stale status at STALE_SECS boundary"
    );
    assert_eq!(
        status(&sample_view(Some(&lines), 2, 4, false)),
        "paused · 4 back",
        "paused status when scrolled back"
    );
    assert_eq!(
        status(&sample_view(Some(&lines), 2, 4, true)),
        "paused · 4 back · newer waiting",
        "paused status with newer waiting"
    );
}

#[test]
fn test_open_capture_argv_builder() {
    let server = ServerId::Ambient;
    let target_ok = Target {
        seat: SeatRef {
            slot: "spawned.0".to_owned(),
            name: "worker".to_owned(),
        },
        session_id: "$1".to_owned(),
        pane: "%42".to_owned(),
        uuid: "0199c0de-cccc-4890-abcd-ef0123456789".to_owned(),
    };
    let argv = capture_argv(&server, &target_ok, HISTORY).expect("valid %N pane builds argv");

    for flag in ["capture-pane", "-p", "-J", "-S", "-200", "-E", "-", "%42"] {
        assert!(argv.iter().any(|arg| arg == flag), "contains {flag}");
    }
    assert!(
        !argv.iter().any(|arg| arg == "-e"),
        "-e omitted per D4 plain text requirement"
    );

    let target_invalid = Target {
        seat: target_ok.seat.clone(),
        session_id: "$1".to_owned(),
        pane: "invalid:0".to_owned(),
        uuid: target_ok.uuid.clone(),
    };
    assert!(
        capture_argv(&server, &target_invalid, HISTORY).is_none(),
        "non-%N pane id returns None"
    );
}

#[test]
fn test_draw_peek_frame_initial_loading() {
    let v_loading = sample_view(None, 0, 0, false);
    let mut buf = Buffer::empty(Rect::new(0, 0, 72, 12));
    let look = Look::read("on", "darcula", "on", "on");
    let bounds: Bounds = draw_peek(&v_loading, Some(look), &mut buf);

    assert_eq!(bounds.visible, 8, "visible rows is height - 4");
    assert_eq!(bounds.max_back, 0, "max_back is 0 when loading");

    let screen = render(&buf);
    assert!(screen.contains("worker"), "header contains seat: {screen}");
    assert!(
        screen.contains("loading"),
        "status or body contains loading: {screen}"
    );
}

#[test]
fn test_draw_peek_frame_normal_and_neutralisation() {
    let long_wrap_line = format!("header_word {}", "word_wrap_test ".repeat(8));
    let lines = Body::Lines(vec![
        "clean text line".to_owned(),
        "\x1b[2J\x1b]0;bad_title\x07\x00\u{0085}\x1b[31mcolor\x1b[0m".to_owned(),
        "mid\rline carriage return".to_owned(),
        long_wrap_line,
    ]);
    let v = sample_view(Some(&lines), 2, 0, false);
    let mut buf = Buffer::empty(Rect::new(0, 0, 72, 12));
    let look = Look::read("on", "darcula", "on", "on");
    let bounds: Bounds = draw_peek(&v, Some(look), &mut buf);

    assert_eq!(bounds.visible, 8);
    let screen = render(&buf);

    assert!(
        screen.contains("clean text line"),
        "clean line present: {screen}"
    );
    assert!(
        screen.chars().all(|ch| ch == '\n' || !ch.is_control()),
        "no raw control char survives into screen: {screen}"
    );
    assert!(
        screen.contains('\u{FFFD}'),
        "hostile escape sequence neutralised to replacement char: {screen}"
    );
    assert!(
        screen.contains("captured 2s ago · 4 rows"),
        "status row displays captured age and row count: {screen}"
    );
    assert!(
        screen.contains("word_wrap_test"),
        "D8 wrap: long line wraps without text loss: {screen}"
    );

    // C1: Assert drawn-look colours are applied (palette background / foreground not default Reset)
    let non_reset_bg =
        (0..buf.area.height).any(|y| (0..buf.area.width).any(|x| buf[(x, y)].bg != Color::Reset));
    assert!(non_reset_bg, "drawn look applies palette background colors");
}

#[test]
fn test_draw_peek_frame_gap_refusal_purges_body() {
    let gap = Body::Gap("refused: peek worker: its pane is dead".to_owned());
    let v = sample_view(Some(&gap), 0, 0, false);
    let mut buf = Buffer::empty(Rect::new(0, 0, 72, 12));
    let look = Look::read("on", "darcula", "on", "on");
    let _bounds: Bounds = draw_peek(&v, Some(look), &mut buf);

    let screen = render(&buf);
    assert!(
        screen.contains("refused: peek worker: its pane is dead"),
        "gap message shown in body: {screen}"
    );
    assert!(
        screen.contains("not captured"),
        "footer status displays not captured: {screen}"
    );
}

#[test]
fn test_draw_peek_floor_fallback() {
    let v = sample_view(None, 0, 0, false);
    let mut buf = Buffer::empty(Rect::new(0, 0, 35, 6));
    let bounds: Bounds = draw_peek(&v, None, &mut buf);

    assert_eq!(
        bounds.visible, 0,
        "visible is 0 when below 40x8 floor fallback"
    );
    assert_eq!(
        bounds.max_back, 0,
        "max_back is 0 when below 40x8 floor fallback"
    );

    let screen = render(&buf);
    assert!(
        screen.trim().is_empty(),
        "nothing painted below minimum floor: {screen:?}"
    );
}

#[test]
fn test_s1_live_private_tmux_peek_lifecycle() {
    let mut scratch = super::cli::OwnedScratch::root("apppeek", "s1");
    let root = scratch.path().to_owned();
    let socket = root.join("sock");
    scratch.add_tmux_server(socket.clone());
    let gates = root.join("gates");
    let _ = std::fs::create_dir_all(&gates);
    let _ = std::fs::write(root.join("config"), "[workspace]\nchat = app\n");
    let session = "dlapppks1".to_owned();

    let tmux_at = |args: &[&str]| {
        let mut full = vec!["-S".to_owned(), socket.display().to_string()];
        full.extend(args.iter().map(|s| (*s).to_owned()));
        let (ok, out) = super::phase2::run_tmux(&full, &root);
        assert!(ok, "tmux {args:?}: {out}");
        out
    };

    let lead_pane = tmux_at(&[
        "-f",
        "/dev/null",
        "new-session",
        "-d",
        "-P",
        "-F",
        "#{pane_id}",
        "-s",
        &session,
        "-x",
        "160",
        "-y",
        "45",
        "sleep",
        "600",
    ])
    .trim()
    .to_owned();

    let sdir = root.join("sessions").join(&session);
    let _ = std::fs::create_dir_all(&sdir);
    let _ = std::fs::write(
        sdir.join("meta"),
        format!(
            "session={session}\nmode=local\nsession_id=0199c0de-cccc-4890-abcd-ef0123456789\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=codex\nlaunch_id.main=tok-lead\nseat.worker.0=colead\nagent_bin.worker.0=codex\nlaunch_id.worker.0=tok-colead\n",
            socket.display()
        ),
    );
    let _ = std::fs::write(
        sdir.join(".launch-attempt"),
        ae::time::Timestamp::now().epoch().to_string(),
    );
    let _ = std::fs::write(sdir.join("events.jsonl"), "");
    let _ = ae::watchdog_glue::touch_beat(&sdir);

    for (opt, val) in [
        ("@ae_look", "off"),
        ("@ae_session_uuid", "0199c0de-cccc-4890-abcd-ef0123456789"),
        ("@ae_motion", "off"),
        ("@ae_main_pane", lead_pane.as_str()),
        ("@ae_attn_rank", "0"),
        ("@ae_attn_glyph", "idle"),
        ("default-size", "160x45"),
    ] {
        tmux_at(&["set-option", "-t", &session, opt, val]);
    }
    tmux_at(&["set-option", "-p", "-t", &lead_pane, "@ae_agent", "lead"]);
    tmux_at(&["set-option", "-p", "-t", &lead_pane, "@ae_slot", "main"]);

    let foreign_ready = root.join("foreign-ready");
    let fixture = format!(
        "printf 'PEEK_MARKER_TEST_77777\\n'; while test ! -e '{}'; do sleep 0.1; done; printf 'SECRET_FOREIGN_TEXT\\n'; exec sleep 600",
        foreign_ready.display()
    );
    let colead_pane = tmux_at(&[
        "new-window",
        "-d",
        "-P",
        "-F",
        "#{pane_id}",
        "-t",
        &session,
        "-n",
        "colead",
        "sh",
        "-c",
        &fixture,
    ])
    .trim()
    .to_owned();
    for (option, value) in [("@ae_slot", "worker.0"), ("@ae_agent", "colead")] {
        tmux_at(&["set-option", "-p", "-t", &colead_pane, option, value]);
    }

    let fact = format!(
        "v1;{};3600;lead:idle:working:{};colead:idle:working:{}",
        ae::time::Timestamp::now().epoch(),
        lead_pane,
        colead_pane,
    );
    tmux_at(&["set-option", "-t", &session, "@ae_agents", &fact]);

    let env_vars = [
        format!("AE_HOME={}", root.display()),
        format!("CONFIG_FILE={}", root.join("config").display()),
        format!("HOME={}", root.display()),
        "AE_TMUX_SERVER_KIND=socket".to_owned(),
        format!("AE_TMUX_SERVER={}", socket.display()),
        format!("AE_TEST_APP_READ_GATE={}", gates.display()),
    ];
    let mut app_args = vec!["new-window", "-d", "-P", "-F", "#{pane_id}", "-t", &session];
    app_args.extend(["env", "-u", "CLAUDE_CONFIG_DIR", "-u", "CODEX_HOME"]);
    app_args.extend(env_vars.iter().map(String::as_str));
    app_args.extend([env!("CARGO_BIN_EXE_ae"), "app", &session]);
    let app_pane = tmux_at(&app_args).trim().to_owned();
    tmux_at(&["resize-window", "-t", &app_pane, "-x", "160", "-y", "45"]);

    let wait_screen = |within: Duration, why: &str, met: &dyn Fn(&str) -> bool| -> String {
        let until = Instant::now() + within;
        loop {
            let sc = tmux_at(&["capture-pane", "-p", "-t", &app_pane]);
            if met(&sc) {
                return sc;
            }
            assert!(Instant::now() < until, "{why}; terminal screen:\n{sc}");
            std::thread::sleep(Duration::from_millis(25));
        }
    };

    wait_screen(Duration::from_secs(15), "app painted", &|sc| {
        sc.contains(&session)
    });

    tmux_at(&["send-keys", "-t", &app_pane, "Tab"]);
    wait_screen(Duration::from_secs(5), "Agents tab drawn", &|sc| {
        sc.contains("colead")
    });
    tmux_at(&["send-keys", "-t", &app_pane, "n"]);
    wait_screen(Duration::from_secs(5), "colead highlighted", &|sc| {
        sc.lines().any(|row| {
            let left = row.split('│').next().unwrap_or_default();
            (left.contains('>') || left.contains('▸')) && left.contains("colead")
        })
    });
    tmux_at(&["send-keys", "-t", &app_pane, "v"]);

    wait_screen(Duration::from_secs(5), "marker visible in peek", &|sc| {
        sc.contains("PEEK_MARKER_TEST_77777")
    });

    let refusal_due = Instant::now() + Duration::from_secs(5);
    tmux_at(&[
        "set-option",
        "-p",
        "-t",
        &colead_pane,
        "@ae_agent",
        "intruder",
    ]);
    wait_screen(
        refusal_due.saturating_duration_since(Instant::now()),
        "refusal when pane carries another agent",
        &|sc| {
            !sc.contains("PEEK_MARKER_TEST_77777")
                && !sc.contains("SECRET_FOREIGN_TEXT")
                && sc.contains("refused: peek colead: its pane carries another agent")
        },
    );

    std::fs::write(&foreign_ready, "foreign pane output").expect("signal fixture");
    let secret_due = Instant::now() + Duration::from_secs(3);
    while !tmux_at(&["capture-pane", "-p", "-t", &colead_pane]).contains("SECRET_FOREIGN_TEXT") {
        assert!(
            Instant::now() < secret_due,
            "fixture printed foreign secret"
        );
        std::thread::sleep(Duration::from_millis(25));
    }

    let until = Instant::now() + Duration::from_secs(6);
    let mut last_sc = String::new();
    while Instant::now() < until {
        let sc = tmux_at(&["capture-pane", "-p", "-t", &app_pane]);
        assert!(
            !sc.contains("PEEK_MARKER_TEST_77777") && !sc.contains("SECRET_FOREIGN_TEXT"),
            "leaked marker or foreign secret during restamped poll: {sc}"
        );
        last_sc = sc;
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        last_sc.contains("refused: peek colead: its pane carries another agent"),
        "refusal line preserved across 6s poll: {last_sc}"
    );

    tmux_at(&["kill-pane", "-t", &colead_pane]);
    wait_screen(Duration::from_secs(5), "refusal after pane killed", &|sc| {
        !sc.contains("PEEK_MARKER_TEST_77777")
            && !sc.contains("SECRET_FOREIGN_TEXT")
            && sc.contains("refused: peek colead: no pane carries its slot")
    });
}
