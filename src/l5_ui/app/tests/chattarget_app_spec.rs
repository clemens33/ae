//! Frozen acceptance for the chat target in `ae app` (slice chattarget).
//!
//! Author chattarget-rev (Anthropic), frozen against base da5c18be0 and the
//! driver's plan `.local/chattarget/plan.md`. The driver installs it
//! byte-identically as `src/l5_ui/app/tests/chattarget_app_spec.rs`, wired by
//! `#[path = "chattarget_app_spec.rs"] mod chattarget_app_spec;` in
//! `src/l5_ui/app/tests/o7app_spec.rs`, so it reuses that file's private
//! socket rig: four running sessions `api`, `b`, `c` and the orchestrator,
//! the real reducer, renderer, fleet reader and submit admission, and only
//! the final external delivery replaced by the recording sink
//! `Rig::deliveries` as `(seat, body)`.
//!
//! Oracle: the lead brief `.local/brief-chattarget.md` (requirements 1-3),
//! orchestrator ruling 95 and the plan's key choice (Alt+o, `ESC o`). The
//! label text comes from `crate::console::target::label`, whose words the
//! unit spec `chattarget_spec.rs` pins independently; this file proves the
//! app draws that one shared label, in which colour, and that both sends
//! reach only the right seat.

#![allow(
    clippy::disallowed_methods,
    reason = "tests read and write their own private fixtures"
)]

use super::{ID, Instant, Key, MAIN, ORCH, PANEL, Rig, cell, take_keys, text};
use crate::console::target::{Target, label};
use std::time::Duration;

