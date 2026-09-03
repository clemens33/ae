//! `_compact`: the whole compact operation, in the frozen order.
//!
//! Ported from `ae`'s `cmd_compact`. Every STEP was already the core's
//! (`_compact-freeze`, `-revalidate`, `-memo-baseline`, `-find-outstanding`,
//! `-cancel`, `-wait`, `-archive`, `-teardown`); what bash held was the
//! sequence and the two lock regions, and that is what moves here:
//!
//! * **(a) freeze** — one coherent, read-only observation of the source, which
//!   every later phase is held to. NOT taken under the lifecycle lock;
//!   revalidation is what protects identity, not the freeze.
//! * **(1) confirm** — on stderr. THE PROMPT IS NOT THE CONTRACT: stdout stays
//!   empty unless the boundary is actually crossed.
//! * **(b) handover** — under the lifecycle lock, so a replacement is never
//!   MESSAGED. The lock is RELEASED before the wait, which polls append-only
//!   records and needs none.
//! * **(c) the bounded wait** — a timeout leaves the request as it found it; a
//!   normal timeout is a resumable pause.
//! * **(d) revalidate, stop, archive, teardown** — under ONE lock. The
//!   recovery command is proven and printed BEFORE the teardown removes the
//!   source.
//! * **(e) the boundary** — the four stdout lines, then the exec plan for the
//!   relaunch.
//!
//! # The roster is FROZEN, and the relaunch must use the frozen one
//!
//! The child is launched from the roster the freeze resolved and the human was
//! SHOWN, never from a config re-read after the boundary: a config rewritten
//! under the window would start different agents than the ones authorised.
//! [`ExecPlan`] is how that frozen answer travels to the relaunch, alongside
//! the `--from` proof the child re-proves before publishing its own meta.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::compact;
use crate::inventory::ServerId;
use crate::meta::{self, ServerSelector};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::time::Timestamp;
use crate::tracked;
use crate::transport;

use super::{kill_verified, live_id, lock, name_is_usable, server_of, sessions_dir};

/// The frozen usage line.
const USAGE: &str =
    "Usage: _compact [-f] [--keep-history] [--digest-only] <session-name> [--exec-plan <path>]";

/// The frozen default handover budget, in seconds.
const DEFAULT_HANDOVER_SECS: u64 = 300;

/// What the argv said.
struct Args {
    name: String,
    force: bool,
    keep_history: bool,
    digest_only: bool,
    /// Where to write the [`ExecPlan`] the caller relaunches from. Optional:
    /// without it the operation still completes, and the recovery line on
    /// stderr is the route back.
    exec_plan: Option<PathBuf>,
}

