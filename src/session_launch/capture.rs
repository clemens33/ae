//! Post-launch session-id capture for the tools with no launch-time id flag.
//!
//! Codex, opencode, gemini, agy and Muse all learn their conversation id only after
//! they start, so ae asks each of them a different way:
//!
//! | Tool | How the id is found |
//! |---|---|
//! | codex | the `codex.<slot>.sid` file its own `developer_instructions` write, verified against the current launch token; then a launch-token scan of the recorded config home's UTC day partitions from capture birth through today (last 30 days when the birth is unknown); legacy seats with no token may scan only today/yesterday by cwd and fall back to their TUI header |
//! | opencode | `opencode session list --format json`, matched on the session's `directory` |
//! | gemini | `~/.gemini/tmp/<project>/chats/session-*.json`, matched on the launch token, then on the project root alone |
//! | agy | the launch token, searched in the BYTES of `~/.gemini/antigravity-cli/conversations/<id>.db` — OR, for a seat that has no token at all, the CLI log that names both the workspace and the conversation it created. Alternatives, not a chain: a token miss stays pending, because falling through cross-wires two seats sharing one directory |
//! | muse | the launch token, searched as raw bytes in `~/.local/share/muse/sessions/YYYY/MM/DD/<id>/session.jsonl`; the directory basename is the id, and a token miss stays pending |
//!
//! Every scan is filtered by the seat's `capture_floor.<slot>`, published
//! before the tool starts. Codex checks the
//! rollout's immutable creation timestamp as well as its mutable file mtime.
//!
//! Runs in ITS OWN DETACHED PROCESS, never on the launch's thread, so a tool
//! that takes half a minute to print its id does not delay the attach.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::inventory::ServerId;
use crate::time::Timestamp;
use crate::tool::{CaptureSpec, InitialTurn, ToolKind};

/// How many times a capture looks.
const POLLS: u32 = 6;

/// The pause between looks.
const POLL: Duration = Duration::from_secs(5);

/// UTC partition width in the Codex session store.
const SECONDS_PER_DAY: i64 = 86_400;

/// A legacy retained seat has no recorded birth. Its positive launch token is
/// strong proof, but the search still needs a finite boundary.
const CODEX_UNKNOWN_FLOOR_DAYS: i64 = 30;

/// How many sessions `opencode session list` is asked for.
const OPENCODE_LIST_LIMIT: &str = "20";

/// How many bytes of that answer ae will read. The argv caps the list at 20
/// entries — measured 5,852 bytes on 2026-09-18 — so this ceiling sits far
/// above any real answer and still bounds a runaway child.
const OPENCODE_LIST_CAP: u64 = 1024 * 1024;

/// One agent whose id must be captured after it starts.
#[derive(Debug, Clone)]
pub(crate) struct Target {
    /// The seat's slot — the roster key the captured id is written under.
    pub(crate) slot: String,
    /// Which harness it is.
    pub(crate) tool: ToolKind,
    /// The pane, for the TUI fallback.
    pub(crate) pane: String,
}

/// A capture argv minted ONLY by [`argv`], so the detached process door cannot
/// be handed an arbitrary command line.
pub struct CaptureArgv(Vec<String>);

impl CaptureArgv {
    /// The argv for the door to spawn.
    pub(crate) fn as_args(&self) -> &[String] {
        &self.0
    }
}

/// The FIXED argv of `opencode session list`, minted only by
/// [`opencode_list_argv`].
pub struct OpenCodeArgv(Vec<String>);

impl OpenCodeArgv {
    /// The argv for the door to run.
    pub(crate) fn as_args(&self) -> &[String] {
        &self.0
    }
}

/// The only `opencode` command ae runs: its own session list, as JSON.
fn opencode_list_argv() -> OpenCodeArgv {
    OpenCodeArgv(vec![
        "session".to_owned(),
        "list".to_owned(),
        "--format".to_owned(),
        "json".to_owned(),
        "-n".to_owned(),
        OPENCODE_LIST_LIMIT.to_owned(),
    ])
}

/// The session-id grammar the export leg accepts before argv: measured ids are
/// `ses_` + 26 alphanumerics (opencode 1.18.31, 2026-09-18), and the mint below
/// refuses everything else — a hand-edited meta can never smuggle a flag or a
/// path into the child's command line.
#[must_use]
pub(crate) fn is_opencode_session_id(value: &str) -> bool {
    value.strip_prefix("ses_").is_some_and(|rest| {
        !rest.is_empty()
            && rest.len() <= 64
            && rest.bytes().all(|byte| byte.is_ascii_alphanumeric())
    })
}

/// The SECOND fixed spelling of the `opencode` leg: `export <id>`. `None` when
/// `id` fails [`is_opencode_session_id`], so the door can never be handed an
/// argv built from an invalid name.
#[must_use]
pub(crate) fn opencode_export_argv(id: &str) -> Option<OpenCodeArgv> {
    is_opencode_session_id(id).then(|| OpenCodeArgv(vec!["export".to_owned(), id.to_owned()]))
}

/// The argv that captures one target: `_capture-sid <dir> <slot> <pane>`.
fn argv(dir: &Path, target: &Target) -> CaptureArgv {
    CaptureArgv(vec![
        crate::cli::CAPTURE_SID.to_owned(),
        dir.display().to_string(),
        target.slot.clone(),
        target.pane.clone(),
    ])
}

/// Start one DETACHED capture per target that needs one.
pub(crate) fn start(dir: &Path, targets: &[Target]) {
    // RESOLVED, never raw: this becomes a detached child's `argv[0]`, and on
    // macOS an unresolved answer is whichever link the caller typed — a helper
    // name, which the shim dispatch would read as that helper instead of
    // `_capture-sid`.
    let Some(exe) = crate::shape::resolved_exe() else {
        return;
    };
    for target in targets {
        if !target.tool.adapter().capture.is_needed() {
            continue;
        }
        let _ = crate::transport::spawn_detached(&exe, &argv(dir, target));
    }
}

/// `_capture-sid <dir> <slot> <pane>` — the detached child's whole job.
pub fn run(dir: &Path, slot: &str, pane: &str, server: &ServerId) -> u8 {
    let Some(facts) = facts(dir, slot) else {
        return 0;
    };
    let home = home_dir();
    let captured = match facts.tool.adapter().capture {
        CaptureSpec::HandshakeRolloutOrTui => {
            let config_home = codex_config_home(&facts, home.as_deref());
            capture_codex(dir, slot, pane, server, config_home.as_deref(), &facts)
                .map(|id| Captured::new(&facts, id))
        }
        CaptureSpec::SessionList => born_captured(&facts, capture_opencode(dir, slot, &facts)),
        CaptureSpec::ChatHistory => home
            .as_deref()
            .and_then(|home| capture_gemini(home, &facts))
            .map(|id| Captured::new(&facts, id)),
        CaptureSpec::ConversationDatabaseOrLog => home
            .as_deref()
            .and_then(|home| capture_agy(home, &facts))
            .map(|id| Captured::new(&facts, id)),
        CaptureSpec::MuseDatedSessions => home
            .as_deref()
            .and_then(|home| capture_muse(home, &facts))
            .map(|id| Captured::new(&facts, id)),
        CaptureSpec::None => None,
    };
    if let Some(captured) = captured {
        let _ = commit(dir, slot, &captured);
    }
    0
}

/// Wrap an attributed opencode find: shared by the launch capture and the
/// watchdog re-scan, so one pin set covers both arms.
fn born_captured(facts: &Facts, found: Option<(String, i64)>) -> Option<Captured> {
    found.map(|(id, born)| Captured::with_born(facts, id, born))
}

// ---------------------------------------------------------------------------
// the watchdog's recovery: one look per tick, for a seat still pending
// ---------------------------------------------------------------------------

/// One seat whose id a recovery tick may still find.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    /// The roster key the captured id is written under.
    pub slot: String,
    /// The agent's name — what an event about the recovery names.
    pub agent: String,
    /// The harness sitting in the seat, read from `agent_bin.<slot>`.
    pub tool: ToolKind,
}

/// One captured id, bound to the launch facts observed before the scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captured {
    id: String,
    agent: String,
    tool: ToolKind,
    launch_id: String,
    work_dir: String,
    provenance: crate::meta::SeatProvenance,
    /// The candidate's immutable birth (milliseconds), opencode only: the
    /// commit recheck compares it against the siblings' floors.
    born_ms: Option<i64>,
}

impl Captured {
    fn new(facts: &Facts, id: String) -> Self {
        Self {
            id,
            agent: facts.agent.clone(),
            tool: facts.tool,
            launch_id: facts.launch_id.clone(),
            work_dir: facts.work_dir.clone(),
            provenance: facts.provenance,
            born_ms: None,
        }
    }

    /// An opencode capture with the candidate's birth attached.
    fn with_born(facts: &Facts, id: String, born: i64) -> Self {
        Self {
            born_ms: Some(born),
            ..Self::new(facts, id)
        }
    }

    /// The harness session id this launch proved.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// The seats one recovery pass tries: an id still unrecorded, in a seat whose
/// tool has no launch-time id flag.
#[must_use]
pub fn pending_seats(roster: &[crate::meta::RosterEntry]) -> Vec<Pending> {
    roster
        .iter()
        .filter(|entry| is_pending(entry.harness_session.as_deref()))
        .filter_map(|entry| {
            let tool = ToolKind::from_binary_name(entry.binary.as_deref().unwrap_or_default());
            tool.adapter().capture.is_needed().then(|| Pending {
                slot: entry.slot.clone(),
                agent: entry.name.clone(),
                tool,
            })
        })
        .collect()
}

/// Whether a roster's recorded id still means "no id yet".
fn is_pending(id: Option<&str>) -> bool {
    id.is_none_or(|id| id.is_empty() || id == crate::launch::PENDING)
}

/// ONE look for a seat's id: no sleeping, no pane, no handshake file.
///
/// The pending row came from the watchdog's roster snapshot. A reused slot is
/// not that row, even when its current occupant already has a capturable id.
#[must_use]
pub fn attempt(dir: &Path, pending: &Pending) -> Option<Captured> {
    let bytes = crate::meta::read_bytes(dir).ok()?;
    let facts = facts_from(&bytes, &pending.slot)?;
    if facts.agent != pending.agent || facts.tool != pending.tool {
        return None;
    }
    let home = home_dir();
    match facts.tool.adapter().capture {
        CaptureSpec::HandshakeRolloutOrTui => codex_config_home(&facts, home.as_deref())
            .as_deref()
            .and_then(|config_home| scan_codex(config_home, &facts))
            .map(|id| Captured::new(&facts, id)),
        CaptureSpec::ChatHistory => home
            .as_deref()
            .and_then(|home| scan_gemini(home, &facts))
            .map(|id| Captured::new(&facts, id)),
        CaptureSpec::ConversationDatabaseOrLog => home
            .as_deref()
            .and_then(|home| scan_agy(home, &facts))
            .map(|id| Captured::new(&facts, id)),
        CaptureSpec::MuseDatedSessions => home
            .as_deref()
            .and_then(|home| scan_muse(home, &facts))
            .map(|id| Captured::new(&facts, id)),
        CaptureSpec::SessionList => {
            born_captured(&facts, scan_opencode(&bytes, &pending.slot, &facts))
        }
        CaptureSpec::None => None,
    }
}

// ---------------------------------------------------------------------------
// what the capture reads about itself
// ---------------------------------------------------------------------------

/// What one seat's capture needs to know, all of it from the session's meta.
struct Facts {
    /// The agent occupying the slot when these facts were read.
    agent: String,
    /// Which harness the seat holds — read from `agent_bin.<slot>`, because the
    /// roster is the core's record of what a seat is.
    tool: ToolKind,
    /// The seat's effective directory, which every cwd match compares against.
    work_dir: String,
    /// Recorded or inherited — provenance, never value, gates the cwd legs.
    provenance: crate::meta::SeatProvenance,
    /// The oldest conversation birth this seat may accept, in epoch seconds.
    capture_floor: i64,
    /// The launch token, empty when none was minted.
    launch_id: String,
    /// The adapter-owned marker prefix written into the tool store.
    launch_marker: Option<&'static str>,
    /// The recorded config store; missing means the legacy default.
    config_home: crate::meta::RecordedConfigHome,
}

/// Read one seat's capture facts, or nothing when the meta cannot be read.
fn facts(dir: &Path, slot: &str) -> Option<Facts> {
    let bytes = crate::meta::read_bytes(dir).ok()?;
    facts_from(&bytes, slot)
}

/// Read one seat's capture facts from meta bytes the caller already holds, so
/// the facts and the sibling set below come from ONE snapshot.
fn facts_from(bytes: &[u8], slot: &str) -> Option<Facts> {
    let (work_dir, provenance) = crate::meta::effective_seat_dir(bytes, slot).ok()?;
    let parsed = crate::meta::Meta::parse(&String::from_utf8_lossy(bytes));
    let value = |key: &str| {
        crate::meta::first_value(bytes, key)
            .map(|raw| String::from_utf8_lossy(raw).into_owned())
            .unwrap_or_default()
    };
    let entry = parsed.roster().iter().find(|entry| entry.slot == slot);
    let tool = ToolKind::from_binary_name(
        entry
            .and_then(|entry| entry.binary.as_deref())
            .unwrap_or_default(),
    );
    Some(Facts {
        agent: entry.map(|entry| entry.name.clone()).unwrap_or_default(),
        tool,
        work_dir,
        provenance,
        // New launches always publish this before exec. A retained conversation
        // from metadata predating the row gets an unbounded floor: its token or
        // recorded id is stronger evidence than a later resume timestamp. A
        // still-pending legacy seat keeps the old launch-time safety floor.
        capture_floor: crate::meta::first_value(bytes, &format!("capture_floor.{slot}"))
            .map_or_else(
                || {
                    if entry.is_some_and(|entry| !is_pending(entry.harness_session.as_deref())) {
                        0
                    } else {
                        crate::meta::first_value(bytes, &format!("launch_time.{slot}"))
                            .map_or(0, epoch_or_zero)
                    }
                },
                epoch_or_zero,
            ),
        launch_id: value(&format!("launch_id.{slot}")),
        launch_marker: tool.adapter().launch_marker,
        config_home: entry
            .map(|entry| entry.config_home.clone())
            .unwrap_or_default(),
    })
}

/// Every OTHER pending seat of this session whose tool shares the list
/// capture: the rival claimants a candidate must not also cover. Read from the
/// same bytes as the seat's own facts, never from a second snapshot. Each
/// sibling's window comes from its own facts — one floor owner — and a seat
/// whose facts cannot be read fails closed to covering everything.
fn siblings_from(bytes: &[u8], slot: &str) -> Vec<Sibling> {
    let parsed = crate::meta::Meta::parse(&String::from_utf8_lossy(bytes));
    parsed
        .roster()
        .iter()
        .filter(|entry| entry.slot != slot)
        .filter(|entry| is_pending(entry.harness_session.as_deref()))
        .filter(|entry| {
            ToolKind::from_binary_name(entry.binary.as_deref().unwrap_or_default())
                .adapter()
                .capture
                == CaptureSpec::SessionList
        })
        .map(|entry| {
            facts_from(bytes, &entry.slot).map_or(
                Sibling {
                    dir: None,
                    floor_ms: 0,
                },
                |seat| Sibling {
                    dir: Some(canonical(&seat.work_dir)),
                    floor_ms: seat.capture_floor.saturating_mul(1000),
                },
            )
        })
        .collect()
}

/// Every id this session already records, any tool: a candidate carrying one
/// is invisible to the attribution. Pending rows record nothing.
fn excluded_from(bytes: &[u8]) -> Vec<String> {
    let parsed = crate::meta::Meta::parse(&String::from_utf8_lossy(bytes));
    parsed
        .roster()
        .iter()
        .filter_map(|entry| entry.harness_session.clone())
        .filter(|id| !is_pending(Some(id)))
        .collect()
}

/// An invalid persisted epoch removes the lower bound rather than inventing
/// one. Identity proof still comes from token/cwd matching.
fn epoch_or_zero(raw: &[u8]) -> i64 {
    let text = String::from_utf8_lossy(raw);
    if text.bytes().all(|byte| byte.is_ascii_digit()) {
        text.parse().unwrap_or(0)
    } else {
        0
    }
}

/// Codex's recorded config root, or the pre-row default for legacy metadata.
fn codex_config_home(facts: &Facts, ambient_home: Option<&Path>) -> Option<PathBuf> {
    match &facts.config_home {
        crate::meta::RecordedConfigHome::Path(path)
        | crate::meta::RecordedConfigHome::Implicit(path) => Some(path.clone()),
        crate::meta::RecordedConfigHome::Missing => ambient_home.map(|home| home.join(".codex")),
        crate::meta::RecordedConfigHome::Absent
        | crate::meta::RecordedConfigHome::Unknown
        | crate::meta::RecordedConfigHome::Invalid => None,
    }
}

/// The caller's own `HOME`, where every tool keeps its conversation history.
fn home_dir() -> Option<PathBuf> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the tool history directories a capture scans live under the caller's HOME — see clippy.toml"
    )]
    let raw = std::env::var_os("HOME");
    raw.filter(|value| !value.is_empty()).map(PathBuf::from)
}

