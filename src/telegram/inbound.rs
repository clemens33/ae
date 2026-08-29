//! The inbound half: one `getUpdates` long poll, routed, then checkpointed.
//!
//! # Delivery semantics: at-least-once, with at most one honest duplicate per
//! # routed-before-offset crash window
//!
//! This is the OPPOSITE of what the frozen bash bridge does, and the difference
//! is deliberate rather than incidental. `poll_updates` persisted the offset
//! BEFORE dispatching (`ae`'s own comment says "at-most-once"), so a crash in
//! between LOST the update: the offset had already moved past a message nobody
//! had acted on, and Telegram would never send it again.
//!
//! Here the durable offset advances ONLY after the update has been successfully
//! AND durably routed. The consequences, stated exactly:
//!
//! * A normal cycle re-delivers nothing: each update is checkpointed before the
//!   next is read.
//! * A crash in the window between "routed" and "the offset reached the disk"
//!   re-delivers exactly THAT ONE update on restart — a single honest
//!   duplicate, bounded to one update, never a loss and never a wholesale
//!   replay. That window is real and is not closed here: closing it would need
//!   a two-phase commit with a party that does not offer one.
//! * The same bound holds under REPEATED checkpoint failure, for the same
//!   reason it does in the outbound half: nothing in memory advances past the
//!   durable offset, so a disk that cannot be written degrades to throttled
//!   re-delivery of that one owed update rather than an unbounded replay.
//!
//! # What "successfully routed" means, and what it does not
//!
//! * An update the POLICY drops — unauthorized chat, unauthorized sender, a
//!   non-private chat, an update carrying no message text — is successfully
//!   handled. Dropping is the correct outcome, nothing is owed to anyone, and
//!   the offset advances past it. Holding on an unauthorized update would let
//!   any stranger who can find the bot wedge the bridge forever.
//! * An authorized update whose ROUTE FAILED — the target session's `send`
//!   helper refused, or the sticky override could not be written — is NOT
//!   routed. The offset does not move, the cycle backs off, and the same update
//!   is retried. See the BOUNDED GIVE-UP below for where that stops.
//! * The chat ACKNOWLEDGEMENT is best effort and is never part of the
//!   condition. An ack that fails to send is a message the operator does not
//!   see; making the offset depend on it would re-deliver commands that had
//!   already run.
//!
//! # The bounded give-up — a SEPARATE policy from the crash window above
//!
//! These two are easy to confuse and must not be: the crash window is about a
//! checkpoint that did not reach the disk, and produces a DUPLICATE. This is
//! about a target that will not take the message, and produces a message that
//! is handed BACK.
//!
//! Retrying forever is not "never lose", it is a DEADLOCK. A held offset blocks
//! every later update — including the `/use` that would redirect away from the
//! very target that is stuck — so a permanently dead pane would wedge the
//! bridge with no way for the operator to talk it out of the corner.
//!
//! So delivery is bounded, mirroring the sweep-delivery contract (SC-936/937/938)
//! that already governs the watchdog's nudges:
//!
//! * **Delivery is CHECKED** (SC-936): the send helper's own status decides, and
//!   a refusal is never counted as a delivery.
//! * **A refusal RETRIES on the short cadence** (SC-937): [`backoff_delay`],
//!   which starts at one second — shorter than the sweep's thirty, because an
//!   inbound message is someone waiting at a keyboard.
//! * **After the bound, the offset ADVANCES and ONE notice goes to the chat**
//!   (SC-938), naming the target that would not take it and QUOTING THE TEXT
//!   BACK. That is the whole point: the message is never *silently* lost. It is
//!   handed back to the only party who can do anything with it, and the queue
//!   goes live again.
//!
//! **Two bounds, because two refusals are not the same fact.** A TRANSIENT
//! refusal (a busy pane, a human mid-keystroke) recovers on its own and earns
//! the full [`Knobs::retry_max`]. A HARD one — the target's pane is not there
//! any more — earns [`Knobs::hard_retry_max`], which is small: retrying a pane
//! that no longer exists only spends the bound slower. Evidence for "hard" is
//! deliberately narrow, and it inherits `crate::watchdog`'s direction that an
//! unusable probe is NOT death: only a pane enumeration that SUCCEEDED and did
//! not list the target counts.
//!
//! The notice is sent BEFORE the checkpoint that ends the hold, so it inherits
//! the crash window above rather than opening one of its own: a crash — or a
//! checkpoint write that fails — replays the update, and the operator sees the
//! quoted message a second time. One honest duplicate per routed-before-offset
//! crash window, the SAME window and not a second policy. A repeatedly
//! unwritable disk therefore degrades to a quiet retry of that one update
//! followed by another hand-back, rather than a notice per cycle.
//!
//! # No state advancement on a bad response
//!
//! A malformed body, a body nested past `crate::json`'s `MAX_DEPTH`, one past
//! the streaming byte cap, a non-2xx, a 200 carrying `ok:false`, an
//! `update_id` that is not an integer this machine can hold — every one of them
//! ends the cycle with the offset EXACTLY where it was. A poisoned response can
//! cost throughput; it cannot skip an update or lose one.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::routing::{self, Inbound, Route, RunningSession, Verb, World};
use super::{Api, ApiFailure, NotRegular, backoff_delay, durable_write, read_regular_file};
use crate::json::Value;

/// The machine-global directory the bridge keeps its state in, under `AE_HOME`.
///
/// Machine-global rather than per-session, and that is the frozen layout: one
/// bot polls one chat for the whole machine, so the update offset and the
/// sticky target belong to the machine and not to whichever session happens to
/// be running.
pub const STATE_DIR: &str = "telegram";

/// The file the durable update offset lives in.
///
/// The frozen bash daemon's name and its one-decimal-line format, deliberately:
/// a Rust bridge starting on a machine where the bash one has run must inherit
/// its position rather than re-deliver everything Telegram still holds.
pub const OFFSET_FILE: &str = "tg_offset";

/// How many updates one poll may ask for. Bounded so a backlog cannot turn one
/// cycle into an unbounded amount of work, exactly as the frozen daemon bounds
/// it.
const UPDATES_LIMIT: u32 = 10;

/// The tunables of one inbound loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Knobs {
    /// Updates per poll.
    pub limit: u32,
    /// How long Telegram may hold the connection before answering empty.
    pub long_poll: Duration,
    /// How many times a TRANSIENT refusal is retried before the give-up.
    ///
    /// The sweep contract's own bound (SC-937/938, `SweepKnobs::retry_max`),
    /// reused rather than re-chosen: this is the same question about the same
    /// send helper, and two different answers to it would be two behaviours an
    /// operator has to learn.
    pub retry_max: u32,
    /// The bound for a HARD refusal — a target whose pane is provably gone.
    ///
    /// Small, and not one: a single enumeration that misses a pane could be a
    /// pane mid-restart, so the give-up wants one confirmation rather than a
    /// hair trigger.
    pub hard_retry_max: u32,
}

impl Default for Knobs {
    fn default() -> Self {
        Self {
            limit: UPDATES_LIMIT,
            long_poll: Duration::from_secs(10),
            retry_max: crate::watchdog::SweepKnobs::default().retry_max,
            hard_retry_max: 2,
        }
    }
}

// ─── the durable offset ──────────────────────────────────────────────────

/// Why the update offset could not be read or written.
#[derive(Debug)]
#[non_exhaustive]
pub enum OffsetError {
    /// The file exists and is not an offset.
    ///
    /// **REFUSED rather than treated as zero**, which is where this differs
    /// from the frozen bash (`tg_load_offset` falls back to `0` for anything
    /// non-numeric). Zero means "send me everything you still hold", so a
    /// damaged offset file would replay the whole retained backlog — breaking
    /// the one-duplicate bound this module promises. A refusal costs
    /// throughput and keeps the promise.
    Unrecognised,
    /// The path is not a regular file. Refused BEFORE the open, because
    /// `open(2)` on a FIFO waits for a writer and this directory is one other
    /// processes can create names in.
    NotRegular(PathBuf),
    /// Reading failed for a reason other than absence.
    Unreadable(io::Error),
    /// The write did not become durable. The caller must treat the update it
    /// was checkpointing as un-checkpointed.
    NotWritten(io::Error),
}

impl fmt::Display for OffsetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unrecognised => f.write_str("telegram offset: unrecognised contents"),
            Self::NotRegular(path) => {
                write!(
                    f,
                    "telegram offset: {} is not a regular file",
                    path.display()
                )
            }
            Self::Unreadable(source) => write!(f, "telegram offset: unreadable: {source}"),
            Self::NotWritten(source) => write!(f, "telegram offset: not written: {source}"),
        }
    }
}

impl std::error::Error for OffsetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unrecognised | Self::NotRegular(_) => None,
            Self::Unreadable(source) | Self::NotWritten(source) => Some(source),
        }
    }
}

/// The highest `update_id` this machine has routed. `0` when there has never
/// been one, which asks Telegram for everything it still holds.
///
/// # Errors
///
/// [`OffsetError`] for a file that exists and cannot be understood or read.
pub fn load_offset(path: &Path) -> Result<i64, OffsetError> {
    let (text, _) = match read_regular_file(path) {
        Ok(read) => read,
        Err(NotRegular::Absent) => return Ok(0),
        Err(NotRegular::Node) => return Err(OffsetError::NotRegular(path.to_owned())),
        Err(NotRegular::Unreadable(why)) => return Err(OffsetError::Unreadable(why)),
    };
    parse_offset(&text).ok_or(OffsetError::Unrecognised)
}

/// The one decimal line an offset is stored as. Strict: anything else is
/// `None`, never a zero.
#[must_use]
pub fn parse_offset(text: &str) -> Option<i64> {
    let line = text.trim();
    if line.is_empty() || !line.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    line.parse().ok()
}

