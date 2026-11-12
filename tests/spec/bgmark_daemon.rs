// Include inside watchdog_daemon.rs's existing cfg(test) mod tests.
mod bgmark_acceptance {
    use std::fmt::Write as _;

    use super::*;
    use crate::watchdog_daemon::{SeatVerdict, mark_with_task};

    fn seat(verdict: Verdict, task: bool) -> SeatVerdict {
        SeatVerdict { verdict, task }
    }

    #[test]
    fn bgmark_every_verdict_crosses_all_three_task_readings() {
        for (verdict, base, eligible) in [
            (Verdict::Dead, Mark::Dead, false),
            (Verdict::Quiet(QuietKind::Blocked), Mark::NeedsYou, false),
            (
                Verdict::Quiet(QuietKind::WaitingUser),
                Mark::NeedsYou,
                false,
            ),
            (
                Verdict::Quiet(QuietKind::WaitingAgent),
                Mark::WaitingAgent,
                true,
            ),
            (Verdict::Quiet(QuietKind::Done), Mark::Done, true),
            (Verdict::Throttled, Mark::NeedsYou, false),
            (Verdict::Limit, Mark::NeedsYou, false),
            (Verdict::HumanPrompt, Mark::NeedsYou, false),
            (Verdict::Idle, Mark::Idle, true),
            (Verdict::Stale, Mark::Stale, false),
            (Verdict::Unreachable, Mark::Stale, false),
            (Verdict::Active, Mark::Working, false),
            (
                Verdict::Meta(SweepVerdict::MetaSweeping),
                Mark::Working,
                false,
            ),
            (
                Verdict::Meta(SweepVerdict::MetaWedged),
                Mark::NeedsYou,
                false,
            ),
            (
                Verdict::Meta(SweepVerdict::MetaStarting),
                Mark::Stale,
                false,
            ),
        ] {
            for task in [None, Some(false), Some(true)] {
                let expected = if eligible && task == Some(true) {
                    Mark::Background
                } else {
                    base
                };
                assert_eq!(
                    mark_with_task(verdict, task),
                    expected,
                    "{verdict:?} {task:?}"
                );
                assert_eq!(seat(verdict, task == Some(true)).mark(), expected);
            }
        }
    }

    fn observed(slot: &str, pane: &str, verdict: Verdict, task: bool) -> AgentObservation {
        AgentObservation {
            slot: slot.to_owned(),
            pane: pane.to_owned(),
            verdict: seat(verdict, task),
            identity: SeatIdentity {
                model: Some("Captured Model".to_owned()),
                effort: Some("high".to_owned()),
                drift: true,
            },
        }
    }

    #[test]
    fn bgmark_all_nonlegacy_rungs_carry_task_legacy_preserves_old_bytes() {
        let roster = [entry("main", "p", "lead"), entry("worker.0", "p", "other")];
        let seen = [
            observed("main", "%3", Verdict::Quiet(QuietKind::WaitingAgent), true),
            observed("worker.0", "%4", Verdict::Quiet(QuietKind::Done), false),
        ];
        let clients = [
            ("main", "claude".to_owned()),
            ("worker.0", "codex".to_owned()),
        ];
        for rung in [FactRung::Full, FactRung::NoEffort, FactRung::ClientOnly] {
            let raw = fact_at(&roster, &seen, &clients, 2_000, 60, rung).expect("rung fits");
            assert!(raw.starts_with("v3;2000;60;"), "{raw}");
            for (row, task) in raw.split(';').skip(3).zip(["1", ""]) {
                let fields: Vec<_> = row.split(':').collect();
                assert_eq!(fields.len(), 9, "{row}");
                assert_eq!(fields[8], task);
            }
            let parsed =
                crate::tmux::parse_picker_agents(&raw, 2_000).expect("writer/parser agree");
            assert!(parsed[0].task);
            assert_eq!(parsed[0].mark(), Mark::Background);
            assert!(!parsed[1].task);
        }
        let raw =
            fact_at(&roster, &seen, &clients, 2_000, 60, FactRung::Legacy).expect("legacy fits");
        assert_eq!(raw, "v1;2000;60;lead:p:waiting-agent:%3;other:p:done:%4");
        let parsed = crate::tmux::parse_picker_agents(&raw, 2_000).expect("legacy parses");
        assert!(!parsed[0].task);
        assert_eq!(parsed[0].mark(), Mark::WaitingAgent);
    }

