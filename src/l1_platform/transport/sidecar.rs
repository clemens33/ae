//! ae's dictation sidecar: one tmux control client on the proved session.
//!
//! It writes one command line at a time and reads the reply blocks back. Every
//! failure is terminal: the client is killed and reaped at once, and a sidecar
//! that failed sends nothing more. The pane text in a reply is hostile, so the
//! reader parses it with [`ControlParser`]'s bounds and never decodes a row.

use std::io::{Read as _, Write as _};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

// The server id and the tmux grammar come through transport's own imports:
// the sidecar is part of that door and adds no edge of its own.
use super::tmux::control::{
    self, ControlEvent, ControlParser, Expect, HookRefusal, PaneFacts, Proven, SidecarCommand,
};
use super::{Out, PROGRAM, Ran, ServerId, note_child_reaped, spawn_sidecar, spawn_until};

/// Why a sidecar is gone, or never came.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidecarLost {
    /// The client could not start or attach.
    Start,
    /// The budget passed first.
    Timeout,
    /// The stream ended, tmux exited, or the sidecar was used out of turn.
    Stream,
    /// The stream broke the reply grammar.
    Malformed,
    /// A line or a body passed its bound.
    Overflow,
    /// The session's hooks would fire for the sidecar.
    Hooks(HookRefusal),
}

/// The answer to one command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidecarReply {
    /// Both Space blocks ended `%end` with no body.
    Spaced,
    /// The proved pane, still in the proved session with the proved process,
    /// and its screen.
    Frame { facts: PaneFacts, capture: String },
    /// An `%error`, a payload (`ae-pane-changed` among them), or facts of
    /// another pane, session or process.
    Refused(String),
}

/// How long a closed client gets to exit by itself before it is killed.
const CLOSE_GRACE: Duration = Duration::from_millis(500);

/// How often a closing client is looked at.
const CLOSE_POLL: Duration = Duration::from_millis(10);

/// The most a hook read may print.
const HOOK_READ_CAP: u64 = 64 * 1024;

/// The client, its pipe ends and its reader, all owned.
#[derive(Debug)]
struct Live {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    events: mpsc::Receiver<ControlEvent>,
    expects: mpsc::Sender<Expect>,
    reader: std::thread::JoinHandle<()>,
    /// The owner's handle on the reader's socket end: shut down for reading,
    /// it ends the reader's read whoever else holds the client's stdout.
    wake: UnixStream,
    /// Set by the reader before it forwards a batch that holds a terminal
    /// event, so no key follows output already known to be fatal.
    doomed: Arc<AtomicBool>,
}

/// One owned control client. See the module documentation.
#[derive(Debug)]
pub struct Sidecar {
    live: Option<Live>,
    proven: Proven,
    pid: u32,
    pending: Option<SidecarCommand>,
    /// A send was refused: the next reply, or the drop, reaps the client.
    broken: bool,
}

impl Sidecar {
    /// Read the session's hooks, then spawn `tmux -u <server> -C
    /// attach-session -E -f no-output,ignore-size -t =<session>` with `TERM`
    /// naming the sidecar, and wait for the attach block, all within
    /// `budget`. A refusal or a miss before the spawn starts no control
    /// client; any failure after it kills and reaps the client first.
    ///
    /// # Errors
    ///
    /// [`SidecarLost::Hooks`] for hooks that would fire for the sidecar, and
    /// the other variants for a client that could not attach in time.
    pub fn attach(
        server: &ServerId,
        proven: &Proven,
        budget: Duration,
    ) -> Result<Self, SidecarLost> {
        let start = Instant::now();
        if !super::addressable(server) {
            return Err(SidecarLost::Start);
        }
        eligible(server, proven, budget)?;
        let left = budget.saturating_sub(start.elapsed());
        if left.is_zero() {
            return Err(SidecarLost::Timeout);
        }
        let (mut child, wake) = spawn_sidecar(&control::sidecar_attach_args(server, proven))
            .ok_or(SidecarLost::Start)?;
        let pid = child.id();
        let (Some(stdin), Ok(stdout)) = (child.stdin.take(), wake.try_clone()) else {
            reap(&mut child, true);
            return Err(SidecarLost::Start);
        };
        let (expects, queued) = mpsc::channel();
        let (sent, events) = mpsc::sync_channel(4);
        let doomed = Arc::new(AtomicBool::new(false));
        let latch = Arc::clone(&doomed);
        let reader = std::thread::Builder::new()
            .name("ae-dictation-reader".to_owned())
            .spawn(move || read_replies(stdout, &queued, &sent, &latch));
        let Ok(reader) = reader else {
            reap(&mut child, true);
            return Err(SidecarLost::Start);
        };
        let mut sidecar = Self {
            live: Some(Live {
                child,
                stdin,
                events,
                expects,
                reader,
                wake,
                doomed,
            }),
            proven: proven.clone(),
            pid,
            pending: None,
            broken: false,
        };
        match sidecar.next_event(start + budget) {
            Ok(ControlEvent::Attached) => Ok(sidecar),
            Ok(event) => {
                sidecar.end();
                Err(lost_by(&event))
            }
            Err(why) => Err(why),
        }
    }