/// Publish a captured id only while the exact observed launch still owns the
/// slot and its id remains pending. The compare and write share one meta lock.
#[must_use]
pub fn commit(dir: &Path, slot: &str, captured: &Captured) -> bool {
    commit_inner(dir, slot, captured, false)
}

/// Publish codex's token-proven handshake even when a scan recorded a wrong id
/// earlier in this same launch.
fn commit_authoritative(dir: &Path, slot: &str, captured: &Captured) -> bool {
    if captured.launch_id.is_empty() {
        return false;
    }
    commit_inner(dir, slot, captured, true)
}

/// The launch-bound compare and write shared by ordinary and authoritative
/// capture paths.
fn commit_inner(dir: &Path, slot: &str, captured: &Captured, may_replace: bool) -> bool {
    if captured.id.is_empty() || captured.id.chars().any(char::is_control) {
        return false;
    }
    let Ok(_held) = crate::meta::lock(dir) else {
        return false;
    };
    let Ok(bytes) = crate::meta::read_bytes(dir) else {
        return false;
    };
    let Ok((now_dir, now_prov)) = crate::meta::effective_seat_dir(&bytes, slot) else {
        return false;
    };
    if now_dir != captured.work_dir || now_prov != captured.provenance {
        return false;
    }
    let Ok(text) = String::from_utf8(bytes) else {
        return false;
    };
    let parsed = crate::meta::Meta::parse(&text);
    if parsed.schema() != Some("2") {
        return false;
    }
    let Some(entry) = parsed.roster().iter().find(|entry| entry.slot == slot) else {
        return false;
    };
    let current_tool = ToolKind::from_binary_name(entry.binary.as_deref().unwrap_or_default());
    if entry.name != captured.agent
        || current_tool != captured.tool
        || (!may_replace && !is_pending(entry.harness_session.as_deref()))
    {
        return false;
    }
    let launch_key = format!("launch_id.{slot}");
    let first_launch = crate::meta::first_value(text.as_bytes(), &launch_key);
    let sole_launch = crate::meta::sole_value(text.as_bytes(), &launch_key);
    if first_launch.is_some() && sole_launch.is_none() {
        return false;
    }
    let current_launch = sole_launch
        .map(String::from_utf8_lossy)
        .map(std::borrow::Cow::into_owned)
        .unwrap_or_default();
    if current_launch != captured.launch_id {
        return false;
    }
    if matches!(captured.tool.adapter().capture, CaptureSpec::SessionList)
        && !opencode_recheck(text.as_bytes(), slot, captured)
    {
        return false;
    }
    // Writer A: an authoritative capture may REPLACE a live id. The replaced id
    // becomes the seat's newest predecessor and both rows land in ONE
    // publication — unless it is `pending`/empty, not a lowercase UUID, or the
    // very id being recorded (an exact codex resume re-registers one
    // conversation).
    let old = entry.harness_session.as_deref().unwrap_or_default();
    let mut next = text;
    // A capture never crosses tools: the id it replaces was recorded by the
    // binary this slot still names, so that is the tag the whole row takes.
    let tool = entry.binary.as_deref().unwrap_or_default();
    if old != captured.id
        && let Some(list) = crate::meta::prior_with(&parsed.harness_session_prior(slot), old, tool)
    {
        next = crate::meta::rewritten(
            &next,
            &format!("{}{}", crate::meta::HARNESS_SESSION_PRIOR_PREFIX, slot),
            Some(&list),
        );
    }
    let next = crate::meta::rewritten(
        &next,
        &format!("harness_session.{slot}"),
        Some(&captured.id),
    );
    let published = match crate::meta::publish_locked(dir, &next) {
        Ok(()) | Err(crate::meta::RewriteError::Unknown(_)) => true,
        Err(crate::meta::RewriteError::NotWritten(_)) => false,
    };
    if published && captured.tool.adapter().launch.initial_turn == InitialTurn::RegisterSessionId {
        let _ = std::fs::remove_file(sid_file(dir, slot));
    }
    published
}

/// The opencode attribution rechecked under the meta lock: the scan's snapshot
/// is stale by the time the list returns, so no other slot may record this id
/// and no other pending opencode sibling's window may cover its birth. One
/// owner with the scan: the same sibling set and the same covers predicate. A
/// capture without a birth refuses: there is nothing to compare.
fn opencode_recheck(bytes: &[u8], slot: &str, captured: &Captured) -> bool {
    let Some(born) = captured.born_ms else {
        return false;
    };
    if excluded_from(bytes).iter().any(|held| held == &captured.id) {
        return false;
    }
    let target = canonical(&captured.work_dir);
    !siblings_from(bytes, slot)
        .iter()
        .any(|sibling| sibling.covers(&target, born))
}

// ---------------------------------------------------------------------------
// codex
// ---------------------------------------------------------------------------

/// The path codex's own `_register-sid` handshake writes to.
fn sid_file(dir: &Path, slot: &str) -> PathBuf {
    dir.join(format!("codex.{slot}.sid"))
}

/// `_register-sid <meta-dir> <slot> [<session-id>]` — codex's own handshake.
///
/// # Errors
///
/// Only a failure to write `out` or `err`. Every refusal is an exit code: `2`
/// for a usage error or a malformed id, `1` for a scan that matched nothing.
pub fn register_sid(
    dir: &Path,
    slot: &str,
    id: Option<&str>,
    out: &mut impl std::io::Write,
    err: &mut impl std::io::Write,
) -> crate::Result<u8> {
    if slot.is_empty() {
        writeln!(err, "{REGISTER_SID_USAGE}")?;
        return Ok(crate::state::EXIT_USAGE);
    }
    let Some(facts) = facts(dir, slot)
        .filter(|facts| facts.tool.adapter().launch.initial_turn == InitialTurn::RegisterSessionId)
    else {
        writeln!(
            err,
            "Error: seat '{slot}' is not a codex seat in {}.",
            dir.display()
        )?;
        return Ok(crate::state::EXIT_USAGE);
    };
    let given = if let Some(given) = id {
        let given = given.trim();
        if !is_lowercase_uuid(given) {
            writeln!(
                err,
                "Error: '{given}' is not a lowercase UUID — a session id is 8-4-4-4-12 hex."
            )?;
            return Ok(crate::state::EXIT_USAGE);
        }
        Some(given)
    } else {
        None
    };
    let ambient_home = home_dir();
    let Some(found) = codex_config_home(&facts, ambient_home.as_deref())
        .as_deref()
        .and_then(|config_home| scan_codex(config_home, &facts))
        .filter(|found| given.is_none_or(|given| found == given))
    else {
        writeln!(err, "No codex session matched seat '{slot}' yet.")?;
        return Ok(crate::state::EXIT_FAILED);
    };
    let file = sid_file(dir, slot);
    if let Err(why) = super::assets::publish_document(&file, &format!("{found}\n")) {
        writeln!(err, "Error: could not write {} ({why})", file.display())?;
        return Ok(crate::state::EXIT_FAILED);
    }
    let captured = Captured::new(&facts, found);
    let committed = if captured.launch_id.is_empty() {
        commit(dir, slot, &captured)
    } else {
        // `scan_codex` takes the token-only arm when this value is nonempty,
        // so replacement authority is backed by positive rollout provenance.
        commit_authoritative(dir, slot, &captured)
    };
    if !committed {
        writeln!(
            err,
            "Error: seat '{slot}' changed before its session id could be recorded."
        )?;
        return Ok(crate::state::EXIT_FAILED);
    }
    writeln!(out, "Registered session id for '{slot}'.")?;
    Ok(0)
}

/// The refusal `_register-sid` prints for a missing seat.
pub const REGISTER_SID_USAGE: &str = "Usage: _register-sid <meta-dir> <slot> [<session-id>]";

/// Whether `value` is a lowercase 8-4-4-4-12 hex UUID — the Muse session-id
/// grammar, shared with the board's Muse seat.
#[must_use]
pub(crate) fn is_lowercase_uuid(value: &str) -> bool {
    let groups = [8, 4, 4, 4, 12];
    let mut parts = value.split('-');
    for width in groups {
        let Some(part) = parts.next() else {
            return false;
        };
        if part.len() != width
            || !part
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return false;
        }
    }
    parts.next().is_none()
}

/// Poll for the self-registered id, verify it against the rollout carrying
/// this launch's token, then scan that rollout directly. Only a legacy seat
/// with no launch token may use the TUI fallback.
fn capture_codex(
    dir: &Path,
    slot: &str,
    pane: &str,
    server: &ServerId,
    config_home: Option<&Path>,
    facts: &Facts,
) -> Option<String> {
    let file = sid_file(dir, slot);
    for _ in 0..POLLS {
        std::thread::sleep(POLL);
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: reads the id codex's own _register-sid handshake wrote — see clippy.toml"
        )]
        let read = std::fs::read_to_string(&file);
        if let Ok(text) = read {
            let id: String = text.chars().filter(|c| !c.is_whitespace()).collect();
            let verified = config_home
                .and_then(|home| scan_codex(home, facts))
                .is_some_and(|found| found == id);
            if verified {
                return Some(id);
            }
        }
        if let Some(id) = config_home.and_then(|home| scan_codex(home, facts)) {
            return Some(id);
        }
    }
    // The TUI scrape, least reliable and therefore last: codex prints
    // `session id: <uuid>` once in its header.
    let read = crate::transport::capture_pane;
    codex_tui_fallback(facts.provenance, &facts.launch_id, pane, server, read)
}

/// The codex TUI fallback behind a pane-reader seam: production passes the
/// live pane reader, the pin a counting double. Only a legacy seat is scraped.
fn codex_tui_fallback(
    provenance: crate::meta::SeatProvenance,
    launch_id: &str,
    pane: &str,
    server: &ServerId,
    read: impl Fn(&ServerId, &str) -> Option<String>,
) -> Option<String> {
    if crate::meta::explicit_token_only(provenance) || !launch_id.is_empty() {
        return None;
    }
    scrape_session_id(&read(server, pane)?)
}

/// One look through codex's own history. A seat with a launch token accepts
/// only that positive proof; the cwd fallback exists for legacy seats alone.
fn scan_codex(config_home: &Path, facts: &Facts) -> Option<String> {
    if !facts.launch_id.is_empty() {
        let marker = facts.launch_marker?;
        return find_codex_by_launch_id(config_home, marker, &facts.launch_id, facts.capture_floor);
    }
    if facts.work_dir.is_empty() {
        return None;
    }
    if crate::meta::explicit_token_only(facts.provenance) {
        return None;
    }
    let days = day_dirs(Timestamp::now());
    find_codex_by_cwd(config_home, &facts.work_dir, facts.capture_floor, &days)
}

/// The first `session id: <hex-and-dashes>` a screen carries.
pub(crate) fn scrape_session_id(screen: &str) -> Option<String> {
    for line in screen.lines() {
        let Some(rest) = line.split("session id: ").nth(1) else {
            continue;
        };
        let id: String = rest
            .chars()
            .take_while(|c| c.is_ascii_hexdigit() || *c == '-')
            .collect();
        if !id.is_empty() {
            return Some(id);
        }
    }
    None
}

/// The newest codex session whose log carries this launch token.
#[must_use]
pub(crate) fn find_codex_by_launch_id(
    config_home: &Path,
    marker_prefix: &str,
    launch_id: &str,
    capture_floor: i64,
) -> Option<String> {
    let days = codex_token_day_dirs(Timestamp::now(), capture_floor);
    find_codex_by_launch_id_in_days(config_home, marker_prefix, launch_id, capture_floor, &days)
}

/// The token lookup within an already resolved set of UTC partitions.
fn find_codex_by_launch_id_in_days(
    config_home: &Path,
    marker_prefix: &str,
    launch_id: &str,
    capture_floor: i64,
    days: &[String],
) -> Option<String> {
    let marker = format!("AE_{marker_prefix}_LAUNCH_ID={launch_id}");
    newest(codex_logs(config_home, days), capture_floor, |text| {
        if !codex_started_since_floor(text, capture_floor) || !text.contains(&marker) {
            return None;
        }
        first_hex_field(text.lines().next().unwrap_or_default(), "id")
    })
}

/// The newest codex session whose recorded `cwd` is this working directory.
#[must_use]
pub(crate) fn find_codex_by_cwd(
    config_home: &Path,
    work_dir: &str,
    capture_floor: i64,
    days: &[String],
) -> Option<String> {
    let target = canonical(work_dir);
    newest(codex_logs(config_home, days), capture_floor, |text| {
        if !codex_started_since_floor(text, capture_floor) {
            return None;
        }
        let first = text.lines().next().unwrap_or_default();
        let cwd = first_string_field(first, "cwd")?;
        if canonical(&cwd) != target {
            return None;
        }
        first_hex_field(first, "id")
    })
}

