//! The settings overlay: a reusable full-pane panel and the footer a full-pane
//! panel keeps.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::{Modifier, Style};
use ratatui_core::text::Span;

use crate::app::model::{Model, SettingsTab};
use crate::app::peek;
use crate::app::settings::{SettingsBodies, StatsTab};
use crate::theme::Look;

use super::Ctx;
use super::geometry::MIN;
use super::layout::{Hit, Layout};
use super::paint::{LEFT, Paint, cells, put};
use super::settings_tabs::{config_rows, orchestrator_tail, settings_body};

/// Whether a full-pane panel is open: the footer row is its own, and the
/// chat's hint row draws no version or gear under it.
pub(super) fn overlay_open(model: &Model) -> bool {
    model.settings_open() || model.peek_open() || model.reader_open()
}

/// The footer a full-pane panel keeps on the pane's last row, the full width:
/// the panel's word and its one hint from the left, then `ae <version>` and the
/// gear right-aligned. The cells are reset first, so the closed frame's hint
/// row, sidebar ground and rule never show under the panel.
pub(super) fn keys_row(ctx: &Ctx<'_, '_>, buf: &mut Buffer, layout: &mut Layout) {
    let paint = ctx.paint;
    let (width, y) = (buf.area.width, buf.area.height - 1);
    for x in 0..width {
        buf[(x, y)].reset();
    }
    buf.set_style(Rect::new(0, y, width, 1), paint.ground(|p| p.ink));
    let reading = ctx.screen.model.reader_open();
    let peeking = ctx.screen.model.peek_open() || reading;
    let (word, hint) = if ctx.screen.model.settings_open() {
        ("settings", "Esc close")
    } else if reading {
        (
            "reading",
            ctx.screen.model.note().unwrap_or("Esc back · g/G ends"),
        )
    } else {
        // A copy's outcome rides here as well as on the hint row.
        ("viewing", ctx.screen.model.note().unwrap_or("Esc chat"))
    };
    let x = put(
        buf,
        LEFT,
        y,
        word,
        width - LEFT,
        paint.fg(|p| p.text).add_modifier(Modifier::BOLD),
    );
    let rest = format!("   {hint}");
    let hints_end = put(
        buf,
        x,
        y,
        &rest,
        width.saturating_sub(x),
        paint.fg(|p| p.dim),
    );
    tail(ctx, buf, layout, (y, hints_end), peeking);
}

/// `ae <version>` and the gear, right-aligned on row `y`, each only when it
/// fits after `hints_end` and a gap: narrowing drops the version first, then
/// the gear. The peek panel hides the gear, because a click on it would be dead.
pub(super) fn tail(
    ctx: &Ctx<'_, '_>,
    buf: &mut Buffer,
    layout: &mut Layout,
    (y, hints_end): (u16, u16),
    peeking: bool,
) {
    let width = buf.area.width;
    let gear = if ctx.icons { "\u{2699}" } else { "*" };
    let tail = if peeking {
        crate::shape::version_line()
    } else {
        format!("{} {gear}", crate::shape::version_line())
    };
    let tail_cells = cells(Span::raw(tail.as_str()).width());
    let gear_cells = cells(Span::raw(gear).width());
    let room = |need: u16| usize::from(hints_end) + 2 + usize::from(need) <= usize::from(width);
    let style = ctx.paint.fg(|p| p.dim);
    // The click covers the gear and the blank before it, like the status
    // line's settings range; `room` keeps that blank free.
    let target = Rect::new(width.saturating_sub(gear_cells + 1), y, gear_cells + 1, 1);
    if room(tail_cells) {
        put(buf, width - tail_cells, y, &tail, tail_cells, style);
        if !peeking {
            layout.record(buf, target, Hit::Settings);
        }
    } else if !peeking && room(gear_cells) {
        put(buf, width - gear_cells, y, gear, gear_cells, style);
        layout.record(buf, target, Hit::Settings);
    }
}

/// The overlay's title row.
const SETTINGS_TITLE: &str = "Settings";

/// The tabs in draw order.
const SETTINGS_TABS: [&str; 7] = [
    "Quota",
    "Config",
    "About",
    "Instructions",
    "Keys",
    "Clients",
    "Statistics",
];

/// All seven targets stay reachable on the smallest pane.
const SETTINGS_SHORT_TABS: [&str; 7] = ["Quota", "Conf", "About", "Instr", "Keys", "Cli", "Stats"];

