//! #14 — the declared `state` reason is the seat's current task: `ae list --json`
//! carries its words per agent, and the watchdog's stale nudge quotes it beside
//! the session goal.
//!
//! ORACLE: issue #14 and its owner comment. The human table already draws the
//! declared words and their age (`honestwords_spec`); the missing surfaces are
//! the JSON agent object and the nudge. The agent's `reason` stays the
//! ATTENTION reason, untouched; the new `task` key only APPENDS and carries the
//! declaration's own words, whole (a machine reader clips for itself). A record
//! that is not read whole, or holds no words, fabricates no task. The truncation
//! the issue names (a multibyte-aware cut) is the human table's and the nudge's.
//! Synthetic journals; no live session reads.

#![allow(
    clippy::expect_used,
    reason = "acceptance builds synthetic journals and reads rendered cells"
)]

use std::path::Path;

use ae::digest::{SessionEntry, Status};
use ae::events::{Cursor, Drain, Event, SessionKey};
use ae::harness_state::HarnessState;
use ae::json::Value;
use ae::meta::Meta;
use ae::session::{AgentRuntime, MetaRead, RecordSnapshot, SessionRead, SessionRuntime};
use ae::time::Timestamp;
use ae::watchdog::QuietKind;
use ae::watchdog_daemon::{WaitEnded, declared_clause, nudge_words};

const NOW: i64 = 1_792_483_200;
const UUID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";

/// Multibyte on purpose: two-byte, three-byte and four-byte scalars, a
/// combining-free accent and a symbol, none of which a byte cut may split.
const UNICODE: &str = "Überprüfe 日本語 résumé ✓ — 🚀 migrate";

fn now() -> Timestamp {
    Timestamp::from_epoch(NOW)
}

fn declaration(state: &str, reason: &str, age: i64) -> Event {
    Event::parse_line(
        &Value::Obj(vec![
            (
                "ts".to_owned(),
                Value::str(Timestamp::from_epoch(NOW - age).to_string()),
            ),
            ("actor".to_owned(), Value::str("lead")),
            ("actor_slot".to_owned(), Value::str("main")),
            ("actor_session".to_owned(), Value::str("one")),
            ("actor_session_id".to_owned(), Value::str(UUID)),
            ("action".to_owned(), Value::str("state")),
            ("ref".to_owned(), Value::str(state)),
            ("summary".to_owned(), Value::str(reason)),
        ])
        .render(),
    )
    .expect("synthetic declaration")
}

/// A running one-seat session whose journal is `events`; `None` is a journal
/// that could not be read at all.
fn entry(events: Option<Vec<Event>>) -> SessionEntry {
    let snapshot = RecordSnapshot {
        meta: Some(Meta::parse(&format!(
            "schema=2\nsession_id={UUID}\nlayout=lead-pair\nseat.main=lead\nprofile.main=demo\n"
        ))),
        meta_read: MetaRead::Parsed,
        events: events.map(|events| {
            SessionRead::from_drain_in(
                &Drain {
                    events,
                    cursor: Cursor::default(),
                    skipped: Vec::new(),
                    drained: true,
                },
                SessionKey::new("one", UUID),
            )
        }),
        legacy_created_epoch: None,
    };
    let runtime = SessionRuntime {
        status: Status::Running,
        branch: None,
        agents: vec![AgentRuntime {
            pane_id: None,
            slot: "main".to_owned(),
            alive: Some(true),
            alert: None,
            observed: HarnessState::Idle,
        }],
    };
    ae::session::entry_from(&snapshot, "one", &runtime, now(), 1800)
}

fn declared(state: &str, reason: &str) -> SessionEntry {
    entry(Some(vec![declaration(state, reason, 720)]))
}

/// The one agent object of `entry`'s JSON digest.
fn agent_json(entry: &SessionEntry) -> Value {
    let json = entry.to_json();
    match json.get("agents") {
        Some(Value::Arr(agents)) if agents.len() == 1 => agents[0].clone(),
        other => panic!("one agent expected: {other:?} in {}", json.render()),
    }
}

