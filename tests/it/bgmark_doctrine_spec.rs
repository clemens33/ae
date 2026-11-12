//! Approved S2 doctrine, lead refinement ae-20261112T075112Z-4cad99e4.
//! Text contract, not runtime proof. Whitespace may wrap in the rendered source.
fn bgmark_doctrine_words() -> String {
    include_str!("../../src/render/core.txt")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn bgmark_own_job_wait_is_working() {
    assert!(
        bgmark_doctrine_words()
            .contains("Waiting on your own running job is working, not waiting-agent.")
    );
}

#[test]
fn bgmark_long_jobs_default_to_the_harness_for_attribution() {
    assert!(bgmark_doctrine_words().contains("Run long jobs as your harness's background tool call: ae sees and attributes only jobs under your seat."));
}

#[test]
fn bgmark_detached_jobs_need_a_seat_lifetime_or_remote_exception() {
    assert!(bgmark_doctrine_words().contains(
        "Use `nohup` or `setsid` only when a job must outlive your seat or runs in a remote session"
    ));
}

#[test]
fn bgmark_exception_jobs_record_pid_log_exit_files_in_the_state_reason() {
    assert!(bgmark_doctrine_words().contains(
        "then write its pid, log and exit files under .local/<slice>/ and name them in your state reason."
    ));
}
