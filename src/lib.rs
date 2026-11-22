//! `ae` — fleet coordination for AI coding agents: a thin layer over tmux.
//!
//! Seven layer files declare the modules in their matching folders. This root
//! re-exports those modules by name.
//!
//! ```
//! let request = ae::cli::Request::parse(&["--version".to_owned()]);
//! assert_eq!(request, ae::cli::Request::Version);
//! assert_eq!(request.exit_code(), Some(0));
//! ```

mod l0_base;
mod l1_platform;
mod l2_model;
mod l3_services;
mod l4_orchestration;
mod l5_ui;
mod l6_shell;

pub use l0_base::attention;
pub use l0_base::error;
pub use l0_base::event_text;
pub use l0_base::json;
pub use l0_base::provenance;
pub use l0_base::redact;
pub use l0_base::sanitize;
pub use l0_base::time;
pub use l0_base::token;
pub use l0_base::words;
pub use l1_platform::doors;
pub use l1_platform::git;
pub use l1_platform::instance;
pub use l1_platform::netprobe;
pub use l1_platform::procs;
pub use l1_platform::shape;
pub use l1_platform::tmux_floor;
pub use l1_platform::transport;
pub use l2_model::archive;
pub use l2_model::birth;
pub use l2_model::config;
pub use l2_model::conversation;
pub use l2_model::desk;
pub use l2_model::digest;
pub use l2_model::effort_journal;
pub use l2_model::events;
pub use l2_model::filters;
pub use l2_model::goal;
pub use l2_model::harness_state;
pub use l2_model::inventory;
pub use l2_model::launch_cmd;
pub use l2_model::liveness;
pub use l2_model::memo;
pub use l2_model::meta;
pub use l2_model::model_drift;
pub use l2_model::pairs;
pub use l2_model::requests;
pub use l2_model::roster;
pub(crate) use l2_model::session_tmux;
pub use l2_model::state;
pub use l2_model::store;
pub use l2_model::theme;
pub use l2_model::tmux;
pub use l2_model::tool;
pub use l2_model::tracked;
pub use l2_model::wait_on;
pub use l2_model::watchdog;
pub use l3_services::autoupgrade;
pub use l3_services::board;
pub use l3_services::brief;
pub use l3_services::brief_retry;
pub use l3_services::carry;
pub(crate) use l3_services::compact;
pub use l3_services::deliver;
pub use l3_services::deskcmd;
pub use l3_services::deskstore;
pub(crate) use l3_services::fleet_read;
pub use l3_services::fleetfeed;
pub use l3_services::go_usage;
pub use l3_services::init;
pub use l3_services::install;
pub use l3_services::launch;
pub use l3_services::listing;
pub use l3_services::muse_usage;
pub use l3_services::opencode_variant;
pub use l3_services::quota;
pub use l3_services::relay;
pub use l3_services::render;
pub use l3_services::reply;
pub use l3_services::run;
pub use l3_services::send;
pub use l3_services::session;
pub use l3_services::stats;
#[cfg(test)]
pub(crate) use l3_services::stats_fixture;
pub use l3_services::stats_store;
pub use l3_services::telegram;
pub use l3_services::tool_stats;
#[cfg(test)]
pub(crate) use l3_services::tool_stats_fixture;
pub use l3_services::upgrade;
pub use l3_services::usage;
pub use l4_orchestration::autoreseat;
pub use l4_orchestration::autotrust;
pub use l4_orchestration::effort;
pub use l4_orchestration::interrupt;
pub use l4_orchestration::lifecycle;
pub use l4_orchestration::migrate;
pub use l4_orchestration::orchestrator;
pub use l4_orchestration::orchestrator_autostart;
pub use l4_orchestration::rename;
pub use l4_orchestration::request_verb;
pub(crate) use l4_orchestration::reseat;
pub use l4_orchestration::restore;
pub(crate) use l4_orchestration::seat_relaunch;
pub use l4_orchestration::seatcompact;
pub(crate) use l4_orchestration::seatcompact_run;
pub use l4_orchestration::seatpack;
pub use l4_orchestration::session_launch;
pub use l4_orchestration::session_notice;
pub use l4_orchestration::spawn;
pub use l4_orchestration::teardown;
pub use l4_orchestration::telegram_lifecycle;
pub use l4_orchestration::watchdog_daemon;
pub use l4_orchestration::watchdog_glue;
pub use l4_orchestration::watchdog_lifecycle;
pub use l4_orchestration::worktrees;
pub use l5_ui::app;
pub(crate) use l5_ui::clipboard;
pub use l5_ui::console;
pub use l5_ui::doctor;
pub use l5_ui::events_tail;
pub use l5_ui::jump;
pub use l5_ui::monitor;
pub use l5_ui::next;
pub use l5_ui::overview;
pub use l5_ui::panes;
pub(crate) use l5_ui::profile_view;
pub(crate) use l5_ui::reader;
pub use l5_ui::session_menu;
pub(crate) use l5_ui::settings_menu;
pub use l6_shell::cli;
pub use l6_shell::entry;
pub use l6_shell::identity;
pub use l6_shell::shim;

use std::io::Write;
use std::ops::ControlFlow;

pub use error::{Error, Result};
pub use fleet_read::current_world;
#[cfg(debug_assertions)]
pub use fleet_read::set_after_classify_hook;
pub use shape::{NO_STATE_ROOT, VERSION, version_line};

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
    error::settle(run_program_dispatch(
        program,
        args,
        &mut error::Stdout::new(out),
        err,
    ))
}

