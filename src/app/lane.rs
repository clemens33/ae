//! The chat column's turns: a [`Lane`] as rows of cells at a width.
//!
//! PURE. The chat's own provenance survives, quieter: a transcript mirror, a
//! prior conversation, a 600-character preview, an uncertain or undelivered
//! ask, a late answer, a reply not admitted, a coverage gap. Every body is
//! wrapped by the chat's own `wrap` and every field neutralised by
//! `terminal_text`.

use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};

use ratatui_core::style::{Modifier, Style};
use ratatui_core::text::{Line, Span};

use super::draw::Paint;
use crate::board::{clock_text, terminal_text};
use crate::console::lane::{Item, Kind, Lane};
use crate::console::view;
use crate::console::wrap::wrap;
use crate::state::CHAT_SUMMARY_CAP;

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
    /// Where each produced turn stands in `rows`, oldest first.
    pub(super) turns: Vec<Turn>,
}

/// A turn's place in [`Tail::rows`]: its item, its header row and its row
/// count, the blank row above it left out.
pub(super) struct Turn {
    pub(super) at: usize,
    pub(super) first: usize,
    pub(super) rows: usize,
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
    // Newest first, each turn's item and rows in order; reversed into place below.
    let mut turns: Vec<(usize, Vec<Line<'static>>)> = Vec::new();
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
        turns.push((at, rows));
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
    let mut placed = Vec::with_capacity(turns.len());
    for (at, turn) in turns.into_iter().rev() {
        let spacer = usize::from(turn.first().is_some_and(|line| line.spans.is_empty()));
        placed.push(Turn {
            at,
            first: rows.len() + spacer,
            rows: turn.len() - spacer,
        });
        rows.extend(turn);
    }
    Tail {
        rows,
        complete: items.len() == 0 && gaps.len() == 0,
        turns: placed,
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

/// What names a turn across re-reads: its fingerprint over everything that
/// tells it apart. In-process only, never persisted; a collision is 2^-64.
pub(super) fn key(item: &Item) -> u64 {
    let mut hasher = DefaultHasher::new();
    item.hash(&mut hasher);
    hasher.finish()
}

/// Why `item`'s copy is not the whole body, or `None` when it is. The words
/// ride the flash after the line count.
pub(super) fn preview_why(item: &Item) -> Option<String> {
    let summary = "journal summary, line breaks flattened, capped at 600 chars";
    match &item.kind {
        Kind::Answer { gap, .. } => gap.clone(),
        Kind::Reply { .. } => Some("the record keeps no body".to_owned()),
        Kind::Unadmitted { .. } => Some("not admitted".to_owned()),
        Kind::Inbound { .. }
        | Kind::Asked { .. }
        | Kind::NotDelivered { .. }
        | Kind::Card { .. }
        | Kind::NeedsYou { .. } => Some(summary.to_owned()),
        Kind::Said { .. } if item.body.chars().count() >= CHAT_SUMMARY_CAP => Some(format!(
            "journal summary, capped at {CHAT_SUMMARY_CAP} chars"
        )),
        Kind::Said { .. } | Kind::Pane { .. } | Kind::Assistant { .. } | Kind::Closed { .. } => {
            None
        }
    }
}

/// Mark the rows of one turn, header first: the speaker takes the selected
/// style and each body row's leading blank cell becomes a bar, so the turn
/// reads as marked in plain text too.
pub(super) fn mark_rows(turn: &mut [Line<'static>], paint: Paint, icons: bool) {
    let bar = Span::styled(
        if icons { "│" } else { "|" },
        paint.fg(|palette| palette.title),
    );
    for (row, line) in turn.iter_mut().enumerate() {
        let Some(lead) = line.spans.first_mut() else {
            continue;
        };
        if row == 0 {
            lead.style = paint.selected();
        } else if let Some(rest) = lead.content.strip_prefix(' ') {
            lead.content = rest.to_owned().into();
            line.spans.insert(0, bar.clone());
        }
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

#[cfg(test)]
mod tests {
    use ratatui_core::style::Modifier;
    use ratatui_core::text::Line;

    use super::{Paint, key, mark_rows, preview_why, tail};
    use crate::console::lane::{Item, Kind, Lane};
    use crate::console::view;

    fn item(kind: Kind, body: &str) -> Item {
        let body = body.to_owned();
        let (micros, record) = (1, None);
        Item {
            micros,
            kind,
            body,
            record,
        }
    }

    fn said(body: &str) -> Item {
        item(Kind::Said { who: s() }, body)
    }

    fn s() -> String {
        "lead".to_owned()
    }

    fn text(line: &Line<'_>) -> String {
        (line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn tail_of(items: Vec<Item>, need: usize) -> super::Tail {
        let lane = Lane {
            items,
            coverage: vec![s()],
        };
        tail(
            &lane,
            40,
            Paint::of(None),
            &view::Style::PLAIN,
            "x",
            need,
            None,
        )
    }

    /// A turn's key follows everything that tells it apart.
    #[test]
    fn a_turn_key_names_the_item_it_was_made_from() {
        let one = said("one");
        assert_eq!(key(&one), key(&one.clone()));
        let at = Item {
            micros: 2,
            ..one.clone()
        };
        let recorded = Item {
            record: Some(3),
            ..one.clone()
        };
        for other in [
            said("two"),
            at,
            recorded,
            item(Kind::Closed { id: s() }, "one"),
        ] {
            assert_ne!(key(&one), key(&other));
        }
    }

    /// The copy says why it is not the whole body, kind by kind.
    #[test]
    fn a_preview_names_why_it_is_not_the_whole_body() {
        let summary = "journal summary, line breaks flattened, capped at 600 chars";
        let answer = |gap: Option<&str>| Kind::Answer {
            seat: s(),
            id: s(),
            follow_up: 0,
            late: false,
            gap: gap.map(str::to_owned),
            speaker: None,
        };
        let cases = [
            (Kind::Inbound { from: s(), to: s() }, Some(summary)),
            (
                Kind::Asked {
                    to: s(),
                    id: s(),
                    uncertain: false,
                },
                Some(summary),
            ),
            (Kind::NotDelivered { to: s(), id: s() }, Some(summary)),
            (Kind::Card { seat: s() }, Some(summary)),
            (Kind::NeedsYou { seat: s() }, Some(summary)),
            (
                Kind::Reply { from: s(), to: s() },
                Some("the record keeps no body"),
            ),
            (
                Kind::Unadmitted {
                    from: s(),
                    id: s(),
                    why: s(),
                },
                Some("not admitted"),
            ),
            (answer(Some("body missing")), Some("body missing")),
            (answer(None), None),
            (
                Kind::Pane {
                    seat: s(),
                    generation: 0,
                },
                None,
            ),
            (Kind::Closed { id: s() }, None),
        ];
        for (kind, want) in cases {
            let why = preview_why(&item(kind.clone(), "b"));
            assert_eq!(why.as_deref(), want, "{kind:?}");
        }
        let cap = |len| preview_why(&said(&"x".repeat(len)));
        assert_eq!((cap(3499), cap(3500).is_some()), (None, true));
    }

    /// Each produced turn stands on its own header and body rows, the blank
    /// row above it left out, oldest first.
    #[test]
    fn a_turn_stands_where_its_rows_are_drawn() {
        let all = tail_of(vec![said("a1\na2"), said("b1"), said("c1")], 100);
        let shape: Vec<(usize, usize)> = (all.turns.iter())
            .map(|turn| (turn.at, turn.rows))
            .collect();
        assert_eq!(shape, [(0, 3), (1, 2), (2, 2)]);
        for turn in &all.turns {
            assert!(
                text(&all.rows[turn.first]).starts_with("lead"),
                "the header"
            );
            assert!(
                text(&all.rows[turn.first - 1]).is_empty(),
                "the blank above"
            );
        }
        let last = &all.turns[2];
        assert_eq!(last.first + last.rows, all.rows.len());
        let newest = tail_of(vec![said("a1"), said("b1"), said("c1")], 1);
        assert_eq!(newest.turns.len(), 1, "a short tail produces what it drew");
    }

    /// A marked turn reads marked in plain text: a bar replaces the first
    /// blank cell of each body row, `|` with icons off; the header's speaker
    /// takes the selected style and keeps its words.
    #[test]
    fn a_marked_turn_wears_a_bar_on_its_body_rows() {
        for (icons, bar) in [(true, "│"), (false, "|")] {
            let mut shown = tail_of(vec![said("a1\na2")], 100);
            let turn = &shown.turns[0];
            let span = turn.first..turn.first + turn.rows;
            let plain: Vec<String> = shown.rows[span.clone()].iter().map(text).collect();
            mark_rows(&mut shown.rows[span.clone()], Paint::of(None), icons);
            let marked: Vec<String> = shown.rows[span.clone()].iter().map(text).collect();
            assert_eq!(marked[0], plain[0], "the header keeps its words");
            let head = shown.rows[span.start].spans[0].style;
            assert!(head.add_modifier.contains(Modifier::BOLD));
            for (was, now) in plain[1..].iter().zip(&marked[1..]) {
                assert_eq!(now, &format!("{bar}{}", &was[1..]));
            }
        }
    }
}