/// `_compact [-f] [--keep-history] [--digest-only] <name> [--exec-plan <path>]`.
#[allow(
    clippy::too_many_lines,
    reason = "the five phases and their two lock regions ARE the contract; splitting them would put the order in two places"
)]
pub(crate) fn run(
    root: &Path,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let args = match parse(tail) {
        Ok(args) => args,
        Err(message) => {
            writeln!(err, "{message}")?;
            return Ok(EXIT_USAGE);
        }
    };
    let name = args.name.clone();
    if !name_is_usable(root, &name) {
        writeln!(err, "ae: '{name}' is not a usable session name.")?;
        return Ok(EXIT_FAILED);
    }
    // FIRST, before a mint, an event, a prompt or an archive read.
    if transport::observe_current_session(&ServerId::Ambient).as_deref() == Some(name.as_str()) {
        writeln!(
            err,
            "Error: cannot compact the current session. Detach, then run: ae compact {name}"
        )?;
        return Ok(EXIT_FAILED);
    }
    let dir = sessions_dir(root).join(&name);

    // ── phase (a): freeze ───────────────────────────────────────────────────
    // CLEAN CUT: the core resolves and freezes the authorization tuple, or
    // refuses a session it cannot classify. There is no fallback.
    let mut tuple = Vec::new();
    if compact::freeze(&dir, args.keep_history, &mut tuple, err)? != 0 {
        return Ok(EXIT_FAILED);
    }
    let tuple = String::from_utf8_lossy(&tuple).trim_end().to_owned();
    let Some(frozen) = Frozen::parse(&tuple) else {
        writeln!(
            err,
            "compact: internal error — the frozen tuple did not parse (expected ten fields)."
        )?;
        return Ok(EXIT_FAILED);
    };

    // The server the SOURCE ran on, read HERE — while its meta is still on
    // disk. The teardown removes the session directory before the boundary, so
    // a read after it finds nothing and the child would land on the caller's
    // ambient server, which is a different server whenever the fleet runs on a
    // named socket.
    let (child_server_kind, child_server_value) = recorded_server(&dir);

    // STDERR: compact's STDOUT is a contract — the four boundary lines, in
    // order, and nothing else. Everything before the boundary is diagnostics.
    writeln!(
        err,
        "compact: {} ({}) resolved and frozen.",
        frozen.name, frozen.mode
    )?;
    writeln!(err, "session: {}", frozen.name)?;
    writeln!(err, "uuid: {} ({})", frozen.uuid, frozen.uuid_origin)?;
    writeln!(err, "mode: {}", frozen.mode)?;
    writeln!(err, "origin: {}", frozen.origin)?;
    writeln!(
        err,
        "config: {}",
        if frozen.config.is_empty() {
            "-"
        } else {
            &frozen.config
        }
    )?;
    writeln!(
        err,
        "conversation files: {}",
        if frozen.purge { "DELETED" } else { "kept" }
    )?;
    writeln!(err, "archive: {}", frozen.archive)?;

    // ── phase (1): confirm ──────────────────────────────────────────────────
    if !args.force {
        confirm_body(&frozen, args.digest_only, err)?;
        write!(err, "Continue? [y/N] ")?;
        err.flush()?;
        match read_reply() {
            Some(reply) if reply.starts_with('y') || reply.starts_with('Y') => {}
            Some(_) => {
                writeln!(err, "Aborted.")?;
                return Ok(0);
            }
            // EOF IS NOT CONSENT — AND NOT A CRASH. The exit STATUS is the only
            // channel that tells "the operator said no" (0) from "the question
            // never reached anyone" (1).
            None => {
                writeln!(
                    err,
                    "Error: could not obtain confirmation — no input on stdin."
                )?;
                writeln!(
                    err,
                    "  Nothing was stopped, archived or changed. Run it from a terminal, or"
                )?;
                writeln!(err, "  pass -f if you mean to proceed without being asked.")?;
                return Ok(EXIT_FAILED);
            }
        }
    }

    // ── phase (b): the semantic handover ────────────────────────────────────
    // Under the lifecycle lock so a replacement is never MESSAGED.
    let reference;
    {
        let Ok(_guard) = lock(root, &frozen.name) else {
            writeln!(
                err,
                "Error: another lifecycle operation is in progress for '{}' — retry shortly. Nothing was changed.",
                frozen.name
            )?;
            return Ok(EXIT_FAILED);
        };
        if compact::revalidate_step(&dir, &tuple, args.keep_history, "after confirmation", err)?
            != 0
        {
            return Ok(EXIT_FAILED);
        }
        // A retry reuses an outstanding request (and, through its stored body,
        // its baseline) rather than delivering a duplicate.
        let mut pending = outstanding(&dir)?;
        if args.digest_only {
            // The ONLY degradation, and it is explicit. Withdraw anything
            // outstanding so no later archive reports an open request nobody is
            // waiting on.
            if !pending.is_empty() {
                if compact::cancel_step(&dir, &pending, err)? != 0 {
                    return Ok(EXIT_FAILED);
                }
                writeln!(err, "compact: withdrew {pending} (--digest-only).")?;
                pending.clear();
            }
            writeln!(
                err,
                "compact: semantic handover skipped (--digest-only); the digest is the handover."
            )?;
        } else if pending.is_empty() {
            let mut baseline = Vec::new();
            let baseline = if compact::memo_baseline_step(&dir, &mut baseline)? == 0 {
                String::from_utf8_lossy(&baseline).trim().to_owned()
            } else {
                "0".to_owned()
            };
            let body = request_text(&dir, &baseline);
            let sender = tracked::Sender {
                display: format!("ae:compact:{}", frozen.uuid),
                slot: String::new(),
            };
            let mut delivered = Vec::new();
            let code = tracked::run(
                tracked::Kind::Ask,
                &dir,
                &[frozen.main_ref.clone(), body],
                Some(&sender),
                &frozen.name,
                Timestamp::now(),
                entropy(),
                std::time::Duration::from_millis(0),
                &mut delivered,
                err,
            )?;
            if code != 0 {
                writeln!(
                    err,
                    "Error: could not deliver the handover request to '{}'.",
                    frozen.main_ref
                )?;
                writeln!(
                    err,
                    "  Nothing was stopped and nothing was archived; the session is untouched."
                )?;
                return Ok(EXIT_FAILED);
            }
            pending = outstanding(&dir)?;
            if pending.is_empty() {
                writeln!(
                    err,
                    "Error: the handover request was delivered but not recorded — refusing to wait on a request ae cannot see."
                )?;
                return Ok(EXIT_FAILED);
            }
            writeln!(
                err,
                "compact: handover requested as {pending}; waiting for the reply AND a new handover memo."
            )?;
        } else {
            writeln!(
                err,
                "compact: reusing the outstanding handover request {pending}."
            )?;
        }
        reference = pending;
    }

    // ── phase (c): the bounded wait ─────────────────────────────────────────
    // On timeout the wait leaves the request as it found it — a normal timeout
    // is a resumable pause, not a withdrawal.
    if !args.digest_only && compact::wait_step(&dir, &reference, handover_secs(err)?, err)? != 0 {
        return Ok(EXIT_FAILED);
    }

    // ── phase (d): revalidate, stop, archive, teardown — under ONE lock ─────
    let plan_line;
    {
        let Ok(_guard) = lock(root, &frozen.name) else {
            writeln!(
                err,
                "Error: another lifecycle operation is in progress for '{}' — retry shortly. Nothing was changed.",
                frozen.name
            )?;
            return Ok(EXIT_FAILED);
        };
        if compact::revalidate_step(
            &dir,
            &tuple,
            args.keep_history,
            "after the handover wait",
            err,
        )? != 0
        {
            return Ok(EXIT_FAILED);
        }
        // STOP on the recorded server, by EXACT id. Empty id = not live there;
        // the archive step's own verify-stopped gate then either proves it
        // stopped or refuses, rather than acting on a live session.
        let bytes = meta::read_bytes(&dir).unwrap_or_default();
        if let ServerSelector::Positive(selector) = server_of(&bytes) {
            let server = ServerId::Selected(selector);
            if let Some(id) = live_id(&server, &frozen.name)
                && !kill_verified(&server, &frozen.name, "compact", &id, err)?
            {
                return Ok(EXIT_FAILED);
            }
        }
        // ARCHIVE: revalidate + PROVE stopped + publish + print the recovery
        // command. Local mode manages no branch, so the git facts are recorded
        // honestly as not-managed.
        let archived_at = Timestamp::now().to_string();
        let mut recovery = Vec::new();
        let code = compact::archive_step(
            &dir,
            &tuple,
            args.keep_history,
            &archived_at,
            "not-managed",
            "-",
            "-",
            "",
            &mut recovery,
            err,
        )?;
        if code != 0 {
            return Ok(code);
        }
        // The recovery point is durable and proven — SHOW it before anything is
        // torn down.
        err.write_all(&recovery)?;
        // TEARDOWN: revalidate + PROVE stopped + re-preflight the archive +
        // remove the live session + print the exec plan.
        let mut plan = Vec::new();
        let code = compact::teardown_step(&dir, &tuple, args.keep_history, &mut plan, err)?;
        if code != 0 {
            return Ok(code);
        }
        plan_line = String::from_utf8_lossy(&plan).trim_end().to_owned();
    }

    // ── phase (e): the boundary ─────────────────────────────────────────────
    // THE ARCHIVE IS PUBLISHED AND THE SOURCE IS GONE. Everything from here is
    // unrecoverable-by-rollback, so the recovery line is emitted BEFORE the
    // relaunch is handed off.
    let (plan_name, plan_uuid) = plan_line
        .split_once('\u{1f}')
        .unwrap_or((frozen.name.as_str(), frozen.uuid.as_str()));
    let archive_root = root.join("archive");
    let mut proof = Vec::new();
    if crate::archive::from::run(&archive_root, &frozen.uuid, &mut proof, err)? != 0 {
        writeln!(
            err,
            "Error: the archive {} was published but does not validate for inheritance.",
            frozen.uuid
        )?;
        writeln!(
            err,
            "  The archive is intact and the source session is gone; start the fresh session by hand:"
        )?;
        writeln!(err, "      {}", recovery_command(&frozen))?;
        return Ok(EXIT_FAILED);
    }
    let proof = String::from_utf8_lossy(&proof).trim_end().to_owned();
    let recovery = recovery_command(&frozen);

    // The exec plan the relaunch is driven from. THE ROSTER IS THE FROZEN ONE:
    // written from the freeze artifact and never re-derived from a config that
    // may have been rewritten under the window.
    if let Some(path) = args.exec_plan.as_ref() {
        let record = [
            plan_name,
            plan_uuid,
            frozen.origin.as_str(),
            frozen.config.as_str(),
            proof.as_str(),
            frozen.roster.as_str(),
        ]
        .join("\u{1f}");
        if let Err(why) = write_plan(path, &record) {
            writeln!(
                err,
                "Error: the archive is published and the source session is gone, but the exec plan could not be written to {}: {why}",
                path.display()
            )?;
            writeln!(err, "  Start the fresh session by hand:")?;
            writeln!(err, "      {recovery}")?;
            return Ok(EXIT_FAILED);
        }
    }

    // The four stdout contract lines, in order, and nothing else.
    writeln!(out, "Archived {}", frozen.uuid)?;
    writeln!(out, "Archive: {}", frozen.archive)?;
    writeln!(out, "Digest: {}/digest.md", frozen.archive)?;
    writeln!(out, "Recovery: {recovery}")?;
    // The recovery information ALSO goes to stderr, so a broken stdout cannot
    // destroy the only route back.
    writeln!(err, "Recovery: {recovery}")?;
    writeln!(
        err,
        "kept agent conversation files (claude/codex token history; purge: ae end --purge-history)"
    )?;
    writeln!(
        err,
        "Starting fresh session {plan_name} from archive {plan_uuid}..."
    )?;
    // THE RELAUNCH, in this process. `--exec-plan` is the one shape that does
    // NOT relaunch: a caller that asked for the plan file has said it will
    // drive the child itself, and starting one here as well would give it two.
    // Everything the child needs travels as the plan — the frozen roster
    // included — so there is no config re-read and no exec through the glue.
    if args.exec_plan.is_some() {
        return Ok(0);
    }
    let child = crate::session_launch::Relaunch {
        home: root,
        name: plan_name,
        mode: &frozen.mode,
        origin: &frozen.origin,
        config: &frozen.config,
        uuid: plan_uuid,
        proof: &proof,
        roster: &frozen.roster,
        server_kind: &child_server_kind,
        server_value: &child_server_value,
    };
    // A child that will not start is NOT a failed compact: the archive is
    // published and proven, and the recovery command above is the route back.
    // Say so loudly and keep the boundary's exit code.
    let mut sink = Vec::new();
    if !matches!(
        crate::session_launch::relaunch(&child, &mut sink, err),
        Ok(0)
    ) {
        writeln!(
            err,
            "ae: the fresh session did not start. The archive is intact — start it by hand:"
        )?;
        writeln!(err, "      {recovery}")?;
    }
    err.write_all(&sink)?;
    Ok(0)
}

