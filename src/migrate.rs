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
//! [`CURRENT`]. Today it holds NO steps: `CURRENT` is 2, every live session is
//! already 2, and what the module does is the version CHECK. That is
//! deliberate — the chain exists so the first real shape change is a step with
//! a fixture beside it rather than a rewrite of every reader, and an empty
//! chain that is already wired at every door is worth more than a full one
//! wired nowhere.
//!
//! It runs wherever the core TOUCHES a session:
//!
//! * resume/attach ([`crate::session_launch`]) — a refusal stops the launch, and
//!   a placed meta is stamped there, which is the first touch most sessions get;
//! * `stop` and `end` ([`crate::lifecycle`]) — a refusal is REPORTED and the
//!   operation continues, because a session that cannot be migrated is exactly
//!   the one an operator needs to be able to tear down;
//! * `ae upgrade` / `ae _install` ([`crate::install::publish`]) — the whole
//!   sweep below, and there a refusal aborts the upgrade before the command
//!   link is repointed.
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
//! EVERY session — running or stopped — is stepped, re-pointed at the new core
//! and re-linked, and the two daemons of a running one are restarted so they
//! run the core the rest of the install just became. Agent panes are never
//! touched: they run the agent tool, not ae.

use std::path::{Path, PathBuf};

use crate::inventory::ServerId;

/// The row that carries the shape, in every session meta.
pub const KEY: &str = "meta_version";

/// The shape this core reads and writes.
pub const CURRENT: u32 = 2;

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

/// The chain, in order. EMPTY today — see the module docs.
///
/// A new step is `Step { from: N, apply: … }` plus a fixture in
/// `tests/it/migrate.rs` that carries a real meta at N and asserts what N+1
/// makes of it.
const STEPS: &[Step] = &[];

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
    /// [`Refusal::Absent`] carries the fresh-start wording the retired v1
    /// roster already uses: there is one way forward from a meta ae cannot
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
/// let current = ae::migrate::migrate("mode=local\nmeta_version=2\n");
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

/// The three rows a session records about the core it was built against.
const CORE_ROWS: [&str; 2] = ["ae_core", "ae_core_version"];

/// The retired glue's own path row. REWRITTEN where a session still carries
/// one and never introduced: a pre-Z3 meta names the deleted wrapper, and an
/// upgrade that left that pointing into a version directory it is about to
/// delete would leave a session naming a binary that is gone.
const LEGACY_PATH_ROW: &str = "ae_path";

/// [`crate::lifecycle::census`] with ONE failure downgraded: a state root with
/// no `sessions/` directory at all has no sessions, and saying so is not a
/// guess. That is a first install, and it is the one reading where "found
/// nothing" and "could not look" genuinely coincide. Every other error stays an
/// error, because it means the directory is there and ae could not read it.
///
/// # Errors
///
/// Whatever the census hit, unless it was a missing sessions root.
fn taken(root: &Path) -> std::io::Result<Vec<String>> {
    match crate::lifecycle::census(root) {
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        other => other,
    }
}

