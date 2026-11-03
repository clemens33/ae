//! Independent acceptance tests; navigator-owned, installed byte-identically.
use super::{HarnessState, StopPolicy, ToolKind, frame_gap, limit_proven, may_stop};

const PLAIN: &str = include_str!("../../tests/fixtures/harness-state/muse-limit-plain.txt");
const WATCHING: &str = include_str!("../../tests/fixtures/harness-state/muse-limit-watching.txt");
const STOPPED: &str = "  This turn stopped because your usage limit was reached.\n";

fn proves(frame: &str) -> bool {
    limit_proven(frame, "muse", ToolKind::Muse)
}

fn before_voice(frame: &str, row: &str) -> String {
    frame.replacen("── Voice input", &format!("{row}\n── Voice input"), 1)
}

#[test]
fn limitmuse_measured_plain_frame_proves_only_the_limit_stop() {
    assert!(proves(PLAIN));
    assert!(proves(&format!("[older transcript]\n{PLAIN}\n\n")));
    let idle = include_str!("../../tests/fixtures/runtime-identity/muse-idle-plain-80x24.txt");
    assert!(!proves(idle));
    assert_eq!(
        crate::harness_state::classify(idle, ToolKind::Muse),
        HarnessState::Unknown
    );
    assert!(!limit_proven(PLAIN, "claude", ToolKind::Claude));
}

