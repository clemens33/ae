//! Launch preflight: the guards before any side effect, server and config resolution, the working copy, the seat roster.

use super::{
    AMBIGUOUS_SERVER, ControlFlow, Destination, EXIT_FAILED, EXIT_USAGE, Env, ExpectedLaunch,
    FromProof, IdentityConfig, JudgedSpawned, MISSING_NAME, Meta, Mode, PUBLIC_USAGE, Path,
    PathBuf, Placement, Placing, Plan, RecordedServer, Seat, SeatOverrideRefusal,
    SeatOverrideSnapshot, ServerId, Write, apply_seat_overrides, assert_status_bindings, config,
    configured_quota_every_secs, configured_unsigned_workspace_seconds, copy_tree,
    destination_is_absent, dir_exists, existing_directories_match, from_preflight, io,
    judge_spawned_labels, meta, meta_value, name, node_exists, path_bytes, placement,
    recorded_selection, recorded_wins_seconds, refuse_unproven, reserved_launch_actor,
    restore_saved_roster, restored_command, resume_absence, roster, running_override_refusal,
    same_directory, set_env_server, spawned_entries, stamp_lead_pair_policy, stamp_main_pane, tmux,
    transport, validate_seat_overrides,
};

// ---------------------------------------------------------------------------
// the launch's named steps, in the order `launch` calls them
// ---------------------------------------------------------------------------

/// The session name, and the guards on it that precede every side effect: it
/// is given, it matches the grammar, it is not a word the entry routes itself,
/// no pending rename owns it, and no unfinished teardown has parked its state
/// or working copy. Read-only, so a refusal has written nothing.
pub(super) fn check_name(
    env: &Env,
    plan: &Plan,
    expected_launch: Option<&ExpectedLaunch>,
    err: &mut impl Write,
) -> crate::error::Result<ControlFlow<u8, String>> {
    let Some(session) = plan.name.clone() else {
        writeln!(err, "{MISSING_NAME}")?;
        writeln!(err, "{PUBLIC_USAGE}")?;
        return Ok(ControlFlow::Break(EXIT_USAGE));
    };
    if !name::is_session_name(&session) {
        writeln!(
            err,
            "Error: invalid session name '{session}'. Names must match {}.",
            name::SESSION_NAME_GRAMMAR
        )?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    }

    // A word the entry routes ITSELF may not become a name: `ae <word>` would
    // answer the command and never reach the session. The picker's own resume
    // is exempt, because it addresses a session that already EXISTS and a
    // legacy one may still carry such a name.
    if expected_launch.is_none() && crate::entry::name_is_routed_verb(&session) {
        writeln!(err, "{}", crate::entry::routed_verb_refusal(&session))?;
        return Ok(ControlFlow::Break(EXIT_USAGE));
    }

    // A pending rename transaction owns this name until its proved retry
    // converges: a launch must not read or mutate a mixed generation as
    // ordinary state.
    if let Some(blocked) = crate::rename::intent_blocks(&env.home, &session) {
        writeln!(err, "Error: {blocked}. Nothing was launched.")?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    }

    for (root, note) in [
        (env.sessions(), "the session state is parked under it"),
        (env.worktrees(), "a working copy is parked under it"),
    ] {
        let tombstone = root.join(format!(".ending.{session}"));
        if node_exists(&tombstone) {
            writeln!(
                err,
                "Error: a previous teardown of '{session}' did not complete."
            )?;
            writeln!(
                err,
                "       {} is standing — {note}, not under '{session}'.",
                tombstone.display()
            )?;
            writeln!(err, "       Inspect it, then remove it by hand and retry:")?;
            writeln!(err, "           rm -rf {}", tombstone.display())?;
            return Ok(ControlFlow::Break(EXIT_FAILED));
        }
    }
    Ok(ControlFlow::Continue(session))
}

/// The refusals the launch's kind owes before the floor: settings Start
/// creates only the canonical orchestrator, `--resume` never CREATES a
/// session, a near-miss word never CREATES one either, and an expected Resume
/// whose record is gone is stale. Continues with whether the record was there
/// at preflight.
#[allow(
    clippy::too_many_arguments,
    reason = "one launch's kind: the captured expectation, the restore, the plan's resume intent and the two derived flags"
)]
pub(super) fn check_launch_kind(
    session: &str,
    dir: &Path,
    expected_launch: Option<&ExpectedLaunch>,
    restore: Option<&crate::restore::Attempt>,
    resume: bool,
    expected_resume: bool,
    settings_start: bool,
    err: &mut impl Write,
) -> crate::error::Result<ControlFlow<u8, bool>> {
    if matches!(
        crate::teardown::classify_dir(dir),
        crate::teardown::DirKind::Symlink
    ) && crate::lifecycle::aliases::target(dir).is_none()
    {
        writeln!(
            err,
            "Error: session path '{}' is a symlink; refusing to launch through it.",
            dir.display()
        )?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    }
    if settings_start && session != crate::orchestrator::ORCHESTRATOR_SESSION {
        writeln!(
            err,
            "Error: settings Start may create only the canonical orchestrator."
        )?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    }
    // Start may not inspect canonical saved state until it owns the canonical
    // lifecycle lock below. Resume may inspect only to select the recorded
    // server for the tmux floor; absence makes the captured action stale and
    // is refused before a lock-file or any other effect.
    let preflight_meta_present = !settings_start
        && crate::lifecycle::aliases::target(dir).is_none()
        && node_exists(&dir.join(crate::store::META));
    // `--resume` is asked first: it names the missing session instead of the
    // near-miss hint that would tell a caller to pick another name to create.
    if let ControlFlow::Break(code) = check_resume_target(
        session,
        resume,
        carries_own_proof(expected_launch, restore),
        preflight_meta_present,
        err,
    )? {
        return Ok(ControlFlow::Break(code));
    }
    // A near-miss word may not CREATE a session: no record is the launch's own
    // create-versus-resume discriminator; a session already named so is judged
    // by `check_stopped_near_miss` once the floor has passed. Read-only above
    // this line, so a refusal has written nothing.
    if expected_launch.is_none()
        && restore.is_none()
        && !preflight_meta_present
        && let Some(refusal) = crate::entry::near_miss_refusal(session)
    {
        writeln!(err, "{refusal}")?;
        return Ok(ControlFlow::Break(EXIT_USAGE));
    }
    if expected_resume && !preflight_meta_present {
        writeln!(
            err,
            "Error: role target '{session}' disappeared before Resume acquired its lifecycle lock. Nothing was resumed."
        )?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    }
    Ok(ControlFlow::Continue(preflight_meta_present))
}

