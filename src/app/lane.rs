//! The chat column's turns: a [`Lane`] as rows of cells at a width.
//!
//! PURE. The chat's own provenance survives, quieter: a transcript mirror, a
//! prior conversation, a 600-character preview, an uncertain or undelivered
//! ask, a late answer, a reply not admitted, a coverage gap. Every body is
//! wrapped by the chat's own `wrap` and every field neutralised by
//! `terminal_text`.

use std::fmt::Write as _;

use ratatui_core::style::{Modifier, Style};
use ratatui_core::text::{Line, Span};

use super::draw::Paint;
use crate::board::{clock_text, terminal_text};
use crate::console::lane::{Kind, Lane};
use crate::console::view;
use crate::console::wrap::wrap;

/// Who speaks a turn, which is all its colour says.
#[derive(Clone, Copy)]
enum Voice {
    You,
    Lead,
    Colead,
    System,
}

/// The newest rows of a lane as [`rows`] draws them.
pub(super) struct Tail {
    /// A suffix of the full render, oldest first, holding at least the rows
    /// asked for when the lane has them.
    pub(super) rows: Vec<Line<'static>>,
    /// Whether `rows` reaches the lane's oldest row, its coverage included.
    pub(super) complete: bool,
}

/// The newest `need` rows of `lane` or more, as [`rows`] draws them: turns
/// from the newest back, whole, then coverage rows from the last back, until
/// `need` rows stand. `visited` gets the index of each item whose body this
/// call wrapped, in the order it wrapped them.
pub(super) fn tail(
    lane: &Lane,
    width: usize,
    paint: Paint,
    clock: &view::Style,
    main: &str,
    need: usize,
    mut visited: Option<&mut Vec<usize>>,
) -> Tail {
    // Newest first, each turn's rows in order; reversed into place below.
    let mut turns: Vec<Vec<Line<'static>>> = Vec::new();
    let mut count = 0;
    let mut items = lane.items.iter().enumerate().rev();
    while count < need {
        let Some((at, item)) = items.next() else {
            break;
        };
        let mut rows = Vec::new();
        if at > 0 || !lane.coverage.is_empty() {
            rows.push(Line::default());
        }
        let (speaker, qualifier, voice) = head(&item.kind, main);
        let (_, time) = clock_text(clock.shift(item.micros));
        let minute: String = time.chars().take(5).collect();
        rows.push(Line::from(vec![
            Span::styled(terminal_text(&speaker), voiced(paint, voice)),
            Span::styled(format!("  {minute}  "), paint.fg(|p| p.dim)),
            Span::styled(terminal_text(&qualifier), paint.fg(|p| p.dim)),
        ]));
        if let Some(visited) = visited.as_deref_mut() {
            visited.push(at);
        }
        for (gap, piece) in wrap(&terminal_text(&item.body), width, 2) {
            rows.push(Line::from(vec![
                Span::raw(" ".repeat(gap)),
                Span::styled(piece, paint.fg(|p| p.text)),
            ]));
        }
        count += rows.len();
        turns.push(rows);
    }
    let mut gaps = lane.coverage.iter().rev();
    let mut coverage = Vec::new();
    while count < need && items.len() == 0 {
        let Some(gap) = gaps.next() else {
            break;
        };
        let text = terminal_text(&format!("coverage incomplete: {gap}"));
        coverage.push(Line::from(Span::styled(text, paint.fg(|p| p.dim))));
        count += 1;
    }
    let mut rows = Vec::with_capacity(count);
    rows.extend(coverage.into_iter().rev());
    rows.extend(turns.into_iter().rev().flatten());
    Tail {
        rows,
        complete: items.len() == 0 && gaps.len() == 0,
    }
}

