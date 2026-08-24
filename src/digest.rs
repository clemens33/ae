//! The `ae list --json` document: SC-509's schema, SC-506's durability.
//!
//! **SC-509** — a single JSON object carrying `schema_version` (1),
//! `generated_at` and `sessions[]`, each session with
//! `name`/`status`/`mode`/`origin`/`work_dir`/`goal`/`goal_set_epoch`/`branch`/
//! `last_active_epoch`/`needs_attention`/`attention`/`attention_rank` and an
//! `agents[]` of `ref`/`alias`/`name`/`session_id`/`alive`/`state`/`reason`.
//! "`schema_version` lets consumers gate on shape", so it is a constant of this
//! module rather than a literal at the emission site.
//!
//! **SC-506** — one bad session degrades its own entry; the document always
//! closes, never truncates. In bash that needs a guard around the emitting loop.
//! Here it is structural: a [`Digest`] is built first and rendered second, and
//! [`crate::json::Value::render`] cannot fail. There is no code path that emits
//! half a document, so there is no path to protect.
//!
//! Two consistency facts are enforced by construction rather than asserted:
//! `needs_attention` is `attention.is_some()`, and `attention_rank` is that
//! reason's own rank. They cannot drift apart because neither is stored.
//!
//! # Presence is a contract, not a serializer convenience
//!
//! **A member that was READ is rendered, whatever its value.** SC-017g as
//! precised gives the session's attention triad (`false` / `null` / `0` when
//! quiet) and SC-509c gives `agents[].reason` (`null` when no agent-owned
//! contribution exists). `push_str` and `push_num` exist for the OTHER case —
//! SC-509b's omission of a fact that could not be read — and the two must not be
//! confused, because rendering a legitimate empty as an absence makes damage and
//! sparsity the same bytes. Eight other optional members still take the
//! `push_*` path and are universally present in frozen v1; whether that is
//! correct is an open question recorded in SC-017g's scope guard, not something
//! this module decides.

use crate::attention::Reason;
use crate::json::Value;
use crate::time::Timestamp;

/// The `schema_version` every SUCCESSOR digest publishes — **SC-509d**.
///
/// Version 2, because `sessions[].status` gained `unknown`. A new value in an
/// existing field is a consumer-visible contract change even though the field
/// name, JSON type and position are unchanged: a consumer that gated on the
/// two-value domain breaks on the third. Versioning is the gate, and this is the
/// same change that made [`Status::Unknown`] constructible — earlier would
/// version a domain nothing could produce, later would ship the break.
///
/// SC-509 remains the true frozen-bash version-1 contract; it is not rewritten
/// after the fact, and this crate has no path that emits version 1.
pub const SCHEMA_VERSION: i64 = 2;

/// Whether a session is running, stopped, or not established either way.
///
/// SC-017a/b/c divide the *established* world into running and stopped, so the
/// digest's `status` is an enumeration rather than free text. SC-816 and
/// SC-835c add the third case: inability to verify is not absence, so a failed
/// liveness check gets its own value instead of collapsing into `Stopped`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The session is live.
    Running,
    /// The session is stopped history.
    Stopped,
    /// Liveness was NOT ESTABLISHED - the recorded server was unreachable, the
    /// query failed, or ownership evidence was missing. It is NOT stopped and
    /// NOT absence.
    ///
    /// **The schema moved to version 2 with this variant's first constructor**
    /// — SC-509d, and [`SCHEMA_VERSION`] carries the reasoning. A new enum value
    /// in an existing field is NOT backward-compatible even though the field
    /// shape is unchanged: a consumer that gated on `status` being one of two
    /// spellings breaks on a third. The variant existed, unreachable, for
    /// exactly one phase before [`crate::liveness::classify`] could produce it;
    /// declaring it was never the boundary, constructing it was.
    ///
    /// Orthogonal to [`SessionEntry::degraded`]: `degraded` means record facts
    /// were LOST, `Unknown` means liveness was NOT ESTABLISHED. Either, both, or
    /// neither can hold; they are never derived from one another.
    Unknown,
}

impl Status {
    /// Every status, in the SC-017n group order: running, unknown, stopped.
    ///
    /// An array literal is NOT exhaustiveness-checked. That is the exact shape
    /// that let `filters.rs` enumerate the variants per scope and go on
    /// compiling — silently dropping a new state from every listing — while the
    /// one `match` on `Status` was updated and the compiler reported success.
    /// So this constant carries a guard rather than a promise: the test
    /// `the_status_list_holds_every_variant_exactly_once` answers each variant
    /// through a `match`, so a fourth variant fails to BUILD the suite until it
    /// is named, and the only thing it can legally name is a real index of this
    /// array.
    pub const ALL: [Self; 3] = [Self::Running, Self::Unknown, Self::Stopped];

