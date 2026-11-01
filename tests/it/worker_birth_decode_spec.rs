//! Frozen stats287 s1: typed family decoding, independent of envelope version.
//! Oracle: approved Wire + Family version table, tests 7 and 8; no statistics fold.
#![allow(
    clippy::expect_used,
    reason = "frozen wire fixtures must parse or fail loudly"
)]

use ae::birth::{Damage, Family, Record, decode, durable_line};
use ae::events::Event;

const SID: &str = "11111111-1111-4111-8111-111111111111";
const BID: &str = "22222222-2222-4222-8222-222222222222";

fn certificate() -> String {
    format!(
        "{{\"ts\":\"2026-11-01T10:00:00Z\",\"actor\":\"lead\",\"action\":\"worker-birth\",\"target\":\"scout\",\"target_slot\":\"spawned.0\",\"target_session_id\":\"{SID}\",\"birth_v\":1,\"birth\":\"{BID}\",\"birth_tool\":\"claude\",\"birth_client\":\"claude-p\",\"birth_model\":\"sonnet\",\"birth_effort\":\"high\",\"birth_profile\":\"preset\"}}"
    )
}

fn read(line: &str) -> Result<Record, Damage> {
    let event = Event::parse_line(line).expect("family payload damage never rejects the envelope");
    decode(&event).expect("a family record is classified")
}

#[test]
fn typed_certificate_preserves_configured_aliases_and_route_labels() {
    let Record::Birth(birth) = read(&certificate()).expect("v1 certificate") else {
        panic!("supported certificate is Birth");
    };
    assert_eq!(birth.birth, BID);
    assert_eq!(birth.session.as_deref(), Some(SID));
    assert_eq!(birth.name.as_deref(), Some("scout"));
    assert_eq!(birth.slot.as_deref(), Some("spawned.0"));
    assert_eq!(birth.tool.as_deref(), Some("claude"));
    assert_eq!(birth.client.as_deref(), Some("claude-p"));
    assert_eq!(
        birth.model.as_deref(),
        Some("sonnet"),
        "an alias is never expanded"
    );
    assert_eq!(birth.effort.as_deref(), Some("high"));
    assert_eq!(birth.profile.as_deref(), Some("preset"));

    let no_flags = certificate()
        .replace(",\"birth_client\":\"claude-p\"", "")
        .replace(",\"birth_model\":\"sonnet\"", "")
        .replace(",\"birth_effort\":\"high\"", "");
    let Record::Birth(birth) = read(&no_flags).expect("proved absence") else {
        panic!("supported certificate");
    };
    assert!(birth.client.is_none());
    assert!(birth.model.is_none());
    assert!(
        birth.effort.is_none(),
        "no vendor effort default is invented"
    );
}

#[test]
fn legacy_spawn_is_unclassified_but_modern_spawn_has_a_salvageable_link() {
    let legacy = r#"{"ts":"2026-11-01T10:00:00Z","actor":"lead","action":"spawn","target":"scout","summary":"private prompt"}"#;
    assert!(decode(&Event::parse_line(legacy).expect("legacy")).is_none());
    let linked = legacy.replace('}', &format!(",\"birth\":\"{BID}\"}}"));
    assert!(matches!(read(&linked), Ok(Record::SpawnLink { birth }) if birth == BID));
    let damaged = linked.replace(BID, "not-an-id");
    let damage = read(&damaged).expect_err("malformed link is not legacy");
    assert!(matches!(damage.family, Family::Spawn));
    assert!(damage.birth.is_none());
    assert!(
        !durable_line(&linked),
        "prompt-bearing spawn announcements still age out"
    );
}

#[test]
fn each_v1_fact_kind_decodes_without_a_writer_or_a_certificate() {
    for kind in [
        "brief-failed",
        "start-attempted",
        "start-failed",
        "observed",
    ] {
        let start = format!("spawned.0:{BID}:41");
        let extra = match kind {
            "brief-failed" => String::new(),
            "observed" => format!(
                ",\"start\":\"{start}\",\"observed_source\":\"frame\",\"observed_model\":\"claude-sonnet-5-5\""
            ),
            _ => format!(",\"start\":\"{start}\""),
        };
        let line = format!(
            "{{\"ts\":\"2026-11-01T10:00:00Z\",\"actor\":\"watchdog\",\"action\":\"worker-fact\",\"target_session_id\":\"{SID}\",\"birth_v\":1,\"birth\":\"{BID}\",\"fact\":\"{kind}\"{extra}}}"
        );
        let Record::Fact(fact) = read(&line).expect("fact wire") else {
            panic!("supported fact is Fact");
        };
        assert_eq!(fact.birth, BID);
        assert_eq!(fact.kind, kind);
        assert_eq!(fact.session.as_deref(), Some(SID));
        assert_eq!(
            fact.start.as_deref(),
            if kind == "brief-failed" {
                None
            } else {
                Some(start.as_str())
            }
        );
        assert!(fact.observed_effort.is_none());
        assert!(durable_line(&line));
    }
}