/// Whether a launch brings its own proof of an existing record: the picker or
/// menu's captured expectation, or a restore. Such a launch is not asked for a
/// sign of intent (`--resume`) and a near-miss word does not refuse it.
pub(super) fn carries_own_proof(
    expected_launch: Option<&ExpectedLaunch>,
    restore: Option<&crate::restore::Attempt>,
) -> bool {
    expected_launch.is_some() || restore.is_some()
}

/// `--resume` names an existing session and never creates one: with no record
/// there is nothing to resume, and the launch refuses before it writes
/// anything. Picker, menu and restore launches carry their own proof of an
/// existing record and are not asked.
pub(super) fn check_resume_target(
    session: &str,
    resume: bool,
    carries_own_proof: bool,
    preflight_meta_present: bool,
    err: &mut impl Write,
) -> crate::error::Result<ControlFlow<u8, ()>> {
    if resume && !carries_own_proof && !preflight_meta_present {
        writeln!(
            err,
            "Error: --resume found no session named '{session}' to resume. Nothing was created."
        )?;
        return Ok(ControlFlow::Break(EXIT_USAGE));
    }
    Ok(ControlFlow::Continue(()))
}

/// A near-miss word names a STOPPED session: `ae sessions` meant `ae list`, and
/// a resume starts every seat of the old session, so it needs a sign that it is
/// meant — `--resume`, the picker or menu's expected launch, or a restore.
/// A RUNNING session of that name only attaches and is not asked. Read-only, so
/// a refusal has written nothing.
pub(super) fn check_stopped_near_miss(
    session: &str,
    deliberate: bool,
    preflight_meta_present: bool,
    running_preflight: bool,
    err: &mut impl Write,
) -> crate::error::Result<ControlFlow<u8, ()>> {
    if !deliberate
        && preflight_meta_present
        && !running_preflight
        && let Some(refusal) = crate::entry::near_miss_resume_refusal(session)
    {
        writeln!(err, "{refusal}")?;
        return Ok(ControlFlow::Break(EXIT_USAGE));
    }
    Ok(ControlFlow::Continue(()))
}

/// The server a launch's tmux floor is asked of: the proposed one when no
/// record was there at preflight. A resume's is decided from a read-only look
/// at the record before any lock: the recorded one when it runs there or stays
/// there, the proposed one when the stopped session moves. An unproven absence
/// refuses, or skips for a restore.
pub(super) fn preflight_server(
    dir: &Path,
    session: &str,
    placing: Placing<'_>,
    preflight_meta_present: bool,
    err: &mut impl Write,
) -> crate::error::Result<ControlFlow<u8, ServerId>> {
    if !preflight_meta_present {
        return Ok(ControlFlow::Continue(placing.proposed.clone()));
    }
    let Ok(bytes) = meta::read_bytes(dir) else {
        writeln!(err, "Error: could not read metadata for '{session}'.")?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    };
    let Some(recorded) =
        RecordedServer::of(Meta::parse(&String::from_utf8_lossy(&bytes)).server_selector())
    else {
        writeln!(err, "Error: '{session}' {AMBIGUOUS_SERVER}.")?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    };
    let (probe, why) = resume_absence(&recorded.server, session, dir);
    Ok(match placement(&probe, placing.in_place, recorded.legacy) {
        Placement::Running | Placement::Stopped(Destination::Recorded) => {
            ControlFlow::Continue(recorded.server)
        }
        Placement::Stopped(Destination::Proposed) => {
            ControlFlow::Continue(placing.proposed.clone())
        }
        Placement::Unproven => ControlFlow::Break(refuse_unproven(
            placing.restore,
            &recorded,
            session,
            why,
            err,
        )?),
    })
}

