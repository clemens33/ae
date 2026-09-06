//! Addressing a tmux server, and reading what it said.
//!
//! Everything here is PURE: argument derivation in, text interpretation out.
//! Running the child process is [`crate::transport`]'s job, and deliberately not
//! this module's. The split is what keeps the two decisions that can be WRONG —
//! WHICH server an argument list addresses, and WHAT a completed run means —
//! unit-testable without a process anywhere near them; the exec is a detail
//! around them, behind the one door `clippy.toml` opens in product code.

use std::path::Path;

use crate::inventory::{QueryFailed, ServerId};
use crate::meta::Selector;

/// The format `list-sessions` is asked for: one exact session name per line.
pub const SESSION_NAME_FORMAT: &str = "#{session_name}";

/// The separator every multi-field format in this module renders between its
/// fields.
const FIELD_SEPARATOR: &str = " | ";

/// The ae-ownership marker, read from a session's own tmux environment.
pub const OWNERSHIP_VARIABLE: &str = "AE_SESSION";

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
    let mut args = server_args(server);
    args.push("show-environment".to_owned());
    args.push("-t".to_owned());
    args.push(session.to_owned());
    args.push(OWNERSHIP_VARIABLE.to_owned());
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
#[must_use]
pub fn interpret_stopped(succeeded: bool, stdout: &str, stderr: &str, name: &str) -> StopProbe {
    if succeeded {
        let present = stdout.lines().map(str::trim_end).any(|line| line == name);
        return if present {
            StopProbe::Present
        } else {
            StopProbe::Absent
        };
    }
    // STRICTLY the clean-exit diagnostic, and NOT the connect error the version
    // probe also reads as absence: a live server whose socket was unlinked
    // answers ENOENT while it keeps running, so ENOENT proves a session gone
    // only if you are willing to be wrong about it. The two probes ask
    // different questions — see [`says_no_server`].
    let clean_dead = stderr
        .lines()
        .next()
        .is_some_and(|line| line.starts_with(NO_SERVER_DIAGNOSTIC));
    if clean_dead {
        StopProbe::Absent
    } else {
        StopProbe::Unknown
    }
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
    let Some(line) = stderr.lines().next().map(str::trim_end) else {
        return false;
    };
    line.starts_with(NO_SERVER_DIAGNOSTIC)
        || (line.starts_with(CONNECT_PREFIX) && line.ends_with(CONNECT_ABSENT_SUFFIX))
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

/// The format `list-panes` is asked for: three fields per pane, tab-separated.
pub const PANE_FORMAT: &str = "#{pane_dead} | #{@ae_slot} | #{pane_current_command}";

/// How many [`FIELD_SEPARATOR`]-separated fields [`PANE_FORMAT`] produces per pane.
pub const PANE_FIELDS: usize = 3;

/// The full argument list for enumerating one session's panes.
#[must_use]
pub fn list_panes_args(server: &ServerId, session: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.push("list-panes".to_owned());
    args.push("-s".to_owned());
    args.push("-t".to_owned());
    args.push(session.to_owned());
    args.push("-F".to_owned());
    args.push(PANE_FORMAT.to_owned());
    args
}

/// One pane the server reported, as three readings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedPane {
    /// `#{pane_dead}` — `Some(true)` for a dead pane, `Some(false)` for a live
    /// one, `None` when the field was not a readable `0`/`1`.
    pub dead: Option<bool>,
    /// The `@ae_slot` value, when the pane carries a usable one.
    pub slot: Option<String>,
    /// `#{pane_current_command}`, when the field carried one.
    pub command: Option<String>,
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
        dead: None,
        slot: None,
        command: None,
    };
    let fields: Vec<&str> = line.trim_end_matches('\r').split(FIELD_SEPARATOR).collect();
    if fields.len() != PANE_FIELDS {
        return blank;
    }
    let usable = |value: &str| Some(value.to_owned()).filter(|v| !v.is_empty());
    ObservedPane {
        // `0`/`1` and nothing else.
        dead: match fields[0] {
            "0" => Some(false),
            "1" => Some(true),
            _ => None,
        },
        slot: usable(fields[1].trim_end()),
        command: usable(fields[2].trim_end()),
    }
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

/// The format the viewer query asks for: the calling pane's routing slot, its
/// tmux session and its display ref — three readings in ONE round trip rather
/// than three, so the pane cannot change identity between them.
pub const VIEWER_FORMAT: &str = "#{@ae_slot} | #{session_name} | #{@ae_agent}";

/// The number of fields [`VIEWER_FORMAT`] yields.
const VIEWER_FIELDS: usize = 3;

/// The arguments that read [`VIEWER_FORMAT`] off `pane` on `server`.
#[must_use]
pub fn viewer_args(server: &ServerId, pane: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["display-message", "-p", "-t", pane, VIEWER_FORMAT].map(ToOwned::to_owned));
    args
}

/// The calling pane's three identity readings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObservedViewer {
    /// `@ae_slot` — the routing key, unvalidated here.
    pub slot: Option<String>,
    /// `#{session_name}` of the pane.
    pub session: Option<String>,
    /// `@ae_agent` — the display `alias:name`.
    pub agent: Option<String>,
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
    let fields: Vec<&str> = line.split(FIELD_SEPARATOR).collect();
    if fields.len() != VIEWER_FIELDS {
        return None;
    }
    let reading = |field: &str| (!field.is_empty()).then(|| field.to_owned());
    Some(ObservedViewer {
        slot: reading(fields[0]),
        session: reading(fields[1]),
        agent: reading(fields[2]),
    })
}

/// The arguments for the resolver's session check — `tmux has-session -t
/// <session>` before a cross-session lookup.
#[must_use]
pub fn has_session_args(server: &ServerId, session: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["has-session", "-t", session].map(ToOwned::to_owned));
    args
}

/// The arguments for the lifecycle kill — `tmux kill-session -t <session-id>`.
#[must_use]
pub fn kill_session_args(server: &ServerId, session_id: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["kill-session", "-t", session_id].map(ToOwned::to_owned));
    args
}

/// The roster the name resolver reads:
/// `list-panes -s -t <session> -F '#{pane_id} | #{@ae_agent}'`.
pub const AGENTS_FORMAT: &str = "#{pane_id} | #{@ae_agent}";

/// The full argument list for that roster.
#[must_use]
pub fn agents_args(server: &ServerId, session: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-panes", "-s", "-t", session, "-F", AGENTS_FORMAT].map(ToOwned::to_owned));
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
    args.extend(["list-panes", "-s", "-t", session, "-F", SLOTS_FORMAT].map(ToOwned::to_owned));
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
pub const WATCH_PANE_FORMAT: &str =
    "#{pane_id} | #{@ae_slot} | #{@ae_agent} | #{pane_pid} | #{pane_current_command}";

/// The number of fields [`WATCH_PANE_FORMAT`] yields.
const WATCH_PANE_FIELDS: usize = 5;

/// The arguments enumerating every pane of `session` on `server` for the
/// watchdog — `list-panes -s -t <session> -F <WATCH_PANE_FORMAT>`, widened to
/// the fields the cycle reads.
#[must_use]
pub fn watch_panes_args(server: &ServerId, session: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(
        ["list-panes", "-s", "-t", session, "-F", WATCH_PANE_FORMAT].map(ToOwned::to_owned),
    );
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
/// let out = format!("%1{sep}main{sep}cl:lead{sep}4321{sep}claude\n%2{sep}{sep}{sep}88{sep}zsh\n");
/// let panes = interpret_watch_panes(true, &out).unwrap();
/// assert_eq!(panes[0], WatchPane {
///     pane_id: "%1".into(), slot: Some("main".into()), agent: Some("cl:lead".into()),
///     current_command: "claude".into(), pane_pid: Some(4321),
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
            current_command: fields[4].to_owned(),
            pane_pid: fields[3].parse::<u32>().ok(),
        });
    }
    if !stdout.is_empty() && panes.is_empty() {
        return None;
    }
    Some(panes)
}

/// The ticker's one observation: identity, visible-output coordinates, and
/// whether anybody is attached to the session.
pub(crate) const MOTION_PANE_FORMAT: &str =
    "#{pane_id} | #{@ae_agent} | #{history_size} | #{cursor_x} | #{cursor_y} | #{session_attached}";

/// The number of fields [`MOTION_PANE_FORMAT`] yields.
const MOTION_PANE_FIELDS: usize = 6;

/// One pane as the motion ticker reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MotionPane {
    /// `#{pane_id}`, e.g. `%3`.
    pub pane_id: String,
    /// `@ae_agent`, or `None` when unstamped.
    pub agent: Option<String>,
    /// Lines in tmux's history buffer.
    pub history_size: u64,
    /// Cursor column.
    pub cursor_x: u32,
    /// Cursor row.
    pub cursor_y: u32,
    /// Clients attached to this session.
    pub session_attached: u32,
}

