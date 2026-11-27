//! Control-mode acceptance. The trusted Facts block precedes any pane text.

use super::{
    ControlEvent, ControlParser, Expect, HookRefusal, PaneFacts, Proven, SidecarCommand,
    hooks_eligible,
};

fn attached() -> ControlParser {
    let mut parser = ControlParser::new();
    assert_eq!(
        parser.feed(b"%begin 20 1 0\n%end 20 1 0\n%session-changed $7 voice\n"),
        [ControlEvent::Attached]
    );
    parser
}

fn expecting(command: SidecarCommand) -> ControlParser {
    let mut parser = attached();
    for block in Expect::for_command(command) {
        parser.expect(block);
    }
    parser
}

#[test]
fn a_frame_keeps_all_screen_rows_even_when_they_spell_protocol_lines() {
    let mut parser = expecting(SidecarCommand::Frame);
    let stream = concat!(
        "%begin 21 2 1\n",
        "$7 %9 18 6 12 4\n",
        "%end 21 2 1\n",
        "%window-renamed @3 Claude\n",
        "%begin 21 4 1\n",
        "%end 21 4 1\n",
        "%begin 21 5 1\n",
        "%exit\n",
        "\n",
        "% ordinary saved text\n",
        "\u{1b}[38;5;246mὌ styled row\n",
        "%end 21 4 1\n",
    );
    let observed: Vec<_> = stream
        .as_bytes()
        .chunks(1)
        .flat_map(|byte| parser.feed(byte))
        .collect();
    assert_eq!(
        observed,
        [
            ControlEvent::Facts(PaneFacts {
                session_id: "$7".into(),
                pane_id: "%9".into(),
                pane_pid: 18,
                height: 6,
                cursor: (12, 4),
            }),
            ControlEvent::Capture(
                "%end 21 4 1\n%begin 21 5 1\n%exit\n\n% ordinary saved text\n\u{1b}[38;5;246mὌ styled row"
                    .into()
            )
        ]
    );
    assert!(parser.finish().is_empty());
}

#[test]
fn only_the_trusted_height_then_the_matching_end_can_close_the_capture() {
    for tail in [
        "%end 21 3 1\n",
        "%end 21 3 1\nfilled row\n%end 21 4 1\n",
        "row one\nrow two\nrow three\n%end 21 3 1\n",
        "row one\nrow two\n%end 22 3 1\n",
        "row one\nrow two\n%end 21 3 0\n",
    ] {
        let mut parser = expecting(SidecarCommand::Frame);
        assert!(matches!(
            parser
                .feed(b"%begin 21 2 1\n$7 %9 18 2 4 0\n%end 21 2 1\n%begin 21 3 1\n")
                .as_slice(),
            [ControlEvent::Facts(_)]
        ));
        let events = parser.feed(tail.as_bytes());
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, ControlEvent::Capture(_))),
            "an incomplete or malformed capture gave a frame: {tail:?}: {events:?}"
        );
        assert!(
            events.contains(&ControlEvent::Malformed)
                || parser.finish() == [ControlEvent::Malformed],
            "the parser accepted an unclosed capture: {tail:?}: {events:?}"
        );
        assert!(parser.feed(b"%begin 21 9 1\n%end 21 9 1\n").is_empty());
    }
}

#[test]
fn each_space_receipt_must_be_empty_and_match_its_entire_header() {
    assert_eq!(
        Expect::for_command(SidecarCommand::Space),
        [Expect::Empty; 2]
    );
    let mut parser = expecting(SidecarCommand::Space);
    assert_eq!(
        parser.feed(b"%begin 30 9 1\n%end 30 9 1\n"),
        [ControlEvent::Empty]
    );
    assert_eq!(
        parser.feed(b"%begin 30 10 1\nae-pane-changed\n%end 30 10 1\n"),
        [ControlEvent::Refused("ae-pane-changed".into())]
    );

    for end in [
        "%end 31 9 1",
        "%end 30 10 1",
        "%end 30 9 0",
        "%begin 30 9 1",
    ] {
        let mut parser = expecting(SidecarCommand::Space);
        let input = format!("%begin 30 9 1\n{end}\n");
        assert_eq!(parser.feed(input.as_bytes()), [ControlEvent::Malformed]);
        assert!(parser.feed(b"%end 30 9 1\n").is_empty());
    }
}

