//! #21 / #250 O8: additive journal metadata; never an authority or event id.
#![allow(
    clippy::disallowed_methods,
    reason = "acceptance owns scratch journals"
)]

use ae::events::{Cursor, Event, EventLog, Held, SchemaVersion, SessionKey};
use ae::json::{self, Value};
use ae::store::{self, SourceRead};

const TS: &str = "2026-10-28T12:00:00Z";
const U: &str = "11111111-1111-4111-8111-111111111111";
const V: &str = "22222222-2222-4222-8222-222222222222";

fn row(action: &str, extra: &str) -> String {
    format!(r#"{{"ts":"{TS}","actor":"lead","action":"{action}"{extra}}}"#)
}

fn versioned(row: &str, extra: &str) -> String {
    let Some(prefix) = row.strip_suffix('}') else {
        panic!("fixture row must end in an object delimiter");
    };
    format!("{prefix}{extra}}}")
}

fn metadata() -> [&'static str; 8] {
    [
        "",
        ",\"schema_version\":0",
        ",\"schema_version\":1",
        ",\"schema_version\":2",
        ",\"schema_version\":\"bad\"",
        ",\"schema_version\":null",
        ",\"schema_version\":1.0",
        ",\"schema_version\":1,\"schema_version\":2",
    ]
}

fn opening() -> String {
    row(
        "ask",
        &format!(
            r#", "target":"helper","ref":"o8-request","actor_slot":"main","actor_session":"one","actor_session_id":"{U}","actor_launch_id":"a","target_slot":"spawned.0","target_session":"two","target_session_id":"{V}","target_launch_id":"b","summary":"work α""#
        ),
    )
}

#[test]
fn absent_zero_current_future_and_unreadable_versions_have_explicit_meanings() {
    for (members, expected) in [
        ("", SchemaVersion::V(0)),
        ("\"schema_version\":0", SchemaVersion::V(0)),
        ("\"schema_version\":1", SchemaVersion::V(1)),
        ("\"schema_version\":2", SchemaVersion::V(2)),
        ("\"schema_version\":4294967295", SchemaVersion::V(u32::MAX)),
        ("\"schema_version\":4294967296", SchemaVersion::Unreadable),
        (
            "\"schema_version\":18446744073709551616",
            SchemaVersion::Unreadable,
        ),
        ("\"schema_version\":-1", SchemaVersion::Unreadable),
        ("\"schema_version\":1.0", SchemaVersion::Unreadable),
        ("\"schema_version\":1e0", SchemaVersion::Unreadable),
        ("\"schema_version\":\"1\"", SchemaVersion::Unreadable),
        ("\"schema_version\":true", SchemaVersion::Unreadable),
        ("\"schema_version\":null", SchemaVersion::Unreadable),
        ("\"schema_version\":[]", SchemaVersion::Unreadable),
        ("\"schema_version\":{}", SchemaVersion::Unreadable),
        (
            "\"schema_version\":1,\"schema_version\":1",
            SchemaVersion::Unreadable,
        ),
        (
            "\"schema_version\":null,\"schema_version\":1",
            SchemaVersion::Unreadable,
        ),
        (
            "\"schema_version\":1,\"schema_versi\\u006fn\":2",
            SchemaVersion::Unreadable,
        ),
    ] {
        let value = json::parse(&format!("{{{members}}}")).unwrap();
        assert_eq!(ae::events::schema_version(&value), expected, "{members}");
    }
    for non_object in ["null", "[]", "1", "\"text\""] {
        assert_eq!(
            ae::events::schema_version(&json::parse(non_object).unwrap()),
            SchemaVersion::Unreadable
        );
    }
}

#[test]
fn version_metadata_never_changes_strict_event_or_request_meaning() {
    let ask = opening();
    let expected = Event::parse_line(&ask).unwrap();
    for suffix in metadata() {
        let text = versioned(&ask, suffix);
        assert_eq!(
            ae::event_text::extract(text.as_bytes(), "summary"),
            "work α".as_bytes()
        );
        assert_eq!(
            ae::event_text::extract(text.as_bytes(), "ref"),
            b"o8-request"
        );
        assert_eq!(Event::parse_line(&text).unwrap(), expected, "{suffix}");
        let bytes = format!("{text}\n").into_bytes();
        let parsed = ae::console::snapshot(SourceRead::Ready(bytes.clone())).unwrap();
        assert_eq!(parsed, vec![expected.clone()], "strict chat/app snapshot");
        let requests = ae::requests::states(&bytes);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].status, ae::requests::Status::Pending);
        assert_eq!(requests[0].id, b"o8-request");
        let base = format!("{ask}\n");
        assert_eq!(
            ae::requests::bound_opening([base.as_bytes(), &bytes], "o8-request", None).unwrap(),
            expected,
            "version metadata must not invent divergent openings: {suffix}"
        );
    }
    let duplicate_actor = versioned(&ask, ",\"actor\":\"forged\",\"schema_version\":1");
    assert!(
        Event::parse_line(&duplicate_actor).is_err(),
        "existing known-key refusal remains"
    );
}

