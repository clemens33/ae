//! Post-launch session-id capture for the tools with no launch-time id flag.
//!
//! Ported from `ae`'s `start_capture_session_id` / `capture_session_id` and the
//! three `capture_*_session_id` chains under them. Codex, opencode, gemini and
//! agy all learn their conversation id only after they start, so ae asks each
//! of them a different way:
//!
//! | Tool | How the id is found |
//! |---|---|
//! | codex | the `codex.<slot>.sid` file its own `developer_instructions` write, then a launch-token scan of `~/.codex/sessions/<day>/*.jsonl`, then a cwd scan of the same files, then its TUI header |
//! | opencode | `opencode session list --format json`, matched on the session's `directory` |
//! | gemini | `~/.gemini/tmp/<project>/chats/session-*.json`, matched on the launch token, then on the project root alone |
//! | agy | the launch token, searched in the BYTES of `~/.gemini/antigravity-cli/conversations/<id>.db` — OR, for a seat that has no token at all, the CLI log that names both the workspace and the conversation it created. Alternatives, not a chain: a token miss stays pending, because falling through cross-wires two seats sharing one directory |
//!
//! Every scan is filtered by the seat's `launch_time.<slot>`, so a stale
//! conversation in the same directory cannot be captured as this one.
//!
//! Runs in ITS OWN DETACHED PROCESS, never on the launch's thread: the frozen
//! path backgrounded it with `&` for the same reason, so a tool that takes half
//! a minute to print its id does not delay the attach.
//!
//! # The second caller: the watchdog's recovery
//!
//! A capture child can die before its tool answers — the machine sleeps, the
//! session is resumed, the process is killed with the pane it was launched
//! beside — and the seat then stays `pending` for the rest of the session. The
//! watchdog closes that gap: every cycle it takes ONE look at each pending seat
//! ([`pending_seats`] + [`attempt`]) and registers whatever it finds. No
//! sleeping and no polling there, because the next tick IS the retry.
//!
//! # What is NOT ported
//!
//! * **opencode's launch-token DB scan.** It queried the `part` table for
//!   `AE_OPENCODE_LAUNCH_ID`, and since opencode's context moved to
//!   `OPENCODE_CONFIG` nothing pastes that marker any more — the marker never
//!   reaches a message part (measured; AGENTS.md records it as inert). The
//!   directory match was already the effective path, so it is the only one
//!   here.
//! * **the legacy `<tool>_launch_id.<slot>` / `<tool>_launch_time.<slot>` meta
//!   keys.** The core writes only the generic spelling, and the Rust line never
//!   reads pre-Rust state.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::inventory::ServerId;
use crate::launch_cmd::ToolKind;
use crate::time::Timestamp;

/// How many times a capture looks — the frozen `for _attempt in 1..6`.
const POLLS: u32 = 6;

/// The pause between looks — the frozen `sleep 5`.
const POLL: Duration = Duration::from_secs(5);

/// How many sessions `opencode session list` is asked for — the frozen `-n 20`.
const OPENCODE_LIST_LIMIT: &str = "20";

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
///
/// Sealed the way [`crate::procs::PsArgv`] is: the inner vector is private to
/// this module and the one constructor takes no caller input at all, so the
/// `opencode` leg of the process door cannot be alias-imported and handed an
/// arbitrary command line.
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

/// The argv that captures one target: `_capture-sid <dir> <slot> <pane>`.
///
/// The TOOL is deliberately absent: the capture child reads it back out of
/// `agent_bin.<slot>`, so the roster the core owns stays the one answer to
/// "what is sitting in that seat" rather than a fact copied into an argv.
fn argv(dir: &Path, target: &Target) -> CaptureArgv {
    CaptureArgv(vec![
        crate::cli::CAPTURE_SID.to_owned(),
        dir.display().to_string(),
        target.slot.clone(),
        target.pane.clone(),
    ])
}

/// Start one DETACHED capture per target that needs one.
///
/// A child process, not a thread: the launch returns as soon as the session is
/// up — `--no-attach` returns immediately — and a thread would die with it,
/// which is exactly why the frozen path backgrounded a subshell with `&`. A
/// capture that never answers leaves the slot `pending`, which is what
/// `pending` means.
pub(crate) fn start(dir: &Path, targets: &[Target]) {
    // RESOLVED, never raw: this becomes a detached child's `argv[0]`, and on
    // macOS an unresolved answer is whichever link the caller typed — a helper
    // name, which the shim dispatch would read as that helper instead of
    // `_capture-sid`.
    let Some(exe) = crate::shape::resolved_exe() else {
        return;
    };
    for target in targets {
        if !crate::launch::supports_launch_id(target.tool) {
            continue;
        }
        let _ = crate::transport::spawn_detached(&exe, &argv(dir, target));
    }
}

