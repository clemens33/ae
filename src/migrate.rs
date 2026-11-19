//! `meta_version`, and the chain that steps a session meta forward.
//!
//! Every session meta carries `meta_version=<N>`. The row is written at launch
//! and it is the ONE fact that says which shape the rest of the file is in.
//!
//! A meta without it is not automatically a refusal, and getting that wrong
//! would have been expensive: the row is younger than the sessions, so on the
//! machine this was written for, 28 of 28 live sessions had none. What they DO
//! have is `schema=2`, which is the same statement in the older word — so such
//! a meta is PLACED at 2 and the row is stamped in on first touch, silently.
//! Only a meta that says neither has told us nothing, and that one is refused
//! rather than guessed at, because a reader that guesses a shape acts on a
//! session it has not understood.
//!
//! The chain steps N -> N+1, one [`Step`] at a time, and stops at
//! [`CURRENT`]. It was built empty on purpose, so that the first real shape
//! change would be one step with a fixture beside it rather than a rewrite of
//! every reader; [`tag_priors`] is that first step, and it walks a v2 meta to 3
//! by telling every predecessor conversation which TOOL owns it.
//!
//! A `schema=2`-only meta is therefore no longer merely STAMPED: it is placed
//! at 2 by [`PRE_CHAIN`] and then STEPPED, so its report reads "migrated from
//! 2 to 4" rather than "stamped". [`Stepped::Stamped`] stays for the day
//! `CURRENT` and `PRE_CHAIN` meet again.
//!
//! It runs wherever the core TOUCHES a session:
//!
//! * resume/attach ([`crate::session_launch`]) — a refusal stops the launch, and
//!   a placed meta is stamped there, which is the first touch most sessions get;
//! * `stop` and `end` ([`crate::lifecycle`]) — a refusal is REPORTED and the
//!   operation continues, because a session that cannot be migrated is exactly
//!   the one an operator needs to be able to tear down;
//! * `ae upgrade` / `ae _install` ([`crate::install::publish`]) — the whole
//!   sweep below, behind ONE census of every root ([`plan_fleet`]) that stops
//!   the upgrade before anything moves when a session's meta is present and
//!   unreadable or a session claiming the orchestrator role cannot be relinked,
//!   running or not. A refusal on a running session aborts before the command
//!   link is repointed; a stopped unplaceable ordinary session is reported and
//!   skipped.
//!   Named instance roots under `<root>/instances` follow the default one,
//!   stricter (`onto_fleet`): a root in any doubt is skipped untouched, one that
//!   fails part-way is reported, and either freezes the version sweep.
//!
//! WHAT THIS DOES NOT PROMISE. There is no quiescent cut across the sweep, the
//! command repoint and the version prune: each session's lifecycle lock is
//! taken and released in turn, and a launch that started on the OLD core
//! before the repoint can publish old-core helpers after the prune has read
//! its keep-set. The human's ruling was explicit — no locking beyond the
//! existing session lock — and the alternative is a global lock every launch
//! would have to wait behind. The window is the seconds a publish takes, the
//! prune re-reads every meta rather than trusting the sweep, and the loser is
//! a session naming a deleted core, which `ae doctor --refresh` repairs.
//!
//! The publisher lock ([`crate::install::LOCK`]) excludes another publish, but
//! only one that TAKES it: a core older than that lock does not, so during the
//! release that introduces it two publishes on one machine can still overlap.
//! The floor under that is [`prune_versions`], which re-reads the command link
//! immediately before it deletes anything and never removes the directory that
//! link resolves into. A stale keep-set can then cost disk space; it cannot
//! cost the `ae` command.
//!
//! LOCK ORDER, because two locks meet here. The sweep takes the per-session
//! LIFECYCLE lock (so an end or a launch cannot land inside a migration) and
//! then, inside [`session`] and [`repoint`], the META lock. `stop` and `end`
//! take them in the same order. The launch takes the meta lock alone, through
//! [`session`], and RELEASES it before it takes the lifecycle lock of its own —
//! so nothing ever holds one while waiting for the other, and the meta lock,
//! which both the chain and the repoint take, is what actually keeps a meta
//! from being read half-rewritten.
//!
//! The upgrade sweep is the reason the module owns more than the chain: after
//! the new version directory is on disk and before `~/.local/bin/ae` names it,
//! Every PLACEABLE session — running or stopped — is stepped, re-pointed at the
//! new core and re-linked, and the two daemons of a running one are restarted
//! so they run the core the rest of the install just became. A stopped session
//! the chain cannot place is reported and skipped untouched. Agent panes are
//! never touched: they run the agent tool, not ae.

use std::path::{Path, PathBuf};

use crate::inventory::ServerId;
use crate::meta::{Selector, ServerSelector};

/// The row that carries the shape, in every session meta.
pub const KEY: &str = "meta_version";

/// The shape this core reads and writes.
pub const CURRENT: u32 = 4;

/// The shape a meta is at when it declares no [`KEY`] but does declare
/// `schema=2`.
///
/// THE PRE-CHAIN v2 SHAPE, and the reason this constant exists. The row was
/// introduced by the chain, so every session that existed before it has none —
/// 28 of 28 on the machine this was written on. Refusing those would have meant
/// that installing the release which added the chain made every running session
/// unresumable, which is the exact opposite of what the chain is for. A meta
/// that says `schema=2` has already told us its shape; it simply said it with
/// the older word. So it is PLACED at 2 and the row is stamped in on first
/// touch, silently. Only a meta that says NEITHER is the pre-version past.
const PRE_CHAIN: u32 = 2;

/// The key that carried the shape before [`KEY`] existed.
const SCHEMA_KEY: &str = "schema";

/// The one value of [`SCHEMA_KEY`] that places a meta.
const SCHEMA_V2: &str = "2";

/// One link of the chain: a meta at `from`, rewritten to `from + 1`.
///
/// The `apply` never writes the version row — [`migrate`] stamps it after the
/// step returns, so a step cannot forget to and cannot disagree with the loop
/// about where it landed.
struct Step {
    /// The version this step consumes.
    from: u32,
    /// The rewrite, meta text in and meta text out, or one line saying why not.
    apply: fn(&str) -> Result<String, String>,
}

/// The chain, in order.
///
/// A new step is `Step { from: N, apply: … }` plus a fixture in
/// `tests/it/migrate.rs` that carries a real meta at N and asserts what N+1
/// makes of it.
const STEPS: &[Step] = &[
    Step {
        from: 2,
        apply: tag_priors,
    },
    Step {
        from: 3,
        apply: keep_v3_shape,
    },
];

/// 2 -> 3: give every UNTAGGED predecessor the tool that owns it.
///
/// A predecessor row used to be bare conversation ids, and every reader looked
/// one up in the store of the tool the slot names NOW. That was right while a
/// seat could only ever have run one tool. A seat can now be moved to another
/// one, so an id has to say whose it is, and the ids already on disk have to be
/// given the answer before anything writes a mixed row: this step is that
/// answer, and it is the ONE place the legacy rule — an untagged id belongs to
/// the tool of the slot at read time — is applied rather than assumed.
///
/// Only the predecessor rows move. A slot that records no binary, a row that is
/// already fully tagged and a row this reader cannot judge are all left exactly
/// as they are: the migration adds a fact it can derive and destroys nothing.
#[allow(
    clippy::unnecessary_wraps,
    reason = "a Step's apply is fallible by type; this one derives its fact or leaves the row alone"
)]
fn tag_priors(text: &str) -> Result<String, String> {
    let parsed = crate::meta::Meta::parse(text);
    let mut out = text.to_owned();
    for entry in parsed.roster() {
        let tool = entry.binary.as_deref().unwrap_or_default();
        let raw = parsed.harness_session_prior(&entry.slot);
        if let Some(row) = crate::meta::priors_tagged(&raw, tool) {
            out = crate::meta::rewritten(
                &out,
                &format!(
                    "{}{}",
                    crate::meta::HARNESS_SESSION_PRIOR_PREFIX,
                    entry.slot
                ),
                Some(&row),
            );
        }
    }
    Ok(out)
}

/// 3 -> 4: nothing moves. The seat-dir rows (`work_dir.<slot>`) are additive
/// and every reader tolerates their absence, so the step is EMPTY on purpose —
/// the bump itself is the guard: a CURRENT=3 core rebuilds the meta on resume
/// and would silently drop rows it never enlisted, so it must refuse a v4
/// meta as Ahead instead.
#[allow(
    clippy::unnecessary_wraps,
    reason = "a Step's apply is fallible by type; this one keeps every row"
)]
fn keep_v3_shape(text: &str) -> Result<String, String> {
    Ok(text.to_owned())
}

/// Why a meta could not be brought to [`CURRENT`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No `meta_version` row at all: the pre-version past.
    Absent,
    /// The row is there and unusable — named twice, or not a plain decimal.
    Unreadable(String),
    /// A version the chain has no step out of.
    NoStep(u32),
    /// A version NEWER than this core's, which no step can walk backwards.
    Ahead(u32),
    /// A step refused the meta it was handed.
    StepFailed { from: u32, why: String },
    /// There is no meta at all — a directory under `sessions` that is not a
    /// session. Kept apart from [`Refusal::Io`] because every caller treats it
    /// differently: nothing to migrate, and someone else's error to report.
    Missing,
    /// The meta could not be read or written.
    Io(String),
}

impl Refusal {
    /// The operator-facing line, naming the session it is about.
    ///
    /// [`Refusal::Absent`] carries the same fresh-start wording a v1 roster
    /// earns: there is one way forward from a meta ae cannot
    /// place, and it is a new session.
    #[must_use]
    pub fn line(&self, session: &str) -> String {
        match self {
            Self::Absent => format!(
                "session {session:?} records no {KEY} — it pre-dates the migration chain. \
                 ae does not guess a meta's shape: end the session and start a fresh one \
                 (`ae end {session}`, then `ae {session}`)."
            ),
            Self::Unreadable(why) => {
                format!("session {session:?} has an unusable {KEY} row: {why}.")
            }
            Self::NoStep(from) => format!(
                "session {session:?} records {KEY}={from} and this ae has no step out of it \
                 (it reads {CURRENT})."
            ),
            Self::Ahead(from) => format!(
                "session {session:?} records {KEY}={from}, which is newer than this ae reads \
                 ({CURRENT}) — upgrade ae rather than downgrading the session."
            ),
            Self::StepFailed { from, why } => {
                format!("session {session:?} could not be stepped from {KEY}={from}: {why}.")
            }
            Self::Missing => format!("session {session:?} has no meta."),
            Self::Io(why) => format!("session {session:?}: {why}."),
        }
    }
}

/// What the chain did to a meta that was not already current in writing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stepped {
    /// It declared no version, `schema=2` placed it at [`PRE_CHAIN`], and the
    /// row was written in. Nothing else about the meta changed.
    Stamped,
    /// It declared `from`, and the chain walked it to [`CURRENT`].
    From(u32),
}

/// A meta the chain wrote, and what it did to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migrated {
    /// What was done.
    pub what: Stepped,
    /// The whole meta, at [`CURRENT`].
    pub text: String,
}

/// The version a meta is at, from the two keys that can say so.
///
/// ONE OWNER for the placement rule, because two readers apply it: the chain,
/// which acts on the answer, and `ae list`, which only reports it. A marker
/// that called a `schema=2` session "old" while the chain called it current
/// would be two answers to one question.
#[must_use]
pub fn placed(row: Option<&str>, schema: Option<&str>) -> Option<u32> {
    match row {
        Some(value) => version_of_row(value).ok(),
        None => (schema == Some(SCHEMA_V2)).then_some(PRE_CHAIN),
    }
}

/// A `meta_version` value, parsed. A plain decimal and nothing else: a `2 ` or
/// a `v2` is a writer this reader does not know, not a version it may round
/// down to.
fn version_of_row(value: &str) -> Result<u32, Refusal> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Refusal::Unreadable(format!("{value:?} is not a version")));
    }
    value
        .parse()
        .map_err(|_| Refusal::Unreadable(format!("{value:?} is out of range")))
}

