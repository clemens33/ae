//! The daemon: one thread long-polls Telegram, one pumps `say` outward.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Duration;

use super::inbound::{self, Accepted, Chat, Inbox, Origin, Policy, Refusal, Submit, Submitted};
use super::routing::{RunningSession, Seat, Target, World};
use super::{Api, Echoes, Outbound, Settings, Watch};
use crate::console::submit::Outcome;
use crate::inventory::{self, Roots, ServerId};
use crate::meta::{self, Meta};
use crate::transport;

/// The shortest a poll cycle may take when it did nothing and failed at
/// nothing.
const IDLE_FLOOR: Duration = Duration::from_millis(200);

/// How often the outbound pump re-scans for sessions and forwards what it
/// finds, when nothing else wakes it.
const OUTBOUND_INTERVAL: Duration = Duration::from_secs(2);

/// How far before its start a bridge still acts on a message and forwards a
/// chat row: anything older is kept where it is and never relayed.
const GRACE: Duration = Duration::from_mins(10);

/// The bridge's own record of what its cutoff stepped over: counts, never
/// text, never read back.
pub const BACKLOG_FILE: &str = "backlog.log";

/// The paths the bridge works between.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// `AE_HOME` — the sessions roots and the machine-global telegram state
    /// live under it.
    pub ae_home: PathBuf,
    /// The ae config holding the `[telegram]` section.
    pub config: PathBuf,
    /// The user's home, for a `~`-prefixed `token_file`.
    pub home: PathBuf,
}

impl Paths {
    /// The conventional layout: `<ae-home>/config`, and the home `AE_HOME` sits
    /// in.
    #[must_use]
    pub fn under(ae_home: impl Into<PathBuf>) -> Self {
        let ae_home = ae_home.into();
        let home = crate::instance::user_home(&ae_home).unwrap_or_else(|| ae_home.clone());
        Self {
            config: ae_home.join("config"),
            ae_home,
            home,
        }
    }

    /// The machine-global telegram state directory.
    #[must_use]
    pub fn state(&self) -> PathBuf {
        self.ae_home.join(inbound::STATE_DIR)
    }

    /// The session roots to scan.
    #[must_use]
    pub fn roots(&self) -> Roots {
        Roots::under(&self.ae_home)
    }
}

/// The daemon's tunables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Knobs {
    /// The inbound loop's.
    pub inbound: inbound::Knobs,
    /// How long the outbound pump waits between passes.
    pub outbound_interval: Duration,
    /// Run exactly one pass of each half and return.
    pub once: bool,
    /// How far before its start this bridge still relays; `None` relays
    /// everything.
    pub grace: Option<Duration>,
}

impl Default for Knobs {
    fn default() -> Self {
        Self {
            inbound: inbound::Knobs::default(),
            outbound_interval: OUTBOUND_INTERVAL,
            once: false,
            grace: Some(GRACE),
        }
    }
}

/// Something the inbound thread produced for the outbound thread to act on.
#[derive(Debug)]
enum Word {
    /// Send this to the operator's chat, best effort.
    Say(String),
    /// Send this and report back whether the platform ACCEPTED it.
    SayAndConfirm(String, Sender<Accepted>),
    /// Write this to the error stream.
    Note(String),
}

/// The [`Chat`] the inbound loop talks through: a channel, not a socket.
struct Outgoing(Sender<Word>);

impl Chat for Outgoing {
    fn say(&self, text: &str) {
        // A closed channel means the pump is gone and this process is on its
        // way out.
        let _ = self.0.send(Word::Say(text.to_owned()));
    }

    fn say_confirmed(&self, text: &str) -> Accepted {
        // EVERY FAILURE DIRECTION IS `No`, and that is the whole design.
        let (answer, reply) = std::sync::mpsc::channel();
        if self
            .0
            .send(Word::SayAndConfirm(text.to_owned(), answer))
            .is_err()
        {
            return Accepted::No;
        }
        reply.recv().unwrap_or(Accepted::No)
    }
}

/// The [`Submit`] the inbound loop routes through: the session's shared chat,
/// by the console's own admission, as the app's asks go.
#[derive(Debug, Clone)]
pub struct SharedChat {
    /// The state root the fleet and quota are read under.
    pub root: PathBuf,
    /// The asks carried, so the outbound pump does not echo them.
    pub echoes: Echoes,
}

impl Submit for SharedChat {
    fn submit(&self, target: &Target, text: &str, origin: &Origin) -> Submitted {
        // Remembered BEFORE the ask can reach the journal, so no pass of the
        // pump sees it first.
        super::remember_ask(&self.echoes, &target.session_id, origin.request_id());
        let ask = crate::console::term::External {
            dir: &target.dir,
            session: &target.session,
            session_id: &target.session_id,
            pair: &target.pair,
            seat: target.seat.as_deref(),
            body: text.to_owned(),
            minted: origin.minted(),
        };
        let Ok(carried) = crate::console::term::submit_external(&self.root, ask) else {
            return Submitted::No(Refusal::Transient);
        };
        let (id, seat) = (carried.id, carried.seat);
        if carried.replayed {
            return Submitted::Replayed { id, seat };
        }
        match carried.outcome {
            Outcome::Sent(..) => Submitted::Sent {
                id,
                seat,
                note: carried.note,
            },
            Outcome::Uncertain(_) | Outcome::Unknown(..) => Submitted::Unconfirmed { id, seat },
            Outcome::NotDelivered(_) => Submitted::No(Refusal::Transient),
        }
    }
}

/// The orchestrator role census: a state root in, the seat's session name out.
/// `None` when no session records the role, `Err` when the census cannot prove
/// one holder. The shell hands in `settings_menu::role_holder`, which lives
/// above this layer.
pub type Census = fn(&Path) -> Result<Option<String>, String>;

/// The census of a machine nobody wired one into: it proves nothing.
fn unwired(_root: &Path) -> Result<Option<String>, String> {
    Err("the orchestrator role census is not wired".to_owned())
}

/// The [`World`] routing resolves against: this machine's running sessions.
#[derive(Debug, Clone)]
pub struct Machine {
    roots: Roots,
    ae_home: PathBuf,
    census: Census,
}

impl Machine {
    /// Scan the sessions under `roots`. The orchestrator seat is unproven until
    /// [`Machine::with_census`] wires a census in.
    #[must_use]
    pub fn under(roots: Roots) -> Self {
        let ae_home = roots
            .sessions()
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        Self {
            roots,
            ae_home,
            census: unwired,
        }
    }

    /// The same machine, with `census` proving its orchestrator seat.
    #[must_use]
    pub fn with_census(self, census: Census) -> Self {
        Self { census, ..self }
    }
}

impl World for Machine {
    /// Every session that is durably recorded AND still on its recorded server
    /// .
    fn running(&self) -> Vec<RunningSession> {
        let scan = inventory::durable_records(&self.roots);
        scan.records
            .iter()
            .filter_map(|record| {
                let selector = record.server.entitles()?;
                let server = ServerId::Selected(selector.clone());
                if !transport::session_exists(&server, &record.name) {
                    return None;
                }
                let bytes = meta::read_bytes(&record.path).ok()?;
                let last_active = record
                    .snapshot
                    .events
                    .as_ref()
                    .and_then(|events| events.last_active)
                    .map(crate::time::Timestamp::epoch);
                Some(session_facts(
                    &record.name,
                    &record.path,
                    &bytes,
                    last_active,
                ))
            })
            .collect()
    }