/// The ticker's `list-panes -s -t <session> -F <MOTION_PANE_FORMAT>` argv.
#[must_use]
pub(crate) fn motion_panes_args(server: &ServerId, session: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(
        ["list-panes", "-s", "-t", session, "-F", MOTION_PANE_FORMAT].map(ToOwned::to_owned),
    );
    args
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
        let [
            pane_id,
            agent,
            history_size,
            cursor_x,
            cursor_y,
            session_attached,
        ] = fields.as_slice()
        else {
            return None;
        };
        panes.push(MotionPane {
            pane_id: (*pane_id).to_owned(),
            agent: (!agent.is_empty()).then(|| (*agent).to_owned()),
            history_size: history_size.parse().ok()?,
            cursor_x: cursor_x.parse().ok()?,
            cursor_y: cursor_y.parse().ok()?,
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

/// The session-scoped user option carrying the roster line.
pub const AGENTS_STATUS_OPTION: &str = "@ae_agents_status";

/// The WINDOW-scoped user option carrying that window's glyphs.
pub const WINDOW_STATUS_OPTION: &str = "@ae_window_status";

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

/// Set one user option on `target`, which MUST be an exact id.
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
    args.extend(["-t", target, name, value].map(ToOwned::to_owned));
    args
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
        args.push("set-option".to_owned());
        if let Some(flag) = write.scope.flag() {
            args.push(flag.to_owned());
        }
        args.extend([
            "-t".to_owned(),
            write.target.clone(),
            write.name.clone(),
            write.value.clone(),
        ]);
    }
    args
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
    args.push("set-option".to_owned());
    if let Some(flag) = scope.flag() {
        args.push(flag.to_owned());
    }
    args.extend(["-u", "-t", target, name].map(ToOwned::to_owned));
    args
}

/// The session option the watchdog publishes the work tree's branch into.
pub const BRANCH_OPTION: &str = "@ae_branch_name";

/// Read one session option's raw value — `show-options -t <session> -qv <name>`.
#[must_use]
pub fn session_option_args(server: &ServerId, session: &str, name: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["show-options", "-t", session, "-qv", name].map(ToOwned::to_owned));
    args
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

/// Show a transient message on `target`'s clients.
#[must_use]
pub fn display_message_args(server: &ServerId, target: &str, text: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(
        [
            "display-message",
            "-d",
            DISPLAY_MESSAGE_MS,
            "-t",
            target,
            text,
        ]
        .map(ToOwned::to_owned),
    );
    args
}

/// The format the fleet picker asks for: which session a pane belongs to, its
/// id, and the agent stamped on it.
pub const FLEET_PANE_FORMAT: &str = "#{session_name} | #{pane_id} | #{@ae_agent}";

/// How many [`FIELD_SEPARATOR`]-separated fields [`FLEET_PANE_FORMAT`] makes.
const FLEET_PANE_FIELDS: usize = 3;

/// One stamped pane of one session, as the caller's own server reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetPane {
    /// The session the pane belongs to.
    pub session: String,
    /// The `%<n>` pane id.
    pub pane: String,
    /// The `@ae_agent` stamp, empty on a pane no agent owns.
    pub agent: String,
}

/// The arguments listing EVERY pane on `server` — `-a`, deliberately, because
/// one listing answers "which of ae's sessions are reachable from this client,
/// and what are their panes" for the whole fleet at once.
#[must_use]
pub fn fleet_panes_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-panes", "-a", "-F", FLEET_PANE_FORMAT].map(ToOwned::to_owned));
    args
}

/// What a completed fleet-pane listing means.
///
/// A line with the wrong field count is dropped rather than guessed at: the
/// last field may legitimately hold the separator, so the split is bounded and
/// a short line is corruption.
#[must_use]
pub fn interpret_fleet_panes(succeeded: bool, stdout: &str) -> Option<Vec<FleetPane>> {
    if !succeeded {
        return None;
    }
    Some(
        stdout
            .lines()
            // Only the carriage return goes: an UNSTAMPED pane ends the line
            // with the separator and an empty field, and trimming that away
            // would drop the pane rather than report it stampless.
            .map(|line| line.trim_end_matches('\r'))
            .filter(|line| !line.is_empty())
            .filter_map(|line| {
                let fields: Vec<&str> = line.splitn(FLEET_PANE_FIELDS, FIELD_SEPARATOR).collect();
                let [session, pane, agent] = fields.as_slice() else {
                    return None;
                };
                Some(FleetPane {
                    session: (*session).to_owned(),
                    pane: (*pane).to_owned(),
                    agent: agent.trim_end().to_owned(),
                })
            })
            .collect(),
    )
}

/// Escape text that is about to enter a tmux MENU format.
///
/// Measured on tmux 3.7b: `display-menu` expands an item's name and its title
/// with the plain format expander, which reads `#` and leaves `%` alone — a
/// `%%` written for the status line renders in a menu as two characters. So
/// this escapes `#` and nothing else, and [`format_literal`] stays the escape
/// for the time-expanded contexts that do collapse `%%`.
#[must_use]
pub fn menu_literal(text: &str) -> String {
    text.replace('#', "##")
}

/// What choosing one menu row does.
pub enum MenuAction {
    /// Run this tmux command — built from ids this crate validated, never from
    /// text a session named itself.
    Run(String),
    /// Open a second menu.
    Open(Menu),
    /// Nothing: the row is drawn dim and cannot be chosen.
    Disabled,
}

/// One row of a tmux menu.
pub struct MenuItem {
    /// The visible text, unescaped — [`display_menu_args`] escapes it.
    pub label: String,
    /// The single-key shortcut, or empty for none.
    pub key: String,
    /// What choosing the row does.
    pub action: MenuAction,
}

/// A menu ae asks tmux to draw.
pub struct Menu {
    /// The bordered title, unescaped.
    pub title: String,
    /// The `#[…]` the title is drawn in — ae's own text, so it is emitted
    /// BEFORE the escape rather than through it. tmux has no menu-title style
    /// option, and the title is format-expanded, so this is where a colour for
    /// it can go at all.
    pub title_style: String,
    /// The rows, in the order they are drawn.
    pub items: Vec<MenuItem>,
}

/// Where every ae menu is drawn — the centre of the client, which is the one
/// position that needs neither a mouse nor a pane geometry to be sensible.
const MENU_POSITION: [&str; 4] = ["-x", "C", "-y", "C"];

/// tmux's marker for a row that is drawn dim and cannot be chosen.
const DISABLED_PREFIX: char = '-';

/// End of flags, before the first item.
///
/// Measured on tmux 3.7b: `display-menu` reads its flags with getopt, so a
/// FIRST item whose name carries the [`DISABLED_PREFIX`] is read as a flag and
/// the whole call fails with `unknown flag -g`. The separator ends the flags
/// before any name is read, whatever the rows turn out to be.
const END_OF_FLAGS: &str = "--";

/// The arguments that draw `menu` on `server`'s current client.
///
/// No `-c` and no `-t`: the client is the one this invocation's `$TMUX` already
/// selects, which is what a key binding and a popup both want.
#[must_use]
pub fn display_menu_args(server: &ServerId, menu: &Menu) -> Vec<String> {
    let mut args = server_args(server);
    args.push("display-menu".to_owned());
    args.extend(MENU_POSITION.map(ToOwned::to_owned));
    args.push("-T".to_owned());
    args.push(titled(menu));
    args.push(END_OF_FLAGS.to_owned());
    for item in &menu.items {
        args.extend(item_words(item));
    }
    args
}

/// One row as the three arguments tmux reads it from.
fn item_words(item: &MenuItem) -> Vec<String> {
    let label = menu_literal(&item.label);
    match &item.action {
        MenuAction::Run(command) => vec![label, item.key.clone(), command.clone()],
        MenuAction::Open(inner) => vec![label, item.key.clone(), menu_command(inner)],
        // The leading hyphen is tmux's own dim-and-unselectable marker; the two
        // empty arguments keep the row a triplet like every other.
        MenuAction::Disabled => vec![
            format!("{DISABLED_PREFIX}{label}"),
            String::new(),
            String::new(),
        ],
    }
}

/// The `-T` argument: the style ae chose, then the title it was given.
fn titled(menu: &Menu) -> String {
    format!("{}{}", menu.title_style, menu_literal(&menu.title))
}

/// `menu` as ONE tmux command word — what a row that opens a second menu runs.
#[must_use]
pub fn menu_command(menu: &Menu) -> String {
    let mut words = vec!["display-menu".to_owned()];
    words.extend(MENU_POSITION.map(ToOwned::to_owned));
    words.push("-T".to_owned());
    words.push(titled(menu));
    words.push(END_OF_FLAGS.to_owned());
    for item in &menu.items {
        words.extend(item_words(item));
    }
    words
        .iter()
        .map(|word| single_quoted(word))
        .collect::<Vec<String>>()
        .join(" ")
}

