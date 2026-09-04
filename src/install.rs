//! `_install --from <dir>` — publish a verified bundle as the current `ae`.
//!
//! Slice Z4 moves the installer's LOGIC here. What is left in bash is a
//! bootstrap that downloads a bundle, checks it against the release manifest,
//! extracts it, and runs the core it just unpacked. Everything after that — the
//! member proof, the version directory, its modes, the command link and the
//! journal that makes a half-done publish reversible — is this module, and
//! [`crate::upgrade`] reaches the same [`publish`] without a second spelling.
//!
//! # The published shape is the contract
//!
//! ```text
//! ~/.ae/versions/<V>/ae-core      0555
//! ~/.ae/versions/<V>/install      0555
//! ~/.ae/versions/<V>/SHA256SUMS   0444
//! ~/.ae/versions/<V>/             0555
//! ~/.local/bin/ae -> ~/.ae/versions/<V>/ae-core
//! ```
//!
//! Three consequences, each of them a decision rather than a detail:
//!
//! * **The command link IS the current pointer.** Switching versions is one
//!   atomic rename of it. `~/.ae/core/current` and `~/.ae/current` are retired,
//!   and a second pointer at a single target is a second answer waiting to
//!   disagree.
//! * **A 0555 directory refuses entry create and unlink**, so a stray
//!   `> $SESSION/send` through a helper symlink gets `EACCES` instead of
//!   truncating the binary every session on the machine is bound to.
//! * **The manifest's exact bytes are a two-party contract.** This module
//!   writes nothing into it — it COPIES the bundle's own — and
//!   [`crate::shape::parse_manifest`] reads it back on every invocation.
//!
//! # Fixed paths, derived from `$HOME` and nothing else
//!
//! There are no path overrides. `AE_HOME` moves ae's STATE for a checkout build;
//! it has never moved an INSTALL, and accepting one here would let a publish
//! land somewhere the shape classifier ([`crate::shape`]) will never look.
//!
//! # Verification is the whole gate, and it runs before any write
//!
//! [`verify`] proves the three members, parses the manifest, and re-digests both
//! executable members with SHA-256 before a byte is published. Only then is the
//! bundle's own core executed to ask its version — so nothing unverified runs on
//! this path, and the version directory's NAME is the version the core will
//! report when [`crate::lib`]'s install gate asks it again at run time.
//!
//! This is the ONE place in ae that hashes anything. The core deliberately does
//! not: a 2.4 MB digest on every helper call is a cost the product will not pay,
//! and structural validation plus this one-time proof is what the immutable
//! directory rests on.

use std::path::{Path, PathBuf};

/// The mode both executable members and the version directory are published at.
///
/// The directory carries the member mode too: 0555 on a directory is what makes
/// an entry create or unlink inside it fail with `EACCES`.
pub const MEMBER_MODE: u32 = 0o555;

/// The manifest's mode. Readable, never writable — it describes the members.
pub const MANIFEST_MODE: u32 = 0o444;

/// The journal's own name, inside the ae home it describes.
pub const JOURNAL: &str = ".ae-install.journal";

/// The journal's format word.
///
/// **3, not 2.** The bash journal carried `config`/`config_had` rows, a
/// leftover from an installer that once seeded `~/.ae/config` and had to be
/// able to put it back. It has not written a config since slice Z3, so those
/// two fields recorded a fact nothing consumed. They are gone, and a journal
/// naming them is refused by the unknown-field arm rather than half-replayed —
/// which is exactly the treatment any journal this parser does not know should
/// get.
pub const JOURNAL_FORMAT: &str = "3";

/// The usage line `_install` prints when its argv is not `--from <dir>`.
pub const USAGE: &str = "Usage: ae _install --from <extracted-bundle-dir>";

// ─── the doors ───────────────────────────────────────────────────────────

/// The filesystem reads this module makes, each named once.
///
/// Registered in `tests/it/phase3.rs`'s inventory: a module that reaches the
/// world is a line in a review, not a diff nobody read.
mod door {
    use std::path::Path;

    /// A member's bytes, for the digest that proves it.
    pub fn read(path: &Path) -> std::io::Result<Vec<u8>> {
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: a bundle member is hashed before it is published — see clippy.toml"
        )]
        let bytes = std::fs::read(path);
        bytes
    }

    /// `lstat`, never `stat`: a member that is a symlink to a mutable file
    /// outside the bundle passes every follow-test and is then published as
    /// though it were ours.
    pub fn lstat(path: &Path) -> std::io::Result<std::fs::Metadata> {
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: bundle members and publication destinations are classified WITHOUT following a link — see clippy.toml"
        )]
        let meta = std::fs::symlink_metadata(path);
        meta
    }

    /// The journal's text, when there is one to replay.
    pub fn read_text(path: &Path) -> std::io::Result<String> {
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: the install journal is the record a rollback replays — see clippy.toml"
        )]
        let text = std::fs::read_to_string(path);
        text
    }

    /// Whether `path` names anything at all, link included.
    pub fn present(path: &Path) -> bool {
        lstat(path).is_ok()
    }
}

// ─── fixed paths ─────────────────────────────────────────────────────────

/// The two paths a publish touches outside the version directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// `<HOME>/.ae`.
    pub home: PathBuf,
    /// `<HOME>/.local/bin/ae`, the command link that IS the current pointer.
    pub link: PathBuf,
}

impl Paths {
    /// `<home>/versions`.
    #[must_use]
    pub fn versions(&self) -> PathBuf {
        self.home.join(crate::shape::VERSIONS)
    }

