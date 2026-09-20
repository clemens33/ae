//! The session `meta` file, and nothing else.
//!
//! * `key=value`, split on the FIRST equals; values are single-line.
//! * `mode`, `origin`, `work_dir`, `goal` are meta keys.
//! * `agent.<slot>` carries `alias:name:provider-session-id`
//!   (the session id is optional) and
//!   `agent_bin.<slot>` the recorded binary.
//!
//! * every OTHER key is tolerated silently and never degrades.
//!   Unknown keys are the normal state of a real meta, so degrading on them
//!   would make the flag constant-true. They are still recorded as an
//!   [`Anomaly`], because seeing them costs nothing and a future tool may want
//!   them, and no list of them lives here to go stale.
//! * a malformed line, a malformed roster value or a DUPLICATE
//!   key is different: the reader could not take a value the writer meant to
//!   give. Those degrade, and a duplicated key INVALIDATES its field
//!   rather than publishing an occurrence, because precedence is still
//!   unclassified and picking one would be fabricating the answer.
//!
//! Two fields that look like meta keys and are not: `goal_set_epoch` is derived
//! from the latest goal EVENT and `branch` is a live tmux/git fact
//! . Neither is read here.

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The file this module reads. The NAME is [`crate::store::META`] — one
/// spelling for the data file and the lock beside it, so neither can move
/// without the other.
pub use crate::store::META as FILE;

/// The roster key prefixes.
const ROSTER_PREFIX: &str = "agent.";
/// Two selector keys, matched literally where they are absorbed.
const SERVER_KEY: &str = "tmux_server";
const SERVER_KIND_KEY: &str = "tmux_server_kind";
const ROSTER_BIN_PREFIX: &str = "agent_bin.";
/// Identity schema v2 (alias-free): the seat's NAME, execution PROFILE,
/// harness conversation id and pinned config home live under named keys
/// instead of one `alias:name:sid` value.
const SEAT_PREFIX: &str = "seat.";
const PROFILE_PREFIX: &str = "profile.";
/// The recorded client-override label (`client.<slot>`), written only when a
/// launch honored a `profile@client` selection. Absent means no override was
/// recorded — never a derived value.
const CLIENT_PREFIX: &str = "client.";
const HARNESS_SESSION_PREFIX: &str = "harness_session.";
/// The durable PREDECESSOR row: the harness conversation ids this seat has
/// ABANDONED, oldest first — both writers leave the id they replace here.
pub const HARNESS_SESSION_PRIOR_PREFIX: &str = "harness_session_prior.";
/// How many predecessor ids one predecessor row carries.
pub const PRIOR_MAX: usize = 4;
/// The longest a predecessor's tool tag may be.
///
/// A tag is a recorded `agent_bin` — a BASENAME, and every real one is under
/// ten bytes. The bound exists so a hand-edited row cannot make the grammar
/// spend a page on one element.
const PRIOR_TOOL_MAX: usize = 64;
/// What separates a predecessor's tool tag from its conversation id.
const PRIOR_TAG: char = ':';
const CONFIG_HOME_PREFIX: &str = "config_home.";
const CONFIG_HOME_BASE_PREFIX: &str = "config_home_base.";
/// A seat's durable working directory (`work_dir.<slot>`): the canonical
/// absolute path the seat starts in. Absence inherits the session `work_dir`;
/// a present row is trusted only when sole, UTF-8, absolute and control-free.
const SEAT_WORK_DIR_PREFIX: &str = "work_dir.";
/// The defects a seat-dir row can carry — the parenthesised detail in the ONE
/// refusal every operational consumer propagates verbatim.
const SEAT_WORK_DIR_EMPTY: &str = "empty value";
const SEAT_WORK_DIR_RELATIVE: &str = "not an absolute path";
const SEAT_WORK_DIR_CONTROL: &str = "control characters";
const SEAT_WORK_DIR_DUPLICATED: &str = "named more than once";
const SEAT_WORK_DIR_NO_VALUE: &str = "no value";
const SEAT_WORK_DIR_NON_UTF8: &str = "not UTF-8";
/// The observed-model pair, as `key<slot>` — the model a live seat actually
/// ran (`observed_model.`) and the profile's own model flag value at the time
/// it was observed (`observed_model_pin.`). The two rows are ONE identity:
/// written together, cleared together. A tool whose model ae cannot observe
/// writes neither.
pub const OBSERVED_MODEL_PREFIX: &str = "observed_model.";
pub const OBSERVED_MODEL_PIN_PREFIX: &str = "observed_model_pin.";
/// `schema=<n>` — the identity schema the writer used.
const SCHEMA_KEY: &str = "schema";
/// The VERSION of the core a session is pinned to, and the shape its meta is
/// written in. Both are READ rather than tolerated: `ae list` marks a session
/// left behind by an upgrade, and it can only do that if it knows these two.
///
/// The version, never the `ae_core` PATH. A path comparison looked equivalent
/// and was not: the installer records `$HOME/.ae/versions/…` as written, while
/// `resolved_exe` canonicalizes, so on macOS every session read as behind
/// because `/tmp` and `/private/tmp` are the same directory spelled twice.
const CORE_KEY: &str = "ae_core_version";
/// Session lifecycle epochs, written by the launch metadata owner.
const CREATED_KEY: &str = "created";
const STARTED_KEY: &str = "started";
const LAUNCH_TIME_PREFIX: &str = "launch_time.";

/// One agent, as the roster records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterEntry {
    /// `main` / `worker.<n>` / `spawned.<n>` — the key's suffix.
    pub slot: String,
    /// The agent's NAME — its identity.
    pub name: String,
    /// The execution profile (`profile.<slot>`).
    pub profile: Option<String>,
    /// The recorded client override (`client.<slot>`): which `[clients]` label
    /// a launch selection pinned this seat to, if any.
    pub client: RecordedClient,
    /// The harness's own conversation id (`harness_session.<slot>`), where the
    /// roster carries one.
    pub harness_session: Option<String>,
    /// `config_home.<slot>` — the conversation store pinned at first start.
    pub config_home: RecordedConfigHome,
    /// `config_home_base.<slot>` — the effective `HOME` that selected an
    /// implicit config home.
    pub config_home_base: RecordedConfigHomeBase,
    /// `agent_bin.<slot>` — the recorded binary, where the meta carries one.
    pub binary: Option<String>,
    /// `work_dir.<slot>` — the seat's durable working directory.
    pub work_dir: RecordedWorkDir,
}

/// What a seat's optional `work_dir.<slot>` row says.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RecordedWorkDir {
    /// No row recorded: the seat inherits the session `work_dir`.
    #[default]
    Missing,
    /// A sole canonical absolute path, control-free.
    Path(PathBuf),
    /// A present row no value may be trusted from, with the defect the
    /// evidence named — the detail [`resolve_seat_dir`]'s refusal carries.
    Invalid(&'static str),
}

impl RecordedWorkDir {
    pub(crate) fn parse(value: &str) -> Self {
        if value.is_empty() {
            Self::Invalid(SEAT_WORK_DIR_EMPTY)
        } else if !Path::new(value).is_absolute() {
            Self::Invalid(SEAT_WORK_DIR_RELATIVE)
        } else if value.chars().any(char::is_control) {
            Self::Invalid(SEAT_WORK_DIR_CONTROL)
        } else {
            Self::Path(PathBuf::from(value))
        }
    }
}

/// What a seat's optional `config_home.<slot>` row says.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RecordedConfigHome {
    /// Legacy meta: no row has been recorded yet.
    #[default]
    Missing,
    /// A canonical absolute config-home selected by the tool-specific variable.
    Path(PathBuf),
    /// A canonical absolute config-home derived from an unset variable and HOME.
    Implicit(PathBuf),
    /// The first-start environment exposed no effective config home.
    Absent,
    /// The first-start environment could not be resolved safely.
    Unknown,
    /// A malformed or duplicated row: no value may be trusted.
    Invalid,
}

impl RecordedConfigHome {
    /// The value emitted when this state belongs in a meta row.
    #[must_use]
    pub fn record_value(&self) -> Option<String> {
        match self {
            Self::Missing | Self::Invalid => None,
            Self::Path(path) => Some(path.display().to_string()),
            Self::Implicit(path) => Some(format!("implicit:{}", path.display())),
            Self::Absent => Some("absent".to_owned()),
            Self::Unknown => Some("unknown".to_owned()),
        }
    }

    pub(crate) fn parse(value: &str) -> Self {
        if value == "absent" {
            Self::Absent
        } else if value == "unknown" {
            Self::Unknown
        } else if let Some(path) = value.strip_prefix("implicit:")
            && Path::new(path).is_absolute()
            && !path.chars().any(char::is_control)
        {
            Self::Implicit(PathBuf::from(path))
        } else if Path::new(value).is_absolute() && !value.chars().any(char::is_control) {
            Self::Path(PathBuf::from(value))
        } else {
            Self::Invalid
        }
    }
}

/// What a seat's optional `config_home_base.<slot>` row says.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RecordedConfigHomeBase {
    /// No base belongs to a legacy or explicit config-home row.
    #[default]
    Missing,
    /// The canonical effective `HOME` that selected an implicit store.
    Path(PathBuf),
    /// A malformed, duplicated, or inconsistent base row.
    Invalid,
}

impl RecordedConfigHomeBase {
    /// The value emitted when this state belongs in a meta row.
    #[must_use]
    pub fn record_value(&self) -> Option<String> {
        match self {
            Self::Path(path) => Some(path.display().to_string()),
            Self::Missing | Self::Invalid => None,
        }
    }

    pub(crate) fn parse(value: &str) -> Self {
        if Path::new(value).is_absolute() && !value.chars().any(char::is_control) {
            Self::Path(PathBuf::from(value))
        } else {
            Self::Invalid
        }
    }
}

/// What a seat's optional `client.<slot>` row says.
///
/// This row selects an ACCOUNT, so it follows [`RecordedConfigHome`], never
/// `profile`: an empty, duplicated or malformed row is [`Self::Invalid`] and
/// fails closed, never collapsing to [`Self::Missing`]. A silent fallback
/// would launch against the wrong store.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RecordedClient {
    /// No row was recorded — no override was honored for this seat.
    #[default]
    Missing,
    /// Exactly one well-formed row naming a client label.
    Label(String),
    /// An empty, duplicated or malformed row: no value may be trusted.
    Invalid,
}

impl RecordedClient {
    /// The value emitted when this state belongs in a meta row.
    #[must_use]
    pub fn record_value(&self) -> Option<String> {
        match self {
            Self::Label(label) => Some(label.clone()),
            Self::Missing | Self::Invalid => None,
        }
    }

    pub(crate) fn parse(value: &str) -> Self {
        if crate::config::is_agent_name(value) {
            Self::Label(value.to_owned())
        } else {
            Self::Invalid
        }
    }
}

impl RosterEntry {
    /// The DISPLAY ref this agent is known by in the ledger and on panes.
    #[must_use]
    pub fn reference(&self) -> String {
        self.name.clone()
    }
}

/// The two spellings a positive server selector normalizes to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Selector {
    /// `positive(name:<nonempty>)`.
    Name(String),
    /// `positive(socket:<absolute-path>)`.
    Socket(PathBuf),
}

/// What a durable record says about its tmux server — typed knowledge
/// fact, normalized from both recorded forms.
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
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingRow {
    slot: String,
    value: String,
    key: String,
    line: usize,
    /// A later duplicate of this metadata key was met, so its VALUE is
    /// invalidated.
    duplicated: bool,
    /// False only for a bare row (no `=`): present but valueless. The key
    /// alone cannot say so — keyed rows arrive already split.
    has_value: bool,
}

/// How often a slot has been claimed by a `seat.<slot>` KEY: a second claim —
/// keyed or bare — is a seat in doubt.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SlotClaim {
    slot: String,
    claims: usize,
}

/// Everything the reader SAW in a meta and could not take at face value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Anomaly {
    /// UNCLASSIFIED — a key outside the context and roster sets.
    UnknownKey {
        /// The key as written.
        key: String,
        /// 1-based line number.
        line: usize,
    },
    /// UNCLASSIFIED — a line with no `=` at all.
    MalformedLine {
        /// 1-based line number.
        line: usize,
    },
    /// UNCLASSIFIED — a key that appears more than once.
    DuplicateKey {
        /// The key as written.
        key: String,
        /// 1-based line number of the repeat.
        line: usize,
    },
    /// A roster value that is not `alias:name[:session-id]`, or an
    /// identity-v2 `seat.<slot>` with an empty name.
    MalformedRosterEntry {
        /// The key as written.
        key: String,
        /// 1-based line number.
        line: usize,
    },
    /// The v1 roster row `agent.<slot>`, which this ae does not read
    /// into a seat.
    LegacyRoster {
        /// The slot as written.
        slot: String,
        /// 1-based line number.
        line: usize,
    },
    /// Identity v2 — one NAME carried by more than one seat.
    DuplicateName {
        /// The name as written.
        name: String,
        /// 1-based line number of the row that revealed the collision.
        line: usize,
    },
    /// The config-home mode and its optional effective-HOME base disagree.
    InconsistentConfigHome {
        /// The affected seat slot.
        slot: String,
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
            Self::LegacyRoster { slot, line } => {
                write!(
                    f,
                    "slot {slot} carries the retired v1 roster agent.{slot} (line {line}): \
                     this session is not served by this ae"
                )
            }
            Self::DuplicateName { name, line } => {
                write!(
                    f,
                    "roster name {name} is claimed by more than one seat (line {line})"
                )
            }
            Self::InconsistentConfigHome { slot } => write!(
                f,
                "inconsistent config_home.{slot}/config_home_base.{slot} metadata"
            ),
        }
    }
}

/// A parsed session `meta`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Meta {
    mode: Option<String>,
    origin: Option<String>,
    work_dir: Option<String>,
    goal: Option<String>,
    /// The executable version recorded when ae created the session.
    ae_version: Option<String>,
    /// The version of the core binary this session's helpers are pinned to.
    ae_core: Option<String>,
    /// First successful publication time; absent on legacy sessions.
    created: Option<String>,
    /// Most recent successful launch/resume time; absent on legacy sessions.
    started: Option<String>,
    /// Per-seat legacy launch times, retained for the started fallback.
    launch_times: Vec<(String, String)>,
    /// `observed_model.<slot>` rows, kept RAW until the accessor validates.
    observed_models: Vec<(String, String)>,
    /// `observed_model_pin.<slot>` rows, same rule.
    observed_model_pins: Vec<(String, String)>,
    /// `harness_session_prior.<slot>` rows, kept RAW until the accessor splits
    /// them; the family is this parser's own, so it never doubts the roster.
    harness_session_priors: Vec<(String, String)>,
    /// The raw `meta_version=` value — the shape this document is written in.
    declared_version: Option<String>,
    roster: Vec<RosterEntry>,
    /// `agent_bin.<slot>` values whose `agent.<slot>` / `seat.<slot>` has not
    /// been read yet.
    pending_binaries: Vec<(String, String)>,
    /// Identity v2 metadata rows (`profile.<slot>`, `harness_session.<slot>`)
    /// read before their `seat.<slot>` — same rule as the binaries.
    pending_profiles: Vec<PendingRow>,
    pending_harness: Vec<PendingRow>,
    pending_config_homes: Vec<PendingRow>,
    pending_config_home_bases: Vec<PendingRow>,
    pending_clients: Vec<PendingRow>,
    pending_work_dirs: Vec<PendingRow>,
    /// Every `seat.<slot>` KEY met so far — `=` or not, valid or not, first or
    /// repeated.
    claims: Vec<SlotClaim>,
    /// v2 names already found on more than one seat: no later seat may take them.
    doubtful_names: Vec<String>,
    /// The raw `schema=` value, where the writer recorded one.
    schema: Option<String>,
    /// The raw pinned idle reminder cadence, kept until the accessor parses it.
    idle_nudge_secs: Option<String>,
    /// Two selector keys, kept RAW.
    server_value: Option<String>,
    server_kind: Option<String>,
    /// Whether either selector key appeared more than once.
    server_duplicated: bool,
    anomalies: Vec<Anomaly>,
}

