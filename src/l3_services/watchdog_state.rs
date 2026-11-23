//! The orchestrator's watchdog checkpoint: the one document the manual
//! `_monitor sweep` and the watchdog both keep, and the locked operations the
//! watchdog runs on it. Its representation lives here, below both of its
//! writers, so the overview gate and the desk-stale tick reach it without a
//! reference into the UI layer; [`crate::monitor`] re-exports what it shows.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use crate::events::Cursor;
use crate::json::{self, Value};

/// The state document's version.
pub const SCHEMA: i64 = 1;

/// The state file's name, under the session directory: the watchdog's
/// heartbeat, so there is exactly one spelling of it in the crate.
pub const STATE_NAME: &str = "meta-agent-state.json";

/// One agent's attention, as the state file remembers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attn {
    /// The reason as reported — the literal `(cleared)` once it has gone away.
    pub reason: String,
    /// Its severity, [`crate::attention::Reason::rank`], or `0` for a cleared entry.
    pub rank: i64,
    /// When this agent first raised this attention.
    pub first_seen: i64,
    /// The last sweep that saw it — advances unconditionally.
    pub last_seen: i64,
    /// Whether a report naming it was DELIVERED.
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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct State {
    /// The sweep that wrote this file.
    pub last_sweep_at: i64,
    /// Consecutive sweeps that reported nothing, reset by the liveness ping.
    pub quiet_sweeps: u64,
    /// Whether the manual `_monitor sweep` baseline has been seeded. A
    /// watchdog-only heartbeat leaves this false.
    pub monitor_initialized: bool,
    /// Per-agent attention, keyed `<session>\x1f<ref>`.
    pub attention: BTreeMap<String, Attn>,
    /// Per-session quiet, keyed by session name.
    pub quiet: BTreeMap<String, Quiet>,
    /// The fleet baseline: session name to agent count.
    pub sessions: BTreeMap<String, usize>,
    /// Digest of the last fleet overview the watchdog delivered to the seat.
    pub overview_hash: Option<String>,
    /// Epoch second of the oldest delivery not yet acknowledged. Repeated
    /// deliveries retain it so the acknowledgement deadline cannot slide.
    pub overview_delivered_at: Option<i64>,
    /// Epoch second of the latest successful delivery; minimum spacing
    /// survives watchdog restarts independently of the acknowledgement age.
    pub overview_last_delivered_at: Option<i64>,
    /// Epoch second of the latest claimed desk-stale tick: the desk's half of
    /// the spacing it shares with the overview. It moves no overview field.
    pub desk_tick_last_at: Option<i64>,
    /// Per session (its canonical uuid, else `path<US><durable dir>`), the journal position the
    /// latest claimed desk-stale tick consumed.
    pub desk_tick_cursor: BTreeMap<String, Cursor>,
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
        let mut fields = vec![
            ("attention", Value::obj(attention)),
            ("last_sweep_at", Value::Num(self.last_sweep_at)),
            ("monitor_initialized", Value::Bool(self.monitor_initialized)),
            ("quiet", Value::obj(quiet)),
            (
                "quiet_sweeps",
                Value::Num(i64::try_from(self.quiet_sweeps).unwrap_or(0)),
            ),
        ];
        if let Some(at) = self.overview_delivered_at {
            fields.push(("overview_delivered_at", Value::Num(at)));
        }
        if let Some(at) = self.overview_last_delivered_at {
            fields.push(("overview_last_delivered_at", Value::Num(at)));
        }
        if let Some(hash) = &self.overview_hash {
            fields.push(("overview_hash", Value::str(hash)));
        }
        if !self.desk_tick_cursor.is_empty() {
            let cursors = self.desk_tick_cursor.iter().map(|(key, at)| {
                (
                    key.clone(),
                    Value::obj([
                        (
                            "generation",
                            Value::Num(i64::try_from(at.generation).unwrap_or(i64::MAX)),
                        ),
                        (
                            "offset",
                            Value::Num(i64::try_from(at.offset).unwrap_or(i64::MAX)),
                        ),
                    ]),
                )
            });
            fields.push(("desk_tick_cursor", Value::obj(cursors)));
        }
        if let Some(at) = self.desk_tick_last_at {
            fields.push(("desk_tick_last_at", Value::Num(at)));
        }
        fields.extend([
            ("schema_version", Value::Num(SCHEMA)),
            ("sessions", Value::obj(sessions)),
        ]);
        let mut text = Value::obj(fields).render();
        text.push('\n');
        text
    }

    /// The document read back, or `None` for anything this reader will not
    /// trust: unparseable, not an object, or a schema it does not know.
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
            // A schema-1 document written before overview heartbeats existed
            // came from `_monitor sweep`, so absence means initialized.
            monitor_initialized: doc
                .get("monitor_initialized")
                .is_none_or(|value| matches!(value, Value::Bool(true))),
            overview_hash: doc.get_str("overview_hash").map(ToOwned::to_owned),
            overview_delivered_at: num(&doc, "overview_delivered_at"),
            overview_last_delivered_at: num(&doc, "overview_last_delivered_at"),
            desk_tick_last_at: num(&doc, "desk_tick_last_at"),
            ..Self::default()
        };
        if let Some(Value::Obj(fields)) = doc.get("desk_tick_cursor") {
            for (key, entry) in fields {
                let position = |name| num(entry, name).and_then(|n| u64::try_from(n).ok());
                if let (Some(generation), Some(offset)) =
                    (position("generation"), position("offset"))
                {
                    state
                        .desk_tick_cursor
                        .insert(key.clone(), Cursor { generation, offset });
                }
            }
        }
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

