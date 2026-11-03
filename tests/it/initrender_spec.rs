//! Byte goldens captured from unchanged 86674177: behaviour pins, not a new template.
//! Requirements: docs/reference/commands.md `ae init` and contract.md Config.
//! The none-found direct call is unreachable through `ae init` (no default choices).
//! Version is the only substituted field, allowing an unrelated version bump.

use super::*;

struct Case {
    name: &'static str,
    tools: &'static [&'static str],
    flags: &'static [&'static str],
    golden: &'static str,
}

const CASES: &[Case] = &[
    Case {
        name: "none",
        tools: &[],
        flags: &[],
        golden: include_str!("initrender/none.config"),
    },
    Case {
        name: "claude",
        tools: &["claude"],
        flags: &[],
        golden: include_str!("initrender/claude.config"),
    },
    Case {
        name: "codex",
        tools: &["codex"],
        flags: &[],
        golden: include_str!("initrender/codex.config"),
    },
    Case {
        name: "grok",
        tools: &["grok"],
        flags: &[],
        golden: include_str!("initrender/grok.config"),
    },
    Case {
        name: "agy",
        tools: &["agy"],
        flags: &[],
        golden: include_str!("initrender/agy.config"),
    },
    Case {
        name: "opencode",
        tools: &["opencode"],
        flags: &[],
        golden: include_str!("initrender/opencode.config"),
    },
    Case {
        name: "gemini",
        tools: &["gemini"],
        flags: &[],
        golden: include_str!("initrender/gemini.config"),
    },
    Case {
        name: "standing-pair",
        tools: &["claude", "codex"],
        flags: &[],
        golden: include_str!("initrender/standing-pair.config"),
    },
    Case {
        name: "all",
        tools: &["claude", "codex", "grok", "agy", "opencode", "gemini"],
        flags: &[],
        golden: include_str!("initrender/all.config"),
    },
    Case {
        name: "fallback-pair",
        tools: &["grok", "agy"],
        flags: &[],
        golden: include_str!("initrender/fallback-pair.config"),
    },
    Case {
        name: "unverified-pair",
        tools: &["agy", "opencode"],
        flags: &[],
        golden: include_str!("initrender/unverified-pair.config"),
    },
    Case {
        name: "unverified-both",
        tools: &["opencode", "gemini"],
        flags: &[],
        golden: include_str!("initrender/unverified-both.config"),
    },
    Case {
        name: "claude-reviewers",
        tools: &["claude", "grok", "agy", "opencode", "gemini"],
        flags: &[],
        golden: include_str!("initrender/claude-reviewers.config"),
    },
    Case {
        name: "codex-reviewers",
        tools: &["codex", "grok", "agy", "opencode", "gemini"],
        flags: &[],
        golden: include_str!("initrender/codex-reviewers.config"),
    },
    Case {
        name: "palette-a",
        tools: &["claude", "codex"],
        flags: &["--palette", "a"],
        golden: include_str!("initrender/palette-a.config"),
    },
    Case {
        name: "palette-b",
        tools: &["claude", "codex"],
        flags: &["--palette", "b"],
        golden: include_str!("initrender/palette-b.config"),
    },
    Case {
        name: "solo",
        tools: &["claude", "codex"],
        flags: &["--solo"],
        golden: include_str!("initrender/solo.config"),
    },
    Case {
        name: "no-orchestrator",
        tools: &["claude", "codex"],
        flags: &["--no-orchestrator"],
        golden: include_str!("initrender/no-orchestrator.config"),
    },
    Case {
        name: "solo-no-orchestrator",
        tools: &["claude", "codex"],
        flags: &["--solo", "--no-orchestrator"],
        golden: include_str!("initrender/solo-no-orchestrator.config"),
    },
    Case {
        name: "catalog-seats",
        tools: &["claude", "codex", "grok", "agy"],
        flags: &[
            "--lead",
            "grok46",
            "--colead",
            "agy",
            "--orchestrator",
            "fablex",
        ],
        golden: include_str!("initrender/catalog-seats.config"),
    },
    Case {
        name: "mixed-picks",
        tools: &["claude", "codex", "grok"],
        flags: &[
            "--lead-effort",
            "low",
            "--colead-model",
            "gpt-6-luna",
            "--colead-effort",
            "max",
            "--orchestrator",
            "grok46",
        ],
        golden: include_str!("initrender/mixed-picks.config"),
    },
    Case {
        name: "duplicate-picks",
        tools: &["claude"],
        flags: &["--lead-effort", "xhigh"],
        golden: include_str!("initrender/duplicate-picks.config"),
    },
];

