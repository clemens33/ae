//! S1c wire, durable retry, role changes and send queue with real pane input.

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ae::json::{self, Value};
use ae::web::claim::{Claims, Lookup, Word};
use ae::web::clock::Clock;
use ae::web::send::{Flow, Hooks, Point, Sends};
use ae::web::server::{self, Followed, NoProbe, Options, Running};
use ae::web::state::LiveSource;

use super::{Rig, await_evidence};
use crate::cli;

const SOCKET_WAIT: Duration = Duration::from_secs(15);
const CARRY_WAIT: Duration = Duration::from_secs(50);
const DAY: u64 = 86_400;
const EPOCH: i64 = 1_795_000_000;
const TEXT: &str = "the owner asks exactly once";
const BODY_HASH: &str = "6d5fe0b4fc3f2bcac7b285ac9a7a04cab2b39cc6bd10e8d52ab2a3f8c207931e";
const UNKNOWN: &str = "unknown: may have landed, not resent";
const EXPIRED: &str = "expired draft id: send it as a new message";
const MISMATCH: &str = "this draft id was sent with other text or route";
const CHANGED: &str = "the orchestrator changed, reload";

fn role_rig(tag: &str) -> Rig {
    let rig = Rig::compact(tag);
    let overlay = rig.scratch.join("orchestrator.config");
    std::fs::write(&overlay, "[workspace]\norchestrator = true\n")
        .unwrap_or_else(|error| panic!("write role overlay: {error}"));
    let meta_path = rig.dir.join("meta");
    let meta =
        std::fs::read_to_string(&meta_path).unwrap_or_else(|error| panic!("read meta: {error}"));
    std::fs::write(
        meta_path,
        format!(
            "{meta}schema=2\nlayout=lead-pair\nmeta_agent=true\nlocal_config={}\n\
         main_pane={}\nseat.worker.0=side-old\nlaunch_id.worker.0=worker-launch\n",
            overlay.display(),
            rig.pane,
        ),
    )
    .unwrap_or_else(|error| panic!("write role meta: {error}"));
    std::fs::write(rig.scratch.join("config"), "[workspace]\n")
        .unwrap_or_else(|error| panic!("write global fixture config: {error}"));
    rig
}

fn change_meta(rig: &Rig, before: &str, after: &str) {
    let path = rig.dir.join("meta");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("read meta: {error}"));
    assert_eq!(raw.matches(before).count(), 1);
    let temp = rig.dir.join("fixture-meta-next");
    std::fs::write(&temp, raw.replacen(before, after, 1))
        .unwrap_or_else(|error| panic!("write fixture meta: {error}"));
    std::fs::rename(temp, path).unwrap_or_else(|error| panic!("publish fixture meta: {error}"));
}

/// The wall and monotonic readings are independent, as on a real host.
struct TestClock {
    base: Instant,
    elapsed: AtomicU64,
    wall: AtomicI64,
}

impl TestClock {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            base: Instant::now(),
            elapsed: AtomicU64::new(0),
            wall: AtomicI64::new(EPOCH),
        })
    }

    fn advance(&self, seconds: u64) {
        self.elapsed.fetch_add(seconds, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now(&self) -> Instant {
        self.base + Duration::from_secs(self.elapsed.load(Ordering::SeqCst))
    }

    fn epoch(&self) -> i64 {
        self.wall.load(Ordering::SeqCst)
    }
}

struct Reply {
    status: u16,
    value: Value,
}

impl Reply {
    fn expect(&self, status: u16, key: &str, word: &str) {
        assert_eq!(self.status, status, "{}", self.value.render());
        assert_eq!(
            self.value.get_str(key),
            Some(word),
            "{}",
            self.value.render()
        );
    }
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value
        .get_str(key)
        .unwrap_or_else(|| panic!("missing {key} in {}", value.render()))
}

fn send_wire(host: SocketAddr, wire: &str) -> Reply {
    let mut tcp = TcpStream::connect_timeout(&host, SOCKET_WAIT)
        .unwrap_or_else(|error| panic!("connect to {host}: {error}"));
    tcp.set_read_timeout(Some(SOCKET_WAIT))
        .unwrap_or_else(|error| panic!("set read deadline: {error}"));
    tcp.set_write_timeout(Some(SOCKET_WAIT))
        .unwrap_or_else(|error| panic!("set write deadline: {error}"));
    tcp.write_all(wire.as_bytes())
        .unwrap_or_else(|error| panic!("write request: {error}"));
    let mut reply = String::new();
    tcp.read_to_string(&mut reply)
        .unwrap_or_else(|error| panic!("read response: {error}"));
    let (header, body) = reply
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("HTTP framing: {reply}"));
    let status = header
        .split_ascii_whitespace()
        .nth(1)
        .and_then(|word| word.parse().ok())
        .unwrap_or_else(|| panic!("status: {header}"));
    Reply {
        status,
        value: json::parse(body).unwrap_or_else(|error| panic!("{header}: {error}: {body}")),
    }
}

