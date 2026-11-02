//! Frozen acceptance spec for slice dechist: a decision declaration that
//! stopped standing stays in chat history, whole, at its own journal
//! position, never reading as still asking.
//!
//! Oracle: the brief rulings plus docs/chat.md — never the current code.
//! Binds NO new product names: every assertion reads public `fold`,
//! `Printed::step` or `ae chat` bytes, so this file compiles before the
//! change (red at run time) and after it (green).
//!
//! Install: copy byte-identical to `tests/it/dechist_spec.rs` and add
//! `mod dechist_spec;` to `tests/it/main.rs` between `declared_task_spec`
//! and `deliver` (the mod line is install mechanics, owned by the driver).

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns fixtures and reads their effects"
)]

use ae::board::Observation;
use ae::console::lane::{Body, Kind, Lane, Seat, fold};
use ae::console::view::Printed;
use ae::events::Event;

use super::board::{claude_roster, plant_session, plant_transcript, rig, user};

const T0: &str = "2026-09-30T06:00:00Z";
const T1: &str = "2026-09-30T06:01:00Z";
const T2: &str = "2026-09-30T06:02:00Z";
const T3: &str = "2026-09-30T06:03:00Z";
const T4: &str = "2026-09-30T06:04:00Z";
const T5: &str = "2026-09-30T06:05:00Z";

/// A shaped reason: options, prose, evidence pointer.
const Q1: &str =
    "Deploy now | wait for review. Recommend now because the window closes. Details: memo deploy.";
const Q2: &str = "Pick a name\tfor v2";
const Q3: &str = "Trust the new folder | keep it quarantined. Recommend trust.";
const HQ: &str = "Pick\x1b[2J now\r | wait\x07 soon.\nWhy\u{0} not?";

fn esc(text: &str) -> String {
    let mut out = String::new();
    ae::json::escape_into(text, &mut out);
    out
}

/// A journal line. Only `text` (and the short fields beside it) is escaped;
/// the caller's stamps, actors and actions are safe literals.
fn ev(ts: &str, actor: &str, action: &str, to: &str, reference: &str, text: &str) -> Event {
    let (to, reference, text) = (esc(to), esc(reference), esc(text));
    Event::parse_line(&format!(
        r#"{{"ts":"{ts}","actor":"{actor}","action":"{action}","target":"{to}","ref":"{reference}","summary":"{text}"}}"#
    ))
    .expect("fixture parses")
}

fn declared(ts: &str, actor: &str, state: &str, reason: &str) -> Event {
    ev(ts, actor, "state", "", state, reason)
}

fn said(ts: &str, actor: &str, text: &str) -> Event {
    ev(ts, actor, "chat", "", "r-1", text)
}

const ASK_ID: &str = "ae-20260930T060100Z-0000abcd";

/// The human's console ask to `to`: ends standing currency, declares nothing.
fn asked(ts: &str, to: &str, text: &str) -> Event {
    ev(ts, "console:local", "ask", to, ASK_ID, text)
}

/// A chat-bridge message to `to`: ends standing currency, declares nothing.
fn bridged(ts: &str, to: &str, text: &str) -> Event {
    ev(ts, "telegram:42", "send", to, "r-1", text)
}

/// A watchdog wait challenge: not a declaration, so it neither opens nor
/// breaks a run.
fn challenge(ts: &str, to: &str) -> Event {
    ev(
        ts,
        "watchdog",
        "wait-challenge",
        to,
        "r-1",
        "still waiting on you?",
    )
}

fn pair() -> [Seat; 2] {
    [("main", "lead"), ("worker.0", "colead")].map(|(slot, name)| Seat {
        slot: slot.to_owned(),
        name: name.to_owned(),
        profile: None,
    })
}

fn lane_of(events: &[Event]) -> Lane {
    fold("s", &pair(), events, &Observation::default(), 0, &|_| {
        Body::OldCore
    })
}

/// The lane micros of journal stamp `ts`: the stamp's epoch second widened.
fn micros(ts: &str) -> i64 {
    Event::parse_line(&format!(
        r#"{{"ts":"{ts}","actor":"x","action":"chat","summary":"t"}}"#
    ))
    .expect("stamp parses")
    .ts
    .epoch()
    .saturating_mul(1_000_000)
}

