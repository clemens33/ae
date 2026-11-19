use super::*;
#[test]
fn prefix_upper_o_captures_the_invoker_and_doctor_agrees_without_touching_lower_o() {
    for mouse in [false, true] {
        let server = ServerId::Selected(Selector::Name("ae".into()));
        let bindings = status_bindings_argv(&server, &["/opt/ae".into()], mouse);
        let key = |words: &[String], verb: &str, letter: &str| {
            words
                .windows(4)
                .any(|w| w == [verb, "-T", "prefix", letter])
        };
        let upper: Vec<_> = bindings
            .iter()
            .map(TmuxArgv::as_args)
            .filter(|w| key(w, "bind-key", "O"))
            .collect();
        assert_eq!(upper.len(), 1, "one prefix O binding in each capability");
        let command = upper[0].join(" ").replace('\'', "");
        assert!(command.contains("run-shell -b "));
        assert!(command.contains("/opt/ae") && command.contains("_orchestrator-jump"));
        for field in [
            "--client #{q:client_name}",
            "--client-pid #{client_pid}",
            "--server-pid #{pid}",
            "--server-start #{start_time}",
            "--from #{session_id} #{window_id} #{pane_id}",
        ] {
            assert!(
                command.contains(field),
                "keypress-time field missing: {command}"
            );
        }
        assert!(!command.contains("##{"), "direct run-shell format layer");
        assert!(bindings.iter().all(|b| !key(b.as_args(), "bind-key", "o")
            && !key(b.as_args(), "unbind-key", "o")), "stock prefix o survives");
        let expected = expected_status_bindings(mouse);
        let upper: Vec<_> = expected
            .iter()
            .filter(|e| e.table == "prefix" && e.key == "O")
            .collect();
        assert_eq!(upper.len(), 1);
        assert!(!upper[0].absent && upper[0].names_launcher);
        assert!(!expected.iter().any(|e| e.table == "prefix" && e.key == "o"));
    }
    assert!(status_bindings_argv(&ServerId::Ambient, &["/opt/ae".into()], true).is_empty());
}
