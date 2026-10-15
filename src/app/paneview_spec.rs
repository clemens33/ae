//! Frozen acceptance cases: paneview plan r2, independent navigator.
//! Pure key/state contracts only; real terminal handoff is a live proof.

use std::time::{Duration, Instant};

use crate::inventory::ServerId;
use crate::meta::Selector;
use crate::tmux;

fn server() -> ServerId {
    ServerId::Selected(Selector::Socket("/tmp/paneview-spec.sock".into()))
}

#[test]
fn handoff_discards_old_key_chunks_but_keeps_keys_from_the_resumed_app() {
    let before = Instant::now();
    let resumed = before + Duration::from_secs(1);
    let after = resumed + Duration::from_nanos(1);
    assert!(!super::paneview::keep_key(before, resumed));
    assert!(super::paneview::keep_key(resumed, resumed));
    assert!(super::paneview::keep_key(after, resumed));
}

#[test]
fn both_return_keys_are_advertised_and_ctrl_c_is_left_to_the_harness() {
    assert_eq!(tmux::VIEW_RETURN_KEYS, ["F12", "C-]"]);
    for key in tmux::VIEW_RETURN_KEYS {
        assert!(tmux::VIEW_HINT.contains(key));
    }
    assert!(tmux::VIEW_HINT.contains("prefix d"));
    assert!(!tmux::VIEW_RETURN_KEYS.contains(&"C-c"));
}

#[test]
fn return_bindings_detach_only_the_view_client_and_pass_the_same_key_elsewhere() {
    for key in tmux::VIEW_RETURN_KEYS {
        let args = tmux::view_return_bind_args(&server(), key);
        assert_eq!(&args[..2], ["-S", "/tmp/paneview-spec.sock"]);
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "-n" && pair[1] == key)
        );
        let words = args.join(" ");
        let predicate = "#{&&:#{@ae_view_client},#{==:#{client_name},#{@ae_view_client}}}";
        let gate = words
            .find(predicate)
            .expect("bind must compare the invoking client");
        let detach = words.find("detach-client").expect("view client returns");
        let passthrough = words
            .find(&format!("send-keys {key}"))
            .expect("other clients keep their key");
        assert!(gate < detach && detach < passthrough);
        assert!(!words.contains("C-c"));
    }
}

#[test]
fn attach_uses_the_proven_session_and_defers_a_guarded_landing_after_the_focus_hook() {
    let args = tmux::view_attach_args(&server(), "$7", "%12", "view-token");
    assert_eq!(&args[..2], ["-S", "/tmp/paneview-spec.sock"]);
    let attach = args
        .iter()
        .position(|word| word == "attach-session")
        .expect("interactive attach");
    assert_eq!(args.get(attach + 1).map(String::as_str), Some("-t"));
    assert!(matches!(
        args.get(attach + 2).map(String::as_str),
        Some("$7" | "$7:")
    ));
    let words = args.join(" ");
    let deferred = words
        .find("run-shell -C")
        .expect("landing follows the focus hook");
    let tail = &words[deferred..];
    assert!(tail.contains("session_id") && tail.contains("$7") && tail.contains("%12"));
    let guard = tail
        .find("if-shell")
        .or_else(|| tail.find("if -F"))
        .expect("membership rechecked");
    let select = tail.find("select-pane").expect("seat selected");
    let detach = tail
        .find("detach-client")
        .expect("failed landing returns instead of opening chat");
    assert!(guard < select && select < detach);
    assert!(words.contains("@ae_view_client") && words.contains("@ae_view_token"));
    assert!(words.contains("view-token"));
}

#[test]
fn release_and_stale_cleanup_are_conditional_on_the_observed_owner_token() {
    for command in [
        tmux::view_release_command("$7", "owner-token"),
        tmux::view_stale_clear_command("$7", "owner-token"),
    ] {
        assert!(command.contains("$7:"));
        assert!(command.contains("#{==:#{@ae_view_token},owner-token}"));
        assert!(command.contains("@ae_view_client"));
        assert!(command.contains("@ae_view_token"));
        let gate = command.find("#{==:").expect("ownership predicate");
        let unset = command
            .find("set -u")
            .or_else(|| command.find("set-option -u"))
            .expect("marker cleanup");
        assert!(gate < unset);
    }
}

#[test]
fn restoration_is_guarded_and_previous_targets_are_scoped_to_the_original_session() {
    let command = tmux::view_restore_command("$7", "uuid-seven", "@9", "%12", "@8", "%13");
    assert!(command.contains("@ae_session_uuid") && command.contains("uuid-seven"));
    assert!(command.contains("window_id") && command.contains("pane_id"));
    assert!(command.contains("@9") && command.contains("%12"));
    assert!(command.contains("select-pane -t $7:@9.%13"));
    assert!(command.contains("select-window -t $7:@8"));
    assert!(!command.contains("select-pane -t %13"));
    assert!(!command.contains("select-window -t @8"));
    let gate = command
        .find("if-shell")
        .or_else(|| command.find("if -F"))
        .expect("restore condition");
    assert!(gate < command.find("select-pane").expect("restore action"));
}

#[test]
fn a_queued_pane_handoff_stops_app_shortcuts_before_the_terminal_is_given_away() {
    use crate::console::input::Key;
    use crate::console::needs::SeatRef;

    let mut app = super::App::new(None, None, None, None);
    app.fleeted = true;
    app.pane_view = Some(super::paneview::Ask {
        session: "demo".to_owned(),
        uuid: "drawn-incarnation".to_owned(),
        dir: "/unused/paneview-spec".into(),
        seat: SeatRef {
            slot: "main".to_owned(),
            name: "lead".to_owned(),
        },
        label: "lead".to_owned(),
    });
    let result = super::take_keys(&mut app, vec![(Key::Text(b"qq".to_vec()), Instant::now())]);
    assert!(
        result.is_some(),
        "keys after a seat click must not quit the app"
    );
    assert!(
        app.pane_view.is_some(),
        "the requested handoff remains queued"
    );
    assert!(app.armed.is_none(), "no app shortcut was armed");
    assert!(
        app.deferred.is_empty(),
        "discarded keys must not replay later"
    );
}

#[test]
fn returning_from_a_pane_discards_old_decoded_deferred_keys_too() {
    use crate::console::input::{Key, Keys};

    let mut app = super::App::new(None, None, None, None);
    app.fleeted = true;
    let old = Instant::now();
    app.resumed_at = Some(old + Duration::from_secs(1));
    app.deferred.push((Key::Text(b"qq".to_vec()), old));
    let (_send, wakes) = std::sync::mpsc::channel();
    let mut keys = Keys::app();
    let result = super::drain(&mut app, &mut keys, &wakes, &mut None);
    assert!(
        result.is_some(),
        "old deferred quit keys must not run after return"
    );
    assert!(app.armed.is_none());
    assert!(app.deferred.is_empty());
}

#[test]
fn returning_from_a_pane_preserves_new_decoded_deferred_keys() {
    use crate::console::input::{Key, Keys};

    let mut app = super::App::new(None, None, None, None);
    app.fleeted = true;
    let resumed = Instant::now();
    app.resumed_at = Some(resumed);
    app.deferred
        .push((Key::Text(b"q".to_vec()), resumed + Duration::from_secs(1)));
    let (_send, wakes) = std::sync::mpsc::channel();
    let mut keys = Keys::app();
    let result = super::drain(&mut app, &mut keys, &wakes, &mut None);
    assert!(result.is_some(), "the first fresh q only arms quit");
    assert!(
        app.armed.is_some(),
        "keys typed after return retain their app action"
    );
    assert!(app.deferred.is_empty());
}