fn post(host: SocketAddr, path: &str, value: &Value, bearer: &str) -> Reply {
    let token = if bearer.is_empty() {
        String::new()
    } else {
        format!("Authorization: Bearer {bearer}\r\n")
    };
    let body = value.render();
    let len = body.len();
    send_wire(
        host,
        &format!(
            "POST {path} HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\n{token}\
         Content-Type: application/json\r\nContent-Length: {len}\r\n\r\n{body}",
        ),
    )
}

#[derive(Clone)]
struct Draft {
    nonce: String,
    route: String,
    body: String,
}

impl Draft {
    fn json(&self) -> Value {
        Value::obj([
            ("nonce", Value::str(self.nonce.as_str())),
            ("route", Value::str(self.route.as_str())),
            ("text", Value::str(self.body.as_str())),
        ])
    }

    fn claim_name(&self) -> String {
        ae::install::sha256_hex(self.nonce.as_bytes())
    }
}

struct Web {
    listen: Option<Running>,
    addr: SocketAddr,
    token: String,
}

impl Web {
    fn live(rig: &Rig, time: &Arc<TestClock>, port: u16) -> Self {
        Self::start(rig, time, port, Sends::live(rig.scratch.clone()))
    }

    fn with_hook(rig: &Rig, time: &Arc<TestClock>, hooks: Arc<dyn Hooks>) -> Self {
        Self::start(
            rig,
            time,
            0,
            Sends {
                root: rig.scratch.clone(),
                hooks,
            },
        )
    }

    fn start(rig: &Rig, time: &Arc<TestClock>, port: u16, sends: Sends) -> Self {
        let run = start_listener(&rig.scratch, time, port, sends);
        let mut web = Self::login(run.addr(), &run.link());
        web.listen = Some(run);
        web
    }

    fn login(addr: SocketAddr, link: &str) -> Self {
        let (_, code) = link
            .split_once("#c=")
            .unwrap_or_else(|| panic!("missing one-time code"));
        let reply = post(
            addr,
            "/api/session",
            &Value::obj([("code", Value::str(code))]),
            "",
        );
        assert_eq!(reply.status, 200);
        Self {
            listen: None,
            addr,
            token: text(&reply.value, "session").to_owned(),
        }
    }

    fn handle(&self) -> &Running {
        self.listen
            .as_ref()
            .unwrap_or_else(|| panic!("running listener"))
    }

    fn get(&self, path: &str) -> Reply {
        let host = self.addr;
        let token = &self.token;
        send_wire(
            host,
            &format!(
                "GET {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\n\r\n"
            ),
        )
    }

    fn nonce(&self) -> String {
        let reply = self.get("/api/nonce");
        assert_eq!(reply.status, 200);
        let nonce = text(&reply.value, "nonce");
        assert!(is_hex(nonce, 112));
        nonce.to_owned()
    }

    fn route(&self, rig: &Rig) -> String {
        let reply = self.get("/api/state");
        assert_eq!(reply.status, 200);
        let route = reply
            .value
            .get("route")
            .unwrap_or_else(|| panic!("missing route key: {}", reply.value.render()));
        assert_eq!(text(route, "to"), rig.session);
        assert_eq!(text(route, "seat"), "tui");
        let id = text(route, "id");
        assert!(is_hex(id, 64));
        id.to_owned()
    }