impl Meta {
    /// Read and parse the `meta` inside the session directory at `dir`.
    ///
    /// # Errors
    ///
    /// Returns the underlying [`io::Error`] — an absent meta included.
    /// That absence DEGRADES the session, in deliberate
    /// contrast with quiet treatment of an absent event log: a fresh
    /// session has no events yet, but a session with no meta has lost its
    /// context and its whole roster at once.
    pub fn read(dir: &Path) -> io::Result<Self> {
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: the meta read itself — see clippy.toml"
        )]
        let text = fs::read_to_string(crate::store::open(dir).meta_path())?;
        Ok(Self::parse(&text))
    }

    /// Parse meta text.
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
            // Split on the FIRST equals.
            let Some((key, value)) = raw.split_once('=') else {
                // A bare `agent.main` / `seat.main` is still a CLAIM on the
                // slot, so it is noted before the line is refused.
                meta.note_claim(raw, line, false);
                // A bare seat-dir row reports ONLY its keyed verdict (which
                // the resolver owns): a second MalformedLine anomaly would
                // doubt the roster in preflight and steal the shared wording.
                let bare_work_dir = raw.strip_prefix(SEAT_WORK_DIR_PREFIX).is_some();
                let metadata = raw
                    .strip_prefix(CONFIG_HOME_PREFIX)
                    .map(|slot| (Metadata::ConfigHome, slot))
                    .or_else(|| {
                        raw.strip_prefix(CONFIG_HOME_BASE_PREFIX)
                            .map(|slot| (Metadata::ConfigHomeBase, slot))
                    })
                    .or_else(|| {
                        raw.strip_prefix(CLIENT_PREFIX)
                            .map(|slot| (Metadata::Client, slot))
                    })
                    .or_else(|| {
                        raw.strip_prefix(SEAT_WORK_DIR_PREFIX)
                            .map(|slot| (Metadata::WorkDir, slot))
                    });
                if let Some((which, slot)) = metadata {
                    let already_seen = seen.iter().any(|previous| previous == raw);
                    if already_seen {
                        meta.invalidate(raw);
                        // A repeated bare seat-dir row reports its duplicate:
                        // MalformedLine is suppressed for this family, so
                        // without this the invalidation would be silent.
                        // DuplicateKey on `work_dir.` never doubts the
                        // roster — the resolver still owns the verdict.
                        if bare_work_dir {
                            meta.anomalies.push(Anomaly::DuplicateKey {
                                key: raw.to_owned(),
                                line,
                            });
                        }
                    } else {
                        seen.push(raw.to_owned());
                        meta.set_metadata(which, slot, raw, "", line, false);
                    }
                }
                if !bare_work_dir {
                    meta.anomalies.push(Anomaly::MalformedLine { line });
                }
                continue;
            };
            // Claims are judged on the raw key, BEFORE the duplicate check and
            // before any value is parsed.
            let already_seen = seen.iter().any(|previous| previous == key);
            meta.note_claim(key, line, already_seen);
            if already_seen {
                // UNCLASSIFIED: nobody has ruled whether the first or the last
                // occurrence wins.
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
        meta.validate_config_home_pairs();
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
            "idle_nudge_secs" => self.idle_nudge_secs = None,
            CORE_KEY => self.ae_core = None,
            CREATED_KEY => self.created = None,
            STARTED_KEY => self.started = None,
            crate::migrate::KEY => self.declared_version = None,
            SCHEMA_KEY => self.schema = None,
            // A repeated selector key is AMBIGUOUS, which is a
            // stronger statement than invalidation: the flag survives
            // even if the repeats agreed.
            SERVER_KEY | SERVER_KIND_KEY => self.server_duplicated = true,
            _ => {
                if key.strip_prefix(LAUNCH_TIME_PREFIX).is_some() {
                    self.launch_times.retain(|(recorded, _)| recorded != key);
                } else if key.strip_prefix(OBSERVED_MODEL_PREFIX).is_some() {
                    self.observed_models.retain(|(recorded, _)| recorded != key);
                } else if key.strip_prefix(OBSERVED_MODEL_PIN_PREFIX).is_some() {
                    self.observed_model_pins
                        .retain(|(recorded, _)| recorded != key);
                } else if let Some(slot) = key.strip_prefix(ROSTER_BIN_PREFIX) {
                    if let Some(entry) = self.roster.iter_mut().find(|e| e.slot == slot) {
                        entry.binary = None;
                    }
                    self.pending_binaries.retain(|(pending, _)| pending != slot);
                } else if let Some(slot) = key.strip_prefix(PROFILE_PREFIX) {
                    if let Some(entry) = self.roster.iter_mut().find(|e| e.slot == slot) {
                        entry.profile = None;
                    }
                    self.mark_metadata_duplicated(key);
                } else if let Some(slot) = key.strip_prefix(HARNESS_SESSION_PREFIX) {
                    if let Some(entry) = self.roster.iter_mut().find(|e| e.slot == slot) {
                        entry.harness_session = None;
                    }
                    self.mark_metadata_duplicated(key);
                } else if key.strip_prefix(HARNESS_SESSION_PRIOR_PREFIX).is_some() {
                    self.harness_session_priors
                        .retain(|(recorded, _)| recorded != key);
                } else if let Some(slot) = key.strip_prefix(CONFIG_HOME_PREFIX) {
                    if let Some(entry) = self.roster.iter_mut().find(|e| e.slot == slot) {
                        entry.config_home = RecordedConfigHome::Invalid;
                    }
                    self.mark_metadata_duplicated(key);
                } else if let Some(slot) = key.strip_prefix(CONFIG_HOME_BASE_PREFIX) {
                    if let Some(entry) = self.roster.iter_mut().find(|e| e.slot == slot) {
                        entry.config_home_base = RecordedConfigHomeBase::Invalid;
                    }
                    self.mark_metadata_duplicated(key);
                } else if let Some(slot) = key.strip_prefix(CLIENT_PREFIX) {
                    if let Some(entry) = self.roster.iter_mut().find(|e| e.slot == slot) {
                        entry.client = RecordedClient::Invalid;
                    }
                    self.mark_metadata_duplicated(key);
                } else if let Some(slot) = key.strip_prefix(SEAT_WORK_DIR_PREFIX) {
                    if let Some(entry) = self.roster.iter_mut().find(|e| e.slot == slot) {
                        entry.work_dir = RecordedWorkDir::Invalid(SEAT_WORK_DIR_DUPLICATED);
                    }
                    self.mark_metadata_duplicated(key);
                } else if let Some(slot) = key.strip_prefix(SEAT_PREFIX) {
                    // A doubly-named slot is a slot whose identity is in doubt,
                    // and agents[] membership is roster-defined —
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
            "idle_nudge_secs" => self.idle_nudge_secs = Some(value.to_owned()),
            CORE_KEY => self.ae_core = Some(value.to_owned()),
            CREATED_KEY => self.created = Some(value.to_owned()),
            STARTED_KEY => self.started = Some(value.to_owned()),
            crate::migrate::KEY => self.declared_version = Some(value.to_owned()),
            // The selector family is the one exception: these two are read and
            // normalized rather than tolerated-and-ignored.
            SERVER_KEY => self.server_value = Some(value.to_owned()),
            SERVER_KIND_KEY => self.server_kind = Some(value.to_owned()),
            SCHEMA_KEY => self.schema = Some(value.to_owned()),
            _ => {
                if key
                    .strip_prefix(LAUNCH_TIME_PREFIX)
                    .is_some_and(|slot| !slot.is_empty())
                {
                    self.launch_times.push((key.to_owned(), value.to_owned()));
                } else if key
                    .strip_prefix(OBSERVED_MODEL_PREFIX)
                    .is_some_and(|slot| !slot.is_empty())
                {
                    self.observed_models
                        .push((key.to_owned(), value.to_owned()));
                } else if key
                    .strip_prefix(OBSERVED_MODEL_PIN_PREFIX)
                    .is_some_and(|slot| !slot.is_empty())
                {
                    self.observed_model_pins
                        .push((key.to_owned(), value.to_owned()));
                } else if let Some(slot) = key.strip_prefix(ROSTER_BIN_PREFIX) {
                    self.set_binary(slot, value);
                } else if let Some(slot) = key.strip_prefix(PROFILE_PREFIX) {
                    self.set_metadata(Metadata::Profile, slot, key, value, line, true);
                } else if let Some(slot) = key.strip_prefix(HARNESS_SESSION_PREFIX) {
                    self.set_metadata(Metadata::HarnessSession, slot, key, value, line, true);
                } else if key.strip_prefix(HARNESS_SESSION_PRIOR_PREFIX).is_some() {
                    // RAW, like the observed-model rows: the accessor is the
                    // one place the list is split and `valid_priors` the one
                    // place it is judged.
                    self.harness_session_priors
                        .push((key.to_owned(), value.to_owned()));
                } else if let Some(slot) = key.strip_prefix(CONFIG_HOME_PREFIX) {
                    self.set_metadata(Metadata::ConfigHome, slot, key, value, line, true);
                } else if let Some(slot) = key.strip_prefix(CONFIG_HOME_BASE_PREFIX) {
                    self.set_metadata(Metadata::ConfigHomeBase, slot, key, value, line, true);
                } else if let Some(slot) = key.strip_prefix(CLIENT_PREFIX) {
                    self.set_metadata(Metadata::Client, slot, key, value, line, true);
                } else if let Some(slot) = key.strip_prefix(SEAT_WORK_DIR_PREFIX) {
                    self.set_metadata(Metadata::WorkDir, slot, key, value, line, true);
                } else if let Some(slot) = key.strip_prefix(ROSTER_PREFIX) {
                    self.note_legacy(slot, line);
                } else if let Some(slot) = key.strip_prefix(SEAT_PREFIX) {
                    // A repeated claim was already refused by `note_claim`.
                    if !self.is_repeated(slot) {
                        self.absorb_seat(key, slot, value, line);
                    }
                } else {
                    // Unclassified: recorded, never interpreted.
                    self.anomalies.push(Anomaly::UnknownKey {
                        key: key.to_owned(),
                        line,
                    });
                }
            }
        }
    }

    /// Record a v1 roster row: it names no seat this ae will serve.
    ///
    /// The row is REPORTED rather than dropped. A silent drop would render a v1
    /// session identically to a healthy one whose roster is empty, and
    /// those are the two facts a reader most needs told apart.
    fn note_legacy(&mut self, slot: &str, line: usize) {
        self.anomalies.push(Anomaly::LegacyRoster {
            slot: slot.to_owned(),
            line,
        });
    }

    /// Identity v2: `seat.<slot>=<name>`.
    fn absorb_seat(&mut self, key: &str, slot: &str, value: &str, line: usize) {
        if value.is_empty() {
            self.anomalies.push(Anomaly::MalformedRosterEntry {
                key: key.to_owned(),
                line,
            });
            return;
        }
        // The name is the identity, so it must be UNIQUE across the roster.
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
        // Empty metadata is ABSENT metadata: the seat is still an agent.
        let profile = take_pending(&mut self.pending_profiles, slot)
            .and_then(|row| (!row.value.is_empty() && !row.duplicated).then_some(row.value));
        let harness_session = take_pending(&mut self.pending_harness, slot)
            .and_then(|row| (!row.value.is_empty() && !row.duplicated).then_some(row.value));
        let config_home = take_pending(&mut self.pending_config_homes, slot).map_or(
            RecordedConfigHome::Missing,
            |row| {
                if row.duplicated {
                    RecordedConfigHome::Invalid
                } else {
                    RecordedConfigHome::parse(&row.value)
                }
            },
        );
        let config_home_base = take_pending(&mut self.pending_config_home_bases, slot).map_or(
            RecordedConfigHomeBase::Missing,
            |row| {
                if row.duplicated {
                    RecordedConfigHomeBase::Invalid
                } else {
                    RecordedConfigHomeBase::parse(&row.value)
                }
            },
        );
        let client =
            take_pending(&mut self.pending_clients, slot).map_or(RecordedClient::Missing, |row| {
                if row.duplicated {
                    RecordedClient::Invalid
                } else {
                    RecordedClient::parse(&row.value)
                }
            });
        let work_dir = take_pending(&mut self.pending_work_dirs, slot).map_or(
            RecordedWorkDir::Missing,
            |row| {
                if row.duplicated {
                    RecordedWorkDir::Invalid(SEAT_WORK_DIR_DUPLICATED)
                } else if row.has_value {
                    RecordedWorkDir::parse(&row.value)
                } else {
                    RecordedWorkDir::Invalid(SEAT_WORK_DIR_NO_VALUE)
                }
            },
        );
        self.roster.push(RosterEntry {
            slot: slot.to_owned(),
            name: value.to_owned(),
            profile,
            client,
            harness_session,
            config_home,
            config_home_base,
            binary,
            work_dir,
        });
    }

    /// Record a `seat.<slot>` KEY claim, and refuse a repeated one.
    fn note_claim(&mut self, key: &str, line: usize, already_seen: bool) {
        let Some(slot) = key.strip_prefix(SEAT_PREFIX) else {
            return;
        };
        let at = self
            .claims
            .iter()
            .position(|claim| claim.slot == slot)
            .unwrap_or_else(|| {
                self.claims.push(SlotClaim {
                    slot: slot.to_owned(),
                    claims: 0,
                });
                self.claims.len() - 1
            });
        self.claims[at].claims += 1;
        if self.claims[at].claims > 1 {
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
    /// provenance.
    fn mark_metadata_duplicated(&mut self, key: &str) {
        for list in [
            &mut self.pending_profiles,
            &mut self.pending_harness,
            &mut self.pending_config_homes,
            &mut self.pending_config_home_bases,
            &mut self.pending_clients,
            &mut self.pending_work_dirs,
        ] {
            for row in list.iter_mut() {
                if row.key == key {
                    row.duplicated = true;
                }
            }
        }
    }

    fn is_repeated(&self, slot: &str) -> bool {
        self.claims
            .iter()
            .any(|claim| claim.slot == slot && claim.claims > 1)
    }

    /// Drop every seat named `name`, record the collision at `line`, and
    /// remember the name so no later seat takes it.
    fn mark_name_doubtful(&mut self, name: &str, line: usize) {
        self.roster.retain(|entry| entry.name != name);
        if !self.doubtful_names.iter().any(|n| n == name) {
            self.doubtful_names.push(name.to_owned());
        }
        self.anomalies.push(Anomaly::DuplicateName {
            name: name.to_owned(),
            line,
        });
    }

    /// Named per-seat metadata: attaches to its seat, or waits for one that has
    /// not been read yet.
    fn set_metadata(
        &mut self,
        which: Metadata,
        slot: &str,
        key: &str,
        value: &str,
        line: usize,
        keyed: bool,
    ) {
        let config_home = (which == Metadata::ConfigHome).then(|| RecordedConfigHome::parse(value));
        let config_home_base =
            (which == Metadata::ConfigHomeBase).then(|| RecordedConfigHomeBase::parse(value));
        let client = (which == Metadata::Client).then(|| RecordedClient::parse(value));
        let work_dir = (which == Metadata::WorkDir).then(|| {
            if keyed {
                RecordedWorkDir::parse(value)
            } else {
                RecordedWorkDir::Invalid(SEAT_WORK_DIR_NO_VALUE)
            }
        });
        if slot.is_empty()
            || config_home
                .as_ref()
                .is_some_and(|value| *value == RecordedConfigHome::Invalid)
            || config_home_base
                .as_ref()
                .is_some_and(|value| *value == RecordedConfigHomeBase::Invalid)
            || client
                .as_ref()
                .is_some_and(|value| *value == RecordedClient::Invalid)
            || work_dir
                .as_ref()
                .is_some_and(|value| matches!(value, RecordedWorkDir::Invalid(_)))
        {
            self.anomalies.push(Anomaly::MalformedRosterEntry {
                key: key.to_owned(),
                line,
            });
        }
        // Ownership FIRST, value second: an empty row on a seat is absent
        // metadata, not a missing seat.
        if let Some(existing) = self.roster.iter_mut().find(|entry| entry.slot == slot) {
            let value = (!value.is_empty()).then(|| value.to_owned());
            match which {
                Metadata::Profile => existing.profile = value,
                Metadata::HarnessSession => existing.harness_session = value,
                Metadata::ConfigHome => {
                    existing.config_home = config_home.unwrap_or(RecordedConfigHome::Invalid);
                }
                Metadata::ConfigHomeBase => {
                    existing.config_home_base =
                        config_home_base.unwrap_or(RecordedConfigHomeBase::Invalid);
                }
                Metadata::Client => {
                    existing.client = client.unwrap_or(RecordedClient::Invalid);
                }
                Metadata::WorkDir => {
                    // Unreachable `None`: `which` is WorkDir, so the verdict
                    // above is `Some`. The fallback fails closed.
                    existing.work_dir =
                        work_dir.unwrap_or(RecordedWorkDir::Invalid(SEAT_WORK_DIR_EMPTY));
                }
            }
            return;
        }
        let row = PendingRow {
            slot: slot.to_owned(),
            value: value.to_owned(),
            key: key.to_owned(),
            line,
            duplicated: false,
            has_value: keyed,
        };
        match which {
            Metadata::Profile => self.pending_profiles.push(row),
            Metadata::HarnessSession => self.pending_harness.push(row),
            Metadata::ConfigHome => self.pending_config_homes.push(row),
            Metadata::ConfigHomeBase => self.pending_config_home_bases.push(row),
            Metadata::Client => self.pending_clients.push(row),
            Metadata::WorkDir => self.pending_work_dirs.push(row),
        }
    }

    /// Only an implicit store has a base, and every implicit store needs one.
    fn validate_config_home_pairs(&mut self) {
        for entry in &mut self.roster {
            let valid = matches!(
                (&entry.config_home, &entry.config_home_base),
                (
                    RecordedConfigHome::Implicit(_),
                    RecordedConfigHomeBase::Path(_)
                ) | (
                    RecordedConfigHome::Missing
                        | RecordedConfigHome::Path(_)
                        | RecordedConfigHome::Absent
                        | RecordedConfigHome::Unknown,
                    RecordedConfigHomeBase::Missing
                )
            );
            if !valid {
                entry.config_home = RecordedConfigHome::Invalid;
                entry.config_home_base = RecordedConfigHomeBase::Invalid;
                self.anomalies.push(Anomaly::InconsistentConfigHome {
                    slot: entry.slot.clone(),
                });
            }
        }
    }

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

    /// The normalized server selector, read side only.
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
            // Absent kind: the one-key form, which named a server.
            None if !value.is_empty() => ServerSelector::Positive(Selector::Name(value.to_owned())),
            None => ServerSelector::Missing,
            // Present but EMPTY.
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

    /// The copy mode the session was started in.
    #[must_use]
    pub fn mode(&self) -> Option<&str> {
        self.mode.as_deref()
    }

    /// Where the session came from.
    #[must_use]
    pub fn origin(&self) -> Option<&str> {
        self.origin.as_deref()
    }

    /// The working directory its agents run in.
    #[must_use]
    pub fn work_dir(&self) -> Option<&str> {
        self.work_dir.as_deref()
    }

    /// The session's one-line objective.
    #[must_use]
    pub fn goal(&self) -> Option<&str> {
        self.goal.as_deref()
    }

    /// The ae version captured in this session's meta for the human list
    /// subline.
    #[must_use]
    pub fn ae_version(&self) -> Option<&str> {
        self.ae_version.as_deref()
    }

    /// The model a seat was positively observed running, when one unique,
    /// well-formed `observed_model.<slot>` row says so.
    ///
    /// An empty, duplicated or malformed row is ABSENT, never a model: a
    /// guessed model here would be written into a launch command.
    #[must_use]
    pub fn observed_model(&self, slot: &str) -> Option<&str> {
        observed_row(&self.observed_models, OBSERVED_MODEL_PREFIX, slot)
    }

    /// The profile's own model flag value, recorded beside the observation.
    #[must_use]
    pub fn observed_model_pin(&self, slot: &str) -> Option<&str> {
        observed_row(&self.observed_model_pins, OBSERVED_MODEL_PIN_PREFIX, slot)
    }

    /// The conversations this seat has ABANDONED, oldest first — the raw
    /// `harness_session_prior.<slot>` list, split on commas. The family is this
    /// parser's own: no unknown-key anomaly, never a doubt against the roster.
    /// The list is RAW — a consumer building a path must judge each element
    /// first ([`prior_parts`] does, and it also says which TOOL owns the
    /// conversation) — and a malformed element is NOT a refusal.
    #[must_use]
    pub fn harness_session_prior(&self, slot: &str) -> Vec<&str> {
        let key = format!("{HARNESS_SESSION_PRIOR_PREFIX}{slot}");
        self.harness_session_priors
            .iter()
            .find(|(recorded, _)| *recorded == key)
            .map(|(_, value)| value.split(',').collect())
            .unwrap_or_default()
    }

    /// The version of the core binary this session's helpers are pinned to.
    #[must_use]
    pub fn ae_core_version(&self) -> Option<&str> {
        self.ae_core.as_deref()
    }

    /// When this session was first created.
    #[must_use]
    pub fn created_epoch(&self) -> Option<i64> {
        positive_epoch(self.created.as_deref())
    }

    /// When this session was most recently launched or resumed.
    #[must_use]
    pub fn started_epoch(&self) -> Option<i64> {
        positive_epoch(self.started.as_deref())
    }

    /// Newest configured-seat launch time, for sessions predating started.
    #[must_use]
    pub fn latest_launch_epoch(&self) -> Option<i64> {
        self.launch_times
            .iter()
            .filter_map(|(key, value)| {
                let slot = key.strip_prefix(LAUNCH_TIME_PREFIX)?;
                if slot == "main" || slot.starts_with("worker.") {
                    positive_epoch(Some(value))
                } else {
                    None
                }
            })
            .max()
    }

    /// The shape this meta declares — see [`crate::migrate`].
    #[must_use]
    pub fn meta_version(&self) -> Option<&str> {
        self.declared_version.as_deref()
    }

    /// The roster, in the order the meta lists its `agent.<slot>`
    /// keys.
    #[must_use]
    pub fn roster(&self) -> &[RosterEntry] {
        &self.roster
    }

    /// The raw `schema=` value the writer recorded, if any.
    #[must_use]
    pub fn schema(&self) -> Option<&str> {
        self.schema.as_deref()
    }

    /// The inputs the ONE [`resolve_idle_nudge_secs`] resolver classifies: the
    /// RAW row (kept even when the value is unusable) and whether the meta
    /// named it twice. Resolving is deliberately NOT done here — the watchdog
    /// and the read surfaces must judge the same row by the same function.
    #[must_use]
    pub fn idle_nudge_pin(&self) -> (Option<&str>, bool) {
        let doubled = self.anomalies.iter().any(|anomaly| {
            matches!(anomaly, Anomaly::DuplicateKey { key, .. } if key == "idle_nudge_secs")
        });
        (self.idle_nudge_secs.as_deref(), doubled)
    }

    /// Everything this reader met and is not authorised to interpret.
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
    ConfigHome,
    ConfigHomeBase,
    Client,
    WorkDir,
}

/// A persisted epoch that can produce a meaningful age.
fn positive_epoch(value: Option<&str>) -> Option<i64> {
    value?.parse::<i64>().ok().filter(|epoch| *epoch > 0)
}

/// Remove and return the row pending for `slot`, if any.
fn take_pending(pending: &mut Vec<PendingRow>, slot: &str) -> Option<PendingRow> {
    let at = pending.iter().position(|row| row.slot == slot)?;
    Some(pending.swap_remove(at))
}

/// The record-side check for a seat-dir value: the same absolute/control-free
/// rule the parser judges, refused with the shared wording before any write.
pub(crate) fn checked_seat_work_dir(slot: &str, value: &str) -> Result<String, String> {
    match RecordedWorkDir::parse(value) {
        RecordedWorkDir::Path(path) => Ok(path.display().to_string()),
        RecordedWorkDir::Invalid(detail) => Err(seat_work_dir_refusal(slot, detail)),
        // `parse` judges a PRESENT value; only absence is Missing.
        RecordedWorkDir::Missing => Err(seat_work_dir_refusal(slot, SEAT_WORK_DIR_EMPTY)),
    }
}

/// The ONE wording for a present-but-unusable seat-dir row, with the defect
/// the evidence named. Operational consumers propagate it verbatim.
fn seat_work_dir_refusal(slot: &str, detail: &str) -> String {
    format!(
        "work_dir.{slot} is present but unusable ({detail}) — \
         restore the recorded path or retire the seat."
    )
}

/// The directory a seat starts in: its recorded `work_dir.<slot>`, or the
/// session default when no row is recorded.
///
/// The ONE judge of a seat-dir row. Absence inherits, a recorded path is
/// returned verbatim, and a present-but-unusable row refuses with the reason
/// every operational consumer propagates. Recovery surfaces (retire, end)
/// never call this — they read the row for audit and are never blocked by it.
/// Absence means a KNOWN seat with no row: an unknown slot refuses, because a
/// typoed slot silently inheriting the session dir is the wrong-repo hazard.
///
/// # Errors
///
/// The shared refusal when the slot's row is present but unusable, or the
/// slot names no recorded seat at all.
pub fn resolve_seat_dir(meta: &Meta, slot: &str) -> Result<String, String> {
    let entry = seat_entry(meta, slot)?;
    match &entry.work_dir {
        RecordedWorkDir::Missing => Ok(meta.work_dir().unwrap_or(".").to_owned()),
        RecordedWorkDir::Path(path) => Ok(path.display().to_string()),
        RecordedWorkDir::Invalid(detail) => Err(seat_work_dir_refusal(slot, detail)),
    }
}

/// The roster row a seat-dir judgment reads — one finder, one unknown-slot
/// wording, shared by the string and the typed resolvers.
fn seat_entry<'a>(meta: &'a Meta, slot: &str) -> Result<&'a RosterEntry, String> {
    meta.roster()
        .iter()
        .find(|entry| entry.slot == slot)
        .ok_or_else(|| {
            format!(
                "no seat is recorded for slot '{slot}' — refusing a directory for an unknown seat."
            )
        })
}

