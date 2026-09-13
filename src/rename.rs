//! `ae rename [old] <new>` — the whole rename, under lifecycle locking.
//!
//! A session name is five things at once: a tmux session, a directory under
//! `<AE_HOME>/sessions`, the `session=` row in its meta, part of the
//! `.lifecycle.<name>.lock` filename, and the text in the status bar. A live
//! rename moves those together or refuses; a stopped rename converges them
//! through a recoverable transaction (durable intent, ordered moves, checked
//! assets, durable result) that a proved retry completes after interruption.
//! Either way every check that reads state it then mutates happens INSIDE
//! the lock rather than in front of it.

use std::io::Write;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use crate::inventory::ServerId;
use crate::meta::Selector;
use crate::session_tmux::{Op, argv};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::transport;

/// The usage, both lines.
pub const USAGE: &str = "Usage: ae rename [old-name] <new-name>";

/// The second usage line, printed only for the one-operand form: it explains
/// why the old name was needed.
pub const USAGE_INSIDE: &str =
    "(Run inside an ae tmux session to rename it without specifying the old name.)";

// ---- Slice-A crash seam ---------------------------------------------------
//
// One CHECKOUT-only variable: `AE_TEST_RENAME_CRASH_AT`. Exact ASCII grammar:
// `after-intent|after-work-move|after-state-move|after-meta|after-assets|
// after-result`. Absent disables; an unknown, empty or combined value refuses
// before rename side effects in CHECKOUT. The use-site activation checks
// `shape::current().honours_environment()` BEFORE any environment read, so an
// installed core never reads the variable and its normal entry ignores even an
// invalid ambient value. Inventory change: one AGENTS.md environment-door row
// only; `src/doors.rs` and `tests/it/doors.rs` are untouched by this seam.
// Each armed value parks at most 60 seconds past its committed facts (the
// ceiling is pinned by the A2 park test), then exits terminally with the
// existing `EXIT_FAILED`/1
// and the exact `rename-crash-timeout: <value>` diagnostic, executing zero
// later rename/result steps. No new product `Command` site, no timeout knob,
// no public flag.
pub(crate) const CRASH_VAR: &str = "AE_TEST_RENAME_CRASH_AT";

/// The closed boundary grammar: committed facts, not sequence positions (the
/// settled stopped order is durable intent → git/full work move → state-dir
/// move → coherent meta → checked assets → durable result/complete).
pub(crate) const CRASH_BOUNDARIES: [&str; 6] = [
    "after-intent",
    "after-work-move",
    "after-state-move",
    "after-meta",
    "after-assets",
    "after-result",
];

/// Whether `value` is exactly one armed boundary.
#[must_use]
pub(crate) fn is_crash_boundary(value: &str) -> bool {
    CRASH_BOUNDARIES.contains(&value)
}

/// What the crash gate decided, before any rename side effect.
enum CrashGate {
    /// Proceed, unarmed (installed, or the variable absent in CHECKOUT).
    Proceed(Option<String>),
    /// Refused: the diagnostic is already on `err`.
    Refuse,
}

/// Validate the crash-seam grammar before any rename side effect.
///
/// The shape check runs BEFORE the environment read: an installed core takes
/// the unarmed path without reading the variable at all.
fn crash_gate(err: &mut impl Write) -> crate::Result<CrashGate> {
    if !crate::shape::current().honours_environment() {
        return Ok(CrashGate::Proceed(None));
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the CHECKOUT-only rename crash seam reads its closed-grammar variable at its rename use site, after the honours_environment gate — see AGENTS.md"
    )]
    let raw = std::env::var_os(CRASH_VAR);
    let Some(raw) = raw else {
        return Ok(CrashGate::Proceed(None));
    };
    let value = raw.to_string_lossy().into_owned();
    if is_crash_boundary(&value) {
        return Ok(CrashGate::Proceed(Some(value)));
    }
    writeln!(
        err,
        "Error: {CRASH_VAR}='{value}' is not a rename crash boundary (want exactly one of: {}). Nothing was renamed.",
        CRASH_BOUNDARIES.join(", ")
    )?;
    Ok(CrashGate::Refuse)
}

/// `rename [old] <new>` — the whole operation.
///
/// # Errors
///
/// Propagates a write failure on the caller's streams.
pub fn run(
    root: &Path,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let (old, new) = match tail {
        [old, new] => (old.clone(), new.clone()),
        [new] => {
            let Some(old) = current_session(root) else {
                writeln!(err, "{USAGE}")?;
                writeln!(err, "{USAGE_INSIDE}")?;
                return Ok(EXIT_USAGE);
            };
            (old, new.clone())
        }
        _ => {
            writeln!(err, "{USAGE}")?;
            return Ok(EXIT_USAGE);
        }
    };

    // The TARGET is a creation boundary, so it takes the one grammar — not a
    // separator blacklist, which accepted `has space` and persisted a session
    // every later launch refused.
    if !crate::lifecycle::name_is_valid(&new) {
        writeln!(err, "Error: invalid session name '{new}'.")?;
        writeln!(
            err,
            "       Names must match {} — start with a letter or digit,",
            crate::session_launch::name::SESSION_NAME_GRAMMAR
        )?;
        writeln!(
            err,
            "       then letters, digits, '_' or '-', up to 128 characters."
        )?;
        return Ok(EXIT_FAILED);
    }
    if !crate::lifecycle::name_is_usable(root, &old) {
        writeln!(
            err,
            "Error: session name '{old}' cannot be used to reach a session."
        )?;
        return Ok(EXIT_FAILED);
    }
    let sessions = crate::lifecycle::sessions_dir(root);
    for name in [&old, &new] {
        if is_symlink(&sessions.join(name)) {
            writeln!(
                err,
                "Error: the session path for '{name}' is a symlink; refusing to rename through it."
            )?;
            return Ok(EXIT_FAILED);
        }
    }

    // The crash-seam grammar is validated before any rename side effect. An
    // armed value is carried into the locked operation; a no-op (below) never
    // emits a boundary for it.
    let CrashGate::Proceed(crash) = crash_gate(err)? else {
        return Ok(EXIT_FAILED);
    };

    // A0: the same name is a validated no-op under ONE lock. Taking the two
    // name-sorted locks below would open the same lockfile twice, wait out
    // the lock bound, and report another lifecycle operation.
    if old == new {
        return same_name_noop(root, &old, out, err);
    }

    // BOTH lifecycle locks, taken in name-sorted order.
    let (first, second) = if old < new {
        (&old, &new)
    } else {
        (&new, &old)
    };
    let held = crate::lifecycle::lock(root, first)
        .and_then(|one| crate::lifecycle::lock(root, second).map(|two| (one, two)));
    let Ok(_held) = held else {
        writeln!(
            err,
            "Error: another lifecycle operation (start/resume/stop/end) is in progress for '{old}' or '{new}' — retry shortly. Nothing was renamed."
        )?;
        return Ok(EXIT_FAILED);
    };
    locked(root, &old, &new, crash, out, err)
}

/// A0: `rename x x` validates the source under a single lock and reports an
/// explicit no-op. No second lock, no transaction, no restart, no crash-seam
/// attestation.
fn same_name_noop(
    root: &Path,
    name: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let held = crate::lifecycle::lock(root, name);
    let Ok(_held) = held else {
        writeln!(
            err,
            "Error: another lifecycle operation (start/resume/stop/end) is in progress for '{name}' — retry shortly. Nothing was renamed."
        )?;
        return Ok(EXIT_FAILED);
    };
    let sessions = crate::lifecycle::sessions_dir(root);
    let dir = sessions.join(name);
    if is_symlink(&dir) {
        writeln!(
            err,
            "Error: the session path for '{name}' is a symlink; refusing to rename through it."
        )?;
        return Ok(EXIT_FAILED);
    }
    if !crate::lifecycle::dir_exists(&dir) {
        writeln!(
            err,
            "Error: session '{name}' does not exist. Nothing was renamed."
        )?;
        return Ok(EXIT_FAILED);
    }
    let bytes = match crate::meta::read_bytes(&dir) {
        Ok(bytes) => bytes,
        Err(why) => {
            writeln!(
                err,
                "Error: session '{name}' has no readable meta ({why}). Nothing was renamed."
            )?;
            return Ok(EXIT_FAILED);
        }
    };
    let recorded = crate::lifecycle::meta_value(&bytes, "session");
    if recorded != name {
        writeln!(
            err,
            "Error: session '{name}' records session '{recorded}' — refusing a no-op over mismatched identity. Nothing was renamed."
        )?;
        return Ok(EXIT_FAILED);
    }
    writeln!(
        out,
        "Session '{name}' is already named '{name}' — nothing to do."
    )?;
    Ok(0)
}

// ---- Slice-A stopped rename: durable intent, ordered moves, recovery ----
//
// A stopped rename is a recoverable transaction in the SETTLED order:
// durable intent → git/full work move → state-dir move → coherent meta →
// checked assets → durable result/complete. Publishing the intent is the
// forward-completion decision: before it, no move occurs; after it, a partial
// failure converges forward on a proved retry of the same command. This is
// not atomic across systems, and no cross-system atomicity is promised.
//
// The intent lives OUTSIDE both moved directories, beside the lifecycle
// locks: `<sessions>/.rename.<old>.<new>.intent`. Old cores ignore the file
// (census skips dotfiles; the pre-A core refuses stopped sources outright),
// so no compatibility migration is needed for readers. Writers serialize on
// the same two lifecycle locks the rename holds throughout.

/// The intent document version this core writes and reads.
const INTENT_VERSION: &str = "1";

/// The most an intent document may be: a hostile file of any length must be
/// refused rather than parsed into an allocation it sizes.
const INTENT_CAP: usize = 16_384;

/// The settled stopped phases. The strings name committed facts, and a retry
/// resumes after the recorded one.
const PHASE_PREPARED: &str = "prepared";
const PHASE_WORK_MOVED: &str = "work-moved";
const PHASE_STATE_MOVED: &str = "state-moved";
const PHASE_META_PUBLISHED: &str = "meta-published";
const PHASE_ASSETS_PUBLISHED: &str = "assets-published";
const PHASE_COMPLETE: &str = "complete";

/// The settled order, first to last.
const PHASES: [&str; 6] = [
    PHASE_PREPARED,
    PHASE_WORK_MOVED,
    PHASE_STATE_MOVED,
    PHASE_META_PUBLISHED,
    PHASE_ASSETS_PUBLISHED,
    PHASE_COMPLETE,
];

/// Ordinal of a settled phase for before/after comparisons. Unknown phases
/// (unreachable: the validator closes the set) sort with `prepared`.
fn phase_ord(phase: &str) -> u8 {
    u8::try_from(PHASES.iter().position(|known| *known == phase).unwrap_or(0)).unwrap_or(0)
}

/// The highest phase whose cumulative facts currently verify: work, then
/// state, then meta, then assets. Local mode has no work phase. A carrier
/// claiming further ahead than the filesystem proves is normalized down to
/// this (and recorded); a record lagging behind verified facts catches up.
/// Either way the driver never skips on an unproved claim.
fn proven_phase(root: &Path, intent: &Intent) -> String {
    if intent.mode != WorkMode::Local && !work_moved(root, intent) {
        return PHASE_PREPARED.to_owned();
    }
    if !state_moved(root, intent) {
        return if intent.mode == WorkMode::Local {
            PHASE_PREPARED.to_owned()
        } else {
            PHASE_WORK_MOVED.to_owned()
        };
    }
    if !meta_coherent(root, intent) {
        return PHASE_STATE_MOVED.to_owned();
    }
    if !assets_ready(root, intent) {
        return PHASE_META_PUBLISHED.to_owned();
    }
    PHASE_ASSETS_PUBLISHED.to_owned()
}

/// A managed-work mode that decides the work path rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkMode {
    Local,
    Git,
    Full,
}

impl WorkMode {
    /// The mode a recorded `mode=` row names. Absent or unknown is damage,
    /// never a guess: the work path rule must not be inferred.
    fn parse(value: &str) -> Option<Self> {
        match value {
            "local" => Some(Self::Local),
            "git" => Some(Self::Git),
            "full" => Some(Self::Full),
            _ => None,
        }
    }

    /// The recorded spelling.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Git => "git",
            Self::Full => "full",
        }
    }
}

/// A validated rename intent: the whole transaction on one card.
///
/// Public because hostile intent bytes arrive from outside the process (a
/// crashed write or a hand edit plants them): the `meta_parse` fuzz target
/// drives the validator directly, the way it drives `Meta::parse`. Fields
/// stay private; construction runs through [`parse_intent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    uuid: String,
    old: String,
    new: String,
    mode: WorkMode,
    old_work: String,
    new_work: String,
    origin: String,
    server_kind: String,
    server_value: String,
    phase: String,
    /// Managed-work identity witness: `(device, inode)` of the work
    /// directory at intent time. A retry re-proves the same directory sits
    /// at the new address rather than a replacement. `0/0` in local mode,
    /// which moves no work.
    work_dev: u64,
    work_ino: u64,
    /// Git administrative identity witness: `(device, inode)` of the
    /// `.git/worktrees/<old>` directory at intent time (`git worktree move`
    /// preserves it). `0/0` outside git mode.
    admin_dev: u64,
    admin_ino: u64,
}

impl Intent {
    /// The source name this transaction renames away from.
    #[must_use]
    pub fn old_name(&self) -> &str {
        &self.old
    }

    /// The destination name this transaction renames toward.
    #[must_use]
    pub fn new_name(&self) -> &str {
        &self.new
    }

    /// The recorded settled phase.
    #[must_use]
    pub fn phase(&self) -> &str {
        &self.phase
    }
}

/// Parse and strictly validate hostile intent bytes.
///
/// Every field is required exactly once; unknown keys refuse (forward
/// compatibility rides on old cores IGNORING the file, not on tolerating new
/// keys). This is the hostile persisted-intent parser the `meta_parse` fuzz
/// target reaches.
///
/// # Errors
///
/// The first structural or semantic defect, naming the offending key or
/// shape — an oversize document, non-UTF-8 bytes, a missing or repeated key,
/// an unknown key, or a field outside its closed grammar.
#[allow(
    clippy::too_many_lines,
    reason = "one strict field table: splitting the closed-grammar checks across helpers would hide which shapes refuse"
)]
pub fn parse_intent(data: &[u8]) -> Result<Intent, String> {
    if data.len() > INTENT_CAP {
        return Err(format!(
            "rename intent exceeds {INTENT_CAP} bytes ({} bytes)",
            data.len()
        ));
    }
    let text = std::str::from_utf8(data).map_err(|_| "rename intent is not UTF-8".to_owned())?;
    let mut seen: Vec<&str> = Vec::new();
    // Collect raw rows first so duplicates and unknown keys are judged on the
    // whole document, not on parse order.
    let mut rows: Vec<(&str, &str)> = Vec::new();
    let mut body = text;
    if let Some(stripped) = body.strip_suffix('\n') {
        body = stripped;
    }
    if body.is_empty() {
        return Err("rename intent is empty".to_owned());
    }
    for line in body.split('\n') {
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("rename intent has a line without '=': {line:?}"));
        };
        if key.is_empty() || value.contains(['\n', '\r']) {
            return Err(format!("rename intent has a malformed line: {line:?}"));
        }
        if seen.contains(&key) {
            return Err(format!("rename intent repeats key '{key}'"));
        }
        seen.push(key);
        rows.push((key, value));
    }
    let value_of = |key: &str| -> Result<String, String> {
        rows.iter()
            .find_map(|(k, v)| (*k == key).then(|| (*v).to_owned()))
            .ok_or_else(|| format!("rename intent is missing key '{key}'"))
    };
    for (key, _) in &rows {
        match *key {
            "rename_intent" | "session_id" | "old" | "new" | "mode" | "old_work" | "new_work"
            | "origin" | "server_kind" | "server_value" | "phase" | "work_dev" | "work_ino"
            | "admin_dev" | "admin_ino" => {}
            _ => return Err(format!("rename intent has an unknown key '{key}'")),
        }
    }
    if value_of("rename_intent")? != INTENT_VERSION {
        return Err("rename intent version is not 1".to_owned());
    }
    let uuid = value_of("session_id")?;
    if crate::archive::canonical_uuid(&uuid).is_empty() {
        return Err("rename intent carries no canonical session UUID".to_owned());
    }
    let old = value_of("old")?;
    let new = value_of("new")?;
    if !crate::lifecycle::name_is_valid(&old) || !crate::lifecycle::name_is_valid(&new) {
        return Err("rename intent carries an invalid session name".to_owned());
    }
    if old == new {
        return Err("rename intent renames a session to itself".to_owned());
    }
    let mode = WorkMode::parse(&value_of("mode")?)
        .ok_or_else(|| "rename intent carries an unknown mode".to_owned())?;
    let old_work = value_of("old_work")?;
    let new_work = value_of("new_work")?;
    for (key, path) in [("old_work", &old_work), ("new_work", &new_work)] {
        if !path.starts_with('/') || path.contains('\0') {
            return Err(format!("rename intent carries a non-absolute {key}"));
        }
    }
    if mode == WorkMode::Local && old_work != new_work {
        return Err("rename intent moves work in local mode".to_owned());
    }
    if mode != WorkMode::Local && old_work == new_work {
        return Err("rename intent names no work move for a managed mode".to_owned());
    }
    let origin = value_of("origin")?;
    if origin.contains('\0') {
        return Err("rename intent carries a malformed origin".to_owned());
    }
    let server_kind = value_of("server_kind")?;
    let server_value = value_of("server_value")?;
    match server_kind.as_str() {
        "name" | "socket" if !server_value.is_empty() => {}
        "ambient" if server_value.is_empty() => {}
        _ => return Err("rename intent carries a malformed server pair".to_owned()),
    }
    if server_kind == "socket" && !server_value.starts_with('/') {
        return Err("rename intent carries a non-absolute server socket".to_owned());
    }
    let phase = value_of("phase")?;
    if !PHASES.contains(&phase.as_str()) {
        return Err(format!("rename intent carries an unknown phase '{phase}'"));
    }
    let witness = |key: &str| -> Result<u64, String> {
        value_of(key)?
            .parse()
            .map_err(|_| format!("rename intent carries a non-numeric {key}"))
    };
    let (work_dev, work_ino, admin_dev, admin_ino) = (
        witness("work_dev")?,
        witness("work_ino")?,
        witness("admin_dev")?,
        witness("admin_ino")?,
    );
    // Witness shape follows the mode: local moves nothing, full moves the
    // work alone, git moves both. A carrier claiming otherwise (including a
    // managed carrier with a zero fingerprint, or a half fingerprint) is
    // damage, not a transaction.
    let whole = |dev: u64, ino: u64| dev != 0 && ino != 0;
    let empty = |dev: u64, ino: u64| dev == 0 && ino == 0;
    let shaped = match mode {
        WorkMode::Local => empty(work_dev, work_ino) && empty(admin_dev, admin_ino),
        WorkMode::Full => whole(work_dev, work_ino) && empty(admin_dev, admin_ino),
        WorkMode::Git => whole(work_dev, work_ino) && whole(admin_dev, admin_ino),
    };
    if !shaped {
        return Err("rename intent carries a witness shape its mode cannot publish".to_owned());
    }
    Ok(Intent {
        uuid,
        old,
        new,
        mode,
        old_work,
        new_work,
        origin,
        server_kind,
        server_value,
        phase,
        work_dev,
        work_ino,
        admin_dev,
        admin_ino,
    })
}

