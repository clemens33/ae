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
Usage: ae orchestrator [--popup --client <name> | --attach | --no-attach | --inside-tmux | --no-autostart]

Bare `ae orchestrator` starts or reattaches the orchestrator seat from its
dedicated config under ae's state home. With `--popup --client`, pick a live
session in a tmux menu and land in its lead pane. Installed bindings supply the
client name.

The bare seat also accepts `_launch`'s `--attach`, `--no-attach`,
`--inside-tmux` and `--no-autostart` flags. Working-directory and archive flags
are picker usage errors.

The menu lists this tmux server's running ae sessions in attention order, then
creation order and name. Each row carries its live state, branch and goal.
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
    /// `--client <name>`: draw on the client that opened a status menu.
    pub client: Option<String>,
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
        }
    }

    /// The exit it takes.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Self::Help => 0,
            Self::Unknown(_) | Self::MissingClient | Self::DuplicateClient => EXIT_USAGE,
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
///     Ok(Args { popup: true, client: None })
/// );
/// assert_eq!(parse(&[]), Ok(Args { popup: false, client: None }));
/// ```
pub fn parse(tail: &[String]) -> Result<Args, Usage> {
    let mut args = Args {
        popup: false,
        client: None,
    };
    let mut rest = tail;
    while let Some((word, after)) = rest.split_first() {
        rest = after;
        match word.as_str() {
            "-h" | "--help" => return Err(Usage::Help),
            "--popup" => args.popup = true,
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

/// How wide a session name is drawn before it is cut.
const NAME_WIDTH: usize = 18;

/// How wide the state-word column is drawn.
const STATE_WIDTH: usize = 9;

/// How wide the branch column is drawn before it is cut.
const BRANCH_WIDTH: usize = 14;

/// How much of a goal survives into a row.
const GOAL_WIDTH: usize = 36;

/// How wide an agent name is drawn.
const AGENT_NAME_WIDTH: usize = 18;

/// How wide an agent profile is drawn.
const AGENT_PROFILE_WIDTH: usize = 12;

/// How wide an agent verdict word is drawn.
const AGENT_STATE_WIDTH: usize = 9;

/// The terminal facts that bound one menu draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickerBounds {
    /// Client rows, including the two menu-border rows.
    pub height: usize,
    /// Client columns, including the four menu-border columns.
    pub width: usize,
    /// Clock used to reject stale watchdog facts.
    pub now_epoch: i64,
    /// The watchdog cadence whose two intervals define freshness.
    pub watchdog_interval_secs: u64,
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
            watchdog_interval_secs: crate::watchdog_daemon::Knobs::default().interval_secs,
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
        .map(|session| {
            crate::tmux::parse_picker_agents(
                &session.agents,
                bounds.now_epoch,
                bounds.watchdog_interval_secs,
            )
        })
        .collect();
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
    let mut session_items = Vec::with_capacity(displayed);
    for (index, session) in visible.iter().take(displayed).enumerate() {
        let expanded = expansion == Expansion::All
            || (expansion == Expansion::Current && Some(index) == current);
        let suffix = (!expanded).then(|| agents_suffix(agents[index].as_deref()));
        session_items.push(session_item(
            session,
            panes,
            icons,
            client,
            opened_session,
            suffix.as_deref(),
            inner_width,
        ));
    }
    assign_keys(&mut session_items);
    let mut items = Vec::with_capacity(item_capacity.min(expanded_rows));
    for (index, session_item) in session_items.into_iter().enumerate() {
        items.push(session_item);
        let expanded = expansion == Expansion::All
            || (expansion == Expansion::Current && Some(index) == current);
        if expanded {
            match &agents[index] {
                Some(found) => items.extend(found.iter().map(|agent| {
                    agent_item(
                        visible[index],
                        agent,
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
    let title = format!(
        " ae {} — {} running · {need_you} need you — prefix a ",
        crate::VERSION,
        ranked.len(),
    );
    Menu {
        title: clip_cells(&title, inner_width),
        title_style: crate::theme::menu_title_style(palette),
        items,
    }
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

/// One session row. Its lead hint earns a guarded jump only when the build-time
/// pane snapshot also places it in this exact session.
fn session_item(
    session: &PickerSession,
    panes: &[PickerPane],
    icons: bool,
    client: Option<&str>,
    opened_session: Option<&str>,
    suffix: Option<&str>,
    max_width: usize,
) -> MenuItem {
    let glyph = if session.glyph.is_empty() {
        Mark::Idle.glyph(icons)
    } else {
        &session.glyph
    };
    let mark = Mark::from_rank_value(session.rank);
    let label = if let Some(suffix) = suffix {
        format!(
            "{} {} {} {}{}",
            pad(&clean(&session.name), NAME_WIDTH),
            pad(&clean(glyph), 1),
            pad(&clean(mark.word()), STATE_WIDTH),
            pad(&clean(&session.branch), BRANCH_WIDTH),
            suffix,
        )
    } else {
        format!(
            "{} {} {} {} {}",
            pad(&clean(&session.name), NAME_WIDTH),
            pad(&clean(glyph), 1),
            pad(&clean(mark.word()), STATE_WIDTH),
            pad(&clean(&session.branch), BRANCH_WIDTH),
            truncate(&clean(&session.goal), GOAL_WIDTH),
        )
    };
    MenuItem {
        label: clip_cells(&label, max_width),
        key: String::new(),
        action: picker_action(session, &session.main_pane, panes, client, opened_session),
    }
}

/// One agent row, sharing the session row's two-phase pane-membership guard.
fn agent_item(
    session: &PickerSession,
    agent: &crate::tmux::PickerAgent,
    panes: &[PickerPane],
    icons: bool,
    client: Option<&str>,
    opened_session: Option<&str>,
    max_width: usize,
) -> MenuItem {
    let label = format!(
        "  {} {} {} {}",
        agent.mark().glyph(icons),
        pad(&agent.name, AGENT_NAME_WIDTH),
        pad(&agent.profile, AGENT_PROFILE_WIDTH),
        pad(&agent.state, AGENT_STATE_WIDTH),
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

fn terminal_cells(text: &str) -> usize {
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
                client: Some("/dev/ttys007".to_owned()),
            })
        );
        assert_eq!(
            parse(&[]),
            Ok(Args {
                popup: false,
                client: None
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
    fn rows_show_bounded_state_branch_and_goal_columns() {
        let mut shown = session("hub", "$1", 4, "");
        shown.glyph = "⚠".to_owned();
        shown.branch = "feat/menu".to_owned();
        shown.goal = "ship it".to_owned();
        assert_eq!(
            labels(&menu(&[shown], &[], true, &Palette::DARCULA))[0],
            concat!(
                "hub               ",
                " ",
                "⚠",
                " ",
                "needs-you",
                " ",
                "feat/menu     ",
                " ",
                "ship it"
            )
        );
    }

    #[test]
    fn expanded_agent_rows_are_indented_static_and_guarded_by_membership() {
        let mut hub = session("hub", "$7", 2, "%10");
        hub.agents =
            "v1;2000;lead:fable5:working:%10;builder:gpt56sol:done:%11;gone:gpt56luna:dead:"
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
                watchdog_interval_secs: 60,
            },
        )
        .expect("room for one session and three agents");
        assert_eq!(drawn.items.len(), 4);
        assert_eq!(drawn.items[0].key, "1");
        assert_eq!(
            drawn.items[1].label,
            "  ● lead               fable5       working  "
        );
        assert_eq!(
            drawn.items[2].label,
            "  ✓ builder            gpt56sol     done     "
        );
        assert_eq!(
            drawn.items[3].label,
            "  ✖ gone               gpt56luna    dead     "
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
        invalid.agents = "v1;2000;ok:fable5:done:%2;bad:fable5:unknown:%3".to_owned();
        let mut stale = session("stale", "$3", 0, "");
        stale.agents = "v1;1879;lead:fable5:working:%3".to_owned();
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
                watchdog_interval_secs: 60,
            },
        )
        .expect("room for every unavailable row");
        let unavailable = drawn
            .items
            .iter()
            .filter(|item| item.label == "  agents: unavailable")
            .count();
        assert_eq!(unavailable, 3);
        missing.agents = "v1;1880;lead:fable5:working:%1".to_owned();
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
                    watchdog_interval_secs: 60,
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
            "v1;{epoch};{}",
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
                watchdog_interval_secs: 60,
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
                watchdog_interval_secs: 60,
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
                        watchdog_interval_secs: 60,
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
            format!(
                " ae {} — 3 running · 2 need you — prefix a ",
                crate::VERSION
            )
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
