//! The launch operation's tmux argv — the sealed builder behind
//! [`crate::transport::run_tmux_op`].
//!
//! The tmux calls the launch path makes: `new-session`, `set-environment`,
//! `split-window`, `new-window`,
//! `respawn-pane`, `select-layout`, `select-pane`, `select-window`, `set-hook`,
//! `bind-key`, `unbind-key`, `set-window-option`.
//!
//! Same shape as [`crate::git`] and for the same reason: the inner vector of
//! [`TmuxArgv`] is private to this module, so no other module can hand the
//! process door an arbitrary tmux command line. Every operation is a variant
//! here, and the variants are the whole surface.
//!
//! Options that a typed builder already covers — pane/window/session
//! `set-option` — are NOT re-spelled here: they go through
//! [`crate::transport::publish_option`], which is the existing door.

use std::path::Path;

use crate::inventory::ServerId;
use crate::meta::Selector;
use crate::tmux::{
    MOUSE_DOWN_STATUS_MENU_ACTION, MOUSE_STATUS_PICKER, MOUSE_STATUS_SESSION,
    MOUSE_STATUS_SETTINGS, MOUSE_STATUS_WINDOW, hotkey_picker_shell, mouse_dispatch_literal,
    server_args, session_target, status_picker_command, status_settings_command,
    tmux_current_format_double_quote,
};

/// The `-P -F` format every pane-creating call here prints.
const PANE_ID_FORMAT: &str = "#{pane_id}";
const STOCK_RIGHT_CLICK_MENU_KEYS: [&str; 5] = [
    "MouseDown3Pane",
    "M-MouseDown3Pane",
    "MouseDown3StatusLeft",
    "M-MouseDown3Status",
    "M-MouseDown3StatusLeft",
];

/// The main layout must never collapse a zoomed pane back into its window.
const LEAD_PAIR_NOT_ZOOMED: &str = "#{==:#{window_zoomed_flag},0}";

fn lead_pair_layout_command(pane: &str) -> String {
    format!("select-layout -t {pane} main-vertical")
}

fn lead_pair_layout_if_unzoomed_command(pane: &str) -> String {
    format!(
        "if-shell -F -t {pane} '{LEAD_PAIR_NOT_ZOOMED}' '{}'",
        lead_pair_layout_command(pane)
    )
}

/// A tmux argv minted ONLY by this module's [`argv`] builder.
pub(crate) struct TmuxArgv(Vec<String>);

impl TmuxArgv {
    /// The argv for the transport door to spawn.
    pub(crate) fn as_args(&self) -> &[String] {
        &self.0
    }
}

/// Where a split goes, relative to its target pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Split {
    /// `-h` — side by side.
    Horizontal,
    /// `-v` — stacked.
    Vertical,
    /// `-v -b` — stacked, the new pane ABOVE.
    VerticalBefore,
}

