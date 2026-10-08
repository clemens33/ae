//! Frozen appopen pure contract. Oracle: lead rulings + measured tmux epochs.
//! New API file is intentionally compile-RED on the pre-feature base; the
//! sibling existing-API live file supplies independent behavioural RED.

use ae::app::model::{Act, Key, Model, browse_keys};
use ae::console::input;
use ae::console::needs::SeatRef;
use ae::console::open::{self, Move, Refusal, Target};
use ae::inventory::ServerId;
use ae::tmux::ObservedClient;

const UUID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";

fn client(name: &str, session: &str, pane: &str, activity: Option<u64>) -> ObservedClient {
    ObservedClient {
        name: name.to_owned(),
        session: session.to_owned(),
        pane: pane.to_owned(),
        activity,
    }
}

#[test]
fn only_app_pane_viewers_participate_in_newest_client_selection() {
    let rows = [
        client("older", "home", "%3", Some(9)),
        client("newest", "home", "%3", Some(10)),
        client("bystander", "elsewhere", "%99", Some(999)),
    ];
    assert_eq!(
        open::mover(&rows, "%3", "target"),
        Ok(Move::Client("newest".to_owned()))
    );
    let reverse = [rows[2].clone(), rows[1].clone(), rows[0].clone()];
    assert_eq!(
        open::mover(&reverse, "%3", "target"),
        Ok(Move::Client("newest".to_owned())),
        "listing order cannot pick the client"
    );
}

#[test]
fn no_viewer_and_tied_foreign_viewers_refuse_without_an_arbitrary_choice() {
    assert_eq!(open::mover(&[], "%3", "target"), Err(Refusal::NoViewer));
    assert_eq!(
        open::mover(&[client("other", "other", "%4", Some(99))], "%3", "target"),
        Err(Refusal::NoViewer)
    );
    let rows = [
        client("a", "home", "%3", Some(10)),
        client("b", "home", "%3", Some(10)),
    ];
    assert_eq!(open::mover(&rows, "%3", "target"), Err(Refusal::Tied));
    assert!(Refusal::Tied.line("scout").contains("scout"));
}

#[test]
fn unknown_activity_is_below_every_known_epoch_and_cannot_break_a_tie() {
    let known = [
        client("unknown", "home", "%3", None),
        client("epoch-zero", "home", "%3", Some(0)),
    ];
    assert_eq!(
        open::mover(&known, "%3", "target"),
        Ok(Move::Client("epoch-zero".to_owned()))
    );
    let unknown = [
        client("a", "home", "%3", None),
        client("b", "home", "%3", None),
    ];
    assert_eq!(open::mover(&unknown, "%3", "target"), Err(Refusal::Tied));
    assert_eq!(
        open::mover(&unknown[..1], "%3", "target"),
        Ok(Move::Client("a".to_owned()))
    );
}

#[test]
fn here_requires_a_viewer_but_never_names_a_client_even_when_activity_ties() {
    let rows = [
        client("a", "home", "%3", Some(10)),
        client("b", "home", "%3", Some(10)),
    ];
    assert_eq!(
        open::mover(&rows, "%3", "home"),
        Ok(Move::Here),
        "lead B2 explicit shortcut"
    );
    assert_eq!(open::mover(&rows, "%99", "home"), Err(Refusal::NoViewer));
    let linked = [
        client("a", "target", "%3", Some(11)),
        client("b", "origin", "%3", Some(10)),
    ];
    assert_eq!(open::mover(&linked, "%3", "target"), Ok(Move::Here));
    assert_eq!(
        open::mover(&linked, "%3", "origin"),
        Ok(Move::Client("a".to_owned()))
    );
}

fn target() -> Target {
    Target {
        seat: SeatRef {
            slot: "spawned.0".to_owned(),
            name: "scout".to_owned(),
        },
        session_id: "$7".to_owned(),
        pane: "%12".to_owned(),
        uuid: UUID.to_owned(),
    }
}

