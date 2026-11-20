//! Frozen acceptance for the shared chat-target guard (slice chattarget).
//!
//! Author chattarget-rev (Anthropic), frozen against base da5c18be0 and the
//! driver's plan `.local/chattarget/plan.md`. The driver installs it
//! byte-identically as `src/l5_ui/console/input/chattarget_spec.rs`, wired by
//! `#[cfg(test)] mod chattarget_spec;` beside `inputfeed_spec` in input.rs.
//!
//! Oracle: the brief's objective (lead brief `.local/brief-chattarget.md`,
//! requirements 1 and 3), orchestrator ruling 95 (a suspicious message needs
//! a second Enter, one key sends it to the orchestrator instead), the board
//! id grammar documented in `src/l2_model/desk.rs` (section letter D, C, R,
//! W or L and a number 1 to 99 without a leading zero) and the plan's word
//! rule (letters of any script, digits, `_` and `-` are word constituents).
//! No expected value comes from the unit under test.
//!
//! Every key goes through `Input::chunk`, the production byte path both the
//! chat and the app reach. Each read is stamped `Instant::now()` when it is
//! fed, as a real terminal read is stamped when it returns, so a key in the
//! same read as the warning's Enter predates the warning.
//!
//! Interface this file fixes (the plan names its owners):
//! - `crate::console::target::Target::{Session(String), Orchestrator(String)}`,
//!   the composer's target and its session name.
//! - `crate::console::target::label(&Target) -> String`, the plain (mono)
//!   label text both surfaces draw.
//! - `Input::set_target(Target)`, `Input::set_running(Option<Vec<String>>)`
//!   (the running session names; `None` = unknown) and
//!   `Input::set_redirect(bool)` (a running orchestrator is proven).
//! - `Effect::Redirect { raw, body }`: the one-shot reroute of the draft to
//!   the orchestrator; `raw` is the draft as typed, `body` the ask body
//!   without the current pair's `@seat` prefix.

use super::{ESC_IDLE, Effect, Input, Reading};
use crate::console::target::{Target, label};
use std::thread::sleep;
use std::time::{Duration, Instant};

const ENTER: &[u8] = b"\r";
const ALT_O: &[u8] = b"\x1bo";
const RUNNING: [&str; 7] = ["superset", "quota", "orch-x", "api_2", "b", "ae", "abc"];

/// A fresh terminal read, strictly after everything fed before it.
fn read() -> Instant {
    sleep(Duration::from_millis(2));
    Instant::now()
}

/// An owner input of the lead pair `lead`, `colead`, bound to `target`.
fn owner(target: Target, running: Option<&[&str]>, redirect: bool) -> Input {
    let mut input = Input::new(vec!["lead".to_owned(), "colead".to_owned()]);
    input.set_target(target);
    input.set_running(running.map(|names| names.iter().map(|n| (*n).to_owned()).collect()));
    input.set_redirect(redirect);
    let _ = input.tick(Reading::Owner, Instant::now());
    input
}

/// The ordinary session `superset`, its fleet known, an orchestrator proven.
fn superset() -> Input {
    owner(Target::Session("superset".to_owned()), Some(&RUNNING), true)
}

fn feed(input: &mut Input, bytes: &[u8]) -> Vec<Effect> {
    input.chunk(bytes, read())
}

fn asks(effects: &[Effect]) -> Vec<(String, String)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Ask { seat, body, .. } => Some((seat.clone(), body.clone())),
            _ => None,
        })
        .collect()
}

fn redirects(effects: &[Effect]) -> Vec<(Vec<u8>, String)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Redirect { raw, body } => Some((raw.clone(), body.clone())),
            _ => None,
        })
        .collect()
}

fn prints(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Print(line) => Some(line.clone()),
            _ => None,
        })
        .collect()
}

/// The warning offers Enter and Esc; a warning line is one that does.
fn warnings(effects: &[Effect]) -> Vec<String> {
    prints(effects)
        .into_iter()
        .filter(|line| line.contains("Enter") && line.contains("Esc"))
        .collect()
}

/// `body` typed, then one Enter in a later read: the effects of that Enter.
fn entered(input: &mut Input, body: &[u8]) -> Vec<Effect> {
    assert!(feed(input, body).is_empty(), "typing alone has no effect");
    feed(input, ENTER)
}

/// Assert `effects` is exactly one warning naming `token`, and nothing sent.
fn assert_warned(effects: &[Effect], token: &str, why: &str) {
    let lines = warnings(effects);
    assert_eq!(lines.len(), 1, "{why}: one warning line: {effects:?}");
    assert!(lines[0].contains(token), "{why}: names {token}: {lines:?}");
    assert!(!lines[0].contains('\n'), "{why}: one line: {lines:?}");
    assert!(asks(effects).is_empty(), "{why}: nothing sent: {effects:?}");
    assert!(redirects(effects).is_empty(), "{why}: nothing rerouted");
}