/// The tmux server a session's meta records, as the launch entry's two flags
/// spell it. Empty/empty when the record names none.
fn recorded_server(dir: &Path) -> (String, String) {
    let bytes = meta::read_bytes(dir).unwrap_or_default();
    match server_of(&bytes) {
        ServerSelector::Positive(crate::meta::Selector::Socket(path)) => {
            ("socket".to_owned(), path.display().to_string())
        }
        ServerSelector::Positive(crate::meta::Selector::Name(name)) => ("name".to_owned(), name),
        ServerSelector::Missing | ServerSelector::Ambiguous => (String::new(), String::new()),
    }
}

fn parse(tail: &[String]) -> Result<Args, String> {
    let mut args = Args {
        name: String::new(),
        force: false,
        keep_history: false,
        digest_only: false,
        exec_plan: None,
    };
    let mut rest = tail;
    while let [arg, tail @ ..] = rest {
        rest = tail;
        match arg.as_str() {
            "-f" | "--force" => args.force = true,
            "--keep-history" => args.keep_history = true,
            "--digest-only" => args.digest_only = true,
            "--exec-plan" => match rest {
                [path, tail @ ..] => {
                    args.exec_plan = Some(PathBuf::from(path));
                    rest = tail;
                }
                [] => return Err("Error: --exec-plan needs a path.".to_owned()),
            },
            "--purge-history" => {
                return Err(
                    "Error: --purge-history contradicts compact, which exists to keep the archive.\n  To end a session and delete its archive: ae end --purge-history <name>"
                        .to_owned(),
                );
            }
            "--assume-stopped" => {
                return Err(
                    "Error: --assume-stopped is an 'ae end' acknowledgement; compact stops the session itself."
                        .to_owned(),
                );
            }
            flag if flag.starts_with("--from") => {
                return Err(
                    "Error: --from is not a compact flag — compact inherits from the archive it just wrote."
                        .to_owned(),
                );
            }
            flag @ ("--local" | "--copy" | "--worktree") => {
                return Err(format!(
                    "Error: '{flag}' is not a compact flag — the fresh session keeps the mode the archived one had."
                ));
            }
            "all" => {
                return Err(
                    "Error: compact takes one session name; 'all' has no meaning for it."
                        .to_owned(),
                );
            }
            "use" => {
                return Err(
                    "Error: 'use' is not a compact argument — the fresh session starts the roster your config names now."
                        .to_owned(),
                );
            }
            flag if flag.starts_with('-') => {
                return Err(format!("Error: unknown flag '{flag}'.\n{USAGE}"));
            }
            name if args.name.is_empty() => name.clone_into(&mut args.name),
            extra => {
                return Err(format!(
                    "Error: unexpected extra argument '{extra}' — compact takes one session name."
                ));
            }
        }
    }
    if args.name.is_empty() {
        return Err(USAGE.to_owned());
    }
    Ok(args)
}