/// Step `text` to [`CURRENT`], or say why it cannot be.
///
/// `Ok(None)` is the ordinary answer: the meta already declares the current
/// version and nothing needs writing.
///
/// ```
/// let current = ae::migrate::migrate("mode=local\nmeta_version=4\n");
/// assert_eq!(current, Ok(None));
/// assert!(ae::migrate::migrate("mode=local\n").is_err());
/// ```
///
/// # Errors
///
/// A [`Refusal`]: an absent, duplicated or unparsable row, a version with no
/// step, a version ahead of this core, or a step that refused the meta.
pub fn migrate(text: &str) -> Result<Option<Migrated>, Refusal> {
    let (from, written) = declared(text)?;
    if from > CURRENT {
        return Err(Refusal::Ahead(from));
    }
    if from == CURRENT {
        if written {
            return Ok(None);
        }
        // PLACED, not stepped: `schema=2` already said this shape with the
        // older word, so the row is written in and NOTHING else about the meta
        // is touched.
        return Ok(Some(Migrated {
            what: Stepped::Stamped,
            text: crate::meta::rewritten(text, KEY, Some(&CURRENT.to_string())),
        }));
    }
    let mut at = from;
    let mut carried = text.to_owned();
    while at < CURRENT {
        let Some(step) = STEPS.iter().find(|step| step.from == at) else {
            return Err(Refusal::NoStep(at));
        };
        carried = (step.apply)(&carried).map_err(|why| Refusal::StepFailed { from: at, why })?;
        at += 1;
        carried = crate::meta::rewritten(&carried, KEY, Some(&at.to_string()));
    }
    Ok(Some(Migrated {
        what: Stepped::From(from),
        text: carried,
    }))
}

/// The version `text` is at, and whether it SAYS so in a [`KEY`] row.
///
/// `false` means the version came from `schema=2` and the row has still to be
/// written in.
fn declared(text: &str) -> Result<(u32, bool), Refusal> {
    let mut found: Option<&str> = None;
    let mut schema: Option<&str> = None;
    let mut schemas = 0_u32;
    for record in text.split('\n') {
        // One trailing carriage return is line ENDING, not value — the same
        // rule `Meta::parse` applies. Without it a CRLF meta parses as `2\r`
        // here and as `2` there, so `ae list` would call a session current
        // while every resume refused it as unreadable.
        let record = record.strip_suffix('\r').unwrap_or(record);
        let Some((key, value)) = record.split_once('=') else {
            // A bare `meta_version` with no `=` is a row the writer MEANT to
            // give and this reader cannot take. That is not the pre-version
            // past, and it must not earn the fresh-start message.
            if record == KEY {
                return Err(Refusal::Unreadable(format!("{KEY} carries no value")));
            }
            continue;
        };
        if key == SCHEMA_KEY {
            schemas += 1;
            schema = Some(value);
            continue;
        }
        if key != KEY {
            continue;
        }
        if found.is_some() {
            return Err(Refusal::Unreadable(format!(
                "{KEY} is named more than once"
            )));
        }
        found = Some(value);
    }
    if let Some(value) = found {
        return version_of_row(value).map(|version| (version, true));
    }
    // A meta that names `schema` twice has not said which one counts, and
    // `Meta::parse` invalidates the field for exactly that reason. It places
    // nothing, so such a meta falls through to the refusal below.
    let schema = schema.filter(|_| schemas == 1);
    placed(None, schema)
        .map(|version| (version, false))
        .ok_or(Refusal::Absent)
}

/// Run the chain over the session directory at `dir`, publishing the result
/// under the meta lock when it stepped.
///
/// `Ok(None)` means the meta was already current and nothing was written.
///
/// # Errors
///
/// A [`Refusal`] — the chain's, or the read/write that could not happen.
pub fn session(dir: &Path) -> Result<Option<Stepped>, Refusal> {
    let _held = crate::meta::lock(dir).map_err(|why| Refusal::Io(format!("meta.lock: {why}")))?;
    let text = crate::meta::read_base(&dir.join(crate::meta::FILE)).map_err(|why| {
        if why.kind() == std::io::ErrorKind::NotFound {
            Refusal::Missing
        } else {
            Refusal::Io(format!("meta could not be read: {why}"))
        }
    })?;
    let Some(migrated) = migrate(&text)? else {
        return Ok(None);
    };
    crate::meta::publish_locked(dir, &migrated.text)
        .map_err(|why| Refusal::Io(format!("meta could not be written: {}", why.cause())))?;
    Ok(Some(migrated.what))
}

/// Run the chain where a refusal must NOT stop the caller — `stop` and `end`.
///
/// Returns the line to report, or nothing when there was nothing to say. A
/// session an operator is tearing down is the one case where an unmigratable
/// meta must not become a wall.
#[must_use]
pub fn session_noted(dir: &Path, name: &str) -> Option<String> {
    match session(dir) {
        // A stamp is not news. It is the pre-chain v2 shape being written down
        // in the words this ae uses, on a session that was always version 2 —
        // telling the operator about it on every stop and every end would be
        // noise about a no-op.
        Ok(None | Some(Stepped::Stamped)) | Err(Refusal::Missing) => None,
        Ok(Some(Stepped::From(from))) => Some(format!(
            "note: session {name:?} was migrated from {KEY}={from} to {CURRENT}."
        )),
        Err(refusal) => Some(format!("note: {}", refusal.line(name))),
    }
}

// ─── the upgrade sweep ───────────────────────────────────────────────────

/// The two rows a session records about the core it was built against.
const CORE_ROWS: [&str; 2] = ["ae_core", "ae_core_version"];

/// The `ae_path` row some metas still carry. REWRITTEN where a session already
/// has one and never introduced: an upgrade that left it pointing into a
/// version directory it is about to delete would leave a session naming a
/// binary that is gone.
const LEGACY_PATH_ROW: &str = "ae_path";

/// [`crate::lifecycle::census`] with its one absent case read as empty: a state
/// root with no `sessions/` directory has no sessions, and saying so is not a
/// guess. That case is TYPED, not sniffed from an error kind — an entry that
/// vanishes mid-walk raises `NotFound` too, and reading THAT as "no sessions"
/// would hand the version sweep a keep-set missing a live session's core.
///
/// # Errors
///
/// Whatever the census hit. Only the absent root is not an error.
fn taken(root: &Path) -> std::io::Result<Vec<String>> {
    crate::lifecycle::census(root).map(Option::unwrap_or_default)
}

/// Whether `name` is live on either server a bash-era session could inhabit.
///
/// This is the fallback when a meta has no usable selector — including a meta
/// that cannot be read at all. Exact-name probes keep an unrelated session on
/// either server from turning this into a refusal.
fn running_on_legacy_server(name: &str) -> bool {
    let owned = ServerId::Selected(Selector::Name(crate::doors::DEFAULT_SERVER_NAME.to_owned()));
    crate::transport::session_exists(&owned, name)
        || crate::transport::session_exists(&crate::doors::historical_server(), name)
}

/// Whether an unplaceable readable meta still has live helpers to protect.
fn unplaceable_is_running(text: &str, name: &str) -> bool {
    match crate::meta::Meta::parse(text).server_selector() {
        ServerSelector::Positive(selector) => {
            crate::transport::session_exists(&ServerId::Selected(selector), name)
        }
        ServerSelector::Missing | ServerSelector::Ambiguous => running_on_legacy_server(name),
    }
}

/// The note for a stopped session the chain cannot place.
fn skipped_refusal(name: &str, refusal: &Refusal) -> String {
    match refusal {
        Refusal::Absent => format!(
            "skipped {name}: it pre-dates the migration chain and is not running — end it (ae end {name})"
        ),
        Refusal::Ahead(_) => format!(
            "skipped {name}: meta newer than this ae, left untouched — upgrade ae to resume it"
        ),
        _ => format!(
            "skipped {name}: {}, not running — end it (ae end {name})",
            refusal.line(name).trim_end_matches('.')
        ),
    }
}

/// Session names a pending rename transaction owns: both endpoints of every
/// pending carrier, plus attributable damaged carriers. Unattributable
/// (digest-shape) damage names no session to skip, so the sweep refuses
/// rather than migrating blind.
fn pending_rename_names(root: &Path) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = Vec::new();
    for intent in crate::rename::pending_intents(root) {
        for name in [intent.old_name(), intent.new_name()] {
            if !names.iter().any(|held| held == name) {
                names.push(name.to_owned());
            }
        }
    }
    for damaged in crate::rename::pending_damaged(root) {
        match (damaged.old, damaged.new) {
            (Some(old), Some(new)) => {
                for name in [old, new] {
                    if !names.iter().any(|held| held == &name) {
                        names.push(name);
                    }
                }
            }
            _ => {
                return Err(format!(
                    "an unattributable damaged rename carrier is pending ({}) — repair or remove it by hand before upgrading",
                    damaged.why
                ));
            }
        }
    }
    Ok(names)
}

/// The skip note for a session a pending rename owns, if any.
fn pending_rename_note(root: &Path, name: &str) -> Option<String> {
    crate::rename::pending_intents(root)
        .into_iter()
        .find(|intent| intent.old_name() == name || intent.new_name() == name)
        .map(|intent| {
            format!(
                "skipped {name}: rename '{}' → '{}' is in progress at phase '{}' — retry 'ae rename {} {}' before upgrading",
                intent.old_name(),
                intent.new_name(),
                intent.phase(),
                intent.old_name(),
                intent.new_name()
            )
        })
        .or_else(|| {
            crate::rename::pending_damaged(root)
                .into_iter()
                .find(|damaged| {
                    damaged.old.as_deref() == Some(name) || damaged.new.as_deref() == Some(name)
                })
                .map(|damaged| {
                    let scope = match (&damaged.old, &damaged.new) {
                        (Some(old), Some(new)) => format!("for '{old}' → '{new}'"),
                        _ => "that names no attributable pair".to_owned(),
                    };
                    format!(
                        "skipped {name}: a damaged rename carrier {scope} is pending ({}) — repair or remove it by hand before upgrading",
                        damaged.why
                    )
                })
        })
}

/// The existing abort line for a meta read failure.
fn unreadable_refusal(name: &str, why: &std::io::Error) -> String {
    Refusal::Io(format!("meta could not be read: {why}")).line(name)
}

/// Bring every placeable session under `root` onto `core`, and restart the
/// daemons of the running ones.
///
/// This is the upgrade's step, and it runs AFTER the new version directory is
/// published and BEFORE the command link names it — so a session that cannot
/// be migrated aborts the upgrade while the old core is still the current one.
/// The exception is an unplaceable session that is not running: it has no live
/// helpers to protect, so it is reported and skipped without touching anything
/// under its directory. A meta without a usable server selector is checked on
/// both servers where bash-era sessions could have lived.
///
/// Per session, under the same per-session lifecycle lock a start or an end
/// takes: the chain, then `ae_core` / `ae_core_version` / `ae_version` (and an
/// `ae_path` where a session has one) rewritten to the new core, then the
/// helper links re-rendered. A RUNNING session then has its watchdog restarted, and the
/// Telegram bridge on its tmux server restarted once per server. Agent panes
/// are never touched.
///
/// # Errors
///
/// The first non-skippable session that could not be migrated, named — nothing
/// after it is attempted.
pub fn onto(root: &Path, core: &Path, version: &str) -> Result<Vec<String>, String> {
    let planned = census_default(root)?;
    let (mut moved, mut stops) = (Vec::new(), false);
    onto_scoped(
        root,
        core,
        version,
        Scope::Default,
        planned,
        &mut moved,
        &mut stops,
    )
}

/// [`onto`] over the default root of a [`FleetPlan`]: its sessions and their
/// verdicts are the census the plan already made.
///
/// # Errors
///
/// As [`onto`].
fn onto_planned(plan: &mut FleetPlan, core: &Path, version: &str) -> Result<Vec<String>, String> {
    let FleetPlan {
        default: (root, planned),
        moved,
        ..
    } = plan;
    let mut stops = false;
    onto_scoped(
        root,
        core,
        version,
        Scope::Default,
        std::mem::take(planned),
        moved,
        &mut stops,
    )
}

// ─── the fleet admission plan ────────────────────────────────────────────

/// What the census judged of one session, for the apply to act on.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Judged {
    /// No meta file: not a session, skipped.
    NoMeta,
    /// A pending rename owns the name: skipped untouched, noted.
    Renaming,
    /// A STOPPED ordinary session the chain cannot place: skipped, noted.
    Skip(String),
    /// The chain can place it: step, repoint, relink.
    Move,
}

/// One session of the one census: what was read of it and what was decided.
/// The apply moves nothing without asking it again, on the bytes it reads
/// under the session's locks.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Planned {
    name: String,
    /// SHA-256 of the meta bytes the census read; `None` when it read none.
    digest: Option<String>,
    /// Whether it records the orchestrator role (`meta_agent`, byte-exact or
    /// damaged) as planned: a session of the protected cohort.
    protected: bool,
    /// Whether it is the seat of its root, which the apply re-seeds.
    seat: bool,
    judged: Judged,
}

/// A named root as the census left it.
#[derive(Debug)]
enum NamedRoot {
    /// A doubt about the root or an ordinary session in it holds the root
    /// untouched; the publish goes on and the version sweep removes nothing.
    Held(String),
    /// Every session read and judged.
    Ready(Vec<Planned>),
}