/// The seat overrides this launch may apply, validated against the record and
/// the running state seen at preflight, and on a create the refusal of an
/// agent name ae reserves for its journal actors.
pub(super) fn check_seat_overrides(
    env: &Env,
    plan: &Plan,
    dir: &Path,
    preflight_meta_present: bool,
    running_preflight: bool,
    err: &mut impl Write,
) -> crate::error::Result<ControlFlow<u8, Option<SeatOverrideSnapshot>>> {
    let seat_overrides =
        match validate_seat_overrides(env, plan, dir, preflight_meta_present, running_preflight) {
            Ok(snapshot) => snapshot,
            Err(SeatOverrideRefusal::Usage(line)) => {
                writeln!(err, "{line}")?;
                return Ok(ControlFlow::Break(EXIT_USAGE));
            }
            Err(SeatOverrideRefusal::Failed(line)) => {
                writeln!(err, "{line}")?;
                return Ok(ControlFlow::Break(EXIT_FAILED));
            }
        };

    if !preflight_meta_present
        && let Some(actor) = reserved_launch_actor(env, plan, seat_overrides.as_ref())
    {
        writeln!(
            err,
            "Error: agent name '{actor}' is reserved for ae's journal actors; pick another name."
        )?;
        return Ok(ControlFlow::Break(EXIT_USAGE));
    }
    Ok(ControlFlow::Continue(seat_overrides))
}

/// Judge every recorded `client.<slot>` label of a stopped session's
/// `spawned.<n>` seats BEFORE the lifecycle lock, through the same adapter gate
/// and store re-take a fixed seat's carried label takes — against the session's
/// recorded GLOBAL identity alone, where `spawn`/`reseat --using p@c` minted the
/// label, so a project overlay can never redirect the pair. Approval is not
/// re-asked: a retired `[client_pairs]` row never strands an admitted seat.
pub(super) fn check_spawned_labels(
    plan: &Plan,
    dir: &Path,
    preflight_meta_present: bool,
    running_preflight: bool,
    err: &mut impl Write,
) -> crate::error::Result<ControlFlow<u8, Vec<JudgedSpawned>>> {
    if !preflight_meta_present || running_preflight {
        return Ok(ControlFlow::Continue(Vec::new()));
    }
    let session = plan.name.as_deref().unwrap_or_default();
    match judge_spawned_labels(dir, session, crate::doors::home().as_deref()) {
        Ok(judged) => Ok(ControlFlow::Continue(judged)),
        Err(SeatOverrideRefusal::Usage(line)) => {
            writeln!(err, "{line}")?;
            Ok(ControlFlow::Break(EXIT_USAGE))
        }
        Err(SeatOverrideRefusal::Failed(line)) => {
            writeln!(err, "{line}")?;
            Ok(ControlFlow::Break(EXIT_FAILED))
        }
    }
}

/// The refusal of a launch whose lifecycle lock another operation holds past
/// the wait: a settings Start is stale by then, anything else may retry.
pub(super) fn lifecycle_busy(
    session: &str,
    settings_start: bool,
    err: &mut impl Write,
) -> io::Result<u8> {
    let detail = if settings_start {
        " — stale Start refused"
    } else {
        " — retry shortly"
    };
    writeln!(
        err,
        "Error: another lifecycle operation is in progress for '{session}'{detail}."
    )?;
    Ok(EXIT_FAILED)
}

/// The server this launch acts on, decided again under the lifecycle lock,
/// where the record is authoritative. A saved session's explicit `--dir` must
/// name its recorded origin; then a running one continues with the server it
/// runs on (a legacy one adopted and backfilled first), and a stopped one with
/// its destination proved free. `env` is pointed at that server. Without a
/// record, a live tmux session of the name is not ours to build over.
pub(super) fn resolve_server(
    env: &mut Env,
    plan: &Plan,
    dir: &Path,
    session: &str,
    placing: Placing<'_>,
    meta_present: bool,
    err: &mut impl Write,
) -> crate::error::Result<ControlFlow<u8, Option<ServerId>>> {
    if !meta_present {
        if transport::session_exists(placing.proposed, session) {
            writeln!(
                err,
                "Error: tmux session '{session}' exists but is not an ae session."
            )?;
            return Ok(ControlFlow::Break(EXIT_FAILED));
        }
        return Ok(ControlFlow::Continue(None));
    }
    let Ok(bytes) = meta::read_bytes(dir) else {
        writeln!(err, "Error: could not read metadata for '{session}'.")?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    };
    // An explicit origin is a claim about this exact session, including a
    // live one. It must agree with the recorded owner before reattach can
    // migrate, seed, or backfill anything below.
    if plan.dir.is_some() {
        let cwd = env.cwd.display().to_string();
        let recorded = meta::first_value(&bytes, "origin")
            .map(|value| String::from_utf8_lossy(value).into_owned())
            .unwrap_or_default();
        if recorded.is_empty() || !same_directory(&recorded, &cwd) {
            writeln!(
                err,
                "Error: session '{session}' exists with a different origin."
            )?;
            writeln!(
                err,
                "       recorded: {}",
                if recorded.is_empty() {
                    "<none>"
                } else {
                    &recorded
                }
            )?;
            writeln!(err, "       requested: {cwd}")?;
            return Ok(ControlFlow::Break(EXIT_FAILED));
        }
    }
    let Some(recorded) =
        RecordedServer::of(Meta::parse(&String::from_utf8_lossy(&bytes)).server_selector())
    else {
        writeln!(err, "Error: '{session}' {AMBIGUOUS_SERVER}.")?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    };
    let (probe, why) = resume_absence(&recorded.server, session, dir);
    match placement(&probe, placing.in_place, recorded.legacy) {
        Placement::Running if recorded.legacy => {
            let backfilled = match adopt_legacy_session(
                &recorded.server,
                &env.home,
                session,
                dir,
                !plan.seat_profiles.is_empty(),
                err,
            )? {
                ControlFlow::Continue(server) => server,
                ControlFlow::Break(code) => return Ok(ControlFlow::Break(code)),
            };
            set_env_server(env, &backfilled);
            Ok(ControlFlow::Continue(Some(backfilled)))
        }
        Placement::Running => {
            if let Some(attempt) = placing.restore {
                attempt.skip("already running");
                return Ok(ControlFlow::Break(0));
            }
            if !plan.seat_profiles.is_empty() {
                writeln!(err, "{}", running_override_refusal(session))?;
                return Ok(ControlFlow::Break(EXIT_USAGE));
            }
            set_env_server(env, &recorded.server);
            Ok(ControlFlow::Continue(Some(recorded.server)))
        }
        Placement::Stopped(destination) => {
            let target = match destination {
                Destination::Recorded => &recorded.server,
                Destination::Proposed => placing.proposed,
            };
            if !destination_is_absent(target, session, err)? {
                return Ok(ControlFlow::Break(EXIT_FAILED));
            }
            if destination == Destination::Recorded {
                set_env_server(env, &recorded.server);
            }
            Ok(ControlFlow::Continue(None))
        }
        Placement::Unproven => Ok(ControlFlow::Break(refuse_unproven(
            placing.restore,
            &recorded,
            session,
            why,
            err,
        )?)),
    }
}