/// Whether a seat's working directory was recorded or inherited — the fact
/// the context render needs, since an explicit path textually equal to the
/// session default still omits `TREE_LOCAL`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatProvenance {
    /// No row recorded: the session directory, inherited.
    Inherited,
    /// A `work_dir.<slot>` row named this place.
    Explicit,
}

/// A seat's working directory as a checked place, not a spelling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatTarget {
    /// The canonical directory, proven by the strict door.
    pub canonical: PathBuf,
    /// Recorded or inherited — provenance, never value comparison, decides
    /// the context a seat is told it sits in.
    pub provenance: SeatProvenance,
}

/// Pure selection: a recorded canonical path wins with [`SeatProvenance::Explicit`];
/// absence inherits the session canonical with [`SeatProvenance::Inherited`].
/// No filesystem touch — the caller strict-checks both inputs through the
/// door, and the fuzz lane drives this against synthetic roots.
#[must_use]
pub fn select_seat_target(
    recorded_canonical: Option<PathBuf>,
    session_canonical: PathBuf,
) -> SeatTarget {
    match recorded_canonical {
        Some(canonical) => SeatTarget {
            canonical,
            provenance: SeatProvenance::Explicit,
        },
        None => SeatTarget {
            canonical: session_canonical,
            provenance: SeatProvenance::Inherited,
        },
    }
}

/// Pure containment: is `target` at-or-under `root`, by path COMPONENTS of
/// canonical forms — never string prefix, so `…/ae2` never matches `…/ae` and
/// no `..` survives to climb out. Both inputs must already be canonical; the
/// door proves that, this proves the nesting. No filesystem touch.
#[must_use]
pub fn contained_in(canonical_target: &Path, canonical_root: &Path) -> bool {
    let target: Vec<_> = canonical_target.components().collect();
    let root: Vec<_> = canonical_root.components().collect();
    target.len() >= root.len() && target[..root.len()] == root[..]
}

/// Typed resolve: the [`SeatTarget`] a seat starts from — recorded canonical
/// with `Explicit`, caller-supplied session canonical with `Inherited`. Pure
/// over the parse; the unknown-slot and unusable-row refusals are the shared
/// [`resolve_seat_dir`] wordings, and the strict door owns every filesystem
/// proof underneath, at record and again at each use.
///
/// # Errors
///
/// The shared refusal when the slot names no recorded seat or its row is
/// present but unusable.
pub fn resolve_seat_target(
    meta: &Meta,
    slot: &str,
    session_canonical: PathBuf,
) -> Result<SeatTarget, String> {
    let entry = seat_entry(meta, slot)?;
    match &entry.work_dir {
        RecordedWorkDir::Missing => Ok(select_seat_target(None, session_canonical)),
        RecordedWorkDir::Path(path) => {
            Ok(select_seat_target(Some(path.clone()), session_canonical))
        }
        RecordedWorkDir::Invalid(detail) => Err(seat_work_dir_refusal(slot, detail)),
    }
}

/// Validate an EXPLICIT seat target for recording: local mode, no legacy
/// `agent.<slot>` rows, strict directory proof, control-root containment.
/// Returns the canonical row value to store. Refuses before any write —
/// callers publish only on `Ok`, so a refusal leaves the meta byte-identical.
///
/// A relative dir joins the invoker cwd (the namer, not the session) and is
/// then proven like any other spelling. Containment binds explicit targets
/// only: inherited session dirs were proven by their own launch.
///
/// # Errors
///
/// The refusal naming the defect: non-local mode, legacy roster rows, an
/// unprovable target, or a target at-or-under ae state or the origin `.ae`.
pub fn record_seat_target(
    meta: &Meta,
    slot: &str,
    dir: &str,
    state_root: &Path,
    invoker_cwd: &Path,
) -> Result<String, String> {
    match meta.mode() {
        Some("local") => {}
        Some(mode) => {
            return Err(format!(
                "explicit seat targets record on local sessions only — this session's mode is '{mode}'."
            ));
        }
        None => {
            return Err(
                "explicit seat targets record on local sessions only — this session records no mode."
                    .to_owned(),
            );
        }
    }
    if meta
        .anomalies()
        .iter()
        .any(|anomaly| matches!(anomaly, Anomaly::LegacyRoster { .. }))
    {
        return Err(
            "a legacy agent.<slot> row is recorded — seat targets need a v2 roster; start a fresh session."
                .to_owned(),
        );
    }
    // Empty input refuses before the join and every door read: joining it
    // would silently accept the caller cwd as a target nobody named.
    if dir.is_empty() {
        return Err(seat_work_dir_refusal(slot, SEAT_WORK_DIR_EMPTY));
    }
    let spelled = PathBuf::from(dir);
    let joined = if spelled.is_absolute() {
        spelled
    } else {
        invoker_cwd.join(spelled)
    };
    let canonical = crate::doors::canonical_strict_dir(&joined)
        .map_err(|error| strict_refusal(dir, &error, "restore it or choose another target"))?;
    let canonical_root = crate::doors::canonical_strict_dir(state_root).map_err(|error| {
        strict_refusal(
            &state_root.display().to_string(),
            &error,
            "ae state itself is unreadable",
        )
    })?;
    if contained_in(&canonical, &canonical_root) {
        return Err(format!(
            "the target '{}' is at-or-under the ae state root '{}' — worker targets must live outside ae state.",
            canonical.display(),
            canonical_root.display()
        ));
    }
    if let Some(origin) = meta.origin().filter(|origin| !origin.is_empty()) {
        let ae = Path::new(origin).join(crate::inventory::WORKTREE_STATE_DIR);
        if let Some(ae_root) = crate::doors::canonical_optional_ae(&ae).map_err(|error| {
            strict_refusal(
                &ae.display().to_string(),
                &error,
                "the origin .ae dir is present but unusable",
            )
        })? && contained_in(&canonical, &ae_root)
        {
            return Err(format!(
                "the target '{}' is at-or-under the origin .ae dir '{}' — worker targets must live outside ae state.",
                canonical.display(),
                ae_root.display()
            ));
        }
    }
    canonical_row_value(slot, &canonical)
}

/// The row value for a proven canonical destination: UTF-8 text or refusal.
/// Pure over the path — the record helper proves the place first, this
/// proves the spelling is row-safe. A non-UTF8 destination refuses with the
/// shared detail rather than letting `display()` lossily rewrite it into a
/// DIFFERENT persisted path.
fn canonical_row_value(slot: &str, canonical: &Path) -> Result<String, String> {
    match canonical.to_str() {
        Some(text) => checked_seat_work_dir(slot, text),
        None => Err(seat_work_dir_refusal(slot, SEAT_WORK_DIR_NON_UTF8)),
    }
}

/// Re-prove a recorded target immediately before use: re-canonicalize the
/// STORED path and require equality — a retargeted alias or a swapped node
/// refuses with the shared row wording instead of following into a surprise
/// destination.
///
/// # Errors
///
/// The shared `work_dir.<slot>` refusal when the target moved, vanished, or
/// stopped being a directory.
pub fn check_seat_target_use(slot: &str, target: &SeatTarget) -> Result<(), String> {
    let proven = crate::doors::canonical_strict_dir(&target.canonical).map_err(|error| {
        let detail = match error {
            crate::doors::StrictDirError::Absent { .. } => "recorded target gone",
            crate::doors::StrictDirError::Unreadable { .. } => "recorded target unreadable",
            crate::doors::StrictDirError::NotDirectory { .. } => {
                "recorded target is no longer a directory"
            }
        };
        seat_work_dir_refusal(slot, detail)
    })?;
    if proven == target.canonical {
        Ok(())
    } else {
        Err(seat_work_dir_refusal(
            slot,
            "recorded target destination changed",
        ))
    }
}

/// The directory a spawn's pane starts in: the seat's checked target when its
/// row resolves, else the session spelling verbatim. Both provenances take
/// the same equality invariant — the ACTUAL RETURNED SPELLING is
/// strict-checked immediately before return against the held canonical —
/// but failure is provenance-correct: explicit rows refuse with the shared
/// row wording, while an inherited session dir names itself as gone,
/// unreadable, not-a-directory, or moved, with a restore-the-session-dir
/// remedy (no row, no retire). Typed helper, exercised by tests; consumer
/// wiring lands with the B2 cutover.
///
/// # Errors
///
/// The shared `work_dir.<slot>` refusal for explicit rows; the inherited
/// session-dir refusal for inherited ones; the unknown-seat refusal when
/// the slot names no recorded seat at all.
pub fn checked_pane_start_dir(
    meta: &Meta,
    slot: &str,
    session_dir: &str,
    session_canonical: PathBuf,
) -> Result<String, String> {
    let target = resolve_seat_target(meta, slot, session_canonical)?;
    match target.provenance {
        SeatProvenance::Explicit => {
            check_seat_target_use(slot, &target)?;
            Ok(target.canonical.display().to_string())
        }
        // Inherited takes the same equality invariant, but the failure is
        // provenance-correct: no row is recorded, so the refusal names the
        // session directory and its remedy — never "present", never
        // "recorded target", never "retire the seat".
        SeatProvenance::Inherited => {
            // Strict-check the ACTUAL RETURNED SPELLING, not just the held
            // canonical: an alias retargeted after the baseline would pass a
            // held-canonical check while the spelling resolves elsewhere.
            let proven = crate::doors::canonical_strict_dir(Path::new(session_dir))
                .map_err(|error| inherited_dir_refusal(session_dir, &error))?;
            if proven == target.canonical {
                Ok(session_dir.to_owned())
            } else {
                Err(format!(
                    "the session directory '{session_dir}' no longer resolves to its recorded place — restore it before resuming this seat."
                ))
            }
        }
    }
}

/// The ONLY pre-lock check a spawn target gets: REPRESENTABILITY, not
/// semantics. A non-UTF8 spelling can never become a row value, and a
/// missing state root can never prove containment — both are unexpressible
/// in [`record_seat_target`]'s `(&str, &Path)` signature, so this gate runs
/// before the session check (fail fast) AND first inside the locked core
/// (authoritative), and it cannot reorder any record refusal. Everything
/// semantic — empty, control, strict proof, containment, mode, legacy —
/// lives only in the locked core. Pure: no door read, no meta read.
///
/// # Errors
///
/// The direct refusal when a `Some` spelling is not UTF-8 or no state root
/// is available. `None` always plans to no explicit target.
pub fn plan_spawn_target(
    target: Option<&Path>,
    state_root: Option<&Path>,
) -> Result<Option<PathBuf>, String> {
    let Some(spelled) = target else {
        return Ok(None);
    };
    #[allow(
        clippy::unnecessary_debug_formatting,
        reason = "Debug escapes non-UTF8 bytes losslessly; Display would print lossy replacement chars for the very bytes refused"
    )]
    if spelled.to_str().is_none() {
        return Err(format!(
            "the spelled target {spelled:?} is not UTF-8 — explicit seat targets must be UTF-8."
        ));
    }
    if state_root.is_none() {
        return Err(
            "no ae state root is available — an explicit target cannot prove containment."
                .to_owned(),
        );
    }
    Ok(Some(spelled.to_path_buf()))
}

/// The PURE explicit-row selector: what a seat's `work_dir.<slot>` row says,
/// with no inheritance fallback. Missing is a refusal — the explicit JIT
/// must never start a seat whose target was never recorded — Invalid carries
/// its owned reason, and Path carries the value for the strict door to
/// re-prove. Pure over the parse: no filesystem touch, so the `meta_parse`
/// fuzz target drives this for fixed slots.
///
/// # Errors
///
/// The honest no-row refusal, the shared row refusal for a present but
/// unusable row, or the unknown-seat refusal.
pub fn explicit_seat_row(meta: &Meta, slot: &str) -> Result<PathBuf, String> {
    let entry = seat_entry(meta, slot)?;
    match &entry.work_dir {
        RecordedWorkDir::Missing => Err(format!(
            "no work_dir.{slot} row is recorded — the seat has no explicit target."
        )),
        RecordedWorkDir::Path(path) => Ok(path.clone()),
        RecordedWorkDir::Invalid(detail) => Err(seat_work_dir_refusal(slot, detail)),
    }
}

/// The directory an EXPLICIT seat's pane starts in: the recorded row,
/// re-proven immediately before use. Unlike [`checked_pane_start_dir`] this
/// takes no session facts — an explicit row never inherits, so there is no
/// session canonical to prove and no unrelated failure to add. A retargeted
/// alias or a swapped node refuses with the shared row wording instead of
/// following into a surprise destination.
///
/// # Errors
///
/// The no-row, unusable-row, unknown-seat, or moved/vanished-target refusal.
pub fn checked_explicit_pane_start_dir(meta: &Meta, slot: &str) -> Result<String, String> {
    let canonical = explicit_seat_row(meta, slot)?;
    check_seat_target_use(
        slot,
        &SeatTarget {
            canonical: canonical.clone(),
            provenance: SeatProvenance::Explicit,
        },
    )?;
    Ok(canonical.display().to_string())
}

/// Factual stem of an inherited-dir refusal: the door finding, no remedy.
pub(crate) fn inherited_dir_cause(
    session_dir: &str,
    error: &crate::doors::StrictDirError,
) -> String {
    use crate::doors::StrictDirError::{Absent, NotDirectory, Unreadable};
    let tail = match error {
        Absent { .. } => String::from("is gone"),
        NotDirectory { .. } => String::from("is not a directory"),
        Unreadable { kind, .. } => format!("cannot be read ({kind:?})"),
    };
    format!("the session directory '{session_dir}' {tail}")
}

/// An inherited session dir that fails its use check, worded for what it
/// is: no row, no retire — restore the session dir.
pub(crate) fn inherited_dir_refusal(
    session_dir: &str,
    error: &crate::doors::StrictDirError,
) -> String {
    format!(
        "{} — restore it before resuming this seat.",
        inherited_dir_cause(session_dir, error)
    )
}

/// A strict-door failure worded for the path it refused, with the remedy the
/// caller names.
fn strict_refusal(spelled: &str, error: &crate::doors::StrictDirError, remedy: &str) -> String {
    use crate::doors::StrictDirError::{Absent, NotDirectory, Unreadable};
    match error {
        Absent { .. } => format!("'{spelled}' is not there — {remedy}."),
        Unreadable { kind, .. } => format!("'{spelled}' cannot be read ({kind:?}) — {remedy}."),
        NotDirectory { .. } => format!("'{spelled}' is not a directory — {remedy}."),
    }
}

/// The validated `work_dir.<slot>` row in raw meta bytes, for a rebuild that
/// must judge the row BEFORE trusting a parse: `sole_value` answers `None`
/// for a duplicate, an empty value and a lossy read alike, and any of those
/// silently becoming absence would inherit the session dir (the wrong-repo
/// hazard). So: no row is `Ok(None)`, one good row is carried, and anything
/// else refuses with the shared wording — including a bare row, which the
/// byte scan sees as a line without `=`.
///
/// # Errors
///
/// The shared refusal when the slot's row is present but unusable.
pub fn raw_seat_work_dir(bytes: &[u8], slot: &str) -> Result<Option<String>, String> {
    let key = format!("{SEAT_WORK_DIR_PREFIX}{slot}");
    let mut valued: Option<&[u8]> = None;
    let mut seen = 0u32;
    for line in bytes.split(|byte| *byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let bare = line == key.as_bytes();
        if bare {
            seen += 1;
            continue;
        }
        let Some(value) = line
            .strip_prefix(key.as_bytes())
            .and_then(|rest| rest.strip_prefix(b"="))
        else {
            continue;
        };
        seen += 1;
        valued = Some(value);
    }
    if seen == 0 {
        return Ok(None);
    }
    if seen > 1 {
        return Err(seat_work_dir_refusal(slot, SEAT_WORK_DIR_DUPLICATED));
    }
    let Some(value) = valued else {
        return Err(seat_work_dir_refusal(slot, SEAT_WORK_DIR_NO_VALUE));
    };
    let value = std::str::from_utf8(value)
        .map_err(|_| seat_work_dir_refusal(slot, SEAT_WORK_DIR_NON_UTF8))?;
    // The VALUE verdict is the parser's own: one judge, no drift between the
    // byte scan and the parsed read.
    match RecordedWorkDir::parse(value) {
        RecordedWorkDir::Path(_) => Ok(Some(value.to_owned())),
        RecordedWorkDir::Invalid(detail) => Err(seat_work_dir_refusal(slot, detail)),
        // `parse` judges a PRESENT value; only absence is Missing.
        RecordedWorkDir::Missing => Err(seat_work_dir_refusal(slot, SEAT_WORK_DIR_EMPTY)),
    }
}

