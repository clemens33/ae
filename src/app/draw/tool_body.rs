//! The Statistics tab's tool pages (#288, plan-r3 §8): the observed skill,
//! MCP and tool tables of the shown scope and filter, the state that stands
//! in for them, and the coverage notes every page carries.

use ratatui_core::style::{Modifier, Style};

use super::{LEFT, Paint, drawn_cols, fit};
use crate::app::settings::{StatsBody, StatsPage, StatsScope};
use crate::board::terminal_text;
use crate::console::wrap::wrap_in;
use crate::stats::Cell;
use crate::tool_stats::{Cols, DetailRow, Filter, Ident, Item, More, Row, State, View};

/// Below this many columns the page leaves Detail out (plan-r3 §8).
const DETAIL_MIN: u16 = 40;
/// The fewest cells an identity keeps before trailing columns are dropped.
const IDENT_MIN: u16 = 12;
/// Between two table columns.
const GAP: &str = "  ";
const NO_SUPPORTED: &str = "No ae-associated Claude or Codex conversations in this state root";
const NO_ID: &str = "This session has no session id: nothing to show";
/// What every page says of its sources, always drawn (plan-r3 §8, D10).
const NOTES: [&str; 7] = [
    "Claude: skills, tools, MCP · Codex: tools, MCP (completed only; no skills, no children) · others not read",
    "Each seat lists its newest 4 earlier conversations",
    "Claude effort: not observed",
    "Client: unknown unless proven for the whole conversation",
    "No observed use in covered sources != unused.",
    "Labels: client labels follow the recorded session meta; a hand edit of that meta relabels its conversations",
    "Resume: a client label added on resume applies to the whole conversation; the account store is proven unchanged",
];

/// The rows of a tool page of `body` at `width`.
pub(super) fn rows(body: &StatsBody, paint: Paint, width: u16) -> Vec<(String, Style)> {
    let mut page = Page {
        rows: Vec::new(),
        text: paint.fg(|p| p.text),
        dim: paint.fg(|p| p.dim),
        room: width.saturating_sub(LEFT),
    };
    page.sentence(&title(body), page.text.add_modifier(Modifier::BOLD));
    let Some(view) = &body.tools else {
        page.sentence("loading", page.dim);
        return page.rows;
    };
    match view.state {
        State::NoSessionId => page.sentence(NO_ID, page.dim),
        State::NoSupported => {
            page.sentence(NO_SUPPORTED, page.text);
            page.gaps(view);
        }
        State::Numbers => page.numbers(view, body.page, page.room >= DETAIL_MIN),
    }
    page.rows.push((String::new(), page.dim));
    for note in NOTES {
        page.sentence(note, page.dim);
    }
    page.rows
}

/// `Skills · All retained  ·  n/p: page  ·  m: This session  ·  [ ]: filter (client=work)`.
fn title(body: &StatsBody) -> String {
    let heading = match body.page {
        StatsPage::Skills => "Skills",
        StatsPage::Mcp => "MCP servers",
        StatsPage::Tools | StatsPage::Spawns => "Tools",
    };
    let (shown, other) = match body.scope {
        StatsScope::All => ("All retained", "This session"),
        StatsScope::ThisSession => ("This session", "All retained"),
    };
    let filter = match &body.filter {
        Filter::All => "all".to_owned(),
        Filter::Harness(adapter) => format!("harness={}", adapter.word()),
        Filter::Client(cell) => format!("client={}", cell.word()),
        Filter::Model(cell) => format!("model={}", cell.word()),
        Filter::Effort(cell) => format!("effort={}", cell.word()),
    };
    format!("{heading} · {shown}  ·  n/p: page  ·  m: {other}  ·  [ ]: filter ({filter})")
}

/// An identity as a table shows it.
fn ident(ident: &Ident) -> &str {
    match ident {
        Ident::Name(name) => name,
        Ident::Unresolved(_) => "unresolved",
    }
}