#[test]
fn explicit_client_extends_the_legacy_guarded_select_and_preserves_chat_bytes() {
    let target = target();
    let server = ServerId::Ambient;
    let legacy = open::select_args(&server, &target).expect("legacy target proven");
    assert_eq!(
        open::select_args_via(&server, &target, None),
        Some(legacy.clone())
    );
    let moved =
        open::select_args_via(&server, &target, Some("/dev/pts/4")).expect("client grammar");
    assert_eq!(
        &moved[..5],
        &legacy[..5],
        "single-hash identity guard is shared"
    );
    assert_eq!(
        moved[5],
        "switch-client -c '/dev/pts/4' -t $7 ; select-window -t %12 ; select-pane -t %12 ; display-message -p ae-open:selected"
    );
    assert_eq!(moved[6], legacy[6]);
    assert!(
        !moved[4].contains("##{"),
        "direct app commands have no display-menu expansion layer"
    );
    for client in ["/dev/ttys003", "client-35", "/dev/pts/4"] {
        assert!(open::select_args_via(&server, &target, Some(client)).is_some());
    }
}

#[test]
fn unsafe_client_names_and_damaged_target_identity_never_build_a_select() {
    for client in [
        "", " ", "a b", "a;b", "a'b", "a\nb", "a|b", "$(id)", "`id`", "a\\b", "é",
    ] {
        assert!(
            open::select_args_via(&ServerId::Ambient, &target(), Some(client)).is_none(),
            "client grammar rejects {client:?}"
        );
    }
    for part in ["pane", "session", "slot", "name", "uuid"] {
        let mut t = target();
        match part {
            "pane" => t.pane = "%12;next".to_owned(),
            "session" => t.session_id = "$x".to_owned(),
            "slot" => t.seat.slot = "spawned.x".to_owned(),
            "name" => t.seat.name = "scout;next".to_owned(),
            _ => t.uuid.clear(),
        }
        assert!(
            open::select_args_via(&ServerId::Ambient, &t, Some("client-35")).is_none(),
            "broken {part}"
        );
    }
}

#[test]
fn app_outcomes_name_return_path_and_never_claim_an_unconfirmed_move() {
    assert_eq!(
        open::outcome_moved(true, "ae-open:selected\n", "scout", "target"),
        "opened scout in target - prefix h opens its chat, prefix L goes back"
    );
    assert_eq!(
        open::outcome(true, "ae-open:selected\n", "scout"),
        "opened scout - prefix h returns"
    );
    for (ok, stdout) in [
        (true, "ae-open:moved\n"),
        (true, "unknown\n"),
        (false, "ae-open:selected\n"),
    ] {
        assert_eq!(
            open::outcome_moved(ok, stdout, "scout", "target"),
            open::outcome(ok, stdout, "scout")
        );
        assert!(!open::outcome_moved(ok, stdout, "scout", "target").starts_with("opened"));
    }
    for (refusal, why) in [
        (Refusal::NoTmux, "ae app is not running inside tmux"),
        (Refusal::NoViewer, "no tmux client is showing this app"),
        (
            Refusal::Tied,
            "two clients showing this app were active in the same second",
        ),
        (
            Refusal::Stopped("target".to_owned()),
            "target is stopped; ae target resumes it",
        ),
        (
            Refusal::Elsewhere("target".to_owned()),
            "target is not on the tmux server this app runs on, or ae cannot prove it is; run ae target",
        ),
        (
            Refusal::NoSeat("the view changed".to_owned()),
            "the view changed",
        ),
    ] {
        assert_eq!(
            refusal.line("scout"),
            format!("refused: /open scout: {why}; nothing selected")
        );
    }
}

#[test]
fn new_browse_letters_route_to_open_and_focus_while_enter_keeps_composing() {
    for (byte, key, act) in [
        (b'o', Key::Open, Act::Open),
        (b'n', Key::SeatNext, Act::SeatNext),
        (b'p', Key::SeatPrev, Act::SeatPrev),
    ] {
        assert_eq!(browse_keys(&input::Key::Text(vec![byte])), [key]);
        let fleet = ae::app::fleet::Fleet::default();
        assert_eq!(Model::new(&fleet).key(key, &fleet, false, true), act);
    }
    assert_eq!(browse_keys(&input::Key::Enter), [Key::Compose]);
    assert!(
        browse_keys(&input::Key::Pasted(b"onp".to_vec())).is_empty(),
        "paste never becomes navigation keys"
    );
}
