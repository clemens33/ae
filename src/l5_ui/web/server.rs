//! The one listener of `ae web`: 127.0.0.1 only, one acceptor, a bounded
//! queue and a fixed pool of read workers, each request and each response
//! under its own monotonic deadline.
//!
//! The acceptor never reads: it queues a connection or, with the queue full,
//! writes the 503 itself. A worker reads one request under [`REQUEST_LIFE`],
//! answers it under [`RESPONSE_LIFE`] and closes; a slow client costs at most
//! one worker for that long.

use std::io::{self, Read as _, Write as _};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ring::rand::{SecureRandom as _, SystemRandom};

use super::assets;
use super::auth::{self, Auth};
use super::clock::Clock;
use super::http::{self, Head, Method, Parsed, Response};
use super::send::{self, Epoch, Sending, Sends};
use super::state::{self, Source};
use crate::json::Value;

/// The read workers.
pub const WORKERS: usize = 4;
/// The connections the acceptor queues while every worker is busy.
pub const QUEUE: usize = 16;
/// The whole time a request may take to arrive, from its worker's take.
pub const REQUEST_LIFE: Duration = Duration::from_secs(10);
/// The whole time a response may take to leave, from its first write.
pub const RESPONSE_LIFE: Duration = Duration::from_secs(10);
/// The longest single socket wait, so the clock is asked at least this often.
const POLL: Duration = Duration::from_secs(1);
/// The bytes one write hands the socket.
const CHUNK: usize = 16 * 1024;
/// The longest wait for each drain read of a lingering close.
const LINGER_POLL: Duration = Duration::from_millis(250);
/// The most drain reads of a lingering close.
const LINGER_READS: usize = 8;
/// The least time between two fresh links printed for pages without a session.
pub const LINK_GAP: Duration = Duration::from_secs(10);
/// The refusal words a page without a session shows.
pub const NO_SESSION: &str = "no session: open the newest link from ae web";

/// What the server saw, told to a [`Probe`] at the moment it happens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    /// The acceptor took a connection.
    Accepted,
    /// The acceptor queued it; `depth` connections now wait.
    Queued {
        /// The connections waiting, this one included.
        depth: usize,
    },
    /// The queue was full: the acceptor wrote the 503 itself.
    Refused503,
    /// Worker `worker` took a queued connection.
    Taken {
        /// The worker's index.
        worker: usize,
    },
    /// The first bytes of the request arrived.
    FirstByte {
        /// The worker's index.
        worker: usize,
    },
    /// A later read landed; `bytes` arrived on this connection in all.
    ReadProgress {
        /// The worker's index.
        worker: usize,
        /// The bytes received on this connection so far.
        bytes: usize,
    },
    /// The connection was cut before an answer.
    Cut {
        /// The worker's index.
        worker: usize,
        /// Why.
        why: Cut,
    },
    /// An answer left whole.
    Answered {
        /// The worker's index.
        worker: usize,
        /// Its status.
        status: u16,
    },
}

/// Why a connection was cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cut {
    /// The request did not arrive whole within [`REQUEST_LIFE`].
    RequestDeadline,
    /// The response did not leave whole within [`RESPONSE_LIFE`].
    ResponseDeadline,
    /// The peer closed or the socket failed.
    Closed,
}

/// Who hears what the server saw: a test orders its steps by it.
pub trait Probe: Send + Sync {
    /// The server saw `seen`.
    fn saw(&self, seen: Seen);
}

/// The probe of a server nobody watches.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoProbe;

impl Probe for NoProbe {
    fn saw(&self, _seen: Seen) {}
}

/// How to start a server.
pub struct Options {
    /// The port; 0 asks the system for a free one.
    pub port: u16,
    /// The clock every deadline and expiry reads.
    pub clock: Arc<dyn Clock>,
    /// What `/api/state` shows.
    pub source: Arc<dyn Source>,
    /// Who hears what the server saw.
    pub probe: Arc<dyn Probe>,
    /// Where a fresh link line goes: the terminal `ae web` runs in.
    pub links: Arc<dyn Fn(&str) + Send + Sync>,
    /// The core version `/api/state` names.
    pub version: String,
}

/// Why a server did not start.
#[derive(Debug)]
pub enum StartError {
    /// Something listens on the port already.
    Busy(u16),
    /// The bind failed otherwise.
    Bind(String),
    /// The system's random source failed.
    Random,
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy(port) => write!(f, "port {port} in use; not starting"),
            Self::Bind(why) => write!(f, "cannot listen: {why}"),
            Self::Random => f.write_str("the system random source failed"),
        }
    }
}