fn run_program_dispatch(
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
        // A helper carries no preamble. Help answers directly; every operation
        // goes through the install gate and ordinary dispatch.
        shim::Invocation::Helper { helper, dir } => {
            if let Some(usage) = shim::help(helper, args) {
                write!(out, "{usage}")?;
                out.flush()?;
                return Ok(0);
            }
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
    // Help needs only argv, even when the named session or install is broken.
    if let Some(code) = short_helper_help(args, out)? {
        return Ok(code);
    }
    // Version and upgrade also answer on a broken install: `version` diagnoses
    // a mismatch, and `upgrade` repairs it, so both sit ahead of the gate.
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
    // A STATUSLINE'S PIPE. It drains stdin to EOF on every path, so the
    // script upstream never meets a closed pipe; it pays the gate, but a
    // refusal is silent; and it writes neither stream and exits 0, because a
    // statusline must never show ae noise.
    if args.first().map(String::as_str) == Some(cli::QUOTA_FEED) {
        let payload = quota::feed::drain(&mut std::io::stdin().lock());
        if let (Some(payload), Ok(None), Some(root), [_]) = (
            payload,
            install_gate(&mut std::io::sink()),
            state_root(),
            args,
        ) {
            quota::publish_feed(
                &root,
                doors::claude_config_dir().as_deref(),
                doors::home().as_deref(),
                &payload,
                time::Timestamp::now().epoch(),
            );
        }
        return Ok(0);
    }
    // THE GATE, above every remaining word.
    if let Some(code) = install_gate(err)? {
        return Ok(code);
    }
    // THE SHORT FORM, `ae @<session> <helper> …` — a second spelling of that
    // session's own helper link, and never a launch. It sits HERE, directly
    // under the gate, because that is the whole observational shape a helper
    // has: the gate, the state root, the target directory, and the helper. The
    // ignored-doors notice and the launch preamble below would put a public
    // command's noise on an agent's stderr and ask tmux a question no helper
    // needs. It is also above every fall-through, so a marked word can never
    // create, resume or rename a session.
    if let Some(code) = short_helper(args, out, err)? {
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
    // The tripwire is pure argv text — no state read at all — so it answers
    // before the preamble, whose missing-root refusal would otherwise mask
    // the exit-2 with an exit-1, and before the ignored-doors notice, which
    // would otherwise join its stderr. A bare `ae compact [name]` falls
    // through to the preamble and the `run_entry` arm, which runs the verb.
    if args.first().map(String::as_str) == Some("compact")
        && let Some(code) = crate::lifecycle::compaction::compact_tripwire(&args[1..], err)?
    {
        err.flush()?;
        error::with_status(code, out.flush())?;
        return Ok(code);
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
        err.flush()?;
        error::with_status(code, out.flush())?;
        return Ok(code);
    }
    // Quota reads only selected config and bounded vendor caches. Route it
    // before the ordinary preamble, whose tmux facts would violate that
    // observational boundary and make a missing server block the report.
    // Usage shares quota's observational boundary: the command classifies
    // sessions once, then hands only live durable roots to the pure reader.
    // Worktree cleanup reads git and durable session records only — no tmux
    // fact — so a missing server never blocks the listing or its removal.
    let observational = match args.first().map(String::as_str) {
        Some("quota") => Some(run_public_quota(shape, &args[1..], out, err)?),
        Some("usage") => Some(run_public_usage(shape, &args[1..], out, err)?),
        Some("worktree") => Some(run_public_worktree(shape, &args[1..], out, err)?),
        _ => None,
    };
    if let Some(code) = observational {
        err.flush()?;
        error::with_status(code, out.flush())?;
        return Ok(code);
    }
    let Some(preamble) = resolve_facts(shape, err)? else {
        err.flush()?;
        return Ok(EXIT_UNAVAILABLE);
    };
    run_entry(&preamble, args, out, err)
}

const PROFILE_TRANSCRIPT_BYTES: u64 = 1 << 30;

fn run_public_quota(
    shape: &shape::Shape,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let profiles = tail.first().is_some_and(|word| word == "--profiles");
    if profiles && tail.len() > 1 {
        writeln!(err, "ae quota: unknown argument: {}", tail[1])?;
        return Ok(entry::EXIT_USAGE);
    }
    let format = match quota::parse_args(if profiles { &[] } else { tail }) {
        Ok(format) => format,
        Err(extra) => {
            writeln!(err, "ae quota: unknown argument: {extra}")?;
            return Ok(entry::EXIT_USAGE);
        }
    };
    let Some(root) = doors::state_root(shape) else {
        writeln!(err, "ae: {NO_STATE_ROOT}")?;
        return Ok(EXIT_UNAVAILABLE);
    };
    let cwd = doors::cwd();
    let global = doors::config_file(shape, &root);
    let local = doors::local_config(&cwd, Some(&global));
    let home = doors::home();
    let roots = inventory::Roots::under(&root);
    if profiles {
        return quota::run_profiles(
            &quota::Inputs {
                home: home.as_deref(),
                global: Some(&global),
                local: local.as_deref(),
                sessions: Some(roots.sessions()),
                now: time::Timestamp::now().epoch(),
            },
            PROFILE_TRANSCRIPT_BYTES,
            out,
            err,
        );
    }
    quota::run_as(
        &quota::Inputs {
            home: home.as_deref(),
            global: Some(&global),
            local: local.as_deref(),
            sessions: Some(roots.sessions()),
            now: time::Timestamp::now().epoch(),
        },
        format,
        out,
        err,
    )
}

/// `ae worktree gc`: usage first (exit 2 before any state is read), then the
/// state root the durable owner scan needs.
fn run_public_worktree(
    shape: &shape::Shape,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let parsed = match worktrees::parse_args(tail) {
        Ok(parsed) => parsed,
        Err(complaint) => {
            writeln!(err, "{complaint}")?;
            writeln!(err, "{}", worktrees::USAGE)?;
            return Ok(entry::EXIT_USAGE);
        }
    };
    let Some(root) = doors::state_root(shape) else {
        writeln!(err, "ae: {NO_STATE_ROOT}")?;
        return Ok(EXIT_UNAVAILABLE);
    };
    let now = crate::time::Timestamp::now().epoch();
    Ok(worktrees::run(&root, &parsed, now, out, err)?)
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
    if parsed.hourly {
        let home = doors::home();
        let observed = usage::observe_hourly(&usage::HourlyInputs {
            home: home.as_deref(),
            sessions: &sessions,
            now: time::Timestamp::now().epoch(),
        });
        write!(out, "{}", usage::hourly::render(&observed, parsed.json))?;
        return Ok(0);
    }
    let cwd = doors::cwd();
    let global = doors::config_file(shape, &root);
    let local = doors::local_config(&cwd, Some(&global));
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
    let global = doors::config_file(shape, &home);
    Ok(Some(entry::Preamble {
        local: doors::local_config(&cwd, Some(&global)),
        global: Some(global),
        home,
        cwd,
        server_kind,
        server_value,
        launch_target,
        caller_server,
        inside_tmux,
        attach: true,
        no_autostart: doors::no_autostart(),
        starter: session_launch::Starter::Caller,
    }))
}

/// The ordinary argv dispatch: [`cli::Request::parse`] and the world it needs.
fn run_dispatch(args: &[String], out: &mut impl Write, err: &mut impl Write) -> Result<u8> {
    if speaks_as_reserved(args) {
        writeln!(
            err,
            "ae: AE_SENDER_OVERRIDE REFUSED — it names the console, the human or the orchestrator sender, and no helper speaks as any of them: only the console's own input and ae's own relay frame do."
        )?;
        err.flush()?;
        return Ok(entry::EXIT_USAGE);
    }
    let request = cli::Request::parse(args);
    // A served launch observes its caller ONCE, where `run_with` meets it
    // ([`run_served_launch`]): the class gate and the seat guard judge the
    // same observation.
    // A rename does the same: its gate and its worker judge one observation.
    if let Some(what) = outbound_of(&request)
        && !matches!(request, cli::Request::Launch { .. })
    {
        if matches!(
            request,
            cli::Request::Rename { .. } | cli::Request::End { .. } | cli::Request::Stop { .. }
        ) {
            return run_observed(what, &request, out, err);
        }
        if let Some(code) = refuse_dispatcher(what, err)? {
            return Ok(code);
        }
    }
    run_admitted(args, &request, out, err)
}

/// The calling pane's one soft reading for a bound stop's fleet row: its class
/// as the entry reads it, or `None` outside tmux and for a pane tmux cannot
/// read. Only the row's actor word reads this; the bound stop proves its
/// caller by its own rules.
fn bound_entry(root: &std::path::Path) -> Option<rename::Entry> {
    let seen = actual_calling_pane()?;
    let class = seat_class_of(root, &seen.0, &seen.1);
    Some(rename::Entry {
        seen: Some(seen),
        class: Some(class),
    })
}

/// A rename's or an end's class gate and the operation, from ONE
/// observation of the calling pane. An ordinary agent seat the gate admits,
/// because it names its own session, carries that observation into the
/// rename, which proves under the source's lifecycle lock that the source is
/// still that session; the bound forms judge their caller on it.
fn run_observed(
    what: Outbound,
    request: &cli::Request,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let root = state_root();
    let seen = match observe_calling_pane().require() {
        Ok(seen) => seen,
        Err(why) => {
            writeln!(err, "{why}")?;
            err.flush()?;
            return Ok(entry::EXIT_FAILED);
        }
    };
    let class = match (&seen, root.as_deref()) {
        (Some((viewer, server)), Some(root)) => Some(seat_class_of(root, viewer, server)),
        _ => None,
    };
    if let Some(class) = &class
        && let Some(code) = refuse_class(what, class, root.as_deref(), err)?
    {
        return Ok(code);
    }
    let observed = rename::Entry { seen, class };
    let code = match request {
        cli::Request::End { tail } => with_state_root(err, |root, err| {
            let pane = doors::calling_pane_id();
            lifecycle::end::run_as(root, tail, pane.as_deref(), Some(&observed), out, err)
        })?,
        cli::Request::Rename { tail } => with_state_root(err, |root, err| {
            rename::run_as(root, tail, Some(&observed), out, err)
        })?,
        cli::Request::Stop { tail } => with_state_root(err, |root, err| {
            lifecycle::run_stop(root, tail, Some(&observed), out, err)
        })?,
        _ => return Ok(entry::EXIT_USAGE),
    };
    err.flush()?;
    error::with_status(code, out.flush())?;
    Ok(code)
}

/// The served `_launch` of an admitted bound resume, in this process with the
/// caller the entry observed: the one `_launch` argv that does not cross the
/// dispatcher gate, because the entry admitted its user words for every
/// caller. Anything else here, including a request whose user words are not
/// exactly the bound resume, goes through [`run_dispatch`] and its gate.
fn run_bound_dispatch(
    args: &[String],
    who: &session_launch::LaunchCaller,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let request = cli::Request::parse(args);
    let cli::Request::Launch { tail } = &request else {
        return run_dispatch(args, out, err);
    };
    if !session_launch::serves_bound_resume(tail) || speaks_as_reserved(args) {
        return run_dispatch(args, out, err);
    }
    let route = session_launch::LaunchRoute::Bound;
    let code = session_launch::run(tail, who, route, out, err)?;
    err.flush()?;
    error::with_status(code, out.flush())?;
    Ok(code)
}

/// The served `_launch` of a launch line the entry admitted past the
/// orchestrator class gate, in this process with the caller that gate
/// observed. Anything else here goes through [`run_dispatch`] and its gate.
fn run_caller_dispatch(
    args: &[String],
    (who, route): (&session_launch::LaunchCaller, session_launch::LaunchRoute),
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let request = cli::Request::parse(args);
    let cli::Request::Launch { tail } = &request else {
        return run_dispatch(args, out, err);
    };
    if speaks_as_reserved(args) {
        return run_dispatch(args, out, err);
    }
    let code = session_launch::run(tail, who, route, out, err)?;
    err.flush()?;
    error::with_status(code, out.flush())?;
    Ok(code)
}

/// [`run_dispatch`] past its gate.
fn run_admitted(
    args: &[String],
    request: &cli::Request,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    // A live harness can retain a filesystem helper address from before a
    // rename. Resolve its directory before any helper derives the session
    // name, including the underscore entries that bypass shim translation.
    if args
        .first()
        .is_some_and(|entry| shim::HELPERS.iter().any(|helper| helper.entry == entry))
        && let Some(dir) = request.session_dir()
        && let Some(canonical) = lifecycle::aliases::resolve(dir)
    {
        let mut canonical_args = args.to_vec();
        canonical_args[1] = canonical.to_string_lossy().into_owned();
        return run_dispatch(&canonical_args, out, err);
    }
    // Only a listing needs a source, and `next` only needs one once its argv has
    // been accepted: a refused word must not pay for a tmux scan of every
    // session before it can say so, which is what frozen's parse-then-scan order
    // already guaranteed.
    let popup = matches!(
        request,
        cli::Request::Orchestrator { tail }
            if orchestrator::parse(tail)
                .is_ok_and(|args| args.popup || args.settings || args.quota_dialog)
    );
    if schedules_automatic_upgrade(request) {
        autoupgrade::schedule();
    }
    // The popup is deliberately dispatched BEFORE the world edge: its latency
    // contract is a handful of live tmux listings plus one META-ONLY durable
    // scan for the stopped rows — never the event journals, git probes or
    // per-session runtime observation behind `current_world`.
    if popup && let cli::Request::Orchestrator { tail } = request {
        return run_orchestrator(tail, err);
    }
    let wants_world = request_needs_world(request);
    if wants_world && let Some(root) = state_root() {
        let mut read = fleet_read::world_read(&root, &mut session::RecordSnapshot::read);
        if matches!(request, cli::Request::List(_)) {
            enrich_list_facts(&read.snapshot, &mut read.world, &read.panes);
            enrich_list_seats(&read.snapshot, &mut read.world, &read.pids);
        }
        return run_with(args, Some(&read.world), out, err);
    }
    run_with(args, None, out, err)
}

/// `ae list` pays for one roster-fact read per Running session: the JSON
/// runtime cells and, in both modes, the seat's proven background job.
fn enrich_list_facts(
    snapshot: &liveness::Snapshot,
    world: &mut listing::World,
    listed: &fleet_read::ListedPanes,
) {
    debug_assert_eq!(snapshot.sessions.len(), world.sessions.len());
    // Presentation preserves classified order, including same-name records.
    for (classified, entry) in snapshot.sessions.iter().zip(&mut world.sessions) {
        if classified.candidate.name != entry.name {
            continue;
        }
        let record = classified.candidate.durable.as_ref();
        let meta = record.and_then(|record| record.snapshot.meta.as_ref());
        let roster = meta.map_or(&[][..], meta::Meta::roster);
        let server = record
            .and_then(|record| record.server.entitles())
            .map(|selector| inventory::ServerId::Selected(selector.clone()));
        let fact = server
            .filter(|_| entry.status == digest::Status::Running)
            .and_then(|server| {
                transport::observe_session_option(&server, &entry.name, theme::AGENTS_OPTION)
            });
        let panes: Option<Vec<_>> = record
            .and_then(|record| listed.get(&record.path))
            .and_then(Option::as_ref)
            .map(|agents| {
                agents
                    .iter()
                    .filter_map(|agent| {
                        // The enrichment seam reads only pane_id and slot.
                        Some(tmux::WatchPane {
                            pane_id: agent.pane_id.clone()?,
                            slot: Some(agent.slot.clone()),
                            agent: None,
                            current_command: String::new(),
                            pane_pid: None,
                            observed: String::new(),
                        })
                    })
                    .collect()
            });
        session::enrich_list_json(entry, roster, fact.as_deref(), panes.as_deref(), world.now);
    }
}

/// Each seat's conversation verdict and harness clash, for `ae list` alone: at
/// most ONE process read for the whole listing, shared by both, and only when a
/// seat's journal could name its running start or a live seat records a tool.
fn enrich_list_seats(
    snapshot: &liveness::Snapshot,
    world: &mut listing::World,
    pids: &fleet_read::SeatPids,
) {
    let table = std::cell::OnceCell::new();
    let table = || table.get_or_init(procs::snapshot).as_deref();
    for (classified, entry) in snapshot.sessions.iter().zip(&mut world.sessions) {
        let Some(record) = classified.candidate.durable.as_ref() else {
            continue;
        };
        let Some(meta) = record.snapshot.meta.as_ref() else {
            continue;
        };
        let seats = pids.get(&record.path);
        for agent in &mut entry.agents {
            let Some(seat) = meta.roster().iter().find(|seat| seat.name == agent.name) else {
                continue;
            };
            let pane = seats
                .and_then(|seats| seats.get(&seat.slot))
                .and_then(Option::as_ref);
            let pid = pane.and_then(|pane| pane.pid);
            agent.conversation = session::conversation_of(
                meta,
                record.snapshot.events.as_ref(),
                &seat.slot,
                entry.status,
                agent.alive,
                pid,
                &table,
            );
            agent.harness_clash = pane
                .filter(|_| entry.status == digest::Status::Running && agent.alive == Some(true))
                .and_then(|pane| {
                    session::harness_clash_of(
                        seat.binary.as_deref(),
                        pane.pid,
                        pane.command.as_deref(),
                        &table,
                    )
                });
        }
    }
}

/// Whether a parsed request needs the durable world behind `ae list`.
fn request_needs_world(request: &cli::Request) -> bool {
    match request {
        // The sweep reads the same world `list` renders — that IS its input,
        // and a reseat seeds its successor with the pack `brief --seat` builds
        // from the same world.
        cli::Request::List(_) | cli::Request::Monitor { .. } | cli::Request::Reseat { .. } => true,
        cli::Request::Next { tail } => next::parse(tail).is_ok(),
        // A brief is a reading of the same world, so a refused argv must not pay
        // for the scan either.
        cli::Request::Brief { tail } => brief::parse(tail).is_ok(),
        // A board with explicit names reads durable state directly and never
        // consults liveness; only the default needs the running world.
        cli::Request::Board { tail } => {
            board::parse(tail).is_ok_and(|args| args.sessions.is_empty())
        }
        _ => false,
    }
}

/// Commands whose fully accepted grammar proves ordinary interactive use.
/// Launch/reattach has its own edge after its deeper name/config validation.
fn schedules_automatic_upgrade(request: &cli::Request) -> bool {
    match request {
        cli::Request::List(_) => true,
        // `--full` is the read-only reading: it never schedules the upgrade check.
        cli::Request::Brief { tail } => brief::parse(tail).is_ok_and(|args| !args.full),
        cli::Request::Board { tail } => board::parse(tail).is_ok(),
        cli::Request::Orchestrator { tail } => orchestrator::parse(tail)
            .is_ok_and(|args| args.popup || args.settings || args.quota_dialog),
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
    if args.settings {
        return Ok(run_settings_menu(
            &server,
            client_name,
            probe.menu_mouse(),
            err,
        ));
    }
    if args.quota_dialog {
        return Ok(run_quota_dialog(&server, tail, probe.menu_mouse(), err));
    }
    // The picker's WHOLE read in one tmux invocation: the client snapshot, the
    // session rows, the membership snapshot, the bare name list, the server
    // identity, its socket path and the invoking client's look. Only the floor
    // probe above and the marker write below stay separate calls.
    let Some(read) = transport::observe_picker_read(&server, client_name) else {
        writeln!(
            err,
            "ae orchestrator: {} answered the picker's batched read with output that is not this read's, so ae cannot build the picker.",
            tmux_floor::server_label(&server)
        )?;
        err.flush()?;
        return Ok(EXIT_UNAVAILABLE);
    };
    let Some(client_snapshot) = read.client else {
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
    let Some(sessions) = read.sessions else {
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
    let panes = read.panes.unwrap_or_default();
    let now = crate::time::Timestamp::now().epoch();
    let root = doors::state_root(shape::current());
    // The second source: which durable sessions the CALLING server proves
    // stopped. Its spellings are reconciled with the caller's socket through
    // the same proof the fleet listing uses, so a record written as `-L ae`
    // still answers to the socket `$TMUX` names — and a server ae cannot prove
    // equivalent stays `unknown` rather than being guessed at.
    let stopped = root.as_ref().map_or_else(Vec::new, |root| {
        // META ONLY: a stopped row needs name, goal and branch, never the
        // event journal — this scan does not open `events.jsonl` at all.
        let scan = inventory::durable_meta_records(&inventory::Roots::under(root));
        // The caller's own socket answer came from the batched read, so the
        // cache never asks the same server twice.
        let mut sockets = fleet_read::SocketPaths::for_caller(&server, read.socket.clone());
        let mut servers = vec![server.clone()];
        for record in &scan.records {
            if let Some(selector) = record.server.entitles() {
                let id = inventory::ServerId::Selected(selector.clone());
                if !servers.contains(&id) {
                    servers.push(id);
                }
            }
        }
        let _ = sockets.deduplicated(servers);
        let backend = PickerStoppedBackend {
            server: &server,
            names: read.names.as_deref(),
            live: &sessions,
            sockets: &sockets,
        };
        picker_stopped(scan, &backend, &sessions)
    });
    // The resume pin: a stopped row acts as the client that opened this menu,
    // never as whichever attachment tmux last saw. A core or server identity
    // that cannot be captured leaves those rows informational instead of
    // offering an action nothing could prove.
    let launcher = picker_launcher_for(&server);
    let identity = read.identity;
    let resume = match (&launcher, &identity) {
        (Some(launcher), Some(identity)) => Some(orchestrator::PickerResume {
            client: client_name,
            client_pid: &client_snapshot.pid,
            server_pid: &identity.pid,
            server_start: &identity.start,
            deadline: now + session_menu::CONFIRM_WINDOW_SECS,
            launcher,
        }),
        _ => None,
    };
    if resume.is_none() && !stopped.is_empty() {
        writeln!(
            err,
            "ae orchestrator: the invoking server's identity or this core could not be captured, so stopped rows are informational only."
        )?;
    }
    // The picker draws in the calling session's own look, so a session running
    // the ASCII fallback gets an ASCII menu and a themed one gets its palette.
    // A look ae could not read draws the picker in the default one: a menu is
    // a transient surface that writes nothing, so a wrong palette on it costs
    // one keystroke rather than a session's appearance.
    let look = read.look.as_ref().map_or(theme::Look::DEFAULT, |read| {
        theme::Look::read(&read.icons, &read.palette, &read.drawn, &read.motion)
    });
    let menu = match orchestrator::menu_for_client_session_in(
        &sessions,
        &stopped,
        &panes,
        look.icons,
        &look.palette,
        client,
        opened_session.as_deref(),
        resume.as_ref(),
        orchestrator::PickerBounds {
            height: client_snapshot.height,
            width: client_snapshot.width,
            now_epoch: now,
        },
        // The SAME global config file the watchdogs re-read every cycle: this
        // path and theirs are both `doors::config_file` over the state root.
        &fleet_order_at(
            root.as_ref()
                .map(|root| doors::config_file(shape::current(), root))
                .as_deref(),
        ),
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
    if !draw_marked_menu(
        || draw_picker(&server, client, &menu, probe.menu_mouse()),
        || {
            opened_session.as_deref().is_none_or(|session_id| {
                clear_menu_marker(&server, session_id, theme::MENU_OPEN_OPTION)
            })
        },
    ) {
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

/// Whether the session in `session_dir` acts on vendor quota: its pinned
/// `quota` meta row, else the LIVE CONFIG, else ON — resolved through the ONE
/// `config::resolve_quota_aware` precedence (see it for why). Absent everywhere
/// means ON — exactly today's behaviour — for sessions that pre-date the knob
/// and when the state cannot be read (a menu that cannot prove unawareness
/// keeps today's surface rather than hiding it).
pub(crate) fn session_quota_aware(
    session_dir: &std::path::Path,
    global: Option<&std::path::Path>,
    local: Option<&std::path::Path>,
) -> bool {
    let pinned = crate::meta::read_bytes(session_dir).ok().and_then(|bytes| {
        crate::meta::first_value(&bytes, "quota")
            .map(|value| String::from_utf8_lossy(value).into_owned())
    });
    let configured = crate::config::read_workspace_keys(global, local, &["quota"])
        .into_iter()
        .next()
        .flatten();
    crate::config::resolve_quota_aware(pinned.as_deref(), configured.as_deref())
}

/// `ae orchestrator --settings --client …` — one bottom-right observational menu.
#[allow(
    clippy::too_many_lines,
    reason = "one exact-client settings snapshot, base proof and quota degradation"
)]
fn run_settings_menu(
    server: &inventory::ServerId,
    client_name: &str,
    menu_mouse: bool,
    err: &mut impl Write,
) -> u8 {
    let stderr = |text: &str, err: &mut dyn Write| {
        let _ = writeln!(err, "ae settings: {text}");
    };
    let Some(client) = transport::observe_menu_client(server, client_name) else {
        stderr(
            "the invoking client vanished before ae could build settings",
            err,
        );
        return EXIT_UNAVAILABLE;
    };
    let Some(identity) = transport::observe_server_identity(server) else {
        stderr(
            "the invoking tmux server did not answer with its identity",
            err,
        );
        return EXIT_UNAVAILABLE;
    };
    let report = |text: &str, err: &mut dyn Write| {
        let server_survives = transport::observe_server_identity(server)
            .is_some_and(|live| live.pid == identity.pid && live.start == identity.start);
        let client_survives = transport::observe_menu_client(server, client_name)
            .is_some_and(|live| live.pid == client.pid);
        if server_survives && client_survives {
            let _ = transport::display_client_message(server, client_name, text);
        }
        let _ = writeln!(err, "ae settings: {text}");
    };
    let Some(root) = doors::state_root(shape::current()) else {
        report(NO_STATE_ROOT, err);
        return EXIT_UNAVAILABLE;
    };
    let Some(core) = shape::resolved_exe() else {
        report(
            "ae cannot name its own executable, so settings cannot act",
            err,
        );
        return EXIT_UNAVAILABLE;
    };
    let declared = doors::declared_server(shape::current());
    let control = doors::launch_target(declared.as_ref()).map_or_else(
        || {
            settings_menu::Control::Unavailable(
                "the configured launch server is not addressable".to_owned(),
            )
        },
        |launch_server| settings_menu::discover(&root, server, &launch_server),
    );
    let look = picker_look(server, Some(&client.session_id));
    let version =
        transport::observe_session_option(server, &client.session_id, theme::VERSION_OPTION);
    let config = doors::config_file(shape::current(), &root);
    let launcher = session_tmux::picker_launcher(shape::current(), &core, &root, &config, server);
    let now = time::Timestamp::now().epoch();
    let snapshot = settings_menu::Snapshot {
        client: client_name,
        client_pid: &client.pid,
        session_id: &client.session_id,
        server_pid: &identity.pid,
        server_start: &identity.start,
        deadline: now + session_menu::CONFIRM_WINDOW_SECS,
    };
    let base = settings_menu::menu(
        &control,
        &launcher,
        &snapshot,
        version.as_deref(),
        &look.palette,
    );
    let (base_columns, base_rows) = session_menu::menu_budget(&base);
    if client.width < base_columns || client.height < base_rows {
        report(
            &format!(
                "this terminal is {}x{}; settings needs {base_columns}x{base_rows}",
                client.width, client.height
            ),
            err,
        );
        return EXIT_UNAVAILABLE;
    }

    let entry = settings_menu::quota_entry(&launcher, &snapshot);
    let session_dir = lifecycle::sessions_dir(&root).join(&client.session_name);
    let local = session::read_meta(&session_dir)
        .ok()
        .as_ref()
        .and_then(meta::Meta::origin)
        .and_then(|origin| config::local_overlay(&session_dir, origin));
    let aware = session_quota_aware(&session_dir, Some(&config), local.as_deref());
    // Unaware sessions draw the base control menu only: `menu_for_awareness`
    // returns it directly, so its budget equals the proven base budget below
    // and the quota-degraded arm is unreachable — there is no quota summary
    // to overflow-notice about.
    let full = settings_menu::menu_for_awareness(
        aware,
        &control,
        &launcher,
        &snapshot,
        version.as_deref(),
        &look.palette,
        entry,
    );
    let (full_columns, full_rows) = session_menu::menu_budget(&full);
    let menu = if client.width >= full_columns && client.height >= full_rows {
        full
    } else {
        let degraded = settings_menu::menu_with_quota_notice(
            &control,
            &launcher,
            &snapshot,
            version.as_deref(),
            &look.palette,
            (
                full_rows.saturating_sub(client.height),
                full_columns.saturating_sub(client.width),
            ),
            base_columns,
        );
        let (columns, rows) = session_menu::menu_budget(&degraded);
        if columns <= client.width && rows <= client.height {
            degraded
        } else {
            report("quota summary did not fit; drawing original settings", err);
            base
        }
    };
    let (menu_columns, _) = session_menu::menu_budget(&menu);
    let menu_x = tmux::settings_menu_x(client.width, menu_columns);
    if !mark_settings_open(server, &client.session_id) {
        report(
            "tmux refused to update the settings marker for the invoking client",
            err,
        );
        return EXIT_UNAVAILABLE;
    }
    if !draw_marked_menu(
        || {
            transport::display_settings_menu(
                server,
                client_name,
                &client.session_id,
                menu_x,
                &menu,
                menu_mouse,
            )
        },
        || clear_menu_marker(server, &client.session_id, theme::SETTINGS_OPEN_OPTION),
    ) {
        report(
            "tmux refused to draw settings (the client may have vanished)",
            err,
        );
        return EXIT_UNAVAILABLE;
    }
    0
}

/// Re-observe the captured quota-dialog clicker and reprove every captured
/// fact against live state, immediately before the draw.
///
/// The scan between the menu and this call is tens of milliseconds in which
/// the client may have detached, been replaced, or switched session, and the
/// server may have been replaced. Anything but the captured clicker on the
/// captured server refuses; a residual race after this final observation is
/// irreducible, because tmux offers no atomic draw-on-this-client-if-still-pid-N.
fn prove_quota_dialog_clicker(
    server: &inventory::ServerId,
    captured: &settings_menu::CapturedQuotaDialog,
) -> std::result::Result<tmux::MenuClient, String> {
    let Some(live) = transport::observe_menu_client(server, &captured.client) else {
        return Err("the invoking client vanished before ae could draw the dialog".to_owned());
    };
    if live.pid != captured.client_pid {
        return Err(
            "the invoking client was replaced since the menu opened; nothing was drawn".to_owned(),
        );
    }
    if live.session_id != captured.session_id {
        return Err(
            "the invoking client switched session since the menu opened; nothing was drawn"
                .to_owned(),
        );
    }
    let Some(live_server) = transport::observe_server_identity(server) else {
        return Err("the invoking tmux server did not answer with its identity".to_owned());
    };
    if live_server.pid != captured.server_pid || live_server.start != captured.server_start {
        return Err(
            "the invoking tmux server was replaced since the menu opened; nothing was drawn"
                .to_owned(),
        );
    }
    Ok(live)
}

/// Best-effort tell for a quota-dialog invocation the strict grammar refused.
/// Only an addressable `--client` value resolving to an attached client hears
/// a STATIC text — the hostile tail is never echoed into tmux's format
/// expander. A hand-run's stderr keeps the exact reason.
fn tell_quota_dialog_parse_refusal(
    server: &inventory::ServerId,
    tail: &[String],
    err: &mut impl Write,
    why: &str,
) {
    let mut values = tail
        .windows(2)
        .filter(|pair| pair[0] == "--client")
        .map(|pair| pair[1].as_str());
    let single = values.next().filter(|_| values.next().is_none());
    if let Some(client) = single.filter(|client| settings_menu::is_client_name(client))
        && transport::observe_menu_client(server, client).is_some()
    {
        let _ = transport::display_client_message(
            server,
            client,
            "the quota dialog invocation was refused; nothing was drawn",
        );
    }
    let _ = writeln!(err, "ae quota: {why}.");
}

/// `ae orchestrator --quota-dialog --client … --client-pid … --server-pid … --server-start … --session-id …`
/// — one centred observational dialog listing every quota window per client scope.
///
/// The continuation carries the identity the settings entry captured. The client
/// and server are observed once to BUILD under (overlay, dimensions, budget) and
/// reproven by [`prove_quota_dialog_clicker`] IMMEDIATELY BEFORE THE DRAW. The
/// dialog refuses a client or server it can prove has been replaced, and a client
/// that switched session reads no overlay. The dialog writes nothing, so no
/// deadline travels with it. No marker is set.
fn run_quota_dialog(
    server: &inventory::ServerId,
    tail: &[String],
    menu_mouse: bool,
    err: &mut impl Write,
) -> u8 {
    let captured = match settings_menu::parse_quota_dialog(tail) {
        Ok(captured) => captured,
        Err(why) => {
            tell_quota_dialog_parse_refusal(server, tail, err, &why);
            return entry::EXIT_USAGE;
        }
    };
    let client_name = captured.client.clone();
    // BUILD round: resolve the client and server the scan reads under. This
    // proves nothing about the draw; the proof runs after the scan.
    let Some(client) = transport::observe_menu_client(server, &client_name) else {
        let _ = writeln!(
            err,
            "ae quota: the invoking client vanished before ae could build the dialog"
        );
        return EXIT_UNAVAILABLE;
    };
    let Some(identity) = transport::observe_server_identity(server) else {
        let _ = writeln!(
            err,
            "ae quota: the invoking tmux server did not answer with its identity"
        );
        return EXIT_UNAVAILABLE;
    };
    let report = |text: &str, err: &mut dyn Write| {
        let server_survives = transport::observe_server_identity(server)
            .is_some_and(|live| live.pid == identity.pid && live.start == identity.start);
        let client_survives = transport::observe_menu_client(server, &client_name)
            .is_some_and(|live| live.pid == client.pid);
        if server_survives && client_survives {
            let _ = transport::display_client_message(server, &client_name, text);
        }
        let _ = writeln!(err, "ae quota: {text}");
    };
    let Some(root) = doors::state_root(shape::current()) else {
        report(NO_STATE_ROOT, err);
        return EXIT_UNAVAILABLE;
    };
    let config = doors::config_file(shape::current(), &root);
    let session_dir = lifecycle::sessions_dir(&root).join(&client.session_name);
    let local_meta = session::read_meta(&session_dir).ok();
    let local = local_meta
        .as_ref()
        .and_then(meta::Meta::origin)
        .and_then(|origin| config::local_overlay(&session_dir, origin));
    // The continuation refuses when the invoking session is quota-unaware: the
    // menu carried no entry, so reaching here means a stale or hand-built
    // invocation. `ae quota` stays the full view in both states.
    if !session_quota_aware(&session_dir, Some(&config), local.as_deref()) {
        report(
            "quota awareness is off for this session ([workspace] quota = off), so there is no quota dialog",
            err,
        );
        return EXIT_UNAVAILABLE;
    }
    let home = doors::home();
    let roots = inventory::Roots::under(&root);
    let now = time::Timestamp::now().epoch();
    let scopes = quota::view_of(&quota::Inputs {
        home: home.as_deref(),
        global: Some(&config),
        local: local.as_deref(),
        sessions: Some(roots.sessions()),
        now,
    });
    let look = picker_look(server, Some(&client.session_id));
    // PROOF round, immediately before the draw: the scan above cannot
    // interleave a replacement past this point. A refusal reports through the
    // same owner as every other dialog failure, so the invoking client reads
    // why nothing drew instead of a bare exit code: the menu row runs
    // backgrounded, where stderr never reaches a human. The texts name no
    // session and no quota fact, so telling a switched client is not a leak;
    // a vanished or replaced clicker still hears nothing, because there is no
    // proven occupant left to tell.
    let live = match prove_quota_dialog_clicker(server, &captured) {
        Ok(live) => live,
        Err(why) => {
            report(&why, err);
            return EXIT_UNAVAILABLE;
        }
    };
    // Lay the facts out against the LIVE dimensions from the proof round;
    // what still refuses — too narrow, or below the floor — reports through
    // the same owner, quoting the FULL menu's budget. The refusal keeps its
    // exit code after the tell, as every menu continuation does: a hand-run is
    // indistinguishable from the menu row and reads the same code.
    let menu = match settings_menu::quota_dialog_menu(
        &scopes,
        &look.palette,
        look.icons,
        live.width,
        live.height,
    ) {
        Ok(menu) => menu,
        Err((columns, lines)) => {
            report(
                &format!(
                    "this terminal is {}x{}; quota needs {columns}x{lines}",
                    live.width, live.height
                ),
                err,
            );
            return EXIT_UNAVAILABLE;
        }
    };
    if !transport::display_menu_centred(server, &client_name, &live.session_id, &menu, menu_mouse) {
        report(
            "tmux refused to draw quota (the client may have vanished)",
            err,
        );
        return EXIT_UNAVAILABLE;
    }
    0
}

/// Draw a menu whose transient marker is already published. A failed draw
/// retracts exactly the marker owned by this attempt.
fn draw_marked_menu(draw: impl FnOnce() -> bool, clear_own_marker: impl FnOnce() -> bool) -> bool {
    if draw() {
        true
    } else {
        let _ = clear_own_marker();
        false
    }
}

/// Refresh the fleet-picker marker whenever a menu opens.
fn mark_picker_open(server: &inventory::ServerId, session_id: &str) -> bool {
    replace_menu_marker(
        server,
        session_id,
        theme::SETTINGS_OPEN_OPTION,
        theme::MENU_OPEN_OPTION,
    )
}

fn mark_settings_open(server: &inventory::ServerId, session_id: &str) -> bool {
    replace_menu_marker(
        server,
        session_id,
        theme::MENU_OPEN_OPTION,
        theme::SETTINGS_OPEN_OPTION,
    )
}

fn replace_menu_marker(
    server: &inventory::ServerId,
    session_id: &str,
    remove: &str,
    set: &str,
) -> bool {
    transport::replace_session_option(
        server,
        session_id,
        remove,
        set,
        &crate::time::Timestamp::now().epoch().to_string(),
    )
}

fn clear_menu_marker(server: &inventory::ServerId, session_id: &str, option: &str) -> bool {
    transport::clear_option(server, tmux::OptionScope::Session, session_id, option)
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

/// The picker's stopped-row `Discovery`: it answers ONLY the server that
/// opened the menu, and only from reads the draw already paid for.
///
/// The name set is the calling server's own bare-name listing; the marker is
/// ae's rank row for that exact name, which is the same ownership fact every
/// running picker row is admitted on. A name that IS there without that fact
/// is `unknown` — never `stopped` — and every other recorded server is a
/// failed query, so a session elsewhere cannot claim a stopped row here.
struct PickerStoppedBackend<'a> {
    server: &'a inventory::ServerId,
    names: Option<&'a [String]>,
    live: &'a [tmux::PickerSession],
    sockets: &'a fleet_read::SocketPaths,
}

impl inventory::Discovery for PickerStoppedBackend<'_> {
    fn enumerate(
        &self,
        server: &inventory::ServerId,
    ) -> std::result::Result<Vec<inventory::DiscoveredSession>, inventory::QueryFailed> {
        if !self.sockets.equivalent(server, self.server) {
            return Err(inventory::QueryFailed);
        }
        let Some(names) = self.names else {
            return Err(inventory::QueryFailed);
        };
        Ok(names
            .iter()
            .map(|name| inventory::DiscoveredSession {
                name: name.clone(),
                marker: self
                    .live
                    .iter()
                    .any(|row| row.name == *name)
                    .then(|| "1".to_owned()),
            })
            .collect())
    }
}

/// The argv prefix that re-executes this core for a picker resume on `server`:
/// the picker's stopped rows and `ae app`'s Start share it. `None` when this
/// core or its state root cannot be named.
pub(crate) fn picker_launcher_for(server: &inventory::ServerId) -> Option<Vec<String>> {
    let root = doors::state_root(shape::current())?;
    let core = shape::resolved_exe()?;
    let config = doors::config_file(shape::current(), &root);
    Some(session_tmux::picker_launcher(
        shape::current(),
        &core,
        &root,
        &config,
        server,
    ))
}

/// The stopped sessions the calling server PROVES, as picker rows.
///
/// One meta-only durable scan, fed through the one classifier with a backend
/// that knows only the calling server: every row here is a candidate `ae list
/// --all` would also read `stopped`, and every gap — another recorded server, a
/// missing or ambiguous selector, a same-named live session whose ownership
/// cannot be proven — stays `unknown` and is simply not a row. The scan never
/// opens a session's event journal: name, goal and branch are meta facts.
fn picker_stopped<D: inventory::Discovery + ?Sized>(
    scan: inventory::DurableScan,
    backend: &D,
    live: &[tmux::PickerSession],
) -> Vec<orchestrator::PickerStopped> {
    let launch_evidence = scan.launch_evidence;
    let taken = inventory::Inventory {
        candidates: scan
            .records
            .into_iter()
            .map(inventory::Candidate::durable)
            .collect(),
        incomplete: scan.incomplete,
    };
    let mut rows: Vec<orchestrator::PickerStopped> = liveness::classify(taken, backend)
        .sessions
        .into_iter()
        .filter(|classified| classified.status == digest::Status::Stopped)
        .filter_map(|classified| classified.candidate.durable)
        .filter(|record| crate::session_launch::name::is_session_name(&record.name))
        .filter(|record| !live.iter().any(|row| row.name == record.name))
        .map(|record| {
            // The picker's read never opened the journal: goal and branch are
            // meta facts, so a stopped row costs its meta file and its stamps,
            // never the session's communication history.
            let (goal, branch) = session::stopped_display(&record.snapshot);
            // The meta half of the sign of life comes from the SCAN's one read
            // of this record's meta, so a draw never re-opens the file.
            let meta = launch_evidence
                .iter()
                .find(|(path, _)| *path == record.path)
                .map_or(tmux::Evidence::Unreadable, |(_, evidence)| *evidence);
            let last_live = match inventory::last_live_with_meta_evidence(&record.path, meta) {
                tmux::Evidence::At(epoch) => Some(epoch),
                tmux::Evidence::Silent | tmux::Evidence::Unreadable => None,
            };
            orchestrator::PickerStopped {
                name: record.name.clone(),
                goal,
                branch,
                last_live,
            }
        })
        .collect();
    sort_stopped_rows(&mut rows);
    rows
}

/// Most recently live first, then name: a row with no readable moment sorts
/// after every dated one rather than guessing at one.
fn sort_stopped_rows(rows: &mut [orchestrator::PickerStopped]) {
    rows.sort_by(|left, right| {
        right
            .last_live
            .cmp(&left.last_live)
            .then_with(|| left.name.cmp(&right.name))
    });
}

/// The two words of the orchestrator's own session that run before the entry
/// router: the settings menu's apply, then the resume. Each proves its caller
/// first, and the orchestrator seat itself is refused. `None` when `argv` is
/// neither word.
fn run_orchestrator_word(
    preamble: &entry::Preamble,
    argv: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<Option<u8>> {
    if argv.first().map(String::as_str) != Some(orchestrator::ORCHESTRATOR_SESSION) {
        return Ok(None);
    }
    let tail = &argv[1..];
    let settings = settings_menu::is_apply(tail);
    if !settings && !orchestrator::is_resume(tail) {
        return Ok(None);
    }
    let refusal = if settings {
        (
            "settings",
            "never changes, starts, resumes or stops its own seat; the owner runs ae orchestrator",
        )
    } else {
        (
            "launch",
            "never starts or resumes its own seat; the owner runs ae orchestrator",
        )
    };
    let who = match launch_caller(
        observe_calling_pane(),
        state_root().as_deref(),
        refusal,
        err,
    )? {
        ControlFlow::Continue(who) => who,
        ControlFlow::Break(code) => return Ok(Some(code)),
    };
    let code = if settings {
        settings_menu::run_apply(preamble, tail, &who, out, err)?
    } else {
        orchestrator::run_resume(preamble, tail, &who, out, err)?
    };
    Ok(Some(code))
}

/// The human route: what `ae` itself answers, once the doors have said what
/// this invocation carries.
fn run_entry(
    preamble: &entry::Preamble,
    argv: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    if let Some(code) = run_orchestrator_word(preamble, argv, out, err)? {
        return Ok(code);
    }
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
        entry::Route::Compact(tail) => {
            let compact = Outbound::Verb(
                "compact",
                "does not compact seats in this release; give the owner the command",
            );
            if let Some(code) = refuse_dispatcher(compact, err)? {
                return Ok(code);
            }
            return run_compact(preamble, &tail, out, err);
        }
        entry::Route::Attach => return run_bare_attach(preamble, Bare::App, out, err),
        entry::Route::BareDetached => return run_bare_attach(preamble, Bare::Detached, out, err),
        entry::Route::BareTmux => return run_bare_attach(preamble, Bare::Tmux, out, err),
        entry::Route::TmuxNoAttach => {
            write!(err, "{}", entry::TMUX_NO_ATTACH)?;
            entry::EXIT_USAGE
        }
        entry::Route::Launch(user) => {
            // The bound resume of a stopped session other than the seat, which
            // the launch owner proves under the target's lifecycle lock, runs
            // for every caller; the proved seat's is audited.
            if session_launch::is_bound_resume(&user) {
                return run_bound_resume(preamble, &user, out, err);
            }
            let who = match run_seat_launch_gate(preamble, &user, out, err)? {
                ControlFlow::Continue(who) => who,
                ControlFlow::Break(code) => return Ok(code),
            };
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
                if flags.autostart {
                    seat.starter = session_launch::Starter::Autostart;
                }
                let route = session_launch::LaunchRoute::OrchestratorStart;
                return run_launch(&seat, &launch, false, None, (&who, route), out, err);
            }
            let route = session_launch::LaunchRoute::Entry;
            return run_launch(preamble, &user, true, None, (&who, route), out, err);
        }
    };
    err.flush()?;
    error::with_status(code, out.flush())?;
    Ok(code)
}

/// Which bare form was typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bare {
    /// `ae`: the app when both ends are a terminal, else the tmux view.
    App,
    /// `ae --tmux`: the tmux view, never the app.
    Tmux,
    /// `ae --no-attach`: the tmux view's attach command, printed.
    Detached,
}

/// Bare `ae`: on a terminal, open the app over the recorded sessions — from
/// outside tmux, or from a pane of ae's own server that is no seat. Otherwise
/// list when already on the launch server, print the cross-server attach hint
/// from another tmux, or attach this terminal and let tmux choose its most
/// recent session. [`Bare::Detached`] only names the command.
fn run_bare_attach(
    preamble: &entry::Preamble,
    bare: Bare,
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
    let attach = bare != Bare::Detached;
    let app = bare == Bare::App && both_terminal();
    if preamble.inside_tmux {
        if on_launch_server(preamble, server) {
            // The pane is read for the app alone.
            if app && caller_pane_lands(preamble) {
                out.flush()?;
                err.flush()?;
                return run_dispatch(&own_app_argv(None), out, err);
            }
            return run_dispatch(&["list".to_owned()], out, err);
        }
        writeln!(
            out,
            "ae: the ae fleet is on another tmux server. Attach with: {}",
            session_launch::server_attach_hint(server)
        )?;
        return Ok(0);
    }
    let verdicts = match config::global_restore(preamble.global.as_deref()) {
        config::Restore::Off => Vec::new(),
        ruling => {
            if let config::Restore::Unusable(why) = &ruling {
                writeln!(err, "ae: restore: {why}; restoring as on")?;
            }
            restore::restorable(&inventory::Roots::under(&preamble.home))
        }
    };
    let mut restored = restore::Report::default();
    if verdicts.iter().any(|verdict| {
        matches!(
            verdict.decision,
            restore::Decision::Restore | restore::Decision::Unproven
        )
    }) {
        if verdicts.iter().any(|verdict| verdict.guard.is_some()) {
            let deps = doctor::check_deps(&[], err)?;
            if deps != 0 {
                err.flush()?;
                return Ok(deps);
            }
        }
        restored = restore::run(preamble, &verdicts, out, err)?;
    }
    let failed = u8::from(restored.failed > 0);
    // AFTER the restore, so a restored session is drawn running, and BEFORE the
    // server is asked, so a fleet that is all stopped still lands.
    if app && !current_world(&preamble.home).1.sessions.is_empty() {
        err.flush()?;
        error::with_status(failed, out.flush())?;
        return match run_dispatch(&own_app_argv(None), out, err) {
            Ok(code) => Ok(code.max(failed)),
            Err(Error::Io(why)) => error::with_status(failed, Err(why)).map_err(Error::from),
        };
    }
    // No recorded session on a terminal: the first use, whatever else the
    // server holds.
    if app && let Some(code) = run_first_use(preamble, out, err)? {
        return Ok(code.max(failed));
    }
    if let Some(names) = transport::session_names(server).filter(|names| !names.is_empty()) {
        if attach {
            // Outside tmux on a terminal: the owner's own `ae` or `ae --tmux`.
            return Ok(kick_then_attach(preamble, server, &names, err)?.max(failed));
        }
        error::with_status(
            failed,
            writeln!(
                out,
                "ae: attach with: {}",
                session_launch::server_attach_hint(server)
            ),
        )?;
        return Ok(failed);
    }
    if restored.restored > 0 {
        writeln!(
            err,
            "ae: restored sessions live on another tmux server; see: ae list"
        )?;
        return Ok(failed);
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

/// Bare `ae`'s orchestrator autostart, then its untargeted attach. When this
/// invocation ran the start and the seat was not there before it, the seat
/// may now be the newest session on the server, so tmux's own pick could
/// land on it: the target is then named instead — tmux's pick among the
/// others, read after the start and attached by exact name. With no such
/// session proven (none left, or the listing failed) nothing is attached, and
/// a target that closes before the attach fails that attach; neither falls
/// back to tmux's untargeted pick. Otherwise tmux chooses, as it always did.
/// `names` is the listing taken before the kick.
fn kick_then_attach(
    preamble: &entry::Preamble,
    server: &inventory::ServerId,
    names: &[String],
    err: &mut impl Write,
) -> Result<u8> {
    let seat = orchestrator::ORCHESTRATOR_SESSION;
    let had_seat = names.iter().any(|name| name == seat);
    let started = kick_orchestrator(preamble, true, err);
    if had_seat || !started {
        return Ok(transport::attach(server));
    }
    if let Some(name) = transport::untargeted_attach_choice(server, seat) {
        return Ok(transport::focus(
            server,
            tmux::FocusVerb::AttachSession,
            &name,
        ));
    }
    writeln!(
        err,
        "ae: found no session to attach besides the orchestrator; attach it with: ae {seat}"
    )?;
    Ok(entry::EXIT_FAILED)
}

/// The orchestrator autostart for a route that has already decided what it
/// does, waited for, its one line on `err`. Only for a human — `human` from
/// the caller the route observed, and both streams a terminal — and at most
/// once per invocation. The app kicks for itself ([`app::run`]). The line is
/// best-effort: a stream that cannot take it never fails the route. Whether
/// the start ran.
fn kick_orchestrator(preamble: &entry::Preamble, human: bool, err: &mut impl Write) -> bool {
    if !human || !both_terminal() || !orchestrator_autostart::claim_once() {
        return false;
    }
    let kick = orchestrator_autostart::Kick {
        global: preamble.global.clone(),
        no_autostart: preamble.no_autostart,
    };
    let (started, line) = match orchestrator_autostart::verdict(&kick) {
        orchestrator_autostart::Verdict::Skip => (false, None),
        orchestrator_autostart::Verdict::Note(line) => (false, Some(line)),
        orchestrator_autostart::Verdict::Start => (true, orchestrator_autostart::start()),
    };
    if let Some(line) = line {
        let _ = writeln!(err, "{line}");
        let _ = err.flush();
    }
    started
}

/// Whether BOTH of this process's streams are a terminal. The app draws on
/// them, so it is a choice only then; `app::run` refuses the rest again itself.
fn both_terminal() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stdin())
        && std::io::IsTerminal::is_terminal(&std::io::stdout())
}

/// Whether the caller's tmux server is proven to be `server`.
fn on_launch_server(preamble: &entry::Preamble, server: &inventory::ServerId) -> bool {
    preamble.caller_server.as_ref().is_some_and(|caller| {
        let mut sockets = fleet_read::SocketPaths::live();
        sockets.proven_same(caller, server)
    })
}

/// The calling pane as tmux describes it, read on the caller's server — which
/// the entry has just proven to be the launch server, so a pane id it answers
/// for is a pane it owns.
fn caller_viewer(preamble: &entry::Preamble) -> Option<tmux::ObservedViewer> {
    preamble
        .caller_server
        .as_ref()
        .zip(doors::calling_pane_id())
        .and_then(|(caller, pane)| transport::observe_viewer(caller, &pane))
}

/// Whether the calling pane may draw the app over the whole fleet: see
/// [`pane_may_land`].
fn caller_pane_lands(preamble: &entry::Preamble) -> bool {
    pane_may_land(caller_viewer(preamble).as_ref(), |name| {
        console::locate(&preamble.home, name).is_some()
    })
}

/// Whether the calling pane is no seat: tmux described it and it carries no
/// `@ae_agent` (a seat or a monitor keeps today's entry). Anything unread
/// fails closed.
fn caller_pane_seatless(preamble: &entry::Preamble) -> bool {
    pane_is_seatless(caller_viewer(preamble).as_ref())
}

/// Whether a described pane carries no `@ae_agent`. A chat pane is stamped
/// `@ae_console`, never `@ae_agent`, so it is no seat. Described means tmux
/// named the pane's session: for a pane id it cannot find, tmux 3.7b still
/// exits 0 and prints every pane field empty.
fn pane_is_seatless(viewer: Option<&tmux::ObservedViewer>) -> bool {
    viewer.is_some_and(|viewer| viewer.session.is_some() && viewer.agent.is_none())
}

/// Whether the calling pane may draw the app with no session named: it is no
/// seat ([`pane_is_seatless`]) and its session is one this state root records
/// — the home `app::run` would otherwise refuse. Anything unread fails closed
/// to the list.
fn pane_may_land(
    viewer: Option<&tmux::ObservedViewer>,
    recorded: impl FnOnce(&str) -> bool,
) -> bool {
    pane_is_seatless(viewer)
        && viewer
            .and_then(|viewer| viewer.session.as_deref())
            .is_some_and(recorded)
}

/// `ae compact [name]` — resolve the target, path-check it, then hand the
/// state root and name to the in-place seat-compaction verb. Dash-led tokens
/// never reach here: the tripwire answered them before the preamble.
fn run_compact(
    preamble: &entry::Preamble,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    if let Some(extra) = tail.get(1) {
        let shown = event_text::display_cell(extra, 64);
        writeln!(
            err,
            "ae compact: unexpected extra argument '{shown}' — compact takes one session name."
        )?;
        err.flush()?;
        return Ok(entry::EXIT_USAGE);
    }
    let named = tail
        .first()
        .map(ToOwned::to_owned)
        .or_else(|| current_session_name(preamble));
    let Some(target) = named else {
        write!(err, "{}", entry::COMPACT_USAGE)?;
        err.flush()?;
        return Ok(entry::EXIT_USAGE);
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
    if tracked::refuse_role_entry(&dir, err)? {
        return Ok(state::EXIT_FAILED);
    }
    let code = seatcompact_run::run(&preamble.home, &target, out, err)?;
    err.flush()?;
    error::with_status(code, out.flush())?;
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
    err.flush()?;
    error::with_status(code, out.flush())?;
    Ok(code)
}

/// The launch fall-through: the prelude the glue ran, then `_launch`. `land`
/// lets a named line on a terminal open the app on its session instead of
/// attaching ([`named_lands`]); the orchestrator seat passes false.
/// `first_use` is the directory bare `ae` made the session from: the landing
/// then says so ([`run_first_use`]). `caller` is who the entry observed and
/// the route it came in by.
fn run_launch(
    preamble: &entry::Preamble,
    user: &[String],
    land: bool,
    first_use: Option<&std::path::Path>,
    (who, route): (&session_launch::LaunchCaller, session_launch::LaunchRoute),
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let gate = Gate::Caller(who, route);
    run_launch_as(preamble, user, land, first_use, gate, out, err)
}

/// Which gate a launch's served `_launch` crosses. Each carries the caller
/// the entry observed, so the launch owner judges and journals that ONE
/// observation.
enum Gate<'a> {
    /// An ordinary caller's launch, which the entry admitted past the
    /// orchestrator class gate: [`run_caller_dispatch`].
    Caller(
        &'a session_launch::LaunchCaller,
        session_launch::LaunchRoute,
    ),
    /// The bound resume the entry admitted for every caller:
    /// [`run_bound_dispatch`].
    Bound(&'a session_launch::LaunchCaller),
    /// The proved seat's start, create or bound resume, which the entry
    /// admitted; the audit records it under the launch's lifecycle lock:
    /// [`run_executor_dispatch`].
    Executor(
        &'a mut dyn session_launch::LaunchAudit,
        &'a session_launch::LaunchCaller,
    ),
}

/// G4 of a launch the proved seat runs: the request row under the launch's
/// lifecycle lock, naming the session that lock holds; the result row after,
/// with the uuid the record held under that same lock.
struct SeatLaunch<'a> {
    seat: &'a relay::Dispatcher,
    /// Which of the seat's three launch forms the argv is.
    form: SeatForm,
    /// The session as typed.
    target: String,
    /// The request row, once written.
    written: Option<tracked::Dispatch>,
}

/// The three launch forms the proved seat runs, each with `--no-attach`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SeatForm {
    /// `ae <name> --resume --no-attach --expect-uuid <uuid>`.
    Resume,
    /// `ae <name> --no-attach`: a stopped session that exists.
    Start,
    /// `ae <name> --dir <path> --local|--copy|--worktree [profiles] --no-attach`.
    Create,
}

impl SeatForm {
    /// The word a dispatch and its refusals name.
    const fn verb(self) -> &'static str {
        match self {
            Self::Resume => "resume",
            Self::Start => "start",
            Self::Create => "create",
        }
    }

    /// The form of the parsed `plan` other than the bound resume, or `None`
    /// for a shape the seat does not run (an unbound `--resume`, `--from`,
    /// a frozen worker list, a mode without `--dir` and the like).
    fn of(plan: &session_launch::Plan) -> Option<Self> {
        let bare = plan.attach == Some(false)
            && !plan.resume
            && plan.expect_uuid.is_none()
            && plan.from.is_none()
            && plan.workers.is_none()
            && plan.main.is_none()
            && !plan.tmux;
        if !bare {
            return None;
        }
        match (&plan.dir, &plan.mode) {
            (None, None) if plan.seat_profiles.is_empty() => Some(Self::Start),
            (Some(_), Some(_)) => Some(Self::Create),
            _ => None,
        }
    }
}

impl session_launch::LaunchAudit for SeatLaunch<'_> {
    fn before_lock(
        &mut self,
        session: &str,
        dir: &std::path::Path,
    ) -> std::result::Result<(), String> {
        // A start of a checked name with nothing at all at its state path, no
        // directory, link or file: any other state is judged under the lock.
        if self.form == SeatForm::Start
            && matches!(store::source_presence(dir), store::SourcePresence::Absent)
        {
            return Err(absent_refusal("start", session));
        }
        Ok(())
    }

    fn locked(
        &mut self,
        session: &str,
        dir: &std::path::Path,
        creates: bool,
    ) -> std::result::Result<(), String> {
        let verb = self.form.verb();
        // What the lock holds decides: a create names no session that
        // exists, and a start or resume one that does.
        match (self.form, creates) {
            (SeatForm::Create, false) => {
                return Err(format!(
                    "ae: create REFUSED — '{session}' already exists; the orchestrator seat starts it with: ae {session} --no-attach. Nothing was changed."
                ));
            }
            (SeatForm::Start | SeatForm::Resume, true) => {
                return Err(absent_refusal(verb, session));
            }
            _ => {}
        }
        let uuid = if creates {
            String::new()
        } else {
            session_uuid(dir)
        };
        if !creates && uuid.is_empty() {
            return Err(format!(
                "ae: {verb} REFUSED — the record of '{session}' names no single canonical session uuid, so the orchestrator's dispatch cannot name its target. Nothing was changed."
            ));
        }
        if !creates && uuid == self.seat.uuid {
            return Err(format!(
                "ae: {verb} REFUSED — '{session}' records the orchestrator seat's own session uuid, and the seat never starts its own session. Nothing was changed."
            ));
        }
        let dispatch = seat_dispatch(self.seat, verb, &self.target, session, &uuid);
        dispatch.request().map_err(|why| {
            format!(
                "ae: {verb} REFUSED — ae could not record the dispatch in the orchestrator's journal ({why}). Nothing was changed."
            )
        })?;
        self.written = Some(dispatch);
        Ok(())
    }

    fn built(&mut self, uuid: &str) {
        if let Some(dispatch) = self.written.as_mut()
            && dispatch.target_uuid.is_empty()
        {
            uuid.clone_into(&mut dispatch.target_uuid);
        }
    }

    fn reference(&self) -> String {
        self.written
            .as_ref()
            .map(|dispatch| dispatch.reference.clone())
            .unwrap_or_default()
    }
}

