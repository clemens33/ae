//! `ae web`: the orchestrator chat in a browser tab on this machine. A
//! foreground listener on 127.0.0.1 only; Ctrl-C stops it, and its key, codes
//! and sessions die with it. One `ae web` per state root.

pub mod assets;
pub mod auth;
pub mod chat;
pub mod choice;
pub mod claim;
pub mod clock;
pub mod http;
pub mod nonce;
pub mod send;
pub mod server;
pub mod state;
pub mod topics;

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::io::Write;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use self::clock::SystemClock;
use self::send::Sends;
use self::server::{Followed, NoProbe, Options, Running};
use self::state::LiveSource;

/// The port `ae web` listens on without `--port`.
pub const DEFAULT_PORT: u16 = 7480;

const USAGE: &str = "usage: ae web [--port N]   (N in 1..=65535; default 7480)\n";
const HELD: &str = "ae web: another ae web runs for this state root";
const OPEN: &str = "ae web: open it in a browser on this machine; Ctrl-C stops it";
/// The terminal lines a worker may have waiting for the main thread.
const LINES_QUEUED: usize = 8;
/// How often `ae web` looks at the command link for another core.
const FOLLOW_EVERY: Duration = Duration::from_secs(5);
/// How long a follow waits for the sends in flight before it looks again later.
const FOLLOW_WAIT: Duration = Duration::from_secs(30);

/// A line for the terminal `ae web` runs in.
enum Line {
    /// For stdout: a fresh link.
    Out(String),
    /// For stderr: why the listener did not follow a new core.
    Err(String),
}

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
/// link, then serve until the process is stopped. A send carried to a busy
/// seat defers after `defer`, the helpers' own setting.
///
/// # Errors
///
/// [`crate::error::Error::Io`] when `out` or `err` cannot be written.
pub fn run(
    tail: &[String],
    defer: Duration,
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
    let (say, said) = mpsc::sync_channel::<Line>(LINES_QUEUED);
    let links = say.clone();
    let running = server::start_with(
        Options {
            port: args.port,
            clock: Arc::new(SystemClock),
            source: Arc::new(LiveSource::new(root.clone(), global)),
            probe: Arc::new(NoProbe),
            links: Arc::new(move |line: &str| drop(links.try_send(Line::Out(line.to_owned())))),
            version: crate::shape::VERSION.to_owned(),
        },
        Sends::live_with(root, defer),
    );
    let running = match running {
        Ok(running) => Arc::new(running),
        Err(why) => {
            writeln!(err, "ae web: {why}")?;
            return Ok(crate::state::EXIT_FAILED);
        }
    };
    // The first look at the command link, taken before the startup line so the
    // line witnesses it: the version the link names now is never followed.
    let first = first_look(shape, linked_version);
    writeln!(out, "ae web: {}", running.link())?;
    writeln!(out, "{OPEN}")?;
    out.flush()?;
    let following = Arc::clone(&running);
    let spawned = std::thread::Builder::new()
        .name("ae-web-follow".to_owned())
        .spawn(move || follow(&following, &say, first));
    if let Err(why) = spawned {
        writeln!(err, "ae web: cannot follow new versions: {why}")?;
    }
    // Serve until a signal ends the process: the listener keeps the sending
    // side, so this loop writes every handed line and never ends while it runs.
    for line in said {
        match line {
            Line::Out(line) => {
                drop(writeln!(out, "{line}"));
                drop(out.flush());
            }
            Line::Err(line) => drop(writeln!(err, "{line}")),
        }
    }
    loop {
        std::thread::park();
    }
}

/// Follow the command link as `ae app` does: when it names another core and
/// automatic upgrades are on, replace this listener with that core once no
/// send is in flight. A version the link named when this listener started is
/// never followed: `first` is that look, which `run` takes before it prints
/// the startup link. A failed exec is told once per version.
fn follow(running: &Running, say: &mpsc::SyncSender<Line>, first: Option<String>) {
    let shape = crate::shape::current();
    let crate::shape::Shape::Installed { version, .. } = shape else {
        return;
    };
    let home = crate::doors::home();
    let paths = || {
        let home = home.as_deref().ok_or("HOME is not set")?;
        crate::install::fixed_paths(home)
    };
    let other = |linked: &str| {
        matches!(
            crate::install::compare_versions(linked, version),
            Some(Ordering::Less | Ordering::Greater)
        )
    };
    let unlinked = first.filter(|first| other(first));
    let mut told = BTreeSet::new();
    loop {
        std::thread::sleep(FOLLOW_EVERY);
        let Ok(now) = linked_version() else { continue };
        let automatic = || crate::autoupgrade::automatic(shape, crate::doors::no_autostart());
        if !due(other(&now), &now, unlinked.as_deref(), &told, automatic) {
            continue;
        }
        let Some(link) = paths()
            .ok()
            .and_then(|paths| paths.link.to_str().map(str::to_owned))
        else {
            continue;
        };
        let port = running.addr().port().to_string();
        let argv = [link, "web".to_owned(), "--port".to_owned(), port];
        if let Followed::Failed(why) = running.follow(&argv, FOLLOW_WAIT) {
            let words =
                format!("ae web: cannot follow version {now}: {why}; still serving {version}");
            drop(say.try_send(Line::Err(words)));
            told.insert(now);
        }
    }
}

