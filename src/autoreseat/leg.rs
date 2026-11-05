//! The leg that MOVES a seat: `_auto-reseat <dir> <slot> <key>`.
//!
//! It re-derives everything the trigger decided from the records and the global
//! config, moves the seat through the one reseat, and journals ONE outcome. A
//! forged call can therefore do only what the watchdog would do at that moment:
//! it moves a seat only under an attempt the watchdog opened and has not closed.

use std::ffi::OsStr;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use crate::reseat::{Ended, Resolved};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::time::Timestamp;
use crate::tracked::EventFields;
use crate::watchdog::WATCHDOG_ACTOR;

use super::{
    ATTEMPT_ACTION, Candidate, CauseClass, Cover, DONE_ACTION, Doubt, FAILED_ACTION, HELD_ACTION,
    Ineligible, ProfileCover, Provenance, REFUSED_ACTION, Room, Scope, Seat, Settings, Shape, Skip,
    Store, Switch, Trigger,
};

#[cfg(test)]
#[allow(
    clippy::format_push_string,
    reason = "frozen #309 acceptance spec, lint-only"
)]
#[path = "leadfloor_spec.rs"]
mod leadfloor_spec;

/// The usage line.
const USAGE: &str = "Usage: ae _auto-reseat <dir> <slot> <key>";

/// The widest a record's summary may be, in characters.
const SUMMARY_CHARS: usize = 160;

/// A tool or model the records do not name.
const UNKNOWN: &str = "unknown";

/// The observed-choice hold a no-candidate ending names instead, so a seat held
/// for its human's choice is never told it has nowhere to go (#269).
fn observed_ending(dir: &Path, slot: &str) -> Option<String> {
    let hold = crate::run::observed_standing(dir, slot)?;
    Some(CauseClass::ObservedChoice.lead(&super::observed_summary(&hold)))
}

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
/// other ending names the key and quotes the refusal the reseat printed, led
/// by the class the refusing site named so a clip never takes it.
pub(crate) fn outcome(
    ended: Ended,
    key: &str,
    left: &str,
    to: &str,
    err: &str,
) -> (&'static str, String, String) {
    // The refusal itself, not a note the reseat printed on its way to it.
    let quoted = |class: CauseClass| {
        bounded(
            &class.lead(
                err.lines()
                    .find(|line| line.starts_with("Error:"))
                    .or_else(|| err.lines().find(|line| !line.trim().is_empty()))
                    .unwrap_or("the reseat printed no reason"),
            ),
        )
    };
    match ended {
        Ended::Moved { fate } => (
            DONE_ACTION,
            left.to_owned(),
            format!("to {to}, {}", fate.word()),
        ),
        Ended::Refused(class) if class.transient() => (HELD_ACTION, key.to_owned(), quoted(class)),
        Ended::Refused(class) => (REFUSED_ACTION, key.to_owned(), quoted(class)),
        Ended::Failed => (FAILED_ACTION, key.to_owned(), quoted(CauseClass::Other)),
    }
}

/// The summary of the hold the daemon names for `reason` in an episode with
/// `cause`: what its writer journals and what its reader compares the journal
/// with, so the two cannot drift apart.
pub(crate) fn held_summary(reason: super::HoldReason, cause: &str) -> String {
    tagged(&reason.record(), cause)
}

/// `summary` naming its episode's trigger: unchanged for a limit episode, whose
/// `cause` is empty, and for a headroom one the cause appended inside the
/// summary bound, so a cut never takes the trigger.
pub(crate) fn tagged(summary: &str, cause: &str) -> String {
    if cause.is_empty() {
        return summary.to_owned();
    }
    let tag = format!(" ({cause})");
    let room = SUMMARY_CHARS.saturating_sub(tag.chars().count());
    let head: String = bounded(summary).chars().take(room).collect();
    format!("{head}{tag}")
}

/// The hold a headroom attempt is closed with once the threshold that applies
/// to its seat, the config row `key`, is off.
fn headroom_off(key: &str) -> String {
    format!("held: {key} is off")
}

/// An attempt started before its grace whose start proof ended on the way here.
const START_GONE: &str = "held: the start's exhausted reading no longer stands";

/// A headroom attempt whose seat's own account, read again by the leg, no
/// longer reads at or within the re-arm gap of the threshold, or reads nothing.
const HEADROOM_GONE: &str = "held: the account no longer reads at the headroom threshold";

/// A return's hold: its origin account no longer proves room on the leg's read.
const RETURN_GONE: &str = "held: the origin account no longer proves room";

/// A return's hold: the seat is no longer on an automatic chain away from its
/// origin, or the origin no longer resolves.
const RETURN_OFF_CHAIN: &str = "held: the seat is no longer on its automatic chain";

/// A return's hold: the leg ran before the episode's grace had passed.
const RETURN_EARLY: &str = "held: the return is not due yet";

/// The hold journaled when the seat is no longer eligible, led by its class.
pub(crate) fn ineligible_record(why: Ineligible) -> String {
    why.class().lead(ineligible_summary(why))
}

/// The words of that hold.
pub(crate) const fn ineligible_summary(why: Ineligible) -> &'static str {
    match why {
        Ineligible::Off => "held: auto reseat is off",
        Ineligible::Orchestrator => "held: an orchestrator session's seat is never moved",
        Ineligible::Session => "held: auto_reseat_sessions does not name this session",
        Ineligible::Slot => "held: the seat's slot is not one auto reseat moves",
        Ineligible::Class => "held: the main seat moves only when auto_reseat = all",
        Ineligible::Unmapped => "held: no declared or derived candidate for this profile",
    }
}

/// The seat's candidates, `declared` then derived from its own family by
/// [`super::listed`], read as a move of it reads them: the session's identity
/// config, the seat's recorded profile through its recorded client, and the
/// store the seat RECORDS — never a model it was followed to. A config or a
/// seat command ae cannot read derives nothing. A lead-pair seat
/// ([`super::lead_seat`]) keeps its derived siblings on the list as
/// [`Provenance::LeadSibling`], which the chooser passes over (#309).
pub(crate) fn listed(
    dir: &Path,
    bytes: &[u8],
    seat: &crate::meta::RosterEntry,
    declared: &[String],
) -> Vec<(String, Provenance)> {
    let home = crate::doors::home();
    let home = home.as_deref();
    let derived = crate::reseat::identity_config(dir, bytes)
        .ok()
        .and_then(|cfg| {
            let profile = seat.profile.as_deref()?;
            let command = crate::run::read_seat_command(
                &cfg,
                profile,
                &seat.name,
                &seat.slot,
                &seat.client,
                home,
            )
            .ok()?;
            let (words, pin, tool) = skeleton(&command)?;
            let store = match (tool.adapter().config_home_env, &seat.config_home) {
                (None, _) => Store::Shared,
                (
                    Some(_),
                    crate::meta::RecordedConfigHome::Path(path)
                    | crate::meta::RecordedConfigHome::Implicit(path),
                ) => Store::Proven(path.clone()),
                (Some(_), _) => Store::Unknown,
            };
            let profiles = shapes(&cfg, home);
            let raw = command.as_str().to_owned();
            Some(super::listed(
                declared,
                (
                    profile,
                    &Shape {
                        words,
                        pin,
                        store,
                        raw,
                        tool,
                    },
                ),
                &profiles,
            ))
        });
    let list = derived.unwrap_or_else(|| {
        declared
            .iter()
            .map(|name| (name.clone(), Provenance::Declared))
            .collect()
    });
    // A lead-pair seat never changes model on its own (#309): its derived
    // siblings stay listed, so the chooser names them, but are never taken.
    let lead_pair = crate::lifecycle::meta_value(bytes, "layout") == "lead-pair";
    if !super::lead_seat(lead_pair, &seat.slot) {
        return list;
    }
    list.into_iter()
        .map(|(name, provenance)| {
            let barred = match provenance {
                Provenance::Derived(super::Kin::Sibling, step) => Provenance::LeadSibling(step),
                other => other,
            };
            (name, barred)
        })
        .collect()
}

/// A resolved command's family skeleton and its tool.
fn skeleton(
    command: &crate::config::ResolvedCommand,
) -> Option<(Vec<String>, Option<String>, crate::tool::ToolKind)> {
    let tool = crate::launch_cmd::lex_simple_command(command.as_str())
        .ok()?
        .tool();
    let (words, pin) = crate::launch_cmd::family_skeleton(
        command.as_str(),
        tool,
        crate::launch_cmd::Account::Drop,
    )?;
    Some((words, pin, tool))
}

/// Profile `name` as derivation compares it, its store resolved as a move to
/// it would resolve it.
fn shape(cfg: &crate::config::IdentityConfig, name: &str, home: Option<&Path>) -> Option<Shape> {
    let command = cfg.command(name, home).ok()??;
    let (words, pin, tool) = skeleton(&command)?;
    let store = if tool.adapter().config_home_env.is_none() {
        Store::Shared
    } else {
        crate::reseat::account_of(&command, tool, home)
            .map_or(Store::Unknown, |account| Store::Proven(account.path))
    };
    let raw = command.as_str().to_owned();
    Some(Shape {
        words,
        pin,
        store,
        raw,
        tool,
    })
}

/// Every `[profiles]` entry whose command reads, as derivation compares it,
/// in config order. An entry that does not read is left out here — a
/// candidate list cannot name it — and [`covers`] judges it apart.
fn shapes(cfg: &crate::config::IdentityConfig, home: Option<&Path>) -> Vec<(String, Shape)> {
    cfg.profiles
        .iter()
        .filter_map(|(name, _)| Some((name.clone(), shape(cfg, name, home)?)))
        .collect()
}

/// The cover verdict of EVERY `[profiles]` key in config order, each over the
/// candidates its `[auto_reseat]` row (none is fine) and its own family give
/// it, or `None` while auto reseat is off. A profile whose command does not
/// read is `Unproven`, never left out, and never covered.
pub(crate) fn covers(
    cfg: &crate::config::IdentityConfig,
    settings: &Settings,
    home: Option<&Path>,
) -> Option<Vec<ProfileCover>> {
    if settings.switch == Switch::Off {
        return None;
    }
    let profiles = shapes(cfg, home);
    Some(
        cfg.profiles
            .iter()
            .map(|(name, _)| {
                let Some((_, own)) = profiles.iter().find(|(key, _)| key == name) else {
                    return ProfileCover {
                        profile: name.clone(),
                        scope: Scope::Unreadable,
                        cover: Cover::Unproven(Doubt::Own),
                        suggested: Vec::new(),
                    };
                };
                let declared = settings.candidates(name).unwrap_or_default();
                let cover = super::cover(declared, (name, own), &profiles);
                let suggested = if matches!(cover, Cover::Exit(_)) {
                    Vec::new()
                } else {
                    super::suggest(name, own, &settings.map, &profiles)
                };
                ProfileCover {
                    profile: name.clone(),
                    scope: Scope::Account {
                        tool: own.tool,
                        store: own.store.clone(),
                    },
                    cover,
                    suggested,
                }
            })
            .collect(),
    )
}

/// The one line `spawn` and a human's `reseat` print to say the profile
/// chosen has no exit off its account, or `None` when auto reseat is off, the
/// profile is covered, or it is not a profile of `cfg`.
pub(crate) fn cover_warning(
    cfg: &crate::config::IdentityConfig,
    settings: &Settings,
    profile: &str,
    home: Option<&Path>,
) -> Option<String> {
    covers(cfg, settings, home)?
        .iter()
        .find(|one| one.profile == profile)
        .and_then(super::nudge_line)
}

/// [`cover_warning`] for a seat that runs `profile` through `command`, the
/// command a `profile@client` choice resolved to: judged on the account THAT
/// command selects, never the profile's default one. The candidates are the
/// profile's own, as a bare seat of it has them — its `[auto_reseat]` row and
/// its family among `[profiles]` — so a pair never becomes an exit.
pub(crate) fn cover_warning_through(
    cfg: &crate::config::IdentityConfig,
    settings: &Settings,
    profile: &str,
    command: &crate::config::ResolvedCommand,
    home: Option<&Path>,
) -> Option<String> {
    if settings.switch == Switch::Off {
        return None;
    }
    let Some((words, pin, tool)) = skeleton(command) else {
        return super::nudge_line(&ProfileCover {
            profile: profile.to_owned(),
            scope: Scope::Unreadable,
            cover: Cover::Unproven(Doubt::Own),
            suggested: Vec::new(),
        });
    };
    let store = if tool.adapter().config_home_env.is_none() {
        Store::Shared
    } else {
        crate::reseat::account_of(command, tool, home)
            .map_or(Store::Unknown, |account| Store::Proven(account.path))
    };
    let own = Shape {
        words,
        pin,
        store,
        raw: command.as_str().to_owned(),
        tool,
    };
    let profiles = shapes(cfg, home);
    let declared = settings.candidates(profile).unwrap_or_default();
    let cover = super::cover(declared, (profile, &own), &profiles);
    let suggested = if matches!(cover, Cover::Exit(_)) {
        Vec::new()
    } else {
        super::suggest(profile, &own, &settings.map, &profiles)
    };
    super::nudge_line(&ProfileCover {
        profile: profile.to_owned(),
        scope: Scope::Account {
            tool: own.tool,
            store: own.store.clone(),
        },
        cover,
        suggested,
    })
}

/// The provider the seat of roster row `row` serves NOW: its recorded profile
/// through its recorded client, as its own launch reads it, classified by
/// [`crate::tool::served_provider`] — and only when that command's tool is the
/// tool the row records running (`agent_bin`). Anything ae cannot read, and a
/// command that disagrees with the running tool, proves nothing.
fn serves(
    cfg: &crate::config::IdentityConfig,
    row: &crate::meta::RosterEntry,
    home: Option<&Path>,
) -> Option<crate::tool::Provider> {
    let profile = row.profile.as_deref()?;
    let command =
        crate::run::read_seat_command(cfg, profile, &row.name, &row.slot, &row.client, home)
            .ok()?;
    let tool = crate::launch_cmd::lex_simple_command(command.as_str())
        .ok()?
        .tool();
    let running = crate::tool::ToolKind::from_binary_name(row.binary.as_deref()?);
    (tool.is_known() && tool == running)
        .then(|| crate::tool::served_provider(command.as_str()))
        .flatten()
}

/// The pair guard of the seat at `slot`, read from `bytes` (the session's
/// meta) and its identity config, or `None` for a seat with no pair partner
/// ([`super::partner`]). Every `[profiles]` entry is resolved as a move to it
/// resolves it, so each candidate's provider is the one its move would serve.
pub(crate) fn pair_of(
    dir: &Path,
    bytes: &[u8],
    roster: &[crate::meta::RosterEntry],
    lead_pair: bool,
    slot: &str,
) -> Option<super::PairGuard> {
    let partner = match super::partner(roster, lead_pair, slot) {
        super::Link::Solo => return None,
        super::Link::Partner(row) => Some(row),
        super::Link::Ambiguous => None,
    };
    let home = crate::doors::home();
    let home = home.as_deref();
    let cfg = crate::reseat::identity_config(dir, bytes).ok();
    let read = |row: &crate::meta::RosterEntry| cfg.as_ref().and_then(|cfg| serves(cfg, row, home));
    let seat = roster.iter().find(|row| row.slot == slot);
    let destinations = cfg.as_ref().map_or_else(Vec::new, |cfg| {
        cfg.profiles
            .iter()
            .map(|(name, _)| {
                let command = cfg.command(name, home).ok().flatten();
                let serves = command.and_then(|found| crate::tool::served_provider(found.as_str()));
                (name.clone(), serves)
            })
            .collect()
    });
    Some(super::PairGuard {
        partner: partner.map(|row| row.name.clone()),
        partner_serves: partner.and_then(read),
        seat_serves: seat.and_then(read),
        destinations,
    })
}

/// [`pair_of`] over the session meta at `dir` as it reads now.
fn pair_now(dir: &Path, slot: &str) -> Option<super::PairGuard> {
    let bytes = crate::meta::read_bytes(dir).unwrap_or_default();
    let meta = crate::meta::Meta::parse(&String::from_utf8_lossy(&bytes));
    let lead_pair = crate::lifecycle::meta_value(&bytes, "layout") == "lead-pair";
    pair_of(dir, &bytes, meta.roster(), lead_pair, slot)
}

/// THE LIFECYCLE GATE (#305), asked by an automatic reseat under the session's
/// lifecycle lock and before any stop, over `locked`, the meta read under that
/// lock: why a move of the seat at `slot` to the profile resolving to `command`
/// no longer keeps its pair on two proven providers, or `None` when it does,
/// the pair already serves one proven provider that the destination is proven
/// on too ([`super::PairGuard::stays_serving`]), or the seat has no partner.
pub(crate) fn pair_changed(dir: &Path, locked: &[u8], slot: &str, command: &str) -> Option<String> {
    let meta = crate::meta::Meta::parse(&String::from_utf8_lossy(locked));
    let lead_pair = crate::lifecycle::meta_value(locked, "layout") == "lead-pair";
    let pair = pair_of(dir, locked, meta.roster(), lead_pair, slot)?;
    let to = crate::tool::served_provider(command);
    // A pair already collapsed may stay on its provider, proven (R7).
    if pair.stays_serving(to).is_some() {
        return None;
    }
    pair.judge(to)?;
    Some(format!(
        "auto reseat: the pair changed before the stop: {}, and the destination serves {} — \
         nothing was stopped",
        pair.words(),
        to.map_or("an unproven provider", crate::tool::Provider::word)
    ))
}