/// Render an intent as its canonical document.
fn intent_document(intent: &Intent) -> String {
    format!(
        "rename_intent={}\nsession_id={}\nold={}\nnew={}\nmode={}\nold_work={}\nnew_work={}\norigin={}\nserver_kind={}\nserver_value={}\nphase={}\nwork_dev={}\nwork_ino={}\nadmin_dev={}\nadmin_ino={}\n",
        INTENT_VERSION,
        intent.uuid,
        intent.old,
        intent.new,
        intent.mode.as_str(),
        intent.old_work,
        intent.new_work,
        intent.origin,
        intent.server_kind,
        intent.server_value,
        intent.phase,
        intent.work_dev,
        intent.work_ino,
        intent.admin_dev,
        intent.admin_ino,
    )
}

/// The intent file for one rename pair: the live carrier a retry reads.
/// Both names satisfy the session grammar, so neither can escape the
/// sessions directory; see [`carrier_stem`] for the length bound.
#[must_use]
pub fn intent_path(root: &Path, old: &str, new: &str) -> PathBuf {
    crate::lifecycle::sessions_dir(root).join(format!("{}.intent", carrier_stem(old, new)))
}

/// The rotated resting place of a superseded completion: durable history no
/// reader consults, numbered when the stem is already taken.
#[must_use]
pub fn rotated_intent_path(root: &Path, old: &str, new: &str) -> PathBuf {
    crate::lifecycle::sessions_dir(root).join(format!("{}.intent.complete", carrier_stem(old, new)))
}

/// Filenames at or below this stem length keep literal names (so scanners can
/// attribute them without reading); longer pairs hash. Two legal 128-byte
/// names would overflow `NAME_MAX` (255); the digest shape stays tiny.
const CARRIER_STEM_CAP: usize = 200;

/// The carrier stem for a pair: literal names, or the stable digest when
/// they would not fit a directory entry.
fn carrier_stem(old: &str, new: &str) -> String {
    let full = format!(".rename.{old}.{new}");
    if full.len() <= CARRIER_STEM_CAP {
        full
    } else {
        format!(".rename.{}", pair_digest(old, new))
    }
}

/// FNV-1a 64-bit over the pair, hex. Stable across runs — `std` hashers are
/// randomly seeded and must never name files a retry recomputes.
fn pair_digest(old: &str, new: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in old.as_bytes().iter().chain(b"\0").chain(new.as_bytes()) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// What a carrier filename claims: literal names (attributable without
/// reading), a digest (attributable only by reading), or history/foreign
/// files no scan consults.
enum CarrierAddr {
    Literal { old: String, new: String },
    Digest,
    Ignored,
}

/// Classify a carrier filename. Rotated completions (`.intent.complete`,
/// numbered or not) are history, never live carriers.
fn carrier_addr(path: &Path) -> CarrierAddr {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return CarrierAddr::Ignored;
    };
    if !name.starts_with(".rename.") || name.ends_with(".intent.complete") {
        return CarrierAddr::Ignored;
    }
    // Numbered rotations end `.intent.complete.N`.
    if name.contains(".intent.complete.") {
        return CarrierAddr::Ignored;
    }
    let Some(inner) = name
        .strip_prefix(".rename.")
        .and_then(|stem| stem.strip_suffix(".intent"))
    else {
        return CarrierAddr::Ignored;
    };
    match inner.split_once('.') {
        Some((old, new))
            if crate::lifecycle::name_is_valid(old) && crate::lifecycle::name_is_valid(new) =>
        {
            CarrierAddr::Literal {
                old: old.to_owned(),
                new: new.to_owned(),
            }
        }
        None if inner.len() == 16 && inner.bytes().all(|byte| byte.is_ascii_hexdigit()) => {
            CarrierAddr::Digest
        }
        _ => CarrierAddr::Ignored,
    }
}

/// How a carrier path node classifies, without following or opening it.
enum NodeKind {
    Absent,
    Regular,
    Symlink,
    Other(&'static str),
}

/// Classify with `lstat`: never follow, never open, never block. A FIFO
/// would block an open forever; a symlink would hand reads to its target.
fn classify_node(path: &Path) -> NodeKind {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the rename's carrier classification must lstat the link itself — following it would hang on a FIFO or read through a link, see clippy.toml"
    )]
    let probe = std::fs::symlink_metadata(path);
    match probe {
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => NodeKind::Absent,
        Err(_) => NodeKind::Other("unclassifiable"),
        Ok(meta) => {
            let kind = meta.file_type();
            if kind.is_symlink() {
                NodeKind::Symlink
            } else if kind.is_file() {
                NodeKind::Regular
            } else if kind.is_dir() {
                NodeKind::Other("a directory")
            } else {
                NodeKind::Other("a special file")
            }
        }
    }
}

/// One classified carrier file: its attributable names (literal filename, or
/// payload names for a digest carrier that parses) and its outcome. `Err`
/// rows are damaged carriers the caller must report, never skip.
#[derive(Debug)]
struct Classified {
    old: Option<String>,
    new: Option<String>,
    result: Result<Intent, String>,
}

/// Classify one carrier file: node check, read, parse, and payload
/// correlation against a literal filename claim. Returns `None` for history
/// and foreign files.
fn classify_file(path: &Path) -> Option<Classified> {
    let addr = carrier_addr(path);
    if matches!(addr, CarrierAddr::Ignored) {
        return None;
    }
    let literal = match &addr {
        CarrierAddr::Literal { old, new } => Some((old.clone(), new.clone())),
        _ => None,
    };
    let damaged = |why: String| {
        let (old, new) = literal.clone().unzip();
        Some(Classified {
            old,
            new,
            result: Err(why),
        })
    };
    match classify_node(path) {
        NodeKind::Absent => None,
        NodeKind::Symlink => damaged(format!(
            "{} is a symlink; refusing to follow it",
            path.display()
        )),
        NodeKind::Other(what) => damaged(format!(
            "{} is {} — only a regular intent file may be read",
            path.display(),
            what
        )),
        NodeKind::Regular => {
            #[allow(
                clippy::disallowed_methods,
                reason = "a door: the rename's durable-intent read of a classified regular file — the recovery half of the stopped transaction, see clippy.toml"
            )]
            let bytes = match std::fs::read(path) {
                Ok(bytes) => bytes,
                Err(why) => {
                    return damaged(format!("could not read {} ({why})", path.display()));
                }
            };
            match parse_intent(&bytes) {
                Err(why) => damaged(format!(
                    "{} is not a valid rename intent ({why})",
                    path.display()
                )),
                Ok(intent) => {
                    if let Some((old, new)) = &literal {
                        if intent.old != *old || intent.new != *new {
                            return damaged(format!(
                                "{} names '{}' → '{}' — refusing a carrier that does not match its filename",
                                path.display(),
                                intent.old,
                                intent.new
                            ));
                        }
                    } else {
                        // Digest shape: the stem must be this payload's own
                        // digest. A mismatch is damage with attributable
                        // endpoints — the file passed the product carrier
                        // grammar and its valid payload names both parties —
                        // never trusted pending state its exact path would
                        // not consult, and never silently foreign: an
                        // interrupted or edited carrier must stay diagnosable.
                        let stem = path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .and_then(|name| {
                                name.strip_prefix(".rename.")
                                    .and_then(|stem| stem.strip_suffix(".intent"))
                            })
                            .unwrap_or("");
                        if format!(".rename.{stem}") != carrier_stem(&intent.old, &intent.new) {
                            return Some(Classified {
                                old: Some(intent.old.clone()),
                                new: Some(intent.new.clone()),
                                result: Err(format!(
                                    "{} names '{}' → '{}' under a foreign carrier stem — refusing a carrier that does not match its filename",
                                    path.display(),
                                    intent.old,
                                    intent.new
                                )),
                            });
                        }
                    }
                    let (old, new) = match &literal {
                        Some((old, new)) => (Some(old.clone()), Some(new.clone())),
                        None => (Some(intent.old.clone()), Some(intent.new.clone())),
                    };
                    Some(Classified {
                        old,
                        new,
                        result: Ok(intent),
                    })
                }
            }
        }
    }
}

/// One classified carrier: the live transaction, its durable result, its
/// absence, or a damaged carrier readers must report rather than skip.
pub(crate) enum Carrier {
    Missing,
    Pending(Intent),
    Complete(Intent),
    Damaged(DamagedCarrier),
}

/// A damaged carrier: attributable filename names where the filename is
/// literal (`None` for digest carriers no read can attribute), plus why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DamagedCarrier {
    pub old: Option<String>,
    pub new: Option<String>,
    pub why: String,
}

/// Read the exact carrier for a pair: node classification, content
/// validation, and filename/payload correlation in one owner, so every
/// caller classifies a mismatch as damage the same way.
pub(crate) fn read_carrier(root: &Path, old: &str, new: &str) -> Carrier {
    let path = intent_path(root, old, new);
    match classify_node(&path) {
        NodeKind::Absent => Carrier::Missing,
        NodeKind::Symlink => Carrier::Damaged(DamagedCarrier {
            old: Some(old.to_owned()),
            new: Some(new.to_owned()),
            why: format!("{} is a symlink; refusing to follow it", path.display()),
        }),
        NodeKind::Other(what) => Carrier::Damaged(DamagedCarrier {
            old: Some(old.to_owned()),
            new: Some(new.to_owned()),
            why: format!(
                "{} is {} — only a regular intent file may be read",
                path.display(),
                what
            ),
        }),
        NodeKind::Regular => match read_intent(&path) {
            Err(why) => Carrier::Damaged(DamagedCarrier {
                old: Some(old.to_owned()),
                new: Some(new.to_owned()),
                why,
            }),
            Ok(None) => Carrier::Missing,
            Ok(Some(intent)) => {
                if intent.old != old || intent.new != new {
                    Carrier::Damaged(DamagedCarrier {
                        old: Some(old.to_owned()),
                        new: Some(new.to_owned()),
                        why: format!(
                            "{} names '{}' → '{}' — refusing a carrier that does not match its filename",
                            path.display(),
                            intent.old,
                            intent.new
                        ),
                    })
                } else if intent.phase == PHASE_COMPLETE {
                    Carrier::Complete(intent)
                } else {
                    Carrier::Pending(intent)
                }
            }
        },
    }
}

/// Read and validate the intent at `path`. `None` when no file is there; any
/// other failure is damage the caller refuses on. Non-regular nodes never
/// open: a FIFO would block, a symlink would read through.
fn read_intent(path: &Path) -> Result<Option<Intent>, String> {
    match classify_node(path) {
        NodeKind::Absent => Ok(None),
        NodeKind::Symlink => Err(format!(
            "{} is a symlink; refusing to follow it",
            path.display()
        )),
        NodeKind::Other(what) => Err(format!(
            "{} is {} — only a regular intent file may be read",
            path.display(),
            what
        )),
        NodeKind::Regular => {
            #[allow(
                clippy::disallowed_methods,
                reason = "a door: the rename's durable-intent read of a classified regular file — the recovery half of the stopped transaction, see clippy.toml"
            )]
            let bytes = match std::fs::read(path) {
                Ok(bytes) => bytes,
                Err(why) if why.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(why) => return Err(format!("could not read {} ({why})", path.display())),
            };
            parse_intent(&bytes)
                .map(Some)
                .map_err(|why| format!("{} is not a valid rename intent ({why})", path.display()))
        }
    }
}

/// Publish an intent atomically (sibling temp + rename, the launch-asset
/// pattern) and prove it by reading it back.
fn publish_intent(root: &Path, intent: &Intent) -> Result<(), String> {
    let path = intent_path(root, &intent.old, &intent.new);
    crate::session_launch::assets::publish_document(&path, &intent_document(intent))
        .map_err(|why| format!("could not publish the rename intent ({why})"))?;
    match read_intent(&path)? {
        Some(back) if back == *intent => Ok(()),
        Some(_) => Err("the rename intent did not read back as written".to_owned()),
        None => Err("the rename intent vanished as it was published".to_owned()),
    }
}

/// The recorded server pair as intent strings.
fn server_pair(server: &ServerId) -> (String, String) {
    match server {
        ServerId::Ambient => ("ambient".to_owned(), String::new()),
        ServerId::Selected(Selector::Socket(path)) => {
            ("socket".to_owned(), path.display().to_string())
        }
        ServerId::Selected(Selector::Name(name)) => ("name".to_owned(), name.clone()),
    }
}

// ---- crash cuts ----------------------------------------------------------
//
// Each armed boundary attests EXACTLY one flushed `rename-crash-boundary:
// <value>` line AFTER its committed facts are verified, then parks for AT
// MOST `CRASH_PARK_SECS` on a monotonic deadline. Expiry attempts the exact
// `rename-crash-timeout: <value>` diagnostic and exits with the existing
// `EXIT_FAILED`/1 even when the sink is unwritable, executing ZERO later
// rename/result steps: no further move, phase update, metadata or asset
// write, rollback, completion append or success output. Process termination
// releases both lifecycle locks. The test child owner waits for the
// attestation and kills through the existing process door; elapsed time never
// selects an early cut.

/// The fixed park ceiling. Pinned by unit test; never a knob or a door.
pub(crate) const CRASH_PARK_SECS: u64 = 60;

/// After `boundary`'s facts are verified, attest and park when armed for it.
///
/// Returns `Some(EXIT_FAILED)` when the operation must stop now (a park that
/// timed out); `None` to continue. A completed result never parks: retrying a
/// durable completion is a success, not a cut.
fn crash_cut(
    boundary: &str,
    armed: Option<&str>,
    err: &mut impl Write,
) -> crate::Result<Option<u8>> {
    crash_cut_until(
        boundary,
        armed,
        err,
        park_deadline(std::time::Instant::now()),
    )
}

/// The park deadline for a cut attested at `now`: the fixed ceiling past it.
/// Pure constructor so tests pin the production 60 seconds without sleeping
/// through them.
fn park_deadline(now: std::time::Instant) -> std::time::Instant {
    now + std::time::Duration::from_secs(CRASH_PARK_SECS)
}

/// [`crash_cut`] against an explicit deadline: the production path passes
/// [`park_deadline`]; tests pass a spent or future deadline to prove
/// terminality and pass-through deterministically.
fn crash_cut_until(
    boundary: &str,
    armed: Option<&str>,
    err: &mut impl Write,
    deadline: std::time::Instant,
) -> crate::Result<Option<u8>> {
    if armed != Some(boundary) {
        return Ok(None);
    }
    writeln!(err, "rename-crash-boundary: {boundary}")?;
    err.flush()?;
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let _ = writeln!(err, "rename-crash-timeout: {boundary}");
    Ok(Some(EXIT_FAILED))
}