fn discovered(tools: &[&str]) -> Vec<Discovery> {
    discover_with(|tool| {
        tools
            .contains(&tool)
            .then(|| PathBuf::from(format!("/bin/{tool}")))
    })
}

fn choices_for(case: &Case, found: &[Discovery]) -> Choices {
    if case.tools.is_empty() {
        assert!(
            default_choices(found).is_none(),
            "none is unreachable from ae init"
        );
        return Choices {
            lead: Seat::Profile("fable5".to_owned()),
            colead: None,
            orchestrator: None,
            palette: "darcula".to_owned(),
        };
    }
    let words: Vec<String> = case.flags.iter().map(|word| (*word).to_owned()).collect();
    let args = parse(&words).expect("documented choice flags");
    apply_args(default_choices(found).expect("found harness"), &args, found)
        .expect("available choices")
}

#[test]
fn render_matches_frozen_base_bytes() {
    for case in CASES {
        let found = discovered(case.tools);
        let choices = choices_for(case, &found);
        let actual = render(
            &found,
            &choices,
            crate::time::Timestamp::from_epoch(1_788_825_600),
        );
        let expected = case.golden.replace("@AE_VERSION@", crate::VERSION);
        assert_eq!(actual, expected, "byte golden: {}", case.name);
    }
}

#[test]
fn render_parses_and_resolves_frozen_roster_commands() {
    for case in CASES {
        let found = discovered(case.tools);
        let choices = choices_for(case, &found);
        let actual = render(
            &found,
            &choices,
            crate::time::Timestamp::from_epoch(1_788_825_600),
        );
        let expected = case.golden.replace("@AE_VERSION@", crate::VERSION);
        let got = crate::config::parse_identity(&actual).expect("rendered INI");
        let pin = crate::config::parse_identity(&expected).expect("frozen base INI");
        assert_eq!(got.roster, pin.roster, "roster: {}", case.name);
        assert_eq!(got.profiles, pin.profiles, "profiles: {}", case.name);
        assert_eq!(got.clients, pin.clients, "clients: {}", case.name);
        assert_eq!(got.main, pin.main, "main: {}", case.name);
        assert_eq!(got.workers, pin.workers, "workers: {}", case.name);
        for (_, profile) in &pin.roster {
            let want = pin.command(profile, None).expect("frozen command");
            let resolved = got.command(profile, None).expect("rendered command");
            assert_eq!(resolved, want, "resolved profile {profile}: {}", case.name);
            if !case.tools.is_empty() {
                assert!(
                    resolved.is_some(),
                    "roster must resolve: {} {profile}",
                    case.name
                );
            }
        }
    }
}

#[test]
fn render_date_uses_the_supplied_timestamp() {
    let case = CASES
        .iter()
        .find(|case| case.name == "standing-pair")
        .expect("pair case");
    let found = discovered(case.tools);
    let choices = choices_for(case, &found);
    let pinned = case.golden.replace("@AE_VERSION@", crate::VERSION);
    for (epoch, date) in [
        (0, "1970-01-01"),
        (86_399, "1970-01-01"),
        (86_400, "1970-01-02"),
    ] {
        assert_eq!(
            render(&found, &choices, crate::time::Timestamp::from_epoch(epoch)),
            pinned.replacen("2026-09-08", date, 1),
            "date {epoch}"
        );
    }
}
