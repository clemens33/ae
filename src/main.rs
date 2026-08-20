//! The `ae` binary. Thin by design: argv in, exit code out, presentation of the
//! one top-level error. Everything testable lives in the library.

use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let stderr = std::io::stderr();
    let mut err = stderr.lock();

    match ae::run(&args, &mut out, &mut err) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            // Presentation itself failed. Say so on a fresh stderr handle and
            // give up quietly if that is gone too — the locked one above may be
            // the very thing that broke.
            let _ = writeln!(std::io::stderr(), "ae: {error}");
            ExitCode::FAILURE
        }
    }
}