    fn draft(&self, route: &str, body: &str) -> Draft {
        Draft {
            nonce: self.nonce(),
            route: route.to_owned(),
            body: body.to_owned(),
        }
    }

    fn submit(&self, draft: &Draft) -> Reply {
        post(self.addr, "/api/send", &draft.json(), &self.token)
    }

    fn wait_outcome(&self, draft: &Draft) -> Reply {
        await_evidence(CARRY_WAIT, || {
            let reply = self.submit(draft);
            if reply.status == 202 {
                reply.expect(202, "state", "queued");
                None
            } else {
                Some(reply)
            }
        })
        .unwrap_or_else(|| panic!("send never settled within {CARRY_WAIT:?}"))
    }

    fn send(&self, draft: &Draft) -> Reply {
        self.submit(draft).expect(202, "state", "queued");
        self.wait_outcome(draft)
    }
}

impl Drop for Web {
    fn drop(&mut self) {
        if let Some(run) = self.listen.take() {
            run.shutdown();
        }
    }
}

fn start_listener(root: &Path, time: &Arc<TestClock>, port: u16, sends: Sends) -> Running {
    server::start_with(
        Options {
            port,
            clock: time.clone(),
            source: Arc::new(LiveSource::new(root.to_path_buf(), root.join("config"))),
            probe: Arc::new(NoProbe),
            links: Arc::new(|_| {}),
            version: "web-send-test".to_owned(),
        },
        sends,
    )
    .unwrap_or_else(|error| panic!("start listener: {error}"))
}

