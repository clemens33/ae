// Frozen bgmark acceptance (.local/bgmark/freeze.tsv), installed byte-identically.
include!("../../tests/spec/bgmark_old_reader.rs");
use super::{
    CLIENT_FORMAT, Key, ObservedClient, ObservedPaneProbe, PANE_PROBE_FORMAT, Styling,
    capture_screen_args, confirm_before_args, display_client_message_args, hotkey_picker_shell,
    interpret_clients, interpret_pane_probe, list_clients_args, load_buffer_args, pane_probe_args,
    paste_buffer_args, run_shell_background_command, send_keys_args, status_picker_command,
    status_settings_command,
};

#[test]
fn the_delivery_argv_addresses_the_named_pane_and_never_selects_it() {
    let sock = ServerId::Selected(Selector::Socket(std::path::PathBuf::from("/tmp/s")));
    assert_eq!(
        pane_probe_args(&ServerId::Ambient, "%3"),
        vec!["display-message", "-p", "-t", "%3", PANE_PROBE_FORMAT]
    );
    assert_eq!(
        capture_screen_args(&ServerId::Ambient, "%3", Styling::Plain),
        vec!["capture-pane", "-p", "-t", "%3"],
        "the start-up marker scan wants the rows the TUI drew, unstyled"
    );
    assert_eq!(
        capture_screen_args(&ServerId::Ambient, "%3", Styling::Escapes),
        vec!["capture-pane", "-e", "-p", "-S", "0", "-t", "%3"],
        "the occupancy sensor decides live-vs-echo from SGR state alone"
    );
    assert_eq!(
        load_buffer_args(&sock, "b"),
        vec!["-S", "/tmp/s", "load-buffer", "-b", "b", "-"],
        "`-` is the SOURCE: the bytes ride stdin, never argv"
    );
    assert_eq!(
        paste_buffer_args(&ServerId::Ambient, "b", "%3"),
        vec!["paste-buffer", "-d", "-p", "-b", "b", "-t", "%3"],
        "-p REQUESTS bracketing; -d leaves no body in the buffer stack"
    );
    assert_eq!(
        send_keys_args(&ServerId::Ambient, "%3", Key::Enter),
        vec!["send-keys", "-t", "%3", "Enter"]
    );
    assert_eq!(
        send_keys_args(&ServerId::Ambient, "%3", Key::ClearLine),
        vec!["send-keys", "-t", "%3", "C-u"]
    );
    assert_eq!(
        send_keys_args(&ServerId::Ambient, "%3", Key::LineEnd),
        vec!["send-keys", "-t", "%3", "C-e"],
        "`C-u` kills only to the line start: the cursor goes to the END first"
    );
    assert_eq!(
        send_keys_args(&ServerId::Ambient, "%3", Key::Escape),
        vec!["send-keys", "-t", "%3", "Escape"]
    );
    assert_eq!(
        send_keys_args(&ServerId::Ambient, "%3", Key::CancelCopyMode),
        vec!["send-keys", "-t", "%3", "-X", "cancel"],
        "a COMMAND, not a key — which is why the set is an enum"
    );
    for (key, name) in [(Key::Up, "Up"), (Key::Down, "Down")] {
        assert_eq!(
            send_keys_args(&ServerId::Ambient, "%3", key),
            vec!["send-keys", "-t", "%3", name],
            "tmux's own arrow names, never a digit or text"
        );
    }
    assert_eq!(
        list_clients_args(&ServerId::Ambient),
        vec!["list-clients", "-F", CLIENT_FORMAT]
    );
    assert!(
        !send_keys_args(&ServerId::Ambient, "%3", Key::Enter)
            .iter()
            .any(|arg| arg == "select-pane"),
        "focus is not part of any TUI's submission contract"
    );
}

#[test]
fn a_pane_probe_is_read_only_from_a_run_that_succeeded() {
    assert_eq!(interpret_pane_probe(false, "123 | claude\n"), None);
    assert_eq!(
        interpret_pane_probe(true, "123 | claude\n"),
        Some(ObservedPaneProbe {
            command: "claude".into(),
            pid: Some(123)
        })
    );
    assert_eq!(
        interpret_pane_probe(true, "not-a-pid | bash\n"),
        Some(ObservedPaneProbe {
            command: "bash".into(),
            pid: None
        }),
        "an unreadable pid is no pid, not no pane"
    );
    assert_eq!(interpret_pane_probe(true, "only-one-field\n"), None);
    assert_eq!(
        interpret_pane_probe(true, "7 | odd | command\n"),
        Some(ObservedPaneProbe {
            command: "odd | command".into(),
            pid: Some(7)
        }),
        "the free-text command is last, so it may carry the separator"
    );
    assert_eq!(interpret_pane_probe(true, ""), None);
}

#[test]
fn a_pane_size_is_three_numbers_from_a_run_that_succeeded() {
    use super::{PANE_SIZE_FORMAT, PaneSize, interpret_pane_size, pane_size_args};

    let size = |width, height, cursor_y| PaneSize {
        width,
        height,
        cursor_y,
    };
    assert_eq!(
        pane_size_args(&ServerId::Ambient, "%3"),
        vec!["display-message", "-p", "-t", "%3", PANE_SIZE_FORMAT]
    );
    assert_eq!(
        interpret_pane_size(true, "80 | 24 | 5\n"),
        Some(size(80, 24, 5))
    );
    assert_eq!(interpret_pane_size(true, "1 | 1 | 0"), Some(size(1, 1, 0)));
    for bad in [
        "",
        "80 | 24\n",
        "80 | 24 | 5 | 9",
        "0 | 24 | 5",
        "80 | 0 | 5",
    ] {
        assert_eq!(interpret_pane_size(true, bad), None, "{bad:?}");
    }
    for bad in ["80 | x | 5", "-1 | 24 | 5", "80 |  24 | 5", "80 | 24 |"] {
        assert_eq!(interpret_pane_size(true, bad), None, "{bad:?}");
    }
    assert_eq!(interpret_pane_size(false, "80 | 24 | 5"), None);
}

#[test]
fn picker_client_session_is_one_exact_client_listing() {
    use super::PickerClient;

    assert_eq!(
        super::picker_client_sessions_args(&ServerId::Ambient),
        [
            "list-clients",
            "-F",
            "#{client_name} | #{session_id} | #{client_pid} | #{client_height} | #{client_width}",
        ]
    );
    let listing = "/dev/ttys001 | $1 | 4242 | 24 | 80\n/dev/ttys002 | $7 | 4243 | 40 | 140\n";
    assert_eq!(
        super::interpret_picker_client_session(true, listing, "/dev/ttys002"),
        Some(PickerClient {
            session_id: "$7".to_owned(),
            pid: "4243".to_owned(),
            height: 40,
            width: 140,
        })
    );
    assert_eq!(
        super::interpret_picker_client_session(true, listing, "/dev/ttys00"),
        None,
        "client names never prefix-match"
    );
    assert_eq!(
        super::interpret_picker_client_session(
            true,
            "/dev/ttys002 | named | 4243 | 40 | 140\n",
            "/dev/ttys002",
        ),
        None,
        "only a tmux session id can become a write target"
    );
    for malformed in [
        "/dev/ttys002 | $7 | 4243 | | 140\n",
        "/dev/ttys002 | $7 | 4243 | 40 | \n",
        "/dev/ttys002 | $7 | 4243 | tall | 140\n",
        "/dev/ttys002 | $7 | 4243 | 40 | wide\n",
        "/dev/ttys002 | $7 | notapid | 40 | 140\n",
        "/dev/ttys002 | $7 | | 40 | 140\n",
    ] {
        assert_eq!(
            super::interpret_picker_client_session(true, malformed, "/dev/ttys002"),
            None,
            "{malformed:?}"
        );
    }
    assert_eq!(
        super::interpret_picker_client_session(false, listing, "/dev/ttys002"),
        None
    );
}

/// The picker's pre-draw read is ONE tmux invocation: every read command and
/// its completion marker in one queued list, with tmux's own `;` between the
/// commands and NO write anywhere in it. The exact argv is the pin; the
/// mutation it kills is moving the `@ae_menu_open` marker write — or any
/// other command — into the batch.
#[test]
fn the_picker_read_is_one_queued_invocation_and_carries_no_write() {
    use super::{READ_CLIENTS, READ_DONE_STEM, READ_LOOK, READ_SESSIONS, READ_STEM};
    let args = super::picker_read_args(&ServerId::Ambient, "/dev/ttys002");
    assert_eq!(
        args.join(" "),
        concat!(
            "list-clients -F ae-picker:c|#{client_name} | #{session_id} | #{client_pid} | #{client_height} | #{client_width}",
            " ; display-message -p ae-picker!c",
            " ; list-sessions -F ae-picker:s|#{session_name} | #{session_id} | #{@ae_attn_rank} | #{@ae_attn_glyph} | #{@ae_main_pane} | #{s/#{l:[|[:cntrl:]]}//:@ae_branch_name} | #{s/#{l:[|[:cntrl:]]}/!/:@ae_agents} | #{session_activity} | #{session_created} | #{@ae_goal_status}",
            " ; display-message -p ae-picker!s",
            " ; list-panes -a -F ae-picker:p|#{session_id} | #{pane_id}",
            " ; display-message -p ae-picker!p",
            " ; list-sessions -F ae-picker:n|#{session_name}",
            " ; display-message -p ae-picker!n",
            " ; display-message -p ae-picker:i|#{pid} | #{start_time}",
            " ; display-message -p ae-picker!i",
            " ; display-message -p ae-picker:k|#{socket_path}",
            " ; display-message -p ae-picker!k",
            " ; display-message -p -c /dev/ttys002 ae-picker:l|#{@ae_icons} | #{@ae_palette} | #{@ae_look} | #{@ae_motion}",
            " ; display-message -p ae-picker!l",
        ),
        "one queued read, one prefix per command, no write"
    );
    assert_eq!(
        args.iter().filter(|arg| *arg == ";").count(),
        13,
        "each separator is an argument of its own: 14 commands, ONE invocation"
    );
    let args = super::picker_read_args(&ServerId::Selected(Selector::Name("ae".to_owned())), "c");
    assert_eq!(args[..2], ["-L", "ae"], "the server args lead, once");
    assert!(
        !args.iter().any(|arg| arg == "set-option"),
        "the marker write stays its own call"
    );
    assert!(
        args.iter().all(|arg| !arg.chars().any(char::is_control)),
        "no format smuggles a control byte tmux would escape"
    );
    assert_eq!(READ_STEM, "ae-picker:");
    assert_eq!(READ_DONE_STEM, "ae-picker!");
    assert_eq!(
        [READ_CLIENTS, READ_SESSIONS, READ_LOOK].map(super::read_section),
        [Some(0), Some(1), Some(6)],
        "each kind letter names its own field"
    );
}

#[test]
fn the_picker_read_splits_by_prefix_and_refuses_a_foreign_line() {
    use super::PickerClient;
    let listing = concat!(
        "ae-picker:c|/dev/ttys002 | $7 | 4243 | 40 | 140\n",
        "ae-picker!c\n",
        "ae-picker:s|hub | $7 | 2 |  | %10 | main | v1;2000;60;lead:fable5:working:%10 | the goal\n",
        "ae-picker!s\n",
        "ae-picker:p|$7 | %10\n",
        "ae-picker!p\n",
        "ae-picker:n|hub\n",
        "ae-picker!n\n",
        "ae-picker:i|4242 | 1700000000\n",
        "ae-picker!i\n",
        "ae-picker:k|/tmp/ae.sock\n",
        "ae-picker!k\n",
        "ae-picker:l|on | a | on | on\n",
        "ae-picker!l\n",
    );
    let read = super::interpret_picker_read(listing, "/dev/ttys002").expect("one read");
    assert_eq!(
        (
            read.client,
            read.sessions
                .as_ref()
                .and_then(|rows| rows.first())
                .map(|row| row.name.as_str()),
            read.panes.as_ref().map(Vec::len),
            read.names.as_deref(),
            read.socket.as_deref(),
        ),
        (
            Some(PickerClient {
                session_id: "$7".to_owned(),
                pid: "4243".to_owned(),
                height: 40,
                width: 140,
            }),
            Some("hub"),
            Some(1),
            Some(["hub".to_owned()].as_slice()),
            Some("/tmp/ae.sock"),
        )
    );
    assert_eq!(
        read.identity,
        Some(super::ServerIdentity {
            pid: "4242".to_owned(),
            start: "1700000000".to_owned(),
        })
    );
    assert_eq!(
        read.look.map(|look| (look.icons, look.palette)),
        Some(("on".to_owned(), "a".to_owned()))
    );
    assert_eq!(
        super::interpret_picker_read(&format!("{listing}plausible but unknown\n"), "/dev/ttys002"),
        None,
        "a line from neither vocabulary refuses the WHOLE read, never a partial roster"
    );
    let missing = listing.replace("ae-picker!p\n", "");
    let read = super::interpret_picker_read(&missing, "/dev/ttys002").expect("one read");
    assert_eq!(read.panes, None, "a marker that never arrived is unknown");
    assert_eq!(
        read.names,
        Some(vec!["hub".to_owned()]),
        "later sections still count"
    );
    assert_eq!(
        super::interpret_picker_read("", "/dev/ttys002")
            .expect("an empty run")
            .client,
        None,
        "a silent server resolves no client"
    );
}

#[test]
fn menu_client_snapshot_carries_one_validated_session_name() {
    assert_eq!(
        super::menu_clients_args(&ServerId::Ambient),
        [
            "list-clients",
            "-F",
            "#{client_name} | #{client_pid} | #{session_id} | #{session_name} | #{client_height} | #{client_width}",
        ]
    );
    let listing =
        "/dev/ttys001 | 41 | $1 | other | 24 | 80\n/dev/ttys002 | 42 | $7 | viewed | 40 | 140\n";
    assert_eq!(
        super::interpret_menu_client(true, listing, "/dev/ttys002"),
        Some(super::MenuClient {
            pid: "42".to_owned(),
            session_id: "$7".to_owned(),
            session_name: "viewed".to_owned(),
            height: 40,
            width: 140,
        })
    );
    for invalid in ["", "../viewed", "viewed space"] {
        let row = format!("/dev/ttys002 | 42 | $7 | {invalid} | 40 | 140\n");
        assert_eq!(
            super::interpret_menu_client(true, &row, "/dev/ttys002"),
            None,
            "{invalid:?}"
        );
    }
    assert_eq!(
        super::interpret_menu_client(true, &format!("{listing}{listing}"), "/dev/ttys002"),
        None,
        "duplicate matching rows remain ambiguous"
    );
}

#[test]
fn picker_menu_anchors_at_the_client_left_above_the_status_line() {
    assert_eq!(super::MENU_POSITION, ["-x", "0", "-y", "S"]);
}

#[test]
fn clients_report_the_pane_each_is_viewing_and_when_it_last_typed() {
    assert_eq!(interpret_clients(false, "/dev/tty1 | s | %1 | 100\n"), None);
    assert_eq!(
        interpret_clients(
            true,
            "/dev/tty1 | s | %1 | 100\n/dev/tty2 | other | %2 | nope\n\n"
        ),
        Some(vec![
            ObservedClient {
                name: "/dev/tty1".into(),
                session: "s".into(),
                pane: "%1".into(),
                activity: Some(100)
            },
            ObservedClient {
                name: "/dev/tty2".into(),
                session: "other".into(),
                pane: "%2".into(),
                activity: None
            }
        ])
    );
    assert_eq!(
        interpret_clients(true, ""),
        Some(Vec::new()),
        "no clients attached is an ANSWER; a failed run is not"
    );
}

#[test]
fn confirmation_targets_one_client_and_runs_the_supervisor_in_background() {
    let argv = vec![
        "/opt/ae core/ae's".to_owned(),
        "_end".to_owned(),
        "--handoff".to_owned(),
        "inside".to_owned(),
    ];
    let continuation = run_shell_background_command(&argv);
    assert_eq!(
        continuation,
        r#"run-shell -b "'/opt/ae core/ae'\\''s' '_end' '--handoff' 'inside'""#
    );
    assert_eq!(
        run_shell_background_command(&["/opt/$ae#core".to_owned()]),
        r#"run-shell -b "'/opt/\$ae##core'""#
    );
    assert_eq!(
        confirm_before_args(
            &ServerId::Selected(Selector::Name("ae-dev".to_owned())),
            "/dev/ttys004",
            "End 'inside'? Archives, then deletes its state. (y/n)",
            &continuation,
        ),
        vec![
            "-L",
            "ae-dev",
            "confirm-before",
            "-p",
            "End 'inside'? Archives, then deletes its state. (y/n)",
            "-t",
            "/dev/ttys004",
            continuation.as_str(),
        ]
    );
    assert_eq!(
        display_client_message_args(&ServerId::Ambient, "/dev/ttys004", "Stopped inside",),
        vec![
            "display-message",
            "-d",
            "10000",
            "-c",
            "/dev/ttys004",
            "Stopped inside",
        ]
    );
}

#[test]
fn the_status_picker_quotes_fixed_words_and_expands_only_the_clicking_client() {
    let launcher = [
        "env".to_owned(),
        "AE_HOME=/tmp/ae home".to_owned(),
        "/tmp/ae checkout/target/debug/ae".to_owned(),
    ];
    assert_eq!(
        status_picker_command(&launcher),
        r#"run-shell -b "'env' 'AE_HOME=/tmp/ae home' '/tmp/ae checkout/target/debug/ae' 'orchestrator' '--popup' '--client' #{q:client_name}""#
    );
    assert_eq!(
        status_picker_command(&[
            "env".to_owned(),
            "AE_HOME=/tmp/ae#,state".to_owned(),
            "/tmp/ae}core".to_owned(),
        ]),
        r#"run-shell -b "'env' 'AE_HOME=/tmp/ae#####,state' '/tmp/ae#}core' 'orchestrator' '--popup' '--client' #{q:client_name}""#
    );
}