#[test]
fn a_server_error_describes_refusal_and_eof_cannot_complete_an_expected_reply() {
    let mut parser = expecting(SidecarCommand::Space);
    assert_eq!(
        parser.feed(b"%begin 30 9 1\ncan't find pane: %9\n%error 30 9 1\n"),
        [ControlEvent::Refused("can't find pane: %9".into())]
    );
    for incomplete in [
        "",
        "%begin 30 9 1\n",
        "%begin 30 9 1\n%end 30 9 1",
        "%begin 30 9 1\n%end 30 9 1\n",
    ] {
        let mut parser = expecting(SidecarCommand::Space);
        parser.feed(incomplete.as_bytes());
        assert_eq!(parser.finish(), [ControlEvent::Malformed], "{incomplete:?}");
    }
}

#[test]
fn empty_facts_refuse_and_exit_cannot_stand_in_for_an_expected_reply() {
    let mut parser = expecting(SidecarCommand::Frame);
    assert_eq!(
        parser.feed(b"%begin 21 2 1\n\n%end 21 2 1\n"),
        [ControlEvent::Refused(String::new())]
    );

    let mut parser = expecting(SidecarCommand::Space);
    assert_eq!(parser.feed(b"%exit\n"), [ControlEvent::Exit]);
}

#[test]
fn no_reply_block_can_appear_unsolicited_and_invalid_utf8_closes_the_stream() {
    for stray in [
        b"%begin 19 4 1\n".as_slice(),
        b"%end 19 4 1\n",
        b"%error 19 4 1\n",
        b"unframed output\n",
    ] {
        assert_eq!(attached().feed(stray), [ControlEvent::Malformed]);
    }
    let mut parser = expecting(SidecarCommand::Frame);
    let prefix = b"%begin 21 2 1\n$7 %9 18 1 0 0\n%end 21 2 1\n%begin 21 3 1\n";
    assert!(matches!(
        parser.feed(prefix).as_slice(),
        [ControlEvent::Facts(_)]
    ));
    assert_eq!(
        parser.feed(b"\xff\n%end 21 3 1\n"),
        [ControlEvent::Malformed]
    );
}

#[test]
fn unsolicited_hook_blocks_cannot_satisfy_a_command_expectation() {
    let mut parser = ControlParser::new();
    assert_eq!(
        parser.feed(b"%begin 20 1 1\n%end 20 1 1\n"),
        [ControlEvent::Malformed],
        "the attach accepts only the initial command flag"
    );

    let mut parser = expecting(SidecarCommand::Space);
    assert_eq!(
        parser.feed(b"%begin 30 9 1\n%end 30 9 1\n"),
        [ControlEvent::Empty]
    );
    assert_eq!(
        parser.feed(b"%begin 30 10 0\n%end 30 10 0\n"),
        [ControlEvent::Malformed]
    );
    assert!(parser.feed(b"%begin 30 11 1\n%end 30 11 1\n").is_empty());

    let mut parser = expecting(SidecarCommand::Frame);
    assert!(matches!(
        parser
            .feed(b"%begin 30 9 1\n$7 %9 18 1 0 0\n%end 30 9 1\n")
            .as_slice(),
        [ControlEvent::Facts(_)]
    ));
    assert_eq!(
        parser.feed(b"%begin 30 10 0\n%end 30 10 0\n"),
        [ControlEvent::Malformed]
    );
    assert!(
        parser
            .feed(b"%begin 30 11 1\nrow\n%end 30 11 1\n")
            .is_empty()
    );
}

