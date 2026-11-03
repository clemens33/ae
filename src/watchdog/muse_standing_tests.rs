//! Driver-owned unit tests of `limit_standing` for muse, beside the frozen
//! acceptance spec in `src/reseat/limitmuse_spec.rs`: each variant of
//! [`LimitStanding`], each row shape the muse reader accepts or names, and the
//! boundaries the spec reaches only through the stop leg.
use super::{LimitStanding, limit_standing};

const PLAIN: &str = include_str!("../../tests/fixtures/harness-state/muse-limit-plain.txt");
const WATCHING: &str = include_str!("../../tests/fixtures/harness-state/muse-limit-watching.txt");
const IDLE_429: &str =
    include_str!("../../tests/fixtures/runtime-identity/muse-idle-plain-80x24.txt");

const VOICE: &str = "── Voice input";
const FOOTER: &str = "  muse-wonder-full-1.0 · max · ~/project · YOLO";
const WARNING: &str =
    "⚠ Usage nearly exhausted · 5h: 2% left · resets 12:11 PM · as of 8:47 AM · /usage";
const WARNING_TAIL: &str = "This task may stop before it finishes.";

fn standing(frame: &str) -> LimitStanding {
    limit_standing(frame, "muse")
}

fn row_after(row: &str) -> LimitStanding {
    LimitStanding::RowAfterCell(row.to_owned())
}

