//! The pane view: an `ae app` running OUTSIDE tmux hands its terminal to a
//! tmux client attached to one seat's pane on the session's recorded server,
//! and takes it back when the human presses a return key or detaches.
//!
//! The client attaches a transient VIEW session grouped with the real one: it
//! shares the real session's windows and none of its options, so its one-line
//! bar names the session, the seat and the way back, and no other client's
//! bar or current window moves. It destroys itself once its client leaves,
//! ae destroys it by exact name on every way out, and a watcher destroys it as
//! soon as the real session is gone, so a stopped session's panes do not live
//! on in it.
//!
//! One view per session: the store's pane-view lock is taken before anything
//! attaches and held until the terminal is back. The attached client is marked
//! by two options on both sessions — the client, read by the return keys, and
//! this app's token, which every cleanup checks — and the seat window's active
//! pane, which the windows share, is put back on return only while the window
//! still shows what the view selected.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::tty::Tty;
use crate::console::Console;
use crate::console::needs::SeatRef;
use crate::console::term::{self, Pause};
use crate::{tmux, transport};

/// How long the stdin reader gets to park before the view is refused. Its
/// read returns within 100 ms in the app's raw mode.
const PARK_WITHIN: Duration = Duration::from_millis(500);

/// The longest token a view marker may carry for ae to name it in a command.
const TOKEN_CAP: usize = 64;

/// How often the watcher asks whether the real session still exists: the
/// bound on how long a stopped session's panes outlive it in a view.
const WATCH_EVERY: Duration = Duration::from_secs(1);

/// How long a detached view session must have existed before a later view
/// sweeps it as left behind; a younger one may be another app's, mid-attach.
const STALE_AFTER: Duration = Duration::from_mins(1);

/// Whether a key read at `stamp` reaches the app after a view that gave the
/// terminal back at `resumed`: every read from before that moment was meant
/// for the pane, or is lost to the hand-over, and is dropped.
#[must_use]
pub(super) fn keep_key(stamp: Instant, resumed: Instant) -> bool {
    stamp >= resumed
}

/// One drawn seat the human asked to see, as the app proved it from its own
/// records: the tmux proof is [`run`]'s, under the lock.
#[derive(Debug, Clone)]
pub(super) struct Ask {
    pub session: String,
    pub uuid: String,
    pub dir: PathBuf,
    pub seat: SeatRef,
    /// The name the hint row says.
    pub label: String,
}

/// What a session's view marker says of a new view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Marker {
    /// No view marks the session.
    Free,
    /// A marker whose client is gone: cleared while it still carries this token.
    Stale(String),
    /// A live client holds a view.
    Busy(String),
    /// A token ae would not have written.
    Unreadable,
}

/// The verdict on the marker `client` + `token`, given the `live` clients.
#[must_use]
pub(super) fn verdict(client: &str, token: &str, live: &[&str]) -> Marker {
    if client.is_empty() && token.is_empty() {
        return Marker::Free;
    }
    if !token_ok(token) {
        return Marker::Unreadable;
    }
    if !client.is_empty() && live.contains(&client) {
        return Marker::Busy(client.to_owned());
    }
    Marker::Stale(token.to_owned())
}

/// A token ae mints — digits and `-` — or the empty one a half-written marker
/// leaves; anything else is never put into a command.
fn token_ok(token: &str) -> bool {
    token.len() <= TOKEN_CAP
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'-')
}

/// This view's token: the app's pid and the moment, unique per view.
fn token() -> String {
    format!("{}-{}", std::process::id(), now_nanos())
}

/// Unix nanoseconds now, `0` before the epoch.
fn now_nanos() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_nanos()).unwrap_or(u64::MAX)
        })
}

/// Destroy every DETACHED view session older than [`STALE_AFTER`]: one an
/// app crashed out of before its client attached.
fn sweep(server: &crate::inventory::ServerId, census: &[tmux::ViewCensusRow]) {
    let now = now_nanos();
    for row in census {
        if !row.attached && tmux::view_session_expired(&row.name, now, STALE_AFTER) {
            let _ = transport::destroy_view(server, &row.name);
        }
    }
}

