//! `ae` — agent environment: a tmux-backed multi-agent session multiplexer.
//!
//! Every quality lane — fmt, clippy, nextest, doctests, coverage, mutants —
//! runs against real code.
//!
//! # Where the behavior comes from
//!
//! Every module here is built from RATIFIED rows of
//! `docs/migration/semantic-contract.md`, and each names its rows in its own
//! module docs. The bash implementation is **not** an oracle: it may be read to
//! understand a mechanism, but it never defines an expected output. A behavior
//! with no row stops the work and goes to the seats — which is why several
//! fields of the `list --json` digest are *inputs* to [`session::entry_for`]
//! rather than things it reads. See that module's docs for the list.
//!
//! # The first read-side slice (P1: `list --json`)
//!
//! | Module | Rows |
//! |---|---|
//! | [`json`] | SC-510d — the escape set, both directions |
//! | [`meta`] | SC-405a–e — the session meta keys, and only those |
//! | [`time`] | SC-510a, SC-509 — the one timestamp spelling |
//! | [`events`] | SC-510a–f, SC-511a–c, SC-405j, SC-519, SC-520, DR-001 — the record and the generation-aware reader |
//! | [`attention`] | SC-017g, SC-509 — severity and the rollup |
//! | [`digest`] | SC-509, SC-509b, SC-506 — the versioned document that always closes, and says when it lost something |
//! | [`filters`] | SC-017a–f, SC-017i, SC-521, SC-523, SC-524 — which sessions a listing shows |
//! | [`inventory`] | SC-017j, SC-404 — which sessions EXIST, before anything asks whether they run |
//! | [`liveness`] | SC-017k, SC-017l — what ae knows about running, and what it says when it cannot tell |
//! | [`tmux`] | SC-017k — which server an argument list addresses, and what a completed run means |
//! | [`transport`] | SC-017k, SC-017l — the exec, and why a run that did not answer is never absence |
//! | [`session`] | SC-017e, SC-017g, SC-405d/f/g/i/j/k, SC-518, SC-520, SC-980 — what a session directory establishes, and what it must be told |
//! | [`listing`] | SC-017f, SC-017h, SC-509, SC-506 — the two renderings of one selection, and the injected world they read |
//! | [`cli`] | SC-021, SC-022, and the argv half of SC-017a–i / SC-521a/b — which word is `list`, and which parser owns its flags |
//!
//! # The read side, slice 2: the two helper query surfaces
//!
//! | Module | Rows |
//! |---|---|
//! | [`event_text`] | SC-211d, SC-211n — the OPAQUE extraction and framing the generated helpers read through, which is not [`events`]'s typed reader |
//! | [`requests`] | SC-212c, SC-518, SC-1306d — pending, replied, cancelled, and the table that shows them |
//! | [`events_tail`] | SC-211n, SC-1306e — the monitor pane's banner, replay and follow |
//!
//! These two are GENERATED SESSION HELPERS in the frozen tree, invoked as
//! `<AE_HOME>/sessions/<name>/requests` and `…/events-tail`. Their successor
//! spelling is [`cli::REQUESTS`] and [`cli::EVENTS_TAIL`], and the argv mapping a
//! parity run must declare for them is recorded in each module's docs.
//!
//! Most of those rows exist BECAUSE this code was written. Two slices stopped on
//! eleven questions rather than inferring answers, and the seats ratified the
//! results: eighteen rows from the first batch, five more and an amendment to
//! SC-017g from the second, plus an amendment to SC-510c whose original text had
//! dropped its own authority's hedge. Three of those rulings REVERSED what this
//! crate first did, and one rejected a row this crate's own evidence might have
//! justified — a live census is evidence, never contract.
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
pub mod error;
pub mod event_text;
pub mod events;
pub mod events_tail;
pub mod filters;
pub mod git;
pub mod goal;
pub mod identity;
pub mod interrupt;
pub mod inventory;
pub mod json;
pub mod launch;
pub mod launch_cmd;
/// The whole `end`/`stop`/`compact` operations — the frozen order, in one place.
pub mod lifecycle;
pub mod listing;
pub mod liveness;
pub mod memo;
pub mod meta;
pub mod netprobe;
pub mod next;
pub mod panes;
pub mod procs;
pub mod render;
pub mod reply;
pub mod requests;
pub mod roster;
pub mod send;
pub mod session;
pub mod session_launch;
mod session_tmux;
pub mod spawn;
pub mod state;
pub mod teardown;
pub mod telegram;
pub mod time;
pub mod tmux;
pub mod tracked;
pub mod transport;
pub mod watchdog;
pub mod watchdog_daemon;
pub mod watchdog_glue;

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
///
/// Every command this binary actually carries has to appear here. Help that
/// omits a shipped surface is a wrong answer wearing the shape of a right one —
/// the same failure `ae list` refuses to commit when it has no source to read.
///
/// **The help LAYOUT is not ratified.** SC-012 owns this surface ("prints the
/// command surface") and is not a row this slice was built from, so what is
/// maintained here is the CONTENT — the commands that exist — and no test
/// asserts its exact bytes. Its acceptance rides SC-012's own seat mark.
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
///
/// **The unwired-source refusal is GONE.** `ae list` answers now: SC-017j
/// enumerates, SC-017k/l classify, SC-017m/n render. What remains is the one
/// case where there is nothing to enumerate FROM — no `AE_HOME` and no `HOME` —
/// which is not a missing feature but a machine that cannot say where its own
/// state lives. Reporting that is not the same as reporting "no source is
/// wired": one is a fact about this invocation, the other was a fact about the
/// build.
pub const NO_STATE_ROOT: &str = "cannot derive the state root: neither AE_HOME nor HOME is set";

