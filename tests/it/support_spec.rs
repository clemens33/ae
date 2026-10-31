//! Independent R1a controls; requirements and API agreed before implementation.

use std::cell::Cell;
use std::time::{Duration, Instant};

use super::support::{first_pane, observe, try_first_pane, try_observe};

#[test]
fn failed_empty_read_cannot_prove_absence_or_reach_predicate() {
    let predicates = Cell::new(0);
    let result = try_observe(
        |_| (false, String::new()),
        &["list-panes", "-t", "=absent"],
        Instant::now(),
        |out| {
            predicates.set(predicates.get() + 1);
            out.is_empty()
        },
    );
    assert!(result.is_err(), "a failed empty read cannot prove absence");
    assert_eq!(predicates.get(), 0, "status must precede the predicate");
}

#[test]
fn failed_plausible_pane_is_never_a_pane() {
    let why = try_first_pane(
        |_| (false, "%72\n".to_owned()),
        &["-s", "-t", "=kept"],
        Instant::now(),
    )
    .expect_err("a failed tmux response cannot yield %72");
    assert!(!why.ok);
    assert_eq!(why.out, "%72\n");
}

#[test]
fn successful_empty_read_is_not_a_first_pane() {
    let why = try_first_pane(|_| (true, String::new()), &["-t", "empty"], Instant::now())
        .expect_err("success alone does not establish a pane");
    assert!(why.ok);
    assert!(why.out.is_empty());
}

#[test]
fn first_pane_keeps_first_line_and_exact_selector_argv() {
    let reads = Cell::new(0);
    let pane = first_pane(
        |args| {
            reads.set(reads.get() + 1);
            assert_eq!(
                args,
                ["list-panes", "-s", "-t", "=session:1", "-F", "#{pane_id}"]
            );
            (true, "%41\n%9\n".to_owned())
        },
        &["-s", "-t", "=session:1"],
        Instant::now(),
    );
    assert_eq!(pane, "%41", "first line, not last line or the whole output");
    assert_eq!(reads.get(), 1, "one successful read suffices");
}

#[test]
fn first_pane_does_not_skip_an_empty_first_line() {
    assert!(
        try_first_pane(
            |_| (true, "\n%41\n".to_owned()),
            &["-t", "empty-first"],
            Instant::now(),
        )
        .is_err(),
        "the first line itself must be nonempty"
    );
}

#[test]
fn failed_read_can_recover_within_the_same_deadline() {
    let reads = Cell::new(0);
    let pane = first_pane(
        |_| {
            reads.set(reads.get() + 1);
            if reads.get() == 1 {
                (false, "%wrong\n".to_owned())
            } else {
                (true, "%83\n".to_owned())
            }
        },
        &["-t", "recover"],
        Instant::now() + Duration::from_secs(30),
    );
    assert_eq!(pane, "%83", "discard the failed read, then observe again");
    assert_eq!(reads.get(), 2, "stop at the first qualifying read");
}

#[test]
fn successful_but_empty_read_can_recover_within_the_deadline() {
    let reads = Cell::new(0);
    let pane = first_pane(
        |_| {
            reads.set(reads.get() + 1);
            if reads.get() == 1 {
                (true, String::new())
            } else {
                (true, "%83\n".to_owned())
            }
        },
        &["-t", "empty-then-ready"],
        Instant::now() + Duration::from_secs(30),
    );
    assert_eq!(pane, "%83", "a successful unmet read is not terminal");
    assert_eq!(reads.get(), 2, "stop at the first qualifying read");
}

#[test]
fn future_deadline_terminates_unmet_observation_with_last_read() {
    let reads = Cell::new(0);
    let why = try_first_pane(
        |_| {
            reads.set(reads.get() + 1);
            (true, String::new())
        },
        &["-t", "never-ready"],
        Instant::now() + Duration::from_millis(1),
    )
    .expect_err("an unmet observation must terminate at the deadline");
    assert!(reads.get() >= 1, "even a spent deadline observes once");
    assert_eq!(why.reads, reads.get(), "report every actual observation");
    assert!(why.ok);
    assert!(why.out.is_empty());
}

