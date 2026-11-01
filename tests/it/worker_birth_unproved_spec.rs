//! Frozen stats287 I2: value, proved absence and unproved flag state stay distinct.
//! Oracle: lead ruling ae-20261101T124339Z-240e55c8 and plan addendum.
#![allow(clippy::expect_used, reason = "independent fixed wire/config fixtures")]

use ae::birth::{Family, Record, UNPROVED_WORDS, configured, decode, durable_line};
use ae::config::ResolvedCommand;
use ae::events::Event;
use ae::tool::ToolKind;

const SID: &str = "11111111-1111-4111-8111-111111111111";
const BID: &str = "22222222-2222-4222-8222-222222222222";
const WORDS: [&str; 8] = [
    "unsupported",
    "not-simple",
    "ambiguous-model",
    "model-missing-value",
    "ambiguous-effort",
    "effort-missing-value",
    "unknown-effort",
    "env-effort",
];

fn resolved(command: &str) -> ResolvedCommand {
    let config = ae::config::parse_identity(&format!("[profiles]\np = \"{command}\"\n"))
        .expect("fixture profile");
    config
        .command("p", None)
        .expect("resolution")
        .expect("profile exists")
}

fn base(action: &str) -> String {
    let fact = if action == "worker-fact" {
        format!(",\"fact\":\"observed\",\"start\":\"spawned.0:{BID}:41\"")
    } else {
        String::new()
    };
    format!(
        "{{\"ts\":\"2026-11-01T10:00:00Z\",\"actor\":\"lead\",\"action\":\"{action}\",\"target\":\"scout\",\"target_slot\":\"spawned.0\",\"target_session_id\":\"{SID}\",\"birth_v\":1,\"birth\":\"{BID}\"{fact}}}"
    )
}

fn with_members(line: &str, extra: &str) -> String {
    format!("{}{extra}}}", &line[..line.len() - 1])
}

#[test]
fn configured_flag_states_are_independent_and_never_fabricate_absence() {
    for (command, tool, model, model_unproved, effort, effort_unproved) in [
        ("claude", ToolKind::Claude, None, None, None, None),
        (
            "claude --model sonnet --effort high",
            ToolKind::Claude,
            Some("sonnet"),
            None,
            Some("high"),
            None,
        ),
        (
            "codex --model gpt-6.1-sol -c model_reasoning_effort=xhigh",
            ToolKind::Codex,
            Some("gpt-6.1-sol"),
            None,
            Some("xhigh"),
            None,
        ),
        (
            "claude --model sonnet --model opus --effort high --effort max",
            ToolKind::Claude,
            None,
            Some("ambiguous-model"),
            None,
            Some("ambiguous-effort"),
        ),
        (
            "claude --model --effort high",
            ToolKind::Claude,
            None,
            Some("model-missing-value"),
            Some("high"),
            None,
        ),
        (
            "claude --model sonnet --effort high --effort max",
            ToolKind::Claude,
            Some("sonnet"),
            None,
            None,
            Some("ambiguous-effort"),
        ),
        (
            "claude --model sonnet --effort",
            ToolKind::Claude,
            Some("sonnet"),
            None,
            None,
            Some("effort-missing-value"),
        ),
        (
            "mystery --model arbitrary",
            ToolKind::Unknown,
            None,
            Some("unsupported"),
            None,
            Some("unsupported"),
        ),
        (
            "claude; echo PRIVATE_COMMAND_CANARY",
            ToolKind::Claude,
            None,
            Some("not-simple"),
            None,
            Some("not-simple"),
        ),
        (
            "claude --effort PRIVATE_VALUE_CANARY",
            ToolKind::Claude,
            None,
            None,
            None,
            Some("unknown-effort"),
        ),
        (
            "CLAUDE_CODE_EFFORT_LEVEL=medium claude --model sonnet --effort high",
            ToolKind::Claude,
            Some("sonnet"),
            None,
            None,
            Some("env-effort"),
        ),
        (
            "MODEL=PRIVATE_ENV_CANARY claude",
            ToolKind::Claude,
            None,
            None,
            None,
            None,
        ),
    ] {
        let actual = configured(&resolved(command), tool, "p");
        assert_eq!(actual.tool, tool.as_str(), "{command}");
        assert_eq!(actual.profile, "p");
        assert!(actual.client.is_none());
        assert_eq!(actual.model.as_deref(), model, "{command}");
        assert_eq!(actual.model_unproved, model_unproved, "{command}");
        assert_eq!(actual.effort.as_deref(), effort, "{command}");
        assert_eq!(actual.effort_unproved, effort_unproved, "{command}");
        assert!(!format!("{actual:?}").contains("PRIVATE_"));
    }
}

