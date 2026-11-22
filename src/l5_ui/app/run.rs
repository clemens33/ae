//! The run loop of ae app: its operands, the step, the drain and the terminal handoffs.

use std::io::IsTerminal as _;

use super::{
    App, Duration, FLIGHT_QUIT_BOUND, HINT, Instant, Keep, Keys, NO_TERMINAL, OWN, Path,
    RecvTimeoutError, ServerId, TICK, Terminal, Timestamp, USAGE, Wake, Write, backend, console,
    doors, draw_frame, forward, handover, loader, maintainer, mpsc, paneview, take_keys,
    terminal_text, toggle, transport, tty, upgrade,
};

/// `ae app [session]`.
///
/// # Errors
///
/// [`crate::error::Error::Io`] when the terminal or `err` cannot be written.
#[allow(
    clippy::too_many_lines,
    reason = "the one loop: its setup, then draw, show, drain"
)]
pub fn run(
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::error::Result<u8> {
    let Some(Operands {
        session,
        own,
        handed,
    }) = operands(tail)
    else {
        write!(err, "{USAGE}")?;
        return Ok(crate::entry::EXIT_USAGE);
    };
    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        write!(err, "{NO_TERMINAL}")?;
        return Ok(crate::EXIT_UNAVAILABLE);
    }
    let Some(root) = crate::doors::state_root(crate::shape::current()) else {
        writeln!(err, "ae: {}", crate::shape::NO_STATE_ROOT)?;
        return Ok(crate::EXIT_UNAVAILABLE);
    };
    // The human's operands as typed: what an exec restart hands on, unchanged.
    let typed: Vec<String> = session.iter().cloned().collect();
    let home = session.or_else(crate::calling_session_name);
    if let Some(name) = home.as_deref()
        && console::locate(&root, name).is_none()
    {
        let line = format!("ae app: no session named {name}");
        writeln!(err, "{}", crate::board::terminal_text(&line))?;
        return Ok(crate::EXIT_UNAVAILABLE);
    }
    let Some((reads, pause)) = console::term::stdin_reads_pausable() else {
        write!(err, "{NO_TERMINAL}")?;
        return Ok(crate::EXIT_UNAVAILABLE);
    };
    let (wake, wakes) = mpsc::channel();
    let (caller, pane) = (doors::caller_server(), doors::calling_pane_id());
    let at = caller.as_ref().zip(pane.as_deref());
    let stamp = if own || handed.is_some() {
        None
    } else {
        at.and_then(|(server, pane)| transport::observe_pane_console(server, pane))
    };
    let hint = !(own || handed.is_some()) && says_hint(at.is_some(), stamp.as_deref());
    // The owner's own start of ae: the app the entry opened (bare `ae`, a
    // launch that lands here), `ae app` typed outside tmux, or typed in a pane
    // proved ordinary — tmux named its session, and it carries no console stamp
    // and no agent (as `pane_is_seatless` reads it). Never a restart handed
    // on, an app or chat window a session runs, or an agent seat.
    let kicks = crate::orchestrator_autostart::app_kicks(
        handed.is_some(),
        own,
        caller.is_none(),
        stamp.as_deref(),
        || {
            at.is_some_and(|(server, pane)| {
                crate::pane_is_seatless(transport::reask_viewer(server, pane).as_ref())
            })
        },
    );
    let (keep, landed) = handed_on(handed);
    let own = (caller.clone(), pane.clone());
    let reader = reader_of(&root, home.clone(), own, landed);
    let notices = wake.clone();
    let autostart = wake.clone();
    let started = forward(reads, wake.clone()).and_then(|()| loader::spawn(reader, wake));
    let (ask, reader) = match started {
        Ok(started) => started,
        Err(why) => {
            writeln!(err, "ae app: {why}")?;
            return Ok(crate::EXIT_UNAVAILABLE);
        }
    };
    // The store's upkeep: an app that cannot start it runs without it. Every
    // return below drops it, which stops the thread.
    let mut upkeep = maintainer::Upkeep::start(root.clone(), notices);
    if hint {
        // Best effort: a stderr that cannot be written must not stop an app that opened before.
        let _ = writeln!(err, "{HINT}");
    }
    let tty = match tty::Tty::start() {
        Ok(tty) => tty,
        Err(why) => {
            writeln!(err, "ae app: {why}")?;
            return Ok(crate::EXIT_UNAVAILABLE);
        }
    };
    // Only once the app owns its terminal: an app that cannot open starts
    // nothing.
    if kicks {
        kick_orchestrator(&root, autostart);
    }
    let mut size = tty.size().unwrap_or((80, 24));
    let mut terminal = Terminal::new(backend::Ansi::new(out, size))?;
    let mut app = App::new(home, caller, pane, Some(ask));
    app.root = Some(root);
    app.keep = keep;
    let mut keys = Keys::app();
    let (mut dirty, mut first, mut restart) = (true, None, None);
    let code = loop {
        if let Some(now) = tty.size().filter(|now| *now != size) {
            size = now;
            terminal.backend_mut().resize(size);
            dirty = true;
        }
        if dirty || !app.queued.is_empty() {
            draw_frame(&mut terminal, |buf| {
                app.frame(buf);
                (app.layout.cursor, app.layout.links.clone())
            })?;
            app.frame_shown(Instant::now());
        }
        // A release's copy ships only once the frame showing it is out. An
        // ask waits for the frame that names its target; its outcome needs
        // another.
        dirty = settle(&mut app, terminal.backend_mut())?;
        // Held keys replay as soon as the frame above has produced more.
        let wait = if app.deferred.is_empty() {
            TICK
        } else {
            Duration::ZERO
        };
        if first.is_none() {
            first = match wakes.recv_timeout(wait) {
                Ok(wake) => Some(wake),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break 0,
            };
        }
        let Some(redraw) = drain(&mut app, &mut keys, &wakes, &mut first) else {
            break clipped(&mut app, terminal.backend_mut()).map(|()| 0)?;
        };
        dirty |= redraw | app.lapse(Instant::now());
        app.focus();
        if reader.is_finished() {
            break 1;
        }
        if let Some(ask) = app.pane_view.take() {
            app.deferred.clear();
            let verb = std::mem::take(&mut app.pane_verb);
            let line = paneview::run(&tty, &pause, &ask, verb);
            app.flash = Some((line, Instant::now()));
            app.resumed_at = Some(Instant::now());
            // A key half decoded before the hand-over is the pane's, not ours.
            keys = Keys::app();
            terminal.clear()?;
            dirty = true;
        }
        let mut outside = Real {
            tty: &tty,
            terminal: &mut terminal,
        };
        match step(&mut app, &mut outside, &mut upkeep, &typed, err)? {
            Step::Stay(redraw) => dirty |= redraw,
            Step::Respawn(version, respawn) => {
                restart = Some((version, respawn));
                break 0;
            }
            Step::Quit(code) => return Ok(code),
        }
    };
    end(tty, app, upkeep, code, restart, err)
}

