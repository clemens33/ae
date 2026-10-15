//! The pane view: an `ae app` running OUTSIDE tmux hands its terminal to a
//! tmux client attached to one seat's pane on the session's recorded server,
//! and takes it back when the human presses a return key or detaches.
//!
//! One view per session: the store's pane-view lock is taken before anything
//! attaches and held until the terminal is back. The attached client is marked
//! by two session options — the client, read by the return keys, and this
//! app's token, which every cleanup checks — and the session's current window
//! and the seat window's active pane are put back on return only while the
//! session still shows what the view selected.

use std::path::PathBuf;
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
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    format!("{}-{nanos}", std::process::id())
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

fn view(tty: &Tty, pause: &Pause, ask: &Ask) -> Result<String, String> {
    let label = &ask.label;
    let refused = |why: &str| format!("refused: view {label}: {why}");
    let session = &ask.session;
    let _lock = crate::store::open(&ask.dir).pane_view().map_err(|err| {
        if err.kind() == std::io::ErrorKind::WouldBlock {
            refused(&format!("another ae app is viewing {session}"))
        } else {
            refused(&format!("the view lock: {err}"))
        }
    })?;
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
    let (client, seen, prev_window) =
        transport::observe_view_marker(&server, id).ok_or_else(|| unread("the view marker"))?;
    let clients = transport::observe_clients(&server).ok_or_else(|| unread("its clients"))?;
    let live: Vec<&str> = clients.iter().map(|client| client.name.as_str()).collect();
    match verdict(&client, &seen, &live) {
        Marker::Free => {}
        Marker::Stale(token) => {
            let clear = tmux::view_stale_clear_command(id, &token);
            if !transport::run_view_command(&server, id, &clear) {
                return Err(refused("tmux refused to clear a stale view marker"));
            }
        }
        Marker::Busy(_) => return Err(refused(&format!("another ae app is viewing {session}"))),
        Marker::Unreadable => {
            return Err(refused(&format!(
                "the view marker on {session} is not ae's"
            )));
        }
    }
    let (window, _) =
        transport::observe_view_place(&server, &target.pane).ok_or_else(|| unread("the seat"))?;
    let (_, prev_pane) = transport::observe_view_place(&server, &format!("{id}:{window}"))
        .ok_or_else(|| unread("the seat's window"))?;
    if !transport::bind_view_return(&server) {
        return Err(refused("tmux refused the return keys"));
    }
    let token = token();
    let Some(handed) = Handed::take(tty, pause) else {
        return Err(refused("the app's input did not pause"));
    };
    let code = transport::view_pane(&server, id, &target.pane, &token);
    let release = tmux::view_release_command(id, &token);
    let _ = transport::run_view_command(&server, id, &release);
    let restore = tmux::view_restore_command(
        id,
        &target.uuid,
        &window,
        &target.pane,
        &prev_window,
        &prev_pane,
    );
    let _ = transport::run_view_command(&server, id, &restore);
    handed
        .finish()
        .map_err(|why| format!("back from {label}, but the terminal: {why}"))?;
    Ok(match code {
        Some(0) => format!("back from {label}"),
        Some(code) => format!("back from {label}: tmux ended with {code}"),
        None => format!("back from {label}: tmux did not start, or was stopped by a signal"),
    })
}

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
