//! `_orchestrator-jump` — the `prefix O` key: take the pressing client to the
//! orchestrator's lead pane, and back to the exact view it left.
//!
//! Every fact of the press is argv expanded by tmux at the keypress. The way
//! back is ONE option on the orchestrator, `@ae_return_<client pid>`. Exit 0
//! means a client read after the move SAW the landing. No other client changes
//! session; co-viewers share tmux's one current window and active pane.

use std::io::Write;

use crate::console::open::{Landing, MOVED, SWITCHED, client_name, guarded_move_args};
use crate::inventory::ServerId;
use crate::transport;

/// The option prefix of a return record; the client pid follows.
const RECORD_PREFIX: &str = "@ae_return_";

/// The longest client name a record carries.
const NAME_MAX: usize = 255;

/// One client's way back, as stored in its option.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub client_name: String,
    pub client_pid: String,
    pub session_id: String,
    pub window_id: String,
    pub pane_id: String,
    pub nonce: String,
}

impl Record {
    /// `1|name|pid|$s|@w|%p|nonce`: exactly seven fields, each in its grammar,
    /// or `None`. Every byte that survives is plain in a tmux format and
    /// command, which is what lets the clear compare it inside tmux.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let fields: Vec<&str> = text.split('|').collect();
        let ["1", name, pid, session, window, pane, nonce] = fields.as_slice() else {
            return None;
        };
        let hex = nonce
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        (nonce.len() == 16 && hex).then_some(())?;
        let origin = Self::origin(name, pid, session, window, pane)?;
        Some(Self {
            nonce: (*nonce).to_owned(),
            ..origin
        })
    }

    /// A client's view with no nonce yet: the same grammar the record keeps.
    fn origin(name: &str, pid: &str, session: &str, window: &str, pane: &str) -> Option<Self> {
        let valid = name.len() <= NAME_MAX
            && client_name(name)
            && is_pid(pid)
            && is_id(session, '$')
            && is_id(window, '@')
            && is_id(pane, '%');
        valid.then(|| Self {
            client_name: name.to_owned(),
            client_pid: pid.to_owned(),
            session_id: session.to_owned(),
            window_id: window.to_owned(),
            pane_id: pane.to_owned(),
            nonce: String::new(),
        })
    }

    #[must_use]
    pub fn render(&self) -> String {
        format!(
            "1|{}|{}|{}|{}|{}|{}",
            self.client_name,
            self.client_pid,
            self.session_id,
            self.window_id,
            self.pane_id,
            self.nonce
        )
    }
}

/// A decimal pid: no sign, no leading zero, at most ten digits.
fn is_pid(text: &str) -> bool {
    (1..=10).contains(&text.len())
        && !text.starts_with('0')
        && text.bytes().all(|b| b.is_ascii_digit())
}

