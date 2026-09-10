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
    MOUSE_DOWN_STATUS_MENU_ACTION, MOUSE_STATUS_PICKER, MOUSE_STATUS_SESSION, MOUSE_STATUS_WINDOW,
    hotkey_picker_shell, mouse_dispatch_literal, server_args, session_target,
    status_picker_command, tmux_current_format_double_quote,
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
    /// `set-window-option -t <target> main-pane-width 60%` — the lead-pair
    /// window's persistent 60/40 main column.
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
    BindMouseDownStatus { picker: &'a str },
    /// Bind ae's root `MouseDown3Status` context menu on an ae-owned server.
    BindMouseDownStatusMenu {
        picker: &'a str,
        menu_mouse: bool,
        flip: bool,
    },
    /// Bind the keyboard-driven picker's root `MouseUp1Status` release.
    BindMouseUpStatus { picker: &'a str },
    /// Bind the keyboard-driven picker's root `MouseUp3Status` release.
    BindMouseUpStatusMenu { picker: &'a str, menu_mouse: bool },
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
                ["set-window-option", "-t", target, "main-pane-width", "60%"]
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
        Op::BindMouseDownStatus { picker } => {
            args.extend(["bind-key", "-T", "root", "MouseDown1Status"].map(ToOwned::to_owned));
            args.extend(left_click_dispatch(picker));
        }
        Op::BindMouseDownStatusMenu {
            picker,
            menu_mouse,
            flip,
        } => {
            args.extend(["bind-key", "-T", "root", "MouseDown3Status"].map(ToOwned::to_owned));
            args.extend(right_click_dispatch(server, picker, menu_mouse, flip));
        }
        Op::BindMouseUpStatus { picker } => {
            args.extend(["bind-key", "-T", "root", "MouseUp1Status"].map(ToOwned::to_owned));
            args.extend(picker_click_dispatch(picker));
        }
        Op::BindMouseUpStatusMenu { picker, menu_mouse } => {
            args.extend(["bind-key", "-T", "root", "MouseUp3Status"].map(ToOwned::to_owned));
            args.extend(right_click_dispatch(server, picker, menu_mouse, true));
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

fn left_click_dispatch(picker: &str) -> Vec<String> {
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
    mouse_dispatch(format_if(MOUSE_STATUS_PICKER, picker, &window))
}

fn picker_click_dispatch(picker: &str) -> Vec<String> {
    mouse_dispatch(format_if(MOUSE_STATUS_PICKER, picker, ""))
}

fn fixed_mouse_shell_word(word: &str) -> String {
    mouse_dispatch_literal(&crate::launch::shell_quote(word))
}

fn flip_menu_command(server: &ServerId, action: &str, menu_mouse: bool) -> String {
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
    shell_words.extend(
        ["-x", "M", "-y", "S", "Flip lead/colead panes", "f", action].map(fixed_mouse_shell_word),
    );
    format!(
        "run-shell -b {}",
        tmux_current_format_double_quote(&shell_words.join(" "))
    )
}

fn right_click_dispatch(
    server: &ServerId,
    picker: &str,
    menu_mouse: bool,
    flip_enabled: bool,
) -> Vec<String> {
    let flip = if flip_enabled {
        flip_menu_command(server, MOUSE_DOWN_STATUS_MENU_ACTION, menu_mouse)
    } else {
        String::new()
    };
    let session = format_if(MOUSE_STATUS_SESSION, &flip, "");
    mouse_dispatch(format_if(MOUSE_STATUS_PICKER, picker, &session))
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
            let hotkey = hotkey_picker_shell(launcher);
            let mut bindings = if menu_mouse {
                vec![
                    argv(server, &Op::BindMouseDownStatus { picker: &picker }),
                    argv(
                        server,
                        &Op::BindMouseDownStatusMenu {
                            picker: &picker,
                            menu_mouse,
                            flip: true,
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
                    argv(server, &Op::BindMouseDownStatus { picker: "" }),
                    argv(
                        server,
                        &Op::BindMouseDownStatusMenu {
                            picker: "",
                            menu_mouse,
                            flip: false,
                        },
                    ),
                    argv(server, &Op::BindMouseUpStatus { picker: &picker }),
                    argv(
                        server,
                        &Op::BindMouseUpStatusMenu {
                            picker: &picker,
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
            vec!["set-window-option", "-t", "%1", "main-pane-width", "60%"]
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
                "#{?#{||:#{==:#{mouse_status_range},ae},#{==:#{mouse_status_range},ae-more}},run-shell -b \"'/opt/ae' 'orchestrator' '--popup' '--client' #{q:client_name}\",#{?#{==:#{mouse_status_range},window},select-window -t #{window_id},#{?#{==:#{mouse_status_range},session},switch-client -c #{q:client_name} -t #{session_id},}}}"
            ]
        );
        assert!(
            !bindings[0]
                .as_args()
                .iter()
                .any(|word| word.contains("@ae_orchestrator_id"))
        );
        assert_eq!(
            bindings[1].as_args(),
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
                "#{?#{||:#{==:#{mouse_status_range},ae},#{==:#{mouse_status_range},ae-more}},run-shell -b \"'/opt/ae' 'orchestrator' '--popup' '--client' #{q:client_name}\",#{?#{==:#{mouse_status_range},session},run-shell -b \"'tmux' '-L' 'ae' 'display-menu' '-M' '-O' '-c' #{q:client_name} '-t' '#{pane_id}' '-T' '#{session_name}' '-x' 'M' '-y' 'S' 'Flip lead/colead panes' 'f' 'if-shell -F '\\\\''########{&&:########{==:########{window_panes#}#,2#}#,########{==:########{window_zoomed_flag#}#,0#}#}'\\\\'' '\\\\''swap-pane -d -s \\\"{top-left#}\\\" -t \\\"{bottom-right#}\\\"'\\\\'' '\\\\''display-message \\\"flip needs an unzoomed two-pane window\\\"'\\\\'''\",}}"
            ]
        );
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
                "#{?#{||:#{==:#{mouse_status_range},ae},#{==:#{mouse_status_range},ae-more}},,#{?#{==:#{mouse_status_range},window},select-window -t #{window_id},#{?#{==:#{mouse_status_range},session},switch-client -c #{q:client_name} -t #{session_id},}}}"
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
                "#{?#{||:#{==:#{mouse_status_range},ae},#{==:#{mouse_status_range},ae-more}},,#{?#{==:#{mouse_status_range},session},,}}"
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
                "#{?#{||:#{==:#{mouse_status_range},ae},#{==:#{mouse_status_range},ae-more}},run-shell -b \"'/opt/ae' 'orchestrator' '--popup' '--client' #{q:client_name}\",}"
            ]
        );
        assert_eq!(
            keyboard[3].as_args(),
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
                "#{?#{||:#{==:#{mouse_status_range},ae},#{==:#{mouse_status_range},ae-more}},run-shell -b \"'/opt/ae' 'orchestrator' '--popup' '--client' #{q:client_name}\",#{?#{==:#{mouse_status_range},session},run-shell -b \"'tmux' '-L' 'ae' 'display-menu' '-O' '-c' #{q:client_name} '-t' '#{pane_id}' '-T' '#{session_name}' '-x' 'M' '-y' 'S' 'Flip lead/colead panes' 'f' 'if-shell -F '\\\\''########{&&:########{==:########{window_panes#}#,2#}#,########{==:########{window_zoomed_flag#}#,0#}#}'\\\\'' '\\\\''swap-pane -d -s \\\"{top-left#}\\\" -t \\\"{bottom-right#}\\\"'\\\\'' '\\\\''display-message \\\"flip needs an unzoomed two-pane window\\\"'\\\\'''\",}}"
            ]
        );
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