/// The listed candidates as the chooser judges them, joined with `quota`:
/// the windows that bind each one's account, per account and window only the
/// newest usable reading ([`super::current`]), and whether another seat of the
/// session is latched on it.
pub(crate) fn candidates(
    list: &[(String, Provenance)],
    resolve: impl Fn(&str) -> Option<Resolved>,
    (left, left_on_headroom): (&[(String, Timestamp)], &[String]),
    quota: Option<&crate::quota::Observation>,
    latched: &[crate::quota::RecordedIdentity],
    now: i64,
) -> Vec<Candidate> {
    let groups = quota.map_or(&[][..], |seen| seen.groups.as_slice());
    list.iter()
        .map(|(profile, provenance)| {
            let resolved = resolve(profile);
            let pin = resolved.as_ref().and_then(|found| found.pin.as_deref());
            // Every reading of the candidate's account, rollouts together: one
            // that does not bind its model family is no reading of it.
            let accounts: Vec<(&crate::quota::Group, &std::path::PathBuf)> = groups
                .iter()
                .filter(|group| group.profiles.iter().any(|named| named == profile))
                .filter_map(|group| group.source.as_ref().map(|source| (group, source)))
                .collect();
            Candidate {
                profile: profile.clone(),
                provenance: *provenance,
                configured: resolved.is_some(),
                peer_latched: accounts.iter().any(|(group, source)| {
                    latched
                        .iter()
                        .any(|seat| seat.tool == group.tool && &seat.source == *source)
                }),
                // Per account and window the newest usable reading rules; one
                // account's reading never supersedes another's.
                windows: super::current(
                    accounts
                        .iter()
                        .flat_map(|(group, source)| {
                            group
                                .rows
                                .iter()
                                .filter(|row| {
                                    !crate::quota::scoped_elsewhere(
                                        group.tool,
                                        row.qualifier.as_deref(),
                                        pin,
                                    )
                                })
                                .filter_map(|row| {
                                    let key =
                                        (group.tool, *source, crate::quota::WindowKey::of(row));
                                    window(group, row, now).map(|window| (key, window))
                                })
                        })
                        .collect(),
                )
                .into_iter()
                .map(|(_, window)| window)
                .collect(),
                left_at: left
                    .iter()
                    .find(|(taken, _)| taken == profile)
                    .map(|(_, at)| at.epoch()),
                left_on_headroom: left_on_headroom.iter().any(|taken| taken == profile),
            }
        })
        .collect()
}

/// One row as the quota module judges it: the effective percentage, the one
/// classifier's level, and whether the number still stands at `now`.
fn window(group: &crate::quota::Group, row: &crate::quota::Row, now: i64) -> Option<super::Window> {
    let reading = crate::quota::Reading::of(group.policy.clone(), row.clone())?;
    let judged = crate::quota::Classified::first(reading);
    let status = match row.status {
        crate::quota::Status::Fresh | crate::quota::Status::Stale => {
            crate::quota::freshness(row.observed_at, row.resets_at, now)
        }
        other => other,
    };
    Some(super::Window {
        judged: judged.judged(),
        critical: judged.level() == crate::quota::QuotaLevel::Critical,
        long: crate::quota::proven_long(row.window_minutes),
        observed_at: judged.observed_at(),
        resets_at: row.resets_at,
        status,
        label: crate::quota::window_name(row),
    })
}

/// The seat's OWN account as `quota` reads it.
pub(crate) struct OwnScope {
    /// Each window that binds the seat's model family, with its identity.
    pub(crate) windows: Vec<(crate::quota::WindowKey, super::Window)>,
    /// A usable row of the scope, of ANY length and model family, reports a
    /// reached spend cap: the cap is the account's, not one window's.
    pub(crate) spend_capped: bool,
    /// The first FRESH row that reports the cap.
    pub(crate) fresh_cap: Option<super::Window>,
}

/// The windows of the one scope the seat's recorded identity names — the join
/// the throttle line and the checkpoint ask make — that bind its model family,
/// per window only the newest usable reading ([`super::current`]).
/// No proven identity, no observation: nothing is read.
pub(crate) fn own_scope(
    quota: Option<&crate::quota::Observation>,
    entry: &crate::meta::RosterEntry,
    pin: Option<&str>,
    now: i64,
) -> Option<OwnScope> {
    let (Some(seen), Some(identity)) = (quota, crate::quota::recorded_identity(entry)) else {
        return None;
    };
    let mut scope = OwnScope {
        windows: Vec::new(),
        spend_capped: false,
        fresh_cap: None,
    };
    for group in seen.groups.iter().filter(|group| {
        group.tool == identity.tool && group.source.as_ref() == Some(&identity.source)
    }) {
        for row in &group.rows {
            let Some(window) = window(group, row, now) else {
                continue;
            };
            let capped = group.policy.spend_capped();
            scope.spend_capped |= capped && super::usable(&window);
            if capped && window.status == crate::quota::Status::Fresh {
                scope.fresh_cap.get_or_insert_with(|| window.clone());
            }
            if !crate::quota::scoped_elsewhere(group.tool, row.qualifier.as_deref(), pin) {
                scope
                    .windows
                    .push((crate::quota::WindowKey::of(row), window));
            }
        }
    }
    // The scope's rollouts each hold their own last reading of a window; the
    // newest usable one rules, so a reading a newer one superseded never decides.
    scope.windows = super::current(scope.windows);
    Some(scope)
}

/// How pressed the seat's OWN account is: the worst judged percentage among
/// [`own_scope`]'s usable windows proven at least a day long, the same windows
/// [`own_room`] weighs; a reached spend cap is the account's and reads full.
/// `None` when nothing is decided: no proven identity, no observation, or no
/// such window.
pub(crate) fn own_pressure(
    quota: Option<&crate::quota::Observation>,
    entry: &crate::meta::RosterEntry,
    pin: Option<&str>,
    now: i64,
) -> Option<f64> {
    let scope = own_scope(quota, entry, pin, now)?;
    if scope.spend_capped {
        return Some(super::FULL);
    }
    let windows: Vec<super::Window> = scope.windows.into_iter().map(|(_, w)| w).collect();
    super::worst_long(&windows).map(|window| window.judged)
}

/// The seat's OWN account against the threshold `at`: [`own_scope`]'s windows.
pub(crate) fn own_room(
    quota: Option<&crate::quota::Observation>,
    entry: &crate::meta::RosterEntry,
    pin: Option<&str>,
    at: u8,
    now: i64,
) -> Room {
    own_scope(quota, entry, pin, now).map_or(Room::Unread, |scope| {
        let windows: Vec<super::Window> = scope.windows.into_iter().map(|(_, w)| w).collect();
        super::room(&windows, at)
    })
}

/// Whether the seat's OWN account still presses the headroom threshold `at`:
/// [`own_room`] reads it crossed, or within [`super::REARM_GAP`] below it, under
/// the pin the seat's `profile` resolves to. The trigger judges this before it
/// reserves an attempt, and the leg again, on its own read, before it moves.
fn source_pressed(
    quota: Option<&crate::quota::Observation>,
    dir: &Path,
    entry: &crate::meta::RosterEntry,
    profile: &str,
    at: u8,
    now: Timestamp,
) -> bool {
    let pin = crate::reseat::resolved(dir, profile).and_then(|found| found.pin);
    matches!(
        own_room(quota, entry, pin.as_deref(), at, now.epoch()),
        Room::Crossed(_) | Room::Near
    )
}

/// With no threshold, the seat's OWN account as a start's episode asks: proven
/// relief, and anything less says nothing.
pub(crate) fn own_relief(
    quota: Option<&crate::quota::Observation>,
    entry: &crate::meta::RosterEntry,
    pin: Option<&str>,
    now: i64,
) -> Room {
    own_scope(quota, entry, pin, now)
        .filter(|scope| !scope.spend_capped)
        .and_then(|scope| super::relieved(&scope.windows))
        .map_or(Room::Unread, Room::Relieved)
}

/// The fresh window that proves the seat's OWN account has nothing left.
///
/// Stricter than [`own_scope`], which keeps a window nothing proves elsewhere:
/// this one waives a wait, so a model-scoped window must be proven to bind the
/// seat ([`crate::quota::scoped_here`]). A spend cap is the account's.
pub(crate) fn own_exhausted(
    quota: Option<&crate::quota::Observation>,
    entry: &crate::meta::RosterEntry,
    pin: Option<&str>,
    now: i64,
) -> Option<super::Window> {
    let tool = crate::quota::recorded_identity(entry)?.tool;
    let scope = own_scope(quota, entry, pin, now)?;
    let capped = scope.fresh_cap.is_some();
    let bound: Vec<_> = scope
        .windows
        .into_iter()
        .filter(|(key, _)| crate::quota::scoped_here(tool, key.qualifier.as_deref(), pin))
        .collect();
    super::exhausted(&bound, capped).or(scope.fresh_cap)
}

/// The binding reset of the seat's OWN spent scope: the instant its limit
/// lifts, from a held quota reading, never from the notice's zone text.
///
/// The windows are [`own_exhausted`]'s bound ones, and a window binds when a
/// FRESH reading of it is full and no reading of it as new is healthy. The
/// binding instant is the LATEST reset among them, because waking at an
/// earlier one while another still binds would hit the limit again at once.
/// It is short only when every binding window states a length under a day.
///
/// `None` — the reset is unknown — when nothing is read, the scope reports a
/// spend cap (no window reset lifts one), no window binds, or a full bound
/// window states no reset: one unknown binding instant makes THE instant
/// unknown.
pub(crate) fn own_reset(
    quota: Option<&crate::quota::Observation>,
    entry: &crate::meta::RosterEntry,
    pin: Option<&str>,
    now: i64,
) -> Option<super::LimitReset> {
    let tool = crate::quota::recorded_identity(entry)?.tool;
    let scope = own_scope(quota, entry, pin, now)?;
    if scope.spend_capped {
        return None;
    }
    let bound: Vec<_> = scope
        .windows
        .into_iter()
        .filter(|(key, _)| crate::quota::scoped_here(tool, key.qualifier.as_deref(), pin))
        .collect();
    let full = |window: &super::Window| window.judged >= super::FULL;
    if bound
        .iter()
        .any(|(_, window)| full(window) && window.resets_at.is_none())
    {
        return None;
    }
    let answered = |key: &crate::quota::WindowKey, spent: &super::Window| {
        bound.iter().any(|(other, window)| {
            other == key
                && window.status == crate::quota::Status::Fresh
                && !full(window)
                && window.observed_at >= spent.observed_at
        })
    };
    let binding: Vec<&(crate::quota::WindowKey, super::Window)> = bound
        .iter()
        .filter(|(key, window)| {
            window.status == crate::quota::Status::Fresh && full(window) && !answered(key, window)
        })
        .collect();
    let at = binding
        .iter()
        .filter_map(|(_, window)| window.resets_at)
        .max()?;
    let short = binding.iter().all(|(key, _)| {
        key.minutes
            .is_some_and(|minutes| !crate::quota::proven_long(Some(minutes)))
    });
    Some(super::LimitReset { at, short })
}

/// Every reading of the account `origin` names, as a return judges it: each
/// row of each group that names the profile and carries a source, every
/// rollout together, keyed by `(tool, source, window)`. A row scoped to
/// another model family than `pin` is no reading of it; a row ae cannot judge
/// is KEPT, as [`super::Sample::Unjudged`], so it still denies the proof.
pub(crate) fn origin_samples(
    quota: Option<&crate::quota::Observation>,
    origin: &str,
    pin: Option<&str>,
    now: i64,
) -> Vec<(OriginKey, super::Sample)> {
    let groups = quota.map_or(&[][..], |seen| seen.groups.as_slice());
    groups
        .iter()
        .filter(|group| group.profiles.iter().any(|named| named == origin))
        .filter_map(|group| group.source.as_ref().map(|source| (group, source)))
        .flat_map(|(group, source)| {
            group
                .rows
                .iter()
                .filter(|row| {
                    !crate::quota::scoped_elsewhere(group.tool, row.qualifier.as_deref(), pin)
                })
                .map(move |row| {
                    let key = (group.tool, source.clone(), crate::quota::WindowKey::of(row));
                    let sample = window(group, row, now).map_or(
                        super::Sample::Unjudged(row.observed_at),
                        super::Sample::Judged,
                    );
                    (key, sample)
                })
        })
        .collect()
}

/// A window of an origin account: its tool, its source and the window.
pub(crate) type AccountKey = (crate::tool::ToolKind, PathBuf);
/// [`AccountKey`] and the window's identity inside it.
pub(crate) type OriginKey = (crate::tool::ToolKind, PathBuf, crate::quota::WindowKey);

/// What a return may do for one seat now: the seat stands on an automatic
/// chain whose origin is not its current profile and still resolves.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Home {
    pub origin: String,
    /// The chain's newest move: what the lifecycle gate checks it still is.
    pub newest: Timestamp,
    /// The origin account's proof of room: the worst long window, or `None`.
    pub ready: Option<super::Window>,
    /// The seat already went home [`super::MAX_RETURNS`] times in the window.
    pub flapped: bool,
}

/// The return the seat at `row` may make, from the records and one quota
/// read: `None` with no chain, a chain whose origin the seat already serves,
/// or an origin that no longer resolves. An origin account with a usable spend
/// cap, or one another seat of the session is latched on, proves no room.
pub(crate) fn home(
    dir: &Path,
    events: &[crate::events::Event],
    (session, row): (&str, &crate::meta::RosterEntry),
    settings: &Settings,
    (quota, latched): (
        Option<&crate::quota::Observation>,
        &[crate::quota::RecordedIdentity],
    ),
    now: i64,
) -> Option<Home> {
    let chain = super::chain(events, session, &row.slot, &row.name)?;
    if row.profile.as_deref() == Some(chain.origin.as_str()) {
        return None;
    }
    let resolved = crate::reseat::resolved(dir, &chain.origin)?;
    let samples = origin_samples(quota, &chain.origin, resolved.pin.as_deref(), now);
    let accounts: Vec<AccountKey> = samples
        .iter()
        .map(|((tool, source, _), _)| (*tool, source.clone()))
        .collect();
    let band = super::return_band(settings.headroom_at);
    let since = now.saturating_sub(super::RETURN_WINDOW_SECS);
    Some(Home {
        ready: (!origin_blocked(quota, &accounts, latched, now))
            .then(|| super::return_ready(&samples, chain.away_at.epoch(), band))
            .flatten(),
        flapped: super::returns_since(events, session, &row.slot, &row.name, since)
            >= super::MAX_RETURNS,
        origin: chain.origin,
        newest: chain.newest,
    })
}

/// Whether one of the origin's `accounts` proves no room whatever its windows
/// read: a spend cap on it with a usable window, or another seat of the
/// session latched on it. An account is its tool and source together.
fn origin_blocked(
    quota: Option<&crate::quota::Observation>,
    accounts: &[AccountKey],
    latched: &[crate::quota::RecordedIdentity],
    now: i64,
) -> bool {
    let named = |tool, source: &PathBuf| accounts.iter().any(|(t, s)| *t == tool && s == source);
    let capped = quota.is_some_and(|seen| {
        seen.groups.iter().any(|group| {
            group.policy.spend_capped()
                && group
                    .source
                    .as_ref()
                    .is_some_and(|source| named(group.tool, source))
                && group
                    .rows
                    .iter()
                    .filter_map(|row| window(group, row, now))
                    .any(|window| super::usable(&window))
        })
    });
    capped || latched.iter().any(|seat| named(seat.tool, &seat.source))
}

/// What a return checks under the session's lifecycle lock: the seat is still
/// on the chain whose newest move is `newest`, and its row in the `locked`
/// meta still records `profile`, the profile the return was chosen from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Expect {
    pub profile: String,
    pub newest: Timestamp,
}

/// Why a return no longer stands under the lifecycle lock, or `None` when it
/// does: a `reseat` anyone else journaled, a `spawn`, or any other move since
/// the return was chosen ends it, and so does a row that records another
/// profile.
pub(crate) fn return_changed(
    dir: &Path,
    locked: &[u8],
    (session, slot, agent): (&str, &str, &str),
    expect: &Expect,
) -> Option<String> {
    let profile = crate::lifecycle::meta_value(locked, &format!("profile.{slot}"));
    if profile != expect.profile {
        return Some(format!(
            "{agent} records '{profile}' now, not '{}': its return no longer stands",
            expect.profile
        ));
    }
    let events = crate::watchdog_daemon::read_events(dir);
    let chain = super::chain(&events, session, slot, agent);
    (chain.map(|found| found.newest) != Some(expect.newest))
        .then(|| format!("{agent} is no longer on the automatic chain its return was chosen from"))
}

/// Whether a seat its `[auto_reseat]` row names nothing for still has a move:
/// a candidate of its own family, or a held choice the trigger will name.
pub(crate) fn derivable(dir: &Path, bytes: &[u8], row: &crate::meta::RosterEntry) -> bool {
    !listed(dir, bytes, row, &[]).is_empty()
        || crate::run::observed_standing(dir, &row.slot).is_some()
}

/// Everything [`start_read`] asks about one seat.
#[derive(Debug, Clone, Copy)]
pub(crate) struct StartSeat<'a> {
    pub dir: &'a Path,
    pub row: &'a crate::meta::RosterEntry,
    pub seat: Seat<'a>,
    /// The session's [`super::start_fact`].
    pub settled: Option<i64>,
    /// The seat's live pane: a brief is armed only on the pane that would
    /// receive it, and with no pane proven nothing can be ruled out.
    pub pane: Option<&'a str>,
    pub pin: Option<&'a str>,
}

/// Whether a resume's exhausted seat moves now: the ONE reading the daemon's
/// step, the trigger, the leg, effort and the quota notices all judge by.
pub(crate) fn start_read(
    settings: &Settings,
    at: &StartSeat<'_>,
    quota: Option<&crate::quota::Observation>,
    now: i64,
) -> super::StartRead {
    let Some(exhausted) = at
        .settled
        .and_then(|_| own_exhausted(quota, at.row, at.pin, now))
    else {
        return super::StartRead::default();
    };
    let bytes = crate::meta::read_bytes(at.dir).unwrap_or_default();
    super::StartRead {
        settled: at.settled,
        brief_armed: brief_armed(at, &bytes),
        exhausted: reachable(settings, &at.seat, (at.dir, &bytes), at.row).then_some(exhausted),
        quiet: false,
    }
}

/// Whether auto reseat may move this seat at all: eligible, or eligible but for
/// a row that names nothing while its own family does.
pub(crate) fn reachable(
    settings: &Settings,
    seat: &Seat<'_>,
    (dir, bytes): (&Path, &[u8]),
    row: &crate::meta::RosterEntry,
) -> bool {
    match super::eligible(settings, seat) {
        Ok(_) => true,
        Err(Ineligible::Unmapped) => derivable(dir, bytes, row),
        Err(_) => false,
    }
}

/// A brief record that would still deliver to this incarnation on this pane.
/// A move mints a new launch id, which would end it undelivered. An unproven
/// pane denies the waiver rather than assume no record applies.
fn brief_armed(at: &StartSeat<'_>, bytes: &[u8]) -> bool {
    let Some(pane) = at.pane else {
        return true;
    };
    let key = format!("launch_id.{}", at.row.slot);
    let launch_id = crate::meta::sole_value(bytes, &key);
    matches!(
        crate::brief_retry::read(at.dir, &at.row.slot),
        Some(Ok(record))
            if pane == record.pane && launch_id == Some(record.launch_id.as_bytes())
    )
}

