//! `ae` — agent environment: a tmux-backed multi-agent session multiplexer.
//!
//! Module layout is 2018-edition style: `cli.rs` beside a future `cli/`, never
//! a `mod.rs`.
//!
//! ```
//! let request = ae::cli::Request::parse(&["--version".to_owned()]);
//! assert_eq!(request, ae::cli::Request::Version);
//! assert_eq!(request.exit_code(), Some(0));
//! ```

pub mod archive;
pub mod attention;
pub mod autoupgrade;
pub mod brief;
pub mod cli;
mod compact;
pub mod config;
pub mod deliver;
pub mod digest;
pub mod doctor;
pub mod doors;
pub mod entry;
pub mod error;
pub mod event_text;
pub mod events;
pub mod events_tail;
pub mod filters;
pub mod git;
pub mod goal;
pub mod harness_state;
pub mod identity;
pub mod init;
pub mod install;
pub mod interrupt;
pub mod inventory;
pub mod json;
pub mod launch;
pub mod launch_cmd;
/// The whole `end`/`stop`/`compact` operations, in one place.
pub mod lifecycle;
pub mod listing;
pub mod liveness;
pub mod memo;
pub mod meta;
pub mod migrate;
pub mod monitor;
pub mod netprobe;
pub mod next;
pub mod orchestrator;
pub mod overview;
pub mod panes;
pub mod procs;
pub mod quota;
pub mod relay;
pub mod rename;
pub mod render;
pub mod reply;
pub mod requests;
pub mod roster;
pub mod run;
pub mod send;
pub mod session;
pub mod session_launch;
pub mod session_menu;
mod session_tmux;
pub mod shape;
pub mod shim;
pub mod spawn;
pub mod state;
pub mod store;
pub mod teardown;
pub mod telegram;
pub mod telegram_lifecycle;
pub mod theme;
pub mod time;
pub mod tmux;
pub mod tmux_floor;
pub mod tool;
pub mod tracked;
pub mod transport;
pub mod upgrade;
pub mod usage;
pub mod watchdog;
pub mod watchdog_daemon;
pub mod watchdog_glue;
pub mod watchdog_lifecycle;
pub mod words;

use std::io::Write;

pub use error::{Error, Result};

/// The crate version, as recorded in `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The line `ae --version` prints.
///
/// ```
/// assert!(ae::version_line().starts_with("ae "));
/// ```
#[must_use]
pub fn version_line() -> String {
    format!("ae {VERSION}")
}

/// The read-only automatic-upgrade rows shared by every version entry.
fn write_autoupgrade_status(out: &mut impl Write) -> Result<()> {
    let status = autoupgrade::status(shape::current());
    writeln!(out, "auto-upgrade: {}", status.policy.detail)?;
    writeln!(out, "upgrade-check: {}", status.check.detail)?;
    Ok(())
}

/// The text `ae --help` prints.
#[must_use]
pub fn help_text() -> String {
    format!(
        "{}\n\n\
         Usage: ae [OPTIONS]\n       \
         ae <COMMAND> [OPTIONS]\n\n\
         Commands:\n  \
         list, ls       List ae sessions (--json for the machine-readable digest)\n\n\
         quota          Show local cached quota windows for configured profiles\n\n\
         usage          Show API-equivalent list-price usage for live sessions\n\n\
         Internal commands (a session's own helpers call these):\n  \
         {} <dir> [mine|inbox|all]\n                 \
         Request state from a session's event log\n  \
         {} <dir>\n                 \
         Follow a session's event log\n\n\
         Options:\n  \
         -h, --help     Print help\n  \
         -V, --version  Print version and local status\n",
        version_line(),
        cli::REQUESTS,
        cli::EVENTS_TAIL
    )
}

/// What the binary says when it cannot derive its own state root.
pub const NO_STATE_ROOT: &str = "cannot derive the state root: neither AE_HOME nor HOME is set";

/// What the binary says for a top-level session name.
pub const NO_LAUNCHER: &str = "start is not implemented in this build";

/// The exit code for a request the binary understood but could not carry out.
pub const EXIT_UNAVAILABLE: u8 = 1;

/// Run the CLI against a whole argv, `argv[0]` included — the binary's real
/// entry.
///
/// # Errors
///
/// Returns [`Error::Io`] if `out` or `err` cannot be written or flushed.
pub fn run_program(
    program: Option<&str>,
    args: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let Some(program) = program else {
        return run(args, out, err);
    };
    match shim::classify(program, &invocation_dir()) {
        shim::Invocation::Core => run(args, out, err),
        shim::Invocation::Bare(name) => {
            writeln!(err, "{}", shim::bare_refusal(name))?;
            err.flush()?;
            Ok(entry::EXIT_USAGE)
        }
        // A helper carries no preamble: the pane execs the link directly, so
        // the translated argv goes straight to the ordinary dispatch — through
        // the gate, which is the ONE thing a helper still pays.
        shim::Invocation::Helper { helper, dir } => {
            if let Some(code) = install_gate(err)? {
                return Ok(code);
            }
            run_dispatch(&shim::translate(helper, &dir, args), out, err)
        }
    }
}

/// The directory a relative `argv[0]` is resolved against.
fn invocation_dir() -> std::path::PathBuf {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: a relative argv[0] means nothing without the working directory it was typed in"
    )]
    let cwd = std::env::current_dir();
    cwd.unwrap_or_else(|_| std::path::PathBuf::from("."))
}

/// Run the CLI against `args` (argv WITHOUT the program name).
///
/// # Errors
///
/// Returns [`Error::Io`] if `out` or `err` cannot be written or flushed.
///
/// ```
/// let (mut out, mut err) = (Vec::new(), Vec::new());
/// let code = ae::run(&["--version".to_owned()], &mut out, &mut err)?;
/// assert_eq!(code, 0);
/// // Four lines: core version, tmux floor, auto-upgrade policy, last check.
/// let text = String::from_utf8(out).unwrap();
/// assert!(text.starts_with(&(ae::version_line() + "\n")));
/// assert!(text.lines().nth(1).unwrap_or_default().starts_with("tmux "));
/// assert!(text.lines().nth(2).unwrap_or_default().starts_with("auto-upgrade: "));
/// assert!(text.lines().nth(3).unwrap_or_default().starts_with("upgrade-check: "));
/// assert!(err.is_empty());
/// # Ok::<(), ae::Error>(())
/// ```
pub fn run(args: &[String], out: &mut impl Write, err: &mut impl Write) -> Result<u8> {
    // TWO WORDS ARE OWED AN ANSWER ON A BROKEN INSTALL, and they are the ONLY
    // two, which is why they sit ahead of the gate: `version` is how a mismatch
    // is diagnosed, so it may not depend on the thing it diagnoses, and
    // `upgrade` is how one is repaired.
    match args.first().map(String::as_str) {
        Some("version" | "--version" | "-V") => {
            writeln!(out, "{}", version_line())?;
            // The SECOND line, on a broken install too: which tmux this machine
            // HAS, and whether the floor admits it.
            //
            // The SAME tmux a launch would use, asked the same way: the running
            // server on the declared socket first, and the `PATH` binary only
            // when nothing answers there. Reading the pair is two env reads
            // through their own door, which needs no install — and a line that
            // reported `PATH` alone would say `ok` on a machine whose declared
            // server is old and whose launches therefore refuse, which is the
            // one case this line exists for. An UNTYPABLE pair names no server
            // to ask, so the binary is all there is to report.
            let probe = match crate::doors::probe_target(
                crate::doors::declared_server(crate::shape::current()).as_ref(),
            ) {
                Some(server) => transport::observe_tmux_floor(&server),
                None => match transport::observe_tmux_program_version() {
                    Some(found) => tmux_floor::Probe::Executable(found),
                    None => tmux_floor::Probe::Silent,
                },
            };
            writeln!(out, "{}", tmux_floor::summary(&probe))?;
            write_autoupgrade_status(out)?;
            out.flush()?;
            return Ok(0);
        }
        Some("upgrade") => return upgrade::run(&args[1..], out, err),
        _ => {}
    }
    // THE GATE, above every remaining word.
    if let Some(code) = install_gate(err)? {
        return Ok(code);
    }
    // The core's OWN namespace.
    if let Some(word) = args.first().map(String::as_str)
        && word.starts_with('_')
    {
        if !cli::serves(word) {
            writeln!(err, "ae: unknown internal command '{word}'.")?;
            err.flush()?;
            return Ok(entry::EXIT_USAGE);
        }
        return run_dispatch(args, out, err);
    }
    let shape = shape::current();
    // ONE aggregated notice, and only on the public path: an agent's `send`
    // would otherwise turn one stale export into a line of noise in every pane.
    if let Some(line) = doors::notice(&doors::ignored(shape)) {
        writeln!(err, "{line}")?;
    }
    // `init` discovers executables by reading PATH and must run NO child. The
    // ordinary preamble asks tmux to resolve a launch server, so init routes
    // before that preamble exists and reads only its state/config doors.
    if args.first().map(String::as_str) == Some(cli::INIT) {
        let Some(root) = doors::state_root(shape) else {
            writeln!(err, "ae: {NO_STATE_ROOT}")?;
            err.flush()?;
            return Ok(EXIT_UNAVAILABLE);
        };
        let path = doors::config_file(shape, &root);
        let code = init::run(&path, &args[1..], out, err)?;
        out.flush()?;
        err.flush()?;
        return Ok(code);
    }
    // Quota reads only selected config and bounded vendor caches. Route it
    // before the ordinary preamble, whose tmux facts would violate that
    // observational boundary and make a missing server block the report.
    if args.first().map(String::as_str) == Some("quota") {
        let code = run_public_quota(shape, &args[1..], out, err)?;
        out.flush()?;
        err.flush()?;
        return Ok(code);
    }
    // Usage shares quota's observational boundary: the command classifies
    // sessions once, then hands only live durable roots to the pure reader.
    if args.first().map(String::as_str) == Some("usage") {
        let code = run_public_usage(shape, &args[1..], out, err)?;
        out.flush()?;
        err.flush()?;
        return Ok(code);
    }
    let Some(preamble) = resolve_facts(shape, err)? else {
        err.flush()?;
        return Ok(EXIT_UNAVAILABLE);
    };
    run_entry(&preamble, args, out, err)
}

fn run_public_quota(
    shape: &shape::Shape,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    if let Some(extra) = tail.first() {
        writeln!(err, "ae quota: unknown argument: {extra}")?;
        return Ok(entry::EXIT_USAGE);
    }
    let Some(root) = doors::state_root(shape) else {
        writeln!(err, "ae: {NO_STATE_ROOT}")?;
        return Ok(EXIT_UNAVAILABLE);
    };
    let cwd = doors::cwd();
    let global = doors::config_file(shape, &root);
    let local = doors::local_config(&cwd);
    let home = doors::home();
    let roots = inventory::Roots::under(&root);
    quota::run(
        &quota::Inputs {
            home: home.as_deref(),
            global: Some(&global),
            local: local.as_deref(),
            sessions: Some(roots.sessions()),
            now: time::Timestamp::now().epoch(),
        },
        out,
        err,
    )
}

