//! `POST /api/send`: one owner ask per draft id, carried by one send worker
//! through the console's own owner admission and the helpers' tracked path.
//!
//! A read worker never waits for a carry. It answers a draft id from its
//! durable claim when one exists, from the queue when the id is waiting, and
//! otherwise checks the nonce and the displayed route and queues the draft:
//! one carrying and [`QUEUE`] waiting, the next answered 503. The send worker
//! claims the draft durably BEFORE any input, carries it, and settles the
//! claim with what became of it; a claim without an outcome is never carried
//! again, so a lost listener answers its retry "unknown", never a second ask.

use std::cell::Cell;
use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use super::auth;
use super::claim::{self, Claim, Claims, Lookup, Word};
use super::clock::Clock;
use super::http::{self, Response};
use super::nonce;
use super::state::{self, RouteFacts};
use crate::console::submit::{self, Admission};
use crate::console::target::RoleRoute;
use crate::json::Value;
use crate::time::Timestamp;
use crate::tracked::{self, Kind, Receipt};

/// The drafts that may wait while one is carried.
pub const QUEUE: usize = 2;
/// The answer to a draft that may have landed and is never sent again.
pub const UNKNOWN: &str = "unknown: may have landed, not resent";
/// The answer to a nonce that is forged, expired or pruned.
pub const EXPIRED: &str = "expired draft id: send it as a new message";
/// The answer to a known draft id sent with other text or another route.
pub const MISMATCH: &str = "this draft id was sent with other text or route";
/// The answer to a route that is no longer the orchestrator's.
pub const CHANGED: &str = "the orchestrator changed, reload";
/// The answer while the queue is full or admission is closed.
pub const BUSY: &str = "busy, retry";
/// The words of a pasted ask whose submit was not confirmed.
const UNCERTAIN: &str = "uncertain: pasted, its submit not confirmed; not resent";
/// The most bytes of refusal words a claim keeps.
const WORDS_MAX: usize = 1024;
/// The longest wait of the send worker between two looks at the stop flag.
const POLL: Duration = Duration::from_secs(1);
/// The listener seconds between two prunes of the claim store.
const PRUNE_EVERY: u64 = 60 * 60;
/// The drafts whose claim could not be written that wait to be told, the
/// oldest dropped first.
const FAILED_KEPT: usize = 8;

/// Where the send worker stops so a test can cut it there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Point {
    /// The claim is durable; nothing is carried yet.
    Claimed {
        /// The request id the claim names.
        id: String,
    },
    /// Admission proved the route and the launch expectation is captured; the
    /// tracked delivery is next.
    BeforeInput {
        /// The request id.
        id: String,
    },
    /// The input is done and no opening is recorded yet.
    Carried {
        /// The request id.
        id: String,
    },
    /// The opening is recorded and the claim has no outcome yet.
    Opened {
        /// The request id.
        id: String,
    },
}

/// Whether the send worker goes on past a [`Point`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// Go on.
    Go,
    /// End the job here and write nothing more, as a lost listener would.
    Halt,
}

/// What the send worker asks at each [`Point`], and how a listener replaces
/// itself with a new core.
pub trait Hooks: Send + Sync {
    /// The worker reached `point`.
    fn at(&self, point: Point) -> Flow {
        let _ = point;
        Flow::Go
    }

    /// Replace this process with `argv`; returns only when that failed.
    fn exec(&self, argv: &[String]) -> io::Error {
        let _ = argv;
        io::Error::other("no exec in this listener")
    }

    /// How long a carry may wait for a busy seat before it defers.
    fn defer(&self) -> Duration {
        crate::deliver::DEFAULT_DEFER
    }
}

/// The production hooks: no cut, the real exec, and the deferral the command
/// line was started with.
struct Live {
    defer: Duration,
}

impl Hooks for Live {
    fn exec(&self, argv: &[String]) -> io::Error {
        crate::run::exec_unchanged(argv)
    }

    fn defer(&self) -> Duration {
        self.defer
    }
}

/// How a listener sends: the state root whose orchestrator it sends to and
/// whose claims it keeps, and its hooks.
pub struct Sends {
    /// The state root.
    pub root: PathBuf,
    /// The hooks.
    pub hooks: Arc<dyn Hooks>,
}

impl Sends {
    /// The production sends of state root `root`, each carry deferring after
    /// the default [`crate::deliver::DEFAULT_DEFER`] on a busy seat.
    #[must_use]
    pub fn live(root: PathBuf) -> Self {
        Self::live_with(root, crate::deliver::DEFAULT_DEFER)
    }

