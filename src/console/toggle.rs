//! `_console [--jump] [--client <name>]` — the console window toggle and the
//! jump to the lead's pane, run by `prefix h` / `prefix H`.
//!
//! The console window is found only by its `@ae_console` stamp (the session's
//! `@ae_session_uuid`) and never carries `@ae_agent`, so the watchdog reads no
//! agent in it. `remain-on-exit` keeps a console that stopped following on
//! screen with its reopen hint; the same key respawns it.

use std::io::Write;
use std::path::Path;

use super::submit::first_console;
use crate::inventory::ServerId;
use crate::reader::{session_of, source_pane};
use crate::session_tmux::{Op, argv, interpret_pane_id, picker_launcher};
use crate::tmux::{OptionScope, WindowPane, session_target};
use crate::{theme, transport};

/// What the toggle does, decided from one pane listing.
#[derive(Debug, PartialEq, Eq)]
enum Plan {
    Open,
    /// The meta's `session_id` is not the stamp the host carries: no console is
    /// started for a session it could not be bound to.
    Unbound,
    Respawn(String),
    Select(String),
    Back,
}

/// The toggle's move for a key pressed in `source`; only a console stamped
/// with `uuid` counts. The live one that owns the session's input is selected
/// (from itself, the key goes back); only with none live is the first dead
/// one respawned, and a console is started only when the meta records `bound`
/// as that same id. `None` when `source` is not in the listing.
fn plan(panes: &[WindowPane], source: &str, uuid: &str, bound: &str) -> Option<Plan> {
    panes.iter().find(|pane| pane.pane_id == source)?;
    if let Some(owner) = first_console(panes, uuid, true) {
        let back = owner.pane_id == source;
        return Some(if back {
            Plan::Back
        } else {
            Plan::Select(owner.pane_id.clone())
        });
    }
    if bound.is_empty() || bound != uuid {
        return Some(Plan::Unbound);
    }
    Some(match first_console(panes, uuid, false) {
        None => Plan::Open,
        Some(dead) => Plan::Respawn(dead.pane_id.clone()),
    })
}

/// The verb: parse, resolve the pressing pane's session, act.
pub(crate) fn run(
    tail: &[String],
    _out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let (jump, rest) = match tail {
        [flag, rest @ ..] if flag == "--jump" => (true, rest),
        _ => (false, tail),
    };
    let client = match rest {
        [] => None,
        [flag, name] if flag == "--client" => Some(name.as_str()),
        _ => {
            writeln!(err, "usage: ae _console [--jump] [--client <name>]")?;
            return Ok(crate::entry::EXIT_USAGE);
        }
    };
    let Err(why) = act(jump, client) else {
        return Ok(0);
    };
    writeln!(err, "ae: {why}")?;
    Ok(crate::entry::EXIT_FAILED)
}