/// A row's item as its first column.
fn item(item: &Item) -> String {
    let label = match item {
        Item::Skill { name, .. } => ident(name).to_owned(),
        Item::Mcp { server, tool } => format!("{} / {}", ident(server), ident(tool)),
        Item::Tool(tool) => ident(tool).to_owned(),
    };
    terminal_text(&label)
}

/// A Skills row's scope column: the proven class and resource id.
fn scope(item: &Item) -> String {
    match item {
        Item::Skill {
            scope: Some((class, resource)),
            ..
        } => format!("{} {}", class.word(), resource.hex()),
        _ => "unresolved".to_owned(),
    }
}

/// The page's column heads after the identity, and one row's cells.
fn heads(page: StatsPage) -> (&'static str, &'static [&'static str]) {
    match page {
        StatsPage::Skills => (
            "Skill",
            &[
                "Scope",
                "Attempts",
                "Loaded",
                "Not loaded",
                "Unknown",
                "Unresolved",
                "Reads",
            ],
        ),
        StatsPage::Mcp => (
            "Server / tool",
            &["Attempts", "Returned", "Success", "Error", "Unresolved"],
        ),
        StatsPage::Tools | StatsPage::Spawns => {
            ("Tool", &["Attempts", "Returned", "Error", "Unresolved"])
        }
    }
}

/// The numbers of `cols`, in its page's column order.
fn numbers(cols: &Cols) -> Vec<String> {
    let all = match cols {
        Cols::Skill(cols) => vec![
            cols.attempts,
            cols.loaded,
            cols.not_loaded,
            cols.unknown,
            cols.unresolved,
            cols.reads,
        ],
        Cols::Mcp(cols) => vec![
            cols.attempts,
            cols.returned,
            cols.success,
            cols.error,
            cols.unresolved,
        ],
        Cols::Tool(cols) => vec![cols.attempts, cols.returned, cols.error, cols.unresolved],
    };
    all.into_iter().map(|count| count.to_string()).collect()
}

/// `+{r} more items, {a} attempts not shown`.
fn more(more: &More) -> String {
    format!(
        "+{} more items, {} attempts not shown",
        more.items, more.attempts
    )
}

/// An epoch-microsecond record time as its UTC spelling.
fn stamp(micros: i64) -> String {
    crate::time::Timestamp::from_epoch(micros.div_euclid(1_000_000)).to_string()
}

/// The rows so far, and how they are drawn.
struct Page {
    rows: Vec<(String, Style)>,
    text: Style,
    dim: Style,
    room: u16,
}

impl Page {
    /// `line`, whole: a row that does not fit wraps.
    fn sentence(&mut self, line: &str, style: Style) {
        let wrapped = wrap_in(&terminal_text(line), usize::from(self.room), 0, 0);
        self.rows.extend(
            wrapped
                .into_iter()
                .map(|(lead, row)| (format!("{:lead$}{row}", ""), style)),
        );
    }

    /// One line per coverage gap: a count, never a path.
    fn gaps(&mut self, view: &View) {
        for gap in &view.gaps {
            self.sentence(&gap.line(), self.dim);
        }
    }

    /// U2-U4: progress, coverage, the table and Detail.
    fn numbers(&mut self, view: &View, page: StatsPage, detail: bool) {
        let (text, dim) = (self.text, self.dim);
        if view.listed == 0 {
            self.sentence("Statistics are being built: no conversation read yet", dim);
        } else if view.partial() {
            let mut line = format!(
                "Partial: {} of {} conversations read",
                view.read, view.listed
            );
            if view.recounting > 0 {
                line.push_str(" · recounting ");
                line.push_str(&view.recounting.to_string());
                line.push_str(" changed");
            }
            self.sentence(&line, text);
        } else if view.recounting > 0 {
            self.sentence(&format!("recounting {} changed", view.recounting), text);
        }
        let range = view.range.map_or_else(
            || "no dated record".to_owned(),
            |(from, to)| format!("{} .. {}", stamp(from), stamp(to)),
        );
        self.sentence(&format!("Covers {} conversations, {range}", view.read), dim);
        self.gaps(view);
        if page == StatsPage::Tools {
            self.sentence(&format!("Orphan results: {}", view.orphans), dim);
        }
        self.rows.push((String::new(), dim));
        let (first, rest) = heads(page);
        let skills = page == StatsPage::Skills;
        let cells = |row: &Row| {
            let mut cells = Vec::new();
            if skills {
                cells.push(scope(&row.item));
            }
            cells.extend(numbers(&row.cols));
            (item(&row.item), cells)
        };
        let mut body: Vec<(String, Vec<String>)> = view.rows.iter().map(cells).collect();
        let mut total = Vec::new();
        if skills {
            total.push(String::new());
        }
        total.extend(numbers(&view.totals));
        body.push((format!("Total ({} items)", view.items), total));
        self.table(first, rest, &body);
        if let Some(left) = &view.more {
            self.sentence(&more(left), dim);
        }
        if detail && !view.detail.is_empty() {
            self.rows.push((String::new(), dim));
            self.rows
                .push(("Detail".to_owned(), text.add_modifier(Modifier::BOLD)));
            let joint: Vec<(String, Vec<String>)> = view.detail.iter().map(joint).collect();
            self.table(
                "Item",
                &["Harness", "Client", "Model", "Effort", "Attempts"],
                &joint,
            );
            if let Some(left) = &view.detail_more {
                self.sentence(&more(left), dim);
            }
        }
    }

    /// A table: the identity column clips with `…`; every other column is
    /// right-aligned, and trailing columns drop before the identity keeps
    /// fewer than [`IDENT_MIN`] cells.
    fn table(&mut self, first: &str, heads: &[&str], body: &[(String, Vec<String>)]) {
        let mut widths: Vec<u16> = heads.iter().map(|head| drawn_cols(head)).collect();
        for (_, cells) in body {
            for (width, cell) in widths.iter_mut().zip(cells) {
                *width = (*width).max(drawn_cols(&terminal_text(cell)));
            }
        }
        let spent = |widths: &[u16]| {
            widths
                .iter()
                .map(|width| width.saturating_add(2))
                .fold(0_u16, u16::saturating_add)
        };
        while widths.len() > 1 && self.room.saturating_sub(spent(&widths)) < IDENT_MIN {
            widths.pop();
        }
        let ident = self.room.saturating_sub(spent(&widths));
        let widest = body
            .iter()
            .map(|(label, _)| drawn_cols(label))
            .chain(std::iter::once(drawn_cols(first)))
            .max()
            .unwrap_or(0)
            .min(ident);
        let line = |label: &str, cells: &mut dyn Iterator<Item = String>| {
            let clipped = fit(label, "", ident);
            let pad = usize::from(widest.saturating_sub(drawn_cols(&clipped)));
            let mut out = format!("{clipped}{:pad$}", "");
            for (width, cell) in widths.iter().zip(cells) {
                let cell = terminal_text(&cell);
                let pad = usize::from(width.saturating_sub(drawn_cols(&cell)));
                out.push_str(GAP);
                out.push_str(&" ".repeat(pad));
                out.push_str(&cell);
            }
            out
        };
        let head = line(first, &mut heads.iter().map(|head| (*head).to_owned()));
        self.rows
            .push((head, self.dim.add_modifier(Modifier::BOLD)));
        for (label, cells) in body {
            let row = line(label, &mut cells.iter().cloned());
            self.rows.push((row, self.text));
        }
    }
}

/// One Detail row: the item and all four dimensions.
fn joint(row: &DetailRow) -> (String, Vec<String>) {
    let word = |cell: &Cell| cell.word().to_owned();
    (
        item(&row.item),
        vec![
            row.harness.word().to_owned(),
            word(&row.client),
            word(&row.model),
            word(&row.effort),
            row.attempts.to_string(),
        ],
    )
}

#[cfg(test)]
#[path = "tool_body_tests.rs"]
mod tests;
