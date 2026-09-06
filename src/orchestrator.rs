//! `ae orchestrator --popup` — the fleet picker tmux draws for itself.
//!
//! Two menus and no program: the first lists ae's running sessions in
//! attention order, the second lists the chosen session's agents, and choosing
//! an agent hands the client to that agent's pane. Nothing is polled, nothing
//! is stored, and nothing new is read: the rows come from the very
//! [`World`] behind `ae list --json`, plus the pane roster the caller's own
//! tmux server reports in one listing.
//!
//! Coming BACK is tmux's own `switch-client -l`: a picker that remembered
//! where you were would be a second answer to a question tmux already answers,
//! and two answers can disagree.

use crate::attention::Reason;
use crate::digest::{SessionEntry, Status};
use crate::listing::World;
use crate::theme::{Mark, Palette};
use crate::time::Timestamp;
use crate::tmux::{Menu, MenuAction, MenuItem, jump_command, switch_command};

/// `--help`, verbatim.
pub const USAGE: &str = "\
Usage: ae orchestrator [--popup]

Bare `ae orchestrator` starts or reattaches the orchestrator seat. With
`--popup`, pick a session, then an agent, in a tmux menu.

The menu lists every running ae session in attention order — dead, stale,
waiting-user, blocked, throttled, unanswered, then the quiet ones by name —
with its agent count and goal. Choosing one lists its agents with their
declared state and attention reason; choosing an agent switches this client to
that agent's pane.

Coming back is tmux's own: switch-client -l (prefix + L by default).

Bind it:  bind o run-shell \"ae orchestrator --popup\"

A session on another tmux server cannot be switched to from here, so its row is
disabled and shows the attach command instead.

";

/// The canonical orchestrator session name.
pub const ORCHESTRATOR_SESSION: &str = "orchestrator";

/// The code a refused orchestrator invocation takes.
pub const EXIT_USAGE: u8 = 2;

/// What the argv asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Args {
    /// `--popup`: draw the picker.
    pub popup: bool,
}

/// An argv `ae orchestrator` refuses, or the help it treats as one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Usage {
    /// `-h` / `--help` — the text, at exit 0.
    Help,
    /// A word this command does not accept, carried for its message.
    Unknown(String),
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
        }
    }

    /// The exit it takes.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Self::Help => 0,
            Self::Unknown(_) => EXIT_USAGE,
        }
    }
}

/// Read `ae orchestrator`'s flags.
///
/// # Errors
///
/// [`Usage::Help`] for the help spellings, [`Usage::Unknown`] for anything else.
///
/// ```
/// use ae::orchestrator::{parse, Args, Usage};
/// assert_eq!(parse(&["--popup".to_owned()]), Ok(Args { popup: true }));
/// assert_eq!(parse(&[]), Ok(Args { popup: false }));
/// ```
pub fn parse(tail: &[String]) -> Result<Args, Usage> {
    let mut args = Args { popup: false };
    for word in tail {
        match word.as_str() {
            "-h" | "--help" => return Err(Usage::Help),
            "--popup" => args.popup = true,
            other => return Err(Usage::Unknown(other.to_owned())),
        }
    }
    Ok(args)
}

/// The user-facing launch tail for the canonical orchestrator seat.
#[must_use]
pub fn seat_launch_args() -> Vec<String> {
    vec![ORCHESTRATOR_SESSION.to_owned(), "--local".to_owned()]
}

/// Where one session's panes are, as the caller's own tmux server sees them.
pub enum Placement {
    /// On this server: its stamped agent panes, in the order tmux listed them.
    Here(Vec<AgentPane>),
    /// On some other server, which no `switch-client` from here can reach.
    /// Carries the command that WOULD reach it.
    Elsewhere(String),
}

/// One stamped agent pane of a session on the caller's server.
pub struct AgentPane {
    /// The `@ae_agent` stamp — the name the roster addresses it by.
    pub agent: String,
    /// The `%<n>` pane id, which is what the jump targets.
    pub pane: String,
}

/// Everything the picker needs about one session beyond its [`SessionEntry`].
pub struct Located {
    /// The session name, matching the entry's.
    pub session: String,
    /// Where its panes are.
    pub placement: Placement,
}

/// At most this many session rows, so the menu fits a terminal and the key
/// alphabet below is never exhausted.
pub const ROW_CAP: usize = 30;

/// The shortcut keys, in the order rows take them. No `q`: tmux closes a menu
/// on `q`, and a row that stole it would trap the human in the picker.
const KEYS: &str = "123456789abcdefghijklmnoprstuvwxyz";

/// How wide a session name is drawn before it is cut.
const NAME_WIDTH: usize = 18;

/// How wide the attention word is drawn.
const ATTENTION_WIDTH: usize = 12;

