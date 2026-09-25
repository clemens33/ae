//! The leg that MOVES a seat: `_auto-reseat <dir> <slot> <key>`.
//!
//! It re-derives everything the trigger decided from the records and the global
//! config, moves the seat through the one reseat, and journals ONE outcome. A
//! forged call can therefore do only what the watchdog would do at that moment:
//! it moves a seat only under an attempt the watchdog opened and has not closed.

use std::ffi::OsStr;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use crate::reseat::Ended;
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::time::Timestamp;
use crate::tracked::EventFields;
use crate::watchdog::WATCHDOG_ACTOR;

use super::{
    ATTEMPT_ACTION, Candidate, DONE_ACTION, FAILED_ACTION, HELD_ACTION, Ineligible, REFUSED_ACTION,
    Seat, Skip,
};

/// The usage line.
const USAGE: &str = "Usage: ae _auto-reseat <dir> <slot> <key>";

/// The widest a record's summary may be, in characters.
const SUMMARY_CHARS: usize = 160;

/// The argv, proven before anything is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Argv {
    pub session: String,
    /// The session directory, canonical.
    pub dir: PathBuf,
    pub slot: String,
    pub key: Timestamp,
}

/// Prove `<dir> <slot> <key>` against `sessions`, the state root's session
/// directory. The dir must already BE its place: absolute, no `.` or `..`,
/// and resolving to exactly `<sessions>/<its own name>` — so a link to another
/// session's directory names that other session and is refused.
pub(crate) fn parse_argv(sessions: &Path, argv: &[String]) -> Result<Argv, String> {
    let usage = || USAGE.to_owned();
    let [dir, slot, key] = argv else {
        return Err(usage());
    };
    let dir = PathBuf::from(dir);
    if !dir.is_absolute()
        || dir
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
    {
        return Err(usage());
    }
    let session = dir
        .file_name()
        .and_then(OsStr::to_str)
        .filter(|name| crate::session_launch::name::is_session_name(name))
        .ok_or_else(usage)?
        .to_owned();
    let root = crate::doors::canonical_strict_dir(sessions).map_err(|_| usage())?;
    let found = crate::doors::canonical_strict_dir(&dir).map_err(|_| usage())?;
    if found != root.join(&session) || !crate::requests::is_slot(slot) {
        return Err(usage());
    }
    let key = Timestamp::parse(key).ok_or_else(usage)?;
    Ok(Argv {
        session,
        dir: found,
        slot: slot.clone(),
        key,
    })
}

/// The record one outcome is journaled as: its action, `ref` and summary. A
/// move names the profile LEFT, which the next episode's chooser reads; every
/// other ending names the key and quotes the refusal the reseat printed.
pub(crate) fn outcome(
    ended: Ended,
    key: &str,
    left: &str,
    to: &str,
    err: &str,
) -> (&'static str, String, String) {
    // The refusal itself, not a note the reseat printed on its way to it.
    let quoted = || {
        bounded(
            err.lines()
                .find(|line| line.starts_with("Error:"))
                .or_else(|| err.lines().find(|line| !line.trim().is_empty()))
                .unwrap_or("the reseat printed no reason"),
        )
    };
    match ended {
        Ended::Moved { carried } => (
            DONE_ACTION,
            left.to_owned(),
            format!("to {to}, {}", if carried { "carried" } else { "seeded" }),
        ),
        Ended::Refused { transient: true } => (HELD_ACTION, key.to_owned(), quoted()),
        Ended::Refused { transient: false } => (REFUSED_ACTION, key.to_owned(), quoted()),
        Ended::Failed => (FAILED_ACTION, key.to_owned(), quoted()),
    }
}

/// The summary of the hold journaled when the seat is no longer eligible.
pub(crate) const fn ineligible_summary(why: Ineligible) -> &'static str {
    match why {
        Ineligible::Off => "held: auto reseat is off",
        Ineligible::Orchestrator => "held: an orchestrator session's seat is never moved",
        Ineligible::Session => "held: auto_reseat_sessions does not name this session",
        Ineligible::Slot => "held: the seat's slot is not one auto reseat moves",
        Ineligible::Class => "held: the main seat moves only when auto_reseat = all",
        Ineligible::Unmapped => "held: [auto_reseat] names no candidates for this profile",
    }
}