/// Why a judgement stops the sweep.
#[derive(Debug)]
enum Stop {
    /// The publish itself stops: a present meta that cannot be read, or a
    /// session of the protected cohort that cannot be relinked, in any root.
    Fatal(String),
    /// A named root's own ordinary failure: the root is held.
    Hold(String),
}

/// What the upgrade sweeps, decided ONCE and before anything moves: every
/// session of the default root and of every named root, read a single time,
/// each with its verdict. The sweeps walk THESE verdicts, so the admission
/// and the repoint never disagree about which sessions exist, and a protected
/// session's refusal in any root stops the whole publish before the first
/// repoint.
#[derive(Debug)]
pub(crate) struct FleetPlan {
    /// The default root and its sessions.
    default: (PathBuf, Vec<Planned>),
    /// Each named root proved a real directory: its instance name, the root
    /// and its census.
    named: Vec<(String, PathBuf, NamedRoot)>,
    /// The named entries that could not be proved, as [`fleet`] found them.
    doubts: Vec<(String, String)>,
    /// Every session moved so far, across the roots, for the refusal that
    /// must name them.
    moved: Vec<String>,
}

/// The one census of the whole fleet under `base`, judged.
///
/// A session whose meta is PRESENT but cannot be read, is not UTF-8 or has a
/// malformed line, or one that records the orchestrator role (a `meta_agent`
/// claim, damaged or not) and cannot be placed or relinked, stops the
/// upgrade, in any root, running or not: the seat it may be is the one session
/// whose helpers must move with the command. That holds behind earlier
/// ordinary damage too: a named root's first doubt never ends its scan. A
/// missing meta, and a stopped ordinary session the chain cannot place, keep
/// the sweep's own skips.
///
/// # Errors
///
/// The first session that stops the upgrade, named with the remedy, and that
/// nothing was repointed.
pub(crate) fn plan_fleet(base: &Path) -> Result<FleetPlan, String> {
    let planned = census_default(base)?;
    let Fleet { named, doubts } = fleet(base);
    let mut roots = Vec::with_capacity(named.len());
    for (name, root) in named {
        let census = census_named(&root)?;
        roots.push((name, root, census));
    }
    Ok(FleetPlan {
        default: (base.to_owned(), planned),
        named: roots,
        doubts,
        moved: Vec::new(),
    })
}

/// The default root's sessions, read once and judged.
fn census_default(root: &Path) -> Result<Vec<Planned>, String> {
    let pending = pending_rename_names(root)?;
    let names = taken(root).map_err(|why| census_failed(Scope::Default, root, &why))?;
    let mut planned = Vec::with_capacity(names.len());
    for name in names {
        let dir = crate::lifecycle::sessions_dir(root).join(&name);
        let read = crate::meta::read_bytes(&dir);
        let renaming = pending.contains(&name);
        match judge(Scope::Default, root, &name, renaming, &read) {
            Ok(session) => planned.push(session),
            Err(Stop::Fatal(why) | Stop::Hold(why)) => return Err(unmoved(&why)),
        }
    }
    Ok(planned)
}

/// A named root's sessions, read once and judged, with the proof
/// `strict_proof` makes of a root: a doubt about the root or an ordinary
/// session HOLDS it, and the scan goes on to the end so no held root hides a
/// protected session.
fn census_named(root: &Path) -> Result<NamedRoot, String> {
    if let Some(why) = named_root_doubt(root) {
        return Ok(NamedRoot::Held(why));
    }
    let sessions = crate::lifecycle::sessions_dir(root);
    if lstat(&sessions).is_err_and(|why| why.kind() == std::io::ErrorKind::NotFound) {
        // No sessions directory: nothing to move and nothing pinned.
        return Ok(NamedRoot::Ready(Vec::new()));
    }
    if let Some(why) = not_real_dir(&sessions) {
        return Ok(NamedRoot::Held(why));
    }
    let mut hold = None;
    if !crate::rename::pending_intents(root).is_empty()
        || !crate::rename::pending_damaged(root).is_empty()
    {
        hold = Some("a rename is in progress".to_owned());
    }
    let pending = pending_rename_names(root).unwrap_or_default();
    let entries = match listing(&sessions) {
        Ok(entries) => entries,
        Err(why) => {
            let why = format!("{} could not be listed ({why})", sessions.display());
            return Ok(NamedRoot::Held(why));
        }
    };
    let mut planned = Vec::new();
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        // A dot entry can never be a session.
        if name.starts_with('.') {
            continue;
        }
        let dir = entry.path();
        if let Some(why) = not_real_dir(&dir).or_else(|| meta_node(&dir).err()) {
            hold.get_or_insert(format!("session {name:?}: {why}"));
            continue;
        }
        let read = crate::meta::read_bytes(&dir);
        match judge(Scope::Named, root, &name, pending.contains(&name), &read) {
            Ok(session) => planned.push(session),
            Err(Stop::Fatal(why)) => return Err(unmoved(&why)),
            Err(Stop::Hold(why)) => {
                hold.get_or_insert(why);
            }
        }
    }
    let Some(why) = hold else {
        return Ok(NamedRoot::Ready(planned));
    };
    // A HOLD LEAVES THE WHOLE ROOT ON THE OLD CORE, and the command moves
    // without it: a planned session of the protected cohort in it would keep
    // helpers the new core refuses. That is a refusal of the publish, not a
    // note. An ordinary root with no such session keeps its hold.
    if let Some(seat) = planned.iter().find(|session| session.protected) {
        return Err(unmoved(&format!(
            "session {:?}{} was planned as an orchestrator session, but {why} would hold its \
             root on the old core; repair or end it",
            seat.name,
            place_of(Scope::Named, root)
        )));
    }
    Ok(NamedRoot::Held(why))
}

/// The sentence that closes a refusal made before anything moved.
const NOT_MOVED: &str = " Nothing was repointed and the ae command was not moved.";

/// A refusal made before anything moved, as one sentence.
fn unmoved(why: &str) -> String {
    format!("{}.{NOT_MOVED}", why.trim_end_matches('.'))
}

/// The sentence a refusal made after some sessions moved owes the operator:
/// which of them moved, across every root, while the command stayed.
fn moved_tail(moved: &[String], version: &str) -> String {
    if moved.is_empty() {
        return NOT_MOVED.to_owned();
    }
    format!(
        " The ae command was NOT moved, but {} session(s) already name {version}: {}. \
         Repair or end the session named above, then re-run the upgrade.",
        moved.len(),
        moved.join(", ")
    )
}

/// The identity a moved session carries in the fleet's list: its name in the
/// default root, `instance/name` in a named one, so equal names of two roots
/// are two sessions.
fn moved_name(scope: Scope, root: &Path, name: &str) -> String {
    match (scope, root.file_name().and_then(std::ffi::OsStr::to_str)) {
        (Scope::Named, Some(instance)) => format!("{instance}/{name}"),
        _ => name.to_owned(),
    }
}

/// The place a refusal names beside the session: nothing for the default
/// root, the instance for a named one.
fn place_of(scope: Scope, root: &Path) -> String {
    match (scope, root.file_name().and_then(std::ffi::OsStr::to_str)) {
        (Scope::Named, Some(instance)) => format!(" of instance {instance}"),
        _ => String::new(),
    }
}

/// The census's and the apply's one judgement of a session, asked of the bytes
/// each of them read.
///
/// A meta that is not there is no session. One that is there and cannot be
/// read whole, or a protected-cohort session the chain cannot place, stops the
/// publish ([`admission`]). Past that, an ordinary session is moved when the
/// chain places it, and otherwise skipped when it is stopped in the default
/// root, refused when it is running there, and a reason to hold a named root.
fn judge(
    scope: Scope,
    root: &Path,
    name: &str,
    renaming: bool,
    read: &std::io::Result<Vec<u8>>,
) -> Result<Planned, Stop> {
    let dir = crate::lifecycle::sessions_dir(root).join(name);
    if let Some(why) = admission(&dir, name, &place_of(scope, root), read, renaming) {
        return Err(Stop::Fatal(why));
    }
    let planned = |digest, protected, seat, judged| Planned {
        name: name.to_owned(),
        digest,
        protected,
        seat,
        judged,
    };
    let Ok(bytes) = read else {
        return Ok(planned(None, false, false, Judged::NoMeta));
    };
    let digest = Some(crate::install::sha256_hex(bytes));
    let protected = crate::meta::meta_agent_role(bytes) != crate::meta::MetaAgentRole::Absent;
    let seat = crate::meta::is_seat(bytes, root);
    if renaming {
        return Ok(planned(digest, protected, seat, Judged::Renaming));
    }
    let text = String::from_utf8_lossy(bytes);
    match migrate(&text) {
        Ok(_) => Ok(planned(digest, protected, seat, Judged::Move)),
        Err(refusal) if scope.skips() && !unplaceable_is_running(&text, name) => Ok(planned(
            digest,
            protected,
            seat,
            Judged::Skip(skipped_refusal(name, &refusal)),
        )),
        Err(refusal) if scope.skips() => Err(Stop::Fatal(refusal.line(name))),
        Err(refusal) => Err(Stop::Hold(refusal.line(name))),
    }
}

/// [`judge`] again at apply, on the bytes read under the session's locks. A
/// planned move or missing meta whose digest is unchanged keeps its verdict;
/// anything else is judged anew, because a skipped session may have started
/// since. A session of the protected cohort whose meta changed at all is a
/// late refusal, even when the new bytes have dropped the role or can be
/// placed: the plan that admitted it no longer describes it.
fn recheck(
    scope: Scope,
    root: &Path,
    planned: &Planned,
    read: &std::io::Result<Vec<u8>>,
) -> Result<Planned, Stop> {
    // UNCHANGED means the same bytes, or a meta that is still not there. A meta
    // that now fails to read for any other reason is a change: two absent
    // digests are NOT equal, or a planned `NoMeta` would hide a present
    // unreadable meta behind its own skip.
    let unchanged = match (&planned.digest, read) {
        (Some(planned), Ok(bytes)) => *planned == crate::install::sha256_hex(bytes),
        (None, Err(why)) => why.kind() == std::io::ErrorKind::NotFound,
        _ => false,
    };
    if unchanged && matches!(planned.judged, Judged::Move | Judged::NoMeta) {
        return Ok(planned.clone());
    }
    if !unchanged && planned.protected {
        return Err(Stop::Fatal(format!(
            "session {:?}{} was an orchestrator session (meta_agent) when this upgrade planned \
             it and its meta changed since; repair or end it",
            planned.name,
            place_of(scope, root)
        )));
    }
    judge(scope, root, &planned.name, false, read)
}

/// Why the session `name` stops the upgrade, or `None`. One verdict, asked by
/// the census and again under the session's lifecycle lock at apply, on the
/// bytes each of them read.
///
/// A meta that is not there is not a session. One that is there and cannot be
/// read whole is in doubt about the role it may carry, so it stops the
/// upgrade; so does a session claiming the role (`meta_agent`, byte-exact or
/// damaged) that the chain cannot place, or that a pending rename holds out of
/// the sweep.
fn admission(
    dir: &Path,
    name: &str,
    place: &str,
    read: &std::io::Result<Vec<u8>>,
    held_by_rename: bool,
) -> Option<String> {
    let path = crate::store::open(dir).meta_path();
    let bytes = match read {
        Ok(bytes) => bytes,
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => return None,
        Err(why) => {
            return Some(format!(
                "session {name:?}{place}: meta could not be read: {why} ({}); repair or end it",
                path.display()
            ));
        }
    };
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Some(format!(
            "session {name:?}{place}: meta is not valid UTF-8 ({}); repair or end it",
            path.display()
        ));
    };
    let parsed = crate::meta::Meta::parse(text);
    let malformed = parsed.anomalies().iter().find_map(|anomaly| match anomaly {
        crate::meta::Anomaly::MalformedLine { line } => Some(*line),
        _ => None,
    });
    if let Some(line) = malformed {
        return Some(format!(
            "session {name:?}{place}: meta has a malformed line {line} ({}); repair or end it",
            path.display()
        ));
    }
    if crate::meta::meta_agent_role(bytes) == crate::meta::MetaAgentRole::Absent {
        return None;
    }
    if held_by_rename {
        return Some(format!(
            "session {name:?}{place} records the orchestrator role (meta_agent) and a rename \
             of it is pending, so this upgrade would skip it: finish that rename, or repair or end it"
        ));
    }
    migrate(text).err().map(|refusal| {
        format!(
            "session {name:?}{place} records the orchestrator role (meta_agent) and this upgrade \
             cannot relink it: {}; repair or end it",
            refusal.line(name).trim_end_matches('.')
        )
    })
}

