//! The migration chain, and the upgrade sweep that runs it over every session.
//!
//! Three arms, because the contract has three halves that cannot be proven the
//! same way:
//!
//! * the VERSION-DIRECTORY sweep and the chain's refusal are library facts over
//!   real directories — no process and no tmux needed;
//! * the STOPPED-session sweep is black-box through `ae _install --from`,
//!   because what is being proven is that a real publish repoints real sessions
//!   before it repoints the command link;
//! * the RUNNING-session daemon restart needs a real tmux server, for the same
//!   reason [`super::daemons`] does: what can go wrong is what the guards make
//!   of a live server's answers.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use super::parity::Invocation;
use super::parity::capture::raw;
use super::phase2::{run_tmux, tmux_present};

/// A fixture `$HOME`: `<home>/.ae` is the state root, `<home>/.local/bin/ae`
/// the command link — the two paths a publish derives.
struct Rig {
    scratch: PathBuf,
    home: PathBuf,
}

impl Rig {
    fn new(tag: &str) -> Self {
        let scratch = PathBuf::from(format!("/tmp/aemig.{}.{tag}", std::process::id()));
        let _ = remove(&scratch);
        let home = scratch.join("home");
        assert!(fs::create_dir_all(&home).is_ok(), "a fixture home");
        Self { scratch, home }
    }

    fn root(&self) -> PathBuf {
        self.home.join(".ae")
    }

    fn versions(&self) -> PathBuf {
        self.root().join("versions")
    }

    fn link(&self) -> PathBuf {
        self.home.join(".local").join("bin").join("ae")
    }

    /// A bundle whose core reports `version` — the three members `just bundle`
    /// packages, with the manifest both `sha256sum` spellings accept.
    fn bundle(&self, version: &str) -> PathBuf {
        let dir = self.scratch.join(format!("ae-{version}-fixture"));
        assert!(fs::create_dir_all(&dir).is_ok(), "a bundle root");
        write_exec(
            &dir.join("ae-core"),
            &format!("#!/bin/sh\necho \"ae {version}\"\n"),
        );
        write_exec(&dir.join("install"), "#!/bin/sh\necho bootstrap\n");
        let mut manifest = String::new();
        for name in ["ae-core", "install"] {
            let bytes = fs::read(dir.join(name))
                .unwrap_or_else(|why| panic!("{name} should be readable: {why}"));
            let _ = writeln!(manifest, "{}  {name}", ae::install::sha256_hex(&bytes));
        }
        let path = dir.join("SHA256SUMS");
        let _ = fs::remove_file(&path);
        assert!(fs::write(&path, manifest).is_ok(), "a manifest");
        dir
    }

    /// A stopped session recording `core` and declaring `version_row`.
    fn session(&self, name: &str, core: &str, version_row: Option<u32>) -> PathBuf {
        let dir = self.root().join("sessions").join(name);
        assert!(fs::create_dir_all(&dir).is_ok(), "a session dir");
        let mut meta = String::new();
        if let Some(version) = version_row {
            let _ = writeln!(meta, "{}={version}", ae::migrate::KEY);
        }
        let _ = write!(
            meta,
            "mode=local\nsession={name}\nwork_dir=/w\nae_version=0.0.1\n\
             ae_core={core}\nae_core_version=0.0.1\n\
             schema=2\nseat.main=lead\nprofile.main=cl\n"
        );
        assert!(fs::write(dir.join("meta"), meta).is_ok(), "a meta");
        dir
    }

    /// A version directory with one member, standing in for a published one.
    fn plant_version(&self, version: &str) -> PathBuf {
        let dir = self.versions().join(version);
        assert!(fs::create_dir_all(&dir).is_ok(), "a version dir");
        assert!(fs::write(dir.join("ae-core"), "old\n").is_ok(), "a core");
        dir
    }

    fn install(&self, from: &Path) -> (Option<i32>, String, String) {
        #[allow(
            clippy::disallowed_types,
            reason = "the black-box door: a publish is what a real process does to a real HOME"
        )]
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_ae"));
        let out = command
            .env_remove("AE_HOME")
            .env_remove("CONFIG_FILE")
            .env_remove("AE_VERSION")
            .env_remove("TMUX")
            .env("HOME", &self.home)
            .args(["_install", "--from", &from.to_string_lossy()])
            .output()
            .unwrap_or_else(|why| panic!("the product binary should run: {why}"));
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = remove(&self.scratch);
    }
}