fn run_public_usage(
    shape: &shape::Shape,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let parsed = match usage::parse_args(tail) {
        Ok(parsed) => parsed,
        Err(extra) => {
            writeln!(err, "ae usage: unknown argument: {extra}")?;
            return Ok(entry::EXIT_USAGE);
        }
    };
    let Some(root) = doors::state_root(shape) else {
        writeln!(err, "ae: {NO_STATE_ROOT}")?;
        return Ok(EXIT_UNAVAILABLE);
    };
    let roots = inventory::Roots::under(&root);
    let (snapshot, _) = current_world(&root);
    let live = snapshot
        .sessions
        .iter()
        .filter(|session| session.status == digest::Status::Running)
        .map(|session| usage::SessionInput {
            name: session.candidate.name.clone(),
            path: session.candidate.durable.as_ref().map_or_else(
                || roots.sessions().join(&session.candidate.name),
                |record| record.path.clone(),
            ),
        })
        .collect::<Vec<_>>();
    let sessions = match usage::select_sessions(&live, &parsed.sessions) {
        Ok(sessions) => sessions,
        Err(name) => {
            writeln!(err, "ae usage: live session not found: {name}")?;
            return Ok(EXIT_UNAVAILABLE);
        }
    };
    let cwd = doors::cwd();
    let global = doors::config_file(shape, &root);
    let local = doors::local_config(&cwd);
    let prices = match usage::prices::read(Some(&global), local.as_deref()) {
        Ok(prices) => prices,
        Err(error) => {
            writeln!(err, "{error}")?;
            return Ok(error.exit_code());
        }
    };
    let home = doors::home();
    usage::run(
        &usage::Inputs {
            home: home.as_deref(),
            sessions: &sessions,
            prices: &prices,
            now: time::Timestamp::now().epoch(),
        },
        parsed.json,
        out,
    )
}

/// The structural install gate — the ONE place every effectful invocation
/// proves this binary is the one `install` published.
fn install_gate(err: &mut impl Write) -> Result<Option<u8>> {
    match shape::current() {
        shape::Shape::Installed {
            version_dir,
            version,
            ..
        } => {
            if let Err(broken) = shape::validate(&shape::OnDisk(version_dir), version, VERSION) {
                writeln!(err, "{broken}")?;
                err.flush()?;
                return Ok(Some(entry::EXIT_USAGE));
            }
        }
        shape::Shape::Displaced { home, declared } => {
            writeln!(err, "{}", shape::displaced_refusal(home, declared))?;
            err.flush()?;
            return Ok(Some(entry::EXIT_USAGE));
        }
        // A checkout has no published directory to vouch for.
        shape::Shape::Checkout => {}
    }
    Ok(None)
}

/// Every ambient fact this invocation carries, read from the doors.
fn resolve_facts(shape: &shape::Shape, err: &mut impl Write) -> Result<Option<entry::Preamble>> {
    let Some(home) = doors::state_root(shape) else {
        // [`NO_STATE_ROOT`] and its code, VERBATIM: the condition the dispatch
        // already refuses, said one layer earlier because the entry cannot
        // build a single fact without it.
        writeln!(err, "ae: {NO_STATE_ROOT}")?;
        return Ok(None);
    };
    let cwd = doors::cwd();
    let declared = doors::declared_server(shape);
    let (server_kind, server_value) = doors::resolve_launch_server(declared.as_ref());
    let launch_target = doors::launch_target(declared.as_ref());
    let caller_server = doors::caller_server();
    let inside_tmux = doors::inside_tmux(caller_server.as_ref(), err)?;
    Ok(Some(entry::Preamble {
        global: Some(doors::config_file(shape, &home)),
        local: doors::local_config(&cwd),
        home,
        cwd,
        server_kind,
        server_value,
        launch_target,
        caller_server,
        inside_tmux,
        attach: true,
        no_autostart: doors::no_autostart(),
    }))
}

/// The ordinary argv dispatch: [`cli::Request::parse`] and the world it needs.
fn run_dispatch(args: &[String], out: &mut impl Write, err: &mut impl Write) -> Result<u8> {
    let request = cli::Request::parse(args);
    // Only a listing needs a source, and `next` only needs one once its argv has
    // been accepted: a refused word must not pay for a tmux scan of every
    // session before it can say so, which is what frozen's parse-then-scan order
    // already guaranteed.
    let popup = matches!(
        &request,
        cli::Request::Orchestrator { tail }
            if orchestrator::parse(tail).is_ok_and(|args| args.popup)
    );
    if schedules_automatic_upgrade(&request) {
        autoupgrade::schedule();
    }
    // The popup is deliberately dispatched BEFORE the world edge: its latency
    // contract is three live tmux listings, never the durable inventory, event
    // journals, git probes or liveness model behind `current_world`.
    if popup && let cli::Request::Orchestrator { tail } = &request {
        return run_orchestrator(tail, err);
    }
    let wants_world = request_needs_world(&request);
    if wants_world && let Some(root) = state_root() {
        let (_, world) = current_world(&root);
        return run_with(args, Some(&world), out, err);
    }
    run_with(args, None, out, err)
}

/// Whether a parsed request needs the durable world behind `ae list`.
fn request_needs_world(request: &cli::Request) -> bool {
    match request {
        // The sweep reads the same world `list` renders — that IS its input.
        cli::Request::List(_) | cli::Request::Monitor { .. } => true,
        cli::Request::Next { tail } => next::parse(tail).is_ok(),
        // A brief is a reading of the same world, so a refused argv must not pay
        // for the scan either.
        cli::Request::Brief { tail } => brief::parse(tail).is_ok(),
        _ => false,
    }
}

/// Commands whose fully accepted grammar proves ordinary interactive use.
/// Launch/reattach has its own edge after its deeper name/config validation.
fn schedules_automatic_upgrade(request: &cli::Request) -> bool {
    match request {
        cli::Request::List(_) => true,
        cli::Request::Brief { tail } => brief::parse(tail).is_ok(),
        cli::Request::Orchestrator { tail } => {
            orchestrator::parse(tail).is_ok_and(|args| args.popup)
        }
        _ => false,
    }
}

/// `ae orchestrator --popup` — gate the tmux version, then hand tmux the menu.
#[allow(
    clippy::too_many_lines,
    reason = "one ordered read-budget-mark-draw command boundary"
)]
fn run_orchestrator(tail: &[String], err: &mut impl Write) -> Result<u8> {
    let args = match orchestrator::parse(tail) {
        Ok(args) => args,
        Err(usage) => {
            write!(err, "{}", usage.render())?;
            err.flush()?;
            return Ok(usage.code());
        }
    };
    // The server whose client invoked ae. A launch override names a
    // destination, never the `%pane` on which this menu must be drawn.
    let Some(server) = doors::caller_server() else {
        writeln!(
            err,
            "ae orchestrator: no calling tmux server, so ae cannot draw a client menu."
        )?;
        err.flush()?;
        return Ok(EXIT_UNAVAILABLE);
    };
    // Only the SERVER is asked here: a menu needs a client to be drawn on, so a
    // `PATH` binary that would clear the floor answers a question the picker
    // never asks.
    let probe = match transport::probe_tmux_version(&server) {
        tmux::VersionProbe::Answered(found) => tmux_floor::Probe::Server(found),
        tmux::VersionProbe::NoServer => tmux_floor::Probe::Silent,
        tmux::VersionProbe::Unreachable => tmux_floor::Probe::Unreachable,
    };
    if !probe.clears_floor() {
        write!(
            err,
            "{}",
            tmux_floor::refusal("orchestrator", &probe, &server)
        )?;
        err.flush()?;
        return Ok(tmux_floor::EXIT_REFUSED);
    }
    let client = args.client.as_deref();
    // Resolve the CLIENT'S session rather than the mouse target's session.
    // A status binding may be evaluated with another session as `{mouse}`;
    // only this explicit client row says which bottom-left button was used.
    let Some(client_name) = client else {
        writeln!(
            err,
            "ae orchestrator: no explicit client snapshot, so ae cannot budget the picker."
        )?;
        err.flush()?;
        return Ok(EXIT_UNAVAILABLE);
    };
    let Some(client_snapshot) = transport::observe_picker_client_session(&server, client_name)
    else {
        writeln!(
            err,
            "ae orchestrator: tmux did not resolve client {client_name:?} to one live session with dimensions (it may have vanished)."
        )?;
        err.flush()?;
        return Ok(EXIT_UNAVAILABLE);
    };
    let opened_session = Some(client_snapshot.session_id.clone());
    // A FAILED listing is not an empty fleet. The server just cleared the
    // version probe, so losing its identity snapshot is a refusal rather than
    // a confident "no sessions" menu.
    let Some(sessions) = transport::observe_picker_sessions(&server) else {
        writeln!(
            err,
            "ae orchestrator: {} did not list its sessions, so ae cannot build the picker.",
            tmux_floor::server_label(&server)
        )?;
        err.flush()?;
        return Ok(EXIT_UNAVAILABLE);
    };
    // A missing membership snapshot removes only the lead hint: every row can
    // still make the rename-safe switch to the captured session id.
    let panes = transport::observe_picker_panes(&server).unwrap_or_default();
    // The picker draws in the calling session's own look, so a session running
    // the ASCII fallback gets an ASCII menu and a themed one gets its palette.
    // A look ae could not read draws the picker in the default one: a menu is
    // a transient surface that writes nothing, so a wrong palette on it costs
    // one keystroke rather than a session's appearance.
    let look = picker_look(&server, opened_session.as_deref());
    let menu = match orchestrator::menu_for_client_session_in(
        &sessions,
        &panes,
        look.icons,
        &look.palette,
        client,
        opened_session.as_deref(),
        orchestrator::PickerBounds {
            height: client_snapshot.height,
            width: client_snapshot.width,
            now_epoch: crate::time::Timestamp::now().epoch(),
        },
    ) {
        Ok(menu) => menu,
        Err(refusal) => {
            writeln!(err, "ae orchestrator: {}.", refusal.message())?;
            err.flush()?;
            return Ok(EXIT_UNAVAILABLE);
        }
    };
    if let Some(session_id) = opened_session.as_deref()
        && !mark_picker_open(&server, session_id)
    {
        writeln!(
            err,
            "ae orchestrator: tmux refused to update the picker marker for client {client:?}."
        )?;
        err.flush()?;
        return Ok(EXIT_UNAVAILABLE);
    }
    if !draw_picker(&server, client, &menu, probe.menu_mouse()) {
        if let Some(client) = client {
            writeln!(
                err,
                "ae orchestrator: tmux refused to draw the menu for client {client:?} (it may have vanished)."
            )?;
        } else {
            writeln!(
                err,
                "ae orchestrator: tmux refused to draw the menu (no attached client?)."
            )?;
        }
        err.flush()?;
        return Ok(EXIT_UNAVAILABLE);
    }
    Ok(0)
}

