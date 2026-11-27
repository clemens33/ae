//! The production door against owned tmux sockets. The fake pane records bytes only.

#![allow(
    clippy::disallowed_methods,
    reason = "acceptance owns every socket and wire file this fixture opens"
)]

use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::inventory::ServerId;
use crate::meta::Selector;
use crate::tmux::CycleBatch;
use crate::tmux::control::{HookRefusal, Proven, SidecarCommand};
use crate::transport::quietwd_spec::Rig;
use crate::transport::{
    ChannelWaiter, Sidecar, SidecarLost, SidecarReply, WaitChildEvent, Waited,
    observe_motion_panes, publish_cycle, run_tmux_op, run_tmux_wait, spawns_during,
    wait_children_during,
};

const IO: Duration = Duration::from_secs(4);
const SHORT: Duration = Duration::from_millis(200);
const READY: &str = "d54-sidecar-wire-ready";

fn until<T>(mut read: impl FnMut() -> Option<T>, description: &str) -> T {
    let end = Instant::now() + IO;
    loop {
        if let Some(value) = read() {
            return value;
        }
        assert!(Instant::now() < end, "{description}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn socket(rig: &Rig) -> &Path {
    let ServerId::Selected(Selector::Socket(path)) = &rig.server else {
        panic!("fixture must own a private socket");
    };
    path
}

fn pane_pid(rig: &Rig) -> u32 {
    rig.must(&["display-message", "-p", "-t", &rig.pane, "#{pane_pid}"])
        .trim()
        .parse()
        .expect("live pane process")
}

fn proof(rig: &Rig) -> Proven {
    Proven::new(&rig.session, &rig.pane, pane_pid(rig)).expect("canonical private pane")
}

fn attach(rig: &Rig, proven: &Proven) -> Sidecar {
    Sidecar::attach(&rig.server, proven, IO).expect("private sidecar attach")
}

fn frame(sidecar: &mut Sidecar) -> SidecarReply {
    assert!(sidecar.send(SidecarCommand::Frame));
    sidecar.reply(IO).expect("complete Frame reply")
}

fn assert_reaped(events: &[WaitChildEvent], pid: u32) {
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, WaitChildEvent::Reaped(id, _) if *id == pid))
            .count(),
        1,
        "the owned child must be reaped before return: {events:?}"
    );
}

fn assert_started_and_reaped(events: &[WaitChildEvent], pid: u32) {
    assert!(events.contains(&WaitChildEvent::Spawned(pid)), "{events:?}");
    assert_reaped(events, pid);
}

struct Wire {
    done: PathBuf,
    received: PathBuf,
}

impl Wire {
    fn start(rig: &Rig) -> Self {
        // The finish file is written only after the command receipt. A final
        // read drains any input tmux delivered just before that receipt.
        const RECEIVER: &str = r#"
use IO::Select;
my ($done, $received) = @ARGV;
STDOUT->autoflush(1);
print "d54-sidecar-wire-ready\r\n";
my $source = IO::Select->new(\*STDIN);
my $keys = "";
while (!-e $done || $source->can_read(0.050)) {
    next unless $source->can_read(0.025);
    my $n = sysread(STDIN, my $bytes, 1024);
    die "pane input closed" unless $n;
    $keys .= $bytes;
}
open(my $out, ">:raw", "$received.tmp") or die $!;
print {$out} $keys;
close($out) or die $!;
rename("$received.tmp", $received) or die $!;
sleep 60;
"#;
        let done = rig.path("done");
        let received = rig.path("received");
        let quote = |value: &str| format!("'{}'", value.replace('\'', "'\\''"));
        let command = format!(
            "stty raw -echo; exec perl -e {} {} {}",
            quote(RECEIVER),
            quote(&done.to_string_lossy()),
            quote(&received.to_string_lossy()),
        );
        rig.must(&["respawn-pane", "-k", "-t", &rig.pane, &command]);
        until(
            || {
                rig.must(&["capture-pane", "-p", "-t", &rig.pane])
                    .contains(READY)
                    .then_some(())
            },
            "the byte receiver did not start",
        );
        Self { done, received }
    }