fn write_exec(path: &Path, text: &str) {
    let _ = fs::remove_file(path);
    assert!(fs::write(path, text).is_ok(), "a script");
    assert!(
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).is_ok(),
        "an executable script"
    );
}

/// Remove a tree whose members may be 0555 — the mode a publish leaves.
fn remove(path: &Path) -> std::io::Result<()> {
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let _ = fs::set_permissions(entry.path(), fs::Permissions::from_mode(0o755));
            if entry.path().is_dir() {
                let _ = remove(&entry.path());
            }
        }
    }
    fs::remove_dir_all(path)
}

fn meta_of(dir: &Path) -> String {
    fs::read_to_string(dir.join("meta"))
        .unwrap_or_else(|why| panic!("{}: {why}", dir.join("meta").display()))
}

fn value_of(meta: &str, key: &str) -> Option<String> {
    meta.lines()
        .filter_map(|line| line.split_once('='))
        .find(|(found, _)| *found == key)
        .map(|(_, value)| value.to_owned())
}

fn present(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

// ─── the chain itself ────────────────────────────────────────────────────

#[test]
fn a_session_at_the_current_version_is_left_byte_for_byte_alone() {
    let rig = Rig::new("noop");
    let dir = rig.session("cur", "/nowhere/ae-core", Some(ae::migrate::CURRENT));
    let before = meta_of(&dir);
    assert_eq!(ae::migrate::session(&dir), Ok(None));
    assert_eq!(meta_of(&dir), before, "a no-op chain rewrote the meta");
}

#[test]
fn a_session_with_no_version_row_is_refused_by_name_with_the_fresh_start_line() {
    let rig = Rig::new("preversion");
    let dir = rig.session("old", "/nowhere/ae-core", None);
    let refused = ae::migrate::session(&dir).expect_err("the pre-version past");
    assert_eq!(refused, ae::migrate::Refusal::Absent);
    let line = refused.line("old");
    assert!(line.contains("ae end old"), "{line}");
    assert!(line.contains(ae::migrate::KEY), "{line}");

    // A stop or an end must still be possible: the note is written, and it is
    // a note, not a refusal.
    let noted = ae::migrate::session_noted(&dir, "old").expect("a reported refusal");
    assert!(noted.starts_with("note: "), "{noted}");
    assert!(noted.contains("ae end old"), "{noted}");
}

#[test]
fn a_directory_under_sessions_with_no_meta_is_nothing_to_migrate() {
    let rig = Rig::new("nometa");
    let dir = rig.root().join("sessions").join("hollow");
    assert!(fs::create_dir_all(&dir).is_ok(), "a hollow session dir");
    assert_eq!(
        ae::migrate::session(&dir),
        Err(ae::migrate::Refusal::Missing)
    );
    assert_eq!(ae::migrate::session_noted(&dir, "hollow"), None);
}

// ─── the version-directory sweep ─────────────────────────────────────────

#[test]
fn the_version_sweep_keeps_the_published_one_and_every_one_a_session_records() {
    let rig = Rig::new("prune");
    for version in ["2026.1.1", "2026.2.2", "2026.3.3"] {
        rig.plant_version(version);
    }
    // One session still names 2026.2.2 — the case the sweep exists to be safe
    // about, because a session left behind must not lose its core.
    let kept = rig.versions().join("2026.2.2").join("ae-core");
    rig.session("holds", &kept.to_string_lossy(), Some(ae::migrate::CURRENT));
    // A session whose core lives somewhere else entirely protects nothing here.
    rig.session("foreign", "/usr/local/bin/ae", Some(ae::migrate::CURRENT));

    let notes = ae::migrate::prune_versions(&rig.root(), "2026.3.3");
    assert!(
        present(&rig.versions().join("2026.3.3")),
        "the published version was pruned"
    );
    assert!(present(&kept), "a version a session records was pruned");
    assert!(
        !present(&rig.versions().join("2026.1.1")),
        "an unreferenced version survived"
    );
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("2026.1.1"), "{notes:?}");
}

// ─── the publish, black-box ──────────────────────────────────────────────

