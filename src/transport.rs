//! The exec: the one place this crate starts a child process.
//!
//! [`crate::tmux`] owns both halves that can be WRONG — which server an
//! argument list addresses, and what a completed run means — and both are
//! proven against real isolated servers. What is here is the part in between,
//! which is a detail rather than a decision: it hands a derived argument list to
//! `tmux`, waits, and hands the completed run back to be interpreted. It derives
//! no argv of its own and interprets no bytes of its own.
//!
//! # The door
//!
//! `clippy.toml` denies `std::process::Command` crate-wide with a small
//! enumerated set of exceptions. This is the third, and the first outside a test
//! harness. It is deliberate: a session multiplexer that cannot run tmux cannot
//! answer the question it exists to answer, and until this landed every liveness
//! answer was `unknown` by construction. The boundary keeps its value precisely
//! because crossing it stays conspicuous — one function, one file, and an
//! inventory in `tests/it/parity_self_test.rs` that goes red when the set of
//! crossings changes.
//!
//! The door is a capability, not a licence: everything a caller could want to do
//! with a child process other than run tmux — and, since P2.5a, the session's
//! own frozen `send` helper — is still denied everywhere, including here,
//! because [`spawn`] is private and its callers take argument lists nothing
//! outside this crate can supply.
//!
//! # The second program: the session's send helpers
//!
//! A tracked request (`ask`, `review`) is composed by the core and DELIVERED by
//! the frozen `send` body — the TUI-modelled paste path with its dead-pane
//! guard, provenance envelope, per-target lock, busy deferral and submit
//! verification, none of which this crate re-implements. [`deliver`] runs a
//! session helper — the internal `_send-deliver`, that body behind its
//! delivery-only entry point, or the public `send` for the no-identity
//! fallback — inherits its stderr so every loud line reaches the caller
//! verbatim, and hands back its exit status and its stdout (for
//! `_send-deliver`, the one line naming the stored body's path). The event is
//! then the core's to write, under its own locked transaction.
//!
//! # The exit status decides; the payload never does
//!
//! **SC-017k** grants `running`/`stopped` only to a SUCCESSFUL query, and
//! **SC-017l** sends every failure to `unknown`. This module is where that can
//! be lost, because it is the last place the two facts are still separable:
//! empty stdout from a live server with no sessions and empty stdout from a
//! `tmux` that could not be spawned at all are the same bytes. So the run's
//! SUCCESS leaves here beside its bytes and goes to
//! [`crate::tmux::interpret_sessions`], which fails the query on the status
//! before it looks at anything.
//!
//! A transport that answered `Ok(Vec::new())` for an unreachable server would
//! make every session recorded on it `stopped` — ae asserting sessions are gone
//! on the strength of a question it never asked, which is #105 restated one
//! layer down. A child that could not be SPAWNED (no `tmux` on `PATH`, no
//! permission to execute it) is that same failure and is reported the same way:
//! a failed run, never an empty answer.

use crate::inventory::{DiscoveredSession, Discovery, QueryFailed, ServerId};
use crate::meta::Selector;
use crate::tmux;

/// The program ae runs to talk to a tmux server.
///
/// A bare name, resolved on `PATH` by the OS at spawn time. ae does not read
/// `PATH` itself — `std::env::var` is denied in product code by the same
/// `clippy.toml` that denies `Command`, and resolving a program name is exactly
/// the job the exec already does correctly.
const PROGRAM: &str = "tmux";

/// The real tmux transport: [`Discovery`], answered by running `tmux`.
///
/// Carries nothing. Which server to ask arrives as the [`ServerId`] parameter,
/// because entitlement is decided by [`crate::inventory`] and a transport that
/// held a server of its own would be a second place that decision could live.
#[derive(Debug, Clone, Copy, Default)]
pub struct Tmux;

impl Discovery for Tmux {
    /// Every session `server` reports, each with the ownership marker that
    /// server holds for it.
    ///
    /// One `list-sessions` for the server, then one `show-environment` per name
    /// it returned. Grouping candidates so that a server is asked once is
    /// [`crate::liveness`]'s job and already done before this is called; the
    /// per-name reads are the marker half of SC-017j, which needs the session's
    /// own environment and cannot be answered by the enumeration.
    ///
    /// A marker read that fails yields no marker rather than no session: the
    /// name was returned by a query that DID succeed, so it is present, and
    /// SC-017l routes present-but-not-provably-ours to `unknown` — never to
    /// `stopped`, which would claim a session that just answered is gone.
    fn enumerate(&self, server: &ServerId) -> Result<Vec<DiscoveredSession>, QueryFailed> {
        if !addressable(server) {
            return Err(QueryFailed);
        }
        let (succeeded, stdout) = run(PROGRAM, &tmux::list_sessions_args(server));
        let names = tmux::interpret_sessions(succeeded, &stdout)?;
        Ok(names
            .into_iter()
            .map(|name| {
                let (succeeded, stdout) = run(PROGRAM, &tmux::marker_args(server, &name));
                let marker = tmux::interpret_marker(succeeded, &stdout);
                DiscoveredSession { name, marker }
            })
            .collect())
    }
}

