//! The Statistics tab's rows: one scope of the store's last read, or the
//! state that stands in for its numbers, under a reset's line.

use ratatui_core::style::{Modifier, Style};

use super::{LEFT, Paint, drawn_cols, fit};
use crate::app::settings::{ResetLine, ScopeLine, StatsBody, StatsScope};
use crate::board::terminal_text;
use crate::console::wrap::wrap_in;
use crate::stats::{Cell, Row, Scoped, ThisSession};

/// A row's cells, as one label.
const CELL_JOIN: &str = " · ";
/// What every number on this tab counts.
const EVIDENCE: &str = "Evidence: configured launch selection, resolved at spawn";
const NONE_SET: &str = "none set is no flag, not no model.";
const DIRECT: &str = "direct has no [clients] label: a route, not an account.";
const RETENTION: &str = "Counts retained records in this state root, from a store kept up to date as workers spawn. A purge removes its archive's counts at once; a deleted session or archive drops out at the next update.";
/// A scope with no published numbers yet.
const BUILDING: &str = "Statistics are being built: no source read yet";
const ALL_TITLE: &str = "All retained  ·  m: This session  ·  r: rebuild";
const THIS_TITLE: &str = "This session  ·  m: All retained  ·  r: rebuild";

/// The rows of `body` at `width`: a reset's line first, then the scope's
/// numbers, or the state that stands in for them.
pub(super) fn rows(body: &StatsBody, paint: Paint, width: u16) -> Vec<(String, Style)> {
    let dim = paint.fg(|p| p.dim);
    let mut page = Page {
        rows: Vec::new(),
        text: paint.fg(|p| p.text),
        dim,
        room: width.saturating_sub(LEFT),
    };
    if let Some(reset) = &body.store.reset {
        page.sentence(&reset_line(reset), page.text);
    }
    let line = match body.scope {
        StatsScope::All => &body.store.all,
        StatsScope::ThisSession => &body.store.this,
    };
    let Some(view) = &body.view else {
        let state = unavailable(line).unwrap_or_else(|| "loading".to_owned());
        page.sentence(&state, dim);
        return page.rows;
    };
    let (title, shown) = match (body.scope, &view.this) {
        (StatsScope::All, _) => (ALL_TITLE, Some(&view.all)),
        (StatsScope::ThisSession, ThisSession::Ready(scoped)) => (THIS_TITLE, Some(&**scoped)),
        (StatsScope::ThisSession, ThisSession::NoUuid) => (THIS_TITLE, None),
    };
    page.sentence(title, page.text.add_modifier(Modifier::BOLD));
    if let Some(state) = unavailable(line) {
        page.sentence(&state, dim);
        return page.rows;
    }
    match shown {
        Some(scoped) => page.scoped(scoped, line),
        None => page.sentence("This session has no session id: nothing to show", dim),
    }
    page.rows
}

/// The sentence that stands in for a scope's numbers, `None` when they can
/// be drawn.
fn unavailable(line: &ScopeLine) -> Option<String> {
    match line {
        ScopeLine::Plain | ScopeLine::Partial { .. } => None,
        ScopeLine::Building => Some(BUILDING.to_owned()),
        ScopeLine::Damaged { file, reason } => {
            Some(format!("Statistics are being built: {file} {reason}"))
        }
        ScopeLine::Unreadable(why) => Some(format!("Statistics cannot be read: {why}")),
    }
}

/// Where the reset `r` asked for stands, as one line.
fn reset_line(reset: &ResetLine) -> String {
    match reset {
        ResetLine::Requested => "Statistics: resetting the store…".to_owned(),
        ResetLine::Waiting => "rebuild waits: statistics upkeep is not running".to_owned(),
        ResetLine::Rebuilt => {
            "Statistics: the store was reset and is built again from the journals.".to_owned()
        }
        ResetLine::Busy => {
            "Statistics: the store is busy; nothing was removed. Press r again.".to_owned()
        }
        ResetLine::Refused(why) => format!("Statistics: the store was not reset: {why}"),
    }
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

    /// One scope's numbers; `line` says whether the store is up to date.
    fn scoped(&mut self, scoped: &Scoped, line: &ScopeLine) {
        let (text, dim) = (self.text, self.dim);
        let total = scoped.total;
        let range = scoped
            .range
            .as_ref()
            .map(|(from, to)| format!("{from} .. {to}"));
        self.sentence(&format!("Recorded worker spawns: {total}"), text);
        self.sentence(
            &format!(
                "{}  ·  sources {} / {}",
                range.as_deref().unwrap_or("no dated record"),
                scoped.sources_read,
                scoped.sources_total
            ),
            dim,
        );
        self.sentence(EVIDENCE, dim);
        self.sentence(
            &format!(
                "Configured model: {} / {total} · effort: {} / {total}",
                scoped.model_known, scoped.effort_known
            ),
            text,
        );
        if let ScopeLine::Partial {
            pending,
            listed,
            updating,
        } = line
        {
            if *pending > 0 {
                self.sentence(
                    &format!("Partial: {pending} of {listed} sources not yet up to date"),
                    text,
                );
            }
            if *updating {
                self.sentence("Partial: an update is in progress", text);
            }
        }
        if scoped.partial() {
            let covers = range.as_deref().unwrap_or("nothing read");
            self.sentence(&format!("Partial: covers {covers}"), text);
        } else if *line == ScopeLine::Plain {
            self.sentence("Coverage: complete", dim);
        }
        for gap in &scoped.gaps {
            self.sentence(&gap.line(), dim);
        }
        if scoped.legacy > 0 {
            let legacy = scoped.legacy;
            self.sentence(&format!("Legacy spawns: {legacy}, a lower bound."), dim);
        }
        for (heading, table) in [
            ("Harness", &scoped.harness),
            ("Client", &scoped.client),
            ("Model + effort", &scoped.model_effort),
            ("Joint", &scoped.joint),
        ] {
            self.rows.push((String::new(), dim));
            self.table(heading, table);
        }
        self.rows.push((String::new(), dim));
        for note in [NONE_SET, DIRECT, RETENTION] {
            self.sentence(note, dim);
        }
    }

    /// One table: a label clips, and its count is always the last thing drawn.
    fn table(&mut self, heading: &str, table: &[Row]) {
        self.rows
            .push((heading.to_owned(), self.text.add_modifier(Modifier::BOLD)));
        let labels: Vec<String> = table.iter().map(label).collect();
        let widest = labels.iter().map(|label| drawn_cols(label)).max();
        for (row, label) in table.iter().zip(labels) {
            let count = row.count.to_string();
            let spare = u16::try_from(count.len() + 1).unwrap_or(self.room);
            let room = self.room.saturating_sub(spare);
            let clipped = fit(&label, "", room);
            let pad = widest
                .unwrap_or(0)
                .min(room)
                .saturating_sub(drawn_cols(&clipped));
            let pad = usize::from(pad);
            self.rows
                .push((format!("{clipped}{:pad$} {count}", ""), self.text));
        }
    }
}

/// A row's cells as one label, safe to draw.
fn label(row: &Row) -> String {
    let words: Vec<&str> = row.cells.iter().map(Cell::word).collect();
    terminal_text(&words.join(CELL_JOIN))
}