/// Everything inside the two locks: the reads, then the moves.
///
/// A live source keeps the existing live behavior (with checked publication);
/// a positively stopped source takes the Slice-A transaction.
#[allow(
    clippy::too_many_lines,
    reason = "the live rename's ordered checks read better in one place than threaded through helpers; the stopped transaction lives in stopped()/recover() below"
)]
fn locked(
    root: &Path,
    old: &str,
    new: &str,
    crash: Option<String>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let sessions = crate::lifecycle::sessions_dir(root);
    let old_dir = sessions.join(old);
    let new_dir = sessions.join(new);
    // Rechecked under the locks: a symlink swapped in after the pre-lock
    // check must not be renamed through.
    for name in [old, new] {
        if is_symlink(&sessions.join(name)) {
            writeln!(
                err,
                "Error: the session path for '{name}' is a symlink; refusing to rename through it."
            )?;
            return Ok(EXIT_FAILED);
        }
    }
    // The server is the OLD session's own recorded one.
    let Some(server) = crate::session_launch::recorded_server_resolved(&old_dir) else {
        writeln!(
            err,
            "Error: session '{old}' {}. Nothing was renamed.",
            crate::session_launch::AMBIGUOUS_SERVER
        )?;
        return Ok(EXIT_FAILED);
    };

    // Addressed by the EXACT id from here on: `-t proj` prefix-matches, so a
    // rename addressed by name can move a live `project` and report success.
    let Some(session_id) = crate::lifecycle::live_id(&server, old) else {
        return stopped(root, old, new, &server, crash.as_deref(), out, err);
    };
    // The crash seam is a stopped-rename instrument: its cuts name stopped
    // facts only. A live source with an armed seam refuses before mutation
    // rather than running a live rename that no cut covers.
    if let Some(value) = crash {
        writeln!(
            err,
            "Error: {CRASH_VAR}='{value}' is armed for a stopped rename, but session '{old}' is live — live crash cuts need a separate ruling. Nothing was renamed."
        )?;
        return Ok(EXIT_FAILED);
    }
    // The look is diagnosed BEFORE mutation: a session whose look cannot be
    // read keeps its wrong-for-a-moment name rather than being redressed in
    // a look it never had.
    let look = crate::session_launch::look_of(&server, old);
    if transport::session_exists(&server, new) {
        writeln!(err, "Error: session '{new}' already exists.")?;
        return Ok(EXIT_FAILED);
    }
    if crate::lifecycle::path_exists(&new_dir) {
        writeln!(
            err,
            "Error: session directory '{}' already exists.",
            new_dir.display()
        )?;
        return Ok(EXIT_FAILED);
    }

    // 1. The tmux session.
    let (renamed, _) = transport::run_tmux_op(&argv(
        &server,
        &Op::RenameSession {
            target: &session_id,
            name: new,
        },
    ));
    if !renamed {
        writeln!(
            err,
            "Error: tmux refused to rename session '{old}'. Nothing was renamed."
        )?;
        return Ok(EXIT_FAILED);
    }

    // 2.
    if crate::lifecycle::dir_exists(&old_dir) && std::fs::rename(&old_dir, &new_dir).is_err() {
        writeln!(
            err,
            "Error: the tmux session was renamed to '{new}' but its state directory could not be moved ({}). Retry 'ae rename {old} {new}' after fixing it.",
            old_dir.display()
        )?;
        return Ok(EXIT_FAILED);
    }

    // 3.
    if crate::lifecycle::path_exists(&new_dir.join(crate::meta::FILE)) {
        if let Err(why) = crate::meta::rewrite(&new_dir, "session", Some(new)) {
            writeln!(
                err,
                "Error: '{new}' was renamed but its meta still says '{old}' ({}). Retry 'ae rename {old} {new}' after fixing it.",
                why.cause()
            )?;
            return Ok(EXIT_FAILED);
        }
        let watchdog_expected = crate::session_launch::watchdog_enabled_for_session(&new_dir);
        let mut monitors =
            crate::session_launch::rebind_monitor_panes(root, &server, new, &new_dir).map(|_| ());
        // Manifest publication is CHECKED: an incompatible workspace.md
        // destination must fail the rename, not print success over it.
        if let Err(why) = publish_manifest(&new_dir, new) {
            writeln!(
                err,
                "Error: '{new}' was renamed but its workspace manifest could not be published ({why}). Retry 'ae rename {old} {new}' after fixing it."
            )?;
            return Ok(EXIT_FAILED);
        }
        // The pre-mutation look, redressed without reseeding verdicts. An
        // unreadable look is diagnosed, never replaced by another look.
        if let Some(look) = look {
            let bytes = crate::meta::read_bytes(&new_dir).unwrap_or_default();
            let value = |key: &str| crate::lifecycle::meta_value(&bytes, key);
            crate::session_launch::redress_status_bar(
                &server,
                new,
                &crate::session_launch::status_paths(
                    &value("mode"),
                    &value("origin"),
                    &value("work_dir"),
                    &crate::doors::home()
                        .unwrap_or_default()
                        .display()
                        .to_string(),
                ),
                &look,
            );
        } else {
            writeln!(
                err,
                "Warning: session '{new}' has no readable look — leaving its status bar as renamed without redressing it (never substituting another look)."
            )?;
        }
        // Success is decided after the renamed session's layout and facts are
        // back in place. The respawn helper proves registration too, but this
        // final look makes the ordering explicit and catches a daemon that
        // disappeared while rename republished the look.
        if monitors.is_ok()
            && watchdog_expected
            && crate::watchdog_lifecycle::await_running(&server, new, &new_dir).is_none()
        {
            monitors = Err("watchdog disappeared after the look was republished".to_owned());
        }
        if let Err(why) = monitors {
            if watchdog_expected {
                writeln!(
                    err,
                    "Error: the session was renamed to '{new}', but its monitor processes were not rebound ({why}). The renamed session has NO verified watchdog; run 'ae watchdog start {new}' after fixing it."
                )?;
            } else {
                writeln!(
                    err,
                    "Error: the session was renamed to '{new}', but its events monitor was not rebound ({why})."
                )?;
            }
            return Ok(EXIT_FAILED);
        }
    }

    writeln!(out, "Renamed '{old}' → '{new}'")?;
    Ok(0)
}

// ---- stopped preflight: reads that must all hold before the intent ------

/// A proved stopped plan: every address, identity and ownership fact the
/// transaction needs, read before the first write.
struct Preflight {
    uuid: String,
    mode: WorkMode,
    origin: String,
    old_work: String,
    new_work: String,
    server_kind: String,
    server_value: String,
    warn_implicit: bool,
}

/// Count exact `key=` rows in raw meta bytes. More than one is damage: meta
/// precedence is unclassified, so a rename must not pick one.
fn count_rows(bytes: &[u8], key: &str) -> usize {
    let mut count = 0;
    for line in bytes.split(|byte| *byte == b'\n') {
        let line = line.strip_suffix(b"\r".as_slice()).unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        if let Some(at) = line.iter().position(|byte| *byte == b'=')
            && &line[..at] == key.as_bytes()
        {
            count += 1;
        }
    }
    count
}

/// The `session_id` row of the meta in `dir`: exactly one canonical claim,
/// or empty. Two UUID rows are unknown identity, never the first row.
fn dir_uuid(dir: &Path) -> String {
    let Ok(bytes) = crate::meta::read_bytes(dir) else {
        return String::new();
    };
    if count_rows(&bytes, "session_id") != 1 {
        return String::new();
    }
    let row = crate::lifecycle::meta_value(&bytes, "session_id");
    if crate::archive::canonical_uuid(&row).is_empty() {
        return String::new();
    }
    crate::archive::canonical_uuid(&row)
}

/// One `git worktree list --porcelain` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WorktreeEntry {
    path: String,
    locked: bool,
}

/// Parse `git worktree list --porcelain`: blank-line-separated entries, a
/// `worktree <path>` opener, a bare `locked` attribute when locked.
fn parse_worktree_porcelain(text: &str) -> Vec<WorktreeEntry> {
    let mut entries = Vec::new();
    let mut current: Option<WorktreeEntry> = None;
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            if let Some(entry) = current.take() {
                entries.push(entry);
            }
            continue;
        }
        if let Some(path) = line.strip_prefix("worktree ") {
            if let Some(entry) = current.take() {
                entries.push(entry);
            }
            current = Some(WorktreeEntry {
                path: unquote_porcelain(path),
                locked: false,
            });
        } else if (line == "locked" || line.starts_with("locked "))
            && let Some(entry) = current.as_mut()
        {
            entry.locked = true;
        }
    }
    if let Some(entry) = current.take() {
        entries.push(entry);
    }
    entries
}

/// Unquote one porcelain path: plain, or C-quoted with the escapes git
/// emits for spaces and quotes. Anything else rides through literally rather
/// than failing the whole listing.
fn unquote_porcelain(path: &str) -> String {
    if path.len() < 2 || !path.starts_with('"') || !path.ends_with('"') {
        return path.to_owned();
    }
    let inner = &path[1..path.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('"') => out.push('"'),
            Some('\\') | None => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

/// The recorded seat's harness, classified from its recorded binary the way
/// launch classifies commands — failing toward unknown, never guessing. (No
/// `ToolKind::` variant is named here by design: production tool branches
/// live in `tool.rs`, pinned by `tests/it/doors.rs`.)
fn seat_tool(seat: &crate::meta::RosterEntry) -> crate::tool::ToolKind {
    let binary = seat.binary.as_deref().unwrap_or("");
    let name = Path::new(binary)
        .file_name()
        .and_then(|stem| stem.to_str())
        .unwrap_or(binary);
    crate::tool::ToolKind::from_binary_name(name)
}

/// The Claude transcript path ae's exact-resume probe reads: `projects/<cwd
/// with '/' as '-'>/<id>.jsonl` under an explicit config home. Replicates
/// `run::resumable`'s `StoreProbe::ProjectTranscript` spelling so the
/// preflight asks the same question a later resume would.
fn transcript_path(home: &Path, cwd: &str, id: &str) -> PathBuf {
    let key: String = cwd
        .chars()
        .map(|ch| if ch == '/' { '-' } else { ch })
        .collect();
    home.join("projects").join(key).join(format!("{id}.jsonl"))
}

/// Pending-request and unreadable-evidence blockers naming `names`.
///
/// The source log contributes every pending request (any record there
/// involves the source by construction); peer logs contribute pending
/// requests whose recorded actor or target session is one of `names`.
/// Unknown or corrupt pending evidence blocks rather than inventing a
/// binding. Readers are the existing `SessionRead` ledger readers.
fn pending_blockers(
    root: &Path,
    names: &[&str],
    state_dir: &Path,
    state_name: &str,
) -> Result<Vec<String>, String> {
    let mut blockers = Vec::new();
    let source = crate::session::SessionRead::open(state_dir).map_err(|why| {
        format!("session '{state_name}' has an unreadable event log ({why}) — refusing to strand requests")
    })?;
    for pending in &source.pending {
        blockers.push(format!(
            "{} {} in '{state_name}'",
            pending.id, pending.action
        ));
    }
    let census = crate::lifecycle::census(root).map_err(|why| {
        format!("cannot enumerate sessions ({why}) — refusing to strand requests")
    })?;
    let sessions = census.unwrap_or_default();
    for peer in sessions {
        if names.contains(&peer.as_str()) {
            continue;
        }
        let peer_dir = crate::lifecycle::sessions_dir(root).join(&peer);
        let Ok(read) = crate::session::SessionRead::open(&peer_dir) else {
            blockers.push(format!(
                "unreadable event log in '{peer}' (repair or end '{peer}' first)"
            ));
            continue;
        };
        for pending in &read.pending {
            let involves = read.events.iter().any(|event| {
                event.reference.as_deref() == Some(pending.id.as_str())
                    && (matches!(event.action.as_str(), "ask" | "review"))
                    && (event
                        .actor_session
                        .value()
                        .is_some_and(|s| names.contains(&s))
                        || event
                            .target_session
                            .value()
                            .is_some_and(|s| names.contains(&s)))
            });
            // A pending id with no matching open record is still evidence, not
            // proof of absence: block on the id rather than guessing.
            if involves
                || !read
                    .events
                    .iter()
                    .any(|event| event.reference.as_deref() == Some(pending.id.as_str()))
            {
                blockers.push(format!(
                    "{} {} in '{peer}' (cross-session)",
                    pending.id, pending.action
                ));
            }
        }
    }
    Ok(blockers)
}

/// No other session may record the old or candidate work path, in either
/// direction. Shared by preflight and recovery: the census is re-taken at
/// each use, never carried across a durable cut.
fn sharing_check(
    root: &Path,
    sessions: &Path,
    old: &str,
    new: &str,
    recorded_work: &str,
    candidate: &Path,
) -> Result<(), String> {
    let census = crate::lifecycle::census(root)
        .map_err(|why| format!("cannot enumerate sessions ({why}) — refusing to share work"))?;
    for peer in census.unwrap_or_default() {
        if peer == old || peer == new {
            continue;
        }
        let peer_dir = sessions.join(&peer);
        let Ok(peer_bytes) = crate::meta::read_bytes(&peer_dir) else {
            continue;
        };
        let peer_work = crate::lifecycle::meta_value(&peer_bytes, "work_dir");
        if peer_work == recorded_work || peer_work == candidate.display().to_string() {
            return Err(format!(
                "session '{peer}' records the same working copy — refusing shared work"
            ));
        }
    }
    Ok(())
}

/// The explicit-home probe over one roster: `true` when an implicit or
/// unclassified home rides along (warn, do not refuse). Shared by preflight
/// and recovery; recovery re-runs it while the state still sits at the old
/// address, where old/new candidate paths are still meaningful.
fn explicit_home_check(
    roster: &[crate::meta::RosterEntry],
    old_work: &str,
    new_work: &str,
) -> Result<bool, String> {
    let mut warn_implicit = false;
    for seat in roster {
        match &seat.config_home {
            crate::meta::RecordedConfigHome::Path(home) => {
                let tool = seat_tool(seat);
                if !tool.explicit_home_is_cwd_keyed() {
                    if !tool.is_known() {
                        warn_implicit = true;
                    }
                    continue;
                }
                let Some(id) = seat.harness_session.as_deref() else {
                    continue;
                };
                if !crate::launch::id_probeable(id) {
                    continue;
                }
                let before = transcript_path(home, old_work, id);
                let after = transcript_path(home, new_work, id);
                if crate::lifecycle::path_exists(&before) && !crate::lifecycle::path_exists(&after)
                {
                    return Err(format!(
                        "seat '{}' ({}) pins an explicit config home ('{}') whose conversation '{id}' ae resumes exactly at '{old_work}' but not at '{new_work}' — renaming would force ae's '--continue' fallback (move the transcript or use an implicit home first)",
                        seat.slot,
                        tool.as_str(),
                        home.display()
                    ));
                }
            }
            crate::meta::RecordedConfigHome::Invalid => {
                return Err(format!(
                    "seat '{}' records an unreadable config_home row — refusing over damaged identity",
                    seat.slot
                ));
            }
            _ => {
                warn_implicit = true;
            }
        }
    }
    Ok(warn_implicit)
}

/// Read the whole stopped preflight. No writes; every refusal names the
/// remedy. `crash` is carried for the local work-move check only.
#[allow(
    clippy::too_many_lines,
    reason = "one ordered admission checklist: splitting the guards across helpers would hide the before-intent order the transaction depends on"
)]
fn preflight(
    root: &Path,
    old: &str,
    new: &str,
    server: &ServerId,
    crash: Option<&str>,
) -> Result<Preflight, String> {
    let sessions = crate::lifecycle::sessions_dir(root);
    let old_dir = sessions.join(old);
    let new_dir = sessions.join(new);
    if !crate::lifecycle::dir_exists(&old_dir) {
        return Err(format!("session '{old}' has no state directory"));
    }
    let bytes = crate::meta::read_bytes(&old_dir)
        .map_err(|why| format!("session '{old}' has no readable meta ({why})"))?;
    if crate::lifecycle::meta_value(&bytes, "session") != old {
        let recorded = crate::lifecycle::meta_value(&bytes, "session");
        return Err(format!(
            "session '{old}' records session '{recorded}' — refusing over mismatched identity (unplaced legacy state needs a manual repair)"
        ));
    }
    for key in ["session", "session_id", "work_dir", "mode", "origin"] {
        if count_rows(&bytes, key) > 1 {
            return Err(format!(
                "session '{old}' records a duplicated '{key}' row — refusing over damaged identity"
            ));
        }
    }
    let uuid = crate::lifecycle::meta_value(&bytes, "session_id");
    if crate::archive::canonical_uuid(&uuid).is_empty() {
        return Err(format!(
            "session '{old}' records no stable session UUID — refusing to mint one implicitly (explicit migration only)"
        ));
    }
    let uuid = crate::archive::canonical_uuid(&uuid);
    let mode = crate::lifecycle::meta_value(&bytes, "mode");
    let Some(mode) = WorkMode::parse(&mode) else {
        return Err(format!(
            "session '{old}' records an unknown or missing mode — refusing an unprovable work identity"
        ));
    };
    let (server_kind, server_value) = server_pair(server);
    // Positive absence on the recorded server: a missing socket with no
    // positive proof is unknown and refuses before mutation.
    match crate::transport::verify_session_absent(server, old) {
        crate::tmux::StopProbe::Absent => {}
        crate::tmux::StopProbe::Present => {
            return Err(format!(
                "session '{old}' answered live on its recorded server after no exact id resolved — refusing over unknown liveness"
            ));
        }
        crate::tmux::StopProbe::Unknown => {
            return Err(format!(
                "cannot prove session '{old}' stopped (its recorded tmux server is unreachable)"
            ));
        }
    }
    if crate::transport::session_exists(server, new) {
        return Err(format!("session '{new}' already exists"));
    }
    // Destinations classify without following links: a dangling symlink is
    // an occupant entry, not an absence, and `rename(2)` would overwrite it.
    match classify_node(&new_dir) {
        NodeKind::Absent => {}
        NodeKind::Symlink => {
            return Err(format!(
                "session path '{}' is a symlink — refusing to rename through it",
                new_dir.display()
            ));
        }
        NodeKind::Regular | NodeKind::Other(_) => {
            return Err(format!(
                "session directory '{}' already exists",
                new_dir.display()
            ));
        }
    }
    let origin = crate::lifecycle::meta_value(&bytes, "origin");
    let recorded_work = crate::lifecycle::meta_value(&bytes, "work_dir");
    let (old_work, new_work) = match mode {
        WorkMode::Local => (recorded_work.clone(), recorded_work.clone()),
        WorkMode::Git | WorkMode::Full => {
            let managed = crate::lifecycle::worktrees_dir(root).join(old);
            if Path::new(&recorded_work) != managed {
                return Err(format!(
                    "session '{old}' records work_dir '{recorded_work}' but a managed '{}' session must own '{}' — refusing an unowned path (manual repair: move it into place or end the session)",
                    mode.as_str(),
                    managed.display()
                ));
            }
            // Teardown-grade root authority: the worktrees root must be a real
            // non-symlink directory, never a link ae would move through.
            let worktrees = crate::lifecycle::worktrees_dir(root);
            if is_symlink(&worktrees) || !crate::lifecycle::dir_exists(&worktrees) {
                return Err(format!(
                    "the configured worktrees root '{}' is not a real directory — refusing",
                    worktrees.display()
                ));
            }
            if !crate::lifecycle::dir_exists(Path::new(&recorded_work)) {
                return Err(format!(
                    "session '{old}' records its working copy at {recorded_work} but it is gone — restore it or end the session (ae end {old})"
                ));
            }
            if mode == WorkMode::Full && is_symlink(Path::new(&recorded_work)) {
                return Err(format!(
                    "session '{old}' records a working copy at {recorded_work} that is a symlink — refusing an unprovable copy"
                ));
            }
            let candidate = worktrees.join(new);
            match classify_node(&candidate) {
                NodeKind::Absent => {}
                NodeKind::Symlink => {
                    return Err(format!(
                        "managed path '{}' is a symlink — refusing to rename through it",
                        candidate.display()
                    ));
                }
                NodeKind::Regular | NodeKind::Other(_) => {
                    return Err(format!(
                        "managed path '{}' already exists — refusing to overwrite it",
                        candidate.display()
                    ));
                }
            }
            // No sharing by another session, in either direction.
            sharing_check(root, &sessions, old, new, &recorded_work, &candidate)?;
            if mode == WorkMode::Git {
                prove_git_worktree(root, old, new, &recorded_work, &candidate)?;
            }
            (recorded_work, candidate.display().to_string())
        }
    };
    if origin.is_empty() && mode != WorkMode::Local {
        return Err(format!(
            "session '{old}' records no origin — refusing a git/full move without one"
        ));
    }
    // Conservative pending-request refusal (pre-B: every pending record is
    // legacy identity). Names affected request IDs and the remedy.
    let blockers = pending_blockers(root, &[old], &old_dir, old)?;
    if !blockers.is_empty() {
        return Err(format!(
            "session '{old}' has {} pending request(s) a rename would strand ({}) — close them first (reply from the target seat, or retire the holding seat), then retry",
            blockers.len(),
            blockers.join(", ")
        ));
    }
    // A stopped rename performs no resume, but it must not newly break ae's
    // own exact-resume probe; the tool-specific check is shared so recovery
    // re-proves the same rule.
    let parsed = crate::meta::Meta::parse(&String::from_utf8_lossy(&bytes));
    let warn_implicit = explicit_home_check(parsed.roster(), &old_work, &new_work)?;
    // The work-move cut needs a real managed move: a local request arms
    // nothing committable, so it refuses before side effects.
    if mode == WorkMode::Local && crash == Some("after-work-move") {
        return Err(format!(
            "{CRASH_VAR}='after-work-move' names a work move, but session '{old}' is mode=local and performs none"
        ));
    }
    Ok(Preflight {
        uuid,
        mode,
        origin,
        old_work,
        new_work,
        server_kind,
        server_value,
        warn_implicit: warn_implicit && mode != WorkMode::Local,
    })
}