/// Refresh the fleet-picker marker whenever a menu opens.
fn mark_picker_open(server: &inventory::ServerId, session_id: &str) -> bool {
    transport::publish_option(
        server,
        tmux::OptionScope::Session,
        session_id,
        theme::MENU_OPEN_OPTION,
        &crate::time::Timestamp::now().epoch().to_string(),
    )
}

/// Read the client's session look, with the transient default fallback.
fn picker_look(server: &inventory::ServerId, session_id: Option<&str>) -> theme::Look {
    let read = session_id.map_or_else(
        || transport::observe_look_here(server),
        |session_id| transport::observe_look(server, session_id),
    );
    read.map_or(theme::Look::DEFAULT, |read| {
        theme::Look::read(&read.icons, &read.palette, &read.drawn, &read.motion)
    })
}

/// Draw the picker at the calling client's left edge.
fn draw_picker(
    server: &inventory::ServerId,
    client: Option<&str>,
    menu: &tmux::Menu,
    menu_mouse: bool,
) -> bool {
    transport::display_menu(server, client, menu, menu_mouse)
}

/// The socket path each server answers with, asked once per server.
///
/// One read per DISTINCT server, however many sessions name it: a fleet on one
/// server is one question, not one per row.
pub(crate) struct SocketPaths {
    /// Who is asked. A parameter so the collision the cache exists to catch can
    /// be proven without two real servers.
    resolve: fn(&inventory::ServerId) -> Option<String>,
    seen: Vec<(inventory::ServerId, Option<String>)>,
}

impl SocketPaths {
    /// A cache that asks `resolve`.
    pub(crate) const fn asking(resolve: fn(&inventory::ServerId) -> Option<String>) -> Self {
        Self {
            resolve,
            seen: Vec::new(),
        }
    }

    /// What `server` calls its own socket, or `None` when it did not answer.
    fn of(&mut self, server: &inventory::ServerId) -> Option<String> {
        if let Some((_, path)) = self.seen.iter().find(|(known, _)| known == server) {
            return path.clone();
        }
        let path = (self.resolve)(server);
        self.seen.push((server.clone(), path.clone()));
        path
    }

    /// Whether both server spellings answered with the same socket path.
    /// Unknown is never equal, even when the spellings themselves match.
    pub(crate) fn proven_same(
        &mut self,
        left: &inventory::ServerId,
        right: &inventory::ServerId,
    ) -> bool {
        let left = self.of(left);
        left.is_some() && self.of(right) == left
    }

    /// Keep the first spelling of each server whose socket identity tmux can
    /// prove. Recorded pointers precede the default named entitlement, so a
    /// durable record retains its own spelling when both name one server.
    fn deduplicated(&mut self, servers: Vec<inventory::ServerId>) -> Vec<inventory::ServerId> {
        let mut paths: Vec<String> = Vec::new();
        let mut distinct = Vec::new();
        for server in servers {
            let path = self.of(&server);
            if path.as_ref().is_some_and(|path| paths.contains(path)) {
                continue;
            }
            if let Some(path) = path {
                paths.push(path);
            }
            distinct.push(server);
        }
        distinct
    }

    /// Whether two spellings were proven to name one socket while building a
    /// de-duplicated entitlement set.
    fn equivalent(&self, left: &inventory::ServerId, right: &inventory::ServerId) -> bool {
        if left == right {
            return true;
        }
        let known = |server: &inventory::ServerId| {
            self.seen
                .iter()
                .find(|(candidate, _)| candidate == server)
                .and_then(|(_, path)| path.as_deref())
        };
        known(left).is_some() && known(left) == known(right)
    }
}

/// Fleet discovery with one opportunistic source: ae's built-in server is
/// useful before any meta points at it, but its cold absence is not inventory
/// loss. Once a durable record names it, an unanswered query is loss again.
/// Warm launches record the socket spelling instead; that server's own failed
/// socket query still carries the loss while the unused name stays optional.
struct FleetDiscovery<'a, D> {
    inner: &'a D,
    default_is_optional: bool,
    /// Which sessions each recorded server holds, for the boot-time reading of
    /// a server whose socket went with the host.
    recorded: &'a inventory::RecordedOn,
    boot: Option<i64>,
    now: i64,
}

impl<D: inventory::Discovery> inventory::Discovery for FleetDiscovery<'_, D> {
    fn enumerate(
        &self,
        server: &inventory::ServerId,
    ) -> std::result::Result<Vec<inventory::DiscoveredSession>, inventory::QueryFailed> {
        match self.inner.enumerate(server) {
            Err(_)
                if self.default_is_optional
                    && server
                        == &inventory::ServerId::Selected(meta::Selector::Name(
                            doors::DEFAULT_SERVER_NAME.to_owned(),
                        )) =>
            {
                Ok(Vec::new())
            }
            // A REBOOT, and the listing's version of the resume's proof: this
            // server did not answer, its socket is not there at all, and every
            // session ae recorded on it was last live before the host booted.
            // None of them can be on it, so it enumerated EMPTY rather than
            // failing — the rows read `stopped` instead of `unknown` and the
            // listing stops calling itself incomplete over it. One gap in that
            // evidence, and it is a failed source again.
            Err(failed)
                if self.recorded.all_predate(server, self.boot, self.now)
                    && transport::server_socket_missing(server) =>
            {
                let _ = failed;
                Ok(Vec::new())
            }
            answer => answer,
        }
    }
}

/// The human route: what `ae` itself answers, once the doors have said what
/// this invocation carries.
fn run_entry(
    preamble: &entry::Preamble,
    argv: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let code = match entry::route(preamble, argv, doors::calling_pane_id().as_deref()) {
        entry::Route::Help => {
            write!(out, "{}", entry::HELP)?;
            0
        }
        entry::Route::Version => {
            writeln!(out, "{}", version_line())?;
            write_autoupgrade_status(out)?;
            0
        }
        // STDERR and 0, as the glue had it: the text is a diagnostic, and a
        // human who asked for it still asked correctly.
        entry::Route::ListHelp => {
            write!(err, "{}", entry::LIST_HELP)?;
            0
        }
        entry::Route::Retired(text) => {
            write!(err, "{text}")?;
            entry::EXIT_USAGE
        }
        entry::Route::ArchiveUsage => {
            write!(err, "{}", entry::ARCHIVE_USAGE)?;
            entry::EXIT_FAILED
        }
        entry::Route::ArchivePreview(name) => {
            return run_archive_preview(preamble, name.as_deref(), out, err);
        }
        entry::Route::Core(effective) => return run_dispatch(&effective, out, err),
        entry::Route::Attach => return run_bare_attach(preamble, out, err),
        entry::Route::Launch(user) => {
            if user.first().map(String::as_str) == Some(orchestrator::ORCHESTRATOR_SESSION) {
                let Some(flags) = orchestrator::parse_launch_tail(&user[1..]) else {
                    return run_dispatch(&user, out, err);
                };
                let mut seat = preamble.clone();
                seat.local = Some(preamble.home.join(orchestrator::CONFIG_FILE));
                let launch = orchestrator::seat_launch_args();
                if let Some(attach) = flags.attach {
                    seat.attach = attach;
                }
                seat.inside_tmux |= flags.inside_tmux;
                seat.no_autostart |= flags.no_autostart;
                return run_launch(&seat, &launch, out, err);
            }
            return run_launch(preamble, &user, out, err);
        }
    };
    out.flush()?;
    err.flush()?;
    Ok(code)
}

/// Bare `ae`: list when already on the launch server, print the cross-server
/// attach hint from another tmux, or attach this terminal and let tmux choose
/// its most recently used session.
fn run_bare_attach(
    preamble: &entry::Preamble,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let Some(server) = preamble.launch_target.as_ref() else {
        writeln!(
            err,
            "Error: '{}' is not a tmux server kind.",
            preamble.server_kind
        )?;
        return Ok(entry::EXIT_USAGE);
    };
    if preamble.inside_tmux {
        let same_server = preamble.caller_server.as_ref().is_some_and(|caller| {
            let mut sockets = SocketPaths::asking(transport::observe_socket_path);
            sockets.proven_same(caller, server)
        });
        if same_server {
            return run_dispatch(&["list".to_owned()], out, err);
        }
        writeln!(
            out,
            "ae: the ae fleet is on another tmux server. Attach with: {}",
            session_launch::server_attach_hint(server)
        )?;
        return Ok(0);
    }
    if transport::session_names(server).is_some_and(|names| !names.is_empty()) {
        return Ok(transport::attach(server));
    }
    writeln!(err, "ae: no running ae session. Start one with: ae <name>")?;
    let (_, world) = current_world(&preamble.home);
    let mut stopped = world
        .sessions
        .iter()
        .filter(|session| session.status == digest::Status::Stopped)
        .map(|session| session.name.as_str())
        .collect::<Vec<_>>();
    stopped.sort_unstable();
    if !stopped.is_empty() {
        writeln!(err, "Stopped sessions: {}", stopped.join(", "))?;
    }
    Ok(entry::EXIT_FAILED)
}

/// `ae archive preview [name]` — resolve the target, path-check it, then hand
/// the resolved directory to the read-only tracer.
fn run_archive_preview(
    preamble: &entry::Preamble,
    name: Option<&str>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let named = name
        .map(ToOwned::to_owned)
        .or_else(|| current_session_name(preamble));
    let Some(target) = named else {
        write!(err, "{}", entry::ARCHIVE_PREVIEW_USAGE)?;
        err.flush()?;
        return Ok(entry::EXIT_FAILED);
    };
    if !session_name_usable(preamble, &target) {
        writeln!(err, "ae: '{target}' is not a usable session name.")?;
        err.flush()?;
        return Ok(entry::EXIT_FAILED);
    }
    let dir = preamble.sessions().join(&target);
    if !lifecycle::dir_exists(&dir) {
        writeln!(err, "ae: no session state for '{target}'.")?;
        err.flush()?;
        return Ok(entry::EXIT_FAILED);
    }
    if !session_path_is_safe(preamble, &target) {
        write_unsafe_path(&dir, err)?;
        err.flush()?;
        return Ok(entry::EXIT_FAILED);
    }
    let code = archive::preview(&dir, out, err)?;
    out.flush()?;
    err.flush()?;
    Ok(code)
}