    /// The spelling SC-509's example carries (`"status": "running"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Stopped => "stopped",
            Self::Unknown => "unknown",
        }
    }
}

/// One entry of a session's `agents[]` (SC-509).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgentEntry {
    /// `ref` — the `alias:name` an agent is addressed by.
    pub reference: String,
    /// The configured alias half of the ref.
    pub alias: String,
    /// The display-name half of the ref.
    pub name: String,
    /// The captured tool session id, where one exists.
    pub session_id: Option<String>,
    /// Whether the agent's pane is alive — **SC-509e**, three-valued.
    ///
    /// `Some(true)` and `Some(false)` carry SC-017p's POSITIVELY established
    /// facts: an exact association to a live pane, or a proof that this exact
    /// roster agent has none. `None` is SC-017q's `unknown`, and it is emitted
    /// as JSON `null` rather than omitted — the field is present even when null,
    /// because a consumer gating on its presence must not have to tell "absent
    /// because unknown" from "absent because this reader is old".
    ///
    /// It was a `bool`, and that was the same defect as the session status one
    /// level up: a missing observation encoded as a negative FACT. Frozen bash
    /// initialised it to false and flipped it only on a positive hit, so an
    /// unavailable pane query rendered exactly like a dead agent.
    pub alive: Option<bool>,
    /// The agent's declared work state.
    pub state: Option<String>,
    /// "each agent's `reason` is its own contribution" to the session marker.
    pub reason: Option<Reason>,
}

impl AgentEntry {
    /// This agent as SC-509's object.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut fields = vec![
            ("ref".to_owned(), Value::str(&self.reference)),
            ("alias".to_owned(), Value::str(&self.alias)),
            ("name".to_owned(), Value::str(&self.name)),
        ];
        // Unconditional, like `reason` below: an `AgentEntry` exists only because
        // the roster was READ (SC-405k — membership is roster-defined), so the
        // question "was this member readable" is answered yes by the entry's
        // existence. A session that lost its roster renders no agent entries.
        push_str_or_null(&mut fields, "session_id", self.session_id.as_deref());
        // Present even when null — see the field's own docs.
        fields.push((
            "alive".to_owned(),
            self.alive.map_or(Value::Null, Value::Bool),
        ));
        push_str_or_null(&mut fields, "state", self.state.as_deref());
        // SC-509c says `reason: null` MEANS no agent-owned contribution exists,
        // so null is the ruled spelling of that answer and absence is not a
        // synonym for it. Frozen v1 agrees: all 840 agent entries in the corpus
        // render the member, every one of them as null.
        //
        // Unconditional here, unlike the session triad: an `AgentEntry` EXISTS
        // only because the roster was read (SC-405k — membership is
        // roster-defined), so the question "was this agent's reason readable" is
        // already answered yes by the entry's existence. A session that lost its
        // roster has no agent entries to render at all.
        fields.push((
            "reason".to_owned(),
            self.reason
                .map_or(Value::Null, |reason| Value::str(reason.as_str())),
        ));
        Value::Obj(fields)
    }
}

