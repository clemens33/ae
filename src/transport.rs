//! The exec: the one place this crate starts a child process.
//!
//! [`crate::tmux`] owns both halves that can be WRONG — which server an
//! argument list addresses, and what a completed run means — and both are
//! proven against real isolated servers. What is here is the part in between,
//! which is a detail rather than a decision: it hands a derived argument list to
//! `tmux`, waits, and hands the completed run back to be interpreted. It derives
//! no argv of its own and interprets no bytes of its own.
//!
//! # The door
//!
//! `clippy.toml` denies `std::process::Command` crate-wide with a small
//! enumerated set of exceptions. This is the third, and the first outside a test
//! harness. It is deliberate: a session multiplexer that cannot run tmux cannot
//! answer the question it exists to answer, and until this landed every liveness
//! answer was `unknown` by construction. The boundary keeps its value precisely
//! because crossing it stays conspicuous — one function, one file, and an
//! inventory in `tests/it/parity_self_test.rs` that goes red when the set of
//! crossings changes.
//!
//! The door is a capability, not a licence: everything a caller could want to do
//! with a child process other than run tmux is still denied everywhere,
//! including here, because [`run`] is private and takes an argument list nothing
//! outside this crate can supply.
//!
//! # The exit status decides; the payload never does
//!
//! **SC-017k** grants `running`/`stopped` only to a SUCCESSFUL query, and
//! **SC-017l** sends every failure to `unknown`. This module is where that can
//! be lost, because it is the last place the two facts are still separable:
//! empty stdout from a live server with no sessions and empty stdout from a
//! `tmux` that could not be spawned at all are the same bytes. So the run's
//! SUCCESS leaves here beside its bytes and goes to
//! [`crate::tmux::interpret_sessions`], which fails the query on the status
//! before it looks at anything.
//!
//! A transport that answered `Ok(Vec::new())` for an unreachable server would
//! make every session recorded on it `stopped` — ae asserting sessions are gone
//! on the strength of a question it never asked, which is #105 restated one
//! layer down. A child that could not be SPAWNED (no `tmux` on `PATH`, no
//! permission to execute it) is that same failure and is reported the same way:
//! a failed run, never an empty answer.

use crate::inventory::{DiscoveredSession, Discovery, QueryFailed, ServerId};
use crate::meta::Selector;
use crate::tmux;

/// The program ae runs to talk to a tmux server.
///
/// A bare name, resolved on `PATH` by the OS at spawn time. ae does not read
/// `PATH` itself — `std::env::var` is denied in product code by the same
/// `clippy.toml` that denies `Command`, and resolving a program name is exactly
/// the job the exec already does correctly.
const PROGRAM: &str = "tmux";

/// The real tmux transport: [`Discovery`], answered by running `tmux`.
///
/// Carries nothing. Which server to ask arrives as the [`ServerId`] parameter,
/// because entitlement is decided by [`crate::inventory`] and a transport that
/// held a server of its own would be a second place that decision could live.
#[derive(Debug, Clone, Copy, Default)]
pub struct Tmux;

impl Discovery for Tmux {
    /// Every session `server` reports, each with the ownership marker that
    /// server holds for it.
    ///
    /// One `list-sessions` for the server, then one `show-environment` per name
    /// it returned. Grouping candidates so that a server is asked once is
    /// [`crate::liveness`]'s job and already done before this is called; the
    /// per-name reads are the marker half of SC-017j, which needs the session's
    /// own environment and cannot be answered by the enumeration.
    ///
    /// A marker read that fails yields no marker rather than no session: the
    /// name was returned by a query that DID succeed, so it is present, and
    /// SC-017l routes present-but-not-provably-ours to `unknown` — never to
    /// `stopped`, which would claim a session that just answered is gone.
    fn enumerate(&self, server: &ServerId) -> Result<Vec<DiscoveredSession>, QueryFailed> {
        if !addressable(server) {
            return Err(QueryFailed);
        }
        let (succeeded, stdout) = run(PROGRAM, &tmux::list_sessions_args(server));
        let names = tmux::interpret_sessions(succeeded, &stdout)?;
        Ok(names
            .into_iter()
            .map(|name| {
                let (succeeded, stdout) = run(PROGRAM, &tmux::marker_args(server, &name));
                let marker = tmux::interpret_marker(succeeded, &stdout);
                DiscoveredSession { name, marker }
            })
            .collect())
    }
}