/// Whether a codex rollout's own creation timestamp belongs to this launch.
/// File mtime is not identity: a live older rollout keeps changing as codex
/// appends turns to it.
fn codex_started_since_floor(text: &str, capture_floor: i64) -> bool {
    if capture_floor <= 0 {
        return true;
    }
    text.lines()
        .next()
        .and_then(|first| first_string_field(first, "timestamp"))
        .as_deref()
        .and_then(crate::quota::vendor_timestamp)
        .is_some_and(|started| started >= capture_floor)
}

/// Every `*.jsonl` under the named day directories of a Codex config home.
fn codex_logs(config_home: &Path, days: &[String]) -> Vec<PathBuf> {
    let root = config_home.join("sessions");
    let mut found = Vec::new();
    for day in days {
        if day.is_empty() {
            continue;
        }
        found.extend(
            entries(&root.join(day))
                .into_iter()
                .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl")),
        );
    }
    found
}

// ---------------------------------------------------------------------------
// Muse Code
// ---------------------------------------------------------------------------

/// Muse's dated session store, relative to the caller's `HOME`.
pub const MUSE_SESSIONS: &str = ".local/share/muse/sessions";

/// Poll Muse's dated session directories for the token that belongs to this
/// seat. The session id is the directory basename, so the log stays opaque.
fn capture_muse(home: &Path, facts: &Facts) -> Option<String> {
    for attempt in 0..POLLS {
        if attempt > 0 {
            std::thread::sleep(POLL);
        }
        if let Some(id) = scan_muse(home, facts) {
            return Some(id);
        }
    }
    None
}

/// One token-only Muse scan. A missing token or a token miss stays pending:
/// the newest directory is not evidence that it belongs to this seat.
fn scan_muse(home: &Path, facts: &Facts) -> Option<String> {
    let marker = facts.launch_marker?;
    if facts.launch_id.is_empty() {
        return None;
    }
    find_muse_by_launch_id(home, marker, &facts.launch_id, facts.capture_floor)
}

/// The one dated Muse session directory whose log carries this launch token.
///
/// `session.jsonl` has nested encoded records, but the launch token is a byte
/// sequence within that file. The directory name is the documented session id,
/// so no JSON field is decoded or trusted here.
#[must_use]
pub(crate) fn find_muse_by_launch_id(
    home: &Path,
    marker_prefix: &str,
    launch_id: &str,
    capture_floor: i64,
) -> Option<String> {
    let marker = format!("AE_{marker_prefix}_LAUNCH_ID={launch_id}").into_bytes();
    let days = codex_token_day_dirs(Timestamp::now(), capture_floor);
    let root = home.join(MUSE_SESSIONS);
    let mut found: Option<String> = None;
    for day in days {
        let mut candidates = entries(&root.join(day));
        candidates.sort();
        for candidate in candidates {
            let Some(id) = candidate
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|id| is_lowercase_uuid(id))
                .map(ToOwned::to_owned)
            else {
                continue;
            };
            if !file_contains(&candidate.join("session.jsonl"), &marker) {
                continue;
            }
            // A duplicated token has no unique directory proof. Staying
            // pending is safer than attaching this seat to either log.
            if found.is_some() {
                return None;
            }
            found = Some(id);
        }
    }
    found
}

/// The board's Muse transcript: `<day>/<id>/session.jsonl` under
/// [`MUSE_SESSIONS`] by directory basename, no token scan. Every enumeration
/// goes through the `entries` door and claims the budget (`QUOTA_MAX_FILES`
/// bounds the walk); two hits refuse. The caller validated the id.
pub(crate) fn find_muse_session_file(
    home: &Path,
    id: &str,
    budget: &mut crate::quota::Budget,
) -> Result<Option<PathBuf>, String> {
    if !budget.claim_file() {
        return Err("transcript scan truncated".to_owned());
    }
    let mut found: Option<PathBuf> = None;
    for year in entries(&home.join(MUSE_SESSIONS)) {
        if !budget.claim_file() {
            return Err("transcript scan truncated".to_owned());
        }
        for month in entries(&year) {
            if !budget.claim_file() {
                return Err("transcript scan truncated".to_owned());
            }
            for day in entries(&month) {
                // Two claims: the day visit and its id probe below.
                if !budget.claim_file() || !budget.claim_file() {
                    return Err("transcript scan truncated".to_owned());
                }
                let under = entries(&day.join(id));
                let present = under.iter().any(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name == "session.jsonl")
                });
                if !present {
                    continue;
                }
                if found.is_some() {
                    return Err("conversation id is not unique".to_owned());
                }
                found = Some(day.join(id).join("session.jsonl"));
            }
        }
    }
    Ok(found)
}

// ---------------------------------------------------------------------------
// gemini
// ---------------------------------------------------------------------------

/// Poll gemini's local chat history: the launch token first, the project root
/// alone as the fallback.
fn capture_gemini(home: &Path, facts: &Facts) -> Option<String> {
    for attempt in 0..POLLS {
        if attempt > 0 {
            std::thread::sleep(POLL);
        }
        if let Some(id) = scan_gemini(home, facts) {
            return Some(id);
        }
    }
    None
}

/// One look through gemini's chat history for this project: the launch token
/// first, the project root alone second.
fn scan_gemini(home: &Path, facts: &Facts) -> Option<String> {
    if facts.work_dir.is_empty() {
        return None;
    }
    if !facts.launch_id.is_empty()
        && let Some(marker) = facts.launch_marker
        && let Some(id) = find_gemini_by_launch_id(
            home,
            &facts.work_dir,
            marker,
            &facts.launch_id,
            facts.capture_floor,
        )
    {
        return Some(id);
    }
    if crate::meta::explicit_token_only(facts.provenance) {
        return None;
    }
    find_gemini_by_cwd(home, &facts.work_dir, facts.capture_floor)
}

/// The newest gemini chat for this project whose file carries the launch token.
#[must_use]
pub(crate) fn find_gemini_by_launch_id(
    home: &Path,
    work_dir: &str,
    marker_prefix: &str,
    launch_id: &str,
    capture_floor: i64,
) -> Option<String> {
    let marker = format!("AE_{marker_prefix}_LAUNCH_ID={launch_id}");
    newest(gemini_chats(home, work_dir), capture_floor, |text| {
        if !text.contains(&marker) {
            return None;
        }
        first_string_field(text, "sessionId")
    })
}

/// The newest gemini chat for this project, whichever launch wrote it.
#[must_use]
pub(crate) fn find_gemini_by_cwd(
    home: &Path,
    work_dir: &str,
    capture_floor: i64,
) -> Option<String> {
    newest(gemini_chats(home, work_dir), capture_floor, |text| {
        first_string_field(text, "sessionId")
    })
}

/// Every `chats/session-*.json` under a `~/.gemini/tmp/<project>` whose
/// `.project_root` names this working directory.
fn gemini_chats(home: &Path, work_dir: &str) -> Vec<PathBuf> {
    let target = canonical(work_dir);
    let mut found = Vec::new();
    for project in entries(&home.join(".gemini").join("tmp")) {
        let Some(root) = read_text(&project.join(".project_root")) else {
            continue;
        };
        if canonical(root.trim_end_matches('\n')) != target {
            continue;
        }
        // The glob is `session-*.json` and both halves are case-sensitive: the
        // extension is compared as a path component rather than as a string
        // suffix, so it stays exactly that.
        found.extend(entries(&project.join("chats")).into_iter().filter(|path| {
            path.extension().is_some_and(|ext| ext == "json")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("session-"))
        }));
    }
    found
}

// ---------------------------------------------------------------------------
// agy (Antigravity CLI)
// ---------------------------------------------------------------------------

/// agy's conversation store, relative to the caller's `HOME`.
pub const AGY_CONVERSATIONS: &str = ".gemini/antigravity-cli/conversations";

/// agy's per-process CLI log directory, relative to the caller's `HOME`.
pub(crate) const AGY_LOGS: &str = ".gemini/antigravity-cli/log";

/// Poll agy's conversation store: the launch token first, its CLI log second.
fn capture_agy(home: &Path, facts: &Facts) -> Option<String> {
    for attempt in 0..POLLS {
        if attempt > 0 {
            std::thread::sleep(POLL);
        }
        if let Some(id) = scan_agy(home, facts) {
            return Some(id);
        }
    }
    None
}

/// One look for this seat's agy conversation.
///
/// The two halves are ALTERNATIVES chosen by whether the seat has a launch
/// token, never a chain: a token miss stays PENDING. Falling through to the
/// workspace search once gave two agy seats in one working directory a single
/// positive answer between them, and the seat that had not yet written its
/// token registered its sibling's conversation.
fn scan_agy(home: &Path, facts: &Facts) -> Option<String> {
    if !facts.launch_id.is_empty() {
        return facts.launch_marker.and_then(|marker| {
            find_agy_by_launch_id(home, marker, &facts.launch_id, facts.capture_floor)
        });
    }
    if facts.work_dir.is_empty() {
        return None;
    }
    if crate::meta::explicit_token_only(facts.provenance) {
        return None;
    }
    find_agy_by_cwd(home, &facts.work_dir, facts.capture_floor)
}

/// The newest agy conversation whose database carries the launch token.
#[must_use]
pub(crate) fn find_agy_by_launch_id(
    home: &Path,
    marker_prefix: &str,
    launch_id: &str,
    capture_floor: i64,
) -> Option<String> {
    let marker = format!("AE_{marker_prefix}_LAUNCH_ID={launch_id}").into_bytes();
    let mut best: Option<(i64, String)> = None;
    let mut candidates = agy_conversations(home);
    candidates.sort();
    for path in candidates {
        let Some(at) = mtime(&path) else {
            continue;
        };
        if at < capture_floor {
            continue;
        }
        if best.as_ref().is_some_and(|(seen, _)| at <= *seen) {
            continue;
        }
        let Some(id) = agy_conversation_id(&path) else {
            continue;
        };
        if file_contains(&path, &marker) {
            best = Some((at, id));
        }
    }
    best.map(|(_, found)| found)
}

/// The ONE conversation an agy run in this working directory created, read out
/// of agy's own CLI log — or nothing, when there is more than one.
#[must_use]
pub(crate) fn find_agy_by_cwd(home: &Path, work_dir: &str, capture_floor: i64) -> Option<String> {
    let target = canonical(work_dir);
    let mut candidates: Vec<String> = Vec::new();
    for path in agy_logs(home) {
        let Some(at) = mtime(&path) else {
            continue;
        };
        if at < capture_floor {
            continue;
        }
        let Some(text) = read_text(&path) else {
            continue;
        };
        if !agy_log_workspace_matches(&text, &target) {
            continue;
        }
        let Some(id) = agy_log_created_conversation(&text) else {
            continue;
        };
        if !candidates.contains(&id) {
            candidates.push(id);
        }
        // Two is already the answer, and reading further logs cannot make it
        // fewer.
        if candidates.len() > 1 {
            return None;
        }
    }
    candidates.pop()
}

/// Does this log's `workspaceDirs=[…]` name `target`?
fn agy_log_workspace_matches(text: &str, target: &str) -> bool {
    let mut rest = text;
    while let Some(at) = rest.find("workspaceDirs=[") {
        let after = &rest[at + "workspaceDirs=[".len()..];
        let Some(end) = after.find(']') else {
            return false;
        };
        if after[..end]
            .split_whitespace()
            .any(|dir| canonical(dir) == target)
        {
            return true;
        }
        rest = &after[end..];
    }
    false
}

/// The first `Created conversation <id>` in a CLI log.
fn agy_log_created_conversation(text: &str) -> Option<String> {
    const KEY: &str = "Created conversation ";
    let at = text.find(KEY)?;
    let id: String = text[at + KEY.len()..]
        .chars()
        .take_while(|ch| ch.is_ascii_hexdigit() || *ch == '-')
        .collect();
    (!id.is_empty()).then_some(id)
}

/// Every `<id>.db` in agy's conversation store.
fn agy_conversations(home: &Path) -> Vec<PathBuf> {
    entries(&home.join(AGY_CONVERSATIONS))
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "db"))
        .collect()
}

/// A conversation database's id: its file stem, when that reads like one.
fn agy_conversation_id(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let looks_like_an_id = !stem.is_empty()
        && stem
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-');
    looks_like_an_id.then(|| stem.to_owned())
}

/// Every `cli-*.log` in agy's log directory.
fn agy_logs(home: &Path) -> Vec<PathBuf> {
    entries(&home.join(AGY_LOGS))
        .into_iter()
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "log")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("cli-"))
        })
        .collect()
}

/// The most any tool log is worth scanning for a launch token.
const TOKEN_SCAN_CAP: u64 = 16 * 1024 * 1024;

/// How much one token scan holds at once.
const TOKEN_SCAN_CHUNK: usize = 64 * 1024;

/// Does `path` contain `needle`, reading at most [`TOKEN_SCAN_CAP`] bytes?
fn file_contains(path: &Path, needle: &[u8]) -> bool {
    use std::io::Read as _;

    // ONE stat, TWO facts, and both are decided BEFORE the open.
    let Some((regular, size)) = file_facts(path) else {
        return false;
    };
    if !regular {
        return false;
    }
    if size > TOKEN_SCAN_CAP {
        skipped(path, size);
        return false;
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: a capture reads the tool's own conversation store, which is binary and unbounded — see clippy.toml"
    )]
    let opened = std::fs::File::open(path);
    let Ok(file) = opened else {
        return false;
    };
    // THE STAT IS NOT THE BOUND. A conversation is a LIVE database and can
    // grow between the stat above and the last read below, so `take` is the
    // bound that holds whatever the file does.
    match scan_stream(file.take(TOKEN_SCAN_CAP.saturating_add(1)), needle) {
        Scan::Found => true,
        Scan::Absent => false,
        Scan::OverCap => {
            skipped(path, TOKEN_SCAN_CAP.saturating_add(1));
            false
        }
    }
}

/// What a bounded search of one stream found.
#[derive(Debug, PartialEq, Eq)]
enum Scan {
    /// The needle is in the bytes read.
    Found,
    /// The stream ended without it.
    Absent,
    /// The budget ran out first, so the answer is unknown and not "no".
    OverCap,
}

/// Search `reader` for `needle` in chunks, spending at most [`TOKEN_SCAN_CAP`]
/// bytes.
fn scan_stream<R: std::io::Read>(mut reader: R, needle: &[u8]) -> Scan {
    let Some(overlap) = needle.len().checked_sub(1) else {
        return Scan::Absent;
    };
    let mut buffer = vec![0_u8; overlap + TOKEN_SCAN_CHUNK];
    // Starts EMPTY, not at `overlap`: seeding the carry with the buffer's own
    // zero fill would put bytes the stream does not contain in front of its
    // first chunk, and a needle is matched against real bytes or nothing.
    let mut filled = 0_usize;
    let mut consumed = 0_u64;
    loop {
        let Ok(read) = reader.read(&mut buffer[filled..]) else {
            return Scan::Absent;
        };
        if read == 0 {
            return Scan::Absent;
        }
        consumed = consumed.saturating_add(read as u64);
        if consumed > TOKEN_SCAN_CAP {
            return Scan::OverCap;
        }
        filled += read;
        if buffer[..filled]
            .windows(needle.len())
            .any(|window| window == needle)
        {
            return Scan::Found;
        }
        // Carry the tail forward: the next chunk is read BEHIND it, so a needle
        // straddling the seam sits contiguously in the next pass.
        if filled > overlap {
            buffer.copy_within(filled - overlap..filled, 0);
            filled = overlap;
        }
    }
}

