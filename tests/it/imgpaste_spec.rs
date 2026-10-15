//! Frozen imgpaste acceptance (#228): Ctrl+V saves the clipboard image into
//! the written session's `messages/` and inserts its path at the cursor; sent never.
//! Fake clipboard: `AE_TEST_CLIPBOARD_TOOL` names a symlink to the rig script;
//! a mode file picks the outcome, removing the target is the missing tool.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private terminal fixtures and reads their effects"
)]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const UUID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
const WAIT: Duration = Duration::from_secs(20);
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nIMGPASTE-PROBE";

struct Rig {
    tool: super::deliver::Rig,
    root: PathBuf,
    name: String,
}

impl Rig {
    fn new(tag: &str, app: bool) -> Self {
        let tool = super::deliver::Rig::new(tag, "codex", 0);
        let root = tool.dir.ancestors().nth(2).expect("root").to_owned();
        let name = format!("dl{tag}");
        let rig = Self { tool, root, name };
        fs::write(rig.tool.dir.join("meta"), format!(
            "session={}\nmode=local\nsession_id={UUID}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=codex\nlaunch_id.main=tok-rig\nseat.worker.0=colead\nagent_bin.worker.0=codex\nlaunch_id.worker.0=tok-colead\n",
            rig.name, rig.root.join("sock").display()
        )).expect("lead pair meta");
        let chat = if app { "app" } else { "on" };
        let cfg = format!("[workspace]\nchat = {chat}\n");
        fs::write(rig.root.join("config"), cfg).expect("config");
        let stamp = ae::time::Timestamp::now().epoch().to_string();
        fs::write(rig.tool.dir.join(".launch-attempt"), stamp).expect("launch stamp");
        rig.write_clip();
        let link = rig.root.join("clip-link");
        std::os::unix::fs::symlink(rig.root.join("clip"), &link).expect("seam symlink");
        fs::write(rig.root.join("clip-mode"), "empty").expect("clipboard mode");
        let (t, p) = (rig.name.as_str(), rig.tool.pane.as_str());
        for (option, value) in [
            ("@ae_session_uuid", UUID),
            ("@ae_motion", "off"),
            ("@ae_main_pane", p),
            ("default-size", "160x45"),
        ] {
            rig.tmux(&["set-option", "-t", t, option, value]);
        }
        rig.tmux(&["set-option", "-p", "-t", p, "@ae_agent", "lead"]);
        let seam = link.display().to_string();
        rig.tmux(&["set-environment", "-t", t, "AE_TEST_CLIPBOARD_TOOL", &seam]);
        rig
    }
    fn write_clip(&self) {
        let clip = self.root.join("clip");
        fs::write(&clip, format!(
            "#!/bin/sh\ncase \"$(cat '{}')\" in\npng) printf '\\211PNG\\r\\n\\032\\nIMGPASTE-PROBE' ;;\njunk) printf 'junkjunkjunk' ;;\nhuge) printf '\\211PNG\\r\\n\\032\\n'; head -c 20971520 /dev/zero ;;\nhang) sleep 30 ;;\nfail) exit 4 ;;\nesac\n",
            self.root.join("clip-mode").display()
        )).expect("fake clipboard tool");
        fs::set_permissions(&clip, fs::Permissions::from_mode(0o755)).expect("clip executable");
    }
    fn tmux(&self, tail: &[&str]) -> String {
        let mut args = vec![
            "-S".to_owned(),
            self.root.join("sock").display().to_string(),
        ];
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        let (ok, out) = super::phase2::run_tmux(&args, &self.root);
        assert!(ok, "private tmux {tail:?}: {out}");
        out
    }