/// Checkpoint the offset DURABLY, through the bridge's one durable write.
///
/// # Errors
///
/// [`OffsetError::NotWritten`]. A failure here means the update WAS routed and
/// the checkpoint was not — the single honest duplicate this module's contract
/// allows.
pub fn store_offset(path: &Path, update_id: i64) -> Result<(), OffsetError> {
    durable_write(path, &format!("{update_id}\n")).map_err(OffsetError::NotWritten)
}

// ─── hostile input, parsed ───────────────────────────────────────────────

/// One update, in the fields this bridge routes on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    /// Its `update_id`. The offset is derived from this, so it must be an
    /// integer this machine can hold — see [`Update::parse`].
    pub id: i64,
    /// Its `message`, when it carries one this bridge can read.
    pub message: Option<Message>,
}

/// One message, in the fields this bridge routes on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Message {
    /// `from.id`, as TEXT. Ids are compared, never arithmetic — see
    /// [`number_text`].
    pub from_id: String,
    /// `chat.id`, as text.
    pub chat_id: String,
    /// `chat.type` — only `private` is ever admitted.
    pub chat_type: String,
    /// `text`, or empty when there is none (a photo, a sticker, a join event).
    pub text: String,
    /// `reply_to_message.text`, when this is a reply.
    pub reply_to: Option<String>,
}

impl Update {
    /// Read one element of `getUpdates`' `result` array.
    ///
    /// `None` when there is no usable `update_id`, and NOTHING ELSE makes this
    /// fail: a message with no text, an update kind this bridge does not route,
    /// a future field it does not know are all fine, and become an update with
    /// no routable message.
    ///
    /// # The id is the one field that may refuse the whole update
    ///
    /// The offset is `update_id + 1`, so an id this machine cannot hold as an
    /// `i64` is an id it cannot checkpoint. `crate::json` keeps such a literal
    /// verbatim as [`Value::Raw`] rather than overflowing it, which is what
    /// makes refusing possible at all — the alternative, a silently wrapped
    /// number, would checkpoint a position that means nothing. A refusal here
    /// stops the cycle with the offset unchanged, which is the "no state
    /// advancement on a bad response" rule: nothing is skipped and nothing is
    /// lost, and a server that keeps sending one is a server we keep refusing.
    #[must_use]
    pub fn parse(value: &Value) -> Option<Self> {
        let id = match value.get("update_id")? {
            Value::Num(id) => *id,
            // EVERYTHING ELSE IS A REFUSAL, including [`Value::Raw`]. That is
            // not a defensive default, it is the exact rule: `crate::json`
            // produces `Raw` precisely when `i64::from_str` FAILS on the
            // literal (src/json.rs), so a `Raw` id is by construction one this
            // machine cannot hold — an id past `i64`, a float, an exponent.
            // An earlier version tried `literal.parse()` here, which could
            // never succeed and read as though some oversized ids were
            // accepted; cargo-mutants found it as an unkillable mutant, which
            // is what dead code looks like from the outside.
            _ => return None,
        };
        Some(Self {
            id,
            message: value.get("message").and_then(Message::parse),
        })
    }
}

impl Message {
    /// Read the `message` object. `None` when it is not an object at all.
    fn parse(value: &Value) -> Option<Self> {
        if !matches!(value, Value::Obj(_)) {
            return None;
        }
        let chat = value.get("chat");
        Some(Self {
            from_id: value
                .get("from")
                .and_then(|from| from.get("id"))
                .and_then(number_text)
                .unwrap_or_default(),
            chat_id: chat
                .and_then(|chat| chat.get("id"))
                .and_then(number_text)
                .unwrap_or_default(),
            chat_type: chat
                .and_then(|chat| chat.get_str("type"))
                .unwrap_or_default()
                .to_owned(),
            text: value.get_str("text").unwrap_or_default().to_owned(),
            reply_to: value
                .get("reply_to_message")
                .and_then(|replied| replied.get_str("text"))
                .map(ToOwned::to_owned),
        })
    }
}

/// A JSON NUMBER as the text it was written as.
///
/// Telegram ids are numbers, and this bridge compares them as strings — which
/// is what the frozen bash does through `jq -r`, and what the allow-list in the
/// config is written in. Keeping them textual means an id larger than any
/// integer type cannot overflow anything on its way to a comparison it was
/// always going to fail.
///
/// **A JSON STRING is not a number and is refused.** Telegram sends ids as
/// numbers; accepting `"123"` here would let a body that merely LOOKS like an
/// update satisfy an allow-list written for the real thing.
fn number_text(value: &Value) -> Option<String> {
    match value {
        Value::Num(id) => Some(id.to_string()),
        Value::Raw(literal) => Some(literal.clone()),
        _ => None,
    }
}

// ─── who is allowed to drive ae from a chat ──────────────────────────────

/// The inbound trust predicate: which chat, and which senders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    chat_id: String,
    allowed: Vec<String>,
}

impl Policy {
    /// The configured control chat and its allow-list.
    #[must_use]
    pub fn new(chat_id: impl Into<String>, allowed: Vec<String>) -> Self {
        Self {
            chat_id: chat_id.into(),
            allowed,
        }
    }

    /// **SC-943: inbound exists ONLY with a non-empty allow-list.** An empty
    /// one is not a permissive default — it is an outbound-only bridge, and the
    /// inbound loop does not poll at all.
    #[must_use]
    pub fn enabled(&self) -> bool {
        !self.allowed.is_empty()
    }

    /// Whether this message may drive ae.
    ///
    /// **ALL FOUR must hold**, and they are four rather than one because each
    /// closes a different door (SC-944a/b/c):
    ///
    /// * the sender id is NUMERIC — an allow-list entry can only ever be
    ///   matched by something shaped like a real id;
    /// * the sender is on the allow-list;
    /// * the chat is EXACTLY the configured control chat — so a bot added to
    ///   some other conversation drives nothing;
    /// * the chat is PRIVATE — so a group that happens to carry the right id
    ///   cannot, and an accidental add cannot.
    ///
    /// A refusal is SILENT to the sender by design: an unauthorized party
    /// learns nothing about whether ae is there.
    #[must_use]
    pub fn admits(&self, message: &Message) -> bool {
        !message.from_id.is_empty()
            && message.from_id.bytes().all(|byte| byte.is_ascii_digit())
            && message.chat_type == "private"
            && message.chat_id == self.chat_id
            && self.allowed.contains(&message.from_id)
    }
}

// ─── the effects, behind seams ───────────────────────────────────────────

/// Deliver a message into an agent's pane.
///
/// A trait so the routing gate can assert WHERE a message went without a tmux
/// server. The production implementation is [`super::bridge::Helper`], which
/// runs the session's own `send` helper and nothing else.
pub trait Deliver {
    /// Run `dir`'s send helper for `agent`.
    ///
    /// [`Delivered::Yes`] means delivered AND logged — the helper's own status,
    /// never re-read as anything weaker (SC-936). A refusal says which KIND it
    /// is, because the two earn different retry bounds.
    fn deliver(
        &self,
        verb: Verb,
        session: &str,
        dir: &Path,
        agent: &str,
        text: &str,
        from_id: &str,
    ) -> Delivered;
}

/// What one delivery attempt produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivered {
    /// The helper took it and logged it.
    Yes,
    /// It did not land, and this is why.
    No(Refusal),
}

/// Why a delivery did not land — the two answers that earn different bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The target could not take it RIGHT NOW: busy, a human mid-keystroke, a
    /// lock held. This recovers on its own, so it earns the full retry bound.
    Transient,
    /// The target cannot take it AT ALL: its pane is gone. Retrying on a
    /// cadence built for a busy pane only spends the bound more slowly.
    ///
    /// **Claimed only on positive evidence.** A probe that failed, or a pane
    /// that is present, is [`Refusal::Transient`] — the same direction
    /// `crate::watchdog::classify_dead` takes, and for the same reason: a
    /// missed hard-refusal costs a few extra retries, while a false one throws
    /// away a message that would have been delivered.
    Hard,
}

/// Say something back to the operator's chat.
pub trait Chat {
    /// Queue one message. Best effort by contract: see the module docs.
    ///
    /// An acknowledgement that never arrives costs the operator a line they do
    /// not see, and the update it belongs to has already been routed AND
    /// checkpointed — so re-running it to deliver an ack would re-run a
    /// command. That is why this one is allowed to fail quietly.
    fn say(&self, text: &str);

    /// Send one message and WAIT for the chat platform to accept it.
    ///
    /// **The give-up notice is the one message whose delivery gates state.**
    /// [`Inbox::poll`] advances the durable offset past an undelivered update
    /// only after this returns [`Accepted::Yes`], because the notice is the
    /// entire reason a give-up is a hand-back rather than a silent loss. A
    /// best-effort notice would make "never silently lose" a phrase: the
    /// offset would move whether or not anyone was ever told.
    ///
    /// Accepting means the platform took it — HTTP 2xx **and** `ok:true`.
    /// Every other outcome, including no answer at all, is
    /// [`Accepted::No`]: absence of an acceptance is never read as one.
    fn say_confirmed(&self, text: &str) -> Accepted;
}

/// Whether the chat platform ACCEPTED a message — the gate on the give-up
/// checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accepted {
    /// The platform took it: 2xx and `ok:true`.
    Yes,
    /// It did not, or nothing said that it did. Fail-safe by construction —
    /// a rejection, a transport error, a dead sender and a silence all land
    /// here, and all of them keep the update owed.
    No,
}

// ─── one cycle ───────────────────────────────────────────────────────────

/// What one poll did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cycle {
    /// Updates routed AND checkpointed.
    pub routed: usize,
    /// Updates dropped by policy, and checkpointed — see the module docs on
    /// why a policy drop counts as handled.
    pub dropped: usize,
    /// Updates that reached the give-up bound: handed back to the chat and
    /// stepped over. Counted apart from both of the above, because "we could
    /// not deliver this, and said so" is neither a delivery nor a policy drop.
    pub undelivered: usize,
    /// What ended the cycle early, if anything did.
    pub failure: Option<CycleFailure>,
    /// How long to wait before polling again, given the failure streak.
    pub retry_after: Duration,
}

