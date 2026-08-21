//! Parity-harness plumbing — issue #93, **stage 1 only**.
//!
//! # What this is
//!
//! Machinery to run the same corpus through two command lanes and keep both
//! sets of raw artifacts side by side. It exists so that a later, seat-gated
//! stage can ask "do these two agree?" against evidence that was captured
//! without an opinion.
//!
//! # It CAPTURES. It never JUDGES.
//!
//! There is deliberately **no** comparison, diff, verdict, expectation or
//! tolerance anywhere in this module, and adding one is a scope violation
//! rather than a convenience. The reason is not stylistic: a harness that knows
//! what the answer should be is a harness whose captures have already been
//! filtered through that belief, and the parity run would then measure the
//! belief. What it produces is bytes, an exit outcome, and a listing of what
//! the filesystem looked like afterwards. Whoever reads them decides.
//!
//! # How much of that is ENFORCED, and how much is watched
//!
//! Read this before trusting the sentence above. Five review rounds went into
//! it, each one closing a channel and finding the next, and what follows is the
//! settled position rather than an aspiration.
//!
//! **Closed by the compiler — these cannot be written:**
//!
//! * A lane's OUTPUT BYTES never enter this process at all. The child's stdout
//!   and stderr are wired straight to their artifact files before it starts, so
//!   there is no value here to compare, filter or normalise. This is the only
//!   kind of closure that is absolute, and the reason is general: possession is
//!   capability. Code that must serialise a value must possess it, and anything
//!   that possesses a value can compare it — Rust has no write-only capability.
//!   So the only way to close a channel completely is for the value never to
//!   arrive, which works here because the OS does the writing for us.
//! * [`capture::LaneCapture`] and [`capture::PairedCapture`] have private
//!   fields, no accessors, and no `Debug`/`PartialEq`/`Hash`/`Ord`: whole-value
//!   reads and whole-value verdicts have no impl to call. [`run_pair`] holds
//!   BOTH lanes and cannot look at either.
//! * `#[expect(dead_code)]` on every evidence field turns a read from INSIDE
//!   the module into a build failure, per field.
//! * `std::process::Command` is a denied type crate-wide (`clippy.toml`). Two
//!   doors relax it, and they do different jobs: THIS harness's is the one in
//!   [`capture::raw`]; the other is in `tests/it/cli.rs`, whose black-box tests
//!   must run the product binary and whose factory is private to that module,
//!   so nothing here can reach a child through it. That is a capability
//!   boundary rather than a naming convention ONLY because this crate has no
//!   dependencies and forbids `unsafe_code`; both premises are written down at
//!   the pin site, because both can stop being true.
//!
//! **Watched, not closed — the signed residuals:**
//!
//! 1. **The filesystem.** Artifacts are files, and any code holding a path can
//!    read them back. Nothing structural prevents a re-read; the source scanner
//!    sees the direct forms.
//! 2. **The exit status**, inside [`capture::raw::run`]. Something must receive
//!    an `ExitStatus` to turn it into the `exit` artifact. [`capture::raw`] is
//!    that something and is deliberately the whole of it.
//! 3. **The manifest**, inside [`capture::capture_lane`]. One lane's listing is
//!    held as a value there. A cross-lane comparison still needs residual 1;
//!    direct judgements on it are caught by the scanner; laundered ones are not.
//! 4. **`RUSTFLAGS` or `--cap-lints` from outside the tree**, and anything that
//!    changes what the guard itself runs. Lint relaxations INSIDE the tree are
//!    no longer residual — see below — but a flag set in the environment that
//!    invokes cargo is not something the tree can see.
//!
//! # Two different strengths, and they are not interchangeable
//!
//! This is the durable lesson of the slice, and it is written here because
//! every round of its review found the same seam: a guard that resolves
//! MEANING and a guard that matches TEXT are not the same kind of thing, and
//! blurring them is how every bypass in this review got written.
//!
//! * The **capability boundary is SEMANTIC** in both halves now. The type deny
//!   resolves paths, so `std::process::Command::output(&mut c)`, `use ... as C`
//!   and spellings nobody has thought of are the same type to it. And the guard
//!   that checks the deny is still in force runs clippy under
//!   `--force-warn clippy::disallowed_types`, which NO relaxation can override:
//!   not an `allow`, not a GROUP `allow`, not a `cfg_attr` around either, not
//!   an `expect`, not a crate-root `#![allow]`. It asserts the set of files
//!   that can start a child process, and it asks the compiler for that set.
//! * A **textual counter of relaxation forms survives as defence in depth
//!   only**, and is PROVEN incomplete: `#[allow(clippy::style)]` relaxes
//!   `disallowed_types` by naming a group rather than the lint, and a review
//!   walked a third `Command` site past it with everything green. Nothing here
//!   rests on it.
//!
//! The lesson generalises past this file, and it was learned the expensive way:
//! an enumeration was beaten FOUR times in this slice — a field-name list, a
//! method-name list, an outer-attribute prefix, and a lint-group name. Each fix
//! was correct and each closed less than it appeared to. The way out was not a
//! fifth enumeration; it was finding a mechanism that resolves meaning, and
//! then writing down what even that does not cover.
//!
//! Residuals 1-3 are accepted for this slice: they exist because this process
//! must compute those values, and no type-system boundary can hold a value the
//! process is required to produce.
//!
//! Structurally enforced: this file contains no `#[test]`. Every assertion
//! about the harness lives in [`super::parity_self_test`], which is the *test*
//! judging the *harness* — never the harness judging a lane. The source scanner
//! there is DEFENCE IN DEPTH over the list above, not the claim itself: it
//! catches shapes the compiler is not asked about, and it is a heuristic.
//!
//! # Stage boundaries
//!
//! * **Stage 1 (here)** — the plumbing, exercised by SYNTHETIC fixtures that
//!   the self-test module authors. No real corpus is imported, no bash-produced
//!   output is read, nothing under `docs/migration/evidence/` is touched.
//! * **Stage 2** — importing the real corpus. Seat-gated. Not here.
//! * **Stage 3** — the first bash-vs-rust run. Seat-gated. Not here.
//!
//! # One clone per lane, never a shared one
//!
//! Each lane gets its OWN copy of the template. Two lanes sharing a directory
//! would mean the second one starts from wherever the first one left it, so any
//! difference measured afterwards could as easily be ordering as behavior. The
//! template itself is only ever read.
//!
//! # Stated limitations
//!
//! * Paths and command words are recorded into the on-disk artifacts through
//!   `to_string_lossy`, so a corpus with non-UTF-8 names is out of scope for
//!   stage 1 rather than silently mangled at one layer only.
//! * A file's content digest is FNV-1a/64 — a fast, NON-cryptographic
//!   fingerprint. Two different files can collide. It is a capture aid for
//!   spotting "same length, different bytes", not evidence of sameness; the
//!   clone directories are kept, so the bytes themselves remain available.
//! * `clone_to` preserves file permission bits (via `fs::copy`) but creates
//!   directories with the process umask. Directory modes are captured in the
//!   manifest, so a run can still SEE them; they are just not reproduced from
//!   the template.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]
#![cfg(unix)]

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use ae::json::Value;

