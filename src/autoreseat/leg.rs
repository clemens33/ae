//! The leg that MOVES a seat: `_auto-reseat <dir> <slot> <key>`.
//!
//! It re-derives everything the trigger decided from the records and the global
//! config, moves the seat through the one reseat, and journals ONE outcome. A
//! forged call can therefore do only what the watchdog would do at that moment:
//! it moves a seat only under an attempt the watchdog opened and has not closed.

use std::ffi::OsStr;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use crate::reseat::{Ended, Resolved};
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

/// The declared candidates as the chooser judges them, joined with `quota`:
/// the windows that bind each one's account and whether another seat of the
/// session is latched on it.
pub(crate) fn candidates(
    list: &[String],
    resolve: impl Fn(&str) -> Option<Resolved>,
    left: &[(String, Timestamp)],
    quota: Option<&crate::quota::Observation>,
    latched: &[crate::quota::RecordedIdentity],
    now: i64,
) -> Vec<Candidate> {
    let _ = (quota, latched, now);
    list.iter()
        .map(|profile| Candidate {
            profile: profile.clone(),
            configured: resolve(profile).is_some(),
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
    let judged = candidates(
        list,
        |to| crate::reseat::resolved(dir, to),
        &left,
        None,
        &[],
        now.epoch(),
    );
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
    gather: impl Fn(&[String], &[(String, Timestamp)]) -> Vec<Candidate>,
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
    let choice = super::choose(&gather(list, &left), now.epoch());
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

/// The line a seat's first-sight limit notice gains when auto reseat may move
/// it: where to and when, or that nothing declared is usable. A forecast: the
/// legs choose again when they act.
pub(crate) fn deadline(agent: &str, choice: &super::Choice, grace_secs: u64) -> String {
    let _ = (agent, choice, grace_secs);
    String::new()
}

/// The environment of every notice delivery, whole. The leg runs under the
/// trigger's own environment, whose action is the attempt: a notice that did
/// not name its own would be taken for another trigger.
pub(crate) fn notice_env(summary: &str) -> [(&'static str, &str); 3] {
    let _ = summary;
    [("", ""), ("", ""), ("", "")]
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
    let gather = |list: &[String], left: &[(String, Timestamp)]| {
        candidates(
            list,
            |to| crate::reseat::resolved(dir, to),
            left,
            None,
            &[],
            now.epoch(),
        )
    };
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
        gather,
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
        let resolve = |profile: &str| (profile != "ghost").then(|| Resolved { pin: None });
        let choice = choose(&candidates(&list, resolve, &left, None, &[], 0), 0);
        assert_eq!(choice.pick, Some(("opus55x".to_owned(), Tier::Unknown)));
        assert_eq!(
            choice.skipped,
            [
                ("ghost".to_owned(), Skip::Unconfigured),
                ("sol6x".to_owned(), Skip::LeftOnLimit),
            ]
        );
    }

    const NOW: i64 = 1_800_000_000;

    /// One Claude weekly window, observed `ago` seconds before [`NOW`] and
    /// resetting `resets_in` seconds after it.
    fn row(qualifier: Option<&str>, used: &str, ago: i64, resets_in: i64) -> crate::quota::Row {
        crate::quota::Row {
            bucket: "weekly".to_owned(),
            qualifier: qualifier.map(ToOwned::to_owned),
            window_minutes: Some(10_080),
            used_percent: Some(used.to_owned()),
            resets_at: Some(NOW + resets_in),
            observed_at: Some(NOW - ago),
            status: crate::quota::Status::Fresh,
        }
    }

    fn plain() -> crate::quota::Policy {
        crate::quota::Policy::for_tests(None, crate::quota::Account::default())
    }

    /// The account at `source` that `profiles` launch on, read as `rows`.
    fn account(
        profiles: &[&str],
        source: &str,
        policy: crate::quota::Policy,
        rows: Vec<crate::quota::Row>,
    ) -> crate::quota::Group {
        crate::quota::Group {
            profiles: words(profiles),
            tool: crate::tool::ToolKind::Claude,
            home: None,
            source: Some(PathBuf::from(source)),
            clients: Vec::new(),
            rollout: None,
            owner: None,
            rows,
            hint: None,
            summary: None,
            policy,
            notes: Vec::new(),
        }
    }

    fn observed(groups: Vec<crate::quota::Group>) -> crate::quota::Observation {
        crate::quota::Observation {
            groups,
            rendered: Vec::new(),
            home: None,
            now: NOW,
        }
    }

    /// Choose among `list` over `quota`, each candidate pinned as `pins` says.
    fn judge(
        quota: &crate::quota::Observation,
        list: &[&str],
        pins: &[(&str, &str)],
        latched: &[crate::quota::RecordedIdentity],
        left: &[(String, Timestamp)],
    ) -> crate::autoreseat::Choice {
        let resolve = |profile: &str| {
            Some(Resolved {
                pin: pins
                    .iter()
                    .find(|(named, _)| *named == profile)
                    .map(|(_, pin)| (*pin).to_owned()),
            })
        };
        choose(
            &candidates(&words(list), resolve, left, Some(quota), latched, NOW),
            NOW,
        )
    }

    fn picked(name: &str, tier: Tier) -> Option<(String, Tier)> {
        Some((name.to_owned(), tier))
    }

    fn passed(rows: &[(&str, Skip)]) -> Vec<(String, Skip)> {
        rows.iter()
            .map(|(name, skip)| ((*name).to_owned(), *skip))
            .collect()
    }

    #[test]
    fn a_candidate_is_judged_by_its_effective_percentage_and_a_spend_cap_exhausts_it() {
        let capped = crate::quota::Account {
            spend_control_reached: Some(true),
            spend_observed_at: Some(NOW - 60),
            ..crate::quota::Account::default()
        };
        let declared = crate::quota::Policy::for_tests(Some(1), crate::quota::Account::default());
        let quota = observed(vec![
            account(
                &["declared"],
                "/a",
                declared,
                vec![row(None, "100", 60, 3600)],
            ),
            account(&["plain"], "/b", plain(), vec![row(None, "100", 60, 3600)]),
            account(
                &["capped"],
                "/c",
                crate::quota::Policy::for_tests(None, capped),
                vec![row(None, "10", 60, 3600)],
            ),
        ]);
        let choice = judge(&quota, &["plain", "capped", "declared"], &[], &[], &[]);
        assert_eq!(
            choice.pick,
            picked("declared", Tier::BelowCritical),
            "raw 100 over 1+1 declared resets judges 50"
        );
        assert_eq!(
            choice.skipped,
            passed(&[("plain", Skip::Exhausted), ("capped", Skip::Exhausted)])
        );
    }

    #[test]
    fn a_window_scoped_to_another_family_binds_nobody_else_and_a_pinless_candidate_is_bound() {
        let quota = observed(vec![account(
            &["fable-x", "bare", "opus-x"],
            "/a",
            plain(),
            vec![
                row(Some("Fable"), "100", 60, 3600),
                row(None, "10", 60, 3600),
            ],
        )]);
        let pins = [("fable-x", "fable"), ("opus-x", "opus")];
        let choice = judge(&quota, &["fable-x", "bare", "opus-x"], &pins, &[], &[]);
        assert_eq!(choice.pick, picked("opus-x", Tier::BelowCritical));
        assert_eq!(
            choice.skipped,
            passed(&[("fable-x", Skip::Exhausted), ("bare", Skip::Exhausted)])
        );
    }

    /// Another seat latched on an account passes it over; the moving seat's
    /// own account, left out of the latch, still reads exhausted from its own
    /// window when the whole account is spent.
    #[test]
    fn another_latched_seat_passes_its_account_over_and_a_spent_account_stays_exhausted() {
        let quota = observed(vec![
            account(&["shared"], "/a", plain(), vec![row(None, "10", 60, 3600)]),
            account(&["own"], "/b", plain(), vec![row(None, "100", 60, 3600)]),
            account(
                &["elsewhere"],
                "/c",
                plain(),
                vec![row(None, "10", 60, 3600)],
            ),
        ]);
        let on = |tool, source: &str| crate::quota::RecordedIdentity {
            tool,
            source: PathBuf::from(source),
        };
        let latched = [
            on(crate::tool::ToolKind::Claude, "/a"),
            on(crate::tool::ToolKind::Codex, "/c"),
        ];
        let choice = judge(&quota, &["shared", "own", "elsewhere"], &[], &latched, &[]);
        assert_eq!(
            choice.pick,
            picked("elsewhere", Tier::BelowCritical),
            "another tool on the same path is another account"
        );
        assert_eq!(
            choice.skipped,
            passed(&[("shared", Skip::PeerLatched), ("own", Skip::Exhausted)])
        );
    }

    #[test]
    fn only_a_fresh_reading_places_a_tier_and_critical_is_the_classifiers() {
        let quota = observed(vec![
            account(&["hot"], "/a", plain(), vec![row(None, "95", 60, 3600)]),
            account(&["warm"], "/b", plain(), vec![row(None, "94", 60, 3600)]),
            account(
                &["stale"],
                "/c",
                plain(),
                vec![row(None, "10", 20 * 60, 3600)],
            ),
            crate::quota::Group {
                source: None,
                ..account(
                    &["sourceless"],
                    "/d",
                    plain(),
                    vec![row(None, "100", 60, 3600)],
                )
            },
        ]);
        let all = ["hot", "stale", "sourceless", "warm"];
        let choice = judge(&quota, &all, &[], &[], &[]);
        assert_eq!(choice.pick, picked("warm", Tier::BelowCritical));
        assert!(choice.skipped.is_empty(), "{:?}", choice.skipped);
        let pick = |list: &[&str]| judge(&quota, list, &[], &[], &[]).pick;
        assert_eq!(
            pick(&["hot", "stale"]),
            picked("stale", Tier::Unknown),
            "stale places no tier"
        );
        assert_eq!(pick(&["hot"]), picked("hot", Tier::Critical));
        assert_eq!(
            pick(&["sourceless"]),
            picked("sourceless", Tier::Unknown),
            "an account with no source joins nothing"
        );
    }

    /// Several rollouts report one Codex account. Their rows are judged
    /// together, so the union must never be more permissive than one reading.
    #[test]
    fn a_rollout_row_from_a_window_already_reset_never_counts_and_an_exhausted_one_always_does() {
        let rollout = |rows| crate::quota::Group {
            tool: crate::tool::ToolKind::Codex,
            ..account(&["sol"], "/a", plain(), rows)
        };
        let previous = row(None, "100", 3 * 3600, -60);
        let current = row(None, "40", 60, 3600);
        for groups in [
            vec![
                rollout(vec![previous.clone()]),
                rollout(vec![current.clone()]),
            ],
            vec![
                rollout(vec![current.clone()]),
                rollout(vec![previous.clone()]),
            ],
        ] {
            let choice = judge(&observed(groups), &["sol"], &[], &[], &[]);
            assert_eq!(
                choice.pick,
                picked("sol", Tier::BelowCritical),
                "the reset row is gone"
            );
        }
        let spent = row(None, "100", 120, 3600);
        for groups in [
            vec![rollout(vec![spent.clone()]), rollout(vec![current.clone()])],
            vec![rollout(vec![current.clone()]), rollout(vec![spent.clone()])],
        ] {
            let choice = judge(&observed(groups), &["sol"], &[], &[], &[]);
            assert_eq!(choice.skipped, passed(&[("sol", Skip::Exhausted)]));
        }
    }

    /// A profile left on its limit waits for a window read after the move:
    /// the stamps and the reset the gather carries are the ones the rule reads.
    #[test]
    fn a_profile_left_on_its_limit_is_relieved_only_by_a_reading_after_it_left() {
        let left = [("sol".to_owned(), Timestamp::from_epoch(NOW - 600))];
        let before = observed(vec![account(
            &["sol"],
            "/a",
            plain(),
            vec![row(None, "10", 900, 3600)],
        )]);
        let after = observed(vec![account(
            &["sol"],
            "/a",
            plain(),
            vec![row(None, "10", 60, 3600)],
        )]);
        let reset = observed(vec![account(
            &["sol"],
            "/a",
            plain(),
            vec![row(None, "100", 300, -10)],
        )]);
        let skip =
            |quota: &crate::quota::Observation| judge(quota, &["sol"], &[], &[], &left).skipped;
        assert_eq!(skip(&before), passed(&[("sol", Skip::LeftOnLimit)]));
        assert!(skip(&after).is_empty());
        assert!(skip(&reset).is_empty(), "its window reset after it left");
    }

    #[test]
    fn the_first_sight_line_names_the_target_and_the_grace_or_why_nothing_is_usable() {
        let to = |tier| crate::autoreseat::Choice {
            pick: picked("opus55x", tier),
            skipped: Vec::new(),
        };
        assert_eq!(
            deadline("scout", &to(Tier::BelowCritical), 600),
            "ae will move scout to opus55x in 10m"
        );
        assert_eq!(
            deadline("scout", &to(Tier::Unknown), 0),
            "ae will move scout to opus55x in 0s"
        );
        assert_eq!(
            deadline("scout", &to(Tier::Critical), 90),
            "ae will move scout to opus55x in 1m, target already critical"
        );
        let none = crate::autoreseat::Choice {
            pick: None,
            skipped: passed(&[("opus55x", Skip::Exhausted), ("ghost", Skip::Unconfigured)]),
        };
        assert_eq!(
            deadline("scout", &none, 600),
            "ae cannot move scout: no usable candidate: opus55x (exhausted), ghost (not configured here)"
        );
    }

    /// The leg inherits the trigger's environment, whose action is the
    /// attempt: every notice names its own, and its actor, whole.
    #[test]
    fn every_notice_names_its_own_action_and_the_watchdog_as_its_sender() {
        assert_eq!(
            notice_env("told"),
            [
                ("AE_SENDER_OVERRIDE", "watchdog"),
                ("_AE_EVENT_ACTION", "auto-reseat-notice"),
                ("_AE_EVENT_SUMMARY", "told"),
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
        let usable = |list: &[String], left: &[(String, Timestamp)]| {
            candidates(list, |_| Some(Resolved { pin: None }), left, None, &[], 0)
        };
        let plan_of = |settings: &Settings, sight: &Sight, events: &[Event]| {
            plan(settings, &seat, sight, events, usable, now)
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
            plan(
                &on(600),
                &seat,
                &clear,
                &limit,
                |list, left| candidates(list, |_| None, left, None, &[], 0),
                now
            ),
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
        let why = "failed: the move could not be started";
        assert_eq!(
            journal(),
            [
                attempt.clone(),
                (FAILED_ACTION.to_owned(), KEY.to_owned(), why.to_owned()),
                (
                    "chat".to_owned(),
                    String::new(),
                    crate::autoreseat::notice(
                        "aedev",
                        "scout",
                        &crate::autoreseat::Ending::Failed(why)
                    ),
                ),
            ]
        );
        let told = crate::watchdog_daemon::read_events(&dir);
        let chat = told.last().expect("the chat line");
        assert_eq!(chat.actor, WATCHDOG_ACTOR, "{chat:?}");
        assert!(
            chat.target.as_deref().unwrap_or_default().is_empty(),
            "target-less: {chat:?}"
        );
        let _ = std::fs::remove_file(dir.join(crate::store::EVENTS));
        let started = commit(&argv, "scout", ("sol6x", "opus55x"), now, || true, &mut err);
        assert_eq!(started.ok(), Some(0));
        assert_eq!(journal(), [attempt]);
    }
    /// The trigger acts only for a seat of its OWN session: `@session:agent`
    /// resolves across sessions, and a foreign seat — or a pane with no slot —
    /// declines with the one line before the lock is taken or anything read.
    #[test]
    fn a_trigger_aimed_outside_its_own_session_declines_before_the_lock() {
        let root = Root(std::env::temp_dir().join(format!("ae-leg-own-{}", std::process::id())));
        let dir = root.0.join("aedev");
        assert!(std::fs::create_dir_all(&dir).is_ok(), "a session dir");
        let meta = "seat.spawned.3=scout\nprofile.spawned.3=sol6x\n";
        assert!(std::fs::write(dir.join(crate::store::META), meta).is_ok());
        for (target, session, slot) in [
            ("@other:scout", "other", "spawned.3"),
            ("scout", "aedev", ""),
        ] {
            crate::tracked::set_test_resolve(
                crate::tracked::Resolved {
                    pane: "%7".to_owned(),
                    agent: target.to_owned(),
                    slot: slot.to_owned(),
                    session: session.to_owned(),
                },
                crate::inventory::ServerId::Ambient,
            );
            let mut err = Vec::new();
            let now = Timestamp::parse(KEY).expect("the key parses");
            let code = trigger(&dir, target, "aedev", now, &mut err);
            crate::tracked::clear_test_hooks();
            assert_eq!(code.ok(), Some(crate::state::EXIT_FAILED), "{target}");
            assert_eq!(
                String::from_utf8_lossy(&err),
                format!("ae: auto reseat: {target} declined: not a seat of this session\n")
            );
            let lock = std::fs::remove_file(crate::autoreseat::lock_path(&dir, "spawned.3"));
            assert!(
                lock.is_err_and(|why| why.kind() == io::ErrorKind::NotFound),
                "{target}: no lock taken"
            );
            assert!(
                crate::watchdog_daemon::read_events(&dir).is_empty(),
                "{target}: nothing journaled"
            );
        }
    }
}