    /// The seat the census proves, bound to its CANONICAL directory under the sessions root:
    /// the census reads that root only, and a worktree-nested record can carry the same name.
    fn seat(&self) -> Seat {
        match (self.census)(&self.ae_home) {
            Ok(Some(name)) => Seat::Held {
                dir: self.roots.sessions().join(&name),
                name,
            },
            Ok(None) => Seat::Vacant,
            Err(why) => Seat::Unproven(why),
        }
    }
}

/// Outbound discovery needs identities, not the journal-derived `/list` ages.
/// Liveness is shared per typed server within this pass, never across passes.
struct ForwardingMachine(Machine);

impl World for ForwardingMachine {
    fn running(&self) -> Vec<RunningSession> {
        // Use the same meta reader as Machine, including its symlink verdict.
        let scan = inventory::durable_records_with(&self.0.roots, &mut |dir| {
            crate::session::RecordSnapshot::read_meta_only(dir)
        });
        let mut servers = BTreeMap::new();
        scan.records
            .iter()
            .filter_map(|record| {
                let selector = record.server.entitles()?;
                let names = servers
                    .entry(selector.clone())
                    .or_insert_with(|| {
                        transport::session_names(&ServerId::Selected(selector.clone()))
                    })
                    .as_ref()?;
                if !names.contains(&record.name) {
                    return None;
                }
                let bytes = meta::read_bytes(&record.path).ok()?;
                Some(session_facts(&record.name, &record.path, &bytes, None))
            })
            .collect()
    }

    fn seat(&self) -> Seat {
        self.0.seat()
    }
}

/// One running session, assembled from the meta bytes that were just read.
fn session_facts(name: &str, dir: &Path, bytes: &[u8], last_active: Option<i64>) -> RunningSession {
    let meta = Meta::parse(&String::from_utf8_lossy(bytes));
    RunningSession {
        name: name.to_owned(),
        dir: dir.to_owned(),
        session_id: meta::first_value(bytes, "session_id")
            .map(|value| String::from_utf8_lossy(value).into_owned())
            .unwrap_or_default(),
        meta_agent: meta::meta_agent_role(bytes) == meta::MetaAgentRole::Role,
        main: meta
            .roster()
            .iter()
            .find(|entry| entry.slot == "main")
            .map(crate::meta::RosterEntry::reference),
        agents: meta
            .roster()
            .iter()
            .map(crate::meta::RosterEntry::reference)
            .collect(),
        pair: lead_pair(bytes, &meta),
        last_active,
    }
}

/// The lead pair's seat names, main first, as the shared chat asks them.
fn lead_pair(bytes: &[u8], meta: &Meta) -> Vec<String> {
    let lead_pair = crate::lifecycle::meta_value(bytes, "layout") == "lead-pair";
    let mut seats: Vec<_> = (meta.roster().iter())
        .filter(|entry| crate::watchdog_daemon::in_lead_pair(&entry.slot, lead_pair))
        .collect();
    seats.sort_by_key(|entry| entry.slot != "main");
    seats.iter().map(|entry| entry.name.clone()).collect()
}

/// Run the bridge until the process is stopped.
///
/// # Errors
///
/// Only writing the status stream. Every network, filesystem and delivery
/// failure degrades within its own cycle and is reported rather than fatal —
/// a bridge that exits because one poll failed is a bridge that stops relaying
/// the first time a laptop's wifi drops.
pub fn run(
    paths: &Paths,
    knobs: Knobs,
    census: Census,
    err: &mut impl Write,
) -> crate::error::Result<u8> {
    let settings = match super::load_settings(&paths.config, &paths.home) {
        Ok(settings) => settings,
        Err(why) => {
            // `CredentialsError` carries paths and reasons, never file CONTENT
            // and never the token — see its declaration.
            writeln!(err, "ae: {why}")?;
            return Ok(1);
        }
    };
    if let Err(why) = std::fs::create_dir_all(paths.state()) {
        writeln!(
            err,
            "ae: telegram: cannot create {}: {why}",
            paths.state().display()
        )?;
        return Ok(1);
    }

    let Settings {
        credentials,
        allowed_user_ids,
        outbound,
    } = settings;
    let policy = Policy::new(credentials.chat_id(), allowed_user_ids);
    let api = Arc::new(Api::production(credentials));
    let start = crate::time::Timestamp::now().epoch();
    let since = knobs.grace.map_or(i64::MIN, |grace| {
        start.saturating_sub(i64::try_from(grace.as_secs()).unwrap_or(i64::MAX))
    });
    let relay = Relay::new(since, paths.config.clone(), outbound, census);
    let (words, inbox_words) = std::sync::mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));

    if policy.enabled() {
        // The command menu, registered while THIS thread is still the only one
        // that exists.
        register_commands(&api, err)?;
    } else {
        // No allow-list, no inbound.
        writeln!(
            err,
            "ae: telegram: inbound is off ([telegram] allowed_user_ids is empty); \
             forwarding outbound only"
        )?;
    }

    let poller = if policy.enabled() {
        Some(spawn_inbound(
            Arc::clone(&api),
            policy,
            paths,
            knobs,
            &relay,
            words,
            Arc::clone(&stop),
        ))
    } else {
        // The sender is dropped so nothing holds a half-open channel, and the
        // pump is told there is no poller at all — see [`pump`] on why those
        // are not the same fact.
        drop(words);
        None
    };

    let listening = poller.is_some().then_some(&inbox_words);
    let outcome = pump(&api, paths, knobs, &relay, listening, &stop, err);
    stop.store(true, Ordering::Relaxed);
    // THE ORDER IS LOAD-BEARING: this drop must happen BEFORE the join, and
    // moving it below — or letting it fall to the end of the scope — hangs the
    // daemon on shutdown.
    drop(inbox_words);
    if let Some(poller) = poller {
        // A poller that is mid-long-poll takes up to its timeout to notice the
        // flag.
        let _ = poller.join();
    }
    outcome
}

/// What both halves share: the cutoff this bridge started with and the asks
/// it carried.
#[derive(Debug, Clone)]
struct Relay {
    since: i64,
    echoes: Echoes,
    /// The global config the passes read `relay_replies` from, afresh each pass.
    config: PathBuf,
    outbound: super::scope::Scope,
    census: Census,
}

impl Relay {
    fn new(since: i64, config: PathBuf, outbound: super::scope::Scope, census: Census) -> Self {
        Self {
            since,
            echoes: Echoes::default(),
            config,
            outbound,
            census,
        }
    }
}

/// Spawn the inbound long poll.
fn spawn_inbound(
    api: Arc<Api>,
    policy: Policy,
    paths: &Paths,
    knobs: Knobs,
    relay: &Relay,
    words: Sender<Word>,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    let inbound = inbound::Knobs {
        stale_before: relay.since,
        ..knobs.inbound
    };
    let mut inbox = Inbox::new(paths.state(), inbound).with_quota(
        paths.home.clone(),
        paths.config.clone(),
        paths.ae_home.clone(),
    );
    let machine = Machine::under(paths.roots()).with_census(relay.census);
    let chat_submit = SharedChat {
        root: paths.ae_home.clone(),
        echoes: Arc::clone(&relay.echoes),
    };
    std::thread::spawn(move || {
        let chat = Outgoing(words);
        loop {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            let cycle = inbox.poll(
                &api,
                &policy,
                &machine,
                &chat_submit,
                &chat,
                crate::time::Timestamp::now().epoch(),
            );
            if let Some(failure) = cycle.failure {
                let _ = chat.0.send(Word::Note(format!("ae: telegram: {failure}")));
            }
            if knobs.once {
                return;
            }
            // A successful poll needs no pause of its own: the long poll IS the
            // pause, and Telegram holds the connection open when it has
            // nothing.
            if cycle.retry_after.is_zero() {
                if cycle.routed == 0 && cycle.dropped == 0 {
                    sleep_until(&stop, IDLE_FLOOR);
                }
            } else {
                sleep_until(&stop, cycle.retry_after);
            }
        }
    })
}

