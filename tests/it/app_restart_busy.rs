//! An app started by hand, in an unstamped pane or outside tmux, against an
//! installed sandbox: what holds its exec back, and what stops it looping
//! into a core the link does not name (lead 15:28Z + 15:52Z; docs/upgrade.md).

#![allow(
    clippy::disallowed_methods,
    clippy::unwrap_used,
    reason = "acceptance owns private terminal fixtures and reads their effects"
)]

use crate::app_mode::writing;
use std::fs;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const UUID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
const WAIT: Duration = Duration::from_secs(20);
/// Past the 5 s quiet window plus one 5 s fleet read.
const HOLD: Duration = Duration::from_secs(12);

/// A published version directory whose core is the real build, or a script.
fn version(dir: &Path, script: Option<&str>) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let core = dir.join("ae-core");
    match script {
        Some(body) => fs::write(&core, format!("#!/bin/sh\n{body}\n")).unwrap(),
        None => drop(fs::copy(env!("CARGO_BIN_EXE_ae"), &core).unwrap()),
    }
    fs::set_permissions(&core, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(dir.join("install"), "not executed").unwrap();
    let sum = "0".repeat(64);
    fs::write(
        dir.join("SHA256SUMS"),
        format!("{sum}  ae-core\n{sum}  install\n"),
    )
    .unwrap();
    core
}

struct Rig {
    tool: super::deliver::Rig,
    scratch: PathBuf,
    ae_home: PathBuf,
    socket: PathBuf,
    link: PathBuf,
    pane: String,
}

impl Rig {
    /// `fleet`: the session is in the fleet and the app, started for it, runs
    /// in an unstamped pane; otherwise the fleet is empty, the app bare and
    /// outside tmux — the nav's probe shape.
    fn new(tag: &str, fleet: bool) -> Self {
        let tool = super::deliver::Rig::new(tag, "codex", 0);
        let scratch = tool.dir.parent().unwrap().parent().unwrap().to_owned();
        let home = scratch.join("home");
        let session = tool.dir.file_name().unwrap().to_string_lossy().into_owned();
        let mut rig = Self {
            ae_home: home.join(".ae"),
            socket: scratch.join("sock"),
            link: home.join(".local/bin/ae"),
            scratch,
            tool,
            pane: String::new(),
        };
        fs::create_dir_all(&rig.ae_home).unwrap();
        fs::write(rig.ae_home.join("config"), "[workspace]\nchat = app\n").unwrap();
        if fleet {
            let state = rig.ae_home.join("sessions").join(&session);
            fs::create_dir_all(&state).unwrap();
            fs::write(state.join("meta"), format!(
                "session={session}\nmode=local\nsession_id={UUID}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=codex\nlaunch_id.main=tok-rig\nseat.worker.0=colead\nagent_bin.worker.0=codex\nlaunch_id.worker.0=tok-colead\n",
                rig.socket.display()
            )).unwrap();
            fs::write(state.join(".launch-attempt"), "1").unwrap();
        }
        for (key, value) in [
            ("@ae_look", "off"),
            ("@ae_session_uuid", UUID),
            ("@ae_motion", "off"),
            ("@ae_main_pane", rig.tool.pane.as_str()),
            ("default-size", "160x45"),
        ] {
            rig.tmux(&["set-option", "-t", &session, key, value]);
        }
        fs::create_dir_all(rig.link.parent().unwrap()).unwrap();
        let own = version(&rig.ae_home.join("versions").join(ae::VERSION), None);
        symlink(own, &rig.link).unwrap();
        let uid = fs::metadata(&rig.scratch).unwrap().uid();
        let named = rig.scratch.join(format!("tmux-{uid}"));
        fs::create_dir_all(&named).unwrap();
        fs::set_permissions(&named, fs::Permissions::from_mode(0o700)).unwrap();
        symlink(&rig.socket, named.join("ae")).unwrap();
        let env = |key: &str, path: &Path| format!("{key}={}", path.display());
        let (h, a, c, t) = (
            env("HOME", &home),
            env("AE_HOME", &rig.ae_home),
            env("CONFIG_FILE", &rig.ae_home.join("config")),
            env("TMUX_TMPDIR", &rig.scratch),
        );
        let (link, dir) = (
            rig.link.display().to_string(),
            rig.scratch.display().to_string(),
        );
        let mut argv = vec!["new-window", "-d", "-t", &session, "-c", &dir, "-P", "-F"];
        argv.extend([
            "#{pane_id}",
            "env",
            "-u",
            "AE_NO_AUTOSTART",
            "-u",
            "AE_SESSION",
        ]);
        if !fleet {
            argv.extend(["-u", "TMUX", "-u", "TMUX_PANE"]);
        }
        argv.extend([h.as_str(), &a, &c, &t, &link, "app"]);
        if fleet {
            argv.push(&session);
        }
        let pane = rig.tmux(&argv).trim().to_owned();
        rig.pane = pane;
        rig.tmux(&["set-window-option", "-t", &rig.pane, "remain-on-exit", "on"]);
        rig
    }

