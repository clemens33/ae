//! `ae web`: the orchestrator chat in a browser tab on this machine. A
//! foreground listener on 127.0.0.1 only; Ctrl-C stops it, and its key, codes
//! and sessions die with it. One `ae web` per state root.

pub mod assets;
pub mod auth;
pub mod chat;
pub mod clock;
pub mod http;
pub mod server;
pub mod state;

use std::io::Write;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use self::clock::SystemClock;
use self::server::{NoProbe, Options};
use self::state::LiveSource;

/// The port `ae web` listens on without `--port`.
pub const DEFAULT_PORT: u16 = 7480;

const USAGE: &str = "usage: ae web [--port N]   (N in 1..=65535; default 7480)\n";
const HELD: &str = "ae web: another ae web runs for this state root";
const OPEN: &str = "ae web: open it in a browser on this machine; Ctrl-C stops it";
/// The terminal lines a worker may have waiting for the main thread.
const LINES_QUEUED: usize = 8;

/// What `ae web` was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Args {
    /// The loopback port to listen on.
    pub port: u16,
}

/// A command line `ae web` does not take: the offending token, if one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage(pub Option<String>);

impl std::fmt::Display for Usage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Some(token) => write!(f, "ae web: unexpected {token}\n{USAGE}"),
            None => f.write_str(USAGE),
        }
    }
}

/// Read `tail`, everything after the word `web`: `--port N` at most once.
///
/// # Errors
///
/// [`Usage`] for any other token, a missing or out-of-range port, or a second
/// `--port`.
pub fn parse(tail: &[String]) -> Result<Args, Usage> {
    let mut port = None;
    let mut tokens = tail.iter();
    while let Some(token) = tokens.next() {
        if token != "--port" || port.is_some() {
            return Err(Usage(Some(token.clone())));
        }
        let value = tokens.next().ok_or(Usage(None))?;
        let number = value
            .parse::<u16>()
            .ok()
            .filter(|number| *number != 0)
            .ok_or_else(|| Usage(Some(value.clone())))?;
        port = Some(number);
    }
    Ok(Args {
        port: port.unwrap_or(DEFAULT_PORT),
    })
}

/// `ae web [--port N]`: take the state root's web lock, listen, print the
/// link, then serve until the process is stopped.
///
/// # Errors
///
/// [`crate::error::Error::Io`] when `out` or `err` cannot be written.
pub fn run(
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::error::Result<u8> {
    let args = match parse(tail) {
        Ok(args) => args,
        Err(usage) => {
            write!(err, "{usage}")?;
            return Ok(crate::state::EXIT_USAGE);
        }
    };
    let shape = crate::shape::current();
    let Some(root) = crate::doors::state_root(shape) else {
        writeln!(err, "ae web: {}", crate::shape::NO_STATE_ROOT)?;
        return Ok(crate::state::EXIT_FAILED);
    };
    let dir = root.join("web");
    if let Err(why) = std::fs::create_dir_all(&dir) {
        writeln!(err, "ae web: {}: {why}", dir.display())?;
        return Ok(crate::state::EXIT_FAILED);
    }
    // Held for the whole run: the kernel lets go when the process ends.
    let _lock = match crate::store::lock(&dir.join("lock"), Duration::ZERO) {
        Ok(lock) => lock,
        Err(why) if why.kind() == std::io::ErrorKind::WouldBlock => {
            writeln!(err, "{HELD}")?;
            return Ok(crate::state::EXIT_FAILED);
        }
        Err(why) => {
            writeln!(err, "ae web: {}: {why}", dir.join("lock").display())?;
            return Ok(crate::state::EXIT_FAILED);
        }
    };
    let global = crate::doors::config_file(shape, &root);
    // The binary holds the stdout lock for the whole command, so a read
    // worker never writes the terminal: it hands its line to this thread,
    // and a full queue drops the line rather than holding up an answer.
    let (say, said) = mpsc::sync_channel::<String>(LINES_QUEUED);
    let running = server::start(Options {
        port: args.port,
        clock: Arc::new(SystemClock),
        source: Arc::new(LiveSource::new(root, global)),
        probe: Arc::new(NoProbe),
        links: Arc::new(move |line: &str| drop(say.try_send(line.to_owned()))),
        version: crate::shape::VERSION.to_owned(),
    });
    let running = match running {
        Ok(running) => running,
        Err(why) => {
            writeln!(err, "ae web: {why}")?;
            return Ok(crate::state::EXIT_FAILED);
        }
    };
    writeln!(out, "ae web: {}", running.link())?;
    writeln!(out, "{OPEN}")?;
    out.flush()?;
    // Serve until a signal ends the process: the listener keeps the sending
    // side, so this loop writes every handed line and never ends while it runs.
    for line in said {
        drop(writeln!(out, "{line}"));
        drop(out.flush());
    }
    loop {
        std::thread::park();
    }
}