/// One entry of the digest's `sessions[]` (SC-509).
///
/// Every field the reader may fail to establish is an [`Option`]. That is what
/// makes SC-506's "degrades its own entry" expressible without a second shape:
/// a degraded entry is this entry with less in it, not a different kind of
/// thing.
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
    /// When that goal was last set — "age it for staleness".
    pub goal_set_epoch: Option<i64>,
    /// The session's live git branch.
    pub branch: Option<String>,
    /// When the session last did anything ae could see.
    pub last_active_epoch: Option<i64>,
    /// The session-level rollup (SC-017g). `None` means nothing needs a human.
    pub attention: Option<Reason>,
    /// The session's agents.
    pub agents: Vec<AgentEntry>,
    /// Whether this entry suffered ACTUAL read/parse loss — SC-509b.
    ///
    /// Emitted as the additive key `degraded: true`, and omitted entirely when
    /// false. The seats reversed the first reading of SC-506 on exactly this
    /// point: "damage is never rendered identically to legitimate sparsity — a
    /// machine digest that hides loss lies by omission". Additive rather than
    /// always-present because SC-511c's evolution rule is what makes a new key
    /// legal at all, and an always-present `false` would change every existing
    /// entry's shape for nothing.
    pub degraded: bool,
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
            goal_set_epoch: None,
            branch: None,
            last_active_epoch: None,
            attention: None,
            agents: Vec::new(),
            degraded: false,
        }
    }

    /// The SC-506/SC-509b degraded entry: identity kept, everything unreadable
    /// dropped, the loss visible, the document unharmed.
    #[must_use]
    pub fn degraded<N: Into<String>>(name: N, status: Status) -> Self {
        Self {
            degraded: true,
            ..Self::new(name, status)
        }
    }

    /// Whether this session needs a human — SC-509's `needs_attention`.
    ///
    /// Derived, never stored: a flag that can disagree with the reason beside it
    /// is a flag that eventually will.
    #[must_use]
    pub const fn needs_attention(&self) -> bool {
        self.attention.is_some()
    }

    /// This session as SC-509's object.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut fields = vec![
            ("name".to_owned(), Value::str(&self.name)),
            ("status".to_owned(), Value::str(self.status.as_str())),
        ];
        // SC-509 as ruled 2026-08-24: PRESENCE IS PART OF THE SCHEMA. Every
        // documented member whose source was READ is present, carrying its
        // legitimate empty value; omission is reserved for SC-509b loss. So a
        // read entry renders `null` where it has nothing, and only a `degraded`
        // entry omits — because for that entry the fact could not be read, and
        // SC-509b says such a fact is "omitted, never fabricated, never null".
        //
        // The `degraded` flag is a COARSE proxy for "this member was unreadable":
        // an entry degraded by a malformed meta line may have read its events
        // perfectly and will still omit what it knows. That imprecision is
        // reported, not resolved here — the type cannot yet say which member was
        // lost, and guessing would put a value where the row demands silence.
        if self.degraded {
            push_str(&mut fields, "mode", self.mode.as_deref());
            push_str(&mut fields, "origin", self.origin.as_deref());
            push_str(&mut fields, "work_dir", self.work_dir.as_deref());
            push_str(&mut fields, "goal", self.goal.as_deref());
            push_num(&mut fields, "goal_set_epoch", self.goal_set_epoch);
            push_str(&mut fields, "branch", self.branch.as_deref());
            push_num(&mut fields, "last_active_epoch", self.last_active_epoch);
        } else {
            push_str_or_null(&mut fields, "mode", self.mode.as_deref());
            push_str_or_null(&mut fields, "origin", self.origin.as_deref());
            push_str_or_null(&mut fields, "work_dir", self.work_dir.as_deref());
            push_str_or_null(&mut fields, "goal", self.goal.as_deref());
            push_num_or_null(&mut fields, "goal_set_epoch", self.goal_set_epoch);
            push_str_or_null(&mut fields, "branch", self.branch.as_deref());
            push_num_or_null(&mut fields, "last_active_epoch", self.last_active_epoch);
        }
        fields.push((
            "needs_attention".to_owned(),
            Value::Bool(self.needs_attention()),
        ));
        // SC-017g as PRECISED (2026-08-24): a READ entry renders all three
        // attention members, and a quiet one renders `false` / `null` / `0`.
        // Omission is SC-509b's spelling for a fact that could not be READ, and
        // reusing it for a legitimately empty one makes loss and legitimate-none
        // the same byte pattern — the collapse that row's closing sentence
        // forbids ("Damage is never rendered identically to legitimate
        // sparsity").
        //
        // **AND THE CONVERSE, which is why this is conditional.** SC-509b says an
        // unreadable optional fact is "omitted, never fabricated, NEVER NULL",
        // so a degraded entry may not render `null` here either — null is the
        // ruled spelling of "read, and empty", which is a claim a lossy entry
        // cannot make. The two rows meet exactly at `degraded`.
        //
        // `degraded` is a COARSE proxy for "this particular fact was unreadable"
        // and the type cannot currently do better: an entry degraded by a
        // malformed meta line may have read its events perfectly, and would omit
        // a triad it actually knows. Reported as an open boundary rather than
        // guessed at — SC-017g's precision speaks only about entries that WERE
        // read, and this is the conservative reading that satisfies both rows
        // with the information the type has.
        if self.degraded {
            push_str(&mut fields, "attention", self.attention.map(Reason::as_str));
            push_num(
                &mut fields,
                "attention_rank",
                self.attention.map(Reason::rank),
            );
        } else {
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
            Value::Arr(self.agents.iter().map(AgentEntry::to_json).collect()),
        ));
        // SC-509b, additive: present only when true. Member order is an open
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
    /// The sessions the active filters selected (SC-017f).
    pub sessions: Vec<SessionEntry>,
    /// **SC-017o** — whether every SC-017j enumeration completed.
    ///
    /// `true` means zero required enumeration losses; `false` means one or more.
    /// Emitted in EVERY document including an empty one, because that is the
    /// case where it carries the most: "no sessions" and "no sessions, and I
    /// could not look everywhere" are different snapshots, and only one of them
    /// is evidence of absence.
    ///
    /// Not conditional on anything. A field that appears only when something
    /// went wrong is a field consumers cannot gate on.
    pub inventory_complete: bool,
}

