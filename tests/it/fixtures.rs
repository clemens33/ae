//! The reader against hand-built session directories.
//!
//! Every fixture is named for the contract row it exercises, and every
//! expectation below is read off that row's SHOULD text — never off a bash run.
//! See `tests/fixtures/README.md` for the map and for what replaces these.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::path::{Path, PathBuf};

use ae::attention::Reason;
use ae::digest::{Digest, SessionEntry, Status};
use ae::events::{Cursor, EventLog, Identity, RefMeaning};
use ae::filters::{Scope, Selection};
use ae::json;
use ae::meta::{Anomaly, Meta};
use ae::session::{AgentRuntime, DEFAULT_UNANSWERED_SECS, SessionRead, SessionRuntime, entry_for};
use ae::time::Timestamp;

/// The moment every fixture is read as of: SC-509's own worked example stamp.
fn now() -> Timestamp {
    match Timestamp::parse("2026-05-29T14:00:00Z") {
        Some(stamp) => stamp,
        // Not `expect`: clippy's allow-*-in-tests covers `#[test]` bodies, and
        // this is a helper beside them.
        None => panic!("the documented stamp must parse"),
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sessions")
        .join(name)
}

fn drain(name: &str) -> ae::events::Drain {
    EventLog::discover(&fixture(name))
        .drain_all(Cursor::default())
        .unwrap_or_else(|err| panic!("fixture {name} should read: {err}"))
}

/// A runtime that knows only that the session is running — what every test uses
/// unless it is specifically about a runtime fact.
fn running() -> SessionRuntime {
    SessionRuntime::new(Status::Running)
}

fn entry(name: &str) -> SessionEntry {
    entry_for(
        &fixture(name),
        name,
        &running(),
        now(),
        DEFAULT_UNANSWERED_SECS,
    )
}

#[test]
fn sc_510a_every_record_carries_the_required_trio() {
    let drain = drain("sc-510a-required-keys");
    assert_eq!(drain.events.len(), 2);
    assert!(drain.skipped.is_empty());
    for event in &drain.events {
        assert!(!event.actor.is_empty());
        assert!(!event.action.is_empty());
    }
    assert_eq!(
        drain.events[0].ts,
        Timestamp::parse("2026-05-29T12:00:00Z").expect("parses")
    );
}

#[test]
fn sc_510b_an_absent_optional_key_reads_as_absent() {
    let drain = drain("sc-510b-optional-keys-omitted");
    let done = &drain.events[0];
    assert_eq!(done.target, None);
    assert_eq!(done.reference, None);
    assert_eq!(done.summary, None);

    let nudge = &drain.events[1];
    assert_eq!(nudge.target.as_deref(), Some("claude:lead"));
    assert_eq!(
        nudge.summary.as_deref(),
        Some("idle 90m, no recent ae activity")
    );
    assert_eq!(nudge.reference, None);
}

#[test]
fn sc_510c_ref_means_what_the_complete_action_table_says() {
    let drain = drain("sc-510c-ref-polysemy");
    let meanings: Vec<RefMeaning<'_>> = drain
        .events
        .iter()
        .map(ae::events::Event::ref_meaning)
        .collect();
    assert_eq!(
        meanings,
        vec![
            RefMeaning::RequestId("ae-20260529T100000Z-aaaa1111"),
            RefMeaning::RequestId("ae-20260529T100000Z-aaaa1111"),
            RefMeaning::MemoTopic("reader-design"),
            RefMeaning::CapturedSessionId("0199c0de-1234-7890-abcd-ef0123456789"),
            // The amended entry: a `state` event's ref IS its declared state.
            RefMeaning::DeclaredState("working"),
        ]
    );
}

#[test]
fn sc_510d_the_documented_escape_set_decodes_and_re_encodes() {
    let drain = drain("sc-510d-escaped-strings");
    let summary = drain.events[0]
        .summary
        .as_deref()
        .expect("the chat event carries a summary");
    assert_eq!(
        summary, "quote \" backslash \\ newline \n tab \t carriage \r done",
        "every escape in the set decodes to the byte it names"
    );

    // And back out again: a digest carrying this text stays one parseable line.
    let mut entry = SessionEntry::new("s", Status::Running);
    entry.goal = Some(summary.to_owned());
    let rendered = Digest::new(now(), vec![entry], true).render();
    assert!(
        !rendered.contains('\n'),
        "no raw newline reaches the document"
    );
    assert!(!rendered.contains('\t'), "no raw tab reaches the document");
    let reparsed = json::parse(&rendered).expect("one complete document");
    let Some(json::Value::Arr(sessions)) = reparsed.get("sessions") else {
        panic!("sessions must be an array");
    };
    assert_eq!(sessions[0].get_str("goal"), Some(summary));
}

#[test]
fn sc_511a_and_b_routing_keys_are_read_and_preferred_where_present() {
    let drain = drain("sc-511a-routing-keys");
    assert_eq!(
        drain.events[0].actor_identity(),
        Identity::Routed {
            slot: "main",
            session: "my-feature"
        }
    );
    assert_eq!(
        drain.events[0].target_identity(),
        Some(Identity::Routed {
            slot: "worker.0",
            session: "my-feature"
        })
    );
    assert_eq!(
        drain.events[1].actor_identity(),
        Identity::Display("claude:lead"),
        "a record without the keys falls back to the display name"
    );
}

#[test]
fn sc_511c_additive_keys_do_not_break_the_reader() {
    let drain = drain("sc-511c-additive-keys");
    assert_eq!(drain.events.len(), 1);
    assert!(drain.skipped.is_empty(), "an unknown key is not a defect");
    assert_eq!(drain.events[0].action, "done");
}

#[test]
fn sc_017e_the_activity_clock_is_the_newest_event_not_the_last_line() {
    let read = SessionRead::open(&fixture("sc-017e-activity-clock")).expect("reads");
    assert_eq!(
        read.last_active,
        Timestamp::parse("2026-05-29T13:57:00Z"),
        "the fixture's newest event sits in the middle of the file"
    );

    // SC-017e: "an ae event within ~5 min". 13:57 is 3 minutes before 14:00.
    let sessions = vec![entry("sc-017e-activity-clock")];
    let active = Selection {
        active_within_secs: Some(ae::filters::DEFAULT_ACTIVE_WINDOW_SECS),
        ..Selection::default()
    };
    assert_eq!(active.select(&sessions, now()).len(), 1);

    // An hour later the same session is no longer recently active.
    let later = Timestamp::from_epoch(now().epoch() + 3600);
    assert!(active.select(&sessions, later).is_empty());
}

#[test]
fn sc_017g_a_stray_reply_leaves_the_request_unanswered() {
    let read = SessionRead::open(&fixture("sc-017g-unanswered-request")).expect("reads");
    assert_eq!(read.pending.len(), 1, "the third agent did not answer it");
    assert_eq!(read.pending[0].id, "ae-20260529T090000Z-bbbb2222");
    // Sent 09:00, read at 14:00 — five hours, well past the 30-minute default.
    assert_eq!(
        read.attention_contribution(now(), DEFAULT_UNANSWERED_SECS),
        Some(Reason::Unanswered)
    );
}

#[test]
fn sc_017g_and_511b_a_renamed_replier_still_closes_its_request() {
    let read = SessionRead::open(&fixture("sc-017g-answered-request")).expect("reads");
    assert!(
        read.pending.is_empty(),
        "the routing key survived the display-name change"
    );
    assert_eq!(
        read.attention_contribution(now(), DEFAULT_UNANSWERED_SECS),
        None
    );
}

#[test]
fn sc_518_a_reply_addressed_to_a_third_party_does_not_close_the_request() {
    // The half the seats reopened after slice 1: right responder, wrong
    // recipient. Both sides carry routing keys, so the mirror is checked on
    // them — and the reply's target is spawned.0, not the asker's main.
    let read = SessionRead::open(&fixture("sc-518-reply-to-someone-else")).expect("reads");
    assert_eq!(read.pending.len(), 1);
    assert_eq!(read.pending[0].id, "ae-20260529T090000Z-dddd4444");
    assert_eq!(
        read.attention_contribution(now(), DEFAULT_UNANSWERED_SECS),
        Some(Reason::Unanswered),
        "a loud false-pending beats a silent false-closure"
    );
}

#[test]
fn sc_519_a_session_with_no_event_log_is_quiet_not_degraded() {
    let built = entry("sc-519-absent-event-log");
    assert!(!built.degraded, "ENOENT is tolerated, not loss");
    assert_eq!(built.last_active_epoch, None);
    assert_eq!(built.mode.as_deref(), Some("local"));
    assert_eq!(built.agents.len(), 1, "the roster still names its agents");
    assert_eq!(built.agents[0].reference, "claude:lead");
}

#[test]
fn sc_509b_a_session_whose_meta_is_gone_is_degraded_and_says_so() {
    let built = entry("sc-509b-meta-missing");
    assert!(built.degraded);
    assert_eq!(built.name, "sc-509b-meta-missing", "identity survives");
    assert_eq!(built.mode, None, "nothing is fabricated");
    assert!(built.agents.is_empty());
    assert_eq!(
        built.to_json().get("degraded"),
        Some(&json::Value::Bool(true)),
        "and the loss reaches the public JSON"
    );
}

#[test]
fn sc_520_a_malformed_record_is_skipped_reported_and_degrades_the_session() {
    let drain = drain("sc-520-malformed-record");
    assert_eq!(drain.events.len(), 3, "every well-formed record is read");
    assert_eq!(
        drain.skipped.len(),
        2,
        "and both bad lines are accounted for"
    );
    assert!(drain.drained);
    assert_eq!(
        drain.events[2].ref_meaning(),
        RefMeaning::MemoTopic("after-the-damage"),
        "a bad line does not stop the scan"
    );
    // SC-520: generation + offset + reason retained internally...
    assert_eq!(drain.skipped[0].generation, 0);
    assert!(drain.skipped[0].offset > 0);

    // ...and the session marked degraded in the public JSON.
    let built = entry("sc-520-malformed-record");
    assert!(built.degraded);
    assert_eq!(
        built.to_json().get("degraded"),
        Some(&json::Value::Bool(true))
    );
    assert!(
        built.last_active_epoch.is_some(),
        "the records that DID read are still reported"
    );
}

#[test]
fn sc_405b_and_c_the_meta_supplies_the_context_fields_and_the_roster() {
    let built = entry("sc-405c-roster");
    assert!(!built.degraded);
    assert_eq!(built.mode.as_deref(), Some("local"));
    assert_eq!(built.origin.as_deref(), Some("/home/c/projects/ae"));
    assert_eq!(built.work_dir.as_deref(), Some("/home/c/projects/ae"));

    assert_eq!(built.agents.len(), 2);
    assert_eq!(built.agents[0].reference, "claude:lead");
    assert_eq!(built.agents[0].alias, "claude");
    assert_eq!(built.agents[0].name, "lead");
    assert_eq!(built.agents[0].session_id.as_deref(), Some("e795c9e9"));
    assert_eq!(built.agents[1].reference, "codex:coworker");
    assert_eq!(
        built.agents[1].session_id, None,
        "the roster's session id is optional"
    );
}

#[test]
fn sc_510c_declared_states_reach_the_agents_and_roll_up() {
    let built = entry("sc-405c-roster");
    // lead declared working at 13:00 then waiting-user at 13:40; coworker
    // declared blocked at 13:30.
    assert_eq!(built.agents[0].state.as_deref(), Some("waiting-user"));
    assert_eq!(built.agents[0].reason, Some(Reason::WaitingUser));
    assert_eq!(built.agents[1].state.as_deref(), Some("blocked"));
    assert_eq!(built.agents[1].reason, Some(Reason::Blocked));
    assert_eq!(
        built.attention,
        Some(Reason::WaitingUser),
        "the single most-actionable reason across the session's agents"
    );
    assert_eq!(
        built.to_json().get("attention_rank"),
        Some(&json::Value::Num(4))
    );
}

#[test]
fn sc_405d_an_unknown_meta_key_is_tolerated_and_never_degrades() {
    let meta = Meta::read(&fixture("sc-405d-unknown-key")).expect("the meta reads");
    assert_eq!(
        meta.anomalies(),
        [Anomaly::UnknownKey {
            key: "ae_path".to_owned(),
            line: 6
        }],
        "the reader still SEES it"
    );

    let built = entry("sc-405d-unknown-key");
    assert!(
        !built.degraded,
        "but an unknown key is the normal state of a real meta, not damage"
    );
    assert_eq!(built.mode.as_deref(), Some("local"));
    assert_eq!(built.agents.len(), 1);
}

#[test]
fn sc_405e_a_meta_shape_the_reader_could_not_take_degrades() {
    let built = entry("sc-405e-malformed-meta");
    assert!(
        built.degraded,
        "a line the reader could not take is real loss"
    );
    assert_eq!(
        built.mode.as_deref(),
        Some("local"),
        "and the keys around it are still read"
    );
    assert_eq!(
        built.to_json().get("degraded"),
        Some(&json::Value::Bool(true))
    );
}

#[test]
fn sc_405j_a_routed_event_with_a_stale_session_stays_unassociated() {
    // The display name matches the roster exactly; the routing key's session
    // does not, because the session was renamed after the event was written.
    // Attributing it by name would be a false attribution, so the state is lost
    // loudly instead — the known limitation until SC-977's stable identity.
    let built = entry("sc-405j-stale-session");
    assert!(
        !built.degraded,
        "a stale association is not read/parse loss"
    );
    assert_eq!(built.agents.len(), 1);
    assert_eq!(built.agents[0].state, None);
    assert_eq!(built.agents[0].reason, None);
    assert_eq!(built.attention, None);
}

#[test]
fn sc_405f_the_goal_epoch_comes_from_the_latest_goal_event() {
    let built = entry("sc-405f-goal-event");
    assert_eq!(
        built.goal.as_deref(),
        Some("ship the login flow"),
        "the goal TEXT is a meta key (SC-405b)"
    );
    assert_eq!(
        built.goal_set_epoch,
        Timestamp::parse("2026-05-29T11:30:00Z").map(Timestamp::epoch),
        "the EPOCH is the latest goal event (SC-405f), not the earlier one"
    );
}

#[test]
fn sc_405g_the_branch_is_a_runtime_input_not_a_meta_key() {
    let runtime = SessionRuntime {
        status: Status::Running,
        branch: Some("feature/login".to_owned()),
        agents: vec![AgentRuntime {
            slot: "main".to_owned(),
            alive: true,
            alert: None,
        }],
    };
    let built = entry_for(
        &fixture("sc-405c-roster"),
        "live",
        &runtime,
        now(),
        DEFAULT_UNANSWERED_SECS,
    );
    assert_eq!(built.branch.as_deref(), Some("feature/login"));
    assert!(built.agents[0].alive, "alive is a runtime fact too");
    assert!(
        !built.agents[1].alive,
        "a slot the runtime does not mention is not alive"
    );
}

#[test]
fn sc_980_a_typed_alert_outranks_a_self_declaration() {
    let runtime = SessionRuntime {
        status: Status::Running,
        branch: None,
        agents: vec![AgentRuntime {
            slot: "worker.0".to_owned(),
            alive: false,
            alert: Some(Reason::Dead),
        }],
    };
    let built = entry_for(
        &fixture("sc-405c-roster"),
        "live",
        &runtime,
        now(),
        DEFAULT_UNANSWERED_SECS,
    );
    assert_eq!(built.agents[1].reason, Some(Reason::Dead));
    assert_eq!(
        built.agents[1].state.as_deref(),
        Some("blocked"),
        "the declaration is still reported beside it"
    );
    assert_eq!(built.attention, Some(Reason::Dead));
}

#[test]
fn dr_001_the_partial_tail_fixture_really_is_partial() {
    // Asserted before anything is read: if a tool normalised the file, this
    // test says so instead of the next one passing for the wrong reason.
    let bytes = std::fs::read(fixture("dr-001-partial-tail").join("events.jsonl"))
        .expect("the fixture exists");
    assert_ne!(
        bytes.last(),
        Some(&b'\n'),
        "the fixture must end mid-record; something normalised it"
    );
}

#[test]
fn dr_001_a_partial_record_is_left_for_the_writer_to_finish() {
    let log = EventLog::discover(&fixture("dr-001-partial-tail"));
    let drain = log.drain(Cursor::default()).expect("reads");

    assert_eq!(drain.events.len(), 1, "only the complete record");
    assert!(
        drain.skipped.is_empty(),
        "an unfinished write is not an error"
    );
    assert!(!drain.drained, "the generation is not finished");
    assert_eq!(
        log.next_cursor(&drain),
        drain.cursor,
        "and an unfinished generation is never left behind"
    );

    let bytes = std::fs::read(fixture("dr-001-partial-tail").join("events.jsonl"))
        .expect("the fixture exists");
    let first_newline = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .expect("the fixture holds one complete record");
    assert_eq!(
        drain.cursor.offset,
        first_newline as u64 + 1,
        "the cursor lands just past the complete record, never inside the partial one"
    );
    assert_eq!(drain.cursor.generation, 0);

    // SC-520 is about a malformed COMPLETE record; a buffered unterminated tail
    // is not one (SC-975b), so this session is NOT degraded.
    assert!(!entry("dr-001-partial-tail").degraded);
}

#[test]
fn sc_506_and_509b_one_damaged_session_degrades_alone_and_the_document_closes() {
    let names = [
        "sc-017e-activity-clock",
        "sc-509b-meta-missing",
        "sc-017g-unanswered-request",
    ];
    let sessions: Vec<SessionEntry> = names.iter().map(|name| entry(name)).collect();

    assert!(!sessions[0].degraded);
    assert!(sessions[1].degraded);
    assert!(!sessions[2].degraded);

    let rendered = Digest::new(now(), sessions, true).render();
    let value = json::parse(&rendered).expect("a complete, parseable document");
    let Some(json::Value::Arr(entries)) = value.get("sessions") else {
        panic!("sessions must be an array");
    };
    assert_eq!(
        entries.len(),
        3,
        "the bad session degrades, it does not vanish"
    );
    assert_eq!(entries[0].get("degraded"), None, "healthy entries omit it");
    assert_eq!(entries[1].get_str("name"), Some("sc-509b-meta-missing"));
    assert_eq!(entries[1].get("degraded"), Some(&json::Value::Bool(true)));
    assert_eq!(
        entries[2].get_str("attention"),
        Some("unanswered"),
        "and the session AFTER the damaged one is intact"
    );
}

#[test]
fn sc_509_and_017f_the_digest_carries_exactly_what_the_filters_selected() {
    let live = entry("sc-017e-activity-clock");
    let flagged = entry("sc-017g-unanswered-request");
    let old = entry_for(
        &fixture("sc-510a-required-keys"),
        "old",
        &SessionRuntime::new(Status::Stopped),
        now(),
        DEFAULT_UNANSWERED_SECS,
    );
    let sessions = vec![live, flagged, old];

    let selected: Vec<SessionEntry> = Selection {
        scope: Scope::All,
        needs_attention: true,
        ..Selection::default()
    }
    .select(&sessions, now())
    .into_iter()
    .cloned()
    .collect();

    let rendered = Digest::new(now(), selected, true).render();
    let value = json::parse(&rendered).expect("one document");
    assert_eq!(
        value.get("schema_version"),
        Some(&json::Value::Num(ae::digest::SCHEMA_VERSION))
    );
    assert_eq!(value.get_str("generated_at"), Some("2026-05-29T14:00:00Z"));
    let Some(json::Value::Arr(entries)) = value.get("sessions") else {
        panic!("sessions must be an array");
    };
    assert_eq!(
        entries.len(),
        1,
        "only the attention session survives the filter"
    );
    assert_eq!(
        entries[0].get_str("name"),
        Some("sc-017g-unanswered-request")
    );
    assert_eq!(
        entries[0].get("needs_attention"),
        Some(&json::Value::Bool(true))
    );
    assert_eq!(entries[0].get("attention_rank"), Some(&json::Value::Num(1)));
}

/// Every fixture directory, by name, in sorted order.
///
/// The NAMES, not a count: a count passes when one fixture is deleted and
/// another added, which is the exact edit a corpus rots through. Each name is
/// also the row it exercises, so this list doubles as the map the README
/// carries in prose.
const FIXTURES: [&str; 19] = [
    "dr-001-partial-tail",
    "sc-017e-activity-clock",
    "sc-017g-answered-request",
    "sc-017g-unanswered-request",
    "sc-405c-roster",
    "sc-405d-unknown-key",
    "sc-405e-malformed-meta",
    "sc-405f-goal-event",
    "sc-405j-stale-session",
    "sc-509b-meta-missing",
    "sc-510a-required-keys",
    "sc-510b-optional-keys-omitted",
    "sc-510c-ref-polysemy",
    "sc-510d-escaped-strings",
    "sc-511a-routing-keys",
    "sc-511c-additive-keys",
    "sc-518-reply-to-someone-else",
    "sc-519-absent-event-log",
    "sc-520-malformed-record",
];

#[test]
fn the_fixture_corpus_is_exactly_the_named_set() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sessions");
    let mut found: Vec<String> = std::fs::read_dir(&root)
        .expect("the fixture root exists")
        .map(|dir_entry| dir_entry.expect("a readable dir entry").path())
        .filter(|path| path.is_dir())
        .map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .expect("a utf-8 fixture name")
                .to_owned()
        })
        .collect();
    found.sort();
    assert_eq!(
        found, FIXTURES,
        "the corpus is not the named set — add or remove the name here and in the README map"
    );
}

#[test]
fn every_fixture_directory_is_read_without_a_panic() {
    // Never panics, by SC-506's construction — whatever the directory holds.
    for name in FIXTURES {
        let built = entry(name);
        assert_eq!(built.name, name);
        // SC-509b: identity survives every degradation, and `agents` is always
        // an array whether or not the read went well.
        assert!(matches!(
            built.to_json().get("agents"),
            Some(json::Value::Arr(_))
        ));
    }
}
