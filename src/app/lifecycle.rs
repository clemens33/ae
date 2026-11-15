//! The lifecycle asks: a Start, Stop or End question, the confirm, and the records reads that the menu opens.

use super::{
    App, CellRow, EndingAsk, Instant, PathBuf, RecordsAsk, Refusal, Request, ServerId, Timestamp,
    loader, menu, open, transport,
};

/// Why a menu action refuses an incarnation its record no longer names.
/// What a Start or the Stop question freezes of the asking client and server.
struct Asker {
    server: ServerId,
    client: String,
    client_pid: String,
    /// The session the asking client is in.
    session: String,
    server_pid: String,
    server_start: String,
}

const REPLACED: &str =
    "its record no longer names the session_id the menu was drawn from - replaced or changed";

impl App {
    /// The menu's Activity… or Memos…: the reader reads the rows, under the
    /// identity the card drew; the menu stays until they land. A read-only
    /// item takes the card's refusals as they are: the fold names its own
    /// gaps, never fills them. A record directory ae cannot read is told.
    pub(super) fn ask_records(&mut self, item: menu::Item, dialog: menu::Dialog) {
        let Some(menu::Modal::Menu(menu)) = &mut self.modal else {
            return;
        };
        if menu.reading.is_some() {
            return;
        }
        let card = menu.card.clone();
        let Some(dir) = self.dirs.get(&card.name).cloned() else {
            self.modal = None;
            let word = item.word();
            let why = "ae read no record directory for it";
            return self.tell(&card.name, format!("refused: {word} {}: {why}", card.name));
        };
        let generation = self.records_generation.wrapping_add(1);
        self.records_generation = generation;
        menu.reading = Some(menu::Asked { generation, dialog });
        let line = format!("reading {} of {}…", item.word(), card.name);
        self.flash = Some((line, Instant::now()));
        let ask = Box::new(RecordsAsk {
            generation,
            dialog,
            session: card.name,
            uuid: card.uuid,
            dir,
            live: !card.stopped,
        });
        if let Some(tx) = &self.ask {
            let _ = tx.send(Request::Records(Some(ask)));
        }
    }

    /// The rows a dialog was asked for. Only the menu that asked takes them:
    /// an answer for a closed menu, or for an older ask, is dropped.
    pub(super) fn records_landed(&mut self, generation: u64, rows: Vec<CellRow>) {
        let Some(menu::Modal::Menu(menu)) = &self.modal else {
            return;
        };
        let Some(asked) = menu.reading.filter(|asked| asked.generation == generation) else {
            return;
        };
        let records = menu::Records::new(asked.dialog, &menu.card.name, rows);
        self.modal = Some(menu::Modal::Records(records));
        if (self.flash.as_ref()).is_some_and(|(line, _)| line.starts_with("reading ")) {
            self.flash = None;
        }
    }

    /// An outcome, on the hint row now and in `name`'s lane.
    pub(super) fn tell(&mut self, name: &str, line: String) {
        self.flash = Some((line.clone(), Instant::now()));
        self.notice(name, line);
        self.show();
    }

    /// What a Start or the Stop question freezes about the asking client and
    /// its server: the record must still name the card's incarnation, tmux must
    /// be reachable, the session recorded on this very server, and one client
    /// view this app.
    fn asker(&self, card: &menu::Card) -> Result<Asker, String> {
        let name = &card.name;
        let dir = (self.dirs.get(name)).ok_or("ae read no record directory for it")?;
        if loader::sole_uuid(dir).as_deref() != Some(card.uuid.as_str()) {
            return Err(REPLACED.to_owned());
        }
        let (Some(server), Some(pane)) = (&self.server, &self.pane) else {
            return Err(Refusal::NoTmux.why());
        };
        let mut sockets = crate::fleet_read::SocketPaths::live();
        let recorded = crate::session_launch::recorded_server_resolved(dir);
        if !recorded.is_some_and(|recorded| sockets.proven_same(server, &recorded)) {
            return Err(Refusal::Elsewhere(name.clone()).why());
        }
        let clients = transport::observe_clients(server)
            .ok_or_else(|| Refusal::Unread("the tmux clients".to_owned()).why())?;
        let client = open::copy_client(&clients, pane).map_err(|why| why.why())?;
        let menu = transport::observe_menu_client(server, &client)
            .ok_or("the asking client did not answer")?;
        let identity =
            transport::observe_server_identity(server).ok_or("the tmux server did not answer")?;
        Ok(Asker {
            server: server.clone(),
            client,
            client_pid: menu.pid,
            session: menu.session_name,
            server_pid: identity.pid,
            server_start: identity.start,
        })
    }

