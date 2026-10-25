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
use crate::console::card::{Part, Role};
use crate::console::lane::{Item, Kind, Lane, OLD_CORE_GAP};
use crate::console::{fold, link, view};
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
    /// The links of each of `rows`, parallel to it.
    pub(super) links: Vec<Links>,
    pub(super) files: Vec<FileRuns>,
}

/// The links of one row, left to right: the cell they start at, counted from
/// the row's left edge, the cells they cover and the URL they open.
pub(super) type Links = Vec<(usize, usize, String)>;

/// A row and its links.
pub(super) type FileRuns = Vec<(usize, usize, ReadLink)>;
type Drawn = (Line<'static>, Links, FileRuns);

/// A drawn file occurrence, bound to its source turn and resolution base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReadLink {
    pub turn: u64,
    pub fingerprint: u64,
    pub index: usize,
    pub file: link::FileRef,
    pub base: link::Base,
}

/// A row without links.
fn plain(line: Line<'static>) -> Drawn {
    (line, Vec::new(), Vec::new())
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
    /// Whether the look's icons are on: an option bullet is `•`, else `-`.
    pub(super) icons: bool,
    /// Where a path in a body resolves.
    pub(super) base: &'a link::Base,
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
        icons: true,
        base: &link::Base::default(),
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
    let mut turns: Vec<(usize, Vec<Drawn>, Option<usize>)> = Vec::new();
    let mut count = 0;
    let mut items = lane.items.iter().enumerate().rev();
    while count < need {
        let Some((at, item)) = items.next() else {
            break;
        };
        let mut rows = Vec::new();
        if at > 0 || !lane.coverage.is_empty() {
            rows.push(plain(Line::default()));
        }
        let sent = show.sent.get(at).copied().unwrap_or(false);
        let closed = closing(&lane.items, at);
        rows.push(plain(header(
            item,
            closed.as_ref(),
            show.clock,
            show.main,
            paint,
            sent,
        )));
        if let Some(visited) = visited.as_deref_mut() {
            visited.push(at);
        }
        let body = body_rows(
            item,
            closed.as_ref(),
            (width, show.base),
            paint,
            bullet(show.icons),
        );
        let foldable = show.open.is_some() && fold::foldable(body.len(), 1);
        let open = show.open.is_none_or(|open| open.get(at) == Some(&true));
        let (body, hidden) = fold::folded(body, open, 1);
        rows.extend(body);
        let fold = foldable.then(|| {
            let words = match hidden {
                0 => "show fewer lines".to_owned(),
                hidden => fold::line(hidden),
            };
            rows.push(plain(Line::from(vec![
                Span::raw("  "),
                Span::styled(words, paint.fg(|p| p.dim)),
            ])));
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
        coverage.push(plain(Line::from(Span::styled(text, paint.fg(|p| p.dim)))));
        count += 1;
    }
    let mut rows = Vec::with_capacity(count);
    rows.extend(coverage.into_iter().rev());
    let mut placed = Vec::with_capacity(turns.len());
    for (at, turn, fold) in turns.into_iter().rev() {
        let spacer = usize::from(
            turn.first()
                .is_some_and(|(line, _, _)| line.spans.is_empty()),
        );
        placed.push(Turn {
            at,
            first: rows.len() + spacer,
            rows: turn.len() - spacer,
            fold: fold.map(|row| row - spacer),
        });
        rows.extend(turn);
    }
    let (mut text_rows, mut links, mut files) = (Vec::new(), Vec::new(), Vec::new());
    for (line, runs, reads) in rows {
        text_rows.push(line);
        links.push(runs);
        files.push(reads);
    }
    Tail {
        rows: text_rows,
        complete: items.len() == 0 && gaps.len() == 0,
        turns: placed,
        links,
        files,
    }
}

/// What a closed ask's row names, whichever window or bridge closed it: the
/// ask's id, and its recipient and words from the ask's own row before it in
/// the lane.
struct Closing {
    id: String,
    /// The recipient and the words' excerpt; `None` when the lane no longer
    /// holds the ask.
    ask: Option<(String, String)>,
    cut: bool,
}

/// The [`Closing`] of `items[at]`, read back from the lane itself.
fn closing(items: &[Item], at: usize) -> Option<Closing> {
    let Kind::Closed { id } = &items.get(at)?.kind else {
        return None;
    };
    let asked = items
        .get(..at)?
        .iter()
        .rev()
        .find_map(|item| match &item.kind {
            Kind::Asked { to, id: asked, .. } if asked == id => {
                Some((to.clone(), crate::console::excerpt(&item.body)))
            }
            _ => None,
        });
    let cut = asked
        .as_ref()
        .is_some_and(|(_, words)| words.chars().count() > crate::console::CLOSED_CHARS);
    Some(Closing {
        id: id.clone(),
        ask: asked,
        cut,
    })
}

/// A turn's header row: its speaker, minute and qualifier.
fn header(
    item: &Item,
    closed: Option<&Closing>,
    clock: &view::Style,
    main: &str,
    paint: Paint,
    sent: bool,
) -> Line<'static> {
    let (speaker, mut qualifier, voice) = head(&item.kind, main, sent);
    if let Some((to, _)) = closed.and_then(|closed| closed.ask.as_ref()) {
        let _ = write!(qualifier, " to {to}");
    }
    // An acceptance is ONE row: its note rides the header, and no body follows.
    if matches!(item.kind, Kind::Accepted { .. }) && !item.body.is_empty() {
        let _ = write!(qualifier, " · {}", item.body);
    }
    let (_, time) = clock_text(clock.shift(item.micros));
    let minute: String = time.chars().take(5).collect();
    Line::from(vec![
        Span::styled(terminal_text(&speaker), voiced(paint, voice)),
        Span::styled(format!("  {minute}  "), paint.fg(|p| p.dim)),
        Span::styled(terminal_text(&qualifier), paint.fg(|p| p.dim)),
    ])
}

/// What an unlabelled option of a decision card wears before its words.
fn bullet(icons: bool) -> &'static str {
    if icons { "• " } else { "- " }
}

