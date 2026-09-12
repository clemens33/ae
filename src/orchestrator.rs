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
creation order and name. Each row carries its live state, branch, spend and
goal; the title carries the fleet's spend. Column widths follow the rows drawn.
Choosing one switches this client to the captured session id and selects its
lead pane when that pane still belongs there.

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

/// At most this many session rows, so the menu fits a terminal and the key
/// alphabet below is never exhausted.
pub const ROW_CAP: usize = 30;

/// The shortcut keys, in the order rows take them. No `q`: tmux closes a menu
/// on `q`, and a row that stole it would trap the human in the picker.
const KEYS: &str = "123456789abcdefghijklmnoprstuvwxyz";

/// The widest a name column is ever drawn, session row or agent row.
const NAME_CAP: usize = 18;

/// The widest a state-word column is ever drawn, session row or agent row.
const STATE_CAP: usize = 9;

/// The widest the branch column is ever drawn.
const BRANCH_CAP: usize = 14;

/// The widest the agent profile column is ever drawn.
const PROFILE_CAP: usize = 12;

/// How much of a goal survives into a row.
const GOAL_WIDTH: usize = 36;

/// The fixed width of the right-aligned spend column.
///
/// Every value [`money`] produces fits it, the `~` of an uncertain reading
/// included, so the column never clips a number.
const SPEND_WIDTH: usize = 8;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expansion {
    All,
    Current,
    Collapsed,
    Capped,
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
        panes,
        icons,
        palette,
        client,
        opened_session,
        PickerBounds {
            height: usize::MAX,
            width: usize::MAX,
            now_epoch: crate::time::Timestamp::now().epoch(),
        },
    )
}

