//! S1b addendum for bounded topic text and group projection.

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use ae::json::{self, Value};
use ae::topic::{self, Card, Entry, Field, Group, Origin};
use ae::web::clock::ManualClock;
use ae::web::server::{self, NoProbe, Options, Running};
use ae::web::state::{Read, Source};
use ae::web::topics::{Column, Skipped, Wait, WaitKind};

const BODY_CAP: usize = 65_536;
const LABEL_CAP: usize = 200;
const SOCKET_WAIT: Duration = Duration::from_secs(5);

fn label(stem: &str) -> (String, String) {
    let tail = "é".repeat(LABEL_CAP - stem.chars().count());
    let raw = format!("\0{stem}\n\t{tail}\u{7f}after the cut");
    (raw, format!("{stem}{tail}"))
}

fn body(stem: &str) -> (String, String) {
    let tail = "x".repeat(BODY_CAP - stem.len() - 3);
    let raw = format!("\0{stem}\r\n\x1b\t{tail}€after the cut");
    (raw, format!("{stem}\n\t{tail}"))
}

fn origin() -> Origin {
    Origin {
        session: label("origin-session").0,
        uuid: label("origin-uuid").0,
        at: label("origin-time").0,
    }
}

fn with_origin(value: String) -> Field {
    Field {
        value,
        origin: origin(),
    }
}

fn snapshot() -> Read {
    let closed = Card {
        slug: "release".to_owned(),
        title: Some(with_origin(label("title").0)),
        state: Some(with_origin("closed".to_owned())),
        what: Some(with_origin(body("what").0)),
        last: Some(with_origin(body("last").0)),
        next: Some(with_origin(body("next").0)),
        src: Some(with_origin(body("src").0)),
        close: Some(with_origin(body("close").0)),
        reviewed: label("reviewed").0,
        unreadable: None,
        history: vec![Entry {
            origin: origin(),
            rank: (0, 0),
            text: body("history").0,
        }],
    };
    let unreadable = Card {
        slug: "unreadable".to_owned(),
        title: None,
        state: None,
        what: None,
        last: None,
        next: None,
        src: None,
        close: None,
        reviewed: label("reviewed").0,
        unreadable: Some(body("unreadable").0),
        history: Vec::new(),
    };
    Read {
        desk: Column {
            groups: vec![
                (Group::NeedsYou, vec![closed]),
                (Group::Moving, Vec::new()),
                (Group::Quiet, Vec::new()),
                (Group::Parked, Vec::new()),
                (Group::Closed, Vec::new()),
                (Group::Unreadable, vec![unreadable]),
            ],
            waits: BTreeMap::from([(
                "release".to_owned(),
                vec![Wait {
                    kind: WaitKind::Ask,
                    text: body("tagged wait").0,
                    id: Some(label("wait-id").0),
                    item: None,
                }],
            )]),
            untagged: vec![Wait {
                kind: WaitKind::Decide,
                text: body("untagged wait").0,
                id: None,
                item: None,
            }],
            decide: Vec::new(),
            gaps: vec![body("gap").0],
            skipped: Skipped::default(),
        },
        ..Read::default()
    }
}

struct Snapshot(Read);

impl Source for Snapshot {
    fn read(&self) -> Read {
        self.0.clone()
    }
}

struct Listener(Option<Running>);

impl Listener {
    fn start() -> Self {
        let running = server::start(Options {
            port: 0,
            clock: Arc::new(ManualClock::new(1_795_000_000)),
            source: Arc::new(Snapshot(snapshot())),
            probe: Arc::new(NoProbe),
            links: Arc::new(|_| {}),
            version: "topic-text-fixture".to_owned(),
        })
        .unwrap_or_else(|error| panic!("start listener: {error}"));
        Self(Some(running))
    }

    fn handle(&self) -> &Running {
        self.0
            .as_ref()
            .unwrap_or_else(|| panic!("running listener"))
    }

    fn login(&self) -> String {
        let host = self.handle().addr();
        let link = self.handle().link();
        let (_, code) = link
            .split_once("#c=")
            .unwrap_or_else(|| panic!("startup fragment"));
        let input = format!(r#"{{"code":"{code}"}}"#);
        let len = input.len();
        let reply = exchange(
            host,
            &format!(
                "POST /api/session HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\n\
             Content-Type: application/json\r\nContent-Length: {len}\r\n\r\n{input}"
            ),
        );
        text(&reply, "session").to_owned()
    }

    fn get(&self, path: &str, token: &str) -> Value {
        let host = self.handle().addr();
        exchange(
            host,
            &format!(
                "GET {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\n\r\n"
            ),
        )
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        if let Some(running) = self.0.take() {
            running.shutdown();
        }
    }
}

fn exchange(host: SocketAddr, wire: &str) -> Value {
    let mut socket = TcpStream::connect_timeout(&host, SOCKET_WAIT)
        .unwrap_or_else(|error| panic!("connect to {host}: {error}"));
    socket
        .set_read_timeout(Some(SOCKET_WAIT))
        .unwrap_or_else(|error| panic!("set read deadline: {error}"));
    socket
        .set_write_timeout(Some(SOCKET_WAIT))
        .unwrap_or_else(|error| panic!("set write deadline: {error}"));
    socket
        .write_all(wire.as_bytes())
        .unwrap_or_else(|error| panic!("request: {error}"));
    let mut reply = String::new();
    socket
        .read_to_string(&mut reply)
        .unwrap_or_else(|error| panic!("UTF-8 response: {error}"));
    let (header, data) = reply
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("response framing"));
    assert!(header.starts_with("HTTP/1.1 200 "), "{header}");
    json::parse(data).unwrap_or_else(|error| panic!("response JSON: {error}"))
}

