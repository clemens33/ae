//! S1b authenticated topic route and shared snapshot through the real listener.

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use ae::json::{self, Value};
use ae::web::clock::ManualClock;
use ae::web::server::{self, NoProbe, Options, Running};
use ae::web::state::{Read, Source};
use ae::web::topics::{self, Sticky};

const WAIT: Duration = Duration::from_secs(5);
const SESSION: &str = "wire-renamed";
const UUID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const SECOND_APPEND_AT: &str = "2026-10-04T08:00:00Z";
const FIRST_APPEND_AT: &str = "2026-10-05T08:00:00Z";
const VERSION: &str = "topic-wire-fixture";

fn snapshot(root: &std::path::Path, title: &str) -> Read {
    let place = root.join("sessions").join(SESSION);
    std::fs::create_dir_all(&place).unwrap_or_else(|error| panic!("fixture: {error}"));
    let overlay = root.join("orchestrator.config");
    let socket = root.join("not-a-running-server");
    std::fs::write(&overlay, "[workspace]\norchestrator = true\n")
        .unwrap_or_else(|error| panic!("overlay fixture: {error}"));
    std::fs::write(
        place.join("meta"),
        format!(
            "schema=2\nsession={SESSION}\nsession_id={UUID}\nlayout=lead-pair\nseat.main=lead\n\
         meta_agent=true\nlocal_config={}\ntmux_server_kind=socket\ntmux_server={}\n",
            overlay.display(),
            socket.display(),
        ),
    )
    .unwrap_or_else(|error| panic!("meta fixture: {error}"));
    std::fs::write(
        place.join("memo.tsv"),
        format!(
            "{FIRST_APPEND_AT}\tlead\ttopic-release\ttitle: overwritten title | state: quiet\n\
         {SECOND_APPEND_AT}\tlead\ttopic-release\ttitle: {title} | what: visible \\| literal\n\
         2026-10-03T08:00:00Z\tlead\ttopic-release\tunknown: still reviewed\n",
        ),
    )
    .unwrap_or_else(|error| panic!("memo fixture: {error}"));
    let board =
        ae::desk::parse_input(b"DECIDE\nD1 | Choose #release today\nD2 | Untagged decision\n")
            .unwrap_or_else(|error| panic!("board fixture: {error}"));
    ae::deskstore::set(
        &place,
        board,
        ae::time::Timestamp::from_epoch(1_795_000_000),
        "lead",
    )
    .unwrap_or_else(|error| panic!("publish board fixture: {error}"));
    Read {
        desk: topics::collect(root, &mut Sticky::default()),
        ..Read::default()
    }
}

struct Snapshots {
    old: Read,
    new: Read,
    changed: AtomicBool,
    calls: AtomicUsize,
}

impl Source for Snapshots {
    fn read(&self) -> Read {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.changed.load(Ordering::SeqCst) {
            self.new.clone()
        } else {
            self.old.clone()
        }
    }
}

struct Listener(Option<Running>);

impl Drop for Listener {
    fn drop(&mut self) {
        if let Some(running) = self.0.take() {
            running.shutdown();
        }
    }
}

struct Response {
    status: u16,
    etag: Option<String>,
    body: String,
}

impl Response {
    fn json(&self) -> Value {
        json::parse(&self.body).unwrap_or_else(|error| panic!("JSON response: {error}"))
    }
}

fn exchange(host: SocketAddr, wire: &str) -> Response {
    let mut socket = TcpStream::connect_timeout(&host, WAIT)
        .unwrap_or_else(|error| panic!("connect to {host}: {error}"));
    socket
        .set_read_timeout(Some(WAIT))
        .unwrap_or_else(|error| panic!("read timeout: {error}"));
    socket
        .set_write_timeout(Some(WAIT))
        .unwrap_or_else(|error| panic!("write timeout: {error}"));
    socket
        .write_all(wire.as_bytes())
        .unwrap_or_else(|error| panic!("send: {error}"));
    let mut response = String::new();
    socket
        .read_to_string(&mut response)
        .unwrap_or_else(|error| panic!("response: {error}"));
    let (head, body) = response
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("HTTP framing"));
    let status = head
        .split_ascii_whitespace()
        .nth(1)
        .and_then(|word| word.parse().ok())
        .unwrap_or_else(|| panic!("status line: {head}"));
    let etag = head
        .split("\r\n")
        .find_map(|line| line.strip_prefix("ETag: "))
        .map(str::to_owned);
    Response {
        status,
        etag,
        body: body.to_owned(),
    }
}

fn bearer(running: &Running) -> String {
    let link = running.link();
    let (_, code) = link
        .split_once("#c=")
        .unwrap_or_else(|| panic!("expected startup fragment"));
    let host = running.addr();
    let input = format!(r#"{{"code":"{code}"}}"#);
    let len = input.len();
    let response = exchange(
        host,
        &format!(
            "POST /api/session HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\n\
         Content-Type: application/json\r\nContent-Length: {len}\r\n\r\n{input}",
        ),
    );
    assert_eq!(response.status, 200);
    response
        .json()
        .get_str("session")
        .unwrap_or_else(|| panic!("session secret"))
        .to_owned()
}

fn get(host: SocketAddr, path: &str, token: Option<&str>, etag: Option<&str>) -> Response {
    use std::fmt::Write as _;

    let mut wire = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n");
    if let Some(token) = token {
        let _ = write!(wire, "Authorization: Bearer {token}\r\n");
    }
    if let Some(etag) = etag {
        let _ = write!(wire, "If-None-Match: {etag}\r\n");
    }
    wire.push_str("\r\n");
    exchange(host, &wire)
}

fn array<'a>(node: &'a Value, key: &str) -> &'a [Value] {
    let Some(Value::Arr(items)) = node.get(key) else {
        panic!("missing JSON array {key}");
    };
    items
}