/// `word` as one token of a tmux command line.
///
/// tmux's parser reads single quotes exactly as a shell does — no expansion
/// and no escape inside — so a value carrying one could not be re-quoted, and
/// dropping it is the only representation left. Nothing loses text to that in
/// practice: labels arrive with their quotes already removed, and the commands
/// are built from grammar-checked session names and pane ids that cannot hold
/// one.
fn single_quoted(word: &str) -> String {
    format!("'{}'", word.replace('\'', ""))
}

/// The `ae next --attach` verbs, and the question of which one applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusVerb {
    /// Inside tmux — `attach-session` errors with "sessions should be nested
    /// with care" and is not what a pane wants anyway.
    SwitchClient,
    /// Outside tmux — there is no client to switch.
    AttachSession,
}

impl FocusVerb {
    /// The tmux command word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SwitchClient => "switch-client",
            Self::AttachSession => "attach-session",
        }
    }

    /// Which verb applies, given whether the caller is inside tmux.
    #[must_use]
    pub const fn for_inside(inside: bool) -> Self {
        if inside {
            Self::SwitchClient
        } else {
            Self::AttachSession
        }
    }
}

/// The command that hands the calling client to `session`, as ONE tmux command
/// word — what a menu row runs.
///
/// The target is UNQUOTED and safe because it is not free text: the caller
/// proves the name against the session grammar first, and that grammar admits
/// no space, quote or semicolon. A quote could not survive the nesting anyway,
/// since tmux's single quotes admit no escape inside them.
#[must_use]
pub fn switch_command(session: &str) -> String {
    format!("{} -t {session}", FocusVerb::SwitchClient.as_str())
}

/// The command that hands the calling client to `pane` of `session`.
///
/// The WINDOW before the pane: a worker lives in its own, and `select-pane`
/// alone does not change which window is viewed — the order the `focus` helper
/// uses. tmux resolves all three ids when the command runs, so a pane that died
/// in the meantime fails the jump rather than landing it somewhere else.
#[must_use]
pub fn jump_command(session: &str, pane: &str) -> String {
    format!(
        "{} ; select-window -t {pane} ; select-pane -t {pane}",
        switch_command(session)
    )
}

/// The arguments that focus `session` with `verb`.
#[must_use]
pub fn focus_args(server: &ServerId, verb: FocusVerb, session: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.push(verb.as_str().to_owned());
    args.push("-t".to_owned());
    args.push(session.to_owned());
    args
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
// The fleet strip's two reads.
// ---------------------------------------------------------------------------

/// The format the fleet strip asks `list-sessions` for: a session's name, the
/// `$<n>` id a click resolves through, and the attention rank its OWN watchdog
/// published.
///
/// The rank is what makes this an ae listing: a session with none is not one ae
/// watches, so the strip skips it rather than drawing a stranger.
pub const FLEET_SESSION_FORMAT: &str = "#{session_name} | #{session_id} | #{@ae_attn_rank}";

/// How many fields [`FLEET_SESSION_FORMAT`] yields.
const FLEET_SESSION_FIELDS: usize = 3;

/// The highest rank a session may publish — [`crate::theme::Mark`]'s own count.
const HIGHEST_RANK: u8 = 5;

/// One ae session as the fleet strip reads it off the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetSession {
    /// The session name.
    pub name: String,
    /// Its `$<n>` id.
    pub id: String,
    /// The published attention rank, proven to be one.
    pub rank: String,
}

/// The arguments listing every session on `server` with its published
/// attention.
#[must_use]
pub fn fleet_sessions_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["list-sessions", "-F", FLEET_SESSION_FORMAT].map(ToOwned::to_owned));
    args
}

/// What a completed fleet-session listing means.
///
/// A session with no rank is dropped: it is not an ae session, or its watchdog
/// has not run a cycle yet, and a strip that drew it would claim to know
/// something about it.
#[must_use]
pub fn interpret_fleet_sessions(succeeded: bool, stdout: &str) -> Option<Vec<FleetSession>> {
    if !succeeded {
        return None;
    }
    Some(
        stdout
            .lines()
            .map(|line| line.trim_end_matches(['\r', '\n']))
            .filter(|line| !line.is_empty())
            .filter_map(|line| {
                let fields: Vec<&str> =
                    line.splitn(FLEET_SESSION_FIELDS, FIELD_SEPARATOR).collect();
                let [name, id, rank] = fields.as_slice() else {
                    return None;
                };
                // PROVEN, not trusted. Every field of this row is rendered into
                // an option value the drawer reads `#[…]` out of, and the row
                // comes from a session ae did not necessarily create: a name
                // with a style directive in it would restyle the strip and tear
                // its click ranges. A session ae launched always passes.
                let (name, id, rank) = (name.trim(), id.trim(), rank.trim());
                let named = crate::session_launch::name::is_session_name(name);
                let identified = id.strip_prefix('$').is_some_and(|digits| {
                    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
                });
                let ranked = rank.parse::<u8>().is_ok_and(|rank| rank <= HIGHEST_RANK);
                (named && identified && ranked).then(|| FleetSession {
                    name: name.to_owned(),
                    id: id.to_owned(),
                    rank: rank.to_owned(),
                })
            })
            .collect(),
    )
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
    args.extend(["display-message", "-p", "-t", session, LOOK_FORMAT].map(ToOwned::to_owned));
    args
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

/// `#{pane_id} | #{window_id} | #{@ae_theme} | #{@ae_agent}` — the
/// window-grouping read.
///
/// The theme stamp rides along because the alternative is a second listing:
/// the watchdog has to know which windows are already dressed, and a user
/// option set on the WINDOW resolves in a pane's format context.
pub const WINDOW_PANE_FORMAT: &str = "#{pane_id} | #{window_id} | #{@ae_theme} | #{@ae_agent}";

/// The number of fields [`WINDOW_PANE_FORMAT`] yields.
const WINDOW_PANE_FIELDS: usize = 4;

/// One pane as the window grouping reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowPane {
    /// `#{pane_id}`.
    pub pane_id: String,
    /// `#{window_id}` — `@N`, server-global and stable for the window's life.
    pub window_id: String,
    /// `@ae_theme` — the window's look stamp, empty when it carries none.
    pub theme: String,
    /// `@ae_agent`, or `None` when unstamped.
    pub agent: Option<String>,
}

/// The arguments grouping `session`'s panes by window.
#[must_use]
pub fn window_panes_args(server: &ServerId, session: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(
        ["list-panes", "-s", "-t", session, "-F", WINDOW_PANE_FORMAT].map(ToOwned::to_owned),
    );
    args
}

/// One [`WindowPane`] per line that split into exactly [`WINDOW_PANE_FIELDS`]
/// fields; `None` on a failed run.
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
                let [pane_id, window_id, theme, agent] = fields.as_slice() else {
                    return None;
                };
                let agent = agent.trim_end();
                Some(WindowPane {
                    pane_id: (*pane_id).to_owned(),
                    window_id: (*window_id).to_owned(),
                    theme: (*theme).to_owned(),
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
    /// `C-u` — clear the input line, the notice path's one measurable retry.
    ClearLine,
    /// `-X cancel` — leave copy mode.
    CancelCopyMode,
}

/// The full argument list for sending one [`Key`] to `pane`.
#[must_use]
pub fn send_keys_args(server: &ServerId, pane: &str, key: Key) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["send-keys", "-t", pane].map(ToOwned::to_owned));
    match key {
        Key::Enter => args.push("Enter".to_owned()),
        Key::Escape => args.push("Escape".to_owned()),
        Key::ClearLine => args.push("C-u".to_owned()),
        Key::CancelCopyMode => {
            args.push("-X".to_owned());
            args.push("cancel".to_owned());
        }
    }
    args
}

/// Each attached client's own active pane and the epoch of its last input.
pub const CLIENT_FORMAT: &str = "#{pane_id} | #{client_activity}";

/// The number of fields [`CLIENT_FORMAT`] renders.
const CLIENT_FIELDS: usize = 2;

/// One attached client's [`CLIENT_FORMAT`] readings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedClient {
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
                    pane: fields[0].to_owned(),
                    activity: fields[1].parse().ok(),
                })
            })
            .collect(),
    )
}

