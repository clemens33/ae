//! Frozen appopen acceptance. Oracle: brief R1-R7 and lead D1-D5/B1/B2/I1/I2.
//! Existing APIs only: real app input, real clients, and observed tmux state.
//! Every terminal, socket, journal and read gate belongs to this fixture.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private stores, tmux clients and terminal fixtures"
)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const HOME: &str = "openhome";
const FOREIGN: &str = "openforeign";
const UUID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
const OTHER_UUID: &str = "0199c0de-bbbb-4890-abcd-ef0123456789";
const WAIT: Duration = Duration::from_secs(20);

struct Rig {
    scratch: super::cli::OwnedScratch,
    socket: PathBuf,
    gates: PathBuf,
    lead: String,
    colead: String,
    scout: String,
}

impl Rig {
    fn new(tag: &str) -> Self {
        let mut scratch = super::cli::OwnedScratch::root("appopen", tag);
        let socket = scratch.join("sock");
        scratch.add_tmux_server(socket.clone());
        let gates = scratch.join("gates");
        fs::create_dir_all(&gates).expect("private read gates");
        fs::write(scratch.join("config"), "[workspace]\nchat = app\n").expect("app config");
        let mut rig = Self {
            scratch,
            socket,
            gates,
            lead: String::new(),
            colead: String::new(),
            scout: String::new(),
        };
        rig.lead = rig.session(HOME, UUID);
        rig.colead = rig.pane(HOME, "colead");
        rig.scout = rig.pane(HOME, "scout");
        for (pane, slot, name) in [
            (&rig.lead, "main", "lead"),
            (&rig.colead, "worker.0", "colead"),
            (&rig.scout, "spawned.0", "scout"),
        ] {
            rig.stamp(pane, slot, name);
        }
        rig.meta(HOME, UUID, &rig.socket, "spawned.0");
        rig.publish(
            HOME,
            &[
                ("lead", &rig.lead),
                ("colead", &rig.colead),
                ("scout", &rig.scout),
            ],
        );
        rig
    }

    fn at(&self, socket: &Path, tail: &[&str]) -> String {
        let mut args = vec!["-S".to_owned(), socket.display().to_string()];
        args.extend(tail.iter().map(|word| (*word).to_owned()));
        let (ok, text) = super::phase2::run_tmux(&args, self.scratch.path());
        assert!(ok, "private tmux {tail:?}: {text}");
        text
    }

    fn tmux(&self, tail: &[&str]) -> String {
        self.at(&self.socket, tail)
    }

    fn session(&self, name: &str, uuid: &str) -> String {
        let pane = self
            .tmux(&[
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-P",
                "-F",
                "#{pane_id}",
                "-s",
                name,
                "-x",
                "160",
                "-y",
                "45",
                "sleep",
                "600",
            ])
            .trim()
            .to_owned();
        for (option, value) in [
            ("@ae_session_uuid", uuid),
            ("@ae_look", "off"),
            ("@ae_motion", "off"),
            ("@ae_main_pane", pane.as_str()),
            ("@ae_attn_rank", "0"),
            ("@ae_attn_glyph", "idle"),
            ("status", "off"),
            ("default-size", "160x45"),
        ] {
            self.tmux(&["set-option", "-t", name, option, value]);
        }
        pane
    }

    fn pane(&self, session: &str, name: &str) -> String {
        self.tmux(&[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            session,
            "-n",
            name,
            "sleep",
            "600",
        ])
        .trim()
        .to_owned()
    }

    fn stamp(&self, pane: &str, slot: &str, name: &str) {
        for (option, value) in [("@ae_slot", slot), ("@ae_agent", name)] {
            self.tmux(&["set-option", "-p", "-t", pane, option, value]);
        }
    }

    fn meta(&self, name: &str, uuid: &str, socket: &Path, scout_slot: &str) {
        let dir = self.scratch.join("sessions").join(name);
        fs::create_dir_all(&dir).expect("session fixture");
        fs::write(dir.join("meta"), format!(
            "session={name}\nmode=local\nsession_id={uuid}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=codex\nlaunch_id.main=tok-lead\nseat.worker.0=colead\nagent_bin.worker.0=codex\nlaunch_id.worker.0=tok-colead\nseat.{scout_slot}=scout\nagent_bin.{scout_slot}=codex\nlaunch_id.{scout_slot}=tok-scout\n",
            socket.display()
        )).expect("recorded seat identities");
        fs::write(
            dir.join(".launch-attempt"),
            ae::time::Timestamp::now().epoch().to_string(),
        )
        .expect("launch evidence");
        if !dir.join("events.jsonl").exists() {
            fs::write(dir.join("events.jsonl"), "").expect("empty journal");
        }
        ae::watchdog_glue::touch_beat(&dir).expect("fresh watchdog beat");
    }