    /// The client's pid.
    #[must_use]
    pub const fn pid(&self) -> u32 {
        self.pid
    }

    /// Queue `command`'s reply blocks, then write its one line. It never waits
    /// for the reply and never reaps: a caller may hold its dispatch lock
    /// around it. False when the sidecar failed before, when a command still
    /// waits for its reply, when the reader already saw fatal output, or when
    /// the write failed; the sidecar then sends nothing more, and the next
    /// reply or the drop reaps it.
    pub fn send(&mut self, command: SidecarCommand) -> bool {
        let Some(live) = self.live.as_mut() else {
            return false;
        };
        if self.broken || self.pending.is_some() || live.doomed.load(Ordering::Acquire) {
            self.broken = true;
            return false;
        }
        let text = control::command_line(&self.proven, command);
        let queued = Expect::for_command(command)
            .into_iter()
            .all(|block| live.expects.send(block).is_ok());
        if queued && live.stdin.write_all(text.as_bytes()).is_ok() && live.stdin.flush().is_ok() {
            self.pending = Some(command);
            true
        } else {
            self.broken = true;
            false
        }
    }

    /// Whether the sidecar can send nothing more: it failed, was refused or
    /// misused, or its reader saw fatal output.
    #[must_use]
    pub fn lost(&self) -> bool {
        self.broken
            || self
                .live
                .as_ref()
                .is_none_or(|live| live.doomed.load(Ordering::Acquire))
    }

    /// The reply to the command sent last, within `deadline`. Any reply but
    /// `Spaced` or a proved `Frame`, and any error, reaps the client first.
    ///
    /// # Errors
    ///
    /// [`SidecarLost::Stream`] when nothing was sent or the sidecar failed
    /// before, and the cause of a failure while it waited.
    pub fn reply(&mut self, deadline: Duration) -> Result<SidecarReply, SidecarLost> {
        let until = Instant::now() + deadline;
        let command = self.pending.take();
        let (false, Some(command)) = (self.broken, command) else {
            self.end();
            return Err(SidecarLost::Stream);
        };
        let reply = match command {
            SidecarCommand::Space => self.spaced(until),
            SidecarCommand::Frame => self.frame(until),
        };
        if !matches!(reply, Ok(SidecarReply::Spaced | SidecarReply::Frame { .. })) {
            self.end();
        }
        reply
    }

    /// Close stdin, give the client [`CLOSE_GRACE`] to exit, then kill it;
    /// it is always reaped. True when it exited by itself.
    #[must_use]
    pub fn close(mut self) -> bool {
        let Some(Live {
            mut child,
            stdin,
            events,
            expects,
            reader,
            wake,
            ..
        }) = self.live.take()
        else {
            return false;
        };
        drop(stdin);
        drop(expects);
        let grace = Instant::now() + CLOSE_GRACE;
        let mut exited = None;
        while Instant::now() < grace {
            match child.try_wait() {
                Ok(Some(status)) => {
                    exited = Some(status);
                    break;
                }
                Ok(None) => std::thread::sleep(CLOSE_POLL),
                Err(_) => break,
            }
        }
        drop(events);
        let by_itself = if let Some(status) = exited {
            note_child_reaped(child.id(), status);
            true
        } else {
            reap(&mut child, true);
            false
        };
        let _ = wake.shutdown(Shutdown::Read);
        let _ = reader.join();
        by_itself
    }

    /// Both empty blocks of a Space.
    fn spaced(&mut self, until: Instant) -> Result<SidecarReply, SidecarLost> {
        for _ in 0..2 {
            match self.next_event(until)? {
                ControlEvent::Empty => {}
                ControlEvent::Refused(text) => return Ok(SidecarReply::Refused(text)),
                event => return Err(lost_by(&event)),
            }
        }
        Ok(SidecarReply::Spaced)
    }

    /// The facts, checked against the proof, then the capture.
    fn frame(&mut self, until: Instant) -> Result<SidecarReply, SidecarLost> {
        let facts = match self.next_event(until)? {
            ControlEvent::Facts(facts) if self.proven.matches(&facts) => facts,
            ControlEvent::Facts(facts) => {
                return Ok(SidecarReply::Refused(format!(
                    "{} {} {}",
                    facts.session_id, facts.pane_id, facts.pane_pid
                )));
            }
            ControlEvent::Refused(text) => return Ok(SidecarReply::Refused(text)),
            event => return Err(lost_by(&event)),
        };
        match self.next_event(until)? {
            ControlEvent::Capture(capture) => Ok(SidecarReply::Frame { facts, capture }),
            ControlEvent::Refused(text) => Ok(SidecarReply::Refused(text)),
            event => Err(lost_by(&event)),
        }
    }