/// Bring every session under `root` onto `core`, and restart the daemons of
/// the running ones.
///
/// This is the upgrade's step, and it runs AFTER the new version directory is
/// published and BEFORE the command link names it — so a session that cannot
/// be migrated aborts the upgrade while the old core is still the current one.
///
/// Per session, under the same per-session lifecycle lock a start or an end
/// takes: the chain, then `ae_core` / `ae_core_version` / `ae_version` (and a
/// legacy `ae_path`) rewritten to the new core, then the helper links
/// re-rendered. A RUNNING session then has its watchdog restarted, and the
/// Telegram bridge on its tmux server restarted once per server. Agent panes
/// are never touched.
///
/// # Errors
///
/// The first session that could not be migrated, named — nothing after it is
/// attempted.
pub fn onto(root: &Path, core: &Path, version: &str) -> Result<Vec<String>, String> {
    // PASS ONE, READ-ONLY: every session is asked whether the chain can place
    // it, and NOTHING is written until they all can. Without this the sweep
    // could repoint the first sessions and then refuse on the fourth, leaving
    // them pointing into a version directory the publish is about to roll back
    // — which is the one outcome "aborts before the repoint" must not mean.
    let census = taken(root).map_err(|why| {
        format!(
            "the sessions under {} could not be enumerated ({why}); nothing was migrated and the ae command was not moved",
            crate::lifecycle::sessions_dir(root).display()
        )
    })?;
    for name in &census {
        let dir = crate::lifecycle::sessions_dir(root).join(name);
        let text = match crate::meta::read_bytes(&dir) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            // No meta, or one this process cannot read: pass two decides what
            // that means, and an unreadable meta is not a refusal to migrate.
            // A directory with NO meta is not a session and pass two skips
            // it. Any OTHER read failure is refused here, while refusing is
            // still free: pass two would meet the same error after it had
            // already repointed every session before this one.
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => continue,
            Err(why) => {
                return Err(Refusal::Io(format!("meta could not be read: {why}")).line(name));
            }
        };
        migrate(&text).map_err(|refusal| refusal.line(name))?;
    }

    let mut notes = Vec::new();
    let mut bridges: Vec<ServerId> = Vec::new();
    // WHAT PASS TWO HAS ALREADY DONE, so a failure inside it can say so. Pass
    // one removes the common reason to fail here, but it cannot make this pass
    // atomic: a lock can time out, a meta can stop being readable, a helper
    // destination can refuse to be replaced. An abort after any of those owes
    // the operator the sessions that DID move rather than a claim that none
    // did.
    let mut repointed: Vec<String> = Vec::new();
    // Stamps are COUNTED, not listed: on the release that adds the chain every
    // session on the machine takes one, and 28 identical lines say less than
    // one line with a number on it.
    let mut stamped = 0_usize;
    for name in census {
        let dir = crate::lifecycle::sessions_dir(root).join(&name);
        // The lifecycle lock, so a resume or an end cannot land inside a
        // migration. A session whose lock is held is a session in the middle
        // of exactly the operation this sweep must not race.
        let Ok(_guard) = crate::lifecycle::lock(root, &name) else {
            return Err(format!(
                "session {name:?} is busy (another start, resume or end is in progress); \
                 retry the upgrade shortly.{}",
                already(&repointed, version)
            ));
        };
        match session(&dir) {
            Ok(None) => {}
            Ok(Some(Stepped::Stamped)) => stamped += 1,
            Ok(Some(Stepped::From(from))) => {
                notes.push(format!("migrated {name} from {KEY}={from} to {CURRENT}"));
            }
            // A directory under `sessions` with no meta is not a session: there
            // is nothing to step, nothing to repoint and no reason to fail an
            // upgrade over it.
            Err(Refusal::Missing) => {
                notes.push(format!("skipped {name}: no meta"));
                continue;
            }
            Err(refusal) => {
                return Err(format!(
                    "{}{}",
                    refusal.line(&name),
                    already(&repointed, version)
                ));
            }
        }
        repoint(&dir, core, version)
            .map_err(|why| format!("session {name:?}: {why}.{}", already(&repointed, version)))?;
        // RECORDED HERE, not after the helpers. The meta already names the new
        // core at this point, so a helper failure below leaves this session
        // repointed — reporting it as untouched would send an operator looking
        // in the wrong place.
        repointed.push(name.clone());
        crate::session_launch::assets::write_helpers(&dir, core).map_err(|why| {
            format!(
                "session {name:?}: {why}. Its meta already names {version} while some of its \
                 helpers still name the old core, so it is PARTIALLY relinked.{}",
                already(&repointed, version)
            )
        })?;
        notes.extend(restart_daemons(root, &dir, &name, core, &mut bridges));
    }
    if stamped > 0 {
        notes.insert(
            0,
            format!("stamped {KEY}={CURRENT} into {stamped} session(s) that carried only schema=2"),
        );
    }
    Ok(notes)
}