/// Deliver whatever the composers queued, reap each flight, and return
/// every `(seat, body)` the sink saw once `want` arrived or 3 s passed.
fn delivered(rig: &mut Rig, want: usize) -> Vec<(String, String)> {
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        let _ = rig.app.deliver();
        let _ = rig.app.land();
        let got = rig.deliveries.lock().unwrap().clone();
        if (got.len() >= want && rig.app.flight.is_none()) || Instant::now() > until {
            return got;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Alt+o as the terminal sends it, decoded by the app's own key reader.
fn alt_o(rig: &mut Rig) {
    let mut keys = crate::console::input::Keys::app();
    let keyed = keys.feed(b"\x1bo", Instant::now());
    assert!(
        take_keys(&mut rig.app, keyed).is_some(),
        "Alt+o never quits"
    );
    rig.flush();
}

/// The notices of lane `name` whose body holds every word of `words`.
fn notices(rig: &Rig, name: &str, words: &[&str]) -> usize {
    rig.app
        .notices
        .iter()
        .filter(|(lane, item)| lane == name && words.iter().all(|w| item.body.contains(w)))
        .count()
}

/// Every notice, in any lane, that announces a target switch.
fn switches(rig: &Rig) -> Vec<String> {
    rig.app
        .notices
        .iter()
        .filter(|(_, item)| item.body.contains("Target: "))
        .map(|(lane, item)| format!("{lane}: {}", item.body))
        .collect()
}

/// Start writing the selected session's main column and type `body`.
fn write(rig: &mut Rig, body: &[u8]) {
    rig.key(Key::Enter);
    assert!(rig.app.composing(), "Enter starts writing");
    rig.text(body);
}

#[test]
fn a_warned_ask_waits_for_a_fresh_enter_and_then_reaches_only_its_own_lead() {
    let mut rig = Rig::new(Some("api"));
    write(&mut rig, b"see D44 now");
    rig.key(Key::Enter);
    assert!(
        delivered(&mut rig, 1).is_empty(),
        "the first Enter sends nothing"
    );
    assert_eq!(rig.draft("api", ID), "see D44 now", "the draft is kept");
    assert!(rig.app.composing(), "and stays in the composer");
    assert_eq!(
        notices(&rig, "api", &["D44", "Enter", "Esc", "Alt+o"]),
        1,
        "one warning line in the target's lane: {:?}",
        rig.app.notices
    );
    rig.key(Key::Enter);
    assert_eq!(
        delivered(&mut rig, 1),
        [("lead".to_owned(), "see D44 now".to_owned())],
        "the fresh Enter sends once, to the current target's lead"
    );
    assert_eq!(rig.draft("api", ID), "");
}

#[test]
fn esc_on_a_warning_keeps_writing_and_the_draft() {
    let mut rig = Rig::new(Some("api"));
    write(&mut rig, b"see C3 later");
    rig.key(Key::Enter);
    assert_eq!(notices(&rig, "api", &["Enter", "Esc"]), 1);
    rig.key(Key::Escape);
    assert!(rig.app.composing(), "Esc on a warning keeps editing");
    assert_eq!(rig.draft("api", ID), "see C3 later");
    rig.key(Key::Enter);
    assert_eq!(
        notices(&rig, "api", &["Enter", "Esc"]),
        2,
        "the dismissed confirmation is asked again"
    );
    assert!(delivered(&mut rig, 1).is_empty());
}

#[test]
fn alt_o_reroutes_the_warned_draft_to_the_orchestrator_lead_and_never_to_the_target() {
    let mut rig = Rig::new(Some("api"));
    // The orchestrator is another running session: a whole-word name match.
    write(&mut rig, b"ask orchestrator about it");
    rig.key(Key::Enter);
    alt_o(&mut rig);
    assert_eq!(
        delivered(&mut rig, 1),
        [("olead".to_owned(), "ask orchestrator about it".to_owned())],
        "one ask, through the orchestrator's own pair"
    );
    assert_eq!(rig.draft("api", ID), "", "the rerouted draft left api");

    // The current pair's seat prefix is not carried to the orchestrator.
    write(&mut rig, b"@colead tell D9 now");
    rig.key(Key::Enter);
    alt_o(&mut rig);
    assert_eq!(
        delivered(&mut rig, 2),
        [
            ("olead".to_owned(), "ask orchestrator about it".to_owned()),
            ("olead".to_owned(), "tell D9 now".to_owned()),
        ]
    );
}

#[test]
fn a_replaced_orchestrator_refuses_the_reroute_and_gives_the_draft_back() {
    let mut rig = Rig::new(Some("api"));
    write(&mut rig, b"see D44");
    rig.key(Key::Enter);
    assert_eq!(notices(&rig, "api", &["Alt+o"]), 1);
    // Another incarnation takes the name after the offer was drawn; no
    // fleet read follows, so only a fresh proof at submit can notice.
    rig.command(&format!(
        "set-option -t ={ORCH}: @ae_session_uuid 0199c0de-3333-4890-abcd-ef0123456789"
    ));
    alt_o(&mut rig);
    assert!(
        delivered(&mut rig, 1).is_empty(),
        "no delivery to any seat, and no fallback to the target"
    );
    assert_eq!(rig.draft("api", ID), "see D44", "the source draft returns");
    assert!(
        rig.app
            .notices
            .iter()
            .any(|(_, item)| item.body.contains("refused") && item.body.contains("orchestrator")),
        "the refusal names the orchestrator: {:?}",
        rig.app.notices
    );
}

#[test]
fn the_orchestrator_target_the_own_name_and_a_short_name_never_warn() {
    let mut rig = Rig::new(Some("api"));
    write(&mut rig, b"api is this session");
    rig.key(Key::Enter);
    assert_eq!(
        delivered(&mut rig, 1),
        [("lead".to_owned(), "api is this session".to_owned())]
    );

    // Lead ruling: `b` is a running session, but a name under 3 characters
    // never triggers.
    rig.text(b"ask b about it");
    rig.key(Key::Enter);
    assert_eq!(
        delivered(&mut rig, 2).last(),
        Some(&("lead".to_owned(), "ask b about it".to_owned())),
        "a short name is no match"
    );
    rig.key(Key::Escape);

    rig.open();
    rig.key(Key::Enter);
    assert_eq!(
        rig.app.writer.as_ref().map(|w| w.console.name()),
        Some(ORCH)
    );
    rig.text(b"D44 and b");
    rig.key(Key::Enter);
    let got = delivered(&mut rig, 3);
    assert_eq!(
        got.last(),
        Some(&("olead".to_owned(), "D44 and b".to_owned())),
        "the orchestrator is never guarded: {got:?}"
    );
    assert_eq!(notices(&rig, ORCH, &["Enter", "Esc"]), 0);
}

#[test]
fn without_an_orchestrator_only_the_second_enter_is_offered() {
    let mut rig = Rig::new(Some("api"));
    // The same name, live: it warns, and the second Enter sends it.
    write(&mut rig, b"ask orchestrator now");
    rig.key(Key::Enter);
    assert_eq!(notices(&rig, "api", &["orchestrator", "Enter", "Esc"]), 1);
    rig.key(Key::Enter);
    assert_eq!(
        delivered(&mut rig, 1),
        [("lead".to_owned(), "ask orchestrator now".to_owned())]
    );
    rig.command(&format!("kill-session -t ={ORCH}"));
    rig.refresh(&[]);
    // Stopped, the same name is no match.
    rig.text(b"ask orchestrator later");
    rig.key(Key::Enter);
    assert_eq!(
        delivered(&mut rig, 2).last(),
        Some(&("lead".to_owned(), "ask orchestrator later".to_owned()))
    );
    assert_eq!(notices(&rig, "api", &["orchestrator", "Enter", "Esc"]), 1);
    rig.text(b"see D44");
    rig.key(Key::Enter);
    assert_eq!(notices(&rig, "api", &["D44", "Enter", "Esc"]), 1);
    assert_eq!(
        notices(&rig, "api", &["D44", "Alt+o"]),
        0,
        "no reroute offered"
    );
    alt_o(&mut rig);
    assert_eq!(delivered(&mut rig, 3).len(), 2, "Alt+o is inert");
    assert_eq!(rig.draft("api", ID), "see D44");
    rig.key(Key::Enter);
    assert_eq!(
        delivered(&mut rig, 3),
        [
            ("lead".to_owned(), "ask orchestrator now".to_owned()),
            ("lead".to_owned(), "ask orchestrator later".to_owned()),
            ("lead".to_owned(), "see D44".to_owned()),
        ]
    );
}

#[test]
fn both_composers_draw_the_shared_label_in_their_own_colour_also_with_theme_off() {
    let mut rig = Rig::new(Some("api"));
    rig.open();
    let main_label = label(&Target::Session("api".to_owned()));
    let orch_label = label(&Target::Orchestrator(ORCH.to_owned()));
    let buffer = rig.frame(160, 45);
    let panel_x = cell(&buffer, PANEL).expect("panel lane").0;
    let (mx, my) = cell(&buffer, &main_label)
        .unwrap_or_else(|| panic!("main label {main_label:?}: {}", text(&buffer)));
    let (ox, oy) = cell(&buffer, &orch_label)
        .unwrap_or_else(|| panic!("panel label {orch_label:?}: {}", text(&buffer)));
    assert!(
        mx < panel_x && ox >= panel_x,
        "each label in its own column"
    );
    assert!(my > cell(&buffer, MAIN).unwrap().1, "below the main lane");
    assert!(oy > cell(&buffer, PANEL).unwrap().1, "below the panel lane");
    let ordinary = buffer[(mx, my)].fg;
    assert_ne!(
        buffer[(ox, oy)].fg,
        ordinary,
        "the orchestrator wears a colour of its own"
    );

    // The ordinary label follows its own session's look.
    assert_ne!(
        crate::theme::Palette::DARCULA.title,
        crate::theme::Palette::WARM.title
    );
    let mut warm = crate::theme::Look::DEFAULT;
    warm.palette = crate::theme::Palette::WARM;
    rig.app.looks.insert("api".to_owned(), (Some(warm), None));
    let buffer = rig.frame(160, 45);
    let (mx, my) = cell(&buffer, &main_label).expect("main label");
    assert_ne!(buffer[(mx, my)].fg, ordinary, "the session's own colour");

    // theme = off: the words carry the difference without colour.
    for name in ["api", ORCH] {
        let mut off = crate::theme::Look::DEFAULT;
        off.drawn = false;
        rig.app.looks.insert(name.to_owned(), (Some(off), None));
    }
    let shown = text(&rig.frame(160, 45));
    assert!(
        shown.contains(&main_label) && shown.contains(&orch_label),
        "{shown}"
    );
}

#[test]
fn each_target_switch_prints_one_notice_and_redraws_print_none() {
    let mut rig = Rig::new(Some("api"));
    assert_eq!(
        switches(&rig),
        Vec::<String>::new(),
        "the first target is seeded"
    );
    rig.text(b"2");
    assert_eq!(rig.app.model.selected(), Some("b"));
    assert_eq!(
        notices(&rig, "b", &["Target: ", "b"]),
        1,
        "{:?}",
        switches(&rig)
    );
    for _ in 0..3 {
        let _ = rig.frame(160, 45);
    }
    rig.refresh(&[]);
    rig.lanes();
    rig.text(b"2");
    assert_eq!(
        switches(&rig).len(),
        1,
        "no switch, no notice: {:?}",
        switches(&rig)
    );

    // Rig::open asserts the api lane; with b selected, open the panel by hand.
    rig.text(b"O");
    rig.lanes();
    let shown = text(&rig.frame(160, 45));
    assert!(
        shown.contains("B_SENTINEL") && shown.contains(PANEL),
        "O shows the panel beside b: {shown}"
    );
    assert_eq!(
        notices(&rig, ORCH, &["Target: ", "orchestrator"]),
        1,
        "the panel taking the keys switches the target: {:?}",
        switches(&rig)
    );
    rig.click_word("B_SENTINEL");
    assert_eq!(notices(&rig, "b", &["Target: "]), 2, "{:?}", switches(&rig));
    assert_eq!(switches(&rig).len(), 3, "{:?}", switches(&rig));
    let _ = rig.frame(160, 45);
    assert_eq!(switches(&rig).len(), 3, "a frame is no switch");
}