/// Items carrying `body` that are NOT the standing card: the past entries.
fn pasts<'a>(lane: &'a Lane, body: &str) -> Vec<&'a ae::console::lane::Item> {
    lane.items
        .iter()
        .filter(|item| item.body == body && !matches!(item.kind, Kind::Card { .. }))
        .collect()
}

fn print(lane: &Lane) -> String {
    Printed::default().step(lane, 0, true)
}

/// The body rows under `header` in `out`: lines after it up to the blank line.
fn block_after<'a>(out: &'a str, header: &str) -> Vec<&'a str> {
    let mut lines = out.lines().skip_while(|line| *line != header);
    assert_eq!(lines.next(), Some(header), "header present:\n{out}");
    lines.take_while(|line| !line.is_empty()).collect()
}

// --- fold: past entries from journal state events ---

#[test]
fn superseded_declarations_stay_whole_at_first_appearance_in_history_order() {
    // The brief's real shape: same text twice (a challenge between), changed
    // text, then working.
    let events = [
        declared(T0, "lead", "waiting-user", Q1),
        challenge(T1, "lead"),
        declared(T2, "lead", "waiting-user", Q1),
        declared(T3, "lead", "waiting-user", Q2),
        said(T4, "lead", "a note between"),
        declared(T5, "lead", "working", "moving on"),
    ];
    let lane = lane_of(&events);
    let first = pasts(&lane, Q1);
    assert_eq!(first.len(), 1, "re-declared text is one entry");
    assert_eq!(first[0].micros, micros(T0), "at first appearance");
    assert_eq!(first[0].body, Q1, "whole");
    let second = pasts(&lane, Q2);
    assert_eq!(second.len(), 1, "changed text keeps the older one");
    assert_eq!(second[0].micros, micros(T3));
    assert!(
        !lane
            .items
            .iter()
            .any(|item| matches!(item.kind, Kind::Card { .. })),
        "nothing still stands"
    );
    let at = |body: &str| lane.items.iter().position(|item| item.body == body);
    assert!(
        at(Q1) < at(Q2) && at(Q2) < at("a note between"),
        "own journal positions, in order"
    );
}

#[test]
fn identical_text_still_standing_is_one_card_and_no_past_duplicate() {
    // PIN: passes before and after; locks ruling 2 against the new machinery.
    let lane = lane_of(&[
        declared(T0, "lead", "waiting-user", Q1),
        declared(T1, "lead", "waiting-user", Q1),
    ]);
    let cards: Vec<_> = lane.items.iter().filter(|item| item.body == Q1).collect();
    assert_eq!(cards.len(), 1);
    assert_eq!(
        cards[0].kind,
        Kind::Card {
            seat: "lead".to_owned()
        }
    );
    assert_eq!(cards[0].micros, micros(T1), "standing keeps the latest");
}

#[test]
fn changed_text_while_asking_keeps_the_older_run_as_past() {
    let lane = lane_of(&[
        declared(T0, "lead", "waiting-user", Q1),
        declared(T1, "lead", "waiting-user", Q2),
    ]);
    let old = pasts(&lane, Q1);
    assert_eq!(old.len(), 1);
    assert_eq!(old[0].micros, micros(T0));
    let now: Vec<_> = lane.items.iter().filter(|item| item.body == Q2).collect();
    assert_eq!(now.len(), 1);
    assert!(matches!(now[0].kind, Kind::Card { .. }));
}

#[test]
fn identical_text_broken_by_another_state_is_two_episodes() {
    let lane = lane_of(&[
        declared(T0, "lead", "waiting-user", Q1),
        declared(T1, "lead", "working", "moving on"),
        declared(T2, "lead", "waiting-user", Q1),
        declared(T3, "lead", "working", "moving on"),
    ]);
    let entries = pasts(&lane, Q1);
    assert_eq!(entries.len(), 2, "a broken run is a new episode");
    let stamps: Vec<i64> = entries.iter().map(|item| item.micros).collect();
    assert_eq!(stamps, [micros(T0), micros(T2)]);
}