/// What the binary says for a top-level session name.
///
/// **SC-022** rules that such a token is a launch candidate and never an
/// unknown-subcommand error. Launching is not this slice's work, so the binary
/// says exactly that — SCAFFOLD, and no part of any
/// acceptance claim.
pub const NO_LAUNCHER: &str = "start is not implemented in this build";

/// The exit code for a request the binary understood but could not carry out.
///
/// Distinct from `0` and from SC-022's usage-error `2`: "you asked wrong" and
/// "it went wrong" stay tellable apart, which is the whole reason `2` exists.
///
/// **SCAFFOLD.** No row rules this code for either of the two requests that
/// currently end in it. It is the least-wrong placeholder while their real
/// surfaces are unratified, and it is deliberately not returned by
/// [`cli::Request::exit_code`], where it could be mistaken for contract.
pub const EXIT_UNAVAILABLE: u8 = 1;

/// Run the CLI against `args` (argv WITHOUT the program name).
///
/// The binary's entry point. `list` reads the real state root, enumerates,
/// classifies and renders; see [`run_with`] for the injected-source path the
/// suite drives, and [`listing::World`] for why the source is a parameter at
/// all.
///
/// # Errors
///
/// Returns [`Error::Io`] if `out` or `err` cannot be written or flushed.
///
/// ```
/// let (mut out, mut err) = (Vec::new(), Vec::new());
/// let code = ae::run(&["--version".to_owned()], &mut out, &mut err)?;
/// assert_eq!(code, 0);
/// assert_eq!(String::from_utf8(out).unwrap(), ae::version_line() + "\n");
/// assert!(err.is_empty());
/// # Ok::<(), ae::Error>(())
/// ```
pub fn run(args: &[String], out: &mut impl Write, err: &mut impl Write) -> Result<u8> {
    // Only a listing needs a source, and `next` only needs one once its argv has
    // been accepted: a refused word must not pay for a tmux scan of every
    // session before it can say so, which is what frozen's parse-then-scan order
    // already guaranteed.
    let wants_world = match cli::Request::parse(args) {
        cli::Request::List(_) => true,
        cli::Request::Next { tail } => next::parse(&tail).is_ok(),
        _ => false,
    };
    if wants_world && let Some(root) = state_root() {
        let (_snapshot, world) = current_world(&root);
        return run_with(args, Some(&world), out, err);
    }
    run_with(args, None, out, err)
}

