//! Auto reseat: move a seat PROVEN stuck on its vendor usage limit, in place,
//! to the first usable profile the human declared for it, and say so.
//!
//! This file holds the DECISIONS and reads nothing but its arguments and the
//! global config: which limit episode a seat is in, whether that episode is due
//! for a move, and which declared candidate to take. The watchdog asks them to
//! trigger, and the legs that act ask them again, so a forged trigger can do no
//! more than the daemon would do at that moment.
//!
//! # The episode
//!
//! One episode is the journal suffix that starts at the first `limit` the
//! watchdog booked for a seat after the seat's newest `alert-cleared` or
//! `spawn`. Its KEY is that `limit` record's timestamp: a daemon restart books
//! `limit` again without an `alert-cleared`, so the key survives the restart. A
//! `reseat` record is no boundary, because the move itself writes one before
//! its outcome. Every record this path writes carries the key as its `ref`,
//! except `auto-reseat-done`, whose `ref` is the profile the seat LEFT.
//!
//! The fold compares action words, the seat's identity and whole `ref` values
//! by equality. It never splits or parses a `ref` or a `summary`.

use std::path::Path;

use crate::config::SectionEntry;
use crate::events::Event;
use crate::harness_state::HarnessState;
use crate::quota::Status;
use crate::time::Timestamp;
use crate::watchdog::{WATCHDOG_ACTOR, event_is_addressed_to};

pub(crate) mod leg;

/// An attempt: the trigger leg is about to start the move. `ref` = the key.
pub const ATTEMPT_ACTION: &str = "auto-reseat";
/// A hold before an attempt, or a transient refusal of one. `ref` = the key.
pub const HELD_ACTION: &str = "auto-reseat-held";
/// The seat moved. `ref` = the profile it left.
pub const DONE_ACTION: &str = "auto-reseat-done";
/// The move was refused and the episode gets no further attempt. `ref` = the key.
pub const REFUSED_ACTION: &str = "auto-reseat-refused";
/// The move failed and the episode gets no further attempt. `ref` = the key.
pub const FAILED_ACTION: &str = "auto-reseat-failed";
/// A notice sent to a recipient about an outcome.
pub const NOTICE_ACTION: &str = "auto-reseat-notice";

/// Attempts one episode may make: one, and one more after a transient hold.
pub const MAX_ATTEMPTS: u8 = 2;
/// How long an attempt may run before its failure is booked without it.
pub const IN_FLIGHT_SECS: i64 = 180;
/// The grace a limit is given before the seat is moved.
pub const DEFAULT_GRACE_SECS: u64 = 600;

const LIMIT_ACTION: &str = "limit";
const CLEARED_ACTION: &str = "alert-cleared";
const SPAWN_ACTION: &str = "spawn";

/// A judged window at or past this percentage has no headroom left.
const EXHAUSTED_PERCENT: f64 = 100.0;

/// How every note that turns the path off begins.
const OFF: &str = "auto reseat stays off: ";

/// The global `[workspace] auto_reseat` switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    /// Nothing moves.
    Off,
    /// Fixed non-main seats and spawned seats move.
    On,
    /// The main seat moves too.
    All,
}

/// Everything the global config says about auto reseat, judged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub switch: Switch,
    /// The sessions auto reseat acts in; `None` is every session.
    pub sessions: Option<Vec<String>>,
    pub grace_secs: u64,
    /// `[auto_reseat]`: a profile and the candidates it may move to, in the
    /// order declared.
    pub map: Vec<(String, Vec<String>)>,
    /// One line per entry ignored or knob refused, for the operator.
    pub notes: Vec<String>,
}

impl Settings {
    fn off(notes: Vec<String>) -> Self {
        Self {
            switch: Switch::Off,
            sessions: None,
            grace_secs: DEFAULT_GRACE_SECS,
            map: Vec::new(),
            notes,
        }
    }

    /// The candidates declared for `profile`, in order.
    #[must_use]
    pub fn candidates(&self, profile: &str) -> Option<&[String]> {
        self.map
            .iter()
            .find(|(key, _)| key == profile)
            .map(|(_, list)| list.as_slice())
    }
}

/// [`settings`] over the text of `file`, already read: the one parser, and the
/// entry the config fuzz target drives. `file` only names the text in a note.
#[must_use]
pub fn settings_in(file: &Path, text: &str) -> Settings {
    let read = |key| crate::config::workspace_key_in(file, text, key);
    match parse_switch(read("auto_reseat")) {
        Ok(Switch::Off) => Settings::off(Vec::new()),
        Err(note) => Settings::off(vec![note]),
        Ok(switch) => settle(
            switch,
            read("auto_reseat_sessions"),
            read("auto_reseat_grace_secs"),
            crate::config::section_entries(file, text, "auto_reseat"),
        ),
    }
}

/// Read the GLOBAL config now. A project overlay never steers spend, and the
/// switch is read on every decision, so turning it off stops the next move.
/// Nothing past an off switch is read or judged.
#[must_use]
pub fn settings(global: Option<&Path>) -> Settings {
    let Some(file) = global else {
        return Settings::off(Vec::new());
    };
    match crate::config::read_global_text(file) {
        Ok(Some(text)) => settings_in(file, &text),
        Ok(None) => Settings::off(Vec::new()),
        Err(why) => Settings::off(vec![format!("{OFF}{why}")]),
    }
}

/// Judge the knobs behind a switch that is on. A knob ae cannot use turns the
/// whole path off with its one note: a move is spend, and a guess is not a
/// ruling. An entry it cannot use is dropped with a note of its own.
fn settle(
    switch: Switch,
    sessions: Result<Option<String>, String>,
    grace: Result<Option<String>, String>,
    map: Result<Vec<SectionEntry>, String>,
) -> Settings {
    let refused = |why: String| Settings::off(vec![format!("{OFF}{why}")]);
    let mut notes = Vec::new();
    let sessions = match sessions {
        Err(why) => return refused(why),
        Ok(None) => None,
        Ok(Some(raw)) => {
            let (names, ignored) = crate::config::fleet_order_entries(&raw);
            notes.extend(ignored.iter().map(|entry| {
                format!(
                    "auto_reseat_sessions: {entry:?} ignored: not a session name, or named twice"
                )
            }));
            Some(names)
        }
    };
    let grace_secs = match grace {
        Err(why) => return refused(why),
        Ok(None) => DEFAULT_GRACE_SECS,
        Ok(Some(raw)) => match raw.trim().parse::<u64>() {
            Ok(secs) => secs,
            Err(_) => {
                return refused(format!(
                    "auto_reseat_grace_secs = {raw:?} is not a whole number of seconds"
                ));
            }
        },
    };
    let map = match map {
        Err(why) => return refused(why),
        Ok(entries) => candidate_map(&entries, &mut notes),
    };
    Settings {
        switch,
        sessions,
        grace_secs,
        map,
        notes,
    }
}

/// The `[auto_reseat]` rows ae can use, each ignored entry named in `notes`.
/// A profile keyed twice keeps its later row, in that row's place.
fn candidate_map(entries: &[SectionEntry], notes: &mut Vec<String>) -> Vec<(String, Vec<String>)> {
    let mut map: Vec<(String, Vec<String>)> = Vec::new();
    for entry in entries {
        let at = format!("[auto_reseat] line {}", entry.line);
        let key = entry.key.as_str();
        if !crate::config::is_config_key(key) {
            notes.push(format!("{at}: {key:?} is not a profile name, ignored"));
            continue;
        }
        let Some(value) = entry.value.as_deref() else {
            notes.push(format!("{at}: {key} names no candidate list, ignored"));
            continue;
        };
        let mut list: Vec<String> = Vec::new();
        for word in value
            .split(',')
            .map(str::trim)
            .filter(|word| !word.is_empty())
        {
            let why = if !crate::config::is_config_key(word) {
                "is not a profile name"
            } else if word == key {
                "is the profile itself"
            } else if list.iter().any(|taken| taken == word) {
                "is named twice"
            } else {
                list.push(word.to_owned());
                continue;
            };
            notes.push(format!("{at}: {key} candidate {word:?} {why}, ignored"));
        }
        if list.is_empty() {
            notes.push(format!("{at}: {key} keeps no usable candidate, ignored"));
            continue;
        }
        if let Some(earlier) = map.iter().position(|(taken, _)| taken == key) {
            map.remove(earlier);
            notes.push(format!("{at}: {key} is keyed twice, this line wins"));
        }
        map.push((key.to_owned(), list));
    }
    map
}

