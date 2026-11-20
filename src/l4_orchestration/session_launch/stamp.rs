//! Launch-time tmux stamping: session, window and pane options, the status bar, the layout, and the monitor panes.

use super::{
    CHAT_FAILED_ACTION, Env, Mode, Op, Path, PathBuf, ServerId, Session, TmuxArgv, Write, argv,
    config, io, meta, monitor_pane, new_window_running, picker_launcher, status_bindings_argv,
    tmux, transport,
};

/// The environment, options and status bar every ae session carries.
pub(super) fn stamp_session(
    server: &ServerId,
    env: &Env,
    shape: &Session,
    main_pane: &str,
    core: &Path,
) {
    let name = shape.name.as_str();
    // Claude Code refuses to start with these set inside tmux.
    for key in ["CLAUDECODE", "CLAUDE_CODE_SESSION"] {
        let _ = transport::run_tmux_op(&argv(server, &Op::UnsetEnv { session: name, key }));
    }
    let work_dir = shape.work_dir.display().to_string();
    let origin = shape.origin.display().to_string();
    let home = env.home.display().to_string();
    for (key, value) in [
        ("AE_SESSION", "1"),
        ("AE_ORIGIN", origin.as_str()),
        ("AE_DIR", work_dir.as_str()),
        ("AE_MODE", shape.mode.as_str()),
        ("AE_HOME", home.as_str()),
    ] {
        let _ = transport::run_tmux_op(&argv(
            server,
            &Op::SetEnv {
                session: name,
                key,
                value,
            },
        ));
    }
    for (option, value) in [
        ("mouse", "on"),
        ("focus-events", "on"),
        ("history-limit", "50000"),
        ("automatic-rename", "off"),
    ] {
        let _ = transport::publish_option(server, tmux::OptionScope::Session, name, option, value);
    }
    stamp_main_pane(server, name, main_pane, true);
    apply_status_bar(
        server,
        &shape.name,
        &status_paths(
            shape.mode.as_str(),
            &shape.origin.display().to_string(),
            &shape.work_dir.display().to_string(),
            &home,
        ),
        &shape.look,
    );
    assert_status_bindings(server, env, core);
    let _ = transport::run_tmux_op(&argv(server, &Op::SelectPane { pane: main_pane }));
}

/// Publish the lead-pane navigation fact only after membership was proven.
///
/// A stale hint is actively unset: leaving one behind would let a future pane
/// with the same text appear authoritative even though this pass disproved it.
pub(crate) fn stamp_main_pane(
    server: &ServerId,
    session: &str,
    main_pane: &str,
    pane_belongs: bool,
) {
    if pane_belongs && !main_pane.is_empty() {
        let _ = transport::publish_option(
            server,
            tmux::OptionScope::Session,
            session,
            crate::theme::MAIN_PANE_OPTION,
            main_pane,
        );
    } else {
        let _ = transport::clear_option(
            server,
            tmux::OptionScope::Session,
            session,
            crate::theme::MAIN_PANE_OPTION,
        );
    }
    stamp_client_session_hook(server, session, main_pane, pane_belongs);
}

/// Install the session focus hook with the session id captured NOW.
///
/// The hook fires after `switch-client`; its pane may have moved by then. The
/// runtime predicate prevents that stale pane from selecting a foreign
/// session's window. Reattach and migration pass through here too, replacing
/// the unguarded hook published by an older core.
pub(crate) fn stamp_client_session_hook(
    server: &ServerId,
    session: &str,
    main_pane: &str,
    pane_belongs: bool,
) {
    let session_target = tmux::session_target(session);
    let Some(session_id) = transport::observe_session_id(server, session) else {
        let _ = transport::run_tmux_op(&argv(
            server,
            &Op::UnsetClientSessionHook {
                target: &session_target,
            },
        ));
        return;
    };
    if pane_belongs && !main_pane.is_empty() {
        let uuid =
            transport::observe_session_option(server, session, crate::theme::SESSION_ID_OPTION)
                .unwrap_or_default();
        let panes = transport::observe_window_panes(server, session).unwrap_or_default();
        let chat = window_zero_chat(&panes, &uuid).map(|pane| (pane.as_str(), uuid.as_str()));
        let _ = transport::run_tmux_op(&argv(
            server,
            &Op::SetClientSessionHook {
                session_id: &session_id,
                pane: main_pane,
                chat,
            },
        ));
    } else {
        let _ = transport::run_tmux_op(&argv(
            server,
            &Op::UnsetClientSessionHook {
                target: &session_id,
            },
        ));
    }
}