    #[test]
    fn bgmark_task_bytes_tip_full_to_noeffort_without_losing_task() {
        // Build the boundary from the specified old wire grammar, independently
        // of fact_at. All rows have the task, so v3 costs exactly two more bytes.
        let (count, profile) = (1..=64)
            .find_map(|count| {
                (1..=64).find_map(|width| {
                    let profile = "p".repeat(width);
                    let mut old = "v2;2000;60".to_owned();
                    for n in 0..count {
                        let _ = write!(
                            old,
                            ";seat{n:02}:{profile}:waiting-agent:%3:claude:Captured Model:high:!"
                        );
                    }
                    (old.len() <= 4096 && old.len() + count * 2 > 4096).then_some((count, profile))
                })
            })
            .expect("a bounded old-fits/new-overflows roster exists");
        let roster: Vec<_> = (0..count)
            .map(|n| entry(&format!("spawned.{n}"), &profile, &format!("seat{n:02}")))
            .collect();
        let seen: Vec<_> = roster
            .iter()
            .map(|e| observed(&e.slot, "%3", Verdict::Quiet(QuietKind::WaitingAgent), true))
            .collect();
        let clients: Vec<_> = roster
            .iter()
            .map(|e| (e.slot.as_str(), "claude".to_owned()))
            .collect();
        assert!(fact_at(&roster, &seen, &clients, 2_000, 60, FactRung::Full).is_none());
        let expected = fact_at(&roster, &seen, &clients, 2_000, 60, FactRung::NoEffort)
            .expect("NoEffort fits");
        assert_eq!(
            agents_fact(&roster, &seen, &clients, 2_000, 60),
            Some(expected.clone())
        );
        let parsed =
            crate::tmux::parse_picker_agents(&expected, 2_000).expect("degraded fact parses");
        assert_eq!(parsed.len(), count);
        assert!(
            parsed
                .iter()
                .all(|a| a.task && a.mark() == Mark::Background && a.effort.is_empty())
        );
    }

    #[test]
    fn bgmark_measured_stat_rows_reach_border_window_and_fact() {
        for (rows, task) in [
            (
                "90 1 Ss -fish\n100 90 S+ claude\n110 100 Ss /bin/zsh\n111 110 S sleep\n",
                Some(true),
            ),
            (
                "90 1 Ss -fish\n100 90 S+ claude\n110 100 S+ /bin/sh\n111 110 S docker\n",
                Some(false),
            ),
            (
                "90 1 Ss -fish\n100 90 S+ claude\n111 1 S sleep\n",
                Some(false),
            ),
            (
                "90 1 Ss -fish\n100 90 S+ claude\n110 100 S+ caffeinate\n",
                Some(false),
            ),
            (
                "90 1 Ss -fish\n100 90 S+ claude\n110 100 ? /bin/zsh\n",
                None,
            ),
        ] {
            let table = crate::procs::parse_stat_table(rows);
            let reading = crate::procs::task_running(table.as_deref(), 90, "claude");
            assert_eq!(reading, task, "{rows}");
            for (verdict, expected, word) in [
                (
                    Verdict::Quiet(QuietKind::WaitingAgent),
                    if task == Some(true) {
                        Mark::Background
                    } else {
                        Mark::WaitingAgent
                    },
                    "waiting-agent",
                ),
                (Verdict::Active, Mark::Working, "working"),
                (Verdict::Stale, Mark::Stale, "stale"),
                (
                    Verdict::Quiet(QuietKind::Blocked),
                    Mark::NeedsYou,
                    "blocked",
                ),
            ] {
                let composed = seat(verdict, reading == Some(true));
                let pane = PaneMark {
                    pane: "%3".to_owned(),
                    verdict: composed,
                    observed: String::new(),
                };
                let writes = super::super::Cycle::pane_state_writes("%3", Some(&pane), &look());
                let args = crate::tmux::set_options_args(&ServerId::Ambient, &writes);
                let state =
                    crate::theme::pane_state(&look().palette, expected, expected.glyph(true), word);
                assert!(args.contains(&state), "{args:?}");
                let window =
                    window_agents_line(&[("lead".to_owned(), composed.mark())], &look(), None);
                assert!(
                    window.contains(&format!("{}#[default]lead", expected.glyph(true))),
                    "{window}"
                );
                let roster = [entry("main", "p", "lead")];
                let seen = [observed("main", "%3", verdict, reading == Some(true))];
                let raw = agents_fact(&roster, &seen, &[("main", "claude".to_owned())], 2000, 60)
                    .expect("fact");
                let parsed = crate::tmux::parse_picker_agents(&raw, 2000).expect("fact parses");
                assert_eq!(parsed[0].mark(), expected);
                assert_eq!(parsed[0].task, reading == Some(true));
                assert_eq!(parsed[0].state, word);
            }
        }
    }