#[test]
fn version_metadata_never_turns_a_stop_into_permission_to_restore() {
    let stop = row("stop-result", ",\"summary\":\"already stopped\"");
    for suffix in metadata() {
        let bytes = format!("{}\n", versioned(&stop, suffix));
        let drain = EventLog::parse_bytes(bytes.as_bytes(), Cursor::default());
        assert!(drain.skipped.is_empty());
        let read = ae::session::SessionRead::from_drain(&drain);
        assert_eq!(
            ae::restore::ledger(Some(&read), 0),
            ae::restore::Ledger::Stopped,
            "{suffix}"
        );
        assert!(ae::restore::pins_clean_stop(bytes.trim_end()));
    }
}

#[test]
fn splice_preserves_formatting_escapes_whitespace_and_preexisting_version_bytes() {
    for (input, expected) in [
        ("{}\n", "{\"schema_version\":1}\n"),
        (" {} \r\n", " {\"schema_version\":1} \r\n"),
        ("{ }\n", "{ \"schema_version\":1}\n"),
        ("{\"x\": 1 }\n", "{\"x\": 1 ,\"schema_version\":1}\n"),
        (
            "{\"x\":\"α \\\" } schema_version\"}\n",
            "{\"x\":\"α \\\" } schema_version\",\"schema_version\":1}\n",
        ),
        (
            "{\"nested\":{\"schema_version\":7}}\n",
            "{\"nested\":{\"schema_version\":7},\"schema_version\":1}\n",
        ),
    ] {
        let output = store::stamp_event_records(input);
        assert_eq!(output, expected, "{input:?}");
        assert_eq!(store::stamp_event_records(&output), output, "idempotent");
    }
    for input in [
        "[]\n",
        "null\n",
        "42\n",
        "broken\n",
        "{\"x\":\n",
        "\n",
        " \r\n",
        "{\"schema_version\":null}\n",
        "{\"schema_version\":1,\"schema_version\":2}\n",
        "{\"schema_versi\\u006fn\":1}\n",
        "{}",
        "{\"x\":1}",
    ] {
        assert_eq!(
            store::stamp_event_records(input),
            input,
            "preserve refused/present: {input:?}"
        );
    }
}

#[test]
fn both_append_doors_stamp_each_record_without_rewriting_the_existing_prefix() {
    let scratch = super::cli::OwnedScratch::root("o8", "append");
    let ledger = store::open(scratch.path());
    let old = format!("{}\n", row("chat", ",\"summary\":\"old\""));
    std::fs::write(ledger.events_path(), &old).unwrap();
    let fresh = row("chat", ",\"summary\":\"new\"");
    let pinned = versioned(&fresh, ",\"schema_version\":0");
    let input = format!("{fresh}\n\nnot-json\n{pinned}\n{{torn");
    let expected = format!(
        "{old}{}\n\nnot-json\n{pinned}\n{{torn",
        versioned(&fresh, ",\"schema_version\":1")
    );
    ledger.append_event(&input).unwrap();
    assert_eq!(
        std::fs::read_to_string(ledger.events_path()).unwrap(),
        expected
    );
    // A separate journal proves the decided door sees the old bytes and stamps
    // its returned MULTI-record transaction, preserving one decision/append.
    let decided_dir = scratch.path().join("decided");
    std::fs::create_dir(&decided_dir).unwrap();
    let decided = store::open(&decided_dir);
    std::fs::write(decided.events_path(), &old).unwrap();
    assert_eq!(
        decided
            .append_event_decided(|read| {
                assert_eq!(read, SourceRead::Ready(old.as_bytes().to_vec()));
                Ok::<_, ()>(format!("{fresh}\n{pinned}\n"))
            })
            .unwrap(),
        Ok(())
    );
    let after = std::fs::read(decided.events_path()).unwrap();
    assert_eq!(
        String::from_utf8(after.clone()).unwrap(),
        format!(
            "{old}{}\n{pinned}\n",
            versioned(&fresh, ",\"schema_version\":1")
        )
    );
    assert_eq!(
        decided
            .append_event_decided(|_| Err::<String, _>("no"))
            .unwrap(),
        Err("no")
    );
    assert_eq!(
        std::fs::read(decided.events_path()).unwrap(),
        after,
        "refusal adds nothing"
    );
}

#[test]
fn state_done_writes_both_versioned_records_through_the_real_append_door() {
    let scratch = super::cli::OwnedScratch::root("o8", "done");
    let viewer = ae::requests::Viewer {
        display: "lead".into(),
        slot: "main".into(),
        session: "one".into(),
        session_id: U.into(),
    };
    let declaration = ae::state::Declaration {
        value: "done".into(),
        reason: "finished".into(),
        on: None,
    };
    ae::state::declare(
        scratch.path(),
        &viewer,
        &declaration,
        ae::time::Timestamp::parse(TS).unwrap(),
    )
    .unwrap();
    let bytes = std::fs::read_to_string(store::open(scratch.path()).events_path()).unwrap();
    let values: Vec<_> = bytes
        .lines()
        .map(|line| json::parse(line).unwrap())
        .collect();
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].get_str("action"), Some("state"));
    assert_eq!(values[1].get_str("action"), Some("done"));
    for value in values {
        assert_eq!(value.get("schema_version"), Some(&Value::Num(1)));
    }
}

