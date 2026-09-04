//! The orchestrator's fleet sweep — what CHANGED since the last one, deduped.
//!
//! The orchestrator agent phrases and orchestrates well and keeps books badly:
//! left to hold attention state in its own memory it drifts — freeform notes,
//! hand-written wrong clocks, an alert announced twice or not at all. This
//! module owns the deterministic half. Given the world [`crate::listing`]
//! renders and a locked state file it computes what changed, dedups it, and
//! prints ready-to-send report lines. Empty output IS the answer: nothing
//! needed reporting.
//!
//! It replaces `contrib/aemonitor`, a 366-line Python sidecar that read
//! `ae list --json` out of a subprocess and re-derived the attention vocabulary
//! for itself. Three things went with that port, and each was a defect rather
//! than a feature:
//!
//! * **The JSON round trip.** The sidecar parsed the digest and refused any
//!   `schema_version` but `1` — and the core has emitted `2` since the identity
//!   slice, so the shipped helper failed closed against the shipped `ae`. This
//!   reads [`World`] directly, so there is no document between the fact and its
//!   reader and no version for the two to disagree about.
//! * **The duplicated rank table.** `RANK` mirrored ae's severity order in
//!   Python and silently IGNORED any reason ae later added — which is exactly
//!   what happened to `unanswered`. [`Reason`] is the one definition now, so a
//!   new reason is reported the day it exists.
//! * **`--notify-cmd <path>`.** An arbitrary program, chosen by whoever wrote
//!   the charter. Delivery goes through the session's OWN `say` helper, at the
//!   fixed name [`SAY_HELPER`] joined onto the session directory — the
//!   [`crate::watchdog_daemon`]'s rule for the same hazard, and the reason
//!   [`Notice`] has one constructor.
//!
//! # The delivery-aware dedup (the guarantee worth stating)
//!
//! `last_seen` advances every sweep; `notified` advances only after `say`
//! exits zero. So a change that was SEEN but not DELIVERED is re-reported next
//! sweep until it lands, and a failed or forgotten send can never permanently
//! swallow an alert. That asymmetry is the whole contract — see
//! [`Outcome::commit`].
//!
//! # Where the state lives
//!
//! `<session-dir>/meta-agent-state.json`, and the name is
//! [`crate::watchdog_daemon::HEARTBEAT_NAME`] rather than a second literal:
//! the watchdog reads that file's mtime as proof the orchestrator is still
//! sweeping, so a monitor writing anywhere else would leave the wedge check
//! watching a file nobody writes. One constant, and the compiler keeps the two
//! agreed.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::attention::Reason;
use crate::digest::Status;
use crate::json::{self, Value};
use crate::listing::World;
use crate::watchdog_daemon::HEARTBEAT_NAME;

/// The state document's version. Bumping it makes every older file read as
/// empty, which is a clean restart and not a migration.
pub const SCHEMA: i64 = 1;

/// The state file's name, under the session directory — the watchdog's
/// heartbeat, so there is exactly one spelling of it in the crate.
pub const STATE_NAME: &str = HEARTBEAT_NAME;

/// The helper a report is delivered through: `<session-dir>/say`.
pub const SAY_HELPER: &str = "say";

/// A non-`done` session idle longer than this is "quiet" — 20 minutes.
pub const DEFAULT_QUIET_SECS: i64 = 1200;

/// Silent sweeps before a "still watching" ping — 36, about three hours at the
/// watchdog's five-minute cadence.
pub const DEFAULT_LIVENESS_SWEEPS: u64 = 36;

/// The usage line, quoted verbatim by the one refusal that raises it.
pub const USAGE: &str = "Usage: _monitor sweep <session-dir> [--now EPOCH] [--quiet-secs N] \
                         [--liveness-sweeps N] [--init] [--dry-run] [--no-notify] \
                         [--format text|json]\n";

/// The subcommand — the only one, and named so the argv stays open.
pub const SWEEP: &str = "sweep";

/// An attention key is `<session>\x1f<ref>`: attention is keyed PER AGENT so a
/// same-session handoff (one blocked agent clearing as another blocks) is two
/// events rather than one deduped non-event. `\x1f` is a unit separator, which
/// no session or agent name may contain.
const SEP: char = '\u{1f}';

/// Longest report field kept whole; anything longer is truncated to
/// [`SANITIZE_KEEP`] plus an ellipsis.
const SANITIZE_CAP: usize = 120;

/// What survives truncation, in characters.
const SANITIZE_KEEP: usize = 117;

/// Everything a sweep is tuned by, plus the three switches that change what it
/// does with its answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Args {
    /// "Now", in epoch seconds. A parameter so a test is a fact rather than a
    /// race.
    pub now: i64,
    /// Idle seconds before a live, non-`done` session counts as quiet.
    pub quiet_secs: i64,
    /// Silent sweeps before the liveness ping.
    pub liveness_sweeps: u64,
    /// Seed the state to the current snapshot SILENTLY and report nothing — the
    /// first-install path, so a fresh orchestrator does not announce a fleet the
    /// operator has been running all week.
    pub init: bool,
    /// Compute and print, mutate nothing.
    pub dry_run: bool,
    /// Deliver through the session's `say` helper. Off means print only, and
    /// `notified` does not advance — an unconfirmed report is not a delivered
    /// one.
    pub notify: bool,
    /// How the answer is printed.
    pub format: Format,
}