/// Adopt a legacy session — a record with no server rows — that is running on
/// the historical default server. It is ours only when its ownership marker
/// names this state root AND its recorded main pane is one of its panes; then,
/// unless seat overrides were asked of a running session, its socket is
/// backfilled into the record before anything acts on it. Continues with the
/// backfilled server.
fn adopt_legacy_session(
    historical: &ServerId,
    home: &Path,
    session: &str,
    dir: &Path,
    overrides: bool,
    err: &mut impl Write,
) -> crate::error::Result<ControlFlow<u8, ServerId>> {
    let ownership = transport::observe_session_ownership(historical, session);
    let main_pane = meta_value(dir, "main_pane").unwrap_or_default();
    let pane_belongs = !main_pane.is_empty()
        && transport::observe_agents(historical, session)
            .is_some_and(|panes| panes.into_iter().any(|pane| pane.pane == main_pane));
    let owned = ownership.is_some_and(|ownership| {
        !ownership.marker.is_empty() && existing_directories_match(Path::new(&ownership.home), home)
    }) && pane_belongs;
    let socket = owned
        .then(|| transport::observe_socket_path(historical))
        .flatten();
    let Some(socket) = socket else {
        writeln!(
            err,
            "Error: tmux session '{session}' exists on the historical default server, but ae could not prove it belongs to this state root. Metadata was not changed."
        )?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    };
    if overrides {
        writeln!(err, "{}", running_override_refusal(session))?;
        return Ok(ControlFlow::Break(EXIT_USAGE));
    }
    if let Err(why) = meta::backfill_server_socket(dir, &socket) {
        writeln!(
            err,
            "Error: could not backfill the tmux server for '{session}' ({}).",
            why.cause()
        )?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    }
    Ok(ControlFlow::Continue(ServerId::Selected(
        meta::Selector::Socket(socket.into()),
    )))
}

/// A saved session's record, made placeable and made the config it reads:
/// the migration chain runs before anything reads a field of the record, then
/// the recorded `config` and the session's local overlay replace the caller's.
/// Continues with whether the overlay is the orchestrator seat's, noting the
/// `[roster]`/`[profiles]` the seat ignores unless seat overrides were asked.
pub(super) fn adopt_recorded_config(
    env: &mut Env,
    dir: &Path,
    session: &str,
    meta_present: bool,
    overridden: bool,
    err: &mut impl Write,
) -> crate::error::Result<ControlFlow<u8, bool>> {
    // THE CHAIN, before anything reads a field of this meta: a resume or a
    // reattach is ae touching a session, and a shape it cannot place is one it
    // must not act on.
    if meta_present && let Err(refusal) = crate::migrate::session(dir) {
        writeln!(err, "Error: {}", refusal.line(session))?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    }
    // On resume the RECORDED config wins: an agent's aliases must resolve from
    // the file the session was created with, not from wherever the caller is.
    if meta_present {
        if let Some(stored) = meta_value(dir, "config").filter(|v| !v.is_empty()) {
            env.global = Some(PathBuf::from(stored));
        }
        let origin = meta_value(dir, "origin").unwrap_or_default();
        env.local = config::local_overlay(dir, &origin);
    }
    let orchestrator_seat = env
        .local
        .as_deref()
        .is_some_and(|local| crate::orchestrator::is_seat_overlay(local, &env.home));
    if orchestrator_seat && !overridden {
        let (_, has_profiles, has_roster) =
            config::read_workspace_keys_with_identity_sections(None, env.local.as_deref(), &[]);
        if (has_profiles || has_roster)
            && let (Some(local), Some(global)) = (env.local.as_deref(), env.global.as_deref())
        {
            writeln!(
                err,
                "ae orchestrator: ignoring [roster]/[profiles] in {}: the seat profile is [roster] orchestrator in {}",
                local.display(),
                global.display()
            )?;
        }
    }
    Ok(ControlFlow::Continue(orchestrator_seat))
}