/// The chat pane a client entering the session lands on: one stamped with the
/// session's canonical `uuid` in its FIRST window, live or not — the hook
/// itself falls back to the lead whenever that pane is dead or gone.
pub(super) fn window_zero_chat<'a>(
    panes: &'a [tmux::WindowPane],
    uuid: &str,
) -> Option<&'a String> {
    if !crate::archive::is_canonical_uuid(uuid) {
        return None;
    }
    let first = panes.iter().map(|pane| pane.window_index).min()?;
    panes
        .iter()
        .filter(|pane| pane.window_index == first && pane.console.as_deref() == Some(uuid))
        .min_by_key(|pane| pane.pane_index)
        .map(|pane| &pane.pane_id)
}

/// Opens the chat at the session's first index once the meta binds it: the
/// `@ae_session_uuid` tmux carries must be the `session_id` the meta records.
/// A failure is named once on `err` and once in the journal, and leaves no
/// window behind.
pub(super) fn open_first_chat(
    server: &ServerId,
    session: &str,
    dir: &Path,
    home: crate::console::toggle::Home<'_>,
    err: &mut impl Write,
) -> io::Result<Option<String>> {
    let uuid = transport::observe_session_option(server, session, crate::theme::SESSION_ID_OPTION)
        .unwrap_or_default();
    let opened = if uuid.is_empty() || uuid != crate::console::recorded_uuid(dir) {
        Err("the session id in its meta is not the one tmux carries".to_owned())
    } else {
        crate::console::toggle::open(server, session, &uuid, true, home)
    };
    let why = match opened {
        Ok(pane) => return Ok(Some(pane)),
        Err(why) => why,
    };
    writeln!(
        err,
        "Warning: the chat window could not be opened ({why}); prefix h opens the chat."
    )?;
    let _ = crate::store::open(dir).append_event(&crate::tracked::event_line(
        &crate::tracked::EventFields::new(
            crate::time::Timestamp::now(),
            "ae",
            CHAT_FAILED_ACTION,
            "chat",
            "",
            "",
            "",
            "",
            "",
            &format!("the chat window could not be opened: {why}"),
            "",
        ),
    ));
    Ok(None)
}

pub(super) fn assert_status_bindings(server: &ServerId, env: &Env, core: &Path) {
    let config = env
        .global
        .clone()
        .unwrap_or_else(|| env.home.join("config"));
    let launcher = picker_launcher(crate::shape::current(), core, &env.home, &config, server);
    let menu_mouse = transport::observe_tmux_floor(server).menu_mouse();
    for binding in status_bindings_argv(server, &launcher, menu_mouse) {
        let _ = transport::run_tmux_op(&binding);
    }
    // The kill-server lock rides the same server-global setup, on a published
    // core only: a checkout's servers (tests, `ae-dev`) stay killable.
    if crate::shape::current().published_home().is_some() {
        transport::assert_kill_lock(server);
    }
}

/// The SESSION-scoped look: the two status lines ae owns, and the `@ae_*`
/// values that fill them before the watchdog's first cycle.
///
/// Session scope throughout, so a non-ae session on the same server keeps the
/// user's own theme. The per-window half — pane borders, menu and popup styles
/// — is [`stamp_window`]'s, because tmux keeps those in the WINDOW table.
pub(crate) fn apply_status_bar(
    server: &ServerId,
    name: &str,
    paths: &str,
    look: &crate::theme::Look,
) {
    write_options(server, name, crate::theme::session_options(look, paths));
}

/// The same look, WITHOUT the attention seed — for a session that is already
/// running and already has a verdict the watchdog published.
pub(crate) fn redress_status_bar(
    server: &ServerId,
    name: &str,
    paths: &str,
    look: &crate::theme::Look,
) {
    write_options(server, name, crate::theme::redress_options(look, paths));
}

/// Write one option set at SESSION scope.
fn write_options(server: &ServerId, name: &str, options: Vec<(String, String)>) {
    for (option, value) in options {
        let _ =
            transport::publish_option(server, tmux::OptionScope::Session, name, &option, &value);
    }
}

