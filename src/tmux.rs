//! Addressing a tmux server, and reading what it said.
//!
//! Everything here is PURE: argument derivation in, text interpretation out.
//! Running the child process is [`crate::transport`]'s job, and deliberately not
//! this module's. The split is what keeps the two decisions that can be WRONG —
//! WHICH server an argument list addresses, and WHAT a completed run means —
//! unit-testable without a process anywhere near them; the exec is a detail
//! around them, behind the one door `clippy.toml` opens in product code.

use std::path::Path;
use std::time::Duration;

use crate::inventory::{QueryFailed, ServerId};
use crate::meta::Selector;
use crate::theme::Mark;
mod queue;
#[cfg(test)]
use queue::queued_sections;
#[cfg(test)]
pub(crate) use queue::{COMMAND_BYTES, OwnershipAnswer, WAKE_HOOK};
pub(crate) use queue::{
    CycleBatch, CycleOp, FOCUS_HOOK, SessionLookRead, WAKE_HOOK_MARKER, WaitRead,
    channel_waiter_args, cycle_batch_chunks, interpret_motion_exchange, interpret_ownership_read,
    interpret_session_look_read, interpret_wait_read, is_wake_channel, motion_exchange_args,
    ownership_read_args, session_look_read_args, waiter_args, wake_channel, wake_release_args,
};
mod menu;
#[cfg(test)]
use menu::MENU_POSITION;
pub use menu::{
    FocusVerb, Menu, MenuAction, MenuItem, attach_args, confirm_before_args,
    display_client_message_args, display_menu_args, display_menu_centred_args,
    display_menu_for_client_args, display_message_args, display_settings_menu_args, focus_args,
    guarded_jump_client_id_command, guarded_jump_id_command, jump_client_command, jump_command,
    menu_literal, menu_run_shell_command, run_shell_background_args, run_shell_background_command,
    seat_jump_client_id_command, seat_jump_id_command, switch_client_args, switch_client_command,
    switch_client_id_command, switch_command, switch_id_command,
};
pub(crate) use menu::{
    MOUSE_DOWN_STATUS_MENU_ACTION, MOUSE_STATUS_PICKER, MOUSE_STATUS_SESSION,
    MOUSE_STATUS_SETTINGS, MOUSE_STATUS_WINDOW, hotkey_console_shell, hotkey_jump_shell,
    hotkey_picker_shell, hotkey_reader_shell, mouse_dispatch_literal, settings_menu_x,
    status_picker_command, status_settings_command, tmux_current_format_double_quote,
    tmux_double_quote,
};
mod views;
pub use views::{
    HolderRow, VIEW_CLIENT_OPTION, VIEW_HINT, VIEW_OF_OPTION, VIEW_RETURN_KEYS, VIEW_RETURN_OPTION,
    VIEW_SESSION_PREFIX, VIEW_TOKEN_OPTION, VIEW_UUID_OPTION, ViewCensusRow, interpret_view_census,
    interpret_view_client_place, interpret_view_holders, interpret_view_marker,
    interpret_view_place, interpret_view_uuids, interpret_window_snapshot, is_view_session,
    orphan_view_ids, pinned_pane_target, session_name_of_args, view_attach_args, view_census_args,
    view_client_place_args, view_command_args, view_destroy_args, view_group_args,
    view_holder_listing_args, view_marker_args, view_member_name, view_membership_args,
    view_of_args, view_of_value, view_place_args, view_release_command, view_restore_command,
    view_return_bind_args, view_return_command, view_session_age_nanos, view_session_expired,
    view_session_name, view_stale_clear_command, view_switch_args, view_uuid_listing_args,
    view_window_snapshot_args, views_holding, with_view_uuid,
};
mod fleet;
#[cfg(test)]
use fleet::{
    FLEET_LISTING_FIELDS, READ_CLIENTS, READ_DONE_STEM, READ_LOOK, READ_SESSIONS, READ_STEM,
    picker_agent_mark, read_section,
};
pub use fleet::{
    FLEET_LISTING_FORMAT, FLEET_PANE_FORMAT, FLEET_SESSION_FORMAT, FleetListingRow, FleetPane,
    FleetSession, MENU_CLIENT_FORMAT, MenuClient, PICKER_AGENTS_MAX_BYTES, PICKER_AGENTS_MAX_COUNT,
    PICKER_AGENTS_MAX_INTERVAL_SECS, PICKER_AGENTS_MAX_MODEL, PICKER_CLIENT_SESSION_FORMAT,
    PICKER_PANE_FORMAT, PICKER_SESSION_FORMAT, PickerAgent, PickerClient, PickerPane, PickerRead,
    PickerSession, SERVER_IDENTITY_FORMAT, ServerIdentity, fleet_listing_args, fleet_panes_args,
    fleet_sessions_args, interpret_fleet_listing, interpret_fleet_panes, interpret_fleet_sessions,
    interpret_menu_client, interpret_picker_client_session, interpret_picker_panes,
    interpret_picker_read, interpret_picker_sessions, interpret_server_identity, menu_clients_args,
    parse_picker_agents, picker_client_sessions_args, picker_panes_args, picker_read_args,
    picker_sessions_args, server_identity_args,
};
use fleet::{fleet_listing_words, is_session_id, pane_pid};
pub(crate) use fleet::{is_decimal, pane_id_is_valid, session_id_is_valid};

/// The format `list-sessions` is asked for: one exact session name per line.
pub const SESSION_NAME_FORMAT: &str = "#{session_name}";

/// Pin `name` to tmux's exact session-name rule.
///
/// A tmux target session resolves by exact match, then prefix, then fnmatch;
/// the leading `=` disables the latter two steps.
#[must_use]
pub(crate) fn session_target(name: &str) -> String {
    format!("={name}")
}

/// The separator every multi-field format in this module renders between its
/// fields, and so does `watchdog_glue`'s ownership probe.
pub(crate) const FIELD_SEPARATOR: &str = " | ";

/// The ae-ownership marker, read from a session's own tmux environment.
pub const OWNERSHIP_VARIABLE: &str = "AE_SESSION";

/// The state-root marker stamped into a session's own tmux environment.
pub const HOME_VARIABLE: &str = "AE_HOME";

/// The arguments that address `server`, before any subcommand.
#[must_use]
pub fn server_args(server: &ServerId) -> Vec<String> {
    match server {
        ServerId::Ambient => Vec::new(),
        ServerId::Selected(Selector::Name(name)) => vec!["-L".to_owned(), name.clone()],
        ServerId::Selected(Selector::Socket(path)) => {
            vec!["-S".to_owned(), path.display().to_string()]
        }
    }
}

/// The full argument list for enumerating `server`'s sessions.
#[must_use]
pub fn list_sessions_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.push("list-sessions".to_owned());
    args.push("-F".to_owned());
    args.push(SESSION_NAME_FORMAT.to_owned());
    args
}

/// The full argument list for reading one session's ownership marker.
#[must_use]
pub fn marker_args(server: &ServerId, session: &str) -> Vec<String> {
    environment_value_args(server, session, OWNERSHIP_VARIABLE)
}

/// The full argument list for reading one exact session environment value.
#[must_use]
pub fn environment_value_args(server: &ServerId, session: &str, variable: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.push("show-environment".to_owned());
    args.push("-t".to_owned());
    args.push(session_target(session));
    args.push(variable.to_owned());
    args
}

/// What a completed `list-sessions` run means.
///
/// # Errors
///
/// [`QueryFailed`] whenever the run did not succeed, whatever it printed. A
/// non-zero tmux is "no server running on …" as often as anything else, and
/// "there is no server" is not the same fact as "the server says there are no
/// sessions" — the first one is explicitly `unknown`.
pub fn interpret_sessions(succeeded: bool, stdout: &str) -> Result<Vec<String>, QueryFailed> {
    if !succeeded {
        return Err(QueryFailed);
    }
    Ok(stdout
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

/// What a stop-verification `list-sessions` proved about one session.
#[derive(Debug, PartialEq, Eq)]
pub enum StopProbe {
    /// The server answered and the session is STILL among its sessions.
    Present,
    /// Verified gone: the server answered without the name, OR it reported the
    /// stale-socket "no server running on …" diagnostic (a clean server exit —
    /// the socket file lingers, so the diagnostic, not socket-absence, is the
    /// proof).
    Absent,
    /// Unproven: the server could not be reached for any OTHER reason (ENOENT,
    /// permission, refused).
    Unknown,
}

/// What a completed stop-verification `list-sessions` means for `name`.
///
/// THE STRICT PROOF, and the one the destructive gates cross: a session is
/// `Absent` only because the server itself said so. A missing socket is
/// `Unknown` here and stays `Unknown` — [`classify_absence`] is the only reader
/// that may weigh it, and only for a resume or a listing.
#[must_use]
pub fn interpret_stopped(succeeded: bool, stdout: &str, stderr: &str, name: &str) -> StopProbe {
    match read_absence(succeeded, stdout, stderr, name) {
        Absence::Listed => StopProbe::Present,
        Absence::Proven => StopProbe::Absent,
        // STRICTLY the clean-exit diagnostic, and NOT the connect error the
        // version probe also reads as absence: a live server whose socket was
        // unlinked answers ENOENT while it keeps running, so ENOENT proves a
        // session gone only if you are willing to be wrong about it. The two
        // probes ask different questions — see [`says_no_server`].
        Absence::SocketMissing | Absence::Unreachable => StopProbe::Unknown,
    }
}

/// What a completed stop-verification `list-sessions` SAW, before anything is
/// concluded from it.
///
/// [`interpret_stopped`] folds the last two together, which is the right answer
/// for a destructive gate and the wrong one for a resume: after a host reboot
/// every socket under `/tmp/tmux-<uid>/` is gone, and that is a different
/// situation from a server that refused to talk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Absence {
    /// The server answered and the name is among its sessions.
    Listed,
    /// PROVEN gone by the server's own answer: it listed its sessions without
    /// the name, or it reported the stale-socket `no server running on …`
    /// diagnostic of a clean exit.
    Proven,
    /// The socket is not there at all (ENOENT). A live server whose socket was
    /// unlinked answers exactly this, so on its own it proves NOTHING.
    SocketMissing,
    /// Any other failure — permission denied, connection refused, no tmux.
    Unreachable,
}

/// What a completed stop-verification `list-sessions` saw about `name`.
#[must_use]
pub fn read_absence(succeeded: bool, stdout: &str, stderr: &str, name: &str) -> Absence {
    if succeeded {
        let present = stdout.lines().map(str::trim_end).any(|line| line == name);
        return if present {
            Absence::Listed
        } else {
            Absence::Proven
        };
    }
    read_failure(stderr)
}

/// What a FAILED run against a server means, from its diagnostic alone.
///
/// Split out because the fleet listing asks the server-level question — is this
/// socket there at all — without naming any session.
#[must_use]
pub fn read_failure(stderr: &str) -> Absence {
    let Some(line) = stderr.lines().next().map(str::trim_end) else {
        return Absence::Unreachable;
    };
    // Anchored at the line start, never a substring: a server literally NAMED
    // "no server running" once made a permission error CONTAIN the words.
    if line.starts_with(NO_SERVER_DIAGNOSTIC) {
        return Absence::Proven;
    }
    if line.starts_with(CONNECT_PREFIX) && line.ends_with(CONNECT_ABSENT_SUFFIX) {
        return Absence::SocketMissing;
    }
    Absence::Unreachable
}

/// What ae's OWN durable files say about when a session was last live — of ONE
/// source, or of all of them folded together.
///
/// Three states, not two, and the third is the point. A source that is simply
/// NOT THERE says nothing and is fine: every session written before the stamp
/// existed is in exactly that position. A source that IS there and cannot be
/// read is DAMAGE, and damage must never collapse into absence — otherwise a
/// newer claim nobody can read lets an older, readable one prove the session
/// gone, which is the one direction this proof may never fail in.
///
/// The facts behind it are gathered by [`crate::inventory::last_live`], which
/// owns the rule about which writes count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Evidence {
    /// Nothing here says when. Not there, or there and claiming nothing.
    #[default]
    Silent,
    /// The moment recorded, epoch seconds.
    At(i64),
    /// It IS there and could not be read, or it claims something that is not a
    /// moment.
    Unreadable,
}