    fn tmux(&self, tail: &[&str]) -> String {
        let mut args = vec!["-S".to_owned(), self.socket.display().to_string()];
        args.extend(tail.iter().map(|s| (*s).to_owned()));
        let (ok, out) = super::phase2::run_tmux(&args, &self.scratch);
        assert!(ok, "tmux {tail:?}: {out}");
        out
    }

    fn wait(&self, why: &str, met: impl Fn(&str) -> bool) -> String {
        let until = Instant::now() + WAIT;
        loop {
            let screen = self.tmux(&["capture-pane", "-p", "-t", &self.pane]);
            if met(&screen) {
                return screen;
            }
            assert!(Instant::now() < until, "{why}; screen:\n{screen}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn keys(&self, keys: &[&str]) {
        self.tmux(&[&["send-keys", "-t", &self.pane][..], keys].concat());
    }

    /// Repoint the command link at version `to`, whose core is `script`.
    fn next(&self, to: &str, script: &str) {
        let core = version(&self.ae_home.join("versions").join(to), Some(script));
        fs::remove_file(&self.link).unwrap();
        symlink(core, &self.link).unwrap();
    }
}

#[test]
fn a_hand_started_app_never_execs_over_a_kept_draft_or_an_open_composer() {
    let rig = Rig::new("rxbusy", true);
    let session = rig.tool.dir.file_name().unwrap().to_string_lossy();
    rig.wait("hand-started app paints", |s| {
        s.contains("Agents ")
            && s.lines().skip(2).any(|row| {
                row.chars()
                    .skip(4)
                    .collect::<String>()
                    .starts_with(session.as_ref())
                    && row.contains('│')
            })
    });
    rig.keys(&["i"]);
    rig.wait("composer opens", writing);
    rig.keys(&["-l", "--", "draft content"]);
    rig.wait("draft typed", |s| s.contains("draft content"));
    rig.keys(&["Escape"]);
    rig.wait("composer closed", |s| !writing(s));
    let entry = rig.scratch.join("entry");
    let newer = crate::calver::newer();
    rig.next(
        &newer,
        &format!("printf '%s\\n' \"$@\" > '{}'", entry.display()),
    );
    let busy = format!("ae {newer} installed - restarts when idle");
    rig.wait("busy hint over the kept draft", |s| s.contains(&busy));
    std::thread::sleep(HOLD);
    assert!(!entry.exists(), "a draft kept after Escape holds the exec");

    rig.keys(&["i"]);
    rig.wait("composer reopens on the kept draft", |s| {
        writing(s) && s.contains("draft content")
    });
    rig.keys(&["-N", "13", "BSpace"]);
    rig.wait("composer open and empty", |s| {
        writing(s) && !s.contains("draft content")
    });
    std::thread::sleep(HOLD);
    assert!(!entry.exists(), "an open empty composer holds the exec");
    rig.wait("still the busy hint", |s| s.contains(&busy));
    rig.keys(&["Escape"]);
    let until = Instant::now() + WAIT;
    while !entry.exists() {
        assert!(
            Instant::now() < until,
            "a closed empty composer lets it exec"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn an_empty_fleet_app_hops_once_into_a_core_the_link_does_not_name() {
    for (tag, to) in [
        ("rxloop", crate::calver::newer()),
        ("rxback", crate::calver::older()),
    ] {
        let rig = Rig::new(tag, false);
        rig.wait("empty-fleet app paints", |s| s.contains("No sessions."));
        let hops = rig.scratch.join("hops");
        let own = rig
            .ae_home
            .join("versions")
            .join(ae::VERSION)
            .join("ae-core");
        let (hops_at, own_at) = (hops.display(), own.display());
        rig.next(
            &to,
            &format!("printf 'hop\\n' >> '{hops_at}'; exec '{own_at}' \"$@\""),
        );
        let until = Instant::now() + WAIT;
        while !hops.exists() {
            assert!(
                Instant::now() < until,
                "{to}: the first exec takes the bridge"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        std::thread::sleep(HOLD);
        let hopped = fs::read_to_string(&hops).unwrap();
        assert_eq!(hopped, "hop\n", "{to}: exactly one hop over two refreshes");
        let hint = format!("ae {to} installed - restart ae app to use it");
        rig.wait("the landed core only hints", |s| s.contains(&hint));
    }
}
