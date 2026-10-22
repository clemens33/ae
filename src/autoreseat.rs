//! Auto reseat: move a seat PROVEN stuck on its vendor usage limit — or, before
//! that, one whose own account crossed the headroom threshold — in place, to
//! the first usable profile the human declared for it, else one of its own
//! family, and say so.
//!
//! This file holds the DECISIONS and reads nothing but its arguments and the
//! global config: which limit episode a seat is in, whether that episode is due
//! for a move, and which listed candidate to take. The watchdog asks them to
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
//! A limit move that is REFUSED or FAILED closes a ROUND, not the episode: after
//! [`RETRY_COOLDOWN_SECS`] from that closing the seat, still latched, is due
//! again under the same key, and the next attempt (or closing) opens a new round
//! with its own [`MAX_ATTEMPTS`]. The [`MAX_ROUNDS`]th closing ends the path for
//! good; a done always does. All of it is read from the journal and the clock.
//!
//! A HEADROOM episode is the same path with another opener: it starts at the
//! first `auto-reseat-headroom` the watchdog booked for the seat after its
//! newest `spawn`, re-arm or move, and its KEY is that record's timestamp. It
//! ends at the re-arm record, which the watchdog books once the seat's own
//! account reads more than [`REARM_GAP`] below the threshold, or at a move. A
//! limit outranks it: a seat on its limit is the limit path's alone.
//!
//! The folds compare action words, the seat's identity, whole `ref` values and
//! whole hold summaries by equality. They never split or parse either.

use std::path::Path;

use crate::config::SectionEntry;
use crate::events::Event;
use crate::harness_state::HarnessState;
use crate::quota::Status;
use crate::time::Timestamp;
use crate::tool::ToolKind;
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
/// A notice sent to a recipient about an outcome; target-less, a config note.
pub const NOTICE_ACTION: &str = "auto-reseat-notice";
/// A headroom episode opens; its `ts` is the key and its summary the trigger.
pub const HEADROOM_ACTION: &str = "auto-reseat-headroom";
/// A headroom episode ends and the seat is armed again. `ref` = the key.
pub const REARMED_ACTION: &str = "auto-reseat-headroom-cleared";

/// Why an attempt ended short, as the short word every outcome record leads its
/// summary with: a clip of the message behind it (a path, a profile name) can
/// then never take the cause.
///
/// The class is decided where the refusal is, never read back out of a printed
/// message: `reseat::Ended` carries it from the site that refused, and the
/// leg and the daemon name it where they decline before asking for a move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CauseClass {
    /// The conversation's transcript is not in the account it moves from.
    CarrySourceMissing,
    /// The account it moves to holds something ae will not overwrite or archive.
    CarryTargetDiffers,
    /// A carry node ae could not read or classify, or one that changed under it.
    CarryUnreadable,
    /// The seat is occupied: a running turn, a draft, a prompt or human input.
    Busy,
    /// ae cannot prove the seat's frame idle.
    UnknownFrame,
    /// Another writer holds the lifecycle lock or the pane's send-lock.
    Lock,
    /// No declared or derived candidate, or none that is usable.
    NoCandidate,
    /// The seat may not be moved by auto reseat at all.
    Ineligible,
    /// Every other refusal, every failure, and the give-up after the last round.
    Other,
}

impl CauseClass {
    /// The word a summary leads with.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::CarrySourceMissing => "carry-source-missing",
            Self::CarryTargetDiffers => "carry-target-differs",
            Self::CarryUnreadable => "carry-unreadable",
            Self::Busy => "busy",
            Self::UnknownFrame => "unknown-frame",
            Self::Lock => "lock",
            Self::NoCandidate => "no-candidate",
            Self::Ineligible => "ineligible",
            Self::Other => "other",
        }
    }

    /// Whether the cause may pass on a retry: the record is a hold, not a refusal.
    #[must_use]
    pub const fn transient(self) -> bool {
        matches!(self, Self::Busy | Self::Lock)
    }

    /// `summary` led by the class: `<class>: <summary>`.
    #[must_use]
    pub fn lead(self, summary: &str) -> String {
        format!("{}: {summary}", self.word())
    }

    /// `summary` without the class [`Self::lead`] put in front of it, for a
    /// reader that shows the class elsewhere.
    #[must_use]
    pub fn details(self, summary: &str) -> &str {
        summary.strip_prefix(&self.lead("")).unwrap_or(summary)
    }
}

/// The threshold an absent or unusable `auto_reseat_at` means.
pub const DEFAULT_HEADROOM_AT: u8 = 95;
/// How far below the threshold the seat's own account must read, strictly,
/// before a crossing can open another episode: jitter around the line cannot.
pub const REARM_GAP: f64 = 5.0;
/// The lowest threshold `auto_reseat_at` accepts.
const HEADROOM_FLOOR: u8 = 50;
const HEADROOM_KEY: &str = "auto_reseat_at";

/// Attempts one episode may make: one, and one more after a transient hold.
pub const MAX_ATTEMPTS: u8 = 2;
/// How long a limit episode whose move was refused or failed waits, counted
/// from that closing, before a seat still on the same limit is tried again.
pub const RETRY_COOLDOWN_SECS: i64 = 1800;
/// Refused or failed moves (closings) one limit episode may take in all; the
/// last ends its auto path for good. Each round keeps its [`MAX_ATTEMPTS`], so
/// one key makes at most `MAX_ROUNDS * MAX_ATTEMPTS` attempts.
pub const MAX_ROUNDS: u8 = 3;
/// The hold the daemon journals, once, when the last round was refused or
/// failed: no further attempt, and the seat is told.
pub const GAVE_UP_SUMMARY: &str = "other: held: gave up after repeated refused or failed moves";
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
    /// `auto_reseat_at`: the judged percentage of the seat's own account that
    /// moves it before its limit; `None` is off.
    pub headroom_at: Option<u8>,
    /// The note an unusable `auto_reseat_at` left, also in `notes`: the one
    /// note the watchdog journals.
    pub headroom_note: Option<String>,
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
            headroom_at: None,
            headroom_note: None,
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
        Ok(switch) => {
            let mut settled = settle(
                switch,
                read("auto_reseat_sessions"),
                read("auto_reseat_grace_secs"),
                crate::config::section_entries(file, text, "auto_reseat"),
            );
            if settled.switch != Switch::Off {
                let times = crate::config::section_entries(file, text, "workspace")
                    .map(|rows| rows.iter().filter(|row| row.key == HEADROOM_KEY).count());
                let (at, note) = parse_headroom(read(HEADROOM_KEY), times);
                settled.headroom_at = at;
                settled.notes.extend(note.clone());
                settled.headroom_note = note;
            }
            settled
        }
    }
}

/// `auto_reseat_at` as written, or the default with the one note that says why.
/// A key written twice is no value at all. A value is echoed escaped.
fn parse_headroom(
    raw: Result<Option<String>, String>,
    times: Result<usize, String>,
) -> (Option<u8>, Option<String>) {
    let fallback = |why: String| {
        (
            Some(DEFAULT_HEADROOM_AT),
            Some(format!("{why}; {DEFAULT_HEADROOM_AT} used")),
        )
    };
    match (raw, times) {
        (Err(why), _) | (_, Err(why)) => fallback(why),
        (Ok(_), Ok(times)) if times > 1 => fallback(format!("{HEADROOM_KEY} is set {times} times")),
        (Ok(None), _) => (Some(DEFAULT_HEADROOM_AT), None),
        (Ok(Some(raw)), _) => match raw.trim() {
            "off" => (None, None),
            value => match value.parse::<u8>() {
                Ok(at) if (HEADROOM_FLOOR..=100).contains(&at) && !value.starts_with('+') => {
                    (Some(at), None)
                }
                _ => fallback(format!(
                    "{HEADROOM_KEY} = {value:?} is not off or a whole percent from \
                     {HEADROOM_FLOOR} to 100"
                )),
            },
        },
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
        headroom_at: None,
        headroom_note: None,
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

impl Ineligible {
    /// The class its hold is journaled under: nothing to move to is
    /// `no-candidate`, anything else the operator ruled is `ineligible`.
    #[must_use]
    pub const fn class(self) -> CauseClass {
        match self {
            Self::Unmapped => CauseClass::NoCandidate,
            Self::Off | Self::Orchestrator | Self::Session | Self::Slot | Self::Class => {
                CauseClass::Ineligible
            }
        }
    }
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

/// How a derived candidate is related to the seat it would move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kin {
    /// The seat's own setup on another, proven account.
    Twin,
    /// The seat's own tool and flags, pinned to another model.
    Sibling,
}

/// How a candidate came to be on a seat's list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Provenance {
    /// The `[auto_reseat]` row names it.
    #[default]
    Declared,
    /// Derived from the seat's own profile.
    Derived(Kin),
}

impl Provenance {
    /// What a record adds after the candidate's name: nothing for a declared
    /// one, so every record a declared row writes reads as it always did.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Declared => "",
            Self::Derived(Kin::Twin) => ", derived twin",
            Self::Derived(Kin::Sibling) => ", derived sibling",
        }
    }
}

/// Where a profile's tool keeps the account it runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Store {
    /// The canonical config home.
    Proven(std::path::PathBuf),
    /// The tool has one, but ae cannot name it: proof of nothing.
    Unknown,
    /// The tool has no account variable: every profile of it shares one.
    Shared,
}

/// One profile as family derivation compares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shape {
    /// The command without its account variable and its model flag's value:
    /// [`crate::launch_cmd::family_skeleton`].
    pub words: Vec<String>,
    pub pin: Option<String>,
    pub store: Store,
    /// The resolved command, byte for byte.
    pub raw: String,
    /// The tool the command launches. [`listed`] never reads it; the cover
    /// verdict does, because tools without an account variable all read
    /// `Store::Shared` and only the tool tells their accounts apart.
    pub tool: ToolKind,
}

impl Shape {
    /// Whether `other` runs on a PROVEN different account: another tool, or
    /// the same tool on another proven store. The same store, or the one
    /// store a tool without an account variable has, is `Some(false)`.
    /// Everything ae cannot prove is `None` — an unknown store, a tool it
    /// cannot name, and a proven store against a shared one (one tool never
    /// has both) — so no doubt reads as another account.
    #[must_use]
    pub fn off_account(&self, other: &Self) -> Option<bool> {
        if !self.tool.is_known() || !other.tool.is_known() {
            return None;
        }
        if self.tool != other.tool {
            return Some(true);
        }
        match (&self.store, &other.store) {
            (Store::Proven(one), Store::Proven(two)) => Some(one != two),
            (Store::Shared, Store::Shared) => Some(false),
            _ => None,
        }
    }

    /// The same setup on the same store, PROVEN: an unknown account is never
    /// the same as another, and on a shared store only the same command byte
    /// for byte is, since a respelled flag proves nothing.
    fn same(&self, other: &Self) -> bool {
        self.words == other.words
            && self.pin == other.pin
            && match (&self.store, &other.store) {
                (Store::Proven(one), Store::Proven(two)) => one == two,
                (Store::Shared, Store::Shared) => self.raw == other.raw,
                _ => false,
            }
    }
}

/// The seat's candidates: `declared` in its own order, then the seat's own
/// family among `profiles` — every twin, then every sibling, each in
/// `profiles` order. A profile is listed once, where it first appears, and a
/// derived one that is the [`Shape::same`] as one listed before it is dropped:
/// its windows are that one's. The seat's own profile is never listed.
#[must_use]
pub fn listed(
    declared: &[String],
    (profile, seat): (&str, &Shape),
    profiles: &[(String, Shape)],
) -> Vec<(String, Provenance)> {
    let shape = |name: &str| {
        profiles
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, shape)| shape)
    };
    let mut list: Vec<(String, Provenance)> = declared
        .iter()
        .map(|name| (name.clone(), Provenance::Declared))
        .collect();
    let mut kept: Vec<&Shape> = declared.iter().filter_map(|name| shape(name)).collect();
    for kin in [Kin::Twin, Kin::Sibling] {
        for (name, candidate) in profiles {
            let family = candidate.words == seat.words
                && match kin {
                    Kin::Twin => {
                        candidate.pin == seat.pin
                            && matches!(
                                (&seat.store, &candidate.store),
                                (Store::Proven(one), Store::Proven(two)) if one != two
                            )
                    }
                    Kin::Sibling => {
                        seat.pin.is_some() && candidate.pin.is_some() && candidate.pin != seat.pin
                    }
                };
            if !family
                || name == profile
                || list.iter().any(|(taken, _)| taken == name)
                || kept.iter().any(|shape| shape.same(candidate))
            {
                continue;
            }
            kept.push(candidate);
            list.push((name.clone(), Provenance::Derived(kin)));
        }
    }
    list
}

/// Whether a profile has a way off its own account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cover {
    /// The first listed candidate that runs on a PROVEN other account.
    Exit(String),
    /// Every listed candidate is proven on the profile's own account, or none
    /// is listed: its siblings share the allowance a limit exhausts.
    Bare,
    /// No exit is proven, and an account is why.
    Unproven(Doubt),
}

/// Whose account an unproven [`Cover`] could not name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Doubt {
    /// The profile's own: its tool or its account ae cannot name, or its
    /// command does not read at all.
    Own,
    /// The profile's own is known, and a listed candidate's is not.
    Candidate,
}

/// The cover verdict of one profile over the ONE candidate list [`listed`]
/// builds: the first candidate on a proven other account is the exit, a
/// candidate answering to no `profiles` entry is none, and an account ae
/// cannot prove is never taken for another one.
///
/// Siblings are candidates and not exits unless they leave the account: a
/// model on the same allowance does not outlast it.
#[must_use]
pub fn cover(
    declared: &[String],
    (profile, seat): (&str, &Shape),
    profiles: &[(String, Shape)],
) -> Cover {
    let mut unproven = false;
    for (name, _) in listed(declared, (profile, seat), profiles) {
        let Some((_, shape)) = profiles.iter().find(|(key, _)| *key == name) else {
            continue;
        };
        match seat.off_account(shape) {
            Some(true) => return Cover::Exit(name),
            Some(false) => {}
            None => unproven = true,
        }
    }
    if !seat.tool.is_known() || seat.store == Store::Unknown {
        Cover::Unproven(Doubt::Own)
    } else if unproven {
        Cover::Unproven(Doubt::Candidate)
    } else {
        Cover::Bare
    }
}

