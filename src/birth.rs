//! The worker birth family: the journal's durable, prompt-free record of who a
//! new worker was at birth, and the facts later stapled to it.
//!
//! `worker-birth` is the certificate, one per committed spawn. `worker-fact`
//! carries a later fact about that birth. `spawn` stays the announcement, with
//! the task text; it only gains a `birth` link, so a modern announcement whose
//! certificate was lost reads apart from a legacy one.
//!
//! `birth` is the seat's FIRST `launch_id`: a UUID born with the seat, which a
//! journal record never rotates. A reseat's new launch id simply matches no
//! birth.
//!
//! `birth_v` is the family's own version, and nothing here reads the envelope
//! `schema_version`. Reading is LENIENT and outside `events::KNOWN_KEYS`, like
//! `wait_on`: a bad family member costs the family evidence and never the
//! record any other reader sees. [`decode`] classifies; a statistics fold decides
//! what a class counts as.

use crate::events::Event;
use crate::json::Value;
use crate::time::Timestamp;

/// The certificate's journal action.
pub const BIRTH: &str = "worker-birth";
/// A later fact's journal action.
pub const FACT: &str = "worker-fact";
/// The family version this core reads and writes.
pub const VERSION: u32 = 1;

/// The announcement that links to its certificate.
const SPAWN: &str = "spawn";
/// The member a linked `spawn` carries, and the family's key within a session.
const ID: &str = "birth";
/// The family version member.
const FAMILY_VERSION: &str = "birth_v";
/// Every member this family reads off a record.
const MEMBERS: [&str; 12] = [
    FAMILY_VERSION,
    ID,
    "birth_tool",
    "birth_client",
    "birth_model",
    "birth_effort",
    "birth_profile",
    "fact",
    "start",
    "observed_source",
    "observed_model",
    "observed_effort",
];
/// The closed v1 fact vocabulary.
const FACT_KINDS: [&str; 4] = [
    "brief-failed",
    "start-attempted",
    "start-failed",
    "observed",
];
/// The longest cell a member may carry, in bytes.
const CELL_MAX_BYTES: usize = 256;

/// Whether `action` is one of the family's two actions: the records the trim
/// keeps, and that no activity or quiet fold reads as news.
#[must_use]
pub fn is_family(action: &str) -> bool {
    matches!(action, BIRTH | FACT)
}

/// Whether a raw journal line is one the resume trim must keep. Decided on the
/// action alone, so damaged and newer-version records survive an older core.
/// A line that is not an event carries nothing to keep. Parsed outright, with
/// no substring fast path, so an escaped action spelling cannot slip past.
#[must_use]
pub fn durable_line(line: &str) -> bool {
    Event::parse_line(line).is_ok_and(|event| is_family(&event.action))
}

/// Which record shape a classification came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// `worker-birth`.
    Birth,
    /// `worker-fact`.
    Fact,
    /// A `spawn` carrying a `birth` link.
    Spawn,
}

/// A family record this reader cannot trust. What identity it still proves
/// comes with it: a gap should name the birth it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Damage {
    /// The record's shape.
    pub family: Family,
    /// The `birth` member, when it passes the id grammar.
    pub birth: Option<String>,
    /// The `target_session_id`, when it passes the id grammar.
    pub session: Option<String>,
}

/// A certificate as read. Every cell is optional: absence is itself a fact the
/// certificate proves, `model` absent meaning the command set no flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Birth {
    pub session: Option<String>,
    pub birth: String,
    pub name: Option<String>,
    pub slot: Option<String>,
    pub tool: Option<String>,
    pub client: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub profile: Option<String>,
}

/// A later fact as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    pub session: Option<String>,
    pub birth: String,
    /// One of the closed v1 fact kinds.
    pub kind: String,
    pub start: Option<String>,
    pub observed_source: Option<String>,
    pub observed_model: Option<String>,
    pub observed_effort: Option<String>,
}

/// One classified family record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    Birth(Birth),
    Fact(Fact),
    /// A newer family version, its frozen identity members still usable.
    Unsupported {
        family: Family,
        version: u32,
        birth: String,
        session: Option<String>,
    },
    /// A modern `spawn`'s link to its certificate.
    SpawnLink {
        birth: String,
    },
}

