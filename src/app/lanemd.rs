//! The chat lane's agent turns through the reader's Markdown: which turns
//! render, and a bounded cache of the semantic rows they lay out to.
//!
//! PURE. The cache keeps [`Rendered`] rows (text and marks), never styled
//! spans, so a look switch repaints from the same parse. A turn renders whole
//! or not at all: any part that cannot, or a turn over its caps, hands the
//! caller `None` and the caller draws today's plain rows.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::mem::size_of;
use std::rc::Rc;

use ratatui_core::style::{Modifier, Style};
use ratatui_core::text::{Line, Span};

use super::draw::{Paint, markdown_style};
use super::lane::Links;
use super::markdown::{Budget, Document, Marks, Opts, Rendered};
use crate::board::terminal_text;
use crate::console::lane::Kind;
use crate::console::link;

/// The most source bytes one turn may hand the renderer, summed over parts.
const MAX_SOURCE: usize = 64 * 1024;
/// The narrowest content width a turn renders at; narrower draws plain.
const MIN_WIDTH: usize = 20;
/// The cache holds at most this many entries ...
const MAX_ENTRIES: usize = 1024;
/// ... and at most this many bytes of [`Cache::retained`] ...
const MAX_BYTES: usize = 8 * 1024 * 1024;
/// ... and never keeps one entry costing more than this.
const ENTRY_CAP: usize = 2 * 1024 * 1024;
/// What a map bucket keeps allocated, its control byte included ...
const BUCKET: usize = size_of::<((u64, usize), Entry)>() + 1;
/// ... and the control bytes the allocation repeats past its last bucket.
const CONTROL_TAIL: usize = 16;
/// The counts an `Rc` keeps ahead of its value.
const RC_HEADER: usize = 2 * size_of::<usize>();

/// Whether `kind` is something an agent wrote whose whole text the lane holds,
/// a decision card's parts included. A journal summary, a human's line and
/// ae's own notice stay plain.
pub(super) fn eligible(kind: &Kind) -> bool {
    match kind {
        Kind::Assistant { .. } | Kind::Answer { gap: None, .. } | Kind::Card { .. } => true,
        Kind::Said { who } => who != "ae",
        _ => false,
    }
}

/// One source text laid out at one width: the rows, or the refusal that sent
/// the turn back to plain rows.
#[derive(Debug)]
struct Entry {
    source: Box<str>,
    rendered: Option<Rc<Rendered>>,
    cost: usize,
    tick: u64,
}

/// The cache of semantic rows, keyed by the source text and the content width.
#[derive(Debug)]
pub(super) struct Cache {
    map: HashMap<(u64, usize), Entry>,
    bytes: usize,
    /// The most buckets the map has held. It keeps them after an eviction, and
    /// its tombstones lower [`HashMap::capacity`] without freeing any.
    buckets: usize,
    tick: u64,
    key: fn(&str) -> u64,
    #[cfg(test)]
    parses: usize,
    #[cfg(test)]
    layouts: usize,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            map: HashMap::new(),
            bytes: 0,
            buckets: 0,
            tick: 0,
            key: hash_text,
            #[cfg(test)]
            parses: 0,
            #[cfg(test)]
            layouts: 0,
        }
    }
}

