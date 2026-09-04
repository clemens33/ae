//! The daemon: one thread long-polls Telegram, one pumps `say` outward.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Duration;

use super::inbound::{self, Accepted, Chat, Deliver, Delivered, Inbox, Policy, Refusal};
use super::routing::{RunningSession, Verb, World};
use super::{Api, Outbound, Settings};
use crate::inventory::{self, Roots, ServerId};
use crate::meta::{self, Meta};
use crate::transport;

/// The shortest a poll cycle may take when it did nothing and failed at
/// nothing. Insurance against a peer that ignores the long-poll timeout: the
/// wait is supposed to happen on the server, and this is what keeps the loop
/// from spinning when it does not.
const IDLE_FLOOR: Duration = Duration::from_millis(200);

/// How often the outbound pump re-scans for sessions and forwards what it
/// finds, when nothing else wakes it. The frozen daemon's discovery cadence.
const OUTBOUND_INTERVAL: Duration = Duration::from_secs(2);

/// The paths the bridge works between. Passed in, never read from the
/// environment: this crate's clippy policy disallows reading it, and a daemon
/// whose destination depends on who exported what is a daemon nobody can
/// reason about.
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
        let home = ae_home
            .parent()
            .map_or_else(|| ae_home.clone(), Path::to_path_buf);
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
    /// Run exactly one pass of each half and return. For a smoke test, and for
    /// a human who wants to see what the bridge would do.
    pub once: bool,
}