/// The calling pane's identity readings, from the AMBIENT server.
///
/// Ambient, deliberately: a generated helper is invoked inside the session it
/// serves, and the frozen helper it replaces runs a bare `tmux` for exactly
/// this read. The listing's refusal to select an ambient server (SC-1410c) is
/// about entitlement to sessions ae did not record; a pane asking who it is
/// has already been placed by tmux.
#[must_use]
pub fn observe_viewer(pane: &str) -> Option<tmux::ObservedViewer> {
    let (succeeded, stdout) = run(PROGRAM, &tmux::viewer_args(&ServerId::Ambient, pane));
    tmux::interpret_viewer(succeeded, &stdout)
}

/// Whether `server` is something this transport may put on the wire.
///
/// Only sockets can fail this, and only by being relative — SC-405l's
/// `ambiguous`, which [`crate::meta`] already refuses when it parses a selector.
/// Asked again here because a relative socket addresses a DIFFERENT server
/// depending on the directory ae was invoked from, and an answer that depends on
/// the caller's working directory is not the recorded server's answer. Refusing
/// is `unknown`, which is what SC-017l says an unusable pointer is worth.
fn addressable(server: &ServerId) -> bool {
    match server {
        ServerId::Selected(Selector::Socket(path)) => tmux::is_addressable_socket(path),
        ServerId::Ambient | ServerId::Selected(Selector::Name(_)) => true,
    }
}