/// The quota readings the chooser judges by, read as the session's own
/// `quota` helper reads them. None when the session is not quota-aware, since
/// `quota = off` says ae must not act on vendor numbers, and when the read
/// fails: every candidate is then unknown and taken in listed order.
pub(crate) fn read_quota(
    dir: &Path,
    inputs: &crate::quota::Inputs<'_>,
) -> Option<crate::quota::Observation> {
    crate::session_quota_aware(dir, inputs.global, inputs.local)
        .then(|| crate::quota::observe(inputs).ok())
        .flatten()
}

/// [`read_quota`] for the session at `dir` under the state root `root`.
pub(crate) fn read_quota_at(
    root: &Path,
    dir: &Path,
    meta: &crate::meta::Meta,
    now: Timestamp,
) -> Option<crate::quota::Observation> {
    let global = crate::doors::config_file(crate::shape::current(), root);
    let local = meta
        .origin()
        .and_then(|origin| crate::config::local_overlay(dir, origin));
    let home = crate::doors::home();
    let roots = crate::inventory::Roots::under(root);
    let inputs = crate::quota::Inputs {
        home: home.as_deref(),
        global: Some(&global),
        local: local.as_deref(),
        sessions: Some(roots.sessions()),
        now: now.epoch(),
    };
    read_quota(dir, &inputs)
}

/// How pressed each lead-pair seat of the session at `dir` is, by seat name in
/// roster order: ONE quota read, then each seat's [`own_pressure`], its model
/// pin resolved as a reseat resolves it. `None` per seat when nothing is
/// decided, every seat when the session is not quota-aware; empty when the
/// meta cannot be read.
pub(crate) fn pair_pressure(root: &Path, dir: &Path, now: Timestamp) -> Vec<(String, Option<f64>)> {
    let Ok(bytes) = crate::meta::read_bytes(dir) else {
        return Vec::new();
    };
    let meta = crate::meta::Meta::parse(&String::from_utf8_lossy(&bytes));
    let lead_pair = crate::lifecycle::meta_value(&bytes, "layout") == "lead-pair";
    let observed = read_quota_at(root, dir, &meta, now);
    (meta.roster().iter())
        .filter(|entry| crate::watchdog_daemon::in_lead_pair(&entry.slot, lead_pair))
        .map(|entry| {
            let pin = (entry.profile.as_deref())
                .and_then(|profile| crate::reseat::resolved(dir, profile))
                .and_then(|found| found.pin);
            let used = own_pressure(observed.as_ref(), entry, pin.as_deref(), now.epoch());
            (entry.name.clone(), used)
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
        Skip::NoRoom => "no headroom",
        Skip::LeftOnHeadroom => "left on headroom",
        Skip::PairProvider => "partner's provider",
        Skip::PairUnproven => "provider unproven",
        Skip::LeadSibling => "a lead seat never changes model unasked",
    }
}

/// `text` as one control-free line of at most [`SUMMARY_CHARS`], or through a named
/// `ae reseat` command kept WHOLE up to `REASON_MAX`, else dropped whole.
fn bounded(text: &str) -> String {
    let clean = crate::seatpack::neutralise(text);
    let keep = match (clean.find("ae reseat "), clean.find(STOP_UNKNOWN)) {
        (Some(start), Some(at)) => {
            let whole = clean[..at + STOP_UNKNOWN.len()].chars().count();
            if whole <= crate::state::REASON_MAX {
                whole.max(SUMMARY_CHARS)
            } else {
                clean[..start].chars().count().min(SUMMARY_CHARS)
            }
        }
        _ => SUMMARY_CHARS,
    };
    crate::state::summary_of(&clean)
        .chars()
        .take(keep)
        .collect()
}

const STOP_UNKNOWN: &str = " --stop-unknown";

/// [`start_read`] with every input read now: config, roster row, pin, journal
/// of the brief, quota. `None` when the slot no longer holds `incarnation`,
/// the agent, profile and launch the attempt was opened for: a seat that has
/// changed under the leg answers for nothing.
fn read_start(
    root: &Path,
    dir: &Path,
    argv: &Argv,
    (agent, profile, launch): (&str, &str, Option<&str>),
    at: Timestamp,
) -> Option<super::StartRead> {
    let bytes = crate::meta::read_bytes(dir).unwrap_or_default();
    let meta = crate::meta::Meta::parse(&String::from_utf8_lossy(&bytes));
    let row = meta.roster().iter().find(|row| row.slot == argv.slot)?;
    let same = row.name == agent
        && row.profile.as_deref().unwrap_or_default() == profile
        && meta.launch_id(&argv.slot) == launch;
    if !same {
        return None;
    }
    let settings = super::settings(Some(&crate::doors::config_file(
        crate::shape::current(),
        root,
    )));
    let live = crate::tracked::resolve_on(agent, &argv.session, dir).ok();
    let pin = crate::reseat::resolved(dir, profile).and_then(|found| found.pin);
    let seat = Seat {
        session: &argv.session,
        slot: &argv.slot,
        agent,
        profile,
        orchestrator: crate::meta::meta_agent_role(&bytes) == crate::meta::MetaAgentRole::Role,
    };
    Some(start_read(
        &settings,
        &StartSeat {
            dir,
            row,
            seat,
            settled: super::start_fact(
                meta.started_epoch(),
                meta.launch_settled_epoch(),
                at.epoch(),
            ),
            pane: live.as_ref().map(|(found, _)| found.pane.as_str()),
            pin: pin.as_deref(),
        },
        read_quota_at(root, dir, &meta, at).as_ref(),
        at.epoch(),
    ))
}

/// `_auto-reseat <dir> <slot> <key>`: move the seat at `slot` off its usage
/// limit as the watchdog, and journal ONE outcome for the attempt under `key`.
///
/// The order is the contract. The argv first, reading nothing. Then the seat
/// and its episode: a seat gone or an episode that is not `key`'s REFUSES, so
/// the attempt it was handed closes; an attempt already closed, or none open,
/// is not the leg's to act on and it journals nothing. Then the global config,
/// whose switch HOLDS; a headroom episode, unless a start made it urgent, HOLDS
/// too once its own read finds the seat's account no longer pressed
/// ([`source_pressed`]). Then the chooser, which refuses when nothing is
/// usable. Only then the move, whose ending is the outcome.
///
/// # Errors
///
/// Only a failure to write `err`.
#[allow(
    clippy::too_many_lines,
    reason = "the order is the contract, kept in one place"
)]
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
    // THE SEAT'S LOCK, without waiting and before anything is read: the leg is
    // a writer of this path's records like every other. The trigger released
    // it before starting this leg, so a lock held now is another writer's, and
    // this leg writes nothing.
    let Ok(_held) = crate::store::lock(&super::lock_path(dir, &argv.slot), Duration::ZERO) else {
        writeln!(
            err,
            "ae: auto reseat: another writer holds {} — nothing written.",
            argv.slot
        )?;
        return Ok(EXIT_FAILED);
    };
    let bytes = crate::meta::read_bytes(dir).unwrap_or_default();
    let meta = crate::meta::Meta::parse(&String::from_utf8_lossy(&bytes));
    let Some(seat) = meta.roster().iter().find(|row| row.slot == argv.slot) else {
        let why = CauseClass::Other.lead(&format!(
            "refused: {} seats no agent at slot {}",
            argv.session, argv.slot
        ));
        return close(&argv, "", now, (REFUSED_ACTION, &why), err);
    };
    let (agent, profile) = (seat.name.clone(), seat.profile.clone().unwrap_or_default());
    let events = crate::watchdog_daemon::read_events(dir);
    let Some(found) = super::keyed(&events, (&argv.session, &argv.slot, &agent), argv.key) else {
        let why = CauseClass::Other.lead(&format!(
            "refused: the seat is in no limit episode keyed {key}"
        ));
        return close(&argv, &agent, now, (REFUSED_ACTION, &why), err);
    };
    let cause = found.cause().to_owned();
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
    let global = super::settings(Some(&crate::doors::config_file(
        crate::shape::current(),
        root,
    )));
    let lead_pair = crate::lifecycle::meta_value(&bytes, "layout") == "lead-pair";
    // The seat's own threshold: the lead pair reads its own row when it is set.
    let settings = global.seat_view(lead_pair, &argv.slot);
    let orchestrator = crate::meta::meta_agent_role(&bytes) == crate::meta::MetaAgentRole::Role;
    let asked = Seat {
        session: &argv.session,
        slot: &argv.slot,
        agent: &agent,
        profile: &profile,
        orchestrator,
    };
    // Every ending from here on is under the attempt the watchdog opened, so
    // each one is also said and told.
    let told = (
        meta.roster(),
        lead_pair,
        super::spawner(&events, &argv.session, &argv.slot, &agent),
    );
    let notice = |ending| super::notice(&argv.session, &agent, &ending);
    // An observed-choice hold is told once per key, however often it is tried.
    let retold = |class| class == CauseClass::ObservedChoice && found.choice_held;
    let threshold = settings
        .headroom_at
        .filter(|_| found.trigger == Trigger::Headroom);
    let quota = read_quota_at(root, dir, &meta, now);
    // No lock spans the trigger and this leg, so the proof is judged again here.
    let incarnation = (agent.as_str(), profile.as_str(), meta.launch_id(&argv.slot));
    let start = read_start(root, dir, &argv, incarnation, now).unwrap_or_default();
    let grace = i64::try_from(settings.grace_secs).unwrap_or(i64::MAX);
    let early = now.epoch() < found.key.epoch().saturating_add(grace);
    // A return re-reads its chain and its origin's room under the seat's lock,
    // and its one candidate is the origin (#305).
    let homeward = found.trigger == Trigger::Return;
    let latched = super::latched_identities(meta.roster(), &events, &argv.session, &argv.slot);
    let home_now = homeward
        .then(|| {
            let read = (quota.as_ref(), latched.as_slice());
            home(
                dir,
                &events,
                (&argv.session, seat),
                &settings,
                read,
                now.epoch(),
            )
        })
        .flatten();
    let eligible = match super::eligible(&settings, &asked) {
        Err(why) if why != Ineligible::Unmapped => Err((why.class(), ineligible_record(why))),
        _ if homeward => match &home_now {
            None => Err((CauseClass::Other, CauseClass::Other.lead(RETURN_OFF_CHAIN))),
            Some(_) if early => Err((CauseClass::Other, CauseClass::Other.lead(RETURN_EARLY))),
            Some(back) if back.ready.is_none() => {
                Err((CauseClass::Other, CauseClass::Other.lead(RETURN_GONE)))
            }
            Some(back) => Ok(vec![(back.origin.clone(), Provenance::Declared)]),
        },
        declared => {
            let list = listed(dir, &bytes, seat, declared.unwrap_or_default());
            if list.is_empty() {
                Err(observed_ending(dir, &argv.slot).map_or_else(
                    || {
                        let why = ineligible_record(Ineligible::Unmapped);
                        (CauseClass::NoCandidate, why)
                    },
                    |why| (CauseClass::ObservedChoice, why),
                ))
            } else if found.trigger == Trigger::Headroom
                && threshold.is_none()
                && !start.urgent(found.key.epoch())
            {
                Err((
                    CauseClass::Ineligible,
                    CauseClass::Ineligible.lead(&headroom_off(settings.headroom_key())),
                ))
            } else if early && !start.urgent(found.key.epoch()) {
                Err((CauseClass::Other, CauseClass::Other.lead(START_GONE)))
            } else if let Some(at) = threshold.filter(|_| !start.urgent(found.key.epoch()))
                && !source_pressed(quota.as_ref(), dir, seat, &profile, at, now)
            {
                Err((CauseClass::Other, CauseClass::Other.lead(HEADROOM_GONE)))
            } else {
                Ok(list)
            }
        }
    };
    let list = match eligible {
        Ok(list) => list,
        Err((class, summary)) => {
            let summary = tagged(&summary, &cause);
            let code = close(&argv, &agent, now, (HELD_ACTION, &summary), err)?;
            if !retold(class) {
                tell(
                    &argv,
                    now,
                    told,
                    &notice(super::Ending::Held(class, &summary)),
                );
            }
            return Ok(code);
        }
    };
    let left = super::left_profiles(&events, &argv.session, &argv.slot, &agent);
    let left_on_headroom = super::left_on_headroom(&events, &argv.session, &argv.slot, &agent);
    let judged = candidates(
        &list,
        |to| crate::reseat::resolved(dir, to),
        (&left, &left_on_headroom),
        quota.as_ref(),
        &latched,
        now.epoch(),
    );
    // The pair as this leg reads it, under the seat's lock (#305).
    let pair = pair_of(dir, &bytes, meta.roster(), told.1, &argv.slot);
    let bar = if homeward {
        settings.headroom_at
    } else {
        threshold
    };
    let choice = super::choose_paired(&judged, now.epoch(), bar, pair.as_ref());
    let named = |pick: &str| format!("{pick}{}", choice.provenance.word());
    let Some((pick, tier)) = choice.pick else {
        if let Some(why) = observed_ending(dir, &argv.slot).map(|why| tagged(&why, &cause)) {
            let code = close(&argv, &agent, now, (HELD_ACTION, &why), err)?;
            if !found.choice_held {
                let ending = super::Ending::Held(CauseClass::ObservedChoice, &why);
                tell(&argv, now, told, &notice(ending));
            }
            return Ok(code);
        }
        let (class, why) = refusal(&choice, pair.as_ref());
        let why = tagged(&why, &cause);
        let seat_of = (argv.session.as_str(), argv.slot.as_str(), agent.as_str());
        let retell = !pair_told(&events, seat_of, class, &key, &why);
        let code = close(&argv, &agent, now, (REFUSED_ACTION, &why), err)?;
        if retell {
            tell(
                &argv,
                now,
                told,
                &notice(super::Ending::Refused(class, &why)),
            );
        }
        return Ok(code);
    };
    // A test seam: the choice is made and the lifecycle lock not yet taken.
    crate::read_gate("@auto-reseat-pair");
    // What the move changes, as the notice names it: the tool and model before
    // it, read now, and the model the target pins.
    let pin = |profile: &str| crate::reseat::resolved(dir, profile).and_then(|found| found.pin);
    let unknown = || UNKNOWN.to_owned();
    let tool_before = seat.binary.clone().unwrap_or_else(unknown);
    let model_before = meta
        .observed_model(&argv.slot)
        .map(ToOwned::to_owned)
        .or_else(|| pin(&profile))
        .unwrap_or_else(unknown);
    let model_after = pin(&pick).unwrap_or_else(unknown);
    let (_, world) = crate::fleet_read::current_world(root);
    let mut said = Vec::new();
    // The stop's last word on an urgent seat, under its locks: everything that
    // made it urgent, read once more.
    let mut still_urgent = || {
        read_start(root, dir, &argv, incarnation, Timestamp::now())
            .is_some_and(|read| read.urgent(found.key.epoch()))
    };
    // A return never takes a resume's quiet waiver.
    let urgent = !homeward && start.urgent(found.key.epoch());
    // Only a limit episode's move may end the seat's background tasks (#304).
    let long_limit = (found.trigger == Trigger::Limit).then(|| crate::reseat::LongLimit {
        key: found.key.epoch(),
        opened: found.open.map_or(now.epoch(), Timestamp::epoch),
    });
    // What a return checks again under the lifecycle lock (#305).
    let expect = home_now.as_ref().map(|back| Expect {
        profile: profile.clone(),
        newest: back.newest,
    });
    let (ended, tasks) = crate::reseat::run_as_watchdog(
        root,
        Some(&world),
        &argv.session,
        &agent,
        &pick,
        now,
        urgent.then_some(&mut still_urgent as &mut dyn FnMut() -> bool),
        long_limit,
        expect,
        &mut io::sink(),
        &mut said,
    )?;
    err.write_all(&said)?;
    let (action, reference, summary) = outcome(
        ended,
        &key,
        &profile,
        &named(&pick),
        &String::from_utf8_lossy(&said),
    );
    // After a move its pair is read once more, BEFORE the move's record, and
    // what the record and notice say of it is judged on that read (#305 R8).
    let moved = matches!(ended, Ended::Moved { .. });
    if moved {
        // A test seam: the move has returned and its pair is not yet read again.
        crate::read_gate("@auto-reseat-after");
    }
    let after = moved.then(|| pair_now(dir, &argv.slot)).flatten();
    let fallback = pair.as_ref().and_then(|pair| pair.stays(&pick));
    let (stays, collapsed) = match super::after_move(fallback, after.as_ref()) {
        super::AfterMove::Stays(provider) => (Some(provider), None),
        super::AfterMove::Collapsed(pair) => (None, Some(pair)),
        super::AfterMove::Quiet => (None, None),
    };
    // A pair that already served one proven provider and still does is named
    // so, ahead of the rest, by the move's record and notice alike (R7).
    let summary = match stays {
        Some(provider) => format!("{}: {summary}", super::stays_on(provider)),
        None => summary,
    };
    let summary = tagged(&summary, &cause);
    record(&argv, &agent, now, action, &reference, &summary);
    let text = match ended {
        Ended::Moved { fate } => {
            let after = crate::meta::read_bytes(dir).unwrap_or_default();
            let after = crate::meta::Meta::parse(&String::from_utf8_lossy(&after));
            let tool_after = after
                .roster()
                .iter()
                .find(|row| row.slot == argv.slot)
                .and_then(|row| row.binary.clone())
                .unwrap_or_else(unknown);
            let dirty = crate::meta::resolve_seat_dir(&after, &argv.slot)
                .is_ok_and(|wdir| crate::git::work_tree_dirty(wdir.as_bytes()));
            // AFTER the move's own record: the episode is closed before the
            // seat is handed anything, so nothing retries or repeats the turn.
            let nudge = (fate != super::Fate::Seeded).then(|| nudge(&argv, &agent, fate));
            let to = named(&pick);
            let moved = super::Move {
                from: &profile,
                to: &to,
                tool: (&tool_before, &tool_after),
                model: (&model_before, &model_after),
                fate,
                nudge,
                critical: tier == super::Tier::Critical,
                dirty,
                cause: &cause,
            };
            notice(match stays {
                Some(provider) => super::Ending::PairStays(provider, moved, &tasks),
                None if tasks.is_empty() => super::Ending::Moved(moved),
                None => super::Ending::MovedEndingTasks(moved, &tasks),
            })
        }
        Ended::Refused(class) if retold(class) => String::new(),
        Ended::Refused(class) if class.transient() => notice(super::Ending::Held(class, &summary)),
        Ended::Refused(class) => notice(super::Ending::Refused(class, &summary)),
        Ended::Failed => notice(super::Ending::Failed(CauseClass::Other, &summary)),
    };
    if !text.is_empty() {
        tell(&argv, now, told, &text);
    }
    if let Some(pair) = collapsed {
        let ending = super::Ending::PairCollapse(pair);
        tell(
            &argv,
            now,
            told,
            &super::notice(&argv.session, &agent, &ending),
        );
    }
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
    /// The seat's own account, read now, is at or within the re-arm gap of the
    /// headroom threshold.
    pub near: bool,
}

