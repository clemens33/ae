//! Frozen acceptance (dialogseat, pure half): a harness start dialog drawn
//! instead of a ready input box is NAMED as needs-human, with its kind, the one
//! human action and, for Claude's settings variant, the vendor's warning rows.
//! It is never a `TrustSighting`, so `auto_trust` never answers it.
//!
//! Oracle: the frames under `tests/fixtures/dialogseat/` are private-tmux
//! captures of the real harnesses (README there), never adapter constants or
//! matcher output. The `.history.txt` frames are `capture-pane -p -S -` of a
//! pane SHORTER than the modal: the rule and title sit above the visible
//! screen, which is the shape the watchdog's `-S -40` read and a bounded
//! history read see. The expected warning heads are the measured rows.

use ae::watchdog::{DialogKind, HumanPrompt, human_prompt_class};

const PLAIN: &str = include_str!("../fixtures/dialogseat/claude-2.1.330-plain-80x24.txt");
const SETTINGS_80: &str = include_str!("../fixtures/dialogseat/claude-2.1.330-settings-80x24.txt");
const SETTINGS_40_HISTORY: &str =
    include_str!("../fixtures/dialogseat/claude-2.1.330-settings-40x12.history.txt");
const SETTINGS_ONLY: &str =
    include_str!("../fixtures/dialogseat/claude-2.1.330-settings-only-120x40.txt");
const HOOKS_80_HISTORY: &str =
    include_str!("../fixtures/dialogseat/claude-2.1.330-hooks-80x24.history.txt");
const HOOKS_80X50: &str = include_str!("../fixtures/dialogseat/claude-2.1.330-hooks-80x50.txt");
const HOOKS_200: &str = include_str!("../fixtures/dialogseat/claude-2.1.330-hooks-200x50.txt");
const MCP40_HISTORY: &str =
    include_str!("../fixtures/dialogseat/claude-2.1.330-mcp40-80x24.history.txt");
const MCP40_NARROW: &str =
    include_str!("../fixtures/dialogseat/claude-2.1.330-mcp40-34x12.history.txt");
const CODEX_LOGIN: &str = include_str!("../fixtures/dialogseat/codex-0.171.0-login-120x40.txt");
const AGY_LOGIN: &str = include_str!("../fixtures/dialogseat/agy-1.5.14-login-120x40.txt");
const MUSE_TRUST: &str = include_str!("../fixtures/dialogseat/muse-1.4.24-trust-120x40.txt");
const GROK_LOGIN: &str = include_str!("../fixtures/dialogseat/grok-1.0.92-login-120x40.txt");
const OPENCODE_IDLE: &str =
    include_str!("../fixtures/dialogseat/opencode-2.3.2-composer-120x40.txt");
const CLAUDE_IDLE: &str = include_str!("../fixtures/harness-state/claude-idle-167x40.txt");

const CLAUDE_KEYS: &str = "Enter to confirm · Esc to cancel";
const PERMS_1: &str = "⚠ This folder pre-approves 1 tool permission in .claude/settings.json";
const PERMS_6: &str = "⚠ This folder pre-approves 6 tool permissions in .claude/settings.json";
const PERMS_40: &str = "⚠ This folder pre-approves 40 tool permissions in .claude/settings.json";
const HOOKS: &str = "⚠ This folder configures hooks that can run commands without asking";
const LOADS: &str = "Trusting this folder loads .claude/settings.json.";
const LOADS_RUNS: &str = "Trusting this folder loads .claude/settings.json (runs commands).";
const LOADS_MCP: &str = "Trusting this folder loads .claude/settings.json (runs commands) and \
                         .mcp.json here or above (MCP servers).";

/// Every binary a seat records; a frame is read as each of them.
const BINS: [&str; 8] = [
    "claude", "codex", "gemini", "agy", "grok", "muse", "opencode", "unknown",
];

fn named(label: &str, frame: &str, bin: &str) -> HumanPrompt {
    human_prompt_class(frame, bin)
        .unwrap_or_else(|| panic!("{label}: a {bin} start dialog must be named needs-human"))
}

