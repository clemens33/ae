//! The flight: the staged copy shipped, the asks queued and in flight, their outcome words and the settle at quit.

use super::{
    App, Arc, Binding, CLIP_MAX, Cell, Duration, Effect, FLASH_WINDOW, File, Instant, Item, Key,
    Kind, LeaseFile, MouseKind, NOTICES, QUIT_WINDOW, Reading, Receiver, RecvTimeoutError, Stage,
    Timestamp, Write, console, lines_word, mpsc, open, shell, submit, term, terminal_text,
    transport, upgrade,
};

/// An ask the input produced, held until the frame that names its target is
/// drawn: the entry that wrote it and the line, for a writer that ended.
pub(super) struct Queued {
    pub(super) key: (String, String),
    pub(super) entry: u64,
    pub(super) raw: Vec<u8>,
    pub(super) seat: String,
    pub(super) body: String,
    pub(super) about: String,
    /// The quiet lane note for a line routed away from the lead.
    pub(super) note: Option<String>,
}

/// The one ask on its way to its seat, carried off the loop's thread.
pub(super) struct Flight {
    id: String,
    seat: String,
    /// The session the ask went to.
    about: String,
    /// The quiet lane note for a line routed away from the lead.
    note: Option<String>,
    /// Its outcome, once the delivery has ended.
    done: Receiver<submit::Outcome>,
    /// The writer lease its submit took on that very incarnation: held until
    /// the delivery ends, whatever happens to the writing, so no other window
    /// is admitted beside it.
    pub(super) lease: Arc<LeaseFile>,
}

/// How long a quit waits for an ask in flight before it leaves it: the helpers'
/// own delivery gives up by itself well inside it (the send lock waits two
/// minutes, the quiet gate [`crate::deliver::DEFAULT_DEFER`], the submit
/// proof seconds more).
pub(super) const FLIGHT_QUIT_BOUND: Duration = Duration::from_mins(4);

/// An ask's outcome word, shown until the lane holds the ask itself.
pub(super) struct Settling {
    pub(super) at: Instant,
    pub(super) id: String,
    pub(super) word: String,
    /// The session the ask went to: its hint row alone shows the word.
    pub(super) about: String,
}

impl App {
    /// Record where this app's pane sits in the lease a submit just took;
    /// outside tmux, or when tmux or the grammar refuses, the record stays empty.
    fn note_holder(&self, lease: &File) {
        let place = (self.server.as_ref().zip(self.pane.as_deref()))
            .and_then(|(server, pane)| transport::observe_pane_place(server, pane));
        let record = place.and_then(|(session, window)| console::holder::encode(&session, &window));
        let _ = crate::store::SessionStore::console_writer_note(
            lease,
            record.as_deref().unwrap_or_default(),
        );
    }

    /// Send `text` to the tmux buffer and to the terminal clipboard of the
    /// client showing this app; the words say what reached where. `count` is
    /// how the text is named. Outside tmux there is no client to name: the
    /// loop writes the text to the terminal itself.
    pub(super) fn ship(&mut self, text: &str, count: &str) -> String {
        let (Some(server), Some(pane)) = (&self.server, &self.pane) else {
            if text.len() > CLIP_MAX {
                let size = text.len();
                return format!(
                    "not copied: {size} bytes is over the {CLIP_MAX}-byte clipboard limit"
                );
            }
            self.clip = Some(text.to_owned());
            return format!("sent {count} to the terminal clipboard - unconfirmed");
        };
        let viewer = match transport::observe_clients(server) {
            None => Err("the tmux clients did not answer".to_owned()),
            Some(clients) => open::copy_client(&clients, pane).map_err(|why| why.why()),
        }
        .and_then(|name| {
            let named = open::client_name(&name);
            named
                .then_some(name)
                .ok_or("the client name failed its grammar".to_owned())
        });
        match (
            transport::copy_buffer(server, viewer.as_deref().ok(), text.as_bytes()),
            viewer,
        ) {
            (Err(why), _) => format!("copy failed: {}", terminal_text(&why)),
            (Ok(()), Ok(_)) => format!(
                "copied {count} to the tmux buffer; terminal clipboard if your terminal allows it"
            ),
            (Ok(()), Err(why)) => format!("copied {count} to the tmux buffer only - {why}"),
        }
    }

