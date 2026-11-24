//! The durable view through the app's own paths: the dispatch of a key, the
//! answers around it and the quit. The writer thread's own pacing is pinned in
//! `saved`; these tests drive `drain`, `persist_view` and `persist_final_view`
//! as the loop and the exits do.
#![allow(clippy::expect_used, reason = "owned fixtures")]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::fleet::{Counts, Fleet, Line2, Row};
use super::handover;
use super::handover::{OrchestratorView, PanelMode};
use super::loader::{Answer, FleetRead, Proof, Reader, Wake};
use super::panel::Opening;
use super::saved::Saver;
use super::tests::{Root, entry, session};
use super::{App, Keep, drain};
use crate::console::input::{Key, Keys};
use crate::digest::Status;
use crate::listing::World;
use crate::theme::{FleetOrder, Mark};
use crate::time::Timestamp;

type Log = Arc<Mutex<Vec<String>>>;

fn read(root: &Root, names: &[&str]) -> FleetRead {
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
    read.scanned = true;
    read
}

/// A sink that records every text it is given.
fn recorder() -> (
    Log,
    impl Fn(&str) -> Result<(), String> + Send + Sync + 'static,
) {
    let log = Log::default();
    let seen = Arc::clone(&log);
    (log, move |text: &str| {
        seen.lock().expect("log").push(text.to_owned());
        Ok(())
    })
}

/// A recording sink whose first write waits until the returned sender fires.
fn gated() -> (
    Log,
    mpsc::Sender<()>,
    impl Fn(&str) -> Result<(), String> + Send + Sync + 'static,
) {
    let (open, gate) = mpsc::channel::<()>();
    let gate = Mutex::new(gate);
    let first = AtomicBool::new(true);
    let (log, record) = recorder();
    (log, open, move |text: &str| {
        if first.swap(false, Ordering::SeqCst) {
            let _ = gate.lock().expect("gate").recv();
        }
        record(text)
    })
}

/// An app that has read a two-session fleet, `api` (its home) selected, with
/// `saver` as its writer.
fn running(root: &Root, saver: Saver) -> App {
    session(root, "web", "");
    let mut app = App::new(Some("api".into()), None, None, None);
    app.saver = Some(saver);
    app.answer(Answer::Fleet(read(root, &["api", "web"])));
    assert_eq!(
        app.model.selected(),
        Some("api"),
        "the first row is selected"
    );
    app
}

/// One turn of the loop over `wakes`, through the real dispatch.
fn pump(app: &mut App, wakes: Vec<Wake>) {
    let (tell, hear) = mpsc::channel();
    for wake in wakes {
        tell.send(wake).expect("queued");
    }
    assert!(drain(app, &mut Keys::app(), &hear, &mut None).is_some());
}

fn newest(log: &Log) -> handover::Handover {
    let written = log.lock().expect("log").clone();
    let text = written.last().expect("a view was written");
    handover::parse_file(text.as_bytes()).expect("a canonical view")
}

#[test]
fn a_first_change_whose_write_is_still_running_is_owed_its_final_write() {
    let root = Root::new("saved-owed");
    let (log, open, sink) = gated();
    let mut app = running(&root, Saver::start(sink, |_| {}));
    pump(&mut app, vec![Wake::Keys(Instant::now(), b"2".to_vec())]);
    assert_eq!(app.model.selected(), Some("web"), "the key picked row 2");
    app.persist_view(Instant::now());
    assert!(
        log.lock().expect("log").is_empty(),
        "the first write is still running"
    );
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        let _ = open.send(());
    });
    assert_eq!(app.persist_final_view(), Ok(()));
    assert_eq!(
        newest(&log).view.session,
        "web",
        "the quit waited for a published view"
    );
    release.join().expect("release");
}

#[test]
fn a_key_that_changes_nothing_does_not_own_a_change_the_fleet_made() {
    let root = Root::new("saved-attribution");
    let (log, sink) = recorder();
    let mut app = running(&root, Saver::start(sink, |_| {}));
    let after = read(&root, &["web"]);
    pump(
        &mut app,
        vec![
            Wake::Keys(Instant::now(), b"w".to_vec()),
            Wake::Answer(Box::new(Answer::Fleet(after))),
        ],
    );
    assert_eq!(
        app.model.selected(),
        Some("web"),
        "the fleet moved the selection"
    );
    assert!(app.touched, "a key was read");
    app.persist_view(Instant::now());
    assert_eq!(app.persist_final_view(), Ok(()));
    // A write handed to the thread lands within microseconds; give it far more.
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        log.lock().expect("log").is_empty(),
        "no human changed the view, so nothing is written"
    );
}