/// Whether `server` has a session `name` — the frozen resolver's
/// `tmux has-session -t` before a cross-session lookup (prefix-matched, as
/// tmux does). A non-addressable server is `false`, never put on the wire.
///
/// The server is a PARAMETER for the same reason [`observe_agents`] takes one:
/// an `@session:agent` target may live on a server that is not the caller's
/// ambient one, so the existence probe must ask the TARGET session's recorded
/// server.
#[must_use]
pub fn session_exists(server: &ServerId, name: &str) -> bool {
    addressable(server) && run(PROGRAM, &tmux::has_session_args(server, name)).0
}

/// Whether `name` is verifiably STOPPED on its recorded `server` — the frozen
/// `_end_verify_gone` tri-state the destructive compact gate crosses.
///
/// Distinct from [`session_exists`] and [`Tmux::enumerate`] because it must read
/// tmux's stderr: a clean server exit (its last session gone) proves the session
/// is stopped, and only the `no server running on …` diagnostic distinguishes
/// that from an unreachable server. A non-addressable server is
/// [`tmux::StopProbe::Unknown`] — never put on the wire, never read as absence.
#[must_use]
pub fn verify_session_absent(server: &ServerId, name: &str) -> tmux::StopProbe {
    if !addressable(server) {
        return tmux::StopProbe::Unknown;
    }
    let (succeeded, stdout, stderr) = run_captured(PROGRAM, &tmux::list_sessions_args(server));
    tmux::interpret_stopped(succeeded, &stdout, &stderr, name)
}

/// The pane roster of `session` on `server`, or `None` when the enumeration
/// failed — see [`tmux::interpret_agents`]. A non-addressable server (a relative
/// socket) is `None` rather than put on the wire, as [`addressable`] rules.
///
/// The server is a PARAMETER because a target session need not live on the
/// caller's ambient server: a PANE-LESS caller (compact delivering a handover)
/// has no `TMUX` pointing at the recorded server, so enumerating the ambient
/// server finds nothing. The caller passes the session's recorded selector; an
/// in-pane caller's ambient IS that server, so the two agree there.
#[must_use]
pub fn observe_agents(server: &ServerId, session: &str) -> Option<Vec<tmux::ObservedAgent>> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::agents_args(server, session));
    tmux::interpret_agents(succeeded, &stdout)
}

/// The slot roster of `session` on the AMBIENT server — the frozen
/// `ae_slot_resolver`'s query — or `None` when the enumeration failed.
#[must_use]
pub fn observe_slots(server: &ServerId, session: &str) -> Option<Vec<tmux::ObservedSlot>> {
    let (succeeded, stdout) = run(PROGRAM, &tmux::slots_args(server, session));
    tmux::interpret_slots(succeeded, &stdout)
}

/// Every pane of `session` on `server`, or `None` when the enumeration failed.
///
/// SC-017q sends a failed pane query to `unknown`, so the failure must stay
/// distinguishable from "the session has no panes" — the `Option` is that
/// distinction, and collapsing it would let an unreachable server prove every
/// roster agent absent. A non-addressable server is `None` for the same reason
/// [`addressable`] gives.
#[must_use]
pub fn observe_panes(server: &ServerId, session: &str) -> Option<Vec<tmux::ObservedPane>> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::list_panes_args(server, session));
    tmux::interpret_panes(succeeded, &stdout).ok()
}

/// The branch the watchdog last published for `session`, or `None`.
///
/// The live half of SC-405g: a running session's watchdog pushes the work
/// tree's branch into a session option every cycle, so reading it costs no git
/// call at all. `None` — a failed read, a watchdog that is not running, an
/// option nobody set — leaves the caller to fall back to the work tree, which
/// is what the frozen `_session_branch` does.
#[must_use]
pub fn observe_branch(server: &ServerId, session: &str) -> Option<String> {
    if !addressable(server) {
        return None;
    }
    let args = tmux::session_option_args(server, session, tmux::BRANCH_OPTION);
    let (succeeded, stdout) = run(PROGRAM, &args);
    tmux::interpret_session_option(succeeded, &stdout)
}

/// What running the frozen `send` helper produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    /// The helper's exit code; `None` when it could not be spawned at all, or
    /// died to a signal.
    pub code: Option<i32>,
    /// Its stdout — from `_send-deliver`, the stored body's path plus `\n`.
    pub stdout: String,
}