use capture::PairedCapture;

/// A corpus template: a directory tree that is cloned per lane and never
/// written to.
#[derive(Debug)]
pub(crate) struct Corpus {
    template: PathBuf,
}

impl Corpus {
    /// Import the template rooted at `template`.
    ///
    /// # Errors
    ///
    /// Fails if the path does not exist or is not a directory. Both are refused
    /// loudly: a corpus that silently resolves to nothing produces two empty
    /// lanes that agree perfectly, which is the most convincing wrong answer
    /// this harness could give.
    pub(crate) fn import(template: &Path) -> io::Result<Self> {
        let meta = fs::metadata(template)?;
        if !meta.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("corpus template is not a directory: {}", template.display()),
            ));
        }
        Ok(Self {
            template: template.to_path_buf(),
        })
    }

    /// The template directory this corpus was imported from.
    pub(crate) fn template(&self) -> &Path {
        &self.template
    }

    /// Copy the template to `dest`, creating it.
    ///
    /// Files, directories and symlinks are reproduced; symlinks are copied as
    /// links rather than followed, so a link that points outside the corpus
    /// stays a dangling link instead of quietly importing whatever it aimed at.
    ///
    /// # Errors
    ///
    /// Any underlying filesystem failure.
    pub(crate) fn clone_to(&self, dest: &Path) -> io::Result<()> {
        copy_tree(&self.template, dest)
    }
}

