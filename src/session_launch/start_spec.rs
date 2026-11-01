//! Frozen u26c product contract: a forked tmux child has not started its tool.
//! Owner ruling ae-20261101T022236Z-7bb52d23; agreed seam in plan v4.

use super::{Start, await_start, pane_start};

/// Scripted observations and a counted clock; never sleeps or queries tmux.
fn script(commands: &[&str], launched: &str) -> (usize, usize) {
    let (mut reads, mut naps) = (0, 0);
    await_start(
        || {
            let word = commands
                .get(reads)
                .expect("the wait read after readiness or its agreed budget");
            reads += 1;
            (*word).to_owned()
        },
        || {
            naps += 1;
            assert!(naps <= 25, "neither observation budget may reset");
        },
        "claude",
        launched,
    );
    (reads, naps)
}

#[test]
fn classification_preserves_existing_tool_shell_and_launcher_rules() {
    for (current, tool, launched, expected) in [
        ("tmux", "claude", "ae", Start::Forked),
        ("", "claude", "ae", Start::Waiting),
        ("sh", "claude", "ae", Start::Waiting),
        ("bash", "claude", "ae", Start::Waiting),
        ("dash", "claude", "ae", Start::Waiting),
        ("zsh", "claude", "ae", Start::Waiting),
        ("fish", "claude", "ae", Start::Waiting),
        ("ae", "claude", "ae", Start::Waiting),
        ("ae-core", "claude", "ae-core", Start::Waiting),
        ("claude", "claude", "ae", Start::Started),
        ("opencode.exe", "opencode", "ae", Start::Started),
        ("claude", "claude", "claude", Start::Started),
        ("perl", "claude", "ae", Start::Started),
        ("ae", "claude", "", Start::Started),
    ] {
        assert_eq!(
            pane_start(current, tool, launched),
            expected,
            "current={current:?}, tool={tool:?}, launched={launched:?}"
        );
    }
}

#[test]
fn fork_shell_and_this_launcher_are_not_accepted_as_the_tool() {
    assert_eq!(
        script(&["tmux", "tmux", "sh", "ae", "claude"], "ae"),
        (5, 4)
    );
}

#[test]
fn a_started_tool_or_existing_unknown_process_stops_without_a_nap() {
    assert_eq!(script(&["claude"], "ae"), (1, 0));
    assert_eq!(script(&["perl"], "ae"), (1, 0));
    assert_eq!(script(&["ae"], ""), (1, 0));
}

#[test]
fn no_fork_keeps_the_existing_ten_poll_never_started_cost() {
    assert_eq!(script(&[""; 10], "ae"), (10, 10));
    assert_eq!(script(&["sh"; 10], "ae"), (10, 10));
    assert_eq!(script(&["ae"; 10], "ae"), (10, 10));
}

#[test]
fn fork_observations_do_not_use_the_shell_and_launcher_budget() {
    let mut commands = vec!["tmux"; 12];
    commands.extend(["sh"; 9]);
    commands.push("claude");
    assert_eq!(script(&commands, "ae"), (22, 21));
}

#[test]
fn a_fork_that_outlasts_the_old_total_budget_can_still_reach_its_tool() {
    let mut commands = vec!["tmux"; 12];
    commands.push("claude");
    assert_eq!(script(&commands, "ae"), (13, 12));
}

#[test]
fn a_permanently_forked_child_has_a_finite_extra_budget() {
    let (reads, naps) = script(&["tmux"; 15], "ae");
    assert_eq!(reads, 15);
    assert!(matches!(naps, 14 | 15), "extra naps: {naps}");
}

#[test]
fn the_fork_budget_does_not_reset_when_a_shell_is_seen_between_forks() {
    let mut commands = vec!["tmux"; 7];
    commands.push("sh");
    commands.extend(["tmux"; 8]);
    let (reads, naps) = script(&commands, "ae");
    assert_eq!(reads, 16);
    assert!(matches!(naps, 15 | 16), "extra naps: {naps}");
}

#[test]
fn the_normal_budget_does_not_reset_after_a_fork_observation() {
    let mut commands = vec!["sh"; 9];
    commands.extend(["tmux", "sh"]);
    assert_eq!(script(&commands, "ae"), (11, 11));
}
