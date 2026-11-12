// Saved P1 direct-admission additions. Include after the frozen direct modules.
// Static proposal only: no installation, runtime calibration or freeze.
mod admission218_p1 {
    use super::*;

    const EXPECT_KEYS: [&str; 3] = ["_AE_EXPECT_SLOT", "_AE_EXPECT_LAUNCH", "_AE_EXPECT_LEGACY"];
    const RETRYABLE: &str = "ae-send: retryable-before-submit";

    fn expected(w: &World) -> Runner {
        let mut command = w.command("send", true, false);
        for key in EXPECT_KEYS {
            command.env_remove(key);
        }
        command
            .env("_AE_EXPECT_SLOT", SLOT)
            .env("_AE_EXPECT_LAUNCH", "tok-kid")
            .env("AE_SENDER_OVERRIDE", "spec218");
        command
    }

    fn only_unaddressed_refusal(w: &World) {
        assert_eq!(w.count("refused"), 1, "one refusal: {}", w.events());
        let events = w.events();
        let refused = events
            .lines()
            .find(|line| line.contains("\"action\":\"refused\""))
            .expect("refusal row");
        for field in ["\"target", "\"ref\":", "\"body_file\":", "\"delivery\":"] {
            assert!(
                !refused.contains(field),
                "addressed/delivery meaning: {refused}"
            );
        }
        assert!(refused.contains("\"actor\":\"spec218\""), "{refused}");
    }

    struct ViewerRelease(PathBuf);

    impl Drop for ViewerRelease {
        fn drop(&mut self) {
            if let Err(error) = fs::write(&self.0, "go") {
                let _ = std::io::Write::write_fmt(
                    &mut std::io::stderr(),
                    format_args!("INCOMPLETE: release viewer hold: {error}\n"),
                );
            }
        }
    }

    fn damaged_after_resolution(tag: &str, mode: &str) {
        let w = World::new(tag);
        let before = w.screen();
        let mut command = expected(&w);
        w.barrier(&mut command);
        w.resume();
        let _release = ViewerRelease(w.rig.scratch.join("barrier/release"));
        command
            .env("SEATPROOF_HOLD_VIEWER", "2")
            .env("SEATPROOF_PANE", &w.pane);
        let child = command.spawn().expect("SETUP: real expected helper");
        wait("SETUP: second pane-stamp read held", || {
            w.rig.scratch.join("barrier/held").is_file()
        });
        match mode {
            "removed" => w.change("remove", SLOT),
            "session-gone" => fs::remove_file(w.meta()).expect("SETUP: remove session meta"),
            "stamp-incomplete" => w.stamp("@ae_agent", None),
            _ => panic!("SETUP: unknown damage"),
        }
        w.release();
        let output = finish(child);
        let word = if mode == "stamp-incomplete" {
            "could not be proven"
        } else {
            "retired"
        };
        w.refuse(&output, word);
        w.untouched(&before);
        only_unaddressed_refusal(&w);
        assert!(output.stdout.is_empty(), "unexpected stdout: {output:?}");
    }

    #[test]
    fn r2_matching_expectation_keeps_removed_seat_refusal() {
        damaged_after_resolution("r218-p1-removed", "removed");
    }

    #[test]
    fn r2_matching_expectation_keeps_session_record_gone_refusal() {
        damaged_after_resolution("r218-p1-session-gone", "session-gone");
    }

    #[test]
    fn r2_matching_expectation_keeps_incomplete_stamp_refusal() {
        damaged_after_resolution("r218-p1-stamp", "stamp-incomplete");
    }

    #[test]
    fn r6_absent_members_preserve_exact_main_refusal_and_stable_journal() {
        let w = World::new("r218-p1-absent");
        w.change("remove", SLOT);
        let before = w.screen();
        let mut command = w.command("send", true, false);
        for key in EXPECT_KEYS {
            // Explicit removal establishes absence even in a contaminated parent.
            command.env_remove(key);
            assert!(
                command
                    .get_envs()
                    .any(|(name, value)| name == key && value.is_none()),
                "SETUP: expected member was not explicitly removed"
            );
        }
        command.env("AE_SENDER_OVERRIDE", "spec218");
        let output = command.output().expect("SETUP: real absent-member helper");
        let reason = "ae: delivery REFUSED — seat kid (spawned.0) is retired: its roster row is gone; NOTHING further was typed.";
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert_eq!(output.stderr, format!("{reason}\n").as_bytes());
        w.untouched(&before);
        only_unaddressed_refusal(&w);
        let events = w.events();
        let refused = events
            .lines()
            .find(|line| line.contains("\"action\":\"refused\""))
            .expect("refusal row");
        // Only ts is nondeterministic for this unaddressed row. Keep every
        // remaining byte, including field order, actor and schema version.
        let (timestamp, stable) = refused
            .split_once("\",\"actor\":")
            .expect("SETUP: main timestamp/actor record shape");
        assert!(timestamp.starts_with("{\"ts\":\""), "{refused}");
        assert_eq!(
            format!("{{\"actor\":{stable}"),
            format!(
                "{{\"actor\":\"spec218\",\"action\":\"refused\",\"summary\":\"{reason}\",\"schema_version\":1}}"
            )
        );
    }