/// `frame` without the rows `drop` selects.
fn without(frame: &str, drop: impl Fn(&str) -> bool) -> String {
    frame
        .lines()
        .filter(|row| !drop(row))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `PLAIN` with `row` inserted ahead of the voice rule, after its furniture.
fn before_voice(row: &str) -> String {
    PLAIN.replacen(VOICE, &format!("{row}\n{VOICE}"), 1)
}

/// `PLAIN` with its two furniture rows replaced by `rows`.
fn with_furniture(rows: &[&str]) -> String {
    let lines: Vec<&str> = PLAIN.lines().collect();
    let (block, rest) = (&lines[..5], &lines[7..]);
    let mut out = block.to_vec();
    out.extend_from_slice(rows);
    out.extend_from_slice(rest);
    out.join("\n")
}

#[test]
fn the_measured_frames_read_as_their_standing() {
    assert_eq!(standing(PLAIN), LimitStanding::Stands);
    assert_eq!(
        standing(WATCHING),
        LimitStanding::BackgroundTask("└ ◇ background evidence watch  watching".to_owned()),
        "the padding and the elapsed counter past it are cut"
    );
    // Trailing blank rows are no ink: the window is measured from the last ink.
    assert_eq!(standing(&format!("{PLAIN}\n\n\n")), LimitStanding::Stands);
    assert_eq!(
        standing(&format!("older transcript\n\n{PLAIN}")),
        LimitStanding::Stands
    );
}

#[test]
fn only_claude_and_muse_have_a_scan_and_each_reads_only_its_own() {
    for bin in ["codex", "opencode", "grok", "", "Muse"] {
        assert_eq!(limit_standing(PLAIN, bin), LimitStanding::NoCell, "{bin}");
    }
    assert_eq!(limit_standing(PLAIN, "claude"), LimitStanding::NoCell);
}

#[test]
fn a_cell_that_is_no_usage_limit_stop_is_no_cell() {
    // The per-turn 429 row is a limit notice but no stop.
    assert_eq!(standing(IDLE_429), LimitStanding::NoCell);
    let no_stop = without(PLAIN, |row| row.contains("This turn stopped"));
    assert_eq!(standing(&no_stop), LimitStanding::NoCell);
    // The stop row must sit under the head, indented.
    let flush = PLAIN.replace("  This turn stopped", "This turn stopped");
    assert_eq!(standing(&flush), LimitStanding::NoCell);
    // A newer head without its own block is the newest cell, and it is no stop.
    assert_eq!(
        standing(&before_voice("◆ Usage limit reached again")),
        LimitStanding::NoCell
    );
    // A row that merely contains the words is no head.
    let inside = PLAIN.replace("◆ Usage limit reached", "◆ Note: Usage limit reached");
    assert_eq!(standing(&inside), LimitStanding::NoCell);
    assert_eq!(standing(""), LimitStanding::NoCell);
    assert_eq!(standing("\n \n"), LimitStanding::NoCell);
}

#[test]
fn an_unknown_follow_up_row_names_itself() {
    let frame = PLAIN.replace(
        "  Check /usage for details.\n",
        "  Check /usage for details.\n  a new transcript continuation\n",
    );
    assert_eq!(standing(&frame), row_after("a new transcript continuation"));
    // The hints are exact; the reset row is a prefix, its time varies.
    let reworded = PLAIN.replace("Check /usage for details.", "Check /usage for details!");
    assert_eq!(standing(&reworded), row_after("Check /usage for details!"));
    let reset = without(WATCHING, |row| {
        row.starts_with("main ·") || row.starts_with("└ ◇")
    });
    assert_eq!(standing(&reset), LimitStanding::Stands);
    let other = reset.replace("Your usage resets at", "Your usage reset at");
    assert_eq!(standing(&other), row_after("Your usage reset at 12:11 PM"));
    // A NBSP-indented row is not the block's: it stands alone after it.
    let nbsp = PLAIN.replace("  Check /usage", "\u{a0}\u{a0}Check /usage");
    assert_eq!(standing(&nbsp), row_after("Check /usage for details."));
}

#[test]
fn rows_between_the_block_and_the_box_are_furniture_or_name_themselves() {
    // Both furniture rows are optional and in either order.
    assert_eq!(
        standing(&with_furniture(&[WARNING_TAIL, WARNING])),
        LimitStanding::Stands
    );
    assert_eq!(standing(&with_furniture(&[WARNING])), LimitStanding::Stands);
    assert_eq!(
        standing(&with_furniture(&[WARNING_TAIL])),
        LimitStanding::Stands
    );
    assert_eq!(standing(&with_furniture(&[])), LimitStanding::Stands);
    // Column 0 and exact (the warning a prefix): anything else is a row.
    for row in [
        "  ⚠ Usage nearly exhausted · 5h: 2% left",
        "⚠ Usage exhausted · 5h",
        "This task may stop before it finishes",
        "This task may stop before it finishes. And more.",
        "  This task may stop before it finishes.",
        "◆ Ran 3 commands",
        "❯ echoed message",
    ] {
        assert_eq!(
            standing(&before_voice(row)),
            row_after(row.trim()),
            "{row:?}"
        );
    }
}

#[test]
fn the_box_is_the_voice_rule_an_empty_prompt_and_a_rule() {
    let no_voice = PLAIN.replace(VOICE, "── Quiet input");
    assert_eq!(standing(&no_voice), LimitStanding::NoBox);
    let indented_voice = PLAIN.replace(VOICE, " ── Voice input");
    assert_eq!(standing(&indented_voice), LimitStanding::NoBox);
    let draft = PLAIN.replace("\n❯\n", "\n❯ draft\n");
    assert_eq!(standing(&draft), LimitStanding::NoBox);
    let second_line = PLAIN.replace("\n❯\n", "\n❯\n  draft\n");
    assert_eq!(standing(&second_line), LimitStanding::NoBox);
    // A text row, not a rule, closes the box.
    let unclosed: Vec<&str> = PLAIN
        .lines()
        .map(|row| {
            if row.starts_with("────") {
                "not a rule"
            } else {
                row
            }
        })
        .collect();
    assert_eq!(standing(&unclosed.join("\n")), LimitStanding::NoBox);
    // The prompt as the capture's last row has no closing rule.
    let cut: Vec<&str> = PLAIN.lines().take(9).collect();
    assert_eq!(standing(&cut.join("\n")), LimitStanding::NoBox);
    // A whole composer without its footer is not a whole frame.
    let bare = without(PLAIN, |row| row.trim_start().starts_with("muse-"));
    assert_eq!(standing(&bare), LimitStanding::NoBox);
}

#[test]
fn a_box_above_the_block_proves_nothing_and_never_panics() {
    // An old empty box, then the block with no box under it.
    let old = format!(
        "{VOICE} (alt+v)\n❯\n────────────────\n{FOOTER}\n\n{}",
        PLAIN.lines().take(4).collect::<Vec<_>>().join("\n")
    );
    assert_eq!(standing(&old), LimitStanding::NoBox);
    // The prompt directly under the block's last row, no voice rule between.
    let flush = without(PLAIN, |row| {
        row.is_empty() || row.starts_with(VOICE) || row.starts_with('⚠') || row == WARNING_TAIL
    });
    assert_eq!(standing(&flush), LimitStanding::NoBox);
    // The voice rule directly under the block: no furniture, no blank row.
    let tight = without(PLAIN, |row| {
        row.is_empty() || row.starts_with('⚠') || row == WARNING_TAIL
    });
    assert_eq!(standing(&tight), LimitStanding::Stands);
}

#[test]
fn only_the_model_footer_may_sit_below_the_box() {
    assert!(PLAIN.contains(FOOTER));
    for (label, row) in [
        ("three fields", "muse-wonder-full-1.0 · max · ~/project"),
        (
            "five fields",
            "muse-wonder-full-1.0 · max · ~/project · YOLO · extra",
        ),
        ("empty field", "muse-wonder-full-1.0 ·  · ~/project · YOLO"),
        ("prose", "◆ A task started after the footer"),
    ] {
        let frame = PLAIN.replace(FOOTER, &format!("  {row}"));
        assert_eq!(standing(&frame), row_after(row), "{label}");
    }
    // A second footer-shaped row is a second row, named.
    let two = format!("{PLAIN}  muse-spark-1.3 · xhigh · /tmp/p · YOLO\n");
    assert_eq!(
        standing(&two),
        row_after("muse-spark-1.3 · xhigh · /tmp/p · YOLO")
    );
    // The task selector alone names itself: with no task row it is no proof.
    let selector = PLAIN.replace(FOOTER, &format!("main · ↓ to select\n{FOOTER}"));
    assert_eq!(standing(&selector), row_after("main · ↓ to select"));
}

#[test]
fn any_task_row_is_a_background_task_whatever_it_says_and_outranks_a_stray() {
    for status in ["watching", "running", "done", "failed", ""] {
        let frame = WATCHING.replace("watching", status);
        let want = format!("└ ◇ background evidence watch  {status}");
        assert_eq!(
            standing(&frame),
            LimitStanding::BackgroundTask(want.trim_end().to_owned()),
            "{status:?}"
        );
    }
    // A long task row is cut to the refusal's length.
    let long = WATCHING.replace("background evidence watch", &"w".repeat(120));
    match standing(&long) {
        LimitStanding::BackgroundTask(row) => assert_eq!(row.chars().count(), 80),
        other => panic!("{other:?}"),
    }
    // Whatever the glyph after the `└`.
    let other = WATCHING.replace("└ ◇", "└ ✓");
    assert!(matches!(standing(&other), LimitStanding::BackgroundTask(_)));
    // A `└` row ABOVE the box is a row after the block, not a task.
    assert_eq!(
        standing(&before_voice("└ ◇ a thing  running")),
        row_after("└ ◇ a thing  running")
    );
}

#[test]
fn a_block_pushed_out_of_the_window_reads_as_none() {
    // The window is the bottom 16 rows: WATCHING is 14, so two more task rows
    // keep its head in view and a third pushes it out. Out of view is no proof,
    // and with a task below the box a refusal either way.
    let task = |n: usize| format!("main · ↓ to select\n{}", "└ ◇ more  running\n".repeat(n));
    let two = WATCHING.replace("main · ↓ to select\n", &task(2));
    assert!(matches!(standing(&two), LimitStanding::BackgroundTask(_)));
    let three = WATCHING.replace("main · ↓ to select\n", &task(3));
    assert_eq!(standing(&three), LimitStanding::NoCell);
    let far = PLAIN.replace(VOICE, &format!("{}{VOICE}", "stray row\n".repeat(20)));
    assert_eq!(standing(&far), LimitStanding::NoCell);
}

#[test]
fn the_footer_is_a_fleet_model_at_a_known_effort_over_a_path_in_yolo_mode() {
    for model in [
        "muse-spark-1.3",
        "muse-spark-1.3-contributor",
        "muse-wonder-1.0",
        "muse-wonder-1.0-contributor",
        "muse-wonder-full-1.0",
        "muse-wonder-full-1.0-contributor",
    ] {
        for effort in ["max", "xhigh"] {
            for path in ["~/project", "/Users/x/ae"] {
                let row = format!("{model} · {effort} · {path} · YOLO");
                let frame = PLAIN.replace(FOOTER, &format!("  {row}"));
                assert_eq!(standing(&frame), LimitStanding::Stands, "{row}");
            }
        }
    }
    // One field wrong refuses and names the row: an id the fleet does not run
    // (and a near miss of one), an effort other than max or xhigh, a path with
    // no root, any other mode.
    for row in [
        "muse-spark-1.4 · max · ~/project · YOLO",
        "muse-spark-1.3-contributor-x · max · ~/project · YOLO",
        "Muse-Spark-1.3 · max · ~/project · YOLO",
        "muse-spark-1.3 · high · ~/project · YOLO",
        "muse-spark-1.3 · ultra · ~/project · YOLO",
        "muse-spark-1.3 · max · project · YOLO",
        "muse-spark-1.3 · max ·  · YOLO",
        "muse-spark-1.3 · max · ~/project · yolo",
        "muse-spark-1.3 · max · ~/project · PLAN",
        "◆ Running job · still active · do not stop · 14 seconds",
    ] {
        let frame = PLAIN.replace(FOOTER, &format!("  {row}"));
        assert_eq!(standing(&frame), row_after(row), "{row}");
    }
}

#[test]
fn the_voice_rule_is_a_labelled_rule_as_wide_as_the_closing_rule() {
    // Both measured hint spellings, at the width of the capture.
    let (rule, voice) = {
        let rows: Vec<&str> = PLAIN.lines().collect();
        (rows[9], rows[7])
    };
    assert!(voice.starts_with(VOICE) && rule.starts_with("────"));
    let width = rule.chars().count();
    let hinted = |hint: &str| {
        let label = format!("{VOICE} ({hint}) ");
        format!("{label}{}", "─".repeat(width - label.chars().count()))
    };
    for hint in ["alt(option)+v to start", "⌥ + v to start", "x"] {
        assert_eq!(
            standing(&PLAIN.replace(voice, &hinted(hint))),
            LimitStanding::Stands
        );
    }
    // A label with no rule, a shorter or a longer rule, no hint, no
    // parenthesis, no space before the rule: not the composer's top rule.
    for row in [
        "── Voice input is recording a new request".to_owned(),
        "── Voice input (alt(option)+v to start)".to_owned(),
        format!("── Voice input (alt+v) {}", "─".repeat(width - 21)),
        format!("── Voice input (alt+v) {}", "─".repeat(width - 19)),
        format!("── Voice input alt+v) {}", "─".repeat(width - 20)),
        format!("── Voice input() {}", "─".repeat(width - 17)),
        format!("── Voice input (alt+v){}", "─".repeat(width - 20)),
        format!("──Voice input (alt+v) {}", "─".repeat(width - 21)),
    ] {
        assert_eq!(
            standing(&PLAIN.replace(voice, &row)),
            LimitStanding::NoBox,
            "{row}"
        );
    }
}

#[test]
fn the_head_is_the_phrase_alone_or_then_a_dot_and_its_tail() {
    let head = "◆ Usage limit reached";
    for ok in [
        head.to_owned(),
        format!("{head}  "),
        format!("{head} · /upgrade (https://example.test) for more, or wait"),
    ] {
        let frame = WATCHING
            .replacen(head, &ok, 1)
            .lines()
            .filter(|row| !row.starts_with("main ·") && !row.starts_with("└ ◇"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(standing(&frame), LimitStanding::Stands, "{ok:?}");
    }
    for bad in [
        "◆ Usage limit reachedElsewhere and a new turn is active",
        "◆ Usage limit reached again",
        "◆ Usage limit reached: a new turn is active",
        "◆ Usage limit reached· no space",
        "◆ Usage limit reached ·",
    ] {
        let frame = PLAIN.replacen(
            "◆ Usage limit reached · /upgrade (https://accountscenter.meta.com/muse_code/?ep=xgrade) for increased limits, or wait for usage to reset at 12:11 PM",
            bad,
            1,
        );
        assert_eq!(standing(&frame), LimitStanding::NoCell, "{bad:?}");
    }
}
