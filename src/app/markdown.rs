//! Inert Markdown: offset events cached once, bounded semantic rows per width.
use std::ops::Range;

use crate::console::wrap::split_row;
use crate::event_text::cell;
use pulldown_cmark::{Alignment, Event, LinkType, Options, Parser, Tag, TagEnd};

use super::reader;
use crate::board::terminal_text;

pub const MAX_ROWS: usize = 50_000;
pub const MAX_CELLS: usize = 2_000_000;
pub const MAX_COLUMNS: usize = 32;
pub const MAX_TABLE_WIDTH: usize = 240;
pub const MAX_DEPTH: usize = 32;

/// Independent semantic attributes, combined for nested inline spans.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent Markdown marks compose"
)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Marks {
    pub heading: Option<u8>,
    pub strong: bool,
    pub emphasis: bool,
    pub strike: bool,
    pub code: bool,
    pub code_block: bool,
    pub quote: bool,
    pub link: bool,
    pub target: bool,
    pub html: bool,
    pub note: bool,
}

#[must_use]
pub fn is_markdown(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, ext)| {
        ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")
    })
}

#[derive(Debug, Default)]
struct Row {
    text: String,
    marks: Vec<(usize, Marks)>,
    line: usize,
}

/// Output owns only budgeted rows; a global refusal discards all of them.
#[derive(Debug, Default)]
pub struct Rendered {
    rows: Vec<Row>,
    cells: usize,
    notes: usize,
    refusal: Option<&'static str>,
    gap: bool,
}

impl Rendered {
    #[must_use]
    pub fn rows(&self) -> usize {
        self.rows.len()
    }
    #[must_use]
    pub fn text(&self, row: usize) -> Option<&str> {
        self.rows.get(row).map(|r| r.text.trim_end_matches(' '))
    }
    #[must_use]
    pub fn marks(&self, row: usize, byte: usize) -> Marks {
        self.rows
            .get(row)
            .and_then(|r| r.marks.iter().rev().find(|(at, _)| *at <= byte))
            .map_or_else(Marks::default, |(_, marks)| *marks)
    }
    #[must_use]
    pub fn line_of(&self, row: usize) -> usize {
        self.rows.get(row).map_or(0, |r| r.line)
    }
    #[must_use]
    pub fn row_of(&self, line: usize) -> usize {
        let end = self.rows.partition_point(|r| r.line < line.max(1));
        let anchor = self.rows.get(end.saturating_sub(1)).map_or(0, |r| r.line);
        self.rows.partition_point(|r| r.line < anchor)
    }
    #[must_use]
    pub fn notes(&self) -> usize {
        self.notes
    }
    #[must_use]
    pub fn refusal(&self) -> Option<&'static str> {
        self.refusal
    }

    /// Ask for one blank row before the next row: never first, never last.
    fn separate(&mut self) {
        self.gap = true;
    }

    fn push(&mut self, mut row: Row, cells: usize) -> bool {
        if std::mem::take(&mut self.gap) && !self.rows.is_empty() && !self.push(Row::default(), 0) {
            return false;
        }
        let why = if self.rows.len() >= MAX_ROWS {
            Some("too large to render (rows limit)")
        } else if cells > MAX_CELLS.saturating_sub(self.cells) {
            Some("too large to render (cells limit)")
        } else {
            None
        };
        if let Some(why) = why {
            self.rows.clear();
            self.cells = 0;
            self.notes = 0;
            self.refusal = Some(why);
            return false;
        }
        row.line = row.line.max(self.rows.last().map_or(0, |r| r.line));
        self.cells += cells;
        self.rows.push(row);
        true
    }
}

