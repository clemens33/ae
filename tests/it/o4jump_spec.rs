//! Frozen O4 oracle: brief + lead-ruling-r1 items 1-5; private real clients.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private tmux fixtures"
)]
use super::cli::{OwnedChild, OwnedScratch};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};
const UUID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
struct Rig {
    root: OwnedScratch,
    socket: PathBuf,
    origin: String,
    lead: String,
    chat: String,
}
impl Rig {
    fn new(tag: &str) -> Self {
        let mut root = OwnedScratch::root("o4", tag);
        let socket = root.join("s");
        root.add_tmux_server(socket.clone());
        let mut r = Self {
            root,
            socket,
            origin: String::new(),
            lead: String::new(),
            chat: String::new(),
        };
        r.tm("-f /dev/null new-session -d -s origin sleep 600");
        r.origin = r.tm("new-window -d -P -F #{pane_id} -t origin -n previous sleep 600");
        r.tm("split-window -d -t origin:previous sleep 600");
        r.chat = r.tm("new-session -d -P -F #{pane_id} -s orchestrator sleep 600");
        r.lead = r.tm("new-window -d -P -F #{pane_id} -t orchestrator -n lead sleep 600");
        r.tm(&format!(
            "set-option -t orchestrator @ae_session_uuid {UUID}"
        ));
        r.tm(&format!(
            "set-option -t orchestrator @ae_main_pane {}",
            r.lead
        ));
        let dir = r.root.join("sessions/orchestrator");
        fs::create_dir_all(&dir).expect("meta dir");
        fs::write(dir.join("meta"), format!("session=orchestrator\nsession_id={UUID}\nmeta_agent=true\nmode=local\ntmux_server_kind=socket\ntmux_server={}\n", r.socket.display())).expect("bound orchestrator");
        r.hook("orchestrator", &r.chat);
        let home = r.tm("display-message -p -t origin:0.0 #{pane_id}");
        r.hook("origin", &home);
        r
    }
    fn raw(&self, tail: &[&str]) -> String {
        let mut args = vec!["-S".into(), self.socket.display().to_string()];
        args.extend(tail.iter().map(|s| (*s).to_owned()));
        let (ok, out) = super::phase2::run_tmux(&args, self.root.path());
        assert!(ok, "tmux {tail:?}: {out}");
        out.trim().to_owned()
    }
    fn tm(&self, line: &str) -> String {
        self.raw(&line.split_whitespace().collect::<Vec<_>>())
    }
    fn hook(&self, session: &str, pane: &str) {
        self.raw(&[
            "set-hook",
            "-t",
            session,
            "client-session-changed",
            &format!("select-window -t {pane} ; select-pane -t {pane}"),
        ]);
    }
    fn attach(&self, session: &str) -> (OwnedChild, String) {
        let c = super::cli::tmux_attached_client(&self.socket, session).expect("client");
        let prefix = format!("{}|", c.id());
        let mut name = None;
        Self::wait(|| {
            name = self
                .tm("list-clients -F #{client_pid}|#{client_name}")
                .lines()
                .find_map(|s| s.strip_prefix(&prefix).map(str::to_owned));
            name.is_some()
        });
        (c, name.expect("listed client"))
    }
    fn wait(mut f: impl FnMut() -> bool) {
        let until = Instant::now() + Duration::from_secs(10);
        while !f() {
            assert!(Instant::now() < until, "client state did not settle");
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    fn view(&self, name: &str) -> String {
        let prefix = format!("{name}|");
        self.tm("list-clients -F #{client_name}|#{session_id}|#{window_id}|#{pane_id}")
            .lines()
            .find_map(|s| s.strip_prefix(&prefix).map(str::to_owned))
            .expect("client exists")
    }
    fn jump(&self, c: &OwnedChild, name: &str, forged: Option<&str>) -> std::process::Output {
        let from = self.view(name);
        let server = self.tm("display-message -p #{pid}|#{start_time}");
        let mut args = vec!["_orchestrator-jump", "--client", name, "--client-pid"];
        let pid = c.id().to_string();
        args.push(forged.unwrap_or(&pid));
        let ids: Vec<_> = server.split('|').collect();
        args.extend(["--server-pid", ids[0], "--server-start", ids[1], "--from"]);
        args.extend(from.split('|'));
        super::cli::ae()
            .args(args)
            .env("AE_HOME", self.root.path())
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &self.socket)
            .output()
            .expect("core")
    }
    fn go(&self, c: &OwnedChild, name: &str) {
        let out = self.jump(c, name, None);
        assert!(
            out.status.success(),
            "jump failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    fn record(&self, c: &OwnedChild) -> String {
        let args = [
            "show-options".to_owned(),
            "-v".into(),
            "-t".into(),
            "orchestrator".into(),
            format!("@ae_return_{}", c.id()),
        ];
        let mut words = vec!["-S".into(), self.socket.display().to_string()];
        words.extend(args);
        super::phase2::run_tmux(&words, self.root.path())
            .1
            .trim()
            .to_owned()
    }
    fn said(&self, name: &str, words: &str) {
        let log = self.raw(&["show-messages", "-t", name]);
        assert!(
            log.lines()
                .any(|line| line.contains("command: display-message -c ")
                    && line.contains(name)
                    && line.contains(words)),
            "client diagnostic missing: {log}"
        );
    }
}

#[test]
fn each_client_returns_to_its_own_exact_view_despite_switch_in_hooks_and_rename() {
    let r = Rig::new("round");
    let (a, an) = r.attach("origin");
    r.tm(&format!("select-window -t {}", r.origin));
    let av = r.view(&an);
    r.tm("new-session -d -s third sleep 600");
    let (b, bn) = r.attach("third");
    let bv = r.view(&bn);
    r.raw(&[
        "set-option",
        "-t",
        "orchestrator",
        "@ae_return_999999",
        "1|dead-client|999999|$0|@0|%0|0000000000000000",
    ]);
    r.go(&a, &an);
    assert!(
        !r.tm("show-options -t orchestrator")
            .contains("@ae_return_999999")
    );
    assert!(r.view(&an).ends_with(&format!("|{}", r.lead)));
    assert_eq!(r.view(&bn), bv, "another client never changes session");
    let ar = r.record(&a);
    assert!(ar.starts_with(&format!("1|{an}|{}|{av}|", a.id())));
    r.go(&b, &bn);
    assert_ne!(r.record(&b), ar, "per-client storage");
    r.tm("rename-session -t origin renamed");
    r.go(&a, &an);
    assert_eq!(
        r.view(&an),
        av,
        "same session id survives rename; hook cannot steal pane"
    );
    assert!(r.record(&a).is_empty());
    assert!(!r.record(&b).is_empty());
    r.go(&b, &bn);
    assert_eq!(r.view(&bn), bv);
    assert!(r.record(&b).is_empty());
    assert!(
        r.tm("show-hooks -t renamed client-session-changed")
            .contains("client-session-changed")
    );
}

#[test]
fn moved_or_dead_panes_fall_back_only_to_the_original_session_id() {
    for (tag, change) in [
        ("window", "join-pane -s"),
        ("foreign", "break-pane -s"),
        ("gone", "kill-pane -t"),
    ] {
        let r = Rig::new(tag);
        let (a, name) = r.attach("origin");
        r.tm(&format!("select-window -t {}", r.origin));
        let before = r.view(&name);
        r.go(&a, &name);
        if tag == "window" {
            r.tm(&format!("{change} {} -t origin:0", r.origin));
        } else if tag == "foreign" {
            r.tm("new-session -d -s elsewhere sleep 600");
            r.tm(&format!("join-pane -s {} -t elsewhere:0", r.origin));
        } else {
            r.tm(&format!("{change} {}", r.origin));
        }
        r.go(&a, &name);
        assert_eq!(r.view(&name).split('|').next(), before.split('|').next());
        r.said(&name, "the pane you left is gone");
        assert!(r.record(&a).is_empty());
    }
}

#[test]
fn destroyed_session_and_damaged_or_foreign_records_refuse_without_movement() {
    let r = Rig::new("refuse");
    let (a, name) = r.attach("origin");
    r.go(&a, &name);
    let stayed = r.view(&name);
    let saved = r.record(&a);
    r.tm("kill-session -t origin");
    r.tm("new-session -d -s origin sleep 600");
    let _ = r.jump(&a, &name, None);
    assert_eq!(r.view(&name), stayed);
    assert_eq!(
        r.record(&a),
        saved,
        "same-name successor cannot consume old return"
    );
    for bad in [
        "broken".to_owned(),
        saved.replacen(&format!("|{}|", a.id()), "|999999|", 1),
    ] {
        r.raw(&[
            "set-option",
            "-t",
            "orchestrator",
            &format!("@ae_return_{}", a.id()),
            &bad,
        ]);
        let _ = r.jump(&a, &name, None);
        assert_eq!(r.view(&name), stayed);
        if bad == "broken" {
            r.said(&name, "return record unreadable");
        }
    }
}

#[test]
fn missing_orchestrator_and_replaced_attachment_never_move_or_start_a_session() {
    let r = Rig::new("absent");
    let (a, name) = r.attach("origin");
    let before = r.view(&name);
    let (b, bn) = r.attach("orchestrator");
    let bv = r.view(&bn);
    let _ = r.jump(&b, &bn, None);
    assert_eq!(r.view(&bn), bv);
    r.said(&bn, "nothing to return to");
    drop(b);
    let sessions = r.tm("list-sessions -F #{session_id}");
    let _ = r.jump(&a, &name, Some("999999"));
    assert_eq!(r.view(&name), before);
    assert!(
        r.record(&a).is_empty(),
        "forged attachment writes no record"
    );
    assert_eq!(r.tm("list-sessions -F #{session_id}"), sessions);
    r.tm("kill-session -t orchestrator");
    let _ = r.jump(&a, &name, None);
    assert_eq!(r.view(&name), before);
    assert!(
        !r.tm("list-sessions -F #{session_name}")
            .contains("orchestrator")
    );
    r.said(&name, "no orchestrator running");
}

#[test]
fn a_newer_return_record_written_by_the_switch_hook_survives_old_cleanup() {
    let r = Rig::new("nonce");
    let (a, name) = r.attach("origin");
    r.go(&a, &name);
    let old = r.record(&a);
    let mut fields: Vec<_> = old.split('|').collect();
    fields[6] = if fields[6] == "0000000000000000" {
        "1111111111111111"
    } else {
        "0000000000000000"
    };
    let newer = fields.join("|");
    r.raw(&[
        "set-hook",
        "-a",
        "-t",
        "origin",
        "client-session-changed",
        &format!("set-option -t orchestrator @ae_return_{} '{newer}'", a.id()),
    ]);
    r.go(&a, &name);
    assert_eq!(
        r.record(&a),
        newer,
        "old readback cannot erase a newer nonce"
    );
}

#[test]
fn a_pane_moved_by_the_return_hook_cannot_claim_exact_landing_or_consume_the_record() {
    let r = Rig::new("late");
    let (a, name) = r.attach("origin");
    r.tm(&format!("select-window -t {}", r.origin));
    let before = r.view(&name);
    r.go(&a, &name);
    let saved = r.record(&a);
    r.raw(&[
        "set-hook",
        "-a",
        "-t",
        "origin",
        "client-session-changed",
        &format!("join-pane -s {} -t orchestrator:0", r.origin),
    ]);
    let _ = r.jump(&a, &name, None);
    assert_eq!(r.view(&name).split('|').next(), before.split('|').next());
    assert_eq!(
        r.record(&a),
        saved,
        "scheduled select is not proven landing"
    );
}