/// The watcher of one view: while the human views, it destroys the view
/// session — exactly `view`, the one this app minted — once a census proves
/// the real session `session_id` gone. Stopped and joined on every way out.
struct Watch {
    stop: Arc<AtomicBool>,
    ended: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Watch {
    fn start(server: &crate::inventory::ServerId, session_id: &str, view: &str) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let ended = Arc::new(AtomicBool::new(false));
        let (server, session_id, view) = (server.clone(), session_id.to_owned(), view.to_owned());
        let (stopping, ending) = (Arc::clone(&stop), Arc::clone(&ended));
        let thread = std::thread::spawn(move || {
            const STEP: Duration = Duration::from_millis(50);
            let mut waited = Duration::ZERO;
            while !stopping.load(Ordering::Acquire) {
                std::thread::sleep(STEP);
                waited += STEP;
                if waited < WATCH_EVERY {
                    continue;
                }
                waited = Duration::ZERO;
                // Only a census tmux answered proves the session gone.
                let gone = transport::observe_view_census(&server)
                    .is_some_and(|rows| !rows.iter().any(|row| row.id == session_id));
                if gone {
                    ending.store(true, Ordering::Release);
                    let _ = transport::destroy_view(&server, &view);
                    return;
                }
            }
        });
        Self {
            stop,
            ended,
            thread: Some(thread),
        }
    }

    /// Stop and join the watcher; whether it saw the real session end.
    fn finish(mut self) -> bool {
        self.join();
        self.ended.load(Ordering::Acquire)
    }

    fn join(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.join();
    }
}

/// The view session, destroyed by its exact name when ae lets go of it —
/// whatever path that is. Gone already is no failure.
struct Owned<'a> {
    server: &'a crate::inventory::ServerId,
    view: String,
}

impl Drop for Owned<'_> {
    fn drop(&mut self) {
        let _ = transport::destroy_view(self.server, &self.view);
    }
}

/// The terminal handed to another program, given back on every way out:
/// [`Handed::finish`] reports, `Drop` covers every early return.
struct Handed<'a> {
    tty: &'a Tty,
    pause: &'a Pause,
    open: bool,
}

impl<'a> Handed<'a> {
    /// Park the reader, then leave the app's screen. `None` when the reader
    /// did not park in time — it reads on, and nothing was handed.
    fn take(tty: &'a Tty, pause: &'a Pause) -> Option<Self> {
        if !pause.hold(PARK_WITHIN) {
            return None;
        }
        tty.hand_over();
        Some(Self {
            tty,
            pause,
            open: true,
        })
    }

    /// The app's terminal back and the reader reading; why it failed.
    fn finish(mut self) -> Result<(), String> {
        self.open = false;
        let back = self.tty.take_back();
        self.pause.release();
        back
    }
}

impl Drop for Handed<'_> {
    fn drop(&mut self) {
        if self.open {
            let _ = self.tty.take_back();
            self.pause.release();
        }
    }
}

/// Show `ask`'s pane, interactive, until the human returns: the line the hint
/// row says afterwards, refused or closed.
pub(super) fn run(tty: &Tty, pause: &Pause, ask: &Ask) -> String {
    match view(tty, pause, ask) {
        Ok(line) | Err(line) => line,
    }
}

/// Whether ae may write on `server`, which `session` runs on: the return
/// keys are root binds, never written on the user's own tmux.
fn owned(server: &crate::inventory::ServerId, session: &str) -> Result<(), String> {
    match server {
        crate::inventory::ServerId::Ambient => Err(format!(
            "pane view needs an ae-owned tmux server; {session} runs on your own tmux"
        )),
        crate::inventory::ServerId::Selected(_) => Ok(()),
    }
}

/// Make `session_id`'s view marker free for a new view: a stale one is
/// cleared while it still carries the token it was judged by; why not.
fn free(server: &crate::inventory::ServerId, id: &str, session: &str) -> Result<(), String> {
    let unread = |what: &str| format!("tmux did not answer for {what}");
    let (client, seen, _) =
        transport::observe_view_marker(server, id).ok_or_else(|| unread("the view marker"))?;
    let clients = transport::observe_clients(server).ok_or_else(|| unread("its clients"))?;
    // A view is held only by a client still IN the view its token names; one
    // that left it another way, or moved to another view, holds nothing.
    let owned = tmux::view_session_name(&seen);
    let live: Vec<&str> = (clients.iter())
        .filter(|client| client.session == owned)
        .map(|client| client.name.as_str())
        .collect();
    match verdict(&client, &seen, &live) {
        Marker::Free => Ok(()),
        Marker::Stale(token) => {
            let clear = tmux::view_stale_clear_command(id, &token);
            transport::run_view_command(server, id, &clear)
                .then_some(())
                .ok_or_else(|| "tmux refused to clear a stale view marker".to_owned())
        }
        Marker::Busy(_) => Err(format!("another ae app is viewing {session}")),
        Marker::Unreadable => Err(format!("the view marker on {session} is not ae's")),
    }
}