impl SeatLaunch<'_> {
    /// Run the launch with both streams held, then settle the result row
    /// before anything reaches the caller.
    fn run(
        mut self,
        preamble: &entry::Preamble,
        user: &[String],
        out: &mut impl Write,
        err: &mut impl Write,
    ) -> Result<u8> {
        let (mut held_out, mut held_err) = (Vec::new(), Vec::new());
        let who = session_launch::LaunchCaller::Orchestrator(self.seat.clone());
        let code = run_launch_as(
            preamble,
            user,
            false,
            None,
            Gate::Executor(&mut self, &who),
            &mut held_out,
            &mut held_err,
        );
        finish_dispatch(self.written, code, &held_out, held_err, out, err)
    }
}

/// The bound resume, which runs for every caller. The proved seat's is
/// audited; an unread pane and a pane of the seat's session that fails the
/// proof are refused.
fn run_bound_resume(
    preamble: &entry::Preamble,
    user: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let seat = match calling_class(observe_calling_pane(), state_root().as_deref(), err)? {
        Err(code) => return Ok(code),
        Ok(None) => {
            let who = session_launch::LaunchCaller::Outside;
            return run_launch_as(preamble, user, false, None, Gate::Bound(&who), out, err);
        }
        Ok(Some(relay::SeatClass::Ordinary(identity))) => {
            let who = session_launch::LaunchCaller::of(identity);
            return run_launch_as(preamble, user, false, None, Gate::Bound(&who), out, err);
        }
        Ok(Some(relay::SeatClass::Unproven(why))) => {
            writeln!(err, "{}", unproven_refusal("launch", why))?;
            err.flush()?;
            return Ok(entry::EXIT_USAGE);
        }
        Ok(Some(relay::SeatClass::Dispatcher(seat))) => seat,
    };
    let target = session_launch::parse_plan(user)
        .ok()
        .and_then(|plan| plan.name)
        .unwrap_or_default();
    SeatLaunch {
        seat: &seat,
        form: SeatForm::Resume,
        target,
        written: None,
    }
    .run(preamble, user, out, err)
}