/// The outbound pump, on the calling thread: forward `say`'s chat events, and
/// send whatever the inbound thread asked for.
fn pump(
    api: &Api,
    paths: &Paths,
    knobs: Knobs,
    relay: &Relay,
    words: Option<&Receiver<Word>>,
    stop: &AtomicBool,
    err: &mut impl Write,
) -> crate::error::Result<u8> {
    // Keyed by state directory, so each session's cursor and scan position
    // survive between passes.
    let mut bridges: BTreeMap<PathBuf, Outbound> = BTreeMap::new();
    let machine = ForwardingMachine(Machine::under(paths.roots()).with_census(relay.census));
    let mut board = super::desk::Machine::new(paths.state());
    let mut told = false;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(0);
        }
        if let Some(words) = words
            && matches!(drain(words, api, err)?, Drained::Closed)
            && !knobs.once
        {
            // The inbound thread is gone and cannot come back.
            return Ok(0);
        }
        let stale = forward(api, &machine, relay, &paths.state(), &mut bridges, err)?;
        board_pass(api, &machine, &mut board, err);
        if stale > 0 && !told {
            told = true;
            let line = format!(
                "ae bridge started: {stale} chat messages from before {} were not relayed; \
                 they stay in each session's chat.",
                crate::time::Timestamp::from_epoch(relay.since)
            );
            say(api, Word::Say(line), err)?;
        }
        if knobs.once {
            return Ok(0);
        }
        match words {
            // `recv_timeout` is the sleep AND the wake-up: a word that arrives
            // early is acted on early, and one that never arrives costs exactly
            // one pass interval.
            Some(words) => match words.recv_timeout(knobs.outbound_interval) {
                Ok(word) => say(api, word, err)?,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(0),
            },
            // Outbound only: there is no word to wait for, so the interval is
            // just an interval.
            None => sleep_until(stop, knobs.outbound_interval),
        }
    }
}

/// The slash commands the chat's `/` menu offers, and the whole of what
/// [`register_commands`] publishes.
const MENU: [(&str, &str); 5] = [
    ("list", "Running sessions and their agents"),
    ("quota", "Cached quota by account"),
    (
        "use",
        "Pin plain messages to one agent (/use clear to unpin)",
    ),
    (
        "session",
        "Send or ask a specific agent: /session <ref> send|ask <agent> <msg>",
    ),
    ("help", "What ae understands here"),
];

/// Publish [`MENU`], and CARRY ON WHATEVER HAPPENS.
fn register_commands(api: &Api, err: &mut impl Write) -> crate::error::Result<()> {
    if let Err(failure) = api.set_my_commands(&MENU) {
        writeln!(err, "ae: telegram: command menu not registered: {failure}")?;
    }
    Ok(())
}

/// Whether the word channel is still open.
enum Drained {
    /// Still connected.
    Open,
    /// The sender is gone.
    Closed,
}

/// Take every word waiting right now.
fn drain(words: &Receiver<Word>, api: &Api, err: &mut impl Write) -> crate::error::Result<Drained> {
    loop {
        match words.try_recv() {
            Ok(word) => say(api, word, err)?,
            Err(TryRecvError::Empty) => return Ok(Drained::Open),
            Err(TryRecvError::Disconnected) => return Ok(Drained::Closed),
        }
    }
}

/// Act on one word.
fn say(api: &Api, word: Word, err: &mut impl Write) -> crate::error::Result<()> {
    match word {
        Word::Note(text) => writeln!(err, "{text}")?,
        Word::Say(text) => {
            if let Err(failure) = api.send_message(&text) {
                writeln!(err, "ae: telegram: reply not sent: {failure}")?;
            }
        }
        Word::SayAndConfirm(text, answer) => {
            let accepted = match api.send_message(&text) {
                Ok(()) => Accepted::Yes,
                Err(failure) => {
                    writeln!(err, "ae: telegram: give-up notice not sent: {failure}")?;
                    Accepted::No
                }
            };
            // A dropped receiver means the inbound thread already stopped
            // waiting — it took an answer, or its reply channel disconnected
            // and its own `recv()` returned the fail-safe `No`.
            let _ = answer.send(accepted);
        }
    }
    Ok(())
}

/// One outbound pass across every RUNNING session on the machine: a stopped
/// session relays nothing, a new one joins on the next pass. Returns how many
/// records the cutoff stepped over, each step also kept in [`BACKLOG_FILE`].
fn forward(
    api: &Api,
    world: &dyn World,
    relay: &Relay,
    state: &Path,
    bridges: &mut BTreeMap<PathBuf, Outbound>,
    err: &mut impl Write,
) -> crate::error::Result<usize> {
    // The raw relay answer echo is the orchestrator's alone: read the setting
    // once per pass, so a changed config is honoured without a restart.
    let replies = crate::config::global_relay_replies(Some(&relay.config));
    let running = world.running();
    let admitted = relay.outbound.admitted(world, &running);
    // A session that stopped keeps its bridge, unpumped: what it sent stays
    // remembered when a resume replaces its ledger with a new inode.
    let mut held_back = 0;
    // A session with no lead pair has no shared chat to relay.
    for session in running.iter().filter(|session| !session.pair.is_empty()) {
        let outbound = bridges.entry(session.dir.clone()).or_insert_with(|| {
            let watch = Watch {
                pair: session.pair.clone(),
                since: relay.since,
                echoes: Arc::clone(&relay.echoes),
            };
            Outbound::watching(&session.dir, session.name.clone(), watch)
        });
        // The pair as this pass reads it: a reseat or a resume may change it.
        outbound.watch.pair.clone_from(&session.pair);
        outbound.session_id.clone_from(&session.session_id);
        outbound.session_id.make_ascii_lowercase();
        outbound.replies_only = admitted
            .as_ref()
            .is_some_and(|dirs| !dirs.contains(&session.dir));
        outbound.relay_replies = if session.meta_agent {
            replies
        } else {
            crate::config::RelayReplies::Shown
        };
        let pass = outbound.pump(api);
        if let Some(failure) = pass.failure {
            writeln!(err, "ae: telegram: {}: {failure}", session.name)?;
        }
        if pass.stale > 0 {
            held_back += pass.stale;
            let line = format!(
                "{} {} stale={} before={}\n",
                crate::time::Timestamp::now(),
                session.name,
                pass.stale,
                crate::time::Timestamp::from_epoch(relay.since)
            );
            if let Err(why) = super::append_private(&state.join(BACKLOG_FILE), &line) {
                writeln!(err, "ae: telegram: {BACKLOG_FILE}: {why}")?;
            }
        }
    }
    Ok(held_back)
}

/// One pass of the pinned board: the board of the orchestrator seat, and only
/// that. The seat is the one the role census proves, and it counts as present
/// only while a running session with the seat's canonical directory and the
/// orchestrator role stands: a living namesake elsewhere is no source. The
/// outbound scope does not apply, because the board is not a chat event.
fn board_pass(
    api: &Api,
    world: &dyn World,
    board: &mut super::desk::Machine,
    err: &mut impl Write,
) {
    let Seat::Held { dir, name } = world.seat() else {
        board.namesake_over();
        return;
    };
    let running = world.running();
    if !running
        .iter()
        .any(|session| session.name == name && session.dir == dir && session.meta_agent)
    {
        // A living namesake elsewhere is no seat, and fails closed: say so
        // once per episode, so a spelling difference does not read as silence.
        match running
            .iter()
            .find(|session| session.name == name && session.dir != dir)
        {
            Some(other) => board.namesake(&name, &other.dir, &dir, err),
            None => board.namesake_over(),
        }
        return;
    }
    board.namesake_over();
    let now = crate::time::Timestamp::now().epoch();
    match crate::deskstore::read(&dir) {
        // No board is still a pass: it reads the checkpoint, so a hold is told.
        Ok(found) => {
            let _ = board.pass(api, api.chat(), found.as_ref(), now, err);
        }
        Err(why) => board.unreadable(api.chat(), &why.to_string(), err),
    }
}