/// Prove a `--from` parent archive for a NEW session: the archive preflight
/// first, then `exists` (asked only once the archive proved) refuses a name
/// that is already a running or resumable session, then THE SECOND PROOF.
/// compact took the first at its boundary; between the two the archive could
/// have been purged, claimed or corrupted, and a child published with a
/// lineage pointer to something that is no longer there is exactly the case
/// `--from` exists to refuse.
pub(super) fn prove_lineage(
    root: &Path,
    uuid: &str,
    expected_proof: Option<&str>,
    exists: impl FnOnce() -> bool,
    session: &str,
    err: &mut impl Write,
) -> crate::error::Result<ControlFlow<u8, FromProof>> {
    let proof = match from_preflight(root, uuid) {
        Ok(proof) => proof,
        Err(line) => {
            writeln!(err, "{line}")?;
            return Ok(ControlFlow::Break(EXIT_FAILED));
        }
    };
    if exists() {
        writeln!(
            err,
            "Error: --from is only valid for a NEW session, and '{session}' already exists."
        )?;
        writeln!(
            err,
            "       Inheriting an archive into a running or resumable session would mean two"
        )?;
        writeln!(
            err,
            "       different things (replace its history? merge it?) with no safe default."
        )?;
        writeln!(err, "       Start the continuation under its own name:")?;
        writeln!(err, "           ae <new-name> --from {}", proof.id)?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    }
    let now = format!("{}\t{}\t{}", proof.id, proof.handover, proof.pending);
    if let Some(expected) = expected_proof
        && now != expected
    {
        writeln!(
            err,
            "Error: parent archive {} changed while this session was being created.",
            proof.id
        )?;
        writeln!(err, "  Expected: {}", expected.replace('\t', " | "))?;
        writeln!(err, "  Now:      {}", now.replace('\t', " | "))?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    }
    Ok(ControlFlow::Continue(proof))
}

/// What a reattach stamps again on a running session's server, rebuilding
/// nothing: the status bindings when the core can name its own binary, the
/// recorded main pane's focus hook, and the lead-pair resize policy only when
/// that pane is one of the session's own.
pub(super) fn restamp_running(
    server: &ServerId,
    env: &Env,
    dir: &Path,
    session: &str,
    observed: &[tmux::ObservedAgent],
) {
    if let Some(core) = env.core.clone().or_else(crate::shape::resolved_exe) {
        assert_status_bindings(server, env, &core);
    }
    let layout = meta_value(dir, "layout").unwrap_or_default();
    let main_pane = meta_value(dir, "main_pane").unwrap_or_default();
    let pane_belongs = observed.iter().any(|pane| pane.pane == main_pane);
    stamp_main_pane(server, session, &main_pane, pane_belongs);
    if pane_belongs {
        stamp_lead_pair_policy(server, &layout, &main_pane);
    }
}

/// The `[workspace]` values a launch takes from config, read once.
pub(super) struct WorkspaceValues {
    /// `layout`, empty when unset.
    pub(super) layout: String,
    /// `copy`, the configured working-copy mode, empty when unset.
    pub(super) copy: String,
    pub(super) look: crate::theme::Look,
    /// `orchestrator` (or its older spellings `hub`, `meta`) is on.
    pub(super) meta_agent: bool,
    /// `sweep`, when it is unsigned seconds.
    pub(super) sweep_sec: Option<String>,
    /// `quota`, empty when unset.
    pub(super) quota: String,
}

pub(super) fn read_workspace_values(env: &Env) -> WorkspaceValues {
    let extras = config::read_workspace_keys(
        env.global.as_deref(),
        env.local.as_deref(),
        &[
            "layout",
            "copy",
            "watchdog",
            "loop",
            "orchestrator",
            "hub",
            "meta",
            "palette",
            "icons",
            "theme",
            "motion",
            "sweep",
            "quota",
        ],
    );
    let look = crate::theme::Look::read(
        &extras[8].clone().unwrap_or_default(),
        &extras[7].clone().unwrap_or_default(),
        &extras[9].clone().unwrap_or_default(),
        &extras[10].clone().unwrap_or_default(),
    );
    let meta_agent = ["true", "1", "yes", "on"].contains(
        &extras[4]
            .clone()
            .or_else(|| extras[5].clone())
            .or_else(|| extras[6].clone())
            .unwrap_or_default()
            .as_str(),
    );
    let sweep_sec = extras[11]
        .as_deref()
        .and_then(|value| value.parse::<u64>().ok())
        .map(|value| value.to_string());
    WorkspaceValues {
        layout: extras[0].clone().unwrap_or_default(),
        copy: extras[1].clone().unwrap_or_default(),
        look,
        meta_agent,
        sweep_sec,
        quota: extras[12].clone().unwrap_or_default(),
    }
}