/// The reader of an app for `home`, in the `own` server and pane tmux names,
/// told the version an exec `landed` this app in when that is not this one.
fn reader_of(
    root: &Path,
    home: Option<String>,
    own: (Option<ServerId>, Option<String>),
    landed: Option<String>,
) -> loader::Reader {
    let declared = doors::declared_server(crate::shape::current());
    let server = doors::launch_target(declared.as_ref());
    let mut reader = loader::Reader::new(root.to_path_buf(), home, server);
    let global = doors::config_file(crate::shape::current(), root);
    reader.settings_paths(loader::SettingsPaths {
        home: doors::home(),
        local: doors::local_config(&doors::cwd(), Some(&global)),
        global,
    });
    reader.own_pane(own.0, own.1).landed(landed);
    reader
}

/// What the view an exec restart handed on behind [`handover::FLAG`] gives
/// this process: the view it keeps, or the version it was meant to be.
fn handed_on(text: Option<String>) -> (Option<Keep>, Option<String>) {
    let now = Timestamp::now().epoch();
    match text.map(|text| upgrade::carried(&text, std::process::id(), crate::shape::VERSION, now)) {
        Some(upgrade::Carried::View(view, at)) => (Some(Keep { view, at }), None),
        Some(upgrade::Carried::Landed(version)) => (None, Some(version)),
        Some(upgrade::Carried::Ignored) | None => (None, None),
    }
}