/// The view lock of the session recorded in `dir`, or why not.
fn lock(dir: &std::path::Path, session: &str) -> Result<std::fs::File, String> {
    crate::store::open(dir).pane_view().map_err(|err| {
        if err.kind() == std::io::ErrorKind::WouldBlock {
            format!("another ae app is viewing {session}")
        } else {
            format!("the view lock: {err}")
        }
    })
}

fn view(tty: &Tty, pause: &Pause, ask: &Ask) -> Result<String, String> {
    let label = &ask.label;
    let refused = |why: &str| format!("refused: view {label}: {why}");
    let session = &ask.session;
    let _lock = lock(&ask.dir, session).map_err(|why| refused(&why))?;
    let Some(server) = crate::session_launch::recorded_server_resolved(&ask.dir) else {
        return Err(refused(&format!(
            "ae cannot say which tmux server {session} is on"
        )));
    };
    owned(&server, session).map_err(|why| refused(&why))?;
    let console = Console::bound(session.clone(), ask.dir.clone(), ask.uuid.clone());
    let target =
        term::prove_open(&server, &console, &ask.seat).map_err(|why| refused(&why.why()))?;
    let id = &target.session_id;
    let unread = |what: &str| refused(&format!("tmux did not answer for {what}"));
    free(&server, id, session).map_err(|why| refused(&why))?;
    let (window, _) =
        transport::observe_view_place(&server, &target.pane).ok_or_else(|| unread("the seat"))?;
    let (_, prev_pane) = transport::observe_view_place(&server, &format!("{id}:{window}"))
        .ok_or_else(|| unread("the seat's window"))?;
    let census = transport::observe_view_census(&server).ok_or_else(|| unread("its sessions"))?;
    sweep(&server, &census);
    if !transport::bind_view_return(&server) {
        return Err(refused("tmux refused the return keys"));
    }
    let token = token();
    let view = tmux::view_session_name(&token);
    // Owned before the verdict: a chain tmux refused part-way still created
    // the session, and its drop destroys exactly that name.
    let made = transport::make_view(&server, id, &view, session, label, &ask.uuid);
    let owned = Owned {
        server: &server,
        view,
    };
    if !made {
        return Err(refused("tmux refused the view session"));
    }
    let Some(handed) = Handed::take(tty, pause) else {
        return Err(refused("the app's input did not pause"));
    };
    let watch = Watch::start(&server, id, &owned.view);
    let code = transport::view_pane(&server, id, &owned.view, &target.pane, &token);
    let ended = watch.finish();
    drop(owned);
    let release = tmux::view_release_command(id, &token);
    let _ = transport::run_view_command(&server, id, &release);
    if !ended {
        let restore =
            tmux::view_restore_command(id, &target.uuid, &window, &target.pane, &prev_pane);
        let _ = transport::run_view_command(&server, id, &restore);
    }
    handed
        .finish()
        .map_err(|why| format!("back from {label}, but the terminal: {why}"))?;
    Ok(match code {
        _ if ended => format!("back from {label}: {session} ended while viewed"),
        Some(0) => format!("back from {label}"),
        Some(code) => format!("back from {label}: tmux ended with {code}"),
        None => format!("back from {label}: tmux did not start, or was stopped by a signal"),
    })
}

/// The one client that pressed `/open` inside tmux, and the app pane it showed.
pub(super) struct Viewer<'a> {
    pub(super) client: &'a str,
    pub(super) app_pane: &'a str,
}

/// Inside tmux, `/open`: move the one proven `viewer` client — the one
/// showing this app's pane — into a clean view of `target`, and leave it
/// there; F12 or C-] runs the return ae captured. `uuid` is the incarnation
/// the open proved. The line to tell.
pub(super) fn open_inside(
    server: &crate::inventory::ServerId,
    dir: &std::path::Path,
    target: &crate::console::open::Target,
    viewer: &Viewer<'_>,
    session: &str,
    label: &str,
    uuid: &str,
) -> String {
    match inside(server, dir, target, viewer, session, label, uuid) {
        Ok(line) | Err(line) => line,
    }
}

