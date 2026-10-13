//! Frozen openfix acceptance: a cross-session open lands on the seat pane.
//! Oracle: brief openfix I1-I5 + lead r1 (a)(b)(c). Base 62b93fca is RED on
//! the open, switched, dead-seat and picker arms; plain-switch arms pin I2.
//! Every socket, session, client and hook belongs to this fixture. Hook bytes
//! mirror `session_tmux.rs::SetClientSessionHook` (chat variant).

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns a private tmux server, clients and hook fixtures"
)]

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ae::console::needs::SeatRef;
use ae::console::open::{self, Target};
use ae::inventory::ServerId;
use ae::meta::Selector;

const UUID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
const PATIENCE: Duration = Duration::from_secs(20);

/// The launch-stamped focus hook, chat variant, for `lead` of `sid`.
fn hook(lead: &str, sid: &str, chat: &str) -> String {
    format!(
        "if-shell -F -t {lead} \"#{{==:#{{session_id}},{sid}}}\" {{ if-shell -F -t '{sid}:^' \
         \"#{{P:#{{?#{{&&:#{{==:#{{pane_id}},{chat}}},#{{&&:#{{==:#{{@ae_console}},{UUID}}},\
         #{{!=:#{{pane_dead}},1}}}}}},1,}}}}\" {{ select-window -t {chat} ; select-pane -t {chat} }} \
         {{ select-window -t {lead} ; select-pane -t {lead} }} }}"
    )
}

struct Rig {
    scratch: super::cli::OwnedScratch,
    socket: PathBuf,
    chat: String,
    lead: String,
    seat: String,
    sid: String,
}

impl Rig {
    /// Alpha plus beta; beta carries the real hook. Without `live_chat` the
    /// chat pane is unstamped, so the hook falls through to lead (I2).
    fn new(tag: &str, live_chat: bool) -> Self {
        let mut scratch = super::cli::OwnedScratch::root("openhook", tag);
        let socket = scratch.join("s");
        scratch.add_tmux_server(socket.clone());
        let mut rig = Self {
            scratch,
            socket,
            chat: String::new(),
            lead: String::new(),
            seat: String::new(),
            sid: String::new(),
        };
        rig.tmux("-f /dev/null new-session -d -s alpha sleep 600");
        rig.chat = rig.tmux("-f /dev/null new-session -d -P -F #{pane_id} -s beta sleep 600");
        rig.lead = rig.tmux("new-window -d -P -F #{pane_id} -t beta -n lead sleep 600");
        rig.seat = rig.tmux("new-window -d -P -F #{pane_id} -t beta -n scout sleep 600");
        rig.tmux(&format!("set-option -t beta @ae_session_uuid {UUID}"));
        if live_chat {
            rig.tmux(&format!("set-option -p -t {} @ae_console {UUID}", rig.chat));
        }
        rig.tmux(&format!("set-option -p -t {} @ae_slot spawned.0", rig.seat));
        rig.tmux(&format!("set-option -p -t {} @ae_agent scout", rig.seat));
        rig.sid = rig.tmux("display-message -p -t beta: #{session_id}");
        let armed = hook(&rig.lead, &rig.sid, &rig.chat);
        rig.tmux_raw(&[
            "set-hook",
            "-t",
            &rig.lead,
            "client-session-changed",
            &armed,
        ]);
        let shown = rig.tmux("show-hooks -t beta client-session-changed");
        assert!(
            shown.contains("client-session-changed"),
            "hook armed: {shown}"
        );
        rig
    }

    /// One tmux call; `line` splits on ASCII whitespace (setup words never
    /// contain any). Words WITH spaces go through [`Rig::tmux_raw`].
    fn tmux(&self, line: &str) -> String {
        let mut args = vec!["-S".to_owned(), self.socket.display().to_string()];
        args.extend(line.split_whitespace().map(str::to_owned));
        self.run(&args, line)
    }

