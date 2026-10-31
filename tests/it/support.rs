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

use std::fmt;
use std::time::{Duration, Instant};

/// How long to wait between two reads while a deadline allows another.
const POLL: Duration = Duration::from_millis(50);

/// The budget of a fixture's SETUP read, for a caller that has no tighter one:
/// `Instant::now() + SETUP_WAIT`. A passing read costs one tmux call; the
/// budget only buys time for a failed read to recover (#293).
pub(crate) const SETUP_WAIT: Duration = Duration::from_secs(10);

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
