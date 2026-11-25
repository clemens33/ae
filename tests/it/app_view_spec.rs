//! Frozen P1: ruling R1-R4/A1-A3 + lead plan ruling 1-8.
//! Single-binary restart WAIVER: bridge logs, then rewrites ONLY version.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private installed/tmux fixtures and observes their effects"
)]

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ae::app::handover::{self, Handover, Verdict, View};

const ID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
const WEB_ID: &str = "6f1c2b7e-0d3a-4c58-9a2e-5b1d7c9e0a11";
const NOW: i64 = 1_760_259_000;
const LITERAL: &str = "v1|%7|2026.10.99|1760259000|web|6f1c2b7e-0d3a-4c58-9a2e-5b1d7c9e0a11|agents|colead|31|9|2|0|1|4";
const WAIT: Duration = Duration::from_secs(20);

fn view() -> View {
    View {
        session: "web".into(),
        id: WEB_ID.into(),
        focus: Some("colead".into()),
        sidebar: Some(31),
        list: Some(9),
        pages: 2,
        rows: 0,
        list_top: Some(1),
        body_top: 4,
        panel: None,
        zoom: false,
        orchestrator: None,
    }
}

fn envelope() -> Handover {
    Handover {
        pane: "%7".into(),
        version: "2026.10.99".into(),
        at: NOW,
        view: view(),
    }
}

#[test]
fn grammar_accepts_exact_literal_and_empty_optional_fields() {
    assert_eq!(
        handover::parse(LITERAL),
        Some(envelope()),
        "literal field contract"
    );
    let empty = format!("v1|%0|2026.10.56|1|web|{WEB_ID}|overview||||0|0||0");
    let mut want = envelope();
    (want.pane, want.version, want.at) = ("%0".into(), "2026.10.56".into(), 1);
    want.view = View {
        focus: None,
        sidebar: None,
        list: None,
        pages: 0,
        rows: 0,
        list_top: None,
        body_top: 0,
        ..view()
    };
    assert_eq!(handover::parse(&empty), Some(want));
}

#[test]
fn grammar_rejects_hostile_noncanonical_or_out_of_bounds_fields() {
    for (old, bad) in [
        ("v1", "v2"),
        ("%7", "7"),
        ("%7", "%00"),
        ("%7", "%12345678901"),
        ("2026.10.99", "a"),
        ("1760259000", "0"),
        ("1760259000", "100000000000"),
        ("|web|", "|../web|"),
        (WEB_ID, "not-a-uuid"),
        (WEB_ID, "6F1C2B7E-0D3A-4C58-9A2E-5B1D7C9E0A11"),
        ("|agents|", "|Agents|"),
        ("|colead|", "|bad/seat|"),
        ("|31|", "|65536|"),
        ("|9|", "|65536|"),
        ("|2|", "|1000001|"),
        ("|0|", "|65536|"),
        ("|1|", "|65536|"),
        ("|4", "|1000001"),
        ("|31|", "|031|"),
        ("|31|", "|+31|"),
        ("|31|", "|-1|"),
        ("|31|", "| 31|"),
        ("|web|", "|wéb|"),
    ] {
        let text = LITERAL.replacen(old, bad, 1);
        assert!(handover::parse(&text).is_none(), "refuse {text:?}");
    }
    for text in [
        format!("{LITERAL}|extra"),
        format!("{LITERAL}\n"),
        LITERAL
            .rsplit_once('|')
            .expect("literal final field")
            .0
            .to_owned(),
        "x".repeat(513),
    ] {
        assert!(handover::parse(&text).is_none(), "field count/total cap");
    }
}

#[test]
fn render_clamps_integer_fields_to_the_approved_wire_caps() {
    let mut saved = envelope();
    saved.view.pages = usize::MAX;
    saved.view.rows = usize::MAX;
    saved.view.list_top = Some(usize::MAX);
    saved.view.body_top = usize::MAX;
    assert_eq!(
        saved.render(),
        format!(
            "v1|%7|2026.10.99|1760259000|web|{WEB_ID}|agents|colead|31|9|1000000|65535|65535|1000000"
        )
    );
    assert_eq!(envelope().render(), LITERAL);
    assert_eq!(
        (handover::OPTION, handover::CAP, handover::MAX_AGE_SECS),
        ("@ae_app_view", 512, 60)
    );
}

#[test]
fn admission_binds_exact_pane_version_and_inclusive_recorded_age() {
    let saved = envelope();
    for (pane, version, now, expected) in [
        ("%7", "2026.10.99", NOW, true),
        ("%7", "2026.10.99", NOW + 60, true),
        ("%7", "2026.10.99", NOW + 61, false),
        ("%7", "2026.10.99", NOW - 1, false),
        ("%8", "2026.10.99", NOW, false),
        ("%7", "2026.10.98", NOW, false),
    ] {
        assert_eq!(
            saved.admits(pane, version, now),
            expected,
            "{pane}/{version}/{now}"
        );
    }
    for (now, expected) in [
        (NOW, true),
        (NOW + 60, true),
        (NOW + 61, false),
        (NOW - 1, false),
    ] {
        assert_eq!(handover::fresh(NOW, now), expected);
    }
    assert!(!handover::fresh(1, i64::MAX));
    assert!(!handover::fresh(i64::MIN, i64::MAX));
}

