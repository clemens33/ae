// Include byte-identically at the end of tests/it/reseat_carry.rs.
// The existing external fake records raw stdin. All delivery and move code is real.
#[allow(
    clippy::expect_used,
    reason = "owned fixtures must fail loudly on setup or observation errors"
)]
mod seatinc_move_acceptance {
    use super::*;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    #[allow(
        clippy::too_many_lines,
        reason = "one serialized move, deterministic race and its complete audit oracle"
    )]
    fn after_publish(replaced: bool) {
        let rig = Rig::new(if replaced {
            "seatincmoved"
        } else {
            "seatincsame"
        });
        let pane = automatic_carry(&rig, "fake-claude-b");
        let gate = rig.scratch.join("seatinc-read-gate");
        std::fs::create_dir(&gate).expect("owned barrier");
        std::fs::write(gate.join("@auto-reseat-after"), "").expect("arm post-move barrier");
        let mut child = super::super::cli::ae()
            .env("TMUX", format!("{},0,0", rig.sock.display()))
            .env("TMUX_PANE", &rig.main_pane)
            .env("AE_HOME", &rig.scratch)
            .env("HOME", rig.scratch.join("home"))
            .env("AE_SEND_DEFER_SEC", "0")
            .env("AE_TEST_APP_READ_GATE", &gate)
            .env_remove("AE_SENDER_OVERRIDE")
            .arg("_auto-reseat")
            .arg(&rig.dir)
            .args(["spawned.0", AUTO_KEY])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("real automatic move");
        let until = Instant::now() + Duration::from_secs(30);
        while !std::fs::read_to_string(gate.join("trace"))
            .unwrap_or_default()
            .lines()
            .any(|line| line == "held @auto-reseat-after")
        {
            assert!(
                child.try_wait().expect("leg status").is_none(),
                "post-move barrier not reached: {}",
                rig.events()
            );
            assert!(
                Instant::now() < until,
                "post-move barrier timed out: {}",
                rig.events()
            );
            std::thread::sleep(Duration::from_millis(25));
        }
        assert_eq!(rig.meta_row("profile.spawned.0"), "fake-claude-b");
        assert_eq!(
            rig.meta_row("harness_session.spawned.0"),
            ID,
            "carried conversation"
        );
        assert!(
            rig.tool_pid(&pane, "claude").is_some(),
            "moved tool already runs"
        );
        let minted = rig.meta_row("launch_id.spawned.0");
        assert!(!minted.is_empty(), "move must mint an incarnation");
        let before = std::fs::read(rig.scratch.join("received")).unwrap_or_default();
        assert!(
            before.is_empty(),
            "no continuation before the leg read: {before:?}"
        );
        if replaced {
            // The physical pane stays fixed. Only the incarnation changes
            // after the reseat returned and before the leg's post-move read.
            let current = rig.meta();
            let expected = format!("launch_id.spawned.0={minted}");
            assert_eq!(current.lines().filter(|line| *line == expected).count(), 1);
            std::fs::write(
                rig.dir.join("meta"),
                current.replace(&expected, "launch_id.spawned.0=successor"),
            )
            .expect("successor incarnation");
        }
        std::fs::remove_file(gate.join("@auto-reseat-after")).expect("release post-move read");
        let output = child.wait_with_output().expect("automatic move ended");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let events: Vec<_> = rig
            .events()
            .lines()
            .map(|line| ae::events::Event::parse_line(line).expect("audit event"))
            .collect();
        let refused: Vec<_> = events
            .iter()
            .filter(|event| event.action == "refused")
            .collect();
        let turns: Vec<_> = events
            .iter()
            .filter(|event| {
                event.action == ae::autoreseat::NOTICE_ACTION
                    && event.target.as_deref() == Some("scout")
            })
            .collect();
        if replaced {
            assert_eq!(
                std::fs::read(rig.scratch.join("received")).unwrap_or_default(),
                before,
                "successor gets no byte"
            );
            assert_eq!(refused.len(), 1, "one attributed refusal: {events:?}");
            assert_eq!(refused[0].actor, "watchdog");
            assert!(
                refused[0].target.is_none(),
                "2b refusal has no target member"
            );
            let expected = format!(
                "ae: delivery REFUSED — seat scout (spawned.0, launch {minted}) was replaced; NOTHING further was typed."
            );
            assert_eq!(refused[0].summary.as_deref(), Some(expected.as_str()));
            assert!(
                turns.is_empty(),
                "refused continuation records no delivery: {turns:?}"
            );
        } else {
            assert!(refused.is_empty(), "unchanged move continues: {refused:?}");
            assert_eq!(turns.len(), 1, "one continuation: {events:?}");
            assert_eq!(
                rig.received().matches("auto reseat moved you").count(),
                1,
                "unchanged incarnation receives its continuation: {}",
                rig.received()
            );
        }
    }

    #[test]
    fn seatinc_successor_after_publish_gets_no_continuation() {
        after_publish(true);
    }

    #[test]
    fn seatinc_unchanged_minted_incarnation_gets_its_continuation() {
        after_publish(false);
    }
}