/// A running server.
pub struct Running {
    addr: SocketAddr,
    link: String,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
    sending: Option<Arc<Sending>>,
}

/// What a [`Running::follow`] that returned did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Followed {
    /// A send was still queued or carrying when the wait ran out: nothing was
    /// replaced and sends are admitted again.
    Busy,
    /// The exec failed, and why: sends are admitted again.
    Failed(String),
}

impl Running {
    /// Where it listens.
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The link with the code minted at start.
    #[must_use]
    pub fn link(&self) -> String {
        self.link.clone()
    }

    /// Replace this process with `argv` once no send is queued or carrying:
    /// close send admission (a send while closed is 503), then wait at most
    /// `wait` for the queue and the carry to empty. Returns only when it did
    /// not: the sends still in flight, or the exec's error; admission is open
    /// again either way.
    #[must_use]
    pub fn follow(&self, argv: &[String], wait: Duration) -> Followed {
        let Some(sending) = &self.sending else {
            return Followed::Failed("this listener sends nothing".to_owned());
        };
        if !sending.drain(wait) {
            sending.reopen();
            return Followed::Busy;
        }
        let why = sending.exec(argv);
        sending.reopen();
        Followed::Failed(why.to_string())
    }

    /// Stop accepting, let every worker finish and join every thread. The
    /// listener's key, codes and sessions end with it.
    pub fn shutdown(self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the acceptor out of its accept; a failed connect means it is gone.
        drop(TcpStream::connect(self.addr));
        for thread in self.threads {
            drop(thread.join());
        }
    }
}

/// What every thread of one server shares.
struct Shared {
    port: u16,
    clock: Arc<dyn Clock>,
    source: Arc<dyn Source>,
    probe: Arc<dyn Probe>,
    links: Arc<dyn Fn(&str) + Send + Sync>,
    version: String,
    auth: Mutex<Auth>,
    /// The listener key: it signs codes, sessions, routes and nonces.
    key: [u8; 32],
    random: SystemRandom,
    cache: state::Cache,
    /// The send half, when this listener sends.
    sending: Option<Arc<Sending>>,
    last_link: Mutex<Option<Instant>>,
    waiting: AtomicUsize,
    stop: Arc<AtomicBool>,
}

/// Bind 127.0.0.1:`port` and start the acceptor and the workers, with no send
/// route: `/api/nonce` and `/api/send` answer 404. The server is ready when
/// this returns.
///
/// # Errors
///
/// [`StartError::Busy`] when the port is taken, [`StartError::Bind`] for any
/// other bind failure, [`StartError::Random`] when no key could be made.
pub fn start(options: Options) -> Result<Running, StartError> {
    start_inner(options, None)
}

/// [`start`] with the send route and its one send worker, carrying to the
/// orchestrator of `sends.root`.
///
/// # Errors
///
/// [`start`]'s.
pub fn start_with(options: Options, sends: Sends) -> Result<Running, StartError> {
    start_inner(options, Some(sends))
}

fn start_inner(options: Options, sends: Option<Sends>) -> Result<Running, StartError> {
    let random = SystemRandom::new();
    let mut key = [0_u8; 32];
    random.fill(&mut key).map_err(|_| StartError::Random)?;
    let listener = listen(options.port)?;
    let addr = listener
        .local_addr()
        .map_err(|why| StartError::Bind(why.to_string()))?;
    let stop = Arc::new(AtomicBool::new(false));
    let epoch = Epoch::new(Arc::clone(&options.clock));
    let (sending, jobs) = match sends {
        Some(sends) => {
            let (sending, jobs) = Sending::new(sends, key, epoch);
            (Some(Arc::clone(&sending)), Some((sending, jobs)))
        }
        None => (None, None),
    };
    let shared = Arc::new(Shared {
        port: addr.port(),
        clock: options.clock,
        source: options.source,
        probe: options.probe,
        links: options.links,
        version: options.version,
        auth: Mutex::new(Auth::new(key)),
        key,
        random,
        cache: state::Cache::default(),
        sending: sending.clone(),
        last_link: Mutex::new(None),
        waiting: AtomicUsize::new(0),
        stop: Arc::clone(&stop),
    });
    let code = shared.mint().ok_or(StartError::Random)?;
    let link = format!("http://127.0.0.1:{}/#c={code}", addr.port());
    let mut threads = Vec::with_capacity(WORKERS + 2);
    if let Err(why) = start_threads(&shared, listener, jobs, &mut threads) {
        // A partial start: the failed spawn dropped the queue's sender, so
        // the read workers end on their next take, and the send worker ends
        // on `stop` within its poll. Nothing outlives this error.
        stop.store(true, Ordering::SeqCst);
        for thread in threads {
            drop(thread.join());
        }
        return Err(why);
    }
    Ok(Running {
        addr,
        link,
        stop,
        threads,
        sending,
    })
}