fn act(jump: bool, client: Option<&str>) -> Result<(), String> {
    let declared = crate::doors::declared_server(crate::shape::current());
    let server = crate::doors::launch_target(declared.as_ref())
        .ok_or("the tmux server pair is ambiguous")?;
    let source = source_pane(&server, client).ok_or("no calling pane for the chat key")?;
    let session =
        session_of(&server, &source).ok_or("the calling pane is not on this tmux server")?;
    let panes = transport::observe_window_panes(&server, &session)
        .ok_or("tmux did not answer the pane listing")?;
    let tmux = |op: &Op<'_>| match transport::run_tmux_op(&argv(&server, op)) {
        (true, out) => Ok(out),
        _ => Err("tmux refused the chat move".to_owned()),
    };
    let option = |name| transport::observe_session_option(&server, &session, name);
    if jump {
        let main = option(theme::MAIN_PANE_OPTION)
            .filter(|id| panes.iter().any(|pane| &pane.pane_id == id));
        let main = main.ok_or("this session has no lead pane to jump to")?;
        tmux(&Op::SelectWindow { pane: &main })?;
        return tmux(&Op::SelectPane { pane: &main }).map(drop);
    }
    let uuid = option(theme::SESSION_ID_OPTION).ok_or("this session has no @ae_session_uuid")?;
    let bound = crate::state_root()
        .and_then(|root| super::locate(&root, &session))
        .map(|dir| super::recorded_uuid(&dir))
        .unwrap_or_default();
    let plan = plan(&panes, &source, &uuid, &bound)
        .ok_or("the calling pane is not in a window ae can read")?;
    let pane = match &plan {
        Plan::Unbound => {
            return Err(
                "the session id in its meta is not the one tmux carries, so no chat is started"
                    .to_owned(),
            );
        }
        Plan::Back => return tmux(&Op::LastWindow { session: &session }).map(drop),
        Plan::Select(pane) => pane.clone(),
        Plan::Open | Plan::Respawn(_) => {
            let root = crate::state_root().ok_or("ae cannot name its own state")?;
            let config = crate::doors::config_file(crate::shape::current(), &root);
            let home = Home {
                root: &root,
                config: &config,
            };
            if let Plan::Respawn(pane) = &plan {
                respawn(&server, pane, &session, home)?;
                pane.clone()
            } else {
                open(&server, &session, &uuid, false, home)?
            }
        }
    };
    tmux(&Op::SelectWindow { pane: &pane })?;
    tmux(&Op::SelectPane { pane: &pane }).map(drop)
}

/// The ae a chat's launcher runs: its state root and its global config.
#[derive(Clone, Copy)]
pub(crate) struct Home<'a> {
    pub(crate) root: &'a Path,
    pub(crate) config: &'a Path,
}

/// Opens a chat window for `session`, stamped with `uuid`, and reports its
/// pane: at the session's FIRST index when `first`, else at the next free one.
///
/// A new window is a shell first: the stamp and `remain-on-exit` land before
/// the chat runs, so one that stops at once still leaves its pane and its
/// hint. A window that cannot be finished is killed, never left half-built.
///
/// # Errors
///
/// What failed, before or after the window existed.
pub(crate) fn open(
    server: &ServerId,
    session: &str,
    uuid: &str,
    first: bool,
    home: Home<'_>,
) -> Result<String, String> {
    let command = command(server, session, home)?;
    let target = format!(
        "{}:{}",
        session_target(session),
        if first { "^" } else { "" }
    );
    let window = Op::NewWindow {
        target: &target,
        before: first,
        name: "chat",
        work_dir: "",
        command: &[],
    };
    let (created, out) = transport::run_tmux_op(&argv(server, &window));
    let pane = interpret_pane_id(created, &out).ok_or("tmux did not open the chat window")?;
    let finished =
        if transport::publish_option(server, OptionScope::Pane, &pane, "@ae_console", uuid) {
            let tmux = |op: &Op<'_>| transport::run_tmux_op(&argv(server, op)).0;
            let kept = tmux(&Op::SetWindowOption {
                target: &pane,
                name: "remain-on-exit",
                value: "on",
            });
            let started = kept
                && tmux(&Op::RespawnPane {
                    pane: &pane,
                    work_dir: "",
                    command: &command,
                });
            if started {
                Ok(())
            } else {
                Err("tmux refused to start the chat in its window")
            }
        } else {
            Err("tmux refused to stamp the chat pane")
        };
    let Err(why) = finished else {
        return Ok(pane);
    };
    if close(server, session, &pane, uuid, true) {
        Err(format!("{why}; its window was closed"))
    } else {
        Err(format!("{why}; its window ({pane}) could not be closed"))
    }
}

/// Whose `pane` is, read from one listing of its session.
#[derive(Debug, PartialEq, Eq)]
enum Owner {
    Gone,
    /// A chat stamped `uuid` — or, for the caller that just `created` it,
    /// the still unstamped shell.
    Chat,
    Other,
}