/// Run the session's send helper at `helper` with `target` and `message`,
/// plus `envs` (the event fields the frozen body store names the recovery
/// file after). stderr is INHERITED — the helper's refusals and
/// unconfirmed-submit lines are the caller's diagnostics, verbatim; stdin is
/// null.
#[must_use]
pub fn deliver(
    helper: &std::path::Path,
    target: &str,
    message: &str,
    envs: &[(&str, &str)],
) -> Delivery {
    let args = [target.to_owned(), message.to_owned()];
    match spawn(
        &helper.display().to_string(),
        &args,
        envs,
        Streams::InheritStderr,
        None,
    ) {
        Some(output) => Delivery {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        },
        None => Delivery {
            code: None,
            stdout: String::new(),
        },
    }
}

/// The calling pane's identity readings, from the AMBIENT server.
///
/// A pane's identity readings from `server`, or `None` when the read failed. A
/// non-addressable server is `None` rather than put on the wire.
///
/// The WHO-AM-I caller passes [`ServerId::Ambient`] deliberately: a generated
/// helper is invoked inside the session it serves, and the frozen helper it
/// replaces runs a bare `tmux` for exactly this read — a pane asking who it is
/// has already been placed by tmux. A TARGET pane resolved for a pane-less
/// caller lives on the RECORDED server instead, which is why the server is a
/// parameter rather than always ambient.
#[must_use]
pub fn observe_viewer(server: &ServerId, pane: &str) -> Option<tmux::ObservedViewer> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::viewer_args(server, pane));
    tmux::interpret_viewer(succeeded, &stdout)
}

/// Whether `server` is something this transport may put on the wire.
///
/// Only sockets can fail this, and only by being relative — SC-405l's
/// `ambiguous`, which [`crate::meta`] already refuses when it parses a selector.
/// Asked again here because a relative socket addresses a DIFFERENT server
/// depending on the directory ae was invoked from, and an answer that depends on
/// the caller's working directory is not the recorded server's answer. Refusing
/// is `unknown`, which is what SC-017l says an unusable pointer is worth.
fn addressable(server: &ServerId) -> bool {
    match server {
        ServerId::Selected(Selector::Socket(path)) => tmux::is_addressable_socket(path),
        ServerId::Ambient | ServerId::Selected(Selector::Name(_)) => true,
    }
}

/// Run `program` with `args`; report whether it succeeded, and what it printed.
///
/// The two returned facts are what [`crate::tmux`]'s interpreters consume, in
/// that order of authority: the `bool` is the decision and the `String` is
/// evidence that only means anything once the `bool` has allowed it to.
///
/// **A spawn failure is `(false, String::new())`** — indistinguishable, by
/// design, from a tmux that ran and failed. Both are "no answer", and inventing
/// a distinction here would put a reason taxonomy in the one place that must not
/// have opinions.
///
/// stderr is captured and dropped. tmux writes `no server running on …` there,
/// and `ae list` must show ae's own diagnostic (SC-017o) rather than leaking the
/// text of a subprocess it chose to run. stdin is not inherited: `output()`
/// nulls it, so a child that reads cannot block a listing forever.
///
/// The stdout bytes are decoded LOSSILY, matching how
/// [`crate::inventory`] derives a durable record's name from a directory entry.
/// The two strings this crate compares are then produced the same way, so a name
/// that survives one survives the other — a stricter decode here would turn one
/// odd name on a server into `unknown` for every session on it, and a different
/// decode would be a way for two spellings of the same name to stop matching.
// THE PRODUCT'S DOOR — the only place PRODUCT code may name
// `std::process::Command`. Two others exist crate-wide, both in the test target:
// the parity harness's (`tests/it/parity.rs`, `mod raw`) and the black-box CLI
// tests' (`tests/it/cli.rs`), which must run the product binary. `clippy.toml`
// denies the TYPE everywhere else, which resolves paths rather than text and so
// holds against UFCS, aliases and re-imports alike.
// `parity_self_test::the_capability_boundary_holds_against_any_lint_relaxation`
// asks clippy under `--force-warn` for the complete list of crossings; the
// counter beside it names them by file. The first is the claim, the second is
// defence in depth — they are not the same strength.
#[allow(
    clippy::disallowed_types,
    reason = "the product's door: ae cannot answer a liveness question without running tmux, nor deliver a tracked request without the session's send helper, nor derive an archive preview's git facts without running git"
)]
fn spawn<A: AsRef<std::ffi::OsStr>>(
    program: &str,
    args: &[A],
    envs: &[(&str, &str)],
    streams: Streams,
    feed: Option<&[u8]>,
) -> Option<std::process::Output> {
    let mut command = std::process::Command::new(program);
    command.args(args);
    command.envs(envs.iter().copied());
    if streams == Streams::InheritStderr {
        command.stderr(std::process::Stdio::inherit());
    }
    if streams == Streams::Terminal {
        // Nothing is captured, so there is nothing to return but the status —
        // and an `Output` carrying it keeps every caller of this door reading
        // one shape.
        return command.status().ok().map(|status| std::process::Output {
            status,
            stdout: Vec::new(),
            stderr: Vec::new(),
        });
    }
    let Some(bytes) = feed else {
        return command.output().ok();
    };
    // THE BODY GOES IN ON STDIN, NOT IN ARGV. `tmux load-buffer -` is the one
    // call that needs it, and the reason is measured: `set-buffer -- "$msg"`
    // carried 16000 bytes and failed at 32000, `load-buffer -` carried 131000.
    //
    // Writing to a child while capturing its output can deadlock in general.
    // It cannot here: `load-buffer` prints nothing, so no pipe this end owns
    // can fill while the write is in flight, and the handle is DROPPED before
    // the wait so the child sees EOF. A write that fails (the child died) is
    // not fatal on its own — the exit status still decides, exactly as it does
    // for every other call through this door.
    command.stdin(std::process::Stdio::piped());
    command.stdout(std::process::Stdio::piped());
    let mut child = command.spawn().ok()?;
    if let Some(mut sink) = child.stdin.take() {
        let _wrote = std::io::Write::write_all(&mut sink, bytes);
    }
    child.wait_with_output().ok()
}

