//! The launch operation's tmux argv — the sealed builder behind
//! [`crate::transport::run_tmux_op`].
//!
//! Ported from the tmux calls the frozen launch path makes directly
//! (`ae:13342`-`13475`, `_watchdog_start` at `ae:12427`, `_monitor_ensure_events_pane`):
//! `new-session`, `set-environment`, `split-window`, `new-window`,
//! `select-layout`, `select-pane`, `select-window`, `show-options -g`.
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
use crate::tmux::server_args;

/// The `-P -F` format every pane-creating call here prints.
///
/// Spelled again rather than borrowed from [`crate::tmux`], where the same
/// constant is private to the watchdog's window builder: one shared constant
/// across two modules would make widening it there a silent change here.
const PANE_ID_FORMAT: &str = "#{pane_id}";

/// A tmux argv minted ONLY by this module's [`argv`] builder.
pub(crate) struct TmuxArgv(Vec<String>);

impl TmuxArgv {
    /// The argv for the transport door to spawn. Reading is harmless;
    /// construction is what is sealed.
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
    /// `-v -b` — stacked, the new pane ABOVE. The watchdog pane's shape, so
    /// the visual order stays watchdog-on-top / events-below.
    VerticalBefore,
}

/// One tmux command the launch operation runs.
pub(crate) enum Op<'a> {
    /// `new-session -d -s <name> -c <dir> -P -F '#{pane_id}'` — the session and
    /// its first pane, in one call, printing the pane id rather than asking for
    /// it afterwards.
    NewSession {
        /// The session name. Already validated against the session grammar.
        name: &'a str,
        /// The first pane's working directory.
        work_dir: &'a str,
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
    /// `select-layout -t <target> <layout>`.
    SelectLayout { target: &'a str, layout: &'a str },
    /// `select-pane -t <pane>` — focus.
    SelectPane { pane: &'a str },
    /// `select-pane -t <pane> -d` — make a monitor pane read-only.
    DisablePane { pane: &'a str },
    /// `select-window -t <pane>` — the `focus` helper's window switch, which
    /// `select-pane` alone does not do.
    SelectWindow { pane: &'a str },
    /// `rename-window -t <target> <name>`. `name` MUST already be
    /// [`crate::tmux::format_literal`]-escaped: a window name is a FORMAT.
    RenameWindow { target: &'a str, name: &'a str },
    /// `show-options -gv <name>` — the GLOBAL option value the session-scoped
    /// `status-format[0]` copy is taken from.
    ShowGlobalOption { name: &'a str },
    /// `set-window-option -t <target> <name> <value>` — the monitor window's
    /// `pane-border-status`.
    SetWindowOption {
        target: &'a str,
        name: &'a str,
        value: &'a str,
    },
    /// `capture-pane -p -J -S -<lines> -E - -t <pane>` — the `peek` helper.
    /// A separate variant from [`crate::tmux::capture_pane_args`], whose window
    /// is fixed at 40 lines.
    CapturePane { pane: &'a str, lines: u32 },
}

/// Build the argv for one operation, server selector first.
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
        Op::SetEnv {
            session,
            key,
            value,
        } => {
            args.extend(["set-environment", "-t", session, key, value].map(ToOwned::to_owned));
        }
        Op::UnsetEnv { session, key } => {
            args.extend(["set-environment", "-t", session, "-u", key].map(ToOwned::to_owned));
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
        Op::SelectLayout { target, layout } => {
            args.extend(["select-layout", "-t", target, layout].map(ToOwned::to_owned));
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
        Op::RenameWindow { target, name } => {
            args.extend(["rename-window", "-t", target, name].map(ToOwned::to_owned));
        }
        Op::ShowGlobalOption { name } => {
            args.extend(["show-options", "-gqv", name].map(ToOwned::to_owned));
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

/// The `#{pane_id}` a `-P -F` run printed, or `None` when nothing usable came
/// back. Mirrors [`crate::tmux::interpret_new_window`] — a pane id starts with
/// `%`, and anything else is a diagnostic that happened to reach stdout.
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
    fn only_a_pane_id_is_a_pane_id() {
        assert_eq!(interpret_pane_id(true, "%12\n").as_deref(), Some("%12"));
        assert_eq!(interpret_pane_id(true, "no server\n"), None);
        assert_eq!(interpret_pane_id(false, "%12\n"), None);
    }
}
