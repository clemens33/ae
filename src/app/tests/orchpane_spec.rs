//! Frozen acceptance for the orchestrator panel's pane control.
//! Oracle: the orchpane brief and the existing seat-open contract in docs/app.md.

#![allow(
    clippy::disallowed_methods,
    reason = "the spec reads and writes only its private socket and session records"
)]

use super::o7app_units::{at as frame, cell_of, orchestrated};
use super::*;
use crate::inventory::ServerId;
use crate::meta::Selector;
use crate::theme::Look;
use crate::transport::{self, fixture_tmux};

const ORCH: &str = "orchestrator";
const OLD: &str = "0199c0de-2222-4890-abcd-ef0123456789";
const NEW: &str = "0199c0de-3333-4890-abcd-ef0123456789";
const ARM: &str = "o again to open orchestrator";
const OUTSIDE: &str =
    "refused: /open orchestrator: ae app is not running inside tmux; nothing selected";
const NO_VIEWER: &str =
    "refused: /open orchestrator: no tmux client is showing this app; nothing selected";
const REPLACED: &str = "refused: /open orchestrator: this session was replaced; nothing selected";
const ROSTER_CHANGED: &str =
    "refused: /open orchestrator: the roster no longer seats it in that slot; nothing selected";

fn press(app: &mut App, key: Key, when: Instant) {
    assert!(
        take_keys(app, vec![(key, when)]).is_some(),
        "app stays open"
    );
}

fn keys(app: &mut App, bytes: &[u8], when: Instant) {
    press(app, Key::Text(bytes.to_vec()), when);
}

fn lines(app: &App) -> Vec<String> {
    app.note()
        .into_iter()
        .chain(app.notices.iter().map(|(_, item)| item.body.clone()))
        .collect()
}

fn pane_target(app: &App, buf: &Buffer) -> Option<Mouse> {
    let header = app.layout.panel.expect("panel draws a header");
    let x = cell_of(buf, "pane", header.y, header.x)?;
    Some(Mouse {
        kind: MouseKind::Click,
        column: x + 1,
        row: header.y,
    })
}

fn main_roster(app: &mut App, uuid: &str, seq: u64, name: &str) {
    let Answer::View(mut read) = view(ORCH, uuid, seq, "the orchestrator chat") else {
        unreachable!()
    };
    read.roster = Some(vec![seat_ref("worker.0", "decoy"), seat_ref("main", name)]);
    app.answer(Answer::View(read));
}

#[test]
fn the_panel_header_pane_click_uses_the_seat_open_refusal() {
    for (full, icons) in [(false, false), (true, false), (false, true)] {
        let root = Root::new("orchpane-click");
        let (mut app, _reader) = orchestrated(&root, full);
        main_roster(&mut app, ID, 1, ORCH);
        app.looks.insert(
            "api".to_owned(),
            (
                Some(Look {
                    icons,
                    ..Look::DEFAULT
                }),
                None,
            ),
        );
        let buf = frame(160, 45, &mut app);
        let mode = if full { "F side" } else { "F full" };
        let header = text(&buf).lines().next().unwrap_or_default().to_owned();
        let (before, _) = header
            .rsplit_once(mode)
            .expect("the mode control stays visible");
        if icons {
            assert!(
                before.trim_end().ends_with("pane"),
                "pane precedes {mode}: {header}"
            );
        } else {
            assert_eq!(
                before.split_whitespace().last(),
                Some("pane"),
                "the plain pane control stands beside {mode}: {header}"
            );
        }
        let target = pane_target(&app, &buf).expect("pane is drawn");
        // The mouse still reaches the panel while the main column has keys.
        app.focus_column(false);
        press(&mut app, Key::Mouse(target), Instant::now());
        assert!(lines(&app).iter().any(|line| line == OUTSIDE));
        assert_eq!(app.model.selected(), Some("api"));
        let after = frame(160, 45, &mut app);
        let shown = text(&after);
        assert!(shown.lines().next().is_some_and(|row| row.contains(mode)));
    }
}