/// How the door wires a child's streams — and therefore what it can report.
///
/// A THIRD value rather than a second door: `clippy.toml` confines the whole
/// capability to one private function, and a sibling that also said
/// `Command::new` would be a second place the confinement has to hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Streams {
    /// Both captured. Every question-shaped call: ae reads the answer.
    Captured,
    /// stdout captured, stderr ae's own — the send helper, whose diagnostics
    /// belong in the pane that invoked it.
    InheritStderr,
    /// Every stream is ae's own. `attach-session` does not answer a question,
    /// it takes over the terminal, and a captured stdout is a terminal the
    /// client never gets.
    Terminal,
}

fn run(program: &str, args: &[String]) -> (bool, String) {
    match spawn(program, args, &[], Streams::Captured, None) {
        Some(output) => (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        ),
        // Nothing ran. Not an empty answer — see the module docs.
        None => (false, String::new()),
    }
}

/// Like [`run`], but ALSO returns the child's stderr — the one caller that needs
/// it is the stop verification, which reads tmux's `no server running on …`
/// diagnostic to tell a clean server exit (proof the session is gone) from any
/// other failure (unproven). Everywhere else stderr is noise and [`run`] drops
/// it; here it carries the SC-017l distinction.
fn run_captured(program: &str, args: &[String]) -> (bool, String, String) {
    match spawn(program, args, &[], Streams::Captured, None) {
        Some(output) => (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ),
        None => (false, String::new(), String::new()),
    }
}

/// The git leg of the one process door — the ONLY way product code runs `git`,
/// and the program is FIXED here so a caller chooses the arguments, never the
/// binary. The argv is not a raw slice but a [`crate::git::GitArgv`], whose
/// inner vector is PRIVATE to `src/git.rs`: only that module's typed, validated
/// builder can mint one, so this entry cannot be alias-imported and handed an
/// arbitrary git command line (a `use … run_git as invoke_git;` still needs a
/// `GitArgv` it cannot construct). The argv is OS-native: a work-tree path can
/// be non-UTF-8, and it rides as one argument, so there is no shell and nothing
/// to inject. Returns whether git exited zero and its stdout decoded lossily —
/// every value the preview reads back (a 40-hex sha, a decimal count) is ASCII,
/// so the lossy decode cannot change a valid answer, and an invalid one is
/// rejected anyway.
///
/// `src/git.rs` is the only caller; the type seal above is the boundary, and a
/// structural guard (`run_git_has_exactly_one_product_caller` in
/// `tests/it/parity_self_test.rs`) is defence in depth, so this fixed-program
/// leg cannot quietly become a general spawner.
pub(crate) fn run_git(argv: &crate::git::GitArgv) -> (bool, String) {
    match spawn("git", argv.as_os_args(), &[], Streams::Captured, None) {
        Some(output) => (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        ),
        None => (false, String::new()),
    }
}

/// The session's own `ae` binary leg of the one process door — the ONLY way
/// product code runs the recorded wrapper, its PATH chosen by the session record
/// rather than by a caller. Mirrors [`run_git`] and [`run_ps`]: the argv is a
/// [`crate::watchdog_glue::AeArgv`] whose inner vector is private to that
/// module, so only its fixed-shape constructors can mint one and this leg cannot
/// be alias-imported and handed an arbitrary command line.
///
/// It exists because two of the watchdog pane's duties are still bash
/// subcommands — `_recover-pending` and `telegram _supervise` — and the frozen
/// wrapper ran exactly this binary with exactly these arguments. Returns whether
/// it exited zero and its stdout decoded lossily; stderr is captured and
/// dropped, because a supervise that complains must not print into a read-only
/// watchdog pane.
pub(crate) fn run_ae(
    bin: &std::path::Path,
    argv: &crate::watchdog_glue::AeArgv,
    envs: &[(&str, &str)],
) -> (bool, String) {
    match spawn(
        &bin.display().to_string(),
        argv.as_args(),
        envs,
        Streams::Captured,
        None,
    ) {
        Some(output) => (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        ),
        None => (false, String::new()),
    }
}