fn is_hex(word: &str, len: usize) -> bool {
    word.len() == len
        && word
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn confirmed(reply: &Reply, rig: &Rig) -> String {
    reply.expect(200, "outcome", "sent");
    assert_eq!(text(&reply.value, "to"), rig.session);
    assert_eq!(text(&reply.value, "seat"), "tui");
    let id = text(&reply.value, "id");
    assert!(ae::tracked::is_request_id(id));
    id.to_owned()
}

fn openings(rig: &Rig) -> Vec<String> {
    rig.events()
        .lines()
        .filter_map(|line| {
            let event =
                json::parse(line).unwrap_or_else(|error| panic!("invalid event: {line}: {error}"));
            (event.get_str("actor") == Some("console:local")
                && event.get_str("action") == Some("ask"))
            .then(|| text(&event, "ref").to_owned())
        })
        .collect()
}

fn claim_file(rig: &Rig, draft: &Draft) -> std::path::PathBuf {
    rig.scratch.join("web/claims").join(draft.claim_name())
}

fn lookup(rig: &Rig, draft: &Draft) -> Lookup {
    Claims::open(&rig.scratch).lookup(&draft.claim_name())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cut {
    Claimed,
    BeforeInput,
    Carried,
    Opened,
}

impl Cut {
    fn of_point(point: Point) -> (Self, String) {
        match point {
            Point::Claimed { id } => (Self::Claimed, id),
            Point::BeforeInput { id } => (Self::BeforeInput, id),
            Point::Carried { id } => (Self::Carried, id),
            Point::Opened { id } => (Self::Opened, id),
        }
    }

    fn word(self) -> &'static str {
        match self {
            Self::Claimed => "claimed",
            Self::BeforeInput => "before-input",
            Self::Carried => "carried",
            Self::Opened => "opened",
        }
    }
}

struct OnePause {
    chosen: Cut,
    seen: Sender<String>,
    reply: Mutex<Option<Receiver<Flow>>>,
}

impl Hooks for OnePause {
    fn at(&self, point: Point) -> Flow {
        let (cut, id) = Cut::of_point(point);
        if cut != self.chosen {
            return Flow::Go;
        }
        let channel = self
            .reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let Some(channel) = channel else {
            return Flow::Go;
        };
        if self.seen.send(id).is_err() {
            return Flow::Halt;
        }
        channel
            .recv_timeout(Duration::from_secs(60))
            .unwrap_or(Flow::Halt)
    }
}

struct Pause {
    notice: Receiver<String>,
    respond: Option<Sender<Flow>>,
}

impl Pause {
    fn at(chosen: Cut) -> (Arc<dyn Hooks>, Self) {
        let (seen, notice) = mpsc::channel();
        let (respond, reply) = mpsc::channel();
        let hook = OnePause {
            chosen,
            seen,
            reply: Mutex::new(Some(reply)),
        };
        (
            Arc::new(hook),
            Self {
                notice,
                respond: Some(respond),
            },
        )
    }

    fn reached(&self) -> String {
        self.notice
            .recv_timeout(CARRY_WAIT)
            .unwrap_or_else(|error| panic!("send cut was not reached: {error}"))
    }

    fn finish(mut self, answer: Flow) {
        self.respond
            .take()
            .unwrap_or_else(|| panic!("unreleased cut"))
            .send(answer)
            .unwrap_or_else(|_| panic!("send worker still paused"));
    }
}

impl Drop for Pause {
    fn drop(&mut self) {
        if let Some(sender) = self.respond.take() {
            let _ = sender.send(Flow::Halt);
        }
    }
}

struct CrashPoint {
    root: PathBuf,
    cut: String,
}

fn publish_signal(root: &Path, name: &str, text: &str) {
    let temporary = root.join(format!("{name}.tmp"));
    std::fs::write(&temporary, text)
        .unwrap_or_else(|error| panic!("write witness {name}: {error}"));
    std::fs::rename(temporary, root.join(name))
        .unwrap_or_else(|error| panic!("publish witness {name}: {error}"));
}

impl Hooks for CrashPoint {
    fn at(&self, point: Point) -> Flow {
        let (cut, id) = Cut::of_point(point);
        if cut.word() != self.cut {
            return Flow::Go;
        }
        publish_signal(&self.root, "crash-point", &id);
        std::thread::park_timeout(Duration::from_secs(60));
        Flow::Halt
    }
}

#[test]
#[ignore = "the loss tests start and kill this listener-only child"]
fn crash_listener_child() {
    let root = PathBuf::from(
        std::env::var_os("AE_WEB_CRASH_ROOT").unwrap_or_else(|| panic!("scratch state root")),
    );
    let cut = std::env::var("AE_WEB_CRASH_POINT").unwrap_or_else(|error| panic!("cut: {error}"));
    let hooks = Arc::new(CrashPoint {
        root: root.clone(),
        cut,
    });
    let running = start_listener(
        &root,
        &TestClock::new(),
        0,
        Sends {
            root: root.clone(),
            hooks,
        },
    );
    publish_signal(&root, "crash-link", &running.link());
    std::thread::park_timeout(Duration::from_secs(60));
    running.shutdown();
    panic!("the parent did not stop this listener at the agreed cut");
}

fn signal(root: &Path, name: &str) -> String {
    let path = root.join(name);
    await_evidence(CARRY_WAIT, || std::fs::read_to_string(&path).ok())
        .unwrap_or_else(|| panic!("child did not publish {name}"))
}

fn crashing_child(rig: &Rig, point: Cut) -> cli::OwnedChild {
    let executable =
        std::env::current_exe().unwrap_or_else(|error| panic!("test executable: {error}"));
    cli::helper(&executable)
        .args([
            "--exact",
            "deliver::web_send_spec::crash_listener_child",
            "--ignored",
            "--nocapture",
        ])
        .env("AE_HOME", &rig.scratch)
        .env("CONFIG_FILE", rig.scratch.join("config"))
        .env("AE_WEB_CRASH_ROOT", &rig.scratch)
        .env("AE_WEB_CRASH_POINT", point.word())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("start crash listener: {error}"))
}