/// One member's value, reduced to what the grammar can use. A string that is no
/// cell is [`Member::Other`] at once, so an oversized value is not kept.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Member {
    Text(String),
    Int(i64),
    Other,
}

impl Member {
    fn of(value: &Value) -> Self {
        match value {
            Value::Str(text) if is_cell(text) => Self::Text(text.clone()),
            Value::Num(number) => Self::Int(*number),
            _ => Self::Other,
        }
    }
}

/// Nonempty, bounded and free of control characters: what may reach a table.
fn is_cell(text: &str) -> bool {
    !text.is_empty() && text.len() <= CELL_MAX_BYTES && !text.chars().any(char::is_control)
}

/// The family members of one record, repeats included, exactly as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raw(Vec<(&'static str, Member)>);

impl Raw {
    /// What `action` carries, or `None` for any other record and for a `spawn`
    /// with no link. Never an error.
    pub(crate) fn of(action: &str, fields: &[(String, Value)]) -> Option<Self> {
        let known: &[&'static str] = match action {
            BIRTH | FACT => &MEMBERS,
            SPAWN => &[ID],
            _ => return None,
        };
        let members: Vec<_> = fields
            .iter()
            .filter_map(|(key, value)| {
                let name = known.iter().find(|name| **name == key)?;
                Some((*name, Member::of(value)))
            })
            .collect();
        (action != SPAWN || !members.is_empty()).then_some(Self(members))
    }

    /// The one value of `key`: `Ok(None)` absent, `Err` repeated.
    fn sole(&self, key: &str) -> Result<Option<&Member>, ()> {
        let mut named = self.0.iter().filter(|(name, _)| *name == key);
        match (named.next(), named.next()) {
            (first, None) => Ok(first.map(|(_, member)| member)),
            _ => Err(()),
        }
    }

    /// The one text of `key`: `Ok(None)` absent, `Err` repeated or no cell.
    fn text(&self, key: &str) -> Result<Option<String>, ()> {
        match self.sole(key)? {
            None => Ok(None),
            Some(Member::Text(text)) => Ok(Some(text.clone())),
            Some(_) => Err(()),
        }
    }

    /// The birth id, when exactly one spelling of it passes the id grammar.
    fn id(&self) -> Option<String> {
        self.text(ID)
            .ok()
            .flatten()
            .filter(|id| crate::archive::is_canonical_uuid(id))
    }

    /// The family version: an integer from 1, written exactly once.
    fn version(&self) -> Option<u32> {
        match self.sole(FAMILY_VERSION) {
            Ok(Some(Member::Int(number))) => u32::try_from(*number).ok().filter(|v| *v >= 1),
            _ => None,
        }
    }
}

/// The session a family record names: `Ok(None)` when it names none, `Err` when
/// what it names is no session id.
fn session_of(event: &Event) -> Result<Option<String>, ()> {
    if !event.target_session_id.is_present() {
        return Ok(None);
    }
    match event.target_session_id.value() {
        Some(id) if crate::events::is_session_id(id) => Ok(Some(id.to_owned())),
        _ => Err(()),
    }
}

/// A cell the record's envelope carries.
fn envelope(text: Option<&str>) -> Result<Option<String>, ()> {
    match text {
        Some(text) if !is_cell(text) => Err(()),
        text => Ok(text.map(ToOwned::to_owned)),
    }
}

/// Classify one record: `None` for every other action and for a legacy `spawn`.
#[must_use]
pub fn decode(event: &Event) -> Option<Result<Record, Damage>> {
    let family = match event.action.as_str() {
        BIRTH => Family::Birth,
        FACT => Family::Fact,
        SPAWN => Family::Spawn,
        _ => return None,
    };
    let raw = event.birth.as_ref()?;
    let birth = raw.id();
    let session = session_of(event);
    let record = match (family, birth.clone(), session.clone()) {
        (Family::Spawn, Some(birth), _) => Some(Record::SpawnLink { birth }),
        (Family::Birth | Family::Fact, Some(birth), Ok(session)) => {
            versioned(event, raw, family, birth, session)
        }
        _ => None,
    };
    Some(record.ok_or(Damage {
        family,
        birth,
        session: session.ok().flatten(),
    }))
}

