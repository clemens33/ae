//! `ae compact`'s freeze/resolve step, in the pinned core.
//!
//! `_compact-freeze <session-dir> [--keep-history]` resolves everything the compact
//! boundary is authorized against — BEFORE anything is messaged, stopped or archived —
//! and emits it as one frozen tuple. It is PURE READ-ONLY: it reads meta and config
//! and resolves paths, and writes nothing.
//!
//! CLEAN CUT. compact is local-mode only, and a session ae cannot cleanly classify —
//! a managed mode, an unknown mode, no origin, an unresolvable origin, an unreadable
//! config, or NO VALID SESSION ID — is refused with a clear reason rather than
//! emulated or migrated on the fly. In particular a session with no parseable
//! `session_id` is unsupported old state: it is refused with a refresh/migrate
//! instruction, never minted a new id (the frozen bash minted one; the clean cut does
//! not).
//!
//! The tuple's ten `0x1f`-separated fields, in order: `name`, `uuid`, `uuid_origin`,
//! `mode`, `origin` (the recorded path, verified to be a directory — not
//! canonicalized), `config`, `purge` (`true`/`false`), `archive_path`, `main_ref`
//! (`alias:name`), `roster` (`main=<alias> workers=<a,b|->`).

use std::ffi::OsStr;
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::archive::{ConfigNode, classify_config_node};
use crate::config::{Workspace, read_workspace};
use crate::inventory::ServerId;
use crate::state::EXIT_FAILED;
use crate::transport;
use crate::{event_text, meta, requests};

/// `_compact-freeze` core entry. Emits the frozen tuple on `out` and returns `0`, or
/// writes a clear one-line refusal to `err` and returns [`EXIT_FAILED`]. Never
/// mutates anything.
#[allow(
    clippy::too_many_lines,
    reason = "a linear resolve-or-refuse sequence; each step's refusal reads best beside the check that raises it"
)]
pub(crate) fn freeze(
    dir: &Path,
    keep_history: bool,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_owned();

    let Ok(bytes) = meta::read_bytes(dir) else {
        writeln!(
            err,
            "compact: no ae session state for '{name}' — nothing to compact."
        )?;
        return Ok(EXIT_FAILED);
    };

    // Mode: local only. A managed or unclassifiable mode is refused, not emulated.
    let mode = meta_str(&bytes, "mode").unwrap_or_default();
    match mode.as_str() {
        "local" => {}
        "git" | "full" => {
            writeln!(
                err,
                "compact: '{name}' is {mode} mode; compact is local-mode only. A fresh {mode} workspace would be based on the canonical origin's HEAD, which normally lags the session's own branch — you would get a session missing the code it just archived. Use: ae end {name}, then start a new session yourself."
            )?;
            return Ok(EXIT_FAILED);
        }
        "" => {
            writeln!(
                err,
                "compact: session '{name}' records no mode — refusing to compact a session ae cannot classify."
            )?;
            return Ok(EXIT_FAILED);
        }
        other => {
            writeln!(
                err,
                "compact: session '{name}' records an unknown mode '{other}'."
            )?;
            return Ok(EXIT_FAILED);
        }
    }

    // Origin: recorded, and it must resolve to a directory. The RAW path is kept, not
    // a canonicalized one — the fresh session `cd`s into it and symlinks resolve then;
    // `metadata` (a tracked capability door) both proves it exists and follows a
    // symlink to its target the way the launch will. A harmless divergence from the
    // frozen bash, which normalized the path via `_canonical_dir`.
    let origin = meta_str(&bytes, "origin").unwrap_or_default();
    if origin.is_empty() {
        writeln!(
            err,
            "compact: session '{name}' records no origin — the fresh session would have nowhere to start."
        )?;
        return Ok(EXIT_FAILED);
    }
    if !dir_exists(Path::new(&origin)) {
        writeln!(
            err,
            "compact: session '{name}' records origin '{origin}', which does not resolve to a directory."
        )?;
        return Ok(EXIT_FAILED);
    }

    // Config: the recorded config layered UNDER the origin's local `.ae/config`, resolved
    // once here and again, identically, at each revalidation gate (see `resolve_workspace`).
    // `config` is emitted RAW in the tuple regardless.
    let config = meta_str(&bytes, "config").unwrap_or_default();
    let workspace = match resolve_workspace(&name, &config, &origin) {
        Ok(w) => w,
        Err(reason) => {
            writeln!(err, "{reason}")?;
            return Ok(EXIT_FAILED);
        }
    };

    // Session id: a valid recorded UUID. CLEAN CUT — no minting; a session with none
    // is unsupported old state, refused with a refresh/migrate instruction.
    let raw_uuid = meta_str(&bytes, "session_id").unwrap_or_default();
    let uuid = crate::archive::canonical_uuid(&raw_uuid);
    if uuid.is_empty() {
        writeln!(
            err,
            "compact: session '{name}' records no valid session id — refresh or migrate the session, then retry."
        )?;
        return Ok(EXIT_FAILED);
    }
    let uuid_origin = meta_str(&bytes, "session_id_origin").unwrap_or_else(|| "session".to_owned());

    // compact keeps the archive and the history by definition; a config that opts into
    // purge is a contradiction the human resolves explicitly with --keep-history.
    let purge = !keep_history && workspace.purge_agent_history;
    if purge {
        writeln!(
            err,
            "compact: session '{name}' has purge_agent_history enabled, which contradicts compact. To proceed: ae compact --keep-history {name}."
        )?;
        return Ok(EXIT_FAILED);
    }

    // The main agent to hand over from — its `alias:name` ref, taken from the TYPED
    // roster grammar (SC-405c), not string-sliced. A malformed `agent.main` (`cl`,
    // `cl:`, `:main`, empty) never becomes a roster entry, so it is refused HERE rather
    // than emitted as a broken handover ref that P3.7b would then try to deliver to.
    let meta_text = String::from_utf8_lossy(&bytes);
    let parsed = meta::Meta::parse(&meta_text);
    let Some(main_ref) = parsed
        .roster()
        .iter()
        .find(|entry| entry.slot == "main")
        .map(meta::RosterEntry::reference)
    else {
        writeln!(
            err,
            "compact: session '{name}' records no valid main agent (alias:name) to hand over from."
        )?;
        return Ok(EXIT_FAILED);
    };

    // The roster the fresh session is PROMISED to start. main is required.
    let Some(roster_main) = workspace.main.as_deref().filter(|m| !m.is_empty()) else {
        writeln!(
            err,
            "compact: the recorded config names no [workspace] main — the fresh session would have no agent."
        )?;
        return Ok(EXIT_FAILED);
    };
    let roster = roster_string(roster_main, workspace.workers.as_deref());

    let Some(root) = crate::state_root() else {
        writeln!(err, "compact: cannot resolve the ae state root.")?;
        return Ok(EXIT_FAILED);
    };
    let archive_path = root.join("archive").join(&uuid);

    let fields = [
        name,
        uuid,
        uuid_origin,
        mode,
        origin,
        config,
        // Always false: compact keeps the history, and a purge config was already
        // refused above (or overridden by --keep-history), so the boundary that
        // consumes this tuple never sees purge=true.
        "false".to_owned(),
        archive_path.to_string_lossy().into_owned(),
        main_ref,
        roster,
    ];
    // Framing guard: the tuple is ONE `0x1f`-separated line. A field carrying the
    // separator byte would forge extra fields, and a newline would split the record —
    // both silently corrupt what the boundary parses back (the TSV-framing hazard, one
    // separator up). Refuse rather than emit a tuple that does not round-trip.
    if let Some(bad) = fields
        .iter()
        .find(|f| f.contains('\u{1f}') || f.contains('\n'))
    {
        writeln!(
            err,
            "compact: a resolved value contains a control byte (U+001F or newline) that would corrupt the frozen tuple: {bad:?}"
        )?;
        return Ok(EXIT_FAILED);
    }
    writeln!(out, "{}", fields.join("\u{1f}"))?;
    Ok(0)
}

/// A meta value as an owned lossy string, or `None` when the key is absent.
fn meta_str(bytes: &[u8], key: &str) -> Option<String> {
    meta::first_value(bytes, key).map(|value| String::from_utf8_lossy(value).into_owned())
}

/// Whether `path` resolves (following symlinks) to a directory — the origin's
/// existence-and-kind gate, and the archive-dir presence check. `metadata` is a tracked
/// capability door.
fn dir_exists(path: &Path) -> bool {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: proves the recorded origin (a fresh-session cwd) or the archive dir exists and is a directory — see clippy.toml"
    )]
    let meta = std::fs::metadata(path);
    meta.is_ok_and(|m| m.is_dir())
}

/// Resolve the `[workspace]` values from the recorded config layered UNDER the origin's
/// local `.ae/config`. Every SELECTED path is CLASSIFIED (`classify_config_node`: stat +
/// one lstat, neither opening the node) BEFORE it is read, so a FIFO/device can never
/// reach `read_to_string` and block; only a confirmed regular file is read, and an
/// unreadable/non-UTF-8 one still refuses (the purge-bypass guard). Resolved once by
/// `freeze` and again, identically, by `revalidate`. `Err` is the ready-to-print refusal.
fn resolve_workspace(name: &str, config: &str, origin: &str) -> Result<Workspace, String> {
    // Global config, when recorded and not /dev/null, is REQUIRED to be a regular file:
    // absent, a non-regular node, or an unprovable existence all refuse (none are opened).
    let global_cfg = if config.is_empty() || config == "/dev/null" {
        None
    } else {
        match classify_config_node(Path::new(config)) {
            ConfigNode::Regular => Some(PathBuf::from(config)),
            ConfigNode::Absent | ConfigNode::Other => {
                return Err(format!(
                    "compact: session '{name}' records config '{config}', which is not a readable regular file (absent, a directory/FIFO/special node, or unreadable). The fresh session's roster comes from that config; compact will not guess it."
                ));
            }
        }
    };
    // The local overlay is OPTIONAL only when truly absent. A present non-regular node, or
    // an error that cannot prove absence (permission/I/O), refuses — never a silent
    // fallback to the global as if the local were not there.
    let local_cfg_path = Path::new(origin).join(".ae").join("config");
    let local_cfg = match classify_config_node(&local_cfg_path) {
        ConfigNode::Absent => None,
        ConfigNode::Regular => Some(local_cfg_path),
        ConfigNode::Other => {
            return Err(format!(
                "compact: session '{name}' has a local .ae/config that exists but is not a readable regular file (a directory/FIFO/special node, or unreadable); refusing rather than silently ignoring it."
            ));
        }
    };
    // Both are confirmed regular files, so the read cannot block; a decode or permission
    // error still refuses.
    read_workspace(global_cfg.as_deref(), local_cfg.as_deref()).map_err(|path| {
        format!(
            "compact: session '{name}' records config '{}', which cannot be read. The fresh session's roster comes from that config; compact will not guess it.",
            path.display()
        )
    })
}