/// The launch fall-through: the prelude the glue ran, then `_launch`.
fn run_launch(
    preamble: &entry::Preamble,
    user: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let plan = match session_launch::parse_plan(user) {
        Ok(plan) => plan,
        Err(line) => {
            writeln!(err, "{line}")?;
            err.flush()?;
            return Ok(entry::EXIT_USAGE);
        }
    };
    let Some(hint) = plan.name.as_deref() else {
        writeln!(err, "{}", session_launch::MISSING_NAME)?;
        writeln!(err, "{}", session_launch::PUBLIC_USAGE)?;
        err.flush()?;
        return Ok(entry::EXIT_USAGE);
    };
    let deps = doctor::check_deps(&[], err)?;
    if deps != 0 {
        err.flush()?;
        return Ok(deps);
    }
    // The first-run config SEEDING is not here: it is a write, and every write
    // a launch makes belongs below the tmux floor gate. `session_launch::launch`
    // does both, in that order, so there is exactly ONE floor decision per
    // launch and nothing can happen between the two.
    // The NAME grammar is the launch's own and answers first, so a traversal
    // name is refused as a name rather than as a path object and the message
    // says what is actually wrong.
    if session_name_usable(preamble, hint) && !session_path_is_safe(preamble, hint) {
        write_unsafe_path(&preamble.sessions().join(hint), err)?;
        err.flush()?;
        return Ok(entry::EXIT_FAILED);
    }
    run_dispatch(&preamble.launch_argv(user), out, err)
}

/// The two lines an unsafe session path is refused with.
fn write_unsafe_path(path: &std::path::Path, err: &mut impl Write) -> Result<()> {
    writeln!(
        err,
        "Error: {} is not a plain directory (symlink, file, or outside the sessions root).",
        path.display()
    )?;
    writeln!(
        err,
        "       Refusing to use it — a symlinked session directory is an escape wearing a valid name."
    )?;
    Ok(())
}

/// The session the CALLER is sitting in, or `None`.
fn current_session_name(preamble: &entry::Preamble) -> Option<String> {
    if !preamble.inside_tmux {
        return None;
    }
    let server = preamble.caller_server.as_ref()?;
    let name = transport::observe_current_session(server)?;
    lifecycle::dir_exists(&preamble.sessions().join(&name)).then_some(name)
}

/// Whether `name` may be used for an EXISTING session: the grammar, or a
/// pre-grammar name that is already a direct-child directory.
fn session_name_usable(preamble: &entry::Preamble, name: &str) -> bool {
    if session_launch::name::is_session_name(name) {
        return true;
    }
    entry::is_direct_child_name(name)
        && lstat_kind(&preamble.sessions().join(name)) == Some(PathKind::Directory)
}

/// Whether the on-disk object at `<sessions>/<name>` is safe to treat as that
/// session's directory — a PATH question, answered INDEPENDENTLY of the name.
fn session_path_is_safe(preamble: &entry::Preamble, name: &str) -> bool {
    if !entry::is_direct_child_name(name) {
        return false;
    }
    // ABSENT is safe (nothing to escape through yet); a symlink of ANY kind is
    // not, DANGLING INCLUDED — which is why this is an lstat and not an
    // existence test.
    match lstat_kind(&preamble.sessions().join(name)) {
        None | Some(PathKind::Directory) => true,
        Some(PathKind::Symlink | PathKind::Other) => false,
    }
}

/// What an lstat says a path IS — `None` for a path that is not there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathKind {
    /// A symlink, dangling or not.
    Symlink,
    /// A real directory.
    Directory,
    /// A file, socket, device — anything else.
    Other,
}

/// `lstat(2)`: classifies the node itself, never what it points at.
fn lstat_kind(path: &std::path::Path) -> Option<PathKind> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the session-path guard must see a DANGLING symlink as standing, which only lstat does — see clippy.toml"
    )]
    let probe = std::fs::symlink_metadata(path);
    let meta = probe.ok()?;
    if meta.file_type().is_symlink() {
        return Some(PathKind::Symlink);
    }
    Some(if meta.is_dir() {
        PathKind::Directory
    } else {
        PathKind::Other
    })
}

/// Write the default config, once, if there is none.
///
/// Called from `session_launch::launch`, BELOW its floor gate: this is the
/// first thing a launch writes, and a machine ae will not run on has to be told
/// so before ae leaves anything behind.
pub(crate) fn seed_default_config(
    path: &std::path::Path,
    contents: &str,
    name: &str,
    err: &mut impl Write,
) -> Result<Option<u8>> {
    if regular_file(path) {
        return Ok(None);
    }
    if path.file_name().is_none() {
        writeln!(err, "ae: {} is not a config file path.", path.display())?;
        return Ok(Some(entry::EXIT_FAILED));
    }
    if let Some(parent) = path.parent()
        && let Err(why) = std::fs::create_dir_all(parent)
    {
        writeln!(err, "ae: could not create {} ({why}).", parent.display())?;
        return Ok(Some(entry::EXIT_FAILED));
    }
    // `File::create` used 0666 before this path gained O_EXCL; retain its
    // caller-umask-derived mode while sharing init's race-safe publisher.
    if let Err(why) = crate::init::create_exclusive(path, contents.as_bytes(), 0o666) {
        // Another first-run publisher won after the initial probe. It owns the
        // complete file because both paths use the same O_EXCL writer.
        if why.kind() == std::io::ErrorKind::AlreadyExists && regular_file(path) {
            return Ok(None);
        }
        writeln!(
            err,
            "ae: could not create the default config at {} ({why}).",
            path.display()
        )?;
        return Ok(Some(entry::EXIT_FAILED));
    }
    // STDERR, not stdout: the launch's stdout belongs to the session it is
    // about to become.
    writeln!(err, "Created {name} config at {}", path.display())?;
    Ok(None)
}

/// Whether `path` is a regular file, symlinks followed.
fn regular_file(path: &std::path::Path) -> bool {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the first-run config seeding asks whether a config is already there — see clippy.toml"
    )]
    let probe = std::fs::metadata(path);
    probe.is_ok_and(|meta| meta.is_file())
}

/// The `_say` arm.
fn run_say(
    dir: &std::path::Path,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    const USAGE: &str = "Usage: say <text>          # push a free-text line to the Telegram chat\n       echo \"text\" | say   # multi-line / long replies via stdin\n\nEmits a `chat` event the telegram bridge forwards. Requires the bridge running\nwith `chat` in its [telegram] include filter (see `ae telegram status`).\n";
    let text = if tail.is_empty() {
        let mut piped = String::new();
        if std::io::IsTerminal::is_terminal(&std::io::stdin())
            || std::io::Read::read_to_string(&mut std::io::stdin(), &mut piped).is_err()
        {
            write!(err, "{USAGE}")?;
            return Ok(state::EXIT_USAGE);
        }
        piped
    } else {
        tail.join(" ")
    };
    if text.trim().is_empty() {
        write!(err, "{USAGE}")?;
        return Ok(state::EXIT_USAGE);
    }
    let viewer = calling_viewer(dir);
    let _ = store::open(dir).append_event(&tracked::event_line(&tracked::EventFields {
        ts: time::Timestamp::now(),
        actor: &viewer.display,
        action: "chat",
        target: "",
        reference: "",
        actor_slot: &viewer.slot,
        actor_session: "",
        target_slot: "",
        target_session: "",
        summary: &text,
        body_file: "",
    }));
    let head: String = text.chars().take(60).collect();
    let ellipsis = if text.chars().count() > 60 { "…" } else { "" };
    writeln!(out, "Sent to Telegram bridge (chat): {head}{ellipsis}")?;
    Ok(0)
}

/// The `_state` arm.
fn run_state(
    dir: &std::path::Path,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let declaration = match state::parse(tail) {
        Err(usage) => {
            write!(err, "{}", usage.render())?;
            return Ok(state::EXIT_USAGE);
        }
        Ok(state::Command::Read) => {
            out.write_all(&state::read(dir, &calling_viewer(dir)))?;
            return Ok(0);
        }
        Ok(state::Command::Declare(declaration)) => declaration,
    };
    match state::declare(
        dir,
        &calling_viewer(dir),
        &declaration,
        time::Timestamp::now(),
    ) {
        Ok(line) => {
            out.write_all(line.as_bytes())?;
            Ok(0)
        }
        Err(failure) => {
            writeln!(err, "{}", failure.message())?;
            Ok(state::EXIT_FAILED)
        }
    }
}

/// The `next`/`jump` arm — frozen `cmd_next`, over the world `list` renders.
/// `ae brief [session] [--all] [--since <dur>]` — the cards, most actionable
/// first.
///
/// The impure half of the command, and deliberately thin: it resolves WHICH
/// sessions the argv names and hands each one to [`brief::card_for`], which owns
/// every read of a session directory. The one thing asked of the outside world
/// beyond that is git's opinion of each live work tree, through the same
/// [`git::work_tree_dirty`] the watchdog's branch marker uses — so a dirty card
/// and a dirty status line can never disagree.
fn run_brief(
    tail: &[String],
    world: Option<&listing::World>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let args = match brief::parse(tail) {
        Ok(args) => args,
        Err(usage) => {
            write!(err, "{}", usage.render())?;
            return Ok(entry::EXIT_USAGE);
        }
    };
    let (Some(world), Some(root)) = (world, state_root()) else {
        writeln!(err, "ae: {NO_STATE_ROOT}")?;
        return Ok(EXIT_UNAVAILABLE);
    };
    // Before ANY absence claim: an enumeration that lost a source cannot prove a
    // session is not there, and both refusals below are absence claims. The
    // warning is `ae list`'s own, so the two surfaces say the same thing.
    if let Some(warning) = listing::diagnostic(world) {
        writeln!(err, "{warning}")?;
    }
    let named = match &args.target {
        brief::Target::All => None,
        brief::Target::Named(name) => Some(name.clone()),
        // No name given: the session this pane is in. A caller outside one, or
        // in a session ae cannot see, gets the whole fleet rather than nothing.
        brief::Target::Caller => calling_session_name()
            .filter(|name| world.sessions.iter().any(|entry| &entry.name == name)),
    };
    let selected: Vec<&digest::SessionEntry> = if let Some(name) = &named {
        let Some(entry) = world.sessions.iter().find(|entry| &entry.name == name) else {
            // A COMPLETE enumeration proves absence; an incomplete one only
            // proves this reader did not see it.
            if world.inventory_complete() {
                writeln!(err, "ae brief: no session named {name}")?;
            } else {
                writeln!(
                    err,
                    "ae brief: {name} is in no session source ae could read, so ae will not \
                     say it is gone."
                )?;
            }
            return Ok(EXIT_UNAVAILABLE);
        };
        vec![entry]
    } else {
        filters::Selection::running().select(&world.sessions, world.now)
    };
    if selected.is_empty() {
        write!(err, "{}", brief::NOTHING)?;
        return Ok(0);
    }
    let sessions = inventory::Roots::under(&root);
    let home = doors::home();
    let cards = brief::ordered(
        selected
            .iter()
            .map(|entry| {
                let dirty = brief::wants_git(entry)
                    && entry
                        .work_dir
                        .as_deref()
                        .is_some_and(|dir| git::work_tree_dirty(dir.as_bytes()));
                brief::card_for(
                    entry,
                    &sessions.sessions().join(&entry.name),
                    home.as_deref(),
                    dirty,
                    world.now,
                    args.since_secs,
                )
            })
            .collect(),
    );
    write!(out, "{}", brief::render(&cards))?;
    Ok(0)
}