#[test]
fn two_same_second_episodes_print_as_two_past_cards() {
    let events = [
        declared(T0, "lead", "waiting-user", Q1),
        declared(T0, "lead", "working", "moving on"),
        declared(T0, "lead", "waiting-user", Q1),
        declared(T0, "lead", "working", "moving on"),
    ];
    let lane = lane_of(&events);
    assert_eq!(pasts(&lane, Q1).len(), 2);
    let out = print(&lane);
    assert_eq!(out.matches("DECISION lead · closed").count(), 2, "{out}");
}

#[test]
fn only_a_seats_own_declarations_count() {
    let lane = lane_of(&[
        declared(T0, "scout", "waiting-user", "scout asks"),
        declared(T1, "scout", "working", "never mind"),
        declared(T2, "colead", "waiting-user", "colead asks"),
        declared(T3, "colead", "working", "done"),
    ]);
    assert!(
        !lane.items.iter().any(|item| item.body == "scout asks"),
        "a non-lead-pair seat stays out"
    );
    let entries = pasts(&lane, "colead asks");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].micros, micros(T2));
}

#[test]
fn a_cleared_human_prompt_still_leaves_no_history_row() {
    // PIN: NEEDS YOU history is out of scope; today's vanish stays.
    let raised = ev(
        T0,
        "watchdog",
        "human-prompt",
        "colead",
        "r-1",
        "Trust this folder?",
    );
    let cleared = ev(
        T1,
        "watchdog",
        "human-prompt-cleared",
        "colead",
        "r-1",
        "gone",
    );
    assert!(lane_of(&[raised, cleared]).items.is_empty());
    let raised = ev(
        T0,
        "watchdog",
        "human-prompt",
        "colead",
        "r-1",
        "Trust this folder?",
    );
    let lane = lane_of(std::slice::from_ref(&raised));
    assert_eq!(lane.items.len(), 1, "a standing prompt still shows");
    assert!(matches!(lane.items[0].kind, Kind::NeedsYou { .. }));
}

#[test]
fn empty_summaries_yield_no_entry_and_break_no_run() {
    let lane = lane_of(&[
        declared(T0, "lead", "waiting-user", Q1),
        declared(T1, "lead", "waiting-user", "   "),
        declared(T2, "lead", "waiting-user", Q1),
        declared(T3, "lead", "working", "moving on"),
    ]);
    let entries = pasts(&lane, Q1);
    assert_eq!(entries.len(), 1, "a contentless record breaks nothing");
    assert_eq!(entries[0].micros, micros(T0));
    assert!(
        !lane.items.iter().any(|item| item.body.trim().is_empty()),
        "no header-only card"
    );
    let lane = lane_of(&[
        declared(T0, "lead", "waiting-user", ""),
        declared(T1, "lead", "working", "moving on"),
    ]);
    assert!(lane.items.is_empty());
}

// --- a card ended with no seat declaration: every non-standing run is past ---

#[test]
fn a_console_ask_ends_the_card_without_a_declaration_and_the_past_stays() {
    let lane = lane_of(&[
        declared(T0, "lead", "waiting-user", Q1),
        asked(T1, "lead", "ship it"),
    ]);
    let entries = pasts(&lane, Q1);
    assert_eq!(entries.len(), 1, "no declaration ends history");
    assert_eq!(entries[0].micros, micros(T0));
    assert!(
        !lane
            .items
            .iter()
            .any(|item| matches!(item.kind, Kind::Card { .. })),
        "the ask ended standing"
    );
    let at = |body: &str| lane.items.iter().position(|item| item.body == body);
    assert_eq!(at(Q1), Some(0), "past at T0");
    assert_eq!(at("ship it"), Some(1), "ask below it");
}

#[test]
fn a_bridge_message_ends_the_card_without_a_declaration() {
    let lane = lane_of(&[
        declared(T0, "lead", "waiting-user", Q1),
        bridged(T1, "lead", "ship it?"),
    ]);
    let entries = pasts(&lane, Q1);
    assert_eq!(entries.len(), 1, "no declaration ends history");
    assert_eq!(entries[0].micros, micros(T0));
    assert!(
        !lane
            .items
            .iter()
            .any(|item| matches!(item.kind, Kind::Card { .. })),
        "the bridge message ended standing"
    );
    let at = |body: &str| lane.items.iter().position(|item| item.body == body);
    assert_eq!(at(Q1), Some(0), "past at T0");
    assert_eq!(at("ship it?"), Some(1), "bridge message below it");
}