/// The orchestrator seat's own config, seeded on its first launch once the
/// global roster names the seat's profile. Asked after the global first-run
/// seed, but only after the live-session reattach branch: a running seat is
/// never rebuilt or revalidated.
pub(super) fn seed_orchestrator_seat(
    env: &Env,
    orchestrator_config: &Path,
    err: &mut impl Write,
) -> crate::error::Result<ControlFlow<u8>> {
    if let Some(global) = env.global.as_deref() {
        // The global roster chooses the seat's profile.
        let cfg = match config::read_identity(Some(global), None) {
            Ok(cfg) => cfg,
            Err(why) => {
                writeln!(err, "{why}")?;
                return Ok(ControlFlow::Break(EXIT_FAILED));
            }
        };
        if cfg
            .roster_profile(crate::orchestrator::ORCHESTRATOR_SESSION)
            .is_none()
        {
            writeln!(
                err,
                "ae orchestrator: no profile for the seat — add \"orchestrator = <profile>\" under [roster] in {}",
                global.display()
            )?;
            return Ok(ControlFlow::Break(EXIT_FAILED));
        }
    }
    if let Some(code) = crate::seed_default_config(
        orchestrator_config,
        crate::orchestrator::DEFAULT_CONFIG,
        "orchestrator",
        err,
    )? {
        return Ok(ControlFlow::Break(code));
    }
    Ok(ControlFlow::Continue(()))
}

/// The watchdog facts a launch records, each META-WINS: a resume keeps what
/// its session recorded, so a config flip applies to NEW sessions only.
pub(super) struct WatchdogCadences {
    pub(super) quota_every_secs: String,
    pub(super) idle_nudge_secs: String,
    pub(super) done_confirmations: String,
    /// `on` or `off`.
    pub(super) quota: String,
}

/// Resolve the [`WatchdogCadences`]: `resumed` is the session directory of a
/// resume, whose recorded rows win; a configured value is read only for a row
/// that is absent. `configured_quota` is the `[workspace] quota` already read.
pub(super) fn resolve_watchdog_cadences(
    env: &Env,
    resumed: Option<&Path>,
    configured_quota: &str,
    err: &mut impl Write,
) -> crate::error::Result<ControlFlow<u8, WatchdogCadences>> {
    let recorded = |key: &str| resumed.and_then(|dir| meta_value(dir, key));
    let quota_every_secs = match recorded_wins_seconds(
        recorded("quota_every_secs"),
        || configured_quota_every_secs(env.global.as_deref(), env.local.as_deref()),
        crate::watchdog_daemon::Knobs::default().quota_every_secs,
    ) {
        Ok(value) => value,
        Err(value) => {
            writeln!(
                err,
                "Error: [workspace] quota_every_secs must be an unsigned integer in seconds; got '{value}'."
            )?;
            return Ok(ControlFlow::Break(EXIT_USAGE));
        }
    };
    let idle_nudge_secs = match recorded_wins_seconds(
        recorded("idle_nudge_secs"),
        || {
            configured_unsigned_workspace_seconds(
                env.global.as_deref(),
                env.local.as_deref(),
                "idle_nudge_secs",
            )
        },
        crate::watchdog_daemon::Knobs::default().idle_nudge_secs,
    ) {
        Ok(value) => value,
        Err(value) => {
            writeln!(
                err,
                "Error: [workspace] idle_nudge_secs must be an unsigned integer in seconds; got '{value}'."
            )?;
            return Ok(ControlFlow::Break(EXIT_USAGE));
        }
    };
    let recorded_done = recorded("done_confirmations");
    let configured_done = if recorded_done.is_none() {
        if let Ok(value) = configured_unsigned_workspace_seconds(
            env.global.as_deref(),
            env.local.as_deref(),
            "done_confirmations",
        ) {
            value
        } else {
            writeln!(err, "Note: done_confirmations ignored; using default 2")?;
            None
        }
    } else {
        None
    };
    let (done_confirmations, note) = crate::meta::resolve_done_confirmations(
        recorded_done.as_deref().or(configured_done.as_deref()),
        false,
        crate::watchdog::DEFAULT_DONE_CONFIRMATIONS,
    );
    if let Some(note) = note {
        writeln!(err, "Note: {note}")?;
    }
    // `quota = off` is pinned like `quota_every_secs`: the recorded meta wins
    // on resume so a config flip applies to NEW sessions only. Absent means ON
    // — exactly today's behaviour — through the ONE `config::quota_aware`
    // predicate. `quota = off` WINS over `quota_every_secs`: a cadence is
    // meaningless when the feature is off, and the watchdog enforces that.
    let quota_raw = recorded("quota").unwrap_or_else(|| configured_quota.to_owned());
    let quota = if crate::config::quota_aware(&quota_raw) {
        "on"
    } else {
        "off"
    }
    .to_owned();
    Ok(ControlFlow::Continue(WatchdogCadences {
        quota_every_secs,
        idle_nudge_secs,
        done_confirmations: done_confirmations.to_string(),
        quota,
    }))
}

