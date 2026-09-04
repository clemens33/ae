//! The test suite's ONE door to a child process.
//!
//! # What this is
//!
//! An [`Invocation`] — a program, its arguments and its environment — and the
//! single function that runs one, wiring its streams straight to files. Seven
//! modules in this target reach a real `tmux`, `git` or `just` through it.
//!
//! # Why it is a door and not a convenience
//!
//! `std::process::Command` is a DENIED TYPE crate-wide (`clippy.toml`). Two
//! places relax it, and they do different jobs: this one, and `tests/it/cli.rs`,
//! whose black-box tests must run the product binary and whose factory is
//! private to that module, so nothing here can reach a child through it. That
//! is a capability boundary rather than a naming convention ONLY because this
//! crate forbids `unsafe_code`; the premise is written down at the pin site,
//! because it can stop being true. `tests/it/doors.rs` asks clippy itself
//! whether the boundary still holds, and pins that a child is run in exactly
//! ONE place here.
//!
//! # The bytes are never in this process
//!
//! A child's stdout and stderr go straight to their artifact files: the file
//! descriptors are handed over before the child starts, and nothing here ever
//! owns a byte of them. That is a stronger statement than "the bytes are behind
//! a private field", because possession is capability — code that holds bytes
//! in order to write them can equally compare them to something it expects, and
//! no Rust mechanism grants write-only access to a value you must serialise. So
//! the value is never obtained.
//!
//! What is unavoidably held is the child's exit STATUS: something must turn it
//! into the `exit` artifact's text, and [`capture::raw::RawStatus`] is
//! deliberately the whole of it — a private field, no `Debug`, one method.
//!
//! # History
//!
//! This was the plumbing of a PARITY harness: it ran one corpus through a bash
//! lane and a core lane and kept both sets of raw artifacts side by side, so
//! that a later stage could ask whether they agreed. Slice Z4 retired the bash
//! it was comparing against, and the lane, corpus, manifest and pairing
//! machinery went with it. The door stayed, because every real-server test in
//! this target runs through it.

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
    ///
    /// A lane that inherits the operator's environment is a lane whose result
    /// depends on whose laptop ran it.
    #[must_use]
    pub(crate) fn env_cleared(mut self) -> Self {
        self.env_cleared = true;
        self
    }
}

/// Where a child's evidence enters this harness.
pub(crate) mod capture {
    /// Whether a child exited with a code, or was killed.
    ///
    /// `Signalled` rather than a number: the signal itself is not portable to
    /// record here, and a capture that guesses is worse than one that says it
    /// does not know.
    ///
    /// No `Debug`, `PartialEq` or `Eq`. A caller may branch on the variant it
    /// asked for; it may not compare two outcomes to each other, which is the
    /// shape a verdict would take.
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
        //!
        //! # The bytes are never in this process
        //!
        //! A lane's stdout and stderr go from the child STRAIGHT to their
        //! artifact files: the file descriptors are handed over before the
        //! child starts, and nothing here ever owns a byte of them. That is a
        //! stronger statement than "the bytes are behind a private field",
        //! because possession is capability — code that holds bytes in order to
        //! write them can equally compare them to something it expects, and no
        //! Rust mechanism grants write-only access to a value you must
        //! serialise. So the value is never obtained.
        //!
        //! What is unavoidably held is the child's exit STATUS: something must
        //! turn it into the `exit` artifact's text. [`RawStatus`] is that
        //! something, and it is deliberately the whole of it — a private field,
        //! no `Debug` (`std::process::ExitStatus` has one, and
        //! `format!("{status:?}")` is a read), and exactly one method.

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
        /// The `Command` is built HERE rather than handed in, and that is the
        /// point rather than tidiness: both of reviewer4's round-4 injections
        /// open with `let output = command.output()?;`, and a caller that has no
        /// `command` cannot write that line. What remains — building a fresh
        /// `Command` from scratch somewhere else — is what the one-call-site pin
        /// is for.
        ///
        /// `stdin` is nulled explicitly. `Command::output` does that for you and
        /// `Command::status` does NOT: it inherits, and a lane that read from a
        /// terminal would hang a test run instead of finishing.
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