/// Resolve symlinked ancestors, or `None` when the path is gone. `canonicalize`
/// is not one of the inventoried entry points; the inventoried readers of
/// this file stay the same with or without it.
fn canonical(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

/// `(device, inode)` of `path`, or `(0, 0)` when it cannot be read. The
/// managed-work identity witness: rename preserves both, so a retry re-proves
/// the same directory sits at the new address rather than a replacement.
fn dir_id(path: &Path) -> (u64, u64) {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the rename's work-identity fingerprint reads the managed directory it is about to move — see clippy.toml"
    )]
    let probe = std::fs::metadata(path);
    probe.map_or((0, 0), |meta| (meta.dev(), meta.ino()))
}

/// The git administrative directory for a managed work path: origin's
/// `.git/worktrees/<leaf>`, which `git worktree move` preserves (name and
/// identity) while repointing its `gitdir` file.
fn admin_dir(origin: &str, work: &str) -> PathBuf {
    let leaf = Path::new(work)
        .file_name()
        .map(std::ffi::OsStr::to_owned)
        .unwrap_or_default();
    Path::new(origin).join(".git/worktrees").join(leaf)
}

/// Whether `porcelain` registers the `name` child of `parent`: a literal hit,
/// or the same place after resolving symlinked ancestors on both sides. Git
/// canonicalizes worktree spellings (a symlinked TMPDIR prints resolved)
/// while the meta records the launch spelling, so text equality alone refuses
/// healthy moves.
fn registered_spot(entries: &[WorktreeEntry], parent: &Path, name: &str) -> Vec<usize> {
    let literal = parent.join(name).display().to_string();
    let placed = canonical(parent).map(|root| root.join(name));
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            entry.path == literal
                || placed
                    .as_ref()
                    .is_some_and(|spot| canonical(Path::new(&entry.path)).as_ref() == Some(spot))
        })
        .map(|(index, _)| index)
        .collect()
}

/// Prove the git worktree facts a move needs: the origin answers, the old
/// path is registered exactly once and unlocked, the new path is not, and no
/// submodule marker forces a manual move.
fn prove_git_worktree(
    root: &Path,
    old: &str,
    new: &str,
    old_work: &str,
    candidate: &Path,
) -> Result<(), String> {
    let sessions = crate::lifecycle::sessions_dir(root);
    let old_dir = sessions.join(old);
    let bytes = crate::meta::read_bytes(&old_dir)
        .map_err(|why| format!("session '{old}' has no readable meta ({why})"))?;
    let origin = crate::lifecycle::meta_value(&bytes, "origin");
    git_registration_check(
        &origin,
        &crate::lifecycle::worktrees_dir(root),
        old,
        new,
        old_work,
        candidate,
    )
}

/// The git registration re-proof for recovery, phase-aware: before the work
/// moves it demands the fresh shape (old registered once and unlocked, new
/// absent); after the move it demands the moved shape (new registered once,
/// old absent). A moved worktree re-locked or unregistered under a parked
/// transaction refuses here rather than failing mid-step.
fn git_recovery_check(root: &Path, intent: &Intent) -> Result<(), String> {
    let worktrees = crate::lifecycle::worktrees_dir(root);
    let listed = crate::git::worktree_list(intent.origin.as_bytes()).ok_or_else(|| {
        format!(
            "origin '{}' did not answer 'git worktree list --porcelain' — refusing",
            intent.origin
        )
    })?;
    let entries = parse_worktree_porcelain(&listed);
    let olds = registered_spot(&entries, &worktrees, &intent.old);
    let news = registered_spot(&entries, &worktrees, &intent.new);
    let old_gone = matches!(classify_node(Path::new(&intent.old_work)), NodeKind::Absent);
    if old_gone {
        if news.len() != 1 || !olds.is_empty() {
            return Err(format!(
                "'{}' is not coherently registered at '{}' after the recorded move — refusing (restore the registration or end the session)",
                intent.new_work, intent.origin
            ));
        }
        return Ok(());
    }
    git_registration_check(
        &intent.origin,
        &worktrees,
        &intent.old,
        &intent.new,
        &intent.old_work,
        Path::new(&intent.new_work),
    )
}

/// The registration facts a git move needs, re-taken at each use: the origin
/// answers, the old path is registered exactly once and unlocked, the new
/// path is not, and no submodule marker forces a manual move.
fn git_registration_check(
    origin: &str,
    worktrees: &Path,
    old: &str,
    new: &str,
    old_work: &str,
    candidate: &Path,
) -> Result<(), String> {
    let listed = crate::git::worktree_list(origin.as_bytes()).ok_or_else(|| {
        format!("origin '{origin}' did not answer 'git worktree list --porcelain' — refusing")
    })?;
    let entries = parse_worktree_porcelain(&listed);
    let olds = registered_spot(&entries, worktrees, old);
    if olds.is_empty() {
        return Err(format!(
            "'{old_work}' is not a registered worktree of '{origin}' — refusing (restore the registration or end the session)"
        ));
    }
    if olds.len() > 1 {
        return Err(format!(
            "'{old_work}' is registered {} times in '{origin}' — refusing over damaged registration",
            olds.len()
        ));
    }
    if !registered_spot(&entries, worktrees, new).is_empty() {
        return Err(format!(
            "'{}' is already a registered worktree of '{origin}' — refusing without force",
            candidate.display()
        ));
    }
    if entries[olds[0]].locked {
        return Err(format!(
            "'{old_work}' is a locked worktree — refusing (unlock it with 'git worktree unlock' first)"
        ));
    }
    if crate::lifecycle::path_exists(Path::new(old_work).join(".gitmodules").as_path()) {
        return Err(format!(
            "'{old_work}' carries submodules — refusing an automatic move (move it by hand, then end and relaunch)"
        ));
    }
    Ok(())
}

/// The renamed session's workspace manifest, published and CHECKED: the
/// caller fails the rename on the returned error rather than printing success
/// over an unpublished manifest. (The status bar is the caller's job: the
/// live path redresses the pre-mutation look, and a stopped session has no
/// live bar to redress.)
fn publish_manifest(dir: &Path, name: &str) -> Result<(), String> {
    let manifest = expected_manifest(dir, name);
    crate::session_launch::assets::publish_document(&dir.join("workspace.md"), &manifest).map_err(
        |why| {
            format!(
                "could not publish {} ({why})",
                dir.join("workspace.md").display()
            )
        },
    )
}

// ---- stopped steps: verify-or-act, so a retry converges ------------------

/// Whether the managed work already lives at its new address with coherent
/// registration. Local mode has no move: always true.
fn work_moved(root: &Path, intent: &Intent) -> bool {
    if intent.mode == WorkMode::Local {
        return true;
    }
    // The old address must be entirely gone — a symlink planted there is an
    // occupant entry, not absence.
    if !matches!(classify_node(Path::new(&intent.old_work)), NodeKind::Absent) {
        return false;
    }
    if !crate::lifecycle::dir_exists(Path::new(&intent.new_work))
        || is_symlink(Path::new(&intent.new_work))
    {
        return false;
    }
    // The witness: the same directory the intent fingerprinted, not a
    // replacement that happens to sit at the new spelling.
    if (intent.work_dev, intent.work_ino) == (0, 0)
        || dir_id(Path::new(&intent.new_work)) != (intent.work_dev, intent.work_ino)
    {
        return false;
    }
    if intent.mode != WorkMode::Git {
        return true;
    }
    // The administrative identity likewise: `git worktree move` preserves
    // the admin directory while repointing its `gitdir` file, so a recreated
    // worktree at the same spelling fails here.
    if (intent.admin_dev, intent.admin_ino) == (0, 0)
        || dir_id(&admin_dir(&intent.origin, &intent.old_work))
            != (intent.admin_dev, intent.admin_ino)
    {
        return false;
    }
    let Some(listed) = crate::git::worktree_list(intent.origin.as_bytes()) else {
        return false;
    };
    let entries = parse_worktree_porcelain(&listed);
    let worktrees = crate::lifecycle::worktrees_dir(root);
    registered_spot(&entries, &worktrees, &intent.new).len() == 1
        && registered_spot(&entries, &worktrees, &intent.old).is_empty()
}

/// Perform the managed work move, then verify it. Same-filesystem rename for
/// a full copy (a cross-device move refuses rather than copying); the typed
/// `git worktree move` for git, which refuses main/locked/submodule cases
/// before moving anything.
fn do_work_move(root: &Path, intent: &Intent) -> Result<(), String> {
    if intent.mode == WorkMode::Local {
        return Err("no work move exists in local mode".to_owned());
    }
    if is_symlink(Path::new(&intent.old_work)) {
        return Err(format!(
            "the working copy at '{}' became a symlink mid-transaction — refusing",
            intent.old_work
        ));
    }
    // Re-prove the fingerprinted identity BEFORE either move: the carrier's
    // witness must describe the directory standing at the old address right
    // now — not a same-spelling replacement, and not a zero fingerprint no
    // publish step emits. A move first and a check afterward would strand
    // real work on a forged or stale carrier.
    if dir_id(Path::new(&intent.old_work)) != (intent.work_dev, intent.work_ino) {
        return Err(format!(
            "the working copy at '{}' does not match the recorded identity — refusing to move an unproved directory",
            intent.old_work
        ));
    }
    if intent.mode == WorkMode::Git
        && dir_id(&admin_dir(&intent.origin, &intent.old_work))
            != (intent.admin_dev, intent.admin_ino)
    {
        return Err(format!(
            "the git administrative directory for '{}' does not match the recorded identity — refusing to move an unproved worktree",
            intent.old_work
        ));
    }
    // The fresh preflight proves the destination absent once, but a retry
    // converges without it: after a crash at `after-intent` that proof is
    // stale, and `rename(2)` onto a planted link or directory would delete
    // an unowned entry. Require lstat absence immediately pre-move, naming
    // every other node.
    match classify_node(Path::new(&intent.new_work)) {
        NodeKind::Absent => {}
        NodeKind::Symlink => {
            return Err(format!(
                "the managed path for '{}' became a symlink mid-transaction — refusing",
                intent.new
            ));
        }
        NodeKind::Regular | NodeKind::Other(_) => {
            return Err(format!(
                "managed path '{}' appeared mid-transaction — refusing to overwrite it",
                intent.new_work
            ));
        }
    }
    if intent.mode == WorkMode::Git {
        if !crate::git::worktree_move(
            intent.origin.as_bytes(),
            intent.old_work.as_bytes(),
            intent.new_work.as_bytes(),
        ) {
            return Err(format!(
                "git refused 'worktree move {} {}' in '{}' — fix the registration and retry",
                intent.old_work, intent.new_work, intent.origin
            ));
        }
    } else {
        match std::fs::rename(&intent.old_work, &intent.new_work) {
            Ok(()) => {}
            Err(why) if why.kind() == std::io::ErrorKind::CrossesDevices => {
                return Err(format!(
                    "cannot move '{}' to '{}' across devices ({why}) — refusing rather than copying",
                    intent.old_work, intent.new_work
                ));
            }
            Err(why) => {
                return Err(format!(
                    "could not move '{}' to '{}' ({why})",
                    intent.old_work, intent.new_work
                ));
            }
        }
    }
    if work_moved(root, intent) {
        Ok(())
    } else {
        Err(format!(
            "moved '{}' to '{}' but the result does not verify (old address lingers or registration incoherent)",
            intent.old_work, intent.new_work
        ))
    }
}

/// Whether the state directory already lives at its new address with the
/// intent's UUID. The old address must be entirely gone (lstat: a planted
/// link is an occupant, not absence).
fn state_moved(root: &Path, intent: &Intent) -> bool {
    let sessions = crate::lifecycle::sessions_dir(root);
    matches!(classify_node(&sessions.join(&intent.old)), NodeKind::Absent)
        && dir_uuid(&sessions.join(&intent.new)) == intent.uuid
}

/// Move the state directory, then verify it.
fn do_state_move(root: &Path, intent: &Intent) -> Result<(), String> {
    let sessions = crate::lifecycle::sessions_dir(root);
    let old_dir = sessions.join(&intent.old);
    let new_dir = sessions.join(&intent.new);
    if is_symlink(&old_dir) {
        return Err(format!(
            "the session path for '{}' became a symlink mid-transaction — refusing",
            intent.old
        ));
    }
    // The destination must be entirely absent (lstat, never followed): the
    // fresh preflight proves this once, but a retry converges without it, and
    // `rename(2)` onto a planted link would overwrite an unowned entry.
    match classify_node(&new_dir) {
        NodeKind::Absent => {}
        NodeKind::Symlink => {
            return Err(format!(
                "the session path for '{}' became a symlink mid-transaction — refusing",
                intent.new
            ));
        }
        NodeKind::Regular | NodeKind::Other(_) => {
            return Err(format!(
                "session directory '{}' appeared mid-transaction — refusing to overwrite it",
                new_dir.display()
            ));
        }
    }
    std::fs::rename(&old_dir, &new_dir).map_err(|why| {
        format!(
            "could not move the state directory '{}' to '{}' ({why})",
            old_dir.display(),
            new_dir.display()
        )
    })?;
    if state_moved(root, intent) {
        Ok(())
    } else {
        Err("moved the state directory but the result does not verify".to_owned())
    }
}

/// Whether the new meta coherently names the new session and managed work
/// path with the stable UUID and all other rows retained.
fn meta_coherent(root: &Path, intent: &Intent) -> bool {
    let dir = crate::lifecycle::sessions_dir(root).join(&intent.new);
    let Ok(bytes) = crate::meta::read_bytes(&dir) else {
        return false;
    };
    if crate::lifecycle::meta_value(&bytes, "session") != intent.new {
        return false;
    }
    if crate::lifecycle::meta_value(&bytes, "session_id") != intent.uuid {
        return false;
    }
    if intent.mode == WorkMode::Local {
        return true;
    }
    crate::lifecycle::meta_value(&bytes, "work_dir") == intent.new_work
}

/// Publish the coherent meta as ONE atomic replacement: the new `session`
/// and (for managed modes) the new `work_dir`, every other byte preserved.
/// Two separate row rewrites could strand a crash between them; one replace
/// cannot.
fn do_meta(root: &Path, intent: &Intent) -> Result<(), String> {
    let dir = crate::lifecycle::sessions_dir(root).join(&intent.new);
    let bytes = crate::meta::read_bytes(&dir)
        .map_err(|why| format!("could not read the new meta ({why})"))?;
    let mut lines: Vec<Vec<u8>> = bytes.split(|b| *b == b'\n').map(<[u8]>::to_vec).collect();
    // The file ends in a newline, so the split leaves one empty final
    // element; anything else empty is a blank line, preserved as-is.
    let mut sessions = 0;
    let mut works = 0;
    for line in &mut lines {
        let key = line.iter().position(|b| *b == b'=').map(|at| &line[..at]);
        if key == Some(b"session".as_slice()) {
            sessions += 1;
            if sessions == 1 {
                let mut next = b"session=".to_vec();
                next.extend_from_slice(intent.new.as_bytes());
                *line = next;
            }
        } else if key == Some(b"work_dir".as_slice()) {
            works += 1;
            if works == 1 && intent.mode != WorkMode::Local {
                let mut next = b"work_dir=".to_vec();
                next.extend_from_slice(intent.new_work.as_bytes());
                *line = next;
            }
        }
    }
    if sessions != 1 {
        return Err(format!(
            "the new meta carries {sessions} 'session' rows — refusing over damaged identity"
        ));
    }
    if intent.mode != WorkMode::Local && works != 1 {
        return Err(format!(
            "the new meta carries {works} 'work_dir' rows — refusing over damaged identity"
        ));
    }
    // Byte-exact join: every row but the two renamed ones survives byte for
    // byte. A non-UTF-8 meta cannot cross `replace`'s `&str`, so it refuses
    // loudly here rather than lossily rewriting history.
    let mut next: Vec<u8> = Vec::with_capacity(bytes.len() + 64);
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            next.push(b'\n');
        }
        next.extend_from_slice(line);
    }
    let next = std::str::from_utf8(&next)
        .map_err(|_| "the new meta is not UTF-8 — refusing a lossy rewrite".to_owned())?;
    crate::meta::replace(&dir, next)
        .map_err(|why| format!("could not publish the coherent meta ({})", why.cause()))?;
    if meta_coherent(root, intent) {
        Ok(())
    } else {
        Err("published the coherent meta but it does not read back".to_owned())
    }
}