fn inside(
    server: &crate::inventory::ServerId,
    dir: &std::path::Path,
    target: &crate::console::open::Target,
    viewer: &Viewer<'_>,
    session: &str,
    label: &str,
    uuid: &str,
) -> Result<String, String> {
    let Viewer { client, app_pane } = *viewer;
    let refused = |why: &str| format!("refused: /open {label}: {why}");
    let _lock = lock(dir, session).map_err(|why| refused(&why))?;
    owned(server, session).map_err(|why| refused(&why))?;
    let id = &target.session_id;
    free(server, id, session).map_err(|why| refused(&why))?;
    let unread = |what: &str| refused(&format!("tmux did not answer for {what}"));
    let (app_session, app_window, shown) = transport::observe_client_place(server, client)
        .ok_or_else(|| unread("the app's client"))?;
    if shown != app_pane {
        return Err(refused("the client left the app"));
    }
    let census = transport::observe_view_census(server).ok_or_else(|| unread("its sessions"))?;
    sweep(server, &census);
    if !transport::bind_view_return(server) {
        return Err(refused("tmux refused the return keys"));
    }
    let token = token();
    let view = tmux::view_session_name(&token);
    let ret = tmux::view_return_command(&app_session, &app_window, app_pane, id, &token)
        .ok_or_else(|| refused("a tmux id failed its grammar"))?;
    if !transport::make_view(server, id, &view, session, label, uuid) {
        // A chain tmux refused part-way still created the session; no client
        // shows it yet, so destroying exactly that name detaches nobody.
        let _ = transport::destroy_view(server, &view);
        return Err(refused("tmux refused the view session"));
    }
    if !transport::switch_view(server, id, &view, &target.pane, client, &token, &ret) {
        // Back first: destroying a session its client still shows would
        // detach the human's terminal from tmux.
        let _ = transport::switch_client(server, client, &app_session);
        let _ = transport::destroy_view(server, &view);
        let release = tmux::view_release_command(id, &token);
        let _ = transport::run_view_command(server, id, &release);
        return Err(refused("tmux refused the view"));
    }
    watch_inside(server, id, &view, client, &app_session);
    Ok(format!("viewing {label} · F12 or C-] back to ae app"))
}

/// The watcher of an inside view, detached: it ends once the view session is
/// gone, and when the real session `session_id` goes first it takes `client`
/// back to `app_session` before destroying exactly `view`.
fn watch_inside(
    server: &crate::inventory::ServerId,
    session_id: &str,
    view: &str,
    client: &str,
    app_session: &str,
) {
    let (server, session_id, view) = (server.clone(), session_id.to_owned(), view.to_owned());
    let (client, app_session) = (client.to_owned(), app_session.to_owned());
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(WATCH_EVERY);
            let Some(rows) = transport::observe_view_census(&server) else {
                continue;
            };
            if !rows.iter().any(|row| row.name == view) {
                return;
            }
            if !rows.iter().any(|row| row.id == session_id) {
                let _ = transport::switch_client(&server, &client, &app_session);
                let _ = transport::destroy_view(&server, &view);
                return;
            }
        }
    });
}

#[cfg(test)]
#[path = "nativeclean_spec.rs"]
mod nativeclean_spec;

#[cfg(test)]
mod tests {
    use super::{Marker, owned, verdict};

    #[test]
    fn a_marker_names_a_view_only_while_its_client_is_attached() {
        assert_eq!(verdict("", "", &["/dev/ttys1"]), Marker::Free);
        assert_eq!(
            verdict("/dev/ttys1", "12-34", &["/dev/ttys1"]),
            Marker::Busy("/dev/ttys1".to_owned())
        );
        assert_eq!(
            verdict("/dev/ttys1", "12-34", &["/dev/ttys2"]),
            Marker::Stale("12-34".to_owned())
        );
        assert_eq!(verdict("/dev/ttys1", "", &[]), Marker::Stale(String::new()));
        assert_eq!(verdict("/dev/ttys1", "1'2", &[]), Marker::Unreadable);
    }

    #[test]
    fn the_users_own_tmux_is_refused_by_name_and_an_ae_server_is_not() {
        use crate::inventory::ServerId;
        assert_eq!(
            owned(&ServerId::Ambient, "pvamb"),
            Err("pane view needs an ae-owned tmux server; pvamb runs on your own tmux".to_owned())
        );
        let ae = ServerId::Selected(crate::meta::Selector::Name("ae".to_owned()));
        assert_eq!(owned(&ae, "pvdemo"), Ok(()));
    }
}
