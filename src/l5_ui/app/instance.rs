//! One `ae app` per state root.
//!
//! The app that runs holds [`store::lock`] on [`LOCK`], with no wait, until it
//! ends, and says who it is in the record [`INSTANCE`]: its pid, its start
//! identity (`LC_ALL=C ps -o lstart=`, letters and digits only), the tmux
//! server it was started under and the session, window and pane it sits in.
//! The lock is the proof of life: the kernel lets go of it when the holder
//! dies, whatever is left in the record. A second `ae app` reads the record
//! and does one of four things ([`judge`]):
//!
//! - the lock is free and the record names a pid that is gone or one the start
//!   identity says was reused (or names nothing): it reclaims the record;
//! - the lock is held and the caller sits in tmux on the same ae-owned server
//!   as the record: it hands its client to the running app and exits 0;
//! - the lock is held and the caller is anywhere else: it asks the running app
//!   to hand over through [`REQUEST`] and waits [`TAKEOVER_WAIT`] for the
//!   acknowledgement [`ACK`]. The running app saves its view, acknowledges,
//!   quits and lets go of the lock last. The new app then starts and restores
//!   the view the old one left. A timeout refuses and names the pid; nothing
//!   is ever signalled;
//! - any doubt refuses with the reason and replaces nothing.
//!
//! All three files are hostile persisted state. Each has exactly one spelling
//! (what [`render_instance`], [`render_request`] and [`render_ack`] write), a
//! hard byte cap, and a parse that succeeds renders back to the very bytes it
//! read; the fuzz target `app_instance` holds that.
//!
//! `ae app --new` ([`NEW_FLAG`]) holds no lock and no record and never writes
//! the saved view. A pane that carries a non-empty `@ae_console` is a session's
//! chat window (`chat = app`): it neither holds nor consults the record, so
//! each such window keeps its own app, as before. The embedded orchestrator
//! panel is part of this process; the `ae orchestrator` seat holds no record.

use std::fs::File;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::TICK;
use crate::console::open;
use crate::inventory::ServerId;
use crate::meta::Selector;
use crate::store::{self, SourcePresence, SourceRead};
use crate::time::Timestamp;
use crate::{doors, procs, transport};

/// The lock a running app holds, directly under the state root.
pub const LOCK: &str = "app-instance.lock";
/// The record of the running app.
pub const INSTANCE: &str = "app-instance";
/// A second app's request that the running one hand over.
pub const REQUEST: &str = "app-takeover";
/// The running app's acknowledgement of a request, carrying its nonce.
pub const ACK: &str = "app-takeover-ack";
/// The flag that starts another app beside the running one.
pub const NEW_FLAG: &str = "--new";
/// The most bytes the record may hold: its line and its one line feed.
pub const INSTANCE_CAP: usize = 512;
/// The most bytes a request may hold.
pub const REQUEST_CAP: usize = 160;
/// The most bytes an acknowledgement may hold.
pub const ACK_CAP: usize = 40;
/// The length of a nonce: 32 lowercase hex digits.
pub const NONCE_LEN: usize = 32;
/// The longest start identity.
pub const IDENTITY_MAX: usize = 64;
/// The longest socket path a record spells.
pub const SOCKET_MAX: usize = 128;
/// The most digits in a window or pane number.
pub const PLACE_DIGITS: usize = 10;
/// How long a second app waits for the acknowledgement of its request.
pub const TAKEOVER_WAIT: Duration = Duration::from_secs(5);

/// The most characters of a session token and of a tmux server name.
const SESSION_MAX: usize = 128;
const NAME_MAX: usize = 64;
/// How long an acknowledged request waits for the old app to let go of the
/// lock. The old app finishes an ask it has in flight first, so this is long.
const HANDOVER_WAIT: Duration = Duration::from_secs(30);
/// How long a held lock waits for the holder's record to settle.
const SETTLE: Duration = Duration::from_secs(1);
/// How long an acknowledgement nobody waits for any more stays after the app
/// that waited for it started.
const SWEEP: Duration = Duration::from_secs(2);

/// The tmux server a record or a caller names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Server {
    /// Outside tmux.
    None,
    /// The server the transport selects with no selector of its own.
    Ambient,
    /// A server named by `-L`.
    Name(String),
    /// A server named by its socket path.
    Socket(String),
}

/// What the record says of the app that wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    pub pid: u32,
    /// The start identity of that pid: `ps -o lstart=` with only its letters
    /// and digits kept.
    pub identity: String,
    pub server: Server,
    /// The session the app's pane was in, empty outside tmux.
    pub session: String,
    /// `@N`, empty outside tmux.
    pub window: String,
    /// `%N`, empty outside tmux.
    pub pane: String,
}

/// What the record file holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    Absent,
    Damaged,
    Parsed(Instance),
}

/// A second app's request to the running one, which honours it only when it
/// names that very pid and start identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub target_pid: u32,
    pub target_identity: String,
    pub requester: u32,
    pub nonce: String,
}

/// The running app's answer: the nonce of the request it honoured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ack {
    pub nonce: String,
}

/// What trying the lock said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hold {
    /// Another process holds it.
    Held,
    /// Nobody did.
    Free,
    /// The lock could not be tried: a link or another node stood at its path,
    /// or the file could not be opened.
    Unreadable,
}

/// What the process table says of the recorded pid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    Gone,
    Same,
    /// The pid is alive with another start identity: the pid was reused.
    Reused,
    Unknown,
}

/// Where the second app was started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub in_tmux: bool,
    /// Whether its tmux server is one this instance of ae owns.
    pub ae_owned: bool,
    pub server: Server,
}

/// Why a start refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Doubt {
    /// The record or the lock cannot be read.
    Unreadable,
    /// The record names a live pid that holds no lock.
    LockNotHeld,
    /// The process table could not say whether the pid is the recorded one.
    Unknown,
    /// The lock is held, but not by the pid and start identity the record names.
    NotHolder,
    /// tmux did not say whether this pane is a session's chat window.
    Stamp,
}

