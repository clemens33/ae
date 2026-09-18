//! `ae orchestrator --popup` — the fleet picker tmux draws for itself.
//!
//! One menu and no program: it lists the calling tmux server's live ae
//! sessions in attention order, and choosing a row hands the client to that
//! session's lead pane when the captured pane still belongs there. Nothing is
//! polled or stored; sessions, client bounds and membership are three tmux listings.
//!
//! Coming BACK is tmux's own `switch-client -l`: a picker that remembered
//! where you were would be a second answer to a question tmux already answers,
//! and two answers can disagree.

use std::io::Write;
use std::path::Path;

use crate::theme::{Mark, Palette};
use crate::tmux::{
    Menu, MenuAction, MenuItem, PickerPane, PickerSession, guarded_jump_client_id_command,
    guarded_jump_id_command, switch_client_id_command, switch_id_command,
};

/// `--help`, verbatim.
pub const USAGE: &str = "\
Usage: ae orchestrator [--popup|--settings --client <name> | --quota-dialog --client <name> --client-pid <pid> --server-pid <pid> --server-start <epoch> --session-id <$id> | --attach | --no-attach | --inside-tmux | --no-autostart]

Bare `ae orchestrator` starts or reattaches the orchestrator seat from its
dedicated config under ae's state home. With `--popup --client`, pick a live
session in a tmux menu and land in its lead pane. With `--settings --client`,
open the bottom-right settings menu. With `--quota-dialog` and its captured
client identity, draw the centred per-window quota dialog: the settings entry
supplies all four identity flags from the snapshot it drew against, and the
dialog reproves them immediately before drawing. Installed bindings supply the
client name.

The bare seat also accepts `_launch`'s `--attach`, `--no-attach`,
`--inside-tmux` and `--no-autostart` flags. Working-directory and archive flags
are picker usage errors.

The menu lists this tmux server's running ae sessions in attention order, then
creation order and name. Each row carries its live state, branch and goal.
Stopped ae sessions are listed after them, marked stopped, and choosing one
resumes it through the ordinary launch and hands this client to it; stopped
sessions recorded on another tmux server, or whose liveness this server cannot
prove, stay unlisted. Only the current session's agent roster expands under it,
never a stopped row's. Column widths follow the rows drawn. Choosing a running
row switches this client to the captured session id and selects its lead pane
when that pane still belongs there.

Coming back is tmux's own: switch-client -l (prefix + L by default).

Open it with prefix a on an ae-owned server.

Sessions on another tmux server are absent: tmux cannot switch a client across
servers, and the picker reads only the calling server.

";

/// The canonical orchestrator session name.
pub const ORCHESTRATOR_SESSION: &str = "orchestrator";

/// The seat's local-overlay config, relative to ae's state home.
pub(crate) const CONFIG_FILE: &str = "orchestrator.config";

/// The config seeded for the seat on its first launch.
pub(crate) const DEFAULT_CONFIG: &str =
    include_str!("../contrib/aeorchestrator/orchestrator.config");

/// Whether `path` is the canonical local overlay owned by the orchestrator
/// seat. A session named `orchestrator` can still be an ordinary project
/// session, so the overlay path—not the session name—decides identity scope.
#[must_use]
pub(crate) fn is_seat_overlay(path: &Path, home: &Path) -> bool {
    path == home.join(CONFIG_FILE)
}

/// The code a refused orchestrator invocation takes.
pub const EXIT_USAGE: u8 = 2;

/// What the argv asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// `--popup`: draw the picker.
    pub popup: bool,
    /// `--settings`: draw the bottom-right settings menu.
    pub settings: bool,
    /// `--quota-dialog`: draw the centred per-window quota dialog.
    pub quota_dialog: bool,
    /// `--client <name>`: draw on the client that opened a status menu.
    pub client: Option<String>,
    /// `--client-pid <pid>`: the invoking client's pid, carried only by the
    /// quota-dialog continuation so it can reprove the client at draw time.
    pub client_pid: Option<String>,
    /// `--server-pid <pid>`: the invoking server's pid, same reproof.
    pub server_pid: Option<String>,
    /// `--server-start <epoch>`: the invoking server's start time, same reproof.
    pub server_start: Option<String>,
    /// `--session-id <$id>`: the session the invoking client viewed when the
    /// settings entry was drawn, so a later session switch reads no overlay.
    pub session_id: Option<String>,
}

/// An argv `ae orchestrator` refuses, or the help it treats as one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Usage {
    /// `-h` / `--help` — the text, at exit 0.
    Help,
    /// A word this command does not accept, carried for its message.
    Unknown(String),
    /// `--client` was not followed by a nonempty client name.
    MissingClient,
    /// One invocation may target only one client.
    DuplicateClient,
    /// A quota-dialog identity flag was not followed by a nonempty value.
    MissingQuotaValue(&'static str),
    /// A quota-dialog identity flag may be given only once.
    DuplicateQuotaValue(&'static str),
}

impl Usage {
    /// The stderr this refusal prints, terminating newline included.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::Help => USAGE.to_owned(),
            Self::Unknown(token) => format!(
                "ae orchestrator: unknown argument '{token}' (see: ae orchestrator --help)\n"
            ),
            Self::MissingClient => {
                "ae orchestrator: --client requires a nonempty tmux client name\n".to_owned()
            }
            Self::DuplicateClient => {
                "ae orchestrator: --client may be given only once\n".to_owned()
            }
            Self::MissingQuotaValue(flag) => {
                format!("ae orchestrator: {flag} requires a nonempty value\n")
            }
            Self::DuplicateQuotaValue(flag) => {
                format!("ae orchestrator: {flag} may be given only once\n")
            }
        }
    }

    /// The exit it takes.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Self::Help => 0,
            Self::Unknown(_)
            | Self::MissingClient
            | Self::DuplicateClient
            | Self::MissingQuotaValue(_)
            | Self::DuplicateQuotaValue(_) => EXIT_USAGE,
        }
    }
}

/// Read `ae orchestrator`'s flags.
///
/// # Errors
///
/// [`Usage::Help`] for the help spellings, or the first malformed option.
///
/// ```
/// use ae::orchestrator::{parse, Args, Usage};
/// assert_eq!(
///     parse(&["--popup".to_owned()]),
///     Ok(Args { popup: true, settings: false, quota_dialog: false, client: None, client_pid: None, server_pid: None, server_start: None, session_id: None })
/// );
/// assert_eq!(parse(&[]), Ok(Args { popup: false, settings: false, quota_dialog: false, client: None, client_pid: None, server_pid: None, server_start: None, session_id: None }));
/// ```
pub fn parse(tail: &[String]) -> Result<Args, Usage> {
    let mut args = Args {
        popup: false,
        settings: false,
        quota_dialog: false,
        client: None,
        client_pid: None,
        server_pid: None,
        server_start: None,
        session_id: None,
    };
    let mut rest = tail;
    while let Some((word, after)) = rest.split_first() {
        rest = after;
        match word.as_str() {
            "-h" | "--help" => return Err(Usage::Help),
            "--popup" => args.popup = true,
            "--settings" => args.settings = true,
            "--quota-dialog" => args.quota_dialog = true,
            "--client-pid" | "--server-pid" | "--server-start" | "--session-id" => {
                let flag = word.as_str();
                let (name, slot): (&'static str, &mut Option<String>) = match flag {
                    "--client-pid" => ("--client-pid", &mut args.client_pid),
                    "--server-pid" => ("--server-pid", &mut args.server_pid),
                    "--server-start" => ("--server-start", &mut args.server_start),
                    _ => ("--session-id", &mut args.session_id),
                };
                if slot.is_some() {
                    return Err(Usage::DuplicateQuotaValue(name));
                }
                let Some((value, after_value)) = rest.split_first() else {
                    return Err(Usage::MissingQuotaValue(name));
                };
                if value.is_empty() {
                    return Err(Usage::MissingQuotaValue(name));
                }
                *slot = Some(value.clone());
                rest = after_value;
            }
            "--client" => {
                if args.client.is_some() {
                    return Err(Usage::DuplicateClient);
                }
                let Some((client, after_client)) = rest.split_first() else {
                    return Err(Usage::MissingClient);
                };
                if client.is_empty() {
                    return Err(Usage::MissingClient);
                }
                args.client = Some(client.clone());
                rest = after_client;
            }
            other => return Err(Usage::Unknown(other.to_owned())),
        }
    }
    if [args.popup, args.settings, args.quota_dialog]
        .into_iter()
        .filter(|selected| *selected)
        .count()
        > 1
    {
        return Err(Usage::Unknown(
            "only one of --popup, --settings, --quota-dialog per invocation".to_owned(),
        ));
    }
    // The pid pair belongs to the quota-dialog continuation: on any other
    // invocation it is malformed, not ignored — before this flag existed the
    // same spelling hit the unknown-argument arm.
    if !args.quota_dialog {
        for flag in [
            "--client-pid",
            "--server-pid",
            "--server-start",
            "--session-id",
        ] {
            let present = match flag {
                "--client-pid" => args.client_pid.is_some(),
                "--server-pid" => args.server_pid.is_some(),
                "--server-start" => args.server_start.is_some(),
                _ => args.session_id.is_some(),
            };
            if present {
                return Err(Usage::Unknown(format!("{flag} without --quota-dialog")));
            }
        }
    }
    Ok(args)
}

/// Parsed flags for a bare orchestrator-seat launch.
pub struct LaunchTail {
    /// A typed attach override, when present.
    pub attach: Option<bool>,
    /// Whether the caller identified itself as already inside tmux.
    pub inside_tmux: bool,
    /// Whether this launch suppresses companion autostart.
    pub no_autostart: bool,
}

/// Parse a bare seat launch tail through the public launch grammar. Shape,
/// origin and lineage flags remain unavailable because the seat has one fixed
/// local session and directory mode.
#[must_use]
pub fn parse_launch_tail(tail: &[String]) -> Option<LaunchTail> {
    let mut public = Vec::new();
    let mut inside_tmux = false;
    let mut no_autostart = false;
    for word in tail {
        match word.as_str() {
            "--inside-tmux" => inside_tmux = true,
            "--no-autostart" => no_autostart = true,
            _ => public.push(word.clone()),
        }
    }
    let plan = crate::session_launch::parse_plan_with_attach(&public, true).ok()?;
    if plan.mode.is_some()
        || plan.name.is_some()
        || plan.main.is_some()
        || plan.from.is_some()
        || plan.workers.is_some()
        || plan.dir.is_some()
    {
        return None;
    }
    Some(LaunchTail {
        attach: plan.attach,
        inside_tmux,
        no_autostart,
    })
}

/// Whether a bare seat launch tail uses only its fixed launch flags.
#[must_use]
pub fn launch_tail_is_valid(tail: &[String]) -> bool {
    parse_launch_tail(tail).is_some()
}

/// The user-facing launch tail for the canonical orchestrator seat.
#[must_use]
pub fn seat_launch_args() -> Vec<String> {
    vec![ORCHESTRATOR_SESSION.to_owned()]
}