/// The arguments creating a spawned agent's own WINDOW, printing its pane id.
#[must_use]
pub fn new_window_args(server: &ServerId, session: &str, work_dir: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(
        [
            "new-window",
            "-d",
            "-t",
            &format!("{session}:"),
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
mod tests {
    use super::{
        CLIENT_FORMAT, Key, ObservedClient, ObservedPaneProbe, PANE_PROBE_FORMAT, Styling,
        capture_screen_args, interpret_clients, interpret_pane_probe, list_clients_args,
        load_buffer_args, pane_probe_args, paste_buffer_args, send_keys_args,
    };

    #[test]
    fn the_delivery_argv_addresses_the_named_pane_and_never_selects_it() {
        let sock = ServerId::Selected(Selector::Socket(std::path::PathBuf::from("/tmp/s")));
        assert_eq!(
            pane_probe_args(&ServerId::Ambient, "%3"),
            vec!["display-message", "-p", "-t", "%3", PANE_PROBE_FORMAT]
        );
        assert_eq!(
            capture_screen_args(&ServerId::Ambient, "%3", Styling::Plain),
            vec!["capture-pane", "-p", "-t", "%3"],
            "the start-up marker scan wants the rows the TUI drew, unstyled"
        );
        assert_eq!(
            capture_screen_args(&ServerId::Ambient, "%3", Styling::Escapes),
            vec!["capture-pane", "-e", "-p", "-S", "0", "-t", "%3"],
            "the occupancy sensor decides live-vs-echo from SGR state alone"
        );
        assert_eq!(
            load_buffer_args(&sock, "b"),
            vec!["-S", "/tmp/s", "load-buffer", "-b", "b", "-"],
            "`-` is the SOURCE: the bytes ride stdin, never argv"
        );
        assert_eq!(
            paste_buffer_args(&ServerId::Ambient, "b", "%3"),
            vec!["paste-buffer", "-d", "-p", "-b", "b", "-t", "%3"],
            "-p REQUESTS bracketing; -d leaves no body in the buffer stack"
        );
        assert_eq!(
            send_keys_args(&ServerId::Ambient, "%3", Key::Enter),
            vec!["send-keys", "-t", "%3", "Enter"]
        );
        assert_eq!(
            send_keys_args(&ServerId::Ambient, "%3", Key::ClearLine),
            vec!["send-keys", "-t", "%3", "C-u"]
        );
        assert_eq!(
            send_keys_args(&ServerId::Ambient, "%3", Key::Escape),
            vec!["send-keys", "-t", "%3", "Escape"]
        );
        assert_eq!(
            send_keys_args(&ServerId::Ambient, "%3", Key::CancelCopyMode),
            vec!["send-keys", "-t", "%3", "-X", "cancel"],
            "a COMMAND, not a key — which is why the set is an enum"
        );
        assert_eq!(
            list_clients_args(&ServerId::Ambient),
            vec!["list-clients", "-F", CLIENT_FORMAT]
        );
        assert!(
            !send_keys_args(&ServerId::Ambient, "%3", Key::Enter)
                .iter()
                .any(|arg| arg == "select-pane"),
            "focus is not part of any TUI's submission contract"
        );
    }

    #[test]
    fn a_pane_probe_is_read_only_from_a_run_that_succeeded() {
        assert_eq!(interpret_pane_probe(false, "123 | claude\n"), None);
        assert_eq!(
            interpret_pane_probe(true, "123 | claude\n"),
            Some(ObservedPaneProbe {
                command: "claude".into(),
                pid: Some(123)
            })
        );
        assert_eq!(
            interpret_pane_probe(true, "not-a-pid | bash\n"),
            Some(ObservedPaneProbe {
                command: "bash".into(),
                pid: None
            }),
            "an unreadable pid is no pid, not no pane"
        );
        assert_eq!(interpret_pane_probe(true, "only-one-field\n"), None);
        assert_eq!(
            interpret_pane_probe(true, "7 | odd | command\n"),
            Some(ObservedPaneProbe {
                command: "odd | command".into(),
                pid: Some(7)
            }),
            "the free-text command is last, so it may carry the separator"
        );
        assert_eq!(interpret_pane_probe(true, ""), None);
    }

    #[test]
    fn clients_report_the_pane_each_is_viewing_and_when_it_last_typed() {
        assert_eq!(interpret_clients(false, "%1 | 100\n"), None);
        assert_eq!(
            interpret_clients(true, "%1 | 100\n%2 | nope\n\n"),
            Some(vec![
                ObservedClient {
                    pane: "%1".into(),
                    activity: Some(100)
                },
                ObservedClient {
                    pane: "%2".into(),
                    activity: None
                }
            ])
        );
        assert_eq!(
            interpret_clients(true, ""),
            Some(Vec::new()),
            "no clients attached is an ANSWER; a failed run is not"
        );
    }

    #[test]
    fn the_format_escape_doubles_hash_then_percent() {
        use super::format_literal;
        // `#(cmd)` in a tmux format RUNS A SHELL; `%` is strftime.
        assert_eq!(format_literal("plain text"), "plain text");
        assert_eq!(format_literal("#(id)"), "##(id)");
        assert_eq!(format_literal("#{session_name}"), "##{session_name}");
        assert_eq!(format_literal("100%"), "100%%");
        assert_eq!(format_literal("#%"), "##%%");
        assert_eq!(format_literal("#"), "##");
        // NOT idempotent, and must not be: escaping twice is a rendering bug,
        // so the one call site is the place that must not grow a second.
        assert_eq!(format_literal("##"), "####");
        // A hostile agent name is the realistic carrier.
        assert_eq!(
            format_literal("[ae watchdog] cl:#(touch /tmp/pwned) is DEAD"),
            "[ae watchdog] cl:##(touch /tmp/pwned) is DEAD"
        );
    }

    #[test]
    fn a_user_option_write_targets_an_exact_id_in_the_right_table() {
        use super::{
            AGENTS_STATUS_OPTION, OptionScope, WINDOW_STATUS_OPTION, set_option_args,
            unset_option_args,
        };
        use crate::inventory::ServerId;
        use crate::meta::Selector;
        let server = ServerId::Selected(Selector::Name("ae".to_owned()));
        assert_eq!(
            set_option_args(
                &server,
                OptionScope::Session,
                "$3",
                AGENTS_STATUS_OPTION,
                "lead● builder◌"
            ),
            [
                "-L",
                "ae",
                "set-option",
                "-t",
                "$3",
                "@ae_agents_status",
                "lead● builder◌"
            ]
        );
        // The window table needs -w, and the target is a window id.
        assert_eq!(
            set_option_args(
                &server,
                OptionScope::Window,
                "@7",
                WINDOW_STATUS_OPTION,
                "●◌"
            ),
            [
                "-L",
                "ae",
                "set-option",
                "-w",
                "-t",
                "@7",
                "@ae_window_status",
                "●◌"
            ]
        );
        // UNSET, not set-to-empty.
        assert_eq!(
            unset_option_args(&server, OptionScope::Session, "$3", AGENTS_STATUS_OPTION),
            [
                "-L",
                "ae",
                "set-option",
                "-u",
                "-t",
                "$3",
                "@ae_agents_status"
            ]
        );
        assert_eq!(
            unset_option_args(&server, OptionScope::Window, "@7", WINDOW_STATUS_OPTION),
            [
                "-L",
                "ae",
                "set-option",
                "-w",
                "-u",
                "-t",
                "@7",
                "@ae_window_status"
            ]
        );
    }

    #[test]
    fn option_writes_are_one_tmux_command_with_literal_argv_separators() {
        use super::{OptionScope, OptionWrite, set_options_args};
        use crate::inventory::ServerId;
        use crate::meta::Selector;
        let server = ServerId::Selected(Selector::Name("ae".to_owned()));
        let writes = [
            OptionWrite::new(
                OptionScope::Pane,
                "%1",
                "@ae_pane_state",
                "#[fg=#6897BB]⠋#[default] working",
            ),
            OptionWrite::new(OptionScope::Window, "@7", super::WINDOW_STATUS_OPTION, "⠋◌"),
        ];
        assert_eq!(
            set_options_args(&server, &writes),
            [
                "-L",
                "ae",
                "set-option",
                "-p",
                "-t",
                "%1",
                "@ae_pane_state",
                "#[fg=#6897BB]⠋#[default] working",
                ";",
                "set-option",
                "-w",
                "-t",
                "@7",
                "@ae_window_status",
                "⠋◌",
            ]
        );
    }

    #[test]
    fn the_transient_alert_carries_the_frozen_duration() {
        use super::display_message_args;
        use crate::inventory::ServerId;
        assert_eq!(
            display_message_args(&ServerId::Ambient, "$3", "[ae watchdog] cl:a is DEAD"),
            [
                "display-message",
                "-d",
                "10000",
                "-t",
                "$3",
                "[ae watchdog] cl:a is DEAD"
            ]
        );
    }

    #[test]
    fn the_window_grouping_read_drops_short_lines_and_reads_an_unstamped_pane_as_none() {
        use super::{WindowPane, interpret_window_panes, window_panes_args};
        use crate::inventory::ServerId;
        assert_eq!(
            window_panes_args(&ServerId::Ambient, "demo"),
            [
                "list-panes",
                "-s",
                "-t",
                "demo",
                "-F",
                super::WINDOW_PANE_FORMAT
            ]
        );
        let listing = "%1 | @0 | 1 | cl:lead\n%2 | @0 | 1 | \n%4 | @2 |  | cl:y\n%3 @1 cl:x\n";
        let panes = interpret_window_panes(true, listing).expect("a successful run");
        assert_eq!(
            panes,
            vec![
                WindowPane {
                    pane_id: "%1".to_owned(),
                    window_id: "@0".to_owned(),
                    theme: "1".to_owned(),
                    agent: Some("cl:lead".to_owned()),
                },
                WindowPane {
                    pane_id: "%2".to_owned(),
                    window_id: "@0".to_owned(),
                    theme: "1".to_owned(),
                    agent: None,
                },
                // An UNDRESSED window: the stamp is empty, which is the fact
                // the watchdog restamps on.
                WindowPane {
                    pane_id: "%4".to_owned(),
                    window_id: "@2".to_owned(),
                    theme: String::new(),
                    agent: Some("cl:y".to_owned()),
                },
            ],
            "the space-delimited line is corruption, not a pane"
        );
        assert!(interpret_window_panes(false, listing).is_none());
    }

    /// Every format this module hands tmux is read back by splitting on the
    /// bytes it asked for, and tmux 3.4 does not hand control characters back
    /// unchanged: measured on tmux 3.4 and tmux 3.7b, `\x1f` returns as the
    /// four literal bytes `\037` and a TAB returns as `_`.
    #[test]
    fn no_tmux_format_carries_a_control_character() {
        use super::{
            AGENTS_FORMAT, CLIENT_FORMAT, FLEET_PANE_FORMAT, MOTION_PANE_FORMAT, PANE_FORMAT,
            PANE_ID_FORMAT, PANE_PROBE_FORMAT, PANE_TTY_FORMAT, SESSION_ID_FORMAT,
            SESSION_NAME_FORMAT, SLOTS_FORMAT, VERSION_FORMAT, VIEWER_FORMAT, WATCH_PANE_FORMAT,
            WINDOW_PANE_FORMAT,
        };

        for format in [
            AGENTS_FORMAT,
            CLIENT_FORMAT,
            FLEET_PANE_FORMAT,
            MOTION_PANE_FORMAT,
            PANE_FORMAT,
            PANE_ID_FORMAT,
            PANE_PROBE_FORMAT,
            PANE_TTY_FORMAT,
            SESSION_ID_FORMAT,
            SESSION_NAME_FORMAT,
            SLOTS_FORMAT,
            VERSION_FORMAT,
            VIEWER_FORMAT,
            WATCH_PANE_FORMAT,
            WINDOW_PANE_FORMAT,
            super::FLEET_SESSION_FORMAT,
            super::LOOK_FORMAT,
        ] {
            assert!(
                !format.chars().any(char::is_control),
                "{format:?} carries a control character, which tmux 3.4 escapes"
            );
        }
    }

    /// The fleet strip is rendered from rows this reader hands it, straight
    /// into an option value the drawer reads styles out of — so a row that is
    /// not provably an ae session must never leave this function.
    #[test]
    fn a_fleet_row_is_proven_before_it_is_admitted() {
        use super::{FleetSession, interpret_fleet_sessions};

        let listing = "\
            good | $1 | 4\n\
            evil#[bg=red] | $2 | 4\n\
            spaced name | $3 | 4\n\
            badid | @4 | 4\n\
            badid2 | $ | 4\n\
            unranked | $5 | \n\
            overranked | $6 | 99\n\
            wordrank | $7 | four\n\
            -leading | $8 | 0\n\
            also-good | $9 | 0\n";
        let read = interpret_fleet_sessions(true, listing).unwrap_or_default();
        assert_eq!(
            read,
            vec![
                FleetSession {
                    name: "good".to_owned(),
                    id: "$1".to_owned(),
                    rank: "4".to_owned(),
                },
                FleetSession {
                    name: "also-good".to_owned(),
                    id: "$9".to_owned(),
                    rank: "0".to_owned(),
                },
            ],
            "only rows whose name, id and rank all check out"
        );
        assert!(interpret_fleet_sessions(false, listing).is_none());
    }

    #[test]
    fn the_watchdog_format_is_pinned_to_its_printable_separator() {
        use super::{WATCH_PANE_FORMAT, WATCH_PANE_SEPARATOR};

        let reconstructed = [
            "#{pane_id}",
            WATCH_PANE_SEPARATOR,
            "#{@ae_slot}",
            WATCH_PANE_SEPARATOR,
            "#{@ae_agent}",
            WATCH_PANE_SEPARATOR,
            "#{pane_pid}",
            WATCH_PANE_SEPARATOR,
            "#{pane_current_command}",
        ]
        .concat();
        assert_eq!(WATCH_PANE_FORMAT, reconstructed);
        assert!(
            !WATCH_PANE_FORMAT
                .chars()
                .any(|character| character.is_ascii_control())
        );
        assert_eq!(WATCH_PANE_FORMAT.matches(WATCH_PANE_SEPARATOR).count(), 4);
    }

    #[test]
    fn the_watchdog_pane_reading_widens_the_enumeration_and_refuses_malformed_lines() {
        use super::{
            WATCH_PANE_FORMAT, WatchPane, capture_pane_args, interpret_watch_panes,
            watch_panes_args,
        };
        use crate::inventory::ServerId;
        assert_eq!(
            watch_panes_args(&ServerId::Ambient, "s"),
            ["list-panes", "-s", "-t", "s", "-F", WATCH_PANE_FORMAT]
        );
        assert_eq!(
            capture_pane_args(&ServerId::Ambient, "%3"),
            [
                "capture-pane",
                "-p",
                "-J",
                "-S",
                "-40",
                "-E",
                "-",
                "-t",
                "%3"
            ]
        );
        let sep = super::WATCH_PANE_SEPARATOR;
        // A well-formed pane and a pane whose pid tmux could not print -> None
        // (never a guessed dead).
        let out = format!("%1{sep}main{sep}cl:lead{sep}9{sep}claude\n%2{sep}{sep}{sep}{sep}zsh\n");
        let panes = interpret_watch_panes(true, &out).expect("a successful enumeration");
        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].pane_pid, Some(9));
        assert_eq!(
            panes[1],
            WatchPane {
                pane_id: "%2".into(),
                slot: None,
                agent: None,
                current_command: "zsh".into(),
                pane_pid: None,
            }
        );
        assert!(
            interpret_watch_panes(true, "").unwrap().is_empty(),
            "no panes is an empty enumeration, not None"
        );
        assert!(
            interpret_watch_panes(true, "%bad | main").is_none(),
            "a line that cannot split is a failure of the whole reading"
        );
        assert!(
            interpret_watch_panes(true, "\n").is_none(),
            "non-empty output with no parseable lines is untrusted"
        );
    }

    #[test]
    fn the_motion_reading_is_one_strict_printable_snapshot() {
        use super::{MotionPane, interpret_motion_panes, motion_panes_args};
        use crate::inventory::ServerId;
        assert_eq!(
            motion_panes_args(&ServerId::Ambient, "s"),
            [
                "list-panes",
                "-s",
                "-t",
                "s",
                "-F",
                super::MOTION_PANE_FORMAT,
            ]
        );
        let listing = "%1 | lead | 42 | 7 | 9 | 1\n%2 |  | 0 | 0 | 0 | 1\n";
        assert_eq!(
            interpret_motion_panes(true, listing),
            Some(vec![
                MotionPane {
                    pane_id: "%1".to_owned(),
                    agent: Some("lead".to_owned()),
                    history_size: 42,
                    cursor_x: 7,
                    cursor_y: 9,
                    session_attached: 1,
                },
                MotionPane {
                    pane_id: "%2".to_owned(),
                    agent: None,
                    history_size: 0,
                    cursor_x: 0,
                    cursor_y: 0,
                    session_attached: 1,
                },
            ])
        );
        assert!(interpret_motion_panes(false, listing).is_none());
        assert!(interpret_motion_panes(true, "%1 | lead | bad | 7 | 9 | 1\n").is_none());
        assert!(interpret_motion_panes(true, "%1 | lead | 42 | 7 | 9\n").is_none());
    }

    #[test]
    fn the_watchdog_fixtures_preserve_tmux_version_framing() {
        use super::{WatchPane, interpret_watch_panes};

        // Literal output captured from tmux 3.4 with the printable separator.
        let tmux_3_4 = "%0 |  | cl:lead | 1234 | fish\n";
        assert_eq!(
            interpret_watch_panes(true, tmux_3_4),
            Some(vec![WatchPane {
                pane_id: "%0".into(),
                slot: None,
                agent: Some("cl:lead".into()),
                current_command: "fish".into(),
                pane_pid: Some(1234),
            }])
        );

        // Literal output captured from tmux 3.7b with the printable separator.
        let tmux_3_7b = "%1 | main | cl:lead | 4321 | claude\n";
        assert_eq!(
            interpret_watch_panes(true, tmux_3_7b),
            Some(vec![WatchPane {
                pane_id: "%1".into(),
                slot: Some("main".into()),
                agent: Some("cl:lead".into()),
                current_command: "claude".into(),
                pane_pid: Some(4321),
            }])
        );

        // The old tmux 3.4 control separator remains untrusted; the parser
        // must not normalize escaped producer output into data.
        let escaped_old = "%0\\037\\037cl:lead\\037fish\\0371234\n";
        assert!(interpret_watch_panes(true, escaped_old).is_none());

        let command_with_separator = "%2 | worker | cl:helper | 77 | tool | with separator\n";
        let panes = interpret_watch_panes(true, command_with_separator).expect("valid reading");
        assert_eq!(panes[0].current_command, "tool | with separator");
    }

    #[test]
    fn the_resolver_queries_are_the_frozen_ones() {
        use super::{AGENTS_FORMAT, SLOTS_FORMAT, agents_args, has_session_args, slots_args};
        use crate::inventory::ServerId;
        assert_eq!(
            has_session_args(&ServerId::Ambient, "other"),
            ["has-session", "-t", "other"]
        );
        assert_eq!(
            agents_args(&ServerId::Ambient, "s"),
            ["list-panes", "-s", "-t", "s", "-F", AGENTS_FORMAT]
        );
        assert_eq!(
            slots_args(&ServerId::Ambient, "s"),
            ["list-panes", "-s", "-t", "s", "-F", SLOTS_FORMAT]
        );
        assert_eq!(SLOTS_FORMAT, "#{pane_id}|#{@ae_slot}|#{@ae_agent}");
    }

    #[test]
    fn the_viewer_query_addresses_the_pane_and_asks_for_the_three_readings() {
        use super::{ObservedViewer, VIEWER_FORMAT, interpret_viewer, viewer_args};
        use crate::inventory::ServerId;
        assert_eq!(
            viewer_args(&ServerId::Ambient, "%7"),
            ["display-message", "-p", "-t", "%7", VIEWER_FORMAT]
        );
        // A stamped agent pane.
        assert_eq!(
            interpret_viewer(true, "main | aerewrite | cl:lead\n"),
            Some(ObservedViewer {
                slot: Some("main".to_owned()),
                session: Some("aerewrite".to_owned()),
                agent: Some("cl:lead".to_owned()),
            })
        );
        // An unstamped pane: unset options expand to empty, and empty is None.
        assert_eq!(
            interpret_viewer(true, " | aerewrite | \n"),
            Some(ObservedViewer {
                slot: None,
                session: Some("aerewrite".to_owned()),
                agent: None,
            })
        );
        // A failed run, a short line and a long line are all no identity.
        assert_eq!(interpret_viewer(false, "main | s | a:b\n"), None);
        // MORE THAN ONE RECORD is no identity either: a second line, however
        // well-formed the first, is content the query never asked for, and
        // reading the first would let it pick the record.
        assert_eq!(
            interpret_viewer(true, "main | s | a:b\nworker.0 | s | a:c\n"),
            None
        );
        assert_eq!(interpret_viewer(true, "main | s | a:b\n\n"), None);
        assert_eq!(interpret_viewer(true, "main | s | a:b\nx"), None);
        // One record with or without its terminating newline is the same record.
        assert_eq!(
            interpret_viewer(true, "main | s | a:b"),
            interpret_viewer(true, "main | s | a:b\n")
        );
        assert_eq!(interpret_viewer(true, "main | s\n"), None);
        assert_eq!(interpret_viewer(true, "main | s | a:b | extra\n"), None);
        assert_eq!(interpret_viewer(true, ""), None);
    }

    use super::{
        ObservedPane, SlotObservation, StopProbe, interpret_marker, interpret_panes,
        interpret_sessions, interpret_stopped, is_addressable_socket, list_panes_args,
        list_sessions_args, marker_args, server_args, slot_observation,
    };
    use crate::inventory::{QueryFailed, ServerId};
    use crate::meta::Selector;
    use std::path::{Path, PathBuf};

    fn named(name: &str) -> ServerId {
        ServerId::Selected(Selector::Name(name.to_owned()))
    }

    fn socket(path: &str) -> ServerId {
        ServerId::Selected(Selector::Socket(PathBuf::from(path)))
    }

    #[test]
    fn the_typed_selector_chooses_the_tmux_routing_flag() {
        assert_eq!(server_args(&ServerId::Ambient), Vec::<String>::new());
        assert_eq!(server_args(&named("work")), ["-L", "work"]);
        assert_eq!(server_args(&socket("/tmp/ae.sock")), ["-S", "/tmp/ae.sock"]);
    }

    #[test]
    fn a_name_and_a_socket_of_the_same_spelling_address_different_servers() {
        // -L /tmp/x and -S /tmp/x are not the same tmux.
        assert_ne!(
            server_args(&named("/tmp/x")),
            server_args(&socket("/tmp/x"))
        );
    }

    #[test]
    fn enumeration_asks_for_exact_names_and_nothing_else() {
        assert_eq!(
            list_sessions_args(&named("work")),
            ["-L", "work", "list-sessions", "-F", "#{session_name}"]
        );
        assert_eq!(
            list_sessions_args(&ServerId::Ambient),
            ["list-sessions", "-F", "#{session_name}"]
        );
    }

    #[test]
    fn the_marker_is_read_from_the_session_s_own_environment() {
        assert_eq!(
            marker_args(&socket("/tmp/ae.sock"), "my-feature"),
            [
                "-S",
                "/tmp/ae.sock",
                "show-environment",
                "-t",
                "my-feature",
                "AE_SESSION"
            ]
        );
    }

    #[test]
    fn a_failed_run_is_a_failure_whatever_it_printed() {
        // The bytes are identical in both arms; only the transport result moves.
        for payload in ["", "alpha\nbeta\n", "no server running on /tmp/x\n"] {
            assert_eq!(
                interpret_sessions(false, payload),
                Err(QueryFailed),
                "{payload:?}"
            );
        }
    }

    #[test]
    fn stop_probe_reads_presence_from_a_successful_listing() {
        // A SUCCESSFUL run is authoritative both ways.
        assert_eq!(
            interpret_stopped(true, "other\nsess\n", "", "sess"),
            StopProbe::Present
        );
        assert_eq!(
            interpret_stopped(true, "other\n", "", "sess"),
            StopProbe::Absent
        );
        assert_eq!(
            interpret_stopped(true, "", "", "sess"),
            StopProbe::Absent,
            "an empty SUCCESS proves the session is gone"
        );
        // Exact-name only: `sess` is NOT present in a list of `session` — tmux would
        // prefix-match, ae never does (the same guard `interpret_sessions` earns).
        assert_eq!(
            interpret_stopped(true, "session\n", "", "sess"),
            StopProbe::Absent,
            "a longer name that merely starts with the target is not the target"
        );
    }

    #[test]
    fn stop_probe_reads_a_clean_server_exit_as_absent() {
        // Killing the last session exits the server; the stale-socket diagnostic
        // is the PROOF the session is gone — the whole reason this classifier
        // exists apart from interpret_sessions, which calls the same bytes a failure.
        assert_eq!(
            interpret_stopped(false, "", "no server running on /tmp/x\n", "sess"),
            StopProbe::Absent
        );
    }

    #[test]
    fn stop_probe_reads_any_other_failure_as_unknown_never_absent() {
        // ENOENT / permission / refused prove nothing — a live server whose socket
        // was unlinked yields a connect error while it keeps running (11th-review B1).
        for stderr in [
            "",
            "error connecting to /tmp/x (No such file or directory)\n",
            "error connecting to /tmp/x (Permission denied)\n",
        ] {
            assert_eq!(
                interpret_stopped(false, "", stderr, "sess"),
                StopProbe::Unknown,
                "{stderr:?}"
            );
        }
    }

    #[test]
    fn stop_probe_anchors_clean_dead_at_the_line_start_never_a_substring() {
        // A server literally NAMED "no server running" once made a permission
        // error CONTAIN the words (10th-review B2).
        assert_eq!(
            interpret_stopped(
                false,
                "",
                "error: cannot reach the no server running on host\n",
                "sess"
            ),
            StopProbe::Unknown
        );
    }

    #[test]
    fn a_successful_run_yields_exactly_the_names_it_printed() {
        assert_eq!(
            interpret_sessions(true, "alpha\nbeta\n"),
            Ok(vec!["alpha".to_owned(), "beta".to_owned()])
        );
        assert_eq!(
            interpret_sessions(true, ""),
            Ok(Vec::new()),
            "an empty SUCCESS is the only thing that can prove a name absent"
        );
        assert_eq!(
            interpret_sessions(true, "solo\n\n"),
            Ok(vec!["solo".to_owned()]),
            "a blank line is not a session called nothing"
        );
    }

    #[test]
    fn a_name_containing_spaces_survives_intact() {
        // The format asks for one field, so the whole line is the name.
        assert_eq!(
            interpret_sessions(true, "my session\n"),
            Ok(vec!["my session".to_owned()])
        );
    }

    #[test]
    fn the_marker_is_the_value_and_an_unset_variable_is_not_one() {
        assert_eq!(
            interpret_marker(true, "AE_SESSION=my-feature\n"),
            Some("my-feature".to_owned())
        );
        assert_eq!(
            interpret_marker(true, "-AE_SESSION\n"),
            None,
            "tmux spells UNSET with a leading dash, and that is not a value"
        );
        assert_eq!(interpret_marker(true, ""), None);
        assert_eq!(
            interpret_marker(false, "AE_SESSION=my-feature\n"),
            None,
            "a failed run reports nothing, however convincing its output looks"
        );
        assert_eq!(
            interpret_marker(true, "AE_SESSION=\n"),
            Some(String::new()),
            "an empty value is a value; whether it PROVES ownership is liveness's question"
        );
    }

    /// A pane reading, spelled the way the fields arrive.
    fn pane(dead: Option<bool>, slot: Option<&str>, command: Option<&str>) -> ObservedPane {
        ObservedPane {
            dead,
            slot: slot.map(ToOwned::to_owned),
            command: command.map(ToOwned::to_owned),
        }
    }

    /// A pane with a usable slot and nothing else said about it.
    fn slotted(slot: &str) -> ObservedPane {
        pane(Some(false), Some(slot), Some("claude"))
    }

    /// A pane carrying no usable identity.
    fn unslotted() -> ObservedPane {
        pane(Some(false), None, Some("zsh"))
    }

    #[test]
    fn pane_enumeration_is_session_wide_and_asks_for_sc_017s_s_three_fields() {
        assert_eq!(
            list_panes_args(&named("work"), "my-feature"),
            [
                "-L",
                "work",
                "list-panes",
                "-s",
                "-t",
                "my-feature",
                "-F",
                "#{pane_dead} | #{@ae_slot} | #{pane_current_command}"
            ]
        );
    }

    #[test]
    fn the_pane_format_asks_for_identity_and_never_for_the_display_field() {
        // `@ae_agent` is DISPLAY, so identity is never associated on it.
        let args = list_panes_args(&ServerId::Ambient, "s").join(" ");
        assert!(args.contains("#{@ae_slot}"));
        assert!(args.contains("#{pane_dead}"), "{args}");
        assert!(args.contains("#{pane_current_command}"), "{args}");
        assert!(!args.contains("@ae_agent"), "{args}");
    }

    #[test]
    fn pane_dead_comes_first_so_nothing_upstream_can_shift_it() {
        // ORDER IS THE SAFETY PROPERTY.
        let fields: Vec<&str> = super::PANE_FORMAT.split(super::FIELD_SEPARATOR).collect();
        assert_eq!(fields.len(), super::PANE_FIELDS);
        assert_eq!(fields[0], "#{pane_dead}");
        assert_eq!(fields[2], "#{pane_current_command}", "free-est text last");
    }

    #[test]
    fn a_failed_pane_query_is_a_failure_whatever_it_printed() {
        for payload in ["", "0 | main | claude\n", "can't find window: nosuch\n"] {
            assert_eq!(
                interpret_panes(false, payload),
                Err(QueryFailed),
                "{payload:?}"
            );
        }
    }

    #[test]
    fn an_unmarked_pane_is_a_pane_and_not_a_dropped_line() {
        // MEASURED against a real server.
        assert_eq!(
            interpret_panes(true, "0 | main | zsh\n0 |  | zsh\n1 |  | true\n"),
            Ok(vec![
                pane(Some(false), Some("main"), Some("zsh")),
                pane(Some(false), None, Some("zsh")),
                pane(Some(true), None, Some("true")),
            ]),
            "three lines, three panes, and the unmarked one is still one"
        );
        assert_eq!(
            interpret_panes(true, ""),
            Ok(Vec::new()),
            "no output at all is no panes"
        );
    }

    #[test]
    fn an_exited_pane_keeps_both_facts_that_tell_it_apart_from_a_live_one() {
        // A `remain-on-exit` pane reports the EXITED
        // process's command, and `true` is not in shell set — so the
        // command field alone reads like a live agent. The only thing that
        // separates it from a live pane is `pane_dead`, and this pins that the
        // read carries it rather than discarding it.
        let exited = interpret_panes(true, "1 | worker | true\n").expect("success");
        assert_eq!(exited, vec![pane(Some(true), Some("worker"), Some("true"))]);
        assert_eq!(
            exited[0].dead,
            Some(true),
            "the conjunct that stops a dead agent reading alive survived the read"
        );
        assert_eq!(
            exited[0].command.as_deref(),
            Some("true"),
            "and so did the command that would otherwise have proven it alive"
        );
    }

    #[test]
    fn a_marker_that_is_empty_or_blank_is_not_a_usable_identity() {
        // RESTORED BY NAME, and not merely as bookkeeping.
        assert_eq!(
            interpret_panes(true, "0 |  | zsh\n"),
            Ok(vec![pane(Some(false), None, Some("zsh"))]),
            "an empty slot field is no identity"
        );
        assert_eq!(
            interpret_panes(true, "0 |     | zsh\n"),
            Ok(vec![pane(Some(false), None, Some("zsh"))]),
            "and neither is a whitespace-only one — same answer, different bytes"
        );
        assert_eq!(
            interpret_panes(true, "0 | main |    \n"),
            Ok(vec![pane(Some(false), Some("main"), None)]),
            "the command field normalizes the same way, and an unassociated \
             unreadable command in the not-alive set for the same reason"
        );
    }

    #[test]
    fn an_unreadable_field_is_no_reading_rather_than_a_convenient_one() {
        // An empty or absent reading is NOT alive, because absence of evidence
        // is not evidence.
        assert_eq!(
            interpret_panes(true, "0 | main | \n"),
            Ok(vec![pane(Some(false), Some("main"), None)]),
            "an empty command is no command, not a non-shell one"
        );
        assert_eq!(
            interpret_panes(true, " | main | claude\n"),
            Ok(vec![pane(None, Some("main"), Some("claude"))]),
            "an empty pane_dead is no reading, and must not pass for `0`"
        );
        assert_eq!(
            interpret_panes(true, "2 | main | claude\n"),
            Ok(vec![pane(None, Some("main"), Some("claude"))]),
            "and neither does anything else that is not 0 or 1"
        );
        assert_eq!(
            interpret_panes(true, "0 |  | claude\n"),
            Ok(vec![pane(Some(false), None, Some("claude"))]),
            "an empty slot is no identity; the other two readings survive it"
        );
    }

    #[test]
    fn a_line_of_the_wrong_arity_is_a_pane_that_says_nothing() {
        // A TAB CANNOT BE SMUGGLED THROUGH A FIELD.
        let forged = interpret_panes(true, "0 | main | evil | zsh\n").expect("success");
        assert_eq!(
            forged,
            vec![pane(None, None, None)],
            "still one pane, and it says nothing at all"
        );
        assert_eq!(
            interpret_panes(true, "\n"),
            Ok(vec![pane(None, None, None)]),
            "a blank line is a pane with no reading, never a dropped pane"
        );
        assert_eq!(
            interpret_panes(true, "0 | main\n"),
            Ok(vec![pane(None, None, None)]),
            "and too few fields is refused for the same reason as too many"
        );
    }

    #[test]
    fn the_two_interpreters_disagree_about_a_blank_line_on_purpose() {
        // A session called nothing does not exist; a pane that reported nothing
        // does.
        assert_eq!(
            interpret_sessions(true, "a\n\nb\n"),
            Ok(vec!["a".to_owned(), "b".to_owned()])
        );
        assert_eq!(
            interpret_panes(true, "a\n\nb\n").map(|panes| panes.len()),
            Ok(3)
        );
    }

    #[test]
    fn an_empty_roster_slot_matches_no_pane() {
        // A ROSTER SLOT CAN BE EMPTY: `absorb_roster` validates alias and name
        // and never the slot, so `agent.=cl:lead` in a hand-edited meta yields a
        // roster entry whose slot is "".
        let panes = interpret_panes(true, "0 |  | zsh\n0 | main | claude\n")
            .expect("a successful enumeration");
        assert_eq!(
            slot_observation(&panes, ""),
            SlotObservation::Absent { unidentified: 1 },
            "an empty roster slot matches no pane, including the unmarked one"
        );
    }

    #[test]
    fn a_slot_is_found_only_by_exact_match() {
        let panes = [slotted("main"), slotted("worker")];
        assert_eq!(slot_observation(&panes, "main"), SlotObservation::Unique);
        assert_eq!(
            slot_observation(&panes, "mai"),
            SlotObservation::Absent { unidentified: 0 },
            "a PREFIX of a slot that is there is absent, not present"
        );
        assert_eq!(
            slot_observation(&panes, "main2"),
            SlotObservation::Absent { unidentified: 0 },
            "and so is a slot that merely extends one"
        );
    }

    #[test]
    fn a_duplicated_slot_is_ambiguous_rather_than_a_match() {
        let panes = [slotted("main"), slotted("main")];
        assert_eq!(
            slot_observation(&panes, "main"),
            SlotObservation::Duplicated { panes: 2 },
            "two panes claiming one slot associate it to neither"
        );
    }

    #[test]
    fn absence_carries_how_many_panes_identified_nothing() {
        // The COUNT, not a conclusion.
        assert_eq!(
            slot_observation(&[slotted("other")], "main"),
            SlotObservation::Absent { unidentified: 0 }
        );
        assert_eq!(
            slot_observation(&[slotted("other"), unslotted()], "main"),
            SlotObservation::Absent { unidentified: 1 },
            "an unassociated pane is exactly the fact the reader needs"
        );
        assert_eq!(
            slot_observation(&[], "main"),
            SlotObservation::Absent { unidentified: 0 },
            "no panes at all identifies nothing and hides nothing"
        );
    }

    #[test]
    fn a_relative_socket_path_cannot_address_a_server() {
        assert!(is_addressable_socket(Path::new("/tmp/ae.sock")));
        assert!(!is_addressable_socket(Path::new("relative/ae.sock")));
    }

    #[test]
    fn a_failed_fleet_listing_is_a_failure_and_an_empty_one_is_an_empty_server() {
        use super::{FLEET_PANE_FORMAT, FleetPane, fleet_panes_args, interpret_fleet_panes};
        use crate::inventory::ServerId;
        use crate::meta::Selector;

        assert_eq!(
            fleet_panes_args(&ServerId::Selected(Selector::Name("ae-dev".to_owned()))),
            vec!["-L", "ae-dev", "list-panes", "-a", "-F", FLEET_PANE_FORMAT]
        );
        // The two must never collapse into one: a failed read that read as an
        // empty server would say every session is somewhere else.
        assert_eq!(interpret_fleet_panes(false, ""), None);
        assert_eq!(interpret_fleet_panes(false, "hub | %1 | lead\n"), None);
        assert_eq!(interpret_fleet_panes(true, ""), Some(Vec::new()));

        let listing = "hub | %1 | lead\nhub | %2 | \nhub | %3 | a | b\ntruncated\n";
        assert_eq!(
            interpret_fleet_panes(true, listing),
            Some(vec![
                FleetPane {
                    session: "hub".to_owned(),
                    pane: "%1".to_owned(),
                    agent: "lead".to_owned(),
                },
                FleetPane {
                    session: "hub".to_owned(),
                    pane: "%2".to_owned(),
                    agent: String::new(),
                },
                // The split is BOUNDED, so a separator inside the last field is
                // part of the stamp rather than a fourth field.
                FleetPane {
                    session: "hub".to_owned(),
                    pane: "%3".to_owned(),
                    agent: "a | b".to_owned(),
                },
            ]),
            "a line with too few fields is corruption, not a pane"
        );
    }

    #[test]
    fn a_menu_jump_spells_its_verb_with_the_one_focus_verb_and_takes_the_window_first() {
        use super::{FocusVerb, jump_command, switch_command};

        // The verb has ONE owner; a second spelling here could drift from it.
        assert!(switch_command("hub").starts_with(FocusVerb::SwitchClient.as_str()));
        assert_eq!(switch_command("hub"), "switch-client -t hub");
        let jump = jump_command("hub", "%12");
        assert_eq!(
            jump,
            "switch-client -t hub ; select-window -t %12 ; select-pane -t %12"
        );
        assert!(jump.starts_with(&switch_command("hub")));
        let window = jump.find("select-window").expect("a window step");
        let pane = jump.find("select-pane").expect("a pane step");
        assert!(window < pane, "{jump}");
    }

    #[test]
    fn the_version_is_asked_of_the_server_rather_than_of_the_binary_on_path() {
        use super::{VERSION_FORMAT, interpret_display_value, version_args};
        use crate::inventory::ServerId;
        use crate::meta::Selector;

        // `-V` would answer for whichever tmux is first on PATH; this asks the
        // server that is actually going to draw the menu.
        assert_eq!(
            version_args(&ServerId::Ambient),
            vec!["display-message", "-p", VERSION_FORMAT]
        );
        assert_eq!(
            version_args(&ServerId::Selected(Selector::Name("ae-dev".to_owned()))),
            vec!["-L", "ae-dev", "display-message", "-p", VERSION_FORMAT]
        );
        assert_eq!(
            interpret_display_value(true, "3.7b\n"),
            Some("3.7b".to_owned())
        );
        assert_eq!(interpret_display_value(false, "3.7b\n"), None);
    }

    #[test]
    fn the_focus_verb_follows_inside_ness_and_carries_the_target_separately() {
        use super::{FocusVerb, focus_args};
        use crate::inventory::ServerId;
        // Frozen's `_next_focus_verb`: attach-session errors inside tmux
        // ("sessions should be nested with care"), and there is no client to
        // switch outside it.
        assert_eq!(FocusVerb::for_inside(true), FocusVerb::SwitchClient);
        assert_eq!(FocusVerb::for_inside(false), FocusVerb::AttachSession);
        assert_eq!(FocusVerb::SwitchClient.as_str(), "switch-client");
        assert_eq!(FocusVerb::AttachSession.as_str(), "attach-session");

        // The target is its own argv element, never concatenated: frozen took
        // the trouble because "a session name with spaces must not be
        // word-split", and an argument vector is where that stays true.
        assert_eq!(
            focus_args(&ServerId::Ambient, FocusVerb::SwitchClient, "a b"),
            vec!["switch-client", "-t", "a b"]
        );
        assert_eq!(
            focus_args(&ServerId::Ambient, FocusVerb::AttachSession, "s"),
            vec!["attach-session", "-t", "s"]
        );
    }

    #[test]
    fn a_pane_tty_listing_is_a_list_and_a_failed_one_is_no_answer() {
        use super::{interpret_pane_ttys, pane_ttys_args};
        use crate::inventory::ServerId;
        assert_eq!(
            pane_ttys_args(&ServerId::Ambient),
            vec!["list-panes", "-a", "-F", "#{pane_tty}"],
            "-a: the question is whether THIS terminal is a pane of the server, \
             and the answer may live in any session"
        );
        assert_eq!(
            interpret_pane_ttys(true, "/dev/ttys001\n/dev/ttys002\n"),
            Some(vec!["/dev/ttys001".to_owned(), "/dev/ttys002".to_owned()])
        );
        assert_eq!(
            interpret_pane_ttys(true, ""),
            Some(Vec::new()),
            "a server with no panes ANSWERED"
        );
        assert_eq!(
            interpret_pane_ttys(false, "/dev/ttys001\n"),
            None,
            "a failed run is no answer, whatever it printed"
        );
    }

    #[test]
    fn the_tty_comparison_strips_dev_from_both_sides_exactly_once() {
        use super::tty_is_a_pane;
        // Procps prints `pts/3` and BSD ps `ttys039`, against tmux's absolute
        // `/dev/…`.
        let panes = vec!["/dev/ttys039".to_owned(), "/dev/pts/3".to_owned()];
        assert!(tty_is_a_pane("ttys039", &panes));
        assert!(tty_is_a_pane("pts/3", &panes));
        assert!(
            tty_is_a_pane("/dev/pts/3", &panes),
            "a ps that spelled the full path must not make every pane look stale"
        );
        assert!(
            !tty_is_a_pane("ttys001", &panes),
            "a real non-pane terminal"
        );
        assert!(!tty_is_a_pane("", &panes), "no tty is not every tty");
        assert!(
            !tty_is_a_pane("ttys039", &[]),
            "a server with no panes matches nothing"
        );
    }

    #[test]
    fn the_current_session_question_carries_no_target() {
        use super::current_session_args;
        use crate::inventory::ServerId;
        // No `-t`: the question is which session THIS client is in, and naming
        // a target would answer about the target instead.
        assert_eq!(
            current_session_args(&ServerId::Ambient),
            vec!["display-message", "-p", "#{session_name}"]
        );
    }
}