fn owner(panes: &[WindowPane], pane: &str, uuid: &str, created: bool) -> Owner {
    let ours = |row: &WindowPane| match row.console.as_deref() {
        Some(stamp) => stamp == uuid,
        None => created,
    };
    match panes.iter().find(|row| row.pane_id == pane) {
        None => Owner::Gone,
        Some(row) if row.agent.is_none() && ours(row) => Owner::Chat,
        Some(_) => Owner::Other,
    }
}

/// Closes the chat in `pane` of `session` only while it is still the chat of
/// `uuid` — an unstamped shell only when this caller `created` it — and says
/// whether it is gone: a second listing proves it, so a kill tmux refused or
/// never ran reads false.
pub(crate) fn close(
    server: &ServerId,
    session: &str,
    pane: &str,
    uuid: &str,
    created: bool,
) -> bool {
    let read = || transport::observe_window_panes(server, session);
    let owned = |panes: Vec<WindowPane>| owner(&panes, pane, uuid, created);
    match read().map(owned) {
        Some(Owner::Gone) => true,
        Some(Owner::Chat) => {
            transport::kill_pane(server, pane) && read().map(owned) == Some(Owner::Gone)
        }
        Some(Owner::Other) | None => false,
    }
}

/// Restarts the chat in `pane` as the chat of `session`.
///
/// # Errors
///
/// What failed.
pub(crate) fn respawn(
    server: &ServerId,
    pane: &str,
    session: &str,
    home: Home<'_>,
) -> Result<(), String> {
    let command = command(server, session, home)?;
    let op = Op::RespawnPane {
        pane,
        work_dir: "",
        command: &command,
    };
    match transport::run_tmux_op(&argv(server, &op)) {
        (true, _) => Ok(()),
        _ => Err("tmux refused to restart the chat".to_owned()),
    }
}

/// The chat window's argv, the same for every opener: the terminal set up,
/// then this core's launcher running the chat of `session` with its input.
fn command(server: &ServerId, session: &str, home: Home<'_>) -> Result<Vec<String>, String> {
    let core = crate::shape::resolved_exe().ok_or("ae cannot name its own binary")?;
    let shape = crate::shape::current();
    let launcher = picker_launcher(shape, &core, home.root, home.config, server);
    Ok(console_command(launcher, session))
}

/// Sets the pane's terminal to hand the console each key as it is typed, with
/// no echo and no flow control — ^C still interrupts and Enter still reads as a
/// newline — then execs the console named by its own arguments.
const TTY_SETUP: &str = "stty -icanon -echo -ixon -iexten min 1 time 0 && exec \"$0\" \"$@\"";

/// The console window's argv: the terminal set up, then `launcher` running
/// the console of `session` with its input.
fn console_command(launcher: Vec<String>, session: &str) -> Vec<String> {
    let setup = ["/bin/sh", "-c", TTY_SETUP].map(ToOwned::to_owned);
    let console = ["chat", session, "--follow", "--input"].map(ToOwned::to_owned);
    setup.into_iter().chain(launcher).chain(console).collect()
}

#[cfg(test)]
mod tests {
    use super::{Owner, Plan, TTY_SETUP, console_command, owner, plan};
    use crate::console::submit::tests::pane;
    use crate::tmux::WindowPane;

    #[test]
    fn the_toggle_opens_selects_returns_respawns_and_ignores_a_foreign_stamp() {
        let lead = pane("%1", "@0", None, false);
        let (live, dead) = (
            pane("%3", "@2", Some("u"), false),
            pane("%3", "@2", Some("u"), true),
        );
        let foreign = pane("%3", "@2", Some("other"), false);
        let cases = [
            (None, "%1", Some(Plan::Open)),
            (
                Some(live.clone()),
                "%1",
                Some(Plan::Select("%3".to_owned())),
            ),
            (Some(live), "%3", Some(Plan::Back)),
            (Some(dead), "%3", Some(Plan::Respawn("%3".to_owned()))),
            (Some(foreign), "%1", Some(Plan::Open)),
            (None, "%9", None),
        ];
        for (console, from, want) in cases {
            let panes: Vec<WindowPane> = std::iter::once(lead.clone()).chain(console).collect();
            assert_eq!(plan(&panes, from, "u", "u"), want, "{from}");
        }
    }