/// The declared candidates as the chooser judges them. No quota window and no
/// peer latch is read here, so every usable candidate judges `Unknown` and the
/// declared order decides.
pub(crate) fn candidates(
    list: &[String],
    resolves: impl Fn(&str) -> bool,
    left: &[(String, Timestamp)],
) -> Vec<Candidate> {
    list.iter()
        .map(|profile| Candidate {
            profile: profile.clone(),
            configured: resolves(profile),
            peer_latched: false,
            windows: Vec::new(),
            left_at: left
                .iter()
                .find(|(taken, _)| taken == profile)
                .map(|(_, at)| at.epoch()),
        })
        .collect()
}

/// Why a candidate was passed over, as a record says it.
const fn skip_word(skip: Skip) -> &'static str {
    match skip {
        Skip::Unconfigured => "not configured here",
        Skip::PeerLatched => "another seat is latched on its account",
        Skip::Exhausted => "exhausted",
        Skip::LeftOnLimit => "left on its limit",
    }
}

/// `text` as one control-free line of at most [`SUMMARY_CHARS`]: the pack's
/// neutraliser, then the journal's own flattening, held tighter than its cap
/// because the full text is on the leg's error stream.
fn bounded(text: &str) -> String {
    crate::state::summary_of(&crate::seatpack::neutralise(text))
        .chars()
        .take(SUMMARY_CHARS)
        .collect()
}

/// `_auto-reseat <dir> <slot> <key>`: move the seat at `slot` off its usage
/// limit as the watchdog, and journal ONE outcome for the attempt under `key`.
///
/// The order is the contract. The argv first, reading nothing. Then the seat
/// and its episode: a seat gone or an episode that is not `key`'s REFUSES, so
/// the attempt it was handed closes; an attempt already closed, or none open,
/// is not the leg's to act on and it journals nothing. Then the global config,
/// whose switch HOLDS, and the chooser, which refuses when nothing is usable.
/// Only then the move, whose ending is the outcome.
///
/// # Errors
///
/// Only a failure to write `err`.
pub(crate) fn run(
    root: &Path,
    tail: &[String],
    now: Timestamp,
    err: &mut impl Write,
) -> io::Result<u8> {
    let roots = crate::inventory::Roots::under(root);
    let argv = match parse_argv(roots.sessions(), tail) {
        Ok(argv) => argv,
        Err(line) => {
            writeln!(err, "{line}")?;
            return Ok(EXIT_USAGE);
        }
    };
    let key = argv.key.to_string();
    let dir = argv.dir.as_path();
    let bytes = crate::meta::read_bytes(dir).unwrap_or_default();
    let meta = crate::meta::Meta::parse(&String::from_utf8_lossy(&bytes));
    let Some(seat) = meta.roster().iter().find(|row| row.slot == argv.slot) else {
        let why = format!(
            "refused: {} seats no agent at slot {}",
            argv.session, argv.slot
        );
        return close(&argv, "", now, (REFUSED_ACTION, &why), err);
    };
    let (agent, profile) = (seat.name.clone(), seat.profile.clone().unwrap_or_default());
    let events = crate::watchdog_daemon::read_events(dir);
    let Some(found) = super::episode(&events, &argv.session, &argv.slot, &agent)
        .filter(|found| found.key == argv.key)
    else {
        let why = format!("refused: the seat is in no limit episode keyed {key}");
        return close(&argv, &agent, now, (REFUSED_ACTION, &why), err);
    };
    if found.terminal.is_some() || found.open.is_none() {
        writeln!(
            err,
            "ae: auto reseat: {} under {key} — nothing to do.",
            if found.terminal.is_some() {
                "the attempt has already ended"
            } else {
                "no open attempt"
            }
        )?;
        return Ok(EXIT_FAILED);
    }
    let settings = super::settings(Some(&crate::doors::config_file(
        crate::shape::current(),
        root,
    )));
    let orchestrator = crate::meta::meta_agent_role(&bytes) == crate::meta::MetaAgentRole::Role;
    let asked = Seat {
        session: &argv.session,
        slot: &argv.slot,
        agent: &agent,
        profile: &profile,
        orchestrator,
    };
    let held = |why| (HELD_ACTION, ineligible_summary(why));
    let list = match super::eligible(&settings, &asked) {
        Ok(list) => list,
        Err(why) => return close(&argv, &agent, now, held(why), err),
    };
    let left = super::left_profiles(&events, &argv.session, &argv.slot, &agent);
    let judged = candidates(list, |to| crate::reseat::resolves(dir, to), &left);
    let choice = super::choose(&judged, now.epoch());
    let Some((pick, _)) = choice.pick else {
        let why = no_candidate(&choice.skipped);
        return close(&argv, &agent, now, (REFUSED_ACTION, &why), err);
    };
    let (_, world) = crate::current_world(root);
    let mut said = Vec::new();
    let ended = crate::reseat::run_as_watchdog(
        root,
        Some(&world),
        &argv.session,
        &agent,
        &pick,
        now,
        &mut io::sink(),
        &mut said,
    )?;
    err.write_all(&said)?;
    let (action, reference, summary) = outcome(
        ended,
        &key,
        &profile,
        &pick,
        &String::from_utf8_lossy(&said),
    );
    record(&argv, &agent, now, action, &reference, &summary);
    Ok(if matches!(ended, Ended::Moved { .. }) {
        0
    } else {
        EXIT_FAILED
    })
}