/// The process-table snapshot leg of the one process door — the ONLY way
/// product code runs `ps`, the program FIXED here. Mirrors [`run_git`]: the argv
/// is a [`crate::procs::PsArgv`] whose inner vector is private to `src/procs.rs`,
/// so only that module's fixed-argv constructor can mint one, and this entry
/// cannot be alias-imported and handed an arbitrary `ps` command line. The argv
/// carries NO caller input at all (the snapshot spelling is a constant), so
/// unlike git there is nothing to inject even in principle. Returns whether `ps`
/// exited zero and its stdout decoded lossily; the watchdog's dead-check treats
/// a failed run (`false`) as UNKNOWN and never as a dead agent.
///
/// `src/procs.rs` is the only caller; the type seal is the boundary and
/// `run_ps_has_exactly_one_product_caller` in `tests/it/parity_self_test.rs` is
/// defence in depth, so this leg cannot quietly become a general spawner.
pub(crate) fn run_ps(argv: &crate::procs::PsArgv) -> (bool, String) {
    match spawn("ps", argv.as_args(), &[], Streams::Captured, None) {
        Some(output) => (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        ),
        None => (false, String::new()),
    }
}

/// The watchdog's pane roster of `session` on `server` — richer than
/// [`observe_agents`], carrying the pid and foreground command the cycle's
/// dead/stale checks read. `None` on a failed run or a non-addressable server,
/// never an empty answer for an unreachable one (the module's exit-status rule).
#[must_use]
pub fn observe_watch_panes(server: &ServerId, session: &str) -> Option<Vec<tmux::WatchPane>> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::watch_panes_args(server, session));
    tmux::interpret_watch_panes(succeeded, &stdout)
}

/// Who a pane says it belongs to — `#{session_name}` and `@ae_agent`, read from
/// the PANE ITSELF at the moment a kill is being authorised.
///
/// `None` is the REFUSAL: a failed run and a successful one that names no
/// session both mean "no owner named", and a kill may be authorised by neither.
/// The argv and the interpreter are
/// [`crate::watchdog_glue`]'s — see that module's docs for why they sit there
/// rather than beside their siblings in [`crate::tmux`].
#[must_use]
pub fn observe_pane_owner(
    server: &ServerId,
    pane: &str,
) -> Option<crate::watchdog_glue::PaneOwner> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(
        PROGRAM,
        &crate::watchdog_glue::pane_owner_args(server, pane),
    );
    crate::watchdog_glue::interpret_pane_owner(succeeded, &stdout)
}

/// Kill one pane by exact id. Returns whether tmux accepted it.
///
/// UNGUARDED on purpose: the ownership check that authorises a kill is
/// [`crate::watchdog_glue::kill_owned_pane`]'s, and putting a second copy here
/// would make two places responsible for one rule. Nothing else may call this.
#[must_use]
pub fn kill_pane(server: &ServerId, pane: &str) -> bool {
    if !addressable(server) {
        return false;
    }
    run(PROGRAM, &crate::watchdog_glue::kill_pane_args(server, pane)).0
}

/// Create a spawned agent's own window and return its pane id.
///
/// `None` is every failure the caller must roll back on: a non-addressable
/// server, a refused `new-window`, or a run that printed no id.
#[must_use]
pub fn new_window(server: &ServerId, session: &str, work_dir: &str) -> Option<String> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::new_window_args(server, session, work_dir));
    tmux::interpret_new_window(succeeded, &stdout)
}

/// Set a pane's title. Returns whether tmux accepted it — a title is
/// decoration, so every caller degrades rather than aborts.
#[must_use]
pub fn set_pane_title(server: &ServerId, pane: &str, title: &str) -> bool {
    if !addressable(server) {
        return false;
    }
    run(PROGRAM, &tmux::pane_title_args(server, pane, title)).0
}

/// Rename the window `pane` lives in. `name` MUST already be
/// [`tmux::format_literal`]-escaped: a window name is a FORMAT, and `#(…)` in
/// one runs a shell. The escape is not applied here ON PURPOSE — the sink is
/// the one place a reviewer must be able to SEE it.
#[must_use]
pub fn rename_window(server: &ServerId, pane: &str, name: &str) -> bool {
    if !addressable(server) {
        return false;
    }
    run(PROGRAM, &tmux::rename_window_args(server, pane, name)).0
}