    #[test]
    fn bgmark_real_codex_background_capture_replays_as_a_task() {
        // Unmodified selected rows from ps -A -o pid=,ppid=,stat=,comm=,
        // captured 2026-11-12 while this author's background tool ran sleep 120.
        // Full raw capture remains local; no hand-built row replaces this input.
        let table = crate::procs::parse_stat_table(include_str!("ps-task-captured.txt"));
        assert_eq!(
            crate::procs::task_running(table.as_deref(), 66864, "codex"),
            Some(true)
        );
    }

    #[test]
    fn bgmark_session_and_slot_rollups_compose_and_attention_still_wins() {
        let pane = |task| PaneMark {
            pane: "%3".to_owned(),
            verdict: seat(Verdict::Quiet(QuietKind::WaitingAgent), task),
            observed: String::new(),
        };
        assert_eq!(
            session_mark(&[pane(true), pane(false)], &[]),
            Mark::Background
        );
        assert_eq!(
            session_mark(&[pane(true)], &[Mark::NeedsYou]),
            Mark::NeedsYou
        );
        let roster = entry("main", "p", "lead");
        assert_eq!(
            slot_mark(
                &roster,
                &[(
                    "main".to_owned(),
                    seat(Verdict::Quiet(QuietKind::WaitingAgent), true)
                )],
                &[]
            ),
            Mark::Background
        );
        let mark = session_mark(&[pane(true), pane(false)], &[]);
        assert_eq!(mark.glyph(true), "◉");
        assert_eq!(mark.rank(), 2);
    }