/// Start the read workers, the send worker when `jobs` is given, and the
/// acceptor last, pushing each handle onto `threads` as it starts. The queue's
/// sender lives only in this call or the acceptor, so a failed spawn leaves it
/// dropped by the time this returns.
fn start_threads(
    shared: &Arc<Shared>,
    listener: TcpListener,
    jobs: Option<(Arc<Sending>, Receiver<send::Job>)>,
    threads: &mut Vec<JoinHandle<()>>,
) -> Result<(), StartError> {
    let (queue, taken) = std::sync::mpsc::sync_channel::<TcpStream>(QUEUE);
    let taken = Arc::new(Mutex::new(taken));
    for worker in 0..WORKERS {
        let (shared, taken) = (Arc::clone(shared), Arc::clone(&taken));
        threads.push(spawn(&format!("ae-web-read-{worker}"), move || {
            work(worker, &shared, &taken);
        })?);
    }
    if let Some((sending, jobs)) = jobs {
        let body = send::worker(sending, jobs, Arc::clone(&shared.stop));
        threads.push(spawn("ae-web-send", body)?);
    }
    let accepting = Arc::clone(shared);
    threads.push(spawn("ae-web-accept", move || {
        accept(&listener, &accepting, &queue);
    })?);
    Ok(())
}

/// The one listener door: 127.0.0.1 only.
fn listen(port: u16) -> Result<TcpListener, StartError> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the one ae web listener, loopback 127.0.0.1 only — see clippy.toml"
    )]
    let bound = TcpListener::bind(("127.0.0.1", port));
    bound.map_err(|why| match why.kind() {
        io::ErrorKind::AddrInUse => StartError::Busy(port),
        _ => StartError::Bind(why.to_string()),
    })
}

fn spawn(name: &str, run: impl FnOnce() + Send + 'static) -> Result<JoinHandle<()>, StartError> {
    std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(run)
        .map_err(|why| StartError::Bind(why.to_string()))
}

/// Queue each accepted connection, or answer 503 when the queue is full.
fn accept(listener: &TcpListener, shared: &Shared, queue: &SyncSender<TcpStream>) {
    for stream in listener.incoming() {
        if shared.stop.load(Ordering::SeqCst) {
            return;
        }
        let Ok(stream) = stream else { continue };
        shared.probe.saw(Seen::Accepted);
        let depth = shared.waiting.fetch_add(1, Ordering::SeqCst) + 1;
        match queue.try_send(stream) {
            Ok(()) => shared.probe.saw(Seen::Queued { depth }),
            Err(TrySendError::Full(mut stream)) => {
                shared.waiting.fetch_sub(1, Ordering::SeqCst);
                let busy = Response::refusal(503, "busy, retry");
                drop(stream.set_write_timeout(Some(POLL)));
                drop(stream.write_all(&http::head(&busy)));
                drop(stream.write_all(http::body(&busy)));
                drop(stream.shutdown(Shutdown::Both));
                shared.probe.saw(Seen::Refused503);
            }
            Err(TrySendError::Disconnected(_)) => return,
        }
    }
}

/// Take queued connections until the acceptor is gone.
fn work(worker: usize, shared: &Shared, taken: &Mutex<Receiver<TcpStream>>) {
    loop {
        let next = taken.lock().unwrap_or_else(PoisonError::into_inner).recv();
        let Ok(stream) = next else { return };
        shared.waiting.fetch_sub(1, Ordering::SeqCst);
        shared.probe.saw(Seen::Taken { worker });
        serve(worker, shared, stream);
    }
}

/// Why a request was not read whole.
enum Stop {
    /// It broke the subset: answer this status.
    Refuse(u16),
    /// The connection was cut.
    Cut(Cut),
    /// The server stops: close without a word.
    Quiet,
}

