//! `_archive-publish <dir> <push-outcome> <push-ref> <preserved> <workdir> <archived-at>`
//! — the archive publisher and its `.publishing.<uuid>` claim (P3.3).
//!
//! The bash `_end_archive_step` boundary runs this in place of the frozen
//! `_ar_publish`, then reads back the one line this prints — `target\tfiles\tbytes`
//! — and reports "Archived …". `AE_CORE=` falls back to the frozen body. Plan,
//! mint, purge and teardown stay in bash; this core only PUBLISHES.
//!
//! # What is reference and what is Rust-native
//!
//! The frozen `_ar_publish`/`_ar_stage_payload`/`_ar_validate_tree` are REFERENCE
//! EVIDENCE for the externally meaningful surface — the payload file set, the
//! `key=value` archive meta, the digest bytes, the `0700`/`0600` modes, and the
//! `target\tfiles\tbytes` diagnostic a bash consumer reads. This module preserves
//! those. It does NOT reproduce the frozen weaknesses; the P3.3 assignment is
//! authoritative on the safety properties, and where they conflict with bash the
//! safer Rust behaviour wins and the divergence is stated:
//!
//! * **Coherent locked snapshot.** meta, memo.tsv and events.jsonl are read
//!   while holding their own `.lock` siblings — `state::acquire`, `flock(2)`, the
//!   Bash-compatible lock the writers take — acquired in the FIXED order
//!   `meta.lock -> memo.tsv.lock -> events.jsonl.lock` and held through the copy.
//!   Current writers hold one at a time, so this adds no inversion. The caller's
//!   lifecycle lock and a before/after fingerprint are defence in depth. Taking a
//!   lock may CREATE its `.lock` file — that is lock infrastructure, not source
//!   data; the live meta/memo/events/messages bytes are never written.
//! * **Durable publication.** Every staged file is `fsync`ed, then the messages
//!   and payload directories, then the payload is `rename`d onto the target, then
//!   the archive root is `fsync`ed — so a crash after the rename cannot lose the
//!   archive. The frozen writer syncs nothing.
//! * **Classified-source refusal.** An existing non-regular meta/memo.tsv/
//!   events.jsonl (symlink, FIFO, directory) is a NAMED `rc=1` refusal, never
//!   followed — the P3.1/P3.2 preview rule. An absent optional memo/events is the
//!   defined empty file. A DIRECT `messages/*.txt` that is a symlink or other
//!   non-regular node is SKIPPED with a loud diagnostic (never followed, never
//!   recursed); an eligible regular one that cannot be read REFUSES the whole
//!   publish rather than archive a digest that references a body that is not
//!   there.
//! * **Atomic claim, standing on crash.** `mkdir` of `.publishing.<uuid>` is the
//!   atomic primitive: it fails if another publisher holds it OR a previous run
//!   crashed holding it, and this module NEVER guess-cleans someone else's claim —
//!   a crash leaves it standing and loud. A failure THIS invocation handles
//!   removes its OWN claim and leaves the source untouched.
//!
//! Nothing here deletes or mutates the live session: every write lands under the
//! archive root, and the only session-directory writes are the `.lock` files the
//! locks stand on.

use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::request_states::request_states;
use super::store::{
    archive_root_of, claim_path, count_tree, exists, fsync_dir, make_claim, messages_fingerprint,
    mkdir_0700, read_dir, read_file, require_real_root, symlink_meta, validate_tree,
    write_file_0600,
};
use super::{
    GitFacts, Sources, Volatiles, canonical_uuid, event_facts, fingerprint, memo_rows, memo_topics,
    meta_get, or_dash, roster, roster_slots,
};
use crate::state::EXIT_FAILED;

/// The Bash-compatible `flock -w 5` wait the writers use.
const LOCK_WAIT: Duration = Duration::from_secs(5);

/// The operation facts Bash owns and hands the publisher — everything the core
/// does NOT derive itself. The commit facts are absent on purpose: the core
/// derives base/final/range/count from `push_outcome` and `workdir` with P3.2
/// typed Git, rather than trust a caller to compute them.
pub(crate) struct Ops<'a> {
    pub(crate) push_outcome: &'a str,
    pub(crate) push_ref: &'a str,
    pub(crate) preserved: &'a str,
    pub(crate) workdir: &'a str,
    pub(crate) archived_at: &'a str,
}