#[test]
fn client_hooks_require_the_guard_to_cover_the_entire_session_command() {
    const NAME: &str = "client-session-changed";
    const GLOBAL: &str = "client-session-changed\n";
    const GUARD: &str =
        "#{?#{&&:#{client_control_mode},#{==:#{client_termname},ae-dictation}},0,1}";
    const LEGACY: [&str; 3] = [
        "client-session-changed[9] wait-for -S ae-wake-0-1764230000\n",
        "client-session-changed[0] if-shell -F -t \"%0\" \"#{==:#{session_id},$0}\" \"select-window -t %0 ; select-pane -t %0\"\n",
        concat!(
            "client-session-changed[0] if-shell -F -t \"%0\" \"#{==:#{session_id},$0}\" { ",
            "if-shell -F -t \"$0:^\" ",
            "\"#{P:#{?#{&&:#{==:#{pane_id},%1},#{&&:#{==:#{@ae_console},",
            "11111111-2222-4333-8444-555555555555},#{!=:#{pane_dead},1}}},1,}}\" ",
            "{ select-window -t \"%1\" ; select-pane -t \"%1\" } ",
            "{ select-window -t \"%0\" ; select-pane -t \"%0\" } }\n",
        ),
    ];
    assert_eq!(hooks_eligible(NAME, "", GLOBAL), Ok(()));
    for (index, released) in [9, 0, 0].into_iter().zip(LEGACY) {
        assert_eq!(
            hooks_eligible(NAME, released, GLOBAL),
            Err(HookRefusal::Unguarded { index })
        );
        let (_, body) = released.split_once("] ").expect("measured hook row");
        let filtered = format!(
            "{NAME}[{index}] if-shell -F \"{GUARD}\" {{ {} }}\n",
            body.trim_end()
        );
        assert_eq!(hooks_eligible(NAME, &filtered, GLOBAL), Ok(()));
    }
    for suffix in [" ; wait-for -S stray", " { wait-for -S stray }"] {
        let row = format!("{NAME}[9] if-shell -F \"{GUARD}\" {{ wait-for -S expected }}{suffix}\n");
        assert_eq!(
            hooks_eligible(NAME, &row, GLOBAL),
            Err(HookRefusal::Unguarded { index: 9 })
        );
    }
    let current = format!("{NAME}[9] if-shell -F \"{GUARD}\" {{ wait-for -S expected }}\n");
    assert_eq!(
        hooks_eligible(
            NAME,
            &current,
            "client-session-changed[3] display-message -p x\n"
        ),
        Err(HookRefusal::Global)
    );
    let detach = current.replace(NAME, "client-detached");
    assert_eq!(
        hooks_eligible("client-detached", &detach, "client-detached\n"),
        Err(HookRefusal::Unguarded { index: 9 })
    );
    assert_eq!(
        hooks_eligible(NAME, "unrecognized hook row\n", GLOBAL),
        Err(HookRefusal::Unread)
    );
}

#[test]
fn parser_caps_an_unterminated_line_and_the_sum_of_complete_screen_rows() {
    assert_eq!(ControlParser::MAX_LINE, 64 * 1024);
    assert_eq!(ControlParser::MAX_BODY, 1024 * 1024);
    let mut parser = expecting(SidecarCommand::Space);
    assert!(parser.feed(b"%begin 21 2 1\n").is_empty());
    let long = vec![b'x'; ControlParser::MAX_LINE + 1];
    assert_eq!(parser.feed(&long), [ControlEvent::Overflow]);
    assert!(parser.feed(b"\n%end 21 2 1\n").is_empty());

    let mut parser = expecting(SidecarCommand::Frame);
    assert!(matches!(
        parser
            .feed(b"%begin 21 2 1\n$7 %9 18 40 0 0\n%end 21 2 1\n%begin 21 3 1\n")
            .as_slice(),
        [ControlEvent::Facts(_)]
    ));
    let mut row = vec![b'x'; ControlParser::MAX_LINE / 2];
    row.push(b'\n');
    let events: Vec<_> = (0..40).flat_map(|_| parser.feed(&row)).collect();
    assert_eq!(events, [ControlEvent::Overflow]);
    assert!(parser.feed(b"%end 21 3 1\n").is_empty());
}

#[test]
fn only_canonical_ids_and_a_nonzero_pid_can_be_interpolated_into_a_command() {
    assert!(Proven::new("$7", "%9", 18).is_some());
    for bad in ["", "voice", "$7:0", "$7\n", "$७", "$7; next", "$07"] {
        assert!(Proven::new(bad, "%9", 18).is_none(), "{bad:?}");
    }
    for bad in ["", "9", "%9:0", "%9\r", "%９", "%9; next", "%09"] {
        assert!(Proven::new("$7", bad, 18).is_none(), "{bad:?}");
    }
    assert!(Proven::new("$7", "%9", 0).is_none());
}