#[test]
fn a_crossed_wait_challenge_without_redeclaration_leaves_a_past_entry() {
    let lane = lane_of(&[
        declared(T0, "lead", "waiting-user", Q1),
        challenge(T1, "lead"),
    ]);
    let entries = pasts(&lane, Q1);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].micros, micros(T0));
    assert!(
        !lane
            .items
            .iter()
            .any(|item| matches!(item.kind, Kind::Card { .. })),
        "a crossed challenge ends standing, not history"
    );
}

#[test]
fn an_ask_between_identical_declarations_keeps_one_standing_card() {
    // PIN: passes before and after; locks that an ask neither breaks a run
    // nor duplicates a standing card.
    let lane = lane_of(&[
        declared(T0, "lead", "waiting-user", Q1),
        asked(T1, "lead", "ship it"),
        declared(T2, "lead", "waiting-user", Q1),
    ]);
    let cards: Vec<_> = lane.items.iter().filter(|item| item.body == Q1).collect();
    assert_eq!(cards.len(), 1);
    assert!(matches!(cards[0].kind, Kind::Card { .. }));
    assert_eq!(cards[0].micros, micros(T2));
}

#[test]
fn an_ask_between_changed_declarations_keeps_the_older_as_past() {
    let lane = lane_of(&[
        declared(T0, "lead", "waiting-user", Q1),
        asked(T1, "lead", "ship it"),
        declared(T2, "lead", "waiting-user", Q2),
    ]);
    let old = pasts(&lane, Q1);
    assert_eq!(old.len(), 1);
    assert_eq!(old[0].micros, micros(T0));
    let now: Vec<_> = lane.items.iter().filter(|item| item.body == Q2).collect();
    assert_eq!(now.len(), 1);
    assert!(matches!(now[0].kind, Kind::Card { .. }));
}

// --- plain chat view: header, body path, fold, width, hostile bytes ---

const PAST_HEAD: &str = "## 06:00:00 DECISION lead · closed";
const OPEN_HEAD: &str = "## 06:00:00 DECISION lead · waiting-user";

#[test]
fn past_header_marks_closed_and_body_matches_the_standing_path_byte_for_byte() {
    let past = lane_of(&[
        declared(T0, "lead", "waiting-user", Q1),
        declared(T1, "lead", "working", "moving on"),
    ]);
    let open = lane_of(&[declared(T0, "lead", "waiting-user", Q1)]);
    let (past_out, open_out) = (print(&past), print(&open));
    assert!(past_out.contains(&format!("{PAST_HEAD}\n")), "{past_out}");
    assert!(open_out.contains(&format!("{OPEN_HEAD}\n")), "{open_out}");
    assert!(!past_out.contains("waiting-user"), "{past_out}");
    let head = past_out
        .lines()
        .find(|line| line.starts_with("## "))
        .expect("past header");
    assert!(
        !head.to_lowercase().contains("nswer"),
        "never reads as answered: {head}"
    );
    assert_eq!(
        past_out.replacen(PAST_HEAD, "", 1),
        open_out.replacen(OPEN_HEAD, "", 1),
        "same laid-out body via the same path"
    );
    assert_eq!(
        block_after(&past_out, PAST_HEAD),
        block_after(&open_out, OPEN_HEAD),
        "same body rows"
    );
}

#[test]
fn a_long_past_card_folds_like_a_standing_one_and_more_prints_it_whole() {
    let options: Vec<String> = (1..=35).map(|n| format!("opt{n}")).collect();
    let reason = format!("Decide now. {} End.", options.join(" | "));
    let past = lane_of(&[
        declared(T0, "lead", "waiting-user", &reason),
        declared(T1, "lead", "working", "moving on"),
    ]);
    let open = lane_of(&[declared(T0, "lead", "waiting-user", &reason)]);
    let mut folded = Printed::default();
    folded.set_fold(true);
    folded.set_width(Some(80));
    let past_out = folded.step(&past, 0, true);
    let marker = past_out
        .lines()
        .find(|line| line.contains("/more"))
        .expect("a long past card folds")
        .to_owned();
    assert!(marker.contains("(/more 1)"), "{marker}");
    let mut unfolded = Printed::default();
    unfolded.set_fold(true);
    unfolded.set_width(Some(80));
    let open_out = unfolded.step(&open, 0, true);
    let open_marker = open_out
        .lines()
        .find(|line| line.contains("/more"))
        .expect("standing folds the same way");
    assert_eq!(marker, open_marker, "same fold rule");
    let whole = folded.more("1", &past).expect("past /more works");
    assert!(
        whole.contains("opt35") && !whole.contains("/more"),
        "{whole}"
    );
}