/// The session the calling pane sits in, on the server this invocation would
/// ask — `None` outside tmux, or when that server does not answer for the pane.
fn calling_session_name() -> Option<String> {
    let pane = doors::calling_pane_id()?;
    let server = doors::caller_server()?;
    transport::observe_viewer(&server, &pane)?.session
}

fn run_next(
    tail: &[String],
    world: Option<&listing::World>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let args = match next::parse(tail) {
        Ok(args) => args,
        Err(usage) => {
            write!(err, "{}", usage.render())?;
            return Ok(usage.code());
        }
    };
    let Some(world) = world else {
        writeln!(err, "ae: {NO_STATE_ROOT}")?;
        return Ok(EXIT_UNAVAILABLE);
    };
    let Some(choice) = next::choose(world) else {
        writeln!(err, "{}", next::NOTHING)?;
        return Ok(next::EXIT_NONE);
    };
    if !args.attach {
        write!(out, "{}", choice.line())?;
        return Ok(0);
    }

    let Some(root) = state_root() else {
        writeln!(err, "ae: {NO_STATE_ROOT}")?;
        return Ok(EXIT_UNAVAILABLE);
    };
    let dir = inventory::Roots::under(&root).sessions().join(&choice.name);
    let server = match meta::read_bytes(&dir)
        .map(|bytes| meta::Meta::parse(&String::from_utf8_lossy(&bytes)).server_selector())
    {
        Ok(meta::ServerSelector::Positive(selector)) => inventory::ServerId::Selected(selector),
        Ok(meta::ServerSelector::Missing | meta::ServerSelector::Ambiguous) | Err(_) => {
            writeln!(
                err,
                "ae next: '{}' has no usable recorded tmux server.",
                choice.name
            )?;
            return Ok(next::EXIT_NONE);
        }
    };

    // Re-validate EXACTLY: the session may have ended between the scan and the
    // jump. The target below is exact too; this remains the race guard between
    // choosing the session and asking tmux to focus it.
    let still_there =
        transport::session_names(&server).is_some_and(|names| names.contains(&choice.name));
    if !still_there {
        writeln!(err, "ae next: '{}' disappeared before attach.", choice.name)?;
        return Ok(next::EXIT_NONE);
    }

    let caller_server = doors::caller_server();
    let inside = doors::inside_tmux(caller_server.as_ref(), err)?;
    let same_server = caller_server.as_ref().is_some_and(|caller| {
        let mut sockets = SocketPaths::asking(transport::observe_socket_path);
        sockets.proven_same(caller, &server)
    });
    if inside
        && same_server
        && caller_server
            .as_ref()
            .and_then(transport::observe_current_session)
            .as_deref()
            == Some(&*choice.name)
    {
        writeln!(
            out,
            "ae next: already in '{}' (attn:{}).",
            choice.name,
            choice.reason.as_str()
        )?;
        return Ok(0);
    }
    // Nothing may still be buffered when tmux takes the terminal.
    out.flush()?;
    err.flush()?;
    Ok(session_launch::attach_on(
        &server,
        caller_server.as_ref(),
        inside,
        &choice.name,
        out,
    )?)
}

/// The `_goal` arm: `--help` and a refused argv are usage at 2; the READ is
/// the first `goal=` record or `(no goal set)`; a set or clear prints its
/// success line only once both writes are down.
fn run_goal(
    dir: &std::path::Path,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let write = match goal::parse(tail) {
        Err(goal::Usage) | Ok(goal::Command::Help) => {
            write!(err, "{}", goal::USAGE)?;
            return Ok(goal::usage_code());
        }
        Ok(goal::Command::Show) => {
            return match goal::show(dir) {
                Ok(bytes) => {
                    out.write_all(&bytes)?;
                    Ok(0)
                }
                Err(why) => {
                    writeln!(err, "{}", goal::Failure::Read(why).message())?;
                    Ok(goal::failure_code())
                }
            };
        }
        Ok(goal::Command::Write(write)) => write,
    };
    match goal::run(dir, &calling_viewer(dir), &write, time::Timestamp::now()) {
        Ok(line) => {
            out.write_all(line.as_bytes())?;
            Ok(0)
        }
        Err(failure) => {
            writeln!(err, "{}", failure.message())?;
            Ok(goal::failure_code())
        }
    }
}

/// The `_ask`/`_review` arm, with the paste
/// delegated to the session's own `send` helper — see [`tracked`].
fn run_tracked(
    kind: tracked::Kind,
    dir: &std::path::Path,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let sender = if let Some(display) = sender_override() {
        Some(tracked::Sender {
            display,
            slot: String::new(),
            session: String::new(),
        })
    } else {
        let own_session = own_session(dir);
        let viewer = actual_calling_pane()
            .as_ref()
            .map(|(pane, _)| requests::Viewer::from_pane(pane, &own_session))
            .unwrap_or_default();
        let known = viewer.is_known();
        known.then_some(tracked::Sender {
            display: viewer.display,
            slot: viewer.slot,
            session: viewer.session,
        })
    };
    let code = tracked::run(
        kind,
        dir,
        tail,
        sender.as_ref(),
        &own_session(dir),
        time::Timestamp::now(),
        entropy(),
        send_defer(),
        out,
        err,
    )?;
    Ok(code)
}

/// `AE_SENDER_OVERRIDE`: how an external actor with no pane (a chat bridge, a
/// webhook) names itself to the tracked-request helpers.
fn sender_override() -> Option<String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the frozen AE_SENDER_OVERRIDE contract for pane-less callers — see clippy.toml"
    )]
    let raw = std::env::var_os("AE_SENDER_OVERRIDE");
    raw.filter(|value| !value.is_empty())
        .map(|value| value.to_string_lossy().into_owned())
}

/// `AE_SEND_DEFER_SEC`: how long a send waits for a busy target before it
/// abandons.
fn send_defer() -> std::time::Duration {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the frozen AE_SEND_DEFER_SEC tunable of the send body — see clippy.toml"
    )]
    let raw = std::env::var_os("AE_SEND_DEFER_SEC");
    raw.and_then(|value| value.to_str().and_then(|text| text.parse::<u64>().ok()))
        .map_or(deliver::DEFAULT_DEFER, std::time::Duration::from_secs)
}

/// The event-field contract of the send body, read off this process's
/// environment:
/// `AE_SENDER_OVERRIDE` and the seven `_AE_EVENT_*` members, an unset or empty
/// variable being none.
fn send_env() -> send::Env {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the frozen _AE_EVENT_*/AE_SENDER_OVERRIDE contract every caller of the send helper writes — see clippy.toml"
    )]
    let read = |name: &str| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string_lossy().into_owned())
    };
    send::Env {
        sender_override: read("AE_SENDER_OVERRIDE"),
        action: read("_AE_EVENT_ACTION"),
        reference: read("_AE_EVENT_REF"),
        summary: read("_AE_EVENT_SUMMARY"),
        actor_slot: read("_AE_EVENT_ACTOR_SLOT").unwrap_or_default(),
        actor_session: read("_AE_EVENT_ACTOR_SESSION").unwrap_or_default(),
        target_slot: read("_AE_EVENT_TARGET_SLOT").unwrap_or_default(),
        target_session: read("_AE_EVENT_TARGET_SESSION").unwrap_or_default(),
    }
}

/// Sixty-four bits nobody chose: `RandomState` is seeded from the OS per
/// process, and needs no door.
fn entropy() -> u64 {
    use std::hash::{BuildHasher, RandomState};
    RandomState::new().hash_one(std::process::id())
}

/// The `_memo` arm: usage at 2; `read`/`tail` render the file to stdout; `add`
/// is the TSV record and its event, with nothing on stdout on success, as the
/// frozen helper prints nothing.
fn run_memo(
    dir: &std::path::Path,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let add = match memo::parse(tail) {
        Err(memo::Usage) => {
            write!(err, "{}", memo::USAGE)?;
            return Ok(state::EXIT_USAGE);
        }
        Ok(memo::Command::View(view)) => {
            return match store::open(dir).memo_bytes() {
                Ok(container) => {
                    out.write_all(&memo::view(&container, &view))?;
                    Ok(0)
                }
                Err(why) => {
                    writeln!(err, "{}", memo::Failure::Read(why).message())?;
                    Ok(state::EXIT_FAILED)
                }
            };
        }
        Ok(memo::Command::Add(add)) => add,
    };
    match memo::run(dir, &calling_viewer(dir), &add, time::Timestamp::now()) {
        Ok(()) => Ok(0),
        Err(failure) => {
            writeln!(err, "{}", failure.message())?;
            Ok(state::EXIT_FAILED)
        }
    }
}

/// Who is invoking a helper: the pane `TMUX_PANE` names on the caller's server,
/// classified by [`requests::Viewer::from_pane`].
fn calling_viewer(dir: &std::path::Path) -> requests::Viewer {
    calling_pane()
        .map(|observed| requests::Viewer::from_pane(&observed, &own_session(dir)))
        .unwrap_or_default()
}

/// The pane a helper was invoked from, observed only on the server named by
/// the caller's `$TMUX` marker. Pane ids repeat across servers, so the target
/// session's recorded server cannot identify the caller.
fn calling_pane() -> Option<tmux::ObservedViewer> {
    actual_calling_pane().map(|(viewer, _)| viewer)
}

/// The caller pane observed on the server this PROCESS is actually inside.
///
/// Relay is an authority boundary, so its caller server cannot be inferred
/// from the helper path: a pane may invoke another session's universally
/// linked helper, and pane ids repeat across servers. `$TMUX` names the actual
/// socket inherited by the invoking pane. [`doors::caller_server`] is the one
/// parser for that marker; an absent or untypeable marker fails closed.
pub(crate) fn actual_calling_pane() -> Option<(tmux::ObservedViewer, inventory::ServerId)> {
    let pane = doors::calling_pane_id()?;
    let server = doors::caller_server()?;
    transport::observe_viewer(&server, &pane).map(|viewer| (viewer, server))
}