/// At most two profiles to name in the `[auto_reseat]` line that would cover
/// `profile`: those the declared rows already name, in order of first
/// appearance, else the `profiles` in config order — each PROVEN off its
/// account, never the profile itself and never a name `profiles` lacks. The
/// second prefers another tool than the first.
#[must_use]
pub fn suggest(
    profile: &str,
    own: &Shape,
    rows: &[(String, Vec<String>)],
    profiles: &[(String, Shape)],
) -> Vec<String> {
    let eligible = |names: Vec<&str>| -> Vec<(String, ToolKind)> {
        let mut seen: Vec<&str> = Vec::new();
        let mut found = Vec::new();
        for name in names {
            if name == profile || seen.contains(&name) {
                continue;
            }
            seen.push(name);
            if let Some((_, shape)) = profiles.iter().find(|(key, _)| key == name)
                && own.off_account(shape) == Some(true)
            {
                found.push((name.to_owned(), shape.tool));
            }
        }
        found
    };
    let mut found = eligible(
        rows.iter()
            .flat_map(|(_, list)| list.iter().map(String::as_str))
            .collect(),
    );
    if found.is_empty() {
        found = eligible(profiles.iter().map(|(key, _)| key.as_str()).collect());
    }
    let Some(first) = found.first() else {
        return Vec::new();
    };
    let second = found
        .iter()
        .skip(1)
        .find(|(_, tool)| *tool != first.1)
        .or_else(|| found.get(1));
    std::iter::once(first)
        .chain(second)
        .map(|(name, _)| name.clone())
        .collect()
}

/// The account a profile draws on, as the doctor row groups profiles by it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// A tool and the store it keeps its account in.
    Account { tool: ToolKind, store: Store },
    /// The profile's command does not read, so it names no account.
    Unreadable,
}

impl Scope {
    /// How a row names the scope: the tool and its account.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Account { tool, store } => {
                let name = tool.as_str();
                match store {
                    Store::Proven(path) => format!("{name} · {}", path.display()),
                    Store::Shared => format!("{name} · shared"),
                    Store::Unknown => format!("{name} · account not provable"),
                }
            }
            Self::Unreadable => "? · command not readable".to_owned(),
        }
    }
}

/// One `[profiles]` entry's cover, with the scope it belongs to and the
/// profiles its `[auto_reseat]` line could name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileCover {
    pub profile: String,
    pub scope: Scope,
    pub cover: Cover,
    /// Empty for a covered profile, and when no profile leaves its account.
    pub suggested: Vec<String>,
}

/// The one stderr line `spawn` and a human's `reseat` print for a profile
/// without an exit off its account, or `None` when it has one.
#[must_use]
pub fn nudge_line(found: &ProfileCover) -> Option<String> {
    let name = &found.profile;
    let add = (!found.suggested.is_empty()).then(|| {
        format!(
            "add under [auto_reseat]: {name} = {}",
            found.suggested.join(", ")
        )
    });
    match &found.cover {
        Cover::Exit(_) => None,
        Cover::Bare => Some(format!(
            "ae: warning: profile '{name}' has no auto-reseat exit off its account — a usage \
             limit would leave the seat stuck; {}",
            add.unwrap_or_else(|| {
                "no profile in [profiles] runs on another account, add one first".to_owned()
            })
        )),
        Cover::Unproven(Doubt::Own) => Some(format!(
            "ae: warning: profile '{name}' cannot prove an auto-reseat exit off its account — \
             its own account is not provable; a usage limit may leave the seat stuck{}",
            add.map(|line| format!("; {line}")).unwrap_or_default()
        )),
        Cover::Unproven(Doubt::Candidate) => Some(format!(
            "ae: warning: profile '{name}' cannot prove an auto-reseat exit off its account — a \
             candidate's account is not provable; a usage limit may leave the seat stuck{}",
            add.map(|line| format!("; {line}")).unwrap_or_default()
        )),
    }
}

/// What ended an episode's auto path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Refused,
    Failed,
}

/// What opened an episode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// The vendor's usage limit, drawn in the seat's pane.
    Limit,
    /// The seat's own account read at or past `auto_reseat_at`.
    Headroom,
}

/// One episode of one seat, as the journal records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Episode {
    /// The `ts` of the episode's opening record.
    pub key: Timestamp,
    reference: String,
    pub trigger: Trigger,
    /// The opener's summary, quoted whole by every record that names the
    /// trigger; empty for a limit episode, whose records stay as they were.
    cause: String,
    /// Attempts of the current round.
    pub attempts: u8,
    /// Refused or failed closings taken under this key, all rounds.
    pub rounds: u8,
    /// When the current round's first closing was journaled.
    closed_at: Option<Timestamp>,
    /// The newest attempt that has no outcome yet.
    pub open: Option<Timestamp>,
    /// The first terminal outcome of the current round. A done is final; a
    /// refused or failed limit round is reopened once [`Self::cooled`].
    pub terminal: Option<Outcome>,
    /// The newest record of this path is a hold.
    pub held: bool,
    /// The summary of the newest hold, whole.
    last_hold: Option<String>,
}

impl Episode {
    /// The key as every record of this episode spells its `ref`.
    #[must_use]
    pub fn reference(&self) -> &str {
        &self.reference
    }

    /// What every record of a headroom episode names its trigger by; empty for
    /// a limit episode.
    #[must_use]
    pub fn cause(&self) -> &str {
        &self.cause
    }

    /// Whether the newest record of this path is a hold summarised `summary`:
    /// naming it again would say nothing new, a daemon restart included.
    #[must_use]
    pub fn holds_with(&self, summary: &str) -> bool {
        self.held && self.last_hold.as_deref() == Some(summary)
    }

    /// Whether the last round of this limit episode was refused or failed:
    /// the path is over for good, and the seat is owed one word of it.
    #[must_use]
    pub fn gave_up(&self) -> bool {
        self.trigger == Trigger::Limit
            && matches!(self.terminal, Some(Outcome::Refused | Outcome::Failed))
            && self.rounds >= MAX_ROUNDS
    }

    /// Whether a refused or failed limit round, closed at `closed_at`, may be
    /// tried again at `at`: the cooldown has passed and rounds are left. A
    /// headroom episode stays closed until its re-arm.
    fn cooled(&self, at: i64) -> bool {
        self.trigger == Trigger::Limit
            && matches!(self.terminal, Some(Outcome::Refused | Outcome::Failed))
            && self.rounds < MAX_ROUNDS
            && self
                .closed_at
                .is_some_and(|closed| at >= closed.epoch().saturating_add(RETRY_COOLDOWN_SECS))
    }

    fn opened(key: Timestamp) -> Self {
        Self::opened_by(key, Trigger::Limit, String::new())
    }

    fn opened_by(key: Timestamp, trigger: Trigger, cause: String) -> Self {
        Self {
            key,
            reference: key.to_string(),
            trigger,
            cause,
            attempts: 0,
            rounds: 0,
            closed_at: None,
            open: None,
            terminal: None,
            held: false,
            last_hold: None,
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
                self.last_hold.clone_from(&event.summary);
            }
            DONE_ACTION => self.close(Outcome::Done, event.ts),
            REFUSED_ACTION if ours => self.close(Outcome::Refused, event.ts),
            FAILED_ACTION if ours => self.close(Outcome::Failed, event.ts),
            _ => {}
        }
    }

    /// The first outcome of a round closes it; a refused or failed one counts
    /// a round and starts its cooldown. A later closing of the same round
    /// changes nothing — not even the hold it may have followed — except a
    /// done, which is final: the seat moved.
    fn close(&mut self, outcome: Outcome, at: Timestamp) {
        let moved = matches!(self.terminal, Some(Outcome::Refused | Outcome::Failed))
            && outcome == Outcome::Done;
        if self.terminal.is_some() && !moved {
            return;
        }
        self.open = None;
        self.held = false;
        self.terminal = Some(outcome);
        if outcome != Outcome::Done {
            self.rounds = self.rounds.saturating_add(1);
            self.closed_at = Some(at);
        }
    }

    /// A new round: the cooldown passed, so the closed attempts count for
    /// nothing and the next attempt opens under the same key.
    fn reopen(&mut self) {
        self.terminal = None;
        self.attempts = 0;
        self.open = None;
        self.held = false;
    }
}

/// The episode the seat is in now, or `None` when it has none.
#[must_use]
pub fn episode(events: &[Event], session: &str, slot: &str, agent: &str) -> Option<Episode> {
    episodes(events, session, slot, agent).0
}

/// The headroom episode the seat is in now, or `None` when it is armed.
///
/// A terminal outcome keeps the episode until its re-arm record: a seat whose
/// candidates all refused is refused once, not once per cycle. A move ends it
/// whatever opened the move, because the seat's account is no longer the one
/// that crossed.
#[must_use]
pub fn headroom_episode(
    events: &[Event],
    session: &str,
    slot: &str,
    agent: &str,
) -> Option<Episode> {
    episodes(events, session, slot, agent).1
}

/// The seat's limit episode and its headroom episode, folded in ONE pass,
/// because a seat has one auto reseat path: an outcome under the live key of
/// either — or a move, whatever its ref — closes every episode still open
/// beside it. An outcome under a key neither holds closes nothing. A closed
/// limit episode waits for its `alert-cleared`, a closed headroom one for its
/// re-arm, before a new one opens. A limit episode closed `refused` or
/// `failed` is the exception: its next attempt or closing, journaled after its
/// cooldown under the same key, opens a new round ([`Episode::cooled`]).
fn episodes(
    events: &[Event],
    session: &str,
    slot: &str,
    agent: &str,
) -> (Option<Episode>, Option<Episode>) {
    let mut limit: Option<Episode> = None;
    let mut headroom: Option<Episode> = None;
    for event in events {
        if !event_is_addressed_to(event, session, slot, agent) {
            continue;
        }
        if event.action == SPAWN_ACTION {
            (limit, headroom) = (None, None);
            continue;
        }
        if event.actor != WATCHDOG_ACTOR {
            continue;
        }
        if let Some(open) = limit.as_mut()
            && matches!(
                event.action.as_str(),
                ATTEMPT_ACTION | REFUSED_ACTION | FAILED_ACTION
            )
            && event.reference.as_deref() == Some(open.reference())
            && open.cooled(event.ts.epoch())
        {
            open.reopen();
        }
        let live = |found: &Option<Episode>| {
            found.as_ref().is_some_and(|open| {
                open.terminal.is_none() && event.reference.as_deref() == Some(open.reference())
            })
        };
        match event.action.as_str() {
            CLEARED_ACTION => limit = None,
            LIMIT_ACTION => {
                if limit.is_none() {
                    limit = Some(Episode::opened(event.ts));
                }
            }
            HEADROOM_ACTION => {
                if headroom.is_none() {
                    let cause = event.summary.clone().unwrap_or_default();
                    headroom = Some(Episode::opened_by(event.ts, Trigger::Headroom, cause));
                }
            }
            REARMED_ACTION => {
                if headroom
                    .as_ref()
                    .is_some_and(|open| event.reference.as_deref() == Some(open.reference()))
                {
                    headroom = None;
                }
            }
            DONE_ACTION => {
                if let Some(open) = limit.as_mut() {
                    open.absorb(event);
                }
                headroom = None;
            }
            REFUSED_ACTION | FAILED_ACTION if live(&limit) || live(&headroom) => {
                let outcome = if event.action == REFUSED_ACTION {
                    Outcome::Refused
                } else {
                    Outcome::Failed
                };
                for open in [limit.as_mut(), headroom.as_mut()].into_iter().flatten() {
                    if open.terminal.is_none() {
                        open.close(outcome, event.ts);
                    }
                }
            }
            _ => {
                for open in [limit.as_mut(), headroom.as_mut()].into_iter().flatten() {
                    open.absorb(event);
                }
            }
        }
    }
    (limit, headroom)
}

/// The episode, of either kind, whose attempt is in flight: opened, no
/// outcome yet. It alone decides the seat until that attempt ends, so no second
/// attempt opens beside it; a limit drawn meanwhile supersedes only a headroom
/// episode no attempt is moving.
#[must_use]
pub fn flying(limit: Option<&Episode>, headroom: Option<&Episode>) -> Option<Episode> {
    [limit, headroom]
        .into_iter()
        .flatten()
        .find(|found| found.open.is_some() && found.terminal.is_none())
        .cloned()
}

/// The episode keyed `key` that a leg acts under. A limit and a headroom
/// episode opened in the same second share a key, so the one whose attempt is
/// in flight is taken first; otherwise the limit episode, then the headroom one.
#[must_use]
pub fn keyed(
    events: &[Event],
    (session, slot, agent): (&str, &str, &str),
    key: Timestamp,
) -> Option<Episode> {
    let (limit, headroom) = episodes(events, session, slot, agent);
    owning(limit, headroom, &key.to_string())
}

/// Of the two episodes, the one whose `ref` is `reference`, by [`keyed`]'s rule.
fn owning(limit: Option<Episode>, headroom: Option<Episode>, reference: &str) -> Option<Episode> {
    let named = |found: &Episode| found.reference() == reference;
    let (limit, headroom) = (limit.filter(named), headroom.filter(named));
    flying(limit.as_ref(), headroom.as_ref())
        .or(limit)
        .or(headroom)
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
    left_moves(events, session, slot, agent)
        .into_iter()
        .map(|(profile, at, _)| (profile, at))
        .collect()
}

/// The profiles of [`left_profiles`] whose newest move closed a HEADROOM
/// episode: the attempt before it named a headroom key.
#[must_use]
pub fn left_on_headroom(events: &[Event], session: &str, slot: &str, agent: &str) -> Vec<String> {
    left_moves(events, session, slot, agent)
        .into_iter()
        .filter(|(_, _, trigger)| *trigger == Trigger::Headroom)
        .map(|(profile, _, _)| profile)
        .collect()
}

fn left_moves(
    events: &[Event],
    session: &str,
    slot: &str,
    agent: &str,
) -> Vec<(String, Timestamp, Trigger)> {
    let mut left: Vec<(String, Timestamp, Trigger)> = Vec::new();
    // The kind of the newest attempt, judged when it was journaled: a later
    // opener under the same second cannot change which move it was.
    let mut attempted: Option<Trigger> = None;
    for (index, event) in events.iter().enumerate() {
        if !event_is_addressed_to(event, session, slot, agent) {
            continue;
        }
        if event.action == SPAWN_ACTION {
            left.clear();
            attempted = None;
            continue;
        }
        if event.actor != WATCHDOG_ACTOR {
            continue;
        }
        match event.action.as_str() {
            ATTEMPT_ACTION => {
                let (limit, headroom) = episodes(&events[..=index], session, slot, agent);
                attempted = event
                    .reference
                    .as_deref()
                    .and_then(|reference| owning(limit, headroom, reference))
                    .map(|found| found.trigger);
            }
            DONE_ACTION => {
                if let Some(profile) = event.reference.as_deref() {
                    let trigger = attempted.unwrap_or(Trigger::Limit);
                    left.retain(|(taken, _, _)| taken != profile);
                    left.push((profile.to_owned(), event.ts, trigger));
                }
                attempted = None;
            }
            _ => {}
        }
    }
    left
}

