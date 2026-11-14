//! The launch record: the meta document a launch writes and the session-id seed.

use super::{
    Env, FromProof, Launching, Mode, Path, ServerId, Session, WatchdogFacts, launch, meta,
    meta_value, path_bytes, roster, transport,
};

/// Build the WHOLE initial meta — base facts, then the v2 roster block.
///
/// # Errors
///
/// The refusal text when a resolved value carries a byte that would corrupt a
/// record.
#[allow(
    clippy::too_many_lines,
    reason = "the frozen meta, built in one place so the document has one author"
)]
pub(super) fn meta_document(
    env: &Env,
    shape: &Session,
    launching: &[Launching],
    watchdog: WatchdogFacts<'_>,
    parent: Option<&FromProof>,
) -> Result<String, String> {
    let dir = env.sessions().join(&shape.name);
    let main_pane = launching
        .first()
        .map(|agent| agent.pane.clone())
        .unwrap_or_default();
    // Launch facts that must survive every rewrite.
    let preserved = |key: &str| meta_value(&dir, key).filter(|v| !v.is_empty());
    // A row named twice says nothing; never copy one forward.
    let sole_preserved = |key: &str| {
        meta::sole_value(&meta::read_bytes(&dir).ok()?, key)
            .map(|value| String::from_utf8_lossy(value).into_owned())
            .filter(|value| !value.is_empty())
    };
    let started = crate::time::Timestamp::now().epoch().to_string();
    let created = if shape.resuming {
        preserved("created")
            .or_else(|| crate::session::legacy_created_epoch(&dir).map(|epoch| epoch.to_string()))
            .or_else(|| {
                preserved("launch_time.main")
                    .filter(|epoch| epoch.parse::<i64>().is_ok_and(i64::is_positive))
            })
    } else {
        Some(started.clone())
    };
    let session_id = preserved("session_id").unwrap_or_else(launch::generate_uuid);
    let git_base = preserved("git_base_commit").or_else(|| {
        (!shape.resuming && shape.mode != Mode::Local)
            .then(|| crate::git::head(path_bytes(&shape.work_dir)))
            .filter(|head| head != "-")
    });
    let (parent_id, handover, pending) = match parent {
        Some(proof) => (
            Some(proof.id.clone()),
            Some(proof.handover.clone()),
            Some(proof.pending.clone()),
        ),
        None => (
            preserved("parent_archive_id"),
            preserved("parent_archive_handover_count"),
            preserved("parent_archive_pending_count"),
        ),
    };

    let mut body = String::new();
    let mut row = |key: &str, value: &str| {
        body.push_str(key);
        body.push('=');
        body.push_str(value);
        body.push('\n');
    };
    // The SHAPE row, first: it says how everything below it is to be read, and
    // its absence is what tells a later ae that this meta pre-dates the chain.
    row(crate::migrate::KEY, &crate::migrate::CURRENT.to_string());
    row("mode", shape.mode.as_str());
    row("origin", &shape.origin.display().to_string());
    row("session", &shape.name);
    row("session_id", &session_id);
    if let Some(created) = created {
        row("created", &created);
    }
    row("started", &started);
    row("work_dir", &shape.work_dir.display().to_string());
    row("layout", &shape.layout);
    row(
        "config",
        &env.global
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
    );
    let origin_local = shape.origin.join(".ae").join("config");
    if let Some(local) = env.local.as_ref().filter(|local| *local != &origin_local) {
        row(
            crate::config::LOCAL_CONFIG_KEY,
            &local.display().to_string(),
        );
    }
    row("main_pane", &main_pane);
    row("ae_version", crate::shape::VERSION);
    if let Some(core) = crate::shape::resolved_exe() {
        // The core binding, pinned per session as a PAIR: a helper that found a
        // binary whose version disagreed with the session's would be running a
        // different contract than the one this session was built against.
        let ae_core = env.core.as_ref().unwrap_or(&core);
        row("ae_core", &ae_core.display().to_string());
        row(
            "ae_core_version",
            env.core_version.as_deref().unwrap_or(crate::shape::VERSION),
        );
    }
    row("tmux_server", &env.server_value);
    row("tmux_server_kind", &env.server_kind);
    for (key, value) in [
        (
            "watchdog",
            preserved("watchdog").or_else(|| preserved("loop")),
        ),
        ("goal", preserved("goal")),
        ("git_base_commit", git_base),
        ("session_id_origin", preserved("session_id_origin")),
    ] {
        if let Some(value) = value {
            row(key, &value);
        }
    }
    row("quota_every_secs", watchdog.quota_every_secs);
    row("idle_nudge_secs", watchdog.idle_nudge_secs);
    row("done_confirmations", watchdog.done_confirmations);
    row("quota", watchdog.quota);
    if let Some(id) = parent_id {
        row("parent_archive_id", &id);
        row(
            "parent_archive_handover_count",
            handover.as_deref().unwrap_or("0"),
        );
        row(
            "parent_archive_pending_count",
            pending.as_deref().unwrap_or("0"),
        );
    }
    if watchdog.meta_agent {
        row("meta_agent", "true");
        if let Some(seconds) = watchdog.sweep_sec {
            row("sweep_sec", seconds);
        }
    }
    // The prior meta's raw bytes, read ONCE: the seat-dir enlistment below
    // judges every launching slot against them. A fresh launch has no prior
    // meta, which reads as no rows — but a RESUME that cannot read its meta
    // must refuse rather than rebuild from nothing and silently drop rows.
    let prior_meta = match meta::read_bytes(&dir) {
        Ok(bytes) => bytes,
        Err(_) if !shape.resuming => Vec::new(),
        Err(why) => {
            return Err(format!(
                "could not read the prior meta to carry seat dirs forward: {why}"
            ));
        }
    };
    // A quota-driven effort move belongs to the conversation a resume keeps:
    // its record is carried RAW, every line in order, damage and CR included,
    // so the resume decision reads what was written. A fresh launch inherits
    // none, and a line that is not UTF-8 is carried as lossy text, still damage.
    let mut effort = String::new();
    for agent in launching {
        if shape.resuming {
            for line in crate::meta::effort_claims(&prior_meta, &agent.slot) {
                effort.push_str(&String::from_utf8_lossy(line));
                effort.push('\n');
            }
        }
        // The observed record likewise, so parse doubt never turns a damaged
        // record into a valid one; the launch baseline is never carried.
        for line in crate::meta::observed_claims(&prior_meta, &agent.slot) {
            effort.push_str(&String::from_utf8_lossy(line));
            effort.push('\n');
        }
        if !agent.launch_id.is_empty() {
            row(&format!("launch_id.{}", agent.slot), &agent.launch_id);
        }
        if agent.tool.adapter().capture.is_needed() {
            let key = format!("capture_floor.{}", agent.slot);
            let retained = shape.resuming && launch::id_probeable(&agent.session_id);
            let floor = if retained {
                // A retained exact conversation was born under the original
                // floor. Metadata predating this row gets 0 so a later resume
                // instant cannot exclude that already-proved origin.
                preserved(&key)
                    .filter(|value| value.parse::<i64>().is_ok_and(|epoch| epoch >= 0))
                    .unwrap_or_else(|| "0".to_owned())
            } else {
                started.clone()
            };
            row(&key, &floor);
        }
        // The observed-model pair and the predecessor chain are EVIDENCE a
        // later resume applies. This document is the WHOLE meta, so a row not
        // enumerated here is deleted by the first resume — before it could
        // ever be honored.
        let key = format!(
            "{}{}",
            crate::meta::HARNESS_SESSION_PRIOR_PREFIX,
            agent.slot
        );
        if let Some(value) = sole_preserved(&key) {
            row(&key, &value);
        }
        // The seat-dir row carries from the RAW prior bytes, judged before any
        // parse: a present-but-unusable row refuses the whole rebuild, and the
        // caller publishes only on Ok — so the prior meta is never half-moved.
        // Only absence inherits silently. The carried value is control-free by
        // construction, so the row cannot corrupt the document.
        match crate::meta::raw_seat_work_dir(&prior_meta, &agent.slot) {
            Ok(Some(target)) => row(&format!("work_dir.{}", agent.slot), &target),
            Ok(None) => {}
            Err(why) => return Err(why),
        }
    }
    body.push_str(&effort);

    let seats: Vec<roster::SeatLines> = launching
        .iter()
        .map(|agent| roster::SeatLines {
            slot: agent.slot.clone(),
            name: agent.name.clone(),
            profile: agent.profile.clone(),
            client: agent.client.clone(),
            binary: (!agent.binary.is_empty()).then(|| agent.binary.clone()),
            harness_session: (!agent.session_id.is_empty()).then(|| agent.session_id.clone()),
            config_home: agent.config_home.clone(),
            config_home_base: agent.config_home_base.clone(),
            // A launch never SETS a seat dir — only a spawn does, and the
            // resume carry-forward above already wrote this slot's row.
            work_dir: None,
        })
        .collect();
    if let Some(bad) = seats
        .iter()
        .find(|seat| seat.name.contains('\n') || seat.slot.contains('\n'))
    {
        return Err(format!(
            "a resolved identity contains a newline that would corrupt the meta: {:?}",
            seat_label(bad)
        ));
    }
    body.push_str(&roster::render(&seats));
    Ok(body)
}

