//! Issue 282 residual: after a live rename the first turn of a resumed seat
//! can still run `_register-sid` through the pre-rename helper path and fail
//! with ENOENT until the next watchdog cycle repairs it.
//!
//! Ruling r0 accepts an old-name symlink alias, published and checked before
//! a LIVE `ae rename` reports success, fresh aliases only on live rename,
//! with rules 1-6 plus the stopped-path and launch pins. Oracle: the brief
//! invariant — from the moment `ae rename` returns success, a helper
//! invocation through either the old or the new FILESYSTEM path works for the
//! renamed session under its new name (`run_dispatch`/`Request::session_dir`
//! canonicalizes the helper directory, including underscore entries; the
//! public `ae @old` short form stays refused) — plus flat retargeting of
//! prior aliases across a later stopped rename, invisibility, teardown and
//! no-rollback. Private tmux socket, watchdog disabled/not started, scratch
//! state only.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "acceptance fixtures inspect their own scratch directories and tmux server"
)]
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::cli::{OwnedScratch, Runner, ae};
use super::phase2::run_tmux;

const OLD: &str = "old";
const MID: &str = "mid";
const NEW: &str = "new";
const ID: &str = "0199c0de-1111-4890-abcd-ef0123456789";
const CEILING: Duration = Duration::from_secs(90);