#[test]
fn a_past_card_lays_out_at_narrow_width_with_every_word_in_order() {
    for reason in ["Go now | wait a bit. Act soon.", "Steps. 1 merge | 2 wait"] {
        let lane = lane_of(&[
            declared(T0, "lead", "waiting-user", reason),
            declared(T1, "lead", "working", "moving on"),
        ]);
        let mut narrow = Printed::default();
        narrow.set_width(Some(20));
        let out = narrow.step(&lane, 0, true);
        assert!(out.contains(PAST_HEAD), "{reason}: {out}");
        let mut at = 0;
        for word in reason.split_whitespace().filter(|word| *word != "|") {
            let found = out[at..].find(word).expect("every word kept");
            at += found + word.len();
        }
    }
}

#[test]
fn hostile_bytes_in_a_past_card_neutralise_exactly_like_a_standing_one() {
    let past = lane_of(&[
        declared(T0, "lead", "waiting-user", HQ),
        declared(T1, "lead", "working", "moving on"),
    ]);
    let open = lane_of(&[declared(T0, "lead", "waiting-user", HQ)]);
    let (past_out, open_out) = (print(&past), print(&open));
    assert!(past_out.contains(&format!("{PAST_HEAD}\n")), "{past_out:?}");
    assert_eq!(
        past_out.replacen(PAST_HEAD, "", 1),
        open_out.replacen(OPEN_HEAD, "", 1),
        "same neutraliser"
    );
    for (name, out) in ["past", "open"].iter().zip([&past_out, &open_out]) {
        assert!(
            !out.chars()
                .any(|ch| ch.is_control() && ch != '\n' && ch != '\t'),
            "{name}: {out:?}"
        );
    }
}

// --- follow: closed once, no reprint, fresh start shows history ---

const CLOSED: &str = "-- closed: DECISION lead · waiting-user is no longer standing";

#[test]
fn follow_prints_closed_once_and_only_the_question_it_never_showed() {
    let asking = [
        declared(T0, "lead", "waiting-user", Q1),
        said(T1, "lead", "a note"),
        declared(T2, "lead", "waiting-user", Q1),
    ];
    let mut journal = asking.to_vec();
    journal.extend([
        declared(T3, "lead", "waiting-user", Q2),
        declared(T4, "lead", "working", "moving on"),
    ]);
    let mut follow = Printed::default();
    let first = follow.step(&lane_of(&asking), 0, true);
    assert!(
        first.contains("## 06:02:00 DECISION lead · waiting-user"),
        "{first}"
    );
    assert_eq!(first.matches("window closes").count(), 1);
    assert_eq!(first.matches("for v2").count(), 0);
    let second = follow.step(&lane_of(&journal), 0, true);
    assert_eq!(second.matches(CLOSED).count(), 1, "{second}");
    assert_eq!(
        second.matches("window closes").count(),
        0,
        "the printed question is not reprinted as past: {second}"
    );
    assert_eq!(
        second.matches("for v2").count(),
        1,
        "the never-shown question prints once as past: {second}"
    );
    let third = follow.step(&lane_of(&journal), 0, true);
    assert_eq!(third, "", "then silence");
    assert_eq!(
        [first, second, third]
            .iter()
            .map(|out| out.matches(CLOSED).count())
            .sum::<usize>(),
        1,
        "closed once across all passes"
    );
}

#[test]
fn a_redeclaration_while_standing_keeps_todays_close_and_reprint() {
    // PIN: standing follow behaviour is unchanged; the run stays standing,
    // so no past entry appears.
    let mut follow = Printed::default();
    let first = follow.step(
        &lane_of(&[declared(T0, "lead", "waiting-user", Q1)]),
        0,
        true,
    );
    assert!(first.contains(OPEN_HEAD));
    let journal = [
        declared(T0, "lead", "waiting-user", Q1),
        declared(T1, "lead", "waiting-user", Q1),
    ];
    let second = follow.step(&lane_of(&journal), 0, true);
    assert!(second.contains(CLOSED), "today's close: {second}");
    assert!(second.contains("DECISION lead · waiting-user"), "{second}");
    assert!(
        !second.contains("· closed"),
        "last run stays standing: {second}"
    );
}

