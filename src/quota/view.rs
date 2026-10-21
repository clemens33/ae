//! The shared quota view: window facts in, aligned lines out.
//!
//! PURE by design: its only reach beyond `std` is the one prose cutter
//! (`event_text::clip_head`). Every number, label and level reaches this
//! module as an already-formatted string or a small enum that the one builder
//! (`Observation::quota_view_paced`) made from the existing single owners; nothing
//! here derives, judges or reads a clock. Both quota surfaces (the tmux dialog
//! and the app's Settings > Quota) lay the same facts out AFTER they know their
//! live geometry.

use std::fmt::Write as _;

use crate::event_text::clip_head;

/// How many cells the usage bar draws.
const BAR_CELLS: usize = 8;
/// Names elide down to this many cells and no further.
const NAME_FLOOR: usize = 12;
/// The text every admit row ends with.
const ADMIT_TAIL: &str = "— ae quota";

/// One window's level, as the one classifier judged it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Headroom,
    Low,
    Critical,
}

/// Why the judged percentage differs from the vendor's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// This many declared manual resets are in hand.
    Resets(u8),
    /// Credits are unlimited.
    Unlimited,
    /// The account's spend cap is reached.
    SpendCap,
}

/// How fast one window is filling, and how much observed span says so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Burn {
    /// A rate (or an upper bound) already spelled, over this observed span.
    Rate { rate: String, span: String },
    /// Too little history yet: this much observed, this much the rule needs.
    Short { seen: String, need: String },
    /// A rate that could not be made from history that was long enough.
    Unusable,
    /// This client keeps no history a rate could be read from.
    NoHistory,
}

/// How much prose a BURN cell keeps, most first. The numbers outlast the words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier {
    Full,
    Brief,
    Bare,
}

/// What a line is drawn in. Every tone also carries its word in the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Dim,
    Head,
    Low,
    Critical,
    Stale,
}

/// One quota window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub name: String,
    /// The vendor's own percentage.
    pub raw: Option<String>,
    /// The judged percentage, only when a derivation moved the label.
    pub counts: Option<String>,
    /// Filled bar cells of the judged percentage, `0..=8`.
    pub fill: Option<u8>,
    pub why: Option<Why>,
    pub level: Level,
    pub stale: bool,
    /// Time left to the window's reset, already spelled.
    pub resets: Option<String>,
    /// Age of the observation, already spelled.
    pub seen: Option<String>,
    /// How fast this window is filling; `None` states nothing for it.
    pub burn: Option<Burn>,
}

/// One client scope: its identity, account facts and windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub title: String,
    pub profiles: String,
    pub account: Option<String>,
    /// Set when the scope has no usable window: the words that say why. Beside windows
    /// it is a note on them, such as an agy answer kept past a refresh that had none.
    pub only: Option<String>,
    pub windows: Vec<Window>,
}

/// One drawn line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    pub tone: Tone,
    /// True only for a calm window: the one kind of line a short surface drops.
    pub calm: bool,
}

/// The room a surface has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    /// Content columns available to a line.
    pub width: usize,
    /// Lines available, or `None` for a surface that scrolls.
    pub rows: Option<usize>,
    /// Block glyphs for the bar, or ASCII.
    pub icons: bool,
}

/// The one scope the surfaces draw when the config cannot be read.
#[must_use]
pub fn unavailable() -> Vec<Scope> {
    vec![Scope {
        title: "quota: unavailable".to_owned(),
        profiles: String::new(),
        account: None,
        only: None,
        windows: Vec::new(),
    }]
}

/// Whether `line` is the row that admits dropped lines: the only dim line that
/// starts with `+` (the column header is dim but space-led, and a scope head
/// is a head), so a surface can draw it inert.
#[must_use]
pub fn is_admit(line: &Line) -> bool {
    matches!(line.tone, Tone::Dim) && line.text.starts_with('+')
}

fn is_calm(window: &Window) -> bool {
    matches!(window.level, Level::Headroom) && !window.stale && window.raw.is_some()
}