    fn publish(&self, session: &str, seats: &[(&str, &str)]) {
        let mut fact = format!("v1;{};3600", ae::time::Timestamp::now().epoch());
        for (name, pane) in seats {
            write!(fact, ";{name}:idle:working:{pane}").expect("fixture fact formatting");
        }
        self.tmux(&["set-option", "-t", session, "@ae_agents", &fact]);
    }

    fn foreign(&self) -> String {
        let lead = self.session(FOREIGN, OTHER_UUID);
        self.stamp(&lead, "main", "lead");
        let colead = self.pane(FOREIGN, "colead");
        self.stamp(&colead, "worker.0", "colead");
        let scout = self.pane(FOREIGN, "scout");
        self.stamp(&scout, "spawned.0", "scout");
        self.meta(FOREIGN, OTHER_UUID, &self.socket, "spawned.0");
        self.publish(
            FOREIGN,
            &[("lead", &lead), ("colead", &colead), ("scout", &scout)],
        );
        scout
    }

    fn app(&self, launch: &Path) -> String {
        self.app_with_caller(launch, true)
    }

    fn app_with_caller(&self, launch: &Path, inside_tmux: bool) -> String {
        let root = self.scratch.path().display().to_string();
        let config = self.scratch.join("config").display().to_string();
        let gate = self.gates.display().to_string();
        let redirect = launch.display().to_string();
        let mut command = vec![
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            HOME,
            "-n",
            "chat",
            "env",
            "-u",
            "CLAUDE_CONFIG_DIR",
            "-u",
            "CODEX_HOME",
        ];
        if !inside_tmux {
            command.extend(["-u", "TMUX", "-u", "TMUX_PANE"]);
        }
        let home_env = format!("HOME={root}");
        let ae_env = format!("AE_HOME={root}");
        let config_env = format!("CONFIG_FILE={config}");
        let redirect_env = format!("AE_TMUX_SERVER={redirect}");
        let gate_env = format!("AE_TEST_APP_READ_GATE={gate}");
        command.extend([
            &home_env,
            &ae_env,
            &config_env,
            "AE_TMUX_SERVER_KIND=socket",
            &redirect_env,
            &gate_env,
            env!("CARGO_BIN_EXE_ae"),
            "app",
            HOME,
        ]);
        let pane = self.tmux(&command).trim().to_owned();
        self.tmux(&["set-option", "-p", "-t", &pane, "@ae_console", UUID]);
        self.tmux(&["select-window", "-t", &pane]);
        self.tmux(&["select-pane", "-t", &pane]);
        self.wait_screen(&pane, "GUARD app painted home", |screen| {
            screen.contains(HOME) && screen.contains("Enter writes")
        });
        pane
    }

    fn attach(&self, session: &str) -> super::cli::OwnedChild {
        let client =
            super::cli::tmux_attached_client(&self.socket, session).expect("real attached client");
        let pid = client.id().to_string();
        Self::wait("GUARD client attached by its process id", || {
            self.tmux(&["list-clients", "-F", "#{client_pid}"])
                .lines()
                .any(|row| row == pid)
        });
        client
    }

    fn viewed(&self, client: &super::cli::OwnedChild) -> (String, String) {
        let pid = client.id().to_string();
        self.tmux(&[
            "list-clients",
            "-F",
            "#{client_pid}|#{session_name}|#{pane_id}",
        ])
        .lines()
        .find_map(|row| {
            let mut fields = row.split('|');
            (fields.next()? == pid).then(|| {
                (
                    fields.next().expect("session").to_owned(),
                    fields.next().expect("pane").to_owned(),
                )
            })
        })
        .expect("attached client remains listed")
    }

    fn activity(&self, client: &super::cli::OwnedChild) -> u64 {
        self.tmux(&["list-clients", "-F", "#{client_pid}|#{client_activity}"])
            .lines()
            .find_map(|row| row.strip_prefix(&format!("{}|", client.id())))
            .expect("client activity exists")
            .parse()
            .expect("tmux epoch is numeric")
    }