/// The launch caller gate: the proved seat's launch runs here, and a pane in
/// the seat's session that ae cannot prove to be the seat is refused. Every
/// other caller goes on with the class the same ONE observation gives it,
/// which the launch owner's seat guard judges under the target's lock.
fn run_seat_launch_gate(
    preamble: &entry::Preamble,
    user: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<ControlFlow<u8, session_launch::LaunchCaller>> {
    match calling_class(observe_calling_pane(), state_root().as_deref(), err)? {
        Err(code) => Ok(ControlFlow::Break(code)),
        Ok(Some(relay::SeatClass::Dispatcher(seat))) => {
            run_seat_launch(preamble, user, &seat, out, err).map(ControlFlow::Break)
        }
        Ok(Some(relay::SeatClass::Unproven(why))) => {
            writeln!(err, "{}", unproven_refusal("launch", why))?;
            err.flush()?;
            Ok(ControlFlow::Break(entry::EXIT_USAGE))
        }
        Ok(None) => Ok(ControlFlow::Continue(session_launch::LaunchCaller::Outside)),
        Ok(Some(relay::SeatClass::Ordinary(identity))) => Ok(ControlFlow::Continue(
            session_launch::LaunchCaller::of(identity),
        )),
    }
}

/// The caller of a launch form outside the entry's launch line (the served
/// `_launch`, the settings apply, the picker resume), from ONE observation:
/// the owner outside tmux, an ordinary pane or agent seat, or the break code
/// once the refusal of `word` is said to a pane of the orchestrator seat's
/// session, with `why` as the seat's clause.
fn launch_caller(
    seen: CallerRead<(tmux::ObservedViewer, inventory::ServerId)>,
    root: Option<&std::path::Path>,
    (word, why): (&'static str, &'static str),
    err: &mut impl Write,
) -> Result<ControlFlow<u8, session_launch::LaunchCaller>> {
    let said = match calling_class(seen, root, err)? {
        Err(code) => return Ok(ControlFlow::Break(code)),
        Ok(None) => return Ok(ControlFlow::Continue(session_launch::LaunchCaller::Outside)),
        Ok(Some(relay::SeatClass::Ordinary(identity))) => {
            return Ok(ControlFlow::Continue(session_launch::LaunchCaller::of(
                identity,
            )));
        }
        Ok(Some(relay::SeatClass::Unproven(reason))) => unproven_refusal(word, reason),
        Ok(Some(relay::SeatClass::Dispatcher(_))) => seat_verb_refusal(word, why),
    };
    writeln!(err, "{said}")?;
    err.flush()?;
    Ok(ControlFlow::Break(entry::EXIT_USAGE))
}

/// The served `_launch`, wherever the dispatch meets it: its caller from ONE
/// observation ([`launch_caller`]), then the launch owner, which judges that
/// caller.
fn run_served_launch(tail: &[String], out: &mut impl Write, err: &mut impl Write) -> Result<u8> {
    let seen = observe_calling_pane();
    let launch = ("launch", SEAT_LAUNCH_ONLY);
    match launch_caller(seen, state_root().as_deref(), launch, err)? {
        ControlFlow::Continue(who) => {
            let route = session_launch::LaunchRoute::Served;
            session_launch::run(tail, &who, route, out, err)
        }
        ControlFlow::Break(code) => Ok(code),
    }
}

/// The proved seat's start or create of another session, from the entry:
/// `--no-attach` only, so no client moves, never the seat's own session, and
/// audited under the launch's lifecycle lock (G4).
fn run_seat_launch(
    preamble: &entry::Preamble,
    user: &[String],
    seat: &relay::Dispatcher,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    // A malformed or nameless line is none of the seat's forms: its typed
    // refusal, before any gate and before any write.
    let plan = match session_launch::parse_seat_plan(user) {
        Ok(plan) if plan.name.is_some() => plan,
        parsed => {
            let why = parsed
                .err()
                .unwrap_or_else(|| "it names no session".to_owned());
            let why = why
                .trim()
                .trim_start_matches("Error: ")
                .trim_end_matches('.');
            writeln!(
                err,
                "{}",
                seat_forms_refusal("<name>", &format!(" ({why})"))
            )?;
            err.flush()?;
            return Ok(entry::EXIT_USAGE);
        }
    };
    let form = SeatForm::of(&plan);
    let name = plan.name.unwrap_or_default();
    let refusal = if plan.attach != Some(false) {
        "ae: launch REFUSED — the orchestrator seat starts and creates other sessions only with --no-attach, so no client moves. Nothing was changed.".to_owned()
    } else if name == seat.session || name == orchestrator::ORCHESTRATOR_SESSION {
        format!(
            "ae: launch REFUSED — '{name}' is the orchestrator seat's own session, and the seat never starts it. The owner runs, in the owner's own terminal: ae orchestrator. Nothing was changed."
        )
    } else if let Some(form) = form {
        return SeatLaunch {
            seat,
            form,
            target: name,
            written: None,
        }
        .run(preamble, user, out, err);
    } else {
        seat_forms_refusal(&name, "")
    };
    writeln!(err, "{refusal}")?;
    err.flush()?;
    Ok(entry::EXIT_USAGE)
}

/// The refusal of a launch line that is none of the seat's three forms, for
/// session `name`, with the parser's reason in `why` when it has one.
fn seat_forms_refusal(name: &str, why: &str) -> String {
    format!(
        "ae: launch REFUSED — the orchestrator seat runs three launch forms only: ae {name} --no-attach (start), ae {name} --dir <path> --local|--copy|--worktree [--lead|--colead|--seat <profile>] --no-attach (create), and ae {name} --resume --no-attach --expect-uuid <uuid> (resume); this line is none of them{why}. Nothing was changed."
    )
}

/// The refusal of the seat's `verb` (start or resume) of `session`, which does
/// not exist.
fn absent_refusal(verb: &str, session: &str) -> String {
    format!(
        "ae: {verb} REFUSED — no session '{session}' exists to {verb}; the orchestrator seat creates one with: ae {session} --dir <path> --local --no-attach. Nothing was changed."
    )
}

/// The canonical session uuid of the one `session_id` row the meta in `dir`
/// records; empty when it records none, more than one or no valid one.
fn session_uuid(dir: &std::path::Path) -> String {
    meta::read_bytes(dir)
        .ok()
        .and_then(|bytes| {
            meta::sole_value(&bytes, "session_id")
                .and_then(|raw| std::str::from_utf8(raw).ok())
                .map(archive::canonical_uuid)
        })
        .unwrap_or_default()
}

/// A dispatch of the proved seat `seat`: `verb` on `target`, in session
/// `target_session` whose uuid is `target_uuid` (empty before a create).
fn seat_dispatch(
    seat: &relay::Dispatcher,
    verb: &'static str,
    target: &str,
    target_session: &str,
    target_uuid: &str,
) -> tracked::Dispatch {
    tracked::Dispatch {
        dir: seat.dir.clone(),
        session: seat.session.clone(),
        uuid: seat.uuid.clone(),
        slot: seat.slot.clone(),
        pane: seat.pane.clone(),
        server: seat.server.clone(),
        reference: tracked::Dispatch::reference(time::Timestamp::now(), entropy()),
        verb,
        target: target.to_owned(),
        target_session: target_session.to_owned(),
        target_uuid: target_uuid.to_owned(),
        owner_input: None,
        requested: false,
    }
}

/// The end of a seat dispatch whose streams were held: the result row when
/// its request row was written, then the streams, so a closed stream never
/// skips the result.
fn finish_dispatch(
    written: Option<tracked::Dispatch>,
    code: Result<u8>,
    held_out: &[u8],
    mut held_err: Vec<u8>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    if let Some(dispatch) = written {
        let outcome = match &code {
            Ok(code) => tracked::dispatch_outcome(*code),
            Err(_) => "failed (error)".to_owned(),
        };
        if let Err(why) = dispatch.result(&outcome) {
            writeln!(
                held_err,
                "Warning: ae could not record the dispatch result in the orchestrator's journal ({why})."
            )?;
        }
    }
    let shown = out.write_all(held_out).and_then(|()| out.flush());
    err.write_all(&held_err)?;
    err.flush()?;
    let code = code?;
    error::with_status(code, shown)?;
    Ok(code)
}

/// [`run_launch`], with the gate its served `_launch` crosses.
#[allow(
    clippy::too_many_arguments,
    reason = "run_launch's inputs plus the one gate word"
)]
fn run_launch_as(
    preamble: &entry::Preamble,
    user: &[String],
    land: bool,
    first_use: Option<&std::path::Path>,
    gate: Gate<'_>,
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
    if plan.tmux && plan.attach == Some(false) {
        write!(err, "{}", entry::TMUX_NO_ATTACH)?;
        err.flush()?;
        return Ok(entry::EXIT_USAGE);
    }
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
    if session_name_usable(preamble, hint)
        && !session_path_is_safe(preamble, hint)
        && lifecycle::aliases::target(&preamble.sessions().join(hint)).is_none()
    {
        write_unsafe_path(&preamble.sessions().join(hint), err)?;
        err.flush()?;
        return Ok(entry::EXIT_FAILED);
    }
    if land
        && named_lands(preamble, &plan)
        && let Gate::Caller(who, route) = gate
    {
        return run_named_landing(preamble, &plan, user, first_use, (who, route), out, err);
    }
    if let Gate::Caller(who, session_launch::LaunchRoute::Entry) = &gate {
        let human = matches!(
            who,
            session_launch::LaunchCaller::Outside | session_launch::LaunchCaller::Pane(_)
        );
        kick_orchestrator(preamble, human, err);
    }
    match gate {
        Gate::Caller(who, route) => {
            run_caller_dispatch(&preamble.launch_argv(user), (who, route), out, err)
        }
        Gate::Bound(who) => run_bound_dispatch(&preamble.launch_argv(user), who, out, err),
        Gate::Executor(audit, who) => {
            run_executor_dispatch(&preamble.launch_argv(user), audit, who, out, err)
        }
    }
}

/// The served `_launch` of a launch the proved seat runs, which the entry
/// admitted: `audit` records it under the lifecycle lock. Anything else goes
/// through [`run_dispatch`] and its gate.
fn run_executor_dispatch(
    args: &[String],
    audit: &mut dyn session_launch::LaunchAudit,
    who: &session_launch::LaunchCaller,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let request = cli::Request::parse(args);
    let cli::Request::Launch { tail } = &request else {
        return run_dispatch(args, out, err);
    };
    if speaks_as_reserved(args) {
        return run_dispatch(args, out, err);
    }
    let code = session_launch::run_audited(tail, audit, who, out, err)?;
    err.flush()?;
    error::with_status(code, out.flush())?;
    Ok(code)
}

/// The argv of an app start ae makes itself, which says no hint.
fn own_app_argv(name: Option<&str>) -> Vec<String> {
    let session = name.into_iter().map(ToOwned::to_owned);
    ["app".to_owned()]
        .into_iter()
        .chain(session)
        .chain([app::OWN.to_owned()])
        .collect()
}

/// Whether a named line opens the app: a name it may land on, no `--tmux` and
/// no `--no-attach`, both streams a terminal, and the caller outside tmux or on
/// a pane of the launch server that is no seat. The app's home is the NAMED
/// session, so the caller's own session need not be one ae records.
fn named_lands(preamble: &entry::Preamble, plan: &session_launch::Plan) -> bool {
    if !plan.name.as_deref().is_some_and(landing_name)
        || plan.tmux
        || plan.attach == Some(false)
        || !both_terminal()
    {
        return false;
    }
    if !preamble.inside_tmux {
        return true;
    }
    preamble
        .launch_target
        .as_ref()
        .is_some_and(|server| on_launch_server(preamble, server))
        && caller_pane_seatless(preamble)
}

/// Whether `name` may be landed on: the session grammar, so it is a plain
/// direct child of the sessions root before any of its state is read, and
/// neither a word the entry answers nor the orchestrator seat. Every other
/// name keeps today's launch, which refuses it.
fn landing_name(name: &str) -> bool {
    session_launch::name::is_session_name(name)
        && !entry::name_is_routed_verb(name)
        && name != orchestrator::ORCHESTRATOR_SESSION
}

/// `ae <name>` on a terminal: the ONE launch, headless — create from the cwd,
/// resume in the stored working copy, or leave a running session as it is —
/// then the app with `<name>` as its home. An explicit `--dir` that
/// contradicts the recorded origin is refused first, as usage. A first use
/// says what it made, once, between the launch and the app.
fn run_named_landing(
    preamble: &entry::Preamble,
    plan: &session_launch::Plan,
    user: &[String],
    first_use: Option<&std::path::Path>,
    caller: (&session_launch::LaunchCaller, session_launch::LaunchRoute),
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let name = plan.name.as_deref().unwrap_or_default();
    if lifecycle::aliases::target(&preamble.sessions().join(name)).is_none()
        && let Some((recorded, requested)) = plan.dir.as_deref().and_then(|dir| {
            session_launch::recorded_origin_conflict(&preamble.sessions().join(name), dir)
        })
    {
        writeln!(
            err,
            "Error: session '{name}' exists with a different origin; --dir cannot repoint it."
        )?;
        writeln!(err, "       recorded: {recorded}")?;
        writeln!(err, "       requested: {}", requested.display())?;
        writeln!(err, "       Open it with: ae {name}")?;
        err.flush()?;
        return Ok(entry::EXIT_USAGE);
    }
    let mut headless = preamble.clone();
    headless.attach = false;
    let code = run_caller_dispatch(&headless.launch_argv(user), caller, out, err)?;
    if code != 0 {
        return Ok(code);
    }
    if let Some(dir) = first_use {
        writeln!(
            err,
            "first use: created session {name} in {}; old view: ae {name} --tmux",
            dir.display()
        )?;
    }
    out.flush()?;
    err.flush()?;
    run_dispatch(&own_app_argv(Some(name)), out, err)
}

/// Bare `ae` on a terminal with no recorded session: [`run_launch`] of a
/// session named after the canonical cwd ([`entry::first_use_name`]) with that
/// directory as its origin, then the app on it. `None` when the cwd cannot be
/// resolved to a path ae can pass on: today's refusal stands.
fn run_first_use(
    preamble: &entry::Preamble,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<Option<u8>> {
    let Some(dir) = session_launch::canonical_directory(&preamble.cwd) else {
        return Ok(None);
    };
    let Some(path) = dir.to_str() else {
        return Ok(None);
    };
    let base = dir.file_name().unwrap_or_default().to_string_lossy();
    let user = [
        entry::first_use_name(&base),
        "--dir".to_owned(),
        path.to_owned(),
    ];
    // Bare `ae` comes here only outside tmux: the owner's own terminal.
    let who = session_launch::LaunchCaller::Outside;
    let caller = (&who, session_launch::LaunchRoute::Entry);
    run_launch(preamble, &user, true, Some(&dir), caller, out, err).map(Some)
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

/// Answer short-form helper help using argv alone.
fn short_helper_help(args: &[String], out: &mut impl Write) -> Result<Option<u8>> {
    if let shim::Short::Call { helper, tail, .. } = shim::short_form(args)
        && let Some(usage) = shim::help(helper, tail)
    {
        write!(out, "{usage}")?;
        out.flush()?;
        return Ok(Some(0));
    }
    Ok(None)
}

/// `ae @<session> <helper> <args…>`: the short spelling of a session helper.
///
/// `Ok(None)` means the argv carried no marker and the ordinary route still
/// owns it. Everything else is this route's own answer, and the ORDER is the
/// contract: the words are judged first, so a malformed call is an exit-2 usage
/// error that reads no state at all; only then is the state root derived and
/// the target directory classified; only then does the helper run.
///
/// What is NOT here is as much of the point as what is. No pane, no server, no
/// caller identity: the typed session picks the directory a helper acts on and
/// supplies no authority whatsoever, so an identity-bearing helper observes the
/// real caller exactly as it does through the link.
fn short_helper(args: &[String], out: &mut impl Write, err: &mut impl Write) -> Result<Option<u8>> {
    let (session, helper, tail) = match shim::short_form(args) {
        shim::Short::Absent => return Ok(None),
        shim::Short::Usage(refusal) => {
            write!(err, "{refusal}")?;
            err.flush()?;
            return Ok(Some(entry::EXIT_USAGE));
        }
        shim::Short::Call {
            session,
            helper,
            tail,
        } => (session, helper, tail),
    };
    let Some(root) = doors::state_root(shape::current()) else {
        writeln!(err, "ae: {NO_STATE_ROOT}")?;
        err.flush()?;
        return Ok(Some(EXIT_UNAVAILABLE));
    };
    let sessions = root.join("sessions");
    let dir = sessions.join(session);
    // The SAME lstat the launch path guard asks, for the same reason: a symlink
    // of any kind — dangling included — is an escape wearing a valid name, and
    // a helper that followed one would act on another directory entirely.
    match lstat_kind(&dir) {
        Some(PathKind::Directory) => {}
        None => {
            writeln!(
                err,
                "ae: no session '{session}' under {}.",
                sessions.display()
            )?;
            err.flush()?;
            return Ok(Some(EXIT_UNAVAILABLE));
        }
        Some(PathKind::Symlink | PathKind::Other) => {
            write_unsafe_path(&dir, err)?;
            err.flush()?;
            return Ok(Some(EXIT_UNAVAILABLE));
        }
    }
    // The CURRENT core runs it, through the one translation a link uses — not
    // the target's symlink, which would be a second binary to trust.
    run_dispatch(&shim::translate(helper, &dir, tail), out, err).map(Some)
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
    let queued = store::open(dir).append_event(&chat_line(
        time::Timestamp::now(),
        &viewer.display,
        &viewer.slot,
        &text,
    ));
    if let Err(why) = queued {
        writeln!(err, "ae: say: the line was not queued ({why}).")?;
        return Ok(state::EXIT_FAILED);
    }
    write!(out, "{}", say_report(&text, bridge_liveness(dir)))?;
    Ok(0)
}

/// The ONE `chat` record: a line the Telegram bridge forwards, written by
/// `actor`, addressed to no seat.
pub(crate) fn chat_line(ts: time::Timestamp, actor: &str, actor_slot: &str, text: &str) -> String {
    tracked::event_line(&tracked::EventFields {
        ts,
        actor,
        action: "chat",
        target: "",
        reference: "",
        actor_slot,
        actor_session: "",
        target_slot: "",
        target_session: "",
        actor_session_id: "",
        target_session_id: "",
        target_server: "",
        target_pane: "",
        target_session_uuid: "",
        caller_server: "",
        caller_pane: "",
        caller_session_uuid: "",
        identity_gap: "",
        summary: text,
        body_file: "",
    })
}

/// The `say` report: a line that was QUEUED for the bridge, never one that was
/// sent — only the bridge's own forward is a send — plus the one liveness
/// reading when it names a stopped bridge. `None` (tmux did not answer, or the
/// session records no usable server) adds nothing: unknown is not stopped.
fn say_report(text: &str, bridge: Option<bool>) -> String {
    let head: String = text.chars().take(60).collect();
    let ellipsis = if text.chars().count() > 60 { "…" } else { "" };
    let queued = format!("Queued for Telegram bridge (chat): {head}{ellipsis}");
    match bridge {
        Some(false) => format!(
            "{queued}\nThe Telegram bridge is not running (tmux session {}) — nothing goes out \
             until it starts; start it with `ae telegram start`.\n",
            telegram_lifecycle::TMUX_SESSION
        ),
        Some(true) | None => format!("{queued}\n"),
    }
}

/// Whether the bridge that forwards this session's events is running: its own
/// RECORDED server, read the one way the core reads it, and the one existing
/// liveness probe. No usable record yields no reading, never an ambient guess.
fn bridge_liveness(dir: &std::path::Path) -> Option<bool> {
    let bytes = meta::read_bytes(dir).ok()?;
    match meta::Meta::parse(&String::from_utf8_lossy(&bytes)).server_selector() {
        meta::ServerSelector::Positive(selector) => {
            telegram_lifecycle::daemon_running(&inventory::ServerId::Selected(selector))
        }
        meta::ServerSelector::Missing | meta::ServerSelector::Ambiguous => None,
    }
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
            // A wait on a seat this session does not hold is a likely typo,
            // not an error: said once, after the record stands.
            let warning = declaration.on.as_ref().and_then(|_| {
                let meta = session::read_meta(dir).ok()?;
                state::unknown_wait_target_warning(&declaration, &own_session(dir), &meta)
            });
            if let Some(warning) = warning {
                writeln!(err, "{warning}")?;
            }
            Ok(0)
        }
        Err(failure) => {
            if matches!(failure, state::Failure::SpawnedWaitingUser) {
                write!(err, "{}", failure.message())?;
                return Ok(state::EXIT_USAGE);
            }
            writeln!(err, "{}", failure.message())?;
            Ok(state::EXIT_FAILED)
        }
    }
}