#[test]
fn damaged_and_unsupported_fact_evidence_is_classified_and_retained() {
    let fact = format!(
        "{{\"ts\":\"2026-11-01T10:00:00Z\",\"actor\":\"watchdog\",\"action\":\"worker-fact\",\"target_session_id\":\"{SID}\",\"birth_v\":1,\"birth\":\"{BID}\",\"fact\":\"observed\",\"start\":\"spawned.0:{BID}:41\",\"observed_source\":\"frame\",\"observed_model\":\"sonnet\",\"observed_effort\":\"high\"}}"
    );
    let unsupported = fact.replace("\"birth_v\":1", "\"birth_v\":2");
    assert!(
        matches!(read(&unsupported), Ok(Record::Unsupported { family: Family::Fact, version: 2, birth, .. }) if birth == BID)
    );
    assert!(durable_line(&unsupported));
    let unknown_kind = fact.replace("\"fact\":\"observed\"", "\"fact\":\"future-kind\"");
    let damage =
        read(&unknown_kind).expect_err("v1 fact vocabulary is closed: unknown kind is Damage");
    assert_eq!(damage.birth.as_deref(), Some(BID));
    assert!(durable_line(&unknown_kind), "retain the gap evidence");
    let start = format!("spawned.0:{BID}:41");
    for (key, value) in [
        ("fact", "observed"),
        ("start", start.as_str()),
        ("observed_source", "frame"),
        ("observed_model", "sonnet"),
        ("observed_effort", "high"),
    ] {
        let needle = format!("\"{key}\":\"{value}\"");
        for bad in [
            format!("{needle},{needle}"),
            format!("\"{key}\":7"),
            format!("\"{key}\":\"bad\\u007fcell\""),
        ] {
            let changed = fact.replace(&needle, &bad);
            let damage = read(&changed).expect_err("fact damage remains evidence of a gap");
            assert!(matches!(damage.family, Family::Fact));
            assert_eq!(damage.birth.as_deref(), Some(BID));
            assert!(durable_line(&changed));
        }
    }
}

#[test]
fn family_version_table_distinguishes_damage_unsupported_and_nonfamily() {
    let good = certificate();
    for changed in [
        good.replace(",\"birth_v\":1", ""),
        good.replace("\"birth_v\":1", "\"birth_v\":\"1\""),
        good.replace("\"birth_v\":1", "\"birth_v\":1,\"birth_v\":1"),
        good.replace("\"birth_v\":1", "\"birth_v\":0"),
        good.replace("\"birth_v\":1", "\"birth_v\":-1"),
        good.replace("\"birth_v\":1", "\"birth_v\":1.5"),
    ] {
        let damage = read(&changed).expect_err("version Damage");
        assert_eq!(
            damage.birth.as_deref(),
            Some(BID),
            "valid identity is salvageable"
        );
        assert!(durable_line(&changed), "damage is not shredded by resume");
    }
    let unsupported = good.replace("\"birth_v\":1", "\"birth_v\":2");
    assert!(matches!(
        read(&unsupported),
        Ok(Record::Unsupported { family: Family::Birth, version: 2, birth, session })
            if birth == BID && session.as_deref() == Some(SID)
    ));
    assert!(durable_line(&unsupported));
    let bad_id = unsupported.replace(BID, "not-an-id");
    assert!(
        read(&bad_id)
            .expect_err("unsupported with no usable identity")
            .birth
            .is_none()
    );

    for action in ["memo", "spawn", "retire", "worker-birth-extra"] {
        let other = good.replace(
            "\"action\":\"worker-birth\"",
            &format!("\"action\":\"{action}\""),
        );
        assert!(
            !durable_line(&other),
            "only the two exact actions are durable"
        );
    }
    assert!(!durable_line("worker-birth"));
    assert!(!durable_line("{broken worker-fact"));
}