/// The switch as written, or the note that keeps the path off. A value is
/// echoed escaped, so a note carries no control byte into the pane it lands in.
fn parse_switch(raw: Result<Option<String>, String>) -> Result<Switch, String> {
    match raw
        .map_err(|why| format!("{OFF}{why}"))?
        .as_deref()
        .map(str::trim)
    {
        None | Some("off") => Ok(Switch::Off),
        Some("on") => Ok(Switch::On),
        Some("all") => Ok(Switch::All),
        Some(other) => Err(format!(
            "{OFF}auto_reseat = {other:?} is not off, on or all"
        )),
    }
}

/// What a roster slot is, for the switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatClass {
    Main,
    Fixed,
    Spawned,
}

impl SeatClass {
    /// The class of a routing slot, or `None` for anything else.
    #[must_use]
    pub fn of(slot: &str) -> Option<Self> {
        if !crate::requests::is_slot(slot) {
            return None;
        }
        Some(if slot == "main" {
            Self::Main
        } else if slot.starts_with("worker.") {
            Self::Fixed
        } else {
            Self::Spawned
        })
    }
}

/// The seat a decision is about.
#[derive(Debug, Clone, Copy)]
pub struct Seat<'a> {
    pub session: &'a str,
    pub slot: &'a str,
    pub agent: &'a str,
    /// The profile the seat records now.
    pub profile: &'a str,
    /// The session is an orchestrator's.
    pub orchestrator: bool,
}

/// Why a seat may not be moved at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ineligible {
    Off,
    Orchestrator,
    Session,
    Slot,
    Class,
    Unmapped,
}

/// The candidates `seat` may move to, or why it may not move. Reads no quota
/// and no frame: this is the whole of "auto-eligible".
///
/// # Errors
///
/// The reason the seat is not eligible.
pub fn eligible<'s>(settings: &'s Settings, seat: &Seat<'_>) -> Result<&'s [String], Ineligible> {
    if settings.switch == Switch::Off {
        return Err(Ineligible::Off);
    }
    if seat.orchestrator {
        return Err(Ineligible::Orchestrator);
    }
    if settings
        .sessions
        .as_ref()
        .is_some_and(|names| !names.iter().any(|name| name == seat.session))
    {
        return Err(Ineligible::Session);
    }
    match (SeatClass::of(seat.slot), settings.switch) {
        (None, _) => return Err(Ineligible::Slot),
        (Some(SeatClass::Main), Switch::On) => return Err(Ineligible::Class),
        _ => {}
    }
    settings
        .candidates(seat.profile)
        .ok_or(Ineligible::Unmapped)
}

/// What ended an episode's auto path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Refused,
    Failed,
}

/// One limit episode of one seat, as the journal records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Episode {
    /// The `ts` of the episode's first `limit` record.
    pub key: Timestamp,
    reference: String,
    pub attempts: u8,
    /// The newest attempt that has no outcome yet.
    pub open: Option<Timestamp>,
    /// The first terminal outcome; it ends the episode's auto path.
    pub terminal: Option<Outcome>,
    /// The newest record of this path is a hold.
    pub held: bool,
}

impl Episode {
    /// The key as every record of this episode spells its `ref`.
    #[must_use]
    pub fn reference(&self) -> &str {
        &self.reference
    }

    fn opened(key: Timestamp) -> Self {
        Self {
            key,
            reference: key.to_string(),
            attempts: 0,
            open: None,
            terminal: None,
            held: false,
        }
    }

    /// Fold one of the watchdog's records addressed to the seat. A record
    /// whose `ref` is not this episode's key belongs to another episode, except
    /// `auto-reseat-done`, which names the profile left and ends the path
    /// whenever it lands: the seat moved.
    fn absorb(&mut self, event: &Event) {
        let ours = event.reference.as_deref() == Some(self.reference.as_str());
        match event.action.as_str() {
            ATTEMPT_ACTION if ours => {
                self.attempts = self.attempts.saturating_add(1);
                self.open = Some(event.ts);
                self.held = false;
            }
            HELD_ACTION if ours => {
                self.open = None;
                self.held = true;
            }
            DONE_ACTION => self.close(Outcome::Done),
            REFUSED_ACTION if ours => self.close(Outcome::Refused),
            FAILED_ACTION if ours => self.close(Outcome::Failed),
            _ => {}
        }
    }

    /// The first terminal outcome wins; a later one closes nothing new.
    fn close(&mut self, outcome: Outcome) {
        self.open = None;
        self.held = false;
        self.terminal.get_or_insert(outcome);
    }
}

/// The episode the seat is in now, or `None` when it has none.
#[must_use]
pub fn episode(events: &[Event], session: &str, slot: &str, agent: &str) -> Option<Episode> {
    let mut current: Option<Episode> = None;
    for event in events {
        if !event_is_addressed_to(event, session, slot, agent) {
            continue;
        }
        if event.action == SPAWN_ACTION {
            current = None;
            continue;
        }
        if event.actor != WATCHDOG_ACTOR {
            continue;
        }
        match event.action.as_str() {
            CLEARED_ACTION => current = None,
            LIMIT_ACTION => {
                if current.is_none() {
                    current = Some(Episode::opened(event.ts));
                }
            }
            _ => {
                if let Some(open) = current.as_mut() {
                    open.absorb(event);
                }
            }
        }
    }
    current
}

/// Each profile this seat left by auto reseat since its newest `spawn`, with
/// the time of the newest such move, ordered by that move, oldest first.
#[must_use]
pub fn left_profiles(
    events: &[Event],
    session: &str,
    slot: &str,
    agent: &str,
) -> Vec<(String, Timestamp)> {
    let mut left: Vec<(String, Timestamp)> = Vec::new();
    for event in events {
        if !event_is_addressed_to(event, session, slot, agent) {
            continue;
        }
        if event.action == SPAWN_ACTION {
            left.clear();
            continue;
        }
        if event.actor != WATCHDOG_ACTOR || event.action != DONE_ACTION {
            continue;
        }
        if let Some(profile) = event.reference.as_deref() {
            left.retain(|(taken, _)| taken != profile);
            left.push((profile.to_owned(), event.ts));
        }
    }
    left
}

/// What the latest capture of the seat's pane proved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame {
    /// Read, idle, with an empty input box.
    Clear,
    Busy,
    /// A human's text sits in the input box.
    Draft,
    /// The capture failed: an absence of evidence.
    Unread,
}

/// The live facts about the seat's pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pane {
    pub frame: Frame,
    /// A prompt only the human may answer is latched.
    pub human_prompt: bool,
    /// When a tmux client last gave input in the pane.
    pub client_input: Option<i64>,
}

/// Why a due seat is not moved this cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldReason {
    Unread,
    Busy,
    Draft,
    HumanPrompt,
    ClientInput,
}

impl HoldReason {
    /// The summary of the `auto-reseat-held` record naming it.
    #[must_use]
    pub const fn summary(self) -> &'static str {
        match self {
            Self::Unread => "held: the pane could not be read",
            Self::Busy => "held: the seat is busy",
            Self::Draft => "held: a human draft sits in the input box",
            Self::HumanPrompt => "held: the seat waits on a prompt only the human may answer",
            Self::ClientInput => "held: a human gave input in the pane",
        }
    }
}

/// What to do for a latched, eligible seat this cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Nothing more this episode: none, a terminal outcome, or attempts spent.
    Rest,
    /// Not due before this epoch.
    Wait {
        due_at: i64,
    },
    Hold(HoldReason),
    Attempt,
    /// An attempt runs inside its bound.
    InFlight,
    /// An attempt passed its bound with no outcome: book its failure.
    Overdue,
}

