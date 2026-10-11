//! Frozen acceptance: lead T1-T7/T3prime and plan d4133501. Real private tmux,
//! real delivery/journal/config owners; only the external vendor UI is fake.
#![allow(
    clippy::disallowed_methods,
    reason = "acceptance fixtures own private filesystem, environment and tmux state"
)]

use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::parity::{
    Invocation,
    capture::{ExitOutcome, raw},
};
use ae::events::Event;

const UUID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const CLAUDE: &str = include_str!("../fixtures/claude-trust/claude-trust-modal-200x50.txt");
const AGY: &str = include_str!("../fixtures/agy-composer/agy-trust-modal-frame.txt");
const CLAUDE_IDLE: &str = include_str!("../fixtures/harness-state/claude-idle-167x40.txt");
const AGY_IDLE: &str = include_str!("../fixtures/agy-composer/agy-composed-frame.txt");

// A copied native perl executable named claude/agy supplies an OS-proven tool
// process, unlike a shebang whose comm is perl. No vendor store is opened.
const VENDOR: &str = r#"use strict;
use warnings;
use utf8;
use Encode qw(encode_utf8 decode_utf8);
my $root = shift @ARGV;
system('stty raw -echo') == 0 or die 'stty';
binmode STDIN; binmode STDOUT; $| = 1;
sub get { my ($p) = @_; open(my $f, '<:raw', $p) or return ''; local $/; return <$f> // ''; }
sub put { my ($p,$s) = @_; open(my $f, '>:raw', $p) or die $p; print $f $s; close $f; }
sub logline { my ($p,$s) = @_; open(my $f, '>>:raw', $p) or die $p; print $f $s; close $f; }
my $initial = decode_utf8(get("$root/frame"));
my $frame = $initial;
my $mode = get("$root/mode");
my $tool = get("$root/tool");
my $dir = get("$root/session-dir");
my $trusted = 0;
my $buffer = '';
my $pasting = 0;
sub draw {
    my $s = $frame; $s =~ s/\r?\n/\r\n/g;
    print "\e[H\e[2J", encode_utf8($s);
}
sub composer {
    $frame = decode_utf8(get("$root/idle"));
    $trusted = 1; draw();
}
print "\e[?2004h"; draw(); put("$root/ready", "$$");
while (1) {
    if (-e "$root/repaint") {
        $frame = decode_utf8(get("$root/repaint")); unlink "$root/repaint";
        $trusted = 0; draw(); put("$root/repainted", 'yes');
    }
    my $bits = ''; vec($bits, fileno(STDIN), 1) = 1;
    if (select($bits, undef, undef, 0.01) > 0) {
        my $bytes; last unless sysread(STDIN, $bytes, 4096);
        put("$root/pre-key", get("$dir/events.jsonl")) unless -e "$root/pre-key";
        logline("$root/wire", $bytes);
        $buffer .= $bytes;
        while (length $buffer) {
            if ($buffer =~ s/^\e\[200~//) { $pasting = 1; next; }
            if ($buffer =~ s/^\e\[201~//) { $pasting = 0; next; }
            if ($buffer =~ /^\e(?:\[|O)?$/) { last; }
            if (!$trusted && $buffer =~ s/^\e(?:\[|O)([AB])//) {
                my $key = $1 eq 'B' ? 'Down' : 'Up';
                logline("$root/actions", "$key\n");
                if ($mode eq 'gone') { composer(); next; }
                if ($mode ne 'stay-no' && (($tool eq 'claude' && $key eq 'Down') || ($tool eq 'agy' && $key eq 'Up'))) {
                    $frame =~ s/❯ No, exit/  No, exit/g;
                    $frame =~ s/  Yes, I trust this folder/❯ Yes, I trust this folder/g if $tool eq 'claude';
                    $frame =~ s/> No, exit/  No, exit/g;
                    $frame =~ s/  Yes, I trust this folder/> Yes, I trust this folder/g if $tool eq 'agy';
                    $frame =~ s/No, exit/No, keep running/g if $mode eq 'drift';
                    if ($mode eq 'change-dir' || $mode eq 'change-start') {
                        my $meta = get("$dir/meta");
                        $meta =~ s/^work_dir.main=.*$/work_dir.main=$root\/other/m if $mode eq 'change-dir';
                        $meta =~ s/^launch_id.main=.*$/launch_id.main=changed-start/m if $mode eq 'change-start';
                        put("$dir/meta", $meta);
                    }
                }
                draw(); next;
            }
            if ($buffer =~ s/^([\r\n])// && !$pasting) {
                logline("$root/actions", "Enter\n");
                composer() if $mode ne 'remain';
                next;
            }
            my $byte = substr($buffer, 0, 1, '');
            logline("$root/submitted", $byte) if $trusted && $pasting;
        }
    }
    if (-e "$root/drain") { unlink "$root/drain"; put("$root/drained", 'yes'); }
}
"#;

// Refuses one external key, or changes the UI before the SECOND post-nav
// capture. All other tmux operations use the real server unchanged.
const TMUX: &str = r#"#!/usr/bin/perl
use strict; use warnings;
my $root = '__ROOT__'; my $real = '__TMUX__';
sub get { open(my $f, '<', $_[0]) or return ''; local $/; return <$f> // ''; }
sub put { open(my $f, '>', $_[0]) or die; print $f $_[1]; close $f; }
my $mode = get("$root/wrapper");
my $owned = grep { $_ eq "$root/sock" } @ARGV;
if ($owned && grep { $_ eq 'send-keys' } @ARGV) {
    open(my $log, '>>', "$root/attempts") or die; print $log "$ARGV[-1]\n"; close $log;
    if (($mode eq 'refuse-enter' && $ARGV[-1] eq 'Enter') || ($mode eq 'refuse-nav' && $ARGV[-1] eq 'Down')) {
        put("$root/refused", $ARGV[-1]); put("$root/pre-refusal", get("__DIR__/events.jsonl"));
        exit 1;
    }
}
if ($owned && $mode eq 'reserve-fail' && grep { $_ eq 'capture-pane' } @ARGV) {
    my $count = 1 + (get("$root/initial-captures") || 0); put("$root/initial-captures", $count);
    if ($count == 1) { mkdir '__DIR__/events.jsonl.lock' or die 'lock fixture'; put("$root/armed", 'yes'); }
}
if ($owned && $mode eq 'second-no' && grep { $_ eq 'capture-pane' } @ARGV) {
    if (get("$root/actions") =~ /Down/) {
        my $count = 1 + (get("$root/nav-captures") || 0); put("$root/nav-captures", $count);
        if ($count == 2) {
            unlink "$root/repainted"; put("$root/repaint", get("$root/frame"));
            for (1..200) { last if -e "$root/repainted"; select undef,undef,undef,0.01; }
            die 'redraw failed' unless -e "$root/repainted";
        }
    }
}
exec {$real} $real, @ARGV; die 'tmux exec';
"#;

fn text(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path).unwrap_or_default()
}
fn write(path: impl AsRef<Path>, body: impl AsRef<[u8]>) {
    assert!(fs::write(path, body).is_ok(), "setup: fixture write");
}
fn wait_for(what: &str, ready: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(15);
    while Instant::now() < end {
        if ready() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {what}");
}

struct Rig {
    root: PathBuf,
    dir: PathBuf,
    pane: String,
    tool: String,
    daemon: Option<JoinHandle<()>>,
}

impl Rig {
    #[allow(
        clippy::too_many_lines,
        reason = "one fixture setup keeps private server, native fake and ownership proof together"
    )]
    fn new(tag: &str, tool: &str, mode: &str) -> Self {
        let root = super::cli::OwnedScratch::root("at", tag).keep();
        let dir = root.join("sessions/s");
        for p in [
            &dir,
            &root.join("bin"),
            &root.join("tmp"),
            &root.join("other"),
        ] {
            assert!(fs::create_dir_all(p).is_ok(), "setup: directories");
        }
        assert!(
            fs::copy("/usr/bin/perl", root.join("bin").join(tool)).is_ok(),
            "setup: native fake vendor"
        );
        write(root.join("vendor.pl"), VENDOR);
        let folder = fs::canonicalize(&root).unwrap_or_else(|why| panic!("fixture folder: {why}"));
        let frame = if tool == "agy" {
            AGY.replace("/tmp/agy-trust-probe", &folder.display().to_string())
        } else {
            CLAUDE.replace("/tmp/claude-trust-probe", &folder.display().to_string())
        };
        write(root.join("frame"), &frame);
        write(
            root.join("idle"),
            if tool == "agy" { AGY_IDLE } else { CLAUDE_IDLE },
        );
        write(root.join("mode"), mode);
        write(root.join("tool"), tool);
        write(root.join("session-dir"), dir.display().to_string());
        write(root.join("wire"), "");
        write(root.join("actions"), "");
        write(
            root.join("config"),
            "[workspace]\nauto_trust = on\nquota = off\nauto_upgrade = off\ntheme = off\nmotion = off\n",
        );
        let mut rig = Self {
            root,
            dir,
            pane: String::new(),
            tool: tool.to_owned(),
            daemon: None,
        };
        let real_tmux = rig
            .invoke(Invocation::new("which").arg("tmux"), "which")
            .1
            .trim()
            .to_owned();
        assert!(Path::new(&real_tmux).is_absolute(), "setup: real tmux path");
        let wrapper = TMUX
            .replace("__ROOT__", &rig.root.display().to_string())
            .replace("__TMUX__", &real_tmux)
            .replace("__DIR__", &rig.dir.display().to_string());
        write(rig.root.join("bin/tmux"), wrapper);
        assert!(
            fs::set_permissions(rig.root.join("bin/tmux"), fs::Permissions::from_mode(0o755))
                .is_ok()
        );
        let command = format!(
            "exec {}/bin/{tool} {}/vendor.pl {}",
            rig.root.display(),
            rig.root.display(),
            rig.root.display()
        );
        rig.tmux(&[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-x",
            "200",
            "-y",
            "50",
            "-s",
            "s",
            &command,
        ]);
        rig.tmux(&["list-panes", "-t", "s", "-F", "#{pane_id}"])
            .trim()
            .clone_into(&mut rig.pane);
        assert!(rig.pane.starts_with('%'), "setup: pane id");
        rig.tmux(&["set-option", "-p", "-t", &rig.pane, "@ae_slot", "main"]);
        rig.tmux(&["set-option", "-p", "-t", &rig.pane, "@ae_agent", "lead"]);
        rig.tmux(&["set-option", "-t", "s", "@ae_session_uuid", UUID]);
        rig.tmux(&["set-environment", "-t", "s", "AE_SESSION", "s"]);
        write(
            rig.dir.join("meta"),
            format!(
                "schema=2\nsession=s\nsession_id={UUID}\nmode=local\nlayout=vertical\ntmux_server_kind=socket\ntmux_server={}/sock\nseat.main=lead\nprofile.main=fake\nagent_bin.main={tool}\nlaunch_id.main=start-one\nwork_dir={}\nwork_dir.main={}\nconfig={}\nquota_aware=off\nidle_nudge_secs=0\n",
                rig.root.display(),
                folder.display(),
                folder.display(),
                rig.root.join("config").display()
            ),
        );
        rig.replace_meta("origin", &folder.display().to_string());
        rig.replace_meta("main_pane", &rig.pane);
        write(
            rig.dir.join("events.jsonl"),
            "{\"ts\":\"2026-10-11T00:00:00Z\",\"actor\":\"ae:launch\",\"action\":\"launch\"}\n",
        );
        for helper in ["send", "_run"] {
            assert!(symlink(env!("CARGO_BIN_EXE_ae"), rig.dir.join(helper)).is_ok());
        }
        wait_for("fake vendor ready", || rig.root.join("ready").is_file());
        let pid: u32 = text(rig.root.join("ready"))
            .parse()
            .unwrap_or_else(|why| panic!("fake pid: {why}"));
        let table = ae::procs::snapshot().unwrap_or_else(|| panic!("setup: process table"));
        assert!(
            table.iter().any(|p| p.pid == pid
                && Path::new(&p.comm)
                    .file_name()
                    .is_some_and(|name| name == tool)),
            "setup: OS proves the actual tool name"
        );
        let capture = rig.tmux(&["capture-pane", "-p", "-t", &rig.pane]);
        assert!(
            ae::watchdog::human_prompt_class(&capture, tool).is_some(),
            "setup: measured human modal"
        );
        rig
    }

    fn invoke(&self, command: Invocation, tag: &str) -> (i32, String, String) {
        let out = self.root.join(format!("{tag}.out"));
        let err = self.root.join(format!("{tag}.err"));
        let status = raw::run(&command, &self.root, &out, &err)
            .unwrap_or_else(|why| panic!("setup: external invocation: {why}"));
        drop(command);
        let ExitOutcome::Code(code) = status.outcome() else {
            panic!("setup: child signalled");
        };
        (code, text(out), text(err))
    }
    fn tmux(&self, args: &[&str]) -> String {
        let mut call = Invocation::new("tmux")
            .arg("-S")
            .arg(self.root.join("sock"));
        for arg in args {
            call = call.arg(arg);
        }
        let (code, out, err) = self.invoke(call, "tmux");
        assert_eq!(code, 0, "setup: private tmux {args:?}: {err}");
        out
    }
    fn command(&self, verb: &str) -> Invocation {
        let inherited_path =
            std::env::var("PATH").unwrap_or_else(|why| panic!("setup: PATH: {why}"));
        Invocation::new(env!("CARGO_BIN_EXE_ae"))
            .env_cleared()
            .env(
                "PATH",
                format!("{}/bin:{inherited_path}", self.root.display()),
            )
            .env("HOME", &self.root)
            .env("AE_HOME", &self.root)
            .env("CONFIG_FILE", self.root.join("config"))
            .env("TMUX_TMPDIR", self.root.join("tmp"))
            .env("AE_NO_AUTOSTART", "1")
            .env("TERM", "xterm-256color")
            .env("LC_ALL", "en_US.UTF-8")
            .env("TMUX", format!("{},0,0", self.root.join("sock").display()))
            .env("TMUX_PANE", &self.pane)
            .arg(verb)
            .arg(&self.dir)
    }
    fn start(&mut self) {
        assert!(self.daemon.is_none(), "setup: one watchdog");
        let command = self
            .command(ae::cli::WATCHDOG_RUN)
            .arg("--interval")
            .arg("1")
            .arg("--quota-every-secs")
            .arg("0")
            .arg("--idle-nudge-secs")
            .arg("0")
            .arg("--sweep-secs")
            .arg("0")
            .arg("--tg-supervise-secs")
            .arg("0");
        let root = self.root.clone();
        self.daemon = Some(std::thread::spawn(move || {
            let _ = raw::run(
                &command,
                &root,
                &root.join("watch.out"),
                &root.join("watch.err"),
            );
        }));
    }
    fn stop(&mut self) {
        if let Some(handle) = self.daemon.take() {
            let meta = self.dir.join("meta");
            let saved = fs::read(&meta).unwrap_or_else(|why| panic!("setup: saved meta: {why}"));
            fs::remove_file(&meta).unwrap_or_else(|why| panic!("setup: end watchdog: {why}"));
            wait_for("watchdog exit", || handle.is_finished());
            handle
                .join()
                .unwrap_or_else(|_| panic!("setup: watchdog thread"));
            write(meta, saved);
        }
    }
    fn events(&self) -> Vec<Event> {
        text(self.dir.join("events.jsonl"))
            .lines()
            .filter_map(|line| Event::parse_line(line).ok())
            .collect()
    }
    fn count(&self, action: &str) -> usize {
        self.events().iter().filter(|e| e.action == action).count()
    }
    fn named(&self) {
        wait_for("named human prompt", || self.count("human-prompt") > 0);
    }
    fn result(&self) {
        wait_for("trust decision", || {
            self.count("auto-trust") > 0 || self.count("human-prompt") > 0
        });
    }
    fn wire(&self) -> Vec<u8> {
        let _ = fs::remove_file(self.root.join("drained"));
        write(self.root.join("drain"), "yes");
        wait_for("causal stdin drain", || self.root.join("drained").is_file());
        fs::read(self.root.join("wire")).unwrap_or_else(|why| panic!("wire receipt: {why}"))
    }
    fn repaint(&self, frame: &str) {
        let _ = fs::remove_file(self.root.join("repainted"));
        write(self.root.join("repaint"), frame);
        wait_for("repaint", || self.root.join("repainted").is_file());
    }
    fn replace_meta(&self, key: &str, value: &str) {
        let mut body = String::new();
        for line in text(self.dir.join("meta"))
            .lines()
            .filter(|line| !line.starts_with(&format!("{key}=")))
        {
            writeln!(body, "{line}").unwrap_or_else(|why| panic!("fixture meta: {why}"));
        }
        write(self.dir.join("meta"), format!("{body}{key}={value}\n"));
    }
    fn trusted(&self, expected: &[u8]) {
        self.result();
        assert_eq!(
            self.count("auto-trust"),
            1,
            "ON accepts measured folder once: {}",
            text(self.root.join("watch.err"))
        );
        assert_eq!(
            self.wire(),
            expected,
            "only the measured navigation/Enter bytes"
        );
        wait_for("completed trust chat receipt", || {
            self.events().iter().any(|e| {
                e.action == "chat" && e.summary.as_deref().is_some_and(|s| s.contains("trusted"))
            })
        });
        let events = self.events();
        let done = events
            .iter()
            .position(|e| e.action == "auto-trust")
            .unwrap_or_else(|| panic!("trust completion"));
        let reserved = events
            .iter()
            .position(|e| e.action == "auto-trust-reserved")
            .unwrap_or_else(|| panic!("reservation"));
        assert!(
            reserved < done
                && reserved
                    < events
                        .iter()
                        .position(|e| e.action == "auto-trust-key")
                        .unwrap_or_else(|| panic!("key record")),
            "reservation precedes keys and completion"
        );
        let pre = text(self.root.join("pre-key"));
        assert!(
            pre.contains("auto-trust-reserved") && !pre.contains("\"action\":\"auto-trust\""),
            "durable reservation observable BEFORE first byte"
        );
        assert_eq!(
            events[reserved].reference, events[done].reference,
            "same episode"
        );
        let receipt = events
            .iter()
            .filter(|e| {
                e.action == "chat" && e.summary.as_deref().is_some_and(|s| s.contains("trusted"))
            })
            .collect::<Vec<_>>();
        assert_eq!(receipt.len(), 1, "one human chat receipt");
        assert!(
            receipt[0].target.as_deref().unwrap_or_default().is_empty(),
            "targetless chat"
        );
        for e in events.iter().filter(|e| {
            e.action.starts_with("auto-trust")
                || (e.action == "chat"
                    && e.summary.as_deref().is_some_and(|s| s.contains("trusted")))
        }) {
            assert_eq!(
                e.actor, "watchdog",
                "automatic provenance preserves declarations"
            );
            for state in ["done", "waiting-user", "waiting-agent", "blocked"] {
                assert!(
                    !ae::watchdog::ends_quiet(state, e, false, &[]),
                    "{state} stands"
                );
            }
        }
        let words = events[done]
            .summary
            .as_deref()
            .unwrap_or_else(|| panic!("trust summary"));
        for word in [
            self.tool.as_str(),
            "lead",
            &fs::canonicalize(&self.root)
                .unwrap_or_else(|why| panic!("folder: {why}"))
                .display()
                .to_string(),
        ] {
            assert!(words.contains(word), "tool/folder/seat audit: {words}");
        }
        assert_eq!(
            receipt[0].summary.as_deref(),
            Some(words),
            "receipt follows completed record"
        );
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = raw::run(
            &Invocation::new("tmux")
                .arg("-S")
                .arg(self.root.join("sock"))
                .arg("kill-server"),
            &self.root,
            &self.root.join("drop.out"),
            &self.root.join("drop.err"),
        );
        if let Some(handle) = self.daemon.take() {
            let _ = handle.join();
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn measured_yes_and_no_use_only_the_arrow_then_enter_once_across_watchdog_restart() {
    for (tool, yes, keys) in [
        ("claude", false, b"\x1b[B\r".as_slice()),
        ("claude", true, b"\r".as_slice()),
        ("agy", true, b"\r".as_slice()),
        ("agy", false, b"\x1b[A\r".as_slice()),
    ] {
        let mut rig = Rig::new(&format!("once-{tool}-{yes}"), tool, "normal");
        let frame = text(rig.root.join("frame"));
        let frame = if tool == "claude" && yes {
            frame
                .replace("❯ No, exit", "  No, exit")
                .replace("  Yes, I trust this folder", "❯ Yes, I trust this folder")
        } else if tool == "agy" && !yes {
            frame
                .replace("> Yes, I trust this folder", "  Yes, I trust this folder")
                .replace("  No, exit", "> No, exit")
        } else {
            frame
        };
        write(rig.root.join("frame"), &frame);
        rig.repaint(&frame);
        rig.start();
        rig.trusted(keys);
        rig.stop();
        rig.repaint(&frame);
        rig.start();
        rig.named();
        assert_eq!(rig.wire(), keys, "spent start survives watchdog restart");
        assert_eq!(rig.count("auto-trust-reserved"), 1, "no second reservation");
        assert_eq!(rig.count("auto-trust"), 1, "no second completion");
        rig.stop();
        let old_pid = text(rig.root.join("ready"));
        fs::remove_file(rig.root.join("ready")).expect("old start receipt");
        let command = format!(
            "exec {}/bin/{tool} {}/vendor.pl {}",
            rig.root.display(),
            rig.root.display(),
            rig.root.display()
        );
        rig.tmux(&["respawn-pane", "-k", "-t", &rig.pane, &command]);
        wait_for("distinct tool start", || rig.root.join("ready").is_file());
        assert_ne!(
            text(rig.root.join("ready")),
            old_pid,
            "OS gives new start a distinct episode"
        );
        rig.start();
        wait_for("second start accepted", || rig.count("auto-trust") == 2);
        assert_eq!(
            rig.wire(),
            [keys, keys].concat(),
            "new process is eligible despite same launch_id"
        );
        assert_eq!(rig.count("auto-trust-reserved"), 2);
    }
}

#[test]
fn global_default_and_grammar_control_answering_project_cannot_vote() {
    for (tag, global, project, allowed) in [
        ("default", "", "off", true),
        ("on", "auto_trust = on\n", "off", true),
        ("off", "auto_trust = off\n", "on", false),
        ("bad", "auto_trust = yes\n", "on", false),
        ("case", "auto_trust = ON\n", "on", false),
        (
            "duplicate",
            "auto_trust = on\nauto_trust = off\n",
            "on",
            false,
        ),
    ] {
        let mut rig = Rig::new(tag, "agy", "normal");
        write(
            rig.root.join("config"),
            format!("[workspace]\n{global}quota = off\nauto_upgrade = off\ntheme = off\n"),
        );
        let local = rig.root.join("project.conf");
        write(&local, format!("[workspace]\nauto_trust = {project}\n"));
        rig.replace_meta("local_config", &local.display().to_string());
        rig.start();
        rig.result();
        if allowed {
            rig.trusted(b"\r");
        } else {
            assert!(
                rig.wire().is_empty(),
                "off/invalid global sends NOTHING, local cannot enable: {tag}"
            );
            assert_eq!(rig.count("human-prompt"), 1, "off names as before");
        }
    }
}

#[test]
fn other_questions_extra_options_order_and_quoted_blocks_never_receive_keys() {
    for tag in [
        "question", "extra", "order", "quoted", "heading", "prose", "tilde",
    ] {
        let mut rig = Rig::new(tag, "agy", "normal");
        let frame = text(rig.root.join("frame"));
        let frame = match tag {
            "question" => frame.replace(
                "Do you trust the contents of this project?",
                "Allow this tool to execute a command?",
            ),
            "extra" => frame.replace("> Yes", "  Allow all future commands\n> Yes"),
            "order" => frame.replace(
                "> Yes, I trust this folder\n  No, exit",
                "  No, exit\n> Yes, I trust this folder",
            ),
            "quoted" => format!(
                "{}\nAllow this tool to execute a command?\n\n> Allow once\n  Deny\n\n↑/↓ Navigate · enter Confirm\n",
                frame.trim_end()
            ),
            "heading" => frame.replace("Accessing workspace:", "Tool output:"),
            "prose" => frame.replace(
                "requires permission to read, edit, and execute files here.",
                "requires permission to execute any command.",
            ),
            _ => frame.replace(
                &fs::canonicalize(&rig.root)
                    .expect("folder")
                    .display()
                    .to_string(),
                "~",
            ),
        };
        rig.repaint(&frame);
        rig.start();
        rig.named();
        assert!(
            rig.wire().is_empty(),
            "only one COMPLETE measured trust identity is answerable: {tag}"
        );
        assert_eq!(rig.count("auto-trust"), 0);
    }
}

#[test]
fn folder_equality_is_canonical_equality_not_containment_or_pane_cwd() {
    for (tag, alias) in [("mismatch", false), ("alias", true)] {
        let mut rig = Rig::new(tag, "claude", "normal");
        let folder = if alias {
            let p = rig.root.join("alias");
            symlink(&rig.root, &p).expect("canonical alias");
            p
        } else {
            rig.root.join("other")
        };
        rig.replace_meta("work_dir.main", &folder.display().to_string());
        rig.start();
        rig.result();
        if alias {
            rig.trusted(b"\x1b[B\r");
        } else {
            assert!(
                rig.wire().is_empty(),
                "different recorded folder never answered"
            );
        }
    }
}

#[test]
fn navigation_requires_two_fresh_yes_reads_and_unchanged_start_and_folder() {
    for mode in [
        "stay-no",
        "gone",
        "drift",
        "change-dir",
        "change-start",
        "second-no",
    ] {
        let mut rig = Rig::new(
            mode,
            "claude",
            if mode == "second-no" { "normal" } else { mode },
        );
        if mode == "second-no" {
            write(rig.root.join("wrapper"), mode);
        }
        rig.start();
        wait_for("navigation or old naming", || {
            !text(rig.root.join("actions")).is_empty() || rig.count("human-prompt") > 0
        });
        wait_for("stopped navigation", || {
            rig.count("auto-trust-failed") > 0 || rig.count("human-prompt") > 0
        });
        assert_eq!(
            rig.wire(),
            b"\x1b[B",
            "one arrow; changed/gone/not-yes proof forbids Enter: {mode}"
        );
        assert_eq!(
            rig.count("auto-trust"),
            0,
            "never report trusted on failed proof"
        );
        assert_eq!(
            rig.count("auto-trust-reserved"),
            1,
            "failed navigation stays spent"
        );
    }
}

#[test]
fn failed_keys_and_a_modal_remaining_after_enter_never_report_success() {
    for (mode, keys) in [
        ("refuse-nav", b"".as_slice()),
        ("refuse-enter", b"\x1b[B".as_slice()),
        ("remain", b"\x1b[B\r".as_slice()),
    ] {
        let mut rig = Rig::new(
            mode,
            "claude",
            if mode == "remain" { mode } else { "normal" },
        );
        write(rig.root.join("wrapper"), mode);
        rig.start();
        wait_for("failure or old naming", || {
            rig.count("auto-trust-failed") > 0 || rig.count("human-prompt") > 0
        });
        assert_eq!(
            rig.wire(),
            keys,
            "wire reflects actual transport outcome: {mode}"
        );
        assert_eq!(rig.count("auto-trust"), 0, "no false completion");
        assert_eq!(
            rig.count("auto-trust-failed"),
            1,
            "failure has durable step/reason"
        );
        assert_eq!(
            rig.count("auto-trust-reserved"),
            1,
            "failure spends episode"
        );
        assert!(
            !rig.events().iter().any(|e| e.action == "chat"
                && e.summary.as_deref().is_some_and(|s| s.contains("trusted"))),
            "no success chat"
        );
        if mode != "remain" {
            assert!(
                text(rig.root.join("pre-refusal")).contains("auto-trust-reserved"),
                "reservation before attempted refused key"
            );
        }
    }
}

#[test]
fn spent_corrupt_and_unreadable_journals_fail_closed() {
    for case in [
        "reserved",
        "bad-action",
        "unrelated",
        "directory",
        "symlink",
        "unreadable",
    ] {
        let mut rig = Rig::new(case, "agy", "normal");
        let journal = rig.dir.join("events.jsonl");
        match case {
            "reserved" => {
                let pid = text(rig.root.join("ready"));
                write(
                    &journal,
                    format!(
                        "{{\"ts\":\"2026-10-11T00:00:00Z\",\"actor\":\"watchdog\",\"action\":\"auto-trust-reserved\",\"target\":\"lead\",\"target_slot\":\"main\",\"ref\":\"main:start-one:{pid}\"}}\n"
                    ),
                );
            }
            "bad-action" => write(
                &journal,
                "{\"ts\":\"bad\",\"actor\":\"watchdog\",\"action\" : \"auto-trust-reserved\",\"ref\":\"live-episode\"}\n",
            ),
            "unrelated" => write(&journal, "an unrelated damaged event\n"),
            "directory" => {
                fs::remove_file(&journal).expect("fixture journal");
                fs::create_dir(&journal).expect("nonregular journal");
            }
            "unreadable" => {
                fs::set_permissions(&journal, fs::Permissions::from_mode(0o000))
                    .expect("unreadable journal");
            }
            _ => {
                fs::remove_file(&journal).expect("fixture journal");
                write(rig.root.join("elsewhere"), "");
                symlink(rig.root.join("elsewhere"), &journal).expect("symlink journal");
            }
        }
        rig.start();
        if matches!(case, "directory" | "symlink" | "unreadable") {
            wait_for("journal refusal diagnostic", || {
                !text(rig.root.join("watch.err")).is_empty()
            });
            std::thread::sleep(Duration::from_secs(3));
        } else {
            rig.named();
        }
        assert!(
            rig.wire().is_empty(),
            "unknown/spent ledger never turns into unspent: {case}"
        );
        assert_eq!(rig.count("auto-trust"), 0);
    }
}

#[test]
fn a_foreign_send_lock_names_once_then_retries_after_release() {
    let mut rig = Rig::new("busy-lock", "agy", "normal");
    let locks = rig.root.join("sessions/.locks");
    fs::create_dir_all(&locks).expect("locks");
    let lock = locks.join(format!("send-lock-{}", rig.pane.replace('%', "_")));
    let held = ae::store::lock(&lock, Duration::ZERO).expect("foreign sender holds target lock");
    rig.start();
    rig.named();
    assert!(
        rig.wire().is_empty(),
        "no first key while other sender holds lock"
    );
    assert_eq!(rig.count("auto-trust-reserved"), 0, "NotYet spends nothing");
    drop(held);
    wait_for("late answer", || rig.count("auto-trust") > 0);
    rig.trusted(b"\r");
    wait_for("prompt cleared", || rig.count("human-prompt-cleared") == 1);
    assert_eq!(rig.count("human-prompt"), 1, "retry never repeats naming");
}

#[test]
fn a_frozen_modal_without_a_live_harness_or_unique_slot_never_receives_a_key() {
    for case in ["shell", "uuid", "duplicate"] {
        let mut rig = Rig::new(case, "agy", "normal");
        match case {
            "shell" => {
                rig.tmux(&[
                    "respawn-pane",
                    "-k",
                    "-t",
                    &rig.pane,
                    &format!("cat {}/frame; exec sleep 60", rig.root.display()),
                ]);
            }
            "uuid" => {
                rig.tmux(&[
                    "set-option",
                    "-t",
                    "s",
                    "@ae_session_uuid",
                    "different-incarnation",
                ]);
            }
            _ => {
                let p = rig.tmux(&[
                    "new-window",
                    "-d",
                    "-P",
                    "-F",
                    "#{pane_id}",
                    "-t",
                    "s:",
                    "sleep 60",
                ]);
                rig.tmux(&["set-option", "-p", "-t", p.trim(), "@ae_slot", "main"]);
            }
        }
        rig.start();
        std::thread::sleep(Duration::from_secs(6));
        if case == "shell" {
            assert!(
                text(rig.root.join("attempts")).is_empty(),
                "dead tool cannot consume retained modal"
            );
        } else {
            assert!(
                rig.wire().is_empty(),
                "no key with unproven ownership: {case}"
            );
        }
        assert_eq!(rig.count("auto-trust"), 0);
    }
}

#[test]
fn a_refused_reservation_sends_zero_keys() {
    let mut rig = Rig::new("reserve-write", "claude", "normal");
    write(rig.root.join("wrapper"), "reserve-fail");
    rig.start();
    wait_for("reservation refusal or old naming", || {
        rig.root.join("armed").is_file() || rig.count("human-prompt") > 0
    });
    assert!(
        rig.root.join("armed").is_file(),
        "first watchdog capture arms the append refusal"
    );
    wait_for("write refusal named", || {
        !text(rig.root.join("watch.err")).is_empty()
    });
    assert!(
        rig.wire().is_empty(),
        "reservation append failure sends ZERO bytes"
    );
    assert_eq!(rig.count("auto-trust-reserved"), 0);
}

fn spawn_wait(value: &str) -> (Rig, PathBuf, i32, String, String) {
    let rig = Rig::new("spawn-wait", "claude", "normal");
    let child = rig.root.join("child");
    fs::create_dir(&child).unwrap_or_else(|why| panic!("child vendor storage: {why}"));
    fs::create_dir(child.join("other"))
        .unwrap_or_else(|why| panic!("child alternative folder: {why}"));
    for name in ["frame", "idle", "tool", "session-dir"] {
        write(
            child.join(name),
            fs::read(rig.root.join(name)).unwrap_or_else(|why| panic!("child fixture: {why}")),
        );
    }
    write(child.join("mode"), "normal");
    write(child.join("wire"), "");
    let config = format!(
        "[profiles]\nfake = \"{}/bin/claude {}/vendor.pl {}\"\n[workspace]\nmain = lead\nlayout = vertical\nauto_trust = {value}\nquota = off\nauto_upgrade = off\ntheme = off\n",
        rig.root.display(),
        rig.root.display(),
        child.display()
    );
    write(rig.root.join("config"), config);
    let (code, out, err) = rig.invoke(
        rig.command(ae::cli::SPAWN)
            .arg("helper")
            .arg("--using")
            .arg("fake")
            .arg("--")
            .arg("original trust brief"),
        "spawn",
    );
    (rig, child, code, out, err)
}

#[test]
fn spawn_brief_wait_answers_without_a_watchdog_and_delivers_the_original_brief() {
    let (rig, child, code, out, err) = spawn_wait("on");
    assert_eq!(
        code, 0,
        "in-wait owner answers with watchdog absent: {out}\n{err}"
    );
    assert_eq!(
        rig.count("watchdog-start"),
        0,
        "in-wait answer needs no daemon"
    );
    assert_eq!(rig.count("auto-trust"), 1, "spawn episode completed once");
    assert!(
        text(child.join("submitted")).contains("original trust brief"),
        "original brief reaches composer after trust"
    );
    assert_eq!(
        text(child.join("actions"))
            .lines()
            .take(2)
            .collect::<Vec<_>>(),
        ["Down", "Enter"]
    );
    let event = rig
        .events()
        .into_iter()
        .find(|e| e.action == "auto-trust")
        .expect("spawn trust record");
    assert_eq!(
        event.actor, "watchdog",
        "in-wait automatic actor preserves quiet states"
    );
    assert_eq!(event.target.as_deref(), Some("helper"));
    assert_eq!(event.target_slot.value(), Some("spawned.0"));
}

fn spawn_wait_refuses(value: &str) {
    let (rig, child, code, out, err) = spawn_wait(value);
    assert_eq!(code, 1, "{value}: brief stays undelivered: {out}\n{err}");
    assert_eq!(rig.count("watchdog-start"), 0, "no watchdog answers");
    assert!(
        fs::read(child.join("wire"))
            .unwrap_or_else(|why| panic!("child wire: {why}"))
            .is_empty()
    );
    assert!(
        text(child.join("submitted")).is_empty(),
        "no brief delivered"
    );
    assert!(
        rig.events()
            .iter()
            .all(|e| !e.action.starts_with("auto-trust"))
    );
    let kept = ae::brief_retry::read(&rig.dir, "spawned.0")
        .unwrap_or_else(|| panic!("undelivered brief retained"))
        .unwrap_or_else(|why| panic!("valid undelivered brief: {why:?}"));
    assert!(kept.body.contains("original trust brief"));
}

#[test]
fn spawn_brief_wait_with_off_never_answers_or_delivers() {
    spawn_wait_refuses("off");
}

#[test]
fn spawn_brief_wait_with_invalid_value_never_answers_or_delivers() {
    spawn_wait_refuses("INVALID");
}