    /// `<home>/.ae-install.journal`.
    #[must_use]
    pub fn journal(&self) -> PathBuf {
        self.home.join(JOURNAL)
    }
}

/// The fixed paths `$HOME` derives, or why it derives none.
///
/// # Errors
///
/// A `$HOME` that is not absolute, that carries `.`/`..`/`//`/a trailing slash,
/// that holds a newline (the journal is line-oriented), or that is itself a
/// symlink.
pub fn fixed_paths(home_var: &Path) -> Result<Paths, String> {
    let trimmed = trim_trailing_slash(home_var);
    let home = trimmed.join(".ae");
    let link = trimmed.join(".local").join("bin").join("ae");
    validate_home(&home)?;
    path_components_ok(&link, "ae command path")?;
    Ok(Paths { home, link })
}

/// `$HOME` with one trailing slash removed — `/` itself is left alone.
fn trim_trailing_slash(home: &Path) -> PathBuf {
    let text = home.to_string_lossy();
    if text.len() > 1 && text.ends_with('/') {
        PathBuf::from(text.trim_end_matches('/'))
    } else {
        home.to_path_buf()
    }
}

/// The path grammar every publication destination is held to.
///
/// A path that reaches the journal is replayed by a LATER process, so its
/// spelling is not cosmetic: `..` re-aims a reversal, `//` and a trailing `/`
/// make two spellings of one path compare unequal, and a newline ends the
/// record and turns its remainder into a phantom row.
fn path_components_ok(path: &Path, label: &str) -> Result<(), String> {
    let text = path.to_string_lossy();
    if text.contains('\n') || text.contains('\r') {
        return Err(format!("{label} must not contain a newline: {text}"));
    }
    if text.contains("//") {
        return Err(format!(
            "{label} must not contain repeated '/' separators: {text}"
        ));
    }
    if text.len() > 1 && text.ends_with('/') {
        return Err(format!("{label} must not have a trailing '/': {text}"));
    }
    if text
        .split('/')
        .any(|component| component == "." || component == "..")
    {
        return Err(format!(
            "{label} must not contain '.' or '..' path components: {text}"
        ));
    }
    Ok(())
}

/// The ae home: absolute, well-spelled, and not itself a symlink.
fn validate_home(home: &Path) -> Result<(), String> {
    path_components_ok(home, "ae home")?;
    if !home.is_absolute() {
        return Err(format!("ae home must be absolute: {}", home.display()));
    }
    if door::lstat(home).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(format!(
            "ae home must not itself be a symlink: {}",
            home.display()
        ));
    }
    Ok(())
}

/// The command destination: outside the home, reached through no symlinked
/// ancestor, and not a directory.
///
/// **Parent versus leaf, and the difference is the attack.** A dangling
/// `~/.local -> ~/.ae` makes `~/.local/bin` land physically INSIDE the home the
/// moment the home is created, so the ANCESTOR is what has to be resolved. The
/// LEAF being a symlink into the home is the canonical published state — it is
/// replaced atomically and never followed — so it is `lstat`ed and not resolved.
///
/// # Errors
///
/// A relative path, a bad spelling, a destination lexically or physically inside
/// the ae home, or a real directory sitting where the link belongs.
pub fn validate_bin_destination(link: &Path, home: &Path) -> Result<(), String> {
    path_components_ok(link, "ae command path")?;
    if !link.is_absolute() {
        return Err(format!(
            "ae command path must be absolute: {}",
            link.display()
        ));
    }
    if link == home || link.starts_with(home) {
        return Err(format!(
            "ae command path must not point inside the ae home: {}",
            link.display()
        ));
    }
    let parent = link
        .parent()
        .ok_or_else(|| format!("ae command path has no parent: {}", link.display()))?;
    let resolved_parent = resolve_nearest(parent);
    let resolved_home = resolve_nearest(home);
    if resolved_parent == resolved_home || resolved_parent.starts_with(&resolved_home) {
        return Err(format!(
            "ae command path resolves inside the ae home: {}",
            link.display()
        ));
    }
    // The leaf, lstat'ed and never followed: a real directory refuses, because
    // a rename onto one NESTS instead of replacing and reports success.
    if door::lstat(link).is_ok_and(|meta| meta.file_type().is_dir()) {
        return Err(format!(
            "ae command destination is a directory: {}",
            link.display()
        ));
    }
    Ok(())
}

/// How many dangling symlinks [`resolve_nearest`] will follow before it stops.
///
/// A dangling link may point at another dangling link, and two of them may
/// point at each other. The budget bounds the walk without needing to track the
/// links already seen.
const RESOLVE_LINK_BUDGET: u8 = 8;

/// `path` with its nearest EXISTING ancestor resolved and the missing tail
/// re-appended.
///
/// `canonicalize` alone cannot answer for a path that does not exist yet, and a
/// DANGLING symlink ancestor is exactly the case that matters: it names a
/// destination that is not there today and will be tomorrow.
fn resolve_nearest(path: &Path) -> PathBuf {
    resolve_nearest_within(path, RESOLVE_LINK_BUDGET)
}

fn resolve_nearest_within(path: &Path, budget: u8) -> PathBuf {
    let mut cursor = path.to_path_buf();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while !door::present(&cursor) {
        let Some(name) = cursor.file_name().map(std::ffi::OsStr::to_os_string) else {
            break;
        };
        let Some(parent) = cursor.parent().map(Path::to_path_buf) else {
            break;
        };
        tail.push(name);
        cursor = parent;
    }
    let resolved = resolve_existing(&cursor, budget);
    let mut out = resolved;
    for name in tail.iter().rev() {
        out.push(name);
    }
    out
}

