//! Bounded, status-checked tmux observations for the integration fixtures.
//!
//! A fixture reads tmux through its own callable (its private socket, scratch
//! root and process door), and that callable answers `(succeeded, stdout)`. A
//! failed read is not evidence of anything: its stdout may be empty (the pane
//! is "missing") or plausible (the pane is "there"), and neither is true. Every
//! observation here checks the status BEFORE it applies a predicate, so a
//! failed read never proves absence and never yields a pane.
//!
//! Nothing here owns a socket, a scratch root or a child: a Rig keeps its setup
//! and its lifetime, and passes this module only the closure that reaches its
//! tmux server.
//!
//! `source_view` is the other thing a guard shares: what it may be shown of the
//! source text.

pub(crate) mod source_view;
// The spec is installed byte for byte as its author froze it, so the two lints
// it trips under `-D warnings` are allowed here, at its registration, and
// nowhere else.
#[allow(
    clippy::manual_let_else,
    clippy::err_expect,
    reason = "frozen spec installed byte-identical; it spells its refusals as a `match` and \
              `.err().expect(..)`"
)]
mod source_view_spec;

mod pane_command_spec;
// Installed byte for byte too; its helper outside a `#[test]` trips these.
#[allow(
    clippy::expect_used,
    reason = "frozen spec installed byte-identical; its failure helper unwraps"
)]
mod seat_recorded_spec;

use std::fmt;
use std::io;
use std::time::{Duration, Instant};

use ae::event_text::member;

/// How long to wait between two reads while a deadline allows another.
const POLL: Duration = Duration::from_millis(50);

/// The budget of a fixture's SETUP read, for a caller that has no tighter one:
/// `Instant::now() + SETUP_WAIT`. A passing read costs one tmux call; the
/// budget only buys time for a failed read to recover (#293).
pub(crate) const SETUP_WAIT: Duration = Duration::from_secs(10);

/// The budget of a wait for a pane's own command. tmux answers a pane start
/// before the pane's child has exec'd, and on a host where that child stalls
/// the pane reads `tmux` for a second or more (#267), longer under load.
pub(crate) const PANE_WAIT: Duration = Duration::from_mins(1);

/// What the last read said when no read satisfied the predicate in time.
#[derive(Debug)]
pub(crate) struct Unmet {
    /// The exact argument slice the last read was handed.
    pub(crate) args: Vec<String>,
    /// Whether that read succeeded.
    pub(crate) ok: bool,
    /// What it printed, untrimmed.
    pub(crate) out: String,
    /// How many reads were made, at least one.
    pub(crate) reads: u32,
}

impl fmt::Display for Unmet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "tmux {:?}: no successful read satisfied the predicate before the deadline; \
             {} read(s), last status ok={}, stdout={:?}",
            self.args, self.reads, self.ok, self.out
        )
    }
}

/// Read tmux until a SUCCESSFUL read satisfies `predicate`, or `deadline`
/// passes. At least one read is made even when the deadline is already spent
/// (`Instant::now()` is one validated read and no retry). The predicate never
/// sees a failed read. Returns that read's untrimmed output.
pub(crate) fn try_observe(
    tmux: impl Fn(&[&str]) -> (bool, String),
    args: &[&str],
    deadline: Instant,
    predicate: impl Fn(&str) -> bool,
) -> Result<String, Unmet> {
    let mut reads = 0;
    loop {
        let (ok, out) = tmux(args);
        reads += 1;
        if ok && predicate(&out) {
            return Ok(out);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(Unmet {
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                ok,
                out,
                reads,
            });
        }
        std::thread::sleep(remaining.min(POLL));
    }
}

/// [`try_observe`], failing the test with the last read's status and output.
#[track_caller]
pub(crate) fn observe(
    tmux: impl Fn(&[&str]) -> (bool, String),
    args: &[&str],
    deadline: Instant,
    predicate: impl Fn(&str) -> bool,
) -> String {
    match try_observe(tmux, args, deadline, predicate) {
        Ok(out) => out,
        Err(why) => panic!("{why}"),
    }
}