    fn finish(self) -> Vec<u8> {
        std::fs::write(&self.done, "").expect("finish the byte receiver");
        until(
            || std::fs::read(&self.received).ok(),
            "the byte receiver did not publish its complete wire",
        )
    }
}

#[test]
fn the_owned_sidecar_attaches_exactly_and_uses_one_child_for_a_frame_and_one_space() {
    let rig = Rig::new("d54-exact");
    let wire = Wire::start(&rig);
    rig.must(&["new-window", "-d", "-t", "=current:", "sleep 60"]);
    rig.add_session("other");
    rig.must(&[
        "set-environment",
        "-t",
        "=current:",
        "TERM",
        "fixture-original",
    ]);
    let dimensions = || {
        rig.must(&[
            "display-message",
            "-p",
            "-t",
            "=current:",
            "#{window_id}|#{window_width}|#{window_height}",
        ])
    };
    let before = dimensions();
    let proven = proof(&rig);
    let ((pid, calls, effects), events) =
        wait_children_during(|| {
            let (mut sidecar, calls) = spawns_during(|| attach(&rig, &proven));
            let pid = sidecar.pid();
            let roster = rig.must(&[
            "list-clients", "-t", "=current", "-F",
            "#{client_pid}|#{session_id}|#{client_control_mode}|#{client_termname}|#{client_flags}",
        ]);
            let columns: Vec<_> = roster.trim_end().split('|').collect();
            assert_eq!(columns.len(), 5, "{roster:?}");
            assert_eq!(columns[0], pid.to_string());
            assert_eq!(columns[1], rig.session);
            assert_eq!(columns[2], "1");
            assert_eq!(columns[3], "ae-dictation");
            for flag in ["no-output", "ignore-size"] {
                assert!(columns[4].split(',').any(|item| item == flag), "{roster:?}");
            }
            assert_eq!(dimensions(), before);
            assert_eq!(
                rig.must(&["show-environment", "-t", "=current:", "TERM"])
                    .trim(),
                "TERM=fixture-original"
            );
            let ((), effects) = spawns_during(|| {
                let SidecarReply::Frame { facts, capture } = frame(&mut sidecar) else {
                    panic!("the private proved pane must produce a Frame");
                };
                assert_eq!(facts.session_id, rig.session);
                assert_eq!(facts.pane_id, rig.pane);
                assert_eq!(facts.pane_pid, proven.pane_pid());
                assert_eq!(capture.split('\n').count(), usize::from(facts.height));
                assert!(capture.contains(READY));
                assert!(sidecar.send(SidecarCommand::Space));
                assert_eq!(sidecar.reply(IO), Ok(SidecarReply::Spaced));
            });
            assert!(sidecar.close(), "close reaps the normal client");
            (pid, calls, effects)
        });
    let attach_calls: Vec<_> = calls
        .iter()
        .filter(|argv| argv.iter().any(|arg| arg == "-C"))
        .collect();
    assert_eq!(attach_calls.len(), 1, "{calls:?}");
    assert_eq!(
        attach_calls[0],
        &[
            "tmux",
            "-u",
            "-S",
            socket(&rig).to_str().expect("UTF-8 fixture socket"),
            "-C",
            "attach-session",
            "-E",
            "-f",
            "no-output,ignore-size",
            "-t",
            &format!("={}", rig.session),
        ]
    );
    assert!(effects.is_empty(), "no per-command process: {effects:?}");
    assert_started_and_reaped(&events, pid);
    rig.await_clients("current", 0);
    assert_eq!(wire.finish(), b" ");
}

#[test]
fn dropping_an_owned_sidecar_reaps_it_once_and_removes_only_its_client() {
    let rig = Rig::new("d54-drop");
    rig.attach("current", 0);
    let (pid, events) = wait_children_during(|| {
        let sidecar = attach(&rig, &proof(&rig));
        rig.await_clients("current", 2);
        let pid = sidecar.pid();
        drop(sidecar);
        pid
    });
    assert_started_and_reaped(&events, pid);
    rig.await_clients("current", 1);
}