fn copy_tree(src: &Path, dest: &Path) -> io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        let meta = fs::symlink_metadata(&from)?;
        if meta.file_type().is_symlink() {
            std::os::unix::fs::symlink(fs::read_link(&from)?, &to)?;
        } else if meta.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

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

    fn to_json(&self, cwd: &Path) -> Value {
        Value::obj([
            ("program", Value::str(self.program.to_string_lossy())),
            (
                "args",
                Value::Arr(
                    self.args
                        .iter()
                        .map(|arg| Value::str(arg.to_string_lossy()))
                        .collect(),
                ),
            ),
            ("cwd", Value::str(cwd.to_string_lossy())),
            ("env_cleared", Value::Bool(self.env_cleared)),
            (
                "env",
                Value::Obj(
                    self.env
                        .iter()
                        .map(|(key, value)| {
                            (
                                key.to_string_lossy().into_owned(),
                                Value::str(value.to_string_lossy()),
                            )
                        })
                        .collect(),
                ),
            ),
        ])
    }
}

/// One side of a paired run: a name, and how to build its command once the
/// clone directory exists.
///
/// The builder takes the clone path because a lane usually needs it — as a
/// `--flag`, as `AE_HOME`, as an argument. Handing it in beats inventing a
/// placeholder syntax the caller would have to learn.
pub(crate) struct Lane {
    name: String,
    build: Box<dyn Fn(&Path) -> Invocation>,
}

impl Lane {
    /// A lane called `name` whose command is built by `build`.
    pub(crate) fn new<N, F>(name: N, build: F) -> Self
    where
        N: Into<String>,
        F: Fn(&Path) -> Invocation + 'static,
    {
        Self {
            name: name.into(),
            build: Box::new(build),
        }
    }

    /// This lane's name.
    pub(crate) fn name(&self) -> &str {
        &self.name
    }
}

pub(crate) mod capture {
    //! Where evidence enters this harness, and the only place it can be read.
    //!
    //! # The boundary is the type system, not a convention
    //!
    //! A lane's output is produced HERE, written to disk HERE, and stored in a
    //! [`LaneCapture`] whose fields are **private with no accessors**. Nothing
    //! outside this module can read them — not [`super::run_pair`], which holds
    //! both captures and provably cannot compare them, and not the self-tests,
    //! which read the artifacts off disk exactly as a later stage would.
    //!
    //! Three mechanisms, because each closes a channel the others cannot see:
    //!
    //! * **Privacy** stops any read from outside this module. Enforced by the
    //!   compiler on every build.
    //! * **`#[expect(dead_code)]` on every evidence field** stops reads from
    //!   INSIDE it too: the moment anything reads one, the expectation is
    //!   unfulfilled and `-D warnings` turns that into a build failure. The
    //!   attribute is not bookkeeping — it is the tripwire, and it is per-field
    //!   because a struct-level one stays fulfilled while any other field is
    //!   still unread.
    //! * **No observation-bearing derives.** `Debug` reads every field at once
    //!   and `PartialEq` compares every field at once, and a *derived* impl does
    //!   NOT unfulfil the expectations above (measured — it is special-cased by
    //!   the dead-code lint), so this one is asserted separately, by the probe in
    //!   the self-tests.
    //!
    //! Writing artifacts is capture, not judgement, which is why persistence
    //! lives in here rather than in the orchestration above.

    use std::fs;
    use std::io;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use ae::json::Value;

    use super::{Corpus, Lane};
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
        // name `std::process::Command`. There is one other door crate-wide, in
        // `tests/it/cli.rs`, whose black-box tests must run the product binary;
        // it is a different job and is private to that module.
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

    /// Whether a lane's process exited with a code, or was killed.
    ///
    /// `Signalled` rather than a number: the signal itself is not portable to
    /// record here, and a capture that guesses is worse than one that says it does
    /// not know.
    /// No `Debug`, `PartialEq` or `Eq`: an outcome is written to the `exit`
    /// artifact and stored, never compared against an expectation here. The
    /// self-tests read the artifact, which is what a consumer gets.
    #[derive(Clone, Copy)]
    pub(crate) enum ExitOutcome {
        /// The process exited with this status code.
        Code(i32),
        /// The process was terminated by a signal.
        Signalled,
    }