/// The rows of `lane`, oldest first, `width` cells wide, a blank row between
/// turns. `clock` is the chat's own style: its viewer zone shifts the stamps.
/// `main` is the seat whose turns wear the lead hue. The full render: the
/// reference [`tail`] is a suffix of.
#[cfg(test)]
pub(super) fn rows(
    lane: &Lane,
    width: usize,
    paint: Paint,
    clock: &view::Style,
    main: &str,
) -> Vec<Line<'static>> {
    let mut rows: Vec<Line<'static>> = lane
        .coverage
        .iter()
        .map(|gap| {
            let text = terminal_text(&format!("coverage incomplete: {gap}"));
            Line::from(Span::styled(text, paint.fg(|p| p.dim)))
        })
        .collect();
    for item in &lane.items {
        if !rows.is_empty() {
            rows.push(Line::default());
        }
        let (speaker, qualifier, voice) = head(&item.kind, main);
        let (_, time) = clock_text(clock.shift(item.micros));
        let minute: String = time.chars().take(5).collect();
        rows.push(Line::from(vec![
            Span::styled(terminal_text(&speaker), voiced(paint, voice)),
            Span::styled(format!("  {minute}  "), paint.fg(|p| p.dim)),
            Span::styled(terminal_text(&qualifier), paint.fg(|p| p.dim)),
        ]));
        for (gap, piece) in wrap(&terminal_text(&item.body), width, 2) {
            rows.push(Line::from(vec![
                Span::raw(" ".repeat(gap)),
                Span::styled(piece, paint.fg(|p| p.text)),
            ]));
        }
    }
    rows
}

/// The speaker, what the turn is and its voice. The words are the chat's
/// tag reordered so the speaker leads: `you … to lead`, `lead … said`.
fn head(kind: &Kind, main: &str) -> (String, String, Voice) {
    let seat = |name: &str| match name {
        "ae" => Voice::System,
        name if name == main => Voice::Lead,
        _ => Voice::Colead,
    };
    let prior = |generation: u8| match generation {
        0 => String::new(),
        n => format!(" · prior {n}"),
    };
    let you = |what: String| ("you".to_owned(), what, Voice::You);
    match kind {
        Kind::Pane { seat, generation } => you(format!(
            "in the {seat} pane · transcript{}",
            prior(*generation)
        )),
        Kind::Inbound { from, to } => (from.clone(), format!("to {to}"), Voice::You),
        Kind::Asked { to, uncertain, .. } => you(if *uncertain {
            format!("to {to} · uncertain: check the {to} pane")
        } else {
            format!("to {to}")
        }),
        Kind::NotDelivered { to, .. } => you(format!("to {to} · not delivered")),
        Kind::Closed { .. } => you("closed an ask".to_owned()),
        Kind::Assistant {
            seat: name,
            generation,
        } => (
            name.clone(),
            format!("transcript{}", prior(*generation)),
            seat(name),
        ),
        Kind::Reply { from, to } => (
            from.clone(),
            format!("to {to} · 600-char preview"),
            seat(from),
        ),
        Kind::Said { who } => (who.clone(), "said".to_owned(), seat(who)),
        Kind::Card { seat: name } => (
            name.clone(),
            "DECISION · waiting-user".to_owned(),
            seat(name),
        ),
        Kind::NeedsYou { seat: name } => (
            name.clone(),
            "NEEDS YOU · in its pane".to_owned(),
            seat(name),
        ),
        Kind::Answer {
            seat: name,
            follow_up,
            late,
            gap,
            speaker,
            ..
        } => {
            let mut what = "answers".to_owned();
            if *follow_up > 0 {
                let _ = write!(what, " · follow-up {follow_up}");
            }
            if *late {
                what.push_str(" · late (closed)");
            }
            if let Some(profile) = speaker {
                let _ = write!(what, " · speaker now {profile}");
            }
            if let Some(gap) = gap {
                let _ = write!(what, " · {gap} · 600-char preview");
            }
            (name.clone(), what, seat(name))
        }
        Kind::Unadmitted { from, why, .. } => (
            from.clone(),
            format!("reply · not admitted: {why} · 600-char preview"),
            seat(from),
        ),
    }
}

/// A speaker's style: the lead in its hue, you and the colead bold text, ae dim.
fn voiced(paint: Paint, voice: Voice) -> Style {
    match voice {
        Voice::Lead => paint.fg(|p| p.working),
        Voice::You | Voice::Colead => paint.fg(|p| p.text).add_modifier(Modifier::BOLD),
        Voice::System => paint.fg(|p| p.dim),
    }
}
