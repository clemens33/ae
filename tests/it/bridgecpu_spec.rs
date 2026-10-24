//! Frozen bridgecpu acceptance (#233 phase 2): one outbound pass of the
//! Telegram bridge asks each entitled tmux server ONCE (`list-sessions`), probes
//! no session one by one (`has-session`) and parses no journal through the
//! inventory, while forwarding exactly the sessions it forwarded before.
//!
//! Oracle: the lead's phase-2 and parity rulings and the phase-1 report, never
//! the code. Production `pump`, loopback Telegram, real private tmux servers;
//! children counted by `transport::spawns_during`, parses by `events::parsed_bytes`.
//!
//! RED on the base: the three COST tests, each asserting behaviour first and
//! cost last. The rest are BEHAVIOUR carries, GREEN on the base by nature.
//! Every test names the wrong implementation it exists to catch.
#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real files; the boundary is about what PRODUCT \
              code may reach"
)]

use super::{Knobs, Paths, Relay, Word};
use crate::inventory::ServerId;
use crate::meta::Selector;
use crate::telegram::routing::World as _;
use crate::telegram::tests::{Fake, Reply};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The chat row every planted journal ends with, and its time.
const SAID_AT: &str = "2026-10-16T09:00:00Z";

/// A scratch ae home with private tmux servers, all removed on drop.
struct Fleet {
    root: PathBuf,
    live: Vec<(ServerId, String)>,
}

impl Fleet {
    fn new(tag: &str) -> Self {
        // `/tmp`-short on purpose: a socket path must fit `sun_path`.
        let root = PathBuf::from("/tmp").join(format!("ae-bcpu-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sessions")).expect("scratch root");
        Self {
            root,
            live: Vec::new(),
        }
    }