    /// [`Sends::live`] deferring after `defer`, the command line's own
    /// setting, as the helpers do.
    #[must_use]
    pub fn live_with(root: PathBuf, defer: Duration) -> Self {
        Self {
            root,
            hooks: Arc::new(Live { defer }),
        }
    }
}

/// The listener's own second: its wall second at start plus the monotonic
/// time since, so a wall clock moved backward never makes a nonce younger.
#[derive(Clone)]
pub(crate) struct Epoch {
    clock: Arc<dyn Clock>,
    start: Instant,
    at_start: u64,
}

impl Epoch {
    pub(crate) fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            start: clock.now(),
            at_start: u64::try_from(clock.epoch()).unwrap_or(0),
            clock,
        }
    }

    pub(crate) fn now(&self) -> u64 {
        let since = self.clock.now().saturating_duration_since(self.start);
        self.at_start.saturating_add(since.as_secs())
    }
}

/// One queued draft.
pub(crate) struct Job {
    name: String,
    payload: String,
    route: String,
    text: String,
    seat: String,
    to: String,
}

/// Admission and the drafts in flight: queued or carrying, by claim name,
/// each with its payload hash and route.
struct Gate {
    open: bool,
    pending: BTreeMap<String, (String, String)>,
    /// The drafts whose claim the worker could not write, oldest first, at
    /// most [`FAILED_KEPT`]: each is told to its next retry once, and its
    /// name is fresh again after that.
    failed: VecDeque<Failed>,
}

/// A draft whose claim could not be written, so nothing was carried.
struct Failed {
    name: String,
    payload: String,
    route: String,
    words: String,
}

/// The send half of one listener, shared by its read workers, its send
/// worker and its handle.
pub(crate) struct Sending {
    root: PathBuf,
    claims: Claims,
    hooks: Arc<dyn Hooks>,
    key: [u8; 32],
    epoch: Epoch,
    gate: Mutex<Gate>,
    idle: Condvar,
    jobs: SyncSender<Job>,
}

/// A draft as a page posts it.
struct Draft {
    text: String,
    route: String,
    nonce: String,
}

impl Draft {
    /// Exactly `text` (non-empty), `route` (64 lowercase hex) and `nonce`,
    /// each a string; anything else is no draft.
    fn parse(body: &[u8]) -> Option<Self> {
        let Ok(Value::Obj(fields)) = http::parse_json_body(body) else {
            return None;
        };
        let mut found: [Option<String>; 3] = [None, None, None];
        for (key, value) in fields {
            let at = ["text", "route", "nonce"]
                .iter()
                .position(|name| *name == key)?;
            let Value::Str(text) = value else { return None };
            let slot = found.get_mut(at)?;
            if slot.replace(text).is_some() {
                return None;
            }
        }
        let [Some(text), Some(route), Some(nonce)] = found else {
            return None;
        };
        (!text.is_empty() && auth::is_hex(&route, 64)).then_some(Self { text, route, nonce })
    }
}

impl Sending {
    /// The send half of a listener keyed `key` at `epoch`, and the receiving
    /// end its send worker takes.
    pub(crate) fn new(sends: Sends, key: [u8; 32], epoch: Epoch) -> (Arc<Self>, Receiver<Job>) {
        let (jobs, taken) = std::sync::mpsc::sync_channel(QUEUE);
        let sending = Arc::new(Self {
            claims: Claims::open(&sends.root),
            root: sends.root,
            hooks: sends.hooks,
            key,
            epoch,
            gate: Mutex::new(Gate {
                open: true,
                pending: BTreeMap::new(),
                failed: VecDeque::new(),
            }),
            idle: Condvar::new(),
            jobs,
        });
        (sending, taken)
    }

    fn gate(&self) -> MutexGuard<'_, Gate> {
        self.gate.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `GET /api/nonce`: a fresh draft id, or 503 when no random bytes came.
    pub(crate) fn nonce(&self, random: Option<[u8; 16]>) -> Response {
        let Some(random) = random else {
            return Response::refusal(503, BUSY);
        };
        let minted = nonce::mint(&self.key, self.epoch.now(), random);
        Response::json(200, &Value::obj([("nonce", Value::str(minted))]))
    }

