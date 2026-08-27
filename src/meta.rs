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

/// One agent, as the roster records it (SC-405c + SC-1207b).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterEntry {
    /// `main` / `worker.<n>` / `spawned.<n>` — the key's suffix.
    pub slot: String,
    /// The configured alias.
    pub alias: String,
    /// The display name.
    pub name: String,
    /// The captured provider session id, where the roster carries one.
    pub session_id: Option<String>,
    /// `agent_bin.<slot>` — the recorded binary, where the meta carries one.
    pub binary: Option<String>,
}

impl RosterEntry {
    /// The `alias:name` an agent is addressed by — SC-509's `ref`.
    #[must_use]
    pub fn reference(&self) -> String {
        format!("{}:{}", self.alias, self.name)
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
    /// SC-405c — a roster value that is not `alias:name[:session-id]`.
    MalformedRosterEntry {
        /// The key as written.
        key: String,
        /// 1-based line number.
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
    /// `agent_bin.<slot>` values whose `agent.<slot>` has not been read yet.
    /// A slot that never gets one simply never becomes an agent.
    pending_binaries: Vec<(String, String)>,
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
                meta.anomalies.push(Anomaly::MalformedLine { line });
                continue;
            };
            if seen.iter().any(|previous| previous == key) {
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
                } else if let Some(slot) = key.strip_prefix(ROSTER_PREFIX) {
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
            _ => {
                if let Some(slot) = key.strip_prefix(ROSTER_BIN_PREFIX) {
                    self.set_binary(slot, value);
                } else if let Some(slot) = key.strip_prefix(ROSTER_PREFIX) {
                    self.absorb_roster(key, slot, value, line);
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
        self.roster.push(RosterEntry {
            slot: slot.to_owned(),
            alias: alias.to_owned(),
            name: name.to_owned(),
            session_id: session_id.filter(|id| !id.is_empty()),
            binary,
        });
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

    /// Everything this reader met and is not authorised to interpret.
    ///
    /// Non-empty means the session is degraded-with-reason until SC-405d and
    /// SC-405e close.
    #[must_use]
    pub fn anomalies(&self) -> &[Anomaly] {
        &self.anomalies
    }
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
    let temp = dir.join(format!("{FILE}.tmp.{}", std::process::id()));
    let staged = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp)?;
        file.write_all(next.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, &path)
    })();
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
        assert_eq!(roster[0].alias, "claude");
        assert_eq!(roster[0].name, "lead");
        assert_eq!(roster[0].session_id.as_deref(), Some("e795c9e9"));
        assert_eq!(roster[0].binary.as_deref(), Some("claude"));
        assert_eq!(roster[0].reference(), "claude:lead");
        // The slot name itself contains a dot; only the PREFIX is stripped.
        assert_eq!(roster[1].slot, "worker.0");
        assert_eq!(roster[1].session_id, None);
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
}
