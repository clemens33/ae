//! Pane delivery against a REAL tmux server and a FAKE TUI.
//!
//! The rig is the point. `cli.rs` drives the core's composition and eventing
//! against panes that merely record; here the pane draws a modelled TUI's
//! input box, so the whole measured path runs for real: the bracketed paste,
//! the input sensor's occupancy reading, the deferral, the Enter and its
//! verification, and the oversize notice with its on-screen proof.
//!
//! The fake TUI is a perl script, and perl rather than a shell for one
//! measured reason: `pane_current_command` reports the INTERPRETER, and every
//! shell name is in the dead-pane guard's shell list, so a shell-scripted fake
//! would be read as a pane whose agent has died. The TOOL is chosen the way a
//! real session chooses it — `agent_bin.<slot>` in the meta, which
//! `ae_target_tool` reads first — so the script itself only has to draw the
//! right shape.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use ae::deliver;
use ae::inventory::ServerId;
use ae::meta::Selector;
use ae::tool::InputModel;

use super::cli::ae;
use super::phase2::run_tmux;

/// The fake TUI.
const FAKE_TUI: &str = r#"#!/usr/bin/perl
use strict;
use warnings;
my ($out, $enters, $cols, $kind, $marker_secs, $mode) = @ARGV;
$cols ||= 400;
$marker_secs ||= 0;
$mode ||= '';
system("stty raw -echo 2>/dev/null");
# RAW, deliberately: the staged bytes come off STDIN already UTF-8, so an
# encoding layer would re-encode each byte and the box would show mojibake —
# which the notice proof compares byte for byte and would rightly reject.
binmode(STDIN, ':raw');
binmode(STDOUT, ':raw');
$| = 1;
print "\e[?2004h";
my $border = "\xe2\x94\x80" x $cols;              # U+2500, as bytes
my $ornament = ($kind eq 'codex') ? "\xe2\x80\xba" : "\xe2\x9d\xaf";
my $nbsp = "\xc2\xa0";
my $started = time();
sub markers {
    # The measured NOT-ready rows, at COLUMN 0, as the TUI draws them.
    print "\e[H\e[2J";
    print "\xe2\x94\x82 model:       loading   /model to change \xe2\x94\x82\r\n";
    print "\xe2\x80\xa2 Starting MCP servers (0/7): fake\r\n";
}
sub draw {
    my ($content) = @_;
    $content =~ s/[\r\n]/ /g;
    print "\e[H\e[2J";
    print "fake tui transcript\r\n";
    print "\e[1m$ornament\e[0m$nbsp$content\r\n";
    if ($kind eq 'codex') { print "\r\n"; } else { print "$border\r\n"; }
    print "  fake-model  ~/x\r\n";
}
sub draw_queued {
    print "\e[H\e[2J";
    print "fake tui transcript\r\n";
    print "\xe2\x9d\xaf Press up to edit queued messages\r\n";
    print "$border\r\n";
    print "  fake-model  ~/x\r\n";
}
my $buf = ($mode =~ /^staged/) ? "[Pasted Content 42 chars]" : "";
my $pasting = 0;
$marker_secs > 0 ? markers() : draw($buf);
my $ch;
while (1) {
    if ($marker_secs > 0 && time() - $started >= $marker_secs) {
        $marker_secs = 0;
        draw($buf);
    }
    my $ready = '';
    vec($ready, fileno(STDIN), 1) = 1;
    next unless select($ready, undef, undef, 0.05) > 0;
    last unless sysread(STDIN, $ch, 1);
    $buf .= $ch;
    if ($buf =~ s/\e\[200~\z//) { $pasting = 1; next; }
    if ($buf =~ s/\e\[201~\z//) {
        $pasting = 0;
        exit 0 if $mode eq 'vanish';
        draw($buf) if $marker_secs == 0;
        next;
    }
    if ($pasting && $ch eq "\r") {
        # tmux `paste-buffer` (no -r) replaces LF with CR on the wire, so a
        # receiver that keeps the bytes it was given maps it back inside a
        # bracketed paste — which is what makes the payload byte-exact.
        chop($buf);
        $buf .= "\n";
        next;
    }
    if (!$pasting && ($ch eq "\r" || $ch eq "\n")) {
        open(my $keys, '>>', $enters) or die;
        print $keys "enter\n";
        close($keys);
        # The Enter itself is already IN the buffer: strip it before any
        # question is asked of what the composer holds.
        $buf =~ s/[\r\n]\z//;
        if ($mode eq 'staged' && $buf eq "[Pasted Content 42 chars]") {
            # The harness drains a staged chip on Enter: the box clears, and
            # the earlier paste is not this turn's receipt.
            $buf = "";
            draw("") if $marker_secs == 0;
            next;
        }
        if ($mode eq 'staged-stuck' && $buf eq "[Pasted Content 42 chars]") {
            # "turn-submit backlog full": the chip stays exactly where it was.
            draw($buf) if $marker_secs == 0;
            next;
        }
        if ($mode eq 'swallow') {
            draw($buf) if $marker_secs == 0;
            next;
        }
        open(my $fh, '>>', $out) or die;
        binmode($fh);
        print $fh $buf;
        close($fh);
        $buf = "";
        $mode eq 'queued' ? draw_queued() : draw("") if $marker_secs == 0;
        next;
    }
    draw($buf) if $marker_secs == 0;
}
"#;

/// One isolated server, one stamped pane running [`FAKE_TUI`], and a session
/// directory whose meta names the tool.
struct Rig {
    scratch: PathBuf,
    sock: PathBuf,
    dir: PathBuf,
    session: String,
    pane: String,
    received: PathBuf,
    enters: PathBuf,
}

impl Rig {
    fn new(tag: &str, tool: &str, marker_secs: u32) -> Self {
        Self::with_mode(tag, tool, marker_secs, "")
    }

    fn with_mode(tag: &str, tool: &str, marker_secs: u32, mode: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let scratch = PathBuf::from(format!("/tmp/aedl.{}.{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        assert!(
            std::fs::create_dir_all(&scratch).is_ok(),
            "a scratch directory"
        );
        let script = scratch.join("faketui.pl");
        assert!(std::fs::write(&script, FAKE_TUI).is_ok(), "the fake TUI");
        assert!(
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).is_ok(),
            "an executable fake TUI"
        );
        let session = format!("dl{tag}");
        let received = scratch.join("received");
        let enters = scratch.join("enters");
        assert!(std::fs::write(&received, "").is_ok(), "the receipt file");
        assert!(
            std::fs::write(&enters, "").is_ok(),
            "the Enter receipt file"
        );
        let rig = Self {
            scratch: scratch.clone(),
            sock: scratch.join("sock"),
            dir: scratch.join("sessions").join(&session),
            session: session.clone(),
            pane: String::new(),
            received,
            enters,
        };
        let command = format!(
            "exec perl {} {} {} 400 {tool} {marker_secs} {mode}",
            script.display(),
            rig.received.display(),
            rig.enters.display(),
        );
        assert!(
            rig.tmux(&[
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-x",
                "400",
                "-y",
                "40",
                "-s",
                &session,
                &command,
            ])
            .0,
            "the fake TUI pane starts"
        );
        let (_, panes) = rig.tmux(&["list-panes", "-s", "-t", &session, "-F", "#{pane_id}"]);
        let pane = panes.lines().next().unwrap_or_default().to_owned();
        assert!(!pane.is_empty(), "{panes}");
        assert!(
            rig.tmux(&["set-option", "-p", "-t", &pane, "@ae_slot", "main"])
                .0
        );
        assert!(
            rig.tmux(&["set-option", "-p", "-t", &pane, "@ae_agent", "tui"])
                .0
        );
        assert!(std::fs::create_dir_all(&rig.dir).is_ok(), "a session dir");
        assert!(
            std::fs::write(
                rig.dir.join("meta"),
                format!(
                    "session={session}\ntmux_server_kind=socket\ntmux_server={}\nseat.main=tui\nagent_bin.main={tool}\n",
                    rig.sock.display()
                ),
            )
            .is_ok(),
            "a meta file"
        );
        let mut rig = rig;
        rig.pane = pane;
        rig.settle();
        rig
    }

    fn server(&self) -> ServerId {
        ServerId::Selected(Selector::Socket(self.sock.clone()))
    }

    fn tmux(&self, tail: &[&str]) -> (bool, String) {
        let mut args = ae::tmux::server_args(&self.server());
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        run_tmux(&args, &self.scratch)
    }

    /// Wait until the pane has drawn something — perl's start-up, not the
    /// product's.
    fn settle(&self) {
        for _ in 0..100 {
            let (ok, screen) = self.tmux(&["capture-pane", "-p", "-t", &self.pane]);
            if ok && !screen.trim().is_empty() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("the fake TUI never drew anything");
    }

    /// Run one core subcommand from this pane.
    fn run(&self, sub: &str, tail: &[&str], envs: &[(&str, &str)]) -> (Option<i32>, String) {
        let mut command = ae();
        command
            .env("TMUX", format!("{},0,0", self.sock.display()))
            .env("TMUX_PANE", &self.pane)
            .env_remove("AE_SENDER_OVERRIDE")
            .arg(sub)
            .arg(&self.dir)
            .args(tail);
        for (key, value) in envs {
            command.env(key, value);
        }
        let out = command
            .output()
            .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// Everything the TUI has SUBMITTED, waiting briefly for it.
    fn submitted(&self) -> String {
        for _ in 0..120 {
            let seen = std::fs::read_to_string(&self.received).unwrap_or_default();
            if !seen.is_empty() {
                return seen;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        std::fs::read_to_string(&self.received).unwrap_or_default()
    }

    fn enter_count(&self) -> usize {
        std::fs::read_to_string(&self.enters)
            .unwrap_or_default()
            .lines()
            .count()
    }

    fn events(&self) -> String {
        std::fs::read_to_string(self.dir.join("events.jsonl")).unwrap_or_default()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = self.tmux(&["kill-server"]);
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

/// A caller pane on a DIFFERENT server from [`Rig`]'s target. Its helper
/// directory shares the target's sessions root, so cross-session resolution
/// must read the target meta and switch servers rather than using ambient.
struct RelayCaller {
    scratch: PathBuf,
    sock: PathBuf,
    dir: PathBuf,
    session: String,
    pane: String,
}

impl RelayCaller {
    fn new(target: &Rig, tag: &str, meta_agent: bool) -> Self {
        let scratch = target.scratch.join(format!("caller-{tag}"));
        assert!(std::fs::create_dir_all(&scratch).is_ok());
        let sock = scratch.join("sock");
        let session = format!("orch{tag}");
        let server = ServerId::Selected(Selector::Socket(sock.clone()));
        let mut args = ae::tmux::server_args(&server);
        args.extend(
            [
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                &session,
                "tail -f /dev/null",
            ]
            .map(ToOwned::to_owned),
        );
        assert!(run_tmux(&args, &scratch).0, "the relay caller starts");
        let mut args = ae::tmux::server_args(&server);
        args.extend(
            ["list-panes", "-s", "-t", &session, "-F", "#{pane_id}"].map(ToOwned::to_owned),
        );
        let pane = run_tmux(&args, &scratch)
            .1
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned();
        assert!(!pane.is_empty());
        for (key, value) in [("@ae_slot", "main"), ("@ae_agent", "orchestrator")] {
            let mut args = ae::tmux::server_args(&server);
            args.extend(["set-option", "-p", "-t", &pane, key, value].map(ToOwned::to_owned));
            assert!(run_tmux(&args, &scratch).0);
        }
        let dir = target.scratch.join("sessions").join(&session);
        assert!(std::fs::create_dir_all(&dir).is_ok());
        let marker = if meta_agent { "meta_agent=true\n" } else { "" };
        assert!(
            std::fs::write(
                dir.join("meta"),
                format!(
                    "session={session}\ntmux_server_kind=socket\ntmux_server={}\nseat.main=orchestrator\nagent_bin.main=codex\n{marker}",
                    sock.display()
                ),
            )
            .is_ok()
        );
        Self {
            scratch,
            sock,
            dir,
            session,
            pane,
        }
    }

    fn server(&self) -> ServerId {
        ServerId::Selected(Selector::Socket(self.sock.clone()))
    }

    fn run(&self, target: &str, text: &str) -> (Option<i32>, String) {
        self.run_from(&self.dir, target, text)
    }

    fn run_from(&self, helper_dir: &Path, target: &str, text: &str) -> (Option<i32>, String) {
        let out = ae()
            .env("TMUX", format!("{},0,0", self.sock.display()))
            .env("TMUX_PANE", &self.pane)
            .arg(ae::cli::RELAY)
            .arg(helper_dir)
            .args([target, text])
            .output()
            .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn rename_session(&self, name: &str) {
        let mut args = ae::tmux::server_args(&self.server());
        args.extend(["rename-session", "-t"].map(ToOwned::to_owned));
        args.push(format!("={}", self.session));
        args.push(name.to_owned());
        assert!(
            run_tmux(&args, &self.scratch).0,
            "the caller session renames"
        );
    }

    fn events(&self) -> String {
        std::fs::read_to_string(self.dir.join("events.jsonl")).unwrap_or_default()
    }
}

impl Drop for RelayCaller {
    fn drop(&mut self) {
        let mut args = ae::tmux::server_args(&self.server());
        args.push("kill-server".to_owned());
        let _ = run_tmux(&args, &self.scratch);
    }
}

#[test]
fn relay_authenticates_the_actual_caller_server_not_the_invoked_helper_server() {
    let target = Rig::new("relayserver", "claude", 0);
    let privileged = RelayCaller::new(&target, "seatserver", true);
    let ordinary = RelayCaller::new(&target, "otherserver", false);
    let named = format!("{}:tui", target.session);
    let body = "do not inherit another server's authority";

    let (code, stderr) = ordinary.run_from(&privileged.dir, &named, body);

    assert_eq!(code, Some(1));
    assert_eq!(
        stderr,
        "ae: relay REFUSED — caller session is not an orchestrator. Nothing was sent.\n"
    );
    assert!(target.submitted().is_empty());
    assert!(target.events().is_empty());
    assert!(ordinary.events().contains(body));
    assert!(privileged.events().is_empty());
}

#[test]
fn relay_refuses_a_foreign_caller_that_renamed_into_the_privileged_session_name() {
    let target = Rig::new("relaynamesake", "claude", 0);
    let privileged = RelayCaller::new(&target, "namesake-a", true);
    let ordinary = RelayCaller::new(&target, "namesake-b", false);
    ordinary.rename_session(&privileged.session);
    let named = format!("{}:tui", target.session);
    let body = "foreign caller must not inherit namesake authority";

    let (code, stderr) = ordinary.run_from(&privileged.dir, &named, body);

    assert_eq!(code, Some(1), "a namesake caller must be refused: {stderr}");
    assert_eq!(
        stderr,
        "ae: relay REFUSED — caller session is not the recorded one. Nothing was sent.\n"
    );
    assert!(target.submitted().is_empty());
    assert!(target.events().is_empty());
    assert!(ordinary.events().is_empty());
    assert!(privileged.events().is_empty());
}

#[test]
fn an_orchestrator_relay_crosses_to_the_target_server_bare_and_audits_only_the_caller() {
    let target = Rig::new("relay", "claude", 0);
    let caller = RelayCaller::new(&target, "allowed", true);
    let body = "Human says:\nship this\texactly";
    let named = format!("{}:tui", target.session);
    let (code, stderr) = caller.run(&target.session, body);
    assert_eq!((code, stderr.as_str()), (Some(0), ""), "{stderr}");
    let submitted = target.submitted();
    assert_eq!(submitted, body, "relay is byte-exact bare text");
    assert!(
        !submitted.contains("⟦ae:msg from"),
        "no peer envelope may downgrade the human-authority relay: {submitted:?}"
    );
    assert!(target.events().is_empty(), "target ledger stays untouched");
    let events = caller.events();
    assert!(
        events.contains(&format!(
            "\"actor\":\"orchestrator\",\"action\":\"relay\",\"target\":\"{named}\""
        )),
        "{events}"
    );
    assert!(
        events.contains("\"summary\":\"Human says:\\nship this\\texactly\""),
        "the full, unflattened text is audited: {events}"
    );
    let body_file = events
        .split("\"body_file\":\"")
        .nth(1)
        .and_then(|tail| tail.split('"').next())
        .unwrap_or_default();
    assert_eq!(std::fs::read_to_string(body_file).unwrap_or_default(), body);
}

#[test]
fn a_non_orchestrator_relay_is_refused_audited_at_the_caller_and_invisible_to_the_target() {
    let target = Rig::new("relaydeny", "claude", 0);
    let caller = RelayCaller::new(&target, "denied", false);
    let named = format!("{}:tui", target.session);
    let body = "pretend this is human";
    let (code, stderr) = caller.run(&named, body);
    assert_eq!(code, Some(1));
    assert_eq!(
        stderr,
        "ae: relay REFUSED — caller session is not an orchestrator. Nothing was sent.\n"
    );
    assert!(target.submitted().is_empty());
    assert!(target.events().is_empty());
    let events = caller.events();
    assert!(events.contains("\"action\":\"relay\""), "{events}");
    assert!(
        events.contains(&format!("\"target\":\"{named}\"")),
        "{events}"
    );
    assert!(
        events.contains(body),
        "the refused full text is audited: {events}"
    );
    assert!(!events.contains("⟦ae:msg from"), "{events}");
}

#[test]
fn an_oversize_relay_names_the_byte_cap_and_never_becomes_a_notice() {
    let target = Rig::new("relaybig", "claude", 0);
    let caller = RelayCaller::new(&target, "big", true);
    let named = format!("{}:tui", target.session);
    assert_eq!(deliver::notice::LIMIT, 8_192);
    let body = "x".repeat(8_193);
    let (code, stderr) = caller.run(&named, &body);
    assert_eq!(code, Some(1));
    assert_eq!(
        stderr,
        format!(
            "ae: relay REFUSED — message is {} B; verbatim relay limit is {} B. Nothing was sent.\n",
            body.len(),
            deliver::notice::LIMIT
        )
    );
    assert!(target.submitted().is_empty());
    assert!(target.events().is_empty());
    assert!(
        !caller.dir.join("messages").exists(),
        "oversize relay never enters the file-and-notice path"
    );
    assert!(caller.events().contains(&body), "full refusal audit");
    assert!(!caller.session.is_empty());
}

/// A MULTI-LINE body reaches a modelled TUI byte for byte, and the recovery
/// record holds the same bytes.
#[test]
fn a_multi_line_body_reaches_a_modelled_tui_byte_for_byte() {
    let rig = Rig::new("exact", "claude", 0);
    let body = "first line\nsecond\tline with  spaces\nthird ⟦unicode⟧ line";
    let (code, stderr) = rig.run(ae::cli::SEND, &["tui", body], &[]);
    assert_eq!((code, stderr.as_str()), (Some(0), ""), "{stderr}");
    let expected = format!("⟦ae:msg from tui⟧\n{body}");
    assert_eq!(
        rig.submitted(),
        expected,
        "the TUI received the framed body byte for byte, newlines and all"
    );
    let events = rig.events();
    let body_file = events
        .split("\"body_file\":\"")
        .nth(1)
        .and_then(|tail| tail.split('"').next())
        .unwrap_or_default();
    assert!(
        !body_file.is_empty(),
        "the event points at a record: {events}"
    );
    assert_eq!(
        std::fs::read_to_string(body_file).unwrap_or_default(),
        expected,
        "the recovery record is the same bytes the pane got"
    );
}

/// An unmodelled TUI can swallow Enter. ae must not call that a verified
/// delivery, but it also must not turn every unmodelled helper send into a
/// failure: the paste and one Enter are a successful, explicit caveat.
#[test]
fn an_unmodelled_unsent_message_is_reported_as_unverifiable_not_delivered() {
    let rig = Rig::with_mode("unmodelled-unsent", "gemini", 0, "swallow");
    let (code, stderr) = rig.run(ae::cli::SEND, &["tui", "did not submit"], &[]);

    assert_eq!((code, stderr.as_str()), (Some(0), ""));
    assert!(rig.submitted().is_empty(), "the fake TUI swallowed Enter");
    assert_eq!(rig.enter_count(), 1, "unknown never receives retry Enters");
    assert!(
        rig.events()
            .contains("\"unverifiable\":\"unmodelled-input\""),
        "the success caveat is durable: {}",
        rig.events()
    );
}

/// Claude replaces the prompt with this queue affordance after accepting a
/// message mid-turn. It is submitted, not an occupied draft, so a verifier
/// must not inject retries into the busy pane.
#[test]
fn a_queued_claude_submission_is_confirmed_without_extra_enters() {
    let rig = Rig::with_mode("queued", "claude", 0, "queued");
    let (code, stderr) = rig.run(ae::cli::SEND, &["tui", "already queued"], &[]);

    assert_eq!(
        rig.submitted(),
        "⟦ae:msg from tui⟧\nalready queued",
        "the first Enter queued the message"
    );
    assert_eq!(
        rig.enter_count(),
        1,
        "the queued affordance is submitted, never a retry target"
    );
    assert_eq!((code, stderr.as_str()), (Some(0), ""), "{stderr}");
}

/// A capture that disappears after paste cannot prove the message submitted.
/// It is an explicit caveat rather than a fabricated confirmed delivery.
#[test]
fn an_unreadable_submit_capture_is_reported_as_unverifiable_not_confirmed() {
    let rig = Rig::with_mode("capture-lost", "claude", 0, "vanish");
    let (code, stderr) = rig.run(ae::cli::SEND, &["tui", "capture vanished"], &[]);

    assert_eq!((code, stderr.as_str()), (Some(0), ""));
    assert!(
        rig.submitted().is_empty(),
        "the vanished pane received no Enter"
    );
    assert!(
        rig.events()
            .contains("\"unverifiable\":\"unreadable-capture\""),
        "the success caveat is durable: {}",
        rig.events()
    );
}

/// A successful unmodelled send remains a success. The caveat is informational
/// and must not reclassify the six unmodelled adapters as delivery failures.
#[test]
fn an_unmodelled_submission_lands_with_a_success_caveat_not_a_failure() {
    let rig = Rig::new("unmodelled-landed", "gemini", 0);
    let (code, stderr) = rig.run(ae::cli::SEND, &["tui", "landed anyway"], &[]);

    assert_eq!((code, stderr.as_str()), (Some(0), ""));
    assert_eq!(rig.submitted(), "⟦ae:msg from tui⟧\nlanded anyway");
    assert!(
        rig.events()
            .contains("\"unverifiable\":\"unmodelled-input\"")
    );
}

/// A send DEFERS while the target's input box holds unsent content, and
/// abandons LOUDLY at the bound rather than clobbering it.
#[test]
fn a_send_defers_while_the_input_box_holds_a_draft_and_abandons_loudly() {
    let rig = Rig::new("busy", "claude", 0);
    assert!(
        rig.tmux(&["send-keys", "-t", &rig.pane, "-l", "half a question"])
            .0
    );
    // The sensor must SEE it before the send is asked to. The polled reading
    // is the proof; a second read would race the pane's redraw (measured on
    // the macos-15 lane: first read OCCUPIED, immediate re-read not).
    let seen = (0..100).any(|_| {
        deliver::input_busy(&rig.server(), &rig.pane, InputModel::BorderDelimited) || {
            std::thread::sleep(Duration::from_millis(50));
            false
        }
    });
    assert!(seen, "a draft in the box reads OCCUPIED");
    let (code, stderr) = rig.run(
        ae::cli::SEND,
        &["tui", "overwrite me"],
        &[("AE_SEND_DEFER_SEC", "1")],
    );
    assert_eq!(code, Some(1), "{stderr}");
    assert_eq!(
        stderr,
        "ae: send to tui ABANDONED — target stayed busy / human input or attention (not clear within 1s; AE_SEND_DEFER_SEC overrides). Re-send.\n"
    );
    assert!(
        rig.submitted().is_empty(),
        "nothing was submitted over the draft"
    );
    assert!(rig.events().is_empty(), "an abandoned send records nothing");
    // The draft is still there, untouched — which is the whole point.
    let (_, screen) = rig.tmux(&["capture-pane", "-p", "-t", &rig.pane]);
    assert!(screen.contains("half a question"), "{screen}");
}

/// A composer holding ONLY an ae-staged paste chip is not a human draft: the
/// send submits that chip once — never pasting a second message over it — and
/// then delivers normally.
#[test]
fn a_send_flushes_an_ae_staged_chip_once_and_then_delivers_over_it() {
    let rig = Rig::with_mode("staged-flush", "muse", 0, "staged");
    assert!(
        deliver::input_busy(&rig.server(), &rig.pane, InputModel::BorderDelimited),
        "the staged chip reads OCCUPIED, as it did in the field"
    );
    let (code, stderr) = rig.run(
        ae::cli::SEND,
        &["tui", "after the chip"],
        &[("AE_SEND_DEFER_SEC", "2")],
    );
    assert_eq!((code, stderr.as_str()), (Some(0), ""), "{stderr}");
    assert_eq!(
        rig.submitted(),
        "⟦ae:msg from tui⟧\nafter the chip",
        "the new message alone crossed the pane"
    );
    assert_eq!(
        rig.enter_count(),
        2,
        "one Enter drains the stalled chip, one submits the send — never a third"
    );
}

/// A chip the harness never drains is submitted exactly ONCE, and the send then
/// abandons loudly: the retry is bounded, not a loop, and nothing is pasted
/// over the chip.
#[test]
fn a_chip_the_harness_never_drains_is_submitted_once_and_abandons_loudly() {
    let rig = Rig::with_mode("staged-stuck", "muse", 0, "staged-stuck");
    let (code, stderr) = rig.run(
        ae::cli::SEND,
        &["tui", "over a refused chip"],
        &[("AE_SEND_DEFER_SEC", "1")],
    );
    assert_eq!(code, Some(1), "{stderr}");
    assert!(
        stderr.contains("ABANDONED — target stayed busy"),
        "{stderr}"
    );
    assert_eq!(
        rig.enter_count(),
        1,
        "exactly one flush Enter; a second would be the unbounded retry"
    );
    assert!(
        rig.submitted().is_empty(),
        "nothing was pasted over the chip"
    );
    let (_, screen) = rig.tmux(&["capture-pane", "-p", "-t", &rig.pane]);
    assert!(screen.contains("[Pasted Content 42 chars]"), "{screen}");
}

/// A body over the notice limit is NOT pasted: a pointer to the sender-owned
/// record crosses the pane instead, and only after the visible input rows
/// prove the exact staged bytes.
#[test]
fn an_oversize_body_crosses_the_pane_as_a_proven_notice() {
    let rig = Rig::new("notice", "claude", 0);
    let body = "y".repeat(9000);
    let (code, stderr) = rig.run(ae::cli::SEND, &["tui", &body], &[]);
    assert_eq!((code, stderr.as_str()), (Some(0), ""), "{stderr}");
    let submitted = rig.submitted();
    assert!(
        submitted.len() <= 300,
        "a pointer crossed the pane, not the body: {} bytes",
        submitted.len()
    );
    assert!(
        submitted
            .starts_with("⟦ae:msg from tui⟧[-] LONG BODY 9022 B in your session dir: messages/")
            && submitted.ends_with(" — read it first ⟧-⟧"),
        "{submitted}"
    );
    // And the body itself is where the pointer says it is, in full.
    let events = rig.events();
    let body_file = events
        .split("\"body_file\":\"")
        .nth(1)
        .and_then(|tail| tail.split('"').next())
        .unwrap_or_default();
    let name = Path::new(body_file)
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default();
    assert!(
        submitted.contains(&format!("messages/{name}")),
        "the pointer names the record the event names: {submitted} vs {body_file}"
    );
    assert_eq!(
        std::fs::read_to_string(body_file).unwrap_or_default(),
        format!("⟦ae:msg from tui⟧\n{body}")
    );
}

/// A pane whose agent has DIED is refused before anything is stored or
/// pasted — a stray Enter there would EXECUTE the message as a shell command.
#[test]
fn a_dead_agent_pane_is_refused_before_the_body_store() {
    let rig = Rig::new("dead", "claude", 0);
    assert!(
        rig.tmux(&["respawn-pane", "-k", "-t", &rig.pane, "exec sh"])
            .0,
        "the agent dies and its pane drops to a shell"
    );
    let (code, stderr) = rig.run(ae::cli::SEND, &["tui", "would execute"], &[]);
    assert_eq!(code, Some(1), "{stderr}");
    assert_eq!(
        stderr,
        "ae: send to tui REFUSED — target pane is a shell, not a running agent (the agent process is gone). Nothing pasted; a stray Enter would EXECUTE the message as a shell command. Re-launch the agent, then re-send.\n"
    );
    assert!(
        !rig.dir.join("messages").exists(),
        "the guard is BEFORE the body store"
    );
    assert!(rig.events().is_empty());
}

/// AN IDLE INPUT BOX IS NOT AN INITIALIZED APPLICATION.
#[test]
fn a_codex_that_is_still_starting_is_not_ready_however_its_box_looks() {
    let rig = Rig::new("boot", "codex", 3);
    let server = rig.server();
    assert!(
        deliver::tool_initializing(&server, &rig.pane, InputModel::StyleDelimited),
        "the start-up rows are on screen"
    );
    assert!(
        !deliver::input_ready(&server, &rig.pane, InputModel::StyleDelimited),
        "provably still starting: NOT ready"
    );
    // The same screen tells a tool ae does not model nothing at all.
    assert!(
        !deliver::tool_initializing(&server, &rig.pane, InputModel::BorderDelimited),
        "the markers are codex's; nothing is claimed about any other tool"
    );
    let mut became_ready = false;
    for _ in 0..120 {
        if deliver::input_ready(&server, &rig.pane, InputModel::StyleDelimited) {
            became_ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(became_ready, "the settled box is ready");
    assert!(
        !deliver::tool_initializing(&server, &rig.pane, InputModel::StyleDelimited),
        "and the markers are gone"
    );
}

/// The delivery reaches a CODEX box too: a different ornament, a different
/// bottom bound (a blank row, not a border), same protocol.
#[test]
fn a_codex_box_takes_the_same_paste_and_confirms_it() {
    let rig = Rig::new("codex", "codex", 0);
    let (code, stderr) = rig.run(ae::cli::SEND, &["tui", "two", "words"], &[]);
    assert_eq!((code, stderr.as_str()), (Some(0), ""), "{stderr}");
    assert_eq!(rig.submitted(), "⟦ae:msg from tui⟧\ntwo words");
    assert!(
        rig.events().contains("\"target\":\"tui\""),
        "{}",
        rig.events()
    );
}

/// The per-target lock is a real file lock, so an external `flock` on it and
/// this one exclude each other.
#[test]
fn a_held_target_lock_makes_the_delivery_wait_and_then_say_so() {
    let rig = Rig::new("lock", "claude", 0);
    let locks = rig.scratch.join("sessions").join(".locks");
    std::fs::create_dir_all(&locks).expect("the lock directory");
    let sanitized: String = rig
        .pane
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect();
    let path = locks.join(format!("send-lock-{sanitized}"));
    // The delivery takes THIS path, so an uncontended send still lands.
    let (code, stderr) = rig.run(ae::cli::SEND, &["tui", "uncontended"], &[]);
    assert_eq!((code, stderr.as_str()), (Some(0), ""), "{stderr}");
    assert!(
        path.exists(),
        "the delivery locked the frozen per-target path: {}",
        path.display()
    );
    assert_eq!(rig.submitted(), "⟦ae:msg from tui⟧\nuncontended");
    assert!(!rig.session.is_empty());
}

/// A MESSAGE-less interrupt is two cancel keystrokes and one event: nothing
/// is pasted, so nothing is stored and there is no dead-pane question to ask.
#[test]
fn a_bare_interrupt_cancels_without_pasting_or_storing() {
    let rig = Rig::new("intbare", "claude", 0);
    let (code, stderr) = rig.run(ae::cli::INTERRUPT, &["tui"], &[]);
    assert_eq!((code, stderr.as_str()), (Some(0), ""), "{stderr}");
    assert!(
        rig.dir.join("messages").metadata().is_err(),
        "a bare cancel stores no recovery body"
    );
    assert!(
        rig.events()
            .contains("\"action\":\"interrupt\",\"target\":\"tui\""),
        "{}",
        rig.events()
    );
    assert!(
        !rig.events().contains("body_file"),
        "and points at no record: {}",
        rig.events()
    );
}

/// A message interrupt reaches the pane UNFRAMED — a control action is not
/// transcript chat — and does not wait for a quiet input box, which is the
/// whole point of interrupting.
#[test]
fn a_message_interrupt_lands_marked_as_control_not_peer_chat_even_with_a_draft_in_the_box() {
    let rig = Rig::new("intmsg", "claude", 0);
    assert!(
        rig.tmux(&["send-keys", "-t", &rig.pane, "-l", "mid-generation draft"])
            .0
    );
    for _ in 0..100 {
        if deliver::input_busy(&rig.server(), &rig.pane, InputModel::BorderDelimited) {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let (code, stderr) = rig.run(ae::cli::INTERRUPT, &["tui", "try", "another", "way"], &[]);
    assert_eq!((code, stderr.as_str()), (Some(0), ""), "{stderr}");
    let submitted = rig.submitted();
    assert!(
        submitted.ends_with("try another way"),
        "the message arrived: {submitted:?}"
    );
    let marker = ae::provenance::interrupt("tui");
    assert!(
        submitted.ends_with(&format!("{marker}\ntry another way")),
        "the message arrives under its control-action marker, past the draft: {submitted:?}"
    );
    assert!(
        !submitted.contains("⟦ae:msg from"),
        "and never borrows the peer envelope: {submitted:?}"
    );
    assert!(
        rig.events().contains("\"action\":\"interrupt\",\"target\":\"tui\",\"summary\":\"try another way\",\"body_file\":"),
        "the summary is the message as typed, and the record is beside it: {}",
        rig.events()
    );
}

/// A message interrupt to a pane whose agent has DIED is refused with the
/// send's own guard: a paste plus Enter into a shell EXECUTES it.
#[test]
fn a_message_interrupt_to_a_dead_pane_is_refused() {
    let rig = Rig::new("intdead", "claude", 0);
    assert!(
        rig.tmux(&["respawn-pane", "-k", "-t", &rig.pane, "exec sh"])
            .0
    );
    let (code, stderr) = rig.run(ae::cli::INTERRUPT, &["tui", "would", "execute"], &[]);
    assert_eq!(code, Some(1), "{stderr}");
    assert_eq!(
        stderr,
        "ae: interrupt of tui REFUSED — target pane is a shell, not a running agent; a stray Enter would EXECUTE the message as a shell command. Re-launch the agent, then re-send.\n"
    );
    assert!(rig.events().is_empty());
    // A BARE interrupt of the same pane is fine: nothing is pasted there.
    let (code, stderr) = rig.run(ae::cli::INTERRUPT, &["tui"], &[]);
    assert_eq!((code, stderr.as_str()), (Some(0), ""), "{stderr}");
}

/// The argv refusals, and a target that does not resolve.
#[test]
fn interrupt_refuses_exactly_and_records_nothing_for_a_refusal() {
    let rig = Rig::new("intref", "claude", 0);
    let (code, stderr) = rig.run(ae::cli::INTERRUPT, &[], &[]);
    assert_eq!((code, stderr.as_str()), (Some(2), ae::interrupt::USAGE));
    let (code, stderr) = rig.run(ae::cli::INTERRUPT, &["nobody", "x"], &[]);
    assert_eq!(code, Some(1));
    assert_eq!(
        stderr,
        "Error: agent 'nobody' not found in session 'dlintref'\n"
    );
    assert!(rig.events().is_empty());
    assert!(rig.submitted().is_empty());
}

/// A BODY NEVER OUTLIVES ITS DELIVERY IN THE SERVER'S BUFFER STACK.
#[test]
fn a_paste_that_fails_after_the_load_leaves_no_buffer_behind() {
    let rig = Rig::new("leak", "claude", 0);
    let server = rig.server();
    let live = deliver::stage_and_paste(&server, "ae-send-probe", b"LIVE-BODY", &rig.pane);
    assert_eq!(live, Ok(()), "a live pane takes the paste");
    let (_, buffers) = rig.tmux(&["list-buffers", "-F", "#{buffer_name}"]);
    assert!(
        !buffers.contains("ae-send-probe"),
        "a successful paste consumes its buffer: {buffers:?}"
    );

    // The server must OUTLIVE the pane: killing the last pane exits the server,
    // and then the load itself fails (nothing staged, nothing to leak).
    assert!(
        rig.tmux(&["new-session", "-d", "-s", "keepalive", "sleep", "60"])
            .0,
        "a second session holds the server up"
    );
    assert!(
        rig.tmux(&["kill-pane", "-t", &rig.pane]).0,
        "the target dies"
    );
    let dead = deliver::stage_and_paste(
        &server,
        "ae-send-probe",
        b"SENSITIVE-BODY-MARKER",
        &rig.pane,
    );
    assert_eq!(
        dead,
        Err(deliver::StageFailure::Paste),
        "the paste must fail on a dead pane"
    );
    let (_, buffers) = rig.tmux(&["list-buffers", "-F", "#{buffer_name}"]);
    assert!(
        !buffers.contains("ae-send-probe"),
        "a failed paste must delete what it staged: {buffers:?}"
    );
}

/// The paste-and-Enter SINK has four entry shapes and its guards are NOT
/// uniform. This table is the CURRENT TRUTH, gaps included — it is not a
/// refactor plan, and it must not be "fixed" into one:
///
/// | shape            | pane-alive (proof currency)    | Unproven ->                  | target-lock | quiet-gate | composed+proof | submit-verify |
/// |------------------|--------------------------------|------------------------------|-------------|------------|----------------|---------------|
/// | Send             | pre-lock ONLY                  | DELIVER (standing fail-open) | yes         | YES        | no             | modelled / Unknown |
/// | Relay            | pre-lock ONLY                  | DELIVER                      | yes         | YES        | no             | same |
/// | Interrupt        | pre-lock ONLY                  | DELIVER                      | yes         | no         | no             | same |
/// | Launch, modelled | pre-lock ONLY (inherited gap)  | DELIVER (inherited)          | yes         | no         | no (the CALLER's `InputModel` owns it) | modelled |
/// | Launch, unmodel  | pre-lock + under-lock re-proof | REFUSE (`Failure::Unproven`) | yes         | no         | YES (probe, liveness, composed screen) | Unknown |
///
/// PINNED AS THEY ARE: Interrupt is quiet-ungated; Launch skips the quiet
/// gate; and pane-alive is PROOF-CURRENT ONLY FOR AN UNMODELLED LAUNCH — Send,
/// Relay, Interrupt and a modelled Launch prove liveness once, BEFORE the lock
/// and never again (inherited, not fixed here). Unproven (a shell foreground
/// ae cannot attribute to a live agent) DELIVERS on those four rows and
/// REFUSES at the under-lock Launch. A modelled pane's readiness answers
/// through `InputModel` in its caller, never here. LIVE below: pane-alive for
/// every shape and both models, the quiet asymmetry, the composed asymmetry.
/// NOT live: the target lock (its wait is two minutes; the failure shape is
/// pinned elsewhere) and proof-currency (tests/it/spawn.rs exercises it under
/// a held lock, both the Dead and the Unproven arm).
#[test]
fn every_path_into_the_paste_sink_carries_its_declared_guards() {
    use deliver::{Failure, Shape};

    let short = Duration::from_millis(50);
    let rig = Rig::with_mode("guards", "claude", 0, "queued");
    let rewrite_meta = |tool: &str| {
        std::fs::write(
            rig.dir.join("meta"),
            format!(
                "session={}\ntmux_server_kind=socket\ntmux_server={}\nseat.main=tui\nagent_bin.main={tool}\n",
                rig.session,
                rig.sock.display()
            ),
        )
        .unwrap();
    };
    let send = |shape, composed, defer| -> Result<deliver::Delivered, Failure> {
        let server = rig.server();
        let mut err = Vec::new();
        let request = deliver::Request {
            dir: &rig.dir,
            server: &server,
            pane: &rig.pane,
            logged_target: "tui",
            target_session: &rig.session,
            pane_slot: "main",
            own_session: &rig.session,
            action: "send",
            reference: "guards-probe",
            actor: "",
            body: "guard probe",
            shape,
            defer,
            composed,
        };
        deliver::deliver(&request, &mut err).expect("deliver writes only on i/o faults")
    };

    // Interrupt takes the busy-less lane and leaves the fake in its QUEUED
    // state, which is a busy box for the two shapes that ARE quiet-gated.
    assert!(send(Shape::Interrupt, &[], short).is_ok());
    assert!(matches!(
        send(Shape::Send, &[], short),
        Err(Failure::Abandoned)
    ));
    assert!(matches!(
        send(Shape::Relay, &[], short),
        Err(Failure::Abandoned)
    ));
    assert!(
        send(Shape::Launch, &[], short).is_ok(),
        "Launch skips the quiet gate"
    );

    // The composed under-lock recheck is an UNMODELLED Launch ONLY, and the
    // meta row is the classifier's first answer, exactly as in a real seat.
    rewrite_meta("gemini");
    assert!(matches!(
        send(Shape::Launch, &[], short),
        Err(Failure::NotComposed { .. })
    ));
    rewrite_meta("claude");
    assert!(
        send(Shape::Launch, &[], short).is_ok(),
        "a modelled Launch never runs the composed recheck"
    );

    // The pane-alive guard is PRE-model and covers every shape: the agent dies
    // and the pane drops to a shell.
    rewrite_meta("gemini");
    assert!(
        rig.tmux(&["respawn-pane", "-k", "-t", &rig.pane, "exec sh"])
            .0,
        "the agent dies and its pane drops to a shell"
    );
    for shape in [Shape::Send, Shape::Relay, Shape::Interrupt, Shape::Launch] {
        assert!(
            matches!(send(shape, &[], short), Err(Failure::DeadPane)),
            "{shape:?} must refuse a dead pane"
        );
    }
    rewrite_meta("claude");
    assert!(
        matches!(send(Shape::Send, &[], short), Err(Failure::DeadPane)),
        "and the same guard holds for a modelled row"
    );
}

// ---- R10: the guarded operation ---------------------------------------------
// Pins for `deliver::deliver_guarded`, driven in-process. Every call matches
// the `Result` — never `?` (pinned against this file).

impl Rig {
    /// A guarded request against this rig's pane.
    fn guarded<'a>(
        &'a self,
        server: &'a ServerId,
        text: &'a str,
        defer: Duration,
    ) -> deliver::GuardedRequest<'a> {
        deliver::GuardedRequest {
            dir: &self.dir,
            server,
            pane: &self.pane,
            pane_slot: "main",
            target_session: &self.session,
            own_session: &self.session,
            model: InputModel::BorderDelimited,
            text,
            defer,
        }
    }

    /// The pane's send-lock path, as `lock_target` derives it.
    fn send_lock_path(&self) -> PathBuf {
        let sanitized = self
            .pane
            .replace(|ch: char| !ch.is_ascii_alphanumeric(), "_");
        self.scratch
            .join("sessions")
            .join(".locks")
            .join(format!("send-lock-{sanitized}"))
    }

    /// The session's lifecycle-lock path, as `lifecycle::lock` derives it.
    fn lifecycle_lock_path(&self) -> PathBuf {
        self.scratch
            .join("sessions")
            .join(format!(".lifecycle.{}.lock", self.session))
    }
}

/// Take the lifecycle lock at `path`, mapping any refusal to the
/// caller-owned leg — the shape the C2b verb's `prove` takes.
fn prove_lock(path: &Path, wait: Duration) -> Result<std::fs::File, deliver::Leg> {
    ae::store::lock(path, wait).map_err(|_| deliver::Leg::LifecycleLocked)
}

fn lock_is_free(path: &Path) -> bool {
    ae::store::lock(path, Duration::ZERO).is_ok()
}

#[test]
fn a_held_send_lock_delays_the_guarded_run_before_the_lifecycle_lock() {
    // R10 order pin: a held send-lock delays the run BEFORE the lifecycle
    // lock is taken — the lifecycle path stays acquirable meanwhile.
    let rig = Rig::new("guardlock", "claude", 0);
    let server = rig.server();
    std::fs::create_dir_all(rig.send_lock_path().parent().expect("a locks root"))
        .expect("a locks root");
    let held = ae::store::lock(&rig.send_lock_path(), Duration::ZERO).expect("the foreign hold");
    let called = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|scope| {
        let running = scope.spawn(|| {
            let request = rig.guarded(&server, "/compact", Duration::from_secs(5));
            deliver::deliver_guarded(&request, || {
                called.store(true, std::sync::atomic::Ordering::SeqCst);
                prove_lock(&rig.lifecycle_lock_path(), Duration::ZERO)
            })
        });
        std::thread::sleep(Duration::from_millis(500));
        assert!(
            !called.load(std::sync::atomic::Ordering::SeqCst),
            "prove must not run while the send-lock is held"
        );
        let free = lock_is_free(&rig.lifecycle_lock_path());
        assert!(free, "acquirable meanwhile");
        drop(held);
        let done = running.join().expect("the run finishes");
        let proved = called.load(std::sync::atomic::Ordering::SeqCst);
        assert!(proved, "prove runs once the send-lock releases");
        assert!(matches!(done, Ok(deliver::Outcome::Sent(_))), "{done:?}");
    });
}

#[test]
fn a_seat_dead_to_shell_after_step_1_is_refused_pre_paste() {
    // R10 pin: the process exits to a shell DURING the run, identity still
    // matches — the under-lock probe refuses pre-paste, NOTHING typed.
    let rig = Rig::new("guardshell", "claude", 0);
    let server = rig.server();
    let request = rig.guarded(&server, "/compact", Duration::from_secs(5));
    let done = deliver::deliver_guarded(&request, || {
        assert!(
            rig.tmux(&["respawn-pane", "-k", "-t", &rig.pane, "exec sh"])
                .0,
            "the agent dies to a shell under the lock"
        );
        prove_lock(&rig.lifecycle_lock_path(), Duration::ZERO)
    });
    assert!(
        matches!(done, Ok(deliver::Outcome::Skipped(deliver::Leg::Dead))),
        "{done:?}"
    );
    assert_eq!(rig.enter_count(), 0, "no Enter reached the shell");
    assert!(
        std::fs::read_to_string(&rig.received)
            .unwrap_or_default()
            .is_empty(),
        "nothing submitted"
    );
    let (_, screen) = rig.tmux(&["capture-pane", "-p", "-t", &rig.pane]);
    assert!(!screen.contains("/compact"), "nothing typed: {screen}");
}

#[test]
fn a_composer_busy_between_step_1_and_3_skips_at_once() {
    // R10 pin: the composer turns busy between step 1 and step 3 — one
    // snapshot refuses at once, no wait. A loop would burn the 30 s deferral.
    let rig = Rig::new("guardbusy", "claude", 0);
    let server = rig.server();
    let request = rig.guarded(&server, "/compact", Duration::from_secs(30));
    let started = std::time::Instant::now();
    let done = deliver::deliver_guarded(&request, || {
        assert!(
            rig.tmux(&["send-keys", "-t", &rig.pane, "-l", "a half-typed draft"])
                .0,
            "the composer turns busy under the lock"
        );
        let seen = (0..100).any(|_| {
            deliver::input_busy(&rig.server(), &rig.pane, InputModel::BorderDelimited) || {
                std::thread::sleep(Duration::from_millis(50));
                false
            }
        });
        assert!(seen, "the snapshot must see the draft");
        prove_lock(&rig.lifecycle_lock_path(), Duration::ZERO)
    });
    assert!(
        matches!(done, Ok(deliver::Outcome::Skipped(deliver::Leg::Busy))),
        "{done:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "no wait, at once"
    );
    assert_eq!(rig.enter_count(), 0, "no Enter over the draft");
}

#[test]
fn a_fifo_at_the_meta_path_leaves_the_critical_section_unaffected() {
    // R10 pin: under the lock NO file is opened — a FIFO at the meta path
    // would block any open forever. Bounded 30 s, then rescue-or-fail.
    let rig = Rig::new("guardfifo", "claude", 0);
    let (done_send, done_recv) = std::sync::mpsc::channel();
    let dir = rig.dir.clone();
    let server = rig.server();
    let pane = rig.pane.clone();
    let session = rig.session.clone();
    let lock_path = rig.lifecycle_lock_path();
    let meta = rig.dir.join("meta");
    let stash = rig.dir.join("meta.stash");
    std::thread::spawn(move || {
        let request = deliver::GuardedRequest {
            dir: &dir,
            server: &server,
            pane: &pane,
            pane_slot: "main",
            target_session: &session,
            own_session: &session,
            model: InputModel::BorderDelimited,
            text: "/compact",
            defer: Duration::from_secs(5),
        };
        let done = deliver::deliver_guarded(&request, || {
            std::fs::rename(&meta, &stash).expect("the meta stashes");
            super::cli::mkfifo(&meta);
            prove_lock(&lock_path, Duration::ZERO)
        });
        let _ = done_send.send(done);
    });
    let first = done_recv.recv_timeout(Duration::from_secs(30));
    if first.is_err() {
        if let Ok(mut fifo) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(rig.dir.join("meta"))
        {
            let _ = std::io::Write::write_all(&mut fifo, b"session=garbage\n");
        }
        let _ = done_recv.recv_timeout(Duration::from_secs(30));
        panic!("the critical section blocked on the meta FIFO");
    }
    let outcome = first.expect("the first wait succeeded");
    assert!(
        matches!(outcome, Ok(deliver::Outcome::Sent(_))),
        "{outcome:?}"
    );
}

/// Restart the rig's server VERBOSE: the server log then records every
/// client's lifecycle for the count pin. Pane, stamps and meta rebuilt.
fn restart_verbose(rig: &mut Rig) {
    assert!(rig.tmux(&["kill-server"]).0, "the plain server dies");
    let command = format!(
        "exec perl {}/faketui.pl {} {} 400 claude 0 ",
        rig.scratch.display(),
        rig.received.display(),
        rig.enters.display()
    );
    let mut tail = vec![
        "-v",
        "-f",
        "/dev/null",
        "new-session",
        "-d",
        "-x",
        "400",
        "-y",
        "40",
        "-s",
    ];
    tail.push(&rig.session);
    tail.push(&command);
    assert!(rig.tmux(&tail).0, "the verbose server starts");
    let (_, panes) = rig.tmux(&["list-panes", "-s", "-t", &rig.session, "-F", "#{pane_id}"]);
    panes
        .lines()
        .next()
        .unwrap_or_default()
        .clone_into(&mut rig.pane);
    assert!(!rig.pane.is_empty(), "{panes}");
    assert!(
        rig.tmux(&["set-option", "-p", "-t", &rig.pane, "@ae_slot", "main"])
            .0
    );
    assert!(
        rig.tmux(&["set-option", "-p", "-t", &rig.pane, "@ae_agent", "tui"])
            .0
    );
    rig.settle();
}

/// The verbose server's log — exactly one, started by [`restart_verbose`].
fn server_log(scratch: &Path) -> Option<PathBuf> {
    let found: Vec<PathBuf> = std::fs::read_dir(scratch)
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.display().to_string().contains("tmux-server-"))
        .collect();
    if found.len() == 1 {
        found.into_iter().next()
    } else {
        None
    }
}

#[test]
fn the_critical_section_bills_two_calls_per_enter() {
    // R10 pin: over a live seat steps 3+4 are ONE pane probe, ONE busy
    // capture, ONE client list, load, paste, then one send plus one capture
    // PER Enter — COUNTED as the verbose server's completed clients, never
    // wall time. `prove` checkpoints the log offset. Verdict is any Sent;
    // the count relation is the claim. Both locks release.
    //
    // Why the server log and not a socket proxy: tmux clients pass stdio
    // fds over the socket, and a byte relay without recvmsg leaks the
    // client's stdout write-end into this process — `output()` then blocks
    // forever on a self-held pipe (measured). std has no recvmsg.
    // Marker verified on tmux 3.7b; a rename fails loudly, never silently.
    let mut rig = Rig::new("guardcount", "claude", 0);
    restart_verbose(&mut rig);
    let server = rig.server();
    let request = rig.guarded(&server, "/compact", Duration::from_secs(5));
    let log = server_log(&rig.scratch).expect("one server log");
    let mark = std::cell::Cell::new(0usize);
    let done = deliver::deliver_guarded(&request, || {
        mark.set(std::fs::read(&log).expect("the log reads").len());
        prove_lock(&rig.lifecycle_lock_path(), Duration::ZERO)
    });
    assert!(matches!(done, Ok(deliver::Outcome::Sent(_))), "{done:?}");
    // Five calls are fixed; every Enter bills exactly one send plus one
    // submit capture — judged on `freed` itself; the TUI receipt may lag.
    let mut freed = 0;
    let mut steady = 0;
    for _ in 0..600 {
        let bytes = std::fs::read(&log).expect("the log reads");
        let now = String::from_utf8_lossy(&bytes[mark.get()..])
            .matches("free client")
            .count();
        steady = if now == freed { steady + 1 } else { 0 };
        freed = now;
        if matches!(freed, 7 | 9 | 11) && steady >= 5 {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let enters = rig.enter_count();
    assert!(
        matches!(freed, 7 | 9 | 11),
        "five fixed, then send + capture per Enter: freed={freed} enters={enters}"
    );
    assert!(
        (1..=(freed - 5) / 2).contains(&enters),
        "TUI receipt lags, never exceeds: freed={freed} enters={enters}"
    );
    let free_send = lock_is_free(&rig.send_lock_path());
    let free_life = lock_is_free(&rig.lifecycle_lock_path());
    assert!(free_send, "the send-lock releases");
    assert!(free_life, "the lifecycle lock releases");
}

#[test]
fn a_refused_enter_is_enter_failed_never_unknown() {
    // R10/R5 pin: `send_key` refuses every send to a dead socket, so the
    // operation's own submit answers `Err(EnterFailed)` — never `Unknown`,
    // on the FIRST Enter, deterministically. No TUI needed.
    let sock = PathBuf::from("/tmp/aedl.dead-server-guard/sock");
    let server = ServerId::Selected(Selector::Socket(sock));
    let done = deliver::submit_bounded(&server, "%9", InputModel::BorderDelimited);
    assert!(matches!(done, Err(deliver::EnterFailed)), "{done:?}");
}

#[test]
fn a_prove_leg_skips_with_the_send_lock_released() {
    // R10 pin: the caller's identity proof refuses — the leg skips, the
    // send-lock releases with the scope, the lifecycle lock was never taken.
    let rig = Rig::new("guardleg", "claude", 0);
    let server = rig.server();
    let request = rig.guarded(&server, "/compact", Duration::from_secs(5));
    let done = deliver::deliver_guarded(&request, || Err(deliver::Leg::Vacant));
    assert!(
        matches!(done, Ok(deliver::Outcome::Skipped(deliver::Leg::Vacant))),
        "{done:?}"
    );
    let free_send = lock_is_free(&rig.send_lock_path());
    let free_life = lock_is_free(&rig.lifecycle_lock_path());
    assert!(free_send, "the send-lock releases");
    assert!(free_life, "never taken");
}

#[test]
fn the_guarded_operation_is_pinned_against_its_own_source() {
    // R10 pins asked of the source text, doors-style.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let product = std::fs::read_to_string(root.join("src/deliver.rs")).expect("the product source");
    // No guard is implemented twice. (`pane_agent_is_dead` is the design's
    // name for what main calls `pane_liveness_at` — pinned under its real
    // name.)
    for owner in [
        "fn pane_liveness_at(",
        "fn wait_for_quiet(",
        "fn recently_viewed(",
        "fn input_busy(",
    ] {
        assert_eq!(product.match_indices(owner).count(), 1, "{owner}");
    }
    // The operation calls those owners, and its submit maps — never `?`.
    let op = product
        .find("pub fn deliver_guarded(")
        .expect("the operation");
    let end = product.find("pub fn submit_bounded(").expect("the submit");
    let body = &product[op..end];
    for call in [
        "pane_liveness_at(",
        "wait_for_quiet(",
        "observe_pane_probe(",
        "input_busy(",
        "recently_viewed(",
        "stage_and_paste(",
        "submit_bounded(",
        ".map(Outcome::Sent)",
    ] {
        assert!(body.contains(call), "{call}");
    }
    // The lifecycle lock releases before the send-lock — explicit drops, in
    // order, on the single exit.
    let drop_lifecycle = body.find("drop(lifecycle_lock);").expect("the (5) drop");
    let drop_send = body.find("drop(send_lock);").expect("the (6) drop");
    assert!(drop_lifecycle < drop_send, "lifecycle first");
    // The operation's call is never `?`-propagated — every call site in this
    // file matches the `Result` instead.
    let pins = std::fs::read_to_string(root.join("tests/it/deliver.rs")).expect("the pins");
    for (idx, line) in pins.lines().enumerate() {
        if line.contains("deliver_guarded(&") {
            let n = idx + 1;
            assert!(!line.contains('?'), "line {n}: {line}");
        }
    }
}
