//! The `ae` binary. Thin by design: argv in, exit code out, presentation of the
//! one top-level error. Everything testable lives in the library.
//!
//! `argv[0]` is kept, not skipped: since slice Z2 every session helper is a
//! symlink to this binary, and the name it was invoked under is what says which
//! helper this is and which session it belongs to.

use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut whole = std::env::args();
    let program = whole.next();
    let args: Vec<String> = whole.collect();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let stderr = std::io::stderr();
    let mut err = stderr.lock();

    match ae::run_program(program.as_deref(), &args, &mut out, &mut err) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            // Presentation itself failed.
            let _ = writeln!(std::io::stderr(), "ae: {error}");
            ExitCode::FAILURE
        }
    }
}