#[test]
fn the_status_settings_button_preserves_launcher_punctuation_and_one_client_expansion() {
    assert_eq!(
        status_settings_command(&[
            "env".to_owned(),
            "AE_HOME=/tmp/ae#,state".to_owned(),
            "/tmp/ae}core".to_owned(),
        ]),
        r#"run-shell -b "'env' 'AE_HOME=/tmp/ae#####,state' '/tmp/ae#}core' 'orchestrator' '--settings' '--client' #{q:client_name}""#
    );
}

#[test]
fn the_hotkey_picker_crosses_one_format_layer_without_conditional_escapes() {
    assert_eq!(
        hotkey_picker_shell(&[
            "env".to_owned(),
            "AE_HOME=/tmp/ae#,state".to_owned(),
            "/tmp/ae}core".to_owned(),
        ]),
        "'env' 'AE_HOME=/tmp/ae##,state' '/tmp/ae}core' 'orchestrator' '--popup' '--client' #{q:client_name}"
    );
}

#[test]
fn the_format_escape_doubles_hash_then_percent() {
    use super::format_literal;
    // `#(cmd)` in a tmux format RUNS A SHELL; `%` is strftime.
    assert_eq!(format_literal("plain text"), "plain text");
    assert_eq!(format_literal("#(id)"), "##(id)");
    assert_eq!(format_literal("#{session_name}"), "##{session_name}");
    assert_eq!(format_literal("100%"), "100%%");
    assert_eq!(format_literal("#%"), "##%%");
    assert_eq!(format_literal("#"), "##");
    // NOT idempotent, and must not be: escaping twice is a rendering bug,
    // so the one call site is the place that must not grow a second.
    assert_eq!(format_literal("##"), "####");
    // A hostile agent name is the realistic carrier.
    assert_eq!(
        format_literal("[ae watchdog] cl:#(touch /tmp/pwned) is DEAD"),
        "[ae watchdog] cl:##(touch /tmp/pwned) is DEAD"
    );
}

#[test]
fn quota_hashes_are_budgeted_as_display_cells_then_escaped_in_the_final_menu_argv() {
    let label = "quota #[fg=red] #";
    let menu = super::Menu {
        title: "ae settings".to_owned(),
        title_style: String::new(),
        items: vec![super::MenuItem {
            label: label.to_owned(),
            key: String::new(),
            action: super::MenuAction::Disabled,
        }],
    };
    let budget = crate::session_menu::menu_budget(&menu);
    assert_eq!(budget.0, label.chars().count() + 4);
    let x = super::settings_menu_x(90, budget.0);
    let args = super::display_settings_menu_args(
        &ServerId::Ambient,
        "/dev/ttys004",
        "$7",
        x,
        &menu,
        false,
    );
    let position = [
        "-x".to_owned(),
        x.to_string(),
        "-y".to_owned(),
        "S".to_owned(),
    ];
    assert!(args.windows(4).any(|words| words == position), "{args:?}");
    assert!(!args.iter().any(|word| word == "R"), "{args:?}");
    let escaped = "-quota ##[fg=red] ##";
    assert!(args.iter().any(|word| word == escaped), "{args:?}");
    assert_ne!(budget.0, escaped.chars().count() + 4);
}

#[test]
fn settings_menu_x_uses_client_width_and_saturates_when_the_budget_is_wider() {
    assert_eq!(super::settings_menu_x(100, 75), 25);
    assert_eq!(super::settings_menu_x(74, 26), 48);
    assert_eq!(super::settings_menu_x(25, 26), 0);
    assert_eq!(super::settings_menu_x(0, 1), 0);
}

#[test]
fn the_zone_question_asks_the_sessions_server_for_its_clock_offset_and_nothing_else() {
    use super::{ZONE_FORMAT, zone_args};
    use crate::inventory::ServerId;
    use crate::meta::Selector;
    let server = ServerId::Selected(Selector::Name("ae".to_owned()));
    assert_eq!(
        zone_args(&server, "one"),
        [
            "-L",
            "ae",
            "display-message",
            "-p",
            "-t",
            "=one:",
            ZONE_FORMAT
        ]
    );
    assert_eq!(ZONE_FORMAT, "#{t/f/%z:start_time}");
}

#[test]
fn a_zone_answer_is_the_first_line_of_a_completed_run_and_nothing_else() {
    use super::interpret_zone;
    let raw = Some("not a zone\x1b[31m".to_owned());
    assert_eq!(interpret_zone(true, "not a zone\x1b[31m\nsecond\n"), raw);
    assert_eq!(interpret_zone(true, "+0200  \n"), Some("+0200".to_owned()));
    assert_eq!(
        interpret_zone(false, "+0200\n"),
        None,
        "a run that failed says nothing"
    );
    assert_eq!(interpret_zone(true, ""), None);
}

#[test]
fn a_user_option_write_targets_an_exact_id_in_the_right_table() {
    use super::{OptionScope, set_option_args, unset_option_args};
    use crate::inventory::ServerId;
    use crate::meta::Selector;
    let server = ServerId::Selected(Selector::Name("ae".to_owned()));
    // The window table needs -w, and the target is a window id.
    assert_eq!(
        set_option_args(
            &server,
            OptionScope::Window,
            "@7",
            crate::theme::WINDOW_AGENTS_OPTION,
            "[●lead ◌builder]"
        ),
        [
            "-L",
            "ae",
            "set-option",
            "-w",
            "-t",
            "@7",
            "@ae_window_agents",
            "[●lead ◌builder]"
        ]
    );
    assert_eq!(
        unset_option_args(
            &server,
            OptionScope::Window,
            "@7",
            crate::theme::WINDOW_AGENTS_OPTION,
        ),
        [
            "-L",
            "ae",
            "set-option",
            "-w",
            "-u",
            "-t",
            "@7",
            "@ae_window_agents"
        ]
    );
}

#[test]
fn option_writes_are_one_tmux_command_with_literal_argv_separators() {
    use super::{OptionScope, OptionWrite, set_options_args};
    use crate::inventory::ServerId;
    use crate::meta::Selector;
    let server = ServerId::Selected(Selector::Name("ae".to_owned()));
    let writes = [
        OptionWrite::new(
            OptionScope::Pane,
            "%1",
            "@ae_pane_state",
            "#[fg=#6897BB]⠋#[default] working",
        ),
        OptionWrite::new(
            OptionScope::Window,
            "@7",
            crate::theme::WINDOW_AGENTS_OPTION,
            "[⠋lead ◌builder]",
        ),
    ];
    assert_eq!(
        set_options_args(&server, &writes),
        [
            "-L",
            "ae",
            "set-option",
            "-p",
            "-t",
            "%1",
            "@ae_pane_state",
            "#[fg=#6897BB]⠋#[default] working",
            ";",
            "set-option",
            "-w",
            "-t",
            "@7",
            "@ae_window_agents",
            "[⠋lead ◌builder]",
        ]
    );
}

/// The command bytes tmux counts for one chunk: its words after the
/// server's own, each with its NUL.
fn chunk_bytes(server: &crate::inventory::ServerId, args: &[String]) -> usize {
    args[super::server_args(server).len()..]
        .iter()
        .map(|word| word.len() + 1)
        .sum()
}

#[test]
fn an_ordinary_cycle_is_one_invocation_with_the_wake_hook_first() {
    use super::{CycleOp, OptionScope, OptionWrite, WAKE_HOOK, WAKE_HOOK_MARKER};
    use crate::inventory::ServerId;
    use crate::meta::Selector;
    let server = ServerId::Selected(Selector::Name("ae".to_owned()));
    let ops = [
        CycleOp::Set(OptionWrite::new(OptionScope::Session, "$3", "@a", "1")),
        CycleOp::Unset {
            scope: OptionScope::Window,
            target: "@7".to_owned(),
            name: "@b".to_owned(),
        },
        CycleOp::Set(OptionWrite::new(OptionScope::Pane, "%2", "@c", "x y")),
    ];
    let chunks = super::cycle_batch_chunks(&server, "$3", Some("ae-wake-3-17"), &ops);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].ops, 0..3);
    assert_eq!(
        chunks[0].args,
        [
            "-L",
            "ae",
            "set-hook",
            "-t",
            "=$3:",
            WAKE_HOOK,
            "wait-for -S ae-wake-3-17",
            ";",
            "display-message",
            "-p",
            "-t",
            "=$3:",
            WAKE_HOOK_MARKER,
            ";",
            "set-option",
            "-t",
            "=$3:",
            "@a",
            "1",
            ";",
            "set-option",
            "-w",
            "-u",
            "-t",
            "@7",
            "@b",
            ";",
            "set-option",
            "-p",
            "-t",
            "%2",
            "@c",
            "x y",
        ]
    );
    // No hook and no operation is no invocation at all.
    assert!(super::cycle_batch_chunks(&server, "$3", None, &[]).is_empty());
}

#[test]
fn a_cycle_past_the_command_bound_splits_on_whole_commands_in_order() {
    use super::{COMMAND_BYTES, CycleOp, OptionScope, OptionWrite, WAKE_HOOK_MARKER};
    use crate::inventory::ServerId;
    use crate::meta::Selector;
    let server = ServerId::Selected(Selector::Name("ae".to_owned()));
    let wide = "x".repeat(5_000);
    let ops: Vec<CycleOp> = (0..7)
        .map(|n| {
            CycleOp::Set(OptionWrite::new(
                OptionScope::Session,
                "$3",
                &format!("@v{n}"),
                &wide,
            ))
        })
        .collect();
    let chunks = super::cycle_batch_chunks(&server, "$3", Some("ae-wake-3-17"), &ops);
    assert!(chunks.len() > 1, "35 KB cannot be one 3.4 command");
    let mut next = 0;
    for (index, chunk) in chunks.iter().enumerate() {
        assert!(
            chunk_bytes(&server, &chunk.args) <= COMMAND_BYTES,
            "chunk {index} passes tmux 3.4's bound"
        );
        assert_eq!(chunk.ops.start, next, "the operations stay in order");
        next = chunk.ops.end;
        assert_eq!(
            chunk.args.iter().any(|word| word == WAKE_HOOK_MARKER),
            index == 0,
            "the hook block rides the first invocation only"
        );
        assert_eq!(
            chunk
                .args
                .iter()
                .filter(|word| *word == "set-option")
                .count(),
            chunk.ops.len(),
            "a chunk carries exactly its own operations"
        );
    }
    assert_eq!(next, ops.len(), "every operation is carried once");

    // One command over the bound travels alone, as it did unbatched.
    let huge = [
        CycleOp::Set(OptionWrite::new(OptionScope::Session, "$3", "@a", "1")),
        CycleOp::Set(OptionWrite::new(
            OptionScope::Session,
            "$3",
            "@huge",
            &"y".repeat(COMMAND_BYTES),
        )),
        CycleOp::Set(OptionWrite::new(OptionScope::Session, "$3", "@b", "2")),
    ];
    let chunks = super::cycle_batch_chunks(&server, "$3", None, &huge);
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.ops.clone())
            .collect::<Vec<_>>(),
        [0..1, 1..2, 2..3]
    );
}

#[test]
fn a_wake_channel_is_one_bare_word_of_the_hook_command() {
    for channel in ["ae-wake-3-1731000000", "qw_switch"] {
        assert!(super::is_wake_channel(channel), "{channel}");
    }
    for channel in ["", "a b", "a;b", "a'b", "a\"b", "a\\b", "ä", "a\nb", "#{x}"] {
        assert!(!super::is_wake_channel(channel), "{channel:?}");
    }
}

#[test]
fn a_wake_channel_names_one_session_incarnation() {
    let identity = |id: &str, created: &str| super::SessionIdentity {
        id: id.to_owned(),
        created: created.to_owned(),
    };
    assert_eq!(
        super::wake_channel(&identity("$3", "1731000000")).as_deref(),
        Some("ae-wake-3-1731000000")
    );
    assert_ne!(
        super::wake_channel(&identity("$3", "1731000000")),
        super::wake_channel(&identity("$3", "1731000001")),
        "a new session under a reused id is a new channel"
    );
    for (id, created) in [
        ("3", "1"),
        ("$", "1"),
        ("$3", ""),
        ("$3a", "1"),
        ("$3", "1 2"),
        ("@3", "1"),
    ] {
        assert_eq!(
            super::wake_channel(&identity(id, created)),
            None,
            "{id:?} {created:?}"
        );
    }
}

#[test]
fn the_waiter_reads_attachment_then_waits_and_its_release_names_the_same_channel() {
    use crate::inventory::ServerId;
    use crate::meta::Selector;
    let server = ServerId::Selected(Selector::Name("ae".to_owned()));
    assert_eq!(
        super::waiter_args(&server, "s1", "ae-wake-3-9"),
        [
            "-L",
            "ae",
            "display-message",
            "-p",
            "-t",
            "=s1:",
            "ae-wd:a|#{session_attached}",
            ";",
            "wait-for",
            "ae-wake-3-9"
        ]
    );
    assert_eq!(
        super::wake_release_args(&server, "ae-wake-3-9"),
        ["-L", "ae", "wait-for", "-S", "--", "ae-wake-3-9"]
    );
    // A channel may lead with `-`: it is an operand, never a flag.
    assert_eq!(
        super::channel_waiter_args(&server, "-step"),
        ["-L", "ae", "wait-for", "--", "-step"]
    );
}

#[test]
fn a_waiter_read_is_one_whole_count_line() {
    use super::WaitRead::{Attached, Malformed, Pending};
    assert_eq!(super::interpret_wait_read(""), Pending);
    assert_eq!(super::interpret_wait_read("ae-wd:a|1"), Pending);
    assert_eq!(super::interpret_wait_read("ae-wd:a|0\n"), Attached(0));
    assert_eq!(super::interpret_wait_read("ae-wd:a|12\n"), Attached(12));
    for bad in [
        "ae-wd:a|\n",
        "ae-wd:a|1\r\n",
        "ae-wd:a|-1\n",
        "ae-wd:a|+1\n",
        "x\nae-wd:a|1\n",
        "ae-wd:k|1\n",
        "ae-wd:a|99999999999999999999999\n",
    ] {
        assert_eq!(super::interpret_wait_read(bad), Malformed, "{bad:?}");
    }
}

#[test]
fn the_ownership_read_is_todays_reads_each_behind_a_nonce_marker() {
    use crate::inventory::ServerId;
    use crate::meta::Selector;
    let server = ServerId::Selected(Selector::Name("ae".to_owned()));
    let query = super::ownership_read_args(&server, "n1", true, &["peer".to_owned()]);
    assert_eq!(query.peers, 1);
    assert_eq!(
        query.args,
        [
            "-L",
            "ae",
            "display-message",
            "-p",
            "ae-wd:k|#{socket_path}",
            ";",
            "display-message",
            "-p",
            "ae-wd!n1!k",
            ";",
            "show-environment",
            "-t",
            "=peer",
            "AE_SESSION",
            ";",
            "display-message",
            "-p",
            "ae-wd!n1!0m",
            ";",
            "show-environment",
            "-t",
            "=peer",
            "AE_HOME",
            ";",
            "display-message",
            "-p",
            "ae-wd!n1!0h",
        ]
    );
}

#[test]
fn a_hostile_multi_line_value_spoils_only_its_own_proof() {
    // Peer 0's AE_SESSION carries lines shaped like another peer's
    // sections; without the nonce they are just extra lines in ITS section.
    let stdout = "ae-wd:k|/tmp/s\nae-wd!N!k\n\
                      AE_SESSION=1\nae-wd!fake!1m\nAE_HOME=/forged\nae-wd!N!0m\n\
                      AE_HOME=/h0\nae-wd!N!0h\n\
                      AE_SESSION=1\nae-wd!N!1m\nAE_HOME=/h1\nae-wd!N!1h\n";
    let answer = super::interpret_ownership_read(stdout, "N", true, 2);
    assert_eq!(answer.socket.as_deref(), Some("/tmp/s"));
    assert_eq!(
        answer.peers,
        [None, Some(("1".to_owned(), "/h1".to_owned()))]
    );
}

#[test]
fn an_ownership_read_stopped_at_a_peer_decides_it_and_nothing_after_it() {
    // Peer 1's AE_HOME is unset: tmux stopped the list after its first marker.
    let stdout = "AE_SESSION=1\nae-wd!N!0m\nAE_HOME=/h0\nae-wd!N!0h\n\
                      AE_SESSION=1\nae-wd!N!1m\n";
    let answer = super::interpret_ownership_read(stdout, "N", false, 3);
    assert_eq!(
        answer.peers,
        [Some(("1".to_owned(), "/h0".to_owned())), None],
        "peer 1 is decided unproven; peer 2 is asked again"
    );
    // A socket section that never completed means nothing after it ran.
    let answer = super::interpret_ownership_read("", "N", true, 3);
    assert_eq!(answer, super::OwnershipAnswer::default());
    // Without the socket, the first peer is always decided.
    let answer = super::interpret_ownership_read("", "N", false, 3);
    assert_eq!(answer.peers, [None]);
    // An empty marker is no proof, and neither is a removed variable.
    let stdout = "AE_SESSION=\nae-wd!N!0m\nAE_HOME=/h\nae-wd!N!0h\n\
                      -AE_SESSION\nae-wd!N!1m\nAE_HOME=/h\nae-wd!N!1h\n";
    assert_eq!(
        super::interpret_ownership_read(stdout, "N", false, 2).peers,
        [None, None]
    );
}

#[test]
fn an_ownership_read_stays_inside_the_command_bound() {
    use super::COMMAND_BYTES;
    use crate::inventory::ServerId;
    use crate::meta::Selector;
    let server = ServerId::Selected(Selector::Name("ae".to_owned()));
    let peers: Vec<String> = (0..200).map(|n| format!("{n:0>40}")).collect();
    let query = super::ownership_read_args(&server, "n1", true, &peers);
    assert!(query.peers > 0 && query.peers < peers.len());
    assert!(chunk_bytes(&server, &query.args) <= COMMAND_BYTES);
    // One peer is always asked, even past the bound.
    let huge = ["x".repeat(COMMAND_BYTES)];
    assert_eq!(
        super::ownership_read_args(&server, "n1", false, &huge).peers,
        1
    );
}