/// The most a recorded moment may spell. A hostile file can claim an epoch of
/// any length, and an unbounded claim is rejected rather than parsed.
const CLAIM_CAP: usize = 32;

impl Evidence {
    /// One source's claim, read from the text it recorded.
    ///
    /// A moment is a STRICTLY POSITIVE epoch. Every other present spelling —
    /// empty, non-numeric, zero, negative, longer than any epoch — is a source
    /// that is there and says nothing readable, so it is damage. Only
    /// [`Self::floor_claim`] knows the one row whose zero is legal.
    ///
    /// PURE, and bounded: the cap is checked before the parse, so a claim of
    /// any size costs the same.
    ///
    /// ```
    /// use ae::tmux::Evidence;
    /// assert_eq!(Evidence::claim("1789105855\n"), Evidence::At(1_789_105_855));
    /// assert_eq!(Evidence::claim("tomorrow"), Evidence::Unreadable);
    /// assert_eq!(Evidence::claim(""), Evidence::Unreadable);
    /// // No source but a capture floor may spell a non-positive epoch.
    /// assert_eq!(Evidence::claim("0"), Evidence::Unreadable);
    /// assert_eq!(Evidence::claim("-1"), Evidence::Unreadable);
    /// ```
    #[must_use]
    pub fn claim(text: &str) -> Self {
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed.len() > CLAIM_CAP {
            return Self::Unreadable;
        }
        match trimmed.parse::<i64>() {
            Ok(epoch) if epoch > 0 => Self::At(epoch),
            _ => Self::Unreadable,
        }
    }

    /// A CAPTURE FLOOR's claim — the one source whose zero is documented.
    ///
    /// `capture_floor.<slot>=0` is published by a retained exact resume whose
    /// legacy conversation has no known origin
    /// ([`crate::session_launch`] writes it). It means "no floor", says nothing
    /// about a launch, and must stay silent for liveness: reading it as damage
    /// would strand every such session after a reboot. A NEGATIVE floor is not
    /// that sentinel, and is damage like any other unreadable claim.
    ///
    /// ```
    /// use ae::tmux::Evidence;
    /// assert_eq!(Evidence::floor_claim("0"), Evidence::Silent);
    /// assert_eq!(Evidence::floor_claim("1789105855"), Evidence::At(1_789_105_855));
    /// assert_eq!(Evidence::floor_claim("-1"), Evidence::Unreadable);
    /// assert_eq!(Evidence::floor_claim(""), Evidence::Unreadable);
    /// ```
    #[must_use]
    pub fn floor_claim(text: &str) -> Self {
        if text.trim() == "0" {
            return Self::Silent;
        }
        Self::claim(text)
    }

    /// A file's mtime as evidence — the reading for the two sources whose
    /// moment is an mtime and nothing else.
    ///
    /// A clock the filesystem cannot express as an epoch is DAMAGE, not
    /// silence: the file is there, and what it says cannot be read.
    #[must_use]
    pub fn at_mtime(modified: std::io::Result<std::time::SystemTime>) -> Self {
        let epoch = modified
            .ok()
            .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|since| i64::try_from(since.as_secs()).ok());
        match epoch {
            Some(epoch) if epoch > 0 => Self::At(epoch),
            _ => Self::Unreadable,
        }
    }

    /// Fold every source's reading into the session's one answer.
    ///
    /// ONE damaged source is enough to make the whole answer damaged. Otherwise
    /// the NEWEST moment wins, because more evidence may only ever move the
    /// answer later, and a later answer only ever refuses.
    ///
    /// ```
    /// use ae::tmux::Evidence;
    /// assert_eq!(
    ///     Evidence::folded(&[Evidence::At(10), Evidence::Silent, Evidence::At(20)]),
    ///     Evidence::At(20)
    /// );
    /// assert_eq!(
    ///     Evidence::folded(&[Evidence::At(10), Evidence::Unreadable]),
    ///     Evidence::Unreadable,
    ///     "a damaged newer source must not leave an older readable one proving absence"
    /// );
    /// assert_eq!(Evidence::folded(&[]), Evidence::Silent);
    /// ```
    #[must_use]
    pub fn folded(readings: &[Self]) -> Self {
        let mut newest = Self::Silent;
        for reading in readings {
            newest = newest.and(*reading);
        }
        newest
    }

    /// This reading and `other`, folded — the step [`Self::folded`] repeats,
    /// exposed so a reducer over a stream folds in constant space.
    #[must_use]
    pub fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::Unreadable, _) | (_, Self::Unreadable) => Self::Unreadable,
            (Self::At(left), Self::At(right)) => Self::At(left.max(right)),
            (Self::At(only), Self::Silent) | (Self::Silent, Self::At(only)) => Self::At(only),
            (Self::Silent, Self::Silent) => Self::Silent,
        }
    }
}

/// The BOOT-TIME proof, for a resume and a listing — never for a stop, an end
/// or a compact.
///
/// A missing socket means one of two things: the server that held it is gone
/// (a reboot took every socket under `/tmp/tmux-<uid>/` with it), or it is
/// still running and something unlinked its socket. The second is why
/// [`interpret_stopped`] refuses to conclude anything. ae can separate them for
/// ONE session at a time: it rewrites this session's own live facts on every
/// launch and resume, so a session ae has not touched since the host booted
/// cannot be sitting on a server that was started after the host booted. That
/// makes the ABSENCE this session's, not the server's — a foreign tmux on the
/// same socket path, holding a same-named session ae never launched, is the
/// accepted and documented residual.
///
/// Every incomplete or contradictory reading is [`StopProbe::Unknown`]: no boot
/// time, no recorded activity, activity at or after the boot, or a boot time in
/// the future.
#[must_use]
pub fn classify_absence(
    probe: Absence,
    evidence: Evidence,
    boot: Option<i64>,
    now: i64,
) -> StopProbe {
    match probe {
        Absence::Listed => StopProbe::Present,
        Absence::Proven => StopProbe::Absent,
        Absence::Unreachable => StopProbe::Unknown,
        Absence::SocketMissing => match (boot, evidence) {
            (Some(boot), Evidence::At(last_live)) if boot <= now && last_live < boot => {
                StopProbe::Absent
            }
            _ => StopProbe::Unknown,
        },
    }
}

/// Why [`classify_absence`] could not prove `server` gone, or `None` when it
/// proved something.
///
/// One line, and it names the evidence rather than the verdict: a human who
/// meets this refusal has to be able to tell "the clock is wrong" from "this
/// session really was alive after the reboot".
#[must_use]
pub fn unproven_reason(
    server: &str,
    probe: Absence,
    evidence: Evidence,
    boot: Option<i64>,
    now: i64,
) -> Option<String> {
    if classify_absence(probe, evidence, boot, now) != StopProbe::Unknown {
        return None;
    }
    let iso = |epoch: i64| crate::time::Timestamp::from_epoch(epoch).to_string();
    let why = match probe {
        Absence::Listed | Absence::Proven => return None,
        Absence::Unreachable => "the server could not be reached".to_owned(),
        Absence::SocketMissing => {
            let head = "socket missing; ";
            match (boot, evidence) {
                // The damaged reading comes FIRST: it is true whatever the boot
                // time says, and it is the one a human can actually repair.
                (_, Evidence::Unreadable) => format!(
                    "{head}a record of this session's own activity is there and could not be read"
                ),
                (None, _) => format!("{head}the host's boot time could not be read"),
                (Some(boot), _) if boot > now => {
                    format!("{head}boot {} is in the future (clock skew)", iso(boot))
                }
                (Some(_), Evidence::Silent) => {
                    format!("{head}this session has no recorded live activity")
                }
                (Some(boot), Evidence::At(last_live)) => format!(
                    "{head}last live activity {} is not before boot {}",
                    iso(last_live),
                    iso(boot)
                ),
            }
        }
    };
    Some(format!(
        "recorded server {server}: {why} — cannot prove the session gone. \
         Run 'ae doctor' for the evidence."
    ))
}

/// tmux's diagnostic after a server exited cleanly and left its socket behind.
const NO_SERVER_DIAGNOSTIC: &str = "no server running on ";

/// tmux's diagnostic when the socket is not there at all — measured on 3.7b for
/// both selectors: `-L nosuch` and `-S /nosuch/sock` each answer
/// `error connecting to <path> (No such file or directory)`.
const CONNECT_PREFIX: &str = "error connecting to ";

/// The errno tail that makes a connect failure an ABSENCE rather than an
/// unknown. Permission denied and connection refused are neither, and each
/// leaves the server's version unproven.
const CONNECT_ABSENT_SUFFIX: &str = "(No such file or directory)";

/// Whether `stderr`'s first line says, in either of tmux's two spellings, that
/// there is NO server on that socket.
///
/// Read by the VERSION probe alone, whose question is "which tmux binary will
/// this launch actually run". A socket that cannot be connected to cannot be
/// launched into either, so the launch will start a fresh server with the
/// `PATH` binary — which is the version to compare. The STOP probe asks a
/// different question ("is that session gone") and deliberately reads the same
/// ENOENT as UNKNOWN, because a live server whose socket was unlinked answers
/// it while still running.
///
/// ```
/// use ae::tmux::says_no_server;
/// assert!(says_no_server("no server running on /tmp/s\n"));
/// assert!(says_no_server("error connecting to /tmp/s (No such file or directory)\n"));
/// assert!(!says_no_server("error connecting to /tmp/s (Permission denied)\n"));
/// assert!(!says_no_server("some other failure\n"));
/// ```
#[must_use]
pub fn says_no_server(stderr: &str) -> bool {
    matches!(
        read_failure(stderr),
        Absence::Proven | Absence::SocketMissing
    )
}

/// The ownership marker a completed `show-environment` run reported.
#[must_use]
pub fn interpret_marker(succeeded: bool, stdout: &str) -> Option<String> {
    if !succeeded {
        return None;
    }
    stdout
        .lines()
        .find_map(|line| {
            line.trim_end()
                .strip_prefix(&format!("{OWNERSHIP_VARIABLE}="))
        })
        .map(ToOwned::to_owned)
}