/// A pane's foreground command, once a successful read shows one `accepts`
/// takes. Trimmed; `accepts` sees it trimmed.
pub(crate) fn try_pane_command(
    tmux: impl Fn(&[&str]) -> (bool, String),
    pane: &str,
    deadline: Instant,
    accepts: impl Fn(&str) -> bool,
) -> Result<String, Unmet> {
    let args = [
        "display-message",
        "-p",
        "-t",
        pane,
        "#{pane_current_command}",
    ];
    try_observe(tmux, &args, deadline, |out| accepts(out.trim())).map(|out| out.trim().to_owned())
}

/// [`try_pane_command`], failing the test with the last read's status and output.
#[track_caller]
pub(crate) fn pane_command(
    tmux: impl Fn(&[&str]) -> (bool, String),
    pane: &str,
    deadline: Instant,
    accepts: impl Fn(&str) -> bool,
) -> String {
    match try_pane_command(tmux, pane, deadline, accepts) {
        Ok(command) => command,
        Err(why) => panic!("{why}"),
    }
}

/// Whether a foreground command is a shell. The names are written out because
/// the product's classifier also calls an empty command a shell.
pub(crate) fn is_shell(command: &str) -> bool {
    matches!(command, "sh" | "bash" | "dash" | "zsh" | "fish")
}

/// Whether a pane's child has exec'd: until then the pane reads `tmux`, the
/// forked server's own name. Says nothing about which command runs now.
pub(crate) fn has_execed(command: &str) -> bool {
    !command.is_empty() && command != "tmux"
}

/// The first `list-panes` line of a session, from a successful read.
///
/// The caller owns the target spelling (`-s`, `-t`, `=name`) in
/// `selector_args`; this owns `list-panes` and the `#{pane_id}` format. An
/// empty first line is not a pane.
pub(crate) fn try_first_pane(
    tmux: impl Fn(&[&str]) -> (bool, String),
    selector_args: &[&str],
    deadline: Instant,
) -> Result<String, Unmet> {
    let args = [&["list-panes"][..], selector_args, &["-F", "#{pane_id}"]].concat();
    try_observe(tmux, &args, deadline, |out| {
        out.lines().next().is_some_and(|line| !line.is_empty())
    })
    .map(|out| out.lines().next().unwrap_or_default().to_owned())
}

/// [`try_first_pane`], failing the test with the last read's status and output.
#[track_caller]
pub(crate) fn first_pane(
    tmux: impl Fn(&[&str]) -> (bool, String),
    selector_args: &[&str],
    deadline: Instant,
) -> String {
    match try_first_pane(tmux, selector_args, deadline) {
        Ok(pane) => pane,
        Err(why) => panic!("{why}"),
    }
}

/// A fixture's witness, for a FRESH session directory, that the pane's `_run`
/// has recorded the main seat's config home and conversation choice. Launch of
/// a tool that skips the start wait returns before its pane execs, by design,
/// so a fixture that kills that seat or freezes its files proves these first.
/// `read` answers a session-relative file; one `deadline` covers both proofs.
///
/// Not a witness for a resume: an earlier incarnation's rows satisfy it. Not
/// proof of the exec: `record_launch` follows, a no-op for an unobserved model.
#[track_caller]
pub(crate) fn seat_recorded(read: impl Fn(&str) -> io::Result<String>, deadline: Instant) {
    recorded(&read, "meta", "config_home.main", deadline, |text| {
        text.lines()
            .any(|line| line.starts_with("config_home.main="))
    });
    recorded(
        &read,
        "events.jsonl",
        "conversation-new",
        deadline,
        |text| {
            text.lines().any(|line| {
                let line = line.as_bytes();
                member(line, "action").value() == Some(b"conversation-new".as_slice())
                    && member(line, "target_slot").value() == Some(b"main".as_slice())
            })
        },
    );
}

/// One proof of [`seat_recorded`]: its file, what it sought, and the reads made.
#[track_caller]
fn recorded(
    read: &impl Fn(&str) -> io::Result<String>,
    file: &str,
    sought: &str,
    deadline: Instant,
    proves: impl Fn(&str) -> bool,
) {
    let reader = |_: &[&str]| match read(file) {
        Ok(text) => (true, text),
        Err(why) => (false, why.to_string()),
    };
    if let Err(why) = try_observe(reader, &[file, sought], deadline, proves) {
        panic!(
            "{file}: no {sought} before the deadline; {} read(s), last status ok={}, last read {:?}",
            why.reads, why.ok, why.out
        );
    }
}