    fn tmux_raw(&self, tail: &[&str]) -> String {
        let mut args = vec!["-S".to_owned(), self.socket.display().to_string()];
        args.extend(tail.iter().map(|word| (*word).to_owned()));
        let what = tail.join(" ");
        self.run(&args, &what)
    }

    fn run(&self, args: &[String], what: &str) -> String {
        let (ok, text) = super::phase2::run_tmux(args, self.scratch.path());
        assert!(ok, "private tmux {what}: {text}");
        text.trim().to_owned()
    }

    fn append_hook(&self, body: &str) {
        self.tmux_raw(&[
            "set-hook",
            "-a",
            "-t",
            &self.lead,
            "client-session-changed",
            body,
        ]);
    }

    fn attach(&self, session: &str) -> (super::cli::OwnedChild, String) {
        let client = super::cli::tmux_attached_client(&self.socket, session).expect("real client");
        let pid = client.id().to_string();
        Self::wait("client listed", || {
            self.tmux("list-clients -F #{client_pid}")
                .lines()
                .any(|row| row == pid)
        });
        let name = self
            .tmux("list-clients -F #{client_pid}|#{client_name}")
            .lines()
            .find_map(|row| row.strip_prefix(&format!("{pid}|")).map(str::to_owned))
            .expect("client named");
        (client, name)
    }

    fn views(&self, client: &super::cli::OwnedChild) -> (String, String) {
        let pid = client.id().to_string();
        self.tmux("list-clients -F #{client_pid}|#{session_name}|#{pane_id}")
            .lines()
            .find_map(|row| {
                let mut f = row.split('|');
                (f.next()? == pid).then(|| {
                    (
                        f.next().expect("session").to_owned(),
                        f.next().expect("pane").to_owned(),
                    )
                })
            })
            .expect("client stays listed")
    }