/// One existing (or dangling-symlink) path, resolved as far as it can be.
///
/// **A dangling target is resolved through its own nearest existing ancestor,
/// not returned raw.** `canonicalize` fails on a path that is not there yet, and
/// returning the link's target verbatim made the two sides of the
/// [`validate_bin_destination`] comparison disagree about spelling whenever the
/// home sat under a symlinked ancestor: on macOS a fixture home in `/tmp`
/// resolved to `/private/tmp/…/.ae` while the dangling `~/.local -> ~/.ae/bin`
/// stayed `/tmp/…/.ae/bin`, so the prefix test could never match and the guard
/// did not fire. Measured: with the home under `/private/tmp` the same install
/// refused correctly, and under `/tmp` it published a version directory first
/// and was only caught later, incidentally, by the symlinked-ancestor check in
/// [`missing_chain`].
fn resolve_existing(path: &Path, budget: u8) -> PathBuf {
    let is_link = door::lstat(path).is_ok_and(|meta| meta.file_type().is_symlink());
    if is_link {
        let Ok(target) = std::fs::read_link(path) else {
            return path.to_path_buf();
        };
        let absolute = if target.is_absolute() {
            target
        } else {
            path.parent().unwrap_or(Path::new("/")).join(target)
        };
        return std::fs::canonicalize(&absolute).unwrap_or_else(|_| {
            if budget == 0 {
                absolute
            } else {
                resolve_nearest_within(&absolute, budget - 1)
            }
        });
    }
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

// ─── the bundle ──────────────────────────────────────────────────────────

/// A bundle directory that has been proven, and the version it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    /// The extracted bundle root.
    pub dir: PathBuf,
    /// The version its own core reports.
    pub version: String,
}

/// Whether `version` is the `CalVer` the product publishes.
#[must_use]
pub fn is_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// SHA-256 of `bytes`, lowercase hex — the spelling both `sha256sum` and
/// `shasum -a 256` emit, and the one [`crate::shape::parse_manifest`] accepts.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
    let mut out = String::with_capacity(64);
    for byte in digest.as_ref() {
        // Infallible: writing to a String cannot fail, and a `?` here would
        // make a formatting detail part of this function's contract.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Prove `dir` is a bundle, and answer with the version it carries.
///
/// The order is the security property. Members are classified WITHOUT following
/// a link, the manifest is parsed and checked to cover exactly the two
/// executable members, both members are re-digested — and only then is the
/// bundle's own core executed to ask its version. Nothing unverified runs.
///
/// # Errors
///
/// One line naming what failed: a missing, non-regular or non-executable
/// member; a manifest that does not parse or covers the wrong set; a digest
/// that disagrees; a core that will not run or reports a version that is not
/// `CalVer`.
pub fn verify(dir: &Path) -> Result<Bundle, String> {
    for name in [
        crate::shape::CORE,
        crate::shape::INSTALLER,
        crate::shape::MANIFEST,
    ] {
        let meta = door::lstat(&dir.join(name))
            .map_err(|_| format!("the bundle is missing {name}: {}", dir.display()))?;
        if !meta.file_type().is_file() {
            return Err(format!(
                "the bundle's {name} is not a regular non-symlink file"
            ));
        }
    }
    for name in [crate::shape::CORE, crate::shape::INSTALLER] {
        use std::os::unix::fs::PermissionsExt as _;
        let meta = door::lstat(&dir.join(name))
            .map_err(|why| format!("the bundle's {name} cannot be read: {why}"))?;
        if meta.permissions().mode() & 0o111 == 0 {
            return Err(format!("the bundle's {name} is not executable"));
        }
    }
    let text = door::read_text(&dir.join(crate::shape::MANIFEST))
        .map_err(|why| format!("the bundle manifest cannot be read: {why}"))?;
    let names = crate::shape::parse_manifest(&text)
        .map_err(|why| format!("{}: {why}", crate::shape::MANIFEST))?;
    if !crate::shape::manifest_covers_members(&names) {
        return Err(format!(
            "{} does not name exactly {} and {}",
            crate::shape::MANIFEST,
            crate::shape::CORE,
            crate::shape::INSTALLER
        ));
    }
    for line in text.lines() {
        let Some((expected, name)) = line.split_once("  ") else {
            continue;
        };
        let bytes = door::read(&dir.join(name))
            .map_err(|why| format!("the bundle's {name} cannot be read: {why}"))?;
        if sha256_hex(&bytes) != expected {
            return Err(format!(
                "checksum mismatch for {name}; nothing was installed"
            ));
        }
    }
    let version = core_version(&dir.join(crate::shape::CORE))?;
    if !is_version(&version) {
        return Err(format!(
            "the bundle core reports a version that is not CalVer: {version}"
        ));
    }
    Ok(Bundle {
        dir: dir.to_path_buf(),
        version,
    })
}

/// Ask a VERIFIED core which version it is.
///
/// The version directory is named for this answer, and
/// [`crate::shape::validate`] compares the two again on every later invocation
/// — so a publish under the wrong name would brick the install at its first
/// run. Asking here turns that into an install-time refusal.
fn core_version(core: &Path) -> Result<String, String> {
    let out = run_core(core).map_err(|why| format!("the bundle core did not run: {why}"))?;
    if !out.status.success() {
        return Err("the bundle core did not report a version".to_owned());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next().unwrap_or_default();
    first
        .split_whitespace()
        .next_back()
        .map(str::to_owned)
        .ok_or_else(|| "the bundle core printed no version line".to_owned())
}

/// THE FOURTH PRODUCT CROSSING of `clippy.toml`'s `Command` deny, and the only
/// one that is neither tmux nor an `exec`.
///
/// It runs ONE program — the bundle member this module has just proven by
/// digest — with ONE fixed argument, and reads its first line. There is no
/// caller input in the argv at all. The alternative was to trust the archive's
/// directory name for the version the whole install gate is keyed on.
fn run_core(core: &Path) -> std::io::Result<std::process::Output> {
    #[allow(
        clippy::disallowed_types,
        reason = "the install's own version probe: a DIGEST-VERIFIED bundle core is asked which version it is, because the version directory is named for that answer"
    )]
    let mut command = std::process::Command::new(core);
    command.arg("--version").output()
}

