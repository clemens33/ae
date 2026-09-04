//! The test suite's ONE door to a child process.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};

/// One command to run: the program, its arguments, and its environment.
#[derive(Debug)]
pub(crate) struct Invocation {
    program: OsString,
    args: Vec<OsString>,
    env: BTreeMap<OsString, OsString>,
    env_cleared: bool,
}

impl Invocation {
    /// An invocation of `program` with no arguments and the inherited
    /// environment.
    pub(crate) fn new<S: AsRef<OsStr>>(program: S) -> Self {
        Self {
            program: program.as_ref().to_os_string(),
            args: Vec::new(),
            env: BTreeMap::new(),
            env_cleared: false,
        }
    }

    /// Append one argument.
    #[must_use]
    pub(crate) fn arg<S: AsRef<OsStr>>(mut self, arg: S) -> Self {
        self.args.push(arg.as_ref().to_os_string());
        self
    }

    /// Set one environment variable.
    #[must_use]
    pub(crate) fn env<K: AsRef<OsStr>, V: AsRef<OsStr>>(mut self, key: K, value: V) -> Self {
        self.env
            .insert(key.as_ref().to_os_string(), value.as_ref().to_os_string());
        self
    }

    /// Drop the inherited environment, keeping only what [`Invocation::env`]
    /// sets.
    #[must_use]
    pub(crate) fn env_cleared(mut self) -> Self {
        self.env_cleared = true;
        self
    }
}

/// Where a child's evidence enters this harness.
pub(crate) mod capture {
    /// Whether a child exited with a code, or was killed.
    #[derive(Clone, Copy)]
    pub(crate) enum ExitOutcome {
        /// The process exited with this status code.
        Code(i32),
        /// The process was terminated by a signal.
        Signalled,
    }

    pub(crate) mod raw {
        //! The one place a child process is run — and the only thing this
        //! harness ever holds of what one produced.

        use std::fs::File;
        use std::io;
        use std::path::Path;
        use std::process::{ExitStatus, Stdio};

        use super::super::Invocation;
        use super::ExitOutcome;

        /// A finished child's exit status, and nothing else about it.
        pub(crate) struct RawStatus(ExitStatus);

        /// Build and run `invocation` in `cwd`, streams wired to `out` and `err`.
        ///
        /// # Errors
        ///
        /// If either artifact file cannot be created, or the child cannot be
        /// spawned.
        // THE HARNESS'S DOOR — the only place in the PARITY HARNESS that may
        // name `std::process::Command`. There are two others crate-wide, each a
        // different job: `tests/it/cli.rs`, whose black-box tests must run the
        // product binary and which is private to that module, and
        // `src/transport.rs`, THE PRODUCT'S — ae cannot answer a liveness
        // question without running tmux. That third one is not reachable from
        // here: `transport::run` is private and the public transport only ever
        // spawns tmux with an argument list `src/tmux.rs` derived.
        //
        // `clippy.toml` denies the type everywhere else, which resolves TYPES
        // rather than text and so holds against UFCS, aliases and re-imports
        // alike. `#[allow]` and not `forbid` at the crate level, because forbid
        // would block this door too — so the residual is that a further
        // relaxation opens a further door.
        // `the_doors_to_a_child_process_are_the_inventoried_ones` inventories them by file
        // and count. That inventory is TEXTUAL and sees only the relaxation
        // forms it enumerates; the type deny is SEMANTIC and closes the class.
        // Do not describe them as one thing — see the module docs.
        #[allow(
            clippy::disallowed_types,
            reason = "the pinned door: see clippy.toml for why one type is the whole boundary"
        )]
        pub(crate) fn run(
            invocation: &Invocation,
            cwd: &Path,
            out: &Path,
            err: &Path,
        ) -> io::Result<RawStatus> {
            let mut command = std::process::Command::new(&invocation.program);
            command.args(&invocation.args).current_dir(cwd);
            if invocation.env_cleared {
                command.env_clear();
            }
            for (key, value) in &invocation.env {
                command.env(key, value);
            }
            command
                .stdin(Stdio::null())
                .stdout(Stdio::from(File::create(out)?))
                .stderr(Stdio::from(File::create(err)?));
            command.status().map(RawStatus)
        }

        impl RawStatus {
            /// The one legal consumption: status becomes an outcome.
            pub(crate) fn outcome(&self) -> ExitOutcome {
                self.0
                    .code()
                    .map_or(ExitOutcome::Signalled, ExitOutcome::Code)
            }
        }
    }
}