impl Digest {
    /// A digest of `sessions`, stamped `generated_at`, carrying SC-017o's
    /// completeness fact.
    ///
    /// The stamp is a parameter rather than a clock read: a document whose
    /// `generated_at` depends on the wall clock cannot be asserted, and the
    /// snapshot's members and values are exactly the thing worth asserting.
    /// Rendered member order is an open choice (phase-3 criterion 15).
    ///
    /// `inventory_complete` is a parameter for the same reason it is not
    /// derived from `sessions`: an incomplete snapshot can hold any number of
    /// sessions, including all of them, and a document that inferred
    /// completeness from what it happened to contain would assert exactly the
    /// thing nobody established.
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
            // SC-017o, an additional member of every successor document.
            ("inventory_complete", Value::Bool(self.inventory_complete)),
        ])
    }

    /// The document as the bytes `ae list --json` prints.
    ///
    /// Infallible by construction — see the SC-506 note on this module.
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

/// Push a string field, or nothing at all when there is no value.
///
/// Omission, not `null`: SC-509's worked example shows no null, and a consumer
/// gating on shape can test for a key's presence. The alternative — inventing a
/// null convention the row does not describe — would be schema this slice has no
/// authority to write.
fn push_str<S: AsRef<str>>(fields: &mut Vec<(String, Value)>, key: &str, value: Option<S>) {
    if let Some(value) = value {
        fields.push((key.to_owned(), Value::str(value.as_ref())));
    }
}

/// Push a numeric field, or nothing at all. See [`push_str`].
fn push_num(fields: &mut Vec<(String, Value)>, key: &str, value: Option<i64>) {
    if let Some(value) = value {
        fields.push((key.to_owned(), Value::Num(value)));
    }
}

/// Push a string field, or an explicit `null` — SC-509's presence rule.
///
/// The counterpart of [`push_str`], and the two are not interchangeable. This one
/// says "read, and empty"; `push_str` says "could not be read" (SC-509b). Using
/// the wrong one makes damage and legitimate sparsity the same bytes, which is
/// what SC-509b's closing sentence forbids.
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
    use super::{AgentEntry, Digest, SCHEMA_VERSION, SessionEntry, Status};
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
        // and `Status::ALL[3]` on a three-element array is a deny-by-default
        // `unconditional_panic`: naming a slot that does not exist fails the
        // build too, which leaves growing ALL as the only way through.
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
        // CHANGED by the 2026-08-24 SC-017g precision. This test previously
        // asserted that `attention` and `attention_rank` were ABSENT on a quiet
        // entry — "the things that may not exist" — and that was the wrong
        // letter: absence is SC-509b's spelling for a fact that could not be
        // READ, so spending it on a legitimately empty one makes loss and
        // legitimate-none the same bytes.
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

        // NARROWED by the 2026-08-24 SC-017g precision, and the narrowing is the
        // point. This test used to assert `sparse_keys ⊆ damaged_keys` and
        // `damaged − sparse == {degraded}` — an ADDITIVE MEMBER SET. The row says
        // something weaker: "additive KEY; normal entries may omit it", plus
        // "identity (name + status) always survives". It also says the opposite
        // of a superset for everything else — "unreadable optional facts are
        // omitted". The old assertion held only because a normal entry ALSO
        // omitted the attention triad, so the two omissions cancelled; once a
        // read entry renders the triad, a superset over the whole member set and
        // "unreadable facts are omitted" cannot both be true.
        //
        // So this now pins what SC-509b actually states.
        assert!(
            BTreeSet::from(["name", "status"]).is_subset(&damaged_keys),
            "identity always survives: {damaged}"
        );
        assert!(
            damaged_keys.contains("degraded") && !sparse_keys.contains("degraded"),
            "the loss key is additive — present on the damaged entry, omitted on the normal one"
        );
        assert!(
            !damaged_keys.contains("attention") && !damaged_keys.contains("attention_rank"),
            "and an unreadable optional fact is omitted, never null: {damaged}"
        );
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
        // SC-509's presence rule as a SET, not member by member. Written this way
        // because the member-by-member version is what let four of these regress
        // unnoticed: mutating `goal`, `mode`, `state` or `reason` back to the
        // omitting push survived the whole suite until this test existed.
        //
        // The entry is maximally EMPTY and NOT degraded — every source was read
        // and had nothing in it, which is precisely the case that must render.
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
        // An AgentEntry exists only because the roster was READ (SC-405k), so
        // there is no unreadable case here and no conditional: all seven members,
        // always. `reason: null` is SC-509c's own spelling for "no agent-owned
        // contribution exists", and frozen v1 renders it on all 840 of its agent
        // entries.
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
        for key in ["session_id", "alive", "state", "reason"] {
            assert_eq!(value.get(key), Some(&json::Value::Null), "{key} is null");
        }
    }
}
