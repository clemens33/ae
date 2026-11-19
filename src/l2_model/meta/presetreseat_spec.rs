//! Frozen #296 part B pre-exec identity acceptance (OpenAI/gpt-6.1-sol).
//! Oracle: brief invariant 3, lead P1/P2/P2a. Assumed API: plan-r2 `SeatMove.pair`
//! and `PairIdentity { client, config_home, config_home_base }`. No `_run` repair
//! can satisfy these assertions: only the pure replacement is observed.
use super::{Conversation, PairIdentity, SeatMove};

const OLD: &str = "11111111-1111-4111-8111-111111111111";
const NEW: &str = "22222222-2222-4222-8222-222222222222";

fn fixture(home: &str, base: Option<&str>) -> String {
    let base = base.map_or_else(String::new, |base| {
        format!("config_home_base.main={base}\n")
    });
    format!(
        "session=sample\nseat.main=lead\nprofile.main=old\nagent_bin.main=claude\nclient.main=old-client\nharness_session.main={OLD}\ncapture_floor.main=100\nlaunch_id.main=old-token\nlaunch_time.main=200\nconfig_home.main={home}\n{base}observed_model.main=old-model\nobserved_model_pin.main=old-pin\nseat.spawned.0=other\nprofile.spawned.0=other-profile\nclient.spawned.0=other-client\nconfig_home.spawned.0=/other-store\n"
    )
}

fn value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    let rows: Vec<_> = text
        .lines()
        .filter_map(|line| line.strip_prefix(&prefix))
        .collect();
    assert!(rows.len() <= 1, "one row per identity fact: {key}: {text}");
    rows.first().copied()
}

fn assert_common(text: &str, home: &str, base: Option<&str>) {
    assert_eq!(value(text, "profile.main"), Some("preset"));
    assert_eq!(value(text, "agent_bin.main"), Some("claude"));
    assert_eq!(value(text, "launch_id.main"), Some("new-token"));
    assert_eq!(value(text, "client.main"), Some("approved-client"));
    assert_eq!(value(text, "config_home.main"), Some(home));
    assert_eq!(value(text, "config_home_base.main"), base);
    assert_eq!(value(text, "seat.main"), Some("lead"));
    assert_eq!(value(text, "launch_time.main"), None);
    assert_eq!(value(text, "observed_model.main"), None);
    assert_eq!(value(text, "observed_model_pin.main"), None);
    assert!(text.contains("seat.spawned.0=other\nprofile.spawned.0=other-profile\nclient.spawned.0=other-client\nconfig_home.spawned.0=/other-store\n"),
        "other seat bytes survive: {text}");
}

fn paired<'a>(
    conversation: Conversation<'a>,
    home: &'a str,
    base: Option<&'a str>,
) -> SeatMove<'a> {
    SeatMove {
        profile: "preset",
        binary: "claude",
        launch_id: "new-token",
        conversation,
        pair: Some(PairIdentity {
            client: "approved-client",
            config_home: home,
            config_home_base: base,
        }),
    }
}

#[test]
fn presetreseat_fresh_explicit_identity_is_complete_in_the_one_pre_exec_replacement() {
    let current = fixture("implicit:/source/.claude", Some("/source"))
        .replace("agent_bin.main=claude\n", "agent_bin.main=codex\n");
    let moved = super::reseated(
        &current,
        "main",
        &paired(
            Conversation::Fresh {
                id: NEW,
                capture_floor: 500,
            },
            "/target",
            None,
        ),
    );
    assert_common(&moved, "/target", None);
    assert_eq!(value(&moved, "harness_session.main"), Some(NEW));
    assert_eq!(value(&moved, "capture_floor.main"), Some("500"));
    assert_eq!(
        value(&moved, "harness_session_prior.main"),
        Some(format!("codex:{OLD}").as_str())
    );
}

#[test]
fn presetreseat_fresh_implicit_identity_includes_its_base_before_any_start() {
    let moved = super::reseated(
        &fixture("/source", None),
        "main",
        &paired(
            Conversation::Fresh {
                id: NEW,
                capture_floor: 500,
            },
            "implicit:/target/.claude",
            Some("/target"),
        ),
    );
    assert_common(&moved, "implicit:/target/.claude", Some("/target"));
    assert_eq!(value(&moved, "harness_session.main"), Some(NEW));
    assert_eq!(value(&moved, "capture_floor.main"), Some("500"));
}