/// The meta file's raw bytes — the read behind a single-key lookup, which
/// scans the file rather than parsing it.
///
/// # Errors
///
/// The underlying [`io::Error`]; an absent meta is `NotFound`.
pub fn read_bytes(dir: &Path) -> io::Result<Vec<u8>> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the raw meta read behind a helper's own-key lookup — see clippy.toml"
    )]
    let bytes = fs::read(crate::store::open(dir).meta_path());
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

/// The ONE `idle_nudge_secs` resolver, judged by the watchdog's own refusal
/// rule so both surfaces answer with one number:
///
/// - no row at all → `Some(fallback)` (the caller's documented default);
/// - exactly one usable row → `Some(pinned)`;
/// - a DOUBLED row, or a value that is not an unsigned integer → `None`.
///
/// `None` FAILS CLOSED. The watchdog refuses to start on it; the read side
/// claims NO ceiling (no `waiting-agent` escalation) and names the gap rather
/// than inventing the default, because a surface that invents a cadence can
/// disagree with the pane about a human claim.
#[must_use]
pub fn resolve_idle_nudge_secs(
    recorded: Option<&str>,
    doubled: bool,
    fallback: u64,
) -> Option<u64> {
    if doubled {
        return None;
    }
    match recorded {
        None => Some(fallback),
        Some(raw) => raw.parse::<u64>().ok(),
    }
}

/// One observed-model row, validated. Duplicates were dropped by the reader,
/// so a remaining row is unique; a value that fails the row grammar reads as
/// absent.
fn observed_row<'a>(rows: &'a [(String, String)], prefix: &str, slot: &str) -> Option<&'a str> {
    let key = format!("{prefix}{slot}");
    let (_, value) = rows.iter().find(|(recorded, _)| *recorded == key)?;
    is_observed_row_value(value).then_some(value.as_str())
}

/// The bounded value grammar both observed-model rows share, on the writer and
/// the reader: nonempty, at most 256 bytes, no control bytes. A model reaches
/// a launch command, so a value that fails this is never a model.
#[must_use]
fn is_observed_row_value(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

/// One predecessor element, split into the tool that OWNS the conversation and
/// the conversation itself.
///
/// The tag is what makes a predecessor readable across a reseat: a seat that
/// moved from codex to claude records ids from BOTH stores, and a reader that
/// looked every one of them up in the current tool's store would either miss a
/// conversation or name a file belonging to nobody. `tool` is `None` for a
/// LEGACY element written before tags existed; the rule for that one is the
/// chain's, and it is the tool of the slot AT READ TIME.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prior<'a> {
    /// The recorded `agent_bin` this conversation belongs to, where the element
    /// carries one.
    pub tool: Option<&'a str>,
    /// The conversation id — always a lowercase UUID.
    pub id: &'a str,
}

/// Whether `tool` may be written as a predecessor's tag.
///
/// A BASENAME grammar, deliberately narrow: the tag is never a path component
/// (every consumer uses it to pick a tool, and the id alone builds the
/// filename) but a row that cannot even look like a path is one fewer thing to
/// argue about. A recorded binary that fails this is not tagged at all — see
/// [`prior_element`] — so the writer can never author a row its own reader
/// drops.
#[must_use]
fn is_prior_tool(tool: &str) -> bool {
    !tool.is_empty()
        && tool.len() <= PRIOR_TOOL_MAX
        && tool
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// Split one raw predecessor element. `None` means the element is unusable.
///
/// ```ignore
/// assert_eq!(prior_parts("codex:11111111-1111-4111-8111-111111111111")?.tool, Some("codex"));
/// assert_eq!(prior_parts("11111111-1111-4111-8111-111111111111")?.tool, None);
/// ```
#[must_use]
pub fn prior_parts(element: &str) -> Option<Prior<'_>> {
    let uuid = crate::session_launch::capture::is_lowercase_uuid;
    match element.split_once(PRIOR_TAG) {
        // Split at the FIRST tag byte, and the tool grammar admits none of its
        // own, so one element can never split two ways.
        Some((tool, id)) => (is_prior_tool(tool) && uuid(id)).then_some(Prior {
            tool: Some(tool),
            id,
        }),
        None => uuid(element).then_some(Prior {
            tool: None,
            id: element,
        }),
    }
}

/// One element, written: `tool:id` where `tool` can be tagged, else the bare id.
#[must_use]
fn prior_element(tool: &str, id: &str) -> String {
    if is_prior_tool(tool) {
        format!("{tool}{PRIOR_TAG}{id}")
    } else {
        id.to_owned()
    }
}

/// The predecessor list `raw`, every UNTAGGED element tagged with `tool`.
///
/// `None` means nothing to write: the list is empty, already fully tagged, or
/// carries something this writer cannot judge. A damaged row is LEFT ALONE
/// rather than cleared — a migration may not destroy what it does not
/// understand, and every reader judges the row element by element anyway.
#[must_use]
pub fn priors_tagged(raw: &[&str], tool: &str) -> Option<String> {
    if raw.is_empty() || raw.len() > PRIOR_MAX || !is_prior_tool(tool) {
        return None;
    }
    let parsed: Option<Vec<Prior<'_>>> = raw.iter().copied().map(prior_parts).collect();
    let parsed = parsed?;
    if parsed.iter().all(|prior| prior.tool.is_some()) {
        return None;
    }
    Some(
        parsed
            .iter()
            .map(|prior| prior_element(prior.tool.unwrap_or(tool), prior.id))
            .collect::<Vec<_>>()
            .join(","),
    )
}

/// The predecessor list `raw` with `id` appended, tagged as `tool`'s — oldest
/// first, the oldest evicted once [`PRIOR_MAX`] is exceeded.
///
/// Every element is `tool:uuid` or a legacy bare UUID (the grammar
/// [`crate::session_launch::capture::is_lowercase_uuid`] owns), and a list
/// carrying anything unusable is not carried forward: the row restarts from
/// `id` alone rather than refusing, because a hand-edited predecessor list
/// must not make a session unresumable. The kept elements are tagged on the
/// way through, by [`priors_tagged`]'s rule, so one write settles the whole
/// row and no reader is left guessing about a mixture.
/// `None` means `id` itself is unusable; nothing is recorded.
#[must_use]
pub(crate) fn prior_with(raw: &[&str], id: &str, tool: &str) -> Option<String> {
    if !crate::session_launch::capture::is_lowercase_uuid(id) {
        return None;
    }
    let mut ids: Vec<String> = if raw.len() <= PRIOR_MAX
        && let Some(parsed) = raw
            .iter()
            .copied()
            .map(prior_parts)
            .collect::<Option<Vec<_>>>()
    {
        parsed
            .iter()
            .map(|prior| prior_element(prior.tool.unwrap_or(tool), prior.id))
            .collect()
    } else {
        Vec::new()
    };
    ids.push(prior_element(tool, id));
    let excess = ids.len().saturating_sub(PRIOR_MAX);
    ids.drain(..excess);
    Some(ids.join(","))
}

/// Everything ONE seat's move to another profile writes, every value resolved
/// by the caller before the meta is opened.
pub(crate) struct SeatMove<'a> {
    /// The `[profiles]` name the seat moves to.
    pub profile: &'a str,
    /// The binary that profile lexes to — the seat's new identity.
    pub binary: &'a str,
    /// The token that guards this seat's observed-model writes from now on.
    pub launch_id: &'a str,
    /// What happens to the conversation the seat was holding.
    pub conversation: Conversation<'a>,
}

/// What a seat move does with the conversation the seat was holding — ONE
/// value, because the two arms differ in three rows at once and a caller that
/// could spell half of each would publish a seat whose records contradict
/// themselves.
pub(crate) enum Conversation<'a> {
    /// The successor opens a NEW conversation and the old one is ABANDONED: it
    /// becomes a predecessor, tagged with the tool that owns its store, and the
    /// seat's store rows go with it — they belong to the account that is
    /// leaving, and `_run` records the successor's own at its first start.
    Fresh {
        /// A fresh UUID where the tool takes one at launch, `pending` where its
        /// id can only be captured afterwards.
        id: &'a str,
        /// The floor every store scan for that new conversation starts at.
        capture_floor: i64,
    },
    /// The conversation TRAVELLED: its files were copied into another account
    /// of the SAME tool, so the id stays, the capture floor it was born under
    /// stays, and nothing is abandoned — there is no predecessor, because the
    /// seat still holds the conversation the row names.
    ///
    /// The store rows are REWRITTEN rather than removed, in this same
    /// replacement: a published conversation whose account no row names is the
    /// window that would let a later reader look in the home the seat just
    /// left.
    Carried {
        /// `config_home.<slot>` — the target account, in
        /// [`RecordedConfigHome::record_value`]'s grammar.
        config_home: &'a str,
        /// `config_home_base.<slot>` — the canonical effective `HOME` an
        /// IMPLICIT store was selected against, and `None` for an explicit one.
        config_home_base: Option<&'a str>,
    },
}

/// The seat's rows after that move — PURE, so every one of them is pinnable
/// without a session on disk.
///
/// WRITES the profile, the tool, the conversation the new tool
/// starts on, its launch token and its capture floor, and appends the OLD
/// conversation to the predecessor list tagged with the tool that owns it
/// ([`prior_with`]) — the whole point of the tag, since the successor's tool
/// reads a different store.
///
/// REMOVES as deliberately as it writes: the config home and its implicit base,
/// which belong to the tool that is LEAVING (`_run` records the new tool's own
/// at its first start, and a stale row would point the successor's reader at
/// the predecessor's store); the launch time, which would date a launch that
/// has not happened; the observed model with its pin, which would read as
/// drift the moment the new tool answers; and the recorded CLIENT override,
/// which a launch's `profile@client` put there for a profile this seat no
/// longer runs. The row is removed rather than emptied, because an empty
/// override is not the same fact as no override.
pub(crate) fn reseated(text: &str, slot: &str, move_to: &SeatMove<'_>) -> String {
    let mut rows: Vec<(String, Option<String>)> = vec![
        (
            format!("{PROFILE_PREFIX}{slot}"),
            Some(move_to.profile.to_owned()),
        ),
        (
            format!("{ROSTER_BIN_PREFIX}{slot}"),
            Some(move_to.binary.to_owned()),
        ),
        (format!("{CLIENT_PREFIX}{slot}"), None),
        (
            format!("launch_id.{slot}"),
            Some(move_to.launch_id.to_owned()),
        ),
        (format!("{LAUNCH_TIME_PREFIX}{slot}"), None),
        (format!("{OBSERVED_MODEL_PREFIX}{slot}"), None),
        (format!("{OBSERVED_MODEL_PIN_PREFIX}{slot}"), None),
    ];
    match &move_to.conversation {
        Conversation::Fresh { id, capture_floor } => {
            let value = |key: &str| crate::lifecycle::meta_value(text.as_bytes(), key);
            let old_id = value(&format!("{HARNESS_SESSION_PREFIX}{slot}"));
            let old_binary = value(&format!("{ROSTER_BIN_PREFIX}{slot}"));
            let parsed = Meta::parse(text);
            let priors = parsed.harness_session_prior(slot);
            rows.push((format!("{CONFIG_HOME_PREFIX}{slot}"), None));
            rows.push((format!("{CONFIG_HOME_BASE_PREFIX}{slot}"), None));
            rows.push((
                format!("{HARNESS_SESSION_PREFIX}{slot}"),
                Some((*id).to_owned()),
            ));
            rows.push((
                format!("capture_floor.{slot}"),
                Some(capture_floor.to_string()),
            ));
            // A predecessor row is written only when there is a conversation to
            // keep: a seat whose id never resolved has nothing to hand on, and
            // `prior_with` refuses an id it cannot judge rather than recording
            // a guess.
            if let Some(row) = prior_with(&priors, &old_id, &old_binary) {
                rows.push((format!("{HARNESS_SESSION_PRIOR_PREFIX}{slot}"), Some(row)));
            }
        }
        // EVERY ROW THE FRESH ARM TOUCHES IS LEFT ALONE HERE, deliberately:
        // `harness_session.<slot>` because the seat still holds it,
        // `capture_floor.<slot>` because a retained exact conversation keeps
        // the floor it was born under, and `harness_session_prior.<slot>`
        // because nothing was abandoned. The store rows are the only ones that
        // move, and they move together.
        Conversation::Carried {
            config_home,
            config_home_base,
        } => {
            rows.push((
                format!("{CONFIG_HOME_PREFIX}{slot}"),
                Some((*config_home).to_owned()),
            ));
            rows.push((
                format!("{CONFIG_HOME_BASE_PREFIX}{slot}"),
                config_home_base.map(str::to_owned),
            ));
        }
    }
    rows.iter().fold(text.to_owned(), |document, (key, value)| {
        rewritten(&document, key, value.as_deref())
    })
}

/// Publish [`reseated`] as ONE guarded replacement: the seat must still be the
/// one the caller proved, or nothing is written.
///
/// The guard is the shape `record_observed_model` uses. A reseat holds the
/// session's lifecycle lock across its whole sequence, so this cannot race a
/// second reseat; what it CAN race is a `retire` plus a re-`spawn` landing a
/// different seat on the slot, and that successor must not inherit the move.
///
/// # Errors
///
/// [`RewriteError::NotWritten`] when the guard does not hold, or the lock,
/// read, write, sync or rename failed.
pub(crate) fn publish_seat_move(
    dir: &Path,
    slot: &str,
    agent: &str,
    move_to: &SeatMove<'_>,
) -> Result<(), RewriteError> {
    rewrite_under_lock(dir, |current| seat_move_for(current, slot, agent, move_to))
}

/// The guarded replacement itself — PURE, so the guard is pinnable without a
/// session on disk. `None` means WRITE NOTHING: either a different seat holds
/// the slot now, or the move changes not one byte.
fn seat_move_for(current: &str, slot: &str, agent: &str, move_to: &SeatMove<'_>) -> Option<String> {
    let seated = crate::lifecycle::meta_value(current.as_bytes(), &format!("{SEAT_PREFIX}{slot}"));
    if seated != agent {
        return None;
    }
    let next = reseated(current, slot, move_to);
    (next != current).then_some(next)
}

/// What raw session metadata says about the privileged orchestrator role.
///
/// This is deliberately byte-exact. `meta_agent=true\r` is not the authority
/// token, and a bare, empty, false, malformed or repeated claim is damage
/// rather than an ordinary session with no claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaAgentRole {
    /// No `meta_agent` claim exists.
    Absent,
    /// Exactly one byte-exact `meta_agent=true` record exists.
    Role,
    /// A claim exists but does not state that one value unambiguously.
    Damaged,
}

/// Reduce hostile raw metadata to its orchestrator-role claim.
#[must_use]
pub fn meta_agent_role(text: &[u8]) -> MetaAgentRole {
    let key = b"meta_agent";
    let mut claims = 0_u8;
    for line in text.split(|byte| *byte == b'\n') {
        if line == key {
            return MetaAgentRole::Damaged;
        }
        if line
            .strip_prefix(key)
            .is_some_and(|rest| rest.starts_with(b"="))
        {
            claims = claims.saturating_add(1);
            if claims > 1 {
                return MetaAgentRole::Damaged;
            }
        }
    }
    match (claims, sole_value(text, "meta_agent")) {
        (0, None) => MetaAgentRole::Absent,
        (1, Some(b"true")) => MetaAgentRole::Role,
        _ => MetaAgentRole::Damaged,
    }
}

/// `text` with `key` set to `value`, or removed when `value` is `None` — the
/// frozen helpers' awk, byte for byte (measured against it):
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
    /// or the rename failed.
    NotWritten(io::Error),
    /// The rename returned — the new meta IS visible — but the directory entry
    /// could not be synced, so whether it survives a crash is unknown.
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
/// `meta.lock`, by rewriting a temp file and renaming it over, durably: the
/// temp is synced before the
/// rename, and the directory is synced after it, because a synced inode
/// behind an unsynced directory entry is a meta that can revert on a crash
/// while the event announcing it survives.
///
/// # Errors
///
/// [`RewriteError::NotWritten`] when nothing visible changed — the lock not
/// acquired within the bound, an absent meta on a SET (an unset of an absent
/// meta is `Ok`),
/// or any read, write, sync or rename failure. [`RewriteError::Unknown`] when
/// the rename returned but the directory sync did not.
pub fn rewrite(dir: &Path, key: &str, value: Option<&str>) -> Result<(), RewriteError> {
    rewrite_rows(dir, &[(key, value)])
}

/// Publish a config-home row and its optional implicit-HOME base in one
/// replacement. No reader can observe an implicit store without its base.
pub(crate) fn record_config_home(
    dir: &Path,
    slot: &str,
    value: &str,
    base: Option<&str>,
) -> Result<(), RewriteError> {
    let home_key = format!("{CONFIG_HOME_PREFIX}{slot}");
    let base_key = format!("{CONFIG_HOME_BASE_PREFIX}{slot}");
    rewrite_rows(dir, &[(&home_key, Some(value)), (&base_key, base)])
}

/// Record the conversation a resume FALLBACK just abandoned: the predecessor
/// list gains `abandoned` (when usable) and `harness_session.<slot>` becomes
/// `pending`, in one locked replacement — no reader sees the cleared row
/// without the predecessor that explains it. Cleared even when `abandoned` is
/// unusable, and idempotent: a row already `pending` writes nothing.
///
/// A capture tool's fallback also republishes `capture_floor.<slot>` with the
/// fallback moment, in that SAME replacement: the fresh conversation is born
/// after it, and the stale floor would still admit the abandoned one's
/// neighbours. Transition-only — an already-`pending` row keeps its floor —
/// and only where the slot's own recorded binary says a capture is needed, so
/// a claude or grok fallback stays byte-identical.
pub(crate) fn record_abandoned_session(
    dir: &Path,
    slot: &str,
    abandoned: &str,
) -> Result<(), RewriteError> {
    rewrite_under_lock(dir, |current| {
        let parsed = Meta::parse(current);
        // A resume fallback never crosses tools, so the conversation it
        // abandons belongs to the slot's own recorded binary.
        let tool = parsed
            .roster()
            .iter()
            .find(|entry| entry.slot == slot)
            .and_then(|entry| entry.binary.as_deref())
            .unwrap_or_default();
        let prior = prior_with(&parsed.harness_session_prior(slot), abandoned, tool);
        let mut next = current.to_owned();
        if let Some(list) = prior {
            next = rewritten(
                &next,
                &format!("{HARNESS_SESSION_PRIOR_PREFIX}{slot}"),
                Some(&list),
            );
        }
        let session_key = format!("{HARNESS_SESSION_PREFIX}{slot}");
        let transitioned = first_value(current.as_bytes(), &session_key)
            != Some(crate::launch::PENDING.as_bytes());
        next = rewritten(&next, &session_key, Some(crate::launch::PENDING));
        if transitioned
            && crate::tool::ToolKind::from_binary_name(tool)
                .adapter()
                .capture
                .is_needed()
        {
            next = rewritten(
                &next,
                &format!("capture_floor.{slot}"),
                Some(&crate::time::Timestamp::now().epoch().to_string()),
            );
        }
        (next != current).then_some(next)
    })
}

