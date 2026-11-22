//! `ae web` through the built binary: a line a read worker hands over reaches
//! the terminal while the binary's main thread owns it.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;

use crate::cli;

const WAIT: Duration = Duration::from_secs(10);

#[allow(
    clippy::disallowed_methods,
    reason = "the fixture picks a free loopback port for the child and lets it go"
)]
fn free_port() -> u16 {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .and_then(|listener| listener.local_addr())
        .map_or_else(
            |error| panic!("a free loopback port: {error}"),
            |addr| addr.port(),
        )
}

#[test]
fn a_request_without_a_session_answers_401_and_the_terminal_prints_a_fresh_link() {
    let scratch = cli::OwnedScratch::root("web", "run");
    let port = free_port();
    // Killed and reaped when it drops, on every exit path.
    let mut served = cli::ae()
        .env("AE_HOME", scratch.path())
        .args(["web", "--port", &port.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|error| panic!("spawn ae web: {error}"));
    let stdout = served
        .stdout
        .take()
        .unwrap_or_else(|| panic!("piped stdout"));
    let (lines, seen) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    let next = |what: &str| {
        seen.recv_timeout(WAIT)
            .unwrap_or_else(|error| panic!("{what}: {error}"))
    };
    let first = next("startup link");
    let startup = code(&first, &format!("ae web: http://127.0.0.1:{port}/#c="));
    assert_eq!(
        next("startup hint"),
        "ae web: open it in a browser on this machine; Ctrl-C stops it"
    );

    let mut client = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .unwrap_or_else(|error| panic!("connect: {error}"));
    client
        .set_read_timeout(Some(WAIT))
        .unwrap_or_else(|error| panic!("read deadline: {error}"));
    write!(
        client,
        "GET /api/state HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"
    )
    .unwrap_or_else(|error| panic!("request: {error}"));
    client
        .shutdown(Shutdown::Write)
        .unwrap_or_else(|error| panic!("half-close: {error}"));
    let mut reply = String::new();
    client
        .read_to_string(&mut reply)
        .unwrap_or_else(|error| panic!("the whole 401 answer: {error}"));
    assert!(reply.starts_with("HTTP/1.1 401 "), "{reply:?}");
    let (_, body) = reply
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("HTTP framing: {reply:?}"));
    assert!(
        body.contains("no session: open the newest link from ae web"),
        "{body:?}"
    );

    let line = next("fresh link");
    let fresh = code(
        &line,
        &format!("ae web: new link http://127.0.0.1:{port}/#c="),
    );
    assert_ne!(fresh, startup);
    drop(served);
}

/// The one-time code at the end of `line`, which must start with `prefix`
/// and end in exactly 32 lowercase hex characters.
fn code(line: &str, prefix: &str) -> String {
    let code = line
        .strip_prefix(prefix)
        .unwrap_or_else(|| panic!("{line:?} does not start with {prefix:?}"));
    assert!(
        code.len() == 32
            && code
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')),
        "{line:?}"
    );
    code.to_owned()
}