/// Decide for one seat whose limit latch stands.
///
/// An open attempt is judged before the spent count, so the second attempt
/// keeps its bound; the grace is judged before the pane, so nothing about the
/// pane is read into a hold while the human still has time to react.
#[must_use]
pub fn decide(episode: Option<&Episode>, grace_secs: u64, pane: &Pane, now: i64) -> Decision {
    let Some(episode) = episode.filter(|found| found.terminal.is_none()) else {
        return Decision::Rest;
    };
    if let Some(started) = episode.open {
        return if now.saturating_sub(started.epoch()) < IN_FLIGHT_SECS {
            Decision::InFlight
        } else {
            Decision::Overdue
        };
    }
    if episode.attempts >= MAX_ATTEMPTS {
        return Decision::Rest;
    }
    let grace = i64::try_from(grace_secs).unwrap_or(i64::MAX);
    let key = episode.key.epoch();
    let due_at = key.saturating_add(grace);
    if now < due_at {
        return Decision::Wait { due_at };
    }
    // Input at or before the key has aged past the grace by the time the seat
    // is due, so only input after the limit can still hold it.
    let touched = pane
        .client_input
        .is_some_and(|at| now < at.saturating_add(grace));
    let reason = match pane.frame {
        Frame::Unread => Some(HoldReason::Unread),
        Frame::Busy => Some(HoldReason::Busy),
        Frame::Draft => Some(HoldReason::Draft),
        Frame::Clear if pane.human_prompt => Some(HoldReason::HumanPrompt),
        Frame::Clear if touched => Some(HoldReason::ClientInput),
        Frame::Clear => None,
    };
    reason.map_or(Decision::Attempt, Decision::Hold)
}

/// The frame one capture proves: a read that failed proves nothing, a running
/// turn is busy, and a human's text in the box is theirs.
#[must_use]
pub fn frame_of(read: bool, state: HarnessState, draft: bool) -> Frame {
    if !read {
        Frame::Unread
    } else if state == HarnessState::Busy {
        Frame::Busy
    } else if draft {
        Frame::Draft
    } else {
        Frame::Clear
    }
}

/// Whether the seat's episode in `events` has an attempt running inside its
/// bound. Off, nothing is folded and nothing is in flight.
#[must_use]
pub fn in_flight(
    settings: &Settings,
    events: &[Event],
    (session, slot, agent): (&str, &str, &str),
    now: i64,
) -> bool {
    // The attempt is judged before the pane, so no pane is read into it.
    let unread = Pane {
        frame: Frame::Unread,
        human_prompt: false,
        client_input: None,
    };
    settings.switch != Switch::Off
        && decide(
            episode(events, session, slot, agent).as_ref(),
            settings.grace_secs,
            &unread,
            now,
        ) == Decision::InFlight
}

/// Whether a hold (or, with `overdue`, an overdue failure) is still owed to the
/// episode keyed `key`, as the journal reads NOW: under the seat's lock, the
/// last check before the record is written.
#[must_use]
pub fn owed(episode: Option<&Episode>, key: Timestamp, overdue: bool, now: i64) -> bool {
    episode
        .filter(|found| found.key == key && found.terminal.is_none())
        .is_some_and(|found| match found.open {
            None => !overdue,
            Some(started) => overdue && now.saturating_sub(started.epoch()) >= IN_FLIGHT_SECS,
        })
}

/// The lock every writer of this path's records takes for `slot`, without
/// waiting: a writer that finds it held skips, and the next cycle asks again.
/// It is taken BEFORE the journal is re-read and appended to, never after.
#[must_use]
pub fn lock_path(dir: &Path, slot: &str) -> std::path::PathBuf {
    dir.join(format!("auto-reseat.{slot}.lock"))
}

/// A candidate's standing, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// Every fresh window judged below critical.
    BelowCritical,
    /// No fresh window to judge by.
    Unknown,
    /// A fresh window judged critical.
    Critical,
}

/// One quota window that binds a candidate, judged by the quota module.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub judged: f64,
    /// The one classifier put this window at critical.
    pub critical: bool,
    pub observed_at: i64,
    pub resets_at: Option<i64>,
    pub status: Status,
}

/// What the leg knows about one declared candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub profile: String,
    /// The profile resolves the way a launch resolves one.
    pub configured: bool,
    /// Another seat of the session is latched on this candidate's account.
    pub peer_latched: bool,
    pub windows: Vec<Window>,
    /// When this seat last left this profile by auto reseat.
    pub left_at: Option<i64>,
}

/// Why a candidate was passed over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    Unconfigured,
    PeerLatched,
    Exhausted,
    LeftOnLimit,
}

/// The candidate taken, and every one passed over with its reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub pick: Option<(String, Tier)>,
    pub skipped: Vec<(String, Skip)>,
}

/// Take the first usable candidate of the best tier, in declared order.
#[must_use]
pub fn choose(candidates: &[Candidate], now: i64) -> Choice {
    let mut skipped = Vec::new();
    let mut best: Option<(&Candidate, Tier)> = None;
    for candidate in candidates {
        if let Some(skip) = passed_over(candidate, now) {
            skipped.push((candidate.profile.clone(), skip));
            continue;
        }
        let tier = tier(&candidate.windows);
        if best.is_none_or(|(_, held)| tier < held) {
            best = Some((candidate, tier));
        }
    }
    Choice {
        pick: best.map(|(candidate, tier)| (candidate.profile.clone(), tier)),
        skipped,
    }
}

/// Why `candidate` cannot be taken now, if it cannot.
///
/// A profile the seat left on its limit waits for a window read after the move
/// that proves relief — a usable reading below critical, or a reset that has
/// passed — and for no usable reading after the move to still sit at critical:
/// a move back onto a nearly spent account would only move the seat again. A
/// number ae cannot use proves nothing either way. A spend cap needs no rule
/// of its own, because it judges as exhausted.
fn passed_over(candidate: &Candidate, now: i64) -> Option<Skip> {
    if !candidate.configured {
        return Some(Skip::Unconfigured);
    }
    if candidate.peer_latched {
        return Some(Skip::PeerLatched);
    }
    if let Some(left) = candidate.left_at {
        let mut relieved = false;
        for window in candidate
            .windows
            .iter()
            .filter(|window| window.observed_at > left)
        {
            if window.resets_at.is_some_and(|at| at <= now) {
                relieved = true;
            } else if usable(window) {
                if window.critical {
                    return Some(Skip::LeftOnLimit);
                }
                relieved = true;
            }
        }
        if !relieved {
            return Some(Skip::LeftOnLimit);
        }
    }
    candidate
        .windows
        .iter()
        .filter(|window| usable(window))
        .any(|window| window.judged >= EXHAUSTED_PERCENT)
        .then_some(Skip::Exhausted)
}

/// A window whose number still stands: read before its reset. The status
/// carries that, because `quota::freshness` reads a passed reset as unknown.
fn usable(window: &Window) -> bool {
    matches!(window.status, Status::Fresh | Status::Stale)
}

/// The tier a candidate's windows put it in. Only a fresh window is known.
fn tier(windows: &[Window]) -> Tier {
    let mut fresh = windows
        .iter()
        .filter(|window| window.status == Status::Fresh)
        .peekable();
    if fresh.peek().is_none() {
        Tier::Unknown
    } else if fresh.any(|window| window.critical) {
        Tier::Critical
    } else {
        Tier::BelowCritical
    }
}

/// The quota identity of every OTHER seat of `session` still on its limit: a
/// `limit` record since its last clear, whatever became of its auto path.
///
/// The seat being moved is left out. Its own limit may bind only its model
/// family, and a window that binds its whole account still reads exhausted on
/// the candidate itself.
pub(crate) fn latched_identities(
    roster: &[crate::meta::RosterEntry],
    events: &[Event],
    session: &str,
    moving: &str,
) -> Vec<crate::quota::RecordedIdentity> {
    let _ = (roster, events, session, moving);
    Vec::new()
}

/// Who opened the seat: the actor of the newest `spawn` record addressed to it.
#[must_use]
pub fn spawner<'e>(events: &'e [Event], session: &str, slot: &str, agent: &str) -> Option<&'e str> {
    let _ = (events, session, slot, agent);
    None
}