/// The `next`/`jump` arm — frozen `cmd_next`, over the world `list` renders.
/// `ae brief [session] [--all] [--since <dur>] [--full]` — the cards, most actionable
/// first.
///
/// The impure half of the command, and deliberately thin: it resolves WHICH
/// sessions the argv names and hands each one to [`brief::card_for`], which owns
/// every read of a session directory. The one thing asked of the outside world
/// beyond that is git's opinion of each live work tree, through the same
/// [`git::work_tree_dirty`] the watchdog's branch marker uses — so a dirty card
/// and a dirty status line can never disagree. `--full` asks the same question
/// through [`git::work_tree_dirty_read_only`], which adds git's
/// `--no-optional-locks` so that read never refreshes or rewrites the index.
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
    // A seed pack is ONE seat of ONE session, so the caller-unknown fallback to
    // the whole fleet is refused here rather than silently packing something
    // else. Every other refusal below is the card path's own, reused.
    if args.seat.is_some() && named.is_none() {
        writeln!(
            err,
            "ae brief: --seat needs one session: name it, or run from a pane inside one"
        )?;
        return Ok(entry::EXIT_USAGE);
    }
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
    let sessions = inventory::Roots::under(&root);
    let home = doors::home();
    if let (Some(seat), Some(entry)) = (args.seat.as_deref(), selected.first()) {
        return run_seat_pack(
            entry,
            &sessions.sessions().join(&entry.name),
            home.as_deref(),
            world.now,
            seat,
            out,
            err,
        );
    }
    if selected.is_empty() {
        write!(err, "{}", brief::NOTHING)?;
        return Ok(0);
    }
    let rendered = brief_cards(
        &args,
        &selected,
        sessions.sessions(),
        home.as_deref(),
        world.now,
    );
    write!(out, "{rendered}")?;
    Ok(0)
}

/// The cards of `selected`, rendered: the default card, or `--full`'s with nothing
/// clipped. The one place the two paths split, so each reads a session directory
/// through its own function and neither borrows the other's result.
fn brief_cards(
    args: &brief::Args,
    selected: &[&digest::SessionEntry],
    sessions: &std::path::Path,
    home: Option<&std::path::Path>,
    now: time::Timestamp,
) -> String {
    let dirty = |entry: &digest::SessionEntry, probe: fn(&[u8]) -> bool| {
        brief::wants_git(entry)
            && entry
                .work_dir
                .as_deref()
                .is_some_and(|dir| probe(dir.as_bytes()))
    };
    if args.full {
        let cards = selected
            .iter()
            .map(|entry| {
                brief::full_card_for(
                    entry,
                    &sessions.join(&entry.name),
                    home,
                    dirty(entry, git::work_tree_dirty_read_only),
                    now,
                    args.since_secs,
                )
            })
            .collect();
        return brief::render_full(&brief::ordered_full(cards));
    }
    let cards = selected
        .iter()
        .map(|entry| {
            brief::card_for(
                entry,
                &sessions.join(&entry.name),
                home,
                dirty(entry, git::work_tree_dirty),
                now,
                args.since_secs,
            )
        })
        .collect();
    brief::render(&brief::ordered(cards))
}

/// `ae brief <session> --seat <agent>` — ONE seat's seed pack, on stdout.
fn run_seat_pack(
    entry: &digest::SessionEntry,
    dir: &std::path::Path,
    home: Option<&std::path::Path>,
    now: time::Timestamp,
    seat: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    match seat_pack(entry, dir, home, now, seat) {
        Ok(pack) => {
            write!(out, "{pack}")?;
            Ok(0)
        }
        Err(why) => {
            writeln!(err, "ae brief: {why}")?;
            Ok(EXIT_UNAVAILABLE)
        }
    }
}

/// The pack itself, rendered rather than printed.
///
/// `ae brief --seat` prints it; `ae reseat` hands it to a successor as that
/// seat's first turn. ONE builder, so what a human reads before a move and what
/// the successor is given are the same document.
///
/// The impure half, and the only place the pack's world is read. It writes
/// nothing, sends nothing and touches no tmux, so it answers for a stopped
/// session exactly as it does for a running one.
///
/// The record is read ONCE, through [`session::RecordSnapshot::read`], and that
/// read's OUTCOME is what the renderer is told. This matters because
/// [`store::Store::container`] is quiet: an unreadable journal comes back as an
/// empty `Vec`, and a pack built off it would tell a successor that it owes
/// nobody anything. An absent or unreadable META refuses outright — without a
/// roster there is no seat to pack.
///
/// # Errors
///
/// The refusal text, with no command prefix: a caller wears its own.
pub(crate) fn seat_pack(
    entry: &digest::SessionEntry,
    dir: &std::path::Path,
    home: Option<&std::path::Path>,
    now: time::Timestamp,
    seat: &str,
) -> std::result::Result<String, String> {
    let snapshot = session::RecordSnapshot::read(dir);
    let Some(meta) = snapshot.meta else {
        return Err(format!(
            "{}'s meta is {}, so ae cannot say which seats it has",
            entry.name,
            match snapshot.meta_read {
                session::MetaRead::Absent => "absent",
                _ => "unreadable",
            }
        ));
    };
    let Some(row) = meta.roster().iter().find(|row| row.name == seat) else {
        let roster: Vec<&str> = meta.roster().iter().map(|row| row.name.as_str()).collect();
        return Err(format!(
            "{} has no seat named {seat}; its roster is {}",
            entry.name,
            if roster.is_empty() {
                "empty".to_owned()
            } else {
                roster.join(", ")
            }
        ));
    };

    let journal = if snapshot.events.is_some() {
        seatpack::Journal::Read
    } else {
        seatpack::Journal::Damaged
    };
    let container = match journal {
        seatpack::Journal::Read => store::open(dir).container(),
        // Deliberately NOT read: nothing consumes it, and a distrusted read is
        // one more chance to render damage as absence.
        seatpack::Journal::Damaged => Vec::new(),
    };
    let events = snapshot.events.map(|read| read.events).unwrap_or_default();
    let agents = match journal {
        seatpack::Journal::Read => brief::agent_lines(entry, &container, now),
        seatpack::Journal::Damaged => Vec::new(),
    };
    // A seat holds its work until it is RETIRED, not until its session stops:
    // only a running session with a pane proven gone drops a name here.
    let live: Vec<String> = meta
        .roster()
        .iter()
        .filter(|row| {
            !(entry.status == digest::Status::Running
                && entry
                    .agents
                    .iter()
                    .find(|agent| agent.name == row.name)
                    .and_then(|agent| agent.alive)
                    == Some(false))
        })
        .map(meta::RosterEntry::reference)
        .collect();
    let memo = store::open(dir).memo_bytes();
    let work = entry.work_dir.as_deref().unwrap_or_default();
    let inputs = seatpack::Inputs {
        session: entry.name.clone(),
        session_id: meta::session_id_in(dir),
        installed_head: run::has_installed_pane_head(),
        status: entry.status.as_str().to_owned(),
        goal: entry.goal.clone(),
        now,
        helpers_dir: dir.to_path_buf(),
        seat_name: row.name.clone(),
        seat_slot: row.slot.clone(),
        seat_reference: row.reference(),
        seat_profile: row.profile.clone(),
        seat_tool: row.binary.clone(),
        roster: meta
            .roster()
            .iter()
            .map(|row| seatpack::RosterRow {
                name: row.name.clone(),
                slot: row.slot.clone(),
            })
            .collect(),
        agents,
        live,
        journal,
        closed_ages: seatpack::closed_ages(&container, now),
        container,
        events,
        topics: brief::topic_lines(memo.as_deref().unwrap_or_default(), now, None),
        memo_readable: memo.is_ok(),
        git: seatpack::Git {
            work_dir: entry
                .work_dir
                .as_deref()
                .map(|path| brief::short_path(path, home)),
            branch: git::branch_head(work.as_bytes()),
            head: git::head(work.as_bytes()),
            dirty: git::work_tree_dirty(work.as_bytes()),
            changed: git::tracked_changes(work.as_bytes()),
            subjects: git::recent_subjects(work.as_bytes()),
            tag: git::latest_tag(work.as_bytes()),
        },
        first_message: first_message_for(dir, &row.slot),
        last_turns: last_turns_for(entry, dir, row, home, now),
    };
    Ok(seatpack::pack(&inputs))
}

/// The seat's own last turns, read live from its harness transcript.
///
/// THE one wiring between a pure pack and the board that owns every transcript
/// read — read-only, one generation, one seat. Coverage is carried INTO the
/// pack rather than swallowed here: a seat whose tool has no reader must say
/// so, or a successor reads an empty section as a silent predecessor.
///
/// A board row's stamp is epoch MICROS in its store's own precision; the pack
/// renders ages, so the conversion belongs on this side of the seam and the
/// renderer never reads a clock.
fn last_turns_for(
    entry: &digest::SessionEntry,
    dir: &std::path::Path,
    row: &meta::RosterEntry,
    home: Option<&std::path::Path>,
    now: time::Timestamp,
) -> seatpack::LastTurns {
    let session = usage::SessionInput {
        name: entry.name.clone(),
        path: dir.to_path_buf(),
    };
    let (rows, coverage) = board::observe_seat_turns(&session, row, home);
    seatpack::LastTurns {
        turns: rows
            .iter()
            .map(|row| seatpack::Turn {
                human: row.role == board::Role::Human,
                age_secs: time::Timestamp::from_epoch(row.ts.div_euclid(1_000_000))
                    .seconds_until(now),
                body: row.body.clone(),
            })
            .collect(),
        // The FIRST reason only: one seat's read refuses once, and a pack that
        // listed every generation's would be reporting a chain it did not read.
        gap: coverage.first().map(|item| item.reason.clone()),
    }
}

/// The seat's recorded first message, and the verdict on the brief file it
/// names.
///
/// Only a SPAWNED seat has one: `spawn` is the sole publisher of a prompt file,
/// so asking for any other slot's would invent a gap that is not there. ABSENT
/// and UNREADABLE are kept apart, because a successor told "none recorded"
/// about a file that exists would stop looking.
fn first_message_for(dir: &std::path::Path, slot: &str) -> seatpack::FirstMessage {
    if seatpack::slot_class(slot) != "spawned" {
        return seatpack::FirstMessage::Absent;
    }
    let path = run::prompt_file(dir, slot);
    let prompt_path = path.display().to_string();
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the seat's own recorded first message, inside its own session dir"
    )]
    let read = std::fs::read_to_string(&path);
    match read {
        Ok(text) => {
            let brief_path = seatpack::brief_path_in(&text).map(|named| {
                #[allow(
                    clippy::disallowed_methods,
                    reason = "a door: whether the brief that message names is still on disk — \
                              the one fact a successor cannot derive from the text"
                )]
                let exists = std::path::Path::new(&named).exists();
                (named, exists)
            });
            seatpack::FirstMessage::Recorded {
                prompt_path,
                text,
                brief_path,
            }
        }
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => seatpack::FirstMessage::Absent,
        Err(_) => seatpack::FirstMessage::Unreadable { prompt_path },
    }
}