fn loss_at(tag: &str, point: Cut, inputs: usize, recorded: bool) {
    let rig = role_rig(tag);
    let mut child = crashing_child(&rig, point);
    let link = signal(&rig.scratch, "crash-link");
    let addr: SocketAddr = link
        .strip_prefix("http://")
        .and_then(|tail| tail.split_once('/'))
        .and_then(|(authority, _)| authority.parse().ok())
        .unwrap_or_else(|| panic!("loopback link"));
    let first = Web::login(addr, &link);
    let draft = first.draft(&first.route(&rig), TEXT);
    first.submit(&draft).expect(202, "state", "queued");
    let id = signal(&rig.scratch, "crash-point");

    let path = claim_file(&rig, &draft);
    let raw =
        std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("durable claim: {error}"));
    assert!(
        raw.lines()
            .any(|line| line.strip_prefix("id ") == Some(id.as_str()))
    );
    assert!(
        raw.lines()
            .any(|line| line.strip_prefix("payload ") == Some(BODY_HASH))
    );
    assert!(
        raw.lines()
            .any(|line| line.strip_prefix("route ") == Some(draft.route.as_str()))
    );
    assert!(!raw.lines().any(|line| line.starts_with("outcome ")));
    assert_eq!(rig.enter_count(), inputs, "pane receipt at {point:?}");
    if inputs == 1 {
        assert!(rig.submitted().contains(TEXT));
    } else {
        assert!(!ae::deliver::input_busy(
            &rig.server(),
            &rig.pane,
            ae::tool::InputModel::BorderDelimited,
        ));
    }
    let expected_openings: Vec<String> = recorded.then(|| id.clone()).into_iter().collect();
    assert_eq!(openings(&rig), expected_openings, "journal at {point:?}");
    first.submit(&draft).expect(202, "state", "queued");

    child
        .kill()
        .unwrap_or_else(|error| panic!("kill owned listener child: {error}"));
    let loss = cli::bounded(child, CARRY_WAIT).unwrap_or_else(|| panic!("reap listener child"));
    assert_eq!(loss.status.signal(), Some(9), "{loss:?}");
    let second = Web::live(&rig, &TestClock::new(), addr.port());
    let retry = second.submit(&draft);
    retry.expect(200, "outcome", "unknown");
    retry.expect(200, "words", UNKNOWN);
    assert_eq!(text(&retry.value, "id"), id);
    assert_eq!(rig.enter_count(), inputs);
    assert_eq!(openings(&rig), expected_openings);

    let fresh = second.draft(&second.route(&rig), "fresh control after simulated loss");
    let control = confirmed(&second.send(&fresh), &rig);
    assert_ne!(control, id);
    assert_eq!(rig.enter_count(), inputs + 1);
    assert_eq!(openings(&rig), [expected_openings, vec![control]].concat());
}

#[test]
fn loss_after_durable_claim_never_carries_on_retry() {
    loss_at("wcclaim", Cut::Claimed, 0, false);
}

#[test]
fn loss_after_carry_and_before_opening_never_carries_on_retry() {
    loss_at("wccarry", Cut::Carried, 1, false);
}

#[test]
fn loss_after_opening_and_before_outcome_never_carries_on_retry() {
    loss_at("wcopen", Cut::Opened, 1, true);
}

#[test]
fn a_completed_draft_keeps_its_receipt_across_restart_and_rejects_other_input() {
    let rig = role_rig("wcreplay");
    let time = TestClock::new();
    let first = Web::live(&rig, &time, 0);
    let route = first.route(&rig);
    let draft = first.draft(&route, TEXT);
    let unused = first.draft(&route, "unused listener A draft");
    let nonce_upper = Draft {
        nonce: draft.nonce.to_uppercase(),
        ..draft.clone()
    };
    assert_eq!(first.submit(&nonce_upper).status, 400);

    let initial = first.send(&draft);
    let id = confirmed(&initial, &rig);
    let changed_body = Draft {
        body: "different text".to_owned(),
        ..draft.clone()
    };
    let changed_route = Draft {
        route: "0".repeat(64),
        ..draft.clone()
    };
    assert_ne!(draft.route, changed_route.route);
    first.submit(&changed_body).expect(409, "error", MISMATCH);
    first.submit(&changed_route).expect(409, "error", MISMATCH);
    let replayed = first.submit(&draft);
    replayed.expect(200, "outcome", "sent");
    assert_eq!(replayed.value, initial.value);
    let port = first.handle().addr().port();
    drop(first);

    let second = Web::live(&rig, &time, port);
    assert_ne!(second.route(&rig), route);
    let replayed = second.submit(&draft);
    replayed.expect(200, "outcome", "sent");
    assert_eq!(replayed.value, initial.value);
    second.submit(&changed_body).expect(409, "error", MISMATCH);
    second.submit(&changed_route).expect(409, "error", MISMATCH);
    second.submit(&unused).expect(410, "error", EXPIRED);
    assert_eq!(rig.enter_count(), 1);
    assert_eq!(openings(&rig), [id]);
}

