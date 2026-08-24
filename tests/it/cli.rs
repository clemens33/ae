//! Black-box tests: run the real binary, not the library.
//!
//! `CARGO_BIN_EXE_ae` is set by cargo for integration tests of a package with a
//! `[[bin]]` — it is the path to the binary this test run just built, so this
//! exercises argv handling, exit code mapping and stdout for real.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use super::parity::Invocation;
use super::parity::capture::ExitOutcome;
use super::parity::capture::raw;

// ONE OF THREE DOORS — `clippy.toml` denies `std::process::Command` crate-wide
// and `parity_self_test::the_capability_boundary_holds_against_any_lint_relaxation`
// pins the complete inventory of exceptions by asking the compiler for it.
//
// This one is not a parity concern: these tests drive the PRODUCT binary and
// asserting on what it printed is their whole job, where the parity harness
// must never judge a lane. `ae` is private to this module, so nothing in the
// harness can reach a child process through it. The third door is the product's
// own, in `src/transport.rs`; a binary this file runs may therefore spawn tmux
// of its own accord, which is what makes the liveness assertions below real.
#[allow(
    clippy::disallowed_types,
    reason = "black-box tests must run the product binary; see clippy.toml"
)]
fn ae() -> std::process::Command {
    std::process::Command::new(env!("CARGO_BIN_EXE_ae"))
}

