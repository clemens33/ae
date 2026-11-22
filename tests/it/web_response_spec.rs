//! S1a response limits and the response deadline through the real listener.

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use ae::json::{self, Value};
use ae::web::chat::Row;
use ae::web::clock::ManualClock;
use ae::web::server::{self, Cut, Options, Probe, Running, Seen};
use ae::web::state::{Read, Source};

const WAIT: Duration = Duration::from_secs(5);
const BODY_CAP: usize = 65_536;
const ROW_CAP: usize = 500;

struct Snapshot(Read);

impl Source for Snapshot {
    fn read(&self) -> Read {
        self.0.clone()
    }
}

struct Events(Sender<Seen>);

impl Probe for Events {
    fn saw(&self, event: Seen) {
        let _ = self.0.send(event);
    }
}

struct Rig {
    running: Option<Running>,
    clock: Arc<ManualClock>,
    events: Receiver<Seen>,
}

impl Rig {
    fn new(rows: Vec<Row>) -> Self {
        let mut snapshot = Read::default();
        snapshot.chat.rows = rows;
        let clock = Arc::new(ManualClock::new(1_795_000_000));
        let (sender, events) = mpsc::channel();
        let running = server::start(Options {
            port: 0,
            clock: clock.clone(),
            source: Arc::new(Snapshot(snapshot)),
            probe: Arc::new(Events(sender)),
            links: Arc::new(|_| {}),
            version: "response-fixture".to_owned(),
        })
        .unwrap_or_else(|error| panic!("start listener: {error}"));
        Self {
            running: Some(running),
            clock,
            events,
        }
    }

    fn handle(&self) -> &Running {
        self.running.as_ref().unwrap_or_else(|| panic!("listener"))
    }

    fn login(&self) -> String {
        let link = self.handle().link();
        let (_, code) = link.split_once("#c=").unwrap_or_else(|| panic!("{link}"));
        let host = self.handle().addr();
        let body = format!(r#"{{"code":"{code}"}}"#);
        let len = body.len();
        let wire = format!(
            "POST /api/session HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\n\
             Content-Type: application/json\r\nContent-Length: {len}\r\n\r\n{body}"
        );
        let reply = read_all(connect(host, &wire));
        json_reply(&reply)
            .get_str("session")
            .unwrap_or_else(|| panic!("expected bearer"))
            .to_owned()
    }

    fn state(&self, bearer: &str) -> TcpStream {
        let host = self.handle().addr();
        connect(
            host,
            &format!(
                "GET /api/state HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {bearer}\r\n\r\n"
            ),
        )
    }

    fn observe(&self, description: &str, predicate: impl Fn(&Seen) -> bool) -> Seen {
        let deadline = Instant::now() + WAIT;
        loop {
            let event = self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| panic!("{description}: {error}"));
            if predicate(&event) {
                return event;
            }
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        if let Some(running) = self.running.take() {
            running.shutdown();
        }
    }
}

fn connect(addr: SocketAddr, wire: &str) -> TcpStream {
    let mut tcp = TcpStream::connect_timeout(&addr, WAIT)
        .unwrap_or_else(|error| panic!("connect to {addr}: {error}"));
    tcp.set_read_timeout(Some(WAIT))
        .unwrap_or_else(|error| panic!("fixture read deadline: {error}"));
    tcp.set_write_timeout(Some(WAIT))
        .unwrap_or_else(|error| panic!("fixture write deadline: {error}"));
    tcp.write_all(wire.as_bytes())
        .unwrap_or_else(|error| panic!("send to {addr}: {error}"));
    tcp
}

fn read_all(mut client: TcpStream) -> String {
    let mut reply = String::new();
    client
        .read_to_string(&mut reply)
        .unwrap_or_else(|error| panic!("full UTF-8 response: {error}"));
    reply
}

fn json_reply(wire: &str) -> Value {
    assert!(wire.starts_with("HTTP/1.1 200 "));
    let (_, body) = wire
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("HTTP body"));
    json::parse(body).unwrap_or_else(|error| panic!("response JSON: {error}"))
}

fn row(index: usize, body: String) -> Row {
    let number = i64::try_from(index).unwrap_or_else(|error| panic!("fixture index: {error}"));
    Row {
        key: number.unsigned_abs(),
        micros: number,
        kind: "answer".to_owned(),
        who: "lead".to_owned(),
        what: "answers".to_owned(),
        voice: "lead".to_owned(),
        body,
    }
}