fn keep_bearer_through(web: &Web, time: &TestClock, mut seconds: u64) {
    const HOP: u64 = 10 * 60 * 60;
    while seconds > HOP {
        time.advance(HOP);
        assert_eq!(web.get("/api/nonce").status, 200);
        seconds -= HOP;
    }
    time.advance(seconds);
}

#[test]
fn nonce_age_advances_despite_a_wall_clock_rollback_and_a_pruned_retry_is_gone() {
    let rig = role_rig("wcage");
    let time = TestClock::new();
    let web = Web::live(&rig, &time, 0);
    let route = web.route(&rig);
    let boundary = web.draft(&route, "the 24-hour edge is still current");
    let expired = web.draft(&route, "this untouched nonce expires");
    let completed = web.draft(&route, TEXT);
    let first_id = confirmed(&web.send(&completed), &rig);

    time.wall.store(EPOCH - 3600, Ordering::SeqCst);
    keep_bearer_through(&web, &time, DAY);
    let edge_id = confirmed(&web.send(&boundary), &rig);
    time.advance(1);
    web.submit(&expired).expect(410, "error", EXPIRED);
    assert_eq!(rig.enter_count(), 2);
    assert_eq!(openings(&rig), [first_id.clone(), edge_id.clone()]);

    keep_bearer_through(&web, &time, 6 * DAY);
    let claims = Claims::open(&rig.scratch);
    if let Lookup::Known(initial) = claims.lookup(&completed.claim_name()) {
        claims.prune(initial.created + 7 * DAY + 1);
    }
    assert!(matches!(
        claims.lookup(&completed.claim_name()),
        Lookup::Absent
    ));
    web.submit(&completed).expect(410, "error", EXPIRED);
    assert_eq!(rig.enter_count(), 2);
    assert_eq!(openings(&rig), [first_id, edge_id]);
}

#[test]
fn an_unreadable_existing_claim_blocks_fresh_input_even_for_a_valid_nonce() {
    let rig = role_rig("wcdamage");
    let time = TestClock::new();
    let web = Web::live(&rig, &time, 0);
    let route = web.route(&rig);
    let badly_written = web.draft(&route, TEXT);
    let not_a_file = web.draft(&route, TEXT);
    std::fs::create_dir_all(rig.scratch.join("web/claims"))
        .unwrap_or_else(|error| panic!("claim fixture directory: {error}"));
    std::fs::write(claim_file(&rig, &badly_written), "ae-web-claim 1\npartial")
        .unwrap_or_else(|error| panic!("claim fixture bytes: {error}"));
    std::fs::create_dir(claim_file(&rig, &not_a_file))
        .unwrap_or_else(|error| panic!("claim fixture node: {error}"));
    for draft in [&badly_written, &not_a_file] {
        let reply = web.submit(draft);
        reply.expect(200, "outcome", "unknown");
        reply.expect(200, "words", UNKNOWN);
    }
    assert_eq!(rig.enter_count(), 0);
    assert!(openings(&rig).is_empty());
    let control = confirmed(
        &web.send(&web.draft(&route, "control on an unclaimed nonce")),
        &rig,
    );
    assert_eq!(rig.enter_count(), 1);
    assert_eq!(openings(&rig), [control]);
}

