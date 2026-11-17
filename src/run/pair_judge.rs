//! The ONE judge of a seat's recorded `profile@client` pair: the adapter gate,
//! the recorded selection, the recorded-store re-take and the refusals they
//! speak. It sits in the services layer, so the seat runner (a snapshot-less
//! `_run`) and the orchestration callers (a resume, a relaunch, `--seat`, a
//! spawn and a reseat's admission) judge a pair by the same code without the
//! runner naming a higher layer (docs/reference/contract.md#layer-map).

use std::path::Path;

use crate::config::{self, IdentityConfig};
use crate::meta;
use crate::tool::ToolKind;

/// One command-line seat override that could not be honored.
#[derive(Debug)]
pub(crate) enum SeatOverrideRefusal {
    /// The user named a profile or launch seat that does not exist, or asked a
    /// stopped conversation to cross harnesses.
    Usage(String),
    /// The selected config or profile command itself is not launchable.
    Failed(String),
}

/// Resolve one `profile@client` selection: both sides must exist, the pair
/// must substitute — and then the R1 gate: BOTH binaries must be the SAME
/// KNOWN harness. `Unknown` on either side refuses, because
/// `ToolKind::from_binary_name` is `from_known_binary_name(...).unwrap_or(Unknown)`
/// and any two unknowns would compare equal through it.
pub(crate) fn resolve_client_override(
    cfg: &IdentityConfig,
    agent: &str,
    profile: &str,
    label: &str,
    home: Option<&Path>,
    known_profiles: &str,
    known_clients: &str,
) -> Result<(config::ResolvedCommand, crate::launch_cmd::SimpleCommand), SeatOverrideRefusal> {
    let resolved = cfg
        .command_with_client(profile, label, home)
        .map_err(|why| match why {
            config::OverrideError::UnknownProfile => SeatOverrideRefusal::Usage(format!(
                "Error: unknown profile '{profile}' in --seat. Known profiles: {known_profiles}."
            )),
            config::OverrideError::UnknownClient => SeatOverrideRefusal::Usage(format!(
                "Error: unknown client '{label}' in --seat. Known clients: {known_clients}."
            )),
            config::OverrideError::ProfileNotSimple(why) => SeatOverrideRefusal::Failed(format!(
                "Error: [profiles] {profile} (seat '{agent}'): the launch command must be one simple command — it has {why}."
            )),
            config::OverrideError::Refused(why) => SeatOverrideRefusal::Failed(why.to_string()),
        })?;
    let parsed = crate::launch_cmd::lex_simple_command(resolved.command.as_str()).map_err(|why| {
        SeatOverrideRefusal::Failed(format!(
            "Error: [profiles] {profile} (seat '{agent}'): the launch command must be one simple command — it has {why}."
        ))
    })?;
    let original = ToolKind::from_known_binary_name(&resolved.original_binary);
    let switched = ToolKind::from_known_binary_name(&parsed.binary);
    if original.is_none() || switched.is_none() || original != switched {
        let from = match resolved.original_client.as_deref() {
            Some(client) => format!("profile '{profile}' (client '{client}')"),
            None => format!("profile '{profile}'"),
        };
        return Err(SeatOverrideRefusal::Usage(client_adapter_refusal(
            agent,
            profile,
            label,
            &from,
            (&resolved.original_binary, original),
            (&parsed.binary, switched),
        )));
    }
    Ok((resolved.command, parsed))
}

/// What one side of an `@` pair runs, for the refusal's two values.
fn adapter_word(binary: &str, known: Option<ToolKind>) -> String {
    match known {
        Some(kind) => kind.as_str().to_owned(),
        None => format!("an unrecognized binary '{binary}'"),
    }
}

fn client_adapter_refusal(
    agent: &str,
    profile: &str,
    label: &str,
    from: &str,
    original: (&str, Option<ToolKind>),
    switched: (&str, Option<ToolKind>),
) -> String {
    format!(
        "Error: cannot launch seat '{agent}' as '{profile}@{label}': {from} runs {} but client '{label}' runs {} — \
         a client override may change the account, never the harness adapter ('@' needs the same known harness on both sides). \
         Use a duplicate profile to cross harnesses.",
        adapter_word(original.0, original.1),
        adapter_word(switched.0, switched.1)
    )
}

/// A recorded `client.<slot>` row resolved against the CURRENT config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RecordedSelection {
    /// No row: no override was recorded for this seat.
    Missing,
    /// Exactly this label was recorded, and it still names a client.
    Label(String),
}

