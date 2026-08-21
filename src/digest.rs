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
    /// This agent as SC-509's object, in the documented field order.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut fields = vec![
            ("ref".to_owned(), Value::str(&self.reference)),
            ("alias".to_owned(), Value::str(&self.alias)),
            ("name".to_owned(), Value::str(&self.name)),
        ];
        push_str(&mut fields, "session_id", self.session_id.as_deref());
        // Present even when null — see the field's own docs.
        fields.push((
            "alive".to_owned(),
            self.alive.map_or(Value::Null, Value::Bool),
        ));
        push_str(&mut fields, "state", self.state.as_deref());
        push_str(&mut fields, "reason", self.reason.map(Reason::as_str));
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

    /// This session as SC-509's object, in the documented field order.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut fields = vec![
            ("name".to_owned(), Value::str(&self.name)),
            ("status".to_owned(), Value::str(self.status.as_str())),
        ];
        push_str(&mut fields, "mode", self.mode.as_deref());
        push_str(&mut fields, "origin", self.origin.as_deref());
        push_str(&mut fields, "work_dir", self.work_dir.as_deref());
        push_str(&mut fields, "goal", self.goal.as_deref());
        push_num(&mut fields, "goal_set_epoch", self.goal_set_epoch);
        push_str(&mut fields, "branch", self.branch.as_deref());
        push_num(&mut fields, "last_active_epoch", self.last_active_epoch);
        fields.push((
            "needs_attention".to_owned(),
            Value::Bool(self.needs_attention()),
        ));
        push_str(&mut fields, "attention", self.attention.map(Reason::as_str));
        push_num(
            &mut fields,
            "attention_rank",
            self.attention.map(Reason::rank),
        );
        fields.push((
            "agents".to_owned(),
            Value::Arr(self.agents.iter().map(AgentEntry::to_json).collect()),
        ));
        // SC-509b, additive: appended AFTER the documented field set, so
        // SC-509's order survives intact as this object's prefix.
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
    /// content depends on the wall clock cannot be asserted byte-for-byte, and
    /// a snapshot format is exactly the thing worth asserting byte-for-byte.
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
            // SC-017o, appended AFTER SC-509's documented set, exactly as
            // SC-509b's `degraded` is appended inside a session object: the
            // documented order survives intact as this object's prefix.
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
    /// assert!(digest.render().starts_with(r#"{"schema_version":2,"#));
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

#[cfg(test)]
mod tests {
    use super::{AgentEntry, Digest, SCHEMA_VERSION, SessionEntry, Status};
    use crate::attention::Reason;
    use crate::json;
    use crate::time::Timestamp;

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
        assert_eq!(rendered, expected);
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
    fn sc_509_the_document_is_one_object_carrying_the_version_first() {
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
    fn a_session_with_nothing_to_report_still_answers_the_attention_question() {
        // needs_attention is a predicate, always answerable; the reason and its
        // rank are the things that may not exist.
        let value = SessionEntry::new("quiet", Status::Running).to_json();
        assert_eq!(
            value.get("needs_attention"),
            Some(&json::Value::Bool(false))
        );
        assert_eq!(value.get("attention"), None);
        assert_eq!(value.get("attention_rank"), None);
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
        let json::Value::Obj(fields) = &damaged else {
            panic!("an object");
        };
        let keys: Vec<&str> = fields.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys,
            ["name", "status", "needs_attention", "agents", "degraded"],
            "identity survives, the loss is stated, nothing is fabricated"
        );
        assert_eq!(damaged.get("degraded"), Some(&json::Value::Bool(true)));

        let sparse = SessionEntry::new("x", Status::Stopped).to_json();
        assert_eq!(sparse.get("degraded"), None, "a normal entry omits the key");
        assert_ne!(damaged, sparse);
    }

    #[test]
    fn sc_509b_is_additive_so_the_documented_order_is_still_a_prefix() {
        let mut entry = SessionEntry::new("x", Status::Running);
        entry.degraded = true;
        let json::Value::Obj(fields) = entry.to_json() else {
            panic!("an object");
        };
        let keys: Vec<String> = fields.iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(
            keys.last().map(String::as_str),
            Some("degraded"),
            "the new key goes after the documented set, never inside it"
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
        assert_eq!(
            rendered,
            concat!(
                r#"{"schema_version":2,"generated_at":"1970-01-01T00:00:00Z","sessions":[],"#,
                r#""inventory_complete":true}"#
            )
        );
    }
}