    fn open(&self) -> String {
        let out = super::cli::ae()
            .env("AE_HOME", &self.root)
            .env("CONFIG_FILE", self.root.join("config"))
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", self.root.join("sock"))
            .env("TMUX", format!("{},1,0", self.root.join("sock").display()))
            .env("TMUX_PANE", &self.tool.pane)
            .arg("_console")
            .output()
            .expect("the chat window opens");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{err}");
        self.tmux(&[
            "list-panes",
            "-s",
            "-t",
            self.name.as_str(),
            "-F",
            "#{pane_id}|#{@ae_console}",
        ])
        .lines()
        .find_map(|row| {
            let (pane, stamp) = row.split_once('|')?;
            (stamp == UUID).then(|| pane.to_owned())
        })
        .expect("the stamped chat window")
    }
    fn wait(&self, pane: &str, within: Duration, why: &str, met: impl Fn(&str) -> bool) -> String {
        let until = Instant::now() + within;
        loop {
            let out = self.tmux(&["capture-pane", "-p", "-t", pane]);
            let screen = out.replace('\n', "");
            if met(&screen) {
                return screen;
            }
            assert!(Instant::now() < until, "{why}; terminal screen:\n{screen}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    fn keys(&self, pane: &str, tail: &[&str]) {
        self.tmux(&[&["send-keys", "-t", pane], tail].concat());
    }
    /// Every entry under `messages/`, sorted: a refusal must add none of any name.
    fn entries(&self) -> Vec<(String, PathBuf)> {
        let mut found = Vec::new();
        if let Ok(listing) = fs::read_dir(self.tool.dir.join("messages")) {
            for entry in listing.flatten() {
                found.push((
                    entry.file_name().to_string_lossy().into_owned(),
                    entry.path(),
                ));
            }
        }
        found.sort();
        found
    }
    fn images(&self) -> Vec<(String, PathBuf)> {
        self.entries().into_iter().filter(|(n, _)| png(n)).collect()
    }

    /// Wait until `n` images are saved; the newest, proven 0600 + whole.
    fn image(&self, n: usize) -> (String, PathBuf) {
        until("saved image count", || self.images().len() == n);
        let (name, path) = self.images().into_iter().last().expect("the newest image");
        assert!(name.starts_with("img-"), "minted name: {name}");
        let mode = fs::metadata(&path).expect("image").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "saved image mode");
        assert_eq!(fs::read(&path).expect("saved bytes"), PNG, "saved bytes");
        (name, path)
    }
}
fn png(name: &str) -> bool {
    std::path::Path::new(name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
}
fn until(why: &str, mut met: impl FnMut() -> bool) {
    let until = Instant::now() + WAIT;
    while !met() {
        assert!(Instant::now() < until, "{why}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn ctrl_v_in_the_app_composer_inserts_the_saved_path_and_refusals_keep_the_draft() {
    let rig = Rig::new("imgapp", true);
    let pane = rig.open();
    let composer = format!("to {}", rig.name);
    rig.wait(&pane, WAIT, "the owner composer", |s| s.contains(&composer));
    rig.keys(&pane, &["i"]);
    rig.wait(&pane, WAIT, "writing starts", crate::app_mode::writing);

    fs::write(rig.root.join("clip-mode"), "png").expect("clipboard mode");
    rig.keys(&pane, &["-l", "ab"]);
    rig.keys(&pane, &["Left"]);
    rig.keys(&pane, &["C-v"]);
    let (first, path) = rig.image(1);
    let screen = rig.wait(&pane, WAIT, "path shown", |s| s.contains(&first));
    assert!(
        screen.contains(&format!("a {} b", path.display())),
        "spaced: {screen}"
    );
    assert!(
        !rig.tool.events().contains("\"ask\""),
        "Ctrl+V sends nothing"
    );

    // Every refusal: ONE why on the hint row, the draft kept, and no new
    // entry of any name in `messages/` (a scratch or temp would show here).
    let kept = rig.entries();
    for (mode, why) in [
        ("empty", "paste not taken: no image on the clipboard"),
        ("junk", "paste not taken: clipboard image is not a PNG"),
        ("huge", "paste not taken: clipboard image over 20 MiB"),
        ("fail", "paste not taken: clipboard tool failed"),
    ] {
        fs::write(rig.root.join("clip-mode"), mode).expect("clipboard mode");
        rig.keys(&pane, &["C-v"]);
        rig.wait(&pane, WAIT, why, |s| s.contains(why));
        assert_eq!(rig.entries(), kept, "{mode} writes nothing");
    }
    fs::remove_file(rig.root.join("clip")).expect("the tool goes missing");
    rig.keys(&pane, &["C-v"]);
    let missing = "paste not taken: clipboard tool missing: clip-link";
    rig.wait(&pane, WAIT, missing, |s| s.contains(missing));
    assert_eq!(rig.entries(), kept, "a missing tool writes nothing");
    rig.write_clip();
    rig.wait(&pane, WAIT, "refusals keep the draft", |s| {
        s.contains(&first)
    });

    // A hung tool never freezes the UI: a typed char is drawn while the
    // read is still running, before its timed-out refusal lands.
    fs::write(rig.root.join("clip-mode"), "hang").expect("clipboard mode");
    rig.keys(&pane, &["C-v"]);
    rig.keys(&pane, &["-l", "Z"]);
    let timed = "paste not taken: clipboard read timed out";
    let typed = format!("{first} Zb");
    let screen = rig.wait(&pane, WAIT, "typing drawn mid-read", |s| s.contains(&typed));
    assert!(
        !screen.contains(timed),
        "drawn before the refusal: {screen}"
    );
    rig.wait(&pane, Duration::from_secs(15), timed, |s| s.contains(timed));
    assert_eq!(rig.entries(), kept, "a hang writes nothing");
    fs::write(rig.root.join("clip-mode"), "png").expect("clipboard mode");
    rig.keys(&pane, &["C-v"]);
    let (second, _) = rig.image(2);
    assert_ne!(first, second, "each paste mints its own name");
}

#[test]
fn ctrl_v_in_the_owner_chat_inserts_and_only_enter_sends() {
    let rig = Rig::new("imgchat", false);
    let pane = rig.open();
    rig.wait(&pane, WAIT, "the owner prompt", |s| s.contains("to lead>"));
    rig.keys(&pane, &["-l", "look"]);
    fs::write(rig.root.join("clip-mode"), "png").expect("clipboard mode");
    rig.keys(&pane, &["C-v"]);
    let (name, path) = rig.image(1);
    rig.wait(&pane, WAIT, "path shown", |s| s.contains(&name));
    assert!(
        !rig.tool.events().contains("\"ask\""),
        "Ctrl+V sends nothing"
    );

    rig.keys(&pane, &["Enter"]);
    let body = path.display().to_string();
    until("the ask body carries the path", || {
        rig.entries()
            .into_iter()
            .any(|(_, p)| fs::read_to_string(p).is_ok_and(|t| t.contains(&body)))
    });

    // `deliver::store_body` publishes by link-then-unlink, so the body's `.txt`
    // shows while its temp name is still there. The ask helper journals only
    // after delivery returned, so the record is the completion signal: the
    // snapshot below is of a settled `messages/`, not a filtered one.
    until("the ask is journaled", || {
        rig.tool.events().contains("\"action\":\"ask\"")
    });

    // A refusal prints its why, keeps the typed draft, and writes nothing.
    let kept = rig.entries();
    rig.keys(&pane, &["-l", "keep"]);
    fs::write(rig.root.join("clip-mode"), "junk").expect("clipboard mode");
    rig.keys(&pane, &["C-v"]);
    let why = "refused: clipboard image is not a PNG";
    let screen = rig.wait(&pane, WAIT, why, |s| s.contains(why));
    assert!(
        screen.contains("to lead> keep"),
        "the draft is kept: {screen}"
    );
    assert_eq!(rig.entries(), kept, "a refusal writes nothing");
}