/// The Statistics tab's inner row, in one form at every width: it is short
/// enough to stay whole on the smallest pane.
const STATS_TABS: [&str; 3] = [
    StatsTab::Agents.title(),
    StatsTab::Skills.title(),
    StatsTab::Tools.title(),
];

/// Between two inner titles.
const STATS_GAP: &str = "  ";

/// The clickable close label, right-aligned on the title row.
const SETTINGS_CLOSE: &str = "Esc close";

/// Paint the open overlay, then record its scroll bounds. The caller
/// stripped the covered layout first.
pub(crate) fn paint_settings(
    tab: SettingsTab,
    scroll: usize,
    bodies: &SettingsBodies,
    look: Option<&Look>,
    buf: &mut Buffer,
    layout: &mut Layout,
) {
    let paint = Paint::of(look);
    let width = buf.area.width;
    layout.settings_keep = (layout.settings_reveal && tab == SettingsTab::Config)
        .then(|| {
            let above = config_rows(bodies, paint).len();
            orchestrator_tail(bodies, paint, width, above).picker
        })
        .flatten();
    let selected = match tab {
        SettingsTab::Quota => 0,
        SettingsTab::Config => 1,
        SettingsTab::About => 2,
        SettingsTab::Instructions => 3,
        SettingsTab::Keys => 4,
        SettingsTab::Accounts => 5,
        SettingsTab::Statistics => 6,
    };
    let titles: usize = SETTINGS_TABS
        .iter()
        .map(|title| Span::raw(*title).width())
        .sum();
    let tabs = if usize::from(LEFT) + titles + SETTINGS_TABS.len() - 1 <= usize::from(width) {
        &SETTINGS_TABS
    } else {
        &SETTINGS_SHORT_TABS
    };
    panel(
        buf,
        layout,
        paint,
        &Panel {
            title: SETTINGS_TITLE,
            note: "",
            tabs,
            hits: &[
                Hit::SettingsTab(SettingsTab::Quota),
                Hit::SettingsTab(SettingsTab::Config),
                Hit::SettingsTab(SettingsTab::About),
                Hit::SettingsTab(SettingsTab::Instructions),
                Hit::SettingsTab(SettingsTab::Keys),
                Hit::SettingsTab(SettingsTab::Accounts),
                Hit::SettingsTab(SettingsTab::Statistics),
            ],
            inner: (tab == SettingsTab::Statistics).then_some(Inner {
                tabs: &STATS_TABS,
                hits: &[
                    Hit::StatsTab(StatsTab::Agents),
                    Hit::StatsTab(StatsTab::Skills),
                    Hit::StatsTab(StatsTab::Tools),
                ],
                selected: StatsTab::ALL
                    .iter()
                    .position(|inner| *inner == StatsTab::of(bodies.stats.page))
                    .unwrap_or(0),
            }),
            close: (SETTINGS_CLOSE, Hit::SettingsClose),
            selected,
            body: &settings_body(
                tab,
                bodies,
                paint,
                width,
                look.is_none_or(|look| look.icons),
            ),
            scroll: Scroll::Top(scroll),
        },
    );
}

/// What one full-pane panel draws: its title, tab row and scrolled body.
pub(super) struct Panel<'a> {
    pub(super) title: &'a str,
    /// A dim line on the tab row, after any tabs: the peek panel's status.
    pub(super) note: &'a str,
    pub(super) tabs: &'a [&'a str],
    /// The click target of each tab title, in tab order.
    pub(super) hits: &'a [Hit],
    /// A second row of tabs directly under the tab row, which pushes the
    /// rule and the body one row down; none for most panels.
    pub(super) inner: Option<Inner<'a>>,
    /// The label drawn right-aligned on the title row, and its target.
    pub(super) close: (&'a str, Hit),
    pub(super) selected: usize,
    pub(super) body: &'a [(String, Style)],
    pub(super) scroll: Scroll,
}

/// A panel's inner tab row: one fixed form, a gap of [`STATS_GAP`] between
/// titles.
pub(super) struct Inner<'a> {
    tabs: &'a [&'a str],
    /// The click target of each title, in order.
    hits: &'a [Hit],
    selected: usize,
}

/// How far a panel's body is scrolled.
pub(super) enum Scroll {
    /// Rows down from the first.
    Top(usize),
    /// Rows back from the newest, which the body ends on.
    Back(usize),
}