#[test]
fn held_tracks_complete_record_offsets_and_resets_on_identity_or_prefix_change() {
    let a = format!("{}\n", row("chat", ",\"summary\":\"aaaa\""));
    let b = format!(
        "{}\n",
        versioned(
            &row("chat", ",\"summary\":\"bbbb\""),
            ",\"schema_version\":1"
        )
    );
    let mut held = Held::default();
    let first = held.read_bytes(format!("{a}{{torn").into_bytes(), Some((1, 1)));
    assert_eq!(first.events.len(), 1);
    assert_eq!(first.cursor.offset, a.len() as u64);
    assert!(!first.drained);
    let both = format!("{a}{b}");
    let next = held.read_bytes(both.as_bytes().to_vec(), Some((1, 1)));
    assert_eq!(next.events.len(), 2);
    assert_eq!(next.cursor.offset, both.len() as u64);
    assert!(next.skipped.is_empty());
    assert_eq!(
        held.read_bytes(both.as_bytes().to_vec(), Some((1, 1)))
            .events
            .len(),
        2,
        "no duplicate drain"
    );
    let rewritten = both.replacen("aaaa", "cccc", 1);
    let changed = held.read_bytes(rewritten.into_bytes(), Some((1, 1)));
    assert_eq!(
        changed.events.len(),
        2,
        "same inode and length cannot substitute for prefix proof"
    );
    assert_eq!(changed.events[0].summary.as_deref(), Some("cccc"));
    assert_eq!(
        held.read_bytes(b.as_bytes().to_vec(), Some((1, 1)))
            .events
            .len(),
        1,
        "shrink resets"
    );
    let replacement = held.read_bytes(a.as_bytes().to_vec(), Some((1, 2)));
    assert_eq!(replacement.events.len(), 1);
    assert_eq!(replacement.events[0].summary.as_deref(), Some("aaaa"));
}

#[test]
fn relay_claim_and_opening_fold_identically_without_suppressing_real_duplicates() {
    let attempt = row(
        "relay-attempt",
        &format!(
            r#", "ref":"o8-relay","actor_session_id":"{U}","target_session":"two","target_session_id":"{V}","target_slot":"main","summary":"same text""#
        ),
    );
    let result = row("relay-result", ",\"ref\":\"o8-relay\",\"outcome\":\"sent\"");
    let ask = format!(
        r#"{{"ts":"{TS}","actor":"console:local","action":"ask","ref":"o8-relay","target_slot":"main","target_session_id":"{V}","relay_origin":"{U}"}}"#
    );
    for suffix in metadata() {
        let events: Vec<_> = [&attempt, &result]
            .map(|line| Event::parse_line(&versioned(line, suffix)).unwrap())
            .into();
        let claim = ae::relay::claimed(&events, "o8-relay", U).unwrap();
        assert_eq!(claim.outcome, Some("sent"));
        assert_eq!(claim.text, "same text");
        let target = vec![Event::parse_line(&versioned(&ask, suffix)).unwrap()];
        assert_eq!(
            ae::relay::opening(&target, "o8-relay", U, &claim).unwrap(),
            Some(0)
        );
        let duplicate = vec![events[0].clone(), events[0].clone(), events[1].clone()];
        assert!(
            ae::relay::claimed(&duplicate, "o8-relay", U).is_err(),
            "schema never deduplicates real rows"
        );
    }
}

#[test]
fn snapshot_cursor_byte_positions_are_separate_from_event_timestamps_and_refs() {
    let event = row("chat", ",\"summary\":\"two real messages in one second\"");
    let bytes = format!("{event}\n{}\n", versioned(&event, ",\"schema_version\":1"));
    let drain = EventLog::parse_bytes(bytes.as_bytes(), Cursor::default());
    assert_eq!(
        drain.events.len(),
        2,
        "same ts and missing ref do not merge distinct rows"
    );
    assert_eq!(drain.cursor.offset, bytes.len() as u64);
    let read = ae::session::SessionRead::from_drain_in(&drain, SessionKey::new("one", U));
    assert_eq!(read.events.len(), 2);
}

#[test]
fn verbatim_append_has_exactly_one_product_caller_in_request_repair() {
    fn walk(dir: &std::path::Path, hits: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, hits);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let source = std::fs::read_to_string(&path).unwrap();
                for _ in source.match_indices(".append_event_decided_verbatim(") {
                    hits.push(path.file_name().unwrap().to_str().unwrap().to_owned());
                }
            }
        }
    }
    let mut hits = Vec::new();
    walk(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut hits,
    );
    assert_eq!(
        hits,
        vec!["request_verb.rs"],
        "a verbatim bypass is repair-only"
    );
}