/// Acquire the three source locks in the one fixed global order and return the
/// held handles (released on drop) — or a diagnostic naming the lock that could
/// not be taken. Holding all three through the read is the coherent snapshot.
fn hold_sources(dir: &Path) -> Result<[fs::File; 3], String> {
    let meta = crate::state::acquire(&dir.join("meta.lock"), LOCK_WAIT)
        .map_err(|why| format!("archive: could not lock meta.lock: {why}"))?;
    let memo = crate::state::acquire(&dir.join("memo.tsv.lock"), LOCK_WAIT)
        .map_err(|why| format!("archive: could not lock memo.tsv.lock: {why}"))?;
    let events = crate::state::acquire(&dir.join("events.jsonl.lock"), LOCK_WAIT)
        .map_err(|why| format!("archive: could not lock events.jsonl.lock: {why}"))?;
    Ok([meta, memo, events])
}

/// `_archive-publish` core entry. Returns the process exit code: `0` after a
/// published archive whose `target\tfiles\tbytes` line is on `out`, or
/// [`EXIT_FAILED`] after a named refusal on `err`. Never touches live source
/// data — only the archive root and the source `.lock` files.
pub(crate) fn run(
    dir: &Path,
    ops: &Ops,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    // The timestamp is Bash's (std cannot format one without a time dependency),
    // but argv is not trusted merely because the normal caller is Bash: the exact
    // frozen UTC shape is validated before it reaches an immutable archive.
    if !is_archived_at(ops.archived_at) {
        writeln!(
            err,
            "archive: archived-at '{}' is not an ISO-8601 UTC instant (YYYY-MM-DDTHH:MM:SSZ).",
            ops.archived_at
        )?;
        return Ok(EXIT_FAILED);
    }

    // The coherent snapshot: hold all three source locks, in the one fixed global
    // order, through every read below. `_held` releases them on drop.
    let _held = match hold_sources(dir) {
        Ok(handles) => handles,
        Err(diag) => {
            writeln!(err, "{diag}")?;
            return Ok(EXIT_FAILED);
        }
    };

    // Classified-source refusal, under the locks, BEFORE any read: an existing
    // non-regular core source (symlink, FIFO, directory) fails the whole publish
    // and is NEVER opened. This runs first precisely so a FIFO cannot block the
    // read while three locks are held, and a symlinked source is never followed —
    // the diagnostic name is read only after meta is a confirmed regular file.
    for file in ["meta", "memo.tsv", "events.jsonl"] {
        if super::nonregular_existing(&dir.join(file)) {
            writeln!(err, "archive: a non-regular {file} cannot be archived.")?;
            return Ok(EXIT_FAILED);
        }
    }

    // The coherent read, now that every source is a confirmed regular file. An
    // ABSENT optional memo/events is the defined empty content; an existing-but-
    // unreadable regular source REFUSES — an immutable ledger must never publish
    // with its evidence silently dropped. Fingerprint before/after as defence in
    // depth against a writer that somehow bypassed the locks (it cannot, held).
    let before = fingerprint(dir);
    let (meta_bytes, memo_bytes, event_bytes) = match read_sources(dir) {
        Ok(triple) => triple,
        Err(diags) => {
            for diag in diags {
                writeln!(err, "{diag}")?;
            }
            return Ok(EXIT_FAILED);
        }
    };
    let name = {
        let n = meta_get(&meta_bytes, "session");
        if n.is_empty() { "?".to_owned() } else { n }
    };

    let raw_id = meta_get(&meta_bytes, "session_id");
    let aid = canonical_uuid(&raw_id);
    if aid.is_empty() {
        writeln!(
            err,
            "archive: session '{name}' has no UUID session_id ('{raw_id}') — it cannot be archived."
        )?;
        return Ok(EXIT_FAILED);
    }
    let id_origin = {
        let o = meta_get(&meta_bytes, "session_id_origin");
        if o.is_empty() {
            "session".to_owned()
        } else {
            o
        }
    };

    // A roster ae cannot parse is a FAILED archive, never a partial one — the same
    // refusal the digest and meta both make, computed once and shared.
    let roster = match roster(&meta_bytes) {
        Ok(pairs) => pairs,
        Err(line) => {
            writeln!(err, "{line}")?;
            writeln!(err, "archive: could not archive '{name}'.")?;
            return Ok(EXIT_FAILED);
        }
    };

    let vol = Volatiles {
        snapshot: "archived",
        archived_at: ops.archived_at.to_owned(),
        git: derive_git(&meta_bytes, ops.push_outcome, ops.workdir.as_bytes()),
        push_outcome: ops.push_outcome.to_owned(),
        push_ref: ops.push_ref.to_owned(),
        preserved: ops.preserved.to_owned(),
    };

    let Some(root) = archive_root_of(dir) else {
        writeln!(err, "archive: '{}' has no archive root.", dir.display())?;
        return Ok(EXIT_FAILED);
    };
    if let Err(line) = require_real_root(&root, true) {
        writeln!(err, "{line}")?;
        return Ok(EXIT_FAILED);
    }
    let target = root.join(&aid);
    if exists(&target) {
        writeln!(
            err,
            "archive: {} already exists — an archive is immutable and is never merged into.",
            target.display()
        )?;
        return Ok(EXIT_FAILED);
    }

    let facts = StagedFacts {
        meta_bytes: &meta_bytes,
        memo_bytes: &memo_bytes,
        event_bytes: &event_bytes,
        aid: &aid,
        id_origin: &id_origin,
        roster: &roster,
        vol: &vol,
    };
    publish_under_claim(&root, &target, dir, &facts, &before, out, err)
}

