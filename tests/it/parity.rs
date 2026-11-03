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

        use std::ffi::OsStr;
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
                        if !reap_fixture_root(owner, &scratch, &probe) {
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

        /// Record a scratch root when a lane is present; outside one there is
        /// no registry to write and nothing to reap it.
        #[cfg(unix)]
        pub(crate) fn register_owned_root(root: &Path) -> io::Result<()> {
            lane_root().map_or(Ok(()), |_| remember_fixture(root))
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

        /// Kill every server under a dead owner's registered root, then delete
        /// the root only when it is the suite's own (`scratch::owned_root`).
        #[cfg(unix)]
        #[allow(
            clippy::disallowed_methods,
            reason = "the parity reaper asks whether a dead owner already removed its root"
        )]
        fn reap_fixture_root(owner: u32, scratch: &Path, probe: &Path) -> bool {
            let gone = fs::symlink_metadata(scratch)
                .is_err_and(|why| why.kind() == io::ErrorKind::NotFound);
            // The owner's whole `ae-it-<owner>` when the grammar allows it, so a
            // sibling root's server dies before its socket is unlinked.
            let ours = crate::scratch::owned_root(owner, scratch);
            let Some(walk) = ours.as_deref().or((!gone).then_some(scratch)) else {
                return true;
            };
            let mut sockets = Vec::new();
            if !find_sockets(walk, &mut sockets) || !kill_servers(&sockets, walk, probe) {
                return false;
            }
            ours.is_none_or(|root| fs::remove_dir_all(root).is_ok())
        }

        /// Kill every server under `root` and each `listed` socket, for an
        /// owner about to remove `root`; whether all are gone. Needs no lane:
        /// the owner's own Drop.
        #[cfg(unix)]
        pub(crate) fn kill_servers_under(root: &Path, listed: &[PathBuf]) -> bool {
            let mut sockets = listed.to_vec();
            let found = find_sockets(root, &mut sockets);
            sockets.sort();
            sockets.dedup();
            kill_servers(&sockets, root, root) && found
        }

        /// Whether every socket's server is gone. One whose server already
        /// died (`no server running`) or whose socket is gone is not a failure.
        #[cfg(unix)]
        #[allow(
            clippy::disallowed_methods,
            reason = "the reaper reads its own kill stderr to tell a gone server from a live one"
        )]
        fn kill_servers(sockets: &[PathBuf], cwd: &Path, probe: &Path) -> bool {
            let mut all = true;
            for (index, socket) in sockets.iter().enumerate() {
                let out = probe.join(format!("tmux-{index}-out"));
                let err = probe.join(format!("tmux-{index}-err"));
                let kill = Invocation::new("tmux")
                    .arg("-S")
                    .arg(socket)
                    .arg("kill-server");
                all &= run_unreaped(&kill, cwd, &out, &err)
                    .is_ok_and(|status| matches!(status.outcome(), ExitOutcome::Code(0)))
                    || fs::read_to_string(&err).is_ok_and(|text| {
                        text.contains("no server running")
                            || text.contains("(No such file or directory)")
                    });
            }
            all
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
                // A pane inherits its SERVER's environment, and the first
                // command sent decides it, so a fixture whose caller named no
                // `HOME` would hand every pane the operator's, where their
                // tools keep their stores. The fixture's root stands in; a
                // caller that names its own (a tool home inside its scratch)
                // keeps it.
                if !invocation.env.contains_key(OsStr::new("HOME")) {
                    command.env("HOME", cwd);
                }
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

/// The pin of the tmux door's environment. A pane inherits its SERVER's
/// environment, not its client's, so the first tmux command a fixture sends
/// decides what every pane under that server can read: the stores of the
/// operator's own tools live under `$HOME`. A server started through this door
/// therefore sees the fixture's `HOME` and never the operator's, whether the
/// caller inherits its environment, clears it down to `PATH`, or names a `HOME`
/// of its own (which the door keeps: a fixture may keep a tool home inside its
/// scratch).
#[test]
#[allow(
    clippy::disallowed_methods,
    reason = "the pin reads its own PATH to hand a cleared tmux its way to the binary, and the HOME file its own pane wrote"
)]
fn a_pane_started_through_the_tmux_door_reports_a_scratch_home() {
    use std::path::Path;
    use std::time::{Duration, Instant};

    fn pane_home(root: &Path, name: &str, start: Invocation) -> String {
        let reported = root.join(format!("{name}.home"));
        let staged = root.join(format!("{name}.staged"));
        // Written whole or not at all, so a read never sees half a path.
        let script = format!(
            "printf %s \"$HOME\" > '{staged}' && mv '{staged}' '{reported}'; sleep 60",
            staged = staged.display(),
            reported = reported.display(),
        );
        let invocation = start
            .arg("-f")
            .arg("/dev/null")
            .arg("-S")
            .arg(root.join(format!("{name}.sock")))
            .arg("new-session")
            .arg("-d")
            .arg("-s")
            .arg(name)
            .arg(script);
        let status = capture::raw::run(
            &invocation,
            root,
            &root.join(format!("{name}.out")),
            &root.join(format!("{name}.err")),
        )
        .unwrap_or_else(|why| panic!("tmux must be runnable for the door pin: {why}"));
        assert!(
            matches!(status.outcome(), capture::ExitOutcome::Code(0)),
            "the {name} server must start"
        );
        let deadline = Instant::now() + super::support::PANE_WAIT;
        loop {
            if let Ok(home) = std::fs::read_to_string(&reported) {
                return home;
            }
            assert!(
                Instant::now() < deadline,
                "the {name} pane never reported its HOME"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    // The scratch owner reaps every server beneath its root when it drops.
    let scratch = super::cli::OwnedScratch::root("door", "home");
    let root = scratch.path().to_owned();
    let path = std::env::var_os("PATH").unwrap_or_else(|| panic!("test PATH"));
    let own = root.join("kept-home");
    assert!(
        std::fs::create_dir_all(&own).is_ok(),
        "the caller's own home"
    );
    let arms = [
        ("a", "inherited", Invocation::new("tmux"), root.clone()),
        (
            "c",
            "cleared to PATH",
            Invocation::new("tmux").env_cleared().env("PATH", path),
            root.clone(),
        ),
        (
            "e",
            "an explicit HOME",
            Invocation::new("tmux").env("HOME", &own),
            own.clone(),
        ),
    ];
    let wrong: Vec<String> = arms
        .into_iter()
        .filter_map(|(name, how, start, expected)| {
            let seen = pane_home(&root, name, start);
            (Path::new(&seen) != expected).then(|| {
                format!(
                    "{how}: the pane reported HOME={seen:?}, not {}",
                    expected.display()
                )
            })
        })
        .collect();
    assert!(
        wrong.is_empty(),
        "a tmux server this door starts must not see the operator's HOME: {wrong:#?}"
    );
}