/// What the trigger does for one seat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Plan {
    /// Nothing, and nothing is written: why, for the trigger's error stream.
    Decline(String),
    /// Close the episode keyed `key`: nothing listed is usable, for the
    /// reason `class` names.
    Refuse {
        key: Timestamp,
        class: CauseClass,
        why: String,
    },
    /// Open an attempt under `key` and start the leg that moves the seat to
    /// `to`; `note` follows `from <profile> to <to>` in the attempt's summary —
    /// the pick's provenance first — and `cause` names a headroom trigger
    /// (empty for a limit).
    Attempt {
        key: Timestamp,
        to: String,
        note: String,
        cause: String,
    },
}

/// Decide the trigger from the records and one fresh sight, by the daemon's
/// own rule: the seat is eligible, and either the limit row is drawn and its
/// limit episode decides [`super::Decision::Attempt`] NOW, or — with no limit
/// drawn — its headroom episode does, while its own account still reads within
/// [`super::REARM_GAP`] of the threshold. `gather` is handed the seat's
/// declared row — empty when it names none — and judges its whole list; a seat
/// it lists nothing for declines as unmapped, writing nothing.
#[cfg(test)]
pub(crate) fn plan(
    settings: &super::Settings,
    seat: &Seat<'_>,
    sight: &Sight,
    events: &[crate::events::Event],
    gather: impl Fn(&[String], (&[(String, Timestamp)], &[String])) -> Vec<Candidate>,
    now: Timestamp,
) -> Plan {
    let plain = super::StartRead::default();
    plan_with(settings, seat, sight, &plain, events, gather, now)
}

/// [`plan`] with `start`, the trigger's own fresh read: an episode it finds
/// urgent is due at once, whatever the threshold says.
#[cfg(test)]
pub(crate) fn plan_with(
    settings: &super::Settings,
    seat: &Seat<'_>,
    sight: &Sight,
    start: &super::StartRead,
    events: &[crate::events::Event],
    gather: impl Fn(&[String], (&[(String, Timestamp)], &[String])) -> Vec<Candidate>,
    now: Timestamp,
) -> Plan {
    plan_paired(settings, seat, sight, start, events, gather, None, now)
}

/// [`plan_with`] for a seat with a pair partner: the chooser passes over every
/// candidate `pair` judges collapsing (#305).
#[allow(
    clippy::too_many_arguments,
    reason = "the trigger's own reads, each handed over once"
)]
pub(crate) fn plan_paired(
    settings: &super::Settings,
    seat: &Seat<'_>,
    sight: &Sight,
    start: &super::StartRead,
    events: &[crate::events::Event],
    gather: impl Fn(&[String], (&[(String, Timestamp)], &[String])) -> Vec<Candidate>,
    pair: Option<&super::PairGuard>,
    now: Timestamp,
) -> Plan {
    let list = match super::eligible(settings, seat) {
        Ok(list) => list,
        Err(Ineligible::Unmapped) => &[],
        Err(why) => return Plan::Decline(format!("not eligible ({why:?})")),
    };
    let limit = super::episode(events, seat.session, seat.slot, seat.agent);
    let headroom = super::headroom_episode(events, seat.session, seat.slot, seat.agent);
    let back = super::return_episode(events, seat.session, seat.slot, seat.agent);
    if super::flying_any(&[limit.as_ref(), headroom.as_ref(), back.as_ref()]).is_some() {
        return Plan::Decline("an attempt is in flight".to_owned());
    }
    let room = settings.headroom_at.filter(|_| !sight.limited);
    let keyed = |found: &super::Episode| start.urgent(found.key.epoch());
    let found = match room {
        _ if !sight.limited && headroom.as_ref().is_some_and(keyed) => headroom,
        None if !sight.limited => return Plan::Decline("no usage limit is drawn".to_owned()),
        None => limit,
        Some(_) if !sight.near => {
            return Plan::Decline(
                "no usage limit is drawn and the account has headroom".to_owned(),
            );
        }
        Some(_) => headroom,
    };
    let key = found
        .as_ref()
        .map_or(now.epoch(), |found| found.key.epoch());
    let waits = start.waits(key, settings.grace_secs);
    let mut pane = sight.pane;
    pane.frame = start.frame(key, pane.frame);
    let decision = super::decide_with_waits(found.as_ref(), waits, &pane, now.epoch());
    let (Some(found), super::Decision::Attempt) = (found, decision) else {
        return Plan::Decline(format!("not due ({decision:?})"));
    };
    let left = super::left_profiles(events, seat.session, seat.slot, seat.agent);
    let left_on_headroom = super::left_on_headroom(events, seat.session, seat.slot, seat.agent);
    let judged = gather(list, (&left, &left_on_headroom));
    if judged.is_empty() {
        let why = Ineligible::Unmapped;
        return Plan::Decline(format!("not eligible ({why:?})"));
    }
    let choice = super::choose_paired(&judged, now.epoch(), room, pair);
    let cause = found.cause();
    let Some((to, _)) = &choice.pick else {
        let (class, why) = refusal(&choice, pair);
        return Plan::Refuse {
            key: found.key,
            class,
            why: tagged(&why, cause),
        };
    };
    Plan::Attempt {
        key: found.key,
        to: to.clone(),
        note: format!(
            "{}{}",
            choice.provenance.word(),
            attempt_note(cause, &choice.skipped)
        ),
        cause: cause.to_owned(),
    }
}

/// Decide the trigger for a seat whose return episode `open` is the one in
/// play, by the daemon's own rule: the seat is eligible (a row naming nothing
/// is no bar: the origin is its candidate), its origin proves room on the
/// trigger's own read (`home`), and the episode is due by the plain grace and
/// an idle frame. The ONE candidate is the origin, judged by `judge` and the
/// same chooser and pair guard every move takes (#305).
pub(crate) fn plan_return(
    settings: &Settings,
    (seat, sight): (&Seat<'_>, &Sight),
    open: &super::Episode,
    home: Option<&Home>,
    judge: impl FnOnce(&str) -> Vec<Candidate>,
    pair: Option<&super::PairGuard>,
    now: Timestamp,
) -> Plan {
    match super::eligible(settings, seat) {
        Ok(_) | Err(Ineligible::Unmapped) => {}
        Err(why) => return Plan::Decline(format!("not eligible ({why:?})")),
    }
    let Some(home) = home.filter(|found| found.ready.is_some()) else {
        return Plan::Decline("the origin account does not prove room".to_owned());
    };
    let decision = super::decide(Some(open), settings.grace_secs, &sight.pane, now.epoch());
    if decision != super::Decision::Attempt {
        return Plan::Decline(format!("not due ({decision:?})"));
    }
    let choice = super::choose_paired(
        &judge(&home.origin),
        now.epoch(),
        settings.headroom_at,
        pair,
    );
    let cause = open.cause();
    let Some((to, _)) = &choice.pick else {
        let (class, why) = refusal(&choice, pair);
        return Plan::Refuse {
            key: open.key,
            class,
            why: tagged(&why, cause),
        };
    };
    Plan::Attempt {
        key: open.key,
        to: to.clone(),
        note: format!(
            "{}{}",
            choice.provenance.word(),
            attempt_note(cause, &choice.skipped)
        ),
        cause: cause.to_owned(),
    }
}

/// What a headroom attempt's summary adds after `from <a> to <b>`: its trigger,
/// then each candidate passed over and why. A limit attempt adds nothing.
fn attempt_note(cause: &str, skipped: &[(String, Skip)]) -> String {
    if cause.is_empty() {
        return String::new();
    }
    let named: Vec<String> = skipped
        .iter()
        .map(|(to, skip)| format!("{to} ({})", skip_word(*skip)))
        .collect();
    if named.is_empty() {
        format!(" ({cause})")
    } else {
        format!(" ({cause}); passed over {}", named.join(", "))
    }
}

/// The refusal that names each candidate passed over, and why.
fn no_candidate(skipped: &[(String, Skip)]) -> String {
    CauseClass::NoCandidate.lead(&format!("refused: {}", unusable(skipped)))
}

/// The refusal of a choice with no pick, and its class: `pair-provider` when
/// the pair guard passed over a candidate (#305), naming what the seat and
/// its partner serve; otherwise the [`no_candidate`] refusal, byte for byte.
fn refusal(choice: &super::Choice, pair: Option<&super::PairGuard>) -> (CauseClass, String) {
    let guarded = choice
        .skipped
        .iter()
        .any(|(_, skip)| matches!(skip, Skip::PairProvider | Skip::PairUnproven));
    match pair.filter(|_| guarded) {
        Some(pair) => (
            CauseClass::PairProvider,
            CauseClass::PairProvider.lead(&format!(
                "refused: {}: {}",
                pair.words(),
                unusable(&choice.skipped)
            )),
        ),
        None => (CauseClass::NoCandidate, no_candidate(&choice.skipped)),
    }
}

/// Whether the watchdog already journaled the pair refusal `why` for this
/// `(session, slot, agent)` seat under the episode `key`: a later round that
/// refuses for the very same reason is recorded, not told again (#305). `why`
/// is compared as [`record`] keeps it, [`bounded`].
fn pair_told(
    events: &[crate::events::Event],
    (session, slot, agent): (&str, &str, &str),
    class: CauseClass,
    key: &str,
    why: &str,
) -> bool {
    let kept = bounded(why);
    class == CauseClass::PairProvider
        && events.iter().any(|event| {
            event.actor == WATCHDOG_ACTOR
                && event.action == REFUSED_ACTION
                && event.reference.as_deref() == Some(key)
                && event.summary.as_deref() == Some(kept.as_str())
                && crate::watchdog::event_is_addressed_to(event, session, slot, agent)
        })
}

fn unusable(skipped: &[(String, Skip)]) -> String {
    let named: Vec<String> = skipped
        .iter()
        .map(|(to, skip)| format!("{to} ({})", skip_word(*skip)))
        .collect();
    format!("no usable candidate: {}", named.join(", "))
}

/// The line a seat's first-sight limit notice gains when auto reseat may move
/// it: where to and when, or that nothing listed is usable. A forecast: the
/// legs choose again when they act.
pub(crate) fn deadline(agent: &str, choice: &super::Choice, grace_secs: u64) -> String {
    let Some((to, tier)) = &choice.pick else {
        return format!("ae cannot move {agent}: {}", unusable(&choice.skipped));
    };
    let grace = crate::brief::age(Some(i64::try_from(grace_secs).unwrap_or(i64::MAX)));
    let critical = if *tier == super::Tier::Critical {
        ", target already critical"
    } else {
        ""
    };
    let named = choice.provenance.word();
    format!("ae will move {agent} to {to}{named} in {grace}{critical}")
}

/// The environment of every notice delivery, whole. The leg runs under the
/// trigger's own environment, whose action is the attempt: a notice that did
/// not name its own would be taken for another trigger.
pub(crate) fn notice_env(summary: &str) -> [(&'static str, &str); 3] {
    [
        ("AE_SENDER_OVERRIDE", WATCHDOG_ACTOR),
        ("_AE_EVENT_ACTION", super::NOTICE_ACTION),
        ("_AE_EVENT_SUMMARY", summary),
    ]
}

/// Say how the attempt under `argv` ended on the human's chat: ONE target-less
/// line, written as the watchdog.
fn said(argv: &Argv, now: Timestamp, text: &str) {
    let _ = crate::store::open(&argv.dir).append_event(&crate::chat_line(
        now,
        WATCHDOG_ACTOR,
        "",
        text,
    ));
}

/// The ONE turn a move whose conversation continued hands its seat: no seed
/// pack does, and without a turn the seat sits idle in it. It opens with what
/// moved ([`continuation`]).
const CONTINUE: &str = "and your conversation came with you. Restore your state first: re-declare \
     it, then check your pending requests (requests inbox, requests mine) and the session memo and \
     brief (memo read, ae brief). Resume only authorized outstanding work. Keep any genuine wait on \
     a human or an agent, and any parked scope, exactly as it was, and send no request again.";

/// The continuation turn for `fate`, and the summary its record carries, which
/// names it among the notices.
fn continuation(fate: super::Fate) -> (String, String) {
    let word = fate.word();
    let moved = match fate {
        super::Fate::Kept => "another model or effort",
        _ => "another account",
    };
    (
        format!("auto reseat moved you to {moved} of the same tool, {CONTINUE}"),
        format!("continuation turn after an automatic {word} reseat"),
    )
}

/// Hand `agent` its [`continuation`] turn through the session's own `send`
/// helper, and read back how it went: submitted only on the helper's exit 0 AND
/// its own unmarked record past the journal as it read before the send.
fn nudge(argv: &Argv, agent: &str, fate: super::Fate) -> super::Nudge {
    let (text, summary) = continuation(fate);
    let before = crate::watchdog_daemon::read_events(&argv.dir).len();
    let send = argv.dir.join("send");
    let env = notice_env(&summary);
    if crate::transport::deliver(&send, agent, &text, false, &env).code != Some(0) {
        return super::Nudge::Refused;
    }
    let events = crate::watchdog_daemon::read_events(&argv.dir);
    let submitted = events
        .get(before..)
        .unwrap_or_default()
        .iter()
        .any(|event| {
            event.actor == WATCHDOG_ACTOR
                && event.action == super::NOTICE_ACTION
                && event.target.as_deref() == Some(agent)
                && event.summary.as_deref() == Some(summary.as_str())
        });
    if submitted {
        super::Nudge::Submitted
    } else {
        super::Nudge::Unconfirmed
    }
}

/// Who is told: the roster, whether it is a lead pair, and the seat's spawner.
type Told<'a> = (&'a [crate::meta::RosterEntry], bool, Option<&'a str>);

/// Tell how the attempt under `argv` ended: the chat line, then the lead pair
/// and the seat's spawner, each through the session's own send helper. A
/// recipient that is busy or gone is the helper's to refuse; the outcome is
/// already durable.
fn tell(argv: &Argv, now: Timestamp, told: Told<'_>, text: &str) {
    let (roster, lead_pair, spawner) = told;
    said(argv, now, text);
    let send = argv.dir.join("send");
    for recipient in super::recipients(roster, lead_pair, &argv.slot, spawner) {
        let _ = crate::transport::deliver(&send, &recipient, text, false, &notice_env(text));
    }
}

/// Tell, as an ending is told, that the seat at `slot` sits on its limit with no
/// candidate at all: no attempt, so no leg, ever opens to say it.
pub(crate) fn tell_unmapped(
    dir: &Path,
    seat: (&str, &str, &str),
    profile: &str,
    key: Timestamp,
    now: Timestamp,
) {
    tell_ending(dir, seat, key, now, &super::Ending::Unmapped(profile));
}

/// Tell the lead pair and the spawner of a hold the daemon journaled under
/// `key`, `summary` led by its class.
pub(crate) fn tell_held(
    dir: &Path,
    seat: (&str, &str, &str),
    key: Timestamp,
    now: Timestamp,
    summary: &str,
) {
    let ending = super::Ending::Held(CauseClass::Other, summary);
    tell_ending(dir, seat, key, now, &ending);
}

/// Tell, as an ending is told, that the last round of the seat's limit episode
/// was refused or failed: no attempt follows, so no leg ever opens to say it.
pub(crate) fn tell_gave_up(dir: &Path, seat: (&str, &str, &str), key: Timestamp, now: Timestamp) {
    tell_ending(dir, seat, key, now, &super::Ending::GaveUp);
}

fn tell_ending(
    dir: &Path,
    (session, slot, agent): (&str, &str, &str),
    key: Timestamp,
    now: Timestamp,
    ending: &super::Ending<'_>,
) {
    let bytes = crate::meta::read_bytes(dir).unwrap_or_default();
    let meta = crate::meta::Meta::parse(&String::from_utf8_lossy(&bytes));
    let events = crate::watchdog_daemon::read_events(dir);
    let argv = Argv {
        session: session.to_owned(),
        dir: dir.to_path_buf(),
        slot: slot.to_owned(),
        key,
    };
    let told = (
        meta.roster(),
        crate::lifecycle::meta_value(&bytes, "layout") == "lead-pair",
        super::spawner(&events, session, slot, agent),
    );
    let text = super::notice(session, agent, ending);
    tell(&argv, now, told, &text);
}

/// Journal the attempt, release the seat's lock `held`, THEN start the leg:
/// the leg acts only under an attempt it can read, and takes that lock itself
/// without waiting. A leg that could not start closes the attempt at once,
/// under the lock taken again, while the journal still shows it open.
#[allow(
    clippy::too_many_arguments,
    reason = "the handoff's facts, each read once by the trigger"
)]
fn commit(
    argv: &Argv,
    agent: &str,
    (from, to): (&str, &str),
    (note, cause): (&str, &str),
    now: Timestamp,
    held: std::fs::File,
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
        &format!("from {from} to {to}{note}"),
    );
    drop(held);
    if spawn() {
        return Ok(0);
    }
    let lock = super::lock_path(&argv.dir, &argv.slot);
    let Ok(_held) = crate::store::lock(&lock, Duration::ZERO) else {
        writeln!(
            err,
            "ae: auto reseat: the move could not be started and another writer holds {}",
            argv.slot
        )?;
        return Ok(EXIT_FAILED);
    };
    let events = crate::watchdog_daemon::read_events(&argv.dir);
    let seat = (argv.session.as_str(), argv.slot.as_str(), agent);
    let open = super::keyed(&events, seat, argv.key)
        .is_some_and(|found| found.open.is_some() && found.terminal.is_none());
    if !open {
        writeln!(err, "ae: auto reseat: the move could not be started")?;
        return Ok(EXIT_FAILED);
    }
    let why = tagged(
        &CauseClass::Other.lead("failed: the move could not be started"),
        cause,
    );
    let code = close(argv, agent, now, (FAILED_ACTION, &why), err)?;
    said(
        argv,
        now,
        &super::notice(
            &argv.session,
            agent,
            &super::Ending::Failed(CauseClass::Other, &why),
        ),
    );
    Ok(code)
}