    fn wait(what: &str, mut settled: impl FnMut() -> bool) {
        let until = Instant::now() + PATIENCE;
        while Instant::now() < until {
            if settled() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("{what} never settled");
    }

    fn open(&self, name: &str) -> (bool, String) {
        let server = ServerId::Selected(Selector::Socket(self.socket.clone()));
        let target = Target {
            seat: SeatRef {
                slot: "spawned.0".to_owned(),
                name: "scout".to_owned(),
            },
            session_id: self.sid.clone(),
            pane: self.seat.clone(),
            uuid: UUID.to_owned(),
        };
        ae::transport::open_seat_via(&server, &target, Some(name)).expect("proven target builds")
    }
}

/// I1: the opening client rests on the proven seat pane, and SELECTED says so.
#[test]
fn cross_session_open_lands_on_the_seat_pane_with_a_live_hook() {
    let rig = Rig::new("i1", true);
    let (client, name) = rig.attach("alpha");
    let (ran, out) = rig.open(&name);
    assert!(ran, "tmux ran the open");
    assert_eq!(out.trim_end(), "ae-open:selected", "guard held");
    Rig::wait("client on the seat pane", || {
        rig.views(&client).1 == rig.seat
    });
}

/// Lead r1 (a): a restamp between the switch and the deferred select reports
/// `switched` and holds the client on chat — never a false `opened`.
#[test]
fn restamp_between_switch_and_select_reports_switched_and_holds_chat() {
    let rig = Rig::new("switched", true);
    rig.append_hook(&format!("set-option -p -t {} @ae_agent other", rig.seat));
    let (client, name) = rig.attach("alpha");
    let (ran, out) = rig.open(&name);
    assert!(ran, "tmux ran the open");
    assert_eq!(out.trim_end(), "ae-open:switched", "inner guard re-proved");
    Rig::wait("client held on chat", || rig.views(&client).1 == rig.chat);
}

/// A seat dead at deferred time reports `switched`: `if-shell -t` on a gone
/// pane reads empty and refuses, it never errors onto `uncertain`.
#[test]
fn dead_seat_at_deferred_time_reports_switched() {
    let rig = Rig::new("deadseat", true);
    rig.append_hook(&format!("kill-pane -t {}", rig.seat));
    let (client, name) = rig.attach("alpha");
    let (ran, out) = rig.open(&name);
    assert!(ran, "tmux ran the open");
    assert_eq!(
        out.trim_end(),
        "ae-open:switched",
        "dead seat refuses inward"
    );
    Rig::wait("client held on chat", || rig.views(&client).1 == rig.chat);
}

/// I2: a plain switch into a session lands on its live chat pane.
#[test]
fn plain_switch_lands_on_the_live_chat_pane() {
    let rig = Rig::new("i2chat", true);
    let (client, name) = rig.attach("alpha");
    rig.tmux(&format!("switch-client -c {name} -t {}", rig.sid));
    Rig::wait("client on chat", || rig.views(&client).1 == rig.chat);
}

/// I2: without a live stamped chat the same switch lands on lead.
#[test]
fn plain_switch_without_a_live_chat_lands_on_lead() {
    let rig = Rig::new("i2lead", false);
    let (client, name) = rig.attach("alpha");
    rig.tmux(&format!("switch-client -c {name} -t {}", rig.sid));
    Rig::wait("client on lead", || rig.views(&client).1 == rig.lead);
}

/// Picker agent rows carry the same deferral: the product's own emitted row
/// command, run verbatim, lands the client on the seat pane. `source-file`
/// preserves the row's quoting; the menu `##` layer is tmux-stable and pinned
/// by the driver's byte golden, so this arm pins the ordering, not the guard.
#[test]
fn picker_agent_row_lands_on_the_seat_pane_with_a_live_hook() {
    let rig = Rig::new("picker", true);
    let (client, name) = rig.attach("alpha");
    let now = ae::time::Timestamp::now().epoch();
    let session = ae::tmux::PickerSession {
        name: "beta".to_owned(),
        id: rig.sid.clone(),
        rank: 0,
        glyph: "·".to_owned(),
        main_pane: rig.lead.clone(),
        branch: String::new(),
        agents: format!("v1;{now};3600;scout:codex:working:{}", rig.seat),
        activity: None,
        created_at: None,
        goal: String::new(),
    };
    let panes = [ae::tmux::PickerPane {
        session_id: rig.sid.clone(),
        pane: rig.seat.clone(),
    }];
    let menu = ae::orchestrator::menu_for_client_session(
        &[session],
        &panes,
        false,
        &ae::theme::Palette::DARCULA,
        Some(&name),
        Some(rig.sid.as_str()),
    );
    let row = menu
        .items
        .iter()
        .filter_map(|item| match &item.action {
            ae::tmux::MenuAction::Run(command) => Some(command.clone()),
            _ => None,
        })
        .find(|command| command.contains(&format!("select-pane -t {}", rig.seat)))
        .expect("agent row runs the seat pane");
    let script = rig.scratch.join("row.tmux");
    fs::write(&script, format!("{row}\n")).expect("row script");
    rig.tmux(&format!("source-file {}", script.display()));
    Rig::wait("client on the seat pane", || {
        rig.views(&client).1 == rig.seat
    });
}

/// Lead r1 (c): the `switched` line tells the move and the missed seat, and
/// never counts as a took.
#[test]
fn switched_outcome_names_the_move_and_never_counts_as_took() {
    assert_eq!(
        open::outcome_moved(true, "ae-open:switched\n", "scout", "beta"),
        "moved to beta, but scout's pane changed after the read; the seat was not selected - prefix L goes back"
    );
    assert!(!open::took(true, "ae-open:switched\n"));
}

// The amended client argv pin rides as the :131 hunk in
// `app_open_pure_spec.rs`, same frozen patch per lead r1: one guarded command
// whose select re-proves the seat AFTER the hook drained. This file pins the
// behaviour behind those bytes, not the bytes twice.