/// Under our atomic claim, stage → validate → durably publish, then report the
/// `target\tfiles\tbytes` line a bash consumer reads. The claim `mkdir` is the
/// mutual-exclusion primitive: it fails if another publisher holds the claim or a
/// crash left it standing, and this NEVER guess-cleans someone else's. The
/// `rename` onto the target is the COMMIT BOUNDARY: a failure before it removes
/// our OWN claim and leaves the source untouched; a failure after it — the
/// archive is already present — never reports success and RETAINS the claim as a
/// recovery marker, because a retry would otherwise only see an existing target.
fn publish_under_claim(
    root: &Path,
    target: &Path,
    dir: &Path,
    facts: &StagedFacts,
    before: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let claim = claim_path(root, facts.aid);
    if let Err(why) = make_claim(&claim) {
        writeln!(
            err,
            "archive: another publisher holds {} (or a previous run crashed holding it): {why}",
            claim.display()
        )?;
        writeln!(
            err,
            "  ae will not guess-clean it. Inspect it, then remove it by hand if it is stale."
        )?;
        return Ok(EXIT_FAILED);
    }
    // Make the claim entry durable, so a crash mid-staging leaves a standing
    // marker rather than vanishing — best effort; a lost claim costs the marker,
    // never data.
    let _ = fsync_dir(root);

    // Everything up to (not including) the rename is a PRE-rename failure: the
    // source is untouched and nothing is published, so remove our own claim.
    if let Err(diag) = stage_and_validate(&claim, target, dir, facts, before, err) {
        if let Err(why) = fs::remove_dir_all(&claim) {
            writeln!(
                err,
                "archive: additionally, ae could not remove its own claim {} ({why}); remove it by hand before retrying.",
                claim.display()
            )?;
        }
        writeln!(err, "{diag}")?;
        return Ok(EXIT_FAILED);
    }

    // The commit boundary. The rename itself failing is still pre-rename in
    // effect (the target was not created): clean our claim and refuse.
    let payload = claim.join("payload");
    if let Err(why) = fs::rename(&payload, target) {
        if let Err(e) = fs::remove_dir_all(&claim) {
            writeln!(
                err,
                "archive: additionally, ae could not remove its own claim {} ({e}).",
                claim.display()
            )?;
        }
        writeln!(
            err,
            "archive: could not publish {}: {why}",
            target.display()
        )?;
        return Ok(EXIT_FAILED);
    }

    // Past the point of no return: the archive IS present at `target`. A failure
    // now is a DURABILITY-confirmation failure, not a publish failure — never
    // report success, and RETAIN the claim as the recovery marker a retry needs.
    if let Err(why) = fsync_dir(root) {
        writeln!(
            err,
            "archive: {} was renamed into place but its durability could not be confirmed (fsync archive root: {why}).",
            target.display()
        )?;
        writeln!(
            err,
            "  The claim {} is RETAINED as a recovery marker; the session is STOPPED and nothing was deleted. Verify the archive, then remove the claim by hand.",
            claim.display()
        )?;
        return Ok(EXIT_FAILED);
    }

    // Durable. Persist the claim removal too (fsync the root after it), so a
    // completed publish does not leave a stale claim behind on a later crash.
    if let Err(why) = fs::remove_dir(&claim) {
        writeln!(
            err,
            "archive: published {}, but could not remove the completed claim {} ({why}); it is harmless — remove it by hand.",
            target.display(),
            claim.display()
        )?;
    } else {
        let _ = fsync_dir(root);
    }

    let (files, bytes) = count_tree(target);
    writeln!(out, "{}\t{files}\t{bytes}", target.display())?;
    Ok(0)
}

