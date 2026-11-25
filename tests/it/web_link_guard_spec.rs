//! The fresh-link guard of `ae web`, through the real loopback listener on a
//! clock that moves by hand: a request without a session mints and prints one
//! fresh link, and while that fresh code is unredeemed and unexpired no later
//! request mints or prints another, so a client that keeps asking cannot push
//! the owner's codes out of the kept set. The code printed at start never holds
//! a fresh link back. The pre-existing 10 s gap and the redeem and expiry rules
//! are pinned by `web_server_spec` and `web_auth_spec`; this file pins how the
//! guard joins them.

use std::io::{Read as _, Write as _};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use ae::web::auth::{CODE_LIFE, CODES_MAX};
use ae::web::clock::ManualClock;
use ae::web::server::{self, LINK_GAP, NoProbe, Options, Running};
use ae::web::state::{Read, Source};

const WAIT: Duration = Duration::from_secs(5);
const EPOCH: i64 = 1_795_000_000;
/// A well-formed code that no listener ever minted.
const UNKNOWN: &str = "00000000000000000000000000000000";

struct Empty;

impl Source for Empty {
    fn read(&self) -> Read {
        Read::default()
    }
}

struct Rig {
    running: Option<Running>,
    clock: Arc<ManualClock>,
    links: Arc<Mutex<Vec<String>>>,
}

impl Rig {
    fn start() -> Self {
        let clock = Arc::new(ManualClock::new(EPOCH));
        let links = Arc::new(Mutex::new(Vec::new()));
        let received = Arc::clone(&links);
        let running = server::start(Options {
            port: 0,
            clock: clock.clone(),
            source: Arc::new(Empty),
            probe: Arc::new(NoProbe),
            links: Arc::new(move |line| {
                received
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(line.to_owned());
            }),
            version: "link-guard".to_owned(),
        })
        .unwrap_or_else(|error| panic!("start loopback server: {error}"));
        Self {
            running: Some(running),
            clock,
            links,
        }
    }

    fn addr(&self) -> SocketAddr {
        self.running
            .as_ref()
            .unwrap_or_else(|| panic!("server handle"))
            .addr()
    }

    /// The code the server minted at start, from its own link.
    fn startup_code(&self) -> String {
        let link = self
            .running
            .as_ref()
            .unwrap_or_else(|| panic!("server handle"))
            .link();
        let (_, code) = link
            .split_once("#c=")
            .unwrap_or_else(|| panic!("a link with a code: {link}"));
        code.to_owned()
    }

