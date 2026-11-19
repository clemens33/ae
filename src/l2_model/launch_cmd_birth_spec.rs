//! Frozen stats287 I2: every parser failure has a safe, decodable wire word.
#![allow(clippy::expect_used, reason = "independent fixed flag-state fixtures")]

use super::{EffortRefusal, ModelFlagError, model_flag_read, model_flag_value};
use crate::tool::ToolKind;

fn expected_model(error: ModelFlagError) -> Option<&'static str> {
    match error {
        ModelFlagError::Unsupported => Some("unsupported"),
        ModelFlagError::Absent => None,
        ModelFlagError::Ambiguous => Some("ambiguous-model"),
        ModelFlagError::MissingValue => Some("model-missing-value"),
    }
}

fn expected_effort(error: &EffortRefusal) -> Option<&'static str> {
    match error {
        EffortRefusal::Unsupported => Some("unsupported"),
        EffortRefusal::NotSimple(_) => Some("not-simple"),
        EffortRefusal::Absent => None,
        EffortRefusal::Ambiguous => Some("ambiguous-effort"),
        EffortRefusal::MissingValue => Some("effort-missing-value"),
        EffortRefusal::UnknownLevel(_) => Some("unknown-effort"),
        EffortRefusal::Environment => Some("env-effort"),
    }
}

#[test]
fn every_nonabsent_parser_error_word_belongs_to_the_closed_wire_vocabulary() {
    // Exhaustive matches above make a new enum arm require a spec decision.
    for error in [
        ModelFlagError::Unsupported,
        ModelFlagError::Absent,
        ModelFlagError::Ambiguous,
        ModelFlagError::MissingValue,
    ] {
        if let Some(expected) = expected_model(error) {
            assert_eq!(error.word(), expected);
            assert!(crate::birth::UNPROVED_WORDS.contains(&error.word()));
        }
    }
    for error in [
        EffortRefusal::Unsupported,
        EffortRefusal::NotSimple("PRIVATE_COMMAND_CANARY".to_owned()),
        EffortRefusal::Absent,
        EffortRefusal::Ambiguous,
        EffortRefusal::MissingValue,
        EffortRefusal::UnknownLevel("PRIVATE_VALUE_CANARY".to_owned()),
        EffortRefusal::Environment,
    ] {
        if let Some(expected) = expected_effort(&error) {
            assert_eq!(error.word(), expected);
            assert!(crate::birth::UNPROVED_WORDS.contains(&error.word()));
        }
    }
}

#[test]
fn model_flag_read_distinguishes_absence_and_error_without_changing_the_old_reader() {
    for (command, tool, expected, old) in [
        ("claude", ToolKind::Claude, Ok(None), None),
        (
            "claude --model sonnet",
            ToolKind::Claude,
            Ok(Some("sonnet".to_owned())),
            Some("sonnet"),
        ),
        (
            "codex --model=gpt-6.1-sol",
            ToolKind::Codex,
            Ok(Some("gpt-6.1-sol".to_owned())),
            Some("gpt-6.1-sol"),
        ),
        (
            "claude --model sonnet --model opus",
            ToolKind::Claude,
            Err("ambiguous-model"),
            None,
        ),
        (
            "claude --model --effort high",
            ToolKind::Claude,
            Err("model-missing-value"),
            None,
        ),
        (
            "mystery --model arbitrary",
            ToolKind::Unknown,
            Err("unsupported"),
            None,
        ),
        (
            "claude; echo PRIVATE_COMMAND_CANARY",
            ToolKind::Claude,
            Err("not-simple"),
            None,
        ),
        (
            "MODEL=PRIVATE_ENV_CANARY claude",
            ToolKind::Claude,
            Ok(None),
            None,
        ),
    ] {
        assert_eq!(model_flag_read(command, tool), expected, "{command}");
        assert_eq!(model_flag_value(command, tool).as_deref(), old, "{command}");
    }
}

#[test]
fn certificate_projection_serializes_unlaunchable_states_without_command_payloads() {
    for (command, tool, reason) in [
        (
            "mystery --model PRIVATE_VALUE_CANARY",
            ToolKind::Unknown,
            "unsupported",
        ),
        (
            "claude; echo PRIVATE_COMMAND_CANARY",
            ToolKind::Claude,
            "not-simple",
        ),
    ] {
        let config = crate::config::parse_identity(&format!("[profiles]\np = \"{command}\"\n"))
            .expect("fixture profile");
        let resolved = config
            .command("p", None)
            .expect("resolution")
            .expect("profile");
        let selection = crate::birth::configured(&resolved, tool, "p");
        let line = crate::birth::certificate_line(
            crate::time::Timestamp::parse("2026-11-01T10:00:00Z").expect("timestamp"),
            "lead",
            &crate::birth::Seat {
                name: "scout",
                slot: "spawned.0",
                session: "11111111-1111-4111-8111-111111111111",
                birth: "22222222-2222-4222-8222-222222222222",
            },
            &selection,
        );
        let raw = crate::json::parse(line.trim_end()).expect("certificate JSON");
        for key in ["birth_model_unproved", "birth_effort_unproved"] {
            assert_eq!(raw.get(key), Some(&crate::json::Value::str(reason)));
        }
        assert!(raw.get("birth_model").is_none());
        assert!(raw.get("birth_effort").is_none());
        for forbidden in ["PRIVATE_", "summary", "command", "argv", "body_file"] {
            assert!(!line.contains(forbidden), "{forbidden}");
        }
        let event = crate::events::Event::parse_line(&line).expect("old envelope");
        assert!(matches!(
            crate::birth::decode(&event),
            Some(Ok(crate::birth::Record::Birth(birth)))
                if birth.model_unproved.as_deref() == Some(reason)
                    && birth.effort_unproved.as_deref() == Some(reason)
        ));
    }
}
