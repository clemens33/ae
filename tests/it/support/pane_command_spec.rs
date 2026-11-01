//! Agreed v3 API contract, with the helper-only spec-first waiver recorded.

use std::cell::Cell;
use std::time::{Duration, Instant};

use super::{PANE_WAIT, has_execed, is_shell, pane_command, try_pane_command};

#[test]
fn shell_and_exec_predicates_prove_different_stages() {
    for name in ["sh", "bash", "dash", "zsh", "fish"] {
        assert!(is_shell(name), "a genuine shell: {name}");
        assert!(has_execed(name), "a shell has exec'd: {name}");
    }
    for name in ["", "tmux", "sleep", "cat", "perl", "ae", "sh.exe", " sh "] {
        assert!(!is_shell(name), "not a bare shell name: {name:?}");
    }
    assert!(!has_execed(""));
    assert!(!has_execed("tmux"));
    for name in ["sleep", "cat", "perl", "ae"] {
        assert!(has_execed(name), "exec alone is not tool readiness: {name}");
    }
}

#[test]
fn one_successful_read_uses_exact_pane_and_format_and_caller_selection() {
    for (raw, expected) in [(" cat \n", "cat"), ("\ttmux\n", "tmux"), (" \n", "")] {
        let reads = Cell::new(0);
        let read = |args: &[&str]| {
            reads.set(reads.get() + 1);
            assert_eq!(
                args,
                [
                    "display-message",
                    "-p",
                    "-t",
                    "%71",
                    "#{pane_current_command}"
                ]
            );
            (true, raw.to_owned())
        };
        let select = |seen: &str| {
            assert_eq!(seen, expected);
            true
        };
        let actual =
            try_pane_command(read, "%71", Instant::now(), select).expect("caller-selected command");
        assert_eq!(actual, expected);
        assert_eq!(
            reads.get(),
            1,
            "even a spent deadline permits one successful read"
        );
        assert_eq!(pane_command(read, "%71", Instant::now(), select), expected);
        assert_eq!(
            reads.get(),
            2,
            "the failing wrapper also stops at its first success"
        );
    }
}

#[test]
fn failed_plausible_command_never_reaches_predicate_and_keeps_raw_diagnostics() {
    let predicates = Cell::new(0);
    let reads = Cell::new(0);
    let why = try_pane_command(
        |_| {
            reads.set(reads.get() + 1);
            (false, "  sh\n".to_owned())
        },
        "%72",
        Instant::now(),
        |_| {
            predicates.set(predicates.get() + 1);
            true
        },
    )
    .expect_err("failed stdout is not a shell");
    assert_eq!(predicates.get(), 0);
    assert_eq!(reads.get(), 1, "a spent deadline buys no retry");
    assert_eq!(why.reads, 1);
    assert!(!why.ok);
    assert_eq!(why.out, "  sh\n");
    assert_eq!(
        why.args,
        [
            "display-message",
            "-p",
            "-t",
            "%72",
            "#{pane_current_command}"
        ]
    );
}

#[test]
fn failed_and_unmet_reads_can_recover_but_stop_at_first_qualifying_success() {
    let reads = Cell::new(0);
    let predicates = Cell::new(0);
    let actual = try_pane_command(
        |_| {
            let sample = [(false, "cat"), (true, " tmux\n"), (true, " cat\n")][reads.get()];
            reads.set(reads.get() + 1);
            (sample.0, sample.1.to_owned())
        },
        "%73",
        Instant::now() + Duration::from_secs(30),
        |name| {
            predicates.set(predicates.get() + 1);
            name == "cat"
        },
    )
    .expect("the third read observes cat");
    assert_eq!(actual, "cat");
    assert_eq!(reads.get(), 3);
    assert_eq!(
        predicates.get(),
        2,
        "failed output never reaches the selector"
    );
}

#[test]
fn unmet_success_keeps_the_last_raw_read_and_panicking_form_names_the_failure() {
    let why = try_pane_command(
        |_| (true, "  tmux\n".to_owned()),
        "%74",
        Instant::now(),
        is_shell,
    )
    .expect_err("a successful pre-exec read is not a shell");
    assert!(why.ok);
    assert_eq!(why.out, "  tmux\n");
    assert_eq!(why.reads, 1);
    let failure = std::panic::catch_unwind(|| {
        pane_command(
            |_| (false, "cat\n".to_owned()),
            "%75",
            Instant::now(),
            |_| true,
        )
    })
    .expect_err("the failing wrapper cannot return failed stdout");
    let text = failure
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| failure.downcast_ref::<&str>().copied())
        .expect("textual diagnostic");
    for expected in ["%75", "pane_current_command", "ok=false", "cat"] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
}

#[test]
fn real_pane_starts_reach_the_named_recorder_and_a_genuine_shell() {
    let rig = crate::seat_relaunch::Rig::new("panecontract");
    for (start, expected) in [("exec cat", "cat"), ("exec /bin/sh", "sh")] {
        let (ok, pane) = rig.tmux(&[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            &rig.session,
            start,
        ]);
        assert!(ok, "private pane starts: {pane}");
        let accepts: fn(&str) -> bool = if expected == "sh" {
            is_shell
        } else {
            |name| name == "cat"
        };
        let command = pane_command(
            |args| rig.tmux(args),
            pane.trim(),
            Instant::now() + PANE_WAIT,
            accepts,
        );
        if expected == "sh" {
            assert!(is_shell(&command), "a genuine shell: {command}");
        } else {
            assert_eq!(command, "cat");
        }
        assert!(has_execed(&command));
    }
}