/// Which root a sweep runs over, because the two answer damage differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// The default root: a stopped unplaceable session is skipped, and an
    /// abort leaves the ae command where it was.
    Default,
    /// A named instance root, swept AFTER the default: every unplaceable or
    /// unreadable session refuses, running or not, and an abort never stops
    /// the publish, so it may not claim the command stayed put.
    Named,
}

impl Scope {
    /// Whether a STOPPED session the chain cannot place may be skipped.
    fn skips(self) -> bool {
        self == Self::Default
    }
}

/// One root's sweep. A failure of a NAMED root holds that root and the
/// publish goes on, unless the failure strands a session of the protected
/// cohort on the old core: one that failed itself, or one the abort kept the
/// sweep from reaching. Then the whole publish stops, before the command
/// moves, and names every session that already moved.
fn onto_scoped(
    root: &Path,
    core: &Path,
    version: &str,
    scope: Scope,
    planned: Vec<Planned>,
    moved: &mut Vec<String>,
    stops: &mut bool,
) -> Result<Vec<String>, String> {
    let protected: Vec<String> = planned
        .iter()
        .filter(|session| session.protected)
        .map(|session| session.name.clone())
        .collect();
    let mut at = String::new();
    let result = sweep(root, core, version, scope, planned, moved, stops, &mut at);
    let Err(why) = result else {
        return result;
    };
    if *stops {
        return Err(why);
    }
    // EXACT identities: a default `orchestrator` or `alpha/orchestrator` that
    // moved never stands in for an unreached `beta/orchestrator`.
    let Some(stranded) = protected
        .iter()
        .find(|name| **name == at || !moved.contains(&moved_name(scope, root, name)))
    else {
        return Err(why);
    };
    *stops = true;
    Err(format!(
        "{why} The orchestrator session {stranded:?}{} was not relinked, so the upgrade stops \
         here.{}",
        place_of(scope, root),
        moved_tail(moved, version)
    ))
}

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "the publish sweep reads as one order: the planned verdict, the locked recheck, then one published meta with its skips"
)]
fn sweep(
    root: &Path,
    core: &Path,
    version: &str,
    scope: Scope,
    planned: Vec<Planned>,
    moved: &mut Vec<String>,
    stops: &mut bool,
    at: &mut String,
) -> Result<Vec<String>, String> {
    // NOTHING HERE IS A SECOND CENSUS. The census ([`plan_fleet`]) read every
    // session of every root once and refused, before anything moved, whatever
    // could not be moved: a pending rename owns its names, a stopped
    // unplaceable session is skipped, a meta that cannot be read or a
    // protected session that cannot be relinked refuses the publish. This
    // sweep acts on those verdicts and asks each session ONE question again,
    // under its own locks and on the bytes it reads there.
    //
    // A named root proves its whole ancestry FIRST, because every read below
    // goes through it: a root that vanished, or a link that replaced a part of
    // it, is doubt — never an empty sessions root.
    if scope == Scope::Named
        && let Some(why) = named_chain(root, None)
    {
        return Err(format!("{why}.{}", already(scope, &[], version)));
    }
    let mut notes = Vec::new();
    let mut bridges: Vec<ServerId> = Vec::new();
    // WHAT THE SWEEP HAS ALREADY DONE, so a failure inside it can say so: a
    // lock can time out, a meta can stop being readable, a helper destination
    // can refuse to be replaced. An abort after any of those owes the
    // operator the sessions that DID move rather than a claim that none did.
    let mut repointed: Vec<String> = Vec::new();
    // Stamps are COUNTED, not listed: on the release that adds the chain every
    // session on the machine takes one, and 28 identical lines say less than
    // one line with a number on it.
    let mut stamped = 0_usize;
    // One counted group per source version, in first-seen order.
    let mut stepped: Vec<(u32, usize)> = Vec::new();
    for plan in planned {
        let name = plan.name.clone();
        at.clone_from(&name);
        // A pending rename's names skip untouched (reported, like any
        // unplaceable session) before any lock, read, or write. A rename that
        // converged since is judged anew below.
        if plan.judged == Judged::Renaming
            && let Some(note) = pending_rename_note(root, &name)
        {
            notes.push(note);
            continue;
        }
        let dir = crate::lifecycle::sessions_dir(root).join(&name);
        // A named session's chain is proved BEFORE its lock is created — the
        // lock file is written inside `sessions` — and again once it is held.
        reproved(scope, root, &dir, &name)
            .map_err(|why| format!("{why}{}", already(scope, &repointed, version)))?;
        // The lifecycle lock, so a resume or an end cannot land inside a
        // migration. A session whose lock is held is a session in the middle
        // of exactly the operation this sweep must not race.
        let Ok(_guard) = crate::lifecycle::lock(root, &name) else {
            return Err(format!(
                "session {name:?} is busy (another start, resume or end is in progress); \
                 retry the upgrade shortly.{}",
                already(scope, &repointed, version)
            ));
        };
        // Again HERE, under the lock and before any read or write: anything on
        // the chain swapped for a link since would otherwise be written through.
        reproved(scope, root, &dir, &name)
            .map_err(|why| format!("{why}{}", already(scope, &repointed, version)))?;
        let halted =
            |stop: Stop, repointed: &[String], moved: &[String], stops: &mut bool| match stop {
                Stop::Fatal(why) => {
                    *stops = true;
                    format!(
                        "{}.{}",
                        why.trim_end_matches('.'),
                        moved_tail(moved, version)
                    )
                }
                Stop::Hold(why) => format!("{why}{}", already(scope, repointed, version)),
            };
        // THE RENAME CARRIER, asked again of THIS name alone and under the
        // lifecycle lock both ends of a rename take: a rename that began after
        // the census owns the name even though its meta did not change. An
        // ordinary session keeps the skip; an orchestrator session cannot be
        // skipped, so the publish stops with the moved list.
        if let Some(note) = pending_rename_note(root, &name) {
            if plan.protected {
                return Err(halted(
                    Stop::Fatal(format!(
                        "session {name:?}{} is an orchestrator session and a pending rename now \
                         owns it; repair or end it",
                        place_of(scope, root)
                    )),
                    &repointed,
                    moved,
                    stops,
                ));
            }
            notes.push(note);
            continue;
        }
        // A session the census did NOT plan to move, or one whose meta may
        // have changed, is asked again without the meta lock: a stopped skip
        // must never take it or gain helper links, and a session that has
        // started since must now abort. Only a planned move goes straight on.
        if plan.judged != Judged::Move {
            let read = crate::meta::read_bytes(&dir);
            match recheck(scope, root, &plan, &read) {
                Err(stop) => return Err(halted(stop, &repointed, moved, stops)),
                Ok(now) => match now.judged {
                    Judged::NoMeta => {
                        notes.push(format!("skipped {name}: no meta"));
                        continue;
                    }
                    Judged::Skip(note) => {
                        notes.push(note);
                        continue;
                    }
                    Judged::Renaming => continue,
                    Judged::Move => {}
                },
            }
        }
        // ONE meta lock, held from the deciding read through the one publish:
        // a writer that takes only `meta.lock` cannot change the bytes this
        // decision was made on. The recheck here is the authoritative one.
        let held_meta = crate::meta::lock(&dir).map_err(|why| {
            format!(
                "session {name:?}: meta.lock: {why}.{}",
                already(scope, &repointed, version)
            )
        })?;
        let read = crate::meta::read_bytes(&dir);
        let now = match recheck(scope, root, &plan, &read) {
            Ok(now) => now,
            Err(stop) => return Err(halted(stop, &repointed, moved, stops)),
        };
        match now.judged {
            Judged::Move => {}
            Judged::NoMeta => {
                notes.push(format!("skipped {name}: no meta"));
                continue;
            }
            Judged::Skip(note) => {
                notes.push(note);
                continue;
            }
            Judged::Renaming => continue,
        }
        let Ok(bytes) = read else {
            return Err(format!(
                "session {name:?}: its meta vanished under its lock.{}",
                already(scope, &repointed, version)
            ));
        };
        let text = String::from_utf8_lossy(&bytes);
        let (what, migrated) = match migrate(&text) {
            Ok(None) => (None, text.into_owned()),
            Ok(Some(done)) => (Some(done.what), done.text),
            Err(refusal) => {
                return Err(format!(
                    "{}{}",
                    refusal.line(&name),
                    already(scope, &repointed, version)
                ));
            }
        };
        let published = with_core_rows(&migrated, core, version);
        crate::meta::publish_locked(&dir, &published).map_err(|why| {
            format!(
                "session {name:?}: meta could not be written ({}).{}",
                why.cause(),
                already(scope, &repointed, version)
            )
        })?;
        drop(held_meta);
        match what {
            None => {}
            Some(Stepped::Stamped) => stamped += 1,
            // COUNTED, not listed. The release that adds a step meets every
            // session on the machine at once — 28 of 28 on the one this was
            // written for — and an operator wants one line with a number, not
            // one line per session. The count is per SOURCE version, because
            // two sessions stepped from different shapes did not have the same
            // thing done to them.
            Some(Stepped::From(from)) => match stepped.iter_mut().find(|(at, _)| *at == from) {
                Some((_, count)) => *count += 1,
                None => stepped.push((from, 1_usize)),
            },
        }
        // RECORDED HERE, not after the helpers. The meta already names the new
        // core at this point, so a helper failure below leaves this session
        // repointed — reporting it as untouched would send an operator looking
        // in the wrong place.
        repointed.push(name.clone());
        moved.push(moved_name(scope, root, &name));
        crate::session_launch::assets::write_helpers(&dir, core).map_err(|why| {
            format!(
                "session {name:?}: {why}. Its meta already names {version} while some of its \
                 helpers still name the old core, so it is PARTIALLY relinked.{}",
                already(scope, &repointed, version)
            )
        })?;
        if now.seat {
            notes.extend(reseed_seat_prompt(root));
        }
        notes.extend(restart_daemons(root, &dir, &name, core, &mut bridges));
    }
    if stamped > 0 {
        notes.insert(
            0,
            format!("stamped {KEY}={CURRENT} into {stamped} session(s) that carried only schema=2"),
        );
    }
    // Oldest source first, so the line an operator reads first is about the
    // oldest shape the publish found.
    stepped.sort_unstable();
    for (from, count) in stepped.into_iter().rev() {
        notes.insert(
            0,
            format!("migrated {count} session(s) from {KEY}={from} to {CURRENT}"),
        );
    }
    Ok(notes)
}

/// The most a seat config may weigh for the re-seed to read it.
const SEAT_CONFIG_CAP: u64 = 1024 * 1024;

/// Bring the seat file of `root` to the current prompt when it still carries,
/// byte for byte, the prompt of the template ae seeded before: the stanza is
/// replaced and every other byte of the file stays. A prompt that is current
/// already, or that anyone has edited, is left alone, and a changed or
/// edited-and-stale one is reported with the step that fixes it. Only a
/// regular file is rewritten: a link (a dotfile manager's) is never replaced.
fn reseed_seat_prompt(root: &Path) -> Option<String> {
    use crate::store::SourceRead;

    let path = root.join(crate::meta::SEAT_CONFIG_FILE);
    let read = crate::store::read_capped(&path, SEAT_CONFIG_CAP);
    let Ok(SourceRead::Ready(bytes)) = read else {
        return Some(format!(
            "left {} alone: it is not a regular file this upgrade can read; copy the current \
             [prompt] stanza of the seat template into it and restart the seat",
            path.display()
        ));
    };
    let text = String::from_utf8(bytes).ok()?;
    if crate::config::seat_prompt_is_current(&text) {
        return None;
    }
    let Some(new) = crate::config::reseeded_seat_config(&text) else {
        return Some(format!(
            "left {} alone: its [prompt] is not a template ae wrote; copy the current [prompt] \
             stanza into it, keeping your edits, and restart the seat",
            path.display()
        ));
    };
    Some(match publish_reseed(&path, text.as_bytes(), &new) {
        Ok(()) => format!(
            "re-seeded the [prompt] of {} (it matched the previous template byte for byte); \
             restart the orchestrator seat to load it",
            path.display()
        ),
        Err(why) => format!(
            "could not re-seed the [prompt] of {} ({why}); copy the current stanza into it and \
             restart the seat",
            path.display()
        ),
    })
}