    /// `POST /api/send`: answer a known draft id from its claim or the queue,
    /// else check its nonce and route and queue it.
    pub(crate) fn post(&self, body: &[u8]) -> Response {
        let Some(draft) = Draft::parse(body) else {
            return Response::refusal(400, http::reason(400));
        };
        if !auth::is_hex(&draft.nonce, nonce::LEN) {
            return Response::refusal(400, http::reason(400));
        }
        let name = nonce::claim_name(&draft.nonce);
        let payload = crate::install::sha256_hex(draft.text.as_bytes());
        if let Some(answer) = self.known(&name, &payload, &draft.route) {
            return answer;
        }
        if let Some(answer) = waiting(&self.gate(), &name, &payload, &draft.route) {
            return answer;
        }
        if nonce::verify(&self.key, &draft.nonce, self.epoch.now()).is_err() {
            return Response::refusal(410, EXPIRED);
        }
        let Some(facts) = state::current_route(&self.root)
            .filter(|facts| state::route_id(&self.key, facts) == draft.route)
        else {
            return Response::refusal(409, CHANGED);
        };
        let mut gate = self.gate();
        if let Some(answer) = waiting(&gate, &name, &payload, &draft.route) {
            return answer;
        }
        // Under the queue lock: the worker drops a name only after its claim
        // settled, so a name neither pending nor claimed now is truly fresh. A
        // claim on disk wins over any failure kept for its name.
        if !matches!(self.claims.lookup(&name), Lookup::Absent) {
            drop(gate);
            return self
                .known(&name, &payload, &draft.route)
                .unwrap_or_else(|| unknown(None));
        }
        if let Some(answer) = told(&mut gate, &name, &payload, &draft.route) {
            return answer;
        }
        if !gate.open {
            return Response::refusal(503, BUSY);
        }
        gate.pending
            .insert(name.clone(), (payload.clone(), draft.route.clone()));
        let job = Job {
            name: name.clone(),
            payload,
            route: draft.route,
            text: draft.text,
            seat: facts.seat,
            to: facts.to,
        };
        match self.jobs.try_send(job) {
            Ok(()) => queued(),
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                gate.pending.remove(&name);
                Response::refusal(503, BUSY)
            }
        }
    }

    /// The answer a claim already gives `name`: `None` when there is none.
    /// An unsettled claim whose name is no longer pending is read once more:
    /// the worker drops the name only after it settled, so a settlement that
    /// raced the first read is answered with its receipt, never "unknown".
    fn known(&self, name: &str, payload: &str, route: &str) -> Option<Response> {
        let claim = match self.claims.lookup(name) {
            Lookup::Absent => return None,
            Lookup::Unreadable => return Some(unknown(None)),
            Lookup::Known(claim) => claim,
        };
        if claim.payload != payload || claim.route != route {
            return Some(Response::refusal(409, MISMATCH));
        }
        if claim.outcome.is_some() {
            return Some(answer(&claim));
        }
        if self.gate().pending.contains_key(name) {
            return Some(queued());
        }
        Some(match self.claims.lookup(name) {
            Lookup::Known(again) if again.outcome.is_some() && again.id == claim.id => {
                answer(&again)
            }
            _ => unknown(Some(&claim.id)),
        })
    }

    /// Close admission, then wait up to `wait` for nothing to be queued or
    /// carrying: whether that came. Admission stays closed either way.
    pub(crate) fn drain(&self, wait: Duration) -> bool {
        let mut gate = self.gate();
        gate.open = false;
        let (gate, _) = self
            .idle
            .wait_timeout_while(gate, wait, |gate| !gate.pending.is_empty())
            .unwrap_or_else(PoisonError::into_inner);
        gate.pending.is_empty()
    }

    /// Admit sends again.
    pub(crate) fn reopen(&self) {
        self.gate().open = true;
    }

    /// Replace this process with `argv` through the hooks.
    pub(crate) fn exec(&self, argv: &[String]) -> io::Error {
        self.hooks.exec(argv)
    }

    /// The send worker: one job at a time until `stop`, the claim store
    /// pruned at start and once per [`PRUNE_EVERY`].
    pub(crate) fn work(&self, taken: &Receiver<Job>, stop: &AtomicBool) {
        let mut pruned: Option<u64> = None;
        loop {
            let now = self.epoch.now();
            if pruned.is_none_or(|at| now.saturating_sub(at) >= PRUNE_EVERY) {
                self.claims.prune(now);
                pruned = Some(now);
            }
            match taken.recv_timeout(POLL) {
                Ok(job) => {
                    let name = job.name.clone();
                    self.carry(job);
                    self.gate().pending.remove(&name);
                    self.idle.notify_all();
                }
                Err(RecvTimeoutError::Timeout) if !stop.load(Ordering::SeqCst) => {}
                Err(_) => return,
            }
        }
    }

    /// Claim `job` durably, carry it, settle the claim; a halt at any point
    /// ends the job with nothing more written.
    fn carry(&self, job: Job) {
        let (now, entropy) = (Timestamp::now(), entropy());
        let id = tracked::request_id(Kind::Ask.id_prefix(), now, entropy);
        let mut claim = Claim {
            payload: job.payload.clone(),
            route: job.route.clone(),
            id: id.clone(),
            created: self.epoch.now(),
            outcome: None,
        };
        // No durable claim, no carry: the next retry is told why, once, and
        // after that it is checked and queued afresh, nonce and route included.
        if let Err(why) = self.claims.create(&job.name, &claim) {
            self.unrecorded(&job, &why);
            return;
        }
        if self.hooks.at(Point::Claimed { id: id.clone() }) == Flow::Halt {
            return;
        }
        let Some((word, words)) = self.deliver(&job, &id, (now, entropy)) else {
            return;
        };
        claim.outcome = Some(claim::Outcome {
            word,
            seat: job.seat,
            to: job.to,
            words,
        });
        // A settle that fails leaves the claim without an outcome: its retry
        // is answered "unknown", never carried.
        drop(self.claims.settle(&job.name, &claim));
    }

    /// Keep why `job`'s claim could not be written for its next retry.
    fn unrecorded(&self, job: &Job, why: &io::Error) {
        let failed = Failed {
            name: job.name.clone(),
            payload: job.payload.clone(),
            route: job.route.clone(),
            words: words_of(&format!("not sent: its claim could not be written: {why}")),
        };
        keep(&mut self.gate(), failed);
    }

    /// Carry `job` as ask `id` minted at `minted`: the route re-proved, the
    /// writer lease, the owner admission with the route proved again inside
    /// it, then the tracked delivery narrowed to the main seat's launch. What
    /// became of it, or `None` after a halt.
    fn deliver(&self, job: &Job, id: &str, minted: (Timestamp, u64)) -> Option<(Word, String)> {
        let proved = || {
            state::current_route(&self.root)
                .filter(|facts| state::route_id(&self.key, facts) == job.route)
        };
        let Some(facts) = proved() else {
            return Some((Word::Changed, String::new()));
        };
        let console =
            crate::console::Console::bound(facts.to.clone(), facts.dir.clone(), facts.uuid.clone());
        let _lease = match crate::console::term::lease(&console) {
            Ok(lease) => lease,
            Err(why) => return Some((Word::Refused, words_of(&why))),
        };
        let mut held = crate::events::Held::default();
        let changed = Cell::new(false);
        let mut admitted_route: Option<(RouteFacts, RoleRoute)> = None;
        // Under the admission lock: the displayed route proved afresh, then its
        // very pair bound as the app binds a confirmed redirect, whose pin the
        // delivery checks. One roster read serves both.
        let owns = || {
            let bound = state::current_route_pair(&self.root)
                .filter(|(now, _)| state::route_id(&self.key, now) == job.route)
                .and_then(|(now, pair)| {
                    let proof = (now.uuid.clone(), now.server.clone());
                    let role = RoleRoute::bind(&self.root, &now.to, &now.dir, proof, pair)?;
                    Some((now, role))
                });
            if bound.is_none() {
                changed.set(true);
                return Err(CHANGED.to_owned());
            }
            admitted_route = bound;
            Ok(())
        };
        let admitted = match submit::admit_external(&mut held, &facts.dir, &facts.to, id, owns) {
            Ok(Admission::Fresh(admitted)) => admitted,
            Ok(Admission::Recorded(outcome)) => return Some(settled(outcome, None, "")),
            Err(_) if changed.get() => return Some((Word::Changed, String::new())),
            Err(why) => return Some((Word::Refused, words_of(&why))),
        };
        let Some((facts, role)) = admitted_route else {
            return Some((Word::Changed, String::new()));
        };
        let launch = facts
            .seats
            .first()
            .map(|(_, _, launch)| launch.clone())
            .unwrap_or_default();
        if self.hooks.at(Point::BeforeInput { id: id.to_owned() }) == Flow::Halt {
            return None;
        }
        let halted = Cell::new(false);
        let mut kept: (String, Option<Receipt>) = (String::new(), None);
        let finished = admitted.finish_with_receipt(&mut held, id, || {
            kept = self.run(job, (&facts, &role), &launch, id, minted, &halted);
            kept.clone()
        });
        if halted.get() || self.hooks.at(Point::Opened { id: id.to_owned() }) == Flow::Halt {
            return None;
        }
        Some(settled(finished.outcome, kept.1, &kept.0))
    }

    /// The tracked delivery of `job` to the proved main seat: what it said
    /// and what it did to the input.
    fn run(
        &self,
        job: &Job,
        (facts, role): (&RouteFacts, &RoleRoute),
        launch: &str,
        id: &str,
        (now, entropy): (Timestamp, u64),
        halted: &Cell<bool>,
    ) -> (String, Option<Receipt>) {
        let sender = tracked::Sender {
            display: tracked::CONSOLE_SINK.to_owned(),
            slot: String::new(),
            session: facts.to.clone(),
        };
        let tail = [role.seat().to_owned(), job.text.clone()];
        let mut receipt = Some(Receipt::NotDelivered);
        let mut said = Vec::new();
        let mut step = || {
            if self.hooks.at(Point::Carried { id: id.to_owned() }) == Flow::Halt {
                halted.set(true);
                return Err(io::Error::other("the listener stopped after the carry"));
            }
            Ok(())
        };
        let run = tracked::run_carried(
            Kind::Ask,
            &facts.dir,
            &tail,
            Some(&sender),
            &facts.to,
            now,
            entropy,
            self.hooks.defer(),
            tracked::Extra {
                relay: None,
                receipt: Some(&mut receipt),
                seat: None,
                role: Some(role.pin()),
            },
            tracked::Carried {
                expect: Ok(Some(("main", launch))),
                after_carry: Some(&mut step),
            },
            &mut io::sink(),
            &mut said,
        );
        if let Err(why) = run {
            said.extend(why.to_string().bytes());
        }
        (String::from_utf8_lossy(&said).into_owned(), receipt)
    }
}