#[test]
fn one_pending_command_is_the_limit_and_reuse_after_misuse_is_refused() {
    let rig = Rig::new("d54-pending");
    let wire = Wire::start(&rig);
    let (pid, events) = wait_children_during(|| {
        let mut sidecar = attach(&rig, &proof(&rig));
        let pid = sidecar.pid();
        assert!(sidecar.send(SidecarCommand::Frame));
        assert!(!sidecar.send(SidecarCommand::Space));
        assert!(!sidecar.send(SidecarCommand::Space));
        assert_eq!(sidecar.reply(IO), Err(SidecarLost::Stream));
        drop(sidecar);
        pid
    });
    assert_started_and_reaped(&events, pid);
    assert!(wire.finish().is_empty());
}

#[test]
fn a_moved_pane_with_the_same_id_and_pid_cannot_be_read_or_keyed() {
    let rig = Rig::new("d54-move");
    let wire = Wire::start(&rig);
    let proven = proof(&rig);
    rig.must(&["new-window", "-d", "-t", "=current:", "sleep 60"]);
    let (other, _, _) = rig.add_session("other");
    let (((read_pid, key_pid), reader, writer), events) = wait_children_during(|| {
        let mut reader = attach(&rig, &proven);
        let mut writer = attach(&rig, &proven);
        let ids = (reader.pid(), writer.pid());
        assert!(matches!(frame(&mut reader), SidecarReply::Frame { .. }));
        rig.must(&["move-pane", "-d", "-s", &rig.pane, "-t", "=other:"]);
        let owned = rig.must(&[
            "list-panes",
            "-t",
            "=other:",
            "-F",
            "#{session_id} #{pane_id} #{pane_pid}",
        ]);
        let moved = format!("{other} {} {}", rig.pane, proven.pane_pid());
        assert!(owned.lines().any(|row| row == moved), "{owned:?}");

        assert!(matches!(frame(&mut reader), SidecarReply::Refused(_)));
        assert!(!reader.send(SidecarCommand::Space));
        assert_eq!(reader.reply(IO), Err(SidecarLost::Stream));
        assert!(writer.send(SidecarCommand::Space));
        assert_eq!(
            writer.reply(IO),
            Ok(SidecarReply::Refused("ae-pane-changed".to_owned()))
        );
        assert!(!writer.send(SidecarCommand::Frame));
        assert_eq!(writer.reply(IO), Err(SidecarLost::Stream));
        (ids, reader, writer)
    });
    assert_started_and_reaped(&events, read_pid);
    assert_started_and_reaped(&events, key_pid);
    let ((), after) = wait_children_during(|| drop((reader, writer)));
    assert!(after.is_empty(), "Drop never reaps twice: {after:?}");
    assert!(wire.finish().is_empty());
}

#[test]
fn a_respawned_process_does_not_inherit_the_proved_panes_read_or_key_right() {
    let rig = Rig::new("d54-respawn");
    let original = proof(&rig);
    let ((read_pid, key_pid, wire), events) = wait_children_during(|| {
        let mut reader = attach(&rig, &original);
        let mut writer = attach(&rig, &original);
        let read_pid = reader.pid();
        let key_pid = writer.pid();
        assert!(matches!(frame(&mut reader), SidecarReply::Frame { .. }));
        let wire = Wire::start(&rig);
        assert_ne!(pane_pid(&rig), original.pane_pid());
        assert!(matches!(frame(&mut reader), SidecarReply::Refused(_)));
        assert!(!reader.send(SidecarCommand::Space));
        assert_eq!(reader.reply(IO), Err(SidecarLost::Stream));
        assert!(writer.send(SidecarCommand::Space));
        assert_eq!(
            writer.reply(IO),
            Ok(SidecarReply::Refused("ae-pane-changed".to_owned()))
        );
        assert!(!writer.send(SidecarCommand::Space));
        (read_pid, key_pid, wire)
    });
    assert_started_and_reaped(&events, read_pid);
    assert_started_and_reaped(&events, key_pid);
    assert!(wire.finish().is_empty());
}