#[test]
fn twin_suppression_is_scoped_to_the_seat() {
    let journal = [
        declared(T0, "lead", "waiting-user", Q1),
        declared(T1, "lead", "working", "moving on"),
        declared(T2, "colead", "waiting-user", Q1),
    ];
    let out = print(&lane_of(&journal));
    assert!(out.contains(PAST_HEAD), "{out}");
    assert!(
        out.contains("## 06:02:00 DECISION colead · waiting-user"),
        "{out}"
    );
    assert_eq!(out.matches("window closes").count(), 2, "{out}");
}

#[test]
fn a_fresh_start_shows_past_cards_in_history_order() {
    let journal = [
        declared(T0, "lead", "waiting-user", Q1),
        declared(T1, "lead", "waiting-user", Q1),
        declared(T2, "lead", "waiting-user", Q2),
        declared(T3, "lead", "working", "moving on"),
        declared(T4, "colead", "waiting-user", Q3),
    ];
    let out = print(&lane_of(&journal));
    let heads = [
        "## 06:00:00 DECISION lead · closed",
        "## 06:02:00 DECISION lead · closed",
        "## 06:04:00 DECISION colead · waiting-user",
    ];
    let mut at = 0;
    for head in heads {
        let found = out[at..].find(head).expect("every card shows");
        at += found + head.len();
    }
}

#[test]
fn a_question_asked_again_after_closing_prints_as_news() {
    let closed = [
        declared(T0, "lead", "waiting-user", Q1),
        declared(T1, "lead", "working", "moving on"),
    ];
    let mut follow = Printed::default();
    let first = follow.step(&lane_of(&closed), 0, true);
    assert!(first.contains(PAST_HEAD), "{first}");
    let mut journal = closed.to_vec();
    journal.push(declared(T2, "lead", "waiting-user", Q1));
    let second = follow.step(&lane_of(&journal), 0, true);
    assert!(
        second.contains("## 06:02:00 DECISION lead · waiting-user"),
        "standing news, not a reprint: {second}"
    );
    assert!(!second.contains(CLOSED), "nothing stood before: {second}");
    assert_eq!(
        [first, second]
            .iter()
            .map(|out| out.matches("window closes").count())
            .sum::<usize>(),
        2
    );
}

// --- same-second ties: a past card keeps its journal position ---

#[test]
fn same_second_console_ask_sorts_after_the_past_card() {
    let lane = lane_of(&[
        declared(T0, "lead", "waiting-user", Q1),
        asked(T0, "lead", "ship it"),
    ]);
    let bodies: Vec<&str> = lane.items.iter().map(|item| item.body.as_str()).collect();
    assert_eq!(bodies, [Q1, "ship it"], "journal order within the second");
}

#[test]
fn same_second_say_rows_keep_journal_order_around_a_past_card() {
    let card_first = lane_of(&[
        declared(T0, "lead", "waiting-user", Q1),
        said(T0, "lead", "note"),
        declared(T1, "lead", "working", "on"),
    ]);
    let bodies: Vec<&str> = card_first
        .items
        .iter()
        .map(|item| item.body.as_str())
        .collect();
    assert_eq!(bodies, [Q1, "note"], "declaration first in the journal");
    let card_last = lane_of(&[
        said(T0, "lead", "note"),
        declared(T0, "lead", "waiting-user", Q1),
        declared(T1, "lead", "working", "on"),
    ]);
    let bodies: Vec<&str> = card_last
        .items
        .iter()
        .map(|item| item.body.as_str())
        .collect();
    assert_eq!(bodies, ["note", Q1], "declaration last in the journal");
}

#[test]
fn same_second_standing_card_keeps_todays_order() {
    // PIN: N1 moves past cards only; standing order is frozen by ruling 2.
    let lane = lane_of(&[
        declared(T0, "lead", "waiting-user", Q1),
        said(T0, "lead", "note"),
    ]);
    let bodies: Vec<&str> = lane.items.iter().map(|item| item.body.as_str()).collect();
    assert_eq!(
        bodies,
        ["note", Q1],
        "standing still appends after journal rows"
    );
}