/// Who is told how an attempt ended: the lead pair without the seat at
/// `moved`, then `spawner` when it names another seat of the roster that is not
/// told already.
#[must_use]
pub fn recipients(
    roster: &[crate::meta::RosterEntry],
    lead_pair: bool,
    moved: &str,
    spawner: Option<&str>,
) -> Vec<String> {
    let _ = (roster, lead_pair, moved, spawner);
    Vec::new()
}

/// How one ending of the auto path is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending<'a> {
    Moved(Move<'a>),
    /// Held, quoting the record's summary.
    Held(&'a str),
    Refused(&'a str),
    Failed(&'a str),
}

/// What a move changed, as its notice names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Move<'a> {
    pub from: &'a str,
    pub to: &'a str,
    /// The tool before and after.
    pub tool: (&'a str, &'a str),
    /// The model before and after.
    pub model: (&'a str, &'a str),
    /// The conversation went with the seat.
    pub carried: bool,
    /// The target was already critical when it was chosen.
    pub critical: bool,
    /// The seat's work tree has tracked changes.
    pub dirty: bool,
}

/// The widest a notice may be, in characters.
pub const NOTICE_CHARS: usize = 400;

/// The ONE line every recipient and the chat are told for `ending`.
#[must_use]
pub fn notice(session: &str, agent: &str, ending: &Ending<'_>) -> String {
    let _ = (session, agent, ending);
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = "aedev";
    const SLOT: &str = "spawned.3";
    const AGENT: &str = "builder";
    /// The episode key every specimen below is timed against.
    const KEY: &str = "2026-09-25T10:00:00Z";
    /// Another episode's key.
    const OTHER_KEY: &str = "2026-09-25T09:00:00Z";

    fn key_epoch() -> i64 {
        Timestamp::parse(KEY).expect("the key parses").epoch()
    }

    fn after_key(since: i64) -> Timestamp {
        Timestamp::from_epoch(key_epoch() + since)
    }

    /// A record `since` seconds after the key, through the typed reader the
    /// daemon uses.
    fn record(since: i64, actor: &str, action: &str, target: &str, rest: &str) -> Event {
        let ts = after_key(since);
        Event::parse_line(&format!(
            r#"{{"ts":"{ts}","actor":"{actor}","action":"{action}","target":"{target}"{rest}}}"#
        ))
        .expect("the specimen is a well-formed event")
    }

    /// A watchdog record addressed to the seat by display name, as the daemon
    /// writes one.
    fn watchdog(since: i64, action: &str, reference: Option<&str>) -> Event {
        let rest = reference.map(|value| format!(r#","ref":"{value}""#));
        record(
            since,
            WATCHDOG_ACTOR,
            action,
            AGENT,
            &rest.unwrap_or_default(),
        )
    }

    /// The same, addressed by routing key, as the legs write one.
    fn routed(since: i64, action: &str, reference: &str) -> Event {
        let rest =
            format!(r#","ref":"{reference}","target_slot":"{SLOT}","target_session":"{SESSION}""#);
        record(since, WATCHDOG_ACTOR, action, AGENT, &rest)
    }

    /// The seat's `spawn`, written by its spawner.
    fn spawned(since: i64) -> Event {
        record(since, "lead", SPAWN_ACTION, AGENT, "")
    }

    /// The `limit` that opens the episode keyed `KEY`.
    fn limit() -> Event {
        watchdog(0, LIMIT_ACTION, None)
    }

    fn entry(key: &str, value: Option<&str>, line: usize) -> SectionEntry {
        SectionEntry {
            key: key.to_owned(),
            value: value.map(ToOwned::to_owned),
            line,
        }
    }

    #[test]
    fn the_switch_is_off_on_or_all_and_anything_else_is_off_with_a_note() {
        assert_eq!(parse_switch(Ok(None)), Ok(Switch::Off));
        assert_eq!(parse_switch(Ok(Some("off".into()))), Ok(Switch::Off));
        assert_eq!(parse_switch(Ok(Some("on".into()))), Ok(Switch::On));
        assert_eq!(parse_switch(Ok(Some(" all ".into()))), Ok(Switch::All));
        for bad in ["", "yes", "ON", "1", "\u{1b}[31m"] {
            let note = parse_switch(Ok(Some(bad.into()))).expect_err(bad);
            assert!(note.starts_with("auto reseat stays off: "), "{note}");
            assert!(
                !note.contains('\u{1b}'),
                "a note echoes no control byte: {note:?}"
            );
        }
        let note = parse_switch(Err("auto_reseat is malformed".into())).expect_err("malformed");
        assert_eq!(note, "auto reseat stays off: auto_reseat is malformed");
    }

    #[test]
    fn the_knobs_and_the_map_are_read_with_every_ignored_entry_named() {
        let settled = settle(
            Switch::On,
            Ok(Some("aedev, bad name!, aedev".into())),
            Ok(Some("900".into())),
            Ok(vec![
                entry("sol6x", Some("opus55x-mic, opus55x, spark13cm"), 3),
                entry("bad key", Some("x"), 4),
                entry("fablex", None, 5),
                entry("opus55x", Some("opus55x, sol6x, sol6x, 9bad, "), 6),
                entry("astrax", Some("astrax"), 7),
                entry("sol6x", Some("spark13cm"), 8),
            ]),
        );
        assert_eq!(settled.switch, Switch::On);
        assert_eq!(settled.sessions, Some(vec!["aedev".to_owned()]));
        assert_eq!(settled.grace_secs, 900);
        // A profile keyed twice keeps its LATER row, in that row's place.
        assert_eq!(
            settled.map,
            [
                ("opus55x".to_owned(), vec!["sol6x".to_owned()]),
                ("sol6x".to_owned(), vec!["spark13cm".to_owned()]),
            ]
        );
        assert_eq!(
            settled.candidates("sol6x"),
            Some(&["spark13cm".to_owned()][..])
        );
        assert_eq!(settled.candidates("fablex"), None);
        // Two session entries, the bad key, the valueless row, two self-moves,
        // the duplicate, the bad name, the list left empty, the second key.
        assert_eq!(settled.notes.len(), 10, "{:#?}", settled.notes);
        assert!(
            settled
                .notes
                .iter()
                .all(|note| note.contains("auto_reseat"))
        );
    }

    #[test]
    fn an_unusable_knob_turns_the_whole_path_off_with_its_note() {
        let map = || Ok(vec![entry("sol6x", Some("opus55x"), 3)]);
        for (sessions, grace, map) in [
            (
                Err("auto_reseat_sessions is malformed".to_owned()),
                Ok(None),
                map(),
            ),
            (Ok(None), Ok(Some("ten".to_owned())), map()),
            (Ok(None), Ok(Some("-5".to_owned())), map()),
            (Ok(None), Ok(None), Err("config unreadable".to_owned())),
        ] {
            let settled = settle(Switch::All, sessions, grace, map);
            assert_eq!(settled.switch, Switch::Off, "{settled:?}");
            assert!(settled.map.is_empty(), "{settled:?}");
            assert_eq!(settled.notes.len(), 1, "{settled:?}");
        }
        let settled = settle(Switch::On, Ok(Some(" , ".into())), Ok(None), map());
        assert_eq!(
            settled.sessions,
            Some(Vec::new()),
            "a list naming nobody acts nowhere"
        );
        assert_eq!(settled.grace_secs, DEFAULT_GRACE_SECS);
        let settled = settle(Switch::On, Ok(None), Ok(Some("0".into())), map());
        assert_eq!(
            (settled.switch, settled.grace_secs),
            (Switch::On, 0),
            "no grace is a grace"
        );
    }

    /// A config file removed with the test that wrote it, even on a failure.
    struct Scratch(std::path::PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn the_global_file_is_read_live_and_nothing_past_an_off_switch_is_judged() {
        let file =
            Scratch(std::env::temp_dir().join(format!("ae-autoreseat-{}", std::process::id())));
        let path = file.0.as_path();
        std::fs::write(
            path,
            "[workspace]\nauto_reseat = on\nauto_reseat_grace_secs = 60\n[auto_reseat]\nsol6x = opus55x\n",
        )
        .expect("write config");
        let on = settings(Some(path));
        assert_eq!((on.switch, on.grace_secs), (Switch::On, 60));
        assert_eq!(on.candidates("sol6x"), Some(&["opus55x".to_owned()][..]));
        std::fs::write(
            path,
            "[workspace]\nauto_reseat = off\nauto_reseat_grace_secs = ten\n[auto_reseat]\nbad key = x\n",
        )
        .expect("rewrite config");
        assert_eq!(settings(Some(path)), Settings::off(Vec::new()));
        let _ = std::fs::remove_file(path);
        assert_eq!(
            settings(Some(path)),
            Settings::off(Vec::new()),
            "no file, no move"
        );
        assert!(
            std::fs::create_dir_all(path).is_ok(),
            "a directory in the file's place"
        );
        let unreadable = settings(Some(path));
        let _ = std::fs::remove_dir_all(path);
        assert_eq!(unreadable.switch, Switch::Off);
        assert!(
            matches!(&unreadable.notes[..], [note]
                if note.starts_with("auto reseat stays off: could not read global config: ")),
            "an unreadable file is named, never read as absent: {unreadable:?}"
        );
        assert_eq!(settings(None), Settings::off(Vec::new()));
    }

    fn with_map(switch: Switch) -> Settings {
        Settings {
            switch,
            sessions: None,
            grace_secs: DEFAULT_GRACE_SECS,
            map: vec![("sol6x".to_owned(), vec!["opus55x".to_owned()])],
            notes: Vec::new(),
        }
    }

    fn seat(slot: &str) -> Seat<'_> {
        Seat {
            session: SESSION,
            slot,
            agent: AGENT,
            profile: "sol6x",
            orchestrator: false,
        }
    }

    #[test]
    fn on_moves_fixed_and_spawned_seats_and_only_all_moves_main() {
        for (slot, class) in [
            ("main", SeatClass::Main),
            ("worker.0", SeatClass::Fixed),
            ("worker.10", SeatClass::Fixed),
            ("worker.01", SeatClass::Fixed),
            ("spawned.0", SeatClass::Spawned),
            ("spawned.12", SeatClass::Spawned),
        ] {
            assert_eq!(SeatClass::of(slot), Some(class), "{slot}");
        }
        for odd in ["", "worker.", "spawned.x", "worker.0x", "Main"] {
            assert_eq!(SeatClass::of(odd), None, "{odd:?}");
        }
        let on = with_map(Switch::On);
        let all = with_map(Switch::All);
        for slot in ["worker.0", "spawned.3"] {
            assert!(eligible(&on, &seat(slot)).is_ok(), "{slot}");
        }
        assert_eq!(eligible(&on, &seat("main")), Err(Ineligible::Class));
        assert_eq!(
            eligible(&all, &seat("main")),
            Ok(&["opus55x".to_owned()][..])
        );
    }

    #[test]
    fn an_orchestrator_an_unlisted_session_a_bad_slot_or_an_unmapped_profile_never_moves() {
        let mut settings = with_map(Switch::All);
        assert_eq!(
            eligible(&with_map(Switch::Off), &seat("worker.0")),
            Err(Ineligible::Off)
        );
        let orchestrator = Seat {
            orchestrator: true,
            ..seat("worker.0")
        };
        assert_eq!(
            eligible(&settings, &orchestrator),
            Err(Ineligible::Orchestrator)
        );
        assert_eq!(
            eligible(&settings, &seat("worker.x")),
            Err(Ineligible::Slot)
        );
        let unmapped = Seat {
            profile: "fablex",
            ..seat("worker.0")
        };
        assert_eq!(eligible(&settings, &unmapped), Err(Ineligible::Unmapped));
        settings.sessions = Some(vec!["other".to_owned()]);
        assert_eq!(
            eligible(&settings, &seat("worker.0")),
            Err(Ineligible::Session)
        );
        settings.sessions = Some(vec![SESSION.to_owned()]);
        assert!(eligible(&settings, &seat("worker.0")).is_ok());
    }

    fn fold(events: &[Event]) -> Option<Episode> {
        episode(events, SESSION, SLOT, AGENT)
    }

    #[test]
    fn the_first_limit_after_the_boundary_keys_the_episode_and_a_restart_keeps_it() {
        assert_eq!(fold(&[]), None);
        let events = [
            watchdog(-3600, LIMIT_ACTION, None),
            watchdog(-1800, CLEARED_ACTION, None),
            limit(),
            // A restarted daemon books the limit again: same episode.
            watchdog(300, LIMIT_ACTION, None),
        ];
        let found = fold(&events).expect("an episode");
        assert_eq!(found.key.to_string(), KEY);
        assert_eq!(found.reference(), KEY);
        assert_eq!(
            (found.attempts, found.open, found.terminal),
            (0, None, None)
        );
    }

    #[test]
    fn alert_cleared_and_spawn_end_the_episode_and_reseat_does_not() {
        assert_eq!(fold(&[limit(), watchdog(60, CLEARED_ACTION, None)]), None);
        assert_eq!(fold(&[limit(), spawned(60)]), None);
        assert!(fold(&[limit(), routed(660, "reseat", "anything")]).is_some());
    }

    #[test]
    fn attempts_and_outcomes_count_by_equal_ref_and_the_first_terminal_outcome_wins() {
        let mut events = vec![limit(), routed(600, ATTEMPT_ACTION, KEY)];
        let open = fold(&events).expect("an episode");
        assert_eq!((open.attempts, open.open), (1, Some(after_key(600))));
        // The success path: the move's own outcome closes the attempt.
        let mut moved = events.clone();
        moved.push(routed(700, DONE_ACTION, "sol6x"));
        let done = fold(&moved).expect("an episode");
        assert_eq!((done.open, done.terminal), (None, Some(Outcome::Done)));
        events.push(routed(780, FAILED_ACTION, KEY));
        events.push(routed(785, DONE_ACTION, "sol6x"));
        events.push(routed(840, REFUSED_ACTION, KEY));
        let closed = fold(&events).expect("an episode");
        assert_eq!(
            (closed.open, closed.terminal),
            (None, Some(Outcome::Failed))
        );
        // A done with no attempt open still ends the path: the seat moved.
        let stray = fold(&[limit(), routed(700, DONE_ACTION, "sol6x")]).expect("an episode");
        assert_eq!((stray.attempts, stray.terminal), (0, Some(Outcome::Done)));
        // Another episode's ref counts for nothing here.
        let stale = [
            limit(),
            routed(600, ATTEMPT_ACTION, OTHER_KEY),
            routed(601, REFUSED_ACTION, OTHER_KEY),
            routed(602, ATTEMPT_ACTION, KEY),
            routed(603, HELD_ACTION, OTHER_KEY),
            routed(604, FAILED_ACTION, OTHER_KEY),
        ];
        let found = fold(&stale).expect("an episode");
        assert_eq!(
            (found.attempts, found.open, found.terminal, found.held),
            (1, Some(after_key(602)), None, false)
        );
    }

    #[test]
    fn a_hold_is_not_terminal_and_a_later_attempt_clears_it() {
        let held = fold(&[limit(), watchdog(600, HELD_ACTION, Some(KEY))]).expect("an episode");
        assert!(held.held && held.terminal.is_none() && held.attempts == 0);
        let events = [
            limit(),
            routed(660, ATTEMPT_ACTION, KEY),
            routed(690, HELD_ACTION, KEY),
        ];
        let transient = fold(&events).expect("an episode");
        assert_eq!(
            (transient.attempts, transient.open, transient.terminal),
            (1, None, None)
        );
        assert!(transient.held);
        let mut again = events.to_vec();
        again.push(routed(720, ATTEMPT_ACTION, KEY));
        let second = fold(&again).expect("an episode");
        assert_eq!((second.attempts, second.held), (2, false));
    }

    #[test]
    fn only_the_watchdogs_records_for_this_seat_are_the_episode() {
        let reference = format!(r#","ref":"{KEY}""#);
        let forged = record(600, AGENT, REFUSED_ACTION, AGENT, &reference);
        let other = record(600, WATCHDOG_ACTOR, REFUSED_ACTION, "other", &reference);
        let found = fold(&[limit(), forged, other]).expect("an episode");
        assert_eq!(found.terminal, None);
        // A rename leaves the display-keyed limit under the old name: no
        // episode, so nothing fires.
        assert_eq!(episode(&[limit()], SESSION, SLOT, "renamed"), None);
    }

    #[test]
    fn left_profiles_names_each_profile_left_since_the_seats_spawn_newest_move_each() {
        let forged = record(-900, AGENT, DONE_ACTION, AGENT, r#","ref":"astrax""#);
        let events = [
            routed(-10_800, DONE_ACTION, "fablex"),
            spawned(-7200),
            routed(-3600, DONE_ACTION, "sol6x"),
            routed(-1800, DONE_ACTION, "opus55x"),
            // Only the watchdog's own done names a profile left.
            forged,
            routed(-600, ATTEMPT_ACTION, KEY),
            routed(0, DONE_ACTION, "sol6x"),
        ];
        assert_eq!(
            left_profiles(&events, SESSION, SLOT, AGENT),
            [
                ("opus55x".to_owned(), after_key(-1800)),
                ("sol6x".to_owned(), after_key(0)),
            ]
        );
    }

    const CLEAR: Pane = Pane {
        frame: Frame::Clear,
        human_prompt: false,
        client_input: None,
    };

    /// A pane whose client input, when there is some, came `since` seconds
    /// after the key.
    fn pane(frame: Frame, human_prompt: bool, input_since: Option<i64>) -> Pane {
        Pane {
            frame,
            human_prompt,
            client_input: input_since.map(|since| key_epoch() + since),
        }
    }

    fn at(events: &[Event], pane: &Pane, since_key: i64) -> Decision {
        decide(fold(events).as_ref(), 600, pane, key_epoch() + since_key)
    }

    #[test]
    fn under_the_grace_it_waits_then_attempts() {
        let limit = [limit()];
        let wait = Decision::Wait {
            due_at: key_epoch() + 600,
        };
        assert_eq!(at(&limit, &CLEAR, 599), wait);
        // Under the grace nothing is judged, not even a busy frame.
        assert_eq!(at(&limit, &pane(Frame::Busy, false, None), 599), wait);
        assert_eq!(at(&limit, &CLEAR, 600), Decision::Attempt);
        assert_eq!(decide(None, 600, &CLEAR, key_epoch() + 900), Decision::Rest);
    }

    #[test]
    fn a_due_seat_holds_on_an_unread_busy_drafted_prompted_or_touched_pane() {
        let limit = [limit()];
        for (pane, reason) in [
            (pane(Frame::Unread, false, None), HoldReason::Unread),
            (pane(Frame::Busy, false, None), HoldReason::Busy),
            (pane(Frame::Draft, false, None), HoldReason::Draft),
            (pane(Frame::Clear, true, None), HoldReason::HumanPrompt),
            (
                pane(Frame::Clear, false, Some(100)),
                HoldReason::ClientInput,
            ),
            // The frame outranks the prompt, and the prompt the input.
            (pane(Frame::Busy, true, Some(100)), HoldReason::Busy),
            (pane(Frame::Clear, true, Some(100)), HoldReason::HumanPrompt),
            (pane(Frame::Draft, true, None), HoldReason::Draft),
            (pane(Frame::Unread, true, None), HoldReason::Unread),
        ] {
            assert_eq!(at(&limit, &pane, 650), Decision::Hold(reason), "{pane:?}");
        }
        // Each reason reads as its own hold in the record that names it.
        let summaries = [
            HoldReason::Unread,
            HoldReason::Busy,
            HoldReason::Draft,
            HoldReason::HumanPrompt,
            HoldReason::ClientInput,
        ]
        .map(HoldReason::summary);
        for (index, summary) in summaries.iter().enumerate() {
            assert!(summary.len() > "held: ".len(), "{summary:?}");
            assert!(summary.starts_with("held: "), "{summary:?}");
            assert!(!summaries[..index].contains(summary), "{summary:?}");
        }
        // Only input strictly after the key is a human reacting to this limit,
        // and it holds until the grace has passed since that input.
        for (input, now) in [(-5, 650), (0, 650), (100, 700)] {
            let touched = pane(Frame::Clear, false, Some(input));
            assert_eq!(
                at(&limit, &touched, now),
                Decision::Attempt,
                "{input} {now}"
            );
        }
    }

    #[test]
    fn an_open_attempt_is_in_flight_until_its_bound_then_overdue() {
        let busy = pane(Frame::Busy, false, None);
        let first = [limit(), routed(600, ATTEMPT_ACTION, KEY)];
        assert_eq!(at(&first, &busy, 600 + 179), Decision::InFlight);
        assert_eq!(at(&first, &busy, 600 + IN_FLIGHT_SECS), Decision::Overdue);
        // The second attempt keeps the same bound: its spent count never
        // silences an attempt that is still open.
        let second = [
            limit(),
            routed(600, ATTEMPT_ACTION, KEY),
            routed(630, HELD_ACTION, KEY),
            routed(700, ATTEMPT_ACTION, KEY),
        ];
        assert_eq!(at(&second, &CLEAR, 700 + 179), Decision::InFlight);
        assert_eq!(at(&second, &CLEAR, 700 + IN_FLIGHT_SECS), Decision::Overdue);
    }

    #[test]
    fn a_terminal_outcome_or_spent_attempts_rest_and_a_transient_hold_earns_one_more() {
        let mut events = vec![
            limit(),
            routed(600, ATTEMPT_ACTION, KEY),
            routed(630, HELD_ACTION, KEY),
        ];
        assert_eq!(at(&events, &CLEAR, 700), Decision::Attempt);
        events.push(routed(720, ATTEMPT_ACTION, KEY));
        events.push(routed(750, HELD_ACTION, KEY));
        assert_eq!(at(&events, &CLEAR, 900), Decision::Rest);
        let refused = [limit(), routed(600, REFUSED_ACTION, KEY)];
        assert_eq!(at(&refused, &CLEAR, 900), Decision::Rest);
        // The latch never broke, so a limit booked after the move is the same
        // episode: the move ended its auto path, and nothing chains.
        let moved = [
            limit(),
            routed(600, ATTEMPT_ACTION, KEY),
            routed(700, DONE_ACTION, "sol6x"),
            watchdog(900, LIMIT_ACTION, None),
        ];
        assert_eq!(at(&moved, &CLEAR, 2_000), Decision::Rest);
    }

    #[test]
    fn hostile_numbers_saturate_instead_of_wrapping_or_panicking() {
        let mut events = vec![limit()];
        events.extend((1..=300).map(|since| routed(since, ATTEMPT_ACTION, KEY)));
        let crowded = fold(&events).expect("an episode");
        assert_eq!(crowded.attempts, u8::MAX);
        assert_eq!(
            decide(Some(&crowded), 600, &CLEAR, i64::MAX),
            Decision::Overdue
        );
        assert_eq!(
            decide(Some(&crowded), 600, &CLEAR, i64::MIN),
            Decision::InFlight
        );
        let quiet = fold(&[limit()]).expect("an episode");
        assert_eq!(
            decide(Some(&quiet), u64::MAX, &CLEAR, key_epoch()),
            Decision::Wait { due_at: i64::MAX }
        );
        let touched = pane(Frame::Clear, false, Some(1));
        assert_eq!(
            decide(Some(&quiet), u64::MAX, &touched, i64::MAX),
            Decision::Attempt
        );
    }

    fn window(judged: f64, critical: bool, observed_at: i64, status: Status) -> Window {
        Window {
            judged,
            critical,
            observed_at,
            resets_at: None,
            status,
        }
    }

    fn candidate(profile: &str, windows: Vec<Window>) -> Candidate {
        Candidate {
            profile: profile.to_owned(),
            configured: true,
            peer_latched: false,
            windows,
            left_at: None,
        }
    }

    const NOW: i64 = 10_000;

    fn pick(candidates: &[Candidate]) -> Option<(String, Tier)> {
        choose(candidates, NOW).pick
    }

    #[test]
    fn exhausted_latched_and_unconfigured_candidates_are_passed_over_by_name() {
        let choice = choose(
            &[
                Candidate {
                    configured: false,
                    ..candidate("ghost", Vec::new())
                },
                Candidate {
                    peer_latched: true,
                    ..candidate("shared", Vec::new())
                },
                candidate("full", vec![window(100.0, true, NOW - 60, Status::Fresh)]),
                candidate(
                    "stale-full",
                    vec![window(100.0, true, NOW - 1800, Status::Stale)],
                ),
                candidate(
                    "expired",
                    vec![window(100.0, true, NOW - 90_000, Status::Unknown)],
                ),
            ],
            NOW,
        );
        assert_eq!(choice.pick, Some(("expired".to_owned(), Tier::Unknown)));
        assert_eq!(
            choice.skipped,
            [
                ("ghost".to_owned(), Skip::Unconfigured),
                ("shared".to_owned(), Skip::PeerLatched),
                ("full".to_owned(), Skip::Exhausted),
                ("stale-full".to_owned(), Skip::Exhausted),
            ]
        );
        assert_eq!(
            pick(&[candidate(
                "full",
                vec![window(100.0, true, NOW, Status::Fresh)]
            )]),
            None
        );
        assert_eq!(pick(&[]), None);
    }

    #[test]
    fn the_best_tier_wins_and_declared_order_decides_inside_it() {
        let hot = || candidate("hot", vec![window(96.0, true, NOW, Status::Fresh)]);
        let dark = || candidate("dark", Vec::new());
        let choice = choose(
            &[
                hot(),
                dark(),
                candidate(
                    "stale-cool",
                    vec![window(10.0, false, NOW - 1800, Status::Stale)],
                ),
                candidate("cool", vec![window(40.0, false, NOW, Status::Fresh)]),
                candidate("cooler", vec![window(5.0, false, NOW, Status::Fresh)]),
            ],
            NOW,
        );
        assert_eq!(choice.pick, Some(("cool".to_owned(), Tier::BelowCritical)));
        assert!(choice.skipped.is_empty());
        assert_eq!(
            pick(&[hot(), dark()]),
            Some(("dark".to_owned(), Tier::Unknown))
        );
        assert_eq!(pick(&[hot()]), Some(("hot".to_owned(), Tier::Critical)));
        // The classifier's flag decides the tier, not the number beside it.
        let edge = candidate("edge", vec![window(99.0, false, NOW, Status::Fresh)]);
        assert_eq!(
            pick(&[edge]),
            Some(("edge".to_owned(), Tier::BelowCritical))
        );
    }

    #[test]
    fn a_profile_left_on_its_limit_waits_for_a_later_reading_below_critical_or_a_reset() {
        let left = |windows| Candidate {
            left_at: Some(5_000),
            ..candidate("sol6x", windows)
        };
        for windows in [
            // Nothing read since the move, or only before it or in its second.
            Vec::new(),
            vec![window(20.0, false, 4_000, Status::Fresh)],
            vec![window(20.0, false, 5_000, Status::Fresh)],
            // Read since, but still critical or still at the limit.
            vec![window(99.0, true, 6_000, Status::Fresh)],
            // A later number ae cannot use is no reading at all.
            vec![window(20.0, false, 6_000, Status::Unknown)],
            vec![
                window(20.0, false, 6_000, Status::Fresh),
                window(100.0, true, 6_000, Status::Stale),
            ],
        ] {
            let choice = choose(&[left(windows)], NOW);
            assert_eq!(choice.skipped, [("sol6x".to_owned(), Skip::LeftOnLimit)]);
        }
        let stale = left(vec![window(20.0, false, 6_000, Status::Stale)]);
        assert_eq!(pick(&[stale]), Some(("sol6x".to_owned(), Tier::Unknown)));
        let headroom = left(vec![window(20.0, false, 6_000, Status::Fresh)]);
        assert_eq!(
            pick(&[headroom]),
            Some(("sol6x".to_owned(), Tier::BelowCritical))
        );
        let reset = Window {
            resets_at: Some(NOW - 1),
            ..window(100.0, true, 6_000, Status::Unknown)
        };
        assert_eq!(
            pick(&[left(vec![reset])]),
            Some(("sol6x".to_owned(), Tier::Unknown))
        );
    }

    /// A frame is judged in the order a hold is: a read that failed first, then
    /// a running turn, then a human's draft. A seat on its limit draws the
    /// vendor's row where a finished turn would sit, so an unrecognised frame
    /// with an empty box is as clear as an idle one; the move proves its own stop.
    #[test]
    fn a_capture_proves_a_clear_frame_only_when_read_and_free_of_turns_and_drafts() {
        use HarnessState::{Busy, Idle, Unknown};
        for (read, state, draft, frame) in [
            (false, Idle, false, Frame::Unread),
            (false, Busy, true, Frame::Unread),
            (true, Busy, true, Frame::Busy),
            (true, Busy, false, Frame::Busy),
            (true, Idle, true, Frame::Draft),
            (true, Unknown, true, Frame::Draft),
            (true, Idle, false, Frame::Clear),
            (true, Unknown, false, Frame::Clear),
        ] {
            assert_eq!(
                frame_of(read, state, draft),
                frame,
                "{read} {state:?} {draft}"
            );
        }
    }

    /// An attempt is in flight from its record until its bound, and only while
    /// the switch is on: off, the watchdog reads the pane as it always has.
    #[test]
    fn an_attempt_is_in_flight_only_inside_its_bound_and_only_while_the_switch_is_on() {
        let on = with_map(Switch::On);
        let open = [limit(), watchdog(600, ATTEMPT_ACTION, Some(KEY))];
        let done = [
            limit(),
            watchdog(600, ATTEMPT_ACTION, Some(KEY)),
            routed(620, DONE_ACTION, "sol6x"),
        ];
        let started = key_epoch() + 600;
        for (settings, events, now, flying) in [
            (&on, &open[..], started, true),
            (&on, &open[..], started + IN_FLIGHT_SECS - 1, true),
            (&on, &open[..], started + IN_FLIGHT_SECS, false),
            (&with_map(Switch::Off), &open[..], started + 1, false),
            (&on, &done[..], started + 1, false),
            (&on, &[limit()][..], started + 1, false),
            (&on, &[][..], started + 1, false),
        ] {
            assert_eq!(
                in_flight(settings, events, (SESSION, SLOT, AGENT), now),
                flying,
                "{events:?} at {now}"
            );
        }
    }

    /// The last check before a hold or an overdue failure is written, under the
    /// seat's lock: the journal as it reads NOW must still owe it. An attempt
    /// opened meanwhile owes no hold, one inside its bound owes no failure, and
    /// an ended episode, or another one, owes nothing.
    #[test]
    fn a_hold_or_an_overdue_failure_is_written_only_while_the_journal_still_owes_it() {
        let key = Timestamp::parse(KEY).expect("the key parses");
        let other = Timestamp::parse(OTHER_KEY).expect("the key parses");
        let fold = |events: &[Event]| episode(events, SESSION, SLOT, AGENT);
        let bare = fold(&[limit()]);
        let open = fold(&[limit(), watchdog(600, ATTEMPT_ACTION, Some(KEY))]);
        let ended = fold(&[
            limit(),
            watchdog(600, ATTEMPT_ACTION, Some(KEY)),
            watchdog(610, REFUSED_ACTION, Some(KEY)),
        ]);
        assert_eq!(
            lock_path(Path::new("/s/aedev"), SLOT),
            Path::new("/s/aedev/auto-reseat.spawned.3.lock")
        );
        let inside = key_epoch() + 600 + IN_FLIGHT_SECS - 1;
        let past = inside + 1;
        for (found, at, overdue, now, still) in [
            (&bare, key, false, past, true),
            (&bare, key, true, past, false),
            (&open, key, false, inside, false),
            (&open, key, true, inside, false),
            (&open, key, true, past, true),
            (&ended, key, false, past, false),
            (&ended, key, true, past, false),
            (&bare, other, false, past, false),
            (&None, key, false, past, false),
        ] {
            assert_eq!(
                owed(found.as_ref(), at, overdue, now),
                still,
                "{found:?} {at} overdue={overdue} at {now}"
            );
        }
    }

    fn roster_seat(slot: &str, name: &str) -> crate::meta::RosterEntry {
        crate::meta::RosterEntry {
            slot: slot.to_owned(),
            name: name.to_owned(),
            profile: None,
            client: crate::meta::RecordedClient::Missing,
            harness_session: None,
            config_home: crate::meta::RecordedConfigHome::Missing,
            config_home_base: crate::meta::RecordedConfigHomeBase::Missing,
            binary: None,
            work_dir: crate::meta::RecordedWorkDir::Missing,
        }
    }

    /// A codex seat whose recorded account is `home`: an identity the quota
    /// join can prove.
    fn codex_seat(slot: &str, name: &str, home: &Path) -> crate::meta::RosterEntry {
        crate::meta::RosterEntry {
            harness_session: Some("018f1f70-7b2c-7000-8000-000000000001".to_owned()),
            config_home: crate::meta::RecordedConfigHome::Path(home.to_path_buf()),
            binary: Some("codex".to_owned()),
            ..roster_seat(slot, name)
        }
    }

    #[test]
    fn a_seat_still_on_its_limit_since_its_last_clear_is_latched_and_the_moving_seat_is_not() {
        let root = Scratch(std::env::temp_dir().join(format!("ae-latched-{}", std::process::id())));
        let home = |name: &str| {
            let path = root.0.join(name);
            std::fs::create_dir_all(path.join("sessions")).expect("an account home");
            path
        };
        let roster = [
            codex_seat("main", "lead", &home("a")),
            codex_seat("spawned.1", "peer", &home("b")),
            codex_seat("spawned.2", "stuck", &home("c")),
            codex_seat("spawned.4", "cleared", &home("d")),
            codex_seat(SLOT, AGENT, &home("a")),
        ];
        let on_limit = |name: &str| record(0, WATCHDOG_ACTOR, LIMIT_ACTION, name, "");
        let events = [
            on_limit("peer"),
            on_limit("stuck"),
            record(
                60,
                WATCHDOG_ACTOR,
                REFUSED_ACTION,
                "stuck",
                &format!(r#","ref":"{KEY}""#),
            ),
            on_limit("cleared"),
            record(60, WATCHDOG_ACTOR, CLEARED_ACTION, "cleared", ""),
            on_limit(AGENT),
        ];
        let identity =
            |at: usize| crate::quota::recorded_identity(&roster[at]).expect("a recorded identity");
        assert_eq!(
            latched_identities(&roster, &events, SESSION, SLOT),
            [identity(1), identity(2)],
            "a refused path leaves its seat on its limit; a clear ends it; the moving seat is not counted"
        );
        assert!(latched_identities(&roster, &[], SESSION, SLOT).is_empty());
    }

    #[test]
    fn the_spawner_is_the_actor_of_the_newest_spawn_addressed_to_the_seat() {
        let events = [
            record(10, "lead", SPAWN_ACTION, AGENT, ""),
            record(20, "colead", SPAWN_ACTION, AGENT, ""),
            record(30, "other", SPAWN_ACTION, "someone-else", ""),
        ];
        assert_eq!(spawner(&events, SESSION, SLOT, AGENT), Some("colead"));
        assert_eq!(spawner(&events[2..], SESSION, SLOT, AGENT), None);
    }

    #[test]
    fn the_lead_pair_without_the_moved_seat_and_its_spawner_once_are_told() {
        let roster = [
            roster_seat("main", "lead"),
            roster_seat("worker.0", "colead"),
            roster_seat("spawned.1", "scout"),
            roster_seat(SLOT, AGENT),
        ];
        let told = |pair: bool, moved: &str, by: Option<&str>| recipients(&roster, pair, moved, by);
        assert_eq!(told(false, SLOT, None), ["lead"], "solo: the main seat");
        assert_eq!(told(true, SLOT, None), ["lead", "colead"], "lead-pair");
        assert_eq!(told(true, SLOT, Some("scout")), ["lead", "colead", "scout"]);
        assert_eq!(
            told(true, SLOT, Some("lead")),
            ["lead", "colead"],
            "told once"
        );
        assert_eq!(
            told(true, SLOT, Some(AGENT)),
            ["lead", "colead"],
            "not itself"
        );
        assert_eq!(
            told(true, SLOT, Some("ghost")),
            ["lead", "colead"],
            "not on the roster"
        );
        assert_eq!(
            told(true, "main", None),
            ["colead"],
            "all moved main in a lead pair"
        );
        assert!(
            told(false, "main", None).is_empty(),
            "a solo main that moved"
        );
    }

    #[test]
    fn each_ending_is_told_in_one_bounded_line_and_only_a_move_warns_the_pair() {
        let moved = Move {
            from: "sol6x",
            to: "opus55x",
            tool: ("codex", "claude"),
            model: ("gpt-6-sol", "opus"),
            carried: true,
            critical: false,
            dirty: false,
        };
        assert_eq!(
            notice(SESSION, AGENT, &Ending::Moved(moved)),
            "auto reseat: builder moved sol6x -> opus55x (tool codex -> claude, model gpt-6-sol -> \
             opus), carried; work tree clean. If builder is half of a review pair, re-check its gate \
             provider. Next: nothing, it continues its conversation."
        );
        let seeded = Move {
            carried: false,
            critical: true,
            dirty: true,
            ..moved
        };
        assert_eq!(
            notice(SESSION, AGENT, &Ending::Moved(seeded)),
            "auto reseat: builder moved sol6x -> opus55x (tool codex -> claude, model gpt-6-sol -> \
             opus), seeded, target already critical; work tree dirty. If builder is half of a review \
             pair, re-check its gate provider. Next: check it picked up its seat pack."
        );
        for (ending, want) in [
            (
                Ending::Held("held: auto reseat is off"),
                "auto reseat: builder not moved yet — held: auto reseat is off. Next: ae tries once \
                 more if it stays eligible.",
            ),
            (
                Ending::Refused("refused: no usable candidate: opus55x (exhausted)"),
                "auto reseat: builder not moved — refused: no usable candidate: opus55x (exhausted). \
                 Next: move it by hand (ae reseat aedev builder --using <profile>) or wait for the \
                 reset.",
            ),
            (
                Ending::Failed("Error: the pane never came back"),
                "auto reseat: builder move failed — Error: the pane never came back. Next: relaunch \
                 builder if its pane sits at a shell, else reseat it by hand.",
            ),
        ] {
            let said = notice(SESSION, AGENT, &ending);
            assert_eq!(said, want);
            assert!(!said.contains("review pair"), "{said}");
        }
        let hostile = format!("Error: {}\u{1b}[2J\nsecond line", "x".repeat(NOTICE_CHARS));
        let said = notice(SESSION, AGENT, &Ending::Failed(&hostile));
        assert!(
            said.chars().count() <= NOTICE_CHARS,
            "{}",
            said.chars().count()
        );
        assert!(!said.chars().any(char::is_control), "{said:?}");
    }

    /// The text entry the config fuzz target drives IS the parser the path read
    /// uses: over every shape the knobs take, the two agree.
    #[test]
    fn the_text_entry_judges_exactly_what_the_path_read_judges() {
        let file = Scratch(
            std::env::temp_dir().join(format!("ae-autoreseat-text-{}", std::process::id())),
        );
        let path = file.0.as_path();
        for text in [
            "",
            "[workspace]\nauto_reseat = on\n",
            "[workspace]\nauto_reseat = off\n[auto_reseat]\nsol6x = opus55x\n",
            "[workspace]\nauto_reseat = maybe\n",
            "[workspace]\nauto_reseat = all\nauto_reseat_sessions = aedev, bad name, aedev\n\
             auto_reseat_grace_secs = 0\n[auto_reseat]\nsol6x = opus55x, sol6x, opus55x\n\
             bad key = x\nfablex =\n",
            "[workspace]\nauto_reseat = on\nauto_reseat_grace_secs = soon\n",
            "[workspace]\nauto_reseat = on\n[auto_reseat]\nsol6x = opus55x\nsol6x = astrax\n",
            "[auto_reseat]\nsol6x = opus55x\n[workspace]\nauto_reseat = on\n",
            "[workspace]\nauto_reseat on\n",
        ] {
            std::fs::write(path, text).expect("write config");
            assert_eq!(settings_in(path, text), settings(Some(path)), "{text:?}");
        }
    }
}