    impl ExitOutcome {
        fn as_artifact(self) -> String {
            match self {
                Self::Code(code) => format!("code {code}\n"),
                Self::Signalled => "signalled\n".to_owned(),
            }
        }
    }

    /// What kind of thing a manifest entry describes.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum EntryKind {
        /// A regular file.
        File,
        /// A directory.
        Dir,
        /// A symbolic link, recorded as a link rather than followed.
        Symlink,
    }

    impl EntryKind {
        fn as_str(self) -> &'static str {
            match self {
                Self::File => "file",
                Self::Dir => "dir",
                Self::Symlink => "symlink",
            }
        }
    }

    /// One line of a lane's recursive file manifest.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) struct ManifestEntry {
        /// Path relative to the clone root, `/`-separated.
        pub(crate) path: String,
        /// File, directory or symlink.
        pub(crate) kind: EntryKind,
        /// Byte length, for regular files.
        pub(crate) len: Option<u64>,
        /// Permission bits, for files and directories.
        pub(crate) mode: Option<u32>,
        /// Where a symlink points, verbatim and unresolved.
        pub(crate) target: Option<String>,
        /// FNV-1a/64 of a regular file's bytes, hex. See the module's limitations.
        pub(crate) digest: Option<String>,
    }

    impl ManifestEntry {
        fn to_json(&self) -> Value {
            let mut fields = vec![
                ("path".to_owned(), Value::str(&self.path)),
                ("kind".to_owned(), Value::str(self.kind.as_str())),
            ];
            if let Some(len) = self.len {
                fields.push((
                    "len".to_owned(),
                    Value::Num(i64::try_from(len).unwrap_or(i64::MAX)),
                ));
            }
            if let Some(mode) = self.mode {
                fields.push(("mode".to_owned(), Value::str(format!("{mode:o}"))));
            }
            if let Some(target) = &self.target {
                fields.push(("target".to_owned(), Value::str(target)));
            }
            if let Some(digest) = &self.digest {
                fields.push(("digest".to_owned(), Value::str(digest)));
            }
            Value::Obj(fields)
        }
    }

    /// A lane's recursive listing of its clone directory, taken AFTER the run.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) struct Manifest {
        /// Every entry below the root, sorted by path.
        pub(crate) entries: Vec<ManifestEntry>,
    }

    impl Manifest {
        /// Walk `root` recursively.
        ///
        /// Sorted by path so that two captures are readable side by side without
        /// anyone first having to normalise directory-iteration order — which is
        /// arbitrary, and would otherwise show up as a difference that is not one.
        ///
        /// # Errors
        ///
        /// Any underlying filesystem failure.
        pub(crate) fn of(root: &Path) -> io::Result<Self> {
            let mut entries = Vec::new();
            walk(root, root, &mut entries)?;
            entries.sort_by(|left, right| left.path.cmp(&right.path));
            Ok(Self { entries })
        }

        /// Every path, in manifest order.
        pub(crate) fn paths(&self) -> Vec<&str> {
            self.entries
                .iter()
                .map(|entry| entry.path.as_str())
                .collect()
        }

        fn to_json(&self) -> Value {
            Value::Arr(self.entries.iter().map(ManifestEntry::to_json).collect())
        }
    }

    fn walk(root: &Path, dir: &Path, out: &mut Vec<ManifestEntry>) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let meta = fs::symlink_metadata(&path)?;
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();

            if meta.file_type().is_symlink() {
                out.push(ManifestEntry {
                    path: relative,
                    kind: EntryKind::Symlink,
                    len: None,
                    mode: None,
                    target: Some(fs::read_link(&path)?.to_string_lossy().into_owned()),
                    digest: None,
                });
            } else if meta.is_dir() {
                out.push(ManifestEntry {
                    path: relative,
                    kind: EntryKind::Dir,
                    len: None,
                    mode: Some(meta.permissions().mode()),
                    target: None,
                    digest: None,
                });
                walk(root, &path, out)?;
            } else {
                let bytes = fs::read(&path)?;
                out.push(ManifestEntry {
                    path: relative,
                    kind: EntryKind::File,
                    len: Some(meta.len()),
                    mode: Some(meta.permissions().mode()),
                    target: None,
                    digest: Some(format!("{:016x}", fnv1a64(&bytes))),
                });
            }
        }
        Ok(())
    }

    /// FNV-1a/64. Non-cryptographic — see the module's limitations.
    fn fnv1a64(bytes: &[u8]) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    /// Everything one lane produced. Raw, and only raw.
    ///
    /// Every field is private, and there is deliberately NO accessor: what a
    /// lane produced is on disk under its artifact directory, and that is where
    /// a consumer reads it. The `#[expect(dead_code)]` on each field is the
    /// enforcement, not an apology for it — see the module docs.
    ///
    /// No `Debug`, no `PartialEq`, no `Hash`, no `Ord`. Each of those is a
    /// whole-value read or a whole-value comparison that needs no field name at
    /// all, which is exactly how the last bypass was written.
    pub(crate) struct LaneCapture {
        /// The lane's name.
        #[expect(
            dead_code,
            reason = "evidence is held for a later, seat-gated stage; stage 1 must never read it"
        )]
        lane: String,
        /// How the process ended.
        #[expect(
            dead_code,
            reason = "evidence is held for a later, seat-gated stage; stage 1 must never read it"
        )]
        exit: ExitOutcome,
        /// Where the child wrote its stdout, verbatim. The path, not the bytes:
        /// this process never held them.
        #[expect(
            dead_code,
            reason = "evidence is held for a later, seat-gated stage; stage 1 must never read it"
        )]
        stdout: PathBuf,
        /// Where the child wrote its stderr, verbatim.
        #[expect(
            dead_code,
            reason = "evidence is held for a later, seat-gated stage; stage 1 must never read it"
        )]
        stderr: PathBuf,
        /// The clone tree as it stood after the run.
        #[expect(
            dead_code,
            reason = "evidence is held for a later, seat-gated stage; stage 1 must never read it"
        )]
        manifest: Manifest,
        /// The directory the command ran in — kept, not deleted.
        #[expect(
            dead_code,
            reason = "evidence is held for a later, seat-gated stage; stage 1 must never read it"
        )]
        clone_dir: PathBuf,
        /// Where this lane's artifacts were written.
        #[expect(
            dead_code,
            reason = "evidence is held for a later, seat-gated stage; stage 1 must never read it"
        )]
        artifact_dir: PathBuf,
    }

    /// Two lanes' captures, stored side by side under one root.
    ///
    /// [`super::run_pair`] builds one of these while holding both lanes, and
    /// cannot read either. That is the point of the type: the one place in this
    /// harness where both sides of a parity run are in scope at once is also a
    /// place where neither can be looked at.
    pub(crate) struct PairedCapture {
        /// The root both lanes were written under.
        #[expect(
            dead_code,
            reason = "evidence is held for a later, seat-gated stage; stage 1 must never read it"
        )]
        root: PathBuf,
        /// The two lanes, in the order they were given.
        #[expect(
            dead_code,
            reason = "evidence is held for a later, seat-gated stage; stage 1 must never read it"
        )]
        lanes: [LaneCapture; 2],
    }

    impl PairedCapture {
        /// Bundle two captures. A constructor, not an accessor: it takes
        /// evidence in and never lets any back out.
        pub(crate) fn of(root: &Path, lanes: [LaneCapture; 2]) -> Self {
            Self {
                root: root.to_path_buf(),
                lanes,
            }
        }
    }

    pub(crate) fn capture_lane(
        corpus: &Corpus,
        root: &Path,
        lane: &Lane,
    ) -> io::Result<LaneCapture> {
        let artifact_dir = root.join(lane.name());
        let clone_dir = artifact_dir.join("clone");
        fs::create_dir_all(&artifact_dir)?;
        corpus.clone_to(&clone_dir)?;

        let invocation = (lane.build)(&clone_dir);
        fs::write(
            artifact_dir.join("command.json"),
            invocation.to_json(&clone_dir).render(),
        )?;

        // The streams are wired to their artifact files BEFORE the child starts,
        // so what it prints is never a value in this process. There is nothing
        // here to compare, filter or normalise, which is the point.
        let stdout = artifact_dir.join("stdout");
        let stderr = artifact_dir.join("stderr");
        let status = raw::run(&invocation, &clone_dir, &stdout, &stderr)?;

        // AFTER the run: the manifest's job is to say what the lane left behind.
        let manifest = Manifest::of(&clone_dir)?;
        fs::write(artifact_dir.join("exit"), status.outcome().as_artifact())?;
        fs::write(
            artifact_dir.join("manifest.json"),
            manifest.to_json().render(),
        )?;

        Ok(LaneCapture {
            lane: lane.name().to_owned(),
            exit: status.outcome(),
            stdout,
            stderr,
            manifest,
            clone_dir,
            artifact_dir,
        })
    }
}

