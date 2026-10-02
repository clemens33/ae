//! The console's terminal: raw reads on a thread, the ownership reading, and
//! each act carried out. What any of it means is `console::input`'s to say.

use std::io::{IsTerminal as _, Read as _};
use std::os::fd::AsFd as _;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::time::Instant;

use super::input::{self, Effect, Input, Reading};
use super::lane::Seat;
use super::{Console, submit};
use crate::inventory::ServerId;
use crate::{doors, theme, time, tracked, transport};

/// Stamped terminal reads, until the terminal's input ends.
struct Reads(Option<Receiver<(Instant, Vec<u8>)>>);

/// What a wait for a read came to.
#[derive(Debug, PartialEq, Eq)]
enum Got {
    Read(Instant, Vec<u8>),
    /// The input just ended; every later wait sleeps to its deadline.
    Ended,
    Due,
}

impl Reads {
    fn wait(&mut self, deadline: Instant) -> Got {
        let left = deadline.saturating_duration_since(Instant::now());
        let Some(reads) = &self.0 else {
            std::thread::sleep(left);
            return Got::Due;
        };
        match reads.recv_timeout(left) {
            Ok((stamp, bytes)) => Got::Read(stamp, bytes),
            Err(RecvTimeoutError::Timeout) => Got::Due,
            Err(RecvTimeoutError::Disconnected) => {
                self.0 = None;
                Got::Ended
            }
        }
    }
}