/// What an exec restart is: it keeps its pid and its start identity, so a
/// record that names them is its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restart {
    /// The same instance (or nobody else holds the lock): it takes the lock
    /// again and keeps the record.
    Governed,
    /// Another app holds the record: this one runs with the `--new` effects.
    Fresh,
}

/// What a start does about the running app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Take the record: nothing alive holds it.
    Reclaim,
    /// Hand the caller's client to the running app's session and pane.
    Focus {
        session: String,
    },
    /// Ask the running app to hand over.
    TakeOver,
    Refuse(Doubt),
}

/// Where a waiting second app stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Waiting,
    Acked,
    TimedOut,
}

/// The operands of `ae app`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    /// The home session as typed.
    pub session: Option<String>,
    /// `--app-own`: an app start ae made itself.
    pub own: bool,
    /// The view an exec restart handed on behind `--app-view`.
    pub handed: Option<String>,
    /// `--new`.
    pub fresh: bool,
}

// ---------------------------------------------------------------------------
// The grammar.

/// A canonical decimal: no sign, no leading zero, at most [`PLACE_DIGITS`]
/// digits.
fn decimal(text: &str) -> Option<u64> {
    let lead = text.bytes().next()?;
    let canonical = lead.is_ascii_digit() && (lead != b'0' || text.len() == 1);
    (canonical && text.len() <= PLACE_DIGITS && text.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

/// A pid: a canonical decimal that fits a `u32`, and is not 0.
fn pid(text: &str) -> Option<u32> {
    u32::try_from(decimal(text)?).ok().filter(|pid| *pid > 0)
}

fn identity_ok(text: &str) -> bool {
    (1..=IDENTITY_MAX).contains(&text.len())
        && text.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn nonce_ok(text: &str) -> bool {
    text.len() == NONCE_LEN
        && text
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn name_ok(text: &str) -> bool {
    (1..=NAME_MAX).contains(&text.len())
        && !text.starts_with('-')
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn socket_ok(path: &str) -> bool {
    path.starts_with('/') && path.len() <= SOCKET_MAX && !path.chars().any(char::is_control)
}

fn session_ok(text: &str) -> bool {
    (1..=SESSION_MAX).contains(&text.len())
        && text != "-"
        && text.bytes().all(|byte| byte.is_ascii_graphic())
}

/// `@N` or `%N`, `lead` and then a canonical decimal.
fn place_ok(text: &str, lead: char) -> bool {
    text.strip_prefix(lead).and_then(decimal).is_some()
}

fn hex(text: &str) -> String {
    use std::fmt::Write as _;
    text.bytes().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn unhex(text: &str) -> Option<String> {
    let lowercase = |byte: u8| matches!(byte, b'0'..=b'9' | b'a'..=b'f');
    if text.is_empty() || !text.len().is_multiple_of(2) || !text.bytes().all(lowercase) {
        return None;
    }
    let digit = |byte: u8| char::from(byte).to_digit(16);
    let bytes: Option<Vec<u8>> = text
        .as_bytes()
        .chunks(2)
        .map(|pair| u8::try_from(digit(pair[0])? * 16 + digit(pair[1])?).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

fn server_token(server: &Server) -> Option<String> {
    match server {
        Server::None => Some("-".to_owned()),
        Server::Ambient => Some("ambient".to_owned()),
        Server::Name(name) => name_ok(name).then(|| format!("L:{name}")),
        Server::Socket(path) => socket_ok(path).then(|| format!("S:{}", hex(path))),
    }
}

fn parse_server(token: &str) -> Option<Server> {
    match token {
        "-" => Some(Server::None),
        "ambient" => Some(Server::Ambient),
        _ => {
            if let Some(name) = token.strip_prefix("L:") {
                return name_ok(name).then(|| Server::Name(name.to_owned()));
            }
            let path = unhex(token.strip_prefix("S:")?)?;
            socket_ok(&path).then_some(Server::Socket(path))
        }
    }
}

/// The record `bytes` spell, or `None` unless they are exactly what
/// [`render_instance`] writes.
#[must_use]
pub fn parse_instance(bytes: &[u8]) -> Option<Instance> {
    if bytes.len() > INSTANCE_CAP {
        return None;
    }
    let body = std::str::from_utf8(bytes).ok()?.strip_suffix('\n')?;
    let fields: Vec<&str> = body.split(' ').collect();
    let [version, pid_text, identity, server, session, window, pane] = fields.as_slice() else {
        return None;
    };
    if *version != "1" || !identity_ok(identity) {
        return None;
    }
    let server = parse_server(server)?;
    let place = if server == Server::None {
        if (*session, *window, *pane) != ("-", "-", "-") {
            return None;
        }
        (String::new(), String::new(), String::new())
    } else {
        if !(session_ok(session) && place_ok(window, '@') && place_ok(pane, '%')) {
            return None;
        }
        (
            (*session).to_owned(),
            (*window).to_owned(),
            (*pane).to_owned(),
        )
    };
    Some(Instance {
        pid: pid(pid_text)?,
        identity: (*identity).to_owned(),
        server,
        session: place.0,
        window: place.1,
        pane: place.2,
    })
}

/// The bytes of `held`, or `None` when the grammar does not spell it.
#[must_use]
pub fn render_instance(held: &Instance) -> Option<Vec<u8>> {
    if held.pid == 0 || !identity_ok(&held.identity) {
        return None;
    }
    let server = server_token(&held.server)?;
    let place = if held.server == Server::None {
        if !(held.session.is_empty() && held.window.is_empty() && held.pane.is_empty()) {
            return None;
        }
        ["-".to_owned(), "-".to_owned(), "-".to_owned()]
    } else {
        if !(session_ok(&held.session) && place_ok(&held.window, '@') && place_ok(&held.pane, '%'))
        {
            return None;
        }
        [held.session.clone(), held.window.clone(), held.pane.clone()]
    };
    let line = format!(
        "1 {} {} {server} {} {} {}\n",
        held.pid, held.identity, place[0], place[1], place[2]
    );
    (line.len() <= INSTANCE_CAP).then(|| line.into_bytes())
}

/// The request `bytes` spell, or `None` unless they are exactly what
/// [`render_request`] writes.
#[must_use]
pub fn parse_request(bytes: &[u8]) -> Option<Request> {
    if bytes.len() > REQUEST_CAP {
        return None;
    }
    let body = std::str::from_utf8(bytes).ok()?.strip_suffix('\n')?;
    let fields: Vec<&str> = body.split(' ').collect();
    let ["1", target, identity, requester, nonce] = fields.as_slice() else {
        return None;
    };
    (identity_ok(identity) && nonce_ok(nonce)).then_some(())?;
    Some(Request {
        target_pid: pid(target)?,
        target_identity: (*identity).to_owned(),
        requester: pid(requester)?,
        nonce: (*nonce).to_owned(),
    })
}

/// The bytes of `request`, or `None` when the grammar does not spell it.
#[must_use]
pub fn render_request(request: &Request) -> Option<Vec<u8>> {
    (request.target_pid > 0
        && request.requester > 0
        && identity_ok(&request.target_identity)
        && nonce_ok(&request.nonce))
    .then(|| {
        format!(
            "1 {} {} {} {}\n",
            request.target_pid, request.target_identity, request.requester, request.nonce
        )
        .into_bytes()
    })
}

/// The acknowledgement `bytes` spell, or `None` unless they are exactly what
/// [`render_ack`] writes.
#[must_use]
pub fn parse_ack(bytes: &[u8]) -> Option<Ack> {
    if bytes.len() > ACK_CAP {
        return None;
    }
    let body = std::str::from_utf8(bytes).ok()?.strip_suffix('\n')?;
    let fields: Vec<&str> = body.split(' ').collect();
    let ["1", nonce] = fields.as_slice() else {
        return None;
    };
    nonce_ok(nonce).then(|| Ack {
        nonce: (*nonce).to_owned(),
    })
}

/// The bytes of `ack`, or `None` when the grammar does not spell it.
#[must_use]
pub fn render_ack(ack: &Ack) -> Option<Vec<u8>> {
    nonce_ok(&ack.nonce).then(|| format!("1 {}\n", ack.nonce).into_bytes())
}

/// Whether `request` names the app with this pid and start identity.
#[must_use]
pub fn targets_me(request: &Request, pid: u32, identity: &str) -> bool {
    request.target_pid == pid && request.target_identity == identity
}

/// Where a request stands `elapsed` after it was written: a matching
/// acknowledgement ends the wait whenever it comes, and only then the deadline.
#[must_use]
pub fn phase(ack: Option<&Ack>, nonce: &str, elapsed: Duration) -> Phase {
    if ack.is_some_and(|ack| ack.nonce == nonce) {
        Phase::Acked
    } else if elapsed >= TAKEOVER_WAIT {
        Phase::TimedOut
    } else {
        Phase::Waiting
    }
}

/// What a start does, given what the lock, the record and the process table
/// say. The lock decides who is alive; the process table only checks that the
/// record names the holder.
#[must_use]
pub fn judge(record: &Record, hold: Hold, live: Liveness, caller: &Caller) -> Verdict {
    match (hold, record) {
        (Hold::Unreadable, _) | (Hold::Held, Record::Absent | Record::Damaged) => {
            Verdict::Refuse(Doubt::Unreadable)
        }
        (Hold::Held, Record::Parsed(holder)) => match live {
            Liveness::Same => engage(holder, caller),
            Liveness::Gone | Liveness::Reused => Verdict::Refuse(Doubt::NotHolder),
            Liveness::Unknown => Verdict::Refuse(Doubt::Unknown),
        },
        (Hold::Free, Record::Parsed(_)) => match live {
            Liveness::Gone | Liveness::Reused => Verdict::Reclaim,
            Liveness::Same => Verdict::Refuse(Doubt::LockNotHeld),
            Liveness::Unknown => Verdict::Refuse(Doubt::Unknown),
        },
        (Hold::Free, Record::Absent | Record::Damaged) => Verdict::Reclaim,
    }
}

/// Whether an exec restart is the instance it was or another app's guest:
/// [`Restart::Fresh`] only while the lock is held and the record does not name
/// this pid and start identity. A free lock is never another holder.
#[must_use]
pub fn restart(hold: Hold, record: &Record, pid: u32, identity: &str) -> Restart {
    let mine = matches!(record, Record::Parsed(own) if own.pid == pid && own.identity == identity);
    if hold == Hold::Held && !mine {
        Restart::Fresh
    } else {
        Restart::Governed
    }
}

/// Focus the running app when the caller sits in tmux on the very server the
/// record names and that server is ae's own; otherwise ask it to hand over.
fn engage(held: &Instance, caller: &Caller) -> Verdict {
    let same = caller.in_tmux
        && caller.ae_owned
        && matches!(held.server, Server::Name(_) | Server::Socket(_))
        && held.server == caller.server
        && !held.session.is_empty();
    if same {
        Verdict::Focus {
            session: held.session.clone(),
        }
    } else {
        Verdict::TakeOver
    }
}

/// The one line the app that was taken over leaves.
#[must_use]
pub fn took_over_line(requester: u32) -> String {
    format!("ae app: {requester} took over\n")
}

/// The one line a second app leaves after it focused the running one, naming
/// a home it was given and did not apply.
#[must_use]
pub fn focused_line(pane: &str, home: Option<&str>) -> String {
    let home = home
        .filter(|home| !home.is_empty())
        .map(|home| format!("; {home} was not applied"))
        .unwrap_or_default();
    format!("ae app is already open at {pane}; switched to it{home}\n")
}

/// The one line a second app leaves when the running one did not hand over.
#[must_use]
pub fn timeout_line(pid: u32) -> String {
    format!("ae app: {pid} did not hand over\n")
}

/// The one line a second app leaves when it could not switch its client to the
/// running one.
#[must_use]
pub fn focus_failed_line(pane: &str) -> String {
    format!("ae app is already open at {pane}; this client could not be switched to it\n")
}

/// The one line a second app leaves when the running one acknowledged and then
/// kept the lock.
#[must_use]
pub fn release_line(pid: u32) -> String {
    format!("ae app: {pid} handed over but still holds the app record\n")
}

/// The one line a refused start leaves.
#[must_use]
pub fn doubt_line(doubt: Doubt, pid: u32) -> String {
    match doubt {
        Doubt::Unreadable => "ae app: the app record cannot be read\n".to_owned(),
        Doubt::LockNotHeld => format!("ae app: {pid} is alive but holds no app record lock\n"),
        Doubt::Unknown => format!("ae app: cannot tell whether {pid} is the recorded app\n"),
        Doubt::NotHolder => format!("ae app: {pid} is not the app that holds the record\n"),
        Doubt::Stamp => "ae app: tmux did not say whether this pane is a chat window\n".to_owned(),
    }
}

/// `ae app`'s operands: the home session, then `--app-own` once, then the view
/// an exec restart handed on behind `--app-view`, last; or the home session
/// and [`NEW_FLAG`] in either order, alone. `None` is a usage error.
#[must_use]
pub fn parse_start(tail: &[String]) -> Option<Start> {
    if tail.iter().any(|word| word == NEW_FLAG) {
        let rest: Vec<&String> = tail.iter().filter(|word| *word != NEW_FLAG).collect();
        if rest.len() + 1 != tail.len() {
            return None;
        }
        let session = match rest.as_slice() {
            [] => None,
            [name] if !name.starts_with('-') => Some((*name).clone()),
            _ => return None,
        };
        return Some(Start {
            session,
            own: false,
            handed: None,
            fresh: true,
        });
    }
    let (session, rest) = match tail.split_first() {
        Some((name, rest)) if !name.starts_with('-') => (Some(name.clone()), rest),
        _ => (None, tail),
    };
    let (own, rest) = match rest.split_first() {
        Some((flag, more)) if flag == super::OWN => (true, more),
        _ => (false, rest),
    };
    let handed = match rest {
        [] => None,
        [flag, text] if flag == super::handover::FLAG => Some(text.clone()),
        _ => return None,
    };
    Some(Start {
        session,
        own,
        handed,
        fresh: false,
    })
}

// ---------------------------------------------------------------------------
// The start: the lock, the record, focus and take-over.

/// What the start decided for this process.
pub(super) enum Entry {
    /// The app holds the lock and the record: it is the instance.
    Hold(Guard),
    /// A session's chat window, or an app whose state root does not exist yet
    /// (no app can have held a record there): today's app, with no lock and
    /// no record.
    Exempt,
    /// `--new`, or an exec restart that found another app holding the record:
    /// an app that holds nothing and writes no saved view.
    Fresh,
    /// The start ended here, with this status and its line already written.
    Exit(u8),
    /// The caller has no terminal, and nothing here ended the start.
    NoTerminal,
}

/// What a start needs to know of its own operands.
pub(super) struct Asked<'a> {
    /// The home session as typed.
    pub(super) home: Option<&'a str>,
    /// The start is an exec restart handing its view on.
    pub(super) handed: bool,
    /// The caller has a terminal on stdin and stdout.
    pub(super) terminal: bool,
    /// The exit status of a start that refuses: the caller names it, so this
    /// layer names no code of the shell above it.
    pub(super) refused: u8,
}

/// The running app's hold on the instance: the lock, until it is dropped.
pub(super) struct Guard {
    _lock: File,
    root: PathBuf,
    pid: u32,
    identity: String,
    born: Instant,
    polled: Instant,
    swept: bool,
    honoured: Option<Request>,
}

impl Guard {
    /// Look for a request that names this app, at most once per tick. A
    /// request that names another pid or start identity is left where it is.
    pub(super) fn poll(&mut self, now: Instant) -> bool {
        if self.honoured.is_some() {
            return true;
        }
        if now.saturating_duration_since(self.polled) < TICK {
            return false;
        }
        self.polled = now;
        if !self.swept && self.born.elapsed() >= SWEEP {
            self.swept = true;
            remove(&self.root.join(ACK));
        }
        let wanted = read_request(&self.root)
            .filter(|request| targets_me(request, self.pid, &self.identity));
        self.honoured = wanted;
        self.honoured.is_some()
    }

    /// The pid of the app that asked, once a request was honoured.
    pub(super) fn requester(&self) -> Option<u32> {
        self.honoured.as_ref().map(|request| request.requester)
    }

    /// Acknowledge the honoured request and clear it: the view is saved.
    /// The lock stays held until this guard is dropped.
    pub(super) fn acknowledge(&self) {
        let Some(request) = &self.honoured else {
            return;
        };
        let ack = Ack {
            nonce: request.nonce.clone(),
        };
        if let Some(bytes) = render_ack(&ack) {
            let _ = publish(&self.root.join(ACK), &bytes);
        }
        remove(&self.root.join(REQUEST));
    }
}

fn remove(path: &Path) {
    let _ = std::fs::remove_file(path);
}

fn publish(path: &Path, bytes: &[u8]) -> Result<(), String> {
    match store::source_presence(path) {
        SourcePresence::Absent | SourcePresence::Regular => {}
        SourcePresence::Irregular(what) => {
            return Err(format!("{} is {what}; it is not replaced", path.display()));
        }
    }
    store::publish_private(path, bytes).map_err(|why| why.to_string())
}

fn read_request(root: &Path) -> Option<Request> {
    match store::read_nofollow_capped(&root.join(REQUEST), REQUEST_CAP as u64) {
        Ok(SourceRead::Ready(bytes)) => parse_request(&bytes),
        _ => None,
    }
}

fn read_ack(root: &Path) -> Option<Ack> {
    match store::read_nofollow_capped(&root.join(ACK), ACK_CAP as u64) {
        Ok(SourceRead::Ready(bytes)) => parse_ack(&bytes),
        _ => None,
    }
}

fn read_record(root: &Path) -> Record {
    match store::read_nofollow_capped(&root.join(INSTANCE), INSTANCE_CAP as u64) {
        Ok(SourceRead::Absent) => Record::Absent,
        Ok(SourceRead::Ready(bytes)) => {
            parse_instance(&bytes).map_or(Record::Damaged, Record::Parsed)
        }
        _ => Record::Damaged,
    }
}

/// The start identity `ps -o lstart=` printed: its letters and digits.
fn identity_text(printed: &str) -> Option<String> {
    let identity: String = printed
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    identity_ok(&identity).then_some(identity)
}

/// The start identity of `pid`: `None` when `ps` could not say.
fn identity_of(pid: u32) -> Option<String> {
    match transport::run_ps_observed(&procs::PsArgv::lstart_of(pid)) {
        Some((true, printed)) => identity_text(&printed),
        _ => None,
    }
}

/// What the process table says of the recorded pid: a `ps` that did not run is
/// doubt ([`Liveness::Unknown`]), never a gone process.
fn liveness(held: &Instance) -> Liveness {
    match transport::run_ps_observed(&procs::PsArgv::lstart_of(held.pid)) {
        None => Liveness::Unknown,
        Some((false, _)) => Liveness::Gone,
        Some((true, printed)) => match identity_text(&printed) {
            None => Liveness::Unknown,
            Some(identity) if identity == held.identity => Liveness::Same,
            Some(_) => Liveness::Reused,
        },
    }
}

/// A server as a record spells it, `Server::None` when it cannot be spelled.
fn server_of(server: &ServerId) -> Server {
    let spelled = match server {
        ServerId::Ambient => Server::Ambient,
        ServerId::Selected(Selector::Name(name)) => Server::Name(name.clone()),
        ServerId::Selected(Selector::Socket(path)) => {
            Server::Socket(path.to_str().unwrap_or_default().to_owned())
        }
    };
    if server_token(&spelled).is_some() {
        spelled
    } else {
        Server::None
    }
}

/// Whether `server` is the tmux server this instance of ae launches on.
fn owned(server: &ServerId) -> bool {
    let declared = doors::declared_server(crate::shape::current());
    doors::launch_target(declared.as_ref())
        .is_some_and(|own| crate::fleet_read::SocketPaths::live().proven_same(server, &own))
}

/// The session, window and pane of `pane` as a record spells them.
fn place_of(server: &ServerId, pane: &str) -> Option<(String, String, String)> {
    let (session, _) = transport::observe_pane_place(server, pane)?;
    let panes = transport::observe_window_panes(server, &session)?;
    let window = panes
        .iter()
        .find(|one| one.pane_id == pane)?
        .window_id
        .clone();
    (session_ok(&session) && place_ok(&window, '@') && place_ok(pane, '%'))
        .then(|| (session, window, pane.to_owned()))
}

/// What this process knows of where it was started.
struct Here {
    server: Option<ServerId>,
    pane: Option<String>,
}

impl Here {
    fn read() -> Self {
        Self {
            server: doors::caller_server(),
            pane: doors::calling_pane_id(),
        }
    }

    fn tmux(&self) -> Option<(&ServerId, &str)> {
        self.server.as_ref().zip(self.pane.as_deref())
    }

    fn caller(&self) -> Caller {
        match self.tmux() {
            Some((server, _)) => Caller {
                in_tmux: true,
                ae_owned: owned(server),
                server: server_of(server),
            },
            None => Caller {
                in_tmux: false,
                ae_owned: false,
                server: Server::None,
            },
        }
    }
}

/// Take the lock's holder place: the record names this process, and stale
/// requests of an older holder are cleared. `None` when the record cannot be
/// written, which leaves the app without a record and the lock dropped.
fn hold(file: File, root: &Path, here: &Here) -> Option<Guard> {
    let pid = std::process::id();
    let identity = identity_of(pid)?;
    let placed = here
        .tmux()
        .and_then(|(server, pane)| place_of(server, pane).map(|place| (server_of(server), place)));
    let (server, (session, window, pane)) =
        placed.unwrap_or_else(|| (Server::None, (String::new(), String::new(), String::new())));
    let held = Instance {
        pid,
        identity: identity.clone(),
        server,
        session,
        window,
        pane,
    };
    let bytes = render_instance(&held).or_else(|| {
        render_instance(&Instance {
            server: Server::None,
            session: String::new(),
            window: String::new(),
            pane: String::new(),
            ..held
        })
    })?;
    remove(&root.join(REQUEST));
    publish(&root.join(INSTANCE), &bytes).ok()?;
    let now = Instant::now();
    Some(Guard {
        _lock: file,
        root: root.to_path_buf(),
        pid,
        identity,
        born: now,
        polled: now,
        swept: false,
        honoured: None,
    })
}

/// [`hold`], or the refusal that says the record cannot be written.
fn held_or_refused(
    file: File,
    root: &Path,
    asked: &Asked<'_>,
    here: &Here,
    err: &mut impl Write,
) -> crate::error::Result<Entry> {
    if let Some(guard) = hold(file, root, here) {
        Ok(Entry::Hold(guard))
    } else {
        say(err, &doubt_line(Doubt::Unreadable, 0))?;
        Ok(Entry::Exit(asked.refused))
    }
}

fn say(err: &mut impl Write, line: &str) -> crate::error::Result<()> {
    err.write_all(line.as_bytes())?;
    Ok(())
}

/// The record of the running app, read until it names a holder or `SETTLE`
/// passes: the holder takes the lock before it writes its record.
fn settled(root: &Path, here: &Here) -> (Record, Verdict) {
    let began = Instant::now();
    let caller = here.caller();
    loop {
        let record = read_record(root);
        let live = match &record {
            Record::Parsed(held) => liveness(held),
            _ => Liveness::Unknown,
        };
        let verdict = judge(&record, Hold::Held, live, &caller);
        let settling = matches!(
            verdict,
            Verdict::Refuse(Doubt::Unreadable | Doubt::NotHolder)
        );
        if !settling || began.elapsed() >= SETTLE {
            return (record, verdict);
        }
        std::thread::sleep(TICK);
    }
}

/// The start sequence of a governed `ae app`, before it takes its terminal.
///
/// # Errors
///
/// [`crate::error::Error::Io`] when `err` cannot be written.
pub(super) fn enter(
    root: &Path,
    asked: &Asked<'_>,
    err: &mut impl Write,
) -> crate::error::Result<Entry> {
    enter_at(root, asked, &Here::read(), err)
}

/// [`enter`] for a caller whose place is already read.
fn enter_at(
    root: &Path,
    asked: &Asked<'_>,
    here: &Here,
    err: &mut impl Write,
) -> crate::error::Result<Entry> {
    if let Some((server, pane)) = here.tmux() {
        match transport::observe_pane_console(server, pane) {
            Some(stamp) if stamp.is_empty() => {}
            Some(_) => return Ok(Entry::Exempt),
            None => {
                say(err, &doubt_line(Doubt::Stamp, 0))?;
                return Ok(Entry::Exit(asked.refused));
            }
        }
    }
    let lock = root.join(LOCK);
    if !asked.terminal && matches!(store::source_presence(&lock), SourcePresence::Absent) {
        return Ok(Entry::NoTerminal);
    }
    if matches!(store::source_presence(root), SourcePresence::Absent) {
        // A first start on a machine that has never made its state root: no
        // app can have held a record there, and the lock file has no
        // directory to live in. Nothing to protect, nothing to take over.
        return Ok(Entry::Exempt);
    }
    let (mut file, mut hold) = try_lock(&lock);
    if asked.handed {
        // An exec restart keeps its pid and start identity: its own record is
        // its own, and only another app holding the lock makes it a guest.
        let own = std::process::id();
        let identity = identity_of(own).unwrap_or_default();
        let record = read_record(root);
        if restart(hold, &record, own, &identity) == Restart::Fresh {
            return Ok(Entry::Fresh);
        }
        if hold == Hold::Held {
            // The record names this very process, yet something still holds
            // the lock (a descriptor the exec carried): it lets go in a
            // moment, or this restart runs as a guest. It never steals.
            let began = Instant::now();
            while hold == Hold::Held && began.elapsed() < SETTLE {
                std::thread::sleep(TICK);
                (file, hold) = try_lock(&lock);
            }
            if hold == Hold::Held {
                say(err, &doubt_line(Doubt::NotHolder, own))?;
                return Ok(Entry::Exit(asked.refused));
            }
        }
    }
    match (file, hold) {
        (Some(file), _) => entered_free(file, root, asked, here, err),
        (None, Hold::Unreadable) => {
            say(err, &doubt_line(Doubt::Unreadable, 0))?;
            Ok(Entry::Exit(asked.refused))
        }
        (None, _) => entered_held(root, asked, here, err),
    }
}

/// Try the instance lock once, with no wait.
fn try_lock(lock: &Path) -> (Option<File>, Hold) {
    match store::lock(lock, Duration::ZERO) {
        Ok(file) => (Some(file), Hold::Free),
        Err(why) if why.kind() == ErrorKind::WouldBlock => (None, Hold::Held),
        Err(_) => (None, Hold::Unreadable),
    }
}

/// The lock was free.
fn entered_free(
    file: File,
    root: &Path,
    asked: &Asked<'_>,
    here: &Here,
    err: &mut impl Write,
) -> crate::error::Result<Entry> {
    if !asked.terminal {
        return Ok(Entry::NoTerminal);
    }
    let record = read_record(root);
    let own = std::process::id();
    let mine = matches!(&record, Record::Parsed(held)
        if held.pid == own && identity_of(own).as_deref() == Some(held.identity.as_str()));
    let live = match &record {
        Record::Parsed(held) if !mine => liveness(held),
        _ => Liveness::Gone,
    };
    match judge(&record, Hold::Free, live, &here.caller()) {
        Verdict::Reclaim => held_or_refused(file, root, asked, here, err),
        Verdict::Refuse(doubt) => {
            let named = match &record {
                Record::Parsed(held) => held.pid,
                _ => 0,
            };
            say(err, &doubt_line(doubt, named))?;
            Ok(Entry::Exit(asked.refused))
        }
        Verdict::Focus { .. } | Verdict::TakeOver => {
            say(err, &doubt_line(Doubt::Unreadable, 0))?;
            Ok(Entry::Exit(asked.refused))
        }
    }
}

/// Another app holds the lock.
fn entered_held(
    root: &Path,
    asked: &Asked<'_>,
    here: &Here,
    err: &mut impl Write,
) -> crate::error::Result<Entry> {
    let (record, verdict) = settled(root, here);
    let named = match &record {
        Record::Parsed(held) => held.pid,
        _ => 0,
    };
    match (verdict, record) {
        (Verdict::Refuse(doubt), _) => {
            say(err, &doubt_line(doubt, named))?;
            Ok(Entry::Exit(asked.refused))
        }
        (Verdict::Focus { session }, Record::Parsed(held)) => {
            if focus(here, &session, &held.pane) {
                say(err, &focused_line(&held.pane, asked.home))?;
                return Ok(Entry::Exit(0));
            }
            // A caller on the running app's own server never takes it over: a
            // client tmux cannot name, or two it cannot tell apart, is doubt.
            say(err, &focus_failed_line(&held.pane))?;
            Ok(Entry::Exit(asked.refused))
        }
        (_, Record::Parsed(held)) => take_over(root, asked, &held, err),
        (Verdict::Reclaim | Verdict::Focus { .. } | Verdict::TakeOver, _) => {
            say(err, &doubt_line(Doubt::Unreadable, 0))?;
            Ok(Entry::Exit(asked.refused))
        }
    }
}

/// Hand the caller's client to the running app's session and pane: the client
/// whose active pane is the caller's own. `false` when it could not be done.
fn focus(here: &Here, session: &str, pane: &str) -> bool {
    let Some((server, own)) = here.tmux() else {
        return false;
    };
    let Some(clients) = transport::observe_clients(server) else {
        return false;
    };
    let Ok(client) = open::copy_client(&clients, own) else {
        return false;
    };
    transport::jump_client(server, &client, session, pane)
}

/// A nonce no other request shares: the OS's own randomness through the
/// standard library's hasher seeds.
fn nonce() -> String {
    use std::hash::{BuildHasher, Hasher};
    let state = std::collections::hash_map::RandomState::new();
    let mut high = state.build_hasher();
    high.write_u32(std::process::id());
    high.write_i64(Timestamp::now().epoch());
    let low = std::collections::hash_map::RandomState::new().build_hasher();
    format!("{:016x}{:016x}", high.finish(), low.finish())
}

/// Ask the running app to hand over, wait for it, and take its place.
fn take_over(
    root: &Path,
    asked: &Asked<'_>,
    held: &Instance,
    err: &mut impl Write,
) -> crate::error::Result<Entry> {
    if !asked.terminal {
        return Ok(Entry::NoTerminal);
    }
    let refuse = |err: &mut dyn Write, line: String| -> crate::error::Result<Entry> {
        err.write_all(line.as_bytes())?;
        Ok(Entry::Exit(asked.refused))
    };
    let nonce = nonce();
    let request = Request {
        target_pid: held.pid,
        target_identity: held.identity.clone(),
        requester: std::process::id(),
        nonce: nonce.clone(),
    };
    let written = render_request(&request)
        .ok_or_else(|| "the request is not spellable".to_owned())
        .and_then(|bytes| publish(&root.join(REQUEST), &bytes));
    if written.is_err() {
        return refuse(err, doubt_line(Doubt::Unreadable, held.pid));
    }
    let lock = root.join(LOCK);
    let began = Instant::now();
    let mut answered = false;
    loop {
        if !answered {
            let ack = read_ack(root);
            match phase(ack.as_ref(), &nonce, began.elapsed()) {
                Phase::Acked => {
                    answered = true;
                    withdraw(root, &nonce);
                }
                Phase::TimedOut => {
                    withdraw(root, &nonce);
                    return refuse(err, timeout_line(held.pid));
                }
                Phase::Waiting => {}
            }
        } else if began.elapsed() >= HANDOVER_WAIT {
            return refuse(err, release_line(held.pid));
        }
        if let Ok(file) = store::lock(&lock, Duration::ZERO) {
            withdraw(root, &nonce);
            let here = Here::read();
            return held_or_refused(file, root, asked, &here, err);
        }
        std::thread::sleep(TICK);
    }
}

/// Remove this process's own request: another app's newer request stays.
fn withdraw(root: &Path, nonce: &str) {
    if read_request(root).is_some_and(|request| request.nonce == nonce) {
        remove(&root.join(REQUEST));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "ae-appinstance-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).expect("scratch root");
            Self(root)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn nowhere() -> Here {
        Here {
            server: None,
            pane: None,
        }
    }

    /// A guard for this very process, as a start outside tmux takes it.
    fn guard_in(root: &Scratch) -> Guard {
        let file = store::lock(&root.0.join(LOCK), Duration::ZERO).expect("the free lock");
        hold(file, &root.0, &nowhere()).expect("the record is written")
    }

    /// Whether anything stands at `name` under the scratch root.
    fn present(root: &Scratch, name: &str) -> bool {
        !matches!(
            store::source_presence(&root.0.join(name)),
            SourcePresence::Absent
        )
    }

    fn nonce_of(digit: char) -> String {
        digit.to_string().repeat(NONCE_LEN)
    }

    #[test]
    fn the_refusal_and_focus_failure_lines_are_these_words() {
        assert_eq!(
            focus_failed_line("%3"),
            "ae app is already open at %3; this client could not be switched to it\n"
        );
        assert_eq!(
            release_line(41),
            "ae app: 41 handed over but still holds the app record\n"
        );
        assert_ne!(release_line(41), timeout_line(41));
    }

    #[test]
    fn a_start_identity_keeps_its_letters_and_digits_only() {
        assert_eq!(
            identity_text("Tue Nov 24 13:51:00 2026\n").as_deref(),
            Some("TueNov241351002026")
        );
        assert_eq!(identity_text(" \n"), None);
        assert_eq!(identity_text(&"x".repeat(IDENTITY_MAX + 1)), None);
    }

    #[test]
    fn a_nonce_is_32_hex_digits_and_never_repeats() {
        let (one, two) = (nonce(), nonce());
        assert!(nonce_ok(&one) && nonce_ok(&two), "{one} {two}");
        assert_ne!(one, two);
    }

    #[test]
    fn the_process_table_names_this_process_gone_or_reused() {
        let own = std::process::id();
        let identity = identity_of(own).expect("ps names this process");
        let record = |pid: u32, identity: &str| Instance {
            pid,
            identity: identity.to_owned(),
            server: Server::None,
            session: String::new(),
            window: String::new(),
            pane: String::new(),
        };
        assert_eq!(liveness(&record(own, &identity)), Liveness::Same);
        assert_eq!(liveness(&record(own, "OtherIdentity")), Liveness::Reused);
        assert_eq!(liveness(&record(u32::MAX - 1, &identity)), Liveness::Gone);
    }

    #[test]
    fn the_holder_writes_a_record_that_names_itself_and_nothing_else() {
        let root = Scratch::new();
        let _guard = guard_in(&root);
        let Record::Parsed(held) = read_record(&root.0) else {
            panic!("the record parses");
        };
        assert_eq!(held.pid, std::process::id());
        assert_eq!(Some(held.identity), identity_of(std::process::id()));
        assert_eq!(held.server, Server::None);
        assert!(held.session.is_empty() && held.window.is_empty() && held.pane.is_empty());
        assert!(
            store::lock(&root.0.join(LOCK), Duration::ZERO).is_err(),
            "held"
        );
        assert!(!present(&root, REQUEST) && !present(&root, ACK));
    }

    #[test]
    fn an_absent_or_damaged_record_is_read_as_such() {
        let root = Scratch::new();
        assert_eq!(read_record(&root.0), Record::Absent);
        std::fs::write(root.0.join(INSTANCE), b"1 nonsense\n").expect("damage");
        assert_eq!(read_record(&root.0), Record::Damaged);
        std::fs::write(root.0.join(INSTANCE), vec![b'a'; INSTANCE_CAP + 1]).expect("oversize");
        assert_eq!(read_record(&root.0), Record::Damaged);
    }

    #[test]
    fn the_guard_honours_only_a_request_that_names_its_own_pid_and_identity() {
        let root = Scratch::new();
        let mut guard = guard_in(&root);
        let later = guard.polled + TICK * 2;
        let other = Request {
            target_pid: std::process::id().wrapping_add(1),
            target_identity: guard.identity.clone(),
            requester: 7,
            nonce: nonce_of('a'),
        };
        std::fs::write(
            root.0.join(REQUEST),
            render_request(&other).expect("spelled"),
        )
        .expect("write");
        assert!(!guard.poll(later), "another pid's request is left alone");
        assert!(present(&root, REQUEST), "and stays where it is");
        assert_eq!(guard.requester(), None);

        let mine = Request {
            target_pid: guard.pid,
            target_identity: guard.identity.clone(),
            requester: 9,
            nonce: nonce_of('b'),
        };
        std::fs::write(
            root.0.join(REQUEST),
            render_request(&mine).expect("spelled"),
        )
        .expect("write");
        assert!(!guard.poll(later), "no second look inside one tick");
        assert!(guard.poll(later + TICK * 2));
        assert_eq!(guard.requester(), Some(9));
        assert!(guard.poll(later), "an honoured request stays honoured");

        assert!(!present(&root, ACK), "nothing is acknowledged early");
        guard.acknowledge();
        assert_eq!(
            read_ack(&root.0).expect("the acknowledgement").nonce,
            nonce_of('b')
        );
        assert!(!present(&root, REQUEST), "the request is cleared");
    }

    #[test]
    fn the_guard_sweeps_a_stale_acknowledgement_only_after_its_own_grace() {
        let root = Scratch::new();
        let mut guard = guard_in(&root);
        std::fs::write(
            root.0.join(ACK),
            render_ack(&Ack {
                nonce: nonce_of('c'),
            })
            .expect("spelled"),
        )
        .expect("write");
        assert!(!guard.poll(guard.polled + TICK * 2));
        assert!(present(&root, ACK), "too early to sweep");
        guard.born -= SWEEP + TICK;
        assert!(!guard.poll(guard.polled + TICK * 2));
        assert!(!present(&root, ACK), "swept once the grace has passed");
    }

    #[test]
    fn a_start_with_no_state_root_yet_is_todays_app_and_a_root_that_is_no_directory_is_doubt() {
        let scratch = Scratch::new();
        let asked = Asked {
            home: None,
            handed: false,
            terminal: true,
            refused: 9,
        };
        // A machine that never made its state root: no app held a record there.
        let absent = scratch.0.join("never-made");
        let mut err = Vec::new();
        assert!(matches!(
            enter_at(&absent, &asked, &nowhere(), &mut err),
            Ok(Entry::Exempt)
        ));
        assert!(err.is_empty(), "a first start says nothing");
        assert!(
            matches!(store::source_presence(&absent), SourcePresence::Absent),
            "and makes no root of its own"
        );
        // Something that is no directory is not "no root": it is doubt.
        let file = scratch.0.join("a-file");
        std::fs::write(&file, b"x").expect("a file where the root should be");
        let mut err = Vec::new();
        assert!(matches!(
            enter_at(&file, &asked, &nowhere(), &mut err),
            Ok(Entry::Exit(9))
        ));
        assert_eq!(
            String::from_utf8_lossy(&err),
            doubt_line(Doubt::Unreadable, 0)
        );
        // A root that exists: this start takes the instance.
        assert!(matches!(
            enter_at(&scratch.0, &asked, &nowhere(), &mut Vec::new()),
            Ok(Entry::Hold(_))
        ));
    }

    #[test]
    fn a_waiter_withdraws_its_own_request_and_never_another_one() {
        let root = Scratch::new();
        let request = |digit: char| Request {
            target_pid: 5,
            target_identity: "Ident".to_owned(),
            requester: 6,
            nonce: nonce_of(digit),
        };
        std::fs::write(
            root.0.join(REQUEST),
            render_request(&request('d')).expect("spelled"),
        )
        .expect("write");
        withdraw(&root.0, &nonce_of('e'));
        assert!(present(&root, REQUEST), "a newer request stays");
        withdraw(&root.0, &nonce_of('d'));
        assert!(!present(&root, REQUEST));
    }
}
