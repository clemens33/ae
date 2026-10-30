#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "private-server fixtures read and alter only their scratch state"
)]
use super::cli::{OwnedChild, Runner};
use super::renametell_routing_spec::{A, CEILING, Rig};
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::{Duration, Instant};
const SLOT: &str = "spawned.0";
const BODY: &str = "seatproof-original-task";
const AGENT: &str = r#"#!/usr/bin/perl
use strict; use warnings; use FindBin;
my $out = (@ARGV && $ARGV[0] =~ m{^/}) ? $ARGV[0] : "$FindBin::Bin/spawn";
system('stty raw -echo 2>/dev/null');
binmode(STDIN, ':raw'); binmode(STDOUT, ':raw'); $| = 1;
my $border = "\xe2\x94\x80" x 399;
my ($buf, $paste, $enters, $ghost) = ('', 0, 0, '');
sub mode { if (open(my $f, '<', "$out.arm")) { my $m=<$f>; chomp($m); return $m; } return ''; }
sub change {
    my ($event) = @_; return unless mode() eq $event;
    open(my $arm, '<', "$out.arm") or die; my @a=<$arm>; close($arm); chomp(@a);
    open(my $f, '<', '__META__') or die; local $/; my $text=<$f>; close($f);
    $text =~ s/^launch_id\.\Q$a[1]\E=.*$/launch_id.$a[1]=tok-replaced/m;
    open(my $w, '>', '__META__.next') or die; print $w $text; close($w);
    rename('__META__.next', '__META__') or die;
    unlink("$out.arm") or die; open(my $seen, '>', "$out.changed") or die; close($seen);
}
sub draw {
    my $text=$buf; $text =~ s/[\r\n]/ /g;
    print "\e[H\e[2J\e[?2004h✻ Crunched for 1s · done 4:01 PM\r\n$border\r\n\e[1m❯\e[0m $text\r\n$border\r\n🧠 fake-model ~/x\r\n⏵⏵ bypass permissions on\r\n$ghost";
}
draw(); while (sysread(STDIN, my $ch, 1)) {
    open(my $wire, '>>', "$out.wire") or die; binmode($wire); print $wire $ch; close($wire);
    $buf .= $ch;
    if ($buf =~ s/\e\[200~\z//) { $paste=1; next; }
    if ($buf =~ s/\e\[201~\z//) { $paste=0; change('paste'); draw(); next; }
    if ($paste && $ch eq "\r") { chop($buf); $buf .= "\n"; next; }
    if (!$paste && ($ch eq "\r" || $ch eq "\n")) {
        $buf =~ s/[\r\n]\z//; $enters++;
        open(my $e, '>>', "$out.enters") or die; print $e "enter\n"; close($e);
        if (mode() eq 'enter') { change('enter'); draw(); next; }
        if (mode() eq 'linger') { $ghost=$buf; $ghost =~ s/[\r\n]/ /g; change('linger'); }
        open(my $f, '>>', $out) or die; binmode($f); print $f $buf, "\0"; close($f);
        $buf=''; draw(); next;
    }
    draw();
}
"#;
const TMUX_BARRIER: &str = r#"#!/usr/bin/perl
use strict; use warnings; use Fcntl qw(O_CREAT O_EXCL O_WRONLY); my $root=$ENV{SEATPROOF_BARRIER};
my $after=$ENV{SEATPROOF_AFTER_INPUT}//'';
sub fail {
    open(my $failed, '>', "$root/revocation-failed") or die; print $failed @_; close($failed);
    die @_;
}
if (($ENV{SEATPROOF_FAULT}//'') eq 'physical' && $ARGV[-1] eq '#{session_id} | #{session_created}' && grep { $_ eq $ENV{SEATPROOF_PANE} } @ARGV) {
    open(my $seen, '>', "$root/physical-failed") or die; close($seen); exit 1;
}
if (!$after && grep { $_ eq 'capture-pane' } @ARGV) {
    if (sysopen(my $seen, "$root/seen", O_CREAT|O_EXCL|O_WRONLY, 0600)) {
        close($seen); my $end=time()+90;
        while (!-e "$root/go") { die 'barrier deadline' if time()>$end; select(undef,undef,undef,.01); }
    }
}
# tmux returning proves input was queued, not consumed. Keep the helper behind
# the agent's on-disk revocation before its next guarded submit stage.
if (($after eq 'paste' && grep { $_ eq 'paste-buffer' } @ARGV) ||
    ($after eq 'enter' && $ARGV[-1] eq 'Enter' && grep { $_ eq 'send-keys' } @ARGV)) {
    system {$ENV{SEATPROOF_TMUX}} $ENV{SEATPROOF_TMUX}, @ARGV;
    fail('revocation input failed') if $? != 0;
    my $end=time()+90;
    while (!-e $ENV{SEATPROOF_CHANGED}) {
        fail('revocation acknowledgement deadline') if time()>$end;
        select(undef,undef,undef,.01);
    }
    open(my $seen, '>', "$root/revocation-seen") or die; close($seen);
    exit 0;
}
exec {$ENV{SEATPROOF_TMUX}} $ENV{SEATPROOF_TMUX}, @ARGV;
die 'exec tmux';
"#;
fn read(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path).unwrap_or_default()
}
fn wait(what: &str, until: impl Fn() -> bool) {
    let end = Instant::now() + CEILING;
    while !until() {
        assert!(Instant::now() < end, "fixture did not reach {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn executable(path: &Path, text: &str) {
    fs::write(path, text).expect("fixture executable");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("executable mode");
}
struct World {
    rig: Rig,
    main: String,
    pane: String,
    out: PathBuf,
}
impl World {
    fn new(tag: &str) -> Self {
        let rig = Rig::new(tag);
        let script = AGENT.replace("__META__", &rig.dir("s").join("meta").display().to_string());
        executable(&rig.scratch.join("claude"), &script);
        let main = rig.create("s", A)[0].clone();
        let out = rig.scratch.join("kid");
        let agent = rig.scratch.join("claude");
        let command = format!("exec perl {} {}", agent.display(), out.display());
        let args = [
            "new-window",
            "-dP",
            "-F",
            "#{pane_id}",
            "-t",
            "s:",
            &command,
        ];
        let pane = rig.tmux(&args).trim().to_owned();
        for (key, value) in [("@ae_slot", SLOT), ("@ae_agent", "kid")] {
            rig.tmux(&["set-option", "-p", "-t", &pane, key, value]);
        }
        let rows = format!(
            "seat.{SLOT}=kid\nprofile.{SLOT}=fake\nagent_bin.{SLOT}=claude\nharness_session.{SLOT}=pending\nlaunch_id.{SLOT}=tok-kid\npane.{SLOT}={pane}\n"
        );
        fs::write(
            rig.dir("s").join("meta"),
            read(rig.dir("s").join("meta")) + &rows,
        )
        .expect("kid roster");
        wait("kid composer", || {
            rig.tmux(&["capture-pane", "-p", "-t", &pane])
                .contains("fake-model")
        });
        Self {
            rig,
            main,
            pane,
            out,
        }
    }
    fn dir(&self) -> PathBuf {
        self.rig.dir("s")
    }
    fn meta(&self) -> PathBuf {
        self.dir().join("meta")
    }
    fn change(&self, mode: &str, slot: &str) {
        let text = read(self.meta());
        let key = format!("launch_id.{slot}");
        let suffix = format!(".{slot}");
        let next = match mode {
            "remove" => {
                text.lines()
                    .filter(|line| {
                        !line
                            .split_once('=')
                            .is_some_and(|(key, _)| key.ends_with(&suffix))
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n"
            }
            "legacy" => {
                text.lines()
                    .filter(|line| !line.starts_with(&format!("{key}=")))
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n"
            }
            "replace" | "restamp" => {
                text.replace(&format!("{key}=tok-kid"), &format!("{key}=tok-replaced"))
            }
            "duplicate" => text + &format!("{key}=tok-kid\n"),
            "duplicate-seat" => text + &format!("seat.{slot}=kid\n"),
            "duplicate-session" => text + &format!("session_id={A}\n"),
            "session-garble" => text.replace("session_id=garbled-a", "session_id=garbled-b"),
            _ => panic!("unknown fixture change"),
        };
        fs::write(self.meta(), next).expect("scratch mutation");
    }
    fn command(&self, verb: &str, text: bool, paneless: bool) -> Runner {
        let mut cmd = self.rig.command((!paneless).then_some(self.main.as_str()));
        cmd.args(["@s", verb, "kid"]);
        if text {
            cmd.arg(BODY);
        }
        cmd.env("AE_SEND_DEFER_SEC", "15")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }
    fn trigger(&self, target: &str, action: &str) -> Runner {
        let mut cmd = self.rig.command(None);
        cmd.args(["@s", "send", target, "trigger placeholder"])
            .env("AE_SENDER_OVERRIDE", "watchdog")
            .env("_AE_EVENT_ACTION", action)
            .env("_AE_EVENT_SUMMARY", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }
    fn screen(&self) -> String {
        self.rig.tmux(&["capture-pane", "-p", "-t", &self.pane])
    }
    fn events(&self) -> String {
        read(self.rig.dir("s").join("events.jsonl"))
    }
    fn count(&self, action: &str) -> usize {
        self.events()
            .matches(&format!("\"action\":\"{action}\""))
            .count()
    }
    fn lock(&self) -> fs::File {
        let dir = self.rig.root.join("sessions/.locks");
        fs::create_dir_all(&dir).expect("lock directory");
        ae::store::lock(
            &dir.join(format!("send-lock-{}", self.pane.replace('%', "_"))),
            Duration::ZERO,
        )
        .expect("own target lock")
    }
    fn bodies(&self) -> usize {
        fs::read_dir(self.rig.dir("s").join("messages")).map_or(0, Iterator::count)
    }
    fn arm(event: &str, slot: &str, out: &Path) {
        fs::write(out.with_extension("arm"), format!("{event}\n{slot}\n"))
            .expect("causal transition");
    }
    fn arm_barrier(&self, cmd: &mut Runner, event: &str, slot: &str, out: &Path) {
        Self::arm(event, slot, out);
        self.barrier(cmd);
        cmd.env("SEATPROOF_AFTER_INPUT", event)
            .env("SEATPROOF_CHANGED", out.with_extension("changed"));
    }
    fn acknowledged(&self) {
        let barrier = self.rig.scratch.join("barrier");
        assert!(
            !barrier.join("revocation-failed").exists(),
            "fixture handshake failed: {}",
            read(barrier.join("revocation-failed"))
        );
        assert!(
            barrier.join("revocation-seen").exists(),
            "fixture never acknowledged revocation"
        );
    }
    fn barrier(&self, cmd: &mut Runner) {
        let path = cmd
            .get_envs()
            .find(|(key, _)| *key == "PATH")
            .and_then(|(_, value)| value)
            .expect("isolated PATH")
            .to_owned();
        let real = std::env::split_paths(&path)
            .map(|dir| dir.join("tmux"))
            .find(|path| path.is_file())
            .expect("real tmux");
        let bin = self.rig.scratch.join("barrier");
        fs::create_dir_all(&bin).expect("barrier directory");
        executable(&bin.join("tmux"), TMUX_BARRIER);
        let mut paths = vec![bin.clone()];
        paths.extend(std::env::split_paths(&path));
        cmd.env("PATH", std::env::join_paths(paths).expect("barrier PATH"))
            .env("SEATPROOF_BARRIER", bin)
            .env("SEATPROOF_TMUX", real);
    }
    fn paused(&self) {
        wait("readiness/quiet capture", || {
            self.rig.scratch.join("barrier/seen").exists()
        });
    }
    fn resume(&self) {
        fs::write(self.rig.scratch.join("barrier/go"), "go").expect("release observation");
    }
    fn refuse(&self, output: &Output, word: &str) {
        let err = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "not refused: {err}");
        assert!(
            err.contains("REFUSED") && err.contains(word) && !err.contains("orchestrator role"),
            "wrong ordinary refusal: {err}"
        );
        let events = self.events();
        assert_eq!(self.count("refused"), 1, "one refusal audit: {events}");
        let audit = events
            .lines()
            .find(|line| line.contains("\"action\":\"refused\""))
            .expect("audit");
        assert!(!audit.contains("\"target"), "{audit}");
        assert!(
            !audit.contains("tok-replaced") && !audit.contains("successor"),
            "{audit}"
        );
    }
    fn untouched(&self, before: &str) {
        assert!(
            read(self.out.with_extension("wire")).is_empty(),
            "a key reached the revoked seat"
        );
        assert_eq!(self.screen(), before, "revoked seat screen changed");
        assert_eq!(self.count("send"), 0);
    }
}
fn finish(child: OwnedChild) -> Output {
    child.wait_with_output().expect("fixture child result")
}
#[test]
fn an_orphan_seat_is_refused_by_every_peer_verb_before_the_first_key() {
    for (verb, text) in [
        ("send", true),
        ("ask", true),
        ("review", true),
        ("interrupt", true),
        ("interrupt", false),
    ] {
        let w = World::new(&format!("sp-orphan-{verb}-{text}"));
        w.change("remove", SLOT);
        let before = w.screen();
        w.rig.tmux(&["copy-mode", "-t", &w.pane]);
        let output = w.command(verb, text, false).output().expect("peer helper");
        w.refuse(&output, "retired");
        w.untouched(&before);
        assert_eq!(
            w.rig
                .tmux(&["display-message", "-p", "-t", &w.pane, "#{pane_in_mode}"])
                .trim(),
            "1",
            "cancel-copy key reached revoked seat"
        );
        assert_eq!(w.count(verb), 0);
    }
}
#[test]
fn paneless_refusals_are_audited_in_the_helper_session() {
    let w = World::new("sp-paneless");
    w.change("remove", SLOT);
    let before = w.screen();
    let mut cmd = w.command("send", true, true);
    cmd.env("AE_SENDER_OVERRIDE", "watchdog");
    let output = cmd.output().expect("paneless send");
    w.refuse(&output, "retired");
    w.untouched(&before);
    assert!(w.events().contains("\"actor\":\"watchdog\""));
}
#[test]
fn duplicated_required_identity_rows_are_unproven_not_retired() {
    for mode in ["duplicate", "duplicate-seat", "duplicate-session"] {
        let w = World::new(&format!("sp-{mode}"));
        w.change(mode, SLOT);
        let before = w.screen();
        let output = w
            .command("send", true, false)
            .output()
            .expect("damaged seat send");
        w.refuse(&output, "could not be proven");
        w.untouched(&before);
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("retired"),
            "damage asserted retirement"
        );
    }
}
#[test]
fn a_send_reproves_removed_and_replaced_seats_after_its_lock_wait() {
    for mode in ["session-garble", "remove", "replace", "restamp"] {
        let w = World::new(&format!("sp-lock-{mode}"));
        if mode == "session-garble" {
            ae::meta::rewrite(&w.dir(), "session_id", Some("garbled-a")).expect("seed bad id");
        }
        let held = w.lock();
        let before = w.screen();
        let child = w
            .command("send", true, false)
            .spawn()
            .expect("blocked send");
        wait("admitted body", || w.bodies() > 0);
        w.change(mode, SLOT);
        if mode == "restamp" {
            w.rig
                .tmux(&["set-option", "-p", "-t", &w.pane, "@ae_agent", "successor"]);
        }
        drop(held);
        let output = finish(child);
        let word = if mode == "remove" {
            "retired"
        } else if mode == "session-garble" {
            "could not be proven"
        } else {
            "replaced"
        };
        w.refuse(&output, word);
        assert!(
            w.events().contains("tok-kid"),
            "original launch missing: {}",
            w.events()
        );
        w.untouched(&before);
    }
}
#[test]
fn a_real_retire_during_the_quiet_gate_gives_an_ordinary_audited_refusal() {
    let w = World::new("sp-retire");
    let server =
        ae::inventory::ServerId::from_typed_flags("socket", &w.rig.socket.display().to_string())
            .expect("socket");
    assert!(ae::transport::load_buffer(&server, "draft", b"human draft"));
    assert!(ae::transport::paste_buffer(&server, "draft", &w.pane));
    let wire = w.out.with_extension("wire");
    wait("fixture draft", || read(&wire).ends_with("\x1b[201~"));
    fs::write(&wire, "").expect("reset fixture receipt");
    let mut cmd = w.command("send", true, false);
    w.barrier(&mut cmd);
    let child = cmd.spawn().expect("quiet send");
    w.paused();
    let result = w.rig.helper("s", "retire", &["kid"], Some(&w.main));
    assert_eq!(result.0, Some(0), "real retire: {result:?}");
    w.resume();
    let output = finish(child);
    w.refuse(&output, "REFUSED");
    assert!(read(wire).is_empty(), "retired seat got input");
}
#[test]
fn revocation_after_a_paste_or_first_enter_suppresses_the_next_submit_key() {
    for event in ["paste", "enter"] {
        let w = World::new(&format!("sp-{event}"));
        let mut cmd = w.command("send", true, false);
        w.arm_barrier(&mut cmd, event, SLOT, &w.out);
        let output = cmd.output().expect("causal send");
        w.acknowledged();
        let word = if event == "paste" {
            "no Enter was sent"
        } else {
            "submission unconfirmed"
        };
        w.refuse(&output, word);
        assert!(
            w.out.with_extension("changed").exists(),
            "transition missed"
        );
        assert_eq!(
            read(w.out.with_extension("enters")).lines().count(),
            usize::from(event == "enter"),
            "extra submit key"
        );
        assert_eq!(w.count("send"), 0);
        assert!(
            fs::read_dir(w.dir().join("messages"))
                .expect("preserved bodies")
                .any(|entry| read(entry.expect("body entry").path()).contains(BODY)),
            "recoverable original body missing"
        );
    }
}
#[test]
fn a_brief_retry_carries_its_record_identity_across_readiness_and_lock_waits() {
    for barrier in ["ready", "lock"] {
        let w = World::new(&format!("sp-retry-{barrier}"));
        let record = ae::brief_retry::Record {
            slot: SLOT.into(),
            reference: "spawn-spawned.0".into(),
            pane: w.pane.clone(),
            launch_id: "tok-kid".into(),
            actor: "lead".into(),
            attempts: 0,
            created: ae::time::Timestamp::now().epoch(),
            phase: ae::brief_retry::Phase::Armed,
            body: BODY.into(),
        };
        ae::brief_retry::publish(&w.dir(), &record).expect("old brief record");
        let before = w.screen();
        let mut cmd = w.trigger("kid", "brief-retry");
        let held = (barrier == "lock").then(|| w.lock());
        if barrier == "ready" {
            w.barrier(&mut cmd);
        }
        let child = cmd.spawn().expect("retry helper");
        if barrier == "ready" {
            w.paused();
        } else {
            wait("retry body's publication", || w.bodies() > 0);
        }
        w.change("replace", SLOT);
        if barrier == "ready" {
            ae::meta::rewrite(&w.dir(), &format!("seat.{SLOT}"), Some("successor"))
                .expect("successor roster");
            w.rig
                .tmux(&["set-option", "-p", "-t", &w.pane, "@ae_agent", "successor"]);
            w.resume();
        }
        drop(held);
        let output = finish(child);
        w.refuse(&output, "replaced");
        w.untouched(&before);
        assert!(!ae::brief_retry::path(&w.dir(), SLOT).exists(), "rearmed");
        assert_eq!(w.count("brief-gave-up"), 1, "{}", w.events());
    }
}
#[test]
fn a_spawn_keeps_its_created_identity_through_readiness_and_the_linger_enter() {
    for cut in ["ready", "linger"] {
        let w = World::new(&format!("sp-spawn-{cut}"));
        assert_eq!(
            w.rig.helper("s", "retire", &["kid"], Some(&w.main)).0,
            Some(0)
        );
        let out = w.rig.scratch.join("spawn");
        if cut == "linger" {
            World::arm("linger", SLOT, &out);
        }
        let mut cmd = w.rig.command(Some(&w.main));
        cmd.args(["@s", "spawn", "fresh", "--using", "fake", "--", BODY])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if cut == "ready" {
            w.barrier(&mut cmd);
        }
        let child = cmd.spawn().expect("real spawn");
        if cut == "ready" {
            w.paused();
            let text = read(w.meta());
            assert!(
                text.contains("seat.spawned.0=fresh"),
                "capture preceded seat creation"
            );
            ae::meta::Meta::parse(&text)
                .launch_id(SLOT)
                .expect("created launch");
            ae::meta::rewrite(&w.dir(), &format!("launch_id.{SLOT}"), Some("tok-replaced"))
                .expect("replace created seat");
            w.resume();
        }
        let output = finish(child);
        let err = String::from_utf8_lossy(&output.stderr);
        assert!(err.contains("REFUSED"), "stale spawn brief ({cut}): {err}");
        assert_eq!(w.count("refused"), 1, "spawn guard audit: {}", w.events());
        assert_eq!(
            read(out.with_extension("enters")).lines().count(),
            usize::from(cut == "linger"),
            "old brief got an extra Enter"
        );
        assert!(
            !ae::brief_retry::path(&w.dir(), SLOT).exists(),
            "retry armed"
        );
        assert!(read(w.dir().join("undelivered.fresh.txt")).contains(BODY));
    }
}
#[test]
fn context_notices_audit_precompose_and_postcompose_revocation_without_claiming_success() {
    for cut in ["quiet", "paste"] {
        let w = World::new(&format!("sp-notice-{cut}"));
        ae::meta::rewrite(&w.dir(), "launch_time.main", Some("0")).expect("notice boundary");
        w.rig.append("s", r#"{"ts":"2026-10-27T10:00:00Z","actor":"lead","action":"goal","target":"","summary":"notice owed"}"#);
        fs::remove_file(w.dir().join("goal")).expect("unlink helper");
        fs::write(w.dir().join("goal"), BODY).expect("current goal");
        let target = w.rig.receipt(A, "lead");
        let mut cmd = w.trigger("lead", "session-notice-due");
        cmd.env("TMUX", format!("{},0,0", w.rig.socket.display()))
            .env("TMUX_PANE", &w.main);
        if cut == "quiet" {
            w.barrier(&mut cmd);
        } else {
            w.arm_barrier(&mut cmd, "paste", "main", &target);
        }
        let child = cmd.spawn().expect("notice leg");
        if cut == "quiet" {
            w.paused();
            let text =
                read(w.meta()).replace("launch_id.main=tok-main", "launch_id.main=tok-replaced");
            fs::write(w.meta(), text).expect("replace notice recipient");
            w.resume();
        }
        let output = finish(child);
        if cut == "paste" {
            w.acknowledged();
        }
        w.refuse(&output, "replaced");
        assert!(
            w.events()
                .lines()
                .any(|line| line.contains("\"action\":\"refused\"")
                    && line.contains("\"actor\":\"watchdog\"")),
            "request actor missing: {}",
            w.events()
        );
        for action in ["session-notice-attempt", "session-notice-failed"] {
            assert_eq!(w.count(action), usize::from(cut == "paste"), "{action}");
        }
        assert_eq!(w.count("session-notice"), 0);
        assert!(read(target.with_extension("enters")).is_empty());
    }
}
#[test]
fn live_reseated_legacy_and_unstamped_raw_targets_keep_receiving_new_messages() {
    for arm in ["ordinary", "reseated", "legacy", "raw"] {
        let w = World::new(&format!("sp-control-{arm}"));
        if arm == "reseated" {
            w.change("replace", SLOT);
        }
        if arm == "legacy" {
            w.change("legacy", SLOT);
        }
        let mut cmd = if arm == "raw" {
            for key in ["@ae_slot", "@ae_agent"] {
                w.rig.tmux(&["set-option", "-p", "-u", "-t", &w.pane, key]);
            }
            let mut cmd = w.rig.command(Some(&w.main));
            cmd.args(["@s", "send", &w.pane, BODY]);
            cmd
        } else {
            w.command("send", true, false)
        };
        let output = cmd.output().expect("compatibility send");
        assert_eq!(
            output.status.code(),
            Some(0),
            "live {arm} refused: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(read(&w.out).contains(BODY), "no live {arm} message");
        assert_eq!(w.count("refused"), 0);
    }
}
#[test]
fn admission_does_not_reread_identity_rows_outside_its_snapshot() {
    let source = include_str!("../../src/tracked.rs");
    let marker = "pub(crate) fn admit_agent_address";
    let body = source.split(marker).nth(1).expect("admission");
    let body = body.split("\nimpl").next().expect("body");
    assert_eq!(body.matches("address_meta(").count(), 1);
    for reread in ["session_id_in(", "launch_id_at("] {
        assert!(!body.contains(reread), "binding rereads {reread}");
    }
}
#[test]
fn an_unstamped_raw_pane_with_unreadable_physical_identity_is_refused() {
    let w = World::new("sp-raw-unproven");
    for key in ["@ae_slot", "@ae_agent"] {
        w.rig.tmux(&["set-option", "-p", "-u", "-t", &w.pane, key]);
    }
    let before = w.screen();
    let mut cmd = w.rig.command(Some(&w.main));
    cmd.args(["@s", "send", &w.pane, BODY]);
    w.barrier(&mut cmd);
    w.resume();
    cmd.env("SEATPROOF_FAULT", "physical")
        .env("SEATPROOF_PANE", &w.pane);
    let output = cmd.output().expect("unproven raw send");
    assert!(
        w.rig.scratch.join("barrier/physical-failed").exists(),
        "physical fault did not run"
    );
    w.refuse(&output, "could not be proven");
    w.untouched(&before);
}