#[test]
fn pending_decision_uses_identity_completeness_and_the_original_write_epoch() {
    for (row, id, scanned, now, want) in [
        (true, Some(WEB_ID), false, NOW + 60, Verdict::Apply),
        (true, Some(ID), false, NOW, Verdict::Discard),
        (false, None, false, NOW, Verdict::Defer),
        (true, None, false, NOW, Verdict::Defer),
        (true, Some(""), false, NOW, Verdict::Defer),
        (false, None, true, NOW, Verdict::Discard),
        (true, None, true, NOW, Verdict::Discard),
        (true, Some(""), true, NOW, Verdict::Discard),
        (false, None, false, NOW + 61, Verdict::Discard),
        (true, Some(WEB_ID), true, NOW + 61, Verdict::Discard),
        (true, Some(WEB_ID), true, NOW - 1, Verdict::Discard),
    ] {
        assert_eq!(
            handover::decide(NOW, now, &view(), row, id, scanned),
            want,
            "row={row}, id={id:?}, scanned={scanned}, now={now}"
        );
    }
}

struct Rig {
    tool: super::deliver::Rig,
    root: PathBuf,
    home: PathBuf,
    state: PathBuf,
    socket: PathBuf,
    name: String,
    core: PathBuf,
    link: PathBuf,
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

impl Rig {
    fn new(tag: &str) -> Self {
        let tool = super::deliver::Rig::new(tag, "codex", 0);
        let root = tool.dir.ancestors().nth(2).expect("root").to_owned();
        let home = root.join("home");
        let state = home.join(".ae");
        let socket = root.join("sock");
        let name = tool
            .dir
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        let version = state.join("versions").join(ae::VERSION);
        fs::create_dir_all(&version).expect("installed version");
        let core = version.join("ae-core");
        fs::copy(env!("CARGO_BIN_EXE_ae"), &core).expect("private core");
        fs::write(version.join("install"), "not executed").expect("manifest member");
        fs::write(
            version.join("SHA256SUMS"),
            format!("{}  ae-core\n{}  install\n", "0".repeat(64), "0".repeat(64)),
        )
        .expect("manifest");
        let config = format!("[workspace]\nchat = app\nfleet_order = {name}, web\n");
        fs::write(state.join("config"), config).expect("config");
        let link = home.join(".local/bin/ae");
        fs::create_dir_all(link.parent().expect("bin")).expect("command parent");
        symlink(&core, &link).expect("private command link");
        let rig = Self {
            tool,
            root,
            home,
            state,
            socket,
            name,
            core,
            link,
        };
        let uid = fs::metadata(&rig.root).expect("owned fixture root").uid();
        let named = rig.root.join(format!("tmux-{uid}"));
        fs::create_dir(&named).expect("private named-server directory");
        fs::set_permissions(&named, fs::Permissions::from_mode(0o700))
            .expect("private permissions");
        symlink(&rig.socket, named.join("ae")).expect("private default-server alias");
        rig.meta(&rig.name, ID);
        rig.meta("web", WEB_ID);
        rig.tmux(&["new-session", "-d", "-s", "web", "sleep", "120"]);
        let lead = rig
            .tmux(&["display-message", "-p", "-t", "web", "#{pane_id}"])
            .trim()
            .to_owned();
        let peer = rig.window("web");
        for (pane, slot, seat) in [(&lead, "main", "lead"), (&peer, "worker.0", "colead")] {
            rig.tmux(&["set-option", "-p", "-t", pane, "@ae_slot", slot]);
            rig.tmux(&["set-option", "-p", "-t", pane, "@ae_agent", seat]);
        }
        rig.tmux(&["set-option", "-t", "web", "@ae_session_uuid", WEB_ID]);
        rig.tmux(&["set-option", "-t", "web", "@ae_icons", "off"]);
        rig.tmux(&["set-option", "-t", "web", "@ae_attn_rank", "0"]);
        rig.tmux(&[
            "set-option",
            "-t",
            "web",
            "@ae_agents",
            &format!(
                "v1;{};3600;lead:idle:working:{lead};colead:idle:working:{peer}",
                ae::time::Timestamp::now().epoch()
            ),
        ]);
        for (key, value) in [
            ("@ae_session_uuid", ID),
            ("@ae_look", "off"),
            ("@ae_icons", "off"),
            ("@ae_motion", "off"),
            ("@ae_main_pane", rig.tool.pane.as_str()),
            ("@ae_attn_rank", "0"),
        ] {
            rig.tmux(&["set-option", "-t", &rig.name, key, value]);
        }
        for (key, value) in [("HOME", &rig.home), ("TMUX_TMPDIR", &rig.root)] {
            rig.tmux(&[
                "set-environment",
                "-t",
                &rig.name,
                key,
                &value.display().to_string(),
            ]);
        }
        rig
    }

    fn meta(&self, name: &str, id: &str) {
        let dir = self.state.join("sessions").join(name);
        fs::create_dir_all(&dir).expect("session record");
        fs::write(dir.join("meta"), format!("schema=2\nsession={name}\nmode=local\nsession_id={id}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=codex\nlaunch_id.main=tok-lead\nseat.worker.0=colead\nagent_bin.worker.0=codex\nlaunch_id.worker.0=tok-peer\n", self.socket.display())).expect("meta");
        fs::write(dir.join("events.jsonl"), "").expect("journal");
        fs::write(
            dir.join(".launch-attempt"),
            ae::time::Timestamp::now().epoch().to_string(),
        )
        .expect("attempt");
    }