#[test]
fn a_board_id_or_another_running_session_as_a_whole_word_warns() {
    let cases: [(&[u8], &str); 14] = [
        (b"see D44", "D44"),
        (b"C3 first", "C3"),
        (b"(R2)", "R2"),
        (b"about W99.", "W99"),
        (b"L1, then", "L1"),
        (b"D1", "D1"),
        (b"[D7] done?", "D7"),
        (b"ask quota now", "quota"),
        (b"quota's lead", "quota"),
        (b"tell orch-x", "orch-x"),
        (b"api_2 is up", "api_2"),
        (b"@colead see D44", "D44"),
        (b"\x1b[200~line one\nquota\x1b[201~", "quota"),
        (b"plan: C12!", "C12"),
    ];
    for (body, token) in cases {
        let mut input = superset();
        let effects = entered(&mut input, body);
        let why = String::from_utf8_lossy(body).into_owned();
        assert_warned(&effects, token, &why);
    }
}

#[test]
fn nothing_else_warns_and_the_ask_goes_out_on_the_first_enter() {
    let cases: [&[u8]; 20] = [
        b"d44 lower case",
        b"D044 leading zero",
        b"D100 three digits",
        b"D0 zero",
        b"X4 other letter",
        b"ID44",
        b"D44x",
        b"D44_b",
        b"D44-b",
        b"2D4",
        "\u{e9}D44".as_bytes(),
        "D4\u{e9}".as_bytes(),
        b"superset is this session",
        b"quota-2",
        b"xquota",
        b"Quota",
        b"quotas",
        b"zeta is not running",
        b"orch",
        b"plain words only",
    ];
    for body in cases {
        let mut input = superset();
        let effects = entered(&mut input, body);
        let text = String::from_utf8_lossy(body).into_owned();
        assert!(warnings(&effects).is_empty(), "{text}: {effects:?}");
        assert_eq!(
            asks(&effects),
            [("lead".to_owned(), text.clone())],
            "{text}: one ask on the first Enter"
        );
    }
}

#[test]
fn commands_shell_lines_attachments_and_the_orchestrator_are_never_guarded() {
    let mut input = superset();
    let effects = entered(&mut input, b"/close C3");
    assert_eq!(effects, [Effect::Close(Some("C3".to_owned()))]);

    let mut input = superset();
    let effects = entered(&mut input, b"!echo D44 quota");
    assert!(warnings(&effects).is_empty(), "{effects:?}");
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::Shell { line, .. } if line == "echo D44 quota")),
        "{effects:?}"
    );

    let mut input = superset();
    let effects = entered(&mut input, b"/open quota");
    assert!(warnings(&effects).is_empty(), "{effects:?}");
    assert!(asks(&effects).is_empty() && redirects(&effects).is_empty());

    // An attachment atom is opaque: its path segments are not words.
    let mut input = superset();
    input
        .image_path("/tmp/D44/quota.png")
        .expect("one attachment fits");
    let effects = entered(&mut input, b"look");
    assert!(warnings(&effects).is_empty(), "{effects:?}");
    let sent = asks(&effects);
    assert_eq!(sent.len(), 1, "{effects:?}");
    assert!(sent[0].1.contains("/tmp/D44/quota.png"), "{sent:?}");

    // The orchestrator is the right target for board work: no guard at all.
    let mut input = owner(
        Target::Orchestrator("orch".to_owned()),
        Some(&RUNNING),
        true,
    );
    let effects = entered(&mut input, b"D44 and quota");
    assert!(warnings(&effects).is_empty(), "{effects:?}");
    assert_eq!(
        asks(&effects),
        [("lead".to_owned(), "D44 and quota".to_owned())]
    );
}

#[test]
fn a_session_name_matches_only_from_three_characters_on() {
    // Lead ruling: a name shorter than 3 characters never triggers; a
    // common word of 3 or more still does (one Enter is cheap).
    for body in [&b"ask b now"[..], b"b", b"ae is up", b"(ae)"] {
        let mut input = superset();
        let effects = entered(&mut input, body);
        let text = String::from_utf8_lossy(body).into_owned();
        assert!(warnings(&effects).is_empty(), "{text}: {effects:?}");
        assert_eq!(
            asks(&effects),
            [("lead".to_owned(), text.clone())],
            "{text}"
        );
    }
    let mut input = superset();
    let effects = entered(&mut input, b"ask abc now");
    assert_warned(&effects, "abc", "exactly three characters match");
    let mut input = owner(
        Target::Session("superset".to_owned()),
        Some(&["test", "superset"]),
        true,
    );
    let effects = entered(&mut input, b"run the test again");
    assert_warned(
        &effects,
        "test",
        "a common word of four characters still matches",
    );
}

