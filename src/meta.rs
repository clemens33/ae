//! The session `meta` file — SC-405a, SC-405b, SC-405c, and nothing else.
//!
//! Wired only after the seats ratified those three rows. Before that this
//! module did not exist: the digest's `mode`/`origin`/`work_dir`/`goal` and its
//! `agents[]` roster sat unread rather than being guessed from the bash writer.
//!
//! * **SC-405a** — `key=value`, split on the FIRST equals; values are single-line.
//! * **SC-405b** — `mode`, `origin`, `work_dir`, `goal` are meta keys.
//! * **SC-405c** — `agent.<slot>` carries `alias:name:provider-session-id`
//!   (SC-1207b, session id optional per the roster authority) and
//!   `agent_bin.<slot>` the recorded binary.
//!
//! * **SC-405d** — every OTHER key is tolerated silently and never degrades.
//!   Unknown keys are the normal state of a real meta, so degrading on them
//!   would make the flag constant-true. They are still recorded as an
//!   [`Anomaly`], because seeing them costs nothing and a future tool may want
//!   them; SC-405h was REJECTED, so no list of them lives here to go stale.
//! * **SC-405e** — a malformed line, a malformed roster value or a DUPLICATE
//!   key is different: the reader could not take a value the writer meant to
//!   give. Those degrade (SC-509b), and a duplicated key INVALIDATES its field
//!   rather than publishing an occurrence, because precedence is still
//!   unclassified and picking one would be fabricating the answer.
//!
//! Two fields that look like meta keys and are not: `goal_set_epoch` is derived
//! from the latest goal EVENT (SC-405f) and `branch` is a live tmux/git fact
//! (SC-405g). Neither is read here.

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The file this module reads, inside a session directory.
pub const FILE: &str = "meta";

/// The roster key prefixes (SC-405c). The four context keys of SC-405b are
/// matched literally where they are absorbed, next to their fields.
const ROSTER_PREFIX: &str = "agent.";
/// SC-405l's two selector keys, matched literally where they are absorbed.
const SERVER_KEY: &str = "tmux_server";
const SERVER_KIND_KEY: &str = "tmux_server_kind";
const ROSTER_BIN_PREFIX: &str = "agent_bin.";
/// Identity schema v2 (alias-free): the seat's NAME, its execution PROFILE and
/// the harness's own conversation id live under three keys instead of one
/// `alias:name:sid` value. New KEYS, deliberately: a v1 parser meets them as
/// SC-405d-unknown and answers an EMPTY roster (fail closed), where a `name:sid`
/// value under the old key would have been misparsed as alias=name, name=sid.
const SEAT_PREFIX: &str = "seat.";
const PROFILE_PREFIX: &str = "profile.";
const HARNESS_SESSION_PREFIX: &str = "harness_session.";
/// `schema=<n>` — the identity schema the writer used. Absent means v1.
const SCHEMA_KEY: &str = "schema";

/// Which identity schema wrote a roster entry. Kept per entry because the two
/// spell the DISPLAY ref differently and pre-SC-511a ledgers pair on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RosterSchema {
    /// `agent.<slot>=alias:name[:sid]` — the alias is the identity's first half.
    V1,
    /// `seat.<slot>=name` + `profile.<slot>` + `harness_session.<slot>` — the
    /// name IS the identity; the profile is metadata.
    V2,
}

/// One agent, as the roster records it (SC-405c + SC-1207b, and identity v2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterEntry {
    /// `main` / `worker.<n>` / `spawned.<n>` — the key's suffix.
    pub slot: String,
    /// The agent's NAME — its identity under v2; the display-name half under v1.
    pub name: String,
    /// The execution profile (v2 `profile.<slot>`; the alias half under v1).
    /// Metadata, never part of the identity. `None` when a v2 seat has no
    /// profile row.
    pub profile: Option<String>,
    /// The harness's own conversation id (v2 `harness_session.<slot>`; the
    /// `:sid` suffix under v1), where the roster carries one.
    pub harness_session: Option<String>,
    /// `agent_bin.<slot>` — the recorded binary, where the meta carries one.
    pub binary: Option<String>,
    /// Which schema wrote this entry.
    pub schema: RosterSchema,
}

impl RosterEntry {
    /// The DISPLAY ref this agent is known by in the ledger and on panes.
    ///
    /// v2: the bare name — the identity. v1: `alias:name`, kept EXACTLY so that
    /// pre-SC-511a events (which carry no routing key and pair on the display
    /// string) still pair for a session or archive written under v1. A reader
    /// never rewrites a legacy string; it only stops producing new ones.
    #[must_use]
    pub fn reference(&self) -> String {
        match (self.schema, self.profile.as_deref()) {
            (RosterSchema::V1, Some(alias)) => format!("{alias}:{}", self.name),
            _ => self.name.clone(),
        }
    }
}

/// The two spellings a positive server selector normalizes to (SC-405l).
///
/// The TYPE is part of the fact, not a rendering detail: `-L name` and
/// `-S /path` address different servers, and a normalizer that flattened both
/// to a string would let one spelling stand in for the other.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Selector {
    /// `positive(name:<nonempty>)`.
    Name(String),
    /// `positive(socket:<absolute-path>)`.
    Socket(PathBuf),
}

/// What a durable record says about its tmux server — SC-405l's typed knowledge
/// fact, normalized from the two-key legacy form.
///
/// Exactly one of four values, and only `Positive` confers SC-017j entitlement
/// or supports SC-017k liveness. `Missing` and `Ambiguous` leave the candidate
/// inventoried and route liveness through SC-017l's `unknown` — they are not
/// failures to retry, they are the ratified shape of not knowing.
///
/// **Fail closed.** Every combination the row does not name explicitly is
/// `Ambiguous`, because the cost of guessing wrong is querying a server ae was
/// never entitled to query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerSelector {
    /// A positive, unambiguous selector.
    Positive(Selector),
    /// No selector was recorded.
    Missing,
    /// Recorded, but it does not identify one server.
    Ambiguous,
}

impl ServerSelector {
    /// The selector this record entitles ae to query, if any.
    #[must_use]
    pub const fn entitles(&self) -> Option<&Selector> {
        match self {
            Self::Positive(selector) => Some(selector),
            Self::Missing | Self::Ambiguous => None,
        }
    }
}

/// Something in the meta this reader is not authorised to interpret.
///
/// Every variant maps to a row that is still open, and carries the 1-based line
/// so the report can point at it.
/// A v2 metadata row waiting for its seat.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingRow {
    slot: String,
    value: String,
    key: String,
    line: usize,
    /// A later duplicate of this metadata key was met: its VALUE is invalidated
    /// (SC-405e), but the provenance survives so a still-later v1/mixed claim
    /// can reclassify the row as an unknown key — the same answer the v1-first
    /// order gives.
    duplicated: bool,
}

/// Which schemas have claimed a slot by KEY so far, and how often v2 did:
/// a second `seat.<slot>` claim — keyed or bare — is a seat in doubt.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SlotClaim {
    slot: String,
    v1: bool,
    v2_claims: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Anomaly {
    /// SC-405d, UNCLASSIFIED — a key outside SC-405b/c.
    UnknownKey {
        /// The key as written.
        key: String,
        /// 1-based line number.
        line: usize,
    },
    /// SC-405e, UNCLASSIFIED — a line with no `=` at all.
    MalformedLine {
        /// 1-based line number.
        line: usize,
    },
    /// SC-405e, UNCLASSIFIED — a key that appears more than once.
    DuplicateKey {
        /// The key as written.
        key: String,
        /// 1-based line number of the repeat.
        line: usize,
    },
    /// SC-405c — a roster value that is not `alias:name[:session-id]`, or an
    /// identity-v2 `seat.<slot>` with an empty name.
    MalformedRosterEntry {
        /// The key as written.
        key: String,
        /// 1-based line number.
        line: usize,
    },
    /// Identity v2 — one slot named by BOTH `agent.<slot>` and `seat.<slot>`.
    /// Two schemas claiming one seat is an identity in doubt: the slot
    /// contributes no agent, and the session degrades with reason. Judged on
    /// the raw KEYS met (bare, malformed or duplicated ones included), never on
    /// a surviving entry, so no order of claims can resurrect the slot.
    MixedSchemaSlot {
        /// The slot as written.
        slot: String,
        /// 1-based line number of the claim that made (or found) it mixed.
        line: usize,
    },
    /// Identity v2 — one NAME carried by more than one seat. Under v2 the name
    /// IS the identity, so two seats with one name are one identity in doubt:
    /// every v2 seat carrying it is dropped (a v1 row keeps its `alias:name`
    /// ref, whose identity is the pair) and the session degrades with reason.
    DuplicateName {
        /// The name as written.
        name: String,
        /// 1-based line number of the row that revealed the collision.
        line: usize,
    },
}

impl fmt::Display for Anomaly {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownKey { key, line } => write!(f, "unknown meta key {key} at line {line}"),
            Self::MalformedLine { line } => write!(f, "malformed meta line {line}"),
            Self::DuplicateKey { key, line } => {
                write!(f, "duplicate meta key {key} at line {line}")
            }
            Self::MalformedRosterEntry { key, line } => {
                write!(f, "malformed roster entry {key} at line {line}")
            }
            Self::MixedSchemaSlot { slot, line } => {
                write!(
                    f,
                    "slot {slot} is named by both agent.* and seat.* (line {line})"
                )
            }
            Self::DuplicateName { name, line } => {
                write!(
                    f,
                    "roster name {name} is claimed by more than one seat (line {line})"
                )
            }
        }
    }
}

/// A parsed session `meta`.
///
/// A roster entry exists only once its `agent.<slot>` has been read and
/// validated, so a [`RosterEntry`] with no identity is unrepresentable. An
/// `agent_bin.<slot>` seen first waits in `pending_binaries` instead of
/// creating a half-built entry — which is why [`Meta::roster`] needs no filter,
/// and why there is no predicate here asking whether an agent has a name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Meta {
    mode: Option<String>,
    origin: Option<String>,
    work_dir: Option<String>,
    goal: Option<String>,
    /// The frozen executable version recorded when ae created the session.
    ///
    /// This belongs to the human list subline, not the SC-509 digest.  Keeping
    /// it here means presentation receives the value read from this session's
    /// meta rather than substituting the version of the binary doing the read.
    ae_version: Option<String>,
    roster: Vec<RosterEntry>,
    /// `agent_bin.<slot>` values whose `agent.<slot>` / `seat.<slot>` has not
    /// been read yet. A slot that never gets one simply never becomes an agent.
    pending_binaries: Vec<(String, String)>,
    /// Identity v2 metadata rows (`profile.<slot>`, `harness_session.<slot>`)
    /// read before their `seat.<slot>` — same rule as the binaries. They keep
    /// their key and line because a v1 claim turns them into SC-405d unknowns.
    pending_profiles: Vec<PendingRow>,
    pending_harness: Vec<PendingRow>,
    /// Provenance (key + line) of every v2 metadata row that ATTACHED to a V2
    /// seat, kept so the row can be reclassified as SC-405d unknown if its
    /// slot later turns out mixed — the same answer the other order gives.
    attached: Vec<PendingRow>,
    /// Every `agent.<slot>` / `seat.<slot>` KEY met so far — `=` or not, valid
    /// or not, first or repeated. A slot's schema is judged on these raw claims,
    /// never on a surviving entry: a malformed, bare or duplicated first claim
    /// leaves no entry, and judging on entries would let the other schema
    /// resurrect the slot. `archive::roster` judges presence the same way.
    claims: Vec<SlotClaim>,
    /// Slots claimed by both schemas: neither claim contributes an agent.
    mixed_slots: Vec<String>,
    /// v2 names already found on more than one seat: no later seat may take them.
    doubtful_names: Vec<String>,
    /// The raw `schema=` value, where the writer recorded one.
    schema: Option<String>,
    /// SC-405l's two selector keys, kept RAW. `None` is absent; `Some("")` is
    /// present-and-empty, and the row makes those two different answers.
    server_value: Option<String>,
    server_kind: Option<String>,
    /// Whether either selector key appeared more than once. SC-405l makes a
    /// duplicate ambiguous whether or not the repeats agree, so what the second
    /// one SAID is not kept.
    server_duplicated: bool,
    anomalies: Vec<Anomaly>,
}

