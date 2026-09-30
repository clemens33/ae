//! `_console [--jump] [--client <name>]` — the console window toggle and the
//! jump to the lead's pane, run by `prefix h` / `prefix H`.
//!
//! The console window is found only by its `@ae_console` stamp (the session's
//! `@ae_session_uuid`) and never carries `@ae_agent`, so the watchdog reads no
//! agent in it. `remain-on-exit` keeps a console that stopped following on
//! screen with its reopen hint; the same key respawns it.

use std::io::Write;

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
/// with `uuid` counts, and one is started only when the meta records `bound`
/// as that same id. `None` when `source` is not in the listing.
fn plan(panes: &[WindowPane], source: &str, uuid: &str, bound: &str) -> Option<Plan> {
    let here = panes.iter().find(|pane| pane.pane_id == source)?;
    let mine = |pane: &&WindowPane| pane.console.as_deref() == Some(uuid);
    let starts = panes.iter().find(mine).is_none_or(|pane| pane.dead);
    if starts && (bound.is_empty() || bound != uuid) {
        return Some(Plan::Unbound);
    }
    Some(match panes.iter().find(mine) {
        None => Plan::Open,
        Some(found) if found.dead => Plan::Respawn(found.pane_id.clone()),
        Some(found) if found.window_id == here.window_id => Plan::Back,
        Some(found) => Plan::Select(found.pane_id.clone()),
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
    let source = source_pane(&server, client).ok_or("no calling pane for the console key")?;
    let session =
        session_of(&server, &source).ok_or("the calling pane is not on this tmux server")?;
    let panes = transport::observe_window_panes(&server, &session)
        .ok_or("tmux did not answer the pane listing")?;
    let tmux = |op: &Op<'_>| match transport::run_tmux_op(&argv(&server, op)) {
        (true, out) => Ok(out),
        _ => Err("tmux refused the console move".to_owned()),
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
                "the session id in its meta is not the one tmux carries, so no console is started"
                    .to_owned(),
            );
        }
        Plan::Back => return tmux(&Op::LastWindow { session: &session }).map(drop),
        Plan::Select(pane) => pane.clone(),
        Plan::Open | Plan::Respawn(_) => {
            let (root, core) = (crate::state_root(), crate::shape::resolved_exe());
            let (Some(root), Some(core)) = (root, core) else {
                return Err("ae cannot name its own state".to_owned());
            };
            let config = crate::doors::config_file(crate::shape::current(), &root);
            let mut command =
                picker_launcher(crate::shape::current(), &core, &root, &config, &server);
            command.extend(["console", &session, "--follow"].map(ToOwned::to_owned));
            // A new window is a shell first: the stamp and `remain-on-exit`
            // land before the console runs, so one that stops at once still
            // leaves its pane and its hint.
            let pane = if let Plan::Respawn(pane) = &plan {
                pane.clone()
            } else {
                let target = format!("{}:", session_target(&session));
                let window = Op::NewWindow {
                    target: &target,
                    name: "console",
                    work_dir: "",
                    command: &[],
                };
                let pane = interpret_pane_id(true, &tmux(&window)?)
                    .ok_or("tmux did not name the console pane")?;
                if !transport::publish_option(
                    &server,
                    OptionScope::Pane,
                    &pane,
                    "@ae_console",
                    &uuid,
                ) {
                    let _ = transport::kill_pane(&server, &pane);
                    return Err("tmux refused to stamp the console pane".to_owned());
                }
                tmux(&Op::SetWindowOption {
                    target: &pane,
                    name: "remain-on-exit",
                    value: "on",
                })?;
                pane
            };
            tmux(&Op::RespawnPane {
                pane: &pane,
                work_dir: "",
                command: &command,
            })?;
            pane
        }
    };
    tmux(&Op::SelectWindow { pane: &pane }).map(drop)
}

#[cfg(test)]
mod tests {
    use super::{Plan, plan};
    use crate::tmux::WindowPane;

    fn pane(id: &str, window: &str, console: Option<&str>, dead: bool) -> WindowPane {
        let console = console.map(str::to_owned);
        let (pane_id, window_id) = (id.to_owned(), window.to_owned());
        WindowPane {
            pane_id,
            window_id,
            theme: String::new(),
            reader_src: None,
            console,
            dead,
            agent: None,
        }
    }

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
}