/// `ae board [session…] [--since <ts>] [--json] [--follow]` — the filtered
/// cross-fleet record, derived on read from harness transcripts.
///
/// The impure half of the command, and deliberately thin: it resolves WHICH
/// sessions the argv names — explicit names straight from durable state,
/// stopped or running, liveness never consulted; no names from the running
/// world `ae list` reports — and hands each one to [`board::observe`], which
/// owns every read of a transcript. `--follow` keeps the same code path for
/// the first pass, then loops [`board::follow_poll`] every
/// [`board::follow::POLL_SECS`] seconds over the same fixed selection.
fn run_board(
    tail: &[String],
    world: Option<&listing::World>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let args = match board::parse(tail) {
        Ok(args) => args,
        Err(usage) => {
            write!(err, "{}", usage.render())?;
            return Ok(entry::EXIT_USAGE);
        }
    };
    let Some(root) = state_root() else {
        writeln!(err, "ae: {NO_STATE_ROOT}")?;
        return Ok(EXIT_UNAVAILABLE);
    };
    let roots = inventory::Roots::under(&root);
    let selected: Vec<usage::SessionInput> = if args.sessions.is_empty() {
        let Some(world) = world else {
            writeln!(err, "ae: {NO_STATE_ROOT}")?;
            return Ok(EXIT_UNAVAILABLE);
        };
        // Before ANY absence claim: an enumeration that lost a source cannot
        // prove a session is not there. The warning is `ae list`'s own, so
        // the two surfaces say the same thing.
        if let Some(warning) = listing::diagnostic(world) {
            writeln!(err, "{warning}")?;
        }
        filters::Selection::running()
            .select(&world.sessions, world.now)
            .iter()
            .map(|entry| usage::SessionInput {
                name: entry.name.clone(),
                path: roots.sessions().join(&entry.name),
            })
            .collect()
    } else {
        let scan = inventory::durable_meta_records(&roots);
        let mut selected = Vec::new();
        for name in &args.sessions {
            let Some(record) = scan.records.iter().find(|record| &record.name == name) else {
                writeln!(err, "ae board: no session named {name}")?;
                return Ok(EXIT_UNAVAILABLE);
            };
            selected.push(usage::SessionInput {
                name: record.name.clone(),
                path: record.path.clone(),
            });
        }
        selected
    };
    let home = doors::home();
    let inputs = board::Inputs {
        home: home.as_deref(),
        sessions: &selected,
        replies: if args.assistant {
            board::Replies::All
        } else {
            board::Replies::Off
        },
    };
    let observation = board::observe(&inputs, args.since_micros);
    let rendered = board::render(&observation, args.json, args.lines);
    write!(out, "{rendered}")?;
    if !args.follow {
        return Ok(0);
    }
    // The follow: the pass above IS the one-shot board, and every poll after
    // it appends only what is new. Selection is fixed at start; Ctrl-C ends it
    // and no signal handling is installed.
    out.flush()?;
    let mut follow =
        board::follow::Follow::seeded(&observation.seeds, &observation.coverage, args.since_micros);
    loop {
        std::thread::sleep(std::time::Duration::from_secs(board::follow::POLL_SECS));
        let batch = board::follow_poll(&inputs, &mut follow);
        let rendered = board::render_batch(&batch, args.json, args.lines);
        write!(out, "{rendered}")?;
        out.flush()?;
    }
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
        let mut sockets = fleet_read::SocketPaths::live();
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
    let Some(mut viewer) = writing_viewer(dir, err)? else {
        return Ok(state::EXIT_FAILED);
    };
    if tracked::refuse_role_entry(dir, err)? {
        return Ok(state::EXIT_FAILED);
    }
    viewer.session_id = tracked::routing_id(dir, &viewer.slot, &viewer.session, &own_session(dir));
    match goal::run(dir, &viewer, &write, time::Timestamp::now()) {
        Ok((line, changed)) => {
            out.write_all(line.as_bytes())?;
            if let Some(line) = changed
                .then(|| session_notice::after_goal(dir, &viewer))
                .flatten()
            {
                writeln!(out, "{line}")?;
            }
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
        if let Some(code) = refuse_seat_override(kind.action(), tail, err)? {
            return Ok(code);
        }
        Some(tracked::Sender {
            display,
            slot: String::new(),
            session: String::new(),
        })
    } else {
        let (viewer, seat) = match writing_caller(dir, kind.action(), err)? {
            Ok(found) => found,
            Err(code) => return Ok(code),
        };
        if let Some(seat) = seat {
            let sender = tracked::Sender {
                display: tracked::ORCHESTRATOR_DISPLAY.to_owned(),
                slot: seat.slot.clone(),
                session: seat.session.clone(),
            };
            let run = |dispatch: &mut tracked::Dispatch, out: &mut Vec<u8>, err: &mut Vec<u8>| {
                tracked::run_with(
                    kind,
                    dir,
                    tail,
                    Some(&sender),
                    &own_session(dir),
                    time::Timestamp::now(),
                    entropy(),
                    send_defer(),
                    tracked::Extra {
                        seat: Some(dispatch),
                        ..tracked::Extra::default()
                    },
                    out,
                    err,
                )
            };
            return run_seat_helper(&seat, kind.action(), run, out, err);
        }
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

/// Whether `args` run a session helper while `AE_SENDER_OVERRIDE` names the
/// console, the human or the orchestrator sender — refused at this one
/// ingress, for every helper, before anything is parsed or read, because the
/// console's actor is built only by its own submit, the human's display only
/// by that actor, and the orchestrator's only by ae's own relay frame.
fn speaks_as_reserved(args: &[String]) -> bool {
    let helper = args
        .first()
        .is_some_and(|entry| shim::HELPERS.iter().any(|helper| helper.entry == entry));
    helper
        && sender_override().is_some_and(|display| {
            display.starts_with(tracked::CONSOLE_NAMESPACE)
                || display.starts_with(tracked::HUMAN_NAMESPACE)
                || display.starts_with(tracked::ORCHESTRATOR_NAMESPACE)
        })
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
pub(crate) fn send_defer() -> std::time::Duration {
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
/// variable being none; and the three private `_AE_EXPECT_*` expected-seat members,
/// kept raw (an empty or non-UTF-8 value stays distinct from unset) so that
/// [`send::expectation`] can refuse a supplied but unusable expectation.
fn send_env() -> send::Env {
    /// The door's one read: a variable's raw value, unset being `None`.
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the frozen _AE_EVENT_*/AE_SENDER_OVERRIDE contract every caller of the send helper writes, and the private _AE_EXPECT_* expected-seat members that only narrow admission — see clippy.toml"
    )]
    fn raw(name: &str) -> Option<std::ffi::OsString> {
        std::env::var_os(name)
    }
    let read = |name: &str| {
        raw(name)
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
        expect: ["_AE_EXPECT_SLOT", "_AE_EXPECT_LAUNCH", "_AE_EXPECT_LEGACY"].map(raw),
    }
}

/// Sixty-four bits nobody chose: `RandomState` is seeded from the OS per
/// process, and needs no door.
pub(crate) fn entropy() -> u64 {
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
    let Some(viewer) = writing_viewer(dir, err)? else {
        return Ok(state::EXIT_FAILED);
    };
    match memo::run(dir, &viewer, &add, time::Timestamp::now()) {
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
    // Legacy readers and boundaries that already refuse an absent proof retain
    // their Option contract. Writers without a sender override must use the
    // typed observation below.
    match observe_calling_pane() {
        CallerRead::Read(observed) => Some(observed),
        CallerRead::Outside | CallerRead::Unread => None,
    }
}

/// One reading of the calling pane for the board writer: what tmux says it is,
/// the server it was read on and the pane id the proof compares.
fn observe_for_desk() -> Option<relay::Observed> {
    actual_calling_pane().map(|(viewer, server)| relay::Observed {
        viewer,
        server,
        pane: doors::calling_pane_id(),
    })
}

/// One owner for absence by design, a positive answer (vacant included), and a
/// failed observation. Only the last refuses a writer that permits humans.
pub(crate) enum CallerRead<T> {
    Outside,
    Read(T),
    Unread,
}

impl<T> CallerRead<T> {
    /// Reuse the surface's existing read and its bounded retry policy. Missing
    /// either marker is OUTSIDE: tmux bindings and popups carry TMUX alone.
    pub(crate) fn observe(
        server: Option<&inventory::ServerId>,
        pane: Option<&str>,
        read: impl FnOnce(&inventory::ServerId, &str) -> Option<T>,
    ) -> Self {
        match (server, pane.filter(|pane| !pane.is_empty())) {
            (Some(server), Some(pane)) => match read(server, pane) {
                Some(answer) => Self::Read(answer),
                None => Self::Unread,
            },
            _ => Self::Outside,
        }
    }

    /// A writer may treat OUTSIDE as human, never a failed read. The Result
    /// forces that decision before the caller can default a vacant identity.
    pub(crate) fn require(self) -> std::result::Result<Option<T>, &'static str> {
        match self {
            Self::Outside => Ok(None),
            Self::Read(answer) => Ok(Some(answer)),
            Self::Unread => Err(
                "Error: could not detect current agent identity; retry from your own ae pane, or unset stale TMUX and TMUX_PANE markers in a shell outside tmux",
            ),
        }
    }
}

/// Observe only the caller's own server/pane pair, with #285's one re-ask.
pub(crate) fn observe_calling_pane() -> CallerRead<(tmux::ObservedViewer, inventory::ServerId)> {
    let pane = doors::calling_pane_id();
    let server = doors::caller_server();
    CallerRead::observe(server.as_ref(), pane.as_deref(), |server, pane| {
        transport::reask_viewer(server, pane).map(|viewer| (viewer, server.clone()))
    })
}

/// What the orchestrator seat's pane runs only by its own class's rules, as one
/// word for the refusal and the clause that says why. The proved seat runs a
/// helper `--cross-session` word; it runs a stop or a rename only in the bound
/// form (the gate admits it, and the bound form proves the rest); it runs a
/// start or a create only from the entry, with `--no-attach`. Every other word
/// here is refused to the seat in this release, and every word here is refused
/// to a pane of the seat's session that fails the dispatcher proof. Of the
/// ordinary panes, only an agent seat's rename of another session is read here
/// ([`refuse_ordinary`]).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outbound {
    /// A helper `--cross-session` send, ask, review or interrupt.
    Cross(&'static str),
    /// A stop other than the bound form: the session it names, or `None` for
    /// the caller's own (`--self`, or no name).
    Stop(Option<String>),
    /// A rename: the session it moves (`None` for the caller's own, the
    /// one-name form), the new name, whether it is the bound form, and the
    /// user words after `ae`.
    Rename {
        old: Option<String>,
        new: String,
        bound: bool,
        words: Vec<String>,
    },
    /// An end or rm: the session it names (`None` for the caller's own),
    /// whether it is the bound Keep form, the word the user typed and the user
    /// words after `ae`.
    End {
        target: Option<String>,
        bound: bool,
        verb: &'static str,
        words: Vec<String>,
    },
    /// A verb the seat does not run: the word and the clause.
    Verb(&'static str, &'static str),
}

/// The clause a launch refusal gives the seat.
const SEAT_LAUNCH_ONLY: &str =
    "starts and creates other sessions only from the entry, with --no-attach, so no client moves";

/// The outbound class of `request`, or `None` for any word every caller may
/// run.
fn outbound_of(request: &cli::Request) -> Option<Outbound> {
    let cross = |word, tail: &[String]| {
        (tail.first().map(String::as_str) == Some(tracked::CROSS_SESSION_FLAG))
            .then_some(Outbound::Cross(word))
    };
    match request {
        cli::Request::Send { tail, .. } => cross("send", tail),
        cli::Request::Ask { tail, .. } => cross("ask", tail),
        cli::Request::Review { tail, .. } => cross("review", tail),
        cli::Request::Interrupt { tail, .. } => cross("interrupt", tail),
        cli::Request::Launch { .. } => Some(Outbound::Verb("launch", SEAT_LAUNCH_ONLY)),
        cli::Request::Spawn { .. } => Some(Outbound::Verb(
            "spawn",
            "does not add seats in this release; give the owner the command",
        )),
        cli::Request::Retire { .. } => Some(Outbound::Verb(
            "retire",
            "does not retire seats in this release; give the owner the command",
        )),
        cli::Request::Compact { .. } => Some(Outbound::Verb(
            "compact",
            "does not compact seats in this release; give the owner the command",
        )),
        cli::Request::Relaunch { .. } => Some(Outbound::Verb(
            "relaunch",
            "does not relaunch seats in this release; give the owner the command",
        )),
        cli::Request::Reseat { .. } => Some(Outbound::Verb(
            "reseat",
            "does not reseat seats in this release; give the owner the command",
        )),
        // The bound stop proves the caller, its target and every seat's
        // declaration itself, under the target's lifecycle lock.
        cli::Request::Stop { tail } if lifecycle::is_bound_stop(tail) => None,
        cli::Request::Stop { tail } => Some(Outbound::Stop(stop_aim(tail))),
        // A resume word on the seat's own name is a launch of the seat.
        cli::Request::Orchestrator { tail }
            if tail
                .iter()
                .any(|word| word == "--resume" || word == "--expect-uuid") =>
        {
            Some(Outbound::Verb(
                "launch",
                "never starts or resumes its own seat; the owner runs ae orchestrator",
            ))
        }
        cli::Request::End { tail } => end_aim(tail),
        cli::Request::Rename { tail } => rename_aim(tail),
        _ => None,
    }
}

/// The outbound class of a `rename` tail: the bound form, the two-name form
/// or the caller's own one-name form. Any other tail is the rename's own usage
/// refusal, which changes nothing, so no class gate reads it.
fn rename_aim(tail: &[String]) -> Option<Outbound> {
    let (old, new, bound) = match rename::bound_form(tail) {
        Some(Ok(request)) => (Some(request.old), request.new, true),
        Some(Err(_)) => return None,
        None => match tail {
            [old, new] => (Some(old.clone()), new.clone(), false),
            [new] => (None, new.clone(), false),
            _ => return None,
        },
    };
    let words = std::iter::once(cli::RENAME.to_owned())
        .chain(tail.iter().cloned())
        .collect();
    Some(Outbound::Rename {
        old,
        new,
        bound,
        words,
    })
}

/// The outbound class of an `_end` tail: the bound Keep form or a plain end
/// or rm, and the session it names. A tail that names the bound words but is
/// not the admitted form is the end's own usage refusal, which changes
/// nothing, so no class gate reads it.
fn end_aim(tail: &[String]) -> Option<Outbound> {
    // The reason's text is data, whatever it spells: lift it before the scan.
    let (_, lifted) = lifecycle::split_reason(tail).ok()?;
    let tail = lifted.as_slice();
    let marker = lifecycle::end::RM_MARKER;
    let verb = if tail.iter().any(|word| word == marker) {
        "rm"
    } else {
        "end"
    };
    let user: Vec<String> = tail
        .iter()
        .filter(|word| *word != marker)
        .cloned()
        .collect();
    let (target, bound) = match lifecycle::end::bound_form(tail) {
        Some(Ok(request)) => (Some(request.name), true),
        Some(Err(_)) => return None,
        None => (
            user.iter().find(|word| !word.starts_with('-')).cloned(),
            false,
        ),
    };
    let words = std::iter::once(verb.to_owned()).chain(user).collect();
    Some(Outbound::End {
        target,
        bound,
        verb,
        words,
    })
}

/// The session a plain `_stop` tail names: `None` for `--self` or no name.
fn stop_aim(tail: &[String]) -> Option<String> {
    // The reason's text is data, whatever it spells: lift it before the scan.
    let (_, tail) = lifecycle::split_reason(tail).ok()?;
    if tail.iter().any(|word| word == "--self") {
        return None;
    }
    let mut words = tail.iter();
    let mut named = None;
    while let Some(word) = words.next() {
        if word == "--pane" {
            words.next();
        } else if !word.starts_with('-') && named.is_none() {
            named = Some(word.clone());
        }
    }
    named
}

/// Apply the seat's class rules to `what` before any lock or write. An
/// ordinary pane, a pane with no slot (the human's shell in the same window), a
/// session that only claims the role and an unobserved or foreign caller go on.
/// `Some(code)` means the refusal is said and nothing was changed. An explicit
/// `--cross-session` helper takes no observation here: its writer, or its
/// sender-override decision, makes the route's ONE observation, which is both
/// the seat's permission and its audit, and refuses an UNREAD pane there.
fn refuse_dispatcher(what: Outbound, err: &mut impl Write) -> Result<Option<u8>> {
    if matches!(what, Outbound::Cross(_)) {
        return Ok(None);
    }
    refuse_dispatcher_seen(what, observe_calling_pane(), state_root().as_deref(), err)
}

/// The class of the calling pane, from ONE observation: `Ok(None)` for no
/// tmux markers (a binding, a popup, a plain shell), and `Err(code)` once an
/// UNREAD pane's typed refusal is said, so a failed read never passes as the
/// human.
fn calling_class(
    seen: CallerRead<(tmux::ObservedViewer, inventory::ServerId)>,
    root: Option<&std::path::Path>,
    err: &mut impl Write,
) -> Result<std::result::Result<Option<relay::SeatClass>, u8>> {
    let (viewer, server) = match seen.require() {
        Ok(Some(seen)) => seen,
        Ok(None) => return Ok(Ok(None)),
        Err(why) => {
            writeln!(err, "{why}")?;
            err.flush()?;
            return Ok(Err(entry::EXIT_FAILED));
        }
    };
    let Some(root) = root else {
        return Ok(Ok(None));
    };
    Ok(Ok(Some(seat_class_of(root, &viewer, &server))))
}

/// The class ONE reading of the calling pane gives it.
fn seat_class_of(
    root: &std::path::Path,
    viewer: &tmux::ObservedViewer,
    server: &inventory::ServerId,
) -> relay::SeatClass {
    let pane = doors::calling_pane_id().unwrap_or_default();
    relay::seat_class(root, viewer, server, &pane)
}

/// [`refuse_dispatcher`] on an observation already taken.
fn refuse_dispatcher_seen(
    what: Outbound,
    seen: CallerRead<(tmux::ObservedViewer, inventory::ServerId)>,
    root: Option<&std::path::Path>,
    err: &mut impl Write,
) -> Result<Option<u8>> {
    let class = match calling_class(seen, root, err)? {
        Ok(Some(class)) => class,
        Ok(None) => return Ok(None),
        Err(code) => return Ok(Some(code)),
    };
    refuse_class(what, &class, root, err)
}

/// The outbound gate's verdict on one observed class of the calling pane.
fn refuse_class(
    what: Outbound,
    class: &relay::SeatClass,
    root: Option<&std::path::Path>,
    err: &mut impl Write,
) -> Result<Option<u8>> {
    let said = match class {
        relay::SeatClass::Ordinary(pane) => return refuse_ordinary(&what, pane.clone(), root, err),
        relay::SeatClass::Unproven(why) => unproven_refusal(outbound_word(&what), why),
        relay::SeatClass::Dispatcher(seat) => match what {
            // The bound rename proves the caller, its target and every seat's
            // declaration itself, under both lifecycle locks.
            Outbound::Cross(_)
            | Outbound::Rename { bound: true, .. }
            | Outbound::End { bound: true, .. } => return Ok(None),
            Outbound::Stop(aim) => match aim {
                Some(name) if !names_own_session(root, seat, &name) => format!(
                    "ae: stop REFUSED — the orchestrator stops another session only in the bound form: ae stop {name} --expect-uuid <uuid>, with --owner-input <ask-id> when a seat there is busy. Nothing was changed."
                ),
                _ => own_session_refusal("stop", &seat.session),
            },
            Outbound::Rename { old, new, .. } => match old {
                Some(name) if !names_own_session(root, seat, &name) => format!(
                    "ae: rename REFUSED — the orchestrator renames another session only in the bound form: ae rename {name} {new} --expect-uuid <uuid>, with --owner-input <ask-id> when a seat there is busy. Nothing was changed."
                ),
                _ => rename::own_session_refusal(&seat.session, &new),
            },
            Outbound::End {
                target,
                verb,
                words,
                ..
            } => match target {
                Some(name)
                    if name == "all" || (verb == "rm" && !names_own_session(root, seat, &name)) =>
                {
                    format!(
                        "ae: {verb} REFUSED — the orchestrator ends one session at a time, only in the bound Keep form, so {verb} {name} stays the owner's. The owner runs, in the owner's own terminal: ae {}. Nothing was changed.",
                        words.join(" ")
                    )
                }
                Some(name) if !names_own_session(root, seat, &name) => format!(
                    "ae: {verb} REFUSED — the orchestrator ends another session only in the bound Keep form: ae end {name} --keep-history --expect-uuid <uuid>, with --owner-input <ask-id> when a seat there is not done. Nothing was changed."
                ),
                _ => lifecycle::end::own_session_refusal(&seat.session),
            },
            Outbound::Verb(word, why) => seat_verb_refusal(word, why),
        },
    };
    writeln!(err, "{said}")?;
    err.flush()?;
    Ok(Some(entry::EXIT_USAGE))
}

/// An ordinary pane's lifecycle form: an agent seat's rename of another
/// session is refused with the owner's command and one row in the seat's own
/// record. The names are the ones the rename itself can reach, a recorded
/// pre-grammar name too. Any other pane, its own session, a name the rename
/// cannot reach (it refuses that itself) and every other word go on.
fn refuse_ordinary(
    what: &Outbound,
    pane: relay::PaneIdentity,
    root: Option<&std::path::Path>,
    err: &mut impl Write,
) -> Result<Option<u8>> {
    let ((verb, old, words), Some(root)) = (lifecycle_aim(what), root) else {
        return Ok(None);
    };
    let Some(old) = old else {
        return Ok(None);
    };
    let session_launch::LaunchCaller::Seat(seat) = session_launch::LaunchCaller::of(pane) else {
        return Ok(None);
    };
    let sessions = lifecycle::sessions_dir(root);
    // The fleet word names every other session, and an end of it is refused
    // like any other session's.
    let fleet = verb != "rename" && old == "all";
    if !fleet
        && (!lifecycle::name_is_usable(root, old)
            || session_launch::is_own(&sessions.join(old), &seat))
    {
        return Ok(None);
    }
    Ok(Some(session_launch::refuse_lifecycle(
        &sessions, old, &seat, verb, words, err,
    )?))
}

/// The verb, the named session and the user words of a lifecycle form an
/// ordinary seat's gate judges: a rename, an end or an rm.
fn lifecycle_aim(what: &Outbound) -> (&'static str, Option<&String>, &[String]) {
    match what {
        Outbound::Rename { old, words, .. } => ("rename", old.as_ref(), words),
        Outbound::End {
            target,
            verb,
            words,
            ..
        } => (verb, target.as_ref(), words),
        _ => ("", None, &[]),
    }
}

/// The one word an [`Outbound`] is refused as.
fn outbound_word(what: &Outbound) -> &'static str {
    match what {
        Outbound::Cross(word) | Outbound::Verb(word, _) => word,
        Outbound::Stop(_) => "stop",
        Outbound::Rename { .. } => "rename",
        Outbound::End { verb, .. } => verb,
    }
}

/// The refusal of a verb the orchestrator seat does not run, with the clause
/// that says why.
fn seat_verb_refusal(word: &str, why: &str) -> String {
    format!("ae: {word} REFUSED — the orchestrator seat {why}. Nothing was changed.")
}

/// The refusal a pane of the seat's session takes when it fails the
/// dispatcher proof: neither the seat's powers nor an ordinary pane's.
fn unproven_refusal(word: &str, why: &str) -> String {
    format!(
        "ae: {word} REFUSED — this pane is in the orchestrator seat's session but ae cannot prove it is the seat ({why}). Nothing was changed."
    )
}

/// G1: the seat never stops, ends or renames the session it runs in; the
/// owner's exact command is the remedy.
fn own_session_refusal(word: &str, own: &str) -> String {
    format!(
        "ae: {word} REFUSED — '{own}' is the orchestrator seat's own session, and the seat never stops it. The owner runs, in the owner's own terminal: ae stop {own} -y. Nothing was changed."
    )
}

/// Whether `name` is the seat's own session: its name, or a rename alias that
/// resolves to its directory.
fn names_own_session(root: Option<&std::path::Path>, seat: &relay::Dispatcher, name: &str) -> bool {
    if name == seat.session {
        return true;
    }
    root.is_some_and(|root| {
        lifecycle::aliases::target(&lifecycle::sessions_dir(root).join(name))
            .is_some_and(|target| target.file_name() == Some(std::ffi::OsStr::new(&seat.session)))
    })
}

/// Refuse UNREAD before invoking any helper writer. The returned viewer IS
/// the observation to use for the action; never re-read it into human later.
fn writing_viewer(dir: &std::path::Path, err: &mut impl Write) -> Result<Option<requests::Viewer>> {
    match observe_calling_pane().require() {
        Ok(answer) => Ok(Some(
            answer
                .map(|(pane, _)| requests::Viewer::from_pane(&pane, &own_session(dir)))
                .unwrap_or_default(),
        )),
        Err(why) => {
            writeln!(err, "{why}")?;
            Ok(None)
        }
    }
}

/// [`writing_viewer`], and from the SAME observation the proved orchestrator
/// seat when the caller is the seat, whichever session's helper serves it. A
/// pane in the seat's session that ae cannot prove to be the seat is refused
/// as `word`: it gets neither the seat's powers nor an ordinary pane's. `Err`
/// carries the exit code of a refusal already written.
fn writing_caller(
    dir: &std::path::Path,
    word: &str,
    err: &mut impl Write,
) -> Result<std::result::Result<(requests::Viewer, Option<relay::Dispatcher>), u8>> {
    let own = own_session(dir);
    match observe_calling_pane().require() {
        Ok(None) => Ok(Ok((requests::Viewer::default(), None))),
        Ok(Some((pane, server))) => {
            let viewer = requests::Viewer::from_pane(&pane, &own);
            let id = doors::calling_pane_id().unwrap_or_default();
            match state_root().map(|root| relay::seat_class(&root, &pane, &server, &id)) {
                Some(relay::SeatClass::Dispatcher(seat)) => Ok(Ok((viewer, Some(seat)))),
                Some(relay::SeatClass::Unproven(why)) => {
                    writeln!(err, "{}", unproven_refusal(word, why))?;
                    err.flush()?;
                    Ok(Err(entry::EXIT_USAGE))
                }
                Some(relay::SeatClass::Ordinary(_)) | None => Ok(Ok((viewer, None))),
            }
        }
        Err(why) => {
            writeln!(err, "{why}")?;
            Ok(Err(state::EXIT_FAILED))
        }
    }
}

/// The refusal of `AE_SENDER_OVERRIDE` from the proved seat's own pane, and
/// from a pane of the seat's session that fails the proof: the seat speaks
/// only under its own name and with its audit, whichever session the helper
/// serves. A pane-less caller and every ordinary pane keep the override. On an
/// explicit `--cross-session` route (`tail` starts with the flag) this is the
/// route's ONE observation, so an UNREAD pane takes its typed refusal here; the
/// bare route keeps the override for it, as the send helper always has.
fn refuse_seat_override(word: &str, tail: &[String], err: &mut impl Write) -> Result<Option<u8>> {
    let explicit = tail.first().map(String::as_str) == Some(tracked::CROSS_SESSION_FLAG);
    let id = doors::calling_pane_id().unwrap_or_default();
    let root = state_root();
    refuse_seat_override_seen(
        word,
        explicit,
        observe_calling_pane(),
        root.as_deref(),
        &id,
        err,
    )
}

/// [`refuse_seat_override`] on an observation already taken: `explicit` for
/// the `--cross-session` route, `id` the calling pane.
fn refuse_seat_override_seen(
    word: &str,
    explicit: bool,
    seen: CallerRead<(tmux::ObservedViewer, inventory::ServerId)>,
    root: Option<&std::path::Path>,
    id: &str,
    err: &mut impl Write,
) -> Result<Option<u8>> {
    let (pane, server) = match seen.require() {
        Ok(Some(seen)) => seen,
        Err(why) if explicit => {
            writeln!(err, "{why}")?;
            err.flush()?;
            return Ok(Some(entry::EXIT_FAILED));
        }
        Ok(None) | Err(_) => return Ok(None),
    };
    let Some(root) = root else {
        return Ok(None);
    };
    let said = match relay::seat_class(root, &pane, &server, id) {
        relay::SeatClass::Ordinary(_) => return Ok(None),
        relay::SeatClass::Unproven(why) => unproven_refusal(word, why),
        relay::SeatClass::Dispatcher(_) => format!(
            "ae: {word} REFUSED — the orchestrator seat speaks under its own name, with its audit, so AE_SENDER_OVERRIDE is refused from its pane. Nothing was sent."
        ),
    };
    writeln!(err, "{said}")?;
    err.flush()?;
    Ok(Some(entry::EXIT_USAGE))
}

/// A helper word of the proved seat: the seat speaks as
/// [`tracked::ORCHESTRATOR_DISPLAY`], `run` delivers with both streams held
/// and the dispatch the helper binds to its resolved target, and the result
/// row is written before anything reaches the caller.
fn run_seat_helper(
    seat: &relay::Dispatcher,
    verb: &'static str,
    run: impl FnOnce(&mut tracked::Dispatch, &mut Vec<u8>, &mut Vec<u8>) -> std::io::Result<u8>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let mut dispatch = seat_dispatch(seat, verb, "", "", "");
    let (mut held_out, mut held_err) = (Vec::new(), Vec::new());
    let code = run(&mut dispatch, &mut held_out, &mut held_err).map_err(Into::into);
    let written = dispatch.requested.then_some(dispatch);
    finish_dispatch(written, code, &held_out, held_err, out, err)
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
pub(crate) fn own_session(dir: &std::path::Path) -> String {
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
    hourly: bool,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    if hourly {
        let home = doors::home();
        let sessions = [usage::SessionInput {
            name: own_session(dir),
            path: dir.to_owned(),
        }];
        let observed = usage::observe_hourly(&usage::HourlyInputs {
            home: home.as_deref(),
            sessions: &sessions,
            now: time::Timestamp::now().epoch(),
        });
        write!(out, "{}", usage::hourly::render(&observed, false))?;
        return Ok(0);
    }
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
fn state_root() -> Option<std::path::PathBuf> {
    doors::state_root(shape::current())
}

/// The human's fleet order, read from the global config file at `global`.
///
/// One resolution for all three readers — the strip, the lifecycle handoff and
/// the picker — so none grows a rule of its own. `None`, an unreadable file and
/// a malformed entry all give the EMPTY order ae drew before this key existed.
pub(crate) fn fleet_order_at(global: Option<&std::path::Path>) -> theme::FleetOrder {
    let (names, _ignored) = config::fleet_order_entries(&config::global_fleet_order(global));
    theme::FleetOrder::from_validated(names)
}

/// The global config a session's own watchdog reads: the state root's, or —
/// with no root to name — the one two levels above the session's directory.
/// The watchdog and `auto_trust`'s in-wait answers read the same file.
pub(crate) fn global_config_of(meta_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    state_root()
        .or_else(|| {
            meta_dir
                .parent()
                .and_then(std::path::Path::parent)
                .map(std::path::Path::to_path_buf)
        })
        .map(|root| doors::config_file(shape::current(), &root))
}

/// [`fleet_order_at`] for a caller that holds no path of its own.
pub(crate) fn fleet_order() -> theme::FleetOrder {
    let global = state_root().map(|root| doors::config_file(shape::current(), &root));
    fleet_order_at(global.as_deref())
}

/// The CHECKOUT-only read gate `AE_TEST_APP_READ_GATE=<dir>`: while
/// `<dir>/<key>` exists, the read named `key` holds — a session's journal read
/// by its name, a standing console's transcript stages as `@board-current` and
/// `@board-earlier` whatever the session, the fleet's world read as `@world`,
/// the in-app reader's file read as `reader`, the auto reseat leg's pair reads
/// as `@auto-reseat-pair` and `@auto-reseat-after` —
/// after appending one line `held <key>` to `<dir>/trace`, polling for at most
/// 60 s. Every read that finds the file holds again; `trace` itself is never a
/// gate. An installed core returns before reading the variable; unset, nothing
/// is read. Test seam, unset in production (AGENTS.md doors table).
pub(crate) fn read_gate(key: &str) {
    if !shape::current().honours_environment() || key == "trace" {
        return;
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the CHECKOUT-only app read gate, after the honours_environment check — see AGENTS.md"
    )]
    let Some(dir) = std::env::var_os("AE_TEST_APP_READ_GATE").map(std::path::PathBuf::from) else {
        return;
    };
    let gate = dir.join(key);
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the CHECKOUT-only app read gate's own file, polled while it holds"
    )]
    let held = || gate.exists();
    if !held() {
        return;
    }
    let trace = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("trace"));
    if let Ok(mut file) = trace {
        let _ = writeln!(file, "held {key}");
    }
    let until = std::time::Instant::now() + std::time::Duration::from_mins(1);
    while held() && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

/// The refusal every command that needs the state root gives when there is
/// none: one stderr line, exit 1.
fn refuse_no_state_root(err: &mut impl Write) -> Result<u8> {
    writeln!(err, "ae: {NO_STATE_ROOT}")?;
    Ok(EXIT_UNAVAILABLE)
}

/// Run `entry` over the state root, read here and only here; refuse when
/// there is none. The root is read before `entry` runs, so whatever `entry`
/// reads comes after it.
fn with_state_root<E: Write, X: Into<Error>>(
    err: &mut E,
    entry: impl FnOnce(&std::path::Path, &mut E) -> std::result::Result<u8, X>,
) -> Result<u8> {
    match state_root() {
        Some(root) => entry(&root, err).map_err(Into::into),
        None => refuse_no_state_root(err),
    }
}

/// Run `entry` as the viewer that may write; an unreadable viewer is refused
/// (the reason is on `err`) and exits 1 without running it.
///
/// The viewer is observed here, once, before `entry` runs, so whatever `entry`
/// reads or stamps comes after it.
fn with_writing_viewer<E: Write, X: Into<Error>>(
    dir: &std::path::Path,
    err: &mut E,
    entry: impl FnOnce(requests::Viewer, &mut E) -> std::result::Result<u8, X>,
) -> Result<u8> {
    match writing_viewer(dir, err)? {
        Some(viewer) => entry(viewer, err).map_err(Into::into),
        None => Ok(state::EXIT_FAILED),
    }
}

/// `send`: the sender is the observed viewer, unless the environment names a
/// sender override, which skips the refusing viewer gate and takes the actual
/// calling pane instead.
fn run_send(
    dir: &std::path::Path,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let own_session = own_session(dir);
    let env = send_env();
    let viewer = if env.sender_override.is_some() {
        if let Some(code) = refuse_seat_override("send", tail, err)? {
            return Ok(code);
        }
        Some(
            actual_calling_pane()
                .as_ref()
                .map(|(pane, _)| requests::Viewer::from_pane(pane, &own_session))
                .unwrap_or_default(),
        )
    } else {
        let (viewer, seat) = match writing_caller(dir, "send", err)? {
            Ok(found) => found,
            Err(code) => return Ok(code),
        };
        if let Some(seat) = seat {
            let words = send::Words {
                display: tracked::ORCHESTRATOR_DISPLAY,
                caller_session: &seat.session,
                own_session: &own_session,
            };
            let run = |dispatch: &mut tracked::Dispatch, out: &mut Vec<u8>, err: &mut Vec<u8>| {
                send::run_with_seat(
                    dir,
                    tail,
                    &env,
                    &words,
                    time::Timestamp::now(),
                    send_defer(),
                    Some(dispatch),
                    out,
                    err,
                )
            };
            return run_seat_helper(&seat, "send", run, out, err);
        }
        Some(viewer)
    };
    let Some(viewer) = viewer else {
        return Ok(state::EXIT_FAILED);
    };
    Ok(send::run(
        dir,
        tail,
        &env,
        &viewer.display,
        &viewer.session,
        &own_session,
        time::Timestamp::now(),
        send_defer(),
        out,
        err,
    )?)
}

/// `interrupt`: the sender is the observed viewer, or the proved seat.
fn run_interrupt(
    dir: &std::path::Path,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    let own_session = own_session(dir);
    let (viewer, seat) = match writing_caller(dir, "interrupt", err)? {
        Ok(found) => found,
        Err(code) => return Ok(code),
    };
    if let Some(seat) = seat {
        let run = |dispatch: &mut tracked::Dispatch, _: &mut Vec<u8>, err: &mut Vec<u8>| {
            interrupt::run(
                dir,
                tail,
                tracked::ORCHESTRATOR_DISPLAY,
                &seat.session,
                &own_session,
                time::Timestamp::now(),
                Some(dispatch),
                err,
            )
        };
        return run_seat_helper(&seat, "interrupt", run, out, err);
    }
    let code = interrupt::run(
        dir,
        tail,
        &viewer.display,
        &viewer.session,
        &own_session,
        time::Timestamp::now(),
        None,
        err,
    )?;
    Ok(code)
}

/// Run `entry` over the world the caller already read; there is none when the
/// state root could not be derived, which is refused like a missing root.
fn with_world<E: Write, X: Into<Error>>(
    world: Option<&listing::World>,
    err: &mut E,
    entry: impl FnOnce(&listing::World, &mut E) -> std::result::Result<u8, X>,
) -> Result<u8> {
    match world {
        Some(world) => entry(world, err).map_err(Into::into),
        None => refuse_no_state_root(err),
    }
}

/// `list`: the inventory warning (not for `--json`), then the pending-rename
/// warnings, then the table. Every warning goes to STDERR and the table still
/// prints.
fn run_list(
    list_args: &filters::ListArgs,
    world: &listing::World,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8> {
    if !list_args.json
        && let Some(warning) = listing::diagnostic(world)
    {
        writeln!(err, "{warning}")?;
    }
    if let Some(root) = state_root() {
        write_pending_renames(&root, err)?;
    }
    write!(out, "{}", listing::render(list_args, world))?;
    Ok(0)
}

/// A pending rename transaction is not ordinary state: surface it on STDERR
/// with its proved retry rather than letting the table read as settled. The
/// damaged carriers are read only after every pending intent was written.
fn write_pending_renames(root: &std::path::Path, err: &mut impl Write) -> Result<()> {
    for intent in rename::pending_intents(root) {
        writeln!(
            err,
            "ae: warning: rename '{}' → '{}' is in progress at phase '{}' — retry 'ae rename {} {}'",
            intent.old_name(),
            intent.new_name(),
            intent.phase(),
            intent.old_name(),
            intent.new_name()
        )?;
    }
    for damaged in rename::pending_damaged(root) {
        writeln!(
            err,
            "ae: warning: a damaged rename carrier is pending ({}) — repair or remove it by hand",
            damaged.why
        )?;
    }
    Ok(())
}

/// What a `MissingOperand` refusal says the command needs.
///
/// Every internal entry but two is per-SESSION and takes that session's meta
/// directory; `_telegram-run` takes an ae home, and `_net-probe` is not a
/// directory at all: the one entry that takes a name to resolve.
fn missing_operand(command: &str) -> &'static str {
    if command == cli::TELEGRAM_RUN {
        "an ae home directory"
    } else if command == cli::NET_PROBE {
        "a host to resolve"
    } else {
        "a session meta directory"
    }
}

/// The bare `orchestrator` word, reached here only by a direct `run_with`
/// caller (a parsed `--popup` is answered before `run_dispatch`): a usage
/// error, the one-line "no preamble" refusal, or the missing state root.
fn refuse_unwired_orchestrator(tail: &[String], err: &mut impl Write) -> Result<u8> {
    let args = match orchestrator::parse(tail) {
        Ok(args) => args,
        Err(usage) => {
            write!(err, "{}", usage.render())?;
            return Ok(usage.code());
        }
    };
    if !(args.popup || args.settings || args.quota_dialog) {
        // The bare word reaches the core only WITHOUT a preamble: the seat is
        // a launch, and a launch needs the ae command.
        writeln!(
            err,
            "ae orchestrator: the seat launches through the ae command"
        )?;
        return Ok(entry::EXIT_USAGE);
    }
    refuse_no_state_root(err)
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
            writeln!(err, "ae: {command} needs {}", missing_operand(command))?;
            request.exit_code().unwrap_or(2)
        }
        // The refusal is a DIAGNOSTIC and never
        // reaches stdout, which is why a refused invocation's stdout is empty
        // rather than a bare header.
        cli::Request::State { dir, tail } => run_state(dir, tail, out, err)?,
        cli::Request::Goal { dir, tail } => run_goal(dir, tail, out, err)?,
        cli::Request::Desk { dir, tail } => deskcmd::run(
            dir,
            tail,
            &mut observe_for_desk,
            time::Timestamp::now(),
            &mut std::io::stdin(),
            out,
            err,
        )?,
        cli::Request::Memo { dir, tail } => run_memo(dir, tail, out, err)?,
        cli::Request::Ask { dir, tail } => run_tracked(tracked::Kind::Ask, dir, tail, out, err)?,
        cli::Request::Interrupt { dir, tail } => run_interrupt(dir, tail, out, err)?,
        cli::Request::Send { dir, tail } => run_send(dir, tail, out, err)?,
        cli::Request::Relay { dir, tail } => relay::run(
            dir,
            tail,
            actual_calling_pane,
            time::Timestamp::now(),
            entropy(),
            out,
            err,
        )?,
        cli::Request::Reply { dir, tail } => reply::run(
            dir,
            tail,
            calling_pane().as_ref(),
            &own_session(dir),
            time::Timestamp::now(),
            send_defer(),
            err,
        )?,
        cli::Request::Withdraw { dir, tail } => request_verb::run(
            request_verb::Verb::Withdraw,
            dir,
            tail,
            calling_pane().as_ref(),
            &own_session(dir),
            time::Timestamp::now(),
            out,
            err,
        )?,
        cli::Request::Accept { dir, tail } => request_verb::run(
            request_verb::Verb::Accept,
            dir,
            tail,
            calling_pane().as_ref(),
            &own_session(dir),
            time::Timestamp::now(),
            out,
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
        cli::Request::Usage { dir } => run_usage_helper(dir, false, out, err)?,
        cli::Request::UsageHourly { dir } => run_usage_helper(dir, true, out, err)?,
        cli::Request::Focus { dir, tail } => {
            panes::focus(dir, tail, &own_session(dir), time::Timestamp::now(), err)?
        }
        cli::Request::Launch { tail } => run_served_launch(tail, out, err)?,
        cli::Request::CaptureSid { dir, slot, pane } => {
            session_launch::capture::run(dir, slot, pane, &session_launch::recorded_server(dir))
        }
        cli::Request::RegisterSid { dir, slot, id } => {
            session_launch::capture::register_sid(dir, slot, id.as_deref(), out, err)?
        }
        cli::Request::Requests { dir, mode, limit } => {
            let rendered = requests::render(dir, *mode, &calling_viewer(dir), *limit);
            out.write_all(&rendered.stdout)?;
            err.write_all(&rendered.stderr)?;
            rendered.code
        }
        cli::Request::RequestsVerify { dir, id } => console::relay_verify::run(dir, id, out)?,
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
        cli::Request::Relaunch { dir, tail } => with_state_root(err, |root, err| {
            with_writing_viewer(dir, err, |viewer, err| {
                seat_relaunch::run(
                    root,
                    dir,
                    tail,
                    &viewer.display,
                    &own_session(dir),
                    time::Timestamp::now(),
                    out,
                    err,
                )
            })
        })?,
        cli::Request::Reseat { tail } => with_state_root(err, |root, err| {
            reseat::run(root, world, tail, time::Timestamp::now(), out, err)
        })?,
        cli::Request::Spawn { dir, tail } => {
            with_writing_viewer(dir, err, |viewer, err| -> Result<u8> {
                if tracked::refuse_role_entry(dir, err)? {
                    return Ok(state::EXIT_FAILED);
                }
                Ok(spawn::run_spawn(
                    dir,
                    tail,
                    &viewer.display,
                    time::Timestamp::now(),
                    out,
                    err,
                )?)
            })?
        }
        cli::Request::Retire { dir, tail } => with_writing_viewer(dir, err, |viewer, err| {
            spawn::run_retire(dir, tail, &viewer.display, time::Timestamp::now(), out, err)
        })?,
        // The three whole lifecycle operations.
        cli::Request::End { tail } => with_state_root(err, |root, err| {
            lifecycle::end::run(root, tail, doors::calling_pane_id().as_deref(), out, err)
        })?,
        cli::Request::Stop { tail } => with_state_root(err, |root, err| {
            // The bound stop proves its caller itself, so the dispatcher's
            // class gate never reads it. The fleet feed still names the agent
            // seat that ran it, from one soft reading of the calling pane: an
            // unread pane or none is the provenance's own word, and no
            // refusal comes from here.
            let observed = lifecycle::is_bound_stop(tail)
                .then(|| bound_entry(root))
                .flatten();
            lifecycle::run_stop(root, tail, observed.as_ref(), out, err)
        })?,
        cli::Request::SessionMenu { tail } => {
            with_state_root(err, |root, err| session_menu::run(root, tail, out, err))?
        }
        cli::Request::Reader { tail } => reader::run(tail, out, err)?,
        cli::Request::ConsoleToggle { tail } => console::toggle::run(tail, out, err)?,
        cli::Request::OrchestratorJump { tail } => jump::run(tail, err)?,
        cli::Request::Compact { tail } => with_state_root(err, |root, err| {
            lifecycle::compaction::run(root, tail, out, err)
        })?,
        // `doctor` and `rename` take no session directory, so like `end`/`stop`
        // they derive the state root themselves and refuse the same way when
        // there is none.
        cli::Request::Doctor { tail } => {
            with_state_root(err, |root, err| doctor::run(root, tail, out, err))?
        }
        cli::Request::Init { tail } => with_state_root(err, |root, err| {
            let path = doors::config_file(shape::current(), root);
            init::run(&path, tail, out, err)
        })?,
        cli::Request::Rename { tail } => {
            with_state_root(err, |root, err| rename::run(root, tail, out, err))?
        }
        cli::Request::AutoReseat { tail } => with_state_root(err, |root, err| {
            autoreseat::leg::run(root, tail, time::Timestamp::now(), err)
        })?,
        cli::Request::AutoEffort { tail } => {
            with_state_root(err, |root, err| effort::leg::run(root, tail, err))?
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
                reason: archive::publish::Reason::End,
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
        cli::Request::Watchdog { tail } => with_state_root(err, |root, err| {
            watchdog_lifecycle::run(root, tail, out, err)
        })?,
        cli::Request::Telegram { tail } => with_state_root(err, |root, err| {
            telegram_lifecycle::run(root, tail, out, err)
        })?,
        cli::Request::WatchdogRun {
            dir,
            knobs,
            step: None,
        } => watchdog_daemon::run(dir, *knobs, out, err)?,
        cli::Request::WatchdogRun {
            dir,
            knobs,
            step: Some(channel),
        } => watchdog_daemon::run_stepped(dir, *knobs, channel, out, err)?,
        cli::Request::TelegramRun { paths, knobs } => {
            telegram::bridge::run(paths, *knobs, settings_menu::role_holder, err)?
        }
        cli::Request::NetProbe { host, port } => netprobe::run(host, *port, out, err)?,
        cli::Request::CompactMemoBaseline { dir } => compact::memo_baseline_step(dir, out)?,
        cli::Request::CompactFindOutstanding { dir } => compact::find_outstanding_step(dir, out)?,
        cli::Request::ListEnded => {
            if let Some(root) = state_root() {
                archive::ended(&root, out, err)?
            } else {
                writeln!(err, "{NO_STATE_ROOT}")?;
                state::EXIT_FAILED
            }
        }
        cli::Request::List(list_args) => with_world(world, err, |world, err| {
            run_list(list_args, world, out, err)
        })?,
        cli::Request::Monitor { dir, args } => with_world(world, err, |world, err| {
            monitor::run(dir, world, args, out, err)
        })?,
        cli::Request::Next { tail } => run_next(tail, world, out, err)?,
        cli::Request::Brief { tail } => run_brief(tail, world, out, err)?,
        cli::Request::Board { tail } => run_board(tail, world, out, err)?,
        cli::Request::Console { tail } => console::run(tail, out, err)?,
        cli::Request::App { tail } => app::run(tail, out, err)?,
        // A parsed `--popup` is answered before `run_dispatch` reaches this
        // generic arm. Only a direct `run_with` caller can bring one here.
        cli::Request::Orchestrator { tail } => refuse_unwired_orchestrator(tail, err)?,
        cli::Request::LaunchCandidate(name) => {
            writeln!(err, "ae: {NO_LAUNCHER}: {name}")?;
            EXIT_UNAVAILABLE
        }
    };
    err.flush()?;
    error::with_status(code, out.flush())?;
    Ok(code)
}

#[cfg(test)]
mod tests {
    /// An end and an rm are outbound with the session they name, the word the
    /// user typed and the bound Keep form; a tail that names the bound words
    /// but is not that form is the end's own usage refusal.
    #[test]
    fn an_end_and_an_rm_are_outbound_with_their_target_and_their_word() {
        use super::{Outbound, cli::Request, outbound_of};
        let of = |argv: &[&str]| {
            let words: Vec<String> = argv.iter().map(|word| (*word).to_owned()).collect();
            outbound_of(&Request::parse(&words))
        };
        let uuid = "0b6c8f5e-1d2a-4c3b-9e8f-7a6b5c4d3e2f";
        let end = |target: Option<&str>, bound: bool, verb: &'static str, words: &[&str]| {
            Some(Outbound::End {
                target: target.map(ToOwned::to_owned),
                bound,
                verb,
                words: words.iter().map(|word| (*word).to_owned()).collect(),
            })
        };
        assert_eq!(
            of(&["_end", "t"]),
            end(Some("t"), false, "end", &["end", "t"])
        );
        assert_eq!(of(&["_end", "-f"]), end(None, false, "end", &["end", "-f"]));
        assert_eq!(
            of(&["_end", "--verb=rm", "t", "-f"]),
            end(Some("t"), false, "rm", &["rm", "t", "-f"])
        );
        let bound = ["_end", "t", "--keep-history", "--expect-uuid", uuid];
        assert_eq!(
            of(&bound),
            end(
                Some("t"),
                true,
                "end",
                &["end", "t", "--keep-history", "--expect-uuid", uuid]
            )
        );
        assert_eq!(of(&["_end", "t", "--expect-uuid", uuid]), None);
        // The reason is lifted first: it is no word of the end and no target.
        assert_eq!(
            of(&[
                "_end",
                "t",
                "--keep-history",
                "--expect-uuid",
                uuid,
                "--reason",
                "work finished"
            ]),
            end(
                Some("t"),
                true,
                "end",
                &["end", "t", "--keep-history", "--expect-uuid", uuid]
            )
        );
        assert_eq!(
            of(&["_end", "--reason", "other", "t"]),
            end(Some("t"), false, "end", &["end", "t"])
        );
        assert_eq!(
            of(&["_end", "t", "--reason", "--keep-history"]),
            end(Some("t"), false, "end", &["end", "t"])
        );
        assert_eq!(
            of(&[
                "_end",
                "--verb=rm",
                "t",
                "--keep-history",
                "--expect-uuid",
                uuid
            ]),
            None
        );
    }

    /// What the seat's class rules decide: a helper `--cross-session` send,
    /// ask, review or interrupt, a non-bound stop with the session it names,
    /// each rename form with its names, and the verbs the seat does not run in
    /// this release.
    #[test]
    fn only_a_helper_cross_session_word_a_plain_stop_and_the_class_verbs_are_outbound() {
        use super::{Outbound, cli::Request, outbound_of};
        let of = |argv: &[&str]| {
            let words: Vec<String> = argv.iter().map(|word| (*word).to_owned()).collect();
            outbound_of(&Request::parse(&words))
        };
        for (verb, word) in [
            ("_send", "send"),
            ("_ask", "ask"),
            ("_review", "review"),
            ("_interrupt", "interrupt"),
        ] {
            let argv = [verb, "/d", "--cross-session", "t:lead", "x"];
            assert_eq!(of(&argv), Some(Outbound::Cross(word)), "{verb}");
            // The flag counts only before the recipient, as the helpers read it.
            assert_eq!(of(&[verb, "/d", "boss", "--cross-session"]), None);
            assert_eq!(of(&[verb, "/d", "boss", "x"]), None);
        }
        assert_eq!(
            of(&["_stop", "t"]),
            Some(Outbound::Stop(Some("t".to_owned())))
        );
        assert_eq!(of(&["_stop", "--self"]), Some(Outbound::Stop(None)));
        assert_eq!(of(&["_stop", "--pane", "%3"]), Some(Outbound::Stop(None)));
        let uuid = "0b6c8f5e-1d2a-4c3b-9e8f-7a6b5c4d3e2f";
        assert_eq!(of(&["_stop", "t", "--expect-uuid", uuid]), None);
        // The reason is data: it neither names the target nor hides a flag, and the
        // bound form stays bound with one.
        assert_eq!(
            of(&["_stop", "t", "--reason", "--self"]),
            Some(Outbound::Stop(Some("t".to_owned())))
        );
        assert_eq!(
            of(&["_stop", "--reason", "other", "t"]),
            Some(Outbound::Stop(Some("t".to_owned())))
        );
        assert_eq!(
            of(&["_stop", "--reason=x", "--self"]),
            Some(Outbound::Stop(None))
        );
        assert_eq!(
            of(&["_stop", "t", "--expect-uuid", uuid, "--reason", "parked"]),
            None
        );
        // A reason that only spells the expectation makes no bound stop.
        assert_eq!(
            of(&["_stop", "t", "--reason", "--expect-uuid", uuid]),
            Some(Outbound::Stop(Some("t".to_owned())))
        );
        for (argv, word) in [
            (vec!["_spawn", "/d", "n", "--using", "p"], "spawn"),
            (vec!["_retire", "/d", "n"], "retire"),
            (vec!["_compact", "/d", "lead"], "compact"),
            (vec!["_relaunch", "/d", "lead"], "relaunch"),
            (vec!["reseat", "t", "lead", "--using", "p"], "reseat"),
            // The served underscore entry of every launch form.
            (vec!["_launch", "--no-attach"], "launch"),
            (vec!["_launch", "new", "--local"], "launch"),
        ] {
            assert!(
                matches!(of(&argv), Some(Outbound::Verb(said, _)) if said == word),
                "{argv:?}"
            );
        }
        assert_eq!(of(&["list"]), None);
        assert_eq!(of(&["_relay", "/d", "t", "--task", "x"]), None);
        let rename = |old: Option<&str>, new: &str, bound: bool, argv: &[&str]| {
            Some(Outbound::Rename {
                old: old.map(ToOwned::to_owned),
                new: new.to_owned(),
                bound,
                words: argv.iter().map(|word| (*word).to_owned()).collect(),
            })
        };
        for (argv, old, bound) in [
            (vec!["rename", "a", "b"], Some("a"), false),
            (vec!["rename", "b"], None, false),
            (
                vec!["rename", "a", "b", "--expect-uuid", uuid],
                Some("a"),
                true,
            ),
            (
                vec![
                    "rename",
                    "a",
                    "b",
                    "--expect-uuid",
                    uuid,
                    "--owner-input",
                    "i",
                ],
                Some("a"),
                true,
            ),
        ] {
            assert_eq!(of(&argv), rename(old, "b", bound, &argv), "{argv:?}");
        }
        // A tail the rename refuses as usage changes nothing, so no gate reads it.
        for argv in [
            vec!["rename"],
            vec!["rename", "a", "b", "c"],
            vec!["rename", "a", "b", "--expect-uuid"],
            vec!["rename", "a", "--expect-uuid", uuid],
        ] {
            assert_eq!(of(&argv), None, "{argv:?}");
        }
    }

    /// A failed read of the calling pane is no observation: the outbound gate
    /// refuses it with the typed message and exit code before any writer below
    /// runs, while OUTSIDE and a positively observed pane with no seat go on.
    #[test]
    fn the_outbound_gate_refuses_an_unread_pane_and_admits_outside_and_a_vacant_one() {
        use super::{CallerRead, Outbound, refuse_dispatcher_seen};
        let root = Some(std::path::PathBuf::from("/state"));
        let run = |seen| {
            let mut said = Vec::new();
            let what = Outbound::Stop(Some("t".to_owned()));
            let code = refuse_dispatcher_seen(what, seen, root.as_deref(), &mut said);
            (code.expect("the gate writes to a vector"), said)
        };
        let (code, said) = run(CallerRead::Unread);
        assert_eq!(code, Some(crate::entry::EXIT_FAILED));
        assert!(
            String::from_utf8_lossy(&said).contains("could not detect current agent identity"),
            "{}",
            String::from_utf8_lossy(&said)
        );
        let (code, said) = run(CallerRead::Outside);
        assert_eq!((code, said.is_empty()), (None, true));
        for viewer in [
            crate::tmux::ObservedViewer::default(),
            crate::tmux::ObservedViewer {
                session: Some("host".to_owned()),
                ..crate::tmux::ObservedViewer::default()
            },
        ] {
            let seen = CallerRead::Read((viewer, crate::inventory::ServerId::Ambient));
            let (code, said) = run(seen);
            assert_eq!((code, said.is_empty()), (None, true));
        }
    }

    /// An explicit `--cross-session` helper takes no observation at the
    /// outbound gate. Its sender-override decision is the route's one
    /// observation: UNREAD refuses there with the typed message, OUTSIDE (a
    /// pane-less caller) keeps the override, and the bare route keeps it for
    /// UNREAD as the send helper always has.
    #[test]
    fn the_explicit_override_refuses_an_unread_pane_and_keeps_a_paneless_one() {
        use super::{CallerRead, Outbound, refuse_dispatcher, refuse_seat_override_seen};
        let mut said = Vec::new();
        let code = refuse_dispatcher(Outbound::Cross("send"), &mut said);
        assert_eq!(code.expect("the gate writes to a vector"), None);
        assert!(said.is_empty());
        let root = std::env::temp_dir().join("ae-no-such-root-for-the-override-gate");
        let run = |explicit, seen| {
            let mut said = Vec::new();
            let code =
                refuse_seat_override_seen("send", explicit, seen, Some(&root), "%1", &mut said);
            (code.expect("the decision writes to a vector"), said)
        };
        let (code, said) = run(true, CallerRead::Unread);
        assert_eq!(code, Some(crate::entry::EXIT_FAILED));
        assert!(
            String::from_utf8_lossy(&said).contains("could not detect current agent identity"),
            "{}",
            String::from_utf8_lossy(&said)
        );
        let vacant = || {
            CallerRead::Read((
                crate::tmux::ObservedViewer::default(),
                crate::inventory::ServerId::Ambient,
            ))
        };
        for (explicit, seen) in [
            (true, CallerRead::Outside),
            (false, CallerRead::Outside),
            (false, CallerRead::Unread),
            (true, vacant()),
            (false, vacant()),
        ] {
            let (code, said) = run(explicit, seen);
            assert_eq!((code, said.is_empty()), (None, true), "explicit {explicit}");
        }
    }

    /// The seat-pane test of bare `ae`: only a described, unstamped pane whose
    /// session is recorded draws the app; every other reading keeps the list.
    #[test]
    fn only_an_unstamped_pane_of_a_recorded_session_lands() {
        let pane = |agent: Option<&str>, session: Option<&str>| crate::tmux::ObservedViewer {
            agent: agent.map(ToOwned::to_owned),
            session: session.map(ToOwned::to_owned),
            ..crate::tmux::ObservedViewer::default()
        };
        let recorded = |name: &str| name == "one";
        let lands =
            |viewer: Option<&crate::tmux::ObservedViewer>| super::pane_may_land(viewer, recorded);
        assert!(
            lands(Some(&pane(None, Some("one")))),
            "a plain or chat pane"
        );
        assert!(!lands(Some(&pane(Some("lead"), Some("one")))), "a seat");
        assert!(
            !lands(Some(&pane(None, Some("host")))),
            "an unrecorded home"
        );
        assert!(!lands(Some(&pane(None, None))), "no session read");
        assert!(!lands(None), "an unread pane fails closed");
    }

    /// The named entry's seat test: any described pane without `@ae_agent`,
    /// whatever session it sits in, because the app's home is the named one.
    #[test]
    fn a_named_landing_asks_only_that_the_pane_is_no_seat() {
        let pane = |agent: Option<&str>, session: Option<&str>| crate::tmux::ObservedViewer {
            agent: agent.map(ToOwned::to_owned),
            session: session.map(ToOwned::to_owned),
            ..crate::tmux::ObservedViewer::default()
        };
        assert!(super::pane_is_seatless(Some(&pane(None, Some("host")))));
        assert!(
            !super::pane_is_seatless(Some(&pane(None, None))),
            "no session named"
        );
        assert!(!super::pane_is_seatless(Some(&pane(
            Some("lead"),
            Some("one")
        ))));
        assert!(!super::pane_is_seatless(None), "unread fails closed");
        // tmux 3.7b's whole answer for a pane id it cannot find: exit 0, every
        // pane field empty, only the server's socket path filled.
        let missing = crate::tmux::interpret_viewer(true, " |  |  |  | /tmp/s/sock\n");
        assert!(missing.is_some(), "tmux answered");
        assert!(!super::pane_is_seatless(missing.as_ref()), "a missing pane");
    }

    /// Only a grammar name lands, so the `--dir` read stays under the sessions
    /// root: a path, a traversal, an entry word and the orchestrator seat keep
    /// today's launch and its refusal.
    #[test]
    fn only_a_plain_session_name_is_landed_on() {
        assert!(super::landing_name("demo"));
        for name in ["/etc", "../demo", "a/b", ".hidden", "", "list", "upgrade"] {
            assert!(!super::landing_name(name), "{name:?}");
        }
        assert!(!super::landing_name(
            crate::orchestrator::ORCHESTRATOR_SESSION
        ));
    }

    /// `say`'s record is the builder's, byte for byte, and the watchdog's
    /// chat line addresses no seat.
    #[test]
    fn the_chat_record_say_writes_is_the_one_builder_byte_for_byte() {
        let ts = Timestamp::parse("2026-09-25T10:00:00Z").expect("a timestamp");
        let today = crate::tracked::event_line(&crate::tracked::EventFields {
            ts,
            actor: "lead",
            action: "chat",
            target: "",
            reference: "",
            actor_slot: "main",
            actor_session: "",
            target_slot: "",
            target_session: "",
            actor_session_id: "",
            target_session_id: "",
            target_server: "",
            target_pane: "",
            target_session_uuid: "",
            caller_server: "",
            caller_pane: "",
            caller_session_uuid: "",
            identity_gap: "",
            summary: "hi there",
            body_file: "",
        });
        assert_eq!(super::chat_line(ts, "lead", "main", "hi there"), today);
        let told = crate::events::Event::parse_line(&super::chat_line(ts, "watchdog", "", "moved"))
            .expect("a well-formed record");
        assert_eq!(
            (
                told.actor.as_str(),
                told.action.as_str(),
                told.target,
                told.summary.as_deref()
            ),
            ("watchdog", "chat", None, Some("moved"))
        );
    }
    use super::{
        EXIT_UNAVAILABLE, Error, NO_LAUNCHER, NO_STATE_ROOT, help_text, listing::World, run,
        run_with, version_line,
    };
    use crate::digest::{SessionEntry, Status};
    use crate::time::Timestamp;
    use crate::{inventory, orchestrator, tmux};
    use std::io::{self, Write};

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the test plants a prompt file through the product publish path, then reads it back"
    )]
    fn a_folded_user_turn_seat_reports_its_recorded_first_message() {
        let dir = std::env::temp_dir().join(format!("ae-fold-first-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a fixture dir");
        // The fold's artifact: a framed, brief-only first message, never ctx.
        let framed =
            crate::provenance::first_line(&crate::provenance::brief("lead"), "do the fold");
        crate::run::publish_prompt(&dir, "spawned.0", &framed).expect("the publish");
        match super::first_message_for(&dir, "spawned.0") {
            crate::seatpack::FirstMessage::Recorded { text, .. } => {
                assert_eq!(text, framed, "section 9 carries the fold's message");
            }
            crate::seatpack::FirstMessage::Absent => {
                panic!("a folded seat reports Recorded, not Absent")
            }
            crate::seatpack::FirstMessage::Unreadable { .. } => {
                panic!("a folded seat reports Recorded, not Unreadable")
            }
        }
        match super::first_message_for(&dir, "spawned.1") {
            crate::seatpack::FirstMessage::Absent => {}
            _ => panic!("no prompt file still reports Absent"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_marked_menu_retracts_its_own_marker_only_when_the_draw_fails() {
        let clears = std::cell::Cell::new(0);
        assert!(!super::draw_marked_menu(
            || false,
            || {
                clears.set(clears.get() + 1);
                true
            },
        ));
        assert_eq!(clears.get(), 1);
        assert!(super::draw_marked_menu(
            || true,
            || {
                clears.set(clears.get() + 1);
                true
            },
        ));
        assert_eq!(clears.get(), 1, "a visible menu keeps its marker");
    }

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

    #[test]
    fn session_quota_awareness_defaults_on_reads_the_pin_and_honours_live_config() {
        let root = std::path::PathBuf::from(format!("/tmp/ae-quota-aware-{}", std::process::id()));
        let dir = root.join("session");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&dir).unwrap();
        let off = root.join("off");
        let on = root.join("on");
        std::fs::write(&off, "[workspace]\nquota = off\n").unwrap();
        std::fs::write(&on, "[workspace]\nquota = on\n").unwrap();
        let ask = |meta: Option<&str>, config: Option<&std::path::Path>| {
            if let Some(body) = meta {
                std::fs::write(dir.join(crate::store::META), body).unwrap();
            } else {
                let _ = std::fs::remove_file(dir.join(crate::store::META));
            }
            super::session_quota_aware(&dir, config, None)
        };
        // No state at all: today's surface, not a refusal.
        assert!(ask(None, None));
        assert!(ask(Some("session=demo\n"), None), "absent row means ON");
        assert!(ask(Some("session=demo\nquota=on\n"), Some(off.as_path())));
        assert!(
            !ask(Some("session=demo\nquota=off\n"), Some(on.as_path())),
            "a pinned off row is what gates the menu and refuses the dialog"
        );
        // The off-diagonal: a pre-knob session (no pin) honours live config.
        assert!(
            !ask(Some("session=demo\n"), Some(off.as_path())),
            "no pin plus config off is unaware everywhere"
        );
        assert!(ask(Some("session=demo\n"), Some(on.as_path())));
        assert!(
            !ask(None, Some(off.as_path())),
            "unreadable meta falls back to config"
        );
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
            argv(&["orchestrator", "--settings"]),
            argv(&["orchestrator", "--quota-dialog", "--client", "x"]),
        ];
        for args in accepted {
            let request = crate::cli::Request::parse(&args);
            assert!(super::schedules_automatic_upgrade(&request), "{args:?}");
        }

        let refused_or_unrelated = [
            argv(&["list", "unexpected"]),
            argv(&["brief", "--since", "bad"]),
            // `--full` is a read-only contract: it never schedules the upgrade check.
            argv(&["brief", "--full"]),
            argv(&["brief", "--all", "--full"]),
            argv(&["brief", "aedev", "--since", "4h", "--full"]),
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
            "the picker must build its own bounded read, never current_world"
        );
        let list = crate::cli::Request::parse(&argv(&["list"]));
        assert!(super::request_needs_world(&list));
    }

    fn durable_record(name: &str, server: crate::meta::ServerSelector) -> inventory::DurableRecord {
        inventory::DurableRecord {
            path: std::path::PathBuf::from(format!("/s/{name}")),
            name: name.to_owned(),
            layout: inventory::Layout::Canonical,
            server,
            meta_read: crate::session::MetaRead::Parsed,
            snapshot: crate::session::RecordSnapshot::default(),
        }
    }

    fn live_row(name: &str) -> tmux::PickerSession {
        tmux::PickerSession {
            name: name.to_owned(),
            id: "$1".to_owned(),
            rank: 0,
            glyph: String::new(),
            main_pane: String::new(),
            branch: String::new(),
            agents: String::new(),
            activity: None,
            created_at: None,
            goal: String::new(),
        }
    }

    /// A fake socket resolution: `-L ae` and `/tmp/ae.sock` are ONE server;
    /// every other spelling does not answer.
    fn fake_socket(server: &inventory::ServerId) -> Option<String> {
        match server {
            inventory::ServerId::Selected(crate::meta::Selector::Socket(path))
                if path == std::path::Path::new("/tmp/ae.sock") =>
            {
                Some("/tmp/ae.sock".to_owned())
            }
            inventory::ServerId::Selected(crate::meta::Selector::Name(name)) if name == "ae" => {
                Some("/tmp/ae.sock".to_owned())
            }
            _ => None,
        }
    }

    fn caller_server() -> inventory::ServerId {
        inventory::ServerId::Selected(crate::meta::Selector::Socket(std::path::PathBuf::from(
            "/tmp/ae.sock",
        )))
    }

    fn picker_scan(records: Vec<inventory::DurableRecord>) -> inventory::DurableScan {
        inventory::DurableScan::from(records)
    }

    #[test]
    fn picker_stopped_reads_only_the_calling_server_spelled_either_way() {
        let calling = caller_server();
        let scan = picker_scan(vec![
            durable_record(
                "on-caller",
                crate::meta::ServerSelector::Positive(crate::meta::Selector::Name("ae".to_owned())),
            ),
            durable_record(
                "bad/name",
                crate::meta::ServerSelector::Positive(crate::meta::Selector::Name("ae".to_owned())),
            ),
            durable_record(
                "elsewhere",
                crate::meta::ServerSelector::Positive(crate::meta::Selector::Name(
                    "other".to_owned(),
                )),
            ),
            durable_record(
                "unowned-live",
                crate::meta::ServerSelector::Positive(crate::meta::Selector::Name("ae".to_owned())),
            ),
            durable_record(
                "running",
                crate::meta::ServerSelector::Positive(crate::meta::Selector::Name("ae".to_owned())),
            ),
            durable_record("no-selector", crate::meta::ServerSelector::Missing),
            durable_record("ambiguous", crate::meta::ServerSelector::Ambiguous),
        ]);
        let mut sockets = crate::fleet_read::SocketPaths::asking(fake_socket);
        let mut servers = vec![calling.clone()];
        for record in &scan.records {
            if let Some(selector) = record.server.entitles() {
                let id = inventory::ServerId::Selected(selector.clone());
                if !servers.contains(&id) {
                    servers.push(id);
                }
            }
        }
        let _ = sockets.deduplicated(servers);
        // The server's own listing: `on-caller` and `bad/name` are ABSENT (so
        // stopped); `unowned-live` exists without an ae fact (unknown, not
        // stopped); `running` exists AND owns a rank row.
        let names = ["unowned-live".to_owned(), "running".to_owned()];
        let live = [live_row("running")];
        let backend = super::PickerStoppedBackend {
            server: &calling,
            names: Some(&names),
            live: &live,
            sockets: &sockets,
        };
        let rows = super::picker_stopped(scan, &backend, &live);
        assert_eq!(
            rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
            ["on-caller"],
            "the caller alias answered; foreign, unowned-live and malformed names never became rows"
        );
    }

    #[test]
    fn picker_stopped_fails_closed_without_a_name_listing() {
        let calling = caller_server();
        let scan = picker_scan(vec![durable_record(
            "on-caller",
            crate::meta::ServerSelector::Positive(crate::meta::Selector::Name("ae".to_owned())),
        )]);
        let mut sockets = crate::fleet_read::SocketPaths::asking(fake_socket);
        let _ = sockets.deduplicated(vec![
            calling.clone(),
            inventory::ServerId::Selected(crate::meta::Selector::Name("ae".to_owned())),
        ]);
        let backend = super::PickerStoppedBackend {
            server: &calling,
            names: None,
            live: &[],
            sockets: &sockets,
        };
        assert!(
            super::picker_stopped(scan, &backend, &[]).is_empty(),
            "a failed name listing proves nothing and claims nothing"
        );
    }

    #[test]
    fn stopped_rows_order_by_their_last_live_moment_then_name() {
        let row = |name: &str, last_live: Option<i64>| orchestrator::PickerStopped {
            name: name.to_owned(),
            goal: String::new(),
            branch: String::new(),
            last_live,
        };
        let mut rows = [
            row("undated-b", None),
            row("old", Some(100)),
            row("recent", Some(900)),
            row("undated-a", None),
        ];
        super::sort_stopped_rows(&mut rows);
        assert_eq!(
            rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
            ["recent", "old", "undated-a", "undated-b"]
        );
    }

    /// The whole picker path over a real state directory: a stopped session's
    /// journal is a DIRECTORY (so any read of it would fail), and the row still
    /// draws with its meta goal — the assembly never opens it.
    #[test]
    fn a_stopped_row_draws_from_meta_alone_over_a_hostile_journal() {
        let root = std::env::temp_dir().join(format!("ae-picker-stopped-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("sessions/parked");
        std::fs::create_dir_all(&dir).expect("a session dir");
        std::fs::write(
            dir.join("meta"),
            "goal=ship the picker\ntmux_server_kind=name\ntmux_server=ae\n",
        )
        .expect("a meta");
        std::fs::create_dir_all(dir.join("events.jsonl")).expect("a hostile journal shape");

        let calling = caller_server();
        let mut sockets = crate::fleet_read::SocketPaths::asking(fake_socket);
        let _ = sockets.deduplicated(vec![
            calling.clone(),
            inventory::ServerId::Selected(crate::meta::Selector::Name("ae".to_owned())),
        ]);
        // The bare name listing succeeded and does not carry `parked`: exactly
        // what proves the session stopped on the calling server.
        let backend = super::PickerStoppedBackend {
            server: &calling,
            names: Some(&[]),
            live: &[],
            sockets: &sockets,
        };
        let scan = inventory::durable_meta_records(&inventory::Roots::under(&root));
        assert_eq!(scan.records.len(), 1, "the scan found the session dir");
        assert_eq!(
            scan.records[0].meta_read,
            crate::session::MetaRead::Parsed,
            "the meta parsed"
        );
        assert_eq!(
            scan.records[0].server,
            crate::meta::ServerSelector::Positive(crate::meta::Selector::Name("ae".to_owned())),
            "the selector is positive"
        );
        assert!(
            scan.records[0].snapshot.events.is_none(),
            "the picker's scan carries no journal"
        );
        let rows = super::picker_stopped(scan, &backend, &[]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "parked");
        assert_eq!(rows[0].goal, "ship the picker");
        let _ = std::fs::remove_dir_all(&root);
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
