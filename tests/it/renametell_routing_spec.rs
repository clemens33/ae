//! Frozen R1/R2 acceptance: real helpers, private tmux, fixture-only agents.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixture setup reads scratch state and fails on setup errors"
)]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::cli::{OwnedScratch, Runner, ae, helper};
use super::phase2::run_tmux;

pub(super) const A: &str = "0199c0de-1111-4890-abcd-ef0123456789";
pub(super) const B: &str = "0199c0de-2222-4890-abcd-ef0123456789";
const C: &str = "0199c0de-3333-4890-abcd-ef0123456789";

// Same bracketed-paste/submit behavior as the suite's existing fake Claude.
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
    draw($buf);
}
"#;

pub(super) struct Rig {
    pub(super) scratch: OwnedScratch,
    pub(super) root: PathBuf,
    pub(super) socket: PathBuf,
    pub(super) config: PathBuf,
}

impl Rig {
    pub(super) fn new(tag: &str) -> Self {
        let mut scratch = OwnedScratch::root("rt", tag);
        let root = scratch.join("state");
        let socket = scratch.join("sock");
        scratch.add_tmux_server(socket.clone());
        fs::create_dir_all(root.join("sessions")).expect("scratch sessions");
        let agent = scratch.join("claude");
        fs::write(&agent, AGENT).expect("fixture agent");
        fs::set_permissions(&agent, fs::Permissions::from_mode(0o755)).expect("executable agent");
        let config = scratch.join("config");
        fs::write(&config, format!("[profiles]\nfake = \"{}\"\n[roster]\nlead = fake\ncolead = fake\n[workspace]\nmain = lead\nworkers = colead\nlayout = lead-pair\nwatchdog = false\nchat = off\nquota = off\n", agent.display())).expect("fixture config");
        Self {
            scratch,
            root,
            socket,
            config,
        }
    }

    pub(super) fn dir(&self, name: &str) -> PathBuf {
        self.root.join("sessions").join(name)
    }

