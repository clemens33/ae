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
use crate::console::lane::{Item, Kind, Lane, OLD_CORE_GAP};
use crate::console::wrap::wrap;
use crate::console::{fold, view};
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
    /// The row, counted from `first`, that folds or unfolds the turn.
    pub(super) fold: Option<usize>,
}

/// What a frame draws of the lane beyond the lane itself: the chat's own style
/// and main seat, which turns show whole and which asks went through, one
/// entry per item.
pub(super) struct Show<'a> {
    pub(super) clock: &'a view::Style,
    pub(super) main: &'a str,
    /// `None` folds nothing and draws no fold row.
    pub(super) open: Option<&'a [bool]>,
    pub(super) sent: &'a [bool],
}

/// [`tail_with`] for a lane that folds nothing and sent nothing.
#[cfg(test)]
pub(super) fn tail(
    lane: &Lane,
    width: usize,
    paint: Paint,
    clock: &view::Style,
    main: &str,
    need: usize,
    visited: Option<&mut Vec<usize>>,
) -> Tail {
    let show = Show {
        clock,
        main,
        open: None,
        sent: &[],
    };
    tail_with(lane, width, paint, &show, need, visited)
}

/// The newest `need` rows of `lane` or more, as [`rows`] draws them: turns
/// from the newest back, whole, then coverage rows from the last back, until
/// `need` rows stand. `visited` gets the index of each item whose body this
/// call wrapped, in the order it wrapped them.
pub(super) fn tail_with(
    lane: &Lane,
    width: usize,
    paint: Paint,
    show: &Show<'_>,
    need: usize,
    mut visited: Option<&mut Vec<usize>>,
) -> Tail {
    // Newest first, each turn's item, rows and fold row in order; reversed
    // into place below.
    let mut turns: Vec<(usize, Vec<Line<'static>>, Option<usize>)> = Vec::new();
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
        let sent = show.sent.get(at).copied().unwrap_or(false);
        rows.push(header(item, show.clock, show.main, paint, sent));
        if let Some(visited) = visited.as_deref_mut() {
            visited.push(at);
        }
        let body = body_rows(item, width, paint);
        let foldable = show.open.is_some() && fold::foldable(body.len(), 1);
        let open = show.open.is_none_or(|open| open.get(at) == Some(&true));
        let (body, hidden) = fold::folded(body, open, 1);
        rows.extend(body);
        let fold = foldable.then(|| {
            let words = match hidden {
                0 => "show fewer lines".to_owned(),
                hidden => fold::line(hidden),
            };
            rows.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(words, paint.fg(|p| p.dim)),
            ]));
            rows.len() - 1
        });
        count += rows.len();
        turns.push((at, rows, fold));
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
    for (at, turn, fold) in turns.into_iter().rev() {
        let spacer = usize::from(turn.first().is_some_and(|line| line.spans.is_empty()));
        placed.push(Turn {
            at,
            first: rows.len() + spacer,
            rows: turn.len() - spacer,
            fold: fold.map(|row| row - spacer),
        });
        rows.extend(turn);
    }
    Tail {
        rows,
        complete: items.len() == 0 && gaps.len() == 0,
        turns: placed,
    }
}

/// A turn's header row: its speaker, minute and qualifier.
fn header(item: &Item, clock: &view::Style, main: &str, paint: Paint, sent: bool) -> Line<'static> {
    let (speaker, qualifier, voice) = head(&item.kind, main, sent);
    let (_, time) = clock_text(clock.shift(item.micros));
    let minute: String = time.chars().take(5).collect();
    Line::from(vec![
        Span::styled(terminal_text(&speaker), voiced(paint, voice)),
        Span::styled(format!("  {minute}  "), paint.fg(|p| p.dim)),
        Span::styled(terminal_text(&qualifier), paint.fg(|p| p.dim)),
    ])
}

/// A turn's body rows, wrapped to `width`.
fn body_rows(item: &Item, width: usize, paint: Paint) -> Vec<Line<'static>> {
    let lines = view::body_lines(item).into_iter();
    lines
        .flat_map(|line| wrap(&terminal_text(line), width, 2))
        .map(|(gap, piece)| {
            Line::from(vec![
                Span::raw(" ".repeat(gap)),
                Span::styled(piece, paint.fg(|p| p.text)),
            ])
        })
        .collect()
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
        rows.push(header(item, clock, main, paint, false));
        rows.extend(body_rows(item, width, paint));
    }
    rows
}