/// The resume continuation's fixed hostile grammar, read from the menu row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapturedResume {
    name: String,
    client: String,
    client_pid: String,
    server_pid: String,
    server_start: String,
    deadline: i64,
}

/// Whether the public orchestrator word carries the picker's resume
/// continuation, decided by its first tail word alone.
#[must_use]
pub(crate) fn is_resume(tail: &[String]) -> bool {
    tail.first().is_some_and(|word| word == PICKER_RESUME_FLAG)
}

/// Read [`PICKER_RESUME_FLAG`]'s grammar: the session name first, then each
/// identity flag exactly once. Every value is validated by the grammar that
/// already owns it — a name by the session grammar, a client by the menu
/// client grammar, a pid by decimal, the deadline by epoch parse.
fn parse_resume(tail: &[String]) -> Result<CapturedResume, String> {
    let [flag, rest @ ..] = tail else {
        return Err("incomplete picker resume invocation".to_owned());
    };
    if flag != PICKER_RESUME_FLAG {
        return Err("not a picker resume invocation".to_owned());
    }
    let Some((name, after_name)) = rest.split_first() else {
        return Err("the picker resume needs a session name".to_owned());
    };
    if !crate::session_launch::name::is_session_name(name) {
        return Err("the picker resume target is not an ae session name".to_owned());
    }
    let mut client = None;
    let mut client_pid = None;
    let mut server_pid = None;
    let mut server_start = None;
    let mut deadline = None;
    let mut remaining = after_name;
    while let [flag, after @ ..] = remaining {
        let Some((value, after)) = after.split_first() else {
            return Err("a picker resume flag is missing its value".to_owned());
        };
        let slot: &mut Option<String> = match flag.as_str() {
            "--client" => &mut client,
            "--client-pid" => &mut client_pid,
            "--server-pid" => &mut server_pid,
            "--server-start" => &mut server_start,
            "--deadline" => &mut deadline,
            other => return Err(format!("unknown picker resume flag {other:?}")),
        };
        if slot.replace(value.clone()).is_some() {
            return Err(format!("{flag} may be given only once"));
        }
        remaining = after;
    }
    let required =
        |value: Option<String>, flag: &str| value.ok_or_else(|| format!("{flag} is required"));
    let client = required(client, "--client")?;
    if !crate::settings_menu::is_client_name(&client) {
        return Err("--client is not an addressable tmux client".to_owned());
    }
    let decimal = |value: Option<String>, flag: &str| -> Result<String, String> {
        let value = required(value, flag)?;
        if !crate::tmux::is_decimal(&value) {
            return Err(format!("{flag} is not a decimal"));
        }
        Ok(value)
    };
    let client_pid = decimal(client_pid, "--client-pid")?;
    let server_pid = decimal(server_pid, "--server-pid")?;
    let server_start = decimal(server_start, "--server-start")?;
    let deadline = required(deadline, "--deadline")?
        .parse::<i64>()
        .map_err(|_| "--deadline is not an epoch second".to_owned())?;
    Ok(CapturedResume {
        name: name.clone(),
        client,
        client_pid,
        server_pid,
        server_start,
        deadline,
    })
}

/// Resume one stopped picker row through the ordinary launch owner, then hand
/// the captured client to the session it resumed.
///
/// The clicker identity and deadline travel with the row and are re-proven
/// under the same expectation the settings menu uses; the launch itself — the
/// meta re-read, the absence proof on the recorded server, create-vs-resume —
/// happens exactly where `ae <name>` makes it happen. On success this client is
/// switched to the resumed session, which is what choosing a row means.
#[allow(
    clippy::too_many_lines,
    reason = "one action from captured identity proof through launch and client hand-off"
)]
pub(crate) fn run_resume(
    preamble: &crate::entry::Preamble,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let captured = match parse_resume(tail) {
        Ok(captured) => captured,
        Err(why) => {
            writeln!(err, "ae picker: {why}.")?;
            return Ok(crate::entry::EXIT_USAGE);
        }
    };
    let Some(server) = preamble.caller_server.clone() else {
        report_resume(
            None,
            None,
            &captured,
            "no calling tmux server; nothing was resumed",
            err,
        );
        return Ok(crate::entry::EXIT_FAILED);
    };
    let expectation = crate::session_launch::ExpectedLaunch::new(
        crate::session_launch::ExpectedState::StoppedSession,
        server.clone(),
        captured.server_pid.clone(),
        captured.server_start.clone(),
        captured.client.clone(),
        captured.client_pid.clone(),
        captured.deadline,
    );
    if let Err(why) = expectation.check_action(crate::time::Timestamp::now().epoch()) {
        report_resume(
            Some(&server),
            Some(&expectation),
            &captured,
            &format!("{why}; nothing was resumed"),
            err,
        );
        return Ok(crate::entry::EXIT_FAILED);
    }
    let deps = crate::doctor::check_deps(&[], err)?;
    if deps != 0 {
        report_resume(
            Some(&server),
            Some(&expectation),
            &captured,
            "launch dependencies are unavailable; nothing was resumed",
            err,
        );
        return Ok(deps);
    }
    let mut seat = preamble.clone();
    seat.attach = false;
    let mut launch_out = Vec::new();
    let mut launch_err = Vec::new();
    let code = crate::session_launch::run_expected(
        &seat,
        std::slice::from_ref(&captured.name),
        &expectation,
        &mut launch_out,
        &mut launch_err,
    )?;
    out.write_all(&launch_out)?;
    err.write_all(&launch_err)?;
    if code != 0 {
        let detail = String::from_utf8_lossy(&launch_err)
            .lines()
            .next()
            .unwrap_or("launch refused")
            .to_owned();
        report_resume(Some(&server), Some(&expectation), &captured, &detail, err);
        return Ok(code);
    }
    if let Err(why) = expectation.check_attachment() {
        report_resume(
            Some(&server),
            Some(&expectation),
            &captured,
            &format!(
                "resumed '{}', but this client is gone ({why})",
                captured.name
            ),
            err,
        );
        return Ok(crate::entry::EXIT_FAILED);
    }
    if !crate::transport::switch_client(&server, &captured.client, &captured.name) {
        report_resume(
            Some(&server),
            Some(&expectation),
            &captured,
            &format!(
                "resumed '{}', but tmux refused to switch this client; it is on this server",
                captured.name
            ),
            err,
        );
        return Ok(crate::entry::EXIT_FAILED);
    }
    Ok(0)
}

/// Report a picker action to the still-proven clicker, then always to stderr.
///
/// The narrower attachment proof is deliberate: only a client that still IS
/// the one that clicked may receive a message about the row.
fn report_resume(
    server: Option<&crate::inventory::ServerId>,
    expectation: Option<&crate::session_launch::ExpectedLaunch>,
    captured: &CapturedResume,
    text: &str,
    err: &mut impl Write,
) {
    if let (Some(server), Some(expectation)) = (server, expectation)
        && expectation.check_attachment().is_ok()
    {
        let _ = crate::transport::display_client_message(server, &captured.client, text);
    }
    let _ = writeln!(err, "ae picker: {text}");
}

/// At most this many session rows, so the menu fits a terminal and the key
/// alphabet below is never exhausted.
pub const ROW_CAP: usize = 30;

/// The shortcut keys, in the order rows take them. No `q`: tmux closes a menu
/// on `q`, and a row that stole it would trap the human in the picker.
const KEYS: &str = "123456789abcdefghijklmnoprstuvwxyz";

/// The widest a name column is ever drawn, session row or agent row.
const NAME_CAP: usize = 18;

/// The widest a state-word column is ever drawn, session row or agent row.
///
/// `waiting-agent` is 13 terminal cells, the longest word in the declared
/// vocabulary; at the old cap of 9 BOTH `waiting-user` (12) and `waiting-agent`
/// (13) clipped to the identical `waiting-…`, so the human could not tell "you
/// are needed" from "an agent is needed" in the one surface they glance at.
/// 13 covers the whole vocabulary. The cost is bounded and paid only by draws
/// that actually contain a 10–13-cell state: the state column is shared, so
/// such a draw widens it by at most 4 cells and the row's existing
/// `clip_cells(label, inner_width)` bounds what the tail loses.
const STATE_CAP: usize = 13;

/// The widest the branch column is ever drawn.
const BRANCH_CAP: usize = 14;

/// The widest the agent profile column is ever drawn.
const PROFILE_CAP: usize = 12;

/// How much of a goal survives into a row.
const GOAL_WIDTH: usize = 36;

/// The narrowest a dynamic column is drawn.
///
/// A column whose rows are all empty still takes this much, so the columns
/// after it stay where the rows above and below them put it.
const MIN_COLUMN_WIDTH: usize = 4;

/// The column widths ONE draw uses: the widest content among the rows that draw
/// actually shows, floored and capped.
///
/// The name and state columns are SHARED between session rows and the agent
/// rows nested under them, so both kinds of row stay in one grid. Widths are
/// per draw rather than fixed because a fleet of short names drawn at the cap is
/// a field of blanks the reader has to scan past.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Columns {
    name: usize,
    state: usize,
    branch: usize,
    profile: usize,
}

impl Columns {
    /// Every column at its floor — what a draw with no rows would use.
    const FLOOR: Self = Self {
        name: MIN_COLUMN_WIDTH,
        state: MIN_COLUMN_WIDTH,
        branch: MIN_COLUMN_WIDTH,
        profile: MIN_COLUMN_WIDTH,
    };

    /// Widen `field` to hold `text`, never past `cap`.
    fn widen(field: &mut usize, text: &str, cap: usize) {
        *field = (*field).max(terminal_cells(text).min(cap));
    }

    fn hold_session(&mut self, name: &str, state: &str, branch: &str) {
        Self::widen(&mut self.name, name, NAME_CAP);
        Self::widen(&mut self.state, state, STATE_CAP);
        Self::widen(&mut self.branch, branch, BRANCH_CAP);
    }

    fn hold_agent(&mut self, name: &str, profile: &str, state: &str) {
        Self::widen(&mut self.name, name, NAME_CAP);
        Self::widen(&mut self.profile, profile, PROFILE_CAP);
        Self::widen(&mut self.state, state, STATE_CAP);
    }
}

/// The terminal facts that bound one menu draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickerBounds {
    /// Client rows, including the two menu-border rows.
    pub height: usize,
    /// Client columns, including the four menu-border columns.
    pub width: usize,
    /// Clock used to reject stale watchdog facts.
    pub now_epoch: i64,
}

/// Why a picker is refused before tmux can silently drop it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerRefusal {
    /// The client snapshot cannot hold a useful bordered menu.
    TinyClient,
}

impl PickerRefusal {
    /// Stable stderr detail for the command boundary.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::TinyClient => {
                "client is too small for the picker (need at least 6 rows and 8 columns)"
            }
        }
    }
}

/// How much of the rosters one draw shows.
///
/// There is no expand-everything state: the human wants the roster under the
/// session they are looking at and nothing else, so a draw that could afford
/// every roster still shows only the current one's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expansion {
    /// The current session's roster fits under the session rows.
    Current,
    /// Not even that fits: every roster collapses to its summary suffix.
    Collapsed,
    /// Not even the session rows fit: the tail goes behind an omission row.
    Capped,
}

