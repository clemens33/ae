//! `ae` — agent environment: a tmux-backed multi-agent session multiplexer.
//!
//! The Rust rewrite (epic #79). P0 laid the skeleton so every quality lane —
//! fmt, clippy, nextest, doctests, coverage, mutants — runs against real code;
//! P1 is adding the read side, slice by slice.
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
//! # The read side so far (P1 slice 1: `list --json`)
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
//! | [`session`] | SC-017e, SC-017g, SC-405d/f/g/i/j/k, SC-518, SC-520, SC-980 — what a session directory establishes, and what it must be told |
//! | [`listing`] | SC-017f, SC-017h, SC-509, SC-506 — the two renderings of one selection, and the injected world they read |
//! | [`cli`] | SC-021, and the argv half of SC-017a–i / SC-521a/b — which word is `list`, and which parser owns its flags |
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

pub mod attention;
pub mod cli;
pub mod digest;
pub mod error;
pub mod events;
pub mod filters;
pub mod inventory;
pub mod json;
pub mod listing;
pub mod liveness;
pub mod meta;
pub mod session;
pub mod time;
pub mod tmux;

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
         Options:\n  \
         -h, --help     Print help\n  \
         -V, --version  Print version\n",
        version_line()
    )
}

/// What the binary says when `list` has nowhere to read sessions from.
///
/// Enumerating session directories and deciding running-vs-stopped are surfaces
/// no ratified row defines yet — see [`listing::World`]. Until one does, `ae
/// list` reports that rather than printing an empty listing, which on a machine
/// that HAS sessions would be a wrong answer wearing the shape of a right one.
pub const NO_SESSION_SOURCE: &str =
    "list: no session source is wired yet — enumeration and liveness are not ratified surfaces";

/// What the binary says for a top-level session name.
///
/// **SC-022** rules that such a token is a launch candidate and never an
/// unknown-subcommand error. Launching is not this slice's work, so the binary
/// says exactly that — SCAFFOLD, like [`NO_SESSION_SOURCE`], and no part of any
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
/// The binary's entry point: there is no session source wired, so `list` ends
/// in [`NO_SESSION_SOURCE`] on `err` with [`EXIT_UNAVAILABLE`]. See
/// [`run_with`] for the tested path, and [`listing::World`] for why the source
/// is a parameter at all.
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
    run_with(args, None, out, err)
}

/// Run the CLI against `args` over `world` — the injected session source.
///
/// `None` is not a missing argument but a FACT: no source is wired. It is the
/// state the shipped binary is in, so it is a state this function models rather
/// than one the caller has to remember to handle.
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
        cli::Request::List(list_args) => {
            if let Some(world) = world {
                write!(out, "{}", listing::render(list_args, world))?;
                0
            } else {
                writeln!(err, "ae: {NO_SESSION_SOURCE}")?;
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
        EXIT_UNAVAILABLE, Error, NO_LAUNCHER, NO_SESSION_SOURCE, help_text, listing::World, run,
        run_with, version_line,
    };
    use crate::digest::{SessionEntry, Status};
    use crate::time::Timestamp;
    use std::io::{self, Write};

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
    fn a_list_with_no_wired_source_says_so_on_stderr_and_exits_one() {
        // The decision, in one test: not 0, not the usage 2, nothing on stdout
        // that could be mistaken for an empty listing.
        for words in [vec!["list"], vec!["ls"], vec!["list", "--all", "--json"]] {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let code = run(&argv(&words), &mut out, &mut err).unwrap();
            assert_eq!(code, EXIT_UNAVAILABLE, "{words:?}");
            assert_ne!(code, 0, "{words:?}");
            assert_ne!(code, 2, "{words:?}");
            assert!(out.is_empty(), "{words:?}: stdout was {out:?}");
            let message = String::from_utf8(err).unwrap();
            assert!(message.contains(NO_SESSION_SOURCE), "{words:?}: {message}");
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