#[test]
fn spent_deadline_keeps_one_observation_but_never_retries() {
    let reads = Cell::new(0);
    let why = try_first_pane(
        |_| {
            reads.set(reads.get() + 1);
            if reads.get() == 1 {
                (false, "%failed\n".to_owned())
            } else {
                (true, "%too-late\n".to_owned())
            }
        },
        &["-t", "spent"],
        Instant::now(),
    )
    .expect_err("an exhausted budget cannot buy a second read");
    assert_eq!(reads.get(), 1);
    assert_eq!(why.reads, 1);
    assert!(!why.ok);
    assert_eq!(why.out, "%failed\n");
}

#[test]
fn unmet_diagnostics_preserve_status_output_and_arguments() {
    let args = ["list-panes", "-t", "=gone", "-F", "#{pane_id}"];
    let why = try_observe(
        |_| (false, "server unavailable\n%17\n".to_owned()),
        &args,
        Instant::now(),
        |_| true,
    )
    .expect_err("plausible failed output must time out");
    assert_eq!(why.args, args);
    assert!(!why.ok);
    assert_eq!(why.out, "server unavailable\n%17\n");
    assert_eq!(why.reads, 1);
    let message = why.to_string();
    for evidence in [
        "list-panes",
        "=gone",
        "ok=false",
        "server unavailable",
        "%17",
    ] {
        assert!(message.contains(evidence), "missing {evidence}: {message}");
    }
}

#[test]
fn observe_preserves_untrimmed_output_and_predicate_selection() {
    let args = ["list-clients", "-F", "#{session_name}"];
    let predicates = Cell::new(0);
    let out = observe(
        |seen| {
            assert_eq!(seen, args);
            (true, "other\nkeepalive\n".to_owned())
        },
        &args,
        Instant::now(),
        |out| {
            predicates.set(predicates.get() + 1);
            out.lines().any(|line| line == "keepalive")
        },
    );
    assert_eq!(out, "other\nkeepalive\n");
    assert_eq!(predicates.get(), 1);
}

#[test]
fn two_pane_cardinality_rejects_one_three_and_failed_two() {
    for (ok, out) in [(true, "%1\n"), (true, "%1\n%2\n%3\n"), (false, "%1\n%2\n")] {
        let result = try_observe(
            |_| (ok, out.to_owned()),
            &["list-panes", "-t", "hub", "-F", "#{pane_id}"],
            Instant::now(),
            |seen| seen.lines().count() == 2,
        );
        assert!(result.is_err(), "status={ok}, output={out:?}");
    }
}

#[test]
fn two_pane_cardinality_keeps_tmux_order() {
    let out = observe(
        |_| (true, "%81\n%3\n".to_owned()),
        &["list-panes", "-t", "hub", "-F", "#{pane_id}"],
        Instant::now(),
        |seen| seen.lines().count() == 2,
    );
    assert_eq!(out.lines().collect::<Vec<_>>(), ["%81", "%3"]);
}

#[test]
fn panicking_wrappers_retain_failed_read_diagnostics() {
    for first in [false, true] {
        let failure = std::panic::catch_unwind(|| {
            if first {
                first_pane(
                    |_| (false, "%failed\n".to_owned()),
                    &["-t", "=panic"],
                    Instant::now(),
                )
            } else {
                observe(
                    |_| (false, "%failed\n".to_owned()),
                    &["list-panes", "-t", "=panic"],
                    Instant::now(),
                    |_| true,
                )
            }
        })
        .expect_err("the infallible wrapper must panic on unmet status");
        let message = failure
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| failure.downcast_ref::<&str>().copied())
            .expect("a textual panic");
        for evidence in ["ok=false", "%failed", "=panic"] {
            assert!(message.contains(evidence), "missing {evidence}: {message}");
        }
    }
}