/// The ONE locked read-modify-write the row writers share: take `meta.lock`,
/// read the document, hand it to `transform`, and publish what it returns.
/// `None` means nothing to write; an ABSENT meta is handed over as an empty
/// document and a write refused with the read's own `NotFound`.
fn rewrite_under_lock(
    dir: &Path,
    transform: impl FnOnce(&str) -> Option<String>,
) -> Result<(), RewriteError> {
    let path = crate::store::open(dir).meta_path();
    let _held = crate::store::lock(
        &crate::store::open(dir).meta_lock(),
        crate::store::LOCK_WAIT,
    )
    .map_err(RewriteError::NotWritten)?;
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the meta read, for its locked rewrite — see clippy.toml"
    )]
    let read = fs::read_to_string(&path);
    let (current, absent) = match read {
        Ok(text) => (text, None),
        Err(why) if why.kind() == io::ErrorKind::NotFound => (String::new(), Some(why)),
        Err(why) => return Err(RewriteError::NotWritten(why)),
    };
    let Some(next) = transform(&current) else {
        return Ok(());
    };
    if let Some(why) = absent {
        return Err(RewriteError::NotWritten(why));
    }
    publish_bytes(dir, &path, next.as_bytes())
}

fn rewrite_rows(dir: &Path, rows: &[(&str, Option<&str>)]) -> Result<(), RewriteError> {
    rewrite_under_lock(dir, |current| {
        let mut next = current.to_owned();
        for (key, value) in rows {
            next = rewritten(&next, key, *value);
        }
        (next != current).then_some(next)
    })
}

/// Publish or clear one seat's observed-model pair as ONE atomic, guarded
/// rewrite.
///
/// The two rows are one identity: `observed: Some((model, pin))` writes the
/// model and sets (`pin = Some`) or removes (`pin = None`) the pin row in the
/// same replacement; `observed: None` removes both. No reader can meet half
/// the pair.
///
/// The compare-and-swap guard is the shape `capture::commit_inner` uses: under
/// the meta lock, the slot must still name `agent`, still hold `tool`, and its
/// `launch_id.<slot>` must still be `launch_id`. A slot re-created by a later
/// spawn or resume therefore cannot inherit the old seat's observation.
///
/// A write that would not change the meta is a no-op, so an observer may call
/// this every cycle.
///
/// # Errors
///
/// [`RewriteError::NotWritten`] when the rows are not well-formed, the guard
/// does not hold, or the lock, read, write, sync or rename failed.
pub(crate) fn record_observed_model(
    dir: &Path,
    slot: &str,
    agent: &str,
    tool: crate::tool::ToolKind,
    launch_id: &str,
    observed: Option<(&str, Option<&str>)>,
) -> Result<(), RewriteError> {
    let refused = |why: &str| {
        RewriteError::NotWritten(io::Error::new(io::ErrorKind::InvalidInput, why.to_owned()))
    };
    if let Some((model, pin)) = observed
        && (!is_observed_row_value(model) || pin.is_some_and(|pin| !is_observed_row_value(pin)))
    {
        return Err(refused("the observed model rows are not well-formed"));
    }
    if launch_id.is_empty() {
        return Err(refused("the seat records no launch id to guard the write"));
    }
    let path = crate::store::open(dir).meta_path();
    let _held = crate::store::lock(
        &crate::store::open(dir).meta_lock(),
        crate::store::LOCK_WAIT,
    )
    .map_err(RewriteError::NotWritten)?;
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the meta read, for its guarded rewrite — see clippy.toml"
    )]
    let current = fs::read_to_string(&path).map_err(RewriteError::NotWritten)?;
    let parsed = Meta::parse(&current);
    let Some(entry) = parsed.roster().iter().find(|entry| entry.slot == slot) else {
        return Err(refused("the slot is no longer seated"));
    };
    if entry.name != agent
        || crate::tool::ToolKind::from_binary_name(entry.binary.as_deref().unwrap_or_default())
            != tool
    {
        return Err(refused("the slot moved to another seat"));
    }
    let launch_key = format!("launch_id.{slot}");
    let recorded_launch = sole_value(current.as_bytes(), &launch_key).map(String::from_utf8_lossy);
    if recorded_launch.as_deref() != Some(launch_id) {
        return Err(refused("the seat was re-created after the observation"));
    }
    let model_key = format!("{OBSERVED_MODEL_PREFIX}{slot}");
    let pin_key = format!("{OBSERVED_MODEL_PIN_PREFIX}{slot}");
    let mut next = current.clone();
    if let Some((model, pin)) = observed {
        next = rewritten(&next, &model_key, Some(model));
        next = rewritten(&next, &pin_key, pin);
    } else {
        next = rewritten(&next, &model_key, None);
        next = rewritten(&next, &pin_key, None);
    }
    if next == current {
        return Ok(());
    }
    publish_bytes(dir, &path, next.as_bytes())
}

/// Backfill the proven socket spelling for a legacy meta that predates the
/// typed server pair, publishing both rows in one atomic replacement.
///
/// This is deliberately narrower than a general selector writer: the only
/// permitted caller has already asked the historical server for its own
/// absolute socket path. A malformed path is rejected before the meta lock is
/// taken, and no intermediate one-row document is ever visible.
///
/// # Errors
///
/// [`RewriteError::NotWritten`] when `socket` is not an absolute, single-line
/// path, when the lock or meta read fails, or when publication does not become
/// visible. [`RewriteError::Unknown`] when the replacement became visible but
/// the directory sync failed.
pub fn backfill_server_socket(dir: &Path, socket: &str) -> Result<(), RewriteError> {
    if !Path::new(socket).is_absolute() || socket.contains(['\n', '\r']) {
        return Err(RewriteError::NotWritten(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the proven tmux socket path must be absolute and single-line",
        )));
    }
    let path = crate::store::open(dir).meta_path();
    let _held = crate::store::lock(
        &crate::store::open(dir).meta_lock(),
        crate::store::LOCK_WAIT,
    )
    .map_err(RewriteError::NotWritten)?;
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the legacy meta read, for its locked atomic server-pair backfill — see clippy.toml"
    )]
    let current = fs::read_to_string(&path).map_err(RewriteError::NotWritten)?;
    if Meta::parse(&current).server_selector() != ServerSelector::Missing {
        return Err(RewriteError::NotWritten(io::Error::new(
            io::ErrorKind::InvalidData,
            "legacy tmux server backfill requires metadata with no server pair",
        )));
    }
    let with_kind = rewritten(&current, SERVER_KIND_KEY, Some("socket"));
    let next = rewritten(&with_kind, SERVER_KEY, Some(socket));
    publish_bytes(dir, &path, next.as_bytes())
}

/// The whole INITIAL meta, published as one document under the meta lock — the
/// core side of `_meta-init`.
///
/// # Errors
///
/// [`RewriteError::NotWritten`] when the lock is not acquired, a meta already
/// exists, or any write/sync/rename fails; [`RewriteError::Unknown`] when the
/// rename returned but the directory sync did not.
pub fn init(dir: &Path, content: &str) -> Result<(), RewriteError> {
    let path = crate::store::open(dir).meta_path();
    let _held = crate::store::lock(
        &crate::store::open(dir).meta_lock(),
        crate::store::LOCK_WAIT,
    )
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

/// The whole meta, published OVER whatever is there — [`init`]'s sibling for a
/// caller that MEANS to replace an existing document.
///
/// # Errors
///
/// [`RewriteError::NotWritten`] when the lock is not acquired or any write,
/// sync or rename fails; [`RewriteError::Unknown`] when the rename returned but
/// the directory sync did not.
pub fn replace(dir: &Path, content: &str) -> Result<(), RewriteError> {
    let path = crate::store::open(dir).meta_path();
    let _held = crate::store::lock(
        &crate::store::open(dir).meta_lock(),
        crate::store::LOCK_WAIT,
    )
    .map_err(RewriteError::NotWritten)?;
    publish_bytes(dir, &path, content.as_bytes())
}

/// Take the `meta.lock` beside `dir`'s meta — the same lock [`rewrite`],
/// [`init`] and [`replace`] take for themselves — and hold it until the
/// returned handle is dropped.
///
/// # Errors
///
/// The underlying [`io::Error`] — the lock not acquired within
/// [`crate::store::LOCK_WAIT`], or the lock file not openable.
pub fn lock(dir: &Path) -> io::Result<fs::File> {
    crate::store::lock(
        &crate::store::open(dir).meta_lock(),
        crate::store::LOCK_WAIT,
    )
}

/// Publish `content` as the whole meta, for a caller that ALREADY HOLDS
/// [`lock`].
///
/// # Errors
///
/// [`RewriteError::NotWritten`] when any write, sync or rename fails;
/// [`RewriteError::Unknown`] when the rename returned but the directory sync
/// did not.
pub fn publish_locked(dir: &Path, content: &str) -> Result<(), RewriteError> {
    publish_bytes(
        dir,
        &crate::store::open(dir).meta_path(),
        content.as_bytes(),
    )
}

/// The staged BASE-FACTS document `_meta-init` consumes: the `key=value` lines
/// written for a session before its roster block exists, read from `path`.
///
/// # Errors
///
/// The underlying [`io::Error`] — an absent file is `NotFound`, an
/// undecodable one is `InvalidData`.
pub fn read_base(path: &Path) -> io::Result<String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the staged base-facts document _meta-init publishes a session's first meta from — see clippy.toml"
    )]
    let text = fs::read_to_string(path);
    text
}