/// The last ~40 joined lines of `pane` on `server`, or `None` when the capture
/// failed or the server is non-addressable. The bytes feed the watchdog's quiet
/// hash and throttle scan; there is nothing to interpret, so the raw stdout is
/// the answer, gated on the run having succeeded.
#[must_use]
pub fn capture_pane(server: &ServerId, pane: &str) -> Option<String> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::capture_pane_args(server, pane));
    succeeded.then_some(stdout)
}

/// The id tmux holds for the session named exactly `name` on `server`.
///
/// Every watchdog WRITE targets this id rather than the name, because `-t`
/// prefix-matches and a session whose name prefixes another's would take the
/// other's publication. `None` — a failed run, a non-addressable server, or a
/// name the server does not hold — means the caller writes NOTHING: an empty
/// target lands on tmux's CURRENT session, which belongs to somebody else.
#[must_use]
pub fn observe_session_id(server: &ServerId, name: &str) -> Option<String> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::session_ids_args(server));
    tmux::interpret_session_id(succeeded, &stdout, name)
}

/// `session`'s panes with the window each belongs to, for the per-window glyphs.
#[must_use]
pub fn observe_window_panes(server: &ServerId, session: &str) -> Option<Vec<tmux::WindowPane>> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::window_panes_args(server, session));
    tmux::interpret_window_panes(succeeded, &stdout)
}

/// Publish one user option on `target`, which must be an exact id.
///
/// The VALUE needs no escaping: a tmux user option interpolates literally, with
/// no format parsing and no `#()`. The option NAME and the target are ours.
/// Returns whether tmux accepted it — a failed publication is a stale bar, not a
/// reason to stop watching, so every caller degrades rather than aborts.
#[must_use]
pub fn publish_option(
    server: &ServerId,
    scope: tmux::OptionScope,
    target: &str,
    name: &str,
    value: &str,
) -> bool {
    if !addressable(server) {
        return false;
    }
    let (succeeded, _) = run(
        PROGRAM,
        &tmux::set_option_args(server, scope, target, name, value),
    );
    succeeded
}

/// Remove one user option from `target`. Unset, never set-to-empty — see
/// [`tmux::unset_option_args`].
#[must_use]
pub fn clear_option(server: &ServerId, scope: tmux::OptionScope, target: &str, name: &str) -> bool {
    if !addressable(server) {
        return false;
    }
    let (succeeded, _) = run(
        PROGRAM,
        &tmux::unset_option_args(server, scope, target, name),
    );
    succeeded
}

/// Show a transient message on `target`'s clients.
///
/// `text` MUST already be [`tmux::format_literal`]-escaped — `display-message`
/// reads a FORMAT, and `#(…)` in one runs a shell. This function does not escape
/// for the caller ON PURPOSE: an escape applied here would be invisible at the
/// call site, and the sink is the one place a reviewer must be able to SEE it.
#[must_use]
pub fn display_message(server: &ServerId, target: &str, text: &str) -> bool {
    if !addressable(server) {
        return false;
    }
    let (succeeded, _) = run(PROGRAM, &tmux::display_message_args(server, target, text));
    succeeded
}

/// Every session name `server` reports, or `None` when it did not answer.
///
/// The frozen `ae next --attach` re-validation: "use an exact `list-sessions`
/// match, NOT `has-session -t` — that prefix-matches, so a vanished session with
/// a surviving prefix-sibling would false-positive and land the focus on the
/// wrong session". The exact match is the caller's; this only gets the names.
#[must_use]
pub fn session_names(server: &ServerId) -> Option<Vec<String>> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::list_sessions_args(server));
    tmux::interpret_sessions(succeeded, &stdout).ok()
}

/// The session the CALLING client is in, or `None` when there is no answer.
#[must_use]
pub fn observe_current_session(server: &ServerId) -> Option<String> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::current_session_args(server));
    tmux::interpret_session_option(succeeded, &stdout)
}

/// The ttys of every pane on `server`, or `None` when it did not answer.
#[must_use]
pub fn observe_pane_ttys(server: &ServerId) -> Option<Vec<String>> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::pane_ttys_args(server));
    tmux::interpret_pane_ttys(succeeded, &stdout)
}