#[test]
fn state_keeps_newest_rows_and_bounds_text_at_utf8_boundaries() {
    let mut rows = (0..ROW_CAP + 3)
        .map(|index| row(index, format!("turn {index}")))
        .collect::<Vec<_>>();

    // The first character beyond the budget straddles the byte boundary.
    let mut boundary = "B\0\r\n\t\u{1b}".to_owned();
    boundary.push_str(&"x".repeat(BODY_CAP - 4));
    boundary.push_str("éafter the cap");
    rows[ROW_CAP + 1].body = boundary;
    // A multi-byte character whose last byte fits the cap must survive.
    rows[ROW_CAP + 2].body = format!("{}é", "x".repeat(BODY_CAP - 2));
    rows[ROW_CAP + 2].who = format!("\u{7f}{}omitted", "é".repeat(200));
    rows[ROW_CAP + 2].what = format!("\u{1b}{}omitted", "鳥".repeat(200));

    let rig = Rig::new(rows);
    let token = rig.login();
    let wire = read_all(rig.state(&token));
    let snapshot = json_reply(&wire);
    let chat = snapshot
        .get("chat")
        .unwrap_or_else(|| panic!("chat object"));
    assert_eq!(chat.get("cut"), Some(&Value::Num(3)));
    let Some(Value::Arr(seen)) = chat.get("rows") else {
        panic!("chat rows");
    };
    assert_eq!(seen.len(), ROW_CAP);
    assert_eq!(seen[0].get_str("body"), Some("turn 3"));
    for (offset, turn) in seen.iter().enumerate() {
        let index = offset + 3;
        let key = format!("{index:016x}");
        assert_eq!(turn.get_str("key"), Some(key.as_str()));
    }

    let truncated = seen[ROW_CAP - 2]
        .get_str("body")
        .unwrap_or_else(|| panic!("truncated body"));
    assert!(truncated.starts_with("B\n\tx"));
    assert!(
        !truncated
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\t'))
    );
    assert!((BODY_CAP - 4..=BODY_CAP).contains(&truncated.len()));
    assert!(!truncated.contains(['é', '\u{fffd}']));

    let last = &seen[ROW_CAP - 1];
    let exact = last.get_str("body").unwrap_or_else(|| panic!("exact body"));
    assert_eq!(exact.len(), BODY_CAP);
    assert!(exact.ends_with('é'));
    assert_eq!(last.get_str("who"), Some("é".repeat(200).as_str()));
    assert_eq!(last.get_str("what"), Some("鳥".repeat(200).as_str()));
}

#[test]
fn a_client_that_does_not_read_is_cut_at_the_whole_response_deadline() {
    // JSON escapes tabs. This yields over 62 MiB on the wire within both caps.
    let rows = (0..ROW_CAP)
        .map(|index| row(index, "\t".repeat(BODY_CAP)))
        .collect();
    let rig = Rig::new(rows);
    let token = rig.login();
    let _ = rig.events.try_iter().count();
    let stalled = rig.state(&token);
    let Seen::FirstByte { worker } = rig.observe("state request reaches a reader", |event| {
        matches!(event, Seen::FirstByte { .. })
    }) else {
        panic!("state request worker");
    };
    let mut first_byte = [0];
    assert_eq!(
        stalled
            .peek(&mut first_byte)
            .unwrap_or_else(|error| panic!("first response byte: {error}")),
        1
    );
    assert_eq!(first_byte, [b'H']);

    rig.clock.advance(Duration::from_secs(9));
    let host = rig.handle().addr();
    let independent = read_all(connect(
        host,
        &format!("GET / HTTP/1.1\r\nHost: {host}\r\n\r\n"),
    ));
    assert!(independent.starts_with("HTTP/1.1 200 "));
    for event in rig.events.try_iter() {
        assert!(!matches!(event, Seen::Cut { worker: reader, .. } if reader == worker));
        assert!(!matches!(event, Seen::Answered { worker: reader, .. } if reader == worker));
    }

    rig.clock.advance(Duration::from_secs(1));
    let _ = rig.observe("non-reading client crosses the write deadline", |event| {
        matches!(event, Seen::Cut { worker: reader, why: Cut::ResponseDeadline } if *reader == worker)
    });
}
