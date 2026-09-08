//! The launch operation's tmux argv — the sealed builder behind
//! [`crate::transport::run_tmux_op`].
//!
//! The tmux calls the launch path makes: `new-session`, `set-environment`,
//! `split-window`, `new-window`,
//! `respawn-pane`, `select-layout`, `select-pane`, `select-window`, `set-hook`,
//! `bind-key`, `set-window-option`.
//!
//! Same shape as [`crate::git`] and for the same reason: the inner vector of
//! [`TmuxArgv`] is private to this module, so no other module can hand the
//! process door an arbitrary tmux command line. Every operation is a variant
//! here, and the variants are the whole surface.
//!
//! Options that a typed builder already covers — pane/window/session
//! `set-option` — are NOT re-spelled here: they go through
//! [`crate::transport::publish_option`], which is the existing door.

use crate::inventory::ServerId;
use crate::tmux::{MOUSE_DOWN_STATUS_DISPATCH, server_args, session_target};

/// The `-P -F` format every pane-creating call here prints.
const PANE_ID_FORMAT: &str = "#{pane_id}";

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
    /// `set-window-option -t <target> main-pane-width 66%` — the lead-pair
    /// window's persistent two-thirds main column.
    SetLeadPairWidth { target: &'a str },
    /// `select-pane -t <pane>` — focus.
    SelectPane { pane: &'a str },
    /// `select-pane -t <pane> -d` — make a monitor pane read-only.
    DisablePane { pane: &'a str },
    /// `select-window -t <pane>` — the `focus` helper's window switch, which
    /// `select-pane` alone does not do.
    SelectWindow { pane: &'a str },
    /// `set-hook -t <pane> client-session-changed <focus command>` — keep a
    /// client's view on the lead pane whenever it enters this pane's session.
    /// `set-hook` takes a target-PANE, so the pane id makes this session-scoped
    /// without a name target (and without prefix matching).
    SetClientSessionHook { pane: &'a str },
    /// `set-hook -w -t <pane> window-resized <layout command>` — keep the
    /// lead-pair ratio when its window follows a client resize.
    SetLeadPairResizeHook { pane: &'a str },
    /// Replace tmux's root `MouseDown1Status` on an ae-owned server so a
    /// window-range click selects the window without firing the session hook.
    BindMouseDownStatus,
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
                ["set-window-option", "-t", target, "main-pane-width", "66%"]
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
        Op::SetClientSessionHook { pane } => {
            args.extend(
                [
                    "set-hook",
                    "-t",
                    pane,
                    "client-session-changed",
                    &format!("select-window -t {pane} ; select-pane -t {pane}"),
                ]
                .map(ToOwned::to_owned),
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
                    &format!("select-layout -t {pane} main-vertical"),
                ]
                .map(ToOwned::to_owned),
            );
        }
        Op::BindMouseDownStatus => {
            args.extend(["bind-key", "-T", "root", "MouseDown1Status"].map(ToOwned::to_owned));
            args.extend(MOUSE_DOWN_STATUS_DISPATCH.map(ToOwned::to_owned));
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

/// The server-global status binding for a positively selected, ae-owned
/// server. Ambient means the user's own root table, which launch never writes.
pub(crate) fn mouse_down_status_binding_argv(server: &ServerId) -> Option<TmuxArgv> {
    match server {
        ServerId::Ambient => None,
        ServerId::Selected(_) => Some(argv(server, &Op::BindMouseDownStatus)),
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
            vec!["set-window-option", "-t", "%1", "main-pane-width", "66%"]
        );
    }

    #[test]
    fn the_session_focus_hook_targets_the_lead_pane_and_keeps_its_command_one_argv_element() {
        assert_eq!(
            words(&Op::SetClientSessionHook { pane: "%9" }),
            vec![
                "set-hook",
                "-t",
                "%9",
                "client-session-changed",
                "select-window -t %9 ; select-pane -t %9"
            ]
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
                "select-layout -t %9 main-vertical"
            ]
        );
    }

    #[test]
    fn the_status_click_binding_is_only_minted_for_an_ae_owned_server() {
        let server = ServerId::Selected(crate::meta::Selector::Name("ae".to_owned()));
        let binding = mouse_down_status_binding_argv(&server)
            .unwrap_or_else(|| panic!("a selected server owns its root table"));
        assert_eq!(
            binding.as_args(),
            [
                "-L",
                "ae",
                "bind-key",
                "-T",
                "root",
                "MouseDown1Status",
                "if-shell",
                "-F",
                "#{==:#{mouse_status_range},window}",
                "select-window -t =",
                "switch-client -t ="
            ]
        );
        assert!(
            mouse_down_status_binding_argv(&ServerId::Ambient).is_none(),
            "an ambient server's root table belongs to its user"
        );
    }

    #[test]
    fn only_a_pane_id_is_a_pane_id() {
        assert_eq!(interpret_pane_id(true, "%12\n").as_deref(), Some("%12"));
        assert_eq!(interpret_pane_id(true, "no server\n"), None);
        assert_eq!(interpret_pane_id(false, "%12\n"), None);
    }
}