/// The fields of the frozen tuple that the RUST destructive gates revalidate against —
/// identity (`uuid`), stability (`mode`/`origin`/`config`), and the sealed history policy
/// (`purge`). The other four the freezer emits — `uuid_origin`, `archive_path`, `main_ref`,
/// `roster` — are read by BASH for the handover ask, the recovery display, and the exec
/// plan; Rust re-derives the archive path from the state root rather than trusting a passed
/// one, so it does not keep them.
pub(crate) struct FrozenTuple {
    pub(crate) name: String,
    pub(crate) uuid: String,
    pub(crate) mode: String,
    pub(crate) origin: String,
    pub(crate) config: String,
    pub(crate) purge: bool,
    /// The roster the fresh session was PROMISED to start (`main=<alias> workers=<a,b|->`),
    /// as SHOWN at the prompt. Revalidate compares the config's CURRENT roster against it, so
    /// a config rewritten under the window that would start different agents is refused.
    pub(crate) roster: String,
}

impl FrozenTuple {
    /// Parse the tuple line. `None` unless there are EXACTLY ten fields — `freeze`'s
    /// framing guard proves no field carries the separator or a newline, so a genuine
    /// tuple round-trips and a malformed argument is refused, not misread. Only the seven
    /// fields Rust revalidates against are kept; the field count is still validated in full.
    pub(crate) fn parse(line: &str) -> Option<Self> {
        let line = line.strip_suffix('\n').unwrap_or(line);
        let fields: Vec<&str> = line.split('\u{1f}').collect();
        let [
            name,
            uuid,
            _uuid_origin,
            mode,
            origin,
            config,
            purge,
            _archive_path,
            _main_ref,
            roster,
        ] = fields.as_slice()
        else {
            return None;
        };
        Some(Self {
            name: (*name).to_owned(),
            uuid: (*uuid).to_owned(),
            mode: (*mode).to_owned(),
            origin: (*origin).to_owned(),
            config: (*config).to_owned(),
            purge: *purge == "true",
            roster: (*roster).to_owned(),
        })
    }
}

/// The result of asking a session's RECORDED tmux server whether it is still there.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StopState {
    /// The recorded server ANSWERED and the session is not among its sessions.
    Stopped,
    /// The recorded server answered and the session is STILL present.
    Alive,
    /// The server could not be asked, or the record does not name one server unambiguously
    /// (a Missing/Ambiguous selector, or an enumeration error). Existence is UNPROVEN —
    /// and unproven is NEVER equated with stopped.
    Unknown,
}

/// Ask the session's OWN recorded tmux server (the SC-405l selector) whether `name` is
/// still live. Crossing the destructive boundary requires [`StopState::Stopped`] — a
/// definitive absence FROM A SERVER THAT ANSWERED. `Alive` and `Unknown` both refuse: an
/// unreachable server, or a Missing/Ambiguous selector, is never read as stopped (the
/// same fail-closed stance `end` takes).
pub(crate) fn verify_stopped(meta_bytes: &[u8], name: &str) -> StopState {
    let selector = meta::Meta::parse(&String::from_utf8_lossy(meta_bytes)).server_selector();
    let meta::ServerSelector::Positive(sel) = selector else {
        // Missing / Ambiguous (or any other non-positive shape): fail closed, as `end`
        // does — an unidentified server is never proven empty.
        return StopState::Unknown;
    };
    // The RECORDED server's stop verdict, including the clean-dead case the generic
    // enumeration cannot see: killing the last session exits the server, and its
    // stale-socket diagnostic PROVES the session gone (frozen `_end_verify_gone`).
    match transport::verify_session_absent(&ServerId::Selected(sel), name) {
        crate::tmux::StopProbe::Present => StopState::Alive,
        crate::tmux::StopProbe::Absent => StopState::Stopped,
        crate::tmux::StopProbe::Unknown => StopState::Unknown,
    }
}

/// Compare the LIVE session to the frozen authorization. `Ok(())` if it is still exactly
/// the session `freeze` authorized; `Err(reason)` (ready to print) if it was replaced or
/// materially changed since — in which case NOTHING is stopped or archived. `when` names
/// the gate. Mirrors the frozen `_compact_revalidate` semantics: the id is the replacement
/// guard, mode/origin/config anchor identity, a purge flip is a different operation, and a
/// surviving spawned agent blocks a roster compact would otherwise silently drop.
/// The roster string the fresh session is PROMISED to start — `main=<alias> workers=<a,b|->`.
/// Derived identically where the prompt SHOWS it (`freeze`) and where a config rewritten
/// under the confirm window would now RESOLVE it (`revalidate`), so the "what was authorized"
/// and "what would start now" strings cannot silently diverge in their formatting.
fn roster_string(main: &str, workers: Option<&str>) -> String {
    let workers = workers.filter(|w| !w.is_empty()).unwrap_or("-");
    format!("main={main} workers={workers}")
}

fn revalidate(
    dir: &Path,
    frozen: &FrozenTuple,
    keep_history: bool,
    when: &str,
) -> Result<(), String> {
    // Bind the authorization's name to the operand itself. `freeze` derived `name` from the
    // session directory's direct-child basename, and the stop query below (`verify_stopped`)
    // trusts it to name the LIVE session. A tuple whose name field was altered to some other
    // (absent) session would otherwise prove THAT name stopped while the live session runs
    // on — and teardown would then delete the live session. The basename is authoritative:
    // meta records only session_id, which the UUID guard already binds.
    let basename = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    if frozen.name != basename {
        return Err(format!(
            "compact: the authorization names '{}' but the session directory is '{basename}' ({when}); refusing — the frozen tuple does not point at this session.",
            frozen.name
        ));
    }
    let Ok(bytes) = meta::read_bytes(dir) else {
        return Err(format!(
            "compact: session '{}' no longer has ae state ({when}).",
            frozen.name
        ));
    };
    let now_uuid =
        crate::archive::canonical_uuid(&meta_str(&bytes, "session_id").unwrap_or_default());
    if now_uuid != frozen.uuid {
        let shown = if now_uuid.is_empty() {
            "<none>"
        } else {
            &now_uuid
        };
        return Err(format!(
            "compact: '{}' is not the session that was authorized ({when}) — authorized id {}, on disk now {shown}. Nothing was stopped and nothing was archived.",
            frozen.name, frozen.uuid
        ));
    }
    let now_mode = meta_str(&bytes, "mode").unwrap_or_default();
    if now_mode != frozen.mode {
        return Err(format!(
            "compact: '{}' changed mode from {} to {now_mode} ({when}); refusing.",
            frozen.name, frozen.mode
        ));
    }
    let now_origin = meta_str(&bytes, "origin").unwrap_or_default();
    if now_origin != frozen.origin {
        return Err(format!(
            "compact: '{}' changed origin from {} to {now_origin} ({when}); refusing.",
            frozen.name, frozen.origin
        ));
    }
    let now_config = meta_str(&bytes, "config").unwrap_or_default();
    if now_config != frozen.config {
        return Err(format!(
            "compact: '{}' changed its recorded config from '{}' to '{now_config}' ({when}); refusing.",
            frozen.name, frozen.config
        ));
    }
    // History policy re-read: a flip to purge is a DIFFERENT operation than authorized.
    let workspace = resolve_workspace(&frozen.name, &now_config, &now_origin)?;
    if !frozen.purge && workspace.purge_agent_history && !keep_history {
        return Err(format!(
            "compact: '{}' now has purge_agent_history enabled ({when}), which contradicts the authorized compact. Re-run with --keep-history if that is what you want.",
            frozen.name
        ));
    }
    // The roster is an AUTHORIZED FACT, not merely a resolvable one: it was PRINTED at the
    // prompt as what the child will start, so a config rewritten during the window that now
    // resolves to a DIFFERENT roster must refuse — proving merely that SOME roster resolves
    // would let a child start agents nobody approved. Edits made BEFORE compact are adopted;
    // only a change DURING the window is caught here. Mirrors the frozen `_compact_revalidate`
    // roster gate, and reads the current roster from the same config `resolve_workspace` did.
    let now_main = workspace.main.as_deref().unwrap_or_default();
    let now_roster = roster_string(now_main, workspace.workers.as_deref());
    if now_roster != frozen.roster {
        return Err(format!(
            "compact: '{}' would now start a different roster than the one shown ({when}); refusing. Shown: {}. Now: {now_roster}. Nothing was stopped and nothing was archived.",
            frozen.name, frozen.roster
        ));
    }
    // Spawn closure: compact never retires someone else's worker, so a spawned agent must
    // be gone before the fresh roster (main + workers) replaces the session.
    let parsed = meta::Meta::parse(&String::from_utf8_lossy(&bytes));
    let spawned: Vec<String> = parsed
        .roster()
        .iter()
        .filter(|entry| entry.slot.starts_with("spawned."))
        .map(meta::RosterEntry::reference)
        .collect();
    if !spawned.is_empty() {
        return Err(format!(
            "compact: '{}' still has spawned agents ({when}): {}. compact never retires someone else's worker — retire them, then re-run.",
            frozen.name,
            spawned.join(", ")
        ));
    }
    Ok(())
}