    /// The fresh link lines printed so far.
    fn printed(&self) -> Vec<String> {
        self.links
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The code of the fresh link line `at`, which must be well formed.
    fn code_printed(&self, at: usize) -> String {
        let lines = self.printed();
        let line = lines
            .get(at)
            .unwrap_or_else(|| panic!("fresh link {at} in {lines:?}"));
        let prefix = format!("ae web: new link http://{}/#c=", self.addr());
        let code = line
            .strip_prefix(&prefix)
            .unwrap_or_else(|| panic!("{line:?} does not start with {prefix:?}"));
        assert_eq!(code.len(), 32, "{line:?}");
        code.to_owned()
    }

    fn move_by(&self, by: Duration) {
        self.clock.advance(by);
    }

    /// The status of one whole exchange, read to the close and bounded.
    fn exchange(&self, wire: &str) -> u16 {
        let mut client =
            TcpStream::connect(self.addr()).unwrap_or_else(|error| panic!("connect: {error}"));
        client
            .set_read_timeout(Some(WAIT))
            .unwrap_or_else(|error| panic!("read bound: {error}"));
        client
            .write_all(wire.as_bytes())
            .unwrap_or_else(|error| panic!("send: {error}"));
        client
            .shutdown(Shutdown::Write)
            .unwrap_or_else(|error| panic!("half-close: {error}"));
        let mut reply = String::new();
        client
            .read_to_string(&mut reply)
            .unwrap_or_else(|error| panic!("the whole answer: {error}"));
        reply
            .strip_prefix("HTTP/1.1 ")
            .and_then(|rest| rest.get(..3))
            .and_then(|digits| digits.parse().ok())
            .unwrap_or_else(|| panic!("a status line: {reply:?}"))
    }

    /// `GET /api/state` with no session: the status of the answer.
    fn signed_out(&self) -> u16 {
        self.exchange(&format!(
            "GET /api/state HTTP/1.1\r\nHost: {}\r\n\r\n",
            self.addr()
        ))
    }

    /// `POST /api/session` with `code`: the status of the answer.
    fn redeem(&self, code: &str) -> u16 {
        let body = format!(r#"{{"code":"{code}"}}"#);
        self.exchange(&format!(
            "POST /api/session HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len(),
            host = self.addr()
        ))
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        if let Some(running) = self.running.take() {
            running.shutdown();
        }
    }
}

#[test]
fn the_first_signed_out_request_prints_a_fresh_link_while_the_start_code_waits() {
    let rig = Rig::start();
    let startup = rig.startup_code();
    assert_eq!(rig.signed_out(), 401);
    assert_eq!(rig.printed().len(), 1, "{:?}", rig.printed());
    let fresh = rig.code_printed(0);
    assert_ne!(fresh, startup);
    assert_eq!(rig.redeem(&startup), 200, "the start code still works");
    assert_eq!(rig.redeem(&fresh), 200, "the fresh code works");
}

#[test]
fn a_client_that_keeps_asking_prints_one_link_while_that_code_waits() {
    let rig = Rig::start();
    for _ in 0..(CODES_MAX * 3) {
        rig.move_by(LINK_GAP);
        assert_eq!(rig.signed_out(), 401);
    }
    assert_eq!(rig.printed().len(), 1, "{:?}", rig.printed());
    let fresh = rig.code_printed(0);
    assert_eq!(
        rig.redeem(&fresh),
        200,
        "the one printed code is still good"
    );
}

#[test]
fn a_client_that_keeps_asking_cannot_push_out_the_start_code() {
    let rig = Rig::start();
    let startup = rig.startup_code();
    for _ in 0..(CODES_MAX * 3) {
        rig.move_by(LINK_GAP);
        assert_eq!(rig.signed_out(), 401);
    }
    assert_eq!(rig.redeem(&startup), 200, "the start code is still kept");
}

#[test]
fn a_redeemed_fresh_code_lets_the_next_request_print_again() {
    let rig = Rig::start();
    assert_eq!(rig.signed_out(), 401);
    let first = rig.code_printed(0);
    // Held back by the waiting code: it must not count as a print, or the
    // request after the redeem below would still be inside the gap.
    rig.move_by(Duration::from_secs(12));
    assert_eq!(rig.signed_out(), 401);
    assert_eq!(rig.printed().len(), 1, "{:?}", rig.printed());
    assert_eq!(rig.redeem(&first), 200);
    rig.move_by(Duration::from_secs(3));
    assert_eq!(rig.signed_out(), 401);
    assert_eq!(rig.printed().len(), 2, "{:?}", rig.printed());
    assert_ne!(rig.code_printed(1), first);
}

#[test]
fn an_expired_fresh_code_lets_the_next_request_print_again() {
    let rig = Rig::start();
    assert_eq!(rig.signed_out(), 401);
    let first = rig.code_printed(0);
    rig.move_by(CODE_LIFE.saturating_sub(Duration::from_secs(1)));
    assert_eq!(rig.signed_out(), 401);
    assert_eq!(
        rig.printed().len(),
        1,
        "one second before it expires the code still holds the next link back"
    );
    rig.move_by(Duration::from_secs(1));
    assert_eq!(rig.signed_out(), 401);
    assert_eq!(rig.printed().len(), 2, "{:?}", rig.printed());
    let second = rig.code_printed(1);
    assert_ne!(second, first);
    assert_eq!(rig.redeem(&first), 401, "the first code expired");
    assert_eq!(rig.redeem(&second), 200);
}

#[test]
fn a_refused_code_exchange_obeys_the_same_guard() {
    let rig = Rig::start();
    assert_eq!(rig.signed_out(), 401);
    let first = rig.code_printed(0);
    rig.move_by(LINK_GAP);
    assert_eq!(rig.redeem(UNKNOWN), 401);
    assert_eq!(
        rig.printed().len(),
        1,
        "a refused exchange prints nothing while a fresh code waits"
    );
    assert_eq!(rig.redeem(&first), 200);
    rig.move_by(LINK_GAP);
    assert_eq!(rig.redeem(UNKNOWN), 401);
    assert_eq!(
        rig.printed().len(),
        2,
        "a refused exchange prints a link once the fresh code is spent"
    );
}