#[test]
fn an_unknown_fleet_still_checks_board_ids_but_never_names() {
    let mut input = owner(Target::Session("superset".to_owned()), None, true);
    let effects = entered(&mut input, b"ask quota now");
    assert_eq!(
        asks(&effects),
        [("lead".to_owned(), "ask quota now".to_owned())]
    );
    let mut input = owner(Target::Session("superset".to_owned()), None, true);
    let effects = entered(&mut input, b"see D44");
    assert_warned(&effects, "D44", "ids need no fleet");
}

#[test]
fn the_warning_names_the_token_the_target_and_every_offer() {
    let mut input = superset();
    let effects = entered(&mut input, b"see D44");
    let line = &warnings(&effects)[0];
    for word in ["D44", "superset", "Enter", "Esc", "Alt+o"] {
        assert!(line.contains(word), "{word} missing: {line}");
    }
    assert_eq!(prints(&effects).len(), 1, "exactly one line: {effects:?}");

    // No proven orchestrator: only the second Enter is offered.
    let mut input = owner(
        Target::Session("superset".to_owned()),
        Some(&RUNNING),
        false,
    );
    let effects = entered(&mut input, b"see D44");
    let line = &warnings(&effects)[0];
    assert!(!line.contains("Alt+o"), "no redirect offered: {line}");
    assert!(line.contains("Enter") && line.contains("Esc"), "{line}");
}

#[test]
fn the_first_enter_keeps_the_draft_and_cursor_and_a_fresh_enter_sends_it_once() {
    let mut input = superset();
    assert!(feed(&mut input, b"see D44 now").is_empty());
    // Cursor four characters left, before " now".
    assert!(feed(&mut input, b"\x1b[D\x1b[D\x1b[D\x1b[D").is_empty());
    let effects = feed(&mut input, ENTER);
    assert_warned(&effects, "D44", "first Enter");
    assert_eq!(input.draft(), "see D44 now", "the draft stays editable");
    // Typing goes in at the kept cursor; the edit changes the decision.
    assert!(feed(&mut input, b"X").is_empty());
    assert_eq!(input.draft(), "see D44X now");
    let effects = feed(&mut input, ENTER);
    assert_eq!(
        asks(&effects),
        [("lead".to_owned(), "see D44X now".to_owned())],
        "D44X is no id: the re-checked draft goes out on its first Enter"
    );

    let mut input = superset();
    let effects = entered(&mut input, b"see D44");
    assert_warned(&effects, "D44", "first Enter");
    let effects = feed(&mut input, ENTER);
    assert_eq!(
        asks(&effects),
        [("lead".to_owned(), "see D44".to_owned())],
        "a fresh Enter confirms: one ask to the current target"
    );
    assert!(warnings(&effects).is_empty());
    assert_eq!(input.draft(), "", "a sent draft leaves the composer");
    assert!(
        feed(&mut input, ENTER).is_empty(),
        "an empty composer sends nothing more"
    );
}

#[test]
fn a_second_enter_or_alt_o_from_the_same_read_never_sends() {
    let mut input = superset();
    let effects = feed(&mut input, b"see D44\r\r");
    assert_warned(&effects, "D44", "a doubled Enter in one read");
    assert_eq!(input.draft(), "see D44");
    let effects = feed(&mut input, ENTER);
    assert_eq!(
        asks(&effects).len(),
        1,
        "a later read confirms: {effects:?}"
    );

    let mut input = superset();
    let effects = feed(&mut input, b"see D44\r\x1bo");
    assert_warned(&effects, "D44", "Alt+o in the warning's read");
    assert_eq!(input.draft(), "see D44");

    let mut input = superset();
    assert!(feed(&mut input, b"see D44").is_empty());
    let effects = feed(&mut input, b"\r\r");
    assert_warned(&effects, "D44", "Enter Enter in one later read");
}

#[test]
fn esc_dismisses_keeps_editing_and_the_next_enter_asks_again() {
    let mut input = superset();
    let effects = entered(&mut input, b"see D44");
    assert_warned(&effects, "D44", "first Enter");
    let esc = read();
    let mut effects = input.chunk(b"\x1b", esc);
    effects.extend(input.idle(esc + ESC_IDLE + Duration::from_millis(5)));
    assert!(asks(&effects).is_empty() && redirects(&effects).is_empty());
    assert!(input.taking(), "Esc keeps the composer");
    assert_eq!(input.draft(), "see D44", "Esc keeps the draft");
    let effects = feed(&mut input, ENTER);
    assert_warned(&effects, "D44", "the confirmation was dismissed");
}