/// One full-pane panel over rows `0..height-1`: title, tab row, rule and a
/// scrolled body, then its scroll bounds. Settings and the peek panel are the
/// callers; the later `?` help reuses the seam. Below [`MIN`] the closed frame's fallback
/// stands and nothing is painted or recorded.
pub(super) fn panel(buf: &mut Buffer, layout: &mut Layout, paint: Paint, panel: &Panel<'_>) {
    let area = buf.area;
    if area.width < MIN.0 || area.height < MIN.1 {
        return;
    }
    let (width, height) = (area.width, area.height);
    // Covered cells are reset whole, not just re-grounded: `set_style` keeps
    // the closed frame's symbols and modifiers, which a sparse body would
    // leave visible. The keys row stands.
    for y in 0..height - 1 {
        for x in 0..width {
            buf[(x, y)].reset();
        }
    }
    buf.set_style(Rect::new(0, 0, width, height - 1), paint.ground(|p| p.base));
    let heading = paint.fg(|p| p.title).add_modifier(Modifier::BOLD);
    let (label, close) = &panel.close;
    let label_x = width.saturating_sub(LEFT + cells(Span::raw(*label).width()));
    // The title is cut a cell short of the close label, which keeps its cells.
    let title_room = label_x.saturating_sub(LEFT + 1);
    let title_end = put(buf, LEFT, 0, panel.title, title_room, heading);
    if label_x > title_end {
        let end = put(buf, label_x, 0, label, width - label_x, paint.fg(|p| p.dim));
        layout.record(buf, Rect::new(label_x, 0, end - label_x, 1), close.clone());
    }
    // Three cells between titles when all of them fit that way, else one.
    let titles: usize = panel.tabs.iter().map(|name| Span::raw(*name).width()).sum();
    let wide = usize::from(LEFT) + titles + 3 * panel.tabs.len().saturating_sub(1);
    let gap = if wide <= usize::from(width) {
        "   "
    } else {
        " "
    };
    let mut x = LEFT;
    for (index, name) in panel.tabs.iter().enumerate() {
        let style = if index == panel.selected {
            paint.selected()
        } else {
            paint.fg(|p| p.text)
        };
        let end = put(buf, x, 1, name, width.saturating_sub(x), style);
        if let Some(hit) = panel.hits.get(index) {
            layout.record(buf, Rect::new(x, 1, end - x, 1), hit.clone());
        }
        x = put(buf, end, 1, gap, width.saturating_sub(end), style);
    }
    put(buf, LEFT, 1, panel.note, width - LEFT, paint.fg(|p| p.dim));
    // The inner row stands on the row under the tabs; the rule and the body
    // move down with it.
    let rule_y: u16 = if let Some(inner) = &panel.inner {
        let mut x = LEFT;
        for (index, name) in inner.tabs.iter().enumerate() {
            let style = if index == inner.selected {
                paint.selected()
            } else {
                paint.fg(|p| p.text)
            };
            let end = put(buf, x, 2, name, width.saturating_sub(x), style);
            if let Some(hit) = inner.hits.get(index) {
                layout.record(buf, Rect::new(x, 2, end - x, 1), hit.clone());
            }
            x = put(
                buf,
                end,
                2,
                STATS_GAP,
                width.saturating_sub(end),
                paint.fg(|p| p.text),
            );
        }
        3
    } else {
        2
    };
    let rule: String = "─".repeat(usize::from(width));
    put(buf, 0, rule_y, &rule, width, paint.fg(|p| p.border));
    let visible = usize::from(height.saturating_sub(rule_y + 2));
    // The draw offset clamps to what is there before a line is chosen: past
    // the end paints an emptied body, and the model's clamp after this frame
    // converges the state to the same bound.
    let (start, max) = match panel.scroll {
        Scroll::Top(offset) => {
            let max = panel.body.len().saturating_sub(visible);
            let mut start = offset.min(max);
            // The highlighted row stays on screen, moving the window the least
            // it takes. The picker's heading stays with it while both fit.
            if let Some((top, row)) = layout.settings_keep {
                let lo = if row + 1 - top <= visible { top } else { row };
                if lo < start {
                    start = lo;
                } else if row >= start + visible {
                    start = (row + 1).saturating_sub(visible);
                }
                start = start.min(max);
            }
            layout.settings_start = start;
            (start, max)
        }
        Scroll::Back(back) => peek::window(panel.body.len(), visible, back),
    };
    for (at, (text, style)) in panel.body.iter().skip(start).take(visible).enumerate() {
        put(
            buf,
            LEFT,
            rule_y + 1 + cells(at),
            text,
            width.saturating_sub(LEFT),
            *style,
        );
    }
    layout.settings_page_rows = visible.max(1);
    layout.settings_max_scroll = max;
}
