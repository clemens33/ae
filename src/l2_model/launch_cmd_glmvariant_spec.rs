//! Frozen #310 seam tests. Not wired or compiled on the base revision.
//! Expected values come from D9's opaque-lexed-value contract, not the helper.

use super::{opencode_launch, requested_model};

#[test]
fn accepted_models_return_the_entire_lexed_value() {
    for (command, expected) in [
        ("opencode --agent build --model p/m#max", "p/m#max"),
        ("opencode --agent build --model 'p/m#max'", "p/m#max"),
        ("opencode --agent build --model \"p/m#max\"", "p/m#max"),
        ("opencode --agent build -m p/m#max", "p/m#max"),
        ("opencode --agent=build --model=p/m#max", "p/m#max"),
        ("opencode --agent build --model p/m#", "p/m#"),
        ("opencode --agent build --model p/m#a#b", "p/m#a#b"),
        ("opencode --agent build --model p/m\\#max", "p/m#max"),
        ("opencode --agent build --model p/'m#'max", "p/m#max"),
        ("opencode --agent build --model 'a b#c\"d'", "a b#c\"d"),
        ("opencode --agent build --model 'p/m#\\tail'", "p/m#\\tail"),
        (
            "opencode --agent build --model 'p/m#\u{202e}'",
            "p/m#\u{202e}",
        ),
        ("opencode --agent build --model pré/m#é", "pré/m#é"),
        (
            "/tmp/opencode --standalone --agent build --model p/m#max",
            "p/m#max",
        ),
        (
            "env FOO=bar opencode --agent build --model p/m#max",
            "p/m#max",
        ),
    ] {
        assert!(
            opencode_launch(command).is_ok(),
            "accepted translator control: {command:?}"
        );
        assert_eq!(
            requested_model(command).as_deref(),
            Some(expected),
            "{command:?}"
        );
    }
}

#[test]
fn absence_or_another_tool_never_becomes_a_request() {
    for command in [
        "opencode",
        "opencode --agent build",
        "opencode --agent build --model p/m",
        "opencode --agent build --model 'p/m with spaces'",
        "codex --model p/m#max",
        "claude --model p/m#max",
        "echo opencode --agent build --model p/m#max",
        "notopencode --agent build --model p/m#max",
        "",
        "#max",
    ] {
        assert_eq!(requested_model(command), None, "{command:?}");
    }
}

#[test]
fn every_translation_refusal_suppresses_the_inventory_request() {
    // Existing oc2b refusal shapes plus hash-bearing off-diagonal cases.
    // The translator is a parity oracle, never the source of expected values.
    for command in [
        "opencode --agent build --model",
        "opencode --agent build -m",
        "opencode --agent",
        "opencode --agent build --model=",
        "opencode --agent=",
        "opencode --agent build --model ''",
        "opencode --agent ''",
        "opencode --agent build --model p/a --model p/b",
        "opencode --agent build -m p/a --model p/b",
        "opencode --agent build --agent plan",
        "opencode --agent=build --agent plan",
        "opencode --agent build -mp/a",
        "opencode --agent build -m=p/a",
        "opencode --model --agent build",
        "opencode --agent build mini --model p/a",
        "opencode --agent build --variant max",
        "opencode --agent build --variant=max",
        "opencode --agent build --standalone --standalone",
        "opencode --agent build --standalone=true",
        "opencode --agent build --server http://127.0.0.1:9",
        "opencode --agent build --server=http://127.0.0.1:9",
        "opencode --agent build -- --model p/a",
        "opencode --agent build --model p/a ; true",
        "opencode --agent build --model $MODEL",
        "opencode --agent build --model `echo x`",
        "OPENCODE_CONFIG=/tmp/other opencode --agent build --model p/a",
        "OPENCODE_CONFIG_CONTENT='{}' opencode --agent build --model p/a",
        "env -u OPENCODE_CONFIG opencode --agent build --model p/a",
        "env -u OPENCODE_CONFIG_CONTENT opencode --agent build --model p/a",
        "opencode --model p/a",
        "opencode --model=p/a",
        "opencode -m p/a",
        "opencode --agent build#max --model",
        "opencode --agent build#max -m",
        "opencode --model p/m#max --agent",
        "opencode --model p/m#max --agent=",
        "opencode --agent build#max --model=",
        "opencode --agent build#max --model ''",
        "opencode --agent '' --model p/m#max",
        "opencode --agent build --model p/m#max --model p/b",
        "opencode --agent build -m p/m#max --model p/b",
        "opencode --agent build --agent plan --model p/m#max",
        "opencode --agent=build --agent plan --model p/m#max",
        "opencode --agent build -mp/m#max",
        "opencode --agent build -m=p/m#max",
        "opencode --model --agent build#max",
        "opencode --agent build mini --model p/m#max",
        "opencode --agent build --model p/m#max --variant max",
        "opencode --agent build --model p/m#max --variant=max",
        "opencode --agent build --model p/m#max --standalone --standalone",
        "opencode --agent build --model p/m#max --standalone=true",
        "opencode --agent build --model p/m#max --server x",
        "opencode --agent build --model p/m#max --server=x",
        "opencode --agent build -- --model p/m#max",
        "opencode --agent build --model p/m#max ; true",
        "opencode --agent build --model '$MODEL#max'",
        "opencode --agent build --model `echo p/m#max`",
        "OPENCODE_CONFIG=x opencode --agent build --model p/m#max",
        "OPENCODE_CONFIG_CONTENT='{}' opencode --agent build --model p/m#max",
        "env -u OPENCODE_CONFIG opencode --agent build --model p/m#max",
        "env -u OPENCODE_CONFIG_CONTENT opencode --agent build --model p/m#max",
        "opencode --model p/m#max",
        "opencode --model=p/m#max",
        "opencode -m p/m#max",
        "opencode --agent '$AGENT' --model p/m#max",
        "opencode --agent 'a\nb' --model p/m#max",
        "opencode --agent 'a\tb' --model p/m#max",
        "opencode --agent 'a\rb' --model p/m#max",
        "opencode --agent build --model 'p/m#max\nnext'",
        "opencode --agent build --model 'p/m#max\tnext'",
        "opencode --agent build --model 'p/m#max\rnext'",
        "opencode --agent build --model 'p/m#max",
    ] {
        assert!(
            opencode_launch(command).is_err(),
            "refusal oracle: {command:?}"
        );
        assert_eq!(requested_model(command), None, "{command:?}");
    }
}