type Events = Vec<(Event<'static>, Range<usize>)>;

#[derive(Debug)]
struct Block {
    range: Range<usize>,
    events: Events,
}

/// Width-free parser result: a resize never reparses the source.
#[derive(Debug)]
pub(crate) struct Document {
    source: String,
    starts: Vec<usize>,
    blocks: Vec<Block>,
}

impl Document {
    pub(crate) fn parse(source: &str) -> Self {
        let mut doc = Self {
            source: source.to_owned(),
            starts: vec![0],
            blocks: Vec::new(),
        };
        doc.starts
            .extend(source.match_indices('\n').map(|(at, _)| at + 1));
        let mut depth = 0usize;
        for (event, range) in Parser::new_ext(
            source,
            Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH,
        )
        .into_offset_iter()
        {
            if depth == 0 {
                doc.blocks.push(Block {
                    range: range.clone(),
                    events: Vec::new(),
                });
            }
            if matches!(event, Event::Start(_)) {
                depth += 1;
            }
            if matches!(event, Event::End(_)) {
                depth = depth.saturating_sub(1);
            }
            if let Some(block) = doc.blocks.last_mut() {
                block.range.end = block.range.end.max(range.end);
                block.events.push((event.into_static(), range));
            }
        }
        doc
    }

    fn line(&self, byte: usize) -> usize {
        self.starts
            .partition_point(|at| *at <= byte)
            .saturating_sub(1)
    }

    pub(crate) fn layout(&self, width: usize) -> Rendered {
        let width = width.clamp(1, 4096);
        let mut out = Rendered::default();
        for block in &self.blocks {
            let mut depth = 0usize;
            let mut deepest = 0;
            for (event, _) in &block.events {
                match event {
                    Event::Start(Tag::BlockQuote(_) | Tag::Item) => {
                        depth += 1;
                        deepest = deepest.max(depth);
                    }
                    Event::End(TagEnd::BlockQuote(_) | TagEnd::Item) => {
                        depth = depth.saturating_sub(1);
                    }
                    _ => {}
                }
            }
            let columns = block
                .events
                .iter()
                .filter_map(|(event, _)| match event {
                    Event::Start(Tag::Table(align)) => Some(align.len()),
                    _ => None,
                })
                .max()
                .unwrap_or(0);
            let mapped = if deepest > MAX_DEPTH || columns > MAX_COLUMNS {
                Mapped::default()
            } else {
                Mapper::new(self, block).map()
            };
            let reason = if deepest > MAX_DEPTH {
                Some("nested deeper than 32".to_owned())
            } else if columns > MAX_COLUMNS {
                Some(format!("table has {columns} columns (limit 32)"))
            } else {
                mapped.table_reason()
            };
            let raw = reason.is_some() || mapped.too_wide(width);
            if let Some(reason) = reason {
                let marks = Marks {
                    note: true,
                    ..Marks::default()
                };
                if !out.wrap(
                    &format!("raw: {reason}"),
                    &[(0, marks)],
                    self.line(block.range.start),
                    width,
                    0,
                    false,
                ) {
                    return out;
                }
                out.notes += 1;
            }
            if raw {
                for (step, text) in self.source[block.range.clone()]
                    .trim_end_matches('\n')
                    .split('\n')
                    .enumerate()
                {
                    if !out.wrap(
                        &clean(text),
                        &[],
                        self.line(block.range.start) + step,
                        width,
                        0,
                        false,
                    ) {
                        return out;
                    }
                }
            } else if !mapped.layout(&mut out, width) {
                return out;
            }
            out.separate();
        }
        out
    }
}

#[must_use]
pub fn render(source: &str, width: usize) -> Rendered {
    Document::parse(source).layout(width)
}

fn clean(text: &str) -> String {
    terminal_text(text).replace('\t', "    ")
}
fn cells(text: &str) -> usize {
    text.chars().map(cell).sum()
}

impl Row {
    fn add(&mut self, text: &str, marks: Marks) {
        if self.marks.last().is_none_or(|(_, old)| *old != marks) {
            self.marks.push((self.text.len(), marks));
        }
        self.text.push_str(text);
    }
}

impl Rendered {
    // Reuse the lane's word cutter, emitting one budgeted row at a time.
    fn wrap(
        &mut self,
        text: &str,
        marks: &[(usize, Marks)],
        line: usize,
        width: usize,
        hang: usize,
        code: bool,
    ) -> bool {
        let lead = text.len() - text.trim_start_matches(' ').len();
        let mut rest = if code { text } else { &text[lead..] };
        let mut gap = if code {
            0
        } else {
            lead.min(width.saturating_sub(2))
        };
        loop {
            let (piece, tail) = if code {
                let mut used = 0;
                let end = rest
                    .char_indices()
                    .find_map(|(at, ch)| {
                        used += cell(ch);
                        (at > 0 && used > width).then_some(at)
                    })
                    .unwrap_or(rest.len());
                rest.split_at(end)
            } else {
                split_row(rest, width.saturating_sub(gap))
            };
            let offset = text.len() - rest.len();
            let mut row = Row {
                line,
                ..Row::default()
            };
            row.add(&" ".repeat(gap), Marks::default());
            let at = marks.partition_point(|(at, _)| *at <= offset);
            row.add(
                piece,
                at.checked_sub(1)
                    .map_or_else(Marks::default, |at| marks[at].1),
            );
            row.marks.extend(
                marks
                    .iter()
                    .skip(at)
                    .take_while(|(at, _)| *at < offset + piece.len())
                    .map(|(at, mark)| (gap + at - offset, *mark)),
            );
            if !self.finish_row(row, gap + cells(piece), width, code) {
                return false;
            }
            if tail.is_empty() {
                return true;
            }
            rest = tail;
            gap = if code {
                0
            } else {
                hang.min(width.saturating_sub(2))
            };
        }
    }

    fn finish_row(&mut self, mut row: Row, col: usize, width: usize, code: bool) -> bool {
        if code {
            // Charge padding before allocating it; marks include every pad cell.
            let count = col.max(width);
            if count > MAX_CELLS.saturating_sub(self.cells) || self.rows.len() >= MAX_ROWS {
                return self.push(row, count);
            }
            row.add(
                &" ".repeat(width.saturating_sub(col)),
                Marks {
                    code_block: true,
                    ..Marks::default()
                },
            );
            self.push(row, count)
        } else {
            self.push(row, col)
        }
    }
}

#[derive(Default)]
struct Mapped {
    items: Vec<Item>,
}

enum Item {
    Line(Row, usize, bool),
    Table(Table),
    Rule(usize),
    Gap,
}

/// One container's sibling state: a loose scope blank-separates its blocks.
struct Scope {
    loose: bool,
    seen: bool,
}

impl Scope {
    fn of(loose: bool) -> Self {
        Self { loose, seen: false }
    }
}

enum Open {
    List(usize, bool),
    Item(usize, bool),
    Quote,
}

/// A blank line of the list's own quote depth lies between `from` and `to`.
fn blank_between(src: &str, from: usize, to: usize, quotes: usize) -> bool {
    let end = src.get(..from).map_or(0, |s| s.trim_end().len());
    src.get(end..to).is_some_and(|gap| {
        let middle = gap.matches('\n').count().saturating_sub(1);
        gap.split('\n').skip(1).take(middle).any(|line| {
            line.matches('>').count() == quotes
                && line.chars().all(|ch| ch == '>' || ch.is_whitespace())
        })
    })
}

/// pulldown-cmark exposes no tightness: a list is loose when a blank line separates
/// two of its items or two blocks directly inside one item (`CommonMark` 5.3).
fn loose_lists(src: &str, events: &Events) -> Vec<bool> {
    let mut loose = Vec::new();
    let mut open = Vec::new();
    let mut leaf = 0;
    for (event, range) in events {
        let apart = match open.last_mut() {
            Some(Open::List(at, seen)) if matches!(event, Event::Start(Tag::Item)) => {
                std::mem::replace(seen, true).then_some(*at)
            }
            Some(Open::Item(at, first)) if matches!(event, Event::Start(_) | Event::Rule) => {
                loose[*at] |= matches!(event, Event::Start(Tag::Paragraph));
                (!std::mem::replace(first, false)).then_some(*at)
            }
            _ => None,
        };
        if let Some(at) = apart {
            let quotes = open.iter().filter(|o| matches!(o, Open::Quote)).count();
            loose[at] |= blank_between(src, leaf, range.start, quotes);
        }
        match event {
            Event::Start(Tag::List(_)) => {
                open.push(Open::List(loose.len(), false));
                loose.push(false);
            }
            Event::Start(Tag::Item) => {
                if let Some(Open::List(at, _)) = open.last() {
                    open.push(Open::Item(*at, true));
                }
            }
            Event::Start(Tag::BlockQuote(_)) => open.push(Open::Quote),
            Event::End(TagEnd::List(_) | TagEnd::Item | TagEnd::BlockQuote(_)) => {
                open.pop();
            }
            Event::Start(_) => {}
            _ => {
                leaf = leaf.max(range.end);
                if let Some(Open::Item(_, first)) = open.last_mut() {
                    *first = false;
                }
            }
        }
    }
    loose
}

struct Table {
    rows: Vec<Vec<Row>>,
    align: Vec<Alignment>,
    hang: usize,
}

impl Mapped {
    fn tables(&self) -> impl Iterator<Item = &Table> {
        self.items.iter().filter_map(|item| match item {
            Item::Table(table) => Some(table),
            _ => None,
        })
    }
    fn table_reason(&self) -> Option<String> {
        self.tables().find_map(Table::reason)
    }
    fn too_wide(&self, width: usize) -> bool {
        self.tables().any(|table| table.width() > width)
    }
    fn layout(self, out: &mut Rendered, width: usize) -> bool {
        self.items.into_iter().all(|item| match item {
            Item::Line(row, hang, code) => {
                out.wrap(&row.text, &row.marks, row.line, width, hang, code)
            }
            Item::Rule(line) => out.wrap(&"-".repeat(width), &[], line, width, 0, false),
            Item::Table(table) => table.layout(out),
            Item::Gap => {
                out.separate();
                true
            }
        })
    }
    fn table_mut(&mut self) -> Option<&mut Table> {
        match self.items.last_mut() {
            Some(Item::Table(table)) => Some(table),
            _ => None,
        }
    }
}

impl Table {
    fn widths(&self) -> Vec<usize> {
        (0..self.align.len())
            .map(|col| {
                self.rows
                    .iter()
                    .filter_map(|r| r.get(col))
                    .map(|r| cells(&r.text))
                    .max()
                    .unwrap_or(0)
            })
            .collect()
    }
    fn width(&self) -> usize {
        self.widths().iter().sum::<usize>() + self.align.len() * 3 + 1 + self.hang
    }
    fn reason(&self) -> Option<String> {
        if self.align.len() > MAX_COLUMNS {
            Some(format!("table has {} columns (limit 32)", self.align.len()))
        } else if self.width() > MAX_TABLE_WIDTH {
            Some("table wider than 240 cells".to_owned())
        } else {
            None
        }
    }
    fn layout(self, out: &mut Rendered) -> bool {
        let widths = self.widths();
        for table_row in self.rows {
            let mut row = Row {
                line: table_row.first().map_or(0, |r| r.line),
                ..Row::default()
            };
            row.add(&format!("{}|", " ".repeat(self.hang)), Marks::default());
            for (col, cell) in table_row.iter().enumerate() {
                let pad = widths[col].saturating_sub(cells(&cell.text));
                let left = match self.align[col] {
                    Alignment::Right => pad,
                    Alignment::Center => pad / 2,
                    _ => 0,
                };
                row.add(&" ".repeat(left + 1), Marks::default());
                let base = row.text.len();
                row.text.push_str(&cell.text);
                row.marks
                    .extend(cell.marks.iter().map(|(at, mark)| (base + at, *mark)));
                row.add(
                    &format!("{}|", " ".repeat(pad - left + 1)),
                    Marks::default(),
                );
            }
            let count = cells(&row.text);
            if !out.push(row, count) {
                return false;
            }
        }
        true
    }
}

struct Mapper<'a> {
    doc: &'a Document,
    block: &'a Block,
    out: Mapped,
    row: Row,
    marks: Marks,
    saved: Vec<Marks>,
    links: Vec<(String, bool, bool)>,
    lists: Vec<Option<u64>>,
    scopes: Vec<Scope>,
    loose: std::vec::IntoIter<bool>,
    quotes: usize,
    prefix: String,
    hang: usize,
    code: bool,
}