/// Publish the re-seeded seat config `new` over `path`, which was read as
/// `seen`: the rename waits on a last look at the target, so an owner's edit,
/// or a link that replaced the file, between the decision and the rename is
/// never overwritten. The window from that look to the rename is the one any
/// writer that does not cooperate leaves.
fn publish_reseed(path: &Path, seen: &[u8], new: &str) -> std::io::Result<()> {
    use crate::store::SourceRead;

    crate::store::publish_private_with(path, new.as_bytes(), |temp, to| {
        match crate::store::read_capped(to, SEAT_CONFIG_CAP) {
            Ok(SourceRead::Ready(now)) if now == seen => std::fs::rename(temp, to),
            _ => Err(std::io::Error::other(
                "the seat config changed while it was being re-seeded",
            )),
        }
    })
}

/// The abort for a sessions root that could not be enumerated.
fn census_failed(scope: Scope, root: &Path, why: &std::io::Error) -> String {
    let outcome = match scope {
        Scope::Default => "nothing was migrated and the ae command was not moved",
        Scope::Named => "nothing in this instance was migrated",
    };
    format!(
        "the sessions under {} could not be enumerated ({why}); {outcome}",
        crate::lifecycle::sessions_dir(root).display()
    )
}

/// The sentence an abort owes the operator: which sessions already moved.
fn already(scope: Scope, repointed: &[String], version: &str) -> String {
    if scope == Scope::Named {
        return format!(
            " {} session(s) of this instance already name {version}{}; repair it, then run \
             AE_VERSION={version} ae upgrade.",
            repointed.len(),
            if repointed.is_empty() {
                String::new()
            } else {
                format!(": {}", repointed.join(", "))
            }
        );
    }
    if repointed.is_empty() {
        return " No session was repointed and the ae command was not moved.".to_owned();
    }
    format!(
        " The ae command was NOT moved, but {} session(s) already name {version}: {}. \
         Re-run the upgrade once the cause is cleared.",
        repointed.len(),
        repointed.join(", ")
    )
}

/// `text` with the core rows of one session's meta naming `core`, as ONE
/// document.
///
/// Published once, with the meta lock held by the caller, together with
/// whatever the chain wrote. Separate [`crate::meta::rewrite`] calls would each
/// take the lock, read, write and rename on their own, so a process that died
/// between them would leave a meta naming the new core at one row and the old
/// core at another — a session that disagrees with itself about which binary
/// it runs.
fn with_core_rows(text: &str, core: &Path, version: &str) -> String {
    let core_text = core.display().to_string();
    let mut rows: Vec<(&str, &str)> = vec![
        (CORE_ROWS[0], core_text.as_str()),
        (CORE_ROWS[1], version),
        ("ae_version", version),
    ];
    // The `ae_path` row is rewritten where one is already there and never
    // introduced, so a meta that never carried it does not start carrying one.
    if text
        .lines()
        .any(|line| line.starts_with(&format!("{LEGACY_PATH_ROW}=")))
    {
        rows.push((LEGACY_PATH_ROW, core_text.as_str()));
    }
    let mut text = text.to_owned();
    for (key, value) in rows {
        text = crate::meta::rewritten(&text, key, Some(value));
    }
    text
}

/// Restart the daemons of a RUNNING session, on the new core.
///
/// Best effort by design: a daemon that will not come back is a degraded
/// session, not a failed upgrade, and the alternative — aborting a publish
/// half-done because tmux did not answer — is worse. Every outcome is
/// reported.
fn restart_daemons(
    root: &Path,
    dir: &Path,
    name: &str,
    core: &Path,
    bridges: &mut Vec<ServerId>,
) -> Vec<String> {
    let mut notes = Vec::new();
    let Some(server) = crate::session_launch::recorded_server_resolved(dir) else {
        return notes;
    };
    if !crate::transport::session_exists(&server, name) {
        return notes;
    }
    let meta_bytes = crate::meta::read_bytes(dir).ok();
    let config = meta_bytes
        .as_deref()
        .and_then(|bytes| crate::meta::first_value(bytes, "config"))
        .filter(|value| !value.is_empty())
        .map_or_else(
            || root.join("config"),
            |value| PathBuf::from(String::from_utf8_lossy(value).into_owned()),
        );
    // The PUBLISH launcher, not the running shape's: this sweep runs as the
    // downloaded core, so the running shape reads Checkout even for an
    // installed home.
    let launcher = crate::session_tmux::publish_launcher(
        crate::shape::current(),
        core,
        root,
        &config,
        &server,
    );
    let menu_mouse = crate::transport::observe_tmux_floor(&server).menu_mouse();
    for binding in crate::session_tmux::status_bindings_argv(&server, &launcher, menu_mouse) {
        let _ = crate::transport::run_tmux_op(&binding);
    }
    // The kill-server lock follows the PUBLISH's own fact, not the running
    // shape: this caller reads Checkout even when its target is an installed
    // home, and the command link is what says it is one.
    if crate::session_tmux::publish_link(root).is_some() {
        crate::transport::assert_kill_lock(&server);
    }
    let main_pane = meta_bytes
        .as_deref()
        .and_then(|bytes| crate::meta::first_value(bytes, "main_pane"))
        .map(|value| String::from_utf8_lossy(value).into_owned())
        .unwrap_or_default();
    // The IMMUTABLE session identity is observed BEFORE the membership proof
    // and carried through the backfill: the name can be replaced between the
    // proof and the write, so the writer reproves this id and addresses the
    // option by it, never by the mutable name.
    // The identity is captured through the RECORDED main pane. A pane id is
    // REUSABLE after a server restart, so this capture alone cannot tell a
    // replacement from the proven session: the ownership pair below and the
    // guarded write's own reproof are what make the backfill safe.
    let live_identity = crate::session_launch::pane_proven_identity(&server, &main_pane);
    let pane_belongs = crate::transport::observe_agents(&server, name)
        .is_some_and(|panes| panes.into_iter().any(|pane| pane.pane == main_pane));
    // OWNERSHIP beside membership. A server restart can hand a same-name
    // replacement the recorded main pane (and even the recorded ids), so pane
    // containment alone cannot tell it from the proven session; the marker pair
    // names the state root whose launch created the live session, and only THIS
    // root's UUID may be stamped.
    let owned_by_root =
        crate::transport::observe_session_ownership(&server, name).is_some_and(|ownership| {
            !ownership.marker.is_empty() && std::path::Path::new(&ownership.home) == root
        });
    crate::session_launch::stamp_main_pane(&server, name, &main_pane, pane_belongs);
    // The UUID fact's upgrade backfill, beside the membership-proven main-pane
    // stamp and ONLY under the same proof: stamping a state directory's UUID
    // onto a same-name tmux session that does not contain its recorded pane
    // would pin a stale identity to a live, unrelated incarnation — and the
    // root would then render that stale directory's declarations. An ambiguous
    // (duplicated) identity is refused outright. VACANT-ONLY through the one
    // owner, so a pre-set differing value survives the sweep.
    if pane_belongs
        && owned_by_root
        && let Some(bytes) = meta_bytes.as_deref()
        && let Some(live_identity) = live_identity.as_ref()
    {
        let uuid = match crate::meta::sole_value(bytes, "session_id") {
            Some(value) => crate::archive::canonical_uuid(&String::from_utf8_lossy(value)),
            None => String::new(),
        };
        if !uuid.is_empty() {
            let _ = crate::session_launch::seed_session_uuid(&server, live_identity, &uuid);
        }
    }
    if let Some(bytes) = meta_bytes {
        let layout = crate::meta::first_value(&bytes, "layout")
            .map(|value| String::from_utf8_lossy(value).into_owned())
            .unwrap_or_default();
        if pane_belongs {
            crate::session_launch::stamp_lead_pair_policy(&server, &layout, &main_pane);
        }
    }
    match crate::watchdog_lifecycle::presence(&server, name, dir) {
        crate::watchdog_lifecycle::Presence::Running(_) => {
            notes.push(restart_watchdog(root, &server, name, dir));
        }
        crate::watchdog_lifecycle::Presence::Stopped
            if crate::session_launch::watchdog_enabled_for_session(dir) =>
        {
            notes.push(start_watchdog(root, &server, name, dir));
        }
        crate::watchdog_lifecycle::Presence::Stopped
        | crate::watchdog_lifecycle::Presence::Unknown => {}
    }
    if !bridges.contains(&server) {
        bridges.push(server.clone());
        let paths = crate::telegram::bridge::Paths::under(root);
        match crate::telegram_lifecycle::restart_on(&paths, &server, core) {
            Ok(true) => notes.push("restarted the telegram bridge on the new core".to_owned()),
            Ok(false) => {}
            Err(why) => notes.push(format!("the telegram bridge did not restart: {why}")),
        }
    }
    notes
}

/// Stop and start one session's watchdog through its own entry, so the pane is
/// rebuilt by the code that owns every guard around it — then PROVE it.
///
/// The exit code is not the proof. `watchdog start` answers 0 for "deferred,
/// another start holds the lock" and for "skipped, tmux did not answer", and
/// this function had just stopped the only watchdog: taking either 0 as
/// success would report a restart over a session that now has none. So the
/// answer is [`crate::watchdog_lifecycle::presence`], asked afterwards, and it
/// names the pid so the note is checkable.
fn restart_watchdog(root: &Path, server: &ServerId, name: &str, dir: &Path) -> String {
    for action in ["stop", "start"] {
        let tail = [action.to_owned(), name.to_owned()];
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let audit = crate::watchdog_lifecycle::UPGRADE_ACTOR;
        match crate::watchdog_lifecycle::run_with_actor(root, &tail, audit, &mut out, &mut err) {
            Ok(0) => {}
            Ok(_) | Err(_) => {
                return format!(
                    "WARNING: the watchdog of {name} did not {action} ({}) — start it by hand with `ae watchdog start {name}`",
                    String::from_utf8_lossy(&err).trim()
                );
            }
        }
    }
    match crate::watchdog_lifecycle::presence(server, name, dir) {
        crate::watchdog_lifecycle::Presence::Running(pid) => {
            format!("restarted the watchdog of {name} on the new core (pid {pid})")
        }
        crate::watchdog_lifecycle::Presence::Stopped
        | crate::watchdog_lifecycle::Presence::Unknown => format!(
            "WARNING: {name} was left with NO watchdog — the restart reported success and none is running; start it by hand with `ae watchdog start {name}`"
        ),
    }
}

/// Start a watchdog missing from a running session, then prove it registered.
fn start_watchdog(root: &Path, server: &ServerId, name: &str, dir: &Path) -> String {
    let tail = ["start".to_owned(), name.to_owned()];
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let audit = crate::watchdog_lifecycle::UPGRADE_ACTOR;
    match crate::watchdog_lifecycle::run_with_actor(root, &tail, audit, &mut out, &mut err) {
        Ok(0) => {}
        Ok(_) | Err(_) => {
            return format!(
                "WARNING: the watchdog of {name} did not start ({}) — start it by hand with `ae watchdog start {name}`",
                String::from_utf8_lossy(&err).trim()
            );
        }
    }
    match crate::watchdog_lifecycle::presence(server, name, dir) {
        crate::watchdog_lifecycle::Presence::Running(pid) => {
            format!("started the watchdog of {name} on the new core (pid {pid})")
        }
        crate::watchdog_lifecycle::Presence::Stopped
        | crate::watchdog_lifecycle::Presence::Unknown => format!(
            "WARNING: {name} was left with NO watchdog — the start reported success and none is running; start it by hand with `ae watchdog start {name}`"
        ),
    }
}

// ─── the named instance roots ────────────────────────────────────────────

/// The named instance roots under one base, read WITHOUT following a link.
#[derive(Debug, Default)]
struct Fleet {
    /// Every root proved a real directory with a selectable name, by name.
    named: Vec<(String, PathBuf)>,
    /// `(label, why)` for every entry that could not be proved. Any one of
    /// them freezes the version sweep: its sessions' pins are unknown.
    doubts: Vec<(String, String)>,
}

/// `lstat`, for the fleet's classification of what a link could hide.
fn lstat(path: &Path) -> std::io::Result<std::fs::Metadata> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the publish classifies every named instance root and its sessions WITHOUT following a link"
    )]
    let meta = std::fs::symlink_metadata(path);
    meta
}

/// A directory's entries, for the fleet sweep, sorted; a listing that failed
/// half-way is a failure, never a shorter list.
fn listing(dir: &Path) -> std::io::Result<Vec<std::fs::DirEntry>> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the publish enumerates the named instance roots and their sessions it must migrate or hold"
    )]
    let entries = std::fs::read_dir(dir)?;
    let mut found = entries.collect::<std::io::Result<Vec<_>>>()?;
    found.sort_by_key(std::fs::DirEntry::file_name);
    Ok(found)
}