impl Default for Knobs {
    fn default() -> Self {
        Self {
            inbound: inbound::Knobs::default(),
            outbound_interval: OUTBOUND_INTERVAL,
            once: false,
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
        // way out. Dropping the word is right: there is nothing left to send it
        // with, and panicking in a worker thread would take the diagnostic with
        let _ = self.0.send(Word::Say(text.to_owned()));
    }

    fn say_confirmed(&self, text: &str) -> Accepted {
        // EVERY FAILURE DIRECTION IS `No`, and that is the whole design. The
        // caller advances a durable offset on `Yes`, so anything short of the
        // pump reporting an acceptance — a dead pump, a dropped reply channel —
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

/// The [`Deliver`] the inbound loop routes through: the session's OWN `send`
/// helper, and nothing else.
#[derive(Debug, Clone, Copy)]
pub struct Helper;

impl Deliver for Helper {
    fn deliver(
        &self,
        verb: Verb,
        session: &str,
        dir: &Path,
        agent: &str,
        text: &str,
        from_id: &str,
    ) -> Delivered {
        // an inbound message acts under the EXTERNAL actor identity, so
        // the event ledger records that a chat sent it and not that ae did.
        let sender = format!("telegram:{from_id}");
        let delivery = transport::deliver(
            &dir.join(verb.helper()),
            agent,
            text,
            &[("AE_SENDER_OVERRIDE", sender.as_str())],
        );
        if delivery.code == Some(0) {
            return Delivered::Yes;
        }
        Delivered::No(classify(session, dir, agent))
    }
}

/// Which give-up bound a refusal earns.
fn classify(session: &str, dir: &Path, agent: &str) -> Refusal {
    let Ok(bytes) = meta::read_bytes(dir) else {
        return Refusal::Transient;
    };
    let selector = Meta::parse(&String::from_utf8_lossy(&bytes)).server_selector();
    let Some(selector) = selector.entitles() else {
        return Refusal::Transient;
    };
    let Some(panes) =
        transport::observe_watch_panes(&ServerId::Selected(selector.clone()), session)
    else {
        return Refusal::Transient;
    };
    if panes
        .iter()
        .any(|pane| pane.agent.as_deref() == Some(agent))
    {
        Refusal::Transient
    } else {
        Refusal::Hard
    }
}

/// The [`World`] routing resolves against: this machine's running sessions.
#[derive(Debug, Clone)]
pub struct Machine {
    roots: Roots,
}

impl Machine {
    /// Scan the sessions under `roots`.
    #[must_use]
    pub fn under(roots: Roots) -> Self {
        Self { roots }
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
        // The watchdog's spelling, and the same reason: `sole_value` refuses a
        // key that appears twice rather than believing the first one.
        meta_agent: meta::sole_value(bytes, "meta_agent") == Some(b"true".as_slice()),
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
        last_active,
    }
}

/// Run the bridge until the process is stopped.
///
/// # Errors
///
/// Only writing the status stream. Every network, filesystem and delivery
/// failure degrades within its own cycle and is reported rather than fatal —
/// a bridge that exits because one poll failed is a bridge that stops relaying
/// the first time a laptop's wifi drops.
pub fn run(paths: &Paths, knobs: Knobs, err: &mut impl Write) -> crate::Result<u8> {
    let settings = match super::load_settings(&paths.config, &paths.home) {
        Ok(settings) => settings,
        Err(why) => {
            // `CredentialsError` carries paths and reasons, never file CONTENT
            // and never the token — see its declaration. It already names the
            // subsystem ("telegram: …"), so this adds only ae's own prefix; a
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
    } = settings;
    let policy = Policy::new(credentials.chat_id(), allowed_user_ids);
    let api = Arc::new(Api::production(credentials));
    let (words, inbox_words) = std::sync::mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));

    if policy.enabled() {
        // The command menu, registered while THIS thread is still the only one
        // that exists. That placement is the single-sender rule, not a
        // convenience: once the pump is running it owns the outbound socket,
        register_commands(&api, err)?;
    } else {
        // no allow-list, no inbound. Said once, so an operator who
        // expected two-way traffic learns why they have one.
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
    let outcome = pump(&api, paths, knobs, listening, &stop, err);
    stop.store(true, Ordering::Relaxed);
    // THE ORDER IS LOAD-BEARING: this drop must happen BEFORE the join, and
    // moving it below — or letting it fall to the end of the scope — hangs the
    // daemon on shutdown. The inbound thread may be blocked awaiting
    drop(inbox_words);
    if let Some(poller) = poller {
        // A poller that is mid-long-poll takes up to its timeout to notice the
        // flag. Joining is still right: the alternative is returning while a
        // thread still holds the durable offset mid-checkpoint.
        let _ = poller.join();
    }
    outcome
}

/// Spawn the inbound long poll.
fn spawn_inbound(
    api: Arc<Api>,
    policy: Policy,
    paths: &Paths,
    knobs: Knobs,
    words: Sender<Word>,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    let mut inbox = Inbox::new(paths.state(), knobs.inbound);
    let machine = Machine::under(paths.roots());
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
                &Helper,
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
            // nothing. The FLOOR is for the case where it does not — a proxy or
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
/// **`words` is an `Option` because "nobody is polling" and "the poller died"
/// are different facts and must not share a code path.** With an empty
/// allow-list there is no inbound thread at all, so there is no sender
/// — and a pump that read that as a disconnected channel would exit
/// immediately, taking the outbound-only bridge down on the one configuration
/// that is supposed to be outbound-only.
fn pump(
    api: &Api,
    paths: &Paths,
    knobs: Knobs,
    words: Option<&Receiver<Word>>,
    stop: &AtomicBool,
    err: &mut impl Write,
) -> crate::Result<u8> {
    // Keyed by state directory, so each session's cursor and scan position
    // survive between passes. `BTreeMap` for a deterministic pass order — a
    // hash order would make one session's backlog jump the queue at random.
    let mut bridges: BTreeMap<PathBuf, Outbound> = BTreeMap::new();
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(0);
        }
        if let Some(words) = words
            && matches!(drain(words, api, err)?, Drained::Closed)
            && !knobs.once
        {
            // The inbound thread is gone and cannot come back. With nothing
            // left to relay inward, the pump has no reason to hold the process
            // open.
            return Ok(0);
        }
        forward(api, paths, &mut bridges, err)?;
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
            // just an interval. Slept in slices so a stop is noticed sooner.
            None => sleep_until(stop, knobs.outbound_interval),
        }
    }
}

/// The slash commands the chat's `/` menu offers, and the whole of what
/// [`register_commands`] publishes.
const MENU: [(&str, &str); 4] = [
    ("list", "Running sessions and their agents"),
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
fn register_commands(api: &Api, err: &mut impl Write) -> crate::Result<()> {
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
fn drain(words: &Receiver<Word>, api: &Api, err: &mut impl Write) -> crate::Result<Drained> {
    loop {
        match words.try_recv() {
            Ok(word) => say(api, word, err)?,
            Err(TryRecvError::Empty) => return Ok(Drained::Open),
            Err(TryRecvError::Disconnected) => return Ok(Drained::Closed),
        }
    }
}

/// Act on one word.
fn say(api: &Api, word: Word, err: &mut impl Write) -> crate::Result<()> {
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
            // waiting — it took an answer, or its reply channel disconnected and
            // its own `recv()` returned the fail-safe `No`. Either way this send
            let _ = answer.send(accepted);
        }
    }
    Ok(())
}

/// One outbound pass across every session on the machine.
fn forward(
    api: &Api,
    paths: &Paths,
    bridges: &mut BTreeMap<PathBuf, Outbound>,
    err: &mut impl Write,
) -> crate::Result<()> {
    let scan = inventory::durable_records(&paths.roots());
    // A session that is no longer recorded takes its bridge with it. Without
    // this the map only ever grows, and a long-lived daemon on a machine that
    // churns sessions accumulates one cursor reader per session it has ever
    bridges.retain(|path, _| scan.records.iter().any(|record| record.path == *path));
    for record in &scan.records {
        let bridge = bridges
            .entry(record.path.clone())
            .or_insert_with(|| Outbound::new(&record.path, record.name.clone()));
        let pass = bridge.pump(api);
        if let Some(failure) = pass.failure {
            writeln!(err, "ae: telegram: {}: {failure}", record.name)?;
        }
    }
    Ok(())
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
mod tests {
    #![allow(
        clippy::disallowed_methods,
        reason = "fixtures build and inspect real files; the boundary is about what PRODUCT \
                  code may reach"
    )]

