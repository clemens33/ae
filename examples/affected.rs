//! The merge-gate selector `just test-affected` runs: the recipe's change stream on stdin, the
//! selection protocol on stdout.
//!
//! The selector lives in the integration suite (`tests/it/gate/affected.rs`), beside its
//! acceptance tests and the module-tree walk it shares with the Mac subset guard. This example
//! mounts both files and owns only the process edges. A malformed stream still prints a
//! selection: the whole suite and the install checks, with reason `stream`. A failed read of
//! stdin or write to stdout exits non-zero, and the recipe then runs both as well.

use std::io::{Read as _, Write as _};
use std::process::ExitCode;

#[allow(
    dead_code,
    reason = "the module walk also serves the Mac subset guard; the selector uses part of it"
)]
#[path = "../tests/it/gate/modtree.rs"]
mod modtree;

#[allow(
    dead_code,
    reason = "the selector's fields and types also serve its acceptance tests"
)]
#[path = "../tests/it/gate/affected.rs"]
mod affected;

fn main() -> ExitCode {
    let mut stream = Vec::new();
    if std::io::stdin().lock().read_to_end(&mut stream).is_err() {
        return ExitCode::FAILURE;
    }
    let mut stdout = std::io::stdout().lock();
    match stdout
        .write_all(affected::respond(&stream).as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