/// How wide an agent's declared state is drawn.
const STATE_WIDTH: usize = 12;

/// How wide an agent's name is drawn.
const AGENT_WIDTH: usize = 22;

/// How much of a goal survives into a row.
const GOAL_WIDTH: usize = 30;

/// What an absent fact is drawn as.
const ABSENT: &str = "-";

/// What a fact ae could not establish is drawn as — never a quiet blank, and
/// never the spelling of a fact that holds.
const UNKNOWN: &str = "?";

/// The ATTENTION column's spelling of the same thing.
///
/// A word, not the `?`: a session ae could not read is a different situation
/// from one that went quiet, and the two share a glyph, so the reason beside it
/// is what has to tell them apart.
const UNKNOWN_ATTENTION: &str = "unknown";

/// The picker, as a menu model — the whole pure step.
///
/// `located` describes the sessions of `world` by name; a session with no
/// entry there is treated as absent from this server.
#[must_use]
pub fn menu(
    world: &World,
    located: &[Located],
    now: Timestamp,
    icons: bool,
    palette: &Palette,
) -> Menu {
    let ranked = ranked_sessions(world);
    let shown = ranked.len().min(ROW_CAP);
    let mut items: Vec<MenuItem> = Vec::with_capacity(shown + 1);
    for session in ranked.iter().take(shown) {
        let placement = located
            .iter()
            .find(|entry| entry.session == session.name)
            .map(|entry| &entry.placement);
        items.push(session_item(session, placement, now, icons, palette));
    }
    if ranked.len() > shown {
        items.push(disabled(format!(
            "… {} more — see ae list",
            ranked.len() - shown
        )));
    }
    if items.is_empty() {
        items.push(disabled("no running ae sessions".to_owned()));
    }
    assign_keys(&mut items);
    Menu {
        title: format!(" ae fleet — {} running ", ranked.len()),
        title_style: crate::theme::menu_title_style(palette),
        items,
    }
}

/// The running sessions of `world`, most actionable first.
///
/// Severity decides, then the name — ascending, so the order is the same on
/// every invocation whatever order the inventory enumerated in.
fn ranked_sessions(world: &World) -> Vec<&SessionEntry> {
    let mut running: Vec<&SessionEntry> = world
        .sessions
        .iter()
        .filter(|session| session.status == Status::Running)
        .collect();
    running.sort_by(|left, right| {
        rank_of(right)
            .cmp(&rank_of(left))
            .then_with(|| left.name.cmp(&right.name))
    });
    running
}

/// A session's severity as a sortable number — 0 when nothing wants a human.
///
/// The marker is read whether or not the evidence behind it was complete: an
/// unproven `dead` is still the row a human should look at first, and the label
/// says `?` so the order is not mistaken for a proven one.
fn rank_of(session: &SessionEntry) -> i64 {
    session.attention.map_or(0, Reason::rank)
}

/// One session's row, and the second menu behind it.
fn session_item(
    session: &SessionEntry,
    placement: Option<&Placement>,
    now: Timestamp,
    icons: bool,
    palette: &Palette,
) -> MenuItem {
    let agents: Vec<&AgentPane> = match placement {
        Some(Placement::Here(panes)) => panes.iter().filter(|pane| is_agent(pane)).collect(),
        _ => Vec::new(),
    };
    let label = format!(
        "{} {} {} {:>2}ag {}",
        pad(&clean(&session.name), NAME_WIDTH),
        attention_mark(session).glyph(icons),
        pad(attention_word(session), ATTENTION_WIDTH),
        agents.len(),
        goal_of(session),
    );
    let action = match placement {
        // The attention column SURVIVES: a session that needs a human needs one
        // whether or not this client can reach it.
        Some(Placement::Elsewhere(attach)) => {
            return disabled(format!(
                "{} {} {} {}",
                pad(&clean(&session.name), NAME_WIDTH),
                attention_mark(session).glyph(icons),
                pad(attention_word(session), ATTENTION_WIDTH),
                clean(attach)
            ));
        }
        _ if !name_is_targetable(&session.name) => {
            return disabled(format!(
                "{} {} {} not a name ae will target",
                pad(&clean(&session.name), NAME_WIDTH),
                attention_mark(session).glyph(icons),
                pad(attention_word(session), ATTENTION_WIDTH)
            ));
        }
        // Nothing to pick between, so the row does the only useful thing.
        _ if agents.is_empty() => MenuAction::Run(switch_command(&session.name)),
        _ => MenuAction::Open(agent_menu(session, &agents, now, icons, palette)),
    };
    MenuItem {
        label,
        key: String::new(),
        action,
    }
}

