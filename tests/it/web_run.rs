//! `ae web` through the built binary: a line a read worker hands over reaches
//! the terminal while the binary's main thread owns it.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc;
use std::time::{Duration, Instant};

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

/// `ae web` with a word it does not take: the usage line on stderr, exit 2.
#[test]
fn an_unknown_word_answers_the_usage_line_and_exit_2() {
    let output = cli::ae()
        .args(["web", "--bogus"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(cli::OwnedChild::wait_with_output)
        .unwrap_or_else(|error| panic!("run ae web --bogus: {error}"));
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "ae web: unexpected --bogus\nusage: ae web [--port N]   (N in 1..=65535; default 7480)\n"
    );
    assert!(output.stdout.is_empty(), "{output:?}");
}

/// docs/web.md: only another `ae web` on the same state root is told that one
/// runs. Any other failure to take `<state root>/web/lock` names the lock and
/// its error, exit 1: here the lock path is a directory.
#[test]
fn a_lock_that_cannot_be_taken_names_the_lock_not_another_listener() {
    let scratch = cli::OwnedScratch::root("web", "lockdir");
    std::fs::create_dir_all(scratch.path().join("web").join("lock"))
        .unwrap_or_else(|error| panic!("a directory where the lock goes: {error}"));
    let output = cli::ae()
        .env("AE_HOME", scratch.path())
        .args(["web", "--port", &free_port().to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(cli::OwnedChild::wait_with_output)
        .unwrap_or_else(|error| panic!("run ae web: {error}"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        stderr.starts_with("ae web: ") && stderr.contains("/web/lock: "),
        "{stderr:?}"
    );
    assert!(
        !stderr.contains("another ae web runs for this state root"),
        "{stderr:?}"
    );
    assert!(output.stdout.is_empty(), "{output:?}");
}

/// The version the follow test repoints the command link to.
const NEXT: &str = "2099.1.1";
/// How long the follow test waits for the told failure after the repoint: the
/// watcher looks every 5 s, so a passing run spends one look of it.
const FOLLOW_DEADLINE: Duration = Duration::from_secs(20);

/// A version directory under `versions` with its installer and manifest, the
/// `app_upgrade_spec` `InstalledRig` recipe: the path its core goes to.
fn version_dir(versions: &Path, version: &str) -> PathBuf {
    let dir = versions.join(version);
    std::fs::create_dir_all(&dir)
        .unwrap_or_else(|error| panic!("version directory {version}: {error}"));
    let zero = "0".repeat(64);
    for (name, text) in [
        ("install", "not executed".to_owned()),
        ("SHA256SUMS", format!("{zero}  ae-core\n{zero}  install\n")),
    ] {
        std::fs::write(dir.join(name), text)
            .unwrap_or_else(|error| panic!("{name} {version}: {error}"));
    }
    dir.join("ae-core")
}

/// Point the command link `link` at `core`.
fn relink(link: &Path, core: &Path) {
    drop(std::fs::remove_file(link));
    std::os::unix::fs::symlink(core, link)
        .unwrap_or_else(|error| panic!("link {}: {error}", core.display()));
}

/// docs/web.md: when the command link names another core and automatic
/// upgrades are on, `ae web` replaces itself with that core; a failed switch
/// is printed once per version and the old core keeps serving. The startup
/// line witnesses the listener's first look, so the link moves only after it.
#[test]
fn an_installed_listener_tells_a_core_it_cannot_follow_and_keeps_serving() {
    let scratch = cli::OwnedScratch::root("web", "follow");
    let home = scratch.path().join("home");
    let ae_home = home.join(".ae");
    let versions = ae_home.join("versions");
    let tmux = scratch.path().join("tmux");
    let link = home.join(".local").join("bin").join("ae");
    for dir in [&tmux, &home.join(".local").join("bin")] {
        std::fs::create_dir_all(dir).unwrap_or_else(|error| panic!("{}: {error}", dir.display()));
    }
    let core = version_dir(&versions, ae::VERSION);
    std::fs::copy(env!("CARGO_BIN_EXE_ae"), &core)
        .unwrap_or_else(|error| panic!("copy the core: {error}"));
    // The next core is complete but cannot run: its exec fails.
    let next = version_dir(&versions, NEXT);
    std::fs::write(&next, "not a core").unwrap_or_else(|error| panic!("next core: {error}"));
    std::fs::set_permissions(&next, std::fs::Permissions::from_mode(0o644))
        .unwrap_or_else(|error| panic!("next core mode: {error}"));
    std::fs::write(ae_home.join("config"), "[workspace]\nauto_upgrade = on\n")
        .unwrap_or_else(|error| panic!("config: {error}"));
    relink(&link, &core);
    let port = free_port();
    // Killed and reaped when it drops, on every exit path.
    let mut served = cli::helper(&link)
        .env("HOME", &home)
        .env("AE_HOME", &ae_home)
        .env("TMUX_TMPDIR", &tmux)
        .env_remove("AE_NO_AUTOSTART")
        .args(["web", "--port", &port.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("spawn the installed ae web: {error}"));
    let out = lines(served.stdout.take());
    let err = lines(served.stderr.take());
    let first = out
        .recv_timeout(WAIT)
        .unwrap_or_else(|error| panic!("startup link: {error}"));
    assert!(first.starts_with("ae web: http://127.0.0.1:"), "{first:?}");

    relink(&link, &next);
    let deadline = Instant::now() + FOLLOW_DEADLINE;
    let prefix = format!("ae web: cannot follow version {NEXT}: ");
    let suffix = format!("; still serving {}", ae::VERSION);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let printed = err.recv_timeout(left).unwrap_or_else(|error| {
            panic!("no `{prefix}...{suffix}` within {FOLLOW_DEADLINE:?}: {error}")
        });
        if printed.starts_with(&prefix) && printed.ends_with(&suffix) {
            break;
        }
    }
    let reply = state(port);
    assert!(reply.starts_with("HTTP/1.1 401 "), "{reply:?}");
    drop(served);
}

/// Every line `stream` prints, handed over by a reader thread.
fn lines(stream: Option<impl std::io::Read + Send + 'static>) -> mpsc::Receiver<String> {
    let stream = stream.unwrap_or_else(|| panic!("a piped stream"));
    let (lines, seen) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    seen
}

/// The whole answer to `GET /api/state` from the listener on `port`.
fn state(port: u16) -> String {
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
        .unwrap_or_else(|error| panic!("the whole answer: {error}"));
    reply
}