// ─── the journal ─────────────────────────────────────────────────────────

/// The record a half-done publish is reversed from.
///
/// It is BOTH the record and the lock. The bash installer kept a separate
/// `mkdir` lock beside it, which meant two objects, two acquire paths and a
/// stale-takeover dance between them. One object with a PID in it answers both
/// questions: a live PID means another install is running, a dead one means an
/// interrupted install to replay before this one starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Journal {
    /// The process that owns this install.
    pub pid: u32,
    /// The version being published.
    pub version: String,
    /// The ae home this record is for.
    pub home: PathBuf,
    /// The command link.
    pub link: PathBuf,
    /// Whether a command link existed before this install.
    pub link_had: bool,
    /// What it pointed at, when it did.
    pub link_old: String,
    /// Directories this install created, in creation order.
    pub created: Vec<PathBuf>,
}

impl Journal {
    /// The file's bytes, in the order [`Journal::parse`] accepts them.
    #[must_use]
    pub fn render(&self) -> String {
        use std::fmt::Write as _;
        let mut text = String::new();
        let _ = writeln!(text, "format={JOURNAL_FORMAT}");
        let _ = writeln!(text, "pid={}", self.pid);
        let _ = writeln!(text, "version={}", self.version);
        let _ = writeln!(text, "home={}", self.home.display());
        let _ = writeln!(text, "link={}", self.link.display());
        let _ = writeln!(text, "link_had={}", u8::from(self.link_had));
        let _ = writeln!(text, "link_old={}", self.link_old);
        for path in &self.created {
            let _ = writeln!(text, "created_dir={}", path.display());
        }
        text
    }

    /// Parse a journal, holding it to the record THIS installer emits.
    ///
    /// Hostile by assumption: the file is hand-editable, it survives a crash,
    /// and replaying it removes directories and rewrites the command link. Every
    /// field is checked against a fact the caller already knows, and an unknown
    /// field is a refusal rather than a row this parser skips — a journal from
    /// another installer names things this one would half-replay.
    ///
    /// # Errors
    ///
    /// One line naming the field that disagreed.
    pub fn parse(text: &str, paths: &Paths) -> Result<Self, String> {
        let mut format = "";
        let mut pid = "";
        let mut version = "";
        let mut home = "";
        let mut link = "";
        let mut link_had = "";
        let mut link_old = "";
        let mut created: Vec<PathBuf> = Vec::new();
        for line in text.lines() {
            if line.is_empty() {
                continue;
            }
            let (key, value) = line.split_once('=').unwrap_or((line, ""));
            match key {
                "format" => format = value,
                "pid" => pid = value,
                "version" => version = value,
                "home" => home = value,
                "link" => link = value,
                "link_had" => link_had = value,
                "link_old" => link_old = value,
                "created_dir" => created.push(PathBuf::from(value)),
                other => {
                    return Err(format!(
                        "install journal has an unknown field: {other} (a journal from another installer names retired fields)"
                    ));
                }
            }
        }
        if format != JOURNAL_FORMAT {
            return Err(format!(
                "install journal is format '{format}', this installer writes {JOURNAL_FORMAT}"
            ));
        }
        let pid: u32 = pid
            .parse()
            .map_err(|_| format!("install journal has an invalid pid: {pid}"))?;
        if !is_version(version) {
            return Err(format!("install journal version is invalid: {version}"));
        }
        if Path::new(home) != paths.home {
            return Err("install journal is malformed or for another home".to_owned());
        }
        let link = PathBuf::from(link);
        if link != paths.link {
            return Err("install journal is for another command path".to_owned());
        }
        validate_bin_destination(&link, &paths.home)?;
        let link_had = match link_had {
            "0" => false,
            "1" => true,
            other => return Err(format!("install journal has an invalid link_had: {other}")),
        };
        // The emitted set, exactly. A replay removes directories, so a row
        // naming anything this installer never creates is refused rather than
        // reversed — that is how `created_dir=<a live session directory>`
        // becomes a refusal instead of an `rmdir`.
        let emitted = emitted_dirs(paths);
        for path in &created {
            path_components_ok(path, "install journal directory")?;
            if !emitted.contains(path) {
                return Err(format!(
                    "install journal directory path is not one of the emitted directories: {}",
                    path.display()
                ));
            }
        }
        Ok(Self {
            pid,
            version: version.to_owned(),
            home: paths.home.clone(),
            link,
            link_had,
            link_old: link_old.to_owned(),
            created,
        })
    }
}