#[test]
fn a_focused_panel_opens_only_after_the_second_o_within_two_seconds() {
    let root = Root::new("orchpane-key");
    let (mut app, _reader) = orchestrated(&root, false);
    main_roster(&mut app, ID, 1, ORCH);
    let _ = frame(160, 45, &mut app);
    let at = Instant::now();
    keys(&mut app, b"o", at);
    assert_eq!(app.note().as_deref(), Some(ARM));
    assert!(!lines(&app).iter().any(|line| line.starts_with("refused:")));
    let late = at + Duration::from_millis(2_001);
    keys(&mut app, b"o", late);
    assert_eq!(app.note().as_deref(), Some(ARM), "a late press rearms");
    assert!(!lines(&app).iter().any(|line| line.starts_with("refused:")));
    keys(&mut app, b"o", late + Duration::from_millis(1_999));
    assert!(lines(&app).iter().any(|line| line == OUTSIDE));
    assert_ne!(app.note().as_deref(), Some(ARM));
}

#[test]
fn a_status_column_has_no_pane_target_and_oo_never_requests_a_seat() {
    for (proof, reason) in [
        (loader::Proof::Absent, "waiting"),
        (
            loader::Proof::Gap("missing live stamp".to_owned()),
            "missing live stamp",
        ),
        (loader::Proof::Proven(NEW.to_owned()), "replaced"),
        (loader::Proof::Proven(ID.to_owned()), "stopped"),
    ] {
        let root = Root::new("orchpane-status");
        let (mut app, reader) = housed(&root);
        let (ask, requests) = std::sync::mpsc::channel();
        app.ask = Some(ask);
        if reason == "replaced" || reason == "stopped" {
            session(&root, ORCH, "");
            refold(
                &mut app,
                &reader,
                &root,
                &[("api", Status::Running), (ORCH, Status::Running)],
            );
            app.answer(Answer::Orchestrator(loader::Proof::Proven(ID.to_owned())));
        } else {
            app.answer(Answer::Orchestrator(proof.clone()));
        }
        keys(&mut app, b"O", Instant::now());
        if reason == "stopped" {
            refold(
                &mut app,
                &reader,
                &root,
                &[("api", Status::Running), (ORCH, Status::Stopped)],
            );
        } else if reason == "replaced" {
            app.answer(Answer::Orchestrator(proof));
        }
        let buf = frame(160, 45, &mut app);
        assert!(app.panel_focused());
        assert!(
            pane_target(&app, &buf).is_none(),
            "{reason} shows no pane target"
        );
        assert!(text(&buf).contains(reason), "status column shows {reason}");
        for _ in requests.try_iter() {}
        keys(&mut app, b"oo", Instant::now());
        let said = lines(&app);
        assert!(
            said.iter()
                .any(|line| line.starts_with("refused:") && line.contains(reason)),
            "{reason} gets a one-line refusal: {said:?}"
        );
        assert_ne!(app.note().as_deref(), Some(ARM));
        assert!(
            !requests
                .try_iter()
                .any(|request| matches!(request, loader::Request::Seat(_))),
            "{reason} cannot start or resume a seat"
        );
    }
}