impl Meta {
    /// Read and parse the `meta` inside the session directory at `dir`.
    ///
    /// # Errors
    ///
    /// Returns the underlying [`io::Error`] — an absent meta included.
    /// **SC-405i** makes that absence DEGRADE the session, in deliberate
    /// contrast with SC-519's quiet treatment of an absent event log: a fresh
    /// session has no events yet, but a session with no meta has lost its
    /// context and its whole roster at once.
    pub fn read(dir: &Path) -> io::Result<Self> {
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: the meta read itself — see clippy.toml"
        )]
        let text = fs::read_to_string(dir.join(FILE))?;
        Ok(Self::parse(&text))
    }

    /// Parse meta text (SC-405a).
    ///
    /// ```
    /// let meta = ae::meta::Meta::parse("mode=local\nwork_dir=/tmp/x\n");
    /// assert_eq!(meta.mode(), Some("local"));
    /// assert!(meta.anomalies().is_empty());
    /// ```
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut meta = Self::default();
        let mut seen: Vec<String> = Vec::new();
        for (index, raw) in text.split('\n').enumerate() {
            let line = index + 1;
            // One trailing carriage return is line ENDING, not value: a CRLF
            // meta would otherwise put an invisible byte on the end of a path.
            let raw = raw.strip_suffix('\r').unwrap_or(raw);
            if raw.is_empty() {
                continue;
            }
            // SC-405a: split on the FIRST equals. A value may contain more.
            let Some((key, value)) = raw.split_once('=') else {
                // A bare `agent.main` / `seat.main` is still a CLAIM on the
                // slot (the frozen `_ar_roster_slots` names the slot from it),
                // so it is noted before the line is refused.
                meta.note_claim(raw, line, false);
                meta.anomalies.push(Anomaly::MalformedLine { line });
                continue;
            };
            // Claims are judged on the raw key, BEFORE the duplicate check and
            // before any value is parsed.
            let already_seen = seen.iter().any(|previous| previous == key);
            meta.note_claim(key, line, already_seen);
            if already_seen {
                // SC-405e is UNCLASSIFIED: nobody has ruled whether the first
                // or the last occurrence wins. Publishing either one would be
                // FABRICATION — the digest would carry a value the contract
                // does not say is the value. So the field is INVALIDATED: the
                // reader reports a duplicate and emits nothing for that key at
                // all, and the session degrades. When the seats rule
                // precedence, this becomes a choice instead of a refusal.
                meta.anomalies.push(Anomaly::DuplicateKey {
                    key: key.to_owned(),
                    line,
                });
                meta.invalidate(key);
                continue;
            }
            seen.push(key.to_owned());
            meta.absorb(key, value, line);
        }
        meta
    }

    /// Drop whatever a key contributed, because the meta named it twice and no
    /// row says which time counts.
    fn invalidate(&mut self, key: &str) {
        match key {
            "mode" => self.mode = None,
            "origin" => self.origin = None,
            "work_dir" => self.work_dir = None,
            "goal" => self.goal = None,
            "ae_version" => self.ae_version = None,
            SCHEMA_KEY => self.schema = None,
            // A repeated selector key is AMBIGUOUS by SC-405l, which is a
            // stronger statement than SC-405e's invalidation: the flag survives
            // even if the repeats agreed.
            SERVER_KEY | SERVER_KIND_KEY => self.server_duplicated = true,
            _ => {
                if let Some(slot) = key.strip_prefix(ROSTER_BIN_PREFIX) {
                    if let Some(entry) = self.roster.iter_mut().find(|e| e.slot == slot) {
                        entry.binary = None;
                    }
                    self.pending_binaries.retain(|(pending, _)| pending != slot);
                } else if let Some(slot) = key.strip_prefix(PROFILE_PREFIX) {
                    // Only a V2 entry ever took the row; a V1 entry's profile
                    // is the alias from `agent.<slot>` and stays byte-stable.
                    if let Some(entry) = self.roster.iter_mut().find(|e| e.slot == slot)
                        && entry.schema == RosterSchema::V2
                    {
                        entry.profile = None;
                    }
                    self.mark_metadata_duplicated(key);
                } else if let Some(slot) = key.strip_prefix(HARNESS_SESSION_PREFIX) {
                    if let Some(entry) = self.roster.iter_mut().find(|e| e.slot == slot)
                        && entry.schema == RosterSchema::V2
                    {
                        entry.harness_session = None;
                    }
                    self.mark_metadata_duplicated(key);
                } else if let Some(slot) = key
                    .strip_prefix(ROSTER_PREFIX)
                    .or_else(|| key.strip_prefix(SEAT_PREFIX))
                {
                    // A doubly-named slot is a slot whose identity is in doubt,
                    // and SC-405k says agents[] membership is roster-defined —
                    // so it contributes no agent rather than a guessed one.
                    self.roster.retain(|entry| entry.slot != slot);
                }
            }
        }
    }

    fn absorb(&mut self, key: &str, value: &str, line: usize) {
        match key {
            "mode" => self.mode = Some(value.to_owned()),
            "origin" => self.origin = Some(value.to_owned()),
            "work_dir" => self.work_dir = Some(value.to_owned()),
            "goal" => self.goal = Some(value.to_owned()),
            "ae_version" => self.ae_version = Some(value.to_owned()),
            // SC-405l supersedes SC-405d for exactly this family: these two are
            // read and normalized rather than tolerated-and-ignored. Every
            // OTHER unknown key stays uninterpreted below.
            SERVER_KEY => self.server_value = Some(value.to_owned()),
            SERVER_KIND_KEY => self.server_kind = Some(value.to_owned()),
            SCHEMA_KEY => self.schema = Some(value.to_owned()),
            _ => {
                if let Some(slot) = key.strip_prefix(ROSTER_BIN_PREFIX) {
                    self.set_binary(slot, value);
                } else if let Some(slot) = key.strip_prefix(PROFILE_PREFIX) {
                    self.set_metadata(Metadata::Profile, slot, key, value, line);
                } else if let Some(slot) = key.strip_prefix(HARNESS_SESSION_PREFIX) {
                    self.set_metadata(Metadata::HarnessSession, slot, key, value, line);
                } else if let Some(slot) = key.strip_prefix(ROSTER_PREFIX) {
                    // `note_claim` already refused a mixed slot with its anomaly.
                    if !self.is_mixed(slot) {
                        self.absorb_roster(key, slot, value, line);
                    }
                } else if let Some(slot) = key.strip_prefix(SEAT_PREFIX) {
                    // A mixed slot and a repeated v2 claim were both refused
                    // by `note_claim` with their anomalies.
                    if !self.is_mixed(slot) && !self.is_v2_repeated(slot) {
                        self.absorb_seat(key, slot, value, line);
                    }
                } else {
                    // SC-405d, unclassified: recorded, never interpreted.
                    self.anomalies.push(Anomaly::UnknownKey {
                        key: key.to_owned(),
                        line,
                    });
                }
            }
        }
    }

    /// SC-405c + SC-1207b: `alias:name` with an optional provider session id.
    fn absorb_roster(&mut self, key: &str, slot: &str, value: &str, line: usize) {
        let parts: Vec<&str> = value.split(':').collect();
        let (alias, name, session_id) = match parts.as_slice() {
            [alias, name] => (*alias, *name, None),
            [alias, name, session_id] => (*alias, *name, Some((*session_id).to_owned())),
            _ => {
                self.anomalies.push(Anomaly::MalformedRosterEntry {
                    key: key.to_owned(),
                    line,
                });
                return;
            }
        };
        if alias.is_empty() || name.is_empty() {
            self.anomalies.push(Anomaly::MalformedRosterEntry {
                key: key.to_owned(),
                line,
            });
            return;
        }
        // A duplicate `agent.<slot>` is caught upstream by the duplicate-key
        // check, so this slot cannot already be in the roster.
        let binary = self.take_pending_binary(slot);
        // v2 metadata on a v1 slot is already uninterpreted by `note_claim`
        // (which fired for this `agent.<slot>` KEY before this absorb, so a
        // malformed or bare v1 claim reclassifies it too). The alias and sid
        // encoded in `agent.<slot>` are the identity, byte-stable.
        // A v2 seat already carrying this NAME is an identity in doubt (v2's
        // identity is the bare name); the v1 row keeps its `alias:name` ref.
        // Two v1 rows sharing a name are two refs (`a:x`, `b:x`) — frozen, no
        // anomaly.
        if self
            .roster
            .iter()
            .any(|entry| entry.name == name && entry.schema == RosterSchema::V2)
        {
            self.mark_name_doubtful(name, line);
        }
        self.roster.push(RosterEntry {
            slot: slot.to_owned(),
            name: name.to_owned(),
            profile: Some(alias.to_owned()),
            harness_session: session_id.filter(|id| !id.is_empty()),
            binary,
            schema: RosterSchema::V1,
        });
    }

    /// Identity v2: `seat.<slot>=<name>`. The name is the identity; an empty
    /// one is a malformed entry exactly as an empty v1 name is.
    fn absorb_seat(&mut self, key: &str, slot: &str, value: &str, line: usize) {
        if value.is_empty() {
            self.anomalies.push(Anomaly::MalformedRosterEntry {
                key: key.to_owned(),
                line,
            });
            return;
        }
        // The name is the identity, so it must be UNIQUE across the roster —
        // against every other seat, v1 rows included (their name half is what a
        // bare-name address matches). A collision drops every v2 seat carrying
        // the name, now and later; a v1 row is never removed by it.
        if self.doubtful_names.iter().any(|n| n == value) {
            self.anomalies.push(Anomaly::DuplicateName {
                name: value.to_owned(),
                line,
            });
            return;
        }
        if self.roster.iter().any(|entry| entry.name == value) {
            self.mark_name_doubtful(value, line);
            return;
        }
        let binary = self.take_pending_binary(slot);
        // Empty v2 metadata is ABSENT metadata (the seat is still an agent);
        // its provenance is kept so a later mixed claim can reclassify it.
        let profile = take_pending(&mut self.pending_profiles, slot).and_then(|row| {
            let usable = (!row.value.is_empty() && !row.duplicated).then(|| row.value.clone());
            self.attached.push(row);
            usable
        });
        let harness_session = take_pending(&mut self.pending_harness, slot).and_then(|row| {
            let usable = (!row.value.is_empty() && !row.duplicated).then(|| row.value.clone());
            self.attached.push(row);
            usable
        });
        self.roster.push(RosterEntry {
            slot: slot.to_owned(),
            name: value.to_owned(),
            profile,
            harness_session,
            binary,
            schema: RosterSchema::V2,
        });
    }

    /// Record which schema a raw KEY claims for its slot. The moment a slot
    /// has been claimed by both, it is MIXED: its entry (if any) is dropped, its
    /// metadata (pending or attached) is uninterpreted, the anomaly is recorded,
    /// and every later claim of either prefix is refused with its own anomaly.
    /// A SECOND v2 claim — keyed or bare — is a seat in doubt: the entry is
    /// dropped and the slot stays absent whichever order the rows came in
    /// (`already_seen` says the duplicate-key check will record it; a bare
    /// repeat is recorded here). A v1 bare row keeps its frozen behaviour: a
    /// malformed line, and the keyed row still names the agent.
    fn note_claim(&mut self, key: &str, line: usize, already_seen: bool) {
        let (slot, schema) = if let Some(slot) = key.strip_prefix(ROSTER_PREFIX) {
            (slot, RosterSchema::V1)
        } else if let Some(slot) = key.strip_prefix(SEAT_PREFIX) {
            (slot, RosterSchema::V2)
        } else {
            return;
        };
        if self.is_mixed(slot) {
            self.anomalies.push(Anomaly::MixedSchemaSlot {
                slot: slot.to_owned(),
                line,
            });
            return;
        }
        let at = self
            .claims
            .iter()
            .position(|claim| claim.slot == slot)
            .unwrap_or_else(|| {
                self.claims.push(SlotClaim {
                    slot: slot.to_owned(),
                    v1: false,
                    v2_claims: 0,
                });
                self.claims.len() - 1
            });
        match schema {
            RosterSchema::V1 => {
                self.claims[at].v1 = true;
                // A v1 claim — VALID, malformed or bare — makes the slot v1, so
                // its v2 metadata (pending or attached) is uninterpreted NOW,
                // at the single point where "a v1 key landed" is known. This is
                // what makes metadata-then-malformed/bare-v1 match the reverse
                // order.
                self.uninterpret_pending(slot);
            }
            RosterSchema::V2 => self.claims[at].v2_claims += 1,
        }
        if self.claims[at].v1 && self.claims[at].v2_claims > 0 {
            self.mixed_slots.push(slot.to_owned());
            self.roster.retain(|entry| entry.slot != slot);
            self.uninterpret_pending(slot);
            self.anomalies.push(Anomaly::MixedSchemaSlot {
                slot: slot.to_owned(),
                line,
            });
        } else if schema == RosterSchema::V2 && self.claims[at].v2_claims > 1 {
            self.roster.retain(|entry| entry.slot != slot);
            if !already_seen {
                self.anomalies.push(Anomaly::DuplicateKey {
                    key: key.to_owned(),
                    line,
                });
            }
        }
    }

    /// Flag every open metadata row for `key` value-invalidated, keeping its
    /// provenance. SC-405e for a v2 metadata key: the value is gone, the row is
    /// not — a later v1/mixed claim still reclassifies it as an unknown key.
    fn mark_metadata_duplicated(&mut self, key: &str) {
        for list in [
            &mut self.pending_profiles,
            &mut self.pending_harness,
            &mut self.attached,
        ] {
            for row in list.iter_mut() {
                if row.key == key {
                    row.duplicated = true;
                }
            }
        }
    }

    fn is_v2_repeated(&self, slot: &str) -> bool {
        self.claims
            .iter()
            .any(|claim| claim.slot == slot && claim.v2_claims > 1)
    }

    fn is_mixed(&self, slot: &str) -> bool {
        self.mixed_slots.iter().any(|mixed| mixed == slot)
    }

    /// Drop every v2 seat named `name` (a v1 row never is), record the
    /// collision at `line`, and remember the name so no later seat takes it.
    fn mark_name_doubtful(&mut self, name: &str, line: usize) {
        self.roster
            .retain(|entry| !(entry.name == name && entry.schema == RosterSchema::V2));
        if !self.doubtful_names.iter().any(|n| n == name) {
            self.doubtful_names.push(name.to_owned());
        }
        self.anomalies.push(Anomaly::DuplicateName {
            name: name.to_owned(),
            line,
        });
    }

    /// Pending AND attached v2 metadata for `slot` becomes SC-405d unknown
    /// keys: the slot turned out not to be a v2 seat (a v1 claim, or a mixed
    /// one) — the same answer whichever order the rows came in.
    fn uninterpret_pending(&mut self, slot: &str) {
        for pending in [
            &mut self.pending_profiles,
            &mut self.pending_harness,
            &mut self.attached,
        ] {
            let mut kept = Vec::with_capacity(pending.len());
            for row in pending.drain(..) {
                if row.slot == slot {
                    self.anomalies.push(Anomaly::UnknownKey {
                        key: row.key,
                        line: row.line,
                    });
                } else {
                    kept.push(row);
                }
            }
            *pending = kept;
        }
    }

    /// `profile.<slot>` / `harness_session.<slot>`: attaches to a V2 seat,
    /// waits for one not read yet, and is an UNKNOWN KEY on a slot that is v1
    /// or mixed — the one explicit rule, order-independent.
    fn set_metadata(&mut self, which: Metadata, slot: &str, key: &str, value: &str, line: usize) {
        let unknown = Anomaly::UnknownKey {
            key: key.to_owned(),
            line,
        };
        if self.is_mixed(slot) {
            self.anomalies.push(unknown);
            return;
        }
        let row = PendingRow {
            slot: slot.to_owned(),
            value: value.to_owned(),
            key: key.to_owned(),
            line,
            duplicated: false,
        };
        // Ownership FIRST, value second: an empty row on a v1 slot is still an
        // unknown key; an empty row on a v2 seat is absent metadata.
        match self.roster.iter_mut().find(|entry| entry.slot == slot) {
            Some(existing) if existing.schema == RosterSchema::V2 => {
                let value = (!value.is_empty()).then(|| value.to_owned());
                match which {
                    Metadata::Profile => existing.profile = value,
                    Metadata::HarnessSession => existing.harness_session = value,
                }
                self.attached.push(row);
            }
            Some(_) => self.anomalies.push(unknown),
            None => {
                // No entry yet. A v1 KEY already claimed the slot (its value
                // was malformed or duplicated) is still a v1 slot.
                if self
                    .claims
                    .iter()
                    .any(|claim| claim.slot == slot && claim.v1)
                {
                    self.anomalies.push(unknown);
                    return;
                }
                match which {
                    Metadata::Profile => self.pending_profiles.push(row),
                    Metadata::HarnessSession => self.pending_harness.push(row),
                }
            }
        }
    }

    /// `agent_bin.<slot>` may appear before or after its `agent.<slot>`. Before,
    /// it waits; after, it attaches. It never creates an agent by itself: a
    /// binary is not an identity.
    fn set_binary(&mut self, slot: &str, value: &str) {
        if value.is_empty() {
            return;
        }
        match self.roster.iter_mut().find(|entry| entry.slot == slot) {
            Some(existing) => existing.binary = Some(value.to_owned()),
            None => self
                .pending_binaries
                .push((slot.to_owned(), value.to_owned())),
        }
    }

    fn take_pending_binary(&mut self, slot: &str) -> Option<String> {
        let at = self
            .pending_binaries
            .iter()
            .position(|(pending, _)| pending == slot)?;
        Some(self.pending_binaries.swap_remove(at).1)
    }

    /// The normalized server selector — SC-405l, read side only.
    ///
    /// The row's mapping, transcribed rather than paraphrased:
    ///
    /// | recorded | normalizes to |
    /// |---|---|
    /// | `kind=name` + nonempty value | `positive(name)` |
    /// | `kind=socket` + nonempty ABSOLUTE value | `positive(socket)` |
    /// | `kind=ambiguous` | `ambiguous` |
    /// | kind ABSENT + nonempty value | `positive(name)` (the legacy form) |
    /// | no value and no nonempty kind | `missing` |
    /// | anything else | `ambiguous` |
    ///
    /// "Anything else" is not a catch-all for tidiness — it is the row's
    /// fail-closed rule, and it covers an unknown kind, a typed empty value, a
    /// relative socket path, a present-but-EMPTY kind beside a nonempty value,
    /// and duplicate or conflicting selector keys. An absent kind and an empty
    /// kind are deliberately different answers.
    ///
    /// This is READ normalization. No successor writer may emit this fact until
    /// its encoding is separately ratified, so there is no inverse here.
    ///
    /// ```
    /// use ae::meta::{Meta, Selector, ServerSelector};
    ///
    /// let legacy = Meta::parse("tmux_server=work\n");
    /// assert_eq!(
    ///     legacy.server_selector(),
    ///     ServerSelector::Positive(Selector::Name("work".to_owned()))
    /// );
    /// assert_eq!(Meta::parse("mode=local\n").server_selector(), ServerSelector::Missing);
    /// ```
    #[must_use]
    pub fn server_selector(&self) -> ServerSelector {
        if self.server_duplicated {
            return ServerSelector::Ambiguous;
        }
        let value = self.server_value.as_deref().unwrap_or_default();
        match self.server_kind.as_deref() {
            // Absent kind: the legacy one-key form, which named a server.
            None if !value.is_empty() => ServerSelector::Positive(Selector::Name(value.to_owned())),
            None => ServerSelector::Missing,
            // Present but EMPTY. With no value it is still "no nonempty kind",
            // which the row calls missing; beside a value it is ambiguous.
            Some("") if value.is_empty() => ServerSelector::Missing,
            Some("name") if !value.is_empty() => {
                ServerSelector::Positive(Selector::Name(value.to_owned()))
            }
            Some("socket") if Path::new(value).is_absolute() => {
                ServerSelector::Positive(Selector::Socket(PathBuf::from(value)))
            }
            // Unknown kind, typed empty value, relative socket, empty kind
            // beside a value, explicit `ambiguous` — all one answer.
            Some(_) => ServerSelector::Ambiguous,
        }
    }

    /// SC-405b — the copy mode the session was started in.
    #[must_use]
    pub fn mode(&self) -> Option<&str> {
        self.mode.as_deref()
    }

    /// SC-405b — where the session came from.
    #[must_use]
    pub fn origin(&self) -> Option<&str> {
        self.origin.as_deref()
    }

    /// SC-405b — the working directory its agents run in.
    #[must_use]
    pub fn work_dir(&self) -> Option<&str> {
        self.work_dir.as_deref()
    }

    /// SC-405b — the session's one-line objective.
    #[must_use]
    pub fn goal(&self) -> Option<&str> {
        self.goal.as_deref()
    }

    /// The ae version captured in this session's meta for the human list
    /// subline. An absent or empty value has frozen's visible `?` fallback.
    #[must_use]
    pub fn ae_version(&self) -> Option<&str> {
        self.ae_version.as_deref()
    }

    /// SC-405c — the roster, in the order the meta lists its `agent.<slot>`
    /// keys.
    ///
    /// No filtering: an entry only exists here once it has an identity, so
    /// there is no half-built one to screen out. A `agent_bin.<slot>` with no
    /// `agent.<slot>` never became an entry in the first place.
    #[must_use]
    pub fn roster(&self) -> &[RosterEntry] {
        &self.roster
    }

    /// The raw `schema=` value the writer recorded, if any. Absent means the
    /// meta predates identity v2 (every roster row is `agent.<slot>`).
    #[must_use]
    pub fn schema(&self) -> Option<&str> {
        self.schema.as_deref()
    }

    /// Everything this reader met and is not authorised to interpret.
    ///
    /// Non-empty means the session is degraded-with-reason until SC-405d and
    /// SC-405e close.
    #[must_use]
    pub fn anomalies(&self) -> &[Anomaly] {
        &self.anomalies
    }
}