fn note(window: &Window, long: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    match window.level {
        Level::Headroom => {}
        Level::Low => parts.push("low".to_owned()),
        Level::Critical => parts.push("critical".to_owned()),
    }
    match (window.why, long) {
        (Some(Why::Resets(1)), true) => parts.push("1 reset in hand".to_owned()),
        (Some(Why::Resets(count)), true) => parts.push(format!("{count} resets in hand")),
        (Some(Why::Resets(1)), false) => parts.push("1 reset".to_owned()),
        (Some(Why::Resets(count)), false) => parts.push(format!("{count} resets")),
        (Some(Why::Unlimited), true) => parts.push("credits unlimited".to_owned()),
        (Some(Why::Unlimited), false) => parts.push("unlimited".to_owned()),
        (Some(Why::SpendCap), true) => parts.push("spend cap reached".to_owned()),
        (Some(Why::SpendCap), false) => parts.push("spend cap".to_owned()),
        (None, _) => {}
    }
    if window.stale {
        parts.push("stale".to_owned());
    }
    parts.join(" · ")
}

fn tone_of(window: &Window) -> Tone {
    match (&window.level, window.stale) {
        (Level::Critical, _) => Tone::Critical,
        (Level::Low, _) => Tone::Low,
        (Level::Headroom, true) => Tone::Stale,
        (Level::Headroom, false) => Tone::Plain,
    }
}

/// One BURN cell at `tier`. Words drop before digits, and a cell is never cut.
fn burn_cell(burn: Option<&Burn>, tier: Tier) -> String {
    match (burn, tier) {
        (None, _) => "-".to_owned(),
        (Some(Burn::Rate { rate, span }), Tier::Full) => format!("{rate} (over {span})"),
        (Some(Burn::Rate { rate, span }), Tier::Brief) => format!("{rate} ({span})"),
        (Some(Burn::Rate { rate, .. }), Tier::Bare) => rate.clone(),
        (Some(Burn::Short { seen, need }), Tier::Full) => {
            format!("insufficient data ({seen} of {need} needed)")
        }
        (Some(Burn::Short { seen, need }), Tier::Brief) => {
            format!("insufficient ({seen} of {need})")
        }
        (Some(Burn::Unusable), Tier::Full) => "insufficient data (no usable rate)".to_owned(),
        (Some(Burn::Short { .. } | Burn::Unusable), Tier::Brief | Tier::Bare) => {
            "insufficient".to_owned()
        }
        (Some(Burn::NoHistory), _) => "no burn history".to_owned(),
    }
}

fn bar(fill: Option<u8>, icons: bool) -> String {
    let Some(fill) = fill else {
        return " ".repeat(BAR_CELLS);
    };
    let on = usize::from(fill).min(BAR_CELLS);
    let (solid, empty) = if icons { ("█", "░") } else { ("#", ".") };
    format!("{}{}", solid.repeat(on), empty.repeat(BAR_CELLS - on))
}

fn head(scope: &Scope, width: usize) -> String {
    let mut text = scope.title.clone();
    if !scope.profiles.is_empty() {
        text.push_str("   ");
        text.push_str(&scope.profiles);
    }
    if let Some(account) = &scope.account {
        text.push_str("   · ");
        text.push_str(account);
    }
    clip_head(&text, width)
}