/// The queue's answer to `name`: `None` when it is not waiting.
fn waiting(gate: &Gate, name: &str, payload: &str, route: &str) -> Option<Response> {
    let (was_payload, was_route) = gate.pending.get(name)?;
    Some(if was_payload == payload && was_route == route {
        queued()
    } else {
        Response::refusal(409, MISMATCH)
    })
}

/// The random part of a request id, as the other request minters take it.
fn entropy() -> u64 {
    use std::hash::{BuildHasher as _, RandomState};
    RandomState::new().hash_one((std::process::id(), Timestamp::now().epoch()))
}

/// Keep `failed` in place of any earlier failure of its name, dropping the
/// oldest past [`FAILED_KEPT`].
fn keep(gate: &mut Gate, failed: Failed) {
    gate.failed.retain(|kept| kept.name != failed.name);
    if gate.failed.len() >= FAILED_KEPT {
        gate.failed.pop_front();
    }
    gate.failed.push_back(failed);
}

/// The failure kept for `name`, told once: a definite refusal, since nothing
/// was carried without a claim. Other text or another route is 409.
fn told(gate: &mut Gate, name: &str, payload: &str, route: &str) -> Option<Response> {
    let at = gate.failed.iter().position(|failed| failed.name == name)?;
    let failed = gate.failed.get(at)?;
    if failed.payload != payload || failed.route != route {
        return Some(Response::refusal(409, MISMATCH));
    }
    let failed = gate.failed.remove(at)?;
    Some(Response::json(
        200,
        &Value::obj([
            ("id", Value::Null),
            ("seat", Value::Null),
            ("to", Value::Null),
            ("outcome", Value::str(Word::Refused.word())),
            ("words", Value::str(failed.words)),
        ]),
    ))
}

