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

// The OTHER door, and the only other one — `clippy.toml` denies
// `std::process::Command` crate-wide and
// `parity_self_test::the_doors_to_a_child_process_are_the_inventoried_ones` pins the
// complete inventory of exceptions.
//
// This one is not a parity concern: these tests drive the PRODUCT binary and
// asserting on what it printed is their whole job, where the parity harness
// must never judge a lane. `ae` is private to this module, so nothing in the
// harness can reach a child process through it.
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

            assert_eq!(
                out.status.code(),
                Some(0),
                "{spelling}/json={json}: {stderr}"
            );
            assert!(
                !stderr.contains("no session source"),
                "{spelling}/json={json}: the unwired refusal came back: {stderr}"
            );
            assert!(
                stdout.contains("AlphaR") && stdout.contains("ZetaR"),
                "{spelling}/json={json}: the planted sessions did not reach output: {stdout}"
            );
            // THE STATUS IS `unknown`, AND THAT IS THE WHOLE POINT. This build
            // has no tmux transport, so no liveness query can succeed — and
            // SC-017l says an unanswerable query is `unknown`, never `stopped`.
            // If the absent transport reported a SUCCESSFUL EMPTY query instead
            // of a failure, every one of these rows would say `stopped`: ae
            // would be asserting these sessions are gone on the strength of a
            // question it never asked. That is #105 restated at the entry point.
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
fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ae-cli-{}-{tag}", std::process::id()));
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
/// reached. A fixture like that cannot tell an absent transport that FAILS from
/// one that answers successfully-empty — and those two differ by exactly the
/// `unknown` versus `stopped` this test exists to pin. The mutation lane found
/// that hole; the selector closes it.
fn plant_session(root: &std::path::Path, name: &str) {
    let dir = root.join("sessions").join(name);
    let written = std::fs::create_dir_all(&dir).and_then(|()| {
        std::fs::write(
            dir.join("meta"),
            "mode=local\nagent.main=cl:lead\ntmux_server_kind=name\ntmux_server=ae-test\n",
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