/// How a sweep prints its answer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Format {
    /// One report line each, and nothing at all when there is nothing to say.
    /// What the orchestrator runs.
    #[default]
    Text,
    /// `{"delivered":…,"report":[…]}` — always a document, so a test can tell
    /// "reported nothing" from "did not run".
    Json,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            now: 0,
            quiet_secs: DEFAULT_QUIET_SECS,
            liveness_sweeps: DEFAULT_LIVENESS_SWEEPS,
            init: false,
            dry_run: false,
            notify: true,
            format: Format::Text,
        }
    }
}

/// One running session, reduced to the four facts a sweep compares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed {
    /// The session name.
    pub name: String,
    /// Every agent of it that is asking for attention, as `(ref, reason)`.
    pub attn: Vec<(String, Reason)>,
    /// Whether the session is SESSION-LEVEL quiet: live, holding a non-`done`
    /// agent, and with no ae activity for `quiet_secs`. Not per-agent — ae does
    /// not publish per-agent activity — which is why the report phrases it as a
    /// possibility and never as a definite "waiting".
    pub quiet: bool,
    /// How many agents it holds.
    pub agents: usize,
}

/// What the world says right now, in the order [`World`] holds it.
///
/// Only RUNNING sessions: a stopped or unknown one has no attention to report
/// and its disappearance is the fleet `ended` line's business.
#[must_use]
pub fn observe(world: &World, now: i64, quiet_secs: i64) -> Vec<Observed> {
    world
        .sessions
        .iter()
        .filter(|session| session.status == Status::Running && !session.name.is_empty())
        .map(|session| {
            let idle = now - session.last_active_epoch.unwrap_or(0);
            // `alive` is three-valued and only a POSITIVE sighting counts: an
            // unknown pane is not proof of a live agent. A missing state is not
            // `done`, so it counts.
            let live_not_done = session.agents.iter().any(|agent| {
                agent.alive == Some(true) && agent.state.as_deref().unwrap_or("") != "done"
            });
            Observed {
                name: session.name.clone(),
                attn: session
                    .agents
                    .iter()
                    .filter_map(|agent| agent.reason.map(|why| (agent.reference.clone(), why)))
                    .collect(),
                quiet: live_not_done && idle > quiet_secs,
                agents: session.agents.len(),
            }
        })
        .collect()
}

/// One agent's attention, as the state file remembers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attn {
    /// The reason as reported — the literal `(cleared)` once it has gone away.
    pub reason: String,
    /// Its severity, [`Reason::rank`], or `0` for a cleared entry.
    pub rank: i64,
    /// When this agent first raised this attention.
    pub first_seen: i64,
    /// The last sweep that saw it — advances unconditionally.
    pub last_seen: i64,
    /// Whether a report naming it was DELIVERED. Advances only on success.
    pub notified: bool,
    /// Whether this entry is the all-clear rather than a live attention.
    pub cleared: bool,
}

/// A session's quiet spell, as the state file remembers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quiet {
    /// When the session first went quiet.
    pub first_seen: i64,
    /// The last sweep that saw it quiet.
    pub last_seen: i64,
    /// Whether the quiet notice was delivered.
    pub notified: bool,
}

/// The whole state file.
///
/// Every map is a [`BTreeMap`], so the document's field order and the report's
/// cleared/ended ordering are the same on every machine and every run. A
/// deduping state file whose ordering wandered would produce diffs nobody can
/// read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct State {
    /// The sweep that wrote this file.
    pub last_sweep_at: i64,
    /// Consecutive sweeps that reported nothing, reset by the liveness ping.
    pub quiet_sweeps: u64,
    /// Per-agent attention, keyed `<session>\x1f<ref>`.
    pub attention: BTreeMap<String, Attn>,
    /// Per-session quiet, keyed by session name.
    pub quiet: BTreeMap<String, Quiet>,
    /// The fleet baseline: session name to agent count.
    pub sessions: BTreeMap<String, usize>,
}

impl State {
    /// The document, as bytes to publish.
    #[must_use]
    pub fn render(&self) -> String {
        let attention = self.attention.iter().map(|(key, entry)| {
            (
                key.clone(),
                Value::obj([
                    ("cleared", Value::Bool(entry.cleared)),
                    ("first_seen", Value::Num(entry.first_seen)),
                    ("last_seen", Value::Num(entry.last_seen)),
                    ("notified", Value::Bool(entry.notified)),
                    ("rank", Value::Num(entry.rank)),
                    ("reason", Value::str(entry.reason.clone())),
                ]),
            )
        });
        let quiet = self.quiet.iter().map(|(name, entry)| {
            (
                name.clone(),
                Value::obj([
                    ("first_seen", Value::Num(entry.first_seen)),
                    ("last_seen", Value::Num(entry.last_seen)),
                    ("notified", Value::Bool(entry.notified)),
                ]),
            )
        });
        let sessions = self.sessions.iter().map(|(name, agents)| {
            (
                name.clone(),
                Value::obj([("agents", Value::Num(i64::try_from(*agents).unwrap_or(0)))]),
            )
        });
        let mut text = Value::obj([
            ("attention", Value::obj(attention)),
            ("last_sweep_at", Value::Num(self.last_sweep_at)),
            ("quiet", Value::obj(quiet)),
            (
                "quiet_sweeps",
                Value::Num(i64::try_from(self.quiet_sweeps).unwrap_or(0)),
            ),
            ("schema_version", Value::Num(SCHEMA)),
            ("sessions", Value::obj(sessions)),
        ])
        .render();
        text.push('\n');
        text
    }

