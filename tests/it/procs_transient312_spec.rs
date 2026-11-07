//! #312 acceptance specimens: a transient `?`-state row (macOS shows `?N` with
//! the comm `(ae)` for a process between fork/exec or exit) in the tool's
//! lineage must not void a POSITIVE #283 reading, while every reading that
//! rests on an unreadable row stays unknown.
//!
//! Oracle: issue #312 and its invariant, never the code under test. The task
//! chain is the measured one (`.local/f587-ps-072913.txt`, tool `claude` ->
//! `zsh Ss` -> `zsh` -> `just` -> `bash` -> `perl` -> `cargo-nextest` -> test
//! binary -> `ae`), with the `?N (ae)` rows hung where that snapshot had them.

use super::{parse_stat_table, task_running};

fn running(raw: &str) -> Option<bool> {
    task_running(parse_stat_table(raw).as_deref(), 90, "claude")
}

const TOOL: &str =
    "1 0 Ss /sbin/launchd\n90 1 Ss -fish\n100 90 S+ /Users/example/.local/bin/claude\n";
/// The measured background task: a session-leading shell with live non-shell
/// descendants (`just`, `perl`, `cargo-nextest`, a test binary, `ae`).
const TASK: &str = "110 100 Ss /bin/zsh\n111 110 S /bin/zsh\n112 111 S just\n113 112 S just\n\
                    114 113 S bash\n115 114 S perl\n116 115 SNs /Users/example/.cargo/bin/cargo-nextest\n\
                    117 116 SN /work/target/debug/deps/it-b2dff2e74cc85b86\n118 117 SN /work/target/debug/ae\n";
/// An idle claude: an MCP wrapper shell (not a session leader) running docker.
const IDLE: &str = "110 100 S+ /bin/sh\n111 110 S+ docker\n";

#[test]
fn a_transient_row_deep_under_the_measured_task_chain_keeps_the_positive_reading() {
    assert_eq!(
        running(&format!("{TOOL}{TASK}119 118 ?N (ae)\n")),
        Some(true)
    );
}

#[test]
fn transient_rows_at_every_depth_of_the_tool_lineage_keep_the_positive_reading() {
    // Seven rows, as measured: direct under the tool, under the task shell,
    // mid-chain, under the test binary and under `ae`.
    let transients = "119 118 ?N (ae)\n120 117 ?N (ae)\n121 100 ?N (ae)\n122 110 ?N (ae)\n\
                      123 115 ?N (ae)\n124 116 ?N (ae)\n125 119 ?N (ae)\n";
    assert_eq!(running(&format!("{TOOL}{TASK}{transients}")), Some(true));
}

#[test]
fn a_transient_row_between_the_task_shell_and_its_witness_does_not_hide_the_witness() {
    assert_eq!(
        running(&format!(
            "{TOOL}110 100 Ss /bin/zsh\n111 110 ?N (ae)\n112 111 S cargo\n"
        )),
        Some(true)
    );
}

#[test]
fn a_transient_row_under_an_execed_session_leading_command_keeps_the_positive_reading() {
    assert_eq!(
        running(&format!("{TOOL}110 100 Ss cargo\n111 110 ?N (ae)\n")),
        Some(true)
    );
}

#[test]
fn an_unreadable_sibling_of_a_real_task_root_does_not_void_that_task() {
    assert_eq!(
        running(&format!("{TOOL}110 100 ?N (ae)\n120 100 Ss cargo\n")),
        Some(true)
    );
}

#[test]
fn an_unreadable_tool_row_is_unknown_even_beside_a_positive_witness() {
    for tool in ["100 90 ?N claude\n", "100 90 ? claude\n"] {
        let raw = format!("1 0 Ss /sbin/launchd\n90 1 Ss -fish\n{tool}{TASK}");
        assert_eq!(running(&raw), None, "{tool}");
    }
}

#[test]
fn an_unreadable_witness_or_task_root_alone_is_unknown() {
    for body in [
        // The only live descendant of the task shell cannot be read.
        "110 100 Ss /bin/zsh\n120 110 ?N cargo\n",
        // A transient is the only thing below a task shell: it may be the witness.
        "110 100 Ss /bin/zsh\n120 110 ?N (ae)\n",
        // The session-leading task root itself cannot be read.
        "110 100 ?s cargo\n",
        "110 100 ?s /bin/zsh\n120 110 S cargo\n",
    ] {
        assert_eq!(running(&format!("{TOOL}{body}")), None, "{body}");
    }
}

#[test]
fn a_transient_row_beside_a_negative_reading_stays_unknown_never_a_positive() {
    let bodies = [
        // Idle MCP wrapper: nothing positive, the transient may hide one.
        format!("{IDLE}120 100 ?N (ae)\n"),
        format!("{IDLE}121 111 ?N (ae)\n"),
        // A transient the table cannot place (its parent never appeared) may
        // be under the tool as well.
        format!("{IDLE}140 777 ?N (ae)\n"),
        // A zombie is not a witness.
        "110 100 Ss /bin/zsh\n120 110 Z cargo\n121 110 ?N (ae)\n".to_owned(),
        // A session-leading shell with no non-shell descendant.
        "110 100 Ss /bin/zsh\n120 110 S /bin/sh\n121 120 ?N (ae)\n".to_owned(),
        // A positive-looking task outside the tool is not this seat's.
        "110 90 Ss cargo\n121 100 ?N (ae)\n".to_owned(),
    ];
    for body in bodies {
        assert_eq!(running(&format!("{TOOL}{body}")), None, "{body}");
    }
}

#[test]
fn a_transient_row_outside_the_tool_subtree_leaves_the_reading_alone() {
    // Parent pid 1: provably not under the tool.
    assert_eq!(
        running(&format!("{TOOL}{IDLE}130 1 ?N (ae)\n")),
        Some(false)
    );
    assert_eq!(running(&format!("{TOOL}{TASK}130 1 ?N (ae)\n")), Some(true));
}

#[test]
fn a_transient_row_the_table_cannot_place_does_not_void_a_positive_reading() {
    // The parent never appeared (it exited between fork and the snapshot). The
    // row may sit under the tool, but a positive reading rests on readable
    // rows only and stands.
    assert_eq!(
        running(&format!("{TOOL}{TASK}140 777 ?N (ae)\n")),
        Some(true)
    );
}

#[test]
fn a_missing_or_ambiguous_tool_stays_unknown_beside_transients() {
    assert_eq!(
        running("1 0 Ss /sbin/launchd\n90 1 Ss -fish\n110 90 ?N (ae)\n"),
        None,
        "no tool row"
    );
    let parallel = format!("{TOOL}105 90 S+ claude\n{TASK}119 118 ?N (ae)\n");
    assert_eq!(running(&parallel), None, "two tool roots in parallel");
}