#[test]
fn a_key_held_from_an_earlier_turn_counts_when_it_is_replayed() {
    let root = Root::new("saved-deferred");
    let (log, sink) = recorder();
    let mut app = running(&root, Saver::start(sink, |_| {}));
    app.deferred
        .push((Key::Text(b"2".to_vec()), Instant::now()));
    pump(&mut app, Vec::new());
    assert_eq!(
        app.model.selected(),
        Some("web"),
        "the held key picked row 2"
    );
    assert_eq!(app.persist_final_view(), Ok(()));
    assert_eq!(newest(&log).view.session, "web");
}

#[test]
fn a_key_read_before_the_first_fleet_counts_when_the_fleet_replays_it() {
    let root = Root::new("saved-early");
    session(&root, "web", "");
    let (log, sink) = recorder();
    let mut app = App::new(Some("api".into()), None, None, None);
    app.saver = Some(Saver::start(sink, |_| {}));
    pump(&mut app, vec![Wake::Keys(Instant::now(), b"2".to_vec())]);
    assert!(!app.fleeted, "no fleet was read yet, so the key waits");
    app.answer(Answer::Fleet(read(&root, &["api", "web"])));
    assert_eq!(
        app.model.selected(),
        Some("web"),
        "the fleet replayed the key"
    );
    assert_eq!(app.persist_final_view(), Ok(()));
    assert_eq!(newest(&log).view.session, "web");
}

/// A stderr that refuses every byte, as one that was closed.
struct Closed;

impl std::io::Write for Closed {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }
}

/// An app whose one change is owed its final write, and whose sink records the
/// write and then fails it.
fn owing_a_failing_write(tag: &str) -> (Log, App) {
    let root = Root::new(tag);
    let (log, record) = recorder();
    let fail = move |text: &str| record(text).and(Err("disk full".to_owned()));
    let mut app = running(&root, Saver::start(fail, |_| {}));
    pump(&mut app, vec![Wake::Keys(Instant::now(), b"2".to_vec())]);
    assert_eq!(app.model.selected(), Some("web"), "the key picked row 2");
    (log, app)
}

#[test]
fn a_failed_final_save_is_said_once_when_the_leave_can_speak() {
    let (log, mut app) = owing_a_failing_write("saved-leave-says");
    let mut said = Vec::new();
    super::run::leave_the_view(&mut app, &mut said);
    let said = String::from_utf8(said).expect("text");
    assert_eq!(log.lock().expect("log").len(), 1, "the final write ran");
    assert_eq!(
        said.matches("could not save the view: disk full").count(),
        1
    );
}

#[test]
fn a_failed_final_save_and_a_closed_stderr_never_abort_the_leave() {
    let (log, mut app) = owing_a_failing_write("saved-leave-closed");
    // A diagnostic nobody can read never aborts a quit or a restart: the call
    // returns, whatever the stderr does, after the write was tried.
    super::run::leave_the_view(&mut app, &mut Closed);
    assert_eq!(log.lock().expect("log").len(), 1, "the final write ran");
}

#[test]
fn a_saved_leave_says_nothing() {
    let root = Root::new("saved-leave-quiet");
    let (log, record) = recorder();
    let mut app = running(&root, Saver::start(record, |_| {}));
    pump(&mut app, vec![Wake::Keys(Instant::now(), b"2".to_vec())]);
    let mut said = Vec::new();
    super::run::leave_the_view(&mut app, &mut said);
    assert!(said.is_empty(), "nothing failed, so nothing is said");
    assert_eq!(newest(&log).view.session, "web");
}

const WEB_ID: &str = "6f1c2b7e-0d3a-4c58-9a2e-5b1d7c9e0a11";
const OID: &str = "0199c0de-2222-4890-abcd-ef0123456789";
const OTHER: &str = "0199c0de-3333-4890-abcd-ef0123456789";

