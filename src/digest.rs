//! The `ae list --json` document.
//!
//! One bad session degrades its own entry and the document always closes. That
//! is structural rather than guarded: a [`Digest`] is built first and rendered
//! second, and [`crate::json::Value::render`] cannot fail, so no code path emits
//! half a document.
//!
//! A member whose own source was complete is rendered whatever its value, and
//! `degraded` is aggregate visibility only: one failed source never erases a
//! fact another established. `null` is the spelling of a complete, legitimate
//! empty answer, never of an unreadable input.

use crate::attention::Reason;
use crate::json::Value;
use crate::time::Timestamp;

/// The `schema_version` every SUCCESSOR digest publishes.
///
/// Version 2, because `sessions[].status` gained `unknown`. A new value in an
/// existing field is a consumer-visible contract change even though the name,
/// type and position are unchanged: a consumer that gated on the two-value
/// domain breaks on the third.
pub const SCHEMA_VERSION: i64 = 2;

/// Whether a session is running, stopped, or not established either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The session is live.
    Running,
    /// The session is stopped history.
    Stopped,
    /// Liveness was NOT ESTABLISHED - the recorded server was unreachable, the
    /// query failed, or ownership evidence was missing. It is NOT stopped and
    /// NOT absence.
    Unknown,
}

impl Status {
    /// Every status, in the group order: running, unknown, stopped.
    ///
    /// An array literal is NOT exhaustiveness-checked, which is how `filters.rs`
    /// once enumerated the variants per scope, went on compiling, and silently
    /// dropped a new state from every listing. The guard is
    /// `the_status_list_holds_every_variant_exactly_once`, which answers each
    /// variant through a `match`: a fourth variant fails to BUILD the suite.
    pub const ALL: [Self; 3] = [Self::Running, Self::Unknown, Self::Stopped];

    /// The spelling example carries (`"status": "running"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Stopped => "stopped",
            Self::Unknown => "unknown",
        }
    }
}

/// One entry of a session's `agents[]`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgentEntry {
    /// `ref` — the display ref an agent is addressed by: `alias:name` for a v1
    /// roster row, the bare name for an identity-v2 row (`RosterEntry::reference`).
    pub reference: String,
    /// The configured alias (v1) / profile (v2) — schema 2 still publishes it.
    pub alias: String,
    /// The display name.
    pub name: String,
    /// The captured tool session id, where one exists.
    pub session_id: Option<String>,
    /// Whether the agent's pane is alive, three-valued.
    ///
    /// `Some` carries a positively established fact — an exact association to a
    /// live pane, or a proof that this roster agent has none. `None` is
    /// `unknown`, emitted as JSON `null` rather than omitted, so a consumer
    /// gating on presence never has to tell absent from unknown.
    pub alive: Option<bool>,
    /// The agent's declared work state.
    pub state: Option<String>,
    /// "each agent's `reason` is its own contribution" to the session marker.
    pub reason: Option<Reason>,
}

/// Frozen's placeholder for an agent whose session id is absent or unresolved.
const ABSENT_SESSION_ID: &str = "-";

/// The literal an unresolved id is recorded as, before capture succeeds.
const PENDING_SESSION_ID: &str = "pending";

/// Frozen's short-id width, in CHARACTERS (ae@72c7293:3143, 3158).
const SHORT_SESSION_ID_CHARS: usize = 8;

impl AgentEntry {
    /// The DISPLAY session id — frozen's short form, shared by BOTH surfaces.
    #[must_use]
    pub(crate) fn display_session_id(&self) -> &str {
        let Some(id) = self.session_id.as_deref() else {
            return ABSENT_SESSION_ID;
        };
        if id.is_empty() || id == PENDING_SESSION_ID {
            return ABSENT_SESSION_ID;
        }
        match id.char_indices().nth(SHORT_SESSION_ID_CHARS) {
            // The ninth character's offset, so the slice is the first eight —
            // and it is a char boundary by construction, never a byte cut.
            Some((end, _)) => &id[..end],
            None => id,
        }
    }

    /// This agent as object.
    #[must_use]
    pub fn to_json(&self) -> Value {
        self.to_json_with_event_knowledge(true, false)
    }