/// Which v2 metadata row a key carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Metadata {
    Profile,
    HarnessSession,
}

/// Remove and return the row pending for `slot`, if any.
fn take_pending(pending: &mut Vec<PendingRow>, slot: &str) -> Option<PendingRow> {
    let at = pending.iter().position(|row| row.slot == slot)?;
    Some(pending.swap_remove(at))
}

/// The lock the frozen `ae_meta_set`/`ae_meta_unset` take: `meta.lock` beside
/// the file, `flock -w 5`.
const LOCK: &str = "meta.lock";

/// The meta file's raw bytes — the read behind the frozen `ae_meta_get`,
/// which greps the file rather than parsing it.
///
/// Not [`Meta::read`]: that parser serves the digest, where a duplicated key is
/// an anomaly the contract has not ruled on (SC-405e) and so contributes
/// nothing. A helper asked for its own key answers with the FIRST record, as
/// its bash body always has — [`first_value`] — and the bytes it prints are
/// the file's, not a decoded string's.
///
/// # Errors
///
/// The underlying [`io::Error`]; an absent meta is `NotFound`.
pub fn read_bytes(dir: &Path) -> io::Result<Vec<u8>> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the raw meta read behind a helper's own-key lookup — see clippy.toml"
    )]
    let bytes = fs::read(dir.join(FILE));
    bytes
}

