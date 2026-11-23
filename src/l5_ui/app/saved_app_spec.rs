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

use super::handover;
use super::loader::{Answer, FleetRead, Reader, Wake};
use super::saved::Saver;
use super::tests::{Root, entry, session};
use super::{App, drain};
use crate::console::input::{Key, Keys};
use crate::digest::Status;
use crate::listing::World;
use crate::theme::FleetOrder;
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