/// The first look at the command link for a listener of `shape`: only an
/// installed core follows the link, so only it reads the link through `look`.
/// A checkout or displaced core reads nothing.
fn first_look(
    shape: &crate::shape::Shape,
    look: impl FnOnce() -> Result<String, String>,
) -> Option<String> {
    match shape {
        crate::shape::Shape::Installed { .. } => look().ok(),
        crate::shape::Shape::Displaced { .. } | crate::shape::Shape::Checkout => None,
    }
}

/// The version the command link names, the link spelled from `$HOME` as a
/// publish spells it, or why it cannot be read.
fn linked_version() -> Result<String, String> {
    let home = crate::doors::home();
    let home = home.as_deref().ok_or("HOME is not set")?;
    crate::install::current_public_version(&crate::install::fixed_paths(home)?)
}

/// Whether the watcher follows `now`, the version the command link names: it
/// names another core than this one (`other`), not the version it named when
/// this listener started (`unlinked`), not one whose follow already failed and
/// was told (`told`), and automatic upgrades are on. `automatic` reads the
/// config, so it is read only when every other condition holds.
fn due(
    other: bool,
    now: &str,
    unlinked: Option<&str>,
    told: &BTreeSet<String>,
    automatic: impl FnOnce() -> bool,
) -> bool {
    if !other || unlinked == Some(now) || told.contains(now) {
        return false;
    }
    if !automatic() {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use super::{due, first_look};
    use crate::shape::Shape;

    const NOW: &str = "2026.11.200";

    /// `due` for `now` = [`NOW`]: whether a follow is due, and whether the
    /// config was read.
    fn check(other: bool, unlinked: Option<&str>, told: &[&str], automatic: bool) -> (bool, bool) {
        let told: BTreeSet<String> = told.iter().map(|version| (*version).to_owned()).collect();
        let read = Cell::new(false);
        let got = due(other, NOW, unlinked, &told, || {
            read.set(true);
            automatic
        });
        (got, read.get())
    }

    /// docs/web.md and `follow`: the link is followed when it names another
    /// core, not the version it named at the first look, not a version already
    /// told as failed, and automatic upgrades are on; the config is read last.
    #[test]
    fn a_follow_is_due_only_for_another_untold_version_with_automatic_upgrades_on() {
        let skipped = (false, false);
        assert_eq!(
            check(false, None, &[], true),
            skipped,
            "this core's own version"
        );
        assert_eq!(
            check(true, Some(NOW), &[], true),
            skipped,
            "named at the first look"
        );
        assert_eq!(check(true, None, &[NOW], true), skipped, "already told");
        assert_eq!(
            check(true, None, &[], false),
            (false, true),
            "automatic upgrades off"
        );
        assert_eq!(
            check(true, None, &[], true),
            (true, true),
            "another version"
        );
        assert_eq!(
            check(true, Some("2026.11.199"), &["2026.11.198"], true),
            (true, true),
            "another than the first look and the told ones"
        );
    }

    /// `first_look` for `shape` when the link reads `linked`: the version it
    /// keeps, and whether the link was read.
    fn look(shape: &Shape, linked: Result<&str, &str>) -> (Option<String>, bool) {
        let read = Cell::new(false);
        let got = first_look(shape, || {
            read.set(true);
            linked.map(str::to_owned).map_err(str::to_owned)
        });
        (got, read.get())
    }

    /// `follow` and docs/web.md: only an installed listener follows the link,
    /// so only it takes the first look; a checkout or displaced listener reads
    /// no install path before it prints the startup link.
    #[test]
    fn only_an_installed_listener_takes_the_first_link_look() {
        let installed = Shape::Installed {
            home: PathBuf::from("/home/u/.ae"),
            version_dir: PathBuf::from("/home/u/.ae/versions/2026.11.1"),
            version: "2026.11.1".to_owned(),
        };
        assert_eq!(
            look(&installed, Ok(NOW)),
            (Some(NOW.to_owned()), true),
            "an installed listener keeps the linked version"
        );
        assert_eq!(
            look(&installed, Err("no link")),
            (None, true),
            "an unreadable link names no version"
        );
        let displaced = Shape::Displaced {
            home: PathBuf::from("/opt/root/.ae"),
            declared: PathBuf::from("/home/u"),
        };
        assert_eq!(look(&displaced, Ok(NOW)), (None, false), "displaced");
        assert_eq!(look(&Shape::Checkout, Ok(NOW)), (None, false), "checkout");
    }
}