    /// The document read back, or `None` for anything this reader will not
    /// trust: unparseable, not an object, or a schema it does not know.
    ///
    /// `None` means START CLEAN — never "assume the fields are absent". A
    /// corrupt file that read as an empty state would silently re-arm
    /// first-run suppression; a corrupt file that reads as NOTHING is a first
    /// run, which is a state this module already handles honestly.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let doc = json::parse(text).ok()?;
        match doc.get("schema_version") {
            Some(&Value::Num(SCHEMA)) => {}
            _ => return None,
        }
        let mut state = Self {
            last_sweep_at: num(&doc, "last_sweep_at").unwrap_or(0),
            quiet_sweeps: u64::try_from(num(&doc, "quiet_sweeps").unwrap_or(0)).unwrap_or(0),
            ..Self::default()
        };
        if let Some(Value::Obj(fields)) = doc.get("attention") {
            for (key, entry) in fields {
                state.attention.insert(
                    key.clone(),
                    Attn {
                        reason: entry.get_str("reason").unwrap_or_default().to_owned(),
                        rank: num(entry, "rank").unwrap_or(0),
                        first_seen: num(entry, "first_seen").unwrap_or(0),
                        last_seen: num(entry, "last_seen").unwrap_or(0),
                        notified: flag(entry, "notified"),
                        cleared: flag(entry, "cleared"),
                    },
                );
            }
        }
        if let Some(Value::Obj(fields)) = doc.get("quiet") {
            for (name, entry) in fields {
                state.quiet.insert(
                    name.clone(),
                    Quiet {
                        first_seen: num(entry, "first_seen").unwrap_or(0),
                        last_seen: num(entry, "last_seen").unwrap_or(0),
                        notified: flag(entry, "notified"),
                    },
                );
            }
        }
        if let Some(Value::Obj(fields)) = doc.get("sessions") {
            for (name, entry) in fields {
                let agents = usize::try_from(num(entry, "agents").unwrap_or(0)).unwrap_or(0);
                state.sessions.insert(name.clone(), agents);
            }
        }
        Some(state)
    }
}

/// A field's integer value, or `None` when absent or another shape.
fn num(value: &Value, key: &str) -> Option<i64> {
    match value.get(key) {
        Some(&Value::Num(n)) => Some(n),
        _ => None,
    }
}

/// A field's boolean value; anything that is not `true` is `false`.
fn flag(value: &Value, key: &str) -> bool {
    matches!(value.get(key), Some(&Value::Bool(true)))
}

/// A value from another agent is untrusted text: control characters become one
/// space, the result is trimmed, and anything past [`SANITIZE_CAP`] characters
/// is cut.
///
/// Report lines go to Telegram and to a pane; a newline in one of them would
/// forge a second line, and a control byte would drive the terminal.
#[must_use]
pub fn sanitize(text: &str) -> String {
    let mut clean = String::with_capacity(text.len());
    let mut in_run = false;
    for ch in text.chars() {
        if ch.is_control() {
            if !in_run {
                clean.push(' ');
                in_run = true;
            }
        } else {
            clean.push(ch);
            in_run = false;
        }
    }
    let trimmed = clean.trim();
    if trimmed.chars().count() <= SANITIZE_CAP {
        return trimmed.to_owned();
    }
    let mut cut: String = trimmed.chars().take(SANITIZE_KEEP).collect();
    cut.push_str("...");
    cut
}

/// What a delivery attempt amounted to. Three states, because "not attempted"
/// and "attempted and failed" commit differently: only the second holds the
/// fleet baseline back so a `started`/`ended` line re-fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// `--no-notify`, `--dry-run`, or nothing to say.
    NotAttempted,
    /// `say` ran and did not exit zero, or could not be run at all.
    Failed,
    /// `say` exited zero.
    Succeeded,
}

/// One sweep's answer: the lines to send, and the state that would follow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The report, one line each. Empty is the normal case.
    pub report: Vec<String>,
    /// The state to write — after [`Outcome::commit`] has been told how the
    /// delivery went.
    pub next: State,
    /// Which attention keys this report names.
    reported_attn: BTreeSet<String>,
    /// Which sessions' quiet notices this report names.
    reported_quiet: BTreeSet<String>,
    /// The fleet baseline as it stood BEFORE this sweep, kept so a failed
    /// delivery can restore it.
    prior_sessions: BTreeMap<String, usize>,
    /// Whether there was no state file at all.
    first_run: bool,
}

impl Outcome {
    /// The report as one block of text — what `say` is handed.
    #[must_use]
    pub fn text(&self) -> String {
        self.report.join("\n")
    }

    /// Fold the delivery result into the state about to be written.
    ///
    /// **The asymmetry is the contract.** `last_seen` already advanced for
    /// everything this sweep saw, whatever happened next; `notified` advances
    /// HERE, and only on [`Delivery::Succeeded`]. So a report that was produced
    /// and not delivered leaves every key it named un-notified and is produced
    /// again next sweep.
    ///
    /// A FAILED delivery also rolls the fleet baseline back, so `started` and
    /// `ended` re-fire too — they are the one class that is not tracked
    /// per-key and would otherwise be lost by the very sweep that failed to
    /// send them. Not on a first run: there the inventory is deliberately
    /// suppressed, and restoring an empty baseline would make the retry
    /// announce every existing session as newly started.
    pub fn commit(&mut self, delivery: Delivery) {
        match delivery {
            Delivery::Succeeded => {
                for key in &self.reported_attn {
                    if let Some(entry) = self.next.attention.get_mut(key) {
                        entry.notified = true;
                    }
                }
                for name in &self.reported_quiet {
                    if let Some(entry) = self.next.quiet.get_mut(name) {
                        entry.notified = true;
                    }
                }
                // A delivered all-clear has said everything it had to say.
                self.next
                    .attention
                    .retain(|_, entry| !(entry.cleared && entry.notified));
            }
            Delivery::Failed if !self.first_run => {
                self.next.sessions = std::mem::take(&mut self.prior_sessions);
            }
            Delivery::Failed | Delivery::NotAttempted => {}
        }
    }
}