/// A record whose identity is proved, read under its version.
fn versioned(
    event: &Event,
    raw: &Raw,
    family: Family,
    birth: String,
    session: Option<String>,
) -> Option<Record> {
    let version = raw.version()?;
    if version > VERSION {
        return Some(Record::Unsupported {
            family,
            version,
            birth,
            session,
        });
    }
    if family == Family::Birth {
        certificate(event, raw, birth, session).ok()
    } else {
        fact(raw, birth, session).ok()
    }
}

/// A supported certificate.
fn certificate(
    event: &Event,
    raw: &Raw,
    birth: String,
    session: Option<String>,
) -> Result<Record, ()> {
    Ok(Record::Birth(Birth {
        session,
        birth,
        name: envelope(event.target.as_deref())?,
        slot: envelope(event.target_slot.value())?,
        tool: raw.text("birth_tool")?,
        client: raw.text("birth_client")?,
        model: raw.text("birth_model")?,
        effort: raw.text("birth_effort")?,
        profile: raw.text("birth_profile")?,
    }))
}

/// A supported fact, of a kind the closed v1 vocabulary names.
fn fact(raw: &Raw, birth: String, session: Option<String>) -> Result<Record, ()> {
    let kind = raw
        .text("fact")?
        .filter(|kind| FACT_KINDS.contains(&kind.as_str()))
        .ok_or(())?;
    Ok(Record::Fact(Fact {
        session,
        birth,
        kind,
        start: raw.text("start")?,
        observed_source: raw.text("observed_source")?,
        observed_model: raw.text("observed_model")?,
        observed_effort: raw.text("observed_effort")?,
    }))
}

/// What a spawn resolved: the configured half of a birth, read once where
/// resolution knows it. Never the profile's definition today.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Configured {
    /// The resolved adapter family.
    pub tool: String,
    /// The `[clients]` label expanded, `None` for a direct command.
    pub client: Option<String>,
    /// The model flag's value as written: an alias stays one.
    pub model: Option<String>,
    /// The effort the command proves.
    pub effort: Option<String>,
    /// Diagnostic provenance, never an aggregation key.
    pub profile: String,
}

/// The configured facts of one resolved command, read the way `_run` reads its
/// first-start baseline.
#[must_use]
pub fn configured(
    command: &crate::config::ResolvedCommand,
    tool: crate::tool::ToolKind,
    profile: &str,
) -> Configured {
    Configured {
        tool: tool.as_str().to_owned(),
        client: command.client_label().map(ToOwned::to_owned),
        model: crate::launch_cmd::model_flag_value(command.as_str(), tool),
        effort: crate::launch_cmd::effort_flag(command.as_str(), tool)
            .ok()
            .map(|level| level.word().to_owned()),
        profile: profile.to_owned(),
    }
}

/// Who one certificate names.
pub(crate) struct Seat<'a> {
    pub(crate) name: &'a str,
    pub(crate) slot: &'a str,
    /// The durable session id, empty when the meta records none.
    pub(crate) session: &'a str,
    /// The birth id: the seat's first launch id.
    pub(crate) birth: &'a str,
}

/// The certificate line, LF included. Built here rather than through the
/// tracked event shape because its version is a number, and because nothing of
/// a task may reach it: no summary, no body file, no command.
pub(crate) fn certificate_line(
    ts: Timestamp,
    actor: &str,
    seat: &Seat<'_>,
    configured: &Configured,
) -> String {
    let mut members = vec![
        ("ts", Value::Str(ts.to_string())),
        ("actor", Value::str(actor)),
        ("action", Value::str(BIRTH)),
        ("target", Value::str(seat.name)),
        ("target_slot", Value::str(seat.slot)),
    ];
    if !seat.session.is_empty() {
        members.push(("target_session_id", Value::str(seat.session)));
    }
    members.push((FAMILY_VERSION, Value::Num(i64::from(VERSION))));
    members.push((ID, Value::str(seat.birth)));
    members.push(("birth_tool", Value::str(configured.tool.as_str())));
    for (key, value) in [
        ("birth_client", &configured.client),
        ("birth_model", &configured.model),
        ("birth_effort", &configured.effort),
    ] {
        if let Some(value) = value {
            members.push((key, Value::str(value.as_str())));
        }
    }
    members.push(("birth_profile", Value::str(configured.profile.as_str())));
    let mut line = Value::obj(members).render();
    line.push('\n');
    line
}

