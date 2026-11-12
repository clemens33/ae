//! Frozen acceptance spec for slice dechist, app surface: past decision
//! cards render in `ae app` with a closed header and the standing card's
//! laid-out body rows.
//!
//! Oracle: the brief rulings. Binds NO new product names: lanes come from
//! public `fold`, rendering from `super::lane::rows`, so this file compiles
//! before the change (red at run time) and after it (green).
//!
//! Install: copy byte-identical to `src/app/dechist_spec.rs` and add
//! `#[cfg(test)] mod dechist_spec;` to `src/app.rs` beside the other spec
//! mods (the mod line is install mechanics, owned by the driver).

#![allow(clippy::expect_used, reason = "pure fixtures use bounded indices")]

use std::cell::RefCell;

use ratatui_core::text::Line;

use super::draw::Paint;
use super::lane::{Show, preview_why, rows, tail_in};
use super::lanemd::Cache;
use crate::board::Observation;
use crate::console::lane::{Body, Kind, Lane, Seat, fold};
use crate::console::view;
use crate::events::Event;

const T0: &str = "2026-09-30T06:00:00Z";
const T1: &str = "2026-09-30T06:01:00Z";
const T2: &str = "2026-09-30T06:02:00Z";
const T3: &str = "2026-09-30T06:03:00Z";
const T4: &str = "2026-09-30T06:04:00Z";

const Q1: &str =
    "Deploy now | wait for review. Recommend now because the window closes. Details: memo deploy.";
const Q2: &str = "Pick a name\tfor v2";
const Q3: &str = "Trust the new folder | keep it quarantined. Recommend trust.";
const HQ: &str = "Pick\x1b[2J now\r | wait\x07 soon.\nWhy\u{0} not?";

fn esc(text: &str) -> String {
    let mut out = String::new();
    crate::json::escape_into(text, &mut out);
    out
}

fn declared(ts: &str, actor: &str, state: &str, reason: &str) -> Event {
    let reason = esc(reason);
    Event::parse_line(&format!(
        r#"{{"ts":"{ts}","actor":"{actor}","action":"state","ref":"{state}","summary":"{reason}"}}"#
    ))
    .expect("fixture parses")
}

const ASK_ID: &str = "ae-20260930T060100Z-0000abcd";