/// The WINDOW-scoped look, stamped on one window.
pub(crate) fn stamp_window(server: &ServerId, target: &str, look: &crate::theme::Look) -> bool {
    if !look.drawn {
        // `[workspace] theme = off`: the window keeps whatever the user's own
        // tmux configuration gave it, stamp included, so nothing here is ever
        // written and the watchdog finds nothing to restamp.
        return true;
    }
    // `&=`, never `&&`: one refused option must not skip the rest of the set.
    let mut ok = true;
    let set = |name: &str, value: &str| {
        let (succeeded, _) = transport::run_tmux_op(&argv(
            server,
            &Op::SetWindowOption {
                target,
                name,
                value,
            },
        ));
        succeeded
    };
    for (option, value) in crate::theme::window_options(look) {
        ok &= set(&option, &value);
    }
    // The STAMP is the claim that all of the above happened, so it is written
    // LAST and only when they did. A stamp over a half-dressed window would
    // tell every later cycle there was nothing left to do.
    if !ok {
        return false;
    }
    set(
        crate::theme::WINDOW_STAMP_OPTION,
        &crate::theme::window_stamp(look),
    )
}

/// Stamp every window this session has — called once the layout, the seats and
/// the monitor panes have all created theirs.
pub(crate) fn stamp_windows(server: &ServerId, session: &str, look: &crate::theme::Look) {
    let Some(panes) = transport::observe_window_panes(server, session) else {
        return;
    };
    let mut done: Vec<String> = Vec::new();
    for pane in panes {
        if done.contains(&pane.window_id) {
            continue;
        }
        done.push(pane.window_id.clone());
        stamp_window(server, &pane.window_id, look);
    }
}

/// The location segment of the status bar, whose shape is mode-aware.
///
/// SHORTENED against `home`: the bar shares one line with the branch and the
/// watch count, and a worktree path spelled in full pushes both off the end.
pub(crate) fn status_paths(mode: &str, origin: &str, work_dir: &str, home: &str) -> String {
    let short = |path: &str| crate::theme::short_path(home, path);
    match Mode::parse(mode) {
        // An unrecorded or unknown mode reads as local: the work dir alone
        // rather than an arrow to nothing.
        Some(Mode::Local) | None => short(work_dir),
        Some(Mode::Git | Mode::Full) => format!("{} → {}", short(origin), short(work_dir)),
    }
}

/// Distribute the panes the layout put in each window.
pub(super) fn apply_layout(server: &ServerId, shape: &Session, panes: &[String], workers: usize) {
    if let Some(main_pane) = panes.first() {
        stamp_lead_pair_policy(server, shape.layout.as_str(), main_pane);
    }
    for command in layout_argvs(server, shape.layout.as_str(), &shape.name, panes, workers) {
        let _ = transport::run_tmux_op(&command);
    }
}

/// Idempotently install and apply the complete lead-pair window policy.
pub(crate) fn stamp_lead_pair_policy(server: &ServerId, layout: &str, main_pane: &str) {
    for command in lead_pair_policy_argvs(server, layout, main_pane) {
        let _ = transport::run_tmux_op(&command);
    }
}

pub(super) fn lead_pair_policy_argvs(
    server: &ServerId,
    layout: &str,
    main_pane: &str,
) -> Vec<TmuxArgv> {
    if layout != "lead-pair" || main_pane.is_empty() {
        return Vec::new();
    }
    vec![
        argv(server, &Op::SetLeadPairWidth { target: main_pane }),
        argv(server, &Op::SelectLeadPairLayout { pane: main_pane }),
        argv(server, &Op::SetLeadPairResizeHook { pane: main_pane }),
    ]
}

/// Build the complete, ordered layout program before touching tmux.
pub(super) fn layout_argvs(
    server: &ServerId,
    layout: &str,
    session: &str,
    panes: &[String],
    workers: usize,
) -> Vec<TmuxArgv> {
    if panes.len() <= 1 {
        return Vec::new();
    }
    let mut commands = Vec::new();
    let select = |target: &str, layout: &str| argv(server, &Op::SelectLayout { target, layout });
    match layout {
        "lead-pair" => {
            if workers > 2 {
                commands.push(select(&panes[2], "even-vertical"));
            }
        }
        "lead-solo" => {
            if workers > 1 {
                commands.push(select(&panes[1], "even-vertical"));
            }
        }
        "vertical" => commands.push(select(
            &format!("{}:", tmux::session_target(session)),
            "even-horizontal",
        )),
        _ => commands.push(select(
            &format!("{}:", tmux::session_target(session)),
            "even-vertical",
        )),
    }
    commands
}