/// One connection: read its request, answer it, close.
fn serve(worker: usize, shared: &Shared, mut stream: TcpStream) {
    let response = match read_request(worker, shared, &mut stream) {
        Ok((head, body)) => answer(shared, &head, &body),
        Err(Stop::Refuse(status)) => Response::refusal(status, http::reason(status)),
        Err(Stop::Cut(why)) => {
            shared.probe.saw(Seen::Cut { worker, why });
            if why == Cut::RequestDeadline {
                // A courtesy the deadline does not wait for: one bounded try.
                let late = Response::refusal(408, http::reason(408));
                drop(stream.set_write_timeout(Some(POLL)));
                drop(stream.write_all(&http::head(&late)));
                drop(stream.write_all(http::body(&late)));
                linger(&mut stream);
            }
            drop(stream.shutdown(Shutdown::Both));
            return;
        }
        Err(Stop::Quiet) => {
            drop(stream.shutdown(Shutdown::Both));
            return;
        }
    };
    let status = response.status;
    match write_response(shared, &mut stream, &response) {
        Ok(()) => {
            shared.probe.saw(Seen::Answered { worker, status });
            linger(&mut stream);
        }
        Err(why) => shared.probe.saw(Seen::Cut { worker, why }),
    }
    drop(stream.shutdown(Shutdown::Both));
}

/// Close the write side and drain what the client still sends, briefly, so
/// unread request bytes do not turn the close into a reset that destroys the
/// answer before the client reads it.
fn linger(stream: &mut TcpStream) {
    drop(stream.shutdown(Shutdown::Write));
    drop(stream.set_read_timeout(Some(LINGER_POLL)));
    let mut sink = [0_u8; 4096];
    let mut drained = 0_usize;
    for _ in 0..LINGER_READS {
        match stream.read(&mut sink) {
            Ok(0) | Err(_) => return,
            Ok(read) => {
                drained = drained.saturating_add(read);
                if drained > http::BODY_MAX {
                    return;
                }
            }
        }
    }
}

/// The request head and body, or why there is none to answer.
fn read_request(
    worker: usize,
    shared: &Shared,
    stream: &mut TcpStream,
) -> Result<(Head, Vec<u8>), Stop> {
    let deadline = shared.clock.now() + REQUEST_LIFE;
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 4096];
    let mut want: Option<(Head, usize, usize)> = None;
    loop {
        if let Some((_, _, total)) = &want
            && buf.len() >= *total
        {
            break;
        }
        if want.is_none() {
            match http::parse_head(&buf) {
                Ok(Parsed::Done {
                    head,
                    head_len,
                    body_len,
                }) => {
                    want = Some((head, head_len, head_len + body_len));
                    continue;
                }
                Ok(Parsed::Partial) => {}
                Err(status) => return Err(Stop::Refuse(status)),
            }
        }
        // `left` is never zero here, so the wait it sets is never "none".
        let left = deadline.saturating_duration_since(shared.clock.now());
        if left.is_zero() {
            return Err(Stop::Cut(Cut::RequestDeadline));
        }
        drop(stream.set_read_timeout(Some(left.min(POLL))));
        match stream.read(&mut chunk) {
            Ok(0) => return Err(Stop::Cut(Cut::Closed)),
            Ok(read) => {
                buf.extend_from_slice(chunk.get(..read).unwrap_or_default());
                shared.probe.saw(if buf.len() == read {
                    Seen::FirstByte { worker }
                } else {
                    Seen::ReadProgress {
                        worker,
                        bytes: buf.len(),
                    }
                });
            }
            Err(why) if waited(&why) => {}
            Err(_) => return Err(Stop::Cut(Cut::Closed)),
        }
        if shared.stop.load(Ordering::SeqCst) {
            return Err(Stop::Quiet);
        }
        // A read that lands at or after the deadline completes nothing.
        if deadline
            .saturating_duration_since(shared.clock.now())
            .is_zero()
        {
            return Err(Stop::Cut(Cut::RequestDeadline));
        }
    }
    let Some((head, head_len, total)) = want else {
        return Err(Stop::Refuse(400));
    };
    let body = buf
        .get(head_len..total)
        .map(<[u8]>::to_vec)
        .unwrap_or_default();
    Ok((head, body))
}

/// Whether a socket call only ran out its own timeout.
fn waited(why: &io::Error) -> bool {
    matches!(
        why.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
    )
}