    /// The Stop question for `card`: every fact its answer is proven against
    /// is read now and frozen in it, or the refusal line for the first that
    /// does not hold.
    pub(super) fn question(&self, mut card: menu::Card) -> Result<menu::Question, String> {
        let name = card.name.clone();
        let refuse = |why: &str| format!("refused: Stop {name}: {why}; nothing stopped");
        if !self.attended() {
            return self.unattended_question(card).map_err(|why| refuse(&why));
        }
        let asker = self.asker(&card).map_err(|why| refuse(&why))?;
        let id = crate::lifecycle::live_id(&asker.server, &name)
            .ok_or_else(|| refuse("tmux does not list it on this server"))?;
        if card.id.as_ref().is_some_and(|painted| *painted != id) {
            return Err(refuse(
                "its tmux session was replaced since the menu was drawn",
            ));
        }
        card.home = asker.session == name;
        let expect = crate::lifecycle::StopExpectation {
            session_id: id,
            uuid: card.uuid.clone(),
            server_pid: asker.server_pid,
            server_start: asker.server_start,
            deadline: Timestamp::now().epoch() + crate::session_menu::CONFIRM_WINDOW_SECS,
            client: asker.client,
            client_pid: asker.client_pid,
            server: asker.server,
            require_meta_agent: false,
        };
        Ok(menu::Question::new(card, expect))
    }

    /// Whether this app runs inside a tmux client: a Start or Stop then rides
    /// that client's proofs. Outside tmux there is none to name.
    fn attended(&self) -> bool {
        self.server.is_some() && self.pane.is_some()
    }

    /// The tmux server `name`'s record names, proven to be the one the card
    /// was drawn from: its record directory still holds the card's
    /// incarnation, and the server it records is a positive selector.
    fn recorded(&self, card_name: &str, uuid: &str) -> Result<(PathBuf, ServerId), String> {
        let dir = (self.dirs.get(card_name)).ok_or("ae read no record directory for it")?;
        if loader::sole_uuid(dir).as_deref() != Some(uuid) {
            return Err(REPLACED.to_owned());
        }
        match crate::session_launch::recorded_server_resolved(dir) {
            Some(server @ ServerId::Selected(_)) => Ok((dir.clone(), server)),
            _ => Err("its record names no tmux server".to_owned()),
        }
    }

    /// The Stop question outside tmux: nobody is attached to answer, so the
    /// app's own modal is the confirmation, and what tmux and the record say
    /// of the session is frozen in it from the server the record names — its
    /// process, its start, the session id — for the supervisor to prove again
    /// under the lock.
    pub(super) fn unattended_question(
        &self,
        mut card: menu::Card,
    ) -> Result<menu::Question, String> {
        let name = card.name.clone();
        let (_, server) = self.recorded(&name, &card.uuid)?;
        let identity = transport::observe_server_identity(&server)
            .ok_or("the tmux server its record names did not answer")?;
        let id = crate::lifecycle::live_id(&server, &name)
            .ok_or("tmux does not list it on the server its record names")?;
        if card.id.as_ref().is_some_and(|painted| *painted != id) {
            return Err("its tmux session was replaced since the menu was drawn".to_owned());
        }
        card.home = false;
        let deadline = Timestamp::now().epoch() + crate::session_menu::CONFIRM_WINDOW_SECS;
        let expect = crate::lifecycle::StopExpectation::unattended(
            id,
            card.uuid.clone(),
            identity,
            deadline,
            server,
        );
        Ok(menu::Question::new(card, expect))
    }