#[test]
fn the_ownership_framing_hands_each_line_over_byte_for_byte() {
    // tmux 3.7b prints a value's trailing CR raw (probed); today's reader
    // keeps it, so the joined read must too.
    let alone = "AE_HOME=/qw/home\r\n";
    assert_eq!(
        super::interpret_environment_value(true, alone, super::HOME_VARIABLE).as_deref(),
        Some("/qw/home\r")
    );
    let joined = "AE_SESSION=1\r\nae-wd!N!0m\nAE_HOME=/qw/home\r\nae-wd!N!0h\n";
    assert_eq!(
        super::interpret_ownership_read(joined, "N", false, 1).peers,
        [Some(("1".to_owned(), "/qw/home\r".to_owned()))],
        "the marker trims as today's interpret_marker does; the home keeps its CR"
    );
}

/// The command words of `args`, without the server words in front.
fn command_words(server: &crate::inventory::ServerId, args: &[String]) -> Vec<String> {
    args[super::server_args(server).len()..].to_vec()
}

/// A queued read's marker command, behind its `;`.
fn marker_words(marker: &str) -> Vec<String> {
    [";", "display-message", "-p", marker]
        .map(ToOwned::to_owned)
        .into()
}

#[test]
fn the_motion_exchange_is_todays_two_reads_each_behind_a_nonce_marker_then_the_writes() {
    use super::{OptionScope, OptionWrite};
    use crate::inventory::ServerId;
    use crate::meta::Selector;
    let server = ServerId::Selected(Selector::Name("ae".to_owned()));
    let writes = [
        OptionWrite::new(OptionScope::Pane, "%3", "@ae_pane_state", "working"),
        OptionWrite::new(OptionScope::Window, "@2", "@ae_window_agents", "x"),
    ];
    let mut reads = super::server_args(&server);
    reads.extend(command_words(
        &server,
        &super::motion_panes_args(&server, "s"),
    ));
    reads.extend(marker_words("ae-wd!n1!p"));
    reads.push(";".to_owned());
    reads.extend(command_words(&server, &super::fleet_listing_args(&server)));
    reads.extend(marker_words("ae-wd!n1!s"));
    let alone = super::motion_exchange_args(&server, "s", "n1", &[]);
    assert!(alone.writes, "no writes leaves nothing behind");
    assert_eq!(alone.args, reads);
    let mut joined = reads;
    joined.push(";".to_owned());
    joined.extend(command_words(
        &server,
        &super::set_options_args(&server, &writes),
    ));
    let query = super::motion_exchange_args(&server, "s", "n1", &writes);
    assert!(query.writes);
    assert_eq!(query.args, joined);
}

#[test]
fn writes_that_would_pass_the_command_bound_stay_behind_the_reads() {
    use super::{COMMAND_BYTES, OptionScope, OptionWrite};
    use crate::inventory::ServerId;
    use crate::meta::Selector;
    let server = ServerId::Selected(Selector::Name("ae".to_owned()));
    let write = |value: &str| [OptionWrite::new(OptionScope::Pane, "%3", "@v", value)];
    let exchange = |value: &str| super::motion_exchange_args(&server, "s", "n1", &write(value));
    let reads = chunk_bytes(
        &server,
        &super::motion_exchange_args(&server, "s", "n1", &[]).args,
    );
    let room = COMMAND_BYTES - chunk_bytes(&server, &exchange("").args);
    let full = exchange(&"x".repeat(room));
    assert!(
        full.writes,
        "a write that reaches the bound exactly still rides"
    );
    assert_eq!(chunk_bytes(&server, &full.args), COMMAND_BYTES);
    let over = exchange(&"x".repeat(room + 1));
    assert!(!over.writes, "one byte more and it stays behind");
    assert_eq!(
        chunk_bytes(&server, &over.args),
        reads,
        "the reads then travel alone"
    );
}

#[test]
fn a_queued_read_hands_each_section_over_byte_for_byte() {
    let sections = ["p".to_owned(), "s".to_owned()];
    let split = |stdout: &str| super::queued_sections(stdout, "N", &sections);
    let some = |section: &str| Some(section.to_owned());
    // tmux prints a value's CR raw; a separate read would hand it on.
    let panes = "%1 | lead\r | 1\n%2 |  | 1\r\n";
    let fleet = "current | $0 | 2 |  | b\r |  | \n";
    let stdout = format!("{panes}ae-wd!N!p\n{fleet}ae-wd!N!s\n");
    assert_eq!(split(&stdout), [some(panes), some(fleet)]);
    assert_eq!(split("ae-wd!N!p\nae-wd!N!s\n"), [some(""), some("")]);
    assert_eq!(
        split("ae-wd!N!p\nae-wd!N!s\nafter\n"),
        [some(""), some("")],
        "lines after the last marker belong to no section"
    );
    assert_eq!(
        split(&format!("{panes}ae-wd!N!p\n{fleet}")),
        [some(panes), None]
    );
    assert_eq!(split(""), [None, None], "nothing ran");
    assert_eq!(split(panes), [None, None]);
    // Marker-like lines without the nonce are lines of their section.
    let forged = "ae-wd!p\nae-wd!M!p\nae-wd:s|x\n";
    assert_eq!(
        split(&format!("{forged}ae-wd!N!p\nae-wd!N!s\n")),
        [some(forged), some("")]
    );
    // A line carrying the nonce that is not the expected marker is framing
    // this read never printed: the read ends there.
    for stray in ["ae-wd!N!p\r", "ae-wd!N!s", "ae-wd!N!", "ae-wd!N!px"] {
        assert_eq!(
            split(&format!("{stray}\nae-wd!N!p\nae-wd!N!s\n")),
            [None, None],
            "{stray:?}"
        );
    }
    assert_eq!(
        split("ae-wd!N!p\nae-wd!N!p\nae-wd!N!s\n"),
        [some(""), None],
        "a repeated marker is not the next section's"
    );
}

#[test]
fn a_motion_exchange_reads_each_section_with_todays_interpreter() {
    let panes = "%1 | lead\r | 1\n%2 |  | 1\n";
    let fleet = "current | $0 | 2 |  |  |  | \nbad row\n";
    let today =
        super::interpret_motion_panes(true, panes).zip(super::interpret_fleet_listing(true, fleet));
    assert!(today.is_some());
    let stdout = format!("{panes}ae-wd!N!p\n{fleet}ae-wd!N!s\n");
    assert_eq!(super::interpret_motion_exchange(&stdout, "N"), today);
    assert_eq!(
        super::interpret_motion_exchange(&format!("{panes}ae-wd!N!p\n{fleet}"), "N"),
        None,
        "a fleet section that did not complete is no fleet"
    );
    assert_eq!(
        super::interpret_motion_exchange(&format!("{panes}x\nae-wd!N!p\n{fleet}ae-wd!N!s\n"), "N"),
        None,
        "a malformed pane row refuses the whole reading, as today"
    );
}

#[test]
fn the_session_look_read_is_todays_reads_each_behind_a_nonce_marker() {
    use crate::inventory::ServerId;
    use crate::meta::Selector;
    let server = ServerId::Selected(Selector::Name("ae".to_owned()));
    let mut expected = super::server_args(&server);
    expected.extend(command_words(&server, &super::look_args(&server, "s")));
    expected.extend(marker_words("ae-wd!n1!l"));
    expected.push(";".to_owned());
    expected.extend(command_words(
        &server,
        &super::session_option_args(&server, "s", "@a"),
    ));
    expected.extend(marker_words("ae-wd!n1!0"));
    expected.push(";".to_owned());
    expected.extend(command_words(
        &server,
        &super::session_option_args(&server, "s", "@b"),
    ));
    expected.extend(marker_words("ae-wd!n1!1"));
    assert_eq!(
        super::session_look_read_args(&server, "s", "n1", &["@a", "@b"]),
        expected
    );
}

#[test]
fn a_session_look_read_answers_as_the_separate_reads_did() {
    use super::{SessionLookRead, interpret_look, interpret_session_look_read};
    let look = "off | b\r | on | off\n";
    let stdout = format!("{look}ae-wd!N!l\nstamp\r\nae-wd!N!0\nae-wd!N!1\n\nae-wd!N!2\n");
    assert_eq!(
        interpret_session_look_read(&stdout, "N", 3),
        SessionLookRead {
            look: interpret_look(true, look),
            options: vec![
                super::interpret_session_option(true, "stamp\r\n"),
                None,
                None
            ],
        },
        "unset prints nothing or an empty line, and both are None, as today"
    );
    let cut = format!("{look}ae-wd!N!l\nstamp\nae-wd!N!0\n");
    assert_eq!(
        interpret_session_look_read(&cut, "N", 3),
        SessionLookRead {
            look: interpret_look(true, look),
            options: vec![Some("stamp".to_owned()), None, None],
        },
        "a read that stopped answers up to its last marker"
    );
    assert_eq!(
        interpret_session_look_read("", "N", 3),
        SessionLookRead {
            look: None,
            options: vec![None; 3],
        }
    );
}

/// Failure and unset are different readings. The conflating `Option`
/// reader stays for read-only show surfaces; a WRITER that treated a
/// failed observation as vacant would overwrite what it could not read.
#[test]
fn interpret_option_reading_distinguishes_failure_from_unset() {
    use super::{OptionReading, interpret_option_reading};
    assert_eq!(
        interpret_option_reading(false, "abc\n"),
        OptionReading::Unknown,
        "a failed run is no reading at all"
    );
    assert_eq!(interpret_option_reading(true, ""), OptionReading::Vacant);
    assert_eq!(interpret_option_reading(true, "\n"), OptionReading::Vacant);
    assert_eq!(
        interpret_option_reading(true, "  \n"),
        OptionReading::Vacant
    );
    assert_eq!(
        interpret_option_reading(true, "abc\n"),
        OptionReading::Set("abc".to_owned())
    );
    // The edge the trim must not collapse: the first line being empty does
    // not make a multiline value absent.
    assert_eq!(
        interpret_option_reading(true, "\nabc\n"),
        OptionReading::Set("abc".to_owned())
    );
    assert_eq!(
        interpret_option_reading(true, "a\nb\n"),
        OptionReading::Set("a\nb".to_owned())
    );
}

/// The UUID write is ONE server-side command: the full incarnation guard
/// and the set travel in the same invocation, so no replacement can
/// interleave between a check and the write.
#[test]
fn the_uuid_guard_and_set_are_one_server_side_command() {
    let args = super::guarded_session_option_args(
        &crate::inventory::ServerId::Ambient,
        &super::ServerIdentity {
            pid: "911".to_owned(),
            start: "1789109000".to_owned(),
        },
        &super::SessionIdentity {
            id: "$7".to_owned(),
            created: "1789109600".to_owned(),
        },
        "@ae_session_uuid",
        "1b4e28ba-2fa1-11d2-883f-0016d3cc4321",
    );
    assert_eq!(&args[..4], ["if-shell", "-F", "-t", "$7"]);
    let guard = &args[4];
    // The EXACT string, not needles: needles pass for a 3-operand shape
    // too, and tmux 3.4 silently drops its third operand (#186).
    assert_eq!(
        guard,
        "#{&&:#{&&:#{==:#{pid},911},#{==:#{start_time},1789109000}},#{&&:#{&&:#{==:#{session_id},$7},#{==:#{session_created},1789109600}},#{==:#{@ae_session_uuid},}}}",
        "the guard nests binary conjunctions: {guard}"
    );
    assert_eq!(
        args[5],
        "set-option -t $7: @ae_session_uuid 1b4e28ba-2fa1-11d2-883f-0016d3cc4321"
    );
    assert_eq!(args.len(), 6, "one invocation carries guard and set");
}

/// The pane-bound capture: one line, id and creation, nothing guessed.
#[test]
fn a_pane_reports_its_sessions_identity_or_nothing() {
    use super::interpret_pane_session_identity;
    let found = interpret_pane_session_identity(
        true,
        "$7 | 1789109600
",
    )
    .expect("the identity");
    assert_eq!(found.id, "$7");
    assert_eq!(found.created, "1789109600");
    assert!(interpret_pane_session_identity(true, "").is_none());
    assert!(
        interpret_pane_session_identity(
            true, "$7 | x
"
        )
        .is_none()
    );
    assert!(
        interpret_pane_session_identity(
            false,
            "$7 | 1789109600
"
        )
        .is_none()
    );
}

#[test]
fn replacing_a_menu_marker_is_one_exact_session_command_queue() {
    assert_eq!(
        super::replace_session_option_args(
            &crate::inventory::ServerId::Ambient,
            "$7",
            "@ae_settings_open",
            "@ae_menu_open",
            "123",
        ),
        [
            "set-option",
            "-u",
            "-t",
            "=$7:",
            "@ae_settings_open",
            ";",
            "set-option",
            "-t",
            "=$7:",
            "@ae_menu_open",
            "123",
        ]
    );
}

#[test]
fn the_transient_alert_carries_the_frozen_duration() {
    use super::display_message_args;
    use crate::inventory::ServerId;
    assert_eq!(
        display_message_args(&ServerId::Ambient, "$3", "[ae watchdog] cl:a is DEAD"),
        [
            "display-message",
            "-d",
            "10000",
            "-t",
            "$3",
            "[ae watchdog] cl:a is DEAD"
        ]
    );
}

#[test]
fn the_window_grouping_read_drops_short_lines_and_reads_an_unstamped_pane_as_none() {
    use super::{WindowPane, interpret_window_panes, window_panes_args};
    use crate::inventory::ServerId;
    assert_eq!(
        window_panes_args(&ServerId::Ambient, "demo"),
        [
            "list-panes",
            "-s",
            "-t",
            "=demo",
            "-F",
            super::WINDOW_PANE_FORMAT
        ]
    );
    let listing = "%1 | @0 | 1 |  |  | 0 | 0 | 0 | cl:lead\n%2 | @0 | 1 | %1 |  | 0 | 0 | 1 | \n%4 | @2 |  |  |  | 0 | 2 | 0 | cl:y\n%5 | @3 |  |  | u | 1 | 3 | 0 | \n%6 | @4 |  |  |  | 0 | x | 0 | \n%3 @1 cl:x\n";
    let panes = interpret_window_panes(true, listing).expect("a successful run");
    assert_eq!(
        panes,
        vec![
            WindowPane {
                pane_id: "%1".to_owned(),
                window_id: "@0".to_owned(),
                theme: "1".to_owned(),
                reader_src: None,
                console: None,
                dead: false,
                window_index: 0,
                pane_index: 0,
                agent: Some("cl:lead".to_owned()),
            },
            // A READER pane: its stamp names the source it shows.
            WindowPane {
                pane_id: "%2".to_owned(),
                window_id: "@0".to_owned(),
                theme: "1".to_owned(),
                reader_src: Some("%1".to_owned()),
                console: None,
                dead: false,
                window_index: 0,
                pane_index: 1,
                agent: None,
            },
            // An UNDRESSED window: the stamp is empty, which is the fact
            // the watchdog restamps on.
            WindowPane {
                pane_id: "%4".to_owned(),
                window_id: "@2".to_owned(),
                theme: String::new(),
                reader_src: None,
                console: None,
                dead: false,
                window_index: 2,
                pane_index: 0,
                agent: Some("cl:y".to_owned()),
            },
            // A CONSOLE pane whose process exited but whose pane remains.
            WindowPane {
                pane_id: "%5".to_owned(),
                window_id: "@3".to_owned(),
                theme: String::new(),
                reader_src: None,
                console: Some("u".to_owned()),
                dead: true,
                window_index: 3,
                pane_index: 0,
                agent: None,
            },
        ],
        "the space-delimited line and a non-numeric index are corruption, not panes"
    );
    assert!(interpret_window_panes(false, listing).is_none());
}

/// The pane-dead read is the PANE-EXISTENCE proof, so its `None` has to
/// mean "no such pane" and nothing else. Measured on a private socket,
/// 2026-09-18: `display-message -p -t %<missing>` EXITS 0 and renders the
/// format EMPTY, which is why a pane that exists must render `0` or `1`
/// and every other reading is `None`.
#[test]
fn a_pane_dead_read_answers_only_for_a_pane_that_exists() {
    use super::interpret_pane_dead;

    assert_eq!(interpret_pane_dead(true, "0\n"), Some(false));
    assert_eq!(interpret_pane_dead(true, "1\n"), Some(true));
    assert_eq!(
        interpret_pane_dead(true, "\n"),
        None,
        "tmux exits 0 and renders nothing for a pane that does not exist"
    );
    assert_eq!(interpret_pane_dead(true, ""), None);
    assert_eq!(
        interpret_pane_dead(true, "#{pane_dead}\n"),
        None,
        "an unexpanded format is not a verdict"
    );
    assert_eq!(
        interpret_pane_dead(false, "0\n"),
        None,
        "a failed command decides nothing, whatever it printed"
    );
}

/// Every format this module hands tmux is read back by splitting on the
/// bytes it asked for, and tmux 3.4 does not hand control characters back
/// unchanged: measured on tmux 3.4 and tmux 3.7b, `\x1f` returns as the
/// four literal bytes `\037`, and to a client tmux does not judge UTF-8 a
/// TAB returns as `_` (#187).
#[test]
fn no_tmux_format_carries_a_control_character() {
    use super::{
        AGENTS_FORMAT, CLIENT_FORMAT, FLEET_PANE_FORMAT, MOTION_PANE_FORMAT,
        MOUSE_DOWN_STATUS_MENU_ACTION, MOUSE_STATUS_PICKER, MOUSE_STATUS_SESSION,
        MOUSE_STATUS_SETTINGS, MOUSE_STATUS_WINDOW, PANE_DEAD_FORMAT, PANE_FORMAT, PANE_ID_FORMAT,
        PANE_PROBE_FORMAT, PANE_TTY_FORMAT, SESSION_ID_FORMAT, SESSION_IDENTITY_FORMAT,
        SESSION_NAME_FORMAT, SLOTS_FORMAT, VERSION_FORMAT, VIEWER_FORMAT, WATCH_PANE_FORMAT,
        WINDOW_PANE_FORMAT,
    };

    for format in [
        AGENTS_FORMAT,
        CLIENT_FORMAT,
        FLEET_PANE_FORMAT,
        MOTION_PANE_FORMAT,
        PANE_DEAD_FORMAT,
        PANE_FORMAT,
        PANE_ID_FORMAT,
        PANE_PROBE_FORMAT,
        PANE_TTY_FORMAT,
        SESSION_IDENTITY_FORMAT,
        SESSION_ID_FORMAT,
        SESSION_NAME_FORMAT,
        SLOTS_FORMAT,
        VERSION_FORMAT,
        VIEWER_FORMAT,
        WATCH_PANE_FORMAT,
        WINDOW_PANE_FORMAT,
        super::FLEET_SESSION_FORMAT,
        super::PICKER_CLIENT_SESSION_FORMAT,
        super::PICKER_PANE_FORMAT,
        super::PICKER_SESSION_FORMAT,
        super::LOOK_FORMAT,
        super::FLEET_LISTING_FORMAT,
    ] {
        assert!(
            !format.chars().any(char::is_control),
            "{format:?} carries a control character, which tmux 3.4 escapes"
        );
    }
    for format in [
        MOUSE_STATUS_PICKER,
        MOUSE_STATUS_SETTINGS,
        MOUSE_STATUS_SESSION,
        MOUSE_STATUS_WINDOW,
        MOUSE_DOWN_STATUS_MENU_ACTION,
    ] {
        assert!(
            !format.chars().any(char::is_control),
            "{format:?} carries a control character, which tmux 3.4 escapes"
        );
    }
}