impl<'a> Mapper<'a> {
    fn new(doc: &'a Document, block: &'a Block) -> Self {
        Self {
            doc,
            block,
            out: Mapped::default(),
            row: Row::default(),
            marks: Marks::default(),
            saved: Vec::new(),
            links: Vec::new(),
            lists: Vec::new(),
            scopes: Vec::new(),
            loose: loose_lists(&doc.source, &block.events).into_iter(),
            quotes: 0,
            prefix: String::new(),
            hang: 0,
            code: false,
        }
    }
    fn flush(&mut self) {
        if !self.row.text.is_empty() {
            self.out.items.push(Item::Line(
                std::mem::take(&mut self.row),
                self.hang,
                self.code,
            ));
            self.prefix = " ".repeat(self.hang);
        }
    }
    fn gap(&mut self) {
        self.flush();
        self.out.items.push(Item::Gap);
    }
    /// A block starts. `set_off` blocks (code, table, quote, rule, heading) always
    /// get a blank row before them; the rest only after a sibling in a loose scope.
    fn block(&mut self, set_off: bool) {
        if set_off || self.scopes.last().is_some_and(|s| s.seen && s.loose) {
            self.gap();
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.seen = true;
        }
    }
    fn indent(&mut self) {
        self.hang = 2 * (self.quotes + self.lists.len())
            + self
                .lists
                .last()
                .and_then(|n| *n)
                .map_or(0, |n| n.saturating_sub(1).to_string().len());
        self.prefix = " ".repeat(self.hang);
    }
    fn add(&mut self, text: &str, marks: Marks) {
        if self.row.text.is_empty() {
            self.row.add(
                &self.prefix,
                Marks {
                    quote: self.quotes > 0,
                    code_block: self.code,
                    ..Marks::default()
                },
            );
        }
        self.row.add(&clean(text), marks);
    }
    #[allow(
        clippy::too_many_lines,
        reason = "one exhaustive event dispatcher keeps paired state transitions visible"
    )]
    fn map(mut self) -> Mapped {
        for (event, range) in &self.block.events {
            match event {
                Event::Start(tag) => {
                    match tag {
                        Tag::Paragraph | Tag::Heading { .. } | Tag::CodeBlock(_) => {
                            self.block(!matches!(tag, Tag::Paragraph));
                            self.flush();
                            self.row.line = self.doc.line(range.start);
                        }
                        Tag::HtmlBlock => self.block(false),
                        Tag::Item => {
                            self.block(false);
                            self.scopes
                                .push(Scope::of(self.scopes.last().is_some_and(|s| s.loose)));
                            self.flush();
                            self.row.line = self.doc.line(range.start);
                            let bullet = match self.lists.last_mut() {
                                Some(Some(n)) => {
                                    let text = format!("{n}. ");
                                    *n = n.saturating_add(1);
                                    text
                                }
                                _ => "- ".to_owned(),
                            };
                            self.prefix = format!(
                                "{}{bullet}",
                                "  ".repeat(self.quotes + self.lists.len().saturating_sub(1))
                            );
                            self.hang = cells(&self.prefix);
                        }
                        Tag::BlockQuote(_) => {
                            self.block(true);
                            self.scopes.push(Scope::of(true));
                            self.quotes += 1;
                            self.marks.quote = true;
                            self.indent();
                        }
                        Tag::List(start) => {
                            self.block(false);
                            self.scopes
                                .push(Scope::of(self.loose.next().unwrap_or(false)));
                            self.lists.push(*start);
                        }
                        Tag::Table(align) => {
                            self.block(true);
                            self.flush();
                            self.out.items.push(Item::Table(Table {
                                rows: Vec::new(),
                                align: align.clone(),
                                hang: self.hang,
                            }));
                            self.prefix.clear();
                        }
                        Tag::TableHead | Tag::TableRow => {
                            if let Some(table) = self.out.table_mut() {
                                table.rows.push(Vec::new());
                            }
                        }
                        Tag::TableCell => {
                            self.row = Row {
                                line: self.doc.line(range.start),
                                ..Row::default()
                            };
                        }
                        _ => {}
                    }
                    match tag {
                        Tag::Heading { level, .. } => self.marks.heading = Some(*level as u8),
                        Tag::CodeBlock(_) => {
                            self.code = true;
                            self.marks.code_block = true;
                        }
                        Tag::Emphasis
                        | Tag::Strong
                        | Tag::Strikethrough
                        | Tag::Link { .. }
                        | Tag::Image { .. } => {
                            self.saved.push(self.marks);
                            match tag {
                                Tag::Emphasis => self.marks.emphasis = true,
                                Tag::Strong => self.marks.strong = true,
                                Tag::Strikethrough => self.marks.strike = true,
                                Tag::Link {
                                    link_type,
                                    dest_url,
                                    ..
                                } => {
                                    self.marks.link = true;
                                    self.links.push((
                                        clean(dest_url),
                                        matches!(link_type, LinkType::Autolink | LinkType::Email),
                                        false,
                                    ));
                                }
                                Tag::Image { dest_url, .. } => {
                                    self.add("[image: ", self.marks);
                                    self.links.push((clean(dest_url), false, true));
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
                Event::End(tag) => match tag {
                    TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::HtmlBlock => {
                        self.flush();
                        self.marks.heading = None;
                        if matches!(tag, TagEnd::Heading(_)) {
                            self.gap();
                        }
                    }
                    TagEnd::CodeBlock => {
                        self.flush();
                        self.gap();
                        self.code = false;
                        self.marks.code_block = false;
                    }
                    TagEnd::Item => {
                        self.flush();
                        self.scopes.pop();
                    }
                    TagEnd::List(_) => {
                        self.lists.pop();
                        self.scopes.pop();
                        self.indent();
                    }
                    TagEnd::BlockQuote(_) => {
                        self.flush();
                        self.gap();
                        self.scopes.pop();
                        self.quotes = self.quotes.saturating_sub(1);
                        self.marks.quote = self.quotes > 0;
                        self.indent();
                    }
                    TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                        self.marks = self.saved.pop().unwrap_or_default();
                    }
                    TagEnd::Link | TagEnd::Image => {
                        if let Some((target, auto, image)) = self.links.pop()
                            && !auto
                        {
                            self.add(
                                &format!("{}{target})", if image { "](" } else { " (" }),
                                Marks {
                                    target: true,
                                    ..self.marks
                                },
                            );
                        }
                        self.marks = self.saved.pop().unwrap_or_default();
                    }
                    TagEnd::TableCell => {
                        if let Some(row) = self.out.table_mut().and_then(|t| t.rows.last_mut()) {
                            row.push(std::mem::take(&mut self.row));
                        }
                    }
                    TagEnd::Table => {
                        self.gap();
                        self.indent();
                    }
                    _ => {}
                },
                Event::Text(text) if self.code => {
                    for text in text.split_inclusive('\n') {
                        self.add(
                            if text == "\n" {
                                " "
                            } else {
                                text.trim_end_matches('\n')
                            },
                            self.marks,
                        );
                        if text.ends_with('\n') {
                            self.flush();
                        }
                    }
                }
                Event::Text(text) => self.add(text, self.marks),
                Event::Code(text) => self.add(
                    text,
                    Marks {
                        code: true,
                        ..self.marks
                    },
                ),
                Event::Html(text) | Event::InlineHtml(text) => {
                    for text in text.split_inclusive('\n') {
                        self.add(
                            if text == "\n" {
                                " "
                            } else {
                                text.trim_end_matches('\n')
                            },
                            Marks {
                                html: true,
                                ..self.marks
                            },
                        );
                        if text.ends_with('\n') {
                            self.flush();
                        }
                    }
                }
                Event::SoftBreak => self.add(" ", self.marks),
                Event::HardBreak => self.flush(),
                Event::Rule => {
                    self.block(true);
                    self.flush();
                    self.out.items.push(Item::Rule(self.doc.line(range.start)));
                    self.gap();
                }
                _ => {}
            }
        }
        self.flush();
        self.out
    }
}

/// Separate draw surface keeps the existing raw reader's literals frozen.
#[derive(Debug, Clone, Copy)]
pub enum Shown<'a> {
    Rendered(&'a Rendered),
    Raw(&'a reader::Flow),
}

#[derive(Debug, Clone, Copy)]
pub struct View<'a> {
    pub path: &'a str,
    pub line: Option<usize>,
    pub lines: &'a [String],
    pub shown: Shown<'a>,
    pub refusal: Option<&'a str>,
    pub top: usize,
    pub links: usize,
}

#[cfg(test)]
#[path = "markdown_fix_spec.rs"]
mod fix_spec;
