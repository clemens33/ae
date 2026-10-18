//! An ask in flight must not stop the app: the loop goes on drawing and taking
//! keys while the delivery waits on its seat, the outcome lands when it ends,
//! and the kept draft is cleared only by a sent ask.

use super::*;
use std::sync::{Mutex, mpsc};

use std::sync::Arc;

/// How long a held delivery waits for its release before it gives up.
const HOLD: Duration = Duration::from_secs(5);

/// What a delivery that submitted nothing says on its error stream.
const SAYS: &str = "notice UNCONFIRMED: nothing was submitted; re-send";

/// How long the loop may take to hand an ask over and be itself again.
const PROMPT: Duration = Duration::from_secs(2);

/// A delivery the test holds: it says its request id on `entered` when it
/// begins, then waits for `release`; `lands` is the journal it writes before
/// it ends, as the helpers' own path writes the ask it carried.
struct Gate {
    entered: mpsc::Receiver<String>,
    release: mpsc::Sender<()>,
}

fn hold(app: &mut App, lands: Option<PathBuf>, says: &'static str) -> Gate {
    let (said, entered) = mpsc::channel();
    let (release, wait) = mpsc::channel::<()>();
    let (said, wait) = (Mutex::new(said), Mutex::new(wait));
    app.carry = Arc::new(move |ask: crate::console::term::Carry| {
        let id = ask.id().to_owned();
        let _ = said.lock().unwrap().send(id.clone());
        let _ = wait.lock().unwrap().recv_timeout(HOLD);
        if let Some(dir) = &lands {
            crate::store::open(dir)
                .append_event(&ask_event(&id))
                .unwrap();
        }
        says.to_owned()
    });
    Gate { entered, release }
}

/// The console's ask `id` to `lead`, as the helpers' tracked path records it.
fn ask_event(id: &str) -> String {
    format!(
        r#"{{"ts":"2026-10-15T06:00:00Z","actor":"console:local","action":"ask","target":"lead","ref":"{id}","target_slot":"main","target_session":"api","target_server":"/t","target_pane":"%1","target_session_uuid":"{ID}","summary":"q"}}
"#
    )
}

/// `root`'s home `api` recorded on a named tmux server, as an ask needs.
fn served(root: &Root) {
    let path = root.0.join("sessions/api/meta");
    let bytes = crate::meta::read_bytes(&root.0.join("sessions/api")).unwrap();
    let meta = String::from_utf8(bytes).unwrap();
    std::fs::write(
        &path,
        format!("{meta}tmux_server_kind=name\ntmux_server=fixture\n"),
    )
    .unwrap();
}

/// Enter `line` in the writing composer.
fn entered(app: &mut App, line: &str, at: Instant) {
    for key in [Key::Text(line.as_bytes().to_vec()), Key::Enter] {
        assert_eq!(app.compose(key, at), Some(()));
    }
}