/// The `_say` arm: the frozen `helper_say_main`.
///
/// It emits a `chat` event and says so — the bridge is what forwards it, and a
/// `say` with no bridge running is deliberately still a recorded line rather
/// than a refusal, because the event log is where the text belongs either way.
/// stdin is read when no text was given, which is how a multi-line reply
/// travels; a whitespace-only payload is the usage refusal, not an empty event.
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
    let _ = state::emit(
        dir,
        &tracked::event_line(&tracked::EventFields {
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
        }),
    );
    let head: String = text.chars().take(60).collect();
    let ellipsis = if text.chars().count() > 60 { "…" } else { "" };
    writeln!(out, "Sent to Telegram bridge (chat): {head}{ellipsis}")?;
    Ok(0)
}

/// The `_state` arm. Nothing after the directory is the READ — the caller's
/// latest declaration, always 0. A declaration: usage at 2, then the write,
/// and nothing reaches stdout until the bytes are down; every refusal or
/// failure is stderr plus a non-zero code.
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
///
/// The argv is answered BEFORE the world is consulted, exactly as frozen's loop
/// ran before its scan: `ae next --bogus` is a refusal whether or not any
/// session needs a human, and `--help` never depends on tmux being reachable.
///
/// Everything after the selection is the jump, and it is here rather than in
/// [`next`] because it runs tmux — the module stays pure so the ordering rules
/// can be tested without a server, which is the same split [`tmux`] and
/// [`transport`] already keep.
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

    // The ambient server, which is what the frozen `tmux()` shim resolves to
    // when no `AE_TMUX_SERVER` redirects it. The core has no shim — see
    // `calling_pane` — so this is a jump to the server the invocation is
    // already talking to, and never to one a session record names.
    let server = inventory::ServerId::Ambient;

    // Re-validate EXACTLY: the session may have ended between the scan and the
    // jump, and a prefix-matching `has-session -t` would land the focus on a
    // surviving sibling. A server that did not answer proves nothing is gone,
    // but it also cannot be jumped to, so it takes the same refusal.
    let still_there =
        transport::session_names(&server).is_some_and(|names| names.contains(&choice.name));
    if !still_there {
        writeln!(err, "ae next: '{}' disappeared before attach.", choice.name)?;
        return Ok(next::EXIT_NONE);
    }

    let inside = inside_tmux(&server, err)?;
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

/// Whether this invocation is really inside a tmux pane — frozen's
/// `_ae_inside_tmux`, and NOT a bare `$TMUX` test.
///
/// `$TMUX` is inherited, so a shell that is not a pane can carry one; with a
/// stale value `display-message` answers for the ORIGINAL pane's session and a
/// genuine jump would be suppressed as "already in". The tty comparison is what
/// separates the two, and an unanswerable comparison trusts `$TMUX` as ae always
/// has — a probe that cannot speak must not become a verdict.
fn inside_tmux(server: &inventory::ServerId, err: &mut impl Write) -> Result<bool> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: whether the caller sits in a pane is $TMUX, which tmux sets in every pane's environment — see clippy.toml"
    )]
    let tmux_env = std::env::var_os("TMUX");
    if tmux_env.is_none_or(|value| value.is_empty()) {
        return Ok(false);
    }
    let (Some(tty), Some(panes)) = (procs::own_tty(), transport::observe_pane_ttys(server)) else {
        return Ok(true);
    };
    if panes.is_empty() || tmux::tty_is_a_pane(&tty, &panes) {
        return Ok(true);
    }
    writeln!(
        err,
        "ae: stale $TMUX inherited (this shell is not a tmux pane) — attaching normally."
    )?;
    Ok(false)
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

/// The `_ask`/`_review` arm: the frozen `ae_tracked_send`, with the paste
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
/// webhook) names itself to the tracked-request helpers. Empty is unset.
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
/// abandons. The frozen body's `[[ $x =~ ^[0-9]+$ ]] || x=30` — a value that
/// is not a plain decimal is the default, not an error, because a typo in an
/// exported tunable must not stop delivery.
fn send_defer() -> std::time::Duration {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the frozen AE_SEND_DEFER_SEC tunable of the send body — see clippy.toml"
    )]
    let raw = std::env::var_os("AE_SEND_DEFER_SEC");
    raw.and_then(|value| value.to_str().and_then(|text| text.parse::<u64>().ok()))
        .map_or(deliver::DEFAULT_DEFER, std::time::Duration::from_secs)
}

