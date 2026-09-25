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
use crate::time::Timestamp;
use crate::watchdog::{WATCHDOG_ACTOR, event_is_addressed_to};

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
        let _ = profile;
        None
    }
}

/// Read the GLOBAL config now. A project overlay never steers spend, and the
/// switch is read on every decision, so turning it off stops the next move.
#[must_use]
pub fn settings(global: Option<&Path>) -> Settings {
    let _ = global;
    Settings::off(Vec::new())
}

fn settle(
    switch: Switch,
    sessions: Result<Option<String>, String>,
    grace: Result<Option<String>, String>,
    map: Result<Vec<SectionEntry>, String>,
) -> Settings {
    let _ = (switch, sessions, grace, map);
    Settings::off(Vec::new())
}

fn parse_switch(raw: Result<Option<String>, String>) -> Result<Switch, String> {
    let _ = raw;
    Ok(Switch::Off)
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
        let _ = slot;
        None
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
    let _ = (settings, seat);
    Err(Ineligible::Off)
}

/// What an attempt came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Refused,
    Failed,
    /// A transient refusal: the episode may try once more.
    Held,
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
}

/// The episode the seat is in now, or `None` when it has none.
#[must_use]
pub fn episode(events: &[Event], session: &str, slot: &str, agent: &str) -> Option<Episode> {
    let _ = (events, session, slot, agent);
    None
}

/// Each profile this seat left by auto reseat since its newest `spawn`, with
/// the time of the newest such move.
#[must_use]
pub fn left_profiles(
    events: &[Event],
    session: &str,
    slot: &str,
    agent: &str,
) -> Vec<(String, Timestamp)> {
    let _ = (events, session, slot, agent);
    Vec::new()
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
#[must_use]
pub fn decide(episode: Option<&Episode>, grace_secs: u64, pane: &Pane, now: i64) -> Decision {
    let _ = (episode, grace_secs, pane, now);
    Decision::Rest
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
    pub status: crate::quota::Status,
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
    let _ = (candidates, now);
    Choice {
        pick: None,
        skipped: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quota::Status;

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
        ];
        let found = fold(&stale).expect("an episode");
        assert_eq!((found.attempts, found.terminal), (0, None));
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
        let events = [
            routed(-10_800, DONE_ACTION, "fablex"),
            spawned(-7200),
            routed(-3600, DONE_ACTION, "sol6x"),
            routed(-1800, DONE_ACTION, "opus55x"),
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
            // Nothing read since the move, or only before it.
            Vec::new(),
            vec![window(20.0, false, 4_000, Status::Fresh)],
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
}