#[test]
fn a_mute_socket_exhausts_the_shared_preflight_budget_without_an_attach_child() {
    let rig = Rig::new("d54-mute");
    let path = rig.path("mute");
    let mute = UnixListener::bind(&path).expect("own mute server socket");
    let server = ServerId::Selected(Selector::Socket(path));
    let proof = proof(&rig);
    let started = Instant::now();
    let ((result, calls), events) =
        wait_children_during(|| spawns_during(|| Sidecar::attach(&server, &proof, SHORT)));
    drop(mute);
    assert!(matches!(result, Err(SidecarLost::Timeout)));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(
        calls.iter().all(|argv| argv.iter().all(|arg| arg != "-C")),
        "{calls:?}"
    );
    let children: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            WaitChildEvent::Spawned(pid) => Some(*pid),
            _ => None,
        })
        .collect();
    assert!(
        !children.is_empty(),
        "a preflight child actually started: {events:?}"
    );
    for pid in children {
        assert_reaped(&events, pid);
    }
}

struct Suspended(String);

impl Drop for Suspended {
    fn drop(&mut self) {
        let _ = super::super::run("kill", &["-CONT".to_owned(), self.0.clone()]);
    }
}

fn suspend(rig: &Rig) -> Suspended {
    let pid = rig
        .must(&["display-message", "-p", "#{pid}"])
        .trim()
        .to_owned();
    assert!(pid.parse::<u32>().is_ok_and(|pid| pid > 1));
    let suspended = Suspended(pid);
    assert!(super::super::run("kill", &["-STOP".to_owned(), suspended.0.clone()]).0);
    suspended
}

#[test]
fn a_reply_deadline_kills_and_reaps_the_client_before_returning() {
    let rig = Rig::new("d54-timeout");
    let ((pid, sidecar), events) = wait_children_during(|| {
        let mut sidecar = attach(&rig, &proof(&rig));
        let pid = sidecar.pid();
        let stopped = suspend(&rig);
        assert!(sidecar.send(SidecarCommand::Frame));
        assert_eq!(sidecar.reply(SHORT), Err(SidecarLost::Timeout));
        assert!(!sidecar.send(SidecarCommand::Space));
        drop(stopped);
        (pid, sidecar)
    });
    assert_started_and_reaped(&events, pid);
    let ((), after) = wait_children_during(|| drop(sidecar));
    assert!(after.is_empty(), "Drop never reaps twice: {after:?}");
    rig.await_clients("current", 0);
}

#[test]
fn a_malformed_hook_block_closes_the_client_instead_of_matching_the_frame() {
    let rig = Rig::new("d54-malformed");
    let ((pid, sidecar), events) = wait_children_during(|| {
        let mut sidecar = attach(&rig, &proof(&rig));
        let pid = sidecar.pid();
        rig.must(&[
            "set-hook",
            "-t",
            "=current:",
            "after-display-message",
            "display-message -p unexpected-hook-block",
        ]);
        assert!(sidecar.send(SidecarCommand::Frame));
        assert_eq!(sidecar.reply(IO), Err(SidecarLost::Malformed));
        assert!(!sidecar.send(SidecarCommand::Space));
        (pid, sidecar)
    });
    assert_started_and_reaped(&events, pid);
    let ((), after) = wait_children_during(|| drop(sidecar));
    assert!(after.is_empty(), "Drop never reaps twice: {after:?}");
}

#[test]
fn release_wake_and_focus_hooks_are_refused_before_the_control_child_can_attach() {
    const CHANNEL: &str = "ae-wake-0-1764230000";
    let rig = Rig::new("d54-old-hook");
    let proven = proof(&rig);
    let old_focus = format!(
        "if-shell -F -t {} \"#{{==:#{{session_id}},{}}}\" \"select-window -t {} ; select-pane -t {}\"",
        rig.pane, rig.session, rig.pane, rig.pane,
    );
    for (index, command) in [(9, format!("wait-for -S {CHANNEL}")), (0, old_focus)] {
        let hook = format!("client-session-changed[{index}]");
        rig.must(&["set-hook", "-t", "=current:", &hook, &command]);
        let (result, calls) = spawns_during(|| Sidecar::attach(&rig.server, &proven, IO));
        assert!(
            matches!(result, Err(SidecarLost::Hooks(HookRefusal::Unguarded { index: seen })) if seen == index)
        );
        assert!(
            calls.iter().all(|argv| argv.iter().all(|arg| arg != "-C")),
            "{calls:?}"
        );
        rig.must(&["set-hook", "-u", "-t", "=current:", &hook]);
    }
    rig.await_clients("current", 0);
}