#[test]
fn closed_unproved_words_reach_their_dimension_consumers() {
    let mut actual = UNPROVED_WORDS.to_vec();
    actual.sort_unstable();
    let mut expected = WORDS;
    expected.sort_unstable();
    assert_eq!(actual, expected, "closed v1 vocabulary, no duplicate words");
    for (member, word) in [
        ("birth_model_unproved", "unsupported"),
        ("birth_model_unproved", "not-simple"),
        ("birth_model_unproved", "ambiguous-model"),
        ("birth_model_unproved", "model-missing-value"),
        ("birth_effort_unproved", "unsupported"),
        ("birth_effort_unproved", "not-simple"),
        ("birth_effort_unproved", "ambiguous-effort"),
        ("birth_effort_unproved", "effort-missing-value"),
        ("birth_effort_unproved", "unknown-effort"),
        ("birth_effort_unproved", "env-effort"),
    ] {
        let line = with_members(&base("worker-birth"), &format!(",\"{member}\":\"{word}\""));
        let event = Event::parse_line(&line).expect("old envelope");
        let Record::Birth(birth) = decode(&event).expect("family").expect("supported word") else {
            panic!("Birth");
        };
        assert_eq!(birth.birth, BID);
        assert_eq!(birth.session.as_deref(), Some(SID));
        assert!(birth.model.is_none());
        assert!(birth.effort.is_none());
        assert_eq!(
            birth.model_unproved.as_deref(),
            (member == "birth_model_unproved").then_some(word)
        );
        assert_eq!(
            birth.effort_unproved.as_deref(),
            (member == "birth_effort_unproved").then_some(word)
        );
        assert!(durable_line(&line));
    }
    let event = Event::parse_line(&base("worker-birth")).expect("absent flags");
    let Record::Birth(birth) = decode(&event).expect("family").expect("absence is proved") else {
        panic!("Birth");
    };
    assert!(birth.model.is_none() && birth.model_unproved.is_none());
    assert!(birth.effort.is_none() && birth.effort_unproved.is_none());
    for (extra, model, model_unproved, effort, effort_unproved) in [
        (
            ",\"birth_model\":\"sonnet\",\"birth_effort\":\"high\"",
            Some("sonnet"),
            None,
            Some("high"),
            None,
        ),
        (
            ",\"birth_model_unproved\":\"ambiguous-model\",\"birth_effort\":\"high\"",
            None,
            Some("ambiguous-model"),
            Some("high"),
            None,
        ),
        (
            ",\"birth_model\":\"sonnet\",\"birth_effort_unproved\":\"ambiguous-effort\"",
            Some("sonnet"),
            None,
            None,
            Some("ambiguous-effort"),
        ),
        (
            ",\"birth_model_unproved\":\"ambiguous-model\",\"birth_effort_unproved\":\"effort-missing-value\"",
            None,
            Some("ambiguous-model"),
            None,
            Some("effort-missing-value"),
        ),
    ] {
        let line = with_members(&base("worker-birth"), extra);
        let event = Event::parse_line(&line).expect("valid mixed flag states");
        let Record::Birth(birth) = decode(&event).expect("family").expect("supported states")
        else {
            panic!("Birth");
        };
        assert_eq!(birth.model.as_deref(), model);
        assert_eq!(birth.model_unproved.as_deref(), model_unproved);
        assert_eq!(birth.effort.as_deref(), effort);
        assert_eq!(birth.effort_unproved.as_deref(), effort_unproved);
    }
}

#[test]
fn new_unproved_members_obey_damage_and_conflict_rules_on_both_shapes() {
    for (action, family) in [
        ("worker-birth", Family::Birth),
        ("worker-fact", Family::Fact),
    ] {
        let clean = base(action);
        for (member, value_member) in [
            ("birth_model_unproved", "birth_model"),
            ("birth_effort_unproved", "birth_effort"),
        ] {
            let value = if value_member == "birth_model" {
                "sonnet"
            } else {
                "high"
            };
            // A valid unused member on a fact is inert, never an unknown key.
            let good = with_members(&clean, &format!(",\"{member}\":\"unsupported\""));
            assert!(
                decode(&Event::parse_line(&good).expect("event"))
                    .expect("family")
                    .is_ok()
            );
            for extra in [
                format!(",\"{member}\":\"unsupported\",\"{member}\":\"unsupported\""),
                format!(",\"{member}\":7"),
                format!(",\"{member}\":\"bad\\ncell\""),
                format!(",\"{member}\":\"{}\"", "x".repeat(1_048_577)),
                format!(",\"{member}\":\"future-word\""),
                format!(",\"{member}\":\"no-effort-flag\""),
                format!(",\"{member}\":\"unsupported\",\"{value_member}\":\"{value}\""),
            ] {
                let line = with_members(&clean, &extra);
                let event = Event::parse_line(&line).expect("damage keeps the envelope");
                let damage = decode(&event)
                    .expect("family")
                    .expect_err("bad or contradictory known state is Damage");
                assert_eq!(damage.family, family, "{action} {member}");
                assert_eq!(damage.birth.as_deref(), Some(BID));
                assert_eq!(damage.session.as_deref(), Some(SID));
                assert!(durable_line(&line));
            }
        }
    }
}