    /// This agent as object when the event source may be incomplete.
    fn to_json_with_event_knowledge(
        &self,
        events_complete: bool,
        runtime_dead_established: bool,
    ) -> Value {
        let mut fields = vec![
            ("ref".to_owned(), Value::str(&self.reference)),
            ("alias".to_owned(), Value::str(&self.alias)),
            ("name".to_owned(), Value::str(&self.name)),
        ];
        // Unconditional: an `AgentEntry` exists only because the roster was READ
        // (membership is roster-defined), so the
        // question "was this member readable" is answered yes by the entry's
        fields.push((
            "session_id".to_owned(),
            Value::str(self.display_session_id()),
        ));
        // Present even when null — see the field's own docs.
        fields.push((
            "alive".to_owned(),
            self.alive.map_or(Value::Null, Value::Bool),
        ));
        if events_complete {
            push_str_or_null(&mut fields, "state", self.state.as_deref());
        }
        // `reason: null` means no contribution exists only when
        // all inputs that could add or supersede one were read. A roster entry
        // proves membership, not that completeness. A runtime `dead` is the
        if events_complete
            || (runtime_dead_established && self.reason.is_some_and(Reason::is_severity_maximum))
        {
            fields.push((
                "reason".to_owned(),
                self.reason
                    .map_or(Value::Null, |reason| Value::str(reason.as_str())),
            ));
        }
        Value::Obj(fields)
    }
}

/// Which sources settled each optional digest member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FactState {
    /// The source settled this member, including a legitimate empty value.
    Complete,
    /// The source lost data that could affect this member.
    Incomplete,
}

impl FactState {
    /// Whether this member may render its legitimate empty value.
    #[must_use]
    pub(crate) const fn is_complete(self) -> bool {
        matches!(self, Self::Complete)
    }