/// The value of the FIRST `<key>=` record — `grep "^<key>=" | head -1 |
/// cut -d= -f2-`: records are `\n`-separated (an unterminated last one
/// counts), the key must be followed by `=` exactly, and everything after
/// that first `=` is the value, bytes verbatim — a trailing CR included, as it
/// is to grep.
///
/// ```
/// use ae::meta::first_value;
///
/// let text = b"goal=a=b\r\ngoals=no\ngoal=second\nmode=local";
/// assert_eq!(first_value(text, "goal"), Some(b"a=b\r".as_slice()));
/// assert_eq!(first_value(text, "mode"), Some(b"local".as_slice()));
/// assert_eq!(first_value(text, "goa"), None);
/// assert_eq!(first_value(b"goal=\n", "goal"), Some(b"".as_slice()));
/// ```
#[must_use]
pub fn first_value<'a>(text: &'a [u8], key: &str) -> Option<&'a [u8]> {
    text.split(|byte| *byte == b'\n').find_map(|line| {
        line.strip_prefix(key.as_bytes())
            .and_then(|rest| rest.strip_prefix(b"="))
    })
}

/// The value of `key` when the meta names it EXACTLY ONCE, or `None`.
///
/// The fail-closed reading of a flat record file, for the keys where a
/// duplicate is not a tie to break but a record whose meaning is in doubt. It
/// is the same answer [`Meta::parse`] reaches for the keys it absorbs (SC-405e:
/// a duplicated key contributes NOTHING, because no row says whether the first
/// or the last occurrence wins, and publishing either would be fabrication).
/// This is that rule for a key `Meta` leaves uninterpreted.
///
/// Distinct from [`first_value`] on purpose, and the caller picks: `first_value`
/// answers "what does the first record say", which is right where a later
/// duplicate is harmless; this answers "does the file say ONE thing", which is
/// what a flag guarding behavior needs.
///
/// # Bash parity
///
/// The frozen helpers read such a flag as `grep '^key=' meta | cut -d= -f2-`
/// captured into a scalar, so two records make a value with a NEWLINE in it,
/// which equals neither one alone. Any duplicate therefore fails every `==`
/// comparison bash makes against it — `None` here is that same answer, reached
/// by a route that does not depend on a shell's field splitting.
#[must_use]
pub fn sole_value<'a>(text: &'a [u8], key: &str) -> Option<&'a [u8]> {
    let mut found = None;
    for line in text.split(|byte| *byte == b'\n') {
        let Some(value) = line
            .strip_prefix(key.as_bytes())
            .and_then(|rest| rest.strip_prefix(b"="))
        else {
            continue;
        };
        if found.is_some() {
            return None; // named twice: the record does not say one thing
        }
        found = Some(value);
    }
    found
}

/// `text` with `key` set to `value`, or removed when `value` is `None` — the
/// frozen helpers' awk, byte for byte (measured against it):
///
/// * a record is what precedes each `\n`; a final unterminated remainder is
///   a record too, and comes out terminated, as awk's `print` terminates it;
/// * a record's key is everything before its first `=` (a record without one
///   is its own key), compared exactly — so a CR before the `\n` is part of
///   the last field, never of the key;
/// * EVERY matching record is replaced by `key=value\n` on a set, and every
///   one is dropped on an unset — a duplicated key stays duplicated, because
///   a rewrite that healed a degraded meta would hide the degradation;
/// * every other record is emitted VERBATIM, its CR included;
/// * a set whose key matched nothing appends `key=value\n`.
#[must_use]
pub fn rewritten(text: &str, key: &str, value: Option<&str>) -> String {
    let mut out = String::new();
    let mut updated = false;
    let mut records: Vec<&str> = text.split('\n').collect();
    // A final empty segment is the terminator of the last record, not a record.
    if records.last() == Some(&"") {
        records.pop();
    }
    for record in records {
        let record_key = record.split_once('=').map_or(record, |(k, _)| k);
        if record_key == key {
            if let Some(value) = value {
                out.push_str(key);
                out.push('=');
                out.push_str(value);
                out.push('\n');
                updated = true;
            }
            continue;
        }
        out.push_str(record);
        out.push('\n');
    }
    if let Some(value) = value
        && !updated
    {
        out.push_str(key);
        out.push('=');
        out.push_str(value);
        out.push('\n');
    }
    out
}

/// Why a [`rewrite`] did not complete — and, crucially, WHAT IS KNOWN about
/// the meta afterwards.
#[derive(Debug)]
pub enum RewriteError {
    /// Nothing visible changed: the lock, the read, the temp write, its sync,
    /// or the rename failed. The previous meta is still the meta.
    NotWritten(io::Error),
    /// The rename returned — the new meta IS visible — but the directory
    /// entry could not be synced, so whether it survives a crash is unknown.
    /// Reported as exactly that, never as "nothing changed".
    Unknown(io::Error),
}

impl RewriteError {
    /// The underlying cause.
    #[must_use]
    pub const fn cause(&self) -> &io::Error {
        match self {
            Self::NotWritten(why) | Self::Unknown(why) => why,
        }
    }
}

/// Set (`Some`) or remove (`None`) `key` in the `meta` at `dir`, under
/// `meta.lock`, by rewriting a temp file and renaming it over — the frozen
/// `ae_meta_set`/`ae_meta_unset`, made durable: the temp is synced before the
/// rename, and the directory is synced after it, because a synced inode
/// behind an unsynced directory entry is a meta that can revert on a crash
/// while the event announcing it survives.
///
/// `value` is written as given; the caller makes it one line.
///
/// # Errors
///
/// [`RewriteError::NotWritten`] when nothing visible changed — the lock not
/// acquired within the bound, an absent meta on a SET (the frozen helper
/// returns 1; an unset of an absent meta is `Ok`, as its helper returns 0),
/// or any read, write, sync or rename failure. [`RewriteError::Unknown`] when
/// the rename returned but the directory sync did not.
pub fn rewrite(dir: &Path, key: &str, value: Option<&str>) -> Result<(), RewriteError> {
    let path = dir.join(FILE);
    let _held = crate::state::acquire(&dir.join(LOCK), crate::state::LOCK_WAIT)
        .map_err(RewriteError::NotWritten)?;
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the meta read, for its locked rewrite — see clippy.toml"
    )]
    let current = fs::read_to_string(&path);
    let current = match current {
        Ok(text) => text,
        Err(why) if why.kind() == io::ErrorKind::NotFound && value.is_none() => return Ok(()),
        Err(why) => return Err(RewriteError::NotWritten(why)),
    };
    let next = rewritten(&current, key, value);
    publish_bytes(dir, &path, next.as_bytes())
}

/// The whole INITIAL meta, published as one document under the meta lock — the
/// core side of `_meta-init`. A reader sees the OLD meta, no meta, or the
/// COMPLETE new one: never a half-built roster. `content` is written verbatim
/// (the caller assembles every `key=value\n` line and the trailing newline),
/// so per-seat appends — the shape that once left the roster observable
/// half-built — are impossible by construction. Publishing OVER an existing
/// meta is refused: initialisation is a create, and clobbering a live session's
/// meta with a fresh document is never what `_meta-init` means (a rewrite of an
/// existing session goes through [`rewrite`], key by key, or a future batch
/// rewrite that takes the current bytes first).
///
/// # Errors
///
/// [`RewriteError::NotWritten`] when the lock is not acquired, a meta already
/// exists, or any write/sync/rename fails; [`RewriteError::Unknown`] when the
/// rename returned but the directory sync did not.
pub fn init(dir: &Path, content: &str) -> Result<(), RewriteError> {
    let path = dir.join(FILE);
    let _held = crate::state::acquire(&dir.join(LOCK), crate::state::LOCK_WAIT)
        .map_err(RewriteError::NotWritten)?;
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: init must not clobber a live meta, so it lstats the target (never following) before publishing — see clippy.toml"
    )]
    let existing = std::fs::symlink_metadata(&path);
    match existing {
        Ok(_) => {
            return Err(RewriteError::NotWritten(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "meta already exists — init publishes the first meta only",
            )));
        }
        Err(why) if why.kind() == io::ErrorKind::NotFound => {}
        Err(why) => return Err(RewriteError::NotWritten(why)),
    }
    publish_bytes(dir, &path, content.as_bytes())
}