/// What the three passes below accumulate.
struct Acc {
    lines: Vec<String>,
    next: State,
    reported_attn: BTreeSet<String>,
    reported_quiet: BTreeSet<String>,
    live_keys: BTreeSet<String>,
}

/// (a) ATTENTION, per agent, delivery-aware.
///
/// Four reasons to report and one to stay silent, and the silent one is the
/// only case where `notified` survives: an entry that was already delivered
/// under the same reason at the same rank.
fn attention_pass(acc: &mut Acc, session: &Observed, prior: &State, now: i64) {
    for (reference, why) in &session.attn {
        let key = format!("{}{SEP}{reference}", session.name);
        acc.live_keys.insert(key.clone());
        let reason = why.as_str();
        let rank = why.rank();
        let was = prior.attention.get(&key);
        let (report, first) = match was {
            // New agent, or one whose all-clear already went out.
            None => (true, now),
            Some(entry) if entry.cleared => (true, now),
            // A different reason, or the same word at a different severity.
            Some(entry) if entry.reason != reason || entry.rank != rank => (true, entry.first_seen),
            // Seen before and never delivered — send it again.
            Some(entry) if !entry.notified => (true, entry.first_seen),
            Some(entry) => (false, entry.first_seen),
        };
        if report {
            match was {
                Some(entry) if !entry.cleared && entry.reason != reason => acc.lines.push(format!(
                    "⚠ {} · {} now: {} (was {})",
                    sanitize(&session.name),
                    sanitize(reference),
                    sanitize(reason),
                    sanitize(&entry.reason)
                )),
                _ => acc.lines.push(format!(
                    "⚠ {} · {} needs you: {}",
                    sanitize(&session.name),
                    sanitize(reference),
                    sanitize(reason)
                )),
            }
            acc.reported_attn.insert(key.clone());
        }
        acc.next.attention.insert(
            key,
            Attn {
                reason: reason.to_owned(),
                rank,
                first_seen: first,
                last_seen: now,
                notified: !report && was.is_some(),
                cleared: false,
            },
        );
    }
}

/// (c) QUIET, session-level.
///
/// An attention signal supersedes it: a session already asking for the operator
/// does not also need "may need you". Suppressed on a first run, and SEEDED as
/// known — otherwise a failed first-attention delivery's retry would surface it.
fn quiet_pass(acc: &mut Acc, session: &Observed, prior: &State, args: &Args, first_run: bool) {
    if !session.quiet || !session.attn.is_empty() {
        return;
    }
    let was = prior.quiet.get(&session.name);
    let first = was.map_or(args.now, |entry| entry.first_seen);
    let mut notified = was.is_some_and(|entry| entry.notified);
    if first_run {
        notified = true;
    } else if !notified {
        acc.lines.push(format!(
            "{} quiet {}m+ (non-done agents) — may need you",
            sanitize(&session.name),
            args.quiet_secs / 60
        ));
        acc.reported_quiet.insert(session.name.clone());
    }
    acc.next.quiet.insert(
        session.name.clone(),
        Quiet {
            first_seen: first,
            last_seen: args.now,
            notified,
        },
    );
}

/// (a-cleared) An attention key that is gone.
///
/// A key whose SESSION ended is covered by the `ended` line and just drops; one
/// whose session still runs is an all-clear, retried until it is delivered.
fn cleared_pass(acc: &mut Acc, prior: &State, live_names: &BTreeSet<&str>, now: i64) {
    for (key, entry) in &prior.attention {
        if acc.live_keys.contains(key) {
            continue;
        }
        let (session, reference) = key.split_once(SEP).unwrap_or((key.as_str(), ""));
        if !live_names.contains(session) || (entry.cleared && entry.notified) {
            continue;
        }
        acc.lines.push(format!(
            "✓ {} · {} cleared",
            sanitize(session),
            sanitize(reference)
        ));
        acc.reported_attn.insert(key.clone());
        acc.next.attention.insert(
            key.clone(),
            Attn {
                reason: "(cleared)".to_owned(),
                rank: 0,
                first_seen: entry.first_seen,
                last_seen: now,
                notified: false,
                cleared: true,
            },
        );
    }
}

/// `--init`: the state becomes the current snapshot with everything marked
/// known, and nothing is reported at all. The first-install path, so a fresh
/// orchestrator does not announce a fleet the operator has been running all
/// week.
fn seed(acc: &mut Acc, cur: &[Observed], now: i64) {
    acc.next.attention.clear();
    acc.next.quiet.clear();
    for session in cur {
        for (reference, why) in &session.attn {
            acc.next.attention.insert(
                format!("{}{SEP}{reference}", session.name),
                Attn {
                    reason: why.as_str().to_owned(),
                    rank: why.rank(),
                    first_seen: now,
                    last_seen: now,
                    notified: true,
                    cleared: false,
                },
            );
        }
        if session.quiet && session.attn.is_empty() {
            acc.next.quiet.insert(
                session.name.clone(),
                Quiet {
                    first_seen: now,
                    last_seen: now,
                    notified: true,
                },
            );
        }
    }
    acc.lines.clear();
    acc.reported_attn.clear();
    acc.reported_quiet.clear();
    acc.next.quiet_sweeps = 0;
}