/// What the latest capture of the seat's pane proved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame {
    /// Read, idle, with an empty input box.
    Clear,
    /// Read, not busy and no draft, but the frame is not one ae can prove
    /// idle: a limit move treats it as clear, a headroom move holds.
    Unproven,
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
    Unproven,
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
            Self::Unproven => "held: the seat's frame is not proven idle",
        }
    }

    /// The class the record names it under: an unreadable or unproven frame is
    /// `unknown-frame`; a turn, a draft, a prompt or human input is `busy`.
    #[must_use]
    pub const fn class(self) -> CauseClass {
        match self {
            Self::Unread | Self::Unproven => CauseClass::UnknownFrame,
            Self::Busy | Self::Draft | Self::HumanPrompt | Self::ClientInput => CauseClass::Busy,
        }
    }

    /// The summary the `auto-reseat-held` record carries: the class, then
    /// [`Self::summary`].
    #[must_use]
    pub fn record(self) -> String {
        self.class().lead(self.summary())
    }
}

/// What to do for a latched, eligible seat this cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Nothing now: no episode, a terminal outcome (a refused or failed limit
    /// round until its cooldown has passed and while rounds are left), or
    /// attempts spent.
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

/// Decide for one seat whose limit latch stands, or whose headroom episode is
/// still due.
///
/// An open attempt is judged before the spent count, so the second attempt
/// keeps its bound; the grace is judged before the pane, so nothing about the
/// pane is read into a hold while the human still has time to react. A
/// headroom move never ends a turn, so it also holds on a frame ae cannot
/// prove idle; a limit move reads that frame as clear, as it always has.
#[must_use]
pub fn decide(episode: Option<&Episode>, grace_secs: u64, pane: &Pane, now: i64) -> Decision {
    let Some(episode) = episode.filter(|found| found.terminal.is_none() || found.cooled(now))
    else {
        return Decision::Rest;
    };
    if let Some(started) = episode.open {
        return if now.saturating_sub(started.epoch()) < IN_FLIGHT_SECS {
            Decision::InFlight
        } else {
            Decision::Overdue
        };
    }
    // A cooled episode's count is its closed round's: the next one starts anew.
    if episode.terminal.is_none() && episode.attempts >= MAX_ATTEMPTS {
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
    let headroom = episode.trigger == Trigger::Headroom;
    let reason = match pane.frame {
        Frame::Unread => Some(HoldReason::Unread),
        Frame::Busy => Some(HoldReason::Busy),
        Frame::Draft => Some(HoldReason::Draft),
        Frame::Clear | Frame::Unproven if pane.human_prompt => Some(HoldReason::HumanPrompt),
        Frame::Unproven if headroom => Some(HoldReason::Unproven),
        Frame::Clear | Frame::Unproven if touched => Some(HoldReason::ClientInput),
        Frame::Clear | Frame::Unproven => None,
    };
    reason.map_or(Decision::Attempt, Decision::Hold)
}

/// The frame one capture proves: a read that failed proves nothing, a running
/// turn is busy, a human's text in the box is theirs, and only a frame the
/// classifier reads idle is clear.
#[must_use]
pub fn frame_of(read: bool, state: HarnessState, draft: bool) -> Frame {
    if !read {
        Frame::Unread
    } else if state == HarnessState::Busy {
        Frame::Busy
    } else if draft {
        Frame::Draft
    } else if state == HarnessState::Idle {
        Frame::Clear
    } else {
        Frame::Unproven
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

/// [`in_flight`] for the seat's headroom episode: the shell its move leaves is
/// the move, not a death. It holds no limit latch.
#[must_use]
pub fn headroom_in_flight(
    settings: &Settings,
    events: &[Event],
    (session, slot, agent): (&str, &str, &str),
    now: i64,
) -> bool {
    settings.switch != Switch::Off
        && headroom_episode(events, session, slot, agent)
            .and_then(|found| found.open)
            .is_some_and(|started| now.saturating_sub(started.epoch()) < IN_FLIGHT_SECS)
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
    /// The vendor stated a length of at least a day ([`crate::quota::proven_long`]):
    /// only such a window decides headroom.
    pub long: bool,
    pub observed_at: i64,
    pub resets_at: Option<i64>,
    pub status: Status,
    /// The window as the quota table names it, for a record.
    pub label: String,
}

/// What the leg knows about one candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub profile: String,
    pub provenance: Provenance,
    /// The profile resolves the way a launch resolves one.
    pub configured: bool,
    /// Another seat of the session is latched on this candidate's account.
    pub peer_latched: bool,
    pub windows: Vec<Window>,
    /// When this seat last left this profile by auto reseat.
    pub left_at: Option<i64>,
    /// That move closed a headroom episode.
    pub left_on_headroom: bool,
}

/// Why a candidate was passed over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    Unconfigured,
    PeerLatched,
    Exhausted,
    LeftOnLimit,
    /// A usable long window, or a critical one, judged at or past the headroom
    /// threshold.
    NoRoom,
    LeftOnHeadroom,
}

/// The candidate taken, and every one passed over with its reason, named as a
/// record names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub pick: Option<(String, Tier)>,
    /// How the pick came to be listed.
    pub provenance: Provenance,
    pub skipped: Vec<(String, Skip)>,
}

/// Take the first usable candidate of the best tier, in listed order — a
/// declared one before any derived one, whatever its tier: the row is the
/// human's own preference, derivation only ae's fallback.
#[must_use]
pub fn choose(candidates: &[Candidate], now: i64) -> Choice {
    choose_with(candidates, now, None)
}

/// [`choose`] for an episode opened at the headroom threshold `room`: a
/// candidate must have room below it. `None` is a limit episode's chooser.
#[must_use]
pub fn choose_with(candidates: &[Candidate], now: i64, room: Option<u8>) -> Choice {
    let mut skipped = Vec::new();
    let mut best: Option<(&Candidate, (bool, Tier))> = None;
    for candidate in candidates {
        if let Some(skip) = passed_over(candidate, now, room) {
            let named = format!("{}{}", candidate.profile, candidate.provenance.word());
            skipped.push((named, skip));
            continue;
        }
        let rank = (
            candidate.provenance != Provenance::Declared,
            tier(&candidate.windows),
        );
        if best.is_none_or(|(_, held)| rank < held) {
            best = Some((candidate, rank));
        }
    }
    Choice {
        pick: best.map(|(candidate, (_, tier))| (candidate.profile.clone(), tier)),
        provenance: best.map_or(Provenance::Declared, |(candidate, _)| candidate.provenance),
        skipped,
    }
}

/// Why `candidate` cannot be taken now, if it cannot.
///
/// A profile the seat left on its limit waits for a window read after the move
/// that proves relief — a usable reading below critical, or a reset that has
/// passed — and for no usable reading after the move to still sit at critical:
/// a move back onto a nearly spent account would only move the seat again. A
/// profile left on headroom waits the same way, its bar the threshold `room`
/// names rather than critical. A number ae cannot use proves nothing either
/// way. A spend cap needs no rule of its own, because it judges as exhausted.
/// With `room`, a candidate with a usable window at or past it has none — a
/// long window, or a short one the classifier put at critical, so a seat never
/// moves onto an account whose short window is spent.
fn passed_over(candidate: &Candidate, now: i64, room: Option<u8>) -> Option<Skip> {
    if !candidate.configured {
        return Some(Skip::Unconfigured);
    }
    if candidate.peer_latched {
        return Some(Skip::PeerLatched);
    }
    let over = |window: &Window, at: u8| {
        window.judged >= f64::from(at) && (window.long || window.critical)
    };
    let crossed = |window: &Window| room.is_some_and(|at| over(window, at));
    if let Some(left) = candidate.left_at {
        let bar = room.filter(|_| candidate.left_on_headroom);
        let spent = |window: &Window| bar.map_or(window.critical, |at| over(window, at));
        let still = if candidate.left_on_headroom {
            Skip::LeftOnHeadroom
        } else {
            Skip::LeftOnLimit
        };
        let mut relieved = false;
        for window in candidate
            .windows
            .iter()
            .filter(|window| window.observed_at > left)
        {
            if window.resets_at.is_some_and(|at| at <= now) {
                relieved = true;
            } else if usable(window) {
                if spent(window) {
                    return Some(still);
                }
                relieved = true;
            }
        }
        if !relieved {
            return Some(still);
        }
    }
    let usable_windows = || candidate.windows.iter().filter(|window| usable(window));
    if usable_windows().any(|window| window.judged >= EXHAUSTED_PERCENT) {
        return Some(Skip::Exhausted);
    }
    usable_windows().any(crossed).then_some(Skip::NoRoom)
}

/// What the seat's OWN account says about its headroom, judged against the
/// threshold `at`: the worst usable window proven at least a day long decides.
#[derive(Debug, Clone, PartialEq)]
pub enum Room {
    /// No usable window of the seat's own account: nothing is decided.
    Unread,
    /// The worst window is at or past the threshold.
    Crossed(Window),
    /// The worst window is below the threshold, but not by [`REARM_GAP`].
    Near,
    /// Every usable window reads more than [`REARM_GAP`] below the threshold.
    Relieved(Window),
}

/// The worst usable window proven at least a day long: the one window that
/// decides how pressed a seat's own account is.
#[must_use]
pub fn worst_long(windows: &[Window]) -> Option<&Window> {
    windows
        .iter()
        .filter(|window| window.long && usable(window))
        .max_by(|left, right| left.judged.total_cmp(&right.judged))
}

/// Judge the seat's own `windows` against the threshold `at`.
#[must_use]
pub fn room(windows: &[Window], at: u8) -> Room {
    let at = f64::from(at);
    match worst_long(windows) {
        None => Room::Unread,
        Some(window) if window.judged >= at => Room::Crossed(window.clone()),
        Some(window) if window.judged < at - REARM_GAP => Room::Relieved(window.clone()),
        Some(_) => Room::Near,
    }
}

/// How a record names a reading: `headroom <pct>% <window>`.
#[must_use]
pub fn headroom_words(window: &Window) -> String {
    format!("headroom {}% {}", percent(window.judged), window.label)
}

/// The re-arm record's summary.
#[must_use]
pub fn relieved_words(window: &Window) -> String {
    format!(
        "headroom cleared: {}% {}",
        percent(window.judged),
        window.label
    )
}

/// One decimal, a whole number without it.
fn percent(value: f64) -> String {
    let text = format!("{value:.1}");
    text.strip_suffix(".0")
        .map_or_else(|| text.clone(), ToOwned::to_owned)
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
    roster
        .iter()
        .filter(|entry| entry.slot != moving)
        .filter(|entry| episode(events, session, &entry.slot, &entry.name).is_some())
        .filter_map(crate::quota::recorded_identity)
        .collect()
}

/// Who opened the seat: the actor of the newest `spawn` record addressed to it.
#[must_use]
pub fn spawner<'e>(events: &'e [Event], session: &str, slot: &str, agent: &str) -> Option<&'e str> {
    events
        .iter()
        .rev()
        .find(|event| {
            event.action == SPAWN_ACTION && event_is_addressed_to(event, session, slot, agent)
        })
        .map(|event| event.actor.as_str())
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
    let mut told: Vec<String> = roster
        .iter()
        .filter(|entry| entry.slot != moved)
        .filter(|entry| crate::watchdog_daemon::in_lead_pair(&entry.slot, lead_pair))
        .map(|entry| entry.name.clone())
        .collect();
    let by = spawner.and_then(|name| roster.iter().find(|entry| entry.name == name));
    if let Some(by) = by.filter(|by| by.slot != moved && !told.contains(&by.name)) {
        told.push(by.name.clone());
    }
    told
}

/// How one ending of the auto path is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending<'a> {
    Moved(Move<'a>),
    /// Held, quoting the record's summary under its class. The refused and
    /// failed endings carry the same pair.
    Held(CauseClass, &'a str),
    Refused(CauseClass, &'a str),
    Failed(CauseClass, &'a str),
    Unmapped(&'a str),
    /// The last round was refused or failed: no further attempt.
    GaveUp,
}

impl Ending<'_> {
    /// The cause class that leads the notice; a move has none.
    const fn class(&self) -> Option<CauseClass> {
        match self {
            Self::Moved(_) => None,
            Self::Held(class, _) | Self::Refused(class, _) | Self::Failed(class, _) => Some(*class),
            Self::Unmapped(_) => Some(CauseClass::NoCandidate),
            Self::GaveUp => Some(CauseClass::Other),
        }
    }
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
    /// What became of the conversation.
    pub fate: Fate,
    /// How the one continuation turn a move whose conversation continued was
    /// handed went; `None` for a seeded move.
    pub nudge: Option<Nudge>,
    /// The target was already critical when it was chosen.
    pub critical: bool,
    /// The seat's work tree has tracked changes.
    pub dirty: bool,
    /// The headroom trigger, as its episode names it; empty for a limit.
    pub cause: &'a str,
}

/// What a move did with the seat's conversation — ONE spelling for every
/// record and notice that names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fate {
    /// A fresh conversation, handed the seed pack.
    Seeded,
    /// Copied into another account of the same tool.
    Carried,
    /// Resumed in place on another model or effort.
    Kept,
}

impl Fate {
    /// The word a record or a notice says it with.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Seeded => "seeded",
            Self::Carried => "carried",
            Self::Kept => "kept",
        }
    }
}

/// How the continuation turn a carried move hands its seat went, as the
/// session's `send` helper left it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nudge {
    /// Exited 0 and recorded the submit as proven.
    Submitted,
    /// Exited 0, but its record says the submit was not proven, or ae could not
    /// find that record.
    Unconfirmed,
    /// Exited non-zero: refused before the paste, or pasted and not recorded.
    Refused,
}

/// The widest a notice may be, in characters.
pub const NOTICE_CHARS: usize = 400;

/// The line for a limit episode whose last round was refused or failed.
fn gave_up_line(agent: &str, hand: &str) -> String {
    format!(
        "{agent} not moved — auto reseat gave up after {MAX_ROUNDS} refused or failed moves. \
         Next: {hand} or wait for the reset."
    )
}