#[test]
fn a_publish_repoints_every_stopped_session_before_it_repoints_the_command_link() {
    let rig = Rig::new("stopped");
    let stale = rig.plant_version("2026.1.1");
    let one = rig.session(
        "alpha",
        &stale.join("ae-core").to_string_lossy(),
        Some(ae::migrate::CURRENT),
    );
    let two = rig.session(
        "beta",
        &stale.join("ae-core").to_string_lossy(),
        Some(ae::migrate::CURRENT),
    );

    let (code, stdout, stderr) = rig.install(&rig.bundle("2026.9.9"));
    assert_eq!(code, Some(0), "install failed: {stdout}{stderr}");

    let published = rig.versions().join("2026.9.9").join("ae-core");
    for dir in [&one, &two] {
        let meta = meta_of(dir);
        assert_eq!(
            value_of(&meta, "ae_core").as_deref(),
            Some(published.to_string_lossy().as_ref()),
            "the session still names the old core: {meta}"
        );
        assert_eq!(
            value_of(&meta, "ae_core_version").as_deref(),
            Some("2026.9.9")
        );
        assert_eq!(value_of(&meta, "ae_version").as_deref(), Some("2026.9.9"));
        // EVERY helper, enumerated from the product's own list rather than
        // sampled: a name left on the old core is a helper an agent calls and
        // gets the wrong binary from, and a five-name sample cannot see it.
        for helper in ae::shim::HELPERS {
            assert_eq!(
                fs::read_link(dir.join(helper.name)).ok(),
                Some(published.clone()),
                "{} does not name the published core",
                helper.name
            );
        }
    }
    assert_eq!(
        fs::read_link(rig.link()).ok(),
        Some(published),
        "the command link does not name the published core"
    );
    // The version nothing records any more is gone, and the publish said so.
    assert!(
        !present(&stale),
        "the superseded version directory survived"
    );
    assert!(stdout.contains("2026.1.1"), "unreported prune: {stdout}");
}

#[test]
fn a_session_the_chain_refuses_aborts_the_publish_with_the_old_link_intact() {
    let rig = Rig::new("refuse");
    // A first publish, so there is a command link with something to lose.
    let (code, stdout, stderr) = rig.install(&rig.bundle("2026.9.9"));
    assert_eq!(code, Some(0), "the first install failed: {stdout}{stderr}");
    let first = rig.versions().join("2026.9.9").join("ae-core");

    let refused = rig.session("legacy", "/nowhere/ae-core", None);
    let before = meta_of(&refused);

    let (code, stdout, stderr) = rig.install(&rig.bundle("2026.9.10"));
    assert_eq!(code, Some(1), "the publish did not fail: {stdout}{stderr}");
    assert!(
        stderr.contains("legacy"),
        "the refusal names no session: {stderr}"
    );
    assert!(
        stderr.contains(ae::migrate::KEY),
        "the refusal does not say what is wrong: {stderr}"
    );
    // THE POINT: the link still names the core that was current when the
    // upgrade started, so nothing on this machine has moved.
    assert_eq!(
        fs::read_link(rig.link()).ok(),
        Some(first),
        "the command link moved despite the refusal"
    );
    // And no session was repointed at a core the operator never got — the
    // refused one included, which is the session an implementation that
    // repointed first and asked afterwards would have already rewritten.
    assert_eq!(
        meta_of(&refused),
        before,
        "the refused session was repointed anyway"
    );
    for helper in ae::shim::HELPERS {
        assert!(
            !present(&refused.join(helper.name)),
            "{} was rendered into a session the publish refused",
            helper.name
        );
    }
    assert!(
        present(&rig.versions().join("2026.9.9")),
        "the current version directory was removed"
    );
}

#[test]
fn a_refusal_late_in_the_sweep_leaves_the_sessions_before_it_untouched() {
    // THE PRECHECK. Sessions are swept in name order, so `aaa` is repointed
    // before `zzz` is even read — unless nothing is written until every session
    // has been asked. Without that pass, a publish that then rolls its version
    // directory back would leave `aaa` naming a core that is not there.
    let rig = Rig::new("precheck");
    let (code, stdout, stderr) = rig.install(&rig.bundle("2026.9.9"));
    assert_eq!(code, Some(0), "the first install failed: {stdout}{stderr}");
    let first = rig.versions().join("2026.9.9").join("ae-core");

    let early = rig.session("aaa", &first.to_string_lossy(), Some(ae::migrate::CURRENT));
    rig.session("zzz", "/nowhere/ae-core", None);
    let before = meta_of(&early);

    let (code, stdout, stderr) = rig.install(&rig.bundle("2026.9.10"));
    assert_eq!(code, Some(1), "the publish did not fail: {stdout}{stderr}");
    assert!(
        stderr.contains("zzz"),
        "the refusal names no session: {stderr}"
    );
    assert_eq!(
        meta_of(&early),
        before,
        "a session before the refusal was repointed anyway"
    );
}

