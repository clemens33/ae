//! Frozen R3/R4 acceptance: real watchdog -> real send -> private tmux composer.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures read their own scratch state"
)]
use super::cli::OwnedChild;
use super::renametell_routing_spec::{A, B, Rig};
use ae::time::Timestamp;
use std::fs;
use std::time::{Duration, Instant, SystemTime};

// Only the external fake agent changes its frame. Delivery and reconciliation are real.
const AGENT: &str = r#"#!/usr/bin/perl
use strict; use warnings;
my ($out) = @ARGV;
system('stty raw -echo 2>/dev/null');
binmode(STDIN, ':raw'); binmode(STDOUT, ':raw'); $| = 1;
my $border = "\xe2\x94\x80" x 399;
my $buf = ''; my $pasting = 0; my $last_mode = '';
sub mode {
    if (open(my $f, '<', "$out.mode")) { local $/; my $m = <$f>; close($f); return $m; }
    return 'idle';
}
sub draw {
    my ($text) = @_; $text =~ s/[\r\n]/ /g;
    my $m = mode(); print "\e[H\e[2J\e[?2004h";
    if ($m eq 'prompt') {
        open(my $f, '<', '__TRUST__') or die; local $/; my $t = <$f>; close($f);
        $t =~ s/\n/\r\n/g; print $t; return;
    }
    my $activity = $m eq 'busy' ? '✶ Thinking… (2s)' : '✻ Crunched for 1s · done 4:01 PM';
    $text = 'human draft' if $m eq 'draft';
    print "$activity\r\n$border\r\n\e[1m❯\e[0m $text\r\n$border\r\n🧠 fake-model xhigh ~/x\r\n⏵⏵ bypass permissions on\r\n";
}
draw('');
while (1) {
    my $m = mode(); if ($m ne $last_mode) { $last_mode = $m; draw($buf); }
    my $ready = ''; vec($ready, fileno(STDIN), 1) = 1;
    next unless select($ready, undef, undef, 0.05) > 0;
    last unless sysread(STDIN, my $ch, 1); $buf .= $ch;
    if ($buf =~ s/\e\[200~\z//) { $pasting = 1; next; }
    if ($buf =~ s/\e\[201~\z//) { $pasting = 0; draw($buf); next; }
    if ($pasting && $ch eq "\r") { chop($buf); $buf .= "\n"; next; }
    if (!$pasting && ($ch eq "\r" || $ch eq "\n")) {
        $buf =~ s/[\r\n]\z//;
        for my $dir (glob('__ROOT__/sessions/*')) {
            next unless open(my $meta, '<', "$dir/meta"); local $/; my $bytes = <$meta>; close($meta);
            next unless $bytes =~ /session_id=__ID__\n/;
            if (open(my $w, '<', "$dir/workspace.md")) {
                my $doc = <$w>; close($w); open(my $seen, '>', "$out.workspace") or die;
                print $seen $doc; close($seen);
            }
        }
        open(my $f, '>>', $out) or die; binmode($f); print $f $buf, "\0"; close($f);
        $buf = ''; draw(''); next;
    }
    draw($buf);
}
"#;

struct NoticeRig {
    rig: Rig,
    panes: [String; 2],
}
impl NoticeRig {
    fn new(tag: &str) -> Self {
        let rig = Rig::new(tag);
        let trust = rig.scratch.join("trust-frame");
        fs::write(
            &trust,
            include_str!("../fixtures/claude-trust/claude-trust-modal-80x24.txt"),
        )
        .expect("measured trust frame");
        let agent = AGENT
            .replace("__TRUST__", &trust.display().to_string())
            .replace("__ROOT__", &rig.root.display().to_string())
            .replace("__ID__", A);
        fs::write(rig.scratch.join("claude"), agent).expect("controlled external agent");
        let panes = rig.create("old", A);
        let this = Self { rig, panes };
        this.set("old", "idle_nudge_secs", "0");
        this.set("old", "launch_time.main", "0");
        this.set("old", "launch_time.worker.0", "0");
        for pane in &this.panes {
            this.until(
                || {
                    ae::harness_state::classify(&this.capture(pane), ae::tool::ToolKind::Claude)
                        == ae::harness_state::HarnessState::Idle
                },
                "fixture must classify idle",
            );
        }
        this
    }
    fn spawned(&self) {
        let command = format!(
            "exec perl {} {}",
            self.rig.scratch.join("claude").display(),
            self.rig.receipt(A, "extra").display()
        );
        let pane = self
            .rig
            .tmux(&[
                "split-window",
                "-d",
                "-P",
                "-F",
                "#{pane_id}",
                "-t",
                "old:main.0",
                &command,
            ])
            .trim()
            .to_owned();
        for (key, value) in [("@ae_slot", "spawned.0"), ("@ae_agent", "extra")] {
            self.rig
                .tmux(&["set-option", "-p", "-t", &pane, key, value]);
        }
        for (key, value) in [
            ("seat", "extra"),
            ("profile", "fake"),
            ("agent_bin", "claude"),
            ("harness_session", "pending"),
            ("launch_id", "tok-extra"),
            ("launch_time", "0"),
            ("pane", &pane),
        ] {
            self.set("old", &format!("{key}.spawned.0"), value);
        }
        self.until(
            || {
                ae::harness_state::classify(&self.capture(&pane), ae::tool::ToolKind::Claude)
                    == ae::harness_state::HarnessState::Idle
            },
            "spawned fixture must classify idle",
        );
    }
    fn set(&self, name: &str, key: &str, value: &str) {
        ae::meta::rewrite(&self.rig.dir(name), key, Some(value)).expect("fixture meta update");
    }
    fn capture(&self, pane: &str) -> String {
        self.rig.tmux(&["capture-pane", "-p", "-t", pane])
    }
    fn until(&self, test: impl Fn() -> bool, why: &str) {
        let deadline = Instant::now() + Duration::from_secs(12);
        while !test() {
            assert!(
                Instant::now() < deadline,
                "{why}; daemon={}",
                fs::read_to_string(self.rig.scratch.join("daemon-err")).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(30));
        }
    }
    fn mode(&self, seat: &str, state: &str) {
        fs::write(self.rig.receipt(A, seat).with_extension("mode"), state)
            .expect("agent frame control");
        let pane = &self.panes[usize::from(seat == "colead")];
        self.until(
            || match state {
                "busy" => {
                    ae::harness_state::classify(&self.capture(pane), ae::tool::ToolKind::Claude)
                        == ae::harness_state::HarnessState::Busy
                }
                "prompt" => {
                    ae::watchdog::human_prompt_class(&self.capture(pane), "claude").is_some()
                }
                "draft" => self.capture(pane).contains("human draft"),
                _ => {
                    ae::harness_state::classify(&self.capture(pane), ae::tool::ToolKind::Claude)
                        == ae::harness_state::HarnessState::Idle
                }
            },
            &format!("controlled {state} frame did not appear"),
        );
    }
    fn watch(&self, name: &str) -> OwnedChild {
        self.watch_interval(name, "1")
    }
    fn watch_interval(&self, name: &str, interval: &str) -> OwnedChild {
        let out = fs::File::create(self.rig.scratch.join("daemon-out")).expect("daemon out");
        let err = fs::File::create(self.rig.scratch.join("daemon-err")).expect("daemon err");
        let mut command = self.rig.command(None);
        command
            .arg("_watchdog-run")
            .arg(self.rig.dir(name))
            .args([
                "--interval",
                interval,
                "--idle-nudge-secs",
                "0",
                "--tg-supervise-secs",
                "0",
                "--stale-secs",
                "999999",
            ])
            .stdout(out)
            .stderr(err);
        let child = command.spawn().expect("real watchdog");
        self.until(
            || ae::watchdog_glue::pidfile(&self.rig.dir(name)).is_file(),
            "watchdog must register",
        );
        child
    }
    fn cycles(&self, name: &str, count: usize) {
        let path = ae::watchdog_glue::beat_path(&self.rig.dir(name));
        let mut last: Option<SystemTime> = None;
        for _ in 0..count {
            self.until(
                || {
                    fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .is_some_and(|t| Some(t) != last)
                },
                "watchdog cycle stalled",
            );
            last = fs::metadata(&path).and_then(|m| m.modified()).ok();
        }
    }
    fn notices(&self, seat: &str) -> Vec<String> {
        self.rig
            .received(A, seat)
            .into_iter()
            .filter(|text| text.lines().next() == Some("⟦ae:ctx⟧"))
            .collect()
    }
    fn told(&self, seat: &str) {
        self.until(
            || !self.notices(seat).is_empty(),
            "due seat received no ctx notice",
        );
    }
    fn events(&self, name: &str) -> Vec<ae::events::Event> {
        ae::store::open(&self.rig.dir(name))
            .container()
            .split(|b| *b == b'\n')
            .filter_map(|line| std::str::from_utf8(line).ok())
            .filter_map(|line| ae::events::Event::parse_line(line).ok())
            .collect()
    }
    fn count(&self, name: &str, action: &str) -> usize {
        self.events(name)
            .iter()
            .filter(|event| event.action == action)
            .count()
    }
    fn goal(&self, name: &str, text: &str, pane: Option<&str>) {
        let result = self.rig.helper(name, "goal", &[text], pane);
        assert_eq!(result.0, Some(0), "goal operation: {result:?}");
    }
    fn rename(&self, old: &str, new: &str) -> (String, String) {
        let started = Instant::now();
        let result = self.rig.run(&["rename", old, new], None);
        assert_eq!(result.0, Some(0), "rename operation: {result:?}");
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "caller blocked on seat delivery"
        );
        (result.1, result.2)
    }
    fn lock(&self, pane: &str) -> fs::File {
        let locks = self.rig.root.join("sessions/.locks");
        fs::create_dir_all(&locks).expect("target locks");
        let name: String = pane
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        ae::store::lock(&locks.join(format!("send-lock-{name}")), Duration::ZERO)
            .expect("controlled send lock")
    }
    fn source(&self, name: &str, action: &str, summary: &str, ts: Timestamp, extra: &str) {
        let actor = if action == "rename" {
            "ae:rename"
        } else {
            "human"
        };
        self.rig.append(name, &format!(r#"{{"ts":"{ts}","actor":"{actor}","action":"{action}","summary":"{summary}"{extra}}}"#));
    }
    fn attempt(
        &self,
        name: &str,
        kind: &str,
        token: &str,
        index: u32,
        age: i64,
        terminal: Option<&str>,
    ) {
        let reference = format!("sn-20261010T100000Z-{index:08x}");
        let ts = Timestamp::from_epoch(Timestamp::now().epoch() - age);
        let common = format!(
            r#""ts":"{ts}","actor":"watchdog","target":"lead","target_slot":"main","target_session":"{name}","target_session_id":"{A}","ref":"{reference}""#
        );
        self.rig.append(
            name,
            &format!(
                r#"{{{common},"action":"session-notice-attempt","summary":"{kind} {token}"}}"#
            ),
        );
        if let Some(action) = terminal {
            let summary = if action == "session-notice" {
                kind.to_owned()
            } else {
                format!("{kind} controlled-failure")
            };
            self.rig.append(
                name,
                &format!(r#"{{{common},"action":"{action}","summary":"{summary}"}}"#),
            );
        }
    }
}

#[test]
fn fixture_idle_composer_really_accepts_a_normal_guarded_send() {
    let n = NoticeRig::new("notice-control");
    let sent = n.rig.helper(
        "old",
        "send",
        &["colead", "control-paste"],
        Some(&n.panes[0]),
    );
    assert_eq!(sent.0, Some(0), "fixture send: {sent:?}");
    assert!(
        n.rig
            .received(A, "colead")
            .iter()
            .any(|text| text.contains("control-paste"))
    );
}

#[test]
fn fixture_frames_and_daemon_cycles_are_observable_without_a_notice() {
    let n = NoticeRig::new("notice-frame-control");
    for state in ["busy", "draft", "prompt", "idle"] {
        n.mode("lead", state);
    }
    n.spawned();
    let _watch = n.watch("old");
    n.cycles("old", 3);
    assert!(
        n.events("old")
            .iter()
            .all(|event| !event.action.starts_with("session-notice"))
    );
}

#[test]
fn a_changed_goal_excludes_spawned_seats() {
    let n = NoticeRig::new("notice-standing");
    n.spawned();
    n.goal("old", "standing-goal", None);
    let _watch = n.watch("old");
    n.told("lead");
    n.told("colead");
    n.cycles("old", 2);
    assert!(n.rig.received(A, "extra").is_empty());
}

#[test]
fn live_rename_tells_every_seat_once_with_current_paths_goal_and_workspace() {
    let n = NoticeRig::new("notice-rename");
    n.spawned();
    n.set("old", "watchdog", "true");
    n.set("old", "goal", "goal-present");
    n.rename("old", "new");
    for seat in ["lead", "colead", "extra"] {
        n.told(seat);
        let notices = n.notices(seat);
        assert_eq!(notices.len(), 1);
        let text = &notices[0];
        for fact in [
            "old",
            "new",
            &n.rig.dir("new").display().to_string(),
            "ae @new",
            "ae @old",
            "reply",
            "goal-present",
            "workspace.md",
        ] {
            assert!(text.contains(fact), "rename context omitted {fact}: {text}");
        }
        assert!(
            text.contains("no longer") || text.contains("gone") || text.contains("invalid"),
            "old address needs explicit invalidation: {text}"
        );
        let observed = fs::read_to_string(n.rig.receipt(A, seat).with_extension("workspace"))
            .expect("workspace at actual paste");
        assert!(
            observed.contains(&n.rig.dir("new").display().to_string()),
            "daemon pasted before new manifest publication"
        );
    }
    n.until(
        || n.count("new", "session-notice") == 3,
        "successful notices lack terminal records",
    );
    assert_eq!(n.count("new", "session-notice-attempt"), 3);
}

#[test]
fn rename_without_goal_says_no_goal_and_watchdog_off_says_not_told() {
    let n = NoticeRig::new("notice-off");
    let (out, err) = n.rename("old", "new");
    let status = format!("{out}{err}").to_lowercase();
    assert!(
        status.contains("watchdog") && status.contains("not told"),
        "caller hid off watchdog: {status}"
    );
    assert!(n.notices("lead").is_empty() && n.notices("colead").is_empty());
    let _watch = n.watch("new");
    n.told("lead");
    let text = &n.notices("lead")[0];
    assert!(
        text.to_lowercase().contains("no goal") || text.to_lowercase().contains("goal: none"),
        "rename omitted absent goal: {text}"
    );
}

#[test]
fn manifest_publication_failure_never_arms_a_rename_notice() {
    let n = NoticeRig::new("notice-manifest");
    fs::create_dir(n.rig.dir("old").join("workspace.md")).expect("unpublishable manifest");
    let renamed = n.rig.run(&["rename", "old", "new"], None);
    assert_eq!(renamed.0, Some(1), "manifest refusal: {renamed:?}");
    assert_eq!(n.count("new", "rename"), 0);
    assert!(
        format!("{}{}", renamed.1, renamed.2)
            .to_lowercase()
            .contains("not told"),
        "partial rename hid notice outcome: {renamed:?}"
    );
}

#[test]
fn changed_goal_tells_only_the_other_standing_seat_and_clear_is_a_notice() {
    let n = NoticeRig::new("notice-goal");
    let _watch = n.watch("old");
    n.goal("old", "goal-one", Some(&n.panes[0]));
    n.told("colead");
    assert!(n.notices("lead").is_empty(), "setter was told its own goal");
    assert!(n.notices("colead")[0].contains("goal-one"));
    let cleared = n.rig.helper("old", "goal", &["--clear"], Some(&n.panes[0]));
    assert_eq!(cleared.0, Some(0), "goal clear: {cleared:?}");
    n.until(
        || n.notices("colead").len() == 2,
        "clear did not re-arm notice",
    );
    assert!(n.notices("colead")[1].to_lowercase().contains("cleared"));
    assert!(n.notices("lead").is_empty());
}

#[test]
fn unchanged_goal_and_empty_clear_write_nothing_and_deliver_nothing() {
    let n = NoticeRig::new("notice-noop");
    n.set("old", "goal", "same-goal");
    n.goal("old", "same-goal", None);
    assert_eq!(
        n.count("old", "goal"),
        0,
        "unchanged goal journaled a source"
    );
    ae::meta::rewrite(&n.rig.dir("old"), "goal", None).expect("empty initial goal");
    assert_eq!(n.rig.helper("old", "goal", &["--clear"], None).0, Some(0));
    assert_eq!(n.count("old", "goal"), 0, "empty clear journaled a source");
    let _watch = n.watch("old");
    n.cycles("old", 2);
    assert!(n.notices("lead").is_empty() && n.notices("colead").is_empty());
}

#[test]
fn foreign_same_slot_setter_does_not_exclude_the_local_lead() {
    let n = NoticeRig::new("notice-foreign");
    let peer = n.rig.create("peer", B);
    let _watch = n.watch("old");
    n.goal("old", "foreign-goal", Some(&peer[0]));
    for seat in ["lead", "colead"] {
        n.told(seat);
        assert!(n.notices(seat)[0].contains("foreign-goal"));
    }
}

#[test]
fn two_goals_before_one_cycle_coalesce_to_the_current_goal() {
    let n = NoticeRig::new("notice-coalesce");
    n.goal("old", "obsolete-goal", None);
    n.goal("old", "newest-goal", None);
    let _watch = n.watch("old");
    for seat in ["lead", "colead"] {
        n.told(seat);
        assert_eq!(n.notices(seat).len(), 1);
        assert!(
            n.notices(seat)[0].contains("newest-goal")
                && !n.notices(seat)[0].contains("obsolete-goal")
        );
    }
}

#[test]
fn held_first_seat_lock_skips_it_and_another_idle_seat_is_told() {
    let n = NoticeRig::new("notice-lock");
    let held = n.lock(&n.panes[0]);
    n.goal("old", "while-locked", None);
    let _watch = n.watch("old");
    n.told("colead");
    assert!(n.notices("lead").is_empty());
    assert!(
        !n.events("old")
            .iter()
            .any(|event| event.action == "session-notice-failed"
                && event.target.as_deref() == Some("lead"))
    );
    n.goal("old", "after-lock-newest", None);
    drop(held);
    n.told("lead");
    assert!(
        n.notices("lead")[0].contains("after-lock-newest")
            && !n.notices("lead")[0].contains("while-locked"),
        "body was frozen before lock acquisition"
    );
}

#[test]
fn multiple_renames_behind_a_held_lock_deliver_the_newest_name_and_goal() {
    let n = NoticeRig::new("notice-newest");
    let held = n.lock(&n.panes[0]);
    n.set("old", "goal", "first-goal");
    n.rename("old", "middle");
    n.goal("middle", "current-goal", None);
    n.rename("middle", "newest");
    let _watch = n.watch("newest");
    n.told("colead");
    drop(held);
    n.told("lead");
    let text = &n.notices("lead")[0];
    assert!(
        text.contains("ae @newest")
            && text.contains("current-goal")
            && !text.contains("first-goal"),
        "late compose used old context: {text}"
    );
    assert!(
        !text.contains(&n.rig.dir("middle").display().to_string()),
        "an obsolete helper directory was advertised"
    );
}

#[test]
fn busy_for_more_than_three_cycles_stays_due_and_recovers_once_idle() {
    let n = NoticeRig::new("notice-busy");
    n.mode("lead", "busy");
    n.goal("old", "busy-goal", None);
    let _watch = n.watch("old");
    n.told("colead");
    n.cycles("old", 5);
    assert!(n.notices("lead").is_empty());
    assert_eq!(
        n.count("old", "session-notice-failed"),
        0,
        "busy skips consumed attempts"
    );
    assert_eq!(n.count("old", "session-notice-gave-up"), 0);
    n.mode("lead", "idle");
    n.told("lead");
    assert_eq!(n.notices("lead").len(), 1);
}

#[test]
fn human_draft_and_prompt_are_not_pasted_over_and_clear_without_attempts() {
    for state in ["draft", "prompt"] {
        let n = NoticeRig::new(state);
        n.mode("lead", state);
        n.goal("old", "guarded-goal", None);
        let _watch = n.watch("old");
        n.told("colead");
        n.cycles("old", 3);
        assert!(
            n.rig.received(A, "lead").is_empty(),
            "guarded frame received bytes"
        );
        assert!(
            !n.events("old")
                .iter()
                .any(|event| event.action == "session-notice-attempt"
                    && event.target.as_deref() == Some("lead"))
        );
        n.mode("lead", "idle");
        n.told("lead");
        assert!(n.notices("lead")[0].contains("guarded-goal"));
    }
}

#[test]
fn a_dead_seat_receives_nothing_and_is_not_counted_as_a_failed_attempt() {
    let n = NoticeRig::new("notice-dead");
    n.rig
        .tmux(&["set-option", "-w", "-t", "old:main", "remain-on-exit", "on"]);
    n.rig
        .tmux(&["respawn-pane", "-k", "-t", &n.panes[0], "true"]);
    n.until(
        || {
            n.rig
                .tmux(&["display-message", "-p", "-t", &n.panes[0], "#{pane_dead}"])
                .trim()
                == "1"
        },
        "fixture seat did not die",
    );
    n.goal("old", "live-peer-goal", None);
    let _watch = n.watch("old");
    n.told("colead");
    assert!(n.rig.received(A, "lead").is_empty());
    assert!(
        !n.events("old")
            .iter()
            .any(|event| event.action == "session-notice-attempt"
                && event.target.as_deref() == Some("lead"))
    );
}

#[test]
fn an_expired_unfinished_attempt_is_retried_after_watchdog_restart() {
    let n = NoticeRig::new("notice-crash");
    n.goal("old", "crash-current", None);
    n.attempt("old", "goal", "tok-main", 1, 3600, None);
    let _watch = n.watch("old");
    n.told("lead");
    assert!(n.notices("lead")[0].contains("crash-current"));
    n.until(
        || {
            n.events("old").iter().any(|event| {
                event.action == "session-notice" && event.target.as_deref() == Some("lead")
            })
        },
        "retry lacks truthful terminal record",
    );
}

#[test]
fn a_fresh_attempt_is_not_false_success_and_terminal_failure_rearms_it() {
    let n = NoticeRig::new("notice-flight");
    n.goal("old", "flight-current", None);
    n.attempt("old", "goal", "tok-main", 2, 0, None);
    let _watch = n.watch_interval("old", "60");
    n.told("colead");
    assert!(
        n.notices("lead").is_empty(),
        "live attempt was pasted twice"
    );
    assert!(
        !n.events("old").iter().any(
            |event| event.action == "session-notice" && event.target.as_deref() == Some("lead")
        ),
        "intent was labelled told"
    );
    n.rig.append("old", &format!(r#"{{"ts":"{}","actor":"watchdog","action":"session-notice-failed","target":"lead","target_slot":"main","target_session":"old","target_session_id":"{A}","ref":"sn-20261010T100000Z-00000002","summary":"goal controlled-failure"}}"#, Timestamp::now()));
    let triggered = n
        .rig
        .command(None)
        .args(["@old", "send", "lead", "ignored-placeholder"])
        .env("AE_SENDER_OVERRIDE", "watchdog")
        .env("_AE_EVENT_ACTION", "session-notice-due")
        .output()
        .expect("real notice leg trigger");
    assert!(
        triggered.status.success(),
        "terminal failure did not re-arm leg: {}",
        String::from_utf8_lossy(&triggered.stderr)
    );
    n.told("lead");
}

#[test]
fn three_delivery_failures_give_up_visibly_and_newer_source_rearms() {
    let n = NoticeRig::new("notice-giveup");
    n.goal("old", "failing-goal", Some(&n.panes[1]));
    let blocked = n.rig.dir("old").join("messages");
    fs::write(&blocked, "controlled body-store failure").expect("unwritable message directory");
    let _watch = n.watch("old");
    n.until(
        || n.count("old", "session-notice-gave-up") == 1,
        "failure ceiling was not journaled",
    );
    assert!(n.notices("lead").is_empty());
    assert_eq!(n.count("old", "session-notice-failed"), 3);
    fs::remove_file(blocked).expect("repair fixture body store");
    n.goal("old", "rearmed-goal", None);
    n.told("lead");
    assert!(n.notices("lead")[0].contains("rearmed-goal"));
}

#[test]
fn goal_told_after_a_rename_never_consumes_the_pending_rename() {
    let n = NoticeRig::new("notice-kind");
    n.rename("old", "new");
    n.attempt("new", "goal", "tok-main", 20, 0, Some("session-notice"));
    let _watch = n.watch("new");
    n.told("lead");
    assert!(
        n.notices("lead")[0].contains("ae @new"),
        "goal receipt erased rename due"
    );
}

#[test]
fn previous_incarnation_told_after_a_new_source_never_consumes_it() {
    let n = NoticeRig::new("notice-incarnation");
    n.rename("old", "new");
    n.attempt(
        "new",
        "rename",
        "old-launch-token",
        21,
        0,
        Some("session-notice"),
    );
    let _watch = n.watch("new");
    n.told("lead");
    assert!(n.notices("lead")[0].contains("ae @new"));
}

#[test]
fn same_second_launch_boundaries_use_journal_order_including_spawn_display() {
    for action in ["spawn", "reseat", "relaunch"] {
        let n = NoticeRig::new(action);
        let ts = Timestamp::now();
        n.source("old", "rename", "previous -> old", ts, "");
        let extra = if action == "spawn" {
            r#", "target":"lead""#
        } else {
            r#", "target":"lead","target_slot":"main""#
        };
        n.source("old", action, "new launch", ts, extra);
        let _watch = n.watch("old");
        n.told("colead");
        n.cycles("old", 2);
        assert!(
            n.notices("lead").is_empty(),
            "source before launch journal position survived"
        );
        n.source("old", "rename", "newer -> old", ts, "");
        n.told("lead");
        assert!(
            n.notices("lead")[0].contains("newer"),
            "same-second source after launch was lost"
        );
    }
}

#[test]
fn legacy_same_second_source_does_not_survive_the_strict_launch_fallback() {
    let n = NoticeRig::new("notice-legacy");
    let ts = Timestamp::now();
    n.set("old", "launch_time.main", &ts.epoch().to_string());
    n.source("old", "rename", "previous -> old", ts, "");
    let _watch = n.watch("old");
    n.told("colead");
    n.cycles("old", 2);
    assert!(
        n.notices("lead").is_empty(),
        "legacy fallback accepted equal timestamp"
    );
    n.source(
        "old",
        "rename",
        "newer -> old",
        Timestamp::from_epoch(ts.epoch() + 1),
        "",
    );
    n.told("lead");
}