/// Word-wrap `text` into lines at most `width` cells, each indented two.
fn wrap(text: &str, width: usize) -> Vec<String> {
    const INDENT: usize = 2;
    let room = width.saturating_sub(INDENT).max(10);
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split(' ') {
        if !current.is_empty() && current.chars().count() + 1 + word.chars().count() > room {
            lines.push(format!("  {current}"));
            current.clear();
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    lines.push(format!("  {current}"));
    lines
}

/// The table geometry for one set of kept windows: which columns draw and how
/// wide each is.
struct Columns {
    name: usize,
    bar: bool,
    used: usize,
    counts: usize,
    note: usize,
    resets: usize,
    seen: Option<usize>,
    long: bool,
    /// The BURN column's tier and width; `None` when no kept window states a burn.
    burn: Option<(Tier, usize)>,
}

impl Columns {
    fn of(windows: &[&Window], width: usize) -> Self {
        let cells =
            |texts: Vec<usize>, floor: usize| texts.into_iter().max().unwrap_or(0).max(floor);
        let name = cells(windows.iter().map(|w| w.name.chars().count()).collect(), 6);
        let used = cells(
            windows
                .iter()
                .map(|w| w.raw.as_deref().map_or(1, |raw| raw.chars().count()))
                .collect(),
            4,
        );
        let counts = windows
            .iter()
            .filter_map(|w| w.counts.as_deref())
            .map(|counts| counts.chars().count() + 2)
            .max()
            .map_or(0, |widest| widest.max("COUNTS".len()));
        let resets = cells(
            windows
                .iter()
                .map(|w| w.resets.as_deref().map_or(1, |text| text.chars().count()))
                .collect(),
            6,
        );
        let seen = cells(
            windows
                .iter()
                .map(|w| w.seen.as_deref().map_or(1, |text| text.chars().count()))
                .collect(),
            4,
        );
        let mut columns = Self {
            name,
            bar: true,
            used,
            counts,
            note: 0,
            resets,
            seen: Some(seen),
            long: true,
            burn: None,
        };
        columns.note = columns.note_width(windows);
        if windows.iter().any(|w| w.burn.is_some()) {
            columns.burn_at(windows, Tier::Full);
        }
        // Drop order: the BURN prose, SEEN, the bar, long notes, the BURN span,
        // then names down to the floor.
        let excess = |columns: &Self| columns.total().saturating_sub(width);
        if excess(&columns) > 0 && columns.burn.is_some() {
            columns.burn_at(windows, Tier::Brief);
        }
        if excess(&columns) > 0 {
            columns.seen = None;
        }
        if excess(&columns) > 0 {
            columns.bar = false;
        }
        if excess(&columns) > 0 {
            columns.long = false;
            columns.note = columns.note_width(windows);
        }
        if excess(&columns) > 0 && columns.burn.is_some() {
            columns.burn_at(windows, Tier::Bare);
        }
        let over = excess(&columns);
        if over > 0 {
            columns.name = columns
                .name
                .saturating_sub(over)
                .max(NAME_FLOOR.min(columns.name));
        }
        columns
    }

    fn note_width(&self, windows: &[&Window]) -> usize {
        windows
            .iter()
            .map(|w| note(w, self.long).chars().count())
            .max()
            .unwrap_or(0)
            .max("NOTE".len())
    }

    /// Draw the BURN column at `tier`, as wide as its widest cell.
    fn burn_at(&mut self, windows: &[&Window], tier: Tier) {
        let widest = windows
            .iter()
            .map(|w| burn_cell(w.burn.as_ref(), tier).chars().count())
            .max()
            .unwrap_or(0)
            .max("BURN".len());
        self.burn = Some((tier, widest));
    }

    fn total(&self) -> usize {
        2 + self.name
            + if self.bar { 1 + BAR_CELLS } else { 0 }
            + 2
            + self.used
            + if self.counts > 0 { 2 + self.counts } else { 0 }
            + 2
            + self.note
            + 2
            + self.resets
            + self.seen.map_or(0, |seen| 2 + seen)
            + self.burn.map_or(0, |(_, burn)| 2 + burn)
    }

    fn row(&self, cells: &Cells<'_>) -> String {
        let mut line = format!("  {:<width$}", cells.name, width = self.name);
        if self.bar {
            let _ = write!(line, " {:<width$}", cells.bar, width = BAR_CELLS);
        }
        let _ = write!(line, "  {:>width$}", cells.used, width = self.used);
        if self.counts > 0 {
            let _ = write!(line, "  {:<width$}", cells.counts, width = self.counts);
        }
        let _ = write!(line, "  {:<width$}", cells.note, width = self.note);
        let _ = write!(line, "  {:>width$}", cells.resets, width = self.resets);
        if let Some(width) = self.seen {
            let _ = write!(line, "  {:>width$}", cells.seen);
        }
        if let Some((_, width)) = self.burn {
            let _ = write!(line, "  {:<width$}", cells.burn);
        }
        line.trim_end().to_owned()
    }
}

/// The text of each column of one table line.
struct Cells<'a> {
    name: &'a str,
    bar: &'a str,
    used: &'a str,
    counts: &'a str,
    note: &'a str,
    resets: &'a str,
    seen: &'a str,
    burn: &'a str,
}

/// Which windows the height rule has dropped, and the admit row's text.
fn admit(dropped_scopes: usize, dropped_windows: usize) -> Option<String> {
    let scopes = match dropped_scopes {
        0 => None,
        1 => Some("1 calm scope".to_owned()),
        count => Some(format!("{count} calm scopes")),
    };
    let windows = match dropped_windows {
        0 => None,
        1 => Some("1 calm window".to_owned()),
        count => Some(format!("{count} calm windows")),
    };
    let words = match (scopes, windows) {
        (None, None) => return None,
        (Some(scopes), None) => scopes,
        (None, Some(windows)) => windows,
        (Some(scopes), Some(windows)) => format!("{scopes}, {windows}"),
    };
    Some(format!("+{words} {ADMIT_TAIL}"))
}

fn draw(scopes: &[Scope], dropped: &[Vec<bool>], geometry: Geometry) -> Vec<Line> {
    let kept: Vec<&Window> = scopes
        .iter()
        .zip(dropped)
        .flat_map(|(scope, gone)| {
            scope
                .windows
                .iter()
                .zip(gone)
                .filter(|(_, gone)| !**gone)
                .map(|(window, _)| window)
        })
        .collect();
    let columns = Columns::of(&kept, geometry.width);
    let mut lines = vec![Line {
        text: columns.row(&Cells {
            name: "WINDOW",
            bar: "",
            used: "USED",
            counts: if columns.counts > 0 { "COUNTS" } else { "" },
            note: "NOTE",
            resets: "RESETS",
            seen: "SEEN",
            burn: "BURN",
        }),
        tone: Tone::Dim,
        calm: false,
    }];
    let (mut gone_scopes, mut gone_windows) = (0, 0);
    for (scope, gone) in scopes.iter().zip(dropped) {
        let removed = gone.iter().filter(|gone| **gone).count();
        if !scope.windows.is_empty() && removed == scope.windows.len() {
            gone_scopes += 1;
            continue;
        }
        gone_windows += removed;
        lines.push(Line {
            text: head(scope, geometry.width),
            tone: Tone::Head,
            calm: false,
        });
        if let Some(why) = &scope.only {
            for piece in wrap(why, geometry.width) {
                lines.push(Line {
                    text: piece,
                    tone: Tone::Stale,
                    calm: false,
                });
            }
        }
        for (window, gone) in scope.windows.iter().zip(gone) {
            if *gone {
                continue;
            }
            let judged_bar = bar(window.fill, geometry.icons);
            let counts = window
                .counts
                .as_deref()
                .map_or_else(String::new, |counts| format!("→ {counts}"));
            lines.push(Line {
                text: columns.row(&Cells {
                    name: &clip_head(&window.name, columns.name),
                    bar: &judged_bar,
                    used: window.raw.as_deref().unwrap_or("-"),
                    counts: &counts,
                    note: &note(window, columns.long),
                    resets: window.resets.as_deref().unwrap_or("?"),
                    seen: window.seen.as_deref().unwrap_or("?"),
                    burn: &columns.burn.map_or_else(String::new, |(tier, _)| {
                        burn_cell(window.burn.as_ref(), tier)
                    }),
                }),
                tone: tone_of(window),
                calm: is_calm(window),
            });
        }
    }
    if let Some(text) = admit(gone_scopes, gone_windows) {
        lines.push(Line {
            text,
            tone: Tone::Dim,
            calm: false,
        });
    }
    lines
}

/// Lay `scopes` out for `geometry`: a column header, then each scope's head and
/// one aligned line per window. On a short surface calm windows drop from the
/// bottom, one at a time, until the lines fit; trouble lines never drop, and an
/// admit row says what went.
#[must_use]
pub fn layout(scopes: &[Scope], geometry: Geometry) -> Vec<Line> {
    if scopes.is_empty() {
        return Vec::new();
    }
    let mut dropped: Vec<Vec<bool>> = scopes
        .iter()
        .map(|scope| vec![false; scope.windows.len()])
        .collect();
    loop {
        let lines = draw(scopes, &dropped, geometry);
        let Some(rows) = geometry.rows else {
            return lines;
        };
        if lines.len() <= rows {
            return lines;
        }
        // The next calm window, last scope first and last window first.
        let next = scopes.iter().enumerate().rev().find_map(|(at, scope)| {
            scope
                .windows
                .iter()
                .enumerate()
                .rev()
                .find(|(index, window)| is_calm(window) && !dropped[at][*index])
                .map(|(index, _)| (at, index))
        });
        match next {
            Some((at, index)) => dropped[at][index] = true,
            None => return lines,
        }
    }
}
