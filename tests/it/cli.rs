//! Black-box tests: run the real binary, not the library.
//!
//! `CARGO_BIN_EXE_ae` is set by cargo for integration tests of a package with a
//! `[[bin]]` — it is the path to the binary this test run just built, so this
//! exercises argv handling, exit code mapping and stdout for real.

use std::process::Command;

fn ae() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ae"))
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
fn an_unknown_argument_exits_two() {
    let out = ae()
        .arg("--frobnicate")
        .output()
        .expect("the ae binary should run");

    assert_eq!(out.status.code(), Some(2), "exit status: {:?}", out.status);
    let stdout = String::from_utf8(out.stdout).expect("stdout should be utf-8");
    assert!(stdout.contains("--frobnicate"), "stdout: {stdout}");
}
