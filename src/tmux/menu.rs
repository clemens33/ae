//! Client UI commands: display-message, confirm-before, menus, hotkeys, mouse binds, and the jump and focus commands.

use super::{DISPLAY_MESSAGE_MS, ServerId, server_args, session_target};

#[cfg(doc)]
use super::format_literal;

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

/// Show a transient message on one attached client, without needing a pane
/// that may have disappeared with the session which produced the message.
#[must_use]
pub fn display_client_message_args(server: &ServerId, client: &str, text: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(
        [
            "display-message",
            "-d",
            DISPLAY_MESSAGE_MS,
            "-c",
            client,
            text,
        ]
        .map(ToOwned::to_owned),
    );
    args
}

/// Ask one attached client to authorize `continuation`.
#[must_use]
pub fn confirm_before_args(
    server: &ServerId,
    client: &str,
    prompt: &str,
    continuation: &str,
) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(
        ["confirm-before", "-p", prompt, "-t", client, continuation].map(ToOwned::to_owned),
    );
    args
}

/// A `run-shell -b` tmux command which re-execs `argv` without allowing any
/// argv element to become shell syntax.
///
/// There are two parsers. [`crate::words::shell_quote`] protects each word
/// from the shell which `run-shell` starts; [`tmux_double_quote`] then carries
/// that shell command through tmux's deferred command parser. Tmux removes a
/// backslash before any character inside double quotes, so the second layer
/// must double the backslashes the first layer minted for a literal quote.
#[must_use]
pub fn run_shell_background_command(argv: &[String]) -> String {
    let shell = argv
        .iter()
        .map(|word| crate::words::shell_quote(word))
        .collect::<Vec<_>>()
        .join(" ");
    format!("run-shell -b {}", tmux_double_quote(&shell))
}

/// A background picker launch whose last word is the client that clicked.
///
/// Fixed words cross both the outer mouse-dispatch format and the nested
/// `run-shell` format. The one unescaped format is deliberate: tmux's `q`
/// modifier makes the expanded client name one safe shell word.
#[must_use]
pub(crate) fn status_picker_command(launcher: &[String]) -> String {
    let mut argv = launcher.to_vec();
    argv.extend(["orchestrator", "--popup", "--client"].map(ToOwned::to_owned));
    let shell = argv
        .iter()
        .map(|word| mouse_dispatch_literal(&crate::words::shell_quote(word)))
        .chain(std::iter::once("#{q:client_name}".to_owned()))
        .collect::<Vec<_>>()
        .join(" ");
    format!("run-shell -b {}", tmux_current_format_double_quote(&shell))
}

/// A background settings launch whose last word is the exact invoking client.
#[must_use]
pub(crate) fn status_settings_command(launcher: &[String]) -> String {
    let mut argv = launcher.to_vec();
    argv.extend(["orchestrator", "--settings", "--client"].map(ToOwned::to_owned));
    let shell = argv
        .iter()
        .map(|word| mouse_dispatch_literal(&crate::words::shell_quote(word)))
        .chain(std::iter::once("#{q:client_name}".to_owned()))
        .collect::<Vec<_>>()
        .join(" ");
    format!("run-shell -b {}", tmux_current_format_double_quote(&shell))
}