/// The manifest facts, read once from the new meta for both publication and
/// verification, so the check compares against canonical expected bytes
/// rather than fragments.
struct ManifestInputs {
    origin: String,
    work_dir: String,
    mode: String,
    main_pane: String,
    config_files: Vec<PathBuf>,
}

/// Read the manifest facts from the meta in `dir`.
fn manifest_inputs(dir: &Path) -> ManifestInputs {
    let bytes = crate::meta::read_bytes(dir).unwrap_or_default();
    let value = |key: &str| crate::lifecycle::meta_value(&bytes, key);
    let origin = or_dot(value("origin"));
    let work_dir = or_dot(value("work_dir"));
    let mode = value("mode");
    let mode = if mode.is_empty() {
        "local".to_owned()
    } else {
        mode
    };
    let main_pane = value("main_pane");
    let main_pane = if main_pane.is_empty() {
        "%0".to_owned()
    } else {
        main_pane
    };
    let mut config_files: Vec<PathBuf> = Vec::new();
    let recorded = value("config");
    if !recorded.is_empty() {
        config_files.push(PathBuf::from(recorded));
    }
    if let Some(local) = crate::config::local_overlay(dir, &origin) {
        config_files.push(local);
    }
    ManifestInputs {
        origin,
        work_dir,
        mode,
        main_pane,
        config_files,
    }
}

/// Render the canonical expected manifest for the renamed session.
fn expected_manifest(dir: &Path, name: &str) -> String {
    let inputs = manifest_inputs(dir);
    crate::render::manifest_document(
        dir,
        name,
        &inputs.work_dir,
        &inputs.origin,
        &inputs.mode,
        &inputs.main_pane,
        &inputs.config_files,
    )
}

/// Whether the checked assets are all current: the manifest is byte-exact
/// the canonical render, every helper link resolves to the session core, and
/// required provider context is byte-exact with no strays.
fn assets_ready(root: &Path, intent: &Intent) -> bool {
    let dir = crate::lifecycle::sessions_dir(root).join(&intent.new);
    if !manifest_ready(&dir, intent) {
        return false;
    }
    if !helpers_ready(&dir) {
        return false;
    }
    opencode_current(&dir, intent)
}

/// Whether `workspace.md` is byte-exact the canonical manifest: no stale
/// line, fragment, or address can pass, in either prefix direction.
fn manifest_ready(dir: &Path, intent: &Intent) -> bool {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the rename's asset check reads back what its own checked publication wrote — see clippy.toml"
    )]
    let current = std::fs::read(dir.join("workspace.md")).unwrap_or_default();
    current == expected_manifest(dir, &intent.new).as_bytes()
}

/// Whether every helper link resolves to the session's own core: a symlink
/// that dangles, or was repointed at another binary, is not a checked asset.
/// The expected target is the recorded `ae_core` pin while it exists, else
/// the running core that a repair would link — check and repair agree, so a
/// repaired directory always verifies. With neither provable there is no
/// expected target at all, and readiness fails loudly instead of blessing
/// arbitrary links.
fn helpers_ready(dir: &Path) -> bool {
    helpers_ready_with(dir, helper_core(dir))
}

/// [`helpers_ready`] against an explicit expected core, so the unproved-core
/// rule pins deterministically.
fn helpers_ready_with(dir: &Path, expected: Option<PathBuf>) -> bool {
    let Some(pinned) = expected else {
        return false;
    };
    crate::shim::HELPERS.iter().all(|helper| {
        let link = dir.join(helper.name);
        if !is_symlink(&link) {
            return false;
        }
        // `read_link` is not an inventoried entry point: it classifies the
        // link the rename published without following it.
        let target = std::fs::read_link(&link).unwrap_or_default();
        target == pinned && crate::lifecycle::path_exists(&target)
    })
}

/// The core a correct helper link addresses — and the one a repair links:
/// the recorded pin while it exists, else the running core.
fn helper_core(dir: &Path) -> Option<PathBuf> {
    if let Ok(bytes) = crate::meta::read_bytes(dir) {
        let recorded = crate::lifecycle::meta_value(&bytes, "ae_core");
        if !recorded.is_empty() && crate::lifecycle::path_exists(Path::new(&recorded)) {
            return Some(PathBuf::from(recorded));
        }
    }
    crate::shape::resolved_exe()
}

/// One required provider-context pair with its canonical bytes, plus stray
/// pairs no roster seat requires (removed, never verified).
struct OpencodePlan {
    required: Vec<OpencodeFile>,
    strays: Vec<(PathBuf, PathBuf)>,
}

/// A context markdown plus its pointer file, byte-exact.
struct OpencodeFile {
    slot: String,
    md: PathBuf,
    json: PathBuf,
    md_bytes: Vec<u8>,
    json_bytes: Vec<u8>,
}

/// Plan the generated provider context from the seat tools — required pairs
/// from seats whose tool takes the config-file channel, never from whatever
/// files happen to be present. A resume regenerates these through the
/// production launcher; the rename republishes them so no stale absolute
/// pointer to the old address survives a stopped session.
fn opencode_plan(dir: &Path, intent: &Intent) -> Result<OpencodePlan, String> {
    let bytes = crate::meta::read_bytes(dir)
        .map_err(|why| format!("could not read the new meta ({why})"))?;
    let value = |key: &str| crate::lifecycle::meta_value(&bytes, key);
    let mut config_files: Vec<PathBuf> = Vec::new();
    let recorded = value("config");
    if !recorded.is_empty() {
        config_files.push(PathBuf::from(recorded));
    }
    if let Some(local) = crate::config::local_overlay(dir, &value("origin")) {
        config_files.push(local);
    }
    let meta = crate::meta::Meta::parse(&String::from_utf8_lossy(&bytes));
    let mut required = Vec::new();
    let mut wanted: Vec<String> = Vec::new();
    for seat in meta.roster() {
        if !seat_tool(seat).needs_generated_context() {
            continue;
        }
        let safe = crate::launch::safe_slot(&seat.slot);
        wanted.push(safe.clone());
        let ctx = crate::render::context_document(
            dir,
            &intent.new,
            &intent.new_work,
            &seat.slot,
            &config_files,
        );
        let md = dir.join(format!("opencode.{safe}.md"));
        let json = dir.join(format!("opencode.{safe}.json"));
        let pointer = crate::json::Value::Str(md.display().to_string()).render();
        required.push(OpencodeFile {
            slot: seat.slot.clone(),
            md,
            json,
            md_bytes: format!("{ctx}\n").into_bytes(),
            json_bytes: format!("{{\"instructions\":[{pointer}]}}\n").into_bytes(),
        });
    }
    // Strays: generated pairs no roster seat requires. Their pointers name a
    // past address; leaving them would leave stale absolute paths behind.
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the rename's stray-asset scan enumerates only its generated opencode.* pair names — see clippy.toml"
    )]
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(why) => {
            return Err(format!("could not enumerate {} ({why})", dir.display()));
        }
    };
    let mut stray_stems: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        for suffix in [".md", ".json"] {
            if let Some(stem) = name
                .strip_prefix("opencode.")
                .and_then(|rest| rest.strip_suffix(suffix))
            {
                let stem = stem.to_owned();
                if !wanted.contains(&stem) && !stray_stems.contains(&stem) {
                    stray_stems.push(stem);
                }
            }
        }
    }
    let strays = stray_stems
        .into_iter()
        .map(|stem| {
            (
                dir.join(format!("opencode.{stem}.md")),
                dir.join(format!("opencode.{stem}.json")),
            )
        })
        .collect();
    Ok(OpencodePlan { required, strays })
}

/// Whether generated provider context needs no work: every required pair is
/// byte-exact the canonical render, and no stray pair survives. A deleted
/// required pair therefore forces republication — absence is never ready.
fn opencode_current(dir: &Path, intent: &Intent) -> bool {
    let Ok(plan) = opencode_plan(dir, intent) else {
        return false;
    };
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the rename's asset check reads back what its own checked publication wrote — see clippy.toml"
    )]
    let read = |path: &Path| std::fs::read(path).unwrap_or_default();
    if plan
        .required
        .iter()
        .any(|file| read(&file.md) != file.md_bytes || read(&file.json) != file.json_bytes)
    {
        return false;
    }
    if plan
        .strays
        .iter()
        .any(|(md, json)| crate::lifecycle::path_exists(md) || crate::lifecycle::path_exists(json))
    {
        return false;
    }
    true
}

/// Republish the checked assets: manifest, helper links (repaired to the
/// running core when one is missing), and generated provider context for
/// slots the launch generated any for. Then verify all three.
fn do_assets(root: &Path, intent: &Intent) -> Result<(), String> {
    let dir = crate::lifecycle::sessions_dir(root).join(&intent.new);
    publish_manifest(&dir, &intent.new)?;
    if !helpers_ready(&dir) {
        let core = helper_core(&dir).ok_or_else(|| {
            "a helper link is wrong and no core can be proven to repair it".to_owned()
        })?;
        crate::session_launch::assets::write_helpers(&dir, &core)
            .map_err(|why| format!("could not repair helper links ({why})"))?;
    }
    // Generated provider context from the seat-tool plan: required pairs
    // published byte-exact, strays removed, then the whole asset set
    // verified — never trusted from whatever files happen to be present.
    let plan = opencode_plan(&dir, intent)?;
    for file in &plan.required {
        crate::session_launch::assets::publish_document(
            &file.md,
            &String::from_utf8_lossy(&file.md_bytes),
        )
        .map_err(|why| {
            format!(
                "could not republish generated context for '{}' ({why})",
                file.slot
            )
        })?;
        crate::session_launch::assets::publish_document(
            &file.json,
            &String::from_utf8_lossy(&file.json_bytes),
        )
        .map_err(|why| {
            format!(
                "could not republish generated context pointer for '{}' ({why})",
                file.slot
            )
        })?;
    }
    for (md, json) in &plan.strays {
        for stray in [md, json] {
            match std::fs::remove_file(stray) {
                Ok(()) => {}
                Err(why) if why.kind() == std::io::ErrorKind::NotFound => {}
                Err(why) => {
                    return Err(format!(
                        "could not remove stray generated context '{}' ({why})",
                        stray.display()
                    ));
                }
            }
        }
    }
    if assets_ready(root, intent) {
        Ok(())
    } else {
        Err("published the checked assets but they do not verify".to_owned())
    }
}

/// Record one phase and prove it read back.
fn advance(root: &Path, intent: &mut Intent, phase: &str) -> Result<(), String> {
    phase.clone_into(&mut intent.phase);
    publish_intent(root, intent)
}

/// Publish the durable result and prove it: the intent at `complete` IS the
/// result a retry reads instead of duplicating.
fn publish_result(root: &Path, intent: &mut Intent) -> Result<(), String> {
    advance(root, intent, PHASE_COMPLETE)
}

// ---- stopped orchestration: fresh transaction and proved retry ----------

/// A non-complete intent that names `old` or `new` without being this pair:
/// another transaction is pending over one of our names.
/// Every carrier file under the sessions root (live carriers only; rotated
/// `.intent.complete` history is never consulted).
fn carrier_files(root: &Path) -> Vec<PathBuf> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the rename's intent scan — recovery must see a peer transaction before starting its own, see clippy.toml"
    )]
    let Ok(entries) = std::fs::read_dir(crate::lifecycle::sessions_dir(root)) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| !matches!(carrier_addr(path), CarrierAddr::Ignored))
        .collect();
    files.sort();
    files
}

/// A non-complete intent that names `old` or `new` without being this pair:
/// another transaction is pending over one of our names. Unrelated literal
/// pairs filter by validated filename BEFORE any read, so a malformed C→D
/// carrier never blocks a fresh A→B; digest carriers (absurd-length names
/// only) must be read to attribute, and unattributable damage refuses
/// loudly. The exact pair is owned by `stopped`, never a conflict.
fn conflicting_intent(root: &Path, old: &str, new: &str) -> Result<Option<Intent>, String> {
    for path in carrier_files(root) {
        let relevant = match carrier_addr(&path) {
            CarrierAddr::Ignored => false,
            CarrierAddr::Literal { old: o, new: n } => {
                (o != old || n != new) && (o == old || o == new || n == old || n == new)
            }
            CarrierAddr::Digest => true,
        };
        if !relevant {
            continue;
        }
        let Some(classified) = classify_file(&path) else {
            continue;
        };
        let scope = match (&classified.old, &classified.new) {
            (Some(old), Some(new)) => format!("for '{old}' → '{new}'"),
            _ => "that names no attributable pair".to_owned(),
        };
        match classified.result {
            Err(why) => {
                // Attributed damage blocks only its own endpoints; genuinely
                // unattributable damage stays global. A foreign-stem A→B
                // carrier must not deny an unrelated C→D rename.
                let overlaps = match (&classified.old, &classified.new) {
                    (Some(o), Some(n)) => o == old || o == new || n == old || n == new,
                    _ => true,
                };
                if overlaps {
                    return Err(format!(
                        "a damaged rename carrier {scope} is pending ({why})"
                    ));
                }
            }
            Ok(intent) => {
                if intent.phase == PHASE_COMPLETE {
                    continue;
                }
                if intent.old == old || intent.old == new || intent.new == old || intent.new == new
                {
                    return Ok(Some(intent));
                }
            }
        }
    }
    Ok(None)
}

/// The retry command that converges one transaction forward.
fn retry_command(intent: &Intent) -> String {
    format!("ae rename {} {}", intent.old, intent.new)
}

/// How a transaction driver ends: clean completion, a checked failure with
/// its retry text, or a terminal crash-cut exit. Crash exits carry no message
/// — the boundary and timeout diagnostics are already on the stream, and no
/// later reporter may speak after the cut.
#[derive(Debug)]
pub(crate) enum TxnError {
    Fail(String),
    Crash(u8),
}

impl From<String> for TxnError {
    fn from(why: String) -> Self {
        Self::Fail(why)
    }
}

/// The stopped rename: preflight, durable intent, settled moves, checked
/// assets, durable result. Every failure names committed paths, remaining
/// work and the proved retry; nothing after the intent is left to a manual
/// rollback.
fn stopped(
    root: &Path,
    old: &str,
    new: &str,
    server: &ServerId,
    crash: Option<&str>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    // A recorded carrier for this exact pair decides first: recover a
    // pending transaction, converge a durable result, refuse damage — never
    // a new ordinary rename over any of them.
    match read_carrier(root, old, new) {
        Carrier::Pending(intent) => {
            return recover(root, old, new, &intent, crash, out, err);
        }
        Carrier::Complete(intent) => {
            return converge_completed(root, old, new, &intent, server, crash, out, err);
        }
        Carrier::Damaged(damaged) => {
            writeln!(
                err,
                "Error: a damaged rename carrier for '{old}' → '{new}' is pending ({}) — repair or remove it by hand. Nothing was renamed.",
                damaged.why
            )?;
            return Ok(EXIT_FAILED);
        }
        Carrier::Missing => {}
    }
    if let Some(other) = match conflicting_intent(root, old, new) {
        Ok(found) => found,
        Err(why) => {
            writeln!(err, "Error: {why}. Nothing was renamed.")?;
            return Ok(EXIT_FAILED);
        }
    } {
        writeln!(
            err,
            "Error: rename '{}' → '{}' is already in progress at phase '{}' — retry '{}' first. Nothing was renamed.",
            other.old,
            other.new,
            other.phase,
            retry_command(&other)
        )?;
        return Ok(EXIT_FAILED);
    }
    let plan = match preflight(root, old, new, server, crash) {
        Ok(plan) => plan,
        Err(why) => {
            writeln!(err, "Error: {why}. Nothing was renamed.")?;
            return Ok(EXIT_FAILED);
        }
    };
    stopped_fresh(root, old, new, plan, crash, out, err)
}

/// A completed carrier that no longer converges idempotently: the names were
/// reused by a fresh session. Re-prove the whole fresh preflight first; only
/// then rotate the old result aside (preserved byte-for-byte, consulted by
/// nothing) and start the new transaction. A failed preflight keeps the old
/// result untouched.
#[allow(
    clippy::too_many_arguments,
    reason = "one ordered handoff: root, the locked pair, the carrier, the seam, and both streams travel together through every rename entry"
)]
fn converge_completed(
    root: &Path,
    old: &str,
    new: &str,
    intent: &Intent,
    server: &ServerId,
    crash: Option<&str>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let sessions = crate::lifecycle::sessions_dir(root);
    if !crate::lifecycle::path_exists(&sessions.join(old))
        && dir_uuid(&sessions.join(new)) == intent.uuid
    {
        if let Err(why) = verify_completion(root, intent) {
            writeln!(
                err,
                "Error: the transaction completed for '{old}' → '{new}' but {why} — refusing over drifted completion (manual repair only). Nothing was renamed."
            )?;
            return Ok(EXIT_FAILED);
        }
        // Retrying a completion is a success, never a cut: no attestation,
        // no park, no duplicate.
        print_stopped_success(intent, false, out, err)?;
        return Ok(0);
    }
    // Not the recorded completion: prove a fresh transaction first.
    if let Some(other) = match conflicting_intent(root, old, new) {
        Ok(found) => found,
        Err(why) => {
            writeln!(err, "Error: {why}. Nothing was renamed.")?;
            return Ok(EXIT_FAILED);
        }
    } {
        writeln!(
            err,
            "Error: rename '{}' → '{}' is already in progress at phase '{}' — retry '{}' first. Nothing was renamed.",
            other.old,
            other.new,
            other.phase,
            retry_command(&other)
        )?;
        return Ok(EXIT_FAILED);
    }
    let plan = match preflight(root, old, new, server, crash) {
        Ok(plan) => plan,
        Err(why) => {
            writeln!(err, "Error: {why}. Nothing was renamed.")?;
            return Ok(EXIT_FAILED);
        }
    };
    // The old result survived the preflight: rotate it aside byte-for-byte
    // (consulted by nothing further) and start the new transaction. A failed
    // rotation keeps the old result and refuses.
    if let Err(why) = rotate_aside(root, old, new) {
        writeln!(err, "Error: {why}. Nothing was renamed.")?;
        return Ok(EXIT_FAILED);
    }
    stopped_fresh(root, old, new, plan, crash, out, err)
}