    use super::{Accepted, Delivered, Helper, Knobs, Paths, Refusal, Word, classify};
    use crate::telegram::inbound::{Chat as _, Deliver as _};
    use crate::telegram::routing::Verb;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

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
    fn the_delivery_helper_path_is_the_session_directory_plus_a_literal() {
        // The security property is that this is a JOIN of a literal, never a
        // value from meta, config or a chat message. Asserted through the one
        // observable the type offers: what it refuses to run.
        assert!(
            matches!(
                Helper.deliver(
                    Verb::Send,
                    "nowhere",
                    std::path::Path::new("/definitely/not/a/session"),
                    "cl:x",
                    "hello",
                    "42"
                ),
                Delivered::No(_)
            ),
            "a missing helper must report failure rather than success"
        );
    }

    #[test]
    fn a_refusal_this_daemon_could_not_probe_is_transient_and_never_hard() {
        // THE FAIL-SAFE DIRECTION, and the reason the classifier is written as
        // a chain of `else { Transient }`. `crate::watchdog` already holds the
        // rule that an unusable probe is not evidence of death; here the cost
        assert_eq!(
            classify("nowhere", std::path::Path::new("/no/such/session"), "cl:x"),
            Refusal::Transient,
            "unreadable meta says nothing about a pane"
        );

        let dir = std::env::temp_dir().join(format!("ae-tg-classify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A meta that names no server at all: Missing, not entitling.
        std::fs::write(dir.join("meta"), "mode=local\n").unwrap();
        assert_eq!(
            classify("nowhere", &dir, "cl:x"),
            Refusal::Transient,
            "a record that does not name exactly one server is not addressable, \
             and a target this daemon cannot address is not a target it may call dead"
        );
        // A server that names a socket nothing is listening on: the run fails,
        // and a failed enumeration is `None` rather than an empty roster.
        std::fs::write(
            dir.join("meta"),
            "mode=local\nserver_socket=/no/such/socket\n",
        )
        .unwrap();
        assert_eq!(
            classify("nowhere", &dir, "cl:x"),
            Refusal::Transient,
            "a tmux run that did not answer is not an answer of 'no panes'"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_sessions_facts_come_off_its_meta_including_the_two_keys_meta_does_not_keep() {
        // `session_id` and `meta_agent` are not fields of `Meta`, so they are
        // read off the same bytes it was parsed from. Both decide routing:
        // `meta_agent` is what makes a session the orchestrator, and
        let meta = concat!(
            "session_id=abc123def456\n",
            "meta_agent=true\n",
            "agent.main=claude:lead:e795c9e9\n",
            "agent.worker.0=codex:coworker\n",
        );
        let facts = super::session_facts(
            "orchestrator",
            std::path::Path::new("/sessions/orchestrator"),
            meta.as_bytes(),
            Some(1_800_000_000),
        );
        assert_eq!(facts.session_id, "abc123def456");
        assert!(facts.meta_agent);
        assert_eq!(facts.main.as_deref(), Some("claude:lead"));
        assert_eq!(facts.agents, vec!["claude:lead", "codex:coworker"]);
        assert_eq!(facts.last_active, Some(1_800_000_000));

        // A session that is NOT a meta-agent, and one whose flag is any other
        // value, are both ordinary sessions — `meta_agent` is `true` or it is
        // not set.
        for absent in [
            "",
            "meta_agent=false\n",
            "meta_agent=TRUE\n",
            "meta_agent=1\n",
        ] {
            let plain = super::session_facts(
                "work",
                std::path::Path::new("/sessions/work"),
                absent.as_bytes(),
                None,
            );
            assert!(!plain.meta_agent, "{absent:?} was read as an orchestrator");
            assert_eq!(plain.session_id, "");
            assert_eq!(plain.main, None);
        }

        // A DUPLICATED flag is doubt, not truth: `sole_value` refuses it rather
        // than believing the first line, which is the watchdog's rule too.
        let doubled = super::session_facts(
            "work",
            std::path::Path::new("/sessions/work"),
            b"meta_agent=true\nmeta_agent=true\n",
            None,
        );
        assert!(
            !doubled.meta_agent,
            "a twice-declared flag must not be read as a declaration"
        );
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
        // The four commands the reference documents, and no fifth: a menu entry
        // the router does not implement is a promise the chat cannot keep.
        for command in ["list", "use", "session", "help"] {
            assert!(
                seen[0].body.contains(&format!(r#""command":"{command}""#)),
                "{command} missing from the registered menu: {}",
                seen[0].body
            );
        }
        assert_eq!(super::MENU.len(), 4);

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
            // place it could leak. It must name the subsystem and nothing else.
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
        // until shutdown. Every branch of `say` must answer; this is what says
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
        // THE BUG THE BOUNDED WAIT HAD, as a test. With `recv_timeout`, a pump
        // busy longer than the bound left the word sitting in the channel: the
        // caller read its timeout as a rejection, came back at the next bound,
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
            // to accumulate. Any bounded wait shorter than this sleep would have
            // returned before the word was ever looked at.
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
        // THE FAIL-SAFE DEFAULT, tested where it actually lives. The inbound
        // loop advances a durable offset on `Yes`, so every way of NOT getting
        // an acceptance has to answer `No` — the dangerous direction is silence
        use crate::telegram::inbound::Chat as _;

        // 1. The pump is already gone: the word cannot even be queued.
        let (tx, rx) = std::sync::mpsc::channel::<Word>();
        drop(rx);
        assert_eq!(
            super::Outgoing(tx).say_confirmed("nobody is listening"),
            Accepted::No,
            "a word that could not be queued was never accepted by anyone"
        );

        // 2. The pump took the word and died holding it — the reply channel
        //    closes with no answer in it. This is the shape a shutdown makes.
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
        // checkpoint. What makes the shutdown clean is not a handshake — there
        // is none — but that the offset on disk is already correct when the
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
            words,
            Arc::clone(&stop),
        );

        // This temp root holds no sessions, so the message has nowhere to go
        // and the operator is told so — which is still a ROUTED update.
        let word = inbox.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(
            matches!(&word, Word::Say(text) if text.contains("No orchestrator running")),
            "{word:?}"
        );

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
    fn an_outbound_only_bridge_keeps_pumping_instead_of_exiting_immediately() {
        // configuration is the one where this can go wrong: with no
        // allow-list there is no inbound thread, so there is no sender — and a
        // pump that read "no sender" as "the poller died" would exit on its
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
                let outcome = super::pump(&api, &paths, Knobs::default(), None, &stop, &mut err);
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