/// A tmux id: `sigil` and one or more digits.
fn is_id(text: &str, sigil: char) -> bool {
    text.strip_prefix(sigil)
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// The press, as the binding captured it: the server pair and the view.
struct Press {
    server_pid: String,
    server_start: String,
    origin: Record,
}

/// The flag words of the press, in their fixed slots.
const FLAGS: [&str; 5] = [
    "--client",
    "--client-pid",
    "--server-pid",
    "--server-start",
    "--from",
];

fn parse_press(tail: &[String]) -> Option<Press> {
    let [f1, cl, f2, cp, f3, sp, f4, st, f5, s, w, p] = tail else {
        return None;
    };
    let valid = [f1, f2, f3, f4, f5].map(String::as_str) == FLAGS
        && is_pid(sp)
        && st.bytes().all(|b| b.is_ascii_digit());
    valid.then_some(())?;
    Some(Press {
        server_pid: sp.clone(),
        server_start: st.clone(),
        origin: Record::origin(cl, cp, s, w, p)?,
    })
}

/// One pane of the server-wide listing.
struct Pane {
    session: String,
    name: String,
    window: String,
    id: String,
    live: bool,
}

/// One attached client: who it is and where it stands.
struct Client {
    name: String,
    pid: String,
    session_id: String,
    pane_id: String,
}

/// The verb: exit 0 only once the client is seen where it was sent.
pub(crate) fn run(tail: &[String], err: &mut impl Write) -> crate::Result<u8> {
    let Some(press) = parse_press(tail) else {
        writeln!(
            err,
            "usage: ae {} --client <name> --client-pid <pid> --server-pid <pid> \
             --server-start <t> --from <$s> <@w> <%p>",
            crate::cli::ORCHESTRATOR_JUMP
        )?;
        return Ok(crate::entry::EXIT_USAGE);
    };
    let declared = crate::doors::declared_server(crate::shape::current());
    let Some(server) = crate::doors::launch_target(declared.as_ref()) else {
        writeln!(err, "ae: the tmux server pair is ambiguous")?;
        return Ok(crate::entry::EXIT_FAILED);
    };
    let jump = Jump { server, press };
    // A client ae cannot prove is never told anything.
    if let Err(why) = jump.prove() {
        writeln!(err, "ae: {why}; nothing moved")?;
        return Ok(crate::entry::EXIT_FAILED);
    }
    let outcome = jump.act();
    if let Some(line) = &outcome.say {
        jump.say(line);
        writeln!(err, "ae: {line}")?;
    }
    Ok(if outcome.landed {
        0
    } else {
        crate::entry::EXIT_FAILED
    })
}

struct Jump {
    server: ServerId,
    press: Press,
}

/// What the press did: whether the client was seen where it was sent, and
/// the line it is shown.
struct Outcome {
    landed: bool,
    say: Option<String>,
}

/// What a client read after a move showed.
enum Seen {
    /// The client stands in this session, at this pane.
    In(String, String),
    /// The read failed, or no row names this client.
    Unknown,
}

/// The line when the read after a move cannot place the client: the way back
/// stays, because nothing proves the client did not move.
const UNSEEN: &str = "could not see where you landed; the way back is kept";

fn refused(line: &str) -> Outcome {
    Outcome {
        landed: false,
        say: Some(line.to_owned()),
    }
}

/// The orchestrator a jump may reach: its session, its uuid and its lead.
struct Orchestrator {
    session_id: String,
    uuid: String,
    lead: String,
    lead_window: String,
}

impl Jump {
    fn tmux(&self, words: &[&str]) -> (bool, String) {
        let mut args = crate::tmux::server_args(&self.server);
        args.extend(words.iter().map(|word| (*word).to_owned()));
        transport::run_jump(&self.server, &args)
    }

    /// The press's server answers with the same pid and start time, and
    /// exactly one attached client carries its name, as the same process. A
    /// client name is a tty path a later attachment can reuse; the pid tells
    /// them apart. A control client draws nothing, so no size is asked.
    fn prove(&self) -> Result<(), String> {
        let p = &self.press;
        crate::session_launch::prove_server(&self.server, &p.server_pid, &p.server_start)?;
        let named = self.clients().map(|clients| {
            clients
                .into_iter()
                .filter(|c| c.name == p.origin.client_name)
                .collect::<Vec<_>>()
        });
        match named.as_deref() {
            Some([c]) if c.pid == p.origin.client_pid => Ok(()),
            Some([_]) => Err("the invoking client is a different attachment now".to_owned()),
            _ => Err("the invoking client is no longer attached".to_owned()),
        }
    }

    fn say(&self, line: &str) {
        let text = format!("ae: {line}");
        let client = &self.press.origin.client_name;
        let _ = self.tmux(&["display-message", "-c", client, &text]);
    }

    fn act(&self) -> Outcome {
        let Some(panes) = self.panes() else {
            return refused("tmux did not answer the pane listing; nothing moved");
        };
        let Some(orch) = self.orchestrator(&panes) else {
            return refused("no orchestrator running on this server");
        };
        let option = format!("{RECORD_PREFIX}{}", self.press.origin.client_pid);
        if let Some(clients) = self.clients() {
            self.prune(&orch.session_id, &clients);
        }
        if self.press.origin.session_id == orch.session_id {
            self.back(&orch, &panes, &option)
        } else {
            self.go(&orch, &option)
        }
    }

    /// JUMP: record the way back, read it back, then move to the lead. The
    /// record is cleared only when a read SAW the client still at its origin.
    fn go(&self, orch: &Orchestrator, option: &str) -> Outcome {
        let origin = &self.press.origin;
        let nonce = crate::launch::generate_uuid().replace('-', "");
        let record = Record {
            nonce: nonce.chars().take(16).collect(),
            ..origin.clone()
        };
        let value = record.render();
        let wrote = self
            .tmux(&["set-option", "-t", &orch.session_id, option, &value])
            .0;
        if !wrote || self.option(&orch.session_id, option).as_deref() != Some(value.as_str()) {
            self.clear(&orch.session_id, option, &value);
            return refused("could not record where you were; nothing moved");
        }
        let guard = tuple_guard(&orch.session_id, &orch.lead_window, Some(&orch.uuid));
        let landing = self.guarded(&orch.session_id, &orch.lead, &guard);
        match (landing, self.seen()) {
            (Landing::Selected, Seen::In(sid, pane))
                if sid == orch.session_id && pane == orch.lead =>
            {
                Outcome {
                    landed: true,
                    say: None,
                }
            }
            (_, Seen::In(sid, _)) if sid == orch.session_id => {
                refused("reached the orchestrator, but not its lead pane")
            }
            (_, Seen::In(sid, _)) if sid == origin.session_id => {
                self.clear(&orch.session_id, option, &value);
                refused("the orchestrator changed before the jump; nothing moved")
            }
            _ => refused(UNSEEN),
        }
    }

    /// RETURN: the exact view when its whole tuple still holds, else the
    /// session the record names, re-proved by id; never anything else.
    fn back(&self, orch: &Orchestrator, panes: &[Pane], option: &str) -> Outcome {
        let Some(value) = self.option(&orch.session_id, option) else {
            return refused("nothing to return to");
        };
        let Some(record) = Record::parse(&value) else {
            return refused("return record unreadable; nothing moved");
        };
        if (&record.client_name, &record.client_pid)
            != (
                &self.press.origin.client_name,
                &self.press.origin.client_pid,
            )
        {
            return refused("the return record belongs to another client; nothing moved");
        }
        let Some(origin) = panes.iter().find(|pane| pane.session == record.session_id) else {
            return refused("the session you left is gone; staying here");
        };
        let exact = panes.iter().any(|pane| {
            pane.session == record.session_id
                && pane.window == record.window_id
                && pane.id == record.pane_id
                && pane.live
        });
        if exact {
            let guard = tuple_guard(&record.session_id, &record.window_id, None);
            let landing = self.guarded(&record.session_id, &record.pane_id, &guard);
            return match (landing, self.seen()) {
                (Landing::Selected, Seen::In(sid, pane))
                    if sid == record.session_id && pane == record.pane_id =>
                {
                    self.clear(&orch.session_id, option, &value);
                    Outcome {
                        landed: true,
                        say: None,
                    }
                }
                (_, Seen::In(sid, _)) if sid == record.session_id => refused(&format!(
                    "the pane you left moved while you returned; back in {}",
                    origin.name
                )),
                (_, Seen::In(sid, _)) if sid == orch.session_id => {
                    refused("the view you left changed before the return; nothing moved")
                }
                _ => refused(UNSEEN),
            };
        }
        let sid = &record.session_id;
        let switch = crate::tmux::switch_client_id_command(&self.press.origin.client_name, sid);
        let landing = landing(self.tmux(&[
            "if-shell",
            "-F",
            "-t",
            sid,
            &format!("#{{==:#{{session_id}},{sid}}}"),
            &format!("{switch} ; display-message -p {SWITCHED}"),
            &format!("display-message -p {MOVED}"),
        ]));
        match (landing, self.seen()) {
            (Landing::Switched, Seen::In(at, _)) if at == *sid => {
                self.clear(&orch.session_id, option, &value);
                Outcome {
                    landed: true,
                    say: Some(format!(
                        "the pane you left is gone; back in {}",
                        origin.name
                    )),
                }
            }
            (_, Seen::In(at, _)) if at == orch.session_id => {
                refused("the session you left changed before the return; nothing moved")
            }
            _ => refused(UNSEEN),
        }
    }

    /// The guarded move of this client to `pane` of `session_id`.
    fn guarded(&self, session_id: &str, pane: &str, guard: &str) -> Landing {
        let args = guarded_move_args(
            &self.server,
            session_id,
            pane,
            guard,
            Some(&self.press.origin.client_name),
        );
        landing(transport::run_jump(&self.server, &args))
    }

    /// Where this client (name AND pid) now stands, or `Unknown` when the
    /// read failed or no row names it: a failed read proves nothing moved.
    fn seen(&self) -> Seen {
        let origin = &self.press.origin;
        self.clients()
            .and_then(|clients| {
                clients
                    .into_iter()
                    .find(|c| c.name == origin.client_name && c.pid == origin.client_pid)
            })
            .map_or(Seen::Unknown, |c| Seen::In(c.session_id, c.pane_id))
    }

    /// Unset `option` only if it still holds `value`: the compare and the
    /// unset run in ONE tmux command, so a newer record survives.
    fn clear(&self, session_id: &str, option: &str, value: &str) {
        if Record::parse(value).is_none() {
            return;
        }
        let _ = self.tmux(&[
            "if-shell",
            "-F",
            "-t",
            session_id,
            &format!("#{{==:#{{{option}}},{value}}}"),
            &format!("set-option -u -t {session_id} {option}"),
        ]);
    }

    /// Drop the records of clients that are gone, from a COMPLETE client read
    /// only; a record of a live pid is that client's to use or refuse.
    fn prune(&self, session_id: &str, clients: &[Client]) {
        let (ok, out) = self.tmux(&["show-options", "-t", session_id]);
        if !ok {
            return;
        }
        for line in out.lines() {
            let Some(pid) = line
                .split_whitespace()
                .next()
                .and_then(|name| name.strip_prefix(RECORD_PREFIX))
                .filter(|pid| is_pid(pid))
            else {
                continue;
            };
            if clients.iter().any(|c| c.pid == pid) {
                continue;
            }
            let option = format!("{RECORD_PREFIX}{pid}");
            if let Some(value) = self.option(session_id, &option) {
                self.clear(session_id, &option, &value);
            }
        }
    }

    fn option(&self, session_id: &str, option: &str) -> Option<String> {
        let (ok, out) = self.tmux(&["show-options", "-qv", "-t", session_id, option]);
        let value = out.strip_suffix('\n').unwrap_or(&out);
        (ok && !value.is_empty()).then(|| value.to_owned())
    }

    /// The orchestrator on THIS server: `=orchestrator`, stamped with the uuid
    /// its meta records, its lead pane a live member.
    fn orchestrator(&self, panes: &[Pane]) -> Option<Orchestrator> {
        let name = crate::orchestrator::ORCHESTRATOR_SESSION;
        let session_id = transport::observe_session_id(&self.server, name)?;
        let stamp =
            transport::observe_session_option(&self.server, name, crate::theme::SESSION_ID_OPTION)?;
        let recorded = crate::state_root()
            .and_then(|root| crate::console::locate(&root, name))
            .map(|dir| crate::console::recorded_uuid(&dir))?;
        if recorded.is_empty() || recorded != stamp {
            return None;
        }
        let lead =
            transport::observe_session_option(&self.server, name, crate::theme::MAIN_PANE_OPTION)?;
        let row = panes
            .iter()
            .find(|pane| pane.id == lead && pane.session == session_id && pane.live)?;
        Some(Orchestrator {
            lead_window: row.window.clone(),
            session_id,
            uuid: recorded,
            lead,
        })
    }

    fn panes(&self) -> Option<Vec<Pane>> {
        let (ok, out) = self.tmux(&[
            "list-panes",
            "-a",
            "-F",
            "#{session_id}|#{window_id}|#{pane_id}|#{pane_dead}|#{session_name}",
        ]);
        ok.then(|| {
            out.lines()
                .filter_map(|line| {
                    let [s, w, p, dead, name] = line.splitn(5, '|').collect::<Vec<_>>()[..] else {
                        return None;
                    };
                    Some(Pane {
                        session: s.to_owned(),
                        name: name.to_owned(),
                        window: w.to_owned(),
                        id: p.to_owned(),
                        live: dead == "0",
                    })
                })
                .collect()
        })
    }

    /// Every attached client, or `None` when tmux failed or a row is damaged:
    /// a partial read is no read.
    fn clients(&self) -> Option<Vec<Client>> {
        let (ok, out) = self.tmux(&[
            "list-clients",
            "-F",
            "#{client_name}|#{client_pid}|#{session_id}|#{pane_id}",
        ]);
        if !ok {
            return None;
        }
        out.lines()
            .map(|line| match line.split('|').collect::<Vec<_>>()[..] {
                [name, pid, session_id, pane_id] if is_pid(pid) => Some(Client {
                    name: name.to_owned(),
                    pid: pid.to_owned(),
                    session_id: session_id.to_owned(),
                    pane_id: pane_id.to_owned(),
                }),
                _ => None,
            })
            .collect()
    }
}

/// The guard a move re-proves at its pane: the session and window it was
/// proved in, the pane alive, and (for the orchestrator) its uuid stamp.
/// Every conjunction binary and nested: tmux 3.4 reads two operands only.
fn tuple_guard(session_id: &str, window_id: &str, uuid: Option<&str>) -> String {
    let alive = "#{==:#{pane_dead},0}";
    let inner = uuid.map_or_else(
        || alive.to_owned(),
        |uuid| format!("#{{&&:#{{==:#{{@ae_session_uuid}},{uuid}}},{alive}}}"),
    );
    format!(
        "#{{&&:#{{==:#{{session_id}},{session_id}}},#{{&&:#{{==:#{{window_id}},{window_id}}},{inner}}}}}"
    )
}

fn landing((succeeded, stdout): (bool, String)) -> Landing {
    Landing::read(succeeded, &stdout)
}