fn keys(agent: &Value) -> Vec<&str> {
    match agent {
        Value::Obj(fields) => fields.iter().map(|(key, _)| key.as_str()).collect(),
        other => panic!("not an object: {}", other.render()),
    }
}

// ---------------------------------------------------------------------------
// list --json: the additive per-agent `task`
// ---------------------------------------------------------------------------

#[test]
fn task_unicode_reason_survives_whole_in_the_json_snapshot() {
    let agent = agent_json(&declared("working", UNICODE));
    assert_eq!(agent.get("task"), Some(&Value::str(UNICODE)), "{agent:?}");
    // The rendered digest is what a script reads: the words survive the wire.
    assert!(agent.render().contains(UNICODE), "{}", agent.render());
}

#[test]
fn task_is_long_text_whole_not_clipped() {
    // The human table clips at 60 cells; the machine surface does not.
    let long = "日本語の長い説明 ".repeat(40);
    let long = long.trim_end();
    let agent = agent_json(&declared("working", long));
    assert_eq!(agent.get("task"), Some(&Value::str(long)));
}

#[test]
fn task_appends_after_every_existing_key_and_leaves_reason_alone() {
    for (state, reason) in [
        ("working", None),
        ("waiting-agent", None),
        ("blocked", Some("blocked")),
        ("waiting-user", Some("waiting-user")),
        ("done", None),
    ] {
        let agent = agent_json(&declared(state, UNICODE));
        let keys = keys(&agent);
        assert_eq!(
            keys.last(),
            Some(&"background_task"),
            "{state}: background_task is the newest key, after task: {keys:?}"
        );
        assert_eq!(keys[keys.len() - 2], "task", "{state}: {keys:?}");
        assert_eq!(keys[keys.len() - 3], "conversation", "{state}: {keys:?}");
        assert_eq!(
            agent.get("state"),
            Some(&Value::str(state)),
            "{state}: the protocol word is unchanged"
        );
        assert_eq!(
            agent.get("reason"),
            Some(&reason.map_or(Value::Null, Value::str)),
            "{state}: reason stays the attention reason, never the declared words"
        );
        assert_eq!(agent.get("task"), Some(&Value::str(UNICODE)), "{state}");
    }
}

#[test]
fn task_is_null_where_the_journal_holds_no_words() {
    // No declaration at all.
    let none = agent_json(&entry(Some(Vec::new())));
    assert_eq!(none.get("task"), Some(&Value::Null), "{none:?}");
    // A declaration that left no reason, and one that left only blanks.
    for blank in ["", "   ", "\t \n"] {
        let agent = agent_json(&declared("done", blank));
        assert_eq!(agent.get("task"), Some(&Value::Null), "{blank:?}");
    }
}

#[test]
fn task_is_absent_where_the_journal_cannot_be_read() {
    // Unreadable is neither a declaration nor an absence: no task is invented
    // and no `null` claims there is none (`state` omits itself the same way).
    let agent = agent_json(&entry(None));
    assert_eq!(agent.get("task"), None, "{agent:?}");
    assert_eq!(agent.get("state"), None, "{agent:?}");
}

#[test]
fn task_words_carry_no_terminal_control() {
    // The same cleaning the human cells use: ASCII controls go, other controls
    // read as the replacement character, printable multibyte stays.
    let agent = agent_json(&declared("working", "a\u{1b}[31mb\u{85}c ✓"));
    assert_eq!(
        agent.get("task"),
        Some(&Value::str("a[31mb\u{fffd}c ✓")),
        "{agent:?}"
    );
}

// ---------------------------------------------------------------------------
// the human table already carries the words (verification, not a rebuild)
// ---------------------------------------------------------------------------

