//! The `desk` session helper: show the orchestrator's board, or let the
//! orchestrator seat replace it.
//!
//! `desk` and `desk show` print the board in the input spelling and need no
//! proof: they read this session's own record. `desk check [--all]` compares
//! it with what ae owes the owner, read-only and also without proof
//! ([`crate::deskcheck`]). `desk set [<file>|-]` replaces
//! the whole board. The caller is proved to be this session's orchestrator
//! seat BEFORE any input is read, so an ordinary caller never has a file or
//! its standard input touched, and again under the writer lock by
//! [`deskstore::publish_admitted`], which carries that first proof and refuses
//! unless the second is the same seat. Every refusal says `desk set REFUSED`, changes
//! nothing and prints nothing on stdout.

use std::io::{self, Read, Write};
use std::path::Path;

use crate::desk::{self as record, INPUT_CAP};
use crate::deskcheck;
use crate::deskstore::{self, ReadError, Rendered, render_set};
use crate::relay::Observed;
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::store::{self, SourceRead};
use crate::time::Timestamp;

#[cfg(test)]
#[path = "desk/cmd_tests.rs"]
mod cmd_tests;

/// What `desk` was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Command<'a> {
    /// Print the board.
    Show,
    /// Replace it from this file, or from standard input.
    Set(Option<&'a str>),
    /// Compare the board with the obligations of the fleet; `all` lifts the cut
    /// of the known-closed list.
    Check { all: bool },
}

/// Why an argv is no command: a bare `-` is standard input, every other word
/// after `set` is a file.
fn parse(tail: &[String]) -> Option<Command<'_>> {
    let words: Vec<&str> = tail.iter().map(String::as_str).collect();
    match words.as_slice() {
        [] | ["show"] => Some(Command::Show),
        ["set"] | ["set", "-"] => Some(Command::Set(None)),
        ["set", path] => Some(Command::Set(Some(path))),
        ["check"] => Some(Command::Check { all: false }),
        ["check", "--all"] => Some(Command::Check { all: true }),
        _ => None,
    }
}

/// The board as `desk show` prints it, or the reason it cannot.
fn show(dir: &Path, out: &mut impl Write, err: &mut impl Write) -> io::Result<u8> {
    match deskstore::read(dir) {
        Ok(None) => {
            writeln!(out, "no board set")?;
            Ok(0)
        }
        Ok(Some(desk)) => {
            out.write_all(record::render_input(&desk.items).as_bytes())?;
            Ok(0)
        }
        Err(why @ (ReadError::NotRegular | ReadError::TooLarge | ReadError::Io(_))) => {
            writeln!(err, "desk show: the board record cannot be read: {why}")?;
            Ok(EXIT_FAILED)
        }
        Err(ReadError::Parse(why)) => {
            writeln!(
                err,
                "desk show: the board record is damaged ({why}); set the board again to repair it"
            )?;
            Ok(EXIT_FAILED)
        }
    }
}

/// The input of a set, at most one byte over the cap so the grammar can refuse
/// an oversize text by name: a file is read without opening a node that is not
/// a regular file, standard input is read as far as the cap.
fn input(source: Option<&str>, stdin: &mut impl Read) -> Result<Vec<u8>, String> {
    let cap = INPUT_CAP as u64 + 1;
    let Some(path) = source else {
        let mut bytes = Vec::new();
        stdin
            .take(cap)
            .read_to_end(&mut bytes)
            .map_err(|why| format!("cannot read standard input: {why}"))?;
        return Ok(bytes);
    };
    match store::read_capped(Path::new(path), cap) {
        Ok(SourceRead::Ready(bytes)) => Ok(bytes),
        Ok(SourceRead::Absent) => Err(format!("cannot read {path}: no such file")),
        Ok(SourceRead::Invalid(why)) => Err(format!("{path} is not a regular file: {why}")),
        Ok(SourceRead::Unreadable(why)) => Err(format!("cannot read {path}: {why}")),
        Err(store::Oversized) => Err(format!("{path} is longer than {INPUT_CAP} bytes")),
    }
}

fn refuse(why: &str) -> Rendered {
    render_set(&Err(deskstore::SetError::Admission(why.to_owned())), 0)
}

/// The refusal of an input the grammar rejects: `input line <n>: <reason>`, or
/// `input: <reason>` when the whole text is at fault.
fn grammar_refusal(why: &record::ParseError) -> String {
    if why.line == 0 {
        format!("input: {why}")
    } else {
        format!("input {why}")
    }
}

/// The helper body: `tail` is everything after the helper's name, `observe`
/// reads the calling pane afresh on every call, and `stdin` is the process's
/// own standard input.
///
/// # Errors
///
/// A failed write to `out` or `err`.
pub(crate) fn run(
    dir: &Path,
    tail: &[String],
    observe: &mut impl FnMut() -> Option<Observed>,
    now: Timestamp,
    stdin: &mut impl Read,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let rendered = match parse(tail) {
        None => {
            writeln!(
                err,
                "Usage: desk show\n       desk set [<file>|-]\n       desk check [--all]\nThe board replaces whole: `desk set` reads the complete board from a file or standard input."
            )?;
            return Ok(EXIT_USAGE);
        }
        Some(Command::Show) => return show(dir, out, err),
        Some(Command::Check { all }) => return deskcheck::run(dir, all, out, err),
        Some(Command::Set(source)) => set(dir, source, observe, now, stdin),
    };
    out.write_all(rendered.stdout.as_bytes())?;
    err.write_all(rendered.stderr.as_bytes())?;
    Ok(rendered.code)
}

fn set(
    dir: &Path,
    source: Option<&str>,
    observe: &mut impl FnMut() -> Option<Observed>,
    now: Timestamp,
    stdin: &mut impl Read,
) -> Rendered {
    let admitted = match deskstore::admit_caller(dir, observe) {
        Ok(admitted) => admitted,
        Err(denied) => return render_set(&Err(denied), 0),
    };
    let bytes = match input(source, stdin) {
        Ok(bytes) => bytes,
        Err(why) => return refuse(&why),
    };
    let items = match record::parse_input(&bytes) {
        Ok(items) => items,
        Err(why) => return refuse(&grammar_refusal(&why)),
    };
    let total = items.len();
    render_set(
        &deskstore::publish_admitted(dir, &admitted, observe, items, now),
        total,
    )
}