#[test]
fn follow_a_console_ask_ends_the_card_closed_once_no_reprint() {
    let asking = [declared(T0, "lead", "waiting-user", Q1)];
    let mut journal = asking.to_vec();
    journal.extend([
        declared(T1, "lead", "waiting-user", Q2),
        asked(T2, "lead", "ship it"),
    ]);
    let mut follow = Printed::default();
    let first = follow.step(&lane_of(&asking), 0, true);
    assert!(first.contains(OPEN_HEAD), "{first}");
    assert_eq!(first.matches("window closes").count(), 1);
    let second = follow.step(&lane_of(&journal), 0, true);
    assert_eq!(second.matches(CLOSED).count(), 1, "{second}");
    assert!(second.contains("## 06:02:00 you → lead"), "{second}");
    assert_eq!(
        second.matches("window closes").count(),
        0,
        "printed question not reprinted: {second}"
    );
    assert_eq!(
        second.matches("for v2").count(),
        1,
        "unshown question prints once as past: {second}"
    );
    assert_eq!(follow.step(&lane_of(&journal), 0, true), "", "then silence");
}

// --- `ae chat` end to end: history, attention, hostile bytes, read-only ---

const LEAD_ID: &str = "0199c0de-1234-4890-abcd-ef01234567a1";
const COLEAD_ID: &str = "0199c0de-1234-4890-abcd-ef01234567a2";
const SCOUT_ID: &str = "0199c0de-1234-4890-abcd-ef01234567a3";
const SESSION_ID: &str = "0199c0de-aaaa-4890-abcd-ef01234567a4";

fn journal_line(
    ts: &str,
    actor: &str,
    action: &str,
    target: &str,
    reference: &str,
    text: &str,
) -> String {
    let (actor, target, reference, text) = (esc(actor), esc(target), esc(reference), esc(text));
    format!(
        r#"{{"ts":"{ts}","actor":"{actor}","action":"{action}","target":"{target}","ref":"{reference}","summary":"{text}"}}"#
    )
}

fn chat(root: &std::path::Path, tail: &[&str]) -> (Option<i32>, String, String) {
    let mut runner = super::cli::ae();
    runner
        .env("HOME", root)
        .env("AE_HOME", root)
        .env_remove("TMUX_PANE")
        .stdin(std::process::Stdio::null())
        .arg("chat")
        .args(tail);
    let child = runner
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("the ae binary should run");
    let out = super::cli::bounded(child, std::time::Duration::from_secs(30))
        .expect("the ae binary did not exit within 30 s");
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    (out.status.code(), text(&out.stdout), text(&out.stderr))
}

/// A lead-pair session: lead asked twice (re-declared, then changed its
/// question) and moved on; colead still asks.
fn history_rig(tag: &str, journal: &[String]) -> (std::path::PathBuf, std::path::PathBuf) {
    let root = rig(tag);
    let store = root.join("claude");
    plant_transcript(
        &store,
        "work",
        LEAD_ID,
        &[user("2026-09-30T05:50:00Z", "lead typed")],
    );
    plant_transcript(
        &store,
        "work",
        COLEAD_ID,
        &[user("2026-09-30T05:51:00Z", "colead typed")],
    );
    plant_transcript(
        &store,
        "work",
        SCOUT_ID,
        &[user("2026-09-30T05:52:00Z", "worker words")],
    );
    let roster = format!(
        "session_id={SESSION_ID}\nlayout=lead-pair\n{}{}{}",
        claude_roster("main", "lead", LEAD_ID, &store),
        claude_roster("worker.0", "colead", COLEAD_ID, &store),
        claude_roster("worker.1", "scout", SCOUT_ID, &store)
    );
    let dir = plant_session(&root, "one", &roster);
    std::fs::write(dir.join("events.jsonl"), journal.join("\n") + "\n").expect("journal");
    (root, dir)
}

