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
use std::io;
use std::path::Path;

/// The file this module reads, inside a session directory.
pub const FILE: &str = "meta";

/// The roster key prefixes (SC-405c). The four context keys of SC-405b are
/// matched literally where they are absorbed, next to their fields.
const ROSTER_PREFIX: &str = "agent.";
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
    roster: Vec<RosterEntry>,
    /// `agent_bin.<slot>` values whose `agent.<slot>` has not been read yet.
    /// A slot that never gets one simply never becomes an agent.
    pending_binaries: Vec<(String, String)>,
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
        Ok(Self::parse(&fs::read_to_string(dir.join(FILE))?))
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

#[cfg(test)]
mod tests {
    use super::{Anomaly, Meta};

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
    fn sc_405e_every_context_key_invalidates_the_same_way() {
        // One field tested is one field proven. All four of SC-405b's keys go
        // through the same refusal, so all four are pinned to it — cargo-mutants
        // walked past three of them while `goal` alone was covered.
        type Accessor = fn(&Meta) -> Option<&str>;
        let read: [(&str, Accessor); 4] = [
            ("mode", Meta::mode),
            ("origin", Meta::origin),
            ("work_dir", Meta::work_dir),
            ("goal", Meta::goal),
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