/// The picker model constrained by the exact calling-client snapshot.
///
/// # Errors
///
/// [`PickerRefusal::TinyClient`] before any tmux draw when the border itself
/// would leave no useful menu surface.
pub fn menu_for_client_session_in(
    sessions: &[PickerSession],
    panes: &[PickerPane],
    icons: bool,
    palette: &Palette,
    client: Option<&str>,
    opened_session: Option<&str>,
    bounds: PickerBounds,
) -> Result<Menu, PickerRefusal> {
    if bounds.height < 6 || bounds.width < 8 {
        return Err(PickerRefusal::TinyClient);
    }
    Ok(build_menu(
        sessions,
        panes,
        icons,
        palette,
        client,
        opened_session,
        bounds,
    ))
}

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "one ordered fit-and-render pass over the picker snapshot"
)]
fn build_menu(
    sessions: &[PickerSession],
    panes: &[PickerPane],
    icons: bool,
    palette: &Palette,
    client: Option<&str>,
    opened_session: Option<&str>,
    bounds: PickerBounds,
) -> Menu {
    let ranked = ranked_sessions(sessions);
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
    // The WHOLE fleet, not only the rows that fit: the title's sum counts the
    // same sessions its running count does.
    let spend: Vec<Option<crate::tmux::PickerSpend>> = ranked
        .iter()
        .map(|session| crate::tmux::parse_picker_spend(&session.spend, bounds.now_epoch))
        .collect();
    let fleet = fleet_spend(&spend);
    let overflow = usize::from(ranked.len() > shown);
    let item_capacity = bounds.height.saturating_sub(2);
    let expanded_rows = agents
        .iter()
        .map(|agents| agent_row_count(agents.as_deref()))
        .fold(shown.saturating_add(overflow), usize::saturating_add);
    let current = opened_session.and_then(|id| visible.iter().position(|row| row.id == id));
    let current_rows = current.map_or(usize::MAX, |index| {
        shown
            .saturating_add(overflow)
            .saturating_add(agent_row_count(agents[index].as_deref()))
    });
    let expansion = if expanded_rows <= item_capacity {
        Expansion::All
    } else if current_rows <= item_capacity {
        Expansion::Current
    } else if shown.saturating_add(overflow) <= item_capacity {
        Expansion::Collapsed
    } else {
        Expansion::Capped
    };
    let inner_width = bounds.width.saturating_sub(4);
    let displayed = if expansion == Expansion::Capped {
        shown.min(item_capacity.saturating_sub(1))
    } else {
        shown
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
    let mut session_items = Vec::with_capacity(displayed);
    for (index, session) in visible.iter().take(displayed).enumerate() {
        let expanded = expanded_row(expansion, current, index);
        let suffix = (!expanded).then(|| agents_suffix(agents[index].as_deref()));
        session_items.push(session_item(
            session,
            &Row {
                columns,
                spend: spend[index],
                suffix: suffix.as_deref(),
                max_width: inner_width,
            },
            panes,
            icons,
            client,
            opened_session,
        ));
    }
    assign_keys(&mut session_items);
    let mut items = Vec::with_capacity(item_capacity.min(expanded_rows));
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
    let omitted = if expansion == Expansion::Capped {
        ranked.len().saturating_sub(displayed)
    } else {
        ranked.len().saturating_sub(shown)
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
    // A fleet nobody published a spend fact for keeps the title it always had:
    // an absent number stays absent rather than becoming a confident zero.
    let spend_segment = fleet.map_or_else(String::new, |fleet| {
        format!(" · {}", money(fleet.usd_micro, fleet.uncertain))
    });
    let title = format!(
        " ae session — {} running · {need_you} need you{spend_segment} — prefix a ",
        ranked.len(),
    );
    Menu {
        title: clip_cells(&title, inner_width),
        title_style: crate::theme::menu_title_style(palette),
        items,
    }
}

/// Whether the row at `index` draws its agents under it.
const fn expanded_row(expansion: Expansion, current: Option<usize>, index: usize) -> bool {
    matches!(expansion, Expansion::All)
        || (matches!(expansion, Expansion::Current)
            && matches!(current, Some(held) if held == index))
}

/// The fleet's spend, summed over the sessions whose fact is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FleetSpend {
    usd_micro: u64,
    uncertain: bool,
}

/// Sum every available fact, or `None` when no session published one.
///
/// The sum is uncertain when ANY counted reading is, and equally when any session
/// in the fleet has no reading at all: a total that silently left a session out
/// would be the most confident number on the screen and the least true.
fn fleet_spend(spend: &[Option<crate::tmux::PickerSpend>]) -> Option<FleetSpend> {
    let mut total: Option<u64> = None;
    // An absent fact is missing coverage, not a zero, so it qualifies the sum
    // rather than joining it.
    let mut uncertain = spend.iter().any(Option::is_none);
    for found in spend.iter().flatten() {
        total = Some(total.unwrap_or(0).saturating_add(found.usd_micro));
        uncertain |= found.confidence.uncertain();
    }
    total.map(|usd_micro| FleetSpend {
        usd_micro,
        uncertain,
    })
}

/// `usd_micro` as the spend column draws it, `~` when the reading is uncertain.
///
/// Cents below a thousand dollars, then tenths of a thousand, then whole
/// thousands, so the widest result still fits [`SPEND_WIDTH`] with its prefix.
/// ROUNDING decides the branch, not the raw value: `$999.999` draws as `$1.0k`
/// rather than as a nine-cell `~$1000.00`.
fn money(usd_micro: u64, uncertain: bool) -> String {
    let prefix = if uncertain { "~" } else { "" };
    let cents = usd_micro.saturating_add(5_000) / 10_000;
    if cents < 100_000 {
        return format!("{prefix}${}.{:02}", cents / 100, cents % 100);
    }
    let tenths = usd_micro.saturating_add(50_000_000) / 100_000_000;
    if tenths < 10_000 {
        format!("{prefix}${}.{}k", tenths / 10, tenths % 10)
    } else {
        format!(
            "{prefix}${}k",
            usd_micro.saturating_add(500_000_000) / 1_000_000_000
        )
    }
}

/// The spend cell for one session row. An unavailable fact draws a dash —
/// never a zero, which would read as a session that has spent nothing.
fn spend_cell(spend: Option<crate::tmux::PickerSpend>) -> String {
    rpad(
        &spend.map_or_else(
            || "-".to_owned(),
            |spend| money(spend.usd_micro, spend.confidence.uncertain()),
        ),
        SPEND_WIDTH,
    )
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
/// Attention decides, then tmux creation order, then name. The id tie-break is
/// defensive: live tmux session ids are unique, but the pure model does not
/// need to assume that in order to stay deterministic.
fn ranked_sessions(sessions: &[PickerSession]) -> Vec<&PickerSession> {
    let mut ranked: Vec<&PickerSession> = sessions.iter().collect();
    ranked.sort_by(|left, right| {
        right
            .rank
            .cmp(&left.rank)
            .then_with(|| left.created().cmp(&right.created()))
            .then_with(|| left.name.cmp(&right.name))
    });
    ranked
}

/// What one session row is drawn with, gathered so the call stays one question.
struct Row<'a> {
    /// This draw's shared column widths.
    columns: Columns,
    /// The session's parsed spend fact, absent when unavailable.
    spend: Option<crate::tmux::PickerSpend>,
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
        "{} {} {} {} {}",
        pad(&clean(&session.name), row.columns.name),
        pad(&clean(glyph), 1),
        pad(&clean(mark.word()), row.columns.state),
        pad(&clean(&session.branch), row.columns.branch),
        spend_cell(row.spend),
    );
    let label = match row.suffix {
        // The summary brings its own separator, so it joins the spend column
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

/// Hand out shortcuts to selectable rows.
fn assign_keys(items: &mut [MenuItem]) {
    let mut next = 0;
    for item in items {
        if matches!(item.action, MenuAction::Disabled) {
            continue;
        }
        item.key = key_at(next);
        next += 1;
    }
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

/// `text` cut to `width` cells and padded on the LEFT, so a column of numbers
/// lines up on its last digit.
fn rpad(text: &str, width: usize) -> String {
    let text = truncate(text, width);
    let mut out = String::new();
    for _ in terminal_cells(&text)..width {
        out.push(' ');
    }
    out.push_str(&text);
    out
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
            spend: String::new(),
            goal: String::new(),
        }
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
    fn rows_show_bounded_state_branch_spend_and_goal_columns() {
        let mut shown = session("hub", "$1", 4, "");
        shown.glyph = "⚠".to_owned();
        shown.branch = "feat/menu".to_owned();
        shown.spend = format!("v1;{NOW};300;12340000;exact");
        shown.goal = "ship it".to_owned();
        assert_eq!(
            labels(&bounded_menu(&[shown], 6))[0],
            concat!(
                // Each column is as wide as its own content, not as wide as its
                // cap: "hub" takes the four-cell floor, "feat/menu" nine cells.
                "hub ",
                " ",
                "⚠",
                " ",
                "needs-you",
                " ",
                "feat/menu",
                " ",
                // Right-aligned in eight cells, so the digits line up down the menu.
                "  $12.34",
                " ",
                "ship it"
            )
        );
    }

    #[test]
    fn the_spend_column_states_money_uncertainty_or_nothing() {
        let fact = |usd_micro: u64, flag: &str| format!("v1;{NOW};300;{usd_micro};{flag}");
        let mut exact = session("exact", "$1", 0, "");
        exact.spend = fact(12_340_000, "exact");
        let mut partial = session("partial", "$2", 0, "");
        partial.spend = fact(1_999_000_000, "partial");
        let mut approx = session("approx", "$3", 0, "");
        approx.spend = fact(500, "approx");
        let mut stale = session("stale", "$4", 0, "");
        stale.spend = format!("v1;{};300;9999;exact", NOW - 601);
        let mut hostile = session("hostile", "$5", 0, "");
        hostile.spend = format!("v1;{NOW};300;1;exact#[fg=red]");
        let missing = session("missing", "$6", 0, "");
        let drawn = labels(&bounded_menu(
            &[exact, partial, approx, stale, hostile, missing],
            10,
        ));
        for (name, expected) in [
            ("exact", "$12.34"),
            ("partial", "~$2.0k"),
            ("approx", "~$0.00"),
            ("stale", "-"),
            ("hostile", "-"),
            ("missing", "-"),
        ] {
            let label = drawn
                .iter()
                .find(|label| label.starts_with(name))
                .unwrap_or_else(|| panic!("a row for {name}: {drawn:?}"));
            // name, glyph, state, then the spend cell: the branch column is
            // empty in this fixture and collapses out of the split.
            assert_eq!(
                label.split_whitespace().nth(3),
                Some(expected),
                "{name}: {label:?}"
            );
        }
    }

    #[test]
    fn money_rounds_into_cents_then_thousands_and_always_fits_its_column() {
        for (usd_micro, drawn) in [
            (0_u64, "$0.00"),
            (4_999, "$0.00"),
            (5_000, "$0.01"),
            (12_345_000, "$12.35"),
            (999_994_999, "$999.99"),
            // ROUNDING picks the branch, not the raw value: one more micro-dollar
            // here would draw an eight-cell "$1000.00" that "~" could not prefix.
            (999_995_000, "$1.0k"),
            (1_000_000_000, "$1.0k"),
            (999_949_999_999, "$999.9k"),
            (999_950_000_000, "$1000k"),
            (crate::tmux::PICKER_SPEND_MAX_USD_MICRO, "$1000k"),
            // A fleet sum has no cap of its own, so the formatter must stay
            // total over the whole range rather than overflow at the top of it.
            (u64::MAX, "$18446744073k"),
        ] {
            assert_eq!(super::money(usd_micro, false), drawn, "{usd_micro}");
            assert_eq!(
                super::money(usd_micro, true),
                format!("~{drawn}"),
                "{usd_micro}"
            );
        }
        // The column never clips a number one session can publish.
        for usd_micro in [
            0,
            1,
            999_994_999,
            999_995_000,
            crate::tmux::PICKER_SPEND_MAX_USD_MICRO,
        ] {
            for uncertain in [false, true] {
                assert!(
                    super::terminal_cells(&super::money(usd_micro, uncertain))
                        <= super::SPEND_WIDTH,
                    "{usd_micro} {uncertain}"
                );
            }
        }
    }

    #[test]
    fn the_title_sums_available_facts_and_omits_the_column_when_none_are() {
        assert_eq!(
            bounded_menu(&[session("quiet", "$1", 0, "")], 6).title,
            " ae session — 1 running · 0 need you — prefix a ",
            "no fact at all leaves the title it always had"
        );
        let mut one = session("one", "$1", 0, "");
        one.spend = format!("v1;{NOW};300;1250000;exact");
        let mut two = session("two", "$2", 0, "");
        two.spend = format!("v1;{NOW};300;2500000;exact");
        let mut unreadable = session("unreadable", "$3", 0, "");
        unreadable.spend = "garbage".to_owned();
        assert_eq!(
            bounded_menu(&[one.clone(), two.clone(), unreadable], 8).title,
            " ae session — 3 running · 0 need you · ~$3.75 — prefix a ",
            "an unavailable session is left out of the sum and marks it incomplete, \
             so the title never implies whole-fleet coverage it does not have"
        );
        two.spend = format!("v1;{NOW};300;2500000;partial");
        assert_eq!(
            bounded_menu(&[one, two], 8).title,
            " ae session — 2 running · 0 need you · ~$3.75 — prefix a ",
            "one uncertain reading makes the whole sum uncertain"
        );
    }

    #[test]
    fn the_title_sum_counts_sessions_whose_rows_did_not_fit() {
        let many: Vec<PickerSession> = (0..ROW_CAP + 2)
            .map(|index| {
                let mut held = session(&format!("s{index:02}"), &format!("${}", index + 1), 0, "");
                held.spend = format!("v1;{NOW};300;1000000;exact");
                held
            })
            .collect();
        let drawn = bounded_menu(&many, 24);
        assert!(
            drawn
                .items
                .last()
                .is_some_and(|item| item.label.contains("sessions omitted")),
            "this fixture must actually omit rows"
        );
        assert!(
            drawn.title.contains(&format!("· ${}.00 ", ROW_CAP + 2)),
            "the fleet sum covers every running session: {}",
            drawn.title
        );
    }

    #[test]
    fn column_widths_follow_the_drawn_rows_and_stop_at_their_caps() {
        let idle = crate::theme::Mark::Idle.glyph(true);
        let short = [session("ab", "$1", 2, ""), session("cd", "$2", 2, "")];
        assert_eq!(
            labels(&bounded_menu(&short, 6))[0],
            format!("ab   {idle} working      {:>8} ", "-"),
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
    fn the_spend_column_stays_inside_a_narrow_client() {
        let mut wide = session("wide", "$1", 0, "");
        wide.branch = "feature/long-branch".to_owned();
        wide.spend = format!("v1;{NOW};300;999999000000;partial");
        wide.goal = "界".repeat(40);
        for width in [20, 32, 48, 80] {
            let drawn = super::menu_for_client_session_in(
                &[wide.clone()],
                &[],
                true,
                &Palette::DARCULA,
                None,
                Some("$1"),
                super::PickerBounds {
                    height: 6,
                    width,
                    now_epoch: NOW,
                },
            )
            .expect("a usable client");
            let inner = width - 4;
            assert!(
                super::terminal_cells(&drawn.title) <= inner,
                "title at {width}"
            );
            for item in &drawn.items {
                assert!(
                    super::terminal_cells(&item.label) <= inner,
                    "row at {width}: {:?}",
                    item.label
                );
            }
        }
    }

    #[test]
    fn expanded_agent_rows_are_indented_static_and_guarded_by_membership() {
        let mut hub = session("hub", "$7", 2, "%10");
        hub.agents =
            "v1;2000;60;lead:fable5:working:%10;builder:gpt56sol:done:%11;gone:gpt56luna:dead:"
                .to_owned();
        let drawn = super::menu_for_client_session_in(
            &[hub],
            &[pane("$7", "%10"), pane("$7", "%11")],
            true,
            &Palette::DARCULA,
            Some("client"),
            Some("$7"),
            super::PickerBounds {
                height: 10,
                width: 100,
                now_epoch: 2_000,
            },
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

    #[test]
    fn missing_invalid_and_stale_agent_facts_are_unavailable_not_partial() {
        let mut missing = session("missing", "$1", 0, "");
        let mut invalid = session("invalid", "$2", 0, "");
        invalid.agents = "v1;2000;60;ok:fable5:done:%2;bad:fable5:unknown:%3".to_owned();
        let mut stale = session("stale", "$3", 0, "");
        stale.agents = "v1;1879;60;lead:fable5:working:%3".to_owned();
        let drawn = super::menu_for_client_session_in(
            &[missing.clone(), invalid, stale],
            &[],
            true,
            &Palette::DARCULA,
            None,
            Some("$1"),
            super::PickerBounds {
                height: 12,
                width: 100,
                now_epoch: 2_000,
            },
        )
        .expect("room for every unavailable row");
        let unavailable = drawn
            .items
            .iter()
            .filter(|item| item.label == "  agents: unavailable")
            .count();
        assert_eq!(unavailable, 3);
        missing.agents = "v1;1880;60;lead:fable5:working:%1".to_owned();
        assert!(
            super::menu_for_client_session_in(
                &[missing],
                &[],
                true,
                &Palette::DARCULA,
                None,
                Some("$1"),
                super::PickerBounds {
                    height: 6,
                    width: 100,
                    now_epoch: 2_000,
                },
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
            true,
            &Palette::DARCULA,
            None,
            Some("$1"),
            super::PickerBounds {
                height,
                width: 100,
                now_epoch: 2_000,
            },
        )
        .expect("test dimensions")
    }

    #[test]
    fn height_degrades_current_then_all_then_caps_sessions() {
        let sessions = [
            with_agents(session("current", "$1", 0, ""), 2_000, 3),
            with_agents(session("other", "$2", 0, ""), 2_000, 3),
        ];
        let all = bounded_menu(&sessions, 10);
        assert_eq!(all.items.len(), 8, "two sessions and all six agents");
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
        for drawn in [&all, &current, &collapsed] {
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
            true,
            &Palette::DARCULA,
            None,
            Some("$1"),
            super::PickerBounds {
                height: 6,
                width: 32,
                now_epoch: 2_000,
            },
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
                    true,
                    &Palette::DARCULA,
                    None,
                    None,
                    super::PickerBounds {
                        height,
                        width,
                        now_epoch: 2_000,
                    },
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
        assert_eq!(drawn.items.len(), ROW_CAP * 2 + 1);
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
        let drawn = menu(&[hostile], &[], true, &Palette::DARCULA);
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
}