#[test]
fn chat_history_keeps_superseded_decisions_whole_and_needs_you_ignores_them() {
    let journal = [T0, T1, T2, T3, T4]
        .into_iter()
        .zip([
            ("lead", "waiting-user", Q1),
            ("lead", "waiting-user", Q1),
            ("lead", "waiting-user", Q2),
            ("lead", "working", "moving on"),
            ("colead", "waiting-user", Q3),
        ])
        .map(|(ts, (actor, state, reason))| journal_line(ts, actor, "state", "", state, reason))
        .collect::<Vec<_>>();
    let (root, dir) = history_rig("dechist-history", &journal);
    let before = std::fs::read(dir.join("events.jsonl")).expect("journal");
    let (code, stdout, stderr) = chat(&root, &["one"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stderr, "");
    let heads = [
        "## 06:00:00 DECISION lead · closed",
        "## 06:02:00 DECISION lead · closed",
        "## 06:04:00 DECISION colead · waiting-user",
    ];
    let mut at = 0;
    for head in heads {
        let found = stdout[at..].find(head).expect("every card shows");
        at += found + head.len();
    }
    for words in [
        "Deploy now",
        "wait for review.",
        "Recommend now because the window closes.",
        "Details: memo deploy.",
        "Pick a name\tfor v2",
    ] {
        assert!(stdout.contains(words), "{words}: {stdout}");
    }
    assert_eq!(
        stdout.matches("DECISION lead · closed").count(),
        2,
        "{stdout}"
    );
    assert!(stdout.contains("-- needs you: 1 seat"), "{stdout}");
    assert!(
        stdout.contains("colead (lead pair) · waiting-user"),
        "{stdout}"
    );
    // Anchored: bare "lead (lead pair)" is a substring of the colead row.
    assert!(
        !stdout.contains("\n  lead (lead pair)"),
        "a past card never counts as needing you: {stdout}"
    );
    let section = stdout
        .split_once("-- needs you: ")
        .map_or("", |(_, rest)| rest);
    assert!(
        !section.contains("window closes"),
        "the past reason stays out of needs-you: {stdout}"
    );
    assert_eq!(
        std::fs::read(dir.join("events.jsonl")).expect("journal after"),
        before,
        "chat only reads"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn chat_neutralises_control_bytes_in_a_past_card() {
    let journal = [
        journal_line(T0, "lead", "state", "", "waiting-user", HQ),
        journal_line(T1, "lead", "state", "", "working", "moving on"),
        journal_line(T2, "colead", "state", "", "waiting-user", Q3),
    ];
    let (root, _) = history_rig("dechist-hostile", &journal);
    let (code, stdout, stderr) = chat(&root, &["one"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains(PAST_HEAD), "{stdout:?}");
    assert!(
        !stdout
            .chars()
            .any(|ch| ch.is_control() && ch != '\n' && ch != '\t'),
        "{stdout:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn chat_card_ended_by_your_ask_stays_past_and_attention_is_untouched() {
    let journal = vec![
        journal_line(T0, "lead", "state", "", "waiting-user", Q1),
        journal_line(
            T1,
            "console:local",
            "ask",
            "lead",
            "ae-20260930T060100Z-0000abcd",
            "ship it",
        ),
        journal_line(T2, "colead", "state", "", "waiting-user", Q3),
    ];
    let (root, _) = history_rig("dechist-ask", &journal);
    let (code, stdout, stderr) = chat(&root, &["one"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let past = "## 06:00:00 DECISION lead · closed";
    let ask_row = "## 06:01:00 you → lead";
    let (found_past, found_ask) = (stdout.find(past), stdout.find(ask_row));
    assert!(
        found_past.is_some() && found_ask.is_some(),
        "past at T0, ask below it: {stdout}"
    );
    assert!(found_past < found_ask, "{stdout}");
    assert!(
        stdout.contains("## 06:02:00 DECISION colead · waiting-user"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("DECISION lead · waiting-user"),
        "no standing card: {stdout}"
    );
    // Attention is verdict-side and untouched by the slice: the ask ends the
    // lane card while the declaration verdict still stands (pre-existing
    // divergence, out of scope). Pin today's rows so the slice adds nothing.
    assert!(stdout.contains("-- needs you: 2 seats"), "{stdout}");
    assert!(
        stdout.contains("lead (lead pair) · waiting-user · /open lead · declared since 06:00:00"),
        "{stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