/// A recorded row no launch may interpret: empty, duplicated or malformed.
pub(crate) fn invalid_client_refusal(session: &str, agent: &str, slot: &str) -> String {
    format!(
        "Error: session '{session}' seat '{agent}' records an unusable client (client.{slot} is empty, duplicated or malformed) — \
         fix the meta row, or end the session (ae end {session})."
    )
}

/// A recorded label that selects a different client than this launch.
pub(crate) fn different_label_refusal(
    session: &str,
    agent: &str,
    profile: &str,
    known: &str,
    label: &str,
) -> String {
    format!(
        "Error: session '{session}' seat '{agent}' recorded client override '{known}' but this launch selects '{label}' — \
         a recorded client is write-once. Resume with '{profile}@{known}', restore the '{known}' client, or end the session (ae end {session})."
    )
}

/// Resolve one recorded seat's client row, or refuse with its remedy.
///
/// Shape-Invalid and config-absent are BOTH unusable, but only the absent
/// label has a cheap remedy: restoring the removed or renamed `[clients]` row
/// is safe, because the row then reads `Label` again with its identity
/// unchanged. A malformed row must be fixed by hand. Either way `ae end` is
/// the last resort, never a silent fallback.
pub(crate) fn recorded_selection(
    session: &str,
    cfg: &IdentityConfig,
    entry: &meta::RosterEntry,
) -> Result<RecordedSelection, String> {
    match &entry.client {
        crate::meta::RecordedClient::Missing => Ok(RecordedSelection::Missing),
        crate::meta::RecordedClient::Label(label) if cfg.client(label).is_some() => {
            Ok(RecordedSelection::Label(label.clone()))
        }
        crate::meta::RecordedClient::Label(label) => Err(format!(
            "Error: session '{session}' seat '{}' recorded client override '{label}' but no [clients] row names it now — \
             restore the '{label}' client, or end the session (ae end {session}).",
            entry.name
        )),
        crate::meta::RecordedClient::Invalid => {
            Err(invalid_client_refusal(session, &entry.name, &entry.slot))
        }
    }
}

/// Refuse an override the recorded seat cannot honor.
///
/// A recorded client is write-once: only its exact label is accepted, any
/// different label refuses whatever its facts (config is mutable, so an
/// equality proof cannot cross a config edit), and a bare profile cannot drop
/// it. Whatever the labels say, the recorded STORE triple is re-taken against
/// the current resolution, which catches a label whose definition moved. The
/// `bool` answers whether that re-take PROVED the pairing — `false` under an
/// `Unknown` current resolution, which honors the seat but records nothing.
#[allow(
    clippy::too_many_arguments,
    reason = "one refusal site for one override against one recorded seat"
)]
pub(crate) fn refuse_recorded_client_conflict(
    session: &str,
    agent: &str,
    profile: &str,
    client: Option<&str>,
    entry: &meta::RosterEntry,
    cfg: &IdentityConfig,
    command: &config::ResolvedCommand,
    tool: ToolKind,
    home: Option<&Path>,
) -> Result<bool, SeatOverrideRefusal> {
    let recorded = recorded_selection(session, cfg, entry).map_err(SeatOverrideRefusal::Failed)?;
    match (recorded, client) {
        // No recorded override and none selected: the legacy path, untouched.
        (RecordedSelection::Missing, None) => Ok(true),
        // First override for this seat: the recorded store still rules.
        (RecordedSelection::Missing, Some(label)) => refuse_store_conflict(
            session,
            agent,
            &format!("{profile}@{label}"),
            entry,
            command,
            tool,
            home,
        ),
        // A bare profile names no label, so it cannot satisfy write-once.
        (RecordedSelection::Label(known), None) => Err(SeatOverrideRefusal::Usage(format!(
            "Error: session '{session}' seat '{agent}' recorded client override '{known}' — \
             re-pair with '{profile}@{known}' to keep it, or end the session (ae end {session}). \
             A bare profile cannot drop a recorded client."
        ))),
        // The exact label, re-proved against the current store below.
        (RecordedSelection::Label(known), Some(label)) if known == label => refuse_store_conflict(
            session,
            agent,
            &format!("{profile}@{label}"),
            entry,
            command,
            tool,
            home,
        ),
        // Any different label refuses, whatever its facts today.
        (RecordedSelection::Label(known), Some(label)) => Err(SeatOverrideRefusal::Usage(
            different_label_refusal(session, agent, profile, &known, label),
        )),
    }
}

