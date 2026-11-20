// Independent #218 slice 2c acceptance. Include in watchdog_daemon::tests.
// The child is the real send owner, not a mock of the incarnation comparison.
// The external helper boundary alone changes the owned meta after the cycle
// decided its effect. Every tmux command uses quietwd_spec's private socket.
#[allow(
    clippy::disallowed_methods,
    reason = "owned acceptance fixtures and child-only environment"
)]
mod seatinc_acceptance {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    const UUID: &str = "11111111-1111-4111-8111-111111111111";
    const CHILD: &str =
        "l4_orchestration::watchdog_daemon::tests::seatinc_acceptance::seatinc_real_send_child";
    const AGENT: &str = r#"#!/usr/bin/perl
use strict; use warnings;
system('stty raw -echo'); binmode(STDIN, ':raw'); $|=1;
my $out=$ARGV[0]; my $buf=''; my $paste=0;
my $border="\xe2\x94\x80" x 399;
sub draw { my $text=$buf; $text =~ s/[\r\n]/ /g;
print "\e[H\e[2J\e[?2004h✻ Crunched for 1s · done 4:01 PM\r\n$border\r\n❯ $text\r\n$border\r\n🧠 fake-model ~/x\r\n⏵⏵ bypass permissions on\r\n"; }
draw(); while (sysread(STDIN,my $ch,1)) {
 open(my $wire,'>>',"$out.wire") or die; binmode($wire); print $wire $ch; close($wire);
 $buf.=$ch;
 if ($buf =~ s/\e\[200~\z//) { $paste=1; next; }
 if ($buf =~ s/\e\[201~\z//) { $paste=0; draw(); next; }
 if (!$paste && ($ch eq "\r" || $ch eq "\n")) {
  open(my $f,'>>',$out) or die; print $f $buf,"\0"; close($f); $buf=''; draw();
 }
}
"#;

    struct Rig {
        tmux: crate::transport::quietwd_spec::Rig,
        dir: PathBuf,
        helper: SendHelper,
        output: PathBuf,
    }

    fn quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\\''"))
    }

    fn executable(path: &Path, text: &str) {
        std::fs::write(path, text).expect("owned fixture executable");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .expect("owned fixture mode");
    }

    impl Rig {
        fn new(tag: &str, real: bool) -> Self {
            let tmux = crate::transport::quietwd_spec::Rig::new(tag);
            let dir = tmux.path("current");
            std::fs::create_dir(&dir).expect("owned session directory");
            tmux.must(&["set-option", "-t", "=current:", "@ae_session_uuid", UUID]);
            tmux.must(&["set-option", "-p", "-t", &tmux.pane, "@ae_slot", "main"]);
            tmux.must(&["resize-window", "-t", &tmux.window, "-x", "400", "-y", "40"]);
            let socket = match &tmux.server {
                ServerId::Selected(Selector::Socket(path)) => path,
                other => panic!("private socket required: {other:?}"),
            };
            std::fs::write(dir.join("meta"), format!(
                "session=current\nsession_id={UUID}\nmode=local\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=claude\nlaunch_id.main=decided\npane.main={}\n",
                socket.display(), tmux.pane
            )).expect("owned meta snapshot");
            let output = dir.join("accepted");
            if real {
                let agent = dir.join("agent");
                executable(&agent, AGENT);
                // Perl is a real non-shell foreground process. The watchdog's
                // shell-death predicate therefore cannot suppress the nudge.
                let command = format!(
                    "exec perl {} {}",
                    quote(&agent.display().to_string()),
                    quote(&output.display().to_string())
                );
                tmux.must(&["respawn-pane", "-k", "-t", &tmux.pane, &command]);
                wait("complete fake-agent frame", || {
                    tmux.must(&["capture-pane", "-p", "-t", &tmux.pane])
                        .contains("bypass permissions on")
                });
                let frame = tmux.must(&["capture-pane", "-p", "-t", &tmux.pane]);
                assert_eq!(
                    crate::harness_state::classify(&frame, crate::tool::ToolKind::Claude),
                    crate::harness_state::HarnessState::Idle,
                    "fixture must draw an idle frame: {frame}"
                );
            }
            let helper = SendHelper::for_session(&dir);
            let root = quote(&dir.display().to_string());
            let finish = if real {
                let exe = quote(
                    &std::env::current_exe()
                        .expect("test executable")
                        .display()
                        .to_string(),
                );
                format!(
                    "unset TMUX TMUX_PANE\nexport SEATINC_DIR={root}\nexport SEATINC_TARGET=\"$1\" SEATINC_BODY=\"$2\"\nexec {exe} --exact {CHILD} --ignored --nocapture\n"
                )
            } else {
                "exit 0\n".into()
            };
            executable(
                helper.path(),
                &format!(
                    "#!/bin/sh\nprintf '%s\\t%s\\t%s\\t%s\\t%s\\t%s\\n' \"$1\" \"$_AE_EVENT_ACTION\" \"$_AE_EXPECT_SLOT\" \"$_AE_EXPECT_LAUNCH\" \"$_AE_EXPECT_LEGACY\" \"$_AE_EVENT_REF\" >> {root}/captured\nif [ -f {root}/replace ]; then\n perl -pi -e 's/^launch_id\\.main=.*/launch_id.main=successor/' {root}/meta\nfi\n{finish}"
                ),
            );
            Self {
                tmux,
                dir,
                helper,
                output,
            }
        }

        fn cycle(&self) -> Cycle<'_> {
            let mut cycle = demo_cycle(&self.dir, &self.helper, &self.tmux.server);
            cycle.session = "current";
            cycle.meta_agent = false;
            cycle.knobs.quota_aware = false;
            cycle.launch_ids = vec![("main".into(), "decided".into())];
            let bytes = crate::meta::read_bytes(&self.dir).expect("owned snapshot");
            cycle.roster = Meta::parse(&String::from_utf8_lossy(&bytes))
                .roster()
                .to_vec();
            cycle
        }

        fn replace_on_send(&self) {
            std::fs::write(self.dir.join("replace"), "yes").expect("owned race arm");
        }

        fn captured(&self) -> Vec<Vec<String>> {
            std::fs::read_to_string(self.dir.join("captured"))
                .unwrap_or_default()
                .lines()
                .map(|line| line.split('\t').map(str::to_owned).collect())
                .collect()
        }

        fn assert_expected(&self, action: &str, slot: &str, launch: &str) {
            let rows = self.captured();
            assert_eq!(rows.len(), 1, "one helper dispatch: {rows:?}");
            assert_eq!(rows[0][1], action);
            assert_eq!(
                (&*rows[0][2], &*rows[0][3], &*rows[0][4]),
                (slot, launch, ""),
                "decision snapshot must reach the helper: {rows:?}"
            );
        }

        fn assert_result(&self, action: &str, replaced: bool) {
            let events = read_events(&self.dir);
            let refused: Vec<_> = events.iter().filter(|e| e.action == "refused").collect();
            let delivered = events.iter().filter(|e| e.action == action).count();
            if replaced {
                assert_eq!(
                    std::fs::read(self.output.with_extension("wire")).unwrap_or_default(),
                    b"",
                    "a successor must receive no byte"
                );
                assert_eq!(refused.len(), 1, "one refusal: {events:?}");
                assert!(
                    refused[0]
                        .summary
                        .as_deref()
                        .unwrap_or_default()
                        .contains("was replaced; NOTHING further was typed."),
                    "{refused:?}"
                );
                assert_eq!(
                    delivered, 0,
                    "no success or attempt record for stale action"
                );
            } else {
                wait("accepted continuation", || {
                    std::fs::read(&self.output).is_ok_and(|v| !v.is_empty())
                });
                assert_eq!(
                    refused.len(),
                    0,
                    "unchanged incarnation delivers: {events:?}"
                );
                assert_eq!(delivered, 1, "one action record: {events:?}");
            }
        }
    }

    fn wait(label: &str, ready: impl Fn() -> bool) {
        let end = std::time::Instant::now() + Duration::from_secs(30);
        while !ready() {
            assert!(
                std::time::Instant::now() < end,
                "fixture did not reach {label}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    #[ignore = "external helper child only, invoked by this fixture"]
    fn seatinc_real_send_child() {
        let dir = PathBuf::from(std::env::var_os("SEATINC_DIR").expect("owned child dir"));
        assert!(
            dir.join("captured").is_file(),
            "never run this child outside its fixture"
        );
        let tail = [
            std::env::var("SEATINC_TARGET").expect("target"),
            std::env::var("SEATINC_BODY").expect("body"),
        ];
        let code = crate::send::run(
            &dir,
            &tail,
            &crate::send_env(),
            "",
            "current",
            "current",
            crate::time::Timestamp::now(),
            Duration::ZERO,
            &mut std::io::stdout(),
            &mut std::io::stderr(),
        )
        .expect("real send owner");
        std::process::exit(i32::from(code));
    }

    fn real_cycle_nudge(replaced: bool) {
        let rig = Rig::new("seatinc-cycle", true);
        if replaced {
            rig.replace_on_send();
        }
        let cycle = rig.cycle();
        let frame = rig.tmux.must(&["capture-pane", "-p", "-t", &rig.tmux.pane]);
        let now = crate::time::Timestamp::now().epoch();
        let mut carry = Carry::new();
        carry.panes.push((
            rig.tmux.pane.clone(),
            PaneState {
                identity: Some(quiet_hash("main\nlead")),
                prev_hash: Some(quiet_hash(&frame)),
                last_hash_change: Some(now - 10_000),
                idle_since_epoch: Some(now - 10_000),
                ..PaneState::default()
            },
        ));
        cycle.run(&mut carry, &mut Vec::new()).expect("real cycle");
        rig.assert_result("nudge", replaced);
        rig.assert_expected("nudge", "main", "decided");
    }

    #[test]
    fn seatinc_real_cycle_nudge_refuses_successor_before_any_input() {
        real_cycle_nudge(true);
    }

    #[test]
    fn seatinc_real_cycle_nudge_delivers_to_unchanged_incarnation() {
        real_cycle_nudge(false);
    }

    #[test]
    fn seatinc_effect_legs_forward_the_decided_launch_and_keep_event_refs() {
        let moved = crate::wait_on::Moved {
            class: crate::wait_on::Class::Done,
            ordinal: 7,
            booked: 0,
            attempts: 0,
            current: true,
            at: crate::time::Timestamp::now(),
            reason: "finished".into(),
        };
        let effects = [
            (Effect::Nudge, "nudge", ""),
            (
                Effect::LimitWake { reset: 123 },
                crate::watchdog::LIMIT_WAKE_ACTION,
                "123",
            ),
            (
                Effect::DoneChallenge {
                    confirmations: 0,
                    required: 2,
                    done_age_secs: 600,
                },
                "done-challenge",
                "decided",
            ),
            (
                Effect::WaitChallenge {
                    confirmations: 0,
                    required: 2,
                    wait_age_secs: 600,
                    state: WaitState::WaitingAgent,
                    escalated: false,
                },
                "wait-challenge",
                "decided",
            ),
            (
                Effect::TargetMoved {
                    moved,
                    age_secs: 10,
                },
                "target-moved",
                "7",
            ),
            (Effect::SweepNudge, "sweep-nudge", ""),
        ];
        for (effect, action, reference) in effects {
            let rig = Rig::new("seatinc-effect", false);
            let cycle = rig.cycle();
            // The decision snapshot stays old while current meta changes. A
            // caller that re-reads at send time must fail this assertion.
            std::fs::write(
                rig.dir.join("meta"),
                std::fs::read_to_string(rig.dir.join("meta"))
                    .expect("meta")
                    .replace("=decided", "=successor"),
            )
            .expect("owned replacement");
            let overview = OverviewReading {
                rendered: "fleet".into(),
                hash: "0123456789abcdef".into(),
                checkpoint: crate::monitor::OverviewCheckpoint {
                    hash: None,
                    outstanding_since: None,
                    last_delivered_at: None,
                },
            };
            let mut observation = seen();
            observation.sweep = Some(SweepObservation::new(at(0), None));
            let on = super::super::Acting {
                agent: "lead",
                slot: "main",
                pane: &rig.tmux.pane,
                seen: &observation,
                events: &[],
                overview: Some(&overview),
            };
            cycle
                .apply(&effect, &on, &mut PaneState::default(), &mut Vec::new())
                .expect("effect");
            rig.assert_expected(action, "main", "decided");
            assert_eq!(
                rig.captured()[0][5],
                reference,
                "event refs are independent of the expected launch"
            );
        }
    }

    #[test]
    fn seatinc_quota_advisory_and_checkpoint_forward_the_recipient_incarnation() {
        for checkpoint in [false, true] {
            let rig = Rig::new("seatinc-quota", false);
            let cycle = rig.cycle();
            let roster = [codex_seat("main", "lead", "/tmp/cx")];
            let candidates = quota_ask_candidates(&roster, &live_panes(&roster));
            let recipients = [quota_recipient("main", "lead")];
            let mut carry = QuotaCarry::default();
            let _ = carry.reconcile_with_candidates(
                &codex_scope(&roster[0], "50", 9_900),
                &recipients,
                &candidates,
                &rig.dir,
            );
            let actions = carry.reconcile_with_candidates(
                &codex_scope(&roster[0], "96", 9_901),
                &recipients,
                &candidates,
                &rig.dir,
            );
            let actions: Vec<_> = actions
                .into_iter()
                .filter(|a| {
                    if checkpoint {
                        matches!(a, QuotaAction::Ask(_))
                    } else {
                        matches!(a, QuotaAction::Deliver(_))
                    }
                })
                .collect();
            assert_eq!(actions.len(), 1, "reachable quota delivery");
            std::fs::write(
                rig.dir.join("meta"),
                std::fs::read_to_string(rig.dir.join("meta"))
                    .expect("deciding meta")
                    .replace("=decided", "=successor"),
            )
            .expect("recipient replaced after quota decision");
            cycle
                .apply_quota_actions(&mut carry, actions, 10_000, &mut Vec::new())
                .expect("quota action");
            rig.assert_expected(
                if checkpoint {
                    crate::quota::action::CHECKPOINT
                } else {
                    crate::quota::action::ADVISORY
                },
                "main",
                "decided",
            );
        }
    }

    #[test]
    fn seatinc_dead_notice_binds_the_spawner_instead_of_the_dead_seat() {
        let rig = DnRig::new("parent", true);
        let root = quote(&rig.dir.display().to_string());
        executable(
            rig.helper.path(),
            &format!(
                "#!/bin/sh\nprintf '%s\\t%s\\t%s\\t%s\\n' \"$1\" \"$_AE_EVENT_ACTION\" \"$_AE_EXPECT_SLOT\" \"$_AE_EXPECT_LAUNCH\" > {root}/expectation\n"
            ),
        );
        rig.shell();
        let cycle = rig.cycle();
        std::fs::write(
            rig.dir.join("meta"),
            std::fs::read_to_string(rig.dir.join("meta"))
                .expect("deciding meta")
                .replace("parent-first", "parent-successor"),
        )
        .expect("spawner replaced after decision");
        cycle
            .run(&mut Carry::new(), &mut Vec::new())
            .expect("real dead-notice cycle");
        let fields =
            std::fs::read_to_string(rig.dir.join("expectation")).expect("helper environment");
        assert_eq!(
            fields.trim_end(),
            "parent\tdead-notice\tspawned.1\tparent-first"
        );
    }

    #[test]
    fn seatinc_cycle_snapshot_legacy_is_positive_absence_and_damage_refuses_once() {
        type Case = (
            &'static str,
            &'static [u8],
            Option<(&'static str, &'static str)>,
        );
        let cases: &[Case] = &[
            ("legacy", b"", Some(("", "1"))),
            (
                "literal-legacy",
                b"launch_id.main=legacy\n",
                Some(("legacy", "")),
            ),
            (
                "duplicate",
                b"launch_id.main=decided\nlaunch_id.main=decided\n",
                None,
            ),
            ("empty", b"launch_id.main=\n", None),
            ("non-utf8", b"launch_id.main=\xff\n", None),
        ];
        for (label, rows, expected) in cases {
            let rig = Rig::new("seatinc-snapshot", false);
            let mut cycle = rig.cycle();
            let mut snapshot = b"session=current\nseat.main=lead\n".to_vec();
            snapshot.extend_from_slice(rows);
            cycle.launch_ids = super::super::seat_launches(&snapshot);
            // Current meta has a usable token. The earlier snapshot must
            // decide legacy versus damage without adopting that later token.
            let observation = seen();
            let on = super::super::Acting {
                agent: "lead",
                slot: "main",
                pane: &rig.tmux.pane,
                seen: &observation,
                events: &[],
                overview: None,
            };
            cycle
                .apply(
                    &Effect::Nudge,
                    &on,
                    &mut PaneState::default(),
                    &mut Vec::new(),
                )
                .expect("snapshot effect");
            let captured = rig.captured();
            if let Some((launch, legacy)) = expected {
                assert_eq!(captured.len(), 1, "{label}: {captured:?}");
                assert_eq!(
                    (&*captured[0][2], &*captured[0][3], &*captured[0][4]),
                    ("main", *launch, *legacy),
                    "{label}"
                );
            } else {
                assert!(
                    captured.is_empty(),
                    "{label}: damaged snapshot cannot dispatch: {captured:?}"
                );
                let events = read_events(&rig.dir);
                let refusals: Vec<_> = events
                    .iter()
                    .filter(|event| event.action == "refused")
                    .collect();
                assert_eq!(refusals.len(), 1, "{label}: one refusal: {events:?}");
                let summary = refusals[0].summary.as_deref().unwrap_or_default();
                assert!(
                    summary.contains("seat lead (main) could not be proven"),
                    "{label}: {summary}"
                );
                assert!(
                    summary.contains("NOTHING further was typed."),
                    "{label}: {summary}"
                );
            }
        }
    }

    #[test]
    fn seatinc_session_notice_trigger_carries_cycle_incarnation_into_early_dispatch() {
        for replaced in [false, true] {
            let rig = Rig::new("seatinc-session-notice", true);
            if replaced {
                rig.replace_on_send();
            }
            // A rename follows launch, so the source is valid for either
            // incarnation: the stale cycle expectation is the deciding axis.
            crate::store::open(&rig.dir).append_event("{\"ts\":\"2026-11-17T00:00:00Z\",\"actor\":\"ae:rename\",\"action\":\"rename\",\"summary\":\"old -> current\"}\n").expect("rename source");
            let bytes = std::fs::read(rig.dir.join("meta")).expect("fixture meta");
            let meta = crate::meta::Meta::parse(std::str::from_utf8(&bytes).expect("valid meta"));
            let seat = crate::session_notice::Seat::read(&meta, &bytes, "main", "current");
            assert!(
                crate::session_notice::due(
                    &read_events(&rig.dir),
                    &seat,
                    crate::time::Timestamp::now().epoch(),
                    2
                )
                .is_some(),
                "fixture must owe a rename notice"
            );
            let verdict = Verdict::Active.into();
            rig.cycle().tell_seats(
                &[("main".into(), verdict)],
                &read_events(&rig.dir),
                crate::time::Timestamp::now().epoch(),
            );
            rig.assert_result(crate::session_notice::TOLD_ACTION, replaced);
            rig.assert_expected(crate::session_notice::TRIGGER_ACTION, "main", "decided");
        }
    }
}