/// The document at `path`, or `None` for absent, empty, unreadable, not a
/// regular file or untrusted: every one of which is a first run.
fn read(path: &Path) -> Option<State> {
    match crate::store::read_source(path) {
        crate::store::SourceRead::Ready(bytes) => State::parse(std::str::from_utf8(&bytes).ok()?),
        _ => None,
    }
}

/// The persisted half of the watchdog's change gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OverviewCheckpoint {
    /// Digest of the last overview that landed in the seat.
    pub hash: Option<String>,
    /// Epoch second of the oldest delivery awaiting acknowledgement.
    pub outstanding_since: Option<i64>,
    /// Epoch second of the latest successful delivery, for minimum spacing.
    pub last_delivered_at: Option<i64>,
}

/// The desk-stale tick's own half of the checkpoint.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeskTick {
    /// Epoch second of the latest claim.
    pub last_at: Option<i64>,
    /// Per session key, the journal position that claim consumed.
    pub cursor: BTreeMap<String, Cursor>,
}

/// Mark one successful watchdog render as a heartbeat and return the last
/// delivered overview checkpoint. No overview hash advances here: observing a
/// changed fleet is not evidence that its nudge landed.
pub(crate) fn overview_heartbeat(dir: &Path, now: i64) -> std::io::Result<OverviewCheckpoint> {
    update_overview_state(dir, now, None)
}

/// Persist the overview whose nudge just landed.
pub(crate) fn record_overview_delivery(
    dir: &Path,
    hash: &str,
    outstanding_since: i64,
    delivered_at: i64,
) -> std::io::Result<()> {
    update_overview_state(
        dir,
        delivered_at,
        Some((hash, outstanding_since, delivered_at)),
    )
    .map(|_| ())
}

/// One locked read-decide-write over the heartbeat document. This keeps the
/// watchdog and `_monitor` from losing each other's optional fields.
fn update_overview_state(
    dir: &Path,
    now: i64,
    delivery: Option<(&str, i64, i64)>,
) -> std::io::Result<OverviewCheckpoint> {
    let state_path = dir.join(STATE_NAME);
    let _lock = crate::store::lock(
        &dir.join(format!("{STATE_NAME}.lock")),
        crate::store::LOCK_WAIT,
    )?;
    let mut state = read(&state_path).unwrap_or_default();
    let checkpoint = OverviewCheckpoint {
        hash: state.overview_hash.clone(),
        outstanding_since: state.overview_delivered_at,
        last_delivered_at: state.overview_last_delivered_at,
    };
    state.last_sweep_at = now;
    if let Some((hash, outstanding_since, delivered_at)) = delivery {
        state.overview_hash = Some(hash.to_owned());
        state.overview_delivered_at = Some(outstanding_since);
        state.overview_last_delivered_at = Some(delivered_at);
    }
    publish(&state_path, &state.render())?;
    Ok(checkpoint)
}