#[test]
fn limitmuse_measured_reset_row_and_short_head_are_accepted_without_the_task() {
    let without_task = WATCHING
        .lines()
        .filter(|row| !row.starts_with("main ·") && !row.starts_with("└ ◇"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        proves(&without_task),
        "the reset continuation and short head are vendor rows"
    );
    let no_warning = PLAIN
        .lines()
        .filter(|row| !row.starts_with('⚠') && !row.starts_with("This task may stop"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(proves(&no_warning), "warning furniture is optional");
}

#[test]
fn limitmuse_background_tasks_refuse_and_name_the_task() {
    for status in ["watching", "running"] {
        let frame = WATCHING.replace("watching", status);
        assert!(
            !proves(&frame),
            "a {status} task still runs below the composer"
        );
        let why = frame_gap(
            (HarnessState::Unknown, false, Some(&frame)),
            "muse",
            ToolKind::Muse,
        )
        .expect("a refusal names the task");
        assert!(
            why.contains("background evidence watch") && why.contains(status),
            "{why}"
        );
    }
}

#[test]
fn limitmuse_any_later_transcript_row_refuses_even_when_indented() {
    for row in [
        "◆ Running a command after the limit",
        "❯ Please continue",
        "  a new transcript continuation",
        "◆ Usage limit reached but a different turn is now active",
    ] {
        let frame = before_voice(PLAIN, row);
        assert!(!proves(&frame), "later transcript: {row}");
    }
    let frame = before_voice(PLAIN, "◆ Running a command after the limit");
    let why = frame_gap(
        (HarnessState::Unknown, false, Some(&frame)),
        "muse",
        ToolKind::Muse,
    )
    .expect("row refusal");
    assert!(why.contains("Running a command after the limit"), "{why}");
    let abutting = PLAIN.replace(
        STOPPED,
        &format!("{STOPPED}  a new transcript continuation\n"),
    );
    assert!(
        !proves(&abutting),
        "unknown indented prose is not vendor furniture"
    );
}

#[test]
fn limitmuse_drafts_and_incomplete_or_old_frames_never_prove() {
    let no_head = PLAIN.lines().skip(1).collect::<Vec<_>>().join("\n");
    let no_bottom = PLAIN
        .lines()
        .filter(|row| !row.starts_with("────"))
        .collect::<Vec<_>>()
        .join("\n");
    let no_footer = PLAIN
        .lines()
        .filter(|row| !row.trim_start().starts_with("muse-"))
        .collect::<Vec<_>>()
        .join("\n");
    let no_voice = PLAIN
        .lines()
        .filter(|row| !row.starts_with("── Voice input"))
        .collect::<Vec<_>>()
        .join("\n");
    let outside_window = before_voice(PLAIN, &"\n".repeat(40));
    for (label, frame) in [
        (
            "draft",
            PLAIN.replace("\n❯\n", "\n❯ unfinished human draft\n"),
        ),
        (
            "multiline draft",
            PLAIN.replace("\n❯\n", "\n❯\n  unfinished human draft\n"),
        ),
        ("missing head", no_head),
        ("missing stopped row", PLAIN.replace(STOPPED, "")),
        (
            "quoted head",
            PLAIN.replacen("◆ Usage limit reached", "  ◆ Usage limit reached", 1),
        ),
        ("missing bottom", no_bottom),
        ("missing footer", no_footer),
        ("missing voice rule", no_voice),
        ("out of scan window", outside_window),
        (
            "unknown footer row",
            format!("{PLAIN}◆ A task started after the footer\n"),
        ),
    ] {
        assert!(!proves(&frame), "must refuse {label}");
    }
}

#[test]
fn limitmuse_every_fleet_model_and_effort_has_a_valid_footer() {
    // Lead footer ruling; exact model ids independently sourced from fleet profiles.
    for model in [
        "muse-spark-1.3",
        "muse-spark-1.3-contributor",
        "muse-wonder-1.0",
        "muse-wonder-1.0-contributor",
        "muse-wonder-full-1.0",
        "muse-wonder-full-1.0-contributor",
    ] {
        for effort in ["max", "xhigh"] {
            for path in ["~/project", "/tmp/project"] {
                let footer = format!("{model} · {effort} · {path} · YOLO");
                let frame = PLAIN.replace("muse-wonder-full-1.0 · max · ~/project · YOLO", &footer);
                assert!(proves(&frame), "authorized fleet footer: {footer}");
            }
        }
    }
}

#[test]
fn limitmuse_unrecognized_footer_fields_refuse() {
    for footer in [
        "muse-spark-1.4 · max · ~/project · YOLO",
        "muse-spark-1.3 · high · ~/project · YOLO",
        "muse-spark-1.3 · max · project · YOLO",
        "muse-spark-1.3 · max · ~/project · manual",
    ] {
        let frame = PLAIN.replace("muse-wonder-full-1.0 · max · ~/project · YOLO", footer);
        assert!(!proves(&frame), "unknown footer field: {footer}");
    }
}

#[test]
fn limitmuse_four_prose_fields_are_not_a_model_footer() {
    let frame = PLAIN.replace(
        "muse-wonder-full-1.0 · max · ~/project · YOLO",
        "◆ Running job · still active · do not stop · 14 seconds",
    );
    assert!(
        !proves(&frame),
        "a field count cannot prove the vendor model footer"
    );
}

#[test]
fn limitmuse_voice_label_without_its_rule_cannot_prove_a_composer() {
    let frame = PLAIN
        .lines()
        .map(|row| {
            if row.starts_with("── Voice input") {
                "── Voice input is recording a new request"
            } else {
                row
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !proves(&frame),
        "the invariant requires a voice rule, not matching prefix text"
    );
}

#[test]
fn limitmuse_a_different_word_after_the_head_is_not_the_vendor_head() {
    let frame = PLAIN.replacen(
        "◆ Usage limit reached",
        "◆ Usage limit reachedElsewhere and a new turn is active",
        1,
    );
    assert!(
        !proves(&frame),
        "head matching must preserve the vendor words"
    );
}

#[test]
fn limitmuse_both_captures_must_independently_prove_the_limit_stop() {
    let valid = (HarnessState::Unknown, proves(PLAIN));
    let draft = PLAIN.replace("\n❯\n", "\n❯ pending\n");
    for invalid in [WATCHING, draft.as_str(), "", "no readable frame"] {
        let invalid = (HarnessState::Unknown, proves(invalid));
        assert!(!may_stop(valid, invalid, StopPolicy::Proven));
        assert!(!may_stop(invalid, valid, StopPolicy::Proven));
    }
    assert!(may_stop(valid, valid, StopPolicy::Proven));
}
