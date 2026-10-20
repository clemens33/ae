//! The console's terminal: raw reads on a thread, the ownership reading, and
//! each act carried out. What any of it means is `console::input`'s to say.

use std::io::{IsTerminal as _, Read as _};
use std::os::fd::AsFd as _;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use super::input::{self, Effect, Input, Reading, Screen, Seen, Size};
use super::lane::Seat;
use super::needs::SeatRef;
use super::open::{self, Refusal};
use super::shell::{Place, ShellArgv, popup_args, popup_client, refused, said};
use super::{Console, holder, submit};
use crate::inventory::ServerId;
use crate::{archive, doors, lifecycle, meta, theme, time, tmux, tracked, transport};

/// Stamped terminal reads, until the terminal's input ends.
struct Reads(Option<Receiver<(Instant, Vec<u8>)>>);

/// What a wait for a read came to.
#[derive(Debug, PartialEq, Eq)]
enum Got {
    Read(Instant, Vec<u8>),
    /// The input just ended; every later wait sleeps to its deadline.
    Ended,
    Due,
}

impl Reads {
    fn wait(&mut self, deadline: Instant) -> Got {
        let left = deadline.saturating_duration_since(Instant::now());
        let Some(reads) = &self.0 else {
            std::thread::sleep(left);
            return Got::Due;
        };
        match reads.recv_timeout(left) {
            Ok((stamp, bytes)) => Got::Read(stamp, bytes),
            Err(RecvTimeoutError::Timeout) => Got::Due,
            Err(RecvTimeoutError::Disconnected) => {
                self.0 = None;
                Got::Ended
            }
        }
    }
}

/// A read `Ok(0)` that came back at least this late is the raw mode's read
/// timeout, never an end: `ae app`'s raw mode returns an empty read after
/// 100 ms without input, while a hung-up terminal answers at once.
const TICK_FLOOR: Duration = Duration::from_millis(50);

/// Fast empty reads, each a [`TICK_FLOOR`] apart, must go on this long with
/// no input between them before they end the input. One alone is no end: a
/// raw read's timeout can come back early (one ~33 ms read measured on macOS
/// under tmux), and no termios call tells a hung-up terminal from a live one
/// there, while a hung-up terminal answers empty at once, every read.
const HANGUP_AFTER: Duration = Duration::from_secs(1);

/// What an empty read means to a reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmptyRead {
    /// The terminal waits for a byte (`ae chat`): empty is the end.
    Ends,
    /// The terminal's read times out (`ae app`): empty is usually a tick.
    MayTimeOut,
}

/// What an empty read meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ZeroRead {
    /// The read timeout: nothing was typed; read again.
    Tick,
    /// Too fast for a timeout: wait a [`TICK_FLOOR`], then read again.
    Wait,
    /// The input ended.
    Ended,
}

/// The run of fast empty reads a reader has seen: since when, if one runs.
#[derive(Debug, Default)]
pub(crate) struct Zeros {
    fast_since: Option<Instant>,
}

impl Zeros {
    /// What an empty read that took `elapsed` and returned at `now` meant.
    #[must_use]
    pub(crate) fn empty(&mut self, elapsed: Duration, now: Instant) -> ZeroRead {
        if elapsed >= TICK_FLOOR {
            self.fast_since = None;
            return ZeroRead::Tick;
        }
        let since = *self.fast_since.get_or_insert(now);
        if now.saturating_duration_since(since) >= HANGUP_AFTER {
            ZeroRead::Ended
        } else {
            ZeroRead::Wait
        }
    }

    /// A read returned bytes, or the reader was parked: whatever ran before
    /// is no evidence of an end.
    pub(crate) fn reset(&mut self) {
        self.fast_since = None;
    }
}

/// The handle that parks stdin's reader while another program owns the
/// terminal, so no key that program is sent is read here.
#[derive(Clone, Default)]
pub(crate) struct Pause(Arc<(Mutex<Parked>, Condvar)>);

/// Whether the reader is asked to park, and whether it has.
#[derive(Default)]
struct Parked {
    asked: bool,
    parked: bool,
}