    pub(super) fn tmux(&self, args: &[&str]) -> String {
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

    pub(super) fn create(&self, name: &str, id: &str) -> [String; 2] {
        let command = |seat: &str| {
            format!(
                "exec perl {} {}",
                self.scratch.join("claude").display(),
                self.receipt(id, seat).display()
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
        fs::write(dir.join("meta"), format!("schema=2\nsession={name}\nsession_id={id}\nsession_id_origin=session\nmode=local\nwork_dir={}\norigin={}\nconfig={}\nlayout=lead-pair\nwatchdog=false\nchat=off\nquota_awareness=false\nidle_nudge_secs=300\nmain_pane={main}\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nprofile.main=fake\nagent_bin.main=claude\nharness_session.main=pending\nlaunch_id.main=tok-main\npane.main={main}\nseat.worker.0=colead\nprofile.worker.0=fake\nagent_bin.worker.0=claude\nharness_session.worker.0=pending\nlaunch_id.worker.0=tok-peer\npane.worker.0={peer}\n", self.scratch.display(), self.scratch.display(), self.config.display(), self.socket.display())).expect("session meta");
        for helper in ae::shim::HELPERS {
            std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ae"), dir.join(helper.name))
                .expect("core helper link");
        }
        let deadline = Instant::now() + Duration::from_secs(5);
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

    pub(super) fn command(&self, pane: Option<&str>) -> Runner {
        let mut command = ae();
        command
            .env("HOME", &self.scratch)
            .env("AE_HOME", &self.root)
            .env("CONFIG_FILE", &self.config)
            .env("TMUX_TMPDIR", &self.scratch)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE");
        if let Some(pane) = pane {
            command
                .env("TMUX", format!("{},0,0", self.socket.display()))
                .env("TMUX_PANE", pane);
        }
        command
    }

    pub(super) fn run(&self, args: &[&str], pane: Option<&str>) -> (Option<i32>, String, String) {
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

    pub(super) fn helper(
        &self,
        name: &str,
        verb: &str,
        args: &[&str],
        pane: Option<&str>,
    ) -> (Option<i32>, String, String) {
        let marker = format!("@{name}");
        let mut words = vec![marker.as_str(), verb];
        words.extend_from_slice(args);
        self.run(&words, pane)
    }

    pub(super) fn receipt(&self, name: &str, seat: &str) -> PathBuf {
        self.scratch.join(format!("recv-{name}-{seat}"))
    }

    pub(super) fn received(&self, name: &str, seat: &str) -> Vec<String> {
        fs::read_to_string(self.receipt(name, seat))
            .unwrap_or_default()
            .split('\0')
            .filter(|text| !text.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    }

    pub(super) fn append(&self, name: &str, line: &str) {
        ae::events::Event::parse_line(line).expect("synthetic fixture event is valid JSON");
        ae::store::open(&self.dir(name))
            .append_event(&format!("{}\n", line.trim_end_matches('\n')))
            .expect("fixture event append");
    }

    fn ask(&self, name: &str, pane: &str, verb: &str, target: &str) -> String {
        let (code, out, err) = self.helper(
            name,
            verb,
            &["--cross-session", target, "question"],
            Some(pane),
        );
        assert_eq!(code, Some(0), "request setup: {out}{err}");
        out.trim().to_owned()
    }

    fn rename(&self, old: &str, new: &str) {
        let result = self.run(&["rename", old, new], None);
        assert_eq!(result.0, Some(0), "live rename setup: {result:?}");
    }
}

fn opening(from: &str, to: &str, extra: &str) -> String {
    format!(
        r#"{{"ts":"2026-10-10T10:00:00Z","actor":"lead","action":"ask","target":"colead","ref":"spec-request","actor_slot":"main","actor_session":"{from}","target_slot":"worker.0","target_session":"{to}","summary":"question"{extra}}}"#
    )
}

fn row(view: &str, id: &str, status: &str) -> bool {
    view.lines()
        .any(|line| line.starts_with(status) && line.contains(id))
}

#[test]
fn request_writers_stamp_both_routing_ids_for_ask_and_review() {
    let rig = Rig::new("writer");
    let panes = rig.create("old", A);
    for verb in ["ask", "review"] {
        let id = rig.ask("old", &panes[0], verb, "colead");
        let bytes = fs::read_to_string(rig.dir("old").join("events.jsonl")).expect("writer events");
        let line = bytes
            .lines()
            .find(|line| line.contains(&id))
            .expect("request event");
        assert!(
            line.contains(&format!(r#""actor_session_id":"{A}""#))
                && line.contains(&format!(r#""target_session_id":"{A}""#)),
            "R1 writer omitted stable endpoint IDs: {line}"
        );
    }
}

#[test]
fn mine_and_inbox_keep_a_pending_request_after_live_rename() {
    let rig = Rig::new("views");
    let panes = rig.create("old", A);
    let id = rig.ask("old", &panes[0], "ask", "colead");
    rig.rename("old", "new");
    for (pane, mode) in [(&panes[0], "mine"), (&panes[1], "inbox")] {
        let result = rig.helper("new", "requests", &[mode], Some(pane));
        assert_eq!(result.0, Some(0), "{result:?}");
        assert!(
            row(&result.1, &id, "pending"),
            "R1 {mode} silently lost {id}: {result:?}"
        );
    }
}

#[test]
fn ask_and_review_reply_through_the_new_helper_after_live_rename() {
    for verb in ["ask", "review"] {
        let rig = Rig::new(verb);
        let panes = rig.create("old", A);
        let id = rig.ask("old", &panes[0], verb, "colead");
        rig.rename("old", "new");
        let reply = rig.helper("new", "reply", &[&id, "answer"], Some(&panes[1]));
        let all = rig.helper("new", "requests", &["all"], None);
        assert_eq!(reply.0, Some(0), "R1 {verb} reply stranded: {reply:?}");
        assert!(
            rig.received(A, "lead")
                .iter()
                .any(|text| text.contains(&format!("[{id}] answer"))),
            "R1 answer missed the original asker"
        );
        assert!(
            row(&all.1, &id, "replied"),
            "R1 reply failed to close request: {all:?}"
        );
    }
}

#[test]
fn both_retire_endpoints_close_after_rename() {
    for slot in ["main", "worker.0"] {
        let rig = Rig::new(slot);
        let panes = rig.create("old", A);
        let id = rig.ask("old", &panes[0], "ask", "colead");
        rig.rename("old", "new");
        rig.append("new", &format!(r#"{{"ts":"2026-10-10T10:01:00Z","actor":"lead","action":"retire","target":"retired-seat","target_slot":"{slot}"}}"#));
        let all = rig.helper("new", "requests", &["all"], None);
        assert!(
            row(&all.1, &id, "retired"),
            "R1 retired {slot} did not close renamed request: {all:?}"
        );
        assert!(
            ae::session::SessionRead::open(&rig.dir("new"))
                .expect("request reader")
                .pending
                .is_empty(),
            "R1 own-work reader disagrees with requests"
        );
    }
}

#[test]
fn list_still_reports_the_askers_own_work_after_rename() {
    let rig = Rig::new("work");
    let panes = rig.create("old", A);
    assert_eq!(
        rig.helper("old", "state", &["working"], Some(&panes[0])).0,
        Some(0)
    );
    rig.ask("old", &panes[0], "ask", "colead");
    assert!(
        rig.run(&["list", "--all"], None)
            .1
            .contains("waiting on 1 request"),
        "fixture must show own-work before rename"
    );
    rig.rename("old", "new");
    let listed = rig.run(&["list", "--all"], None);
    assert_eq!(listed.0, Some(0), "{listed:?}");
    assert!(
        listed
            .1
            .lines()
            .any(|line| line.contains("lead") && line.contains("waiting on 1 request")),
        "R1 own-work deferral disappeared: {listed:?}"
    );
}

#[test]
fn legacy_pending_request_refuses_live_and_stopped_rename_without_mutation() {
    for stopped in [false, true] {
        let rig = Rig::new(if stopped {
            "legacy-stop"
        } else {
            "legacy-live"
        });
        rig.create("old", A);
        rig.append("old", &opening("old", "old", ""));
        if stopped {
            rig.tmux(&["kill-session", "-t", "old"]);
        }
        let renamed = rig.run(&["rename", "old", "new"], None);
        assert_eq!(
            renamed.0,
            Some(1),
            "R1 legacy rename stranded a request: {renamed:?}"
        );
        assert!(
            renamed.2.contains("spec-request")
                && (renamed.2.contains("reply") || renamed.2.contains("retire")),
            "R1 legacy refusal omitted ID/remedy: {renamed:?}"
        );
        assert!(
            rig.dir("old").is_dir() && !rig.dir("new").exists(),
            "R1 refusal mutated state paths"
        );
        if !stopped {
            assert!(
                rig.tmux(&["list-sessions", "-F", "#{session_name}"])
                    .lines()
                    .any(|name| name == "old"),
                "R1 refusal changed live name"
            );
        }
    }
}

#[test]
fn peer_only_legacy_rows_block_rename_in_both_directions() {
    for (from, to) in [("old", "peer"), ("peer", "old")] {
        let rig = Rig::new(from);
        rig.create("old", A);
        rig.create("peer", B);
        rig.append("peer", &opening(from, to, ""));
        let renamed = rig.run(&["rename", "old", "new"], None);
        assert_eq!(
            renamed.0,
            Some(1),
            "R1 peer-only {from}->{to} legacy row silently stranded: {renamed:?}"
        );
        assert!(
            renamed.2.contains("spec-request"),
            "R1 peer refusal omitted request ID: {renamed:?}"
        );
        assert!(rig.dir("old").is_dir() && !rig.dir("new").exists());
    }
}

#[test]
fn stopped_rename_accepts_pending_request_with_stable_endpoint_ids() {
    let rig = Rig::new("stopped-id");
    rig.create("old", A);
    rig.append(
        "old",
        &opening(
            "old",
            "old",
            &format!(r#", "actor_session_id":"{A}","target_session_id":"{A}""#),
        ),
    );
    rig.tmux(&["kill-session", "-t", "old"]);
    let renamed = rig.run(&["rename", "old", "new"], None);
    assert_eq!(
        renamed.0,
        Some(0),
        "R1 stable pending request unnecessarily blocks stopped rename: {renamed:?}"
    );
    assert!(!rig.dir("old").exists() && rig.dir("new").is_dir());
    assert!(row(
        &rig.helper("new", "requests", &["all"], None).1,
        "spec-request",
        "pending"
    ));
}

#[test]
fn external_slotless_actor_does_not_block_a_stable_target() {
    let rig = Rig::new("external");
    rig.create("old", A);
    rig.append("old", &format!(r#"{{"ts":"2026-10-10T10:00:00Z","actor":"ae:seats:checkpoint","action":"ask","target":"colead","ref":"spec-external","actor_session":"old","target_slot":"worker.0","target_session":"old","target_session_id":"{A}"}}"#));
    rig.rename("old", "new");
    assert!(row(
        &rig.helper("new", "requests", &["all"], None).1,
        "spec-external",
        "pending"
    ));
}

#[test]
fn cross_session_reply_survives_either_endpoint_rename_and_old_name_reuse() {
    for renamed_asker in [false, true] {
        let rig = Rig::new(if renamed_asker { "from" } else { "to" });
        let old = rig.create("old", A);
        let peer = rig.create("peer", B);
        let (from, pane, target) = if renamed_asker {
            ("old", &old[0], "@peer:colead")
        } else {
            ("peer", &peer[0], "@old:colead")
        };
        let id = rig.ask(from, pane, "ask", target);
        rig.rename("old", "new");
        rig.create("old", C);
        let (reply_dir, replier) = if renamed_asker {
            ("peer", &peer[1])
        } else {
            ("new", &old[1])
        };
        let reply = rig.helper(reply_dir, "reply", &[&id, "cross-answer"], Some(replier));
        assert_eq!(
            reply.0,
            Some(0),
            "R2 renamed endpoint refused reply: {reply:?}"
        );
        for name in ["new", "peer"] {
            assert!(
                row(
                    &rig.helper(name, "requests", &["all"], None).1,
                    &id,
                    "replied"
                ),
                "R2 mirror at {name} did not close"
            );
        }
        let recipient = if renamed_asker { A } else { B };
        assert!(
            rig.received(recipient, "lead")
                .iter()
                .any(|text| text.contains(&format!("[{id}] cross-answer"))),
            "R2 reply missed original asker"
        );
        for seat in ["lead", "colead"] {
            assert!(
                !rig.received(C, seat)
                    .iter()
                    .any(|text| text.contains("cross-answer")),
                "R2 delivered to reused old session"
            );
        }
    }
}

#[test]
fn valid_asker_id_missing_or_ambiguous_never_routes_by_the_old_name() {
    for ambiguous in [false, true] {
        let rig = Rig::new(if ambiguous { "ambiguous" } else { "missing" });
        let old = rig.create("old", A);
        let peer = rig.create("peer", B);
        let id = rig.ask("old", &old[0], "ask", "@peer:colead");
        rig.rename("old", "new");
        if ambiguous {
            rig.create("duplicate", A);
        } else {
            fs::remove_dir_all(rig.dir("new")).expect("fixture ended state");
            rig.tmux(&["kill-session", "-t", "new"]);
        }
        rig.create("old", C);
        let before = rig.received(C, "lead").len();
        let reply = rig.helper("peer", "reply", &[&id, "wrong-place"], Some(&peer[1]));
        assert_eq!(
            reply.0,
            Some(1),
            "R2 present ID downgraded to old name: {reply:?}"
        );
        assert!(
            reply.2.contains(A),
            "R2 ID resolution failure omitted pinned ID: {reply:?}"
        );
        assert_eq!(
            rig.received(C, "lead").len(),
            before,
            "R2 wrong session received reply"
        );
    }
}

#[test]
fn stale_helper_path_is_gone_and_reused_path_cannot_answer_old_request() {
    let rig = Rig::new("footer");
    let panes = rig.create("old", A);
    let id = rig.ask("old", &panes[0], "ask", "colead");
    let stale = rig.dir("old").join("reply");
    rig.rename("old", "new");
    let missing = helper(&stale).args([&id, "answer"]).output();
    assert!(missing.is_err(), "R2 old helper alias was retained");
    let replacement = rig.create("old", C);
    let refused = rig.helper("old", "reply", &[&id, "answer"], Some(&replacement[1]));
    assert_eq!(
        refused.0,
        Some(1),
        "R2 reused helper accepted old request: {refused:?}"
    );
    assert!(
        refused.2.contains(&id) && refused.2.contains("not found"),
        "R2 reused helper refusal omitted remedy evidence: {refused:?}"
    );
}

#[test]
fn event_identity_preserves_bad_id_presence() {
    for value in [r#""""#, r#""not-a-uuid""#] {
        for key in ["actor_session_id", "target_session_id"] {
            let line = opening("old", "old", &format!(",\"{key}\":{value}"));
            let event = ae::events::Event::parse_line(&line)
                .expect("bad ID is an unassociated side, not absent");
            let identity = if key == "actor_session_id" {
                event.actor_identity()
            } else {
                event.target_identity().expect("target")
            };
            assert!(
                matches!(identity, ae::events::Identity::Unassociated),
                "R1 bad {key} downgraded to legacy identity: {identity:?}"
            );
        }
    }
}

#[test]
fn requests_reader_never_closes_bad_id_by_legacy_name() {
    for value in [r#""""#, r#""not-a-uuid""#] {
        for key in ["actor_session_id", "target_session_id"] {
            let line = opening("old", "old", &format!(",\"{key}\":{value}"));
            let rig = Rig::new(key);
            rig.create("old", A);
            rig.append("old", &line);
            rig.append("old", r#"{"ts":"2026-10-10T10:01:00Z","actor":"colead","action":"reply","target":"lead","ref":"spec-request","actor_slot":"worker.0","actor_session":"old","target_slot":"main","target_session":"old"}"#);
            assert!(
                row(
                    &rig.helper("old", "requests", &["all"], None).1,
                    "spec-request",
                    "pending"
                ),
                "R1 requests reader closed malformed ID by name"
            );
        }
    }
}

#[test]
fn event_parser_rejects_duplicate_or_wrong_type_routing_id() {
    for extra in [
        r#", "actor_session_id":7"#,
        r#", "target_session_id":{}"#,
        r#", "actor_session_id":"", "actor_session_id":"""#,
        r#", "target_session_id":"", "target_session_id":"""#,
    ] {
        assert!(
            ae::events::Event::parse_line(&opening("old", "old", extra)).is_err(),
            "R1 duplicate/wrong-type routing ID remained an unknown inert key: {extra}"
        );
    }
}

#[test]
fn routed_cancel_uses_stable_id_after_rename() {
    for cancel_id in [A, B] {
        let rig = Rig::new(if cancel_id == A {
            "cancel-good"
        } else {
            "cancel-wrong"
        });
        rig.create("new", A);
        rig.append(
            "new",
            &opening(
                "old",
                "old",
                &format!(r#", "actor_session_id":"{A}","target_session_id":"{A}""#),
            ),
        );
        rig.append("new", &format!(r#"{{"ts":"2026-10-10T10:01:00Z","actor":"lead","action":"cancel","ref":"spec-request","actor_slot":"main","actor_session":"new","actor_session_id":"{cancel_id}"}}"#));
        let status = if cancel_id == A {
            "cancelled"
        } else {
            "pending"
        };
        assert!(
            row(
                &rig.helper("new", "requests", &["all"], None).1,
                "spec-request",
                status
            ),
            "R1 cancel failed stable identity rule for {cancel_id}"
        );
        let pending = ae::session::SessionRead::open(&rig.dir("new"))
            .expect("own-work reader")
            .pending
            .len();
        assert_eq!(
            pending,
            usize::from(cancel_id != A),
            "R1 ledger readers disagree on cancel"
        );
    }
}

#[test]
fn unequal_valid_id_never_matches_equal_names() {
    let line = opening(
        "new",
        "new",
        &format!(r#", "actor_session_id":"{A}","target_session_id":"{A}""#),
    );
    let reply = format!(
        r#"{{"ts":"2026-10-10T10:01:00Z","actor":"colead","action":"reply","target":"lead","ref":"spec-request","actor_slot":"worker.0","actor_session":"new","target_slot":"main","target_session":"new","actor_session_id":"{B}","target_session_id":"{A}"}}"#
    );
    let rig = Rig::new("unequal");
    rig.create("new", A);
    rig.append("new", &line);
    rig.append("new", &reply);
    assert!(
        row(
            &rig.helper("new", "requests", &["all"], None).1,
            "spec-request",
            "pending"
        ),
        "R1 unequal valid IDs matched equal names"
    );
}

#[test]
fn mixed_legacy_and_stable_ids_keep_the_same_name_compatibility_rule() {
    let rig = Rig::new("mixed");
    rig.create("old", A);
    rig.append("old", &opening("old", "old", ""));
    rig.append("old", &format!(r#"{{"ts":"2026-10-10T10:01:00Z","actor":"colead","action":"reply","target":"lead","ref":"spec-request","actor_slot":"worker.0","actor_session":"old","target_slot":"main","target_session":"old","actor_session_id":"{A}","target_session_id":"{A}"}}"#));
    assert!(
        row(
            &rig.helper("old", "requests", &["all"], None).1,
            "spec-request",
            "replied"
        ),
        "R1 mixed legacy/current compatibility lost"
    );
}
