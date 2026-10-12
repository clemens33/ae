//! Frozen appupgrade acceptance spec.
//! Oracles: brief-appupgrade.md U1-U6, lead rulings (Q-A any-different, Q-B hyphen, Q-C chat=app, quiet window 5s).
//! Cap: 600 insertions post-rustfmt.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::struct_excessive_bools,
    clippy::unchecked_time_subtraction,
    clippy::unwrap_used,
    reason = "acceptance owns private terminal fixtures and reads their effects"
)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ae::app::upgrade::{self, Blocker, Upgrade};
use ae::autoupgrade;
use ae::console::toggle::{self, Restart};
use ae::inventory::ServerId;
use ae::shape::Shape;
use ae::tmux::WindowPane;

const UUID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
const WAIT: Duration = Duration::from_secs(20);

fn pane(id: &str, console: Option<&str>, agent: Option<&str>) -> WindowPane {
    WindowPane {
        pane_id: id.to_owned(),
        window_id: "@1".to_owned(),
        theme: String::new(),
        reader_src: None,
        console: console.map(ToOwned::to_owned),
        dead: false,
        window_index: 0,
        pane_index: 0,
        agent: agent.map(ToOwned::to_owned),
    }
}

fn check_seen(link: Result<&str, &str>, auto: bool) -> Option<Upgrade> {
    let inst = Shape::Installed {
        home: "/Users/test/.ae".into(),
        version_dir: "/Users/test/.ae/versions/2026.10.50".into(),
        version: "2026.10.50".into(),
    };
    upgrade::seen(
        &inst,
        |_| link.map(str::to_owned).map_err(str::to_owned),
        || auto,
        || {
            Ok(Restart {
                server: ServerId::Ambient,
                pane: "%0".into(),
                command: vec!["ae".into()],
            })
        },
    )
}

// Pure Seam Tests (U1, U2, U3, U5, Q-A, Q-B)

#[test]
fn seen_detects_newer_and_older_link_versions_and_ignores_equal_or_unparseable() {
    let ok = Ok(Restart {
        server: ServerId::Ambient,
        pane: "%0".into(),
        command: vec!["ae".into()],
    });
    // 1. Newer version & 2. Q-A older version both planned
    assert_eq!(
        check_seen(Ok("2026.10.51"), true),
        Some(Upgrade {
            version: "2026.10.51".into(),
            restart: ok.clone()
        })
    );
    assert_eq!(
        check_seen(Ok("2026.10.49"), true),
        Some(Upgrade {
            version: "2026.10.49".into(),
            restart: ok
        })
    );
    // 3. Equal version, 4. unparseable, 5. read error -> None
    assert_eq!(check_seen(Ok("2026.10.50"), true), None);
    assert_eq!(check_seen(Ok("invalid"), true), None);
    assert_eq!(check_seen(Err("fail"), true), None);
    // 6. Automatic disabled -> Upgrade with Err restart
    assert_eq!(
        check_seen(Ok("2026.10.51"), false),
        Some(Upgrade {
            version: "2026.10.51".into(),
            restart: Err("auto_upgrade is off".into())
        })
    );
    // 7. Checkout & Displaced shapes never check link
    for shape in [
        Shape::Checkout,
        Shape::Displaced {
            home: "/a".into(),
            declared: "/b".into(),
        },
    ] {
        assert_eq!(
            upgrade::seen(&shape, |_| panic!(), || panic!(), || panic!()),
            None
        );
    }
}

#[test]
fn automatic_policy_checks_installed_shape_no_autostart_and_config() {
    let tmp = super::cli::OwnedScratch::root("upg", "pol");
    let home = tmp.path().join(".ae");
    fs::create_dir_all(&home).unwrap();
    let inst = Shape::Installed {
        home: home.clone(),
        version_dir: home.join("versions/1"),
        version: "1".into(),
    };
    assert!(autoupgrade::automatic(&inst, false));
    for (cfg, no_auto, exp) in [
        ("auto_upgrade = on\n", false, true),
        ("auto_upgrade = off\n", false, false),
        ("auto_upgrade = invalid\n", false, false),
        ("auto_upgrade = on\n", true, false),
    ] {
        fs::write(home.join("config"), format!("[workspace]\n{cfg}")).unwrap();
        assert_eq!(autoupgrade::automatic(&inst, no_auto), exp);
    }
    assert!(!autoupgrade::automatic(&Shape::Checkout, false));
    assert!(!autoupgrade::automatic(
        &Shape::Displaced {
            home: "/a".into(),
            declared: "/b".into()
        },
        false
    ));
}