/// A seat's label for a refusal.
fn seat_label(seat: &roster::SeatLines) -> String {
    format!("{}={}", seat.slot, seat.name)
}

/// What a vacant-only seed of the session UUID fact did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedOutcome {
    /// The option now holds this session's UUID — whether this call wrote it
    /// or it was already there. A post-read cannot tell those apart, and this
    /// type never pretends to: it reports the FINAL STATE, not a cause.
    Recorded,
    /// A DIFFERENT nonempty value is recorded; nothing was overwritten.
    Held,
    /// The option is vacant: the guarded branch refused (a replaced
    /// incarnation, a failed guard) or nothing was written. Again a state.
    Vacant,
    /// The option or the server could not be observed.
    Unobserved,
}

/// The full immutable identity a UUID write must reprove: the SERVER
/// incarnation (pid and start epoch) and the session's id and creation inside
/// it.
///
/// `#{session_id}` is REUSED once a server empties, and `#{session_created}` is
/// whole seconds, so neither alone survives a same-second replacement. The
/// server pair is the house model's discriminator: a replacement behind a
/// restarted or different server process is caught even when the reclaimed id
/// and the creation second coincide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvenIdentity {
    /// The tmux server the session was proven on.
    pub server: crate::tmux::ServerIdentity,
    /// The session's id and creation instant on that server.
    pub session: crate::tmux::SessionIdentity,
}