/// The fields of the frozen tuple this ORCHESTRATION reads. The destructive
/// gates parse their own copy from the same line — this is the presentation and
/// relaunch half, not a second authority.
struct Frozen {
    name: String,
    uuid: String,
    uuid_origin: String,
    mode: String,
    origin: String,
    config: String,
    purge: bool,
    archive: String,
    main_ref: String,
    /// `main=<alias> workers=<a,b|->` — the roster the fresh session was
    /// PROMISED to start, as shown at the prompt.
    roster: String,
}

impl Frozen {
    fn parse(line: &str) -> Option<Self> {
        let fields: Vec<&str> = line.split('\u{1f}').collect();
        let [
            name,
            uuid,
            uuid_origin,
            mode,
            origin,
            config,
            purge,
            archive,
            main_ref,
            roster,
        ] = fields.as_slice()
        else {
            return None;
        };
        Some(Self {
            name: (*name).to_owned(),
            uuid: (*uuid).to_owned(),
            uuid_origin: (*uuid_origin).to_owned(),
            mode: (*mode).to_owned(),
            origin: (*origin).to_owned(),
            config: (*config).to_owned(),
            purge: *purge == "true",
            archive: (*archive).to_owned(),
            main_ref: (*main_ref).to_owned(),
            roster: (*roster).to_owned(),
        })
    }
}