/// The Claude settings-variant frames, each with the warning heads it draws,
/// in drawn order.
fn settings_variants() -> [(&'static str, &'static str, Vec<&'static str>); 8] {
    [
        ("settings-80x24", SETTINGS_80, vec![PERMS_1, LOADS]),
        (
            "settings-40x12-history",
            SETTINGS_40_HISTORY,
            vec![PERMS_1, LOADS],
        ),
        ("settings-only-120x40", SETTINGS_ONLY, vec![LOADS]),
        (
            "hooks-80x24-history",
            HOOKS_80_HISTORY,
            vec![PERMS_6, HOOKS, LOADS_RUNS],
        ),
        ("hooks-80x50", HOOKS_80X50, vec![PERMS_6, HOOKS, LOADS_RUNS]),
        ("hooks-200x50", HOOKS_200, vec![PERMS_6, HOOKS, LOADS_RUNS]),
        (
            "mcp40-80x24-history",
            MCP40_HISTORY,
            vec![PERMS_40, HOOKS, LOADS_MCP],
        ),
        (
            "mcp40-34x12-history",
            MCP40_NARROW,
            vec![PERMS_40, HOOKS, LOADS_MCP],
        ),
    ]
}

#[test]
fn dialogseat_claude_settings_variants_are_needs_human_with_warnings_and_never_a_trust_sighting() {
    for (label, frame, heads) in settings_variants() {
        let prompt = named(label, frame, "claude");
        assert!(
            prompt.question.starts_with("Quick safety check:"),
            "{label}: {prompt:?}"
        );
        assert_eq!(prompt.keys, CLAUDE_KEYS, "{label}");
        assert_eq!(
            prompt.trust, None,
            "{label}: an unmeasured trust variant is never answerable"
        );
        assert!(!prompt.dismiss, "{label}: never dismissable");
        assert_eq!(prompt.kind, DialogKind::FolderTrust, "{label}");
        assert_eq!(
            prompt.warnings.len(),
            heads.len(),
            "{label}: one entry per drawn warning: {:?}",
            prompt.warnings
        );
        for (warning, head) in prompt.warnings.iter().zip(&heads) {
            assert!(
                warning.starts_with(head),
                "{label}: wrapped rows joined with one space: {warning:?} vs {head:?}"
            );
            assert!(
                !warning.chars().any(char::is_control),
                "{label}: one terminal-safe line: {warning:?}"
            );
        }
    }
}

#[test]
fn dialogseat_measured_plain_trust_keeps_its_sighting_and_reports_no_warning() {
    for (label, frame, bin, folder) in [
        ("claude-2.1.330", PLAIN, "claude", "/tmp/claude-trust-probe"),
        (
            "claude-2.1.322",
            include_str!("../fixtures/claude-trust/claude-trust-modal-2.1.322-80x24.txt"),
            "claude",
            "/tmp/claude-trust-probe",
        ),
        (
            "claude-2.1.281",
            include_str!("../fixtures/claude-trust/claude-trust-modal-200x50.txt"),
            "claude",
            "/tmp/claude-trust-probe",
        ),
    ] {
        let prompt = named(label, frame, bin);
        assert_eq!(prompt.kind, DialogKind::FolderTrust, "{label}");
        assert!(prompt.warnings.is_empty(), "{label}: {:?}", prompt.warnings);
        let trust = prompt
            .trust
            .unwrap_or_else(|| panic!("{label}: the measured block stays answerable"));
        assert_eq!(trust.folder, folder, "{label}");
        assert_eq!(trust.toward_yes, Some(ae::tmux::Key::Down), "{label}");
    }
    for (label, frame, bin) in [
        (
            "codex-trust",
            include_str!("../fixtures/codex-trust/codex-trust-modal-80x24.txt"),
            "codex",
        ),
        (
            "agy-trust",
            include_str!("../fixtures/agy-composer/agy-trust-modal-frame.txt"),
            "agy",
        ),
    ] {
        let prompt = named(label, frame, bin);
        assert_eq!(prompt.kind, DialogKind::FolderTrust, "{label}");
        assert!(prompt.warnings.is_empty(), "{label}");
        assert!(
            prompt.trust.is_some(),
            "{label}: measured block stays answerable"
        );
    }
}

#[test]
fn dialogseat_measured_login_and_trust_start_dialogs_of_other_harnesses_are_needs_human() {
    for (label, frame, bin, kind) in [
        (
            "codex-0.171.0-login",
            CODEX_LOGIN,
            "codex",
            DialogKind::Login,
        ),
        ("agy-1.5.14-login", AGY_LOGIN, "agy", DialogKind::Login),
        (
            "muse-1.4.24-trust",
            MUSE_TRUST,
            "muse",
            DialogKind::FolderTrust,
        ),
        ("grok-1.0.92-login", GROK_LOGIN, "grok", DialogKind::Login),
    ] {
        let prompt = named(label, frame, bin);
        assert_eq!(prompt.kind, kind, "{label}");
        assert_eq!(prompt.trust, None, "{label}: no new auto-answer");
        assert!(!prompt.dismiss, "{label}");
        assert!(!prompt.question.trim().is_empty(), "{label}: {prompt:?}");
        assert!(!prompt.keys.trim().is_empty(), "{label}: {prompt:?}");
        assert!(
            !format!("{prompt:?}").contains("ABCD-WXYZ"),
            "{label}: a one-time sign-in code is never reported: {prompt:?}"
        );
    }
}

#[test]
fn dialogseat_every_new_frame_is_read_only_by_its_own_harness() {
    let own: [(&str, &str); 13] = [
        ("claude", SETTINGS_80),
        ("claude", MCP40_NARROW),
        ("claude", SETTINGS_40_HISTORY),
        ("claude", SETTINGS_ONLY),
        ("claude", HOOKS_80_HISTORY),
        ("claude", HOOKS_80X50),
        ("claude", HOOKS_200),
        ("claude", MCP40_HISTORY),
        ("claude", PLAIN),
        ("codex", CODEX_LOGIN),
        ("agy", AGY_LOGIN),
        ("muse", MUSE_TRUST),
        ("grok", GROK_LOGIN),
    ];
    for (index, (owner, frame)) in own.iter().enumerate() {
        for bin in BINS.iter().filter(|bin| *bin != owner) {
            assert_eq!(
                human_prompt_class(frame, bin),
                None,
                "frame {index} of {owner} read as {bin}"
            );
        }
    }
    for bin in BINS {
        assert_eq!(
            human_prompt_class(OPENCODE_IDLE, bin),
            None,
            "a ready opencode composer is no dialog, read as {bin}"
        );
    }
}

#[test]
fn dialogseat_stale_quoted_and_incomplete_settings_frames_are_not_dialogs() {
    let stale = [
        (
            "history above a ready composer",
            format!("{HOOKS_80_HISTORY}\n{CLAUDE_IDLE}"),
        ),
        (
            "wide modal above a ready composer",
            format!("{HOOKS_200}{CLAUDE_IDLE}"),
        ),
        (
            "short history above a ready composer",
            format!("{SETTINGS_40_HISTORY}{CLAUDE_IDLE}"),
        ),
    ];
    for (label, frame) in &stale {
        assert_eq!(
            human_prompt_class(frame, "claude"),
            None,
            "{label}: retained history is not a live dialog"
        );
    }
    let incomplete = [
        ("no key hint", HOOKS_80X50.replace(CLAUDE_KEYS, "")),
        (
            "no selected option",
            HOOKS_80X50.replace("❯ No, exit", "  No, exit"),
        ),
        (
            "no title under the rule",
            HOOKS_80X50.replace(" Accessing workspace:", " Tool output:"),
        ),
    ];
    for (label, frame) in &incomplete {
        assert!(frame != HOOKS_80X50, "setup: {label} changed the frame");
        assert_eq!(
            human_prompt_class(frame, "claude"),
            None,
            "{label}: only a complete modal is a dialog"
        );
    }
}

#[test]
fn dialogseat_hostile_settings_text_is_reported_terminal_safe() {
    // A permission string is repo-controlled text the vendor echoes.
    let hostile = HOOKS_80X50.replace(
        "Bash(ls:*)",
        "Bash(\u{1b}]52;c;cGF5bG9hZA==\u{7}x\u{1b}[2J\u{202e}:*)",
    );
    assert!(hostile != HOOKS_80X50, "setup: the permission row changed");
    let prompt = named("hostile", &hostile, "claude");
    assert_eq!(prompt.trust, None);
    assert!(!prompt.warnings.is_empty(), "the warning is still reported");
    for text in prompt
        .warnings
        .iter()
        .chain([&prompt.question, &prompt.keys])
    {
        assert!(
            !text.chars().any(|ch| ch.is_control() || ch == '\u{202e}'),
            "terminal-safe: {text:?}"
        );
    }
}

#[test]
fn dialogseat_kind_names_and_the_one_human_action() {
    let trust = DialogKind::FolderTrust;
    let login = DialogKind::Login;
    assert!(
        trust.label().to_lowercase().contains("trust"),
        "{}",
        trust.label()
    );
    assert!(
        login.label().to_lowercase().contains("login"),
        "{}",
        login.label()
    );
    assert_ne!(trust.label(), login.label());
    for kind in [trust, login] {
        let action = kind.action();
        assert!(!action.trim().is_empty(), "{kind:?}");
        assert!(!action.contains('\n'), "{kind:?}: one line");
        assert!(
            action.to_lowercase().contains("attach"),
            "{kind:?}: the human attaches to the seat to answer: {action}"
        );
    }
}