/// Run `program` with `args`; report whether it succeeded, and what it printed.
///
/// The two returned facts are what [`crate::tmux`]'s interpreters consume, in
/// that order of authority: the `bool` is the decision and the `String` is
/// evidence that only means anything once the `bool` has allowed it to.
///
/// **A spawn failure is `(false, String::new())`** — indistinguishable, by
/// design, from a tmux that ran and failed. Both are "no answer", and inventing
/// a distinction here would put a reason taxonomy in the one place that must not
/// have opinions.
///
/// stderr is captured and dropped. tmux writes `no server running on …` there,
/// and `ae list` must show ae's own diagnostic (SC-017o) rather than leaking the
/// text of a subprocess it chose to run. stdin is not inherited: `output()`
/// nulls it, so a child that reads cannot block a listing forever.
///
/// The stdout bytes are decoded LOSSILY, matching how
/// [`crate::inventory`] derives a durable record's name from a directory entry.
/// The two strings this crate compares are then produced the same way, so a name
/// that survives one survives the other — a stricter decode here would turn one
/// odd name on a server into `unknown` for every session on it, and a different
/// decode would be a way for two spellings of the same name to stop matching.
// THE PRODUCT'S DOOR — the only place PRODUCT code may name
// `std::process::Command`. Two others exist crate-wide, both in the test target:
// the parity harness's (`tests/it/parity.rs`, `mod raw`) and the black-box CLI
// tests' (`tests/it/cli.rs`), which must run the product binary. `clippy.toml`
// denies the TYPE everywhere else, which resolves paths rather than text and so
// holds against UFCS, aliases and re-imports alike.
// `parity_self_test::the_capability_boundary_holds_against_any_lint_relaxation`
// asks clippy under `--force-warn` for the complete list of crossings; the
// counter beside it names them by file. The first is the claim, the second is
// defence in depth — they are not the same strength.
#[allow(
    clippy::disallowed_types,
    reason = "the product's door: ae cannot answer a liveness question without running tmux"
)]
fn run(program: &str, args: &[String]) -> (bool, String) {
    let mut command = std::process::Command::new(program);
    command.args(args);
    match command.output() {
        Ok(output) => (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        ),
        // Nothing ran. Not an empty answer — see the module docs.
        Err(_) => (false, String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::{Tmux, run};
    use crate::inventory::{Discovery, QueryFailed, ServerId};
    use crate::meta::Selector;
    use std::path::PathBuf;

    #[test]
    fn a_program_that_ran_reports_success_and_its_output() {
        // THE CONTROL, AND IT COMES FIRST. Without it a `run` that answered
        // `(false, "")` unconditionally would pass every other test in this
        // file, and the transport would report every server unreachable while
        // looking perfectly well tested.
        //
        // `/bin/echo` is required to exist rather than probed for: a skipped
        // control is a control that never ran.
        assert_eq!(
            run("/bin/echo", &["ok".to_owned()]),
            (true, "ok\n".to_owned()),
            "this suite needs /bin/echo to prove the exec can succeed at all"
        );
    }

    #[test]
    fn a_program_that_cannot_be_spawned_is_a_failed_run_and_not_an_empty_one() {
        // The bytes are the same as a successful empty query's. The BOOL is
        // what tells them apart, and it is the half SC-017l depends on: this
        // pair reaching `interpret_sessions` yields `QueryFailed`, and a
        // transport that returned `(true, "")` here would report every session
        // on the server `stopped`.
        let (succeeded, stdout) = run("ae-no-such-program-exists-anywhere", &[]);
        assert!(!succeeded, "a program that is not there did not run");
        assert!(stdout.is_empty());
        assert_eq!(
            crate::tmux::interpret_sessions(succeeded, &stdout),
            Err(QueryFailed),
            "and the pair is a FAILED query rather than a server with no sessions"
        );
    }

    #[test]
    fn a_child_that_ran_and_failed_is_a_failed_run_though_its_output_reads_like_an_answer() {
        // THE SHAPE THIS SLICE EXISTS TO KILL, and the one the two arms beside
        // it cannot reach. They cover Ok + success, and Err + never-spawned.
        // NEITHER covers Ok + `success() == false` — a child that RAN TO
        // COMPLETION and exited non-zero — which is precisely what
        // `tmux list-sessions` does against a server that is not there.
        //
        // A mutant mapping every completed wait to `(true, stdout)` passes both
        // of the others, and integration only catches it on a machine where
        // tmux happens to be installed. So it is pinned here, at the unit
        // level, where nothing external decides whether the arm runs.
        //
        // The stdout is deliberately NON-EMPTY and plausible. An empty payload
        // would let this pass for the wrong reason — the temptation is not
        // ignoring a status beside no bytes, it is ignoring a status beside
        // bytes that look like an answer. Fed to `interpret_sessions` with the
        // status dropped, "plausible" becomes a SESSION NAMED `plausible`.
        let (succeeded, stdout) = run(
            "/bin/sh",
            &["-c".to_owned(), "echo plausible; exit 1".to_owned()],
        );
        assert!(
            !succeeded,
            "a child that completed with a non-zero status did not succeed"
        );
        assert_eq!(
            stdout, "plausible\n",
            "and its bytes ARE here, which is exactly what makes dropping the status tempting"
        );
        assert_eq!(
            crate::tmux::interpret_sessions(succeeded, &stdout),
            Err(QueryFailed),
            "the status decides; output that reads like an answer does not"
        );
    }

    #[test]
    fn a_child_killed_by_a_signal_is_a_failed_run_although_it_has_no_exit_code_at_all() {
        // THE ARM NEITHER OF THE OTHERS REACHES, and it was found by the
        // reviewer attacking cold rather than by my own mutation list — which
        // is the argument for cold attacks in one sentence.
        //
        // A signalled child has NO exit code: `status.code()` is `None`. The
        // natural spelling of this check — "is the code zero?" — has to decide
        // what `None` means, and `code().map_or(true, |c| c == 0)` reads it as
        // SUCCESS. That mutant leaves the entire suite green: the exit-1 arm
        // beside this one still HAS a code, so it cannot see the difference.
        //
        // What it would cost in production: a tmux killed mid-query returns
        // empty stdout and "success", which is `Ok(vec![])` — a successful
        // query proving every name absent. Every session on that server goes
        // `stopped`. That is #105 arriving through a door none of my own
        // attacks knocked on.
        //
        // `success()` is the spelling that is right for free, because it asks
        // whether the child SUCCEEDED rather than what number it returned.
        // This arm exists to keep it that way.
        let (succeeded, stdout) = run("/bin/sh", &["-c".to_owned(), "kill -TERM $$".to_owned()]);
        assert!(
            !succeeded,
            "a child that died on a signal did not succeed, though it reported no code"
        );
        assert!(stdout.is_empty(), "and it printed nothing before dying");
        assert_eq!(
            crate::tmux::interpret_sessions(succeeded, &stdout),
            Err(QueryFailed),
            "empty output from a killed child is not a server with no sessions"
        );
    }

    #[test]
    fn a_relative_socket_is_refused_rather_than_put_on_the_wire() {
        // WHAT THIS PINS IS THE OUTCOME, NOT THE REFUSAL. `tmux -S rel/x` would
        // also fail, so this test cannot distinguish the guard from tmux's own
        // answer, and it does not claim to. The guard exists for the case the
        // test cannot stage: a relative path that DOES resolve, to a different
        // server for every directory ae is invoked from.
        let relative = ServerId::Selected(Selector::Socket(PathBuf::from("relative/ae.sock")));
        assert_eq!(Tmux.enumerate(&relative), Err(QueryFailed));
    }
}