/// What the trigger's own fresh reading of the seat's pane proved.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Sight {
    pub pane: super::Pane,
    /// The vendor's limit row is drawn in that same capture.
    pub limited: bool,
}

/// What the trigger does for one seat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Plan {
    /// Nothing, and nothing is written: why, for the trigger's error stream.
    Decline(String),
    /// Close the episode keyed `key`: nothing declared is usable.
    Refuse { key: Timestamp, why: String },
    /// Open an attempt under `key` and start the leg that moves the seat to `to`.
    Attempt { key: Timestamp, to: String },
}

/// Decide the trigger from the records and one fresh sight, by the daemon's
/// own rule: the seat is eligible, the limit row is drawn, and its episode
/// decides [`super::Decision::Attempt`] NOW.
pub(crate) fn plan(
    settings: &super::Settings,
    seat: &Seat<'_>,
    sight: &Sight,
    events: &[crate::events::Event],
    resolves: impl Fn(&str) -> bool,
    now: Timestamp,
) -> Plan {
    let list = match super::eligible(settings, seat) {
        Ok(list) => list,
        Err(why) => return Plan::Decline(format!("not eligible ({why:?})")),
    };
    if !sight.limited {
        return Plan::Decline("no usage limit is drawn".to_owned());
    }
    let found = super::episode(events, seat.session, seat.slot, seat.agent);
    let decision = super::decide(
        found.as_ref(),
        settings.grace_secs,
        &sight.pane,
        now.epoch(),
    );
    let (Some(found), super::Decision::Attempt) = (found, decision) else {
        return Plan::Decline(format!("not due ({decision:?})"));
    };
    let left = super::left_profiles(events, seat.session, seat.slot, seat.agent);
    let choice = super::choose(&candidates(list, resolves, &left), now.epoch());
    match choice.pick {
        Some((to, _)) => Plan::Attempt { key: found.key, to },
        None => Plan::Refuse {
            key: found.key,
            why: no_candidate(&choice.skipped),
        },
    }
}

/// The refusal that names each declared candidate passed over, and why.
fn no_candidate(skipped: &[(String, Skip)]) -> String {
    let named: Vec<String> = skipped
        .iter()
        .map(|(to, skip)| format!("{to} ({})", skip_word(*skip)))
        .collect();
    format!("refused: no usable candidate: {}", named.join(", "))
}

/// Journal the attempt, THEN start the leg: the leg acts only under an attempt
/// it can read. A leg that could not start closes the attempt at once.
fn commit(
    argv: &Argv,
    agent: &str,
    (from, to): (&str, &str),
    now: Timestamp,
    spawn: impl FnOnce() -> bool,
    err: &mut impl Write,
) -> io::Result<u8> {
    let key = argv.key.to_string();
    record(
        argv,
        agent,
        now,
        ATTEMPT_ACTION,
        &key,
        &format!("from {from} to {to}"),
    );
    if spawn() {
        return Ok(0);
    }
    let why = "failed: the move could not be started";
    close(argv, agent, now, (FAILED_ACTION, why), err)
}

