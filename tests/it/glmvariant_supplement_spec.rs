//! #310 supplementary assertions, driver-owned and separate from the frozen
//! author payload (SPEC NIT dispositions): further format characters through the
//! same Debug quoting, and another tool with an agent and a `#` model.

use std::path::PathBuf;

use ae::doctor::{Facts, ProfileFacts, ReseatCover, report};

const TAIL: &str = "; effective variant and availability cannot be verified; native saved preferences or a session selection may override it";

fn facts(command: &str) -> Facts {
    Facts {
        version: "2026.9.1".into(),
        core: Some(PathBuf::from("/opt/ae/versions/1/ae-core")),
        core_writable: Some(false),
        core_published: true,
        tmux: Some(PathBuf::from("/usr/bin/tmux")),
        tmux_floor: ae::tmux_floor::Probe::Executable("3.7b".into()),
        boot: Some(1_789_105_855),
        git: Some(PathBuf::from("/usr/bin/git")),
        config: PathBuf::from("/fixture/config"),
        config_error: None,
        local_config: None,
        now: 1_789_200_000,
        quota: vec![],
        main: Some("lead".into()),
        workers: Some(String::new()),
        fleet_order: String::new(),
        fleet_order_error: None,
        profiles: vec![ProfileFacts {
            profile: "go".into(),
            command: command.into(),
            executable: Some("opencode".into()),
            resolves: true,
            resolution_error: None,
        }],
        sessions_dir: PathBuf::from("/fixture/sessions"),
        worktrees_dir: PathBuf::from("/fixture/worktrees"),
        sessions: vec![],
        seats: vec![],
        damaged_records: vec![],
        bindings: vec![],
        reseat_cover: ReseatCover::Off,
    }
}

#[test]
fn further_format_characters_are_escaped_in_the_request() {
    for ch in ['\u{2066}', '\u{200f}'] {
        let model = format!("p/m#{ch}");
        let actual = report(&facts(&format!("opencode --agent build --model '{model}'")));
        let detail = &actual
            .rows()
            .iter()
            .find(|row| row.label == "agent:go")
            .expect("agent row")
            .detail;
        assert_eq!(
            detail,
            &format!("opencode; current profile requests {model:?}{TAIL}"),
            "{ch:?}"
        );
        assert!(!detail.contains(ch), "{ch:?} must not reach the row raw");
    }
}

#[test]
fn another_tool_with_an_agent_and_a_hash_keeps_the_whole_report() {
    let control = report(&facts("codex --agent build --model p/m"));
    for command in [
        "codex --agent build --model p/m#max",
        "claude --agent build --model p/m#max",
    ] {
        assert_eq!(report(&facts(command)), control, "{command:?}");
    }
}
