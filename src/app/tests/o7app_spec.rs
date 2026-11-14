//! Frozen acceptance for the app-native orchestrator panel.
//! Oracle: brief-o7app, plan r2 plus delta 1, and lead ruling
//! ae-20261114T095906Z-3a9a435f. This module is a child of `app::tests`.
//! It drives the real reducer, renderer, reader and submit admission.
//! Only the final external delivery is replaced by a recording sink.
//! Every tmux endpoint belongs to this fixture and uses an explicit socket.

#![allow(
    clippy::disallowed_methods,
    reason = "tests read and write their own private fixtures"
)]

use super::*;
use std::sync::{Arc, Mutex, mpsc};

const ORCH: &str = "orchestrator";
const OID: &str = "0199c0de-2222-4890-abcd-ef0123456789";
const NEW: &str = "0199c0de-3333-4890-abcd-ef0123456789";
const MAIN: &str = "MAIN_SENTINEL";
const PANEL: &str = "ORCH_SENTINEL";

/// Registered before the first server starts; a failed cleanup keeps its root.
struct Socket {
    server: crate::inventory::ServerId,
    retained: Arc<std::sync::atomic::AtomicBool>,
}
impl Drop for Socket {
    fn drop(&mut self) {
        use crate::transport::fixture_tmux as wire;
        let _ = wire::end(&self.server);
        let mut last = wire::probe(&self.server);
        for _ in 0..20 {
            if last.proven() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
            last = wire::probe(&self.server);
        }
        self.retained
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(
            std::thread::panicking(),
            "private socket cleanup unproven: {last:?}"
        );
    }
}

/// Drop order ends the server before removing its state.
struct Rig {
    socket: Socket,
    root: Root,
    app: App,
    reader: Reader,
    deliveries: Arc<Mutex<Vec<(String, String)>>>,
    detaches: Arc<std::sync::atomic::AtomicUsize>,
    tx: mpsc::Sender<loader::Request>,
    rx: mpsc::Receiver<loader::Request>,
}
impl Rig {
    fn new(home: Option<&str>) -> Self {
        let root = Root::new("oa");
        let server =
            crate::inventory::ServerId::Selected(crate::meta::Selector::Socket(root.0.join("s")));
        let socket = Socket {
            server: server.clone(),
            retained: root.retention(),
        };
        let (tx, rx) = mpsc::channel();
        let app = App::new(
            home.map(str::to_owned),
            Some(server.clone()),
            None,
            Some(tx.clone()),
        );
        let reader = Reader::new(root.0.clone(), home.map(str::to_owned), Some(server));
        let mut rig = Self {
            socket,
            root,
            app,
            reader,
            deliveries: Arc::new(Mutex::new(Vec::new())),
            detaches: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            tx,
            rx,
        };
        rig.isolate();
        for name in ["api", "b", "c", ORCH] {
            rig.record(name, if name == ORCH { OID } else { ID });
            let (ok, _) = crate::transport::run_tmux_op(&crate::session_tmux::argv(
                &rig.socket.server,
                &crate::session_tmux::Op::NewSession {
                    name,
                    work_dir: rig.root.0.to_str().unwrap(),
                },
            ));
            assert!(ok, "create private session {name}");
            for (key, value) in [
                ("AE_SESSION", name),
                ("AE_HOME", rig.root.0.to_str().unwrap()),
            ] {
                let (ok, _) = crate::transport::run_tmux_op(&crate::session_tmux::argv(
                    &rig.socket.server,
                    &crate::session_tmux::Op::SetEnv {
                        session: name,
                        key,
                        value,
                    },
                ));
                assert!(ok, "mark private session {name}");
            }
            rig.command(&format!(
                "set-option -t ={name}: @ae_session_uuid {}",
                if name == ORCH { OID } else { ID },
            ));
        }
        rig.refresh(&[]);
        for name in ["api", "b", "c", ORCH] {
            assert_eq!(
                rig.app.entry(name).map(|e| e.status),
                Some(Status::Running),
                "the fixture must establish positive liveness before a panel assertion"
            );
        }
        rig.lanes();
        rig
    }

    /// Replace external effects, while retaining actual entry and submit admission.
    fn isolate(&mut self) {
        let deliveries = Arc::clone(&self.deliveries);
        self.app.carry = Arc::new(move |ask| {
            deliveries
                .lock()
                .unwrap()
                .push((ask.seat().to_owned(), ask.body().to_owned()));
            "notice UNCONFIRMED: fixture external delivery sink".to_owned()
        });
        let detaches = Arc::clone(&self.detaches);
        self.app.detach = Box::new(move |_| {
            detaches.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            false
        });
    }

    fn record(&self, name: &str, uuid: &str) {
        let dir = self.root.0.join("sessions").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let lead = if name == ORCH { "olead" } else { "lead" };
        let peer = if name == ORCH { "opeer" } else { "colead" };
        std::fs::write(dir.join("meta"), format!(
            "schema=2\nsession_id={uuid}\nlayout=lead-pair\nseat.main={lead}\nseat.worker.0={peer}\ntmux_server_kind=socket\ntmux_server={}\nwork_dir={}\n",
            self.root.0.join("s").display(), self.root.0.display(),
        )).unwrap();
    }

    fn command(&self, command: &str) {
        assert!(
            crate::transport::run_view_command(&self.socket.server, "api", command),
            "private fixture command: {command}"
        );
    }

    fn refresh(&mut self, needy: &[&str]) {
        for answer in self.reader.fleet() {
            let answer = match answer {
                Answer::Fleet(mut read) => {
                    // The lane sentry and other tests may publish sessions on
                    // another endpoint. This fixture's input World is exactly
                    // its four sessions, including the orchestrator under test.
                    // Keep the actual identity proof from the private socket.
                    read.world
                        .sessions
                        .retain(|entry| matches!(entry.name.as_str(), "api" | "b" | "c" | ORCH));
                    // The independent World input names attention. Keep every
                    // actual identity proof produced by this real fleet read.
                    for entry in &mut read.world.sessions {
                        entry.attention = needy
                            .contains(&entry.name.as_str())
                            .then_some(Reason::WaitingUser);
                    }
                    // Needs are independent fixture inputs to the pure fleet
                    // reducer. Attention alone determines ink, not ring membership.
                    read.needs.clear();
                    for name in needy {
                        use crate::console::needs::{
                            Row as Need, SeatRef, Section, Source, Verdict,
                        };
                        read.needs.insert(
                            (*name).to_owned(),
                            Section {
                                rows: vec![Need {
                                    seat: SeatRef {
                                        slot: "main".to_owned(),
                                        name: if *name == ORCH { "olead" } else { "lead" }
                                            .to_owned(),
                                    },
                                    lead_pair: true,
                                    verdict: Verdict::Reason(Reason::WaitingUser),
                                    source: Source::Declaration,
                                    stale: None,
                                    since_micros: None,
                                    detail: "fixture question".to_owned(),
                                    record: None,
                                }],
                            },
                        );
                    }
                    read.fleet = fleet::rows(
                        &read.world,
                        &read.facts,
                        &std::collections::BTreeMap::new(),
                        &crate::theme::FleetOrder::EMPTY,
                        &read.needs,
                        self.app.home.as_deref(),
                        Timestamp::now(),
                    );
                    Answer::Fleet(read)
                }
                other => other,
            };
            self.app.answer(answer);
        }
        for name in ["api", "b", "c", ORCH] {
            self.app
                .looks
                .insert(name.to_owned(), (Some(crate::theme::Look::DEFAULT), None));
        }
        self.flush();
    }