/// The confirmation body — the frozen `_compact_confirm_body`, on stderr.
fn confirm_body(frozen: &Frozen, digest_only: bool, err: &mut impl Write) -> io::Result<()> {
    writeln!(err, "Compact session '{}'?", frozen.name)?;
    writeln!(
        err,
        "  - Session id: {} ({}), mode: {}",
        frozen.uuid, frozen.uuid_origin, frozen.mode
    )?;
    writeln!(err, "  - Origin:     {}", frozen.origin)?;
    writeln!(
        err,
        "  - ARCHIVES its memory to {}/, then ENDS it.",
        frozen.archive
    )?;
    writeln!(
        err,
        "  - Then starts a FRESH session '{}' that inherits that archive's digest.",
        frozen.name
    )?;
    writeln!(
        err,
        "  - The fresh session starts the roster your config names now: {}",
        frozen.roster
    )?;
    writeln!(
        err,
        "  - NOT carried over: provider conversations, panes, spawned agents, launch scratch."
    )?;
    writeln!(
        err,
        "  - KEPT: the archive, and the claude/codex conversation files on disk."
    )?;
    if digest_only {
        writeln!(
            err,
            "  - No handover will be requested (--digest-only); the digest is the handover."
        )?;
    } else {
        writeln!(
            err,
            "  - Your main agent will be asked to hand over first, and must reply AND write a handover memo."
        )?;
    }
    Ok(())
}

/// The single command a human can paste to redo the relaunch by hand — the
/// frozen `_compact_recovery_command`, minus the environment prefixes bash
/// derived from its own `AE_HOME`/`HOME`.
fn recovery_command(frozen: &Frozen) -> String {
    let mode_flag = match frozen.mode.as_str() {
        "git" => "--worktree",
        "full" => "--copy",
        _ => "--local",
    };
    format!(
        "cd {} && ae {mode_flag} {} --from {}",
        shell_quote(&frozen.origin),
        shell_quote(&frozen.name),
        frozen.uuid
    )
}

/// Single-quote a word for a shell, the frozen `shell_quote`.
fn shell_quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

/// The ref of the still-pending handover request this session's compact actor
/// opened, or empty.
fn outstanding(dir: &Path) -> io::Result<String> {
    let mut found = Vec::new();
    compact::find_outstanding_step(dir, &mut found)?;
    Ok(String::from_utf8_lossy(&found).trim().to_owned())
}