/// `_compact-revalidate` core entry — the authorization gate bash crosses at BOTH lifecycle
/// points: after confirmation (before anything is messaged) and after the handover wait
/// (before anything is stopped). `when` is the driver's label for which crossing this is, so
/// a refusal names the gate that caught the drift — the two are not interchangeable, and a
/// test proves an identity change during the unlocked wait is caught by the SECOND, not the
/// first. `0` if it is still the authorized session; a named refusal on `err` and
/// [`EXIT_FAILED`] otherwise. Read-only.
pub(crate) fn revalidate_step(
    dir: &Path,
    tuple: &str,
    keep_history: bool,
    when: &str,
    err: &mut impl Write,
) -> io::Result<u8> {
    let Some(frozen) = FrozenTuple::parse(tuple) else {
        writeln!(
            err,
            "compact: internal error — the frozen tuple did not parse (expected ten fields)."
        )?;
        return Ok(EXIT_FAILED);
    };
    match revalidate(dir, &frozen, keep_history, when) {
        Ok(()) => Ok(0),
        Err(reason) => {
            writeln!(err, "{reason}")?;
            Ok(EXIT_FAILED)
        }
    }
}

/// The two gates every destructive stage crosses: the session is still the one authorized
/// (`revalidate`), AND it is PROVEN stopped on its recorded server (`verify_stopped`).
/// `Ok(Ok(()))` proceeds; `Ok(Err(code))` is a refusal already written to `err`.
fn gate_revalidate_and_stopped(
    dir: &Path,
    frozen: &FrozenTuple,
    keep_history: bool,
    when: &str,
    err: &mut impl Write,
) -> io::Result<Result<(), u8>> {
    if let Err(reason) = revalidate(dir, frozen, keep_history, when) {
        writeln!(err, "{reason}")?;
        return Ok(Err(EXIT_FAILED));
    }
    let Ok(bytes) = meta::read_bytes(dir) else {
        writeln!(
            err,
            "compact: session '{}' state vanished at the stop check ({when}).",
            frozen.name
        )?;
        return Ok(Err(EXIT_FAILED));
    };
    match verify_stopped(&bytes, &frozen.name) {
        StopState::Stopped => Ok(Ok(())),
        StopState::Alive => {
            writeln!(
                err,
                "compact: '{}' is still running on its recorded tmux server ({when}) — refusing to cross the destructive boundary on a live session.",
                frozen.name
            )?;
            Ok(Err(EXIT_FAILED))
        }
        StopState::Unknown => {
            writeln!(
                err,
                "compact: could not PROVE '{}' is stopped on its recorded tmux server ({when}) — the server did not answer, or none is recorded. Refusing rather than act on a session that may still be live.",
                frozen.name
            )?;
            Ok(Err(EXIT_FAILED))
        }
    }
}

/// `_compact-archive` core entry — the FIRST stage of the two-stage destructive protocol.
/// Revalidates, PROVES the session stopped, makes the archive durable (publishing when
/// absent, or REUSING an equivalent existing one — never clobbering, never publishing over
/// drift), preflights that the recovery command will restore, and PRINTS that recovery
/// command on `out` before returning. It tears NOTHING down — `teardown_step` does, only
/// after bash has seen this stage's recovery line (the recovery point is visible before
/// anything is destroyed).
#[allow(
    clippy::too_many_arguments,
    reason = "the archive volatiles bash supplies at the boundary — git push facts, preserved/workdir, and the UTC instant std cannot format — are each a distinct field"
)]
pub(crate) fn archive_step(
    dir: &Path,
    tuple: &str,
    keep_history: bool,
    archived_at: &str,
    push_outcome: &str,
    push_ref: &str,
    preserved: &str,
    workdir: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let Some(frozen) = FrozenTuple::parse(tuple) else {
        writeln!(
            err,
            "compact: internal error — the frozen tuple did not parse (expected ten fields)."
        )?;
        return Ok(EXIT_FAILED);
    };
    if let Err(code) =
        gate_revalidate_and_stopped(dir, &frozen, keep_history, "before archive", err)?
    {
        return Ok(code);
    }
    let Some(root) = crate::state_root() else {
        writeln!(err, "compact: cannot resolve the ae state root.")?;
        return Ok(EXIT_FAILED);
    };
    let archive_root = root.join("archive");
    let archive_path = archive_root.join(&frozen.uuid);
    // The C3 gate: an existing archive is REUSED only if the live payload still matches it;
    // drift refuses (retain both); publish is NEVER called over an existing archive (it is
    // immutable and would refuse).
    if dir_exists(&archive_path) {
        match crate::archive::publish::live_matches_existing_archive(
            dir,
            &archive_path,
            &frozen.uuid,
        ) {
            Ok(true) => {
                writeln!(
                    err,
                    "compact: an archive for this session already exists and still matches the live state — reusing it as the recovery point."
                )?;
            }
            Ok(false) => {
                writeln!(
                    err,
                    "compact: '{}' has an existing archive that no longer matches the live session (it drifted after a previous attempt). Refusing teardown — that would lose the drift. Both the live session and the archive are retained.",
                    frozen.name
                )?;
                return Ok(EXIT_FAILED);
            }
            Err(reason) => {
                writeln!(err, "{reason}")?;
                return Ok(EXIT_FAILED);
            }
        }
    } else {
        let ops = crate::archive::publish::Ops {
            push_outcome,
            push_ref,
            preserved,
            workdir,
            archived_at,
        };
        // publish's own `target\tfiles\tbytes` line is diagnostics for compact; `out` is
        // kept for the recovery contract alone.
        let mut sink = Vec::new();
        let code = crate::archive::publish::run(dir, &ops, &mut sink, err)?;
        if code != 0 {
            return Ok(code);
        }
    }
    // Preflight the recovery command itself: `--from` must accept this archive. Read-only
    // (proven: from::run only writes to out/err); its stdout is discarded here.
    let mut sink = Vec::new();
    let code = crate::archive::from::run(&archive_root, &frozen.uuid, &mut sink, err)?;
    if code != 0 {
        writeln!(
            err,
            "compact: the archive was published but is not inheritable — refusing to tear down against a recovery point that would not restore."
        )?;
        return Ok(code);
    }
    // The recovery point, now durable and proven — printed BEFORE any teardown runs.
    writeln!(out, "recover: ae {} --from {}", frozen.name, frozen.uuid)?;
    Ok(0)
}

/// Re-validate the durable archive as an inheritable recovery point, immediately before
/// teardown. Runs the SAME read-only preflight the archive step ran ([`from::run`](crate::archive::from::run)),
/// discarding its stdout; its diagnostics go to `err`. `Ok(true)` only when it accepts the
/// archive as a real, non-symlink, unclaimed, well-formed tree whose id matches — an empty
/// directory, a symlink, a claimed/replaced directory, or a vanished archive all yield
/// `Ok(false)`. `metadata`-level existence is deliberately NOT trusted: teardown is its own
/// invocation, so it must re-prove the recovery point rather than trust the archive step's.
fn archive_recovery_point_valid(
    archive_root: &Path,
    uuid: &str,
    err: &mut impl Write,
) -> io::Result<bool> {
    let mut sink = Vec::new();
    Ok(crate::archive::from::run(archive_root, uuid, &mut sink, err)? == 0)
}

/// `_compact-teardown` core entry — the SECOND stage. Re-proves the authorization and the
/// stop, RE-VALIDATES the durable archive as an inheritable recovery point (teardown without
/// one is forbidden), removes the live session, and prints the exec plan bash relaunches from.
pub(crate) fn teardown_step(
    dir: &Path,
    tuple: &str,
    keep_history: bool,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let Some(frozen) = FrozenTuple::parse(tuple) else {
        writeln!(
            err,
            "compact: internal error — the frozen tuple did not parse (expected ten fields)."
        )?;
        return Ok(EXIT_FAILED);
    };
    if let Err(code) =
        gate_revalidate_and_stopped(dir, &frozen, keep_history, "before teardown", err)?
    {
        return Ok(code);
    }
    let Some(root) = crate::state_root() else {
        writeln!(err, "compact: cannot resolve the ae state root.")?;
        return Ok(EXIT_FAILED);
    };
    let archive_root = root.join("archive");
    // C2's separate-stage boundary: teardown is its OWN invocation, so it must not trust
    // that the archive the archive step proved is still the same tree now. Re-run the same
    // read-only preflight immediately before destroying the live session — the live session
    // is its ONLY recovery source, and a symlinked, emptied, claimed, or vanished archive
    // must retain it, never delete it against a recovery point that would not restore.
    if !archive_recovery_point_valid(&archive_root, &frozen.uuid, err)? {
        writeln!(
            err,
            "compact: refusing teardown — no durable, inheritable archive at {} to recover from. The live session is retained.",
            archive_root.join(&frozen.uuid).display()
        )?;
        return Ok(EXIT_FAILED);
    }
    let code = crate::teardown::run(dir, out, err)?;
    if code != 0 {
        return Ok(code);
    }
    // The exec plan: bash relaunches `ae <name> --from <uuid>` (compact is local-only, so
    // the fresh session is the default local mode).
    writeln!(out, "{}\u{1f}{}", frozen.name, frozen.uuid)?;
    Ok(0)
}

// ── The semantic handover: wait for BOTH facts, or withdraw (slice B) ────────────
//
// The handover is delivered through the session's own `ask` (bash, pane-side) as a tracked
// request from the reserved compact actor `ae:compact:<uuid>`, carrying a memo BASELINE in
// its body. These two cores own the WAIT and the WITHDRAWAL — the stateful half the clean
// cut keeps in Rust — and read the ledger and memo the frozen `_compact_wait_handover` /
// `_compact_cancel_outstanding` read. Bash branches on the exit code alone.