    /// The hint-row line now, loudest first: an ask on its way, an armed
    /// key, a refused paste, an outcome the lane has not caught up with,
    /// another installed core.
    pub(super) fn note(&self) -> Option<String> {
        let queued = self.queued.first().map(|ask| ask.seat.as_str());
        let flying = self.flight.as_ref().map(|flight| flight.seat.as_str());
        let sending = queued.or(flying).map(|seat| format!("sending to {seat}…"));
        sending
            .or_else(|| self.armed.as_ref().map(|armed| armed.what.line()))
            .or_else(|| self.flash.as_ref().map(|(line, _)| line.clone()))
            .or_else(|| {
                let settling = self.settling.as_ref();
                let here = |s: &&Settling| self.model.selected() == Some(s.about.as_str());
                settling.filter(here).map(|settling| settling.word.clone())
            })
            .or_else(|| {
                let failed = self.failed.as_ref();
                self.upgrade
                    .as_ref()
                    .map(|seen| upgrade::hint(seen, failed))
            })
    }

    /// What holds a restart back now: any draft, not only the writer's —
    /// one kept in memory after Esc is lost too.
    pub(super) fn blocker(&self, now: Instant) -> Option<upgrade::Blocker> {
        upgrade::restart_blocker(
            self.inputs.values().any(|input| !input.draft().is_empty()),
            !self.queued.is_empty() || self.flight.is_some(),
            !self.deferred.is_empty(),
            self.composing(),
            self.held.is_some(),
            self.model.settings_open()
                || self.model.peek_open()
                || self.model.reader_open()
                || self.modal.is_some(),
            self.model.turn().is_some(),
            self.armed.is_some(),
            self.last_input,
            now,
        )
    }

    /// The restart due now: another core is installed, this app may restart
    /// into it, no exec into it has failed, and nothing holds it back.
    pub(super) fn restart_due(&self, now: Instant) -> Option<(String, upgrade::Via)> {
        let upgrade = self.upgrade.as_ref()?;
        let restart = upgrade.restart.as_ref().ok()?;
        let tried = (self.failed.as_ref()).is_some_and(|(version, _)| *version == upgrade.version);
        (!tried && self.blocker(now).is_none() && self.clip.is_none() && !self.holds_lane())
            .then(|| (upgrade.version.clone(), restart.clone()))
    }

    /// End what `now` outlived: whether any line went away.
    pub(super) fn lapse(&mut self, now: Instant) -> bool {
        let over = |at: Instant, window| now.saturating_duration_since(at) > window;
        let armed = self.armed.take_if(|a| over(a.at, QUIT_WINDOW)).is_some();
        let flash = (self.flash.take_if(|(_, at)| over(*at, FLASH_WINDOW))).is_some();
        let word = (self.settling.take_if(|s| over(s.at, FLASH_WINDOW))).is_some();
        // The open panel's capture ages: one redraw for each new second.
        let aged = self.model.peek_open()
            && self.peek.at.is_some()
            && self.peek.painted != Some(self.peek.age(now));
        armed | flash | word | aged
    }

    /// A wheel notch landing past a bound the last frame has not proven the
    /// top — page keys may have asked further back — waits for the next
    /// frame rather than stopping short at it. A viewed seat owns its wheel
    /// at once: the chat the bounds describe is hidden, so its bound waits
    /// for nothing.
    pub(super) fn unbounded(&self, key: &Key) -> bool {
        if self.model.peek_open() || self.model.reader_open() {
            return false;
        }
        let Key::Mouse(mouse) = key else {
            return false;
        };
        matches!(mouse.kind, MouseKind::WheelUp | MouseKind::WheelDown)
            && self.modal.is_none()
            && self.layout.in_chat(*mouse)
            && !self.layout.complete
            && self.model.wheel_passes(
                mouse.kind == MouseKind::WheelUp,
                self.layout.page_rows,
                self.layout.max_scroll,
            )
    }