impl Pause {
    fn state(&self) -> MutexGuard<'_, Parked> {
        self.0.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Ask the reader to park and wait up to `within` for it to say it has.
    /// `false` takes the ask back: the reader is still reading.
    pub(crate) fn hold(&self, within: Duration) -> bool {
        let mut state = self.state();
        state.asked = true;
        self.0.1.notify_all();
        let deadline = Instant::now() + within;
        while !state.parked {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                state.asked = false;
                self.0.1.notify_all();
                return false;
            }
            state = match self.0.1.wait_timeout(state, left) {
                Ok((state, _)) => state,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
        true
    }

    /// Let the reader read again.
    pub(crate) fn release(&self) {
        let mut state = self.state();
        state.asked = false;
        self.0.1.notify_all();
    }

    /// The reader's side, between two reads: park while asked; whether it
    /// parked.
    fn checkpoint(&self) -> bool {
        let mut state = self.state();
        if !state.asked {
            return false;
        }
        state.parked = true;
        self.0.1.notify_all();
        while state.asked {
            state = self.0.1.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
        state.parked = false;
        true
    }
}

/// Read `stdin` unbuffered, stamping each read when it completes and before the
/// send that may block on a full channel: a stamp is when the read returned,
/// never when the send got through, and never how long the kernel held input.
/// Between two reads it parks while `pause` asks it to.
fn reader(
    mut stdin: std::fs::File,
    reads: &SyncSender<(Instant, Vec<u8>)>,
    pause: &Pause,
    empty: EmptyRead,
) {
    let mut buffer = [0; 4096];
    let mut zeros = Zeros::default();
    loop {
        if pause.checkpoint() {
            zeros.reset();
        }
        let started = Instant::now();
        let read = stdin.read(&mut buffer);
        if matches!(read, Ok(0)) && empty == EmptyRead::MayTimeOut {
            match zeros.empty(started.elapsed(), Instant::now()) {
                ZeroRead::Tick => continue,
                ZeroRead::Wait => {
                    std::thread::sleep(TICK_FLOOR);
                    continue;
                }
                ZeroRead::Ended => return,
            }
        }
        let stamp = Instant::now();
        crate::read_gate("@app-read");
        match read {
            Ok(0) => return,
            Ok(n) => {
                zeros.reset();
                if reads.send((stamp, buffer[..n].to_vec())).is_err() {
                    return;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return,
        }
    }
}

/// What the reader thread sends: each read's bytes and when it returned.
type StampedReads = Receiver<(Instant, Vec<u8>)>;

/// Stdin's raw reads, stamped, from a reader thread; `None` when stdin is
/// not a terminal ae can hold. The terminal waits for a byte, so an empty
/// read ends them.
pub(crate) fn stdin_reads() -> Option<Receiver<(Instant, Vec<u8>)>> {
    spawn_reader(EmptyRead::Ends).map(|(reads, _)| reads)
}

/// [`stdin_reads`] from a terminal whose reads time out, and the [`Pause`]
/// that parks its reader.
pub(crate) fn stdin_reads_pausable() -> Option<(StampedReads, Pause)> {
    spawn_reader(EmptyRead::MayTimeOut)
}

/// The reader thread over stdin, reading empty reads as `empty` says.
fn spawn_reader(empty: EmptyRead) -> Option<(StampedReads, Pause)> {
    let stdin = std::io::stdin();
    let owned = stdin
        .is_terminal()
        .then(|| stdin.as_fd().try_clone_to_owned())?
        .ok()?;
    let (send, reads) = sync_channel(8);
    let file = std::fs::File::from(owned);
    let pause = Pause::default();
    let parked = pause.clone();
    std::thread::spawn(move || reader(file, &send, &parked, empty));
    Some((reads, pause))
}

/// The lead pair a console asks, main seat first: exactly one seat in slot
/// `main`, and every name one an agent name may be — refused by name, never
/// dropped. The meta keeps a name verbatim, and an ask would read `%3` as a
/// pane or `--cross-session` as a flag.
pub(crate) fn pair_of(mut seats: Vec<Seat>) -> Result<Vec<Seat>, String> {
    if seats.iter().filter(|seat| seat.slot == "main").count() != 1 {
        return Err("the meta names no single main seat".to_owned());
    }
    let named = |seat: &&Seat| crate::config::is_agent_name(&seat.name);
    if let Some(bad) = seats.iter().find(|seat| !named(seat)) {
        return Err(format!("the {} seat is not an agent name", bad.slot));
    }
    seats.sort_by_key(|seat| seat.slot != "main");
    Ok(seats)
}

/// Whether `pair`, slots and names, is still the session's lead pair. A
/// seat's profile is how it runs, not where an ask goes, so a reseat of
/// either seat leaves the pair what it was; this is the one comparer.
pub(crate) fn still(pair: &[Seat], seats: Result<Vec<Seat>, String>) -> Result<(), String> {
    let now = pair_of(seats?)?;
    let same = |(was, is): (&Seat, &Seat)| was.slot == is.slot && was.name == is.name;
    if now.len() == pair.len() && pair.iter().zip(&now).all(same) {
        return Ok(());
    }
    Err("the lead pair changed since this chat opened - restart this chat".to_owned())
}

/// The writer lease on `console`'s session, taken without waiting; a refusal
/// names the app that holds it, or the lock node ae will not open. The holder
/// is named only from a record that parses; anything else reads `holder unknown`.
pub(crate) fn lease(console: &Console) -> Result<std::fs::File, String> {
    let store = crate::store::open(&console.dir);
    store.console_writer().map_err(|why| match why.kind() {
        std::io::ErrorKind::WouldBlock => {
            let at = holder_place(console);
            format!("an ae app is writing to {} ({at})", console.name)
        }
        kind => lease_unusable(kind),
    })
}

/// Where `console`'s writer lease record says its holder is: `held at
/// <session>:<window>`, or `holder unknown` for a record that does not parse.
fn holder_place(console: &Console) -> String {
    let record = crate::store::open(&console.dir).console_writer_holder();
    let held = record.and_then(|body| holder::parse(&body));
    held.map_or_else(
        || "holder unknown".to_owned(),
        |(session, window)| format!("held at {session}:{window}"),
    )
}

/// Why the writer lease could not be taken, for any kind but another holder.
fn lease_unusable(kind: std::io::ErrorKind) -> String {
    match kind {
        std::io::ErrorKind::InvalidInput => "the writer lease is not a regular file".to_owned(),
        kind => format!("the writer lease could not be taken ({kind})"),
    }
}

/// What an `ae app`'s submit holds from its admission through that ask's
/// delivery: the writer lease, then the admission lock, each taken without
/// waiting so the loop's thread never stalls on either. Another holder of
/// either refuses at once, naming the lease's recorded holder when it is the
/// lease; an admission refused lets the lease go with it.
///
/// # Errors
///
/// Why the submit may not start now.
pub(crate) fn send_locks(console: &Console) -> Result<(std::fs::File, std::fs::File), String> {
    let store = crate::store::open(&console.dir);
    let busy = |at: String| {
        format!(
            "another window is sending to {}{at} — Enter retries",
            console.name
        )
    };
    let lease = store.console_writer().map_err(|why| match why.kind() {
        std::io::ErrorKind::WouldBlock => busy(format!(" ({})", holder_place(console))),
        kind => lease_unusable(kind),
    })?;
    let admission = store
        .console_admission_now()
        .map_err(|why| match why.kind() {
            std::io::ErrorKind::WouldBlock => busy(String::new()),
            _ => format!("the chat admission lock is not free ({why})"),
        })?;
    Ok((lease, admission))
}

/// Whether `console`'s pane `me` owns its session's input on `server`:
/// `None` when tmux did not answer.
pub(crate) fn owns(
    console: &Console,
    server: Option<&ServerId>,
    me: Option<&str>,
) -> Option<Result<(), String>> {
    let server = server?;
    let panes = transport::observe_window_panes(server, &console.name)?;
    let stamp = transport::observe_session_option(server, &console.name, theme::SESSION_ID_OPTION)?;
    Some(submit::owner(&panes, me, &stamp, &console.uuid))
}

/// Run `line`, the human's own command, in the session's work dir `dir`: a
/// popup on the tmux client whose active pane is `place`'s, or a detached
/// `/bin/sh` outside tmux. What to say when it started. The line goes to tmux
/// or the shell and nowhere else: it is never journaled, drafted or sent.
///
/// # Errors
///
/// Why nothing started.
pub(crate) fn shell(place: Place<'_>, dir: &str, line: &str) -> Result<String, String> {
    if !lifecycle::dir_exists(std::path::Path::new(dir)) {
        return Err(format!("its work dir {dir} is gone"));
    }
    let Place::Tmux { server, pane } = place else {
        let argv = ShellArgv::of(line).ok_or("a fact failed its grammar")?;
        if transport::run_shell_line(&argv, std::path::Path::new(dir)) {
            return Ok(said(line, false));
        }
        return Err("/bin/sh did not start".to_owned());
    };
    let clients = transport::observe_clients(server).ok_or("the tmux clients did not answer")?;
    let client = popup_client(&clients, pane)?;
    let args = popup_args(server, &client, dir, line).ok_or("a fact failed its grammar")?;
    if transport::open_popup(server, &args) {
        Ok(said(line, true))
    } else {
        Err("tmux did not start".to_owned())
    }
}

/// `seat`'s pane in `console`'s session on `server`, proven against the meta
/// and tmux NOW: the one proof `/open` makes, in the chat and in the app.
pub(crate) fn prove_open(
    server: &ServerId,
    console: &Console,
    seat: &SeatRef,
) -> Result<open::Target, Refusal> {
    let Ok(bytes) = meta::read_bytes(&console.dir) else {
        return Err(Refusal::Unread("the meta".to_owned()));
    };
    if archive::canonical_uuid(&lifecycle::meta_value(&bytes, "session_id")) != console.uuid {
        return Err(Refusal::SessionReplaced);
    }
    let meta = meta::Meta::parse(&String::from_utf8_lossy(&bytes));
    let session = &console.name;
    let session_id = transport::observe_session_id(server, session);
    let stamp = transport::observe_session_option(server, session, theme::SESSION_ID_OPTION);
    let slots = transport::observe_slots(server, session);
    let panes = transport::observe_window_panes(server, session);
    let members = transport::observe_picker_panes(server);
    open::target(&open::Facts {
        seat,
        roster: meta.roster(),
        bound_uuid: &console.uuid,
        session_id: session_id.as_deref(),
        uuid_stamp: stamp.as_deref(),
        slots: slots.as_deref(),
        panes: panes.as_deref(),
        members: members.as_deref(),
    })
}

/// Carry one ask of `seat` through the helpers' own tracked path, as the
/// console, under the admission `owns` re-proves: the request id and what
/// became of it, or why it was refused before anything was sent.
pub(crate) fn submit_ask(
    console: &Console,
    record: Arc<crate::store::DraftRecord>,
    owns: impl FnOnce() -> Result<(), String>,
    raw: &[u8],
    seat: &str,
    body: String,
) -> Result<(String, submit::Outcome), String> {
    let (ask, admitted) = admit_ask(console, record, owns, raw, seat, body)?;
    let id = ask.id().to_owned();
    let outcome = admitted.finish(&mut console.held.borrow_mut(), &id, || ask.run());
    Ok((id, outcome))
}

/// The admission of one ask of `seat`: the console admission lock, `owns`,
/// the kept draft and the open-ask cap, all proven now, and the ask ready to
/// carry. The lock is held for as long as the [`submit::Admitted`] lives.
///
/// # Errors
///
/// Why nothing may be delivered.
pub(crate) fn admit_ask(
    console: &Console,
    record: Arc<crate::store::DraftRecord>,
    owns: impl FnOnce() -> Result<(), String>,
    raw: &[u8],
    seat: &str,
    body: String,
) -> Result<(Carry, submit::Admitted), String> {
    let admitted = submit::admit_record(
        &mut console.held.borrow_mut(),
        &console.dir,
        &console.name,
        raw,
        owns,
        record,
    )?;
    Ok((Carry::of(console, seat, body), admitted))
}

/// [`admit_ask`] under the admission lock `lock` the caller already holds.
///
/// # Errors
///
/// Why nothing may be delivered.
pub(crate) fn admit_ask_locked(
    console: &Console,
    record: Arc<crate::store::DraftRecord>,
    lock: std::fs::File,
    owns: impl FnOnce() -> Result<(), String>,
    raw: &[u8],
    seat: &str,
    body: String,
) -> Result<(Carry, submit::Admitted), String> {
    let admitted = submit::admit_record_locked(
        &mut console.held.borrow_mut(),
        &console.dir,
        &console.name,
        raw,
        owns,
        record,
        lock,
    )?;
    Ok((Carry::of(console, seat, body), admitted))
}

/// One ask a writer with no composer of its own carries into the shared chat
/// of session `session` (a chat bridge): the incarnation and lead pair it
/// resolved, the seat it named if any, and the request id it mints.
pub(crate) struct External<'a> {
    pub(crate) dir: &'a std::path::Path,
    pub(crate) session: &'a str,
    pub(crate) session_id: &'a str,
    pub(crate) pair: &'a [String],
    pub(crate) seat: Option<&'a str>,
    pub(crate) body: String,
    pub(crate) minted: (time::Timestamp, u64),
}

/// What the plain-line router weighs, per pair seat in the pair's order: the
/// verdict `ae list` shows and the judged quota.
pub(crate) struct Standing {
    pub(crate) takes: Vec<crate::app::route::Take>,
    pub(crate) quota: Vec<Option<f64>>,
}

/// One external ask, carried or found already carried.
#[derive(Debug)]
pub(crate) struct Carried {
    pub(crate) id: String,
    pub(crate) seat: String,
    /// What the router said of its choice, when it said anything.
    pub(crate) note: Option<String>,
    pub(crate) outcome: submit::Outcome,
    /// The journal already recorded this ask: nothing was carried again.
    pub(crate) replayed: bool,
}

/// [`submit_external_with`] on the world as it stands: the fleet's verdicts,
/// the pair's quota and the helpers' tracked path.
///
/// # Errors
///
/// Why nothing was carried.
pub(crate) fn submit_external(
    root: &std::path::Path,
    ask: External<'_>,
) -> Result<Carried, String> {
    let (dir, session, pair) = (ask.dir, ask.session, ask.pair);
    submit_external_with(ask, || read_standing(root, dir, session, pair), Carry::run)
}

/// The fleet's verdict and the judged quota of each `pair` seat of `session`.
fn read_standing(
    root: &std::path::Path,
    dir: &std::path::Path,
    session: &str,
    pair: &[String],
) -> Standing {
    let (_, world) = crate::current_world(root);
    let entry = world.sessions.iter().find(|entry| entry.name == session);
    let agents = entry.map(|entry| entry.agents.as_slice());
    let takes = pair
        .iter()
        .map(|name| {
            let agent = agents.and_then(|agents| agents.iter().find(|a| a.name == *name));
            crate::app::route::take(agent)
        })
        .collect();
    let read = crate::autoreseat::leg::pair_pressure(root, dir, time::Timestamp::now());
    let quota = pair
        .iter()
        .map(|name| {
            read.iter()
                .find(|(seat, _)| seat == name)
                .and_then(|(_, used)| *used)
        })
        .collect();
    Standing { takes, quota }
}

/// Carry `ask` into its session's shared chat through the console's own
/// admission and the helpers' tracked path, as the console: the seat it names,
/// else the one the plain-line router picks from `standing` and the chat's
/// open thread, as the app picks it. Under the admission lock the incarnation
/// and the lead pair are proven to be the ones `ask` resolved, and an ask the
/// journal already records under the same request id is answered from that
/// record and never carried again.
///
/// # Errors
///
/// Why nothing was carried.
pub(crate) fn submit_external_with(
    ask: External<'_>,
    standing: impl FnOnce() -> Standing,
    carry: impl FnOnce(Carry) -> String,
) -> Result<Carried, String> {
    let uuid = archive::canonical_uuid(ask.session_id);
    if uuid.is_empty() {
        return Err("the session records no usable session id".to_owned());
    }
    let console = Console::bound(ask.session.to_owned(), ask.dir.to_owned(), uuid);
    let Some(main) = ask.pair.first() else {
        return Err("the session has no lead pair".to_owned());
    };
    let (seat, body, note) = match ask.seat {
        Some(seat) if ask.pair.iter().any(|name| name == seat) => (seat.to_owned(), ask.body, None),
        Some(seat) => return Err(format!("{seat} is not a lead-pair seat of {}", ask.session)),
        None => {
            let events = super::snapshot_in(&mut console.held.borrow_mut(), ask.dir)?;
            let items = super::lane::thread_items(&events);
            let Standing { takes, quota } = standing();
            let weighed: Vec<_> = ask
                .pair
                .iter()
                .zip(takes)
                .map(|(name, take)| (name.as_str(), take))
                .collect();
            let route =
                crate::app::route::decide(&weighed, &quota, crate::app::route::thread_seat(&items));
            let seat = if route.seat.is_empty() {
                main.clone()
            } else {
                route.seat
            };
            let body = match route.prefix {
                Some(prefix) => format!("{prefix}\n{}", ask.body),
                None => ask.body,
            };
            (seat, body, route.note)
        }
    };
    let ask_carry = Carry::minted(&console, &seat, body, ask.minted);
    let id = ask_carry.id().to_owned();
    let owns = || {
        let now = pair_of(console.seats()?)?;
        let names = now.iter().map(|seat| seat.name.as_str());
        if !names.eq(ask.pair.iter().map(String::as_str)) {
            return Err("the lead pair changed since the message was routed".to_owned());
        }
        let name = console.name();
        let routed = tracked::named_server(console.dir(), name, name);
        routed.map(drop).map_err(|why| why.message())
    };
    let admitted = submit::admit_external(
        &mut console.held.borrow_mut(),
        ask.dir,
        ask.session,
        &id,
        owns,
    )?;
    let (outcome, replayed) = match admitted {
        submit::Admission::Recorded(outcome) => (outcome, true),
        submit::Admission::Fresh(admitted) => {
            let outcome = admitted.finish(&mut console.held.borrow_mut(), &id, || carry(ask_carry));
            (outcome, false)
        }
    };
    Ok(Carried {
        id,
        seat,
        note,
        outcome,
        replayed,
    })
}

/// One ask of a seat, owned whole and ready to be carried: everything the
/// helpers' tracked path takes, so it can run on another thread.
pub(crate) struct Carry {
    dir: std::path::PathBuf,
    session: String,
    seat: String,
    body: String,
    id: String,
    now: time::Timestamp,
    entropy: u64,
}

impl Carry {
    /// The ask of `seat` from `console`, under a request id minted now.
    pub(crate) fn of(console: &Console, seat: &str, body: String) -> Self {
        Self::minted(
            console,
            seat,
            body,
            (time::Timestamp::now(), crate::entropy()),
        )
    }

    /// The ask of `seat` from `console`, under the request id `minted` names.
    fn minted(
        console: &Console,
        seat: &str,
        body: String,
        (now, entropy): (time::Timestamp, u64),
    ) -> Self {
        Self {
            dir: console.dir.clone(),
            session: console.name.clone(),
            seat: seat.to_owned(),
            body,
            id: tracked::request_id(tracked::Kind::Ask.id_prefix(), now, entropy),
            now,
            entropy,
        }
    }

    /// The request id this ask is recorded under.
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    /// The seat this ask goes to.
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the ingress tests"))]
    pub(crate) fn seat(&self) -> &str {
        &self.seat
    }

    /// The text this ask carries.
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the ingress tests"))]
    pub(crate) fn body(&self) -> &str {
        &self.body
    }

    /// Carry the ask through the helpers' own tracked path, as the console,
    /// and say what the path wrote to its error stream.
    pub(crate) fn run(self) -> String {
        let sender = tracked::Sender {
            display: tracked::CONSOLE_SINK.to_owned(),
            slot: String::new(),
            session: self.session,
        };
        let (tail, mut said) = ([self.seat, self.body], Vec::new());
        let run = tracked::run(
            tracked::Kind::Ask,
            &self.dir,
            &tail,
            Some(&sender),
            &crate::own_session(&self.dir),
            self.now,
            self.entropy,
            crate::send_defer(),
            &mut std::io::sink(),
            &mut said,
        );
        if let Err(why) = run {
            said.extend(why.to_string().bytes());
        }
        String::from_utf8_lossy(&said).into_owned()
    }
}

/// A console that takes input from its terminal.
pub(super) struct Term {
    input: Input,
    recovery: submit::Recovery,
    clipboard: crate::clipboard::Job<Instant>,
    pair: Vec<Seat>,
    reads: Reads,
    server: Option<ServerId>,
    /// This console's own pane.
    me: Option<String>,
    /// What the last paint left on the terminal.
    screen: Screen,
}

impl Term {
    /// Input on this process's terminal for `console`'s lead pair. `Err` is
    /// the line saying why there is none: no usable pair, or no terminal.
    pub(super) fn start(console: &Console) -> Result<Self, String> {
        let off = |why: &str| format!("input off: {why}; this chat only reads\n");
        let pair = console.seats().and_then(pair_of).map_err(|why| off(&why))?;
        let Some(reads) = stdin_reads() else {
            return Err(off("stdin is not a terminal"));
        };
        let declared = doors::declared_server(crate::shape::current());
        Ok(Self {
            input: Input::new(pair.iter().map(|seat| seat.name.clone()).collect()),
            recovery: crate::console::submit::Recovery::default(),
            clipboard: None,
            pair,
            reads: Reads(Some(reads)),
            server: doors::launch_target(declared.as_ref()),
            me: doors::calling_pane_id(),
            screen: Screen::styled(console.printed.style().clone()),
        })
    }

    /// Whether this console owns its session's input: `None` when tmux did
    /// not answer.
    fn owns(&self, console: &Console) -> Option<Result<(), String>> {
        owns(console, self.server.as_ref(), self.me.as_deref())
    }

    /// Read ownership now and take the answer.
    pub(super) fn tick(&mut self, console: &Console) -> Vec<Effect> {
        if let Some(seats) = &console.roster {
            self.input.set_seats(seats.clone());
        }
        let reading = match self.owns(console) {
            None => Reading::Unknown,
            Some(Ok(())) => Reading::Owner,
            Some(Err(why)) => Reading::NotOwner(why),
        };
        self.take(console, reading)
    }

    /// Take a fresh ownership `reading`; a promotion also restores the kept draft.
    fn take(&mut self, console: &Console, reading: Reading) -> Vec<Effect> {
        let was = self.input.taking();
        let mut effects = self.input.tick(reading, Instant::now());
        crate::clipboard::sweep(&mut self.input);
        if !was && self.input.taking() {
            if self.input.offering() {
                effects.extend(self.input.reoffer());
            } else if self.input.draft().is_empty() {
                let draft = self
                    .recovery
                    .record(&console.dir)
                    .map_or_else(submit::Draft::Refused, |record| submit::recover(&record));
                effects.extend(self.input.restore(draft));
            }
        }
        effects
    }

    /// Wait for the terminal until `deadline`; what its input did, every ask
    /// and close already carried out, or `None` when it did nothing.
    pub(super) fn wait(&mut self, console: &mut Console, deadline: Instant) -> Option<Vec<Effect>> {
        let epoch = self.input.writer_epoch();
        if let Some(answer) = crate::clipboard::poll(&mut self.clipboard, epoch.as_ref())
            && self.owned(console).is_ok()
        {
            let result = crate::clipboard::finish(&mut self.input, answer);
            return Some(
                result
                    .err()
                    .map(|why| Effect::Print(format!("refused: {why}")))
                    .into_iter()
                    .collect(),
            );
        }
        let deadline = if self.clipboard.is_some() {
            deadline.min(Instant::now() + std::time::Duration::from_millis(100))
        } else {
            deadline
        };
        let deadline = if self.input.offering() {
            deadline.min(Instant::now() + input::ESC_IDLE)
        } else {
            deadline
        };
        let effects = match self.reads.wait(deadline) {
            Got::Read(stamp, bytes) => self.input.chunk(&bytes, stamp),
            Got::Ended => self.input.closed(),
            Got::Due => {
                let effects = self.input.idle(Instant::now());
                if effects.is_empty() {
                    return None;
                }
                effects
            }
        };
        crate::clipboard::sweep(&mut self.input);
        let act = |effect| match effect {
            Effect::DiscardDraft => {
                let result = self.owned(console).and_then(|()| {
                    self.recovery
                        .record(&console.dir)?
                        .clear()
                        .map_err(|why| why.to_string())
                });
                match result {
                    Ok(()) => {
                        self.input.discarded();
                        Effect::Print("kept line discarded".to_owned())
                    }
                    Err(why) => Effect::Print(format!("refused: {why}")),
                }
            }
            Effect::ImagePaste => {
                if self.owned(console).is_ok()
                    && let Some(epoch) = self.input.writer_epoch()
                {
                    crate::clipboard::start(&mut self.clipboard, epoch, console.dir().to_owned());
                }
                Effect::ImagePaste
            }
            Effect::Ask { raw, seat, body } => self.ask(console, &raw, &seat, body),
            Effect::Close(id) => {
                // An app sending holds the admission through its delivery:
                // its lease refuses at once rather than after that wait.
                let owns = || self.owned(console);
                let closed = self.owned(console).and_then(|()| {
                    submit::close_owned_held(
                        &mut console.held.borrow_mut(),
                        &console.dir,
                        &console.name,
                        id.as_deref(),
                        owns,
                    )
                });
                Effect::Print(match closed {
                    Ok(closed) => closed.line(),
                    Err(why) => format!("refused: {why}"),
                })
            }
            Effect::Open(seat) => Effect::Print(self.open(console, &seat)),
            Effect::Shell { raw, line } => self.bang(console, &raw, &line),
            Effect::More(handle) => match console.printed.more_last(&handle) {
                Ok(text) => console.printed.lane(text),
                Err(why) => Effect::Print(why),
            },
            effect => effect,
        };
        Some(effects.into_iter().map(act).collect())
    }

    /// `/open`: prove `seat`'s pane against the meta and tmux NOW, then select
    /// it inside this session, its identity guarded again by the select itself.
    fn open(&self, console: &Console, seat: &SeatRef) -> String {
        let name = &seat.name;
        let Some(server) = self.server.as_ref() else {
            return Refusal::Unread("tmux".to_owned()).line(name);
        };
        match prove_open(server, console, seat).map(|target| transport::open_seat(server, &target))
        {
            Err(refusal) => refusal.line(name),
            Ok(None) => {
                format!("refused: /open {name}: a fact failed its grammar; nothing selected")
            }
            Ok(Some((ran, stdout))) => open::outcome(ran, &stdout, name),
        }
    }

    /// `!`: the human's own line, in a popup on the client showing this owner
    /// chat. Only the owner runs it, read now; the app's lease is not asked,
    /// because a `!` line writes nothing to the session. A refusal puts the
    /// line back.
    fn bang(&mut self, console: &Console, raw: &[u8], line: &str) -> Effect {
        let unread = || Err("tmux did not answer who owns the input".to_owned());
        let started = self.owns(console).unwrap_or_else(unread).and_then(|()| {
            let (server, pane) = (self.server.as_ref().zip(self.me.as_deref()))
                .ok_or("this chat is not in a tmux pane")?;
            shell(Place::Tmux { server, pane }, &console.work_dir()?, line)
        });
        started.map_or_else(
            |why| {
                self.input.put_back(raw);
                Effect::Print(refused(line, &why))
            },
            Effect::Print,
        )
    }

    /// The write that shows `text` and `effects` above the composer, drawn
    /// again for the pane as it is now: its size read every paint, its screen
    /// only after a narrowing.
    pub(super) fn paint(&mut self, text: &str, effects: &[Effect]) -> String {
        self.painted(text, effects, Self::pane)
    }

    /// `paint` with the pane read through `read`, asked only while a composer is
    /// shown or on the screen.
    fn painted(
        &mut self,
        text: &str,
        effects: &[Effect],
        read: impl Fn(&Self) -> Option<tmux::PaneSize>,
    ) -> String {
        let live = self.input.composing() || self.screen.drawn();
        let pane = if live { read(self) } else { None };
        let size = pane.map_or(Size::FALLBACK, |pane| Size {
            width: pane.width,
            height: pane.height,
        });
        let view = self.input.view(size);
        let seen = pane
            .filter(|_| self.screen.needs_seen(size))
            .and_then(|pane| self.seen(pane.cursor_y));
        input::paint(
            &mut self.screen,
            text,
            effects,
            view.as_ref(),
            size,
            seen.as_ref(),
        )
    }

    /// The write that leaves the cursor below the composer.
    pub(super) fn settle(&mut self) -> String {
        self.screen.settle()
    }

    fn pane(&self) -> Option<tmux::PaneSize> {
        let (server, pane) = self.server.as_ref().zip(self.me.as_deref())?;
        transport::observe_pane_size(server, pane)
    }

    fn seen(&self, cursor_y: usize) -> Option<Seen> {
        let (server, pane) = self.server.as_ref().zip(self.me.as_deref())?;
        let capture = transport::capture_screen(server, pane, tmux::Styling::Plain)?;
        Some(Seen { cursor_y, capture })
    }

    /// Whether this chat owns the input and no app writes: the lease is
    /// tried under the admission and dropped at once.
    fn owned(&self, console: &Console) -> Result<(), String> {
        let unread = || Err("tmux did not answer who owns the input".to_owned());
        self.owns(console).unwrap_or_else(unread)?;
        lease(console).map(drop)
    }

    /// Carry one ask through the helpers' own tracked path, as the console.
    fn ask(&mut self, console: &mut Console, raw: &[u8], seat: &str, body: String) -> Effect {
        // Refuse ownership before opening even this process's lifetime record.
        let record = still(&self.pair, console.seats())
            .and_then(|()| self.owned(console))
            .and_then(|()| self.recovery.record(&console.dir));
        let record = match record {
            Ok(record) => record,
            Err(why) => {
                self.input.put_back(raw);
                return Effect::Print(format!("refused: {why}"));
            }
        };
        let owns = || still(&self.pair, console.seats()).and_then(|()| self.owned(console));
        let (id, outcome) = match submit_ask(console, record, owns, raw, seat, body) {
            Ok(submitted) => submitted,
            Err(why) => {
                self.input.put_back(raw);
                return Effect::Print(format!("refused: {why}"));
            }
        };
        let line = input::outcome_line(&outcome, seat);
        if matches!(outcome, submit::Outcome::Unknown(..)) {
            return Effect::Print(line);
        }
        match outcome {
            submit::Outcome::Sent(_, kept) => console.printed.sent(&id, kept),
            _ => console.printed.outcome(&id, line),
        }
        match console.pass() {
            Ok(text) => console.printed.lane(text),
            Err(why) => {
                let gap = console
                    .printed
                    .style()
                    .dim(&format!("coverage incomplete: {why}\n"));
                let held = console.printed.flush_outcomes();
                console.printed.lane(format!("{gap}{held}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Effect, Got, Input, Reading, Reads, Screen, Seat, Term, ZeroRead, Zeros, pair_of, reader,
        still,
    };
    use crate::console::tests::{ID, Rig};
    use std::sync::mpsc::{RecvTimeoutError, sync_channel};
    use std::time::{Duration, Instant};

    #[test]
    fn only_a_second_of_nothing_but_fast_empty_reads_ends_the_input() {
        let (slow, quick) = (Duration::from_millis(100), Duration::ZERO);
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut zeros = Zeros::default();
        // The measured transient: one early empty read, then ticks again.
        assert_eq!(
            zeros.empty(Duration::from_millis(33), at(0)),
            ZeroRead::Wait
        );
        assert_eq!(zeros.empty(slow, at(150)), ZeroRead::Tick);
        // A hung-up terminal: empty at once, every read, for a second.
        assert_eq!(zeros.empty(quick, at(200)), ZeroRead::Wait);
        assert_eq!(zeros.empty(quick, at(1199)), ZeroRead::Wait);
        assert_eq!(zeros.empty(quick, at(1200)), ZeroRead::Ended);
    }

    #[test]
    fn a_park_between_two_fast_empty_reads_is_no_evidence_of_an_end() {
        let start = Instant::now();
        let mut zeros = Zeros::default();
        assert_eq!(zeros.empty(Duration::ZERO, start), ZeroRead::Wait);
        // The reader parked for a pane view that lasted two seconds.
        zeros.reset();
        let back = start + Duration::from_secs(2);
        assert_eq!(zeros.empty(Duration::ZERO, back), ZeroRead::Wait);
    }

    #[test]
    fn the_reader_learns_whether_it_parked() {
        let pause = super::Pause::default();
        assert!(!pause.checkpoint(), "not asked: no park");
        // Asked before the reader runs, so it parks whatever the scheduling.
        pause.state().asked = true;
        let parked = pause.clone();
        let reader = std::thread::spawn(move || parked.checkpoint());
        assert!(pause.hold(Duration::from_secs(5)), "the reader parks");
        pause.release();
        assert!(reader.join().unwrap(), "a park is reported");
    }

    #[test]
    fn an_ended_input_is_said_once_and_every_later_wait_sleeps_to_its_deadline() {
        let (send, reads) = sync_channel(1);
        let stamp = Instant::now();
        send.send((stamp, b"x".to_vec())).unwrap();
        drop(send);
        let mut reads = Reads(Some(reads));
        let far = stamp + Duration::from_mins(1);
        assert_eq!(reads.wait(far), Got::Read(stamp, b"x".to_vec()));
        assert_eq!(reads.wait(far), Got::Ended);
        let start = Instant::now();
        for _ in 0..3 {
            assert_eq!(reads.wait(start + Duration::from_millis(40)), Got::Due);
        }
        assert!(start.elapsed() >= Duration::from_millis(40), "no spin");
    }

    #[test]
    fn a_read_that_fails_ends_the_input_and_is_never_retried() {
        let dir = std::fs::File::options()
            .read(true)
            .open(std::env::temp_dir())
            .unwrap();
        let (send, reads) = sync_channel(1);
        let pause = super::Pause::default();
        std::thread::spawn(move || reader(dir, &send, &pause, super::EmptyRead::Ends));
        let ended = reads.recv_timeout(Duration::from_secs(5));
        assert_eq!(ended, Err(RecvTimeoutError::Disconnected));
    }

    #[test]
    fn one_main_seat_and_only_agent_names_make_a_pair_and_nothing_is_dropped() {
        let seat = |slot: &str, name: &str| Seat {
            slot: slot.to_owned(),
            name: name.to_owned(),
            profile: None,
        };
        let (lead, colead) = (seat("main", "lead"), seat("worker.0", "colead"));
        let pair = Ok(vec![lead.clone(), colead.clone()]);
        assert_eq!(pair_of(vec![colead.clone(), lead.clone()]), pair);
        let none = Err("the meta names no single main seat".to_owned());
        assert_eq!(pair_of(vec![colead.clone()]), none);
        assert_eq!(pair_of(vec![lead.clone(), seat("main", "b")]), none);
        let bad = Err("the worker.0 seat is not an agent name".to_owned());
        assert_eq!(pair_of(vec![lead, seat("worker.0", "%9")]), bad);
        for main in ["%3", "--cross-session", "le\x1bad", ""] {
            let bad = Err("the main seat is not an agent name".to_owned());
            assert_eq!(pair_of(vec![seat("main", main), colead.clone()]), bad);
        }
    }

    #[test]
    fn a_tick_that_cannot_prove_ownership_promotes_and_restores_nothing() {
        let rig = Rig::new("term-restore");
        let meta =
            format!("session_id={ID}\nlayout=lead-pair\nseat.main=lead\nseat.worker.0=colead\n");
        std::fs::write(rig.0.join("s/meta"), meta).unwrap();
        let console = rig.console();
        crate::store::open(&console.dir)
            .publish_console_draft(b"kept")
            .unwrap();
        let pair = console.seats().and_then(pair_of).unwrap();
        let input = Input::new(pair.iter().map(|seat| seat.name.clone()).collect());
        let mut term = Term {
            recovery: crate::console::submit::Recovery::default(),
            input,
            pair,
            reads: Reads(None),
            clipboard: None,
            server: None,
            me: None,
            screen: Screen::default(),
        };
        assert_eq!(term.tick(&console), [], "tmux unanswered is Unknown");
        assert_eq!(term.input.line(), None);
    }

    #[test]
    fn only_a_promotion_restores_the_kept_draft_and_each_promotion_restores_it_again() {
        let rig = Rig::new("term-promote");
        let meta =
            format!("session_id={ID}\nlayout=lead-pair\nseat.main=lead\nseat.worker.0=colead\n");
        std::fs::write(rig.0.join("s/meta"), meta).unwrap();
        let console = rig.console();
        let mut recovery = crate::console::submit::Recovery::default();
        recovery
            .record(&console.dir)
            .unwrap()
            .publish(b"kept")
            .unwrap();
        let pair = console.seats().and_then(pair_of).unwrap();
        let input = Input::new(pair.iter().map(|seat| seat.name.clone()).collect());
        let mut term = Term {
            recovery,
            input,
            pair,
            reads: Reads(None),
            clipboard: None,
            server: None,
            me: None,
            screen: Screen::default(),
        };
        let said =
            "Kept line, maybe already sent: check lead, colead panes (prefix H) before Enter";
        let banner = Effect::Print(said.to_owned());
        assert_eq!(
            term.take(&console, Reading::Owner),
            [Effect::Paste(true), banner.clone()]
        );
        assert_eq!(term.input.line().as_deref(), Some("to lead> kept"));
        assert_eq!(
            term.take(&console, Reading::Owner),
            [],
            "an owner stays one"
        );
        assert_eq!(
            term.input.line().as_deref(),
            Some("to lead> kept"),
            "restored once"
        );
        let _ = term.take(&console, Reading::NotOwner("owned elsewhere".to_owned()));
        let accepting = Effect::Print("accepting input".to_owned());
        let again = [Effect::Paste(true), accepting, banner];
        assert_eq!(term.take(&console, Reading::Owner), again);
        assert_eq!(term.input.line().as_deref(), Some("to lead> kept"));
    }

    /// A terminal-less `Term` over a rig's lead pair.
    fn bare_term(tag: &str) -> (Rig, crate::console::Console, Term) {
        let rig = Rig::new(tag);
        let meta =
            format!("session_id={ID}\nlayout=lead-pair\nseat.main=lead\nseat.worker.0=colead\n");
        std::fs::write(rig.0.join("s/meta"), meta).unwrap();
        let console = rig.console();
        let pair = console.seats().and_then(pair_of).unwrap();
        let input = Input::new(pair.iter().map(|seat| seat.name.clone()).collect());
        let (reads, screen) = (Reads(None), Screen::default());
        let term = Term {
            recovery: crate::console::submit::Recovery::default(),
            input,
            pair,
            reads,
            clipboard: None,
            server: None,
            me: None,
            screen,
        };
        (rig, console, term)
    }

    #[test]
    fn the_pane_is_read_only_while_a_composer_is_taken_or_on_screen() {
        let (_rig, console, mut term) = bare_term("term-live");
        let calls = std::cell::Cell::new(0);
        let read = |_: &Term| {
            calls.set(calls.get() + 1);
            None
        };
        let _ = term.painted("", &[], read);
        assert_eq!(calls.get(), 0, "nothing taken, nothing drawn");
        let _ = term.take(&console, Reading::Owner);
        let _ = term.painted("", &[], read);
        assert_eq!(calls.get(), 1, "taken, not yet drawn");
        let _ = term.take(&console, Reading::NotOwner("elsewhere".to_owned()));
        let _ = term.painted("", &[], read);
        assert_eq!(calls.get(), 2, "no longer taken, still on the screen");
        let _ = term.settle();
        let _ = term.painted("", &[], read);
        assert_eq!(calls.get(), 2, "settled and not taken");
    }

    #[test]
    fn needs_open_first_read_only_draft_reads_pane_size_before_drawing() {
        let (_rig, console, mut term) = bare_term("term-open-first-reader");
        let _ = term.take(&console, Reading::NotOwner("elsewhere".to_owned()));
        let effects = term.input.chunk(b"/open scout", Instant::now());
        assert!(effects.is_empty(), "unfinished draft performs no act");
        assert!(!term.input.taking(), "read-only chat owns no input");
        assert!(term.input.composing(), "read-only draft is visible");
        assert!(!term.screen.drawn(), "this is the first draft paint");
        let calls = std::cell::Cell::new(0);
        let drawn = term.painted("", &[], |_| {
            calls.set(calls.get() + 1);
            Some(crate::tmux::PaneSize {
                width: 12,
                height: 8,
                cursor_y: 0,
            })
        });
        assert_eq!(
            calls.get(),
            1,
            "first read-only draft reads actual pane dimensions"
        );
        assert!(
            drawn.contains("\r\n"),
            "first draft draws more than one row in the narrow pane: {drawn:?}"
        );
    }

    #[test]
    fn a_paint_with_no_tmux_answer_assumes_80_by_24_and_a_settle_ends_the_composer() {
        let (_rig, console, mut term) = bare_term("term-paint");
        assert_eq!(term.paint("", &[]), "", "nothing taken, nothing drawn");
        let _ = term.take(&console, Reading::Owner);
        let drawn = "\x1b[?7l\rto lead> \x1b7\x1b8\x1b[?7h";
        assert_eq!(term.paint("", &[]), drawn);
        assert_eq!(term.paint("x\n", &[]), format!("\r\x1b[Jx\n{drawn}"));
        assert_eq!(term.settle(), "\r\n");
        assert_eq!(term.settle(), "");
    }

    #[test]
    fn an_ask_after_the_meta_changed_its_lead_pair_is_refused_and_writes_nothing() {
        let (rig, head) = (
            Rig::new("term-pair"),
            format!("session_id={ID}\nlayout=lead-pair"),
        );
        let meta = |m: &str, c: &str| format!("{head}\nseat.main={m}\nseat.worker.0={c}\n");
        std::fs::write(rig.0.join("s/meta"), meta("lead", "colead")).unwrap();
        let mut console = rig.console();
        let (store, journal) = (crate::store::open(&console.dir), b"{\"ts\":\"x\"}\n");
        std::fs::write(rig.0.join("s/events.jsonl"), journal).unwrap();
        store.publish_console_draft(b"an earlier draft").unwrap();
        let pair = console.seats().and_then(pair_of).unwrap();
        let (input, reads, server, me) = (Input::default(), Reads(None), None, None);
        let screen = Screen::default();
        let mut term = Term {
            recovery: crate::console::submit::Recovery::default(),
            input,
            pair,
            reads,
            clipboard: None,
            server,
            me,
            screen,
        };
        let changed = "the lead pair changed since this chat opened - restart this chat";
        for (main, colead, why) in [
            ("lead", "other", changed),
            ("colead", "lead", changed),
            ("lead", "%3", "the worker.0 seat is not an agent name"),
            ("lead", "colead", "tmux did not answer who owns the input"),
        ] {
            std::fs::write(rig.0.join("s/meta"), meta(main, colead)).unwrap();
            let said = term.ask(&mut console, b"hi", "lead", "hi".to_owned());
            assert_eq!(
                said,
                Effect::Print(format!("refused: {why}")),
                "{main} {colead}"
            );
        }
        let (draft, kept) = (store.console_draft().unwrap(), b"an earlier draft".to_vec());
        assert_eq!(
            draft,
            crate::store::SourceRead::Ready(kept),
            "the draft is untouched"
        );
        assert_eq!(store.container(), journal.to_vec(), "no ask was recorded");
        let deadline = Instant::now() + Duration::from_millis(20);
        assert_eq!(
            term.wait(&mut console, deadline),
            None,
            "a quiet terminal says nothing"
        );
        assert!(Instant::now() >= deadline, "and says it at its deadline");
    }

    #[test]
    fn only_slots_and_names_make_it_the_same_lead_pair() {
        let seat = |slot: &str, name: &str, profile: Option<&str>| Seat {
            slot: slot.to_owned(),
            name: name.to_owned(),
            profile: profile.map(str::to_owned),
        };
        let opened = [
            seat("main", "lead", Some("fable5")),
            seat("worker.0", "colead", Some("sol61x")),
        ];
        let now = |seats: Vec<Seat>| still(&opened, Ok(seats));
        let reseated = vec![
            seat("main", "lead", Some("opus55x")),
            seat("worker.0", "colead", None),
        ];
        assert_eq!(now(reseated), Ok(()), "a profile is not routing identity");
        let changed =
            Err("the lead pair changed since this chat opened - restart this chat".to_owned());
        let (lead, colead) = (opened[0].clone(), opened[1].clone());
        let renamed_main = vec![
            seat("main", "other", lead.profile.as_deref()),
            colead.clone(),
        ];
        let renamed_worker = vec![
            lead.clone(),
            seat("worker.0", "other", colead.profile.as_deref()),
        ];
        let swapped = vec![
            seat("main", "colead", lead.profile.as_deref()),
            seat("worker.0", "lead", colead.profile.as_deref()),
        ];
        let added = vec![
            lead.clone(),
            colead.clone(),
            seat("worker.1", "third", None),
        ];
        for (what, seats) in [
            ("a renamed main seat", renamed_main),
            ("a renamed colead", renamed_worker),
            ("names swapped between slots", swapped),
            ("a seat removed", vec![lead.clone()]),
            ("a seat added", added),
        ] {
            assert_eq!(now(seats), changed, "{what}");
        }
    }

    #[test]
    fn an_ask_after_a_reseat_clears_the_pair_check_and_stops_only_at_ownership() {
        let (rig, head) = (
            Rig::new("term-reseat"),
            format!("session_id={ID}\nlayout=lead-pair"),
        );
        let meta = |m: &str, c: &str| {
            format!(
                "{head}\nseat.main=lead\nprofile.main={m}\nseat.worker.0=colead\nprofile.worker.0={c}\n"
            )
        };
        std::fs::write(rig.0.join("s/meta"), meta("fable5", "sol61x")).unwrap();
        let mut console = rig.console();
        let pair = console.seats().and_then(pair_of).unwrap();
        assert_eq!(
            pair[0].profile.as_deref(),
            Some("fable5"),
            "profile rows parse"
        );
        let mut term = Term {
            recovery: crate::console::submit::Recovery::default(),
            input: Input::default(),
            pair,
            reads: Reads(None),
            clipboard: None,
            server: None,
            me: None,
            screen: Screen::default(),
        };
        for (main, colead) in [
            ("opus55x", "sol61x"),
            ("fable5", "luna6m"),
            ("opus55x", "luna6m"),
        ] {
            std::fs::write(rig.0.join("s/meta"), meta(main, colead)).unwrap();
            let said = term.ask(&mut console, b"hi", "lead", "hi".to_owned());
            let stops = "refused: tmux did not answer who owns the input";
            assert_eq!(said, Effect::Print(stops.to_owned()), "{main} {colead}");
        }
    }

    #[test]
    fn a_fatal_follow_pass_prints_held_outcomes_before_exiting() {
        let (_rig, mut console, mut term) = bare_term("term-fatal-hold");
        console.printed.outcome("id", "sent id".to_owned());
        std::fs::write(
            console.dir.join("meta"),
            "session_id=0199c0de-bbbb-4890-abcd-ef0123456789\n",
        )
        .unwrap();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code =
            crate::console::pump(&mut console, Some(&mut term), true, &mut out, &mut err).unwrap();
        assert_eq!(code, crate::EXIT_UNAVAILABLE);
        assert_eq!(out, b"sent id\n", "fatal exit never drops a held outcome");
        assert!(
            String::from_utf8(err)
                .unwrap()
                .contains("no longer follows")
        );
    }

    /// appcue R2 frozen acceptance: the refusal names the recorded holder's place,
    /// else says it is unknown.
    use crate::console::Console;
    use crate::store::{SessionStore as Store, open};
    use std::fs::File;
    fn scratch() -> std::path::PathBuf {
        let dir = std::path::PathBuf::from(format!("/tmp/ae-cue-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
    fn refused(console: &Console) -> String {
        let Err(said) = super::lease(console) else {
            panic!("lease unheld")
        };
        said
    }
    #[test]
    fn refusal_names_the_holder_or_says_unknown_byte_exact() {
        let dir = scratch();
        let console = Console::bound("api".into(), dir.clone(), "uuid".into());
        let store = open(&dir);
        let mut held = store.console_writer().unwrap();
        let note = |held: &mut File, bytes: &[u8]| Store::console_writer_note(held, bytes).unwrap();
        let at = "an ae app is writing to api (held at alpha:2)";
        let unknown = "an ae app is writing to api (holder unknown)";
        note(&mut held, b"1 alpha 2\n");
        assert_eq!(refused(&console), at);
        note(&mut held, b"");
        assert_eq!(refused(&console), unknown);
        note(&mut held, &[b'x'; 161]);
        assert_eq!(refused(&console), unknown);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Frozen independent acceptance: tgchat-nav, OpenAI GPT-6.1-Sol.
    // Install byte-identically inside src/console/term.rs::tests.
    fn nav_external_rig(tag: &str) -> Rig {
        let rig = Rig::new(tag);
        std::fs::write(
                rig.0.join("s/meta"),
                format!("schema=2\nsession_id={ID}\nlayout=lead-pair\nseat.main=lead\nseat.worker.0=colead\ntmux_server_kind=socket\ntmux_server=/tmp/ae-nav-never-opened.sock\n"),
            ).unwrap();
        rig
    }

    #[test]
    fn nav_production_external_submit_uses_normal_colead_route_and_replays_recorded_update() {
        let rig = nav_external_rig("nav-external-co-route");
        let dir = rig.0.join("s");
        let pair = vec!["lead".to_owned(), "colead".to_owned()];
        let store = crate::store::open(&dir);
        let _interactive_writer = store.console_writer().unwrap();
        store.publish_console_draft(b"app draft remains").unwrap();
        let external = || super::External {
            dir: &dir,
            session: "s",
            session_id: ID,
            pair: &pair,
            seat: None,
            body: "phone question".to_owned(),
            minted: (crate::time::Timestamp::from_epoch(1_792_141_200), 77),
        };
        let standing = || super::Standing {
            takes: vec![crate::app::route::Take::Yes, crate::app::route::Take::Yes],
            quota: vec![Some(86.0), Some(60.0)],
        };
        let first = super::submit_external_with(external(), standing, |carry| {
                assert_eq!(carry.seat(), "colead", "external path bypassed normal pair chooser");
                assert_eq!(carry.body(), "(routed to you: lead quota 86%)\nphone question");
                assert_eq!(carry.id(), "ae-20261016T090000Z-0000004d");
                let id = carry.id();
                let recorded = format!(
                    r#"{{"ts":"2026-10-16T09:00:00Z","actor":"console:local","action":"ask","target":"colead","ref":"{id}","target_slot":"worker.0","target_session":"s","target_server":"/tmp/ae-nav-never-opened.sock","target_pane":"%2","target_session_uuid":"{ID}","summary":"phone question"}}"#,
                );
                store.append_event(&(recorded + "\n")).unwrap();
                String::new()
            }).unwrap();
        assert!(!first.replayed);
        assert_eq!(first.seat, "colead");
        assert!(matches!(
            first.outcome,
            crate::console::submit::Outcome::Sent(_, _)
        ));
        assert_eq!(
            store.console_draft().unwrap(),
            crate::store::SourceRead::Ready(b"app draft remains".to_vec())
        );
        let before = store.container();
        let replay = super::submit_external_with(external(), standing, |_| {
            panic!("checkpoint-loss replay carried a second ask")
        })
        .unwrap();
        assert!(
            replay.replayed,
            "recorded Telegram update not recognized after fresh Console construction"
        );
        assert_eq!(replay.id, first.id);
        assert_eq!(store.container(), before);
    }

    #[test]
    fn nav_production_external_submit_refuses_replaced_session_or_pair_before_carry() {
        let rig = nav_external_rig("nav-external-refused-identity");
        let dir = rig.0.join("s");
        let actual = vec!["lead".to_owned(), "colead".to_owned()];
        let replaced = vec!["lead".to_owned(), "other".to_owned()];
        for (session_id, pair) in [
            ("0199c0de-bbbb-4890-abcd-ef0123456789", &actual),
            (ID, &replaced),
        ] {
            let result = super::submit_external_with(
                super::External {
                    dir: &dir,
                    session: "s",
                    session_id,
                    pair,
                    seat: None,
                    body: "must not paste".to_owned(),
                    minted: (crate::time::Timestamp::from_epoch(1_792_141_200), 78),
                },
                || super::Standing {
                    takes: vec![crate::app::route::Take::Yes; 2],
                    quota: vec![None; 2],
                },
                |_| panic!("stale session identity reached final delivery"),
            );
            assert!(result.is_err(), "replaced session/pair was accepted");
        }
        assert!(crate::store::open(&dir).container().is_empty());
    }

    #[test]
    fn bang_shell_pure_popup_args_client_argv_and_work_dir_contract() {
        use crate::console::shell::popup_args;
        use crate::inventory::ServerId;
        use crate::meta::Selector;
        use std::path::PathBuf;

        let words = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            popup_args(&ServerId::Ambient, "/dev/ttys003", "/w", "git status"),
            Some(words(&[
                "display-popup",
                "-EE",
                "-c",
                "/dev/ttys003",
                "-d",
                "/w",
                "--",
                "git status",
            ]))
        );
        assert_eq!(
            popup_args(
                &ServerId::Selected(Selector::Name("ae-x".to_owned())),
                "/dev/pts/1",
                "/work/dir",
                "-x",
            ),
            Some(words(&[
                "-L",
                "ae-x",
                "display-popup",
                "-EE",
                "-c",
                "/dev/pts/1",
                "-d",
                "/work/dir",
                "--",
                "-x",
            ]))
        );
        assert_eq!(
            popup_args(
                &ServerId::Selected(Selector::Socket(PathBuf::from("/tmp/s.sock"))),
                "/dev/ttys001",
                "/w",
                r"find . -exec rm {} \;",
            ),
            Some(words(&[
                "-S",
                "/tmp/s.sock",
                "display-popup",
                "-EE",
                "-c",
                "/dev/ttys001",
                "-d",
                "/w",
                "--",
                "find . -exec rm {} \\; ",
            ]))
        );
        assert!(
            popup_args(&ServerId::Ambient, "/dev/ttys003", "/w", "printf 'a\tb'").is_some(),
            "tab is permitted in shell line"
        );
        for (client, dir, line) in [
            ("", "/w", "ls"),
            ("-bad", "/w", "ls"),
            ("/dev/tty\n1", "/w", "ls"),
            ("/dev/ttys003", "", "ls"),
            ("/dev/ttys003", "relative/dir", "ls"),
            ("/dev/ttys003", "/tmp/semi;", "ls"),
            ("/dev/ttys003", "/tmp/a\nb", "ls"),
            ("/dev/ttys003", "/tmp/a\tb", "ls"),
            ("/dev/ttys003", "/w", ""),
            ("/dev/ttys003", "/w", "ls\n"),
            ("/dev/ttys003", "/w", "a\rb"),
            ("/dev/ttys003", "/w", "a\x1bb"),
            ("/dev/ttys003", "/w", "a\0b"),
        ] {
            assert_eq!(
                popup_args(&ServerId::Ambient, client, dir, line),
                None,
                "must reject ({client:?}, {dir:?}, {line:?})"
            );
        }
    }

    #[test]
    fn bang_shell_pure_popup_client_shell_argv_and_work_dir_contract() {
        use crate::console::shell::{ShellArgv, popup_client, work_dir};
        use crate::tmux::ObservedClient;

        let client = |name: &str, pane: &str, activity: u64| ObservedClient {
            name: name.to_owned(),
            session: "s".to_owned(),
            pane: pane.to_owned(),
            activity: Some(activity),
        };
        assert_eq!(
            popup_client(&[], "%1"),
            Err("no tmux client is showing this pane".to_owned())
        );
        assert_eq!(
            popup_client(&[client("/dev/ttys001", "%2", 50)], "%1"),
            Err("no tmux client is showing this pane".to_owned())
        );
        assert_eq!(
            popup_client(
                &[
                    client("/dev/ttys001", "%1", 20),
                    client("/dev/ttys002", "%1", 20),
                ],
                "%1",
            ),
            Err("two clients showing this pane were active in the same second".to_owned())
        );
        assert_eq!(
            popup_client(
                &[
                    client("/dev/ttys001", "%1", 10),
                    client("/dev/ttys002", "%1", 25),
                    client("/dev/ttys003", "%2", 99),
                ],
                "%1",
            ),
            Ok("/dev/ttys002".to_owned())
        );

        assert!(ShellArgv::of("git status").is_some());
        assert!(ShellArgv::of("a\tb").is_some());
        for bad in ["", "a\nb", "a\rb", "a\x1bb", "a\0b"] {
            assert!(
                ShellArgv::of(bad).is_none(),
                "ShellArgv must reject {bad:?}"
            );
        }

        assert_eq!(
            work_dir(b"session_id=x\nwork_dir=/tmp/w\n"),
            Ok("/tmp/w".to_owned())
        );
        for missing in [&b"session_id=x\n"[..], &b"session_id=x\nwork_dir=\n"[..]] {
            assert_eq!(
                work_dir(missing),
                Err("the session records no work dir".to_owned())
            );
        }
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the test inspects its own fixture files"
    )]
    fn bang_term_shell_refuses_when_unowned_puts_line_back_and_never_touches_events_or_draft() {
        let (rig, mut console, mut term) = bare_term("term-bang-never-recorded");
        let store = crate::store::open(&console.dir);
        let journal = b"{\"ts\":\"x\"}\n";
        std::fs::write(rig.0.join("s/events.jsonl"), journal).unwrap();
        let _ = term.take(&console, Reading::Owner);
        store.publish_console_draft(b"earlier draft").unwrap();
        let (send, reads) = sync_channel(1);
        term.reads = Reads(Some(reads));
        send.send((Instant::now(), b"!echo secret\r".to_vec()))
            .unwrap();

        let got = term.wait(&mut console, Instant::now() + Duration::from_secs(1));
        assert_eq!(
            got,
            Some(vec![Effect::Print(
                "refused: ! echo secret: tmux did not answer who owns the input".to_owned()
            )])
        );
        assert_eq!(
            term.input.draft(),
            "!echo secret",
            "refused ! line put back"
        );
        assert_eq!(
            store.container(),
            journal.to_vec(),
            "events.jsonl untouched"
        );
        assert!(matches!(
            store.memo_source(),
            crate::store::SourceRead::Absent
        ));
        assert_eq!(
            store.console_draft().unwrap(),
            crate::store::SourceRead::Ready(b"earlier draft".to_vec()),
            "console.draft untouched"
        );
        assert!(
            !console.dir.join("messages").exists() && !rig.0.join("telegram").exists(),
            "no message body or Telegram outbox written"
        );
    }
}