/// The poll interval between reads while waiting. The frozen `_compact_wait_handover`
/// polled every two seconds; the bounded deadline is never overshot.
const HANDOVER_POLL: Duration = Duration::from_secs(2);

/// The default handover bound when bash passes no `--timeout` — the frozen
/// `_compact_handover_secs` fallback (its `AE_COMPACT_HANDOVER_SECS` override lives in the
/// bash driver, which passes the resolved value through).
pub(crate) const DEFAULT_HANDOVER_SECS: u64 = 300;

/// One flat member of an event line equals `want` — present AND equal, distinguishing an
/// absent key (which never matches) from one present and equal.
fn field_eq(line: &[u8], key: &str, want: &[u8]) -> bool {
    event_text::member(line, key).value() == Some(want)
}

/// The pre-delivery memo boundary the frozen `_compact_request_text` embeds in the request
/// body (`AE-COMPACT-MEMO-BASELINE=<n>`). A handover is real only if a memo row was
/// appended AFTER this byte offset, so a memo rewritten wholesale beforehand cannot
/// masquerade as new content.
///
/// `Some(n)` ONLY when the marker is present AND its value parses — `Some(0)` is legitimate
/// (an empty memo at request time, so any row is new). A marker that is ABSENT or MALFORMED
/// returns `None`: UNAVAILABLE EVIDENCE, which the wait refuses. It must never fall back to
/// `0`, because a `0` baseline lets a memo written BEFORE this request satisfy the "new
/// handover memo" fact, collapsing the two-fact safety gate to one (p4runner BLOCKER,
/// review dc59b178).
fn baseline_from_body(body: &[u8]) -> Option<usize> {
    for line in body.split(|&b| b == b'\n') {
        if let Some(rest) = line.strip_prefix(b"AE-COMPACT-MEMO-BASELINE=") {
            // The marker is authoritative once found: a malformed value is a corrupt
            // request, NOT a reason to fall back to 0. Return None so the wait fails closed.
            return std::str::from_utf8(rest)
                .ok()
                .and_then(|s| s.trim().parse::<usize>().ok());
        }
    }
    None
}

/// The stored request body IF it is a readable regular file, else `None`. A deleted, FIFO,
/// directory, or otherwise non-regular `body_file` is UNAVAILABLE EVIDENCE — the wait must
/// refuse rather than read it as empty (which would yield baseline `0`, the same one-fact
/// collapse the marker guard closes). Follows symlinks, matching the frozen `[[ -f ]]` gate;
/// a TOCTOU removal between the check and the read yields empty bytes, hence `None` baseline,
/// hence the same closed refusal.
fn read_body_regular(path_bytes: &[u8]) -> Option<Vec<u8>> {
    let path = Path::new(OsStr::from_bytes(path_bytes));
    if !crate::archive::regular_file(path) {
        return None;
    }
    Some(event_text::read_container(path))
}

/// A `reply` for the EXACT ref, from the `main` slot of the FROZEN session — the frozen
/// `_compact_reply_seen`. Not the request sensor's "is it closed" row: this answers
/// "closed by exactly the main agent the handover addressed".
fn reply_seen(events: &[u8], reference: &str, session: &str) -> bool {
    event_text::read_lines(events).into_iter().any(|line| {
        event_text::event_line(line).is_some_and(|line| {
            field_eq(line, "action", b"reply")
                && field_eq(line, "ref", reference.as_bytes())
                && field_eq(line, "actor_slot", b"main")
                && field_eq(line, "actor_session", session.as_bytes())
        })
    })
}

/// At least one VALID handover row appended after `baseline` — the frozen
/// `_compact_memo_after`. Rows are read from the byte offset onward, so a wholesale rewrite
/// cannot masquerade; a row counts only with a non-empty ts and author, the `handover`
/// topic, and non-empty text.
fn memo_after(memo: &[u8], baseline: usize) -> bool {
    let tail = memo.get(baseline..).unwrap_or(&[]);
    tail.split(|&b| b == b'\n').any(|row| {
        if row.is_empty() {
            return false;
        }
        let mut cols = row.splitn(4, |&b| b == b'\t');
        let ts = cols.next().unwrap_or_default();
        let author = cols.next().unwrap_or_default();
        let topic = cols.next().unwrap_or_default();
        let text = cols.next().unwrap_or_default();
        !ts.is_empty() && !author.is_empty() && topic == b"handover" && !text.is_empty()
    })
}

/// The request `reference` names, from the live ledger, or `None` if the ledger has no such
/// request.
fn request_by_ref(dir: &Path, reference: &str) -> Option<requests::Request> {
    let container = event_text::read_container(&dir.join("events.jsonl"));
    requests::states(&container)
        .into_iter()
        .find(|r| r.id == reference.as_bytes())
}

/// `_compact-wait <session-dir> <ref> [--timeout <secs>]` core — the bounded wait for the
/// TWO handover facts. `0` only when a `reply` (exact ref, main slot, frozen session) AND a
/// new `handover` memo row (after the request's own baseline) have BOTH arrived. On timeout
/// it returns [`EXIT_FAILED`] and LEAVES the request as it found it (ruling A, 2026-08-29):
/// a normal timeout is a resumable pause, and only `--digest-only` withdraws. The guidance
/// naming WHICH fact arrived is written here — bash branches on the exit code alone.
pub(crate) fn wait_step(
    dir: &Path,
    reference: &str,
    timeout_secs: u64,
    err: &mut impl Write,
) -> io::Result<u8> {
    let name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_owned();
    let Some(req) = request_by_ref(dir, reference) else {
        writeln!(
            err,
            "compact: cannot wait on {reference} — the ledger has no such request. Nothing was stopped and nothing was archived."
        )?;
        return Ok(EXIT_FAILED);
    };
    // The memo baseline is REQUIRED evidence: without it the "new handover memo" fact cannot
    // be told from a memo that predates this request. A missing/non-regular body, or a body
    // with no valid AE-COMPACT-MEMO-BASELINE marker, fails the wait CLOSED — no stop, no
    // archive, no teardown (p4runner BLOCKER, review dc59b178).
    let Some(baseline) = read_body_regular(&req.body_file).and_then(|b| baseline_from_body(&b))
    else {
        writeln!(
            err,
            "compact: cannot establish the handover memo baseline for {reference} — its request body is missing, not a regular file, or carries no valid AE-COMPACT-MEMO-BASELINE marker. Refusing the wait; nothing was stopped and nothing was archived."
        )?;
        return Ok(EXIT_FAILED);
    };
    let events_path = dir.join("events.jsonl");
    let memo_path = dir.join("memo.tsv");

    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut saw_reply;
    let mut saw_memo;
    loop {
        saw_reply = reply_seen(&event_text::read_container(&events_path), reference, &name);
        saw_memo = memo_after(&event_text::read_container(&memo_path), baseline);
        if saw_reply && saw_memo {
            writeln!(
                err,
                "compact: handover complete (reply {reference} and a new handover memo)."
            )?;
            return Ok(0);
        }
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        std::thread::sleep(HANDOVER_POLL.min(deadline - now));
    }

    writeln!(
        err,
        "compact: no complete handover within {timeout_secs}s — a reply AND a new handover memo are both required."
    )?;
    if saw_reply {
        // The reply CLOSED the request, so there is nothing left to keep waiting on and a
        // re-run asks again from scratch.
        writeln!(
            err,
            "  The reply arrived; the handover memo did not. {reference} is answered and closed, so a re-run sends a FRESH request. Ask your main agent to write the memo first (memo add --topic handover ...), or run ae compact --digest-only {name} to archive the digest as it stands."
        )?;
    } else if saw_memo {
        writeln!(
            err,
            "  The handover memo was written; the reply did not arrive. The request stays open: re-run ae compact {name} to keep waiting on it, or ae compact --digest-only {name} to archive the digest as it stands."
        )?;
    } else {
        writeln!(
            err,
            "  The request stays open: re-run ae compact {name} to keep waiting on it, or ae compact --digest-only {name} to archive the digest as it stands."
        )?;
    }
    Ok(EXIT_FAILED)
}

/// `_compact-cancel <session-dir> <ref>` core — withdraw an outstanding compact handover
/// request, the `--digest-only` degradation. Emits a Rust-owned, COMPLETELY SLOTLESS
/// `cancel` event whose actor is the request's own opener (`ae:compact:<uuid>`), which is
/// the exact shape [`requests::states`] reads as a withdrawal (the `withdrawn_by` display
/// arm: all four routing keys absent, actor bytes equal to the opening actor). Idempotent:
/// a request already closed is a no-op success. Refuses to withdraw anything not opened by
/// compact.
pub(crate) fn cancel_step(dir: &Path, reference: &str, err: &mut impl Write) -> io::Result<u8> {
    let Some(req) = request_by_ref(dir, reference) else {
        writeln!(
            err,
            "compact: cannot withdraw {reference} — the ledger has no such request."
        )?;
        return Ok(EXIT_FAILED);
    };
    let actor = String::from_utf8_lossy(&req.from).into_owned();
    if !actor.starts_with("ae:compact:") {
        writeln!(
            err,
            "compact: refusing to withdraw {reference} — it was not opened by compact (opener '{actor}')."
        )?;
        return Ok(EXIT_FAILED);
    }
    if req.status != requests::Status::Pending {
        // Already replied or already withdrawn — nothing to take back.
        return Ok(0);
    }
    let line = crate::state::event_line(
        crate::time::Timestamp::now(),
        &actor,
        "cancel",
        reference,
        "withdrawn: --digest-only, semantic handover skipped",
    );
    crate::state::emit(dir, &line)?;
    Ok(0)
}

/// The reserved compact actor for the session at `dir` — `ae:compact:<session-uuid>`, the
/// sender the handover is delivered as and the opener a withdrawal must match. Empty when
/// meta records no valid session id (then nothing is an outstanding compact request).
fn compact_actor(dir: &Path) -> String {
    let uuid = meta::read_bytes(dir)
        .ok()
        .and_then(|b| meta_str(&b, "session_id"))
        .map(|raw| crate::archive::canonical_uuid(&raw))
        .unwrap_or_default();
    if uuid.is_empty() {
        String::new()
    } else {
        format!("ae:compact:{uuid}")
    }
}