/// `session=` in `<dir>/meta`, empty-or-missing folded to `None`.
fn session_key(dir: &std::path::Path) -> Option<String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the session name a helper serves, read the way _lib reads _AE_SESSION — see clippy.toml"
    )]
    let raw = std::fs::read(dir.join(crate::store::META));
    raw.ok().and_then(|meta| {
        String::from_utf8_lossy(&meta)
            .lines()
            .find_map(|line| line.strip_prefix("session="))
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

/// The session a helper serves — [`session_key`], or the directory's own name
/// when the key is missing, because that is what the directory IS named.
fn own_session(dir: &std::path::Path) -> String {
    session_key(dir).unwrap_or_else(|| {
        dir.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    })
}

/// The session helper's quota entry: recover the config selection from the
/// helper's durable session rather than consulting tmux.
fn run_quota_helper(
    dir: &std::path::Path,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let meta = session::read_meta(dir).ok();
    let local = meta
        .as_ref()
        .and_then(meta::Meta::origin)
        .and_then(|origin| config::local_overlay(dir, origin));
    let root = state_root().or_else(|| {
        dir.parent()
            .and_then(std::path::Path::parent)
            .map(std::path::Path::to_path_buf)
    });
    let global = root
        .as_deref()
        .map(|root| doors::config_file(shape::current(), root));
    let home = doors::home();
    let roots = root.as_deref().map(inventory::Roots::under);
    quota::run(
        &quota::Inputs {
            home: home.as_deref(),
            global: global.as_deref(),
            local: local.as_deref(),
            sessions: roots.as_ref().map(inventory::Roots::sessions),
            now: time::Timestamp::now().epoch(),
        },
        out,
        err,
    )
}

/// The session helper's usage entry: read only its durable session.
fn run_usage_helper(
    dir: &std::path::Path,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let local = usage::local_config(dir);
    let root = state_root().or_else(|| {
        dir.parent()
            .and_then(std::path::Path::parent)
            .map(std::path::Path::to_path_buf)
    });
    let global = root
        .as_deref()
        .map(|root| doors::config_file(shape::current(), root));
    let prices = match usage::prices::read(global.as_deref(), local.as_deref()) {
        Ok(prices) => prices,
        Err(error) => {
            writeln!(err, "{error}")?;
            return Ok(error.exit_code());
        }
    };
    let home = doors::home();
    let sessions = [usage::SessionInput {
        name: own_session(dir),
        path: dir.to_owned(),
    }];
    usage::run(
        &usage::Inputs {
            home: home.as_deref(),
            sessions: &sessions,
            prices: &prices,
            now: time::Timestamp::now().epoch(),
        },
        false,
        out,
    )
}

/// Where this invocation's state lives — [`doors::state_root`] for this
/// process's [`shape`], which is the one derivation.
pub(crate) fn state_root() -> Option<std::path::PathBuf> {
    doors::state_root(shape::current())
}

/// The classified snapshot AND the world `ae list` shows right now — the real
/// route, returned in both halves so it can be observed from outside.
#[must_use]
pub fn current_world(root: &std::path::Path) -> (liveness::Snapshot, listing::World) {
    let scan = inventory::durable_records(&inventory::Roots::under(root));
    // No caller server: fleet discovery spans recorded destinations plus ae's
    // own default server. A recorded socket may be another spelling of `-L
    // ae`; tmux's own socket answer is the only proof that lets us query it
    // once without losing the durable record's preferred spelling.
    let entitled = inventory::entitled_servers(None, &scan.records);
    let mut sockets = SocketPaths::asking(transport::observe_socket_path);
    let entitled = sockets.deduplicated(entitled);
    let default_recorded = scan.records.iter().any(|record| {
        record.server.entitles()
            == Some(&meta::Selector::Name(doors::DEFAULT_SERVER_NAME.to_owned()))
    });
    let tmux = transport::Tmux;
    let recorded = inventory::RecordedOn::of(&scan.records);
    let discovery = FleetDiscovery {
        inner: &tmux,
        default_is_optional: !default_recorded,
        recorded: &recorded,
        boot: doors::boot_time(shape::current()),
        now: time::Timestamp::now().epoch(),
    };
    let taken = inventory::take_entitled(scan, entitled, &discovery, |left, right| {
        sockets.equivalent(left, right)
    });
    // The SAME discovery for the classification: a server the boot-time proof
    // read as empty must read as empty in both phases, or the rows would go
    // `unknown` under a listing that no longer says anything is missing.
    let snapshot = liveness::classify(taken, &discovery);
    // Criterion 3 only: the opposed disk must change HERE, on this function's
    // path, not after it returns.
    #[cfg(debug_assertions)]
    AFTER_CLASSIFY.with(|slot| {
        if let Some(hook) = slot.get() {
            hook(root);
        }
    });
    let runtimes = observed_runtimes(&snapshot);
    let world = listing::Presentation::enter(&snapshot).world_with(
        time::Timestamp::now(),
        session::DEFAULT_UNANSWERED_SECS,
        &runtimes,
    );
    (snapshot, world)
}

/// What tmux says RIGHT NOW about every classified candidate, in snapshot order.
fn observed_runtimes(snapshot: &liveness::Snapshot) -> Vec<session::SessionRuntime> {
    snapshot
        .sessions
        .iter()
        .map(|classified| {
            let mut runtime = session::SessionRuntime::new(classified.status);
            if classified.status != digest::Status::Running {
                return runtime;
            }
            let Some(record) = classified.candidate.durable.as_ref() else {
                return runtime;
            };
            let Some(selector) = record.server.entitles() else {
                return runtime;
            };
            let server = inventory::ServerId::Selected(selector.clone());
            runtime.branch = transport::observe_branch(&server, &record.name);
            if let (Some(panes), Some(meta)) = (
                transport::observe_panes(&server, &record.name),
                record.snapshot.meta.as_ref(),
            ) {
                let slots: Vec<String> = meta
                    .roster()
                    .iter()
                    .map(|entry| entry.slot.clone())
                    .collect();
                runtime.agents = liveness::agent_runtimes(&panes, &slots);
            }
            runtime
        })
        .collect()
}

#[cfg(debug_assertions)]
thread_local! {
    static AFTER_CLASSIFY: std::cell::Cell<Option<fn(&std::path::Path)>> =
        const { std::cell::Cell::new(None) };
}

/// Arm a callback between classification and [`listing::Presentation::enter`].
#[cfg(debug_assertions)]
pub fn set_after_classify_hook(hook: Option<fn(&std::path::Path)>) {
    AFTER_CLASSIFY.with(|slot| slot.set(hook));
}

/// Run the CLI against `args` over `world` — the injected session source.
///
/// # Errors
///
/// Returns [`Error::Io`] if `out` or `err` cannot be written or flushed.
///
/// ```
/// use ae::digest::{SessionEntry, Status};
/// use ae::listing::World;
/// use ae::time::Timestamp;
///
/// let world = World::new(
///     Timestamp::from_epoch(0),
///     vec![SessionEntry::new("live", Status::Running)],
/// );
/// let (mut out, mut err) = (Vec::new(), Vec::new());
/// let code = ae::run_with(&["list".to_owned()], Some(&world), &mut out, &mut err)?;
/// assert_eq!(code, 0);
/// assert!(String::from_utf8(out).unwrap().contains("live"));
/// # Ok::<(), ae::Error>(())
/// ```
#[allow(
    clippy::too_many_lines,
    reason = "the top-level command dispatch: one match arm per subcommand, kept as one readable table rather than fragmented into sub-dispatchers"
)]
pub fn run_with(
    args: &[String],
    world: Option<&listing::World>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let request = cli::Request::parse(args);
    let code = match &request {
        // Asked-for output goes to stdout.
        cli::Request::Version => {
            writeln!(out, "{}", version_line())?;
            write_autoupgrade_status(out)?;
            request.exit_code().unwrap_or(0)
        }
        cli::Request::Help => {
            write!(out, "{}", help_text())?;
            request.exit_code().unwrap_or(0)
        }
        cli::Request::UsageError(token) => {
            writeln!(err, "ae: unknown argument: {token}")?;
            request.exit_code().unwrap_or(2)
        }
        cli::Request::MissingOperand(command) => {
            // Every internal entry but one is per-SESSION and takes that
            // session's meta directory.
            let operand = if *command == cli::TELEGRAM_RUN {
                "an ae home directory"
            } else if *command == cli::NET_PROBE {
                // Not a directory at all: the one entry here that takes a name
                // to resolve.
                "a host to resolve"
            } else {
                "a session meta directory"
            };
            writeln!(err, "ae: {command} needs {operand}")?;
            request.exit_code().unwrap_or(2)
        }
        // The refusal is a DIAGNOSTIC and never
        // reaches stdout, which is why a refused invocation's stdout is empty
        // rather than a bare header.
        cli::Request::State { dir, tail } => run_state(dir, tail, out, err)?,
        cli::Request::Goal { dir, tail } => run_goal(dir, tail, out, err)?,
        cli::Request::Memo { dir, tail } => run_memo(dir, tail, out, err)?,
        cli::Request::Ask { dir, tail } => run_tracked(tracked::Kind::Ask, dir, tail, out, err)?,
        cli::Request::Interrupt { dir, tail } => {
            let own_session = own_session(dir);
            let viewer = actual_calling_pane()
                .as_ref()
                .map(|(pane, _)| requests::Viewer::from_pane(pane, &own_session))
                .unwrap_or_default();
            interrupt::run(
                dir,
                tail,
                &viewer.display,
                &viewer.session,
                &own_session,
                time::Timestamp::now(),
                err,
            )?
        }
        cli::Request::Send { dir, tail } => {
            let own_session = own_session(dir);
            let viewer = actual_calling_pane()
                .as_ref()
                .map(|(pane, _)| requests::Viewer::from_pane(pane, &own_session))
                .unwrap_or_default();
            send::run(
                dir,
                tail,
                &send_env(),
                &viewer.display,
                &viewer.session,
                &own_session,
                time::Timestamp::now(),
                send_defer(),
                out,
                err,
            )?
        }
        cli::Request::Relay { dir, tail } => {
            let actual = actual_calling_pane();
            relay::run(
                dir,
                tail,
                actual.as_ref().map(|(viewer, server)| (viewer, server)),
                time::Timestamp::now(),
                send_defer(),
                err,
            )?
        }
        cli::Request::Reply { dir, tail } => reply::run(
            dir,
            tail,
            calling_pane().as_ref(),
            &own_session(dir),
            time::Timestamp::now(),
            send_defer(),
            err,
        )?,
        cli::Request::Review { dir, tail } => {
            run_tracked(tracked::Kind::Review, dir, tail, out, err)?
        }
        cli::Request::Say { dir, tail } => run_say(dir, tail, out, err)?,
        cli::Request::Peek { dir, tail } => panes::peek(dir, tail, &own_session(dir), out, err)?,
        cli::Request::Agents { dir, tail } => {
            panes::agents(dir, tail, &own_session(dir), out, err)?
        }
        cli::Request::Quota { dir } => run_quota_helper(dir, out, err)?,
        cli::Request::Usage { dir } => run_usage_helper(dir, out, err)?,
        cli::Request::Focus { dir, tail } => {
            panes::focus(dir, tail, &own_session(dir), time::Timestamp::now(), err)?
        }
        cli::Request::Launch { tail } => session_launch::run(tail, out, err)?,
        cli::Request::CaptureSid { dir, slot, pane } => {
            session_launch::capture::run(dir, slot, pane, &session_launch::recorded_server(dir))
        }
        cli::Request::RegisterSid { dir, slot, id } => {
            session_launch::capture::register_sid(dir, slot, id.as_deref(), out, err)?
        }
        cli::Request::Requests { dir, mode } => {
            let rendered = requests::render(dir, *mode, &calling_viewer(dir));
            out.write_all(&rendered.stdout)?;
            err.write_all(&rendered.stderr)?;
            rendered.code
        }
        // NEVER RETURNS.
        cli::Request::EventsTail { dir } => match events_tail::follow(dir, out)? {},
        cli::Request::LaunchPlan { tail } => identity::launch_plan(tail, out, err)?,
        // The only entry whose payload arrives on STDIN: a launch's seat list
        // is many records, and an argv is not the place for a document.
        cli::Request::MetaInit { dir, tail } => {
            let mut stdin = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut stdin)?;
            identity::meta_init(dir, tail, &stdin, out, err)?
        }
        cli::Request::Spawn { dir, tail } => spawn::run_spawn(
            dir,
            tail,
            &calling_viewer(dir).display,
            time::Timestamp::now(),
            out,
            err,
        )?,
        cli::Request::Retire { dir, tail } => spawn::run_retire(
            dir,
            tail,
            &calling_viewer(dir).display,
            time::Timestamp::now(),
            out,
            err,
        )?,
        // The three whole lifecycle operations.
        cli::Request::End { tail } => {
            if let Some(root) = state_root() {
                lifecycle::end::run(&root, tail, doors::calling_pane_id().as_deref(), out, err)?
            } else {
                writeln!(err, "ae: {NO_STATE_ROOT}")?;
                EXIT_UNAVAILABLE
            }
        }
        cli::Request::Stop { tail } => {
            if let Some(root) = state_root() {
                lifecycle::run_stop(&root, tail, out, err)?
            } else {
                writeln!(err, "ae: {NO_STATE_ROOT}")?;
                EXIT_UNAVAILABLE
            }
        }
        cli::Request::SessionMenu { tail } => {
            if let Some(root) = state_root() {
                session_menu::run(&root, tail, out, err)?
            } else {
                writeln!(err, "ae: {NO_STATE_ROOT}")?;
                EXIT_UNAVAILABLE
            }
        }
        cli::Request::Compact { tail } => {
            if let Some(root) = state_root() {
                lifecycle::compaction::run(&root, tail, out, err)?
            } else {
                writeln!(err, "ae: {NO_STATE_ROOT}")?;
                EXIT_UNAVAILABLE
            }
        }
        // `doctor` and `rename` take no session directory, so like `end`/`stop`
        // they derive the state root themselves and refuse the same way when
        // there is none.
        cli::Request::Doctor { tail } => {
            if let Some(root) = state_root() {
                doctor::run(&root, tail, out, err)?
            } else {
                writeln!(err, "ae: {NO_STATE_ROOT}")?;
                EXIT_UNAVAILABLE
            }
        }
        cli::Request::Init { tail } => {
            if let Some(root) = state_root() {
                let path = doors::config_file(shape::current(), &root);
                init::run(&path, tail, out, err)?
            } else {
                writeln!(err, "ae: {NO_STATE_ROOT}")?;
                EXIT_UNAVAILABLE
            }
        }
        cli::Request::Rename { tail } => {
            if let Some(root) = state_root() {
                rename::run(&root, tail, out, err)?
            } else {
                writeln!(err, "ae: {NO_STATE_ROOT}")?;
                EXIT_UNAVAILABLE
            }
        }
        cli::Request::CheckDeps { tail } => doctor::check_deps(tail, err)?,
        cli::Request::ShimsRender { dir, tail } => doctor::shims_render(dir, tail, err)?,
        cli::Request::Install { tail } => install::run(tail, out, err)?,
        cli::Request::Autoupgrade { tail } => autoupgrade::run(tail),
        cli::Request::Run {
            dir,
            slot,
            print,
            command_snapshot,
        } => run::run(dir, slot, *print, command_snapshot.as_deref(), out, err)?,
        cli::Request::Roster { dir, tail } => identity::roster(dir, tail, out, err)?,
        cli::Request::ManifestRender { dir, tail } => render::run_manifest(dir, tail, out, err)?,
        cli::Request::Context { dir, tail } => render::run_context(dir, tail, out, err)?,
        cli::Request::ArchivePreview { dir } => archive::preview(dir, out, err)?,
        cli::Request::ArchivePublish {
            dir,
            push_outcome,
            push_ref,
            preserved,
            workdir,
            archived_at,
        } => archive::publish::run(
            dir,
            &archive::publish::Ops {
                push_outcome,
                push_ref,
                preserved,
                workdir,
                archived_at,
            },
            out,
            err,
        )?,
        cli::Request::ArchiveFromPreflight { root, raw_uuid } => {
            archive::from::run(root, raw_uuid, out, err)?
        }
        cli::Request::ArchivePurge {
            dir,
            aid,
            source_session,
            parent_id,
        } => archive::purge::run(dir, aid, source_session, parent_id, out, err)?,
        cli::Request::EndLocalTeardown { dir } => teardown::run(dir, out, err)?,
        cli::Request::EndNonlocalTeardown { dir, preserve } => {
            teardown::run_nonlocal(dir, *preserve, out, err)?
        }
        cli::Request::CompactFreeze { dir, keep_history } => {
            compact::freeze(dir, *keep_history, out, err)?
        }
        cli::Request::CompactRevalidate {
            dir,
            tuple,
            when,
            keep_history,
        } => compact::revalidate_step(dir, tuple, *keep_history, when, err)?,
        cli::Request::CompactArchive {
            dir,
            tuple,
            archived_at,
            push_outcome,
            push_ref,
            preserved,
            workdir,
            keep_history,
        } => compact::archive_step(
            dir,
            tuple,
            *keep_history,
            archived_at,
            push_outcome,
            push_ref,
            preserved,
            workdir,
            out,
            err,
        )?,
        cli::Request::CompactTeardown {
            dir,
            tuple,
            keep_history,
        } => compact::teardown_step(dir, tuple, *keep_history, out, err)?,
        cli::Request::CompactWait {
            dir,
            reference,
            timeout_secs,
        } => compact::wait_step(dir, reference, *timeout_secs, err)?,
        cli::Request::CompactCancel { dir, reference } => {
            compact::cancel_step(dir, reference, err)?
        }
        // The two daemons' lifecycle.
        cli::Request::Watchdog { tail } => {
            if let Some(root) = state_root() {
                watchdog_lifecycle::run(&root, tail, out, err)?
            } else {
                writeln!(err, "ae: {NO_STATE_ROOT}")?;
                EXIT_UNAVAILABLE
            }
        }
        cli::Request::Telegram { tail } => {
            if let Some(root) = state_root() {
                telegram_lifecycle::run(&root, tail, out, err)?
            } else {
                writeln!(err, "ae: {NO_STATE_ROOT}")?;
                EXIT_UNAVAILABLE
            }
        }
        cli::Request::WatchdogRun { dir, knobs } => watchdog_daemon::run(dir, *knobs, out, err)?,
        cli::Request::TelegramRun { paths, knobs } => telegram::bridge::run(paths, *knobs, err)?,
        cli::Request::NetProbe { host, port } => netprobe::run(host, *port, out, err)?,
        cli::Request::CompactMemoBaseline { dir } => compact::memo_baseline_step(dir, out)?,
        cli::Request::CompactFindOutstanding { dir } => compact::find_outstanding_step(dir, out)?,
        cli::Request::List(list_args) => {
            if let Some(world) = world {
                // The warning goes to STDERR and the table still prints.
                if !list_args.json
                    && let Some(warning) = listing::diagnostic(world)
                {
                    writeln!(err, "{warning}")?;
                }
                write!(out, "{}", listing::render(list_args, world))?;
                0
            } else {
                writeln!(err, "ae: {NO_STATE_ROOT}")?;
                EXIT_UNAVAILABLE
            }
        }
        cli::Request::Monitor { dir, args } => {
            if let Some(world) = world {
                monitor::run(dir, world, args, out, err)?
            } else {
                writeln!(err, "ae: {NO_STATE_ROOT}")?;
                EXIT_UNAVAILABLE
            }
        }
        cli::Request::Next { tail } => run_next(tail, world, out, err)?,
        cli::Request::Brief { tail } => run_brief(tail, world, out, err)?,
        // A parsed `--popup` is answered before `run_dispatch` reaches this
        // generic arm. Only a direct `run_with` caller can bring one here.
        cli::Request::Orchestrator { tail } => {
            if let Err(usage) = orchestrator::parse(tail) {
                write!(err, "{}", usage.render())?;
                usage.code()
            } else if !orchestrator::parse(tail).is_ok_and(|args| args.popup) {
                // The bare word reaches the core only WITHOUT a preamble: the
                // seat is a launch, and a launch needs the ae command.
                writeln!(
                    err,
                    "ae orchestrator: the seat launches through the ae command"
                )?;
                entry::EXIT_USAGE
            } else {
                writeln!(err, "ae: {NO_STATE_ROOT}")?;
                EXIT_UNAVAILABLE
            }
        }
        cli::Request::LaunchCandidate(name) => {
            writeln!(err, "ae: {NO_LAUNCHER}: {name}")?;
            EXIT_UNAVAILABLE
        }
    };
    out.flush()?;
    err.flush()?;
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::{
        EXIT_UNAVAILABLE, Error, NO_LAUNCHER, NO_STATE_ROOT, help_text, listing::World, run,
        run_with, version_line,
    };
    use crate::digest::{SessionEntry, Status};
    use crate::time::Timestamp;
    use std::io::{self, Write};

    #[test]
    fn the_own_session_is_the_meta_key_or_the_directory_name() {
        let root = std::path::PathBuf::from(format!("/tmp/ae-own-session-{}", std::process::id()));
        let dir = root.join("named");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            super::own_session(&dir),
            "named",
            "no meta: the directory's name"
        );
        std::fs::write(dir.join(crate::store::META), "name=x\nsession=renamed\n").unwrap();
        assert_eq!(super::own_session(&dir), "renamed");
        std::fs::write(dir.join(crate::store::META), "session=\n").unwrap();
        assert_eq!(super::own_session(&dir), "named", "an empty key is no key");
        let _ = std::fs::remove_dir_all(&root);
    }

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    fn world() -> World {
        World::new(
            Timestamp::from_epoch(1_780_000_000),
            vec![
                SessionEntry::new("live", Status::Running),
                SessionEntry::new("old", Status::Stopped),
            ],
        )
    }

    fn aliased_socket_of(server: &crate::inventory::ServerId) -> Option<String> {
        match server {
            crate::inventory::ServerId::Selected(crate::meta::Selector::Name(name))
                if name == crate::doors::DEFAULT_SERVER_NAME =>
            {
                Some("/private/tmux/ae".to_owned())
            }
            crate::inventory::ServerId::Selected(crate::meta::Selector::Socket(path)) => {
                Some(path.display().to_string())
            }
            _ => None,
        }
    }

    fn named(name: &str) -> crate::inventory::ServerId {
        crate::inventory::ServerId::Selected(crate::meta::Selector::Name(name.to_owned()))
    }

    #[test]
    fn a_recorded_socket_suppresses_the_default_name_only_when_tmux_proves_the_alias() {
        let recorded = crate::inventory::ServerId::Selected(crate::meta::Selector::Socket(
            "/private/tmux/ae".into(),
        ));
        let default = named(crate::doors::DEFAULT_SERVER_NAME);
        let unresolved = crate::inventory::ServerId::Ambient;
        let mut sockets = super::SocketPaths::asking(aliased_socket_of);
        let candidates = vec![recorded.clone(), default.clone(), unresolved.clone()];
        assert_eq!(
            sockets.deduplicated(candidates),
            [recorded.clone(), unresolved],
            "the durable spelling wins; an unproved server is never discarded"
        );
        assert!(sockets.equivalent(&recorded, &default));
    }

    #[test]
    fn version_line_names_the_tool_and_the_crate_version() {
        assert_eq!(version_line(), format!("ae {}", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn run_writes_the_version_and_succeeds() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&["--version".to_owned()], &mut out, &mut err).unwrap();
        assert_eq!(code, 0);
        // The expectation is spelled out rather than reusing `version_line()`:
        // comparing the output against the same function that produced it is a
        // test that passes no matter what that function returns.
        let text = String::from_utf8(out).unwrap();
        let mut lines = text.lines();
        assert_eq!(
            lines.next(),
            Some(format!("ae {}", env!("CARGO_PKG_VERSION")).as_str())
        );
        // The floor line is SECOND, and it names a tmux either way: this
        // machine's, or the fact that it has none.
        let floor = lines.next().unwrap_or_default();
        assert!(floor.starts_with("tmux "), "{text}");
        assert!(
            floor.contains(&crate::tmux_floor::REQUIRED.to_string()),
            "{text}"
        );
        assert_eq!(
            lines.next(),
            Some("auto-upgrade: unavailable (checkout builds never auto-upgrade)"),
            "{text}"
        );
        assert_eq!(
            lines.next(),
            Some("upgrade-check: not read for this binary shape"),
            "{text}"
        );
        assert_eq!(lines.next(), None, "{text}");
    }

    #[test]
    fn only_valid_list_brief_and_picker_requests_schedule_automatic_upgrade() {
        let accepted = [
            argv(&["list"]),
            argv(&["list", "--all"]),
            argv(&["brief"]),
            argv(&["brief", "--all", "--since", "4h"]),
            argv(&["orchestrator", "--popup"]),
        ];
        for args in accepted {
            let request = crate::cli::Request::parse(&args);
            assert!(super::schedules_automatic_upgrade(&request), "{args:?}");
        }

        let refused_or_unrelated = [
            argv(&["list", "unexpected"]),
            argv(&["brief", "--since", "bad"]),
            argv(&["orchestrator", "--unknown"]),
            argv(&["next"]),
            argv(&["quota"]),
            argv(&[crate::cli::MONITOR, "sweep"]),
        ];
        for args in refused_or_unrelated {
            let request = crate::cli::Request::parse(&args);
            assert!(!super::schedules_automatic_upgrade(&request), "{args:?}");
        }
    }

    #[test]
    fn the_popup_never_requests_the_durable_world() {
        let popup = crate::cli::Request::parse(&argv(&["orchestrator", "--popup"]));
        assert!(
            !super::request_needs_world(&popup),
            "the fast picker must bypass current_world"
        );
        let list = crate::cli::Request::parse(&argv(&["list"]));
        assert!(super::request_needs_world(&list));
    }

    #[test]
    fn help_names_every_command_the_binary_actually_carries() {
        // CONTENT, not layout (the surface itself is owned elsewhere):
        // row): each shipped spelling has to be findable in the help text, so a
        // command can never ship without appearing here.
        let text = help_text();
        for surface in ["list", "ls", "--json", "--help", "--version"] {
            assert!(text.contains(surface), "help omits {surface}: {text}");
        }
    }

    #[test]
    fn the_help_the_binary_prints_is_the_help_text() {
        // THE DISPATCH, not the human entry.
        for words in [vec!["--help"], vec!["-h"], vec!["help"], vec![]] {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let code = run_with(&argv(&words), None, &mut out, &mut err).unwrap();
            assert_eq!(code, 0, "{words:?}");
            assert_eq!(String::from_utf8(out).unwrap(), help_text(), "{words:?}");
            assert!(err.is_empty(), "{words:?}");
        }
    }

    #[test]
    fn sc_022_an_unknown_option_is_diagnosed_on_stderr_with_stdout_empty() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run_with(&["--nope".to_owned()], None, &mut out, &mut err).unwrap();
        assert_eq!(code, 2);
        assert!(out.is_empty(), "stdout must stay empty: {out:?}");
        assert!(String::from_utf8(err).unwrap().contains("--nope"));
    }

    #[test]
    fn sc_022_a_top_level_session_name_is_not_a_usage_error() {
        // The row's scope clause, at the binary boundary: whatever the binary
        // does with a launch candidate, it must not be `2`, and it must not
        // call the token an unknown command.
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run_with(&["my-feature".to_owned()], None, &mut out, &mut err).unwrap();
        assert_ne!(code, 2, "a session name is not a usage error");
        assert!(out.is_empty(), "stdout must stay empty: {out:?}");
        let message = String::from_utf8(err).unwrap();
        assert!(message.contains(NO_LAUNCHER), "{message}");
        assert!(message.contains("my-feature"), "{message}");
        assert!(
            !message.contains("unknown"),
            "no unknown-command phrase may exist for this token: {message}"
        );
    }

    #[test]
    fn the_version_word_answers_ahead_of_every_gate() {
        // `version` diagnoses a broken install, so it may not depend on the
        // thing it diagnoses: no shape classification, no version-directory
        // validation and no environment door runs before it.
        for word in ["version", "--version", "-V"] {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let code = run(&[word.to_owned()], &mut out, &mut err).unwrap();
            assert_eq!(code, 0, "{word}");
            let text = String::from_utf8(out).unwrap();
            assert!(text.starts_with(&(version_line() + "\n")), "{word}: {text}");
            assert!(err.is_empty(), "{word}");
        }
    }

    #[test]
    fn an_unserved_internal_word_fails_closed_rather_than_launching() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&["_recover-pending".to_owned()], &mut out, &mut err).unwrap();
        assert_eq!(code, crate::entry::EXIT_USAGE);
        assert!(out.is_empty(), "stdout must stay empty: {out:?}");
        let message = String::from_utf8(err).unwrap();
        assert!(message.contains("unknown internal command"), "{message}");
        assert!(message.contains("_recover-pending"), "{message}");
    }

    #[test]
    fn a_served_internal_word_reaches_the_dispatch_without_the_doors() {
        // `_run` is the command every pane execs.
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&[crate::cli::RUN.to_owned()], &mut out, &mut err).unwrap();
        assert_ne!(code, 0);
        assert!(out.is_empty(), "stdout must stay empty: {out:?}");
        assert!(!String::from_utf8(err).unwrap().contains("unknown internal"));
    }

    /// A sink that refuses every write, so the fallible path is a tested path
    /// rather than a documented one.
    struct ClosedPipe;

    impl Write for ClosedPipe {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
    }

    #[test]
    fn a_list_with_no_state_root_says_so_on_stderr_and_exits_one() {
        // What is left of the old refusal.
        for words in [vec!["list"], vec!["ls"], vec!["list", "--all", "--json"]] {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let code = run_with(&argv(&words), None, &mut out, &mut err).unwrap();
            assert_eq!(code, EXIT_UNAVAILABLE, "{words:?}");
            assert_ne!(code, 0, "{words:?}");
            assert_ne!(code, 2, "{words:?}");
            assert!(out.is_empty(), "{words:?}: stdout was {out:?}");
            let message = String::from_utf8(err).unwrap();
            assert!(message.contains(NO_STATE_ROOT), "{words:?}: {message}");
        }
    }

    #[test]
    fn a_wired_list_writes_the_listing_to_stdout_and_exits_zero() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run_with(&argv(&["list"]), Some(&world()), &mut out, &mut err).unwrap();
        assert_eq!(code, 0);
        assert!(err.is_empty(), "nothing is wrong, so nothing is reported");
        let listing = String::from_utf8(out).unwrap();
        assert!(listing.contains("live"), "{listing}");
        assert!(!listing.contains("old"), "running only");
    }

    #[test]
    fn a_wired_list_json_is_the_digest_document() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run_with(
            &argv(&["ls", "--all", "--json"]),
            Some(&world()),
            &mut out,
            &mut err,
        )
        .unwrap();
        assert_eq!(code, 0);
        let rendered = String::from_utf8(out).unwrap();
        let value = crate::json::parse(rendered.trim_end()).expect("one complete document");
        assert_eq!(
            value.get("schema_version"),
            Some(&crate::json::Value::Num(crate::digest::SCHEMA_VERSION)),
            "every successor digest is version 2"
        );
        assert_eq!(
            value.get("inventory_complete"),
            Some(&crate::json::Value::Bool(true)),
            "and every successor digest carries the completeness fact"
        );
    }

    #[test]
    fn a_usage_error_stays_a_usage_error_whether_or_not_a_source_is_wired() {
        // The unwired path must not swallow argv errors: `list --frobnicate`
        // is 2 in both worlds, because the argv was wrong before the source
        // ever mattered.
        for source in [None, Some(&world())] {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let code =
                run_with(&argv(&["list", "--frobnicate"]), source, &mut out, &mut err).unwrap();
            assert_eq!(code, 2);
            assert!(out.is_empty(), "stdout must stay empty: {out:?}");
            assert!(String::from_utf8(err).unwrap().contains("--frobnicate"));
        }
    }

    #[test]
    fn a_positional_after_list_is_a_usage_error_at_the_binary_boundary() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run_with(
            &argv(&["list", "my-feature"]),
            Some(&world()),
            &mut out,
            &mut err,
        )
        .unwrap();
        assert_eq!(code, 2);
        assert!(out.is_empty(), "stdout must stay empty: {out:?}");
        assert!(String::from_utf8(err).unwrap().contains("my-feature"));
    }

    #[test]
    fn a_request_that_succeeded_says_nothing_on_stderr() {
        for words in [
            vec!["--version"],
            vec!["--help"],
            vec!["list"],
            vec!["ls", "--json"],
        ] {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let code = run_with(&argv(&words), Some(&world()), &mut out, &mut err).unwrap();
            assert_eq!(code, 0, "{words:?}");
            assert!(err.is_empty(), "{words:?}: {err:?}");
            assert!(!out.is_empty(), "{words:?}: stdout should carry the answer");
        }
    }

    #[test]
    fn run_surfaces_a_write_failure_on_the_error_stream_too() {
        // The unwired report writes to `err`.
        let mut sink = Vec::new();
        let failed = run(&["list".to_owned()], &mut sink, &mut ClosedPipe).err();
        assert!(matches!(failed, Some(Error::Io(_))), "expected an io error");
    }

    #[test]
    fn run_surfaces_a_write_failure() {
        let mut sink = Vec::new();
        let failed = run(&["--version".to_owned()], &mut ClosedPipe, &mut sink).err();
        assert!(matches!(failed, Some(Error::Io(_))), "expected an io error");
    }
}