/// Say that a conversation was too big to search, and where.
fn skipped(path: &Path, size: u64) {
    eprintln!(
        "ae: capture skipped {} ({size} bytes over the {TOKEN_SCAN_CAP}-byte scan cap)",
        path.display()
    );
}

/// Whether `path` is a regular file, and how long it is — from ONE stat, and
/// without opening the node.
fn file_facts(path: &Path) -> Option<(bool, u64)> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the node classification and scan cap that keep a FIFO and an unbounded conversation store off the watchdog's cycle — see clippy.toml"
    )]
    let read = std::fs::metadata(path);
    let meta = read.ok()?;
    Some((meta.is_file(), meta.len()))
}

// ---------------------------------------------------------------------------
// opencode
// ---------------------------------------------------------------------------

/// Poll `opencode session list` for a session attributable to this seat alone.
fn capture_opencode(dir: &Path, slot: &str, facts: &Facts) -> Option<(String, i64)> {
    for attempt in 0..POLLS {
        if attempt > 0 {
            std::thread::sleep(POLL);
        }
        // Each poll re-reads the meta: a sibling that captured since the last
        // look drops out of the next attribution.
        let Ok(bytes) = crate::meta::read_bytes(dir) else {
            return None;
        };
        if let Some(found) = scan_opencode(&bytes, slot, facts) {
            return Some(found);
        }
    }
    None
}

/// One `opencode session list`, read for a session attributable to this seat
/// alone: recorded ids are invisible and no pending sibling's window may cover
/// it. Without the sibling set there is no attribution, only a refusal.
fn scan_opencode(bytes: &[u8], slot: &str, facts: &Facts) -> Option<(String, i64)> {
    if facts.work_dir.is_empty() {
        return None;
    }
    // opencode timestamps are MILLISECONDS.
    let since = facts.capture_floor.saturating_mul(1000);
    // A failed run is "no answer", never an empty one: opencode may not be
    // installed at all.
    let (ran, listed) = crate::transport::run_opencode(&opencode_list_argv(), OPENCODE_LIST_CAP);
    if !ran {
        return None;
    }
    attribute_opencode(
        &listed,
        &facts.work_dir,
        since,
        &siblings_from(bytes, slot),
        &excluded_from(bytes),
    )
}

/// Every listed session with an id, a directory and an immutable birth, in
/// list order. The attribution judges them; this only extracts.
fn opencode_candidates(listed: &str) -> Vec<(String, String, i64)> {
    let mut out = Vec::new();
    for record in json_records(listed) {
        let Some(id) = first_string_field(record, "id") else {
            continue;
        };
        let Some(directory) = first_string_field(record, "directory") else {
            continue;
        };
        let Some(created) = first_num_field(record, "created") else {
            continue;
        };
        out.push((id, directory, created));
    }
    out
}

/// Another pending opencode seat of this session — a rival claimant.
struct Sibling {
    /// Canonical work dir, or `None` when the row is unusable: an unusable
    /// sibling is assumed to share this seat's dir (fail closed).
    dir: Option<String>,
    /// The oldest birth this sibling may accept, in milliseconds.
    floor_ms: i64,
}

impl Sibling {
    /// Whether this sibling's window covers a candidate born at `born` in the
    /// seat's own canonical dir.
    fn covers(&self, target: &str, born: i64) -> bool {
        self.dir.as_deref().is_none_or(|dir| dir == target) && self.floor_ms <= born
    }
}

/// The newest-born session in `listed` attributable to this seat alone: its
/// `directory` is `work_dir`, its immutable `created` is at or after `since`
/// (milliseconds), no recorded id excludes it, and no pending opencode
/// sibling's window covers it. `created` is both the launch-safety proof and
/// rank, so last-touched time cannot influence identity; an equal birth
/// timestamp breaks by greatest id. Excluded candidates are invisible to both
/// the attribution and the ambiguity: a seat whose every covering candidate is
/// recorded elsewhere, or also covered, captures nothing.
#[must_use]
fn attribute_opencode(
    listed: &str,
    work_dir: &str,
    since: i64,
    siblings: &[Sibling],
    excluded: &[String],
) -> Option<(String, i64)> {
    let target = canonical(work_dir);
    let mut best: Option<(i64, String)> = None;
    for (id, directory, created) in opencode_candidates(listed) {
        if created < since || canonical(&directory) != target {
            continue;
        }
        if excluded.iter().any(|held| held == &id) {
            continue;
        }
        if siblings
            .iter()
            .any(|sibling| sibling.covers(&target, created))
        {
            continue;
        }
        if best.as_ref().is_none_or(|(seen, best_id)| {
            created > *seen || (created == *seen && id.as_str() > best_id.as_str())
        }) {
            best = Some((created, id));
        }
    }
    best.map(|(born, id)| (id, born))
}

/// Split a JSON array of objects into its records, on the `},{` boundary.
fn json_records(listed: &str) -> Vec<&str> {
    let folded: String = listed
        .chars()
        .map(|ch| if ch == '\n' || ch == '\r' { ' ' } else { ch })
        .collect();
    // Boundaries are found on the FOLDED text but sliced out of the original:
    // folding replaces one char with one char, so the byte offsets agree.
    let bytes = folded.as_bytes();
    let mut cuts = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'}' {
            at += 1;
            continue;
        }
        let mut probe = at + 1;
        while bytes.get(probe).is_some_and(u8::is_ascii_whitespace) {
            probe += 1;
        }
        if bytes.get(probe) != Some(&b',') {
            at += 1;
            continue;
        }
        probe += 1;
        while bytes.get(probe).is_some_and(u8::is_ascii_whitespace) {
            probe += 1;
        }
        if bytes.get(probe) != Some(&b'{') {
            at += 1;
            continue;
        }
        cuts.push((at + 1, probe));
        at = probe;
    }
    let mut records = Vec::new();
    let mut start = 0;
    for (end, next) in cuts {
        records.push(&listed[start..end]);
        start = next;
    }
    records.push(&listed[start..]);
    records
}

// ---------------------------------------------------------------------------
// the shared scan primitives
// ---------------------------------------------------------------------------

/// Today and yesterday as `YYYY/MM/DD` in UTC — the day-partitioned layout
/// codex uses.
fn day_dirs(now: Timestamp) -> Vec<String> {
    [
        now,
        Timestamp::from_epoch(now.epoch().saturating_sub(SECONDS_PER_DAY)),
    ]
    .iter()
    .map(|at| day_dir(*at))
    .collect()
}

/// Every UTC Codex partition from a known capture birth through today. A
/// legacy retained seat with no birth is bounded to thirty partitions; its
/// launch token remains the positive identity proof within that range.
fn codex_token_day_dirs(now: Timestamp, capture_floor: i64) -> Vec<String> {
    let today = now.epoch().div_euclid(SECONDS_PER_DAY);
    let first = if capture_floor > 0 {
        capture_floor.div_euclid(SECONDS_PER_DAY)
    } else {
        today.saturating_sub(CODEX_UNKNOWN_FLOOR_DAYS - 1)
    };
    if first > today {
        return Vec::new();
    }
    (first..=today)
        .map(|day| day_dir(Timestamp::from_epoch(day.saturating_mul(SECONDS_PER_DAY))))
        .collect()
}

/// One UTC day in Codex's `YYYY/MM/DD` partition spelling.
fn day_dir(at: Timestamp) -> String {
    at.to_string()
        .get(..10)
        .map(|day| day.replace('-', "/"))
        .unwrap_or_default()
}

/// The candidate with the greatest mtime whose text `read` accepts.
fn newest<F>(mut candidates: Vec<PathBuf>, capture_floor: i64, read: F) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    candidates.sort();
    let mut best: Option<(i64, String)> = None;
    for path in candidates {
        let Some(at) = mtime(&path) else {
            continue;
        };
        if at < capture_floor {
            continue;
        }
        if best.as_ref().is_some_and(|(seen, _)| at <= *seen) {
            continue;
        }
        let Some(text) = read_text(&path) else {
            continue;
        };
        if let Some(found) = read(&text) {
            best = Some((at, found));
        }
    }
    best.map(|(_, found)| found)
}

/// Every direct child of `dir`, or nothing when it cannot be listed.
fn entries(dir: &Path) -> Vec<PathBuf> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: a capture scans the tool's own history directory — see clippy.toml"
    )]
    let read = std::fs::read_dir(dir);
    let Ok(listing) = read else {
        return Vec::new();
    };
    listing
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect()
}

/// One file's text, or nothing when it cannot be read as UTF-8.
fn read_text(path: &Path) -> Option<String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: a capture reads the tool's own session log — see clippy.toml"
    )]
    let read = std::fs::read_to_string(path);
    read.ok()
}

/// One file's mtime in epoch seconds, or nothing when it has none.
fn mtime(path: &Path) -> Option<i64> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the launch-time filter that keeps a stale conversation out of a capture — see clippy.toml"
    )]
    let read = std::fs::metadata(path);
    let at = read.ok()?.modified().ok()?;
    let since = at.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(i64::try_from(since.as_secs()).unwrap_or(i64::MAX))
}

/// One directory spelling, canonicalised, falling back to its raw form.
fn canonical(path: &str) -> String {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: every cwd match compares canonical directories — see clippy.toml"
    )]
    let resolved = std::fs::canonicalize(path);
    match resolved {
        Ok(resolved) => resolved.display().to_string(),
        Err(_) => path.trim_end_matches('/').to_owned(),
    }
}

/// The first `"key": "<value>"` in `text`, the value being any run of
/// non-quote bytes.
#[must_use]
pub(crate) fn first_string_field(text: &str, key: &str) -> Option<String> {
    first_field(text, key, |_| true)
}

/// The first `"key": "<value>"` in `text` whose value is only hex digits and
/// dashes.
#[must_use]
pub(crate) fn first_hex_field(text: &str, key: &str) -> Option<String> {
    first_field(text, key, |ch| {
        ch.is_ascii_digit() || matches!(ch, 'a'..='f' | '-')
    })
}

/// The first `"key": <digits>` in `text`.
#[must_use]
pub(crate) fn first_num_field(text: &str, key: &str) -> Option<i64> {
    let quoted = format!("\"{key}\"");
    let mut from = 0;
    while let Some(hit) = text[from..].find(&quoted) {
        let after = from + hit + quoted.len();
        from = after;
        let Some(rest) = separator(&text[after..]) else {
            continue;
        };
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty()
            && let Ok(value) = digits.parse()
        {
            return Some(value);
        }
    }
    None
}

/// The shared scanner behind the two string readers.
fn first_field<F>(text: &str, key: &str, allowed: F) -> Option<String>
where
    F: Fn(char) -> bool,
{
    let quoted = format!("\"{key}\"");
    let mut from = 0;
    while let Some(hit) = text[from..].find(&quoted) {
        let after = from + hit + quoted.len();
        from = after;
        let Some(rest) = separator(&text[after..]) else {
            continue;
        };
        let Some(open) = rest.strip_prefix('"') else {
            continue;
        };
        let value: String = open.chars().take_while(|ch| *ch != '"').collect();
        // The whole value must be in class, and the closing quote must be
        // there: a truncated line is not a match.
        if open.len() > value.len() && value.chars().all(&allowed) {
            return Some(value);
        }
    }
    None
}