/// The MIGRATION-side capture: the identity of the session ONE pane belongs
/// to, for a recorded main pane. It is NOT the launch capture — a launch takes
/// its identity from the creating command's own output — and a pane id is
/// reusable after a server restart, so this value alone cannot tell a
/// replacement from the proven session. The caller pairs it with the session's
/// ownership pair, and the guarded write reproves the full identity.
#[must_use]
pub fn pane_proven_identity(server: &ServerId, pane: &str) -> Option<ProvenIdentity> {
    transport::observe_server_identity(server)
        .zip(transport::observe_pane_session_identity(server, pane))
        .map(|(server, session)| ProvenIdentity { server, session })
}

/// Seed `SESSION_ID_OPTION` for one live tmux session through ONE guarded
/// server-side operation.
///
/// The guard (`server pid/start`, session `id/created`, option empty) and the
/// set live inside the same tmux invocation's queued command, so nothing can
/// interleave between them: neither a full-server replacement after a first
/// proof call nor a same-name recreation gets the proven incarnation's UUID by
/// a read-then-write window. The outcome is deliberately a FINAL-STATE reading:
/// an option that already held this UUID is `Recorded` exactly like one this
/// call wrote, a replacement carrying a nonempty value is `Held`, and a vacant
/// option is `Vacant` — the type never labels a cause the post-read cannot
/// know. The
/// write itself is the guarded set. The ONE writer of the fact; the watchdog
/// never calls it.
#[must_use]
pub fn seed_session_uuid(
    server: &ServerId,
    expected_identity: &ProvenIdentity,
    uuid: &str,
) -> SeedOutcome {
    if !transport::publish_guarded_session_option(
        server,
        &expected_identity.server,
        &expected_identity.session,
        crate::theme::SESSION_ID_OPTION,
        uuid,
    ) {
        return SeedOutcome::Unobserved;
    }
    match transport::observe_option_reading(
        server,
        &expected_identity.session.id,
        crate::theme::SESSION_ID_OPTION,
    ) {
        crate::tmux::OptionReading::Set(value)
            if crate::archive::canonical_uuid(&value) == uuid =>
        {
            SeedOutcome::Recorded
        }
        crate::tmux::OptionReading::Set(_) => SeedOutcome::Held,
        crate::tmux::OptionReading::Vacant => SeedOutcome::Vacant,
        crate::tmux::OptionReading::Unknown => SeedOutcome::Unobserved,
    }
}

/// Publish `document` as the session meta, then — ONLY on success — seed the
/// session UUID fact from the SAME in-memory document, onto the SAME tmux
/// incarnation the caller proved.
///
/// One helper so neither ordering nor identity can drift: a failed publication
/// returns before any option write, the identity stamped is the one in the
/// document just published (never a second read), and the WRITE itself reproves
/// `expected_session_id` before it touches anything.
///
/// # Errors
///
/// The meta publication's own error; no option is written on it.
pub fn publish_meta_and_seed_uuid(
    server: &ServerId,
    expected_identity: Option<&ProvenIdentity>,
    dir: &Path,
    resuming: bool,
    document: &str,
) -> Result<SeedOutcome, crate::meta::RewriteError> {
    if resuming {
        crate::meta::replace(dir, document)?;
    } else {
        crate::meta::init(dir, document)?;
    }
    let uuid = crate::meta::first_value(document.as_bytes(), "session_id")
        .map(|value| crate::archive::canonical_uuid(&String::from_utf8_lossy(value)))
        .unwrap_or_default();
    let Some(expected_identity) = expected_identity.filter(|_| !uuid.is_empty()) else {
        return Ok(SeedOutcome::Unobserved);
    };
    Ok(seed_session_uuid(server, expected_identity, &uuid))
}
