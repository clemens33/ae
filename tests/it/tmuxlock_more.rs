//! The kill-server lock, beyond the frozen acceptance spec: every spelling tmux
//! resolves to `kill-server`, the exit status of a refusal, and a user's own
//! same-name alias. A real installed core launches a real server; every tmux
//! client is a private-socket one (`-S`, or a seat-style `TMUX` with its own
//! `TMUX_TMPDIR`), never the user's server.

#![allow(
    clippy::disallowed_methods,
    reason = "the fixture plants a real installed core and reads its private server"
)]

use std::os::unix::fs::MetadataExt as _;
use std::path::PathBuf;

use super::cli::{OwnedScratch, helper, helper_by_name};

const CONFIG: &str = "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\n\n\
    [workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\ntheme = off\nchat = off\nauto_upgrade = off\n";
const SESSION: &str = "lockmore";
const SPELLINGS: [&str; 4] = ["kill-server", "kill-serve", "kill-serv", "kill-ser"];

struct Rig {
    scratch: OwnedScratch,
    home: PathBuf,
    root: PathBuf,
    core: PathBuf,
    socket: PathBuf,
    project: PathBuf,
}

impl Rig {
    fn new(tag: &str) -> Self {
        let mut scratch = OwnedScratch::root("tm", tag);
        let home = scratch.join("h");
        let root = home.join(".ae");
        let version = root.join("versions").join(ae::VERSION);
        let core = version.join("ae-core");
        let project = scratch.join("p");
        let uid = std::fs::metadata(scratch.path())
            .unwrap_or_else(|why| panic!("scratch uid: {why}"))
            .uid();
        let socket = scratch.join(format!("tmux-{uid}")).join("ae");
        scratch.add_tmux_server(socket.clone());
        for dir in [&version, &project, &home.join(".local/bin")] {
            std::fs::create_dir_all(dir).unwrap_or_else(|why| panic!("fixture directory: {why}"));
        }
        std::fs::copy(env!("CARGO_BIN_EXE_ae"), &core)
            .unwrap_or_else(|why| panic!("real installed core: {why}"));
        std::fs::write(version.join("install"), "fixture installer member\n")
            .unwrap_or_else(|why| panic!("installer member: {why}"));
        std::fs::write(
            version.join("SHA256SUMS"),
            format!(
                "{}  ae-core\n{}  install\n",
                ae::install::sha256_hex(
                    &std::fs::read(&core)
                        .unwrap_or_else(|why| panic!("installed core bytes: {why}"))
                ),
                ae::install::sha256_hex(b"fixture installer member\n")
            ),
        )
        .unwrap_or_else(|why| panic!("installed manifest: {why}"));
        std::os::unix::fs::symlink(&core, home.join(".local/bin/ae"))
            .unwrap_or_else(|why| panic!("public command: {why}"));
        std::fs::write(root.join("config"), CONFIG)
            .unwrap_or_else(|why| panic!("fixture config: {why}"));
        Self {
            scratch,
            home,
            root,
            core,
            socket,
            project,
        }
    }

    /// One private tmux client. A seat-style client names the server only
    /// through `TMUX`, with a `TMUX_TMPDIR` that does not reach it: the shape
    /// that ended the real server.
    fn tmux_at(&self, seat: bool, words: &[&str]) -> (bool, String) {
        let mut command = helper_by_name("tmux");
        command
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("TMUX_TMPDIR", self.scratch.path())
            .env("SHELL", "/bin/sh");
        if seat {
            command.env("TMUX", format!("{},1,0", self.socket.display()));
            command.env("TMUX_PANE", "%0");
            command.env("TMUX_TMPDIR", &self.project);
        } else {
            command.args(["-S", &self.socket.display().to_string()]);
        }
        let out = command
            .args(words)
            .output()
            .unwrap_or_else(|why| panic!("private tmux client: {why}"));
        (
            out.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        )
    }

    fn tmux(&self, words: &[&str]) -> String {
        let (ok, out) = self.tmux_at(false, words);
        assert!(ok, "private tmux {words:?}: {out}");
        out
    }

