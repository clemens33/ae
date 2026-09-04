//! The session `meta` file, and nothing else.
//!
//! Wired only after the seats ratified those three rows. Before that this
//! module did not exist: the digest's `mode`/`origin`/`work_dir`/`goal` and its
//! `agents[]` roster sat unread rather than being guessed from the bash writer.
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

/// The file this module reads, inside a session directory.
pub const FILE: &str = "meta";

/// The roster key prefixes.
const ROSTER_PREFIX: &str = "agent.";
/// Two selector keys, matched literally where they are absorbed.
const SERVER_KEY: &str = "tmux_server";
const SERVER_KIND_KEY: &str = "tmux_server_kind";
const ROSTER_BIN_PREFIX: &str = "agent_bin.";
/// Identity schema v2 (alias-free): the seat's NAME, its execution PROFILE and
/// the harness's own conversation id live under three keys instead of one
/// `alias:name:sid` value.
const SEAT_PREFIX: &str = "seat.";
const PROFILE_PREFIX: &str = "profile.";
const HARNESS_SESSION_PREFIX: &str = "harness_session.";
/// `schema=<n>` — the identity schema the writer used.
const SCHEMA_KEY: &str = "schema";

/// One agent, as the roster records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterEntry {
    /// `main` / `worker.<n>` / `spawned.<n>` — the key's suffix.
    pub slot: String,
    /// The agent's NAME — its identity.
    pub name: String,
    /// The execution profile (`profile.<slot>`).
    pub profile: Option<String>,
    /// The harness's own conversation id (`harness_session.<slot>`), where the
    /// roster carries one.
    pub harness_session: Option<String>,
    /// `agent_bin.<slot>` — the recorded binary, where the meta carries one.
    pub binary: Option<String>,
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
/// fact, normalized from the two-key legacy form.
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
}

/// How often a slot has been claimed by a `seat.<slot>` KEY: a second claim —
/// keyed or bare — is a seat in doubt.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SlotClaim {
    slot: String,
    claims: usize,
}

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
    /// The retired v1 roster row `agent.<slot>`, which this ae does not read
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
    /// The frozen executable version recorded when ae created the session.
    ae_version: Option<String>,
    roster: Vec<RosterEntry>,
    /// `agent_bin.<slot>` values whose `agent.<slot>` / `seat.<slot>` has not
    /// been read yet.
    pending_binaries: Vec<(String, String)>,
    /// Identity v2 metadata rows (`profile.<slot>`, `harness_session.<slot>`)
    /// read before their `seat.<slot>` — same rule as the binaries.
    pending_profiles: Vec<PendingRow>,
    pending_harness: Vec<PendingRow>,
    /// Every `seat.<slot>` KEY met so far — `=` or not, valid or not, first or
    /// repeated.
    claims: Vec<SlotClaim>,
    /// v2 names already found on more than one seat: no later seat may take them.
    doubtful_names: Vec<String>,
    /// The raw `schema=` value, where the writer recorded one.
    schema: Option<String>,
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
        let text = fs::read_to_string(dir.join(FILE))?;
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
            // A repeated selector key is AMBIGUOUS, which is a
            // stronger statement than invalidation: the flag survives
            // even if the repeats agreed.
            SERVER_KEY | SERVER_KIND_KEY => self.server_duplicated = true,
            _ => {
                if let Some(slot) = key.strip_prefix(ROSTER_BIN_PREFIX) {
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
            // The selector family is the one exception: these two are read and
            // normalized rather than tolerated-and-ignored.
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

    /// Record a retired v1 roster row: it names no seat this ae will serve.
    ///
    /// The row is REPORTED rather than dropped. A silent drop would render a
    /// legacy session identically to a healthy one whose roster is empty, and
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
        self.roster.push(RosterEntry {
            slot: slot.to_owned(),
            name: value.to_owned(),
            profile,
            harness_session,
            binary,
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
        for list in [&mut self.pending_profiles, &mut self.pending_harness] {
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

    /// `profile.<slot>` / `harness_session.<slot>`: attaches to its seat, or
    /// waits for one that has not been read yet.
    fn set_metadata(&mut self, which: Metadata, slot: &str, key: &str, value: &str, line: usize) {
        // Ownership FIRST, value second: an empty row on a seat is absent
        // metadata, not a missing seat.
        if let Some(existing) = self.roster.iter_mut().find(|entry| entry.slot == slot) {
            let value = (!value.is_empty()).then(|| value.to_owned());
            match which {
                Metadata::Profile => existing.profile = value,
                Metadata::HarnessSession => existing.harness_session = value,
            }
            return;
        }
        let row = PendingRow {
            slot: slot.to_owned(),
            value: value.to_owned(),
            key: key.to_owned(),
            line,
            duplicated: false,
        };
        match which {
            Metadata::Profile => self.pending_profiles.push(row),
            Metadata::HarnessSession => self.pending_harness.push(row),
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
            // Absent kind: the legacy one-key form, which named a server.
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
/// `meta.lock`, by rewriting a temp file and renaming it over — the frozen
/// `ae_meta_set`/`ae_meta_unset`, made durable: the temp is synced before the
/// rename, and the directory is synced after it, because a synced inode
/// behind an unsynced directory entry is a meta that can revert on a crash
/// while the event announcing it survives.
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
/// core side of `_meta-init`.
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

/// The whole meta, published OVER whatever is there — [`init`]'s sibling for a
/// caller that MEANS to replace an existing document.
///
/// # Errors
///
/// [`RewriteError::NotWritten`] when the lock is not acquired or any write,
/// sync or rename fails; [`RewriteError::Unknown`] when the rename returned but
/// the directory sync did not.
pub fn replace(dir: &Path, content: &str) -> Result<(), RewriteError> {
    let path = dir.join(FILE);
    let _held = crate::state::acquire(&dir.join(LOCK), crate::state::LOCK_WAIT)
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
/// [`crate::state::LOCK_WAIT`], or the lock file not openable.
pub fn lock(dir: &Path) -> io::Result<fs::File> {
    crate::state::acquire(&dir.join(LOCK), crate::state::LOCK_WAIT)
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
    publish_bytes(dir, &dir.join(FILE), content.as_bytes())
}

/// The staged BASE-FACTS document `_meta-init` consumes: the `key=value` lines
/// bash wrote for a session before its roster block existed, read from `path`.
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
        // hide the degradation the reader reports.
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

    use super::{Anomaly, Meta, Selector, ServerSelector};
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
        // The human ruling: this ae does not read `agent.<slot>` into a seat,
        // and a legacy session is one to start over from. The row is REPORTED
        // rather than dropped — a silent drop would render a legacy session
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
}
