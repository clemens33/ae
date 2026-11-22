//! S1a HTTP acceptance through the real loopback listener. The source and clock
//! are fixture inputs. Parsing, authentication, routing and sockets are real.

use std::io::{self, Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ae::json::{self, Value};
use ae::web::clock::ManualClock;
use ae::web::server::{self, Cut, Options, Probe, Running, Seen};
use ae::web::state::{Read, Source};

const WAIT: Duration = Duration::from_secs(5);
const EPOCH: i64 = 1_795_000_000;
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; \
connect-src 'self'; img-src 'self'; manifest-src 'self'; base-uri 'none'; form-action 'none'; \
frame-ancestors 'none'";
const VERSION: &str = "2026.11.108-fixture";

#[derive(Default)]
struct SourceFixture {
    reads: AtomicUsize,
    changed: AtomicBool,
}

impl Source for SourceFixture {
    fn read(&self) -> Read {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let mut snapshot = Read::default();
        if self.changed.load(Ordering::SeqCst) {
            snapshot
                .chat
                .coverage
                .push("opposed fixture content".into());
        }
        snapshot
    }
}

struct EventProbe(Sender<Seen>);

impl Probe for EventProbe {
    fn saw(&self, event: Seen) {
        let _ = self.0.send(event);
    }
}

struct Rig {
    running: Option<Running>,
    clock: Arc<ManualClock>,
    source: Arc<SourceFixture>,
    links: Arc<Mutex<Vec<String>>>,
    events: Receiver<Seen>,
}

impl Rig {
    fn start(port: u16) -> Self {
        let clock = Arc::new(ManualClock::new(EPOCH));
        let source = Arc::new(SourceFixture::default());
        let links = Arc::new(Mutex::new(Vec::new()));
        let received = Arc::clone(&links);
        let (sender, events) = mpsc::channel();
        let running = server::start(Options {
            port,
            clock: clock.clone(),
            source: source.clone(),
            probe: Arc::new(EventProbe(sender)),
            links: Arc::new(move |line| {
                received
                    .lock()
                    .unwrap_or_else(|_| panic!("fixture link queue"))
                    .push(line.to_owned());
            }),
            version: VERSION.into(),
        })
        .unwrap_or_else(|error| panic!("start loopback server: {error}"));
        let addr = running.addr();
        assert_eq!(addr.ip(), Ipv4Addr::LOCALHOST);
        assert_ne!(addr.port(), 0);
        Self {
            running: Some(running),
            clock,
            source,
            links,
            events,
        }
    }

    fn handle(&self) -> &Running {
        self.running
            .as_ref()
            .unwrap_or_else(|| panic!("server handle"))
    }

    fn addr(&self) -> SocketAddr {
        self.handle().addr()
    }

    fn host(&self) -> String {
        self.addr().to_string()
    }

    fn startup_code(&self) -> String {
        code_in(&self.handle().link(), self.addr())
    }

    fn fresh_links(&self) -> Vec<String> {
        self.links
            .lock()
            .unwrap_or_else(|_| panic!("fixture link queue"))
            .clone()
    }

    fn event(&self, description: &str, matches: impl Fn(&Seen) -> bool) -> Seen {
        let end = Instant::now() + WAIT;
        loop {
            let event = self
                .events
                .recv_timeout(end.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| panic!("server event {description}: {error}"));
            if matches(&event) {
                return event;
            }
        }
    }

    fn request(&self, method: &str, target: &str, fields: &[(&str, &str)], body: &str) -> Reply {
        self.at_host(&self.host(), method, target, fields, body)
    }

    fn at_host(
        &self,
        host: &str,
        method: &str,
        target: &str,
        fields: &[(&str, &str)],
        body: &str,
    ) -> Reply {
        let wire = request_bytes(host, method, target, fields, body);
        self.exchange(&wire)
    }

    fn exchange(&self, wire: &str) -> Reply {
        let mut stream = connect(self.addr());
        write_to(&mut stream, wire);
        Reply::read(stream)
    }

    fn redeem_at(&self, host: &str, origin: Option<&str>, code: &str) -> Reply {
        let mut fields = vec![("Content-Type", "application/json")];
        if let Some(origin) = origin {
            fields.push(("Origin", origin));
        }
        let body = format!(r#"{{"code":"{code}"}}"#);
        self.at_host(host, "POST", "/api/session", &fields, &body)
    }

    fn redeem(&self, code: &str) -> Reply {
        self.redeem_at(&self.host(), Some(&format!("http://{}", self.host())), code)
    }

    fn login(&self) -> String {
        session(&self.redeem(&self.startup_code()))
    }

    fn state(&self, secret: &str, etag: Option<&str>) -> Reply {
        let auth = format!("Bearer {secret}");
        let mut fields = vec![("Authorization", auth.as_str())];
        if let Some(etag) = etag {
            fields.push(("If-None-Match", etag));
        }
        self.request("GET", "/api/state", &fields, "")
    }

    fn stop(&mut self) {
        if let Some(running) = self.running.take() {
            running.shutdown();
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Debug)]
struct Reply {
    status: u16,
    fields: Vec<String>,
    body: String,
}

impl Reply {
    fn read(stream: TcpStream) -> Self {
        let bytes = observed_close(stream);
        let wire = String::from_utf8(bytes)
            .unwrap_or_else(|error| panic!("text response is UTF-8: {error}"));
        let (framing, body) = wire
            .split_once("\r\n\r\n")
            .unwrap_or_else(|| panic!("complete HTTP response: {wire:?}"));
        let mut fields = framing.split("\r\n");
        let line = fields.next().unwrap_or_default();
        let status = line
            .strip_prefix("HTTP/1.1 ")
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|code| code.parse().ok())
            .unwrap_or_else(|| panic!("HTTP/1.1 status: {line:?}"));
        Self {
            status,
            fields: fields.map(str::to_owned).collect(),
            body: body.to_owned(),
        }
    }

    fn field(&self, key: &str) -> Option<&str> {
        let mut matching = self
            .fields
            .iter()
            .filter_map(|line| line.split_once(": "))
            .filter(|(name, _)| name.eq_ignore_ascii_case(key));
        let value = matching.next().map(|(_, value)| value);
        assert!(matching.next().is_none(), "duplicate {key}: {self:?}");
        value
    }

    fn json(&self) -> Value {
        json::parse(&self.body).unwrap_or_else(|error| panic!("JSON response {self:?}: {error}"))
    }
}

fn code_in(link: &str, addr: SocketAddr) -> String {
    let prefix = format!("http://{addr}/#c=");
    let code = link
        .split_ascii_whitespace()
        .last()
        .and_then(|field| field.strip_prefix(&prefix))
        .unwrap_or_else(|| panic!("printed fragment link for {addr}: {link:?}"));
    assert!(lower_hex(code, 32), "{link:?}");
    code.to_owned()
}

fn lower_hex(text: &str, count: usize) -> bool {
    text.len() == count
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn session(reply: &Reply) -> String {
    assert_eq!(reply.status, 200, "{reply:?}");
    let token = reply.json();
    let secret = token
        .get_str("session")
        .unwrap_or_else(|| panic!("session secret: {reply:?}"));
    assert!(lower_hex(secret, 64), "{reply:?}");
    secret.to_owned()
}

fn connect(addr: SocketAddr) -> TcpStream {
    let client = TcpStream::connect_timeout(&addr, WAIT)
        .unwrap_or_else(|error| panic!("connect to {addr}: {error}"));
    client
        .set_read_timeout(Some(WAIT))
        .unwrap_or_else(|error| panic!("read timeout: {error}"));
    client
        .set_write_timeout(Some(WAIT))
        .unwrap_or_else(|error| panic!("write timeout: {error}"));
    client
}

fn write_to(client: &mut TcpStream, text: &str) {
    client
        .write_all(text.as_bytes())
        .unwrap_or_else(|error| panic!("write HTTP request: {error}"));
}

fn observed_close(mut client: TcpStream) -> Vec<u8> {
    let mut received = Vec::new();
    if let Err(error) = client.read_to_end(&mut received) {
        assert_eq!(
            error.kind(),
            io::ErrorKind::ConnectionReset,
            "wait for close"
        );
    }
    received
}

fn request_bytes(
    host: &str,
    method: &str,
    target: &str,
    fields: &[(&str, &str)],
    body: &str,
) -> String {
    use std::fmt::Write as _;

    let mut wire = format!("{method} {target} HTTP/1.1\r\nHost: {host}\r\n");
    for (key, value) in fields {
        let _ = write!(wire, "{key}: {value}\r\n");
    }
    if method == "POST" {
        let _ = write!(wire, "Content-Length: {}\r\n", body.len());
    }
    wire.push_str("\r\n");
    wire.push_str(body);
    wire
}

fn security_fields(reply: &Reply) {
    let expected_csp = format!("Content-Security-Policy: {CSP}");
    let pinned = [
        "Cache-Control: no-store",
        expected_csp.as_str(),
        "X-Content-Type-Options: nosniff",
        "Referrer-Policy: no-referrer",
        "Cross-Origin-Opener-Policy: same-origin",
        "Cross-Origin-Resource-Policy: same-origin",
    ];
    assert!(
        reply
            .fields
            .windows(pinned.len())
            .any(|rows| rows == pinned),
        "security headers are wire-byte pins, {reply:?}"
    );
    for header in &reply.fields {
        let lower = header.to_ascii_lowercase();
        assert!(!lower.starts_with("access-control-"), "{reply:?}");
        assert!(!lower.starts_with("set-cookie:"), "{reply:?}");
    }
    for line in pinned {
        let (key, value) = line.split_once(": ").unwrap_or_default();
        assert_eq!(reply.field(key), Some(value), "{reply:?}");
    }
    assert_eq!(reply.field("Connection"), Some("close"), "{reply:?}");
    if reply.status == 304 {
        assert_eq!(reply.field("Content-Length"), None);
        assert_eq!(reply.field("Content-Type"), None);
        assert!(reply.body.is_empty(), "{reply:?}");
    } else {
        let length = reply.body.len().to_string();
        assert_eq!(
            reply.field("Content-Length"),
            Some(length.as_str()),
            "{reply:?}"
        );
    }
}

#[test]
fn host_is_exact_and_each_post_requires_its_own_origin() {
    let rig = Rig::start(0);
    let port = rig.addr().port();
    let hosts = [
        rig.host(),
        format!("localhost:{port}"),
        format!("[::1]:{port}"),
    ];
    for host in &hosts {
        let shell = rig.at_host(host, "GET", "/", &[], "");
        assert_eq!(shell.status, 200, "{host}, {shell:?}");
    }
    for host in [
        "example.invalid".to_owned(),
        format!("127.0.0.1.example.invalid:{port}"),
        format!("LOCALHOST:{port}"),
        format!("127.0.0.1:{}", if port == 7480 { 7481 } else { 7480 }),
        "localhost".to_owned(),
    ] {
        let refused = rig.at_host(&host, "GET", "/", &[], "");
        assert_eq!(refused.status, 421, "{host}, {refused:?}");
    }
    let missing = rig.exchange("GET / HTTP/1.1\r\n\r\n");
    assert_eq!(missing.status, 421, "{missing:?}");
    let code = rig.startup_code();
    let foreign = "http://example.invalid";
    let localhost = format!("http://{}", hosts[1]);
    for origin in [None, Some(foreign), Some(localhost.as_str())] {
        let refused = rig.redeem_at(&hosts[0], origin, &code);
        assert_eq!(refused.status, 403, "{origin:?}, {refused:?}");
    }
    let host_first = rig.redeem_at("foreign.invalid", None, &code);
    assert_eq!(host_first.status, 421, "{host_first:?}");
    let matched = rig.redeem_at(&hosts[1], Some(&localhost), &code);
    let _ = session(&matched);
}

#[test]
fn bearer_rejection_precedes_dispatch_and_etag_matching() {
    let rig = Rig::start(0);
    let secret = rig.login();
    let auth = format!("Bearer {secret}");
    let fresh = rig.state(&secret, None);
    assert_eq!(fresh.status, 200, "{fresh:?}");
    let etag = fresh.field("ETag").unwrap_or_else(|| panic!("state ETag"));
    let matching = rig.state(&secret, Some(etag));
    assert_eq!(matching.status, 304, "{matching:?}");
    security_fields(&fresh);
    security_fields(&matching);

    let unauthenticated = [
        rig.request("GET", "/api/state", &[("If-None-Match", etag)], ""),
        rig.request("GET", "/api/unknown", &[], ""),
        rig.request("GET", "/api/session", &[], ""),
        rig.request("GET", "/api/state", &[("Authorization", &secret)], ""),
        rig.request(
            "GET",
            "/api/state",
            &[("Authorization", "Bearer wrong")],
            "",
        ),
    ];
    for denial in unauthenticated {
        assert_eq!(denial.status, 401, "{denial:?}");
        assert_eq!(
            denial.json().get_str("error"),
            Some("no session: open the newest link from ae web")
        );
        assert_eq!(denial.field("ETag"), None);
        security_fields(&denial);
    }

    for (target, expected) in [("/api/unknown", 404), ("/api/session", 405)] {
        let reply = rig.request("GET", target, &[("Authorization", &auth)], "");
        assert_eq!(reply.status, expected, "{reply:?}");
    }
    let origin = format!("http://{}", rig.host());
    let wrong_method = rig.request(
        "POST",
        "/api/state",
        &[("Authorization", &auth), ("Origin", &origin)],
        "{}",
    );
    assert_eq!(wrong_method.status, 405, "{wrong_method:?}");
    let no_origin = rig.request("POST", "/api/state", &[], "{}");
    assert_eq!(no_origin.status, 403, "{no_origin:?}");
}

#[test]
fn static_shell_is_public_with_security_headers_and_no_range_or_upgrade() {
    let rig = Rig::start(0);
    let shell = rig.request("GET", "/", &[], "");
    assert_eq!(shell.status, 200, "{shell:?}");
    assert!(shell.body.contains("/assets/app.mjs"), "{shell:?}");
    assert!(
        shell
            .field("Content-Type")
            .is_some_and(|kind| kind.starts_with("text/html"))
    );
    security_fields(&shell);
    let ranged = rig.request(
        "GET",
        "/",
        &[
            ("Range", "bytes=0-1"),
            ("Connection", "Upgrade"),
            ("Upgrade", "websocket"),
        ],
        "",
    );
    assert_eq!(ranged.status, 200, "{ranged:?}");
    assert_eq!(ranged.body, shell.body);
    assert_eq!(ranged.field("Content-Range"), None);
    security_fields(&ranged);

    let asset = rig.request("GET", "/assets/app.mjs", &[], "");
    assert_eq!(asset.status, 200, "{asset:?}");
    assert!(asset.body.contains("/api/"));
    let missing = rig.request("GET", "/assets/VENDOR.md", &[], "");
    assert_eq!(missing.status, 404, "{missing:?}");
    security_fields(&missing);
}

#[test]
fn framing_and_duplicate_security_fields_fail_at_the_wire() {
    let rig = Rig::start(0);
    let host = rig.host();
    let malformed = [
        (format!("GET / HTTP/1.0\r\nHost: {host}\r\n\r\n"), 505),
        (format!("HEAD / HTTP/1.1\r\nHost: {host}\r\n\r\n"), 405),
        (
            format!("POST /api/session HTTP/1.1\r\nHost: {host}\r\n\r\n"),
            411,
        ),
        (
            format!(
                "POST /api/session HTTP/1.1\r\nHost: {host}\r\nTransfer-Encoding: chunked\r\n\r\n"
            ),
            411,
        ),
        (
            format!("POST /api/session HTTP/1.1\r\nHost: {host}\r\nContent-Length: 65537\r\n\r\n"),
            413,
        ),
    ];
    for (wire, status) in malformed {
        let refused = rig.exchange(&wire);
        assert_eq!(refused.status, status, "{wire:?}, {refused:?}");
        security_fields(&refused);
    }
    let duplicates = [
        ("Host", "localhost"),
        ("Origin", "http://localhost"),
        ("Authorization", "Bearer junk"),
        ("If-None-Match", "\"etag\""),
        ("Content-Length", "0"),
    ];
    for (header, value) in duplicates.into_iter().skip(1) {
        let wire = format!(
            "GET / HTTP/1.1\r\nHost: {host}\r\n{header}: {value}\r\n{}: {value}\r\n\r\n",
            header.to_ascii_lowercase()
        );
        let refused = rig.exchange(&wire);
        assert_eq!(refused.status, 400, "{wire:?}, {refused:?}");
    }
    let duplicate_host = format!("GET / HTTP/1.1\r\nHost: {host}\r\nhost: {host}\r\n\r\n");
    assert_eq!(rig.exchange(&duplicate_host).status, 400);
    assert_eq!(rig.request("GET", "/", &[], "").status, 200);
}

#[test]
fn one_connection_carries_one_response_even_if_two_requests_arrive() {
    let rig = Rig::start(0);
    let first = request_bytes(&rig.host(), "GET", "/", &[("Connection", "keep-alive")], "");
    let second = request_bytes(&rig.host(), "GET", "/api/state", &[], "");
    let reply = rig.exchange(&format!("{first}{second}"));
    assert_eq!(reply.status, 200, "{reply:?}");
    assert!(!reply.body.contains("HTTP/1.1 401"));
    security_fields(&reply);
}

#[test]
fn session_json_refuses_wrong_shapes_and_a_redeemed_code() {
    let rig = Rig::start(0);
    let origin = format!("http://{}", rig.host());
    for input in [r#"{}"#, r#"{"code":7}"#, r#"["code"]"#, r#"{"code":"a",}"#] {
        let response = rig.request(
            "POST",
            "/api/session",
            &[("Origin", &origin), ("Content-Type", "application/json")],
            input,
        );
        assert_eq!(response.status, 400, "{input}: {response:?}");
        assert_eq!(response.json().get_str("session"), None);
    }

    let code = rig.startup_code();
    let secret = session(&rig.redeem(&code));
    let replay = rig.redeem(&code);
    assert!((400..500).contains(&replay.status), "{replay:?}");
    assert_eq!(replay.json().get_str("session"), None);
    assert_eq!(rig.state(&secret, None).status, 200);
}

#[test]
fn the_listener_applies_the_ten_minute_code_lifetime() {
    let within = Rig::start(0);
    let fresh = within.startup_code();
    within.clock.advance(Duration::from_secs(599));
    let _ = session(&within.redeem(&fresh));

    let beyond = Rig::start(0);
    let expired = beyond.startup_code();
    beyond.clock.advance(Duration::from_secs(601));
    let refusal = beyond.redeem(&expired);
    assert!((400..500).contains(&refusal.status), "{refusal:?}");
    assert_eq!(refusal.json().get_str("session"), None);
}

#[test]
fn empty_session_requests_print_one_fresh_link_per_ten_seconds() {
    let rig = Rig::start(0);
    let original = rig.startup_code();
    let _ = session(&rig.redeem(&original));
    rig.clock.advance(Duration::from_secs(11));
    for _ in 0..2 {
        assert_eq!(rig.request("GET", "/api/state", &[], "").status, 401);
    }
    let first = rig.fresh_links();
    assert_eq!(first.len(), 1, "{first:?}");
    let new_code = code_in(&first[0], rig.addr());
    assert_ne!(new_code, original);
    let _ = session(&rig.redeem(&new_code));

    rig.clock.advance(Duration::from_secs(9));
    assert_eq!(rig.request("GET", "/api/state", &[], "").status, 401);
    assert_eq!(rig.fresh_links(), first);
    rig.clock.advance(Duration::from_secs(2));
    assert_eq!(rig.request("GET", "/api/state", &[], "").status, 401);
    let second = rig.fresh_links();
    assert_eq!(second.len(), 2, "{second:?}");
    let newest = code_in(&second[1], rig.addr());
    assert_ne!(newest, new_code);
    let _ = session(&rig.redeem(&newest));
}

#[test]
fn code_and_bearer_do_not_cross_listener_lifetimes_on_the_same_origin() {
    let mut first = Rig::start(0);
    let addr = first.addr();
    let old_secret = first.login();
    assert_eq!(first.state(&old_secret, None).status, 200);
    first.clock.advance(Duration::from_secs(11));
    assert_eq!(first.request("GET", "/api/state", &[], "").status, 401);
    let links = first.fresh_links();
    assert_eq!(links.len(), 1, "{links:?}");
    let unused = code_in(&links[0], addr);
    first.stop();

    let second = Rig::start(addr.port());
    assert_eq!(second.addr(), addr);
    assert_eq!(second.request("GET", "/", &[], "").status, 200);
    let new_secret = second.login();
    let refused_code = second.redeem(&unused);
    assert!(
        (400..500).contains(&refused_code.status),
        "{refused_code:?}"
    );
    assert_eq!(refused_code.json().get_str("session"), None);
    assert_eq!(second.state(&old_secret, None).status, 401);
    assert_eq!(second.state(&new_secret, None).status, 200);
}

#[test]
fn snapshot_reuse_is_shared_and_an_unchanged_poll_keeps_its_etag() {
    let rig = Rig::start(0);
    let one = rig.login();
    rig.clock.advance(Duration::from_secs(11));
    assert_eq!(rig.request("GET", "/api/state", &[], "").status, 401);
    let links = rig.fresh_links();
    assert_eq!(links.len(), 1);
    let two = session(&rig.redeem(&code_in(&links[0], rig.addr())));
    assert_ne!(one, two);

    let reads_before = rig.source.reads.load(Ordering::SeqCst);
    let initial = rig.state(&one, None);
    assert_eq!(initial.status, 200, "{initial:?}");
    let etag = initial
        .field("ETag")
        .unwrap_or_else(|| panic!("initial ETag"));
    let digest = ae::install::sha256_hex(initial.body.as_bytes());
    assert_eq!(etag, format!("\"{}\"", &digest[..16]));
    let snapshot = initial.json();
    assert_eq!(snapshot.get("v"), Some(&Value::Num(1)));
    assert_eq!(snapshot.get_str("version"), Some(VERSION));
    assert!(
        matches!(snapshot.get("read_at"), Some(Value::Num(at)) if (EPOCH..=EPOCH + 11).contains(at))
    );
    assert_eq!(
        snapshot
            .get("orchestrator")
            .and_then(|seat| seat.get_str("state")),
        Some("absent")
    );
    let current = rig.state(&two, None);
    assert_eq!(current.status, 200, "{current:?}");
    assert_eq!(current.body, initial.body);
    assert_eq!(current.field("ETag"), Some(etag));
    let first_read = rig.source.reads.load(Ordering::SeqCst);
    assert_eq!(first_read, reads_before + 1);

    for (seconds, reads) in [(2, first_read + 1), (30, first_read + 2)] {
        rig.clock.advance(Duration::from_secs(seconds));
        let unchanged = rig.state(&two, Some(etag));
        assert_eq!(unchanged.status, 304, "{unchanged:?}");
        assert_eq!(unchanged.field("ETag"), Some(etag));
        assert_eq!(rig.source.reads.load(Ordering::SeqCst), reads);
    }

    rig.source.changed.store(true, Ordering::SeqCst);
    let same_window = rig.state(&one, Some(etag));
    assert_eq!(same_window.status, 304, "{same_window:?}");
    assert_eq!(rig.source.reads.load(Ordering::SeqCst), first_read + 2);
    rig.clock.advance(Duration::from_secs(1));
    let changed = rig.state(&one, Some(etag));
    assert_eq!(changed.status, 200, "{changed:?}");
    assert!(changed.field("ETag").is_some_and(|new| new != etag));
    let changed_json = changed.json();
    assert_eq!(changed_json.get("read_at"), Some(&Value::Num(EPOCH + 44)));
    assert_eq!(
        changed_json
            .get("chat")
            .and_then(|chat| chat.get("coverage")),
        Some(&Value::Arr(vec![Value::str("opposed fixture content")]))
    );
    assert_eq!(rig.source.reads.load(Ordering::SeqCst), first_read + 3);
}

#[test]
fn the_request_deadline_does_not_slide_when_a_slow_client_makes_progress() {
    let rig = Rig::start(0);
    let mut stalled = connect(rig.addr());
    write_to(&mut stalled, "G");
    let Seen::FirstByte { worker } = rig.event("first byte of incomplete request", |event| {
        matches!(event, Seen::FirstByte { .. })
    }) else {
        panic!("first read worker");
    };
    assert_eq!(rig.request("GET", "/", &[], "").status, 200);

    rig.clock.advance(Duration::from_secs(9));
    write_to(&mut stalled, "ET ");
    let _ = rig.event(
        "same worker has consumed more bytes at nine seconds",
        |event| matches!(event, Seen::ReadProgress { worker: seen, bytes: 4 } if *seen == worker),
    );
    rig.clock.advance(Duration::from_secs(2));
    // A timeout may already have closed the socket before this wake-up write.
    let _ = stalled.write_all(b"/");
    let _ = rig.event("original whole request deadline", |event| {
        matches!(event, Seen::Cut { worker: seen, why: Cut::RequestDeadline } if *seen == worker)
    });
    let closed = observed_close(stalled);
    assert!(closed.is_empty() || closed.starts_with(b"HTTP/1.1 408 "));
    assert_eq!(rig.request("GET", "/", &[], "").status, 200);
}

#[test]
fn four_reads_and_sixteen_queued_connections_yield_an_acceptor_503() {
    let rig = Rig::start(0);
    let mut blockers = Vec::new();
    for _ in 0..4 {
        let mut client = connect(rig.addr());
        write_to(&mut client, "G");
        let _ = rig.event("incomplete request reaches a worker", |event| {
            matches!(event, Seen::FirstByte { .. })
        });
        blockers.push(client);
    }
    let get_index = request_bytes(&rig.host(), "GET", "/", &[], "");
    let mut queue = Vec::new();
    for depth in 1..=16 {
        let mut client = connect(rig.addr());
        write_to(&mut client, &get_index);
        let _ = rig.event(
            "a distinct request enters the acceptor queue",
            |event| matches!(event, Seen::Queued { depth: seen } if *seen == depth),
        );
        queue.push(client);
    }
    let overflow = connect(rig.addr());
    let _ = rig.event("acceptor refuses a full queue", |event| {
        matches!(event, Seen::Refused503)
    });
    let refusal = Reply::read(overflow);
    assert_eq!(refusal.status, 503, "{refusal:?}");
    security_fields(&refusal);
    drop(blockers);
    for client in queue {
        let reply = Reply::read(client);
        assert_eq!(reply.status, 200, "{reply:?}");
    }
}
