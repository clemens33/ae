//! Ghostty hyperlinks on an ae-owned tmux server.
//!
//! Behaviour only. A selected server's binding list carries one op whose
//! argv contains the entry `xterm-ghostty:hyperlinks` as its own word; that
//! pin lives beside `status_bindings_argv`. Here the checkout binary's launch
//! is what adds the row: exact complete-line equality, so a pretend row and
//! `xterm-ghostty:RGB:hyperlinks` do not count as present. A second launch
//! does not add another. `ae doctor` names a missing row and goes quiet once
//! the launch has added it. `theme = off` is in the config because the assert
//! is input capability, not look.

#![allow(
    clippy::disallowed_methods,
    reason = "the fixture builds a scratch home and reads the report and the option back"
)]

use std::path::{Path, PathBuf};

use super::cli::{OwnedScratch, ae};
use super::phase2::run_tmux;

const ENTRY: &str = "xterm-ghostty:hyperlinks";
const MISSING: &str = "terminal-features missing xterm-ghostty:hyperlinks";
const SESSION: &str = "lfonce";

const CONFIG: &str = "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\n\n\
     [workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\ntheme = off\nchat = off\n";

struct Rig {
    scratch: OwnedScratch,
    sock: PathBuf,
    home: PathBuf,
    project: PathBuf,
    config: PathBuf,
}

impl Rig {
    fn new(tag: &str) -> Self {
        let mut scratch = OwnedScratch::root("lf", tag);
        let home = scratch.join("aehome");
        let project = scratch.join("project");
        assert!(std::fs::create_dir_all(&project).is_ok(), "a project dir");
        let config = scratch.join("config");
        assert!(std::fs::write(&config, CONFIG).is_ok(), "a config");
        let sock = scratch.join("sock");
        scratch.add_tmux_server(sock.clone());
        Self {
            scratch,
            sock,
            home,
            project,
            config,
        }
    }

    fn tmux(&self, tail: &[&str]) -> (bool, String) {
        let mut args = ae::tmux::server_args(&ae::inventory::ServerId::Selected(
            ae::meta::Selector::Socket(self.sock.clone()),
        ));
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        run_tmux(&args, &self.scratch)
    }

    fn start(&self) {
        let (ok, out) = self.tmux(&[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "holder",
            "-x",
            "80",
            "-y",
            "24",
        ]);
        assert!(ok, "server did not start: {out}");
    }

    fn seed(&self, value: &str) {
        let (ok, out) = self.tmux(&["set-option", "-sa", "terminal-features", value]);
        assert!(ok, "seed {value}: {out}");
    }

    fn values(&self) -> Vec<String> {
        let (ok, out) = self.tmux(&["show-options", "-s", "-v", "terminal-features"]);
        assert!(ok, "show-options: {out}");
        out.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect()
    }

    fn launch(&self) -> (Option<i32>, String, String) {
        let out = ae()
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("HOME", self.scratch.path())
            .env("TMUX_TMPDIR", self.scratch.path())
            .env("AE_HOME", &self.home)
            .env("CONFIG_FILE", &self.config)
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &self.sock)
            .env("AE_NO_AUTOSTART", "1")
            .current_dir(&self.project)
            .args([SESSION, "--no-attach"])
            .output()
            .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn doctor(&self) -> (Option<i32>, String, String) {
        let config = self.config.display().to_string();
        let out = ae()
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("HOME", self.scratch.path())
            .env("TMUX_TMPDIR", self.scratch.path())
            .env("AE_HOME", &self.home)
            .env("CONFIG_FILE", &self.config)
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &self.sock)
            .args(["doctor", "--global", &config])
            .output()
            .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

fn exact_count(values: &[String]) -> usize {
    values.iter().filter(|line| line.as_str() == ENTRY).count()
}

#[test]
fn a_launch_adds_ghostty_hyperlinks_once_and_a_decoy_row_does_not_count() {
    let rig = Rig::new("once");
    rig.start();
    rig.seed("user-terminal:RGB");
    rig.seed("pretend-xterm-ghostty:hyperlinks");
    rig.seed("xterm-ghostty:RGB:hyperlinks");
    assert_eq!(exact_count(&rig.values()), 0, "setup adds no exact row");
    let first = rig.launch();
    assert_eq!(first.0, Some(0), "launch: {first:?}");
    let once = rig.values();
    assert_eq!(
        exact_count(&once),
        1,
        "one exact row after launch: {once:?}"
    );
    assert!(
        once.iter().any(|line| line == "user-terminal:RGB"),
        "{once:?}"
    );
    assert!(
        once.iter()
            .any(|line| line == "pretend-xterm-ghostty:hyperlinks"),
        "{once:?}"
    );
    assert!(
        once.iter()
            .any(|line| line == "xterm-ghostty:RGB:hyperlinks"),
        "{once:?}"
    );
    let second = rig.launch();
    assert_eq!(second.0, Some(0), "second launch: {second:?}");
    assert_eq!(
        exact_count(&rig.values()),
        1,
        "a second assert must not append"
    );
}

#[test]
fn doctor_names_a_missing_ghostty_hyperlinks_entry_until_launch() {
    let rig = Rig::new("doc");
    rig.start();
    rig.seed("pretend-xterm-ghostty:hyperlinks");
    rig.seed("xterm-ghostty:RGB:hyperlinks");
    let (before_code, before, before_err) = rig.doctor();
    assert!(
        before.contains(MISSING),
        "doctor before launch ({before_code:?}): {before}\n{before_err}"
    );
    let launched = rig.launch();
    assert_eq!(launched.0, Some(0), "launch: {launched:?}");
    let (_after_code, after, after_err) = rig.doctor();
    assert!(
        !after.contains(MISSING),
        "doctor after launch still names a missing row: {after}\n{after_err}"
    );
}

#[test]
fn the_docs_name_ghostty_hyperlinks_on_ae_servers_only() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let read = |rel: &str| {
        std::fs::read_to_string(root.join(rel)).unwrap_or_else(|err| panic!("{rel}: {err}"))
    };
    for rel in ["docs/app.md", "docs/chat.md"] {
        let text = read(rel);
        assert!(text.contains(ENTRY), "{rel} names the Ghostty entry");
        assert!(
            !text.contains("ae never writes the option"),
            "{rel} still says ae never writes terminal-features"
        );
    }
    let agents = read("AGENTS.md");
    assert!(agents.contains(ENTRY), "AGENTS.md names the Ghostty entry");
    assert!(
        agents.contains("the user's tmux is untouched"),
        "ae's own server is still the boundary"
    );
    let theme = read("src/theme.rs");
    assert!(theme.contains(ENTRY), "theme.rs names the Ghostty entry");
    assert!(
        !theme.contains("is never touched"),
        "theme.rs still says terminal-features is never touched"
    );
    assert!(
        read("docs/internals/theme.md").contains(ENTRY),
        "docs/internals/theme.md names the Ghostty entry"
    );
    for rel in [
        "src/app/applink_spec.rs",
        "src/console/applink_chat_spec.rs",
    ] {
        let text = read(rel);
        assert!(text.contains(ENTRY), "{rel}");
        assert!(
            !text.contains("ae never writes that server option"),
            "{rel}"
        );
    }
}