// ─── the running session, against a real tmux ────────────────────────────

/// A scratch dir short enough to hold a socket path — `sun_path` is 104 bytes
/// on macOS and the usual temp dir eats most of it.
fn tmux_scratch(tag: &str) -> PathBuf {
    let dir = PathBuf::from(format!("/tmp/ae-mg-{tag}-{}", std::process::id()));
    let _ = remove(&dir);
    assert!(fs::create_dir_all(&dir).is_ok(), "a short scratch dir");
    dir
}

/// Kill the arm's server and remove its scratch, WHATEVER ended the arm.
struct Cleanup {
    socket: PathBuf,
    scratch: PathBuf,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        // NOT the panicking helper: this runs while a panic may already be
        // unwinding, and a second panic there takes the failure report with it.
        let out = self.scratch.join("cleanup-out");
        let err = self.scratch.join("cleanup-err");
        let invocation = Invocation::new("tmux")
            .arg("-S")
            .arg(&self.socket)
            .arg("kill-server");
        let _ = raw::run(&invocation, &self.scratch, &out, &err);
        let _ = remove(&self.scratch);
    }
}

/// The id of the session's one AGENT pane — the pane whose `@ae_agent` stamp is
/// a seat name rather than one of the two `_`-prefixed monitors.
fn agent_pane_of(socket: &Path, scratch: &Path, session: &str) -> String {
    let (_, listed) = tmux(
        socket,
        scratch,
        &[
            "list-panes",
            "-s",
            "-t",
            session,
            "-F",
            "#{@ae_agent} #{pane_id}",
        ],
    );
    listed
        .lines()
        .find(|line| !line.starts_with('_'))
        .and_then(|line| line.split_whitespace().next_back())
        .unwrap_or_else(|| panic!("no agent pane in {session}: {listed:?}"))
        .to_owned()
}

fn tmux(socket: &Path, scratch: &Path, words: &[&str]) -> (bool, String) {
    let mut args = vec!["-S".to_owned(), socket.display().to_string()];
    args.extend(words.iter().map(|word| (*word).to_owned()));
    run_tmux(&args, scratch)
}

/// A core that behaves like the two things a session runs it as: the watchdog
/// publishes a pidfile, everything else just stays alive.
const FAKE_CORE: &str = "#!/bin/sh\n\
     d=$(cd \"$(dirname \"$0\")\" && pwd)\n\
     case \"$(basename \"$0\")\" in\n\
     watchdog) printf '%s\\n' \"$$\" > \"$d/.watchdog.pid.staged\"\n\
     mv \"$d/.watchdog.pid.staged\" \"$d/.watchdog.pid\" ;;\n\
     esac\n\
     exec sleep 60\n";

/// A LIVE session on `socket`: a meta at the current version pinned to
/// `old_core`, the two helpers the start path runs, its tmux session, a
/// stand-in bridge, and a started watchdog.
fn plant_running(
    scratch: &Path,
    socket: &Path,
    root: &Path,
    session: &str,
    old_core: &Path,
) -> PathBuf {
    let dir = root.join("sessions").join(session);
    assert!(fs::create_dir_all(&dir).is_ok(), "a session dir");
    let meta = format!(
        "{}={}\nmode=local\nsession={session}\nwork_dir=/w\nae_version=0.0.1\n\
         ae_core={}\nae_core_version=0.0.1\n\
         tmux_server_kind=socket\ntmux_server={}\n\
         schema=2\nseat.main=lead\nprofile.main=cl\nagent_bin.main=claude\n",
        ae::migrate::KEY,
        ae::migrate::CURRENT,
        old_core.display(),
        socket.display()
    );
    assert!(fs::write(dir.join("meta"), meta).is_ok(), "a meta");
    // Linked at the OLD core, by hand — so the test asserts against the
    // product's own re-render rather than against itself.
    for helper in ["watchdog", "events-tail"] {
        assert!(
            std::os::unix::fs::symlink(old_core, dir.join(helper)).is_ok(),
            "a helper link"
        );
    }
    assert!(
        tmux(
            socket,
            scratch,
            &["new-session", "-d", "-s", session, "sleep", "60"]
        )
        .0,
        "the session the watchdog watches"
    );
    // The bridge is machine-global: a tmux session under its own name is
    // exactly what the liveness check looks for, and no real bridge is ever
    // spawned here.
    assert!(
        tmux(
            socket,
            scratch,
            &["new-session", "-d", "-s", "ae-telegram", "sleep", "60"]
        )
        .0,
        "a stand-in bridge"
    );
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = ae::watchdog_lifecycle::run(
        root,
        &["start".to_owned(), session.to_owned()],
        &mut out,
        &mut err,
    )
    .unwrap_or_else(|why| panic!("the entry writes to in-memory buffers: {why}"));
    assert_eq!(
        code,
        0,
        "the watchdog did not start: {}",
        String::from_utf8_lossy(&err)
    );
    dir
}

