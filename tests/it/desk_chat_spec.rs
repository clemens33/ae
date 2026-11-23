//! Run /board through the real console input and read what the pane displays.

use std::fs;

use super::Rig;

#[test]
fn board_command_reads_the_bound_record_at_each_use_without_writing_the_journal() {
    let rig = Rig::new("deskspec", 130, 34);
    let meta = rig.tool.dir.join("meta");
    let overlay = rig.root.join("orchestrator.config");
    fs::write(
        &overlay,
        include_str!("../../contrib/aeorchestrator/orchestrator.config"),
    )
    .expect("real orchestrator overlay");
    let before_meta = fs::read_to_string(&meta).expect("console session meta");
    fs::write(
        &meta,
        format!(
            "{before_meta}meta_agent=true\nlocal_config={}\n",
            overlay.display()
        ),
    )
    .expect("orchestrator role in the bound session");
    let path = rig.tool.dir.join("desk-record");
    let journal = rig.tool.events();
    let command = |line| {
        rig.paste(line);
        rig.key("Enter");
    };

    command("/board");
    let screen = rig.wait("the live sample follows the absent record", |screen| {
        screen
            .split_once("no board set")
            .is_some_and(|(_, rest)| rest.contains("LIVE  as of"))
    });
    let sample = screen
        .lines()
        .find(|line| line.contains("LIVE  as of"))
        .expect("first live sample")
        .to_owned();
    // Make the second read observably later even on a fast fixture.
    std::thread::sleep(std::time::Duration::from_millis(1100));

    let first = concat!(
        "ae-desk 1\nupdated 2026-11-20T08:40:12Z\n",
        "D7\tChoose the west region\thttps://example.test/west?q=1&n=2\t/tmp/board/west.md\n",
    );
    fs::write(&path, first).expect("first record");
    command("/board");
    let screen = rig.wait("first board appears", |screen| {
        screen
            .split_once("/tmp/board/west.md")
            .is_some_and(|(board, rest)| {
                board.contains("Choose the west region") && rest.contains("LIVE  as of")
            })
    });
    assert_eq!(
        screen.lines().rfind(|line| line.contains("LIVE  as of")),
        Some(sample.as_str()),
        "a second /board in the same minute reuses the first sample"
    );
    assert!(screen.contains("08:40"), "{screen}");
    assert!(
        screen.contains("D7") && screen.contains("DECIDE"),
        "{screen}"
    );
    assert!(
        screen.contains("https://example.test/west?q=1&n=2"),
        "{screen}"
    );

    let second = "ae-desk 1\nupdated 2026-11-20T08:41:12Z\nC3\tSchedule the east release\n";
    fs::write(&path, second).expect("replace the whole record");
    command("/board more");
    let screen = rig.wait("extra operand is refused", |screen| {
        screen.contains("/board takes no argument")
    });
    assert!(!screen.contains("Schedule the east release"), "{screen}");
    command("/board");
    let screen = rig.wait("changed board appears", |screen| {
        screen.contains("Schedule the east release")
    });
    assert!(
        screen.contains("08:41") && screen.contains("C3"),
        "{screen}"
    );
    assert_eq!(fs::read_to_string(&path).expect("read-only board"), second);

    let damaged = "ae-desk 1\nupdated yesterday\nL1\tthis malformed item must not show\n";
    fs::write(&path, damaged).expect("damaged input fixture");
    command("/board");
    let screen = rig.wait("damage is explicit", |screen| {
        screen.contains("board unreadable")
    });
    assert!(
        !screen.contains("this malformed item must not show"),
        "{screen}"
    );
    assert_eq!(
        fs::read_to_string(&path).expect("damaged record kept"),
        damaged
    );
    assert_eq!(
        rig.tool.events(),
        journal,
        "board reads never become asks or events"
    );
}

mod desklive_snapshot {
    use std::fmt::Write as _;
    use std::fs;

    use ae::time::Timestamp;

    use super::Rig;

    const DESK: &str = "ae-desk 1\nupdated 2026-11-20T08:40:12Z\nD7\tManual desk decision\n";
    const PEER_UUID: &str = "0199c0de-aabb-4890-abcd-ef0123456789";