    fn quota(tag: &str, action: &str, mismatch: bool) {
        let w = World::new(tag);
        let before = w.screen();
        if mismatch {
            w.change("replace", SLOT);
        }
        let output = expected(&w)
            .env("_AE_EVENT_ACTION", action)
            .env("_AE_EVENT_SUMMARY", "P1 quota contract")
            .env("_AE_EVENT_REF", "p1-contract-reference")
            .env("_AE_EVENT_TARGET_SLOT", SLOT)
            .env("_AE_EVENT_TARGET_SESSION", "s")
            .output()
            .expect("SETUP: real quota-shaped helper");
        if mismatch {
            // B0 must fail on forbidden input, never an absent future API.
            w.untouched(&before);
            w.refuse(&output, "replaced");
            only_unaddressed_refusal(&w);
            assert_eq!(w.count(action), 0, "stale addressed action: {}", w.events());
        } else {
            assert_eq!(output.status.code(), Some(0), "{output:?}");
            wait("SETUP: matching quota body", || read(&w.out).contains(BODY));
            assert_eq!(w.count(action), 1, "{}", w.events());
            assert_eq!(w.count("refused"), 0);
        }
        assert!(
            !String::from_utf8_lossy(&output.stdout).contains(RETRYABLE)
                && !String::from_utf8_lossy(&output.stderr).contains(RETRYABLE),
            "seat refusal or delivered quota gained retry permission: {output:?}"
        );
    }

    #[test]
    fn r8_stale_advisory_refuses_without_retry_or_addressed_record() {
        quota("r218-p1-quota-stale-a", "quota-advisory", true);
    }

    #[test]
    fn r8_stale_checkpoint_refuses_without_retry_or_addressed_record() {
        quota("r218-p1-quota-stale-c", "quota-checkpoint", true);
    }

    #[test]
    fn r8_matching_advisory_delivers_without_retry_marker() {
        quota("r218-p1-quota-match-a", "quota-advisory", false);
    }

    #[test]
    fn r8_matching_checkpoint_delivers_without_retry_marker() {
        quota("r218-p1-quota-match-c", "quota-checkpoint", false);
    }

    #[test]
    fn r8_matching_expectation_preserves_human_draft_abandoned_and_resend() {
        draft_abandoned("r218-p1-draft", None);
    }

    #[test]
    fn r8_matching_expectation_preserves_human_draft_abandoned_and_resend_the_tool_draws_late() {
        draft_abandoned("r218-p1-draft-late", Some(400));
    }

    fn draft_abandoned(tag: &str, slow_draw_ms: Option<u32>) {
        let w = World::new(tag);
        if let Some(ms) = slow_draw_ms {
            fs::write(w.out.with_extension("slowdraw"), ms.to_string())
                .expect("SETUP: slow draw knob");
        }
        w.rig.tmux(&["set-buffer", "-b", "p1-draft", "human draft"]);
        w.rig
            .tmux(&["paste-buffer", "-p", "-d", "-b", "p1-draft", "-t", &w.pane]);
        let wire = w.out.with_extension("wire");
        wait("SETUP: acknowledged human draft", || {
            read(&wire).ends_with("\x1b[201~")
        });
        fs::write(&wire, "").expect("SETUP: reset draft input receipt");
        let before = w.screen();
        let output = expected(&w)
            .env("AE_SEND_DEFER_SEC", "0")
            .output()
            .expect("SETUP: real matching helper with draft");
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("ABANDONED") && error.contains("Re-send."),
            "{error}"
        );
        assert!(
            !error.contains("UNCONFIRMED") && !error.contains("REFUSED"),
            "{error}"
        );
        w.untouched(&before);
        assert_eq!(w.count("delivery-abandoned"), 1, "{}", w.events());
        assert_eq!(w.count("refused"), 0);
        assert!(
            w.events().contains("refused: "),
            "missing held-half diagnostic: {}",
            w.events()
        );
        assert!(
            fs::read_dir(w.dir().join("messages"))
                .expect("preserved body directory")
                .any(|entry| read(entry.expect("stored body").path()).contains(BODY)),
            "recoverable original body missing"
        );
    }
}