/// One side of the R4 identity comparison, as the refusal names it.
fn recorded_store_shown(
    home: &crate::meta::RecordedConfigHome,
    base: &crate::meta::RecordedConfigHomeBase,
) -> String {
    match home {
        crate::meta::RecordedConfigHome::Path(path) => {
            format!("{} (explicit)", path.display())
        }
        crate::meta::RecordedConfigHome::Implicit(path) => match base {
            crate::meta::RecordedConfigHomeBase::Path(home) => {
                format!("{} (implicit, base {})", path.display(), home.display())
            }
            _ => format!("{} (implicit)", path.display()),
        },
        crate::meta::RecordedConfigHome::Absent => "absent".to_owned(),
        crate::meta::RecordedConfigHome::Unknown
        | crate::meta::RecordedConfigHome::Invalid
        | crate::meta::RecordedConfigHome::Missing => "unusable".to_owned(),
    }
}

/// F6: an override whose current resolution is `Unknown` on a seat with no
/// retained store refuses — proceeding would ignore an explicit flag while
/// recording nothing recoverable, and a first start strands nothing.
pub(crate) fn unknown_store_refusal(
    agent: &str,
    spelling: &str,
    reason: &str,
) -> SeatOverrideRefusal {
    SeatOverrideRefusal::Usage(format!(
        "Error: cannot launch seat '{agent}' as '{spelling}': the override resolves to an unknown conversation store ({reason}) — \
         the launcher cannot verify where this conversation would live. Make the account path absolute, then retry."
    ))
}

/// Re-take the R4 identity check for one override: the recorded store triple
/// (MODE + PATH + BASE — never path alone, an implicit and an explicit row
/// over one path are different accounts) against the CURRENT resolution of
/// the command this launch would exec.
///
/// The lookup is controlled, not ambient: `HOME` is the launch home, every
/// other variable unset. An explicit client assigns its own variable, so its
/// resolution never consults the environment at all; an implicit one needs
/// exactly `HOME`. A CURRENT resolution of `Unknown` (a pane-only `$VAR` the
/// launcher cannot see) proves no conflict — `_run` retains the recorded
/// store with a notice, so the seat stays safe without this gate refusing —
/// but it proves no pairing either, and the `false` carries exactly that: the
/// caller honors the seat and records no new label fact. That mercy belongs
/// to a RESUME — a seat with a retained store to proceed on. A first start
/// (no recorded store) that resolves `Unknown` refuses outright instead: F6,
/// because proceeding would ignore an explicit flag and strand nothing.
pub(crate) fn refuse_store_conflict(
    session: &str,
    agent: &str,
    spelling: &str,
    entry: &meta::RosterEntry,
    command: &config::ResolvedCommand,
    tool: ToolKind,
    home: Option<&Path>,
) -> Result<bool, SeatOverrideRefusal> {
    if entry.config_home == crate::meta::RecordedConfigHome::Invalid {
        return Err(SeatOverrideRefusal::Failed(format!(
            "Error: cannot resume seat '{agent}' as '{spelling}': seat '{}' has malformed or duplicate config_home metadata — \
             fix the meta row, or end the session (ae end {session}).",
            entry.slot
        )));
    }
    if entry.config_home == crate::meta::RecordedConfigHome::Unknown {
        return Err(SeatOverrideRefusal::Failed(format!(
            "Error: cannot resume seat '{agent}' as '{spelling}': the recorded conversation store (config_home.{}) is unknown — \
             end the session and start over (ae end {session}).",
            entry.slot
        )));
    }
    let home_value = home.map(|path| path.display().to_string());
    let resolution = crate::launch_cmd::config_home_resolution(command, tool, &|name| {
        if name == "HOME" {
            home_value.clone()
        } else {
            None
        }
    });
    if entry.config_home == crate::meta::RecordedConfigHome::Missing {
        // First start for this seat: the override participates in the one
        // resolution and is recorded normally. But an `Unknown` resolution
        // refuses — F6: nothing retained means nothing stranded, and
        // proceeding would silently ignore the explicit flag.
        if let crate::launch_cmd::Resolved::Unknown(reason) = &resolution.home {
            return Err(unknown_store_refusal(agent, spelling, reason));
        }
        return Ok(true);
    }
    let current =
        crate::run::canonical_config_home(&resolution.home).map_err(SeatOverrideRefusal::Failed)?;
    let current_base =
        crate::run::canonical_config_home(&resolution.base).map_err(SeatOverrideRefusal::Failed)?;
    if matches!(current, crate::launch_cmd::Resolved::Unknown(_)) {
        return Ok(false);
    }
    let matches = match (&entry.config_home, &current) {
        (crate::meta::RecordedConfigHome::Absent, crate::launch_cmd::Resolved::Absent) => true,
        (
            crate::meta::RecordedConfigHome::Path(recorded),
            crate::launch_cmd::Resolved::Path(now),
        ) => resolution.explicit && recorded == now,
        (
            crate::meta::RecordedConfigHome::Implicit(recorded),
            crate::launch_cmd::Resolved::Path(now),
        ) => {
            !resolution.explicit
                && recorded == now
                && matches!(
                    (&entry.config_home_base, &current_base),
                    (
                        crate::meta::RecordedConfigHomeBase::Path(recorded),
                        crate::launch_cmd::Resolved::Path(now)
                    ) if recorded == now
                )
        }
        _ => false,
    };
    if matches {
        return Ok(true);
    }
    let now = match (&current, resolution.explicit) {
        (crate::launch_cmd::Resolved::Path(path), true) => {
            format!("{} (explicit)", path.display())
        }
        (crate::launch_cmd::Resolved::Path(path), false) => match &current_base {
            crate::launch_cmd::Resolved::Path(base) => {
                format!("{} (implicit, base {})", path.display(), base.display())
            }
            _ => format!("{} (implicit)", path.display()),
        },
        (crate::launch_cmd::Resolved::Absent, _) => "absent".to_owned(),
        (crate::launch_cmd::Resolved::Unknown(_), _) => "unresolvable".to_owned(),
    };
    // A store the pane selects through an exported account variable reads
    // exactly like a mode flip here: the launcher cannot see pane-only
    // environment, so the refusal names the escape — pinning the store in the
    // row makes the resolution observable and the refusal exact again.
    let exported_hint = if !resolution.explicit
        && matches!(entry.config_home, crate::meta::RecordedConfigHome::Path(_))
    {
        " If this seat's store is selected by an exported account variable the launcher cannot see, \
         pin it in the [clients] row as config_home=<path> instead."
    } else {
        ""
    };
    Err(SeatOverrideRefusal::Usage(format!(
        "Error: cannot resume seat '{agent}' as '{spelling}': the recorded conversation lives in {}, \
         but this launch resolves to {now} — a client override cannot move a retained conversation. \
         End the session to adopt the new account (ae end {session}).{exported_hint}",
        recorded_store_shown(&entry.config_home, &entry.config_home_base)
    )))
}