#[test]
fn version_prints_the_version_line_and_exits_zero() {
    let out = ae()
        .arg("--version")
        .output()
        .expect("the ae binary should run");

    assert!(out.status.success(), "exit status: {:?}", out.status);
    let stdout = String::from_utf8(out.stdout).expect("stdout should be utf-8");
    assert_eq!(stdout, format!("ae {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn sc_022_an_unknown_option_exits_two_and_diagnoses_on_stderr() {
    let out = ae()
        .arg("--frobnicate")
        .output()
        .expect("the ae binary should run");

    assert_eq!(out.status.code(), Some(2), "exit status: {:?}", out.status);
    assert!(
        out.stdout.is_empty(),
        "stdout must stay empty for a machine caller, got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8(out.stderr).expect("stderr should be utf-8");
    assert!(stderr.contains("--frobnicate"), "stderr: {stderr}");
}

#[test]
fn sc_022_a_top_level_session_name_is_never_an_unknown_command() {
    let out = ae()
        .arg("my-feature")
        .output()
        .expect("the ae binary should run");

    assert_ne!(
        out.status.code(),
        Some(2),
        "a session name is not usage-wrong"
    );
    assert!(out.stdout.is_empty(), "stdout: {:?}", out.stdout);
    let stderr = String::from_utf8(out.stderr).expect("stderr should be utf-8");
    assert!(
        !stderr.contains("unknown"),
        "the row forbids this phrase for such a token: {stderr}"
    );
}

#[test]
fn criterion_1_the_real_list_and_ls_surfaces_answer_over_a_real_state_root() {
    // THE REFUSAL IS GONE. The shipped binary is invoked exactly as an operator
    // would invoke it, against a state root planted on disk, and it renders —
    // human and JSON, `list` and `ls`. The phase-2 baseline refused here with
    // `no session source is wired`; nothing in this build can print that,
    // because the constant no longer exists.
    let root = scratch("entry-point");
    plant_session(&root, "AlphaR");
    plant_session(&root, "ZetaR");

    for spelling in ["list", "ls"] {
        for json in [false, true] {
            let mut command = ae();
            command.env("AE_HOME", &root).arg(spelling);
            if json {
                command.arg("--json");
            }
            let out = command.output().expect("the ae binary should run");
            let stdout = String::from_utf8(out.stdout).expect("stdout should be utf-8");
            let stderr = String::from_utf8(out.stderr).expect("stderr should be utf-8");

            // Per-surface (gate blob 8cccbe44 / OC-P3-HUMAN-DIAGNOSTIC vs
            // OC-P3-JSON-WARNING): incomplete-human rc is open; JSON process
            // rc is retained. These planted sessions record a server this
            // invocation cannot query, so the snapshot is incomplete.
            if json {
                assert_eq!(
                    out.status.code(),
                    Some(0),
                    "{spelling}/json={json}: {stderr}"
                );
            }
            assert!(
                !stderr.contains("no session source"),
                "{spelling}/json={json}: the unwired refusal came back: {stderr}"
            );
            assert!(
                stdout.contains("AlphaR") && stdout.contains("ZetaR"),
                "{spelling}/json={json}: the planted sessions did not reach output: {stdout}"
            );
            // THE STATUS IS `unknown`, AND THAT IS THE WHOLE POINT. The
            // transport is real now and it really ran: these sessions record a
            // server that is not running, so the query FAILED — and SC-017l says
            // an unanswerable query is `unknown`, never `stopped`. If the
            // transport reported a SUCCESSFUL EMPTY query instead of a failure,
            // every one of these rows would say `stopped`: ae would be asserting
            // these sessions are gone on the strength of a question that got no
            // answer. That is #105 restated at the entry point.
            //
            // The opposed arm — a server that DOES answer, making the same route
            // say `running` and `stopped` — is in `transport.rs`. Without it this
            // assertion would also pass on a transport that can never succeed.
            assert!(
                stdout.contains("unknown"),
                "{spelling}/json={json}: an unverifiable session must be unknown: {stdout}"
            );
            assert!(
                !stdout.contains("stopped"),
                "{spelling}/json={json}: nothing was proven absent, so nothing may say \
                 stopped: {stdout}"
            );
            if json {
                assert!(
                    stdout.contains(r#""schema_version":2"#),
                    "{spelling}: the successor document is what reached stdout: {stdout}"
                );
                // INCOMPLETE, and correctly so: these sessions record a server,
                // ae is entitled to ask it, and this build cannot. SC-017o makes
                // an entitled server whose enumeration fails a loss, so the
                // snapshot says it could not look everywhere — rather than
                // reporting a complete picture it did not establish.
                assert!(
                    stdout.contains(r#""inventory_complete":false"#),
                    "{spelling}: a build that cannot query must not claim completeness: {stdout}"
                );
            } else {
                assert!(
                    !stderr.is_empty(),
                    "{spelling}: and the human surface says so on stderr"
                );
            }
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// Run `git` with `args` and return its stdout, or `None` if it could not run.
///
/// Through the parity harness's pinned door rather than a second `Command` in
/// this file: `the_doors_to_a_child_process_are_the_inventoried_ones` counts
/// relaxations per file, and this needs no new one.
fn git(args: &[&str]) -> Option<String> {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let scratch = std::env::temp_dir().join(format!("ae-git-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&scratch);
    let out = scratch.join("out");
    let err = scratch.join("err");
    let mut invocation = Invocation::new("git");
    for arg in args {
        invocation = invocation.arg(arg);
    }
    let status = raw::run(&invocation, manifest, &out, &err).ok()?;
    let text = std::fs::read_to_string(&out).ok();
    let _ = std::fs::remove_dir_all(&scratch);
    matches!(status.outcome(), ExitOutcome::Code(0)).then_some(text?)
}

#[test]
fn criterion_1_the_opposed_control_is_that_the_phase_2_baseline_still_refused() {
    // A POSITIVE WITHOUT AN OPPOSED CONTROL PROVES LESS THAN IT LOOKS. The arm
    // above shows `list` and `ls` answer today; this one shows they did NOT
    // before, so the change is attributable to this work rather than to a
    // refusal that was never reachable in the first place.
    //
    // The baseline is DERIVED, not written down: the most recent commit that
    // changed the refusal constant in `src/lib.rs` is the one that removed it,
    // so its parent is the last tree that still had it. A hardcoded sha would
    // rot the first time history is rewritten, and would say nothing about WHY
    // that commit is the boundary.
    let Some(removal) = git(&[
        "log",
        "-S",
        "NO_SESSION_SOURCE",
        "--format=%H",
        "--",
        "src/lib.rs",
    ]) else {
        panic!("this control needs git and the repository history to be present");
    };
    let removal = removal
        .lines()
        .next()
        .unwrap_or_else(|| panic!("the refusal constant must appear in history"))
        .to_owned();
    let baseline = format!("{removal}^:src/lib.rs");
    let Some(before) = git(&["show", &baseline]) else {
        panic!("the baseline tree must be readable: {baseline}");
    };

    // THE OPPOSED CONTROL: the baseline refused, and had no callable render path
    // from the entry point — `run` handed `run_with` no source at all.
    assert!(
        before.contains("NO_SESSION_SOURCE"),
        "the baseline must still carry the refusal, or it is not the opposed control"
    );
    assert!(
        before.contains("no session source is wired"),
        "including the message a user would have seen"
    );
    assert!(
        before.contains("run_with(args, None, out, err)"),
        "and its entry point reached no world at all"
    );

    // TODAY: neither the constant nor its message exists anywhere in the crate.
    let Ok(now) = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"),
    ) else {
        panic!("src/lib.rs must be readable");
    };
    assert!(
        !now.contains("NO_SESSION_SOURCE") && !now.contains("no session source is wired"),
        "the refusal is gone from the product, not merely unreachable"
    );
}

#[test]
fn a_machine_that_cannot_say_where_its_state_lives_is_told_so() {
    // The one remaining refusal, and it is about THIS INVOCATION rather than
    // about the build: no AE_HOME, no HOME, nothing to enumerate from.
    let out = ae()
        .env_clear()
        .arg("list")
        .output()
        .expect("the ae binary should run");

    assert_eq!(out.status.code(), Some(1), "{:?}", out.status);
    assert!(out.stdout.is_empty(), "stdout must stay empty");
    let stderr = String::from_utf8(out.stderr).expect("stderr should be utf-8");
    assert!(stderr.contains(ae::NO_STATE_ROOT), "stderr: {stderr}");
}

/// A scratch state root, short-lived and per-test.
///
/// `/tmp` DIRECTLY rather than `std::env::temp_dir()`, because a socket path
/// lives under here now. `sun_path` is 104 bytes on macOS and `temp_dir()`
/// eats most of them, so `<root>/no-server.sock` can exceed the limit — and
/// then tmux fails for PATH LENGTH rather than for the absence this fixture
/// means to assert. That is the right answer for the wrong reason, which is
/// worse than the premise it was meant to replace: it would survive a transport
/// that had stopped being able to look at all. `phase2.rs` and `transport.rs`
/// use `/tmp` for the same reason.
fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(format!("/tmp/ae-cli-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        std::fs::create_dir_all(dir.join("sessions")).is_ok(),
        "a scratch state root"
    );
    dir
}

/// A durable session the product will discover for itself.
///
/// The meta carries a POSITIVE server selector, and that is load-bearing rather
/// than decorative: without one, SC-405l normalizes the selector to `missing`,
/// the classifier never asks anything, and the liveness query branch is never
/// reached. A fixture like that cannot tell a transport that FAILS from one that
/// answers successfully-empty — and those two differ by exactly the `unknown`
/// versus `stopped` this test exists to pin. The mutation lane found that hole;
/// the selector closes it.
///
/// THE SERVER MUST NOT EXIST, and that premise is now ASSERTED rather than
/// argued. It became contingent the moment the transport stopped being inert: a
/// fixture recording a server ae actually queries depends on nobody running one
/// by that address, or the query SUCCEEDS, legitimately reports these sessions
/// absent, and renders `stopped` against an assertion of not-stopped.
///
/// A named server could only NARROW that — a per-process name is unlikely to be
/// occupied, never proven unoccupied, and pids are reused. A socket path inside
/// this test's own scratch directory is STRUCTURAL: the directory was created
/// empty moments ago, the path is checked absent here, and no other process has
/// a reason to bind it. The residual on the old form was in the safe direction
/// (a collision can only fabricate an alarm, never mask a defect) but the red
/// would have been unexplainable — a developer cannot tell ae breaking from
/// someone's stray tmux server.
fn plant_session(root: &std::path::Path, name: &str) {
    let dir = root.join("sessions").join(name);
    let server = root.join("no-server.sock");
    assert!(
        !server.exists(),
        "this fixture's whole premise is that nothing answers at {}",
        server.display()
    );
    let written = std::fs::create_dir_all(&dir).and_then(|()| {
        std::fs::write(
            dir.join("meta"),
            format!(
                "mode=local\nagent.main=cl:lead\ntmux_server_kind=socket\ntmux_server={}\n",
                server.display()
            ),
        )
    });
    assert!(written.is_ok(), "a planted session");
}

#[test]
fn an_unknown_list_flag_exits_two_not_one() {
    // The usage error is decided by argv, before the missing source matters —
    // so `2` must not be swallowed by the unwired path's `1`.
    for tail in [["list", "--frobnicate"], ["ls", "my-feature"]] {
        let out = ae().args(tail).output().expect("the ae binary should run");

        assert_eq!(out.status.code(), Some(2), "{tail:?}: {:?}", out.status);
        assert!(out.stdout.is_empty(), "{tail:?}: stdout must stay empty");
        let stderr = String::from_utf8(out.stderr).expect("stderr should be utf-8");
        assert!(stderr.contains(tail[1]), "{tail:?}: {stderr}");
    }
}