struct Socket {
    server: ServerId,
    retained: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for Socket {
    fn drop(&mut self) {
        let _ = fixture_tmux::end(&self.server);
        for _ in 0..20 {
            if fixture_tmux::probe(&self.server).proven() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        self.retained
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(std::thread::panicking(), "fixture server cleanup unproven");
    }
}

impl Socket {
    fn new(root: &Root) -> Self {
        Self {
            server: ServerId::Selected(Selector::Socket(root.0.join("s"))),
            retained: root.retention(),
        }
    }

    fn session(&self, root: &Root, name: &str) -> String {
        let argv = crate::session_tmux::argv(
            &self.server,
            &crate::session_tmux::Op::NewSession {
                name,
                work_dir: root.0.to_str().expect("fixture directory"),
            },
        );
        let (ok, out) = transport::run_tmux_op(&argv);
        assert!(ok, "private tmux new session {name}: {out}");
        transport::observe_window_panes(&self.server, name)
            .expect("private pane read")
            .first()
            .expect("one live fixture pane")
            .pane_id
            .clone()
    }

    fn stamp(&self, pane: &str, uuid: &str, name: &str) {
        let command = |command: &str| {
            assert!(transport::run_view_command(&self.server, ORCH, command));
        };
        command(&format!("set-option -t ={ORCH}: @ae_session_uuid {uuid}"));
        command(&format!("set-option -p -t {pane} @ae_slot main"));
        command(&format!("set-option -p -t {pane} @ae_agent {name}"));
    }
}

fn record(root: &Root, pane: &str, uuid: &str, name: &str) -> PathBuf {
    let dir = root.0.join("sessions").join(ORCH);
    std::fs::create_dir_all(&dir).expect("private session directory");
    let overlay = root.0.join("orchestrator.config");
    std::fs::write(&overlay, "[workspace]\norchestrator = true\n").expect("private overlay");
    let meta = format!(
        "schema=2\nsession_id={uuid}\nlayout=lead-pair\nseat.main={name}\nseat.worker.0=decoy\n\
         meta_agent=true\nlocal_config={}\nmain_pane={pane}\ntmux_server_kind=socket\ntmux_server={}\n",
        overlay.display(),
        root.0.join("s").display(),
    );
    std::fs::write(dir.join("meta"), meta).expect("private session record");
    dir
}

fn live_panel(root: &Root, socket: &Socket) -> (App, String, PathBuf) {
    let app_pane = socket.session(root, "api");
    let orch_pane = socket.session(root, ORCH);
    socket.stamp(&orch_pane, OLD, ORCH);
    let dir = record(root, &orch_pane, OLD, ORCH);
    let target =
        super::super::proven_pane(&socket.server, ORCH, OLD, &dir, Ok(&seat_ref("main", ORCH)))
            .expect("the private fixture has a proven main seat");
    assert_eq!(target.pane, orch_pane);

    let (mut app, reader) = housed(root);
    app.server = Some(socket.server.clone());
    app.pane = Some(app_pane);
    refold(
        &mut app,
        &reader,
        root,
        &[("api", Status::Running), (ORCH, Status::Running)],
    );
    app.answer(Answer::Orchestrator(loader::Proof::Proven(OLD.to_owned())));
    keys(&mut app, b"O", Instant::now());
    main_roster(&mut app, OLD, 1, ORCH);
    let _ = frame(160, 45, &mut app);
    (app, orch_pane, dir)
}

#[test]
fn a_press_uses_the_main_seat_and_reproves_the_bound_uuid_before_any_client_can_move() {
    let root = Root::new("orchpane-proof");
    let socket = Socket::new(&root);
    let (mut app, orch_pane, dir) = live_panel(&root, &socket);
    let journal = dir.join("events.jsonl");
    std::fs::write(&journal, b"").expect("private empty journal");
    keys(&mut app, b"oo", Instant::now());
    let said = lines(&app);
    assert!(
        said.iter().any(|line| line == NO_VIEWER),
        "the proved main seat passes every check before the client read: {said:?}"
    );

    let at = Instant::now();
    keys(&mut app, b"o", at);
    assert_eq!(app.note().as_deref(), Some(ARM));
    // The live evidence changes before the loader can answer. The panel
    // still shows the old incarnation. The new, valid stamp is a decoy.
    socket.stamp(&orch_pane, NEW, ORCH);
    let _ = record(&root, &orch_pane, NEW, ORCH);
    keys(&mut app, b"o", at + Duration::from_millis(100));
    let said = lines(&app);
    assert!(
        said.iter().any(|line| line == REPLACED),
        "the second press cannot follow the replacement: {said:?}"
    );
    assert_eq!(std::fs::read(journal).expect("private journal"), b"");
}

#[test]
fn the_second_o_keeps_the_main_seat_named_when_it_armed() {
    let root = Root::new("orchpane-roster");
    let socket = Socket::new(&root);
    let (mut app, orch_pane, dir) = live_panel(&root, &socket);
    let at = Instant::now();
    keys(&mut app, b"o", at);
    assert_eq!(app.note().as_deref(), Some(ARM));

    // The roster and slot both change while the session UUID stays valid.
    let successor = "successor";
    socket.stamp(&orch_pane, OLD, successor);
    let _ = record(&root, &orch_pane, OLD, successor);
    main_roster(&mut app, OLD, 2, successor);
    let target = super::super::proven_pane(
        &socket.server,
        ORCH,
        OLD,
        &dir,
        Ok(&seat_ref("main", successor)),
    )
    .expect("the new main seat would pass the proof if reselected");
    assert_eq!(target.pane, orch_pane);
    let _ = frame(160, 45, &mut app);

    keys(&mut app, b"o", at + Duration::from_millis(100));
    let said = lines(&app);
    assert!(
        said.iter().any(|line| line == ROSTER_CHANGED),
        "the armed key proves the original seat and refuses: {said:?}"
    );
}