    /// What the loop owes after a frame: the staged copy shipped and a hint
    /// the frame had no room to draw. Whether another frame is due.
    pub(super) fn shipped(&mut self) -> bool {
        self.ship_staged() || std::mem::take(&mut self.again)
    }

    /// A frame's debts, settled together: the copy it shipped and the asks
    /// that waited for it. Neither skips the other; whether a frame is due.
    pub(super) fn owed(&mut self) -> bool {
        self.inputs.values_mut().for_each(crate::clipboard::sweep);
        self.shipped() | self.deliver() | self.land() | self.unpark() | self.image_ready()
    }

    pub(super) fn image_ready(&mut self) -> bool {
        let token = self.writer.as_ref().map(|w| (w.key(), w.entry));
        let Some(answer) = crate::clipboard::poll(&mut self.clipboard, token.as_ref()) else {
            return false;
        };
        if self.owns(&Cell::default()).is_err() {
            return false;
        }
        let input = token.and_then(|(key, _)| self.inputs.get_mut(&key));
        if let Some(input) = input {
            if let Err(why) = crate::clipboard::finish(input, answer) {
                self.refuse_paste(&why);
            }
            self.show();
        }
        true
    }

    /// Ship the copy the last frame highlighted, now that it is written: the
    /// text goes the way `y` sends a turn. Whether it said anything.
    pub(super) fn ship_staged(&mut self) -> bool {
        let ready = (self.select.as_mut()).filter(|held| matches!(held.stage, Stage::Ready(_)));
        let Some(Stage::Ready(text)) = ready.map(|held| std::mem::take(&mut held.stage)) else {
            return false;
        };
        let line = if text.trim().is_empty() {
            "nothing to copy: the selection is blank".to_owned()
        } else {
            self.ship(&text, &lines_word(&text))
        };
        self.flash = Some((line, Instant::now()));
        true
    }