fn hash_text(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// `text` parsed and laid out at `width` in the lane's budget; `None` when it
/// refuses or draws nothing though it is not blank.
fn lay_out(text: &str, width: usize) -> Option<Rc<Rendered>> {
    let opts = Opts {
        soft_break_row: true,
    };
    let rendered = Document::parse(text).layout_in(width, &Budget::LANE, opts);
    let drew = rendered.rows() > 0 || text.trim().is_empty();
    (rendered.refusal().is_none() && drew).then(|| Rc::new(rendered))
}

impl Cache {
    /// The semantic rows of one part's `text` at content `width`, in the lane's
    /// budget and soft-break mode. `None` is a refusal: a width under
    /// [`MIN_WIDTH`] or a text over [`MAX_SOURCE`] (neither parses), a layout
    /// over the budget, or a non-blank text that draws no row. A refusal the
    /// parse reached is remembered. A render dearer than [`ENTRY_CAP`] is
    /// returned and not kept.
    pub(super) fn render(&mut self, text: &str, width: usize) -> Option<Rc<Rendered>> {
        if width < MIN_WIDTH || text.len() > MAX_SOURCE {
            return None;
        }
        let key = ((self.key)(text), width);
        self.tick += 1;
        if let Some(entry) = self.map.get_mut(&key)
            && *entry.source == *text
        {
            entry.tick = self.tick;
            return entry.rendered.clone();
        }
        self.forget(&key);
        let rendered = lay_out(text, width);
        #[cfg(test)]
        {
            self.parses += 1;
            self.layouts += 1;
        }
        let entry = Entry {
            source: text.into(),
            cost: size_of::<Entry>()
                + text.len()
                + rendered
                    .as_ref()
                    .map_or(0, |r| RC_HEADER + r.retained_bytes()),
            rendered,
            tick: self.tick,
        };
        let shown = entry.rendered.clone();
        if entry.cost <= ENTRY_CAP {
            self.keep(key, entry);
        }
        shown
    }

    fn forget(&mut self, key: &(u64, usize)) {
        if let Some(old) = self.map.remove(key) {
            self.bytes -= old.cost;
        }
    }

    /// What the cache keeps allocated: its entries and the map's buckets.
    fn retained(&self) -> usize {
        let map = match self.buckets {
            0 => 0,
            buckets => buckets * BUCKET + CONTROL_TAIL,
        };
        self.bytes + map
    }

    /// Insert `entry`, evicting the least recently used until it fits. The map
    /// grows first, so the bound already holds its buckets.
    fn keep(&mut self, key: (u64, usize), entry: Entry) {
        self.map.reserve(1);
        // Seven buckets in eight are usable, and a map has a power of two.
        let buckets = (self.map.capacity() * 8).div_ceil(7).next_power_of_two();
        self.buckets = self.buckets.max(buckets);
        while self.map.len() >= MAX_ENTRIES || self.retained() + entry.cost > MAX_BYTES {
            let Some(oldest) = (self.map.iter())
                .min_by_key(|(_, held)| held.tick)
                .map(|(held, _)| *held)
            else {
                break;
            };
            self.forget(&oldest);
        }
        self.bytes += entry.cost;
        self.map.insert(key, entry);
    }

    /// `(entries, retained bytes, parses, layouts)`.
    #[cfg(test)]
    pub(super) fn test_stats(&self) -> (usize, usize, usize, usize) {
        (self.map.len(), self.retained(), self.parses, self.layouts)
    }

    /// The buckets the map keeps allocated.
    #[cfg(test)]
    #[allow(dead_code, reason = "read by the cache's regression spec")]
    pub(super) fn test_map_capacity(&self) -> usize {
        self.map.capacity()
    }

    /// Key texts with `key` from now on, to force a collision.
    #[cfg(test)]
    pub(super) fn test_key_with(&mut self, key: fn(&str) -> u64) {
        self.key = key;
    }
}

/// Every `(text, content width)` of one turn's parts through the cache, in
/// order, or `None` when any part refuses or the turn goes over its caps:
/// the sum of the sources, of the rows or of the cells. The caller then draws
/// the whole turn plain.
pub(super) fn turn(cache: &RefCell<Cache>, parts: &[(&str, usize)]) -> Option<Vec<Rc<Rendered>>> {
    if parts.iter().map(|(text, _)| text.len()).sum::<usize>() > MAX_SOURCE {
        return None;
    }
    let (mut rows, mut cells) = (0, 0);
    let mut drawn = Vec::with_capacity(parts.len());
    for (text, width) in parts {
        let part = cache.borrow_mut().render(text, *width)?;
        rows += part.rows();
        cells += part.cells();
        if rows > Budget::LANE.rows || cells > Budget::LANE.cells {
            return None;
        }
        drawn.push(part);
    }
    Some(drawn)
}

/// What leads a part's drawn rows after the two-cell margin: `mark` on the
/// first (an option's label or bullet), `hang` blanks on the others.
pub(super) struct Lead<'a> {
    pub(super) mark: &'a str,
    pub(super) hang: usize,
}

/// One drawn row of a part: its cells, the links on them and its file
/// occurrences as column, cells, ordinal within the part and target.
pub(super) struct MdRow {
    pub(super) line: Line<'static>,
    pub(super) links: Links,
    pub(super) files: Vec<(usize, usize, usize, link::FileRef)>,
}

/// A link of one displayed line, with the ordinal and target of its file.
type Found = (link::Link, Option<(usize, link::FileRef)>);

/// A link placed on one row: its byte range in the row's text, its URL and
/// its file.
type Hit = (usize, usize, String, Option<(usize, link::FileRef)>);