/// The speaker, what the turn is and its voice. The words are the chat's
/// tag reordered so the speaker leads: `you … to lead`, `lead … said`.
fn head(kind: &Kind, main: &str, through: bool) -> (String, String, Voice) {
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
        Kind::Asked {
            to, uncertain, gap, ..
        } => {
            let mut what = format!("to {to}");
            if *uncertain {
                let _ = write!(what, " · uncertain: check the {to} pane");
            } else if through {
                what.push_str(" · sent");
            }
            // A legacy ask has no stored body to name as missing.
            if let Some(gap) = gap.as_deref().filter(|gap| *gap != OLD_CORE_GAP) {
                let _ = write!(what, " · {gap}");
            }
            you(what)
        }
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
        Kind::Asked { gap, .. } => gap.as_ref().map(|_| summary.to_owned()),
        Kind::Inbound { .. }
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
/// reads as marked in plain text too. The answer is the rows that took a bar.
pub(super) fn mark_rows(turn: &mut [Line<'static>], paint: Paint, icons: bool) -> Vec<usize> {
    let bar = Span::styled(
        if icons { "│" } else { "|" },
        paint.fg(|palette| palette.title),
    );
    let mut barred = Vec::new();
    for (row, line) in turn.iter_mut().enumerate() {
        let Some(lead) = line.spans.first_mut() else {
            continue;
        };
        if row == 0 {
            lead.style = paint.selected();
        } else if let Some(rest) = lead.content.strip_prefix(' ') {
            lead.content = rest.to_owned().into();
            line.spans.insert(0, bar.clone());
            barred.push(row);
        }
    }
    barred
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
    use crate::console::lane::{Item, Kind, Lane, OLD_CORE_GAP};
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

    #[test]
    fn decision_cards_render_options_as_separate_rows_without_changing_other_turns() {
        let reason = "Pick layout: vertical keeps panes readable | horizontal shows more context (recommend vertical because readability; details: .local/layout.md)";
        let want = [
            "  Pick layout:",
            "  vertical keeps panes readable",
            "  horizontal shows more context",
            "  (recommend vertical because readability; details: .local/layout.md)",
        ];
        for (kind, body, expected) in [
            (Kind::Card { seat: s() }, reason, want.to_vec()),
            (Kind::Said { who: s() }, reason, vec![]),
            (
                Kind::Card { seat: s() },
                "Waiting on colead for the S2 freeze report",
                vec!["  Waiting on colead for the S2 freeze report"],
            ),
            (
                Kind::Card { seat: s() },
                "Release v1: approve the tag",
                vec!["  Release v1: approve the tag"],
            ),
        ] {
            let lane = Lane {
                items: vec![item(kind, body)],
                coverage: vec![],
            };
            let full = super::rows(&lane, 400, Paint::of(None), &view::Style::PLAIN, "lead");
            let shown = tail(
                &lane,
                400,
                Paint::of(None),
                &view::Style::PLAIN,
                "lead",
                100,
                None,
            );
            assert_eq!(shown.rows, full);
            let body_rows: Vec<_> = full[1..].iter().map(text).collect();
            if expected.is_empty() {
                assert_eq!(body_rows, [format!("  {reason}")]);
            } else {
                assert_eq!(body_rows, expected);
            }
            assert_eq!(
                lane.items[0].body, body,
                "display leaves the stored reason raw"
            );
        }
    }

    #[test]
    fn decision_card_rows_wrap_under_their_own_text_and_neutralise_controls() {
        let lane = Lane {
            items: vec![item(
                Kind::Card { seat: s() },
                "Pick: vertical panes stay readable\x1b[2J | horizontal context stays visible",
            )],
            coverage: vec![],
        };
        let shown = tail(
            &lane,
            25,
            Paint::of(None),
            &view::Style::PLAIN,
            "lead",
            100,
            None,
        );
        assert_eq!(
            shown.rows[1..].iter().map(text).collect::<Vec<_>>(),
            [
                "  Pick:",
                "  vertical panes stay",
                "  readable\u{fffd}[2J",
                "  horizontal context",
                "  stays visible",
            ]
        );
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
                    gap: None,
                },
                None,
            ),
            (
                Kind::Asked {
                    to: s(),
                    id: s(),
                    uncertain: false,
                    gap: Some("body missing".to_owned()),
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

    fn shown_with(item: Item, open: Option<&[bool]>, sent: bool) -> super::Tail {
        let lane = Lane {
            items: vec![item],
            coverage: vec![],
        };
        let show = super::Show {
            clock: &view::Style::PLAIN,
            main: "x",
            open,
            sent: &[sent],
        };
        super::tail_with(&lane, 40, Paint::of(None), &show, 100, None)
    }

    fn rows_of(shown: &super::Tail) -> Vec<String> {
        shown.rows.iter().map(text).collect()
    }

    #[test]
    fn a_long_turn_folds_to_eight_rows_and_its_fold_row_toggles_it() {
        let lines = |n: usize| (1..=n).map(|n| format!("l{n}")).collect::<Vec<_>>();
        let turn =
            |n: usize, open: Option<&[bool]>| shown_with(said(&lines(n).join("\n")), open, false);
        // Ten rows show whole and a lane that folds nothing never has a fold row.
        for shown in [turn(10, Some(&[false])), turn(12, None)] {
            assert_eq!(shown.turns[0].fold, None);
        }
        assert_eq!(rows_of(&turn(10, Some(&[false]))).len(), 11);
        let folded = turn(11, Some(&[false]));
        let rows = rows_of(&folded);
        assert_eq!(folded.turns[0].fold, Some(9));
        assert_eq!(rows[8], "  l8");
        assert_eq!(rows[9], "  ... 3 more lines");
        assert_eq!(rows.len(), 10);
        let open = turn(12, Some(&[true]));
        let rows = rows_of(&open);
        assert_eq!((rows.len(), open.turns[0].fold), (14, Some(13)));
        assert_eq!(
            (rows[12].as_str(), rows[13].as_str()),
            ("  l12", "  show fewer lines")
        );
    }

    #[test]
    fn an_ask_header_says_sent_and_names_why_it_shows_a_summary_but_a_legacy_ask_names_nothing() {
        let ask = |uncertain: bool, gap: Option<&str>| {
            let kind = Kind::Asked {
                to: s(),
                id: "q".to_owned(),
                uncertain,
                gap: gap.map(str::to_owned),
            };
            item(kind, "body")
        };
        for (item, sent, want) in [
            (ask(false, None), true, "to lead · sent"),
            (ask(false, None), false, "to lead"),
            (
                ask(false, Some("body missing")),
                true,
                "to lead · sent · body missing",
            ),
            (ask(false, Some(OLD_CORE_GAP)), true, "to lead · sent"),
            (
                ask(true, None),
                true,
                "to lead · uncertain: check the lead pane",
            ),
        ] {
            let head = text(&shown_with(item, None, sent).rows[0]);
            assert!(head.ends_with(want), "{head:?} for {want:?}");
        }
    }
}