/// One STOPPED ae session the durable reader proved absent from the calling
/// server, as the picker draws it.
///
/// The caller supplies these rows already ordered (most recently live first);
/// the builder draws them after the running rows and never expands a roster
/// under one — a stopped session has no agents to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerStopped {
    /// The durable session name.
    pub name: String,
    /// The recorded goal, empty when the record carries none.
    pub goal: String,
    /// The recorded git branch, empty when the record carries none.
    pub branch: String,
    /// The last moment only a live session produces, when the record dates one.
    pub last_live: Option<i64>,
}

/// The captured clicker a stopped row's resume is pinned to.
///
/// Every field was read from the invoking client and its server at DRAW time;
/// the continuation re-proves them before it launches anything, so a row a
/// different attachment picked up cannot resume a session on its behalf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerResume<'a> {
    /// The client that opened the picker.
    pub client: &'a str,
    /// That client's process at draw time.
    pub client_pid: &'a str,
    /// The invoking server's process at draw time.
    pub server_pid: &'a str,
    /// The invoking server's start second at draw time.
    pub server_start: &'a str,
    /// The epoch after which the row is too stale to act on.
    pub deadline: i64,
    /// The argv prefix that re-executes this core (`picker_launcher`).
    pub launcher: &'a [String],
}

/// The picker, as a menu model — the whole pure step.
#[must_use]
pub fn menu(
    sessions: &[PickerSession],
    panes: &[PickerPane],
    icons: bool,
    palette: &Palette,
) -> Menu {
    menu_for_client(sessions, panes, icons, palette, None)
}

/// The picker model with every action pinned to the client that opened it.
#[must_use]
pub fn menu_for_client(
    sessions: &[PickerSession],
    panes: &[PickerPane],
    icons: bool,
    palette: &Palette,
    client: Option<&str>,
) -> Menu {
    menu_for_client_session(sessions, panes, icons, palette, client, None)
}

/// The picker model with actions that clear the open marker from the session
/// where this menu was drawn before they switch the client elsewhere.
#[must_use]
pub fn menu_for_client_session(
    sessions: &[PickerSession],
    panes: &[PickerPane],
    icons: bool,
    palette: &Palette,
    client: Option<&str>,
    opened_session: Option<&str>,
) -> Menu {
    build_menu(
        sessions,
        &[],
        panes,
        icons,
        palette,
        client,
        opened_session,
        None,
        PickerBounds {
            height: usize::MAX,
            width: usize::MAX,
            now_epoch: crate::time::Timestamp::now().epoch(),
        },
        // The UNBOUNDED convenience entry: it holds no state root, so it has no
        // global config to read a fleet order out of. The real picker threads
        // the human's order through `menu_for_client_session_in`; this one draws
        // the order ae drew before the key existed.
        &crate::theme::FleetOrder::EMPTY,
    )
}

/// The picker model constrained by the exact calling-client snapshot.
///
/// `stopped` arrives already ordered by its caller and is drawn after the
/// running rows. `resume` pins every stopped row's action to the client that
/// opened the picker; without it those rows draw dim and unselectable rather
/// than offering an action nothing could prove.
///
/// # Errors
///
/// [`PickerRefusal::TinyClient`] before any tmux draw when the border itself
/// would leave no useful menu surface.
#[allow(
    clippy::too_many_arguments,
    reason = "one pure build call carrying both row sources and the resume pin"
)]
pub fn menu_for_client_session_in(
    sessions: &[PickerSession],
    stopped: &[PickerStopped],
    panes: &[PickerPane],
    icons: bool,
    palette: &Palette,
    client: Option<&str>,
    opened_session: Option<&str>,
    resume: Option<&PickerResume<'_>>,
    bounds: PickerBounds,
    order: &crate::theme::FleetOrder,
) -> Result<Menu, PickerRefusal> {
    if bounds.height < 6 || bounds.width < 8 {
        return Err(PickerRefusal::TinyClient);
    }
    Ok(build_menu(
        sessions,
        stopped,
        panes,
        icons,
        palette,
        client,
        opened_session,
        resume,
        bounds,
        order,
    ))
}

/// The state word every stopped row draws beside its glyph.
///
/// The mark is `Dead`'s own glyph — the process behind the pane IS gone — and
/// this word keeps the row from reading as the attention verdict `dead`: a
/// stopped session is a fleet fact, not a verdict, and it is never ranked.
const STOPPED_WORD: &str = "stopped";

/// The resume continuation's flag word, unique enough that it can never be
/// mistaken for a session name or a menu flag.
pub(crate) const PICKER_RESUME_FLAG: &str = "--picker-resume";

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "one ordered fit-and-render pass over the picker snapshot"
)]
fn build_menu(
    sessions: &[PickerSession],
    stopped: &[PickerStopped],
    panes: &[PickerPane],
    icons: bool,
    palette: &Palette,
    client: Option<&str>,
    opened_session: Option<&str>,
    resume: Option<&PickerResume<'_>>,
    bounds: PickerBounds,
    order: &crate::theme::FleetOrder,
) -> Menu {
    let ranked = ranked_sessions(sessions, order);
    let need_you = ranked
        .iter()
        .filter(|session| {
            matches!(
                Mark::from_rank_value(session.rank),
                Mark::NeedsYou | Mark::Dead
            )
        })
        .count();
    let shown = ranked.len().min(ROW_CAP);
    let visible = &ranked[..shown];
    let agents: Vec<Option<Vec<crate::tmux::PickerAgent>>> = visible
        .iter()
        .map(|session| crate::tmux::parse_picker_agents(&session.agents, bounds.now_epoch))
        .collect();
    let running_overflow = usize::from(ranked.len() > shown);
    // Stopped rows carry their own cap and their own overflow note, so a huge
    // stopped fleet cannot crowd out the running rows above it.
    let shown_stopped = stopped.len().min(ROW_CAP);
    let stopped_visible = &stopped[..shown_stopped];
    let stopped_overflow = usize::from(stopped.len() > shown_stopped);
    let item_capacity = bounds.height.saturating_sub(2);
    // The widest this draw could ever become — every roster expanded — which
    // bounds the pre-allocation when the client height is unbounded.
    let widest_rows = agents
        .iter()
        .map(|agents| agent_row_count(agents.as_deref()))
        .fold(
            shown
                .saturating_add(running_overflow)
                .saturating_add(shown_stopped)
                .saturating_add(stopped_overflow),
            usize::saturating_add,
        );
    let current = opened_session.and_then(|id| visible.iter().position(|row| row.id == id));
    let current_rows = current.map_or(usize::MAX, |index| {
        shown
            .saturating_add(running_overflow)
            .saturating_add(shown_stopped)
            .saturating_add(stopped_overflow)
            .saturating_add(agent_row_count(agents[index].as_deref()))
    });
    // Every row this draw knows about, rosters excluded — the collapsed floor.
    let flat_rows = shown
        .saturating_add(running_overflow)
        .saturating_add(shown_stopped)
        .saturating_add(stopped_overflow);
    let expansion = if current_rows <= item_capacity {
        Expansion::Current
    } else if flat_rows <= item_capacity {
        Expansion::Collapsed
    } else {
        Expansion::Capped
    };
    let inner_width = bounds.width.saturating_sub(4);
    // Running rows keep today's share exactly; stopped rows fill what is left,
    // one row staying reserved for the omission note.
    let (displayed, displayed_stopped) = if expansion == Expansion::Capped {
        let running = shown.min(item_capacity.saturating_sub(1));
        let remaining = item_capacity.saturating_sub(running).saturating_sub(1);
        (running, shown_stopped.min(remaining))
    } else {
        (shown, shown_stopped)
    };
    // One pass over exactly the rows this draw will show, so every column is
    // only as wide as the content under it.
    let mut columns = Columns::FLOOR;
    for (index, session) in visible.iter().take(displayed).enumerate() {
        columns.hold_session(
            &clean(&session.name),
            Mark::from_rank_value(session.rank).word(),
            &clean(&session.branch),
        );
        if expanded_row(expansion, current, index) {
            for agent in agents[index].iter().flatten() {
                columns.hold_agent(&agent.name, &agent.profile, &agent.state);
            }
        }
    }
    for row in stopped_visible.iter().take(displayed_stopped) {
        columns.hold_session(&clean(&row.name), STOPPED_WORD, &clean(&row.branch));
    }
    let mut session_items = Vec::with_capacity(displayed);
    for (index, session) in visible.iter().take(displayed).enumerate() {
        let expanded = expanded_row(expansion, current, index);
        let suffix = (!expanded).then(|| agents_suffix(agents[index].as_deref()));
        session_items.push(session_item(
            session,
            &Row {
                columns,
                suffix: suffix.as_deref(),
                max_width: inner_width,
            },
            panes,
            icons,
            client,
            opened_session,
        ));
    }
    let next_key = assign_keys(&mut session_items);
    let mut stopped_items: Vec<MenuItem> = stopped_visible
        .iter()
        .take(displayed_stopped)
        .map(|row| {
            stopped_item(
                row,
                &Row {
                    columns,
                    suffix: None,
                    max_width: inner_width,
                },
                icons,
                resume,
            )
        })
        .collect();
    assign_keys_from(&mut stopped_items, next_key);
    let mut items = Vec::with_capacity(item_capacity.min(widest_rows));
    for (index, session_item) in session_items.into_iter().enumerate() {
        items.push(session_item);
        if expanded_row(expansion, current, index) {
            match &agents[index] {
                Some(found) => items.extend(found.iter().map(|agent| {
                    agent_item(
                        visible[index],
                        agent,
                        columns,
                        panes,
                        icons,
                        client,
                        opened_session,
                        inner_width,
                    )
                })),
                None => items.push(disabled(clip_cells("  agents: unavailable", inner_width))),
            }
        }
    }
    items.extend(stopped_items);
    let omitted = if expansion == Expansion::Capped {
        ranked
            .len()
            .saturating_sub(displayed)
            .saturating_add(stopped.len().saturating_sub(displayed_stopped))
    } else {
        ranked
            .len()
            .saturating_sub(shown)
            .saturating_add(stopped.len().saturating_sub(shown_stopped))
    };
    if omitted > 0 {
        items.push(disabled(clip_cells(
            &format!("+{omitted} sessions omitted"),
            inner_width,
        )));
    }
    if items.is_empty() {
        items.push(disabled(clip_cells("no running ae sessions", inner_width)));
    }
    // The stopped count joins the title only when there is one, so a draw
    // without stopped rows keeps today's bytes exactly.
    let title = if stopped.is_empty() {
        format!(
            " ae session — {} running · {need_you} need you — prefix a ",
            ranked.len(),
        )
    } else {
        format!(
            " ae session — {} running · {} stopped · {need_you} need you — prefix a ",
            ranked.len(),
            stopped.len(),
        )
    };
    Menu {
        title: clip_cells(&title, inner_width),
        title_style: crate::theme::menu_title_style(palette),
        items,
    }
}