/// One exact value from a successful single-variable `show-environment`.
///
/// More than one output record is refused. In particular, a newline inside a
/// hostile value cannot turn its first line into proof while hiding trailing
/// material on another line.
#[must_use]
pub fn interpret_environment_value(
    succeeded: bool,
    stdout: &str,
    variable: &str,
) -> Option<String> {
    if !succeeded {
        return None;
    }
    let line = stdout.strip_suffix('\n').unwrap_or(stdout);
    if line.contains('\n') {
        return None;
    }
    line.strip_prefix(&format!("{variable}="))
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

/// The format `list-panes` is asked for: four fields per pane, pipe-separated.
pub const PANE_FORMAT: &str =
    "#{pane_dead} | #{@ae_slot} | #{@ae_observed} | #{pane_current_command}";

/// How many [`FIELD_SEPARATOR`]-separated fields [`PANE_FORMAT`] produces per pane.
pub const PANE_FIELDS: usize = 4;

/// The full argument list for enumerating one session's panes.
#[must_use]
pub fn list_panes_args(server: &ServerId, session: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.push("list-panes".to_owned());
    args.push("-s".to_owned());
    args.push("-t".to_owned());
    args.push(session_target(session));
    args.push("-F".to_owned());
    args.push(PANE_FORMAT.to_owned());
    args
}

/// One pane the server reported, as four readings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedPane {
    /// Valid pane id from the all-panes listing, absent from per-session reads.
    pub pane_id: Option<String>,
    /// `#{pane_pid}` from the all-panes listing: the pane's root process,
    /// never the tool's. Absent from per-session reads.
    pub pid: Option<u32>,
    /// `#{pane_dead}` — `Some(true)` for a dead pane, `Some(false)` for a live
    /// one, `None` when the field was not a readable `0`/`1`.
    pub dead: Option<bool>,
    /// The `@ae_slot` value, when the pane carries a usable one.
    pub slot: Option<String>,
    /// `#{pane_current_command}`, when the field carried one.
    pub command: Option<String>,
    /// The watchdog's positive current-frame observation.
    pub observed: crate::harness_state::HarnessState,
}

/// What a completed `list-panes` run means.
///
/// # Errors
///
/// [`QueryFailed`] whenever the run did not succeed, whatever it printed —
/// A failed pane query goes to `unknown`, exactly as a failed session query
/// does. Measured: a `-t` naming no session exits 1.
///
/// **EVERY LINE IS A PANE.** A line that does not split into exactly
/// [`PANE_FIELDS`] fields still yields an [`ObservedPane`] — one with no usable
/// reading at all — rather than being dropped. Dropping it would delete the
/// pane whose existence is what keeps a missing roster agent `unknown` instead
/// of `dead`.
///
/// **ARITY IS EXACT, AND THAT IS A GUARD RATHER THAN TIDINESS.** None of the
/// three fields may legitimately contain [`FIELD_SEPARATOR`], so a line with more than
/// [`PANE_FIELDS`] fields is a reading nothing should trust. It matters because
/// a slot carrying an embedded separator could otherwise split into a PREFIX that
/// matches a real roster slot while pushing the rest of that slot into the
/// command field — forging a non-shell command for a pane that is running a
/// shell, which is a fabricated `alive` for the wrong agent. Refusing the whole
/// line answers `unknown` instead, which is the direction that cannot
/// assert.
pub fn interpret_panes(succeeded: bool, stdout: &str) -> Result<Vec<ObservedPane>, QueryFailed> {
    if !succeeded {
        return Err(QueryFailed);
    }
    Ok(stdout.lines().map(read_pane).collect())
}

/// One enumeration line as an [`ObservedPane`].
fn read_pane(line: &str) -> ObservedPane {
    let blank = ObservedPane {
        pane_id: None,
        pid: None,
        dead: None,
        slot: None,
        command: None,
        observed: crate::harness_state::HarnessState::Unknown,
    };
    let fields: Vec<&str> = line.trim_end_matches('\r').split(FIELD_SEPARATOR).collect();
    <[&str; PANE_FIELDS]>::try_from(fields.as_slice()).map_or(blank, pane_from_cells)
}

/// The four cells of [`PANE_FORMAT`] as an [`ObservedPane`].
fn pane_from_cells(cells: [&str; PANE_FIELDS]) -> ObservedPane {
    let usable = |value: &str| Some(value.to_owned()).filter(|v| !v.is_empty());
    ObservedPane {
        pane_id: None,
        pid: None,
        // `0`/`1` and nothing else.
        dead: match cells[0] {
            "0" => Some(false),
            "1" => Some(true),
            _ => None,
        },
        slot: usable(cells[1].trim_end()),
        command: usable(cells[3].trim_end()),
        observed: crate::harness_state::observed_from_option(cells[2].trim_end()),
    }
}

/// The format `list-panes -a` is asked for: the exact session name, the branch
/// that session published, then the four [`PANE_FORMAT`] cells, the pane id and its pid. The session
/// name is its own whole field, so a pane is attributed by equality and never by
/// containment.
pub const ALL_PANES_FORMAT: &str = "#{session_name} | #{@ae_branch_name} | #{pane_dead} | #{@ae_slot} | #{@ae_observed} | #{pane_current_command} | #{pane_id} | #{pane_pid}";

/// How many [`FIELD_SEPARATOR`]-separated fields [`ALL_PANES_FORMAT`] produces per row.
const ALL_PANES_FIELDS: usize = PANE_FIELDS + 4;

/// The full argument list for enumerating every pane on `server`.
#[must_use]
pub fn list_all_panes_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-panes", "-a", "-F", ALL_PANES_FORMAT].map(ToOwned::to_owned));
    args
}

/// One session's rows out of a `list-panes -a`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPanes {
    /// The branch the session published, when a row of it carried one.
    pub branch: Option<String>,
    /// Its panes, in the order the server listed them.
    pub panes: Vec<ObservedPane>,
}

/// Every session's panes out of ONE `list-panes -a`, keyed by exact session name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerPanes(std::collections::BTreeMap<String, SessionPanes>);

impl ServerPanes {
    /// The rows of the session named EXACTLY `name`, or `None` when no row
    /// names it — the same reading a failed `list-panes -t =name` gives.
    #[must_use]
    pub fn session(&self, name: &str) -> Option<&SessionPanes> {
        self.0.get(name)
    }
}

/// What a completed `list-panes -a` run means.
///
/// # Errors
///
/// [`QueryFailed`] whenever the run did not succeed, whatever it printed.
///
/// **A ROW IS KEPT ONLY WHOLE.** It must split into exactly [`ALL_PANES_FIELDS`]
/// fields with a non-empty session name; anything else is dropped, because a
/// row that does not parse cannot be attributed to a session. That happens to a
/// non-ae session named with [`FIELD_SEPARATOR`], and it can happen to an ae
/// pane whose cell carries one. [`interpret_panes`] keeps such a line as a blank
/// pane because its session is known from the target; here it is not. So an ae
/// session with one dropped row and another good row can read a missing roster
/// slot as dead where the per-session read called it unknown. That residual is
/// accepted: R3 forbids attributing a row we cannot prove.
/// An invalid pane id or pid leaves every other fact intact. A command carrying
/// the separator still drops the row, as it did before the id cell was added.
pub fn interpret_all_panes(succeeded: bool, stdout: &str) -> Result<ServerPanes, QueryFailed> {
    if !succeeded {
        return Err(QueryFailed);
    }
    let mut sessions = std::collections::BTreeMap::<String, SessionPanes>::new();
    for line in stdout.lines() {
        let fields: Vec<&str> = line.trim_end_matches('\r').split(FIELD_SEPARATOR).collect();
        let Ok([name, branch, cells @ .., pane_id, pid]) =
            <[&str; ALL_PANES_FIELDS]>::try_from(fields.as_slice())
        else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let held = sessions.entry(name.to_owned()).or_insert(SessionPanes {
            branch: None,
            panes: Vec::new(),
        });
        if held.branch.is_none() {
            held.branch = Some(branch.trim_end().to_owned()).filter(|value| !value.is_empty());
        }
        let mut pane = pane_from_cells(cells);
        pane.pane_id = pane_id_is_valid(pane_id).then(|| pane_id.to_owned());
        pane.pid = pane_pid(pid);
        held.panes.push(pane);
    }
    Ok(ServerPanes(sessions))
}

/// What an enumeration says about one roster slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotObservation {
    /// Exactly one observed pane carries this slot.
    Unique,
    /// More than one does, so the association is ambiguous.
    Duplicated {
        /// How many panes carry it.
        panes: usize,
    },
    /// No observed pane carries this slot.
    Absent {
        /// How many observed panes carry no usable marker at all.
        unidentified: usize,
    },
}

/// What `panes` says about `slot`.
#[must_use]
pub fn slot_observation(panes: &[ObservedPane], slot: &str) -> SlotObservation {
    let carrying = panes
        .iter()
        .filter(|pane| pane.slot.as_deref() == Some(slot))
        .count();
    match carrying {
        0 => SlotObservation::Absent {
            unidentified: panes.iter().filter(|pane| pane.slot.is_none()).count(),
        },
        1 => SlotObservation::Unique,
        panes => SlotObservation::Duplicated { panes },
    }
}

/// Whether `path` can address a tmux server at all.
#[must_use]
pub fn is_addressable_socket(path: &Path) -> bool {
    path.is_absolute()
}

/// The format the viewer query asks for: routing slot, tmux session, display
/// ref, `@ae_session_uuid`, and `#{socket_path}` — five readings in ONE round
/// trip. The socket path is the server's own spelling, so a symlink alias and
/// a replacement between two queries cannot splice two identities.
pub const VIEWER_FORMAT: &str =
    "#{@ae_slot} | #{session_name} | #{@ae_agent} | #{@ae_session_uuid} | #{socket_path}";

/// The number of fields [`VIEWER_FORMAT`] yields.
const VIEWER_FIELDS: usize = 5;

/// The arguments that read [`VIEWER_FORMAT`] off `pane` on `server`.
#[must_use]
pub fn viewer_args(server: &ServerId, pane: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["display-message", "-p", "-t", pane, VIEWER_FORMAT].map(ToOwned::to_owned));
    args
}

/// The calling pane's identity readings from one display-message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedViewer {
    /// `@ae_slot` — the routing key, unvalidated here.
    pub slot: Option<String>,
    /// `#{session_name}` of the pane.
    pub session: Option<String>,
    /// `@ae_agent` — the display `alias:name`.
    pub agent: Option<String>,
    /// `@ae_session_uuid` from the SAME pane-keyed query. A successful query
    /// yields [`OptionReading::Set`] or [`OptionReading::Vacant`] — never
    /// [`OptionReading::Unknown`], which is only the failed-query case
    /// ([`interpret_viewer`] returning `None`).
    pub session_uuid: OptionReading,
    /// `#{socket_path}` from the SAME pane-keyed query. The server's own
    /// spelling, independent of the selector used to address it.
    pub socket_path: Option<String>,
}

impl Default for ObservedViewer {
    fn default() -> Self {
        Self {
            slot: None,
            session: None,
            agent: None,
            session_uuid: OptionReading::Unknown,
            socket_path: None,
        }
    }
}

/// What a completed viewer query means.
#[must_use]
pub fn interpret_viewer(succeeded: bool, stdout: &str) -> Option<ObservedViewer> {
    if !succeeded {
        return None;
    }
    let line = stdout.strip_suffix('\n').unwrap_or(stdout);
    if line.contains('\n') {
        return None;
    }
    // The LAST field is the socket path, and a VALID absolute socket path may
    // itself contain the separator: split at the first four separators only,
    // so the path is never cut. Fewer than four separators cannot be five
    // fields and is refused.
    let fields: Vec<&str> = line.splitn(VIEWER_FIELDS, FIELD_SEPARATOR).collect();
    if fields.len() != VIEWER_FIELDS {
        return None;
    }
    let reading = |field: &str| (!field.is_empty()).then(|| field.to_owned());
    Some(ObservedViewer {
        slot: reading(fields[0]),
        session: reading(fields[1]),
        agent: reading(fields[2]),
        session_uuid: if fields[3].is_empty() {
            OptionReading::Vacant
        } else {
            OptionReading::Set(fields[3].to_owned())
        },
        socket_path: reading(fields[4]),
    })
}