#[test]
fn a_same_uuid_pair_change_refuses_the_route_both_before_queue_and_inside_admission() {
    let rig = role_rig("wcpair");
    let time = TestClock::new();
    let (hooks, pause) = Pause::at(Cut::Claimed);
    let web = Web::with_hook(&rig, &time, hooks);
    let cut = pause;
    let shown = web.route(&rig);
    let stale = web.draft(&shown, TEXT);
    change_meta(&rig, "seat.worker.0=side-old", "seat.worker.0=side-new");
    web.submit(&stale).expect(409, "error", CHANGED);
    assert!(matches!(lookup(&rig, &stale), Lookup::Absent));

    time.advance(1);
    let current = web.route(&rig);
    assert_ne!(current, shown);
    let admitted = web.draft(&current, "pair changes between enqueue and admission");
    web.submit(&admitted).expect(202, "state", "queued");
    let claimed_id = cut.reached();
    change_meta(&rig, "seat.worker.0=side-new", "seat.worker.0=side-again");
    cut.finish(Flow::Go);
    web.wait_outcome(&admitted).expect(409, "error", CHANGED);
    match lookup(&rig, &admitted) {
        Lookup::Known(claim) => {
            assert_eq!(claim.id, claimed_id);
            assert!(
                matches!(claim.outcome, Some(outcome) if matches!(outcome.word, Word::Changed))
            );
        }
        _ => panic!("changed route claim must stand"),
    }
    assert_eq!(rig.enter_count(), 0);
    assert!(openings(&rig).is_empty());

    time.advance(1);
    let fresh = web.route(&rig);
    assert_ne!(fresh, current);
    let control = confirmed(&web.send(&web.draft(&fresh, TEXT)), &rig);
    assert_eq!(rig.enter_count(), 1);
    assert_eq!(openings(&rig), [control]);
    web.submit(&admitted).expect(409, "error", CHANGED);
}

#[test]
fn a_same_seat_launch_change_after_admission_refuses_before_any_input() {
    let rig = role_rig("wclaunch");
    let time = TestClock::new();
    let (hooks, pause) = Pause::at(Cut::BeforeInput);
    let web = Web::with_hook(&rig, &time, hooks);
    let cut = pause;
    let route = web.route(&rig);
    let draft = web.draft(&route, TEXT);
    web.submit(&draft).expect(202, "state", "queued");
    let claimed_id = cut.reached();
    change_meta(&rig, "launch_id.main=tok-rig", "launch_id.main=tok-new");
    cut.finish(Flow::Go);

    let refused = web.wait_outcome(&draft);
    refused.expect(200, "outcome", "refused");
    assert_eq!(text(&refused.value, "id"), claimed_id);
    assert!(!text(&refused.value, "words").is_empty());
    assert_eq!(rig.enter_count(), 0);
    assert!(!ae::deliver::input_busy(
        &rig.server(),
        &rig.pane,
        ae::tool::InputModel::BorderDelimited
    ));
    assert!(openings(&rig).is_empty());

    time.advance(1);
    let fresh = web.route(&rig);
    assert_ne!(fresh, route);
    let control = confirmed(&web.send(&web.draft(&fresh, TEXT)), &rig);
    assert_eq!(rig.enter_count(), 1);
    assert_eq!(openings(&rig), [control]);
}

#[test]
fn owner_can_use_the_fifth_slot_and_the_sixth_refusal_is_the_console_line() {
    let rig = role_rig("wccap");
    let time = TestClock::new();
    let web = Web::live(&rig, &time, 0);
    let route = web.route(&rig);
    let mut expected_ids = Vec::new();
    for number in 1..=4 {
        let id = format!("ae-20261122T200000Z-{number:08x}");
        let opening = Value::obj([
            ("ts", Value::str("2026-11-22T20:00:00Z")),
            ("actor", Value::str("console:local")),
            ("action", Value::str("ask")),
            ("target", Value::str("tui")),
            ("actor_session", Value::str(rig.session.as_str())),
            ("target_session", Value::str(rig.session.as_str())),
            ("target_slot", Value::str("main")),
            ("ref", Value::str(id.as_str())),
        ]);
        ae::store::open(&rig.dir)
            .append_event(&format!("{}\n", opening.render()))
            .unwrap_or_else(|error| panic!("seed an open console ask: {error}"));
        expected_ids.push(id);
    }

    let fifth = confirmed(&web.send(&web.draft(&route, TEXT)), &rig);
    expected_ids.push(fifth);
    let sixth = web.send(&web.draft(&route, "the sixth open owner ask"));
    sixth.expect(200, "outcome", "refused");
    let line = format!(
        "cap-full: 5/5 console asks open; open ids: {}; owner /close can withdraw an ask",
        expected_ids.join(", "),
    );
    sixth.expect(200, "words", &line);
    assert_eq!(rig.enter_count(), 1);
    assert_eq!(openings(&rig), expected_ids);
}