/// The second menu: one row per stamped agent pane of `session`.
fn agent_menu(
    session: &SessionEntry,
    agents: &[&AgentPane],
    now: Timestamp,
    icons: bool,
    palette: &Palette,
) -> Menu {
    let shown = agents.len().min(ROW_CAP);
    let mut items: Vec<MenuItem> = Vec::with_capacity(shown + 1);
    for pane in agents.iter().take(shown) {
        items.push(agent_item(session, pane, icons));
    }
    if agents.len() > shown {
        items.push(disabled(format!("… {} more", agents.len() - shown)));
    }
    assign_keys(&mut items);
    Menu {
        title: format!(
            " {} — attn:{} — active {} ",
            clean(&session.name),
            attention_word(session),
            age(now, session.last_active_epoch)
        ),
        title_style: crate::theme::menu_title_style(palette),
        items,
    }
}

/// One agent's row: what it declared, what it is asking for, and its pane.
fn agent_item(session: &SessionEntry, pane: &AgentPane, icons: bool) -> MenuItem {
    let entry = session
        .agents
        .iter()
        .find(|agent| agent.name == pane.agent || agent.reference == pane.agent);
    // CLEANED: a declared state is whatever an event line carried, and the
    // journal it came from is hand-editable.
    let state = match (session.agent_state_is_exact(), entry) {
        (false, _) => UNKNOWN.to_owned(),
        (true, None) => ABSENT.to_owned(),
        (true, Some(entry)) => entry
            .state
            .as_deref()
            .map(clean)
            .filter(|state| !state.is_empty())
            .unwrap_or_else(|| ABSENT.to_owned()),
    };
    let reason = match (session.attention_is_exact(), entry.and_then(|e| e.reason)) {
        (false, _) => UNKNOWN.to_owned(),
        (true, None) => ABSENT.to_owned(),
        (true, Some(reason)) => reason.as_str().to_owned(),
    };
    let label = format!(
        "{} {} {} {} {}",
        pad(&clean(&pane.agent), AGENT_WIDTH),
        entry
            .and_then(|entry| entry.reason)
            .map_or(Mark::Idle, Mark::for_reason)
            .glyph(icons),
        pad(&state, STATE_WIDTH),
        pad(&reason, ATTENTION_WIDTH),
        clean(&pane.pane),
    );
    let action = if pane_is_targetable(&pane.pane) {
        MenuAction::Run(jump_command(&session.name, &pane.pane))
    } else {
        MenuAction::Disabled
    };
    MenuItem {
        label,
        key: String::new(),
        action,
    }
}

/// Hand out the shortcuts, to the rows that can actually be chosen.
///
/// A disabled row takes no key: it cannot be chosen, and a key spent on one
/// would push the row below it onto a different digit than the fleet's shape
/// suggests.
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

/// Whether a stamped pane is an AGENT's.
///
/// The core stamps its own monitor panes with names outside the agent grammar
/// — `_watchdog`, `_events` — precisely so a surface that lists agents can tell
/// them apart from one, and the roster the digest carries holds neither.
fn is_agent(pane: &AgentPane) -> bool {
    crate::config::is_agent_name(&pane.agent)
}

/// Whether a session name is one ae will put inside a tmux command.
fn name_is_targetable(name: &str) -> bool {
    crate::session_launch::name::is_session_name(name)
}

/// Whether a pane id is tmux's own `%<digits>` and nothing else.
fn pane_is_targetable(pane: &str) -> bool {
    match pane.strip_prefix('%') {
        Some(digits) => !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()),
        None => false,
    }
}

/// The mark a row draws: the rollup's own, and [`Mark::Stale`] when ae could
/// not establish one — never a quiet or working glyph, which would claim a
/// verdict ae does not have.
fn attention_mark(session: &SessionEntry) -> Mark {
    if !session.attention_is_exact() {
        return Mark::Stale;
    }
    session.attention.map_or(Mark::Idle, Mark::for_reason)
}

/// The attention word a row shows: the rollup, `-` for a quiet session, and
/// `unknown` when the evidence behind the marker was incomplete.
fn attention_word(session: &SessionEntry) -> &str {
    if !session.attention_is_exact() {
        return UNKNOWN_ATTENTION;
    }
    session.attention.map_or(ABSENT, Reason::as_str)
}

/// The goal, cut to the row's width.
fn goal_of(session: &SessionEntry) -> String {
    let goal = session.goal.as_deref().unwrap_or_default();
    truncate(&clean(goal), GOAL_WIDTH)
}

/// `text` cut to `width` CHARACTERS, the last of them an ellipsis when it was
/// cut. Nothing is padded: a row's last column has no column after it.
fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// `text` cut to `width` characters and then padded to it, so the column after
/// it starts in the same place on every row.
fn pad(text: &str, width: usize) -> String {
    let mut out = truncate(text, width);
    for _ in out.chars().count()..width {
        out.push(' ');
    }
    out
}