/// The arguments for the resolver's exact session check — `tmux has-session
/// -t =<session>` before a cross-session lookup.
#[must_use]
pub fn has_session_args(server: &ServerId, session: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["has-session", "-t"].map(ToOwned::to_owned));
    args.push(session_target(session));
    args
}

/// The arguments for the lifecycle kill — `tmux kill-session -t <session-id>`.
#[must_use]
pub fn kill_session_args(server: &ServerId, session_id: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["kill-session", "-t"].map(ToOwned::to_owned));
    args.push(session_target(session_id));
    args
}

/// The roster the name resolver reads:
/// `list-panes -s -t <session> -F '#{pane_id} | #{@ae_agent}'`.
pub const AGENTS_FORMAT: &str = "#{pane_id} | #{@ae_agent}";

/// The full argument list for that roster.
#[must_use]
pub fn agents_args(server: &ServerId, session: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-panes", "-s", "-t"].map(ToOwned::to_owned));
    args.push(session_target(session));
    args.extend(["-F", AGENTS_FORMAT].map(ToOwned::to_owned));
    args
}

/// The roster the slot resolver reads: `list-panes -s -t <session>
/// -F '#{pane_id}|#{@ae_slot}|#{@ae_agent}'` — `|`, not tab, because the middle
/// field is empty on an unstamped pane and tab is an IFS whitespace character
/// there.
pub const SLOTS_FORMAT: &str = "#{pane_id}|#{@ae_slot}|#{@ae_agent}";

/// The full argument list for that roster.
#[must_use]
pub fn slots_args(server: &ServerId, session: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-panes", "-s", "-t"].map(ToOwned::to_owned));
    args.push(session_target(session));
    args.extend(["-F", SLOTS_FORMAT].map(ToOwned::to_owned));
    args
}

/// One pane of the slot roster: its id, its `@ae_slot` stamp and its
/// `@ae_agent` stamp, each empty when unset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedSlot {
    /// `#{pane_id}`.
    pub pane: String,
    /// `@ae_slot`, or empty.
    pub slot: String,
    /// `@ae_agent`, or empty.
    pub agent: String,
}

/// What a completed slot-roster run means: `None` when the run failed (the
/// frozen loop reads nothing from a `2>/dev/null` failure); otherwise every
/// line split at its first two `|`, in the order tmux printed.
///
/// ```
/// use ae::tmux::{ObservedSlot, interpret_slots};
///
/// let rows = interpret_slots(true, "%1|main|cl:lead\n%2||\n").unwrap();
/// assert_eq!(rows[0], ObservedSlot { pane: "%1".into(), slot: "main".into(), agent: "cl:lead".into() });
/// assert_eq!(rows[1], ObservedSlot { pane: "%2".into(), slot: String::new(), agent: String::new() });
/// assert!(interpret_slots(false, "").is_none());
/// ```
#[must_use]
pub fn interpret_slots(succeeded: bool, stdout: &str) -> Option<Vec<ObservedSlot>> {
    if !succeeded {
        return None;
    }
    Some(
        stdout
            .lines()
            .map(|line| {
                let (pane, rest) = line.split_once('|').unwrap_or((line, ""));
                let (slot, agent) = rest.split_once('|').unwrap_or((rest, ""));
                ObservedSlot {
                    pane: pane.to_owned(),
                    slot: slot.to_owned(),
                    agent: agent.to_owned(),
                }
            })
            .collect(),
    )
}

/// One pane of the roster: its id and its `@ae_agent` stamp (empty when the
/// pane is unstamped, which matches nothing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedAgent {
    /// `#{pane_id}`, e.g. `%3`.
    pub pane: String,
    /// `@ae_agent`, the display `alias:name`, or empty.
    pub agent: String,
}

/// What a completed roster run means: `None` when the run failed, which
/// resolves nothing;
/// otherwise every line, split at its FIRST [`FIELD_SEPARATOR`], in the order
/// tmux printed.
///
/// ```
/// use ae::tmux::{ObservedAgent, interpret_agents};
///
/// let rows = interpret_agents(true, "%1 | cl:lead\n%2 | \n").unwrap();
/// assert_eq!(rows[0], ObservedAgent { pane: "%1".into(), agent: "cl:lead".into() });
/// assert_eq!(rows[1], ObservedAgent { pane: "%2".into(), agent: String::new() });
/// assert!(interpret_agents(false, "%1 | cl:lead\n").is_none());
/// ```
#[must_use]
pub fn interpret_agents(succeeded: bool, stdout: &str) -> Option<Vec<ObservedAgent>> {
    if !succeeded {
        return None;
    }
    Some(
        stdout
            .lines()
            .map(|line| {
                let (pane, agent) = line.split_once(FIELD_SEPARATOR).unwrap_or((line, ""));
                ObservedAgent {
                    pane: pane.to_owned(),
                    agent: agent.to_owned(),
                }
            })
            .collect(),
    )
}

/// The watchdog's per-pane reading — richer than [`PANE_FORMAT`]'s liveness
/// three, and deliberately its OWN format so widening it never touches the
/// contract that [`interpret_panes`] answers.
const WATCH_PANE_SEPARATOR: &str = FIELD_SEPARATOR;

/// The `-F` string itself, in the field order [`interpret_watch_panes`] reads.
pub const WATCH_PANE_FORMAT: &str = "#{pane_id} | #{@ae_slot} | #{@ae_agent} | #{pane_pid} | #{@ae_observed} | #{pane_current_command}";

/// The number of fields [`WATCH_PANE_FORMAT`] yields.
const WATCH_PANE_FIELDS: usize = 6;

/// The arguments enumerating every pane of `session` on `server` for the
/// watchdog — `list-panes -s -t <session> -F <WATCH_PANE_FORMAT>`, widened to
/// the fields the cycle reads.
#[must_use]
pub fn watch_panes_args(server: &ServerId, session: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-panes", "-s", "-t"].map(ToOwned::to_owned));
    args.push(session_target(session));
    args.extend(["-F", WATCH_PANE_FORMAT].map(ToOwned::to_owned));
    args
}

/// One pane as the watchdog reads it: its id, its `@ae_slot`/`@ae_agent` stamps
/// (empty -> `None`), its foreground command, and its pid (empty or unparseable
/// -> `None`, which the dead-check treats as no usable descendant probe rather
/// than a guess).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchPane {
    /// `#{pane_id}`, e.g. `%3`.
    pub pane_id: String,
    /// `@ae_slot`, or `None` when unstamped.
    pub slot: Option<String>,
    /// `@ae_agent`, the display `alias:name`, or `None` when unstamped.
    pub agent: Option<String>,
    /// `#{pane_current_command}`.
    pub current_command: String,
    /// `#{pane_pid}`, or `None` when tmux printed no parseable pid.
    pub pane_pid: Option<u32>,
    /// The watchdog-owned raw current-frame option, including idle carry.
    pub observed: String,
}

/// What a completed watchdog enumeration means: `None` on a failed run or an
/// untrusted successful reading; otherwise one [`WatchPane`] per line. Every
/// non-empty line must split into exactly [`WATCH_PANE_FIELDS`] fields. The
/// split is limited to that arity so the free-text command remains intact. A
/// line that does not is a parse failure of the whole reading, not a dropped
/// pane: silently dropping it could turn a present pane into a false `Hard`
/// verdict.
///
/// ```
/// use ae::tmux::{WatchPane, interpret_watch_panes};
///
/// let sep = " | ";
/// let out = format!("%1{sep}main{sep}cl:lead{sep}4321{sep}idle:1:0:0:2{sep}claude\n%2{sep}{sep}{sep}88{sep}{sep}zsh\n");
/// let panes = interpret_watch_panes(true, &out).unwrap();
/// assert_eq!(panes[0], WatchPane {
///     pane_id: "%1".into(), slot: Some("main".into()), agent: Some("cl:lead".into()),
///     current_command: "claude".into(), pane_pid: Some(4321),
///     observed: "idle:1:0:0:2".into(),
/// });
/// assert_eq!(panes[1].slot, None);
/// assert_eq!(panes[1].agent, None);
/// assert_eq!(panes[1].pane_pid, Some(88));
/// assert!(interpret_watch_panes(false, "").is_none());
/// ```
#[must_use]
pub fn interpret_watch_panes(succeeded: bool, stdout: &str) -> Option<Vec<WatchPane>> {
    if !succeeded {
        return None;
    }
    let reading = |field: &str| (!field.is_empty()).then(|| field.to_owned());
    let mut panes = Vec::new();
    for line in stdout.lines() {
        let fields: Vec<&str> = line
            .splitn(WATCH_PANE_FIELDS, WATCH_PANE_SEPARATOR)
            .collect();
        if fields.len() != WATCH_PANE_FIELDS {
            return None;
        }
        panes.push(WatchPane {
            pane_id: fields[0].to_owned(),
            slot: reading(fields[1]),
            agent: reading(fields[2]),
            current_command: fields[5].to_owned(),
            pane_pid: fields[3].parse::<u32>().ok(),
            observed: fields[4].to_owned(),
        });
    }
    if !stdout.is_empty() && panes.is_empty() {
        return None;
    }
    Some(panes)
}

/// The ticker's one observation: identity and whether anybody is attached to
/// the session.
pub(crate) const MOTION_PANE_FORMAT: &str = "#{pane_id} | #{@ae_agent} | #{session_attached}";

/// The number of fields [`MOTION_PANE_FORMAT`] yields.
const MOTION_PANE_FIELDS: usize = 3;

/// One pane as the motion ticker reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MotionPane {
    /// `#{pane_id}`, e.g. `%3`.
    pub pane_id: String,
    /// `@ae_agent`, or `None` when unstamped.
    pub agent: Option<String>,
    /// Clients attached to this session.
    pub session_attached: u32,
}

/// The ticker's `list-panes -s -t <session> -F <MOTION_PANE_FORMAT>` argv, as
/// one invocation of its own. Production reads it inside the motion exchange
/// ([`motion_exchange_args`]); tests read it alone.
#[cfg(test)]
#[must_use]
pub(crate) fn motion_panes_args(server: &ServerId, session: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(motion_panes_words(session));
    args
}

/// The `list-panes -s -t <session> -F <MOTION_PANE_FORMAT>` command, without
/// the server words.
fn motion_panes_words(session: &str) -> Vec<String> {
    let mut words: Vec<String> = ["list-panes", "-s", "-t"].map(ToOwned::to_owned).into();
    words.push(session_target(session));
    words.extend(["-F", MOTION_PANE_FORMAT].map(ToOwned::to_owned));
    words
}

