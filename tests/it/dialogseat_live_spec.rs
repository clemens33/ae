//! Frozen acceptance (dialogseat, live half): a Claude seat held at the
//! settings-variant folder-trust dialog, drawn in a pane SHORTER than the
//! modal, is reported by the relay at once, by the watchdog as needs-human and
//! by a launch and a resume, and no key ever reaches the dialog with
//! `auto_trust = on`. Real private tmux, real `ae` binary and owners; only the
//! vendor UI is fake: a native perl copy named `claude` that draws a measured
//! frame (`tests/fixtures/dialogseat/`) and records every input byte.
//!
//! This half names no new type, so it runs unchanged against the base: every
//! wording pin is a literal. The kind words ("trust") and the action word
//! ("attach") are the ones `dialogseat_spec` pins on `DialogKind`.
#![allow(
    clippy::disallowed_methods,
    reason = "acceptance fixtures own private filesystem, environment and tmux state"
)]

use std::cell::RefCell;
use std::fs;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::cli::OwnedScratch;
use super::parity::{
    Invocation,
    capture::{ExitOutcome, raw},
};
use super::watchdog_step::Stepper;
use ae::deliver;
use ae::events::Event;
use ae::inventory::ServerId;
use ae::meta::Selector;
use ae::tool::Composed;

/// 40x12, 1 permission: rule and question are above the visible screen.
const SHORT_40: &str =
    include_str!("../fixtures/dialogseat/claude-2.1.330-settings-40x12.history.txt");
/// 80x24, 40 permissions + hooks + `.mcp.json`: 27 rows from rule to hint.
const MCP40_80: &str =
    include_str!("../fixtures/dialogseat/claude-2.1.330-mcp40-80x24.history.txt");
const UUID: &str = "0199c0de-bbbb-4890-abcd-ef0123456789";
const TITLE: &str = "Accessing workspace:";
const HINT: &str = "Enter to confirm · Esc to cancel";
const LOADS: &str = "Trusting this folder loads .claude/settings.json";
/// The measured warning head, as the 80-column frame draws it on one row.
const PERMS_40: &str = "pre-approves 40 tool permissions";
/// The deferral a quiet gate would spend on an occupied composer; a refusal
/// "at once" ends in well under half of it.
const DEFER: Duration = Duration::from_mins(1);
/// The bound of a wait on evidence; a passing run spends none of it (#234).
const WITNESS_WAIT: Duration = Duration::from_mins(1);
/// A safety against a hung launch, never the measure of "bounded" (#234).
const LAUNCH_HANG: Duration = Duration::from_mins(10);
/// A launch-wide observation spends pane reads, not seconds: the fast path
/// reads a pane a handful of times, a per-seat long poll reads it 60+ times.
const LAUNCH_READS: usize = 60;

