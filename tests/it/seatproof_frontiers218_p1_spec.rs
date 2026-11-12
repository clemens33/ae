// Saved frontier additions. Include after the unchanged nine-case child.
mod frontiers218_p1 {
    use super::*;

    const KEYS: [&str; 3] = ["_AE_EXPECT_SLOT", "_AE_EXPECT_LAUNCH", "_AE_EXPECT_LEGACY"];
    const OTHER_ID: &str = "0199c0de-2222-4890-abcd-ef0123456789";
    const ROLE: &str = "orchestrator role refuses unproven/outside initiation; use its pane, ae chat, ae app or Telegram";
    const ENTER_FAULT: &str = r#"#!/usr/bin/perl
use strict; use warnings;
if (@ARGV && $ARGV[-1] eq 'Enter' &&
    (grep { $_ eq 'send-keys' } @ARGV) &&
    (grep { $_ eq $ENV{SEATPROOF_PANE} } @ARGV)) {
    open(my $seen, '>>', "$ENV{SEATPROOF_BARRIER}/enter-attempted") or die;
    print $seen "real submit Enter path\n"; close($seen);
    if (($ENV{FRONTIER218_BLOCK_ENTER}//'') eq '1') {
        open(my $f, '>>', "$ENV{SEATPROOF_BARRIER}/enter-failed") or die;
        print $f "native Enter command failed\n"; close($f);
        exit 1;
    }
}
exec {$ENV{SEATPROOF_TMUX}} $ENV{SEATPROOF_TMUX}, @ARGV;
die 'exec real private tmux';
"#;

    fn matching(w: &World, caller: &str, cross: bool) -> Runner {
        let mut command = w.rig.command(Some(caller));
        command.args(["@s", "send"]);
        if cross {
            command.arg("--cross-session");
        }
        command.args(["kid", BODY]);
        command.env_remove("AE_SENDER_OVERRIDE");
        for key in KEYS {
            command.env_remove(key);
        }
        command
            .env("_AE_EXPECT_SLOT", SLOT)
            .env("_AE_EXPECT_LAUNCH", "tok-kid")
            .env("AE_SEND_DEFER_SEC", "0")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    fn foreign(w: &World, tag: &str) -> String {
        let [main, peer] = w.rig.create("other", OTHER_ID);
        // The first packet already covered the original three ready panes.
        // This second packet covers exactly these two new ready payloads plus
        // the known original main. Union by (socket, pid, birth), not row count.
        // Packet labels main/colead/kid map to other-main/other-peer/s-main here.
        w.rig.world218_capture(tag, [&main, &peer, &w.main]);
        main
    }

    fn delivered(w: &World, output: &Output) {
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        wait("SETUP: real target consumed body", || {
            read(&w.out).contains(BODY)
        });
        assert_eq!(w.count("send"), 1, "{}", w.events());
        assert_eq!(w.count("refused"), 0, "{}", w.events());
    }

    fn refusal_in(w: &World, session: &str, output: &Output, reason: &str) {
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("REFUSED") && stderr.contains(reason),
            "{stderr}"
        );
        assert!(
            !stderr.contains("ABANDONED") && !stderr.contains("UNCONFIRMED"),
            "{stderr}"
        );
        let events = read(w.rig.dir(session).join("events.jsonl"));
        assert_eq!(
            events.matches("\"action\":\"refused\"").count(),
            1,
            "{events}"
        );
        assert_eq!(events.matches("\"action\":\"send\"").count(), 0, "{events}");
        assert_eq!(w.count("send"), 0, "{}", w.events());
    }

    #[test]
    fn r7_matching_expectation_keeps_role_outside_initiation_refusal() {
        let tag = "r218-p1f-role-outside";
        let w = World::new(tag);
        let caller = foreign(&w, tag);
        w.row("meta_agent", Some("true"));
        let before = w.screen();
        // Cross-session permission cannot grant permission to initiate a Role.
        let output = matching(&w, &caller, true)
            .output()
            .expect("SETUP: real role helper");
        w.untouched(&before);
        refusal_in(&w, "other", &output, ROLE);
    }

    #[test]
    fn r7_matching_expectation_keeps_inside_role_initiation_control() {
        let w = World::new("r218-p1f-role-inside");
        w.row("meta_agent", Some("true"));
        // The stamped caller belongs to the exact physical Role session.
        let output = matching(&w, &w.main, true)
            .output()
            .expect("SETUP: eligible role helper");
        delivered(&w, &output);
    }

    #[test]
    fn r7_matching_expectation_keeps_cross_session_refusal() {
        let tag = "r218-p1f-cross-refused";
        let w = World::new(tag);
        let caller = foreign(&w, tag);
        let before = w.screen();
        let output = matching(&w, &caller, false)
            .output()
            .expect("SETUP: real foreign caller");
        w.untouched(&before);
        refusal_in(
            &w,
            "other",
            &output,
            "another ae session; pass --cross-session only when the human explicitly instructed it",
        );
    }

    #[test]
    fn r7_matching_expectation_keeps_authorized_cross_session_control() {
        let tag = "r218-p1f-cross-allowed";
        let w = World::new(tag);
        let caller = foreign(&w, tag);
        let output = matching(&w, &caller, true)
            .output()
            .expect("SETUP: explicit cross-session helper");
        delivered(&w, &output);
    }

    fn physical(tag: &str, fail: bool) {
        let w = World::new(tag);
        let before = w.screen();
        let mut command = matching(&w, &w.main, false);
        w.barrier(&mut command);
        w.resume();
        command
            .env("SEATPROOF_PANE", &w.pane)
            .env("SEATPROOF_HOLD_VIEWER", "0");
        if fail {
            command.env("SEATPROOF_FAULT", "physical");
        }
        let output = command
            .output()
            .expect("SETUP: physical-observation helper");
        if fail {
            assert!(
                w.rig.scratch.join("barrier/physical-failed").is_file(),
                "SETUP: fault never fired"
            );
            w.untouched(&before);
            w.refuse(&output, "could not be proven");
            assert_eq!(w.count("refused"), 1, "{}", w.events());
        } else {
            assert!(
                w.rig.scratch.join("barrier/viewer-reads").is_file(),
                "SETUP: physical control bypassed wrapper"
            );
            delivered(&w, &output);
        }
    }

    #[test]
    fn r2_matching_expectation_keeps_physical_io_failure_refusal() {
        physical("r218-p1f-physical-failed", true);
    }

    #[test]
    fn r2_matching_expectation_keeps_physical_observation_control() {
        physical("r218-p1f-physical-control", false);
    }

    /// Prove the held tool still shows the box as it was before the paste,
    /// then let it read the paste and every key after it.
    fn release(w: &World) {
        wait("SETUP: the tool held the paste", || {
            w.out.with_extension("held").is_file()
        });
        assert!(
            read(w.out.with_extension("wire")).ends_with("\x1b[200~"),
            "SETUP: the tool read past the paste opener while held"
        );
        let frame = w.rig.tmux(&["capture-pane", "-p", "-t", &w.pane]);
        assert!(
            frame.lines().any(|row| row.trim_end() == "❯"),
            "SETUP: the held frame does not show an empty box: {frame}"
        );
        fs::write(w.out.with_extension("go"), "").expect("SETUP: release render");
    }

    fn submit(tag: &str, fail: bool, held: bool) {
        let w = World::new(tag);
        if held {
            // The tool reads nothing after the paste opener until the helper
            // returned: every capture the helper takes shows an empty box.
            fs::write(w.out.with_extension("hold"), "").expect("SETUP: hold render");
        }
        let binding = fs::read(w.meta()).expect("SETUP: capture unchanged binding");
        let stamps = w.rig.tmux(&[
            "display-message",
            "-p",
            "-t",
            &w.pane,
            "#{@ae_slot} #{@ae_agent}",
        ]);
        let mut command = matching(&w, &w.main, false);
        w.barrier(&mut command);
        // Replace only this private client's transport wrapper. Every command
        // still reaches real tmux except the deliberately failing Enter command.
        executable(&w.rig.scratch.join("barrier/tmux"), ENTER_FAULT);
        command
            .env("SEATPROOF_PANE", &w.pane)
            .env("FRONTIER218_BLOCK_ENTER", if fail { "1" } else { "0" });
        let output = command.output().expect("SETUP: real post-paste helper");
        if held {
            release(&w);
        }
        assert!(
            w.rig.scratch.join("barrier/enter-attempted").is_file(),
            "SETUP: submit wrapper never reached"
        );
        if fail {
            assert!(
                w.rig.scratch.join("barrier/enter-failed").is_file(),
                "SETUP: no native Enter failure"
            );
            wait("SETUP: real paste reached unchanged target", || {
                let wire = read(w.out.with_extension("wire"));
                wire.contains(BODY) && wire.ends_with("\x1b[201~")
            });
            assert_eq!(
                fs::read(w.meta()).expect("binding after paste"),
                binding,
                "SETUP: binding changed"
            );
            assert_eq!(
                w.rig.tmux(&[
                    "display-message",
                    "-p",
                    "-t",
                    &w.pane,
                    "#{@ae_slot} #{@ae_agent}"
                ]),
                stamps,
                "SETUP: stamps changed"
            );
            assert_eq!(output.status.code(), Some(0), "{output:?}");
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(
                error.contains("UNCONFIRMED") && error.contains("message may not have sent"),
                "{error}"
            );
            assert!(
                !error.contains("REFUSED") && !error.contains("ABANDONED"),
                "{error}"
            );
            assert!(
                read(&w.out).is_empty(),
                "native failed Enter nevertheless submitted"
            );
            assert_eq!(w.count("unconfirmed"), 0, "{}", w.events());
            assert_eq!(w.count("send"), 1, "{}", w.events());
            assert_eq!(w.count("refused"), 0, "{}", w.events());
            let events = w.events();
            let pending = events
                .lines()
                .find(|line| line.contains("\"action\":\"send\""))
                .expect("pending send attempt");
            assert!(
                pending.contains("\"summary\":\"[unconfirmed] ") && pending.contains(BODY),
                "pending send summary lost its marker or original body: {pending}"
            );
            assert!(
                fs::read_dir(w.dir().join("messages"))
                    .expect("pending body directory")
                    .any(|entry| {
                        let path = entry.expect("pending body file").path();
                        pending.contains(&format!("\"body_file\":\"{}\"", path.display()))
                            && read(&path).contains(BODY)
                    }),
                "pending send does not name its preserved messages body: {pending}"
            );
            assert!(
                fs::read_dir(w.dir().join("messages"))
                    .expect("preserved body directory")
                    .any(|entry| read(entry.expect("stored body").path()).contains(BODY)),
                "original body lost"
            );
        } else {
            assert!(
                !w.rig.scratch.join("barrier/enter-failed").exists(),
                "SETUP: unexpected fault"
            );
            delivered(&w, &output);
        }
    }

    #[test]
    fn r8_matching_expectation_keeps_unchanged_binding_unconfirmed() {
        submit("r218-p1f-unconfirmed", true, false);
    }

    #[test]
    fn r8_matching_expectation_keeps_submit_forwarding_control() {
        submit("r218-p1f-submit-control", false, false);
    }

    #[test]
    fn r8_a_refused_enter_stays_unconfirmed_while_the_paste_is_undrawn() {
        submit("r218-p1f-held-unconfirmed", true, true);
    }

    #[test]
    fn r8_an_accepted_enter_still_delivers_while_the_paste_is_undrawn() {
        submit("r218-p1f-held-control", false, true);
    }

    #[test]
    fn r8_matching_expectation_qualifies_occupied_unviewed_draft() {
        let w = World::new("r218-p1f-draft-occupied");
        w.rig.tmux(&["set-buffer", "-b", "p1-draft", "human draft"]);
        w.rig
            .tmux(&["paste-buffer", "-p", "-d", "-b", "p1-draft", "-t", &w.pane]);
        let wire = w.out.with_extension("wire");
        wait("SETUP: completed real draft paste", || {
            read(&wire).ends_with("\x1b[201~")
        });
        fs::write(&wire, "").expect("SETUP: reset draft receipt");
        let before = w.screen();
        // Match the frozen nine-case draft's helper attribution entry exactly.
        // This is not the Role/cross-session constructor's authorization proof.
        let output = matching(&w, &w.main, false)
            .env("AE_SENDER_OVERRIDE", "spec218")
            .output()
            .expect("SETUP: occupied unviewed draft helper");
        eprintln!(
            "p1draft218 BEGIN tag=r218-p1f-draft-occupied native={:?}",
            output.status.code()
        );
        eprintln!(
            "p1draft218 stdout: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        eprintln!(
            "p1draft218 stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        eprintln!("p1draft218 END tag=r218-p1f-draft-occupied");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("composer occupied")
                && !error.contains("human input or attention")
                && !error.contains("composer unreadable")
                && !error.contains("human-only prompt"),
            "SETUP: draft did not isolate occupied/viewed=false: {error}"
        );
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(
            error.contains("ABANDONED") && error.contains("Re-send."),
            "{error}"
        );
        assert!(
            !error.contains("REFUSED") && !error.contains("UNCONFIRMED"),
            "{error}"
        );
        w.untouched(&before);
        assert_eq!(w.count("delivery-abandoned"), 1, "{}", w.events());
        assert_eq!(w.count("refused"), 0, "{}", w.events());
        assert!(
            w.events().contains("refused: "),
            "diagnostic missing: {}",
            w.events()
        );
        assert!(
            fs::read_dir(w.dir().join("messages"))
                .expect("preserved draft body")
                .any(|entry| read(entry.expect("stored draft body").path()).contains(BODY)),
            "original body lost"
        );
    }
}
