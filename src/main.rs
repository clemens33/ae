//! The `ae` binary. Thin by design: argv in, exit code out, presentation of the
//! one top-level error. Everything testable lives in the library.

use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    match ae::run(&args, &mut out) {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            // Presentation failed on stdout; say so on stderr and give up
            // quietly if that is gone too.
            let _ = writeln!(std::io::stderr(), "ae: {err}");
            ExitCode::FAILURE
        }
    }
}
