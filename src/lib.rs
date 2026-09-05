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
pub mod identity;
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
pub mod panes;
pub mod procs;
pub mod rename;
pub mod render;
pub mod reply;
pub mod requests;
pub mod roster;
pub mod run;
pub mod send;
pub mod session;
pub mod session_launch;
mod session_tmux;
pub mod shape;
pub mod shim;
pub mod spawn;
pub mod state;
pub mod store;
pub mod teardown;
pub mod telegram;
pub mod telegram_lifecycle;
pub mod time;
pub mod tmux;
pub mod tmux_floor;
pub mod tool;
pub mod tracked;
pub mod transport;
pub mod upgrade;
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

/// The text `ae --help` prints.
#[must_use]
pub fn help_text() -> String {
    format!(
        "{}\n\n\
         Usage: ae [OPTIONS]\n       \
         ae <COMMAND> [OPTIONS]\n\n\
         Commands:\n  \
         list, ls       List ae sessions (--json for the machine-readable digest)\n\n\
         Internal commands (a session's own helpers call these):\n  \
         {} <dir> [mine|inbox|all]\n                 \
         Request state from a session's event log\n  \
         {} <dir>\n                 \
         Follow a session's event log\n\n\
         Options:\n  \
         -h, --help     Print help\n  \
         -V, --version  Print version\n",
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
/// // Two lines: the core version, then this machine's tmux against the floor.
/// let text = String::from_utf8(out).unwrap();
/// assert!(text.starts_with(&(ae::version_line() + "\n")));
/// assert!(text.lines().nth(1).unwrap_or_default().starts_with("tmux "));
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
    // ONE aggregated notice, and only on this path: an agent's `send` would
    // otherwise turn one stale export into a line of noise in every pane.
    if let Some(line) = doors::notice(&doors::ignored(shape)) {
        writeln!(err, "{line}")?;
    }
    let Some(preamble) = resolve_facts(shape, err)? else {
        err.flush()?;
        return Ok(EXIT_UNAVAILABLE);
    };
    run_entry(&preamble, args, out, err)
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
    let inside_tmux = doors::inside_tmux(doors::probe_target(declared.as_ref()).as_ref(), err)?;
    Ok(Some(entry::Preamble {
        global: Some(doors::config_file(shape, &home)),
        local: doors::local_config(&cwd),
        home,
        cwd,
        server_kind,
        server_value,
        inside_tmux,
        attach: true,
        no_autostart: doors::no_autostart(),
    }))
}

/// The ordinary argv dispatch: [`cli::Request::parse`] and the world it needs.
fn run_dispatch(args: &[String], out: &mut impl Write, err: &mut impl Write) -> Result<u8> {
    // Only a listing needs a source, and `next` only needs one once its argv has
    // been accepted: a refused word must not pay for a tmux scan of every
    // session before it can say so, which is what frozen's parse-then-scan order
    // already guaranteed.
    let wants_world = match cli::Request::parse(args) {
        // The sweep reads the same world `list` renders — that IS its input.
        cli::Request::List(_) | cli::Request::Monitor { .. } => true,
        cli::Request::Next { tail } => next::parse(&tail).is_ok(),
        cli::Request::Orchestrator { tail } => orchestrator::parse(&tail).is_ok(),
        _ => false,
    };
    if wants_world && let Some(root) = state_root() {
        let (snapshot, world) = current_world(&root);
        // The picker is the one caller that needs BOTH halves: the world for
        // the rows, and the snapshot for the server a session off this one was
        // recorded on.
        if let cli::Request::Orchestrator { tail } = cli::Request::parse(args) {
            return run_orchestrator(&tail, &snapshot, &world, err);
        }
        return run_with(args, Some(&world), out, err);
    }
    run_with(args, None, out, err)
}

/// `ae orchestrator --popup` — gate the tmux version, then hand tmux the menu.
fn run_orchestrator(
    tail: &[String],
    snapshot: &liveness::Snapshot,
    world: &listing::World,
    err: &mut impl Write,
) -> Result<u8> {
    if let Err(usage) = orchestrator::parse(tail) {
        write!(err, "{}", usage.render())?;
        err.flush()?;
        return Ok(usage.code());
    }
    // The server this invocation may ASK — the declared pair when one redirects
    // ae (the ae-dev namespace), the ambient one otherwise. The menu is drawn
    // for that server's client, and `switch-client` reaches nothing outside it.
    // Which is why a session elsewhere becomes a disabled row.
    let Some(server) = doors::probe_target(doors::declared_server(shape::current()).as_ref())
    else {
        writeln!(
            err,
            "ae orchestrator: the declared tmux server is ambiguous, so ae will not guess one."
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
    // A FAILED listing is not an empty one. Treating it as empty would paint
    // every session as remote and print attach commands for sessions that are
    // right here.
    let Some(panes) = transport::observe_fleet_panes(&server) else {
        writeln!(
            err,
            "ae orchestrator: {} did not list its panes, so ae cannot say which sessions this \
             client can reach.",
            tmux_floor::server_label(&server)
        )?;
        err.flush()?;
        return Ok(EXIT_UNAVAILABLE);
    };
    let recorded = recorded_servers(snapshot);
    let mut sockets = SocketPaths::asking(transport::observe_socket_path);
    let located = placements(&recorded, &server, world, &panes, &mut sockets);
    let menu = orchestrator::menu(world, &located, world.now);
    if !transport::display_menu(&server, &menu) {
        writeln!(
            err,
            "ae orchestrator: tmux refused to draw the menu (no attached client?)."
        )?;
        err.flush()?;
        return Ok(EXIT_UNAVAILABLE);
    }
    Ok(0)
}

/// Where each session of `world` sits, from `caller`'s point of view.
///
/// A NAME is not an address. `switch-client` targets a session by name on the
/// server it is given, so a session ae recorded on some other server, with a
/// same-named stranger on this one, would otherwise get an enabled row whose
/// jump lands in the stranger — carrying the recorded session's goal and
/// attention. The two servers are therefore compared by the SOCKET PATH each
/// one reports for itself, and a session ae cannot prove is here is a row that
/// cannot be chosen.
fn placements(
    recorded: &[(String, inventory::ServerId)],
    caller: &inventory::ServerId,
    world: &listing::World,
    panes: &[tmux::FleetPane],
    sockets: &mut SocketPaths,
) -> Vec<orchestrator::Located> {
    let here = sockets.of(caller);
    world
        .sessions
        .iter()
        .map(|session| {
            let recorded = recorded_server(recorded, &session.name);
            let same_server = here.is_some() && sockets.of(&recorded) == here;
            let mine: Vec<&tmux::FleetPane> = panes
                .iter()
                .filter(|pane| pane.session == session.name)
                .collect();
            let placement = if same_server && !mine.is_empty() {
                orchestrator::Placement::Here(
                    mine.iter()
                        .filter(|pane| !pane.agent.is_empty())
                        .map(|pane| orchestrator::AgentPane {
                            agent: pane.agent.clone(),
                            pane: pane.pane.clone(),
                        })
                        .collect(),
                )
            } else {
                orchestrator::Placement::Elsewhere(orchestrator::attach_command(
                    &recorded,
                    &session.name,
                ))
            };
            orchestrator::Located {
                session: session.name.clone(),
                placement,
            }
        })
        .collect()
}

/// The socket path each server answers with, asked once per server.
///
/// One read per DISTINCT server, however many sessions name it: a fleet on one
/// server is one question, not one per row.
struct SocketPaths {
    /// Who is asked. A parameter so the collision the cache exists to catch can
    /// be proven without two real servers.
    resolve: fn(&inventory::ServerId) -> Option<String>,
    seen: Vec<(inventory::ServerId, Option<String>)>,
}

impl SocketPaths {
    /// A cache that asks `resolve`.
    const fn asking(resolve: fn(&inventory::ServerId) -> Option<String>) -> Self {
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
}

/// Each classified candidate's name and the server its record entitles ae to
/// ask — the snapshot half of a placement, taken once.
fn recorded_servers(snapshot: &liveness::Snapshot) -> Vec<(String, inventory::ServerId)> {
    snapshot
        .sessions
        .iter()
        .map(|classified| {
            let server = classified
                .candidate
                .durable
                .as_ref()
                .and_then(|record| record.server.entitles())
                .map_or(inventory::ServerId::Ambient, |selector| {
                    inventory::ServerId::Selected(selector.clone())
                });
            (classified.candidate.name.clone(), server)
        })
        .collect()
}

/// The server `name` was RECORDED on, or the ambient one when no record
/// entitles ae to name a server for it.
fn recorded_server(recorded: &[(String, inventory::ServerId)], name: &str) -> inventory::ServerId {
    recorded
        .iter()
        .find(|(known, _)| known == name)
        .map_or(inventory::ServerId::Ambient, |(_, server)| server.clone())
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
        entry::Route::Launch(user) => return run_launch(preamble, &user, out, err),
    };
    out.flush()?;
    err.flush()?;
    Ok(code)
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
    let hint = entry::session_hint(user);
    if !hint.is_empty()
        && session_name_usable(preamble, &hint)
        && !session_path_is_safe(preamble, &hint)
    {
        write_unsafe_path(&preamble.sessions().join(&hint), err)?;
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
    let server =
        inventory::ServerId::from_typed_flags(&preamble.server_kind, &preamble.server_value)
            .ok()?;
    let name = transport::observe_current_session(&server)?;
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
    err: &mut impl Write,
) -> Result<Option<u8>> {
    if regular_file(path) {
        return Ok(None);
    }
    let Some(file_name) = path.file_name() else {
        writeln!(err, "ae: {} is not a config file path.", path.display())?;
        return Ok(Some(entry::EXIT_FAILED));
    };
    if let Some(parent) = path.parent()
        && let Err(why) = std::fs::create_dir_all(parent)
    {
        writeln!(err, "ae: could not create {} ({why}).", parent.display())?;
        return Ok(Some(entry::EXIT_FAILED));
    }
    let mut temp_name = file_name.to_os_string();
    temp_name.push(format!(".tmp.{}", std::process::id()));
    let temp = path.with_file_name(temp_name);
    let written = std::fs::File::create(&temp)
        .and_then(|mut file| file.write_all(entry::DEFAULT_CONFIG.as_bytes()));
    if let Err(why) = written {
        let _ = std::fs::remove_file(&temp);
        writeln!(
            err,
            "ae: could not write the default config at {} ({why}).",
            path.display()
        )?;
        return Ok(Some(entry::EXIT_FAILED));
    }
    if let Err(why) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        writeln!(
            err,
            "ae: could not publish the default config at {} ({why}).",
            path.display()
        )?;
        return Ok(Some(entry::EXIT_FAILED));
    }
    // STDERR, not stdout: the launch's stdout belongs to the session it is
    // about to become.
    writeln!(err, "Created default config at {}", path.display())?;
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

    // The ambient server: where a call lands when no `AE_TMUX_SERVER` redirects
    // it.
    let server = inventory::ServerId::Ambient;

    // Re-validate EXACTLY: the session may have ended between the scan and the
    // jump, and a prefix-matching `has-session -t` would land the focus on a
    // surviving sibling.
    let still_there =
        transport::session_names(&server).is_some_and(|names| names.contains(&choice.name));
    if !still_there {
        writeln!(err, "ae next: '{}' disappeared before attach.", choice.name)?;
        return Ok(next::EXIT_NONE);
    }

    let inside = doors::inside_tmux(Some(&server), err)?;
    if inside && transport::observe_current_session(&server).as_deref() == Some(&*choice.name) {
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
    Ok(transport::focus(
        &server,
        tmux::FocusVerb::for_inside(inside),
        &choice.name,
    ))
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
        })
    } else {
        let viewer = calling_viewer(dir);
        let known = viewer.is_known();
        known.then_some(tracked::Sender {
            display: viewer.display,
            slot: viewer.slot,
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

/// Who is invoking a helper: the pane `TMUX_PANE` names, read from the ambient
/// server and classified by [`requests::Viewer::from_pane`].
fn calling_viewer(dir: &std::path::Path) -> requests::Viewer {
    calling_pane(dir)
        .map(|observed| requests::Viewer::from_pane(&observed, &own_session(dir)))
        .unwrap_or_default()
}

/// The tmux server a helper reads its OWN pane on: the session's recorded
/// selector when usable, else the ambient server.
fn viewer_server(dir: &std::path::Path) -> crate::inventory::ServerId {
    match crate::meta::read_bytes(dir)
        .map(|bytes| crate::meta::Meta::parse(&String::from_utf8_lossy(&bytes)).server_selector())
    {
        Ok(crate::meta::ServerSelector::Positive(selector)) => {
            crate::inventory::ServerId::Selected(selector)
        }
        _ => crate::inventory::ServerId::Ambient,
    }
}

/// The pane a helper was invoked from, observed on the session's recorded
/// server — `None` for no `TMUX_PANE`, an empty one, or one the server does not
/// answer for.
fn calling_pane(dir: &std::path::Path) -> Option<tmux::ObservedViewer> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the pane a helper was invoked from is TMUX_PANE, which tmux sets in every pane's environment — see clippy.toml"
    )]
    let pane = std::env::var_os("TMUX_PANE");
    let pane = pane.filter(|value| !value.is_empty())?;
    // Who-am-I reads the caller's pane on the session's OWN recorded server —
    // the core has no $AE_TMUX_SERVER shim, so a session on a non-ambient server
    // (or a caller with no $TMUX) would otherwise read no identity.
    transport::observe_viewer(&viewer_server(dir), pane.to_str()?)
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
    // No ambient server: selecting one is unratified question, and
    // entitlement without a pointer is exactly what is forbidden.
    let taken = inventory::take(scan, None, &transport::Tmux);
    let snapshot = liveness::classify(taken, &transport::Tmux);
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
        cli::Request::Interrupt { dir, tail } => interrupt::run(
            dir,
            tail,
            &calling_viewer(dir).display,
            &own_session(dir),
            time::Timestamp::now(),
            err,
        )?,
        cli::Request::Send { dir, tail } => send::run(
            dir,
            tail,
            &send_env(),
            &calling_viewer(dir).display,
            &own_session(dir),
            time::Timestamp::now(),
            send_defer(),
            err,
        )?,
        cli::Request::Reply { dir, tail } => reply::run(
            dir,
            tail,
            calling_pane(dir).as_ref(),
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
                lifecycle::end::run(&root, tail, out, err)?
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
        cli::Request::Run { dir, slot, print } => run::run(dir, slot, *print, out, err)?,
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
        // A parsed `--popup` is answered in `run_dispatch`, which has the
        // snapshot this arm does not: reaching here with one means there was no
        // state root to read a world from at all.
        cli::Request::Orchestrator { tail } => {
            if let Err(usage) = orchestrator::parse(tail) {
                write!(err, "{}", usage.render())?;
                usage.code()
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

    /// A resolver that gives each NAMED server its own socket, the way tmux
    /// does, and refuses to answer for anything else.
    fn socket_of(server: &crate::inventory::ServerId) -> Option<String> {
        match server {
            crate::inventory::ServerId::Selected(crate::meta::Selector::Name(name)) => {
                Some(format!("/sockets/{name}"))
            }
            _ => None,
        }
    }

    fn named(name: &str) -> crate::inventory::ServerId {
        crate::inventory::ServerId::Selected(crate::meta::Selector::Name(name.to_owned()))
    }

    fn pane_of(session: &str, pane: &str, agent: &str) -> crate::tmux::FleetPane {
        crate::tmux::FleetPane {
            session: session.to_owned(),
            pane: pane.to_owned(),
            agent: agent.to_owned(),
        }
    }

    #[test]
    fn a_same_named_session_on_this_server_is_not_the_one_ae_recorded_elsewhere() {
        use crate::orchestrator::Placement;

        // The hazard: `switch-client -t foo` names a SESSION on whichever server
        // it is given, so a stranger called `foo` here would take the jump — and
        // the row would be wearing the recorded session's goal and attention.
        let mut entry = SessionEntry::new("foo", crate::digest::Status::Running);
        entry.goal = Some("the recorded one".to_owned());
        let world = World::new(Timestamp::from_epoch(1_780_000_000), vec![entry]);
        let recorded = [("foo".to_owned(), named("B"))];
        let stranger = [pane_of("foo", "%7", "lead")];
        let mut sockets = super::SocketPaths::asking(socket_of);

        let located = super::placements(&recorded, &named("A"), &world, &stranger, &mut sockets);
        let [only] = located.as_slice() else {
            panic!("one session, one placement");
        };
        let Placement::Elsewhere(attach) = &only.placement else {
            panic!("a session recorded on another server is not reachable from here");
        };
        assert_eq!(attach, "tmux -L B attach -t foo");

        // …and the same session, recorded on the server that IS this one, is.
        let mut sockets = super::SocketPaths::asking(socket_of);
        let located = super::placements(
            &[("foo".to_owned(), named("A"))],
            &named("A"),
            &world,
            &stranger,
            &mut sockets,
        );
        let Placement::Here(agents) = &located[0].placement else {
            panic!("a session on this very server is reachable");
        };
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].pane, "%7");
    }

    #[test]
    fn a_server_that_will_not_name_its_socket_proves_nothing_and_reaches_nothing() {
        use crate::orchestrator::Placement;

        // Unproven is not the same as proven-here. A caller whose own socket is
        // unknown cannot match anything, and neither can a record's.
        let entry = SessionEntry::new("foo", crate::digest::Status::Running);
        let world = World::new(Timestamp::from_epoch(1_780_000_000), vec![entry]);
        let here = [pane_of("foo", "%1", "lead")];
        for (recorded, caller) in [
            (crate::inventory::ServerId::Ambient, named("A")),
            (named("A"), crate::inventory::ServerId::Ambient),
        ] {
            let mut sockets = super::SocketPaths::asking(socket_of);
            let located = super::placements(
                &[("foo".to_owned(), recorded.clone())],
                &caller,
                &world,
                &here,
                &mut sockets,
            );
            assert!(
                matches!(located[0].placement, Placement::Elsewhere(_)),
                "{recorded:?} from {caller:?} must not be treated as reachable"
            );
        }
    }

    #[test]
    fn one_server_is_asked_for_its_socket_once_however_many_sessions_name_it() {
        let world = World::new(
            Timestamp::from_epoch(1_780_000_000),
            (0..5)
                .map(|index| SessionEntry::new(format!("s{index}"), crate::digest::Status::Running))
                .collect(),
        );
        let recorded: Vec<(String, crate::inventory::ServerId)> = (0..5)
            .map(|index| (format!("s{index}"), named("A")))
            .collect();
        let panes: Vec<crate::tmux::FleetPane> = (0..5)
            .map(|index| pane_of(&format!("s{index}"), &format!("%{index}"), "lead"))
            .collect();
        let mut sockets = super::SocketPaths::asking(socket_of);
        let _located = super::placements(&recorded, &named("A"), &world, &panes, &mut sockets);
        assert_eq!(sockets.seen.len(), 1, "one server, one question");
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
        assert_eq!(lines.next(), None, "{text}");
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