fn property<'a>(node: &'a Value, name: &str) -> &'a Value {
    node.get(name)
        .unwrap_or_else(|| panic!("missing JSON property {name}"))
}

fn row(snapshot: &Value) -> &Value {
    let desk = property(snapshot, "desk");
    let groups = array(desk, "groups");
    assert_eq!(
        groups
            .iter()
            .filter_map(|group| group.get_str("group"))
            .collect::<Vec<_>>(),
        [
            "needs_you",
            "moving",
            "quiet",
            "parked",
            "closed",
            "unreadable",
        ]
    );
    let rows = array(&groups[0], "rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get_str("slug"), Some("release"));
    assert_eq!(rows[0].get_str("state"), Some("quiet"));
    let untagged = array(desk, "untagged");
    assert_eq!(untagged.len(), 1);
    assert_eq!(untagged[0].get_str("text"), Some("Untagged decision"));
    &rows[0]
}

#[test]
fn topic_detail_and_state_share_a_snapshot_and_keep_independent_etags() {
    let scratch = crate::cli::OwnedScratch::root("wtpc", "route");
    let injection = Arc::new(Snapshots {
        old: snapshot(scratch.path(), "Old release"),
        new: snapshot(scratch.path(), "New release"),
        changed: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
    });
    let clock = Arc::new(ManualClock::new(1_795_000_000));
    let listener = Listener(Some(
        server::start(Options {
            port: 0,
            clock: clock.clone(),
            source: injection.clone(),
            probe: Arc::new(NoProbe),
            links: Arc::new(|_| {}),
            version: VERSION.into(),
        })
        .unwrap_or_else(|error| panic!("start listener: {error}")),
    ));
    let running = listener
        .0
        .as_ref()
        .unwrap_or_else(|| panic!("running listener"));
    let host = running.addr();
    let token = bearer(running);
    let auth = Some(token.as_str());

    let snapshot_response = get(host, "/api/state", auth, None);
    assert_eq!(snapshot_response.status, 200);
    let state_value = snapshot_response.json();
    let state_row = row(&state_value);
    assert_eq!(state_row.get_str("title"), Some("Old release"));
    assert_eq!(state_row.get_str("reviewed"), Some("2026-10-03T08:00:00Z"));
    let calls = injection.calls.load(Ordering::SeqCst);
    assert!(calls > 0);
    injection.changed.store(true, Ordering::SeqCst);

    let card_response = get(
        host,
        "/api/topic/release",
        auth,
        snapshot_response.etag.as_deref(),
    );
    assert_eq!(card_response.status, 200);
    assert_eq!(injection.calls.load(Ordering::SeqCst), calls);
    let tag = card_response
        .etag
        .as_deref()
        .unwrap_or_else(|| panic!("topic ETag"));
    assert_ne!(snapshot_response.etag.as_deref(), Some(tag));
    let card = card_response.json();
    assert_eq!(card.get_str("slug"), Some("release"));
    assert_eq!(card.get_str("group"), Some("needs_you"));
    assert_eq!(card.get_str("reviewed"), Some("2026-10-03T08:00:00Z"));
    let fields = property(&card, "fields");
    let title = property(fields, "title");
    assert_eq!(title.get_str("value"), Some("Old release"));
    assert_eq!(title.get_str("session"), Some(SESSION));
    assert_eq!(title.get_str("uuid"), Some(UUID));
    assert_eq!(title.get_str("at"), Some(SECOND_APPEND_AT));
    assert_eq!(
        property(fields, "state").get_str("at"),
        Some(FIRST_APPEND_AT)
    );
    assert_eq!(
        property(fields, "what").get_str("value"),
        Some("visible | literal")
    );
    assert_eq!(fields.get("close"), Some(&Value::Null));
    assert_eq!(card.get("unreadable"), Some(&Value::Null));
    assert_eq!(array(&card, "waits"), array(state_row, "waits"));
    assert_eq!(array(&card, "waits")[0].get_str("kind"), Some("decide"));
    assert_eq!(
        array(&card, "history")[0].get_str("text"),
        Some("unknown: still reviewed")
    );

    let matching = get(host, "/api/topic/release", auth, Some(tag));
    assert_eq!(matching.status, 304);
    assert!(matching.body.is_empty());
    assert_eq!(get(host, "/api/topic/release", None, Some(tag)).status, 401);
    for invalid in [
        "/api/topic/unknown",
        "/api/topic/Release",
        "/api/topic/release/extra",
    ] {
        let response = get(host, invalid, auth, None);
        assert_eq!(response.status, 404, "{invalid}");
        assert_eq!(response.json().get_str("error"), Some("no such topic"));
    }

    clock.advance(Duration::from_secs(1));
    let changed = get(host, "/api/topic/release", auth, Some(tag));
    assert_eq!(changed.status, 200);
    assert_ne!(changed.etag.as_deref(), Some(tag));
    let next_card = changed.json();
    assert_eq!(
        property(property(&next_card, "fields"), "title").get_str("value"),
        Some("New release")
    );
    let next_state = get(host, "/api/state", auth, None).json();
    assert_eq!(row(&next_state).get_str("title"), Some("New release"));
    assert_eq!(injection.calls.load(Ordering::SeqCst), calls + 1);
}

#[allow(
    clippy::expect_used,
    reason = "the frozen spec uses expect in its fixtures and its bytes stay as nav froze them"
)]
#[path = "web_revisit_spec.rs"]
mod web_revisit_spec;
