//! Driver-owned unit tests of the muse routing in `limit_proven` and
//! `frame_gap`, beside the frozen acceptance spec in `limitmuse_spec.rs`: the
//! refusal text of each standing, and that claude's own text and proof are
//! untouched by the muse arm.
use super::{HarnessState, ToolKind, frame_gap, limit_proven, refusal_why};

const PLAIN: &str = include_str!("../../tests/fixtures/harness-state/muse-limit-plain.txt");
const WATCHING: &str = include_str!("../../tests/fixtures/harness-state/muse-limit-watching.txt");
const CLAUDE_LIMITED: &str =
    include_str!("../../tests/fixtures/harness-state/claude-limit-echo-2.1.317.txt");

fn muse_gap(capture: Option<&str>) -> Option<String> {
    frame_gap(
        (HarnessState::Unknown, false, capture),
        "muse",
        ToolKind::Muse,
    )
}

#[test]
fn a_muse_refusal_names_its_standing() {
    assert_eq!(muse_gap(None).as_deref(), Some("no frame"));
    // The muse arm reads any capture as a frame to judge; nothing is "no frame".
    assert_eq!(muse_gap(Some("")).as_deref(), Some("no limit row"));
    assert_eq!(
        muse_gap(Some("not a frame")).as_deref(),
        Some("no limit row")
    );
    let headless = PLAIN.replace("  This turn stopped", "  This turn did not stop");
    assert_eq!(muse_gap(Some(&headless)).as_deref(), Some("no limit row"));
    let draft = PLAIN.replace("\n❯\n", "\n❯ draft\n");
    assert_eq!(
        muse_gap(Some(&draft)).as_deref(),
        Some("no empty box under the limit row")
    );
    let later = PLAIN.replacen("── Voice", "◆ Ran \"a\" command\n── Voice", 1);
    assert_eq!(
        muse_gap(Some(&later)).as_deref(),
        Some("row after the limit row: \"◆ Ran a command\"")
    );
    assert_eq!(
        muse_gap(Some(WATCHING)).as_deref(),
        Some("background task: \"└ ◇ background evidence watch  watching\"")
    );
    let quoted = WATCHING.replace("background evidence watch", "a \"b\" watch");
    assert_eq!(
        muse_gap(Some(&quoted)).as_deref(),
        Some("background task: \"└ ◇ a b watch  watching\"")
    );
}

#[test]
fn a_proven_muse_reading_has_no_gap() {
    // The scan stands, so a reading that was not marked proven has nothing to name.
    assert_eq!(muse_gap(Some(PLAIN)), None);
    // A reading that proves on its own is no gap, whatever the capture.
    for (state, limited) in [(HarnessState::Idle, false), (HarnessState::Unknown, true)] {
        assert_eq!(
            frame_gap((state, limited, Some(WATCHING)), "muse", ToolKind::Muse),
            None
        );
    }
}

#[test]
fn the_first_muse_reading_that_proves_nothing_leads_the_refusal() {
    let who = |first: Option<&str>, second: Option<&str>| {
        refusal_why(
            (HarnessState::Unknown, first == Some(PLAIN), first),
            (HarnessState::Unknown, second == Some(PLAIN), second),
            "muse",
            ToolKind::Muse,
        )
    };
    assert_eq!(
        who(Some(PLAIN), Some(WATCHING)),
        "background task: \"└ ◇ background evidence watch  watching\""
    );
    assert_eq!(
        who(Some(WATCHING), Some(PLAIN)),
        "background task: \"└ ◇ background evidence watch  watching\""
    );
    assert_eq!(who(Some(PLAIN), None), "no frame");
    assert_eq!(
        who(Some(PLAIN), Some(PLAIN)),
        "the frame changed between the two readings"
    );
}

#[test]
fn muse_is_proven_by_its_scan_only_and_claude_by_its_own() {
    assert!(limit_proven(PLAIN, "muse", ToolKind::Muse));
    assert!(!limit_proven(WATCHING, "muse", ToolKind::Muse));
    // Each tool reads its own grammar: neither accepts the other's frame.
    assert!(!limit_proven(PLAIN, "claude", ToolKind::Claude));
    assert!(!limit_proven(CLAUDE_LIMITED, "muse", ToolKind::Muse));
    assert!(limit_proven(CLAUDE_LIMITED, "claude", ToolKind::Claude));
    // A tool with neither a grammar nor a scan proves nothing, on any frame.
    assert!(!limit_proven(PLAIN, "opencode", ToolKind::OpenCode));
    assert!(!limit_proven(CLAUDE_LIMITED, "codex", ToolKind::Codex));
}

#[test]
fn claude_keeps_its_own_refusal_text_for_a_box_it_cannot_prove() {
    // A blank row between the rule and the prompt: the frame grammar (blank
    // rows dropped) still reads the box, the limit scan (blank rows kept) does not.
    let lines: Vec<&str> = CLAUDE_LIMITED.lines().collect();
    let at = lines
        .iter()
        .position(|row| row.trim() == "❯" && !row.starts_with("❯ "))
        .unwrap_or_default();
    let mut gapped = lines.clone();
    gapped.insert(at, "");
    let gapped = gapped.join("\n");
    assert_eq!(
        frame_gap(
            (HarnessState::Unknown, false, Some(&gapped)),
            "claude",
            ToolKind::Claude
        )
        .as_deref(),
        Some("no limit row")
    );
}