#[test]
fn own_chat_proof_requires_matching_uuid_and_no_agent() {
    let valid = vec![pane("%0", Some(UUID), None)];
    assert_eq!(toggle::own_chat(&valid, "%0", UUID, UUID), Ok(()));
    for (panes, target, uuid, bound) in [
        (vec![pane("%0", Some(UUID), Some("lead"))], "%0", UUID, UUID),
        (vec![pane("%0", Some("foreign"), None)], "%0", UUID, UUID),
        (valid.clone(), "%0", UUID, "other"),
        (valid.clone(), "%0", UUID, ""),
        (valid.clone(), "%999", UUID, UUID),
    ] {
        assert!(toggle::own_chat(&panes, target, uuid, bound).is_err());
    }
}

#[test]
fn idle_gate_blocks_on_drafts_in_any_session_queued_deferred_composing_settings_turn_armed_and_recent_input()
 {
    let now = Instant::now();
    let quiet_past = now - Duration::from_secs(6);
    let quiet_recent = now - Duration::from_secs(2);
    let f = [false; 8];
    let check = |f: [bool; 8], i| {
        upgrade::restart_blocker(f[0], f[1], f[2], f[3], f[4], f[5], f[6], f[7], i, now)
    };
    assert_eq!(check(f, None), None);
    for (i, exp) in [
        (0, Blocker::Draft),
        (1, Blocker::Queued),
        (2, Blocker::Deferred),
        (3, Blocker::Composing),
        (4, Blocker::Held),
        (5, Blocker::Settings),
        (6, Blocker::Turn),
        (7, Blocker::Armed),
    ] {
        let mut fi = f;
        fi[i] = true;
        assert_eq!(check(fi, None), Some(exp));
    }
    assert_eq!(check(f, Some(quiet_recent)), Some(Blocker::RecentInput));
    assert_eq!(check(f, Some(quiet_past)), None);
}

#[test]
fn hints_use_hyphen_typography_per_app_style() {
    assert_eq!(
        upgrade::busy_hint("2026.10.51"),
        "ae 2026.10.51 installed - restarts when idle"
    );
    assert_eq!(
        upgrade::hint_only("2026.10.51"),
        "ae 2026.10.51 installed - restart ae app to use it"
    );
}

// Live Rig Acceptance Tests (Private socket tmux + Installed shape)

struct InstalledRig {
    tool: super::deliver::Rig,
    scratch: PathBuf,
    home: PathBuf,
    ae_home: PathBuf,
    link: PathBuf,
    socket: PathBuf,
    session: String,
    restarted_log: PathBuf,
}

impl InstalledRig {
    fn new(tag: &str, auto_upgrade: bool) -> Self {
        let tool = super::deliver::Rig::new(tag, "codex", 0);
        let scratch = tool.dir.parent().unwrap().parent().unwrap().to_owned();
        let home = scratch.join("home");
        let ae_home = home.join(".ae");
        let socket = scratch.join("sock");
        let session = tool.dir.file_name().unwrap().to_string_lossy().into_owned();
        let restarted_log = home.join("restarted.log");

        fs::create_dir_all(&ae_home).unwrap();
        let cfg = if auto_upgrade {
            "[workspace]\nchat = app\n"
        } else {
            "[workspace]\nchat = app\nauto_upgrade = off\n"
        };
        fs::write(ae_home.join("config"), cfg).unwrap();

        let v_curr = ae_home.join("versions").join(ae::VERSION);
        fs::create_dir_all(&v_curr).unwrap();
        let core = v_curr.join("ae-core");
        fs::copy(env!("CARGO_BIN_EXE_ae"), &core).unwrap();
        fs::write(v_curr.join("install"), "not executed").unwrap();
        let s = "0".repeat(64);
        fs::write(
            v_curr.join("SHA256SUMS"),
            format!("{s}  ae-core\n{s}  install\n"),
        )
        .unwrap();

        let link = home.join(".local/bin/ae");
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink(&core, &link).unwrap();

        let s_dir = ae_home.join("sessions").join(&session);
        fs::create_dir_all(&s_dir).unwrap();
        fs::write(
            s_dir.join("meta"),
            format!(
                "session={session}\nmode=local\nsession_id={UUID}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=codex\nlaunch_id.main=tok-rig\nseat.worker.0=colead\nagent_bin.worker.0=codex\nlaunch_id.worker.0=tok-colead\n",
                socket.display()
            ),
        ).unwrap();
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        fs::write(s_dir.join(".launch-attempt"), epoch.to_string()).unwrap();

        let rig = Self {
            tool,
            scratch,
            home,
            ae_home,
            link,
            socket,
            session,
            restarted_log,
        };
        rig.tmux(&["set-option", "-t", &rig.session, "@ae_look", "off"]);
        for (k, v) in [
            ("@ae_session_uuid", UUID),
            ("@ae_motion", "off"),
            ("@ae_main_pane", rig.tool.pane.as_str()),
            ("default-size", "160x45"),
        ] {
            rig.tmux(&["set-option", "-t", &rig.session, k, v]);
        }
        rig
    }