/// The frozen event-field contract of the send body, read off this process's
/// environment exactly where `ae_emit_event` and `helper_send_body` read it:
/// `AE_SENDER_OVERRIDE` and the seven `_AE_EVENT_*` members, an unset or
/// empty variable being none. One door, one reading.
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
/// process, which is the same quality the frozen `uuidgen | cut` suffix has,
/// and needs no door.
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
            return match memo::read(dir, &view) {
                Ok(bytes) => {
                    out.write_all(&bytes)?;
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
///
/// No `TMUX_PANE`, an empty one, or a pane the server does not answer for is
/// [`requests::Viewer::default`] — no identity — and the requests surface then
/// refuses `mine`/`inbox`. See the `requests` module docs for why this does not
/// copy the frozen helper's fallback to the server's current pane.
fn calling_viewer(dir: &std::path::Path) -> requests::Viewer {
    calling_pane(dir)
        .map(|observed| requests::Viewer::from_pane(&observed, &own_session(dir)))
        .unwrap_or_default()
}

/// The tmux server a helper reads its OWN pane on: the session's recorded
/// selector when usable, else the ambient server.
///
/// The bash helpers read `TMUX_PANE` off `$AE_TMUX_SERVER` (the shim ae exports
/// into every session's environment); the core does not read that env, so it
/// reads the same server from the session's recorded selector instead — they
/// name the same server, since the launch pins one from the other. An unusable
/// selector degrades to the ambient server (who-am-I then simply finds no
/// identity, exactly as an unstamped pane does) rather than refusing: this is a
/// read of the caller's own identity, not a destructive routing decision.
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

/// `session=` in `<dir>/meta` — what the frozen `_lib` reads into
/// `_AE_SESSION`, empty-or-missing folded to `None`.
fn session_key(dir: &std::path::Path) -> Option<String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the session name a helper serves, read the way _lib reads _AE_SESSION — see clippy.toml"
    )]
    let raw = std::fs::read(dir.join("meta"));
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

/// Where this invocation's state lives — **SC-404**'s default derivation.
///
/// `AE_HOME` if it names something, else `<HOME>/.ae`. An empty value is
/// treated as unset rather than as the root of the filesystem: the alternative
/// is deriving `/sessions` from a variable someone exported blank, which is a
/// worse answer than saying the root cannot be derived.
///
/// **SC-1410a is UNCLASSIFIED** — the variable's unset/override/malformed
/// semantics are not ratified. This implements only what SC-404 already says
/// (the derivation and its default) and refuses rather than guessing where the
/// row is silent. A relative `AE_HOME` is used as given; nothing here rewrites
/// it, because normalising a path the operator supplied is a decision no row
/// makes.
pub(crate) fn state_root() -> Option<std::path::PathBuf> {
    let named = |key: &str| {
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: SC-404's state-root derivation — see clippy.toml"
        )]
        let raw = std::env::var_os(key);
        raw.filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from)
    };
    named("AE_HOME").or_else(|| named("HOME").map(|home| home.join(".ae")))
}