/// Sleep, in slices, so a stop flag is noticed sooner than the whole delay.
fn sleep_until(stop: &AtomicBool, total: Duration) {
    const SLICE: Duration = Duration::from_millis(200);
    let mut left = total;
    while !left.is_zero() {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let slice = left.min(SLICE);
        std::thread::sleep(slice);
        left -= slice;
    }
}

#[cfg(test)]
#[path = "../../../tests/it/bridgecpu_spec.rs"]
mod bridgecpu_spec;

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        reason = "fixtures build and inspect real files; the boundary is about what PRODUCT \
                  code may reach"
    )]

    use super::{Accepted, Knobs, Machine, Paths, Refusal, Roots, RunningSession, Seat, Word};
    use crate::telegram::inbound::Chat as _;
    use crate::telegram::routing::World as _;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    /// A bridge relay with no cutoff and no carried asks.
    fn relay() -> super::Relay {
        super::Relay::new(
            i64::MIN,
            PathBuf::new(),
            crate::telegram::scope::Scope::default(),
            super::unwired,
        )
    }

    fn relay_from_config(config: &std::path::Path, home: &std::path::Path) -> super::Relay {
        let settings = crate::telegram::load_settings(config, home).unwrap();
        super::Relay::new(
            i64::MIN,
            config.to_owned(),
            settings.outbound,
            super::unwired,
        )
    }

    fn carry_for(relay: &super::Relay, session_id: &str, id: &str) {
        crate::telegram::remember_ask(&relay.echoes, session_id, id.to_owned());
    }

    #[path = "tgorchonly_spec.rs"]
    mod tgorchonly_spec;

    #[path = "desk_bridge_spec.rs"]
    mod desk_bridge_spec;

    #[path = "desk_namesake_tests.rs"]
    mod desk_namesake_tests;

    #[test]
    fn the_conventional_layout_derives_the_config_and_the_home_from_ae_home() {
        let paths = Paths::under("/home/someone/.ae");
        assert_eq!(paths.config, PathBuf::from("/home/someone/.ae/config"));
        assert_eq!(paths.home, PathBuf::from("/home/someone"));
        assert_eq!(paths.state(), PathBuf::from("/home/someone/.ae/telegram"));
    }

    #[test]
    fn a_root_ae_home_still_has_a_home_rather_than_none() {
        // `Path::parent` of "/" is None, and a daemon that cannot name a home
        // cannot expand a `~/token` path.
        let paths = Paths::under("/");
        assert_eq!(paths.home, PathBuf::from("/"));
    }

    #[test]
    fn a_shared_chat_submit_the_console_cannot_bind_is_transient_and_never_an_ack() {
        // THE FAIL-SAFE DIRECTION: a session the console cannot open is owed
        // and retried, never acknowledged; the id is still remembered so a
        // later carry cannot echo.
        use crate::telegram::inbound::{Origin, Submit as _, Submitted};
        let root = std::env::temp_dir().join(format!("ae-tg-shared-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let chat = super::SharedChat {
            root: root.clone(),
            echoes: super::Echoes::default(),
        };
        let target = crate::telegram::routing::Target {
            session: "nowhere".to_owned(),
            dir: PathBuf::from("/definitely/not/a/session"),
            session_id: "abc123".to_owned(),
            pair: vec!["lead".to_owned()],
            seat: None,
        };
        let origin = Origin {
            update_id: 77,
            date: 1_792_141_200,
            from_id: "42".to_owned(),
        };
        assert_eq!(
            chat.submit(&target, "hello", &origin),
            Submitted::No(Refusal::Transient)
        );
        assert_eq!(
            crate::telegram::carried_ask(&chat.echoes, &target.session_id, &origin.request_id()),
            Some(true)
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn forward_keeps_a_raw_relay_answer_off_the_phone_of_an_orchestrator_by_default() {
        // Production's gate, through `forward` itself: the orchestrator's
        // journal carries the receipt echo, the raw answer echo and another
        // ae row; the phone gets the receipt and the other ae row, not the
        // answer. An ordinary session's journal with the same rows sends all
        // three, so the hiding is the orchestrator's alone.
        use crate::telegram::routing::RunningSession;
        use crate::telegram::tests::{Fake, Reply};
        let root = std::env::temp_dir().join(format!("ae-tg-quiet-{}", std::process::id()));
        // Disposable: a recycled pid must not pick up an earlier run's journal or cursor.
        std::fs::remove_dir_all(&root).ok();
        for name in ["orch", "home"] {
            std::fs::create_dir_all(root.join(name)).unwrap();
            let rows = [
                ("ae:relay", "routed to home: ask (sent), request r-1"),
                ("ae:relay", "reply fd5bf0 from lead: RAW-RELAY-ANSWER"),
                ("ae:compact", "reboot: handover complete"),
            ];
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(root.join(name).join(crate::store::EVENTS))
                .unwrap();
            for (actor, text) in rows {
                let line = format!(
                    r#"{{"ts":"2026-10-16T09:00:00Z","actor":"{actor}","action":"chat","ref":"ae-20261016T090000Z-0000abcd","summary":"{text}"}}"#
                );
                std::io::Write::write_all(&mut file, format!("{line}\n").as_bytes()).unwrap();
            }
        }
        let session = |name: &str, meta_agent: bool| RunningSession {
            name: name.to_owned(),
            dir: root.join(name),
            session_id: "abc123".to_owned(),
            meta_agent,
            main: Some("lead".to_owned()),
            agents: vec!["lead".to_owned()],
            pair: vec!["lead".to_owned()],
            last_active: None,
        };
        let fake = Fake::start(vec![Reply::ok()]);
        let api = fake.api();
        let mut bridges = std::collections::BTreeMap::new();
        let mut err = Vec::new();
        let world = vec![session("orch", true), session("home", false)];
        super::forward(&api, &world, &relay(), &root, &mut bridges, &mut err).unwrap();
        let texts: Vec<String> = (fake.requests().iter())
            .map(|seen| crate::json::parse(&seen.body).unwrap())
            .map(|value| value.get_str("text").unwrap_or_default().to_owned())
            .collect();
        let of = |label: &str, needle: &str| {
            texts
                .iter()
                .filter(|text| text.starts_with(label) && text.contains(needle))
                .count()
        };
        assert_eq!(of("[orch] ", "routed to home"), 1, "{texts:?}");
        assert_eq!(of("[orch] ", "reboot: handover"), 1, "{texts:?}");
        assert_eq!(of("[orch] ", "RAW-RELAY-ANSWER"), 0, "{texts:?}");
        for needle in ["routed to home", "reboot: handover", "RAW-RELAY-ANSWER"] {
            assert_eq!(of("[home] ", needle), 1, "an ordinary session: {texts:?}");
        }
        assert_eq!(texts.len(), 5, "{texts:?}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn forward_follows_the_global_relay_replies_setting_each_pass_and_replays_nothing() {
        // The setting is read at every pass from the global config: `on`
        // forwards an orchestrator's raw answer echo as before the key
        // existed, anything else keeps it off the phone. A line skipped while
        // hidden is past the cursor, so turning the setting on later does not
        // send it; a line sent while on stays sent when the setting turns off.
        use crate::telegram::routing::RunningSession;
        use crate::telegram::tests::{Fake, Reply};
        let root = std::env::temp_dir().join(format!("ae-tg-quiet-on-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(root.join("orch")).unwrap();
        let config = root.join("config");
        let write_config = |value: Option<&str>| {
            let text = value.map_or_else(String::new, |value| {
                format!("[workspace]\nrelay_replies = {value}\n")
            });
            std::fs::write(&config, text).unwrap();
        };
        let say = |actor: &str, text: &str| {
            let line = format!(
                r#"{{"ts":"2026-10-16T09:00:00Z","actor":"{actor}","action":"chat","ref":"ae-20261016T090000Z-0000abcd","summary":"{text}"}}"#
            );
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(root.join("orch").join(crate::store::EVENTS))
                .unwrap();
            std::io::Write::write_all(&mut file, format!("{line}\n").as_bytes()).unwrap();
        };
        let world = vec![RunningSession {
            name: "orch".to_owned(),
            dir: root.join("orch"),
            session_id: "abc123".to_owned(),
            meta_agent: true,
            main: Some("lead".to_owned()),
            agents: vec!["lead".to_owned()],
            pair: vec!["lead".to_owned()],
            last_active: None,
        }];
        let fake = Fake::start(vec![Reply::ok()]);
        let api = fake.api();
        let relay = super::Relay {
            config: config.clone(),
            ..relay()
        };
        let mut bridges = std::collections::BTreeMap::new();
        let mut err = Vec::new();
        let mut pass =
            |bridges: &mut std::collections::BTreeMap<PathBuf, crate::telegram::Outbound>| {
                super::forward(&api, &world, &relay, &root, bridges, &mut err).unwrap();
            };
        let texts = || -> Vec<String> {
            (fake.requests().iter())
                .map(|seen| crate::json::parse(&seen.body).unwrap())
                .map(|value| value.get_str("text").unwrap_or_default().to_owned())
                .collect()
        };
        let count = |needle: &str| texts().iter().filter(|t| t.contains(needle)).count();

        write_config(None);
        say("ae:relay", "reply fd5bf0 from lead: HIDDEN-ECHO");
        say("ae:relay", "routed to home: ask (sent), request r-1");
        pass(&mut bridges);
        assert_eq!((count("HIDDEN-ECHO"), count("routed to home")), (0, 1));

        write_config(Some("on"));
        pass(&mut bridges);
        assert_eq!(count("HIDDEN-ECHO"), 0, "a skipped line is not replayed");
        say("ae:relay", "reply 0a1b2c from lead: SHOWN-ECHO");
        say("ae:compact", "reboot: handover complete");
        pass(&mut bridges);
        assert_eq!((count("SHOWN-ECHO"), count("reboot: handover")), (1, 1));
        assert_eq!(count("routed to home"), 1, "nothing sent twice");

        write_config(Some("off"));
        say("ae:relay", "reply 0d0d0d from lead: OFF-AGAIN-ECHO");
        say("ae:compact", "reboot: second");
        pass(&mut bridges);
        assert_eq!((count("OFF-AGAIN-ECHO"), count("reboot: second")), (0, 1));
        assert_eq!(count("SHOWN-ECHO"), 1, "a sent line stays sent");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn forward_relays_only_running_sessions_with_a_lead_pair_and_follows_the_pair() {
        // Production's gate, through `forward` itself: every running session
        // with a lead pair relays under its own label, one that starts later
        // joins on the next pass; a live session whose roster names no lead
        // pair has no shared chat, so a worker's `say` there never reaches the
        // phone; the pair is re-read each pass; and a session that stopped
        // relays nothing until it resumes. A stop plus journal retention must
        // preserve accepted-line memory: retained accepted rows stay silent,
        // while fresh rows written during the stop and after resume still send.
        use crate::telegram::routing::RunningSession;
        use crate::telegram::tests::{Fake, Reply};
        let root = std::env::temp_dir().join(format!("ae-tg-forward-{}", std::process::id()));
        for name in ["work", "home", "solo"] {
            std::fs::create_dir_all(root.join(name)).unwrap();
        }
        let say = |session: &str, actor: &str, text: &str| {
            let line = format!(
                r#"{{"ts":"2026-10-16T09:00:00Z","actor":"{actor}","action":"chat","summary":"{text}"}}"#
            );
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(root.join(session).join(crate::store::EVENTS))
                .unwrap();
            std::io::Write::write_all(&mut file, format!("{line}\n").as_bytes()).unwrap();
        };
        let session = |name: &str, pair: &[&str]| RunningSession {
            name: name.to_owned(),
            dir: root.join(name),
            session_id: "abc123".to_owned(),
            meta_agent: false,
            main: Some("lead".to_owned()),
            agents: vec!["lead".to_owned(), "dev".to_owned()],
            pair: pair.iter().map(|&seat| seat.to_owned()).collect(),
            last_active: None,
        };
        let fake = Fake::start(vec![Reply::ok()]);
        let api = fake.api();
        let mut bridges = std::collections::BTreeMap::new();
        let mut err = Vec::new();
        let mut pass = |world: Vec<RunningSession>,
                        bridges: &mut std::collections::BTreeMap<
            PathBuf,
            crate::telegram::Outbound,
        >| {
            super::forward(&api, &world, &relay(), &root, bridges, &mut err).unwrap()
        };
        let sent = || {
            let mut texts: Vec<String> = (fake.requests().iter())
                .map(|seen| crate::json::parse(&seen.body).unwrap())
                .map(|value| value.get_str("text").unwrap_or_default().to_owned())
                .collect();
            texts.sort();
            texts
        };

        say("work", "dev", "WORKER_SAY");
        say("solo", "dev", "SOLO_WORKER_SAY");
        pass(
            vec![session("work", &["colead"]), session("solo", &[])],
            &mut bridges,
        );
        assert!(sent().is_empty(), "{:?}", sent());
        assert_eq!(
            bridges.len(),
            1,
            "a session with no lead pair is not watched"
        );

        say("work", "lead", "WORK_LEAD_SAY");
        say("home", "lead", "HOME_LEAD_SAY");
        let world = vec![
            session("work", &["lead"]),
            session("home", &["lead"]),
            session("solo", &[]),
        ];
        pass(world, &mut bridges);
        let texts = sent();
        assert_eq!(texts.len(), 2, "{texts:?}");
        assert!(texts[0].starts_with("[home] ") && texts[0].contains("HOME_LEAD_SAY"));
        assert!(texts[1].starts_with("[work] ") && texts[1].contains("WORK_LEAD_SAY"));

        pass(vec![session("home", &["lead"])], &mut bridges);
        say("work", "lead", "AFTER_STOP");
        pass(vec![session("home", &["lead"])], &mut bridges);
        assert_eq!(sent().len(), 2, "a stopped session relayed: {:?}", sent());
        // Retention replaces the inode while work is stopped. Keep the old
        // file as evidence and write its bytes to a new file at the log path.
        let log = root.join("work").join(crate::store::EVENTS);
        let retained = std::fs::read(&log).unwrap();
        std::fs::rename(&log, root.join("work").join("retained-evidence.jsonl")).unwrap();
        std::fs::write(&log, retained).unwrap();
        say("work", "lead", "FRESH_AFTER_RESUME");
        pass(
            vec![session("home", &["lead"]), session("work", &["lead"])],
            &mut bridges,
        );
        assert_eq!(
            sent(),
            vec![
                "[home] lead\nHOME_LEAD_SAY".to_owned(),
                "[work] lead\nAFTER_STOP".to_owned(),
                "[work] lead\nFRESH_AFTER_RESUME".to_owned(),
                "[work] lead\nWORK_LEAD_SAY".to_owned(),
            ],
            "stop/resume with a new inode must retain accepted rows without losing fresh ones"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_sessions_facts_come_off_its_meta_including_the_two_keys_meta_does_not_keep() {
        // `session_id` is not a field of `Meta`, and the role reducer reads its
        // exact claim from the same raw bytes.
        let meta = concat!(
            "session_id=abc123def456\n",
            "meta_agent=true\n",
            "seat.main=lead\nprofile.main=claude\nharness_session.main=e795c9e9\n",
            "seat.worker.0=coworker\nprofile.worker.0=codex\n",
        );
        let facts = super::session_facts(
            "orchestrator",
            std::path::Path::new("/sessions/orchestrator"),
            meta.as_bytes(),
            Some(1_800_000_000),
        );
        assert_eq!(facts.session_id, "abc123def456");
        assert!(facts.meta_agent);
        assert_eq!(facts.main.as_deref(), Some("lead"));
        assert_eq!(facts.agents, vec!["lead", "coworker"]);
        assert_eq!(facts.last_active, Some(1_800_000_000));

        // The reducer grants this role only to one byte-exact valued claim;
        // every other spelling is an ordinary session. A bare record makes the
        // claim damaged even beside a valid value in either order.
        let ordinary = [
            "",
            "meta_agent\n",
            "meta_agent\nmeta_agent=true\n",
            "meta_agent=true\nmeta_agent\n",
            "meta_agent=\n",
            "meta_agent=false\n",
            "meta_agent=truth\n",
            "meta_agent=TRUE\n",
            "meta_agent=1\n",
            "meta_agent=yes\n",
            "meta_agent=true\r\n",
            "meta_agent=true\nmeta_agent=true\n",
            "meta_agent=true\nmeta_agent=false\n",
            "meta_agent=false\nmeta_agent=true\n",
        ];
        let observed: Vec<(&str, bool)> = ordinary
            .iter()
            .map(|meta| {
                (
                    *meta,
                    super::session_facts(
                        "work",
                        std::path::Path::new("/sessions/work"),
                        meta.as_bytes(),
                        None,
                    )
                    .meta_agent,
                )
            })
            .collect();
        let expected: Vec<(&str, bool)> = ordinary.iter().map(|meta| (*meta, false)).collect();
        assert_eq!(observed, expected, "meta_agent authority observations");
        for meta in ordinary {
            let facts = super::session_facts(
                "work",
                std::path::Path::new("/sessions/work"),
                meta.as_bytes(),
                None,
            );
            assert_eq!(facts.session_id, "");
            assert_eq!(facts.main, None);
        }
    }

    #[test]
    fn a_sliced_sleep_actually_sleeps_when_nothing_stops_it() {
        // The companion to the early-return test below: without this, a
        // `sleep_until` that returned immediately — or never advanced its
        // remaining time — would look correct to the only test that watched it.
        let stop = std::sync::atomic::AtomicBool::new(false);
        let at = std::time::Instant::now();
        super::sleep_until(&stop, Duration::from_millis(250));
        assert!(
            at.elapsed() >= Duration::from_millis(250),
            "the sleep returned after {:?}",
            at.elapsed()
        );
    }

    #[test]
    fn the_command_menu_is_registered_on_startup_and_a_refusal_is_logged_and_ignored() {
        // BEST EFFORT, and "ignored" is the load-bearing half: the menu is
        // cosmetic — every command works typed out — so a daemon that refused
        // to start because Telegram would not take the menu would trade the
        // whole bridge for a nicety. Both directions are asserted, because
        // "attempted" and "survivable" are different claims.
        use crate::telegram::tests::{Fake, Reply};

        let fake = Fake::one(Reply::json(400, r#"{"ok":false,"description":"nope"}"#));
        let api = fake.api();
        let mut err = Vec::new();
        super::register_commands(&api, &mut err).expect("a refusal must not fail the daemon");

        let seen = fake.requests();
        assert_eq!(seen.len(), 1, "registration must be ATTEMPTED: {seen:?}");
        assert!(
            seen[0].path.contains("setMyCommands"),
            "the startup call must be setMyCommands: {:?}",
            seen[0].path
        );
        // The five commands the router implements, and no sixth: a menu entry
        // the router does not implement is a promise the chat cannot keep.
        for command in ["list", "use", "session", "quota", "help"] {
            assert!(
                seen[0].body.contains(&format!(r#""command":"{command}""#)),
                "{command} missing from the registered menu: {}",
                seen[0].body
            );
        }
        assert_eq!(super::MENU.len(), 5);

        let logged = String::from_utf8(err).unwrap();
        assert!(
            logged.contains("command menu not registered"),
            "a refusal must be LOGGED, not swallowed: {logged:?}"
        );
        assert!(
            !logged.contains(crate::telegram::tests::FAKE_TOKEN),
            "the token reached the error stream"
        );
    }

    #[test]
    fn the_confirmed_word_accepts_only_what_telegram_actually_took() {
        // THE ACCEPTANCE PREDICATE, on the real client against a real socket.
        use crate::telegram::tests::{Fake, Reply};

        for (label, reply, want) in [
            (
                "2xx and ok:true",
                Reply::json(200, r#"{"ok":true,"result":{}}"#),
                Accepted::Yes,
            ),
            ("non-2xx", Reply::json(500, r#"{"ok":false}"#), Accepted::No),
            (
                "200 but ok:false",
                Reply::json(200, r#"{"ok":false,"description":"nope"}"#),
                Accepted::No,
            ),
        ] {
            let fake = Fake::one(reply);
            let api = fake.api();
            let (answer, reply_rx) = std::sync::mpsc::channel();
            let mut err = Vec::new();
            super::say(
                &api,
                Word::SayAndConfirm("handing this back".to_owned(), answer),
                &mut err,
            )
            .unwrap();
            assert_eq!(
                reply_rx.recv().unwrap(),
                want,
                "{label}: the give-up gate read the platform wrong"
            );
            // The token rides in the URL path, so a failure line is the one
            // place it could leak.
            let logged = String::from_utf8(err).unwrap();
            assert!(
                !logged.contains(crate::telegram::tests::FAKE_TOKEN),
                "{label}: the token reached the error stream"
            );
        }
    }

    #[test]
    fn a_rejected_give_up_notice_still_answers_rather_than_hanging_the_inbound_thread() {
        // The inbound thread blocks on this answer with NO timeout, so a path
        // that logged the failure and returned without replying would wedge it
        // until shutdown.
        use crate::telegram::tests::{Fake, Reply};

        let fake = Fake::one(Reply::json(503, r#"{"ok":false}"#));
        let api = fake.api();
        let (answer, reply_rx) = std::sync::mpsc::channel();
        let mut err = Vec::new();
        super::say(&api, Word::SayAndConfirm("x".to_owned(), answer), &mut err).unwrap();
        assert_eq!(
            reply_rx.recv(),
            Ok(Accepted::No),
            "a rejection must arrive as an ANSWER, never as a dropped channel"
        );
    }

    #[test]
    fn a_slow_pump_yields_exactly_one_surfaced_notice_and_one_checkpoint() {
        // THE BUG THE BOUNDED WAIT HAD, as a test.
        use crate::telegram::inbound::Chat as _;
        use crate::telegram::tests::{Fake, Reply};

        for (label, reply, want, surfaced) in [
            (
                "accepted",
                Reply::json(200, r#"{"ok":true}"#),
                Accepted::Yes,
                1,
            ),
            (
                "rejected",
                Reply::json(500, r#"{"ok":false}"#),
                Accepted::No,
                1,
            ),
        ] {
            let fake = Fake::start(vec![reply]);
            let api = fake.api();
            let (tx, rx) = std::sync::mpsc::channel::<Word>();

            // A pump that is BUSY first and drains second — the shape that used
            // to accumulate.
            let slow = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                let mut err = Vec::new();
                let mut served = 0;
                while let Ok(word) = rx.try_recv() {
                    super::say(&api, word, &mut err).unwrap();
                    served += 1;
                }
                // Nothing arrives after the first, because the caller is
                // blocked until this answers it.
                served
            });

            let outgoing = super::Outgoing(tx);
            let answered = outgoing.say_confirmed("handing this back");
            drop(outgoing);
            let served = slow.join().unwrap();

            assert_eq!(answered, want, "{label}: wrong acceptance");
            assert_eq!(served, 1, "{label}: the pump saw more than one word");
            let posts = fake
                .requests()
                .iter()
                .filter(|seen| seen.path.contains("sendMessage"))
                .count();
            assert_eq!(
                posts, surfaced,
                "{label}: EXACTLY ONE notice may reach the operator, whatever the pump's pace"
            );
        }
    }

    #[test]
    fn a_confirmation_that_never_comes_back_is_a_rejection() {
        // THE FAIL-SAFE DEFAULT, tested where it actually lives.
        use crate::telegram::inbound::Chat as _;

        // 1. The pump is already gone: the word cannot even be queued.
        let (tx, rx) = std::sync::mpsc::channel::<Word>();
        drop(rx);
        assert_eq!(
            super::Outgoing(tx).say_confirmed("nobody is listening"),
            Accepted::No,
            "a word that could not be queued was never accepted by anyone"
        );

        // 2.
        let (tx, rx) = std::sync::mpsc::channel::<Word>();
        let pump = std::thread::spawn(move || drop(rx.recv()));
        assert_eq!(
            super::Outgoing(tx).say_confirmed("taken, then dropped"),
            Accepted::No,
            "a dropped reply channel is an absent acceptance, not a granted one"
        );
        pump.join().unwrap();
    }

    #[test]
    fn a_chat_word_survives_a_closed_channel_without_panicking() {
        // The inbound thread outlives the pump by design during shutdown.
        let (tx, rx) = std::sync::mpsc::channel();
        let outgoing = super::Outgoing(tx);
        outgoing.say("first");
        assert!(
            matches!(rx.recv().unwrap(), Word::Say(text) if text == "first"),
            "a plain say must stay a plain, best-effort say"
        );
        drop(rx);
        outgoing.say("into the void");
    }

    #[test]
    fn the_poller_routes_then_stops_on_the_flag_and_leaves_its_offset_durable() {
        // CLEAN SHUTDOWN, end to end: a real thread, a real socket, a real
        // checkpoint.
        use crate::telegram::inbound::load_offset;
        use crate::telegram::tests::{CHAT, Fake, Reply};
        use std::sync::atomic::AtomicBool;
        use std::sync::mpsc::channel;

        let root = std::env::temp_dir().join(format!("ae-tg-bridge-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let paths = Paths::under(&root);
        std::fs::create_dir_all(paths.state()).unwrap();

        let update = format!(
            r#"{{"ok":true,"result":[{{"update_id":31,"message":{{"text":"hello",
               "from":{{"id":42}},"chat":{{"id":{CHAT},"type":"private"}}}}}}]}}"#
        );
        let fake = Fake::start(vec![
            Reply::json(200, &update),
            Reply::json(200, r#"{"ok":true,"result":[]}"#),
        ]);
        let (words, inbox) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let poller = super::spawn_inbound(
            Arc::new(fake.api()),
            crate::telegram::inbound::Policy::new(CHAT, vec!["42".to_owned()]),
            &paths,
            Knobs::default(),
            &super::Relay {
                census: crate::settings_menu::role_holder,
                ..relay()
            },
            words,
            Arc::clone(&stop),
        );

        // This temp root holds no sessions and records no orchestrator seat, so
        // the message has nowhere to go and the operator is told so, CONFIRMED
        // — which is still a ROUTED update once the platform accepts the notice.
        let word = inbox.recv_timeout(Duration::from_secs(10)).unwrap();
        let Word::SayAndConfirm(text, answer) = word else {
            panic!("the unavailable-default notice must be sent confirmed: {word:?}");
        };
        assert!(
            text.to_lowercase()
                .contains("no orchestrator seat is recorded"),
            "{text}"
        );
        answer.send(Accepted::Yes).unwrap();

        stop.store(true, Ordering::Relaxed);
        poller.join().expect("the poller must stop on the flag");
        assert_eq!(
            load_offset(&paths.state().join(crate::telegram::inbound::OFFSET_FILE)).unwrap(),
            31,
            "the offset was not durable when the thread returned"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_default_notice_telegram_refuses_leaves_the_offset_unmoved() {
        // The thread, the socket and the real census: the operator was NOT told, so the
        // update is not checkpointed and stays owed.
        use crate::telegram::inbound::load_offset;
        use crate::telegram::tests::{CHAT, Fake, Reply};
        use std::sync::atomic::AtomicBool;
        use std::sync::mpsc::channel;

        let root =
            std::env::temp_dir().join(format!("ae-tg-bridge-refused-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let paths = Paths::under(&root);
        std::fs::create_dir_all(paths.state()).unwrap();
        let update = format!(
            r#"{{"ok":true,"result":[{{"update_id":31,"message":{{"text":"hello",
               "from":{{"id":42}},"chat":{{"id":{CHAT},"type":"private"}}}}}}]}}"#
        );
        let fake = Fake::start(vec![
            Reply::json(200, &update),
            Reply::json(200, &update),
            Reply::json(200, &update),
        ]);
        let (words, inbox) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let poller = super::spawn_inbound(
            Arc::new(fake.api()),
            crate::telegram::inbound::Policy::new(CHAT, vec!["42".to_owned()]),
            &paths,
            Knobs::default(),
            &super::Relay {
                census: crate::settings_menu::role_holder,
                ..relay()
            },
            words,
            Arc::clone(&stop),
        );
        let word = inbox.recv_timeout(Duration::from_secs(10)).unwrap();
        let Word::SayAndConfirm(_, answer) = word else {
            panic!("the unavailable-default notice must be sent confirmed: {word:?}");
        };
        answer.send(Accepted::No).unwrap();
        stop.store(true, Ordering::Relaxed);
        poller.join().expect("the poller must stop on the flag");
        assert_eq!(
            load_offset(&paths.state().join(crate::telegram::inbound::OFFSET_FILE)).unwrap(),
            0,
            "an unheard notice must not checkpoint the update"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    // ─── the orchestrator seat: the strict role census, through the machine ───

    /// The meta of a proven seat: the role AND the seat overlay of its state root.
    const PROVEN: &str = "meta_agent=true\nlocal_config={ROOT}/orchestrator.config\n";

    /// A state root holding these sessions, each `(name, its meta text)`; `{ROOT}` in a meta
    /// text is the root itself.
    fn seat_root(tag: &str, sessions: &[(&str, &str)]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ae-tg-seat-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(root.join("sessions")).unwrap();
        for (name, meta) in sessions {
            let dir = root.join("sessions").join(name);
            std::fs::create_dir_all(&dir).unwrap();
            let meta = meta.replace("{ROOT}", &root.display().to_string());
            std::fs::write(dir.join("meta"), format!("session={name}\n{meta}")).unwrap();
        }
        root
    }

    /// A running record whose state directory is `dir`; its name is the directory's leaf.
    fn running_at(dir: PathBuf, role_bit: bool) -> RunningSession {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        RunningSession {
            session_id: format!("{}-0123456789abcdef", dir.display()),
            name,
            dir,
            meta_agent: role_bit,
            main: Some("claude:lead".to_owned()),
            agents: vec!["claude:lead".to_owned(), "codex:colead".to_owned()],
            pair: vec!["lead".to_owned(), "colead".to_owned()],
            last_active: Some(0),
        }
    }

    /// The seat a census over `root` proves for `name`: its canonical directory.
    fn held(root: &std::path::Path, name: &str) -> Seat {
        Seat::Held {
            name: name.to_owned(),
            dir: root.join("sessions").join(name),
        }
    }

    fn wired(root: &std::path::Path) -> Machine {
        Machine::under(Roots::under(root)).with_census(crate::settings_menu::role_holder)
    }

    /// Where an unselected plain message goes, given the machine's census and `running` sessions.
    fn default_route(
        machine: &Machine,
        running: &[RunningSession],
    ) -> crate::telegram::routing::Route {
        crate::telegram::routing::decide(
            crate::telegram::routing::Inbound {
                text: "status please",
                reply_to: None,
            },
            running,
            &crate::telegram::routing::Sticky::Unset,
            &machine.seat(),
            0,
        )
    }

    #[test]
    fn a_renamed_proven_seat_is_the_default_and_a_session_named_orchestrator_is_not() {
        let root = seat_root("renamed", &[("boss", PROVEN), ("orchestrator", "")]);
        let machine = wired(&root);
        assert_eq!(machine.seat(), held(&root, "boss"));
        let route = default_route(
            &machine,
            &[
                running_at(root.join("sessions/orchestrator"), false),
                running_at(root.join("sessions/boss"), true),
            ],
        );
        let crate::telegram::routing::Route::Chat { target, chosen, .. } = route else {
            panic!("the proven seat takes the default ask: {route:?}");
        };
        assert_eq!(target.session, "boss");
        assert_eq!(chosen, crate::telegram::routing::Chosen::Default);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_workspace_with_only_the_role_bit_is_no_seat_and_the_default_refuses() {
        let root = seat_root("rolebit", &[("ws", "meta_agent=true\n")]);
        let machine = wired(&root);
        let Seat::Unproven(why) = machine.seat() else {
            panic!("a role bit without the seat overlay proves nothing");
        };
        assert!(why.contains("without the seat overlay"), "{why}");
        // The workspace IS running and carries the bit; it still gets nothing.
        let route = default_route(&machine, &[running_at(root.join("sessions/ws"), true)]);
        assert!(
            matches!(&route, crate::telegram::routing::Route::Refused { notice, .. }
                if notice.contains("without the seat overlay")),
            "{route:?}"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_stopped_proven_seat_is_not_replaced_by_an_unrelated_running_workspace() {
        let root = seat_root("stopped", &[("boss", PROVEN), ("ws", "")]);
        let machine = wired(&root);
        assert_eq!(machine.seat(), held(&root, "boss"));
        // Only the workspace runs: the seat is stopped.
        let route = default_route(&machine, &[running_at(root.join("sessions/ws"), false)]);
        assert!(
            matches!(&route, crate::telegram::routing::Route::Refused { notice, .. }
                if notice.contains("boss") && notice.contains("is not running")),
            "{route:?}"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_live_nested_namesake_never_stands_in_for_a_stopped_canonical_seat() {
        // `role_holder` proves a name under `<AE_HOME>/sessions` only; the running listing also
        // holds worktree-nested records, and one of them can carry the seat's name.
        let root = seat_root("nested", &[("boss", PROVEN)]);
        let machine = wired(&root);
        let namesake = root.join("worktrees/wt/.ae/boss");
        // The canonical seat is stopped: the live namesake gets nothing.
        let route = default_route(&machine, &[running_at(namesake.clone(), false)]);
        assert!(
            matches!(&route, crate::telegram::routing::Route::Refused { notice, .. }
                if notice.contains("boss") && notice.contains("is not running")),
            "{route:?}"
        );
        // Both run: the canonical directory is the seat, and the namesake is no ambiguity.
        let route = default_route(
            &machine,
            &[
                running_at(namesake, false),
                running_at(root.join("sessions/boss"), true),
            ],
        );
        let crate::telegram::routing::Route::Chat { target, .. } = route else {
            panic!("the canonical seat runs and takes the default ask: {route:?}");
        };
        assert_eq!(target.dir, root.join("sessions/boss"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn duplicate_damaged_absent_and_unwired_censuses_never_name_a_seat() {
        let root = seat_root("dup", &[("a", PROVEN), ("b", PROVEN)]);
        let Seat::Unproven(why) = wired(&root).seat() else {
            panic!("two claims prove nothing");
        };
        assert!(why.contains("multiple sessions claim"), "{why}");
        std::fs::remove_dir_all(&root).ok();

        let root = seat_root("damaged", &[("a", "meta_agent=true\nmeta_agent=false\n")]);
        let Seat::Unproven(why) = wired(&root).seat() else {
            panic!("a damaged claim proves nothing");
        };
        assert!(why.contains("damaged"), "{why}");
        std::fs::remove_dir_all(&root).ok();

        // No role anywhere, and no sessions directory at all.
        let root = seat_root("plain", &[("work", "")]);
        assert_eq!(wired(&root).seat(), Seat::Vacant);
        std::fs::remove_dir_all(&root).ok();
        let bare = std::env::temp_dir().join(format!("ae-tg-seat-bare-{}", std::process::id()));
        std::fs::remove_dir_all(&bare).ok();
        assert_eq!(wired(&bare).seat(), Seat::Vacant);

        // A machine nobody wired a census into proves nothing.
        let root = seat_root("unwired", &[("boss", PROVEN)]);
        let Seat::Unproven(why) = Machine::under(Roots::under(&root)).seat() else {
            panic!("an unwired census must fail closed");
        };
        assert!(why.contains("not wired"), "{why}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_outbound_only_bridge_keeps_pumping_instead_of_exiting_immediately() {
        // Configuration is the one where this can go wrong: with no
        // allow-list there is no inbound thread, so there is no sender — and a
        // pump that read "no sender" as "the poller died" would exit on its
        // first iteration and take the outbound-only bridge down.
        use crate::telegram::tests::{Fake, Reply};
        use std::sync::atomic::AtomicBool;

        let root = std::env::temp_dir().join(format!("ae-tg-pump-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let paths = Paths::under(&root);
        let fake = Fake::start(vec![Reply::ok()]);
        let api = fake.api();
        let stop = AtomicBool::new(false);
        let returned = AtomicBool::new(false);

        std::thread::scope(|scope| {
            let pumping = scope.spawn(|| {
                let mut err = Vec::new();
                let outcome = super::pump(
                    &api,
                    &paths,
                    Knobs::default(),
                    &relay(),
                    None,
                    &stop,
                    &mut err,
                );
                returned.store(true, Ordering::Relaxed);
                outcome
            });
            std::thread::sleep(Duration::from_millis(300));
            assert!(
                !returned.load(Ordering::Relaxed),
                "the outbound-only pump exited instead of forwarding"
            );
            stop.store(true, Ordering::Relaxed);
            assert_eq!(pumping.join().unwrap().unwrap(), 0);
        });
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_sliced_sleep_returns_early_when_the_stop_flag_is_already_set() {
        let stop = std::sync::atomic::AtomicBool::new(true);
        let at = std::time::Instant::now();
        super::sleep_until(&stop, std::time::Duration::from_secs(30));
        assert!(at.elapsed() < std::time::Duration::from_secs(1));
    }
}