/// Name a window for the first agent placed in it and freeze that name.
///
/// Later splits do not call this: once a second agent joins, the watchdog's
/// `@ae_window_agents` value carries both identities while the routing name
/// stays stable.
pub(crate) fn name_agent_window(server: &ServerId, pane: &str, name: &str) {
    let _ = transport::publish_option(
        server,
        tmux::OptionScope::Window,
        pane,
        "automatic-rename",
        "off",
    );
    let _ = transport::rename_window(server, pane, &tmux::format_literal(name));
}

/// Label one pane with the identity it holds.
///
/// The PROFILE goes on the pane too: the border title names what an agent is
/// as well as who, and the roster it would otherwise be read from is in the
/// meta, not in tmux.
pub(super) fn stamp_pane(server: &ServerId, pane: &str, name: &str, slot: &str, profile: &str) {
    let _ = transport::set_pane_title(server, pane, &format!("ae:{name}"));
    // The IDENTITY, verbatim: this is what the roster, the monitor's own names
    // and every pane lookup match against, so it is never rewritten.
    let _ = transport::publish_option(server, tmux::OptionScope::Pane, pane, "@ae_agent", name);
    // The same name as DRAWN. A seat name comes back off a hand-editable meta,
    // and the border format reads this one, so what a drawer would take for a
    // style is dropped here rather than in the identity.
    let _ = transport::publish_option(
        server,
        tmux::OptionScope::Pane,
        pane,
        crate::theme::AGENT_LABEL_OPTION,
        &crate::theme::agent_label(name),
    );
    let _ = transport::publish_option(server, tmux::OptionScope::Pane, pane, "@ae_slot", slot);
    let _ = transport::publish_option(
        server,
        tmux::OptionScope::Pane,
        pane,
        crate::theme::PROFILE_OPTION,
        // SANITISED at the sink: a profile name comes back off a hand-editable
        // meta, and the drawer reads `#[…]` out of an option value, so a
        // profile carrying one would restyle the pane border it names.
        &crate::theme::bar_text(profile, crate::theme::PROFILE_WIDTH),
    );
}

/// The look `session` declares, or `None` when the server did not answer.
///
/// Read from the session rather than from the config: the look is the
/// SESSION's, and a helper that ran with a different config must not redress it.
/// A failed READ is not a default look either — standing one in would stamp a
/// window of a theme-off session in ae's colours, and the stamp would then hide
/// the mismatch from every later cycle.
pub(crate) fn look_of(server: &ServerId, session: &str) -> Option<crate::theme::Look> {
    let read = transport::observe_look(server, session)?;
    Some(crate::theme::Look::read(
        &read.icons,
        &read.palette,
        &read.drawn,
        &read.motion,
    ))
}

/// The monitor window's events pane, created if it is not already there.
pub(crate) fn ensure_events_pane(server: &ServerId, session: &str, dir: &Path) -> Option<String> {
    if let Some(existing) = monitor_pane(server, session, "_events") {
        mark_plumbing_window(server, &existing);
        return Some(existing);
    }
    let command = vec![dir.join("events-tail").display().to_string()];
    let pane = new_window_running(
        server,
        &format!("{}:99", tmux::session_target(session)),
        crate::theme::MONITOR_WINDOW,
        &command,
    )
    .or_else(|| {
        new_window_running(
            server,
            &tmux::session_target(session),
            crate::theme::MONITOR_WINDOW,
            &command,
        )
    })?;
    mark_plumbing_window(server, &pane);
    for (option, value) in [
        ("@ae_agent", "_events".to_owned()),
        (
            crate::theme::AGENT_LABEL_OPTION,
            crate::theme::agent_label("_events"),
        ),
    ] {
        let _ = transport::publish_option(server, tmux::OptionScope::Pane, &pane, option, &value);
    }
    let _ = transport::set_pane_title(server, &pane, "ae events");
    // The monitor window's borders are the LOOK's, so they are written by the
    // look and by nothing else: with `[workspace] theme = off` this window is
    // left exactly as the user's own tmux configuration draws it, like every
    // other window of the session.
    if let Some(look) = look_of(server, session) {
        stamp_window(
            server,
            &format!("{}:ae-monitor", tmux::session_target(session)),
            &look,
        );
    }
    let _ = transport::run_tmux_op(&argv(server, &Op::DisablePane { pane: &pane }));
    Some(pane)
}