/// Every directory a publish can create, and therefore the only ones a replay
/// may remove.
fn emitted_dirs(paths: &Paths) -> Vec<PathBuf> {
    let mut out = vec![paths.versions()];
    let mut cursor = paths.link.parent().map(Path::to_path_buf);
    // The command link's parent and its parent: `~/.local/bin` and `~/.local`.
    for _ in 0..2 {
        if let Some(path) = cursor {
            out.push(path.clone());
            cursor = path.parent().map(Path::to_path_buf);
        }
    }
    out
}

// ─── publication ─────────────────────────────────────────────────────────

/// What a publish did, for the caller that reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    /// The version directory now on disk.
    pub version_dir: PathBuf,
    /// The version published.
    pub version: String,
}

/// Publish `bundle` under `paths`, atomically, and repoint the command link.
///
/// The steps, in the order that makes each one reversible:
///
/// 1. the home is created (untracked — an empty `~/.ae` is not a failure), then
///    the command destination is REVALIDATED, because creating the home can
///    make a dangling ancestor of that destination live;
/// 2. an existing journal is replayed and removed, or refused;
/// 3. this install's journal is created with `O_EXCL` — the create IS the lock;
/// 4. `versions/` is created and recorded;
/// 5. the version directory is staged beside itself, moded, and RENAMED into
///    place, so it is never visible at a writable mode;
/// 6. the command link's parent is created and recorded, and the link is
///    replaced by a rename of a temp symlink beside it;
/// 7. the journal is removed.
///
/// A failure anywhere after step 3 replays the journal and removes it. A SIGNAL
/// leaves the journal on disk, which is the same outcome by a different route:
/// the next run finds a dead PID and replays it before starting.
///
/// # Errors
///
/// One line naming what failed. On a failure whose rollback also failed, the
/// journal is PRESERVED and the message says so — the journal is the
/// retryability, so removing it is the irreversible step.
pub fn publish(bundle: &Bundle, paths: &Paths) -> Result<Published, String> {
    validate_home(&paths.home)?;
    validate_bin_destination(&paths.link, &paths.home)?;
    mkdir_all_plain(&paths.home)?;
    // B14: creating the home can make a dangling ancestor of the command path
    // live. Revalidate before anything is published.
    validate_bin_destination(&paths.link, &paths.home)?;
    recover_existing(paths)?;

    let (link_had, link_old) = current_link(&paths.link);
    let mut journal = Journal {
        pid: std::process::id(),
        version: bundle.version.clone(),
        home: paths.home.clone(),
        link: paths.link.clone(),
        link_had,
        link_old,
        created: Vec::new(),
    };
    let path = paths.journal();
    write_new(&path, &journal.render())
        .map_err(|why| format!("could not write install journal: {why}"))?;

    match publish_steps(bundle, paths, &mut journal) {
        Ok(published) => {
            std::fs::remove_file(&path).map_err(|why| {
                format!(
                    "install succeeded but the journal could not be removed; it is preserved at {}: {why}",
                    path.display()
                )
            })?;
            Ok(published)
        }
        Err(why) => {
            if let Err(unwound) = replay(&journal, &path) {
                return Err(format!(
                    "{why}\nae: rollback incomplete ({unwound}); journal preserved at {} — rerun the installer to recover.",
                    path.display()
                ));
            }
            Err(why)
        }
    }
}

/// Everything between the journal's creation and its removal.
fn publish_steps(
    bundle: &Bundle,
    paths: &Paths,
    journal: &mut Journal,
) -> Result<Published, String> {
    let versions = paths.versions();
    mkdir_recorded(&versions, journal, paths)?;
    let version_dir = versions.join(&bundle.version);
    publish_version_dir(bundle, &version_dir)?;

    let parent = paths
        .link
        .parent()
        .ok_or_else(|| format!("ae command path has no parent: {}", paths.link.display()))?;
    mkdir_recorded(parent, journal, paths)?;
    validate_bin_destination(&paths.link, &paths.home)?;
    relink(&paths.link, &version_dir.join(crate::shape::CORE))?;
    Ok(Published {
        version_dir,
        version: bundle.version.clone(),
    })
}

/// Stage the three members, mode them, and rename the whole directory in.
///
/// A version already on disk is not republished: its members must be regular
/// files with the SAME bytes, and then the published modes are re-asserted. A
/// version directory holding different bytes under the same name is refused —
/// the name is a promise about the content.
fn publish_version_dir(bundle: &Bundle, version_dir: &Path) -> Result<(), String> {
    if door::lstat(version_dir).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(format!(
            "installed version directory is a symlink, refusing to write through it: {}",
            version_dir.display()
        ));
    }
    if door::present(version_dir) {
        for name in [
            crate::shape::CORE,
            crate::shape::INSTALLER,
            crate::shape::MANIFEST,
        ] {
            let meta = door::lstat(&version_dir.join(name))
                .map_err(|_| format!("version {} is installed without {name}", bundle.version))?;
            if !meta.file_type().is_file() {
                return Err(format!(
                    "the installed {name} is not a regular non-symlink file"
                ));
            }
            let installed = door::read(&version_dir.join(name))
                .map_err(|why| format!("the installed {name} cannot be read: {why}"))?;
            let incoming = door::read(&bundle.dir.join(name))
                .map_err(|why| format!("the bundle's {name} cannot be read: {why}"))?;
            if installed != incoming {
                return Err(format!(
                    "version {} is already installed with different bytes",
                    bundle.version
                ));
            }
        }
        // Re-assert the published modes: the directory is the only thing
        // standing between the installed core and a stray write, so it is
        // restated rather than assumed.
        apply_modes(version_dir)?;
        return Ok(());
    }

    let stage = version_dir.with_file_name(format!(".stage.{}", std::process::id()));
    let _ = remove_private_tree(&stage);
    std::fs::create_dir(&stage).map_err(|why| {
        format!(
            "could not stage the version directory at {}: {why}",
            stage.display()
        )
    })?;
    let staged = stage_members(bundle, &stage).and_then(|()| apply_member_modes(&stage));
    if let Err(why) = staged {
        let _ = remove_private_tree(&stage);
        return Err(why);
    }
    if let Err(why) = std::fs::rename(&stage, version_dir) {
        let _ = remove_private_tree(&stage);
        return Err(format!(
            "could not publish {}: {why}",
            version_dir.display()
        ));
    }
    seal_dir(version_dir)
}