/// Interpret one complete motion observation. Any malformed row refuses the
/// whole snapshot: dropping it could fabricate a stopped pane.
#[must_use]
pub(crate) fn interpret_motion_panes(succeeded: bool, stdout: &str) -> Option<Vec<MotionPane>> {
    if !succeeded {
        return None;
    }
    let mut panes = Vec::new();
    for line in stdout.lines() {
        let fields: Vec<&str> = line.trim_end_matches('\r').split(FIELD_SEPARATOR).collect();
        if fields.len() != MOTION_PANE_FIELDS {
            return None;
        }
        let [pane_id, agent, session_attached] = fields.as_slice() else {
            return None;
        };
        panes.push(MotionPane {
            pane_id: (*pane_id).to_owned(),
            agent: (!agent.is_empty()).then(|| (*agent).to_owned()),
            session_attached: session_attached.parse().ok()?,
        });
    }
    Some(panes)
}

/// The arguments capturing `pane`'s recent output for the watchdog's hash and
/// throttle scan — `capture-pane -p -J -S -40 -E - -t <pane>`: print to stdout,
/// join wrapped lines, start 40 lines back, end at the last line.
#[must_use]
pub fn capture_pane_args(server: &ServerId, pane: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(
        [
            "capture-pane",
            "-p",
            "-J",
            "-S",
            "-40",
            "-E",
            "-",
            "-t",
            pane,
        ]
        .map(ToOwned::to_owned),
    );
    args
}

// ---------------------------------------------------------------------------
// The watchdog's tmux WRITES — status publication and the transient alert.
// ---------------------------------------------------------------------------

/// The session-scoped user option carrying the watchdog bar, `[watch <glyph>
/// <active>/<total>]`.
pub const WATCHDOG_STATUS_OPTION: &str = "@ae_watchdog_status";

/// How long a transient watchdog alert stays on screen, in milliseconds — the
/// frozen `display-message -d 10000`.
const DISPLAY_MESSAGE_MS: &str = "10000";

/// Which option table a name lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionScope {
    /// A session option: `set-option -t <session-id> …`.
    Session,
    /// A window option: `set-option -w -t <window-id> …`.
    Window,
    /// A PANE option: `set-option -p -t <pane-id> …`.
    Pane,
}

impl OptionScope {
    /// The flag tmux needs for this table, if any.
    const fn flag(self) -> Option<&'static str> {
        match self {
            Self::Session => None,
            Self::Window => Some("-w"),
            Self::Pane => Some("-p"),
        }
    }
}

/// Escape text that is about to enter a tmux FORMAT context — `#` then `%`.
#[must_use]
pub fn format_literal(text: &str) -> String {
    text.replace('#', "##").replace('%', "%%")
}

/// Set one user option on `target`, which must name the selected table.
#[must_use]
pub fn set_option_args(
    server: &ServerId,
    scope: OptionScope,
    target: &str,
    name: &str,
    value: &str,
) -> Vec<String> {
    let mut args = server_args(server);
    args.push("set-option".to_owned());
    if let Some(flag) = scope.flag() {
        args.push(flag.to_owned());
    }
    args.push("-t".to_owned());
    args.push(match scope {
        OptionScope::Session => format!("{}:", session_target(target)),
        OptionScope::Window | OptionScope::Pane => target.to_owned(),
    });
    args.extend([name, value].map(ToOwned::to_owned));
    args
}

/// ONE server-side operation that sets a session option to `value` only while
/// the session is still the PROVEN incarnation and the option is vacant.
///
/// `if-shell -F` is evaluated by the tmux server, in its own command queue, and
/// its `set-option` branch runs in the SAME queued command: no replacement can
/// interleave between the checks and the write. The guard reads only facts the
/// server itself owns:
///
/// * `#{pid}` and `#{start_time}` — the server incarnation;
/// * `#{session_id}` and `#{session_created}` — the session incarnation (the id
///   alone is reused once a server empties, and the creation instant is whole
///   seconds);
/// * `#{<name>}` — the option is still unset or empty.
///
/// Every component is ae-validated (`$<n>`, decimal epochs, a canonical value),
/// so the guard and the branch need no quoting. Conjunctions stay binary and
/// nested: tmux 3.4 reads only the first two operands and drops a third.
#[must_use]
pub fn guarded_session_option_args(
    server: &ServerId,
    expected_server: &ServerIdentity,
    expected_session: &SessionIdentity,
    name: &str,
    value: &str,
) -> Vec<String> {
    let target = format!("{}:", expected_session.id);
    let guard = format!(
        "#{{&&:#{{&&:#{{==:#{{pid}},{pid}}},#{{==:#{{start_time}},{start}}}}},#{{&&:#{{&&:#{{==:#{{session_id}},{id}}},#{{==:#{{session_created}},{created}}}}},#{{==:#{{{name}}},}}}}}}",
        pid = expected_server.pid,
        start = expected_server.start,
        id = expected_session.id,
        created = expected_session.created,
    );
    let mut args = server_args(server);
    args.extend(["if-shell", "-F", "-t"].map(ToOwned::to_owned));
    args.push(expected_session.id.clone());
    args.push(guard);
    args.push(format!("set-option -t {target} {name} {value}"));
    args
}

/// The `-P -F` format `new-session` itself prints: the identity of the session
/// it JUST created, in the SAME command that created it.
///
/// This is what makes the launch's proof atomic: there is no later capture to
/// interleave a replacement into. The server pair (`#{pid}`, `#{start_time}`)
/// and the session pair (`#{session_id}`, `#{session_created}`) are read by the
/// server at creation, and the pane id is the handle ae keeps.
pub const NEW_SESSION_IDENTITY_FORMAT: &str =
    "#{session_id} | #{session_created} | #{pid} | #{start_time} | #{pane_id}";

/// What one successful `new-session -P -F` printed: the full proven identity
/// and the created pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSession {
    /// The server incarnation at creation.
    pub server: ServerIdentity,
    /// The session incarnation at creation.
    pub session: SessionIdentity,
    /// The first pane's `%<n>`.
    pub pane: String,
}

/// The arguments creating a session and printing its full identity at once.
#[must_use]
pub fn new_session_identity_args(name: &str, work_dir: &str) -> Vec<String> {
    [
        "new-session",
        "-d",
        "-s",
        name,
        "-c",
        work_dir,
        "-P",
        "-F",
        NEW_SESSION_IDENTITY_FORMAT,
    ]
    .map(ToOwned::to_owned)
    .to_vec()
}

/// Parse one `new-session -P -F` line into the created identity, or `None` for
/// a failed run, a wrong field count, a non-decimal epoch or a malformed pane.
#[must_use]
pub fn interpret_new_session(succeeded: bool, stdout: &str) -> Option<NewSession> {
    if !succeeded {
        return None;
    }
    let line = stdout.lines().next()?;
    let fields: Vec<&str> = line.split(FIELD_SEPARATOR).collect();
    let [id, created, pid, start, pane] = fields.as_slice() else {
        return None;
    };
    if id.is_empty()
        || !is_decimal(created)
        || !is_decimal(pid)
        || !is_decimal(start)
        || !pane_id_is_valid(pane)
    {
        return None;
    }
    Some(NewSession {
        server: ServerIdentity {
            pid: (*pid).to_owned(),
            start: (*start).to_owned(),
        },
        session: SessionIdentity {
            id: (*id).to_owned(),
            created: (*created).to_owned(),
        },
        pane: (*pane).to_owned(),
    })
}

/// `#{session_id} | #{session_created}` for ONE pane's session — the read that
/// binds a launch's identity to the pane ae CREATED, never to the mutable name.
pub const PANE_SESSION_IDENTITY_FORMAT: &str = "#{session_id} | #{session_created}";

/// The arguments asking which session one exact pane belongs to, and when that
/// session was created.
#[must_use]
pub fn pane_session_identity_args(server: &ServerId, pane: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(
        [
            "display-message",
            "-p",
            "-t",
            pane,
            PANE_SESSION_IDENTITY_FORMAT,
        ]
        .map(ToOwned::to_owned),
    );
    args
}

/// The identity one pane's session reports, or `None` when the read failed,
/// the line is malformed or the creation instant is not a decimal epoch.
#[must_use]
pub fn interpret_pane_session_identity(succeeded: bool, stdout: &str) -> Option<SessionIdentity> {
    if !succeeded {
        return None;
    }
    let line = stdout.lines().next()?;
    let fields: Vec<&str> = line.split(FIELD_SEPARATOR).collect();
    let [id, created] = fields.as_slice() else {
        return None;
    };
    if id.is_empty() || !is_decimal(created) {
        return None;
    }
    Some(SessionIdentity {
        id: (*id).to_owned(),
        created: (*created).to_owned(),
    })
}

/// One `set-option` in a batched tmux command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OptionWrite {
    scope: OptionScope,
    target: String,
    name: String,
    value: String,
}

impl OptionWrite {
    /// A write to one exact option table and target.
    pub(crate) fn new(scope: OptionScope, target: &str, name: &str, value: &str) -> Self {
        Self {
            scope,
            target: target.to_owned(),
            name: name.to_owned(),
            value: value.to_owned(),
        }
    }

    /// Which option this write sets — the fact a caller writing into ANOTHER
    /// session's table has to be held to, read directly rather than recovered
    /// from its argv. Only the pins on that boundary ask.
    #[cfg(test)]
    pub(crate) fn option_name(&self) -> &str {
        &self.name
    }
}

/// Batch several option writes into one tmux invocation. `;` is the argv form
/// of the shell spelling `\;`; no shell sits between this vector and tmux.
#[must_use]
pub(crate) fn set_options_args(server: &ServerId, writes: &[OptionWrite]) -> Vec<String> {
    let mut args = server_args(server);
    for (index, write) in writes.iter().enumerate() {
        if index > 0 {
            args.push(";".to_owned());
        }
        args.extend(set_option_words(write));
    }
    args
}

/// One `set-option` command of a queued invocation, without the server words.
fn set_option_words(write: &OptionWrite) -> Vec<String> {
    let mut words = vec!["set-option".to_owned()];
    if let Some(flag) = write.scope.flag() {
        words.push(flag.to_owned());
    }
    words.push("-t".to_owned());
    words.push(option_target(write.scope, &write.target));
    words.extend([write.name.clone(), write.value.clone()]);
    words
}

/// One `set-option -u` command of a queued invocation, without the server words.
fn unset_option_words(scope: OptionScope, target: &str, name: &str) -> Vec<String> {
    let mut words = vec!["set-option".to_owned()];
    if let Some(flag) = scope.flag() {
        words.push(flag.to_owned());
    }
    words.extend(["-u", "-t"].map(ToOwned::to_owned));
    words.push(option_target(scope, target));
    words.push(name.to_owned());
    words
}

/// The `-t` an option command names: an exact session table, or a window or
/// pane id as given.
fn option_target(scope: OptionScope, target: &str) -> String {
    match scope {
        OptionScope::Session => format!("{}:", session_target(target)),
        OptionScope::Window | OptionScope::Pane => target.to_owned(),
    }
}

/// Remove one user option from `target` — `set-option -u`.
#[must_use]
pub fn unset_option_args(
    server: &ServerId,
    scope: OptionScope,
    target: &str,
    name: &str,
) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(unset_option_words(scope, target, name));
    args
}

/// Replace one transient session option with another in one tmux command
/// queue. Both commands use the same exact session-id target.
#[must_use]
pub fn replace_session_option_args(
    server: &ServerId,
    target: &str,
    remove: &str,
    set: &str,
    value: &str,
) -> Vec<String> {
    let target = format!("{}:", session_target(target));
    let mut args = server_args(server);
    args.extend(["set-option", "-u", "-t"].map(ToOwned::to_owned));
    args.push(target.clone());
    args.push(remove.to_owned());
    args.push(";".to_owned());
    args.extend(["set-option", "-t"].map(ToOwned::to_owned));
    args.push(target);
    args.extend([set, value].map(ToOwned::to_owned));
    args
}