/// The human's console ask to `to`: ends standing currency, declares nothing.
fn asked(ts: &str, to: &str, text: &str) -> Event {
    let text = esc(text);
    Event::parse_line(&format!(
        r#"{{"ts":"{ts}","actor":"console:local","action":"ask","target":"{to}","ref":"{ASK_ID}","summary":"{text}"}}"#
    ))
    .expect("fixture parses")
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

fn text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

fn drawn(lane: &Lane, width: usize) -> Vec<String> {
    rows(lane, width, Paint::of(None), &view::Style::PLAIN, "lead")
        .iter()
        .map(text)
        .collect()
}

/// The lane through the Markdown-enabled draw path, as text rows.
fn drawn_md(lane: &Lane, width: usize) -> Vec<String> {
    let md = RefCell::new(Cache::default());
    let show = Show {
        clock: &view::Style::PLAIN,
        main: "lead",
        open: None,
        sent: &[],
        icons: true,
        base: &crate::console::link::Base::default(),
        today: None,
    };
    tail_in(lane, width, Paint::of(None), (&show, Some(&md)), 1000, None)
        .rows
        .iter()
        .map(text)
        .collect()
}

#[test]
fn app_past_card_has_a_closed_header_and_the_standing_cards_body_rows() {
    let past = lane_of(&[
        declared(T0, "lead", "waiting-user", Q1),
        declared(T1, "lead", "working", "moving on"),
    ]);
    let open = lane_of(&[declared(T0, "lead", "waiting-user", Q1)]);
    let (past, open) = (drawn(&past, 400), drawn(&open, 400));
    assert!(!past.is_empty(), "the past card renders");
    assert!(
        past[0].contains("DECISION") && past[0].contains("lead") && past[0].contains("closed"),
        "closed header: {}",
        past[0]
    );
    assert!(!past[0].contains("waiting-user"), "{}", past[0]);
    assert!(
        open[0].contains("DECISION · waiting-user"),
        "standing header unchanged: {}",
        open[0]
    );
    assert_eq!(past[1..], open[1..], "same laid-out body via the same path");
}

#[test]
fn app_past_card_lays_out_at_narrow_width_with_every_word_in_order() {
    for reason in ["Go now | wait a bit. Act soon.", "Steps. 1 merge | 2 wait"] {
        let lane = lane_of(&[
            declared(T0, "lead", "waiting-user", reason),
            declared(T1, "lead", "working", "moving on"),
        ]);
        let out = drawn(&lane, 20).join("\n");
        assert!(
            out.contains("DECISION") && out.contains("closed"),
            "{reason}: {out}"
        );
        let mut at = 0;
        for word in reason.split_whitespace().filter(|word| *word != "|") {
            let found = out[at..].find(word).expect("every word kept");
            at += found + word.len();
        }
    }
}

#[test]
fn app_hostile_bytes_in_a_past_card_neutralise_exactly_like_a_standing_one() {
    let past = lane_of(&[
        declared(T0, "lead", "waiting-user", HQ),
        declared(T1, "lead", "working", "moving on"),
    ]);
    let open = lane_of(&[declared(T0, "lead", "waiting-user", HQ)]);
    let (past, open) = (drawn(&past, 400), drawn(&open, 400));
    assert!(!past.is_empty(), "the past card renders");
    assert_eq!(past[1..], open[1..], "same neutraliser");
    for (name, rows) in ["past", "open"].iter().zip([&past, &open]) {
        for row in rows {
            assert!(
                !row.chars().any(|ch| ch.is_control() && ch != '\t'),
                "{name}: {row:?}"
            );
        }
    }
}

#[test]
fn app_fresh_lane_shows_past_cards_in_history_order() {
    let lane = lane_of(&[
        declared(T0, "lead", "waiting-user", Q1),
        declared(T1, "lead", "waiting-user", Q1),
        declared(T2, "lead", "waiting-user", Q2),
        declared(T3, "lead", "working", "moving on"),
        declared(T4, "colead", "waiting-user", Q3),
    ]);
    let heads: Vec<String> = drawn(&lane, 400)
        .into_iter()
        .filter(|row| row.contains("DECISION"))
        .collect();
    assert_eq!(heads.len(), 3, "two past cards plus the standing one");
    assert!(
        heads[0].contains("lead") && heads[0].contains("closed"),
        "{}",
        heads[0]
    );
    assert!(heads[0].contains("06:00"), "first appearance: {}", heads[0]);
    assert!(
        heads[1].contains("lead") && heads[1].contains("closed"),
        "{}",
        heads[1]
    );
    assert!(heads[1].contains("06:02"), "changed text: {}", heads[1]);
    assert!(
        heads[2].contains("colead") && heads[2].contains("waiting-user"),
        "{}",
        heads[2]
    );
    assert!(heads[2].contains("06:04"), "{}", heads[2]);
}

#[test]
fn app_card_ended_by_your_ask_renders_past_with_ask_below() {
    let lane = lane_of(&[
        declared(T0, "lead", "waiting-user", Q1),
        asked(T1, "lead", "ship it"),
    ]);
    let rows = drawn(&lane, 400);
    let past = rows
        .iter()
        .position(|row| row.contains("DECISION"))
        .expect("past renders");
    assert!(
        rows[past].contains("closed") && !rows[past].contains("waiting-user"),
        "{}",
        rows[past]
    );
    let ask = rows
        .iter()
        .position(|row| row.contains("you") && row.contains("to lead"))
        .expect("ask renders");
    assert!(past < ask, "past at T0, ask below it");
    assert!(
        !rows.iter().any(|row| row.contains("waiting-user")),
        "no standing card"
    );
}

#[test]
fn app_past_card_draws_through_markdown_exactly_like_a_standing_one() {
    let reason = "Read [the memo](https://example.test/memo) | skip it. Recommend read.";
    let past = lane_of(&[
        declared(T0, "lead", "waiting-user", reason),
        declared(T1, "lead", "working", "on"),
    ]);
    let open = lane_of(&[declared(T0, "lead", "waiting-user", reason)]);
    let (past_md, open_md) = (drawn_md(&past, 100), drawn_md(&open, 100));
    assert_eq!(past_md.len(), open_md.len(), "the past card renders");
    assert_ne!(
        open_md[1..],
        drawn(&open, 100)[1..],
        "the markdown path is engaged for standing"
    );
    assert!(
        past_md[0].contains("closed") && !past_md[0].contains("waiting-user"),
        "{}",
        past_md[0]
    );
    assert_eq!(
        past_md[1..],
        open_md[1..],
        "past draws through the same markdown path"
    );
}

#[test]
fn app_past_card_carries_the_standing_cards_copy_note() {
    let past_lane = lane_of(&[
        declared(T0, "lead", "waiting-user", Q1),
        declared(T1, "lead", "working", "on"),
    ]);
    let open_lane = lane_of(&[declared(T0, "lead", "waiting-user", Q1)]);
    let past = past_lane
        .items
        .iter()
        .find(|item| item.body == Q1 && !matches!(item.kind, Kind::Card { .. }))
        .expect("past item");
    let open = open_lane
        .items
        .iter()
        .find(|item| matches!(item.kind, Kind::Card { .. }))
        .expect("standing item");
    assert_eq!(preview_why(past), preview_why(open));
    assert!(
        preview_why(past).is_some_and(|why| why.contains("capped at 600 chars")),
        "a journal summary, like the standing card"
    );
}