/// A resumed seat whose command now launches another tool than it recorded.
pub(crate) fn cross_tool_refusal(agent: &str, old: ToolKind, new: ToolKind) -> String {
    format!(
        "Error: cannot change agent '{agent}' from {} to {} on resume — its recorded conversation cannot cross tool kinds.",
        old.as_str(),
        new.as_str()
    )
}

/// Validate every explicit seat profile before the launch's first write.
/// Comma list of known names for a refusal, or `<none>` when empty.
pub(crate) fn known_list(names: &[&str]) -> String {
    if names.is_empty() {
        "<none>".to_owned()
    } else {
        names.join(", ")
    }
}

/// Whether `entry` is a LABELED spawned seat: a `spawned.<n>` row that records
/// a well-formed `client.<slot>` label. The one predicate every start of such
/// a seat asks — the resume, a relaunch, a snapshot-less `_run` and a reseat's
/// admission of it — before it judges the pair on the global config alone.
pub(crate) fn is_labeled_spawned(entry: &meta::RosterEntry) -> bool {
    entry.slot.starts_with("spawned.")
        && matches!(entry.client, crate::meta::RecordedClient::Label(_))
}

/// A meta value as an owned lossy string, empty when the key is absent: the
/// reading `lifecycle::meta_value` gives, kept here because the services layer
/// may not name `lifecycle`.
fn meta_text(bytes: &[u8], key: &str) -> String {
    meta::first_value(bytes, key).map_or_else(String::new, |value| {
        String::from_utf8_lossy(value).into_owned()
    })
}

/// The session's recorded GLOBAL identity, read from the meta `bytes` alone: the
/// only config a labeled spawned seat's pair was approved by, never an overlay.
///
/// # Errors
///
/// Why there is none to judge in: no `config` row, or a file that cannot be used.
pub(crate) fn global_authority(bytes: &[u8]) -> Result<IdentityConfig, String> {
    let global = meta_text(bytes, "config");
    if global.is_empty() {
        return Err("the session records no global config".to_owned());
    }
    config::read_identity(Some(Path::new(&global)), None)
        .map_err(|why| format!("its global config {global} cannot be used ({why})"))
}