/// What follows a key's `:` separator, or nothing when the key is not followed
/// by one.
fn separator(after: &str) -> Option<&str> {
    let rest = after.trim_start_matches([' ', '\t']);
    let rest = rest.strip_prefix(':')?;
    Some(rest.trim_start_matches([' ', '\t']))
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about \
              what PRODUCT code may reach"
)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A scratch directory, unique per instance — these tests run in threads.
    fn scratch(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "ae-capture-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch");
        path
    }

    fn write(path: &Path, body: &str) {
        write_bytes(path, body.as_bytes());
    }

    fn write_bytes(path: &Path, body: &[u8]) {
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("dirs");
        std::fs::write(path, body).expect("a fixture file");
    }

    /// One roster row, spelled the way a v2 meta writes it.
    fn seat(slot: &str, name: &str, binary: &str, id: Option<&str>) -> crate::meta::RosterEntry {
        crate::meta::RosterEntry {
            slot: slot.to_owned(),
            name: name.to_owned(),
            profile: Some("p".to_owned()),
            client: crate::meta::RecordedClient::Missing,
            harness_session: id.map(ToOwned::to_owned),
            config_home: crate::meta::RecordedConfigHome::Missing,
            config_home_base: crate::meta::RecordedConfigHomeBase::Missing,
            binary: Some(binary.to_owned()),
            work_dir: crate::meta::RecordedWorkDir::Missing,
        }
    }

    #[test]
    fn a_recovery_takes_the_pending_seats_whose_tool_has_no_launch_time_id() {
        let roster = [
            // Pending, both spellings: the literal, and the empty row a meta
            // reads as absent metadata.
            seat("worker.1", "w1", "codex", Some(crate::launch::PENDING)),
            seat("worker.2", "w2", "gemini", None),
            seat("worker.3", "w3", "opencode", Some("")),
            // Pending, but claude and grok launch WITH an id — a seat holding
            // one is never waiting for a capture, whatever its row says.
            seat("main", "lead", "claude", Some(crate::launch::PENDING)),
            seat("worker.4", "w4", "grok", None),
            // Already captured: nothing to recover.
            seat("worker.5", "w5", "codex", Some("0191aaaa-bbbb")),
            // No `agent_bin.<slot>` at all — an unclassifiable seat is not a
            // capture target, and guessing one would ask the wrong tool.
            crate::meta::RosterEntry {
                binary: None,
                ..seat("worker.6", "w6", "codex", None)
            },
        ];

        let picked = pending_seats(&roster);
        let taken: Vec<(&str, &str)> = picked
            .iter()
            .map(|seat| (seat.slot.as_str(), seat.tool.as_str()))
            .collect();
        assert_eq!(
            taken,
            vec![
                ("worker.1", "codex"),
                ("worker.2", "gemini"),
                ("worker.3", "opencode"),
            ],
            "the pending capture-tool seats, in roster order"
        );
        assert_eq!(picked[0].agent, "w1", "the event names the AGENT");
    }

    #[test]
    fn the_scrape_takes_the_first_id_and_stops_at_the_first_other_byte() {
        let screen = "codex v1\n  session id: 0f9c-4a2b xyz\nmore\n";
        assert_eq!(scrape_session_id(screen).as_deref(), Some("0f9c-4a2b"));
    }

    #[test]
    fn a_screen_with_no_id_captures_nothing() {
        assert_eq!(scrape_session_id("nothing here\n"), None);
    }

    #[test]
    fn a_field_read_takes_the_first_match_and_skips_one_outside_its_class() {
        let line = r#"{"type":"session_meta","payload":{"id":"a1b2-c3","cwd":"/w/x"}}"#;
        assert_eq!(first_hex_field(line, "id").as_deref(), Some("a1b2-c3"));
        assert_eq!(first_string_field(line, "cwd").as_deref(), Some("/w/x"));
        // The class REJECTS and the scan carries on to the next `"id"`, which
        // is what keeps a non-uuid `id` from being captured as one.
        let mixed = r#"{"id":"session_meta","payload":{"id":"beef-01"}}"#;
        assert_eq!(first_hex_field(mixed, "id").as_deref(), Some("beef-01"));
        // Spacing around the separator is tolerated; a missing one is not.
        assert_eq!(
            first_string_field(r#""k" 	: 	"v""#, "k").as_deref(),
            Some("v")
        );
        assert_eq!(first_string_field(r#""k" "v""#, "k"), None);
        // An unterminated value is not a match.
        assert_eq!(first_string_field(r#""k":"v"#, "k"), None);
        assert_eq!(
            first_num_field(r#"{"time":{"updated":1700}}"#, "updated"),
            Some(1700)
        );
        assert_eq!(first_num_field(r#"{"updated":"1700"}"#, "updated"), None);
    }

    #[test]
    fn the_day_directories_are_today_and_yesterday_in_utc() {
        // 2026-03-01T00:30:00Z — the previous day is in another month.
        let at = Timestamp::parse("2026-03-01T00:30:00Z").expect("the documented form");
        assert_eq!(day_dirs(at), vec!["2026/03/01", "2026/02/28"]);
        let floor = Timestamp::parse("2026-02-27T23:59:59Z")
            .expect("the capture floor")
            .epoch();
        assert_eq!(
            codex_token_day_dirs(at, floor),
            vec!["2026/02/27", "2026/02/28", "2026/03/01"],
            "a known origin includes its UTC day through today"
        );
        let unknown = codex_token_day_dirs(at, 0);
        assert_eq!(unknown.len(), 30);
        assert_eq!(unknown.first().map(String::as_str), Some("2026/01/31"));
        assert_eq!(unknown.last().map(String::as_str), Some("2026/03/01"));
        assert!(
            codex_token_day_dirs(at, at.epoch().saturating_add(SECONDS_PER_DAY)).is_empty(),
            "a floor in a future UTC day cannot name a rollout partition"
        );
    }

    #[test]
    fn a_token_proven_codex_scan_reaches_old_partitions_but_legacy_cwd_stays_narrow() {
        let root = scratch("codex-token-days");
        let config_home = root.join("home").join(".codex");
        let work = root.join("project");
        std::fs::create_dir_all(&work).expect("a project dir");
        let now = Timestamp::now();
        let today = day_dirs(now).into_iter().next().expect("today");
        let older = day_dirs(Timestamp::from_epoch(
            now.epoch().saturating_sub(3 * 86_400),
        ))
        .into_iter()
        .next()
        .expect("three days ago");
        let outside_unknown_bound = day_dirs(Timestamp::from_epoch(
            now.epoch().saturating_sub(30 * 86_400),
        ))
        .into_iter()
        .next()
        .expect("thirty days ago");
        let old_id = "aaaa1111-bbbb-4ccc-8ddd-eeeeeeeeeeee";
        let today_id = "bbbb2222-cccc-4ddd-8eee-ffffffffffff";
        let outside_id = "cccc3333-dddd-4eee-8fff-aaaaaaaaaaaa";
        let old_work = work.display().to_string();
        for (day, name, id, token, cwd) in [
            (&older, "old", old_id, "old-token", old_work.as_str()),
            (&today, "today", today_id, "today-token", "/today-control"),
            (
                &outside_unknown_bound,
                "outside",
                outside_id,
                "outside-token",
                "/outside-control",
            ),
        ] {
            write(
                &config_home
                    .join("sessions")
                    .join(day)
                    .join(format!("rollout-{name}.jsonl")),
                &format!(
                    "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"{cwd}\"}}}}\n\
                     {{\"text\":\"AE_CODEX_LAUNCH_ID={token}\"}}\n"
                ),
            );
        }
        let facts_for = |launch_id: &str| Facts {
            agent: "lead".to_owned(),
            tool: ToolKind::Codex,
            work_dir: work.display().to_string(),
            provenance: crate::meta::SeatProvenance::Inherited,
            capture_floor: 0,
            launch_id: launch_id.to_owned(),
            launch_marker: Some("CODEX"),
            config_home: crate::meta::RecordedConfigHome::Path(config_home.clone()),
        };

        assert_eq!(
            scan_codex(&config_home, &facts_for("old-token")).as_deref(),
            Some(old_id),
            "positive token proof reaches a rollout partition older than yesterday"
        );
        assert_eq!(
            scan_codex(&config_home, &facts_for("today-token")).as_deref(),
            Some(today_id),
            "the same-day control still resolves"
        );
        let legacy = Facts {
            launch_id: String::new(),
            ..facts_for("")
        };
        assert_eq!(
            scan_codex(&config_home, &legacy),
            None,
            "tokenless cwd discovery stays restricted to today and yesterday"
        );
        assert_eq!(
            scan_codex(&config_home, &facts_for("outside-token")),
            None,
            "an unknown origin scans at most the last thirty UTC day partitions"
        );
    }

    #[test]
    fn a_muse_token_miss_never_adopts_a_session_directory() {
        // Two seats can share one Muse config home. A directory born after this
        // seat is still not evidence that it belongs to this launch.
        let root = scratch("muse-token-miss");
        let home = root.join("home");
        let day = day_dirs(Timestamp::now())
            .into_iter()
            .next()
            .expect("today");
        let sibling = "02b09b51-c88a-7fc0-8f71-200ea396c8a7";
        write_bytes(
            &home
                .join(MUSE_SESSIONS)
                .join(day)
                .join(sibling)
                .join("session.jsonl"),
            b"AE_MUSE_LAUNCH_ID=sibling-token",
        );
        let facts = Facts {
            agent: "lead".to_owned(),
            tool: ToolKind::Muse,
            work_dir: root.display().to_string(),
            provenance: crate::meta::SeatProvenance::Inherited,
            capture_floor: 0,
            launch_id: "own-token".to_owned(),
            launch_marker: Some("MUSE"),
            config_home: crate::meta::RecordedConfigHome::Missing,
        };

        assert_eq!(
            scan_muse(&home, &facts),
            None,
            "a token miss remains pending; never adopt a sibling's directory"
        );
    }

    /// One rival claimant: a pending opencode sibling in `dir` with a floor
    /// in milliseconds, or an unusable row when `dir` is `None`.
    fn sibling(dir: Option<&str>, floor_ms: i64) -> Sibling {
        Sibling {
            dir: dir.map(canonical),
            floor_ms,
        }
    }

    #[test]
    fn an_opencode_attribution_picks_the_newest_session_in_this_directory() {
        let dir = scratch("oc");
        let work = dir.display().to_string();
        let listed = format!(
            r#"[{{"id":"ses_before_floor","directory":"{work}","created":999,"updated":9000}},
               {{"id":"ses_new","directory":"{work}","created":1000,"updated":5000}},
               {{"id":"ses_newest","directory":"{work}","created":2000,"updated":2000}},
               {{"id":"ses_newest_tie","directory":"{work}","created":2000,"updated":3000}},
               {{"id":"ses_elsewhere","directory":"/nowhere","created":3000,"updated":9999}}]"#
        );
        // A lone seat with nothing recorded captures exactly as before: newest
        // birth, greatest id breaking the tie.
        assert_eq!(
            attribute_opencode(&listed, &work, 1000, &[], &[]),
            Some(("ses_newest_tie".to_owned(), 2000))
        );
        // The launch-time floor excludes a session born before it even when
        // that old session was touched after it.
        assert_eq!(attribute_opencode(&listed, &work, 3500, &[], &[]), None);
        // A record missing immutable birth evidence is never captured.
        assert_eq!(
            attribute_opencode(
                &format!(r#"[{{"id":"ses_missing_created","directory":"{work}","updated":9999}}]"#),
                &work,
                1000,
                &[],
                &[],
            ),
            None
        );
        // A record for another directory is never captured.
        assert_eq!(
            attribute_opencode(
                r#"[{"id":"ses_elsewhere","directory":"/nowhere","created":3000,"updated":9999}]"#,
                &work,
                0,
                &[],
                &[],
            ),
            None
        );
        // Nothing parseable is nothing captured, never a panic.
        assert_eq!(attribute_opencode("", &work, 0, &[], &[]), None);
        assert_eq!(
            attribute_opencode("opencode: not logged in", &work, 0, &[], &[]),
            None
        );
    }

    /// #56 G1: two seats converge. S's floor is 100 ms, T's 200; S's own
    /// session covers only S's window, so the first pass attributes it while
    /// T's stays covered by both. Once S records (no longer a rival), the next
    /// pass attributes T's.
    #[test]
    fn two_pending_seats_converge_one_session_per_pass() {
        let dir = scratch("oc-g1");
        let work = dir.display().to_string();
        let listed = format!(
            r#"[{{"id":"ses_s","directory":"{work}","created":150,"updated":150}},
               {{"id":"ses_t","directory":"{work}","created":250,"updated":250}}]"#
        );
        assert_eq!(
            attribute_opencode(&listed, &work, 100, &[sibling(Some(&work), 200)], &[]),
            Some(("ses_s".to_owned(), 150))
        );
        assert_eq!(
            attribute_opencode(&listed, &work, 200, &[sibling(Some(&work), 100)], &[]),
            None
        );
        assert_eq!(
            attribute_opencode(&listed, &work, 200, &[], &["ses_s".to_owned()]),
            Some(("ses_t".to_owned(), 250))
        );
    }

    /// #56 G2: the tie-break survives the attribution — an equal birth breaks
    /// by greatest id, and a recorded rival for the winner falls through to
    /// the next attributable candidate rather than to nothing.
    #[test]
    fn an_attribution_breaks_equal_births_by_greatest_id() {
        let dir = scratch("oc-g2");
        let work = dir.display().to_string();
        let listed = format!(
            r#"[{{"id":"ses_a","directory":"{work}","created":2000,"updated":2000}},
               {{"id":"ses_b","directory":"{work}","created":2000,"updated":2000}}]"#
        );
        assert_eq!(
            attribute_opencode(&listed, &work, 1000, &[], &[]),
            Some(("ses_b".to_owned(), 2000))
        );
        assert_eq!(
            attribute_opencode(&listed, &work, 1000, &[], &["ses_b".to_owned()]),
            Some(("ses_a".to_owned(), 2000))
        );
    }

    /// #56 G3': one candidate covering two pending windows is attributable to
    /// neither — the seat stays pending, and an excluded-only covering set
    /// reads the same as no candidate at all.
    #[test]
    fn a_session_covering_two_pending_windows_is_captured_by_neither() {
        let dir = scratch("oc-g3");
        let work = dir.display().to_string();
        let listed = format!(
            r#"[{{"id":"ses_shared","directory":"{work}","created":5000,"updated":5000}}]"#
        );
        assert_eq!(
            attribute_opencode(&listed, &work, 1000, &[sibling(Some(&work), 4000)], &[]),
            None
        );
        assert_eq!(
            attribute_opencode(&listed, &work, 1000, &[], &["ses_shared".to_owned()]),
            None
        );
    }

    /// #56 G4: a sibling whose directory row is unusable is assumed to share
    /// this seat's dir — it blocks, never silently drops out — while its floor
    /// still bounds what it covers. A sibling proven to be in another
    /// directory covers nothing.
    #[test]
    fn an_unusable_sibling_directory_covers_every_candidate() {
        let dir = scratch("oc-g4");
        let work = dir.display().to_string();
        let listed =
            format!(r#"[{{"id":"ses_own","directory":"{work}","created":5000,"updated":5000}}]"#);
        assert_eq!(
            attribute_opencode(&listed, &work, 1000, &[sibling(None, 0)], &[]),
            None
        );
        assert_eq!(
            attribute_opencode(&listed, &work, 1000, &[sibling(None, 999_999)], &[]),
            Some(("ses_own".to_owned(), 5000))
        );
        assert_eq!(
            attribute_opencode(&listed, &work, 1000, &[sibling(Some("/nowhere"), 0)], &[],),
            Some(("ses_own".to_owned(), 5000))
        );
    }

    /// #56 G7: a recorded id excludes whatever tool recorded it — a codex
    /// seat's uuid and an opencode seat's ses_ id alike.
    #[test]
    fn a_recorded_id_excludes_whatever_tool_recorded_it() {
        let dir = scratch("oc-g7");
        let work = dir.display().to_string();
        let listed = format!(
            r#"[{{"id":"ses_held","directory":"{work}","created":5000,"updated":5000}},
               {{"id":"ses_free","directory":"{work}","created":4000,"updated":4000}}]"#
        );
        assert_eq!(
            attribute_opencode(&listed, &work, 1000, &[], &["ses_held".to_owned()]),
            Some(("ses_free".to_owned(), 4000))
        );
    }

    /// #56 G9 (attribute): a birth exactly on the sibling's floor is covered —
    /// the `<=` the boundary mutant would flip to `<`.
    #[test]
    fn a_birth_on_the_sibling_floor_is_covered() {
        let dir = scratch("oc-g9a");
        let work = dir.display().to_string();
        let listed =
            format!(r#"[{{"id":"ses_edge","directory":"{work}","created":4000,"updated":4000}}]"#);
        assert_eq!(
            attribute_opencode(&listed, &work, 1000, &[sibling(Some(&work), 4000)], &[]),
            None
        );
        assert_eq!(
            attribute_opencode(&listed, &work, 1000, &[sibling(Some(&work), 4001)], &[]),
            Some(("ses_edge".to_owned(), 4000))
        );
    }

    /// One scratch session meta with the given seat rows, under a project dir
    /// the seats inherit.
    fn meta_dir(tag: &str, seats: &str) -> PathBuf {
        let dir = scratch(tag);
        let project = dir.join("project");
        std::fs::create_dir_all(&project).expect("a project dir");
        write(
            &dir.join("meta"),
            &format!(
                "session=cap\nwork_dir={}\nmode=local\nschema=2\n{seats}",
                project.display()
            ),
        );
        dir
    }

    /// #56 R3: a commit refuses an id another slot already records. The lock
    /// serializes concurrent captures, so the recheck under it closes the race
    /// the scan's snapshot leaves open. RED on base; on green this birthless
    /// capture refuses via the born-None rule (G8), and G10 proves the
    /// recorded-id check itself with a birth attached.
    #[test]
    fn a_commit_refuses_an_id_another_slot_already_records() {
        let dir = meta_dir(
            "oc-r3",
            "seat.main=lead\nprofile.main=tool\nagent_bin.main=opencode\n\
             harness_session.main=ses_recorded\nlaunch_time.main=1\ncapture_floor.main=1\n\
             launch_id.main=tok-1\nseat.worker.1=w1\nprofile.worker.1=tool\n\
             agent_bin.worker.1=opencode\nharness_session.worker.1=pending\n\
             launch_time.worker.1=1\ncapture_floor.worker.1=1\nlaunch_id.worker.1=tok-2\n",
        );
        let seat_facts = facts(&dir, "worker.1").expect("facts");
        let captured = Captured::new(&seat_facts, "ses_recorded".to_owned());
        assert!(
            !commit(&dir, "worker.1", &captured),
            "another slot's recorded id was committed"
        );
    }

    /// #56 R-commit: a commit refuses a session a pending sibling also covers.
    /// Both seats pending in one dir, one candidate covering both windows: on
    /// base the commit checks its own slot only and records it. On green this
    /// birthless capture refuses via the born-None rule (G8); G5 proves the
    /// sibling-floor check with a birth attached.
    #[test]
    fn a_commit_refuses_a_session_a_pending_sibling_also_covers() {
        let dir = meta_dir(
            "oc-rc",
            "seat.main=lead\nprofile.main=tool\nagent_bin.main=opencode\n\
             harness_session.main=pending\nlaunch_time.main=1\ncapture_floor.main=1\n\
             launch_id.main=tok-1\nseat.worker.1=w1\nprofile.worker.1=tool\n\
             agent_bin.worker.1=opencode\nharness_session.worker.1=pending\n\
             launch_time.worker.1=4\ncapture_floor.worker.1=4\nlaunch_id.worker.1=tok-2\n",
        );
        let seat_facts = facts(&dir, "main").expect("facts");
        let captured = Captured::new(&seat_facts, "ses_shared".to_owned());
        assert!(
            !commit(&dir, "main", &captured),
            "a session a pending sibling also covers was committed"
        );
    }

    /// Two pending opencode seats in one dir, floors 1 s and 4 s — the arena
    /// the commit isolation pins share.
    fn two_pending(tag: &str) -> PathBuf {
        meta_dir(
            tag,
            "seat.main=lead\nprofile.main=tool\nagent_bin.main=opencode\n\
             harness_session.main=pending\nlaunch_time.main=1\ncapture_floor.main=1\n\
             launch_id.main=tok-1\nseat.worker.1=w1\nprofile.worker.1=tool\n\
             agent_bin.worker.1=opencode\nharness_session.worker.1=pending\n\
             launch_time.worker.1=4\ncapture_floor.worker.1=4\nlaunch_id.worker.1=tok-2\n",
        )
    }

    /// #56 G5: the commit rechecks the sibling floor with the birth attached —
    /// a birth the sibling covers refuses even though the id is recorded
    /// nowhere.
    #[test]
    fn a_commit_rechecks_the_sibling_floor_with_the_birth() {
        let dir = two_pending("oc-g5");
        let seat_facts = facts(&dir, "main").expect("facts");
        let captured = Captured::with_born(&seat_facts, "ses_new".to_owned(), 5000);
        assert!(
            !commit(&dir, "main", &captured),
            "a birth the pending sibling covers was committed"
        );
    }

    /// #56 G10: the recorded-id check with a birth attached — no rival at
    /// all, the recorded id alone refuses.
    #[test]
    fn a_commit_refuses_a_recorded_id_with_a_birth_attached() {
        let dir = meta_dir(
            "oc-g10",
            "seat.main=lead\nprofile.main=tool\nagent_bin.main=opencode\n\
             harness_session.main=ses_recorded\nlaunch_time.main=1\ncapture_floor.main=1\n\
             launch_id.main=tok-1\nseat.worker.1=w1\nprofile.worker.1=tool\n\
             agent_bin.worker.1=opencode\nharness_session.worker.1=pending\n\
             launch_time.worker.1=1\ncapture_floor.worker.1=1\nlaunch_id.worker.1=tok-2\n",
        );
        let seat_facts = facts(&dir, "worker.1").expect("facts");
        let captured = Captured::with_born(&seat_facts, "ses_recorded".to_owned(), 9000);
        assert!(
            !commit(&dir, "worker.1", &captured),
            "a recorded id with a birth attached was committed"
        );
    }

    /// #56 G8: a birthless opencode capture refuses on its own — no rival, no
    /// recorded id, still nothing published.
    #[test]
    fn a_birthless_opencode_capture_refuses_alone() {
        let dir = meta_dir(
            "oc-g8",
            "seat.main=lead\nprofile.main=tool\nagent_bin.main=opencode\n\
             harness_session.main=pending\nlaunch_time.main=1\ncapture_floor.main=1\n\
             launch_id.main=tok-1\n",
        );
        let seat_facts = facts(&dir, "main").expect("facts");
        let captured = Captured::new(&seat_facts, "ses_new".to_owned());
        assert!(
            !commit(&dir, "main", &captured),
            "a birthless capture was committed"
        );
    }

    /// #56 G9 (commit): the boundary, under the lock — a birth exactly on the
    /// sibling's floor refuses, one millisecond below it commits.
    #[test]
    fn a_commit_covers_a_birth_on_the_sibling_floor() {
        let dir = two_pending("oc-g9c");
        let seat_facts = facts(&dir, "main").expect("facts");
        let edge = Captured::with_born(&seat_facts, "ses_edge".to_owned(), 4000);
        assert!(
            !commit(&dir, "main", &edge),
            "a birth on the sibling floor was committed"
        );
        let dir = two_pending("oc-g9c2");
        let seat_facts = facts(&dir, "main").expect("facts");
        let past = Captured::with_born(&seat_facts, "ses_past".to_owned(), 3999);
        assert!(
            commit(&dir, "main", &past),
            "a birth below the sibling floor refused"
        );
    }

    /// #56 A2: a legacy pending sibling with `launch_time` but no
    /// `capture_floor` row keeps its launch-time floor — one floor owner with
    /// its own capture — so it does not cover a birth before its launch.
    #[test]
    fn a_legacy_sibling_without_a_floor_row_keeps_its_launch_time() {
        let dir = meta_dir(
            "oc-a2",
            "seat.main=lead\nprofile.main=tool\nagent_bin.main=opencode\n\
             harness_session.main=pending\nlaunch_time.main=1\ncapture_floor.main=1\n\
             launch_id.main=tok-1\nseat.worker.1=w1\nprofile.worker.1=tool\n\
             agent_bin.worker.1=opencode\nharness_session.worker.1=pending\n\
             launch_time.worker.1=4\nlaunch_id.worker.1=tok-2\n",
        );
        let bytes = std::fs::read(dir.join("meta")).expect("meta");
        let siblings = siblings_from(&bytes, "main");
        assert_eq!(siblings.len(), 1);
        let target = canonical(&dir.join("project").display().to_string());
        assert!(
            !siblings[0].covers(&target, 3999),
            "a birth before the sibling's launch is not covered"
        );
        assert!(
            siblings[0].covers(&target, 4000),
            "a birth on the sibling's launch is covered"
        );
    }

    /// The recheck's `true` path: a lone pending seat commits its attributed
    /// birth. A pending codex seat beside it in the same dir is no rival: only
    /// the list capture shares candidates (N1).
    #[test]
    fn a_lone_pending_seat_commits_its_attributed_birth() {
        let dir = meta_dir(
            "oc-lone",
            "seat.main=lead\nprofile.main=tool\nagent_bin.main=opencode\n\
             harness_session.main=pending\nlaunch_time.main=1\ncapture_floor.main=1\n\
             launch_id.main=tok-1\nseat.worker.9=w9\nprofile.worker.9=tool\n\
             agent_bin.worker.9=codex\nharness_session.worker.9=pending\n\
             launch_time.worker.9=1\ncapture_floor.worker.9=1\nlaunch_id.worker.9=tok-9\n",
        );
        let seat_facts = facts(&dir, "main").expect("facts");
        let captured = Captured::with_born(&seat_facts, "ses_new".to_owned(), 5000);
        assert!(
            commit(&dir, "main", &captured),
            "a lone seat's attribution refused"
        );
        let meta = std::fs::read_to_string(dir.join("meta")).expect("meta");
        assert!(meta.contains("harness_session.main=ses_new\n"), "{meta}");
    }

    #[test]
    fn an_agy_conversation_is_matched_by_its_launch_token_and_by_the_cli_log() {
        let root = scratch("agy");
        let home = root.join("home");
        let work = root.join("project");
        std::fs::create_dir_all(&work).expect("a project dir");
        let store = home.join(AGY_CONVERSATIONS);
        let logs = home.join(AGY_LOGS);
        let id = "643393ad-eb92-4b9e-ab7a-0fe7b1221fa1";

        // A conversation database is `SQLite`: BINARY, and not valid UTF-8.
        write_bytes(
            &store.join(format!("{id}.db")),
            b"SQLite format 3\x00\xff\xfe AE_AGY_LAUNCH_ID=tok-1 \xc3\x28",
        );
        // Another launch's conversation, and the sidecars SQLite writes beside
        // a live database.
        write_bytes(
            &store.join("11111111-2222-4333-8444-555555555555.db"),
            b"\xffAE_AGY_LAUNCH_ID=tok-9",
        );
        write_bytes(
            &store.join(format!("{id}.db-wal")),
            b"AE_AGY_LAUNCH_ID=tok-1",
        );
        // A file whose stem is not an id at all — the class check, not decoration.
        write_bytes(&store.join("notes.db"), b"AE_AGY_LAUNCH_ID=tok-1");

        write(
            &logs.join("cli-20260904_180410.log"),
            &format!(
                "server.go:285] Creating CLI server backend: product=antigravity \
                 workspaceDirs=[{work}] appDataDir=/x\n\
                 server.go:1137] Created conversation {id}\n\
                 server.go:1137] Created conversation 99999999-9999-4999-8999-999999999999\n",
                work = work.display()
            ),
        );
        // A newer run in ANOTHER workspace, and the `cli.log` pointer that
        // names the same file a second time.
        write(
            &logs.join("cli-20260904_181500.log"),
            "workspaceDirs=[/nowhere]\nCreated conversation deadbeef-0000-4000-8000-000000000000\n",
        );
        write(
            &logs.join("cli.log"),
            "workspaceDirs=[/nowhere]\nCreated conversation deadbeef-0000-4000-8000-000000000000\n",
        );

        let work = work.display().to_string();
        assert_eq!(
            find_agy_by_launch_id(&home, "AGY", "tok-1", 0).as_deref(),
            Some(id)
        );
        assert_eq!(find_agy_by_launch_id(&home, "AGY", "tok-2", 0), None);
        // The log fallback takes the FIRST conversation the matching run
        // created, never a later hand-started one and never another workspace's.
        assert_eq!(find_agy_by_cwd(&home, &work, 0).as_deref(), Some(id));
        // The launch-time floor keeps a conversation that predates this launch
        // out of BOTH halves.
        let future = i64::MAX / 2;
        assert_eq!(find_agy_by_launch_id(&home, "AGY", "tok-1", future), None);
        assert_eq!(find_agy_by_cwd(&home, &work, future), None);
        // A home with no agy state at all is quiet.
        assert_eq!(find_agy_by_cwd(&root.join("empty"), &work, 0), None);
        assert_eq!(
            find_agy_by_launch_id(&root.join("empty"), "AGY", "tok-1", 0),
            None
        );
    }

    #[test]
    fn two_agy_seats_in_one_directory_stay_pending_rather_than_take_each_other_s() {
        // THE CROSS-WIRING DEFECT, pinned. Two agy seats share a working
        // directory; the sibling has a conversation on disk and this seat has
        // only a token. A chain that fell through answered with the sibling's
        // id, and a resume makes that permanent.
        let root = scratch("agy-siblings");
        let home = root.join("home");
        let work = root.join("project");
        std::fs::create_dir_all(&work).expect("a project dir");
        let logs = home.join(AGY_LOGS);
        let sibling = "aaaaaaaa-1111-4111-8111-111111111111";
        let mine = "bbbbbbbb-2222-4222-8222-222222222222";
        for (name, id) in [("cli-000-sibling.log", sibling), ("cli-999-own.log", mine)] {
            write(
                &logs.join(name),
                &format!(
                    "workspaceDirs=[{work}]\nCreated conversation {id}\n",
                    work = work.display()
                ),
            );
        }
        let work = work.display().to_string();

        // The seat HAS a token, and no database carries it yet.
        let facts = Facts {
            agent: "lead".to_owned(),
            tool: ToolKind::Agy,
            work_dir: work.clone(),
            provenance: crate::meta::SeatProvenance::Inherited,
            capture_floor: 0,
            launch_id: "own-token".to_owned(),
            launch_marker: Some("AGY"),
            config_home: crate::meta::RecordedConfigHome::Missing,
        };
        assert_eq!(
            scan_agy(&home, &facts),
            None,
            "a token miss must stay pending, never fall through to the workspace"
        );

        // And the workspace search on its own refuses to pick between the two,
        // which is what protects the no-token seat the fallback is FOR.
        assert_eq!(
            find_agy_by_cwd(&home, &work, 0),
            None,
            "two candidate conversations in one directory is not an answer"
        );

        // Once the token IS on disk, the same seat resolves — and to ITS OWN
        // conversation, not the newer sibling log's.
        write_bytes(
            &home.join(AGY_CONVERSATIONS).join(format!("{mine}.db")),
            b"\x00\xffAE_AGY_LAUNCH_ID=own-token\x00",
        );
        assert_eq!(scan_agy(&home, &facts).as_deref(), Some(mine));
    }

    #[test]
    fn a_stream_that_never_ends_is_bounded_rather_than_followed() {
        // A conversation database is LIVE, so its length at stat time is not a
        // bound on what a read loop will be handed.
        let marker = b"AE_AGY_LAUNCH_ID=tok-1";
        assert_eq!(
            scan_stream(std::io::repeat(0x00), marker),
            Scan::OverCap,
            "an endless stream must exhaust the budget, not the machine"
        );
        // OverCap is not Absent: the answer is UNKNOWN, and a caller that
        // conflated them would report "no such conversation" for a database it
        // simply stopped reading.
        assert_ne!(scan_stream(std::io::repeat(0x00), marker), Scan::Absent);
        // The ordinary answers still work through the same path.
        let mut body = vec![0_u8; 4096];
        body.extend_from_slice(marker);
        assert_eq!(scan_stream(body.as_slice(), marker), Scan::Found);
        assert_eq!(scan_stream(&b"nothing here"[..], marker), Scan::Absent);
    }

    #[test]
    fn a_node_that_is_not_a_regular_file_is_never_opened() {
        // `open(2)` on a FIFO BLOCKS until a writer appears, and this runs in
        // the watchdog's cycle — so a named pipe called `<uuid>.db` would hang
        // the daemon.
        let root = scratch("agy-nodes");
        let marker = b"AE_AGY_LAUNCH_ID=tok-1";

        let directory = root.join("a-directory.db");
        std::fs::create_dir_all(&directory).expect("a directory node");
        assert!(!file_contains(&directory, marker));

        let socket = root.join("a-socket.db");
        let listener = std::os::unix::net::UnixListener::bind(&socket).expect("a socket node");
        assert!(!file_contains(&socket, marker));
        drop(listener);

        // The control: the same call on a REGULAR file with the same content
        // still answers yes, so the guard is refusing the node and not the
        // needle.
        let regular = root.join("a-regular.db");
        write_bytes(&regular, marker);
        assert!(file_contains(&regular, marker));
    }

    #[test]
    fn a_marker_lying_across_a_chunk_boundary_is_still_found_and_a_huge_file_is_skipped() {
        // The chunked scan's OWN defect class: a needle split by the seam
        // between two reads.
        let root = scratch("agy-chunks");
        let marker = b"AE_AGY_LAUNCH_ID=tok-1";
        let seam = marker.len() - 1 + TOKEN_SCAN_CHUNK;
        for shift in 1..marker.len() {
            let at = seam - marker.len() + shift;
            let path = root.join(format!("straddle-{shift}.db"));
            let mut body = vec![0_u8; at];
            body.extend_from_slice(marker);
            body.extend_from_slice(&[0xff_u8; 4096]);
            write_bytes(&path, &body);
            assert!(
                file_contains(&path, marker),
                "a marker at byte {at}, {shift} bytes across the seam, must still be found"
            );
            assert!(
                !file_contains(&path, b"AE_AGY_LAUNCH_ID=tok-9"),
                "and a different token must not match at {at}"
            );
        }

        // A file past the cap is SKIPPED, not read: the marker is there and the
        // answer is still no, which is the trade the constant records.
        let cap = usize::try_from(TOKEN_SCAN_CAP).unwrap_or(usize::MAX);
        let big = root.join("huge.db");
        let mut body = marker.to_vec();
        body.resize(cap + 1, 0);
        write_bytes(&big, &body);
        assert!(
            !file_contains(&big, marker),
            "a database past the scan cap is skipped rather than walked"
        );
        // A file exactly AT the cap is still scanned — the boundary is `>`.
        let edge = root.join("edge.db");
        body.truncate(cap);
        write_bytes(&edge, &body);
        assert!(file_contains(&edge, marker));
    }

    #[test]
    fn a_gemini_chat_is_matched_by_its_launch_token_and_by_its_project_root() {
        let root = scratch("gem");
        let home = root.join("home");
        let work = root.join("project");
        std::fs::create_dir_all(&work).expect("a project dir");
        let project = home.join(".gemini").join("tmp").join("digest");
        write(&project.join(".project_root"), &work.display().to_string());
        write(
            &project.join("chats").join("session-a.json"),
            r#"{"sessionId":"gem-a","messages":["AE_GEMINI_LAUNCH_ID=tok-1"]}"#,
        );
        // A chat for ANOTHER project, carrying the same token, must not match.
        let other = home.join(".gemini").join("tmp").join("elsewhere");
        write(&other.join(".project_root"), "/nowhere");
        write(
            &other.join("chats").join("session-b.json"),
            r#"{"sessionId":"gem-b","messages":["AE_GEMINI_LAUNCH_ID=tok-1"]}"#,
        );
        let work = work.display().to_string();
        assert_eq!(
            find_gemini_by_launch_id(&home, &work, "GEMINI", "tok-1", 0).as_deref(),
            Some("gem-a")
        );
        assert_eq!(
            find_gemini_by_launch_id(&home, &work, "GEMINI", "tok-2", 0),
            None
        );
        assert_eq!(
            find_gemini_by_cwd(&home, &work, 0).as_deref(),
            Some("gem-a")
        );
        // The launch-time floor keeps a chat that predates this launch out.
        let future = i64::MAX / 2;
        assert_eq!(find_gemini_by_cwd(&home, &work, future), None);
        // A home with no gemini history at all is quiet.
        assert_eq!(find_gemini_by_cwd(&root.join("empty"), &work, 0), None);
    }

    #[test]
    fn a_codex_log_is_matched_by_its_launch_token_and_by_its_recorded_cwd() {
        let root = scratch("codex");
        let home = root.join("home");
        let work = root.join("project");
        std::fs::create_dir_all(&work).expect("a project dir");
        let days = vec!["2026/09/03".to_owned()];
        let config_home = home.join(".codex");
        let day = config_home.join("sessions").join("2026/09/03");
        write(
            &day.join("rollout-1.jsonl"),
            &format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"c0de-01\",\"cwd\":\"{}\"}}}}\n\
                 {{\"text\":\"AE_CODEX_LAUNCH_ID=tok-1\"}}\n",
                work.display()
            ),
        );
        // Another conversation in another directory, with no token.
        write(
            &day.join("rollout-2.jsonl"),
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"c0de-02\",\"cwd\":\"/nowhere\"}}\n",
        );
        // A file that is not a session log is not read as one.
        write(&day.join("notes.txt"), "AE_CODEX_LAUNCH_ID=tok-1\n");
        let work = work.display().to_string();
        assert_eq!(
            find_codex_by_launch_id_in_days(&config_home, "CODEX", "tok-1", 0, &days).as_deref(),
            Some("c0de-01")
        );
        assert_eq!(
            find_codex_by_launch_id_in_days(&config_home, "CODEX", "tok-2", 0, &days),
            None
        );
        assert_eq!(
            find_codex_by_cwd(&config_home, &work, 0, &days).as_deref(),
            Some("c0de-01")
        );
        assert_eq!(
            find_codex_by_cwd(&config_home, "/nowhere", 0, &days).as_deref(),
            Some("c0de-02")
        );
        // A day that was never written is not an error.
        assert_eq!(
            find_codex_by_launch_id_in_days(
                &config_home,
                "CODEX",
                "tok-1",
                0,
                &["2020/01/01".to_owned()],
            ),
            None
        );
    }

    #[test]
    fn a_codex_token_miss_does_not_fall_back_to_another_launch_in_the_same_cwd() {
        let root = scratch("codex-token-miss");
        let config_home = root.join("home").join(".codex");
        let work = root.join("project");
        std::fs::create_dir_all(&work).expect("a project dir");
        let day = day_dirs(Timestamp::from_epoch(
            Timestamp::now().epoch().saturating_sub(3 * SECONDS_PER_DAY),
        ))
        .into_iter()
        .next()
        .expect("three days ago");
        write(
            &config_home
                .join("sessions")
                .join(day)
                .join("rollout-retired.jsonl"),
            &format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"aaaa1111-bbbb-4ccc-8ddd-eeeeeeeeeeee\",\"cwd\":\"{}\"}}}}\n\
                 {{\"text\":\"AE_CODEX_LAUNCH_ID=retired-token\"}}\n",
                work.display()
            ),
        );
        let facts = Facts {
            agent: "lead".to_owned(),
            tool: ToolKind::Codex,
            work_dir: work.display().to_string(),
            provenance: crate::meta::SeatProvenance::Inherited,
            capture_floor: 0,
            launch_id: "current-token".to_owned(),
            launch_marker: Some("CODEX"),
            config_home: crate::meta::RecordedConfigHome::Path(config_home.clone()),
        };

        assert_eq!(
            scan_codex(&config_home, &facts),
            None,
            "a present launch token is the identity boundary; cwd alone cannot replace it"
        );
    }

    #[test]
    fn a_codex_rollout_started_before_the_launch_is_never_a_candidate() {
        let root = scratch("codex-before-launch");
        let config_home = root.join("home").join(".codex");
        let day = "2026/09/10";
        let id = "aaaa1111-bbbb-4ccc-8ddd-eeeeeeeeeeee";
        write(
            &config_home
                .join("sessions")
                .join(day)
                .join("rollout-retired.jsonl"),
            &format!(
                "{{\"timestamp\":\"2026-09-10T10:00:00Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"/work\"}}}}\n\
                 {{\"text\":\"AE_CODEX_LAUNCH_ID=reused-token\"}}\n"
            ),
        );
        let launched = Timestamp::parse("2026-09-10T10:01:00Z")
            .expect("the launch timestamp")
            .epoch();

        assert_eq!(
            find_codex_by_launch_id_in_days(
                &config_home,
                "CODEX",
                "reused-token",
                launched,
                &[day.to_owned()],
            ),
            None,
            "a later mtime cannot turn an older rollout into this launch"
        );
    }

    #[test]
    fn a_late_capture_cannot_commit_after_its_slot_is_reoccupied() {
        let dir = scratch("late-commit");
        write(
            &dir.join("meta"),
            "schema=2\nseat.spawned.0=current\nprofile.spawned.0=p\n\
             agent_bin.spawned.0=codex\nharness_session.spawned.0=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\n\
             launch_id.spawned.0=current-token\n",
        );
        let retired = Captured {
            id: "aaaa1111-bbbb-4ccc-8ddd-eeeeeeeeeeee".to_owned(),
            agent: "retired".to_owned(),
            tool: ToolKind::Codex,
            launch_id: "retired-token".to_owned(),
            work_dir: String::new(),
            provenance: crate::meta::SeatProvenance::Inherited,
            born_ms: None,
        };

        assert!(
            !commit(&dir, "spawned.0", &retired),
            "the capture result belongs to the retired launch"
        );
        let meta = std::fs::read_to_string(dir.join("meta")).expect("the meta remains");
        assert!(
            meta.contains("harness_session.spawned.0=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),
            "{meta}"
        );

        let current = Captured {
            id: "bbbb2222-cccc-4ddd-8eee-ffffffffffff".to_owned(),
            agent: "current".to_owned(),
            tool: ToolKind::Codex,
            launch_id: "current-token".to_owned(),
            work_dir: String::new(),
            provenance: crate::meta::SeatProvenance::Inherited,
            born_ms: None,
        };
        assert!(
            !commit(&dir, "spawned.0", &current),
            "an ordinary late scan cannot replace an already recorded id"
        );
        assert!(commit_authoritative(&dir, "spawned.0", &current));
        let meta = std::fs::read_to_string(dir.join("meta")).expect("the committed meta");
        assert!(
            meta.contains("harness_session.spawned.0=bbbb2222-cccc-4ddd-8eee-ffffffffffff"),
            "{meta}"
        );
    }

    #[test]
    fn register_sid_scans_the_recorded_codex_home_not_the_capture_process_home() {
        let dir = scratch("recorded-codex-home");
        let config_home = dir.join("account");
        let work = dir.join("project");
        std::fs::create_dir_all(&work).expect("project");
        let day = day_dirs(Timestamp::from_epoch(
            Timestamp::now().epoch().saturating_sub(3 * SECONDS_PER_DAY),
        ))
        .into_iter()
        .next()
        .expect("three days ago");
        let id = "88888888-8888-4888-8888-888888888888";
        write(
            &config_home
                .join("sessions")
                .join(day)
                .join(format!("rollout-{id}.jsonl")),
            &format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"{}\"}}}}\n\
                 {{\"text\":\"AE_CODEX_LAUNCH_ID=tok-recorded\"}}\n",
                work.display()
            ),
        );
        write(
            &dir.join("meta"),
            &format!(
                "schema=2\nwork_dir={}\nseat.main=lead\nagent_bin.main=codex\n\
                 harness_session.main=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\n\
                 config_home.main={}\nlaunch_id.main=tok-recorded\nlaunch_time.main=0\n\
                 work_dir.main={}\n",
                work.display(),
                config_home.display(),
                work.display()
            ),
        );
        let mut out = Vec::new();
        let mut err = Vec::new();
        assert_eq!(
            register_sid(&dir, "main", None, &mut out, &mut err).expect("register"),
            0,
            "{}",
            String::from_utf8_lossy(&err)
        );
        let meta = std::fs::read_to_string(dir.join("meta")).expect("the committed meta");
        assert!(
            meta.contains(&format!("harness_session.main={id}\n")),
            "the token-proven handshake must replace a wrong earlier capture: {meta}"
        );
        let prior = "harness_session_prior.main=codex:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\n";
        assert!(
            meta.contains(prior),
            "the replaced id must be kept as the newest tagged predecessor: {meta}"
        );
        assert!(
            !sid_file(&dir, "main").exists(),
            "a successful direct commit leaves no handshake artifact"
        );
    }

    #[test]
    fn the_capture_facts_use_the_capture_floor_and_a_bad_value_is_zero() {
        let dir = scratch("facts");
        write(
            &dir.join("meta"),
            "session=s\nwork_dir=/w\nschema=2\nseat.main=lead\nagent_bin.main=opencode\n\
             config_home.main=/account/codex\ncapture_floor.main=not-a-number\nlaunch_id.main=tok-9\n",
        );
        let read = facts(&dir, "main").expect("the meta reads");
        assert_eq!(read.agent, "lead");
        assert_eq!(read.tool, ToolKind::OpenCode);
        assert_eq!(read.work_dir, "/w");
        assert_eq!(read.capture_floor, 0);
        assert_eq!(read.launch_id, "tok-9");
        assert_eq!(
            read.config_home,
            crate::meta::RecordedConfigHome::Path(PathBuf::from("/account/codex"))
        );
        assert_eq!(
            codex_config_home(&read, Some(Path::new("/ambient"))),
            Some(PathBuf::from("/account/codex")),
            "the recorded root wins over the detached capture process's HOME"
        );
        // A seat that is not in the meta is a tool nothing captures.
        let missing = facts(&dir, "worker.0").expect("the meta still reads");
        assert_eq!(missing.tool, ToolKind::Unknown);
        assert!(
            facts(&scratch("empty"), "main").is_none(),
            "no meta, no facts"
        );
    }

    #[test]
    fn the_opencode_argv_is_fixed() {
        assert_eq!(
            opencode_list_argv().as_args(),
            ["session", "list", "--format", "json", "-n", "20"]
        );
    }

    #[test]
    fn the_opencode_export_argv_is_fixed_and_the_grammar_gates_it() {
        let id = "ses_00000000000000000000000000";
        assert_eq!(
            opencode_export_argv(id).map(|argv| argv.as_args().to_vec()),
            Some(vec!["export".to_owned(), id.to_owned()])
        );
        for bad in [
            "",
            "ses_",
            "ses_x/y",
            "ses_x y",
            "ses_x\ny",
            "ses_x\"q",
            "-ses_x",
            "msg_00000000000000000000000000",
            &format!("ses_{}", "a".repeat(65)),
        ] {
            assert!(
                opencode_export_argv(bad).is_none(),
                "{bad:?} must not mint an argv"
            );
            assert!(!is_opencode_session_id(bad), "{bad:?}");
        }
        assert!(is_opencode_session_id("ses_a1B2"));
    }

    /// One attempt-time capture result via the production snapshot owner.
    fn attempt_captured(dir: &Path, id: &str) -> Captured {
        let snapshot = facts(dir, "main").expect("attempt facts");
        Captured::new(&snapshot, id.to_owned())
    }

    #[test]
    fn facts_derive_seat_dir_and_refuse_damage() {
        use crate::meta::SeatProvenance as P;
        for (meta, dir, prov) in [
            ("work_dir=/s\nwork_dir.main=/e\n", "/e", P::Explicit),
            ("work_dir=/s\n", "/s", P::Inherited),
            ("work_dir=/s\nwork_dir.main=/s\n", "/s", P::Explicit),
        ] {
            let d = scratch("facts-seat");
            let m = d.join("meta");
            write(&m, &format!("schema=2\nseat.main=lead\n{meta}"));
            let got = facts(&d, "main").map(|f| (f.work_dir, f.provenance));
            assert_eq!(got, Some((dir.to_owned(), prov)));
        }
        for meta in [
            &b"schema=2\nwork_dir=/s\nseat.main=lead\nwork_dir.main=\n"[..],
            &b"schema=2\nwork_dir=/s\nseat.main=lead\nwork_dir.main=/a\nwork_dir.main=/b\n"[..],
            &b"schema=2\nwork_dir=/s\nwork_dir.main=/x/\xff\n"[..],
        ] {
            let d = scratch("facts-seat-bad");
            write_bytes(&d.join("meta"), meta);
            assert!(facts(&d, "main").is_none());
        }
    }

    #[test]
    fn commit_refuses_when_the_seat_directory_moves_midflight() {
        let session = "work_dir=/s\n";
        for (attempt, now) in [
            ("", "work_dir.main=/s\n"),
            ("work_dir.main=/s\n", ""),
            ("work_dir.main=/a\n", "work_dir.main=/b\n"),
            ("work_dir.main=/a\n", "work_dir.main=\n"),
        ] {
            let dir = scratch("commit-bind");
            let base = "schema=2\nseat.main=lead\nagent_bin.main=codex\n\
                        harness_session.main=pending\nlaunch_id.main=tok\n";
            write(&dir.join("meta"), &format!("{base}{session}{attempt}"));
            let got = attempt_captured(&dir, "id");
            write(&dir.join("meta"), &format!("{base}{session}{now}"));
            assert!(!commit(&dir, "main", &got), "{attempt:?} -> {now:?}");
        }
    }

    #[test]
    fn commit_refuses_inherited_session_dir_drift_and_stays_pending() {
        let dir = scratch("commit-drift");
        let base = "schema=2\nseat.main=lead\nagent_bin.main=codex\n\
                    harness_session.main=pending\nlaunch_id.main=tok\n";
        write(&dir.join("meta"), &format!("{base}work_dir=/s\n"));
        let got = attempt_captured(&dir, "id");
        write(&dir.join("meta"), &format!("{base}work_dir=/t\n"));
        assert!(!commit(&dir, "main", &got));
        let meta = std::fs::read_to_string(dir.join("meta")).expect("meta");
        assert!(meta.contains("harness_session.main=pending"), "{meta}");
    }

    #[test]
    fn authoritative_commit_refuses_a_drifted_directory() {
        let dir = scratch("commit-auth-drift");
        let old = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let base = "schema=2\nseat.main=lead\nagent_bin.main=codex\nlaunch_id.main=tok\n";
        let id_row = format!("harness_session.main={old}\n");
        let m = dir.join("meta");
        write(&m, &format!("{base}{id_row}work_dir.main=/a\n"));
        let got = attempt_captured(&dir, "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
        write(&m, &format!("{base}{id_row}work_dir.main=/b\n"));
        let before = std::fs::read(&m).expect("meta");
        assert!(!commit_authoritative(&dir, "main", &got));
        assert_eq!(std::fs::read(&m).expect("meta"), before);
    }

    #[test]
    fn damaged_row_register_sid_exits_2_with_meta_unchanged() {
        let dir = scratch("sid-damaged");
        let meta = "schema=2\nwork_dir=/s\nwork_dir.main=\nseat.main=lead\n\
                    agent_bin.main=codex\nconfig_home.main=/nonexistent-ae-t8t\n";
        write(&dir.join("meta"), meta);
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = register_sid(&dir, "main", None, &mut out, &mut err).expect("exit");
        assert_eq!(code, 2);
        let after = std::fs::read(dir.join("meta")).expect("meta");
        assert_eq!(after.as_slice(), meta.as_bytes());
        assert!(!dir.join("codex.main.sid").exists());
    }

    /// One codex rollout under today's partition: id/cwd/stamp/token lines.
    fn rollout(config_home: &Path, id: &str, cwd: &str, stamp: &str, token: &str) {
        let day = day_dirs(Timestamp::now())
            .into_iter()
            .next()
            .expect("today");
        let path = config_home.join(format!("sessions/{day}/rollout-{id}.jsonl"));
        let body = format!(
            "{{\"timestamp\":\"{stamp}\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"{cwd}\"}}}}\n\
             {token}\n"
        );
        write(&path, &body);
    }

    #[test]
    fn explicit_codex_without_token_stays_pending() {
        let root = scratch("codex-exp-empty");
        let config_home = root.join("home").join(".codex");
        let seat = root.join("seat").display().to_string();
        rollout(&config_home, "c0de-e1", &seat, "", "");
        let facts = Facts {
            agent: "lead".to_owned(),
            tool: ToolKind::Codex,
            work_dir: seat,
            provenance: crate::meta::SeatProvenance::Explicit,
            capture_floor: 0,
            launch_id: String::new(),
            launch_marker: Some("CODEX"),
            config_home: crate::meta::RecordedConfigHome::Missing,
        };
        assert_eq!(scan_codex(&config_home, &facts), None);
    }

    #[test]
    fn explicit_codex_with_missing_token_stays_pending() {
        let root = scratch("codex-exp-miss");
        let config_home = root.join("home").join(".codex");
        let seat = root.join("seat").display().to_string();
        let token = "{\"text\":\"AE_CODEX_LAUNCH_ID=other-token\"}";
        rollout(&config_home, "c0de-0e", &seat, "", token);
        let facts = Facts {
            agent: "lead".to_owned(),
            tool: ToolKind::Codex,
            work_dir: seat,
            provenance: crate::meta::SeatProvenance::Explicit,
            capture_floor: 0,
            launch_id: "missing-token".to_owned(),
            launch_marker: Some("CODEX"),
            config_home: crate::meta::RecordedConfigHome::Missing,
        };
        assert_eq!(scan_codex(&config_home, &facts), None);
    }

    #[test]
    fn inherited_codex_matches_cwd_with_a_floored_decoy_present() {
        let root = scratch("codex-inh-decoy");
        let config_home = root.join("home").join(".codex");
        let seat = root.join("seat").display().to_string();
        let old = "2026-09-10T10:00:00Z";
        let new = "2026-09-10T10:02:00Z";
        rollout(&config_home, "c0de-00", &seat, old, "");
        rollout(&config_home, "c0de-ff", &seat, new, "");
        let facts = Facts {
            agent: "lead".to_owned(),
            tool: ToolKind::Codex,
            work_dir: seat,
            provenance: crate::meta::SeatProvenance::Inherited,
            capture_floor: 1_789_034_460, // 2026-09-10T10:01:00Z
            launch_id: String::new(),
            launch_marker: Some("CODEX"),
            config_home: crate::meta::RecordedConfigHome::Missing,
        };
        let got = scan_codex(&config_home, &facts);
        assert_eq!(got.as_deref(), Some("c0de-ff"));
    }

    /// One gemini project: root row + one chat carrying id and token text.
    fn gemini_chat(home: &Path, proj: &str, root: &str, id: &str, token: &str) {
        let project = home.join(".gemini").join("tmp").join(proj);
        write(&project.join(".project_root"), root);
        let body = format!("{{\"sessionId\":\"{id}\",\"messages\":[\"{token}\"]}}");
        write(&project.join("chats").join("session-a.json"), &body);
    }

    #[test]
    fn gemini_token_hit_wins_over_a_same_token_decoy_project() {
        let root = scratch("gem-tok-decoy");
        let home = root.join("home");
        let seat = root.display().to_string();
        let tok = "AE_GEMINI_LAUNCH_ID=tok-a";
        gemini_chat(&home, "digest", &seat, "sess-expected", tok);
        gemini_chat(&home, "elsewhere", "/nowhere", "sess-decoy", tok);
        let facts = Facts {
            agent: "lead".to_owned(),
            tool: ToolKind::Gemini,
            work_dir: seat,
            provenance: crate::meta::SeatProvenance::Explicit,
            capture_floor: 0,
            launch_id: "tok-a".to_owned(),
            launch_marker: Some("GEMINI"),
            config_home: crate::meta::RecordedConfigHome::Missing,
        };
        let got = scan_gemini(&home, &facts);
        assert_eq!(got.as_deref(), Some("sess-expected"));
    }

    #[test]
    fn explicit_gemini_without_token_stays_pending() {
        let root = scratch("gem-exp-empty");
        let home = root.join("home");
        let seat = root.display().to_string();
        gemini_chat(&home, "digest", &seat, "sess-seat", "");
        let facts = Facts {
            agent: "lead".to_owned(),
            tool: ToolKind::Gemini,
            work_dir: seat,
            provenance: crate::meta::SeatProvenance::Explicit,
            capture_floor: 0,
            launch_id: String::new(),
            launch_marker: Some("GEMINI"),
            config_home: crate::meta::RecordedConfigHome::Missing,
        };
        assert_eq!(scan_gemini(&home, &facts), None);
    }

    #[test]
    fn explicit_gemini_with_missing_token_stays_pending() {
        let root = scratch("gem-exp-miss");
        let home = root.join("home");
        let seat = root.display().to_string();
        gemini_chat(&home, "digest", &seat, "sess-seat", "");
        let facts = Facts {
            agent: "lead".to_owned(),
            tool: ToolKind::Gemini,
            work_dir: seat,
            provenance: crate::meta::SeatProvenance::Explicit,
            capture_floor: 0,
            launch_id: "missing-gem".to_owned(),
            launch_marker: Some("GEMINI"),
            config_home: crate::meta::RecordedConfigHome::Missing,
        };
        assert_eq!(scan_gemini(&home, &facts), None);
    }

    #[test]
    fn inherited_gemini_with_missing_token_keeps_cwd_fallback() {
        let root = scratch("gem-inh-miss");
        let home = root.join("home");
        let seat = root.display().to_string();
        gemini_chat(&home, "digest", &seat, "sess-fallback", "");
        let facts = Facts {
            agent: "lead".to_owned(),
            tool: ToolKind::Gemini,
            work_dir: seat,
            provenance: crate::meta::SeatProvenance::Inherited,
            capture_floor: 0,
            launch_id: "missing-gem".to_owned(),
            launch_marker: Some("GEMINI"),
            config_home: crate::meta::RecordedConfigHome::Missing,
        };
        let got = scan_gemini(&home, &facts);
        assert_eq!(got.as_deref(), Some("sess-fallback"));
    }

    /// One agy home: a token db plus the cli log naming the seat.
    fn agy_home(home: &Path, seat: &str, dbid: &str, tok: &str, logid: &str) {
        let db = home.join(AGY_CONVERSATIONS).join(format!("{dbid}.db"));
        let marker = format!("AE_AGY_LAUNCH_ID={tok}");
        write_bytes(&db, marker.as_bytes());
        let log = home.join(AGY_LOGS).join("cli-1.log");
        let text = format!("workspaceDirs=[{seat}]\nCreated conversation {logid}\n");
        write(&log, &text);
    }

    #[test]
    fn inherited_agy_token_hit_and_cwd_fallback_stay_split_by_arm() {
        let root = scratch("agy-inh-arms");
        let home = root.join("home");
        let seat = root.display().to_string();
        agy_home(&home, &seat, "aa01", "tok-agy", "bb02");
        let base = Facts {
            agent: "lead".to_owned(),
            tool: ToolKind::Agy,
            work_dir: seat,
            provenance: crate::meta::SeatProvenance::Inherited,
            capture_floor: 0,
            launch_id: "tok-agy".to_owned(),
            launch_marker: Some("AGY"),
            config_home: crate::meta::RecordedConfigHome::Missing,
        };
        let got = scan_agy(&home, &base);
        assert_eq!(got.as_deref(), Some("aa01"));
        let empty = Facts {
            launch_id: String::new(),
            ..base
        };
        let got = scan_agy(&home, &empty);
        assert_eq!(got.as_deref(), Some("bb02"));
    }

    #[test]
    fn explicit_agy_without_token_stays_pending() {
        let root = scratch("agy-exp-empty");
        let home = root.join("home");
        let seat = root.display().to_string();
        agy_home(&home, &seat, "aa01", "tok-agy", "bb02");
        let facts = Facts {
            agent: "lead".to_owned(),
            tool: ToolKind::Agy,
            work_dir: seat,
            provenance: crate::meta::SeatProvenance::Explicit,
            capture_floor: 0,
            launch_id: String::new(),
            launch_marker: Some("AGY"),
            config_home: crate::meta::RecordedConfigHome::Missing,
        };
        assert_eq!(scan_agy(&home, &facts), None);
    }

    #[test]
    fn explicit_muse_token_hit_captures_its_directory() {
        let home = scratch("muse-exp-hit").join("home");
        let day = day_dir(Timestamp::now());
        let mine = "01a09b51-c88a-7fc0-8f71-200ea396c8a7";
        write_bytes(
            &home
                .join(MUSE_SESSIONS)
                .join(&day)
                .join(mine)
                .join("session.jsonl"),
            b"AE_MUSE_LAUNCH_ID=tok-m",
        );
        let facts = Facts {
            agent: "lead".to_owned(),
            tool: ToolKind::Muse,
            work_dir: home.display().to_string(),
            provenance: crate::meta::SeatProvenance::Explicit,
            capture_floor: 0,
            launch_id: "tok-m".to_owned(),
            launch_marker: Some("MUSE"),
            config_home: crate::meta::RecordedConfigHome::Missing,
        };
        assert_eq!(scan_muse(&home, &facts).as_deref(), Some(mine));
    }

    #[test]
    fn the_tui_fallback_reads_only_a_tokenless_inherited_seat() {
        use crate::meta::SeatProvenance as P;
        let calls = std::cell::Cell::new(0);
        let read = |_: &ServerId, _: &str| {
            calls.set(calls.get() + 1);
            None
        };
        let no = |p: P, t: &str| codex_tui_fallback(p, t, "%0", &ServerId::Ambient, read);
        assert!(no(P::Explicit, "t").is_none());
        assert_eq!(calls.get(), 0);
        assert!(no(P::Explicit, "").is_none());
        assert_eq!(calls.get(), 0);
        assert!(no(P::Inherited, "t").is_none());
        assert_eq!(calls.get(), 0);
        assert!(no(P::Inherited, "").is_none());
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn explicit_empty_register_sid_refuses_a_matching_rollout() {
        let dir = scratch("sid-exp-empty");
        let account = dir.join("account");
        rollout(
            &account,
            "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
            &dir.join("seat").display().to_string(),
            "2026-09-10T10:02:00Z",
            "",
        );
        write(
            &dir.join("meta"),
            &format!(
                "schema=2\nseat.main=lead\nagent_bin.main=codex\nharness_session.main=pending\n\
                 config_home.main={h}\nwork_dir.main={s}\n",
                h = account.display(),
                s = dir.join("seat").display(),
            ),
        );
        let before = std::fs::read(dir.join("meta")).expect("meta");
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = register_sid(&dir, "main", None, &mut out, &mut err).expect("exit");
        assert_eq!(code, crate::state::EXIT_FAILED);
        assert_eq!(std::fs::read(dir.join("meta")).expect("meta"), before);
        assert!(!dir.join("codex.main.sid").exists());
    }
}
