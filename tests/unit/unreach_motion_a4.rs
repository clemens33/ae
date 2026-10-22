// Frozen unreach addendum a4. Include inside watchdog_daemon.rs's tests module.
mod unreach_motion_a4 {
    use super::*;

    #[test]
    fn unreach_busy_draft_and_repaint_do_not_end_or_hide_a_failure() {
        for case in ["busy", "draft", "repaint", "escalated-wait"] {
            let mut observed = seen();
            observed.unreachable = true;
            observed.outstanding_alert = Some(crate::attention::Reason::Stale);
            observed.harness.durable_stale = true;
            match case {
                "busy" => observed.harness.frame = crate::harness_state::HarnessState::Busy,
                "draft" => observed.harness.human_draft = true,
                "escalated-wait" => {
                    observed.quiet = Some(QuietKind::WaitingAgent);
                    observed.declared_age_secs = 10_000;
                }
                _ => observed.harness.durable_stale = false,
            }
            let accounted = account(&PaneState::default(), &observed, &Knobs::default());
            assert_eq!(accounted.verdict.reason(), "unreachable", "{case}");
            assert_eq!(accounted.verdict.mark(), crate::theme::Mark::Stale);
            assert!(
                !accounted.effects.iter().any(|effect| matches!(
                    effect,
                    Effect::Emit {
                        action: "alert-cleared",
                        ..
                    }
                )),
                "{case}: pixels alone are not a landed delivery, own record or new start"
            );
        }
    }
}