/// Copy the three members into the stage.
fn stage_members(bundle: &Bundle, stage: &Path) -> Result<(), String> {
    for name in [
        crate::shape::CORE,
        crate::shape::INSTALLER,
        crate::shape::MANIFEST,
    ] {
        std::fs::copy(bundle.dir.join(name), stage.join(name))
            .map_err(|why| format!("could not stage {name}: {why}"))?;
    }
    Ok(())
}

/// The published modes, on a stage or on an already-installed directory.
///
/// The DIRECTORY is moded last: 0555 refuses an entry create, so moding it
/// first would make the member `chmod`s fail on a fresh publish.
fn apply_modes(dir: &Path) -> Result<(), String> {
    apply_member_modes(dir)?;
    seal_dir(dir)
}

/// The three members' modes, without touching the directory they sit in.
fn apply_member_modes(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    for (name, mode) in [
        (crate::shape::CORE, MEMBER_MODE),
        (crate::shape::INSTALLER, MEMBER_MODE),
        (crate::shape::MANIFEST, MANIFEST_MODE),
    ] {
        std::fs::set_permissions(dir.join(name), std::fs::Permissions::from_mode(mode))
            .map_err(|why| format!("could not set the published mode on {name}: {why}"))?;
    }
    Ok(())
}

/// The directory's own read-only mode: the LAST step of a publish, after the
/// rename. macOS refuses to rename a directory it may not write (measured on
/// the macos-15 lane: `rename` of a 0555 stage fails with EACCES even within
/// one parent), so the stage is renamed writable and sealed in place.
fn seal_dir(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(MEMBER_MODE)).map_err(|why| {
        format!(
            "could not set the published mode on {}: {why}",
            dir.display()
        )
    })
}

/// Replace the command link atomically.
///
/// A symlink at a temp name beside the destination, then a rename over it. The
/// rename is the switch: there is no window in which the command is absent, and
/// no `rm` that could leave one.
fn relink(link: &Path, target: &Path) -> Result<(), String> {
    let parent = link
        .parent()
        .ok_or_else(|| format!("ae command path has no parent: {}", link.display()))?;
    let temp = parent.join(format!(".ae.{}", std::process::id()));
    let _ = std::fs::remove_file(&temp);
    std::os::unix::fs::symlink(target, &temp)
        .map_err(|why| format!("could not stage the command link: {why}"))?;
    std::fs::rename(&temp, link).map_err(|why| {
        let _ = std::fs::remove_file(&temp);
        format!(
            "could not publish the command link {}: {why}",
            link.display()
        )
    })
}

// ─── directories ─────────────────────────────────────────────────────────

/// Create `path` and every missing ancestor, refusing to create THROUGH a
/// symlink.
///
/// Used for the home alone, which is created before there is a journal to record
/// it in. An empty `~/.ae` left behind by a failure is not a defect worth a
/// reversal.
fn mkdir_all_plain(path: &Path) -> Result<(), String> {
    for missing in missing_chain(path)?.iter().rev() {
        std::fs::create_dir(missing)
            .map_err(|why| format!("could not create directory {}: {why}", missing.display()))?;
    }
    Ok(())
}

/// The same, recording each directory this process actually created.
///
/// **Recorded AFTER the syscall succeeded**, which is the whole ownership rule:
/// a row written first would claim a directory an operator created in the gap,
/// and a replay that removes one it does not own is the irreversible mistake.
/// The gap in this direction leaves an empty directory unrecorded, which is the
/// safe half of the trade.
fn mkdir_recorded(path: &Path, journal: &mut Journal, paths: &Paths) -> Result<(), String> {
    for missing in missing_chain(path)?.iter().rev() {
        std::fs::create_dir(missing)
            .map_err(|why| format!("could not create directory {}: {why}", missing.display()))?;
        journal.created.push(missing.clone());
        append_line(
            &paths.journal(),
            &format!("created_dir={}\n", missing.display()),
        )
        .map_err(|why| format!("could not record directory ownership: {why}"))?;
    }
    Ok(())
}

/// The missing directories between `path` and its nearest existing ancestor,
/// deepest first.
///
/// A symlink or a non-directory anywhere on the way is a refusal, not something
/// to create through: a dangling `~/.local -> ~/.ae` would otherwise be followed
/// into the ae home.
fn missing_chain(path: &Path) -> Result<Vec<PathBuf>, String> {
    let mut missing = Vec::new();
    let mut cursor = path.to_path_buf();
    loop {
        match door::lstat(&cursor) {
            Ok(meta) if meta.file_type().is_dir() => return Ok(missing),
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!(
                    "cannot create directory through a symlink: {}",
                    cursor.display()
                ));
            }
            Ok(_) => {
                return Err(format!(
                    "cannot create directory over a non-directory path: {}",
                    cursor.display()
                ));
            }
            Err(_) => {}
        }
        missing.push(cursor.clone());
        let Some(parent) = cursor.parent().map(Path::to_path_buf) else {
            return Err(format!(
                "cannot find a parent directory for: {}",
                path.display()
            ));
        };
        if parent == cursor {
            return Err(format!(
                "cannot find a parent directory for: {}",
                path.display()
            ));
        }
        cursor = parent;
    }
}