/// Replace both monitor processes after a session directory moves.
///
/// Existing panes are respawned in place so window 99 keeps its pane ids and
/// layout. A monitor pane that vanished before rename reached it is recreated
/// through the ordinary start path. An enabled watchdog is not considered
/// rebound until its pidfile, process and stamped pane agree.
pub(crate) fn rebind_monitor_panes(
    root: &Path,
    server: &ServerId,
    session: &str,
    dir: &Path,
) -> Result<Option<u32>, String> {
    let events_command = vec![dir.join("events-tail").display().to_string()];
    if let Some(events) = monitor_pane(server, session, "_events") {
        let (respawned, reason) = transport::run_tmux_op(&argv(
            server,
            &Op::RespawnPane {
                pane: &events,
                work_dir: "",
                command: &events_command,
            },
        ));
        if !respawned {
            return Err(format!("could not respawn events pane {events}: {reason}"));
        }
    } else if ensure_events_pane(server, session, dir).is_none() {
        return Err("could not recreate the events pane".to_owned());
    }

    if !watchdog_enabled_for_session(dir) {
        return Ok(None);
    }
    if let Some(watchdog) = monitor_pane(server, session, "_watchdog") {
        let command = vec![dir.join("watchdog").display().to_string()];
        let (respawned, reason) = replace_watchdog_registration(dir, || {
            transport::run_tmux_op(&argv(
                server,
                &Op::RespawnPane {
                    pane: &watchdog,
                    work_dir: "",
                    command: &command,
                },
            ))
        });
        if !respawned {
            return Err(format!(
                "could not respawn watchdog pane {watchdog}: {reason}"
            ));
        }
    } else {
        let tail = ["start".to_owned(), session.to_owned()];
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let audit = crate::watchdog_lifecycle::RENAME_ACTOR;
        match crate::watchdog_lifecycle::run_with_actor(root, &tail, audit, &mut out, &mut err) {
            Ok(0) => {}
            Ok(_) | Err(_) => {
                return Err(format!(
                    "could not recreate the watchdog pane ({})",
                    String::from_utf8_lossy(&err).trim()
                ));
            }
        }
    }
    crate::watchdog_lifecycle::await_running(server, session, dir)
        .map(Some)
        .ok_or_else(|| "watchdog did not publish a live pid within the start bound".to_owned())
}

/// Replace a watchdog after releasing its old registration.
pub(super) fn replace_watchdog_registration<F>(dir: &Path, respawn: F) -> (bool, String)
where
    F: FnOnce() -> (bool, String),
{
    let old_pid = crate::watchdog_glue::read_pid(dir);
    if let Some(pid) = old_pid {
        let _ = crate::watchdog_glue::clear_pid(dir, pid);
    }
    respawn()
}

/// Whether the session's persisted settings ask for a watchdog.
///
/// The meta flag wins over config, as it does at launch. Otherwise the global
/// config recorded in meta and the session's resolved local overlay recreate
/// the launch-time layering. No setting means the documented default: on.
pub(crate) fn watchdog_enabled_for_session(dir: &Path) -> bool {
    let bytes = meta::read_bytes(dir).unwrap_or_default();
    let value = |key: &str| crate::lifecycle::meta_value(&bytes, key);
    let recorded = [value("watchdog"), value("loop")]
        .into_iter()
        .find(|value| !value.is_empty());
    let configured = if recorded.is_none() {
        let global = value("config");
        let global = (!global.is_empty()).then(|| PathBuf::from(global));
        let origin = value("origin");
        let local = config::local_overlay(dir, &origin);
        let values =
            config::read_workspace_keys(global.as_deref(), local.as_deref(), &["watchdog", "loop"]);
        values.into_iter().flatten().next()
    } else {
        None
    };
    !matches!(
        recorded
            .or(configured)
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "false" | "no" | "off" | "0"
    )
}

/// Mark the monitor pane's window as ae-owned plumbing for status rendering.
fn mark_plumbing_window(server: &ServerId, pane: &str) {
    let _ = transport::publish_option(
        server,
        tmux::OptionScope::Window,
        pane,
        crate::theme::WINDOW_PLUMBING_OPTION,
        "1",
    );
}
