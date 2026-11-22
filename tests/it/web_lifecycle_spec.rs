//! S1a command lifecycle through the built binary and an owned loopback socket.

use std::net::{Ipv4Addr, TcpListener};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use crate::cli;

#[allow(
    clippy::disallowed_methods,
    reason = "the fixture owns the busy loopback port independently of ae"
)]
fn occupied_port() -> TcpListener {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap_or_else(|error| panic!("reserve an ephemeral loopback port: {error}"))
}

fn refusal(root: &Path, port: u16, reason: &str) {
    let child = cli::ae()
        .env("AE_HOME", root)
        .args(["web", "--port", &port.to_string()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("spawn ae web: {error}"));
    let out = cli::bounded(child, Duration::from_secs(5))
        .unwrap_or_else(|| panic!("ae web must refuse promptly: {reason}"));
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains(reason), "{stderr}");
    assert!(!stdout.contains("#c="), "{stdout}");
    assert!(!stderr.contains("#c="), "{stderr}");
}

#[test]
fn the_binary_refuses_an_occupied_port_before_printing_any_code() {
    let holder = occupied_port();
    let port = holder
        .local_addr()
        .unwrap_or_else(|error| panic!("owned loopback address: {error}"))
        .port();
    let scratch = cli::OwnedScratch::root("web", "busy");
    refusal(
        scratch.path(),
        port,
        &format!("port {port} in use; not starting"),
    );
}

#[test]
fn a_held_state_root_lock_refuses_and_an_unheld_lock_file_does_not() {
    let holder = occupied_port();
    let port = holder
        .local_addr()
        .unwrap_or_else(|error| panic!("owned loopback address: {error}"))
        .port();
    let scratch = cli::OwnedScratch::root("web", "locked");
    let web = scratch.path().join("web");
    std::fs::create_dir(&web).unwrap_or_else(|error| panic!("fixture web directory: {error}"));
    let held = ae::store::lock(&web.join("lock"), Duration::ZERO)
        .unwrap_or_else(|error| panic!("fixture state root lock: {error}"));
    refusal(
        scratch.path(),
        port,
        "another ae web runs for this state root",
    );
    drop(held);
    refusal(
        scratch.path(),
        port,
        &format!("port {port} in use; not starting"),
    );
}

#[test]
fn only_the_documented_nonzero_port_flag_is_accepted() {
    use ae::web;

    assert_eq!(web::DEFAULT_PORT, 7480);
    for (args, port) in [
        (Vec::new(), 7480),
        (vec!["--port", "1"], 1),
        (vec!["--port", "65535"], 65535),
    ] {
        let args = args.into_iter().map(str::to_owned).collect::<Vec<_>>();
        let accepted = web::parse(&args).unwrap_or_else(|_| panic!("valid argv: {args:?}"));
        assert_eq!(accepted.port, port);
    }
    for args in [
        vec!["--port"],
        vec!["--port", "0"],
        vec!["--port", "65536"],
        vec!["--port", "port"],
        vec!["--port", "7480", "--port", "7481"],
        vec!["--host", "0.0.0.0"],
        vec!["7480"],
    ] {
        let args = args.into_iter().map(str::to_owned).collect::<Vec<_>>();
        assert!(web::parse(&args).is_err(), "{args:?}");
    }
}