/// One tmux command the launch operation runs.
pub(crate) enum Op<'a> {
    /// `new-session -d -s <name> -c <dir> -P -F '#{pane_id}'` — the session and
    /// its first pane, in one call, printing the pane id rather than asking for
    /// it afterwards.
    NewSession {
        /// The session name.
        name: &'a str,
        /// The first pane's working directory.
        work_dir: &'a str,
    },
    /// `new-session -d -s <name> -c <dir> <command…>` — a DAEMON's own session,
    /// which is not the launch's shape: no `-P -F`, because nothing reads a
    /// pane id back from it, and a command, because the session exists only to
    /// hold that process.
    NewDaemonSession {
        /// The session name — a constant here, never operator input.
        name: &'a str,
        /// The daemon's working directory.
        work_dir: &'a str,
        /// The command the session's one pane runs.
        command: &'a [String],
    },
    /// `set-environment -t <session> <key> <value>`.
    SetEnv {
        session: &'a str,
        key: &'a str,
        value: &'a str,
    },
    /// `set-environment -t <session> -u <key>` — the two Claude Code variables
    /// that stop it starting inside tmux.
    UnsetEnv { session: &'a str, key: &'a str },
    /// `split-window <dir> -t <target> -c <dir> -P -F '#{pane_id}' [command]`.
    SplitWindow {
        target: &'a str,
        work_dir: &'a str,
        split: Split,
        /// The command the new pane runs, or empty for a shell.
        command: &'a [String],
    },
    /// `new-window -d -t <target> [-n <name>] -c <dir> -P -F '#{pane_id}' [command]`.
    NewWindow {
        /// `<session>:` for "next free index", `<session>:99` for the pinned
        /// monitor window.
        target: &'a str,
        /// The window name, or empty for tmux's default.
        name: &'a str,
        /// The working directory, or empty to inherit.
        work_dir: &'a str,
        /// The command the new pane runs, or empty for a shell.
        command: &'a [String],
    },
    /// `respawn-pane -k -t <pane> <command…>` — replace a monitor process in
    /// place, preserving its pane id and the monitor window's layout.
    RespawnPane {
        pane: &'a str,
        command: &'a [String],
    },
    /// `select-layout -t <target> <layout>`.
    SelectLayout { target: &'a str, layout: &'a str },
    /// `set-window-option -t <target> main-pane-width 50%` — the lead-pair
    /// window's persistent equal main columns.
    SetLeadPairWidth { target: &'a str },
    /// Apply `main-vertical` to the lead-pair window unless one of its panes
    /// is zoomed.
    SelectLeadPairLayout { pane: &'a str },
    /// `select-pane -t <pane>` — focus.
    SelectPane { pane: &'a str },
    /// `select-pane -t <pane> -d` — make a monitor pane read-only.
    DisablePane { pane: &'a str },
    /// `select-window -t <pane>` — the `focus` helper's window switch, which
    /// `select-pane` alone does not do.
    SelectWindow { pane: &'a str },
    /// `set-hook -t <pane> client-session-changed <guarded focus command>` —
    /// keep a client's view on the lead pane whenever it enters this pane's
    /// session, while the pane still belongs to the captured session id.
    /// `set-hook` takes a target-PANE, so the pane id makes this session-scoped
    /// without a name target (and without prefix matching).
    SetClientSessionHook { session_id: &'a str, pane: &'a str },
    /// Remove the session-scoped focus hook when its pane or session identity
    /// cannot be proven.
    UnsetClientSessionHook { target: &'a str },
    /// `set-hook -w -t <pane> window-resized <layout command>` — keep the
    /// lead-pair ratio when its window follows a client resize.
    SetLeadPairResizeHook { pane: &'a str },
    /// Replace tmux's root `MouseDown1Status` on an ae-owned server so a
    /// window-range click selects the window without firing the session hook.
    BindMouseDownStatus { picker: &'a str, settings: &'a str },
    /// Bind ae's root `MouseDown3Status` context menu on an ae-owned server.
    BindMouseDownStatusMenu {
        picker: &'a str,
        settings: &'a str,
        /// The words a menu row re-execs ae with.
        launcher: &'a [String],
        menu_mouse: bool,
        menu: bool,
    },
    /// Bind the keyboard-driven picker's root `MouseUp1Status` release.
    BindMouseUpStatus { picker: &'a str, settings: &'a str },
    /// Bind the keyboard-driven picker's root `MouseUp3Status` release.
    BindMouseUpStatusMenu {
        picker: &'a str,
        settings: &'a str,
        /// The words a menu row re-execs ae with.
        launcher: &'a [String],
        menu_mouse: bool,
    },
    /// Remove one stale root binding when the server capability changes.
    UnbindRootKey { key: &'a str },
    /// Bind the fleet picker to mnemonic `prefix a` on an ae-owned server.
    BindPickerHotkey { shell: &'a str },
    /// `rename-session -t <target> <name>` — `ae rename`'s tmux half.
    RenameSession { target: &'a str, name: &'a str },
    /// `set-window-option -t <target> <name> <value>` — the monitor window's
    /// `pane-border-status`.
    SetWindowOption {
        target: &'a str,
        name: &'a str,
        value: &'a str,
    },
    /// `capture-pane -p -J -S -<lines> -E - -t <pane>` — the `peek` helper.
    CapturePane { pane: &'a str, lines: u32 },
}

/// Build the argv for one operation, server selector first.
#[allow(
    clippy::too_many_lines,
    reason = "the tmux argv table: one match arm per operation, kept as one readable dispatch rather than fragmented into per-op builders"
)]
pub(crate) fn argv(server: &ServerId, op: &Op<'_>) -> TmuxArgv {
    let mut args = server_args(server);
    match *op {
        Op::NewSession { name, work_dir } => {
            args.extend(
                [
                    "new-session",
                    "-d",
                    "-s",
                    name,
                    "-c",
                    work_dir,
                    "-P",
                    "-F",
                    PANE_ID_FORMAT,
                ]
                .map(ToOwned::to_owned),
            );
        }
        Op::NewDaemonSession {
            name,
            work_dir,
            command,
        } => {
            args.extend(["new-session", "-d", "-s", name].map(ToOwned::to_owned));
            if !work_dir.is_empty() {
                args.extend(["-c", work_dir].map(ToOwned::to_owned));
            }
            args.extend(command.iter().cloned());
        }
        Op::SetEnv {
            session,
            key,
            value,
        } => {
            args.extend(["set-environment", "-t"].map(ToOwned::to_owned));
            args.push(session_target(session));
            args.extend([key, value].map(ToOwned::to_owned));
        }
        Op::UnsetEnv { session, key } => {
            args.extend(["set-environment", "-t"].map(ToOwned::to_owned));
            args.push(session_target(session));
            args.extend(["-u", key].map(ToOwned::to_owned));
        }
        Op::SplitWindow {
            target,
            work_dir,
            split,
            command,
        } => {
            args.push("split-window".to_owned());
            match split {
                Split::Horizontal => args.push("-h".to_owned()),
                Split::Vertical => args.push("-v".to_owned()),
                Split::VerticalBefore => {
                    args.push("-v".to_owned());
                    args.push("-b".to_owned());
                }
            }
            args.extend(["-t", target].map(ToOwned::to_owned));
            if !work_dir.is_empty() {
                args.extend(["-c", work_dir].map(ToOwned::to_owned));
            }
            args.extend(["-P", "-F", PANE_ID_FORMAT].map(ToOwned::to_owned));
            args.extend(command.iter().cloned());
        }
        Op::NewWindow {
            target,
            name,
            work_dir,
            command,
        } => {
            args.extend(["new-window", "-d", "-t", target].map(ToOwned::to_owned));
            if !name.is_empty() {
                args.extend(["-n", name].map(ToOwned::to_owned));
            }
            if !work_dir.is_empty() {
                args.extend(["-c", work_dir].map(ToOwned::to_owned));
            }
            args.extend(["-P", "-F", PANE_ID_FORMAT].map(ToOwned::to_owned));
            args.extend(command.iter().cloned());
        }
        Op::RespawnPane { pane, command } => {
            args.extend(["respawn-pane", "-k", "-t", pane].map(ToOwned::to_owned));
            args.extend(command.iter().cloned());
        }
        Op::SelectLayout { target, layout } => {
            args.extend(["select-layout", "-t", target, layout].map(ToOwned::to_owned));
        }
        Op::SetLeadPairWidth { target } => {
            args.extend(
                ["set-window-option", "-t", target, "main-pane-width", "50%"]
                    .map(ToOwned::to_owned),
            );
        }
        Op::SelectLeadPairLayout { pane } => {
            args.extend(
                [
                    "if-shell",
                    "-F",
                    "-t",
                    pane,
                    LEAD_PAIR_NOT_ZOOMED,
                    &lead_pair_layout_command(pane),
                ]
                .map(ToOwned::to_owned),
            );
        }
        Op::SelectPane { pane } => {
            args.extend(["select-pane", "-t", pane].map(ToOwned::to_owned));
        }
        Op::DisablePane { pane } => {
            args.extend(["select-pane", "-t", pane, "-d"].map(ToOwned::to_owned));
        }
        Op::SelectWindow { pane } => {
            args.extend(["select-window", "-t", pane].map(ToOwned::to_owned));
        }
        Op::SetClientSessionHook { session_id, pane } => {
            args.extend(
                [
                    "set-hook",
                    "-t",
                    pane,
                    "client-session-changed",
                    &format!(
                        "if-shell -F -t {pane} \"#{{==:#{{session_id}},{session_id}}}\" \
                         \"select-window -t {pane} ; select-pane -t {pane}\""
                    ),
                ]
                .map(ToOwned::to_owned),
            );
        }
        Op::UnsetClientSessionHook { target } => {
            args.extend(
                ["set-hook", "-u", "-t", target, "client-session-changed"].map(ToOwned::to_owned),
            );
        }
        Op::SetLeadPairResizeHook { pane } => {
            args.extend(
                [
                    "set-hook",
                    "-w",
                    "-t",
                    pane,
                    "window-resized",
                    &lead_pair_layout_if_unzoomed_command(pane),
                ]
                .map(ToOwned::to_owned),
            );
        }
        Op::BindMouseDownStatus { picker, settings } => {
            args.extend(["bind-key", "-T", "root", "MouseDown1Status"].map(ToOwned::to_owned));
            args.extend(left_click_dispatch(picker, settings));
        }
        Op::BindMouseDownStatusMenu {
            picker,
            settings,
            launcher,
            menu_mouse,
            menu,
        } => {
            args.extend(["bind-key", "-T", "root", "MouseDown3Status"].map(ToOwned::to_owned));
            args.extend(right_click_dispatch(
                server, picker, settings, launcher, menu_mouse, menu,
            ));
        }
        Op::BindMouseUpStatus { picker, settings } => {
            args.extend(["bind-key", "-T", "root", "MouseUp1Status"].map(ToOwned::to_owned));
            args.extend(picker_click_dispatch(picker, settings));
        }
        Op::BindMouseUpStatusMenu {
            picker,
            settings,
            launcher,
            menu_mouse,
        } => {
            args.extend(["bind-key", "-T", "root", "MouseUp3Status"].map(ToOwned::to_owned));
            args.extend(right_click_dispatch(
                server, picker, settings, launcher, menu_mouse, true,
            ));
        }
        Op::UnbindRootKey { key } => {
            args.extend(["unbind-key", "-T", "root", key].map(ToOwned::to_owned));
        }
        Op::BindPickerHotkey { shell } => {
            args.extend(
                ["bind-key", "-T", "prefix", "a", "run-shell", "-b", shell].map(ToOwned::to_owned),
            );
        }
        Op::RenameSession { target, name } => {
            args.extend(["rename-session", "-t"].map(ToOwned::to_owned));
            args.push(session_target(target));
            args.push(name.to_owned());
        }
        Op::SetWindowOption {
            target,
            name,
            value,
        } => {
            args.extend(["set-window-option", "-t", target, name, value].map(ToOwned::to_owned));
        }
        Op::CapturePane { pane, lines } => {
            args.extend(["capture-pane", "-p", "-J", "-S"].map(ToOwned::to_owned));
            args.push(format!("-{lines}"));
            args.extend(["-E", "-", "-t", pane].map(ToOwned::to_owned));
        }
    }
    TmuxArgv(args)
}

fn format_if(predicate: &str, yes: &str, no: &str) -> String {
    format!("#{{?{predicate},{yes},{no}}}")
}

fn mouse_dispatch(command: String) -> Vec<String> {
    ["run-shell", "-C", "-t", "{mouse}"]
        .map(ToOwned::to_owned)
        .into_iter()
        .chain(std::iter::once(command))
        .collect()
}

fn left_click_dispatch(picker: &str, settings: &str) -> Vec<String> {
    let session = format_if(
        MOUSE_STATUS_SESSION,
        "switch-client -c #{q:client_name} -t #{session_id}",
        "",
    );
    let window = format_if(
        MOUSE_STATUS_WINDOW,
        "select-window -t #{window_id}",
        &session,
    );
    let picker = format_if(MOUSE_STATUS_PICKER, picker, &window);
    mouse_dispatch(format_if(MOUSE_STATUS_SETTINGS, settings, &picker))
}

fn picker_click_dispatch(picker: &str, settings: &str) -> Vec<String> {
    let picker = format_if(MOUSE_STATUS_PICKER, picker, "");
    mouse_dispatch(format_if(MOUSE_STATUS_SETTINGS, settings, &picker))
}

fn fixed_mouse_shell_word(word: &str) -> String {
    mouse_dispatch_literal(&crate::launch::shell_quote(word))
}

/// The clicked session's facts, captured by the mouse dispatch that draws its
/// menu. Each is spliced in AFTER every escaping layer, so the format itself
/// crosses no escape and the value it expands to crosses no format.
const CAPTURED: [(&str, &str); 7] = [
    ("AEMENUCLIENTNAME", "#{client_name}"),
    ("AEMENUCLIENTPID", "#{client_pid}"),
    ("AEMENUSESSIONNAME", "#{session_name}"),
    ("AEMENUSESSIONID", "#{session_id}"),
    ("AEMENUPANEID", "#{pane_id}"),
    ("AEMENUSERVERPID", "#{pid}"),
    ("AEMENUSERVERSTART", "#{start_time}"),
];

/// Replace every capture placeholder with the tmux format it stands for.
fn splice_captured(text: &str) -> String {
    let mut out = text.to_owned();
    for (placeholder, format) in CAPTURED {
        out = out.replace(placeholder, format);
    }
    out
}

/// One context-menu ROW that re-execs ae with the click's captured facts.
///
/// The row is written with placeholders, escaped for every layer between the
/// binding and the shell the chosen row starts, and only then given its
/// formats: the placeholders are plain letters, so no escaper touches them and
/// no format is escaped.
fn session_menu_row_command(launcher: &[String], step: &str, action: &str) -> String {
    let mut argv: Vec<String> = launcher.to_vec();
    argv.extend([crate::cli::SESSION_MENU, step, "--action", action].map(ToOwned::to_owned));
    for (flag, (placeholder, _)) in [
        "--client",
        "--client-pid",
        "--session",
        "--session-id",
        "--pane",
        "--server-pid",
        "--server-start",
    ]
    .into_iter()
    .zip(CAPTURED)
    {
        argv.push(flag.to_owned());
        argv.push(placeholder.to_owned());
    }
    // The shell the ROW starts, then the three expanders above it.
    let shell = argv
        .iter()
        .map(|word| crate::launch::shell_quote(word))
        .collect::<Vec<_>>()
        .join(" ");
    let item = crate::tmux::menu_literal(&format!(
        "run-shell -b {}",
        crate::tmux::tmux_double_quote(&shell)
    ));
    splice_captured(&mouse_dispatch_literal(&crate::launch::shell_quote(&item)))
}

/// The centred context menu a right-click on a session's status range opens.
///
/// `-x C -y C` is the invoking CLIENT's terminal centre, never the active pane
/// or the mouse. Drawing it writes nothing: the Flip row keeps its own
/// deferred guard, and the Stop row only asks ae to prepare a confirmation.
fn session_menu_command(
    server: &ServerId,
    launcher: &[String],
    action: &str,
    menu_mouse: bool,
) -> String {
    let mut shell_words = vec![fixed_mouse_shell_word("tmux")];
    shell_words.extend(
        server_args(server)
            .iter()
            .map(|word| fixed_mouse_shell_word(word)),
    );
    shell_words.push(fixed_mouse_shell_word("display-menu"));
    if menu_mouse {
        shell_words.push(fixed_mouse_shell_word("-M"));
    }
    shell_words.extend(["-O", "-c"].map(fixed_mouse_shell_word));
    shell_words.push("#{q:client_name}".to_owned());
    shell_words.push(fixed_mouse_shell_word("-t"));
    shell_words.push(crate::launch::shell_quote("#{pane_id}"));
    shell_words.push(fixed_mouse_shell_word("-T"));
    shell_words.push(crate::launch::shell_quote("#{session_name}"));
    shell_words.extend(["-x", "C", "-y", "C"].map(fixed_mouse_shell_word));
    shell_words.extend(["Flip lead/colead panes", "f", action].map(fixed_mouse_shell_word));
    if !launcher.is_empty() {
        shell_words.extend([crate::session_menu::STOP_ROW_LABEL, "s"].map(fixed_mouse_shell_word));
        shell_words.push(session_menu_row_command(
            launcher,
            crate::session_menu::CONFIRM,
            crate::session_menu::STOP,
        ));
    }
    format!(
        "run-shell -b {}",
        tmux_current_format_double_quote(&shell_words.join(" "))
    )
}

fn right_click_dispatch(
    server: &ServerId,
    picker: &str,
    settings: &str,
    launcher: &[String],
    menu_mouse: bool,
    menu_enabled: bool,
) -> Vec<String> {
    let menu = if menu_enabled {
        session_menu_command(server, launcher, MOUSE_DOWN_STATUS_MENU_ACTION, menu_mouse)
    } else {
        String::new()
    };
    let session = format_if(MOUSE_STATUS_SESSION, &menu, "");
    let picker = format_if(MOUSE_STATUS_PICKER, picker, &session);
    mouse_dispatch(format_if(MOUSE_STATUS_SETTINGS, settings, &picker))
}

/// Words run by a status binding before the picker subcommand and client.
///
/// A published core always calls the public pointer, which survives pruning.
/// A checkout bakes every namespace door into the job because tmux owns the
/// job's environment and the last ae session to assert the binding wins.
pub(crate) fn picker_launcher(
    shape: &crate::shape::Shape,
    core: &Path,
    root: &Path,
    config: &Path,
    server: &ServerId,
) -> Vec<String> {
    if let Some(link) = shape.command_link() {
        return vec![link.display().to_string()];
    }
    let mut words = vec![
        "env".to_owned(),
        format!("AE_HOME={}", root.display()),
        format!("CONFIG_FILE={}", config.display()),
    ];
    match server {
        ServerId::Selected(Selector::Name(name)) => {
            words.push(format!("AE_TMUX_SERVER={name}"));
            words.push("AE_TMUX_SERVER_KIND=name".to_owned());
        }
        ServerId::Selected(Selector::Socket(path)) => {
            words.push(format!("AE_TMUX_SERVER={}", path.display()));
            words.push("AE_TMUX_SERVER_KIND=socket".to_owned());
        }
        ServerId::Ambient => {}
    }
    words.push(core.display().to_string());
    words
}

/// The server-global status bindings for a positively selected, ae-owned
/// server. Ambient means the user's own root table, which launch never writes.
pub(crate) fn status_bindings_argv(
    server: &ServerId,
    launcher: &[String],
    menu_mouse: bool,
) -> Vec<TmuxArgv> {
    match server {
        ServerId::Ambient => Vec::new(),
        ServerId::Selected(_) => {
            let picker = status_picker_command(launcher);
            let settings = status_settings_command(launcher);
            let hotkey = hotkey_picker_shell(launcher);
            let mut bindings = if menu_mouse {
                vec![
                    argv(
                        server,
                        &Op::BindMouseDownStatus {
                            picker: &picker,
                            settings: &settings,
                        },
                    ),
                    argv(
                        server,
                        &Op::BindMouseDownStatusMenu {
                            picker: &picker,
                            settings: &settings,
                            launcher,
                            menu_mouse,
                            menu: true,
                        },
                    ),
                    argv(server, &Op::BindPickerHotkey { shell: &hotkey }),
                    argv(
                        server,
                        &Op::UnbindRootKey {
                            key: "MouseUp1Status",
                        },
                    ),
                    argv(
                        server,
                        &Op::UnbindRootKey {
                            key: "MouseUp3Status",
                        },
                    ),
                ]
            } else {
                vec![
                    argv(
                        server,
                        &Op::BindMouseDownStatus {
                            picker: "",
                            settings: "",
                        },
                    ),
                    argv(
                        server,
                        &Op::BindMouseDownStatusMenu {
                            picker: "",
                            settings: "",
                            launcher,
                            menu_mouse,
                            menu: false,
                        },
                    ),
                    argv(
                        server,
                        &Op::BindMouseUpStatus {
                            picker: &picker,
                            settings: &settings,
                        },
                    ),
                    argv(
                        server,
                        &Op::BindMouseUpStatusMenu {
                            picker: &picker,
                            settings: &settings,
                            launcher,
                            menu_mouse,
                        },
                    ),
                    argv(server, &Op::BindPickerHotkey { shell: &hotkey }),
                ]
            };
            bindings.extend(
                STOCK_RIGHT_CLICK_MENU_KEYS.map(|key| argv(server, &Op::UnbindRootKey { key })),
            );
            bindings
        }
    }
}

/// The `#{pane_id}` a `-P -F` run printed, or `None` when nothing usable came
/// back.
pub(crate) fn interpret_pane_id(succeeded: bool, stdout: &str) -> Option<String> {
    if !succeeded {
        return None;
    }
    let id = stdout.trim();
    (id.starts_with('%') && id.len() > 1).then(|| id.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(op: &Op<'_>) -> Vec<String> {
        argv(&ServerId::Ambient, op).as_args().to_vec()
    }

    #[test]
    fn a_new_session_asks_for_its_pane_id() {
        assert_eq!(
            words(&Op::NewSession {
                name: "s",
                work_dir: "/w"
            }),
            vec![
                "new-session",
                "-d",
                "-s",
                "s",
                "-c",
                "/w",
                "-P",
                "-F",
                "#{pane_id}"
            ]
        );
    }

    #[test]
    fn the_watchdog_split_is_above_its_target() {
        let cmd = vec!["/m/watchdog".to_owned()];
        assert_eq!(
            words(&Op::SplitWindow {
                target: "%9",
                work_dir: "",
                split: Split::VerticalBefore,
                command: &cmd,
            }),
            vec![
                "split-window",
                "-v",
                "-b",
                "-t",
                "%9",
                "-P",
                "-F",
                "#{pane_id}",
                "/m/watchdog"
            ]
        );
    }

    #[test]
    fn a_monitor_respawn_kills_the_old_process_in_place() {
        let cmd = vec!["/m/events-tail".to_owned()];
        assert_eq!(
            words(&Op::RespawnPane {
                pane: "%9",
                command: &cmd,
            }),
            vec!["respawn-pane", "-k", "-t", "%9", "/m/events-tail"]
        );
    }

    #[test]
    fn a_capture_window_is_the_requested_line_count() {
        assert_eq!(
            words(&Op::CapturePane {
                pane: "%1",
                lines: 120
            }),
            vec![
                "capture-pane",
                "-p",
                "-J",
                "-S",
                "-120",
                "-E",
                "-",
                "-t",
                "%1"
            ]
        );
    }

    #[test]
    fn the_lead_pair_width_is_window_scoped_and_percentage_based() {
        assert_eq!(
            words(&Op::SetLeadPairWidth { target: "%1" }),
            vec!["set-window-option", "-t", "%1", "main-pane-width", "50%"]
        );
    }

    #[test]
    fn the_lead_pair_layout_is_applied_only_while_the_window_is_not_zoomed() {
        assert_eq!(
            words(&Op::SelectLeadPairLayout { pane: "%9" }),
            vec![
                "if-shell",
                "-F",
                "-t",
                "%9",
                "#{==:#{window_zoomed_flag},0}",
                "select-layout -t %9 main-vertical"
            ]
        );
    }

    #[test]
    fn the_session_focus_hook_targets_the_lead_pane_and_keeps_its_command_one_argv_element() {
        assert_eq!(
            words(&Op::SetClientSessionHook {
                session_id: "$7",
                pane: "%9",
            }),
            vec![
                "set-hook",
                "-t",
                "%9",
                "client-session-changed",
                "if-shell -F -t %9 \"#{==:#{session_id},$7}\" \"select-window -t %9 ; select-pane -t %9\""
            ]
        );
        assert_eq!(
            words(&Op::UnsetClientSessionHook { target: "=gone:" }),
            vec!["set-hook", "-u", "-t", "=gone:", "client-session-changed"]
        );
    }

    #[test]
    fn the_lead_pair_resize_hook_is_window_scoped_and_targets_the_lead_pane() {
        assert_eq!(
            words(&Op::SetLeadPairResizeHook { pane: "%9" }),
            vec![
                "set-hook",
                "-w",
                "-t",
                "%9",
                "window-resized",
                "if-shell -F -t %9 '#{==:#{window_zoomed_flag},0}' 'select-layout -t %9 main-vertical'"
            ]
        );
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "the two exact capability argv sets are one canonical binding contract"
    )]
    fn the_status_bindings_are_only_minted_for_an_ae_owned_server() {
        let server = ServerId::Selected(crate::meta::Selector::Name("ae".to_owned()));
        let bindings = status_bindings_argv(&server, &["/opt/ae".to_owned()], true);
        assert_eq!(
            bindings.len(),
            10,
            "mouse-aware servers assert Down bindings and remove every stock right-click menu"
        );
        assert_eq!(
            bindings[0].as_args(),
            [
                "-L",
                "ae",
                "bind-key",
                "-T",
                "root",
                "MouseDown1Status",
                "run-shell",
                "-C",
                "-t",
                "{mouse}",
                "#{?#{==:#{mouse_status_range},ae-settings},run-shell -b \"'/opt/ae' 'orchestrator' '--settings' '--client' #{q:client_name}\",#{?#{||:#{==:#{mouse_status_range},ae},#{==:#{mouse_status_range},ae-more}},run-shell -b \"'/opt/ae' 'orchestrator' '--popup' '--client' #{q:client_name}\",#{?#{==:#{mouse_status_range},window},select-window -t #{window_id},#{?#{==:#{mouse_status_range},session},switch-client -c #{q:client_name} -t #{session_id},}}}}"
            ]
        );
        assert!(
            !bindings[0]
                .as_args()
                .iter()
                .any(|word| word.contains("@ae_orchestrator_id"))
        );
        assert_eq!(
            &bindings[1].as_args()[..10],
            [
                "-L",
                "ae",
                "bind-key",
                "-T",
                "root",
                "MouseDown3Status",
                "run-shell",
                "-C",
                "-t",
                "{mouse}",
            ]
        );
        let down_right = bindings[1].as_args().last().expect("binding command");
        assert!(down_right.starts_with("#{?#{==:#{mouse_status_range},ae-settings},"));
        assert!(down_right.contains("'--settings' '--client' #{q:client_name}"));
        assert!(down_right.contains("'--popup' '--client' #{q:client_name}"));
        assert!(down_right.contains("#{==:#{mouse_status_range},session}"));
        assert!(down_right.contains("'display-menu' '-M' '-O' '-c'"));
        assert_eq!(down_right.matches("'--settings'").count(), 1);
        assert_eq!(down_right.matches("'--popup'").count(), 1);
        assert_eq!(
            bindings[2].as_args(),
            [
                "-L",
                "ae",
                "bind-key",
                "-T",
                "prefix",
                "a",
                "run-shell",
                "-b",
                "'/opt/ae' 'orchestrator' '--popup' '--client' #{q:client_name}",
            ]
        );
        assert_eq!(
            bindings[3].as_args(),
            ["-L", "ae", "unbind-key", "-T", "root", "MouseUp1Status"]
        );
        assert_eq!(
            bindings[4].as_args(),
            ["-L", "ae", "unbind-key", "-T", "root", "MouseUp3Status"]
        );
        for (binding, key) in bindings[5..].iter().zip([
            "MouseDown3Pane",
            "M-MouseDown3Pane",
            "MouseDown3StatusLeft",
            "M-MouseDown3Status",
            "M-MouseDown3StatusLeft",
        ]) {
            assert_eq!(
                binding.as_args(),
                ["-L", "ae", "unbind-key", "-T", "root", key],
                "remove the stock right-click menu for {key}"
            );
        }
        let keyboard = status_bindings_argv(&server, &["/opt/ae".to_owned()], false);
        assert_eq!(
            keyboard.len(),
            10,
            "keyboard-driven servers bind release and remove every stock right-click menu"
        );
        assert_eq!(
            keyboard[0].as_args(),
            [
                "-L",
                "ae",
                "bind-key",
                "-T",
                "root",
                "MouseDown1Status",
                "run-shell",
                "-C",
                "-t",
                "{mouse}",
                "#{?#{==:#{mouse_status_range},ae-settings},,#{?#{||:#{==:#{mouse_status_range},ae},#{==:#{mouse_status_range},ae-more}},,#{?#{==:#{mouse_status_range},window},select-window -t #{window_id},#{?#{==:#{mouse_status_range},session},switch-client -c #{q:client_name} -t #{session_id},}}}}"
            ]
        );
        assert_eq!(
            keyboard[1].as_args(),
            [
                "-L",
                "ae",
                "bind-key",
                "-T",
                "root",
                "MouseDown3Status",
                "run-shell",
                "-C",
                "-t",
                "{mouse}",
                "#{?#{==:#{mouse_status_range},ae-settings},,#{?#{||:#{==:#{mouse_status_range},ae},#{==:#{mouse_status_range},ae-more}},,#{?#{==:#{mouse_status_range},session},,}}}"
            ]
        );
        assert_eq!(
            keyboard[2].as_args(),
            [
                "-L",
                "ae",
                "bind-key",
                "-T",
                "root",
                "MouseUp1Status",
                "run-shell",
                "-C",
                "-t",
                "{mouse}",
                "#{?#{==:#{mouse_status_range},ae-settings},run-shell -b \"'/opt/ae' 'orchestrator' '--settings' '--client' #{q:client_name}\",#{?#{||:#{==:#{mouse_status_range},ae},#{==:#{mouse_status_range},ae-more}},run-shell -b \"'/opt/ae' 'orchestrator' '--popup' '--client' #{q:client_name}\",}}"
            ]
        );
        assert_eq!(
            &keyboard[3].as_args()[..10],
            [
                "-L",
                "ae",
                "bind-key",
                "-T",
                "root",
                "MouseUp3Status",
                "run-shell",
                "-C",
                "-t",
                "{mouse}",
            ]
        );
        let up_right = keyboard[3].as_args().last().expect("binding command");
        assert!(up_right.starts_with("#{?#{==:#{mouse_status_range},ae-settings},"));
        assert!(up_right.contains("'--settings' '--client' #{q:client_name}"));
        assert!(up_right.contains("'--popup' '--client' #{q:client_name}"));
        assert!(up_right.contains("'display-menu' '-O' '-c'"));
        assert!(!up_right.contains("'display-menu' '-M'"));
        assert_eq!(up_right.matches("'--settings'").count(), 1);
        assert_eq!(up_right.matches("'--popup'").count(), 1);
        assert_eq!(
            keyboard[4].as_args(),
            [
                "-L",
                "ae",
                "bind-key",
                "-T",
                "prefix",
                "a",
                "run-shell",
                "-b",
                "'/opt/ae' 'orchestrator' '--popup' '--client' #{q:client_name}",
            ]
        );
        for (binding, key) in keyboard[5..].iter().zip([
            "MouseDown3Pane",
            "M-MouseDown3Pane",
            "MouseDown3StatusLeft",
            "M-MouseDown3Status",
            "M-MouseDown3StatusLeft",
        ]) {
            assert_eq!(
                binding.as_args(),
                ["-L", "ae", "unbind-key", "-T", "root", key],
                "remove the stock right-click menu for {key}"
            );
        }
        assert!(
            status_bindings_argv(&ServerId::Ambient, &["/opt/ae".to_owned()], true).is_empty(),
            "an ambient server's root table belongs to its user"
        );
    }

    /// The context menu is drawn in the middle of the terminal that asked for
    /// it. `M`/`S` put it at the mouse, which is the bottom status line the
    /// click happened on, and `0`/`S` is the fleet picker's own corner.
    #[test]
    fn the_session_context_menu_is_centred_on_its_client_and_the_picker_is_not() {
        let server = ServerId::Selected(crate::meta::Selector::Name("ae".to_owned()));
        let menu = session_menu_command(
            &server,
            &["/opt/ae".to_owned()],
            MOUSE_DOWN_STATUS_MENU_ACTION,
            true,
        );
        assert!(
            menu.contains("'-x' 'C' '-y' 'C'"),
            "the session menu centres on the whole client: {menu}"
        );
        assert!(
            !menu.contains("'-x' 'M'") && !menu.contains("'-y' 'S'"),
            "neither the mouse nor the status line positions it: {menu}"
        );
        assert!(
            crate::tmux::display_menu_args(
                &server,
                &crate::tmux::Menu {
                    title: String::new(),
                    title_style: String::new(),
                    items: Vec::new(),
                },
                true
            )
            .windows(4)
            .any(|words| words == ["-x", "0", "-y", "S"]),
            "the fleet picker keeps its bottom-left button"
        );
    }

    /// The Flip row is UNCHANGED by the new neighbour: same target pane, same
    /// deferred predicate, same hash count.
    #[test]
    fn the_flip_row_keeps_its_target_and_its_deferred_guard() {
        let server = ServerId::Selected(crate::meta::Selector::Name("ae".to_owned()));
        let menu = session_menu_command(
            &server,
            &["/opt/ae".to_owned()],
            MOUSE_DOWN_STATUS_MENU_ACTION,
            true,
        );
        assert!(
            menu.contains("'-t' '#{pane_id}'"),
            "the menu acts on the CLICKED pane, not the client's own: {menu}"
        );
        assert!(
            menu.contains("'Flip lead/colead panes' 'f' ")
                && menu.contains(
                    crate::tmux::tmux_current_format_double_quote(&fixed_mouse_shell_word(
                        MOUSE_DOWN_STATUS_MENU_ACTION
                    ))
                    .trim_start_matches('"')
                    .trim_end_matches('"')
                ),
            "the guard is still the deferred one: {menu}"
        );
    }

    /// The Stop row carries the click's own facts, each as ONE shell word, and
    /// the formats that produce them are never escaped.
    /// A single quote inside the row crosses the outer double quote too, so
    /// the backslash the inner shell needs is written twice by the time the
    /// binding holds it.
    const ESC: &str = "\\\\";

    #[test]
    fn the_stop_row_carries_every_captured_fact_as_one_quoted_word() {
        let server = ServerId::Selected(crate::meta::Selector::Name("ae".to_owned()));
        let menu = session_menu_command(
            &server,
            &["/opt/ae".to_owned()],
            MOUSE_DOWN_STATUS_MENU_ACTION,
            true,
        );
        assert!(
            menu.contains("'Stop session...' 's' "),
            "the row and its own key: {menu}"
        );
        for (flag, format) in [
            ("--client", "#{client_name}"),
            ("--client-pid", "#{client_pid}"),
            ("--session", "#{session_name}"),
            ("--session-id", "#{session_id}"),
            ("--pane", "#{pane_id}"),
            ("--server-pid", "#{pid}"),
            ("--server-start", "#{start_time}"),
        ] {
            assert!(
                menu.contains(&format!("{flag}'{ESC}'' '{ESC}''{format}")),
                "{flag} is followed by {format} as its own word: {menu}"
            );
        }
        assert!(
            !menu.contains("AEMENU"),
            "no placeholder survives into the binding: {menu}"
        );
        assert!(
            menu.contains(&format!(
                "'_session-menu'{ESC}'' '{ESC}''confirm'{ESC}'' '{ESC}''--action'{ESC}'' '{ESC}''stop'"
            )),
            "the row names the internal step and its action: {menu}"
        );
    }

    /// A server ae may not bind gets no row that would re-exec ae at all.
    #[test]
    fn a_menu_without_a_launcher_offers_only_the_native_flip() {
        let server = ServerId::Selected(crate::meta::Selector::Name("ae".to_owned()));
        let menu = session_menu_command(&server, &[], MOUSE_DOWN_STATUS_MENU_ACTION, true);
        assert!(
            !menu.contains("Stop session") && !menu.contains("_session-menu"),
            "no launcher, no ae row: {menu}"
        );
        assert!(menu.contains("'Flip lead/colead panes'"));
    }

    #[test]
    fn picker_launcher_uses_the_public_pointer_or_bakes_the_checkout_namespace() {
        let installed = crate::shape::Shape::Installed {
            home: "/Users/me/.ae".into(),
            version_dir: "/Users/me/.ae/versions/2026.9.34".into(),
            version: "2026.9.34".to_owned(),
        };
        assert_eq!(
            picker_launcher(
                &installed,
                Path::new("/Users/me/.ae/versions/2026.9.34/ae-core"),
                Path::new("/ignored/state"),
                Path::new("/ignored/config"),
                &ServerId::Selected(Selector::Name("ignored".to_owned())),
            ),
            ["/Users/me/.local/bin/ae"]
        );

        let socket = ServerId::Selected(Selector::Socket("/tmp/private.sock".into()));
        assert_eq!(
            picker_launcher(
                &crate::shape::Shape::Checkout,
                Path::new("/work/ae/target/debug/ae"),
                Path::new("/tmp/custom-state"),
                Path::new("/tmp/custom-config"),
                &socket,
            ),
            [
                "env",
                "AE_HOME=/tmp/custom-state",
                "CONFIG_FILE=/tmp/custom-config",
                "AE_TMUX_SERVER=/tmp/private.sock",
                "AE_TMUX_SERVER_KIND=socket",
                "/work/ae/target/debug/ae",
            ]
        );
    }

    #[test]
    fn only_a_pane_id_is_a_pane_id() {
        assert_eq!(interpret_pane_id(true, "%12\n").as_deref(), Some("%12"));
        assert_eq!(interpret_pane_id(true, "no server\n"), None);
        assert_eq!(interpret_pane_id(false, "%12\n"), None);
    }
}