/// Clone `corpus` twice, run both lanes, and store the paired artifacts under
/// `root`.
///
/// Layout, which is the whole point of "side by side":
///
/// ```text
/// <root>/pair.json          the template, and the lane names in order
/// <root>/<lane>/clone/      the tree that lane ran against, as it was left
/// <root>/<lane>/command.json  program, args, cwd, environment
/// <root>/<lane>/exit        `code <n>` or `signalled`
/// <root>/<lane>/stdout      raw bytes
/// <root>/<lane>/stderr      raw bytes
/// <root>/<lane>/manifest.json  the recursive listing
/// ```
///
/// Both lanes are ATTEMPTED even if the first one fails, because "one lane
/// refused to start" is itself a finding, and a harness that aborts on it
/// destroys the evidence for the other side. A lane that ran wrote its
/// artifacts under `<root>/<lane>/` before this returns, so they outlive the
/// other lane's failure; the error reported is the first lane's, in order.
///
/// # Errors
///
/// Fails if a lane name is unusable, if the two names collide (they would share
/// an artifact directory and overwrite each other), or on any filesystem or
/// spawn failure.
pub(crate) fn run_pair(
    corpus: &Corpus,
    root: &Path,
    lanes: [Lane; 2],
) -> io::Result<PairedCapture> {
    for lane in &lanes {
        check_lane_name(lane.name())?;
    }
    if lanes[0].name() == lanes[1].name() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "both lanes are called {:?}; they would share one artifact directory",
                lanes[0].name()
            ),
        ));
    }

    fs::create_dir_all(root)?;
    fs::write(
        root.join("pair.json"),
        Value::obj([
            ("template", Value::str(corpus.template().to_string_lossy())),
            (
                "lanes",
                Value::Arr(lanes.iter().map(|lane| Value::str(lane.name())).collect()),
            ),
        ])
        .render(),
    )?;

    let [first, second] = lanes;
    // Both attempts happen BEFORE either result is unwrapped. A `?` on the
    // first line would mean a first lane that could not even be spawned takes
    // the second lane's evidence with it — the one case the paragraph above
    // promises to survive, and the one an `exit 127` lane does not exercise,
    // because that process started.
    let first = capture::capture_lane(corpus, root, &first);
    let second = capture::capture_lane(corpus, root, &second);
    let (first, second) = (first?, second?);
    Ok(PairedCapture::of(root, [first, second]))
}

/// A lane name has to be a single, ordinary directory component: it becomes one.
fn check_lane_name(name: &str) -> io::Result<()> {
    let legal = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && name.starts_with(|c: char| c.is_ascii_alphanumeric());
    if legal {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("lane name {name:?} is not an ordinary directory component"),
        ))
    }
}

/// A temporary directory that removes itself.
///
///
/// Hand-rolled rather than a dev-dependency, for the same reason the crate has
/// no runtime ones: it is a dozen lines, and a test harness that drags in a
/// dependency tree is a supply-chain surface for a lane that runs commands.
#[derive(Debug)]
pub(crate) struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    /// A fresh directory under the system temp dir, tagged `tag`.
    ///
    /// # Errors
    ///
    /// Fails if the directory cannot be created — including when the unique
    /// name is somehow already taken, which is refused rather than reused: a
    /// scratch dir with someone else's files in it is not scratch.
    pub(crate) fn new(tag: &str) -> io::Result<Self> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("ae-parity-{tag}-{}-{unique}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    /// The directory's path.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