/// Rotate a superseded completion aside, preserving its bytes under a
/// numbered history name no reader consults.
fn rotate_aside(root: &Path, old: &str, new: &str) -> Result<(), String> {
    let from = intent_path(root, old, new);
    for n in 0..1000 {
        let to = if n == 0 {
            rotated_intent_path(root, old, new)
        } else {
            PathBuf::from(format!(
                "{}.{}",
                rotated_intent_path(root, old, new).display(),
                n
            ))
        };
        if matches!(classify_node(&to), NodeKind::Absent) {
            return std::fs::rename(&from, &to).map_err(|why| {
                format!(
                    "could not rotate the superseded result aside ({} → {}, {why})",
                    from.display(),
                    to.display()
                )
            });
        }
    }
    Err(
        "the rotation chain for this pair is full — refusing rather than overwriting history"
            .to_owned(),
    )
}

/// The fresh-transaction tail shared by `stopped` and `converge_completed`:
/// witness, publish, cut, drive, report.
fn stopped_fresh(
    root: &Path,
    old: &str,
    new: &str,
    plan: Preflight,
    crash: Option<&str>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let (work_dev, work_ino) = match plan.mode {
        WorkMode::Local => (0, 0),
        _ => dir_id(Path::new(&plan.old_work)),
    };
    let (admin_dev, admin_ino) = match plan.mode {
        WorkMode::Git => dir_id(&admin_dir(&plan.origin, &plan.old_work)),
        _ => (0, 0),
    };
    if plan.mode != WorkMode::Local && (work_dev, work_ino) == (0, 0) {
        writeln!(
            err,
            "Error: the managed working copy at '{}' cannot be fingerprinted — refusing before the intent that would demand the proof. Nothing was renamed.",
            plan.old_work
        )?;
        return Ok(EXIT_FAILED);
    }
    if plan.mode == WorkMode::Git && (admin_dev, admin_ino) == (0, 0) {
        writeln!(
            err,
            "Error: the git administrative directory for '{}' cannot be fingerprinted — refusing before the intent that would demand the proof. Nothing was renamed.",
            plan.old_work
        )?;
        return Ok(EXIT_FAILED);
    }
    let mut intent = Intent {
        uuid: plan.uuid,
        old: old.to_owned(),
        new: new.to_owned(),
        mode: plan.mode,
        old_work: plan.old_work,
        new_work: plan.new_work,
        origin: plan.origin,
        server_kind: plan.server_kind,
        server_value: plan.server_value,
        phase: PHASE_PREPARED.to_owned(),
        work_dev,
        work_ino,
        admin_dev,
        admin_ino,
    };
    if let Err(why) = publish_intent(root, &intent) {
        writeln!(err, "Error: {why}. Nothing was renamed.")?;
        return Ok(EXIT_FAILED);
    }
    if let Some(code) = crash_cut("after-intent", crash, err)? {
        return Ok(code);
    }
    match complete_transaction(root, &mut intent, crash, err) {
        Ok(()) => {
            print_stopped_success(&intent, plan.warn_implicit, out, err)?;
            Ok(0)
        }
        Err(TxnError::Fail(why)) => {
            writeln!(err, "Error: {why}")?;
            Ok(EXIT_FAILED)
        }
        Err(TxnError::Crash(code)) => Ok(code),
    }
}

/// Drive a prepared intent through the settled order to `complete`.
///
/// Each step verifies-or-acts, records its phase, then offers its crash
/// boundary. A step whose facts already hold re-verifies without re-acting
/// and still offers its boundary. Failures name committed paths, remaining
/// work and the proved retry; the transaction stays recorded for the next
/// retry.
fn complete_transaction(
    root: &Path,
    intent: &mut Intent,
    crash: Option<&str>,
    err: &mut impl Write,
) -> Result<(), TxnError> {
    complete_transaction_until(
        root,
        intent,
        crash,
        err,
        park_deadline(std::time::Instant::now()),
    )
}

/// [`complete_transaction`] against an explicit park deadline: the production
/// path passes one fresh deadline per drive (observably identical to
/// per-cut deadlines — at most one cut fires per drive); tests drive spent
/// deadlines to prove terminality without sleeping.
fn complete_transaction_until(
    root: &Path,
    intent: &mut Intent,
    crash: Option<&str>,
    err: &mut impl Write,
    deadline: std::time::Instant,
) -> Result<(), TxnError> {
    let retry = retry_command(intent);
    // Local mode performs no work move and records no work-moved phase; the
    // armed work-move cut refuses in preflight, so reaching here unarmed is
    // the only local route.
    if intent.mode != WorkMode::Local && intent.phase == PHASE_PREPARED {
        if !work_moved(root, intent) {
            do_work_move(root, intent).map_err(|why| {
                format!(
                    "{why}. Committed: durable intent at phase '{PHASE_PREPARED}'. Remaining: work move, state-dir move, meta, assets, result. Retry '{retry}' to converge forward."
                )
            })?;
        }
        advance(root, intent, PHASE_WORK_MOVED).map_err(|why| {
            format!("the work move verified but its record failed ({why}). Retry '{retry}' to converge forward.")
        })?;
        if let Some(code) = crash_cut_until("after-work-move", crash, err, deadline)
            .map_err(|why| format!("could not write the crash attestation ({why})"))?
        {
            return Err(TxnError::Crash(code));
        }
    }
    if intent.phase == PHASE_PREPARED || intent.phase == PHASE_WORK_MOVED {
        if !state_moved(root, intent) {
            do_state_move(root, intent).map_err(|why| {
                format!(
                    "{why}. Committed: {}. Remaining: state-dir move, meta, assets, result. Retry '{}' to converge forward.",
                    committed_through(intent),
                    retry
                )
            })?;
        }
        advance(root, intent, PHASE_STATE_MOVED).map_err(|why| {
            format!("the state-dir move verified but its record failed ({why}). Retry '{retry}' to converge forward.")
        })?;
        if let Some(code) = crash_cut_until("after-state-move", crash, err, deadline)
            .map_err(|why| format!("could not write the crash attestation ({why})"))?
        {
            return Err(TxnError::Crash(code));
        }
    }
    if intent.phase == PHASE_STATE_MOVED {
        if !meta_coherent(root, intent) {
            do_meta(root, intent).map_err(|why| {
                format!(
                    "{why}. Committed: {}. Remaining: meta, assets, result. Retry '{}' to converge forward.",
                    committed_through(intent),
                    retry
                )
            })?;
        }
        advance(root, intent, PHASE_META_PUBLISHED).map_err(|why| {
            format!("the coherent meta verified but its record failed ({why}). Retry '{retry}' to converge forward.")
        })?;
        if let Some(code) = crash_cut_until("after-meta", crash, err, deadline)
            .map_err(|why| format!("could not write the crash attestation ({why})"))?
        {
            return Err(TxnError::Crash(code));
        }
    }
    if intent.phase == PHASE_META_PUBLISHED {
        if !assets_ready(root, intent) {
            do_assets(root, intent).map_err(|why| {
                format!(
                    "{why}. Committed: {}. Remaining: assets, result. Retry '{}' to converge forward.",
                    committed_through(intent),
                    retry
                )
            })?;
        }
        advance(root, intent, PHASE_ASSETS_PUBLISHED).map_err(|why| {
            format!("the checked assets verified but their record failed ({why}). Retry '{retry}' to converge forward.")
        })?;
        if let Some(code) = crash_cut_until("after-assets", crash, err, deadline)
            .map_err(|why| format!("could not write the crash attestation ({why})"))?
        {
            return Err(TxnError::Crash(code));
        }
    }
    if intent.phase == PHASE_ASSETS_PUBLISHED {
        publish_result(root, intent).map_err(|why| {
            format!(
                "{why}. Committed: {}. Remaining: durable result. Retry '{}' to converge forward.",
                committed_through(intent),
                retry
            )
        })?;
        // The durability cut: the completed result is already published, so
        // the attestation proves survival of process death, not partial work.
        if let Some(code) = crash_cut_until("after-result", crash, err, deadline)
            .map_err(|why| format!("could not write the crash attestation ({why})"))?
        {
            return Err(TxnError::Crash(code));
        }
    }
    if intent.phase != PHASE_COMPLETE {
        return Err(TxnError::Fail(format!(
            "the transaction rests at unexpected phase '{}' — refusing (manual repair only). Committed: {}. Retry '{}' after fixing it.",
            intent.phase,
            committed_through(intent),
            retry
        )));
    }
    Ok(())
}

/// What the recorded phase proves is committed, in fact names.
fn committed_through(intent: &Intent) -> String {
    // Local mode performs no work move: its phases must never claim one.
    let work = if intent.mode == WorkMode::Local {
        if intent.old_work.is_empty() {
            "local work path unchanged".to_owned()
        } else {
            format!("local work preserved at '{}'", intent.old_work)
        }
    } else {
        format!("managed work moved to '{}'", intent.new_work)
    };
    match intent.phase.as_str() {
        PHASE_PREPARED => "durable intent; no work or state move yet".to_owned(),
        PHASE_WORK_MOVED => format!("durable intent; {work}"),
        PHASE_STATE_MOVED => format!(
            "durable intent; {work}; state directory moved to '{}'",
            intent.new
        ),
        PHASE_META_PUBLISHED => format!(
            "durable intent; {work}; state directory moved to '{}'; coherent meta published",
            intent.new
        ),
        PHASE_ASSETS_PUBLISHED => format!(
            "durable intent; {work}; state directory moved to '{}'; coherent meta and checked assets published",
            intent.new
        ),
        _ => format!("durable intent at phase '{}'", intent.phase),
    }
}

/// The one user-visible stopped result: final stopped-state status without
/// waiting for any provider. Local mode preserves the caller-owned path;
/// managed modes converge to the fresh-name address.
fn print_stopped_success(
    intent: &Intent,
    warn_implicit: bool,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<()> {
    if intent.mode == WorkMode::Local {
        if intent.old_work.is_empty() {
            writeln!(
                out,
                "Renamed '{}' → '{}' (stopped; local work path unchanged).",
                intent.old, intent.new
            )?;
        } else {
            writeln!(
                out,
                "Renamed '{}' → '{}' (stopped; work preserved at '{}').",
                intent.old, intent.new, intent.old_work
            )?;
        }
    } else {
        writeln!(
            out,
            "Renamed '{}' → '{}' (stopped; managed work moved to '{}').",
            intent.old, intent.new, intent.new_work
        )?;
    }
    if warn_implicit {
        writeln!(
            err,
            "Warning: '{}' relied on implicit or unclassified config homes — the rename moved state without starting the provider, and a later resume re-proves the conversation with existing provider behavior.",
            intent.new
        )?;
    }
    Ok(())
}

/// A proved retry of the same command: reconcile actual paths and
/// registration against the recorded intent (including a move completed
/// before its phase update), then converge forward. Never a new ordinary
/// rename over partial state, never an overwrite of a replacement occupant.
#[allow(
    clippy::too_many_lines,
    reason = "one ordered re-proof checklist mirroring preflight: the locate/re-proof/complete sequence must read as a single order"
)]
fn recover(
    root: &Path,
    old: &str,
    new: &str,
    intent: &Intent,
    crash: Option<&str>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let mut intent = intent.clone();
    // Filename/argv/payload correlation, re-proved at every entry: only the
    // pair this command holds locks for may be recovered here.
    if intent.old != old || intent.new != new {
        writeln!(
            err,
            "Error: the rename intent for '{old}' → '{new}' names '{}' → '{}' — refusing a carrier that does not match this command. Nothing was renamed.",
            intent.old, intent.new
        )?;
        return Ok(EXIT_FAILED);
    }
    if intent.old == intent.new {
        writeln!(
            err,
            "Error: the recorded intent renames '{}' to itself — refusing damaged intent. Nothing was renamed.",
            intent.old
        )?;
        return Ok(EXIT_FAILED);
    }
    if intent.mode == WorkMode::Local && intent.phase == PHASE_WORK_MOVED {
        writeln!(
            err,
            "Error: the recorded intent is local mode at unreachable phase '{}' — refusing damaged intent. Nothing was renamed.",
            intent.phase
        )?;
        return Ok(EXIT_FAILED);
    }
    let sessions = crate::lifecycle::sessions_dir(root);
    let old_dir = sessions.join(&intent.old);
    let new_dir = sessions.join(&intent.new);
    // A live session over partial state stops recovery cold: stop it first,
    // then retry. Locks alone cannot serialize a running harness.
    for name in [&intent.old, &intent.new] {
        let dir = sessions.join(name);
        if let Some(server) = crate::session_launch::recorded_server_resolved(&dir)
            && crate::lifecycle::live_id(&server, name).is_some()
        {
            writeln!(
                err,
                "Error: session '{name}' is live while rename '{}' → '{}' is pending at phase '{}' — stop it first, then retry '{}'. Nothing was renamed.",
                intent.old,
                intent.new,
                intent.phase,
                retry_command(&intent)
            )?;
            return Ok(EXIT_FAILED);
        }
    }
    // Locate the UUID: exactly one side may hold it. A replacement occupant
    // (different UUID) is never overwritten; a missing both is manual repair.
    let old_holds = crate::lifecycle::dir_exists(&old_dir) && dir_uuid(&old_dir) == intent.uuid;
    let new_holds = crate::lifecycle::dir_exists(&new_dir) && dir_uuid(&new_dir) == intent.uuid;
    let old_occupied = crate::lifecycle::path_exists(&old_dir) && !old_holds;
    let new_occupied = crate::lifecycle::path_exists(&new_dir) && !new_holds;
    if new_occupied {
        writeln!(
            err,
            "Error: '{}' is now occupied by another session — refusing to overwrite a replacement path/UUID. Retry '{}' after freeing it.",
            new_dir.display(),
            retry_command(&intent)
        )?;
        return Ok(EXIT_FAILED);
    }
    if !old_holds && !new_holds {
        if old_occupied
            || crate::lifecycle::dir_exists(&old_dir)
            || crate::lifecycle::dir_exists(&new_dir)
        {
            writeln!(
                err,
                "Error: neither '{}' nor '{}' holds UUID '{}' anymore — refusing over replaced identity (manual repair only). Nothing was renamed.",
                intent.old, intent.new, intent.uuid
            )?;
        } else {
            writeln!(
                err,
                "Error: both '{}' and '{}' are gone for UUID '{}' — refusing (manual repair only). Nothing was renamed.",
                intent.old, intent.new, intent.uuid
            )?;
        }
        return Ok(EXIT_FAILED);
    }
    if old_occupied {
        writeln!(
            err,
            "Error: '{}' is now occupied by another session — refusing to touch a replacement occupant. Retry '{}' after freeing it.",
            old_dir.display(),
            retry_command(&intent)
        )?;
        return Ok(EXIT_FAILED);
    }
    // Re-proof: the recorded server pair and the managed addresses must read
    // back as recorded. A root or registration that moved under the handoff
    // refuses rather than converging the wrong transaction.
    let current_dir = if new_holds { &new_dir } else { &old_dir };
    let Some(server) = crate::session_launch::recorded_server_resolved(current_dir) else {
        writeln!(
            err,
            "Error: the recorded server for this transaction is now ambiguous — refusing a stale plan. Nothing was renamed."
        )?;
        return Ok(EXIT_FAILED);
    };
    let (kind, value) = server_pair(&server);
    if kind != intent.server_kind || value != intent.server_value {
        writeln!(
            err,
            "Error: the recorded server for this transaction changed while pending (recorded '{}/{}') — refusing a stale plan. Nothing was renamed.",
            intent.server_kind, intent.server_value
        )?;
        return Ok(EXIT_FAILED);
    }
    // Liveness and destination proofs are re-taken at point of use, never
    // carried from preflight across the durable cut: a revived session, an
    // occupied new name, or an unreachable server refuses the retry.
    match crate::transport::verify_session_absent(&server, &intent.old) {
        crate::tmux::StopProbe::Absent => {}
        crate::tmux::StopProbe::Present => {
            writeln!(
                err,
                "Error: session '{}' answered live on its recorded server while rename '{}' → '{}' is pending — stop it first, then retry '{}'. Nothing was renamed.",
                intent.old,
                intent.old,
                intent.new,
                retry_command(&intent)
            )?;
            return Ok(EXIT_FAILED);
        }
        crate::tmux::StopProbe::Unknown => {
            writeln!(
                err,
                "Error: cannot prove session '{}' stopped (its recorded tmux server is unreachable) — refusing a retry over unknown liveness. Nothing was renamed.",
                intent.old
            )?;
            return Ok(EXIT_FAILED);
        }
    }
    if crate::transport::session_exists(&server, &intent.new) {
        writeln!(
            err,
            "Error: session '{}' already exists — refusing to converge over an occupied destination. Nothing was renamed.",
            intent.new
        )?;
        return Ok(EXIT_FAILED);
    }
    if intent.mode != WorkMode::Local {
        let managed_old = crate::lifecycle::worktrees_dir(root).join(&intent.old);
        let managed_new = crate::lifecycle::worktrees_dir(root).join(&intent.new);
        if Path::new(&intent.old_work) != managed_old || Path::new(&intent.new_work) != managed_new
        {
            writeln!(
                err,
                "Error: the managed addresses for this transaction moved while pending — refusing a stale plan. Nothing was renamed."
            )?;
            return Ok(EXIT_FAILED);
        }
        // Teardown-grade root authority, re-taken: a swapped worktrees root
        // under a pending transaction refuses rather than moving through a
        // link.
        let worktrees = crate::lifecycle::worktrees_dir(root);
        if is_symlink(&worktrees) || !crate::lifecycle::dir_exists(&worktrees) {
            writeln!(
                err,
                "Error: the configured worktrees root '{}' is not a real directory — refusing a retry over moved ground. Nothing was renamed.",
                worktrees.display()
            )?;
            return Ok(EXIT_FAILED);
        }
        // Sharing and registration re-proofs: another session may have
        // claimed the path, or the worktree may have been locked, repointed,
        // or unregistered while the transaction was parked.
        let sessions = crate::lifecycle::sessions_dir(root);
        if let Err(why) = sharing_check(
            root,
            &sessions,
            &intent.old,
            &intent.new,
            &intent.old_work,
            Path::new(&intent.new_work),
        ) {
            writeln!(err, "Error: {why}. Nothing was renamed.")?;
            return Ok(EXIT_FAILED);
        }
        if intent.mode == WorkMode::Git
            && let Err(why) = git_recovery_check(root, &intent)
        {
            writeln!(err, "Error: {why}. Nothing was renamed.")?;
            return Ok(EXIT_FAILED);
        }
    }
    // Re-prove the immutable payload facts against the located meta. Mode,
    // origin and UUID never change; session/work rows legitimately change
    // across the transaction, so each address admits exactly its coherent
    // before/after shapes. A forged mode/origin/work intent over a
    // mismatched session refuses before either path moves.
    if let Err(why) = bind_payload_to_meta(current_dir, &intent, new_holds) {
        writeln!(err, "Error: {why}. Nothing was renamed.")?;
        return Ok(EXIT_FAILED);
    }
    // Re-probe explicit homes while the state still sits at the old address,
    // where the candidate paths are still meaningful: a transcript layout
    // that changed under the handoff refuses like a fresh one. Past the
    // state move the decision is irrevocable and the check is meaningless.
    if !new_holds {
        let home_bytes = crate::meta::read_bytes(current_dir).unwrap_or_default();
        let home_meta = crate::meta::Meta::parse(&String::from_utf8_lossy(&home_bytes));
        if let Err(why) =
            explicit_home_check(home_meta.roster(), &intent.old_work, &intent.new_work)
        {
            writeln!(err, "Error: {why}. Nothing was renamed.")?;
            return Ok(EXIT_FAILED);
        }
    }
    // Re-prove the recorded phase's cumulative facts before using it as a
    // skip boundary: a carrier claiming work the filesystem never did
    // normalizes to the highest proven phase (recorded forward) instead of
    // skipping into success; a record lagging verified facts catches up.
    let proven = proven_phase(root, &intent);
    if phase_ord(&proven) != phase_ord(&intent.phase) {
        intent.phase = proven;
        if let Err(why) = publish_intent(root, &intent) {
            writeln!(
                err,
                "Error: the transaction phase corrected but its record failed ({why}) — retry '{}'. Nothing was renamed.",
                retry_command(&intent)
            )?;
            return Ok(EXIT_FAILED);
        }
    }
    // A completed record converges to its durable result without duplicating
    // it — and never parks: retrying a completion is a success, not a cut.
    if intent.phase == PHASE_COMPLETE {
        if !new_holds {
            writeln!(
                err,
                "Error: the transaction completed for '{}' → '{}' but '{}' no longer holds it — refusing over drifted completion (manual repair only). Nothing was renamed.",
                intent.old, intent.new, intent.new
            )?;
            return Ok(EXIT_FAILED);
        }
        if let Err(why) = verify_completion(root, &intent) {
            writeln!(
                err,
                "Error: the transaction completed for '{}' → '{}' but {why} — refusing over drifted completion (manual repair only). Nothing was renamed.",
                intent.old, intent.new
            )?;
            return Ok(EXIT_FAILED);
        }
        print_stopped_success(&intent, false, out, err)?;
        return Ok(0);
    }
    // Pending requests that arrived mid-transaction strand the same way.
    let state_name = current_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let names = [intent.old.as_str(), intent.new.as_str()];
    match pending_blockers(root, &names, current_dir, state_name) {
        Ok(blockers) if !blockers.is_empty() => {
            writeln!(
                err,
                "Error: {} pending request(s) arrived while rename '{}' → '{}' was pending ({}) — close them first (reply from the target seat, or retire the holding seat), then retry '{}'. Nothing was renamed.",
                blockers.len(),
                intent.old,
                intent.new,
                blockers.join(", "),
                retry_command(&intent)
            )?;
            return Ok(EXIT_FAILED);
        }
        Ok(_) => {}
        Err(why) => {
            writeln!(err, "Error: {why}. Nothing was renamed.")?;
            return Ok(EXIT_FAILED);
        }
    }
    match complete_transaction(root, &mut intent, crash, err) {
        Ok(()) => {
            print_stopped_success(&intent, false, out, err)?;
            Ok(0)
        }
        Err(TxnError::Fail(why)) => {
            writeln!(err, "Error: {why}")?;
            Ok(EXIT_FAILED)
        }
        Err(TxnError::Crash(code)) => Ok(code),
    }
}