/// Why `path` is not a real directory, or `None` when it is one.
fn not_real_dir(path: &Path) -> Option<String> {
    match lstat(path) {
        Ok(meta) if meta.file_type().is_dir() => None,
        Ok(_) => Some(format!("{} is not a real directory", path.display())),
        Err(why) => Some(format!("{} could not be read ({why})", path.display())),
    }
}

/// The named roots under `base`. An absent `<base>/instances` is an EMPTY
/// fleet with no doubt — the byte-identical publish of a machine with none.
fn fleet(base: &Path) -> Fleet {
    let mut fleet = Fleet::default();
    let container = base.join(crate::instance::INSTANCES);
    let every = "every named instance".to_owned();
    if lstat(&container).is_err_and(|why| why.kind() == std::io::ErrorKind::NotFound) {
        return fleet;
    }
    if let Some(why) = not_real_dir(&container) {
        fleet.doubts.push((every, why));
        return fleet;
    }
    let entries = match listing(&container) {
        Ok(entries) => entries,
        Err(why) => {
            let why = format!("{} could not be listed ({why})", container.display());
            fleet.doubts.push((every, why));
            return fleet;
        }
    };
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        // A dot entry (`.DS_Store`) can never be selected, so it hides no root.
        if name.starts_with('.') {
            continue;
        }
        if !crate::instance::is_instance_name(&name) {
            let why = "not a selectable instance name".to_owned();
            fleet.doubts.push((format!("instance {name:?}"), why));
            continue;
        }
        let root = entry.path();
        match not_real_dir(&root) {
            Some(why) => fleet.doubts.push((format!("instance {name}"), why)),
            None => fleet.named.push((name, root)),
        }
    }
    fleet
}

/// The versions a meta's core rows record.
fn core_versions(bytes: &[u8]) -> Vec<String> {
    let mut found = Vec::new();
    for key in CORE_ROWS {
        let Some(value) = crate::meta::first_value(bytes, key) else {
            continue;
        };
        let value = value.strip_suffix(b"\r").unwrap_or(value);
        let version = if key == CORE_ROWS[0] {
            version_of(&String::from_utf8_lossy(value))
        } else {
            std::str::from_utf8(value)
                .ok()
                .filter(|version| !version.is_empty())
                .map(ToOwned::to_owned)
        };
        found.extend(version);
    }
    found
}

/// Why the meta node of `dir` is in doubt: anything but a regular file, link
/// included, or an `lstat` that failed for any reason but absence.
fn meta_node(dir: &Path) -> Result<(), String> {
    let meta = crate::store::open(dir).meta_path();
    match lstat(&meta) {
        Ok(node) if node.file_type().is_file() => Ok(()),
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(format!("{} is not a regular file", meta.display())),
        Err(why) => Err(format!("{} could not be read ({why})", meta.display())),
    }
}

/// Whether the meta node of `dir` is PROVED absent: `NotFound` and nothing else.
fn meta_absent(dir: &Path) -> bool {
    lstat(&crate::store::open(dir).meta_path())
        .is_err_and(|why| why.kind() == std::io::ErrorKind::NotFound)
}

/// Why a named `root` — its `instances` container and itself — is in doubt.
fn named_root_doubt(root: &Path) -> Option<String> {
    match root.parent() {
        Some(container) => not_real_dir(container).or_else(|| not_real_dir(root)),
        None => Some(format!("{} has no container", root.display())),
    }
}

/// Why the chain from the container down to `dir`'s meta is in doubt: every
/// directory on it a real one, the meta a regular file or genuinely absent.
fn named_chain(root: &Path, dir: Option<&Path>) -> Option<String> {
    named_root_doubt(root)
        .or_else(|| not_real_dir(&crate::lifecycle::sessions_dir(root)))
        .or_else(|| dir.and_then(|dir| not_real_dir(dir).or_else(|| meta_node(dir).err())))
}

/// A named session's chain, re-proved before a lock or a write; the default
/// root keeps the sweep it always had.
fn reproved(scope: Scope, root: &Path, dir: &Path, name: &str) -> Result<(), String> {
    if scope.skips() {
        return Ok(());
    }
    named_chain(root, Some(dir)).map_or(Ok(()), |why| Err(format!("session {name:?}: {why}.")))
}

/// READ-ONLY proof that every session under the named `root` is placeable,
/// answering the versions their core rows record. Stricter than the default
/// sweep by ruling: a pending rename, a sessions entry that is not a real
/// directory, an unreadable meta or one the chain cannot place — running or
/// not — is doubt about the WHOLE root, so nothing of it is written and no
/// version is pruned.
fn strict_proof(root: &Path) -> Result<Option<Vec<String>>, String> {
    if let Some(why) = named_root_doubt(root) {
        return Err(why);
    }
    let sessions = crate::lifecycle::sessions_dir(root);
    if lstat(&sessions).is_err_and(|why| why.kind() == std::io::ErrorKind::NotFound) {
        return Ok(None);
    }
    // The ancestry FIRST: the rename scan reads the carriers inside
    // `sessions`, and through a link it would classify what lives elsewhere.
    if let Some(why) = not_real_dir(&sessions) {
        return Err(why);
    }
    if !crate::rename::pending_intents(root).is_empty()
        || !crate::rename::pending_damaged(root).is_empty()
    {
        return Err("a rename is in progress".to_owned());
    }
    let entries = listing(&sessions)
        .map_err(|why| format!("{} could not be listed ({why})", sessions.display()))?;
    let mut versions = Vec::new();
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let dir = entry.path();
        if let Some(why) = not_real_dir(&dir).or_else(|| meta_node(&dir).err()) {
            return Err(format!("session {name:?}: {why}"));
        }
        let bytes = match crate::meta::read_bytes(&dir) {
            Ok(bytes) => bytes,
            Err(why) if why.kind() == std::io::ErrorKind::NotFound && meta_absent(&dir) => {
                continue;
            }
            Err(why) => return Err(unreadable_refusal(&name, &why)),
        };
        migrate(&String::from_utf8_lossy(&bytes)).map_err(|refusal| refusal.line(&name))?;
        versions.extend(core_versions(&bytes));
    }
    Ok(Some(versions))
}

/// Bring the default root of a [`FleetPlan`], then every named instance root,
/// onto `core`, in name order. A named root in doubt is skipped untouched and
/// one that fails part-way is reported; neither stops the publish, and both are
/// HELD — returned beside the notes — so the version sweep removes nothing,
/// unless the hold would strand a session of the protected cohort.
///
/// # Errors
///
/// The default root's refusal, or a protected session's, with nothing relinked
/// and the ae command not moved.
pub(crate) fn onto_fleet(
    mut plan: FleetPlan,
    core: &Path,
    version: &str,
) -> Result<(Vec<String>, Vec<String>), String> {
    // The default root goes first, so its refusal stays exactly what it was
    // and nothing of a named root moves behind a refusal there.
    let mut notes = onto_planned(&mut plan, core, version)?;
    let FleetPlan {
        named,
        doubts,
        mut moved,
        ..
    } = plan;
    let repair = format!("repair it, then run AE_VERSION={version} ae upgrade");
    let mut held = Vec::new();
    for (label, why) in doubts {
        notes.push(format!("skipped {label}: {why} — {repair}"));
        held.push(label);
    }
    for (name, root, census) in named {
        let label = format!("instance {name}");
        let planned = match census {
            NamedRoot::Held(why) => {
                notes.push(format!("skipped {label}: {why} — {repair}"));
                held.push(label);
                continue;
            }
            // No sessions directory: nothing to move and nothing pinned.
            NamedRoot::Ready(planned) if planned.is_empty() => continue,
            NamedRoot::Ready(planned) => planned,
        };
        let mut stops = false;
        match onto_scoped(
            &root,
            core,
            version,
            Scope::Named,
            planned,
            &mut moved,
            &mut stops,
        ) {
            Ok(done) => notes.extend(done.into_iter().map(|note| format!("{label}: {note}"))),
            // A protected session, or a meta that stopped being readable, found
            // at apply stops the publish before the command is relinked,
            // exactly as it would have at the census. Any other failure holds
            // only this root.
            Err(why) if stops => return Err(format!("{label}: {why}")),
            Err(why) => {
                notes.push(format!("{label}: {why}"));
                held.push(label);
            }
        }
    }
    Ok((notes, held))
}

// ─── the version-directory sweep ─────────────────────────────────────────

/// Delete every `versions/<V>` no session meta names, keeping `published` and
/// whatever `link` currently resolves into.
///
/// This runs AFTER the command link is repointed, which is what makes it safe:
/// by then every placeable session's core rows have been rewritten. Every core
/// row of a skipped unplaceable session is kept too, and an unreadable meta
/// suppresses the whole prune because its references cannot be proved. The
/// resulting set of referenced version directories is therefore complete and
/// current. `published` is never a candidate even when no session names it — a
/// first install has no sessions at all.
///
/// `link` is the second floor, and it is there for the one publisher this
/// process cannot exclude: a core older than the publisher lock takes no lock,
/// so during the rollout that added it, one could repoint the command between
/// this keep-set being built and this sweep running. Deleting the directory the
/// live `ae` resolves into is the one outcome worth a second reading of the
/// world — whatever else a stale keep-set gets wrong costs disk space, and this
/// would cost the command.
///
/// Best effort: a directory that will not go is reported and the rest are
/// still swept. A failed cleanup is disk space, not a broken install.
///
/// Named instance roots under `root` are swept too: their core rows join the
/// keep-set only through `strict_proof`, and any doubt about one removes
/// nothing. With no `<root>/instances` the sweep is the default root's alone.
#[must_use]
pub fn prune_versions(root: &Path, link: &Path, published: &str) -> Vec<String> {
    prune_fleet(root, link, published, &[])
}

/// [`prune_versions`], told which named roots the publish itself HELD — skipped
/// or failed part-way — so it removes nothing while any of them is unrepaired.
#[must_use]
pub(crate) fn prune_fleet(
    root: &Path,
    link: &Path,
    published: &str,
    held: &[String],
) -> Vec<String> {
    if !held.is_empty() {
        return vec![format!(
            "WARNING: no version was removed — {} did not move onto {published}, so the versions it records cannot be ruled out of use",
            held.join(", ")
        )];
    }
    let mut keep: Vec<String> = vec![published.to_owned()];
    if let Some(current) = current_link_version(link)
        && !keep.contains(&current)
    {
        keep.push(current);
    }
    // A CENSUS THAT FAILED IS NOT AN EMPTY CENSUS. Everything below decides
    // what to DELETE from what it did not find, so a keep-set built from a
    // partial reading is a keep-set that authorises removing the core a session
    // is running on. Both the enumeration and every meta read are therefore
    // fatal to the whole sweep, not to one entry: the sweep is skipped and the
    // operator is told, which costs disk space and nothing else.
    let census = match taken(root) {
        Ok(census) => census,
        Err(why) => {
            return vec![format!(
                "WARNING: no version was removed — the sessions root could not be enumerated ({why}), so ae cannot tell which versions are still in use"
            )];
        }
    };
    for name in census {
        let dir = crate::lifecycle::sessions_dir(root).join(&name);
        let bytes = match crate::meta::read_bytes(&dir) {
            Ok(bytes) => bytes,
            // A directory with no meta records no core. Anything else is a
            // session whose pin ae could not read, and pruning past it would be
            // guessing.
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => continue,
            Err(why) => {
                return vec![format!(
                    "WARNING: no version was removed — session {name:?} has a meta ae could not read ({why}), so its core cannot be ruled out of use"
                )];
            }
        };
        keep.extend(core_versions(&bytes));
    }
    let fleet = fleet(root);
    if let Some((label, why)) = fleet.doubts.first() {
        return vec![format!(
            "WARNING: no version was removed — {label}: {why}, so the versions it records cannot be ruled out of use"
        )];
    }
    for (name, named) in &fleet.named {
        match strict_proof(named) {
            Ok(versions) => keep.extend(versions.into_iter().flatten()),
            Err(why) => {
                return vec![format!(
                    "WARNING: no version was removed — instance {name}: {why}, so the versions it records cannot be ruled out of use"
                )];
            }
        }
    }
    let mut notes = Vec::new();
    for dir in version_dirs(root) {
        let Some(name) = dir.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if keep.iter().any(|kept| kept == name) {
            continue;
        }
        match crate::install::remove_private_tree(&dir) {
            Ok(()) => notes.push(format!("removed the unreferenced version {name}")),
            Err(why) => notes.push(format!("could not remove the version {name}: {why}")),
        }
    }
    notes
}

/// The `<V>` the command link resolves into right now, or `None` when it names
/// nothing this sweep would touch.
///
/// Read immediately before the deletions, not with the rest of the keep-set:
/// the whole point is that it may have changed since.
fn current_link_version(link: &Path) -> Option<String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the version sweep asks what the command link names right now, because that directory may never be deleted"
    )]
    let target = std::fs::read_link(link).ok()?;
    version_of(&target.to_string_lossy())
}