const AGENT: &str = r#"#!/usr/bin/perl
use strict; use warnings;
my ($out) = @ARGV;
system('stty raw -echo 2>/dev/null');
binmode(STDIN, ':raw'); binmode(STDOUT, ':raw'); $| = 1;
print "\e[?2004h";
my $border = "\xe2\x94\x80" x 400;
sub draw {
    my ($text) = @_; $text =~ s/[\r\n]/ /g;
    print "\e[H\e[2Jfake transcript\r\n\e[1m\xe2\x9d\xaf\e[0m\xc2\xa0$text\r\n$border\r\n  fake-model ~/x\r\n";
}
draw(''); my $buf = ''; my $pasting = 0;
while (sysread(STDIN, my $ch, 1)) {
    $buf .= $ch;
    if ($buf =~ s/\e\[200~\z//) { $pasting = 1; next; }
    if ($buf =~ s/\e\[201~\z//) { $pasting = 0; draw($buf); next; }
    if ($pasting && $ch eq "\r") { chop($buf); $buf .= "\n"; next; }
    if (!$pasting && ($ch eq "\r" || $ch eq "\n")) {
        $buf =~ s/[\r\n]\z//;
        open(my $log, '>>', $out) or die; binmode($log);
        print $log $buf, "\0"; close($log); $buf = ''; draw(''); next;
    }
    my $paste_prefix = $buf =~ /\e(?:\[|\[2|\[20|\[200)?\z/;
    draw($buf) if !$pasting && !$paste_prefix;
}
"#;

struct Rig {
    scratch: OwnedScratch,
    root: PathBuf,
    socket: PathBuf,
    config: PathBuf,
}

impl Rig {
    fn new(tag: &str) -> Self {
        let mut scratch = OwnedScratch::root("regsid", tag);
        let root = scratch.join("state");
        let socket = scratch.join("sock");
        scratch.add_tmux_server(socket.clone());
        fs::create_dir_all(root.join("sessions")).expect("scratch sessions");
        let agent = scratch.join("claude");
        fs::write(&agent, AGENT).expect("fixture agent");
        fs::set_permissions(&agent, fs::Permissions::from_mode(0o755)).expect("executable agent");
        let config = scratch.join("config");
        fs::write(
            &config,
            format!(
                "[profiles]\nfake = \"{}\"\n[roster]\nlead = fake\ncolead = fake\n[workspace]\nmain = lead\nworkers = colead\nlayout = lead-pair\nwatchdog = false\nchat = off\nquota = off\n",
                agent.display()
            ),
        )
        .expect("fixture config");
        Self {
            scratch,
            root,
            socket,
            config,
        }
    }

    fn dir(&self, name: &str) -> PathBuf {
        self.root.join("sessions").join(name)
    }

    fn tmux(&self, args: &[&str]) -> String {
        let mut words = vec!["-S".to_owned(), self.socket.display().to_string()];
        words.extend(args.iter().map(|arg| (*arg).to_owned()));
        let (ok, out) = run_tmux(&words, &self.scratch);
        assert!(
            ok,
            "private tmux {args:?}: {out}; stderr={}",
            fs::read_to_string(self.scratch.join("stderr")).unwrap_or_default()
        );
        out
    }

    fn create(&self, name: &str, id: &str) -> [String; 2] {
        let command = |seat: &str| {
            format!(
                "exec perl {} {}",
                self.scratch.join("claude").display(),
                self.scratch.join(format!("recv-{id}-{seat}")).display()
            )
        };
        self.tmux(&[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-x",
            "400",
            "-y",
            "40",
            "-s",
            name,
            "-n",
            "main",
            &command("lead"),
        ]);
        self.tmux(&[
            "split-window",
            "-d",
            "-t",
            &format!("{name}:main"),
            &command("colead"),
        ]);
        self.tmux(&["set-option", "-t", name, "@ae_session_uuid", id]);
        self.tmux(&["set-environment", "-t", name, "AE_SESSION", "1"]);
        let panes = self.tmux(&[
            "list-panes",
            "-s",
            "-t",
            &format!("{name}:main"),
            "-F",
            "#{pane_id}",
        ]);
        let mut panes = panes.lines();
        let main = panes.next().expect("main pane").to_owned();
        let peer = panes.next().expect("colead pane").to_owned();
        for (pane, slot, agent) in [(&main, "main", "lead"), (&peer, "worker.0", "colead")] {
            self.tmux(&["set-option", "-p", "-t", pane, "@ae_slot", slot]);
            self.tmux(&["set-option", "-p", "-t", pane, "@ae_agent", agent]);
        }
        let dir = self.dir(name);
        fs::create_dir_all(&dir).expect("session directory");
        fs::write(
            dir.join("meta"),
            format!(
                "schema=2\nsession={name}\nsession_id={id}\nsession_id_origin=session\nmode=local\nwork_dir={}\norigin={}\nconfig={}\nlayout=lead-pair\nwatchdog=false\nchat=off\nquota_awareness=false\nidle_nudge_secs=300\nmain_pane={main}\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nprofile.main=fake\nagent_bin.main=claude\nharness_session.main=pending\nlaunch_id.main=tok-main\npane.main={main}\nseat.worker.0=colead\nprofile.worker.0=fake\nagent_bin.worker.0=claude\nharness_session.worker.0=pending\nlaunch_id.worker.0=tok-peer\npane.worker.0={peer}\n",
                self.scratch.display(),
                self.scratch.display(),
                self.config.display(),
                self.socket.display()
            ),
        )
        .expect("session meta");
        for helper in ae::shim::HELPERS {
            std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ae"), dir.join(helper.name))
                .expect("core helper link");
        }
        let deadline = Instant::now() + CEILING;
        for pane in [&main, &peer] {
            while !self
                .tmux(&["capture-pane", "-p", "-t", pane])
                .contains("fake-model")
            {
                assert!(Instant::now() < deadline, "fixture composer did not start");
                std::thread::sleep(Duration::from_millis(25));
            }
        }
        [main, peer]
    }

    fn command(&self, pane: Option<&str>) -> Runner {
        let mut command = ae();
        command
            .env("HOME", &self.scratch)
            .env("AE_HOME", &self.root)
            .env("CONFIG_FILE", &self.config)
            .env("TMUX_TMPDIR", &self.scratch)
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &self.socket)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("AE_VERSION");
        if let Some(pane) = pane {
            command
                .env("TMUX", format!("{},0,0", self.socket.display()))
                .env("TMUX_PANE", pane);
        }
        command
    }

    fn run(&self, args: &[&str], pane: Option<&str>) -> (Option<i32>, String, String) {
        let out = self
            .command(pane)
            .args(args)
            .output()
            .expect("fixture ae process");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    // Invoke the helper FILE through its symlink path, not the `ae @..` short
    // form: this is the alias surface, `<sessions>/<name>/<helper>`.
    fn run_helper_path(
        &self,
        session: &str,
        helper_name: &str,
        args: &[&str],
        pane: Option<&str>,
    ) -> (Option<i32>, String, String) {
        let helper_path = self.dir(session).join(helper_name);
        let mut command = super::cli::helper(&helper_path);
        command
            .env("HOME", &self.scratch)
            .env("AE_HOME", &self.root)
            .env("CONFIG_FILE", &self.config)
            .env("TMUX_TMPDIR", &self.scratch)
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &self.socket)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("AE_VERSION");
        if let Some(pane) = pane {
            command
                .env("TMUX", format!("{},0,0", self.socket.display()))
                .env("TMUX_PANE", pane);
        } else {
            command.env_remove("TMUX").env_remove("TMUX_PANE");
        }
        let out = command.args(args).output().expect("fixture helper process");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn rename(&self, old: &str, new: &str) -> (Option<i32>, String, String) {
        self.run(&["rename", old, new], None)
    }

    fn stop_tmux(&self, name: &str) {
        self.tmux(&["kill-session", "-t", name]);
    }

    // A launch needs a shell harness that stays alive without the perl
    // composer's receipt argument; the perl agent stays for the ask tests.
    fn use_sh(&self) {
        fs::write(
            &self.config,
            "[profiles]\nfake = \"sh\"\n[roster]\nlead = fake\ncolead = fake\n[workspace]\nmain = lead\nworkers = colead\nlayout = lead-pair\nwatchdog = false\nchat = off\nquota = off\n",
        )
        .expect("sh fixture config");
    }

    fn launch(&self, name: &str) -> (Option<i32>, String, String) {
        let out = self
            .command(None)
            .current_dir(&self.scratch)
            .args(["--local", name, "--no-attach"])
            .output()
            .expect("fixture launch process");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn readlink(&self, name: &str) -> PathBuf {
        let link = self.dir(name);
        let meta = fs::symlink_metadata(&link).expect("alias entry exists");
        assert!(
            meta.file_type().is_symlink(),
            "ruling 1-4: {name} is a symlink alias, not a real slot ({:?})",
            meta.file_type()
        );
        fs::read_link(&link).expect("readlink of alias")
    }

    fn session_of(&self, name: &str) -> String {
        let bytes = fs::read(self.dir(name).join("meta")).expect("meta bytes");
        ae::meta::Meta::parse(&String::from_utf8_lossy(&bytes))
            .session_id()
            .unwrap_or_default()
            .to_owned()
    }
}

fn assert_alias_points_at(rig: &Rig, alias: &str, target: &str) {
    let want = rig.dir(target);
    let got = rig.readlink(alias);
    assert_eq!(
        got,
        want,
        "ruling 1: {alias} -> {} is flat to the current directory, not chained",
        want.display()
    );
    // Direct-child target in the same sessions root, never an escape.
    assert_eq!(
        got.parent(),
        Some(rig.root.join("sessions").as_path()),
        "ruling 3: alias target is a direct child of the sessions root"
    );
}

fn at_old_refused(rig: &Rig, pane: &str) {
    let at = format!("@{OLD}");
    let (code, out, err) = rig.run(&[at.as_str(), "requests", "all"], Some(pane));
    assert_eq!(code, Some(1), "short form stays refused: {out}{err}");
    assert!(
        err.contains("symlink") || err.contains("plain directory"),
        "short-form alias refusal names the symlink: {out}{err}"
    );
}

#[test]
fn old_helper_path_works_after_live_rename_and_records_the_new_name() {
    let rig = Rig::new("old-path");
    let panes = rig.create(OLD, ID);
    let renamed = rig.rename(OLD, NEW);
    assert_eq!(renamed.0, Some(0), "live rename: {renamed:?}");
    // The alias itself, before any helper through it. RED on main: no alias.
    assert_alias_points_at(&rig, OLD, NEW);
    // A helper through the old FILE path, with the pane watching the renamed
    // tmux session. `ask` opens a tracked request; its durable `actor_session`
    // is the canonical proof that the call acted under the NEW name.
    let (code, out, err) =
        rig.run_helper_path(OLD, "ask", &["colead", "canonical proof"], Some(&panes[0]));
    assert_eq!(code, Some(0), "old-path ask: {out}{err}");
    let id = out.trim().to_owned();
    assert!(
        ae::tracked::is_request_id(&id),
        "request id: {id:?} {out}{err}"
    );
    let events = fs::read_to_string(rig.dir(NEW).join("events.jsonl")).expect("renamed events");
    let line = events
        .lines()
        .find(|line| line.contains(&id))
        .unwrap_or_else(|| panic!("request {id} landed in the renamed journal: {events}"));
    assert!(
        line.contains(&format!(r#""actor_session":"{NEW}""#))
            && line.contains(&format!(r#""target_session":"{NEW}""#)),
        "ruling 2: call through the old path acts under {NEW}: {line}"
    );
    // The new path still works for the same session, and the reply closes it.
    let reply = rig.run_helper_path(NEW, "reply", &[&id, "answer"], Some(&panes[1]));
    assert_eq!(reply.0, Some(0), "new-path reply: {reply:?}");
    // The incident helper itself: a real codex registration through the old
    // path lands in the renamed directory under the new name. The fixture
    // rewires the seat to Codex with a developer-proven rollout, then the
    // call must exit 0 and record the rollout id in NEW meta.
    let token = "regsid-acceptance-282";
    let codex_id = "22222222-2222-4222-8222-222222222222";
    let codex_home = rig.scratch.join("codex-account");
    fs::create_dir_all(&codex_home).expect("codex home");
    let stamp = ae::time::Timestamp::now().to_string();
    let day = stamp.get(..10).expect("timestamp day").replace('-', "/");
    let dir_day = codex_home.join("sessions").join(&day);
    fs::create_dir_all(&dir_day).expect("rollout day");
    let rollout = dir_day.join(format!("rollout-{codex_id}.jsonl"));
    fs::write(
        &rollout,
        format!(
            "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{codex_id}\",\"cwd\":\"{}\"}}}}\n{{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"developer\",\"content\":[{{\"type\":\"input_text\",\"text\":\"AE_CODEX_LAUNCH_ID={token}\"}}]}}}}\n",
            codex_home.display()
        ),
    )
    .expect("rollout");
    let new_dir = rig.dir(NEW);
    for (key, value) in [
        ("agent_bin.main", "codex".to_owned()),
        ("harness_session.main", "pending".to_owned()),
        ("launch_id.main", token.to_owned()),
        ("capture_floor.main", "0".to_owned()),
        ("config_home.main", codex_home.display().to_string()),
        ("work_dir.main", rig.scratch.display().to_string()),
    ] {
        ae::meta::rewrite(&new_dir, key, Some(value.as_str())).expect("codex meta");
    }
    let reg = rig.run_helper_path(OLD, "_register-sid", &["main"], None);
    assert_eq!(reg.0, Some(0), "old-path registration succeeds: {reg:?}");
    assert!(
        reg.1.contains("Registered session id"),
        "registration line: {reg:?}"
    );
    let after = fs::read_to_string(new_dir.join("meta")).expect("meta after register");
    assert!(
        after
            .lines()
            .any(|line| line == format!("harness_session.main={codex_id}")),
        "ruling 2: registration is recorded in the renamed session: {after}"
    );
    assert_eq!(
        rig.session_of(NEW),
        ID,
        "recorded session id is the renamed one"
    );
    let meta = fs::read_to_string(rig.dir(NEW).join("meta")).expect("meta");
    assert!(
        meta.lines().any(|line| line == format!("session={NEW}")),
        "ruling 2: meta still names {NEW} after an old-path call: {meta}"
    );
    // The filesystem path and the marked short form are different spells:
    // `ae @old` stays refused while the old filesystem path resolves.
    at_old_refused(&rig, &panes[0]);
}

#[test]
fn alias_stays_flat_across_two_renames() {
    let rig = Rig::new("flat");
    let _ = rig.create(OLD, ID);
    assert_eq!(rig.rename(OLD, MID).0, Some(0), "first rename");
    assert_alias_points_at(&rig, OLD, MID);
    assert_eq!(rig.rename(MID, NEW).0, Some(0), "second rename");
    // Flat, never chained: both aliases land directly on the current dir.
    assert_alias_points_at(&rig, OLD, NEW);
    assert_alias_points_at(&rig, MID, NEW);
    let old_target = rig.readlink(OLD);
    let mid_target = rig.readlink(MID);
    assert_eq!(
        old_target, mid_target,
        "second rename retargeted every alias"
    );
    assert!(!old_target.ends_with(MID), "never A->B->C: {old_target:?}");
}

#[test]
fn rename_back_reclaims_its_own_alias() {
    let rig = Rig::new("back");
    let _ = rig.create(OLD, ID);
    assert_eq!(rig.rename(OLD, NEW).0, Some(0), "first rename");
    assert_alias_points_at(&rig, OLD, NEW);
    // B -> A while A is an alias of B: the alias is reclaimed by lstat, only
    // because it is a symlink to a direct child of the same root.
    assert_eq!(rig.rename(NEW, OLD).0, Some(0), "rename back");
    let meta = fs::read_to_string(rig.dir(OLD).join("meta")).expect("meta");
    assert!(
        meta.lines().any(|line| line == format!("session={OLD}")),
        "rename-back carries the session: {meta}"
    );
    let new_meta = fs::symlink_metadata(rig.dir(NEW));
    assert!(
        new_meta.is_ok_and(|m| m.file_type().is_symlink()),
        "B -> A leaves B as an alias, not a real slot"
    );
    assert_alias_points_at(&rig, NEW, OLD);
}

#[test]
fn teardown_of_the_new_session_removes_its_aliases() {
    let rig = Rig::new("teardown");
    let _ = rig.create(OLD, ID);
    assert_eq!(rig.rename(OLD, NEW).0, Some(0), "rename");
    assert_alias_points_at(&rig, OLD, NEW);
    let (code, out, err) = rig.run(&["_end", "-f", NEW], None);
    assert_eq!(code, Some(0), "end with cleanup: {out}{err}");
    assert!(
        fs::symlink_metadata(rig.dir(OLD)).is_err(),
        "ruling 4: teardown removes every alias pointing at the removed directory"
    );
    assert!(
        fs::symlink_metadata(rig.dir(NEW)).is_err(),
        "the target itself is gone"
    );
}

#[test]
fn alias_never_appears_in_listing() {
    let rig = Rig::new("invisible");
    let _ = rig.create(OLD, ID);
    assert_eq!(rig.rename(OLD, NEW).0, Some(0), "rename");
    assert_alias_points_at(&rig, OLD, NEW);
    // Inventory already skips symlinks; pin it through the public list whose
    // cursor comes from the same durable walk.
    let (code, out, err) = rig.run(&["list", "--all"], None);
    assert_eq!(code, Some(0), "list --all: {out}{err}");
    assert!(
        out.contains(NEW),
        "new session is listed: {out}{err} / {err}"
    );
    assert!(
        !out.lines()
            .any(|line| line.split_whitespace().next() == Some(OLD)),
        "ruling 5: alias {OLD} never lists as a session: {out}"
    );
    // The helper file it points at does not change that verdict.
    let (code, out, err) = rig.run_helper_path(OLD, "requests", &["all"], None);
    assert_eq!(code, Some(0), "old-path requests: {out}{err}");
    let (code, out, err) = rig.run(&["list", "--all"], None);
    assert_eq!(code, Some(0), "list after old-path call: {out}{err}");
    assert!(
        !out.lines()
            .any(|line| line.split_whitespace().next() == Some(OLD)),
        "an old-path call does not promote the alias: {out}"
    );
}

#[test]
fn foreign_entry_at_prior_alias_name_is_never_deleted() {
    let rig = Rig::new("race");
    let _ = rig.create(OLD, ID);
    assert_eq!(rig.rename(OLD, MID).0, Some(0), "first rename");
    assert_alias_points_at(&rig, OLD, MID);
    // Scan-based retargeting cannot know a pre-existing foreign entry was
    // ever an alias: the oracle is preservation, not a fresh-alias race.
    // A deterministic E2E window for fresh-alias EEXIST is infeasible without
    // a fault seam, so this pins what the scan proves — the rename succeeds,
    // the foreign entry is untouched, and the fresh MID alias still lands.
    let old_path = rig.dir(OLD);
    let _ = fs::remove_file(&old_path);
    fs::write(&old_path, b"raced").expect("raced entry");
    let (code, out, err) = rig.rename(MID, NEW);
    assert_eq!(
        code,
        Some(0),
        "ruling 6: a rename never rolls back for a foreign entry: {out}{err}"
    );
    let meta = fs::read_to_string(rig.dir(NEW).join("meta")).expect("meta after rename");
    assert!(
        meta.lines().any(|line| line == format!("session={NEW}")),
        "the rename itself landed: {meta}"
    );
    assert_eq!(
        fs::read(&old_path).expect("raced entry survives"),
        b"raced",
        "a non-alias entry is never deleted by a rename"
    );
    assert!(
        !fs::symlink_metadata(&old_path).is_ok_and(|m| m.file_type().is_symlink()),
        "the foreign file was not turned into an alias"
    );
    assert_alias_points_at(&rig, MID, NEW);
}

#[test]
fn stopped_rename_creates_no_fresh_alias_but_surviving_aliases_retarget_flat() {
    let rig = Rig::new("stopped");
    let _ = rig.create(OLD, ID);
    // A fresh stopped rename leaves the old name absent: aliases only on live.
    rig.stop_tmux(OLD);
    let renamed = rig.rename(OLD, NEW);
    assert_eq!(renamed.0, Some(0), "stopped rename: {renamed:?}");
    assert!(
        fs::symlink_metadata(rig.dir(OLD)).is_err(),
        "fresh aliases only on live rename: OLD is absent after a stopped rename"
    );
    assert!(
        rig.dir(NEW).is_dir()
            && !fs::symlink_metadata(rig.dir(NEW)).is_ok_and(|m| m.file_type().is_symlink()),
        "stopped rename lands a real directory"
    );
}

#[test]
fn prior_live_alias_survives_a_later_stopped_rename_flat() {
    let rig = Rig::new("stopped-retarget");
    let _ = rig.create(OLD, ID);
    assert_eq!(rig.rename(OLD, MID).0, Some(0), "live rename");
    assert_alias_points_at(&rig, OLD, MID);
    rig.stop_tmux(MID);
    // The second rename takes the stopped path: no fresh alias for MID, but
    // the prior live alias retargets flat onto the new directory.
    let renamed = rig.rename(MID, NEW);
    assert_eq!(renamed.0, Some(0), "stopped rename: {renamed:?}");
    assert_alias_points_at(&rig, OLD, NEW);
    assert!(
        fs::symlink_metadata(rig.dir(MID)).is_err(),
        "no fresh alias on the stopped path: MID is absent"
    );
}

#[test]
fn launch_reclaims_a_recognized_alias_and_refuses_an_external_link() {
    let rig = Rig::new("launch-alias");
    let _ = rig.create(OLD, ID);
    rig.use_sh();
    assert_eq!(rig.rename(OLD, NEW).0, Some(0), "live rename");
    assert_alias_points_at(&rig, OLD, NEW);
    // A recognized alias (symlink to a direct child) is absent to a launch:
    // it is reclaimed under the lifecycle lock and a fresh session takes the
    // name, while the renamed session keeps its own directory untouched.
    let (code, out, err) = rig.launch(OLD);
    assert_eq!(
        code,
        Some(0),
        "launch reclaims a recognized alias as absent: {out}{err}"
    );
    let meta = fs::read_to_string(rig.dir(OLD).join("meta")).expect("new meta");
    assert!(
        meta.lines().any(|line| line == format!("session={OLD}")),
        "the reclaimed name is a fresh session: {meta}"
    );
    assert!(
        !fs::symlink_metadata(rig.dir(OLD)).is_ok_and(|m| m.file_type().is_symlink()),
        "the alias was reclaimed, not followed"
    );
    let kept = fs::read_to_string(rig.dir(NEW).join("meta")).expect("renamed meta");
    assert!(
        kept.lines().any(|line| line == format!("session={NEW}")),
        "the renamed session was not resumed by the launch: {kept}"
    );
}

#[test]
fn launch_refuses_an_external_link_at_the_wanted_name() {
    let rig = Rig::new("launch-external");
    let _ = rig.create(OLD, ID);
    rig.use_sh();
    assert_eq!(rig.rename(OLD, NEW).0, Some(0), "live rename");
    assert_alias_points_at(&rig, OLD, NEW);
    // Replace the alias with an external symlink (target outside the sessions
    // root): a launch wanting that name refuses and leaves it untouched.
    let external = rig.scratch.join("external-target");
    fs::create_dir_all(&external).expect("external dir");
    fs::write(external.join("keep"), b"keep").expect("external keep");
    let old_path = rig.dir(OLD);
    let _ = fs::remove_file(&old_path);
    std::os::unix::fs::symlink(&external, &old_path).expect("external alias");
    let (code, out, err) = rig.launch(OLD);
    assert_eq!(code, Some(1), "external link refuses: {out}{err}");
    assert!(
        out.contains("symlink") || err.contains("symlink") || err.contains("plain directory"),
        "refusal names the symlink: {out}{err}"
    );
    assert_eq!(
        fs::read_link(&old_path).expect("still a symlink"),
        external,
        "a non-recognized link is never followed or reclaimed"
    );
    assert_eq!(
        fs::read(external.join("keep")).expect("external intact"),
        b"keep",
        "nothing was written through the external link"
    );
}

#[test]
fn internal_teardown_through_an_alias_is_refused_and_new_still_works() {
    let rig = Rig::new("teardown-alias");
    let _panes = rig.create(OLD, ID);
    assert_eq!(rig.rename(OLD, NEW).0, Some(0), "live rename");
    assert_alias_points_at(&rig, OLD, NEW);
    // The internal authority is not a shim helper, so dispatch must not
    // normalize the alias for it: the symlink refusal stands through the
    // alias itself, and neither the new directory nor the alias is removed.
    let old_dir = rig.dir(OLD).display().to_string();
    let refused = rig.run(&["_end-local-teardown", old_dir.as_str()], None);
    assert_eq!(refused.0, Some(1), "alias teardown refuses: {refused:?}");
    assert!(
        rig.dir(NEW).is_dir()
            && !fs::symlink_metadata(rig.dir(NEW)).is_ok_and(|m| m.file_type().is_symlink()),
        "the canonical directory survives the refused teardown"
    );
    assert_alias_points_at(&rig, OLD, NEW);
    // The canonical new helper still works after the refusal.
    let ok = rig.run_helper_path(NEW, "requests", &["all"], None);
    assert_eq!(ok.0, Some(0), "canonical helper after refusal: {ok:?}");
}