    fn socket(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn server(&self, socket: &str) -> ServerId {
        ServerId::Selected(Selector::Socket(self.socket(socket)))
    }

    fn paths(&self) -> Paths {
        Paths::under(&self.root)
    }

    /// A durable record `name` whose meta carries `selector` rows, a journal
    /// of `padding` state rows, and one lead chat row.
    fn plant(&self, name: &str, selector: &str, padding: usize) -> PathBuf {
        let dir = self.root.join("sessions").join(name);
        std::fs::create_dir_all(&dir).expect("session dir");
        let meta = format!("session_id=5e55{name}\nseat.main=lead\nprofile.main=cl\n{selector}");
        std::fs::write(dir.join("meta"), meta).expect("meta");
        let row =
            r#"{"ts":"2026-10-16T08:00:00Z","actor":"lead","action":"state","ref":"working"}"#;
        let journal = format!("{row}\n").repeat(padding) + &chat_row(&format!("SAY {name}"));
        std::fs::write(dir.join(crate::store::EVENTS), journal).expect("journal");
        dir
    }

    /// The selector rows naming the socket `socket` under this root.
    fn on(&self, socket: &str) -> String {
        format!(
            "tmux_server_kind=socket\ntmux_server={}\n",
            self.socket(socket).display()
        )
    }

    fn start(&mut self, socket: &str, name: &str) {
        let server = self.server(socket);
        let (ok, _) = crate::transport::run_tmux_op(&crate::session_tmux::argv(
            &server,
            &crate::session_tmux::Op::NewSession {
                name,
                work_dir: self.root.to_str().expect("fixture path"),
            },
        ));
        assert!(ok, "private tmux session {name} on {socket}");
        let id = crate::transport::observe_session_id(&server, name).expect("session id");
        self.live.push((server, id));
    }

    fn stop(&mut self, socket: &str, name: &str) {
        let server = self.server(socket);
        let id = crate::transport::observe_session_id(&server, name).expect("session id");
        assert!(crate::transport::kill_session(&server, &id), "stop {name}");
        self.live.retain(|(_, kept)| *kept != id);
    }
}

impl Drop for Fleet {
    fn drop(&mut self) {
        for (server, id) in &self.live {
            let _ = crate::transport::kill_session(server, id);
        }
        // Wait for each emptied server to exit: no fixture outlives the test.
        for (server, _) in &self.live {
            let at = Instant::now();
            while crate::transport::session_names(server).is_some()
                && at.elapsed() < Duration::from_secs(3)
            {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn chat_row(text: &str) -> String {
    format!(r#"{{"ts":"{SAID_AT}","actor":"lead","action":"chat","summary":"{text}"}}"#) + "\n"
}

fn append(dir: &Path, text: &str) {
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(dir.join(crate::store::EVENTS))
        .expect("journal");
    std::io::Write::write_all(&mut file, chat_row(text).as_bytes()).expect("append");
}

/// No cutoff, no carried asks: every planted row is fresh.
fn relay() -> Relay {
    Relay {
        since: i64::MIN,
        echoes: crate::telegram::Echoes::default(),
    }
}

/// What one production outbound pass did: the children it started, the
/// journal bytes the inventory parsed, the texts Telegram received.
struct Pass {
    spawns: Vec<Vec<String>>,
    parsed: u64,
    sent: Vec<String>,
}

fn one_pass(fleet: &Fleet, fake: &Fake) -> Pass {
    let api = fake.api();
    let before_sent = fake.requests().len();
    let knobs = Knobs {
        once: true,
        ..Knobs::default()
    };
    let stop = AtomicBool::new(false);
    let mut err = Vec::new();
    let before = crate::events::parsed_bytes();
    let (outcome, spawns) = crate::transport::spawns_during(|| {
        super::pump(&api, &fleet.paths(), knobs, &relay(), None, &stop, &mut err)
    });
    let parsed = crate::events::parsed_bytes() - before;
    let err = String::from_utf8_lossy(&err);
    assert!(matches!(outcome, Ok(0)), "{err}");
    let sent = texts(fake, before_sent);
    Pass {
        spawns,
        parsed,
        sent,
    }
}

/// The texts Telegram received after the first `skip` requests, sorted.
fn texts(fake: &Fake, skip: usize) -> Vec<String> {
    let mut sent: Vec<String> = (fake.requests().iter().skip(skip))
        .map(|seen| crate::json::parse(&seen.body).expect("a JSON body"))
        .map(|value| value.get_str("text").unwrap_or_default().to_owned())
        .collect();
    sent.sort();
    sent
}

/// Stops a pump when dropped, so a failed assertion cannot leave one running.
struct Halt<'a>(&'a AtomicBool);

impl Drop for Halt<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// The argv one listing must be: a UTF-8 client, the explicit server
/// (`-S` socket or `-L` name), the session names.
fn listing(flag: &str, server: impl std::fmt::Display) -> Vec<String> {
    let argv = format!("tmux -u {flag} {server} list-sessions -F #{{session_name}}");
    argv.split(' ').map(ToOwned::to_owned).collect()
}

fn said<S: AsRef<str>>(names: &[S]) -> Vec<String> {
    let mut texts: Vec<String> = (names.iter().map(AsRef::as_ref))
        .map(|n| format!("[{n}] lead\nSAY {n}"))
        .collect();
    texts.sort();
    texts
}

/// Phase 1's shape: 32 durable records on one socket with 40-row journals,
/// every fourth one live. Returns the live names.
fn thirty_two(fleet: &mut Fleet) -> Vec<String> {
    let mut live = Vec::new();
    for i in 0..32 {
        let name = format!("s{i:02}");
        fleet.plant(&name, &fleet.on("a.sock"), 40);
        if i % 4 == 0 {
            fleet.start("a.sock", &name);
            live.push(name);
        }
    }
    assert_eq!(live.len(), 8, "fixture: eight live of 32");
    live
}

#[test]
fn one_pass_lists_the_server_once_and_probes_no_session() {
    // COST, RED on the base: 32 `has-session` children per pass. Wrong
    // implementations caught: per-record has-session; a listing per record; a
    // listing plus confirmation; any other child; a manufactured live set.
    let mut fleet = Fleet::new("one");
    let live = thirty_two(&mut fleet);
    let fake = Fake::start(vec![Reply::ok()]);

    let pass = one_pass(&fleet, &fake);
    assert_eq!(pass.sent, said(&live), "exactly the live sessions forward");
    assert_eq!(
        pass.spawns,
        vec![listing("-S", fleet.socket("a.sock").display())],
        "one pass = exactly one list-sessions of the one entitled server, nothing else"
    );
}

#[test]
fn one_pass_parses_no_journal_while_the_inbound_world_still_does() {
    // COST, RED on the base: every journal parsed only to test liveness. Wrong
    // implementations caught: `inventory::durable_records` kept on the outbound
    // path; the inbound world made meta-only (its /list ages vanish).
    let mut fleet = Fleet::new("journal");
    let live = thirty_two(&mut fleet);
    let fake = Fake::start(vec![Reply::ok()]);

    let pass = one_pass(&fleet, &fake);
    assert_eq!(pass.sent, said(&live), "exactly the live sessions forward");

    // Control: the inbound world still parses journals for its /list ages.
    let before = crate::events::parsed_bytes();
    let running = super::Machine::under(fleet.paths().roots()).running();
    assert!(
        crate::events::parsed_bytes() > before,
        "control: parsed_bytes counts inventory reads"
    );
    let at = crate::time::Timestamp::parse(SAID_AT)
        .expect("time")
        .epoch();
    let ages: Vec<(&str, Option<i64>)> = (running.iter())
        .map(|session| (session.name.as_str(), session.last_active))
        .collect();
    let want: Vec<(&str, Option<i64>)> =
        live.iter().map(|name| (name.as_str(), Some(at))).collect();
    assert_eq!(
        ages, want,
        "/list ages come from the journal on the inbound world"
    );

    assert_eq!(
        pass.parsed, 0,
        "the outbound pass parsed journal bytes through the inventory"
    );
}

#[test]
fn each_selector_is_listed_once_and_only_its_own_names_count() {
    // a.sock holds alpha, worker-long, gamma, delta; b.sock holds beta;
    // dead.sock has no server. gamma selects b.sock; delta's `-L` NAME selector
    // is spelled like a.sock's path. Wrong implementations caught: a listing per
    // record or per failed record; names unioned across servers (gamma); prefix
    // match (worker); selectors keyed by text, not type (delta); an ambient
    // fallback (nosel, amb); one failed server suppressing the healthy ones.
    let mut fleet = Fleet::new("sel");
    let [a, b, dead] = ["a.sock", "b.sock", "dead.sock"].map(|socket| fleet.on(socket));
    for (name, selector) in [
        ("alpha", &a),
        ("worker", &a),
        ("gamma", &b),
        ("beta", &b),
        ("c1", &dead),
        ("c2", &dead),
        ("c3", &dead),
    ] {
        fleet.plant(name, selector, 5);
    }
    fleet.plant("nosel", "", 5);
    fleet.plant("amb", "tmux_server_kind=bogus\ntmux_server=x\n", 5);
    let by_name = format!(
        "tmux_server_kind=name\ntmux_server={}\n",
        fleet.socket("a.sock").display()
    );
    fleet.plant("delta", &by_name, 5);
    for name in ["alpha", "worker-long", "gamma", "delta"] {
        fleet.start("a.sock", name);
    }
    fleet.start("b.sock", "beta");
    let fake = Fake::start(vec![Reply::ok()]);

    let pass = one_pass(&fleet, &fake);
    assert_eq!(
        pass.sent,
        said(&["alpha", "beta"]),
        "only names on their own server forward"
    );
    let mut spawns = pass.spawns.clone();
    spawns.sort();
    let mut want: Vec<_> = (["a.sock", "b.sock", "dead.sock"].iter())
        .map(|socket| listing("-S", fleet.socket(socket).display()))
        .collect();
    want.push(listing("-L", fleet.socket("a.sock").display()));
    want.sort();
    assert_eq!(
        spawns, want,
        "one listing per typed selector, failed one included"
    );
}

#[test]
fn a_running_pump_drops_a_stopped_session_and_picks_up_a_started_one() {
    // ONE long-lived pump, membership changed between its passes. Wrong
    // implementations caught: a live set computed once per pump or cached
    // across passes (two never joins); a stopped session's rows lost or sent.
    let mut fleet = Fleet::new("life");
    let one = fleet.plant("one", &fleet.on("a.sock"), 3);
    let two = fleet.plant("two", &fleet.on("a.sock"), 3);
    // An unrecorded keeper holds the server: no stop races a server exit.
    fleet.start("a.sock", "keeper");
    fleet.start("a.sock", "one");
    let fake = Fake::start(vec![Reply::ok()]);
    let (api, paths, stop) = (fake.api(), fleet.paths(), AtomicBool::new(false));
    let knobs = Knobs {
        outbound_interval: Duration::from_millis(100),
        ..Knobs::default()
    };
    let sent = |count: usize| {
        let at = Instant::now();
        while fake.requests().len() < count && at.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(20));
        }
        texts(&fake, 0)
    };
    std::thread::scope(|scope| {
        let pumping = scope
            .spawn(|| super::pump(&api, &paths, knobs, &relay(), None, &stop, &mut Vec::new()));
        let halt = Halt(&stop);
        assert_eq!(sent(1), said(&["one"]), "first passes: only one is live");
        fleet.stop("a.sock", "one");
        fleet.start("a.sock", "two");
        // Every pass after the one that forwards two listed after the stop.
        assert_eq!(sent(2), said(&["one", "two"]), "the SAME pump follows");
        append(&one, "WHILE STOPPED");
        append(&two, "SECOND");
        let mut want = said(&["one", "two"]);
        want.push("[two] lead\nSECOND".to_owned());
        want.sort();
        assert_eq!(
            sent(3),
            want,
            "two continues; the stopped one holds its row"
        );
        fleet.start("a.sock", "one");
        want.push("[one] lead\nWHILE STOPPED".to_owned());
        want.sort();
        assert_eq!(sent(4), want, "the resumed one sends what it held");
        drop(halt);
        assert_eq!(pumping.join().expect("pump thread").expect("pump"), 0);
    });
}

#[test]
fn a_symlinked_meta_gets_the_same_verdict_inbound_and_outbound() {
    // Lead parity ruling: a perf slice changes no forwarding membership, and
    // the inbound world accepts a linked meta today. Wrong implementation
    // caught: `inventory::durable_meta_records`, which refuses the link.
    let mut fleet = Fleet::new("link");
    let dir = fleet.plant("linked", &fleet.on("a.sock"), 3);
    let real = fleet.root.join("linked-meta");
    std::fs::rename(dir.join("meta"), &real).expect("move meta");
    std::os::unix::fs::symlink(&real, dir.join("meta")).expect("link meta");
    fleet.start("a.sock", "linked");
    let fake = Fake::start(vec![Reply::ok()]);

    let inbound = super::Machine::under(fleet.paths().roots()).running();
    let names: Vec<&str> = inbound.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["linked"], "control: the inbound world accepts it");
    assert_eq!(
        one_pass(&fleet, &fake).sent,
        said(&["linked"]),
        "outbound agrees"
    );
}

#[test]
fn a_word_wakes_the_pump_before_its_interval_and_the_interval_stays_two_seconds() {
    // The ruling keeps the 2 s cadence and the immediate wake-up. Wrong
    // implementations caught: a longer interval; `recv_timeout` made a sleep.
    assert_eq!(super::OUTBOUND_INTERVAL, Duration::from_secs(2));
    let fleet = Fleet::new("wake");
    let fake = Fake::start(vec![Reply::ok()]);
    let api = fake.api();
    let paths = fleet.paths();
    let knobs = Knobs {
        outbound_interval: Duration::from_mins(1),
        ..Knobs::default()
    };
    let stop = AtomicBool::new(false);
    let (words, inbox) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        let (api, paths, stop) = (&api, &paths, &stop);
        let pumping = scope.spawn(move || {
            let mut err = Vec::new();
            super::pump(api, paths, knobs, &relay(), Some(&inbox), stop, &mut err)
        });
        std::thread::sleep(Duration::from_millis(500));
        let at = Instant::now();
        words
            .send(Word::Say("WAKE".to_owned()))
            .expect("pump listening");
        while fake.requests().is_empty() && at.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let waited = at.elapsed();
        stop.store(true, Ordering::Relaxed);
        drop(words);
        assert_eq!(pumping.join().expect("pump thread").expect("pump"), 0);
        assert!(
            waited < Duration::from_secs(20),
            "a word waited {waited:?} on a 1 min interval: it must wake the pump"
        );
    });
    let sent = fake.requests();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].body.contains("WAKE"), "{}", sent[0].body);
}