/// The known profile and client names of `cfg`, as the adapter gate words them.
pub(crate) fn known_names(cfg: &IdentityConfig) -> (String, String) {
    let profiles: Vec<&str> = cfg.profiles.iter().map(|(name, _)| name.as_str()).collect();
    let clients: Vec<&str> = cfg.clients.iter().map(|(name, _)| name.as_str()).collect();
    (known_list(&profiles), known_list(&clients))
}

/// One labeled seat's judgement: the label it records and the command its pair
/// runs, `None` when the judging config defines no such profile.
#[derive(Debug, Clone)]
pub(crate) struct JudgedLabel {
    pub(crate) label: String,
    pub(crate) command: Option<config::ResolvedCommand>,
}

/// Judge ONE labeled spawned seat against `cfg`, the session's recorded GLOBAL
/// identity: the recorded client must still be a global `[clients]` row (else
/// the removed-client refusal, ahead of any missing profile); a profile the
/// global config lacks judges `command: None`; any other pair passes the
/// launch's adapter gate, the recorded store re-take and the recorded harness.
/// `None` for a seat that records no label, else its label and the judged
/// command. The resume keys its own restore identity off the label.
pub(crate) fn judge_spawned_label(
    session: &str,
    cfg: &IdentityConfig,
    entry: &meta::RosterEntry,
    home: Option<&Path>,
    (known_profiles, known_clients): &(String, String),
) -> Result<Option<JudgedLabel>, SeatOverrideRefusal> {
    let RecordedSelection::Label(label) =
        recorded_selection(session, cfg, entry).map_err(SeatOverrideRefusal::Failed)?
    else {
        return Ok(None);
    };
    let profile = entry.profile.as_deref().unwrap_or_default();
    let command = if cfg.profile(profile).is_none() {
        None
    } else {
        let (command, lexed) = resolve_client_override(
            cfg,
            &entry.name,
            profile,
            &label,
            home,
            known_profiles,
            known_clients,
        )?;
        refuse_recorded_client_conflict(
            session,
            &entry.name,
            profile,
            Some(&label),
            entry,
            cfg,
            &command,
            lexed.tool(),
            home,
        )?;
        let recorded = ToolKind::from_binary_name(entry.binary.as_deref().unwrap_or_default());
        if recorded != lexed.tool() {
            return Err(SeatOverrideRefusal::Usage(cross_tool_refusal(
                &entry.name,
                recorded,
                lexed.tool(),
            )));
        }
        Some(command)
    };
    Ok(Some(JudgedLabel { label, command }))
}

/// The command a LABELED spawned seat ([`is_labeled_spawned`]) may run when it
/// starts outside a resume — a relaunch, a `_run` with no command snapshot — and
/// the command a reseat's admission judges its source by: the resume's own
/// judgement ([`judge_spawned_label`]) on the session's recorded GLOBAL config
/// alone, never a project overlay. A profile that config lacks refuses here,
/// because only a resume can keep a seat unstarted.
///
/// # Errors
///
/// One clause (no `Error:` lead, no final period, for the caller's own frame)
/// naming the seat, its client, the judge's reason and both remedies.
pub(crate) fn labeled_spawned_command(
    bytes: &[u8],
    entry: &meta::RosterEntry,
    home: Option<&Path>,
) -> Result<config::ResolvedCommand, String> {
    let session = meta_text(bytes, "session");
    let label = match &entry.client {
        crate::meta::RecordedClient::Label(label) => label.as_str(),
        _ => "",
    };
    let refuse = |why: &str| {
        let why = why.trim();
        let why = why
            .strip_prefix("Error: ")
            .unwrap_or(why)
            .trim_end_matches('.');
        format!(
            "seat '{}' ({}) records client '{label}', which the session's global config no longer \
             runs: {why} — restore the global [clients] row or the recorded store, or retire the \
             seat and spawn it again",
            entry.name, entry.slot
        )
    };
    let refusal = |why: SeatOverrideRefusal| match why {
        SeatOverrideRefusal::Usage(line) | SeatOverrideRefusal::Failed(line) => refuse(&line),
    };
    let cfg = global_authority(bytes).map_err(|why| refuse(&why))?;
    match judge_spawned_label(&session, &cfg, entry, home, &known_names(&cfg)) {
        Ok(Some(JudgedLabel {
            command: Some(command),
            ..
        })) => Ok(command),
        Ok(Some(JudgedLabel { command: None, .. })) => Err(refuse(&format!(
            "its global config defines no profile '{}'",
            entry.profile.as_deref().unwrap_or_default()
        ))),
        Ok(None) => Err(refuse("it records no client label")),
        Err(why) => Err(refusal(why)),
    }
}