/// The apostrophe a quote is rewritten to — U+02BC, which reads as one and is
/// not the character tmux's parser ends a quoted word on.
const SAFE_APOSTROPHE: char = '\u{02bc}';

/// `text` as a menu row can carry it.
///
/// A goal is operator text and reaches this from a file anyone can edit.
/// Control characters would break the drawing, so they go; a straight quote
/// would end the quoting of the nested menu the row lives in, so it is
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

/// How long ago `epoch` was, as a menu column rather than as a sentence.
fn age(now: Timestamp, epoch: Option<i64>) -> String {
    let Some(epoch) = epoch.filter(|epoch| *epoch > 0) else {
        return ABSENT.to_owned();
    };
    let delta = now.epoch().saturating_sub(epoch);
    if delta < 0 {
        "now".to_owned()
    } else if delta < 60 {
        format!("{delta}s")
    } else if delta < 3_600 {
        format!("{}m", delta / 60)
    } else if delta < 86_400 {
        format!("{}h", delta / 3_600)
    } else if delta < 604_800 {
        format!("{}d", delta / 86_400)
    } else {
        ">7d".to_owned()
    }
}

/// The command that reaches `session` on the server ae recorded for it.
///
/// ```
/// use ae::inventory::ServerId;
/// use ae::meta::Selector;
/// let server = ServerId::Selected(Selector::Name("ae-dev".to_owned()));
/// assert_eq!(ae::orchestrator::attach_command(&server, "hub"), "tmux -L ae-dev attach -t hub");
/// ```
#[must_use]
pub fn attach_command(server: &crate::inventory::ServerId, session: &str) -> String {
    use crate::inventory::ServerId;
    use crate::meta::Selector;
    match server {
        ServerId::Ambient => format!("tmux attach -t {session}"),
        ServerId::Selected(Selector::Name(name)) => format!("tmux -L {name} attach -t {session}"),
        ServerId::Selected(Selector::Socket(path)) => {
            format!("tmux -S {} attach -t {session}", path.display())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AgentPane, Args, KEYS, Located, Placement, ROW_CAP, Usage, attach_command, menu, parse,
    };
    use crate::attention::Reason;
    use crate::digest::{AgentEntry, SessionEntry, Status};
    use crate::inventory::ServerId;
    use crate::listing::World;
    use crate::meta::Selector;
    use crate::theme::{Mark, Palette};
    use crate::time::Timestamp;
    use crate::tmux::{Menu, MenuAction, display_menu_args};

    const NOW: Timestamp = Timestamp::from_epoch(1_780_000_000);

    fn world(sessions: Vec<SessionEntry>) -> World {
        World::new(NOW, sessions)
    }

    fn running(name: &str) -> SessionEntry {
        SessionEntry::new(name, Status::Running)
    }

    fn agent(name: &str, state: Option<&str>, reason: Option<Reason>) -> AgentEntry {
        AgentEntry {
            reference: name.to_owned(),
            alias: "cl".to_owned(),
            name: name.to_owned(),
            session_id: None,
            alive: Some(true),
            state: state.map(ToOwned::to_owned),
            reason,
        }
    }

    fn here(session: &str, panes: &[(&str, &str)]) -> Located {
        Located {
            session: session.to_owned(),
            placement: Placement::Here(
                panes
                    .iter()
                    .map(|(agent, pane)| AgentPane {
                        agent: (*agent).to_owned(),
                        pane: (*pane).to_owned(),
                    })
                    .collect(),
            ),
        }
    }

    /// The rows' labels, in the order they are drawn.
    fn labels(menu: &Menu) -> Vec<String> {
        menu.items.iter().map(|item| item.label.clone()).collect()
    }

    /// The first word of every row — the session or agent name column.
    fn names(menu: &Menu) -> Vec<String> {
        labels(menu)
            .iter()
            .filter_map(|label| label.split_whitespace().next().map(ToOwned::to_owned))
            .collect()
    }

    /// The submenu behind the row whose label starts with `name`.
    fn submenu<'a>(menu: &'a Menu, name: &str) -> &'a Menu {
        let item = menu
            .items
            .iter()
            .find(|item| item.label.starts_with(name))
            .unwrap_or_else(|| panic!("no row for {name} in {:?}", labels(menu)));
        match &item.action {
            MenuAction::Open(inner) => inner,
            _ => panic!("{name} opens no second menu"),
        }
    }

    #[test]
    fn the_flags_the_picker_accepts_and_the_ones_it_refuses() {
        assert!(parse(&["--popup".to_owned()]).unwrap().popup);
        assert_eq!(parse(&[]), Ok(Args { popup: false }));
        for spelling in ["-h", "--help"] {
            assert_eq!(
                parse(&[spelling.to_owned()]),
                Err(Usage::Help),
                "{spelling}"
            );
        }
        assert_eq!(Usage::Help.code(), 0);
        assert_eq!(
            parse(&["--nope".to_owned()]),
            Err(Usage::Unknown("--nope".to_owned()))
        );
        assert_eq!(Usage::Unknown("--nope".to_owned()).code(), 2);
    }

    #[test]
    fn the_bare_word_routes_to_the_canonical_local_seat() {
        assert_eq!(
            super::seat_launch_args(),
            vec!["orchestrator".to_owned(), "--local".to_owned()]
        );
        assert_eq!(super::ORCHESTRATOR_SESSION, "orchestrator");
    }

    #[test]
    fn the_usage_names_the_binding_and_the_way_back() {
        assert!(super::USAGE.contains("run-shell"));
        assert!(
            super::USAGE.contains("switch-client -l"),
            "the return key is tmux's own, so the usage has to say so"
        );
    }

    #[test]
    fn severity_orders_the_rows_and_the_name_breaks_every_tie() {
        let mut dead = running("zzz");
        dead.attention = Some(Reason::Dead);
        let mut blocked = running("aaa");
        blocked.attention = Some(Reason::Blocked);
        let mut stale_b = running("bbb");
        stale_b.attention = Some(Reason::Stale);
        let mut stale_a = running("abb");
        stale_a.attention = Some(Reason::Stale);
        let quiet = running("aab");

        let world = world(vec![quiet, blocked, stale_b, dead, stale_a]);
        let drawn = menu(&world, &[], NOW, true, &Palette::DARCULA);
        assert_eq!(names(&drawn), ["zzz", "abb", "bbb", "aaa", "aab"]);
    }

    #[test]
    fn an_unproven_marker_still_sorts_as_the_reason_it_claims_and_shows_that_it_is_unproven() {
        let mut degraded = SessionEntry::degraded("murky", Status::Running);
        degraded.attention = Some(Reason::Dead);
        let mut calm = running("aaa");
        calm.attention = Some(Reason::Blocked);
        let drawn = menu(
            &world(vec![calm, degraded]),
            &[],
            NOW,
            true,
            &Palette::DARCULA,
        );
        assert_eq!(names(&drawn), ["murky", "aaa"], "unproven is not quiet");
        assert!(
            labels(&drawn)[0].contains("unknown"),
            "{:?}",
            labels(&drawn)
        );
        assert!(
            !labels(&drawn)[0].contains("dead"),
            "the row must not claim a reason the evidence did not establish"
        );
    }

    #[test]
    fn only_a_running_session_is_a_row_because_only_it_can_be_switched_to() {
        let mut stopped = SessionEntry::new("gone", Status::Stopped);
        stopped.attention = Some(Reason::Dead);
        let mut unknown = SessionEntry::new("unsure", Status::Unknown);
        unknown.attention = Some(Reason::Dead);
        let drawn = menu(
            &world(vec![stopped, unknown, running("live")]),
            &[],
            NOW,
            true,
            &Palette::DARCULA,
        );
        assert_eq!(names(&drawn), ["live"]);
        assert!(drawn.title.contains("1 running"));
    }

    #[test]
    fn an_empty_fleet_says_so_in_a_row_that_cannot_be_chosen() {
        let drawn = menu(&world(Vec::new()), &[], NOW, true, &Palette::DARCULA);
        assert_eq!(labels(&drawn), ["no running ae sessions"]);
        assert!(matches!(drawn.items[0].action, MenuAction::Disabled));
    }

    #[test]
    fn the_row_cap_holds_and_names_what_it_left_out() {
        let sessions: Vec<SessionEntry> = (0..ROW_CAP + 7)
            .map(|index| running(&format!("s{index:03}")))
            .collect();
        let drawn = menu(&world(sessions), &[], NOW, true, &Palette::DARCULA);
        assert_eq!(drawn.items.len(), ROW_CAP + 1, "the cap, plus its own note");
        let last = drawn.items.last().expect("a note");
        assert_eq!(last.label, "… 7 more — see ae list");
        assert!(matches!(last.action, MenuAction::Disabled));
        // Stable: the SAME rows, whatever order the world enumerated in.
        let reversed: Vec<SessionEntry> = (0..ROW_CAP + 7)
            .rev()
            .map(|index| running(&format!("s{index:03}")))
            .collect();
        assert_eq!(
            names(&menu(&world(reversed), &[], NOW, true, &Palette::DARCULA)),
            names(&drawn)
        );
    }

    #[test]
    fn every_shortcut_is_distinct_and_none_of_them_is_the_key_that_closes_a_menu() {
        assert!(KEYS.chars().count() >= ROW_CAP);
        assert!(!KEYS.contains('q'), "q closes the menu");
        let mut seen: Vec<char> = KEYS.chars().collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), KEYS.chars().count());

        let sessions: Vec<SessionEntry> =
            (0..3).map(|index| running(&format!("s{index}"))).collect();
        let drawn = menu(&world(sessions), &[], NOW, true, &Palette::DARCULA);
        let keys: Vec<String> = drawn.items.iter().map(|item| item.key.clone()).collect();
        assert_eq!(keys, ["1", "2", "3"]);
    }

    #[test]
    fn a_long_name_and_a_long_goal_are_cut_with_an_ellipsis_and_never_wrap() {
        let mut session = running("a-session-name-far-longer-than-its-column");
        session.goal = Some("x".repeat(200));
        let drawn = menu(&world(vec![session]), &[], NOW, true, &Palette::DARCULA);
        let label = &labels(&drawn)[0];
        assert!(label.contains("a-session-name-fa…"), "{label}");
        assert!(label.ends_with('…'), "{label}");
        assert!(
            label.chars().count() <= 18 + 1 + 1 + 1 + 12 + 1 + 4 + 1 + 30,
            "the row is bounded: {}",
            label.chars().count()
        );
    }

    #[test]
    fn a_quiet_session_shows_a_dash_and_an_inexact_one_shows_a_question_mark() {
        let quiet = running("calm");
        let mut degraded = SessionEntry::degraded("murky", Status::Running);
        degraded.attention = Some(Reason::Dead);
        let drawn = menu(
            &world(vec![quiet, degraded]),
            &[],
            NOW,
            true,
            &Palette::DARCULA,
        );
        let rendered = labels(&drawn).join("\n");
        // The glyph sits between the name and the word, and shares its
        // vocabulary with the status bar: nothing for a quiet session, the
        // stale mark for one whose evidence was incomplete — never a green one.
        // The WORD behind that mark says which of the two it was.
        assert!(
            rendered.contains(&format!("calm               {} -", Mark::Idle.glyph(true))),
            "{rendered}"
        );
        assert!(
            rendered.contains(&format!(
                "murky              {} unknown",
                Mark::Stale.glyph(true)
            )),
            "{rendered}"
        );
    }

    /// The picker draws the SAME marks the status bar does, in whichever
    /// vocabulary the calling session asked for.
    #[test]
    fn the_picker_draws_the_shared_marks_and_falls_back_to_ascii() {
        let mut waiting = running("waits");
        waiting.attention = Some(Reason::WaitingUser);
        let rendered = |icons| {
            labels(&menu(
                &world(vec![waiting.clone()]),
                &[],
                NOW,
                icons,
                &Palette::DARCULA,
            ))
            .join("\n")
        };
        assert!(
            rendered(true).contains("⚠ waiting-user"),
            "{}",
            rendered(true)
        );
        assert!(
            rendered(false).contains("! waiting-user"),
            "{}",
            rendered(false)
        );
    }

    #[test]
    fn choosing_an_agent_switches_the_session_then_the_window_then_the_pane() {
        let mut session = running("hub");
        session.agents = vec![agent("lead", Some("working"), None)];
        let world = world(vec![session]);
        let located = [here("hub", &[("lead", "%12")])];
        let drawn = menu(&world, &located, NOW, true, &Palette::DARCULA);
        let agents = submenu(&drawn, "hub");
        let MenuAction::Run(command) = &agents.items[0].action else {
            panic!("the agent row runs a command");
        };
        assert_eq!(
            command,
            "switch-client -t hub ; select-window -t %12 ; select-pane -t %12"
        );
        // The window BEFORE the pane: a worker lives in its own window and
        // select-pane alone does not change which window is viewed.
        let window = command.find("select-window").expect("a window step");
        let pane = command.find("select-pane").expect("a pane step");
        assert!(window < pane, "{command}");
    }

    #[test]
    fn the_agent_row_carries_the_declared_state_the_reason_and_the_pane() {
        let mut session = running("hub");
        session.agents = vec![
            agent("lead", Some("blocked"), Some(Reason::Blocked)),
            agent("quiet", None, None),
        ];
        let world = world(vec![session]);
        let located = [here("hub", &[("lead", "%1"), ("quiet", "%2")])];
        let drawn = menu(&world, &located, NOW, true, &Palette::DARCULA);
        let agents = submenu(&drawn, "hub");
        let rendered = labels(agents);
        assert!(rendered[0].starts_with("lead"), "{rendered:?}");
        assert!(rendered[0].contains("blocked"), "{rendered:?}");
        assert!(rendered[0].ends_with("%1"), "{rendered:?}");
        assert!(rendered[1].contains('-'), "{rendered:?}");
        assert!(rendered[1].ends_with("%2"), "{rendered:?}");
    }

    #[test]
    fn the_core_own_monitor_panes_are_not_agents_and_are_neither_counted_nor_listed() {
        // `_watchdog` and `_events` are stamped OUTSIDE the agent grammar, so
        // the count on the session row agrees with `ae list`'s roster.
        let mut session = running("hub");
        session.agents = vec![agent("lead", Some("working"), None)];
        let located = [here(
            "hub",
            &[("lead", "%1"), ("_watchdog", "%2"), ("_events", "%3")],
        )];
        let drawn = menu(
            &world(vec![session]),
            &located,
            NOW,
            true,
            &Palette::DARCULA,
        );
        assert!(labels(&drawn)[0].contains(" 1ag "), "{:?}", labels(&drawn));
        let agents = submenu(&drawn, "hub");
        assert_eq!(names(agents), ["lead"]);
    }

    #[test]
    fn a_pane_with_no_roster_entry_is_still_a_row_because_it_is_still_a_pane() {
        // The stamp is what tmux reports; a roster that has not caught up
        // must not hide a live pane from the human.
        let session = running("hub");
        let world = world(vec![session]);
        let located = [here("hub", &[("ghost", "%9")])];
        let drawn = menu(&world, &located, NOW, true, &Palette::DARCULA);
        let agents = submenu(&drawn, "hub");
        assert!(labels(agents)[0].starts_with("ghost"));
        assert!(matches!(agents.items[0].action, MenuAction::Run(_)));
    }

    #[test]
    fn the_second_menu_names_the_session_its_attention_and_how_long_since_it_moved() {
        let mut session = running("hub");
        session.attention = Some(Reason::Stale);
        session.last_active_epoch = Some(NOW.epoch() - 5_400);
        session.agents = vec![agent("lead", Some("working"), None)];
        let located = [here("hub", &[("lead", "%1")])];
        let drawn = menu(
            &world(vec![session]),
            &located,
            NOW,
            true,
            &Palette::DARCULA,
        );
        let agents = submenu(&drawn, "hub");
        assert_eq!(agents.title, " hub — attn:stale — active 1h ");
    }

    #[test]
    fn a_session_with_no_stamped_pane_still_switches_to_the_session() {
        let world = world(vec![running("bare")]);
        let located = [here("bare", &[])];
        let drawn = menu(&world, &located, NOW, true, &Palette::DARCULA);
        let MenuAction::Run(command) = &drawn.items[0].action else {
            panic!("a session with nothing to pick between still jumps");
        };
        assert_eq!(command, "switch-client -t bare");
    }

    #[test]
    fn a_session_on_another_server_is_a_disabled_row_carrying_its_attach_command() {
        let quiet = world(vec![running("far")]);
        let located = [Located {
            session: "far".to_owned(),
            placement: Placement::Elsewhere(attach_command(
                &ServerId::Selected(Selector::Name("ae-dev".to_owned())),
                "far",
            )),
        }];
        let drawn = menu(&quiet, &located, NOW, true, &Palette::DARCULA);
        assert!(matches!(drawn.items[0].action, MenuAction::Disabled));
        assert!(
            labels(&drawn)[0].contains("tmux -L ae-dev attach -t far"),
            "{:?}",
            labels(&drawn)
        );

        // A session ae cannot reach can still be the one that needs a human,
        // so the attention column survives on the row that cannot be chosen.
        let mut hot = running("far");
        hot.attention = Some(Reason::Dead);
        let located = [Located {
            session: "far".to_owned(),
            placement: Placement::Elsewhere("tmux -L ae-dev attach -t far".to_owned()),
        }];
        let drawn = menu(&world(vec![hot]), &located, NOW, true, &Palette::DARCULA);
        assert!(labels(&drawn)[0].contains("dead"), "{:?}", labels(&drawn));
    }

    #[test]
    fn every_way_of_naming_a_server_names_the_command_that_reaches_it() {
        assert_eq!(attach_command(&ServerId::Ambient, "s"), "tmux attach -t s");
        assert_eq!(
            attach_command(&ServerId::Selected(Selector::Socket("/tmp/x".into())), "s"),
            "tmux -S /tmp/x attach -t s"
        );
    }

    #[test]
    fn a_pane_id_that_is_not_tmux_own_grammar_cannot_be_jumped_to() {
        let world = world(vec![running("hub")]);
        for hostile in ["", "12", "%", "%1;kill-server", "%1 x"] {
            let located = [here("hub", &[("lead", hostile)])];
            let drawn = menu(&world, &located, NOW, true, &Palette::DARCULA);
            let agents = submenu(&drawn, "hub");
            assert!(
                matches!(agents.items[0].action, MenuAction::Disabled),
                "{hostile:?} must not become a target"
            );
        }
    }

    #[test]
    fn a_declared_state_cannot_corrupt_the_menu_it_is_drawn_in() {
        // The state comes off a journal line, and a journal is hand-editable.
        let mut session = running("hub");
        session.agents = vec![agent(
            "lead",
            Some("work\u{7}ing' ; kill-server ; #{q}"),
            None,
        )];
        let located = [here("hub", &[("lead", "%1")])];
        let drawn = menu(
            &world(vec![session]),
            &located,
            NOW,
            true,
            &Palette::DARCULA,
        );
        let agents = submenu(&drawn, "hub");
        let label = &labels(agents)[0];
        assert!(!label.contains('\''), "{label:?}");
        assert!(!label.chars().any(char::is_control), "{label:?}");
        let words = display_menu_args(&ServerId::Ambient, &drawn);
        let nested = words.last().expect("the nested menu");
        assert!(!nested.contains("kill-server ;"), "{nested}");
        assert!(!nested.contains("#{q}"), "the hash is escaped: {nested}");
    }

    #[test]
    fn a_goal_cannot_end_the_quoting_of_the_menu_it_is_drawn_in() {
        let mut session = running("hub");
        session.goal = Some("ship 'it' now\u{7}\nand more".to_owned());
        session.agents = vec![agent("lead", Some("working"), None)];
        let located = [here("hub", &[("lead", "%1")])];
        let drawn = menu(
            &world(vec![session]),
            &located,
            NOW,
            true,
            &Palette::DARCULA,
        );
        let label = &labels(&drawn)[0];
        assert!(!label.contains('\''), "{label:?}");
        assert!(!label.chars().any(char::is_control), "{label:?}");
        assert!(
            label.contains("ship \u{02bc}it\u{02bc}"),
            "the word survives: {label:?}"
        );
        // …and the nested menu it opens is one word tmux can still parse.
        let words = display_menu_args(&ServerId::Ambient, &drawn);
        assert!(words.iter().all(|word| !word.contains('\u{7}')));
    }

    #[test]
    fn a_hash_in_a_label_is_escaped_and_a_percent_is_left_alone() {
        // Measured on tmux 3.7b: a menu name is expanded by the plain format
        // expander, so `##` collapses to `#` and `%%` stays two characters.
        let mut session = running("hub");
        session.goal = Some("100% of #{everything}".to_owned());
        let drawn = menu(&world(vec![session]), &[], NOW, true, &Palette::DARCULA);
        let words = display_menu_args(&ServerId::Ambient, &drawn);
        let label = words
            .iter()
            .find(|word| word.contains("hub"))
            .expect("the row");
        assert!(label.contains("100% of ##{everything}"), "{label}");
    }

    #[test]
    fn a_disabled_row_can_lead_the_menu_without_being_read_as_a_flag() {
        // tmux reads display-menu's flags with getopt: a first item named
        // `-far …` is `unknown flag -g` unless the flags were ended first.
        let world = world(vec![running("far")]);
        let located = [Located {
            session: "far".to_owned(),
            placement: Placement::Elsewhere("tmux -L other attach -t far".to_owned()),
        }];
        let words = display_menu_args(
            &ServerId::Ambient,
            &menu(&world, &located, NOW, true, &Palette::DARCULA),
        );
        let first_item = words
            .iter()
            .position(|word| word.starts_with('-') && word.contains("far"))
            .expect("the disabled row");
        let end_of_flags = words.iter().position(|word| word == "--").expect("--");
        assert!(end_of_flags < first_item, "{words:?}");
    }

    #[test]
    fn the_argv_is_a_menu_at_the_centre_with_a_title_and_three_words_per_row() {
        let mut session = running("hub");
        session.agents = vec![agent("lead", Some("working"), None)];
        let located = [here("hub", &[("lead", "%1")])];
        let drawn = menu(
            &world(vec![session]),
            &located,
            NOW,
            true,
            &Palette::DARCULA,
        );
        let words = display_menu_args(&ServerId::Ambient, &drawn);
        assert_eq!(&words[..5], ["display-menu", "-x", "C", "-y", "C"]);
        assert_eq!(words[5], "-T");
        assert_eq!(
            words[7], "--",
            "the flags end before the first row, or a disabled one is read as a flag"
        );
        assert_eq!(words.len(), 8 + 3, "the header, then one triplet");
        // The nested menu travels as ONE word, quoted for tmux's parser.
        assert!(words[10].starts_with("'display-menu'"), "{}", words[10]);
        assert!(words[10].contains("'--'"), "{}", words[10]);
        assert!(
            words[10].ends_with("'switch-client -t hub ; select-window -t %1 ; select-pane -t %1'"),
            "the jump travels intact through the nesting: {}",
            words[10]
        );
    }
}