/// The fleet strip is rendered from rows this reader hands it, straight
/// into an option value the drawer reads styles out of — so a row that is
/// not provably an ae session must never leave this function.
#[test]
fn a_fleet_row_is_proven_before_it_is_admitted() {
    use super::{FleetSession, interpret_fleet_sessions};

    let listing = "\
            good | $1 | 4\n\
            evil#[bg=red] | $2 | 4\n\
            spaced name | $3 | 4\n\
            badid | @4 | 4\n\
            badid2 | $ | 4\n\
            unranked | $5 | \n\
            overranked | $6 | 99\n\
            wordrank | $7 | four\n\
            -leading | $8 | 0\n\
            also-good | $9 | 0\n";
    let read = interpret_fleet_sessions(true, listing).unwrap_or_default();
    assert_eq!(
        read,
        vec![
            FleetSession {
                name: "good".to_owned(),
                id: "$1".to_owned(),
                rank: "4".to_owned(),
            },
            FleetSession {
                name: "also-good".to_owned(),
                id: "$9".to_owned(),
                rank: "0".to_owned(),
            },
        ],
        "only rows whose name, id and rank all check out"
    );
    assert!(interpret_fleet_sessions(false, listing).is_none());
}

/// PIN (lead condition C2): the listing's tail IS the look format.
///
/// The whole reason one `list-sessions` can answer "who is here" and "what
/// is each of them drawn in" at once. Written out at both sites because a
/// format ae cannot read at its use site is a format nobody checks — so the
/// equality is pinned instead of assembled, and a hand edit to either
/// literal reds here rather than silently feeding four fields in the wrong
/// order to the one look parser.
#[test]
fn the_listing_format_ends_in_the_one_look_format() {
    assert!(
        super::FLEET_LISTING_FORMAT.ends_with(super::LOOK_FORMAT),
        "the look tail must be LOOK_FORMAT verbatim: {}",
        super::FLEET_LISTING_FORMAT
    );
    assert!(
        super::FLEET_LISTING_FORMAT.starts_with(super::FLEET_SESSION_FORMAT),
        "and the head must be the strip's own three fields: {}",
        super::FLEET_LISTING_FORMAT
    );
    assert_eq!(
        super::FLEET_LISTING_FORMAT
            .matches(super::FIELD_SEPARATOR)
            .count()
            + 1,
        super::FLEET_LISTING_FIELDS,
        "the field count the parser demands is the one the format yields"
    );
}

/// PIN (lead condition C4 + navigator I5): a row that does not parse
/// EXACTLY is skipped, never shifted into its neighbour.
///
/// Every field here is attacker-reachable: the look options are settable by
/// hand on any session, and a session ae did not create may hold the
/// separator in its NAME. Reading such a row positionally would put a
/// session name in the rank column and a palette in the look column.
#[test]
fn a_listing_row_is_dropped_whole_rather_than_read_misaligned() {
    use super::{FleetListingRow, LookOptions, interpret_fleet_listing};

    let listing = "\
            good | $1 | 4 | off | b | on | off\n\
            unwatched | $2 |  | on | a | on | on\n\
            piped-palette | $3 | 4 | on | a | b | on | on\n\
            pipe | name | $4 | 4 | on | a | on | on\n\
            short | $5 | 4 | on\n\
            overranked | $6 | 99 | on | a | on | on\n\
            wordrank | $7 | four | on | a | on | on\n\
            evil#[bg=red] | $8 | 4 | on | a | on | on\n\
            badid | @9 | 4 | on | a | on | on\n\
            un|spaced | $10 | 4 | on | a | on | on\n";
    let read = interpret_fleet_listing(true, listing).unwrap_or_default();
    assert_eq!(
        read,
        vec![
            FleetListingRow {
                name: "good".to_owned(),
                id: "$1".to_owned(),
                rank: Some("4".to_owned()),
                look: LookOptions {
                    icons: "off".to_owned(),
                    palette: "b".to_owned(),
                    drawn: "on".to_owned(),
                    motion: "off".to_owned(),
                },
            },
            FleetListingRow {
                name: "unwatched".to_owned(),
                id: "$2".to_owned(),
                // The POINT of this reader: no rank is not a bad rank.
                rank: None,
                look: LookOptions {
                    icons: "on".to_owned(),
                    palette: "a".to_owned(),
                    drawn: "on".to_owned(),
                    motion: "on".to_owned(),
                },
            },
        ],
        "a separator inside a value, a short row, an unreadable rank, a \
             styled name and a bad id each drop their whole row"
    );
    assert!(interpret_fleet_listing(false, listing).is_none());
    assert_eq!(interpret_fleet_listing(true, ""), Some(Vec::new()));
}

/// PIN: the two fleet readers differ in EXACTLY one way.
///
/// A rank the three-field reader will not read is a row neither reader
/// admits; a row with NO rank is the one the listing keeps and the strip's
/// own reader drops. Anything else diverging means one of them could draw a
/// session the other proved it should not.
#[test]
fn the_two_fleet_readers_disagree_only_about_a_missing_rank() {
    use super::{interpret_fleet_listing, interpret_fleet_sessions};

    for rank in ["99", "four", "-1", "6"] {
        let three = format!("peer | $1 | {rank}\n");
        let seven = format!("peer | $1 | {rank} | on | a | on | on\n");
        assert_eq!(
            interpret_fleet_sessions(true, &three),
            Some(Vec::new()),
            "the strip reader drops rank {rank:?}"
        );
        assert_eq!(
            interpret_fleet_listing(true, &seven).map(|rows| rows.len()),
            Some(0),
            "and so does the listing reader"
        );
    }
    assert_eq!(
        interpret_fleet_sessions(true, "peer | $1 | \n"),
        Some(Vec::new()),
        "an unranked row is not a row the strip reader can draw"
    );
    assert_eq!(
        interpret_fleet_listing(true, "peer | $1 |  | on | a | on | on\n").map(|rows| rows.len()),
        Some(1),
        "but the listing keeps it, for ae's own records to judge"
    );
}

#[test]
fn picker_rows_admit_identity_default_display_fields_and_keep_the_goal_last() {
    use super::{PickerSession, interpret_picker_sessions};

    let listing = concat!(
        "good | $1 | 4 | ⚠ | %7 | featuremenu | v1;2000;60;lead:fable5:working:%7 | ship | v1;9;tuple:p:done:%8 | keep #[bg=red]",
        "\u{7}",
        " now\n",
        "missing-display | $2 | 0\n",
        "bad name | $3 | 1 | x | %3 | no\n",
        "bad-id | @4 | 1 | x | %4 | no\n",
        "bad-rank | $5 | many | x | %5 | no\n",
        "too-high | $6 | 6 | x | %6 | no\n",
    );
    assert_eq!(
        interpret_picker_sessions(true, listing),
        Some(vec![
            PickerSession {
                name: "good".to_owned(),
                id: "$1".to_owned(),
                rank: 4,
                glyph: "⚠".to_owned(),
                main_pane: "%7".to_owned(),
                branch: "featuremenu".to_owned(),
                agents: "v1;2000;60;lead:fable5:working:%7".to_owned(),
                activity: None,
                created_at: None,
                goal: "ship | v1;9;tuple:p:done:%8 | keep #[bg=red]\u{7} now".to_owned(),
            },
            PickerSession {
                name: "missing-display".to_owned(),
                id: "$2".to_owned(),
                rank: 0,
                glyph: String::new(),
                main_pane: String::new(),
                branch: String::new(),
                agents: String::new(),
                activity: None,
                created_at: None,
                goal: String::new(),
            },
        ])
    );
    assert_eq!(interpret_picker_sessions(true, ""), Some(Vec::new()));
    assert!(interpret_picker_sessions(false, listing).is_none());
    assert!(
        super::PICKER_SESSION_FORMAT.contains("#{s/#{l:[|[:cntrl:]]}//:@ae_branch_name}"),
        "the tmux reader strips delimiters and control bytes before splitting"
    );
    assert!(
        super::PICKER_SESSION_FORMAT.contains("#{s/#{l:[|[:cntrl:]]}/!/:@ae_agents}"),
        "hostile fact delimiters become a byte the strict parser always rejects"
    );
}

/// A client's touch is `session_activity` past `session_created`; equal is
/// tmux's own start value. The two times ride before the goal, only in the
/// ten-piece shape whose 8th and 9th pieces are numbers or empty: a line
/// from before them keeps its goal, a number and pipes in it included.
#[test]
fn a_session_is_touched_only_by_activity_past_its_creation() {
    use super::interpret_picker_sessions;

    let head = "api | $1 | 0 | x | %1 | b | v1;2000;60;lead:p:done:%1";
    let parse = |tail: &str| {
        let rows = interpret_picker_sessions(true, &format!("{head}{tail}\n"));
        let row = rows.and_then(|rows| rows.into_iter().next());
        row.map(|row| (row.touched(), row.activity, row.created_at, row.goal))
    };
    let some = |touched, activity, created, goal: &str| {
        Some((touched, activity, created, goal.to_owned()))
    };
    assert_eq!(
        parse(" | 150 | 100 | a | b"),
        some(Some(150), Some(150), Some(100), "a | b")
    );
    assert_eq!(
        parse(" | 100 | 100 | g"),
        some(None, Some(100), Some(100), "g")
    );
    assert_eq!(
        parse(" | 90 | 100 | g"),
        some(None, Some(90), Some(100), "g")
    );
    assert_eq!(parse(" |  | 100 | g"), some(None, None, Some(100), "g"));
    assert_eq!(parse(" | 150 |  | g"), some(None, Some(150), None, "g"));
    assert_eq!(parse(" | 0 | 0 | g"), some(None, None, None, "g"));
    assert_eq!(
        parse(" | 9x | 100 | g"),
        some(None, None, None, "9x | 100 | g")
    );
    assert_eq!(parse(" | 12345"), some(None, None, None, "12345"));
    assert_eq!(
        parse(" | one | two | three"),
        some(None, None, None, "one | two | three")
    );
}

/// The row a session nobody measures publishes is still a row.
///
/// A watchdog that stops takes back everything it vouched for and puts the
/// launch seed over the three attention options, so both readers then see a
/// Stale rank beside the empty display fields only a live daemon fills.
/// Dropping that row is what used to hide a RUNNING session from every
/// other session's strip and from the picker while `ae list` still called
/// it running.
#[test]
fn an_unwatched_session_keeps_its_row_in_both_fleet_readers() {
    use super::{
        FIELD_SEPARATOR, FleetSession, PickerSession, interpret_fleet_sessions,
        interpret_picker_sessions,
    };
    use crate::theme::Mark;

    let rank = Mark::Stale.rank();
    let glyph = Mark::Stale.glyph(true);
    assert_eq!(
        interpret_fleet_sessions(true, &format!("unwatched | $3 | {rank}\n")),
        Some(vec![FleetSession {
            name: "unwatched".to_owned(),
            id: "$3".to_owned(),
            rank: rank.to_string(),
        }]),
        "a session nobody is measuring still belongs on every strip"
    );

    // The post-stop shape exactly: seeded rank and glyph, and the branch,
    // roster and goal the retraction cleared.
    let row = [
        "unwatched",
        "$3",
        &rank.to_string(),
        glyph,
        "%4",
        "",
        "",
        "",
    ]
    .join(FIELD_SEPARATOR);
    assert_eq!(
        interpret_picker_sessions(true, &format!("{row}\n")),
        Some(vec![PickerSession {
            name: "unwatched".to_owned(),
            id: "$3".to_owned(),
            rank,
            glyph: glyph.to_owned(),
            main_pane: "%4".to_owned(),
            branch: String::new(),
            agents: String::new(),
            activity: None,
            created_at: None,
            goal: String::new(),
        }]),
        "and it stays selectable in the picker"
    );
}

/// One v1 row as the parser yields it: the four admitting fields, and the
/// four observed cells a v1 fact cannot carry.
fn v1_agent(name: &str, profile: &str, state: &str, pane: &str) -> super::PickerAgent {
    super::PickerAgent {
        name: name.to_owned(),
        profile: profile.to_owned(),
        state: state.to_owned(),
        pane: pane.to_owned(),
        client: String::new(),
        model: String::new(),
        effort: String::new(),
        drift: false,
        task: false,
    }
}

#[test]
fn picker_agents_fact_is_typed_bounded_and_all_or_nothing() {
    use super::parse_picker_agents;

    let now = 2_000;
    assert_eq!(
        parse_picker_agents(
            "v1;1900;300;lead:fable5:working:%1;builder:gpt56sol:done:%2",
            now,
        ),
        Some(vec![
            v1_agent("lead", "fable5", "working", "%1"),
            v1_agent("builder", "gpt56sol", "done", "%2"),
        ])
    );
    assert_eq!(
        parse_picker_agents("v1;1880;60;lead:fable5:working:", now),
        Some(vec![v1_agent("lead", "fable5", "working", "")]),
        "exactly two intervals old is still fresh"
    );
    assert_eq!(
        parse_picker_agents("v1;1879;60;lead:fable5:working:%1", now),
        None,
        "one second past two intervals is stale"
    );
    assert!(
        parse_picker_agents("v1;1900;300;lead:fable5:idle:%1", now).is_some(),
        "a 300-second publisher remains fresh after 100 seconds"
    );
    assert!(
        parse_picker_agents("v1;2000;1;lead:fable5:busy:%1", now).is_some(),
        "harness-observed busy is a valid live state"
    );
    assert!(
        parse_picker_agents("v1;2000;60;lead:fable5:waiting-agent:%1", now).is_some(),
        "the fifth declared state is a valid live state"
    );
    assert_eq!(
        parse_picker_agents("v1;1997;1;lead:fable5:busy:%1", now),
        None,
        "a one-second publisher is stale after three seconds"
    );
    assert!(
        parse_picker_agents("v1;2060;60;lead:fable5:working:%1", now).is_some(),
        "one interval of future clock skew stays usable"
    );
    assert_eq!(
        parse_picker_agents("v1;2061;60;lead:fable5:working:%1", now),
        None,
        "one second beyond the future-skew allowance is stale"
    );
    assert_eq!(
        parse_picker_agents("v1;9223372036854775807;3600;lead:fable5:working:%1", now,),
        None,
        "a hostile far-future epoch never stays trusted"
    );

    for invalid in [
        "",
        "v1;2000",
        "v1;2000;0;lead:fable5:working:%1",
        "v1;2000;3601;lead:fable5:working:%1",
        "v1;2000;soon;lead:fable5:working:%1",
        "v2;2000;60;lead:fable5:working:%1",
        "v1;old;60;lead:fable5:working:%1",
        "v1;2000;60;bad name:fable5:working:%1",
        "v1;2000;60;lead:bad profile:working:%1",
        "v1;2000;60;lead:fablex@cc-mic:working:%1",
        "v1;2000;60;lead:fable5:unknown:%1",
        "v1;2000;60;lead:fable5:working:pane",
        "v1;2000;60;lead:fable5:working:%1:extra",
        "v1;2000;60;lead:fable5:working:%1;lead:fable5:done:%2",
        "v1;2000;60;lead:fable5:working:%1|shift",
        "v1;2000;60;lead:fable5:working:%1,shift",
        "v1;2000;60;lead:fable5:working:%1!shift",
        "v1;2000;60;lead:fable5:working:%1\n",
        "v1;2000;60;lead:fable5:working:%1#[fg=red]",
    ] {
        assert_eq!(parse_picker_agents(invalid, now), None, "{invalid}");
    }
    let too_many = format!(
        "v1;2000;60;{}",
        (0..65)
            .map(|index| format!("a{index}:p:idle:"))
            .collect::<Vec<_>>()
            .join(";")
    );
    assert_eq!(parse_picker_agents(&too_many, now), None);
    assert_eq!(parse_picker_agents(&"x".repeat(4_097), now), None);
}

#[test]
fn a_v2_entry_carries_client_model_effort_and_drift() {
    use super::{PickerAgent, parse_picker_agents};

    let now = 2_000;
    assert_eq!(
        parse_picker_agents(
            "v2;2000;60;lead:fable5:working:%1:cc:Fable 5.1:xhigh:!;\
                 nav:spark13m:idle:%2:muse:muse-spark-1.3:max:;\
                 gone:gpt56sol:dead:::::",
            now,
        ),
        Some(vec![
            PickerAgent {
                name: "lead".to_owned(),
                profile: "fable5".to_owned(),
                state: "working".to_owned(),
                pane: "%1".to_owned(),
                client: "cc".to_owned(),
                model: "Fable 5.1".to_owned(),
                effort: "xhigh".to_owned(),
                drift: true,
                task: false,
            },
            PickerAgent {
                name: "nav".to_owned(),
                profile: "spark13m".to_owned(),
                state: "idle".to_owned(),
                pane: "%2".to_owned(),
                client: "muse".to_owned(),
                model: "muse-spark-1.3".to_owned(),
                effort: "max".to_owned(),
                drift: false,
                task: false,
            },
            // A seat with no pane keeps its v1 shape and observes nothing:
            // there was no frame to read.
            v1_agent("gone", "gpt56sol", "dead", ""),
        ]),
        "a model may carry spaces; only the separators and the style bytes \
             are forbidden"
    );
    // An unobserved seat on a v2 fact still names its client, which ae
    // knows from the recorded binary and never guesses.
    assert_eq!(
        parse_picker_agents("v2;2000;60;lead:fable5:working:%1:cx:::", now)
            .as_deref()
            .and_then(<[PickerAgent]>::first)
            .map(|agent| (agent.client.clone(), agent.model.clone())),
        Some(("cx".to_owned(), String::new()))
    );
}