#[test]
fn even_a_guarded_detach_hook_is_refused_before_spawning() {
    let rig = Rig::new("d54-detach-hook");
    let command = concat!(
        "if-shell -F '#{?#{&&:#{client_control_mode},",
        "#{==:#{client_termname},ae-dictation}},0,1}' ",
        "{ wait-for -S ae-wake-0-1764230000 }",
    );
    rig.must(&["set-hook", "-t", "=current:", "client-detached[6]", command]);
    let (result, calls) = spawns_during(|| Sidecar::attach(&rig.server, &proof(&rig), IO));
    assert!(matches!(
        result,
        Err(SidecarLost::Hooks(HookRefusal::Unguarded { index: 6 }))
    ));
    assert!(
        calls.iter().all(|argv| argv.iter().all(|arg| arg != "-C")),
        "{calls:?}"
    );
}

#[test]
fn a_sidecar_neither_wakes_nor_focuses_the_session_but_a_terminal_viewer_does() {
    const CHANNEL: &str = "ae-wake-0-1764230000";
    let rig = Rig::new("d54-hook");
    let other = rig.must(&[
        "new-window",
        "-P",
        "-F",
        "#{window_id}",
        "-t",
        "=current:",
        "sleep 60",
    ]);
    let other = other.trim();
    let selected = || rig.must(&["display-message", "-p", "-t", "=current:", "#{window_id}"]);
    assert_eq!(selected().trim(), other);
    let focus = crate::session_tmux::Op::SetClientSessionHook {
        session_id: &rig.session,
        pane: &rig.pane,
        chat: None,
    };
    assert!(run_tmux_op(&crate::session_tmux::argv(&rig.server, &focus)).0);
    assert!(
        publish_cycle(
            &rig.server,
            &CycleBatch {
                session: rig.session.clone(),
                channel: Some(CHANNEL.to_owned()),
                ops: Vec::new(),
            }
        )
        .hook_proven
    );
    let mut wake = ChannelWaiter::start(&rig.server, CHANNEL).expect("owned wake waiter");
    let sidecar = attach(&rig, &proof(&rig));
    let wake_on_attach = wake.wait_until(Instant::now() + SHORT);
    let selected_during_attach = selected();
    let motion = observe_motion_panes(&rig.server, "current").expect("private motion census");
    let sidecar_only = run_tmux_wait(
        &rig.server,
        "current",
        "ae-d54-motion-probe",
        Instant::now() + SHORT,
    );
    assert!(sidecar.close());
    rig.await_clients("current", 0);
    let wake_on_detach = if wake_on_attach == Waited::TimedOut {
        wake.wait_until(Instant::now() + SHORT)
    } else {
        wake_on_attach
    };
    assert_eq!(
        wake_on_detach,
        Waited::TimedOut,
        "sidecar woke the watchdog on attach or detach ({wake_on_attach:?})"
    );
    rig.attach("current", 0);
    let wake_on_human = wake.wait_until(Instant::now() + IO);
    if wake_on_human == Waited::TimedOut {
        wake.release(&rig.server);
    }
    assert_eq!(wake_on_human, Waited::Woke);
    assert_eq!(selected_during_attach.trim(), other);
    assert_eq!(selected().trim(), rig.window);
    assert!(!motion.is_empty() && motion.iter().all(|pane| pane.session_attached == 0));
    assert_eq!(sidecar_only, Waited::TimedOut);
    let human = observe_motion_panes(&rig.server, "current").expect("human motion census");
    assert!(!human.is_empty() && human.iter().all(|pane| pane.session_attached == 1));
}