/// Why a cycle stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CycleFailure {
    /// The call itself failed, in one of the redacted classes.
    Api(ApiFailure),
    /// The offset could not be read, or a checkpoint could not be made
    /// durable. Rendered as ae's own text — never a network value.
    Offset(String),
    /// A response this reader cannot frame as updates.
    Malformed,
    /// An authorized update could not be routed. The offset is unchanged and
    /// the update is owed.
    Undelivered(String),
}

impl fmt::Display for CycleFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Api(failure) => write!(f, "{failure}"),
            Self::Offset(why) => f.write_str(why),
            Self::Malformed => f.write_str("getUpdates: a result this reader cannot frame"),
            Self::Undelivered(what) => write!(f, "undelivered: {what}"),
        }
    }
}

/// What one update's handling produced.
///
/// [`Handled::Routed`] and [`Handled::Dropped`] both advance the offset and are
/// still kept apart, because they are what the cycle REPORTS: an operator
/// reading "routed 12" wants the number of messages that reached an agent or
/// earned an answer, not that number plus every stranger the policy turned
/// away.
enum Handled {
    /// The update produced its effect: delivered, or answered.
    Routed,
    /// Dropped by policy, or carrying nothing to route. Nothing is owed.
    Dropped,
    /// The update is still owed. The offset must NOT advance past it — until
    /// the give-up bound, which is [`Inbox::poll`]'s decision to make and not
    /// this value's.
    Owed(Owing),
}

/// An update that did not land, and everything a give-up notice would need.
///
/// The TEXT is CARRIED rather than looked up again: at the bound the message
/// has to be quoted back to the operator, and re-deriving it from a re-read of
/// anything is how a notice ends up quoting a different message than the one
/// that was lost.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Owing {
    /// What would not take it, as the notice names it.
    target: String,
    /// The text that did not land.
    text: String,
    /// Which bound applies.
    refusal: Refusal,
}

/// How many times the CURRENTLY owed update has been attempted.
///
/// Keyed by `update_id`, so a different update owing something resets the count
/// rather than inheriting a stranger's. Held in memory only: a restart forgets
/// it and the update gets a fresh set of attempts — which can only ever cause
/// MORE retries, never a skip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Attempts {
    id: i64,
    count: u32,
}

/// The inbound loop's state: the paths it works between, and its failure
/// streak.
///
/// Holds no position of its own. The durable offset is re-read at the start of
/// every cycle rather than cached, which is what makes a failed checkpoint
/// degrade to re-delivering ONE update instead of letting an in-memory position
/// run ahead of the disk — the exact bug the outbound half documents at its own
/// checkpoint site.
#[derive(Debug)]
pub struct Inbox {
    state: PathBuf,
    knobs: Knobs,
    failures: u32,
    attempts: Option<Attempts>,
}

impl Inbox {
    /// Bind an inbox to the machine-global telegram state directory.
    #[must_use]
    pub fn new(state: impl Into<PathBuf>, knobs: Knobs) -> Self {
        Self {
            state: state.into(),
            knobs,
            failures: 0,
            attempts: None,
        }
    }

    /// Where the durable offset lives.
    #[must_use]
    pub fn offset_path(&self) -> PathBuf {
        self.state.join(OFFSET_FILE)
    }

    /// Where the sticky `/use` target lives.
    #[must_use]
    pub fn sticky_path(&self) -> PathBuf {
        self.state.join(routing::TARGET_FILE)
    }

    /// Poll once: fetch, route, checkpoint.
    ///
    /// Every early return leaves the durable offset EXACTLY where it was.
    pub fn poll(
        &mut self,
        api: &Api,
        policy: &Policy,
        world: &dyn World,
        send: &dyn Deliver,
        chat: &dyn Chat,
        now: i64,
    ) -> Cycle {
        // SC-943: with no allow-list there is no authorized sender, so there is
        // nothing to poll FOR. Not an error, and not a failure streak.
        if !policy.enabled() {
            self.failures = 0;
            return Self::quiet();
        }
        let stored = match load_offset(&self.offset_path()) {
            Ok(stored) => stored,
            Err(why) => return self.failed(CycleFailure::Offset(why.to_string())),
        };
        // The lowest id this poll may legitimately be answered with. It becomes
        // the running floor below, so the offset can only ever move FORWARD.
        let mut expected = stored.saturating_add(1);
        let updates = match api.get_updates(expected, self.knobs.limit, self.knobs.long_poll) {
            Ok(updates) => updates,
            Err(failure) => return self.failed(CycleFailure::Api(failure)),
        };

        let mut cycle = Cycle {
            routed: 0,
            dropped: 0,
            undelivered: 0,
            failure: None,
            retry_after: Duration::ZERO,
        };
        for value in &updates {
            let Some(update) = Update::parse(value) else {
                // An update whose id cannot be held cannot be checkpointed, and
                // an update after it cannot be checkpointed either without
                // silently stepping over this one.
                return self.failed_after(cycle, CycleFailure::Malformed);
            };
            // A REPLAYED OR REORDERED ID, which is the one hostile shape that
            // does not look malformed. We asked for everything from `expected`
            // onwards; an id below that is either an update this machine has
            // already routed or one the server put out of order. Routing it
            // would deliver a second copy, and checkpointing it would move the
            // durable offset BACKWARDS — re-delivering every update in between
            // and turning the one-duplicate bound into an unbounded replay a
            // remote party controls. Refused with the offset untouched; a gap
            // (an id higher than expected) is normal and is not refused.
            if update.id < expected {
                return self.failed_after(cycle, CycleFailure::Malformed);
            }
            let handled = self.handle(&update, policy, world, send, chat, now);
            let mut gave_up = false;
            if let Handled::Owed(owing) = &handled {
                let attempts = self.attempt(update.id);
                let bound = match owing.refusal {
                    Refusal::Transient => self.knobs.retry_max,
                    Refusal::Hard => self.knobs.hard_retry_max,
                };
                if attempts < bound {
                    // INSIDE THE BOUND: hold the offset and retry on the short
                    // cadence. The refusal is REPORTED (SC-936) as the cycle's
                    // failure, which the daemon logs; the chat hears nothing
                    // yet, exactly as an undelivered sweep is logged and not
                    // announced.
                    return self
                        .failed_after(cycle, CycleFailure::Undelivered(owing.target.clone()));
                }
                // THE BOUND. Hand the message back and step over it, so the
                // queue — and with it the `/use` that could redirect away from
                // this target — is live again. The module's give-up policy has
                // the reasoning; the checkpoint below is what ends the hold.
                //
                // THE HAND-BACK MUST LAND FIRST. Queuing the notice and
                // advancing anyway would be a silent loss wearing a notice's
                // clothes: the offset would move past the update whether or not
                // the operator was ever told, which is the exact failure the
                // give-up exists to prevent. So the notice is CONFIRMED — the
                // platform accepted it — before the checkpoint below runs.
                if chat.say_confirmed(&give_up_notice(owing, attempts)) == Accepted::No {
                    // Nobody was told, so nothing is handed back and the offset
                    // does not move. The update stays owed exactly as it was
                    // inside the bound: the cycle backs off and the give-up is
                    // re-attempted next cycle. `attempts` is deliberately NOT
                    // cleared — clearing it would grant a fresh full bound of
                    // delivery retries every time the notice failed to send.
                    return self
                        .failed_after(cycle, CycleFailure::Undelivered(owing.target.clone()));
                }
                self.attempts = None;
                gave_up = true;
            }
            // THE ADVANCE IS THE CHECKPOINT, and it happens BEFORE the next
            // update is looked at. A failure here means this update was routed
            // and its position was not recorded: the next cycle re-delivers
            // exactly this one — the single honest duplicate per
            // routed-before-offset crash window — and re-attempts the write.
            if let Err(why) = store_offset(&self.offset_path(), update.id) {
                return self.failed_after(cycle, CycleFailure::Offset(why.to_string()));
            }
            expected = update.id.saturating_add(1);
            match handled {
                Handled::Routed => cycle.routed += 1,
                Handled::Dropped => cycle.dropped += 1,
                // Reachable ONLY past the give-up bound: an owed update inside
                // its bound returned above with the offset untouched.
                Handled::Owed(_) => {
                    debug_assert!(gave_up, "an owed update was checkpointed inside its bound");
                    cycle.undelivered += 1;
                }
            }
        }
        self.failures = 0;
        cycle.retry_after = Duration::ZERO;
        cycle
    }

    /// Handle one authorized-or-not update.
    fn handle(
        &self,
        update: &Update,
        policy: &Policy,
        world: &dyn World,
        send: &dyn Deliver,
        chat: &dyn Chat,
        now: i64,
    ) -> Handled {
        // Everything below is a POLICY DROP: handled, nothing owed, offset
        // advances. An update with no message is one this bridge did not ask
        // for; an unadmitted one is silently discarded (SC-944a/b/c); an
        // admitted one with no text carries nothing to route.
        let Some(message) = update.message.as_ref() else {
            return Handled::Dropped;
        };
        if !policy.admits(message) || message.text.trim().is_empty() {
            return Handled::Dropped;
        }
        let sessions = world.running();
        let sticky = routing::read_sticky(&self.sticky_path());
        let route = routing::decide(
            Inbound {
                text: &message.text,
                reply_to: message.reply_to.as_deref(),
            },
            &sessions,
            &sticky,
            now,
        );
        self.execute(route, &message.from_id, send, chat)
    }