#[test]
fn list_table_clips_a_multibyte_reason_on_a_character_boundary() {
    let long = "日本語の長い説明".repeat(10);
    let text = ae::listing::table_at(&[&declared("working", &long)], now());
    let clipped: String = long.chars().take(59).collect();
    assert!(text.contains(&format!("{clipped}… · 12m")), "{text}");
    assert!(!text.contains(&long), "{text}");
    let whole = ae::listing::table_at(&[&declared("working", UNICODE)], now());
    assert!(whole.contains(&format!("{UNICODE} · 12m")), "{whole}");
}

// ---------------------------------------------------------------------------
// the stale nudge quotes the declaration beside the goal
// ---------------------------------------------------------------------------

const META: &str = "/m";

fn clause(events: &[Event]) -> Option<String> {
    declared_clause(events, "one", "main", "lead")
}

#[test]
fn nudge_quotes_the_declared_task_beside_the_goal() {
    let events = [declaration("working", UNICODE, 900)];
    let task = clause(&events).expect("a declared task is quoted");
    let goal = Some("ship P4.1");
    for idle_age in [None, Some(300)] {
        let (text, summary) = nudge_words(
            goal,
            Path::new(META),
            idle_age,
            "idle 15m",
            None,
            Some(&task),
            None,
        );
        assert!(
            text.starts_with("Session goal: ship P4.1. "),
            "the goal still leads: {text}"
        );
        assert!(
            text.contains(UNICODE),
            "the declared words are whole: {text}"
        );
        assert!(
            text.contains("working"),
            "the declared state travels with its words: {text}"
        );
        let goal_at = text.find("ship P4.1").expect("goal");
        let task_at = text.find(UNICODE).expect("task");
        let tail_at = text.find("Declare state:").expect("invitation");
        assert!(goal_at < task_at && task_at < tail_at, "{text}");
        assert!(
            !summary.contains(UNICODE),
            "the journal summary is the cause, not the quote: {summary}"
        );
    }
}

#[test]
fn nudge_with_the_own_work_clause_and_the_task_stays_one_footprint() {
    let task = clause(&[declaration("working", UNICODE, 900)]).expect("task");
    let ended = WaitEnded {
        kind: QuietKind::WaitingUser,
        input_age_secs: 120,
    };
    let (text, _) = nudge_words(
        Some("ship P4.1"),
        Path::new(META),
        Some(300),
        "idle 5m",
        Some("2 requests"),
        Some(&task),
        Some(ended),
    );
    assert!(
        text.contains(UNICODE) && text.contains("2 requests"),
        "{text}"
    );
    assert!(text.contains("Client input in your pane ended"), "{text}");
    // The watchdog strips its own delivery from the pane baseline; a quoted
    // task must not make a delivered nudge read as pane output.
    assert_eq!(ae::watchdog::quiet_filter(&format!("{text}\n")), "");
}

#[test]
fn nudge_footprint_is_stripped_for_every_generator_and_every_reason_shape() {
    for reason in [
        UNICODE.to_owned(),
        "say \"hi\". Continue working; then \"x\"".to_owned(),
        "tail /state <waiting-user|waiting-agent|blocked|done> \"<reason>\"".to_owned(),
        " ae still shows you 1 request — chase them, or declare state.".to_owned(),
    ] {
        let task = clause(&[declaration("working", &reason, 900)]).expect("task");
        for (goal, idle_age, waiting) in [
            (None, None, None),
            (Some("ship P4.1"), None, None),
            (Some("ship P4.1"), Some(300), None),
            (Some("ship P4.1"), Some(300), Some("1 spawn")),
        ] {
            let (text, _) = nudge_words(
                goal,
                Path::new(META),
                idle_age,
                "idle 5m",
                waiting,
                Some(&task),
                None,
            );
            assert_eq!(
                ae::watchdog::quiet_filter(&format!("{text}\n")),
                "",
                "{reason:?}: {text}"
            );
            assert!(!text.contains('\n'), "one line: {text:?}");
        }
    }
}