/// Prove a completed transaction still reads whole: the UUID, coherent meta
/// and verified assets at the new address, with no old address left behind.
fn verify_completion(root: &Path, intent: &Intent) -> Result<(), String> {
    let sessions = crate::lifecycle::sessions_dir(root);
    if !matches!(classify_node(&sessions.join(&intent.old)), NodeKind::Absent) {
        return Err(format!("the old address '{}' is back", intent.old));
    }
    if !meta_coherent(root, intent) {
        return Err("the completed meta no longer reads coherent".to_owned());
    }
    if !assets_ready(root, intent) {
        return Err("the completed assets no longer verify".to_owned());
    }
    if intent.mode != WorkMode::Local && !work_moved(root, intent) {
        return Err("the completed work move no longer verifies".to_owned());
    }
    Ok(())
}

/// Prove the intent's immutable facts against the located session meta.
/// `at_new` tells which address holds the UUID. Mode, origin and UUID never
/// change and must match exactly; session/work rows admit exactly the two
/// coherent shapes of their own publication (before/after), so a forged
/// mode/work intent over a mismatched session cannot move an unrelated path
/// or publish a contradictory meta.
fn bind_payload_to_meta(dir: &Path, intent: &Intent, at_new: bool) -> Result<(), String> {
    let bytes = crate::meta::read_bytes(dir)
        .map_err(|why| format!("the located session has no readable meta ({why})"))?;
    for key in ["session", "session_id", "work_dir", "mode", "origin"] {
        if count_rows(&bytes, key) > 1 {
            return Err(format!(
                "the located session records a duplicated '{key}' row — refusing over damaged identity"
            ));
        }
    }
    let value = |key: &str| crate::lifecycle::meta_value(&bytes, key);
    if value("session_id") != intent.uuid {
        return Err("the located session no longer holds the transaction UUID".to_owned());
    }
    if value("mode") != intent.mode.as_str() {
        return Err(format!(
            "the located session records mode '{}' but the transaction is '{}' — refusing a mismatched carrier",
            value("mode"),
            intent.mode.as_str()
        ));
    }
    if value("origin") != intent.origin {
        return Err(
            "the located session records a different origin than the transaction — refusing a mismatched carrier"
                .to_owned(),
        );
    }
    let (session, work) = (value("session"), value("work_dir"));
    let coherent = if intent.mode == WorkMode::Local {
        session == intent.old && work == intent.old_work
            || at_new && session == intent.new && work == intent.new_work
    } else if at_new {
        session == intent.old && work == intent.old_work
            || session == intent.new && work == intent.new_work
    } else {
        session == intent.old && work == intent.old_work
    };
    if !coherent {
        return Err(format!(
            "the located session records session '{session}' with work '{work}' — refusing a carrier that does not match its address"
        ));
    }
    Ok(())
}

/// Whether a launch, resume or end of `name` must stand aside for a pending
/// rename transaction — or a damaged carrier it can be attributed to.
/// Returns the remedy when blocked. A corrupt/truncated intent is evidence,
/// never absence. Unrelated literal pairs filter by filename before any read;
/// digest carriers (absurd-length names only) must be read to attribute, and
/// unattributable damage blocks loudly.
pub(crate) fn intent_blocks(root: &Path, name: &str) -> Option<String> {
    for path in carrier_files(root) {
        let relevant = match carrier_addr(&path) {
            CarrierAddr::Ignored => false,
            CarrierAddr::Literal { old, new } => old == name || new == name,
            CarrierAddr::Digest => true,
        };
        if !relevant {
            continue;
        }
        let Some(classified) = classify_file(&path) else {
            continue;
        };
        match classified.result {
            Err(why) => {
                // Attributed damage blocks only its own endpoints, like a
                // pending transaction does; genuinely unattributable damage
                // stays global. A foreign-stem A→B carrier must not deny an
                // unrelated C.
                let overlaps = match (&classified.old, &classified.new) {
                    (Some(old), Some(new)) => old == name || new == name,
                    _ => true,
                };
                if !overlaps {
                    continue;
                }
                let scope = match (classified.old, classified.new) {
                    (Some(old), Some(new)) => format!("for '{old}' → '{new}'"),
                    _ => "that names no attributable pair".to_owned(),
                };
                return Some(format!(
                    "a damaged rename carrier {scope} is pending ({why}) — repair or remove it by hand; this operation stands aside"
                ));
            }
            // Attributed transactions block only their own endpoints: one
            // pending long-name rename must not deny every unrelated
            // session. Unattributable damage above stays global.
            Ok(intent) if intent.phase != PHASE_COMPLETE => {
                if intent.old != name && intent.new != name {
                    continue;
                }
                return Some(format!(
                    "rename '{}' → '{}' is in progress at phase '{}' — retry '{}' to converge it forward; this operation stands aside",
                    intent.old,
                    intent.new,
                    intent.phase,
                    retry_command(&intent)
                ));
            }
            Ok(_) => {}
        }
    }
    None
}

/// Every pending (non-complete) rename transaction under `root`, oldest
/// first. Damaged carriers never appear here; readers that must not mistake
/// damage for absence use [`classify_file`] or [`pending_damaged`].
pub(crate) fn pending_intents(root: &Path) -> Vec<Intent> {
    let mut intents = Vec::new();
    for path in carrier_files(root) {
        if let Some(classified) = classify_file(&path)
            && let Ok(intent) = classified.result
            && intent.phase != PHASE_COMPLETE
        {
            intents.push(intent);
        }
    }
    intents.sort_by(|left, right| (&left.old, &left.new).cmp(&(&right.old, &right.new)));
    intents
}

/// Every damaged rename carrier under `root`, with attributable names where
/// the filename is literal. Doctor fails them loudly; lifecycle endpoints
/// consult [`intent_blocks`].
pub(crate) fn pending_damaged(root: &Path) -> Vec<DamagedCarrier> {
    let mut damaged = Vec::new();
    for path in carrier_files(root) {
        if let Some(classified) = classify_file(&path)
            && let Err(why) = classified.result
        {
            damaged.push(DamagedCarrier {
                old: classified.old,
                new: classified.new,
                why,
            });
        }
    }
    damaged.sort_by(|left, right| (&left.old, &left.new).cmp(&(&right.old, &right.new)));
    damaged
}

/// A missing path fact renders as `.`.
fn or_dot(value: String) -> String {
    if value.is_empty() {
        ".".to_owned()
    } else {
        value
    }
}

/// The session the caller is sitting in: the ambient server's current session,
/// and only when it is a real session directory.
fn current_session(root: &Path) -> Option<String> {
    let name = transport::observe_current_session(&ServerId::Ambient)?;
    crate::lifecycle::dir_exists(&crate::lifecycle::sessions_dir(root).join(&name)).then_some(name)
}