#[test]
fn a_v2_client_cell_carries_a_full_label_or_a_legacy_token() {
    use super::parse_picker_agents;

    let now = 2_000;
    let fact = |client: &str| format!("v2;2000;60;lead:p:working:%1:{client}:M::");
    let client_of = |fact: &str| {
        parse_picker_agents(fact, now)
            .and_then(|agents| agents.into_iter().next().map(|agent| agent.client))
    };
    // Full labels ride, 32 cells included; legacy tokens still parse, so
    // a mixed-version fleet renders old rows rather than dropping them.
    for client in ["cc-mic", "claude", &"l".repeat(32)] {
        assert_eq!(
            client_of(&fact(client)),
            Some(client.to_owned()),
            "{client:?}"
        );
    }
    for token in ["cc", "cx", "gem", "agy", "grok", "muse", "oc", "-"] {
        assert_eq!(client_of(&fact(token)), Some(token.to_owned()), "{token:?}");
    }
    for invalid in [
        fact(&"l".repeat(33)),
        fact("cc mic"),
        fact("-x"),
        fact("_x"),
    ] {
        assert_eq!(parse_picker_agents(&invalid, now), None, "{invalid}");
    }
}

#[test]
fn a_v1_fact_still_parses_and_leaves_the_new_cells_empty() {
    use super::parse_picker_agents;

    // Versions run side by side: an upgrade cannot reach every live
    // watchdog at once, so the reader keeps the old grammar.
    assert_eq!(
        parse_picker_agents("v1;2000;60;lead:fable5:working:%1", 2_000),
        Some(vec![v1_agent("lead", "fable5", "working", "%1")])
    );
    // The same roster in each grammar differs ONLY in the observed cells.
    let v1 =
        parse_picker_agents("v1;2000;60;lead:fable5:working:%1", 2_000).expect("the v1 roster");
    let v2 = parse_picker_agents("v2;2000;60;lead:fable5:working:%1:cc:Opus 5:xhigh:", 2_000)
        .expect("the v2 roster");
    assert_eq!(v1[0].name, v2[0].name);
    assert_eq!(v1[0].profile, v2[0].profile);
    assert_eq!(v1[0].state, v2[0].state);
    assert_eq!(v1[0].pane, v2[0].pane);
    assert_eq!(v1[0].mark(), v2[0].mark());
    assert_eq!((v1[0].model.as_str(), v2[0].model.as_str()), ("", "Opus 5"));
}

#[test]
fn the_v2_parser_refuses_an_over_cap_unknown_or_incoherent_cell() {
    use super::{PICKER_AGENTS_MAX_MODEL, parse_picker_agents};

    let now = 2_000;
    let entry = |model: &str| format!("v2;2000;60;lead:fable5:working:%1:cc:{model}:xhigh:");
    let at_cap = "M".repeat(PICKER_AGENTS_MAX_MODEL);
    assert!(
        parse_picker_agents(&entry(&at_cap), now).is_some(),
        "exactly at the cap is representable"
    );
    assert_eq!(
        parse_picker_agents(&entry(&"M".repeat(PICKER_AGENTS_MAX_MODEL + 1)), now),
        None,
        "one byte past the cap refuses the whole roster"
    );
    for invalid in [
        // Arity: each grammar's entry has EXACTLY its own field count.
        "v2;2000;60;lead:fable5:working:%1:cc:Opus 5:xhigh",
        "v2;2000;60;lead:fable5:working:%1:cc:Opus 5:xhigh::",
        "v1;2000;60;lead:fable5:working:%1:cc:Opus 5:xhigh:",
        // A version ae has never written.
        "v3;2000;60;lead:fable5:working:%1:cc:Opus 5:xhigh:",
        // Client: a legacy adapter token or an agent-shaped label of at
        // most 32 cells — anything else refuses the whole snapshot.
        "v2;2000;60;lead:fable5:working:%1:cc mic:Opus 5:xhigh:",
        "v2;2000;60;lead:fable5:working:%1:-x:Opus 5:xhigh:",
        "v2;2000;60;lead:fable5:working:%1:_x:Opus 5:xhigh:",
        // Effort: the closed vocabulary, not a shape.
        "v2;2000;60;lead:fable5:working:%1:cc:Opus 5:turbo:",
        "v2;2000;60;lead:fable5:working:%1:cc:Opus 5:XHIGH:",
        // Drift: one spelling.
        "v2;2000;60;lead:fable5:working:%1:cc:Opus 5:xhigh:yes",
        "v2;2000;60;lead:fable5:working:%1:cc:Opus 5:xhigh:!!",
        // Coherence: the observed cells are a unit the writer fills or
        // empties together.
        "v2;2000;60;lead:fable5:working:%1::Opus 5:xhigh:",
        "v2;2000;60;lead:fable5:working:%1:cc::xhigh:",
        "v2;2000;60;lead:fable5:working:%1:cc:::!",
        // A model is text, so its edges are pinned where a separator is not
        // available to do it.
        "v2;2000;60;lead:fable5:working:%1:cc: Opus 5:xhigh:",
        "v2;2000;60;lead:fable5:working:%1:cc:Opus 5 :xhigh:",
        // The v1 refusals are unchanged under the new grammar.
        "v2;2000;60;bad name:fable5:working:%1:cc:Opus 5:xhigh:",
        "v2;2000;60;lead:fable5:unknown:%1:cc:Opus 5:xhigh:",
        "v2;2000;60;lead:fable5:working:pane:cc:Opus 5:xhigh:",
        "v2;2000;60;lead:fable5:working:%1:cc:Opus|5:xhigh:",
        "v2;2000;60;lead:fable5:working:%1:cc:Opus,5:xhigh:",
        "v2;2000;60;lead:fable5:working:%1:cc:Opus#5:xhigh:",
    ] {
        assert_eq!(parse_picker_agents(invalid, now), None, "{invalid}");
    }
}

#[test]
fn every_daemon_verdict_word_has_a_picker_mark() {
    // The daemon publishes `Verdict::reason()` into `@ae_agents`; one
    // unmapped word refuses the whole roster. Enumerate the verdicts
    // THEMSELVES so the next word cannot repeat this.
    use crate::watchdog::{QuietKind, SweepVerdict};
    use crate::watchdog_daemon::Verdict;
    for verdict in [
        Verdict::Dead,
        Verdict::Quiet(QuietKind::Done),
        Verdict::Quiet(QuietKind::WaitingUser),
        Verdict::Quiet(QuietKind::WaitingAgent),
        Verdict::Quiet(QuietKind::Blocked),
        Verdict::Throttled,
        Verdict::Limit,
        Verdict::HumanPrompt,
        Verdict::Idle,
        Verdict::Stale,
        Verdict::Unreachable,
        Verdict::Active,
        Verdict::Meta(SweepVerdict::MetaSweeping),
        Verdict::Meta(SweepVerdict::MetaWedged),
        Verdict::Meta(SweepVerdict::MetaStarting),
    ] {
        // Beside the list: an exhaustive match with NO wildcard. A new
        // `Verdict` variant — or a new `QuietKind` or `SweepVerdict` —
        // fails to compile here, forcing its word into the list above.
        match verdict {
            Verdict::Dead
            | Verdict::Quiet(
                QuietKind::Done
                | QuietKind::WaitingUser
                | QuietKind::WaitingAgent
                | QuietKind::Blocked,
            )
            | Verdict::Throttled
            | Verdict::Limit
            | Verdict::HumanPrompt
            | Verdict::Idle
            | Verdict::Stale
            | Verdict::Unreachable
            | Verdict::Active
            | Verdict::Meta(
                SweepVerdict::MetaSweeping | SweepVerdict::MetaWedged | SweepVerdict::MetaStarting,
            ) => {}
        }
        let word = verdict.reason();
        assert_eq!(
            super::picker_agent_mark(word),
            Some(verdict.mark()),
            "{word:?} must map to the verdict's own mark"
        );
    }
}

#[test]
fn picker_agents_fuzz_seeds_reach_their_named_parser_paths() {
    let valid = include_str!("../../fuzz/seeds/picker_agents/valid");
    let empty_pane = include_str!("../../fuzz/seeds/picker_agents/empty-pane");
    let duplicate = include_str!("../../fuzz/seeds/picker_agents/duplicate");
    let style_byte = include_str!("../../fuzz/seeds/picker_agents/style-byte");
    let limit = include_str!("../../fuzz/seeds/picker_agents/limit");
    for seed in [valid, empty_pane, duplicate, style_byte, limit] {
        assert!(
            seed.bytes().all(|byte| (b' '..=b'~').contains(&byte)),
            "seed must reach its named parser branch"
        );
    }
    assert!(super::parse_picker_agents(valid, 2_000_000_000).is_some());
    assert!(
        super::parse_picker_agents(empty_pane, 2_000_000_000)
            .is_some_and(|agents| agents[0].pane.is_empty())
    );
    assert_eq!(duplicate.matches(";lead:").count(), 2);
    assert_eq!(super::parse_picker_agents(duplicate, 2_000_000_000), None);
    assert!(style_byte.contains('#'));
    assert_eq!(super::parse_picker_agents(style_byte, 2_000_000_000), None);
    assert!(
        super::parse_picker_agents(limit, 2_000_000_000)
            .is_some_and(|agents| agents[0].state == "limit" && agents[0].pane == "%1"),
        "the usage-limit verdict word reaches its parser path"
    );
}

/// The v2 corpus, which must land WITH the writer that first emits v2:
/// the grammar is hostile persisted state and the lane is the evidence
/// that its new fields cannot crash the parser.
#[test]
fn v2_picker_agents_fuzz_seeds_reach_their_named_parser_paths() {
    let valid = include_str!("../../fuzz/seeds/picker_agents/v2-valid");
    let unobserved = include_str!("../../fuzz/seeds/picker_agents/v2-unobserved");
    let over_cap = include_str!("../../fuzz/seeds/picker_agents/v2-over-cap-model");
    let separator = include_str!("../../fuzz/seeds/picker_agents/v2-separator-in-model");
    let limit = include_str!("../../fuzz/seeds/picker_agents/v2-limit");
    let label = include_str!("../../fuzz/seeds/picker_agents/v2-label");
    let bad_label = include_str!("../../fuzz/seeds/picker_agents/v2-bad-label");
    for seed in [
        valid, unobserved, over_cap, separator, limit, label, bad_label,
    ] {
        assert!(
            seed.bytes().all(|byte| (b' '..=b'~').contains(&byte)),
            "seed must reach its named parser branch"
        );
        assert!(seed.starts_with("v2;"), "{seed}");
    }
    // Two observed seats, one with a drift mark and one without.
    let agents = super::parse_picker_agents(valid, 2_000_000_000).expect("the v2 corpus seed");
    assert_eq!(agents.len(), 2);
    assert_eq!(
        (
            agents[0].client.as_str(),
            agents[0].model.as_str(),
            agents[0].drift
        ),
        ("cc", "Fable 5.1", true)
    );
    assert_eq!(agents[1].effort, "max");
    // A seat whose frame proved nothing still names its client.
    assert!(
        super::parse_picker_agents(unobserved, 2_000_000_000)
            .is_some_and(|agents| agents[0].client == "cc" && agents[0].model.is_empty())
    );
    // One byte past the cap, and a model carrying the field separator:
    // both reach the refusal, neither is escaped into something valid.
    assert_eq!(
        over_cap
            .rsplit(':')
            .nth(2)
            .map(str::len)
            .expect("the seed's model field"),
        super::PICKER_AGENTS_MAX_MODEL + 1
    );
    assert_eq!(super::parse_picker_agents(over_cap, 2_000_000_000), None);
    assert_eq!(super::parse_picker_agents(separator, 2_000_000_000), None);
    // The 64-seat bound, spelled in the widest grammar.
    assert!(
        super::parse_picker_agents(limit, 2_000_000_000)
            .is_some_and(|agents| agents.len() == super::PICKER_AGENTS_MAX_COUNT)
    );
    assert!(limit.len() <= super::PICKER_AGENTS_MAX_BYTES);
    // The full-label client and the legacy `-` beside it; a label with a
    // space reaches the shape refusal instead.
    let labeled = super::parse_picker_agents(label, 2_000_000_000).expect("label seed");
    assert_eq!(
        (labeled[0].client.as_str(), labeled[1].client.as_str()),
        ("cc-mic", "-")
    );
    assert_eq!(super::parse_picker_agents(bad_label, 2_000_000_000), None);
}

#[test]
fn picker_pane_membership_keeps_only_tmux_ids() {
    use super::{PickerPane, interpret_picker_panes};

    let listing = "$1 | %7\r\n$2 | %8\nname | %9\n$3 | pane\nshort\n";
    assert_eq!(
        interpret_picker_panes(true, listing),
        Some(vec![
            PickerPane {
                session_id: "$1".to_owned(),
                pane: "%7".to_owned(),
            },
            PickerPane {
                session_id: "$2".to_owned(),
                pane: "%8".to_owned(),
            },
        ])
    );
    assert!(interpret_picker_panes(false, listing).is_none());
}

#[test]
fn the_watchdog_format_is_pinned_to_its_printable_separator() {
    use super::{WATCH_PANE_FORMAT, WATCH_PANE_SEPARATOR};

    let reconstructed = [
        "#{pane_id}",
        WATCH_PANE_SEPARATOR,
        "#{@ae_slot}",
        WATCH_PANE_SEPARATOR,
        "#{@ae_agent}",
        WATCH_PANE_SEPARATOR,
        "#{pane_pid}",
        WATCH_PANE_SEPARATOR,
        "#{@ae_observed}",
        WATCH_PANE_SEPARATOR,
        "#{pane_current_command}",
    ]
    .concat();
    assert_eq!(WATCH_PANE_FORMAT, reconstructed);
    assert!(
        !WATCH_PANE_FORMAT
            .chars()
            .any(|character| character.is_ascii_control())
    );
    assert_eq!(WATCH_PANE_FORMAT.matches(WATCH_PANE_SEPARATOR).count(), 5);
}

#[test]
fn the_watchdog_pane_reading_widens_the_enumeration_and_refuses_malformed_lines() {
    use super::{
        WATCH_PANE_FORMAT, WatchPane, capture_pane_args, interpret_watch_panes, watch_panes_args,
    };
    use crate::inventory::ServerId;
    assert_eq!(
        watch_panes_args(&ServerId::Ambient, "s"),
        ["list-panes", "-s", "-t", "=s", "-F", WATCH_PANE_FORMAT]
    );
    assert_eq!(
        capture_pane_args(&ServerId::Ambient, "%3"),
        [
            "capture-pane",
            "-p",
            "-J",
            "-S",
            "-40",
            "-E",
            "-",
            "-t",
            "%3"
        ]
    );
    let sep = super::WATCH_PANE_SEPARATOR;
    // A well-formed pane and a pane whose pid tmux could not print -> None
    // (never a guessed dead).
    let out = format!(
        "%1{sep}main{sep}cl:lead{sep}9{sep}busy{sep}claude\n%2{sep}{sep}{sep}{sep}{sep}zsh\n"
    );
    let panes = interpret_watch_panes(true, &out).expect("a successful enumeration");
    assert_eq!(panes.len(), 2);
    assert_eq!(panes[0].pane_pid, Some(9));
    assert_eq!(
        panes[1],
        WatchPane {
            pane_id: "%2".into(),
            slot: None,
            agent: None,
            current_command: "zsh".into(),
            pane_pid: None,
            observed: String::new(),
        }
    );
    assert!(
        interpret_watch_panes(true, "").unwrap().is_empty(),
        "no panes is an empty enumeration, not None"
    );
    assert!(
        interpret_watch_panes(true, "%bad | main").is_none(),
        "a line that cannot split is a failure of the whole reading"
    );
    assert!(
        interpret_watch_panes(true, "\n").is_none(),
        "non-empty output with no parseable lines is untrusted"
    );
}

#[test]
fn the_motion_reading_is_one_strict_printable_snapshot() {
    use super::{MotionPane, interpret_motion_panes, motion_panes_args};
    use crate::inventory::ServerId;
    assert_eq!(
        motion_panes_args(&ServerId::Ambient, "s"),
        [
            "list-panes",
            "-s",
            "-t",
            "=s",
            "-F",
            super::MOTION_PANE_FORMAT,
        ]
    );
    let listing = "%1 | lead | 1\n%2 |  | 1\n";
    assert_eq!(
        interpret_motion_panes(true, listing),
        Some(vec![
            MotionPane {
                pane_id: "%1".to_owned(),
                agent: Some("lead".to_owned()),
                session_attached: 1,
            },
            MotionPane {
                pane_id: "%2".to_owned(),
                agent: None,
                session_attached: 1,
            },
        ])
    );
    assert!(interpret_motion_panes(false, listing).is_none());
    assert!(interpret_motion_panes(true, "%1 | lead | bad\n").is_none());
    assert!(interpret_motion_panes(true, "%1 | lead\n").is_none());
}