/// Diff `cur` against `prior` and compose the report.
///
/// `prior` is `None` on a FIRST RUN — by the file's absence, never by its maps
/// being empty. An initialised fleet that happens to hold nothing is not a
/// first run, and treating it as one would re-arm the inventory suppression
/// every time the fleet emptied.
#[must_use]
pub fn sweep(prior: Option<&State>, cur: &[Observed], args: &Args) -> Outcome {
    let empty = State::default();
    let first_run = prior.is_none();
    let prior = prior.unwrap_or(&empty);
    let now = args.now;
    let live_names: BTreeSet<&str> = cur.iter().map(|session| session.name.as_str()).collect();

    let mut acc = Acc {
        lines: Vec::new(),
        next: State {
            last_sweep_at: now,
            ..State::default()
        },
        reported_attn: BTreeSet::new(),
        reported_quiet: BTreeSet::new(),
        live_keys: BTreeSet::new(),
    };

    for session in cur {
        acc.next
            .sessions
            .insert(session.name.clone(), session.agents);
        attention_pass(&mut acc, session, prior, now);
        quiet_pass(&mut acc, session, prior, args, first_run);
        // (b) FLEET — started. Conservative on purpose: no agent-count churn,
        // no active/idle narration. Suppressed on a first run.
        if !prior.sessions.contains_key(&session.name) && !first_run {
            acc.lines.push(format!(
                "▶ {} started ({} agents)",
                sanitize(&session.name),
                session.agents
            ));
        }
    }

    cleared_pass(&mut acc, prior, &live_names, now);

    // (b) FLEET — ended.
    for name in prior.sessions.keys() {
        if !live_names.contains(name.as_str()) && !first_run {
            acc.lines.push(format!("■ {} ended", sanitize(name)));
        }
    }

    // The liveness counter, and only when the sweep is otherwise silent: a ping
    // riding along with a real report would be noise on top of signal.
    if acc.lines.is_empty() {
        acc.next.quiet_sweeps = prior.quiet_sweeps.saturating_add(1);
        if acc.next.quiet_sweeps >= args.liveness_sweeps {
            acc.lines.push(format!(
                "🛰️ still watching {} sessions · all healthy",
                live_names.len()
            ));
            acc.next.quiet_sweeps = 0;
        }
    }

    if args.init {
        seed(&mut acc, cur, now);
    }

    Outcome {
        report: acc.lines,
        next: acc.next,
        reported_attn: acc.reported_attn,
        reported_quiet: acc.reported_quiet,
        prior_sessions: prior.sessions.clone(),
        first_run,
    }
}

/// The one program a sweep may run: `<session-dir>/say`, and one argument.
///
/// A newtype with ONE constructor, for the reason [`crate::watchdog_daemon`]'s
/// send helper has one — the program a daemon EXECUTES must never come from
/// meta, config, an environment variable or pane content, and the Python this
/// replaces took it as `--notify-cmd <any path>` from a hand-edited charter.
/// Joining a literal onto the session directory makes the path unforgeable by
/// construction, so "every reviewer must check every call site" becomes "there
/// is one constructor".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    helper: PathBuf,
    message: String,
}

impl Notice {
    /// The `say` helper of `dir`, carrying `message`.
    #[must_use]
    pub fn for_session(dir: &Path, message: &str) -> Self {
        Self {
            helper: dir.join(SAY_HELPER),
            message: message.to_owned(),
        }
    }

    /// The program to run. Reading is harmless; CONSTRUCTION is what is sealed.
    pub(crate) fn helper(&self) -> &Path {
        &self.helper
    }

    /// Its single argument.
    pub(crate) fn args(&self) -> [String; 1] {
        [self.message.clone()]
    }
}

/// Run one sweep for the session at `dir` and report what it found.
///
/// # Errors
///
/// Whatever `out`/`err` return — the entry writes nothing else.
pub fn run(
    dir: &Path,
    world: &World,
    args: &Args,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> std::io::Result<u8> {
    let cur = observe(world, args.now, args.quiet_secs);
    let state_path = dir.join(STATE_NAME);

    // The lock covers read-decide-write, exactly as the Python's `flock` did:
    // two sweeps racing would each read the same prior state and each report
    // the same change. Dropping the handle releases it.
    let lock = crate::state::acquire(
        &dir.join(format!("{STATE_NAME}.lock")),
        crate::state::LOCK_WAIT,
    );
    let _lock = match lock {
        Ok(handle) => handle,
        Err(why) => {
            writeln!(
                err,
                "ae: monitor: could not lock {}: {why}",
                state_path.display()
            )?;
            return Ok(1);
        }
    };

    let prior = read_state(&state_path);
    let mut outcome = sweep(prior.as_ref(), &cur, args);
    let text = outcome.text();

    let delivery = if text.is_empty() || !args.notify || args.dry_run {
        Delivery::NotAttempted
    } else if crate::transport::run_say(&Notice::for_session(dir, &text)) {
        Delivery::Succeeded
    } else {
        writeln!(err, "ae: monitor: say failed — retrying next sweep")?;
        Delivery::Failed
    };
    outcome.commit(delivery);

    if !args.dry_run
        && let Err(why) = publish(&state_path, &outcome.next.render())
    {
        writeln!(
            err,
            "ae: monitor: could not write {}: {why}",
            state_path.display()
        )?;
        return Ok(1);
    }

    if args.format == Format::Json {
        let report = Value::Arr(outcome.report.iter().map(Value::str).collect());
        let doc = Value::obj([
            ("delivered", Value::Bool(delivery == Delivery::Succeeded)),
            ("report", report),
        ]);
        writeln!(out, "{}", doc.render())?;
    } else if !text.is_empty() {
        writeln!(out, "{text}")?;
    }
    Ok(0)
}

/// The state file, or `None` for absent, empty, unreadable or untrusted — every
/// one of which is a first run.
fn read_state(path: &Path) -> Option<State> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the orchestrator's own sweep state, the file this module owns — see clippy.toml"
    )]
    let text = std::fs::read_to_string(path);
    State::parse(&text.ok()?)
}