/// The loop's own debts, settled until `done` holds or the wait is over.
fn pump(app: &mut App, done: impl Fn(&App) -> bool) -> bool {
    let until = Instant::now() + Duration::from_secs(8);
    while Instant::now() < until {
        let _ = app.owed();
        if done(app) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn sent(app: &App) -> bool {
    app.settling.as_ref().is_some_and(|s| s.word == "sent")
}

fn said(app: &App, word: &str) -> bool {
    app.notices.iter().any(|(_, item)| item.body.contains(word))
}

/// The loop hands an ask to its delivery and is itself again at once: the
/// hint row says the ask is on its way, a key is taken meanwhile, and the
/// outcome lands, with the kept draft cleared, only when the delivery ends.
#[test]
fn an_ask_in_flight_does_not_stop_the_loop_and_its_outcome_lands_when_it_ends() {
    let root = Root::new("flight-loop");
    let dir = root.0.join("sessions").join("api");
    let (mut app, _reader) = housed(&root);
    served(&root);
    let at = writing(&mut app);
    let gate = hold(&mut app, Some(dir.clone()), "");
    entered(&mut app, "hello", at);
    let begun = Instant::now();
    let owed = app.owed();
    let took = begun.elapsed();
    assert!(took < PROMPT, "the loop was held for {took:?} by one ask");
    assert!(owed, "handing the ask over owes a frame");
    let id = gate
        .entered
        .recv_timeout(PROMPT)
        .expect("the delivery began");
    assert_eq!(app.note().as_deref(), Some("sending to lead…"));
    assert!(!sent(&app), "nothing is said before the delivery ends");
    assert_eq!(
        crate::console::submit::recover(&app.recovery.record(&dir).unwrap()),
        crate::console::submit::Draft::Kept(b"hello".to_vec()),
        "the line is kept while it is in flight"
    );
    // The loop still takes keys and draws.
    assert_eq!(app.compose(Key::Text(b"next".to_vec()), at), Some(()));
    assert_eq!(drafted(&app), "next");
    assert!(framed(&mut app).contains("next"));
    gate.release.send(()).unwrap();
    assert!(pump(&mut app, sent), "the outcome landed: {id}");
    assert_eq!(
        crate::console::submit::recover(&app.recovery.record(&dir).unwrap()),
        crate::console::submit::Draft::Nothing,
        "a sent ask clears the kept draft"
    );
    assert_eq!(
        drafted(&app),
        "next",
        "the line typed meanwhile is untouched"
    );
    assert_eq!(app.note().as_deref(), Some("sent"));
}

/// A second Enter while one ask is in flight is refused where the human
/// sees it, its line is back in the draft, and the seat is asked once.
#[test]
fn a_second_ask_while_one_is_in_flight_is_refused_and_never_delivered_twice() {
    let root = Root::new("flight-twice");
    let dir = root.0.join("sessions").join("api");
    let (mut app, _reader) = housed(&root);
    served(&root);
    let at = writing(&mut app);
    let gate = hold(&mut app, Some(dir.clone()), "");
    entered(&mut app, "one", at);
    assert!(app.deliver());
    gate.entered.recv_timeout(PROMPT).expect("the first began");
    entered(&mut app, "two", at);
    let begun = Instant::now();
    assert!(app.deliver());
    assert!(begun.elapsed() < PROMPT, "the refusal was not prompt");
    assert!(said(&app, "still being delivered"), "{:?}", app.notices);
    assert_eq!(
        drafted(&app),
        "two",
        "the refused line is back in the draft"
    );
    assert!(
        gate.entered.try_recv().is_err(),
        "a second delivery began beside the first"
    );
    gate.release.send(()).unwrap();
    assert!(pump(&mut app, sent), "the first ask still lands");
    assert_eq!(drafted(&app), "two");
}

/// An ask whose delivery leaves no record keeps its draft and is told in
/// the lane, whenever the delivery ends.
#[test]
fn an_unrecorded_delivery_keeps_the_draft_and_says_so_when_it_ends() {
    let root = Root::new("flight-unknown");
    let dir = root.0.join("sessions").join("api");
    let (mut app, _reader) = housed(&root);
    served(&root);
    let at = writing(&mut app);
    let gate = hold(&mut app, None, SAYS);
    entered(&mut app, "lost", at);
    let begun = Instant::now();
    assert!(app.deliver());
    assert!(begun.elapsed() < PROMPT, "the loop was held");
    gate.entered
        .recv_timeout(PROMPT)
        .expect("the delivery began");
    gate.release.send(()).unwrap();
    assert!(pump(&mut app, |app| said(app, "no record of it")));
    assert!(
        said(&app, SAYS),
        "the delivery's own words are shown: {:?}",
        app.notices
    );
    assert_eq!(
        crate::console::submit::recover(&app.recovery.record(&dir).unwrap()),
        crate::console::submit::Draft::Kept(b"lost".to_vec()),
        "an ask nothing recorded keeps its draft"
    );
    assert!(!sent(&app));
}

/// Whether the console admission lock of `root`'s home is free right now.
fn admission_free(root: &Root) -> bool {
    // Tried, never waited for: the lock is the flight's for as long as it lasts.
    let lock = root.0.join("sessions/api/.console-admission.lock");
    crate::store::lock(&lock, Duration::ZERO).is_ok()
}

/// Whether another writer could take session `api`'s writer lease right now:
/// tried without waiting, and dropped at once.
fn lease_free(root: &Root) -> bool {
    crate::store::open(&root.0.join("sessions/api"))
        .console_writer()
        .is_ok()
}

/// The lease and the admission lock an ask went out under are the flight's
/// until the delivery ends, whatever happens to the writing meanwhile: a stop
/// of the writing ends nothing in flight, no other writer is admitted beside
/// it, the writing may start again without being refused by its own flight,
/// and the outcome still lands, both locks free again, a new ask admitted
/// after it.
#[test]
fn a_released_lease_stays_held_by_the_flight_and_both_locks_are_held_until_it_lands() {
    let root = Root::new("flight-release");
    let dir = root.0.join("sessions").join("api");
    let (mut app, _reader) = housed(&root);
    served(&root);
    let at = writing(&mut app);
    let gate = hold(&mut app, Some(dir.clone()), "");
    entered(&mut app, "one", at);
    assert!(app.deliver());
    gate.entered.recv_timeout(PROMPT).expect("the first began");
    assert!(!admission_free(&root), "the flight holds the admission");
    assert!(app.blocker(Instant::now()).is_some(), "a restart waits");
    assert!(!lease_free(&root), "the writing holds the lease");
    app.write(false);
    assert!(!app.composing(), "the writing stopped");
    assert!(!admission_free(&root), "released writing frees nothing");
    assert!(!lease_free(&root), "the flight still holds the lease");
    // The same app writes the session again: its own flight does not refuse it.
    let _ = writing(&mut app);
    assert!(
        app.held.is_none(),
        "refused by its own flight: {:?}",
        app.held
    );
    app.write(false);
    assert!(!lease_free(&root), "the flight holds the lease through it");
    gate.release.send(()).unwrap();
    assert!(pump(&mut app, sent), "the outcome landed after the release");
    assert!(admission_free(&root), "the landed flight let the lock go");
    assert!(lease_free(&root), "the landed flight let the lease go");
    // And the app writes again: the next ask is admitted and carried.
    let at = writing(&mut app);
    entered(&mut app, "two", at);
    assert!(app.deliver());
    gate.entered.recv_timeout(PROMPT).expect("the next began");
    gate.release.send(()).unwrap();
    assert!(pump(&mut app, sent));
}

/// The lease a flight keeps is its own incarnation's. A session replaced under
/// the same name while the ask is in flight has a lease of its own, and the
/// writing locks that one: a competitor already holding it still refuses the
/// app, and the app's hold, once it has it, is not the flight's old file.
#[test]
fn a_replaced_session_is_never_entered_through_the_old_flights_lease() {
    let root = Root::new("flight-replaced");
    let dir = root.0.join("sessions").join("api");
    let (mut app, _reader) = housed(&root);
    served(&root);
    let at = writing(&mut app);
    let gate = hold(&mut app, None, "");
    entered(&mut app, "one", at);
    assert!(app.deliver());
    gate.entered.recv_timeout(PROMPT).expect("the first began");
    app.write(false);
    // Another incarnation under the same name: a new directory, a new id.
    let new_id = ID.replace("1234", "bbbb");
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    meta(&root, &new_id, "colead");
    app.ids.insert("api".to_owned(), new_id);
    let rival = crate::store::open(&dir)
        .console_writer()
        .expect("the replacement's lease is its own: the old flight holds an unlinked file");
    app.take(Reading::Owner, Instant::now());
    app.write(true);
    assert!(!app.composing(), "entered through the old flight's lease");
    assert!(app.held.is_some(), "the refusal is shown");
    drop(rival);
    app.write(true);
    assert!(app.composing(), "{:?}", app.held);
    let (mine, flight) = (app.lease.as_ref().unwrap(), app.flight.as_ref().unwrap());
    assert!(
        !Arc::ptr_eq(&mine.held, &flight.lease),
        "the replacement is written under the old flight's lease"
    );
    assert!(!lease_free(&root), "the replacement's lease is held");
    gate.release.send(()).unwrap();
    assert!(
        pump(&mut app, |app| app.flight.is_none()),
        "the flight ended"
    );
}

/// `/close` needs the admission an ask in flight holds: refused by name, then
/// served once the ask landed.
#[test]
fn close_while_an_ask_is_in_flight_is_refused_by_name_and_works_after() {
    let root = Root::new("flight-close");
    let dir = root.0.join("sessions").join("api");
    let (mut app, _reader) = housed(&root);
    served(&root);
    let at = writing(&mut app);
    let gate = hold(&mut app, Some(dir.clone()), "");
    entered(&mut app, "one", at);
    assert!(app.deliver());
    gate.entered.recv_timeout(PROMPT).expect("the first began");
    let line = app.close(None);
    assert!(line.contains("still being delivered"), "{line}");
    assert!(line.starts_with("refused: "), "{line}");
    gate.release.send(()).unwrap();
    assert!(pump(&mut app, sent));
    let line = app.close(None);
    assert!(!line.contains("still being delivered"), "{line}");
}

/// A quit waits for the ask in flight, at most its bound, and says what it
/// awaits; a flight that ended on its own costs nothing.
#[test]
fn a_quit_waits_for_the_flight_within_its_bound_and_says_so() {
    let root = Root::new("flight-quit");
    let dir = root.0.join("sessions").join("api");
    let (mut app, _reader) = housed(&root);
    served(&root);
    let at = writing(&mut app);
    let gate = hold(&mut app, Some(dir.clone()), "");
    entered(&mut app, "one", at);
    assert!(app.deliver());
    gate.entered.recv_timeout(PROMPT).expect("the first began");
    let mut told = Vec::new();
    let begun = Instant::now();
    app.finish_flight(&mut told, Duration::from_millis(150))
        .unwrap();
    let waited = begun.elapsed();
    let told = String::from_utf8(told).unwrap();
    assert!(waited >= Duration::from_millis(150), "{waited:?}");
    assert!(
        waited < PROMPT,
        "the bound did not bound the wait: {waited:?}"
    );
    assert!(told.contains("finishing the delivery to lead"), "{told}");
    assert!(told.contains("was not over"), "{told}");
    // A delivery that ends inside the bound is waited for, not abandoned.
    gate.release.send(()).unwrap();
    let until = Instant::now() + Duration::from_secs(4);
    while !admission_free(&root) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(admission_free(&root), "the first delivery ended");
    drop(app);
    let (mut app, _reader) = housed(&root);
    let at = writing(&mut app);
    let gate = hold(&mut app, Some(dir.clone()), "");
    entered(&mut app, "two", at);
    assert!(app.deliver());
    gate.entered.recv_timeout(PROMPT).expect("the second began");
    let release = gate.release.clone();
    let late = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        release.send(()).unwrap();
    });
    let mut told = Vec::new();
    app.finish_flight(&mut told, Duration::from_secs(4))
        .unwrap();
    late.join().unwrap();
    let told = String::from_utf8(told).unwrap();
    assert!(told.contains("finishing the delivery"), "{told}");
    assert!(!told.contains("was not over"), "{told}");
    // No flight, no words.
    let mut told = Vec::new();
    app.finish_flight(&mut told, Duration::from_secs(4))
        .unwrap();
    assert!(told.is_empty());
}

/// A delivery thread that dies lands as an unrecorded ask in the words of
/// what happened, its lock freed and its draft kept: nothing is claimed sent.
#[test]
fn a_delivery_that_dies_lands_as_unrecorded_and_frees_the_lock() {
    let root = Root::new("flight-panic");
    let dir = root.0.join("sessions").join("api");
    let (mut app, _reader) = housed(&root);
    served(&root);
    let at = writing(&mut app);
    app.carry = std::sync::Arc::new(|_| panic!("the delivery died"));
    entered(&mut app, "doomed", at);
    assert!(app.deliver());
    assert!(pump(&mut app, |app| said(app, "no record of it")));
    assert!(
        said(&app, "the delivery ended without an answer"),
        "{:?}",
        app.notices
    );
    assert!(!sent(&app));
    assert!(admission_free(&root), "a dead thread let the lock go");
    assert_eq!(
        crate::console::submit::recover(&app.recovery.record(&dir).unwrap()),
        crate::console::submit::Draft::Kept(b"doomed".to_vec()),
        "an ask nothing recorded keeps its draft"
    );
}