/// The seats this launch starts with: the config's roster, its workers
/// replaced by the plan's when given, and its main seat by `use <name>`.
pub(super) fn resolve_seats(
    cfg: &mut IdentityConfig,
    plan: &Plan,
    home: Option<&Path>,
    err: &mut impl Write,
) -> crate::error::Result<ControlFlow<u8, Vec<Seat>>> {
    if let Some(workers) = &plan.workers {
        cfg.workers = Some(workers.clone());
    }
    match config::launch_plan(cfg, plan.main.as_deref(), home) {
        Ok(resolved) => Ok(ControlFlow::Continue(resolved.seats)),
        Err(violations) => {
            write!(err, "{}", config::render_violations(&violations))?;
            Ok(ControlFlow::Break(EXIT_FAILED))
        }
    }
}

/// The recorded-client gate for a resume WITHOUT overrides (an override launch
/// took it in preflight): an unusable row refuses with its own remedy before
/// the generic doubtful-roster refusal, which stays as the backstop for every
/// other anomaly.
pub(super) fn check_resumed_roster(
    dir: &Path,
    session: &str,
    cfg: &IdentityConfig,
    err: &mut impl Write,
) -> crate::error::Result<ControlFlow<u8>> {
    let parsed = meta::read_bytes(dir)
        .ok()
        .map(|bytes| Meta::parse(&String::from_utf8_lossy(&bytes)));
    if let Some(refusal) = parsed.as_ref().and_then(|meta| {
        meta.roster()
            .iter()
            .find_map(|entry| recorded_selection(session, cfg, entry).err())
    }) {
        writeln!(err, "{refusal}")?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    }
    if let Some(anomaly) = parsed.as_ref().and_then(|meta| {
        meta.anomalies()
            .iter()
            .find(|item| roster::roster_doubting(item))
    }) {
        writeln!(
            err,
            "Error: session '{session}' has doubtful roster metadata ({anomaly}); run 'ae doctor' before resuming."
        )?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    }
    Ok(ControlFlow::Continue(()))
}

/// The directory a launch's agents work in, the origin it records, and the
/// mode it ends in (a `--worktree` outside a git repo falls back to a full
/// copy).
pub(super) struct WorkingCopy {
    pub(super) work_dir: PathBuf,
    pub(super) origin: PathBuf,
    pub(super) mode: Mode,
}