#[test]
fn hostile_family_members_cost_the_family_evidence_not_the_existing_event() {
    let good = certificate();
    for (member, old) in [
        ("birth", BID),
        ("birth_tool", "claude"),
        ("birth_client", "claude-p"),
        ("birth_model", "sonnet"),
        ("birth_effort", "high"),
        ("birth_profile", "preset"),
    ] {
        let needle = format!("\"{member}\":\"{old}\"");
        for bad in [
            format!("{needle},{needle}"),
            format!("\"{member}\":7"),
            format!("\"{member}\":\"bad\\ncell\""),
            format!("\"{member}\":\"{}\"", "x".repeat(1_048_577)),
        ] {
            let changed = good.replace(&needle, &bad);
            let event = Event::parse_line(&changed).expect("old readers keep the event");
            assert_eq!(event.action, "worker-birth");
            assert!(
                decode(&event).expect("family classification").is_err(),
                "{member}"
            );
        }
    }
}

#[test]
fn envelope_versions_and_inert_extensions_never_change_family_decisions() {
    let good = certificate();
    let expected = read(&good).expect("base certificate");
    for extra in [
        ",\"schema_version\":0",
        ",\"schema_version\":1",
        ",\"schema_version\":\"unreadable\"",
        ",\"schema_version\":1,\"schema_version\":2",
        ",\"future_inert\":7,\"future_inert\":\"ignored\"",
    ] {
        let extended = format!("{}{extra}}}", &good[..good.len() - 1]);
        assert_eq!(
            read(&extended).expect("envelope version is not the family version"),
            expected
        );
    }
}

fn observed_fact() -> String {
    format!(
        "{{\"ts\":\"2026-11-01T10:00:00Z\",\"actor\":\"watchdog\",\"action\":\"worker-fact\",\"target_session_id\":\"{SID}\",\"birth_v\":1,\"birth\":\"{BID}\",\"fact\":\"observed\",\"start\":\"spawned.0:{BID}:41\",\"observed_source\":\"frame\",\"observed_model\":\"sonnet\",\"observed_effort\":\"high\"}}"
    )
}

fn with_members(line: &str, extra: &str) -> String {
    format!("{}{extra}}}", &line[..line.len() - 1])
}

fn known_member_damage_on_other_shape(line: &str, family: Family, members: &[(&str, &str)]) {
    for (key, value) in members {
        for extra in [
            format!(",\"{key}\":\"{value}\",\"{key}\":\"{value}\""),
            format!(",\"{key}\":7"),
            format!(",\"{key}\":\"bad\\ncell\""),
            format!(",\"{key}\":\"{}\"", "x".repeat(1_048_577)),
        ] {
            let changed = with_members(line, &extra);
            let event = Event::parse_line(&changed).expect("the old envelope remains usable");
            let damage = decode(&event)
                .expect("known family")
                .expect_err("known members obey Damage rules on either shape");
            assert_eq!(damage.family, family, "{key}");
            assert_eq!(damage.birth.as_deref(), Some(BID), "{key}");
            assert_eq!(damage.session.as_deref(), Some(SID), "{key}");
            assert!(durable_line(&changed), "retain damaged {key} evidence");
        }
    }
}

#[test]
fn fact_fields_on_a_birth_still_obey_known_member_damage_rules() {
    let start = format!("spawned.0:{BID}:41");
    known_member_damage_on_other_shape(
        &certificate(),
        Family::Birth,
        &[
            ("fact", "observed"),
            ("start", &start),
            ("observed_source", "frame"),
            ("observed_model", "sonnet"),
            ("observed_effort", "high"),
        ],
    );
}

#[test]
fn birth_fields_on_a_fact_still_obey_known_member_damage_rules() {
    known_member_damage_on_other_shape(
        &observed_fact(),
        Family::Fact,
        &[
            ("birth_tool", "claude"),
            ("birth_client", "claude-p"),
            ("birth_model", "sonnet"),
            ("birth_effort", "high"),
            ("birth_profile", "preset"),
        ],
    );
}

#[test]
fn unknown_family_extensions_remain_inert_on_both_shapes() {
    for line in [certificate(), observed_fact()] {
        let clean = read(&line).expect("supported family");
        for extra in [
            ",\"future_extension\":7,\"future_extension\":8",
            ",\"future_extension\":\"bad\\ncell\"",
            ",\"future_extension\":{\"unexpected\":true}",
        ] {
            let changed = with_members(&line, extra);
            assert_eq!(read(&changed).expect("unknown is inert"), clean);
            assert!(durable_line(&changed));
        }
    }
}