    /// Perform one decided route.
    fn execute(&self, route: Route, from_id: &str, send: &dyn Deliver, chat: &dyn Chat) -> Handled {
        match route {
            Route::Answer(text) => {
                chat.say(&text);
                Handled::Routed
            }
            Route::Use { session, agent } => {
                if durable_write(
                    &self.sticky_path(),
                    &routing::render_sticky(&session, &agent),
                )
                .is_err()
                {
                    // The command did not take effect, so it is still owed. A
                    // disk that will not take a write is transient by nature —
                    // and where it is not, the bound ends it anyway.
                    return Handled::Owed(Owing {
                        target: "the /use target (it could not be saved)".to_owned(),
                        text: format!("/use {session} {agent}"),
                        refusal: Refusal::Transient,
                    });
                }
                chat.say(&format!(
                    "Override set: {session} → {agent}. Plain messages go here until /use clear \
                     (then back to the orchestrator)."
                ));
                Handled::Routed
            }
            Route::Unuse => {
                if clear_sticky(&self.sticky_path()).is_err() {
                    return Handled::Owed(Owing {
                        target: "the /use target (it could not be cleared)".to_owned(),
                        text: "/use clear".to_owned(),
                        refusal: Refusal::Transient,
                    });
                }
                chat.say("Override cleared — plain messages go to the orchestrator again.");
                Handled::Routed
            }
            Route::Deliver {
                verb,
                session,
                dir,
                agent,
                text,
            } => {
                match send.deliver(verb, &session, &dir, &agent, &text, from_id) {
                    Delivered::Yes => {
                        chat.say(&format!("→ {} to {agent} in {session}", verb.past()));
                        Handled::Routed
                    }
                    // Owed, and retried on the short cadence until the bound.
                    // The frozen bash reported this and moved on with the offset
                    // already past the update, which LOSES the message; holding
                    // it forever deadlocks the queue. The bound is the third
                    // answer, and it is the caller's to apply.
                    Delivered::No(refusal) => Handled::Owed(Owing {
                        target: format!("{agent} in {session}"),
                        text,
                        refusal,
                    }),
                }
            }
        }
    }

    /// Count one attempt at `id`, RESETTING when a different update is owed.
    fn attempt(&mut self, id: i64) -> u32 {
        let count = match self.attempts {
            Some(prior) if prior.id == id => prior.count.saturating_add(1),
            _ => 1,
        };
        self.attempts = Some(Attempts { id, count });
        count
    }

    /// A cycle that did nothing because there was nothing to do.
    fn quiet() -> Cycle {
        Cycle {
            routed: 0,
            dropped: 0,
            undelivered: 0,
            failure: None,
            retry_after: Duration::ZERO,
        }
    }

    /// A cycle that failed before any update was read.
    fn failed(&mut self, failure: CycleFailure) -> Cycle {
        self.failed_after(
            Cycle {
                routed: 0,
                dropped: 0,
                undelivered: 0,
                failure: None,
                retry_after: Duration::ZERO,
            },
            failure,
        )
    }

    /// A cycle that failed after some of its updates had already been
    /// checkpointed — those stay reported, because they really did happen.
    fn failed_after(&mut self, mut cycle: Cycle, failure: CycleFailure) -> Cycle {
        self.failures = self.failures.saturating_add(1);
        cycle.failure = Some(failure);
        cycle.retry_after = backoff_delay(self.failures);
        cycle
    }
}

/// The ONE notice a give-up sends.
///
/// It names what would not take the message and QUOTES THE MESSAGE BACK.
/// Quoting is the substance and not a courtesy: without it the operator learns
/// only that something of theirs vanished, and "never silently lose" would be a
/// phrase rather than a property. `Api::send_message` caps the length, so an
/// over-long message is truncated by the sender rather than dropped here.
///
/// THIS NOTICE CAN ITSELF ARRIVE TWICE, and its duplicate window is the SAME
/// one the module documents for a routed message: the notice is sent before the
/// checkpoint, so a crash — or a checkpoint write that fails — replays the
/// update, and the retry either delivers it (the operator sees the quote and
/// then the delivery) or reaches the bound again (the operator sees the quote
/// twice). One honest duplicate per routed-before-offset crash window, not a
/// second policy.
fn give_up_notice(owing: &Owing, attempts: u32) -> String {
    let why = match owing.refusal {
        Refusal::Transient => "it would not accept the message",
        Refusal::Hard => "its pane is gone",
    };
    format!(
        "Could not deliver to {} after {attempts} attempts ({why}). Giving up on it so later \
         messages are not blocked — your message is NOT lost, here it is:\n\n{}",
        owing.target, owing.text
    )
}

/// Remove the sticky target. An ALREADY-ABSENT file is success: `/use clear`
/// asks for a state, not for an event.
fn clear_sticky(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(why) if why.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(why) => Err(why),
    }
}