/// What the loop does about the restart due now.
pub(super) enum Step {
    /// Nothing due, or an exec that failed and took the terminal back: go on,
    /// redrawing everything when it did.
    Stay(bool),
    /// End into the chat window's respawn.
    Respawn(String, toggle::Restart),
    /// The terminal was not taken back: end with this code.
    Quit(u8),
}

/// The restart due now, if any: a chat window ends into its respawn; any other
/// app replaces its process with the core the link names, handing on the
/// human's `typed` operands and the view, the terminal put back first exactly
/// as on quit — exec keeps the pid, so the shell that started it stays. An
/// exec returns only when it failed: the SAME terminal is taken back and the
/// app told why, so it never tries that version again.
pub(super) fn step(
    app: &mut App,
    outside: &mut dyn Outside,
    upkeep: &mut maintainer::Upkeep,
    typed: &[String],
    err: &mut impl Write,
) -> crate::error::Result<Step> {
    // A thread a failed exec owes starts here, on the first turn after the
    // stopped one ended.
    upkeep.tend();
    // A profile pick of the Config tab is written on a thread of its own.
    if let Some(pick) = app.take_orchestrator_pick() {
        upkeep.orchestrator(pick);
    }
    // `r` on the Statistics tab goes to the thread, or waits for one to run.
    let mut waits = false;
    if let Some(generation) = app.take_reset()
        && upkeep.reset(generation) == maintainer::Handed::Held
    {
        app.reset_waits(generation);
        waits = true;
    }
    let (version, link) = match app.restart_due(Instant::now()) {
        None => return Ok(Step::Stay(waits)),
        Some((version, upgrade::Via::Respawn(respawn))) => {
            return Ok(Step::Respawn(version, respawn));
        }
        Some((version, upgrade::Via::Exec { link })) => (version, link),
    };
    let pid = format!("%{}", std::process::id());
    let view = app.handover(&pid, &version, Timestamp::now().epoch());
    let view = view.map(|handover| handover.render());
    let argv = upgrade::exec_argv(&link, typed, view, &version, crate::shape::VERSION);
    // The store's upkeep stops before the process is replaced: an exec never
    // returns to stop it.
    upkeep.pause(maintainer::STOP_BOUND);
    outside.suspend();
    let why = outside.exec(&argv).to_string();
    if let Err(lost) = outside.resume() {
        let said = format!("ae app: could not restart into ae {version} - {why}; {lost}");
        writeln!(err, "{}", terminal_text(&said))?;
        return Ok(Step::Quit(crate::entry::EXIT_FAILED));
    }
    upkeep.resume();
    app.failed = Some((version, why));
    outside.clear()?;
    Ok(Step::Stay(true))
}

/// What a due restart touches outside the app: the terminal and the process.
pub(super) trait Outside {
    /// Give the terminal back before an exec.
    fn suspend(&mut self);
    /// Replace this process with `argv`: returns only when that failed.
    fn exec(&mut self, argv: &[String]) -> std::io::Error;
    /// Take the terminal back after a failed exec.
    ///
    /// # Errors
    ///
    /// The terminal could not be taken back, and why.
    fn resume(&mut self) -> Result<(), String>;
    /// Clear the frame, so the next one is drawn whole.
    ///
    /// # Errors
    ///
    /// The terminal could not be written.
    fn clear(&mut self) -> std::io::Result<()>;
}

/// The app's own terminal, and the real exec.
struct Real<'a, W: Write> {
    tty: &'a tty::Tty,
    terminal: &'a mut Terminal<backend::Ansi<W>>,
}