/// The session option the watchdog publishes the work tree's branch into.
pub const BRANCH_OPTION: &str = "@ae_branch_name";

/// Read one session option's raw value — `show-options -t <session> -qv <name>`.
#[must_use]
pub fn session_option_args(server: &ServerId, session: &str, name: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(session_option_words(session, name));
    args
}

/// The `show-options` command of [`session_option_args`], without the server
/// words.
fn session_option_words(session: &str, name: &str) -> Vec<String> {
    let mut words: Vec<String> = ["show-options", "-t"].map(ToOwned::to_owned).into();
    words.push(format!("{}:", session_target(session)));
    words.extend(["-qv", name].map(ToOwned::to_owned));
    words
}

/// The value a completed [`session_option_args`] run reported, or `None`.
///
/// A failed run is no reading, and an unset option prints an empty line — both
/// are `None` rather than an empty branch name, because a branch field carrying
/// `""` renders as a session on a branch called nothing.
///
/// ```
/// use ae::tmux::interpret_session_option;
/// assert_eq!(interpret_session_option(true, "main\n"), Some("main".to_owned()));
/// assert_eq!(interpret_session_option(true, "\n"), None);
/// assert_eq!(interpret_session_option(false, "main\n"), None);
/// ```
#[must_use]
pub fn interpret_session_option(succeeded: bool, stdout: &str) -> Option<String> {
    if !succeeded {
        return None;
    }
    let value = stdout.lines().next().unwrap_or_default().trim_end();
    (!value.is_empty()).then(|| value.to_owned())
}

/// What one completed `show-options -qv` run says about an option — the
/// three-state reading a WRITE must never collapse.
///
/// [`interpret_session_option`] cannot carry this decision: it maps a failed
/// run and an unset option onto the same `None`, so a writer built on it would
/// read "unset" from a server that never answered and overwrite a value it
/// failed to observe. `Vacant` does not claim the option was never set either —
/// `show-options -qv` cannot distinguish never-set from explicitly-empty — but
/// it is the only state that may be written through, and a nonempty `Set`
/// (equal or differing) and an `Unknown` both block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionReading {
    /// The server answered with a usable nonempty value, trimmed.
    Set(String),
    /// The server answered, and the option carries no usable nonempty value.
    Vacant,
    /// The server did not answer, so nothing was observed.
    Unknown,
}

/// The reading a completed [`session_option_args`] run produced.
///
/// ```
/// use ae::tmux::OptionReading;
/// use ae::tmux::interpret_option_reading;
/// assert_eq!(interpret_option_reading(false, "main\n"), OptionReading::Unknown);
/// assert_eq!(interpret_option_reading(true, "\n"), OptionReading::Vacant);
/// assert_eq!(interpret_option_reading(true, " \n"), OptionReading::Vacant);
/// assert_eq!(interpret_option_reading(true, "abc\n"), OptionReading::Set("abc".to_owned()));
/// ```
#[must_use]
pub fn interpret_option_reading(succeeded: bool, stdout: &str) -> OptionReading {
    if !succeeded {
        return OptionReading::Unknown;
    }
    let value = stdout.trim();
    if value.is_empty() {
        OptionReading::Vacant
    } else {
        OptionReading::Set(value.to_owned())
    }
}

/// `#{version}` — the running server's own version, as tmux spells it
/// (`3.4`, `3.5a`, `next-3.6`, `master`).
pub const VERSION_FORMAT: &str = "#{version}";

/// The arguments asking `server` which tmux version it IS.
///
/// The SERVER is asked rather than the `tmux -V` binary on `PATH`: a long-lived
/// server keeps running the binary that started it, so the two disagree exactly
/// when an upgrade has happened and the answer matters most. Read the result
/// with [`interpret_display_value`].
#[must_use]
pub fn version_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["display-message", "-p", VERSION_FORMAT].map(ToOwned::to_owned));
    args
}

/// What asking a server for its `#{version}` PROVED.
///
/// Three answers, because two of the failures are different facts: a server
/// that is not there can be replaced by one this launch starts, while a server
/// that could not be reached might be any version at all — and a floor that
/// treated the second as the first would clear a 3.4 server on the strength of
/// a 3.7 binary that will never run it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionProbe {
    /// The server answered with this text.
    Answered(String),
    /// tmux said there is no server on that socket.
    NoServer,
    /// Anything else: a permission error, a refused connection, a run that
    /// never happened, or an answer with nothing in it.
    Unreachable,
}

/// What a completed `display-message -p '#{version}'` run proved.
///
/// ```
/// use ae::tmux::{VersionProbe, interpret_version};
/// assert_eq!(interpret_version(true, "3.7b\n", ""), VersionProbe::Answered("3.7b".to_owned()));
/// assert_eq!(
///     interpret_version(false, "", "no server running on /tmp/s\n"),
///     VersionProbe::NoServer
/// );
/// assert_eq!(
///     interpret_version(false, "", "error connecting to /tmp/s (No such file or directory)\n"),
///     VersionProbe::NoServer
/// );
/// assert_eq!(
///     interpret_version(false, "", "error connecting to /tmp/s (Permission denied)\n"),
///     VersionProbe::Unreachable
/// );
/// assert_eq!(interpret_version(true, "\n", ""), VersionProbe::Unreachable);
/// ```
#[must_use]
pub fn interpret_version(succeeded: bool, stdout: &str, stderr: &str) -> VersionProbe {
    match interpret_display_value(succeeded, stdout) {
        Some(found) => VersionProbe::Answered(found),
        // A run that SUCCEEDED and printed nothing is not a server that is
        // absent; it is one whose answer ae could not read.
        None if succeeded => VersionProbe::Unreachable,
        None if says_no_server(stderr) => VersionProbe::NoServer,
        None => VersionProbe::Unreachable,
    }
}

/// The arguments asking the tmux BINARY on `PATH` which version it is — `tmux
/// -V`, with no server selector, because no server is involved.
///
/// The executable is asked only when no server answered: it is the binary that
/// a `new-session` would start, so it is the version the launch would get.
#[must_use]
pub fn program_version_args() -> Vec<String> {
    vec!["-V".to_owned()]
}

/// The version a completed `tmux -V` run reported, or `None`.
///
/// `tmux -V` prints `tmux <version>`; the program name is dropped so the answer
/// is spelled exactly as `#{version}` spells it.
///
/// ```
/// use ae::tmux::interpret_program_version;
/// assert_eq!(interpret_program_version(true, "tmux 3.7b\n").as_deref(), Some("3.7b"));
/// assert_eq!(interpret_program_version(true, "3.7b\n").as_deref(), Some("3.7b"));
/// assert_eq!(interpret_program_version(false, "tmux 3.7b\n"), None);
/// ```
#[must_use]
pub fn interpret_program_version(succeeded: bool, stdout: &str) -> Option<String> {
    let line = interpret_display_value(succeeded, stdout)?;
    let value = line.strip_prefix("tmux ").unwrap_or(&line).trim();
    (!value.is_empty()).then(|| value.to_owned())
}

// ---------------------------------------------------------------------------
// The key-table read: `ae doctor`'s input-map check.
// ---------------------------------------------------------------------------

/// One key-table entry, as `list-keys -T <table>` prints it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBinding {
    /// The key name (`MouseDown1Status`, `a`).
    pub key: String,
    /// The bound command, as printed.
    pub command: String,
}

/// The full argument list for reading `server`'s `table` key bindings.
#[must_use]
pub fn list_keys_args(server: &ServerId, table: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-keys", "-T", table].map(ToOwned::to_owned));
    args
}

/// The key-table entries a `list-keys` run printed, or `None` when it failed.
///
/// A line that is not a `bind-key` row is skipped, never fatal: a future
/// spelling must degrade to "that key reads as missing", not to "the table is
/// unreadable".
#[must_use]
pub fn interpret_list_keys(succeeded: bool, stdout: &str) -> Option<Vec<KeyBinding>> {
    if !succeeded {
        return None;
    }
    let mut out = Vec::new();
    for line in stdout.lines() {
        let mut parts = line.split_whitespace();
        if parts.next() != Some("bind-key") {
            continue;
        }
        let mut token = parts.next();
        while matches!(token, Some("-n" | "-r" | "-nr" | "-rn")) {
            token = parts.next();
        }
        if token != Some("-T") {
            continue;
        }
        if parts.next().is_none() {
            continue;
        }
        let Some(key) = parts.next() else {
            continue;
        };
        out.push(KeyBinding {
            key: key.to_owned(),
            command: parts.collect::<Vec<_>>().join(" "),
        });
    }
    Some(out)
}

/// The two look knobs the watchdog re-reads every cycle, in ONE query — so a
/// human who flips `@ae_icons` on a live session sees the next cycle in ASCII.
pub const LOOK_FORMAT: &str = "#{@ae_icons} | #{@ae_palette} | #{@ae_look} | #{@ae_motion}";

/// The four look values a session carries, each empty when unset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LookOptions {
    /// `@ae_icons`.
    pub icons: String,
    /// `@ae_palette`.
    pub palette: String,
    /// `@ae_look`.
    pub drawn: String,
    /// `@ae_motion`.
    pub motion: String,
}

/// The arguments asking `session` which look it is drawn in.
#[must_use]
pub fn look_args(server: &ServerId, session: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(look_words(session));
    args
}

/// The `display-message` command of [`look_args`], without the server words.
fn look_words(session: &str) -> Vec<String> {
    let mut words: Vec<String> = ["display-message", "-p", "-t"]
        .map(ToOwned::to_owned)
        .into();
    words.push(format!("{}:", session_target(session)));
    words.push(LOOK_FORMAT.to_owned());
    words
}

/// The same question with NO target, so tmux answers for the session the
/// CALLING client is in — which is the one a picker is being drawn on.
#[must_use]
pub fn look_here_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["display-message", "-p", LOOK_FORMAT].map(ToOwned::to_owned));
    args
}

/// The look values a completed [`look_args`] run reported.
///
/// A short answer leaves the missing fields EMPTY rather than failing: an older
/// core's session carries only the first two, and empty is the default in every
/// position. A read that did not RUN is `None`, which is a different fact
/// entirely — the caller must not stand a default look in for it.
///
/// ```
/// use ae::tmux::interpret_look;
/// let read = interpret_look(true, "off | b | on | off\n").unwrap_or_default();
/// assert_eq!(read.icons, "off");
/// assert_eq!(read.palette, "b");
/// assert_eq!(read.motion, "off");
/// assert_eq!(interpret_look(true, "on | a").unwrap_or_default().drawn, "");
/// assert_eq!(interpret_look(false, "on | a | on | on"), None);
/// ```
#[must_use]
pub fn interpret_look(succeeded: bool, stdout: &str) -> Option<LookOptions> {
    if !succeeded {
        return None;
    }
    let line = stdout.lines().next().unwrap_or_default().trim_end();
    let mut fields = line.split(FIELD_SEPARATOR).map(str::trim);
    let mut next = || fields.next().unwrap_or_default().to_owned();
    Some(LookOptions {
        icons: next(),
        palette: next(),
        drawn: next(),
        motion: next(),
    })
}

/// The numeric zone tmux formats its server's clock in (`+0200`): the time
/// zone a viewer of that server lives in.
pub const ZONE_FORMAT: &str = "#{t/f/%z:start_time}";