    /// The End question: the modal is the confirmation, inside tmux or not,
    /// so what tmux and the record say of the session is frozen in it from
    /// the server the record names, for the owner to prove under the lock.
    pub(super) fn end_question(&self, mut card: menu::Card) -> Result<menu::EndQuestion, String> {
        let root = (self.root.as_ref()).ok_or("ae app has no state root to end under")?;
        let (_, server) = self.recorded(&card.name, &card.uuid)?;
        let deadline = Timestamp::now().epoch() + crate::session_menu::CONFIRM_WINDOW_SECS;
        let expect = crate::lifecycle::end::EndExpectation::unattended(
            root,
            &card.name,
            &card.uuid,
            server.clone(),
            card.stopped,
            deadline,
        )?;
        if let Some(live) = expect.live_id() {
            if card.stopped {
                return Err("it was started since the menu was drawn".to_owned());
            }
            if card.id.as_ref().is_some_and(|painted| painted != live) {
                return Err("its tmux session was replaced since the menu was drawn".to_owned());
            }
            card.home = self.runs_in(&server, live);
        }
        Ok(menu::EndQuestion::new(card, expect))
    }

    /// Whether this app's own pane is in the tmux session `id` on `server`.
    fn runs_in(&self, server: &ServerId, id: &str) -> bool {
        let (Some(own), Some(pane)) = (&self.server, &self.pane) else {
            return false;
        };
        crate::fleet_read::SocketPaths::live().proven_same(own, server)
            && transport::observe_pane_session_identity(own, pane)
                .is_some_and(|session| session.id == id)
    }