impl<W: Write> Outside for Real<'_, W> {
    fn suspend(&mut self) {
        self.tty.suspend();
    }

    fn exec(&mut self, argv: &[String]) -> std::io::Error {
        crate::run::exec_unchanged(argv)
    }

    fn resume(&mut self) -> Result<(), String> {
        self.tty.resume()
    }

    fn clear(&mut self) -> std::io::Result<()> {
        self.terminal.clear()
    }
}

/// `ae app`'s operands.
pub(super) struct Operands {
    /// The home session as typed.
    pub(super) session: Option<String>,
    /// [`OWN`] was given: an app start ae made itself.
    pub(super) own: bool,
    /// The view an exec restart handed on behind [`handover::FLAG`].
    pub(super) handed: Option<String>,
}

/// `ae app`'s operands: the home session, then [`OWN`] once, then the view an
/// exec restart handed on behind [`handover::FLAG`], last. `None` is a usage
/// error.
/// The orchestrator autostart for the owner's own app, once per invocation:
/// on a thread, so the app paints at once, and its one line arrives as a
/// flash. Success says nothing.
fn kick_orchestrator(root: &Path, wake: mpsc::Sender<Wake>) {
    use crate::orchestrator_autostart::{self as autostart, Verdict};
    if !autostart::claim_once() {
        return;
    }
    let kick = autostart::Kick {
        global: Some(doors::config_file(crate::shape::current(), root)),
        no_autostart: doors::no_autostart(),
    };
    let said = wake.clone();
    let spawned = std::thread::Builder::new()
        .name("ae-orchestrator-autostart".to_owned())
        .spawn(move || {
            let line = match autostart::verdict(&kick) {
                Verdict::Skip => None,
                Verdict::Note(line) => Some(line),
                Verdict::Start => autostart::start(),
            };
            if let Some(line) = line {
                let _ = wake.send(Wake::Notice(line));
            }
        });
    if spawned.is_err() {
        let line = autostart::failure_line("the app could not start its thread");
        let _ = said.send(Wake::Notice(line));
    }
}

pub(super) fn operands(tail: &[String]) -> Option<Operands> {
    let (session, rest) = match tail.split_first() {
        Some((name, rest)) if !name.starts_with('-') => (Some(name.clone()), rest),
        _ => (None, tail),
    };
    let (own, rest) = match rest.split_first() {
        Some((flag, more)) if flag == OWN => (true, more),
        _ => (false, rest),
    };
    let handed = match rest {
        [] => None,
        [flag, text] if flag == handover::FLAG => Some(text.clone()),
        _ => return None,
    };
    Some(Operands {
        session,
        own,
        handed,
    })
}

/// Whether a start that no marker or carrier calls ae's own says [`HINT`],
/// given whether it runs in a tmux pane and that pane's `@ae_console` stamp,
/// `None` when tmux did not answer ([`upgrade::via`] reads it the same way). A
/// stamped pane is a chat window, ae's own start; a silent tmux is doubt, and
/// doubt says nothing.
pub(super) fn says_hint(in_pane: bool, stamp: Option<&str>) -> bool {
    !in_pane || stamp == Some("")
}

/// How `ae app` ends, its terminal put back first: restarted into the core
/// the command link names when that was due, else with the line that says
/// how to start it again.
fn end(
    tty: tty::Tty,
    mut app: App,
    upkeep: maintainer::Upkeep,
    code: u8,
    restart: Option<(String, toggle::Restart)>,
    err: &mut impl Write,
) -> crate::error::Result<u8> {
    // The store's upkeep stops before anything else.
    upkeep.stop(maintainer::STOP_BOUND);
    drop(tty);
    app.finish_flight(err, FLIGHT_QUIT_BOUND)?;
    let in_tmux = app.server.is_some();
    if let Some((version, restart)) = restart {
        app.publish_view(&restart, &version);
        // The lease goes before the pane, so the app that replaces it can
        // write at once.
        drop(app);
        let Err(why) = toggle::respawn_own(&restart) else {
            return Ok(0);
        };
        let line = format!("ae app: could not restart into ae {version} - {why}");
        writeln!(err, "{}", terminal_text(&line))?;
        writeln!(err, "{}", exit_hint(in_tmux))?;
        return Ok(crate::entry::EXIT_FAILED);
    }
    if code != 0 {
        writeln!(err, "ae app: its background reader stopped")?;
    }
    writeln!(err, "{}", exit_hint(in_tmux))?;
    Ok(code)
}