struct ExecFailure {
    delivery_hook: Arc<dyn Hooks>,
    called: Sender<Vec<String>>,
    release: Mutex<Receiver<()>>,
}

impl Hooks for ExecFailure {
    fn at(&self, point: Point) -> Flow {
        self.delivery_hook.at(point)
    }

    fn exec(&self, argv: &[String]) -> std::io::Error {
        let _ = self.called.send(argv.to_vec());
        let _ = self
            .release
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .recv_timeout(Duration::from_secs(60));
        std::io::Error::other("exec fixture refusal")
    }
}

#[test]
fn reads_and_duplicate_retries_continue_during_a_send_and_follow_reopens_after_failure() {
    let rig = role_rig("wcqueue");
    let time = TestClock::new();
    let (delivery_hook, pause) = Pause::at(Cut::BeforeInput);
    let (called, watched) = mpsc::channel();
    let (unblock_exec, release) = mpsc::channel();
    let hooks = Arc::new(ExecFailure {
        delivery_hook,
        called,
        release: Mutex::new(release),
    });
    let web = Web::with_hook(&rig, &time, hooks);
    let cut = pause;
    let route = web.route(&rig);
    let first = web.draft(&route, "carrying one");
    let second = web.draft(&route, "queued two");
    let third = web.draft(&route, "queued three");
    let fourth = web.draft(&route, "cannot enter the closed gate");
    web.submit(&first).expect(202, "state", "queued");
    let first_id = cut.reached();

    assert!(matches!(
        web.handle().follow(&[], Duration::ZERO),
        Followed::Busy
    ));
    assert!(matches!(watched.try_recv(), Err(mpsc::TryRecvError::Empty)));
    web.submit(&second).expect(202, "state", "queued");
    assert!(matches!(lookup(&rig, &second), Lookup::Absent));
    web.submit(&second).expect(202, "state", "queued");
    let conflict = Draft {
        body: "other pending text".to_owned(),
        ..second.clone()
    };
    web.submit(&conflict).expect(409, "error", MISMATCH);
    web.submit(&third).expect(202, "state", "queued");
    web.submit(&fourth).expect(503, "error", "busy, retry");
    time.advance(1);
    assert_eq!(web.get("/api/state").status, 200);
    assert_eq!(web.get("/api/nonce").status, 200);
    assert_eq!(rig.enter_count(), 0);

    let argv = vec!["/fixture/ae-successor".to_owned(), "web".to_owned()];
    std::thread::scope(|scope| {
        let unblock = unblock_exec;
        let follow = scope.spawn(|| web.handle().follow(&argv, CARRY_WAIT));
        cut.finish(Flow::Go);
        let actual_argv = watched
            .recv_timeout(CARRY_WAIT)
            .unwrap_or_else(|error| panic!("quiescent exec: {error}"));
        assert_eq!(actual_argv, argv);
        assert_eq!(rig.enter_count(), 3);
        let ids = openings(&rig);
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0], first_id);
        for expected in ["carrying one", "queued two", "queued three"] {
            assert_eq!(rig.submitted().matches(expected).count(), 1);
        }
        assert_eq!(web.get("/api/state").status, 200);
        web.submit(&fourth).expect(503, "error", "busy, retry");
        assert!(matches!(lookup(&rig, &fourth), Lookup::Absent));
        unblock
            .send(())
            .unwrap_or_else(|_| panic!("exec fixture was still blocked"));
        let result = follow
            .join()
            .unwrap_or_else(|_| panic!("follow test thread"));
        assert!(matches!(result, Followed::Failed(why) if why == "exec fixture refusal"));
    });

    let fresh = confirmed(&web.send(&fourth), &rig);
    assert_eq!(rig.enter_count(), 4);
    assert_eq!(openings(&rig).last(), Some(&fresh));
}