    #[test]
    fn no_console_starts_for_a_meta_id_that_is_not_the_stamp() {
        let lead = pane("%1", "@0", None, false);
        let (live, dead) = (
            pane("%3", "@2", Some("u"), false),
            pane("%3", "@2", Some("u"), true),
        );
        for bound in ["", "other"] {
            for console in [None, Some(dead.clone())] {
                let panes: Vec<WindowPane> = std::iter::once(lead.clone()).chain(console).collect();
                let got = plan(&panes, "%1", "u", bound);
                assert_eq!(got, Some(Plan::Unbound), "{bound:?}");
            }
        }
        let panes = [lead, live];
        let got = plan(&panes, "%1", "u", "other");
        assert_eq!(got, Some(Plan::Select("%3".to_owned())));
    }

    #[test]
    fn the_toggle_takes_the_owner_by_index_never_by_listing_order() {
        let lead = pane("%1", "@0", None, false);
        let at = |id, window, dead| pane(id, window, Some("u"), dead);
        let first = WindowPane {
            window_index: 1,
            ..at("%9", "@9", false)
        };
        let select = |id: &str| Some(Plan::Select(id.to_owned()));
        let respawn = Some(Plan::Respawn("%3".to_owned()));
        let cases = [
            (
                vec![at("%7", "@5", false), at("%3", "@2", false)],
                "%1",
                select("%3"),
            ),
            (vec![at("%7", "@5", false), first], "%1", select("%9")),
            (
                vec![at("%3", "@2", true), at("%7", "@5", false)],
                "%1",
                select("%7"),
            ),
            (
                vec![at("%4", "@2", false), at("%3", "@2", false)],
                "%4",
                select("%3"),
            ),
            (
                vec![at("%4", "@2", false), at("%3", "@2", false)],
                "%3",
                Some(Plan::Back),
            ),
            (
                vec![at("%7", "@5", true), at("%3", "@2", true)],
                "%1",
                respawn,
            ),
        ];
        for (consoles, from, want) in cases {
            let panes: Vec<WindowPane> = std::iter::once(lead.clone()).chain(consoles).collect();
            assert_eq!(plan(&panes, from, "u", "u"), want, "{from} {panes:?}");
        }
    }

    #[test]
    fn only_a_chat_of_this_session_or_the_shell_its_opener_made_is_closed() {
        let seat = WindowPane {
            agent: Some("lead".to_owned()),
            ..pane("%1", "@0", None, false)
        };
        let panes = [
            seat,
            pane("%2", "@1", None, false),
            pane("%3", "@2", Some("u"), true),
            pane("%4", "@3", Some("other"), false),
        ];
        for (id, created, want) in [
            ("%9", false, Owner::Gone),
            ("%2", true, Owner::Chat),
            ("%2", false, Owner::Other),
            ("%3", false, Owner::Chat),
            ("%3", true, Owner::Chat),
            ("%1", true, Owner::Other),
            ("%4", true, Owner::Other),
        ] {
            assert_eq!(owner(&panes, id, "u", created), want, "{id} {created}");
        }
    }

    #[test]
    fn the_console_window_runs_its_launcher_on_a_terminal_set_up_for_keys() {
        let launcher = vec!["env".to_owned(), "/core".to_owned()];
        let want = ["/bin/sh", "-c", TTY_SETUP, "env", "/core", "chat", "s"];
        let want = [&want[..], &["--follow", "--input"]].concat();
        assert_eq!(console_command(launcher, "s"), want);
    }
}
