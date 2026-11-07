//! Independent #283 task specimens, including the measured Codex execed root.

use super::{parse_stat_table, task_running};

fn running(raw: &str, binary: &str) -> Option<bool> {
    task_running(parse_stat_table(raw).as_deref(), 90, binary)
}

const CLAUDE: &str = "1 0 Ss /sbin/launchd\n90 1 Ss -zsh\n100 90 S+ claude\n";
const CODEX: &str =
    "1 0 Ss /sbin/launchd\n90 1 Ss -zsh\n100 90 S+ /Users/example/.local/bin/codex\n";

#[test]
fn a_task_shell_with_a_live_deep_non_shell_command_counts() {
    assert_eq!(
        running(
            &format!("{CLAUDE}110 100 Ss /bin/zsh\n120 110 S cargo\n"),
            "claude"
        ),
        Some(true)
    );
    assert_eq!(
        running(
            &format!("{CLAUDE}110 100 Ss /bin/zsh\n115 110 S /bin/sh\n120 115 R cargo-nextest\n"),
            "claude"
        ),
        Some(true)
    );
}

#[test]
fn the_measured_codex_execed_session_leader_counts_without_a_shell() {
    // Independent observation: .local/chal225/ptree-codex.txt, both exec_command
    // sleep 120 and bash -lc sleep 120 left a direct sleep Ss row under Codex.
    assert_eq!(
        running(&format!("{CODEX}110 100 Ss sleep\n"), "codex"),
        Some(true)
    );
    assert_eq!(
        running(&format!("{CODEX}110 100 Rs cargo-nextest\n"), "codex"),
        Some(true)
    );
}

#[test]
fn idle_mcp_children_and_their_session_leading_grandchildren_do_not_count() {
    let idle = format!(
        "{CODEX}110 100 S /bin/sh\n111 110 S docker\n120 100 S node\n130 100 S caffeinate\n140 100 S python\n141 140 Ss uv\n142 140 Ss rust-analyzer\n"
    );
    assert_eq!(running(&idle, "codex"), Some(false));
    assert_eq!(
        running(&format!("{idle}150 100 Ss cargo\n"), "codex"),
        Some(true),
        "a real task still counts beside idle MCP roots"
    );
}

#[test]
fn shell_wrappers_zombies_and_unrelated_siblings_are_not_live_tasks() {
    for suffix in [
        "110 100 Ss /bin/zsh\n",
        "110 100 Ss /bin/zsh\n120 110 S /bin/sh\n",
        "110 100 Ss /bin/zsh\n120 110 Z cargo\n",
        "110 100 Zs cargo\n",
        "110 90 Ss cargo\n",
        "110 100 S /bin/sh\n120 110 R cargo\n",
    ] {
        assert_eq!(
            running(&format!("{CODEX}{suffix}"), "codex"),
            Some(false),
            "{suffix}"
        );
    }
}

#[test]
fn a_launcher_inner_tool_chain_is_one_seat_but_parallel_tool_roots_are_ambiguous() {
    let chain = format!("{CODEX}105 100 S /opt/codex/bin/codex\n110 105 Ss cargo\n");
    assert_eq!(running(&chain, "codex"), Some(true));
    let parallel = format!("{CODEX}105 90 S /opt/codex/bin/codex\n110 100 Ss cargo\n");
    assert_eq!(running(&parallel, "codex"), None);
    let forked = format!(
        "{CODEX}105 100 S /opt/codex/bin/codex\n106 100 S /other/codex\n110 105 Ss cargo\n"
    );
    assert_eq!(
        running(&forked, "codex"),
        None,
        "two inner branches do not prove one tool chain"
    );
}

#[test]
fn failed_or_unreadable_process_proofs_never_manufacture_activity() {
    assert_eq!(task_running(None, 90, "codex"), None);
    assert_eq!(running("1 0 Ss launchd\n90 1 Ss -zsh\n", "codex"), None);
    assert_eq!(running(&format!("{CODEX}110 100 ? cargo\n"), "codex"), None);
    assert_eq!(running(&format!("{CODEX}110 100 Ss\n"), "codex"), None);
    assert_eq!(
        running(
            &format!("{CODEX}110 100 Ss cargo\n120 90 ? unrelated\n"),
            "codex"
        ),
        Some(true),
        "an unrelated unreadable sibling is outside the tool subtree"
    );
}