/// A world with no sessions in it — what a machine with nothing running looks
/// like to routing.
impl World for Vec<RunningSession> {
    fn running(&self) -> Vec<RunningSession> {
        self.clone()
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        reason = "fixtures build and inspect real files; the boundary is about what PRODUCT \
                  code may reach"
    )]

    use super::{
        Accepted, Chat, Cycle, CycleFailure, Deliver, Delivered, Inbox, Knobs, Message, Policy,
        Refusal, Update, durable_write, load_offset, parse_offset, store_offset,
    };
    use crate::json::{self, Value};
    use crate::telegram::routing::{self, RunningSession, Verb};
    use crate::telegram::tests::{CHAT, Fake, Reply, with_unwritable_dir};
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    const NOW: i64 = 1_800_000_000;
    /// The fake Telegram's chat, so the policy under test and the updates the
    /// server sends cannot drift apart into a test that passes for the wrong
    /// reason.
    const CHAT_ID: &str = CHAT;
    const SENDER: &str = "42";

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ae-tg-inbound-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn policy() -> Policy {
        Policy::new(CHAT_ID, vec![SENDER.to_owned()])
    }

    fn message(text: &str) -> Message {
        Message {
            from_id: SENDER.to_owned(),
            chat_id: CHAT_ID.to_owned(),
            chat_type: "private".to_owned(),
            text: text.to_owned(),
            reply_to: None,
        }
    }

    struct Recorder {
        delivered: Mutex<Vec<(PathBuf, String, String, String)>>,
        /// The verb each delivery ran under, so a downgraded `ask` is visible.
        verbs: Mutex<Vec<Verb>>,
        said: Mutex<Vec<String>>,
        /// `None` delivers; `Some(refusal)` refuses with that classification.
        refuse: Option<Refusal>,
        /// What the chat platform answers for a CONFIRMED message. Defaults to
        /// acceptance, so only a test about rejection has to say so.
        confirm: Accepted,
    }

    /// Written out rather than derived, because `Accepted` deliberately has NO
    /// `Default`: a fail-safe type whose default is `Yes` would let a caller
    /// that forgot to set it advance a durable offset, which is the exact bug
    /// the type exists to prevent. The permissive value is a TEST convenience
    /// and belongs here, spelled out.
    impl Default for Recorder {
        fn default() -> Self {
            Self {
                delivered: Mutex::default(),
                verbs: Mutex::default(),
                said: Mutex::default(),
                refuse: None,
                confirm: Accepted::Yes,
            }
        }
    }

    impl Recorder {
        fn refusing(refusal: Refusal) -> Self {
            Self {
                refuse: Some(refusal),
                ..Self::default()
            }
        }

        /// Refuses delivery AND has the chat platform reject the notice.
        fn refusing_unheard(refusal: Refusal) -> Self {
            Self {
                confirm: Accepted::No,
                ..Self::refusing(refusal)
            }
        }
    }

    impl Deliver for Recorder {
        fn deliver(
            &self,
            verb: Verb,
            _session: &str,
            dir: &Path,
            agent: &str,
            text: &str,
            from_id: &str,
        ) -> Delivered {
            self.verbs.lock().unwrap().push(verb);
            self.delivered.lock().unwrap().push((
                dir.to_owned(),
                agent.to_owned(),
                text.to_owned(),
                from_id.to_owned(),
            ));
            match self.refuse {
                Some(refusal) => Delivered::No(refusal),
                None => Delivered::Yes,
            }
        }
    }

    impl Chat for Recorder {
        fn say(&self, text: &str) {
            self.said.lock().unwrap().push(text.to_owned());
        }

        fn say_confirmed(&self, text: &str) -> Accepted {
            self.said.lock().unwrap().push(text.to_owned());
            self.confirm
        }
    }

    fn world() -> Vec<RunningSession> {
        vec![RunningSession {
            name: "work".to_owned(),
            dir: PathBuf::from("/sessions/work"),
            session_id: "abc123".to_owned(),
            meta_agent: true,
            main: Some("claude:lead".to_owned()),
            agents: vec!["claude:lead".to_owned(), "codex:dev".to_owned()],
            last_active: Some(NOW),
        }]
    }

    // ─── the fake getUpdates server ──────────────────────────────────────
    //
    // THE PRODUCT GATE. Everything above this line tests a decision; what
    // follows drives the real client against a real socket, because the
    // properties at stake — that the offset does not move, that a body is
    // capped while it streams, that a restart re-delivers nothing — are
    // properties of the whole cycle and cannot be asserted on a pure function.

    /// A `getUpdates` envelope carrying these update objects.
    fn result(updates: &[String]) -> String {
        format!(r#"{{"ok":true,"result":[{}]}}"#, updates.join(","))
    }

    /// One update, as Telegram frames it.
    fn from(id: i64, text: &str, sender: &str) -> String {
        format!(
            r#"{{"update_id":{id},"message":{{"text":{},"from":{{"id":{sender}}},
               "chat":{{"id":{CHAT},"type":"private"}}}}}}"#,
            Value::str(text).render()
        )
    }

    /// Poll once against `api`, recording what happened.
    fn cycle(inbox: &mut Inbox, api: &crate::telegram::Api, recorder: &Recorder) -> Cycle {
        inbox.poll(api, &policy(), &world(), recorder, recorder, NOW)
    }

    #[test]
    fn a_routed_update_advances_the_offset_and_a_restart_re_delivers_nothing() {
        let dir = temp("cycle-normal");
        let fake = Fake::start(vec![
            Reply::json(200, &result(&[from(7, "do the thing", SENDER)])),
            Reply::json(200, &result(&[])),
        ]);
        let api = fake.api();
        let recorder = Recorder::default();

        let mut inbox = Inbox::new(&dir, Knobs::default());
        let first = cycle(&mut inbox, &api, &recorder);
        assert_eq!((first.routed, first.dropped), (1, 0), "{first:?}");
        assert!(first.failure.is_none(), "{first:?}");
        assert_eq!(
            recorder.delivered.lock().unwrap().first().unwrap().1,
            "claude:lead",
            "a plain message must reach the orchestrator's main agent"
        );
        assert_eq!(
            load_offset(&inbox.offset_path()).unwrap(),
            7,
            "the offset must be durable the moment the update is routed"
        );

        // A RESTART: a fresh inbox over the same directory, exactly as the
        // daemon comes back after being killed.
        let mut restarted = Inbox::new(&dir, Knobs::default());
        let second = cycle(&mut restarted, &api, &recorder);
        assert_eq!(second.routed, 0);
        assert_eq!(
            recorder.delivered.lock().unwrap().len(),
            1,
            "a normal restart re-delivered something it had already routed"
        );
        let asked = fake.requests();
        assert!(
            asked[1].body.contains(r#""offset":8"#),
            "the restart did not resume past the routed update: {}",
            asked[1].body
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_fault_between_the_route_and_the_checkpoint_replays_exactly_one_update() {
        // THE CRASH WINDOW, made real: the update is routed, the checkpoint
        // cannot be written, and the contract says exactly that one update
        // comes back — never nothing, never everything.
        let dir = temp("cycle-crash");
        let fake = Fake::one(Reply::json(200, &result(&[from(11, "once", SENDER)])));
        let api = fake.api();
        let recorder = Recorder::default();
        let mut inbox = Inbox::new(&dir, Knobs::default());

        let held = with_unwritable_dir(&dir, || cycle(&mut inbox, &api, &recorder));
        assert!(
            matches!(held.failure, Some(CycleFailure::Offset(_))),
            "{held:?}"
        );
        assert_eq!(
            recorder.delivered.lock().unwrap().len(),
            1,
            "the update WAS routed — that is what makes this a duplicate rather than a loss"
        );
        assert_eq!(
            load_offset(&inbox.offset_path()).unwrap(),
            0,
            "the offset moved despite the checkpoint failing"
        );

        // Telegram re-sends it, because nothing ever acknowledged it.
        let recovered = cycle(&mut inbox, &api, &recorder);
        assert!(recovered.failure.is_none(), "{recovered:?}");
        assert_eq!(
            recorder.delivered.lock().unwrap().len(),
            2,
            "exactly ONE honest duplicate per routed-before-offset crash window"
        );
        assert_eq!(load_offset(&inbox.offset_path()).unwrap(), 11);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_update_that_cannot_be_routed_holds_the_offset_and_is_retried() {
        // The deliberate hold. The frozen bash advanced past this update and
        // reported the failure to chat — which loses the message.
        let dir = temp("cycle-hold");
        let fake = Fake::one(Reply::json(200, &result(&[from(3, "urgent", SENDER)])));
        let api = fake.api();
        let recorder = Recorder::refusing(Refusal::Transient);
        let mut inbox = Inbox::new(&dir, Knobs::default());
        let held = cycle(&mut inbox, &api, &recorder);
        assert!(
            matches!(held.failure, Some(CycleFailure::Undelivered(_))),
            "{held:?}"
        );
        assert_eq!(load_offset(&inbox.offset_path()).unwrap(), 0);
        assert!(
            !held.retry_after.is_zero(),
            "a hold must ask for a backoff, or it becomes a spin"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The `offset` each `getUpdates` asked for, in order.
    ///
    /// Read off the wire rather than off the client's own state: what this
    /// bridge REQUESTS is what decides whether Telegram can hand over the
    /// updates behind a stuck one.
    fn requested_offsets(fake: &Fake) -> Vec<i64> {
        fake.requests()
            .iter()
            .filter(|seen| seen.path.contains("getUpdates"))
            .filter_map(|seen| {
                let value = json::parse(&seen.body).ok()?;
                match value.get("offset")? {
                    Value::Num(offset) => Some(*offset),
                    _ => None,
                }
            })
            .collect()
    }

    /// Poll `n` times against a server that answers every poll with the same
    /// one update — which is exactly what Telegram does while the offset is
    /// held.
    fn poll_repeatedly(
        tag: &str,
        refusal: Refusal,
        text: &str,
        polls: usize,
    ) -> (PathBuf, Recorder, Vec<Cycle>, i64) {
        let dir = temp(tag);
        let held = from(3, text, SENDER);
        let mut script: Vec<Reply> = (0..polls)
            .map(|_| Reply::json(200, &result(std::slice::from_ref(&held))))
            .collect();
        // What the queue looks like once it is free again: the /use that the
        // deadlock argument says an operator must be able to get through.
        script.push(Reply::json(
            200,
            &result(&[from(4, "/use work codex:dev", SENDER)]),
        ));
        let fake = Fake::start(script);
        let api = fake.api();
        let recorder = Recorder::refusing(refusal);
        let mut inbox = Inbox::new(&dir, Knobs::default());
        let cycles = (0..polls)
            .map(|_| cycle(&mut inbox, &api, &recorder))
            .collect();
        let offset = load_offset(&inbox.offset_path()).unwrap();
        (dir, recorder, cycles, offset)
    }

    #[test]
    fn a_target_that_will_not_take_it_gives_up_at_the_bound_and_hands_the_message_back() {
        // RULING A, end to end. Retrying forever is not "never lose", it is a
        // deadlock: the held offset blocks every later update, INCLUDING the
        // /use that would redirect away from the stuck target. So the bound
        // ends the hold — by handing the message back, never by dropping it.
        let bound = Knobs::default().retry_max as usize;
        let (dir, recorder, cycles, offset) =
            poll_repeatedly("give-up", Refusal::Transient, "urgent thing", bound);

        // Every cycle BEFORE the bound holds, silently.
        for (n, held) in cycles.iter().take(bound - 1).enumerate() {
            assert!(
                matches!(held.failure, Some(CycleFailure::Undelivered(_))),
                "poll {n} should still be holding: {held:?}"
            );
            assert!(!held.retry_after.is_zero(), "a hold must ask for a backoff");
        }
        assert_eq!(
            recorder.delivered.lock().unwrap().len(),
            bound,
            "the bound counts ATTEMPTS, and every one of them must be a real attempt"
        );

        // The bound itself: one notice, the offset advanced, the queue live.
        let gave_up = cycles.last().unwrap();
        assert!(
            gave_up.failure.is_none(),
            "a give-up ENDS the failure — the cycle succeeded in handing the message back: {gave_up:?}"
        );
        assert_eq!(
            (gave_up.routed, gave_up.dropped, gave_up.undelivered),
            (0, 0, 1),
            "a give-up is neither a delivery nor a policy drop"
        );
        assert_eq!(
            offset, 3,
            "the give-up must ADVANCE the offset, or nothing later runs"
        );

        let said = recorder.said.lock().unwrap().clone();
        assert_eq!(said.len(), 1, "exactly ONE notice, at the bound: {said:?}");
        assert!(
            said[0].contains("claude:lead") && said[0].contains("work"),
            "the notice must NAME the target that would not take it: {said:?}"
        );
        assert!(
            said[0].contains("urgent thing"),
            "the notice must QUOTE THE MESSAGE BACK — that is what makes this a hand-back \
             rather than a silent loss: {said:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_give_up_notice_the_platform_refused_does_not_advance_the_offset() {
        // THE HAND-BACK IS THE CHECKPOINT'S PRECONDITION. A give-up steps over
        // an update on the strength of having handed the message back; if the
        // notice was never accepted, nothing was handed back, and advancing
        // anyway would be a silent loss with a notice-shaped comment next to
        // it.
        //
        // SCOPE: this is the DECISION — what the cycle does when the answer is
        // `No`, whatever produced it. Which wire shapes produce a `No` is a
        // question about the client, and it is answered against the real one in
        // `bridge::tests::the_confirmed_word_accepts_only_what_telegram_actually_took`.
        // Asserting it here too would be asserting it about a test double.
        let bound = Knobs::default().retry_max as usize;
        {
            let tag = "giveup-refused";
            let refused = Accepted::No;
            let dir = temp(tag);
            let held = from(3, "unheard", SENDER);
            let fake = Fake::start(
                (0..bound)
                    .map(|_| Reply::json(200, &result(std::slice::from_ref(&held))))
                    .collect(),
            );
            let api = fake.api();
            let recorder = Recorder {
                confirm: refused,
                ..Recorder::refusing(Refusal::Transient)
            };
            let mut inbox = Inbox::new(&dir, Knobs::default());
            let cycles: Vec<Cycle> = (0..bound)
                .map(|_| cycle(&mut inbox, &api, &recorder))
                .collect();

            let at_bound = cycles.last().unwrap();
            assert!(
                matches!(at_bound.failure, Some(CycleFailure::Undelivered(_))),
                "{tag}: an unheard give-up must leave the update owed: {at_bound:?}"
            );
            assert_eq!(
                at_bound.undelivered, 0,
                "{tag}: nothing was handed back, so nothing may be counted as handed back"
            );
            assert_eq!(
                load_offset(&inbox.offset_path()).unwrap(),
                0,
                "{tag}: THE OFFSET MUST NOT MOVE — this is the invariant"
            );
            assert!(
                !at_bound.retry_after.is_zero(),
                "{tag}: a still-owed update must back off rather than spin"
            );
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn the_give_up_retries_until_the_notice_is_accepted_and_advances_only_then() {
        // The other half of the same rule, and the reason a rejection is a HOLD
        // and not a drop: once the platform takes the notice, the give-up
        // completes and the queue moves. Nothing is lost by the wait.
        let bound = Knobs::default().retry_max as usize;
        let dir = temp("giveup-eventually");
        let held = from(3, "heard at last", SENDER);
        let fake = Fake::start(
            (0..=bound)
                .map(|_| Reply::json(200, &result(std::slice::from_ref(&held))))
                .collect(),
        );
        let api = fake.api();
        let mut inbox = Inbox::new(&dir, Knobs::default());

        // The platform is refusing: the bound is reached, and held.
        let unheard = Recorder::refusing_unheard(Refusal::Transient);
        for _ in 0..bound {
            cycle(&mut inbox, &api, &unheard);
        }
        assert_eq!(load_offset(&inbox.offset_path()).unwrap(), 0);
        let said = unheard.said.lock().unwrap().len();
        assert!(said >= 1, "the notice was attempted");

        // The platform recovers. The very next cycle completes the give-up.
        let heard = Recorder::refusing(Refusal::Transient);
        let freed = cycle(&mut inbox, &api, &heard);
        assert!(freed.failure.is_none(), "{freed:?}");
        assert_eq!((freed.routed, freed.dropped, freed.undelivered), (0, 0, 1));
        assert_eq!(
            load_offset(&inbox.offset_path()).unwrap(),
            3,
            "an ACCEPTED notice is what releases the checkpoint"
        );
        let notice = heard.said.lock().unwrap().clone();
        assert!(
            notice.last().unwrap().contains("heard at last"),
            "the accepted notice still quotes the message back: {notice:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_give_up_frees_the_queue_for_the_command_that_redirects_away_from_the_stuck_target() {
        // The deadlock argument, as a test — and the reason hold-forever was
        // rejected. While the offset is held, EVERY later update is blocked,
        // including the `/use` an operator would send to redirect away from the
        // target that is stuck. The release is only real if that command is
        // seen, so this asserts the effect it has and not merely the offset.
        let bound = Knobs::default().retry_max as usize;
        let dir = temp("give-up-frees");
        let held = from(3, "stuck", SENDER);
        let mut script: Vec<Reply> = (0..bound)
            .map(|_| Reply::json(200, &result(std::slice::from_ref(&held))))
            .collect();
        script.push(Reply::json(
            200,
            &result(&[from(4, "/use work codex:dev", SENDER)]),
        ));
        let fake = Fake::start(script);
        let api = fake.api();
        // Delivery is refused throughout: the pane never recovers, which is the
        // case hold-forever could not get out of.
        let recorder = Recorder::refusing(Refusal::Transient);
        let mut inbox = Inbox::new(&dir, Knobs::default());
        for _ in 0..bound {
            cycle(&mut inbox, &api, &recorder);
        }
        assert!(
            matches!(
                routing::read_sticky(&inbox.sticky_path()),
                routing::Sticky::Unset
            ),
            "nothing has redirected anything yet"
        );

        // THE RELEASE, at the only place it is really observable: the offset
        // this client now ASKS Telegram for. While the offset is held, every
        // poll re-requests the stuck update and Telegram can never hand over
        // what is behind it — the deadlock, stated as a wire fact. Asserting it
        // here rather than only through the scripted reply matters: this fake
        // answers its script in order, so a scripted `/use` arrives whether or
        // not the client asked past the update in front of it, and a test that
        // only read the reply would pass under hold-forever.
        let asked = requested_offsets(&fake);
        assert_eq!(
            asked.first().copied(),
            Some(1),
            "the first poll asks from the start: {asked:?}"
        );
        assert!(
            asked.iter().take(bound).all(|offset| *offset == 1),
            "while the update is owed, every poll must re-request IT: {asked:?}"
        );
        assert_eq!(
            *asked.last().unwrap(),
            1,
            "the give-up happens during the last of those polls: {asked:?}"
        );

        let freed = cycle(&mut inbox, &api, &recorder);
        let asked = requested_offsets(&fake);
        assert_eq!(
            *asked.last().unwrap(),
            4,
            "after the give-up the client must ask PAST the stuck update, or the queue is \
             still deadlocked: {asked:?}"
        );
        assert!(freed.failure.is_none(), "{freed:?}");
        assert_eq!(
            routing::read_sticky(&inbox.sticky_path()),
            routing::Sticky::Set {
                session: "work".to_owned(),
                agent: "codex:dev".to_owned(),
            },
            "the /use behind the stuck update must be reachable once the bound released it"
        );
        assert_eq!(load_offset(&inbox.offset_path()).unwrap(), 4);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_hard_refusal_reaches_the_give_up_sooner_than_a_transient_one() {
        // Two refusals are not the same fact. A busy pane recovers on its own
        // and earns the full bound; a pane that is provably gone does not, and
        // spending the long bound on it only delays the hand-back.
        let knobs = Knobs::default();
        assert!(
            knobs.hard_retry_max < knobs.retry_max,
            "a hard refusal must not wait as long as a transient one"
        );
        let (dir, recorder, cycles, offset) = poll_repeatedly(
            "give-up-hard",
            Refusal::Hard,
            "to a dead pane",
            knobs.hard_retry_max as usize,
        );
        let gave_up = cycles.last().unwrap();
        assert_eq!(
            (gave_up.routed, gave_up.dropped, gave_up.undelivered),
            (0, 0, 1),
            "the hard bound must have been reached: {gave_up:?}"
        );
        assert_eq!(offset, 3);
        let said = recorder.said.lock().unwrap().clone();
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].contains("pane is gone"),
            "the notice must say WHY it gave up early: {said:?}"
        );
        assert!(
            said[0].contains("to a dead pane"),
            "a hard give-up hands the message back too: {said:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_attempt_count_belongs_to_one_update_and_resets_when_another_owes() {
        // The counter is keyed by update id. Without that key, a second update
        // failing once would inherit the first one's exhausted count and be
        // given up on its FIRST attempt.
        let mut inbox = Inbox::new(temp("attempts"), Knobs::default());
        assert_eq!(inbox.attempt(7), 1);
        assert_eq!(inbox.attempt(7), 2);
        assert_eq!(inbox.attempt(9), 1, "a different update starts over");
        assert_eq!(inbox.attempt(9), 2);
    }

    #[test]
    fn an_unauthorized_update_is_stepped_over_rather_than_held() {
        // The mirror image of the test above, and the reason the two rules are
        // not one: holding on an unauthorized update would let any stranger who
        // finds the bot wedge the bridge forever.
        let dir = temp("cycle-stranger");
        let fake = Fake::start(vec![
            Reply::json(200, &result(&[from(5, "/list", "999")])),
            Reply::json(200, &result(&[])),
        ]);
        let api = fake.api();
        let recorder = Recorder::default();
        let mut inbox = Inbox::new(&dir, Knobs::default());
        let cycled = cycle(&mut inbox, &api, &recorder);
        assert!(cycled.failure.is_none(), "{cycled:?}");
        assert_eq!(
            (cycled.routed, cycled.dropped),
            (0, 1),
            "a policy drop must be counted as a drop and never as a routed message"
        );
        assert_eq!(load_offset(&inbox.offset_path()).unwrap(), 5);
        assert!(recorder.delivered.lock().unwrap().is_empty());
        assert!(
            recorder.said.lock().unwrap().is_empty(),
            "an unauthorized sender must learn nothing about ae"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Poll one hostile response with a known offset already on disk, and hand
    /// back what the cycle did plus where the offset ended up.
    fn hostile(tag: &str, reply: Reply) -> (Cycle, i64) {
        let dir = temp(tag);
        let fake = Fake::one(reply);
        let api = fake.api();
        let recorder = Recorder::default();
        let mut inbox = Inbox::new(&dir, Knobs::default());
        store_offset(&inbox.offset_path(), 5).unwrap();
        let cycled = cycle(&mut inbox, &api, &recorder);
        let offset = load_offset(&inbox.offset_path()).unwrap();
        assert!(
            recorder.delivered.lock().unwrap().is_empty(),
            "a hostile response delivered something"
        );
        std::fs::remove_dir_all(&dir).ok();
        (cycled, offset)
    }

    #[test]
    fn no_hostile_response_advances_the_offset() {
        // The whole "no state advancement on a bad response" rule, one row per
        // shape a public network can produce.
        let deep = format!(
            r#"{{"ok":true,"result":[{}{}]}}"#,
            "[".repeat(70),
            "]".repeat(70)
        );
        let cases = vec![
            ("malformed", Reply::json(200, r#"{"ok":true,"result":"#)),
            ("not-json", Reply::json(200, "<html>nope</html>")),
            ("server-error", Reply::json(500, r#"{"ok":false}"#)),
            ("client-error", Reply::json(400, r#"{"ok":false}"#)),
            (
                "ok-false",
                Reply::json(200, r#"{"ok":false,"description":"no"}"#),
            ),
            (
                "no-result-array",
                Reply::json(200, r#"{"ok":true,"result":{}}"#),
            ),
            ("nested-past-max-depth", Reply::json(200, &deep)),
            (
                "replayed-id",
                Reply::json(
                    200,
                    r#"{"ok":true,"result":[{"update_id":5,"message":{}}]}"#,
                ),
            ),
            (
                "id-below-the-one-we-asked-from",
                Reply::json(
                    200,
                    r#"{"ok":true,"result":[{"update_id":1,"message":{}}]}"#,
                ),
            ),
            (
                "id-too-large-for-i64",
                Reply::json(
                    200,
                    r#"{"ok":true,"result":[{"update_id":99999999999999999999999}]}"#,
                ),
            ),
        ];
        for (index, (label, reply)) in cases.into_iter().enumerate() {
            let (cycled, offset) = hostile(&format!("hostile-{index}"), reply);
            assert!(cycled.failure.is_some(), "{label} was not a failure");
            assert_eq!(offset, 5, "{label} moved the offset");
            assert!(
                !cycled.retry_after.is_zero(),
                "{label} did not ask for a backoff"
            );
        }
    }

    #[test]
    fn the_very_next_update_is_accepted_which_is_the_only_case_that_happens_normally() {
        // THE STEADY STATE, and the one the reversal guard could break without
        // any other test noticing: an id EXACTLY one past the stored offset is
        // what every consecutive Telegram update looks like. A guard written
        // `<=` instead of `<` would refuse the whole normal path while every
        // hostile-input test stayed green.
        let dir = temp("steady-state");
        let fake = Fake::one(Reply::json(200, &result(&[from(6, "next", SENDER)])));
        let api = fake.api();
        let recorder = Recorder::default();
        let mut inbox = Inbox::new(&dir, Knobs::default());
        store_offset(&inbox.offset_path(), 5).unwrap();

        let cycled = cycle(&mut inbox, &api, &recorder);
        assert!(cycled.failure.is_none(), "{cycled:?}");
        assert_eq!(cycled.routed, 1);
        assert_eq!(load_offset(&inbox.offset_path()).unwrap(), 6);
        assert!(
            fake.requests()[0].body.contains(r#""offset":6"#),
            "the poll must ask from one past what it stored: {}",
            fake.requests()[0].body
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_id_too_large_for_i64_is_kept_verbatim_rather_than_overflowed() {
        // The design's requirement, on the fields where `Value::Raw` is
        // REACHABLE: a chat or sender id past `i64` survives as the text it
        // was written as, is compared as text, and simply fails to match. No
        // wrap, no panic, and no accidental equality with a real id.
        let huge = "99999999999999999999999";
        let update = json::parse(&format!(
            r#"{{"update_id":4,"message":{{"text":"hi","from":{{"id":{huge}}},
               "chat":{{"id":{huge},"type":"private"}}}}}}"#
        ))
        .unwrap();
        let message = Update::parse(&update).unwrap().message.unwrap();
        assert_eq!(message.from_id, huge, "the id was not kept verbatim");
        assert_eq!(message.chat_id, huge);
        assert!(!policy().admits(&message));
        // And an allow-list that literally names it still fails the chat check,
        // so a verbatim id is not a way in.
        let permissive = Policy::new(CHAT_ID, vec![huge.to_owned()]);
        assert!(!permissive.admits(&message));
    }

    #[test]
    fn a_command_whose_effect_cannot_be_written_is_held_rather_than_reported_done() {
        // `/use clear` that cannot remove the file has NOT cleared anything.
        // Reporting success would leave the operator believing plain messages
        // go to the orchestrator while they still go to the override.
        let dir = temp("clear-fails");
        let mut inbox = Inbox::new(&dir, Knobs::default());
        let recorder = Recorder::default();
        durable_write(&inbox.sticky_path(), "work\tcodex:dev\n").unwrap();

        let handled = with_unwritable_dir(&dir, || {
            run(&mut inbox, &[&update("/use clear")], &recorder)
        });
        assert!(
            matches!(handled[0], super::Handled::Owed(_)),
            "a clear that did not happen must stay owed"
        );
        assert!(inbox.sticky_path().is_file(), "the override is still there");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_offset_errors_say_which_step_failed_and_keep_their_cause() {
        use super::OffsetError;
        use std::error::Error as _;

        let unrecognised = OffsetError::Unrecognised;
        assert_eq!(
            unrecognised.to_string(),
            "telegram offset: unrecognised contents"
        );
        assert!(unrecognised.source().is_none());
        let not_regular = OffsetError::NotRegular(PathBuf::from("/x/tg_offset"));
        assert!(
            not_regular.to_string().contains("/x/tg_offset")
                && not_regular.to_string().contains("not a regular file"),
            "{not_regular}"
        );
        let unwritten =
            OffsetError::NotWritten(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        assert!(
            unwritten
                .to_string()
                .starts_with("telegram offset: not written: ")
        );
        assert!(
            unwritten.source().is_some(),
            "a wrapped io error must stay reachable, or the cause is lost"
        );
        let unreadable =
            OffsetError::Unreadable(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        assert!(
            unreadable
                .to_string()
                .starts_with("telegram offset: unreadable: ")
        );
        assert!(unreadable.source().is_some());
    }

    #[test]
    fn a_batch_that_goes_backwards_stops_at_the_reversal_and_keeps_what_it_earned() {
        // The mixed case the single-update table cannot show: the first update
        // is genuinely new and IS routed and checkpointed; the second replays an
        // id below it. The offset must hold what it earned and refuse to go
        // back — a backwards checkpoint would re-deliver everything in between.
        let dir = temp("reversal");
        let fake = Fake::one(Reply::json(
            200,
            &result(&[from(9, "first", SENDER), from(7, "backwards", SENDER)]),
        ));
        let api = fake.api();
        let recorder = Recorder::default();
        let mut inbox = Inbox::new(&dir, Knobs::default());
        let cycled = cycle(&mut inbox, &api, &recorder);
        assert_eq!(cycled.failure, Some(CycleFailure::Malformed), "{cycled:?}");
        assert_eq!(cycled.routed, 1, "the legitimate update must still count");
        assert_eq!(
            recorder.delivered.lock().unwrap().len(),
            1,
            "the reversed update must not be delivered"
        );
        assert_eq!(
            load_offset(&inbox.offset_path()).unwrap(),
            9,
            "the offset must keep what it earned and never move backwards"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_oversized_body_is_capped_while_it_streams_and_the_offset_stays_put() {
        // Two shapes, and the difference is the point: one declares nothing and
        // streams forever, the other declares a length far past the cap. A
        // reader that trusted `Content-Length` would fail the first and
        // allocate the second.
        let endless = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (cycled, offset) = hostile(
            "oversized-chunked",
            Reply::Chunked {
                chunk: "x".repeat(8 * 1024),
                chunks: 4_096,
                written: std::sync::Arc::clone(&endless),
            },
        );
        assert!(cycled.failure.is_some(), "{cycled:?}");
        assert_eq!(offset, 5);

        let streamed = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (declared, offset) = hostile(
            "oversized-declared",
            Reply::Oversized {
                declared: 64 * 1024 * 1024,
                written: std::sync::Arc::clone(&streamed),
            },
        );
        assert!(declared.failure.is_some(), "{declared:?}");
        assert_eq!(offset, 5);
        assert!(
            streamed.load(std::sync::atomic::Ordering::Relaxed) < 64 * 1024 * 1024,
            "the whole declared body was streamed before the cap fired"
        );
    }

    #[test]
    fn the_token_is_in_the_url_and_in_nothing_this_cycle_can_print() {
        // The proof has to show both halves: that the secret really IS in the
        // request path (so the assertion below is about something), and that no
        // rendering of any failure this cycle produces carries it.
        let dir = temp("token");
        let fake = Fake::one(Reply::json(500, r#"{"ok":false}"#));
        let api = fake.api();
        let recorder = Recorder::default();
        let mut inbox = Inbox::new(&dir, Knobs::default());
        let cycled = cycle(&mut inbox, &api, &recorder);
        let asked = fake.requests();
        assert!(
            asked[0].path.contains(crate::telegram::tests::FAKE_TOKEN),
            "the fake never saw the token; this test is not testing anything"
        );
        assert!(asked[0].path.ends_with("/getUpdates"), "{}", asked[0].path);
        let shown = cycled.failure.map(|failure| failure.to_string()).unwrap();
        assert!(
            !shown.contains(crate::telegram::tests::FAKE_TOKEN),
            "{shown}"
        );
        assert!(!shown.contains("http"), "{shown}");
        assert!(shown.starts_with("POST getUpdates: "), "{shown}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_long_poll_never_asks_telegram_to_outlast_the_agents_own_ceiling() {
        // The relationship, not the number: a wait longer than the agent's
        // receive timeout would end every quiet poll as our own timeout, and the
        // loop would back off on its most normal outcome.
        let dir = temp("long-poll");
        let fake = Fake::one(Reply::json(200, &result(&[])));
        let api = fake.api();
        let recorder = Recorder::default();
        let mut inbox = Inbox::new(
            &dir,
            Knobs {
                long_poll: std::time::Duration::from_mins(10),
                ..Knobs::default()
            },
        );
        cycle(&mut inbox, &api, &recorder);
        assert!(
            fake.requests()[0].body.contains(r#""timeout":10"#),
            "the long poll was not clamped: {}",
            fake.requests()[0].body
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_poll_asks_only_for_messages_and_a_bounded_number_of_them() {
        let dir = temp("shape");
        let fake = Fake::one(Reply::json(200, &result(&[])));
        let api = fake.api();
        let recorder = Recorder::default();
        let mut inbox = Inbox::new(&dir, Knobs::default());
        cycle(&mut inbox, &api, &recorder);
        let asked = fake.requests();
        assert_eq!(asked[0].method, "POST");
        assert_eq!(
            asked[0].content_type.as_deref(),
            Some("application/json"),
            "{:?}",
            asked[0].content_type
        );
        for expected in [
            r#""limit":10"#,
            r#""allowed_updates":["message"]"#,
            r#""offset":1"#,
        ] {
            assert!(asked[0].body.contains(expected), "{}", asked[0].body);
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_empty_allow_list_never_reaches_the_network_at_all() {
        // SC-943 as a WIRE fact, not only a predicate: an outbound-only bridge
        // must not be long-polling.
        let dir = temp("outbound-only");
        let fake = Fake::one(Reply::json(200, &result(&[from(1, "hi", SENDER)])));
        let api = fake.api();
        let recorder = Recorder::default();
        let mut inbox = Inbox::new(&dir, Knobs::default());
        let cycled = inbox.poll(
            &api,
            &Policy::new(CHAT_ID, Vec::new()),
            &world(),
            &recorder,
            &recorder,
            NOW,
        );
        assert!(cycled.failure.is_none());
        assert!(
            fake.requests().is_empty(),
            "an outbound-only bridge polled anyway"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_offset_round_trips_and_an_absent_one_is_zero_rather_than_an_error() {
        let dir = temp("offset");
        let path = dir.join("tg_offset");
        assert_eq!(load_offset(&path).unwrap(), 0, "absence is a fresh start");
        store_offset(&path, 4_242).unwrap();
        assert_eq!(load_offset(&path).unwrap(), 4_242);
        // The frozen one-decimal-line format, so a bash-era file is inherited
        // rather than ignored.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "4242\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_damaged_offset_is_refused_rather_than_read_as_zero() {
        // The frozen bash falls back to 0 here, which asks Telegram for its
        // whole retained backlog — a wholesale replay, which is exactly the
        // bound this module promises not to break.
        for junk in ["", "  ", "-1\n", "12x\n", "nine\n", "1 2\n"] {
            assert_eq!(parse_offset(junk), None, "{junk:?}");
        }
        assert_eq!(parse_offset(" 77\n"), Some(77));
        let dir = temp("damaged");
        let path = dir.join("tg_offset");
        std::fs::write(&path, "not-a-number\n").unwrap();
        assert!(load_offset(&path).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_update_id_too_large_for_this_machine_refuses_the_update_without_panicking() {
        // `crate::json` keeps the literal verbatim rather than overflowing it,
        // which is what makes a refusal possible instead of a wrapped number
        // being checkpointed as a position.
        let huge = json::parse(r#"{"update_id":99999999999999999999999}"#).unwrap();
        assert!(matches!(huge.get("update_id"), Some(Value::Raw(_))));
        assert_eq!(Update::parse(&huge), None);
        // i64::MAX itself is holdable, and the offset arithmetic saturates.
        let max = json::parse(r#"{"update_id":9223372036854775807}"#).unwrap();
        assert_eq!(Update::parse(&max).unwrap().id, i64::MAX);
        assert_eq!(i64::MAX.saturating_add(1), i64::MAX);
    }

    #[test]
    fn a_string_id_is_not_a_number_and_never_matches_the_allow_list() {
        let update = json::parse(
            r#"{"update_id":1,"message":{"text":"hi","from":{"id":"42"},
                "chat":{"id":"-1001234567890","type":"private"}}}"#,
        )
        .unwrap();
        let parsed = Update::parse(&update).unwrap();
        let message = parsed.message.unwrap();
        assert_eq!(message.from_id, "", "a quoted id must not become an id");
        assert!(!policy().admits(&message));
    }

    #[test]
    fn the_trust_predicate_needs_all_four_of_its_conditions() {
        // SC-944a/b/c: each of these is one door, and each must close alone.
        let good = message("hello");
        assert!(policy().admits(&good));
        let mut wrong_chat = good.clone();
        wrong_chat.chat_id = "-100999".to_owned();
        let mut group = good.clone();
        group.chat_type = "group".to_owned();
        let mut stranger = good.clone();
        stranger.from_id = "7".to_owned();
        let mut not_numeric = good.clone();
        not_numeric.from_id = "4 2".to_owned();
        let mut nameless = good.clone();
        nameless.from_id = String::new();
        for (label, bad) in [
            ("wrong chat", wrong_chat),
            ("group chat", group),
            ("sender not on the allow-list", stranger),
            ("non-numeric sender", not_numeric),
            ("no sender", nameless),
        ] {
            assert!(!policy().admits(&bad), "{label} was admitted");
        }
    }

    #[test]
    fn an_empty_allow_list_is_an_outbound_only_bridge_and_not_a_permissive_one() {
        // SC-943.
        let off = Policy::new(CHAT_ID, Vec::new());
        assert!(!off.enabled());
        assert!(!off.admits(&message("hello")));
    }

    fn run(inbox: &mut Inbox, updates: &[&str], recorder: &Recorder) -> Vec<super::Handled> {
        let world = world();
        updates
            .iter()
            .map(|raw| {
                let value = json::parse(raw).unwrap();
                let update = Update::parse(&value).unwrap();
                inbox.handle(&update, &policy(), &world, recorder, recorder, NOW)
            })
            .collect()
    }

    fn update(text: &str) -> String {
        format!(
            r#"{{"update_id":1,"message":{{"text":{},"from":{{"id":42}},
               "chat":{{"id":-1001234567890,"type":"private"}}}}}}"#,
            Value::str(text).render()
        )
    }

    #[test]
    fn an_authorized_plain_message_reaches_the_orchestrator_through_the_send_helper() {
        let dir = temp("route");
        let mut inbox = Inbox::new(&dir, Knobs::default());
        let recorder = Recorder::default();
        let handled = run(&mut inbox, &[&update("do the thing")], &recorder);
        assert!(matches!(handled[0], super::Handled::Routed));
        let delivered = recorder.delivered.lock().unwrap().clone();
        assert_eq!(
            delivered,
            vec![(
                PathBuf::from("/sessions/work"),
                "claude:lead".to_owned(),
                "do the thing".to_owned(),
                SENDER.to_owned(),
            )]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_refused_delivery_leaves_the_update_owed_and_the_chat_is_told_nothing_yet() {
        // Inside the bound a refusal is LOGGED, not announced (SC-936): the
        // cycle carries the failure and the chat stays quiet, because a bridge
        // that narrates every retry of a wedged pane is a chat nobody reads.
        // The one notice comes at the bound, and only there.
        let dir = temp("owed");
        let mut inbox = Inbox::new(&dir, Knobs::default());
        let recorder = Recorder::refusing(Refusal::Transient);
        let handled = run(&mut inbox, &[&update("hello")], &recorder);
        assert!(
            matches!(handled[0], super::Handled::Owed(_)),
            "a refused delivery must stay owed, not be reported and dropped"
        );
        assert!(
            recorder.said.lock().unwrap().is_empty(),
            "a refusal inside the bound must not announce itself"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_ask_verb_reaches_the_tracked_helper_and_is_never_downgraded_to_send() {
        // `/session <ref> ask <agent> <msg>` promises a TRACKED REQUEST: a
        // request id, an `ask` event, and a reply command embedded in the
        // message — which is the route the agent's answer takes back to the
        // chat. An earlier version validated the verb and then ran `send`
        // anyway, so the request was never opened and the documented reply
        // route could not happen. The operator was told it worked.
        let dir = temp("verb");
        let mut inbox = Inbox::new(&dir, Knobs::default());
        let recorder = Recorder::default();
        let handled = run(
            &mut inbox,
            &[
                &update("/session work ask codex:dev please review"),
                &update("/session work send codex:dev just telling you"),
            ],
            &recorder,
        );
        assert!(matches!(handled[0], super::Handled::Routed));
        assert_eq!(
            *recorder.verbs.lock().unwrap(),
            vec![Verb::Ask, Verb::Send],
            "the verb the operator typed must be the verb that ran"
        );
        // And the two verbs must not merely differ in a field — they must
        // select DIFFERENT HELPERS, which is what makes one tracked.
        assert_eq!(Verb::Ask.helper(), "ask");
        assert_eq!(Verb::Send.helper(), "send");
        let said = recorder.said.lock().unwrap().clone();
        assert!(
            said[0].contains("ask opened"),
            "the acknowledgement must not call an ask a send: {said:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unauthorized_update_is_dropped_silently_and_owes_nothing() {
        let dir = temp("stranger");
        let mut inbox = Inbox::new(&dir, Knobs::default());
        let recorder = Recorder::default();
        let hostile = r#"{"update_id":9,"message":{"text":"/list","from":{"id":666},
            "chat":{"id":-1001234567890,"type":"private"}}}"#;
        let handled = run(&mut inbox, &[hostile], &recorder);
        assert!(
            matches!(handled[0], super::Handled::Dropped),
            "a policy drop must be REPORTED as a drop, not counted as a routed message"
        );
        assert!(
            recorder.said.lock().unwrap().is_empty(),
            "an unauthorized sender must learn nothing"
        );
        assert!(recorder.delivered.lock().unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn use_writes_the_override_and_clear_removes_it() {
        // SC-939e, through the real files.
        let dir = temp("use");
        let mut inbox = Inbox::new(&dir, Knobs::default());
        let recorder = Recorder::default();
        run(&mut inbox, &[&update("/use work dev")], &recorder);
        assert_eq!(
            std::fs::read_to_string(inbox.sticky_path()).unwrap(),
            "work\tcodex:dev\n",
            "the override must record the CANONICAL agent"
        );
        // And it takes effect.
        run(&mut inbox, &[&update("ping")], &recorder);
        let delivered = recorder.delivered.lock().unwrap().clone();
        assert_eq!(delivered.last().unwrap().1, "codex:dev");

        run(&mut inbox, &[&update("/use clear")], &recorder);
        assert!(!inbox.sticky_path().is_file());
        run(&mut inbox, &[&update("ping again")], &recorder);
        let delivered = recorder.delivered.lock().unwrap().clone();
        assert_eq!(
            delivered.last().unwrap().1,
            "claude:lead",
            "clear must restore orchestrator routing"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clearing_an_override_that_is_not_there_is_success_and_not_a_hold() {
        let dir = temp("clear-absent");
        let mut inbox = Inbox::new(&dir, Knobs::default());
        let recorder = Recorder::default();
        let handled = run(&mut inbox, &[&update("/use clear")], &recorder);
        assert!(matches!(handled[0], super::Handled::Routed));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_cycle_reports_its_own_backoff_and_a_quiet_one_asks_for_none() {
        let dir = temp("backoff");
        let mut inbox = Inbox::new(&dir, Knobs::default());
        let first = inbox.failed(CycleFailure::Malformed);
        assert_eq!(first.retry_after, crate::telegram::backoff_delay(1));
        let second = inbox.failed(CycleFailure::Malformed);
        assert_eq!(second.retry_after, crate::telegram::backoff_delay(2));
        assert_eq!(
            Inbox::quiet(),
            Cycle {
                routed: 0,
                dropped: 0,
                undelivered: 0,
                failure: None,
                retry_after: std::time::Duration::ZERO,
            }
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