/// `_compact-memo-baseline <dir>` core — print the current `memo.tsv` byte size, the
/// pre-delivery boundary the bash driver embeds in the handover request body (the frozen
/// `_compact_memo_offset`). `_compact-wait` counts a handover only for a memo row appended
/// PAST this offset, which is why it is captured before delivery. No newline — bash
/// captures it with `$(…)`.
pub(crate) fn memo_baseline_step(dir: &Path, out: &mut impl Write) -> io::Result<u8> {
    let memo = event_text::read_container(&dir.join("memo.tsv"));
    write!(out, "{}", memo.len())?;
    Ok(0)
}

/// `_compact-find-outstanding <dir>` core — print the ref of the FIRST still-pending
/// handover request opened by this session's compact actor, or nothing. The frozen
/// `_compact_find_outstanding`: a retry reuses this ref (and, through its stored body, its
/// baseline) instead of delivering a duplicate. No newline — bash captures it with `$(…)`
/// and tests it with `-n`.
pub(crate) fn find_outstanding_step(dir: &Path, out: &mut impl Write) -> io::Result<u8> {
    let actor = compact_actor(dir);
    if actor.is_empty() {
        return Ok(0);
    }
    let container = event_text::read_container(&dir.join("events.jsonl"));
    if let Some(req) = requests::states(&container)
        .into_iter()
        .find(|r| r.status == requests::Status::Pending && r.from == actor.as_bytes())
    {
        out.write_all(&req.id)?;
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp session dir under a scratch root that doubles as `AE_HOME`.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("ae-compact-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch");
            Self(dir)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const UUID: &str = "11111111-1111-1111-1111-111111111111";

    /// Build a session dir with `meta` lines and a `[workspace]` config, returning the
    /// session dir. `origin` is the scratch root (a real, canonicalizable dir).
    fn session(s: &Scratch, extra_meta: &str, config_body: Option<&str>) -> PathBuf {
        let dir = s.0.join("sess");
        std::fs::create_dir_all(&dir).unwrap();
        let config_path = s.0.join("config");
        let config_line = if let Some(body) = config_body {
            std::fs::write(&config_path, body).unwrap();
            format!("config={}\n", config_path.display())
        } else {
            String::new()
        };
        let meta = format!(
            "session_id={UUID}\nmode=local\norigin={}\nagent.main=cl:main:{UUID}\n{config_line}{extra_meta}",
            s.0.display()
        );
        std::fs::write(dir.join("meta"), meta).unwrap();
        dir
    }

    fn run(dir: &Path, keep_history: bool) -> (u8, String, String) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = freeze(dir, keep_history, &mut out, &mut err).expect("freeze ran");
        (
            code,
            String::from_utf8_lossy(&out).into_owned(),
            String::from_utf8_lossy(&err).into_owned(),
        )
    }

    #[test]
    fn a_local_session_emits_the_ten_field_tuple() {
        let s = Scratch::new("ok");
        let dir = session(&s, "", Some("[workspace]\nmain = cl\nworkers = a, b\n"));
        let (code, out, err) = run(&dir, false);
        assert_eq!(code, 0, "{err}");
        let fields: Vec<&str> = out.trim_end().split('\u{1f}').collect();
        assert_eq!(fields.len(), 10, "ten fields: {out:?}");
        assert_eq!(fields[0], "sess");
        assert_eq!(fields[1], UUID);
        assert_eq!(fields[3], "local");
        assert_eq!(fields[6], "false", "purge false without a purge config");
        assert_eq!(fields[8], "cl:main");
        assert_eq!(fields[9], "main=cl workers=a, b");
    }

    #[test]
    fn a_managed_mode_is_refused_local_only() {
        let s = Scratch::new("git");
        let dir = session(&s, "", Some("[workspace]\nmain = cl\n"));
        std::fs::write(
            dir.join("meta"),
            format!(
                "session_id={UUID}\nmode=git\norigin={}\nagent.main=cl:main\n",
                s.0.display()
            ),
        )
        .unwrap();
        let (code, _out, err) = run(&dir, false);
        assert_eq!(code, 1);
        assert!(err.contains("local-mode only"), "{err}");
    }

    #[test]
    fn no_valid_session_id_is_refused_with_refresh_migrate() {
        let s = Scratch::new("nouuid");
        let dir = session(&s, "", Some("[workspace]\nmain = cl\n"));
        std::fs::write(
            dir.join("meta"),
            format!(
                "session_id=not-a-uuid\nmode=local\norigin={}\nagent.main=cl:main\n",
                s.0.display()
            ),
        )
        .unwrap();
        let (code, _out, err) = run(&dir, false);
        assert_eq!(code, 1);
        assert!(
            err.contains("no valid session id") && err.contains("refresh"),
            "{err}"
        );
    }

    #[test]
    fn a_missing_origin_is_refused() {
        let s = Scratch::new("noorigin");
        let dir = s.0.join("sess");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("meta"),
            format!("session_id={UUID}\nmode=local\nagent.main=cl:main\n"),
        )
        .unwrap();
        let (code, _out, err) = run(&dir, false);
        assert_eq!(code, 1);
        assert!(err.contains("records no origin"), "{err}");
    }

    #[test]
    fn an_unresolvable_origin_is_refused() {
        let s = Scratch::new("badorigin");
        let dir = s.0.join("sess");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("meta"),
            format!(
                "session_id={UUID}\nmode=local\norigin=/no/such/place/xyz\nagent.main=cl:main\n"
            ),
        )
        .unwrap();
        let (code, _out, err) = run(&dir, false);
        assert_eq!(code, 1);
        assert!(err.contains("does not resolve"), "{err}");
    }

    #[test]
    fn no_main_agent_is_refused() {
        let s = Scratch::new("nomain");
        let dir = s.0.join("sess");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("meta"),
            format!("session_id={UUID}\nmode=local\norigin={}\n", s.0.display()),
        )
        .unwrap();
        let (code, _out, err) = run(&dir, false);
        assert_eq!(code, 1);
        assert!(err.contains("no valid main agent"), "{err}");
    }

    #[test]
    fn a_config_naming_no_workspace_main_is_refused() {
        let s = Scratch::new("nowsmain");
        let dir = session(&s, "", Some("[agents]\ncl = claude\n"));
        let (code, _out, err) = run(&dir, false);
        assert_eq!(code, 1);
        assert!(err.contains("names no [workspace] main"), "{err}");
    }

    #[test]
    fn a_purge_config_is_refused_unless_keep_history() {
        let s = Scratch::new("purge");
        let dir = session(
            &s,
            "",
            Some("[workspace]\nmain = cl\npurge_agent_history = true\n"),
        );
        let (code, _out, err) = run(&dir, false);
        assert_eq!(code, 1);
        assert!(
            err.contains("purge_agent_history") && err.contains("--keep-history"),
            "{err}"
        );
        // keep_history overrides the contradiction and succeeds.
        let (code2, out2, err2) = run(&dir, true);
        assert_eq!(code2, 0, "{err2}");
        assert_eq!(out2.trim_end().split('\u{1f}').nth(6), Some("false"));
    }

    #[test]
    fn a_present_but_undecodable_config_is_refused() {
        // The purge-bypass regression: a config that is a REAL regular file (so the old
        // is_file gate passed) but whose bytes cannot be decoded must refuse — not read
        // as empty and let a purge=true setting slip through. Non-UTF-8 stands in for
        // "present but unreadable".
        let s = Scratch::new("badcfg");
        let dir = s.0.join("sess");
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = s.0.join("bad-config");
        std::fs::write(&cfg, [0xff, 0xfe, 0x00, 0x9c]).unwrap();
        std::fs::write(
            dir.join("meta"),
            format!(
                "session_id={UUID}\nmode=local\norigin={}\nagent.main=cl:main\nconfig={}\n",
                s.0.display(),
                cfg.display()
            ),
        )
        .unwrap();
        let (code, _out, err) = run(&dir, false);
        assert_eq!(code, 1);
        assert!(err.contains("cannot be read"), "{err}");
    }

    #[test]
    fn a_field_carrying_the_separator_byte_is_refused() {
        // A resolved value with a 0x1f would forge extra tuple fields; the framing guard
        // refuses rather than emit a tuple that does not round-trip. Here the config's
        // workers value smuggles one in.
        let s = Scratch::new("framing");
        let dir = session(&s, "", Some("[workspace]\nmain = cl\nworkers = a\u{1f}b\n"));
        let (code, out, err) = run(&dir, false);
        assert_eq!(code, 1, "{err}");
        assert!(
            err.contains("U+001F") || err.contains("control byte"),
            "{err}"
        );
        assert!(out.is_empty(), "no tuple emitted on refusal: {out:?}");
    }

    #[test]
    fn a_malformed_agent_main_is_refused_not_emitted() {
        // `cl`, `cl:`, `:main` each fail the typed roster grammar (SC-405c) and so never
        // become a `main` entry — freeze refuses rather than emitting a broken handover
        // ref P3.7b would try to deliver to. Everything else in the session is valid.
        for bad in ["cl", "cl:", ":main"] {
            let s = Scratch::new("malformedmain");
            let dir = s.0.join("sess");
            std::fs::create_dir_all(&dir).unwrap();
            let cfg = s.0.join("config");
            std::fs::write(&cfg, "[workspace]\nmain = cl\n").unwrap();
            std::fs::write(
                dir.join("meta"),
                format!(
                    "session_id={UUID}\nmode=local\norigin={}\nagent.main={bad}\nconfig={}\n",
                    s.0.display(),
                    cfg.display()
                ),
            )
            .unwrap();
            let (code, out, err) = run(&dir, false);
            assert_eq!(code, 1, "agent.main={bad:?}: {err}");
            assert!(
                err.contains("no valid main agent"),
                "agent.main={bad:?}: {err}"
            );
            assert!(
                out.is_empty(),
                "agent.main={bad:?} emitted a tuple: {out:?}"
            );
        }
    }

    // The FIFO global regression lives in the black-box `tests/it/compact.rs`: creating
    // a FIFO needs `mkfifo(1)`, and `std::process::Command` is a crate-wide disallowed
    // type whose only sanctioned doors are in the it-target (`crate::cli::mkfifo`). A
    // unit test here would open a new Command door in `src/` and fail the capability
    // self-test. The DIRECTORY-as-local case below needs no such fixture.

    #[test]
    fn a_present_nonregular_local_config_is_refused_not_ignored() {
        // The origin's local `.ae/config` exists but is a DIRECTORY (a present, non-
        // regular node). It must refuse — not silently fall back to the valid global as
        // if the local overlay were absent.
        let s = Scratch::new("nonreglocal");
        std::fs::create_dir_all(s.0.join(".ae").join("config")).unwrap();
        let dir = session(&s, "", Some("[workspace]\nmain = cl\n"));
        let (code, out, err) = run(&dir, false);
        assert_eq!(code, 1, "{err}");
        assert!(
            err.contains("local .ae/config") && err.contains("not a readable regular file"),
            "{err}"
        );
        assert!(out.is_empty(), "no tuple emitted on refusal: {out:?}");
    }

    #[test]
    fn an_untraversable_local_config_is_refused_not_ignored() {
        // The local override EXISTS but its parent `.ae` is untraversable, so lstat on it
        // fails with a permission error — NOT NotFound. Existence cannot be proven, so it
        // must refuse rather than silently use the (valid) global config. Regression for
        // the absent-vs-can't-prove-absence conflation.
        use std::os::unix::fs::PermissionsExt as _;
        let s = Scratch::new("untraversable");
        let dotae = s.0.join(".ae");
        std::fs::create_dir_all(&dotae).unwrap();
        std::fs::write(dotae.join("config"), "[workspace]\nmain = local\n").unwrap();
        std::fs::set_permissions(&dotae, std::fs::Permissions::from_mode(0o000)).unwrap();
        // If this process can still traverse `.ae` (e.g. it runs as root), the premise
        // does not hold — restore and skip rather than assert a false negative.
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: this test's own premise check — that `.ae` really is \
                      untraversable for this process — see clippy.toml"
        )]
        let probe = std::fs::symlink_metadata(dotae.join("config"));
        let denied = probe.is_err();
        if !denied {
            let _ = std::fs::set_permissions(&dotae, std::fs::Permissions::from_mode(0o755));
            eprintln!("`.ae` traversal not denied (root?); skipping untraversable regression");
            return;
        }
        let dir = session(&s, "", Some("[workspace]\nmain = cl\n"));
        let (code, out, err) = run(&dir, false);
        // Restore before asserting so the scratch dir can always be cleaned up on drop.
        let _ = std::fs::set_permissions(&dotae, std::fs::Permissions::from_mode(0o755));
        assert_eq!(code, 1, "{err}");
        assert!(err.contains("local .ae/config"), "{err}");
        assert!(out.is_empty(), "no tuple emitted on refusal: {out:?}");
    }

    // ---- FrozenTuple ----

    fn tuple10(
        name: &str,
        uuid: &str,
        mode: &str,
        origin: &str,
        config: &str,
        purge: bool,
    ) -> String {
        [
            name,
            uuid,
            "session",
            mode,
            origin,
            config,
            if purge { "true" } else { "false" },
            "/arch",
            "cl:main",
            "main=cl workers=-",
        ]
        .join("\u{1f}")
    }

    #[test]
    fn frozen_tuple_parses_ten_fields_and_keeps_the_seven_rust_uses() {
        let f = FrozenTuple::parse(&tuple10("sess", UUID, "local", "/o", "/c", true))
            .expect("ten fields parse");
        assert_eq!(f.name, "sess");
        assert_eq!(f.uuid, UUID);
        assert_eq!(f.mode, "local");
        assert_eq!(f.origin, "/o");
        assert_eq!(f.config, "/c");
        assert!(f.purge);
        // The roster (10th field) is now kept too — the revalidate roster-drift gate reads it.
        assert_eq!(f.roster, "main=cl workers=-");
    }

    #[test]
    fn frozen_tuple_rejects_wrong_arity() {
        assert!(FrozenTuple::parse("only-one").is_none());
        assert!(FrozenTuple::parse("a\u{1f}b\u{1f}c").is_none());
        let eleven = "x\u{1f}".repeat(10) + "extra";
        assert!(FrozenTuple::parse(&eleven).is_none(), "eleven fields");
        // A trailing newline is tolerated (the tuple is one line).
        assert!(
            FrozenTuple::parse(&format!(
                "{}\n",
                tuple10("s", UUID, "local", "/o", "/c", false)
            ))
            .is_some()
        );
    }

    // ---- verify_stopped selector guard (C1) ----
    //
    // The recorded-server QUERY tri-state (present → Alive, answered-absent and
    // clean-dead → Stopped, any other failure → Unknown) is the pure
    // `crate::tmux::interpret_stopped`, unit-tested at its own site. Here we cover
    // only what `verify_stopped` itself decides BEFORE any query: a non-positive
    // selector fails closed without asking a server at all.

    #[test]
    fn stop_missing_or_ambiguous_selector_is_unknown() {
        // No selector recorded → Missing → Unknown, before any server is queried.
        assert_eq!(verify_stopped(b"mode=local\n", "sess"), StopState::Unknown);
        // Two selector keys → Ambiguous → Unknown.
        let ambiguous = b"tmux_server=a\ntmux_server=b\ntmux_server_kind=name\n";
        assert_eq!(verify_stopped(ambiguous, "sess"), StopState::Unknown);
    }

    // ---- revalidate (the authorization gate) ----

    fn frozen(origin: &Path, config: &Path) -> FrozenTuple {
        FrozenTuple {
            name: "sess".to_owned(),
            uuid: UUID.to_owned(),
            mode: "local".to_owned(),
            origin: origin.display().to_string(),
            config: config.display().to_string(),
            purge: false,
            // Every revalidate fixture's config is `[workspace] main = cl` (no workers), so
            // this is the roster it resolves to — a match, so the drift gate passes and each
            // test exercises the check it targets rather than tripping the roster gate first.
            roster: "main=cl workers=-".to_owned(),
        }
    }

    #[test]
    fn revalidate_accepts_the_unchanged_session() {
        let s = Scratch::new("reval-ok");
        let dir = session(&s, "", Some("[workspace]\nmain = cl\n"));
        let f = frozen(&s.0, &s.0.join("config"));
        assert!(revalidate(&dir, &f, false, "test").is_ok());
    }

    #[test]
    fn revalidate_refuses_an_altered_name_binding_it_to_the_operand() {
        // The altered-tuple attack: field 1 is rewritten from the real session name to some
        // other (absent) name. Revalidation must refuse on the name↔operand mismatch BEFORE
        // any stop query — otherwise the stop check would prove the WRONG (absent) name
        // stopped while the live session runs on, and teardown would delete the live session.
        let s = Scratch::new("reval-name");
        let dir = session(&s, "", Some("[workspace]\nmain = cl\n"));
        let mut f = frozen(&s.0, &s.0.join("config"));
        f.name = "ghost".to_owned(); // the real session dir basename is "sess"
        let e = revalidate(&dir, &f, false, "test").unwrap_err();
        assert!(
            e.contains("the session directory is 'sess'")
                && e.contains("does not point at this session"),
            "{e}"
        );
    }

    #[test]
    fn revalidate_refuses_a_replacement_uuid() {
        let s = Scratch::new("reval-repl");
        let dir = session(&s, "", Some("[workspace]\nmain = cl\n"));
        let mut f = frozen(&s.0, &s.0.join("config"));
        f.uuid = "99999999-9999-9999-9999-999999999999".to_owned();
        let e = revalidate(&dir, &f, false, "test").unwrap_err();
        assert!(e.contains("not the session that was authorized"), "{e}");
    }

    #[test]
    fn revalidate_refuses_a_changed_origin() {
        let s = Scratch::new("reval-origin");
        let dir = session(&s, "", Some("[workspace]\nmain = cl\n"));
        let mut f = frozen(&s.0, &s.0.join("config"));
        f.origin = "/somewhere/else".to_owned();
        assert!(
            revalidate(&dir, &f, false, "test")
                .unwrap_err()
                .contains("changed origin")
        );
    }

    #[test]
    fn revalidate_refuses_a_purge_flip_unless_keep_history() {
        let s = Scratch::new("reval-purge");
        let dir = session(
            &s,
            "",
            Some("[workspace]\nmain = cl\npurge_agent_history = true\n"),
        );
        let f = frozen(&s.0, &s.0.join("config"));
        // frozen.purge is false, but the live config now purges → refuse.
        assert!(
            revalidate(&dir, &f, false, "test")
                .unwrap_err()
                .contains("purge_agent_history")
        );
        // --keep-history authorizes it.
        assert!(revalidate(&dir, &f, true, "test").is_ok());
    }

    #[test]
    fn revalidate_refuses_a_roster_rewritten_under_the_window() {
        let s = Scratch::new("reval-roster");
        let dir = session(&s, "", Some("[workspace]\nmain = cl\n"));
        let f = frozen(&s.0, &s.0.join("config"));
        // Unchanged config → the shown roster still resolves → accepted.
        assert!(revalidate(&dir, &f, false, "test").is_ok());
        // Rewritten DURING the window to start a different main. The config PATH is unchanged
        // (that gate passes), but the roster it now resolves to is not the one shown.
        std::fs::write(s.0.join("config"), "[workspace]\nmain = other\n").unwrap();
        let err = revalidate(&dir, &f, false, "after confirmation").unwrap_err();
        assert!(
            err.contains("would now start a different roster than the one shown"),
            "{err}"
        );
        assert!(err.contains("after confirmation"), "{err}"); // the gate label threads through
        assert!(err.contains("main=cl"), "{err}"); // names what was shown
        assert!(err.contains("main=other"), "{err}"); // and what would start now
    }

    #[test]
    fn revalidate_refuses_a_surviving_spawned_agent() {
        let s = Scratch::new("reval-spawn");
        let dir = s.0.join("sess");
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = s.0.join("config");
        std::fs::write(&cfg, "[workspace]\nmain = cl\n").unwrap();
        std::fs::write(
            dir.join("meta"),
            format!(
                "session_id={UUID}\nmode=local\norigin={}\nagent.main=cl:main:{UUID}\nagent.spawned.0=gpt:helper:{UUID}\nconfig={}\n",
                s.0.display(),
                cfg.display()
            ),
        )
        .unwrap();
        let f = frozen(&s.0, &cfg);
        let e = revalidate(&dir, &f, false, "test").unwrap_err();
        assert!(
            e.contains("still has spawned agents") && e.contains("gpt:helper"),
            "{e}"
        );
    }

    // ---- teardown's archive re-preflight (BLOCKER 2 / C2 separate-stage boundary) ----

    /// Publish a real archive from a session under `<home>/sessions/sess`, returning the
    /// archive root. Mirrors the archive step's own inputs so the recovery-point preflight
    /// sees exactly what teardown would.
    fn published(s: &Scratch) -> PathBuf {
        let dir = s.0.join("sessions").join("sess");
        std::fs::create_dir_all(dir.join("messages")).unwrap();
        let archive_root = s.0.join("archive");
        std::fs::create_dir_all(&archive_root).unwrap();
        std::fs::write(
            dir.join("meta"),
            format!(
                "session=sess\nsession_id={UUID}\nsession_id_origin=session\nmode=local\norigin=/o\nagent.main=cl:lead:sid\n"
            ),
        )
        .unwrap();
        std::fs::write(dir.join("memo.tsv"), "ts1\tcl:lead\thandover\thi\n").unwrap();
        std::fs::write(
            dir.join("events.jsonl"),
            "{\"ts\":\"2026-08-01T00:00:00Z\",\"actor\":\"cl:lead\",\"action\":\"ask\"}\n",
        )
        .unwrap();
        std::fs::write(dir.join("messages").join("m1.txt"), "hello\n").unwrap();
        let ops = crate::archive::publish::Ops {
            push_outcome: "-",
            push_ref: "-",
            preserved: "-",
            workdir: "-",
            archived_at: "2026-08-01T00:00:00Z",
        };
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code =
            crate::archive::publish::run(&dir, &ops, &mut out, &mut err).expect("publish ran");
        assert_eq!(code, 0, "publish: {}", String::from_utf8_lossy(&err));
        archive_root
    }

    #[test]
    fn teardown_preflight_accepts_a_real_archive_and_rejects_every_bad_shape() {
        let s = Scratch::new("td-archive");
        let archive_root = published(&s);
        let archive = archive_root.join(UUID);

        // A real, freshly published archive is an inheritable recovery point.
        let mut err = Vec::new();
        assert!(
            archive_recovery_point_valid(&archive_root, UUID, &mut err).unwrap(),
            "a valid archive must be accepted: {}",
            String::from_utf8_lossy(&err)
        );

        // Emptied after the archive step proved it (an empty dir is not an archive).
        let stash = s.0.join("stash");
        std::fs::rename(&archive, &stash).unwrap();
        std::fs::create_dir_all(&archive).unwrap();
        let mut err2 = Vec::new();
        assert!(
            !archive_recovery_point_valid(&archive_root, UUID, &mut err2).unwrap(),
            "an emptied archive dir must be rejected"
        );

        // Replaced by a symlink to the valid tree elsewhere (never followed out of root).
        std::fs::remove_dir(&archive).unwrap();
        std::os::unix::fs::symlink(&stash, &archive).unwrap();
        let mut err3 = Vec::new();
        assert!(
            !archive_recovery_point_valid(&archive_root, UUID, &mut err3).unwrap(),
            "a symlinked archive must be rejected"
        );

        // Vanished entirely.
        std::fs::remove_file(&archive).unwrap(); // remove the symlink
        let mut err4 = Vec::new();
        assert!(
            !archive_recovery_point_valid(&archive_root, UUID, &mut err4).unwrap(),
            "a missing archive must be rejected"
        );
    }

    // ---- the semantic handover wait + cancel (slice B) ----

    const REF: &str = "ae-20260829T000000Z-abcd1234";

    /// A bare session dir named `sess` under the scratch root — the basename `wait_step`
    /// and the reply/session checks bind to.
    fn handover_dir(s: &Scratch) -> PathBuf {
        let dir = s.0.join("sess");
        std::fs::create_dir_all(dir.join("messages")).unwrap();
        dir
    }

    /// Seed a compact handover ask opening from the reserved compact actor — SLOTLESS
    /// sender (`actor_session` present, no `actor_slot`), addressed to main, with a stored
    /// body carrying the memo baseline. The ledger reads it as one Pending request.
    fn seed_handover(dir: &Path, baseline: usize) {
        let body = dir.join("messages").join("handover.ask.txt");
        std::fs::write(
            &body,
            format!("COMPACT HANDOVER ...\nAE-COMPACT-MEMO-BASELINE={baseline}\n"),
        )
        .unwrap();
        let ask = format!(
            "{{\"ts\":\"2026-08-29T00:00:00Z\",\"actor\":\"ae:compact:{UUID}\",\"action\":\"ask\",\"target\":\"cl:main\",\"ref\":\"{REF}\",\"body_file\":\"{}\",\"actor_session\":\"sess\",\"target_slot\":\"main\",\"target_session\":\"sess\"}}\n",
            body.display()
        );
        std::fs::write(dir.join("events.jsonl"), ask).unwrap();
    }

    fn append_reply(dir: &Path) {
        use std::io::Write as _;
        let reply = format!(
            "{{\"ts\":\"2026-08-29T00:01:00Z\",\"actor\":\"cl:main\",\"action\":\"reply\",\"ref\":\"{REF}\",\"actor_slot\":\"main\",\"actor_session\":\"sess\",\"target\":\"ae:compact:{UUID}\"}}\n"
        );
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join("events.jsonl"))
            .unwrap();
        f.write_all(reply.as_bytes()).unwrap();
    }

    fn status_of(dir: &Path) -> requests::Status {
        let container = event_text::read_container(&dir.join("events.jsonl"));
        requests::states(&container)
            .into_iter()
            .find(|r| r.id == REF.as_bytes())
            .expect("the seeded request")
            .status
    }

    // ---- pure predicates ----

    #[test]
    fn baseline_reads_the_body_marker() {
        assert_eq!(
            baseline_from_body(b"intro\nAE-COMPACT-MEMO-BASELINE=42\nmore\n"),
            Some(42)
        );
        // A legitimate zero (empty memo at request time) is NOT the same as no evidence.
        assert_eq!(
            baseline_from_body(b"AE-COMPACT-MEMO-BASELINE=0\n"),
            Some(0),
            "a real 0 baseline is valid"
        );
        // Absent or malformed is UNAVAILABLE EVIDENCE — None, never a silent 0.
        assert_eq!(
            baseline_from_body(b"no marker here\n"),
            None,
            "absent -> None"
        );
        assert_eq!(
            baseline_from_body(b"AE-COMPACT-MEMO-BASELINE=notanumber\n"),
            None,
            "malformed -> None, never a fallback 0"
        );
    }

    #[test]
    fn memo_after_requires_a_handover_row_past_the_offset() {
        let memo = b"a\tcl\thandover\told\n";
        assert!(!memo_after(memo, memo.len()), "nothing past the end");
        assert!(memo_after(memo, 0), "a valid handover row from offset 0");
        assert!(!memo_after(b"a\tcl\tchat\thi\n", 0), "wrong topic");
        assert!(!memo_after(b"a\tcl\thandover\t\n", 0), "empty text");
        assert!(!memo_after(b"\t\thandover\ttext\n", 0), "no ts/author");
    }

    #[test]
    fn reply_seen_matches_ref_slot_and_session_exactly() {
        let ev = format!(
            "{{\"ts\":\"t\",\"actor\":\"cl:main\",\"action\":\"reply\",\"ref\":\"{REF}\",\"actor_slot\":\"main\",\"actor_session\":\"sess\"}}\n"
        );
        assert!(reply_seen(ev.as_bytes(), REF, "sess"));
        assert!(!reply_seen(ev.as_bytes(), "other-ref", "sess"), "wrong ref");
        assert!(!reply_seen(ev.as_bytes(), REF, "other"), "wrong session");
        let worker = ev.replace("\"actor_slot\":\"main\"", "\"actor_slot\":\"worker.0\"");
        assert!(!reply_seen(worker.as_bytes(), REF, "sess"), "wrong slot");
    }

    // ---- wait_step (timeout 0 => a single poll then the timeout branch) ----

    #[test]
    fn wait_succeeds_when_both_facts_are_present() {
        let s = Scratch::new("wait-both");
        let dir = handover_dir(&s);
        seed_handover(&dir, 0);
        append_reply(&dir);
        std::fs::write(
            dir.join("memo.tsv"),
            "2026-08-29T00:01:00Z\tcl:main\thandover\tpicking up\n",
        )
        .unwrap();
        let mut err = Vec::new();
        let code = wait_step(&dir, REF, 0, &mut err).unwrap();
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&err));
        assert!(String::from_utf8_lossy(&err).contains("handover complete"));
    }

    #[test]
    fn wait_times_out_and_leaves_the_request_pending_when_only_the_memo_arrived() {
        let s = Scratch::new("wait-memo");
        let dir = handover_dir(&s);
        seed_handover(&dir, 0);
        std::fs::write(
            dir.join("memo.tsv"),
            "2026-08-29T00:01:00Z\tcl:main\thandover\tpicking up\n",
        )
        .unwrap();
        let mut err = Vec::new();
        let code = wait_step(&dir, REF, 0, &mut err).unwrap();
        assert_eq!(code, EXIT_FAILED);
        let msg = String::from_utf8_lossy(&err);
        // The guidance names the session (the frozen contract the integration arms assert),
        // and points at the two ways forward.
        assert!(
            msg.contains("stays open") && msg.contains("ae compact --digest-only sess"),
            "{msg}"
        );
        // Ruling A: a normal timeout writes NOTHING to the ledger — the request is reusable.
        assert_eq!(status_of(&dir), requests::Status::Pending);
    }

    #[test]
    fn wait_reports_the_reply_closed_case() {
        // The reply arrived, the memo did not: the reply already closed the ref, so a
        // re-run must send a fresh request — the guidance says so.
        let s = Scratch::new("wait-reply");
        let dir = handover_dir(&s);
        seed_handover(&dir, 0);
        append_reply(&dir);
        let mut err = Vec::new();
        let code = wait_step(&dir, REF, 0, &mut err).unwrap();
        assert_eq!(code, EXIT_FAILED);
        let msg = String::from_utf8_lossy(&err);
        assert!(
            msg.contains("answered and closed") && msg.contains("FRESH"),
            "{msg}"
        );
    }

    #[test]
    fn wait_refuses_an_unknown_ref() {
        let s = Scratch::new("wait-unknown");
        let dir = handover_dir(&s);
        std::fs::write(dir.join("events.jsonl"), "").unwrap();
        let mut err = Vec::new();
        let code = wait_step(&dir, REF, 0, &mut err).unwrap();
        assert_eq!(code, EXIT_FAILED);
        assert!(String::from_utf8_lossy(&err).contains("no such request"));
    }

    #[test]
    fn wait_fails_closed_when_the_baseline_evidence_is_unavailable() {
        // The BLOCKER (p4runner review dc59b178): a memo written BEFORE the request PLUS a
        // reply must NOT satisfy the two-fact gate when the baseline cannot be established.
        // A missing/non-regular body or a body with no valid marker would default to
        // baseline 0, and `memo_after(old_memo, 0)` would then pass the pre-existing memo as
        // "new". This row is a VALID handover row that baseline 0 would have accepted.
        let old_memo = "2026-08-29T00:00:00Z\tcl:main\thandover\twritten BEFORE the request\n";
        let body = |dir: &Path| dir.join("messages").join("handover.ask.txt");

        // (a) body present but carries NO valid marker.
        {
            let s = Scratch::new("wait-nomark");
            let dir = handover_dir(&s);
            seed_handover(&dir, 0);
            std::fs::write(body(&dir), "COMPACT HANDOVER but no baseline marker\n").unwrap();
            append_reply(&dir);
            std::fs::write(dir.join("memo.tsv"), old_memo).unwrap();
            let mut err = Vec::new();
            let code = wait_step(&dir, REF, 0, &mut err).unwrap();
            let msg = String::from_utf8_lossy(&err);
            assert_eq!(code, EXIT_FAILED, "{msg}");
            assert!(
                msg.contains("cannot establish the handover memo baseline"),
                "{msg}"
            );
            // Fail-closed writes nothing to the ledger — the request stays reusable.
            assert_eq!(status_of(&dir), requests::Status::Pending);
        }

        // (b) body MISSING entirely — the same old memo + reply still fails closed.
        {
            let s = Scratch::new("wait-nobody");
            let dir = handover_dir(&s);
            seed_handover(&dir, 0);
            std::fs::remove_file(body(&dir)).unwrap();
            append_reply(&dir);
            std::fs::write(dir.join("memo.tsv"), old_memo).unwrap();
            let mut err = Vec::new();
            let code = wait_step(&dir, REF, 0, &mut err).unwrap();
            let msg = String::from_utf8_lossy(&err);
            assert_eq!(code, EXIT_FAILED, "{msg}");
            assert!(
                msg.contains("cannot establish the handover memo baseline"),
                "{msg}"
            );
        }
        // The legitimate-zero control (a real `AE-COMPACT-MEMO-BASELINE=0` proceeds) is
        // `wait_succeeds_when_both_facts_are_present`, which seeds baseline 0 and succeeds.
    }

    // ---- cancel_step (--digest-only withdrawal), pinned in the request VIEW ----

    #[test]
    fn cancel_withdraws_a_compact_handover_and_pins_it_cancelled() {
        let s = Scratch::new("cancel-ok");
        let dir = handover_dir(&s);
        seed_handover(&dir, 0);
        assert_eq!(status_of(&dir), requests::Status::Pending, "precondition");
        let mut err = Vec::new();
        assert_eq!(
            cancel_step(&dir, REF, &mut err).unwrap(),
            0,
            "{}",
            String::from_utf8_lossy(&err)
        );
        // The PIN: requests reads it as cancelled, not merely that a cancel byte was written.
        assert_eq!(status_of(&dir), requests::Status::Cancelled);
    }

    #[test]
    fn cancel_refuses_a_request_not_opened_by_compact() {
        let s = Scratch::new("cancel-foreign");
        let dir = handover_dir(&s);
        // A normal pane-to-pane ask, not the reserved compact actor.
        let ask = format!(
            "{{\"ts\":\"2026-08-29T00:00:00Z\",\"actor\":\"cl:lead\",\"action\":\"ask\",\"target\":\"cl:main\",\"ref\":\"{REF}\",\"actor_slot\":\"main\",\"actor_session\":\"sess\",\"target_slot\":\"worker.0\",\"target_session\":\"sess\"}}\n"
        );
        std::fs::write(dir.join("events.jsonl"), ask).unwrap();
        let mut err = Vec::new();
        assert_eq!(cancel_step(&dir, REF, &mut err).unwrap(), EXIT_FAILED);
        assert!(String::from_utf8_lossy(&err).contains("not opened by compact"));
    }

    #[test]
    fn cancel_is_idempotent_once_withdrawn() {
        // The already-closed no-op: a second withdrawal finds the request no longer
        // Pending and writes nothing — no duplicate cancel event, still Cancelled.
        let s = Scratch::new("cancel-twice");
        let dir = handover_dir(&s);
        seed_handover(&dir, 0);
        let mut err = Vec::new();
        assert_eq!(cancel_step(&dir, REF, &mut err).unwrap(), 0);
        assert_eq!(status_of(&dir), requests::Status::Cancelled);
        let before = event_text::read_container(&dir.join("events.jsonl"));
        let mut err2 = Vec::new();
        assert_eq!(cancel_step(&dir, REF, &mut err2).unwrap(), 0);
        let after = event_text::read_container(&dir.join("events.jsonl"));
        assert_eq!(before, after, "no second cancel event appended");
        assert_eq!(status_of(&dir), requests::Status::Cancelled);
    }

    #[test]
    fn cancel_refuses_an_unknown_ref() {
        let s = Scratch::new("cancel-unknown");
        let dir = handover_dir(&s);
        std::fs::write(dir.join("events.jsonl"), "").unwrap();
        let mut err = Vec::new();
        assert_eq!(cancel_step(&dir, REF, &mut err).unwrap(), EXIT_FAILED);
        assert!(String::from_utf8_lossy(&err).contains("no such request"));
    }

    // ---- memo baseline + find-outstanding (the bash-driver state reads) ----

    #[test]
    fn memo_baseline_is_the_file_byte_size() {
        let s = Scratch::new("memo-baseline");
        let dir = handover_dir(&s);
        // No memo yet → 0.
        let mut out = Vec::new();
        memo_baseline_step(&dir, &mut out).unwrap();
        assert_eq!(out, b"0");
        // With content → its byte length, printed without a trailing newline.
        let body = "2026-08-29T00:00:00Z\tcl:main\thandover\thi\n";
        std::fs::write(dir.join("memo.tsv"), body).unwrap();
        let mut out2 = Vec::new();
        memo_baseline_step(&dir, &mut out2).unwrap();
        assert_eq!(out2, body.len().to_string().into_bytes());
    }

    #[test]
    fn find_outstanding_prints_a_pending_compact_ref_only() {
        let s = Scratch::new("find-outstanding");
        let dir = handover_dir(&s);
        std::fs::write(dir.join("meta"), format!("session_id={UUID}\nmode=local\n")).unwrap();
        // No events → nothing outstanding.
        std::fs::write(dir.join("events.jsonl"), "").unwrap();
        let mut out = Vec::new();
        find_outstanding_step(&dir, &mut out).unwrap();
        assert!(out.is_empty(), "no request → no ref");
        // A pending compact handover → its ref.
        seed_handover(&dir, 0);
        let mut out2 = Vec::new();
        find_outstanding_step(&dir, &mut out2).unwrap();
        assert_eq!(out2, REF.as_bytes());
        // Once withdrawn it is no longer outstanding.
        let mut e = Vec::new();
        cancel_step(&dir, REF, &mut e).unwrap();
        let mut out3 = Vec::new();
        find_outstanding_step(&dir, &mut out3).unwrap();
        assert!(out3.is_empty(), "a cancelled request is not outstanding");
    }

    #[test]
    fn find_outstanding_ignores_a_non_compact_pending_request() {
        let s = Scratch::new("find-foreign");
        let dir = handover_dir(&s);
        std::fs::write(dir.join("meta"), format!("session_id={UUID}\nmode=local\n")).unwrap();
        // A normal pane-to-pane ask is pending but NOT a compact request.
        let ask = format!(
            "{{\"ts\":\"t\",\"actor\":\"cl:lead\",\"action\":\"ask\",\"target\":\"cl:main\",\"ref\":\"{REF}\",\"actor_slot\":\"main\",\"actor_session\":\"sess\"}}\n"
        );
        std::fs::write(dir.join("events.jsonl"), ask).unwrap();
        let mut out = Vec::new();
        find_outstanding_step(&dir, &mut out).unwrap();
        assert!(out.is_empty(), "only a compact-actor request counts");
    }
}