/// `send` under the attempt action: the daemon's trigger. It re-derives the
/// whole decision under the seat's lock, so a forged call does exactly what the
/// daemon would do now; anything else declines with one line and writes
/// nothing.
///
/// # Errors
///
/// Only a failure to write `err`.
pub(crate) fn trigger(
    dir: &Path,
    target: &str,
    own_session: &str,
    now: Timestamp,
    err: &mut impl Write,
) -> io::Result<u8> {
    let decline = |err: &mut dyn Write, why: &str| -> io::Result<u8> {
        writeln!(err, "ae: auto reseat: {target} declined: {why}")?;
        Ok(EXIT_FAILED)
    };
    let (resolved, server) = match crate::tracked::resolve_on(target, own_session, dir) {
        Ok(found) if found.0.session == own_session && !found.0.slot.is_empty() => found,
        _ => return decline(err, "not a seat of this session"),
    };
    let slot = resolved.slot.as_str();
    let Ok(_held) = crate::store::lock(&super::lock_path(dir, slot), std::time::Duration::ZERO)
    else {
        return decline(err, "another writer holds the seat");
    };
    let bytes = crate::meta::read_bytes(dir).unwrap_or_default();
    let meta = crate::meta::Meta::parse(&String::from_utf8_lossy(&bytes));
    let Some(row) = meta.roster().iter().find(|row| row.slot == slot) else {
        return decline(err, "no seat at its slot");
    };
    let bin = row.binary.clone().unwrap_or_default();
    let tool = crate::tool::ToolKind::from_binary_name(&bin);
    let capture = crate::transport::capture_pane(&server, &resolved.pane);
    let text = capture.as_deref().unwrap_or_default();
    let clients = crate::transport::observe_clients(&server);
    let input = crate::watchdog_daemon::viewing_input(clients.as_deref(), &resolved.pane);
    let sight = Sight {
        pane: super::Pane {
            frame: super::frame_of(
                capture.is_some(),
                crate::harness_state::classify(text, tool),
                crate::harness_state::has_human_draft(text, tool),
            ),
            human_prompt: crate::watchdog::human_prompt_class(text, &bin).is_some(),
            client_input: input.and_then(|at| i64::try_from(at).ok()),
        },
        limited: crate::watchdog::limit_notice(text, &bin).is_some(),
    };
    let config =
        crate::state_root().map(|root| crate::doors::config_file(crate::shape::current(), &root));
    let profile = row.profile.clone().unwrap_or_default();
    let seat = Seat {
        session: own_session,
        slot,
        agent: &row.name,
        profile: &profile,
        orchestrator: crate::meta::meta_agent_role(&bytes) == crate::meta::MetaAgentRole::Role,
    };
    let events = crate::watchdog_daemon::read_events(dir);
    let usable = |to: &str| crate::reseat::resolves(dir, to);
    let argv = |key| Argv {
        session: own_session.to_owned(),
        dir: dir.to_path_buf(),
        slot: slot.to_owned(),
        key,
    };
    match plan(
        &super::settings(config.as_deref()),
        &seat,
        &sight,
        &events,
        usable,
        now,
    ) {
        Plan::Decline(why) => decline(err, &why),
        Plan::Refuse { key, why } => close(&argv(key), &row.name, now, (REFUSED_ACTION, &why), err),
        Plan::Attempt { key, to } => {
            let exe = crate::shape::resolved_exe();
            let leg = crate::session_launch::capture::auto_reseat_argv(dir, slot, key);
            // No core path, no argv (unreachable here: eligibility asked the
            // same slot grammar) and a refused spawn are one failure: the
            // attempt is journaled, then closed `failed`, and the verb exits 1.
            let spawn = || {
                exe.zip(leg)
                    .is_some_and(|(exe, leg)| crate::transport::spawn_detached(&exe, &leg))
            };
            commit(&argv(key), &row.name, (&profile, &to), now, spawn, err)
        }
    }
}

/// Close the attempt under `argv`'s key with `(action, summary)`, and say so.
fn close(
    argv: &Argv,
    agent: &str,
    now: Timestamp,
    (action, summary): (&str, &str),
    err: &mut impl Write,
) -> io::Result<u8> {
    record(argv, agent, now, action, &argv.key.to_string(), summary);
    writeln!(err, "ae: auto reseat: {summary}")?;
    Ok(EXIT_FAILED)
}