    fn tmux(&self, tail: &[&str]) -> String {
        let mut args = vec!["-S".into(), self.socket.display().to_string()];
        args.extend(tail.iter().map(|s| (*s).to_owned()));
        let (ok, out) = super::phase2::run_tmux(&args, &self.root);
        assert!(ok, "GUARD private tmux {tail:?}: {out}");
        out
    }

    fn window(&self, name: &str) -> String {
        self.tmux(&[
            "new-window",
            "-d",
            "-t",
            name,
            "-P",
            "-F",
            "#{pane_id}",
            "sleep",
            "120",
        ])
        .trim()
        .to_owned()
    }

    fn sleeper(&self, stamped: bool) -> String {
        let pane = self.window(&self.name);
        self.tmux(&["resize-window", "-t", &pane, "-x", "160", "-y", "45"]);
        if stamped {
            self.tmux(&["set-option", "-p", "-t", &pane, "@ae_console", ID]);
        }
        pane
    }

    fn start(&self, pane: &str) {
        self.tmux(&[
            "respawn-pane",
            "-k",
            "-t",
            pane,
            "env",
            "-u",
            "AE_HOME",
            "-u",
            "CONFIG_FILE",
            "-u",
            "AE_NO_AUTOSTART",
            &format!("HOME={}", self.home.display()),
            &format!("TMUX_TMPDIR={}", self.root.display()),
            &self.link.display().to_string(),
            "app",
            &self.name,
        ]);
    }

    fn screen(&self, pane: &str) -> String {
        self.tmux(&["capture-pane", "-p", "-t", pane])
    }