    /// Turn a source-completeness predicate into its provenance state.
    #[must_use]
    pub(crate) const fn from_complete(complete: bool) -> Self {
        if complete {
            Self::Complete
        } else {
            Self::Incomplete
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RenderKnowledge {
    /// The corresponding `meta` member was read completely.
    pub mode: FactState,
    /// The corresponding `meta` member was read completely.
    pub origin: FactState,
    /// The corresponding `meta` member was read completely.
    pub work_dir: FactState,
    /// The corresponding `meta` member was read completely.
    pub goal: FactState,
    /// The event stream was complete for event-derived members.
    pub events: FactState,
    /// The roster was fully enumerable, including a readable empty roster.
    pub roster: FactState,
}

impl RenderKnowledge {
    const fn complete() -> Self {
        Self {
            mode: FactState::Complete,
            origin: FactState::Complete,
            work_dir: FactState::Complete,
            goal: FactState::Complete,
            events: FactState::Complete,
            roster: FactState::Complete,
        }
    }

    const fn unavailable() -> Self {
        Self {
            mode: FactState::Incomplete,
            origin: FactState::Incomplete,
            work_dir: FactState::Incomplete,
            goal: FactState::Incomplete,
            events: FactState::Incomplete,
            roster: FactState::Incomplete,
        }
    }
}

/// One entry of the digest's `sessions[]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEntry {
    /// The session's name.
    pub name: String,
    /// Running or stopped.
    pub status: Status,
    /// The copy mode the session was started in.
    pub mode: Option<String>,
    /// Where the session came from.
    pub origin: Option<String>,
    /// The working directory its agents run in.
    pub work_dir: Option<String>,
    /// The session's one-line objective.
    pub goal: Option<String>,
    /// The ae version captured when this session was created.
    pub ae_version: Option<String>,
    /// When that goal was last set — "age it for staleness".
    pub goal_set_epoch: Option<i64>,
    /// The session's live git branch.
    pub branch: Option<String>,
    /// When the session last did anything ae could see.
    pub last_active_epoch: Option<i64>,
    /// The session-level rollup. `None` means nothing needs a human.
    pub attention: Option<Reason>,
    /// The session's agents.
    pub agents: Vec<AgentEntry>,
    /// Whether this entry suffered ACTUAL read/parse loss.
    pub degraded: bool,
    /// Per-member source completeness. It stays private so callers cannot make
    /// an aggregate `degraded` flag masquerade as member provenance.
    knowledge: RenderKnowledge,
    /// Roster references whose `Dead` came from a complete, typed runtime
    /// hand-in. Ledger-derived `Dead` never enters this set: a skipped later
    /// record may clear or supersede it.
    established_runtime_dead_agents: Vec<String>,
}

impl SessionEntry {
    /// A session entry with nothing established beyond its identity.
    #[must_use]
    pub fn new<N: Into<String>>(name: N, status: Status) -> Self {
        Self {
            name: name.into(),
            status,
            mode: None,
            origin: None,
            work_dir: None,
            goal: None,
            ae_version: None,
            goal_set_epoch: None,
            branch: None,
            last_active_epoch: None,
            attention: None,
            agents: Vec::new(),
            degraded: false,
            knowledge: RenderKnowledge::complete(),
            established_runtime_dead_agents: Vec::new(),
        }
    }

    /// The degraded entry: identity kept, everything unreadable
    /// dropped, the loss visible, the document unharmed.
    #[must_use]
    pub fn degraded<N: Into<String>>(name: N, status: Status) -> Self {
        let mut entry = Self::new(name, status);
        entry.degraded = true;
        entry.knowledge = RenderKnowledge::unavailable();
        entry
    }

    /// Attach the source knowledge captured at the presentation boundary.
    pub(crate) fn set_render_knowledge(&mut self, knowledge: RenderKnowledge) {
        self.knowledge = knowledge;
    }

    /// Bind the independently established runtime maximum facts produced with
    /// this raw snapshot. Keeping provenance on the entry lets JSON and the
    /// table agree without treating a partial event record as complete.
    pub(crate) fn set_established_runtime_dead_agents(&mut self, agents: Vec<String>) {
        self.established_runtime_dead_agents = agents;
    }

    /// Whether the session's attention maximum is exact enough to render.
    #[must_use]
    pub(crate) fn attention_is_exact(&self) -> bool {
        (self.knowledge.roster.is_complete() && self.knowledge.events.is_complete())
            || (self.attention.is_some_and(Reason::is_severity_maximum)
                && !self.established_runtime_dead_agents.is_empty())
    }

    /// Whether event-derived agent state is exact enough for a human cell.
    #[must_use]
    pub(crate) const fn agent_state_is_exact(&self) -> bool {
        self.knowledge.events.is_complete()
    }

    /// Whether this session needs a human — `needs_attention`.
    #[must_use]
    pub const fn needs_attention(&self) -> bool {
        self.attention.is_some()
    }

    /// This session as object.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut fields = vec![
            ("name".to_owned(), Value::str(&self.name)),
            ("status".to_owned(), Value::str(self.status.as_str())),
        ];
        // Source knowledge, never aggregate loss, selects presence.
        push_known_str(
            &mut fields,
            "mode",
            self.mode.as_deref(),
            self.knowledge.mode.is_complete(),
        );
        push_known_str(
            &mut fields,
            "origin",
            self.origin.as_deref(),
            self.knowledge.origin.is_complete(),
        );
        push_known_str(
            &mut fields,
            "work_dir",
            self.work_dir.as_deref(),
            self.knowledge.work_dir.is_complete(),
        );
        push_known_str(
            &mut fields,
            "goal",
            self.goal.as_deref(),
            self.knowledge.goal.is_complete(),
        );
        push_known_num(
            &mut fields,
            "goal_set_epoch",
            self.goal_set_epoch,
            self.knowledge.events.is_complete(),
        );
        // Legacy shape: the runtime has not yet supplied watchdog/git
        // branch observation. Preserve the predecessor's two shapes until that
        // source lands in its own slice: healthy `None` is `null`; degraded
        if self.degraded {
            if let Some(branch) = self.branch.as_deref() {
                fields.push(("branch".to_owned(), Value::str(branch)));
            }
        } else {
            push_str_or_null(&mut fields, "branch", self.branch.as_deref());
        }
        push_known_num(
            &mut fields,
            "last_active_epoch",
            self.last_active_epoch,
            self.knowledge.events.is_complete(),
        );
        fields.push((
            "needs_attention".to_owned(),
            Value::Bool(self.needs_attention()),
        ));
        // `needs_attention` always renders its partial-evidence value. The
        // other two need the maximum to be exact: every relevant source
        // completed, or a separately established runtime `dead` remains after
        if self.attention_is_exact() {
            fields.push((
                "attention".to_owned(),
                self.attention
                    .map_or(Value::Null, |reason| Value::str(reason.as_str())),
            ));
            fields.push((
                "attention_rank".to_owned(),
                Value::Num(self.attention.map_or(0, Reason::rank)),
            ));
        }
        fields.push((
            "agents".to_owned(),
            Value::Arr(
                self.agents
                    .iter()
                    .map(|agent| {
                        agent.to_json_with_event_knowledge(
                            self.knowledge.events.is_complete(),
                            self.established_runtime_dead_agents
                                .iter()
                                .any(|reference| reference == &agent.reference),
                        )
                    })
                    .collect(),
            ),
        ));
        // Additive: present only when true. Member order is an open
        // choice; tests compare the member set, not this key's position.
        if self.degraded {
            fields.push(("degraded".to_owned(), Value::Bool(true)));
        }
        Value::Obj(fields)
    }
}