/// The `<V>` a recorded `ae_core` path names, read from the path's SHAPE:
/// `…/versions/<V>/ae-core`.
///
/// By shape, never by comparing against this root's absolute `versions` path.
/// The installer records the path as written while other readers canonicalize,
/// and on macOS `/tmp` and `/private/tmp` are the same directory spelled twice
/// — an equality test there would fail to recognise a version a session really
/// does record, and this answer decides what gets DELETED. Reading the shape
/// can only ever keep more than strictly necessary, which is the safe way for
/// this particular function to be wrong.
fn version_of(core: &str) -> Option<String> {
    let path = Path::new(core);
    // The whole shape, terminal member included: `…/versions/<V>/ae-core`.
    if path.file_name()? != crate::shape::CORE {
        return None;
    }
    let dir = path.parent()?;
    if dir.parent()?.file_name()? != crate::shape::VERSIONS {
        return None;
    }
    dir.file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
}

/// Every immediate child of `<root>/versions`, sorted.
fn version_dirs(root: &Path) -> Vec<PathBuf> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the upgrade's version-directory sweep enumerates the versions root it is about to prune — see clippy.toml"
    )]
    let entries = std::fs::read_dir(root.join(crate::shape::VERSIONS));
    let Ok(entries) = entries else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| {
            entry
                .file_type()
                .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
        })
        .map(|entry| entry.path())
        .collect();
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effort_resume_upgrade_chain_preserves_raw_effort_claims() {
        let claims = "effort.main=v1 applied slot:main base:xhigh low:high at:80 ep:1700000000 sid:11111111-1111-4111-8111-111111111111 agent:lead tool:codex launch:tok-1 pid:42 conv:22222222-2222-4222-8222-222222222222 profile:custom client:- model:gpt-6.1-sol win:weekly/~/10080/1700000000/1700000100\neffort.spawned.0\neffort.spawned.0=damaged duplicate\r\neffort.spawned.0=second duplicate\n";
        for version in ["schema=2\n", "meta_version=2\n", "meta_version=3\n"] {
            let before = format!("{version}session=fixture\nseat.main=lead\n{claims}");
            let after = migrate(&before)
                .expect("placeable pre-upgrade meta")
                .expect("migration performed")
                .text;
            assert!(
                after.contains(claims),
                "ordered raw claims survive: {after:?}"
            );
            assert!(after.contains("session=fixture\nseat.main=lead\n"));
            assert!(migrate(&after).expect("current meta").is_none());
        }
    }

    #[test]
    fn a_core_row_names_its_version_by_shape_and_an_empty_row_names_none() {
        let meta = b"ae_core=/h/.ae/versions/2026.1.1/ae-core\r\nae_core_version=2026.2.2\n";
        assert_eq!(core_versions(meta), ["2026.1.1", "2026.2.2"]);
        let none = b"ae_core=/h/.ae/elsewhere/ae-core\nae_core_version=\n";
        assert!(core_versions(none).is_empty());
    }

    #[test]
    fn a_named_abort_names_the_repair_and_never_a_command_left_in_place() {
        for repointed in [Vec::new(), vec!["demo".to_owned()]] {
            let said = already(Scope::Named, &repointed, "2099.1.1");
            assert!(said.contains("AE_VERSION=2099.1.1 ae upgrade"), "{said}");
            assert!(
                !said.to_lowercase().contains("command was not moved"),
                "{said}"
            );
        }
        assert!(already(Scope::Default, &[], "2099.1.1").contains("ae command was not moved"));
        let (root, why) = (Path::new("/h/.ae"), std::io::Error::other("denied"));
        assert_eq!(
            census_failed(Scope::Default, root, &why),
            "the sessions under /h/.ae/sessions could not be enumerated (denied); nothing was \
             migrated and the ae command was not moved"
        );
        assert!(!census_failed(Scope::Named, root, &why).contains("command"));
    }

    /// A scratch named root holding one session directory, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("ae-p2-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("sessions/demo")).expect("scratch root");
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_link_in_a_named_root_is_doubt_and_never_absence() {
        // A readable meta link and a dangling one: neither is a meta, neither is none.
        for dangling in [false, true] {
            let root = Scratch::new(if dangling { "dangling" } else { "linked" });
            let target = root.0.join("elsewhere");
            if !dangling {
                std::fs::write(&target, "meta_version=4\nae_core_version=2026.1.1\n")
                    .expect("meta");
            }
            std::os::unix::fs::symlink(&target, root.0.join("sessions/demo/meta")).expect("link");
            let why = strict_proof(&root.0).expect_err("a linked meta is doubt");
            assert!(why.contains("not a regular file"), "{why}");
        }
        // A sessions link is refused BEFORE the rename scan reads a carrier through it.
        let root = Scratch::new("sessions");
        let outside = root.0.join("outside");
        std::fs::create_dir(&outside).expect("outside");
        std::fs::write(outside.join(".rename.0000000000000000.intent"), "x").expect("carrier");
        std::fs::remove_dir_all(root.0.join("sessions")).expect("clear sessions");
        std::os::unix::fs::symlink(&outside, root.0.join("sessions")).expect("sessions link");
        let why = strict_proof(&root.0).expect_err("a sessions link is doubt");
        assert!(why.contains("not a real directory"), "{why}");
    }

    #[test]
    fn a_named_chain_is_proved_from_its_container_down_and_a_gap_is_doubt() {
        let scratch = Scratch::new("chain");
        let root = scratch.0.join("instances/a");
        let dir = root.join("sessions/demo");
        std::fs::create_dir_all(&dir).expect("named session");
        std::fs::write(dir.join("meta"), "meta_version=4\n").expect("meta");
        assert_eq!(named_chain(&root, Some(&dir)), None);
        assert!(named_chain(&scratch.0.join("instances/gone"), Some(&dir)).is_some());
        // An ancestor swapped for a link: the session still lstats as a real dir.
        let moved = scratch.0.join("moved");
        std::fs::rename(root.join("sessions"), &moved).expect("move sessions");
        std::os::unix::fs::symlink(&moved, root.join("sessions")).expect("sessions link");
        assert!(lstat(&dir).is_ok_and(|node| node.is_dir()));
        assert!(named_chain(&root, Some(&dir)).is_some_and(|why| why.contains("sessions")));
        std::fs::rename(scratch.0.join("instances"), scratch.0.join("real")).expect("move roots");
        std::os::unix::fs::symlink(scratch.0.join("real"), scratch.0.join("instances"))
            .expect("link");
        assert!(named_root_doubt(&root).is_some_and(|why| why.contains("instances")));
    }

    #[test]
    fn a_meta_at_the_current_version_is_already_where_the_chain_ends() {
        assert_eq!(
            migrate(&format!("mode=local\n{KEY}={CURRENT}\nwork_dir=/w\n")),
            Ok(None)
        );
    }

    #[test]
    fn a_meta_that_says_only_schema_2_is_placed_at_two_and_stamped() {
        // The pre-chain v2 shape, which is what every session predating the
        // chain actually looks like. Placed, never refused.
        let text = "mode=local\nschema=2\nseat.main=lead\n";
        let migrated = migrate(text).expect("placed").expect("a stamp");
        // Placed at two, then STEPPED: the row is stamped in at the version the
        // chain walked it to, and a pre-chain session is never refused.
        assert_eq!(migrated.what, Stepped::From(PRE_CHAIN));
        assert_eq!(migrated.text, format!("{text}{KEY}={CURRENT}\n"));
        // Idempotent, and the placement rule is the one `ae list` uses.
        assert_eq!(migrate(&migrated.text), Ok(None));
        assert_eq!(placed(None, Some("2")), Some(PRE_CHAIN));
        assert_eq!(placed(None, Some("3")), None);
        assert_eq!(placed(None, None), None);
        assert_eq!(placed(Some("2"), None), Some(2));
        // A `schema` named twice says nothing, so it places nothing.
        assert_eq!(
            migrate("schema=2\nschema=2\n").expect_err("ambiguous"),
            Refusal::Absent
        );
    }

    #[test]
    fn a_meta_with_no_version_row_earns_the_fresh_start_refusal() {
        let refused =
            migrate("mode=local\nwork_dir=/w\nagent.main=lead\n").expect_err("the v1 past");
        assert_eq!(refused, Refusal::Absent);
        let line = refused.line("proj");
        assert!(line.contains("ae end proj"), "{line}");
        assert!(line.contains("meta_version"), "{line}");
    }

    #[test]
    fn a_version_row_that_does_not_say_one_number_is_unreadable() {
        for hostile in [
            "meta_version=2\nmeta_version=2\n",
            "schema=2\nmeta_version=v2\n",
            "meta_version=\n",
            "meta_version=v2\n",
            "meta_version=2 \n",
            "meta_version=-1\n",
            "meta_version=99999999999999999999\n",
        ] {
            let refused = migrate(hostile).expect_err("a hostile version row");
            assert!(
                matches!(refused, Refusal::Unreadable(_)),
                "{hostile:?} gave {refused:?}"
            );
        }
    }

    #[test]
    fn the_chain_reads_a_meta_line_exactly_the_way_the_meta_parser_does() {
        // ONE READER'S RULES, TWO READERS. A CRLF meta parsed as `2` in
        // `Meta::parse` and as `2\r` here, so `ae list` called the session
        // current while every resume refused it as unreadable — a session no
        // command agreed about.
        let crlf = format!("mode=local\r\n{KEY}={CURRENT}\r\n");
        assert_eq!(migrate(&crlf), Ok(None));
        let current = CURRENT.to_string();
        assert_eq!(
            crate::meta::Meta::parse(&crlf).meta_version(),
            Some(current.as_str())
        );

        // And a row the writer MEANT to give and this reader cannot take is
        // unreadable, not absent: it must not earn the fresh-start message,
        // which tells the operator their session pre-dates the chain.
        let bare = migrate("mode=local\nmeta_version\n").expect_err("a valueless row");
        assert!(
            matches!(bare, Refusal::Unreadable(_)),
            "a bare row read as {bare:?}"
        );
        assert!(!bare.line("proj").contains("ae end proj"), "{bare:?}");
    }

    #[test]
    fn a_version_ahead_of_this_core_is_refused_rather_than_stepped_backwards() {
        let ahead = CURRENT + 1;
        let refused = migrate(&format!("{KEY}={ahead}\n")).expect_err("a newer writer");
        assert_eq!(refused, Refusal::Ahead(ahead));
        assert!(refused.line("proj").contains("upgrade ae"));
    }

    #[test]
    fn a_version_the_chain_has_no_step_out_of_names_itself() {
        // The chain starts at 2, so every version below that lands here.
        let refused = migrate("meta_version=1\n").expect_err("no step");
        assert_eq!(refused, Refusal::NoStep(1));
        assert!(refused.line("proj").contains("meta_version=1"));
    }

    #[test]
    fn the_row_a_launch_writes_reads_back_as_the_current_version() {
        assert_eq!(declared(&format!("{KEY}={CURRENT}\n")), Ok((CURRENT, true)));
    }

    #[test]
    fn a_recorded_core_is_placed_by_the_shape_of_its_path() {
        assert_eq!(
            version_of("/u/me/.ae/versions/2026.9.2/ae-core"),
            Some("2026.9.2".to_owned())
        );
        // The SAME directory, spelled the way macOS also spells it. An
        // equality test against one root answered None here and pruned a
        // version a session was still running.
        assert_eq!(
            version_of("/private/tmp/h/.ae/versions/2026.9.2/ae-core"),
            Some("2026.9.2".to_owned())
        );
        for foreign in [
            "/u/me/.ae/versions/ae-core",
            "/u/me/.ae/versions/2026.9.2/nested/ae-core",
            "/u/me/.ae/other/2026.9.2/ae-core",
            "/u/me/.ae/versions/2026.9.2/not-ae-core",
            "ae-core",
            "",
        ] {
            assert_eq!(version_of(foreign), None, "placed {foreign}");
        }
    }

    #[test]
    fn the_3_to_4_step_keeps_every_row_byte_identical() {
        assert_eq!(CURRENT, 4, "P1 bumps the chain for the seat-dir guard");
        let text = "mode=local\nmeta_version=3\nwork_dir=/w\nseat.main=lead\nwork_dir.main=/t\n";
        let migrated = migrate(text).expect("a step").expect("a migration");
        assert_eq!(migrated.what, Stepped::From(3));
        for row in [
            "mode=local\n",
            "work_dir=/w\n",
            "seat.main=lead\n",
            "work_dir.main=/t\n",
        ] {
            assert!(migrated.text.contains(row), "{row} lost: {}", migrated.text);
        }
        assert!(
            migrated.text.contains(&format!("{KEY}=4\n")),
            "{}",
            migrated.text
        );
        assert_eq!(migrate(&migrated.text), Ok(None));
    }

    #[test]
    fn a_schema_2_meta_walks_all_the_way_to_four() {
        let migrated = migrate("mode=local\nschema=2\nseat.main=lead\n")
            .expect("placed")
            .expect("stepped");
        assert_eq!(migrated.what, Stepped::From(PRE_CHAIN));
        assert!(
            migrated.text.contains(&format!("{KEY}={CURRENT}\n")),
            "{}",
            migrated.text
        );
        assert_eq!(migrate(&migrated.text), Ok(None));
    }
    /// A placeable (or, with a high `version`, unplaceable) session meta under
    /// `root`, with a server row that points at a socket nothing listens on, so
    /// no probe can reach a real tmux server.
    fn planted(root: &Path, name: &str, role: &str, version: &str) {
        let dir = root.join("sessions").join(name);
        std::fs::create_dir_all(&dir).expect("session dir");
        let socket = root.join("no-such-socket");
        let text = format!(
            "meta_version={version}\nschema=2\nmode=local\nsession={name}\n\
             session_id=0199c0de-aaaa-4890-abcd-ef0123456789\nwork_dir=/w\nseat.main=boss\n\
             profile.main=cl\ntmux_server_kind=socket\ntmux_server={}\n{role}",
            socket.display()
        );
        std::fs::write(dir.join("meta"), text).expect("meta");
    }

    #[test]
    fn the_recheck_keeps_an_unchanged_move_and_asks_again_of_anything_else() {
        let scratch = Scratch::new("recheck");
        let root = &scratch.0;
        let dir = root.join("sessions/a-ok");
        planted(root, "a-ok", "", &CURRENT.to_string());
        let read = crate::meta::read_bytes(&dir);
        let planned = judge(Scope::Default, root, "a-ok", false, &read).expect("placeable");
        assert_eq!(planned.judged, Judged::Move);
        assert!(!planned.protected && !planned.seat);
        assert_eq!(
            recheck(Scope::Default, root, &planned, &read).expect("unchanged"),
            planned
        );
        // An ordinary change is judged anew: unplaceable and stopped, it is
        // skipped, not refused.
        planted(root, "a-ok", "", &u32::MAX.to_string());
        let changed = crate::meta::read_bytes(&dir);
        let again = recheck(Scope::Default, root, &planned, &changed).expect("judged again");
        assert!(matches!(again.judged, Judged::Skip(_)), "{again:?}");
        // A session of the protected cohort whose meta changed at all is a
        // late refusal, even when the new bytes drop the role and place.
        planted(root, "p", "meta_agent=true\n", &CURRENT.to_string());
        let seat = root.join("sessions/p");
        let read = crate::meta::read_bytes(&seat);
        let protected = judge(Scope::Default, root, "p", false, &read).expect("placeable");
        assert!(protected.protected);
        planted(root, "p", "", &CURRENT.to_string());
        let dropped = crate::meta::read_bytes(&seat);
        assert!(
            matches!(
                recheck(Scope::Default, root, &protected, &dropped),
                Err(Stop::Fatal(why)) if why.contains("meta_agent") && why.contains("changed since")
            ),
            "a protected session's change is a late refusal"
        );
    }

    #[test]
    fn a_protected_session_that_changed_after_the_plan_stops_the_publish_naming_what_moved() {
        let scratch = Scratch::new("late-protected");
        let root = &scratch.0;
        planted(root, "a-ok", "", &CURRENT.to_string());
        planted(root, "z-seat", "meta_agent=true\n", &CURRENT.to_string());
        let plan = plan_fleet(root).expect("both sessions are placeable");
        // After the plan, the seat's meta changes and no longer claims the role.
        let seat = root.join("sessions/z-seat/meta");
        let changed = String::from_utf8_lossy(
            &crate::meta::read_bytes(&root.join("sessions/z-seat")).expect("meta"),
        )
        .replace("meta_agent=true\n", "");
        std::fs::write(&seat, &changed).expect("changed after the plan");
        let core = root.join("new-core");
        std::fs::write(&core, b"").expect("core");
        let why =
            onto_fleet(plan, &core, "2099.1.1").expect_err("the late change stops the publish");
        assert!(
            why.contains("z-seat")
                && why.contains("meta_agent")
                && why.contains("repair or end it"),
            "{why}"
        );
        assert!(why.contains("NOT moved") && why.contains("a-ok"), "{why}");
        assert_eq!(
            String::from_utf8_lossy(
                &crate::meta::read_bytes(&root.join("sessions/z-seat")).expect("meta")
            ),
            changed,
            "the changed session was not touched"
        );
    }

    #[test]
    fn a_held_named_root_still_refuses_a_protected_session_behind_ordinary_damage() {
        let scratch = Scratch::new("held-protected");
        let root = &scratch.0;
        let ahead = u32::MAX.to_string();
        planted(root, "a-damaged", "", &ahead);
        let held = census_named(root).expect("ordinary damage holds the root");
        assert!(
            matches!(held, NamedRoot::Held(ref why) if why.contains("a-damaged")),
            "{held:?}"
        );
        planted(root, "z-seat", "meta_agent=true\n", &ahead);
        let why = census_named(root).expect_err("the protected session stops the publish");
        assert!(
            why.contains("z-seat")
                && why.contains("repair or end it")
                && why.contains("meta_agent"),
            "{why}"
        );
    }
    #[test]
    fn a_planned_missing_meta_that_became_unreadable_is_not_skipped() {
        use std::os::unix::fs::PermissionsExt as _;
        let scratch = Scratch::new("nometa-denied");
        let root = &scratch.0;
        let dir = root.join("sessions/demo");
        let read = crate::meta::read_bytes(&dir);
        let planned =
            judge(Scope::Default, root, "demo", false, &read).expect("no meta is skipped");
        assert_eq!(planned.judged, Judged::NoMeta);
        // A directory at the meta path cannot be read, whoever asks.
        std::fs::create_dir(dir.join("meta")).expect("a present, unreadable meta");
        let now = crate::meta::read_bytes(&dir);
        assert!(
            matches!(
                recheck(Scope::Default, root, &planned, &now),
                Err(Stop::Fatal(why)) if why.contains("meta could not be read")
            ),
            "a present unreadable meta hides behind a planned NoMeta"
        );
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755));
    }

    #[test]
    fn a_hold_that_strands_a_planned_seat_refuses_and_one_that_does_not_holds() {
        let scratch = Scratch::new("strand");
        let root = &scratch.0;
        planted(root, "a-damaged", "", &u32::MAX.to_string());
        planted(root, "z-seat", "meta_agent=true\n", &CURRENT.to_string());
        let why = census_named(root).expect_err("the hold would strand the seat");
        assert!(
            why.contains("z-seat") && why.contains("repair or end it") && why.contains("old core"),
            "{why}"
        );
        std::fs::remove_dir_all(root.join("sessions/z-seat")).expect("seat gone");
        assert!(matches!(
            census_named(root).expect("an ordinary hold"),
            NamedRoot::Held(_)
        ));
    }

    #[test]
    fn a_failure_that_keeps_a_named_sweep_from_a_planned_seat_stops_the_publish() {
        let scratch = Scratch::new("late-strand");
        let root = &scratch.0;
        planted(root, "a-ordinary", "", &CURRENT.to_string());
        planted(root, "z-seat", "meta_agent=true\n", &CURRENT.to_string());
        let NamedRoot::Ready(planned) = census_named(root).expect("both placeable") else {
            panic!("a root with no doubt is ready");
        };
        // A helper path that is a directory refuses the helper relink of the
        // ordinary session, so the sweep aborts before it reaches the seat.
        std::fs::create_dir(root.join("sessions/a-ordinary/send")).expect("obstruction");
        let core = root.join("new-core");
        std::fs::write(&core, b"").expect("core");
        // The same names moved in the default root and in another instance: only
        // THIS root's exact identity counts, so neither stands in for the seat.
        let (mut moved, mut stops) = (vec!["z-seat".to_owned(), "alpha/z-seat".to_owned()], false);
        let why = onto_scoped(
            root,
            &core,
            "2099.1.1",
            Scope::Named,
            planned,
            &mut moved,
            &mut stops,
        )
        .expect_err("the ordinary failure aborts the sweep");
        assert!(stops, "a stranded seat stops the publish: {why}");
        assert!(why.contains("z-seat") && why.contains("NOT moved"), "{why}");
    }
    /// A damaged rename carrier of `old` → `new` under `root`, as a rename that
    /// began after the census leaves it.
    fn carrier(root: &Path, old: &str, new: &str) {
        std::fs::write(
            root.join("sessions")
                .join(format!(".rename.{old}.{new}.intent")),
            "broken\n",
        )
        .expect("carrier");
    }

    #[test]
    fn a_rename_that_began_after_the_census_skips_an_ordinary_session_and_stops_for_a_seat() {
        for (owned, seat) in [("a-ordinary", false), ("z-seat", true)] {
            let scratch = Scratch::new(if seat {
                "late-rename-seat"
            } else {
                "late-rename"
            });
            let root = &scratch.0;
            planted(root, "a-ordinary", "", &CURRENT.to_string());
            planted(root, "z-seat", "meta_agent=true\n", &CURRENT.to_string());
            let NamedRoot::Ready(planned) = census_named(root).expect("both placeable") else {
                panic!("a root with no doubt is ready");
            };
            // The census saw no carrier; one appears, with the metas unchanged.
            carrier(root, owned, &format!("{owned}-new"));
            let before = crate::meta::read_bytes(&root.join("sessions").join(owned)).expect("meta");
            let core = root.join("new-core");
            std::fs::write(&core, b"").expect("core");
            let (mut moved, mut stops) = (Vec::new(), false);
            let done = onto_scoped(
                root,
                &core,
                "2099.1.1",
                Scope::Named,
                planned,
                &mut moved,
                &mut stops,
            );
            assert_eq!(
                crate::meta::read_bytes(&root.join("sessions").join(owned)).expect("meta"),
                before,
                "{owned}: a session a rename owns was not touched"
            );
            if seat {
                let why = done.expect_err("an orchestrator session cannot be skipped");
                assert!(stops, "{why}");
                assert!(
                    why.contains("z-seat")
                        && why.contains("pending rename")
                        && why.contains("NOT moved"),
                    "{why}"
                );
            } else {
                let notes = done.expect("the ordinary session is skipped, the sweep goes on");
                assert!(
                    notes.iter().any(|note| note.contains("skipped a-ordinary")),
                    "{notes:?}"
                );
                assert!(!stops && moved.len() == 1, "only the seat moved: {moved:?}");
            }
        }
    }
    #[test]
    fn a_reseed_never_overwrites_a_seat_config_that_changed_or_became_a_link() {
        let scratch = Scratch::new("reseed-recheck");
        let dir = &scratch.0;
        let path = dir.join("orchestrator.config");
        std::fs::write(&path, "known\n").expect("seat config");
        // The decision was made on other bytes: the owner edited it since.
        assert!(publish_reseed(&path, b"older\n", "new\n").is_err());
        assert_eq!(
            crate::store::read_capped(&path, SEAT_CONFIG_CAP).ok(),
            Some(crate::store::SourceRead::Ready(b"known\n".to_vec()))
        );
        // A link now stands in the file's place, with the bytes the decision saw.
        let target = dir.join("dotfiles-copy");
        std::fs::write(&target, "known\n").expect("target");
        std::fs::remove_file(&path).expect("unlink");
        std::os::unix::fs::symlink(&target, &path).expect("link");
        assert!(publish_reseed(&path, b"known\n", "new\n").is_err());
        assert!(
            !matches!(
                crate::store::read_capped(&path, SEAT_CONFIG_CAP),
                Ok(crate::store::SourceRead::Ready(_))
            ),
            "the link was not replaced by a regular file"
        );
        assert_eq!(
            crate::store::read_capped(&target, SEAT_CONFIG_CAP).ok(),
            Some(crate::store::SourceRead::Ready(b"known\n".to_vec()))
        );
        // Unchanged, it is replaced.
        std::fs::remove_file(&path).expect("unlink");
        std::fs::write(&path, "known\n").expect("seat config");
        assert!(publish_reseed(&path, b"known\n", "new\n").is_ok());
        assert_eq!(
            crate::store::read_capped(&path, SEAT_CONFIG_CAP).ok(),
            Some(crate::store::SourceRead::Ready(b"new\n".to_vec()))
        );
    }
}