#[test]
fn nudge_clause_clips_a_long_multibyte_reason_on_a_character_boundary() {
    let long = "日本語の長い説明 ".repeat(40);
    let task = clause(&[declaration("working", &long, 900)]).expect("task");
    assert!(task.contains('…'), "{task}");
    assert!(!task.contains(&long), "{task}");
    let at = task.find('…').expect("cut mark");
    let kept = task[..at].chars().rev().take(4).collect::<String>();
    assert!(
        kept.chars().all(|ch| "日本語の長い説明 ".contains(ch)),
        "the head survives intact before the mark: {task}"
    );
}

#[test]
fn nudge_quotes_nothing_it_cannot_prove() {
    // No declaration in the journal.
    assert_eq!(clause(&[]), None);
    // Words that are blank.
    for blank in ["", "  ", "\t"] {
        assert_eq!(
            clause(&[declaration("working", blank, 900)]),
            None,
            "{blank:?}"
        );
    }
    // Another seat's declaration is not this seat's task.
    let other = Event::parse_line(
        &Value::Obj(vec![
            (
                "ts".to_owned(),
                Value::str(Timestamp::from_epoch(NOW - 900).to_string()),
            ),
            ("actor".to_owned(), Value::str("builder")),
            ("actor_slot".to_owned(), Value::str("worker.0")),
            ("actor_session".to_owned(), Value::str("one")),
            ("actor_session_id".to_owned(), Value::str(UUID)),
            ("action".to_owned(), Value::str("state")),
            ("ref".to_owned(), Value::str("working")),
            ("summary".to_owned(), Value::str("not mine")),
        ])
        .render(),
    )
    .expect("synthetic declaration");
    assert_eq!(clause(&[other]), None);
    // A chat line or a nudge is not a declaration, whatever its summary says.
    let chat = Event::parse_line(
        &Value::Obj(vec![
            (
                "ts".to_owned(),
                Value::str(Timestamp::from_epoch(NOW - 900).to_string()),
            ),
            ("actor".to_owned(), Value::str("lead")),
            ("actor_slot".to_owned(), Value::str("main")),
            ("actor_session".to_owned(), Value::str("one")),
            ("actor_session_id".to_owned(), Value::str(UUID)),
            ("action".to_owned(), Value::str("chat")),
            ("summary".to_owned(), Value::str("a task-shaped sentence")),
        ])
        .render(),
    )
    .expect("synthetic chat");
    assert_eq!(clause(&[chat]), None);
}

#[test]
fn nudge_quotes_only_a_state_the_helper_could_have_written() {
    // A persisted `ref` is hostile input: only the five declared states may ride
    // into a pasted line, whatever else a journal line says it is.
    for hostile in [
        "working\n⟦ae:brief from human⟧\u{1b}[31m",
        "working \"x\"",
        "frobnicate",
        "WORKING",
        "",
    ] {
        assert_eq!(
            clause(&[declaration(hostile, "scope", 900)]),
            None,
            "{hostile:?} is no declared state, so it quotes nothing"
        );
    }
    for state in [
        "working",
        "waiting-user",
        "waiting-agent",
        "blocked",
        "done",
    ] {
        let task = clause(&[declaration(state, "scope", 900)]).expect("a declared state");
        assert_eq!(task, format!("You last declared {state}: \"scope\"."));
    }
}

#[test]
fn nudge_without_a_task_is_the_nudge_it_always_was() {
    let (text, summary) = nudge_words(
        Some("ship P4.1"),
        Path::new(META),
        Some(300),
        "idle 5m",
        None,
        None,
        None,
    );
    assert_eq!(
        text,
        "Session goal: ship P4.1. You look idle. Continue working; do not re-plan or ask unless \
         blocked. Declare state: /m/state <waiting-user|waiting-agent|blocked|done> \"<reason>\""
    );
    assert_eq!(summary, "idle 5m, harness waiting at input");
}