#[test]
fn a_running_sessions_daemons_are_restarted_on_the_new_core() {
    let scratch = tmux_scratch("run");
    if !tmux_present(&scratch) {
        let _ = remove(&scratch);
        panic!(
            "tmux is not runnable here, so the running-session half of the upgrade sweep cannot \
             be proven; install tmux or run this suite where one exists"
        );
    }
    let socket = scratch.join("s");
    let _cleanup = Cleanup {
        socket: socket.clone(),
        scratch: scratch.clone(),
    };
    let root = scratch.join("home");
    let session = "wdmig";

    let old_core = scratch.join("old-core");
    let new_core = scratch.join("new-core");
    write_exec(&old_core, FAKE_CORE);
    write_exec(&new_core, FAKE_CORE);

    let dir = plant_running(&scratch, &socket, &root, session, &old_core);
    let before = ae::watchdog_glue::read_pid(&dir).expect("a pidfile");
    let agent_pane = agent_pane_of(&socket, &scratch, session);

    let notes = ae::migrate::onto(&root, &new_core, "2026.9.9").expect("the sweep");

    // The meta and every helper now name the new core.
    let text = meta_of(&dir);
    assert_eq!(
        value_of(&text, "ae_core").as_deref(),
        Some(new_core.to_string_lossy().as_ref()),
        "{text}"
    );
    assert_eq!(
        fs::read_link(dir.join("watchdog")).ok(),
        Some(new_core.clone()),
        "the watchdog helper still names the old core"
    );

    // The watchdog is a NEW process, in a live stamped pane.
    let after = ae::watchdog_glue::read_pid(&dir).expect("a republished pidfile");
    assert_ne!(after, before, "the watchdog was not restarted");
    let (_, panes) = tmux(
        &socket,
        &scratch,
        &["list-panes", "-s", "-t", session, "-F", "#{@ae_agent}"],
    );
    assert!(
        panes.lines().any(|line| line == "_watchdog"),
        "no stamped watchdog pane after the restart: {panes:?}"
    );
    // Its command word is the new core, reached through the re-rendered helper.
    let (_, started) = tmux(
        &socket,
        &scratch,
        &[
            "list-panes",
            "-s",
            "-t",
            session,
            "-F",
            "#{@ae_agent} #{pane_start_command}",
        ],
    );
    assert!(
        started
            .lines()
            .any(|line| line.starts_with("_watchdog") && line.contains("/watchdog")),
        "the watchdog pane does not run the session helper: {started:?}"
    );

    // The bridge was replaced, on the new core.
    let (_, bridge) = tmux(
        &socket,
        &scratch,
        &[
            "list-panes",
            "-t",
            "ae-telegram",
            "-F",
            "#{pane_start_command}",
        ],
    );
    assert!(
        bridge.contains(&new_core.display().to_string()),
        "the bridge was not restarted on the new core: {bridge:?}"
    );
    assert!(
        notes.iter().any(|note| note.contains("watchdog")),
        "the sweep did not report the watchdog restart: {notes:?}"
    );
    assert!(
        notes.iter().any(|note| note.contains("telegram")),
        "the sweep did not report the bridge restart: {notes:?}"
    );

    // Agent panes are NEVER touched — the ORIGINAL pane, by id. A count would
    // be satisfied by a sweep that killed the agent and left the two monitor
    // panes standing, which is the failure this line exists to catch.
    let (_, after_panes) = tmux(
        &socket,
        &scratch,
        &["list-panes", "-s", "-t", session, "-F", "#{pane_id}"],
    );
    assert!(
        after_panes.lines().any(|line| line == agent_pane),
        "the agent pane {agent_pane} did not survive the sweep: {after_panes:?}"
    );
}