#[test]
fn the_watchdog_fixtures_preserve_tmux_version_framing() {
    use super::{WatchPane, interpret_watch_panes};

    // Literal output captured from tmux 3.4 with the printable separator.
    let tmux_3_4 = "%0 |  | cl:lead | 1234 |  | fish\n";
    assert_eq!(
        interpret_watch_panes(true, tmux_3_4),
        Some(vec![WatchPane {
            pane_id: "%0".into(),
            slot: None,
            agent: Some("cl:lead".into()),
            current_command: "fish".into(),
            pane_pid: Some(1234),
            observed: String::new(),
        }])
    );

    // Literal output captured from tmux 3.7b with the printable separator.
    let tmux_3_7b = "%1 | main | cl:lead | 4321 | idle:1:0:0:2 | claude\n";
    assert_eq!(
        interpret_watch_panes(true, tmux_3_7b),
        Some(vec![WatchPane {
            pane_id: "%1".into(),
            slot: Some("main".into()),
            agent: Some("cl:lead".into()),
            current_command: "claude".into(),
            pane_pid: Some(4321),
            observed: "idle:1:0:0:2".into(),
        }])
    );

    // The old tmux 3.4 control separator remains untrusted; the parser
    // must not normalize escaped producer output into data.
    let escaped_old = "%0\\037\\037cl:lead\\037fish\\0371234\n";
    assert!(interpret_watch_panes(true, escaped_old).is_none());

    let command_with_separator = "%2 | worker | cl:helper | 77 | busy | tool | with separator\n";
    let panes = interpret_watch_panes(true, command_with_separator).expect("valid reading");
    assert_eq!(panes[0].current_command, "tool | with separator");
}

#[test]
fn the_resolver_queries_are_the_frozen_ones() {
    use super::{
        AGENTS_FORMAT, SLOTS_FORMAT, agents_args, has_session_args, kill_session_args,
        list_panes_args, session_target, slots_args,
    };
    use crate::inventory::ServerId;
    assert_eq!(session_target("dotfiles"), "=dotfiles");
    assert_eq!(
        has_session_args(&ServerId::Ambient, "dotfiles"),
        ["has-session", "-t", "=dotfiles"]
    );
    assert_eq!(
        kill_session_args(&ServerId::Ambient, "dotfiles"),
        ["kill-session", "-t", "=dotfiles"]
    );
    assert_eq!(
        list_panes_args(&ServerId::Ambient, "dotfiles"),
        [
            "list-panes",
            "-s",
            "-t",
            "=dotfiles",
            "-F",
            super::PANE_FORMAT
        ]
    );
    assert_eq!(
        agents_args(&ServerId::Ambient, "s"),
        ["list-panes", "-s", "-t", "=s", "-F", AGENTS_FORMAT]
    );
    assert_eq!(
        slots_args(&ServerId::Ambient, "s"),
        ["list-panes", "-s", "-t", "=s", "-F", SLOTS_FORMAT]
    );
    assert_eq!(SLOTS_FORMAT, "#{pane_id}|#{@ae_slot}|#{@ae_agent}");
}

#[test]
fn the_viewer_query_addresses_the_pane_and_asks_for_the_five_readings() {
    use super::{ObservedViewer, OptionReading, VIEWER_FORMAT, interpret_viewer, viewer_args};
    use crate::inventory::ServerId;
    use crate::theme::SESSION_ID_OPTION;
    assert_eq!(
        viewer_args(&ServerId::Ambient, "%7"),
        ["display-message", "-p", "-t", "%7", VIEWER_FORMAT]
    );
    assert!(
        VIEWER_FORMAT.contains(SESSION_ID_OPTION),
        "the incarnation fact rides the same pane-keyed query as the names"
    );
    assert!(
        VIEWER_FORMAT.contains("#{socket_path}"),
        "the server fact rides the same pane-keyed query, not a second observe"
    );
    let uuid = "1b4e28ba-2fa1-11d2-883f-0016d3cc4321";
    assert_eq!(
        interpret_viewer(
            true,
            &format!("main | aerewrite | cl:lead | {uuid} | /tmp/tmux-501/default\n")
        ),
        Some(ObservedViewer {
            slot: Some("main".to_owned()),
            session: Some("aerewrite".to_owned()),
            agent: Some("cl:lead".to_owned()),
            session_uuid: OptionReading::Set(uuid.to_owned()),
            socket_path: Some("/tmp/tmux-501/default".to_owned()),
        })
    );
    // An unstamped pane: unset options expand to empty. Names become None;
    // an empty uuid field is Vacant, never Unknown — the query answered.
    assert_eq!(
        interpret_viewer(true, " | aerewrite |  |  | /tmp/s\n"),
        Some(ObservedViewer {
            slot: None,
            session: Some("aerewrite".to_owned()),
            agent: None,
            session_uuid: OptionReading::Vacant,
            socket_path: Some("/tmp/s".to_owned()),
        })
    );
    assert_eq!(interpret_viewer(false, "main | s | a:b | u | /s\n"), None);
    assert_eq!(
        interpret_viewer(
            true,
            "main | s | a:b | u | /s\nworker.0 | s | a:c | u | /s\n"
        ),
        None
    );
    assert_eq!(interpret_viewer(true, "main | s | a:b | u | /s\n\n"), None);
    assert_eq!(interpret_viewer(true, "main | s | a:b | u | /s\nx"), None);
    assert_eq!(
        interpret_viewer(true, "main | s | a:b | u | /s"),
        interpret_viewer(true, "main | s | a:b | u | /s\n")
    );
    assert_eq!(interpret_viewer(true, "main | s | a:b | u\n"), None);
    assert_eq!(interpret_viewer(true, ""), None);
    let vacant = interpret_viewer(true, "main | s | a:b |  | /s\n");
    assert_eq!(
        vacant.as_ref().map(|v| &v.session_uuid),
        Some(&OptionReading::Vacant)
    );
    // Alias spelling: the recorded server is the format's socket_path, not
    // the selector used to address the query.
    let alias = interpret_viewer(true, &format!("main | s | a | {uuid} | /tmp/alias\n"));
    let real = interpret_viewer(true, &format!("main | s | a | {uuid} | /tmp/real\n"));
    assert_eq!(
        alias.as_ref().and_then(|v| v.socket_path.as_deref()),
        Some("/tmp/alias")
    );
    assert_eq!(
        real.as_ref().and_then(|v| v.socket_path.as_deref()),
        Some("/tmp/real")
    );
    assert_ne!(
        alias.as_ref().and_then(|v| v.socket_path.as_deref()),
        real.as_ref().and_then(|v| v.socket_path.as_deref()),
        "two spellings stay two facts; tmux's returned path is what we record"
    );
}

#[test]
fn a_socket_path_containing_the_separator_does_not_cut_the_viewer() {
    use super::{ObservedViewer, OptionReading, interpret_viewer};
    // The LAST field keeps every separator it contains: a valid absolute
    // socket path may hold one, and cutting at it would reject the whole
    // viewer for a legitimate server spelling.
    assert_eq!(
        interpret_viewer(true, "main | s | a:b | u | /tmp/ae | sock\n"),
        Some(ObservedViewer {
            slot: Some("main".to_owned()),
            session: Some("s".to_owned()),
            agent: Some("a:b".to_owned()),
            session_uuid: OptionReading::Set("u".to_owned()),
            socket_path: Some("/tmp/ae | sock".to_owned()),
        })
    );
}

use super::{
    Absence, Evidence, ObservedPane, SlotObservation, StopProbe, classify_absence,
    environment_value_args, interpret_environment_value, interpret_marker, interpret_panes,
    interpret_sessions, interpret_stopped, is_addressable_socket, list_panes_args,
    list_sessions_args, marker_args, read_absence, server_args, slot_observation, unproven_reason,
};
use crate::inventory::{QueryFailed, ServerId};
use crate::meta::Selector;
use std::path::{Path, PathBuf};

fn named(name: &str) -> ServerId {
    ServerId::Selected(Selector::Name(name.to_owned()))
}

fn socket(path: &str) -> ServerId {
    ServerId::Selected(Selector::Socket(PathBuf::from(path)))
}

#[test]
fn the_typed_selector_chooses_the_tmux_routing_flag() {
    assert_eq!(server_args(&ServerId::Ambient), Vec::<String>::new());
    assert_eq!(server_args(&named("work")), ["-L", "work"]);
    assert_eq!(server_args(&socket("/tmp/ae.sock")), ["-S", "/tmp/ae.sock"]);
}

#[test]
fn a_name_and_a_socket_of_the_same_spelling_address_different_servers() {
    // -L /tmp/x and -S /tmp/x are not the same tmux.
    assert_ne!(
        server_args(&named("/tmp/x")),
        server_args(&socket("/tmp/x"))
    );
}

#[test]
fn enumeration_asks_for_exact_names_and_nothing_else() {
    assert_eq!(
        list_sessions_args(&named("work")),
        ["-L", "work", "list-sessions", "-F", "#{session_name}"]
    );
    assert_eq!(
        list_sessions_args(&ServerId::Ambient),
        ["list-sessions", "-F", "#{session_name}"]
    );
}

#[test]
fn the_marker_is_read_from_the_session_s_own_environment() {
    assert_eq!(
        marker_args(&socket("/tmp/ae.sock"), "my-feature"),
        [
            "-S",
            "/tmp/ae.sock",
            "show-environment",
            "-t",
            "=my-feature",
            "AE_SESSION"
        ]
    );
}

#[test]
fn an_ownership_value_is_one_exact_nonempty_environment_record() {
    assert_eq!(
        environment_value_args(&named("default"), "old", "AE_HOME"),
        ["-L", "default", "show-environment", "-t", "=old", "AE_HOME"]
    );
    assert_eq!(
        interpret_environment_value(true, "AE_HOME=/tmp/state\n", "AE_HOME"),
        Some("/tmp/state".to_owned())
    );
    for output in [
        "",
        "-AE_HOME\n",
        "AE_HOME=\n",
        "OTHER=/tmp/state\n",
        "AE_HOME=/tmp/state\ntrailing\n",
    ] {
        assert_eq!(
            interpret_environment_value(true, output, "AE_HOME"),
            None,
            "{output:?}"
        );
    }
    assert_eq!(
        interpret_environment_value(false, "AE_HOME=/tmp/state\n", "AE_HOME"),
        None
    );
}

#[test]
fn a_failed_run_is_a_failure_whatever_it_printed() {
    // The bytes are identical in both arms; only the transport result moves.
    for payload in ["", "alpha\nbeta\n", "no server running on /tmp/x\n"] {
        assert_eq!(
            interpret_sessions(false, payload),
            Err(QueryFailed),
            "{payload:?}"
        );
    }
}

#[test]
fn stop_probe_reads_presence_from_a_successful_listing() {
    // A SUCCESSFUL run is authoritative both ways.
    assert_eq!(
        interpret_stopped(true, "other\nsess\n", "", "sess"),
        StopProbe::Present
    );
    assert_eq!(
        interpret_stopped(true, "other\n", "", "sess"),
        StopProbe::Absent
    );
    assert_eq!(
        interpret_stopped(true, "", "", "sess"),
        StopProbe::Absent,
        "an empty SUCCESS proves the session is gone"
    );
    // Exact-name only: `sess` is NOT present in a list of `session` — tmux would
    // prefix-match, ae never does (the same guard `interpret_sessions` earns).
    assert_eq!(
        interpret_stopped(true, "session\n", "", "sess"),
        StopProbe::Absent,
        "a longer name that merely starts with the target is not the target"
    );
}

#[test]
fn stop_probe_reads_a_clean_server_exit_as_absent() {
    // Killing the last session exits the server; the stale-socket diagnostic
    // is the PROOF the session is gone — the whole reason this classifier
    // exists apart from interpret_sessions, which calls the same bytes a failure.
    assert_eq!(
        interpret_stopped(false, "", "no server running on /tmp/x\n", "sess"),
        StopProbe::Absent
    );
}

#[test]
fn stop_probe_reads_any_other_failure_as_unknown_never_absent() {
    // ENOENT / permission / refused prove nothing — a live server whose socket
    // was unlinked yields a connect error while it keeps running (11th-review B1).
    for stderr in [
        "",
        "error connecting to /tmp/x (No such file or directory)\n",
        "error connecting to /tmp/x (Permission denied)\n",
    ] {
        assert_eq!(
            interpret_stopped(false, "", stderr, "sess"),
            StopProbe::Unknown,
            "{stderr:?}"
        );
    }
}

#[test]
fn stop_probe_anchors_clean_dead_at_the_line_start_never_a_substring() {
    // A server literally NAMED "no server running" once made a permission
    // error CONTAIN the words (10th-review B2).
    assert_eq!(
        interpret_stopped(
            false,
            "",
            "error: cannot reach the no server running on host\n",
            "sess"
        ),
        StopProbe::Unknown
    );
}

/// Boot, and a `last_live` on each side of it, as epoch seconds.
const BOOT: i64 = 1_789_105_855;
const NOW: i64 = 1_789_120_000;

#[test]
fn the_absence_reader_separates_a_missing_socket_from_every_other_failure() {
    // The WHOLE point of the second reader: `interpret_stopped` folds ENOENT
    // and permission-denied into one `Unknown`, and the boot-time proof may
    // reason about exactly one of them.
    assert_eq!(
        read_absence(true, "other\nsess\n", "", "sess"),
        Absence::Listed
    );
    assert_eq!(read_absence(true, "other\n", "", "sess"), Absence::Proven);
    assert_eq!(
        read_absence(false, "", "no server running on /tmp/x\n", "sess"),
        Absence::Proven
    );
    assert_eq!(
        read_absence(
            false,
            "",
            "error connecting to /tmp/x (No such file or directory)\n",
            "sess"
        ),
        Absence::SocketMissing
    );
    for stderr in [
        "",
        "error connecting to /tmp/x (Permission denied)\n",
        "error connecting to /tmp/x (Connection refused)\n",
        "error: cannot reach the no server running on host\n",
    ] {
        assert_eq!(
            read_absence(false, "", stderr, "sess"),
            Absence::Unreachable,
            "{stderr:?}"
        );
    }
}

#[test]
fn the_boot_time_proof_is_the_documented_table_and_fails_closed() {
    let live = |epoch: Option<i64>| epoch.map_or(Evidence::Silent, Evidence::At);
    let table = [
        // A server that ANSWERED needs no boot reasoning at all.
        (
            Absence::Listed,
            live(Some(BOOT - 1)),
            Some(BOOT),
            StopProbe::Present,
        ),
        (
            Absence::Proven,
            live(Some(BOOT + 1)),
            Some(BOOT),
            StopProbe::Absent,
        ),
        // The new proof: nothing ae wrote for this session postdates the boot,
        // so no post-boot server can be holding it.
        (
            Absence::SocketMissing,
            live(Some(BOOT - 1)),
            Some(BOOT),
            StopProbe::Absent,
        ),
        // Fail closed: the session was live on THIS boot, so the socket may
        // have been unlinked under a server that is still running.
        (
            Absence::SocketMissing,
            live(Some(BOOT)),
            Some(BOOT),
            StopProbe::Unknown,
        ),
        (
            Absence::SocketMissing,
            live(Some(BOOT + 1)),
            Some(BOOT),
            StopProbe::Unknown,
        ),
        // No boot time, no proof.
        (
            Absence::SocketMissing,
            live(Some(BOOT - 1)),
            None,
            StopProbe::Unknown,
        ),
        // No activity recorded at all: an absence of evidence is not evidence.
        (
            Absence::SocketMissing,
            live(None),
            Some(BOOT),
            StopProbe::Unknown,
        ),
        // Any OTHER failure keeps the strict proof, boot time or not.
        (
            Absence::Unreachable,
            live(Some(BOOT - 1)),
            Some(BOOT),
            StopProbe::Unknown,
        ),
        // DAMAGE IS NOT ABSENCE. This is the row that says an unreadable
        // record can never be stepped over: the boot is comfortably after
        // everything, and the answer is still no.
        (
            Absence::SocketMissing,
            Evidence::Unreadable,
            Some(BOOT),
            StopProbe::Unknown,
        ),
        // And a server that ANSWERED is still answered for — damage in ae's
        // own files cannot make a listed session disappear.
        (
            Absence::Listed,
            Evidence::Unreadable,
            Some(BOOT),
            StopProbe::Present,
        ),
        (
            Absence::Proven,
            Evidence::Unreadable,
            Some(BOOT),
            StopProbe::Absent,
        ),
    ];
    for (probe, evidence, boot, expected) in table {
        assert_eq!(
            classify_absence(probe, evidence, boot, NOW),
            expected,
            "{probe:?} / {evidence:?} / boot {boot:?}"
        );
    }
}

#[test]
fn a_boot_time_in_the_future_is_clock_skew_and_proves_nothing() {
    // The clock went backwards under us: every comparison against it is
    // meaningless, so the proof is refused rather than believed.
    assert_eq!(
        classify_absence(
            Absence::SocketMissing,
            Evidence::At(BOOT - 1),
            Some(NOW + 1),
            NOW
        ),
        StopProbe::Unknown
    );
    // The boundary: boot == now is not skew.
    assert_eq!(
        classify_absence(
            Absence::SocketMissing,
            Evidence::At(NOW - 1),
            Some(NOW),
            NOW
        ),
        StopProbe::Absent
    );
}

#[test]
fn every_unproven_absence_says_why_and_a_proven_one_says_nothing() {
    let reason = |probe, evidence, boot| {
        unproven_reason("/tmp/tmux-501/default", probe, evidence, boot, NOW)
    };
    assert_eq!(
        reason(Absence::SocketMissing, Evidence::At(BOOT + 60), Some(BOOT)),
        Some(
            "recorded server /tmp/tmux-501/default: socket missing; last live activity \
                 2026-09-11T05:51:55Z is not before boot 2026-09-11T05:50:55Z — cannot prove \
                 the session gone. Run 'ae doctor' for the evidence."
                .to_owned()
        )
    );
    for (probe, evidence, boot, needle) in [
        (
            Absence::SocketMissing,
            Evidence::At(BOOT - 1),
            None,
            "the host's boot time could not be read",
        ),
        (
            Absence::SocketMissing,
            Evidence::Silent,
            Some(BOOT),
            "no recorded live activity",
        ),
        (
            Absence::SocketMissing,
            Evidence::At(BOOT - 1),
            Some(NOW + 1),
            "is in the future (clock skew)",
        ),
        (
            Absence::Unreachable,
            Evidence::At(BOOT - 1),
            Some(BOOT),
            "the server could not be reached",
        ),
        // DAMAGE says so, and says it whatever the boot time is — including
        // the boot that would otherwise have PROVEN this session gone.
        (
            Absence::SocketMissing,
            Evidence::Unreadable,
            Some(BOOT),
            "is there and could not be read",
        ),
        (
            Absence::SocketMissing,
            Evidence::Unreadable,
            None,
            "is there and could not be read",
        ),
    ] {
        let text = reason(probe, evidence, boot).unwrap_or_default();
        assert!(text.contains(needle), "{probe:?}/{evidence:?}: {text:?}");
        assert!(
            text.contains("ae doctor"),
            "{probe:?}/{evidence:?}: {text:?}"
        );
    }
    // A PROVEN verdict has nothing to explain.
    for probe in [Absence::Listed, Absence::Proven] {
        assert_eq!(
            reason(probe, Evidence::At(BOOT - 1), Some(BOOT)),
            None,
            "{probe:?}"
        );
    }
}