/// The already-read, already-validated facts staging needs, grouped so the
/// staging entry stays under clippy's argument limit.
struct StagedFacts<'a> {
    meta_bytes: &'a [u8],
    memo_bytes: &'a [u8],
    event_bytes: &'a [u8],
    aid: &'a str,
    id_origin: &'a str,
    roster: &'a [(String, String)],
    vol: &'a Volatiles,
}

/// Stage the payload under the held claim and validate it — up to but NOT
/// including the rename (the caller owns the commit boundary). `Ok(())` when the
/// payload is staged, validated and fsynced, ready to rename; `Err(diag)` on any
/// PRE-rename failure. The live source is never written.
fn stage_and_validate(
    claim: &Path,
    target: &Path,
    dir: &Path,
    facts: &StagedFacts,
    before: &str,
    err: &mut impl Write,
) -> Result<(), String> {
    let payload = claim.join("payload");
    let messages_dst = payload.join("messages");
    mkdir_0700(&payload).map_err(|why| format!("archive: could not stage payload: {why}"))?;
    mkdir_0700(&messages_dst).map_err(|why| format!("archive: could not stage messages: {why}"))?;

    // meta and the two ledgers come from the coherent snapshot; the meta is the
    // composed archive meta, not the live one.
    let meta_out = compose_meta(facts);
    write_file_0600(&payload.join("meta"), meta_out.as_bytes())
        .map_err(|why| format!("archive: could not write meta: {why}"))?;
    write_file_0600(&payload.join("memo.tsv"), facts.memo_bytes)
        .map_err(|why| format!("archive: could not write memo.tsv: {why}"))?;
    write_file_0600(&payload.join("events.jsonl"), facts.event_bytes)
        .map_err(|why| format!("archive: could not write events.jsonl: {why}"))?;

    // Messages: NEVER follow a symlinked or non-directory messages/ root. A real
    // directory is required to enumerate; anything else is skipped loud (the
    // archive gets an empty messages/, and any referenced body then renders
    // 'Payload: unavailable' from the staged set). Within a real directory the
    // per-entry skip/refuse rules apply, and the directory is fingerprinted
    // around the copy so a mid-publish change refuses rather than mix generations.
    let messages_src = dir.join("messages");
    let messages_real = matches!(symlink_meta(&messages_src), Ok(m) if m.is_dir());
    if !messages_real && exists(&messages_src) {
        let _ = writeln!(
            err,
            "archive: skipping messages/ — it is a symlink or not a directory, and an archive never follows one."
        );
    }
    if messages_real {
        let before_msgs = messages_fingerprint(&messages_src);
        stage_messages(&messages_src, &messages_dst, err)?;
        if messages_fingerprint(&messages_src) != before_msgs {
            return Err(format!(
                "archive: session '{}' messages changed while publishing; retry.",
                meta_get(facts.meta_bytes, "session")
            ));
        }
    }

    // The digest is rendered against the STAGED messages, so a skipped body reads
    // 'Payload: unavailable' and no dangling messages/<base> link is ever named.
    let digest = super::render(
        &Sources {
            meta: facts.meta_bytes,
            memo: facts.memo_bytes,
            events: facts.event_bytes,
        },
        &messages_dst,
        facts.aid,
        &event_facts(facts.event_bytes),
        facts.roster,
        facts.vol,
    );
    write_file_0600(&payload.join("digest.md"), digest.as_bytes())
        .map_err(|why| format!("archive: could not write digest.md: {why}"))?;

    // Defence in depth: the locked sources must not have moved under us.
    if fingerprint(dir) != before {
        return Err(format!(
            "archive: session '{}' changed while publishing; retry.",
            meta_get(facts.meta_bytes, "session")
        ));
    }

    validate_tree(&payload, facts.aid)?;

    // TOCTOU: refuse if the target appeared while we staged.
    if exists(target) {
        return Err(format!(
            "archive: {} appeared while staging — refusing to overwrite it.",
            target.display()
        ));
    }

    // Durable staging: fsync the files' directories before the caller renames, so
    // the payload is wholly present after a crash, never a torn tree.
    fsync_dir(&messages_dst).map_err(|why| format!("archive: fsync messages: {why}"))?;
    fsync_dir(&payload).map_err(|why| format!("archive: fsync payload: {why}"))?;
    Ok(())
}