/// The same displayed parts serve keyboard reads and drawn occurrences.
fn parts(item: &Item, closed: Option<&Closing>) -> Vec<(Role, String)> {
    if matches!(item.kind, Kind::Accepted { .. }) {
        return Vec::new();
    }
    let said = closed.map(|closed| match &closed.ask {
        Some((_, words)) if !words.is_empty() => format!("{}: {words}", closed.id),
        _ => closed.id.clone(),
    });
    let parts = match &said {
        Some(said) => vec![Part {
            role: Role::Text,
            text: said.as_str(),
        }],
        None => view::body_parts(item),
    };
    parts
        .into_iter()
        .map(|part| (part.role, terminal_text(part.text)))
        .collect()
}

fn fingerprint(item: &Item, parts: &[(Role, String)]) -> u64 {
    let mut hash = DefaultHasher::new();
    item.kind.hash(&mut hash);
    for (_, text) in parts {
        text.hash(&mut hash);
    }
    hash.finish()
}

fn file_refs(text: &str, base: &link::Base, cut: bool) -> Vec<link::FileRef> {
    link::files(text, base)
        .into_iter()
        .filter(|file| {
            !cut || text
                .get(file.end..)
                .and_then(|rest| rest.strip_suffix('…'))
                .is_none_or(|gap| gap.chars().any(char::is_whitespace))
        })
        .collect()
}

/// All displayed file occurrences, before folding, clipping or scrolling.
pub(super) fn read_links(
    items: &[Item],
    at: usize,
    base: &link::Base,
) -> (u64, Vec<link::FileRef>) {
    let Some(item) = items.get(at) else {
        return (0, Vec::new());
    };
    let closed = closing(items, at);
    let parts = parts(item, closed.as_ref());
    let files = parts
        .iter()
        .flat_map(|(role, text)| {
            let label = match role {
                Role::Option { label } => *label,
                _ => 0,
            };
            file_refs(
                text.get(label..).unwrap_or(text),
                base,
                closed.as_ref().is_some_and(|c| c.cut),
            )
        })
        .collect();
    (fingerprint(item, &parts), files)
}