/// The sentence an abort owes the operator: which sessions already moved.
fn already(repointed: &[String], version: &str) -> String {
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

/// Rewrite the core rows of one session's meta, as ONE document.
///
/// Under a single hold of the meta lock, published once. Four separate
/// [`crate::meta::rewrite`] calls would each take the lock, read, write and
/// rename on their own, so a process that died between them would leave a meta
/// naming the new core at one row and the old core at another — a session that
/// disagrees with itself about which binary it runs.
fn repoint(dir: &Path, core: &Path, version: &str) -> Result<(), String> {
    let _held = crate::meta::lock(dir).map_err(|why| format!("meta.lock: {why}"))?;
    let mut text = crate::meta::read_base(&dir.join(crate::meta::FILE))
        .map_err(|why| format!("meta could not be read: {why}"))?;
    let core_text = core.display().to_string();
    let mut rows: Vec<(&str, &str)> = vec![
        (CORE_ROWS[0], core_text.as_str()),
        (CORE_ROWS[1], version),
        ("ae_version", version),
    ];
    // The legacy row is rewritten where one is already there and never
    // introduced, so a meta that never named the deleted wrapper does not
    // start naming a replacement for it.
    if text
        .lines()
        .any(|line| line.starts_with(&format!("{LEGACY_PATH_ROW}=")))
    {
        rows.push((LEGACY_PATH_ROW, core_text.as_str()));
    }
    for (key, value) in rows {
        text = crate::meta::rewritten(&text, key, Some(value));
    }
    crate::meta::publish_locked(dir, &text)
        .map_err(|why| format!("meta could not be written ({})", why.cause()))
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
    if matches!(
        crate::watchdog_lifecycle::presence(&server, name, dir),
        crate::watchdog_lifecycle::Presence::Running(_)
    ) {
        notes.push(restart_watchdog(root, &server, name, dir));
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
        match crate::watchdog_lifecycle::run(root, &tail, &mut out, &mut err) {
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

// ─── the version-directory sweep ─────────────────────────────────────────

/// Delete every `versions/<V>` no session meta names, keeping `published`.
///
/// This runs AFTER the command link is repointed, which is what makes it safe:
/// by then every session's `ae_core` has been rewritten, so the set of
/// referenced version directories is complete and current. `published` is
/// never a candidate even when no session names it — a first install has no
/// sessions at all.
///
/// Best effort: a directory that will not go is reported and the rest are
/// still swept. A failed cleanup is disk space, not a broken install.
#[must_use]
pub fn prune_versions(root: &Path, published: &str) -> Vec<String> {
    let mut keep: Vec<String> = vec![published.to_owned()];
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
        let Some(value) = crate::meta::first_value(&bytes, CORE_ROWS[0]) else {
            continue;
        };
        if let Some(version) = version_of(&String::from_utf8_lossy(value))
            && !keep.contains(&version)
        {
            keep.push(version);
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
    fn a_meta_at_the_current_version_is_already_where_the_chain_ends() {
        assert_eq!(
            migrate("mode=local\nmeta_version=2\nwork_dir=/w\n"),
            Ok(None)
        );
    }

    #[test]
    fn a_meta_that_says_only_schema_2_is_placed_at_two_and_stamped() {
        // The pre-chain v2 shape, which is what every session predating the
        // chain actually looks like. Placed, never refused.
        let text = "mode=local\nschema=2\nseat.main=lead\n";
        let migrated = migrate(text).expect("placed").expect("a stamp");
        assert_eq!(migrated.what, Stepped::Stamped);
        assert_eq!(migrated.text, format!("{text}{KEY}={CURRENT}\n"));
        // Idempotent, and the placement rule is the one `ae list` uses.
        assert_eq!(migrate(&migrated.text), Ok(None));
        assert_eq!(placed(None, Some("2")), Some(CURRENT));
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
        let crlf = "mode=local\r\nmeta_version=2\r\n";
        assert_eq!(migrate(crlf), Ok(None));
        assert_eq!(crate::meta::Meta::parse(crlf).meta_version(), Some("2"));

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
        let refused = migrate("meta_version=3\n").expect_err("a newer writer");
        assert_eq!(refused, Refusal::Ahead(3));
        assert!(refused.line("proj").contains("upgrade ae"));
    }

    #[test]
    fn a_version_the_chain_has_no_step_out_of_names_itself() {
        // The chain is empty today, so every version below CURRENT lands here.
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
}