/// The rows of one rendered part as the lane draws them. Links and file
/// occurrences are found once per displayed line, as the chat finds them, then
/// placed on every row their line piece overlaps; a path a row break cuts is
/// one ordinal on each of its rows. A row is one walk along its sorted mark
/// changes and links.
pub(super) fn rows(
    r: &Rendered,
    lead: &Lead<'_>,
    paint: Paint,
    hue: Style,
    base: &link::Base,
) -> Vec<MdRow> {
    let mut files = 0;
    let found: Vec<Vec<Found>> = (r.lines().iter())
        .map(|line| {
            (link::found(line, base).into_iter())
                .map(|(link, file)| {
                    let file = file.map(|file| {
                        files += 1;
                        (files - 1, file)
                    });
                    (link, file)
                })
                .collect()
        })
        .collect();
    let quiet = hue != paint.fg(|p| p.text);
    (0..r.rows())
        .map(|row| {
            let text = r.text(row).unwrap_or_default();
            let mut hits: Vec<Hit> = Vec::new();
            if let Some(cut) = r.cut(row).filter(|_| !text.is_empty()) {
                let line = found.get(cut.line).map_or(&[][..], Vec::as_slice);
                let end = cut.from + cut.len;
                let first = line.partition_point(|(link, _)| link.end <= cut.from);
                for (link, file) in
                    (line.iter().skip(first)).take_while(|(link, _)| link.start < end)
                {
                    let (start, stop) = (link.start.max(cut.from), link.end.min(end));
                    let (a, b) = (cut.gap + start - cut.from, cut.gap + stop - cut.from);
                    if link::is_safe_url(&link.url) && text.get(a..b).is_some_and(|t| !t.is_empty())
                    {
                        hits.push((a, b, link.url.clone(), file.clone()));
                    }
                }
            }
            let mut spans = vec![Span::raw("  ")];
            let mut drawn = MdRow {
                line: Line::default(),
                links: Vec::new(),
                files: Vec::new(),
            };
            // A code row's ground runs to the render width, as the reader's does.
            let end = r.marks(row, text.len());
            let blanks = if end.code_block { r.pad(row) } else { 0 };
            let pad = Span::styled(" ".repeat(blanks), styled(end, paint, (hue, quiet), false));
            if text.is_empty() {
                spans.push(pad);
                drawn.line = Line::from(spans);
                return drawn;
            }
            let front = if row == 0 {
                terminal_text(lead.mark)
            } else {
                " ".repeat(lead.hang)
            };
            let lead_cells = Span::raw(front.clone()).width();
            if !front.is_empty() {
                spans.push(Span::styled(front, hue));
            }
            let runs = r.runs(row);
            let mut edges: Vec<usize> = std::iter::once(0)
                .chain(runs.iter().map(|(at, _)| *at))
                .chain(hits.iter().flat_map(|(a, b, ..)| [*a, *b]))
                .chain([text.len()])
                .filter(|at| *at <= text.len() && text.is_char_boundary(*at))
                .collect();
            edges.sort_unstable();
            edges.dedup();
            let (mut mark, mut run, mut live, mut cells) = (Marks::default(), 0, 0, 0);
            for pair in edges.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                while let Some((_, marks)) = runs.get(run).filter(|(at, _)| *at <= a) {
                    mark = *marks;
                    run += 1;
                }
                while hits.get(live).is_some_and(|(_, end, ..)| *end <= a) {
                    live += 1;
                }
                let hit = hits.get(live).filter(|(start, ..)| *start <= a);
                if let Some((start, end, url, file)) = hit
                    && *start == a
                {
                    let (column, width) =
                        (2 + lead_cells + cells, Span::raw(&text[a..*end]).width());
                    if let Some((ordinal, file)) = file {
                        drawn.files.push((column, width, *ordinal, file.clone()));
                    }
                    drawn.links.push((column, width, url.clone()));
                }
                let style = styled(mark, paint, (hue, quiet), hit.is_some());
                spans.push(Span::styled(terminal_text(&text[a..b]), style));
                cells += Span::raw(&text[a..b]).width();
            }
            if blanks > 0 {
                spans.push(pad);
            }
            drawn.line = Line::from(spans);
            drawn
        })
        .collect()
}

/// A cell run's style: the reader's, in the turn's ink where that is dimmed,
/// and underlined inside a link where the look is drawn.
fn styled(mark: Marks, paint: Paint, (hue, quiet): (Style, bool), linked: bool) -> Style {
    let mut style = markdown_style(mark, paint);
    if quiet && mark.heading.is_none() && !mark.target && !mark.note {
        style = style.patch(hue);
    }
    if linked && paint.drawn() {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    style
}

/// Every file occurrence a part draws, in display order: the ordinals
/// [`rows`] gives.
pub(super) fn refs(r: &Rendered, base: &link::Base) -> Vec<link::FileRef> {
    (r.lines().iter())
        .flat_map(|line| link::found(line, base))
        .filter_map(|(_, file)| file)
        .collect()
}