    fn row(at: i64, actor: &str, action: &str, fields: &str) -> String {
        let at = Timestamp::from_epoch(at);
        format!(r#"{{"ts":"{at}","actor":"{actor}","action":"{action}"{fields}}}"#)
    }

    fn mark_owned(rig: &Rig, name: &str) {
        rig.tmux(&["set-environment", "-t", name, "AE_SESSION", name]);
        rig.tmux(&[
            "set-environment",
            "-t",
            name,
            "AE_HOME",
            rig.root.to_str().expect("fixture root is UTF-8"),
        ]);
    }

    fn peer(rig: &Rig, name: &str, now: i64, running: bool) -> std::path::PathBuf {
        let dir = rig.root.join("sessions").join(name);
        fs::create_dir_all(&dir).expect("peer record");
        let uuid = if running {
            PEER_UUID
        } else {
            "0199c0de-aacc-4890-abcd-ef0123456789"
        };
        let mut meta = format!(
            "schema=2\nsession={name}\nmode=local\nsession_id={uuid}\nlayout=vertical\n\
             tmux_server_kind=socket\ntmux_server={}\n",
            rig.socket.display(),
        );
        let roster = [
            ("main", "birchlead"),
            ("worker.0", "cedar"),
            ("worker.1", "fern"),
            ("worker.2", "oak"),
            ("worker.3", "elm"),
        ];
        for (slot, agent) in roster {
            writeln!(meta, "seat.{slot}={agent}\nagent_bin.{slot}=codex").expect("format roster");
        }
        fs::write(dir.join("meta"), meta).expect("peer meta");
        fs::write(dir.join(".launch-attempt"), (now - 900).to_string())
            .expect("peer launch attempt");
        if running {
            let pane = rig.tmux(&[
                "new-session",
                "-d",
                "-P",
                "-F",
                "#{pane_id}",
                "-s",
                name,
                "sleep",
                "120",
            ]);
            mark_owned(rig, name);
            rig.tmux(&["set-option", "-t", name, "@ae_session_uuid", uuid]);
            rig.tmux(&["set-option", "-p", "-t", pane.trim(), "@ae_slot", "main"]);
            for (slot, _) in roster.into_iter().skip(1) {
                let pane = rig.tmux(&[
                    "new-window",
                    "-d",
                    "-P",
                    "-F",
                    "#{pane_id}",
                    "-t",
                    name,
                    "sleep",
                    "120",
                ]);
                rig.tmux(&["set-option", "-p", "-t", pane.trim(), "@ae_slot", slot]);
            }
        }
        dir
    }

    fn screen_with_board(rig: &Rig) -> String {
        rig.paste("/board");
        rig.key("Enter");
        rig.wait("the prior manual record still renders", |screen| {
            screen.contains("Manual desk decision")
                && screen.lines().any(|line| line.trim_end() == "to lead>")
        })
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one journal supplies the opposed states and request directions"
    )]
    fn the_chat_board_shows_the_fleet_facts_and_preserves_its_record() {
        let rig = Rig::new("desklivechat", 155, 40);
        let dir = rig.tool.dir.join("desk-record");
        fs::write(&dir, DESK).expect("manual desk record");
        mark_owned(&rig, &rig.name);
        let now = Timestamp::now().epoch();
        let birch = peer(&rig, "birch-live", now, true);
        let stopped = peer(&rig, "stopped-decoy", now, false);
        fs::write(
            stopped.join("events.jsonl"),
            format!(
                "{}\n",
                row(
                    now - 80,
                    "birchlead",
                    "state",
                    r#","ref":"blocked","summary":"closed-fleet decoy""#,
                )
            ),
        )
        .expect("stopped journal");
        let open_human = "ae-20261120T084012Z-00000001";
        let closed_human = "ae-20261120T084012Z-00000002";
        let internal = "ae-20261120T084012Z-00000003";
        let owner_inbound = "ae-20261120T084012Z-00000004";
        let automation = "ae-20261120T084012Z-00000005";
        let events = [
            row(
                now - 850,
                "elm",
                "ask",
                &format!(r#","ref":"{open_human}","target":"telegram:42""#),
            ),
            row(
                now - 845,
                "elm",
                "ask",
                &format!(r#","ref":"{closed_human}","target":"telegram:42""#),
            ),
            row(
                now - 840,
                "elm",
                "cancel",
                &format!(r#","ref":"{closed_human}""#),
            ),
            row(
                now - 835,
                "oak",
                "ask",
                &format!(r#","ref":"{internal}","target":"elm""#),
            ),
            row(
                now - 830,
                "console:local",
                "ask",
                &format!(r#","ref":"{owner_inbound}","target":"birchlead""#),
            ),
            row(
                now - 825,
                "oak",
                "ask",
                &format!(r#","ref":"{automation}","target":"ae:compact:fixture""#),
            ),
            row(
                now - 670,
                "birchlead",
                "state",
                r#","ref":"waiting-user","summary":"amber decision""#,
            ),
            row(
                now - 490,
                "cedar",
                "state",
                r#","ref":"waiting-agent","summary":"cedar evidence""#,
            ),
            row(
                now - 430,
                "fern",
                "state",
                r#","ref":"blocked","summary":"fern dependency""#,
            ),
            row(
                now - 250,
                "oak",
                "state",
                r#","ref":"working","summary":"oak checks""#,
            ),
            row(
                now - 190,
                "elm",
                "state",
                r#","ref":"done","summary":"elm verified""#,
            ),
        ]
        .join("\n")
            + "\n";
        fs::write(birch.join("events.jsonl"), &events).expect("complete peer journal");
        let before = rig.tool.events();

        let screen = screen_with_board(&rig);
        let (_, live) = screen
            .split_once("LIVE")
            .expect("live block after the manual desk");
        assert!(
            live.lines().next().is_some_and(|row| row.contains("as of")),
            "{screen}"
        );
        assert!(
            live.contains(&rig.name),
            "the bound session is also in the fleet: {screen}"
        );
        assert!(
            live.lines().any(|row| row.contains("birch-live")
                && row.contains("running workers: 1")
                && row.contains("open asks to you: 1")),
            "{screen}"
        );
        for (seat, state, age, reason) in [
            ("birchlead", "waiting-user", "11m", "amber decision"),
            ("cedar", "waiting-agent", "8m", "cedar evidence"),
            ("fern", "blocked", "7m", "fern dependency"),
            ("oak", "working", "4m", "oak checks"),
            ("elm", "done", "3m", "elm verified"),
        ] {
            assert!(
                live.lines().any(|line| [seat, state, age, reason]
                    .iter()
                    .all(|term| line.contains(term))),
                "missing {seat} {state} {age} {reason}: {screen}"
            );
        }
        assert!(live.contains(open_human), "{screen}");
        for absent in [
            closed_human,
            internal,
            owner_inbound,
            automation,
            "stopped-decoy",
        ] {
            assert!(!live.contains(absent), "unexpected {absent}: {screen}");
        }
        assert_eq!(fs::read(&dir).expect("manual desk bytes"), DESK.as_bytes());
        assert_eq!(
            rig.tool.events(),
            before,
            "the board command appends no events"
        );
        assert_eq!(
            fs::read_to_string(birch.join("events.jsonl")).expect("peer journal"),
            events,
        );
    }

    #[test]
    fn a_running_session_with_an_unreadable_journal_is_shown_as_unavailable() {
        let rig = Rig::new("desklivegap", 150, 38);
        fs::write(rig.tool.dir.join("desk-record"), DESK).expect("manual desk record");
        let unread = peer(&rig, "missing-facts", Timestamp::now().epoch(), true);
        fs::create_dir(unread.join("events.jsonl")).expect("unreadable journal node");
        let screen = screen_with_board(&rig);
        let (_, live) = screen.split_once("LIVE").expect("live block");
        assert!(
            live.lines()
                .next()
                .is_some_and(|line| line.contains("partial")),
            "{screen}"
        );
        assert!(
            live.lines().any(|line| line.contains("missing-facts")
                && line.contains("unavailable")
                && !line.contains("open asks to you: 0")),
            "{screen}"
        );
    }
}