/// Hand this terminal to tmux and report what tmux exited with.
///
/// **The one call in this module that does not capture.** Every other door here
/// reads an answer, so it takes the child's stdout; `attach-session` does not
/// answer anything — it takes over the terminal — and a captured stdout is a
/// terminal the client never gets. `switch-client` goes the same way because it
/// is the same operation from the other side, and because a jump that printed
/// tmux's diagnostic into a variable nobody reads would fail silently.
///
/// A tmux that could not be spawned at all is [`FOCUS_FAILED`], the shell's own
/// answer for a command that did not run.
#[must_use]
pub fn focus(server: &ServerId, verb: tmux::FocusVerb, session: &str) -> u8 {
    if !addressable(server) {
        return FOCUS_FAILED;
    }
    let args = tmux::focus_args(server, verb, session);
    match spawn(PROGRAM, &args, &[], Streams::Terminal, None) {
        Some(output) => output
            .status
            .code()
            .and_then(|code| u8::try_from(code).ok())
            .unwrap_or(FOCUS_FAILED),
        None => FOCUS_FAILED,
    }
}

/// What a focus that never ran reports — `127`, the shell's command-not-found.
pub const FOCUS_FAILED: u8 = 127;

// ---------------------------------------------------------------------------
// Pane DELIVERY — the paste path's runs (B move 1).
//
// Each is one derived argument list handed to the door and one completed run
// handed back. The ones that WRITE report a bool, because tmux's exit status
// is the only honest answer to "did the server take it": a paste that could
// not be staged and a paste that vanished are the same empty screen, and the
// frozen body shipped a bug for exactly that reason (a failed `set-buffer`
// left an empty input box that the staged-check read as a successful submit).
// ---------------------------------------------------------------------------

/// What `pane` is running and under which pid, or `None` when the read failed.
///
/// A pane that cannot be read is NOT a pane running a shell: the dead-pane
/// guard fails OPEN on `None`, so an unreadable pane is delivered to rather
/// than refused.
#[must_use]
pub fn observe_pane_probe(server: &ServerId, pane: &str) -> Option<tmux::ObservedPaneProbe> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::pane_probe_args(server, pane));
    tmux::interpret_pane_probe(succeeded, &stdout)
}

/// `pane`'s visible screen, or `None` when the capture failed.
///
/// The distinction is load-bearing in BOTH directions and the two consumers
/// resolve it oppositely: an unreadable region is UNSAFE to the pre-paste
/// guard (defer, never clobber) and CLEAR to the post-submit check (never
/// duplicate an already-sent message). Collapsing `None` into an empty string
/// here would pick one of those for both.
#[must_use]
pub fn capture_screen(server: &ServerId, pane: &str, styling: tmux::Styling) -> Option<String> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::capture_screen_args(server, pane, styling));
    succeeded.then_some(stdout)
}

/// Stage `bytes` in `server`'s buffer `buffer`, on STDIN. Whether it took.
#[must_use]
pub fn load_buffer(server: &ServerId, buffer: &str, bytes: &[u8]) -> bool {
    if !addressable(server) {
        return false;
    }
    let args = tmux::load_buffer_args(server, buffer);
    spawn(PROGRAM, &args, &[], Streams::Captured, Some(bytes))
        .is_some_and(|output| output.status.success())
}

/// Paste `buffer` into `pane`, bracketed, deleting the buffer. Whether it took.
#[must_use]
pub fn paste_buffer(server: &ServerId, buffer: &str, pane: &str) -> bool {
    write_run(server, &tmux::paste_buffer_args(server, buffer, pane))
}

/// Drop a staged buffer that was never pasted. Whether it took.
#[must_use]
pub fn delete_buffer(server: &ServerId, buffer: &str) -> bool {
    write_run(server, &tmux::delete_buffer_args(server, buffer))
}

/// Send one key to `pane` WITHOUT selecting it. Whether it took.
#[must_use]
pub fn send_key(server: &ServerId, pane: &str, key: tmux::Key) -> bool {
    write_run(server, &tmux::send_keys_args(server, pane, key))
}

/// Every attached client's viewed pane and last-input epoch, or `None`.
#[must_use]
pub fn observe_clients(server: &ServerId) -> Option<Vec<tmux::ObservedClient>> {
    if !addressable(server) {
        return None;
    }
    let (succeeded, stdout) = run(PROGRAM, &tmux::list_clients_args(server));
    tmux::interpret_clients(succeeded, &stdout)
}

/// A write whose only answer is its exit status.
fn write_run(server: &ServerId, args: &[String]) -> bool {
    addressable(server) && run(PROGRAM, args).0
}

#[cfg(test)]
mod tests {
    use super::{Tmux, run};
    use crate::inventory::{Discovery, QueryFailed, ServerId};
    use crate::meta::Selector;
    use std::path::PathBuf;

    #[test]
    fn a_program_that_ran_reports_success_and_its_output() {
        // THE CONTROL, AND IT COMES FIRST. Without it a `run` that answered
        // `(false, "")` unconditionally would pass every other test in this
        // file, and the transport would report every server unreachable while
        // looking perfectly well tested.
        //
        // `/bin/echo` is required to exist rather than probed for: a skipped
        // control is a control that never ran.
        assert_eq!(
            run("/bin/echo", &["ok".to_owned()]),
            (true, "ok\n".to_owned()),
            "this suite needs /bin/echo to prove the exec can succeed at all"
        );
    }