/// A turn's body rows, wrapped to `width`, with the links each carries. A
/// close shows the id it closed and the words of that ask, the same `closed
/// <id>: <words>` its closer was told. A decision card is drawn in its laid-out
/// parts: an option wears its label or `bullet` and hangs its wrapped rows
/// under its words, evidence dims, a gap is one empty row. `base` is where a
/// path in the words resolves.
fn body_rows(
    item: &Item,
    closed: Option<&Closing>,
    (width, base): (usize, &link::Base),
    paint: Paint,
    bullet: &str,
) -> Vec<Drawn> {
    let parts = parts(item, closed);
    let stamp = fingerprint(item, &parts);
    // A link is underlined where the look is drawn, and nowhere else.
    let underline = |hue: Style| {
        if paint.drawn() {
            hue.add_modifier(Modifier::UNDERLINED)
        } else {
            hue
        }
    };
    let mut rows = Vec::new();
    let mut index = 0;
    for (role, text) in parts {
        let label = match role {
            Role::Option { label } => label,
            _ => 0,
        };
        let refs = file_refs(
            text.get(label..).unwrap_or(&text),
            base,
            closed.is_some_and(|c| c.cut),
        );
        let row = |gap: usize, lead: &str, placed: &link::Placed, hue: Style| {
            let piece = &placed.row;
            let (line, links) = linked_row(
                gap,
                lead,
                (&piece.piece, &piece.links),
                (hue, underline(hue)),
            );
            let files = placed
                .files
                .iter()
                .filter_map(|(slot, file)| {
                    let ordinal = refs
                        .binary_search_by_key(&file.start, |found| found.start)
                        .ok()?;
                    let (column, width, _) = links.get(*slot)?;
                    Some((
                        *column,
                        *width,
                        ReadLink {
                            turn: key(item),
                            fingerprint: stamp,
                            index: index + ordinal,
                            file: file.clone(),
                            base: base.clone(),
                        },
                    ))
                })
                .collect();
            (line, links, files)
        };
        match role {
            Role::Gap => rows.push(plain(Line::from(vec![
                Span::raw(" ".repeat(2)),
                Span::styled(String::new(), paint.fg(|p| p.text)),
            ]))),
            Role::Text | Role::Evidence => {
                let hue = if role == Role::Evidence {
                    paint.fg(|p| p.dim)
                } else {
                    paint.fg(|p| p.text)
                };
                let pieces = link::placed(&text, width, 2, 1, base);
                rows.extend(
                    pieces
                        .iter()
                        .map(|piece| row(piece.row.gap, "", piece, hue)),
                );
            }
            Role::Option { label } => {
                let (mark, words) = match text.split_at_checked(label) {
                    Some((mark, words)) if label > 0 => (mark, words),
                    _ => (bullet, text.as_str()),
                };
                let hang = mark.chars().count();
                let pieces = link::placed(words, width.saturating_sub(hang), 2, 1, base);
                rows.extend(pieces.iter().enumerate().map(|(at, piece)| {
                    let mark = if at == 0 {
                        terminal_text(mark)
                    } else {
                        " ".repeat(hang)
                    };
                    // The mark sits at the margin; the words' own indent follows it.
                    let indent = " ".repeat(piece.row.gap.saturating_sub(2));
                    let hue = paint.fg(|p| p.text);
                    row(piece.row.gap.min(2), &format!("{mark}{indent}"), piece, hue)
                }));
            }
        }
        index += refs.len();
    }
    rows
}