/// Compose the archive `meta` file — the frozen `_ar_build_meta` key order and
/// values, from the coherent snapshot and the shared, already-validated facts.
/// Rendered from the SAME helpers the digest uses, so the two agree by
/// construction and the validator's cross-check cannot fail here.
fn compose_meta(facts: &StagedFacts) -> String {
    let meta = facts.meta_bytes;
    let g = |key: &str| meta_get(meta, key);
    let ev = event_facts(facts.event_bytes);
    let rows = memo_rows(facts.memo_bytes);
    let handover = rows.iter().filter(|r| r.topic == "handover").count();
    let topics = memo_topics(&rows).len();
    let pending = request_states(facts.event_bytes)
        .iter()
        .filter(|r| r.status == "pending")
        .count();

    let mut out = String::new();
    let _ = writeln!(out, "archive_version=1");
    let _ = writeln!(out, "archive_id={}", facts.aid);
    let _ = writeln!(out, "archive_id_origin={}", facts.id_origin);
    let _ = writeln!(out, "archived_at={}", facts.vol.archived_at);
    let _ = writeln!(out, "source_session={}", g("session"));
    let source_id = if facts.id_origin == "session" {
        g("session_id")
    } else {
        "-".to_owned()
    };
    let _ = writeln!(out, "source_session_id={source_id}");
    let _ = writeln!(out, "source_mode={}", g("mode"));
    let _ = writeln!(out, "source_origin={}", g("origin"));
    let _ = writeln!(out, "source_layout={}", g("layout"));
    let _ = writeln!(out, "source_ae_version={}", or_dash(&g("ae_version")));
    let _ = writeln!(out, "source_goal={}", g("goal"));
    let _ = writeln!(
        out,
        "parent_archive_id={}",
        or_dash(&g("parent_archive_id"))
    );
    let _ = writeln!(out, "git_base_commit={}", facts.vol.git.base);
    let _ = writeln!(out, "git_final_commit={}", facts.vol.git.final_commit);
    let _ = writeln!(out, "git_commit_range={}", facts.vol.git.range);
    let _ = writeln!(out, "git_commit_count={}", facts.vol.git.count);
    let _ = writeln!(out, "git_push_outcome={}", facts.vol.push_outcome);
    let _ = writeln!(out, "git_push_ref={}", facts.vol.push_ref);
    let _ = writeln!(out, "preserved_work_dir={}", facts.vol.preserved);
    let _ = writeln!(out, "event_count={}", ev.count);
    let _ = writeln!(out, "event_first_at={}", or_dash(&ev.first));
    let _ = writeln!(out, "event_last_at={}", or_dash(&ev.last));
    let _ = writeln!(out, "handover_count={handover}");
    let _ = writeln!(out, "memo_topic_count={topics}");
    let _ = writeln!(out, "pending_request_count={pending}");
    // The roster is already parsed and stripped to alias:name; agents first, then
    // the per-slot binaries — the frozen two-pass order.
    for (slot, refname) in facts.roster {
        let _ = writeln!(out, "agent.{slot}={refname}");
    }
    for slot in roster_slots(meta) {
        let bin = meta_get(meta, &format!("agent_bin.{slot}"));
        if !bin.is_empty() {
            let _ = writeln!(out, "agent_bin.{slot}={bin}");
        }
    }
    out
}

/// `_ar_git_head`/`_ar_git_range` as `_end_archive_step` drives them for a
/// PUBLISH: nothing when the run is not git-managed; otherwise the base recorded
/// at launch and the passed work dir's HEAD and range. Keyed on the push outcome
/// and the PASSED work dir — not the meta's, and not the mode as a preview keys
/// it.
fn derive_git(meta_bytes: &[u8], push_outcome: &str, workdir: &[u8]) -> GitFacts {
    if push_outcome == "not-managed" {
        return GitFacts {
            base: "-".to_owned(),
            final_commit: "-".to_owned(),
            range: "-".to_owned(),
            count: "-".to_owned(),
        };
    }
    let base = {
        let b = meta_get(meta_bytes, "git_base_commit");
        if b.is_empty() { "-".to_owned() } else { b }
    };
    let final_commit = crate::git::head(workdir);
    let (range, count) = crate::git::range(workdir, &base, &final_commit);
    GitFacts {
        base,
        final_commit,
        range,
        count,
    }
}