/// The classified snapshot AND the world `ae list` shows right now — the real
/// route, returned in both halves so it can be observed from outside.
///
/// **Both halves, deliberately.** The phase-3 gate's criterion 2 requires the
/// presentation input to equal the completed classified set, and a test that
/// entered presentation itself was observing a boundary IT chose rather than the
/// one the CLI crosses: anything this function did between classification and
/// entry was invisible to it. Returning the snapshot beside the world makes both
/// sides comparable on the route the product actually takes.
///
/// Phase 1 discovers, phase 2 classifies, phase 3 renders — and the transport is
/// [`transport::Tmux`], which runs the real thing. A candidate whose recorded
/// server answers is `running` or `stopped` on that answer; a candidate whose
/// server does not answer is still `unknown`, because SC-017l is about what was
/// established and not about which build is running.
#[must_use]
pub fn current_world(root: &std::path::Path) -> (liveness::Snapshot, listing::World) {
    let scan = inventory::durable_records(&inventory::Roots::under(root));
    // No ambient server: selecting one is SC-1410c's unratified question, and
    // entitlement without a pointer is exactly what SC-017j forbids.
    let taken = inventory::take(scan, None, &transport::Tmux);
    let snapshot = liveness::classify(taken, &transport::Tmux);
    // Criterion 3 only: the opposed disk must change HERE, on this function's
    // path, not after it returns. A test that mutates then calls
    // `Presentation::enter` itself is below the list/ls caller. Compiled out
    // when `debug_assertions` is off.
    //
    // Not precedent for the rejected transport seam. The rejected route-(c)
    // seam would have created a presentation-only product route supplying a
    // product fact. This callback schedules a product-valid external change
    // inside the existing route and can inject nothing — not a snapshot, a
    // World, a Discovery, or a tmux substitute. It serves a ratified
    // criterion's mandated arm (C3) and is compiled out when
    // `debug_assertions` is off. Single-purpose: a new consumer needs a new
    // ruling.
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
///
/// The facts no session directory holds, and the ones the frozen `cmd_list`
/// read straight from tmux: each running session's pane enumeration (SC-017p/q's
/// per-agent liveness, and the vanished-pane `dead` the attention rollup raises)
/// and the branch its watchdog publishes (SC-405g's live half).
///
/// **Only a RUNNING session is asked.** A `stopped` classification already
/// proves every pane of it gone, and an `unknown` one establishes nothing that a
/// pane query could repair — so both would spend a round-trip to learn what the
/// status already said.
///
/// Every read is allowed to fail into "nothing observed": a session with no
/// entitled server, a pane query that did not come back, an absent branch
/// option. None of those may become a verdict, because a failed question is the
/// one thing SC-017q says must never look like an answer.
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
///
/// **Single-purpose.** Criterion 3's opposed-disk plant, and nothing else. A
/// new consumer requires a new ruling. Not the rejected transport seam: that
/// route-(c) hook would have created a presentation-only product route
/// supplying a product fact. This callback schedules a product-valid external
/// change inside the existing route and can inject nothing. Compiled out when
/// `debug_assertions` is off. Integration tests cannot see `cfg(test)` on this
/// crate, which is why the hook is not `cfg(test)`.
#[cfg(debug_assertions)]
pub fn set_after_classify_hook(hook: Option<fn(&std::path::Path)>) {
    AFTER_CLASSIFY.with(|slot| slot.set(hook));
}

/// Run the CLI against `args` over `world` — the injected session source.
///
/// `None` means the caller could not supply one, which after three phases is no
/// longer "nothing is wired" but "this invocation has no state root". [`run`]
/// supplies a real world; the suite supplies fixtures.
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
        // Asked-for output goes to stdout. A DIAGNOSTIC never does — SC-022 —
        // because a machine reading `ae list --json` must not have to tell the
        // document apart from the complaint about the document.
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
            // session's meta directory. The telegram bridge is machine-global —
            // one bot, one chat, every session — so it takes AE_HOME, and being
            // told to supply a meta directory would send an operator looking
            // for a path that has nothing to do with it.
            let operand = if *command == cli::TELEGRAM_RUN {
                "an ae home directory"
            } else if *command == cli::NET_PROBE {
                // Not a directory at all: the one entry here that takes a name
                // to resolve. Telling its caller to supply a meta directory
                // would send them looking for a path with nothing to do with it.
                "a host to resolve"
            } else {
                "a session meta directory"
            };
            writeln!(err, "ae: {command} needs {operand}")?;
            request.exit_code().unwrap_or(2)
        }
        // The frozen helper writes its table and its refusal to the streams a
        // pane reads, and so does this: the refusal is a DIAGNOSTIC and never
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
        cli::Request::Requests { dir, mode } => {
            let rendered = requests::render(dir, *mode, &calling_viewer(dir));
            out.write_all(&rendered.stdout)?;
            err.write_all(&rendered.stderr)?;
            rendered.code
        }
        // NEVER RETURNS. The surface has no completion condition — see
        // `events_tail::follow` — so the only way out is a signal or a write
        // failure, and the write failure is the one this arm can report.
        cli::Request::EventsTail { dir } => match events_tail::follow(dir, out)? {},
        cli::Request::LaunchPlan { tail } => identity::launch_plan(tail, out, err)?,
        // The only entry whose payload arrives on STDIN: a launch's seat list
        // is many records, and an argv is not the place for a document. Read to
        // the end BEFORE anything is published, so a truncated pipe is a
        // refusal the trailer catches rather than a half-built roster.
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
        // The three whole lifecycle operations. They take a session NAME, so
        // they derive the state root for themselves — the same derivation
        // `list` uses, and the same refusal when there is none.
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
        cli::Request::WatchdogRun { dir, knobs } => watchdog_daemon::run(dir, *knobs, out, err)?,
        cli::Request::TelegramRun { paths, knobs } => telegram::bridge::run(paths, *knobs, err)?,
        cli::Request::NetProbe { host, port } => netprobe::run(host, *port, out, err)?,
        cli::Request::CompactMemoBaseline { dir } => compact::memo_baseline_step(dir, out)?,
        cli::Request::CompactFindOutstanding { dir } => compact::find_outstanding_step(dir, out)?,
        cli::Request::List(list_args) => {
            if let Some(world) = world {
                // SC-017o: the warning goes to STDERR and the table still
                // prints. A machine reading `--json` must not have to tell the
                // document apart from the complaint about it, and a human must
                // not have to notice a missing row to learn ae could not look.
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
        cli::Request::Next { tail } => run_next(tail, world, out, err)?,
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
        std::fs::write(dir.join("meta"), "name=x\nsession=renamed\n").unwrap();
        assert_eq!(super::own_session(&dir), "renamed");
        std::fs::write(dir.join("meta"), "session=\n").unwrap();
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
        assert_eq!(
            String::from_utf8(out).unwrap(),
            format!("ae {}\n", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn help_names_every_command_the_binary_actually_carries() {
        // CONTENT, not layout (SC-012 owns the surface and is not this slice's
        // row): each shipped spelling has to be findable in the help text, so a
        // command can never ship without appearing here.
        let text = help_text();
        for surface in ["list", "ls", "--json", "--help", "--version"] {
            assert!(text.contains(surface), "help omits {surface}: {text}");
        }
    }

    #[test]
    fn the_help_the_binary_prints_is_the_help_text() {
        // Both routes — the flag and the bare invocation — and no second copy.
        for words in [vec!["--help"], vec!["-h"], vec!["help"], vec![]] {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let code = run(&argv(&words), &mut out, &mut err).unwrap();
            assert_eq!(code, 0, "{words:?}");
            assert_eq!(String::from_utf8(out).unwrap(), help_text(), "{words:?}");
            assert!(err.is_empty(), "{words:?}");
        }
    }

    #[test]
    fn sc_022_an_unknown_option_is_diagnosed_on_stderr_with_stdout_empty() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&["--nope".to_owned()], &mut out, &mut err).unwrap();
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
        let code = run(&["my-feature".to_owned()], &mut out, &mut err).unwrap();
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
    fn run_with_no_arguments_prints_help() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&[], &mut out, &mut err).unwrap();
        assert_eq!(code, 0);
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("Usage: ae"),
            "help text missing usage: {text}"
        );
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
        // What is left of the old refusal. It is no longer "no source is wired"
        // — that was a fact about the BUILD and it is gone — but "this machine
        // did not tell me where its state lives", which is a fact about the
        // invocation. Not 0, not the usage 2, nothing on stdout that could be
        // mistaken for an empty listing.
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
        assert!(!listing.contains("old"), "SC-017a: running only");
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
            "SC-509d: every successor digest is version 2"
        );
        assert_eq!(
            value.get("inventory_complete"),
            Some(&crate::json::Value::Bool(true)),
            "SC-017o: and every successor digest carries the completeness fact"
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
        // The unwired report writes to `err`. If that write fails the failure
        // is surfaced, not swallowed — the same contract stdout already had.
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