/// One body row: `gap` blank cells, then `lead` and `piece` in `hue`, each of
/// `links` (byte ranges of `piece`) in `linked` and recorded with the cell it
/// starts at and the cells it covers. A row without links is the two spans it
/// always was.
fn linked_row(
    gap: usize,
    lead: &str,
    (piece, links): (&str, &[link::Link]),
    (hue, linked): (Style, Style),
) -> (Line<'static>, Links) {
    let text = format!("{lead}{piece}");
    let mut spans = vec![Span::raw(" ".repeat(gap))];
    let (mut runs, mut at) = (Vec::new(), 0);
    for found in links {
        let (start, end) = (lead.len() + found.start, lead.len() + found.end);
        let (Some(before), Some(inside)) = (text.get(at..start), text.get(start..end)) else {
            continue;
        };
        if inside.is_empty() || !link::is_safe_url(&found.url) {
            continue;
        }
        if !before.is_empty() {
            spans.push(Span::styled(before.to_owned(), hue));
        }
        // The cells the frame draws, as ratatui measures them: not the chat's
        // safe upper bound, which wraps early but would misplace a link.
        let column = gap + Span::raw(text.get(..start).unwrap_or_default()).width();
        runs.push((column, Span::raw(inside).width(), found.url.clone()));
        spans.push(Span::styled(inside.to_owned(), linked));
        at = end;
    }
    let rest = text.get(at..).unwrap_or_default();
    if !rest.is_empty() || runs.is_empty() {
        spans.push(Span::styled(rest.to_owned(), hue));
    }
    (Line::from(spans), runs)
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
    for (at, item) in lane.items.iter().enumerate() {
        if !rows.is_empty() {
            rows.push(Line::default());
        }
        let closed = closing(&lane.items, at);
        rows.push(header(item, closed.as_ref(), clock, main, paint, false));
        let base = link::Base::default();
        let body = body_rows(item, closed.as_ref(), (width, &base), paint, bullet(true));
        rows.extend(body.into_iter().map(|(line, _, _)| line));
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
            to,
            uncertain,
            gap,
            via,
            ..
        } => {
            let mut what = format!("to {to}");
            if let Some(via) = via {
                let _ = write!(what, " · {}", via.badge());
            }
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
        // Progress, not a turn the human must read: the whole row is dim.
        Kind::Accepted { seat: name, .. } => (name.clone(), "accepted".to_owned(), Voice::System),
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
        | Kind::NeedsYou { .. }
        | Kind::Accepted { .. } => Some(summary.to_owned()),
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
#[path = "lane_cut_spec.rs"]
mod cut_spec;

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
            "  ",
            "  • vertical keeps panes readable",
            "  • horizontal shows more context",
            "  ",
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
                "  ",
                "  • vertical panes stay",
                "    readable\u{fffd}[2J",
                "  • horizontal context",
                "    stays visible",
            ]
        );
    }

    // cardread spec r2: a DECISION card's body rows in the app lane, wide
    // enough that nothing wraps; the header and the stored body stand raw.
    fn cardread_rows(body: &str) -> Vec<String> {
        let lane = Lane {
            items: vec![item(Kind::Card { seat: s() }, body)],
            coverage: vec![],
        };
        let rows = super::rows(&lane, 400, Paint::of(None), &view::Style::PLAIN, "lead");
        assert!(text(&rows[0]).contains("DECISION"), "header stands");
        assert_eq!(lane.items[0].body, body, "the stored reason stays raw");
        rows[1..].iter().map(text).collect()
    }

    #[test]
    fn cardread_brief_examples_render_as_rows() {
        let ex1 = "Clemens decides: may lead release X (fix, review PASS, tests green) and roll Y only? Yes lets the retest show why it fails | no leaves Y broken with unknown cause. Recommend yes. Asked via say; AGENTS.md:49 needs his go. Memo p2.";
        let lane = Lane {
            items: vec![item(Kind::Card { seat: s() }, ex1)],
            coverage: vec![],
        };
        let full = super::rows(&lane, 400, Paint::of(None), &view::Style::PLAIN, "lead");
        assert_eq!(
            full[1..].iter().map(text).collect::<Vec<_>>(),
            [
                "  Clemens decides:",
                "  may lead release X (fix, review PASS, tests green) and roll Y only?",
                "  ",
                "  • Yes lets the retest show why it fails",
                "  • no leaves Y broken with unknown cause.",
                "  ",
                "  Recommend yes.",
                "  Asked via say; AGENTS.md:49 needs his go.",
                "  ",
                "  Memo p2.",
            ]
        );
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
        let folded = shown_with(item(Kind::Card { seat: s() }, ex1), Some(&[false]), false);
        assert_eq!(folded.turns[0].fold, None, "the example shows whole");
        assert!(
            rows_of(&folded).iter().any(|row| row.contains("Memo p2.")),
            "the last line shows"
        );
        assert!(
            !rows_of(&folded)
                .iter()
                .any(|row| row.contains("more lines")),
            "whole, unfolded"
        );
        assert_eq!(
            cardread_rows(
                "Asked Clemens to dismiss a modal in pane P; request queued. Last peek at 06:10 confirms modal; prior send ABANDONED after 30s. Clearing it permits retry. Evidence: .local/x/REPORT.md and memo parking."
            ),
            [
                "  Asked Clemens to dismiss a modal in pane P; request queued.",
                "  Last peek at 06:10 confirms modal; prior send ABANDONED after 30s.",
                "  Clearing it permits retry.",
                "  ",
                "  Evidence: .local/x/REPORT.md and memo parking.",
            ]
        );
        assert_eq!(
            cardread_rows(
                "Which layout: vertical keeps panes readable | horizontal shows more context (recommend vertical because readability wins; details: .local/layout.md)"
            ),
            [
                "  Which layout:",
                "  ",
                "  • vertical keeps panes readable",
                "  • horizontal shows more context",
                "  ",
                "  (recommend vertical because readability wins; details: .local/layout.md)",
            ]
        );
        let body = "One claim. A second sentence follows it. Evidence: .local/x.md";
        let lane = Lane {
            items: vec![item(Kind::Said { who: s() }, body)],
            coverage: vec![],
        };
        let rows = super::rows(&lane, 400, Paint::of(None), &view::Style::PLAIN, "lead");
        assert_eq!(
            rows[1..].iter().map(text).collect::<Vec<_>>(),
            [format!("  {body}")],
            "a non-card body stays raw on one row"
        );
    }

    #[test]
    fn cardread_evidence_row_dims_and_options_stay_text() {
        use crate::theme::Look;
        let look = Look::DEFAULT;
        let paint = Paint::of(Some(&look));
        let det = "Pick a slot: morning slot frees the day | evening slot risks a clash. details: .local/slots.md";
        let lane = Lane {
            items: vec![item(Kind::Card { seat: s() }, det)],
            coverage: vec![],
        };
        let rows = super::rows(&lane, 400, paint, &view::Style::PLAIN, "lead");
        let evidence = rows
            .iter()
            .find(|row| text(row).contains("details: .local/slots.md"))
            .expect("an evidence row");
        assert_eq!(evidence.spans[1].style, paint.fg(|p| p.dim));
        let option = rows
            .iter()
            .find(|row| text(row).contains("morning slot frees the day"))
            .expect("an option row");
        assert_eq!(text(option), "  • morning slot frees the day");
        assert_eq!(option.spans[1].style, paint.fg(|p| p.text));
        let sg = "Which layout: vertical keeps panes readable | horizontal shows more context (recommend vertical because readability wins; details: .local/layout.md)";
        let lane = Lane {
            items: vec![item(Kind::Card { seat: s() }, sg)],
            coverage: vec![],
        };
        let rows = super::rows(&lane, 400, paint, &view::Style::PLAIN, "lead");
        let recommend = rows
            .iter()
            .find(|row| text(row).contains("(recommend vertical"))
            .expect("a recommend row");
        assert_eq!(
            recommend.spans[1].style,
            paint.fg(|p| p.text),
            "a recommend line never dims as evidence"
        );
    }

    #[test]
    fn cardread_narrow_rows_wrap_with_bullets_and_neutralise() {
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
                "  ",
                "  • vertical panes stay",
                "    readable\u{fffd}[2J",
                "  • horizontal context",
                "    stays visible",
            ]
        );
        // A labelled option hangs the same way, under the text after its
        // label: continuation rows sit at margin + label width.
        let lane = Lane {
            items: vec![item(
                Kind::Card { seat: s() },
                "Steps. 1 merge the alpha branch today | 2 wait for the beta freeze",
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
                "  Steps.",
                "  ",
                "  1 merge the alpha",
                "    branch today",
                "  2 wait for the beta",
                "    freeze",
            ]
        );
    }

    #[test]
    fn cardread_words_survive_the_app_rows() {
        let words = |text: &str| -> Vec<String> {
            text.split_whitespace()
                .filter(|word| *word != "|")
                .map(str::to_owned)
                .collect()
        };
        let bodies = [
            "Clemens decides: may lead release X (fix, review PASS, tests green) and roll Y only? Yes lets the retest show why it fails | no leaves Y broken with unknown cause. Recommend yes. Asked via say; AGENTS.md:49 needs his go. Memo p2.",
            "Ship 2026.10.6 today, e.g. after lunch. The note src/app.rs:12 explains why. Wait... then roll it. Done?",
            "A | | B",
            "A\x1b[2J bold claim. B safe path | C other path (recommend B)",
            "Héllo wörld. Nächste Frage? Ja bitte | Nein danke.",
            "",
        ];
        for body in bodies {
            let rendered: Vec<String> = cardread_rows(body)
                .iter()
                .map(|row| {
                    let row = row.trim_start();
                    let row = row
                        .strip_prefix("• ")
                        .or_else(|| row.strip_prefix("- "))
                        .unwrap_or(row);
                    row.to_owned()
                })
                .collect();
            assert_eq!(
                words(&rendered.join(" ")),
                words(&crate::board::terminal_text(body)),
                "{body:?}"
            );
        }
    }

    // cardread spec r3: agent-supplied spaces after a label cannot consume
    // the pane. Both renderers keep every label and word inside its width.
    #[test]
    fn cardread_label_whitespace_cannot_push_option_text_outside_pane() {
        use crate::console::view::{Printed, Style};
        use crate::theme::Look;

        let body = "Pick: 1 keep | 2                   wait";
        let lane = Lane {
            items: vec![item(Kind::Card { seat: s() }, body)],
            coverage: vec![],
        };
        let check = |rows: Vec<String>, bar: usize| {
            assert!(!rows.is_empty(), "the body must be drawn");
            for row in &rows {
                assert!(
                    row.chars().count() + bar <= 12,
                    "overflow {} cells: {row:?}",
                    row.chars().count() + bar
                );
            }
            assert_eq!(
                rows.concat()
                    .chars()
                    .filter(|ch| !ch.is_whitespace())
                    .collect::<String>(),
                "Pick:1keep2wait",
                "the labels and all option text survive the bounded rows"
            );
        };
        // Plain with folding counts visual rows; dressed output also wraps.
        for (style, bar) in [
            (Style::PLAIN, 0),
            (Style::resolve(true, Some(Look::DEFAULT), None, "lead"), 1),
        ] {
            let mut printed = Printed::styled(style);
            printed.set_width(Some(12));
            printed.set_fold(true);
            let shown = printed.step(&lane, 0, true);
            let mut plain = String::new();
            let mut chars = shown.chars();
            while let Some(ch) = chars.next() {
                if ch == '\x1b' {
                    for next in chars.by_ref() {
                        if next == 'm' {
                            break;
                        }
                    }
                } else {
                    plain.push(ch);
                }
            }
            let rows: Vec<String> = plain
                .lines()
                .map(|row| row.strip_prefix('▌').unwrap_or(row))
                .skip_while(|row| !row.contains("Pick:"))
                .map(str::to_owned)
                .collect();
            check(rows, bar);
        }
        let rows = super::rows(&lane, 12, Paint::of(None), &Style::PLAIN, "lead");
        check(rows[1..].iter().map(text).collect(), 0);
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
                    via: None,
                },
                None,
            ),
            (
                Kind::Asked {
                    to: s(),
                    id: s(),
                    uncertain: false,
                    gap: Some("body missing".to_owned()),
                    via: None,
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
            icons: true,
            base: &crate::console::link::Base::default(),
        };
        super::tail_with(&lane, 40, Paint::of(None), &show, 100, None)
    }

    fn rows_of(shown: &super::Tail) -> Vec<String> {
        shown.rows.iter().map(text).collect()
    }

    #[test]
    fn a_long_turn_folds_to_twenty_four_rows_and_its_fold_row_toggles_it() {
        let lines = |n: usize| (1..=n).map(|n| format!("l{n}")).collect::<Vec<_>>();
        let turn =
            |n: usize, open: Option<&[bool]>| shown_with(said(&lines(n).join("\n")), open, false);
        // Thirty rows show whole and a lane that folds nothing never has a fold row.
        for shown in [turn(30, Some(&[false])), turn(32, None)] {
            assert_eq!(shown.turns[0].fold, None);
        }
        assert_eq!(rows_of(&turn(30, Some(&[false]))).len(), 31);
        let folded = turn(31, Some(&[false]));
        let rows = rows_of(&folded);
        assert_eq!(folded.turns[0].fold, Some(25));
        assert_eq!(rows[24], "  l24");
        assert_eq!(rows[25], "  ... 7 more lines");
        assert_eq!(rows.len(), 26);
        let open = turn(32, Some(&[true]));
        let rows = rows_of(&open);
        assert_eq!((rows.len(), open.turns[0].fold), (34, Some(33)));
        assert_eq!(
            (rows[32].as_str(), rows[33].as_str()),
            ("  l32", "  show fewer lines")
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
                via: None,
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
