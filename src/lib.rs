//! `ae` — agent environment: a tmux-backed multi-agent session multiplexer.
//!
//! This is the P0 skeleton of the Rust rewrite (epic #79). It exists so every
//! quality lane — fmt, clippy, nextest, doctests, coverage, mutants — runs
//! against real code from the first commit rather than against a placeholder.
//!
//! Module layout is 2018-edition style: `cli.rs` beside a future `cli/`, never
//! a `mod.rs`.
//!
//! ```
//! let request = ae::cli::Request::parse(&["--version".to_owned()]);
//! assert_eq!(request, ae::cli::Request::Version);
//! assert_eq!(request.exit_code(), 0);
//! ```

pub mod cli;
pub mod error;

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
        "{}\n\nUsage: ae [OPTIONS]\n\nOptions:\n  -h, --help     Print help\n  -V, --version  Print version\n",
        version_line()
    )
}

/// Run the CLI against `args` (argv WITHOUT the program name), writing to `out`.
///
/// Returns the process exit code. Writing is fallible — a closed pipe is the
/// ordinary case, not an exceptional one — so the caller gets a [`Result`] and
/// presents the failure itself.
///
/// # Errors
///
/// Returns [`Error::Io`] if `out` cannot be written or flushed.
///
/// ```
/// let mut out = Vec::new();
/// let code = ae::run(&["--version".to_owned()], &mut out)?;
/// assert_eq!(code, 0);
/// assert_eq!(String::from_utf8(out).unwrap(), ae::version_line() + "\n");
/// # Ok::<(), ae::Error>(())
/// ```
pub fn run(args: &[String], out: &mut impl Write) -> Result<u8> {
    let request = cli::Request::parse(args);
    match &request {
        cli::Request::Version => writeln!(out, "{}", version_line())?,
        cli::Request::Help => write!(out, "{}", help_text())?,
        cli::Request::Unknown(arg) => writeln!(out, "ae: unknown argument: {arg}")?,
    }
    out.flush()?;
    Ok(request.exit_code())
}

#[cfg(test)]
mod tests {
    use super::{Error, run, version_line};
    use std::io::{self, Write};

    #[test]
    fn version_line_names_the_tool_and_the_crate_version() {
        assert_eq!(version_line(), format!("ae {}", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn run_writes_the_version_and_succeeds() {
        let mut out = Vec::new();
        let code = run(&["--version".to_owned()], &mut out).unwrap();
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
    fn run_reports_an_unknown_argument_and_fails() {
        let mut out = Vec::new();
        let code = run(&["--nope".to_owned()], &mut out).unwrap();
        assert_eq!(code, 2);
        assert!(String::from_utf8(out).unwrap().contains("--nope"));
    }

    #[test]
    fn run_with_no_arguments_prints_help() {
        let mut out = Vec::new();
        let code = run(&[], &mut out).unwrap();
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
    fn run_surfaces_a_write_failure() {
        let err = run(&["--version".to_owned()], &mut ClosedPipe).err();
        assert!(matches!(err, Some(Error::Io(_))), "expected an io error");
    }
}