/// Whether the row at `index` draws its agents under it.
///
/// Only ever under the CURRENT session, and only while the ladder holds that
/// state: with no current session resolved there is nothing to expand, and a
/// fleet that cannot fit even one roster keeps every session collapsed.
const fn expanded_row(expansion: Expansion, current: Option<usize>, index: usize) -> bool {
    matches!(expansion, Expansion::Current) && matches!(current, Some(held) if held == index)
}

fn agent_row_count(agents: Option<&[crate::tmux::PickerAgent]>) -> usize {
    agents.map_or(1, <[crate::tmux::PickerAgent]>::len)
}

fn agents_suffix(agents: Option<&[crate::tmux::PickerAgent]>) -> String {
    agents.map_or_else(
        || " · agents unavailable".to_owned(),
        |agents| {
            let working = agents
                .iter()
                .filter(|agent| agent.mark() == Mark::Working)
                .count();
            format!(" · {} agents, {working} working", agents.len())
        },
    )
}

/// The live server's admitted sessions, most actionable first.
///
/// ATTENTION still decides: this menu exists to be acted on, and a session that
/// needs the human outranks where they filed it. Below that it defers to the
/// fleet's one shared tail, [`crate::theme::fleet_tail_cmp`], so the picker and
/// the status strip cannot drift on the part they share. The name tie-break is
/// defensive: live tmux session ids are unique, but the pure model does not need
/// to assume that to stay deterministic.
fn ranked_sessions<'a>(
    sessions: &'a [PickerSession],
    order: &crate::theme::FleetOrder,
) -> Vec<&'a PickerSession> {
    let mut ranked: Vec<&PickerSession> = sessions.iter().collect();
    ranked.sort_by(|left, right| {
        right.rank.cmp(&left.rank).then_with(|| {
            crate::theme::fleet_tail_cmp(
                order,
                (&left.name, left.created()),
                (&right.name, right.created()),
            )
        })
    });
    ranked
}

/// What one session row is drawn with, gathered so the call stays one question.
struct Row<'a> {
    /// This draw's shared column widths.
    columns: Columns,
    /// The collapsed-roster summary, present only while the agents stay hidden.
    suffix: Option<&'a str>,
    /// The menu's inner width, which the finished label is clipped to.
    max_width: usize,
}

/// One session row. Its lead hint earns a guarded jump only when the build-time
/// pane snapshot also places it in this exact session.
fn session_item(
    session: &PickerSession,
    row: &Row<'_>,
    panes: &[PickerPane],
    icons: bool,
    client: Option<&str>,
    opened_session: Option<&str>,
) -> MenuItem {
    let glyph = if session.glyph.is_empty() {
        Mark::Idle.glyph(icons)
    } else {
        &session.glyph
    };
    let mark = Mark::from_rank_value(session.rank);
    let head = format!(
        "{} {} {} {}",
        pad(&clean(&session.name), row.columns.name),
        pad(&clean(glyph), 1),
        pad(&clean(mark.word()), row.columns.state),
        pad(&clean(&session.branch), row.columns.branch),
    );
    let label = match row.suffix {
        // The summary brings its own separator, so it joins the branch column
        // where the goal would have had a space before it.
        Some(suffix) => format!("{head}{suffix}"),
        None => format!("{head} {}", truncate(&clean(&session.goal), GOAL_WIDTH)),
    };
    MenuItem {
        label: clip_cells(&label, row.max_width),
        key: String::new(),
        action: picker_action(session, &session.main_pane, panes, client, opened_session),
    }
}

/// One stopped session row. It shares the running rows' column grid, draws a
/// keyless-dim row when no resume pin exists, and never expands anything.
fn stopped_item(
    row: &PickerStopped,
    drawn: &Row<'_>,
    icons: bool,
    resume: Option<&PickerResume<'_>>,
) -> MenuItem {
    let label = format!(
        "{} {} {} {} {}",
        pad(&clean(&row.name), drawn.columns.name),
        pad(Mark::Dead.glyph(icons), 1),
        pad(STOPPED_WORD, drawn.columns.state),
        pad(&clean(&row.branch), drawn.columns.branch),
        truncate(&clean(&row.goal), GOAL_WIDTH),
    );
    let action = resume.map_or(MenuAction::Disabled, |resume| {
        MenuAction::Run(resume_command(row, resume))
    });
    MenuItem {
        label: clip_cells(&label, drawn.max_width),
        key: String::new(),
        action,
    }
}

/// The `run-shell -b` re-exec that resumes `row` for the captured clicker.
///
/// The continuation re-proves the captured client and server before the
/// ordinary launch owner runs, so this command carries identity, never
/// authority: nothing here decides anything on its own.
fn resume_command(row: &PickerStopped, resume: &PickerResume<'_>) -> String {
    let mut argv: Vec<String> = resume.launcher.to_vec();
    argv.extend([
        "orchestrator".to_owned(),
        PICKER_RESUME_FLAG.to_owned(),
        row.name.clone(),
        "--client".to_owned(),
        resume.client.to_owned(),
        "--client-pid".to_owned(),
        resume.client_pid.to_owned(),
        "--server-pid".to_owned(),
        resume.server_pid.to_owned(),
        "--server-start".to_owned(),
        resume.server_start.to_owned(),
        "--deadline".to_owned(),
        resume.deadline.to_string(),
    ]);
    crate::tmux::menu_run_shell_command(&argv)
}

/// One agent row, sharing the session row's two-phase pane-membership guard.
#[allow(
    clippy::too_many_arguments,
    reason = "one nested row carries its session, the shared columns and the jump guard"
)]
fn agent_item(
    session: &PickerSession,
    agent: &crate::tmux::PickerAgent,
    columns: Columns,
    panes: &[PickerPane],
    icons: bool,
    client: Option<&str>,
    opened_session: Option<&str>,
    max_width: usize,
) -> MenuItem {
    let label = format!(
        "  {} {} {} {}",
        agent.mark().glyph(icons),
        pad(&agent.name, columns.name),
        pad(&agent.profile, columns.profile),
        pad(&agent.state, columns.state),
    );
    MenuItem {
        label: clip_cells(&label, max_width),
        key: String::new(),
        action: picker_action(session, &agent.pane, panes, client, opened_session),
    }
}

fn picker_action(
    session: &PickerSession,
    pane_hint: &str,
    panes: &[PickerPane],
    client: Option<&str>,
    opened_session: Option<&str>,
) -> MenuAction {
    let pane_is_member = !pane_hint.is_empty()
        && panes
            .iter()
            .any(|pane| pane.session_id == session.id && pane.pane == pane_hint);
    let action = if pane_is_member {
        MenuAction::Run(client.map_or_else(
            || guarded_jump_id_command(&session.id, pane_hint),
            |client| guarded_jump_client_id_command(client, &session.id, pane_hint),
        ))
    } else {
        MenuAction::Run(client.map_or_else(
            || switch_id_command(&session.id),
            |client| switch_client_id_command(client, &session.id),
        ))
    };
    match (action, opened_session) {
        (MenuAction::Run(command), Some(opened_session))
            if crate::tmux::session_id_is_valid(opened_session) =>
        {
            MenuAction::Run(format!(
                "set-option -u -t {opened_session} {} ; {command}",
                crate::theme::MENU_OPEN_OPTION,
            ))
        }
        (action, _) => action,
    }
}

/// Hand out shortcuts to selectable rows; returns the first unused index, so a
/// second block of rows (stopped sessions) can CONTINUE the alphabet.
fn assign_keys(items: &mut [MenuItem]) -> usize {
    assign_keys_from(items, 0)
}

/// Hand out shortcuts from `next` on, returning the first index after them.
fn assign_keys_from(items: &mut [MenuItem], mut next: usize) -> usize {
    for item in items {
        if matches!(item.action, MenuAction::Disabled) {
            continue;
        }
        item.key = key_at(next);
        next += 1;
    }
    next
}

/// A row that is drawn dim and cannot be chosen.
fn disabled(label: String) -> MenuItem {
    MenuItem {
        label,
        key: String::new(),
        action: MenuAction::Disabled,
    }
}

/// The shortcut for the row at `index`, or none once the alphabet runs out.
fn key_at(index: usize) -> String {
    KEYS.chars()
        .nth(index)
        .map(String::from)
        .unwrap_or_default()
}

/// `text` cut to `width` terminal cells, the last cell an ellipsis when cut.
fn truncate(text: &str, width: usize) -> String {
    clip_cells(text, width)
}

/// `text` cut to `width` cells and padded so the next column stays aligned.
fn pad(text: &str, width: usize) -> String {
    let mut out = truncate(text, width);
    for _ in terminal_cells(&out)..width {
        out.push(' ');
    }
    out
}

/// Clip text by terminal cells, not UTF-8 bytes or scalar count.
fn clip_cells(text: &str, width: usize) -> String {
    if terminal_cells(text) <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let content_width = width.saturating_sub(1);
    let mut used = 0_usize;
    let mut out = String::new();
    for character in text.chars() {
        let cells = terminal_cell_width(character);
        if used.saturating_add(cells) > content_width {
            break;
        }
        out.push(character);
        used = used.saturating_add(cells);
    }
    out.push('…');
    out
}

pub(crate) fn terminal_cells(text: &str) -> usize {
    text.chars().map(terminal_cell_width).sum()
}

/// The terminal widths relevant to picker text, following the usual wcwidth
/// split: combining marks are zero, East Asian wide/fullwidth and emoji are
/// two, and the remaining printable scalars are one.
fn terminal_cell_width(character: char) -> usize {
    let code = u32::from(character);
    if character.is_control()
        || (0x0300..=0x036f).contains(&code)
        || (0x1ab0..=0x1aff).contains(&code)
        || (0x1dc0..=0x1dff).contains(&code)
        || (0x20d0..=0x20ff).contains(&code)
        || (0xfe00..=0xfe0f).contains(&code)
        || (0xfe20..=0xfe2f).contains(&code)
        || (0xe0100..=0xe01ef).contains(&code)
    {
        0
    } else if (0x1100..=0x115f).contains(&code)
        || code == 0x2329
        || code == 0x232a
        || (0x2e80..=0xa4cf).contains(&code)
        || (0xac00..=0xd7a3).contains(&code)
        || (0xf900..=0xfaff).contains(&code)
        || (0xfe10..=0xfe19).contains(&code)
        || (0xfe30..=0xfe6f).contains(&code)
        || (0xff00..=0xff60).contains(&code)
        || (0xffe0..=0xffe6).contains(&code)
        || (0x1f000..=0x1faff).contains(&code)
        || (0x20000..=0x3fffd).contains(&code)
    {
        2
    } else {
        1
    }
}

/// The apostrophe a quote is rewritten to — U+02BC, which reads as one and is
/// not the character tmux's parser ends a quoted word on.
const SAFE_APOSTROPHE: char = '\u{02bc}';