    /// The End question answered End: its deadline and the record proven once
    /// more against what it froze, never captured again, then the owner,
    /// detached. The journal's length is taken first: the reader believes only
    /// what is written past it, and only of the incarnation that was asked.
    fn end(&self, question: &menu::EndQuestion) -> Result<String, String> {
        let (expect, name) = (&question.expect, &question.card.name);
        if Timestamp::now().epoch() > expect.deadline() {
            return Err("the question expired - ask again".to_owned());
        }
        let root = (self.root.as_ref()).ok_or("ae app has no state root to end under")?;
        let (dir, server) = self.recorded(name, expect.uuid())?;
        if server != *expect.server() {
            return Err("it is recorded on a different tmux server than the one the question was asked about".to_owned());
        }
        let position = match crate::store::open(&dir).events_source() {
            crate::store::SourceRead::Absent => 0,
            crate::store::SourceRead::Ready(bytes) => bytes.len(),
            crate::store::SourceRead::Invalid(why) | crate::store::SourceRead::Unreadable(why) => {
                return Err(format!("ae cannot read its journal: {why}"));
            }
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let detach = |argv: &crate::lifecycle::DetachedArgv| (self.detach)(argv);
        let code = crate::lifecycle::end::end_confirmed_via(
            root, expect, name, detach, &mut out, &mut err,
        )
        .map_err(|why| why.to_string())?;
        out.extend(err);
        let line = String::from_utf8_lossy(&out).trim().to_owned();
        if code != 0 {
            return Err(line);
        }
        let ask = Box::new(EndingAsk {
            name: name.clone(),
            dir,
            uuid: expect.uuid().to_owned(),
            position,
            asked: Instant::now(),
        });
        if let Some(tx) = &self.ask {
            let _ = tx.send(Request::Ending(ask));
        }
        Ok(line)
    }

    /// Start `card`'s stopped session outside tmux: the session resumed by name
    /// with nothing attached, detached from this app, once its record still
    /// names the incarnation the card was drawn from. No client moves.
    fn start_unattended(&self, card: &menu::Card) -> Result<String, String> {
        let name = &card.name;
        let dir = (self.dirs.get(name)).ok_or("ae read no record directory for it")?;
        if loader::sole_uuid(dir).as_deref() != Some(card.uuid.as_str()) {
            return Err(REPLACED.to_owned());
        }
        let named = crate::session_launch::name::is_session_name(name)
            && !crate::entry::name_is_routed_verb(name);
        if !named {
            return Err("ae cannot resume a session of that name from here".to_owned());
        }
        let argv =
            crate::lifecycle::resume_argv(name).ok_or("ae cannot name its own executable")?;
        if !(self.detach)(&argv) {
            return Err("ae could not start the resume job".to_owned());
        }
        Ok(format!("Starting '{name}'."))
    }

    /// Start `card`'s stopped session: the picker row's own resume job — the
    /// ordinary launch owner behind its captured-client proofs — run in the
    /// background by tmux. Its last step moves this client to the session.
    pub(super) fn start(&self, card: &menu::Card) -> Result<String, String> {
        if !self.attended() {
            return self.start_unattended(card);
        }
        let asker = self.asker(card)?;
        let launcher =
            crate::picker_launcher_for(&asker.server).ok_or("ae cannot name its own executable")?;
        let resume = crate::orchestrator::PickerResume {
            client: &asker.client,
            client_pid: &asker.client_pid,
            server_pid: &asker.server_pid,
            server_start: &asker.server_start,
            deadline: Timestamp::now().epoch() + crate::session_menu::CONFIRM_WINDOW_SECS,
            launcher: &launcher,
        };
        let argv = crate::orchestrator::resume_argv(&card.name, &resume);
        if !transport::run_shell_background(&asker.server, &argv) {
            return Err("tmux refused the start job".to_owned());
        }
        Ok(format!("Starting '{}'.", card.name))
    }

    /// A question answered yes. Stop: its deadline, the record and the asking
    /// client proven once more against what it froze, never captured again,
    /// then the one guarded Stop owner, detached. End: [`App::end`].
    pub(super) fn affirm(&mut self) {
        match self.modal.take() {
            Some(menu::Modal::Question(question)) => {
                let name = question.card.name.clone();
                let line = (self.stop(&question))
                    .unwrap_or_else(|why| format!("refused: Stop {name}: {why}; nothing stopped"));
                self.tell(&name, line);
            }
            Some(menu::Modal::End(question)) => {
                let name = question.card.name.clone();
                let line = (self.end(&question))
                    .unwrap_or_else(|why| format!("refused: End {name}: {why}; nothing ended"));
                self.tell(&name, line);
            }
            _ => {}
        }
    }

    /// [`App::affirm`]'s proofs, then the owner's own line.
    fn stop(&self, question: &menu::Question) -> Result<String, String> {
        let (expect, name) = (&question.expect, &question.card.name);
        if Timestamp::now().epoch() > expect.deadline {
            return Err("the question expired - ask again".to_owned());
        }
        let dir = (self.dirs.get(name)).ok_or("ae read no record directory for it")?;
        if loader::sole_uuid(dir).as_deref() != Some(expect.uuid.as_str()) {
            return Err(REPLACED.to_owned());
        }
        if !expect.is_attended() {
            return self.stop_unattended(question);
        }
        let (Some(pane), Some(root)) = (&self.pane, &self.root) else {
            return Err(Refusal::NoTmux.why());
        };
        let clients =
            transport::observe_clients(&expect.server).ok_or("the tmux clients did not answer")?;
        let client = open::copy_client(&clients, pane).map_err(|why| why.why())?;
        let pid = transport::observe_menu_client(&expect.server, &client).map(|asker| asker.pid);
        if client != expect.client || pid.as_ref() != Some(&expect.client_pid) {
            return Err("the client that was asked changed or is gone".to_owned());
        }
        let (mut out, mut err) = (Vec::new(), Vec::new());
        crate::lifecycle::stop_confirmed(root, expect, name, &mut out, &mut err)
            .map_err(|why| why.to_string())?;
        out.extend(err);
        Ok(String::from_utf8_lossy(&out).trim().to_owned())
    }

    /// [`App::stop`] for a question asked outside tmux: the record must still
    /// name the incarnation and the very server the question froze, then the
    /// supervisor is handed the frozen identity to prove under the lock.
    fn stop_unattended(&self, question: &menu::Question) -> Result<String, String> {
        let (expect, name) = (&question.expect, &question.card.name);
        let root = self
            .root
            .as_ref()
            .ok_or("ae app has no state root to stop under")?;
        let (_, server) = self.recorded(name, &expect.uuid)?;
        if server != expect.server {
            return Err("it is recorded on a different tmux server than the one the question was asked about".to_owned());
        }
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let detach = |argv: &crate::lifecycle::DetachedArgv| (self.detach)(argv);
        crate::lifecycle::stop_confirmed_via(root, expect, name, detach, &mut out, &mut err)
            .map_err(|why| why.to_string())?;
        out.extend(err);
        Ok(String::from_utf8_lossy(&out).trim().to_owned())
    }
}
