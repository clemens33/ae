//! Independent behavioral contract for installed-server kill-server refusal.
//! Frozen by tmuxlock-nav; driver installs these bytes without modification.

#![allow(
    clippy::disallowed_methods,
    reason = "the acceptance fixture plants a real installed core and reads its private server"
)]

use std::os::unix::fs::MetadataExt as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use super::cli::{OwnedScratch, ae, helper, helper_by_name};

const CONFIG: &str = "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\n\n\
    [workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\ntheme = off\nchat = off\nauto_upgrade = off\n";
const SESSION: &str = "lockproof";

struct Rig {
    scratch: OwnedScratch,
    home: PathBuf,
    root: PathBuf,
    core: PathBuf,
    socket: PathBuf,
    user_socket: PathBuf,
    project: PathBuf,
}

impl Rig {
    fn new(tag: &str) -> Self {
        let mut scratch = OwnedScratch::root("tl", tag);
        let home = scratch.join("h");
        let root = home.join(".ae");
        let version = root.join("versions").join(ae::VERSION);
        let core = version.join("ae-core");
        let project = scratch.join("p");
        let uid = std::fs::metadata(scratch.path())
            .unwrap_or_else(|why| panic!("scratch uid: {why}"))
            .uid();
        let socket = scratch.join(format!("tmux-{uid}")).join("ae");
        let user_socket = scratch.join("user");
        scratch.add_tmux_server(socket.clone());
        scratch.add_tmux_server(user_socket.clone());
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
            user_socket,
            project,
        }
    }

    fn tmux_at(&self, user: bool, seat: bool, words: &[&str]) -> (bool, String) {
        let socket = if user {
            &self.user_socket
        } else {
            &self.socket
        };
        let mut command = helper_by_name("tmux");
        command
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("TMUX_TMPDIR", self.scratch.path())
            .env("SHELL", "/bin/sh");
        if seat {
            command.env("TMUX", format!("{},1,0", socket.display()));
            command.env("TMUX_PANE", "%0");
            command.env("TMUX_TMPDIR", &self.project);
        } else {
            command.args(["-S", &socket.display().to_string()]);
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
        let (ok, out) = self.tmux_at(false, false, words);
        assert!(ok, "private tmux {words:?}: {out}");
        out
    }

    fn start(&self, user: bool) {
        let socket = if user {
            &self.user_socket
        } else {
            &self.socket
        };
        std::fs::create_dir_all(socket.parent().unwrap_or_else(|| panic!("socket parent")))
            .unwrap_or_else(|why| panic!("private socket directory: {why}"));
        std::fs::set_permissions(
            socket.parent().unwrap_or_else(|| panic!("socket parent")),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap_or_else(|why| panic!("private socket directory permissions: {why}"));
        let (ok, out) = self.tmux_at(
            user,
            false,
            &[
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                "holder",
                "sleep",
                "600",
            ],
        );
        assert!(ok, "private server start: {out}");
    }

    fn launch(&self, installed: bool) {
        let mut command = if installed { helper(&self.core) } else { ae() };
        let out = command
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("HOME", &self.home)
            .env("TMUX_TMPDIR", self.scratch.path())
            .env("AE_HOME", &self.root)
            .env("CONFIG_FILE", self.root.join("config"))
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &self.socket)
            .env("AE_NO_AUTOSTART", "1")
            .current_dir(&self.project)
            .args([SESSION, "--no-attach"])
            .output()
            .unwrap_or_else(|why| panic!("launch the product: {why}"));
        assert!(
            out.status.success(),
            "launch installed={installed}: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn aliases(&self) -> String {
        self.tmux(&["show-options", "-s", "command-alias"])
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

    fn assert_refused(&self, identity: &str) {
        for seat in [false, true] {
            let (_, out) = self.tmux_at(false, seat, &["kill-server"]);
            assert_eq!(
                self.identity(),
                identity,
                "kill-server seat={seat} changed live identity"
            );
            assert!(
                out.contains("ae stop") || out.contains("ae end"),
                "refusal must name supported stop, seat={seat}: {out}"
            );
        }
    }

    fn remove_guard(&self) {
        let aliases = self.aliases();
        for line in aliases.lines().filter(|line| line.contains("kill-server=")) {
            let option = line.split_whitespace().next().unwrap_or_default();
            self.tmux(&["set-option", "-su", option]);
        }
    }

    fn publish(&self) {
        let bundle = self.scratch.join("bundle");
        std::fs::create_dir_all(&bundle).unwrap_or_else(|why| panic!("bundle directory: {why}"));
        let core_bytes = std::fs::read(env!("CARGO_BIN_EXE_ae"))
            .unwrap_or_else(|why| panic!("bundle core bytes: {why}"));
        let installer_bytes = b"fixture installer member\n";
        for (name, bytes) in [
            ("ae-core", core_bytes.as_slice()),
            ("install", installer_bytes.as_slice()),
        ] {
            std::fs::write(bundle.join(name), bytes)
                .unwrap_or_else(|why| panic!("bundle member: {why}"));
            std::fs::set_permissions(bundle.join(name), std::fs::Permissions::from_mode(0o755))
                .unwrap_or_else(|why| panic!("bundle member permissions: {why}"));
        }
        std::fs::write(
            bundle.join("SHA256SUMS"),
            format!(
                "{}  ae-core\n{}  install\n",
                ae::install::sha256_hex(&core_bytes),
                ae::install::sha256_hex(installer_bytes)
            ),
        )
        .unwrap_or_else(|why| panic!("bundle manifest: {why}"));
        let out = ae()
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("AE_HOME")
            .env_remove("CONFIG_FILE")
            .env_remove("AE_TMUX_SERVER")
            .env_remove("AE_TMUX_SERVER_KIND")
            .env_remove("AE_VERSION")
            .env("HOME", &self.home)
            .env("TMUX_TMPDIR", self.scratch.path())
            .env("AE_NO_AUTOSTART", "1")
            .current_dir(&self.project)
            .args(["_install", "--from", &bundle.display().to_string()])
            .output()
            .unwrap_or_else(|why| panic!("checkout _install process: {why}"));
        assert!(
            out.status.success(),
            "publish: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn stop_sessions(&self) {
        let sessions = self.tmux(&["list-sessions", "-F", "#{session_id}"]);
        for session in sessions.lines() {
            self.tmux(&["kill-session", "-t", session]);
        }
        assert!(
            !self.tmux_at(false, false, &["list-sessions"]).0,
            "fixture teardown must end the server, even while the alias remains"
        );
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        // Permit OwnedScratch's existing kill-server reaper after an assertion
        // failure. Scope is this fixture's explicit private socket only.
        let output = helper_by_name("tmux")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .args([
                "-S",
                &self.socket.display().to_string(),
                "show-options",
                "-s",
                "command-alias",
            ])
            .output();
        if let Ok(output) = output {
            let aliases = String::from_utf8_lossy(&output.stdout);
            for line in aliases.lines().filter(|line| line.contains("kill-server=")) {
                let option = line.split_whitespace().next().unwrap_or_default();
                let _ = helper_by_name("tmux")
                    .env_remove("TMUX")
                    .env_remove("TMUX_PANE")
                    .args([
                        "-S",
                        &self.socket.display().to_string(),
                        "set-option",
                        "-su",
                        option,
                    ])
                    .output();
            }
        }
    }
}

#[test]
fn installed_launch_refuses_both_clients_preserves_aliases_and_other_servers() {
    let rig = Rig::new("launch");
    rig.start(false);
    rig.start(true);
    rig.tmux(&[
        "set-option",
        "-s",
        "command-alias[17]",
        "user-kept=display-message -p user-kept",
    ]);
    let before = rig.aliases();
    let user_before = rig
        .tmux_at(true, false, &["show-options", "-s", "command-alias"])
        .1;
    rig.launch(true);
    let identity = rig.identity();
    rig.assert_refused(&identity);
    let guarded = rig.aliases();
    for line in before.lines() {
        assert!(
            guarded.lines().any(|after| after == line),
            "user alias lost: {line}"
        );
    }
    rig.launch(true);
    assert_eq!(
        rig.aliases(),
        guarded,
        "repeated launch must not grow aliases"
    );

    let guard_option = guarded
        .lines()
        .find(|line| line.contains("kill-server="))
        .and_then(|line| line.split_whitespace().next())
        .unwrap_or_else(|| panic!("installed guard alias missing"));
    rig.tmux(&[
        "set-option",
        "-s",
        guard_option,
        "collision-kept=display-message -p collision-kept",
    ]);
    let collision = rig.aliases();
    rig.launch(true);
    let restored = rig.aliases();
    for line in collision.lines() {
        assert!(
            restored.lines().any(|after| after == line),
            "index collision overwrote user alias: {line}"
        );
    }
    rig.assert_refused(&identity);
    rig.launch(true);
    assert_eq!(
        rig.aliases(),
        restored,
        "collision repair must remain idempotent"
    );
    assert_eq!(
        rig.tmux_at(true, false, &["show-options", "-s", "command-alias"])
            .1,
        user_before
    );
    assert!(
        rig.tmux_at(true, false, &["kill-server"]).0,
        "separate user server remains killable"
    );
    assert!(
        !rig.tmux_at(true, false, &["list-sessions"]).0,
        "separate user server actually ended"
    );
    rig.stop_sessions();
}

#[test]
fn checkout_publish_reasserts_guard_on_running_installed_root_without_restart() {
    let rig = Rig::new("publish");
    rig.launch(true);
    let identity = rig.identity();
    rig.remove_guard();
    rig.tmux(&[
        "set-option",
        "-s",
        "command-alias[23]",
        "publish-kept=display-message -p publish-kept",
    ]);
    let before = rig.aliases();
    // A downloaded _install runs outside a published version directory too.
    // Use the real publisher with the fixture's named-server namespace.
    rig.publish();
    rig.assert_refused(&identity);
    let guarded = rig.aliases();
    for line in before.lines() {
        assert!(
            guarded.lines().any(|after| after == line),
            "publish lost alias: {line}"
        );
    }
    rig.publish();
    assert_eq!(rig.aliases(), guarded, "publish must not grow aliases");
    rig.assert_refused(&identity);
    rig.stop_sessions();
}

#[test]
fn checkout_launch_remains_killable_for_existing_fixture_cleanup() {
    let rig = Rig::new("checkout");
    rig.start(false);
    rig.launch(false);
    let _ = rig.identity();
    assert!(
        rig.tmux_at(false, false, &["kill-server"]).0,
        "checkout cleanup command succeeds"
    );
    assert!(
        !rig.tmux_at(false, false, &["list-sessions"]).0,
        "checkout server actually ended"
    );
}