#[test]
fn presetreseat_carried_identity_moves_client_home_and_base_with_the_same_conversation() {
    for (home, base) in [
        ("/target", None),
        ("implicit:/target/.claude", Some("/target")),
    ] {
        let moved = super::reseated(
            &fixture("implicit:/source/.claude", Some("/source")),
            "main",
            &paired(
                Conversation::Carried {
                    config_home: home,
                    config_home_base: base,
                },
                home,
                base,
            ),
        );
        assert_common(&moved, home, base);
        assert_eq!(value(&moved, "harness_session.main"), Some(OLD));
        assert_eq!(value(&moved, "capture_floor.main"), Some("100"));
        assert_eq!(value(&moved, "harness_session_prior.main"), None);
    }
}

#[test]
fn presetreseat_kept_identity_retains_store_mode_base_and_conversation() {
    for (home, base) in [
        ("/target", None),
        ("implicit:/target/.claude", Some("/target")),
    ] {
        let current = fixture(home, base);
        let moved = super::reseated(&current, "main", &paired(Conversation::Kept, home, base));
        assert_common(&moved, home, base);
        assert_eq!(value(&moved, "harness_session.main"), Some(OLD));
        assert_eq!(value(&moved, "capture_floor.main"), Some("100"));
        assert_eq!(value(&moved, "harness_session_prior.main"), None);
    }
}

#[test]
fn presetreseat_bare_replacements_keep_the_existing_exact_document_for_all_three_arms() {
    let current = fixture("implicit:/source/.claude", Some("/source"));
    let common_start = "session=sample\nseat.main=lead\nprofile.main=preset\nagent_bin.main=claude\nharness_session.main=";
    let other = "seat.spawned.0=other\nprofile.spawned.0=other-profile\nclient.spawned.0=other-client\nconfig_home.spawned.0=/other-store\n";
    // Byte parity is expressly required for pair=None. These literals state
    // the existing bare row contract; they are not computed by reseated.
    for (conversation, expected) in [
        (
            Conversation::Fresh {
                id: NEW,
                capture_floor: 500,
            },
            format!(
                "{common_start}{NEW}\ncapture_floor.main=500\nlaunch_id.main=new-token\n{other}harness_session_prior.main=claude:{OLD}\n"
            ),
        ),
        (
            Conversation::Carried {
                config_home: "/target",
                config_home_base: None,
            },
            format!(
                "{common_start}{OLD}\ncapture_floor.main=100\nlaunch_id.main=new-token\nconfig_home.main=/target\n{other}"
            ),
        ),
        (
            Conversation::Kept,
            format!(
                "{common_start}{OLD}\ncapture_floor.main=100\nlaunch_id.main=new-token\nconfig_home.main=implicit:/source/.claude\nconfig_home_base.main=/source\n{other}"
            ),
        ),
    ] {
        let moved = super::reseated(
            &current,
            "main",
            &SeatMove {
                profile: "preset",
                binary: "claude",
                launch_id: "new-token",
                conversation,
                pair: None,
            },
        );
        assert_eq!(moved, expected, "bare identity row bytes changed");
    }
}

#[test]
fn presetreseat_pair_collapses_duplicate_invalid_client_rows_while_bare_removes_them() {
    let current = format!(
        "{}client.main=\nclient.main=other-client\n",
        fixture("/target", None)
    );
    let paired = super::reseated(
        &current,
        "main",
        &paired(Conversation::Kept, "/target", None),
    );
    assert_common(&paired, "/target", None);
    let bare = super::reseated(
        &current,
        "main",
        &SeatMove {
            profile: "preset",
            binary: "claude",
            launch_id: "new-token",
            conversation: Conversation::Kept,
            pair: None,
        },
    );
    assert_eq!(value(&bare, "client.main"), None);
    assert_eq!(value(&bare, "client.spawned.0"), Some("other-client"));
}

#[test]
fn presetreseat_guard_writes_nothing_for_a_replaced_seat_and_the_matching_guard_publishes_identity()
{
    let current = fixture("/source", None);
    let movement = paired(
        Conversation::Fresh {
            id: NEW,
            capture_floor: 500,
        },
        "/target",
        None,
    );
    assert!(super::seat_move_for(&current, "main", "different-agent", &movement).is_none());
    let next = super::seat_move_for(&current, "main", "lead", &movement)
        .unwrap_or_else(|| panic!("matching guard must yield one replacement"));
    assert_common(&next, "/target", None);
}