/// The ONE line every recipient and the chat are told for `ending`.
#[must_use]
pub fn notice(session: &str, agent: &str, ending: &Ending<'_>) -> String {
    // The class of a hold, refusal or failure leads the notice, ahead of the
    // agent and session names, so no cut of what follows can take it.
    let class = ending.class();
    let hand = format!("move it by hand (ae reseat {session} {agent} --using <profile>)");
    let line = match ending {
        Ending::Moved(moved) => format!(
            "{agent} moved {} -> {}{} (tool {} -> {}, model {} -> {}), {}{}; work tree {}. If \
             {agent} is half of a review pair, re-check its gate provider. Next: {}.",
            moved.from,
            moved.to,
            if moved.cause.is_empty() {
                String::new()
            } else {
                format!(" on {}", moved.cause)
            },
            moved.tool.0,
            moved.tool.1,
            moved.model.0,
            moved.model.1,
            moved.fate.word(),
            if moved.critical {
                ", target already critical"
            } else {
                ""
            },
            if moved.dirty { "dirty" } else { "clean" },
            // Never a resend the pane does not call for: a nudge ae could not
            // prove absent may already be there.
            match moved.nudge {
                None => "check it picked up its seat pack".to_owned(),
                Some(Nudge::Submitted) =>
                    "nothing, its continuation nudge was submitted".to_owned(),
                Some(Nudge::Unconfirmed) => format!(
                    "its continuation nudge is unconfirmed: peek {agent}; re-send a resume turn \
                     only if the nudge still sits in its input box"
                ),
                Some(Nudge::Refused) => format!(
                    "its continuation nudge was refused or unrecorded: peek {agent}; send it one \
                     turn to resume only if no nudge reached its pane"
                ),
            },
        ),
        Ending::Held(class, why) => {
            let why = class.details(why);
            format!("{agent} not moved yet — {why}. Next: ae tries once more if it stays eligible.")
        }
        Ending::Refused(class, why) => {
            let why = class.details(why);
            let next = why.find("once its box is empty").and_then(|at| {
                why.get(at..at + why[at..].find(" --stop-unknown")? + " --stop-unknown".len())
            });
            format!(
                "{agent} not moved — {why}. Next: {}.",
                next.unwrap_or(&format!("{hand} or wait for the reset"))
            )
        }
        Ending::Unmapped(profile) => format!(
            "{agent} not moved — no declared or derived candidate for {profile}. Next: add an \
             [auto_reseat] row for {profile}, or {hand}."
        ),
        Ending::Failed(class, why) => format!(
            "{agent} move failed — {}. Next: relaunch {agent} if its pane sits at a shell, \
             else reseat it by hand.",
            class.details(why)
        ),
        Ending::GaveUp => gave_up_line(agent, &hand),
    };
    let head = class.map_or_else(
        || "auto reseat: ".to_owned(),
        |class| format!("auto reseat: {}: ", class.word()),
    );
    let line = crate::seatpack::neutralise(&line);
    // A cut takes the details, never the head or the step after them: the head
    // names the cause and the step is what the human acts on, and half a
    // resend condition could ask for a duplicate. Cut BEFORE `summary_of`,
    // whose own cap would otherwise take the step first.
    let at = line.rfind(" Next: ").unwrap_or(line.len());
    let (body, step) = line.split_at(at);
    // A profile name is unbounded, so a step that quotes one can fail to fit
    // beside the head at all. It is then replaced WHOLE by a fixed one that
    // names only the session and the agent: a clipped command is worse than a
    // shorter one.
    let short = match ending {
        Ending::Refused(..) => Some(format!("Next: {hand} or wait for the reset.")),
        Ending::Unmapped(_) => Some(format!(
            "Next: add an [auto_reseat] row for the profile, or {hand}."
        )),
        _ => None,
    };
    let step = match short {
        Some(short) if head.chars().count() + step.chars().count() > NOTICE_CHARS => {
            format!(" {}", crate::seatpack::neutralise(&short))
        }
        _ => step.to_owned(),
    };
    let room = NOTICE_CHARS.saturating_sub(head.chars().count() + step.chars().count());
    let cut: String = head
        .chars()
        .chain(body.chars().take(room))
        .chain(step.chars())
        .collect();
    crate::state::summary_of(&cut)
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
            headroom_at: Some(DEFAULT_HEADROOM_AT),
            headroom_note: None,
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
        // A second closing inside the same round is the same round's: the
        // first of a round wins.
        let mut doubled = events.clone();
        doubled.push(routed(840, REFUSED_ACTION, KEY));
        let first = fold(&doubled).expect("an episode");
        assert_eq!((first.open, first.terminal), (None, Some(Outcome::Failed)));
        // A done that lands after a failed closing still ends the path: the
        // seat moved, and a refused or failed round is no longer final.
        events.push(routed(785, DONE_ACTION, "sol6x"));
        events.push(routed(840, REFUSED_ACTION, KEY));
        let closed = fold(&events).expect("an episode");
        assert_eq!((closed.open, closed.terminal), (None, Some(Outcome::Done)));
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
            long: true,
            observed_at,
            resets_at: None,
            status,
            label: "weekly_all 7d".to_owned(),
        }
    }

    fn shape(pin: Option<&str>, store: Store) -> Shape {
        Shape {
            words: vec!["claude".to_owned(), "--model".to_owned()],
            pin: pin.map(ToOwned::to_owned),
            store,
            raw: String::new(),
            tool: ToolKind::Claude,
        }
    }

    /// A shape of `tool`: its executable word differs per tool, as a real
    /// command's does, so two tools are never one family.
    fn on(tool: ToolKind, pin: Option<&str>, store: Store) -> Shape {
        Shape {
            words: vec![tool.as_str().to_owned(), "--model".to_owned()],
            tool,
            ..shape(pin, store)
        }
    }

    fn proven(path: &str) -> Store {
        Store::Proven(std::path::PathBuf::from(path))
    }

    fn rows<T: Clone>(rows: &[(&str, T)]) -> Vec<(String, T)> {
        rows.iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect()
    }

    #[test]
    fn the_family_lists_twins_then_siblings_and_collapses_only_a_proven_alias() {
        let seat = shape(Some("opus"), proven("/a"));
        let twin = Provenance::Derived(Kin::Twin);
        let sibling = Provenance::Derived(Kin::Sibling);
        let other = Shape {
            words: vec!["claude".to_owned(), "--model".to_owned(), "low".to_owned()],
            ..shape(Some("opus"), proven("/d"))
        };
        let all = rows(&[
            ("source", seat.clone()),
            ("sonnet", shape(Some("sonnet"), proven("/a"))),
            ("b", shape(Some("opus"), proven("/b"))),
            ("dup", shape(Some("opus"), proven("/b"))),
            ("same", shape(Some("opus"), proven("/a"))),
            ("unknown", shape(Some("opus"), Store::Unknown)),
            ("pinless", shape(None, proven("/c"))),
            ("other", other),
        ]);
        assert_eq!(
            listed(&[], ("source", &seat), &all),
            rows(&[("b", twin), ("sonnet", sibling)])
        );
        assert_eq!(
            listed(&["dup".to_owned(), "x".to_owned()], ("source", &seat), &all),
            rows(&[
                ("dup", Provenance::Declared),
                ("x", Provenance::Declared),
                ("sonnet", sibling),
            ]),
            "declared first; a derived alias of a declared one adds nothing"
        );
        let pinless_seat = shape(None, proven("/a"));
        assert_eq!(
            listed(&[], ("source", &pinless_seat), &all),
            rows(&[("pinless", twin)]),
            "a pinless seat has pinless twins and no sibling"
        );
    }

    #[test]
    fn distinct_unknown_accounts_never_collapse_even_against_a_declared_one() {
        let seat = shape(Some("opus"), proven("/a"));
        let sibling = Provenance::Derived(Kin::Sibling);
        let unknown = rows(&[
            ("first", shape(Some("sonnet"), Store::Unknown)),
            ("second", shape(Some("sonnet"), Store::Unknown)),
        ]);
        assert_eq!(
            listed(&[], ("source", &seat), &unknown),
            rows(&[("first", sibling), ("second", sibling)])
        );
        assert_eq!(
            listed(&["first".to_owned()], ("source", &seat), &unknown),
            rows(&[("first", Provenance::Declared), ("second", sibling)])
        );
        // An unknown account is no proof of ANOTHER account either.
        let copy = rows(&[("copy", shape(Some("opus"), Store::Unknown))]);
        assert!(listed(&[], ("source", &seat), &copy).is_empty());
        let unknown_seat = shape(Some("opus"), Store::Unknown);
        let proven_b = rows(&[("b", shape(Some("opus"), proven("/b")))]);
        assert!(listed(&[], ("source", &unknown_seat), &proven_b).is_empty());
        // A tool with no account variable has one store: no twin, and the
        // same command collapses.
        let shared = shape(Some("opus"), Store::Shared);
        let commands = rows(&[
            ("same", shape(Some("opus"), Store::Shared)),
            ("one", shape(Some("sonnet"), Store::Shared)),
            ("two", shape(Some("sonnet"), Store::Shared)),
        ]);
        assert_eq!(
            listed(&[], ("source", &shared), &commands),
            rows(&[("one", sibling)])
        );
    }

    #[test]
    fn a_respelled_model_flag_on_a_shared_store_is_never_an_alias() {
        let sibling = Provenance::Derived(Kin::Sibling);
        let on = |pin: &str, raw: &str| Shape {
            raw: raw.to_owned(),
            ..shape(Some(pin), Store::Shared)
        };
        let seat = on("opus", "tool --model opus");
        let commands = rows(&[
            ("equals", on("sonnet", "tool --model=sonnet")),
            ("spaced", on("sonnet", "tool --model sonnet")),
            ("copy", on("sonnet", "tool --model=sonnet")),
        ]);
        assert_eq!(
            listed(&[], ("source", &seat), &commands),
            rows(&[("equals", sibling), ("spaced", sibling)]),
            "only the byte-identical copy adds nothing"
        );
    }

    fn candidate(profile: &str, windows: Vec<Window>) -> Candidate {
        Candidate {
            profile: profile.to_owned(),
            provenance: Provenance::Declared,
            configured: true,
            peer_latched: false,
            windows,
            left_at: None,
            left_on_headroom: false,
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
            (true, Unknown, false, Frame::Unproven),
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
            fate: super::Fate::Carried,
            nudge: Some(Nudge::Submitted),
            critical: false,
            dirty: false,
            cause: "",
        };
        assert_eq!(
            notice(SESSION, AGENT, &Ending::Moved(moved)),
            "auto reseat: builder moved sol6x -> opus55x (tool codex -> claude, model gpt-6-sol -> \
             opus), carried; work tree clean. If builder is half of a review pair, re-check its gate \
             provider. Next: nothing, its continuation nudge was submitted."
        );
        // A nudge ae cannot prove submitted never says nothing is left to do,
        // and never asks for a resend the pane does not call for.
        for (nudge, want) in [
            (
                Nudge::Unconfirmed,
                "Next: its continuation nudge is unconfirmed: peek builder; re-send a resume turn \
                 only if the nudge still sits in its input box.",
            ),
            (
                Nudge::Refused,
                "Next: its continuation nudge was refused or unrecorded: peek builder; send it one \
                 turn to resume only if no nudge reached its pane.",
            ),
        ] {
            let said = notice(
                SESSION,
                AGENT,
                &Ending::Moved(Move {
                    nudge: Some(nudge),
                    ..moved
                }),
            );
            assert!(said.ends_with(want), "{nudge:?}: {said}");
        }
        let seeded = Move {
            fate: super::Fate::Seeded,
            nudge: None,
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
                Ending::Held(
                    CauseClass::Ineligible,
                    "ineligible: held: auto reseat is off",
                ),
                "auto reseat: ineligible: builder not moved yet — held: auto reseat is off. Next: ae \
                 tries once more if it stays eligible.",
            ),
            (
                Ending::Refused(
                    CauseClass::NoCandidate,
                    "no-candidate: refused: no usable candidate: opus55x (exhausted)",
                ),
                "auto reseat: no-candidate: builder not moved — refused: no usable candidate: \
                 opus55x (exhausted). Next: move it by hand (ae reseat aedev builder --using \
                 <profile>) or wait for the reset.",
            ),
            (
                Ending::Failed(CauseClass::Other, "other: Error: the pane never came back"),
                "auto reseat: other: builder move failed — Error: the pane never came back. Next: \
                 relaunch builder if its pane sits at a shell, else reseat it by hand.",
            ),
        ] {
            let said = notice(SESSION, AGENT, &ending);
            assert_eq!(said, want);
            assert!(!said.contains("review pair"), "{said}");
        }
        let hostile = format!("Error: {}\u{1b}[2J\nsecond line", "x".repeat(NOTICE_CHARS));
        let said = notice(SESSION, AGENT, &Ending::Failed(CauseClass::Other, &hostile));
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

    #[test]
    fn the_headroom_threshold_is_off_or_a_whole_percent_and_anything_else_is_95_noted() {
        assert_eq!(parse_headroom(Ok(None), Ok(0)), (Some(95), None));
        assert_eq!(parse_headroom(Ok(Some("off".into())), Ok(1)), (None, None));
        for (raw, at) in [(" 96 ", 96), ("50", 50), ("100", 100), ("95", 95)] {
            assert_eq!(
                parse_headroom(Ok(Some(raw.into())), Ok(1)),
                (Some(at), None),
                "{raw:?}"
            );
        }
        for bad in ["49", "101", "+96", "95.5", "", "OFF", "\u{1b}[31m", "256"] {
            let (at, note) = parse_headroom(Ok(Some(bad.into())), Ok(1));
            assert_eq!(at, Some(95), "{bad:?}");
            let note = note.expect("a fallback is noted");
            assert!(note.contains("auto_reseat_at"), "{note}");
            assert!(note.ends_with("; 95 used"), "{note}");
            assert!(
                !note.contains('\u{1b}'),
                "a note echoes no control byte: {note:?}"
            );
        }
        let (at, note) = parse_headroom(Ok(Some("96".into())), Ok(2));
        assert_eq!(at, Some(95));
        assert_eq!(
            note.as_deref(),
            Some("auto_reseat_at is set 2 times; 95 used")
        );
        let (at, note) = parse_headroom(Err("unreadable".into()), Ok(1));
        assert_eq!(
            (at, note.as_deref()),
            (Some(95), Some("unreadable; 95 used"))
        );
        let (at, note) = parse_headroom(Ok(Some("96".into())), Err("unlisted".into()));
        assert_eq!((at, note.as_deref()), (Some(95), Some("unlisted; 95 used")));
    }

    #[test]
    fn the_threshold_is_read_only_behind_a_switch_that_is_on() {
        let file = Path::new("/nonexistent/ae-config");
        let off = settings_in(
            file,
            "[workspace]\nauto_reseat = off\nauto_reseat_at = bad\n",
        );
        assert_eq!((off.headroom_at, off.notes.len()), (None, 0));
        let on = settings_in(
            file,
            "[workspace]\nauto_reseat = on\nauto_reseat_at = bad\n",
        );
        assert_eq!(on.headroom_at, Some(95));
        assert_eq!(on.notes.len(), 1, "{:?}", on.notes);
        assert_eq!(on.headroom_note.as_ref(), on.notes.first());
        let twice = settings_in(
            file,
            "[workspace]\nauto_reseat = all\nauto_reseat_at = 60\nauto_reseat_at = 70\n",
        );
        assert_eq!(twice.headroom_at, Some(95));
        let absent = settings_in(file, "[workspace]\nauto_reseat = on\n");
        assert_eq!((absent.headroom_at, absent.headroom_note), (Some(95), None));
    }

    /// A headroom opener, `since` seconds after the key, naming its reading.
    fn crossed(since: i64) -> Event {
        record(
            since,
            WATCHDOG_ACTOR,
            HEADROOM_ACTION,
            AGENT,
            r#","summary":"headroom 95% weekly_all 7d""#,
        )
    }

    fn headroom(events: &[Event]) -> Option<Episode> {
        headroom_episode(events, SESSION, SLOT, AGENT)
    }

    #[test]
    fn a_headroom_episode_is_keyed_by_its_first_crossing_and_ends_on_relief_a_move_or_a_spawn() {
        let opened = headroom(&[crossed(0), crossed(30)]).expect("an open episode");
        assert_eq!(opened.key, after_key(0));
        assert_eq!(opened.trigger, Trigger::Headroom);
        assert_eq!(opened.cause(), "headroom 95% weekly_all 7d");
        assert_eq!(
            fold(&[limit()]).map(|found| (found.trigger, found.cause().to_owned())),
            Some((Trigger::Limit, String::new()))
        );
        // A limit opens no headroom episode, and only the watchdog opens one.
        assert_eq!(headroom(&[limit()]), None);
        let forged = record(0, "lead", HEADROOM_ACTION, AGENT, "");
        assert_eq!(headroom(&[forged]), None);
        // Relief for ANOTHER key re-arms nothing; its own key does.
        let other = watchdog(10, REARMED_ACTION, Some(OTHER_KEY));
        assert!(headroom(&[crossed(0), other]).is_some());
        let own = watchdog(10, REARMED_ACTION, Some(KEY));
        assert_eq!(headroom(&[crossed(0), own.clone()]), None);
        let reopened = headroom(&[crossed(0), own, crossed(20)]).expect("a new episode");
        assert_eq!(reopened.key, after_key(20));
        // A move ends it whatever its ref names; so does the seat's spawn.
        let done = routed(10, DONE_ACTION, "fake-claude");
        assert_eq!(headroom(&[crossed(0), done]), None);
        assert_eq!(headroom(&[crossed(0), spawned(10)]), None);
        // A terminal outcome keeps the episode until relief: one refusal.
        let refused = routed(10, REFUSED_ACTION, KEY);
        let rests = headroom(&[crossed(0), routed(5, ATTEMPT_ACTION, KEY), refused])
            .expect("a terminal episode stays");
        assert_eq!(rests.terminal, Some(Outcome::Refused));
        assert_eq!(rests.attempts, 1);
    }

    #[test]
    fn a_hold_is_named_once_by_its_whole_summary() {
        let held = record(
            10,
            WATCHDOG_ACTOR,
            HELD_ACTION,
            AGENT,
            &format!(r#","ref":"{KEY}","summary":"held: busy""#),
        );
        let found = headroom(&[crossed(0), held]).expect("an open episode");
        assert!(found.holds_with("held: busy"));
        assert!(!found.holds_with("held: busy on headroom"));
        let moved_on = headroom(&[
            crossed(0),
            routed(5, HELD_ACTION, KEY),
            routed(9, ATTEMPT_ACTION, KEY),
        ])
        .expect("an open episode");
        assert!(!moved_on.held);
        assert!(!moved_on.holds_with(""));
    }

    #[test]
    fn only_a_move_that_closed_a_headroom_episode_leaves_its_profile_on_headroom() {
        let late = after_key(30).to_string();
        let events = [
            limit(),
            routed(10, ATTEMPT_ACTION, KEY),
            routed(20, DONE_ACTION, "sol6x"),
            crossed(30),
            routed(40, ATTEMPT_ACTION, &late),
            routed(50, DONE_ACTION, "opus55x"),
        ];
        assert_eq!(left_on_headroom(&events, SESSION, SLOT, AGENT), ["opus55x"]);
        let all: Vec<String> = left_profiles(&events, SESSION, SLOT, AGENT)
            .into_iter()
            .map(|(profile, _)| profile)
            .collect();
        assert_eq!(all, ["sol6x", "opus55x"]);
        // Leaving the same profile again on a limit makes it a limit leave.
        let mut again = events.to_vec();
        again.extend([
            watchdog(60, LIMIT_ACTION, None),
            routed(70, ATTEMPT_ACTION, &after_key(60).to_string()),
            routed(80, DONE_ACTION, "opus55x"),
        ]);
        assert!(left_on_headroom(&again, SESSION, SLOT, AGENT).is_empty());
        // A spawn forgets every leave.
        again.push(spawned(90));
        assert!(left_profiles(&again, SESSION, SLOT, AGENT).is_empty());
    }

    #[test]
    fn a_headroom_move_holds_an_unproven_frame_where_a_limit_move_takes_it() {
        let events = [crossed(0)];
        let found = headroom(&events);
        let decide_on = |pane: &Pane| decide(found.as_ref(), 600, pane, key_epoch() + 650);
        let unproven = pane(Frame::Unproven, false, None);
        assert_eq!(decide_on(&unproven), Decision::Hold(HoldReason::Unproven));
        assert_eq!(at(&[limit()], &unproven, 650), Decision::Attempt);
        assert_eq!(decide_on(&CLEAR), Decision::Attempt);
        // The prompt still outranks it; it outranks the client's input.
        let prompted = pane(Frame::Unproven, true, Some(100));
        assert_eq!(
            decide_on(&prompted),
            Decision::Hold(HoldReason::HumanPrompt)
        );
        let touched = pane(Frame::Unproven, false, Some(100));
        assert_eq!(decide_on(&touched), Decision::Hold(HoldReason::Unproven));
        assert_eq!(
            at(&[limit()], &touched, 650),
            Decision::Hold(HoldReason::ClientInput)
        );
        let summary = HoldReason::Unproven.summary();
        assert!(summary.starts_with("held: "), "{summary}");
        assert_ne!(summary, HoldReason::Unread.summary());
        // Under the grace a headroom episode waits like a limit one.
        let early = decide(found.as_ref(), 600, &CLEAR, key_epoch() + 599);
        assert_eq!(
            early,
            Decision::Wait {
                due_at: key_epoch() + 600
            }
        );
    }

    #[test]
    fn the_worst_usable_window_decides_room_with_a_five_point_gap_below_the_threshold() {
        let fresh = |judged| window(judged, false, 1, Status::Fresh);
        assert_eq!(room(&[], 95), Room::Unread);
        assert_eq!(
            room(&[window(99.0, true, 1, Status::Unknown)], 95),
            Room::Unread
        );
        assert_eq!(room(&[fresh(95.0)], 95), Room::Crossed(fresh(95.0)));
        assert_eq!(room(&[fresh(94.9)], 95), Room::Near);
        assert_eq!(room(&[fresh(90.0)], 95), Room::Near);
        assert_eq!(room(&[fresh(89.9)], 95), Room::Relieved(fresh(89.9)));
        assert_eq!(room(&[fresh(99.0)], 100), Room::Near);
        assert_eq!(room(&[fresh(100.0)], 100), Room::Crossed(fresh(100.0)));
        // A stale number still binds; one ae cannot use is no reading.
        let stale = window(97.0, true, 1, Status::Stale);
        assert_eq!(
            room(&[fresh(10.0), stale.clone()], 95),
            Room::Crossed(stale)
        );
        assert_eq!(
            room(&[fresh(10.0), window(99.0, true, 1, Status::Unknown)], 95),
            Room::Relieved(fresh(10.0))
        );
        assert_eq!(headroom_words(&fresh(95.0)), "headroom 95% weekly_all 7d");
        assert_eq!(
            headroom_words(&fresh(95.06)),
            "headroom 95.1% weekly_all 7d"
        );
        assert_eq!(
            relieved_words(&fresh(89.9)),
            "headroom cleared: 89.9% weekly_all 7d"
        );
    }

    fn short(judged: f64, critical: bool) -> Window {
        Window {
            long: false,
            label: "session 5h".to_owned(),
            ..window(judged, critical, 6_000, Status::Fresh)
        }
    }

    #[test]
    fn only_a_window_proven_a_day_long_decides_room() {
        let weekly = window(50.0, false, 1, Status::Fresh);
        assert_eq!(room(&[short(99.0, true)], 95), Room::Unread);
        assert_eq!(
            room(&[short(99.0, true), weekly.clone()], 95),
            Room::Relieved(weekly)
        );
        assert_eq!(
            room(
                &[short(40.0, false), window(94.0, false, 1, Status::Fresh)],
                95
            ),
            Room::Near
        );
    }

    #[test]
    fn a_short_window_blocks_a_headroom_target_only_at_critical() {
        let choose = |windows| choose_with(&[candidate("far", windows)], NOW, Some(80));
        assert!(choose(vec![short(85.0, false)]).pick.is_some());
        let spent = choose(vec![short(96.0, true)]);
        assert_eq!(spent.pick, None);
        assert_eq!(spent.skipped, [("far".to_owned(), Skip::NoRoom)]);
        // Below the threshold a critical short window is only deprioritized.
        let below = choose_with(&[candidate("far", vec![short(96.0, true)])], NOW, Some(98));
        assert_eq!(below.pick, Some(("far".to_owned(), Tier::Critical)));
        let long = choose(vec![window(85.0, false, 6_000, Status::Fresh)]);
        assert_eq!(long.skipped, [("far".to_owned(), Skip::NoRoom)]);
    }
    #[test]
    fn a_headroom_episode_passes_over_a_candidate_without_room_where_a_limit_takes_it() {
        let near = candidate("near", vec![window(96.0, true, 6_000, Status::Fresh)]);
        let headroom = choose_with(std::slice::from_ref(&near), NOW, Some(95));
        assert_eq!(headroom.pick, None);
        assert_eq!(headroom.skipped, [("near".to_owned(), Skip::NoRoom)]);
        assert!(
            pick(std::slice::from_ref(&near)).is_some(),
            "a limit move takes it"
        );
        let stale = candidate("stale", vec![window(95.0, true, 6_000, Status::Stale)]);
        let unknown = candidate("unknown", vec![window(99.0, true, 6_000, Status::Unknown)]);
        let roomy = candidate("roomy", vec![window(94.9, false, 6_000, Status::Fresh)]);
        let choice = choose_with(&[stale, unknown, roomy], NOW, Some(95));
        assert_eq!(choice.skipped, [("stale".to_owned(), Skip::NoRoom)]);
        assert_eq!(choice.pick, Some(("roomy".to_owned(), Tier::BelowCritical)));
        // Exhausted outranks the lack of room.
        let spent = candidate("spent", vec![window(100.0, true, 6_000, Status::Fresh)]);
        let choice = choose_with(&[spent], NOW, Some(95));
        assert_eq!(choice.skipped, [("spent".to_owned(), Skip::Exhausted)]);
    }

    #[test]
    fn a_profile_left_on_headroom_waits_for_a_later_reading_below_the_threshold() {
        let left = |windows| Candidate {
            left_at: Some(5_000),
            left_on_headroom: true,
            ..candidate("opus55x", windows)
        };
        let skipped = |windows, room| choose_with(&[left(windows)], NOW, room).skipped;
        let still = vec![("opus55x".to_owned(), Skip::LeftOnHeadroom)];
        assert_eq!(skipped(Vec::new(), Some(95)), still);
        assert_eq!(
            skipped(vec![window(10.0, false, 4_000, Status::Fresh)], Some(95)),
            still,
            "a reading from before the move proves nothing"
        );
        assert_eq!(
            skipped(vec![window(95.0, true, 6_000, Status::Fresh)], Some(95)),
            still
        );
        // The bar is the threshold, not critical: 99 relieves at 100.
        let critical = vec![window(99.0, true, 6_000, Status::Fresh)];
        assert_eq!(
            choose_with(&[left(critical.clone())], NOW, Some(100)).pick,
            Some(("opus55x".to_owned(), Tier::Critical))
        );
        // A limit episode judges the same leave by the critical bar.
        assert_eq!(skipped(critical, None), still);
        let low = vec![window(20.0, false, 6_000, Status::Fresh)];
        assert!(choose_with(&[left(low)], NOW, None).pick.is_some());
        // A passed reset relieves it as it relieves a limit leave.
        let reset = Window {
            resets_at: Some(NOW - 1),
            ..window(100.0, true, 6_000, Status::Unknown)
        };
        assert!(
            choose_with(&[left(vec![reset])], NOW, Some(95))
                .pick
                .is_some()
        );
    }

    #[test]
    fn a_headroom_attempt_is_in_flight_only_inside_its_bound_and_only_while_on() {
        let events = [crossed(0), routed(5, ATTEMPT_ACTION, KEY)];
        let on = with_map(Switch::On);
        let seat = (SESSION, SLOT, AGENT);
        assert!(headroom_in_flight(&on, &events, seat, key_epoch() + 6));
        let bound = key_epoch() + 5 + IN_FLIGHT_SECS;
        assert!(headroom_in_flight(&on, &events, seat, bound - 1));
        assert!(!headroom_in_flight(&on, &events, seat, bound));
        assert!(!headroom_in_flight(
            &with_map(Switch::Off),
            &events,
            seat,
            key_epoch() + 6
        ));
        assert!(!headroom_in_flight(
            &on,
            &[crossed(0)],
            seat,
            key_epoch() + 6
        ));
        // A limit attempt is not a headroom move.
        assert!(!headroom_in_flight(
            &on,
            &[limit(), routed(5, ATTEMPT_ACTION, KEY)],
            seat,
            key_epoch() + 6
        ));
    }

    #[test]
    fn an_outcome_under_either_live_key_closes_every_open_episode_and_a_stale_one_none() {
        let late = after_key(30).to_string();
        let both = [limit(), crossed(30)];
        let terminal = |found: Option<Episode>| found.expect("still the seat's").terminal;
        for (action, reference, outcome) in [
            (REFUSED_ACTION, KEY, Some(Outcome::Refused)),
            (REFUSED_ACTION, late.as_str(), Some(Outcome::Refused)),
            (FAILED_ACTION, KEY, Some(Outcome::Failed)),
            (FAILED_ACTION, late.as_str(), Some(Outcome::Failed)),
            (REFUSED_ACTION, OTHER_KEY, None),
            (FAILED_ACTION, OTHER_KEY, None),
        ] {
            let mut events = both.to_vec();
            events.push(routed(40, action, reference));
            let (limit, headroom) = episodes(&events, SESSION, SLOT, AGENT);
            assert_eq!(terminal(limit), outcome, "{action} {reference}");
            assert_eq!(terminal(headroom), outcome, "{action} {reference}");
        }
        // A move closes both, whatever its ref names.
        let mut moved = both.to_vec();
        moved.push(routed(40, DONE_ACTION, "fake-claude"));
        let (limit, headroom) = episodes(&moved, SESSION, SLOT, AGENT);
        assert_eq!(terminal(limit), Some(Outcome::Done));
        assert_eq!(headroom, None);
        // Closed, each waits for its own arming: a fresh latch, a strict relief.
        let mut refused = both.to_vec();
        refused.extend([
            routed(40, REFUSED_ACTION, KEY),
            watchdog(50, CLEARED_ACTION, None),
            watchdog(60, LIMIT_ACTION, None),
        ]);
        let (limit, headroom) = episodes(&refused, SESSION, SLOT, AGENT);
        assert_eq!(
            limit.map(|found| (found.key, found.terminal)),
            Some((after_key(60), None))
        );
        assert_eq!(terminal(headroom), Some(Outcome::Refused));
        // The closed key is stale now: it closes nothing of the fresh episode.
        refused.push(routed(70, FAILED_ACTION, KEY));
        assert_eq!(terminal(episode(&refused, SESSION, SLOT, AGENT)), None);
        refused.push(watchdog(80, REARMED_ACTION, Some(&late)));
        assert_eq!(headroom_episode(&refused, SESSION, SLOT, AGENT), None);
    }

    #[test]
    fn an_attempt_in_flight_of_either_kind_decides_and_an_equal_key_takes_the_flying_kind() {
        let seat = (SESSION, SLOT, AGENT);
        // A headroom attempt in flight, and a limit latched in the same second
        // after it: both episodes are keyed alike, only one has the attempt.
        let events = [crossed(0), routed(0, ATTEMPT_ACTION, KEY), limit()];
        let (limit_now, headroom_now) = episodes(&events, SESSION, SLOT, AGENT);
        assert_eq!(limit_now.as_ref().map(|found| found.open), Some(None));
        let moving = flying(limit_now.as_ref(), headroom_now.as_ref()).expect("one flies");
        assert_eq!(moving.trigger, Trigger::Headroom);
        let found = keyed(&events, seat, after_key(0)).expect("keyed");
        assert_eq!(found.trigger, Trigger::Headroom);
        // Nothing in flight: the limit episode under the key, then headroom.
        let still = keyed(&[limit(), crossed(0)], seat, after_key(0)).expect("keyed");
        assert_eq!(still.trigger, Trigger::Limit);
        let only = keyed(&[crossed(0)], seat, after_key(0)).expect("keyed");
        assert_eq!(only.trigger, Trigger::Headroom);
        assert_eq!(keyed(&[crossed(0)], seat, after_key(1)), None);
        // An ended attempt flies no more.
        let ended = [
            crossed(0),
            routed(5, ATTEMPT_ACTION, KEY),
            routed(9, FAILED_ACTION, KEY),
        ];
        let (limit_now, headroom_now) = episodes(&ended, SESSION, SLOT, AGENT);
        assert_eq!(flying(limit_now.as_ref(), headroom_now.as_ref()), None);
        assert_eq!(flying(None, None), None);
    }

    /// The kind of a move is the kind of the attempt it closed, judged when
    /// that attempt was journaled: an opener under the same second afterwards
    /// changes nothing.
    #[test]
    fn a_leave_is_the_kind_of_its_attempt_whatever_opens_after_it() {
        let limit_first = [
            limit(),
            routed(0, ATTEMPT_ACTION, KEY),
            crossed(0),
            routed(5, DONE_ACTION, "sol6x"),
        ];
        assert!(left_on_headroom(&limit_first, SESSION, SLOT, AGENT).is_empty());
        let headroom_first = [
            crossed(0),
            routed(0, ATTEMPT_ACTION, KEY),
            limit(),
            routed(5, DONE_ACTION, "sol6x"),
        ];
        assert_eq!(
            left_on_headroom(&headroom_first, SESSION, SLOT, AGENT),
            ["sol6x"]
        );
        // Both open before the attempt: the limit takes it, as a leg would.
        let both = [
            limit(),
            crossed(0),
            routed(0, ATTEMPT_ACTION, KEY),
            routed(5, DONE_ACTION, "sol6x"),
        ];
        assert!(left_on_headroom(&both, SESSION, SLOT, AGENT).is_empty());
        // A done with no attempt before it is a limit leave, and a done ends
        // the attempt it closed.
        let bare = [crossed(0), routed(5, DONE_ACTION, "sol6x")];
        assert!(left_on_headroom(&bare, SESSION, SLOT, AGENT).is_empty());
        let twice = [
            crossed(0),
            routed(1, ATTEMPT_ACTION, KEY),
            routed(5, DONE_ACTION, "sol6x"),
            routed(20, DONE_ACTION, "opus55x"),
        ];
        assert_eq!(left_on_headroom(&twice, SESSION, SLOT, AGENT), ["sol6x"]);
    }
    /// Every outcome a seat can be held, refused or failed with leads with a
    /// short class word, each class is its own word, and only the two that may
    /// pass on a retry are transient.
    #[test]
    fn every_outcome_record_leads_with_its_cause_class() {
        let classes = [
            (
                CauseClass::CarrySourceMissing,
                "carry-source-missing",
                false,
            ),
            (
                CauseClass::CarryTargetDiffers,
                "carry-target-differs",
                false,
            ),
            (CauseClass::CarryUnreadable, "carry-unreadable", false),
            (CauseClass::Busy, "busy", true),
            (CauseClass::UnknownFrame, "unknown-frame", false),
            (CauseClass::Lock, "lock", true),
            (CauseClass::NoCandidate, "no-candidate", false),
            (CauseClass::Ineligible, "ineligible", false),
            (CauseClass::Other, "other", false),
        ];
        for (index, (class, word, transient)) in classes.iter().enumerate() {
            assert_eq!(class.word(), *word);
            assert_eq!(class.transient(), *transient, "{word}");
            assert_eq!(class.lead("the message"), format!("{word}: the message"));
            assert!(
                !classes[..index].iter().any(|(_, seen, _)| seen == word),
                "{word} is its own word"
            );
        }
        // A hold the daemon names: the class is chosen by the reason, and the
        // record is that class then the reason's own words.
        for (reason, class) in [
            (HoldReason::Unread, CauseClass::UnknownFrame),
            (HoldReason::Unproven, CauseClass::UnknownFrame),
            (HoldReason::Busy, CauseClass::Busy),
            (HoldReason::Draft, CauseClass::Busy),
            (HoldReason::HumanPrompt, CauseClass::Busy),
            (HoldReason::ClientInput, CauseClass::Busy),
        ] {
            assert_eq!(reason.class(), class, "{reason:?}");
            assert_eq!(
                reason.record(),
                format!("{}: {}", class.word(), reason.summary()),
                "{reason:?}"
            );
        }
        // A seat the path will not move: nothing to move to is its own class.
        for (why, class) in [
            (Ineligible::Unmapped, CauseClass::NoCandidate),
            (Ineligible::Off, CauseClass::Ineligible),
            (Ineligible::Orchestrator, CauseClass::Ineligible),
            (Ineligible::Session, CauseClass::Ineligible),
            (Ineligible::Slot, CauseClass::Ineligible),
            (Ineligible::Class, CauseClass::Ineligible),
        ] {
            assert_eq!(why.class(), class, "{why:?}");
            assert!(
                leg::ineligible_record(why).starts_with(&format!("{}: held: ", class.word())),
                "{why:?}"
            );
        }
        // The notice for an unmapped seat leads with the class, ahead of the agent.
        let told = notice(SESSION, AGENT, &Ending::Unmapped("sol6x"));
        assert!(
            told.starts_with(
                "auto reseat: no-candidate: builder not moved — no declared or derived candidate \
                 for sol6x."
            ),
            "{told}"
        );
    }
    /// The boundary the notice cut must hold: a session and agent at their
    /// longest and a profile at 64 characters leave a notice with almost no
    /// room beside the whole safe next step, and the class still leads it, the
    /// step stays whole, and the notice stays inside its bound.
    #[test]
    fn a_notice_at_the_longest_names_keeps_its_class_and_the_whole_next_step() {
        let (session, agent, profile) = ("s".repeat(128), "a".repeat(64), "p".repeat(64));
        assert!(crate::config::is_agent_name(&agent));
        assert!(crate::session_launch::name::is_session_name(&session));
        let command = format!("ae reseat {session} {agent} --using {profile} --stop-unknown");
        let said = CauseClass::UnknownFrame.lead(&format!(
            "Error: once its box is empty and the limit line is its newest row, run: {command} \
             — ae cannot read the frame in pane %1, so it cannot tell a running turn from an \
             idle box. Nothing was reseated."
        ));
        let told = notice(
            &session,
            &agent,
            &Ending::Refused(CauseClass::UnknownFrame, &said),
        );
        assert!(told.starts_with("auto reseat: unknown-frame: "), "{told}");
        assert!(told.contains(&command), "the whole step survives: {told}");
        assert!(
            told.chars().count() <= NOTICE_CHARS,
            "{}",
            told.chars().count()
        );
        // Every other class, with a message long enough to be cut.
        let long = format!("Error: {}", "x".repeat(900));
        for class in [
            CauseClass::CarrySourceMissing,
            CauseClass::CarryTargetDiffers,
            CauseClass::CarryUnreadable,
            CauseClass::Busy,
            CauseClass::Lock,
            CauseClass::NoCandidate,
            CauseClass::Ineligible,
            CauseClass::Other,
        ] {
            let said = class.lead(&long);
            for ending in [
                Ending::Held(class, &said),
                Ending::Refused(class, &said),
                Ending::Failed(class, &said),
            ] {
                let told = notice(&session, &agent, &ending);
                assert!(
                    told.starts_with(&format!("auto reseat: {}: ", class.word())),
                    "{told}"
                );
                assert!(told.contains(" Next: "), "the step survives: {told}");
                assert!(told.chars().count() <= NOTICE_CHARS, "{told}");
            }
        }
    }

    /// A profile name has no length bound, so the step that quotes one can fail
    /// to fit beside the class at all. It is then replaced whole by a fixed
    /// step, and a notice never prints half a command.
    #[test]
    fn a_step_too_long_for_the_notice_is_replaced_whole_and_never_clipped() {
        let (session, agent) = ("s".repeat(128), "a".repeat(64));
        for len in [1, 64, 65, 80, 400, 5000] {
            let profile = "p".repeat(len);
            let command = format!("ae reseat {session} {agent} --using {profile} --stop-unknown");
            let said = CauseClass::UnknownFrame.lead(&format!(
                "Error: once its box is empty and the limit line is its newest row, run: \
                 {command} — ae cannot read the frame in pane %1. Nothing was reseated."
            ));
            let told = notice(
                &session,
                &agent,
                &Ending::Refused(CauseClass::UnknownFrame, &said),
            );
            assert!(
                told.starts_with("auto reseat: unknown-frame: "),
                "{len}: {told}"
            );
            assert!(told.chars().count() <= NOTICE_CHARS, "{len}: {told}");
            assert!(told.ends_with('.'), "{len}: {told}");
            // 28 + 7 + 65 + 10 + 193 + 9 + len + 15 + 1 is the whole notice: the
            // step fits through a 65-character profile and no further.
            assert_eq!(told.contains(&command), len <= 65, "{len}: {told}");
            if !told.contains(&command) {
                // No piece of the command: neither its precondition nor the
                // start of its profile.
                assert!(!told.contains("--using p"), "{len}: {told}");
                assert!(!told.contains("--stop"), "{len}: {told}");
                assert!(!told.contains("once its box is empty"), "{len}: {told}");
                assert!(
                    told.ends_with(&format!(
                        "Next: move it by hand (ae reseat {session} {agent} --using <profile>) \
                         or wait for the reset."
                    )),
                    "{len}: {told}"
                );
            }
        }
        // The same for a profile with no candidate.
        for len in [1, 200, 5000] {
            let profile = "p".repeat(len);
            let told = notice(&session, &agent, &Ending::Unmapped(&profile));
            assert!(
                told.starts_with("auto reseat: no-candidate: "),
                "{len}: {told}"
            );
            assert!(told.chars().count() <= NOTICE_CHARS, "{len}: {told}");
            assert!(told.ends_with("--using <profile>)."), "{len}: {told}");
            assert!(
                told.contains(" Next: add an [auto_reseat] row for "),
                "{len}: {told}"
            );
            if len >= 200 {
                assert!(
                    told.contains(" Next: add an [auto_reseat] row for the profile, or move it"),
                    "{len}: {told}"
                );
            }
        }
    }

    /// A time far past every cooldown.
    const FAR: i64 = 10_000_000;

    /// The journal of a limit episode whose moves were each closed by the next
    /// action in `closings`, an attempt before each, the next attempt one
    /// cooldown after the last closing; and when the last one closed.
    fn rounds_of(closings: &[&str]) -> (Vec<Event>, i64) {
        let mut events = vec![limit()];
        let (mut attempt, mut closed) = (600, 0);
        for action in closings {
            closed = attempt + 30;
            events.push(routed(attempt, ATTEMPT_ACTION, KEY));
            events.push(routed(closed, action, KEY));
            attempt = closed + RETRY_COOLDOWN_SECS;
        }
        (events, closed)
    }

    #[test]
    fn a_refused_or_failed_limit_move_is_due_again_once_its_cooldown_has_passed() {
        for action in [REFUSED_ACTION, FAILED_ACTION] {
            let (events, closed) = rounds_of(&[action]);
            let due = closed + RETRY_COOLDOWN_SECS;
            assert_eq!(at(&events, &CLEAR, due - 1), Decision::Rest, "{action}");
            assert_eq!(at(&events, &CLEAR, due), Decision::Attempt, "{action}");
            assert_eq!(at(&events, &CLEAR, FAR), Decision::Attempt, "{action}");
        }
        // The cooldown counts from the closing, not from the key; and the
        // refusal the trigger writes when nothing is usable has no attempt
        // before it.
        let bare = [limit(), routed(700, REFUSED_ACTION, KEY)];
        let due = 700 + RETRY_COOLDOWN_SECS;
        assert_eq!(at(&bare, &CLEAR, due - 1), Decision::Rest);
        assert_eq!(at(&bare, &CLEAR, due), Decision::Attempt);
    }

    #[test]
    fn a_due_retry_answers_to_the_pane_and_the_grace_like_the_first_attempt() {
        let (events, closed) = rounds_of(&[REFUSED_ACTION]);
        let due = closed + RETRY_COOLDOWN_SECS;
        for (pane, reason) in [
            (pane(Frame::Unread, false, None), HoldReason::Unread),
            (pane(Frame::Busy, false, None), HoldReason::Busy),
            (pane(Frame::Draft, false, None), HoldReason::Draft),
            (pane(Frame::Clear, true, None), HoldReason::HumanPrompt),
            (
                pane(Frame::Clear, false, Some(due - 100)),
                HoldReason::ClientInput,
            ),
        ] {
            assert_eq!(at(&events, &pane, due), Decision::Hold(reason), "{pane:?}");
        }
        // A limit move reads a frame ae cannot prove idle as clear, as ever.
        let unproven = pane(Frame::Unproven, false, None);
        assert_eq!(at(&events, &unproven, due), Decision::Attempt);
    }

    #[test]
    fn each_round_has_its_own_transient_hold_allowance() {
        let (due, retried) = (
            730 + RETRY_COOLDOWN_SECS,
            routed(730 + RETRY_COOLDOWN_SECS, ATTEMPT_ACTION, KEY),
        );
        // A first round that used its second attempt before it was refused.
        let mut events = vec![
            limit(),
            routed(600, ATTEMPT_ACTION, KEY),
            routed(630, HELD_ACTION, KEY),
            routed(700, ATTEMPT_ACTION, KEY),
            routed(730, REFUSED_ACTION, KEY),
        ];
        assert_eq!(at(&events, &CLEAR, due), Decision::Attempt);
        events.extend([retried, routed(due + 30, HELD_ACTION, KEY)]);
        assert_eq!(at(&events, &CLEAR, due + 60), Decision::Attempt);
        events.extend([
            routed(due + 70, ATTEMPT_ACTION, KEY),
            routed(due + 100, HELD_ACTION, KEY),
        ]);
        assert_eq!(at(&events, &CLEAR, due + 200), Decision::Rest);
    }

    #[test]
    fn a_whole_key_makes_at_most_two_attempts_in_each_of_three_closings() {
        // Every round uses its transient allowance: attempt, hold, attempt, then
        // the closing; the next round starts one cooldown after it.
        let mut events = vec![limit()];
        let (mut start, mut closed) = (600, 0);
        for round in 0..MAX_ROUNDS {
            events.push(routed(start, ATTEMPT_ACTION, KEY));
            events.push(routed(start + 30, HELD_ACTION, KEY));
            assert_eq!(
                at(&events, &CLEAR, start + 60),
                Decision::Attempt,
                "{round}"
            );
            events.push(routed(start + 60, ATTEMPT_ACTION, KEY));
            // The second attempt of a round, the sixth of the key in the last,
            // is judged by the same bound as any open attempt.
            let bound = start + 60 + IN_FLIGHT_SECS;
            assert_eq!(
                at(&events, &CLEAR, bound - 1),
                Decision::InFlight,
                "{round}"
            );
            assert_eq!(at(&events, &CLEAR, bound), Decision::Overdue, "{round}");
            closed = start + 90;
            let action = if round % 2 == 0 {
                REFUSED_ACTION
            } else {
                FAILED_ACTION
            };
            events.push(routed(closed, action, KEY));
            if round + 1 < MAX_ROUNDS {
                let due = closed + RETRY_COOLDOWN_SECS;
                assert_eq!(at(&events, &CLEAR, due - 1), Decision::Rest, "{round}");
                assert_eq!(at(&events, &CLEAR, due), Decision::Attempt, "{round}");
            }
            start = closed + RETRY_COOLDOWN_SECS;
        }
        let attempts = events
            .iter()
            .filter(|event| event.action == ATTEMPT_ACTION)
            .count();
        assert_eq!(attempts, 2 * usize::from(MAX_ROUNDS), "six at most");
        let found = fold(&events).expect("an episode");
        assert_eq!((found.rounds, found.gave_up()), (MAX_ROUNDS, true));
        // No seventh attempt, however long the seat stays latched.
        for since in [
            closed + 1,
            closed + RETRY_COOLDOWN_SECS,
            closed + 7 * RETRY_COOLDOWN_SECS,
            FAR,
        ] {
            assert_eq!(at(&events, &CLEAR, since), Decision::Rest, "{since}");
        }
    }

    #[test]
    fn refusals_with_no_attempt_alone_reach_the_cap() {
        // The trigger refuses with nothing usable and journals no attempt.
        let step = RETRY_COOLDOWN_SECS;
        let mut events = vec![limit(), routed(700, REFUSED_ACTION, KEY)];
        assert_eq!(fold(&events).map(|found| found.rounds), Some(1));
        // A second one inside the cooldown is the same round.
        events.push(routed(710, REFUSED_ACTION, KEY));
        assert_eq!(fold(&events).map(|found| found.rounds), Some(1));
        assert_eq!(at(&events, &CLEAR, 700 + step), Decision::Attempt);
        events.push(routed(700 + step, REFUSED_ACTION, KEY));
        assert_eq!(fold(&events).map(|found| found.rounds), Some(2));
        assert_eq!(at(&events, &CLEAR, 700 + 2 * step), Decision::Attempt);
        events.push(routed(700 + 2 * step, REFUSED_ACTION, KEY));
        let found = fold(&events).expect("an episode");
        assert_eq!((found.rounds, found.gave_up()), (MAX_ROUNDS, true));
        assert_eq!(at(&events, &CLEAR, FAR), Decision::Rest);
    }

    #[test]
    fn the_last_round_ends_the_path_for_good_and_no_later_time_revives_it() {
        for (action, outcome) in [
            (REFUSED_ACTION, Outcome::Refused),
            (FAILED_ACTION, Outcome::Failed),
        ] {
            for rounds in 1..MAX_ROUNDS {
                let (events, closed) = rounds_of(&vec![action; usize::from(rounds)]);
                let found = fold(&events).expect("an episode");
                assert_eq!((found.rounds, found.gave_up()), (rounds, false), "{action}");
                let due = closed + RETRY_COOLDOWN_SECS;
                assert_eq!(
                    at(&events, &CLEAR, due),
                    Decision::Attempt,
                    "{action} {rounds}"
                );
            }
            let (events, closed) = rounds_of(&vec![action; usize::from(MAX_ROUNDS)]);
            let found = fold(&events).expect("an episode");
            assert_eq!(
                (found.rounds, found.gave_up(), found.terminal),
                (MAX_ROUNDS, true, Some(outcome)),
                "{action}"
            );
            for since in [closed + 1, closed + RETRY_COOLDOWN_SECS, FAR] {
                assert_eq!(
                    at(&events, &CLEAR, since),
                    Decision::Rest,
                    "{action} {since}"
                );
            }
        }
        // Refused and failed rounds are counted together.
        let (mixed, _) = rounds_of(&[REFUSED_ACTION, FAILED_ACTION, REFUSED_ACTION]);
        let found = fold(&mixed).expect("an episode");
        assert_eq!((found.rounds, found.gave_up()), (MAX_ROUNDS, true));
        // The trigger's attempt-less refusal can be the last round.
        let (mut bare, closed) = rounds_of(&vec![REFUSED_ACTION; usize::from(MAX_ROUNDS) - 1]);
        bare.push(routed(closed + RETRY_COOLDOWN_SECS, REFUSED_ACTION, KEY));
        let found = fold(&bare).expect("an episode");
        assert_eq!((found.rounds, found.gave_up()), (MAX_ROUNDS, true));
        assert_eq!(at(&bare, &CLEAR, FAR), Decision::Rest);
        // Holds and a re-booked limit are no rounds.
        let (mut events, _) = rounds_of(&[REFUSED_ACTION]);
        events.extend([
            watchdog(900, LIMIT_ACTION, None),
            routed(950, HELD_ACTION, KEY),
        ]);
        assert_eq!(fold(&events).map(|found| found.rounds), Some(1));
    }

    #[test]
    fn a_second_closing_inside_the_cooldown_belongs_to_the_same_round() {
        // The daemon books an attempt overdue; the slow leg then reports its
        // own refusal. One round, closed by the first.
        let events = [
            limit(),
            routed(600, ATTEMPT_ACTION, KEY),
            routed(780, FAILED_ACTION, KEY),
            routed(800, REFUSED_ACTION, KEY),
        ];
        let found = fold(&events).expect("an episode");
        assert_eq!((found.rounds, found.terminal), (1, Some(Outcome::Failed)));
        assert_eq!(
            at(&events, &CLEAR, 780 + RETRY_COOLDOWN_SECS - 1),
            Decision::Rest
        );
        assert_eq!(
            at(&events, &CLEAR, 780 + RETRY_COOLDOWN_SECS),
            Decision::Attempt
        );
    }

    #[test]
    fn a_done_never_rearms_whatever_came_before_it() {
        let moved = [
            limit(),
            routed(600, ATTEMPT_ACTION, KEY),
            routed(700, DONE_ACTION, "sol6x"),
        ];
        assert_eq!(at(&moved, &CLEAR, FAR), Decision::Rest);
        // A done after a refused or failed closing (the overdue-then-slow-leg
        // race) ends the path too: the seat moved.
        for action in [REFUSED_ACTION, FAILED_ACTION] {
            let mut late = rounds_of(&[action]).0;
            late.push(routed(700, DONE_ACTION, "sol6x"));
            let found = fold(&late).expect("an episode");
            assert_eq!(found.terminal, Some(Outcome::Done), "{action}");
            assert_eq!(at(&late, &CLEAR, FAR), Decision::Rest, "{action}");
        }
        // A done in a retry round.
        let (mut retried, closed) = rounds_of(&[REFUSED_ACTION]);
        let due = closed + RETRY_COOLDOWN_SECS;
        retried.extend([
            routed(due, ATTEMPT_ACTION, KEY),
            routed(due + 40, DONE_ACTION, "opus55x"),
        ]);
        let found = fold(&retried).expect("an episode");
        assert_eq!(found.terminal, Some(Outcome::Done));
        assert_eq!(at(&retried, &CLEAR, FAR), Decision::Rest);
        // The latch never broke: a limit re-booked after the move chains nothing.
        retried.push(watchdog(due + 900, LIMIT_ACTION, None));
        assert_eq!(at(&retried, &CLEAR, FAR), Decision::Rest);
    }

    #[test]
    fn an_alert_cleared_or_a_spawn_still_resets_a_refused_episode() {
        let (mut refused, _) = rounds_of(&[REFUSED_ACTION, FAILED_ACTION]);
        let mut respawned = refused.clone();
        refused.push(watchdog(5_000, CLEARED_ACTION, None));
        assert_eq!(fold(&refused), None);
        assert_eq!(at(&refused, &CLEAR, FAR), Decision::Rest);
        // The next limit is a fresh episode with a fresh count.
        refused.push(watchdog(6_000, LIMIT_ACTION, None));
        let fresh = fold(&refused).expect("a fresh episode");
        assert_eq!(
            (fresh.key, fresh.rounds, fresh.terminal, fresh.attempts),
            (after_key(6_000), 0, None, 0)
        );
        respawned.push(spawned(5_000));
        assert_eq!(fold(&respawned), None);
    }

    #[test]
    fn a_restart_that_rebooks_the_limit_reads_the_same_retry_from_the_journal() {
        let (base, closed) = rounds_of(&[REFUSED_ACTION]);
        let due = closed + RETRY_COOLDOWN_SECS;
        // A restart books the limit again: once while cooling down, once
        // after the retry came due.
        let mut rebooked = base.clone();
        rebooked.extend([
            watchdog(closed + 300, LIMIT_ACTION, None),
            watchdog(due + 50, LIMIT_ACTION, None),
        ]);
        assert_eq!(fold(&rebooked), fold(&base));
        let found = fold(&rebooked).expect("an episode");
        assert_eq!((found.reference(), found.rounds), (KEY, 1));
        for since in [closed + 1, due - 1, due, due + 60] {
            assert_eq!(
                at(&rebooked, &CLEAR, since),
                at(&base, &CLEAR, since),
                "{since}"
            );
        }
        assert_eq!(at(&rebooked, &CLEAR, due + 60), Decision::Attempt);
        // The same journal, folded twice, gives the same verdict.
        assert_eq!(fold(&rebooked), fold(&rebooked));
    }

    #[test]
    fn a_retry_attempt_reopens_the_episode_for_the_leg_under_the_same_key() {
        let seat = (SESSION, SLOT, AGENT);
        let (mut events, closed) = rounds_of(&[REFUSED_ACTION]);
        let due = closed + RETRY_COOLDOWN_SECS;
        // Closed, the leg finds nothing to act on.
        let shut = keyed(&events, seat, after_key(0)).expect("keyed");
        assert!(shut.terminal.is_some() && shut.open.is_none());
        // The retry's attempt, as the trigger journals it, opens it again.
        events.push(routed(due, ATTEMPT_ACTION, KEY));
        let found = keyed(&events, seat, after_key(0)).expect("keyed");
        assert_eq!(found.reference(), KEY);
        assert_eq!(
            (found.terminal, found.open, found.attempts, found.rounds),
            (None, Some(after_key(due)), 1, 1)
        );
        // It is in flight and overdue by the same bound.
        assert_eq!(
            at(&events, &CLEAR, due + IN_FLIGHT_SECS - 1),
            Decision::InFlight
        );
        assert_eq!(at(&events, &CLEAR, due + IN_FLIGHT_SECS), Decision::Overdue);
        // Its own closing is the next round.
        events.push(routed(due + 40, FAILED_ACTION, KEY));
        let again = keyed(&events, seat, after_key(0)).expect("keyed");
        assert_eq!(
            (again.terminal, again.open, again.rounds),
            (Some(Outcome::Failed), None, 2)
        );
        assert_eq!(
            at(&events, &CLEAR, due + 40 + RETRY_COOLDOWN_SECS - 1),
            Decision::Rest
        );
        assert_eq!(
            at(&events, &CLEAR, due + 40 + RETRY_COOLDOWN_SECS),
            Decision::Attempt
        );
    }

    #[test]
    fn a_refused_headroom_episode_is_not_retried_by_the_cooldown() {
        let events = [
            crossed(0),
            routed(5, ATTEMPT_ACTION, KEY),
            routed(9, REFUSED_ACTION, KEY),
        ];
        let found = headroom(&events);
        assert_eq!(
            decide(found.as_ref(), 600, &CLEAR, key_epoch() + 9 + FAR),
            Decision::Rest
        );
        assert_eq!(
            found.map(|found| found.terminal),
            Some(Some(Outcome::Refused))
        );
    }

    #[test]
    fn the_give_up_notice_names_the_step_the_human_takes() {
        let line = notice(SESSION, AGENT, &Ending::GaveUp);
        assert!(
            line.starts_with("auto reseat: other: builder not moved"),
            "{line}"
        );
        assert!(line.contains("gave up"), "{line}");
        assert!(
            line.ends_with(
                "move it by hand (ae reseat aedev builder --using <profile>) or wait for the reset."
            ),
            "{line}"
        );
        assert!(line.chars().count() <= NOTICE_CHARS, "{line}");
    }

    /// The hold the daemon journals for the give-up leads with its cause class
    /// like every other hold, so a clip of the notice that quotes it keeps the cause.
    #[test]
    fn the_give_up_hold_leads_with_its_cause_class() {
        assert_eq!(
            GAVE_UP_SUMMARY,
            "other: held: gave up after repeated refused or failed moves"
        );
        assert_eq!(
            CauseClass::Other.details(GAVE_UP_SUMMARY),
            "held: gave up after repeated refused or failed moves"
        );
    }

    fn named(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    /// An account is LEFT only when another one is proven: the tool differs, or
    /// the same tool names another proven store. Everything unproven is `None`.
    #[test]
    fn an_account_is_left_only_when_another_is_proven() {
        let claude_a = on(ToolKind::Claude, Some("opus"), proven("/a"));
        let claude_b = on(ToolKind::Claude, Some("opus"), proven("/b"));
        assert_eq!(claude_a.off_account(&claude_b), Some(true), "two homes");
        assert_eq!(claude_a.off_account(&claude_a), Some(false), "one home");
        // Muse and Grok have no account variable, so both are `Shared`: only
        // the tool tells them apart (the reason `Shape` carries one).
        let muse = on(ToolKind::Muse, Some("spark"), Store::Shared);
        let grok = on(ToolKind::Grok, Some("grok"), Store::Shared);
        assert_eq!(muse.off_account(&grok), Some(true), "two vendors");
        let wonder = on(ToolKind::Muse, Some("wonder"), Store::Shared);
        assert_eq!(muse.off_account(&wonder), Some(false), "one allowance");
        // A different tool proves a different account whatever the store says.
        let codex_unknown = on(ToolKind::Codex, Some("x"), Store::Unknown);
        assert_eq!(claude_a.off_account(&codex_unknown), Some(true));
        // Unproven is never "other", and a tool ae cannot name proves nothing.
        let unknown_store = on(ToolKind::Claude, Some("opus"), Store::Unknown);
        assert_eq!(claude_a.off_account(&unknown_store), None);
        assert_eq!(unknown_store.off_account(&claude_a), None);
        assert_eq!(unknown_store.off_account(&unknown_store), None);
        let shared_claude = on(ToolKind::Claude, Some("opus"), Store::Shared);
        assert_eq!(
            claude_a.off_account(&shared_claude),
            None,
            "one tool never mixes a proven and a shared store: fail closed"
        );
        let stranger = on(ToolKind::Unknown, None, Store::Shared);
        assert_eq!(stranger.off_account(&claude_a), None);
        assert_eq!(claude_a.off_account(&stranger), None);
        assert_eq!(stranger.off_account(&stranger), None);
    }

    /// `Shape.tool` is read by the cover verdict alone: the candidate list the
    /// seat's family derives is what it was.
    #[test]
    fn the_tool_changes_no_list_the_family_derives() {
        let seat = on(ToolKind::Muse, Some("spark"), Store::Shared);
        let all = rows(&[
            ("wonder", on(ToolKind::Muse, Some("wonder"), Store::Shared)),
            ("grok", on(ToolKind::Grok, Some("grok"), Store::Shared)),
        ]);
        assert_eq!(
            listed(&[], ("spark", &seat), &all),
            rows(&[("wonder", Provenance::Derived(Kin::Sibling))]),
            "a Grok profile is no sibling of a Muse one"
        );
    }

    #[test]
    fn the_first_listed_candidate_on_a_proven_other_account_is_the_exit() {
        let seat = on(ToolKind::Claude, Some("opus"), proven("/a"));
        let all = rows(&[
            ("sonnet", on(ToolKind::Claude, Some("sonnet"), proven("/a"))),
            ("home-b", on(ToolKind::Claude, Some("opus"), proven("/b"))),
            ("sol", on(ToolKind::Codex, Some("x"), proven("/c"))),
        ]);
        assert_eq!(
            cover(&[], ("src", &seat), &all),
            Cover::Exit("home-b".to_owned()),
            "a derived twin on another home is an exit with no declared row"
        );
        assert_eq!(
            cover(&named(&["sonnet", "sol"]), ("src", &seat), &all),
            Cover::Exit("sol".to_owned()),
            "declared order; a same-account candidate is passed over, not an exit"
        );
    }

    /// The Muse shape: every candidate shares the seat's one allowance.
    #[test]
    fn siblings_on_the_seats_own_account_are_candidates_and_no_exit() {
        let spark = on(ToolKind::Muse, Some("spark"), Store::Shared);
        let all = rows(&[
            ("spark", spark.clone()),
            ("wonder", on(ToolKind::Muse, Some("wonder"), Store::Shared)),
            ("grk", on(ToolKind::Grok, Some("grok"), Store::Shared)),
        ]);
        assert_eq!(
            listed(&[], ("spark", &spark), &all),
            rows(&[("wonder", Provenance::Derived(Kin::Sibling))])
        );
        assert_eq!(cover(&[], ("spark", &spark), &all), Cover::Bare);
        assert_eq!(
            cover(&named(&["grk"]), ("spark", &spark), &all),
            Cover::Exit("grk".to_owned()),
            "a declared Grok profile leaves a Muse seat's allowance"
        );
        let alone = rows(&[("lonely", on(ToolKind::Grok, None, Store::Shared))]);
        assert_eq!(
            cover(
                &[],
                ("lonely", &on(ToolKind::Grok, None, Store::Shared)),
                &alone
            ),
            Cover::Bare,
            "no candidate at all is bare"
        );
    }

    #[test]
    fn a_declared_name_no_profile_answers_to_is_no_exit() {
        let seat = on(ToolKind::Claude, Some("opus"), proven("/a"));
        let all = rows(&[("sol", on(ToolKind::Codex, Some("x"), proven("/c")))]);
        assert_eq!(
            cover(&named(&["ghost"]), ("src", &seat), &all),
            Cover::Bare,
            "a typo is not a way out"
        );
        assert_eq!(
            cover(&named(&["ghost", "sol"]), ("src", &seat), &all),
            Cover::Exit("sol".to_owned())
        );
    }

    /// Unproven names WHICH account: the seat's own, or a candidate's. Neither
    /// is ever covered, and a proven exit beats both.
    #[test]
    fn what_cannot_be_proven_is_never_covered_and_says_whose_account() {
        let known = on(ToolKind::Claude, Some("opus"), proven("/a"));
        let fuzzy = rows(&[("fuzzy", on(ToolKind::Claude, Some("opus"), Store::Unknown))]);
        assert_eq!(
            cover(&named(&["fuzzy"]), ("src", &known), &fuzzy),
            Cover::Unproven(Doubt::Candidate),
            "own account known, a candidate's is not"
        );
        let unknown = on(ToolKind::Claude, Some("opus"), Store::Unknown);
        let homed = rows(&[("home-b", on(ToolKind::Claude, Some("opus"), proven("/b")))]);
        assert_eq!(
            cover(&named(&["home-b"]), ("src", &unknown), &homed),
            Cover::Unproven(Doubt::Own),
            "own account unknown: nothing about another one proves it is another"
        );
        assert_eq!(
            cover(&[], ("src", &unknown), &[]),
            Cover::Unproven(Doubt::Own),
            "an unknown account with no candidate is still unknown"
        );
        let codex = rows(&[("sol", on(ToolKind::Codex, Some("x"), proven("/c")))]);
        assert_eq!(
            cover(&named(&["sol"]), ("src", &unknown), &codex),
            Cover::Exit("sol".to_owned()),
            "another TOOL proves another account even when the own store is unknown"
        );
        let stranger = on(ToolKind::Unknown, None, Store::Shared);
        let claude = rows(&[("c", on(ToolKind::Claude, Some("opus"), proven("/a")))]);
        assert_eq!(
            cover(&named(&["c"]), ("sh", &stranger), &claude),
            Cover::Unproven(Doubt::Own),
            "a tool ae cannot name has no account ae can name"
        );
        let strange = rows(&[("sh", stranger)]);
        assert_eq!(
            cover(&named(&["sh"]), ("src", &known), &strange),
            Cover::Unproven(Doubt::Candidate)
        );
        let mixed = rows(&[
            ("fuzzy", on(ToolKind::Claude, Some("opus"), Store::Unknown)),
            ("sol", on(ToolKind::Codex, Some("x"), proven("/c"))),
        ]);
        assert_eq!(
            cover(&named(&["fuzzy", "sol"]), ("src", &known), &mixed),
            Cover::Exit("sol".to_owned()),
            "a proven exit beats an unproven candidate before it"
        );
    }

    #[test]
    fn the_suggestion_follows_the_declared_rows_and_names_only_a_profile_that_leaves_the_account() {
        let own = on(ToolKind::Claude, Some("opus"), proven("/a"));
        let all = rows(&[
            ("sol", on(ToolKind::Codex, Some("x"), proven("/c"))),
            ("sol2", on(ToolKind::Codex, Some("y"), proven("/d"))),
            ("same", on(ToolKind::Claude, Some("sonnet"), proven("/a"))),
            ("grk", on(ToolKind::Grok, None, Store::Shared)),
            ("fuzzy", on(ToolKind::Claude, Some("o"), Store::Unknown)),
        ]);
        let declared: Vec<(String, Vec<String>)> = vec![
            (
                "other".to_owned(),
                named(&["same", "ghost", "sol", "src", "fuzzy", "sol2"]),
            ),
            ("again".to_owned(), named(&["grk"])),
        ];
        assert_eq!(
            suggest("src", &own, &declared, &all),
            named(&["sol", "grk"]),
            "declared order; never the seat, a typo, a same-account or an unproven profile; \
             the second prefers another tool than the first"
        );
        let twins: Vec<(String, Vec<String>)> = vec![("other".to_owned(), named(&["sol", "sol2"]))];
        assert_eq!(
            suggest("src", &own, &twins, &all),
            named(&["sol", "sol2"]),
            "with no other tool declared the second is the next one"
        );
        assert_eq!(
            suggest("src", &own, &[], &all),
            named(&["sol", "grk"]),
            "no declared row leaves a candidate: profiles in config order"
        );
        let cramped = rows(&[("same", on(ToolKind::Claude, Some("sonnet"), proven("/a")))]);
        assert!(
            suggest("src", &own, &[], &cramped).is_empty(),
            "no profile runs on another account: nothing is invented"
        );
    }

    #[test]
    fn a_scope_is_named_by_its_tool_and_its_account() {
        let claude = |store| Scope::Account {
            tool: ToolKind::Claude,
            store,
        };
        assert_eq!(
            claude(proven("/h/.claude")).label(),
            "claude · /h/.claude",
            "two homes of one tool are two scopes"
        );
        assert_eq!(
            claude(proven("/h/.claude-two")).label(),
            "claude · /h/.claude-two"
        );
        assert_eq!(
            Scope::Account {
                tool: ToolKind::Muse,
                store: Store::Shared
            }
            .label(),
            "muse · shared"
        );
        assert_eq!(
            claude(Store::Unknown).label(),
            "claude · account not provable"
        );
        assert_eq!(Scope::Unreadable.label(), "? · command not readable");
    }

    fn judged(profile: &str, cover: Cover, suggested: &[&str]) -> ProfileCover {
        ProfileCover {
            profile: profile.to_owned(),
            scope: Scope::Account {
                tool: ToolKind::Muse,
                store: Store::Shared,
            },
            cover,
            suggested: named(suggested),
        }
    }

    #[test]
    fn the_nudge_is_one_line_naming_the_profile_and_the_line_to_add() {
        assert_eq!(
            nudge_line(&judged("spark13m", Cover::Exit("sol61x".to_owned()), &[])),
            None,
            "a covered profile says nothing"
        );
        assert_eq!(
            nudge_line(&judged("spark13m", Cover::Bare, &["sol61x", "opus55x"])).as_deref(),
            Some(
                "ae: warning: profile 'spark13m' has no auto-reseat exit off its account — \
                 a usage limit would leave the seat stuck; add under [auto_reseat]: \
                 spark13m = sol61x, opus55x"
            )
        );
        assert_eq!(
            nudge_line(&judged("spark13m", Cover::Bare, &[])).as_deref(),
            Some(
                "ae: warning: profile 'spark13m' has no auto-reseat exit off its account — \
                 a usage limit would leave the seat stuck; no profile in [profiles] runs on \
                 another account, add one first"
            )
        );
        assert_eq!(
            nudge_line(&judged("p", Cover::Unproven(Doubt::Own), &[])).as_deref(),
            Some(
                "ae: warning: profile 'p' cannot prove an auto-reseat exit off its account — \
                 its own account is not provable; a usage limit may leave the seat stuck"
            )
        );
        assert_eq!(
            nudge_line(&judged("p", Cover::Unproven(Doubt::Own), &["x"])).as_deref(),
            Some(
                "ae: warning: profile 'p' cannot prove an auto-reseat exit off its account — \
                 its own account is not provable; a usage limit may leave the seat stuck; \
                 add under [auto_reseat]: p = x"
            ),
            "an exit on another tool is provable though the own account is not: say the line"
        );
        assert_eq!(
            nudge_line(&judged("p", Cover::Unproven(Doubt::Candidate), &["x"])).as_deref(),
            Some(
                "ae: warning: profile 'p' cannot prove an auto-reseat exit off its account — \
                 a candidate's account is not provable; a usage limit may leave the seat \
                 stuck; add under [auto_reseat]: p = x"
            )
        );
        for line in [
            nudge_line(&judged("p", Cover::Bare, &["x"])),
            nudge_line(&judged("p", Cover::Unproven(Doubt::Own), &[])),
        ]
        .into_iter()
        .flatten()
        {
            assert!(!line.contains('\n'), "one line: {line}");
        }
    }

    #[cfg(test)]
    mod keepconv_spec {
        #[test]
        fn automatic_kept_notice_names_kept_and_submitted_continuation() {
            let moved = super::super::Move {
                from: "old",
                to: "new",
                tool: ("claude", "claude"),
                model: ("opus", "sonnet"),
                fate: super::super::Fate::Kept,
                nudge: Some(super::super::Nudge::Submitted),
                critical: false,
                dirty: false,
                cause: "",
            };
            let actual = super::super::notice("s", "builder", &super::super::Ending::Moved(moved));
            assert_eq!(
                actual,
                "auto reseat: builder moved old -> new (tool claude -> claude, model opus -> sonnet), kept; work tree clean. If builder is half of a review pair, re-check its gate provider. Next: nothing, its continuation nudge was submitted."
            );
            assert_eq!(super::super::Fate::Kept.word(), "kept");
            assert_eq!(super::super::Fate::Carried.word(), "carried");
            assert_eq!(super::super::Fate::Seeded.word(), "seeded");
        }
    }
}
