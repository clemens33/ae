//! Driver-owned unit tests of the two muse readers a long-limit stop is proven
//! from (#304), beside the frozen acceptance in `src/autoreseat/bgtask_spec.rs`:
//! `muse_background_tasks` and `muse_limit_reset`, each shape they accept and
//! each row that must refuse.
use super::{muse_background_tasks, muse_limit_reset, muse_task_title};

const PLAIN: &str = include_str!("../../tests/fixtures/harness-state/muse-limit-plain.txt");
const WATCHING: &str = include_str!("../../tests/fixtures/harness-state/muse-limit-watching.txt");
/// A measured muse pane whose own 429 row muse wrapped over three rows at 80
/// columns, above an idle composer.
const IDLE_429: &str =
    include_str!("../../tests/fixtures/runtime-identity/muse-idle-plain-80x24.txt");

const TITLE: &str = "background evidence watch";
const TASK: &str = "└ ◇ background evidence watch  watching";
const SELECTOR: &str = "main · ↓ to select";
const FOOTER: &str = "  muse-wonder-full-1.0 · max · ~/project · YOLO";
/// `date -u -j -f %Y-%m-%dT%H:%M:%SZ 2026-09-14T22:56:52Z +%s`.
const MEASURED_RESET: i64 = 1_789_426_612;
/// `date -u -j -f %Y-%m-%dT%H:%M:%SZ 2027-01-15T08:00:00Z +%s`.
const RESET: i64 = 1_800_000_000;
const WRAPPED: &str = "◆ model failed: API error 429 \
    [request_id=00000000-0000-0000-0000-000000000000]:\n  Subscription quota exhausted. Your \
    usage window resets at\n  2027-01-15T08:00:00Z. (rate_limit_error)";
const UNWRAPPED: &str = "◆ model failed: API error 429 \
    [request_id=00000000-0000-0000-0000-000000000000]: Subscription quota exhausted. Your usage \
    window resets at 2027-01-15T08:00:00Z. (rate_limit_error)";

/// The measured 429 block of [`IDLE_429`], its three rows as muse drew them.
fn measured_block() -> String {
    let rows: Vec<&str> = IDLE_429.lines().collect();
    let start = rows
        .iter()
        .position(|row| row.starts_with("◆ model failed: "))
        .unwrap_or_else(|| panic!("the measured 429 row"));
    rows[start..start + 3].join("\n")
}

/// `block`, then `between`, then the watching stop frame.
fn stop_after(block: &str, between: &str) -> String {
    format!("{block}\n\n{between}{WATCHING}")
}

/// The watching frame with its footer region replaced by `rows`.
fn footer(rows: &[&str]) -> String {
    let (head, _) = WATCHING
        .split_once(&format!("{SELECTOR}\n"))
        .unwrap_or_else(|| panic!("the measured selector"));
    format!("{head}{}\n", rows.join("\n"))
}

#[test]
fn the_measured_frame_names_its_one_task() {
    assert_eq!(
        muse_background_tasks(WATCHING),
        Some(vec![TITLE.to_owned()])
    );
}

#[test]
fn several_tasks_keep_their_order_and_the_selector_is_optional() {
    let two = footer(&[SELECTOR, TASK, "└ ◆ second task  done 1s", FOOTER]);
    assert_eq!(
        muse_background_tasks(&two),
        Some(vec![TITLE.to_owned(), "second task".to_owned()])
    );
    let bare = footer(&[TASK, FOOTER]);
    assert_eq!(muse_background_tasks(&bare), Some(vec![TITLE.to_owned()]));
}

#[test]
fn a_frame_with_anything_but_tasks_under_the_box_is_no_proof() {
    for (what, frame) in [
        ("no task at all", PLAIN.to_owned()),
        ("the selector alone", footer(&[SELECTOR, FOOTER])),
        ("no footer", footer(&[SELECTOR, TASK])),
        ("two footers", footer(&[SELECTOR, TASK, FOOTER, FOOTER])),
        (
            "a row before the selector",
            footer(&["human prompt", SELECTOR, TASK, FOOTER]),
        ),
        (
            "a row after the task",
            footer(&[SELECTOR, TASK, "human prompt", FOOTER]),
        ),
        (
            "a second selector",
            footer(&[SELECTOR, SELECTOR, TASK, FOOTER]),
        ),
        ("the footer first", footer(&[SELECTOR, FOOTER, TASK])),
        ("a draft", WATCHING.replace("\n❯\n", "\n❯ draft\n")),
        (
            "a turn after the stop",
            WATCHING.replace("── Voice input", "◆ Ran a command\n── Voice input"),
        ),
        ("no stop", IDLE_429.to_owned()),
        ("nothing", String::new()),
    ] {
        assert_eq!(muse_background_tasks(&frame), None, "{what}");
    }
}