/// Find the working copy a resume recorded, or make the one a fresh launch
/// asked for. Local mode works in `cwd` and never copies; a managed copy is
/// created under `work_root` only when nothing stands there, and a failed one
/// is removed again. `resumed` is the session directory of a resume, whose
/// record names the copy; `None` for a fresh launch.
pub(super) fn prepare_working_copy(
    cwd: &Path,
    work_root: &Path,
    session: &str,
    mut mode: Mode,
    resumed: Option<&Path>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::error::Result<ControlFlow<u8, WorkingCopy>> {
    let mut origin = cwd.to_path_buf();
    let mut work_dir = work_root.to_path_buf();
    if mode == Mode::Local {
        if dir_exists(work_root) {
            writeln!(
                err,
                "Error: stopped session '{session}' exists as worktree/copy. End it first (ae end {session}), or use a different name."
            )?;
            return Ok(ControlFlow::Break(EXIT_FAILED));
        }
        work_dir = cwd.to_path_buf();
        if let Some(dir) = resumed {
            if let Some(stored) = meta_value(dir, "work_dir").filter(|v| dir_exists(Path::new(v))) {
                work_dir = PathBuf::from(stored);
            }
            if let Some(stored) = meta_value(dir, "origin").filter(|v| !v.is_empty()) {
                origin = PathBuf::from(stored);
            }
            writeln!(
                out,
                "Resuming session {session} (dir: {})...",
                work_dir.display()
            )?;
        }
    } else if let Some(dir) = resumed {
        let stored_work = meta_value(dir, "work_dir").filter(|value| !value.is_empty());
        if let Some(stored) = stored_work.as_deref() {
            if !dir_exists(Path::new(stored)) {
                writeln!(
                    err,
                    "Error: '{session}' records its working copy at {stored} but it is gone — restore it or end the session (ae end {session})"
                )?;
                return Ok(ControlFlow::Break(EXIT_FAILED));
            }
            work_dir = PathBuf::from(stored);
        } else if !dir_exists(work_root) {
            writeln!(
                err,
                "Error: '{session}' records its working copy at {} but it is gone — restore it or end the session (ae end {session})",
                work_root.display()
            )?;
            return Ok(ControlFlow::Break(EXIT_FAILED));
        }
        if let Some(stored) = meta_value(dir, "origin").filter(|v| !v.is_empty()) {
            origin = PathBuf::from(stored);
        }
        writeln!(
            out,
            "Resuming session {session} (dir: {})...",
            work_dir.display()
        )?;
    } else {
        match create_working_copy(cwd, work_root, mode, out, err)? {
            ControlFlow::Continue(created) => mode = created,
            ControlFlow::Break(code) => return Ok(ControlFlow::Break(code)),
        }
    }
    Ok(ControlFlow::Continue(WorkingCopy {
        work_dir,
        origin,
        mode,
    }))
}

/// Create a fresh launch's managed copy under `work_root` — a detached git
/// worktree, or a full copy (also the fallback outside a git repo) — refusing
/// to touch anything already standing there and removing a copy that failed.
/// Continues with the mode the copy was made in.
fn create_working_copy(
    cwd: &Path,
    work_root: &Path,
    mode: Mode,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::error::Result<ControlFlow<u8, Mode>> {
    if dir_exists(work_root) {
        writeln!(
            err,
            "Error: working copy '{}' exists without session metadata; refusing to overwrite it — choose a fresh name or remove/relocate the preserved workdir yourself.",
            work_root.display()
        )?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    }
    if let Err(why) = std::fs::create_dir_all(work_root) {
        writeln!(
            err,
            "Error: could not create {} ({why})",
            work_root.display()
        )?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    }
    let mut mode = mode;
    match mode {
        Mode::Git if crate::git::is_work_tree(path_bytes(cwd)) => {
            writeln!(out, "Creating git worktree...")?;
            if !crate::git::worktree_add_detached(path_bytes(cwd), path_bytes(work_root)) {
                let _ = std::fs::remove_dir_all(work_root);
                writeln!(
                    err,
                    "Error: could not create a git worktree at {}",
                    work_root.display()
                )?;
                return Ok(ControlFlow::Break(EXIT_FAILED));
            }
        }
        Mode::Git => {
            writeln!(err, "Warning: not a git repo, falling back to full copy.")?;
            mode = Mode::Full;
            writeln!(out, "Creating full copy...")?;
            if let Err(why) = copy_tree(cwd, work_root) {
                let _ = std::fs::remove_dir_all(work_root);
                writeln!(err, "Error: could not copy the working tree ({why})")?;
                return Ok(ControlFlow::Break(EXIT_FAILED));
            }
        }
        Mode::Full => {
            writeln!(out, "Creating full copy...")?;
            if let Err(why) = copy_tree(cwd, work_root) {
                let _ = std::fs::remove_dir_all(work_root);
                writeln!(err, "Error: could not copy the working tree ({why})")?;
                return Ok(ControlFlow::Break(EXIT_FAILED));
            }
        }
        Mode::Local => unreachable!("local mode never creates a copy"),
    }
    writeln!(out, "Working copy ready.")?;
    Ok(ControlFlow::Continue(mode))
}

/// A RESUME RESTORES THE ROSTER IT SAVED: `resumed` is the session directory
/// of a resume, whose recorded seat profiles replace the config's, leaving to
/// the seat overrides the seats they replace; then the overrides are applied.
pub(super) fn restore_roster(
    resumed: Option<&Path>,
    session: &str,
    cfg: &IdentityConfig,
    home: Option<&Path>,
    seat_overrides: Option<&SeatOverrideSnapshot>,
    seats: &mut [Seat],
    err: &mut impl Write,
) -> crate::error::Result<ControlFlow<u8>> {
    if let Some(dir) = resumed {
        let overridden: Vec<&str> = seat_overrides.map_or_else(Vec::new, |snapshot| {
            snapshot
                .overrides
                .iter()
                .map(|replacement| replacement.agent.as_str())
                .collect()
        });
        if let Err(line) = restore_saved_roster(dir, session, cfg, home, &overridden, seats) {
            writeln!(err, "{line}")?;
            return Ok(ControlFlow::Break(EXIT_FAILED));
        }
    }
    if let Some(snapshot) = seat_overrides
        && let Err(line) = apply_seat_overrides(snapshot, seats)
    {
        writeln!(err, "{line}")?;
        return Ok(ControlFlow::Break(EXIT_FAILED));
    }
    Ok(ControlFlow::Continue(()))
}

/// Lex every restored spawned seat's command before anything is created.
///
/// A spawned seat records a PROFILE NAME, and the resume looks that name up
/// in whatever config is current — so the command it resolves to is one no
/// earlier validation ever saw. `config::launch_plan` lexes the [roster]
/// seats and `_spawn` lexes the profile it is handed, but this path did
/// neither: it read `cfg.command()`, opened a pane, and let the pane shell
/// run the string. A profile holding `touch m ; tail -f /dev/null` therefore
/// executed BOTH commands on resume — the same defect the spawn gate closed
/// (colead gate b5d60fec), reached through the restore instead.
pub(super) fn lex_spawned_seats(
    dir: &Path,
    session: &str,
    cfg: &IdentityConfig,
    judged: &[JudgedSpawned],
    home: Option<&Path>,
    err: &mut impl Write,
) -> crate::error::Result<ControlFlow<u8>> {
    let spawned = match spawned_entries(dir, session) {
        Ok(spawned) => spawned,
        Err(line) => {
            writeln!(err, "{line}")?;
            return Ok(ControlFlow::Break(EXIT_FAILED));
        }
    };
    for entry in spawned {
        // An unconfigured profile is not a refusal: the seat is preserved
        // verbatim and never launched, which the restore already handles.
        let command = match restored_command(cfg, &entry, judged, session, home) {
            Ok(command) => command,
            Err(why) => {
                writeln!(err, "{why}")?;
                return Ok(ControlFlow::Break(EXIT_FAILED));
            }
        };
        let Some(command) = command.filter(|command| !command.as_str().is_empty()) else {
            continue;
        };
        if let Err(why) = crate::launch_cmd::lex_simple_command(command.as_str()) {
            writeln!(
                err,
                "Error: seat '{}' ({}) — profile '{}' refused — {why}. Nothing was resumed.",
                entry.name, entry.slot, entry.profile
            )?;
            return Ok(ControlFlow::Break(EXIT_FAILED));
        }
    }
    Ok(ControlFlow::Continue(()))
}