#[test]
fn the_strict_probe_never_reaches_the_boot_time_proof() {
    // R3: `stop`, `end` and `compact` keep the proof they had. The strict
    // reader folds BOTH failure classes into `Unknown` and takes no boot
    // time at all — this is the pin that a later refactor cannot quietly
    // wire the new evidence into the destructive gate.
    assert_eq!(
        interpret_stopped(
            false,
            "",
            "error connecting to /tmp/x (No such file or directory)\n",
            "sess"
        ),
        StopProbe::Unknown,
        "a missing socket is still unproven for the destructive gate"
    );
    // And it CANNOT reach the new proof: the strict reader takes neither a
    // boot time nor any evidence, so there is no argument to pass it.
    assert_eq!(
        classify_absence(
            read_absence(
                false,
                "",
                "error connecting to /tmp/x (No such file or directory)\n",
                "sess"
            ),
            Evidence::At(BOOT - 1),
            Some(BOOT),
            NOW
        ),
        StopProbe::Absent,
        "the same bytes ARE provable once the evidence is supplied — which is \
             the difference the two readers exist to keep"
    );
}

#[test]
fn a_successful_run_yields_exactly_the_names_it_printed() {
    assert_eq!(
        interpret_sessions(true, "alpha\nbeta\n"),
        Ok(vec!["alpha".to_owned(), "beta".to_owned()])
    );
    assert_eq!(
        interpret_sessions(true, ""),
        Ok(Vec::new()),
        "an empty SUCCESS is the only thing that can prove a name absent"
    );
    assert_eq!(
        interpret_sessions(true, "solo\n\n"),
        Ok(vec!["solo".to_owned()]),
        "a blank line is not a session called nothing"
    );
}

#[test]
fn a_name_containing_spaces_survives_intact() {
    // The format asks for one field, so the whole line is the name.
    assert_eq!(
        interpret_sessions(true, "my session\n"),
        Ok(vec!["my session".to_owned()])
    );
}

#[test]
fn the_marker_is_the_value_and_an_unset_variable_is_not_one() {
    assert_eq!(
        interpret_marker(true, "AE_SESSION=my-feature\n"),
        Some("my-feature".to_owned())
    );
    assert_eq!(
        interpret_marker(true, "-AE_SESSION\n"),
        None,
        "tmux spells UNSET with a leading dash, and that is not a value"
    );
    assert_eq!(interpret_marker(true, ""), None);
    assert_eq!(
        interpret_marker(false, "AE_SESSION=my-feature\n"),
        None,
        "a failed run reports nothing, however convincing its output looks"
    );
    assert_eq!(
        interpret_marker(true, "AE_SESSION=\n"),
        Some(String::new()),
        "an empty value is a value; whether it PROVES ownership is liveness's question"
    );
}

/// A pane reading, spelled the way the fields arrive.
fn pane(dead: Option<bool>, slot: Option<&str>, command: Option<&str>) -> ObservedPane {
    ObservedPane {
        pane_id: None,
        pid: None,
        dead,
        slot: slot.map(ToOwned::to_owned),
        command: command.map(ToOwned::to_owned),
        observed: crate::harness_state::HarnessState::Unknown,
    }
}

/// A pane with a usable slot and nothing else said about it.
fn slotted(slot: &str) -> ObservedPane {
    pane(Some(false), Some(slot), Some("claude"))
}

/// A pane carrying no usable identity.
fn unslotted() -> ObservedPane {
    pane(Some(false), None, Some("zsh"))
}

#[test]
fn pane_enumeration_is_session_wide_and_asks_for_liveness_and_observed_frame() {
    assert_eq!(
        list_panes_args(&named("work"), "my-feature"),
        [
            "-L",
            "work",
            "list-panes",
            "-s",
            "-t",
            "=my-feature",
            "-F",
            "#{pane_dead} | #{@ae_slot} | #{@ae_observed} | #{pane_current_command}"
        ]
    );
}

#[test]
fn the_pane_format_asks_for_identity_and_never_for_the_display_field() {
    // `@ae_agent` is DISPLAY, so identity is never associated on it.
    let args = list_panes_args(&ServerId::Ambient, "s").join(" ");
    assert!(args.contains("#{@ae_slot}"));
    assert!(args.contains("#{pane_dead}"), "{args}");
    assert!(args.contains("#{pane_current_command}"), "{args}");
    assert!(!args.contains("@ae_agent"), "{args}");
}

#[test]
fn pane_dead_comes_first_so_nothing_upstream_can_shift_it() {
    // ORDER IS THE SAFETY PROPERTY.
    let fields: Vec<&str> = super::PANE_FORMAT.split(super::FIELD_SEPARATOR).collect();
    assert_eq!(fields.len(), super::PANE_FIELDS);
    assert_eq!(fields[0], "#{pane_dead}");
    assert_eq!(fields[3], "#{pane_current_command}", "free-est text last");
}

#[test]
fn a_failed_pane_query_is_a_failure_whatever_it_printed() {
    for payload in ["", "0 | main |  | claude\n", "can't find window: nosuch\n"] {
        assert_eq!(
            interpret_panes(false, payload),
            Err(QueryFailed),
            "{payload:?}"
        );
    }
}

#[test]
fn an_unmarked_pane_is_a_pane_and_not_a_dropped_line() {
    // MEASURED against a real server.
    assert_eq!(
        interpret_panes(true, "0 | main |  | zsh\n0 |  |  | zsh\n1 |  |  | true\n"),
        Ok(vec![
            pane(Some(false), Some("main"), Some("zsh")),
            pane(Some(false), None, Some("zsh")),
            pane(Some(true), None, Some("true")),
        ]),
        "three lines, three panes, and the unmarked one is still one"
    );
    assert_eq!(
        interpret_panes(true, ""),
        Ok(Vec::new()),
        "no output at all is no panes"
    );
}

#[test]
fn an_exited_pane_keeps_both_facts_that_tell_it_apart_from_a_live_one() {
    // A `remain-on-exit` pane reports the EXITED
    // process's command, and `true` is not in shell set — so the
    // command field alone reads like a live agent. The only thing that
    // separates it from a live pane is `pane_dead`, and this pins that the
    // read carries it rather than discarding it.
    let exited = interpret_panes(true, "1 | worker |  | true\n").expect("success");
    assert_eq!(exited, vec![pane(Some(true), Some("worker"), Some("true"))]);
    assert_eq!(
        exited[0].dead,
        Some(true),
        "the conjunct that stops a dead agent reading alive survived the read"
    );
    assert_eq!(
        exited[0].command.as_deref(),
        Some("true"),
        "and so did the command that would otherwise have proven it alive"
    );
}

#[test]
fn a_marker_that_is_empty_or_blank_is_not_a_usable_identity() {
    // RESTORED BY NAME, and not merely as bookkeeping.
    assert_eq!(
        interpret_panes(true, "0 |  |  | zsh\n"),
        Ok(vec![pane(Some(false), None, Some("zsh"))]),
        "an empty slot field is no identity"
    );
    assert_eq!(
        interpret_panes(true, "0 |     |  | zsh\n"),
        Ok(vec![pane(Some(false), None, Some("zsh"))]),
        "and neither is a whitespace-only one — same answer, different bytes"
    );
    assert_eq!(
        interpret_panes(true, "0 | main |  |    \n"),
        Ok(vec![pane(Some(false), Some("main"), None)]),
        "the command field normalizes the same way, and an unassociated \
             unreadable command in the not-alive set for the same reason"
    );
}

#[test]
fn an_unreadable_field_is_no_reading_rather_than_a_convenient_one() {
    // An empty or absent reading is NOT alive, because absence of evidence
    // is not evidence.
    assert_eq!(
        interpret_panes(true, "0 | main |  | \n"),
        Ok(vec![pane(Some(false), Some("main"), None)]),
        "an empty command is no command, not a non-shell one"
    );
    assert_eq!(
        interpret_panes(true, " | main |  | claude\n"),
        Ok(vec![pane(None, Some("main"), Some("claude"))]),
        "an empty pane_dead is no reading, and must not pass for `0`"
    );
    assert_eq!(
        interpret_panes(true, "2 | main |  | claude\n"),
        Ok(vec![pane(None, Some("main"), Some("claude"))]),
        "and neither does anything else that is not 0 or 1"
    );
    assert_eq!(
        interpret_panes(true, "0 |  |  | claude\n"),
        Ok(vec![pane(Some(false), None, Some("claude"))]),
        "an empty slot is no identity; the other two readings survive it"
    );
}

#[test]
fn a_line_of_the_wrong_arity_is_a_pane_that_says_nothing() {
    // A TAB CANNOT BE SMUGGLED THROUGH A FIELD.
    let forged = interpret_panes(true, "0 | main | idle | evil | zsh\n").expect("success");
    assert_eq!(
        forged,
        vec![pane(None, None, None)],
        "still one pane, and it says nothing at all"
    );
    assert_eq!(
        interpret_panes(true, "\n"),
        Ok(vec![pane(None, None, None)]),
        "a blank line is a pane with no reading, never a dropped pane"
    );
    assert_eq!(
        interpret_panes(true, "0 | main\n"),
        Ok(vec![pane(None, None, None)]),
        "and too few fields is refused for the same reason as too many"
    );
}

#[test]
fn the_two_interpreters_disagree_about_a_blank_line_on_purpose() {
    // A session called nothing does not exist; a pane that reported nothing
    // does.
    assert_eq!(
        interpret_sessions(true, "a\n\nb\n"),
        Ok(vec!["a".to_owned(), "b".to_owned()])
    );
    assert_eq!(
        interpret_panes(true, "a\n\nb\n").map(|panes| panes.len()),
        Ok(3)
    );
}

#[test]
fn an_empty_roster_slot_matches_no_pane() {
    // A ROSTER SLOT CAN BE EMPTY: `absorb_roster` validates alias and name
    // and never the slot, so `agent.=cl:lead` in a hand-edited meta yields a
    // roster entry whose slot is "".
    let panes = interpret_panes(true, "0 |  |  | zsh\n0 | main | busy | claude\n")
        .expect("a successful enumeration");
    assert_eq!(
        slot_observation(&panes, ""),
        SlotObservation::Absent { unidentified: 1 },
        "an empty roster slot matches no pane, including the unmarked one"
    );
}

#[test]
fn a_slot_is_found_only_by_exact_match() {
    let panes = [slotted("main"), slotted("worker")];
    assert_eq!(slot_observation(&panes, "main"), SlotObservation::Unique);
    assert_eq!(
        slot_observation(&panes, "mai"),
        SlotObservation::Absent { unidentified: 0 },
        "a PREFIX of a slot that is there is absent, not present"
    );
    assert_eq!(
        slot_observation(&panes, "main2"),
        SlotObservation::Absent { unidentified: 0 },
        "and so is a slot that merely extends one"
    );
}

#[test]
fn a_duplicated_slot_is_ambiguous_rather_than_a_match() {
    let panes = [slotted("main"), slotted("main")];
    assert_eq!(
        slot_observation(&panes, "main"),
        SlotObservation::Duplicated { panes: 2 },
        "two panes claiming one slot associate it to neither"
    );
}

#[test]
fn absence_carries_how_many_panes_identified_nothing() {
    // The COUNT, not a conclusion.
    assert_eq!(
        slot_observation(&[slotted("other")], "main"),
        SlotObservation::Absent { unidentified: 0 }
    );
    assert_eq!(
        slot_observation(&[slotted("other"), unslotted()], "main"),
        SlotObservation::Absent { unidentified: 1 },
        "an unassociated pane is exactly the fact the reader needs"
    );
    assert_eq!(
        slot_observation(&[], "main"),
        SlotObservation::Absent { unidentified: 0 },
        "no panes at all identifies nothing and hides nothing"
    );
}

#[test]
fn a_relative_socket_path_cannot_address_a_server() {
    assert!(is_addressable_socket(Path::new("/tmp/ae.sock")));
    assert!(!is_addressable_socket(Path::new("relative/ae.sock")));
}

#[test]
fn stop_listings_are_read_whole_or_not_at_all() {
    use super::{interpret_view_holders, interpret_view_uuids, interpret_window_snapshot};
    assert_eq!(
        interpret_window_snapshot(true, "@0\n@12\n"),
        Some(vec!["@0".to_owned(), "@12".to_owned()])
    );
    assert_eq!(interpret_window_snapshot(true, ""), None);
    assert_eq!(interpret_window_snapshot(true, "@0\nx\n"), None);
    assert_eq!(interpret_window_snapshot(false, "@0\n"), None);
    assert_eq!(
        interpret_view_holders(true, "$1 | a | b | @3 | $0\n"),
        Some(vec![(
            "$1".to_owned(),
            "a | b".to_owned(),
            "@3".to_owned(),
            "$0".to_owned()
        )]),
        "a name may carry the separator; ids are read from the ends"
    );
    assert_eq!(interpret_view_holders(true, "$1 | a | 3 | \n"), None);
    assert_eq!(interpret_view_holders(true, "1 | a | @3 | \n"), None);
    assert_eq!(interpret_view_holders(false, ""), None);
    assert_eq!(interpret_view_holders(true, ""), Some(Vec::new()));
    assert_eq!(
        interpret_view_uuids(true, "$4 | _aeview-1 | u\n$5 | real | \n"),
        Some(vec![
            ("$4".to_owned(), "_aeview-1".to_owned(), "u".to_owned()),
            ("$5".to_owned(), "real".to_owned(), String::new()),
        ])
    );
    assert_eq!(interpret_view_uuids(true, "x | a | u\n"), None);
}

#[test]
fn only_views_carrying_the_identity_are_its_orphans() {
    let rows = vec![
        ("$4".to_owned(), "_aeview-1".to_owned(), "u".to_owned()),
        ("$5".to_owned(), "real".to_owned(), "u".to_owned()),
        ("$6".to_owned(), "_aeview-2".to_owned(), "v".to_owned()),
        ("$7".to_owned(), "_aeview-3".to_owned(), String::new()),
    ];
    assert_eq!(super::orphan_view_ids("u", &rows), vec!["$4".to_owned()]);
    assert!(super::orphan_view_ids("", &rows).is_empty());
}

#[test]
fn a_pane_views_grouped_session_never_claims_the_real_sessions_panes() {
    use super::{FleetPane, interpret_fleet_panes};

    // tmux lists sessions by name, so `_aeview-…` sorts before a
    // lowercase real name and would win a first-match lookup.
    let listing = "_aeview-1-2 | %4 | lead\nhub | %4 | lead\n";
    assert_eq!(
        interpret_fleet_panes(true, listing),
        Some(vec![FleetPane {
            session: "hub".to_owned(),
            pane: "%4".to_owned(),
            agent: "lead".to_owned(),
        }])
    );
}

#[test]
fn a_view_census_with_a_hole_proves_no_absence() {
    use super::{ViewCensusRow, interpret_view_census};

    assert_eq!(interpret_view_census(false, "$0 | 1 | hub\n"), None);
    assert_eq!(
        interpret_view_census(true, "$0 | 1 | hub\n$3 | 0 | _aeview-1-2\n"),
        Some(vec![
            ViewCensusRow {
                id: "$0".to_owned(),
                attached: true,
                name: "hub".to_owned(),
            },
            ViewCensusRow {
                id: "$3".to_owned(),
                attached: false,
                name: "_aeview-1-2".to_owned(),
            },
        ])
    );
    assert_eq!(interpret_view_census(true, "$0 | 1 | hub\ngarbage\n"), None);
    assert_eq!(interpret_view_census(true, "$0 | x | hub\n"), None);
}

#[test]
fn a_view_lands_only_on_a_pane_its_session_still_holds() {
    let args = super::view_attach_args(
        &crate::inventory::ServerId::Ambient,
        "$7",
        "_aeview-1-2",
        "%12",
        "1-2",
    );
    let landing = args.last().map(String::as_str).unwrap_or_default();
    let window = "#{W:#{P:#{?#{==:#{pane_id},%12},#{window_id},}}}";
    assert_eq!(
        landing,
        format!(
            "if-shell -F '{window}' 'select-window -t =_aeview-1-2:{window} ; \
                 select-pane -t =_aeview-1-2:{window}.%12' \"detach-client\""
        )
    );
    // Expanded against the VIEW session, never the real one.
    let run = args
        .iter()
        .position(|word| word == "run-shell")
        .unwrap_or_default();
    assert_eq!(
        args[run..run + 4],
        ["run-shell", "-C", "-t", "=_aeview-1-2:"]
    );
}

#[test]
fn a_failed_fleet_listing_is_a_failure_and_an_empty_one_is_an_empty_server() {
    use super::{FLEET_PANE_FORMAT, FleetPane, fleet_panes_args, interpret_fleet_panes};
    use crate::inventory::ServerId;
    use crate::meta::Selector;

    assert_eq!(
        fleet_panes_args(&ServerId::Selected(Selector::Name("ae-dev".to_owned()))),
        vec!["-L", "ae-dev", "list-panes", "-a", "-F", FLEET_PANE_FORMAT]
    );
    // The two must never collapse into one: a failed read that read as an
    // empty server would say every session is somewhere else.
    assert_eq!(interpret_fleet_panes(false, ""), None);
    assert_eq!(interpret_fleet_panes(false, "hub | %1 | lead\n"), None);
    assert_eq!(interpret_fleet_panes(true, ""), Some(Vec::new()));

    let listing = "hub | %1 | lead\nhub | %2 | \nhub | %3 | a | b\ntruncated\n";
    assert_eq!(
        interpret_fleet_panes(true, listing),
        Some(vec![
            FleetPane {
                session: "hub".to_owned(),
                pane: "%1".to_owned(),
                agent: "lead".to_owned(),
            },
            FleetPane {
                session: "hub".to_owned(),
                pane: "%2".to_owned(),
                agent: String::new(),
            },
            // The split is BOUNDED, so a separator inside the last field is
            // part of the stamp rather than a fourth field.
            FleetPane {
                session: "hub".to_owned(),
                pane: "%3".to_owned(),
                agent: "a | b".to_owned(),
            },
        ]),
        "a line with too few fields is corruption, not a pane"
    );
}