/// The handover budget — `AE_COMPACT_HANDOVER_SECS` when it is a positive
/// number of seconds, else the frozen 300.
fn handover_secs(err: &mut impl Write) -> io::Result<u64> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the frozen AE_COMPACT_HANDOVER_SECS knob — see clippy.toml"
    )]
    let raw = std::env::var_os("AE_COMPACT_HANDOVER_SECS");
    let Some(raw) = raw.filter(|value| !value.is_empty()) else {
        return Ok(DEFAULT_HANDOVER_SECS);
    };
    let text = raw.to_string_lossy().into_owned();
    match text.parse::<u64>() {
        Ok(secs) if secs > 0 => Ok(secs),
        _ => {
            writeln!(
                err,
                "note: AE_COMPACT_HANDOVER_SECS='{text}' is not a positive number of seconds — using {DEFAULT_HANDOVER_SECS}."
            )?;
            Ok(DEFAULT_HANDOVER_SECS)
        }
    }
}

/// The handover request body — the frozen `_compact_request_text`.
fn request_text(dir: &Path, baseline: &str) -> String {
    format!(
        "COMPACT HANDOVER — this session is about to be archived and restarted fresh from its\n\
         digest. You are its main agent; what you write now is what the next session begins with.\n\
         \n\
         1. Stop accepting new work.\n\
         2. Quiesce your configured workers and consume their results.\n\
         3. Persist the load-bearing continuation with:\n\
         \x20      {}/memo add --topic handover \"<what the next session must know>\"\n\
         \x20  Write what cannot be re-derived from the code: decisions and their reasons, what was\n\
         \x20  ruled out, what is in flight, what will bite. Not a summary of the diff.\n\
         4. Retire every agent you spawned. compact refuses while any spawned slot remains, and it\n\
         \x20  will not retire them for you.\n\
         5. Reply with the exact command in this message.\n\
         6. Do no further work after replying.\n\
         \n\
         AE-COMPACT-MEMO-BASELINE={baseline}\n",
        dir.display(),
    )
}

/// Publish the exec plan — temp, then rename, so a caller can never read a
/// half-written plan and relaunch from it.
fn write_plan(path: &Path, record: &str) -> io::Result<()> {
    let temp = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&temp, format!("{record}\n"))?;
    std::fs::rename(&temp, path)
}

/// Randomness for the tracked request's id. Mirrors the crate's one source.
fn entropy() -> u64 {
    use std::hash::{BuildHasher as _, RandomState};
    RandomState::new().hash_one(std::process::id())
}

/// Read the confirmation. A LINE, not a raw keystroke — see
/// [`super::end`]'s `read_reply` for why. `None` is EOF, which is never
/// consent.
fn read_reply() -> Option<String> {
    let mut buffer = String::new();
    // A LINE, not to EOF: a human at a terminal answers and presses Enter, and
    // reading to EOF there would block forever waiting for a ^D they were never
    // asked for. An empty read and a failed read are the same answer — nobody
    // replied.
    match std::io::stdin().read_line(&mut buffer) {
        Ok(read) if read > 0 => Some(buffer.trim_start().to_owned()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Frozen, recovery_command, shell_quote};

    fn tuple() -> String {
        [
            "proj",
            "11111111-2222-3333-4444-555555555555",
            "session",
            "local",
            "/o",
            "/c",
            "false",
            "/a/archive/11111111-2222-3333-4444-555555555555",
            "lead",
            "main=cl workers=-",
        ]
        .join("\u{1f}")
    }

    #[test]
    fn the_frozen_tuple_round_trips_all_ten_fields() {
        let frozen = Frozen::parse(&tuple()).expect("ten fields parse");
        assert_eq!(frozen.name, "proj");
        assert_eq!(frozen.mode, "local");
        assert_eq!(frozen.main_ref, "lead");
        assert_eq!(frozen.roster, "main=cl workers=-");
        assert!(!frozen.purge);
        assert!(Frozen::parse("too\u{1f}few").is_none());
    }

    #[test]
    fn the_recovery_command_names_the_mode_and_the_archive() {
        let frozen = Frozen::parse(&tuple()).expect("ten fields parse");
        assert_eq!(
            recovery_command(&frozen),
            "cd '/o' && ae --local 'proj' --from 11111111-2222-3333-4444-555555555555"
        );
    }

    #[test]
    fn a_quote_in_a_word_cannot_end_the_quoting() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }
}