/// The desk tick's half of the checkpoint as stored now; empty when the
/// document is absent or untrusted. A published document is whole, so a read
/// without the lock sees one writer's complete claim.
pub(crate) fn desk_tick(dir: &Path) -> DeskTick {
    read(&dir.join(STATE_NAME)).map_or_else(DeskTick::default, |state| DeskTick {
        last_at: state.desk_tick_last_at,
        cursor: state.desk_tick_cursor,
    })
}

/// The two stamps the shared delivery spacing reads: the latest overview
/// delivery and the latest desk-tick claim.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stamps {
    /// Epoch second of the latest overview delivery.
    pub overview_last: Option<i64>,
    /// Epoch second of the latest desk-tick claim.
    pub desk_last: Option<i64>,
}

/// The spacing stamps as stored now; empty when the document is absent or
/// untrusted, which differs from any stamp an observation read.
pub(crate) fn stamps(dir: &Path) -> Stamps {
    read(&dir.join(STATE_NAME)).map_or_else(Stamps::default, |state| Stamps {
        overview_last: state.overview_last_delivered_at,
        desk_last: state.desk_tick_last_at,
    })
}

/// Claim one desk-stale tick before its submit: store `claim` only while the
/// stored claim is still `expected` and the latest overview delivery is still
/// `overview_last`, both as the cycle's decision read them. A tick or an
/// overview another writer delivered since makes this one stand down, so the
/// shared spacing holds under the lock. Returns whether the claim is stored.
/// The overview fields stay as they are.
pub(crate) fn claim_desk_tick(
    dir: &Path,
    expected: &DeskTick,
    overview_last: Option<i64>,
    claim: &DeskTick,
) -> std::io::Result<bool> {
    swap_desk_tick(dir, expected, Spacing::Unmoved(overview_last), claim)
}

/// Put `previous` back after a proven pre-paste refusal, only while the stored
/// claim is still exactly `claim`: a newer claim is never overwritten.
pub(crate) fn restore_desk_tick(
    dir: &Path,
    claim: &DeskTick,
    previous: &DeskTick,
) -> std::io::Result<bool> {
    swap_desk_tick(dir, claim, Spacing::Any, previous)
}

/// What a desk-tick swap requires of the latest overview delivery.
#[derive(Clone, Copy)]
enum Spacing {
    /// It must still be this one.
    Unmoved(Option<i64>),
    /// Any: the swap is about the desk claim alone.
    Any,
}

/// One locked compare-and-swap of the desk tick's half of the checkpoint.
fn swap_desk_tick(
    dir: &Path,
    expected: &DeskTick,
    spacing: Spacing,
    desk: &DeskTick,
) -> std::io::Result<bool> {
    let state_path = dir.join(STATE_NAME);
    let _lock = crate::store::lock(
        &dir.join(format!("{STATE_NAME}.lock")),
        crate::store::LOCK_WAIT,
    )?;
    let mut state = read(&state_path).unwrap_or_default();
    let moved = match spacing {
        Spacing::Unmoved(last) => state.overview_last_delivered_at != last,
        Spacing::Any => false,
    };
    if moved
        || state.desk_tick_last_at != expected.last_at
        || state.desk_tick_cursor != expected.cursor
    {
        return Ok(false);
    }
    state.desk_tick_last_at = desk.last_at;
    state.desk_tick_cursor.clone_from(&desk.cursor);
    publish(&state_path, &state.render())?;
    Ok(true)
}

/// Publish `content` at `path`: a temp beside it, then a rename, then a
/// directory sync — so the first observable version is a complete one and a
/// crashed sweep cannot leave a half-written state file behind.
pub(crate) fn publish(path: &Path, content: &str) -> std::io::Result<()> {
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
    // Visible now.
    std::fs::OpenOptions::new()
        .read(true)
        .open(dir)
        .and_then(|directory| directory.sync_all())
}

#[cfg(test)]
#[path = "watchdog_state/watchdog_state_tests.rs"]
mod tests;