    /// The reader's next event before `until`; a miss or a closed stream
    /// reaps the client first.
    fn next_event(&mut self, until: Instant) -> Result<ControlEvent, SidecarLost> {
        let Some(live) = self.live.as_ref() else {
            return Err(SidecarLost::Stream);
        };
        // A queued event still arrives on a zero timeout: a passed deadline
        // is checked before the receive, and again before the event counts.
        let left = until.saturating_duration_since(Instant::now());
        let received = if left.is_zero() {
            Err(mpsc::RecvTimeoutError::Timeout)
        } else {
            live.events.recv_timeout(left)
        };
        match received {
            Ok(event) if Instant::now() < until => Ok(event),
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {
                self.end();
                Err(SidecarLost::Timeout)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.end();
                Err(SidecarLost::Stream)
            }
        }
    }

    /// Kill and reap the client now, if it still lives. Every later call
    /// fails.
    fn end(&mut self) {
        self.pending = None;
        if let Some(Live {
            mut child,
            stdin,
            events,
            expects,
            reader,
            wake,
            ..
        }) = self.live.take()
        {
            let _ = child.kill();
            let _ = wake.shutdown(Shutdown::Read);
            drop((stdin, expects, events));
            reap(&mut child, false);
            let _ = reader.join();
        }
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        self.end();
    }
}

/// The error a terminal or unexpected event means.
const fn lost_by(event: &ControlEvent) -> SidecarLost {
    match event {
        ControlEvent::Overflow => SidecarLost::Overflow,
        ControlEvent::Exit => SidecarLost::Stream,
        ControlEvent::Refused(_) => SidecarLost::Start,
        ControlEvent::Malformed
        | ControlEvent::Attached
        | ControlEvent::Empty
        | ControlEvent::Facts(_)
        | ControlEvent::Capture(_) => SidecarLost::Malformed,
    }
}

/// Kill `child` when `kill` asks for it, then wait for it, and report the
/// reap to the test observers.
fn reap(child: &mut std::process::Child, kill: bool) {
    if kill {
        let _ = child.kill();
    }
    if let Ok(status) = child.wait() {
        note_child_reaped(child.id(), status);
    }
}

/// The hook preflight: ONE tmux client reads every watched hook within
/// `budget`, before any control client exists.
fn eligible(server: &ServerId, proven: &Proven, budget: Duration) -> Result<(), SidecarLost> {
    let args = control::sidecar_hook_args(server, proven);
    let stdout = match spawn_until(
        PROGRAM,
        &args,
        &[],
        HOOK_READ_CAP,
        budget,
        Out::Tagged("hooks"),
        None,
    ) {
        Ran::Exited(status, stdout) if status.success() => stdout,
        Ran::TimedOut => return Err(SidecarLost::Timeout),
        Ran::Absent => return Err(SidecarLost::Start),
        Ran::Exited(..) | Ran::Over | Ran::Failed => {
            return Err(SidecarLost::Hooks(HookRefusal::Unread));
        }
    };
    let text = String::from_utf8(stdout).map_err(|_| SidecarLost::Hooks(HookRefusal::Unread))?;
    let read =
        control::interpret_sidecar_hooks(&text).ok_or(SidecarLost::Hooks(HookRefusal::Unread))?;
    for (name, session, global) in read {
        control::hooks_eligible(name, &session, &global).map_err(SidecarLost::Hooks)?;
    }
    Ok(())
}

/// The reader thread: the client's stdout through the parser, each event to
/// the owner in order. The expectations queued before a command line was
/// written are taken in before the bytes that answer it are parsed.
///
/// Before it forwards a batch that holds a terminal event, and before the
/// stream's end, it sets `doomed`, so the owner writes no key behind output
/// already known to be fatal. It stops after forwarding that event.
fn read_replies(
    mut stdout: UnixStream,
    queued: &mpsc::Receiver<Expect>,
    sent: &mpsc::SyncSender<ControlEvent>,
    doomed: &AtomicBool,
) {
    let mut parser = ControlParser::new();
    let mut chunk = [0_u8; 8192];
    let forward = |events: Vec<ControlEvent>| {
        let fatal = events.iter().any(ControlEvent::is_terminal);
        if fatal {
            doomed.store(true, Ordering::Release);
        }
        events.into_iter().all(|event| sent.send(event).is_ok()) && !fatal
    };
    loop {
        let read = match stdout.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => read,
            Err(why) if why.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        while let Ok(next) = queued.try_recv() {
            parser.expect(next);
        }
        if !forward(parser.feed(chunk.get(..read).unwrap_or_default())) {
            return;
        }
    }
    doomed.store(true, Ordering::Release);
    while let Ok(next) = queued.try_recv() {
        parser.expect(next);
    }
    forward(parser.finish());
}

#[cfg(test)]
#[path = "../../../tests/fixtures/harness-state/d54-sidecar-tmux.rs"]
mod acceptance;