    fn tied(&self) -> (super::cli::OwnedChild, super::cli::OwnedChild) {
        let until = Instant::now() + WAIT;
        loop {
            let first = self.attach(HOME);
            let second = self.attach(HOME);
            if self.activity(&first) == self.activity(&second) {
                return (first, second);
            }
            assert!(
                Instant::now() < until,
                "GUARD two real control clients share one activity second"
            );
        }
    }

    fn opened(&self, client: &super::cli::OwnedChild, session: &str, pane: &str) {
        let app = self
            .tmux(&[
                "list-panes",
                "-s",
                "-t",
                HOME,
                "-F",
                "#{pane_id}|#{@ae_console}",
            ])
            .lines()
            .find_map(|row| {
                row.split_once('|')
                    .filter(|(_, stamp)| *stamp == UUID)
                    .map(|(id, _)| id.to_owned())
            })
            .expect("GUARD app pane remains stamped");
        let until = Instant::now() + WAIT;
        loop {
            let actual = self.viewed(client);
            if actual == (session.to_owned(), pane.to_owned()) {
                return;
            }
            assert!(
                Instant::now() < until,
                "R1/R2 human client lands in exact seat pane: actual={actual:?}, expected={session}:{pane}\n{}",
                self.screen(&app)
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn screen(&self, pane: &str) -> String {
        self.tmux(&["capture-pane", "-p", "-t", pane])
    }

    fn wait(description: &str, mut met: impl FnMut() -> bool) {
        let until = Instant::now() + WAIT;
        while !met() {
            assert!(Instant::now() < until, "{description}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn wait_screen(&self, pane: &str, description: &str, met: impl Fn(&str) -> bool) -> String {
        let until = Instant::now() + WAIT;
        loop {
            let result = self.screen(pane);
            if met(&result) {
                return result;
            }
            assert!(Instant::now() < until, "{description}\n{result}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn raw(&self, pane: &str, bytes: &str) {
        self.tmux(&["send-keys", "-t", pane, "-l", "--", bytes]);
    }

    fn key(&self, pane: &str, key: &str) {
        self.tmux(&["send-keys", "-t", pane, key]);
    }

    fn composer(&self, app: &str) {
        self.key(app, "Enter");
        self.wait_screen(app, "D2 Enter still enters composer", |s| {
            s.contains("Enter sends")
        });
        // A visible journal row from a NEW settled read proves that the
        // composer entered before the roster answer arrived. The marker
        // is fixture input, never an ask or a delivery to a seat.
        let selected = if self.screen(app).contains(&format!("to {FOREIGN}")) {
            FOREIGN
        } else {
            HOME
        };
        let journal = self
            .scratch
            .join("sessions")
            .join(selected)
            .join("events.jsonl");
        let mut rows = fs::read_to_string(&journal).expect("fixture journal");
        writeln!(
            rows,
            "{{\"ts\":\"{}\",\"actor\":\"ae:fixture\",\"action\":\"chat\",\"summary\":\"APPOPEN-COMPOSER-READY\"}}",
            ae::time::Timestamp::now()
        ).expect("fixture readiness row formatting");
        fs::write(journal, rows).expect("fixture readiness row");
        self.wait_screen(app, "GUARD settled roster after composer entry", |s| {
            s.contains("APPOPEN-COMPOSER-READY")
        });
    }

    fn submit_open(&self, app: &str, name: &str) {
        self.raw(app, &format!("/open {name}"));
        self.wait_screen(app, "GUARD complete explicit command was drawn", |s| {
            s.contains(&format!("/open {name}"))
        });
        self.key(app, "Enter");
    }

    fn compose_open(&self, app: &str, name: &str) {
        self.composer(app);
        self.submit_open(app, name);
    }

    fn click(&self, app: &str, needle: &str, body: bool) {
        let screen = self.screen(app);
        let tab = screen
            .lines()
            .position(|row| row.contains("Overview") && row.contains("Agents"));
        let row = screen
            .lines()
            .enumerate()
            .find_map(|(at, line)| {
                let side = line.split('│').next().unwrap_or("");
                (side.contains(needle) && (!body || tab.is_some_and(|tab| at > tab))).then_some(at)
            })
            .unwrap_or_else(|| panic!("GUARD drawn target {needle}:\n{screen}"));
        self.raw(app, &format!("\x1b[<0;9;{}M", row + 1));
    }

    fn agents(&self, app: &str) {
        self.key(app, "Tab");
        self.wait_screen(app, "GUARD Agents facts drawn", |s| {
            s.lines()
                .any(|row| row.split('│').next().unwrap_or("").contains("scout"))
        });
        self.wait_screen(app, "R5 drawn seats advertise their open action", |s| {
            s.contains("o open seat")
        });
    }

    fn focused(&self, app: &str, seat: &str) {
        self.wait_screen(app, "R4 focused seat visibly identified", |s| {
            let lines: Vec<&str> = s.lines().collect();
            let marked = |row: &str| row.starts_with('>') || row.starts_with('▸');
            let named = |row: &str| {
                row.split('│')
                    .next()
                    .unwrap_or_default()
                    .split_whitespace()
                    .any(|word| word == seat)
            };
            lines
                .windows(2)
                .any(|pair| pair.iter().all(|row| marked(row)) && pair.iter().any(|row| named(row)))
        });
    }

    fn notice(&self, app: &str, text: &str) {
        self.wait_screen(app, "R3 refusal/outcome is human-visible", |s| {
            s.contains(text)
        });
    }

    fn unchanged(&self, before: &[(&super::cli::OwnedChild, (String, String))]) {
        for (client, expected) in before {
            assert_eq!(
                self.viewed(client),
                *expected,
                "refusal cannot move a client"
            );
        }
    }
}

/// A read is held only after its actual entry point records a receipt.
struct Hold(PathBuf);

impl Hold {
    fn key(rig: &Rig, app: &str) -> Self {
        let path = rig.gates.join("@app-keys");
        let trace = rig.gates.join("trace");
        let count = fs::read_to_string(&trace)
            .unwrap_or_default()
            .lines()
            .count();
        fs::write(&path, "hold").expect("fixture gate");
        let hold = Self(path);
        rig.raw(app, "o");
        Rig::wait("GUARD the real app key read is held", || {
            fs::read_to_string(&trace)
                .unwrap_or_default()
                .lines()
                .skip(count)
                .any(|row| row == "held @app-keys")
        });
        hold
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[test]
fn home_composer_opens_spawned_seat_without_sending_an_ask() {
    let rig = Rig::new("home");
    let app = rig.app(&rig.socket);
    let client = rig.attach(HOME);
    rig.composer(&app);
    let journal = fs::read(rig.scratch.join("sessions").join(HOME).join("events.jsonl"))
        .expect("journal before");
    rig.submit_open(&app, "scout");
    rig.opened(&client, HOME, &rig.scout);
    rig.notice(&app, "opened scout - prefix h returns");
    assert_eq!(
        fs::read(rig.scratch.join("sessions").join(HOME).join("events.jsonl"))
            .expect("journal after"),
        journal,
        "R1 /open navigates; it never asks or delivers keys to a seat"
    );
}

#[test]
fn foreign_composer_moves_one_app_viewer_and_keeps_return_session() {
    let rig = Rig::new("foreign");
    let scout = rig.foreign();
    let app = rig.app(&rig.socket);
    let viewer = rig.attach(HOME);
    let target_viewer = rig.attach(FOREIGN);
    rig.click(&app, FOREIGN, false);
    rig.wait_screen(&app, "GUARD foreign input target drawn", |s| {
        s.contains(&format!("to {FOREIGN}"))
    });
    rig.compose_open(&app, "scout");
    rig.opened(&viewer, FOREIGN, &scout);
    rig.opened(&target_viewer, FOREIGN, &scout); // D3: target-session viewers follow.
    rig.notice(&app, &format!("opened scout in {FOREIGN}"));
    rig.notice(&app, "prefix L goes back");
    let name = rig
        .tmux(&["list-clients", "-F", "#{client_pid}|#{client_name}"])
        .lines()
        .find_map(|row| {
            row.strip_prefix(&format!("{}|", viewer.id()))
                .map(str::to_owned)
        })
        .expect("exact viewer name");
    rig.tmux(&["switch-client", "-c", &name, "-l"]);
    rig.opened(&viewer, HOME, &app);
}

#[test]
fn agents_next_previous_and_mouse_focus_open_the_drawn_seat() {
    let rig = Rig::new("agents");
    let app = rig.app(&rig.socket);
    let viewer = rig.attach(HOME);
    rig.agents(&app);
    rig.focused(&app, "lead");
    rig.raw(&app, "n");
    rig.focused(&app, "colead");
    rig.raw(&app, "n");
    rig.focused(&app, "scout");
    rig.raw(&app, "p");
    rig.focused(&app, "colead");
    rig.click(&app, "scout", true);
    rig.focused(&app, "scout");
    rig.raw(&app, "o");
    rig.opened(&viewer, HOME, &rig.scout);
}

#[test]
fn overview_need_for_spawned_seat_has_the_same_open_action() {
    let rig = Rig::new("need");
    let event = format!(
        "{{\"ts\":\"{}\",\"actor\":\"scout\",\"actor_slot\":\"spawned.0\",\"actor_session\":\"{HOME}\",\"action\":\"state\",\"ref\":\"blocked\",\"summary\":\"SCOUT-DEPENDENCY-BLOCK\"}}\n",
        ae::time::Timestamp::now()
    );
    fs::write(
        rig.scratch.join("sessions").join(HOME).join("events.jsonl"),
        event,
    )
    .expect("blocked spawned seat needs a human");
    let app = rig.app(&rig.socket);
    let viewer = rig.attach(HOME);
    rig.wait_screen(&app, "GUARD Overview waiting-on-you entry exists", |s| {
        s.contains("Waiting on you") && s.contains("scout")
    });
    rig.click(&app, "scout", true);
    rig.focused(&app, "scout");
    rig.raw(&app, "o");
    rig.opened(&viewer, HOME, &rig.scout);
}

#[test]
fn no_app_viewer_refuses_instead_of_moving_a_bystander() {
    let rig = Rig::new("noviewer");
    rig.foreign();
    let app = rig.app(&rig.socket);
    let bystander = rig.attach(FOREIGN);
    let before = rig.viewed(&bystander);
    rig.compose_open(&app, "scout");
    rig.notice(&app, "no tmux client is showing this app");
    rig.unchanged(&[(&bystander, before)]);
    assert_eq!(
        rig.tmux(&["display-message", "-p", "-t", HOME, "#{pane_id}"])
            .trim(),
        app
    );
}

#[test]
fn terminal_without_tmux_identity_refuses_by_name_and_moves_nothing() {
    let rig = Rig::new("outside");
    let app = rig.app_with_caller(&rig.socket, false);
    let viewer = rig.attach(HOME);
    let before = rig.viewed(&viewer);
    rig.compose_open(&app, "scout");
    rig.notice(
        &app,
        "refused: /open scout: ae app is not running inside tmux",
    );
    rig.unchanged(&[(&viewer, before)]);
}

#[test]
fn home_open_allows_multiple_viewers_even_when_their_activity_ties() {
    let rig = Rig::new("here");
    let app = rig.app(&rig.socket);
    let (first, second) = rig.tied();
    rig.compose_open(&app, "scout");
    rig.opened(&first, HOME, &rig.scout);
    rig.opened(&second, HOME, &rig.scout);
}

#[test]
fn foreign_open_refuses_tied_app_viewers_without_moving_any_client() {
    let rig = Rig::new("tie");
    rig.foreign();
    let app = rig.app(&rig.socket);
    let (first, second) = rig.tied();
    let first_before = rig.viewed(&first);
    let second_before = rig.viewed(&second);
    rig.click(&app, FOREIGN, false);
    rig.wait_screen(&app, "GUARD foreign composer selected", |s| {
        s.contains(&format!("to {FOREIGN}"))
    });
    rig.compose_open(&app, "scout");
    rig.notice(
        &app,
        "two clients showing this app were active in the same second",
    );
    rig.unchanged(&[(&first, first_before), (&second, second_before)]);
}

#[test]
fn foreign_open_moves_only_the_newest_app_viewer() {
    let rig = Rig::new("newest");
    let scout = rig.foreign();
    let app = rig.app(&rig.socket);
    let older = rig.attach(HOME);
    let epoch = rig.activity(&older);
    Rig::wait("GUARD another activity second", || {
        u64::try_from(ae::time::Timestamp::now().epoch()).expect("current epoch") > epoch
    });
    let newest = rig.attach(HOME);
    assert!(
        rig.activity(&newest) > epoch,
        "GUARD unique actual client activity"
    );
    let before = rig.viewed(&older);
    rig.click(&app, FOREIGN, false);
    rig.wait_screen(&app, "GUARD foreign composer selected", |s| {
        s.contains(&format!("to {FOREIGN}"))
    });
    rig.compose_open(&app, "scout");
    rig.opened(&newest, FOREIGN, &scout);
    rig.unchanged(&[(&older, before)]);
}

#[test]
fn frozen_frame_identity_refuses_same_name_in_a_new_slot_and_changed_uuid() {
    for change in ["slot", "uuid"] {
        let rig = Rig::new(change);
        let app = rig.app(&rig.socket);
        let viewer = rig.attach(HOME);
        rig.agents(&app);
        rig.click(&app, "scout", true);
        rig.focused(&app, "scout");
        let before = rig.viewed(&viewer);
        let held = Hold::key(&rig, &app);
        if change == "slot" {
            rig.meta(HOME, UUID, &rig.socket, "spawned.1");
            rig.stamp(&rig.scout, "spawned.1", "scout");
        } else {
            rig.meta(HOME, OTHER_UUID, &rig.socket, "spawned.0");
            rig.tmux(&["set-option", "-t", HOME, "@ae_session_uuid", OTHER_UUID]);
        }
        drop(held);
        rig.notice(
            &app,
            if change == "slot" {
                "the roster no longer seats it in that slot"
            } else {
                "this session was replaced"
            },
        );
        rig.unchanged(&[(&viewer, before)]);
    }
}

#[test]
fn open_refuses_a_restamped_seat_pane() {
    let rig = Rig::new("restamped");
    let app = rig.app(&rig.socket);
    let viewer = rig.attach(HOME);
    let before = rig.viewed(&viewer);
    let held = {
        rig.composer(&app);
        rig.raw(&app, "/open scout");
        rig.wait_screen(&app, "GUARD command ready", |s| s.contains("/open scout"));
        let path = rig.gates.join("@app-keys");
        fs::write(&path, "hold").expect("held Enter read");
        rig.key(&app, "Enter");
        Rig::wait("GUARD Enter held", || {
            fs::read_to_string(rig.gates.join("trace"))
                .unwrap_or_default()
                .contains("held @app-keys")
        });
        Hold(path)
    };
    rig.stamp(&rig.scout, "spawned.0", "replacement");
    drop(held);
    rig.notice(&app, "its pane carries another agent");
    rig.unchanged(&[(&viewer, before)]);
}

#[test]
fn launch_server_redirect_cannot_reclassify_the_actual_app_server() {
    let mut rig = Rig::new("redirect");
    let other = rig.scratch.join("redirect.sock");
    rig.scratch.add_tmux_server(other.clone());
    rig.at(
        &other,
        &[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "redirect",
            "sleep",
            "600",
        ],
    );
    let app = rig.app(&other);
    let viewer = rig.attach(HOME);
    rig.compose_open(&app, "scout");
    rig.opened(&viewer, HOME, &rig.scout);
}

#[test]
fn other_server_same_named_session_refuses_with_the_working_command() {
    let mut rig = Rig::new("otherserver");
    let other = rig.scratch.join("other.sock");
    rig.scratch.add_tmux_server(other.clone());
    let local_scout = rig.foreign();
    rig.at(
        &other,
        &[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            FOREIGN,
            "sleep",
            "600",
        ],
    );
    rig.meta(FOREIGN, OTHER_UUID, &other, "spawned.0");
    let app = rig.app(&rig.socket);
    let viewer = rig.attach(HOME);
    let before = rig.viewed(&viewer);
    rig.click(&app, FOREIGN, false);
    rig.wait_screen(&app, "GUARD foreign input target drawn", |s| {
        s.contains(&format!("to {FOREIGN}"))
    });
    rig.compose_open(&app, "scout");
    rig.notice(&app, "not on the tmux server this app runs on");
    rig.notice(&app, &format!("ae {FOREIGN}"));
    rig.unchanged(&[(&viewer, before)]);
    assert_ne!(
        rig.viewed(&viewer).1,
        local_scout,
        "same name and pane ids never substitute a server"
    );
}

#[test]
fn stopped_selection_names_resume_command_and_does_not_resume() {
    let rig = Rig::new("stopped");
    rig.meta("openstopped", OTHER_UUID, &rig.socket, "spawned.0");
    let app = rig.app(&rig.socket);
    let viewer = rig.attach(HOME);
    let before = rig.viewed(&viewer);
    rig.click(&app, "openstopped", false);
    rig.wait_screen(&app, "GUARD stopped row is read-only", |s| {
        s.contains("openstopped is stopped")
    });
    rig.raw(&app, "o");
    rig.notice(&app, "refused:");
    rig.notice(&app, "ae openstopped resumes it");
    rig.unchanged(&[(&viewer, before)]);
    let (running, _) = super::phase2::run_tmux(
        &[
            "-S".to_owned(),
            rig.socket.display().to_string(),
            "has-session".to_owned(),
            "-t".to_owned(),
            "=openstopped".to_owned(),
        ],
        rig.scratch.path(),
    );
    assert!(!running, "R3 stopped sessions are never resumed by open");
}