#[test]
fn a_task_title_is_cut_from_its_glyph_status_and_time() {
    for (row, title) in [
        (TASK, Some(TITLE)),
        (
            "└ ◆ two words here  running            1m 02s",
            Some("two words here"),
        ),
        ("└ ◈ a single  done", Some("a single")),
        ("└ ◇ trailing", Some("trailing")),
        ("└ ◇  watching", None),
        ("└ ◇ ", None),
        ("└ ◇", None),
        ("└ ◇title", None),
        ("└  title", None),
        ("└", None),
        ("◇ title", None),
        ("main · ↓ to select", None),
    ] {
        assert_eq!(muse_task_title(row), title, "{row:?}");
    }
}

#[test]
fn the_measured_wrapped_row_states_its_reset() {
    assert_eq!(
        muse_limit_reset(&stop_after(&measured_block(), "")),
        Some(MEASURED_RESET)
    );
    assert_eq!(muse_limit_reset(&stop_after(WRAPPED, "")), Some(RESET));
    assert_eq!(muse_limit_reset(&stop_after(UNWRAPPED, "")), Some(RESET));
    // A stop with no task under it reads the same row.
    assert_eq!(
        muse_limit_reset(&format!("{WRAPPED}\n\n{PLAIN}")),
        Some(RESET)
    );
}

#[test]
fn queued_user_echoes_may_sit_between_the_row_and_the_stop() {
    for between in [
        "❯ ⟦ae:msg from watchdog⟧\n  the queued body\n\n",
        "❯ one\n\n  a later paragraph of it\n\n❯ two\n\n",
        "   \n\n",
    ] {
        assert_eq!(
            muse_limit_reset(&stop_after(WRAPPED, between)),
            Some(RESET),
            "{between:?}"
        );
    }
}

#[test]
fn any_other_row_between_the_row_and_the_stop_is_no_proof() {
    for between in [
        "── Voice input (alt(option)+v to start) ───\n",
        "❯\n",
        "❯  \n",
        "  an indented row with no echo above it\n",
        "❯ echo\n\u{a0} a non-breaking indent under it\n",
        "◆ Ran command · new work\n",
        "$ muse\n",
        "└ ◇ an old task  watching\n",
        "main · ↓ to select\n",
        "muse-wonder-full-1.0 · max · ~/project · YOLO\n",
        "❯ echo\nnot indented\n",
    ] {
        assert_eq!(
            muse_limit_reset(&stop_after(WRAPPED, between)),
            None,
            "{between:?}"
        );
    }
}

#[test]
fn only_the_newest_whole_column_zero_row_counts() {
    // Two rows: the newer one decides.
    let older = WRAPPED.replace("2027-01-15", "2027-01-20");
    assert_eq!(
        muse_limit_reset(&stop_after(&format!("{older}\n\n{WRAPPED}"), "")),
        Some(RESET)
    );
    for (what, block) in [
        (
            "quoted",
            WRAPPED.replacen("◆ model failed", "  ◆ model failed", 1),
        ),
        (
            "upper-case request id",
            WRAPPED.replace("00000000-0000", "ABCDEF00-0000"),
        ),
        (
            "another error",
            WRAPPED.replace("Subscription quota exhausted.", "Overloaded."),
        ),
        (
            "a local time",
            WRAPPED.replace("2027-01-15T08:00:00Z", "Jan 15 at 8:00 AM"),
        ),
        (
            "a tail after it",
            WRAPPED.replace("(rate_limit_error)", "(rate_limit_error) more"),
        ),
        (
            "a cut sentence",
            WRAPPED.replace("\n  2027-01-15T08:00:00Z. (rate_limit_error)", ""),
        ),
        (
            "a year zero clock",
            WRAPPED.replace("2027-01-15T08:00:00Z", "0000-01-01T00:00:00Z"),
        ),
    ] {
        assert_eq!(muse_limit_reset(&stop_after(&block, "")), None, "{what}");
    }
}

#[test]
fn a_row_with_no_stop_below_it_or_below_the_stop_is_no_proof() {
    // The measured idle pane: a 429 row and no stop.
    assert_eq!(muse_limit_reset(IDLE_429), None);
    // The stop's own local times are never read.
    assert_eq!(muse_limit_reset(WATCHING), None);
    // A row below the stop head is not above it.
    let below = WATCHING.replace("── Voice input", &format!("{UNWRAPPED}\n── Voice input"));
    assert_eq!(muse_limit_reset(&below), None);
}