/// Whether `path` is a symlink.
fn is_symlink(path: &Path) -> bool {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the rename's path guard must classify the LINK, not what it points at — see clippy.toml"
    )]
    let probe = std::fs::symlink_metadata(path);
    probe.is_ok_and(|meta| meta.file_type().is_symlink())
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "tests read back what the doors wrote; the boundary is on product code — see clippy.toml"
)]
mod tests {
    use super::*;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|word| (*word).to_owned()).collect()
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = PathBuf::from(format!("/tmp/ae-rename-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sessions")).unwrap();
        dir
    }

    /// A0/A2: the crash seam takes exactly the six closed values — nothing
    /// empty, unknown, combined, padded or cased. Smallest defeating
    /// mutation: accept any nonempty value.
    #[test]
    fn the_crash_boundary_grammar_is_the_closed_six() {
        for boundary in CRASH_BOUNDARIES {
            assert!(is_crash_boundary(boundary), "{boundary}");
        }
        for refused in [
            "",
            "after-intent ",
            " after-intent",
            "AFTER-INTENT",
            "after-intent,after-meta",
            "after-intent after-meta",
            "after-",
            "never",
        ] {
            assert!(!is_crash_boundary(refused), "{refused:?}");
        }
    }

    #[test]
    fn no_operand_is_a_usage_refusal() {
        let root = scratch("noargs");
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&root, &[], &mut out, &mut err).unwrap();
        assert_eq!(code, EXIT_USAGE);
        assert!(String::from_utf8_lossy(&err).contains(USAGE));
        assert!(out.is_empty());
    }

    /// A2: the intent validator takes exactly its canonical document and
    /// refuses every other shape without panicking. Smallest defeating
    /// mutation: tolerate unknown keys or duplicate rows.
    fn valid_intent_doc() -> String {
        "rename_intent=1\n\
         session_id=e795c9e9-1234-4890-abcd-ef0123456789\n\
         old=sold\n\
         new=snew\n\
         mode=git\n\
         old_work=/tmp/ae/worktrees/sold\n\
         new_work=/tmp/ae/worktrees/snew\n\
         origin=/tmp/ae/project\n\
         server_kind=socket\n\
         server_value=/tmp/ae/sock\n\
         phase=prepared\n\
         work_dev=16777230\n\
         work_ino=100666701\n\
         admin_dev=16777230\n\
         admin_ino=100666699\n"
            .to_owned()
    }

    #[test]
    fn the_intent_document_round_trips() {
        let intent = parse_intent(valid_intent_doc().as_bytes()).expect("the canonical doc");
        assert_eq!(intent.uuid, "e795c9e9-1234-4890-abcd-ef0123456789");
        assert_eq!(intent.old, "sold");
        assert_eq!(intent.new, "snew");
        assert_eq!(intent.mode, WorkMode::Git);
        assert_eq!(intent.phase, PHASE_PREPARED);
        assert_eq!(
            parse_intent(intent_document(&intent).as_bytes()).expect("reparse"),
            intent
        );
    }

    #[test]
    fn the_intent_validator_refuses_every_non_canonical_shape() {
        let good = valid_intent_doc();
        let mutate = |key: &str, value: &str| {
            good.lines()
                .map(|line| {
                    if line.starts_with(&format!("{key}=")) {
                        format!("{key}={value}")
                    } else {
                        line.to_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
                + "\n"
        };
        let mut bad = vec![
            String::new(),
            "rename_intent=1\n".to_owned(),
            "a line without equals\n".to_owned(),
            good.clone() + "phase=complete\n",
            good.clone() + "locker=main\n",
            good.replace("phase=prepared\n", "phase=halfway\n"),
            good.replace("mode=git\n", "mode=copy\n"),
            good.replace("mode=git\n", "mode=\n"),
            good.replace(
                "session_id=e795c9e9-1234-4890-abcd-ef0123456789\n",
                "session_id=pending\n",
            ),
            good.replace("old=sold\n", "old=has space\n"),
            good.replace("server_kind=socket\n", "server_kind=spooky\n"),
            good.replace(
                "server_kind=socket\nserver_value=/tmp/ae/sock\n",
                "server_kind=ambient\nserver_value=/tmp/ae/sock\n",
            ),
            good.replace(
                "old_work=/tmp/ae/worktrees/sold\n",
                "old_work=relative/path\n",
            ),
            mutate("rename_intent", "2"),
            mutate("new", "sold"),
            mutate("work_dev", "abc"),
            mutate("admin_ino", "-1"),
            // Witness shape must follow the mode: local carries none, full
            // carries work alone, git carries both.
            good.replace("mode=git\n", "mode=local\n"),
            good.replace(
                "admin_dev=16777230\nadmin_ino=100666699\n",
                "admin_dev=0\nadmin_ino=0\n",
            ),
            good.replace(
                "work_dev=16777230\nwork_ino=100666701\n",
                "work_dev=0\nwork_ino=0\n",
            ),
        ];
        // Local mode with a move, and managed mode without one.
        bad.push(good.replace("mode=git\n", "mode=local\n").replace(
            "old_work=/tmp/ae/worktrees/sold\nnew_work=/tmp/ae/worktrees/snew\n",
            "old_work=/tmp/ae/project\nnew_work=/tmp/ae/other\n",
        ));
        bad.push(good.replace(
            "old_work=/tmp/ae/worktrees/sold\nnew_work=/tmp/ae/worktrees/snew\n",
            "old_work=/tmp/ae/same\nnew_work=/tmp/ae/same\n",
        ));
        // A missing key.
        bad.push(
            good.lines()
                .filter(|line| !line.starts_with("origin="))
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        );
        for doc in &bad {
            assert!(parse_intent(doc.as_bytes()).is_err(), "{doc:?}");
        }
        // The no-trailing-newline spelling still parses.
        assert!(parse_intent(good.trim_end().as_bytes()).is_ok());
        // The ambient server pair parses.
        assert!(
            parse_intent(
                good.replace(
                    "server_kind=socket\nserver_value=/tmp/ae/sock\n",
                    "server_kind=ambient\nserver_value=\n"
                )
                .as_bytes()
            )
            .is_ok()
        );
        // Non-UTF-8 and oversize refuse rather than panic or allocate.
        assert!(parse_intent(b"rename_intent=\xff\n").is_err());
        assert!(parse_intent(&vec![b'x'; INTENT_CAP + 1]).is_err());
    }

    #[test]
    fn the_porcelain_parser_reads_entries_and_locks() {
        let listed = "worktree /w/old\nHEAD abc\nbranch refs/heads/main\n\nworktree \"/w/my new\"\nHEAD abc\ndetached\nlocked\n";
        let entries = parse_worktree_porcelain(listed);
        assert_eq!(
            entries,
            vec![
                WorktreeEntry {
                    path: "/w/old".to_owned(),
                    locked: false,
                },
                WorktreeEntry {
                    path: "/w/my new".to_owned(),
                    locked: true,
                },
            ]
        );
        assert!(parse_worktree_porcelain("").is_empty());
    }

    /// I8: the production ceiling stays 60 seconds and terminal, proved
    /// without sleeping: the deadline constructor pins the seconds, and an
    /// already-spent deadline exits `EXIT_FAILED` with both diagnostics and
    /// no wait. Smallest defeating mutations: change the constant, map the
    /// timeout to success, or drop either diagnostic line.
    #[test]
    fn the_park_ceiling_is_sixty_seconds() {
        assert_eq!(CRASH_PARK_SECS, 60);
        let now = std::time::Instant::now();
        assert_eq!(
            park_deadline(now).saturating_duration_since(now),
            std::time::Duration::from_secs(CRASH_PARK_SECS)
        );
    }

    #[test]
    fn a_spent_deadline_is_terminal_with_both_diagnostics() {
        let mut err = Vec::new();
        let code = crash_cut_until(
            "after-intent",
            Some("after-intent"),
            &mut err,
            std::time::Instant::now(),
        )
        .expect("writes to a buffer");
        assert_eq!(code, Some(EXIT_FAILED));
        let text = String::from_utf8_lossy(&err);
        assert!(
            text.contains("rename-crash-boundary: after-intent"),
            "{text}"
        );
        assert!(
            text.contains("rename-crash-timeout: after-intent"),
            "{text}"
        );
    }

    #[test]
    fn an_unarmed_or_mismatched_cut_passes_through_silently() {
        for armed in [None, Some("after-meta")] {
            let mut err = Vec::new();
            let code = crash_cut_until("after-intent", armed, &mut err, std::time::Instant::now())
                .expect("writes to a buffer");
            assert_eq!(code, None);
            assert!(err.is_empty());
        }
    }

    /// IMPORTANT (r2-9): an already-spent deadline through the fresh
    /// transaction driver exits terminally with exactly the two diagnostics
    /// and byte-identical post-cut state — no marker string can leak into a
    /// reporter. Smallest defeating mutation: restore the string marker (the
    /// fresh caller prints it).
    #[test]
    fn a_spent_deadline_through_the_driver_is_terminal_and_exact() {
        let root = scratch("spent-driver");
        let dir = root.join("sessions").join("tdold");
        std::fs::create_dir_all(&dir).unwrap();
        let uuid = "e795c9e9-1234-4890-abcd-ef0123456789";
        std::fs::write(
            dir.join("meta"),
            format!("session=tdold\nsession_id={uuid}\nmode=local\norigin=/o\nwork_dir=/o\n"),
        )
        .unwrap();
        let mut intent = Intent {
            uuid: uuid.to_owned(),
            old: "tdold".to_owned(),
            new: "tdnew".to_owned(),
            mode: WorkMode::Local,
            old_work: "/o".to_owned(),
            new_work: "/o".to_owned(),
            origin: "/o".to_owned(),
            server_kind: "ambient".to_owned(),
            server_value: String::new(),
            phase: PHASE_PREPARED.to_owned(),
            work_dev: 0,
            work_ino: 0,
            admin_dev: 0,
            admin_ino: 0,
        };
        let mut err = Vec::new();
        let end = complete_transaction_until(
            &root,
            &mut intent,
            Some("after-state-move"),
            &mut err,
            std::time::Instant::now(),
        );
        assert!(matches!(end, Err(TxnError::Crash(1))), "{end:?}");
        assert_eq!(
            String::from_utf8_lossy(&err),
            "rename-crash-boundary: after-state-move\nrename-crash-timeout: after-state-move\n",
        );
        // The attested fact happened; nothing past it did.
        assert_eq!(intent.phase, PHASE_STATE_MOVED);
        assert!(root.join("sessions").join("tdnew").is_dir());
        assert_eq!(
            std::fs::read_to_string(root.join("sessions").join("tdnew").join("meta")).unwrap(),
            format!("session=tdold\nsession_id={uuid}\nmode=local\norigin=/o\nwork_dir=/o\n"),
            "no meta step past the cut"
        );
    }

    /// A prefix-trap intent: `tfoo`/`tfoobar` in both directions, so a
    /// substring proof passes stale bytes. Shape-valid (full with a work
    /// witness) so the trap tests the asset check, not the validator.
    fn trap_intent(old: &str, new: &str, old_work: &str, new_work: &str) -> Intent {
        parse_intent(
            format!(
                "rename_intent=1\n\
                 session_id=e795c9e9-1234-4890-abcd-ef0123456789\n\
                 old={old}\n\
                 new={new}\n\
                 mode=full\n\
                 old_work={old_work}\n\
                 new_work={new_work}\n\
                 origin=/o\n\
                 server_kind=ambient\n\
                 server_value=\n\
                 phase=prepared\n\
                 work_dev=5\n\
                 work_ino=7\n\
                 admin_dev=0\n\
                 admin_ino=0\n"
            )
            .as_bytes(),
        )
        .expect("a trap intent")
    }

    /// B4: the manifest proof compares canonical expected bytes, so any stale
    /// byte fails in both prefix directions. Smallest defeating mutation:
    /// fragment containment for the session or the address.
    #[test]
    fn the_manifest_proof_is_exact_in_both_prefix_directions() {
        let dir = scratch("manifest-trap");
        // The meta the render reads: managed mode, new work address.
        std::fs::write(
            dir.join("meta"),
            "session=tfoobar\nmode=full\norigin=/o\nwork_dir=/wt/tfoobar\nmain_pane=%0\n",
        )
        .unwrap();
        let forward = trap_intent("tfoo", "tfoobar", "/wt/tfoo", "/wt/tfoobar");
        let backward = trap_intent("tfoobar", "tfoo", "/wt/tfoobar", "/wt/tfoo");
        // Fresh canonical bytes verify for the pair they render...
        let fresh = expected_manifest(&dir, "tfoobar");
        std::fs::write(dir.join("workspace.md"), &fresh).unwrap();
        assert!(manifest_ready(&dir, &forward));
        // ...but never for the reverse pair, whose every line differs only by
        // the shared prefix.
        assert!(!manifest_ready(&dir, &backward));
        // Any stale byte fails, including a mixed address a fragment check
        // accepts (it contains every new-name/new-work fragment).
        let stale = fresh.replacen("Directory: /wt/tfoobar", "Directory: /wt/tfoo", 1);
        assert_ne!(stale, fresh);
        std::fs::write(dir.join("workspace.md"), &stale).unwrap();
        assert!(!manifest_ready(&dir, &forward));
        assert!(!manifest_ready(&dir, &backward));
    }

    /// B4 + I7: opencode pairs compare canonical expected bytes per
    /// tool-required seat, and helper links must address the recorded core
    /// and exist. Smallest defeating mutations: fragment pointers;
    /// presence-based pairs; type-only helper checks.
    #[test]
    fn the_provider_and_helper_proofs_reject_stale_targets() {
        let dir = scratch("asset-trap");
        let core = dir.join("ae-core");
        std::fs::write(&core, b"core").unwrap();
        let other = dir.join("other");
        std::fs::write(&other, b"other").unwrap();
        let meta = format!(
            "session=tfoobar\nmode=full\norigin=/o\nwork_dir=/wt/tfoobar\nseat.main=lead\nprofile.main=opencode\nharness_session.main=e795c9e9-1234-4890-abcd-ef0123456789\nagent_bin.main=/tmp/fake-bin/opencode\nae_core={}\n",
            core.display()
        );
        std::fs::write(dir.join("meta"), &meta).unwrap();
        let forward = trap_intent("tfoo", "tfoobar", "/wt/tfoo", "/wt/tfoobar");
        // Canonical bytes from the plan itself verify...
        let plan = opencode_plan(&dir, &forward).expect("a plan");
        assert_eq!(plan.required.len(), 1, "the opencode seat requires a pair");
        for file in &plan.required {
            std::fs::write(&file.md, &file.md_bytes).unwrap();
            std::fs::write(&file.json, &file.json_bytes).unwrap();
        }
        assert!(opencode_current(&dir, &forward));
        // ...a missing pair does not (absence is never ready)...
        let md_only = plan.required[0].md.clone();
        assert!(std::fs::remove_file(&md_only).is_ok());
        assert!(!opencode_current(&dir, &forward));
        std::fs::write(
            &md_only,
            "stale prompt mentioning tfoobar and /wt/tfoobar fragments\n",
        )
        .unwrap();
        // ...and neither does a stale file carrying every fragment.
        assert!(!opencode_current(&dir, &forward));

        // Helpers: good links pass; a dangling or repointed `send` fails.
        for helper in crate::shim::HELPERS {
            std::os::unix::fs::symlink(&core, dir.join(helper.name)).unwrap();
        }
        assert!(helpers_ready(&dir));
        let _ = std::fs::remove_file(dir.join("send"));
        std::os::unix::fs::symlink(dir.join("nowhere"), dir.join("send")).unwrap();
        assert!(!helpers_ready(&dir), "a dangling link is not checked");
        let _ = std::fs::remove_file(dir.join("send"));
        std::os::unix::fs::symlink(&other, dir.join("send")).unwrap();
        assert!(!helpers_ready(&dir), "a repointed link is not checked");
    }

    /// Sweep ADD: the state move itself refuses a destination entry even
    /// past the under-lock recheck. Smallest defeating mutation: drop the
    /// destination classification (the planted link is replaced).
    #[test]
    fn the_state_move_refuses_a_destination_entry() {
        let root = scratch("state-dest-entry");
        let sessions = root.join("sessions");
        let old = sessions.join("sold");
        std::fs::create_dir_all(&old).unwrap();
        let uuid = "e795c9e9-1234-4890-abcd-ef0123456789";
        std::fs::write(
            old.join("meta"),
            format!("session=sold\nsession_id={uuid}\nmode=local\norigin=/o\nwork_dir=/o\n"),
        )
        .unwrap();
        let intent = Intent {
            uuid: uuid.to_owned(),
            old: "sold".to_owned(),
            new: "snew".to_owned(),
            mode: WorkMode::Local,
            old_work: "/o".to_owned(),
            new_work: "/o".to_owned(),
            origin: "/o".to_owned(),
            server_kind: "ambient".to_owned(),
            server_value: String::new(),
            phase: PHASE_PREPARED.to_owned(),
            work_dev: 0,
            work_ino: 0,
            admin_dev: 0,
            admin_ino: 0,
        };
        std::os::unix::fs::symlink("/nonexistent-target-under-test", sessions.join("snew"))
            .unwrap();
        let refused = do_state_move(&root, &intent);
        assert!(refused.is_err(), "a planted link must refuse");
        assert!(old.is_dir(), "the old state is intact");
        assert_eq!(
            std::fs::read_link(sessions.join("snew")).unwrap(),
            PathBuf::from("/nonexistent-target-under-test"),
            "the planted link is preserved"
        );
    }

    /// B7: the helper core prefers the recorded pin while it exists, else the
    /// running core. Smallest defeating mutation: always the running core
    /// (a pinned session repairs away from its pin).
    #[test]
    fn the_helper_core_prefers_the_recorded_pin() {
        let dir = scratch("helper-core");
        let pinned = dir.join("pinned-core");
        std::fs::write(&pinned, b"pinned").unwrap();
        std::fs::write(
            dir.join("meta"),
            format!("session=s\nmode=local\nae_core={}\n", pinned.display()),
        )
        .unwrap();
        assert_eq!(helper_core(&dir), Some(pinned.clone()));
        assert!(std::fs::remove_file(&pinned).is_ok());
        assert_eq!(helper_core(&dir), crate::shape::resolved_exe());
    }

    /// IMPORTANT (r4-I2): with no provable core, readiness fails instead of
    /// blessing arbitrary links. Smallest defeating mutation: existence-only
    /// fallback (unproved links read ready).
    #[test]
    fn unproved_core_never_reads_ready() {
        let dir = scratch("unproved-core");
        let other = dir.join("other");
        std::fs::write(&other, b"other").unwrap();
        for helper in crate::shim::HELPERS {
            std::os::unix::fs::symlink(&other, dir.join(helper.name)).unwrap();
        }
        assert!(!helpers_ready_with(&dir, None));
    }

    /// IMPORTANT (r4-I3 + r5): a valid payload under a foreign digest stem
    /// is damage with attributable endpoints — never trusted pending state,
    /// never silently foreign. Smallest defeating mutation: skip the stem
    /// correlation (the ghost becomes pending).
    #[test]
    fn a_foreign_digest_stem_is_damaged_not_pending() {
        let root = scratch("foreign-digest");
        let sessions = root.join("sessions");
        let old: String = std::iter::repeat_n('o', 128).collect();
        let new: String = std::iter::repeat_n('n', 128).collect();
        let doc = format!(
            "rename_intent=1\nsession_id=e795c9e9-1234-4890-abcd-ef0123456789\nold={old}\nnew={new}\nmode=local\nold_work=/w\nnew_work=/w\norigin=/w\nserver_kind=ambient\nserver_value=\nphase=prepared\nwork_dev=0\nwork_ino=0\nadmin_dev=0\nadmin_ino=0\n"
        );
        // The payload's own stem would be trusted; a foreign stem is not.
        let own = sessions.join(format!("{}.intent", carrier_stem(&old, &new)));
        std::fs::write(&own, &doc).unwrap();
        assert!(matches!(
            classify_file(&own),
            Some(Classified { result: Ok(_), .. })
        ));
        let ghost = sessions.join(".rename.0000000000000000.intent");
        std::fs::write(&ghost, &doc).unwrap();
        match classify_file(&ghost) {
            Some(Classified {
                old: Some(ghost_old),
                new: Some(ghost_new),
                result: Err(_),
            }) => {
                assert_eq!(ghost_old, old);
                assert_eq!(ghost_new, new);
            }
            other => panic!("a foreign stem must be attributed damage: {other:?}"),
        }
        assert!(intent_blocks(&root, &old).is_some(), "own stem blocks");
    }

    /// IMPORTANT (r4-I4): an attributed pending digest transaction blocks
    /// only its own endpoints; unrelated sessions proceed. Smallest defeating
    /// mutation: drop the endpoint filter (one long-name rename denies the
    /// fleet).
    #[test]
    fn a_pending_digest_blocks_only_its_endpoints() {
        let root = scratch("digest-endpoints");
        let sessions = root.join("sessions");
        let old: String = std::iter::repeat_n('o', 128).collect();
        let new: String = std::iter::repeat_n('n', 128).collect();
        let doc = format!(
            "rename_intent=1\nsession_id=e795c9e9-1234-4890-abcd-ef0123456789\nold={old}\nnew={new}\nmode=local\nold_work=/w\nnew_work=/w\norigin=/w\nserver_kind=ambient\nserver_value=\nphase=prepared\nwork_dev=0\nwork_ino=0\nadmin_dev=0\nadmin_ino=0\n"
        );
        std::fs::write(
            sessions.join(format!("{}.intent", carrier_stem(&old, &new))),
            &doc,
        )
        .unwrap();
        assert!(intent_blocks(&root, &old).is_some());
        assert!(intent_blocks(&root, &new).is_some());
        assert_eq!(intent_blocks(&root, "unrelated"), None);
    }

    #[test]
    fn more_than_two_operands_is_a_usage_refusal() {
        let root = scratch("threeargs");
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&root, &words(&["a", "b", "c"]), &mut out, &mut err).unwrap();
        assert_eq!(code, EXIT_USAGE);
    }

    #[test]
    fn the_target_takes_the_grammar_and_the_refusal_quotes_it() {
        let root = scratch("grammar");
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&root, &words(&["old", "has space"]), &mut out, &mut err).unwrap();
        assert_eq!(code, EXIT_FAILED);
        let text = String::from_utf8_lossy(&err);
        assert!(text.contains("invalid session name 'has space'"), "{text}");
        assert!(
            text.contains(crate::session_launch::name::SESSION_NAME_GRAMMAR),
            "{text}"
        );
        assert!(out.is_empty(), "nothing was renamed");
    }

    #[test]
    fn a_symlinked_session_path_is_refused_before_any_move() {
        let root = scratch("symlink");
        let sessions = root.join("sessions");
        std::fs::create_dir_all(sessions.join("real")).unwrap();
        std::os::unix::fs::symlink(sessions.join("real"), sessions.join("linked")).unwrap();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&root, &words(&["linked", "fresh"]), &mut out, &mut err).unwrap();
        assert_eq!(code, EXIT_FAILED);
        assert!(
            String::from_utf8_lossy(&err).contains("is a symlink"),
            "{}",
            String::from_utf8_lossy(&err)
        );
        // The link is still a link: nothing was moved through it.
        assert!(is_symlink(&sessions.join("linked")));
    }

    /// A1: a stopped source without a stable UUID refuses explicitly — minting
    /// one implicitly is an explicit migration decision, never a rename side
    /// effect. Smallest defeating mutation: skip the UUID proof in preflight.
    #[test]
    fn a_source_without_a_stable_uuid_is_refused_and_moves_nothing() {
        let root = scratch("notrunning");
        let dir = root.join("sessions").join("old");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("meta"), "session=old\nmode=local\n").unwrap();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&root, &words(&["old", "new"]), &mut out, &mut err).unwrap();
        assert_eq!(code, EXIT_FAILED);
        assert!(
            String::from_utf8_lossy(&err).contains("no stable session UUID"),
            "{}",
            String::from_utf8_lossy(&err)
        );
        assert!(dir.join("meta").exists(), "the source survived");
        assert!(!root.join("sessions").join("new").exists());
        assert!(
            !root
                .join("sessions")
                .join(".rename.old.new.intent")
                .exists(),
            "no intent was published"
        );
    }

    #[test]
    fn the_status_paths_shape_is_mode_aware_and_shortened_against_home() {
        assert_eq!(
            crate::session_launch::status_paths("local", "/o", "/w", "/h"),
            "/w"
        );
        assert_eq!(
            crate::session_launch::status_paths("git", "/o", "/w", "/h"),
            "/o → /w"
        );
        // The shortening is what keeps the branch and the watch count on the
        // bar beside a worktree path spelled in full.
        assert_eq!(
            crate::session_launch::status_paths("git", "/h/p/ae", "/h/.ae/worktrees/feat", "/h"),
            "~/p/ae → ~/…/worktrees/feat"
        );
    }
}