/// The fake vendor: draws `<root>/frame` from the top of a cleared screen, so a
/// frame taller than the pane scrolls its first rows into history exactly as
/// the real harness does, then records every input byte in `<root>/wire`.
/// `<root>/drain` is acknowledged by `<root>/drained` after the bytes already
/// queued on its terminal were read.
const VENDOR: &str = r#"use strict;
use warnings;
my $root = shift @ARGV;
system('stty raw -echo') == 0 or die 'stty';
binmode STDIN; binmode STDOUT; $| = 1;
sub get { my ($p) = @_; open(my $f, '<:raw', $p) or return ''; local $/; return <$f> // ''; }
sub put { my ($p,$s) = @_; open(my $f, '>:raw', $p) or die $p; print $f $s; close $f; }
sub logline { my ($p,$s) = @_; open(my $f, '>>:raw', $p) or die $p; print $f $s; close $f; }
my $frame = get("$root/frame"); $frame =~ s/\r?\n/\r\n/g; $frame =~ s/\r\n\z//;
print "\e[H\e[2J", $frame;
put("$root/ready.tmp", "$$"); rename("$root/ready.tmp", "$root/ready") or die 'ready';
while (1) {
    my $bits = ''; vec($bits, fileno(STDIN), 1) = 1;
    if (select($bits, undef, undef, 0.01) > 0) {
        my $bytes; last unless sysread(STDIN, $bytes, 4096);
        logline("$root/wire", $bytes);
        next;
    }
    if (-e "$root/drain") { unlink "$root/drain"; put("$root/drained", 'yes'); }
}
"#;

/// `<root>/bin/tmux`: counts the pane reads on this fixture's socket and keeps
/// the watchdog's notices, then runs the real tmux unchanged.
const TMUX: &str = r#"#!/usr/bin/perl
use strict; use warnings;
my $root = '__ROOT__'; my $real = '__TMUX__';
my $owned = grep { $_ eq "$root/sock" } @ARGV;
if ($owned && grep { $_ eq 'capture-pane' } @ARGV) {
    open(my $log, '>>', "$root/captures") or die; print $log "capture\n"; close $log;
}
if ($owned && (grep { $_ eq 'display-message' } @ARGV) && $ARGV[-1] =~ /^\[ae watchdog\] /) {
    open(my $log, '>>', "$root/notifications") or die; print $log "$ARGV[-1]\n"; close $log;
}
exec {$real} $real, @ARGV; die 'tmux exec';
"#;

fn text(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn write(path: impl AsRef<Path>, body: impl AsRef<[u8]>) {
    assert!(fs::write(path, body).is_ok(), "setup: fixture write");
}

fn wait_within(what: &str, within: Duration, ready: impl Fn() -> bool) {
    let end = Instant::now() + within;
    while Instant::now() < end {
        if ready() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {what}");
}

fn real_tmux(root: &Path) -> String {
    let (out, err) = (root.join("which.out"), root.join("which.err"));
    let status = raw::run(&Invocation::new("which").arg("tmux"), root, &out, &err)
        .unwrap_or_else(|why| panic!("setup: which tmux: {why}"));
    assert!(
        matches!(status.outcome(), ExitOutcome::Code(0)),
        "setup: which"
    );
    let real = text(&out).trim().to_owned();
    assert!(Path::new(&real).is_absolute(), "setup: real tmux path");
    real
}

/// The native fake named `claude`, the vendor script and the counting tmux
/// wrapper, all under `<root>/bin` and `<root>`.
fn install_fakes(root: &Path, frame: &str) {
    for dir in [root.join("bin"), root.join("tmp")] {
        assert!(fs::create_dir_all(dir).is_ok(), "setup: directories");
    }
    assert!(
        fs::copy("/usr/bin/perl", root.join("bin/claude")).is_ok(),
        "setup: native fake vendor named claude"
    );
    write(root.join("vendor.pl"), VENDOR);
    write(root.join("frame"), frame);
    write(root.join("wire"), "");
    let wrapper = TMUX
        .replace("__ROOT__", &root.display().to_string())
        .replace("__TMUX__", &real_tmux(root));
    write(root.join("bin/tmux"), wrapper);
    assert!(
        fs::set_permissions(root.join("bin/tmux"), fs::Permissions::from_mode(0o755)).is_ok(),
        "setup: executable wrapper"
    );
}

/// Every byte the fake read, after the bytes queued on its terminal drained.
fn wire(root: &Path) -> String {
    let _ = fs::remove_file(root.join("drained"));
    write(root.join("drain"), "drain");
    wait_within("the fake drained its input", WITNESS_WAIT, || {
        root.join("drained").is_file()
    });
    text(root.join("wire"))
}

fn pane_reads(root: &Path) -> usize {
    text(root.join("captures")).lines().count()
}

fn events(path: &Path) -> Vec<Event> {
    text(path)
        .lines()
        .filter_map(|line| Event::parse_line(line).ok())
        .collect()
}

fn no_auto_answer(events: &[Event]) {
    assert!(
        events.iter().all(|e| !e.action.starts_with("auto-trust")),
        "auto_trust reserved or answered an unmeasured variant: {events:?}"
    );
}

/// One seat, `main` = agent `lead` on tool `claude`, in session `s` of a
/// private server, its pane `cols` x `rows` drawing `frame`.
struct Seat {
    scratch: OwnedScratch,
    root: PathBuf,
    dir: PathBuf,
    pane: String,
    daemon: Option<JoinHandle<()>>,
    stepper: RefCell<Option<Stepper>>,
}

impl Seat {
    fn new(tag: &str, cols: u16, rows: u16, frame: &str) -> Self {
        let mut scratch = OwnedScratch::root("ds", tag);
        let root =
            fs::canonicalize(scratch.path()).unwrap_or_else(|why| panic!("setup: scratch: {why}"));
        scratch.add_tmux_server(root.join("sock"));
        let dir = root.join("sessions/s");
        assert!(fs::create_dir_all(&dir).is_ok(), "setup: session dir");
        install_fakes(&root, frame);
        write(
            root.join("config"),
            "[workspace]\nauto_trust = on\nquota = off\nauto_upgrade = off\ntheme = off\nmotion = off\n",
        );
        let mut seat = Self {
            scratch,
            root,
            dir,
            pane: String::new(),
            daemon: None,
            stepper: RefCell::new(None),
        };
        let command = format!("exec {0}/bin/claude {0}/vendor.pl {0}", seat.root.display());
        let (cols, rows) = (cols.to_string(), rows.to_string());
        seat.tmux(&[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-x",
            &cols,
            "-y",
            &rows,
            "-s",
            "s",
            &command,
        ]);
        seat.tmux(&["set-option", "-t", "s", "status", "off"]);
        seat.tmux(&["list-panes", "-t", "s", "-F", "#{pane_id}"])
            .trim()
            .clone_into(&mut seat.pane);
        assert!(seat.pane.starts_with('%'), "setup: pane id");
        let pane = seat.pane.clone();
        seat.tmux(&["set-option", "-p", "-t", &pane, "@ae_slot", "main"]);
        seat.tmux(&["set-option", "-p", "-t", &pane, "@ae_agent", "lead"]);
        seat.tmux(&["set-option", "-t", "s", "@ae_session_uuid", UUID]);
        seat.tmux(&["set-environment", "-t", "s", "AE_SESSION", "s"]);
        let folder = seat.root.display().to_string();
        write(
            seat.dir.join("meta"),
            format!(
                "schema=2\nsession=s\nsession_id={UUID}\nmode=local\nlayout=vertical\ntmux_server_kind=socket\ntmux_server={0}/sock\nseat.main=lead\nprofile.main=fake\nagent_bin.main=claude\nlaunch_id.main=start-one\nwork_dir={folder}\nwork_dir.main={folder}\nconfig={0}/config\nquota_aware=off\nidle_nudge_secs=0\norigin={folder}\nmain_pane={pane}\n",
                seat.root.display()
            ),
        );
        write(
            seat.dir.join("events.jsonl"),
            "{\"ts\":\"2026-11-21T00:00:00Z\",\"actor\":\"ae:launch\",\"action\":\"launch\"}\n",
        );
        for helper in ["send", "_run"] {
            assert!(
                symlink(env!("CARGO_BIN_EXE_ae"), seat.dir.join(helper)).is_ok(),
                "setup: helper link"
            );
        }
        wait_within("fake vendor ready", WITNESS_WAIT, || {
            seat.root.join("ready").is_file()
        });
        // The frame is drawn whole before ready; the setup proves the CLIPPED
        // shape: the hint on screen, the title only in history.
        let screen = seat.tmux(&["capture-pane", "-p", "-t", &pane]);
        assert!(
            screen.contains(HINT) && !screen.contains(TITLE),
            "setup: the modal is taller than the pane: {screen}"
        );
        let history = seat.tmux(&["capture-pane", "-p", "-S", "-", "-t", &pane]);
        assert!(history.contains(TITLE), "setup: the title is in history");
        seat
    }

    fn server(&self) -> ServerId {
        ServerId::Selected(Selector::Socket(self.root.join("sock")))
    }

    fn tmux(&self, args: &[&str]) -> String {
        let mut call = Invocation::new("tmux")
            .arg("-S")
            .arg(self.root.join("sock"));
        for arg in args {
            call = call.arg(arg);
        }
        let (out, err) = (self.root.join("tmux.out"), self.root.join("tmux.err"));
        let status = raw::run(&call, &self.root, &out, &err)
            .unwrap_or_else(|why| panic!("setup: tmux {args:?}: {why}"));
        assert!(
            matches!(status.outcome(), ExitOutcome::Code(0)),
            "setup: tmux {args:?}: {}",
            text(&err)
        );
        text(&out)
    }

    fn events(&self) -> Vec<Event> {
        events(&self.dir.join("events.jsonl"))
    }

    fn watchdog(&mut self) {
        static STARTS: AtomicU64 = AtomicU64::new(0);
        let channel = format!("ds-step-{}", STARTS.fetch_add(1, Ordering::Relaxed));
        let path = std::env::var("PATH").unwrap_or_else(|why| panic!("setup: PATH: {why}"));
        let mut command = Invocation::new(env!("CARGO_BIN_EXE_ae"))
            .env_cleared()
            .env("PATH", format!("{}/bin:{path}", self.root.display()))
            .env("HOME", &self.root)
            .env("AE_HOME", &self.root)
            .env("CONFIG_FILE", self.root.join("config"))
            .env("TMUX_TMPDIR", self.root.join("tmp"))
            .env("AE_NO_AUTOSTART", "1")
            .env("TERM", "xterm-256color")
            .env("LC_ALL", "en_US.UTF-8")
            .env("TMUX", format!("{},0,0", self.root.join("sock").display()))
            .env("TMUX_PANE", &self.pane)
            .arg(ae::cli::WATCHDOG_RUN)
            .arg(&self.dir);
        for arg in [
            "--interval",
            "1",
            "--quota-every-secs",
            "0",
            "--idle-nudge-secs",
            "0",
            "--sweep-secs",
            "0",
            "--tg-supervise-secs",
            "0",
            "--step-channel",
            channel.as_str(),
        ] {
            command = command.arg(arg);
        }
        let root = self.root.clone();
        self.daemon = Some(std::thread::spawn(move || {
            let _ = raw::run(
                &command,
                &root,
                &root.join("watch.out"),
                &root.join("watch.err"),
            );
        }));
        let mut stepper = Stepper::new(&self.root.join("sock"), &channel);
        stepper.ready();
        *self.stepper.get_mut() = Some(stepper);
    }

    fn step_until(&self, what: &str, ready: impl Fn() -> bool) {
        let mut stepper = self.stepper.borrow_mut();
        let Some(stepper) = stepper.as_mut() else {
            panic!("setup: a stepped watchdog");
        };
        assert!(
            stepper.step_until_within(WITNESS_WAIT, ready).is_some(),
            "timed out waiting for {what}"
        );
    }

    fn stop_watchdog(&mut self) {
        if let Some(handle) = self.daemon.take() {
            let meta = self.dir.join("meta");
            let saved = fs::read(&meta).unwrap_or_else(|why| panic!("setup: meta: {why}"));
            fs::remove_file(&meta).unwrap_or_else(|why| panic!("setup: end watchdog: {why}"));
            drop(self.stepper.get_mut().take());
            wait_within("watchdog exit", WITNESS_WAIT, || handle.is_finished());
            handle
                .join()
                .unwrap_or_else(|_| panic!("setup: watchdog thread"));
            write(meta, saved);
        }
    }
}

impl Drop for Seat {
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
        drop(self.stepper.get_mut().take());
        if let Some(handle) = self.daemon.take() {
            let _ = handle.join();
        }
        let _ = self.scratch.path();
    }
}

#[test]
fn dialogseat_a_relay_to_a_seat_held_at_a_clipped_settings_dialog_is_refused_at_once() {
    for (tag, cols, rows, frame) in [
        ("relay-40", 40, 12, SHORT_40),
        ("relay-80", 80, 24, MCP40_80),
    ] {
        let seat = Seat::new(tag, cols, rows, frame);
        let server = seat.server();
        let before = seat.events().len();
        let request = deliver::Request {
            dir: &seat.dir,
            server: &server,
            pane: &seat.pane,
            logged_target: "lead",
            target_session: "s",
            pane_slot: "main",
            own_session: "other",
            action: "send",
            reference: "",
            actor: "lead",
            body: "dialogseat acceptance: must never reach a trust dialog",
            shape: deliver::Shape::Send,
            defer: DEFER,
            composed: Composed::NONE,
        };
        let started = Instant::now();
        let mut err = Vec::new();
        let result = deliver::deliver(&request, &mut err)
            .unwrap_or_else(|why| panic!("{tag}: delivery io: {why}"));
        let spent = started.elapsed();
        let stderr = String::from_utf8_lossy(&err).into_owned();
        assert!(result.is_err(), "{tag}: not delivered: {stderr}");
        assert!(
            spent < DEFER / 2,
            "{tag}: refused at once, not after the quiet deferral: {spent:?}: {stderr}"
        );
        assert!(
            stderr.contains(" REFUSED — ") && !stderr.contains("ABANDONED"),
            "{tag}: the exact refusal: {stderr}"
        );
        let lower = stderr.to_lowercase();
        assert!(
            lower.contains("trust") && lower.contains("attach"),
            "{tag}: the kind and the one human action: {stderr}"
        );
        assert!(stderr.contains(LOADS), "{tag}: the warning rows: {stderr}");
        assert!(
            !stderr.contains('\u{1b}'),
            "{tag}: terminal-safe: {stderr:?}"
        );
        assert!(
            wire(&seat.root).is_empty(),
            "{tag}: NO byte reached the pane"
        );
        let delta = &seat.events()[before..];
        assert!(
            delta.iter().all(|e| e.action != "send"),
            "{tag}: no delivery recorded: {delta:?}"
        );
        no_auto_answer(delta);
    }
}

#[test]
fn dialogseat_the_watchdog_names_a_clipped_settings_dialog_needs_human_and_sends_no_key() {
    let mut seat = Seat::new("watch-80", 80, 24, MCP40_80);
    seat.watchdog();
    seat.step_until(
        "a named human prompt and a published prompt verdict",
        || {
            seat.events().iter().any(|e| e.action == "human-prompt")
                && seat
                    .tmux(&["show-options", "-qv", "-t", "s", ae::theme::AGENTS_OPTION])
                    .contains(":prompt:")
        },
    );
    seat.stop_watchdog();
    let events = seat.events();
    let summaries: String = events
        .iter()
        .filter(|e| e.action == "human-prompt")
        .filter_map(|e| e.summary.clone())
        .collect::<Vec<_>>()
        .join("\n");
    let told = format!(
        "{summaries}\n{}\n{}",
        text(seat.root.join("notifications")),
        text(seat.root.join("watch.err"))
    );
    assert!(
        told.contains(PERMS_40) && told.contains(LOADS),
        "the watchdog reports the warning rows: {told}"
    );
    assert!(
        told.to_lowercase().contains("trust"),
        "the watchdog names the kind: {told}"
    );
    assert!(wire(&seat.root).is_empty(), "NO key with auto_trust = on");
    no_auto_answer(&events);
}

/// One `ae --local lw --no-attach` against the fake seat: a fresh launch the
/// first time, a resume after its session is killed.
fn launch(root: &Path, home: &Path, sock: &Path, round: &str) -> String {
    let path = std::env::var("PATH").unwrap_or_else(|why| panic!("setup: PATH: {why}"));
    let command = Invocation::new(env!("CARGO_BIN_EXE_ae"))
        .env_cleared()
        .env("PATH", format!("{}/bin:{path}", root.display()))
        .env("HOME", root)
        .env("AE_HOME", home)
        .env("CONFIG_FILE", root.join("config"))
        .env("TMUX_TMPDIR", root.join("tmp"))
        .env("AE_NO_AUTOSTART", "1")
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", sock)
        .env("TERM", "xterm-256color")
        .env("LC_ALL", "en_US.UTF-8")
        .arg("--local")
        .arg("lw")
        .arg("--no-attach");
    for stale in ["ready", "captures"] {
        let _ = fs::remove_file(root.join(stale));
    }
    write(root.join("wire"), "");
    let (cwd, out, err) = (
        root.join("project"),
        root.join(format!("{round}.out")),
        root.join(format!("{round}.err")),
    );
    let (out_path, err_path) = (out.clone(), err.clone());
    let launched = std::thread::spawn(move || {
        raw::run(&command, &cwd, &out, &err)
            .unwrap_or_else(|why| panic!("setup: launch: {why}"))
            .outcome()
    });
    wait_within("fake vendor ready", LAUNCH_HANG, || {
        root.join("ready").is_file() || launched.is_finished()
    });
    assert!(
        root.join("ready").is_file(),
        "{round}: the launch ended before the fake drew: {}",
        text(&err_path)
    );
    wait_within("the launch to end", LAUNCH_HANG, || {
        let reads = pane_reads(root);
        assert!(
            reads < LAUNCH_READS,
            "{round}: one bounded observation: {reads} pane reads"
        );
        launched.is_finished()
    });
    let reads = pane_reads(root);
    assert!(reads < LAUNCH_READS, "{round}: {reads} pane reads");
    let outcome = launched
        .join()
        .unwrap_or_else(|_| panic!("setup: launch thread"));
    assert!(
        matches!(outcome, ExitOutcome::Code(0)),
        "{round}: the session did start, so the launch exits 0: {}",
        text(&err_path)
    );
    format!("{}\n{}", text(out_path), text(err_path))
}

fn reported(round: &str, output: &str) {
    let line = output.lines().find(|line| {
        let lower = line.to_lowercase();
        (line.contains("lead") || line.contains("main"))
            && lower.contains("trust")
            && lower.contains("attach")
    });
    assert!(
        line.is_some(),
        "{round}: one result line names the seat, the kind and the action: {output}"
    );
    assert!(
        output.contains(PERMS_40) && output.contains(LOADS),
        "{round}: the warning rows: {output}"
    );
    assert!(
        !output.contains('\u{1b}'),
        "{round}: terminal-safe: {output:?}"
    );
}

#[test]
fn dialogseat_a_launch_and_a_resume_report_a_seat_held_at_the_settings_dialog() {
    let mut scratch = OwnedScratch::root("ds", "launch");
    let root =
        fs::canonicalize(scratch.path()).unwrap_or_else(|why| panic!("setup: scratch: {why}"));
    let sock = root.join("sock");
    scratch.add_tmux_server(sock.clone());
    let home = root.join("home");
    for dir in [root.join("project"), home.clone()] {
        assert!(fs::create_dir_all(dir).is_ok(), "setup: directories");
    }
    install_fakes(&root, MCP40_80);
    write(
        root.join("config"),
        format!(
            "[profiles]\nfake = \"{0}/bin/claude {0}/vendor.pl {0}\"\n\n[roster]\nlead = fake\n\n[workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\nchat = off\nquota = off\nauto_upgrade = off\nauto_trust = on\ntheme = off\nmotion = off\n",
            root.display()
        ),
    );
    let journal = home.join("sessions/lw/events.jsonl");
    let fresh = launch(&root, &home, &sock, "launch");
    reported("launch", &fresh);
    assert!(wire(&root).is_empty(), "launch: NO key reached the dialog");
    no_auto_answer(&events(&journal));
    // Resume: the session's tmux is gone, its meta stays.
    let kill = Invocation::new("tmux")
        .arg("-S")
        .arg(&sock)
        .arg("kill-session")
        .arg("-t")
        .arg("=lw");
    let status = raw::run(&kill, &root, &root.join("kill.out"), &root.join("kill.err"))
        .unwrap_or_else(|why| panic!("setup: kill-session: {why}"));
    assert!(
        matches!(status.outcome(), ExitOutcome::Code(0)),
        "setup: kill-session: {}",
        text(root.join("kill.err"))
    );
    let resumed = launch(&root, &home, &sock, "resume");
    reported("resume", &resumed);
    assert!(wire(&root).is_empty(), "resume: NO key reached the dialog");
    no_auto_answer(&events(&journal));
}