    fn wait(&self, pane: &str, why: &str, met: impl Fn(&str) -> bool) -> String {
        let until = Instant::now() + WAIT;
        loop {
            let text = self.screen(pane);
            if met(&text) {
                return text;
            }
            assert!(Instant::now() < until, "{why}:\n{text}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn raw(&self) -> String {
        self.tmux(&["display-message", "-p", "-t", &self.name, "#{@ae_app_view}"])
            .trim_end_matches('\n')
            .to_owned()
    }

    fn put(&self, text: &str) {
        self.tmux(&["set-option", "-t", &self.name, "@ae_app_view", text]);
    }

    fn literal(pane: &str) -> String {
        format!(
            "v1|{pane}|{}|{}|web|{WEB_ID}|agents|colead|31|9|0|0||0",
            ae::VERSION,
            ae::time::Timestamp::now().epoch()
        )
    }

    fn bridge(&self) {
        let next = self.state.join("versions").join(crate::calver::newer());
        fs::create_dir_all(&next).expect("successor fixture");
        let path = |name: &str| quote(&self.root.join(name).display().to_string());
        let tmux = format!("tmux -S {}", quote(&self.socket.display().to_string()));
        let target = quote(&self.name);
        let script = format!(
            "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$*\" > {}\nraw=$({tmux} show-option -v -t {target} @ae_app_view 2>/dev/null || true)\nprintf '%s\\n' \"$raw\" > {}\nnext=$(printf '%s' \"$raw\" | awk -F '|' -v version={} 'BEGIN{{OFS=\"|\"}} {{$3=version; print}}')\nprintf '%s\\n' \"$next\" > {}\n{tmux} set-option -t {target} @ae_app_view \"$next\"\nrm {}\nln -s {} {}\nexec {} \"$@\"\n",
            path("argv"),
            path("raw"),
            quote(ae::VERSION),
            path("rewritten"),
            quote(&self.link.display().to_string()),
            quote(&self.core.display().to_string()),
            quote(&self.link.display().to_string()),
            quote(&self.core.display().to_string()),
        );
        fs::write(next.join("ae-core"), script).expect("test-only bridge");
        fs::set_permissions(next.join("ae-core"), fs::Permissions::from_mode(0o755))
            .expect("bridge executable");
        fs::remove_file(&self.link).expect("remove owned link");
        symlink(next.join("ae-core"), &self.link).expect("repoint owned link");
    }
}

fn selected(screen: &str, name: &str) -> bool {
    screen.lines().next().is_some_and(|row| {
        row.split('│')
            .next_back()
            .unwrap_or_default()
            .trim_start()
            .starts_with(name)
    })
}

/// Whether the sidebar row (the cell left of the rule) naming `seat` carries
/// the seat cursor: `-` in the first cell (this rig draws plain ASCII), never
/// the idle mark two cells in.
fn focused(screen: &str, seat: &str) -> bool {
    screen.lines().any(|row| {
        row.split_once('│').is_some_and(|(left, _)| {
            left.starts_with('-') && left.split_whitespace().any(|word| word == seat)
        })
    })
}

/// Whether the sidebar draws a row naming `seat`.
fn seat_drawn(screen: &str, seat: &str) -> bool {
    screen.lines().any(|row| {
        row.split_once('│')
            .is_some_and(|(left, _)| left.split_whitespace().any(|word| word == seat))
    })
}

/// Whether the session list draws a row for `name`: a mark, then the name
/// (a lifted row has `>` before the mark). No row number is drawn.
fn card_drawn(screen: &str, name: &str) -> bool {
    screen.lines().any(|row| {
        let left = row.split('│').next().unwrap_or_default();
        let mut words = left.trim_start().trim_start_matches('>').split_whitespace();
        words.next().is_some() && words.next() == Some(name)
    })
}

fn fleet_drawn(screen: &str, home: &str) -> bool {
    card_drawn(screen, home) && card_drawn(screen, "web")
}

#[test]
fn installed_restart_carries_nonhome_agents_and_focus_then_consumes_the_option() {
    let rig = Rig::new("viewrestart");
    let pane = rig.sleeper(true);
    rig.start(&pane);
    rig.wait(&pane, "GUARD home loaded", |s| {
        selected(s, &rig.name) && s.contains("Agents")
    });
    rig.tmux(&["send-keys", "-t", &pane, "2"]);
    rig.wait(&pane, "GUARD web selected", |s| selected(s, "web"));
    rig.tmux(&["send-keys", "-t", &pane, "Tab"]);
    rig.wait(&pane, "GUARD Agents drawn", |s| {
        seat_drawn(s, "lead") && !focused(s, "lead")
    });
    // No seat carries the cursor until a step key: the first `n` lands on the
    // first seat, the second on colead.
    rig.tmux(&["send-keys", "-t", &pane, "n"]);
    rig.wait(&pane, "GUARD lead focused", |s| focused(s, "lead"));
    rig.tmux(&["send-keys", "-t", &pane, "n"]);
    rig.wait(&pane, "GUARD colead focused", |s| focused(s, "colead"));
    rig.bridge();
    rig.wait(
        &pane,
        "restart restores selection, tab and focused seat",
        |s| {
            rig.root.join("rewritten").exists()
                && selected(s, "web")
                && focused(s, "colead")
                && rig.raw().is_empty()
        },
    );
    assert_eq!(
        fs::read_to_string(rig.root.join("argv"))
            .expect("bridge argv")
            .trim(),
        format!("app {}", rig.name)
    );
    let raw = fs::read_to_string(rig.root.join("raw")).expect("raw writer carrier");
    let fields: Vec<_> = raw.trim_end().split('|').collect();
    let newer = crate::calver::newer();
    assert_eq!(
        &fields[..8],
        &[
            "v1",
            pane.as_str(),
            newer.as_str(),
            fields[3],
            "web",
            WEB_ID,
            "agents",
            "colead"
        ],
        "real writer fields"
    );
    let rewritten = fs::read_to_string(rig.root.join("rewritten")).expect("logged rewrite");
    let mut changed = fields.clone();
    changed[2] = ae::VERSION;
    assert_eq!(
        rewritten.trim_end(),
        changed.join("|"),
        "WAIVER rewrites only version"
    );
}

#[test]
fn valid_own_chat_reader_restores_the_literal_view_and_unsets_the_option() {
    let rig = Rig::new("viewvalid");
    let pane = rig.sleeper(true);
    rig.put(&Rig::literal(&pane));
    rig.start(&pane);
    rig.wait(&pane, "valid reader restores Agents/focus", |s| {
        selected(s, "web") && focused(s, "colead")
    });
    assert!(rig.raw().is_empty(), "valid option consumed");
}

fn refused(tag: &str, change: impl Fn(String) -> String) {
    let rig = Rig::new(tag);
    let pane = rig.sleeper(true);
    rig.put(&change(Rig::literal(&pane)));
    rig.start(&pane);
    let shown = rig.wait(&pane, "invalid option consumed and fleet drawn", |s| {
        fleet_drawn(s, &rig.name) && rig.raw().is_empty()
    });
    assert!(
        selected(&shown, &rig.name) && !selected(&shown, "web") && !focused(&shown, "colead"),
        "invalid view keeps default Overview"
    );
    assert!(rig.raw().is_empty(), "invalid option consumed once");
}

#[test]
fn wrong_pane_is_default_and_consumed() {
    refused("viewpane", |s| {
        s.replacen(&s.split('|').nth(1).expect("pane").to_owned(), "%999999", 1)
    });
}
#[test]
fn wrong_version_is_default_and_consumed() {
    refused("viewversion", |s| {
        s.replacen(ae::VERSION, &crate::calver::older(), 1)
    });
}
#[test]
fn stale_is_default_and_consumed() {
    refused("viewstale", |s| {
        s.replacen(
            s.split('|').nth(3).expect("epoch"),
            &(ae::time::Timestamp::now().epoch() - 61).to_string(),
            1,
        )
    });
}
#[test]
fn future_is_default_and_consumed() {
    refused("viewfuture", |s| {
        s.replacen(
            s.split('|').nth(3).expect("epoch"),
            &(ae::time::Timestamp::now().epoch() + 60).to_string(),
            1,
        )
    });
}
#[test]
fn garbage_is_default_and_consumed() {
    refused("viewgarbage", |_| "garbage".into());
}
#[test]
fn oversize_is_default_and_consumed() {
    refused("viewoversize", |_| "x".repeat(513));
}
#[test]
fn same_name_other_uuid_is_default_and_consumed() {
    refused("viewuuid", |s| s.replacen(WEB_ID, ID, 1));
}

#[test]
fn standalone_app_does_not_consume_another_chats_handover() {
    let rig = Rig::new("viewstandalone");
    let pane = rig.sleeper(false);
    let text = Rig::literal(&pane);
    rig.put(&text);
    rig.start(&pane);
    let shown = rig.wait(&pane, "GUARD standalone fleet drawn", |s| {
        fleet_drawn(s, &rig.name)
    });
    assert!(selected(&shown, &rig.name) && !selected(&shown, "web") && !focused(&shown, "colead"));
    assert_eq!(rig.raw(), text, "unstamped app never consumes chat state");
}
// Durable app view acceptance. Input and screen assertions use real app processes
// in an owned tmux fixture. Disk tests take valid bytes from the real writer.
impl Rig {
    fn persisted(&self) -> PathBuf {
        self.state.join("app-view")
    }

    fn durable_start(&self, pane: &str, args: &[&str]) {
        let home = format!("HOME={}", self.home.display());
        let root = format!("TMUX_TMPDIR={}", self.root.display());
        let socket = format!("AE_TMUX_SERVER={}", self.socket.display());
        let executable = self.link.display().to_string();
        let mut command = vec![
            "respawn-pane",
            "-k",
            "-t",
            pane,
            "env",
            "-u",
            "AE_HOME",
            "-u",
            "CONFIG_FILE",
            "-u",
            "TMUX",
            "-u",
            "TMUX_PANE",
            &home,
            &root,
            "AE_TMUX_SERVER_KIND=socket",
            &socket,
            &executable,
        ];
        command.extend_from_slice(args);
        self.tmux(&command);
    }

    fn durable_pane(&self, stamped: bool) -> String {
        let pane = self.sleeper(stamped);
        self.tmux(&["set-window-option", "-t", &pane, "remain-on-exit", "on"]);
        self.tmux(&["resize-window", "-t", &pane, "-x", "200", "-y", "45"]);
        pane
    }

    fn durable_keys(&self, pane: &str, keys: &[&str]) {
        let mut command = vec!["send-keys", "-t", pane];
        command.extend_from_slice(keys);
        self.tmux(&command);
    }

    fn durable_quit(&self, pane: &str, before: &[&str]) -> String {
        let mut keys = before.to_vec();
        keys.extend(["q", "q"]);
        self.durable_keys(pane, &keys);
        let until = Instant::now() + WAIT;
        loop {
            let status = self.tmux(&[
                "display-message",
                "-p",
                "-t",
                pane,
                "#{pane_dead}:#{pane_dead_status}",
            ]);
            if let Some(status) = status
                .trim()
                .strip_prefix("1:")
                .filter(|status| !status.is_empty())
            {
                assert_eq!(status, "0", "normal app quit:\n{}", self.screen(pane));
                return self.screen(pane);
            }
            assert!(
                Instant::now() < until,
                "app did not quit: {}",
                self.screen(pane)
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn durable_select_web(&self) -> String {
        let pane = self.durable_pane(false);
        self.durable_start(&pane, &["app", &self.name]);
        self.wait(&pane, "GUARD home and web drawn", |screen| {
            selected(screen, &self.name) && fleet_drawn(screen, &self.name)
        });
        self.durable_keys(&pane, &["2"]);
        self.wait(&pane, "GUARD web selected", |screen| {
            selected(screen, "web")
        });
        pane
    }

    fn durable_saved_web(&self) -> Vec<u8> {
        let pane = self.durable_select_web();
        self.durable_quit(&pane, &[]);
        fs::read(self.persisted()).expect("quit publishes the selected web view")
    }

    fn durable_role(&self, id: &str) {
        let pane = self
            .tmux(&[
                "new-session",
                "-d",
                "-P",
                "-F",
                "#{pane_id}",
                "-s",
                "orchestrator",
                "-e",
                "AE_SESSION=orchestrator",
                "sleep",
                "120",
            ])
            .trim()
            .to_owned();
        let dir = self.state.join("sessions/orchestrator");
        fs::create_dir(&dir).expect("fixture orchestrator");
        // A published core derives its state root from its canonical executable
        // path, so a real launch records the overlay under that spelling. The
        // scratch base is `/tmp`, a link to `/private/tmp` on macOS: the seat
        // proof is a textual match, and an uncanonical overlay is no seat there.
        let overlay = fs::canonicalize(&self.state)
            .expect("canonical state root")
            .join("orchestrator.config");
        fs::write(&overlay, "[workspace]\norchestrator = true\n").expect("role overlay");
        fs::write(
            dir.join("meta"),
            format!("schema=2\nsession=orchestrator\nmode=local\nsession_id={id}\nlayout=vertical\nseat.main=orchestrator\nagent_bin.main=codex\nmain_pane={pane}\nmeta_agent=true\nlocal_config={}\ntmux_server_kind=socket\ntmux_server={}\n", overlay.display(), self.socket.display()),
        ).expect("orchestrator meta");
        self.tmux(&["set-option", "-t", "=orchestrator:", "@ae_session_uuid", id]);
        self.tmux(&["set-option", "-t", "=orchestrator:", "@ae_icons", "off"]);
        self.tmux(&["set-option", "-t", "=orchestrator:", "@ae_main_pane", &pane]);
        self.tmux(&["set-option", "-p", "-t", &pane, "@ae_slot", "main"]);
        self.tmux(&["set-option", "-p", "-t", &pane, "@ae_agent", "orchestrator"]);
        ae::watchdog_glue::touch_beat(&dir).expect("role watchdog beat");
        let board = ae::desk::Desk {
            updated: ae::time::Timestamp::now(),
            items: ae::desk::parse_input(
                b"DECIDE\nD1 | Durable decision\nRUNNING\nR1 | Durable running item\n",
            )
            .expect("fixture board"),
        };
        fs::write(dir.join(ae::desk::RECORD_FILE), ae::desk::encode(&board))
            .expect("fixture board record");
        #[allow(clippy::format_collect, reason = "a fixture: one formatted line per turn")]
        let history: String = (0..80)
            .map(|n| format!("{{\"ts\":\"2026-10-03T06:00:00Z\",\"actor\":\"orchestrator\",\"action\":\"chat\",\"summary\":\"DURABLE-TURN-{n:03}\"}}\n"))
            .collect();
        fs::write(dir.join("events.jsonl"), history).expect("fixture orchestrator history");
    }
}

fn durable_sidebar(screen: &str) -> Option<usize> {
    screen.lines().next()?.chars().position(|cell| cell == '│')
}

fn durable_header(screen: &str, word: &str) -> bool {
    screen.lines().next().is_some_and(|row| row.contains(word))
}

#[test]
fn durable_view_writes_while_running_and_a_bare_start_restores_the_quit_view() {
    let rig = Rig::new("diskview");
    let pane = rig.durable_select_web();
    rig.durable_keys(&pane, &["Tab"]);
    rig.wait(&pane, "GUARD web seats drawn", |s| seat_drawn(s, "colead"));
    rig.durable_keys(&pane, &["n", "n"]);
    let shown = rig.wait(&pane, "GUARD colead focused", |s| focused(s, "colead"));
    let width = durable_sidebar(&shown).expect("GUARD main divider");
    let from = width + 1;
    let to = from + 7;
    let drag = format!("\x1b[<0;{from};10M\x1b[<32;{to};10M\x1b[<0;{to};10m");
    rig.durable_keys(&pane, &["-l", "--", &drag]);
    rig.wait(&pane, "GUARD drawer moved the divider", |s| {
        durable_sidebar(s) == Some(width + 7)
    });
    let file = rig.persisted();
    let until = Instant::now() + WAIT;
    let old = loop {
        if let Ok(file) = fs::File::open(&file) {
            break file;
        }
        assert!(Instant::now() < until, "view changes save before quit");
        std::thread::sleep(Duration::from_millis(25));
    };
    let before = old.metadata().expect("open file metadata");
    assert_eq!(before.permissions().mode() & 0o777, 0o600);
    // An already opened file must keep its inode alive across the final write.
    rig.durable_quit(&pane, &["X"]);
    let after = fs::symlink_metadata(&file).expect("quit view");
    assert!(after.is_file());
    assert_eq!(after.permissions().mode() & 0o777, 0o600);
    assert_ne!(
        before.ino(),
        after.ino(),
        "atomic replacement changes the inode"
    );
    // The disk record survives age and upgrades. The 60-second handover
    // follows a different admission rule.
    let saved = fs::read_to_string(&file).expect("canonical durable view");
    let mut fields: Vec<_> = saved.split('|').collect();
    let previous_version = crate::calver::older();
    fields[2] = &previous_version;
    fields[3] = "1";
    fs::write(&file, fields.join("|")).expect("the same view recorded by an older app");
    let fresh = rig.durable_pane(false);
    rig.durable_start(&fresh, &[]);
    rig.wait(
        &fresh,
        "bare ae restores web and its main-column zoom",
        |s| selected(s, "web") && durable_header(s, "X unzoom"),
    );
    rig.durable_keys(&fresh, &["X"]);
    rig.wait(
        &fresh,
        "unzoom reveals the saved divider and seat focus",
        |s| selected(s, "web") && focused(s, "colead") && durable_sidebar(s) == Some(width + 7),
    );
}

#[test]
fn durable_view_never_writes_for_a_loader_only_selection_change() {
    let rig = Rig::new("diskidle");
    fs::write(
        rig.state.join("config"),
        format!("[workspace]\nchat = app\nfleet_order = web, {}\n", rig.name),
    )
    .expect("web is the first fleet row");
    let pane = rig.durable_pane(false);
    rig.durable_start(&pane, &["app"]);
    rig.wait(&pane, "GUARD unhomed app selects web without input", |s| {
        card_drawn(s, &rig.name) && selected(s, "web")
    });
    rig.tmux(&["kill-session", "-t", "=web:"]);
    fs::remove_dir_all(rig.state.join("sessions/web")).expect("selected session removed");
    rig.wait(&pane, "GUARD loader selects the surviving session", |s| {
        selected(s, &rig.name) && card_drawn(s, &rig.name) && !card_drawn(s, "web")
    });
    std::thread::sleep(Duration::from_millis(2200));
    assert!(
        !rig.persisted().exists(),
        "loader changes never write app-view"
    );
    rig.durable_quit(&pane, &[]);
    assert!(
        !rig.persisted().exists(),
        "untouched quit leaves the file absent"
    );
}

#[test]
fn durable_view_defers_to_an_admitted_handover_and_covers_a_refused_one() {
    let rig = Rig::new("diskorder");
    let first = rig.durable_select_web();
    rig.durable_quit(&first, &["X"]);
    let saved = fs::read(rig.persisted()).expect("quit publishes the zoomed view");
    for admitted in [true, false] {
        fs::write(rig.persisted(), &saved).expect("restore the original durable web view");
        let pane = rig.durable_pane(true);
        let carrier = Rig::literal(&pane)
            .replace("|web|", &format!("|{}|", rig.name))
            .replace(WEB_ID, ID);
        let carrier = if admitted {
            carrier
        } else {
            carrier.replacen(ae::VERSION, &crate::calver::older(), 1)
        };
        rig.put(&carrier);
        rig.start(&pane);
        rig.wait(
            &pane,
            "admitted handover takes precedence, refused handover falls back",
            |s| {
                rig.raw().is_empty()
                    && selected(s, &rig.name)
                    && if admitted {
                        durable_sidebar(s) == Some(31)
                    } else {
                        durable_header(s, "X unzoom")
                    }
            },
        );
        rig.durable_quit(&pane, &[]);
        assert_eq!(fs::read(rig.persisted()).expect("durable view"), saved);
    }
}

#[test]
fn durable_view_refuses_hostile_nodes_and_bytes_then_starts() {
    let rig = Rig::new("diskhostile");
    let saved = rig.durable_saved_web();
    let file = rig.persisted();
    fs::remove_file(&file).expect("remove owned view");
    let victim = rig.root.join("valid-view-target");
    fs::write(&victim, &saved).expect("a symlink target with valid opposed view bytes");
    symlink(&victim, &file).expect("fixture symlink");
    assert_eq!(
        fs::read(&file).expect("symlink control has valid bytes"),
        saved
    );
    let pane = rig.durable_pane(false);
    rig.durable_start(&pane, &["app"]);
    rig.wait(
        &pane,
        "symlink leaves the fleet at its default selection",
        |s| fleet_drawn(s, &rig.name) && selected(s, &rig.name),
    );
    let closed = rig.durable_quit(&pane, &["X"]);
    assert!(
        closed.contains("ae app closed"),
        "failed save must permit quit: {closed}"
    );
    assert_eq!(
        closed.matches("app-view").count(),
        1,
        "failed final save is reported once: {closed}"
    );
    assert_eq!(
        fs::read_link(&file).expect("symlink refused on write"),
        victim
    );
    assert_eq!(fs::read(&victim).expect("symlink target untouched"), saved);
    fs::remove_file(&file).expect("remove fixture symlink");

    fs::create_dir(&file).expect("fixture directory at file path");
    let pane = rig.durable_pane(false);
    rig.durable_start(&pane, &["app"]);
    rig.wait(&pane, "non-regular path permits start", |s| {
        fleet_drawn(s, &rig.name)
    });
    rig.durable_quit(&pane, &["X"]);
    assert!(file.is_dir(), "non-regular path refused on write");
    fs::remove_dir(&file).expect("remove fixture directory");

    let mut oversized = saved.clone();
    oversized.resize(1024 * 1024, b'x');
    let mut damaged = saved.clone();
    damaged.push(0xff);
    let version = saved
        .iter()
        .position(|&byte| byte == b'|')
        .expect("wire tag");
    let foreign = [b"v999999".as_slice(), &saved[version..]].concat();
    for bytes in [oversized, damaged, foreign] {
        fs::write(&file, bytes).expect("hostile view record");
        let pane = rig.durable_pane(false);
        rig.durable_start(&pane, &["app"]);
        rig.wait(&pane, "hostile view bytes permit a default start", |s| {
            fleet_drawn(s, &rig.name) && selected(s, &rig.name)
        });
        rig.durable_quit(&pane, &[]);
    }

    fs::write(&file, saved).expect("valid view before web disappears");
    rig.tmux(&["kill-session", "-t", "=web:"]);
    fs::remove_dir_all(rig.state.join("sessions/web")).expect("remove saved session record");
    let pane = rig.durable_pane(false);
    rig.durable_start(&pane, &["app"]);
    rig.wait(
        &pane,
        "a missing saved home leaves the app on the fleet",
        |s| card_drawn(s, &rig.name) && selected(s, &rig.name) && !card_drawn(s, "web"),
    );
}

#[test]
fn durable_view_keeps_a_full_panel_its_scroll_and_a_closed_panel_without_an_ordinary_session() {
    let rig = Rig::new("diskorch");
    rig.durable_role(WEB_ID);
    for session in [&rig.name, "web"] {
        fs::remove_dir_all(rig.state.join("sessions").join(session))
            .expect("ordinary record removed");
    }
    let first = rig.durable_pane(false);
    rig.durable_start(&first, &["app", "orchestrator"]);
    rig.wait(
        &first,
        "GUARD proven homed role draws its full panel and no board",
        |s| {
            durable_header(s, "F side")
                && s.contains("DURABLE-TURN-079")
                && !s.contains("Durable decision")
                && !durable_header(s, "b hide")
        },
    );
    // A scroll alone never writes the view: a change before it does.
    rig.durable_keys(&first, &["F"]);
    rig.wait(&first, "GUARD the panel turned side", |s| {
        durable_header(s, "F full")
    });
    rig.durable_keys(&first, &["F"]);
    rig.wait(&first, "GUARD back to the full panel", |s| {
        durable_header(s, "F side") && s.contains("DURABLE-TURN-079")
    });
    rig.durable_keys(&first, &["PageUp"]);
    rig.wait(
        &first,
        "GUARD panel scrolled away from its latest turn",
        |s| s.contains("newer turns") && !s.contains("DURABLE-TURN-079"),
    );
    rig.durable_quit(&first, &[]);
    let saved =
        fs::read(rig.persisted()).expect("persist with the orchestrator as the only record");

    let fresh = rig.durable_pane(false);
    rig.durable_start(&fresh, &["app"]);
    rig.wait(
        &fresh,
        "fresh app reopens the proved full panel and its scroll",
        |s| {
            durable_header(s, "F side")
                && s.contains("newer turns")
                && !s.contains("Durable decision")
        },
    );
    rig.durable_quit(&fresh, &["O"]);

    rig.meta(&rig.name, ID);
    let closed = rig.durable_pane(false);
    rig.durable_start(&closed, &["app", "orchestrator"]);
    rig.wait(
        &closed,
        "a saved closed panel overrides the homed default",
        |s| card_drawn(s, &rig.name) && durable_header(s, "X zoom") && !durable_header(s, "F side"),
    );
    rig.durable_quit(&closed, &[]);

    rig.tmux(&["kill-session", "-t", "=orchestrator:"]);
    fs::remove_dir_all(rig.state.join("sessions/orchestrator")).expect("remove old role record");
    fs::write(rig.persisted(), &saved).expect("restore the old open full record");
    let absent = rig.durable_pane(false);
    rig.durable_start(&absent, &["app"]);
    rig.wait(
        &absent,
        "an absent role keeps the default panel closed",
        |s| card_drawn(s, &rig.name) && durable_header(s, "X zoom") && !durable_header(s, "F side"),
    );
    rig.durable_quit(&absent, &[]);

    rig.durable_role(OTHER_ORCH_ID);
    let changed = rig.durable_pane(false);
    rig.durable_start(&changed, &["app"]);
    rig.wait(
        &changed,
        "a proved replacement keeps the full view and drops old scroll",
        |s| {
            durable_header(s, "F side")
                && s.contains("DURABLE-TURN-079")
                && !s.contains("newer turns")
        },
    );
}

const OTHER_ORCH_ID: &str = "0199c0de-bbbb-4890-abcd-ef0123456789";

#[test]
fn durable_view_keeps_side_panel_and_distinguishes_which_chat_has_focus() {
    let rig = Rig::new("diskside");
    rig.durable_role(OTHER_ORCH_ID);
    let first = rig.durable_pane(false);
    rig.durable_start(&first, &["app", &rig.name]);
    rig.wait(&first, "GUARD named app draws its main chat", |s| {
        fleet_drawn(s, &rig.name) && selected(s, &rig.name)
    });
    rig.durable_keys(&first, &["O"]);
    rig.wait(&first, "GUARD side panel is live", |s| {
        durable_header(s, "pane  F full") && s.contains("DURABLE-TURN-079")
    });
    rig.durable_quit(&first, &[]);
    assert!(rig.persisted().is_file(), "quit saves the open side panel");

    let side = rig.durable_pane(false);
    rig.durable_start(&side, &["app", &rig.name]);
    rig.wait(&side, "the side panel reopens beside the main chat", |s| {
        durable_header(s, "pane  F full") && s.contains("DURABLE-TURN-079")
    });
    rig.durable_keys(&side, &["i"]);
    rig.wait(&side, "the restored side panel has the keys", |s| {
        s.contains("to orchestrator") && s.contains("Esc browses")
    });
    rig.durable_keys(&side, &["Escape"]);
    let focus_main = "\x1b[<0;60;5M\x1b[<0;60;5m";
    rig.durable_keys(&side, &["-l", "--", focus_main]);
    rig.durable_keys(&side, &["i"]);
    rig.wait(
        &side,
        "GUARD side panel visible with main chat focused",
        |s| {
            durable_header(s, "pane  F full")
                && s.contains(&format!("to {}", rig.name))
                && s.contains("Esc browses")
        },
    );
    rig.durable_keys(&side, &["Escape"]);
    rig.wait(&side, "the main chat browses again", |s| {
        s.contains("O Enter writes")
    });
    // A key within ESC_IDLE of the Escape is read as Alt+key, and one right
    // behind the browse frame starts the composer again.
    std::thread::sleep(ae::console::input::ESC_IDLE + Duration::from_millis(200));
    rig.durable_quit(&side, &[]);

    let side_main = rig.durable_pane(false);
    rig.durable_start(&side_main, &["app", &rig.name]);
    rig.wait(&side_main, "the side panel reopens again", |s| {
        durable_header(s, "pane  F full") && s.contains("DURABLE-TURN-079")
    });
    rig.durable_keys(&side_main, &["i"]);
    rig.wait(&side_main, "the restored main chat has the keys", |s| {
        s.contains(&format!("to {}", rig.name)) && s.contains("Esc browses")
    });
}
