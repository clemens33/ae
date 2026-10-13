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
        let next = self.state.join("versions/2026.10.99");
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

fn focused(screen: &str, seat: &str) -> bool {
    screen.lines().any(|row| {
        let left = row.split('│').next().unwrap_or_default();
        left.trim_start().starts_with('>') && left.contains(seat)
    })
}

fn fleet_drawn(screen: &str, home: &str) -> bool {
    screen.contains(&format!("1 {home}")) && screen.contains("2 web")
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
    rig.wait(&pane, "GUARD Agents drawn", |s| focused(s, "lead"));
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
    assert_eq!(
        &fields[..8],
        &[
            "v1",
            pane.as_str(),
            "2026.10.99",
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
    refused("viewversion", |s| s.replacen(ae::VERSION, "2026.10.1", 1));
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