fn queued() -> Response {
    Response::json(202, &Value::obj([("state", Value::str("queued"))]))
}

/// The answer to a claim with no outcome, or to a file that is no claim.
fn unknown(id: Option<&str>) -> Response {
    Response::json(
        200,
        &Value::obj([
            ("id", id.map_or(Value::Null, Value::str)),
            ("seat", Value::Null),
            ("to", Value::Null),
            ("outcome", Value::str(Word::Unknown.word())),
            ("words", Value::str(UNKNOWN)),
        ]),
    )
}

/// The answer a settled claim gives, the same on every retry and restart.
fn answer(claim: &Claim) -> Response {
    let Some(outcome) = &claim.outcome else {
        return unknown(Some(&claim.id));
    };
    if outcome.word == Word::Changed {
        return Response::refusal(409, CHANGED);
    }
    Response::json(
        200,
        &Value::obj([
            ("id", Value::str(claim.id.as_str())),
            ("seat", Value::str(outcome.seat.as_str())),
            ("to", Value::str(outcome.to.as_str())),
            ("outcome", Value::str(outcome.word.word())),
            ("words", Value::str(outcome.words.as_str())),
        ]),
    )
}

/// A console outcome as a claim's: a confirmed send, a refusal with what the
/// delivery said, or unknown when input may have reached the seat.
fn settled(outcome: submit::Outcome, receipt: Option<Receipt>, said: &str) -> (Word, String) {
    match outcome {
        submit::Outcome::Sent(..) => (Word::Sent, String::new()),
        submit::Outcome::Uncertain(_) => (Word::Unknown, UNCERTAIN.to_owned()),
        // As the app's redirect keeps it: a typed receipt that input may
        // have reached the seat outranks the journal's abandonment.
        submit::Outcome::NotDelivered(_) if receipt == Some(Receipt::Uncertain) => {
            (Word::Unknown, UNCERTAIN.to_owned())
        }
        submit::Outcome::NotDelivered(_) => (Word::Refused, words_of(said)),
        submit::Outcome::Unknown(_, why) => match receipt {
            None | Some(Receipt::NotDelivered) => {
                let why = if said.trim().is_empty() {
                    why.as_str()
                } else {
                    said
                };
                (Word::Refused, words_of(why))
            }
            Some(Receipt::Sent | Receipt::Uncertain) => (Word::Unknown, UNKNOWN.to_owned()),
        },
    }
}