fn plain_view(session: &str, id: &str) -> handover::View {
    handover::View {
        session: session.to_owned(),
        id: id.to_owned(),
        focus: None,
        sidebar: None,
        list: None,
        pages: 0,
        rows: 0,
        list_top: None,
        body_top: 0,
        panel: None,
        desk: None,
        desk_hidden: false,
        desk_tab: crate::desk::Section::Decide,
        zoom: false,
        orchestrator: None,
    }
}

fn panel_view(id: &str, mode: PanelMode) -> OrchestratorView {
    OrchestratorView {
        id: id.to_owned(),
        mode,
        pages: 3,
        rows: 2,
    }
}

/// A bare app that has just proved the orchestrator `proof`, with `opening`
/// waiting for it and `before` having set the stage.
fn proving(opening: Opening, proof: &str, before: impl FnOnce(&mut App)) -> App {
    let mut app = App::new(None, None, None, None);
    before(&mut app);
    app.orch.opening = opening;
    app.proved(Proof::Proven(proof.to_owned()), Timestamp::now().epoch());
    app
}

#[test]
fn a_saved_panel_opens_in_its_mode_and_scrolls_only_on_its_own_incarnation() {
    for mode in [PanelMode::Side, PanelMode::SideMain, PanelMode::Full] {
        let own = proving(
            Opening::Saved(Some(panel_view(OID, mode)), false),
            OID,
            |_| {},
        );
        let panel = own.orch.panel.as_ref().expect("the saved panel opens");
        assert_eq!(
            panel.handover(),
            Some(panel_view(OID, mode)),
            "{mode:?}: its mode and its scroll"
        );

        let other = proving(
            Opening::Saved(Some(panel_view(OTHER, mode)), false),
            OID,
            |_| {},
        );
        let panel = other
            .orch
            .panel
            .as_ref()
            .expect("another incarnation opens");
        let mut newest = panel_view(OID, mode);
        (newest.pages, newest.rows) = (0, 0);
        assert_eq!(panel.handover(), Some(newest), "{mode:?}: its newest turn");
    }
}

#[test]
fn a_saved_panel_yields_to_a_human_who_acted_and_to_another_incarnations_panel() {
    let saved = || Opening::Saved(Some(panel_view(OID, PanelMode::Full)), false);
    let acted = proving(saved(), OID, |app| app.touched = true);
    assert!(acted.orch.panel.is_none(), "the human's first act wins");

    let held = proving(saved(), OID, |app| {
        app.orch.proof = Proof::Proven(OTHER.to_owned());
        app.open_panel(false);
    });
    let panel = held.orch.panel.as_ref().expect("their panel stays");
    assert_eq!(panel.pin(), Some(OTHER));
    assert_eq!(
        panel.handover().map(|view| view.mode),
        Some(PanelMode::Side)
    );

    let ours = proving(saved(), OID, |app| {
        app.orch.proof = Proof::Proven(OID.to_owned());
        app.open_panel(false);
    });
    let panel = ours.orch.panel.as_ref().expect("open");
    assert_eq!(
        panel.handover().map(|view| view.mode),
        Some(PanelMode::Full),
        "the saved mode applies to a panel of its own seat"
    );
}

#[test]
fn a_panel_the_file_left_closed_stays_closed_also_for_a_home_on_the_seat() {
    for homed in [false, true] {
        let app = proving(Opening::Saved(None, homed), OID, |_| {});
        assert!(app.orch.panel.is_none(), "homed {homed}");
    }
}

fn fleet_of(running: &str, stopped: &str) -> Fleet {
    let row = |(at, name, counts): (usize, &str, Counts)| Row {
        name: name.to_owned(),
        index: at + 1,
        mark: Mark::Working,
        needy: false,
        unanswered: None,
        counts,
        line2: Line2::NoGoal,
        home: name == running,
    };
    Fleet {
        rows: vec![
            row((0, running, Counts::Unknown)),
            row((1, stopped, Counts::Stopped)),
        ],
        home: Some(running.to_owned()),
    }
}