/// The one plain line `ae app` leaves on the normal screen when it ends: how
/// to start it again. Outside tmux it names no tmux key.
#[must_use]
pub fn exit_hint(in_tmux: bool) -> &'static str {
    if in_tmux {
        "ae app closed - run: ae app (in a chat window, prefix h reopens it)"
    } else {
        "ae app closed - run: ae app"
    }
}

/// What a frame owes (`App::owed`), then the copy staged for the terminal
/// outside tmux: written once the frame is out, so no escape lands inside it.
pub(super) fn settle<W: Write>(
    app: &mut App,
    backend: &mut backend::Ansi<W>,
) -> std::io::Result<bool> {
    let dirty = app.owed();
    clipped(app, backend)?;
    Ok(dirty)
}

/// The copy staged for the terminal, written. A quit writes it too: a copy
/// accepted in the batch that quit is not lost with the app.
fn clipped<W: Write>(app: &mut App, backend: &mut backend::Ansi<W>) -> std::io::Result<()> {
    match app.clip.take() {
        Some(text) => backend.clipboard(&text),
        None => Ok(()),
    }
}

/// Take the held keys, then `first` and every wake already waiting behind
/// it, then the idle keys when none came: `Some(redraw)`, or `None` to quit.
/// Once an ask is queued nothing more is taken, `first` included: it stays in
/// place with the rest of the channel until the loop has delivered the ask.
pub(super) fn drain(
    app: &mut App,
    keys: &mut Keys,
    wakes: &mpsc::Receiver<Wake>,
    first: &mut Option<Wake>,
) -> Option<bool> {
    let mut held = std::mem::take(&mut app.deferred);
    // A key held from before a pane view was typed for the seat, not here.
    if let Some(resumed) = app.resumed_at {
        held.retain(|(_, origin)| paneview::keep_key(*origin, resumed));
    }
    let (mut redraw, mut fed) = (take_keys(app, held)?, false);
    while app.queued.is_empty()
        && !app.warning_unpainted()
        && app.pane_view.is_none()
        && let Some(wake) = first.take().or_else(|| wakes.try_recv().ok())
    {
        let keyed = match wake {
            Wake::Keys(stamp, bytes) => {
                if app
                    .resumed_at
                    .is_some_and(|resumed| !paneview::keep_key(stamp, resumed))
                {
                    continue;
                }
                crate::read_gate("@app-keys");
                fed = true;
                app.last_input = Some(stamp);
                // A lone Esc, a paste start or a dropped control byte decodes
                // to no key yet: the read itself is the human's input.
                if !bytes.is_empty() {
                    (app.keep, app.touched) = (None, true);
                }
                keys.feed(&bytes, stamp)
            }
            Wake::Answer(answer) => {
                app.answer(*answer);
                redraw = true;
                continue;
            }
            Wake::Notice(line) => {
                app.flash = Some((line, Instant::now()));
                redraw = true;
                continue;
            }
            Wake::StatsStopped(line) => {
                app.stats_upkeep_stopped(line);
                redraw = true;
                continue;
            }
            Wake::StatsReset {
                generation,
                outcome,
            } => {
                app.reset_landed(generation, outcome);
                redraw = true;
                continue;
            }
            Wake::OrchestratorPick {
                generation,
                outcome,
            } => {
                app.orchestrator_landed(generation, outcome);
                redraw = true;
                continue;
            }
            Wake::Closed => return None,
        };
        redraw |= take_keys(app, keyed)?;
    }
    if !fed && app.pane_view.is_none() && !app.warning_unpainted() {
        redraw |= take_keys(app, keys.idle(Instant::now()))?;
    }
    app.forget(keys.begun());
    Some(redraw)
}