/// The shell command run by the server-global `prefix a` picker binding.
///
/// Unlike [`status_picker_command`], this crosses no format conditional: the
/// fixed words need one hash escape for `run-shell`, while commas and closing
/// braces remain literal. The client format is deliberately left live.
#[must_use]
pub(crate) fn hotkey_picker_shell(launcher: &[String]) -> String {
    let mut argv = launcher.to_vec();
    argv.extend(["orchestrator", "--popup", "--client"].map(ToOwned::to_owned));
    argv.iter()
        .map(|word| menu_literal(&crate::words::shell_quote(word)))
        .chain(std::iter::once("#{q:client_name}".to_owned()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The shell command run by the server-global `prefix v` reader binding.
///
/// The SAME launcher boundary and quoting as [`hotkey_picker_shell`]: the
/// installed shape names `~/.local/bin/ae`, a checkout its core plus state,
/// config and server words, and the client format stays live so the verb
/// resolves the client that pressed the key.
#[must_use]
pub(crate) fn hotkey_reader_shell(launcher: &[String]) -> String {
    hotkey_verb_shell(launcher, &[crate::cli::READER, "--client"])
}

/// The console keys' shell (`prefix h`, and `prefix H` with `--jump`), on the
/// same launcher boundary as the reader hotkey.
#[must_use]
pub(crate) fn hotkey_console_shell(launcher: &[String], jump: bool) -> String {
    let mut verb = vec![crate::cli::CONSOLE_TOGGLE];
    verb.extend(jump.then_some("--jump"));
    verb.push("--client");
    hotkey_verb_shell(launcher, &verb)
}

/// The `prefix O` orchestrator jump's shell, on the same launcher boundary:
/// every fact of the press is a live format, so the verb reads the client, the
/// server and the view the key was pressed in, never a guess made later. The
/// ids are single-quoted because a session id's `$` is shell syntax.
#[must_use]
pub(crate) fn hotkey_jump_shell(launcher: &[String]) -> String {
    let fixed = hotkey_verb_shell(launcher, &[crate::cli::ORCHESTRATOR_JUMP, "--client"]);
    format!(
        "{fixed} --client-pid '#{{client_pid}}' --server-pid '#{{pid}}' \
         --server-start '#{{start_time}}' --from '#{{session_id}}' '#{{window_id}}' '#{{pane_id}}'"
    )
}

fn hotkey_verb_shell(launcher: &[String], verb: &[&str]) -> String {
    let mut argv = launcher.to_vec();
    argv.extend(verb.iter().map(|word| (*word).to_owned()));
    argv.iter()
        .map(|word| menu_literal(&crate::words::shell_quote(word)))
        .chain(std::iter::once("#{q:client_name}".to_owned()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Carry literal text through the mouse dispatch and its nested `run-shell`.
///
/// Each hash crosses two format expansions. Commas and closing braces need one
/// escape because the outer expansion embeds the text in a format conditional.
pub(crate) fn mouse_dispatch_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for ch in text.chars() {
        match ch {
            '#' => out.push_str("####"),
            ',' => out.push_str("#,"),
            '}' => out.push_str("#}"),
            _ => out.push(ch),
        }
    }
    out
}

/// One double-quoted argument in the current tmux command language.
///
/// Hashes remain untouched: callers deliberately mix layer-counted literals
/// with formats which the outer mouse dispatch must expand.
pub(crate) fn tmux_current_format_double_quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// One argument in tmux's deferred command language.
pub(crate) fn tmux_double_quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            // `run-shell` format-expands its shell command. `##` is one
            // literal hash at that second boundary.
            '#' => out.push_str("##"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
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
#[derive(Debug)]
pub enum MenuAction {
    /// Run this tmux command — built from ids this crate validated, never from
    /// text a session named itself.
    Run(String),
    /// Nothing: the row is drawn dim and cannot be chosen.
    Disabled,
    /// A selectable no-op drawn in this style: ae's own `#[…]` directive,
    /// emitted BEFORE the escaped label the way the title's is.
    Styled(String),
}

/// One row of a tmux menu.
#[derive(Debug)]
pub struct MenuItem {
    /// The visible text, unescaped — [`display_menu_args`] escapes it.
    pub label: String,
    /// The single-key shortcut, or empty for none.
    pub key: String,
    /// What choosing the row does.
    pub action: MenuAction,
}

/// A menu ae asks tmux to draw.
#[derive(Debug)]
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

/// Where the fleet picker is drawn — at client column zero, immediately above
/// the status line and its bottom-left button.
///
/// tmux 3.4 expands a numeric `-x` directly as the menu's bottom-left
/// position: <https://github.com/tmux/tmux/blob/3.4/cmd-display-menu.c#L214-L233>.
pub(super) const MENU_POSITION: [&str; 4] = ["-x", "0", "-y", "S"];

/// tmux's marker for a row that is drawn dim and cannot be chosen.
const DISABLED_PREFIX: char = '-';

/// End of flags, before the first item.
///
/// Measured on tmux 3.7b: `display-menu` reads its flags with getopt, so a
/// FIRST item whose name carries the [`DISABLED_PREFIX`] is read as a flag and
/// the whole call fails with `unknown flag -g`. The separator ends the flags
/// before any name is read, whatever the rows turn out to be.
const END_OF_FLAGS: &str = "--";

/// Where a session's context menu is drawn — centred on the invoking client's
/// WHOLE terminal, not on its active pane or the mouse.
///
/// tmux 3.4 reads `C` from the client's own `tty->sx`/`tty->sy` and clamps the
/// result, so a split client centres on the terminal it owns:
/// <https://github.com/tmux/tmux/blob/3.4/cmd-display-menu.c#L165-L244>.
const CENTRE_POSITION: [&str; 4] = ["-x", "C", "-y", "C"];

/// The exact invoking client's absolute left coordinate for settings.
///
/// The final menu budget and the client snapshot decide both fit and this
/// position. tmux still clamps a concurrent resize, but a target pane never
/// gets to move this client-relative coordinate.
#[must_use]
pub(crate) const fn settings_menu_x(client_width: usize, menu_columns: usize) -> usize {
    client_width.saturating_sub(menu_columns)
}

/// The arguments that draw `menu` on `server`'s current client.
#[must_use]
pub fn display_menu_args(server: &ServerId, menu: &Menu, menu_mouse: bool) -> Vec<String> {
    display_menu_for_client_args(server, None, menu, menu_mouse)
}

/// The arguments that draw `menu` CENTRED on one explicit client, with one
/// explicit target pane supplying the command context every row inherits.
///
/// The fleet picker keeps its bottom-left button position; a session's context
/// menu is a modal answer to a click on that session's own status range, so it
/// is drawn in the middle of the terminal that asked for it.
#[must_use]
pub fn display_menu_centred_args(
    server: &ServerId,
    client: &str,
    target: &str,
    menu: &Menu,
    menu_mouse: bool,
) -> Vec<String> {
    display_targeted_menu_args(server, client, target, menu, menu_mouse, &CENTRE_POSITION)
}

/// The arguments that draw settings at one explicit client's numeric
/// bottom-right coordinate, with one explicit target pane supplying every
/// row's command context.
#[must_use]
pub fn display_settings_menu_args(
    server: &ServerId,
    client: &str,
    target: &str,
    x: usize,
    menu: &Menu,
    menu_mouse: bool,
) -> Vec<String> {
    let x = x.to_string();
    display_targeted_menu_args(
        server,
        client,
        target,
        menu,
        menu_mouse,
        &["-x", &x, "-y", "S"],
    )
}

fn display_targeted_menu_args(
    server: &ServerId,
    client: &str,
    target: &str,
    menu: &Menu,
    menu_mouse: bool,
    position: &[&str; 4],
) -> Vec<String> {
    let mut args = server_args(server);
    args.push("display-menu".to_owned());
    if menu_mouse {
        args.push("-M".to_owned());
    }
    args.push("-O".to_owned());
    args.extend(["-c".to_owned(), client.to_owned()]);
    args.extend(["-t".to_owned(), target.to_owned()]);
    args.extend(position.iter().map(|word| (*word).to_owned()));
    args.push("-T".to_owned());
    args.push(titled(menu));
    args.push(END_OF_FLAGS.to_owned());
    for item in &menu.items {
        args.extend(item_words(item));
    }
    args
}

/// A `run-shell -b` re-exec of `argv` written as ONE MENU ITEM command.
///
/// Two expanders read it, in this order: `display-menu` expands an item's
/// command when the menu is built, and `run-shell` expands its shell command
/// when the chosen row runs. [`run_shell_background_command`] already answers
/// the second, so this doubles every hash once more for the first.
#[must_use]
pub fn menu_run_shell_command(argv: &[String]) -> String {
    menu_literal(&run_shell_background_command(argv))
}

/// The arguments that draw `menu` on one explicit client.
///
/// A status binding can run outside either attached client's command queue,
/// and `$TMUX` identifies only the server. Carrying `client` through this menu
/// and every action prevents tmux from choosing whichever client was most
/// recently active when two clients watch the same pane.
#[must_use]
pub fn display_menu_for_client_args(
    server: &ServerId,
    client: Option<&str>,
    menu: &Menu,
    menu_mouse: bool,
) -> Vec<String> {
    let mut args = server_args(server);
    args.push("display-menu".to_owned());
    if menu_mouse {
        args.push("-M".to_owned());
    }
    args.push("-O".to_owned());
    if let Some(client) = client {
        args.extend(["-c".to_owned(), client.to_owned()]);
    }
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
        // The leading hyphen is tmux's own dim-and-unselectable marker; the two
        // empty arguments keep the row a triplet like every other.
        MenuAction::Disabled => vec![
            format!("{DISABLED_PREFIX}{label}"),
            String::new(),
            String::new(),
        ],
        MenuAction::Styled(style) => {
            vec![format!("{style}{label}"), item.key.clone(), String::new()]
        }
    }
}

/// The `-T` argument: the style ae chose, then the title it was given.
fn titled(menu: &Menu) -> String {
    format!("{}{}", menu.title_style, menu_literal(&menu.title))
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
    format!(
        "{} -t {}",
        FocusVerb::SwitchClient.as_str(),
        session_target(session)
    )
}

/// Hand one explicit client to `session`.
#[must_use]
pub fn switch_client_command(client: &str, session: &str) -> String {
    format!(
        "{} -c {} -t {}",
        FocusVerb::SwitchClient.as_str(),
        single_quoted(client),
        session_target(session)
    )
}

/// Predicate shared by the two root status bindings.
pub(crate) const MOUSE_STATUS_PICKER: &str =
    "#{||:#{==:#{mouse_status_range},ae},#{==:#{mouse_status_range},ae-more}}";
pub(crate) const MOUSE_STATUS_SETTINGS: &str = "#{==:#{mouse_status_range},ae-settings}";
pub(crate) const MOUSE_STATUS_SESSION: &str = "#{==:#{mouse_status_range},session}";
pub(crate) const MOUSE_STATUS_WINDOW: &str = "#{==:#{mouse_status_range},window}";

/// The one action in ae's root status context menu.
///
/// `display-menu` expands item commands when the menu opens, so every format
/// hash is doubled: menu creation consumes one hash and leaves the predicate
/// for `if-shell -F` to evaluate when the item is chosen.
pub(crate) const MOUSE_DOWN_STATUS_MENU_ACTION: &str = "if-shell -F '##{&&:##{==:##{window_panes},2},##{==:##{window_zoomed_flag},0}}' 'swap-pane -d -s \"{top-left}\" -t \"{bottom-right}\"' 'display-message \"flip needs an unzoomed two-pane window\"'";

/// Arguments that switch one exact client to one exact session.
///
/// The client target is the exact id read from `list-clients`; the session
/// target uses tmux's leading `=` exact-match form.
#[must_use]
pub fn switch_client_args(server: &ServerId, client: &str, session: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["switch-client", "-c"].map(ToOwned::to_owned));
    args.push(client.to_owned());
    args.push("-t".to_owned());
    args.push(session_target(session));
    args
}

/// The arguments that run `words` as one background shell job on `server`: the
/// words are shell-quoted into the job's single command string.
#[must_use]
pub fn run_shell_background_args(server: &ServerId, words: &[String]) -> Vec<String> {
    let mut line = server_args(server);
    line.extend(["run-shell", "-b"].map(ToOwned::to_owned));
    line.push(
        (words.iter())
            .map(|word| crate::words::shell_quote(word))
            .collect::<Vec<_>>()
            .join(" "),
    );
    line
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

/// Hand one explicit client to `pane` of `session`.
#[must_use]
pub fn jump_client_command(client: &str, session: &str, pane: &str) -> String {
    format!(
        "{} ; select-window -t {pane} ; select-pane -t {pane}",
        switch_client_command(client, session)
    )
}

/// Hand the calling client to a captured session id, then select `pane` only
/// if it STILL belongs to that session when the menu row runs.
///
/// `display-menu` consumes one format-expansion layer when it opens, so the
/// predicate's hashes are doubled here and survive until `if-shell` evaluates
/// them after the switch. A pane that vanished or moved makes only the guarded
/// tail fail; the client has already landed in the intended session.
#[must_use]
pub fn guarded_jump_id_command(session_id: &str, pane: &str) -> String {
    guarded_jump(&switch_id_command(session_id), session_id, pane)
}

/// The explicit-client form of [`guarded_jump_id_command`].
#[must_use]
pub fn guarded_jump_client_id_command(client: &str, session_id: &str, pane: &str) -> String {
    guarded_jump(
        &switch_client_id_command(client, session_id),
        session_id,
        pane,
    )
}

/// [`guarded_jump_id_command`] for a seat's own pane. The switch fires the
/// session's `client-session-changed` hook, which tmux runs only after this
/// command's queue pass; the guarded select waits in `run-shell -C` for the
/// next event-loop turn, so it lands after the hook instead of under it.
#[must_use]
pub fn seat_jump_id_command(session_id: &str, pane: &str) -> String {
    seat_jump(&switch_id_command(session_id), session_id, pane)
}

/// The explicit-client form of [`seat_jump_id_command`].
#[must_use]
pub fn seat_jump_client_id_command(client: &str, session_id: &str, pane: &str) -> String {
    seat_jump(
        &switch_client_id_command(client, session_id),
        session_id,
        pane,
    )
}

fn guarded_jump(switch: &str, session_id: &str, pane: &str) -> String {
    format!("{switch} ; {}", guarded_select(session_id, pane))
}

fn seat_jump(switch: &str, session_id: &str, pane: &str) -> String {
    format!(
        "{switch} ; run-shell -C {{ {} }}",
        guarded_select(session_id, pane)
    )
}

fn guarded_select(session_id: &str, pane: &str) -> String {
    format!(
        "if-shell -F -t {pane} '##{{==:##{{session_id}},{session_id}}}' \
         'select-window -t {pane} ; select-pane -t {pane}'"
    )
}

/// Hand the calling client to a captured tmux `$<n>` session id.
#[must_use]
pub fn switch_id_command(session_id: &str) -> String {
    format!("{} -t {session_id}", FocusVerb::SwitchClient.as_str())
}

/// Hand one explicit client to a captured tmux `$<n>` session id.
#[must_use]
pub fn switch_client_id_command(client: &str, session_id: &str) -> String {
    format!(
        "{} -c {} -t {session_id}",
        FocusVerb::SwitchClient.as_str(),
        single_quoted(client)
    )
}

/// The arguments that focus `session` with `verb`.
#[must_use]
pub fn focus_args(server: &ServerId, verb: FocusVerb, session: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.push(verb.as_str().to_owned());
    args.push("-t".to_owned());
    args.push(session_target(session));
    args
}

/// Arguments that attach to `server` and let tmux choose its most recently
/// used session.
#[must_use]
pub fn attach_args(server: &ServerId) -> Vec<String> {
    let mut args = server_args(server);
    args.push(FocusVerb::AttachSession.as_str().to_owned());
    args
}
