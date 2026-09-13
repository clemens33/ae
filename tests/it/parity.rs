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

        use std::fs::{self, File};
        use std::io;
        use std::path::{Path, PathBuf};
        use std::process::{ExitStatus, Stdio};
        use std::sync::Once;

        #[cfg(unix)]
        use std::os::unix::fs::FileTypeExt as _;

        use super::super::Invocation;
        use super::ExitOutcome;

        #[cfg(unix)]
        const FIXTURE_REGISTRY: &str = ".ae-parity-fixtures";

        /// A finished child's exit status, and nothing else about it.
        pub(crate) struct RawStatus(ExitStatus);

        /// Build and run `invocation` in `cwd`, streams wired to `out` and `err`.
        ///
        /// # Errors
        ///
        /// If either artifact file cannot be created, or the child cannot be
        /// spawned.
        // The reaper runs through the same child-process door below.
        pub(crate) fn run(
            invocation: &Invocation,
            cwd: &Path,
            out: &Path,
            err: &Path,
        ) -> io::Result<RawStatus> {
            if invocation.program == "tmux" {
                static REAP_ORPHANS: Once = Once::new();
                REAP_ORPHANS.call_once(reap_orphaned_tmux_servers);
                // Write ahead: a SIGKILL after tmux creates its socket must
                // still leave an entry for another test process to reap.
                register_fixture_root(cwd)?;
            }
            let status = run_unreaped(invocation, cwd, out, err);
            if invocation.program == "tmux"
                && invocation
                    .env
                    .keys()
                    .any(|key| key == "AE_PARITY_KILL_AFTER_TMUX_CREATE")
            {
                let _ = run_unreaped(
                    &Invocation::new("kill")
                        .arg("-KILL")
                        .arg(std::process::id().to_string()),
                    cwd,
                    out,
                    err,
                );
            }
            status
        }

        /// Reap fixture servers whose test-process owner has already died.
        ///
        /// A `Drop` guard handles normal unwinding. SIGKILL cannot unwind, so
        /// the next test process must remove the server before it creates or
        /// addresses another one. The lane-owned write-ahead registry records
        /// each fixture's owner and root, so this never scans foreign /tmp.
        #[allow(
            clippy::disallowed_methods,
            reason = "the parity door reaps only test-owned fixture roots after SIGKILL"
        )]
        fn reap_orphaned_tmux_servers() {
            #[cfg(unix)]
            {
                let Some(lane) = lane_root() else {
                    return;
                };
                let registry = lane.join(FIXTURE_REGISTRY);
                let probe = lane.join(format!(".ae-parity-reaper-{}", std::process::id()));
                let _ = fs::remove_dir_all(&probe);
                if fs::create_dir_all(&probe).is_err() {
                    return;
                }
                let Ok(owners) = fs::read_dir(&registry) else {
                    let _ = fs::remove_dir_all(&probe);
                    return;
                };
                for owner_entry in owners.flatten() {
                    let Ok(kind) = owner_entry.file_type() else {
                        continue;
                    };
                    if !kind.is_dir() {
                        continue;
                    }
                    let Some(owner) = owner_entry
                        .file_name()
                        .to_str()
                        .and_then(|name| name.parse::<u32>().ok())
                    else {
                        continue;
                    };
                    // Any ambiguous liveness result is ALIVE: leave it alone.
                    if !owner_is_dead(owner, &probe) {
                        continue;
                    }
                    let mut complete = true;
                    let Ok(entries) = fs::read_dir(owner_entry.path()) else {
                        continue;
                    };
                    for entry in entries {
                        let Ok(entry) = entry else {
                            complete = false;
                            break;
                        };
                        let Ok(kind) = entry.file_type() else {
                            complete = false;
                            continue;
                        };
                        if !kind.is_symlink() {
                            complete = false;
                            continue;
                        }
                        let Ok(scratch) = fs::read_link(entry.path()) else {
                            complete = false;
                            continue;
                        };
                        if !reap_fixture_servers(&scratch, &probe) {
                            complete = false;
                        }
                    }
                    if complete {
                        let _ = fs::remove_dir_all(owner_entry.path());
                    }
                }
                let _ = fs::remove_dir_all(probe);
            }
        }

        #[cfg(unix)]
        #[allow(
            clippy::disallowed_methods,
            reason = "the parity reaper reads only its test-lane TMUX_TMPDIR registry"
        )]
        fn lane_root() -> Option<PathBuf> {
            std::env::var_os("TMUX_TMPDIR")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
        }

        #[cfg(unix)]
        fn remember_fixture(cwd: &Path) -> io::Result<()> {
            let lane = lane_root().ok_or_else(|| {
                io::Error::other("TMUX_TMPDIR is required to register a tmux test fixture")
            })?;
            if !cwd.is_absolute() {
                return Err(io::Error::other("tmux test fixture root must be absolute"));
            }
            let owner = lane
                .join(FIXTURE_REGISTRY)
                .join(std::process::id().to_string());
            fs::create_dir_all(&owner)?;
            for index in 0..usize::MAX {
                let entry = owner.join(index.to_string());
                if let Err(error) = std::os::unix::fs::symlink(cwd, &entry) {
                    if error.kind() != io::ErrorKind::AlreadyExists {
                        return Err(error);
                    }
                } else {
                    return Ok(());
                }
            }
            Err(io::Error::other("tmux test fixture registry is full"))
        }

        /// Record a root before a black-box product child may start tmux there.
        #[cfg(unix)]
        pub(crate) fn register_fixture_root(cwd: &Path) -> io::Result<()> {
            remember_fixture(cwd)
        }

        #[cfg(unix)]
        #[allow(
            clippy::disallowed_methods,
            reason = "the parity reaper reads its own probe stderr to distinguish ESRCH"
        )]
        fn owner_is_dead(owner: u32, probe: &Path) -> bool {
            let out = probe.join("owner-out");
            let err = probe.join("owner-err");
            run_unreaped(
                &Invocation::new("kill")
                    .env("LC_ALL", "C")
                    .arg("-0")
                    .arg(owner.to_string()),
                probe,
                &out,
                &err,
            )
            .is_ok_and(|status| matches!(status.outcome(), ExitOutcome::Code(1)))
                && fs::read_to_string(err).is_ok_and(|text| text.contains("No such process"))
        }

        #[cfg(unix)]
        fn reap_fixture_servers(scratch: &Path, probe: &Path) -> bool {
            let mut sockets = Vec::new();
            if !find_sockets(scratch, &mut sockets) {
                return false;
            }
            for (index, socket) in sockets.iter().enumerate() {
                let out = probe.join(format!("tmux-{index}-out"));
                let err = probe.join(format!("tmux-{index}-err"));
                if !run_unreaped(
                    &Invocation::new("tmux")
                        .arg("-S")
                        .arg(socket)
                        .arg("kill-server"),
                    scratch,
                    &out,
                    &err,
                )
                .is_ok_and(|status| matches!(status.outcome(), ExitOutcome::Code(0)))
                {
                    return false;
                }
            }
            if !sockets.is_empty() {
                return fs::remove_dir_all(scratch).is_ok();
            }
            true
        }

        #[cfg(unix)]
        #[allow(
            clippy::disallowed_methods,
            reason = "the parity reaper walks only a dead test fixture root"
        )]
        fn find_sockets(dir: &Path, sockets: &mut Vec<PathBuf>) -> bool {
            let Ok(entries) = fs::read_dir(dir) else {
                return false;
            };
            for entry in entries {
                let Ok(entry) = entry else {
                    return false;
                };
                let Ok(kind) = entry.file_type() else {
                    return false;
                };
                if kind.is_socket() {
                    sockets.push(entry.path());
                } else if kind.is_dir() && !find_sockets(&entry.path(), sockets) {
                    return false;
                }
            }
            true
        }

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
        fn run_unreaped(
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
            if invocation.program == "tmux" {
                // Every real-tmux fixture is isolated structurally at the one
                // process door. An inherited client marker can redirect even
                // a named/default command to the developer's live server.
                command
                    .env_remove("TMUX")
                    .env_remove("TMUX_PANE")
                    .env("TMUX_TMPDIR", cwd)
                    .env("SHELL", "/bin/sh");
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