/// Whether `s` is EXACTLY the frozen archive instant `YYYY-MM-DDTHH:MM:SSZ`:
/// 20 ASCII bytes, digits in the field positions, the fixed separators, and
/// cheap in-range field values (a control byte, a newline, or a wrong width is
/// refused). Not a full calendar validity check — leap days pass — but no
/// interpreted or truncating input reaches an immutable archive's metadata.
fn is_archived_at(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 20 {
        return false;
    }
    for (i, &c) in b.iter().enumerate() {
        let ok = match i {
            4 | 7 => c == b'-',
            10 => c == b'T',
            13 | 16 => c == b':',
            19 => c == b'Z',
            _ => c.is_ascii_digit(),
        };
        if !ok {
            return false;
        }
    }
    let field = |from: usize, to: usize| -> u32 {
        s[from..to]
            .bytes()
            .fold(0, |acc, c| acc * 10 + u32::from(c - b'0'))
    };
    let (month, day) = (field(5, 7), field(8, 10));
    let (hour, min, sec) = (field(11, 13), field(14, 16), field(17, 19));
    (1..=12).contains(&month) && (1..=31).contains(&day) && hour <= 23 && min <= 59 && sec <= 60
}

/// `meta`'s value for `key` read directly from a path (only used before the
/// coherent snapshot exists, for the session name in a refusal message).
/// Read a core source from the coherent snapshot. An ABSENT optional file is the
/// defined empty content (a session that sent no memo, emitted no events); an
/// existing-but-unreadable regular file REFUSES the whole publish rather than
/// publish an immutable ledger with its evidence silently dropped to empty.
fn read_source(path: &Path) -> Result<Vec<u8>, String> {
    match read_file(path) {
        Ok(bytes) => Ok(bytes),
        Err(why) if why.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(why) => Err(format!(
            "archive: cannot read {} — refusing to publish an archive with its evidence dropped: {why}",
            path.display()
        )),
    }
}

/// The three core source blobs — meta, memo.tsv, events.jsonl — from the
/// coherent snapshot.
type CoreBytes = (Vec<u8>, Vec<u8>, Vec<u8>);

/// Read the three core sources from the coherent snapshot. `Ok` with the triple,
/// or `Err` with every read diagnostic — an existing-but-unreadable regular
/// source refuses, an absent optional one is empty (see [`read_source`]).
fn read_sources(dir: &Path) -> Result<CoreBytes, Vec<String>> {
    match (
        read_source(&dir.join("meta")),
        read_source(&dir.join("memo.tsv")),
        read_source(&dir.join("events.jsonl")),
    ) {
        (Ok(m), Ok(mo), Ok(e)) => Ok((m, mo, e)),
        (m, mo, e) => Err([m, mo, e].into_iter().filter_map(Result::err).collect()),
    }
}

/// Copy the DIRECT, regular, non-symlink `messages/*.txt` into `dst` at `0600`.
/// A symlink or other non-regular entry is skipped with a loud diagnostic and
/// NEVER followed; an eligible regular file that cannot be read refuses the whole
/// publish (a digest that names a body must not archive without it).
fn stage_messages(src: &Path, dst: &Path, err: &mut impl Write) -> Result<(), String> {
    let mut entries: Vec<PathBuf> = match read_dir(src) {
        Ok(paths) => paths,
        // The caller reaches here only for a REAL directory. A genuinely absent
        // one (NotFound) is the empty session — a classified absence. Any other
        // failure to enumerate it (an unreadable dir) is UNKNOWN LOSS: refuse
        // rather than publish an archive that may be missing message bodies.
        Err(why) if why.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(why) => {
            return Err(format!(
                "archive: cannot enumerate messages/ ({why}) — refusing rather than publish an archive that may be missing message bodies."
            ));
        }
    };
    entries.sort();
    for path in entries {
        if path.extension().is_none_or(|e| e != "txt") {
            continue;
        }
        let base = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Ok(meta) = symlink_meta(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            let _ = writeln!(
                err,
                "archive: skipping messages/{base} — it is a symlink, and an archive never follows one."
            );
            continue;
        }
        if !meta.is_file() {
            let _ = writeln!(
                err,
                "archive: skipping messages/{base} — not a regular file."
            );
            continue;
        }
        let bytes = read_file(&path).map_err(|_| {
            format!(
                "archive: cannot read messages/{base} — refusing to publish an archive missing a payload it references."
            )
        })?;
        write_file_0600(&dst.join(&base), &bytes)
            .map_err(|why| format!("archive: could not stage messages/{base}: {why}"))?;
    }
    Ok(())
}