/// Stage `bytes` to a per-process temp beside `path`, fsync it, rename it over
/// `path`, then fsync the directory so the entry is durable. The rename makes
/// the first observable version the complete one; the two syncs are why a
/// meta cannot revert behind an event that announced it. The temp name carries
/// the pid AND a nonce so two writers under the same lock owner (or a crashed
/// one's leftover) cannot collide.
fn publish_bytes(dir: &Path, path: &Path, bytes: &[u8]) -> Result<(), RewriteError> {
    static NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let temp = dir.join(format!(
        "{FILE}.tmp.{}.{}",
        std::process::id(),
        NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let staged = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
    })();
    if staged.is_err() {
        let _ = fs::remove_file(&temp);
    }
    staged.map_err(RewriteError::NotWritten)?;
    // Visible now. Publish the directory entry, or say that it is not known
    // to be published.
    fs::OpenOptions::new()
        .read(true)
        .open(dir)
        .and_then(|directory| directory.sync_all())
        .map_err(RewriteError::Unknown)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        reason = "fixtures build and inspect real directories; the boundary is about \
                  what PRODUCT code may reach"
    )]

    #[test]
    fn a_rewrite_replaces_appends_or_drops_byte_for_byte_as_the_awk_does() {
        // Every expectation here was MEASURED against the frozen awk
        // (`ae_meta_set` / `ae_meta_unset`) on the same input, not derived.
        use super::rewritten;
        let text = "mode=local\ngoal=old\nsession=s\n";
        assert_eq!(
            rewritten(text, "goal", Some("new")),
            "mode=local\ngoal=new\nsession=s\n",
            "replaced in place"
        );
        assert_eq!(
            rewritten("mode=local\n", "goal", Some("g")),
            "mode=local\ngoal=g\n",
            "appended when absent"
        );
        assert_eq!(
            rewritten("", "goal", Some("x")),
            "goal=x\n",
            "an empty meta gets the line"
        );
        assert_eq!(
            rewritten(text, "goal", None),
            "mode=local\nsession=s\n",
            "dropped"
        );
        assert_eq!(
            rewritten(text, "absent", None),
            text,
            "unset of an absent key is a no-op"
        );
        // A value containing `=` keeps nothing of the old value; a record
        // without `=` is its own key; an unterminated last record comes out
        // terminated, as awk's print terminates it.
        assert_eq!(
            rewritten("goal=a=b\nbare\nlast=1", "goal", Some("x")),
            "goal=x\nbare\nlast=1\n"
        );
        assert_eq!(rewritten("bare\n", "bare", None), "");
        // CRLF PARITY: a non-matching record keeps its CR verbatim; a matching
        // record's CR was part of its last field and goes with it.
        assert_eq!(
            rewritten("a=1\r\ngoal=old\r\nb=2\r\n", "goal", Some("new")),
            "a=1\r\ngoal=new\nb=2\r\n"
        );
        assert_eq!(
            rewritten("a=1\r\ngoal=old\r\nb=2\r\n", "goal", None),
            "a=1\r\nb=2\r\n"
        );
        assert_eq!(
            rewritten("bare\r\nlast=1", "goal", Some("x")),
            "bare\r\nlast=1\ngoal=x\n",
            "a bare CR record is not the key `bare`"
        );
        // DUPLICATE PARITY: every matching record is replaced, so a duplicated
        // key stays duplicated — a rewrite that healed a degraded meta would
        // hide the degradation SC-405 reports.
        assert_eq!(rewritten("k=1\nk=2\n", "k", Some("3")), "k=3\nk=3\n");
        assert_eq!(rewritten("k=1\nk=2\n", "k", None), "");
    }

    #[test]
    fn init_publishes_the_whole_document_once_and_refuses_over_an_existing_meta() {
        let dir = std::env::temp_dir().join(format!("ae-meta-init-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        let content = "mode=local
schema=2
seat.main=lead
profile.main=fable5
agent_bin.main=claude
";
        super::init(&dir, content).expect("first init publishes");
        assert_eq!(
            std::fs::read_to_string(dir.join("meta")).unwrap(),
            content,
            "the file is exactly the document handed in"
        );
        // The roster it parses back is the complete v2 seat — never half-built.
        let meta = Meta::read(&dir).expect("readable");
        assert_eq!(meta.roster().len(), 1);
        assert_eq!(meta.roster()[0].name, "lead");
        assert_eq!(meta.roster()[0].profile.as_deref(), Some("fable5"));
        // A second init refuses: init is a create, never a clobber.
        let err = super::init(
            &dir,
            "mode=local
",
        )
        .unwrap_err();
        assert!(
            matches!(&err, super::RewriteError::NotWritten(why) if why.kind() == std::io::ErrorKind::AlreadyExists),
            "{err:?}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("meta")).unwrap(),
            content,
            "the refused second init left the first byte-identical"
        );
        // No temp survives a successful publish.
        let temps: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("meta.tmp."))
            .collect();
        assert!(temps.is_empty(), "a staged temp was left behind: {temps:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    use super::{Anomaly, Meta, RosterSchema, Selector, ServerSelector};
    use std::path::PathBuf;

    #[test]
    fn sc_405l_the_well_formed_selector_forms_normalize_to_their_typed_facts() {
        // Transcribed from the row's mapping table, one case per line, opposed
        // so that a normalizer collapsing two of them fails rather than passes.
        for (text, expected, why) in [
            (
                "tmux_server_kind=name\ntmux_server=work\n",
                ServerSelector::Positive(Selector::Name("work".to_owned())),
                "kind=name + nonempty value",
            ),
            (
                "tmux_server_kind=socket\ntmux_server=/tmp/ae/s.sock\n",
                ServerSelector::Positive(Selector::Socket(PathBuf::from("/tmp/ae/s.sock"))),
                "kind=socket + nonempty ABSOLUTE value",
            ),
            (
                "tmux_server_kind=ambiguous\ntmux_server=work\n",
                ServerSelector::Ambiguous,
                "explicit ambiguous",
            ),
            (
                "tmux_server=work\n",
                ServerSelector::Positive(Selector::Name("work".to_owned())),
                "kind ABSENT + nonempty value is the legacy positive(name)",
            ),
            (
                "mode=local\n",
                ServerSelector::Missing,
                "both selector fields absent",
            ),
        ] {
            assert_eq!(Meta::parse(text).server_selector(), expected, "{why}");
        }
    }

    #[test]
    fn sc_405l_all_four_readable_empty_combinations_are_missing() {
        // kind absent/empty x value absent/empty. `missing` means NO SELECTOR
        // FACT IS AVAILABLE — not a claim that readable bytes positively omitted
        // the keys — so all four land in the same place, and none of them is
        // `ambiguous`: nothing here was readable-and-contradictory.
        for (text, why) in [
            ("mode=local\n", "kind absent, value absent"),
            ("tmux_server=\n", "kind absent, value empty"),
            ("tmux_server_kind=\n", "kind empty, value absent"),
            (
                "tmux_server_kind=\ntmux_server=\n",
                "kind empty, value empty",
            ),
        ] {
            assert_eq!(
                Meta::parse(text).server_selector(),
                ServerSelector::Missing,
                "{why}"
            );
        }
    }

    #[test]
    fn sc_405l_every_other_combination_is_ambiguous_and_confers_nothing() {
        for (text, why) in [
            (
                "tmux_server_kind=socket-ish\ntmux_server=work\n",
                "unknown kind",
            ),
            ("tmux_server_kind=name\ntmux_server=\n", "typed empty value"),
            ("tmux_server_kind=name\n", "typed value absent entirely"),
            (
                "tmux_server_kind=socket\ntmux_server=relative/s.sock\n",
                "non-absolute socket",
            ),
            (
                "tmux_server_kind=\ntmux_server=work\n",
                "present-but-EMPTY kind beside a nonempty value",
            ),
            (
                "tmux_server=work\ntmux_server=work\n",
                "duplicate EQUAL keys — agreeing is not the same as unambiguous",
            ),
            (
                "tmux_server=work\ntmux_server=other\n",
                "duplicate CONFLICTING keys",
            ),
            (
                "tmux_server_kind=name\ntmux_server_kind=socket\ntmux_server=/tmp/s\n",
                "duplicate conflicting KIND keys",
            ),
        ] {
            assert_eq!(
                Meta::parse(text).server_selector(),
                ServerSelector::Ambiguous,
                "{why}"
            );
        }
    }

    #[test]
    fn sc_405l_an_absent_kind_and_an_empty_kind_are_different_answers() {
        // The gate requires these two fixtures to differ BY CONSTRUCTION: one
        // omits the key, the other writes it empty, and they normalize to
        // opposite sides of the entitlement line.
        let absent = Meta::parse("tmux_server=work\n");
        let empty = Meta::parse("tmux_server_kind=\ntmux_server=work\n");
        assert_eq!(
            absent.server_selector(),
            ServerSelector::Positive(Selector::Name("work".to_owned()))
        );
        assert_eq!(empty.server_selector(), ServerSelector::Ambiguous);
        assert_ne!(absent.server_selector(), empty.server_selector());
    }

    #[test]
    fn sc_405l_a_name_payload_and_a_socket_payload_keep_their_types() {
        // Flattening both to a string would let one spelling address the
        // other's server. `-L /tmp/x` and `-S /tmp/x` are not the same tmux.
        let by_name = Meta::parse("tmux_server_kind=name\ntmux_server=/tmp/x\n");
        let by_socket = Meta::parse("tmux_server_kind=socket\ntmux_server=/tmp/x\n");
        assert_eq!(
            by_name.server_selector(),
            ServerSelector::Positive(Selector::Name("/tmp/x".to_owned()))
        );
        assert_eq!(
            by_socket.server_selector(),
            ServerSelector::Positive(Selector::Socket(PathBuf::from("/tmp/x")))
        );
        assert_ne!(by_name.server_selector(), by_socket.server_selector());
    }

    #[test]
    fn sc_405l_reading_the_selector_never_loses_the_rest_of_the_meta() {
        // The family left SC-405d's catch-all; nothing else did.
        let meta = Meta::parse(
            "mode=local\ntmux_server_kind=name\ntmux_server=work\nae_path=/usr/local/bin/ae\n",
        );
        assert_eq!(meta.mode(), Some("local"));
        assert_eq!(
            meta.server_selector(),
            ServerSelector::Positive(Selector::Name("work".to_owned()))
        );
        assert_eq!(
            meta.anomalies(),
            [Anomaly::UnknownKey {
                key: "ae_path".to_owned(),
                line: 4
            }],
            "the selector keys are consumed; every other unknown key is still merely tolerated"
        );
    }

    #[test]
    fn sc_405a_a_value_may_contain_the_separator_because_the_split_is_on_the_first_equals() {
        let meta = Meta::parse("goal=ship a=b=c\n");
        assert_eq!(meta.goal(), Some("ship a=b=c"));
        assert!(meta.anomalies().is_empty());
    }

    #[test]
    fn sc_405a_a_line_with_no_equals_is_an_anomaly_not_a_key() {
        let meta = Meta::parse("mode=local\nthis is not a key value line\n");
        assert_eq!(meta.mode(), Some("local"));
        assert_eq!(meta.anomalies(), [Anomaly::MalformedLine { line: 2 }]);
    }

    #[test]
    fn sc_405a_an_empty_value_is_a_value() {
        let meta = Meta::parse("goal=\n");
        assert_eq!(meta.goal(), Some(""));
        assert!(meta.anomalies().is_empty());
    }

    #[test]
    fn sc_405b_the_four_context_keys_are_read() {
        let meta = Meta::parse(concat!(
            "mode=worktree\n",
            "origin=/home/c/projects/ae\n",
            "work_dir=/home/c/.ae/worktrees/x\n",
            "goal=ship the login flow\n",
        ));
        assert_eq!(meta.mode(), Some("worktree"));
        assert_eq!(meta.origin(), Some("/home/c/projects/ae"));
        assert_eq!(meta.work_dir(), Some("/home/c/.ae/worktrees/x"));
        assert_eq!(meta.goal(), Some("ship the login flow"));
        assert!(meta.anomalies().is_empty());
    }

    #[test]
    fn frozen_human_subline_version_is_retained_without_becoming_an_unknown_key() {
        let meta = Meta::parse("mode=local\nae_version=0.2.1\n");
        assert_eq!(meta.ae_version(), Some("0.2.1"));
        assert!(meta.anomalies().is_empty());
    }

    #[test]
    fn sc_405c_a_roster_entry_carries_alias_name_and_an_optional_session_id() {
        let meta = Meta::parse(concat!(
            "agent.main=claude:lead:e795c9e9\n",
            "agent_bin.main=claude\n",
            "agent.worker.0=codex:coworker\n",
        ));
        let roster = meta.roster();
        assert_eq!(roster.len(), 2);
        assert_eq!(roster[0].slot, "main");
        assert_eq!(roster[0].profile.as_deref(), Some("claude"));
        assert_eq!(roster[0].name, "lead");
        assert_eq!(roster[0].harness_session.as_deref(), Some("e795c9e9"));
        assert_eq!(roster[0].binary.as_deref(), Some("claude"));
        assert_eq!(roster[0].reference(), "claude:lead");
        // The slot name itself contains a dot; only the PREFIX is stripped.
        assert_eq!(roster[1].slot, "worker.0");
        assert_eq!(roster[1].harness_session, None);
        assert_eq!(roster[1].binary, None);
        assert!(meta.anomalies().is_empty());
    }

    #[test]
    fn sc_405c_the_binary_may_be_recorded_before_the_identity() {
        let meta = Meta::parse("agent_bin.main=claude\nagent.main=claude:lead\n");
        let roster = meta.roster();
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].binary.as_deref(), Some("claude"));
        assert_eq!(roster[0].name, "lead");
    }

    #[test]
    fn a_binary_with_no_identity_is_not_an_agent() {
        let meta = Meta::parse("agent_bin.worker.3=codex\n");
        assert!(meta.roster().is_empty());
        assert!(
            meta.anomalies().is_empty(),
            "not an anomaly, just not an agent"
        );
    }

    #[test]
    fn a_roster_value_the_reader_rejects_leaves_no_half_built_entry_behind() {
        // The state that made a "does this entry have a name" filter necessary
        // is the state this parser no longer produces.
        let meta = Meta::parse("agent_bin.main=claude\nagent.main=broken\n");
        assert!(meta.roster().is_empty());
        assert_eq!(meta.anomalies().len(), 1);
    }

    #[test]
    fn sc_405c_a_roster_value_that_is_not_alias_name_is_an_anomaly() {
        for (value, why) in [
            ("justanalias", "no name half"),
            ("a:b:c:d", "too many parts"),
            (":name", "empty alias"),
            ("alias:", "empty name"),
        ] {
            let meta = Meta::parse(&format!("agent.main={value}\n"));
            assert!(meta.roster().is_empty(), "{why}");
            assert_eq!(
                meta.anomalies(),
                [Anomaly::MalformedRosterEntry {
                    key: "agent.main".to_owned(),
                    line: 1
                }],
                "{why}"
            );
        }
    }

    #[test]
    fn sc_405d_an_unknown_key_is_recorded_rather_than_interpreted_or_ignored() {
        // UNCLASSIFIED: the parser refuses to decide, and says so.
        let meta = Meta::parse("mode=local\nae_path=/usr/local/bin/ae\nwatchdog=1234\n");
        assert_eq!(meta.mode(), Some("local"));
        assert_eq!(
            meta.anomalies(),
            [
                Anomaly::UnknownKey {
                    key: "ae_path".to_owned(),
                    line: 2
                },
                Anomaly::UnknownKey {
                    key: "watchdog".to_owned(),
                    line: 3
                }
            ]
        );
    }

    /// Identity v2 (P1). The tracer that preceded this test pinned the v1
    /// parser's answer to these keys — SC-405d-unknown, EMPTY roster, i.e. fail
    /// closed (git history: `identity_v2_keys_are_unknown_to_the_v1_parser_...`).
    /// The v2 parser reads them as a seat: the NAME is the identity, the profile
    /// and the harness session are metadata, and the display ref is the bare name.
    #[test]
    fn identity_v2_a_seat_carries_name_profile_and_harness_session() {
        let meta = Meta::parse(concat!(
            "schema=2\n",
            "seat.main=lead\n",
            "profile.main=fable5\n",
            "harness_session.main=e795c9e9\n",
            "agent_bin.main=claude\n",
        ));
        assert_eq!(meta.schema(), Some("2"));
        let roster = meta.roster();
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].slot, "main");
        assert_eq!(roster[0].name, "lead");
        assert_eq!(roster[0].profile.as_deref(), Some("fable5"));
        assert_eq!(roster[0].harness_session.as_deref(), Some("e795c9e9"));
        assert_eq!(roster[0].binary.as_deref(), Some("claude"));
        assert_eq!(roster[0].schema, RosterSchema::V2);
        assert_eq!(
            roster[0].reference(),
            "lead",
            "v2 display ref is the name alone"
        );
        assert!(
            meta.anomalies().is_empty(),
            "every v2 key is read, none is unknown"
        );
    }

    #[test]
    fn identity_v2_metadata_rows_may_precede_their_seat() {
        // Same rule as `agent_bin`: profile/harness wait for the seat, never
        // create a half-built entry, and a slot that never gets a seat is not an agent.
        let meta = Meta::parse(concat!(
            "profile.worker.0=gpt56sol\n",
            "harness_session.worker.0=abc\n",
            "agent_bin.worker.0=codex\n",
            "seat.worker.0=colead\n",
            "profile.worker.9=orphan\n",
        ));
        let roster = meta.roster();
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].slot, "worker.0");
        assert_eq!(roster[0].name, "colead");
        assert_eq!(roster[0].profile.as_deref(), Some("gpt56sol"));
        assert_eq!(roster[0].harness_session.as_deref(), Some("abc"));
        assert_eq!(roster[0].binary.as_deref(), Some("codex"));
        assert!(
            meta.anomalies().is_empty(),
            "an orphan profile row is not an agent and not an anomaly"
        );
    }

    #[test]
    fn identity_v2_a_seat_without_a_profile_is_still_an_agent() {
        let meta = Meta::parse("seat.spawned.3=builder\n");
        let roster = meta.roster();
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].name, "builder");
        assert_eq!(
            roster[0].profile, None,
            "metadata is optional; identity is not"
        );
        assert_eq!(roster[0].reference(), "builder");
    }

    #[test]
    fn identity_v2_an_empty_seat_name_is_a_malformed_roster_entry() {
        let meta = Meta::parse("seat.main=\nprofile.main=fable5\n");
        assert!(meta.roster().is_empty());
        assert_eq!(
            meta.anomalies(),
            [Anomaly::MalformedRosterEntry {
                key: "seat.main".to_owned(),
                line: 1
            }]
        );
    }

    #[test]
    fn identity_v2_a_slot_claimed_by_both_schemas_contributes_no_agent() {
        // Either order: the second claim drops the slot and pins the anomaly, and
        // a third claim cannot resurrect it. SC-405k — membership is roster-
        // defined, so an identity in doubt is no identity.
        for (text, second_line) in [
            ("agent.main=fable5:lead\nseat.main=lead\n", 2),
            ("seat.main=lead\nagent.main=fable5:lead\n", 2),
        ] {
            let meta = Meta::parse(text);
            assert!(meta.roster().is_empty(), "{text:?}");
            assert_eq!(
                meta.anomalies(),
                [Anomaly::MixedSchemaSlot {
                    slot: "main".to_owned(),
                    line: second_line
                }],
                "{text:?}"
            );
        }
        let meta = Meta::parse("agent.main=fable5:lead\nseat.main=lead\nseat.main=lead2\n");
        assert!(
            meta.roster().is_empty(),
            "no resurrection after a mixed claim"
        );
        // The third line is BOTH a duplicate key (SC-405e) and a mixed claim; the
        // duplicate check runs first and refuses to absorb it at all.
        assert!(
            meta.anomalies()
                .iter()
                .any(|a| matches!(a, Anomaly::MixedSchemaSlot { .. }))
        );
        assert!(
            meta.anomalies()
                .iter()
                .any(|a| matches!(a, Anomaly::DuplicateKey { .. }))
        );
        // Other slots are untouched by one slot's doubt.
        let meta = Meta::parse("agent.main=fable5:lead\nseat.main=lead\nseat.worker.0=colead\n");
        assert_eq!(meta.roster().len(), 1);
        assert_eq!(meta.roster()[0].name, "colead");
    }

    #[test]
    fn identity_v2_duplicate_metadata_keys_invalidate_only_that_field() {
        let meta = Meta::parse(concat!(
            "seat.main=lead\n",
            "profile.main=fable5\n",
            "profile.main=opus5\n",
            "harness_session.main=one\n",
        ));
        let roster = meta.roster();
        assert_eq!(roster.len(), 1, "the seat survives a duplicate profile row");
        assert_eq!(
            roster[0].profile, None,
            "neither profile value is published"
        );
        assert_eq!(roster[0].harness_session.as_deref(), Some("one"));
        // A duplicate SEAT is an identity in doubt: no agent, like a duplicate agent.<slot>.
        let meta = Meta::parse("seat.main=lead\nseat.main=lead\n");
        assert!(meta.roster().is_empty());
    }

    #[test]
    fn identity_v1_rows_keep_their_alias_name_display_ref() {
        // Pre-SC-511a ledgers pair on the display string; a reader that changed
        // it would unpair every legacy state event. v1 stays v1 on the read side.
        let meta = Meta::parse("agent.main=fable5:lead:sid\n");
        assert_eq!(meta.roster()[0].schema, RosterSchema::V1);
        assert_eq!(meta.roster()[0].profile.as_deref(), Some("fable5"));
        assert_eq!(meta.roster()[0].reference(), "fable5:lead");
        assert_eq!(meta.schema(), None, "no schema row means v1");
    }

    /// Colead P1 gate BLOCKER-1: v2 metadata rows must never rewrite a v1
    /// identity, in EITHER order. The one rule: on a v1 (or mixed) slot they are
    /// SC-405d unknown keys — exactly what a v1 parser would have called them.
    #[test]
    fn identity_v1_ref_and_sid_are_byte_stable_under_v2_metadata_in_both_orders() {
        for text in [
            "agent.main=fable5:lead:sid\nprofile.main=opus5\nharness_session.main=other\n",
            "profile.main=opus5\nharness_session.main=other\nagent.main=fable5:lead:sid\n",
            "profile.main=opus5\nagent.main=fable5:lead:sid\nharness_session.main=other\n",
        ] {
            let meta = Meta::parse(text);
            let roster = meta.roster();
            assert_eq!(roster.len(), 1, "{text:?}");
            assert_eq!(roster[0].schema, RosterSchema::V1, "{text:?}");
            assert_eq!(roster[0].reference(), "fable5:lead", "{text:?}");
            assert_eq!(roster[0].profile.as_deref(), Some("fable5"), "{text:?}");
            assert_eq!(
                roster[0].harness_session.as_deref(),
                Some("sid"),
                "{text:?}"
            );
            let unknown: Vec<&str> = meta
                .anomalies()
                .iter()
                .filter_map(|a| match a {
                    Anomaly::UnknownKey { key, .. } => Some(key.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(
                unknown,
                ["profile.main", "harness_session.main"],
                "the rows are recorded, never interpreted: {text:?}"
            );
            assert_eq!(meta.anomalies().len(), 2, "{text:?}");
        }
        // A DUPLICATE profile row on a v1 slot invalidates nothing of the v1
        // identity: its alias is not a profile row's to take away.
        let meta = Meta::parse("agent.main=fable5:lead:sid\nprofile.main=x\nprofile.main=y\n");
        assert_eq!(meta.roster()[0].profile.as_deref(), Some("fable5"));
        assert_eq!(meta.roster()[0].harness_session.as_deref(), Some("sid"));
        // A v1 KEY whose value was malformed still claims the slot for v1: a
        // later profile row is unknown, not pending-for-a-seat-that-never-comes.
        let meta = Meta::parse("agent.main=broken\nprofile.main=x\n");
        assert!(meta.roster().is_empty());
        assert!(
            meta.anomalies()
                .iter()
                .any(|a| matches!(a, Anomaly::UnknownKey { key, .. } if key == "profile.main"))
        );
    }

    /// Colead P1 gate BLOCKER-2: mixed-schema detection is by raw KEY claim,
    /// so a malformed, bare or duplicated FIRST claim (which leaves no entry)
    /// cannot let the other schema resurrect the slot — in both orders.
    #[test]
    fn identity_v2_a_slot_claimed_by_both_keys_is_dropped_whatever_the_first_claim_was() {
        let cases = [
            // malformed first
            "agent.main=broken\nseat.main=lead\n",
            "seat.main=\nagent.main=fable5:lead\n",
            // bare first (no `=`: a MalformedLine, but still a claim)
            "agent.main\nseat.main=lead\n",
            "seat.main\nagent.main=fable5:lead\n",
            // duplicate first (the duplicate invalidated the entry)
            "seat.main=lead\nseat.main=other\nagent.main=fable5:lead\n",
            "agent.main=fable5:lead\nagent.main=x:y\nseat.main=lead\n",
            // valid first
            "agent.main=fable5:lead\nseat.main=lead\n",
            "seat.main=lead\nagent.main=fable5:lead\n",
            // both bare
            "agent.main\nseat.main\n",
        ];
        for text in cases {
            let meta = Meta::parse(text);
            assert!(meta.roster().is_empty(), "resurrected: {text:?}");
            assert!(
                meta.anomalies()
                    .iter()
                    .any(|a| matches!(a, Anomaly::MixedSchemaSlot { slot, .. } if slot == "main")),
                "no mixed anomaly: {text:?} → {:?}",
                meta.anomalies()
            );
        }
        // Once mixed, a further claim of either prefix is refused with its own
        // anomaly and never recreates the slot.
        let meta = Meta::parse(
            "agent.main=fable5:lead\nseat.main=lead\nseat.spawned.1=b\nagent.main=z:q\n",
        );
        assert_eq!(meta.roster().len(), 1);
        assert_eq!(meta.roster()[0].name, "b", "other slots are untouched");
        assert_eq!(
            meta.anomalies()
                .iter()
                .filter(|a| matches!(a, Anomaly::MixedSchemaSlot { .. }))
                .count(),
            2,
            "the line that made it mixed and the later refused claim"
        );
        // Pending metadata for a slot that turns out mixed is uninterpreted.
        let meta = Meta::parse("profile.main=x\nagent.main=fable5:lead\nseat.main=lead\n");
        assert!(meta.roster().is_empty());
        assert!(
            meta.anomalies()
                .iter()
                .any(|a| matches!(a, Anomaly::UnknownKey { key, .. } if key == "profile.main"))
        );
    }

    /// Colead P1 gate IMPORTANT-1: under v2 the NAME is the identity, so one
    /// name on two seats is one identity in doubt — every v2 seat carrying it
    /// is dropped, in both orders; a v1 row keeps its `alias:name` ref.
    #[test]
    fn identity_v2_a_name_on_two_seats_is_dropped_from_both_in_both_orders() {
        for text in [
            "seat.main=lead\nseat.worker.0=lead\n",
            "seat.worker.0=lead\nseat.main=lead\n",
        ] {
            let meta = Meta::parse(text);
            assert!(meta.roster().is_empty(), "{text:?}");
            assert_eq!(
                meta.anomalies(),
                [Anomaly::DuplicateName {
                    name: "lead".to_owned(),
                    line: 2
                }],
                "{text:?}"
            );
        }
        // A third seat with the doubtful name is dropped too; distinct names
        // beside it survive.
        let meta = Meta::parse(
            "seat.main=lead\nseat.worker.0=lead\nseat.spawned.0=lead\nseat.spawned.1=builder\n",
        );
        assert_eq!(meta.roster().len(), 1);
        assert_eq!(meta.roster()[0].name, "builder");
        assert_eq!(
            meta.anomalies()
                .iter()
                .filter(|a| matches!(a, Anomaly::DuplicateName { .. }))
                .count(),
            2
        );
        // v1 beside v2 with one name: the v1 row stays (its identity is the
        // pair `cl:lead`), the v2 seat is dropped — both orders.
        for text in [
            "agent.main=cl:lead:sid\nseat.worker.0=lead\n",
            "seat.worker.0=lead\nagent.main=cl:lead:sid\n",
        ] {
            let meta = Meta::parse(text);
            assert_eq!(meta.roster().len(), 1, "{text:?}");
            assert_eq!(meta.roster()[0].reference(), "cl:lead", "{text:?}");
            assert_eq!(meta.roster()[0].schema, RosterSchema::V1, "{text:?}");
            assert!(
                meta.anomalies()
                    .iter()
                    .any(|a| matches!(a, Anomaly::DuplicateName { name, .. } if name == "lead")),
                "{text:?}"
            );
        }
        // CONTROLS — the check is not over-strong: distinct names, a single
        // seat, and two v1 rows sharing a name (two refs, frozen behaviour).
        let meta = Meta::parse("seat.main=lead\nseat.worker.0=colead\n");
        assert_eq!(meta.roster().len(), 2);
        assert!(meta.anomalies().is_empty());
        let meta = Meta::parse("seat.main=lead\n");
        assert_eq!(meta.roster().len(), 1);
        assert!(meta.anomalies().is_empty());
        let meta = Meta::parse("agent.main=a:lead\nagent.worker.0=b:lead\n");
        assert_eq!(meta.roster().len(), 2);
        assert!(
            meta.anomalies().is_empty(),
            "v1 duplicate names are two refs"
        );
    }

    /// Colead P1 round-2 IMPORTANT-1: metadata that ATTACHED to a v2 seat is
    /// reclassified as unknown when the slot turns out mixed — the answer the
    /// other order already gave.
    #[test]
    fn identity_v2_attached_metadata_is_uninterpreted_when_the_slot_turns_mixed() {
        for text in [
            "seat.main=lead\nprofile.main=x\nagent.main=a:lead\n",
            "seat.main=lead\nharness_session.main=sid\nagent.main=a:lead\n",
            "agent.main=a:lead\nprofile.main=x\nseat.main=lead\n",
            "agent.main=a:lead\nharness_session.main=sid\nseat.main=lead\n",
            "profile.main=x\nseat.main=lead\nagent.main=a:lead\n",
        ] {
            let meta = Meta::parse(text);
            assert!(meta.roster().is_empty(), "{text:?}");
            let unknown = meta
                .anomalies()
                .iter()
                .filter(|a| {
                    matches!(a, Anomaly::UnknownKey { key, .. }
                        if key == "profile.main" || key == "harness_session.main")
                })
                .count();
            assert_eq!(
                unknown,
                1,
                "the metadata row is unknown whichever order: {text:?} → {:?}",
                meta.anomalies()
            );
            assert!(
                meta.anomalies()
                    .iter()
                    .any(|a| matches!(a, Anomaly::MixedSchemaSlot { .. })),
                "{text:?}"
            );
        }
    }

    /// Colead P1 round-2 IMPORTANT-2: an EMPTY metadata row is judged by the
    /// slot's schema first — unknown on v1 (both orders), absent metadata on v2.
    #[test]
    fn identity_v2_empty_metadata_rows_are_judged_by_schema_first() {
        for text in [
            "agent.main=a:lead:sid\nprofile.main=\n",
            "profile.main=\nagent.main=a:lead:sid\n",
            "agent.main=a:lead:sid\nharness_session.main=\n",
            "harness_session.main=\nagent.main=a:lead:sid\n",
        ] {
            let meta = Meta::parse(text);
            assert_eq!(meta.roster().len(), 1, "{text:?}");
            assert_eq!(meta.roster()[0].reference(), "a:lead", "{text:?}");
            assert_eq!(
                meta.roster()[0].harness_session.as_deref(),
                Some("sid"),
                "{text:?}"
            );
            assert_eq!(
                meta.anomalies()
                    .iter()
                    .filter(|a| matches!(a, Anomaly::UnknownKey { .. }))
                    .count(),
                1,
                "{text:?} → {:?}",
                meta.anomalies()
            );
        }
        for text in [
            "seat.main=lead\nprofile.main=\n",
            "profile.main=\nseat.main=lead\n",
            "seat.main=lead\nharness_session.main=\n",
            "harness_session.main=\nseat.main=lead\n",
        ] {
            let meta = Meta::parse(text);
            assert_eq!(meta.roster().len(), 1, "{text:?}");
            assert_eq!(meta.roster()[0].profile, None, "{text:?}");
            assert_eq!(meta.roster()[0].harness_session, None, "{text:?}");
            assert!(
                meta.anomalies().is_empty(),
                "{text:?} → {:?}",
                meta.anomalies()
            );
        }
    }

    /// Colead P1 round-4 IMPORTANT-1: v2 metadata classification is decided
    /// from RAW v1 claims, so a metadata row's anomaly KIND is the same in both
    /// orders — even when the v1 claim is MALFORMED or BARE, and even across a
    /// duplicate-metadata-then-mixed sequence.
    #[test]
    fn identity_v2_metadata_is_order_independent_across_malformed_bare_and_duplicate_v1_claims() {
        // A malformed or bare v1 claim reclassifies pending metadata the same
        // as a valid one — both orders yield {UnknownKey, <the v1 refusal>}.
        let kinds = |meta: &Meta| -> (usize, usize, usize, usize) {
            let mut unknown = 0;
            let mut malformed_entry = 0;
            let mut malformed_line = 0;
            let mut mixed = 0;
            for a in meta.anomalies() {
                match a {
                    Anomaly::UnknownKey { .. } => unknown += 1,
                    Anomaly::MalformedRosterEntry { .. } => malformed_entry += 1,
                    Anomaly::MalformedLine { .. } => malformed_line += 1,
                    Anomaly::MixedSchemaSlot { .. } => mixed += 1,
                    _ => {}
                }
            }
            (unknown, malformed_entry, malformed_line, mixed)
        };
        for field in ["profile", "harness_session"] {
            // malformed v1 value, both orders → 1 UnknownKey + 1 MalformedRosterEntry.
            let a = Meta::parse(&format!("{field}.main=x\nagent.main=broken\n"));
            let b = Meta::parse(&format!("agent.main=broken\n{field}.main=x\n"));
            assert!(a.roster().is_empty() && b.roster().is_empty(), "{field}");
            assert_eq!(kinds(&a), (1, 1, 0, 0), "{field} metadata-first malformed");
            assert_eq!(kinds(&b), (1, 1, 0, 0), "{field} v1-first malformed");
            // bare v1 line, both orders → 1 UnknownKey + 1 MalformedLine.
            let a = Meta::parse(&format!("{field}.main=x\nagent.main\n"));
            let b = Meta::parse(&format!("agent.main\n{field}.main=x\n"));
            assert_eq!(kinds(&a), (1, 0, 1, 0), "{field} metadata-first bare");
            assert_eq!(kinds(&b), (1, 0, 1, 0), "{field} v1-first bare");
        }
        // Duplicate metadata, THEN the slot goes mixed: both orders carry an
        // UnknownKey (the first metadata row) as well as the DuplicateKey and
        // the MixedSchemaSlot — the duplicate no longer erases the provenance.
        let has = |meta: &Meta, f: &dyn Fn(&Anomaly) -> bool| meta.anomalies().iter().any(f);
        for text in [
            "profile.main=x\nprofile.main=y\nagent.main=a:lead\nseat.main=lead\n",
            "agent.main=a:lead\nprofile.main=x\nprofile.main=y\nseat.main=lead\n",
            "harness_session.main=x\nharness_session.main=y\nagent.main=a:lead\nseat.main=lead\n",
        ] {
            let meta = Meta::parse(text);
            assert!(meta.roster().is_empty(), "{text:?}");
            assert!(
                has(&meta, &|a| matches!(a, Anomaly::UnknownKey { .. })),
                "no UnknownKey: {text:?} → {:?}",
                meta.anomalies()
            );
            assert!(
                has(&meta, &|a| matches!(a, Anomaly::DuplicateKey { .. })),
                "no DuplicateKey: {text:?}"
            );
            assert!(
                has(&meta, &|a| matches!(a, Anomaly::MixedSchemaSlot { .. })),
                "no Mixed: {text:?}"
            );
        }
        // CONTROL — a duplicate metadata key on a genuine v2 seat invalidates
        // only the value; no UnknownKey, the seat survives.
        let meta = Meta::parse("seat.main=lead\nprofile.main=x\nprofile.main=y\n");
        assert_eq!(meta.roster().len(), 1);
        assert_eq!(meta.roster()[0].profile, None);
        assert!(!has(&meta, &|a| matches!(a, Anomaly::UnknownKey { .. })));
    }

    /// Colead P1 round-2 IMPORTANT-3: a bare `seat.<slot>` beside a keyed one
    /// is a repeated v2 claim — the slot stays absent in both orders, as the
    /// archive reader (which counts lines) already refuses it. A v1 bare row
    /// keeps its frozen behaviour: a malformed line, and the keyed row names
    /// the agent.
    #[test]
    fn identity_v2_a_bare_seat_row_beside_a_keyed_one_is_a_repeated_claim() {
        for text in ["seat.main\nseat.main=lead\n", "seat.main=lead\nseat.main\n"] {
            let meta = Meta::parse(text);
            assert!(meta.roster().is_empty(), "{text:?}");
            assert!(
                meta.anomalies()
                    .iter()
                    .any(|a| matches!(a, Anomaly::DuplicateKey { key, .. } if key == "seat.main")),
                "{text:?} → {:?}",
                meta.anomalies()
            );
            assert!(
                meta.anomalies()
                    .iter()
                    .any(|a| matches!(a, Anomaly::MalformedLine { .. })),
                "{text:?}"
            );
        }
        // Two bare rows: absent as well, one DuplicateKey.
        let meta = Meta::parse("seat.main\nseat.main\n");
        assert!(meta.roster().is_empty());
        assert_eq!(
            meta.anomalies()
                .iter()
                .filter(|a| matches!(a, Anomaly::DuplicateKey { .. }))
                .count(),
            1
        );
        // A keyed repeat is recorded ONCE (by the duplicate-key check).
        let meta = Meta::parse("seat.main=lead\nseat.main=other\n");
        assert!(meta.roster().is_empty());
        assert_eq!(
            meta.anomalies(),
            [Anomaly::DuplicateKey {
                key: "seat.main".to_owned(),
                line: 2
            }]
        );
        // v1 control — frozen.
        for text in [
            "agent.main\nagent.main=a:lead\n",
            "agent.main=a:lead\nagent.main\n",
        ] {
            let meta = Meta::parse(text);
            assert_eq!(meta.roster().len(), 1, "{text:?}");
            assert_eq!(meta.roster()[0].reference(), "a:lead", "{text:?}");
            assert!(
                meta.anomalies()
                    .iter()
                    .all(|a| matches!(a, Anomaly::MalformedLine { .. })),
                "{text:?} → {:?}",
                meta.anomalies()
            );
        }
    }

    #[test]
    fn sc_405e_a_duplicate_key_invalidates_the_field_rather_than_picking_one() {
        // Precedence is UNCLASSIFIED. Publishing "first" would put a value in
        // the digest that no row says is the value — the reader would be
        // inventing an answer to a question the seats have not settled.
        let meta = Meta::parse("goal=first\ngoal=second\n");
        assert_eq!(meta.goal(), None, "neither occurrence is published");
        assert_eq!(
            meta.anomalies(),
            [Anomaly::DuplicateKey {
                key: "goal".to_owned(),
                line: 2
            }]
        );
    }

    #[test]
    fn sc_405e_invalidation_is_per_field_and_leaves_the_rest_intact() {
        let meta = Meta::parse("mode=local\ngoal=first\ngoal=second\norigin=/src\n");
        assert_eq!(meta.goal(), None);
        assert_eq!(meta.mode(), Some("local"), "an untouched key is untouched");
        assert_eq!(meta.origin(), Some("/src"));
    }

    #[test]
    fn sc_405e_every_retained_scalar_key_invalidates_the_same_way() {
        // One field tested is one field proven. The four SC-405b keys and the
        // frozen human-subline version all go through the same refusal.
        type Accessor = fn(&Meta) -> Option<&str>;
        let read: [(&str, Accessor); 5] = [
            ("mode", Meta::mode),
            ("origin", Meta::origin),
            ("work_dir", Meta::work_dir),
            ("goal", Meta::goal),
            ("ae_version", Meta::ae_version),
        ];
        for (key, accessor) in read {
            let once = Meta::parse(&format!("{key}=only\n"));
            assert_eq!(accessor(&once), Some("only"), "{key} reads when named once");

            let twice = Meta::parse(&format!("{key}=first\n{key}=second\n"));
            assert_eq!(accessor(&twice), None, "{key} is invalidated when doubled");
            assert_eq!(
                twice.anomalies(),
                [Anomaly::DuplicateKey {
                    key: key.to_owned(),
                    line: 2
                }],
                "{key}"
            );
        }
    }

    #[test]
    fn sc_405e_a_doubly_named_slot_contributes_no_agent() {
        // SC-405k: membership is roster-defined, so a slot whose identity is in
        // doubt supplies no agent rather than a guessed one.
        let meta = Meta::parse(concat!(
            "agent.main=claude:lead\n",
            "agent.main=codex:someone-else\n",
            "agent.worker.0=codex:coworker\n",
        ));
        assert_eq!(
            meta.roster()
                .iter()
                .map(|e| e.slot.as_str())
                .collect::<Vec<_>>(),
            ["worker.0"],
            "the doubled slot is gone, the sound one stays"
        );
        assert_eq!(meta.anomalies().len(), 1);
    }

    #[test]
    fn sc_405e_a_doubled_binary_leaves_the_agent_without_one() {
        let meta = Meta::parse(concat!(
            "agent.main=claude:lead\n",
            "agent_bin.main=claude\n",
            "agent_bin.main=codex\n",
        ));
        let roster = meta.roster();
        assert_eq!(roster.len(), 1, "the identity is not in doubt");
        assert_eq!(roster[0].binary, None, "but which binary is");
    }

    #[test]
    fn sc_405e_a_doubled_binary_seen_before_its_identity_is_dropped_too() {
        let meta = Meta::parse(concat!(
            "agent_bin.main=claude\n",
            "agent_bin.main=codex\n",
            "agent.main=claude:lead\n",
        ));
        let roster = meta.roster();
        assert_eq!(roster.len(), 1);
        assert_eq!(
            roster[0].binary, None,
            "the pending value is invalidated too"
        );
    }

    #[test]
    fn blank_lines_and_a_missing_final_newline_are_both_ordinary() {
        let meta = Meta::parse("mode=local\n\n\ngoal=x");
        assert_eq!(meta.mode(), Some("local"));
        assert_eq!(meta.goal(), Some("x"));
        assert!(meta.anomalies().is_empty());
    }

    #[test]
    fn a_carriage_return_is_line_ending_not_value() {
        let meta = Meta::parse("work_dir=/tmp/x\r\nmode=local\r\n");
        assert_eq!(meta.work_dir(), Some("/tmp/x"));
        assert_eq!(meta.mode(), Some("local"));
    }

    #[test]
    fn an_empty_meta_yields_nothing_and_complains_about_nothing() {
        let meta = Meta::parse("");
        assert_eq!(meta.mode(), None);
        assert!(meta.roster().is_empty());
        assert!(meta.anomalies().is_empty());
    }

    #[test]
    fn anomalies_present_their_line_so_a_report_can_point_at_it() {
        assert_eq!(
            Anomaly::UnknownKey {
                key: "tmux_server".to_owned(),
                line: 7
            }
            .to_string(),
            "unknown meta key tmux_server at line 7"
        );
        assert_eq!(
            Anomaly::MalformedLine { line: 3 }.to_string(),
            "malformed meta line 3"
        );
        assert_eq!(
            Anomaly::DuplicateKey {
                key: "goal".to_owned(),
                line: 9
            }
            .to_string(),
            "duplicate meta key goal at line 9"
        );
        assert_eq!(
            Anomaly::MalformedRosterEntry {
                key: "agent.main".to_owned(),
                line: 2
            }
            .to_string(),
            "malformed roster entry agent.main at line 2"
        );
    }

    #[test]
    fn a_sole_value_is_the_one_a_key_names_exactly_once() {
        use super::{first_value, sole_value};
        // The contrast with `first_value` IS the point, so both are asserted on
        // the same inputs: `first_value` answers "what does the first record
        // say", which is right where a later duplicate is harmless; this
        // answers "does the file say ONE thing", which is what a flag guarding
        // behavior needs.
        let once = b"session=x\nmeta_agent=true\n";
        assert_eq!(sole_value(once, "meta_agent"), Some(b"true".as_slice()));
        assert_eq!(first_value(once, "meta_agent"), Some(b"true".as_slice()));

        for doubled in [
            &b"meta_agent=true\nmeta_agent=false\n"[..],
            &b"meta_agent=true\nmeta_agent=true\n"[..],
            &b"meta_agent=false\nsession=x\nmeta_agent=true\n"[..],
        ] {
            assert_eq!(
                sole_value(doubled, "meta_agent"),
                None,
                "a key named twice does not say one thing"
            );
            assert!(
                first_value(doubled, "meta_agent").is_some(),
                "the control: `first_value` still answers, which is exactly why \
                 a flag must not be read with it"
            );
        }

        assert_eq!(sole_value(b"session=x\n", "meta_agent"), None, "absent");
        // An empty value is a value: present once, and it is the empty string.
        assert_eq!(sole_value(b"meta_agent=\n", "meta_agent"), Some(&b""[..]));
        // The key match is a whole prefix up to `=`, not a substring.
        assert_eq!(sole_value(b"not_meta_agent=true\n", "meta_agent"), None);
        assert_eq!(sole_value(b"meta_agent_x=true\n", "meta_agent"), None);
    }
}