    #[test]
    fn a_program_that_cannot_be_spawned_is_a_failed_run_and_not_an_empty_one() {
        // The bytes are the same as a successful empty query's. The BOOL is
        // what tells them apart, and it is the half SC-017l depends on: this
        // pair reaching `interpret_sessions` yields `QueryFailed`, and a
        // transport that returned `(true, "")` here would report every session
        // on the server `stopped`.
        let (succeeded, stdout) = run("ae-no-such-program-exists-anywhere", &[]);
        assert!(!succeeded, "a program that is not there did not run");
        assert!(stdout.is_empty());
        assert_eq!(
            crate::tmux::interpret_sessions(succeeded, &stdout),
            Err(QueryFailed),
            "and the pair is a FAILED query rather than a server with no sessions"
        );
    }

    #[test]
    fn a_child_that_ran_and_failed_is_a_failed_run_though_its_output_reads_like_an_answer() {
        // THE SHAPE THIS SLICE EXISTS TO KILL, and the one the two arms beside
        // it cannot reach. They cover Ok + success, and Err + never-spawned.
        // NEITHER covers Ok + `success() == false` — a child that RAN TO
        // COMPLETION and exited non-zero — which is precisely what
        // `tmux list-sessions` does against a server that is not there.
        //
        // A mutant mapping every completed wait to `(true, stdout)` passes both
        // of the others, and integration only catches it on a machine where
        // tmux happens to be installed. So it is pinned here, at the unit
        // level, where nothing external decides whether the arm runs.
        //
        // The stdout is deliberately NON-EMPTY and plausible. An empty payload
        // would let this pass for the wrong reason — the temptation is not
        // ignoring a status beside no bytes, it is ignoring a status beside
        // bytes that look like an answer. Fed to `interpret_sessions` with the
        // status dropped, "plausible" becomes a SESSION NAMED `plausible`.
        let (succeeded, stdout) = run(
            "/bin/sh",
            &["-c".to_owned(), "echo plausible; exit 1".to_owned()],
        );
        assert!(
            !succeeded,
            "a child that completed with a non-zero status did not succeed"
        );
        assert_eq!(
            stdout, "plausible\n",
            "and its bytes ARE here, which is exactly what makes dropping the status tempting"
        );
        assert_eq!(
            crate::tmux::interpret_sessions(succeeded, &stdout),
            Err(QueryFailed),
            "the status decides; output that reads like an answer does not"
        );
    }

    #[test]
    fn a_child_killed_by_a_signal_is_a_failed_run_although_it_has_no_exit_code_at_all() {
        // THE ARM NEITHER OF THE OTHERS REACHES, and it was found by the
        // reviewer attacking cold rather than by my own mutation list — which
        // is the argument for cold attacks in one sentence.
        //
        // A signalled child has NO exit code: `status.code()` is `None`. The
        // natural spelling of this check — "is the code zero?" — has to decide
        // what `None` means, and `code().map_or(true, |c| c == 0)` reads it as
        // SUCCESS. That mutant leaves the entire suite green: the exit-1 arm
        // beside this one still HAS a code, so it cannot see the difference.
        //
        // What it would cost in production: a tmux killed mid-query returns
        // empty stdout and "success", which is `Ok(vec![])` — a successful
        // query proving every name absent. Every session on that server goes
        // `stopped`. That is #105 arriving through a door none of my own
        // attacks knocked on.
        //
        // `success()` is the spelling that is right for free, because it asks
        // whether the child SUCCEEDED rather than what number it returned.
        // This arm exists to keep it that way.
        let (succeeded, stdout) = run("/bin/sh", &["-c".to_owned(), "kill -TERM $$".to_owned()]);
        assert!(
            !succeeded,
            "a child that died on a signal did not succeed, though it reported no code"
        );
        assert!(stdout.is_empty(), "and it printed nothing before dying");
        assert_eq!(
            crate::tmux::interpret_sessions(succeeded, &stdout),
            Err(QueryFailed),
            "empty output from a killed child is not a server with no sessions"
        );
    }

    #[test]
    fn a_relative_socket_is_refused_rather_than_put_on_the_wire() {
        // WHAT THIS PINS IS THE OUTCOME, NOT THE REFUSAL. `tmux -S rel/x` would
        // also fail, so this test cannot distinguish the guard from tmux's own
        // answer, and it does not claim to. The guard exists for the case the
        // test cannot stage: a relative path that DOES resolve, to a different
        // server for every directory ae is invoked from.
        let relative = ServerId::Selected(Selector::Socket(PathBuf::from("relative/ae.sock")));
        assert_eq!(Tmux.enumerate(&relative), Err(QueryFailed));
    }
}