/// Whether a kept view of `kept` (written by the file or handed over) opens
/// the stopped group of an app homed on `home`, in a fleet of the running
/// `api` and the stopped `web`, where `web` has the uuid `web_id`.
fn opens_the_stopped_group(kept: &str, saved: bool, home: Option<&str>, web_id: &str) -> bool {
    let mut app = App::new(home.map(str::to_owned), None, None, None);
    app.fleet = fleet_of("api", "web");
    app.ids.insert("api".to_owned(), WEB_ID.to_owned());
    app.ids.insert("web".to_owned(), web_id.to_owned());
    app.keep = Some(Keep {
        view: plain_view(kept, WEB_ID),
        at: Timestamp::now().epoch(),
        saved,
    });
    app.fold_stopped(false, true);
    app.stopped_open
}

#[test]
fn a_kept_view_of_a_stopped_session_opens_the_stopped_group_only_when_it_will_apply() {
    assert!(
        opens_the_stopped_group("web", true, None, WEB_ID),
        "no home"
    );
    assert!(
        opens_the_stopped_group("web", true, Some("web"), WEB_ID),
        "its home"
    );
    assert!(
        !opens_the_stopped_group("web", true, Some("api"), WEB_ID),
        "the file's selection never overrides another home"
    );
    assert!(
        opens_the_stopped_group("web", false, Some("api"), WEB_ID),
        "a handed view applies whatever the home"
    );
    assert!(
        !opens_the_stopped_group("web", true, None, OTHER),
        "another incarnation is never applied"
    );
    assert!(
        !opens_the_stopped_group("api", true, None, WEB_ID),
        "a running session opens nothing"
    );
}

#[test]
fn the_saved_layout_is_taken_in_only_before_the_human_or_a_handover_has_acted() {
    let mut view = plain_view("web", WEB_ID);
    view.zoom = true;
    view.orchestrator = Some(panel_view(OID, PanelMode::Full));
    let fresh = || App::new(None, None, None, None);

    let mut app = fresh();
    app.saved_in(view.clone());
    assert!(app.model.zoom(), "the layout applies at once");
    assert_eq!(
        app.orch.opening,
        Opening::Saved(view.orchestrator.clone(), false)
    );
    let kept = app
        .keep
        .as_ref()
        .expect("the selection waits for the fleet");
    assert!(kept.saved);
    assert_eq!(kept.view, view);

    let mut app = fresh();
    app.orch.opening = Opening::Home;
    app.saved_in(view.clone());
    assert_eq!(
        app.orch.opening,
        Opening::Saved(view.orchestrator.clone(), true),
        "a home on the seat is remembered"
    );

    let mut app = fresh();
    app.touched = true;
    app.saved_in(view.clone());
    assert!(!app.model.zoom(), "after a first act nothing is taken in");
    assert!(app.keep.is_none());
    assert_eq!(app.orch.opening, Opening::No);

    let mut app = fresh();
    app.keep = Some(Keep {
        view: plain_view("api", WEB_ID),
        at: Timestamp::now().epoch(),
        saved: false,
    });
    app.saved_in(view);
    assert!(!app.model.zoom(), "a handed view is never read over");
    assert_eq!(
        app.keep.as_ref().map(|keep| keep.view.session.as_str()),
        Some("api")
    );
}

#[test]
fn an_exec_handover_is_kept_for_its_own_process_and_a_landing_names_the_core() {
    let now = Timestamp::now().epoch();
    let view = plain_view("web", WEB_ID);
    let carrier = |pane: String, version: &str| {
        let held = handover::Handover {
            pane,
            version: version.to_owned(),
            at: now,
            view: view.clone(),
        };
        Some(held.render())
    };
    let mine = format!("%{}", std::process::id());
    let (keep, landed) = super::run::handed_on(carrier(mine.clone(), crate::shape::VERSION));
    let keep = keep.expect("the view of this process and core is kept");
    assert_eq!((keep.saved, keep.at, &keep.view), (false, now, &view));
    assert_eq!(landed, None);
    let (keep, landed) = super::run::handed_on(carrier(mine, "2099.1.1"));
    assert!(keep.is_none(), "another core landed: no view");
    assert_eq!(landed.as_deref(), Some("2099.1.1"), "only its version");
    assert!(
        matches!(
            super::run::handed_on(carrier("%1".to_owned(), crate::shape::VERSION)),
            (None, None)
        ),
        "another process's view is ignored"
    );
    assert!(matches!(super::run::handed_on(None), (None, None)));
}