    #[test]
    fn bgmark_private_cycle_publishes_background_attention_border_window_and_fact() {
        let rig = crate::transport::quietwd_spec::Rig::new("bgmark-publish");
        let meta = rig.path("meta");
        let helper = SendHelper::for_session(&meta);
        let cycle = Cycle {
            knobs: Knobs::default(),
            meta_dir: &meta,
            helper: &helper,
            server: &rig.server,
            session: "current",
            goal: None,
            roster: Vec::new(),
            local_config: None,
            lead_pair: false,
            fleet_order: crate::theme::FleetOrder::EMPTY,
            auto: crate::autoreseat::settings(None),
            auto_trust: crate::config::AutoTrust::Off,
            meta_agent: false,
            launch_ids: Vec::new(),
            started: None,
            settled: None,
        };
        let table = crate::procs::parse_stat_table(
            "90 1 Ss -fish\n100 90 S+ claude\n110 100 Ss /bin/zsh\n111 110 S sleep\n",
        );
        let task = crate::procs::task_running(table.as_deref(), 90, "claude") == Some(true);
        assert!(task);
        let pane = PaneMark {
            pane: rig.pane.clone(),
            verdict: seat(Verdict::Quiet(QuietKind::WaitingAgent), task),
            observed: String::new(),
        };
        let roster = [entry("main", "p", "lead")];
        let observed = [observed(
            "main",
            &rig.pane,
            Verdict::Quiet(QuietKind::WaitingAgent),
            task,
        )];
        let fact = agents_fact(
            &roster,
            &observed,
            &[("main", "claude".to_owned())],
            2000,
            60,
        )
        .expect("fact");
        let mut carry = Carry::new();
        for extra in [None, Some(Mark::NeedsYou)] {
            carry.pending = super::super::PendingCycle::new(
                crate::transport::observe_session_identity(&rig.server, "current"),
            );
            let attention = session_mark(
                std::slice::from_ref(&pane),
                &extra.into_iter().collect::<Vec<_>>(),
            );
            cycle.publish(
                &super::super::Published {
                    bar: "",
                    active: 1,
                    total: 1,
                    by_pane: std::slice::from_ref(&pane),
                    agents: Some(&fact),
                    attention,
                    look: &look(),
                },
                &[None, None],
                &mut carry,
                None,
            );
            super::super::flush_cycle(&rig.server, &mut carry);
            assert_eq!(
                rig.option("", "=current:", crate::theme::ATTENTION_GLYPH_OPTION),
                if extra.is_some() { "⚠" } else { "◉" }
            );
            assert_eq!(
                rig.option("", "=current:", crate::theme::ATTENTION_RANK_OPTION),
                if extra.is_some() { "4" } else { "2" }
            );
            let state = rig.option("-p", &rig.pane, crate::theme::PANE_STATE_OPTION);
            assert!(state.contains("◉#[default] waiting"), "{state}");
            let window = rig.option("-w", &rig.window, crate::theme::WINDOW_AGENTS_OPTION);
            assert!(window.contains("◉#[default]lead"), "{window}");
            assert_eq!(
                rig.option("", "=current:", crate::theme::AGENTS_OPTION),
                fact
            );
        }
    }

    #[test]
    fn bgmark_shared_window_ticker_preserves_background_and_pulses_working() {
        for quiet in [
            Verdict::Quiet(QuietKind::WaitingAgent),
            Verdict::Quiet(QuietKind::Done),
            Verdict::Idle,
        ] {
            let bg = MotionVerdict {
                pane: "%1".to_owned(),
                window: "@7".to_owned(),
                verdict: seat(quiet, true),
            };
            let active = MotionVerdict {
                pane: "%2".to_owned(),
                window: "@7".to_owned(),
                verdict: seat(Verdict::Active, true),
            };
            let mut state = MotionState {
                verdicts: vec![bg.clone()],
                panes: vec![motion("%1", "lead")],
                ..MotionState::default()
            };
            assert!(MotionState::working(&state.verdicts, &state.panes).is_empty());
            assert!(state.step(&look()).is_empty());
            let mut control = MotionState {
                verdicts: vec![MotionVerdict {
                    pane: "%1".to_owned(),
                    window: "@7".to_owned(),
                    verdict: seat(quiet, false),
                }],
                panes: vec![motion("%1", "lead")],
                ..MotionState::default()
            };
            assert!(
                control.step(&look()).is_empty(),
                "neutral fleet path is also empty"
            );
            state.verdicts.push(active);
            state.panes.push(motion("%2", "other"));
            let writes = state.step(&look());
            let args = crate::tmux::set_options_args(&ServerId::Ambient, &writes);
            let frame = crate::theme::working_frame(state.spin, &look().palette, true);
            let window = window_agents_line(
                &[
                    ("lead".to_owned(), Mark::Background),
                    ("other".to_owned(), Mark::Working),
                ],
                &look(),
                Some(&frame),
            );
            assert!(args.contains(&window), "{args:?}");
            assert!(args.contains(&crate::theme::pane_state_frame(&frame, "working")));
            assert!(
                !args.iter().any(|a| a == "%1"),
                "background pane receives no pulse write: {args:?}"
            );
        }
    }
}