/// `_capture-sid <dir> <slot> <pane>` — the detached child's whole job.
///
/// Answers 0 whatever happens: nothing reads its status, and a capture that
/// found no id is not a failure of the session it was launched for.
pub fn run(dir: &Path, slot: &str, pane: &str, server: &ServerId) -> u8 {
    let Some(facts) = facts(dir, slot) else {
        return 0;
    };
    let home = home_dir();
    let captured = match facts.tool {
        ToolKind::Codex => capture_codex(dir, slot, pane, server, home.as_deref(), &facts),
        ToolKind::OpenCode => capture_opencode(&facts),
        ToolKind::Gemini => home
            .as_deref()
            .and_then(|home| capture_gemini(home, &facts)),
        ToolKind::Agy => home.as_deref().and_then(|home| capture_agy(home, &facts)),
        _ => None,
    };
    if let Some(id) = captured {
        register(dir, slot, &id);
    }
    0
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

/// The seats one recovery pass tries: an id still unrecorded, in a seat whose
/// tool has no launch-time id flag.
///
/// Both halves are the frozen `walk_pending_session_ids`' selection, ported to
/// the roster the core owns: it took `harness_session.<slot>` rows reading
/// `pending` and kept only codex, gemini and opencode — claude and grok launch
/// with an ae-generated id, so a seat holding one is never waiting for a
/// capture. The empty spelling counts as pending beside the frozen literal:
/// [`crate::meta`] reads an empty metadata row as ABSENT metadata, so a seat
/// whose id was never written and one whose row says `pending` reach here as
/// `None` and `Some("pending")` for the same reason.
#[must_use]
pub fn pending_seats(roster: &[crate::meta::RosterEntry]) -> Vec<Pending> {
    roster
        .iter()
        .filter(|entry| is_pending(entry.harness_session.as_deref()))
        .filter_map(|entry| {
            let tool = ToolKind::from_binary_name(entry.binary.as_deref().unwrap_or_default());
            crate::launch::supports_launch_id(tool).then(|| Pending {
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
/// What the watchdog runs per tick, and the reason it is not [`run`]: the
/// launch's capture may sleep for half a minute because it is a detached child
/// nobody waits on, while this is a step inside a daemon cycle that must not
/// hold up a nudge. There is no polling to lose — the watchdog ticks again.
///
/// The methods are the frozen `doctor_try_capture_session_id`'s: the history
/// scans, and only those. Codex's `codex.<slot>.sid` handshake is excluded
/// because the file carries no launch time of its own, so a leftover from an
/// earlier launch of the same slot would read as this one's; the TUI scrape is
/// excluded because it is a guess at a header, and because a recovery must work
/// for a seat whose pane is long past its banner.
///
/// Within a tool, the chain is the CAPTURE's, not the frozen recovery's — one
/// scan per tool, shared with [`run`], rather than two that can drift apart.
/// Two arms are therefore wider here than in bash: gemini falls back to the
/// project root when the seat carries no launch token, and opencode matches on
/// the session's directory rather than on a token nothing pastes any more
/// (AGENTS.md records that scan as inert). Both are already what the launch's
/// own child does, and both stay floored by `launch_time.<slot>` and rooted in
/// this session's working directory, so neither widens what a capture may
/// match — only which caller may match it.
#[must_use]
pub fn attempt(dir: &Path, slot: &str) -> Option<String> {
    let facts = facts(dir, slot)?;
    let home = home_dir();
    match facts.tool {
        ToolKind::Codex => home.as_deref().and_then(|home| scan_codex(home, &facts)),
        ToolKind::Gemini => home.as_deref().and_then(|home| scan_gemini(home, &facts)),
        ToolKind::Agy => home.as_deref().and_then(|home| scan_agy(home, &facts)),
        ToolKind::OpenCode => scan_opencode(&facts),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// what the capture reads about itself
// ---------------------------------------------------------------------------

/// What one seat's capture needs to know, all of it from the session's meta.
struct Facts {
    /// Which harness the seat holds — read from `agent_bin.<slot>`, because the
    /// roster is the core's record of what a seat is.
    tool: ToolKind,
    /// The session's working directory, which every cwd match compares against.
    work_dir: String,
    /// The launch instant, in epoch seconds. `0` when unrecorded, which widens
    /// the scan rather than closing it — the frozen default.
    launch_time: i64,
    /// The launch token, empty when none was minted.
    launch_id: String,
}

/// Read one seat's capture facts, or nothing when the meta cannot be read.
fn facts(dir: &Path, slot: &str) -> Option<Facts> {
    let bytes = crate::meta::read_bytes(dir).ok()?;
    let value = |key: &str| {
        crate::meta::first_value(&bytes, key)
            .map(|raw| String::from_utf8_lossy(raw).into_owned())
            .unwrap_or_default()
    };
    let launch_time = value(&format!("launch_time.{slot}"));
    Some(Facts {
        tool: ToolKind::from_binary_name(&value(&format!("agent_bin.{slot}"))),
        work_dir: value("work_dir"),
        // A non-numeric value is 0, never a refusal: the frozen reader made the
        // same choice, and a scan with no lower bound still cannot pick a
        // session in another directory.
        launch_time: if launch_time.bytes().all(|byte| byte.is_ascii_digit()) {
            launch_time.parse().unwrap_or(0)
        } else {
            0
        },
        launch_id: value(&format!("launch_id.{slot}")),
    })
}

/// The caller's own `HOME`, where every tool keeps its conversation history.
///
/// Not `AE_HOME`: these are the TOOL's directories, not ae's state root, and
/// the two are unrelated even when both are derived from the same variable.
fn home_dir() -> Option<PathBuf> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the tool history directories a capture scans live under the caller's HOME — see clippy.toml"
    )]
    let raw = std::env::var_os("HOME");
    raw.filter(|value| !value.is_empty()).map(PathBuf::from)
}

/// Write the captured id into the roster, under the meta lock the core holds.
pub(crate) fn register(dir: &Path, slot: &str, id: &str) {
    let tail = [
        "set-harness-session".to_owned(),
        slot.to_owned(),
        id.to_owned(),
    ];
    let mut out = Vec::new();
    let mut err = Vec::new();
    let _ = crate::identity::roster(dir, &tail, &mut out, &mut err);
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
/// THE LINK THAT WAS DEAD. Codex's `developer_instructions` have always told it
/// to run `<session-dir>/_register-sid <slot>` as its first action, and
/// [`capture_codex`] has always polled for what that run writes — but the shim
/// left the helper set with the `declare -f` template library, and no core
/// entry replaced it, so the instruction named a file that was not there and
/// every capture fell through to the scans below it.
///
/// Two shapes, because a codex that KNOWS its id and one that does not are
/// different facts:
///
/// * **With an id** the caller is the authority: the id is validated as a
///   lowercase UUID and written, with no meta read at all. Validation is not
///   politeness — the value lands in a file the capture reads back as a session
///   id and writes into the roster, so a malformed one would pin a seat to a
///   conversation that does not exist.
/// * **Without one** the core does what the frozen helper did: scans codex's
///   own history for this seat, floored by `launch_time.<slot>` and filtered by
///   the launch token, and writes what it finds. That is [`scan_codex`], which
///   the capture and the watchdog's recovery already share.
///
/// The write is temp + rename, so the polling capture never reads a half-written
/// id.
///
/// The INSTRUCTION deliberately names only the seat, never the id: a model
/// asked for its own session id can produce a well-formed one it invented, and
/// a validator sees shape, not truth — a hallucinated UUID would pin the seat
/// to a conversation that does not exist, which is worse than falling through
/// to the scan. The id argument is for a caller that has one from somewhere
/// trustworthy.
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
    let id = if let Some(given) = id {
        let given = given.trim();
        if !is_lowercase_uuid(given) {
            writeln!(
                err,
                "Error: '{given}' is not a lowercase UUID — a session id is 8-4-4-4-12 hex."
            )?;
            return Ok(crate::state::EXIT_USAGE);
        }
        given.to_owned()
    } else {
        let Some(facts) = facts(dir, slot).filter(|facts| facts.tool == ToolKind::Codex) else {
            writeln!(
                err,
                "Error: seat '{slot}' is not a codex seat in {}.",
                dir.display()
            )?;
            return Ok(crate::state::EXIT_USAGE);
        };
        let Some(found) = home_dir().and_then(|home| scan_codex(&home, &facts)) else {
            writeln!(err, "No codex session matched seat '{slot}' yet.")?;
            return Ok(crate::state::EXIT_FAILED);
        };
        found
    };
    let file = sid_file(dir, slot);
    if let Err(why) = super::assets::publish_document(&file, &format!("{id}\n")) {
        writeln!(err, "Error: could not write {} ({why})", file.display())?;
        return Ok(crate::state::EXIT_FAILED);
    }
    writeln!(out, "Registered session id for '{slot}'.")?;
    Ok(0)
}

/// The refusal `_register-sid` prints for a missing seat.
pub const REGISTER_SID_USAGE: &str = "Usage: _register-sid <meta-dir> <slot> [<session-id>]";

/// Whether `value` is a lowercase 8-4-4-4-12 hex UUID.
///
/// Lowercase deliberately: ae's ids and every agent history filename are
/// lowercase-only, and macOS `uuidgen` is not — so accepting either spelling
/// would let a value be recorded that no scan can ever match again.
#[must_use]
fn is_lowercase_uuid(value: &str) -> bool {
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

/// Poll for the self-registered id, then the two history scans, then the TUI.
///
/// The order is the frozen chain's: the handshake is what ae ASKED for, the
/// scans are what it can find without being told, and the scrape is a guess at
/// a header. The handshake sleeps BEFORE its first look, unlike the scans
/// below — codex cannot have written the file at t=0, and an immediate read
/// could only find a leftover from an earlier launch of the same slot.
fn capture_codex(
    dir: &Path,
    slot: &str,
    pane: &str,
    server: &ServerId,
    home: Option<&Path>,
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
            let _ = std::fs::remove_file(&file);
            let id: String = text.chars().filter(|c| !c.is_whitespace()).collect();
            if !id.is_empty() {
                return Some(id);
            }
        }
    }
    if let Some(id) = home.and_then(|home| scan_codex(home, facts)) {
        return Some(id);
    }
    // The TUI scrape, least reliable and therefore last: codex prints
    // `session id: <uuid>` once in its header.
    let screen = crate::transport::capture_pane(server, pane)?;
    scrape_session_id(&screen)
}

/// One look through codex's own history: the launch token first, this
/// directory's newest conversation second.
///
/// Split out of [`capture_codex`] because it is the half that neither sleeps
/// nor needs a pane, which is exactly the half [`attempt`] may run.
fn scan_codex(home: &Path, facts: &Facts) -> Option<String> {
    let days = day_dirs(Timestamp::now());
    if !facts.launch_id.is_empty()
        && let Some(id) = find_codex_by_launch_id(home, &facts.launch_id, facts.launch_time, &days)
    {
        return Some(id);
    }
    if facts.work_dir.is_empty() {
        return None;
    }
    find_codex_by_cwd(home, &facts.work_dir, facts.launch_time, &days)
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
///
/// `days` are the `YYYY/MM/DD` directories to look in — today and yesterday, so
/// a launch either side of UTC midnight is still found.
#[must_use]
pub(crate) fn find_codex_by_launch_id(
    home: &Path,
    launch_id: &str,
    launch_time: i64,
    days: &[String],
) -> Option<String> {
    let marker = format!("AE_CODEX_LAUNCH_ID={launch_id}");
    newest(codex_logs(home, days), launch_time, |text| {
        if !text.contains(&marker) {
            return None;
        }
        first_hex_field(text.lines().next().unwrap_or_default(), "id")
    })
}

/// The newest codex session whose recorded `cwd` is this working directory.
#[must_use]
pub(crate) fn find_codex_by_cwd(
    home: &Path,
    work_dir: &str,
    launch_time: i64,
    days: &[String],
) -> Option<String> {
    let target = canonical(work_dir);
    newest(codex_logs(home, days), launch_time, |text| {
        let first = text.lines().next().unwrap_or_default();
        let cwd = first_string_field(first, "cwd")?;
        if canonical(&cwd) != target {
            return None;
        }
        first_hex_field(first, "id")
    })
}

/// Every `*.jsonl` under the named day directories of `~/.codex/sessions`.
fn codex_logs(home: &Path, days: &[String]) -> Vec<PathBuf> {
    let root = home.join(".codex").join("sessions");
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
// gemini
// ---------------------------------------------------------------------------

/// Poll gemini's local chat history: the launch token first, the project root
/// alone as the fallback.
///
/// Unlike codex's handshake this LOOKS FIRST and sleeps only between attempts.
/// A scan cannot return a stale answer — every candidate must be newer than
/// `launch_time.<slot>` and rooted in this session's own working directory — so
/// the frozen leading `sleep 5` bought nothing but latency.
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
        && let Some(id) =
            find_gemini_by_launch_id(home, &facts.work_dir, &facts.launch_id, facts.launch_time)
    {
        return Some(id);
    }
    find_gemini_by_cwd(home, &facts.work_dir, facts.launch_time)
}

/// The newest gemini chat for this project whose file carries the launch token.
#[must_use]
pub(crate) fn find_gemini_by_launch_id(
    home: &Path,
    work_dir: &str,
    launch_id: &str,
    launch_time: i64,
) -> Option<String> {
    let marker = format!("AE_GEMINI_LAUNCH_ID={launch_id}");
    newest(gemini_chats(home, work_dir), launch_time, |text| {
        if !text.contains(&marker) {
            return None;
        }
        first_string_field(text, "sessionId")
    })
}

/// The newest gemini chat for this project, whichever launch wrote it.
#[must_use]
pub(crate) fn find_gemini_by_cwd(home: &Path, work_dir: &str, launch_time: i64) -> Option<String> {
    newest(gemini_chats(home, work_dir), launch_time, |text| {
        first_string_field(text, "sessionId")
    })
}

/// Every `chats/session-*.json` under a `~/.gemini/tmp/<project>` whose
/// `.project_root` names this working directory.
///
/// The project directory name is a digest gemini computes, so it is never
/// derived here — the `.project_root` file inside it is the only claim about
/// which directory a chat belongs to.
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
        // The frozen glob is `session-*.json`, and both halves are
        // case-sensitive: the extension is compared as a path component rather
        // than as a string suffix so it stays exactly that.
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
///
/// One `SQLite` file per conversation, named for the id, in one flat directory —
/// so the FILE STEM is the id and nothing has to be parsed out of the database.
/// Shared with the resume probe in [`crate::run`], which asks the same
/// directory whether a recorded id still names a conversation.
pub const AGY_CONVERSATIONS: &str = ".gemini/antigravity-cli/conversations";

/// agy's per-process CLI log directory, relative to the caller's `HOME`.
pub(crate) const AGY_LOGS: &str = ".gemini/antigravity-cli/log";

/// Poll agy's conversation store: the launch token first, its CLI log second.
///
/// Looks first and sleeps only between attempts, for the same reason as
/// gemini's scan — every candidate is floored by `launch_time.<slot>`, so an
/// immediate look cannot return a stale answer and a leading sleep buys
/// nothing but latency.
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

/// One look for this seat's agy conversation. The two halves are ALTERNATIVES,
/// chosen by whether the seat has a launch token — never a chain.
///
/// They read DIFFERENT FILES because agy splits the two facts across them
/// (measured 2026-09-04, 1.1.25): the conversation database carries the injected
/// context — and so the launch token — but never the working directory, while
/// the CLI log carries `workspaceDirs=[…]` and the id of the conversation that
/// run created, but never the token.
///
/// # A token miss is PENDING, never a fallback
///
/// This was a shipped defect, and the shape of it is worth keeping: the token
/// search used to fall THROUGH to the workspace search on a miss. Two agy seats
/// in one working directory, started in the same second, then had one positive
/// answer between them — the workspace — and the seat that had not yet written
/// its token registered its SIBLING's conversation. From there the roster is
/// cross-wired and the resume puts two agents into one transcript.
///
/// So: a seat that HAS a token is answered by the token and by nothing else.
/// A miss costs a cycle, not a conversation — the token lands in the database as
/// soon as the first turn persists, and the watchdog looks again every tick.
/// The workspace search is for a seat with NO token at all (a legacy meta, a
/// seat whose launch predates the marker), where it is the only question that
/// can be asked, and even there it refuses to guess between two candidates.
fn scan_agy(home: &Path, facts: &Facts) -> Option<String> {
    if !facts.launch_id.is_empty() {
        return find_agy_by_launch_id(home, &facts.launch_id, facts.launch_time);
    }
    if facts.work_dir.is_empty() {
        return None;
    }
    find_agy_by_cwd(home, &facts.work_dir, facts.launch_time)
}

/// The newest agy conversation whose database carries the launch token.
///
/// The token is searched for in the file's BYTES, not its text: a conversation
/// database is `SQLite` and is not valid UTF-8, so the reader every other scan
/// uses answers `None` for all of them. No working-directory filter is applied
/// or needed — the token is minted per launch, so a database holding it is this
/// seat's conversation wherever it was started.
#[must_use]
pub(crate) fn find_agy_by_launch_id(
    home: &Path,
    launch_id: &str,
    launch_time: i64,
) -> Option<String> {
    let marker = format!("AE_AGY_LAUNCH_ID={launch_id}").into_bytes();
    let mut best: Option<(i64, String)> = None;
    let mut candidates = agy_conversations(home);
    candidates.sort();
    for path in candidates {
        let Some(at) = mtime(&path) else {
            continue;
        };
        if at < launch_time {
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
///
/// The log is the only place agy records which directory a conversation belongs
/// to. One log per CLI process, plain UTF-8, carrying `workspaceDirs=[<dir>]`
/// once at start-up and `Created conversation <id>` for each conversation that
/// run began. The FIRST created id of a run is this launch's, because a launch's
/// own conversation is the first one its process creates; a later one is a
/// conversation somebody started by hand in the same pane.
///
/// # Exactly one, or none
///
/// A workspace does not identify a SEAT — two agy agents in one ae session share
/// a working directory — so where two runs match, this cannot say which is the
/// caller's and does not try. It is not "the newest wins": between two seats
/// started in the same second, newest is a coin toss whose losing side
/// cross-wires the roster. Candidates are counted by conversation id (two logs
/// naming the same conversation are one candidate, which is what a re-read of
/// the same run looks like), and two distinct ids answer `None`. Pending is
/// recoverable; a wrong id is not.
#[must_use]
pub(crate) fn find_agy_by_cwd(home: &Path, work_dir: &str, launch_time: i64) -> Option<String> {
    let target = canonical(work_dir);
    let mut candidates: Vec<String> = Vec::new();
    for path in agy_logs(home) {
        let Some(at) = mtime(&path) else {
            continue;
        };
        if at < launch_time {
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
///
/// agy can be given several directories (`--add-dir`), so the field is a list
/// and every entry is compared canonically — the same rule gemini's
/// `.project_root` match uses, for the same reason.
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
///
/// The `-wal` and `-shm` siblings `SQLite` writes beside a live database are
/// excluded by the extension test: neither is a conversation, and a stem
/// ending in `.db-wal` is not an id.
fn agy_conversations(home: &Path) -> Vec<PathBuf> {
    entries(&home.join(AGY_CONVERSATIONS))
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "db"))
        .collect()
}

/// A conversation database's id: its file stem, when that reads like one.
///
/// The class is the same one the codex scan uses on a JSON value, and it is not
/// decoration: it is what stops a stray `notes.db` in the same directory from
/// being registered as a conversation id.
fn agy_conversation_id(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let looks_like_an_id = !stem.is_empty()
        && stem
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-');
    looks_like_an_id.then(|| stem.to_owned())
}

/// Every `cli-*.log` in agy's log directory.
///
/// `cli.log` itself is a symlink to the newest one and is skipped by the
/// prefix test — following it would read the same file twice under two mtimes.
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

/// The most a single conversation database is worth scanning for a token.
///
/// A conversation grows without bound — the operator's own store held one at
/// 6.7 MB after a day's work — and this search runs SIX TIMES per launch and
/// again on every watchdog tick. Slurping the file was the first shape and it is
/// the wrong one twice over: it holds the whole database in memory at once, and
/// it pays for the whole file when the marker is in the first kilobyte.
///
/// So the scan streams, stops at the first match, and refuses a file larger than
/// this outright rather than reading it in full. 16 MiB is well above the
/// largest conversation measured (6.7 MB, 2026-09-04) and far below a size worth
/// walking repeatedly; a database past it is a conversation ae will not identify
/// by token, which costs a `pending` seat and never a wrong one.
const AGY_SCAN_CAP: u64 = 16 * 1024 * 1024;

/// How much of a conversation database is held at once while scanning it.
const AGY_SCAN_CHUNK: usize = 64 * 1024;

/// Does `path` contain `needle`, reading at most [`AGY_SCAN_CAP`] bytes?
///
/// The text reader cannot serve here: a conversation database is `SQLite` and is
/// not valid UTF-8, so `read_to_string` answers `None` for every one of them.
///
/// Read in chunks, keeping `needle.len() - 1` bytes of the previous chunk in
/// front of each new one, so a marker LYING ACROSS A CHUNK BOUNDARY is still
/// found — the defect a naive chunked search ships with, and the reason the
/// overlap is derived from the needle rather than fixed.
fn file_contains(path: &Path, needle: &[u8]) -> bool {
    use std::io::Read as _;

    // ONE stat, TWO facts, and both are decided BEFORE the open.
    //
    // `is_file` is not a tidiness check: `open(2)` on a FIFO BLOCKS until a
    // writer appears, and this runs inside the watchdog's cycle. A named pipe
    // called `<uuid>.db` in the store reports a length of 0, would sail through
    // a size-only gate, and hang the daemon on the open — so the node is
    // classified first and anything that is not a regular file is not opened at
    // all. Symlinks are FOLLOWED here deliberately (`metadata`, not
    // `symlink_metadata`): what must not block is the open, and the open
    // reaches the target, so the target is what has to be regular.
    //
    // What a pre-check cannot close is the race — the node could be replaced
    // between the stat and the open — and there is no non-blocking open in std
    // to close it with. Named rather than papered over: the store is the
    // caller's own home, so the exposure is a machine already writing there.
    let Some((regular, size)) = file_facts(path) else {
        return false;
    };
    if !regular {
        return false;
    }
    if size > AGY_SCAN_CAP {
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
    // THE STAT IS NOT THE BOUND. A conversation is a LIVE database: it can grow
    // between the stat above and the last read below, and a file that measured
    // 1 KB can feed this loop for as long as agy keeps writing. `take` is the
    // bound that holds whatever the file does — one byte past the cap, so
    // exceeding it is observable rather than indistinguishable from a file that
    // ends exactly there.
    match scan_stream(file.take(AGY_SCAN_CAP.saturating_add(1)), needle) {
        Scan::Found => true,
        Scan::Absent => false,
        Scan::OverCap => {
            skipped(path, AGY_SCAN_CAP.saturating_add(1));
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

/// Search `reader` for `needle` in chunks, spending at most [`AGY_SCAN_CAP`]
/// bytes.
///
/// Takes a READER rather than a path so the budget is provable without a file
/// that lies about its length: an endless stream is the shape a growing
/// database presents, and the only honest way to show the loop ends is to run
/// it against one.
fn scan_stream<R: std::io::Read>(mut reader: R, needle: &[u8]) -> Scan {
    let Some(overlap) = needle.len().checked_sub(1) else {
        return Scan::Absent;
    };
    let mut buffer = vec![0_u8; overlap + AGY_SCAN_CHUNK];
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
        if consumed > AGY_SCAN_CAP {
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
        // straddling the seam sits contiguously in the next pass. A read too
        // short to fill even the carry is left to accumulate instead.
        if filled > overlap {
            buffer.copy_within(filled - overlap..filled, 0);
            filled = overlap;
        }
    }
}

/// Say that a conversation was too big to search, and where.
///
/// Visible where a look can still be retried: the watchdog's recovery runs in
/// the daemon, whose stderr reaches its pane. The launch's own capture is
/// detached with its streams dropped, so there it is silent — said plainly
/// rather than pretended otherwise.
fn skipped(path: &Path, size: u64) {
    eprintln!(
        "ae: agy capture skipped {} ({size} bytes over the {AGY_SCAN_CAP}-byte scan cap)",
        path.display()
    );
}

/// Whether `path` is a regular file, and how long it is — from ONE stat, and
/// without opening the node.
///
/// The two facts travel together because both gate the same open and a second
/// stat would be a second answer to race against.
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

/// Poll `opencode session list` for a session in this working directory.
///
/// Looks first and sleeps between attempts, for the same reason as gemini's
/// scan: the `updated` filter makes an immediate look safe.
fn capture_opencode(facts: &Facts) -> Option<String> {
    for attempt in 0..POLLS {
        if attempt > 0 {
            std::thread::sleep(POLL);
        }
        if let Some(id) = scan_opencode(facts) {
            return Some(id);
        }
    }
    None
}

/// One `opencode session list`, read for a session in this working directory.
fn scan_opencode(facts: &Facts) -> Option<String> {
    if facts.work_dir.is_empty() {
        return None;
    }
    // opencode timestamps are MILLISECONDS. Saturating, because a meta holding
    // a nonsense epoch must not overflow into a negative lower bound that
    // matches everything.
    let since = facts.launch_time.saturating_mul(1000);
    // A failed run is "no answer", never an empty one: opencode may not be
    // installed at all, which the frozen path checked with `command -v`.
    let (ran, listed) = crate::transport::run_opencode(&opencode_list_argv());
    if !ran {
        return None;
    }
    pick_opencode_session(&listed, &facts.work_dir, since)
}

/// The newest session in `listed` whose `directory` is `work_dir` and whose
/// `updated` is at or after `since` (milliseconds).
///
/// `listed` is opencode's own `--format json`. It is split into per-session
/// records and read field by field rather than parsed as one document, exactly
/// as the frozen path did: the interesting keys (`id`, `directory`, `updated`)
/// are read as the first occurrence in each record, so a nested `time.updated`
/// is found without this code knowing opencode's schema — and a log line
/// printed before the JSON cannot make the whole answer unreadable.
#[must_use]
pub(crate) fn pick_opencode_session(listed: &str, work_dir: &str, since: i64) -> Option<String> {
    let target = canonical(work_dir);
    let mut best: Option<(i64, String)> = None;
    for record in json_records(listed) {
        let Some(id) = first_string_field(record, "id") else {
            continue;
        };
        let Some(directory) = first_string_field(record, "directory") else {
            continue;
        };
        let Some(updated) = first_num_field(record, "updated") else {
            continue;
        };
        if updated < since || canonical(&directory) != target {
            continue;
        }
        if best.as_ref().is_none_or(|(seen, _)| updated > *seen) {
            best = Some((updated, id));
        }
    }
    best.map(|(_, id)| id)
}

/// Split a JSON array of objects into its records, on the `},{` boundary.
///
/// Newlines are folded out first, so a pretty-printed array splits the same way
/// a compact one does. The record text keeps its brackets and braces — nothing
/// downstream reads them.
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
/// codex uses, and the frozen `_ae_yesterday` chokepoint's whole job.
///
/// Yesterday is included because a launch shortly before UTC midnight writes
/// into the previous day's directory while the capture runs in the next one.
fn day_dirs(now: Timestamp) -> Vec<String> {
    [
        now,
        Timestamp::from_epoch(now.epoch().saturating_sub(86_400)),
    ]
    .iter()
    .map(|at| {
        at.to_string()
            .get(..10)
            .map(|day| day.replace('-', "/"))
            .unwrap_or_default()
    })
    .collect()
}

/// The candidate with the greatest mtime whose text `read` accepts.
///
/// Candidates older than `launch_time` are skipped before anything is read: a
/// conversation that predates this launch is not this launch's, however well it
/// matches. Ties keep the first in sorted order, which is the glob order the
/// frozen loops saw.
fn newest<F>(mut candidates: Vec<PathBuf>, launch_time: i64, read: F) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    candidates.sort();
    let mut best: Option<(i64, String)> = None;
    for path in candidates {
        let Some(at) = mtime(&path) else {
            continue;
        };
        if at < launch_time {
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
///
/// Canonical rather than raw: a worktree reached through a symlink and the same
/// place spelled with a trailing slash are one directory, and the frozen path
/// ran `realpath` on both sides of every comparison for exactly that reason.
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
/// non-quote bytes — the frozen `_ae_json_first`.
#[must_use]
pub(crate) fn first_string_field(text: &str, key: &str) -> Option<String> {
    first_field(text, key, |_| true)
}

/// The first `"key": "<value>"` in `text` whose value is only hex digits and
/// dashes — the frozen `_ae_json_first <key> '[0-9a-f-]'`.
///
/// The class is not decoration: it is what stops a `"id": "session_meta"` row
/// from being read as a conversation id.
#[must_use]
pub(crate) fn first_hex_field(text: &str, key: &str) -> Option<String> {
    first_field(text, key, |ch| {
        ch.is_ascii_digit() || matches!(ch, 'a'..='f' | '-')
    })
}

/// The first `"key": <digits>` in `text` — the frozen `_ae_json_first_num`.
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
///
/// It keeps looking after a rejected match, because the frozen `grep -oE`
/// did: a pattern that fails at one occurrence of the key matches at the next,
/// and a value outside the class is a non-match rather than an answer of
/// "absent".
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
/// by one. Blanks are spaces and tabs only — a JSON member never spans a line
/// here, and the frozen per-line `grep` could not either.
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
            harness_session: id.map(ToOwned::to_owned),
            binary: Some(binary.to_owned()),
            schema: crate::meta::RosterSchema::V2,
        }
    }

    #[test]
    fn a_recovery_takes_the_pending_seats_whose_tool_has_no_launch_time_id() {
        let roster = [
            // Pending, both spellings: the frozen literal and the empty row a
            // v2 meta reads as absent metadata.
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
    }

    #[test]
    fn an_opencode_list_picks_the_newest_session_in_this_directory() {
        let dir = scratch("oc");
        let work = dir.display().to_string();
        let listed = format!(
            r#"[{{"id":"ses_old","directory":"{work}","time":{{"created":1,"updated":2000}}}},
               {{"id":"ses_new","directory":"{work}","time":{{"created":1,"updated":9000}}}},
               {{"id":"ses_elsewhere","directory":"/nowhere","time":{{"updated":9999}}}}]"#
        );
        assert_eq!(
            pick_opencode_session(&listed, &work, 1000).as_deref(),
            Some("ses_new")
        );
        // The launch-time floor excludes every session that predates it.
        assert_eq!(pick_opencode_session(&listed, &work, 10_000), None);
        // A directory that is not this session's is never captured.
        assert_eq!(
            pick_opencode_session(&listed, "/nowhere", 1000).as_deref(),
            Some("ses_elsewhere")
        );
        // Nothing parseable is nothing captured, never a panic.
        assert_eq!(pick_opencode_session("", &work, 0), None);
        assert_eq!(
            pick_opencode_session("opencode: not logged in", &work, 0),
            None
        );
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

        // A conversation database is `SQLite`: BINARY, and not valid UTF-8. The
        // text reader every other scan uses answers None for it, so the byte
        // search is the capability under test and not an implementation detail.
        write_bytes(
            &store.join(format!("{id}.db")),
            b"SQLite format 3\x00\xff\xfe AE_AGY_LAUNCH_ID=tok-1 \xc3\x28",
        );
        // Another launch's conversation, and the sidecars SQLite writes beside
        // a live database. Neither is this seat's.
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
            find_agy_by_launch_id(&home, "tok-1", 0).as_deref(),
            Some(id)
        );
        assert_eq!(find_agy_by_launch_id(&home, "tok-2", 0), None);
        // The log fallback takes the FIRST conversation the matching run
        // created, never a later hand-started one and never another workspace's.
        assert_eq!(find_agy_by_cwd(&home, &work, 0).as_deref(), Some(id));
        // The launch-time floor keeps a conversation that predates this launch
        // out of BOTH halves.
        let future = i64::MAX / 2;
        assert_eq!(find_agy_by_launch_id(&home, "tok-1", future), None);
        assert_eq!(find_agy_by_cwd(&home, &work, future), None);
        // A home with no agy state at all is quiet.
        assert_eq!(find_agy_by_cwd(&root.join("empty"), &work, 0), None);
        assert_eq!(find_agy_by_launch_id(&root.join("empty"), "tok-1", 0), None);
    }

    #[test]
    fn two_agy_seats_in_one_directory_stay_pending_rather_than_take_each_other_s() {
        // THE CROSS-WIRING DEFECT, pinned. Two agy seats in one ae session share
        // a working directory. The sibling has already written its conversation
        // and its log; THIS seat has a token and nothing on disk yet. A chain
        // that fell through from the token miss to the workspace search answered
        // with the sibling's id, and the roster then pointed two agents at one
        // transcript — which a resume makes permanent.
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
            tool: ToolKind::Agy,
            work_dir: work.clone(),
            launch_time: 0,
            launch_id: "own-token".to_owned(),
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
        // bound on what a read loop will be handed. The endless stream is that
        // hazard in its purest form: without the budget this call does not
        // return, so the test passing at all IS the assertion.
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
        // the daemon. A FIFO cannot be made here (`mkfifo(1)` needs a child
        // process, and `std::process::Command` is a crate-wide disallowed type
        // whose only doors are in the it-target), so the FIFO itself is pinned
        // by `capture::an_agy_fifo_in_the_store_is_skipped_and_does_not_block`
        // over there. What is constructible here is the rest of the class, and
        // the guard does not distinguish between them: it opens a regular file
        // and nothing else.
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
        // between two reads. One offset does not prove it — a marker that lands
        // just inside the first buffer is found by a scan with no carry at all —
        // so the marker is walked across EVERY position that crosses the seam,
        // and a scan that drops its overlap misses at least one of them.
        // Verified against a deliberately broken carry before this was trusted.
        let root = scratch("agy-chunks");
        let marker = b"AE_AGY_LAUNCH_ID=tok-1";
        let seam = marker.len() - 1 + AGY_SCAN_CHUNK;
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
        let cap = usize::try_from(AGY_SCAN_CAP).unwrap_or(usize::MAX);
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
            find_gemini_by_launch_id(&home, &work, "tok-1", 0).as_deref(),
            Some("gem-a")
        );
        assert_eq!(find_gemini_by_launch_id(&home, &work, "tok-2", 0), None);
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
        let day = home.join(".codex").join("sessions").join("2026/09/03");
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
            find_codex_by_launch_id(&home, "tok-1", 0, &days).as_deref(),
            Some("c0de-01")
        );
        assert_eq!(find_codex_by_launch_id(&home, "tok-2", 0, &days), None);
        assert_eq!(
            find_codex_by_cwd(&home, &work, 0, &days).as_deref(),
            Some("c0de-01")
        );
        assert_eq!(
            find_codex_by_cwd(&home, "/nowhere", 0, &days).as_deref(),
            Some("c0de-02")
        );
        // A day that was never written is not an error.
        assert_eq!(
            find_codex_by_launch_id(&home, "tok-1", 0, &["2020/01/01".to_owned()]),
            None
        );
    }

    #[test]
    fn the_capture_facts_come_from_the_roster_and_a_bad_launch_time_is_zero() {
        let dir = scratch("facts");
        write(
            &dir.join("meta"),
            "session=s\nwork_dir=/w\nschema=2\nseat.main=lead\nagent_bin.main=opencode\n\
             launch_time.main=not-a-number\nlaunch_id.main=tok-9\n",
        );
        let read = facts(&dir, "main").expect("the meta reads");
        assert_eq!(read.tool, ToolKind::OpenCode);
        assert_eq!(read.work_dir, "/w");
        assert_eq!(read.launch_time, 0);
        assert_eq!(read.launch_id, "tok-9");
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
}