/// Journal one record as the watchdog, routed to the seat by slot and session
/// so the seat's episode fold reads it whatever the seat is called.
fn record(argv: &Argv, agent: &str, now: Timestamp, action: &str, reference: &str, summary: &str) {
    let _ = crate::store::open(&argv.dir).append_event(&crate::tracked::event_line(&EventFields {
        ts: now,
        actor: WATCHDOG_ACTOR,
        action,
        target: if agent.is_empty() { &argv.slot } else { agent },
        reference,
        actor_slot: "",
        actor_session: "",
        target_slot: &argv.slot,
        target_session: &argv.session,
        target_server: "",
        target_pane: "",
        target_session_uuid: "",
        caller_server: "",
        caller_pane: "",
        caller_session_uuid: "",
        identity_gap: "",
        summary: &bounded(summary),
        body_file: "",
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autoreseat::{
        DONE_ACTION, FAILED_ACTION, HELD_ACTION, REFUSED_ACTION, Skip, Tier, choose, left_profiles,
    };
    use crate::events::Event;

    const KEY: &str = "2026-09-25T10:00:00Z";

    /// A state root with one session directory, removed with the test.
    struct Root(PathBuf);

    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn words(argv: &[&str]) -> Vec<String> {
        argv.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn the_argv_is_proven_before_anything_is_read() {
        let root = Root(std::env::temp_dir().join(format!("ae-leg-{}", std::process::id())));
        let sessions = root.0.join("sessions");
        let dir = sessions.join("aedev");
        assert!(std::fs::create_dir_all(&dir).is_ok(), "a session dir");
        assert!(std::fs::create_dir_all(sessions.join("bad name")).is_ok());
        assert!(std::os::unix::fs::symlink(&dir, sessions.join("alias")).is_ok());
        let path = |name: &str| sessions.join(name).display().to_string();
        let parsed = parse_argv(&sessions, &words(&[&path("aedev"), "spawned.3", KEY]));
        assert_eq!(
            parsed.map(|argv| (argv.session, argv.slot, argv.key.to_string())),
            Ok(("aedev".to_owned(), "spawned.3".to_owned(), KEY.to_owned()))
        );
        let climbs = format!("{}/../sessions/aedev", sessions.display());
        for argv in [
            words(&[&path("aedev"), "spawned.3"]),
            words(&[&path("aedev"), "spawned.3", KEY, "extra"]),
            words(&[&climbs, "spawned.3", KEY]),
            words(&[&path("alias"), "spawned.3", KEY]),
            words(&[&path("missing"), "spawned.3", KEY]),
            words(&[&path("bad name"), "spawned.3", KEY]),
            words(&["aedev", "spawned.3", KEY]),
            words(&[&path("aedev"), "worker.x", KEY]),
            words(&[&path("aedev"), "spawned.3", "yesterday"]),
        ] {
            assert!(parse_argv(&sessions, &argv).is_err(), "{argv:?}");
        }
    }

    #[test]
    fn each_ending_is_journaled_as_one_outcome_with_its_ref() {
        let left = "sol6x";
        for (ended, action, reference, summary) in [
            (
                Ended::Moved { carried: true },
                DONE_ACTION,
                left,
                "to opus55x, carried",
            ),
            (
                Ended::Moved { carried: false },
                DONE_ACTION,
                left,
                "to opus55x, seeded",
            ),
            (
                Ended::Refused { transient: true },
                HELD_ACTION,
                KEY,
                "Error: busy",
            ),
            (
                Ended::Refused { transient: false },
                REFUSED_ACTION,
                KEY,
                "Error: busy",
            ),
            (Ended::Failed, FAILED_ACTION, KEY, "Error: busy"),
        ] {
            assert_eq!(
                outcome(ended, KEY, left, "opus55x", "Error: busy\n  next line\n"),
                (action, reference.to_owned(), summary.to_owned()),
                "{ended:?}"
            );
        }
        // The summary is ONE line, control-free and bounded.
        let (_, _, summary) = outcome(
            Ended::Failed,
            KEY,
            left,
            "opus55x",
            &format!("\u{1b}[31mError: {}\n", "x".repeat(900)),
        );
        assert!(
            !summary.contains('\u{1b}') && !summary.contains('\n'),
            "{summary:?}"
        );
        assert!(summary.len() <= 200, "{}", summary.len());
    }

    #[test]
    fn each_reason_a_seat_is_no_longer_eligible_reads_as_its_own_hold() {
        let summaries = [
            Ineligible::Off,
            Ineligible::Orchestrator,
            Ineligible::Session,
            Ineligible::Slot,
            Ineligible::Class,
            Ineligible::Unmapped,
        ]
        .map(ineligible_summary);
        for (index, summary) in summaries.iter().enumerate() {
            assert!(summary.starts_with("held: "), "{summary:?}");
            assert!(summary.len() > "held: ".len(), "{summary:?}");
            assert!(!summaries[..index].contains(summary), "{summary:?}");
        }
    }

    #[test]
    fn with_no_quota_read_the_first_usable_declared_candidate_is_taken() {
        let done = Event::parse_line(&format!(
            r#"{{"ts":"{KEY}","actor":"watchdog","action":"auto-reseat-done","target":"scout","ref":"sol6x"}}"#
        ))
        .expect("a well-formed record");
        let left = left_profiles(&[done], "aedev", "spawned.3", "scout");
        let list = words(&["ghost", "sol6x", "opus55x", "astrax"]);
        let choice = choose(&candidates(&list, |profile| profile != "ghost", &left), 0);
        assert_eq!(choice.pick, Some(("opus55x".to_owned(), Tier::Unknown)));
        assert_eq!(
            choice.skipped,
            [
                ("ghost".to_owned(), Skip::Unconfigured),
                ("sol6x".to_owned(), Skip::LeftOnLimit),
            ]
        );
    }

    /// With no `Error:` line the first line that says anything is quoted, and
    /// with no output at all the record says so.
    #[test]
    fn an_ending_with_no_error_line_quotes_the_first_line_that_says_anything() {
        for (err, summary) in [
            ("\n  \ntmux said no\nmore\n", "tmux said no"),
            ("", "the reseat printed no reason"),
        ] {
            assert_eq!(
                outcome(Ended::Failed, KEY, "sol6x", "opus55x", err).2,
                summary,
                "{err:?}"
            );
        }
    }

    /// The trigger decides by the daemon's own rule over its OWN fresh sight:
    /// anything short of an eligible seat, drawing its limit, due NOW, declines
    /// and writes nothing; a due seat with nothing usable is refused; otherwise
    /// one attempt to the first usable candidate.
    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one table of trigger rows against one seat, read top to bottom"
    )]
    fn the_trigger_attempts_only_what_the_daemon_would_attempt_now() {
        use crate::autoreseat::{ATTEMPT_ACTION, Frame, Pane, Settings, Switch};
        let key = Timestamp::parse(KEY).expect("the key parses");
        let record = |since: i64, action: &str, reference: &str| {
            Event::parse_line(&format!(
                r#"{{"ts":"{}","actor":"watchdog","action":"{action}","target":"scout","ref":"{reference}"}}"#,
                Timestamp::from_epoch(key.epoch() + since)
            ))
            .expect("a well-formed record")
        };
        let limit = [record(0, "limit", "")];
        let open = [record(0, "limit", ""), record(5, ATTEMPT_ACTION, KEY)];
        let ended = [record(0, "limit", ""), record(5, REFUSED_ACTION, KEY)];
        let on = |grace_secs| Settings {
            switch: Switch::On,
            sessions: None,
            grace_secs,
            map: vec![("sol6x".to_owned(), vec!["opus55x".to_owned()])],
            notes: Vec::new(),
        };
        let seat = Seat {
            session: "aedev",
            slot: "spawned.3",
            agent: "scout",
            profile: "sol6x",
            orchestrator: false,
        };
        let sight = |frame, human_prompt, client_input, limited| Sight {
            pane: Pane {
                frame,
                human_prompt,
                client_input,
            },
            limited,
        };
        let clear = sight(Frame::Clear, false, None, true);
        let now = Timestamp::from_epoch(key.epoch() + 700);
        let attempt = Plan::Attempt {
            key,
            to: "opus55x".to_owned(),
        };
        let plan_of = |settings: &Settings, sight: &Sight, events: &[Event]| {
            plan(settings, &seat, sight, events, |_| true, now)
        };
        assert_eq!(plan_of(&on(600), &clear, &limit), attempt);
        let touched = |ago: i64| sight(Frame::Clear, false, Some(now.epoch() - ago), true);
        assert_eq!(
            plan_of(&on(600), &touched(600), &limit),
            attempt,
            "input aged out"
        );
        assert_eq!(
            plan_of(&on(0), &touched(1), &limit),
            attempt,
            "no grace, no input hold"
        );
        let off = Settings {
            switch: Switch::Off,
            ..on(600)
        };
        let unmapped = Settings {
            map: Vec::new(),
            ..on(600)
        };
        for (settings, sight, events, why) in [
            (&off, clear, &limit[..], "off"),
            (&unmapped, clear, &limit[..], "unmapped"),
            (
                &on(600),
                sight(Frame::Clear, false, None, false),
                &limit[..],
                "no limit row",
            ),
            (
                &on(600),
                sight(Frame::Busy, false, None, true),
                &limit[..],
                "busy",
            ),
            (
                &on(600),
                sight(Frame::Draft, false, None, true),
                &limit[..],
                "draft",
            ),
            (
                &on(600),
                sight(Frame::Unread, false, None, true),
                &limit[..],
                "unread",
            ),
            (
                &on(600),
                sight(Frame::Clear, true, None, true),
                &limit[..],
                "human prompt",
            ),
            (&on(600), touched(599), &limit[..], "input inside the grace"),
            (&on(800), clear, &limit[..], "not due"),
            (&on(600), clear, &open[..], "an attempt is open"),
            (&on(600), clear, &ended[..], "the episode ended"),
            (&on(600), clear, &[][..], "no episode"),
        ] {
            assert!(
                matches!(plan_of(settings, &sight, events), Plan::Decline(_)),
                "{why}"
            );
        }
        assert_eq!(
            plan(&on(600), &seat, &clear, &limit, |_| false, now),
            Plan::Refuse {
                key,
                why: "refused: no usable candidate: opus55x (not configured here)".to_owned()
            }
        );
    }

    /// The attempt is DURABLE before the leg starts, because the leg acts only
    /// under an attempt it can read; a leg that could not start closes the
    /// attempt at once instead of leaving it to age out.
    #[test]
    fn the_attempt_is_journaled_before_the_leg_starts_and_a_leg_that_cannot_start_closes_it() {
        use crate::autoreseat::ATTEMPT_ACTION;
        let root = Root(std::env::temp_dir().join(format!("ae-leg-commit-{}", std::process::id())));
        let dir = root.0.join("aedev");
        assert!(std::fs::create_dir_all(&dir).is_ok(), "a session dir");
        let argv = Argv {
            session: "aedev".to_owned(),
            dir: dir.clone(),
            slot: "spawned.3".to_owned(),
            key: Timestamp::parse(KEY).expect("the key parses"),
        };
        let journal = || {
            crate::watchdog_daemon::read_events(&dir)
                .into_iter()
                .map(|event| {
                    (
                        event.action,
                        event.reference.unwrap_or_default(),
                        event.summary.unwrap_or_default(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let now = Timestamp::from_epoch(argv.key.epoch() + 700);
        let mut before = Vec::new();
        let mut err = Vec::new();
        let code = commit(
            &argv,
            "scout",
            ("sol6x", "opus55x"),
            now,
            || {
                before = journal();
                false
            },
            &mut err,
        );
        let attempt = (
            ATTEMPT_ACTION.to_owned(),
            KEY.to_owned(),
            "from sol6x to opus55x".to_owned(),
        );
        assert_eq!(
            before,
            std::slice::from_ref(&attempt),
            "durable before the start"
        );
        assert_eq!(code.ok(), Some(EXIT_FAILED));
        assert_eq!(
            journal(),
            [
                attempt.clone(),
                (
                    FAILED_ACTION.to_owned(),
                    KEY.to_owned(),
                    "failed: the move could not be started".to_owned()
                ),
            ]
        );
        let _ = std::fs::remove_file(dir.join(crate::store::EVENTS));
        let started = commit(&argv, "scout", ("sol6x", "opus55x"), now, || true, &mut err);
        assert_eq!(started.ok(), Some(0));
        assert_eq!(journal(), [attempt]);
    }
}