/// Publish `content` at `path`: a temp beside it, then a rename, then a
/// directory sync — so the first observable version is a complete one and a
/// crashed sweep cannot leave a half-written state file behind.
///
/// Mode `0600`: the file names every session on the machine and what each is
/// waiting for.
fn publish(path: &Path, content: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let temp = dir.join(format!(".{STATE_NAME}.{}", std::process::id()));
    let staged = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, path)
    })();
    if staged.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    staged?;
    // Visible now. Publish the directory entry too, so the sweep the watchdog
    // reads a heartbeat from cannot be lost to a crash between the two.
    std::fs::OpenOptions::new()
        .read(true)
        .open(dir)
        .and_then(|directory| directory.sync_all())
}

#[cfg(test)]
mod tests {
    use super::{
        Args, Attn, Delivery, Format, Observed, Quiet, SEP, State, observe, sanitize, sweep,
    };
    use crate::attention::Reason;
    use crate::digest::{AgentEntry, SessionEntry, Status};
    use crate::listing::World;
    use crate::time::Timestamp;

    const NOW: i64 = 1_788_000_000;

    fn args() -> Args {
        Args {
            now: NOW,
            ..Args::default()
        }
    }

    /// A prior state that already knows `sessions`.
    ///
    /// NOT `State::default()`: that is a state file which has seen nothing, so
    /// every live session is genuinely new to it and earns a `started` line.
    /// First-run suppression keys on the file's ABSENCE, never on its maps being
    /// empty — these two priors are different facts and the tests keep them
    /// apart.
    fn known(sessions: &[(&str, usize)]) -> State {
        State {
            sessions: sessions
                .iter()
                .map(|(name, agents)| ((*name).to_owned(), *agents))
                .collect(),
            ..State::default()
        }
    }

    fn agent(reference: &str, state: Option<&str>, why: Option<Reason>) -> AgentEntry {
        AgentEntry {
            reference: reference.to_owned(),
            alive: Some(true),
            state: state.map(ToOwned::to_owned),
            reason: why,
            ..AgentEntry::default()
        }
    }

    fn session(name: &str, idle: i64, agents: Vec<AgentEntry>) -> SessionEntry {
        // Built by the constructor and then adjusted: two of `SessionEntry`'s
        // fields are private, so `..` update syntax is not available here.
        let mut entry = SessionEntry::new(name, Status::Running);
        entry.last_active_epoch = Some(NOW - idle);
        entry.agents = agents;
        entry
    }

    fn world(sessions: Vec<SessionEntry>) -> World {
        World::new(Timestamp::from_epoch(NOW), sessions)
    }

    /// One sweep of `sessions` against `prior`, delivered successfully — the
    /// orchestrator's real path, so a follow-up sweep sees notified entries.
    fn run(
        prior: Option<&State>,
        sessions: Vec<SessionEntry>,
        args: &Args,
    ) -> (Vec<String>, State) {
        let seen = observe(&world(sessions), args.now, args.quiet_secs);
        let mut outcome = sweep(prior, &seen, args);
        outcome.commit(Delivery::Succeeded);
        (outcome.report, outcome.next)
    }