    fn launch(&self) {
        let out = helper(&self.core)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("HOME", &self.home)
            .env("TMUX_TMPDIR", self.scratch.path())
            .env("AE_HOME", &self.root)
            .env("CONFIG_FILE", self.root.join("config"))
            .env("AE_NO_AUTOSTART", "1")
            .current_dir(&self.project)
            .args([SESSION, "--no-attach"])
            .output()
            .unwrap_or_else(|why| panic!("launch the product: {why}"));
        assert!(
            out.status.success(),
            "launch: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn identity(&self) -> String {
        self.tmux(&[
            "display-message",
            "-p",
            "-t",
            SESSION,
            "#{pid}|#{session_id}|#{pane_id}",
        ])
    }

    fn aliases(&self) -> String {
        self.tmux(&["show-options", "-s", "command-alias"])
    }

    /// The index of the lowest entry named `name`, as tmux prints it.
    fn option_of(&self, name: &str) -> String {
        self.aliases()
            .lines()
            .find(|line| {
                let rest = line.split_once(' ').map_or("", |(_, rest)| rest);
                rest.trim_start_matches('"')
                    .starts_with(&format!("{name}="))
            })
            .and_then(|line| line.split_whitespace().next())
            .unwrap_or_else(|| panic!("no alias named {name}"))
            .to_owned()
    }

    fn stop_session(&self) {
        self.tmux(&["kill-session", "-t", SESSION]);
        assert!(
            !self.tmux_at(false, &["list-sessions"]).0,
            "the server ends with its last session, guard or not"
        );
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        // Drop the guard so OwnedScratch's own kill-server reaper can end the
        // server after an assertion failure. This fixture's socket only.
        let _ = helper_by_name("tmux")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .args([
                "-S",
                &self.socket.display().to_string(),
                "set-option",
                "-su",
                "command-alias",
            ])
            .output();
    }
}

#[test]
fn every_spelling_of_kill_server_is_refused_with_a_failing_exit_and_kill_session_is_not() {
    let rig = Rig::new("spell");
    rig.launch();
    let identity = rig.identity();
    for seat in [false, true] {
        for spelling in SPELLINGS {
            let (ok, out) = rig.tmux_at(seat, &[spelling]);
            assert!(
                !ok,
                "{spelling} seat={seat}: a refusal exits non-zero, or a reaper reads it as done: {out}"
            );
            assert!(
                out.contains("ae stop"),
                "{spelling} seat={seat} names the supported stop: {out}"
            );
            assert_eq!(
                rig.identity(),
                identity,
                "{spelling} seat={seat} ended or replaced the server"
            );
        }
    }
    rig.tmux(&["new-session", "-d", "-s", "extra", "sleep", "600"]);
    rig.tmux(&["kill-session", "-t", "extra"]);
    assert_eq!(rig.identity(), identity, "kill-session is not guarded");
    rig.stop_session();
}

#[test]
fn a_users_own_kill_server_alias_cannot_defeat_the_guard() {
    let rig = Rig::new("shadow");
    rig.launch();
    let identity = rig.identity();
    // The user's entry takes the guard's place, at the LOWEST index, with
    // another of theirs above it that tmux would only reach if the first went.
    let guard = rig.option_of("kill-server");
    rig.tmux(&["set-option", "-su", &guard]);
    rig.tmux(&[
        "set-option",
        "-s",
        &guard,
        "kill-server=display-message -p user-lowest",
    ]);
    rig.tmux(&[
        "set-option",
        "-s",
        "command-alias[40]",
        "kill-server=display-message -p user-higher",
    ]);
    rig.tmux(&[
        "set-option",
        "-s",
        "command-alias[41]",
        "mine=display-message -p mine",
    ]);
    let before = rig.aliases();
    rig.launch();
    let after = rig.aliases();
    let (ok, out) = rig.tmux_at(false, &["kill-server"]);
    assert!(!ok && out.contains("ae stop"), "guard wins: {out}");
    assert_eq!(rig.identity(), identity);
    assert!(
        !after.contains("user-lowest"),
        "the lowest same-name entry is the one displaced: {after}"
    );
    for kept in ["user-higher", "mine=display-message -p mine"] {
        assert!(
            after.contains(kept),
            "{kept} is not the guard's to touch: {after}"
        );
    }
    let count = |text: &str| text.lines().count();
    assert_eq!(
        count(&after),
        count(&before),
        "overwrite in place, no growth"
    );
    rig.launch();
    assert_eq!(rig.aliases(), after, "a repeat changes nothing");
    rig.stop_session();
}