    fn flush(&mut self) {
        self.app.focus();
        for ask in self.rx.try_iter() {
            self.reader.take(ask);
        }
    }

    fn lanes(&mut self) {
        self.app.answer(view("api", ID, 100, MAIN));
        self.app.answer(view("b", ID, 100, "B_SENTINEL"));
        self.app.answer(view("c", ID, 100, "C_SENTINEL"));
        self.app.answer(view(ORCH, OID, 100, PANEL));
    }

    fn key(&mut self, key: Key) {
        assert!(
            take_keys(&mut self.app, vec![(key, Instant::now())]).is_some(),
            "an acceptance key must not quit the app"
        );
        self.flush();
        assert_eq!(
            self.detaches.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "acceptance input must never launch a detached process"
        );
    }

    fn text(&mut self, bytes: &[u8]) {
        self.key(Key::Text(bytes.to_vec()));
    }

    fn frame(&mut self, width: u16, height: u16) -> Buffer {
        let mut buffer = Buffer::empty(Rect::new(0, 0, width, height));
        self.app.frame(&mut buffer);
        self.flush();
        buffer
    }

    fn open(&mut self) {
        let selected = self.app.model.selected().map(str::to_owned);
        self.text(b"O");
        self.lanes();
        let buffer = self.frame(160, 45);
        let shown = text(&buffer);
        assert_eq!(
            self.app.model.selected(),
            selected.as_deref(),
            "opening the panel preserves the ordinary selection"
        );
        assert!(
            shown.contains(MAIN) && shown.contains(PANEL) && shown.contains("F full"),
            "O must show the native panel beside the main lane: {shown}"
        );
        assert!(cell(&buffer, MAIN).unwrap().0 < cell(&buffer, PANEL).unwrap().0);
    }

    fn click(&mut self, x: u16, y: u16) {
        for kind in [MouseKind::Click, MouseKind::Release] {
            self.key(Key::Mouse(Mouse {
                kind,
                column: x,
                row: y,
            }));
        }
    }

    fn click_word(&mut self, word: &str) {
        let buffer = self.frame(160, 45);
        let (x, y) =
            cell(&buffer, word).unwrap_or_else(|| panic!("missing {word}: {}", text(&buffer)));
        self.click(x, y);
    }

    fn draft(&self, name: &str, uuid: &str) -> String {
        self.app
            .inputs
            .get(&(name.to_owned(), uuid.to_owned()))
            .map(Input::draft)
            .unwrap_or_default()
    }

    fn assert_no_delivery(&mut self) {
        self.key(Key::Enter);
        assert!(!self.app.composing(), "an inert panel must refuse entry");
        self.key(Key::Pasted(b"blocked ask".to_vec()));
        assert!(!self.app.composing(), "an inert panel must refuse a paste");
        self.key(Key::Enter);
        let _ = self.app.deliver();
        assert!(
            self.app.queued.is_empty() && self.app.flight.is_none(),
            "an unproven panel must admit no ask"
        );
        assert!(
            self.deliveries.lock().unwrap().is_empty(),
            "an inert panel never calls the external delivery sink"
        );
        assert_eq!(self.detaches.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}

/// Locate literal rendered cells, independently of the product hit map.
fn cell(buffer: &Buffer, word: &str) -> Option<(u16, u16)> {
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let tail: String = (x..buffer.area.width)
                .map(|at| buffer[(at, y)].symbol())
                .collect();
            if tail.starts_with(word) {
                return Some((x, y));
            }
        }
    }
    None
}