    #[test]
    fn only_running_sessions_reach_the_sweep() {
        // A stopped session has no attention to report and an unknown one
        // established nothing; both would otherwise contribute a `started` line
        // and a fleet baseline row for a session ae cannot see.
        let mut stopped = session("gone", 0, vec![agent("lead", None, Some(Reason::Blocked))]);
        stopped.status = Status::Stopped;
        let mut unknown = session("maybe", 0, vec![agent("lead", None, Some(Reason::Dead))]);
        unknown.status = Status::Unknown;
        let live = session("here", 0, vec![agent("lead", None, None)]);

        let seen = observe(&world(vec![stopped, unknown, live]), NOW, 1200);

        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].name, "here");
    }

    #[test]
    fn a_first_run_reports_attention_but_never_the_fleet_it_inherited() {
        // A fresh install must not announce every session the operator has been
        // running all week — but it must not swallow what needs them either.
        let (report, next) = run(
            None,
            vec![
                session("alpha", 0, vec![agent("lead", None, Some(Reason::Blocked))]),
                session("beta", 0, vec![agent("lead", None, None)]),
            ],
            &args(),
        );

        assert_eq!(report, vec!["⚠ alpha · lead needs you: blocked".to_owned()]);
        assert_eq!(next.sessions.len(), 2, "the baseline is still seeded");

        // Second sweep, same fleet: silent. The inventory was taken, not
        // reported, so nothing arrives late.
        let (report, _) = run(
            Some(&next),
            vec![
                session("alpha", 0, vec![agent("lead", None, Some(Reason::Blocked))]),
                session("beta", 0, vec![agent("lead", None, None)]),
            ],
            &args(),
        );
        assert!(report.is_empty(), "{report:?}");
    }

    #[test]
    fn a_session_that_starts_or_ends_is_narrated_once_each() {
        let (_, first) = run(
            Some(&State::default()),
            vec![session("alpha", 0, vec![agent("lead", None, None)])],
            &args(),
        );
        let (report, second) = run(
            Some(&first),
            vec![
                session("alpha", 0, vec![agent("lead", None, None)]),
                session(
                    "beta",
                    0,
                    vec![agent("a", None, None), agent("b", None, None)],
                ),
            ],
            &args(),
        );
        assert_eq!(report, vec!["▶ beta started (2 agents)".to_owned()]);

        let (report, _) = run(
            Some(&second),
            vec![session(
                "beta",
                0,
                vec![agent("a", None, None), agent("b", None, None)],
            )],
            &args(),
        );
        assert_eq!(report, vec!["■ alpha ended".to_owned()]);
    }

    #[test]
    fn a_quiet_session_is_flagged_once_and_never_while_it_is_already_asking() {
        // "Quiet" is SESSION-level and phrased as a possibility: ae publishes no
        // per-agent activity, so the honest claim is "may need you".
        let quiet = || session("alpha", 3600, vec![agent("lead", Some("working"), None)]);
        let (_, seeded) = run(Some(&known(&[("alpha", 1)])), vec![quiet()], &args());
        assert_eq!(
            seeded.quiet.get("alpha").map(|entry| entry.notified),
            Some(true),
            "the first sweep past a real prior reports and delivers it"
        );

        let (report, _) = run(Some(&seeded), vec![quiet()], &args());
        assert!(report.is_empty(), "still quiet is not news: {report:?}");

        // An attention signal SUPERSEDES quiet: a session already asking for the
        // operator does not also get "may need you".
        let asking = session(
            "alpha",
            3600,
            vec![agent("lead", Some("blocked"), Some(Reason::Blocked))],
        );
        let (report, next) = run(Some(&known(&[("alpha", 1)])), vec![asking], &args());
        assert_eq!(report, vec!["⚠ alpha · lead needs you: blocked".to_owned()]);
        assert!(next.quiet.is_empty(), "no quiet entry beside an attention");
    }

    #[test]
    fn the_quiet_line_states_the_threshold_it_was_measured_against() {
        let mut args = args();
        args.quiet_secs = 600;
        let (report, _) = run(
            Some(&known(&[("alpha", 1)])),
            vec![session(
                "alpha",
                1200,
                vec![agent("lead", Some("working"), None)],
            )],
            &args,
        );
        assert_eq!(
            report,
            vec!["alpha quiet 10m+ (non-done agents) — may need you".to_owned()],
            "the minutes come from the knob, not from a hardcoded 20"
        );
    }

    #[test]
    fn a_done_or_unproven_agent_never_makes_a_session_quiet() {
        // `done` is a finished agent, and an `alive` ae could not establish is
        // not a live one. Either would turn an idle, finished session into a
        // recurring "may need you".
        for agents in [
            vec![agent("lead", Some("done"), None)],
            vec![AgentEntry {
                alive: None,
                ..agent("lead", Some("working"), None)
            }],
        ] {
            let (report, _) = run(
                Some(&known(&[("alpha", 1)])),
                vec![session("alpha", 3600, agents)],
                &args(),
            );
            assert!(report.is_empty(), "{report:?}");
        }
    }

    #[test]
    fn a_silent_stretch_eventually_pings_and_then_starts_counting_again() {
        let mut args = args();
        args.liveness_sweeps = 3;
        let quiet = || vec![session("alpha", 0, vec![agent("lead", None, None)])];
        let mut state = known(&[("alpha", 1)]);
        for expected in [1, 2] {
            let (report, next) = run(Some(&state), quiet(), &args);
            assert!(report.is_empty(), "{report:?}");
            assert_eq!(next.quiet_sweeps, expected);
            state = next;
        }
        let (report, next) = run(Some(&state), quiet(), &args);
        assert_eq!(
            report,
            vec!["🛰️ still watching 1 sessions · all healthy".to_owned()]
        );
        assert_eq!(next.quiet_sweeps, 0, "the counter restarts after the ping");
    }

    #[test]
    fn an_undelivered_report_holds_back_notified_and_the_fleet_baseline_alike() {
        // The asymmetry the whole dedup rests on, at the unit: `last_seen`
        // advances regardless, `notified` does not, and a `started` line that
        // failed to send is not lost by the sweep that produced it.
        let (_, first) = run(
            Some(&State::default()),
            vec![session("alpha", 0, vec![agent("lead", None, None)])],
            &args(),
        );
        let now = vec![
            session("alpha", 0, vec![agent("lead", None, None)]),
            session("beta", 0, vec![agent("lead", None, Some(Reason::Blocked))]),
        ];
        let seen = observe(&world(now.clone()), NOW, 1200);
        let mut outcome = sweep(Some(&first), &seen, &args());
        assert_eq!(outcome.report.len(), 2, "{:?}", outcome.report);
        outcome.commit(Delivery::Failed);

        let key = format!("beta{SEP}lead");
        let held = outcome.next.attention.get(&key).expect("the attention");
        assert_eq!(held.last_seen, NOW, "last_seen advances whatever happened");
        assert!(
            !held.notified,
            "an unconfirmed report is not a delivered one"
        );
        assert!(
            !outcome.next.sessions.contains_key("beta"),
            "the fleet baseline rolled back, so `started` re-fires"
        );

        let (report, _) = run(Some(&outcome.next), now, &args());
        assert_eq!(report.len(), 2, "both lines come back: {report:?}");
    }

    #[test]
    fn attention_is_keyed_per_agent_so_a_handoff_is_two_events() {
        // One blocked agent clearing as another blocks is a CHANGE the operator
        // needs. Keyed per session it would dedup to nothing at all.
        let (_, first) = run(
            Some(&State::default()),
            vec![session(
                "alpha",
                0,
                vec![
                    agent("one", None, Some(Reason::Blocked)),
                    agent("two", None, None),
                ],
            )],
            &args(),
        );
        let (report, _) = run(
            Some(&first),
            vec![session(
                "alpha",
                0,
                vec![
                    agent("one", None, None),
                    agent("two", None, Some(Reason::Blocked)),
                ],
            )],
            &args(),
        );
        assert_eq!(
            report,
            vec![
                "⚠ alpha · two needs you: blocked".to_owned(),
                "✓ alpha · one cleared".to_owned(),
            ]
        );
    }

    #[test]
    fn an_attention_whose_session_ended_drops_without_a_second_line() {
        // The `ended` line already says it. A per-agent all-clear beside it
        // would be the same news twice.
        let (_, first) = run(
            Some(&State::default()),
            vec![session(
                "alpha",
                0,
                vec![agent("lead", None, Some(Reason::Dead))],
            )],
            &args(),
        );
        let (report, next) = run(Some(&first), vec![], &args());
        assert_eq!(report, vec!["■ alpha ended".to_owned()]);
        assert!(next.attention.is_empty(), "the key drops with its session");
    }

    #[test]
    fn every_reason_the_core_knows_reaches_a_report() {
        // The Python this replaces carried its own rank table and silently
        // IGNORED any reason ae later added — which is what happened to
        // `unanswered`. One definition means a new reason is reported the day
        // it exists.
        for why in Reason::BY_SEVERITY {
            let (report, _) = run(
                Some(&known(&[("alpha", 1)])),
                vec![session("alpha", 0, vec![agent("lead", None, Some(why))])],
                &args(),
            );
            assert_eq!(
                report,
                vec![format!("⚠ alpha · lead needs you: {}", why.as_str())],
                "{why:?} must not be silently dropped"
            );
        }
    }

    #[test]
    fn init_seeds_the_snapshot_as_known_and_says_nothing() {
        let mut args = args();
        args.init = true;
        let (report, next) = run(
            None,
            vec![session(
                "alpha",
                3600,
                vec![agent("lead", Some("blocked"), Some(Reason::Blocked))],
            )],
            &args,
        );
        assert!(report.is_empty(), "{report:?}");
        let seeded = next
            .attention
            .get(&format!("alpha{SEP}lead"))
            .expect("the seeded key");
        assert!(
            seeded.notified,
            "seeded means known, so it never re-reports"
        );
    }

    #[test]
    fn the_document_round_trips_and_an_untrusted_one_is_no_document_at_all() {
        let state = State {
            last_sweep_at: NOW,
            quiet_sweeps: 4,
            attention: [(
                format!("alpha{SEP}lead"),
                Attn {
                    reason: "blocked".to_owned(),
                    rank: Reason::Blocked.rank(),
                    first_seen: NOW - 60,
                    last_seen: NOW,
                    notified: true,
                    cleared: false,
                },
            )]
            .into_iter()
            .collect(),
            quiet: [(
                "beta".to_owned(),
                Quiet {
                    first_seen: NOW - 120,
                    last_seen: NOW,
                    notified: false,
                },
            )]
            .into_iter()
            .collect(),
            sessions: [("alpha".to_owned(), 2)].into_iter().collect(),
        };
        let text = state.render();
        assert_eq!(State::parse(&text), Some(state), "parse(render(x)) == x");

        // Every untrusted shape is the SAME answer, and the answer is "no
        // document" — a corrupt file that read as an empty state would re-arm
        // the first-run suppression it must not.
        for hostile in [
            "",
            "{",
            "[]",
            r#""a string""#,
            r#"{"schema_version":2}"#,
            r#"{"schema_version":"1"}"#,
            r#"{"attention":{}}"#,
        ] {
            assert_eq!(State::parse(hostile), None, "{hostile:?}");
        }
    }

    #[test]
    fn a_hostile_name_cannot_forge_a_line_or_drive_a_terminal() {
        // Report lines go to Telegram and to a pane, and a session or agent name
        // is another agent's text. A newline in one would forge a second report
        // line; an escape would drive the terminal it lands in.
        assert_eq!(sanitize("a\nb"), "a b");
        assert_eq!(sanitize("a\r\n\tb"), "a b", "a RUN is one space");
        assert_eq!(sanitize("\u{1b}[31mred"), "[31mred");
        assert_eq!(sanitize("  padded  "), "padded");

        let long = "x".repeat(200);
        let cut = sanitize(&long);
        assert_eq!(cut.chars().count(), 120);
        assert!(cut.ends_with("..."));
        assert_eq!(
            sanitize(&"y".repeat(120)).chars().count(),
            120,
            "the cap is inclusive"
        );

        // Multibyte truncation cuts on a CHARACTER, never mid-codepoint.
        let wide = "é".repeat(200);
        assert_eq!(sanitize(&wide).chars().count(), 120);
    }

    #[test]
    fn the_json_format_is_a_document_even_when_there_is_nothing_to_say() {
        // A test — and an operator — must be able to tell "reported nothing"
        // from "did not run". Empty stdout cannot carry that distinction.
        assert_eq!(Args::default().format, Format::Text);
        let observed = Observed {
            name: "alpha".to_owned(),
            attn: Vec::new(),
            quiet: false,
            agents: 1,
        };
        let outcome = sweep(Some(&known(&[("alpha", 1)])), &[observed], &args());
        assert!(outcome.report.is_empty());
        assert_eq!(outcome.text(), "");
    }
}