/// The whole `ae list --json` document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digest {
    /// When this snapshot was taken.
    pub generated_at: Timestamp,
    /// The sessions the active filters selected.
    pub sessions: Vec<SessionEntry>,
    /// whether every enumeration completed.
    pub inventory_complete: bool,
}

impl Digest {
    /// A digest of `sessions`, stamped `generated_at`, carrying the
    /// completeness fact.
    #[must_use]
    pub fn new(
        generated_at: Timestamp,
        sessions: Vec<SessionEntry>,
        inventory_complete: bool,
    ) -> Self {
        Self {
            generated_at,
            sessions,
            inventory_complete,
        }
    }

    /// The document as a JSON value.
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::obj([
            ("schema_version", Value::Num(SCHEMA_VERSION)),
            ("generated_at", Value::str(self.generated_at.to_string())),
            (
                "sessions",
                Value::Arr(self.sessions.iter().map(SessionEntry::to_json).collect()),
            ),
            // An additional member of every successor document.
            ("inventory_complete", Value::Bool(self.inventory_complete)),
        ])
    }

    /// The document as the bytes `ae list --json` prints.
    ///
    /// Infallible by construction — see the note on this module.
    ///
    /// ```
    /// use ae::digest::{Digest, SessionEntry, Status};
    /// use ae::time::Timestamp;
    ///
    /// let digest = Digest::new(
    ///     Timestamp::from_epoch(0),
    ///     vec![SessionEntry::new("my-feature", Status::Running)],
    ///     true,
    /// );
    /// assert!(digest.render().contains(r#""schema_version":2"#));
    /// ```
    #[must_use]
    pub fn render(&self) -> String {
        self.to_json().render()
    }
}

/// Render a nullable string only when its own source completed.
fn push_known_str<S: AsRef<str>>(
    fields: &mut Vec<(String, Value)>,
    key: &str,
    value: Option<S>,
    known: bool,
) {
    if known {
        push_str_or_null(fields, key, value);
    }
}

/// Render a nullable number only when its own source completed.
fn push_known_num(fields: &mut Vec<(String, Value)>, key: &str, value: Option<i64>, known: bool) {
    if known {
        push_num_or_null(fields, key, value);
    }
}

/// Push a string field, or an explicit `null` — presence rule.
fn push_str_or_null<S: AsRef<str>>(fields: &mut Vec<(String, Value)>, key: &str, value: Option<S>) {
    fields.push((
        key.to_owned(),
        value.map_or(Value::Null, |value| Value::str(value.as_ref())),
    ));
}

/// Push a numeric field, or an explicit `null`. See [`push_str_or_null`].
fn push_num_or_null(fields: &mut Vec<(String, Value)>, key: &str, value: Option<i64>) {
    fields.push((key.to_owned(), value.map_or(Value::Null, Value::Num)));
}

