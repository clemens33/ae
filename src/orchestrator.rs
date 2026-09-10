//! `ae orchestrator --popup` — the fleet picker tmux draws for itself.
//!
//! One menu and no program: it lists the calling tmux server's live ae
//! sessions in attention order, and choosing a row hands the client to that
//! session's lead pane when the captured pane still belongs there. Nothing is
//! polled or stored; the rows and membership proof are two tmux listings.
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
Usage: ae orchestrator [--popup [--client <name>] | --attach | --no-attach | --inside-tmux | --no-autostart]

Bare `ae orchestrator` starts or reattaches the orchestrator seat from its
dedicated config under ae's state home. With `--popup`, pick a live session in
a tmux menu and land in its lead pane.

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
    let mut items: Vec<MenuItem> = Vec::with_capacity(shown + 1);
    for session in ranked.iter().take(shown) {
        items.push(session_item(session, panes, icons, client));
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
        title: format!(
            " ae {} — {} running · {need_you} need you — prefix a ",
            crate::VERSION,
            ranked.len(),
        ),
        title_style: crate::theme::menu_title_style(palette),
        items,
    }
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
) -> MenuItem {
    let glyph = if session.glyph.is_empty() {
        Mark::Idle.glyph(icons)
    } else {
        &session.glyph
    };
    let mark = Mark::from_rank_value(session.rank);
    let label = format!(
        "{} {} {} {} {}",
        pad(&clean(&session.name), NAME_WIDTH),
        pad(&clean(glyph), 1),
        pad(&clean(mark.word()), STATE_WIDTH),
        pad(&clean(&session.branch), BRANCH_WIDTH),
        truncate(&clean(&session.goal), GOAL_WIDTH),
    );
    let pane_is_member = !session.main_pane.is_empty()
        && panes
            .iter()
            .any(|pane| pane.session_id == session.id && pane.pane == session.main_pane);
    let action = if pane_is_member {
        MenuAction::Run(client.map_or_else(
            || guarded_jump_id_command(&session.id, &session.main_pane),
            |client| guarded_jump_client_id_command(client, &session.id, &session.main_pane),
        ))
    } else {
        MenuAction::Run(client.map_or_else(
            || switch_id_command(&session.id),
            |client| switch_client_id_command(client, &session.id),
        ))
    };
    MenuItem {
        label,
        key: String::new(),
        action,
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
        Args, KEYS, ROW_CAP, Usage, launch_tail_is_valid, menu, menu_for_client, parse,
        parse_launch_tail,
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
            labels(&menu(&[shown], &[], true, &Palette::DARCULA)),
            [concat!(
                "hub               ",
                " ",
                "⚠",
                " ",
                "needs-you",
                " ",
                "feat/menu     ",
                " ",
                "ship it"
            )]
        );
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
        let drawn = menu_for_client(
            &sessions,
            &panes,
            true,
            &Palette::DARCULA,
            Some("/dev/ttys007"),
        );
        let MenuAction::Run(command) = &drawn.items[0].action else {
            panic!("the session row runs one command");
        };
        assert_eq!(
            command,
            "switch-client -c '/dev/ttys007' -t $7 ; if-shell -F -t %12 '##{==:##{session_id},$7}' 'select-window -t %12 ; select-pane -t %12'"
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
        assert_eq!(drawn.items.len(), ROW_CAP + 1);
        let last = drawn.items.last().expect("overflow note");
        assert_eq!(last.label, "… 7 more — see ae list");
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
    fn display_argv_targets_the_explicit_client_and_has_one_triplet_per_row() {
        let drawn = menu_for_client(
            &[session("hub", "$1", 0, "")],
            &[],
            true,
            &Palette::DARCULA,
            Some("client"),
        );
        let words = display_menu_for_client_args(&ServerId::Ambient, Some("client"), &drawn, true);
        assert_eq!(&words[..5], ["display-menu", "-M", "-O", "-c", "client"]);
        let keyboard =
            display_menu_for_client_args(&ServerId::Ambient, Some("client"), &drawn, false);
        assert_eq!(&keyboard[..4], ["display-menu", "-O", "-c", "client"]);
        assert!(!keyboard.iter().any(|word| word == "-M"));
        assert_eq!(words.iter().filter(|word| word.as_str() == "--").count(), 1);
        assert!(
            words
                .last()
                .is_some_and(|word| word == "switch-client -c 'client' -t $1")
        );
    }
}