/// `text` as a menu row can carry it.
///
/// A goal is operator text and reaches this from a file anyone can edit.
/// Control characters would break the drawing, so they go; a straight quote
/// would end quoting around menu data, so it is
/// REWRITTEN rather than dropped — "don't ship" should not read "dont ship".
fn clean(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_control())
        .map(|character| {
            if character == '\'' {
                SAFE_APOSTROPHE
            } else {
                character
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        Args, KEYS, ROW_CAP, Usage, launch_tail_is_valid, menu, menu_for_client,
        menu_for_client_session, parse, parse_launch_tail,
    };
    use crate::inventory::ServerId;
    use crate::theme::Palette;
    use crate::tmux::{
        Menu, MenuAction, PickerPane, PickerSession, display_menu_args,
        display_menu_for_client_args,
    };

    /// The clock every watchdog fact in this module's fixtures is stamped at.
    const NOW: i64 = 2_000;

    fn session(name: &str, id: &str, rank: u8, main_pane: &str) -> PickerSession {
        PickerSession {
            name: name.to_owned(),
            id: id.to_owned(),
            rank,
            glyph: String::new(),
            main_pane: main_pane.to_owned(),
            branch: String::new(),
            agents: String::new(),
            goal: String::new(),
        }
    }

    /// PIN: the picker's running rows defer to the SAME shared tail the strip
    /// sorts on — the human's `fleet_order` before tmux creation order. ATTENTION
    /// still wins above it, so the two surfaces share the tail and not the whole
    /// comparator; both halves are pinned here.
    #[test]
    fn the_picker_running_rows_take_the_humans_order_under_attention() {
        let all = [
            session("alpha", "$1", 0, ""),
            session("beta", "$2", 0, ""),
            session("gamma", "$3", 0, ""),
        ];
        let order =
            crate::theme::FleetOrder::from_validated(vec!["gamma".to_owned(), "beta".to_owned()]);
        let names: Vec<&str> = super::ranked_sessions(&all, &order)
            .iter()
            .map(|row| row.name.as_str())
            .collect();
        assert_eq!(
            names,
            ["gamma", "beta", "alpha"],
            "named first in the human's order, unnamed behind by creation"
        );
        // Equal rank is what lets the order speak; an EMPTY order is today's.
        let names: Vec<&str> = super::ranked_sessions(&all, &crate::theme::FleetOrder::EMPTY)
            .iter()
            .map(|row| row.name.as_str())
            .collect();
        assert_eq!(names, ["alpha", "beta", "gamma"], "creation order alone");
    }

    /// PIN: rank DOMINATES the human's order — an unnamed session that needs
    /// the human still sorts above a named one that is idle.
    #[test]
    fn attention_still_outranks_the_humans_order_in_the_picker() {
        let all = [
            session("named-idle", "$1", 0, ""),
            session("unnamed-loud", "$2", 9, ""),
        ];
        let order = crate::theme::FleetOrder::from_validated(vec!["named-idle".to_owned()]);
        let names: Vec<&str> = super::ranked_sessions(&all, &order)
            .iter()
            .map(|row| row.name.as_str())
            .collect();
        assert_eq!(
            names,
            ["unnamed-loud", "named-idle"],
            "the picker stays most-actionable-first"
        );
    }

    /// PIN: the human's order reaches the drawn menu, and the stopped rows —
    /// which carry their own rule and arrive already ordered — are untouched.
    #[test]
    fn the_drawn_picker_orders_running_rows_and_leaves_stopped_rows_alone() {
        let all = [session("alpha", "$1", 0, ""), session("beta", "$2", 0, "")];
        let stopped = [
            super::PickerStopped {
                name: "older".to_owned(),
                goal: String::new(),
                branch: String::new(),
                last_live: Some(10),
            },
            super::PickerStopped {
                name: "newer".to_owned(),
                goal: String::new(),
                branch: String::new(),
                last_live: Some(20),
            },
        ];
        // The human named beta first AND named a stopped row, which must not
        // move: stopped rows are a second source with their own ordering.
        let order =
            crate::theme::FleetOrder::from_validated(vec!["beta".to_owned(), "newer".to_owned()]);
        let drawn = super::menu_for_client_session_in(
            &all,
            &stopped,
            &[],
            true,
            &Palette::DARCULA,
            None,
            None,
            None,
            super::PickerBounds {
                height: 40,
                width: 120,
                now_epoch: 1_700_000_000,
            },
            &order,
        )
        .expect("a menu");
        let text = drawn
            .items
            .iter()
            .map(|item| item.label.clone())
            .collect::<Vec<_>>()
            .join("\n");
        let at = |name: &str| text.find(name).unwrap_or(usize::MAX);
        assert!(at("beta") < at("alpha"), "running rows reordered: {text}");
        assert!(at("alpha") < at("older"), "running above stopped: {text}");
        assert!(
            at("older") < at("newer"),
            "stopped rows keep their caller's order: {text}"
        );
    }

    fn pane(session_id: &str, pane: &str) -> PickerPane {
        PickerPane {
            session_id: session_id.to_owned(),
            pane: pane.to_owned(),
        }
    }

    fn labels(menu: &Menu) -> Vec<String> {
        menu.items.iter().map(|item| item.label.clone()).collect()
    }

    fn names(menu: &Menu) -> Vec<String> {
        labels(menu)
            .iter()
            .filter(|label| !label.starts_with("  ") && !label.starts_with('+'))
            .filter_map(|label| label.split_whitespace().next().map(ToOwned::to_owned))
            .collect()
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one acceptance table for every orchestrator menu flag and refusal"
    )]
    fn the_flags_the_picker_accepts_and_the_ones_it_refuses() {
        assert!(parse(&["--popup".to_owned()]).unwrap().popup);
        assert_eq!(
            parse(&[
                "--client".to_owned(),
                "/dev/ttys007".to_owned(),
                "--popup".to_owned(),
            ]),
            Ok(Args {
                popup: true,
                settings: false,
                quota_dialog: false,
                client: Some("/dev/ttys007".to_owned()),
                client_pid: None,
                server_pid: None,
                server_start: None,
                session_id: None,
            })
        );
        assert_eq!(
            parse(&[]),
            Ok(Args {
                popup: false,
                settings: false,
                quota_dialog: false,
                client: None,
                client_pid: None,
                server_pid: None,
                server_start: None,
                session_id: None,
            })
        );
        for spelling in ["-h", "--help"] {
            assert_eq!(parse(&[spelling.to_owned()]), Err(Usage::Help));
        }
        assert_eq!(Usage::Help.code(), 0);
        assert_eq!(
            parse(&["--nope".to_owned()]),
            Err(Usage::Unknown("--nope".to_owned()))
        );
        assert_eq!(parse(&["--client".to_owned()]), Err(Usage::MissingClient));
        assert!(parse(&["--quota-dialog".to_owned()]).unwrap().quota_dialog);
        assert_eq!(
            parse(&[
                "--quota-dialog".to_owned(),
                "--client".to_owned(),
                "tty".to_owned(),
                "--client-pid".to_owned(),
                "4242".to_owned(),
                "--server-pid".to_owned(),
                "911".to_owned(),
                "--server-start".to_owned(),
                "1789109660".to_owned(),
            ])
            .unwrap()
            .client_pid,
            Some("4242".to_owned())
        );
        assert_eq!(
            parse(&["--client-pid".to_owned()]),
            Err(Usage::MissingQuotaValue("--client-pid"))
        );
        assert_eq!(
            parse(&[
                "--server-pid".to_owned(),
                "1".to_owned(),
                "--server-pid".to_owned(),
                "2".to_owned(),
            ]),
            Err(Usage::DuplicateQuotaValue("--server-pid"))
        );
        for pair in [
            vec!["--popup".to_owned(), "--settings".to_owned()],
            vec!["--popup".to_owned(), "--quota-dialog".to_owned()],
            vec!["--settings".to_owned(), "--quota-dialog".to_owned()],
        ] {
            assert!(parse(&pair).is_err(), "one menu per invocation: {pair:?}");
        }
        // Identity flags belong to the dialog continuation: on the other two
        // menus they are malformed again, exactly as before they existed.
        for (menu, flag) in [
            ("--popup", "--client-pid"),
            ("--popup", "--server-pid"),
            ("--popup", "--server-start"),
            ("--popup", "--session-id"),
            ("--settings", "--client-pid"),
            ("--settings", "--server-pid"),
            ("--settings", "--server-start"),
            ("--settings", "--session-id"),
        ] {
            assert_eq!(
                parse(&[menu.to_owned(), flag.to_owned(), "1".to_owned()]),
                Err(Usage::Unknown(format!("{flag} without --quota-dialog"))),
                "{menu} with {flag}"
            );
        }
        assert_eq!(
            parse(&[
                "--client".to_owned(),
                "one".to_owned(),
                "--client".to_owned(),
                "two".to_owned(),
            ]),
            Err(Usage::DuplicateClient)
        );
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one acceptance table for the picker resume grammar"
    )]
    fn the_picker_resume_grammar_refuses_every_malformed_shape() {
        let good = super::parse_resume(
            &[
                "--picker-resume",
                "hub",
                "--client",
                "/dev/ttys007",
                "--client-pid",
                "42",
                "--server-pid",
                "9",
                "--server-start",
                "1789109660",
                "--deadline",
                "1789109890",
            ]
            .map(ToOwned::to_owned),
        )
        .expect("the documented grammar");
        assert_eq!(good.name, "hub");
        assert_eq!(good.deadline, 1_789_109_890);
        assert!(super::is_resume(&[
            "--picker-resume".to_owned(),
            "hub".to_owned()
        ]));
        assert!(!super::is_resume(&["--popup".to_owned()]));

        for (case, bad) in [
            ("empty", Vec::new()),
            ("no name", vec!["--picker-resume"]),
            ("bad name", vec!["--picker-resume", "bad/name"]),
            (
                "unknown flag",
                vec![
                    "--picker-resume",
                    "hub",
                    "--client",
                    "/dev/ttys007",
                    "--client-pid",
                    "42",
                    "--server-pid",
                    "9",
                    "--server-start",
                    "1789109660",
                    "--deadline",
                    "1789109890",
                    "--nope",
                    "x",
                ],
            ),
            ("missing value", vec!["--picker-resume", "hub", "--client"]),
            (
                "bad client",
                vec![
                    "--picker-resume",
                    "hub",
                    "--client",
                    "not a client",
                    "--client-pid",
                    "42",
                    "--server-pid",
                    "9",
                    "--server-start",
                    "1789109660",
                    "--deadline",
                    "1789109890",
                ],
            ),
            (
                "non-decimal pid",
                vec![
                    "--picker-resume",
                    "hub",
                    "--client",
                    "/dev/ttys007",
                    "--client-pid",
                    "4x",
                    "--server-pid",
                    "9",
                    "--server-start",
                    "1789109660",
                    "--deadline",
                    "1789109890",
                ],
            ),
            (
                "duplicate flag",
                vec![
                    "--picker-resume",
                    "hub",
                    "--client",
                    "/dev/ttys007",
                    "--client",
                    "/dev/ttys008",
                    "--client-pid",
                    "42",
                    "--server-pid",
                    "9",
                    "--server-start",
                    "1789109660",
                    "--deadline",
                    "1789109890",
                ],
            ),
            (
                "missing deadline",
                vec![
                    "--picker-resume",
                    "hub",
                    "--client",
                    "/dev/ttys007",
                    "--client-pid",
                    "42",
                    "--server-pid",
                    "9",
                    "--server-start",
                    "1789109660",
                ],
            ),
            (
                "bad deadline",
                vec![
                    "--picker-resume",
                    "hub",
                    "--client",
                    "/dev/ttys007",
                    "--client-pid",
                    "42",
                    "--server-pid",
                    "9",
                    "--server-start",
                    "1789109660",
                    "--deadline",
                    "soon",
                ],
            ),
        ] {
            assert!(
                super::parse_resume(
                    &bad.iter()
                        .map(|word| (*word).to_owned())
                        .collect::<Vec<String>>()
                )
                .is_err(),
                "{case}"
            );
        }
    }

    #[test]
    fn the_bare_seat_accepts_only_launch_preamble_flags() {
        assert!(launch_tail_is_valid(&[]));
        for flag in ["--attach", "--no-attach", "--inside-tmux", "--no-autostart"] {
            assert!(launch_tail_is_valid(&[flag.to_owned()]), "{flag}");
        }
        let parsed = parse_launch_tail(&[
            "--no-attach".to_owned(),
            "--inside-tmux".to_owned(),
            "--no-autostart".to_owned(),
        ])
        .expect("seat flags");
        assert_eq!(parsed.attach, Some(false));
        assert!(parsed.inside_tmux);
        assert!(parsed.no_autostart);
        for flag in [
            "--popup",
            "--copy",
            "--worktree",
            "--dir",
            "--from",
            "use",
            "--nope",
        ] {
            assert!(!launch_tail_is_valid(&[flag.to_owned()]), "{flag}");
        }
    }

    #[test]
    fn the_bare_word_routes_to_the_canonical_seat() {
        assert_eq!(super::seat_launch_args(), vec!["orchestrator".to_owned()]);
        assert_eq!(super::ORCHESTRATOR_SESSION, "orchestrator");
    }

    #[test]
    fn the_usage_names_the_binding_lead_destination_and_way_back() {
        assert!(super::USAGE.contains("prefix a"));
        assert!(!super::USAGE.contains("Bind it:"));
        assert!(super::USAGE.contains("lead pane"));
        assert!(super::USAGE.contains("switch-client -l"));
    }

    #[test]
    fn attention_then_creation_then_name_orders_rows() {
        let sessions = vec![
            session("quiet-new", "$9", 0, ""),
            session("hot-new", "$8", 5, ""),
            session("hot-old-b", "$2", 5, ""),
            session("hot-old-a", "$2", 5, ""),
        ];
        assert_eq!(
            names(&menu(&sessions, &[], true, &Palette::DARCULA)),
            ["hot-old-a", "hot-old-b", "hot-new", "quiet-new"]
        );
    }

    #[test]
    fn rows_show_bounded_state_branch_and_goal_columns() {
        let mut shown = session("hub", "$1", 4, "");
        shown.glyph = "⚠".to_owned();
        shown.branch = "feat/menu".to_owned();
        shown.goal = "ship it".to_owned();
        assert_eq!(
            labels(&bounded_menu(&[shown], 6))[0],
            format!(
                "{} {} {} {} {}",
                // Each column is as wide as its own content, not as wide as its
                // cap: "hub" takes the four-cell floor, "feat/menu" nine cells.
                super::pad("hub", 4),
                super::pad("⚠", 1),
                super::pad("needs-you", 9),
                super::pad("feat/menu", 9),
                "ship it"
            )
        );
    }

    /// The invariant the fifth state exists under at the picker: `waiting-user`
    /// (12 cells) and `waiting-agent` (13) must NEVER render identically. The
    /// old `STATE_CAP` of 9 clipped both to `waiting-…` — byte-identical in the
    /// one surface the human glances at.
    #[test]
    fn waiting_user_and_waiting_agent_never_render_identically_in_the_picker() {
        let mut hub = session("hub", "$1", 0, "");
        hub.agents = format!("v1;{NOW};60;u:p:waiting-user:%10;a:p:waiting-agent:%11");
        let drawn = labels(&bounded_menu(&[hub], 8));
        let row = |name: &str| {
            drawn
                .iter()
                .find(|label| label.split_whitespace().any(|cell| cell == name))
                .unwrap_or_else(|| panic!("agent row {name:?} renders: {drawn:?}"))
                .clone()
        };
        let user = row("u");
        let agent = row("a");
        let state = |label: &str| {
            label
                .split_whitespace()
                .last()
                .unwrap_or_default()
                .to_owned()
        };
        assert!(user.contains("waiting-user"), "{user}");
        assert!(agent.contains("waiting-agent"), "{agent}");
        assert_eq!(state(&user), "waiting-user");
        assert_eq!(state(&agent), "waiting-agent");
        assert_ne!(state(&user), state(&agent), "the words must differ");
        assert_eq!(
            super::terminal_cells("waiting-agent"),
            13,
            "the widest word the cap must hold, measured"
        );
    }

    #[test]
    fn column_widths_follow_the_drawn_rows_and_stop_at_their_caps() {
        let idle = crate::theme::Mark::Idle.glyph(true);
        let short = [session("ab", "$1", 2, ""), session("cd", "$2", 2, "")];
        assert_eq!(
            labels(&bounded_menu(&short, 6))[0],
            format!(
                "{} {} {} {} {}",
                super::pad("ab", 4),
                super::pad(idle, 1),
                super::pad("working", 7),
                super::pad("", 4),
                ""
            ),
            "two short names give a four-cell name column, and an empty branch its floor"
        );

        let mut long = session("a-very-long-session-name-indeed", "$1", 2, "");
        long.branch = "feature/a-branch-name-that-runs-on".to_owned();
        let capped = labels(&bounded_menu(&[long], 6))[0].clone();
        assert!(
            capped.starts_with("a-very-long-sessi… "),
            "a long name stops at the cap the layout was designed for: {capped:?}"
        );
        assert!(
            capped.contains("feature/a-bra… "),
            "so does a long branch: {capped:?}"
        );

        let mut hub = session("hub", "$1", 2, "%1");
        hub.agents = format!("v1;{NOW};300;w:p:done:%1;a-much-longer-agent:gpt56sol:working:%2");
        let expanded = labels(&bounded_menu(&[hub], 8));
        // The widest agent name is 19 cells, past the 18-cell cap; the widest
        // profile is 8 and the widest state word 7.
        assert_eq!(
            expanded[1],
            format!("  ✓ {:<18} {:<8} {:<7}", "w", "p", "done"),
        );
        assert_eq!(
            expanded[2],
            format!(
                "  ● {:<18} {:<8} {:<7}",
                "a-much-longer-age…", "gpt56sol", "working"
            ),
        );
        assert!(
            expanded[0].starts_with(&format!("{:<18} ", "hub")),
            "a session row and the agent rows under it share one name column: {:?}",
            expanded[0]
        );
    }

    #[test]
    fn expanded_agent_rows_are_indented_static_and_guarded_by_membership() {
        let mut hub = session("hub", "$7", 2, "%10");
        hub.agents =
            "v1;2000;60;lead:fable5:working:%10;builder:gpt56sol:done:%11;gone:gpt56luna:dead:"
                .to_owned();
        let drawn = super::menu_for_client_session_in(
            &[hub],
            &[],
            &[pane("$7", "%10"), pane("$7", "%11")],
            true,
            &Palette::DARCULA,
            Some("client"),
            Some("$7"),
            None,
            super::PickerBounds {
                height: 10,
                width: 100,
                now_epoch: 2_000,
            },
            &crate::theme::FleetOrder::EMPTY,
        )
        .expect("room for one session and three agents");
        assert_eq!(drawn.items.len(), 4);
        assert_eq!(drawn.items[0].key, "1");
        // Seven cells of name, nine of profile, seven of state — the widest of
        // each among these three rows, not the caps.
        assert_eq!(
            drawn.items[1].label,
            format!("  ● {:<7} {:<9} {:<7}", "lead", "fable5", "working")
        );
        assert_eq!(
            drawn.items[2].label,
            format!("  ✓ {:<7} {:<9} {:<7}", "builder", "gpt56sol", "done")
        );
        assert_eq!(
            drawn.items[3].label,
            format!("  ✖ {:<7} {:<9} {:<7}", "gone", "gpt56luna", "dead")
        );
        assert!(drawn.items[1..].iter().all(|item| item.key.is_empty()));
        let MenuAction::Run(jump) = &drawn.items[2].action else {
            panic!("live agent row is selectable");
        };
        assert!(jump.contains("switch-client -c 'client' -t $7"), "{jump}");
        assert!(jump.contains("-t %11"), "{jump}");
        let MenuAction::Run(fallback) = &drawn.items[3].action else {
            panic!("missing agent row still switches session");
        };
        assert_eq!(
            fallback,
            "set-option -u -t $7 @ae_menu_open ; switch-client -c 'client' -t $7"
        );
    }

    /// A fresh `waiting-agent` carries its own seventh mark in the picker's
    /// agent rows — `◔` with icons, `~` without — and the collapsed summary
    /// counts only working-mark seats: waiting is quiet, not working.
    #[test]
    fn waiting_agent_agent_rows_carry_the_seventh_mark_and_leave_the_working_count() {
        let mut hub = session("hub", "$7", 2, "%10");
        hub.agents =
            "v1;2000;60;lead:fable5:working:%10;colead:gpt56sol:waiting-agent:%11".to_owned();
        let panes = [pane("$7", "%10"), pane("$7", "%11")];
        for (icons, glyph, working) in [(true, "◔", "●"), (false, "~", "*")] {
            let drawn = super::menu_for_client_session_in(
                &[hub.clone()],
                &[],
                &panes,
                icons,
                &Palette::DARCULA,
                Some("client"),
                Some("$7"),
                None,
                super::PickerBounds {
                    height: 10,
                    width: 100,
                    now_epoch: 2_000,
                },
                &crate::theme::FleetOrder::EMPTY,
            )
            .expect("room for one session and two agents");
            assert_eq!(drawn.items.len(), 3, "icons={icons}");
            let row = &drawn.items[2].label;
            assert!(
                row.starts_with(&format!("  {glyph} ")) && row.contains("waiting-agent"),
                "icons={icons}: {row}"
            );
            assert!(
                !row.contains(working),
                "icons={icons}: no working glyph on a waiting row: {row}"
            );
        }

        let mut other = session("other", "$2", 0, "");
        other.agents = "v1;2000;60;lead:p:working:%10;colead:p:waiting-agent:%11".to_owned();
        let sessions = [
            with_agents(session("current", "$1", 0, ""), 2_000, 1),
            other,
        ];
        let roomy = bounded_menu(&sessions, 10);
        assert!(
            roomy.items[2].label.contains("2 agents, 1 working"),
            "waiting-agent is not counted as working: {}",
            roomy.items[2].label
        );
    }

    #[test]
    fn missing_invalid_and_stale_agent_facts_are_unavailable_not_partial() {
        let mut facts = [
            session("missing", "$1", 0, ""),
            session("invalid", "$2", 0, ""),
        ];
        facts[1].agents = "v1;2000;60;ok:fable5:done:%2;bad:fable5:unknown:%3".to_owned();
        let mut stale = session("stale", "$3", 0, "");
        stale.agents = "v1;1879;60;lead:fable5:working:%3".to_owned();
        let mut all = facts.to_vec();
        all.push(stale);
        // ONLY the current session's roster expands, so each fact is judged by
        // drawing its own session as the current one.
        for current in ["$1", "$2", "$3"] {
            let drawn = super::menu_for_client_session_in(
                &all,
                &[],
                &[],
                true,
                &Palette::DARCULA,
                None,
                Some(current),
                None,
                super::PickerBounds {
                    height: 12,
                    width: 100,
                    now_epoch: 2_000,
                },
                &crate::theme::FleetOrder::EMPTY,
            )
            .expect("room for every session row");
            assert_eq!(
                drawn
                    .items
                    .iter()
                    .filter(|item| item.label == "  agents: unavailable")
                    .count(),
                1,
                "{current} draws its own unavailable roster and no other"
            );
        }
        all[0].agents = "v1;1880;60;lead:fable5:working:%1".to_owned();
        assert!(
            super::menu_for_client_session_in(
                &all[..1],
                &[],
                &[],
                true,
                &Palette::DARCULA,
                None,
                Some("$1"),
                None,
                super::PickerBounds {
                    height: 6,
                    width: 100,
                    now_epoch: 2_000,
                },
                &crate::theme::FleetOrder::EMPTY,
            )
            .expect("boundary is fresh")
            .items
            .iter()
            .any(|item| item.label.contains("lead"))
        );
    }

    fn with_agents(mut session: PickerSession, epoch: i64, count: usize) -> PickerSession {
        session.agents = format!(
            "v1;{epoch};60;{}",
            (0..count)
                .map(|index| format!("a{index}:p:working:%{}", index + 10))
                .collect::<Vec<_>>()
                .join(";")
        );
        session
    }

    fn bounded_menu(sessions: &[PickerSession], height: usize) -> Menu {
        super::menu_for_client_session_in(
            sessions,
            &[],
            &[],
            true,
            &Palette::DARCULA,
            None,
            Some("$1"),
            None,
            super::PickerBounds {
                height,
                width: 100,
                now_epoch: 2_000,
            },
            &crate::theme::FleetOrder::EMPTY,
        )
        .expect("test dimensions")
    }

    /// RULING 4's byte pin: a fixture fleet parsed from the BATCHED read's own
    /// output builds exactly the `display-menu` argv this golden fixes. The read
    /// side may change its connections; the bytes drawn may not.
    #[test]
    fn the_batched_read_draws_the_fixture_fleet_byte_identically() {
        let listing = concat!(
            "ae-picker:c|/dev/ttys002 | $7 | 4243 | 40 | 140\n",
            "ae-picker!c\n",
            "ae-picker:s|hub | $7 | 2 |  | %10 | main | v1;2000;60;lead:fable5:working:%10 | ship it\n",
            "ae-picker:s|rest | $2 | 0 |  | %20 | main |  | \n",
            "ae-picker!s\n",
            "ae-picker:p|$7 | %10\n",
            "ae-picker!p\n",
            "ae-picker:n|hub\n",
            "ae-picker:n|rest\n",
            "ae-picker!n\n",
            "ae-picker:i|4242 | 1700000000\n",
            "ae-picker!i\n",
            "ae-picker:k|/tmp/ae.sock\n",
            "ae-picker!k\n",
            "ae-picker:l|on | a | on | on\n",
            "ae-picker!l\n",
        );
        let read = crate::tmux::interpret_picker_read(listing, "/dev/ttys002").expect("one read");
        let look = read
            .look
            .as_ref()
            .map_or(crate::theme::Look::DEFAULT, |look| {
                crate::theme::Look::read(&look.icons, &look.palette, &look.drawn, &look.motion)
            });
        let menu = super::menu_for_client_session_in(
            &read.sessions.expect("sessions"),
            &[super::PickerStopped {
                name: "old".to_owned(),
                goal: "gone".to_owned(),
                branch: "main".to_owned(),
                last_live: Some(1_999),
            }],
            &read.panes.expect("panes"),
            look.icons,
            &look.palette,
            Some("/dev/ttys002"),
            Some("$7"),
            None,
            super::PickerBounds {
                height: 24,
                width: 100,
                now_epoch: NOW,
            },
            &crate::theme::FleetOrder::EMPTY,
        )
        .expect("room for the fixture fleet");
        let argv =
            display_menu_for_client_args(&ServerId::Ambient, Some("/dev/ttys002"), &menu, false);
        assert_eq!(
            argv,
            [
                "display-menu",
                "-O",
                "-c",
                "/dev/ttys002",
                "-x",
                "0",
                "-y",
                "S",
                "-T",
                "#[fg=#e5a03c bold] ae session — 2 running · 1 stopped · 0 need you — prefix a ",
                "--",
                "hub  · working main ship it",
                "1",
                "set-option -u -t $7 @ae_menu_open ; switch-client -c '/dev/ttys002' -t $7 ; if-shell -F -t %10 '##{==:##{session_id},$7}' 'select-window -t %10 ; select-pane -t %10'",
                "  ● lead fable5 working",
                "",
                "set-option -u -t $7 @ae_menu_open ; switch-client -c '/dev/ttys002' -t $7 ; if-shell -F -t %10 '##{==:##{session_id},$7}' 'select-window -t %10 ; select-pane -t %10'",
                "rest · idle    main · agents unavailable",
                "2",
                "set-option -u -t $7 @ae_menu_open ; switch-client -c '/dev/ttys002' -t $2",
                "-old  ✖ stopped main gone",
                "",
                "",
            ],
            "the fixture fleet's drawn bytes, frozen"
        );
    }

    #[test]
    fn height_shows_the_current_roster_then_collapses_then_caps_sessions() {
        let sessions = [
            with_agents(session("current", "$1", 0, ""), 2_000, 3),
            with_agents(session("other", "$2", 0, ""), 2_000, 3),
        ];
        // There is no expand-everything state: room for both rosters still
        // draws only the current session's, and the other keeps its summary.
        let roomy = bounded_menu(&sessions, 10);
        assert_eq!(
            roomy.items.len(),
            5,
            "two sessions and the current roster, never the other's"
        );
        assert!(roomy.items[4].label.contains("3 agents, 3 working"));
        let current = bounded_menu(&sessions, 7);
        assert_eq!(
            current.items.len(),
            5,
            "two sessions and current's three agents"
        );
        assert!(current.items[4].label.contains("3 agents, 3 working"));
        let collapsed = bounded_menu(&sessions, 6);
        assert_eq!(collapsed.items.len(), 2);
        assert!(
            collapsed
                .items
                .iter()
                .all(|item| item.label.contains("3 agents, 3 working"))
        );
        for drawn in [&roomy, &current, &collapsed] {
            assert_eq!(
                drawn
                    .items
                    .iter()
                    .filter(|item| !item.key.is_empty())
                    .map(|item| item.key.as_str())
                    .collect::<Vec<_>>(),
                ["1", "2"],
                "agent expansion never renumbers session shortcuts"
            );
        }

        let packed = [
            with_agents(session("current", "$1", 0, ""), 2_000, 20),
            with_agents(session("other", "$2", 0, ""), 2_000, 20),
        ];
        let current_exact = bounded_menu(&packed, 24);
        assert_eq!(
            current_exact.items.len(),
            22,
            "20 current agents and two sessions exactly fit 24 lines"
        );
        assert!(
            current_exact
                .items
                .last()
                .is_some_and(|item| item.label.contains("20 agents, 20 working")),
            "the other session is the only collapsed roster"
        );

        let many: Vec<PickerSession> = (0..30)
            .map(|index| {
                with_agents(
                    session(&format!("s{index:02}"), &format!("${}", index + 1), 0, ""),
                    2_000,
                    1,
                )
            })
            .collect();
        let capped = bounded_menu(&many, 24);
        assert_eq!(capped.items.len(), 22, "items + two borders equals 24");
        assert_eq!(
            capped.items.last().expect("omission row").label,
            "+9 sessions omitted"
        );
        assert_eq!(capped.items[0].key, "1");
        assert_eq!(capped.items[20].key, "l");
    }

    #[test]
    fn clipping_counts_terminal_cells_and_tiny_clients_refuse() {
        assert_eq!(super::clip_cells("ab界cd", 5), "ab界…");
        let mut wide = session("wide", "$1", 0, "");
        wide.goal = "界".repeat(40);
        let drawn = super::menu_for_client_session_in(
            &[wide],
            &[],
            &[],
            true,
            &Palette::DARCULA,
            None,
            Some("$1"),
            None,
            super::PickerBounds {
                height: 6,
                width: 32,
                now_epoch: 2_000,
            },
            &crate::theme::FleetOrder::EMPTY,
        )
        .expect("32 columns");
        assert!(super::terminal_cells(&drawn.title) <= 28);
        assert!(
            drawn
                .items
                .iter()
                .all(|item| super::terminal_cells(&item.label) <= 28)
        );
        for (height, width) in [(5, 80), (24, 7)] {
            assert_eq!(
                super::menu_for_client_session_in(
                    &[],
                    &[],
                    &[],
                    true,
                    &Palette::DARCULA,
                    None,
                    None,
                    None,
                    super::PickerBounds {
                        height,
                        width,
                        now_epoch: 2_000,
                    },
                    &crate::theme::FleetOrder::EMPTY,
                )
                .err(),
                Some(super::PickerRefusal::TinyClient)
            );
        }
    }

    #[test]
    fn the_title_counts_only_sessions_that_need_a_human() {
        let sessions = [
            session("dead", "$1", 5, ""),
            session("waiting", "$2", 4, ""),
            session("working", "$3", 2, ""),
        ];
        assert_eq!(
            menu(&sessions, &[], true, &Palette::DARCULA).title,
            " ae session — 3 running · 2 need you — prefix a "
        );
    }

    #[test]
    fn row_jumps_to_a_membership_proven_main_pane_without_opening_a_submenu() {
        let sessions = [session("hub", "$7", 3, "%12")];
        let panes = [pane("$7", "%12")];
        let drawn = menu_for_client_session(
            &sessions,
            &panes,
            true,
            &Palette::DARCULA,
            Some("/dev/ttys007"),
            Some("$3"),
        );
        let MenuAction::Run(command) = &drawn.items[0].action else {
            panic!("the session row runs one command");
        };
        assert_eq!(
            command,
            "set-option -u -t $3 @ae_menu_open ; switch-client -c '/dev/ttys007' -t $7 ; if-shell -F -t %12 '##{==:##{session_id},$7}' 'select-window -t %12 ; select-pane -t %12'"
        );
        assert!(
            !command.contains("display-menu"),
            "the row has no agent submenu: {command}"
        );
    }

    #[test]
    fn an_unknown_or_foreign_main_pane_falls_back_to_the_rename_safe_switch() {
        let sessions = [session("hub", "$7", 3, "%12")];
        for panes in [vec![], vec![pane("$8", "%12")], vec![pane("$7", "%13")]] {
            let drawn = menu(&sessions, &panes, true, &Palette::DARCULA);
            let MenuAction::Run(command) = &drawn.items[0].action else {
                panic!("the row remains selectable");
            };
            assert_eq!(command, "switch-client -t $7");
        }
    }

    #[test]
    fn the_row_cap_holds_and_names_what_it_left_out() {
        let sessions: Vec<PickerSession> = (0..ROW_CAP + 7)
            .map(|index| session(&format!("s{index:03}"), &format!("${index}"), 0, ""))
            .collect();
        let drawn = menu(&sessions, &[], true, &Palette::DARCULA);
        assert_eq!(
            drawn.items.len(),
            ROW_CAP + 1,
            "no current session resolved, so no roster expands"
        );
        let last = drawn.items.last().expect("overflow note");
        assert_eq!(last.label, "+7 sessions omitted");
        assert!(matches!(last.action, MenuAction::Disabled));
    }

    #[test]
    fn every_shortcut_is_distinct_and_q_stays_the_close_key() {
        assert!(KEYS.chars().count() >= ROW_CAP);
        assert!(!KEYS.contains('q'));
        let mut seen: Vec<char> = KEYS.chars().collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), KEYS.chars().count());
    }

    #[test]
    fn an_empty_live_fleet_says_so_without_a_selectable_row() {
        let drawn = menu(&[], &[], true, &Palette::DARCULA);
        assert_eq!(labels(&drawn), ["no running ae sessions"]);
        assert!(matches!(drawn.items[0].action, MenuAction::Disabled));
    }

    #[test]
    fn goal_bytes_are_cleaned_then_escaped_at_the_menu_boundary() {
        let mut hostile = session("hub", "$1", 0, "");
        hostile.glyph = "!\u{7}".to_owned();
        hostile.branch = "bad\nbranch#[fg=blue]".to_owned();
        hostile.goal = "100% | #[bg=red] don't\nstop".to_owned();
        // The goal only draws under the CURRENT session's expanded row.
        let drawn =
            menu_for_client_session(&[hostile], &[], true, &Palette::DARCULA, None, Some("$1"));
        let label = &drawn.items[0].label;
        assert!(!label.chars().any(char::is_control), "{label:?}");
        assert!(!label.contains('\''), "{label:?}");
        assert!(label.contains("badbranch#[fg…"), "{label:?}");
        let words = display_menu_args(&ServerId::Ambient, &drawn, true);
        let rendered = words.iter().find(|word| word.contains("hub")).expect("row");
        assert!(
            rendered.contains("100% | ##[bg=red] donʼtstop"),
            "{rendered}"
        );
        assert!(rendered.contains("badbranch##[fg…"), "{rendered}");
    }

    #[test]
    fn display_argv_anchors_the_explicit_client_menu_at_the_left() {
        let drawn = menu_for_client(
            &[session("hub", "$1", 0, "")],
            &[],
            true,
            &Palette::DARCULA,
            Some("client"),
        );
        let words = display_menu_for_client_args(&ServerId::Ambient, Some("client"), &drawn, true);
        assert_eq!(
            &words[..9],
            [
                "display-menu",
                "-M",
                "-O",
                "-c",
                "client",
                "-x",
                "0",
                "-y",
                "S",
            ]
        );
        let keyboard =
            display_menu_for_client_args(&ServerId::Ambient, Some("client"), &drawn, false);
        assert_eq!(
            &keyboard[..8],
            ["display-menu", "-O", "-c", "client", "-x", "0", "-y", "S",]
        );
        assert!(!words.iter().any(|word| word == "-t"));
        assert!(!keyboard.iter().any(|word| word == "-M"));
        assert_eq!(words.iter().filter(|word| word.as_str() == "--").count(), 1);
        assert!(
            words
                .iter()
                .any(|word| word == "switch-client -c 'client' -t $1")
        );
    }

    fn stopped_row(name: &str, last_live: Option<i64>) -> super::PickerStopped {
        super::PickerStopped {
            name: name.to_owned(),
            goal: format!("goal of {name}"),
            branch: "feat/x".to_owned(),
            last_live,
        }
    }

    fn resume_ctx(launcher: &[String]) -> super::PickerResume<'_> {
        super::PickerResume {
            client: "/dev/ttys007",
            client_pid: "4242",
            server_pid: "911",
            server_start: "1789109660",
            deadline: 1_789_109_890,
            launcher,
        }
    }

    fn stopped_menu(
        sessions: &[PickerSession],
        stopped: &[super::PickerStopped],
        opened: Option<&str>,
        resume: Option<&super::PickerResume<'_>>,
        height: usize,
    ) -> Menu {
        super::menu_for_client_session_in(
            sessions,
            stopped,
            &[],
            true,
            &Palette::DARCULA,
            Some("/dev/ttys007"),
            opened,
            resume,
            super::PickerBounds {
                height,
                width: 100,
                now_epoch: 2_000,
            },
            &crate::theme::FleetOrder::EMPTY,
        )
        .expect("test dimensions")
    }

    #[test]
    fn stopped_rows_draw_after_running_rows_and_continue_the_shortcut_alphabet() {
        let launcher = vec!["/opt/ae".to_owned()];
        let resume = resume_ctx(&launcher);
        let sessions = [session("hub", "$1", 0, ""), session("two", "$2", 0, "")];
        let stopped = [stopped_row("old-a", Some(100)), stopped_row("old-b", None)];
        let drawn = stopped_menu(&sessions, &stopped, None, Some(&resume), 12);
        let rows: Vec<String> = drawn
            .items
            .iter()
            .map(|item| {
                item.label
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect();
        assert_eq!(rows, ["hub", "two", "old-a", "old-b"]);
        assert_eq!(
            drawn
                .items
                .iter()
                .map(|item| item.key.as_str())
                .collect::<Vec<_>>(),
            ["1", "2", "3", "4"],
            "stopped rows continue the alphabet after the running rows"
        );
        // Shares the session-row grid: name, glyph, state word, branch, goal.
        let old = &drawn.items[2].label;
        assert!(old.contains("stopped"), "{old:?}");
        assert!(
            old.contains(crate::theme::Mark::Dead.glyph(true)),
            "{old:?}"
        );
        assert!(
            old.starts_with(&format!("{} ", super::pad("old-a", 5))),
            "{old:?}"
        );
        assert!(old.contains("goal of old-a"), "{old:?}");
    }

    #[test]
    fn a_stopped_row_runs_the_resume_continuation_for_its_captured_client() {
        let launcher = vec!["/opt/ae".to_owned()];
        let resume = resume_ctx(&launcher);
        let drawn = stopped_menu(&[], &[stopped_row("old", Some(5))], None, Some(&resume), 8);
        let MenuAction::Run(command) = &drawn.items[0].action else {
            panic!("a stopped row with a resume pin is selectable");
        };
        let expected = crate::tmux::menu_run_shell_command(
            &[
                "/opt/ae",
                "orchestrator",
                "--picker-resume",
                "old",
                "--client",
                "/dev/ttys007",
                "--client-pid",
                "4242",
                "--server-pid",
                "911",
                "--server-start",
                "1789109660",
                "--deadline",
                "1789109890",
            ]
            .map(ToOwned::to_owned),
        );
        assert_eq!(command, &expected);
        // And with no pin at all the row is information, never a live action.
        let unpinned = stopped_menu(&[], &[stopped_row("old", Some(5))], None, None, 8);
        assert!(matches!(unpinned.items[0].action, MenuAction::Disabled));
        assert!(unpinned.items[0].key.is_empty());
    }

    #[test]
    fn stopped_rows_join_the_height_budget_before_the_omission_row() {
        let launcher = vec!["/opt/ae".to_owned()];
        let resume = resume_ctx(&launcher);
        let sessions = [
            session("hub", "$1", 0, ""),
            session("two", "$2", 0, ""),
            session("tri", "$3", 0, ""),
        ];
        let stopped = [
            stopped_row("old-a", Some(3)),
            stopped_row("old-b", Some(2)),
            stopped_row("old-c", Some(1)),
        ];
        // Six rows and a six-line client: the running rows keep today's share,
        // every stopped row waits behind the omission note.
        let tight = stopped_menu(&sessions, &stopped, None, Some(&resume), 6);
        assert_eq!(
            tight.items.last().map(|item| item.label.as_str()),
            Some("+3 sessions omitted")
        );
        // One more line lets exactly one stopped row through.
        let looser = stopped_menu(&sessions, &stopped, None, Some(&resume), 7);
        assert_eq!(looser.items.len(), 5);
        assert!(
            looser.items[3].label.contains("old-a"),
            "{}",
            looser.items[3].label
        );
        assert_eq!(
            looser.items.last().map(|item| item.label.as_str()),
            Some("+2 sessions omitted")
        );
    }

    #[test]
    fn a_stopped_row_never_expands_a_roster_and_never_renumbers_a_running_key() {
        let launcher = vec!["/opt/ae".to_owned()];
        let resume = resume_ctx(&launcher);
        let mut hub = session("hub", "$1", 0, "");
        hub.agents = "v1;2000;60;lead:fable5:working:%10".to_owned();
        let drawn = stopped_menu(
            &[hub],
            &[stopped_row("old", Some(5))],
            Some("$1"),
            Some(&resume),
            10,
        );
        assert_eq!(drawn.items.len(), 3, "session, its agent, the stopped row");
        assert!(drawn.items[1].label.contains("lead"));
        assert!(drawn.items[2].label.contains("stopped"));
        assert_eq!(drawn.items[0].key, "1");
        assert_eq!(
            drawn.items[2].key, "2",
            "the running key was not renumbered"
        );
        assert!(drawn.items[1].key.is_empty(), "agent rows stay keyless");
    }

    #[test]
    fn without_stopped_rows_the_draw_is_byte_identical_to_todays() {
        let sessions = [session("hub", "$1", 0, ""), session("two", "$2", 0, "")];
        let legacy = menu_for_client_session(&sessions, &[], true, &Palette::DARCULA, None, None);
        let with_slice = stopped_menu(&sessions, &[], None, None, usize::MAX >> 1);
        assert_eq!(legacy.title, with_slice.title);
        assert_eq!(labels(&legacy), labels(&with_slice));
        assert_eq!(
            legacy.title,
            " ae session — 2 running · 0 need you — prefix a "
        );
        let stopped_present = stopped_menu(
            &sessions,
            &[stopped_row("old", Some(5))],
            None,
            None,
            usize::MAX >> 1,
        );
        assert_eq!(
            stopped_present.title,
            " ae session — 2 running · 1 stopped · 0 need you — prefix a "
        );
    }
}