/// The arguments asking the server of `session` for its [`ZONE_FORMAT`].
#[must_use]
pub fn zone_args(server: &ServerId, session: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["display-message", "-p", "-t"].map(ToOwned::to_owned));
    args.push(format!("{}:", session_target(session)));
    args.push(ZONE_FORMAT.to_owned());
    args
}

/// The zone answer of a completed [`zone_args`] run, or `None` when it did not
/// run or said nothing. The text is unvalidated: a tmux without the `t/f/`
/// modifier prints something else, and the caller keeps only `[+-]HHMM`.
///
/// ```
/// assert_eq!(ae::tmux::interpret_zone(true, "+0200\n"), Some("+0200".to_owned()));
/// assert_eq!(ae::tmux::interpret_zone(true, "\n"), None);
/// assert_eq!(ae::tmux::interpret_zone(false, "+0200\n"), None);
/// ```
#[must_use]
pub fn interpret_zone(succeeded: bool, stdout: &str) -> Option<String> {
    interpret_session_option(succeeded, stdout)
}

/// `#{pane_tty}` — the tty of every pane on the server, one per line.
pub const PANE_TTY_FORMAT: &str = "#{pane_tty}";

/// The arguments listing the ttys of ALL panes on `server` — `-a`, deliberately
/// across every session, because the question is "is THIS terminal a pane of
/// this server" and the answer may live in any of them.
#[must_use]
pub fn pane_ttys_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-panes", "-a", "-F", PANE_TTY_FORMAT].map(ToOwned::to_owned));
    args
}

/// What a completed pane-tty listing means.
#[must_use]
pub fn interpret_pane_ttys(succeeded: bool, stdout: &str) -> Option<Vec<String>> {
    if !succeeded {
        return None;
    }
    Some(
        stdout
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
    )
}

/// The arguments asking `server` which session the CALLING client is in —
/// frozen's `tmux display-message -p '#S'`, with no `-t`, so tmux answers for
/// the client the invocation inherited.
#[must_use]
pub fn current_session_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["display-message", "-p", SESSION_NAME_FORMAT].map(ToOwned::to_owned));
    args
}

/// The arguments asking `server` for its own socket path — frozen's
/// `tmux display-message -p '#{socket_path}'`, the probe that decides WHERE a
/// launch actually lands.
#[must_use]
pub fn socket_path_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["display-message", "-p", "#{socket_path}"].map(ToOwned::to_owned));
    args
}

/// The arguments asking `server` for its own process id — the other half of the
/// round trip that PROVES a relative socket path.
#[must_use]
pub fn server_pid_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["display-message", "-p", "#{pid}"].map(ToOwned::to_owned));
    args
}

/// `display-message -p -t <pane> '#{pane_current_path}'` — TEST ONLY.
/// No product reader observes pane paths; none may be added unruled.
#[cfg(test)]
#[must_use]
pub fn pane_current_path_args(server: &ServerId, pane: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(
        ["display-message", "-p", "-t", pane, "#{pane_current_path}"].map(ToOwned::to_owned),
    );
    args
}

/// `display-message -p -t <pane> '#{@ae_console}'`: the chat-window stamp of
/// one pane.
#[must_use]
pub fn pane_console_args(server: &ServerId, pane: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["display-message", "-p", "-t", pane, "#{@ae_console}"].map(ToOwned::to_owned));
    args
}

/// The stamp a completed [`pane_console_args`] run reported: `Some("")` for an
/// unstamped pane, `None` when tmux did not answer — never the same thing.
#[must_use]
pub fn interpret_pane_console(succeeded: bool, stdout: &str) -> Option<String> {
    succeeded.then(|| stdout.lines().next().unwrap_or_default().trim().to_owned())
}

/// `display-message -p -t <pane>`: its session name and window index, printed.
#[must_use]
pub fn pane_place_args(server: &ServerId, pane: &str) -> Vec<String> {
    let mut args = server_args(server);
    let format = "#{session_name} | #{window_index}";
    args.extend(["display-message", "-p", "-t", pane, format].map(ToOwned::to_owned));
    args
}

/// The two fields [`pane_place_args`] printed, split raw, or `None`.
#[must_use]
pub fn interpret_pane_place(succeeded: bool, stdout: &str) -> Option<(String, String)> {
    if !succeeded {
        return None;
    }
    let (session, window) = stdout.lines().next()?.split_once(FIELD_SEPARATOR)?;
    Some((session.trim().to_owned(), window.trim().to_owned()))
}

/// The single value a `display-message -p` answer carries: its first line,
/// trimmed, and only when the query SUCCEEDED.
#[must_use]
pub fn interpret_display_value(succeeded: bool, stdout: &str) -> Option<String> {
    if !succeeded {
        return None;
    }
    let value = stdout.lines().next().unwrap_or_default().trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// Whether `tty` is one of `pane_ttys`, comparing with `/dev/` stripped from
/// BOTH sides.
#[must_use]
pub fn tty_is_a_pane(tty: &str, pane_ttys: &[String]) -> bool {
    let bare = |value: &str| {
        let value = value.trim();
        value.strip_prefix("/dev/").unwrap_or(value).to_owned()
    };
    let mine = bare(tty);
    !mine.is_empty() && pane_ttys.iter().any(|pane| bare(pane) == mine)
}

/// `#{session_id} | #{session_name}` — the pair the id resolver reads.
pub const SESSION_ID_FORMAT: &str = "#{session_id} | #{session_name}";

/// The arguments listing `server`'s sessions as id/name pairs.
#[must_use]
pub fn session_ids_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-sessions", "-F", SESSION_ID_FORMAT].map(ToOwned::to_owned));
    args
}

/// The id tmux holds for the session named EXACTLY `name`, or `None`.
///
/// Exact, never a prefix: the id exists to make the write target unambiguous, so
/// resolving it by prefix would reintroduce the hazard it removes. `None` for a
/// failed run and for a name the server does not hold — and the caller must then
/// write NOTHING, because `-t ""` lands on tmux's CURRENT session, which is some
/// other user's bar.
///
/// ```
/// use ae::tmux::interpret_session_id;
/// let listing = "$0 | other\n$3 | demo\n";
/// assert_eq!(interpret_session_id(true, listing, "demo"), Some("$3".to_owned()));
/// assert_eq!(interpret_session_id(true, listing, "dem"), None);
/// assert_eq!(interpret_session_id(false, listing, "demo"), None);
/// ```
#[must_use]
pub fn interpret_session_id(succeeded: bool, stdout: &str, name: &str) -> Option<String> {
    if !succeeded {
        return None;
    }
    stdout.lines().find_map(|line| {
        let (id, held) = line.split_once(FIELD_SEPARATOR)?;
        (held == name && !id.is_empty()).then(|| id.to_owned())
    })
}

/// The id, the creation instant and the name of every session.
///
/// The creation instant is the second half of a session's IMMUTABLE identity:
/// `#{session_id}` alone can be REUSED — tmux starts again at `$0` once every
/// session on a server is gone — so a name whose session was killed and
/// recreated can answer the same `$<n>` while being a different incarnation.
pub const SESSION_IDENTITY_FORMAT: &str = "#{session_id} | #{session_created} | #{session_name}";

/// One session's immutable identity, as the server reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionIdentity {
    /// The `$<n>` id.
    pub id: String,
    /// The epoch second tmux created this session.
    pub created: String,
}

/// The arguments listing every session's immutable identity.
#[must_use]
pub fn session_identities_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-sessions", "-F", SESSION_IDENTITY_FORMAT].map(ToOwned::to_owned));
    args
}

/// The immutable identity of the session named EXACTLY `name`, or `None`.
///
/// Exact for the same reason [`interpret_session_id`] is: this value keys a
/// WRITE. A line with the wrong field count, a non-decimal creation instant or
/// a duplicated name resolves to nothing rather than to a guess.
///
/// ```
/// use ae::tmux::interpret_session_identity;
/// let listing = "$0 | 1789100000 | other\n$3 | 1789100001 | demo\n";
/// let found = interpret_session_identity(true, listing, "demo").unwrap();
/// assert_eq!(found.id, "$3");
/// assert_eq!(found.created, "1789100001");
/// assert_eq!(interpret_session_identity(true, listing, "dem"), None);
/// assert_eq!(interpret_session_identity(true, "$3 | x | demo\n", "demo"), None);
/// assert_eq!(interpret_session_identity(false, listing, "demo"), None);
/// ```
#[must_use]
pub fn interpret_session_identity(
    succeeded: bool,
    stdout: &str,
    name: &str,
) -> Option<SessionIdentity> {
    if !succeeded {
        return None;
    }
    let mut matches = stdout.lines().filter_map(|line| {
        let fields: Vec<&str> = line.split(FIELD_SEPARATOR).collect();
        let [id, created, held] = fields.as_slice() else {
            return None;
        };
        if *held != name || id.is_empty() || !is_decimal(created) {
            return None;
        }
        Some(SessionIdentity {
            id: (*id).to_owned(),
            created: (*created).to_owned(),
        })
    });
    let identity = matches.next()?;
    // A duplicated name is an ambiguity, not a choice.
    matches.next().is_none().then_some(identity)
}

/// `#{pane_id} | #{window_id} | #{@ae_theme} | #{@ae_agent}` — the
/// window-grouping read.
///
/// The theme stamp rides along because the alternative is a second listing:
/// the watchdog has to know which windows are already dressed, and a user
/// option set on the WINDOW resolves in a pane's format context. The two
/// indexes order the panes the way the session shows them, which tmux's
/// listing order does not promise.
pub const WINDOW_PANE_FORMAT: &str = "#{pane_id} | #{window_id} | #{@ae_theme} | #{@ae_reader_src} | #{@ae_console} | #{pane_dead} | #{window_index} | #{pane_index} | #{@ae_agent}";

/// The number of fields [`WINDOW_PANE_FORMAT`] yields.
const WINDOW_PANE_FIELDS: usize = 9;

/// One pane as the window grouping reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowPane {
    /// `#{pane_id}`.
    pub pane_id: String,
    /// `#{window_id}` — `@N`, server-global and stable for the window's life.
    pub window_id: String,
    /// `@ae_theme` — the window's look stamp, empty when it carries none.
    pub theme: String,
    /// `@ae_reader_src` — the source a READER pane shows, or `None` when the
    /// pane carries no reader stamp.
    pub reader_src: Option<String>,
    /// `@ae_console` — the session UUID a console pane is stamped with, or
    /// `None` for every other pane.
    pub console: Option<String>,
    /// `#{pane_dead}` — the pane's process has exited and the pane remains.
    pub dead: bool,
    /// `#{window_index}` — the window's place in the session.
    pub window_index: u32,
    /// `#{pane_index}` — the pane's place in its window.
    pub pane_index: u32,
    /// `@ae_agent`, or `None` when unstamped.
    pub agent: Option<String>,
}

/// The arguments grouping `session`'s panes by window.
#[must_use]
pub fn window_panes_args(server: &ServerId, session: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-panes", "-s", "-t"].map(ToOwned::to_owned));
    args.push(session_target(session));
    args.extend(["-F", WINDOW_PANE_FORMAT].map(ToOwned::to_owned));
    args
}