    fn tmux(&self, tail: &[&str]) -> String {
        let mut args = vec!["-S".to_owned(), self.socket.display().to_string()];
        args.extend(tail.iter().map(|s| (*s).to_owned()));
        let (ok, out) = super::phase2::run_tmux(&args, &self.scratch);
        assert!(ok, "tmux {tail:?}: {out}");
        out
    }

    fn repoint_link(&self, new_version: &str) {
        let next_dir = self.ae_home.join("versions").join(new_version);
        fs::create_dir_all(&next_dir).unwrap();
        let next_core = next_dir.join("ae-core");
        fs::write(
            &next_core,
            format!(
                "#!/bin/sh\necho \"$@\" > \"{}\"\n",
                self.restarted_log.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&next_core, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(next_dir.join("install"), "not executed").unwrap();
        let s = "0".repeat(64);
        fs::write(
            next_dir.join("SHA256SUMS"),
            format!("{s}  ae-core\n{s}  install\n"),
        )
        .unwrap();
        let _ = fs::remove_file(&self.link);
        std::os::unix::fs::symlink(&next_core, &self.link).unwrap();
    }

    fn open_app(&self) -> String {
        let link_str = self.link.display().to_string();
        let home_env = format!("HOME={}", self.home.display());
        let pane = self
            .tmux(&[
                "new-window",
                "-d",
                "-t",
                &self.session,
                "-P",
                "-F",
                "#{pane_id}",
                "env",
                "-u",
                "AE_NO_AUTOSTART",
                &home_env,
                &link_str,
                "app",
                &self.session,
            ])
            .trim()
            .to_owned();
        self.tmux(&["set-option", "-p", "-t", &pane, "@ae_console", UUID]);
        self.tmux(&["set-window-option", "-t", &pane, "remain-on-exit", "on"]);
        pane
    }

    fn open_checkout_app(&self) -> String {
        let home_env = format!("HOME={}", self.home.display());
        let ae_env = format!("AE_HOME={}", self.ae_home.display());
        let cfg_env = format!("CONFIG_FILE={}", self.ae_home.join("config").display());
        let sock_env = format!("AE_TMUX_SERVER={}", self.socket.display());
        let pane = self
            .tmux(&[
                "new-window",
                "-d",
                "-t",
                &self.session,
                "-P",
                "-F",
                "#{pane_id}",
                "env",
                "-u",
                "AE_NO_AUTOSTART",
                &home_env,
                &ae_env,
                &cfg_env,
                "AE_TMUX_SERVER_KIND=socket",
                &sock_env,
                env!("CARGO_BIN_EXE_ae"),
                "app",
                &self.session,
            ])
            .trim()
            .to_owned();
        self.tmux(&["set-option", "-p", "-t", &pane, "@ae_console", UUID]);
        self.tmux(&["set-window-option", "-t", &pane, "remain-on-exit", "on"]);
        pane
    }

    fn screen(&self, pane: &str) -> String {
        self.tmux(&["capture-pane", "-p", "-t", pane])
    }

    fn wait(&self, pane: &str, within: Duration, why: &str, met: impl Fn(&str) -> bool) -> String {
        let until = Instant::now() + within;
        loop {
            let screen = self.screen(pane);
            if met(&screen) {
                return screen;
            }
            assert!(Instant::now() < until, "{why}; terminal screen:\n{screen}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn wait_restarted(&self, within: Duration, why: &str) -> String {
        let until = Instant::now() + within;
        loop {
            if let Ok(c) = fs::read_to_string(&self.restarted_log)
                && !c.trim().is_empty()
            {
                return c;
            }
            assert!(Instant::now() < until, "{why}; restarted log not written");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

#[test]
fn installed_app_idle_restarts_into_newer_link_core() {
    let rig = InstalledRig::new("upn", true);
    let pane = rig.open_app();
    rig.wait(&pane, WAIT, "first paint", |s| s.contains("▣ 1 "));
    rig.repoint_link("2026.10.99");

    let restarted = rig.wait_restarted(WAIT, "idle app restarts into newer link core");
    assert_eq!(restarted.trim(), format!("app {}", rig.session));

    let listed = rig.tmux(&[
        "list-panes",
        "-s",
        "-t",
        &rig.session,
        "-F",
        "#{pane_id}|#{@ae_console}",
    ]);
    assert!(
        listed.contains(&format!("{pane}|{UUID}")),
        "pane id and console stamp kept after restart"
    );
}

#[test]
fn installed_app_idle_restarts_into_older_link_core() {
    let rig = InstalledRig::new("upo", true);
    // Q-A rollback ruling: older link version also restarts
    let pane = rig.open_app();
    rig.wait(&pane, WAIT, "first paint", |s| s.contains("▣ 1 "));
    rig.repoint_link("2026.10.40");

    let restarted = rig.wait_restarted(WAIT, "idle app restarts into older link core");
    assert_eq!(restarted.trim(), format!("app {}", rig.session));

    let listed = rig.tmux(&[
        "list-panes",
        "-s",
        "-t",
        &rig.session,
        "-F",
        "#{pane_id}|#{@ae_console}",
    ]);
    assert!(listed.contains(&format!("{pane}|{UUID}")));
}

#[test]
fn installed_app_busy_with_draft_never_restarts_and_shows_busy_hint() {
    let rig = InstalledRig::new("upb", true);
    let pane = rig.open_app();
    rig.wait(&pane, WAIT, "first paint", |s| s.contains("▣ 1 "));

    // Enter compose mode (Enter or i per model.rs) then type text into composer
    rig.tmux(&["send-keys", "-t", &pane, "i"]);
    rig.wait(&pane, WAIT, "write mode acquired", |s| {
        crate::app_mode::writing(s)
    });
    rig.tmux(&["send-keys", "-t", &pane, "-l", "--", "draft content"]);
    rig.wait(&pane, WAIT, "draft content entered", |s| {
        s.contains("draft content")
    });

    // Repoint link to newer version
    rig.repoint_link("2026.10.99");

    // Must show busy hint and NOT restart
    rig.wait(
        &pane,
        WAIT,
        "shows busy hint while draft is present",
        |screen| screen.contains("ae 2026.10.99 installed - restarts when idle"),
    );
    // Hold >= REFRESH + 5s (10s) after hint appears before asserting no restart
    std::thread::sleep(Duration::from_secs(10));
    assert!(
        !rig.restarted_log.exists(),
        "busy app must never restart while draft exists"
    );
}

#[test]
fn standalone_app_in_unstamped_pane_hints_only() {
    let rig = InstalledRig::new("ups", true);
    let link_str = rig.link.display().to_string();
    let home_env = format!("HOME={}", rig.home.display());
    let unstamped = rig
        .tmux(&[
            "new-window",
            "-d",
            "-t",
            &rig.session,
            "-P",
            "-F",
            "#{pane_id}",
            "env",
            "-u",
            "AE_NO_AUTOSTART",
            &home_env,
            &link_str,
            "app",
            &rig.session,
        ])
        .trim()
        .to_owned();
    rig.tmux(&[
        "set-window-option",
        "-t",
        &unstamped,
        "remain-on-exit",
        "on",
    ]);
    rig.wait(&unstamped, WAIT, "first paint", |s| s.contains("▣ 1 "));
    rig.repoint_link("2026.10.99");

    rig.wait(
        &unstamped,
        WAIT,
        "standalone app shows hint only",
        |screen| screen.contains("ae 2026.10.99 installed - restart ae app to use it"),
    );
    // Hold >= REFRESH + 5s (10s) after hint appears before asserting no restart
    std::thread::sleep(Duration::from_secs(10));
    assert!(
        !rig.restarted_log.exists(),
        "standalone app must never restart"
    );
}

#[test]
fn checkout_shape_never_restarts_and_shows_no_hint() {
    let rig = InstalledRig::new("upc", true);
    let pane = rig.open_checkout_app();
    rig.wait(&pane, WAIT, "first paint", |s| s.contains("▣ 1 "));
    rig.repoint_link("2026.10.99");

    // Hold >= 2*REFRESH (12s) before asserting checkout never hints or restarts
    std::thread::sleep(Duration::from_secs(12));
    let screen = rig.screen(&pane);
    assert!(
        !screen.contains("installed"),
        "checkout shape never shows upgrade hint"
    );
    assert!(!rig.restarted_log.exists(), "checkout shape never restarts");
}

#[test]
fn auto_upgrade_disabled_hints_only() {
    let rig = InstalledRig::new("upd", false); // auto_upgrade = off
    let pane = rig.open_app();
    rig.wait(&pane, WAIT, "first paint", |s| s.contains("▣ 1 "));
    rig.repoint_link("2026.10.99");

    rig.wait(
        &pane,
        WAIT,
        "shows hint only when auto_upgrade is disabled",
        |screen| screen.contains("ae 2026.10.99 installed - restart ae app to use it"),
    );
    // Hold >= REFRESH + 5s (10s) after hint appears before asserting no restart
    std::thread::sleep(Duration::from_secs(10));
    assert!(
        !rig.restarted_log.exists(),
        "must not restart when auto_upgrade is disabled"
    );
}