fn body(buffer: &Buffer, x: u16, width: u16) -> String {
    (2..buffer.area.height.saturating_sub(5))
        .map(|y| {
            (x..x + width)
                .map(|at| buffer[(at, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn p1_the_orchestrator_is_not_a_session_row_or_a_count_but_remains_live() {
    let mut rig = Rig::new(Some("api"));
    let names: Vec<_> = rig.app.fleet.rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names.len(), 3);
    assert!(
        !names.contains(&ORCH),
        "the orchestrator is never an ordinary row"
    );
    assert_eq!(
        rig.app
            .fleet
            .rows
            .iter()
            .map(|r| r.index)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(rig.app.fleet.rows.len(), 3);
    // Give each ordinary row two known seats. The real orchestrator has no
    // roster publication, so counting it would also add an unknown marker.
    for row in &mut rig.app.fleet.rows {
        row.counts = Counts::Marks(vec![(Mark::Working, false, 1), (Mark::Idle, false, 1)]);
    }
    let summary = fleet::summary(&rig.app.fleet);
    assert_eq!(
        (
            summary.agents,
            summary.working,
            summary.needy,
            summary.unknown
        ),
        (6, 3, 0, false)
    );
    assert_eq!(rig.app.entry(ORCH).map(|e| e.status), Some(Status::Running));
    rig.text(b"3");
    assert_ne!(rig.app.model.selected(), Some(ORCH));
    rig.text(b"jjjjkkkk");
    assert_ne!(rig.app.model.selected(), Some(ORCH));
    rig.command("kill-session -t =orchestrator");
    rig.refresh(&[]);
    assert!(!rig.app.fleet.rows.iter().any(|r| r.name == ORCH));
    assert_eq!(
        rig.app.stopped().total,
        0,
        "hidden orchestrator is not a stopped-group member"
    );
    assert_eq!(rig.app.entry(ORCH).map(|e| e.status), Some(Status::Stopped));
}

#[test]
fn p1_the_needy_ring_reaches_the_closed_panel_and_wraps_without_starving_rows() {
    let mut rig = Rig::new(Some("api"));
    let quiet = rig.frame(160, 45);
    let quiet_cell = cell(&quiet, "◆").expect("closed orchestrator control");
    let quiet_ink = quiet[(quiet_cell.0, quiet_cell.1)].fg;
    rig.refresh(&[ORCH, "b", "c"]);
    rig.lanes();
    let button = rig.frame(160, 45);
    let (x, y) = cell(&button, "◆").expect("closed orchestrator has its own control");
    assert_eq!(y, 44, "the button is on the hint row");
    assert_ne!(
        button[(x, y)].fg,
        quiet_ink,
        "the closed button carries the needy mark"
    );
    for expected in ["b", "c"] {
        rig.text(b"!");
        assert_eq!(rig.app.model.selected(), Some(expected));
    }
    rig.text(b"!");
    rig.lanes();
    let opened = text(&rig.frame(160, 45));
    assert!(
        opened.contains(PANEL) && opened.contains("C_SENTINEL"),
        "{opened}"
    );
    assert_eq!(
        rig.app.model.selected(),
        Some("c"),
        "panel opening preserves main selection"
    );
    rig.text(b"!");
    assert_eq!(
        rig.app.model.selected(),
        Some("b"),
        "the cursor advances from the panel"
    );
    assert_eq!(
        fleet::summary(&rig.app.fleet).needy,
        2,
        "summary excludes the needy orchestrator"
    );
}

#[test]
fn p1_a_closed_unproven_orchestrator_is_skipped_by_the_needy_ring() {
    let mut rig = Rig::new(Some("api"));
    rig.command(&format!(
        "set-option -t =orchestrator: @ae_session_uuid {NEW}"
    ));
    rig.refresh(&[ORCH, "b"]);
    rig.text(b"!");
    assert_eq!(rig.app.model.selected(), Some("b"));
    rig.text(b"!");
    assert_eq!(rig.app.model.selected(), Some("b"));
    assert!(!text(&rig.frame(160, 45)).contains(PANEL));
}

#[test]
fn p2_side_full_mode_clicks_and_the_same_button_hide_without_moving_the_selection() {
    let mut rig = Rig::new(Some("api"));
    let closed = rig.frame(160, 45);
    let (button_x, button_y) = cell(&closed, "◆").expect("dedicated visible toggle");
    rig.click(button_x, button_y);
    rig.lanes();
    let side = rig.frame(160, 45);
    let main = cell(&side, MAIN).expect("main lane remains visible");
    let orch = cell(&side, PANEL).expect("panel has its own native lane");
    assert!(
        main.0 < orch.0,
        "orchestrator is to the right of the main lane"
    );
    assert_eq!(rig.app.model.selected(), Some("api"));
    rig.click_word("F full");
    let full = text(&rig.frame(160, 45));
    assert!(full.contains(PANEL) && !full.contains(MAIN), "{full}");
    assert!(full.contains("F side"));
    rig.click_word("F side");
    let side = text(&rig.frame(160, 45));
    assert!(side.contains(MAIN) && side.contains(PANEL));
    let before_hide = rig.frame(160, 45);
    let (x, y) = cell(&before_hide, "◆").unwrap();
    rig.click(x, y);
    let hidden = text(&rig.frame(160, 45));
    assert!(hidden.contains(MAIN) && !hidden.contains(PANEL));
    assert_eq!(rig.app.model.selected(), Some("api"));
    assert_eq!(rig.app.entry(ORCH).map(|e| e.status), Some(Status::Running));
    rig.open();
    rig.text(b"F");
    assert!(text(&rig.frame(160, 45)).contains("F side"));
    rig.click_word("◆");
    let hidden_full = text(&rig.frame(160, 45));
    assert!(
        hidden_full.contains(MAIN) && !hidden_full.contains(PANEL),
        "the same button also hides a full-screen panel"
    );
}

#[test]
fn p2_a_narrow_side_request_falls_back_and_small_frames_never_panic() {
    let mut rig = Rig::new(Some("api"));
    rig.open();
    let narrow = text(&rig.frame(90, 20));
    assert!(
        narrow.contains(PANEL) && !narrow.contains(MAIN) && narrow.contains("F side"),
        "{narrow}"
    );
    assert!(
        narrow.contains("side needs"),
        "fallback explains its width requirement: {narrow}"
    );
    for (width, height) in [(160, 45), (120, 30), (90, 20), (40, 8), (1, 1), (0, 0)] {
        let _ = rig.frame(width, height); // Rendering even an empty frame must not panic.
    }
}

#[test]
fn p2_key_modes_literal_text_and_both_escape_orders_preserve_two_drafts() {
    let mut rig = Rig::new(Some("api"));
    rig.key(Key::Enter);
    rig.text(b"main draft");
    rig.key(Key::Escape);
    rig.open();
    rig.key(Key::Enter);
    rig.text(b"OF");
    rig.key(Key::Pasted(b" O F pasted".to_vec()));
    assert_eq!(
        rig.draft(ORCH, OID),
        "OF O F pasted",
        "composer owns command letters and paste"
    );
    assert_eq!(rig.draft("api", ID), "main draft");
    rig.key(Key::Escape);
    let browsing = text(&rig.frame(160, 45));
    assert!(browsing.contains(PANEL), "first Esc only leaves writing");
    assert!(!rig.app.composing());
    rig.text(b"F");
    let full = text(&rig.frame(160, 45));
    assert!(full.contains(PANEL) && !full.contains(MAIN) && full.contains("F side"));
    rig.text(b"F");
    assert!(text(&rig.frame(160, 45)).contains(MAIN));
    rig.key(Key::Escape);
    assert!(
        !text(&rig.frame(160, 45)).contains(PANEL),
        "second Esc closes the focused panel"
    );
    assert_eq!(rig.draft(ORCH, OID), "OF O F pasted");
    rig.key(Key::Enter);
    assert_eq!(
        rig.app.writer.as_ref().map(|w| w.console.name()),
        Some("api")
    );
    assert_eq!(rig.draft("api", ID), "main draft");
}

#[test]
fn p3_click_focus_ends_the_old_entry_and_composer_click_enters_the_main_column() {
    let mut rig = Rig::new(Some("api"));
    rig.open();
    rig.key(Key::Enter);
    rig.text(b"panel kept");
    let previous = rig.app.writer.as_ref().unwrap().entry;
    // The native renderer's address is the exact visible composer target.
    let buffer = rig.frame(160, 45);
    let (main_x, _) = cell(&buffer, MAIN).unwrap();
    let (orch_x, _) = cell(&buffer, PANEL).unwrap();
    let main_address = (0..44)
        .rev()
        .find(|y| {
            let row: String = (main_x..orch_x).map(|x| buffer[(x, *y)].symbol()).collect();
            row.contains("api")
        })
        .expect("main composer address names its own session");
    rig.click(main_x, main_address);
    assert_eq!(
        rig.app.writer.as_ref().map(|w| w.console.name()),
        Some("api")
    );
    assert_ne!(rig.app.writer.as_ref().unwrap().entry, previous);
    rig.text(b"main kept");
    assert_eq!(rig.draft("api", ID), "main kept");
    assert_eq!(rig.draft(ORCH, OID), "panel kept");
    rig.key(Key::Escape);
    rig.key(Key::Escape);
    assert!(
        text(&rig.frame(160, 45)).contains(PANEL),
        "main-focused Esc does not close the panel"
    );
    rig.click_word(PANEL);
    assert!(!rig.app.composing(), "body focus does not enter a composer");
    rig.key(Key::Enter);
    assert_eq!(
        rig.app.writer.as_ref().map(|w| w.console.name()),
        Some(ORCH)
    );
    assert_eq!(rig.draft(ORCH, OID), "panel kept");
}

#[test]
fn p3_a_key_stamped_before_the_new_panel_entry_cannot_cross_into_its_draft() {
    let mut rig = Rig::new(Some("api"));
    rig.open();
    let old = Instant::now();
    rig.key(Key::Enter);
    let entered = rig.app.entered.as_ref().unwrap().at;
    assert!(old < entered, "the stale key precedes this entry");
    assert!(take_keys(&mut rig.app, vec![(Key::Text(b"old column".to_vec()), old)]).is_some());
    assert_eq!(rig.draft(ORCH, OID), "");
    rig.text(b"new column");
    assert_eq!(rig.draft(ORCH, OID), "new column");
}

#[test]
fn p3_focus_has_a_visible_monochrome_cue_and_an_overlay_keeps_its_keys() {
    let mut rig = Rig::new(Some("api"));
    for name in ["api", ORCH] {
        let mut look = crate::theme::Look::DEFAULT;
        look.drawn = false;
        rig.app.looks.insert(name.to_owned(), (Some(look), None));
    }
    rig.open();
    let before = rig.frame(160, 45);
    let (_, header_y) = cell(&before, "orchestrator").expect("named panel header");
    let main_x = cell(&before, MAIN).unwrap().0;
    let panel_x = cell(&before, PANEL).unwrap().0;
    let middle = main_x + (panel_x - main_x) / 2;
    assert!(
        (middle..160).any(|x| before[(x, header_y)].symbol().contains('▸')),
        "theme off must identify the focused panel header"
    );
    rig.click_word(MAIN);
    let main_focused = rig.frame(160, 45);
    assert!(
        (main_x.saturating_sub(5)..middle)
            .any(|x| main_focused[(x, header_y)].symbol().contains('▸')),
        "the visible focus cue must transfer to the main header"
    );
    assert!(
        !(middle..160).any(|x| main_focused[(x, header_y)].symbol().contains('▸')),
        "the unfocused panel must lose its cue"
    );
    rig.app.open_settings();
    let _ = rig.frame(160, 45);
    rig.text(b"OF");
    assert!(
        rig.app.model.settings_open(),
        "the overlay owns O/F before the panel"
    );
    rig.key(Key::Escape);
    assert!(!rig.app.model.settings_open());
    assert!(text(&rig.frame(160, 45)).contains(PANEL));
}

#[test]
fn p3_two_visible_lanes_refresh_without_changing_each_others_selected_view() {
    let mut rig = Rig::new(Some("api"));
    rig.open();
    rig.app.answer(view("api", ID, 101, "MAIN_NEW"));
    rig.app.answer(view(ORCH, OID, 101, "ORCH_NEW"));
    let shown = text(&rig.frame(160, 45));
    assert!(
        shown.contains("MAIN_NEW") && shown.contains("ORCH_NEW"),
        "{shown}"
    );
    // Main focus follows its drawn body; the digit selects b while panel state survives.
    rig.click_word("MAIN_NEW");
    rig.text(b"2");
    assert_eq!(rig.app.model.selected(), Some("b"));
    rig.app.answer(view("b", ID, 102, "B_NEW"));
    let shown = text(&rig.frame(160, 45));
    assert!(
        shown.contains("B_NEW") && shown.contains("ORCH_NEW"),
        "{shown}"
    );
}

#[test]
fn p3_panel_scroll_is_independent_of_the_main_column_and_survives_selection_changes() {
    let mut rig = Rig::new(Some("api"));
    rig.open();
    let mut lane = Lane::default();
    for n in 0..80 {
        lane.items
            .push(said(&format!("ORCH_ROW_{n:02}"), i64::from(n)));
    }
    rig.app.answer(Answer::View(ViewRead {
        name: ORCH.to_owned(),
        id: OID.to_owned(),
        seq: 105,
        lane,
        needs: None,
        roster: None,
    }));
    let initial = rig.frame(160, 45);
    let panel_body_x = cell(&initial, "ORCH_ROW_79").unwrap().0;
    let main_x = cell(&initial, MAIN).unwrap().0;
    // Turn bodies are indented. Their x is not the column boundary.
    // Find the actual continuous vertical border, independently of the hit map.
    let split = (main_x..panel_body_x)
        .rev()
        .find(|x| {
            (2..initial.area.height.saturating_sub(5))
                .all(|y| matches!(initial[(*x, y)].symbol(), "│" | "|"))
        })
        .expect("a visible border separates the two native columns");
    assert!(main_x < split && split < panel_body_x);
    let main_before = body(&initial, main_x, split.saturating_sub(main_x));
    rig.key(Key::PageUp);
    let scrolled = rig.frame(160, 45);
    assert_ne!(
        body(&initial, split, 160 - split),
        body(&scrolled, split, 160 - split)
    );
    assert_eq!(
        body(&scrolled, main_x, split.saturating_sub(main_x)),
        main_before,
        "panel scrolling never scrolls the main lane"
    );
    let panel_before = body(&scrolled, split, 160 - split);
    rig.click_word(MAIN);
    rig.text(b"2");
    assert_eq!(rig.app.model.selected(), Some("b"));
    let changed = rig.frame(160, 45);
    assert_eq!(
        body(&changed, split, 160 - split),
        panel_before,
        "a main selection does not reset panel scroll"
    );
}

#[test]
fn p3_panel_reader_interest_is_additive_and_reaches_earlier_conversations() {
    let mut rig = Rig::new(Some("api"));
    let meta = rig.root.0.join("sessions").join(ORCH).join("meta");
    let mut bytes = std::fs::read_to_string(&meta).unwrap();
    bytes.push_str("harness_session_prior.main=claude:0199c0de-bbbb-4890-abcd-ef0123456789\n");
    std::fs::write(&meta, bytes).unwrap();
    rig.refresh(&[]);
    rig.open();
    let mut coverage = Vec::new();
    for _ in 0..8 {
        if let Some(Answer::View(read)) = rig.reader.stage(ORCH) {
            coverage = read.lane.coverage;
        }
    }
    assert!(
        !coverage.iter().any(|s| s.contains("loading:")),
        "{coverage:?}"
    );
    assert!(
        coverage.iter().any(|s| s.contains("predecessor 1")),
        "the visible unselected panel must actually read its earlier stage: {coverage:?}"
    );
    // Focus request still names the ordinary selected session, independently.
    rig.app.answer(view("api", ID, 201, "MAIN_STILL_CURRENT"));
    assert_eq!(rig.app.model.selected(), Some("api"));
    assert!(text(&rig.frame(160, 45)).contains("MAIN_STILL_CURRENT"));
    rig.text(b"O");
    rig.app.evict();
    assert!(
        !rig.app.shown.contains_key(ORCH),
        "closing releases the hidden panel lane cache"
    );
    rig.app.answer(view(ORCH, OID, 500, "LATE_CLOSED"));
    assert!(!text(&rig.frame(160, 45)).contains("LATE_CLOSED"));
    assert!(
        !rig.app.shown.contains_key(ORCH),
        "a late closed-panel answer is dropped"
    );
}

#[test]
fn p3_a_conflicting_live_uuid_with_unchanged_meta_withholds_every_panel_record() {
    let mut rig = Rig::new(Some("api"));
    rig.open();
    rig.key(Key::Enter);
    rig.text(b"old draft");
    rig.command(&format!(
        "set-option -t =orchestrator: @ae_session_uuid {NEW}"
    ));
    rig.refresh(&[ORCH]);
    let shown = text(&rig.frame(160, 45));
    assert!(shown.contains("orchestrator unproven:"), "{shown}");
    assert!(
        !shown.contains(PANEL),
        "the old lane cannot leak through a live/meta conflict"
    );
    assert!(!rig.app.composing());
    assert_eq!(rig.draft(ORCH, OID), "old draft");
    rig.assert_no_delivery();
    assert_eq!(rig.draft(ORCH, OID), "old draft");
}

#[test]
fn p3_a_replacement_never_rebinds_an_open_panel_or_a_late_lane() {
    let mut rig = Rig::new(Some("api"));
    rig.open();
    rig.record(ORCH, NEW);
    rig.command(&format!(
        "set-option -t =orchestrator: @ae_session_uuid {NEW}"
    ));
    rig.refresh(&[ORCH]);
    rig.app.answer(view(ORCH, OID, 501, "OLD_LATE"));
    rig.app.answer(view(ORCH, NEW, 502, "NEW_UNOPENED"));
    let shown = text(&rig.frame(160, 45));
    assert!(
        shown.contains("orchestrator replaced; O reopens"),
        "{shown}"
    );
    assert!(
        !shown.contains("OLD_LATE") && !shown.contains("NEW_UNOPENED") && !shown.contains(PANEL)
    );
    rig.text(b"!");
    rig.app.answer(view(ORCH, NEW, 503, "NEW_STILL_UNOPENED"));
    assert!(
        !text(&rig.frame(160, 45)).contains("NEW_STILL_UNOPENED"),
        "! never re-pins"
    );
    rig.assert_no_delivery();
    rig.click_word("◆");
    assert!(
        !text(&rig.frame(160, 45)).contains("orchestrator replaced; O reopens"),
        "the same button hides a replaced panel"
    );
    rig.text(b"O");
    rig.app.answer(view(ORCH, NEW, 504, "NEW_REOPENED"));
    assert!(
        text(&rig.frame(160, 45)).contains("NEW_REOPENED"),
        "explicit reopening admits the new uuid"
    );
}

#[test]
fn p3_an_open_with_gap_panel_never_pins_a_later_recovery_until_reopened() {
    let mut rig = Rig::new(Some("api"));
    rig.command("set-option -u -t =orchestrator: @ae_session_uuid");
    rig.refresh(&[]);
    rig.text(b"O");
    let gap = text(&rig.frame(160, 45));
    assert!(gap.contains("orchestrator unproven:"), "{gap}");
    rig.command(&format!(
        "set-option -t =orchestrator: @ae_session_uuid {OID}"
    ));
    rig.refresh(&[ORCH]);
    rig.lanes();
    assert!(
        !text(&rig.frame(160, 45)).contains(PANEL),
        "recovery cannot invent an opening pin"
    );
    rig.assert_no_delivery();
    rig.key(Key::Escape);
    rig.text(b"O");
    rig.lanes();
    assert!(text(&rig.frame(160, 45)).contains(PANEL));
}

#[test]
fn p3_no_running_orchestrator_has_one_inert_line_and_never_starts_a_session() {
    let mut rig = Rig::new(Some("api"));
    rig.command("kill-session -t =orchestrator");
    rig.refresh(&[]);
    rig.text(b"O");
    let shown = text(&rig.frame(160, 45));
    assert_eq!(
        shown.matches("no orchestrator running").count(),
        1,
        "{shown}"
    );
    rig.assert_no_delivery();
    rig.refresh(&[]);
    assert_eq!(rig.app.entry(ORCH).map(|e| e.status), Some(Status::Stopped));
}

#[test]
fn p3_an_orchestrator_home_or_handover_opens_full_without_an_ordinary_row() {
    for handover in [false, true] {
        let mut rig = Rig::new(if handover { None } else { Some(ORCH) });
        if handover {
            rig.app = App::new(
                None,
                Some(rig.socket.server.clone()),
                None,
                Some(rig.tx.clone()),
            );
            rig.isolate();
            rig.app.answer(Answer::Handover {
                view: crate::app::handover::View {
                    session: ORCH.to_owned(),
                    id: OID.to_owned(),
                    focus: None,
                    sidebar: None,
                    list: None,
                    pages: 0,
                    rows: 0,
                    list_top: None,
                    body_top: 0,
                },
                at: Timestamp::now().epoch(),
            });
            rig.refresh(&[]);
        }
        rig.lanes();
        let shown = text(&rig.frame(160, 45));
        assert!(shown.contains(PANEL) && shown.contains("F side"), "{shown}");
        assert!(!shown.contains(MAIN));
        assert!(!rig.app.fleet.rows.iter().any(|r| r.name == ORCH));
    }
}

#[test]
fn p3_submit_uses_the_panel_pair_and_keeps_the_main_draft() {
    let mut rig = Rig::new(Some("api"));
    rig.key(Key::Enter);
    rig.text(b"main untouched");
    rig.key(Key::Escape);
    rig.open();
    let (dispatch, received) = mpsc::channel();
    rig.app.carry = Arc::new(move |ask| {
        dispatch
            .send((ask.seat().to_owned(), ask.body().to_owned()))
            .unwrap();
        "notice UNCONFIRMED: fixture external delivery sink".to_owned()
    });
    rig.key(Key::Enter);
    rig.text(b"orchestrator question");
    rig.key(Key::Enter);
    assert!(
        rig.app.deliver(),
        "the actual queued admission starts a delivery"
    );
    let (seat, question) = received
        .recv_timeout(Duration::from_secs(3))
        .expect("panel delivery");
    assert_eq!(
        seat, "olead",
        "the main session lead must never receive panel input"
    );
    assert_eq!(question, "orchestrator question");
    assert_eq!(rig.draft("api", ID), "main untouched");
    let until = Instant::now() + Duration::from_secs(3);
    while rig.app.flight.is_some() && Instant::now() < until {
        let _ = rig.app.land();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(rig.app.flight.is_none(), "the test delivery is reaped");
}

#[test]
fn p3_a_busy_submit_lease_keeps_the_panel_draft_and_retries_after_release() {
    let mut rig = Rig::new(Some("api"));
    rig.open();
    let (dispatch, received) = mpsc::channel();
    rig.app.carry = Arc::new(move |ask| {
        dispatch.send(ask.seat().to_owned()).unwrap();
        "notice UNCONFIRMED: fixture external delivery sink".to_owned()
    });
    let lease = crate::store::open(&rig.root.0.join("sessions").join(ORCH))
        .console_writer()
        .unwrap();
    rig.key(Key::Enter);
    assert!(
        rig.app.composing(),
        "another submit lease does not forbid composition"
    );
    rig.text(b"retry me");
    rig.key(Key::Enter);
    let _ = rig.app.deliver();
    assert!(received.try_recv().is_err());
    assert_eq!(rig.draft(ORCH, OID), "retry me");
    assert!(
        rig.app
            .notices
            .iter()
            .any(|(name, item)| name == ORCH && item.body.contains("refused:"))
    );
    drop(lease);
    rig.key(Key::Enter);
    let _ = rig.app.deliver();
    assert_eq!(
        received.recv_timeout(Duration::from_secs(3)).unwrap(),
        "olead"
    );
    let until = Instant::now() + Duration::from_secs(3);
    while rig.app.flight.is_some() && Instant::now() < until {
        let _ = rig.app.land();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(rig.app.flight.is_none());
}

#[test]
fn p2_the_button_remains_mouse_accessible_with_icons_off() {
    let mut rig = Rig::new(Some("api"));
    for name in ["api", ORCH] {
        let mut look = crate::theme::Look::DEFAULT;
        look.icons = false;
        rig.app.looks.insert(name.to_owned(), (Some(look), None));
    }
    let buffer = rig.frame(160, 45);
    let x = (0..160)
        .rev()
        .find(|x| buffer[(*x, 44)].symbol() == "O")
        .expect("icons off still has the dedicated O control beside the gear");
    rig.click(x, 44);
    rig.lanes();
    assert!(text(&rig.frame(160, 45)).contains(PANEL));
    rig.click_word("F full");
    assert!(text(&rig.frame(160, 45)).contains("F side"));
}

#[test]
fn p3_needy_navigation_from_full_restores_side_and_focuses_the_main_target() {
    let mut rig = Rig::new(Some("api"));
    rig.refresh(&[ORCH, "b", "c"]);
    rig.open();
    rig.text(b"F");
    assert!(!text(&rig.frame(160, 45)).contains(MAIN));
    rig.text(b"!");
    rig.app.answer(view("b", ID, 301, "B_SIDE"));
    let shown = text(&rig.frame(160, 45));
    assert_eq!(rig.app.model.selected(), Some("b"));
    assert!(
        shown.contains("B_SIDE") && shown.contains(PANEL) && shown.contains("F full"),
        "{shown}"
    );
    rig.key(Key::Enter);
    assert_eq!(rig.app.writer.as_ref().map(|w| w.console.name()), Some("b"));
}

#[test]
fn p3_proof_loss_on_the_unfocused_panel_never_cancels_a_main_writer() {
    let mut rig = Rig::new(Some("api"));
    rig.open();
    rig.click_word(MAIN);
    rig.key(Key::Enter);
    rig.text(b"main still writing");
    assert_eq!(
        rig.app.writer.as_ref().map(|w| w.console.name()),
        Some("api")
    );
    rig.command(&format!(
        "set-option -t =orchestrator: @ae_session_uuid {NEW}"
    ));
    rig.refresh(&[]);
    rig.text(b" continues");
    assert_eq!(rig.draft("api", ID), "main still writing continues");
    assert_eq!(
        rig.app.writer.as_ref().map(|w| w.console.name()),
        Some("api")
    );
    assert!(text(&rig.frame(160, 45)).contains("orchestrator unproven:"));
}

#[test]
fn p3_panel_view_commands_do_not_open_a_terminal_or_a_seat() {
    let mut rig = Rig::new(Some("api"));
    rig.open();
    rig.text(b"voo");
    assert!(!rig.app.model.peek_open());
    assert!(rig.app.pane_view.is_none());
    assert_eq!(rig.app.model.selected(), Some("api"));
    assert!(text(&rig.frame(160, 45)).contains(PANEL));
}

#[test]
fn p3_r_on_a_panel_turn_opens_the_existing_reader_in_the_main_column() {
    let mut rig = Rig::new(Some("api"));
    rig.open();
    let path = rig.root.0.join("panel-note.rs");
    let link = format!("open {}:1", path.display());
    rig.app.answer(view(ORCH, OID, 301, &link));
    let _ = rig.frame(160, 45);
    rig.text(b"[");
    assert!(
        take_keys(
            &mut rig.app,
            vec![(Key::Text(b"r".to_vec()), Instant::now())],
        )
        .is_some()
    );
    let mut requested = None;
    for ask in rig.rx.try_iter() {
        match ask {
            loader::Request::Read(Some(ask)) => {
                assert!(requested.is_none(), "one reader request per open");
                requested = Some(ask);
            }
            other => rig.reader.take(other),
        }
    }
    let ask = requested.expect("r must read the marked panel turn's file");
    assert_eq!(ask.path, path.to_str().unwrap());
    assert_eq!(ask.line, Some(1));
    rig.app.answer(Answer::Read {
        generation: ask.generation,
        body: crate::app::reader::Body::Text {
            lines: vec!["PANEL_FILE_CONTENT".to_owned()],
            size: 0,
        },
    });
    let opened = rig.frame(160, 45);
    assert!(
        rig.app.model.reader_open(),
        "the existing reader owns the read"
    );
    let file_x = cell(&opened, "PANEL_FILE_CONTENT")
        .expect("file body in main")
        .0;
    let panel_x = cell(&opened, "open ").expect("panel lane stays visible").0;
    assert!(
        file_x < panel_x,
        "r draws the file beside the panel, in main"
    );
    assert_eq!(rig.app.model.selected(), Some("api"));
    rig.key(Key::Escape);
    assert!(!rig.app.model.reader_open());
    assert!(text(&rig.frame(160, 45)).contains("panel-note.rs"));
    rig.key(Key::Enter);
    assert_eq!(
        rig.app.writer.as_ref().map(|w| w.console.name()),
        Some("api")
    );
}

#[test]
fn p1_unread_cues_and_fleet_order_never_reintroduce_the_orchestrator() {
    let mut rig = Rig::new(Some("api"));
    let now = Timestamp::now().epoch();
    for entry in &mut rig.app.world.sessions {
        entry.answer_epoch = Some(now - 100);
    }
    let _ = rig.frame(160, 45);
    for entry in &mut rig.app.world.sessions {
        if matches!(entry.name.as_str(), ORCH | "b") {
            entry.answer_epoch = Some(now + 3600);
        }
    }
    let _ = rig.frame(160, 45);
    assert!(
        rig.app.model.is_unread("b"),
        "an ordinary row receives its unread cue"
    );
    assert!(
        !rig.app.model.is_unread(ORCH),
        "a hidden orchestrator never has a list cue"
    );
    let empty = std::collections::BTreeMap::new();
    let ordered = fleet::rows(
        &rig.app.world,
        &empty,
        &std::collections::BTreeMap::new(),
        &crate::theme::FleetOrder::from_validated(
            [ORCH, "c", "b", "api"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        ),
        &std::collections::BTreeMap::new(),
        Some("api"),
        Timestamp::now(),
    );
    assert_eq!(
        ordered
            .rows
            .iter()
            .map(|row| row.name.as_str())
            .collect::<Vec<_>>(),
        ["c", "b", "api"],
        "an explicit orchestrator order pin cannot create a session row",
    );
}

#[test]
fn p2_narrowing_keeps_the_main_writer_and_its_draft_on_screen() {
    let mut rig = Rig::new(Some("api"));
    rig.open();
    rig.click_word(MAIN);
    rig.key(Key::Enter);
    rig.text(b"MAIN_NARROW_DRAFT");
    assert_eq!(
        rig.app.writer.as_ref().map(|w| w.console.name()),
        Some("api")
    );
    let narrow = text(&rig.frame(90, 20));
    assert!(
        narrow.contains(MAIN) && narrow.contains("MAIN_NARROW_DRAFT"),
        "the column that owns a writing entry must stay visible: {narrow}"
    );
    assert!(
        !narrow.contains(PANEL),
        "a panel that cannot fit must not cover the main writer: {narrow}"
    );
    assert!(
        rig.app.composing(),
        "narrowing does not end the visible writer"
    );
    rig.text(b"_CONTINUED");
    let continued = text(&rig.frame(90, 20));
    assert!(
        continued.contains("MAIN_NARROW_DRAFT_CONTINUED"),
        "new keys must appear in the visible draft: {continued}"
    );
    assert_eq!(rig.draft("api", ID), "MAIN_NARROW_DRAFT_CONTINUED");
    assert_eq!(rig.draft(ORCH, OID), "");
    let wide = text(&rig.frame(160, 45));
    assert!(
        wide.contains(MAIN) && wide.contains(PANEL) && wide.contains("F full"),
        "widening restores the still-open side panel: {wide}"
    );
    assert!(rig.app.composing());
    assert_eq!(
        rig.app.writer.as_ref().map(|w| w.console.name()),
        Some("api")
    );
    assert!(rig.deliveries.lock().unwrap().is_empty());
}

#[test]
fn p2_needy_navigation_at_narrow_width_shows_the_column_that_receives_the_keys() {
    let mut rig = Rig::new(Some("api"));
    rig.refresh(&[ORCH, "b"]);
    rig.open();
    assert!(text(&rig.frame(90, 20)).contains(PANEL));
    rig.text(b"!");
    assert_eq!(rig.app.model.selected(), Some("b"));
    rig.app.answer(view("b", ID, 301, "B_NARROW_TARGET"));
    let main = text(&rig.frame(90, 20));
    assert!(
        main.contains("B_NARROW_TARGET") && !main.contains(PANEL),
        "! must show its ordinary target instead of covering it: {main}"
    );
    rig.key(Key::Enter);
    assert_eq!(rig.app.writer.as_ref().map(|w| w.console.name()), Some("b"));
    rig.text(b"B_NARROW_DRAFT");
    assert!(text(&rig.frame(90, 20)).contains("B_NARROW_DRAFT"));
    rig.key(Key::Escape);
    rig.text(b"!");
    let panel = text(&rig.frame(90, 20));
    assert!(
        panel.contains(PANEL) && !panel.contains("B_NARROW_TARGET"),
        "! must also show its orchestrator target: {panel}"
    );
    assert_eq!(rig.app.model.selected(), Some("b"));
    rig.key(Key::Enter);
    assert_eq!(
        rig.app.writer.as_ref().map(|w| w.console.name()),
        Some(ORCH)
    );
    rig.text(b"ORCH_NARROW_DRAFT");
    assert!(text(&rig.frame(90, 20)).contains("ORCH_NARROW_DRAFT"));
    assert_eq!(rig.draft("b", ID), "B_NARROW_DRAFT");
    assert!(rig.deliveries.lock().unwrap().is_empty());
}

#[test]
fn p2_r_on_a_panel_turn_at_narrow_width_shows_the_main_reader() {
    let mut rig = Rig::new(Some("api"));
    rig.open();
    let path = rig.root.0.join("narrow-panel-note.rs");
    let link = format!("PANEL_NARROW_LINK\nopen {}:1", path.display());
    rig.app.answer(view(ORCH, OID, 301, &link));
    let panel = text(&rig.frame(90, 20));
    assert!(panel.contains("PANEL_NARROW_LINK"), "{panel}");
    rig.text(b"[");
    assert!(
        take_keys(
            &mut rig.app,
            vec![(Key::Text(b"r".to_vec()), Instant::now())],
        )
        .is_some()
    );
    let mut requested = None;
    for ask in rig.rx.try_iter() {
        match ask {
            loader::Request::Read(Some(ask)) => {
                assert!(requested.is_none(), "one reader request per open");
                requested = Some(ask);
            }
            other => rig.reader.take(other),
        }
    }
    let ask = requested.expect("r must read the marked panel turn's file");
    assert_eq!(ask.path, path.to_str().unwrap());
    assert_eq!(ask.line, Some(1));
    rig.app.answer(Answer::Read {
        generation: ask.generation,
        body: crate::app::reader::Body::Text {
            lines: vec!["NARROW_FILE_CONTENT".to_owned()],
            size: 0,
        },
    });
    let opened = text(&rig.frame(90, 20));
    assert!(rig.app.model.reader_open());
    assert!(
        opened.contains("NARROW_FILE_CONTENT") && !opened.contains("PANEL_NARROW_LINK"),
        "r must uncover its main-column reader target: {opened}"
    );
    assert_eq!(rig.app.model.selected(), Some("api"));
    rig.key(Key::Escape);
    assert!(!rig.app.model.reader_open());
    let main = text(&rig.frame(90, 20));
    assert!(
        main.contains(MAIN) && !main.contains("PANEL_NARROW_LINK"),
        "closing the reader leaves its main column visible: {main}"
    );
    rig.key(Key::Enter);
    assert_eq!(
        rig.app.writer.as_ref().map(|w| w.console.name()),
        Some("api")
    );
    rig.text(b"AFTER_NARROW_READER");
    assert!(text(&rig.frame(90, 20)).contains("AFTER_NARROW_READER"));
    assert!(rig.deliveries.lock().unwrap().is_empty());
}

#[test]
fn p2_widening_restores_the_open_side_panel_after_either_column_covered_the_frame() {
    let mut rig = Rig::new(Some("api"));
    rig.open();
    rig.key(Key::Enter);
    rig.text(b"PANEL_WIDTH_DRAFT");
    rig.key(Key::Escape);
    let narrow_panel = text(&rig.frame(90, 20));
    assert!(
        narrow_panel.contains(PANEL)
            && !narrow_panel.contains(MAIN)
            && narrow_panel.contains("F side")
            && narrow_panel.contains("side needs"),
        "{narrow_panel}"
    );
    let first_wide = rig.frame(160, 45);
    assert!(cell(&first_wide, MAIN).unwrap().0 < cell(&first_wide, PANEL).unwrap().0);
    assert!(text(&first_wide).contains("F full"));
    rig.click_word(MAIN);
    let narrow_main = text(&rig.frame(90, 20));
    assert!(
        narrow_main.contains(MAIN) && !narrow_main.contains(PANEL),
        "a narrow frame follows main focus as well: {narrow_main}"
    );
    assert!(!rig.app.composing(), "focus alone creates no writer");
    let second_wide = rig.frame(160, 45);
    assert!(
        cell(&second_wide, MAIN).unwrap().0 < cell(&second_wide, PANEL).unwrap().0,
        "covering the panel does not close it"
    );
    assert!(text(&second_wide).contains("F full"));
    rig.key(Key::Enter);
    assert_eq!(
        rig.app.writer.as_ref().map(|w| w.console.name()),
        Some("api"),
        "widening does not steal main focus"
    );
    rig.key(Key::Escape);
    rig.click_word(PANEL);
    rig.key(Key::Enter);
    assert_eq!(
        rig.app.writer.as_ref().map(|w| w.console.name()),
        Some(ORCH)
    );
    assert_eq!(rig.draft(ORCH, OID), "PANEL_WIDTH_DRAFT");
}

#[test]
fn p3_a_panel_press_from_an_older_frame_never_targets_the_replacement_turn() {
    // Run the same event sequence against main and panel. Both must preserve
    // the target of the frame on which the press was made, or refuse it.
    for panel in [false, true] {
        let mut rig = Rig::new(Some("api"));
        if panel {
            rig.open();
        }
        let name = if panel { ORCH } else { "api" };
        let uuid = if panel { OID } else { ID };
        let old_path = rig.root.0.join("old-click.rs");
        let new_path = rig.root.0.join("new-click.rs");
        let old_body = format!("OLD_CLICK_TARGET\nopen {}:1", old_path.display());
        let new_body = format!("NEW_CLICK_TARGET\nopen {}:1", new_path.display());
        rig.app.answer(view(name, uuid, 401, &old_body));
        let old = rig.frame(160, 45);
        let (x, y) = cell(&old, "OLD_CLICK_TARGET").expect("old turn is drawn");
        let origin = Instant::now();
        rig.app.answer(view(name, uuid, 402, &new_body));
        let current = rig.frame(160, 45);
        assert_eq!(
            cell(&current, "NEW_CLICK_TARGET"),
            Some((x, y)),
            "the replacement must occupy exactly the old press cells"
        );
        assert!(!text(&current).contains("OLD_CLICK_TARGET"));
        let press = Mouse {
            kind: MouseKind::Click,
            column: x,
            row: y,
        };
        let release = Mouse {
            kind: MouseKind::Release,
            ..press
        };
        assert!(
            take_keys(
                &mut rig.app,
                vec![
                    (Key::Mouse(press), origin),
                    (Key::Mouse(release), origin),
                    (Key::Text(b"r".to_vec()), Instant::now()),
                ],
            )
            .is_some()
        );
        let mut stale_reads = Vec::new();
        for ask in rig.rx.try_iter() {
            match ask {
                loader::Request::Read(Some(ask)) => stale_reads.push(ask.path),
                other => rig.reader.take(other),
            }
        }
        assert!(
            stale_reads
                .iter()
                .all(|path| path == old_path.to_str().unwrap()),
            "a delayed press must refuse or retain its old target, never the replacement: panel={panel}, reads={stale_reads:?}"
        );
        assert_eq!(rig.app.model.selected(), Some("api"));
        if rig.app.model.reader_open() {
            rig.key(Key::Escape);
        }
        // A refusal of every press cannot satisfy the contract: a fresh
        // press at the same visible turn must still open its actual file.
        rig.click_word("NEW_CLICK_TARGET");
        assert!(
            take_keys(
                &mut rig.app,
                vec![(Key::Text(b"r".to_vec()), Instant::now())],
            )
            .is_some()
        );
        let mut fresh_reads = Vec::new();
        for ask in rig.rx.try_iter() {
            match ask {
                loader::Request::Read(Some(ask)) => fresh_reads.push(ask.path),
                other => rig.reader.take(other),
            }
        }
        assert_eq!(
            fresh_reads,
            [new_path.to_str().unwrap()],
            "a fresh press must select the currently drawn turn: panel={panel}"
        );
        assert!(rig.deliveries.lock().unwrap().is_empty());
    }
}