/// The family's wire reaches readers that predate it. A summary-less record must
/// stay inert in each: an omitted row or an action and identity label, never a
/// panic, an error, a body load, or any configured selection.
#[cfg(test)]
#[allow(clippy::expect_used, reason = "fixed fixture records")]
mod render_spec {
    use std::cell::Cell;

    use crate::board::Observation;
    use crate::console::lane::{self, Body, Seat};
    use crate::events::Event;

    const SECRET_CELLS: [&str; 4] = ["ROUTE_CELL", "MODEL_CELL", "EFFORT_CELL", "PROFILE_CELL"];

    fn certificate() -> String {
        concat!(
            r#"{"ts":"2026-11-01T10:00:00Z","actor":"lead","action":"worker-birth","#,
            r#""target":"scout","target_slot":"spawned.0","birth_v":1,"#,
            r#""birth":"22222222-2222-4222-8222-222222222222","birth_tool":"claude","#,
            r#""birth_client":"ROUTE_CELL","birth_model":"MODEL_CELL","#,
            r#""birth_effort":"EFFORT_CELL","birth_profile":"PROFILE_CELL"}"#
        )
        .to_owned()
    }

    fn fact() -> String {
        concat!(
            r#"{"ts":"2026-11-01T10:00:01Z","actor":"watchdog","action":"worker-fact","#,
            r#""target":"scout","target_slot":"spawned.0","birth_v":1,"#,
            r#""birth":"22222222-2222-4222-8222-222222222222","fact":"observed","#,
            r#""start":"spawned.0:x:41","observed_source":"frame","#,
            r#""observed_model":"MODEL_CELL","observed_effort":"EFFORT_CELL"}"#
        )
        .to_owned()
    }

    #[test]
    fn event_text_reads_a_summary_less_record_as_absent_not_as_damage() {
        for line in [certificate(), fact()] {
            let event = crate::event_text::event_line(line.as_bytes()).expect("one record");
            assert_eq!(
                crate::event_text::member(event, "summary").value(),
                None,
                "the family has no summary"
            );
            assert!(crate::event_text::member(event, "action").value().is_some());
        }
        let container = format!("{}\n{}\n", certificate(), fact());
        assert_eq!(
            crate::event_text::last_records(container.as_bytes(), 2),
            container.as_bytes()
        );
    }

    #[test]
    fn events_tail_prints_only_the_action_and_identity_labels() {
        for (line, action, actor) in [
            (certificate(), "worker-birth", "lead"),
            (fact(), "worker-fact", "watchdog"),
        ] {
            let row = crate::events_tail::format_event(line.as_bytes()).expect("a tail row");
            let row = String::from_utf8(row).expect("utf-8 fixture");
            assert!(row.contains(action), "{row}");
            assert!(row.contains(actor), "{row}");
            assert!(row.contains("scout"), "{row}");
            for secret in SECRET_CELLS {
                assert!(!row.contains(secret), "{row}");
            }
            assert!(!row.contains("22222222"), "{row}");
        }
    }

    #[test]
    fn the_chat_lane_loads_no_body_and_shows_no_family_item() {
        let events = [certificate(), fact()]
            .iter()
            .map(|line| Event::parse_line(line).expect("fixture event"))
            .collect::<Vec<_>>();
        let seats = [Seat {
            slot: "main".to_owned(),
            name: "lead".to_owned(),
            profile: None,
        }];
        let loads = Cell::new(0_usize);
        let body = |_: &Event| {
            loads.set(loads.get() + 1);
            Body::OldCore
        };
        let lane = lane::fold("s", &seats, &events, &Observation::default(), 0, &body);
        assert_eq!(loads.get(), 0, "no task or body is loaded");
        assert!(lane.items.is_empty(), "{:?}", lane.items);
        assert!(lane.coverage.is_empty(), "{:?}", lane.coverage);
    }
}