/// One [`WindowPane`] per line that split into exactly [`WINDOW_PANE_FIELDS`]
/// fields with numeric indexes; `None` on a failed run.
#[must_use]
pub fn interpret_window_panes(succeeded: bool, stdout: &str) -> Option<Vec<WindowPane>> {
    if !succeeded {
        return None;
    }
    Some(
        stdout
            .lines()
            .filter_map(|line| {
                // BOUNDED: the agent stamp is last and is the one field that
                // could carry the separator, so a longer line is that name and
                // not a corrupt row.
                let fields: Vec<&str> = line.splitn(WINDOW_PANE_FIELDS, FIELD_SEPARATOR).collect();
                let [
                    pane_id,
                    window_id,
                    theme,
                    reader_src,
                    console,
                    dead,
                    window,
                    pane,
                    agent,
                ] = fields.as_slice()
                else {
                    return None;
                };
                let (Ok(window_index), Ok(pane_index)) = (window.parse(), pane.parse()) else {
                    return None;
                };
                let reader_src = reader_src.trim_end();
                let agent = agent.trim_end();
                Some(WindowPane {
                    pane_id: (*pane_id).to_owned(),
                    window_id: (*window_id).to_owned(),
                    theme: (*theme).to_owned(),
                    reader_src: (!reader_src.is_empty()).then(|| reader_src.to_owned()),
                    console: (!console.trim_end().is_empty())
                        .then(|| console.trim_end().to_owned()),
                    dead: dead.trim_end() == "1",
                    window_index,
                    pane_index,
                    agent: (!agent.is_empty()).then(|| agent.to_owned()),
                })
            })
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// Pane DELIVERY — the paste path's argv (B move 1).

/// What a pane is running and under which process — `pane_current_command`
/// for the tool model, `pane_pid` for the dead-pane walk.
pub const PANE_PROBE_FORMAT: &str = "#{pane_pid} | #{pane_current_command}";

/// The number of fields [`PANE_PROBE_FORMAT`] renders.
const PANE_PROBE_FIELDS: usize = 2;

/// The readings of [`PANE_PROBE_FORMAT`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObservedPaneProbe {
    /// `#{pane_current_command}`, empty when tmux rendered nothing.
    pub command: String,
    /// `#{pane_pid}`, `None` when it was not a decimal number.
    pub pid: Option<u32>,
}

/// The full argument list for one pane's [`PANE_PROBE_FORMAT`] readings.
#[must_use]
pub fn pane_probe_args(server: &ServerId, pane: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["display-message", "-p", "-t", pane, PANE_PROBE_FORMAT].map(ToOwned::to_owned));
    args
}

/// What a completed [`pane_probe_args`] run means.
#[must_use]
pub fn interpret_pane_probe(succeeded: bool, stdout: &str) -> Option<ObservedPaneProbe> {
    if !succeeded {
        return None;
    }
    let line = stdout.lines().next()?;
    let fields: Vec<&str> = line.splitn(PANE_PROBE_FIELDS, FIELD_SEPARATOR).collect();
    if fields.len() != PANE_PROBE_FIELDS {
        return None;
    }
    Some(ObservedPaneProbe {
        command: fields[1].to_owned(),
        pid: fields[0].parse().ok(),
    })
}

/// A pane's size and cursor row: the console composer's wrap, cap and erase.
pub const PANE_SIZE_FORMAT: &str = "#{pane_width} | #{pane_height} | #{cursor_y}";

/// The readings of [`PANE_SIZE_FORMAT`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneSize {
    pub width: usize,
    pub height: usize,
    pub cursor_y: usize,
}

/// The full argument list for one pane's [`PANE_SIZE_FORMAT`] readings.
#[must_use]
pub fn pane_size_args(server: &ServerId, pane: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["display-message", "-p", "-t", pane, PANE_SIZE_FORMAT].map(ToOwned::to_owned));
    args
}

/// What a completed [`pane_size_args`] run means: three numbers, a pane at
/// least one cell each way, or nothing.
#[must_use]
pub fn interpret_pane_size(succeeded: bool, stdout: &str) -> Option<PaneSize> {
    if !succeeded {
        return None;
    }
    let mut fields = stdout.lines().next()?.split(FIELD_SEPARATOR);
    let mut next = || fields.next()?.parse::<usize>().ok();
    let (width, height, cursor_y) = (next()?, next()?, next()?);
    let size = PaneSize {
        width,
        height,
        cursor_y,
    };
    (fields.next().is_none() && width > 0 && height > 0).then_some(size)
}

/// Whether a pane is a DEAD pane tmux is keeping on screen (`remain-on-exit`).
///
/// Its own read rather than a field on [`PANE_PROBE_FORMAT`]: every existing
/// probe caller asks what a LIVE pane is running, and only the seat relaunch
/// has to tell "the shell is idle" from "the shell is gone and tmux is holding
/// the corpse". One field, so no separator.
pub const PANE_DEAD_FORMAT: &str = "#{pane_dead}";

/// The full argument list for reading whether `pane` is a dead pane.
#[must_use]
pub fn pane_dead_args(server: &ServerId, pane: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["display-message", "-p", "-t", pane, PANE_DEAD_FORMAT].map(ToOwned::to_owned));
    args
}

/// What a completed [`pane_dead_args`] run means — `None` when the server did
/// not answer or the field was not a readable `0`/`1`.
#[must_use]
pub fn interpret_pane_dead(succeeded: bool, stdout: &str) -> Option<bool> {
    if !succeeded {
        return None;
    }
    match stdout.lines().next()?.trim() {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

/// Whether a capture keeps the pane's styling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Styling {
    /// `capture-pane -p` — printable text only.
    Plain,
    /// `capture-pane -e -p -S 0` — SGR preserved, from the top of the visible
    /// pane.
    Escapes,
}

/// The full argument list for capturing `pane`'s visible screen.
#[must_use]
pub fn capture_screen_args(server: &ServerId, pane: &str, styling: Styling) -> Vec<String> {
    let mut args = server_args(server);
    args.push("capture-pane".to_owned());
    if styling == Styling::Escapes {
        args.push("-e".to_owned());
    }
    args.push("-p".to_owned());
    if styling == Styling::Escapes {
        args.push("-S".to_owned());
        args.push("0".to_owned());
    }
    args.push("-t".to_owned());
    args.push(pane.to_owned());
    args
}

/// The full argument list for staging a message in buffer `buffer`.
#[must_use]
pub fn load_buffer_args(server: &ServerId, buffer: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["load-buffer", "-b", buffer, "-"].map(ToOwned::to_owned));
    args
}

/// The full argument list for pasting `buffer` into `pane` and deleting it.
#[must_use]
pub fn paste_buffer_args(server: &ServerId, buffer: &str, pane: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["paste-buffer", "-d", "-p", "-b", buffer, "-t", pane].map(ToOwned::to_owned));
    args
}

/// The full argument list for dropping a staged buffer that was never pasted.
#[must_use]
pub fn delete_buffer_args(server: &ServerId, buffer: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["delete-buffer", "-b", buffer].map(ToOwned::to_owned));
    args
}

/// A keystroke the delivery path sends — the closed set, so no caller can
/// name a key of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Submit.
    Enter,
    /// `Escape` — the interrupt's second cancel, for a TUI with no copy mode
    /// to leave.
    Escape,
    /// `C-e` — move to the END of the line, so the `C-u` that follows clears
    /// ALL of it. `C-u` alone kills only to the line start, which leaves
    /// whatever sits right of a mid-line cursor to be executed by the paste.
    LineEnd,
    /// `C-u` — clear the input line, the notice path's one measurable retry.
    ClearLine,
    /// `-X cancel` — leave copy mode.
    CancelCopyMode,
    /// `Up` and `Down` — the arrows that move a measured trust modal's
    /// selection one row, and nothing else.
    Up,
    Down,
}

/// The full argument list for sending one [`Key`] to `pane`.
#[must_use]
pub fn send_keys_args(server: &ServerId, pane: &str, key: Key) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["send-keys", "-t", pane].map(ToOwned::to_owned));
    match key {
        Key::Enter => args.push("Enter".to_owned()),
        Key::Escape => args.push("Escape".to_owned()),
        Key::LineEnd => args.push("C-e".to_owned()),
        Key::ClearLine => args.push("C-u".to_owned()),
        Key::Up => args.push("Up".to_owned()),
        Key::Down => args.push("Down".to_owned()),
        Key::CancelCopyMode => {
            args.push("-X".to_owned());
            args.push("cancel".to_owned());
        }
    }
    args
}

/// Each attached client's address, session, active pane and last-input epoch.
pub const CLIENT_FORMAT: &str =
    "#{client_name} | #{session_name} | #{pane_id} | #{client_activity}";

/// The number of fields [`CLIENT_FORMAT`] renders.
const CLIENT_FIELDS: usize = 4;

/// One attached client's [`CLIENT_FORMAT`] readings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedClient {
    /// The target-client name, normally its tty path.
    pub name: String,
    /// The session this client is currently viewing.
    pub session: String,
    /// The pane this client is viewing.
    pub pane: String,
    /// The epoch of its last input, or `None` when it was not a number.
    pub activity: Option<u64>,
}

/// The full argument list for listing `server`'s attached clients.
#[must_use]
pub fn list_clients_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-clients", "-F", CLIENT_FORMAT].map(ToOwned::to_owned));
    args
}

/// What a completed [`list_clients_args`] run means.
#[must_use]
pub fn interpret_clients(succeeded: bool, stdout: &str) -> Option<Vec<ObservedClient>> {
    if !succeeded {
        return None;
    }
    Some(
        stdout
            .lines()
            .filter(|line| !line.is_empty())
            .filter_map(|line| {
                let fields: Vec<&str> = line.split(FIELD_SEPARATOR).collect();
                if fields.len() != CLIENT_FIELDS {
                    return None;
                }
                Some(ObservedClient {
                    name: fields[0].to_owned(),
                    session: fields[1].to_owned(),
                    pane: fields[2].to_owned(),
                    activity: fields[3].parse().ok(),
                })
            })
            .collect(),
    )
}

/// The arguments creating a spawned agent's own WINDOW, printing its pane id.
#[must_use]
pub fn new_window_args(server: &ServerId, session: &str, work_dir: &str) -> Vec<String> {
    let mut args = server_args(server);
    let target = format!("{}:", session_target(session));
    args.extend(
        [
            "new-window",
            "-d",
            "-t",
            &target,
            "-c",
            work_dir,
            "-P",
            "-F",
            PANE_ID_FORMAT,
        ]
        .map(ToOwned::to_owned),
    );
    args
}

/// The `#{pane_id}` a `-P -F` run prints — the id, or `None`.
#[must_use]
pub fn interpret_new_window(succeeded: bool, stdout: &str) -> Option<String> {
    if !succeeded {
        return None;
    }
    let id = stdout.trim();
    (!id.is_empty() && id.starts_with('%')).then(|| id.to_owned())
}

/// The `-F` a [`new_window_args`] run asks for.
const PANE_ID_FORMAT: &str = "#{pane_id}";

/// The arguments setting a pane's TITLE — `select-pane -t <pane> -T <title>`.
#[must_use]
pub fn pane_title_args(server: &ServerId, pane: &str, title: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["select-pane", "-t", pane, "-T", title].map(ToOwned::to_owned));
    args
}

/// The arguments renaming the window a pane lives in.
#[must_use]
pub fn rename_window_args(server: &ServerId, pane: &str, name: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["rename-window", "-t", pane, name].map(ToOwned::to_owned));
    args
}

#[cfg(test)]
mod tests;