// ─── recovery ────────────────────────────────────────────────────────────

/// Replay a journal left behind by an earlier install, or refuse.
///
/// A LIVE pid means another install owns this home: refuse without touching
/// anything. A dead one means an interrupted install: replay it and remove it,
/// then carry on. A journal this parser will not accept is preserved for
/// diagnosis and refuses the run — nothing is half-replayed.
fn recover_existing(paths: &Paths) -> Result<(), String> {
    let path = paths.journal();
    let Ok(meta) = door::lstat(&path) else {
        return Ok(());
    };
    if !meta.file_type().is_file() {
        return Err(format!(
            "install journal is not a regular file: {}",
            path.display()
        ));
    }
    let text =
        door::read_text(&path).map_err(|why| format!("install journal cannot be read: {why}"))?;
    let journal = Journal::parse(&text, paths).map_err(|why| {
        format!(
            "{why}; journal preserved at {} for diagnosis",
            path.display()
        )
    })?;
    if owner_alive(journal.pid) {
        return Err(format!(
            "another install is in progress (pid {}, journal {}); if no install is running, remove that file and rerun",
            journal.pid,
            path.display()
        ));
    }
    replay(&journal, &path).map_err(|why| {
        format!(
            "{why}; journal preserved at {} — rerun the installer to recover",
            path.display()
        )
    })
}

/// Whether the process that wrote a journal is still there.
///
/// **A snapshot that cannot be taken answers YES**, and the direction is the
/// decision: `crate::procs` documents failure as UNKNOWN and never as dead, and
/// replaying a LIVE install's journal would restore its old command link while
/// it publishes. A wedge costs one documented `rm`; the other direction costs
/// an install nobody can name.
fn owner_alive(pid: u32) -> bool {
    crate::procs::snapshot().is_none_or(|table| table.iter().any(|proc| proc.pid == pid))
}

/// Undo what `journal` records, then remove it.
///
/// The command link is restored first because it is the only object a caller can
/// already be using; the created directories follow, deepest last recorded and
/// therefore removed first. `remove_dir` never removes content, so a directory
/// that has since gained an entry is left as it stands.
fn replay(journal: &Journal, path: &Path) -> Result<(), String> {
    if journal.link_had {
        relink(&journal.link, Path::new(&journal.link_old))?;
    } else if door::present(&journal.link) {
        if door::lstat(&journal.link).is_ok_and(|meta| meta.file_type().is_dir()) {
            return Err(format!(
                "rollback cannot restore the command link over a directory: {}",
                journal.link.display()
            ));
        }
        std::fs::remove_file(&journal.link)
            .map_err(|why| format!("rollback could not remove the command link: {why}"))?;
    }
    for created in journal.created.iter().rev() {
        // `remove_dir` never removes content, so its failure is not a failed
        // reversal: an absent directory is already undone, and a NON-EMPTY one
        // holds something this install did not put there — a retained version
        // directory, or an operator's own file. Both are the designed final
        // state.
        let _ = std::fs::remove_dir(created);
    }
    std::fs::remove_file(path).map_err(|why| format!("could not remove the journal: {why}"))
}

// ─── small filesystem helpers ────────────────────────────────────────────

/// The command link's current target, when it has one.
fn current_link(link: &Path) -> (bool, String) {
    if door::lstat(link).is_ok_and(|meta| meta.file_type().is_symlink()) {
        let target = std::fs::read_link(link).unwrap_or_default();
        return (true, target.to_string_lossy().into_owned());
    }
    (false, String::new())
}

/// Create a file that must not already exist, at 0600.
///
/// `create_new` is the `O_EXCL` create, and it is what makes the journal the lock:
/// exactly one of two racing installs wins it, and the loser sees the winner's
/// live PID.
fn write_new(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()
}

/// Append one whole line to the journal.
fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new().append(true).open(path)?;
    file.write_all(line.as_bytes())?;
    file.sync_all()
}

/// Remove one of OUR OWN private trees, whose members may be 0555.
///
/// A 0555 directory refuses the unlink of its own entries, so a plain
/// `remove_dir_all` on a staged (or extracted) bundle fails with `EACCES`. The
/// only arguments are this process's own temp namespaces, never a canonical
/// path.
///
/// # Errors
///
/// The tree could not be removed even after its members were re-moded.
pub fn remove_private_tree(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: a transaction-private tree is enumerated to re-mode its members before removal — 0555 refuses the unlink otherwise"
    )]
    let entries = std::fs::read_dir(path);
    if let Ok(entries) = entries {
        for entry in entries.flatten() {
            let child = entry.path();
            if door::lstat(&child).is_ok_and(|meta| meta.file_type().is_dir()) {
                remove_private_tree(&child)?;
            } else {
                let _ = std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o644));
            }
        }
    }
    std::fs::remove_dir_all(path)
}

// ─── the command ─────────────────────────────────────────────────────────