/// Read `stdin` unbuffered, stamping each read when it completes and before the
/// send that may block on a full channel: a stamp is when the read returned,
/// never when the send got through, and never how long the kernel held input.
fn reader(mut stdin: std::fs::File, reads: &SyncSender<(Instant, Vec<u8>)>) {
    let mut buffer = [0; 4096];
    loop {
        let read = stdin.read(&mut buffer);
        let stamp = Instant::now();
        match read {
            Ok(0) => return,
            Ok(n) => {
                if reads.send((stamp, buffer[..n].to_vec())).is_err() {
                    return;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return,
        }
    }
}

/// The lead pair a console asks, main seat first: exactly one seat in slot
/// `main`, and every name one an agent name may be — refused by name, never
/// dropped. The meta keeps a name verbatim, and an ask would read `%3` as a
/// pane or `--cross-session` as a flag.
fn pair_of(mut seats: Vec<Seat>) -> Result<Vec<Seat>, String> {
    if seats.iter().filter(|seat| seat.slot == "main").count() != 1 {
        return Err("the meta names no single main seat".to_owned());
    }
    let named = |seat: &&Seat| crate::config::is_agent_name(&seat.name);
    if let Some(bad) = seats.iter().find(|seat| !named(seat)) {
        return Err(format!("the {} seat is not an agent name", bad.slot));
    }
    seats.sort_by_key(|seat| seat.slot != "main");
    Ok(seats)
}

/// Whether `pair`, slots and names, is still the session's lead pair.
fn still(pair: &[Seat], seats: Result<Vec<Seat>, String>) -> Result<(), String> {
    if pair_of(seats?)? == pair {
        return Ok(());
    }
    Err("the lead pair changed since this console opened - restart this console".to_owned())
}

/// A console that takes input from its terminal.
pub(super) struct Term {
    input: Input,
    pair: Vec<Seat>,
    reads: Reads,
    server: Option<ServerId>,
    /// This console's own pane.
    me: Option<String>,
}

impl Term {
    /// Input on this process's terminal for `console`'s lead pair. `Err` is
    /// the line saying why there is none: no usable pair, or no terminal.
    pub(super) fn start(console: &Console) -> Result<Self, String> {
        let off = |why: &str| format!("input off: {why}; this console only reads\n");
        let pair = console.seats().and_then(pair_of).map_err(|why| off(&why))?;
        let stdin = std::io::stdin();
        let owned = stdin
            .is_terminal()
            .then(|| stdin.as_fd().try_clone_to_owned());
        let Some(Ok(owned)) = owned else {
            return Err(off("stdin is not a terminal"));
        };
        let (send, reads) = sync_channel(8);
        let file = std::fs::File::from(owned);
        std::thread::spawn(move || reader(file, &send));
        let declared = doors::declared_server(crate::shape::current());
        Ok(Self {
            input: Input::new(pair.iter().map(|seat| seat.name.clone()).collect()),
            pair,
            reads: Reads(Some(reads)),
            server: doors::launch_target(declared.as_ref()),
            me: doors::calling_pane_id(),
        })
    }

    /// Whether this console owns its session's input: `None` when tmux did
    /// not answer.
    fn owns(&self, console: &Console) -> Option<Result<(), String>> {
        let server = self.server.as_ref()?;
        let panes = transport::observe_window_panes(server, &console.name)?;
        let stamp =
            transport::observe_session_option(server, &console.name, theme::SESSION_ID_OPTION)?;
        let me = self.me.as_deref();
        Some(submit::owner(&panes, me, &stamp, &console.uuid))
    }

    /// Read ownership now and take the answer.
    pub(super) fn tick(&mut self, console: &Console) -> Vec<Effect> {
        let reading = match self.owns(console) {
            None => Reading::Unknown,
            Some(Ok(())) => Reading::Owner,
            Some(Err(why)) => Reading::NotOwner(why),
        };
        let was = self.input.line().is_some();
        let mut effects = self.input.tick(reading, Instant::now());
        if !was && self.input.line().is_some() {
            effects.extend(self.input.restore(submit::restore(&console.dir)));
        }
        effects
    }

    /// Wait for the terminal until `deadline`; what its input did, every ask
    /// and close already carried out, or `None` when it did nothing.
    pub(super) fn wait(&mut self, console: &Console, deadline: Instant) -> Option<Vec<Effect>> {
        let effects = match self.reads.wait(deadline) {
            Got::Read(stamp, bytes) => self.input.chunk(&bytes, stamp),
            Got::Ended => self.input.closed(),
            Got::Due => return None,
        };
        let act = |effect| match effect {
            Effect::Ask { raw, seat, body } => Effect::Print(self.ask(console, &raw, &seat, body)),
            Effect::Close(id) => {
                let owns = || self.owned(console);
                Effect::Print(match submit::close_owned(&console.dir, &id, owns) {
                    Ok(()) => format!("closed {id}"),
                    Err(why) => format!("refused: {why}"),
                })
            }
            effect => effect,
        };
        Some(effects.into_iter().map(act).collect())
    }

    /// The composer line, while this console takes input.
    pub(super) fn line(&self) -> Option<String> {
        self.input.line()
    }

    fn owned(&self, console: &Console) -> Result<(), String> {
        let unread = || Err("tmux did not answer who owns the input".to_owned());
        self.owns(console).unwrap_or_else(unread)
    }

    /// Carry one ask through the helpers' own tracked path, as the console.
    fn ask(&self, console: &Console, raw: &[u8], seat: &str, body: String) -> String {
        let (now, entropy) = (time::Timestamp::now(), crate::entropy());
        let id = tracked::request_id(tracked::Kind::Ask.id_prefix(), now, entropy);
        let sender = tracked::Sender {
            display: tracked::CONSOLE_SINK.to_owned(),
            slot: String::new(),
            session: console.name.clone(),
        };
        let deliver = || {
            let (tail, mut said) = ([seat.to_owned(), body], Vec::new());
            let run = tracked::run(
                tracked::Kind::Ask,
                &console.dir,
                &tail,
                Some(&sender),
                &crate::own_session(&console.dir),
                now,
                entropy,
                crate::send_defer(),
                &mut std::io::sink(),
                &mut said,
            );
            if let Err(why) = run {
                said.extend(why.to_string().bytes());
            }
            String::from_utf8_lossy(&said).into_owned()
        };
        let owns = || still(&self.pair, console.seats()).and_then(|()| self.owned(console));
        match submit::submit(&console.dir, &console.name, raw, &id, owns, deliver) {
            Ok(outcome) => input::outcome_line(&outcome, seat),
            Err(why) => format!("refused: {why}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Got, Input, Reads, Seat, Term, pair_of, reader};
    use crate::console::tests::{ID, Rig};
    use std::sync::mpsc::{RecvTimeoutError, sync_channel};
    use std::time::{Duration, Instant};

    #[test]
    fn an_ended_input_is_said_once_and_every_later_wait_sleeps_to_its_deadline() {
        let (send, reads) = sync_channel(1);
        let stamp = Instant::now();
        send.send((stamp, b"x".to_vec())).unwrap();
        drop(send);
        let mut reads = Reads(Some(reads));
        let far = stamp + Duration::from_mins(1);
        assert_eq!(reads.wait(far), Got::Read(stamp, b"x".to_vec()));
        assert_eq!(reads.wait(far), Got::Ended);
        let start = Instant::now();
        for _ in 0..3 {
            assert_eq!(reads.wait(start + Duration::from_millis(40)), Got::Due);
        }
        assert!(start.elapsed() >= Duration::from_millis(40), "no spin");
    }

    #[test]
    fn a_read_that_fails_ends_the_input_and_is_never_retried() {
        let dir = std::fs::File::options()
            .read(true)
            .open(std::env::temp_dir())
            .unwrap();
        let (send, reads) = sync_channel(1);
        std::thread::spawn(move || reader(dir, &send));
        let ended = reads.recv_timeout(Duration::from_secs(5));
        assert_eq!(ended, Err(RecvTimeoutError::Disconnected));
    }

    #[test]
    fn one_main_seat_and_only_agent_names_make_a_pair_and_nothing_is_dropped() {
        let seat = |slot: &str, name: &str| Seat {
            slot: slot.to_owned(),
            name: name.to_owned(),
            profile: None,
        };
        let (lead, colead) = (seat("main", "lead"), seat("worker.0", "colead"));
        let pair = Ok(vec![lead.clone(), colead.clone()]);
        assert_eq!(pair_of(vec![colead.clone(), lead.clone()]), pair);
        let none = Err("the meta names no single main seat".to_owned());
        assert_eq!(pair_of(vec![colead.clone()]), none);
        assert_eq!(pair_of(vec![lead.clone(), seat("main", "b")]), none);
        let bad = Err("the worker.0 seat is not an agent name".to_owned());
        assert_eq!(pair_of(vec![lead, seat("worker.0", "%9")]), bad);
        for main in ["%3", "--cross-session", "le\x1bad", ""] {
            let bad = Err("the main seat is not an agent name".to_owned());
            assert_eq!(pair_of(vec![seat("main", main), colead.clone()]), bad);
        }
    }

    #[test]
    fn a_tick_that_cannot_prove_ownership_promotes_and_restores_nothing() {
        let rig = Rig::new("term-restore");
        let meta =
            format!("session_id={ID}\nlayout=lead-pair\nseat.main=lead\nseat.worker.0=colead\n");
        std::fs::write(rig.0.join("s/meta"), meta).unwrap();
        let console = rig.console();
        crate::store::open(&console.dir)
            .publish_console_draft(b"kept")
            .unwrap();
        let pair = console.seats().and_then(pair_of).unwrap();
        let input = Input::new(pair.iter().map(|seat| seat.name.clone()).collect());
        let mut term = Term {
            input,
            pair,
            reads: Reads(None),
            server: None,
            me: None,
        };
        assert_eq!(term.tick(&console), [], "tmux unanswered is Unknown");
        assert_eq!(term.line(), None);
    }

    #[test]
    fn an_ask_after_the_meta_changed_its_lead_pair_is_refused_and_writes_nothing() {
        let (rig, head) = (
            Rig::new("term-pair"),
            format!("session_id={ID}\nlayout=lead-pair"),
        );
        let meta = |m: &str, c: &str| format!("{head}\nseat.main={m}\nseat.worker.0={c}\n");
        std::fs::write(rig.0.join("s/meta"), meta("lead", "colead")).unwrap();
        let console = rig.console();
        let (store, journal) = (crate::store::open(&console.dir), b"{\"ts\":\"x\"}\n");
        std::fs::write(rig.0.join("s/events.jsonl"), journal).unwrap();
        store.publish_console_draft(b"an earlier draft").unwrap();
        let pair = console.seats().and_then(pair_of).unwrap();
        let (input, reads, server, me) = (Input::default(), Reads(None), None, None);
        let mut term = Term {
            input,
            pair,
            reads,
            server,
            me,
        };
        let changed = "the lead pair changed since this console opened - restart this console";
        for (main, colead, why) in [
            ("lead", "other", changed),
            ("colead", "lead", changed),
            ("lead", "%3", "the worker.0 seat is not an agent name"),
            ("lead", "colead", "tmux did not answer who owns the input"),
        ] {
            std::fs::write(rig.0.join("s/meta"), meta(main, colead)).unwrap();
            let said = term.ask(&console, b"hi", "lead", "hi".to_owned());
            assert_eq!(said, format!("refused: {why}"), "{main} {colead}");
        }
        let (draft, kept) = (store.console_draft().unwrap(), b"an earlier draft".to_vec());
        assert_eq!(
            draft,
            crate::store::SourceRead::Ready(kept),
            "the draft is untouched"
        );
        assert_eq!(store.container(), journal.to_vec(), "no ask was recorded");
        let deadline = Instant::now() + Duration::from_millis(20);
        assert_eq!(
            term.wait(&console, deadline),
            None,
            "a quiet terminal says nothing"
        );
        assert!(Instant::now() >= deadline, "and says it at its deadline");
    }
}
