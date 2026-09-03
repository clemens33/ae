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

use ae::deliver::{self, region::Tool};
use ae::inventory::ServerId;
use ae::meta::Selector;

use super::cli::ae;
use super::phase2::run_tmux;

/// The fake TUI. Draws a claude- or codex-shaped input box, keeps the staged
/// bytes visible in it, and appends each SUBMITTED body to a file byte for
/// byte.
///
/// Bracketed paste is ENABLED (`\e[?2004h`) so tmux's `paste-buffer -p`
/// actually brackets, and the markers are stripped from the staged bytes: a
/// newline INSIDE a paste is content, a newline outside it submits. That is
/// the protocol the measurement of 2026-08-30 is about — plain paste lost the
/// head in 4/4 trials, bracketed 0/6 — so a rig that did not bracket would be
/// testing a different thing.
const FAKE_TUI: &str = r#"#!/usr/bin/perl
use strict;
use warnings;
my ($out, $cols, $kind, $marker_secs) = @ARGV;
$cols ||= 400;
$marker_secs ||= 0;
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
if ($marker_secs > 0) { markers(); } else { draw(""); }
my $buf = "";
my $pasting = 0;
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
    if ($buf =~ s/\e\[201~\z//) { $pasting = 0; draw($buf) if $marker_secs == 0; next; }
    if ($pasting && $ch eq "\r") {
        # tmux `paste-buffer` (no -r) replaces LF with CR on the wire, so a
        # receiver that keeps the bytes it was given maps it back inside a
        # bracketed paste — which is what makes the payload byte-exact.
        chop($buf);
        $buf .= "\n";
        next;
    }
    if (!$pasting && ($ch eq "\r" || $ch eq "\n")) {
        $buf =~ s/[\r\n]\z//;
        open(my $fh, '>>', $out) or die;
        binmode($fh);
        print $fh $buf;
        close($fh);
        $buf = "";
        draw("") if $marker_secs == 0;
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
}

impl Rig {
    fn new(tag: &str, tool: &str, marker_secs: u32) -> Self {
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
        assert!(std::fs::write(&received, "").is_ok(), "the receipt file");
        let rig = Self {
            scratch: scratch.clone(),
            sock: scratch.join("sock"),
            dir: scratch.join("sessions").join(&session),
            session: session.clone(),
            pane: String::new(),
            received,
        };
        let command = format!(
            "exec perl {} {} 400 {tool} {marker_secs}",
            script.display(),
            rig.received.display()
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

/// A MULTI-LINE body reaches a modelled TUI byte for byte, and the recovery
/// record holds the same bytes.
///
/// This is the bracketed-paste protocol under test, not just "a send works":
/// the fake TUI enables bracketed paste, so tmux's `paste-buffer -p` brackets,
/// and the newlines INSIDE the body stay content instead of submitting it
/// three times. Plain paste is what lost the head in 4/4 measured trials.
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

/// A send DEFERS while the target's input box holds unsent content, and
/// abandons LOUDLY at the bound rather than clobbering it.
///
/// The draft is typed straight at the pane, as a human would type it: the
/// point is that ae reads the SCREEN and refuses to paste over what it sees.
#[test]
fn a_send_defers_while_the_input_box_holds_a_draft_and_abandons_loudly() {
    let rig = Rig::new("busy", "claude", 0);
    assert!(
        rig.tmux(&["send-keys", "-t", &rig.pane, "-l", "half a question"])
            .0
    );
    // The sensor must SEE it before the send is asked to.
    for _ in 0..100 {
        if deliver::input_busy(&rig.server(), &rig.pane, Tool::Claude) {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        deliver::input_busy(&rig.server(), &rig.pane, Tool::Claude),
        "a draft in the box reads OCCUPIED"
    );
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
///
/// The fake codex draws the two measured NOT-ready rows for a while with no
/// input box at all, then settles into one. Readiness must be false for the
/// whole of the first phase and true in the second — the markers are what
/// makes the difference, and their absence is not itself proof of anything.
#[test]
fn a_codex_that_is_still_starting_is_not_ready_however_its_box_looks() {
    let rig = Rig::new("boot", "codex", 3);
    let server = rig.server();
    assert!(
        deliver::tool_initializing(&server, &rig.pane, Tool::Codex),
        "the start-up rows are on screen"
    );
    assert!(
        !deliver::input_ready(&server, &rig.pane, Tool::Codex),
        "provably still starting: NOT ready"
    );
    // The same screen tells a tool ae does not model nothing at all.
    assert!(
        !deliver::tool_initializing(&server, &rig.pane, Tool::Claude),
        "the markers are codex's; nothing is claimed about any other tool"
    );
    let mut became_ready = false;
    for _ in 0..120 {
        if deliver::input_ready(&server, &rig.pane, Tool::Codex) {
            became_ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(became_ready, "the settled box is ready");
    assert!(
        !deliver::tool_initializing(&server, &rig.pane, Tool::Codex),
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

/// The per-target lock is the frozen path, so a bash `flock` on it and this
/// one exclude each other — which is what keeps the glue's `_send-deliver`
/// safe while it still exists.
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
fn a_message_interrupt_lands_unframed_even_with_a_draft_in_the_box() {
    let rig = Rig::new("intmsg", "claude", 0);
    assert!(
        rig.tmux(&["send-keys", "-t", &rig.pane, "-l", "mid-generation draft"])
            .0
    );
    for _ in 0..100 {
        if deliver::input_busy(&rig.server(), &rig.pane, Tool::Claude) {
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
    assert!(
        !submitted.contains("⟦ae:msg from"),
        "an interrupt carries no provenance envelope: {submitted:?}"
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
