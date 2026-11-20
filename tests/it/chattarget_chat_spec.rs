//! Frozen acceptance for the chat target in `ae chat` (slice chattarget).
//!
//! Author chattarget-rev (Anthropic), frozen against base da5c18be0 and the
//! driver's plan `.local/chattarget/plan.md`. The driver installs it
//! byte-identically as `tests/it/chattarget_chat_spec.rs`, wired by
//! `#[path = "chattarget_chat_spec.rs"] mod chattarget_chat_spec;` in
//! `tests/it/console_p36_spec.rs`, so it reuses that file's real console:
//! `ae _console` in a private tmux server, typed through bracketed paste and
//! `send-keys`, judged by `capture-pane` and the session's own journal.
//!
//! Oracle: the lead brief `.local/brief-chattarget.md` (requirements 1 and
//! 3) and orchestrator ruling 95. The rig's session is ordinary and no
//! orchestrator exists, so only the second Enter may be offered. The label
//! words come from `ae::console::target::label`, pinned independently by the
//! unit spec; this file proves the real pane draws them next to the input,
//! on the row directly above the unchanged prompt (lead ruling).

use ae::console::target::{Target, label};
use ae::events::Event;

use super::Rig;

/// The asks the console recorded in the session's journal.
fn asks(rig: &Rig) -> usize {
    rig.tool
        .events()
        .lines()
        .filter_map(|row| Event::parse_line(row).ok())
        .filter(|event| event.actor == "console:local" && event.action == "ask")
        .count()
}

/// The screen rows that read as the warning for `token`.
fn warnings(screen: &str, token: &str) -> usize {
    screen
        .lines()
        .filter(|row| row.contains(token) && row.contains("Enter") && row.contains("Esc"))
        .count()
}

#[test]
fn the_real_chat_shows_its_target_beside_the_input() {
    let rig = Rig::new("ctlabel", 120, 32);
    let want = label(&Target::Session(rig.name.clone()));
    assert!(
        want.contains(&rig.name),
        "the label names the session: {want}"
    );
    assert!(!want.contains("[orchestrator]"), "{want}");
    let screen = rig.wait("the target label is drawn", |screen| {
        let rows: Vec<&str> = screen.lines().collect();
        // Lead ruling: the label is the row directly above the prompt, and
        // the prompt row keeps its bytes.
        rows.iter()
            .enumerate()
            .any(|(at, row)| row.trim_end() == "to lead>" && at > 0 && rows[at - 1].contains(&want))
    });
    assert!(!screen.contains("[orchestrator]"), "{screen}");
}

#[test]
fn a_board_id_needs_a_second_enter_in_the_real_chat_and_offers_no_reroute_without_an_orchestrator()
{
    let rig = Rig::new("ctguard", 120, 32);
    rig.paste("see D44");
    rig.key("Enter");
    let screen = rig.wait("one warning line", |screen| warnings(screen, "D44") == 1);
    assert!(
        !screen
            .lines()
            .any(|row| row.contains("D44") && row.contains("Alt+o")),
        "no orchestrator, no reroute offered: {screen}"
    );
    assert_eq!(asks(&rig), 0, "the first Enter records no ask");
    assert!(
        screen.contains("to lead> see D44"),
        "the draft stays: {screen}"
    );
    rig.submit("see D44");
}

#[test]
fn esc_dismisses_the_real_warning_and_the_next_enter_asks_again() {
    let rig = Rig::new("ctesc", 120, 32);
    rig.paste("see C3");
    rig.key("Enter");
    rig.wait("first warning", |screen| warnings(screen, "C3") == 1);
    rig.key("Escape");
    std::thread::sleep(std::time::Duration::from_millis(300));
    rig.key("Enter");
    let screen = rig.wait("asked again", |screen| warnings(screen, "C3") == 2);
    assert!(
        screen.contains("to lead> see C3"),
        "Esc kept the draft: {screen}"
    );
    assert_eq!(asks(&rig), 0);
}