/// `text` as one claim line: every control character a space, runs of
/// spaces one, trimmed, at most [`WORDS_MAX`] bytes; never empty.
fn words_of(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(WORDS_MAX));
    for word in text.split(|c: char| c.is_control() || c == ' ') {
        if word.is_empty() {
            continue;
        }
        if out.len() + word.len() + 1 > WORDS_MAX {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    if out.is_empty() {
        "nothing was delivered".to_owned()
    } else {
        out
    }
}

/// The send worker's thread body, for the server to spawn.
pub(crate) fn worker(
    sending: Arc<Sending>,
    taken: Receiver<Job>,
    stop: Arc<AtomicBool>,
) -> impl FnOnce() + Send + 'static {
    move || sending.work(&taken, &stop)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, VecDeque};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Receiver, Sender};
    use std::time::Duration;

    use super::super::claim::KEEP;
    use super::super::clock::ManualClock;
    use super::{
        Claim, Draft, Epoch, FAILED_KEPT, Failed, Flow, Gate, Hooks, Job, Lookup, POLL, Point,
        Sending, Sends, keep, told, words_of, worker,
    };

    #[test]
    fn a_draft_is_exactly_three_strings_with_text_and_a_route() {
        let route = "a".repeat(64);
        let good = format!(r#"{{"text":"hi","route":"{route}","nonce":"n"}}"#);
        assert!(Draft::parse(good.as_bytes()).is_some());
        for bad in [
            format!(r#"{{"text":"","route":"{route}","nonce":"n"}}"#),
            format!(
                r#"{{"text":"hi","route":"{}","nonce":"n"}}"#,
                "A".repeat(64)
            ),
            format!(r#"{{"text":"hi","route":"{route}"}}"#),
            format!(r#"{{"text":"hi","route":"{route}","nonce":"n","more":"x"}}"#),
            format!(r#"{{"text":"hi","text":"hi","route":"{route}","nonce":"n"}}"#),
            format!(r#"{{"text":1,"route":"{route}","nonce":"n"}}"#),
        ] {
            assert!(Draft::parse(bad.as_bytes()).is_none(), "{bad}");
        }
    }

    #[test]
    fn refusal_words_become_one_bounded_line() {
        assert_eq!(words_of("  a\tb\n\nc  "), "a b c");
        assert_eq!(words_of("\n"), "nothing was delivered");
        assert!(words_of(&"x ".repeat(4000)).len() <= super::WORDS_MAX);
    }

    #[test]
    fn words_fill_the_cap_exactly_and_never_split_a_word() {
        let full = format!("{} {}", "a".repeat(511), "b".repeat(512));
        assert_eq!(full.len(), super::WORDS_MAX);
        assert_eq!(words_of(&full), full, "a line of exactly the cap is whole");
        let over = format!("{} {}", "a".repeat(512), "b".repeat(512));
        assert_eq!(
            words_of(&over),
            "a".repeat(512),
            "the word that would pass the cap is dropped whole"
        );
    }

    struct NoCut;

    impl Hooks for NoCut {}

    /// A scratch directory this test created itself, so its cleanup removes
    /// nothing it did not make.
    fn scratch(tag: &str) -> std::path::PathBuf {
        let base = std::env::temp_dir();
        for at in 0..64 {
            let root = base.join(format!("ae-web-{tag}-{}-{at}", std::process::id()));
            if std::fs::create_dir(&root).is_ok() {
                return root;
            }
        }
        panic!("no unused scratch directory under {}", base.display());
    }

    /// The worker keeps why it could not write a claim. `post` tells that
    /// refusal to a retry only after the store answers no claim for the name
    /// and the nonce and the route still pass; this test reaches the store
    /// step and the gate directly, not the nonce and route checks.
    #[test]
    fn a_claim_the_worker_cannot_write_leaves_its_refusal_for_a_retry_past_the_store() {
        let root = scratch("unrecorded");
        // `web` is a file, so the claim directory cannot be made.
        std::fs::write(root.join("web"), b"").unwrap_or_else(|error| panic!("web: {error}"));
        let sends = Sends {
            root: root.clone(),
            hooks: Arc::new(NoCut),
        };
        let epoch = Epoch::new(Arc::new(ManualClock::new(1_700_000_000)));
        let (sending, _taken) = Sending::new(sends, [7; 32], epoch);
        let (name, payload, route) = ("c".repeat(64), "a".repeat(64), "b".repeat(64));
        sending.carry(Job {
            name: name.clone(),
            payload: payload.clone(),
            route: route.clone(),
            text: "hi".to_owned(),
            seat: "tui".to_owned(),
            to: "sample".to_owned(),
        });
        // The store is readable again and holds no claim under the name.
        std::fs::remove_file(root.join("web")).unwrap_or_else(|error| panic!("web: {error}"));
        let stored = sending
            .known(&name, &payload, &route)
            .map(|answer| answer.status);
        let answer = told(&mut sending.gate(), &name, &payload, &route).map(|answer| {
            (
                answer.status,
                String::from_utf8_lossy(&answer.body).into_owned(),
            )
        });
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(stored, None, "no claim was written");
        let (status, body) = answer.unwrap_or_else(|| panic!("the next retry is told nothing"));
        assert_eq!(status, 200);
        assert!(
            body.starts_with(
                r#"{"id":null,"seat":null,"to":null,"outcome":"refused","words":"not sent: its claim could not be written: "#
            ),
            "{body}"
        );
    }

    fn failed(name: &str) -> Failed {
        Failed {
            name: name.to_owned(),
            payload: "p".to_owned(),
            route: "r".to_owned(),
            words: format!("not sent: {name}"),
        }
    }

    #[test]
    fn an_unwritten_claim_is_told_once_and_the_kept_failures_are_bounded() {
        let mut gate = Gate {
            open: true,
            pending: BTreeMap::new(),
            failed: VecDeque::new(),
        };
        for at in 0..=FAILED_KEPT {
            keep(&mut gate, failed(&at.to_string()));
        }
        keep(&mut gate, failed("3"));
        assert_eq!(gate.failed.len(), FAILED_KEPT, "the oldest is dropped");
        assert!(
            told(&mut gate, "0", "p", "r").is_none(),
            "the oldest is gone"
        );
        let mismatch = told(&mut gate, "3", "other", "r").map(|answer| answer.status);
        assert_eq!(mismatch, Some(409), "other text is a mismatch");
        let answer = told(&mut gate, "3", "p", "r").map(|answer| answer.body);
        let body = answer.as_deref().map(String::from_utf8_lossy);
        assert_eq!(
            body.as_deref(),
            Some(r#"{"id":null,"seat":null,"to":null,"outcome":"refused","words":"not sent: 3"}"#),
            "a definite refusal with the kept words"
        );
        assert!(told(&mut gate, "3", "p", "r").is_none(), "told only once");
        assert_eq!(gate.failed.len(), FAILED_KEPT - 1);
    }

    /// Hooks that end every job once its claim is durable and say so: the
    /// worker has then passed the prune check that comes before that job.
    struct Beat(Sender<()>);

    impl Hooks for Beat {
        fn at(&self, point: Point) -> Flow {
            if matches!(point, Point::Claimed { .. }) {
                let _ = self.0.send(());
            }
            Flow::Halt
        }
    }

    /// Hand the worker job `n` and wait until its claim is durable.
    fn pulse(sending: &Sending, beat: &Receiver<()>, n: u8) -> bool {
        let job = Job {
            name: format!("{n:064x}"),
            payload: "a".repeat(64),
            route: "b".repeat(64),
            text: "hi".to_owned(),
            seat: "tui".to_owned(),
            to: "sample".to_owned(),
        };
        sending.jobs.send(job).is_ok() && beat.recv_timeout(Duration::from_secs(30)).is_ok()
    }

    /// The worker prunes the claim store when it starts and then once an hour
    /// of listener time, never sooner: a claim that expires one second after
    /// the first prune is kept at 59:59 and gone at 60:00.
    #[test]
    fn the_worker_prunes_the_claim_store_once_an_hour_of_listener_time() {
        const START: u64 = 1_700_000_000;
        let root = scratch("prune");
        let (beats, beat) = std::sync::mpsc::channel();
        let clock = Arc::new(ManualClock::new(1_700_000_000));
        let sends = Sends {
            root: root.clone(),
            hooks: Arc::new(Beat(beats)),
        };
        let (sending, taken) = Sending::new(sends, [7; 32], Epoch::new(clock.clone()));
        // Its age at START is exactly KEEP, so only a later prune removes it.
        let expiring = "e".repeat(64);
        let made = sending.claims.create(
            &expiring,
            &Claim {
                payload: "a".repeat(64),
                route: "b".repeat(64),
                id: "ae-20261122T200000Z-0000000a".to_owned(),
                created: START - KEEP,
                outcome: None,
            },
        );
        let stop = Arc::new(AtomicBool::new(false));
        let thread = std::thread::spawn(worker(Arc::clone(&sending), taken, Arc::clone(&stop)));
        // Each advance is followed by two jobs: the check before the second
        // one reads the advanced clock.
        let started = pulse(&sending, &beat, 1);
        clock.advance(Duration::from_secs(3599));
        let early = pulse(&sending, &beat, 2) && pulse(&sending, &beat, 3);
        let at_hour_less_a_second = sending.claims.lookup(&expiring);
        clock.advance(Duration::from_secs(1));
        let late = pulse(&sending, &beat, 4) && pulse(&sending, &beat, 5);
        let at_hour = sending.claims.lookup(&expiring);
        stop.store(true, Ordering::SeqCst);
        let joined = thread.join().is_ok();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(made.map_err(|error| error.kind()), Ok(()));
        assert!(
            started && early && late && joined,
            "the worker took every job"
        );
        assert!(
            matches!(at_hour_less_a_second, Lookup::Known(_)),
            "no prune before the hour: {at_hour_less_a_second:?}"
        );
        assert_eq!(at_hour, Lookup::Absent, "the prune at the hour removes it");
    }

    /// The worker waits for jobs until it is stopped: a job handed to it after
    /// it idled through two polls is still carried.
    #[test]
    fn the_worker_takes_a_job_after_an_idle_poll() {
        let root = scratch("idle");
        let (beats, beat) = std::sync::mpsc::channel();
        let sends = Sends {
            root: root.clone(),
            hooks: Arc::new(Beat(beats)),
        };
        let clock = Arc::new(ManualClock::new(1_700_000_000));
        let (sending, taken) = Sending::new(sends, [7; 32], Epoch::new(clock));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = std::thread::spawn(worker(Arc::clone(&sending), taken, Arc::clone(&stop)));
        std::thread::sleep(POLL * 2 + Duration::from_millis(500));
        let taken_after_idle = pulse(&sending, &beat, 1);
        stop.store(true, Ordering::SeqCst);
        let joined = thread.join().is_ok();
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            taken_after_idle,
            "a job handed after an idle poll is carried"
        );
        assert!(joined, "the worker ends once it is stopped");
    }

    /// Hooks that do not say otherwise defer a busy seat after the helpers'
    /// default; the listener's test hooks rely on it. A contract pin.
    #[test]
    fn hooks_that_do_not_say_otherwise_defer_like_the_helpers() {
        assert_eq!(NoCut.defer(), crate::deliver::DEFAULT_DEFER);
    }

    /// The production sends defer a busy seat after the command line's own
    /// setting, and after the helpers' default without one.
    #[test]
    fn the_live_sends_defer_after_the_command_lines_setting() {
        let root = std::path::PathBuf::from("unused");
        let set = Duration::from_secs(7);
        assert_eq!(Sends::live_with(root.clone(), set).hooks.defer(), set);
        assert_eq!(
            Sends::live(root).hooks.defer(),
            crate::deliver::DEFAULT_DEFER
        );
    }
}