#[cfg(test)]
mod tests {
    use super::{ABSENT_SESSION_ID, AgentEntry, Digest, SCHEMA_VERSION, SessionEntry, Status};
    /// Frozen normalised the agent session id at PARSE time and both of its
    /// surfaces rendered the normalised value: an eight-character prefix, or a
    /// dash for absent and `pending`. Never `null`, never the raw id, never the
    /// literal `pending` — measured over the governed frozen digest population,
    /// where every agent id is a dash or an eight-character short id.
    #[test]
    fn sc_509_the_agent_session_id_renders_frozen_s_short_form_never_null_or_raw() {
        let cases = [
            (None, "-", "an absent id is frozen's dash, not JSON null"),
            (
                Some(""),
                "-",
                "an empty recorded id normalises like an absent one",
            ),
            (
                Some("pending"),
                "-",
                "the literal `pending` is not a session id",
            ),
            (
                Some("11111111"),
                "11111111",
                "a captured eight-char id is unchanged",
            ),
            (
                Some("e795c9e9-1c2b-4a3d-8e5f-0a1b2c3d4e5f"),
                "e795c9e9",
                "a full uuid is truncated to frozen's eight characters",
            ),
            (
                Some("abc"),
                "abc",
                "an id shorter than eight characters is whole",
            ),
        ];
        for (recorded, want, why) in cases {
            let agent = AgentEntry {
                reference: "fake:lead".to_owned(),
                alias: "fake".to_owned(),
                name: "lead".to_owned(),
                session_id: recorded.map(ToOwned::to_owned),
                alive: Some(true),
                state: None,
                reason: None,
            };
            assert_eq!(agent.display_session_id(), want, "{why}");
            // The RAW field is untouched: resume and capture logic still need it.
            assert_eq!(agent.session_id.as_deref(), recorded, "raw field preserved");
            let rendered = agent.to_json().render();
            assert!(
                rendered.contains(&format!(r#""session_id":"{want}""#)),
                "{why}: {rendered}"
            );
            assert!(
                !rendered.contains(r#""session_id":null"#),
                "the digest never spells this field null: {rendered}"
            );
        }
    }

    /// Frozen truncated with `${sid:0:8}`, and bash substring expansion counts
    /// CHARACTERS — its own comment at ae@72c7293:3143 says "8-char short session
    /// id". A byte slice at index 8 would panic mid-character on a multibyte id,
    /// and an id recorded by a foreign tool is not a proven-ASCII grammar, so the
    /// boundary is found rather than assumed.
    #[test]
    fn the_short_session_id_truncates_on_a_character_boundary() {
        let mut agent = AgentEntry {
            reference: "fake:lead".to_owned(),
            alias: "fake".to_owned(),
            name: "lead".to_owned(),
            session_id: Some(
                "\u{3b1}\u{3b1}\u{3b1}\u{3b1}\u{3b1}\u{3b1}\u{3b1}\u{3b1}\u{3b2}\u{3b2}".to_owned(),
            ),
            alive: None,
            state: None,
            reason: None,
        };
        let short = agent.display_session_id();
        assert_eq!(
            short.chars().count(),
            8,
            "eight CHARACTERS, not eight bytes"
        );
        assert_eq!(short.len(), 16, "those eight characters are sixteen bytes");
        assert!(
            !short.contains('\u{3b2}'),
            "the ninth character is dropped whole"
        );

        // A multibyte id shorter than the boundary is returned intact rather
        // than sliced at a byte index that is not a character boundary.
        agent.session_id = Some("\u{3b1}\u{3b2}".to_owned());
        assert_eq!(agent.display_session_id(), "\u{3b1}\u{3b2}");
    }

    use crate::attention::Reason;
    use crate::json;
    use crate::time::Timestamp;
    use std::collections::BTreeSet;

    /// The worked example from commands.md's `--json digest` block, rebuilt as
    /// the model. The expected bytes below are read off that block, not off any
    /// implementation.
    fn documented_example() -> Digest {
        let mut session = SessionEntry::new("my-feature", Status::Running);
        session.mode = Some("local".to_owned());
        session.origin = Some("/…".to_owned());
        session.work_dir = Some("/…".to_owned());
        session.goal = Some("ship the login flow".to_owned());
        session.goal_set_epoch = Some(1_779_990_000);
        session.branch = Some("feature/login".to_owned());
        session.last_active_epoch = Some(1_780_000_000);
        session.attention = Some(Reason::Blocked);
        session.agents = vec![AgentEntry {
            reference: "claude:lead".to_owned(),
            alias: "claude".to_owned(),
            name: "lead".to_owned(),
            session_id: Some("e795c9e9".to_owned()),
            alive: Some(true),
            state: Some("blocked".to_owned()),
            reason: Some(Reason::Blocked),
        }];
        Digest::new(
            Timestamp::parse("2026-05-29T14:00:00Z").expect("the documented stamp"),
            vec![session],
            true,
        )
    }

    #[test]
    fn sc_509_renders_the_documented_example_field_for_field() {
        // Member set and values, read off commands.md's worked example. The
        // concat below is that bag of members, not a pin of rendered order.
        let rendered = documented_example().render();
        let expected = concat!(
            r#"{"schema_version":2,"generated_at":"2026-05-29T14:00:00Z","sessions":[{"#,
            r#""name":"my-feature","status":"running","#,
            r#""mode":"local","origin":"/…","work_dir":"/…","#,
            r#""goal":"ship the login flow","goal_set_epoch":1779990000,"#,
            r#""branch":"feature/login","last_active_epoch":1780000000,"#,
            r#""needs_attention":true,"attention":"blocked","attention_rank":3,"#,
            r#""agents":[{"ref":"claude:lead","alias":"claude","name":"lead","#,
            r#""session_id":"e795c9e9","alive":true,"state":"blocked","reason":"blocked"}]"#,
            r#"}],"inventory_complete":true}"#
        );
        let actual = json::parse(&rendered).expect("the digest is json");
        let expected = json::parse(expected).expect("the documented bag is json");
        assert!(
            actual.same_members(&expected),
            "documented members and values, any order: {rendered}"
        );
    }

    #[test]
    fn the_status_list_holds_every_variant_exactly_once() {
        // The compiler cannot check an array literal for completeness — it can
        // check a `match`. Each arm answers with the ALL entry for its own
        // variant, so a fourth variant does not compile until it names a slot,
        let entry = |status: Status| match status {
            Status::Running => Status::ALL[0],
            Status::Unknown => Status::ALL[1],
            Status::Stopped => Status::ALL[2],
        };
        for status in Status::ALL {
            assert_eq!(entry(status), status, "ALL disagrees about {status:?}");
        }
        let mut spellings: Vec<&str> = Status::ALL.iter().map(|s| s.as_str()).collect();
        spellings.sort_unstable();
        spellings.dedup();
        assert_eq!(
            spellings.len(),
            Status::ALL.len(),
            "a status is listed twice, so some variant is missing"
        );
    }

    #[test]
    fn every_status_has_its_own_spelling() {
        assert_eq!(
            Status::ALL.map(Status::as_str),
            ["running", "unknown", "stopped"]
        );
    }

    #[test]
    fn sc_509_the_document_is_one_object_carrying_the_version() {
        let value = json::parse(&documented_example().render()).expect("the digest is json");
        assert_eq!(
            value.get("schema_version"),
            Some(&json::Value::Num(SCHEMA_VERSION))
        );
        assert!(value.get("generated_at").is_some());
        assert!(matches!(
            value.get("sessions"),
            Some(json::Value::Arr(sessions)) if sessions.len() == 1
        ));
    }

    #[test]
    fn sc_509_needs_attention_and_rank_cannot_disagree_with_the_reason() {
        for reason in Reason::BY_SEVERITY {
            let mut session = SessionEntry::new("s", Status::Running);
            session.attention = Some(reason);
            let value = session.to_json();
            assert_eq!(value.get("needs_attention"), Some(&json::Value::Bool(true)));
            assert_eq!(value.get_str("attention"), Some(reason.as_str()));
            assert_eq!(
                value.get("attention_rank"),
                Some(&json::Value::Num(reason.rank()))
            );
        }
    }

    #[test]
    fn sc_017g_a_quiet_read_entry_renders_the_whole_triad() {
        // CHANGED by the 2026-08-24 precision. This test previously
        // asserted that `attention` and `attention_rank` were ABSENT on a quiet
        // entry — "the things that may not exist" — and that was the wrong
        let value = SessionEntry::new("quiet", Status::Running).to_json();
        assert_eq!(
            value.get("needs_attention"),
            Some(&json::Value::Bool(false))
        );
        assert_eq!(
            value.get("attention"),
            Some(&json::Value::Null),
            "null, not absent — a read entry answers"
        );
        assert_eq!(
            value.get("attention_rank"),
            Some(&json::Value::Num(0)),
            "zero, not absent — frozen v1 renders 0 on all 193 of its quiet entries"
        );
        assert_eq!(value.get("agents"), Some(&json::Value::Arr(vec![])));
    }

    #[test]
    fn sc_506_a_degraded_entry_keeps_its_identity_and_closes_the_document() {
        // "one bad session degrades its own entry; the document always closes,
        // never truncates."
        let digest = Digest::new(
            Timestamp::from_epoch(0),
            vec![
                SessionEntry::new("good-one", Status::Running),
                SessionEntry::degraded("unreadable", Status::Running),
                SessionEntry::new("good-two", Status::Stopped),
            ],
            true,
        );
        let rendered = digest.render();
        let value = json::parse(&rendered).expect("a complete, parseable document");
        let Some(json::Value::Arr(sessions)) = value.get("sessions") else {
            panic!("sessions must be an array");
        };
        assert_eq!(
            sessions.len(),
            3,
            "the bad session degrades, it does not vanish"
        );
        assert_eq!(sessions[1].get_str("name"), Some("unreadable"));
        assert_eq!(sessions[1].get_str("status"), Some("running"));
        assert_eq!(
            sessions[2].get_str("name"),
            Some("good-two"),
            "and nothing after it is lost"
        );
        assert!(rendered.ends_with('}'), "the document closes");
    }

    #[test]
    fn sc_509b_loss_is_visible_and_sparsity_is_not() {
        // The seats' reversal: a machine digest that hides loss lies by
        // omission. These two entries carry the same FACTS and must not render
        // identically, because one of them lost data and the other did not.
        let damaged = SessionEntry::degraded("x", Status::Stopped).to_json();
        let expected = json::Value::obj([
            ("name", json::Value::str("x")),
            ("status", json::Value::str("stopped")),
            ("needs_attention", json::Value::Bool(false)),
            ("agents", json::Value::Arr(vec![])),
            ("degraded", json::Value::Bool(true)),
        ]);
        assert!(
            damaged.same_members(&expected),
            "identity survives, the loss is stated, nothing is fabricated: {damaged}"
        );

        let sparse = SessionEntry::new("x", Status::Stopped).to_json();
        assert_eq!(sparse.get("degraded"), None, "a normal entry omits the key");
        assert_ne!(damaged, sparse);
    }

    #[test]
    fn sc_509b_is_additive_so_a_degraded_entry_keeps_the_other_members() {
        let mut entry = SessionEntry::new("x", Status::Running);
        entry.degraded = true;
        let damaged = entry.to_json();
        let sparse = SessionEntry::new("x", Status::Running).to_json();
        let json::Value::Obj(damaged_fields) = &damaged else {
            panic!("an object");
        };
        let json::Value::Obj(sparse_fields) = &sparse else {
            panic!("an object");
        };
        let damaged_keys: BTreeSet<&str> = damaged_fields.iter().map(|(k, _)| k.as_str()).collect();
        let sparse_keys: BTreeSet<&str> = sparse_fields.iter().map(|(k, _)| k.as_str()).collect();

        // aggregate flag has no authority to select another member's
        // presence. This entry's constructors established every source as
        // complete; flipping only the aggregate loss flag must retain those
        assert!(
            BTreeSet::from(["name", "status"]).is_subset(&damaged_keys),
            "identity always survives: {damaged}"
        );
        assert!(
            damaged_keys.contains("degraded") && !sparse_keys.contains("degraded"),
            "the loss key is additive — present on the damaged entry, omitted on the normal one"
        );
        assert!(
            damaged_keys.contains("attention") && damaged_keys.contains("attention_rank"),
            "known quiet remains explicit despite unrelated aggregate loss: {damaged}"
        );
    }

    #[test]
    fn sc_509b_an_unproven_dead_is_not_exact_under_loss() {
        // A raw `Dead` has no source provenance here. A lost ledger may contain
        // a later clear, so its severity alone cannot make it exact.
        let mut entry = SessionEntry::degraded("dead", Status::Running);
        entry.attention = Some(Reason::Dead);
        let value = entry.to_json();
        assert_eq!(value.get("needs_attention"), Some(&json::Value::Bool(true)));
        assert_eq!(value.get("attention"), None);
        assert_eq!(value.get("attention_rank"), None);
    }

    #[test]
    fn sc_510d_text_in_the_digest_is_escaped_not_pasted() {
        // A goal is free text a human typed. It reaches a JSON emitter.
        let mut session = SessionEntry::new("s", Status::Running);
        session.goal = Some("ship \"it\"\nnow\ttoday\\".to_owned());
        let rendered = Digest::new(Timestamp::from_epoch(0), vec![session], true).render();
        assert!(
            rendered.contains(r#""goal":"ship \"it\"\nnow\ttoday\\""#),
            "{rendered}"
        );
        let value = json::parse(&rendered).expect("still one parseable document");
        let Some(json::Value::Arr(sessions)) = value.get("sessions") else {
            panic!("sessions must be an array");
        };
        assert_eq!(
            sessions[0].get_str("goal"),
            Some("ship \"it\"\nnow\ttoday\\"),
            "and the text survives the round trip"
        );
    }

    #[test]
    fn a_session_name_that_is_hostile_to_json_cannot_break_the_document() {
        // Session names are allowlisted elsewhere; the emitter does not rely on
        // that, because a name also arrives from a hand-edited meta file.
        let digest = Digest::new(
            Timestamp::from_epoch(0),
            vec![SessionEntry::new(
                "a\"},{\"name\":\"injected",
                Status::Running,
            )],
            true,
        );
        let value = json::parse(&digest.render()).expect("one document");
        let Some(json::Value::Arr(sessions)) = value.get("sessions") else {
            panic!("sessions must be an array");
        };
        assert_eq!(sessions.len(), 1, "no second session was smuggled in");
    }

    #[test]
    fn an_empty_digest_is_still_a_versioned_document() {
        let rendered = Digest::new(Timestamp::from_epoch(0), Vec::new(), true).render();
        let actual = json::parse(&rendered).expect("the empty digest is json");
        let expected = json::parse(concat!(
            r#"{"schema_version":2,"generated_at":"1970-01-01T00:00:00Z","sessions":[],"#,
            r#""inventory_complete":true}"#
        ))
        .expect("the expected bag is json");
        assert!(
            actual.same_members(&expected),
            "empty still carries version, stamp, sessions, completeness: {rendered}"
        );
    }

    #[test]
    fn sc_509_a_read_entry_renders_every_documented_session_member() {
        // presence rule as a SET, not member by member. Written this way
        // because the member-by-member version is what let four of these regress
        // unnoticed: mutating `goal`, `mode`, `state` or `reason` back to the
        let value = SessionEntry::new("bare", Status::Running).to_json();
        let json::Value::Obj(fields) = &value else {
            panic!("an object")
        };
        let members: BTreeSet<&str> = fields.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            members,
            BTreeSet::from([
                "name",
                "status",
                "mode",
                "origin",
                "work_dir",
                "goal",
                "goal_set_epoch",
                "branch",
                "last_active_epoch",
                "needs_attention",
                "attention",
                "attention_rank",
                "agents",
            ]),
            "every documented member of a READ entry is present: {value}"
        );
        // ...and each empty one carries its legitimate empty VALUE, never absence.
        for key in [
            "mode",
            "origin",
            "work_dir",
            "goal",
            "goal_set_epoch",
            "branch",
            "last_active_epoch",
            "attention",
        ] {
            assert_eq!(value.get(key), Some(&json::Value::Null), "{key} is null");
        }
        assert_eq!(value.get("attention_rank"), Some(&json::Value::Num(0)));
        assert_eq!(
            value.get("needs_attention"),
            Some(&json::Value::Bool(false))
        );
        assert_eq!(value.get("degraded"), None, "and loss is not claimed");
    }

    #[test]
    fn sc_509_an_agent_entry_renders_every_documented_member() {
        // An AgentEntry exists only because the roster was READ, so
        // there is no unreadable case here and no conditional: all seven members,
        // always. `reason: null` is own spelling for "no agent-owned
        let value = AgentEntry {
            reference: "claude:lead".to_owned(),
            alias: "claude".to_owned(),
            name: "lead".to_owned(),
            session_id: None,
            alive: None,
            state: None,
            reason: None,
        }
        .to_json();
        let json::Value::Obj(fields) = &value else {
            panic!("an object")
        };
        let members: BTreeSet<&str> = fields.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            members,
            BTreeSet::from([
                "ref",
                "alias",
                "name",
                "session_id",
                "alive",
                "state",
                "reason"
            ]),
            "every documented agent member is present: {value}"
        );
        // `session_id` is the ONE member here that is never null. Frozen
        // normalised it at parse time and rendered a DASH for an absent or
        // `pending` id, on both surfaces — the governed frozen population is 842
        assert_eq!(
            value.get("session_id"),
            Some(&json::Value::Str(ABSENT_SESSION_ID.to_owned())),
            "an absent session id is frozen's dash, never null: {value}"
        );
        for key in ["alive", "state", "reason"] {
            assert_eq!(value.get(key), Some(&json::Value::Null), "{key} is null");
        }
    }
}
