// Frozen unreach r1. Insert verbatim INSIDE watchdog_daemon.rs's existing
// #[cfg(test)] mod tests, immediately before that module's final brace.
mod unreach_episode_spec {
    use super::*;

    fn bounded(state: &str) -> Observation {
        let mut observed = seen();
        observed.outstanding_alert = None;
        observed.unreachable = false;
        if state == "done" {
            observed.quiet = Some(QuietKind::Done);
            observed.done_progress = DoneProgress::ChallengeDue {
                confirmations: 0,
                required: 2,
                done_age_secs: 300,
                attempts: 3,
            };
        } else {
            observed.quiet = Some(QuietKind::Blocked);
            observed.wait_progress = wait_due(3);
        }
        observed
    }

    fn alerts(effects: &[Effect]) -> usize {
        effects
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    Effect::Emit {
                        action: "alert",
                        ..
                    }
                )
            })
            .count()
    }

    fn challenges(effects: &[Effect]) -> usize {
        effects
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    Effect::DoneChallenge { .. } | Effect::WaitChallenge { .. }
                )
            })
            .count()
    }

    #[test]
    fn unreach_done_and_wait_book_one_alert_then_keep_one_verdict_after_restart() {
        for state in ["done", "blocked"] {
            let knobs = Knobs::default();
            let mut observed = bounded(state);
            let first = account(&PaneState::default(), &observed, &knobs);
            assert_eq!(
                alerts(&first.effects),
                1,
                "first unreachable alert: {state}"
            );
            assert_eq!(
                challenges(&first.effects),
                0,
                "no attempt on an unreadable pane"
            );
            observed.unreachable = true;
            observed.outstanding_alert = Some(crate::attention::Reason::Stale);
            let mut prior = first.next;
            for cycle in 0..6 {
                if cycle == 3 {
                    prior = PaneState::default();
                }
                let held = account(&prior, &observed, &knobs);
                assert_eq!(
                    alerts(&held.effects),
                    0,
                    "{state}, cycle {cycle}: durable standing record prevents rebook"
                );
                assert_eq!(challenges(&held.effects), 0, "no idle composer, no retry");
                assert_eq!(
                    held.verdict.reason(),
                    "unreachable",
                    "unreachable outranks quiet {state}"
                );
                assert_eq!(
                    held.verdict.mark(),
                    crate::theme::Mark::Stale,
                    "frozen seven-mark contract"
                );
                prior = held.next;
            }
        }
    }

    #[test]
    fn unreach_recovery_challenge_requires_the_drawn_empty_idle_composer() {
        for state in ["done", "blocked"] {
            for (frame, draft, expected) in [
                (crate::harness_state::HarnessState::Unknown, false, 0),
                (crate::harness_state::HarnessState::Busy, false, 0),
                (crate::harness_state::HarnessState::Idle, true, 0),
                (crate::harness_state::HarnessState::Idle, false, 1),
            ] {
                let mut observed = bounded(state);
                observed.unreachable = true;
                observed.outstanding_alert = Some(crate::attention::Reason::Stale);
                observed.harness.frame = frame;
                observed.harness.human_draft = draft;
                let recovered = account(&PaneState::default(), &observed, &Knobs::default());
                assert_eq!(
                    challenges(&recovered.effects),
                    expected,
                    "{state}: {frame:?}, draft={draft}"
                );
                assert_eq!(
                    alerts(&recovered.effects),
                    0,
                    "recovery does not rebook standing alert"
                );
            }
        }
    }
}