#[test]
fn any_edit_drops_the_pending_confirmation_even_when_the_bytes_return() {
    let mut input = superset();
    let effects = entered(&mut input, b"see D44");
    assert_warned(&effects, "D44", "first Enter");
    assert!(feed(&mut input, b"x\x7f").is_empty());
    assert_eq!(input.draft(), "see D44");
    let effects = feed(&mut input, ENTER);
    assert_warned(&effects, "D44", "an edit re-arms the check");
}

#[test]
fn a_target_change_drops_the_pending_confirmation() {
    let mut input = superset();
    let effects = entered(&mut input, b"see D44");
    assert_warned(&effects, "D44", "first Enter");
    input.set_target(Target::Session("other".to_owned()));
    let effects = feed(&mut input, ENTER);
    assert_warned(&effects, "D44", "a new target needs a new choice");
    assert!(warnings(&effects)[0].contains("other"), "{effects:?}");
}

#[test]
fn each_new_line_gets_its_own_decision() {
    let mut input = superset();
    let _ = entered(&mut input, b"see D44");
    assert_eq!(asks(&feed(&mut input, ENTER)).len(), 1);
    let effects = entered(&mut input, b"C3 again");
    assert_warned(&effects, "C3", "a confirmed line confirms nothing after it");
    let effects = entered(&mut input, b" more");
    assert_warned(
        &effects,
        "C3",
        "an edited draft that still matches warns again",
    );
    assert_eq!(
        asks(&feed(&mut input, ENTER)),
        [("lead".to_owned(), "C3 again more".to_owned())],
        "and its own fresh Enter sends it"
    );
}

#[test]
fn alt_o_reroutes_the_warned_draft_once_and_sends_nothing_to_the_target() {
    let mut input = superset();
    let effects = entered(&mut input, b"see D44");
    assert_warned(&effects, "D44", "first Enter");
    let effects = feed(&mut input, ALT_O);
    assert_eq!(
        redirects(&effects),
        [(b"see D44".to_vec(), "see D44".to_owned())]
    );
    assert!(asks(&effects).is_empty(), "{effects:?}");
    assert_eq!(input.draft(), "", "the rerouted draft leaves the composer");
    assert!(feed(&mut input, ALT_O).is_empty(), "one-shot");
    assert!(feed(&mut input, ENTER).is_empty(), "nothing left to send");

    // The current pair's seat prefix stays behind.
    let mut input = superset();
    let effects = entered(&mut input, b"@colead see D44");
    assert_warned(&effects, "D44", "first Enter");
    let effects = feed(&mut input, ALT_O);
    assert_eq!(
        redirects(&effects),
        [(b"@colead see D44".to_vec(), "see D44".to_owned())]
    );
    assert!(asks(&effects).is_empty());
}

#[test]
fn alt_o_without_a_pending_warning_or_without_an_orchestrator_does_nothing() {
    let mut input = superset();
    assert!(feed(&mut input, b"hello").is_empty());
    assert!(feed(&mut input, ALT_O).is_empty(), "no warning: inert");
    assert_eq!(input.draft(), "hello", "and no byte joins the draft");
    assert_eq!(
        asks(&feed(&mut input, ENTER)),
        [("lead".to_owned(), "hello".to_owned())]
    );

    let mut input = owner(
        Target::Session("superset".to_owned()),
        Some(&RUNNING),
        false,
    );
    let effects = entered(&mut input, b"see D44");
    assert_warned(&effects, "D44", "first Enter");
    let effects = feed(&mut input, ALT_O);
    assert!(redirects(&effects).is_empty() && asks(&effects).is_empty());
    assert_eq!(input.draft(), "see D44");
    assert_eq!(
        asks(&feed(&mut input, ENTER)),
        [("lead".to_owned(), "see D44".to_owned())],
        "the second Enter is still offered"
    );
}

#[test]
fn the_label_names_the_target_and_marks_only_the_real_orchestrator() {
    let ordinary = label(&Target::Session("superset".to_owned()));
    assert!(ordinary.contains("superset"), "{ordinary}");
    assert!(!ordinary.contains("[orchestrator]"), "{ordinary}");

    let orchestrator = label(&Target::Orchestrator("orch".to_owned()));
    assert!(orchestrator.contains("[orchestrator]"), "{orchestrator}");

    // A session merely named orchestrator is ordinary: the mark is the role.
    let namesake = label(&Target::Session("orchestrator".to_owned()));
    assert!(namesake.contains("orchestrator"), "{namesake}");
    assert!(!namesake.contains("[orchestrator]"), "{namesake}");
    assert_ne!(namesake, orchestrator);

    // A hostile name from a meta is drawn inert.
    let hostile = label(&Target::Session("a\u{1b}[31mb\nc".to_owned()));
    assert!(
        !hostile.contains('\u{1b}') && !hostile.contains('\n'),
        "{hostile:?}"
    );
}