/// Write `response` whole under [`RESPONSE_LIFE`] from the first write. The
/// clock is asked after every socket call, timed out or not, so a client that
/// stops reading is cut at the deadline however full its buffers are.
fn write_response(shared: &Shared, stream: &mut TcpStream, response: &Response) -> Result<(), Cut> {
    let deadline = shared.clock.now() + RESPONSE_LIFE;
    let head = http::head(response);
    for part in [head.as_slice(), http::body(response)] {
        let mut sent = 0;
        while sent < part.len() {
            let left = deadline.saturating_duration_since(shared.clock.now());
            if left.is_zero() {
                return Err(Cut::ResponseDeadline);
            }
            if shared.stop.load(Ordering::SeqCst) {
                return Err(Cut::Closed);
            }
            drop(stream.set_write_timeout(Some(left.min(POLL))));
            let end = sent.saturating_add(CHUNK).min(part.len());
            match stream.write(part.get(sent..end).unwrap_or_default()) {
                Ok(0) => return Err(Cut::Closed),
                Ok(wrote) => sent += wrote,
                Err(why) if waited(&why) => {}
                Err(_) => return Err(Cut::Closed),
            }
            // A write that lands at or after the deadline completes nothing.
            if deadline
                .saturating_duration_since(shared.clock.now())
                .is_zero()
            {
                return Err(Cut::ResponseDeadline);
            }
        }
    }
    Ok(())
}

/// The answer to one whole request: Host, then Origin for a POST, then the
/// bearer for every API route but the code exchange, then the route.
fn answer(shared: &Shared, head: &Head, body: &[u8]) -> Response {
    let Some(host) = head
        .header("host")
        .filter(|host| auth::host_ok(host, shared.port))
    else {
        return Response::refusal(421, "wrong host");
    };
    if head.method == Method::Post && !auth::origin_ok(head.header("origin"), host) {
        return Response::refusal(403, "wrong origin");
    }
    let path = head.path();
    if head.method == Method::Post && path == "/api/session" {
        return shared.session(body);
    }
    if path == "/api" || path.starts_with("/api/") {
        if !shared.bearer(head) {
            shared.fresh_link();
            return Response::refusal(401, NO_SESSION);
        }
        if let Some(slug) = path.strip_prefix("/api/topic/") {
            return match head.method {
                Method::Get => shared.topic(head, slug),
                Method::Post => Response::refusal(405, http::reason(405)),
            };
        }
        return match (head.method, path, &shared.sending) {
            (Method::Get, "/api/state", _) => shared.state(head),
            (Method::Get, "/api/nonce", Some(sending)) => sending.nonce(shared.random16()),
            (Method::Post, "/api/send", Some(sending)) => sending.post(body),
            (_, "/api/state" | "/api/session", _) | (_, "/api/nonce" | "/api/send", Some(_)) => {
                Response::refusal(405, http::reason(405))
            }
            _ => Response::refusal(404, http::reason(404)),
        };
    }
    match assets::find(path) {
        Some(asset) if head.method == Method::Get => Response {
            status: 200,
            content_type: asset.content_type,
            body: asset.bytes.into(),
            etag: None,
        },
        Some(_) => Response::refusal(405, http::reason(405)),
        None => Response::refusal(404, http::reason(404)),
    }
}

impl Shared {
    /// Sixteen fresh random bytes, or `None` when the random source failed.
    fn random16(&self) -> Option<[u8; 16]> {
        let mut random = [0_u8; 16];
        self.random.fill(&mut random).ok()?;
        Some(random)
    }

    /// A fresh one-time code, or `None` when the random source failed.
    fn mint(&self) -> Option<String> {
        let mut random = [0_u8; 16];
        self.random.fill(&mut random).ok()?;
        let now = self.clock.now();
        Some(
            self.auth
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .mint_code(random, now),
        )
    }

    /// Print a fresh link, at most once per [`LINK_GAP`].
    fn fresh_link(&self) {
        let now = self.clock.now();
        let mut last = self
            .last_link
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if last.is_some_and(|at| now.saturating_duration_since(at) < LINK_GAP) {
            return;
        }
        if let Some(code) = self.mint() {
            *last = Some(now);
            (self.links)(&format!(
                "ae web: new link http://127.0.0.1:{}/#c={code}",
                self.port
            ));
        }
    }

    /// Whether the request carries a live session secret.
    fn bearer(&self, head: &Head) -> bool {
        let Some(secret) = head
            .header("authorization")
            .and_then(|value| value.strip_prefix("Bearer "))
        else {
            return false;
        };
        let now = self.clock.now();
        self.auth
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .check(secret, now)
    }