/// `ae _install --from <dir>` — verify a bundle and publish it.
///
/// The bootstrap `install` script's whole second half: it downloads, checks the
/// archive against the release manifest, extracts, and runs the core it just
/// unpacked with this word.
///
/// # Errors
///
/// Propagates a write failure on the caller's streams.
pub fn run(
    tail: &[String],
    out: &mut impl std::io::Write,
    err: &mut impl std::io::Write,
) -> crate::Result<u8> {
    let [flag, dir] = tail else {
        writeln!(err, "{USAGE}")?;
        err.flush()?;
        return Ok(crate::entry::EXIT_USAGE);
    };
    if flag != "--from" {
        writeln!(err, "{USAGE}")?;
        err.flush()?;
        return Ok(crate::entry::EXIT_USAGE);
    }
    let Some(home) = crate::doors::home() else {
        writeln!(err, "ae: HOME is not set, so there is nowhere to install.")?;
        err.flush()?;
        return Ok(crate::entry::EXIT_USAGE);
    };
    match install_from(Path::new(dir), &home) {
        Ok(published) => {
            writeln!(
                out,
                "ae: installed {} under {}",
                published.version,
                published.version_dir.display()
            )?;
            out.flush()?;
            Ok(0)
        }
        Err(why) => {
            writeln!(err, "ae: {why}")?;
            err.flush()?;
            Ok(crate::entry::EXIT_FAILED)
        }
    }
}

/// Verify then publish — the one path `_install` and `upgrade` share.
///
/// # Errors
///
/// Whatever [`verify`] or [`publish`] refused, verbatim.
pub fn install_from(dir: &Path, home: &Path) -> Result<Published, String> {
    let paths = fixed_paths(home)?;
    let bundle = verify(dir)?;
    publish(&bundle, &paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Paths {
        Paths {
            home: PathBuf::from("/u/me/.ae"),
            link: PathBuf::from("/u/me/.local/bin/ae"),
        }
    }

    #[test]
    fn the_fixed_paths_are_home_and_nothing_else() {
        let derived = fixed_paths(Path::new("/u/me")).expect("a plain home");
        assert_eq!(derived, paths());
        // One trailing slash is a spelling, not a different home.
        assert_eq!(fixed_paths(Path::new("/u/me/")).expect("trailing"), paths());
    }

    #[test]
    fn a_home_that_cannot_be_journalled_is_refused_before_anything_is_derived() {
        for hostile in ["relative/home", "/u/../etc", "/u//me", "/u/me\n/evil"] {
            assert!(
                fixed_paths(Path::new(hostile)).is_err(),
                "accepted a hostile HOME: {hostile}"
            );
        }
    }

    #[test]
    fn a_command_path_inside_the_ae_home_is_refused() {
        let paths = paths();
        assert!(
            validate_bin_destination(Path::new("/u/me/.ae/bin/ae"), &paths.home).is_err(),
            "a destination inside the home would be published under the immutable tree"
        );
        assert!(validate_bin_destination(&paths.link, &paths.home).is_ok());
    }

    #[test]
    fn calver_is_three_numbers_and_nothing_else() {
        assert!(is_version("2026.9.1"));
        assert!(is_version("2026.12.10"));
        for bad in [
            "",
            "2026.9",
            "2026.9.1.2",
            "v2026.9.1",
            "2026.9.x",
            "2026..1",
        ] {
            assert!(!is_version(bad), "accepted {bad}");
        }
    }

    #[test]
    fn the_digest_is_the_spelling_both_checksum_tools_emit() {
        // The empty input's SHA-256, lowercase hex — the value `sha256sum` and
        // `shasum -a 256` both print, and the one `parse_manifest` accepts.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn a_journal_round_trips_through_its_own_parser() {
        let paths = paths();
        let journal = Journal {
            pid: 4242,
            version: "2026.9.1".to_owned(),
            home: paths.home.clone(),
            link: paths.link.clone(),
            link_had: true,
            link_old: "/u/me/.ae/versions/2026.8.2/ae-core".to_owned(),
            created: vec![paths.versions()],
        };
        assert_eq!(
            Journal::parse(&journal.render(), &paths).expect("its own record"),
            journal
        );
    }

    #[test]
    fn a_journal_from_another_installer_is_refused_rather_than_half_replayed() {
        let paths = paths();
        let good = Journal {
            pid: 1,
            version: "2026.9.1".to_owned(),
            home: paths.home.clone(),
            link: paths.link.clone(),
            link_had: false,
            link_old: String::new(),
            created: Vec::new(),
        }
        .render();
        // The bash journal's retired fields, a foreign home, a foreign command
        // path, a bad version, a bad flag, and a directory row naming live
        // state — every one a refusal, none a partial replay.
        for hostile in [
            format!("{good}config=/u/me/.ae/config\n"),
            good.replace("format=3", "format=2"),
            good.replace("/u/me/.ae", "/u/you/.ae"),
            good.replace("link=/u/me/.local/bin/ae", "link=/u/me/.local/bin/ae-next"),
            good.replace("version=2026.9.1", "version=latest"),
            good.replace("link_had=0", "link_had=maybe"),
            format!("{good}created_dir=/u/me/.ae/sessions/live\n"),
        ] {
            assert!(
                Journal::parse(&hostile, &paths).is_err(),
                "accepted a hostile journal: {hostile}"
            );
        }
    }

    #[test]
    fn the_emitted_directories_are_the_only_ones_a_replay_may_remove() {
        let paths = paths();
        let emitted = emitted_dirs(&paths);
        assert!(emitted.contains(&PathBuf::from("/u/me/.ae/versions")));
        assert!(emitted.contains(&PathBuf::from("/u/me/.local/bin")));
        assert!(emitted.contains(&PathBuf::from("/u/me/.local")));
        assert!(
            !emitted.contains(&PathBuf::from("/u/me/.ae")),
            "the home is created untracked and must never be a reversal target"
        );
    }
}