#[test]
fn a_menu_jump_spells_its_verb_with_the_one_focus_verb_and_takes_the_window_first() {
    use super::{
        FocusVerb, guarded_jump_client_id_command, guarded_jump_id_command, jump_command,
        switch_command,
    };

    // The verb has ONE owner; a second spelling here could drift from it.
    assert!(switch_command("hub").starts_with(FocusVerb::SwitchClient.as_str()));
    assert_eq!(switch_command("hub"), "switch-client -t =hub");
    let jump = jump_command("hub", "%12");
    assert_eq!(
        jump,
        "switch-client -t =hub ; select-window -t %12 ; select-pane -t %12"
    );
    assert!(jump.starts_with(&switch_command("hub")));
    let window = jump.find("select-window").expect("a window step");
    let pane = jump.find("select-pane").expect("a pane step");
    assert!(window < pane, "{jump}");
    assert_eq!(
        guarded_jump_id_command("$7", "%12"),
        "switch-client -t $7 ; if-shell -F -t %12 '##{==:##{session_id},$7}' 'select-window -t %12 ; select-pane -t %12'"
    );
    assert_eq!(
        guarded_jump_client_id_command("/dev/ttys001", "$7", "%12"),
        "switch-client -c '/dev/ttys001' -t $7 ; if-shell -F -t %12 '##{==:##{session_id},$7}' 'select-window -t %12 ; select-pane -t %12'"
    );
}

/// A seat's jump is the session row's guarded select, waiting in
/// `run-shell -C` so it runs after the session's focus hook.
#[test]
fn a_seat_jump_waits_behind_the_session_hook_with_the_same_guard() {
    use super::{seat_jump_client_id_command, seat_jump_id_command};

    assert_eq!(
        seat_jump_id_command("$7", "%12"),
        "switch-client -t $7 ; run-shell -C { if-shell -F -t %12 '##{==:##{session_id},$7}' 'select-window -t %12 ; select-pane -t %12' }"
    );
    assert_eq!(
        seat_jump_client_id_command("/dev/ttys001", "$7", "%12"),
        "switch-client -c '/dev/ttys001' -t $7 ; run-shell -C { if-shell -F -t %12 '##{==:##{session_id},$7}' 'select-window -t %12 ; select-pane -t %12' }"
    );
}

#[test]
fn status_click_predicates_name_each_owned_range() {
    assert_eq!(
        super::MOUSE_STATUS_PICKER,
        "#{||:#{==:#{mouse_status_range},ae},#{==:#{mouse_status_range},ae-more}}"
    );
    assert_eq!(
        super::MOUSE_STATUS_SETTINGS,
        "#{==:#{mouse_status_range},ae-settings}"
    );
    assert_eq!(
        super::MOUSE_STATUS_SESSION,
        "#{==:#{mouse_status_range},session}"
    );
    assert_eq!(
        super::MOUSE_STATUS_WINDOW,
        "#{==:#{mouse_status_range},window}"
    );
}

#[test]
fn the_version_is_asked_of_the_server_rather_than_of_the_binary_on_path() {
    use super::{VERSION_FORMAT, interpret_display_value, version_args};
    use crate::inventory::ServerId;
    use crate::meta::Selector;

    // `-V` would answer for whichever tmux is first on PATH; this asks the
    // server that is actually going to draw the menu.
    assert_eq!(
        version_args(&ServerId::Ambient),
        vec!["display-message", "-p", VERSION_FORMAT]
    );
    assert_eq!(
        version_args(&ServerId::Selected(Selector::Name("ae-dev".to_owned()))),
        vec!["-L", "ae-dev", "display-message", "-p", VERSION_FORMAT]
    );
    assert_eq!(
        interpret_display_value(true, "3.7b\n"),
        Some("3.7b".to_owned())
    );
    assert_eq!(interpret_display_value(false, "3.7b\n"), None);
}

/// A pane's chat stamp: an unstamped pane is an empty stamp, never the
/// same thing as tmux not answering.
#[test]
fn an_unstamped_pane_is_not_an_unanswered_one() {
    assert_eq!(
        super::pane_console_args(&ServerId::Ambient, "%3"),
        ["display-message", "-p", "-t", "%3", "#{@ae_console}"]
    );
    assert_eq!(
        super::interpret_pane_console(true, "u\n"),
        Some("u".to_owned())
    );
    assert_eq!(
        super::interpret_pane_console(true, "\n"),
        Some(String::new())
    );
    assert_eq!(super::interpret_pane_console(false, "u\n"), None);
}

#[test]
fn the_focus_verb_follows_inside_ness_and_carries_the_target_separately() {
    use super::{FocusVerb, attach_args, focus_args};
    use crate::inventory::ServerId;
    use crate::meta::Selector;
    // Frozen's `_next_focus_verb`: attach-session errors inside tmux
    // ("sessions should be nested with care"), and there is no client to
    // switch outside it.
    assert_eq!(FocusVerb::for_inside(true), FocusVerb::SwitchClient);
    assert_eq!(FocusVerb::for_inside(false), FocusVerb::AttachSession);
    assert_eq!(FocusVerb::SwitchClient.as_str(), "switch-client");
    assert_eq!(FocusVerb::AttachSession.as_str(), "attach-session");

    // The target is its own argv element, never concatenated: frozen took
    // the trouble because "a session name with spaces must not be
    // word-split", and an argument vector is where that stays true.
    assert_eq!(
        focus_args(&ServerId::Ambient, FocusVerb::SwitchClient, "a b"),
        vec!["switch-client", "-t", "=a b"]
    );
    assert_eq!(
        focus_args(&ServerId::Ambient, FocusVerb::AttachSession, "s"),
        vec!["attach-session", "-t", "=s"]
    );
    assert_eq!(
        attach_args(&ServerId::Selected(Selector::Name("ae".to_owned()))),
        vec!["-L", "ae", "attach-session"]
    );
    assert_eq!(
        attach_args(&ServerId::Selected(Selector::Socket("/tmp/ae.sock".into()))),
        vec!["-S", "/tmp/ae.sock", "attach-session"]
    );
}

#[test]
fn client_switch_targets_exact_client_and_session_ids() {
    use super::switch_client_args;
    use crate::inventory::ServerId;
    assert_eq!(
        switch_client_args(&ServerId::Ambient, "/dev/ttys001", "next"),
        vec!["switch-client", "-c", "/dev/ttys001", "-t", "=next"]
    );
}

#[test]
fn a_pane_tty_listing_is_a_list_and_a_failed_one_is_no_answer() {
    use super::{interpret_pane_ttys, pane_ttys_args};
    use crate::inventory::ServerId;
    assert_eq!(
        pane_ttys_args(&ServerId::Ambient),
        vec!["list-panes", "-a", "-F", "#{pane_tty}"],
        "-a: the question is whether THIS terminal is a pane of the server, \
             and the answer may live in any session"
    );
    assert_eq!(
        interpret_pane_ttys(true, "/dev/ttys001\n/dev/ttys002\n"),
        Some(vec!["/dev/ttys001".to_owned(), "/dev/ttys002".to_owned()])
    );
    assert_eq!(
        interpret_pane_ttys(true, ""),
        Some(Vec::new()),
        "a server with no panes ANSWERED"
    );
    assert_eq!(
        interpret_pane_ttys(false, "/dev/ttys001\n"),
        None,
        "a failed run is no answer, whatever it printed"
    );
}

#[test]
fn the_tty_comparison_strips_dev_from_both_sides_exactly_once() {
    use super::tty_is_a_pane;
    // Procps prints `pts/3` and BSD ps `ttys039`, against tmux's absolute
    // `/dev/…`.
    let panes = vec!["/dev/ttys039".to_owned(), "/dev/pts/3".to_owned()];
    assert!(tty_is_a_pane("ttys039", &panes));
    assert!(tty_is_a_pane("pts/3", &panes));
    assert!(
        tty_is_a_pane("/dev/pts/3", &panes),
        "a ps that spelled the full path must not make every pane look stale"
    );
    assert!(
        !tty_is_a_pane("ttys001", &panes),
        "a real non-pane terminal"
    );
    assert!(!tty_is_a_pane("", &panes), "no tty is not every tty");
    assert!(
        !tty_is_a_pane("ttys039", &[]),
        "a server with no panes matches nothing"
    );
}

#[test]
fn the_current_session_question_carries_no_target() {
    use super::current_session_args;
    use crate::inventory::ServerId;
    // No `-t`: the question is which session THIS client is in, and naming
    // a target would answer about the target instead.
    assert_eq!(
        current_session_args(&ServerId::Ambient),
        vec!["display-message", "-p", "#{session_name}"]
    );
}

/// PIN (a): the `list-keys` read behind `ae doctor`'s input-map check.
#[test]
fn the_key_table_read_keeps_key_and_command_and_tolerates_garbage() {
    use super::{interpret_list_keys, list_keys_args};
    use crate::inventory::ServerId;
    assert_eq!(
        list_keys_args(&ServerId::Ambient, "root"),
        vec!["list-keys", "-T", "root"]
    );
    // A failed run is no table, not an empty one.
    assert_eq!(interpret_list_keys(false, "bind-key -T root a b\n"), None);
    let printed = "bind-key  -T root MouseDown1Status          run-shell -C -t \"{mouse}\" \"#{?x}\"\n\
             not a bind row at all\n\
             bind-key -T root\n\
             bind-key -r -T prefix a run-shell -b '/opt/ae' 'orchestrator'\n";
    let entries = interpret_list_keys(true, printed).expect("a table");
    assert_eq!(entries.len(), 2, "garbage lines are skipped: {entries:?}");
    assert_eq!(entries[0].key, "MouseDown1Status");
    assert!(
        entries[0].command.starts_with("run-shell -C"),
        "{}",
        entries[0].command
    );
    assert!(
        entries[0].command.contains("#{?x}"),
        "{}",
        entries[0].command
    );
    assert_eq!(entries[1].key, "a");
    assert!(
        entries[1].command.contains("'/opt/ae'"),
        "{}",
        entries[1].command
    );
}

#[test]
fn pane_current_path_argv_targets_the_pane_and_format() {
    let sock = ServerId::Selected(Selector::Socket(std::path::PathBuf::from("/tmp/s")));
    assert_eq!(
        super::pane_current_path_args(&sock, "%3"),
        vec![
            "-S",
            "/tmp/s",
            "display-message",
            "-p",
            "-t",
            "%3",
            "#{pane_current_path}"
        ]
    );
}

/// appcue R2 frozen acceptance: the pane-place query prints (never flashes)
/// and its two fields split raw; validation lives in the encoder downstream.
#[test]
fn pane_place_args_print_and_interpret_splits_two_fields() {
    let args = super::pane_place_args(&super::ServerId::Ambient, "%7");
    assert!(args.contains(&"-p".to_owned()), "prints");
    assert!(!args.contains(&"-d".to_owned()), "never transient");
    let targeted = args.windows(2).any(|w| w == ["-t", "%7"]);
    assert!(targeted, "targets the pane: {args:?}");
    let both = args
        .iter()
        .any(|a| a.contains("#{session_name}") && a.contains("#{window_index}"));
    assert!(both, "both fields: {args:?}");
    let split = super::interpret_pane_place;
    assert_eq!(split(true, "alpha | 2"), Some(("alpha".into(), "2".into())));
    assert_eq!(split(false, "alpha | 2"), None);
    assert_eq!(split(true, "nope"), None);
    assert_eq!(split(true, "a | b | c"), Some(("a".into(), "b | c".into())));
}

fn all_panes_row(name: &str, branch: &str, dead: &str, slot: &str, command: &str) -> String {
    [name, branch, dead, slot, "", command, "", "100"].join(super::FIELD_SEPARATOR)
}

#[test]
fn an_all_panes_row_carries_its_valid_trailing_pane_id() {
    let all =
        super::interpret_all_panes(true, "seat | branch | 0 | main | busy | claude | %42 | 100")
            .unwrap();
    assert!(
        all.session("seat").is_some(),
        "an eight-cell row is attributed"
    );
    let row = &all.session("seat").unwrap().panes[0];
    assert_eq!(row.pane_id.as_deref(), Some("%42"));
    assert!(
        super::interpret_all_panes(true, "seat | branch | 0 | main | busy | claude | %42")
            .unwrap()
            .session("seat")
            .is_none(),
        "a short row cannot prove the new format's arity"
    );
}

#[test]
fn an_invalid_all_panes_id_preserves_every_existing_row_fact() {
    for id in ["", "%", "%x", "42", "%4x"] {
        let text = format!("seat | branch | 1 | main | busy | claude | {id} | 100");
        let all = super::interpret_all_panes(true, &text).unwrap();
        assert!(
            all.session("seat").is_some(),
            "invalid id {id:?} keeps the row"
        );
        let session = all.session("seat").unwrap();
        assert_eq!(session.branch.as_deref(), Some("branch"));
        let row = &session.panes[0];
        assert_eq!(row.pane_id, None, "invalid pane id {id:?}");
        assert_eq!(row.dead, Some(true));
        assert_eq!(row.slot.as_deref(), Some("main"));
        assert_eq!(row.command.as_deref(), Some("claude"));
        assert_eq!(row.observed, crate::harness_state::HarnessState::Busy);
    }
}

#[test]
fn the_all_panes_format_is_the_name_the_branch_then_the_per_session_cells() {
    assert!(
        super::ALL_PANES_FORMAT.starts_with("#{session_name} | #{@ae_branch_name} | "),
        "{}",
        super::ALL_PANES_FORMAT
    );
    assert!(
        super::ALL_PANES_FORMAT
            .strip_suffix(" | #{pane_id} | #{pane_pid}")
            .is_some_and(|prefix| prefix.ends_with(super::PANE_FORMAT)),
        "{}",
        super::ALL_PANES_FORMAT
    );
    let args = super::list_all_panes_args(&ServerId::Selected(Selector::Socket(
        std::path::PathBuf::from("/tmp/s.sock"),
    )));
    assert_eq!(
        args,
        [
            "-S",
            "/tmp/s.sock",
            "list-panes",
            "-a",
            "-F",
            super::ALL_PANES_FORMAT
        ]
    );
}

#[test]
fn rows_are_attributed_by_the_exact_name_never_a_prefix() {
    let text = [
        all_panes_row("fan", "", "0", "lead", "claude"),
        all_panes_row("fanout", "featurex", "0", "lead", "claude"),
        all_panes_row("fanout", "featurex", "1", "worker", "zsh"),
    ]
    .join("\n");
    let all = super::interpret_all_panes(true, &text).unwrap();
    let fan = all.session("fan").unwrap();
    assert_eq!(fan.panes.len(), 1);
    assert_eq!(fan.branch, None, "an empty branch cell is no branch");
    let fanout = all.session("fanout").unwrap();
    assert_eq!(fanout.panes.len(), 2);
    assert_eq!(fanout.branch.as_deref(), Some("featurex"));
    assert!(all.session("fa").is_none(), "a prefix names no session");
    assert!(all.session("fanout2").is_none());
}

#[test]
fn a_session_with_no_rows_reads_as_not_listed_and_a_failed_run_as_failed() {
    let all =
        super::interpret_all_panes(true, &all_panes_row("up", "", "0", "lead", "claude")).unwrap();
    assert!(all.session("gone").is_none());
    assert!(
        super::interpret_all_panes(true, "")
            .unwrap()
            .session("up")
            .is_none()
    );
    assert_eq!(
        super::interpret_all_panes(false, "up | | 0 | lead | | x"),
        Err(QueryFailed)
    );
}

#[test]
fn a_row_that_does_not_parse_is_dropped_and_never_misattributed() {
    let piped_name = all_panes_row("fan | extra", "", "0", "lead", "claude");
    let hostile_cell = all_panes_row("fan", "", "0", "lead | worker", "claude");
    let short = ["fan", "", "0", "lead"].join(super::FIELD_SEPARATOR);
    let nameless = all_panes_row("", "", "0", "lead", "claude");
    let text = [
        piped_name,
        hostile_cell,
        short,
        nameless,
        all_panes_row("fan", "", "0", "good", "claude"),
    ]
    .join("\n");
    let all = super::interpret_all_panes(true, &text).unwrap();
    let fan = all.session("fan").unwrap();
    assert_eq!(
        fan.panes.len(),
        1,
        "only the whole row is kept: {:?}",
        fan.panes
    );
    assert_eq!(fan.panes[0].slot.as_deref(), Some("good"));
}

#[test]
fn carriage_returns_and_trailing_blanks_do_not_change_a_cell() {
    let text = format!(
        "{}\r\n{}",
        all_panes_row("up", "br  ", "0", "lead", "claude"),
        all_panes_row("up", "", "1", "worker", "zsh  ")
    );
    let up = super::interpret_all_panes(true, &text).unwrap();
    let up = up.session("up").unwrap();
    assert_eq!(up.branch.as_deref(), Some("br"));
    assert_eq!(up.panes.len(), 2);
    assert_eq!(up.panes[1].command.as_deref(), Some("zsh"));
}

#[test]
fn the_per_session_read_still_keeps_a_malformed_line_as_a_blank_pane() {
    let panes = super::interpret_panes(true, "0 | lead | | claude\nnot four fields").unwrap();
    assert_eq!(panes.len(), 2, "interpret_panes keeps every line");
    assert_eq!(panes[1].slot, None);
    assert_eq!(panes[1].dead, None);
}