/// `send` under the attempt action: the daemon's trigger. It re-derives the
/// whole decision under the seat's lock, so a forged call does exactly what the
/// daemon would do now; anything else declines with one line and writes
/// nothing.
///
/// # Errors
///
/// Only a failure to write `err`.
#[allow(
    clippy::too_many_lines,
    reason = "the trigger's reads and its one decision are kept in one place, as the leg's"
)]
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
    let Ok(held) = crate::store::lock(&super::lock_path(dir, slot), Duration::ZERO) else {
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
    let config =
        crate::state_root().map(|root| crate::doors::config_file(crate::shape::current(), &root));
    let global = super::settings(config.as_deref());
    let lead_pair = crate::lifecycle::meta_value(&bytes, "layout") == "lead-pair";
    // The seat's own threshold: the lead pair reads its own row when it is set.
    let settings = global.seat_view(lead_pair, slot);
    let profile = row.profile.clone().unwrap_or_default();
    // ONE quota read, and only once a decision needs it.
    let quota = std::cell::OnceCell::new();
    let observed = || {
        quota
            .get_or_init(|| {
                crate::state_root().and_then(|root| read_quota_at(&root, dir, &meta, now))
            })
            .as_ref()
    };
    let limited = crate::watchdog::limit_notice(text, &bin).is_some();
    let near = settings
        .headroom_at
        .filter(|_| !limited)
        .is_some_and(|at| source_pressed(observed(), dir, row, &profile, at, now));
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
        limited,
        near,
    };
    let seat = Seat {
        session: own_session,
        slot,
        agent: &row.name,
        profile: &profile,
        orchestrator: crate::meta::meta_agent_role(&bytes) == crate::meta::MetaAgentRole::Role,
    };
    let pin = crate::reseat::resolved(dir, &profile).and_then(|found| found.pin);
    let mut start = start_read(
        &settings,
        &StartSeat {
            dir,
            row,
            seat,
            settled: super::start_fact(
                meta.started_epoch(),
                meta.launch_settled_epoch(),
                now.epoch(),
            ),
            pane: Some(&resolved.pane),
            pin: pin.as_deref(),
        },
        observed(),
        now.epoch(),
    );
    // Its own pair, owing nothing to the daemon's memory of the last cycle.
    if start.exhausted.is_some() && sight.pane.frame == super::Frame::Unproven {
        std::thread::sleep(crate::deliver::FRAME_GAP);
        let again = crate::transport::capture_pane(&server, &resolved.pane);
        start.quiet = super::quiet_pair(capture.as_deref(), again.as_deref(), &bin);
    }
    let events = crate::watchdog_daemon::read_events(dir);
    // A seat whose pair partner is mid-move waits for that move: two halves
    // never choose against each other's row at once (#305).
    if let super::Link::Partner(other) = super::partner(meta.roster(), lead_pair, slot) {
        let (at, name) = (other.slot.as_str(), other.name.as_str());
        let limit = super::episode(&events, own_session, at, name);
        let headroom = super::headroom_episode(&events, own_session, at, name);
        let back = super::return_episode(&events, own_session, at, name);
        if super::flying_any(&[limit.as_ref(), headroom.as_ref(), back.as_ref()]).is_some() {
            return decline(err, "its pair partner's move is in flight");
        }
    }
    let pair = pair_of(dir, &bytes, meta.roster(), lead_pair, slot);
    let unlisted = std::cell::Cell::new(false);
    // Read only once the plan reaches the chooser.
    let gather = |list: &[String], left: (&[(String, Timestamp)], &[String])| {
        let latched = super::latched_identities(meta.roster(), &events, own_session, slot);
        let judged = candidates(
            &listed(dir, &bytes, row, list),
            |to| crate::reseat::resolved(dir, to),
            left,
            observed(),
            &latched,
            now.epoch(),
        );
        unlisted.set(judged.is_empty());
        judged
    };
    let argv = |key| Argv {
        session: own_session.to_owned(),
        dir: dir.to_path_buf(),
        slot: slot.to_owned(),
        key,
    };
    // A seat on its automatic chain whose return episode is open goes home,
    // unless its own limit or headroom path holds it (#305).
    let homeward = super::return_episode(&events, own_session, slot, &row.name).filter(|_| {
        !sight.limited && super::headroom_episode(&events, own_session, slot, &row.name).is_none()
    });
    let planned = if let Some(open) = &homeward {
        let latched = super::latched_identities(meta.roster(), &events, own_session, slot);
        let left = super::left_profiles(&events, own_session, slot, &row.name);
        let left_on_headroom = super::left_on_headroom(&events, own_session, slot, &row.name);
        let found = home(
            dir,
            &events,
            (own_session, row),
            &settings,
            (observed(), &latched),
            now.epoch(),
        );
        let judge = |origin: &str| {
            candidates(
                &[(origin.to_owned(), Provenance::Declared)],
                |to| crate::reseat::resolved(dir, to),
                (&left, &left_on_headroom),
                observed(),
                &latched,
                now.epoch(),
            )
        };
        plan_return(
            &settings,
            (&seat, &sight),
            open,
            found.as_ref(),
            judge,
            pair.as_ref(),
            now,
        )
    } else {
        plan_paired(
            &settings,
            &seat,
            &sight,
            &start,
            &events,
            gather,
            pair.as_ref(),
            now,
        )
    };
    // A no-candidate ending of a due episode names a held choice instead (#269),
    // once per key: the episode `plan` judged, chosen as it chose it.
    let due = || {
        let headroom = super::headroom_episode(&events, own_session, slot, &row.name);
        let keyed = headroom
            .as_ref()
            .is_some_and(|found| start.urgent(found.key.epoch()));
        if !sight.limited && (keyed || settings.headroom_at.is_some()) {
            headroom
        } else {
            super::episode(&events, own_session, slot, &row.name)
        }
    };
    let ended = homeward.is_none()
        && (matches!(planned, Plan::Refuse { .. })
            || (matches!(planned, Plan::Decline(_)) && unlisted.get()));
    if let Some(found) = due().filter(|_| ended)
        && let Some(why) = observed_ending(dir, slot)
    {
        if found.choice_held {
            return decline(err, "its observed choice is already held");
        }
        let why = tagged(&why, found.cause());
        let code = close(&argv(found.key), &row.name, now, (HELD_ACTION, &why), err)?;
        let ending = super::Ending::Held(CauseClass::ObservedChoice, &why);
        tell_ending(dir, (own_session, slot, &row.name), found.key, now, &ending);
        return Ok(code);
    }
    match planned {
        Plan::Decline(why) => decline(err, &why),
        Plan::Refuse { key, class, why } => {
            let argv = argv(key);
            let seat_of = (own_session, slot, row.name.as_str());
            let retell = !pair_told(&events, seat_of, class, &key.to_string(), &why);
            let code = close(&argv, &row.name, now, (REFUSED_ACTION, &why), err)?;
            if retell {
                let ending = super::Ending::Refused(class, &why);
                said(&argv, now, &super::notice(own_session, &row.name, &ending));
            }
            Ok(code)
        }
        Plan::Attempt {
            key,
            to,
            note,
            cause,
        } => {
            let exe = crate::shape::resolved_exe();
            let leg = crate::session_launch::capture::auto_reseat_argv(dir, slot, key);
            // No core path, no argv (unreachable here: eligibility asked the
            // same slot grammar) and a refused spawn are one failure: the
            // attempt is journaled, then closed `failed`, and the verb exits 1.
            let spawn = || {
                exe.zip(leg)
                    .is_some_and(|(exe, leg)| crate::transport::spawn_detached(&exe, &leg))
            };
            commit(
                &argv(key),
                &row.name,
                (&profile, &to),
                (&note, &cause),
                now,
                held,
                spawn,
                err,
            )
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
    let line = journal_line(argv, agent, now, action, reference, summary);
    let _ = crate::store::open(&argv.dir).append_event(&line);
}

/// The event line [`record`] appends: its summary [`bounded`].
fn journal_line(
    argv: &Argv,
    agent: &str,
    now: Timestamp,
    action: &str,
    reference: &str,
    summary: &str,
) -> String {
    crate::tracked::event_line(&EventFields {
        ts: now,
        actor: WATCHDOG_ACTOR,
        action,
        target: if agent.is_empty() { &argv.slot } else { agent },
        reference,
        actor_slot: "",
        actor_session: "",
        target_slot: &argv.slot,
        target_session: &argv.session,
        actor_session_id: "",
        target_session_id: "",
        target_server: "",
        target_pane: "",
        target_session_uuid: "",
        caller_server: "",
        caller_pane: "",
        caller_session_uuid: "",
        identity_gap: "",
        summary: &bounded(summary),
        body_file: "",
    })
}

#[cfg(test)]
mod effort_scope_spec {
    use crate::meta::{
        RecordedClient, RecordedConfigHome, RecordedConfigHomeBase, RecordedWorkDir, RosterEntry,
    };
    use crate::quota::{Account, Credits, Group, Observation, Policy, Row, Status};
    use crate::tool::ToolKind;

    const NOW: i64 = 1_700_000_010;

    fn seat() -> RosterEntry {
        RosterEntry {
            slot: "spawned.0".into(),
            name: "scout".into(),
            profile: Some("opusx".into()),
            client: RecordedClient::Missing,
            harness_session: None,
            config_home: RecordedConfigHome::Path("/effort-spec-account".into()),
            config_home_base: RecordedConfigHomeBase::Missing,
            binary: Some("claude".into()),
            work_dir: RecordedWorkDir::Missing,
        }
    }

    fn group(source: &str, qualifier: Option<&str>, percent: &str, policy: Policy) -> Group {
        Group {
            profiles: vec!["opusx".into()],
            tool: ToolKind::Claude,
            home: None,
            source: Some(source.into()),
            clients: vec![],
            rollout: None,
            owner: None,
            rows: vec![Row {
                bucket: "weekly".into(),
                qualifier: qualifier.map(str::to_owned),
                window_minutes: Some(10_080),
                used_percent: Some(percent.into()),
                resets_at: Some(NOW + 100_000),
                observed_at: Some(NOW),
                status: Status::Fresh,
            }],
            hint: None,
            summary: None,
            policy,
            notes: vec![],
            plan: None,
        }
    }

    fn observation(groups: Vec<Group>) -> Observation {
        Observation {
            groups,
            rendered: vec![],
            home: None,
            now: NOW,
        }
    }

    #[test]
    fn effort_scope_uses_existing_effective_math_for_resets_credits_and_stale_rows() {
        for (policy, expected) in [
            (Policy::for_tests(Some(1), Account::default()), 47.5_f64),
            (
                Policy::for_tests(
                    None,
                    Account {
                        credits: Credits::Unlimited,
                        credits_observed_at: Some(NOW),
                        ..Account::default()
                    },
                ),
                0.0,
            ),
            (Policy::for_tests(None, Account::default()), 95.0),
        ] {
            let mut g = group(
                "/effort-spec-account/.claude.json",
                Some("opus"),
                "95",
                policy,
            );
            g.rows[0].observed_at = Some(NOW - 1000);
            let observed = observation(vec![g]);
            let actual = super::own_scope(Some(&observed), &seat(), Some("opus"), NOW);
            assert!(actual.is_some(), "own recorded account must join");
            let actual = actual.expect("asserted own scope");
            assert_eq!(actual.windows.len(), 1);
            assert_eq!(
                actual.windows[0].1.judged.to_bits(),
                expected.to_bits(),
                "raw95 is never effort's quota arithmetic"
            );
            assert_eq!(actual.windows[0].1.status, Status::Stale);
            assert_eq!(actual.windows[0].0.bucket, "weekly");
            assert_eq!(actual.windows[0].0.qualifier.as_deref(), Some("opus"));
            assert_eq!(actual.windows[0].0.minutes, Some(10_080));
        }
    }

    #[test]
    fn effort_scope_ignores_other_accounts_and_model_family_pressure() {
        let observed = observation(vec![
            group(
                "/effort-spec-account/.claude.json",
                Some("opus"),
                "40",
                Policy::default(),
            ),
            group(
                "/effort-spec-account/.claude.json",
                Some("sonnet"),
                "99",
                Policy::default(),
            ),
            group(
                "/effort-spec-other/.claude.json",
                Some("opus"),
                "99",
                Policy::default(),
            ),
        ]);
        let actual = super::own_scope(Some(&observed), &seat(), Some("opus"), NOW);
        assert!(actual.is_some());
        let actual = actual.expect("asserted own scope");
        assert_eq!(
            actual.windows.len(),
            1,
            "foreign account and sonnet family are filtered"
        );
        assert_eq!(actual.windows[0].1.judged.to_bits(), 40.0_f64.to_bits());
        assert!(!actual.spend_capped);
    }

    #[test]
    fn effort_scope_any_usable_own_account_cap_precedes_family_and_length_filters() {
        let capped = Policy::for_tests(
            None,
            Account {
                spend_control_reached: Some(true),
                spend_observed_at: Some(NOW),
                ..Account::default()
            },
        );
        let mut foreign_family = group(
            "/effort-spec-account/.claude.json",
            Some("sonnet"),
            "0",
            capped,
        );
        foreign_family.rows[0].window_minutes = Some(300);
        let observed = observation(vec![foreign_family]);
        let actual = super::own_scope(Some(&observed), &seat(), Some("opus"), NOW);
        assert!(actual.is_some());
        let actual = actual.expect("asserted own scope");
        assert!(
            actual.spend_capped,
            "scope-wide cap holds even when its only usable row is short and another model family"
        );
        assert!(
            actual.windows.is_empty(),
            "foreign-family pressure remains filtered"
        );
    }

    #[test]
    fn effort_scope_foreign_or_unusable_cap_cannot_block_this_account() {
        let capped = Policy::for_tests(
            None,
            Account {
                spend_control_reached: Some(true),
                spend_observed_at: Some(NOW),
                ..Account::default()
            },
        );
        let mut unusable = group(
            "/effort-spec-account/.claude.json",
            Some("opus"),
            "95",
            capped.clone(),
        );
        unusable.rows[0].resets_at = Some(NOW);
        let observed = observation(vec![
            unusable,
            group(
                "/effort-spec-other/.claude.json",
                Some("opus"),
                "95",
                capped,
            ),
        ]);
        let actual = super::own_scope(Some(&observed), &seat(), Some("opus"), NOW);
        assert!(actual.is_some());
        assert!(!actual.expect("asserted own scope").spend_capped);
    }

    /// The routing pressure oracle: the seat's own account, effective (never
    /// raw) judged percentage, worst usable window proven at least a day long.
    fn weekly(qualifier: Option<&str>, percent: &str) -> Group {
        group(
            "/effort-spec-account/.claude.json",
            qualifier,
            percent,
            Policy::default(),
        )
    }

    /// A resume waives grace on proof, so a scoped full window counts only when
    /// its family and the seat's are both named and equal. "Not proven elsewhere"
    /// is enough for an ask, and not for a move.
    #[test]
    fn a_scoped_window_exhausts_a_seat_only_on_a_proven_family_match() {
        for (qualifier, pin, expected) in [
            (None, Some("opus"), true),
            (None, None, true),
            (Some("Opus"), Some("opus"), true),
            (Some("Opus"), Some("sonnet"), false),
            (Some("mystery"), Some("opus"), false),
            (Some("Opus"), None, false),
            (Some("Opus"), Some("mystery"), false),
            (Some("mystery"), None, false),
        ] {
            let observed = observation(vec![weekly(qualifier, "100")]);
            assert_eq!(
                super::own_exhausted(Some(&observed), &seat(), pin, NOW).is_some(),
                expected,
                "qualifier={qualifier:?} pin={pin:?}"
            );
        }
    }

    /// A spend cap leaves the account nothing, so the seat reads full however
    /// little a window has used: on its own bound row, on a row no family binds
    /// with nothing bound left, and on a row that binds while the cap came from
    /// one that does not.
    #[test]
    fn a_spend_cap_reads_full_whichever_window_carried_it() {
        let capped = Policy::for_tests(
            None,
            Account {
                spend_control_reached: Some(true),
                spend_observed_at: Some(NOW),
                ..Account::default()
            },
        );
        let source = "/effort-spec-account/.claude.json";
        for (case, groups) in [
            ("bound", vec![group(source, None, "3", capped.clone())]),
            (
                "nothing bound",
                vec![group(source, Some("mystery"), "3", capped.clone())],
            ),
            (
                "another row binds",
                vec![
                    group(source, Some("mystery"), "3", capped.clone()),
                    group(source, None, "3", Policy::default()),
                ],
            ),
        ] {
            let observed = observation(groups);
            let judged = super::own_exhausted(Some(&observed), &seat(), Some("opus"), NOW)
                .map(|window| window.judged.to_bits());
            assert_eq!(judged, Some(100.0_f64.to_bits()), "{case}");
        }
    }

    #[test]
    fn pressure_is_the_worst_usable_day_long_window_of_the_seats_own_account() {
        let mut short = weekly(None, "99");
        short.rows[0].window_minutes = Some(300);
        let mut stale = weekly(None, "70");
        stale.rows[0].observed_at = Some(NOW - 1000);
        // Older than the stale reading of the same window, so it rules nothing.
        let mut older = weekly(None, "62");
        older.rows[0].observed_at = Some(NOW - 1100);
        let mut passed = weekly(None, "97");
        passed.rows[0].resets_at = Some(NOW);
        let mut unstated = weekly(None, "98");
        unstated.rows[0].window_minutes = None;
        let observed = observation(vec![
            short,
            stale,
            older,
            passed,
            unstated,
            group(
                "/effort-spec-other/.claude.json",
                None,
                "100",
                Policy::default(),
            ),
        ]);
        let actual = super::own_pressure(Some(&observed), &seat(), None, NOW);
        assert_eq!(
            actual.map(f64::to_bits),
            Some(70.0_f64.to_bits()),
            "a stale reading stands; a short, passed-reset, unstated-length or foreign window never counts"
        );
    }

    #[test]
    fn pressure_is_the_effective_percentage_never_the_raw_one() {
        let one_reset = Policy::for_tests(Some(1), Account::default());
        let observed = observation(vec![group(
            "/effort-spec-account/.claude.json",
            None,
            "95",
            one_reset,
        )]);
        let actual = super::own_pressure(Some(&observed), &seat(), None, NOW);
        assert_eq!(actual.map(f64::to_bits), Some(47.5_f64.to_bits()));
    }

    #[test]
    fn pressure_skips_a_window_scoped_to_another_model_family() {
        let observed = observation(vec![
            weekly(Some("opus"), "40"),
            weekly(Some("sonnet"), "99"),
        ]);
        let actual = super::own_pressure(Some(&observed), &seat(), Some("opus"), NOW);
        assert_eq!(actual.map(f64::to_bits), Some(40.0_f64.to_bits()));
    }

    /// Without a model pin no window is proven to bind another family, so the
    /// worst of them reads: the seat is asked, not excused.
    #[test]
    fn pressure_without_a_pin_counts_every_family() {
        let observed = observation(vec![
            weekly(Some("opus"), "40"),
            weekly(Some("sonnet"), "99"),
        ]);
        let actual = super::own_pressure(Some(&observed), &seat(), None, NOW);
        assert_eq!(actual.map(f64::to_bits), Some(99.0_f64.to_bits()));
    }

    /// A reached spend cap is the account's, not one window's: the account
    /// reads full whatever lengths and families its rows are.
    #[test]
    fn pressure_of_a_spend_capped_account_is_full_even_when_only_a_short_window_reads() {
        let capped = Policy::for_tests(
            None,
            Account {
                spend_control_reached: Some(true),
                spend_observed_at: Some(NOW),
                ..Account::default()
            },
        );
        let mut short = group("/effort-spec-account/.claude.json", None, "3", capped);
        short.rows[0].window_minutes = Some(300);
        let observed = observation(vec![short]);
        let actual = super::own_pressure(Some(&observed), &seat(), None, NOW);
        assert_eq!(actual.map(f64::to_bits), Some(100.0_f64.to_bits()));
    }

    #[test]
    fn pressure_is_unknown_without_a_proven_identity_an_observation_or_a_long_window() {
        let observed = observation(vec![weekly(None, "95")]);
        let mut entry = seat();
        entry.config_home = RecordedConfigHome::Missing;
        assert_eq!(
            super::own_pressure(Some(&observed), &entry, None, NOW),
            None
        );
        assert_eq!(super::own_pressure(None, &seat(), None, NOW), None);
        let mut short = weekly(None, "95");
        short.rows[0].window_minutes = Some(300);
        let observed = observation(vec![short]);
        assert_eq!(
            super::own_pressure(Some(&observed), &seat(), None, NOW),
            None
        );
        let observed = observation(vec![group(
            "/effort-spec-other/.claude.json",
            None,
            "95",
            Policy::default(),
        )]);
        assert_eq!(
            super::own_pressure(Some(&observed), &seat(), None, NOW),
            None
        );
    }

    #[test]
    fn effort_scope_missing_or_inconsistent_recorded_identity_is_unread() {
        let observed = observation(vec![group(
            "/effort-spec-account/.claude.json",
            None,
            "95",
            Policy::default(),
        )]);
        let mut entry = seat();
        entry.config_home = RecordedConfigHome::Missing;
        assert!(super::own_scope(Some(&observed), &entry, Some("opus"), NOW).is_none());
        entry = seat();
        entry.config_home_base = RecordedConfigHomeBase::Path("/other".into());
        assert!(super::own_scope(Some(&observed), &entry, Some("opus"), NOW).is_none());
        assert!(super::own_scope(None, &seat(), Some("opus"), NOW).is_none());
    }
}

#[cfg(test)]
#[path = "leg_limitwake_spec.rs"]
mod leg_limitwake_spec;

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

    /// `list` as a row declares it.
    fn declared(list: &[String]) -> Vec<(String, Provenance)> {
        list.iter()
            .map(|profile| (profile.clone(), Provenance::Declared))
            .collect()
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
                Ended::Moved {
                    fate: crate::autoreseat::Fate::Carried,
                },
                DONE_ACTION,
                left,
                "to opus55x, carried",
            ),
            (
                Ended::Moved {
                    fate: crate::autoreseat::Fate::Seeded,
                },
                DONE_ACTION,
                left,
                "to opus55x, seeded",
            ),
            (
                Ended::Refused(CauseClass::Busy),
                HELD_ACTION,
                KEY,
                "busy: Error: busy",
            ),
            (
                Ended::Refused(CauseClass::Lock),
                HELD_ACTION,
                KEY,
                "lock: Error: busy",
            ),
            (
                Ended::Refused(CauseClass::CarryTargetDiffers),
                REFUSED_ACTION,
                KEY,
                "carry-target-differs: Error: busy",
            ),
            (
                Ended::Refused(CauseClass::Other),
                REFUSED_ACTION,
                KEY,
                "other: Error: busy",
            ),
            (Ended::Failed, FAILED_ACTION, KEY, "other: Error: busy"),
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

    /// A message long enough to clip keeps the class the refusing site named
    /// at its head, in every record and in the notice built from it, for every
    /// class a reseat can end with.
    #[test]
    fn a_clipped_outcome_keeps_its_class_in_the_record_and_the_notice() {
        let path = "/Users/u/".to_owned() + &"deep/".repeat(60);
        let err = format!("Error: 'colead' could not carry conversation c from {path} to {path}\n");
        for class in [
            CauseClass::CarrySourceMissing,
            CauseClass::CarryTargetDiffers,
            CauseClass::CarryUnreadable,
            CauseClass::Busy,
            CauseClass::UnknownFrame,
            CauseClass::Lock,
            CauseClass::Other,
        ] {
            let (action, _, summary) = outcome(Ended::Refused(class), KEY, "a", "b", &err);
            assert!(
                summary.starts_with(&format!("{}: Error:", class.word())),
                "{summary:?}"
            );
            assert_eq!(
                action,
                if class.transient() {
                    HELD_ACTION
                } else {
                    REFUSED_ACTION
                }
            );
            let ending = if class.transient() {
                crate::autoreseat::Ending::Held(class, &summary)
            } else {
                crate::autoreseat::Ending::Refused(class, &summary)
            };
            let told = crate::autoreseat::notice("aedev", "colead", &ending);
            assert!(
                told.find(class.word()).is_some_and(|at| at < 60),
                "{told:?}"
            );
        }
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
        let resolve = |profile: &str| (profile != "ghost").then_some(Resolved { pin: None });
        let choice = choose(
            &candidates(&declared(&list), resolve, (&left, &[]), None, &[], 0),
            0,
        );
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
            plan: None,
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

    #[test]
    fn the_origin_samples_keep_every_rollout_and_unjudgeable_row_of_its_accounts_only() {
        let mut unread = row(None, "10", 60, 3600);
        unread.used_percent = None;
        let quota = observed(vec![
            account(
                &["origin"],
                "/a",
                plain(),
                vec![row(None, "10", 60, 3600), unread],
            ),
            account(
                &["origin", "twin"],
                "/a",
                plain(),
                vec![row(None, "40", 30, 3600)],
            ),
            account(
                &["origin"],
                "/b",
                plain(),
                vec![row(Some("Fable"), "99", 60, 3600)],
            ),
            account(&["other"], "/c", plain(), vec![row(None, "99", 60, 3600)]),
        ]);
        let samples = origin_samples(Some(&quota), "origin", Some("opus"), NOW);
        let sources: Vec<&Path> = samples
            .iter()
            .map(|((_, source, _), _)| source.as_path())
            .collect();
        assert_eq!(
            sources,
            [Path::new("/a"); 3],
            "another family's row and another profile's account are no reading of it"
        );
        assert!(
            matches!(samples[1].1, super::super::Sample::Unjudged(Some(_))),
            "a row with no percentage is kept, unjudged, with its stamp"
        );
        assert!(
            samples
                .iter()
                .filter(|(_, sample)| matches!(sample, super::super::Sample::Judged(_)))
                .count()
                == 2,
            "both rollouts of the account stay"
        );
        assert!(origin_samples(None, "origin", None, NOW).is_empty());
    }

    /// Under the lifecycle lock a return stands only while the locked row still
    /// records the profile it was chosen from and the journal's chain still
    /// ends in the move it was chosen after (M2).
    #[test]
    fn a_return_stands_under_the_lock_only_on_its_own_profile_and_chain() {
        let root = Root(std::env::temp_dir().join(format!("ae-leg-return-{}", std::process::id())));
        assert!(std::fs::create_dir_all(&root.0).is_ok(), "a session dir");
        let line = |at: i64, actor: &str, action: &str, rest: &str| {
            format!(
                r#"{{"ts":"{}","actor":"{actor}","action":"{action}","target":"scout"{rest},"summary":"reseated scout (pane %1, slot spawned.3) to substitute, seed submitted"}}"#,
                Timestamp::from_epoch(at)
            ) + "\n"
        };
        let routed = r#","target_slot":"spawned.3","target_session":"aedev""#;
        let journal = [
            line(100, "watchdog", "auto-reseat-headroom", routed),
            line(
                101,
                "watchdog",
                "auto-reseat",
                &format!(r#"{routed},"ref":"{}""#, Timestamp::from_epoch(100)),
            ),
            line(102, "watchdog", "reseat", r#","target_slot":"spawned.3""#),
            line(
                103,
                "watchdog",
                "auto-reseat-done",
                &format!(r#"{routed},"ref":"origin""#),
            ),
        ]
        .concat();
        let write = |text: &str| {
            assert!(
                std::fs::write(root.0.join("events.jsonl"), text).is_ok(),
                "a journal"
            );
        };
        write(&journal);
        let seat = ("aedev", "spawned.3", "scout");
        let expect = Expect {
            profile: "substitute".to_owned(),
            newest: Timestamp::from_epoch(103),
        };
        let locked = b"profile.spawned.3=substitute\n";
        assert_eq!(return_changed(&root.0, locked, seat, &expect), None);
        let moved = return_changed(&root.0, b"profile.spawned.3=manual\n", seat, &expect);
        assert!(
            moved.as_deref().is_some_and(|why| why.contains("'manual'")),
            "{moved:?}"
        );
        let later = Expect {
            newest: Timestamp::from_epoch(99),
            ..expect.clone()
        };
        assert!(
            return_changed(&root.0, locked, seat, &later).is_some(),
            "another move since the choice"
        );
        for (ender, why) in [
            (
                line(110, "human", "reseat", r#","target_slot":"spawned.3""#),
                "a manual reseat",
            ),
            (line(110, "lead", "spawn", routed), "a spawn"),
        ] {
            write(&(journal.clone() + &ender));
            assert!(
                return_changed(&root.0, locked, seat, &expect)
                    .is_some_and(|said| said.contains("no longer on the automatic chain")),
                "{why}"
            );
        }
    }

    #[test]
    fn a_usable_spend_cap_or_a_latched_peer_on_an_origin_account_blocks_the_return() {
        let claude = crate::tool::ToolKind::Claude;
        let accounts = [(claude, PathBuf::from("/a"))];
        let cap = |observed_at| {
            crate::quota::Policy::for_tests(
                None,
                crate::quota::Account {
                    spend_control_reached: Some(true),
                    spend_observed_at: Some(observed_at),
                    ..crate::quota::Account::default()
                },
            )
        };
        let on = |tool, source: &str| crate::quota::RecordedIdentity {
            tool,
            source: PathBuf::from(source),
        };
        let open = observed(vec![account(
            &["origin"],
            "/a",
            plain(),
            vec![row(None, "10", 60, 3600)],
        )]);
        assert!(!origin_blocked(Some(&open), &accounts, &[], NOW));
        assert!(!origin_blocked(None, &accounts, &[], NOW));
        let capped = observed(vec![account(
            &["origin"],
            "/a",
            cap(NOW - 60),
            vec![row(None, "10", 60, 3600)],
        )]);
        assert!(origin_blocked(Some(&capped), &accounts, &[], NOW));
        let mut spent_row = row(None, "10", 60, 3600);
        spent_row.resets_at = Some(NOW);
        let unusable = observed(vec![account(
            &["origin"],
            "/a",
            cap(NOW - 60),
            vec![spent_row],
        )]);
        assert!(
            !origin_blocked(Some(&unusable), &accounts, &[], NOW),
            "a cap with no usable window proves nothing"
        );
        let elsewhere = observed(vec![account(
            &["other"],
            "/c",
            cap(NOW - 60),
            vec![row(None, "10", 60, 3600)],
        )]);
        assert!(
            !origin_blocked(Some(&elsewhere), &accounts, &[], NOW),
            "another account's cap"
        );
        assert!(origin_blocked(
            Some(&open),
            &accounts,
            &[on(claude, "/a")],
            NOW
        ));
        assert!(
            !origin_blocked(
                Some(&open),
                &accounts,
                &[on(crate::tool::ToolKind::Codex, "/a"), on(claude, "/c")],
                NOW
            ),
            "another tool on the same path, or another path, is another account"
        );
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
            &candidates(
                &declared(&words(list)),
                resolve,
                (left, &[]),
                Some(quota),
                latched,
                NOW,
            ),
            NOW,
        )
    }

    #[allow(
        clippy::unnecessary_wraps,
        reason = "compared against a pick, which is optional"
    )]
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
    /// together, and per window the newest usable reading rules: a row whose
    /// reset passed never counts, an exhausted one counts while it is the
    /// newest, and a newer healthy reading of its window supersedes it.
    #[test]
    fn a_reset_rollout_row_never_counts_and_an_exhausted_one_counts_while_newest() {
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
        let spent = row(None, "100", 30, 3600);
        for groups in [
            vec![rollout(vec![spent.clone()]), rollout(vec![current.clone()])],
            vec![rollout(vec![current.clone()]), rollout(vec![spent.clone()])],
        ] {
            let choice = judge(&observed(groups), &["sol"], &[], &[], &[]);
            assert_eq!(choice.skipped, passed(&[("sol", Skip::Exhausted)]));
        }
        let superseded = row(None, "100", 120, 3600);
        for groups in [
            vec![
                rollout(vec![superseded.clone()]),
                rollout(vec![current.clone()]),
            ],
            vec![
                rollout(vec![current.clone()]),
                rollout(vec![superseded.clone()]),
            ],
        ] {
            let choice = judge(&observed(groups), &["sol"], &[], &[], &[]);
            assert_eq!(
                choice.pick,
                picked("sol", Tier::BelowCritical),
                "a newer healthy reading of the window supersedes the spent one"
            );
        }
    }

    /// One path read as two tools' sources is two accounts, as a recorded
    /// identity names one by tool and source: a newer healthy reading of one
    /// tool's window never supersedes the other tool's spent reading of the
    /// same profile, path and window, while within one tool it does.
    #[test]
    fn a_reading_of_another_tool_on_the_same_path_never_supersedes_one() {
        let on = |tool, rows| crate::quota::Group {
            tool,
            ..account(&["sol"], "/a", plain(), rows)
        };
        let (claude, codex) = (crate::tool::ToolKind::Claude, crate::tool::ToolKind::Codex);
        let spent = row(None, "100", 120, 3600);
        let healthy = row(None, "40", 60, 3600);
        for groups in [
            vec![
                on(claude, vec![spent.clone()]),
                on(codex, vec![healthy.clone()]),
            ],
            vec![
                on(codex, vec![spent.clone()]),
                on(claude, vec![healthy.clone()]),
            ],
            vec![
                on(codex, vec![healthy.clone()]),
                on(claude, vec![spent.clone()]),
            ],
            vec![
                on(claude, vec![healthy.clone()]),
                on(codex, vec![spent.clone()]),
            ],
        ] {
            let choice = judge(&observed(groups), &["sol"], &[], &[], &[]);
            assert_eq!(
                choice.skipped,
                passed(&[("sol", Skip::Exhausted)]),
                "another tool's reading of the path supersedes nothing"
            );
        }
        let same = vec![
            on(codex, vec![spent.clone()]),
            on(codex, vec![healthy.clone()]),
        ];
        let choice = judge(&observed(same), &["sol"], &[], &[], &[]);
        assert_eq!(
            choice.pick,
            picked("sol", Tier::BelowCritical),
            "within one tool the newer reading rules"
        );
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
            provenance: Provenance::Declared,
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
            provenance: Provenance::Declared,
            skipped: passed(&[("opus55x", Skip::Exhausted), ("ghost", Skip::Unconfigured)]),
        };
        assert_eq!(
            deadline("scout", &none, 600),
            "ae cannot move scout: no usable candidate: opus55x (exhausted), ghost (not configured here)"
        );
    }

    #[test]
    fn the_forecast_and_the_attempt_name_a_derived_candidate_by_its_provenance() {
        let candidate = |profile: &str, provenance, judged: f64| Candidate {
            profile: profile.to_owned(),
            provenance,
            configured: true,
            peer_latched: false,
            windows: vec![crate::autoreseat::Window {
                judged,
                critical: judged >= 95.0,
                long: true,
                observed_at: 0,
                resets_at: None,
                status: crate::quota::Status::Fresh,
                label: "weekly_all 7d".to_owned(),
            }],
            left_at: None,
            left_on_headroom: false,
        };
        let full = candidate(
            "full",
            Provenance::Derived(
                crate::autoreseat::Kin::Sibling,
                crate::autoreseat::Step::Seeded,
            ),
            100.0,
        );
        let twin = candidate(
            "twin",
            Provenance::Derived(
                crate::autoreseat::Kin::Twin,
                crate::autoreseat::Step::Account,
            ),
            10.0,
        );
        let choice = choose(&[full.clone(), twin], 0);
        assert_eq!(
            deadline("scout", &choice, 600),
            "ae will move scout to twin, derived twin in 10m"
        );
        assert_eq!(
            attempt_note("headroom 95% weekly_all 7d", &choice.skipped),
            " (headroom 95% weekly_all 7d); passed over full, derived sibling (exhausted)"
        );
        let none = choose(&[full, candidate("spent", Provenance::Declared, 100.0)], 0);
        assert_eq!(
            deadline("scout", &none, 600),
            "ae cannot move scout: no usable candidate: full, derived sibling (exhausted), \
             spent (exhausted)"
        );
    }

    /// `quota = off` says ae must not act on vendor numbers: the chooser reads
    /// none, so a spent account is taken in declared order like any unknown.
    #[test]
    fn with_quota_off_a_spent_account_is_judged_unknown_in_declared_order() {
        let root = Root(std::env::temp_dir().join(format!("ae-leg-quota-{}", std::process::id())));
        let dir = root.0.join("aedev");
        assert!(std::fs::create_dir_all(&dir).is_ok(), "a session dir");
        let now = Timestamp::now().epoch();
        let spent = format!(
            "{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":{},\"utilization\":{{\"limits\":[{{\"kind\":\"session\",\"group\":\"session\",\"percent\":100,\"resets_at\":\"{}\",\"scope\":null}}]}}}}}}\n",
            now * 1_000,
            Timestamp::from_epoch(now + 7_200),
        );
        assert!(std::fs::write(root.0.join(".claude.json"), spent).is_ok());
        let config = root.0.join("config");
        for (switch, pick, skipped) in [
            ("on", None, passed(&[("spent", Skip::Exhausted)])),
            ("off", picked("spent", Tier::Unknown), Vec::new()),
        ] {
            let text = format!("[workspace]\nquota = {switch}\n[profiles]\nspent = claude\n");
            assert!(std::fs::write(&config, text).is_ok(), "a config");
            let inputs = crate::quota::Inputs {
                home: Some(&root.0),
                global: Some(&config),
                local: None,
                sessions: None,
                now,
            };
            let quota = read_quota(&dir, &inputs);
            let resolve = |_: &str| Some(Resolved { pin: None });
            let judged = candidates(
                &declared(&words(&["spent"])),
                resolve,
                (&[], &[]),
                quota.as_ref(),
                &[],
                now,
            );
            let choice = choose(&judged, now);
            assert_eq!(
                (choice.pick, choice.skipped),
                (pick, skipped),
                "quota = {switch}"
            );
        }
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
            ("\n  \ntmux said no\nmore\n", "other: tmux said no"),
            ("", "other: the reseat printed no reason"),
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
            headroom_at: None,
            headroom_note: None,
            lead: crate::autoreseat::Lead::default(),
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
            near: false,
        };
        let clear = sight(Frame::Clear, false, None, true);
        let now = Timestamp::from_epoch(key.epoch() + 700);
        let attempt = Plan::Attempt {
            key,
            to: "opus55x".to_owned(),
            note: String::new(),
            cause: String::new(),
        };
        let usable = |list: &[String], left: (&[(String, Timestamp)], &[String])| {
            candidates(
                &declared(list),
                |_| Some(Resolved { pin: None }),
                left,
                None,
                &[],
                0,
            )
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
                |list, left| candidates(&declared(list), |_| None, left, None, &[], 0),
                now
            ),
            Plan::Refuse {
                key,
                class: CauseClass::NoCandidate,
                why: "no-candidate: refused: no usable candidate: opus55x (not configured here)"
                    .to_owned()
            }
        );
    }

    /// A refused or failed limit move is attempted again by the trigger, under
    /// the same key, only once its cooldown has passed and only until the last
    /// round; a retry with nothing usable is refused under that key again.
    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one table of retry rows against one seat, read top to bottom"
    )]
    fn the_trigger_retries_a_closed_limit_move_under_its_key_after_the_cooldown_and_not_past_the_cap()
     {
        use crate::autoreseat::{
            ATTEMPT_ACTION, FAILED_ACTION, Frame, MAX_ROUNDS, Pane, RETRY_COOLDOWN_SECS, Settings,
            Switch,
        };
        type Gather = dyn Fn(&[String], (&[(String, Timestamp)], &[String])) -> Vec<Candidate>;
        let key = Timestamp::parse(KEY).expect("the key parses");
        let record = |since: i64, action: &str, reference: &str| {
            Event::parse_line(&format!(
                r#"{{"ts":"{}","actor":"watchdog","action":"{action}","target":"scout","ref":"{reference}"}}"#,
                Timestamp::from_epoch(key.epoch() + since)
            ))
            .expect("a well-formed record")
        };
        let on = Settings {
            switch: Switch::On,
            sessions: None,
            grace_secs: 600,
            map: vec![("sol6x".to_owned(), vec!["opus55x".to_owned()])],
            headroom_at: None,
            headroom_note: None,
            lead: crate::autoreseat::Lead::default(),
            notes: Vec::new(),
        };
        let seat = Seat {
            session: "aedev",
            slot: "spawned.3",
            agent: "scout",
            profile: "sol6x",
            orchestrator: false,
        };
        let clear = Sight {
            pane: Pane {
                frame: Frame::Clear,
                human_prompt: false,
                client_input: None,
            },
            limited: true,
            near: false,
        };
        let usable = |list: &[String], left: (&[(String, Timestamp)], &[String])| {
            candidates(
                &declared(list),
                |_| Some(Resolved { pin: None }),
                left,
                None,
                &[],
                0,
            )
        };
        let nothing = |list: &[String], left: (&[(String, Timestamp)], &[String])| {
            candidates(&declared(list), |_| None, left, None, &[], 0)
        };
        let at = |since: i64| Timestamp::from_epoch(key.epoch() + since);
        // A journal of `closings`, an attempt before each, the next one
        // cooldown after the last closing; and when the last one closed.
        let rounds = |closings: &[&str]| {
            let mut events = vec![record(0, "limit", "")];
            let (mut attempt, mut closed) = (700, 0);
            for action in closings {
                closed = attempt + 30;
                events.push(record(attempt, ATTEMPT_ACTION, KEY));
                events.push(record(closed, action, KEY));
                attempt = closed + RETRY_COOLDOWN_SECS;
            }
            (events, closed)
        };
        let (events, closed) = rounds(&[FAILED_ACTION]);
        let due = closed + RETRY_COOLDOWN_SECS;
        let retry = Plan::Attempt {
            key,
            to: "opus55x".to_owned(),
            note: String::new(),
            cause: String::new(),
        };
        let plan_at = |events: &[Event], since: i64, gather: &Gather| {
            plan(&on, &seat, &clear, events, gather, at(since))
        };
        assert!(
            matches!(plan_at(&events, due - 1, &usable), Plan::Decline(_)),
            "cooling down"
        );
        assert_eq!(plan_at(&events, due, &usable), retry, "due again, same key");
        assert_eq!(
            plan_at(&events, due, &nothing),
            Plan::Refuse {
                key,
                class: CauseClass::NoCandidate,
                why: "no-candidate: refused: no usable candidate: opus55x (not configured here)"
                    .to_owned()
            },
            "a retry with nothing usable is refused under the key again"
        );
        let (spent, closed) = rounds(&vec![FAILED_ACTION; usize::from(MAX_ROUNDS)]);
        for since in [
            closed + 1,
            closed + RETRY_COOLDOWN_SECS,
            closed + 10_000_000,
        ] {
            assert!(
                matches!(plan_at(&spent, since, &usable), Plan::Decline(_)),
                "past the last round: {since}"
            );
        }
    }

    /// The attempt is DURABLE before the leg starts, because the leg acts only
    /// under an attempt it can read; a leg that could not start closes the
    /// attempt at once instead of leaving it to age out.
    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one handoff, its start and both failed starts"
    )]
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
        let lock = crate::autoreseat::lock_path(&dir, "spawned.3");
        let held = || crate::store::lock(&lock, Duration::ZERO).expect("the trigger's lock");
        // The limit the trigger moves under, opened at the key.
        let opener =
            format!(r#"{{"ts":"{KEY}","actor":"watchdog","action":"limit","target":"scout"}}"#)
                + "\n";
        let open = || {
            let _ = std::fs::remove_file(dir.join(crate::store::EVENTS));
            assert!(
                crate::store::open(&dir).append_event(&opener).is_ok(),
                "the opener"
            );
        };
        open();
        let mut before = Vec::new();
        let mut released = false;
        let mut err = Vec::new();
        let code = commit(
            &argv,
            "scout",
            ("sol6x", "opus55x"),
            ("", ""),
            now,
            held(),
            || {
                before = journal();
                released = crate::store::lock(&lock, Duration::ZERO).is_ok();
                false
            },
            &mut err,
        );
        let limit = ("limit".to_owned(), String::new(), String::new());
        let attempt = (
            ATTEMPT_ACTION.to_owned(),
            KEY.to_owned(),
            "from sol6x to opus55x".to_owned(),
        );
        assert_eq!(
            before,
            [limit.clone(), attempt.clone()],
            "durable before the start"
        );
        assert!(released, "the leg finds the seat's lock free");
        assert_eq!(code.ok(), Some(EXIT_FAILED));
        let why = "other: failed: the move could not be started";
        assert_eq!(
            journal(),
            [
                limit.clone(),
                attempt.clone(),
                (FAILED_ACTION.to_owned(), KEY.to_owned(), why.to_owned()),
                (
                    "chat".to_owned(),
                    String::new(),
                    crate::autoreseat::notice(
                        "aedev",
                        "scout",
                        &crate::autoreseat::Ending::Failed(CauseClass::Other, why)
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
        open();
        let started = commit(
            &argv,
            "scout",
            ("sol6x", "opus55x"),
            ("", ""),
            now,
            held(),
            || true,
            &mut err,
        );
        assert_eq!(started.ok(), Some(0));
        assert_eq!(journal(), [limit.clone(), attempt.clone()]);
        // A failed start writes nothing when another writer holds the lock, or
        // when the journal no longer shows the attempt open.
        for (rival, closed) in [(true, false), (false, true)] {
            open();
            let mut err = Vec::new();
            let mut taken = None;
            let code = commit(
                &argv,
                "scout",
                ("sol6x", "opus55x"),
                ("", ""),
                now,
                held(),
                || {
                    if rival {
                        taken = Some(held());
                    }
                    if closed {
                        let line = format!(
                            r#"{{"ts":"{KEY}","actor":"watchdog","action":"{FAILED_ACTION}","target":"scout","ref":"{KEY}"}}"#
                        ) + "\n";
                        assert!(crate::store::open(&dir).append_event(&line).is_ok());
                    }
                    false
                },
                &mut err,
            );
            drop(taken);
            assert_eq!(code.ok(), Some(EXIT_FAILED));
            let failed = journal()
                .iter()
                .filter(|(action, _, _)| action == FAILED_ACTION)
                .count();
            assert_eq!(failed, usize::from(closed), "{rival} {closed}");
        }
        // Nor when a hold for the key landed since: the episode then shows no
        // attempt open, so this start has nothing left to close.
        open();
        let mut err = Vec::new();
        let code = commit(
            &argv,
            "scout",
            ("sol6x", "opus55x"),
            ("", ""),
            now,
            held(),
            || {
                let line = format!(
                    r#"{{"ts":"{now}","actor":"watchdog","action":"{HELD_ACTION}","target":"scout","ref":"{KEY}"}}"#
                ) + "\n";
                assert!(crate::store::open(&dir).append_event(&line).is_ok());
                false
            },
            &mut err,
        );
        assert_eq!(code.ok(), Some(EXIT_FAILED));
        assert!(
            journal()
                .iter()
                .all(|(action, _, _)| action != FAILED_ACTION),
            "a hold since"
        );
    }
    /// The stop's last word answers for the seat the attempt was opened for: a
    /// slot that now holds another agent, profile or launch answers for nothing.
    #[test]
    fn a_start_read_answers_only_for_the_incarnation_its_attempt_was_opened_for() {
        let root = Root(std::env::temp_dir().join(format!("ae-leg-start-{}", std::process::id())));
        let dir = root.0.join("aedev");
        assert!(std::fs::create_dir_all(&dir).is_ok(), "a session dir");
        let meta = "seat.main=lead\nprofile.main=source\nlaunch_id.main=one\n";
        assert!(std::fs::write(dir.join(crate::store::META), meta).is_ok());
        let argv = |slot: &str| super::Argv {
            session: "aedev".to_owned(),
            dir: dir.clone(),
            slot: slot.to_owned(),
            key: crate::time::Timestamp::from_epoch(NOW),
        };
        let at = crate::time::Timestamp::from_epoch(NOW);
        let read = |slot: &str, seat| super::read_start(&root.0, &dir, &argv(slot), seat, at);
        assert!(read("main", ("lead", "source", Some("one"))).is_some());
        for seat in [
            ("scout", "source", Some("one")),
            ("lead", "other", Some("one")),
            ("lead", "source", Some("two")),
            ("lead", "source", None),
        ] {
            assert!(read("main", seat).is_none(), "{seat:?}");
        }
        assert!(read("spawned.1", ("lead", "source", Some("one"))).is_none());
    }

    const TOLD_AT: i64 = 1_793_000_000;
    const AMBIGUOUS: &str =
        "pair-provider: refused: it serves Anthropic, its pair link is ambiguous";

    /// The pair refusal the leg journals for `slot`, read back as the journal keeps it.
    fn refused_as_journaled(slot: &str, agent: &str, why: &str) -> Event {
        let key = Timestamp::from_epoch(TOLD_AT);
        let argv = Argv {
            session: "aedev".into(),
            dir: PathBuf::from("/nonexistent"),
            slot: slot.into(),
            key,
        };
        let line = journal_line(&argv, agent, key, REFUSED_ACTION, &key.to_string(), why);
        Event::parse_line(&line).expect("the leg writes a well-formed record")
    }

    /// Whether the seat `slot`/`agent` of session `aedev` was already told `why`.
    fn told(events: &[Event], (slot, agent): (&str, &str), why: &str) -> bool {
        let key = Timestamp::from_epoch(TOLD_AT).to_string();
        let seat = ("aedev", slot, agent);
        pair_told(events, seat, CauseClass::PairProvider, &key, why)
    }

    /// A pair refusal counts as told for the seat it is addressed to, by slot
    /// or by its legacy display name, never for another seat (#305).
    #[test]
    fn a_pair_refusal_counts_as_told_only_for_the_seat_it_addresses() {
        let theirs = [refused_as_journaled("spawned.1", "scout", AMBIGUOUS)];
        assert!(told(&theirs, ("spawned.1", "scout"), AMBIGUOUS));
        assert!(!told(&theirs, ("spawned.2", "scout-nav"), AMBIGUOUS));
        let key = Timestamp::from_epoch(TOLD_AT);
        let by_name = Event::parse_line(&format!(
            r#"{{"ts":"{key}","actor":"watchdog","action":"{REFUSED_ACTION}","target":"scout","ref":"{key}","summary":"{AMBIGUOUS}"}}"#
        ))
        .expect("a well-formed record");
        let by_name = std::slice::from_ref(&by_name);
        assert!(told(by_name, ("spawned.1", "scout"), AMBIGUOUS));
        assert!(!told(by_name, ("spawned.2", "scout-nav"), AMBIGUOUS));
        let other = CauseClass::NoCandidate;
        let seat = ("aedev", "spawned.1", "scout");
        assert!(!pair_told(
            &theirs,
            seat,
            other,
            &key.to_string(),
            AMBIGUOUS
        ));
    }

    /// Only the watchdog's own refusal counts as told (#305).
    #[test]
    fn a_pair_refusal_counts_as_told_only_when_the_watchdog_wrote_it() {
        let key = Timestamp::from_epoch(TOLD_AT);
        let forged = Event::parse_line(&format!(
            r#"{{"ts":"{key}","actor":"lead","action":"{REFUSED_ACTION}","target":"scout","ref":"{key}","target_slot":"spawned.1","target_session":"aedev","summary":"{AMBIGUOUS}"}}"#
        ))
        .expect("a well-formed record");
        assert!(!told(&[forged], ("spawned.1", "scout"), AMBIGUOUS));
    }

    /// A reason longer than the journal keeps counts as told once journaled:
    /// it is compared as the journal keeps it (#305).
    #[test]
    fn a_long_pair_refusal_counts_as_told_as_the_journal_keeps_it() {
        let long = format!(
            "{AMBIGUOUS}: no usable candidate: {}",
            ["first", "second", "third", "fourth", "fifth", "sixth"]
                .map(|to| format!("{to} (partner's provider)"))
                .join(", ")
        );
        assert!(long.chars().count() > SUMMARY_CHARS, "{long}");
        let journaled = [refused_as_journaled("spawned.1", "scout", &long)];
        assert!(told(&journaled, ("spawned.1", "scout"), &long));
        assert!(!told(&journaled, ("spawned.1", "scout"), AMBIGUOUS));
    }

    /// What a seat serves is its CURRENT config command, and only while that
    /// command's tool is the one its row records running: a same-tool edit
    /// after launch reads as edited (the named approximation), a tool change
    /// proves nothing, and every profile's provider is the one its move would
    /// serve (#305).
    #[test]
    fn a_pair_guard_reads_the_current_command_and_the_running_tool() {
        use crate::tool::Provider::{Anthropic, OpenAI};
        let root = Root(std::env::temp_dir().join(format!("ae-leg-pair-{}", std::process::id())));
        let dir = root.0.join("demo");
        assert!(std::fs::create_dir_all(&dir).is_ok(), "a session dir");
        let config = dir.join("config");
        let meta = format!(
            "session=demo\nconfig={}\nseat.spawned.0=x-nav\nprofile.spawned.0=own\nagent_bin.spawned.0=claude\n\
             seat.spawned.1=x\nprofile.spawned.1=peer\nagent_bin.spawned.1=codex\n",
            config.display()
        );
        let read = |peer: &str| {
            let text = format!(
                "[profiles]\nown = claude --pg own\npeer = {peer}\nother = agy --model gemini-4\n"
            );
            assert!(std::fs::write(&config, text).is_ok(), "the config");
            let parsed = crate::meta::Meta::parse(&meta);
            pair_of(&dir, meta.as_bytes(), parsed.roster(), false, "spawned.0")
        };
        let pair = read("codex -m gpt-6").expect("a paired seat");
        assert_eq!(pair.partner.as_deref(), Some("x"));
        assert_eq!(
            (pair.seat_serves, pair.partner_serves),
            (Some(Anthropic), Some(OpenAI))
        );
        assert_eq!(
            pair.destinations,
            [
                ("own".to_owned(), Some(Anthropic)),
                ("peer".to_owned(), Some(OpenAI)),
                ("other".to_owned(), None)
            ]
        );
        // Edited after launch, same tool: the edited command decides.
        assert_eq!(
            read("codex --oss").and_then(|pair| pair.partner_serves),
            None
        );
        // Edited onto another tool than the one running: nothing is proven.
        assert_eq!(
            read("claude --pg peer").and_then(|pair| pair.partner_serves),
            None
        );
        let solo = crate::meta::Meta::parse("seat.spawned.0=solo\nprofile.spawned.0=own\n");
        assert_eq!(pair_of(&dir, b"", solo.roster(), false, "spawned.0"), None);
    }

    /// A pair session `demo` under `root`: `x-nav` (spawned.0) and `x`
    /// (spawned.1) on the profiles `own` and `peer` of `profiles`, each with
    /// its `(agent_bin, client)` row; returns the session dir and its meta.
    fn pair_session(
        root: &Root,
        profiles: &str,
        seat: (&str, Option<&str>),
        peer: (&str, Option<&str>),
    ) -> (PathBuf, String) {
        let dir = root.0.join("demo");
        assert!(std::fs::create_dir_all(&dir).is_ok(), "a session dir");
        let config = dir.join("config");
        assert!(std::fs::write(&config, profiles).is_ok(), "the config");
        let client = |slot: &str, label: Option<&str>| {
            label.map_or_else(String::new, |label| format!("client.{slot}={label}\n"))
        };
        let meta = format!(
            "session=demo\nconfig={}\nseat.spawned.0=x-nav\nprofile.spawned.0=own\nagent_bin.spawned.0={}\n{}\
             seat.spawned.1=x\nprofile.spawned.1=peer\nagent_bin.spawned.1={}\n{}",
            config.display(),
            seat.0,
            client("spawned.0", seat.1),
            peer.0,
            client("spawned.1", peer.1),
        );
        (dir, meta)
    }

    /// A seat whose recorded client swaps its profile's binary for another
    /// tool's serves the CLIENT's tool's provider, while that tool runs (#305).
    #[test]
    fn a_recorded_client_that_swaps_the_tool_decides_what_the_seat_serves() {
        use crate::tool::Provider::{Anthropic, OpenAI};
        let root = Root(std::env::temp_dir().join(format!("ae-leg-client-{}", std::process::id())));
        let profiles =
            "[clients]\nalt = codex\n[profiles]\nown = claude --pg own\npeer = claude --pg peer\n";
        let read = |seat| {
            let (dir, meta) = pair_session(&root, profiles, seat, ("claude", None));
            let parsed = crate::meta::Meta::parse(&meta);
            pair_of(&dir, meta.as_bytes(), parsed.roster(), false, "spawned.0")
                .map(|pair| (pair.seat_serves, pair.partner_serves))
        };
        assert_eq!(
            read(("codex", Some("alt"))),
            Some((Some(OpenAI), Some(Anthropic)))
        );
        assert_eq!(
            read(("claude", None)),
            Some((Some(Anthropic), Some(Anthropic)))
        );
        // The client names codex, the row records claude running: unproven.
        assert_eq!(read(("claude", Some("alt"))), Some((None, Some(Anthropic))));
    }

    /// Under the lifecycle lock the lead pair is read from the LOCKED meta's
    /// layout: in a lead-pair session `main` is held off its partner's
    /// provider, and in any other layout `main` and `worker.0` are no pair and
    /// nothing is held (#305).
    #[test]
    fn the_lifecycle_gate_reads_the_lead_pair_from_the_locked_meta() {
        let root = Root(std::env::temp_dir().join(format!("ae-leg-lead-{}", std::process::id())));
        let dir = root.0.join("demo");
        assert!(std::fs::create_dir_all(&dir).is_ok(), "a session dir");
        let config = dir.join("config");
        let profiles = "[profiles]\nown = claude --pg own\npeer = codex -m gpt-6\n";
        assert!(std::fs::write(&config, profiles).is_ok(), "the config");
        let meta = |layout: &str| {
            format!(
                "session=demo\nconfig={}\nlayout={layout}\nseat.main=lead\nprofile.main=own\n\
                 agent_bin.main=claude\nseat.worker.0=colead\nprofile.worker.0=peer\n\
                 agent_bin.worker.0=codex\n",
                config.display()
            )
        };
        let gate = |layout: &str, command: &str| {
            pair_changed(&dir, meta(layout).as_bytes(), "main", command)
        };
        let held = gate("lead-pair", "codex -m gpt-6.1").unwrap_or_default();
        assert!(held.contains("partner colead serves OpenAI"), "{held:?}");
        assert_eq!(gate("lead-pair", "claude --pg other"), None);
        assert_eq!(gate("vertical", "codex -m gpt-6.1"), None);
    }

    /// Under the lifecycle lock, a pair already collapsed on one provider lets
    /// a move go on only to a destination PROVEN on some provider: the same
    /// proof the chooser and the `stays` label use (#305 R7).
    #[test]
    fn the_lifecycle_gate_holds_a_collapsed_pair_whose_destination_proves_nothing() {
        let root = Root(std::env::temp_dir().join(format!("ae-leg-gate-{}", std::process::id())));
        let profiles = "[profiles]\nown = claude --pg own\npeer = claude --pg peer\n";
        let (dir, meta) = pair_session(&root, profiles, ("claude", None), ("claude", None));
        let gate = |command: &str| pair_changed(&dir, meta.as_bytes(), "spawned.0", command);
        assert_eq!(gate("claude --pg other"), None);
        assert_eq!(gate("codex -m gpt-6"), None);
        let passed: Vec<&str> = ["opencode --model x", "agy --model gemini-4", "codex --oss"]
            .into_iter()
            .filter(|unproven| {
                !gate(unproven).is_some_and(|held| {
                    held.contains("the destination serves an unproven provider")
                })
            })
            .collect();
        assert!(passed.is_empty(), "unproven yet not held: {passed:?}");
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

    #[test]
    fn a_headroom_summary_keeps_its_trigger_inside_the_bound_and_a_limit_one_is_untouched() {
        let cause = "headroom 95% weekly_all 7d";
        assert_eq!(tagged("from a to b", ""), "from a to b");
        assert_eq!(
            tagged("from a to b", cause),
            "from a to b (headroom 95% weekly_all 7d)"
        );
        let long = tagged(&"x".repeat(400), cause);
        assert_eq!(long.chars().count(), SUMMARY_CHARS);
        assert!(long.ends_with(" (headroom 95% weekly_all 7d)"), "{long}");
        let skipped = [
            ("a".to_owned(), Skip::NoRoom),
            ("b".to_owned(), Skip::LeftOnHeadroom),
        ];
        assert_eq!(attempt_note("", &skipped), "");
        assert_eq!(attempt_note(cause, &[]), " (headroom 95% weekly_all 7d)");
        assert_eq!(
            attempt_note(cause, &skipped),
            " (headroom 95% weekly_all 7d); passed over a (no headroom), b (left on headroom)"
        );
    }

    /// The trigger's headroom arm: due only with no limit drawn, the account
    /// still near the threshold and a proven idle frame; a candidate without
    /// room is passed over by name, and a drawn limit takes the limit spelling.
    #[test]
    #[allow(clippy::too_many_lines, reason = "one table of the arm's rows")]
    fn the_trigger_moves_on_headroom_only_while_near_idle_and_unlimited() {
        use crate::autoreseat::{Frame, HEADROOM_ACTION, Pane, Settings, Switch, Window};
        let key = Timestamp::parse(KEY).expect("the key parses");
        let record = |since: i64, action: &str, rest: &str| {
            Event::parse_line(&format!(
                r#"{{"ts":"{}","actor":"watchdog","action":"{action}","target":"scout"{rest}}}"#,
                Timestamp::from_epoch(key.epoch() + since)
            ))
            .expect("a well-formed record")
        };
        let crossed = [record(
            0,
            HEADROOM_ACTION,
            r#","summary":"headroom 96% weekly_all 7d""#,
        )];
        let limit = [record(0, "limit", "")];
        let settings = |headroom_at| Settings {
            switch: Switch::On,
            sessions: None,
            grace_secs: 0,
            map: vec![("sol6x".to_owned(), vec!["opus55x".to_owned()])],
            headroom_at,
            headroom_note: None,
            lead: crate::autoreseat::Lead::default(),
            notes: Vec::new(),
        };
        let seat = Seat {
            session: "aedev",
            slot: "spawned.3",
            agent: "scout",
            profile: "sol6x",
            orchestrator: false,
        };
        let sight = |frame, limited, near| Sight {
            pane: Pane {
                frame,
                human_prompt: false,
                client_input: None,
            },
            limited,
            near,
        };
        let now = Timestamp::from_epoch(key.epoch() + 10);
        let at = |judged: f64| {
            move |list: &[String], _: (&[(String, Timestamp)], &[String])| {
                list.iter()
                    .map(|profile| Candidate {
                        profile: profile.clone(),
                        provenance: Provenance::Declared,
                        configured: true,
                        peer_latched: false,
                        windows: vec![Window {
                            judged,
                            critical: judged >= 95.0,
                            long: true,
                            observed_at: now.epoch(),
                            resets_at: None,
                            status: crate::quota::Status::Fresh,
                            label: "weekly_all 7d".to_owned(),
                        }],
                        left_at: None,
                        left_on_headroom: false,
                    })
                    .collect()
            }
        };
        let near = sight(Frame::Clear, false, true);
        assert_eq!(
            plan(&settings(Some(95)), &seat, &near, &crossed, at(10.0), now),
            Plan::Attempt {
                key,
                to: "opus55x".to_owned(),
                note: " (headroom 96% weekly_all 7d)".to_owned(),
                cause: "headroom 96% weekly_all 7d".to_owned(),
            }
        );
        assert_eq!(
            plan(&settings(Some(95)), &seat, &near, &crossed, at(96.0), now),
            Plan::Refuse {
                key,
                class: CauseClass::NoCandidate,
                why: "no-candidate: refused: no usable candidate: opus55x (no headroom) \
                      (headroom 96% weekly_all 7d)"
                    .to_owned(),
            }
        );
        for (settings, sight, events, why) in [
            (settings(None), near, &crossed[..], "threshold off"),
            (
                settings(Some(95)),
                sight(Frame::Clear, false, false),
                &crossed[..],
                "relieved since",
            ),
            (
                settings(Some(95)),
                sight(Frame::Unproven, false, true),
                &crossed[..],
                "unproven frame",
            ),
            (settings(Some(95)), near, &[][..], "no episode"),
            (
                settings(Some(95)),
                sight(Frame::Clear, true, true),
                &crossed[..],
                "a drawn limit asks the limit episode",
            ),
        ] {
            assert!(
                matches!(
                    plan(&settings, &seat, &sight, events, at(10.0), now),
                    Plan::Decline(_)
                ),
                "{why}"
            );
        }
        // An attempt in flight of either kind declines a second one.
        let mut flying = crossed.to_vec();
        flying.push(record(
            5,
            crate::autoreseat::ATTEMPT_ACTION,
            &format!(r#","ref":"{KEY}""#),
        ));
        flying.extend(limit.iter().cloned());
        for sight in [sight(Frame::Clear, true, true), near] {
            assert!(matches!(
                plan(&settings(Some(95)), &seat, &sight, &flying, at(10.0), now),
                Plan::Decline(_)
            ));
        }
        // A drawn limit moves by the limit episode, with its own spelling and
        // its own chooser: the 96% candidate a headroom move passes over.
        let mut both = crossed.to_vec();
        both.extend(limit.iter().cloned());
        assert_eq!(
            plan(
                &settings(Some(95)),
                &seat,
                &sight(Frame::Unproven, true, true),
                &limit,
                at(96.0),
                now
            ),
            Plan::Attempt {
                key,
                to: "opus55x".to_owned(),
                note: String::new(),
                cause: String::new(),
            }
        );
        assert!(matches!(
            plan(
                &settings(Some(95)),
                &seat,
                &sight(Frame::Clear, true, true),
                &both,
                at(96.0),
                now
            ),
            Plan::Attempt { ref cause, .. } if cause.is_empty()
        ));
    }

    /// Synthetic profiles only. `spark` and `wonder` share one Muse allowance,
    /// `fab` and `opu` one Claude account; `sol` and `grk` are other tools.
    const COVER_CONFIG: &str = "[profiles]\n\
        fab = claude --model fable\n\
        opu = claude --model opus\n\
        sol = codex --model sol\n\
        spark = muse --model spark\n\
        wonder = muse --model wonder\n\
        grk = grok --model grk\n\
        dark = /bin/sh -c :\n\
        bad = \"touch m; tail -f /dev/null\"\n\
        \n[workspace]\nauto_reseat = on\n\
        \n[auto_reseat]\nspark = grk\nsol = fab\ndark = fab\n";

    const COVER_HOME: &str = "/ae-cover-test-home";

    fn cover_at(text: &str) -> Option<Vec<crate::autoreseat::ProfileCover>> {
        let cfg = crate::config::parse_identity(text).expect("a synthetic identity config");
        let settings = crate::autoreseat::settings_in(Path::new("config"), text);
        covers(&cfg, &settings, Some(Path::new(COVER_HOME)))
    }

    fn verdicts(found: &[crate::autoreseat::ProfileCover]) -> Vec<(&str, &Cover)> {
        found
            .iter()
            .map(|one| (one.profile.as_str(), &one.cover))
            .collect()
    }

    #[test]
    fn every_profile_key_is_judged_and_an_unreadable_command_is_unproven_never_omitted() {
        let found = cover_at(COVER_CONFIG).expect("auto reseat is on");
        assert_eq!(
            found
                .iter()
                .map(|one| one.profile.as_str())
                .collect::<Vec<_>>(),
            ["fab", "opu", "sol", "spark", "wonder", "grk", "dark", "bad"],
            "config order, one verdict per key"
        );
        let bad = found.iter().find(|one| one.profile == "bad").expect("bad");
        assert_eq!(bad.cover, Cover::Unproven(Doubt::Own));
        assert_eq!(bad.scope, Scope::Unreadable);
    }

    #[test]
    fn the_verdicts_follow_the_declared_row_then_the_family_across_tools_and_accounts() {
        let found = cover_at(COVER_CONFIG).expect("auto reseat is on");
        let want = [
            ("fab", Cover::Bare),
            ("opu", Cover::Bare),
            ("sol", Cover::Exit("fab".to_owned())),
            ("spark", Cover::Exit("grk".to_owned())),
            ("wonder", Cover::Bare),
            ("grk", Cover::Bare),
            ("dark", Cover::Unproven(Doubt::Own)),
            ("bad", Cover::Unproven(Doubt::Own)),
        ];
        assert_eq!(
            verdicts(&found),
            want.iter()
                .map(|(name, cover)| (*name, cover))
                .collect::<Vec<_>>(),
            "claude siblings share an account; muse spark->grok leaves the allowance; \
             an unnamed tool and a command that does not lex prove nothing"
        );
        let label = |name: &str| {
            found
                .iter()
                .find(|one| one.profile == name)
                .map(|one| one.scope.label())
                .unwrap_or_default()
        };
        assert_eq!(label("fab"), format!("claude · {COVER_HOME}/.claude"));
        assert_eq!(label("wonder"), "muse · shared");
        let suggested = |name: &str| {
            found
                .iter()
                .find(|one| one.profile == name)
                .map(|one| one.suggested.clone())
                .unwrap_or_default()
        };
        assert_eq!(
            suggested("fab"),
            ["grk"],
            "from the declared rows, off its account"
        );
        assert_eq!(
            suggested("wonder"),
            ["grk", "fab"],
            "the second is another tool"
        );
        assert_eq!(suggested("grk"), ["fab"], "never the profile itself");
        assert!(
            suggested("sol").is_empty(),
            "a covered profile needs no suggestion"
        );
    }

    /// `[auto_reseat]` is keyed by profile: a row covers its own key and
    /// nobody else, even a profile on the same account.
    #[test]
    fn one_added_row_covers_that_profile_alone() {
        let before = cover_at(COVER_CONFIG).expect("auto reseat is on");
        let after = cover_at(&format!("{COVER_CONFIG}wonder = grk\n")).expect("auto reseat is on");
        let bare = |found: &[crate::autoreseat::ProfileCover]| {
            found
                .iter()
                .filter(|one| one.cover == Cover::Bare)
                .map(|one| one.profile.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(bare(&before), ["fab", "opu", "wonder", "grk"]);
        assert_eq!(
            bare(&after),
            ["fab", "opu", "grk"],
            "wonder leaves the list; its neighbours on the same account stay on it"
        );
        let wonder = after
            .iter()
            .find(|one| one.profile == "wonder")
            .expect("wonder");
        assert_eq!(wonder.cover, Cover::Exit("grk".to_owned()));
    }

    /// A tool ae names whose account it cannot prove has no verdict of its own
    /// — but a profile on ANOTHER tool leaves any account, so the line that
    /// would cover it is still provable and still printed.
    #[test]
    fn an_unprovable_own_account_still_names_the_cross_tool_exit_that_would_cover_it() {
        let text = "[profiles]\na = claude --model x\nb = codex --model y\n\
                    \n[workspace]\nauto_reseat = on\n";
        let judge = |text: &str| {
            let cfg = crate::config::parse_identity(text).expect("a synthetic identity config");
            let settings = crate::autoreseat::settings_in(Path::new("config"), text);
            covers(&cfg, &settings, None).expect("auto reseat is on")
        };
        let found = judge(text);
        let a = found.iter().find(|one| one.profile == "a").expect("a");
        assert_eq!(a.cover, Cover::Unproven(Doubt::Own), "no HOME, no store");
        assert_eq!(a.suggested, ["b"], "another tool is off any account");
        let after = judge(&format!("{text}\n[auto_reseat]\na = b\n"));
        let a = after.iter().find(|one| one.profile == "a").expect("a");
        assert_eq!(
            a.cover,
            Cover::Exit("b".to_owned()),
            "the printed line covers it, as the doctor row and the nudge promise"
        );
    }

    #[test]
    fn nothing_is_judged_while_auto_reseat_is_off() {
        let off = COVER_CONFIG.replace("auto_reseat = on", "auto_reseat = off");
        assert_eq!(cover_at(&off), None);
        let absent = COVER_CONFIG.replace("auto_reseat = on\n", "");
        assert_eq!(cover_at(&absent), None);
        let cfg = crate::config::parse_identity(&off).expect("identity");
        let settings = crate::autoreseat::settings_in(Path::new("config"), &off);
        assert_eq!(
            cover_warning(&cfg, &settings, "wonder", Some(Path::new(COVER_HOME))),
            None
        );
    }

    #[test]
    fn the_nudge_is_for_the_chosen_profile_only() {
        let cfg = crate::config::parse_identity(COVER_CONFIG).expect("identity");
        let settings = crate::autoreseat::settings_in(Path::new("config"), COVER_CONFIG);
        let home = Some(Path::new(COVER_HOME));
        assert_eq!(
            cover_warning(&cfg, &settings, "wonder", home).as_deref(),
            Some(
                "ae: warning: profile 'wonder' has no auto-reseat exit off its account — \
                 a usage limit would leave the seat stuck; add under [auto_reseat]: \
                 wonder = grk, fab"
            )
        );
        assert_eq!(cover_warning(&cfg, &settings, "sol", home), None, "covered");
        assert_eq!(
            cover_warning(&cfg, &settings, "ghost", home),
            None,
            "no such profile"
        );
        assert!(
            cover_warning(&cfg, &settings, "bad", home)
                .is_some_and(|line| line.contains("its own account is not provable")),
            "an unreadable command is named, not skipped"
        );
    }

    #[cfg(test)]
    mod keepconv_spec {
        #[test]
        fn automatic_kept_outcome_records_kept_not_carried_or_seeded() {
            let actual = super::super::outcome(
                crate::reseat::Ended::Moved {
                    fate: crate::autoreseat::Fate::Kept,
                },
                "episode",
                "old",
                "new",
                "",
            );
            assert_eq!(
                actual,
                (
                    crate::autoreseat::DONE_ACTION,
                    "old".to_owned(),
                    "to new, kept".to_owned()
                )
            );
        }
    }
}