    /// `POST /api/session`: a code for a session secret. A body without a
    /// string `code` is malformed (400); a code that does not redeem is
    /// refused (401) and a fresh link is printed for the page.
    fn session(&self, body: &[u8]) -> Response {
        let value = match http::parse_json_body(body) {
            Ok(value) => value,
            Err(status) => return Response::refusal(status, http::reason(status)),
        };
        let Some(code) = value.get("code").and_then(Value::as_str) else {
            return Response::refusal(400, http::reason(400));
        };
        let mut random = [0_u8; 32];
        if self.random.fill(&mut random).is_err() {
            return Response::refusal(503, "busy, retry");
        }
        let now = self.clock.now();
        let redeemed = self
            .auth
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .redeem(code, random, now);
        match redeemed {
            Ok(secret) => Response::json(200, &Value::obj([("session", Value::str(secret))])),
            Err(auth::Refused::Malformed) => Response::refusal(400, http::reason(400)),
            Err(auth::Refused::Unknown) => {
                self.fresh_link();
                Response::refusal(401, NO_SESSION)
            }
        }
    }

    /// `GET /api/state`: the shared snapshot, or 304 when the page has it.
    fn state(&self, head: &Head) -> Response {
        let snapshot = self
            .cache
            .get(&*self.clock, &*self.source, &self.version, &self.key);
        let unchanged = head.header("if-none-match") == Some(snapshot.etag.as_str());
        Response {
            status: if unchanged { 304 } else { 200 },
            content_type: http::JSON,
            body: Arc::clone(&snapshot.body),
            etag: Some(snapshot.etag.clone()),
        }
    }

    /// `GET /api/topic/<slug>`: the card of `slug` from the same shared
    /// snapshot, with its own entity tag, or 404 for no such topic.
    fn topic(&self, head: &Head, slug: &str) -> Response {
        let snapshot = self
            .cache
            .get(&*self.clock, &*self.source, &self.version, &self.key);
        let Some((body, etag)) = state::topic(&snapshot.read, slug) else {
            return Response::refusal(404, "no such topic");
        };
        let unchanged = head.header("if-none-match") == Some(etag.as_str());
        Response {
            status: if unchanged { 304 } else { 200 },
            content_type: http::JSON,
            body,
            etag: Some(etag),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::net::TcpStream;
    use std::sync::Arc;
    use std::time::Duration;

    use super::{Options, Probe, Seen, start};
    use crate::web::clock::ManualClock;
    use crate::web::state::{Read, Source};

    struct Empty;

    impl Source for Empty {
        fn read(&self) -> Read {
            Read::default()
        }
    }

    /// Moves the clock past the request deadline as soon as the first read
    /// lands, before the reader looks at what it read.
    struct Late(Arc<ManualClock>);

    impl Probe for Late {
        fn saw(&self, event: Seen) {
            if matches!(event, Seen::FirstByte { .. }) {
                self.0.advance(Duration::from_secs(10));
            }
        }
    }

    #[test]
    fn a_request_whose_last_read_lands_after_the_deadline_is_cut() {
        let clock = Arc::new(ManualClock::new(1_795_000_000));
        let running = start(Options {
            port: 0,
            clock: clock.clone(),
            source: Arc::new(Empty),
            probe: Arc::new(Late(clock)),
            links: Arc::new(|_| {}),
            version: "deadline-unit".to_owned(),
        })
        .unwrap_or_else(|error| panic!("start: {error}"));
        let host = running.addr();
        let mut client = TcpStream::connect(host).unwrap_or_else(|error| panic!("{error}"));
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap_or_else(|error| panic!("{error}"));
        client
            .write_all(format!("GET / HTTP/1.1\r\nHost: {host}\r\n\r\n").as_bytes())
            .unwrap_or_else(|error| panic!("{error}"));
        let mut reply = Vec::new();
        let _ = client.read_to_end(&mut reply);
        running.shutdown();
        assert!(
            reply.starts_with(b"HTTP/1.1 408 "),
            "{}",
            String::from_utf8_lossy(reply.get(..40).unwrap_or_default())
        );
    }

    /// driver-plan.md: a response leaves in writes of at most 16 KiB. A contract pin
    /// of the planned chunk size; the bytes a reader gets do not show it.
    #[test]
    fn a_response_write_hands_the_socket_at_most_16_kib() {
        assert_eq!(super::CHUNK, 16_384);
    }
}