/// Stage `bytes` to a per-process temp beside `path`, fsync it, rename it over
/// `path`, then fsync the directory so the entry is durable.
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
    // Visible now.
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
    fn a_prior_row_is_the_parsers_own_and_a_duplicate_reads_as_none() {
        const A: &str = "11111111-1111-4111-8111-111111111111";
        const B: &str = "22222222-2222-4222-8222-222222222222";
        const CURRENT: &str = "aabbccdd-1122-4333-8444-5566778899aa";
        // The family is the parser's own: NO anomaly, so it can never doubt the
        // roster (which would refuse every resume) nor read as an unknown key.
        let meta = super::Meta::parse(&format!(
            "seat.main=lead\nagent_bin.main=claude\nharness_session.main={CURRENT}\n\
             harness_session_prior.main={A},{B}\n"
        ));
        assert!(meta.anomalies().is_empty(), "{:?}", meta.anomalies());
        assert_eq!(meta.harness_session_prior("main"), [A, B]);
        assert_eq!(meta.harness_session_prior("other"), Vec::<&str>::new());
        // The prior row never disturbs the CURRENT id's classification.
        assert_eq!(meta.roster()[0].harness_session.as_deref(), Some(CURRENT));

        // A key named twice says nothing: neither occurrence is a predecessor,
        // and the roster is still not in doubt.
        let duplicated = super::Meta::parse(&format!(
            "harness_session_prior.main={A}\nharness_session_prior.main={B}\n"
        ));
        assert!(duplicated.harness_session_prior("main").is_empty());
        assert!(
            !duplicated
                .anomalies()
                .iter()
                .any(crate::roster::roster_doubting)
        );
    }

    #[test]
    fn a_prior_append_is_grammar_checked_capped_and_oldest_first() {
        const FOUR: [&str; 4] = [
            "11111111-1111-4111-8111-111111111111",
            "22222222-2222-4222-8222-222222222222",
            "33333333-3333-4333-8333-333333333333",
            "44444444-4444-4444-8444-444444444444",
        ];
        const FIFTH: &str = "55555555-5555-4555-8555-555555555555";
        // The 5th append evicts the OLDEST; order stays oldest first.
        assert_eq!(
            super::prior_with(&FOUR, FIFTH, "").as_deref(),
            Some(
                "22222222-2222-4222-8222-222222222222,33333333-3333-4333-8333-333333333333,\
                 44444444-4444-4444-8444-444444444444,55555555-5555-4555-8555-555555555555"
            )
        );
        // An unusable id is never appended; a malformed or over-cap list is
        // DROPPED, never carried and never a refusal.
        for id in [
            "pending",
            "",
            "ses_abc",
            "AAAA1111-1111-4111-8111-111111111111",
        ] {
            assert_eq!(super::prior_with(&FOUR, id, ""), None, "{id:?}");
        }
        assert_eq!(
            super::prior_with(&["not-a-uuid"], FIFTH, "").as_deref(),
            Some(FIFTH)
        );
        let over = [FOUR[0], FOUR[1], FOUR[2], FOUR[3], FIFTH];
        assert_eq!(super::prior_with(&over, FIFTH, "").as_deref(), Some(FIFTH));
    }

    #[test]
    fn a_prior_element_says_which_tool_owns_the_conversation() {
        const ID: &str = "11111111-1111-4111-8111-111111111111";
        // TAGGED: the tool comes off the element and the conversation is what is
        // left, so a reader picks the store by the id's OWN tool.
        let element = format!("codex:{ID}");
        let tagged = super::prior_parts(&element).expect("a tagged element");
        assert_eq!((tagged.tool, tagged.id), (Some("codex"), ID));
        // LEGACY: no tag, and the id still reads. Naming the tool is the
        // chain's job, not this reader's.
        let legacy = super::prior_parts(ID).expect("a legacy element");
        assert_eq!((legacy.tool, legacy.id), (None, ID));
        // A wrapper binary is an ordinary tag: the grammar is a BASENAME, not a
        // list of the tools ae knows.
        let wrapper = format!("claude-mic.1:{ID}");
        assert_eq!(
            super::prior_parts(&wrapper).map(|prior| prior.tool),
            Some(Some("claude-mic.1"))
        );
        let over = "c".repeat(super::PRIOR_TOOL_MAX + 1);
        // The id grammar is LOWERCASE, and this id has letters to prove it.
        let upper = "AABBCCDD-1122-4333-8444-5566778899AA";
        for element in [
            String::new(),
            "codex:".to_owned(),
            format!(":{ID}"),
            // A tag that wants to be a path, whole and by one byte. No consumer
            // joins a tag onto anything, and none may become able to.
            format!("../..:{ID}"),
            format!("co/dex:{ID}"),
            format!("co dex:{ID}"),
            format!("cod\tex:{ID}"),
            // One tag, never two.
            format!("codex:claude:{ID}"),
            format!("codex:{upper}"),
            // A state is not a conversation.
            "codex:pending".to_owned(),
            format!("{over}:{ID}"),
        ] {
            assert_eq!(super::prior_parts(&element), None, "{element:?}");
        }
        let at_bound = "c".repeat(super::PRIOR_TOOL_MAX);
        assert!(super::prior_parts(&format!("{at_bound}:{ID}")).is_some());
    }

    #[test]
    fn a_prior_append_tags_the_whole_row_with_the_tool_that_owns_it() {
        const A: &str = "11111111-1111-4111-8111-111111111111";
        const B: &str = "22222222-2222-4222-8222-222222222222";
        const NEW: &str = "33333333-3333-4333-8333-333333333333";
        // ONE write settles the row: the appended id and every legacy element
        // beside it take the tool, so no reader meets a mixture to reason about.
        let settled = format!("codex:{A},codex:{NEW}");
        assert_eq!(
            super::prior_with(&[A], NEW, "codex").as_deref(),
            Some(settled.as_str())
        );
        // An element that already names its tool KEEPS it — that is the whole
        // point of the tag, and a reseat's row carries two tools at once.
        let kept = format!("claude:{A},codex:{NEW}");
        assert_eq!(
            super::prior_with(&[&format!("claude:{A}")], NEW, "codex").as_deref(),
            Some(kept.as_str())
        );
        // A tool that cannot BE a tag is not written as one: the row stays
        // legacy rather than becoming one this reader would drop whole.
        let legacy = format!("{A},{NEW}");
        for tool in ["", "co/dex"] {
            assert_eq!(
                super::prior_with(&[A], NEW, tool).as_deref(),
                Some(legacy.as_str()),
                "{tool:?}"
            );
        }
        // The cap counts ELEMENTS however they are spelled, and the oldest is
        // still the one evicted.
        let four = [A, B, NEW, "44444444-4444-4444-8444-444444444444"];
        let fifth = "55555555-5555-4555-8555-555555555555";
        let capped = format!("muse:{B},muse:{NEW},muse:{},muse:{fifth}", four[3]);
        assert_eq!(
            super::prior_with(&four, fifth, "muse").as_deref(),
            Some(capped.as_str())
        );
    }

    #[test]
    fn tagging_an_existing_prior_row_adds_the_tool_and_destroys_nothing() {
        const A: &str = "11111111-1111-4111-8111-111111111111";
        const B: &str = "22222222-2222-4222-8222-222222222222";
        // The one thing it does: a legacy element gains the slot's tool, a
        // tagged one is left exactly as it is.
        let tagged = format!("codex:{A},claude:{B}");
        assert_eq!(
            super::priors_tagged(&[A, &format!("claude:{B}")], "codex").as_deref(),
            Some(tagged.as_str())
        );
        // NOTHING TO WRITE, every reason: no row, a row already fully tagged, a
        // slot whose binary cannot be a tag, a row this reader cannot judge,
        // and one over the cap. A damaged row is LEFT ALONE — a migration may
        // not destroy what it does not understand.
        let five = [A, A, A, A, A];
        assert_eq!(super::priors_tagged(&[], "codex"), None);
        assert_eq!(
            super::priors_tagged(&[&format!("codex:{A}")], "codex"),
            None
        );
        assert_eq!(super::priors_tagged(&[A], ""), None);
        assert_eq!(super::priors_tagged(&["not-a-uuid"], "codex"), None);
        assert_eq!(super::priors_tagged(&five, "codex"), None);
    }

    /// The document a reseat moves: one fully-recorded seat beside a second
    /// the move must not touch.
    fn seated() -> String {
        "session=work\nseat.main=lead\nseat.spawned.0=scout\n\
         profile.main=fable5\nagent_bin.main=claude\nclient.main=cc-mic\n\
         harness_session.main=11111111-1111-4111-8111-111111111111\n\
         launch_id.main=L-OLD\ncapture_floor.main=1000\nlaunch_time.main=1200\n\
         config_home.main=/home/x/.claude\nconfig_home_base.main=/home/x\n\
         observed_model.main=Opus 5\nobserved_model_pin.main=fable\n\
         profile.spawned.0=lunam\nagent_bin.spawned.0=codex\n\
         harness_session.spawned.0=99999999-9999-4999-8999-999999999999\n"
            .to_owned()
    }

    fn moving_to<'a>(profile: &'a str, binary: &'a str, id: &'a str) -> super::SeatMove<'a> {
        super::SeatMove {
            profile,
            binary,
            launch_id: "L-NEW",
            conversation: super::Conversation::Fresh {
                id,
                capture_floor: 5000,
            },
        }
    }

    /// The same move, but the conversation TRAVELLED: another account of the
    /// same tool, reached by copying its files.
    fn carrying<'a>(profile: &'a str, binary: &'a str, home: &'a str) -> super::SeatMove<'a> {
        super::SeatMove {
            profile,
            binary,
            launch_id: "L-NEW",
            conversation: super::Conversation::Carried {
                config_home: home,
                config_home_base: None,
            },
        }
    }

    #[test]
    fn a_carried_conversation_keeps_its_id_its_floor_and_leaves_no_predecessor() {
        // The seat moved; the conversation did not. Nothing was abandoned, so
        // nothing may be recorded as abandoned — and the store rows go with the
        // conversation in this SAME document, because a published conversation
        // whose account no row names would resolve to the home it just left.
        let moved = super::reseated(&seated(), "main", &carrying("cc-mic", "claude", "/h/b"));
        let value = |key: &str| crate::lifecycle::meta_value(moved.as_bytes(), key);
        assert_eq!(value("profile.main"), "cc-mic");
        assert_eq!(value("agent_bin.main"), "claude");
        assert_eq!(
            value("harness_session.main"),
            "11111111-1111-4111-8111-111111111111",
            "the conversation the seat still holds is untouched"
        );
        assert_eq!(
            value("capture_floor.main"),
            "1000",
            "a retained exact conversation keeps the floor it was born under"
        );
        assert!(
            !moved.contains("harness_session_prior.main="),
            "nothing was abandoned:\n{moved}"
        );
        // The store rows moved TOGETHER: the target path, and the base row this
        // seat used to carry REMOVED, because an explicit store has none and a
        // stale base would pair this account with another one's HOME.
        assert_eq!(value("config_home.main"), "/h/b");
        assert!(
            !moved.contains("config_home_base.main="),
            "an explicit store has no base row:\n{moved}"
        );
        // An IMPLICIT target is one identity with the HOME that selected it,
        // and both of its rows are published in this same document.
        let implicit = super::SeatMove {
            profile: "cc-mic",
            binary: "claude",
            launch_id: "L-NEW",
            conversation: super::Conversation::Carried {
                config_home: "implicit:/h/b/.claude",
                config_home_base: Some("/h/b"),
            },
        };
        let moved = super::reseated(&seated(), "main", &implicit);
        let value = |key: &str| crate::lifecycle::meta_value(moved.as_bytes(), key);
        assert_eq!(value("config_home.main"), "implicit:/h/b/.claude");
        assert_eq!(value("config_home_base.main"), "/h/b");
        // Everything a move always did to the seat's identity still happens.
        assert_eq!(value("launch_id.main"), "L-NEW");
        for gone in ["launch_time.main=", "observed_model.main=", "client.main="] {
            assert!(!moved.contains(gone), "{gone} survived:\n{moved}");
        }
    }

    #[test]
    fn a_seat_move_writes_the_new_tool_and_keeps_the_old_conversation_tagged() {
        const NEW: &str = "22222222-2222-4222-8222-222222222222";
        let moved = super::reseated(&seated(), "main", &moving_to("lunam", "codex", NEW));
        let value = |key: &str| crate::lifecycle::meta_value(moved.as_bytes(), key);
        assert_eq!(value("profile.main"), "lunam");
        assert_eq!(value("agent_bin.main"), "codex");
        assert_eq!(value("harness_session.main"), NEW);
        assert_eq!(value("launch_id.main"), "L-NEW");
        assert_eq!(value("capture_floor.main"), "5000");
        // The predecessor carries the tool that OWNS it, not the one arriving:
        // the successor's reader looks in a different store entirely.
        assert_eq!(
            value("harness_session_prior.main"),
            "claude:11111111-1111-4111-8111-111111111111"
        );
        // The seat beside it is untouched, every row.
        assert_eq!(value("profile.spawned.0"), "lunam");
        assert_eq!(value("agent_bin.spawned.0"), "codex");
        assert_eq!(
            value("harness_session.spawned.0"),
            "99999999-9999-4999-8999-999999999999"
        );
        assert_eq!(value("seat.main"), "lead");
        assert_eq!(value("session"), "work");
    }

    #[test]
    fn a_seat_move_removes_every_row_that_belonged_to_the_tool_that_left() {
        const NEW: &str = "22222222-2222-4222-8222-222222222222";
        let moved = super::reseated(&seated(), "main", &moving_to("lunam", "codex", NEW));
        for gone in [
            "config_home.main",
            "config_home_base.main",
            "launch_time.main",
            "observed_model.main",
            "observed_model_pin.main",
            "client.main",
        ] {
            assert!(!moved.contains(&format!("{gone}=")), "{gone} survived");
        }
    }

    #[test]
    fn a_seat_move_hands_on_no_conversation_it_cannot_prove() {
        const NEW: &str = "22222222-2222-4222-8222-222222222222";
        // `pending` is a claim about a capture that never completed, and the
        // predecessor list is addresses only.
        let pending = seated().replace(
            "harness_session.main=11111111-1111-4111-8111-111111111111",
            "harness_session.main=pending",
        );
        let moved = super::reseated(&pending, "main", &moving_to("lunam", "codex", NEW));
        assert!(!moved.contains("harness_session_prior.main="));
        assert_eq!(
            crate::lifecycle::meta_value(moved.as_bytes(), "harness_session.main"),
            NEW
        );
    }

    #[test]
    fn the_seat_move_guard_refuses_a_slot_a_different_agent_now_holds() {
        const NEW: &str = "22222222-2222-4222-8222-222222222222";
        let text = seated();
        let move_to = moving_to("lunam", "codex", NEW);
        // The race this closes: a reseat holds the session's lifecycle lock,
        // so no second reseat interleaves — but a `retire` plus a re-`spawn`
        // can land a DIFFERENT seat on the slot, and that successor must not
        // inherit a move published for the one before it.
        assert_eq!(
            super::seat_move_for(&text, "main", "someone-else", &move_to),
            None
        );
        assert_eq!(
            super::seat_move_for(&text, "spawned.9", "lead", &move_to),
            None
        );
        // The seat the caller proved, and only that one, is written.
        let moved = super::seat_move_for(&text, "main", "lead", &move_to)
            .expect("the seat the caller proved");
        assert_eq!(
            crate::lifecycle::meta_value(moved.as_bytes(), "profile.main"),
            "lunam"
        );
    }

    #[test]
    fn a_seat_move_that_writes_nothing_new_is_still_the_same_document() {
        // Idempotence is what makes the publication's "changed?" guard safe: a
        // second move to the SAME profile must not keep appending priors.
        const NEW: &str = "22222222-2222-4222-8222-222222222222";
        let once = super::reseated(&seated(), "main", &moving_to("lunam", "codex", NEW));
        let twice = super::reseated(&once, "main", &moving_to("lunam", "codex", NEW));
        assert_ne!(
            once, twice,
            "the second move retires the first conversation"
        );
        assert_eq!(
            crate::lifecycle::meta_value(twice.as_bytes(), "harness_session_prior.main"),
            format!("claude:11111111-1111-4111-8111-111111111111,codex:{NEW}")
        );
    }

    #[test]
    fn observed_model_rows_read_as_one_validated_pair() {
        let meta = super::Meta::parse(
            "seat.main=lead\nprofile.main=fable5\nagent_bin.main=claude\nlaunch_id.main=L1\n\
             observed_model.main=Opus 5 (1M context)\nobserved_model_pin.main=fable\n",
        );
        assert_eq!(meta.observed_model("main"), Some("Opus 5 (1M context)"));
        assert_eq!(meta.observed_model_pin("main"), Some("fable"));
        assert_eq!(meta.observed_model("other"), None);
        assert_eq!(meta.observed_model_pin("main"), Some("fable"));

        // A row named twice says nothing: neither occurrence is a model.
        let duplicated = super::Meta::parse(
            "observed_model.main=fable\nobserved_model.main=opus\nobserved_model_pin.main=fable\n",
        );
        assert_eq!(duplicated.observed_model("main"), None);

        // Empty is absent; a control byte is never a model.
        assert_eq!(
            super::Meta::parse("observed_model.main=\n").observed_model("main"),
            None
        );
        assert_eq!(
            super::Meta::parse("observed_model.main=a\u{1}b\n").observed_model("main"),
            None
        );
        // A pin alone (no observed row) is a readable report-only pair half.
        let pin_only = super::Meta::parse("observed_model_pin.main=fable\n");
        assert_eq!(pin_only.observed_model_pin("main"), Some("fable"));
        assert_eq!(pin_only.observed_model("main"), None);
    }

    #[test]
    fn the_observed_pair_is_published_and_cleared_as_one_guarded_identity() {
        let dir = std::env::temp_dir().join(format!("ae-meta-observed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        std::fs::write(
            dir.join("meta"),
            "schema=2\nseat.main=lead\nprofile.main=fable5\nagent_bin.main=claude\nlaunch_id.main=L1\n",
        )
        .expect("meta");

        super::record_observed_model(
            &dir,
            "main",
            "lead",
            crate::tool::ToolKind::Claude,
            "L1",
            Some(("Opus 5", Some("fable"))),
        )
        .expect("publish");
        let text = std::fs::read_to_string(dir.join("meta")).unwrap();
        assert!(text.contains("observed_model.main=Opus 5\n"), "{text}");
        assert!(text.contains("observed_model_pin.main=fable\n"), "{text}");

        // The same observation again is a no-op, byte for byte.
        let before = std::fs::read_to_string(dir.join("meta")).unwrap();
        super::record_observed_model(
            &dir,
            "main",
            "lead",
            crate::tool::ToolKind::Claude,
            "L1",
            Some(("Opus 5", Some("fable"))),
        )
        .expect("no-op");
        assert_eq!(std::fs::read_to_string(dir.join("meta")).unwrap(), before);

        // A stale launch id, a renamed seat and another tool cannot write.
        for (agent, tool, launch) in [
            ("lead", crate::tool::ToolKind::Claude, "L2"),
            ("other", crate::tool::ToolKind::Claude, "L1"),
            ("lead", crate::tool::ToolKind::Codex, "L1"),
        ] {
            assert!(
                super::record_observed_model(
                    &dir,
                    "main",
                    agent,
                    tool,
                    launch,
                    Some(("Opus 5", Some("fable")))
                )
                .is_err(),
                "{agent}/{tool:?}/{launch}"
            );
        }

        // Clearing removes both rows in one replacement.
        super::record_observed_model(
            &dir,
            "main",
            "lead",
            crate::tool::ToolKind::Claude,
            "L1",
            None,
        )
        .expect("clear");
        let text = std::fs::read_to_string(dir.join("meta")).unwrap();
        assert!(!text.contains("observed_model"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_fallback_on_a_capture_tool_republishes_a_fresh_capture_floor() {
        let dir = std::env::temp_dir().join(format!("ae-meta-capfloor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        let gone = "0199c0de-1234-4890-abcd-ef0123456789";
        std::fs::write(
            dir.join("meta"),
            format!(
                "schema=2\nseat.main=lead\nprofile.main=codex\nagent_bin.main=codex\n\
                 harness_session.main={gone}\ncapture_floor.main=1700000000\n"
            ),
        )
        .expect("meta");
        let before = crate::time::Timestamp::now().epoch();
        super::record_abandoned_session(&dir, "main", gone).expect("record");
        let text = std::fs::read_to_string(dir.join("meta")).unwrap();
        assert!(
            text.contains(&format!("harness_session_prior.main=codex:{gone}\n")),
            "{text}"
        );
        assert!(text.contains("harness_session.main=pending\n"), "{text}");
        let floor: i64 = text
            .lines()
            .find_map(|line| {
                line.strip_prefix("capture_floor.main=")
                    .and_then(|value| value.parse().ok())
            })
            .expect("a capture floor row");
        assert!(
            floor >= before && floor > 1_700_000_000,
            "the floor is the fallback moment, not the planted one: {text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_fallback_on_a_tool_that_needs_no_capture_leaves_the_floor_alone() {
        for (tag, binary, prior) in [
            ("claude", "agent_bin.main=claude\n", "claude:"),
            ("unknown", "", ""),
        ] {
            let dir =
                std::env::temp_dir().join(format!("ae-meta-capfloor-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch");
            let gone = "0199c0de-1234-4890-abcd-ef0123456789";
            std::fs::write(
                dir.join("meta"),
                format!(
                    "schema=2\nseat.main=lead\nprofile.main=custom\n{binary}\
                     harness_session.main={gone}\ncapture_floor.main=1000\n"
                ),
            )
            .expect("meta");
            super::record_abandoned_session(&dir, "main", gone).expect("record");
            let text = std::fs::read_to_string(dir.join("meta")).unwrap();
            assert!(
                text.contains(&format!("harness_session_prior.main={prior}{gone}\n")),
                "{tag}: {text}"
            );
            assert!(
                text.contains("harness_session.main=pending\n"),
                "{tag}: {text}"
            );
            assert!(
                text.contains("capture_floor.main=1000\n"),
                "{tag}: the planted floor is untouched: {text}"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_second_fallback_call_on_an_already_cleared_slot_writes_nothing() {
        // An unusable id records no predecessor, so both calls take the same
        // path and the second must be a byte-identical no-op — the floor
        // included. (A usable id never reaches a second call: `_run` passes
        // no abandoned id for a pending seat.)
        let dir =
            std::env::temp_dir().join(format!("ae-meta-capfloor-noop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        std::fs::write(
            dir.join("meta"),
            "schema=2\nseat.main=lead\nprofile.main=codex\nagent_bin.main=codex\n\
             harness_session.main=u-9\ncapture_floor.main=1700000000\n",
        )
        .expect("meta");
        super::record_abandoned_session(&dir, "main", "u-9").expect("record");
        let settled = std::fs::read_to_string(dir.join("meta")).unwrap();
        assert!(
            settled.contains("harness_session.main=pending\n"),
            "{settled}"
        );
        assert!(
            !settled.contains("harness_session_prior"),
            "an unusable id records no predecessor: {settled}"
        );
        let floor: i64 = settled
            .lines()
            .find_map(|line| {
                line.strip_prefix("capture_floor.main=")
                    .and_then(|value| value.parse().ok())
            })
            .expect("a capture floor row");
        assert!(floor > 1_700_000_000, "{settled}");
        // Age the floor by hand: a second call on the cleared slot must still
        // write nothing — the stale floor included — rather than refresh it.
        // (Without the re-age both calls could land in one clock second and a
        // rewrite would be invisible.)
        let aged = settled.replace(
            &format!("capture_floor.main={floor}\n"),
            "capture_floor.main=1700000000\n",
        );
        assert!(
            aged.contains("capture_floor.main=1700000000\n"),
            "{settled}"
        );
        std::fs::write(dir.join("meta"), &aged).expect("age the floor");
        super::record_abandoned_session(&dir, "main", "u-9").expect("no-op");
        let again = std::fs::read_to_string(dir.join("meta")).unwrap();
        assert_eq!(again, aged, "the cleared slot writes nothing more");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rewrite_replaces_appends_or_drops_byte_for_byte_as_the_awk_does() {
        // Every expectation is written out, not derived from the same code
        // that produces it.
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
        // hide the degradation the reader reports.
        assert_eq!(rewritten("k=1\nk=2\n", "k", Some("3")), "k=3\nk=3\n");
        assert_eq!(rewritten("k=1\nk=2\n", "k", None), "");
    }

    #[test]
    fn the_legacy_server_backfill_publishes_one_typed_socket_pair() {
        let dir =
            std::env::temp_dir().join(format!("ae-meta-server-backfill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        std::fs::write(dir.join("meta"), "session=old\nmain_pane=%0\n").expect("legacy meta");

        super::backfill_server_socket(&dir, "/tmp/private/default").expect("backfill");

        assert_eq!(
            std::fs::read_to_string(dir.join("meta")).unwrap(),
            "session=old\nmain_pane=%0\ntmux_server_kind=socket\ntmux_server=/tmp/private/default\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_legacy_server_backfill_refuses_an_unusable_path_without_touching_meta() {
        let dir = std::env::temp_dir().join(format!(
            "ae-meta-server-backfill-invalid-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        let original = "session=old\nmain_pane=%0\n";
        std::fs::write(dir.join("meta"), original).expect("legacy meta");

        assert!(super::backfill_server_socket(&dir, "relative/default").is_err());
        assert_eq!(std::fs::read_to_string(dir.join("meta")).unwrap(), original);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_legacy_server_backfill_never_replaces_a_pair_that_appeared_before_its_lock() {
        let dir = std::env::temp_dir().join(format!(
            "ae-meta-server-backfill-race-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        let original = "session=old\ntmux_server_kind=name\ntmux_server=already-there\n";
        std::fs::write(dir.join("meta"), original).expect("paired meta");

        assert!(super::backfill_server_socket(&dir, "/tmp/private/default").is_err());
        assert_eq!(std::fs::read_to_string(dir.join("meta")).unwrap(), original);
        let _ = std::fs::remove_dir_all(&dir);
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

    use super::{
        Anomaly, Meta, RecordedClient, RecordedConfigHome, RecordedConfigHomeBase, Selector,
        ServerSelector,
    };
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
        // Kind absent/empty x value absent/empty.
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
        // other's server.
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
        // The family left catch-all; nothing else did.
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
    fn sc_405c_the_binary_may_be_recorded_before_the_identity() {
        let meta = Meta::parse("agent_bin.main=claude\nseat.main=lead\n");
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

    /// Identity v2 (P1).
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
        assert_eq!(roster[0].reference(), "lead", "the display ref is the name");
        assert!(
            meta.anomalies().is_empty(),
            "every v2 key is read, none is unknown"
        );
    }

    #[test]
    fn config_home_rows_are_typed_and_hostile_shapes_are_invalid() {
        for (value, expected) in [
            (
                "/accounts/claude",
                RecordedConfigHome::Path(PathBuf::from("/accounts/claude")),
            ),
            ("absent", RecordedConfigHome::Absent),
            ("unknown", RecordedConfigHome::Unknown),
        ] {
            for text in [
                format!("seat.main=lead\nconfig_home.main={value}\n"),
                format!("config_home.main={value}\nseat.main=lead\n"),
            ] {
                let meta = Meta::parse(&text);
                assert_eq!(meta.roster()[0].config_home, expected, "{text:?}");
                assert!(meta.anomalies().is_empty(), "{text:?}");
            }
        }
        for text in [
            "seat.main=lead\nconfig_home.main=implicit:/accounts/claude\nconfig_home_base.main=/people/lead\n",
            "config_home_base.main=/people/lead\nconfig_home.main=implicit:/accounts/claude\nseat.main=lead\n",
        ] {
            let meta = Meta::parse(text);
            assert_eq!(
                meta.roster()[0].config_home,
                RecordedConfigHome::Implicit(PathBuf::from("/accounts/claude")),
                "{text:?}"
            );
            assert_eq!(
                meta.roster()[0].config_home_base,
                RecordedConfigHomeBase::Path(PathBuf::from("/people/lead")),
                "{text:?}"
            );
            assert!(meta.anomalies().is_empty(), "{text:?}");
        }

        for text in [
            "seat.main=lead\nconfig_home.main=relative\n",
            "config_home.main=\nseat.main=lead\n",
            "seat.main=lead\nconfig_home.main\n",
            "config_home.main\nseat.main=lead\n",
            "seat.main=lead\nconfig_home.main=/one\nconfig_home.main=/two\n",
            "config_home.main=/one\nconfig_home.main=/two\nseat.main=lead\n",
            "seat.main=lead\nconfig_home.main=implicit:/one\nconfig_home_base.main=relative\n",
            "config_home_base.main\nconfig_home.main=implicit:/one\nseat.main=lead\n",
            "config_home_base.main=/one\nconfig_home_base.main=/two\nconfig_home.main=implicit:/store\nseat.main=lead\n",
        ] {
            let meta = Meta::parse(text);
            assert_eq!(
                meta.roster()[0].config_home,
                RecordedConfigHome::Invalid,
                "{text:?}"
            );
            assert!(
                meta.anomalies().iter().any(|anomaly| matches!(
                    anomaly,
                    Anomaly::MalformedRosterEntry { .. } | Anomaly::DuplicateKey { .. }
                )),
                "{text:?}"
            );
        }
        assert_eq!(
            Meta::parse("seat.main=lead\n").roster()[0].config_home,
            RecordedConfigHome::Missing,
            "the optional row keeps old v2 metadata readable"
        );
        assert_eq!(
            Meta::parse("seat.main=lead\nconfig_home.main=implicit:/store\n").roster()[0]
                .config_home,
            RecordedConfigHome::Invalid,
            "an implicit store without its HOME base is unusable"
        );
    }

    #[test]
    fn client_rows_are_typed_and_hostile_shapes_are_invalid_never_missing() {
        // Label: exactly one well-formed row, both orders, no anomaly.
        for text in [
            "seat.main=lead\nclient.main=cc-mic\n",
            "client.main=cc-mic\nseat.main=lead\n",
        ] {
            let meta = Meta::parse(text);
            assert_eq!(
                meta.roster()[0].client,
                RecordedClient::Label("cc-mic".to_owned()),
                "{text:?}"
            );
            assert!(meta.anomalies().is_empty(), "{text:?}");
        }
        // Missing: the row is absent — no override was recorded.
        assert_eq!(
            Meta::parse("seat.main=lead\n").roster()[0].client,
            RecordedClient::Missing,
            "absence of evidence, never a derived label"
        );
        // Invalid: empty, duplicated or malformed — every one fails closed,
        // exactly like `config_home` and never like `profile`'s collapse.
        let long = "a".repeat(65);
        let hostile: Vec<String> = [
            "seat.main=lead\nclient.main=\n",
            "client.main=\nseat.main=lead\n",
            "seat.main=lead\nclient.main\n",
            "client.main\nseat.main=lead\n",
            "seat.main=lead\nclient.main=cc-mic\nclient.main=cc\n",
            "client.main=cc-mic\nclient.main=cc\nseat.main=lead\n",
            "seat.main=lead\nclient.main=cc mic\n",
            "seat.main=lead\nclient.main=fablex@cc-mic\n",
            "seat.main=lead\nclient.main=-lead\n",
            "seat.main=lead\nclient.main=_lead\n",
            "seat.main=lead\nclient.main=cc\tmic\n",
        ]
        .iter()
        .map(ToString::to_string)
        .chain([format!("seat.main=lead\nclient.main={long}\n")])
        .collect();
        for text in &hostile {
            let meta = Meta::parse(text);
            assert_eq!(meta.roster()[0].client, RecordedClient::Invalid, "{text:?}");
            assert!(
                meta.anomalies().iter().any(|anomaly| matches!(
                    anomaly,
                    Anomaly::MalformedRosterEntry { .. }
                        | Anomaly::DuplicateKey { .. }
                        | Anomaly::MalformedLine { .. }
                )),
                "{text:?}"
            );
        }
        // Only a label round-trips; Missing and Invalid emit no row.
        assert_eq!(
            RecordedClient::Label("cc-mic".to_owned()).record_value(),
            Some("cc-mic".to_owned())
        );
        assert_eq!(RecordedClient::Missing.record_value(), None);
        assert_eq!(RecordedClient::Invalid.record_value(), None);
    }

    #[test]
    fn identity_v2_metadata_rows_may_precede_their_seat() {
        // Same rule as `agent_bin`: profile/harness wait for the seat, never
        // create a half-built entry, and a slot that never gets one is not an
        // agent.
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

    /// Colead P1 gate BLOCKER-1: v2 metadata rows must never rewrite a v1
    /// identity, in EITHER order.
    /// Colead P1 gate BLOCKER-2: mixed-schema detection is by raw KEY claim,
    /// so a malformed, bare or duplicated FIRST claim (which leaves no entry)
    /// cannot let the other schema resurrect the slot — in both orders.
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
        // CONTROLS — the check is not over-strong: distinct names and a
        // single seat.
        let meta = Meta::parse("seat.main=lead\nseat.worker.0=colead\n");
        assert_eq!(meta.roster().len(), 2);
        assert!(meta.anomalies().is_empty());
        let meta = Meta::parse("seat.main=lead\n");
        assert_eq!(meta.roster().len(), 1);
        assert!(meta.anomalies().is_empty());
    }

    /// Colead P1 round-2 IMPORTANT-1: metadata that ATTACHED to a v2 seat is
    /// reclassified as unknown when the slot turns out mixed — the answer the
    /// other order already gave.
    /// Colead P1 round-2 IMPORTANT-2: an EMPTY metadata row is judged by the
    /// slot's schema first — unknown on v1 (both orders), absent metadata on v2.
    #[test]
    fn identity_v2_an_empty_metadata_row_is_absent_metadata_in_both_orders() {
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
    /// Colead P1 round-2 IMPORTANT-3: a bare `seat.<slot>` beside a keyed one
    /// is a repeated v2 claim — the slot stays absent in both orders, as the
    /// archive reader (which counts lines) already refuses it.
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
    }

    #[test]
    fn sc_405e_a_duplicate_key_invalidates_the_field_rather_than_picking_one() {
        // Precedence is UNCLASSIFIED.
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
    fn the_idle_nudge_pin_reads_once_and_invalidates_like_every_other_key() {
        let resolve = |text: &str| {
            let meta = Meta::parse(text);
            let (recorded, doubled) = meta.idle_nudge_pin();
            super::resolve_idle_nudge_secs(recorded, doubled, 300)
        };
        assert_eq!(resolve("idle_nudge_secs=420\n"), Some(420));
        assert_eq!(
            resolve("idle_nudge_secs=soon\n"),
            None,
            "an unusable value FAILS CLOSED; it does not fall back to 300"
        );
        assert_eq!(
            resolve("idle_nudge_secs=\n"),
            None,
            "an empty value is a row that cannot be used, not an absent row"
        );
        assert_eq!(
            resolve("idle_nudge_secs=60\nidle_nudge_secs=120\n"),
            None,
            "a doubled key is unusable, not picked"
        );
        assert_eq!(
            resolve("mode=local\n"),
            Some(300),
            "an ABSENT row takes the caller's fallback"
        );
    }

    #[test]
    fn sc_405e_every_retained_scalar_key_invalidates_the_same_way() {
        // One field tested is one field proven.
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
        // Membership is roster-defined, so a slot whose identity is in
        // doubt supplies no agent rather than a guessed one.
        let meta = Meta::parse(concat!(
            "seat.main=lead\n",
            "seat.main=someone-else\n",
            "seat.worker.0=coworker\n",
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
            "seat.main=lead\n",
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
            "seat.main=lead\n",
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
                key: "seat.main".to_owned(),
                line: 2
            }
            .to_string(),
            "malformed roster entry seat.main at line 2"
        );
        assert_eq!(
            Anomaly::LegacyRoster {
                slot: "main".to_owned(),
                line: 4
            }
            .to_string(),
            "slot main carries the retired v1 roster agent.main (line 4): \
             this session is not served by this ae"
        );
    }

    #[test]
    fn a_retired_v1_roster_row_names_no_seat_and_says_so_out_loud() {
        // This ae does not read `agent.<slot>` into a seat, and a v1 session is
        // one to start over from. The row is REPORTED rather than dropped — a
        // silent drop would render a v1 session
        // identically to a healthy one whose roster is simply empty.
        let meta = Meta::parse(concat!(
            "mode=local\n",
            "agent.main=claude:lead:e795c9e9\n",
            "agent_bin.main=claude\n",
            "agent.worker.0=codex:coworker\n",
        ));
        assert!(meta.roster().is_empty(), "no seat comes from a v1 row");
        assert_eq!(meta.mode(), Some("local"), "the rest still reads");
        let legacy: Vec<&str> = meta
            .anomalies()
            .iter()
            .filter_map(|anomaly| match anomaly {
                Anomaly::LegacyRoster { slot, .. } => Some(slot.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(legacy, ["main", "worker.0"], "every row is named");
        assert!(
            meta.anomalies()[0]
                .to_string()
                .contains("not served by this ae"),
            "the anomaly says why: {:?}",
            meta.anomalies()[0]
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

    #[test]
    fn meta_agent_role_is_byte_exact_and_three_valued() {
        use super::{MetaAgentRole, meta_agent_role};

        assert_eq!(meta_agent_role(b"meta_agent=true\n"), MetaAgentRole::Role);
        assert_eq!(
            meta_agent_role(b"session=ordinary\n"),
            MetaAgentRole::Absent
        );
        for damaged in [
            &b"meta_agent=true\r\n"[..],
            &b"meta_agent\n"[..],
            &b"meta_agent=\n"[..],
            &b"meta_agent=truth\n"[..],
            &b"meta_agent=true\nmeta_agent=false\n"[..],
            &b"meta_agent=false\nmeta_agent=true\n"[..],
        ] {
            assert_eq!(
                meta_agent_role(damaged),
                MetaAgentRole::Damaged,
                "{damaged:?}"
            );
        }
    }

    #[test]
    fn seat_work_dir_row_parses_to_recorded_path() {
        use super::{Meta, RecordedWorkDir};
        let meta = Meta::parse("seat.main=lead\nwork_dir.main=/w/target\n");
        assert!(meta.anomalies().is_empty());
        assert_eq!(
            meta.roster()[0].work_dir,
            RecordedWorkDir::Path(std::path::PathBuf::from("/w/target"))
        );
        // A row read BEFORE its seat still attaches; spaces are content.
        let meta = Meta::parse("work_dir.spawned.0=/w/with space\nseat.spawned.0=scout\n");
        assert!(meta.anomalies().is_empty());
        assert_eq!(
            meta.roster()[0].work_dir,
            RecordedWorkDir::Path(std::path::PathBuf::from("/w/with space"))
        );
        // No row is Missing, and the scalar row is untouched by the prefix.
        let meta = Meta::parse("work_dir=/session\nseat.main=lead\n");
        assert_eq!(meta.roster()[0].work_dir, RecordedWorkDir::Missing);
        assert_eq!(meta.work_dir(), Some("/session"));
    }

    #[test]
    fn seat_work_dir_malformed_values_are_invalid_and_reported() {
        use super::{Anomaly, Meta, RecordedWorkDir};
        for (bad, detail) in [
            ("", "empty value"),
            ("relative/path", "not an absolute path"),
            ("/x/\u{7}/y", "control characters"),
        ] {
            let meta = Meta::parse(&format!("seat.main=lead\nwork_dir.main={bad}\n"));
            assert_eq!(
                meta.roster()[0].work_dir,
                RecordedWorkDir::Invalid(detail),
                "{bad:?}"
            );
            assert!(
                meta.anomalies().iter().any(|a| matches!(
                    a,
                    Anomaly::MalformedRosterEntry { key, .. } if key == "work_dir.main"
                )),
                "{bad:?}: {:?}",
                meta.anomalies()
            );
        }
        // A bare row (no `=`) is PRESENT-but-valueless, never inherited. It
        // reports ONLY its keyed verdict — no second MalformedLine that
        // would doubt the roster and steal the shared wording.
        let meta = Meta::parse("seat.main=lead\nwork_dir.main\n");
        assert_eq!(
            meta.roster()[0].work_dir,
            RecordedWorkDir::Invalid("no value")
        );
        assert!(
            meta.anomalies()
                .iter()
                .all(|a| !matches!(a, Anomaly::MalformedLine { .. })),
            "{:?}",
            meta.anomalies()
        );
        // A doubled row destroys trust in both.
        let meta = Meta::parse("seat.main=lead\nwork_dir.main=/a\nwork_dir.main=/b\n");
        assert_eq!(
            meta.roster()[0].work_dir,
            RecordedWorkDir::Invalid("named more than once")
        );
        assert!(
            meta.anomalies().iter().any(|a| matches!(
                a,
                Anomaly::DuplicateKey { key, .. } if key == "work_dir.main"
            )),
            "{:?}",
            meta.anomalies()
        );
    }

    #[test]
    fn mixed_keyed_and_bare_seat_dir_rows_report_duplicate_without_doubting() {
        use super::{Anomaly, Meta, RecordedWorkDir};
        for text in [
            "seat.main=lead\nwork_dir.main=/a\nwork_dir.main\n",
            "seat.main=lead\nwork_dir.main\nwork_dir.main=/a\n",
            "seat.main=lead\nwork_dir.main\nwork_dir.main\n",
        ] {
            let meta = Meta::parse(text);
            assert_eq!(
                meta.roster()[0].work_dir,
                RecordedWorkDir::Invalid("named more than once"),
                "{text:?}"
            );
            assert!(
                meta.anomalies().iter().any(|a| matches!(
                    a,
                    Anomaly::DuplicateKey { key, .. } if key == "work_dir.main"
                )),
                "{text:?}: {:?}",
                meta.anomalies()
            );
            assert!(
                meta.anomalies()
                    .iter()
                    .all(|a| !crate::roster::roster_doubting(a)),
                "{text:?}: a seat-dir row must never doubt the roster"
            );
        }
    }

    #[test]
    fn resolve_seat_dir_inherits_or_returns_recorded() {
        use super::{Meta, resolve_seat_dir};
        let meta = Meta::parse("work_dir=/session\nseat.main=lead\n");
        assert_eq!(resolve_seat_dir(&meta, "main"), Ok("/session".to_owned()));
        // An unknown slot FAILS CLOSED: absence means a known seat with no
        // row, never a typo inheriting the session dir.
        assert_eq!(
            resolve_seat_dir(&meta, "spawned.9"),
            Err("no seat is recorded for slot 'spawned.9' — \
                 refusing a directory for an unknown seat."
                .to_owned())
        );
        let meta = Meta::parse("work_dir=/session\nseat.main=lead\nwork_dir.main=/w/t\n");
        assert_eq!(resolve_seat_dir(&meta, "main"), Ok("/w/t".to_owned()));
        // No session scalar either: the process default, never an error.
        let meta = Meta::parse("seat.main=lead\n");
        assert_eq!(resolve_seat_dir(&meta, "main"), Ok(".".to_owned()));
    }

    #[test]
    fn resolve_seat_dir_refuses_invalid_with_the_owned_reason() {
        use super::{Meta, resolve_seat_dir};
        let meta = Meta::parse("work_dir=/session\nseat.main=lead\nwork_dir.main=relative\n");
        assert_eq!(
            resolve_seat_dir(&meta, "main"),
            Err(
                "work_dir.main is present but unusable (not an absolute path) — \
                 restore the recorded path or retire the seat."
                    .to_owned()
            )
        );
    }

    #[test]
    fn raw_seat_work_dir_judges_bytes_before_any_rebuild() {
        use super::raw_seat_work_dir;
        assert_eq!(raw_seat_work_dir(b"seat.main=lead\n", "main"), Ok(None));
        assert_eq!(
            raw_seat_work_dir(b"work_dir.main=/w\n", "main"),
            Ok(Some("/w".to_owned()))
        );
        // CRLF: the carriage return is line ending, not value.
        assert_eq!(
            raw_seat_work_dir(b"work_dir.main=/w\r\n", "main"),
            Ok(Some("/w".to_owned()))
        );
        // The scalar and near-miss keys never match a slotted scan.
        assert_eq!(
            raw_seat_work_dir(b"work_dir=/s\nwork_dir.mainx=/x\n", "main"),
            Ok(None)
        );
        for (bytes, detail) in [
            (
                &b"work_dir.main=/a\nwork_dir.main=/b\n"[..],
                "named more than once",
            ),
            (&b"work_dir.main=\n"[..], "empty value"),
            (&b"work_dir.main\n"[..], "no value"),
            (&b"work_dir.main=relative\n"[..], "not an absolute path"),
            (&b"work_dir.main=/x/\x07/y\n"[..], "control characters"),
            (&b"work_dir.main=/x/\xff\n"[..], "not UTF-8"),
        ] {
            assert_eq!(
                raw_seat_work_dir(bytes, "main"),
                Err(format!(
                    "work_dir.main is present but unusable ({detail}) — \
                     restore the recorded path or retire the seat."
                )),
                "{bytes:?}"
            );
        }
    }

    /// A scratch root, removed on drop.
    struct Scratch(std::path::PathBuf);
    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("ae-meta-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch");
            Self(dir)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn provenance_follows_the_row_never_the_value() {
        use super::{SeatProvenance, select_seat_target};
        let session = std::path::PathBuf::from("/session");
        // Absence inherits.
        let inherited = select_seat_target(None, session.clone());
        assert_eq!(inherited.provenance, SeatProvenance::Inherited);
        assert_eq!(inherited.canonical, session);
        // A recorded path is explicit — even textually equal to the default.
        let explicit =
            select_seat_target(Some(std::path::PathBuf::from("/session")), session.clone());
        assert_eq!(explicit.provenance, SeatProvenance::Explicit);
        assert_eq!(explicit.canonical, session);
    }

    #[test]
    fn containment_compares_components_never_spellings() {
        use super::contained_in;
        use std::path::Path;
        // At the root and under it.
        assert!(contained_in(Path::new("/ae"), Path::new("/ae")));
        assert!(contained_in(Path::new("/ae/sessions/x"), Path::new("/ae")));
        // Siblings, parents and suffix collisions are outside.
        assert!(!contained_in(Path::new("/ae2"), Path::new("/ae")));
        assert!(!contained_in(Path::new("/ae"), Path::new("/ae/sessions")));
        assert!(!contained_in(Path::new("/other"), Path::new("/ae")));
    }

    #[test]
    fn the_typed_resolver_shares_the_string_wordings() {
        use super::{Meta, SeatProvenance, resolve_seat_target};
        let session = std::path::PathBuf::from("/session");
        // Unknown slot: the one unknown-seat wording.
        let meta = Meta::parse("seat.main=lead\n");
        assert_eq!(
            resolve_seat_target(&meta, "worker.9", session.clone()),
            Err("no seat is recorded for slot 'worker.9' — refusing a directory for an unknown seat."
                .to_owned())
        );
        // Missing row inherits with the Inherited tag.
        let target = resolve_seat_target(&meta, "main", session.clone()).unwrap();
        assert_eq!(target.provenance, SeatProvenance::Inherited);
        assert_eq!(target.canonical, session);
        // Recorded row carries through with the Explicit tag.
        let meta = Meta::parse("seat.main=lead\nwork_dir.main=/w/target\n");
        let target = resolve_seat_target(&meta, "main", session).unwrap();
        assert_eq!(target.provenance, SeatProvenance::Explicit);
        assert_eq!(target.canonical, std::path::PathBuf::from("/w/target"));
        // Unusable row refuses with the shared row wording.
        let meta = Meta::parse("seat.main=lead\nwork_dir.main=\n");
        assert!(
            resolve_seat_target(&meta, "main", std::path::PathBuf::from("/s"))
                .unwrap_err()
                .starts_with("work_dir.main is present but unusable")
        );
    }

    #[test]
    fn record_refuses_non_local_legacy_and_unprovable_before_any_write() {
        use super::{Meta, record_seat_target};
        let scratch = Scratch::new("record");
        let state = scratch.0.join("state");
        std::fs::create_dir(&state).unwrap();
        let target = scratch.0.join("target");
        std::fs::create_dir(&target).unwrap();
        // Managed mode refuses.
        let meta = Meta::parse("mode=git\nseat.spawned.0=scout\n");
        assert_eq!(
            record_seat_target(&meta, "spawned.0", "/x", &state, &scratch.0),
            Err("explicit seat targets record on local sessions only — this session's mode is 'git'."
                .to_owned())
        );
        // Missing mode refuses.
        let meta = Meta::parse("seat.spawned.0=scout\n");
        assert!(
            record_seat_target(&meta, "spawned.0", "/x", &state, &scratch.0)
                .unwrap_err()
                .contains("records no mode")
        );
        // Legacy v1 rows refuse.
        let meta = Meta::parse("mode=local\nagent.main=fable5:lead\nseat.spawned.0=scout\n");
        assert!(
            record_seat_target(
                &meta,
                "spawned.0",
                target.to_str().unwrap(),
                &state,
                &scratch.0
            )
            .unwrap_err()
            .contains("legacy agent.<slot> row")
        );
        // A file is not a target.
        let file = scratch.0.join("file");
        std::fs::write(&file, "x").unwrap();
        let meta = Meta::parse("mode=local\nseat.spawned.0=scout\n");
        assert!(
            record_seat_target(
                &meta,
                "spawned.0",
                file.to_str().unwrap(),
                &state,
                &scratch.0
            )
            .unwrap_err()
            .contains("is not a directory")
        );
        // A missing target refuses as absent.
        assert!(
            record_seat_target(
                &meta,
                "spawned.0",
                scratch.0.join("missing").to_str().unwrap(),
                &state,
                &scratch.0
            )
            .unwrap_err()
            .contains("is not there")
        );
        // A good target records its canonical spelling.
        let stored = record_seat_target(
            &meta,
            "spawned.0",
            target.to_str().unwrap(),
            &state,
            &scratch.0,
        )
        .unwrap();
        assert!(std::path::Path::new(&stored).is_absolute(), "{stored}");
        // A relative dir joins the INVOKER cwd, not the session.
        let stored = record_seat_target(&meta, "spawned.0", "target", &state, &scratch.0).unwrap();
        assert!(stored.ends_with("target"), "{stored}");
    }

    #[test]
    fn record_containment_binds_explicit_targets_to_outside_ae_state() {
        use super::{Meta, record_seat_target};
        let scratch = Scratch::new("contain");
        let state = scratch.0.join("state");
        std::fs::create_dir_all(state.join("sessions")).unwrap();
        let meta = Meta::parse("mode=local\nseat.spawned.0=scout\n");
        // At the root and under it refuse.
        for target in [state.clone(), state.join("sessions")] {
            assert!(
                record_seat_target(
                    &meta,
                    "spawned.0",
                    target.to_str().unwrap(),
                    &state,
                    &scratch.0
                )
                .unwrap_err()
                .contains("outside ae state"),
                "{target:?}"
            );
        }
        // A suffix collision is outside and passes.
        let outside = scratch.0.join("state2");
        std::fs::create_dir(&outside).unwrap();
        assert!(
            record_seat_target(
                &meta,
                "spawned.0",
                outside.to_str().unwrap(),
                &state,
                &scratch.0
            )
            .is_ok()
        );
        // Absent origin .ae: no legacy subtree, ordinary targets pass.
        let origin = scratch.0.join("origin");
        std::fs::create_dir(&origin).unwrap();
        let meta = Meta::parse(&format!(
            "mode=local\norigin={}\nseat.spawned.0=scout\n",
            origin.display()
        ));
        assert!(
            record_seat_target(
                &meta,
                "spawned.0",
                outside.to_str().unwrap(),
                &state,
                &scratch.0
            )
            .is_ok()
        );
        // Present origin .ae contains.
        let ae = origin.join(".ae");
        std::fs::create_dir(&ae).unwrap();
        assert!(
            record_seat_target(&meta, "spawned.0", ae.to_str().unwrap(), &state, &scratch.0)
                .unwrap_err()
                .contains("origin .ae dir")
        );
        // Dangling origin .ae refuses as present-invalid, never absence.
        let origin2 = scratch.0.join("origin2");
        std::fs::create_dir(&origin2).unwrap();
        std::os::unix::fs::symlink(origin2.join("gone"), origin2.join(".ae")).unwrap();
        let meta = Meta::parse(&format!(
            "mode=local\norigin={}\nseat.spawned.0=scout\n",
            origin2.display()
        ));
        assert!(
            record_seat_target(
                &meta,
                "spawned.0",
                outside.to_str().unwrap(),
                &state,
                &scratch.0
            )
            .unwrap_err()
            .contains("present but unusable")
        );
    }

    #[test]
    fn a_non_utf8_canonical_is_no_row_value() {
        use super::canonical_row_value;
        use std::os::unix::ffi::OsStrExt as _;
        // Pure over the path: no filesystem, so this runs on every platform
        // even where no such name can exist on disk.
        let weird = std::path::Path::new(std::ffi::OsStr::from_bytes(b"/w/we\xffird"));
        assert_eq!(
            canonical_row_value("main", weird),
            Err("work_dir.main is present but unusable (not UTF-8) — \
                 restore the recorded path or retire the seat."
                .to_owned())
        );
        assert_eq!(
            canonical_row_value("main", std::path::Path::new("/w/ok")),
            Ok("/w/ok".to_owned())
        );
    }

    #[test]
    fn record_containment_follows_a_symlinked_origin_ae() {
        use super::{Meta, record_seat_target};
        let scratch = Scratch::new("symae");
        let state = scratch.0.join("state");
        std::fs::create_dir(&state).unwrap();
        let origin = scratch.0.join("origin");
        std::fs::create_dir(&origin).unwrap();
        let real_ae = scratch.0.join("real-ae");
        std::fs::create_dir(&real_ae).unwrap();
        std::os::unix::fs::symlink(&real_ae, origin.join(".ae")).unwrap();
        let meta = Meta::parse(&format!(
            "mode=local\norigin={}\nseat.spawned.0=scout\n",
            origin.display()
        ));
        let record = |target: &std::path::Path| {
            record_seat_target(
                &meta,
                "spawned.0",
                target.to_str().unwrap(),
                &state,
                &scratch.0,
            )
        };
        // Under the RESOLVED root: refuses as origin .ae.
        let under = real_ae.join("sub");
        std::fs::create_dir(&under).unwrap();
        assert!(record(&under).unwrap_err().contains("origin .ae dir"));
        // The symlinked spelling itself is contained too.
        assert!(
            record(&origin.join(".ae"))
                .unwrap_err()
                .contains("origin .ae dir")
        );
        // Elsewhere passes.
        let outside = scratch.0.join("outside");
        std::fs::create_dir(&outside).unwrap();
        assert!(record(&outside).is_ok());
    }

    #[test]
    fn record_refuses_empty_input_before_any_filesystem_read() {
        use super::{Meta, record_seat_target};
        // A bad state root would refuse first if empty were not checked
        // before every door read; the empty wording winning proves the
        // boundary. Pure over the meta: nothing is written, ever.
        let meta = Meta::parse("mode=local\nseat.spawned.0=scout\n");
        let text = "mode=local\nseat.spawned.0=scout\n";
        assert_eq!(
            record_seat_target(
                &meta,
                "spawned.0",
                "",
                std::path::Path::new("/no/such/state"),
                std::path::Path::new("/no/such/cwd"),
            ),
            Err(
                "work_dir.spawned.0 is present but unusable (empty value) — \
                 restore the recorded path or retire the seat."
                    .to_owned()
            )
        );
        assert_eq!(Meta::parse(text).anomalies(), meta.anomalies());
    }

    #[test]
    fn record_refuses_a_proven_but_non_utf8_destination_before_conversion() {
        use super::{Meta, record_seat_target};
        use std::os::unix::ffi::OsStrExt as _;
        let scratch = Scratch::new("nonutf8");
        let state = scratch.0.join("state");
        std::fs::create_dir(&state).unwrap();
        // UTF-8 spelling resolving to a non-UTF8 directory: the door proves
        // the place, but no row value may be produced from it.
        let weird = scratch.0.join(std::ffi::OsStr::from_bytes(b"we\xffird"));
        match std::fs::create_dir(&weird) {
            Ok(()) => {}
            #[cfg(target_os = "macos")]
            Err(error) if error.raw_os_error() == Some(92) => {
                // APFS cannot name a non-UTF8 directory (EILSEQ 92): no
                // record path can meet one here. The pure pin above carries
                // the policy; Linux CI covers this wiring.
                return;
            }
            Err(error) => panic!("non-UTF8 fixture failed unexpectedly: {error:?}"),
        }
        let link = scratch.0.join("link");
        std::os::unix::fs::symlink(&weird, &link).unwrap();
        let meta = Meta::parse("mode=local\nseat.spawned.0=scout\n");
        assert_eq!(
            record_seat_target(
                &meta,
                "spawned.0",
                link.to_str().unwrap(),
                &state,
                &scratch.0
            ),
            Err("work_dir.spawned.0 is present but unusable (not UTF-8) — \
                 restore the recorded path or retire the seat."
                .to_owned())
        );
    }

    #[test]
    fn use_re_proves_the_stored_path_and_refuses_a_moved_destination() {
        use super::{SeatProvenance, SeatTarget, check_seat_target_use};
        let scratch = Scratch::new("use");
        let real = scratch.0.join("real");
        std::fs::create_dir(&real).unwrap();
        let canonical = crate::doors::canonical_strict_dir(&real).unwrap();
        let target = SeatTarget {
            canonical: canonical.clone(),
            provenance: SeatProvenance::Explicit,
        };
        // Unmoved: passes.
        assert_eq!(check_seat_target_use("main", &target), Ok(()));
        // Deleted: gone.
        std::fs::remove_dir(&real).unwrap();
        assert!(
            check_seat_target_use("main", &target)
                .unwrap_err()
                .contains("recorded target gone")
        );
        // Replaced by a file at the same path: no longer a directory.
        std::fs::write(&real, "x").unwrap();
        assert!(
            check_seat_target_use("main", &target)
                .unwrap_err()
                .contains("no longer a directory")
        );
        // Stored path swapped to another destination: the re-resolved
        // canonical no longer equals the stored one, so use refuses rather
        // than following into the surprise target.
        std::fs::remove_file(&real).unwrap();
        let elsewhere = scratch.0.join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &real).unwrap();
        assert!(
            check_seat_target_use("main", &target)
                .unwrap_err()
                .contains("destination changed")
        );
    }

    #[test]
    fn pane_start_inherits_verbatim_and_proves_explicit_rows() {
        use super::{Meta, checked_pane_start_dir};
        let scratch = Scratch::new("pane");
        let target = scratch.0.join("target");
        std::fs::create_dir(&target).unwrap();
        let canonical = crate::doors::canonical_strict_dir(&target).unwrap();
        let session = scratch.0.join("session");
        std::fs::create_dir(&session).unwrap();
        let session_canonical = crate::doors::canonical_strict_dir(&session).unwrap();
        // Missing row: the proven session spelling passes through
        // byte-identical — same text back, not the canonical form.
        let meta = Meta::parse("seat.main=lead\nwork_dir=/session\n");
        let spelling = session.to_str().unwrap().to_owned();
        assert_eq!(
            checked_pane_start_dir(&meta, "main", &spelling, session_canonical.clone()),
            Ok(spelling)
        );
        // Explicit row: proven and returned canonical. The row carries the
        // canonical spelling, as the record helper stores it.
        let meta = Meta::parse(&format!(
            "seat.main=lead\nwork_dir.main={}\n",
            canonical.display()
        ));
        assert_eq!(
            checked_pane_start_dir(&meta, "main", "/session", session_canonical.clone()),
            Ok(canonical.display().to_string())
        );
        // Unusable row: the shared wording.
        let meta = Meta::parse("seat.main=lead\nwork_dir.main=\n");
        assert!(
            checked_pane_start_dir(&meta, "main", "/session", session_canonical)
                .unwrap_err()
                .starts_with("work_dir.main is present but unusable")
        );
        // Unknown slot refuses.
        let meta = Meta::parse("seat.main=lead\n");
        assert!(
            checked_pane_start_dir(
                &meta,
                "worker.9",
                "/session",
                std::path::PathBuf::from("/s")
            )
            .unwrap_err()
            .contains("unknown seat")
        );
    }

    #[test]
    fn pane_start_re_proves_an_inherited_session_dir_with_its_own_wording() {
        use super::{Meta, checked_pane_start_dir};
        let scratch = Scratch::new("inherit");
        let meta = Meta::parse("seat.main=lead\nwork_dir=/session\n");
        // Gone: names the session dir and its remedy — no row, no retire.
        let gone = scratch.0.join("gone");
        let err = checked_pane_start_dir(
            &meta,
            "main",
            gone.to_str().unwrap(),
            std::path::PathBuf::from("/held"),
        )
        .unwrap_err();
        assert!(
            err.contains("session directory") && err.contains("is gone"),
            "{err}"
        );
        assert!(!err.contains("retire the seat"), "{err}");
        // Replaced by a file: same provenance-correct shape.
        let file = scratch.0.join("file");
        std::fs::write(&file, "x").unwrap();
        let held = crate::doors::canonical_strict_dir(&scratch.0).unwrap();
        let err = checked_pane_start_dir(&meta, "main", file.to_str().unwrap(), held).unwrap_err();
        assert!(
            err.contains("session directory") && err.contains("is not a directory"),
            "{err}"
        );
        // Same path, other directory: a DIRECT path replaced by a symlink
        // elsewhere — no alias in the baseline, so this is distinct from
        // the alias-retarget pin below.
        let swap = scratch.0.join("swap");
        std::fs::create_dir(&swap).unwrap();
        let held = crate::doors::canonical_strict_dir(&swap).unwrap();
        let elsewhere = scratch.0.join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::fs::remove_dir(&swap).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &swap).unwrap();
        let err = checked_pane_start_dir(&meta, "main", swap.to_str().unwrap(), held).unwrap_err();
        assert!(
            err.contains("no longer resolves to its recorded place"),
            "{err}"
        );
    }

    #[test]
    fn pane_start_checks_the_returned_spelling_not_just_the_baseline() {
        use super::{Meta, checked_pane_start_dir};
        // Alias A→real1 is the baseline; retargeting A→real2 while real1
        // lives must refuse: checking the held canonical alone would pass
        // while the pane entered real2.
        let scratch = Scratch::new("aliascheck");
        let real1 = scratch.0.join("real1");
        let real2 = scratch.0.join("real2");
        std::fs::create_dir(&real1).unwrap();
        std::fs::create_dir(&real2).unwrap();
        let alias = scratch.0.join("a");
        std::os::unix::fs::symlink(&real1, &alias).unwrap();
        let held = crate::doors::canonical_strict_dir(&alias).unwrap();
        std::fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(&real2, &alias).unwrap();
        let meta = Meta::parse("seat.main=lead\nwork_dir=/session\n");
        let err = checked_pane_start_dir(&meta, "main", alias.to_str().unwrap(), held).unwrap_err();
        assert!(
            err.contains("no longer resolves to its recorded place"),
            "{err}"
        );
    }

    // B2-spawn U1: omission is not a target and proves nothing.
    #[test]
    fn plan_spawn_target_omitted_means_no_explicit_target() {
        use super::plan_spawn_target;
        use std::path::Path;
        let root = Path::new("/state");
        assert_eq!(plan_spawn_target(None, Some(root)), Ok(None));
        // None skips even the state-root proof: nothing to contain.
        assert_eq!(plan_spawn_target(None, None), Ok(None));
    }

    // B2-spawn U2: a non-UTF8 spelling refuses before any door read. Pure:
    // no fixture directory exists, so a door touch would fail, not pass.
    #[test]
    fn plan_spawn_target_refuses_a_non_utf8_spelling_up_front() {
        use super::plan_spawn_target;
        use std::os::unix::ffi::OsStrExt as _;
        let weird = std::path::Path::new(std::ffi::OsStr::from_bytes(b"/w/we\xffird"));
        let root = std::path::Path::new("/state");
        assert_eq!(
            plan_spawn_target(Some(weird), Some(root)),
            Err("the spelled target \"/w/we\\xFFird\" is not UTF-8 — \
                 explicit seat targets must be UTF-8."
                .to_owned())
        );
    }

    // B2-spawn U3: Some without a state root cannot prove containment.
    #[test]
    fn plan_spawn_target_refuses_some_without_a_state_root() {
        use super::plan_spawn_target;
        use std::path::Path;
        assert_eq!(
            plan_spawn_target(Some(Path::new("/w/t")), None),
            Err("no ae state root is available — an explicit target cannot \
                 prove containment."
                .to_owned())
        );
    }

    // B2-spawn U11: the explicit-row selector + JIT use the recorded place.
    #[test]
    fn explicit_jit_starts_from_the_recorded_place_or_refuses() {
        use super::{Meta, checked_explicit_pane_start_dir, explicit_seat_row};
        let scratch = Scratch::new("explicit-jit");
        let target = scratch.0.join("repo");
        std::fs::create_dir(&target).unwrap();
        let canonical = std::fs::canonicalize(&target).unwrap();
        let meta = Meta::parse(&format!(
            "mode=local\nseat.spawned.0=scout\nwork_dir.spawned.0={}\n",
            canonical.display()
        ));
        assert_eq!(explicit_seat_row(&meta, "spawned.0"), Ok(canonical.clone()));
        assert_eq!(
            checked_explicit_pane_start_dir(&meta, "spawned.0"),
            Ok(canonical.display().to_string())
        );
        // The recorded node removed, meta unchanged: the JIT refuses with
        // the shared row wording instead of following the ghost.
        std::fs::remove_dir(&target).unwrap();
        assert_eq!(
            checked_explicit_pane_start_dir(&meta, "spawned.0"),
            Err(
                "work_dir.spawned.0 is present but unusable (recorded target \
                 gone) — restore the recorded path or retire the seat."
                    .to_owned()
            )
        );
        // No row is a refusal, never an inheritance: the explicit JIT must
        // not start a seat whose target was never recorded.
        let bare = Meta::parse("mode=local\nseat.spawned.0=scout\n");
        assert_eq!(
            explicit_seat_row(&bare, "spawned.0"),
            Err("no work_dir.spawned.0 row is recorded — the seat has no \
                 explicit target."
                .to_owned())
        );
        // A present-but-unusable row refuses with its owned reason.
        let bad = Meta::parse("mode=local\nseat.spawned.0=scout\nwork_dir.spawned.0=relative\n");
        assert_eq!(
            explicit_seat_row(&bad, "spawned.0"),
            Err(
                "work_dir.spawned.0 is present but unusable (not an absolute \
                 path) — restore the recorded path or retire the seat."
                    .to_owned()
            )
        );
    }
}