fn field<'a>(node: &'a Value, key: &str) -> &'a Value {
    node.get(key).unwrap_or_else(|| panic!("missing key {key}"))
}

fn text<'a>(node: &'a Value, key: &str) -> &'a str {
    node.get_str(key)
        .unwrap_or_else(|| panic!("missing text {key}"))
}

fn only<'a>(node: &'a Value, key: &str) -> &'a Value {
    let Value::Arr(items) = field(node, key) else {
        panic!("{key} must be an array");
    };
    let [item] = items.as_slice() else {
        panic!("{key} must hold one item");
    };
    item
}

fn expect_text(actual: &str, expected: &str, name: &str) {
    assert!(
        actual == expected,
        "bad {name} ({} bytes, {} chars)",
        actual.len(),
        actual.chars().count()
    );
}

fn expect_origin(node: &Value) {
    expect_text(
        text(node, "session"),
        &label("origin-session").1,
        "source session",
    );
    expect_text(text(node, "uuid"), &label("origin-uuid").1, "source uuid");
    expect_text(text(node, "at"), &label("origin-time").1, "source time");
}

fn expect_tagged_wait(node: &Value) {
    let wait = only(node, "waits");
    assert_eq!(text(wait, "kind"), "ask");
    expect_text(text(wait, "id"), &label("wait-id").1, "ask id");
    expect_text(
        text(wait, "text"),
        &body("tagged wait").1,
        "tagged ask text",
    );
}

#[test]
fn topic_and_desk_sanitize_and_cap_their_text_with_distinct_body_and_label_rules() {
    let listener = Listener::start();
    let token = listener.login();
    let state = listener.get("/api/state", &token);
    let desk = field(&state, "desk");
    let Value::Arr(groups) = field(desk, "groups") else {
        panic!("desk groups");
    };
    assert_eq!(text(&groups[0], "group"), "needs_you");
    let row = only(&groups[0], "rows");
    assert_eq!(text(row, "slug"), "release");
    expect_text(text(row, "title"), &label("title").1, "desk title");
    expect_text(text(row, "reviewed"), &label("reviewed").1, "desk reviewed");
    expect_tagged_wait(row);
    let untagged = only(desk, "untagged");
    assert_eq!(text(untagged, "kind"), "decide");
    assert_eq!(field(untagged, "id"), &Value::Null);
    expect_text(
        text(untagged, "text"),
        &body("untagged wait").1,
        "untagged wait",
    );
    let Value::Str(gap) = only(desk, "gaps") else {
        panic!("gap text");
    };
    expect_text(gap, &body("gap").1, "gap line");

    let card = listener.get("/api/topic/release", &token);
    assert_eq!(text(&card, "slug"), "release");
    expect_text(
        text(&card, "reviewed"),
        &label("reviewed").1,
        "card reviewed",
    );
    expect_tagged_wait(&card);
    let fields = field(&card, "fields");
    let title = field(fields, "title");
    expect_text(text(title, "value"), &label("title").1, "card title");
    expect_origin(title);
    for key in ["what", "last", "next", "src", "close"] {
        let value = field(fields, key);
        expect_text(text(value, "value"), &body(key).1, key);
        expect_origin(value);
    }
    let history = only(&card, "history");
    expect_text(text(history, "text"), &body("history").1, "history text");
    expect_origin(history);
    let unreadable = listener.get("/api/topic/unreadable", &token);
    expect_text(
        text(&unreadable, "unreadable"),
        &body("unreadable").1,
        "unreadable text",
    );
}

#[test]
fn each_topic_state_has_its_group_and_a_wait_promotes_every_topic() {
    let fixture = topic::Source {
        session: "renamed-orchestrator".to_owned(),
        uuid: "0199c0de-aaaa-4890-abcd-ef0123456789".to_owned(),
        incarnation: 0,
        memo: b"2026-01-01T00:00:00Z\tlead\ttopic-needs\tstate: needs\n\
                2026-01-01T00:00:00Z\tlead\ttopic-moving\tstate: moving\n\
                2026-01-01T00:00:00Z\tlead\ttopic-quiet\tstate: quiet\n\
                2026-01-01T00:00:00Z\tlead\ttopic-parked\tstate: parked\n\
                2026-01-01T00:00:00Z\tlead\ttopic-closed\tstate: closed\n\
                2026-01-01T00:00:00Z\tlead\ttopic-unreadable\tstate: absent\n"
            .to_vec(),
    };
    let cards = topic::cards(&[fixture]);
    for (slug, word) in [
        ("needs", "needs_you"),
        ("moving", "moving"),
        ("quiet", "quiet"),
        ("parked", "parked"),
        ("closed", "closed"),
        ("unreadable", "unreadable"),
    ] {
        let card = cards
            .iter()
            .find(|card| card.slug == slug)
            .unwrap_or_else(|| panic!("card {slug}"));
        assert_eq!(topic::group(card, false).word(), word);
        assert_eq!(topic::group(card, true).word(), "needs_you");
    }
}