    /// Carry out what the input asked for; each outcome becomes a notice,
    /// shown at once.
    pub(super) fn effects(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            // A writer's session is always the selection.
            let about = self.model.selected().or(self.home.as_deref());
            let about = about.unwrap_or_default().to_owned();
            let line = match effect {
                Effect::DiscardDraft => {
                    let result = self.owns(&Cell::default()).and_then(|()| {
                        let writer = self.writer.as_ref().ok_or("not writing")?;
                        self.recovery
                            .record(writer.console.dir())?
                            .clear()
                            .map_err(|why| why.to_string())
                    });
                    match result {
                        Ok(()) => {
                            if let Some(input) = self
                                .writer
                                .as_ref()
                                .and_then(|w| self.inputs.get_mut(&w.key()))
                            {
                                input.discarded();
                            }
                            "kept line discarded".to_owned()
                        }
                        Err(why) => format!("refused: {why}"),
                    }
                }
                Effect::ImagePaste => {
                    if self.owns(&Cell::default()).is_ok()
                        && let Some(w) = &self.writer
                    {
                        crate::clipboard::start(
                            &mut self.clipboard,
                            (w.key(), w.entry),
                            w.console.dir().to_owned(),
                        );
                    }
                    continue;
                }
                Effect::Print(line) => line,
                Effect::Ask { raw, seat, body } => {
                    let Some((key, entry)) = (self.writer.as_ref()).map(|w| (w.key(), w.entry))
                    else {
                        self.notice(&about, "refused: not writing".to_owned());
                        continue;
                    };
                    let (seat, body, note) = self.routed(&about, &raw, seat, body);
                    self.queued.push(Queued {
                        key,
                        entry,
                        raw,
                        seat,
                        body,
                        about,
                        note,
                    });
                    continue;
                }
                Effect::Close(id) => self.close(id.as_deref()),
                Effect::Open(seat) => match self.open_written(&seat) {
                    Some(line) => line,
                    None => continue,
                },
                Effect::Shell { raw, line } => {
                    let lost = Cell::default();
                    match self.owns(&lost) {
                        Ok(()) => self.shell_written(&raw, &line),
                        Err(why) => {
                            self.unproven(lost, &raw);
                            format!("refused: {why}")
                        }
                    }
                }
                Effect::More(_) => {
                    "refused: /more is the tmux chat's; click or press Enter on the fold line"
                        .to_owned()
                }
                Effect::Paste(_) | Effect::Lane(_) | Effect::Styled(_) => continue,
            };
            self.notice(&about, line);
        }
        self.show();
    }

    /// `!` in the window that writes: the human's own line, run in the
    /// session's work dir — a popup on the client showing this app, or a
    /// detached shell when the app has no tmux pane. A refusal puts the line
    /// back in the writer's composer; nothing is recorded either way.
    fn shell_written(&mut self, raw: &[u8], line: &str) -> String {
        let writer = self.writer.as_ref().ok_or_else(|| "not writing".to_owned());
        let started = writer.and_then(|writer| {
            let dir = writer.console.work_dir()?;
            let place = match (self.pane.as_deref(), self.server.as_ref()) {
                (None, _) => shell::Place::Outside,
                (Some(pane), Some(server)) => shell::Place::Tmux { server, pane },
                (Some(_), None) => return Err("tmux did not answer".to_owned()),
            };
            term::shell(place, &dir, line)
        });
        started.unwrap_or_else(|why| {
            let key = self.writer.as_ref().map(Binding::key);
            if let Some(input) = key.and_then(|key| self.inputs.get_mut(&key)) {
                input.put_back(raw);
            }
            shell::refused(line, &why)
        })
    }

    /// One of ae's own lines about session `name`; each keeps its newest few.
    pub(super) fn notice(&mut self, name: &str, line: String) {
        let item = Item {
            micros: Timestamp::now().epoch().saturating_mul(1_000_000),
            kind: Kind::Said {
                who: "ae".to_owned(),
            },
            body: line,
            record: None,
        };
        self.notices.push((name.to_owned(), item));
        self.noticed = self.noticed.wrapping_add(1);
        let mut of = self.notices.iter().filter(|(of, _)| of == name);
        if of.nth(NOTICES).is_some()
            && let Some(first) = self.notices.iter().position(|(of, _)| of == name)
        {
            self.notices.remove(first);
        }
    }

    /// What the admission re-proves: this window writes, and the incarnation
    /// and lead pair its entry proved are the session's now. A session no
    /// longer proven is kept in `lost`.
    fn owns(&self, lost: &Cell<Option<String>>) -> Result<(), String> {
        let writer = self.writer.as_ref().ok_or("not writing")?;
        let seats = writer.console.seats();
        term::still(&writer.seats, seats).inspect_err(|why| lost.set(Some(why.clone())))
    }

    /// A session the admission no longer proved ends the writing at once,
    /// HELD with `line`, the entry it refused, back in its draft: memory only.
    fn unproven(&mut self, lost: Cell<Option<String>>, line: &[u8]) {
        let Some(why) = lost.into_inner() else {
            return;
        };
        let key = self.writer.as_ref().map(Binding::key);
        self.take(Reading::NotOwner(why), Instant::now());
        if let Some(input) = key.and_then(|key| self.inputs.get_mut(&key)) {
            let _ = input.restore(submit::Draft::Kept(line.to_vec()));
        }
    }

    /// One ask of `seat`, admitted here through the chat's own admission and
    /// carried on a thread of its own through the tracked path: its request id,
    /// the channel its outcome arrives on and the writer lease it holds. The
    /// lease and the admission lock are taken without waiting, so a session
    /// another window is sending to refuses at once. A session whose recorded
    /// server that path cannot name is no longer proven: refused in its words
    /// before anything is written.
    pub(super) fn begin(
        &mut self,
        raw: &[u8],
        seat: &str,
        body: String,
    ) -> Result<(String, Receiver<submit::Outcome>, Arc<LeaseFile>), String> {
        let Some(writer) = &self.writer else {
            return Err("refused: not writing".to_owned());
        };
        let (console, lost) = (&writer.console, Cell::default());
        let record = self.recovery.record(console.dir())?;
        let (lease, lock) = term::send_locks(console).map_err(|why| format!("refused: {why}"))?;
        let held = Arc::new(LeaseFile { file: lease });
        self.note_holder(&held.file);
        let owns = || {
            self.owns(&lost)?;
            let name = console.name();
            let routed = crate::tracked::named_server(console.dir(), name, name);
            let routed = routed.map(drop).map_err(|why| why.message());
            routed.inspect_err(|why| lost.set(Some(why.clone())))
        };
        let admitted = term::admit_ask_locked(console, record, lock, owns, raw, seat, body);
        self.unproven(lost, raw);
        let (ask, admitted) = admitted.map_err(|why| format!("refused: {why}"))?;
        let id = ask.id().to_owned();
        let (send, done) = mpsc::channel();
        let (carry, named) = (Arc::clone(&self.carry), id.clone());
        let thread = std::thread::Builder::new().name("ae-app-ask".to_owned());
        // The admission lock goes with the ask: it is held until the delivery
        // ends, and a thread that dies lets it go.
        thread
            .spawn(move || {
                let outcome =
                    admitted.finish(&mut crate::events::Held::default(), &named, || carry(ask));
                let _ = send.send(outcome);
            })
            .map_err(|why| format!("refused: could not start the delivery ({why})"))?;
        Ok((id, done, held))
    }

    /// An ask whose delivery ended: its outcome, said as an ask's outcome is.
    /// Whether one landed.
    pub(super) fn land(&mut self) -> bool {
        let Some(flight) = &self.flight else {
            return false;
        };
        let outcome = match flight.done.try_recv() {
            Ok(outcome) => outcome,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => {
                let why = "the delivery ended without an answer";
                submit::Outcome::Unknown(flight.id.clone(), why.to_owned())
            }
        };
        let Some(flight) = self.flight.take() else {
            return false;
        };
        // The delivery is over: its lease goes, so another window may send.
        drop(flight.lease);
        let mut lines = self.settle(flight.id, &flight.seat, &flight.about, outcome);
        lines.splice(0..0, flight.note);
        for line in lines {
            self.notice(&flight.about, line);
        }
        self.reread();
        self.show();
        true
    }

    /// End the loop's wait on an ask in flight, so a quit never cuts a
    /// delivery off half way: the terminal is already back, the human is told
    /// what is awaited, and the wait is [`FLIGHT_QUIT_BOUND`] at most.
    pub(super) fn finish_flight(
        &mut self,
        err: &mut impl Write,
        bound: Duration,
    ) -> std::io::Result<()> {
        let Some(flight) = self.flight.take() else {
            return Ok(());
        };
        let seat = terminal_text(&flight.seat);
        writeln!(
            err,
            "ae app: finishing the delivery to {seat} (^C stops it)"
        )?;
        if flight.done.recv_timeout(bound) == Err(RecvTimeoutError::Timeout) {
            let secs = bound.as_secs_f32();
            writeln!(
                err,
                "ae app: the delivery to {seat} was not over in {secs}s"
            )?;
        }
        Ok(())
    }

    /// Deliver the queued asks, after the frame that named their target. Each
    /// is re-proven first: a writer that ended since asks nothing and its
    /// line goes back to the draft in memory. An admitted ask is carried on a
    /// thread of its own and lands in [`App::land`]; one at a time, so an ask
    /// behind it is refused where the human sees it. Whether any ran.
    pub(super) fn deliver(&mut self) -> bool {
        let queued = std::mem::take(&mut self.queued);
        let ran = !queued.is_empty();
        for q in queued {
            let live = (self.writer.as_ref()).is_some_and(|writer| writer.entry == q.entry);
            let told = if live {
                let begun = match &self.flight {
                    Some(flight) => Err(format!(
                        "not sent to {}: an ask to {} is still being delivered; the line is back in the draft",
                        q.seat, flight.seat
                    )),
                    None => self.begin(&q.raw, &q.seat, q.body),
                };
                match begun {
                    Ok((id, done, lease)) => {
                        self.flight = Some(Flight {
                            id,
                            seat: q.seat,
                            about: q.about.clone(),
                            note: q.note,
                            done,
                            lease,
                        });
                        Vec::new()
                    }
                    Err(refused) => {
                        // A writer the refusal ended got its line back from `unproven`; the
                        // one still writing gets it here. A newer writer is not touched: the
                        // line then stays only in the kept-draft file.
                        let live = (self.writer.as_ref()).is_some_and(|w| w.entry == q.entry);
                        if let Some(input) = self.inputs.get_mut(&q.key).filter(|_| live) {
                            input.put_back(&q.raw);
                        }
                        vec![refused]
                    }
                }
            } else {
                if let Some(input) = self.inputs.get_mut(&q.key) {
                    let _ = input.restore(submit::Draft::Kept(q.raw));
                }
                let why = "writing ended; the line is back in the draft";
                vec![format!("not sent to {}: {why}", q.seat)]
            };
            for line in told {
                self.notice(&q.about, line);
            }
            self.reread();
        }
        // A held drag keeps the screen as drawn: re-aging the open asks under it
        // would change the cells it lit.
        if ran || !self.holds_lane() {
            self.show();
        }
        ran
    }

    /// What an ask's outcome shows. A sent ask is the lane's `· sent` header
    /// suffix and only its kept-draft warning is a notice; the lane already
    /// draws an uncertain or undelivered ask in the same words, so those wait
    /// on the hint row until it holds the ask; the rest has no lane line and
    /// is a notice.
    pub(super) fn settle(
        &mut self,
        id: String,
        seat: &str,
        about: &str,
        outcome: submit::Outcome,
    ) -> Vec<String> {
        let check = format!("check {seat} pane (Agents: n/p, oo opens a seat)");
        let word = match outcome {
            submit::Outcome::Sent(_, kept) => {
                self.sent.insert(self.sent_key(about, &id));
                let (at, about) = (Instant::now(), about.to_owned());
                let word = "sent".to_owned();
                self.settling = Some(Settling {
                    at,
                    id,
                    word,
                    about,
                });
                return kept.into_iter().collect();
            }
            submit::Outcome::Unknown(_, said) => {
                return vec![crate::console::input::unrecorded_line(&said, &check)];
            }
            submit::Outcome::Uncertain(_) => format!("uncertain: {check}"),
            submit::Outcome::NotDelivered(_) => "not delivered".to_owned(),
        };
        let (at, about) = (Instant::now(), about.to_owned());
        self.settling = Some(Settling {
            at,
            id,
            word,
            about,
        });
        Vec::new()
    }

    /// `/close`: the session's newest open console ask, or the one named,
    /// through the chat's admission under the same zero-wait locks as a send.
    /// Locks another window holds refuse at once and put the command back.
    pub(super) fn close(&mut self, id: Option<&str>) -> String {
        let Some(writer) = &self.writer else {
            return "refused: not writing".to_owned();
        };
        // The ask in flight holds the admission lock a close needs.
        if let Some(flight) = &self.flight {
            return format!(
                "refused: the ask to {} is still being delivered; /close waits for it",
                flight.seat
            );
        }
        let entry = id.map_or_else(|| "/close".to_owned(), |id| format!("/close {id}"));
        let (console, lost) = (&writer.console, Cell::default());
        let (lease, lock) = match term::send_locks(console) {
            Ok(locks) => locks,
            Err(why) => {
                let key = writer.key();
                if let Some(input) = self.inputs.get_mut(&key) {
                    input.put_back(entry.as_bytes());
                }
                return format!("refused: {why}");
            }
        };
        let held = LeaseFile { file: lease };
        self.note_holder(&held.file);
        let owns = || self.owns(&lost);
        let line = match console.close_owned(id, owns, lock) {
            Ok(closed) => closed.line(),
            Err(why) => format!("refused: {why}"),
        };
        drop(held);
        self.unproven(lost, entry.as_bytes());
        line
    }
}
