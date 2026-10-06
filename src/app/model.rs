//! The browse reducer: which session is selected, which tab shows, how far
//! the chat scrolls. Keys in, an act out; it draws nothing and reads nothing.

use super::fleet::Fleet;
use crate::console::input;

/// The chat rows one wheel notch scrolls.
pub(crate) const WHEEL_ROWS: usize = 3;

/// A browse key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Digit(u8),
    Up,
    Down,
    NextNeed,
    Tab,
    Compose,
    Esc,
    PageUp,
    PageDown,
    Quit,
}

/// The browse keys one decoded key spells. A paste is swallowed whole: text
/// a human pasted while browsing is never read as a run of commands.
#[must_use]
pub fn browse_keys(key: &input::Key) -> Vec<Key> {
    let one = match key {
        input::Key::Text(bytes) => return bytes.iter().filter_map(|byte| letter(*byte)).collect(),
        input::Key::Enter => Key::Compose,
        input::Key::Up => Key::Up,
        input::Key::Down => Key::Down,
        input::Key::PageUp => Key::PageUp,
        input::Key::PageDown => Key::PageDown,
        input::Key::Tab => Key::Tab,
        input::Key::Escape => Key::Esc,
        input::Key::Interrupt => Key::Quit,
        _ => return Vec::new(),
    };
    vec![one]
}

/// The browse key one typed byte spells.
fn letter(byte: u8) -> Option<Key> {
    match byte {
        b'1'..=b'9' => Some(Key::Digit(byte - b'0')),
        b'!' => Some(Key::NextNeed),
        b'j' => Some(Key::Down),
        b'k' => Some(Key::Up),
        b'i' => Some(Key::Compose),
        b'q' => Some(Key::Quit),
        _ => None,
    }
}

/// What a key did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    None,
    Redraw,
    Select(String),
    Compose,
    Quit,
}

/// The selected session's tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Overview,
    Agents,
}

/// A border the mouse can drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Edge {
    /// The vertical rule between the sidebar and the chat.
    Sidebar,
    /// The horizontal rule under the tab row.
    List,
}

/// The sizes the borders were dragged to, kept for the app's run and clamped
/// by every frame; `None` keeps the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Split {
    /// The sidebar's width, which is where its rule stands.
    pub(crate) sidebar: Option<u16>,
    /// The session list's rows, its more row included.
    pub(crate) list: Option<u16>,
}

/// A drag in progress: the edge, the pointer's cell along the edge's axis at
/// the press, and where the edge stood then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Drag {
    pub(crate) edge: Edge,
    pub(crate) from: u16,
    pub(crate) at: u16,
}

/// One tab of the read-only settings overlay, in draw order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SettingsTab {
    #[default]
    Quota,
    Config,
    About,
}

/// The open settings overlay: its tab, how far its body scrolled, the body
/// rows the last frame drew (what one page key moves), and the generation
/// that pins its loader answers. Read data lives in the App, not here: this
/// is browse state only, like the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SettingsOverlay {
    pub(crate) tab: SettingsTab,
    pub(crate) scroll: usize,
    pub(crate) page_rows: usize,
    pub(crate) generation: u64,
}

/// The browse state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Model {
    selected: Option<String>,
    tab: Tab,
    /// Keyboard pages scrolled back from the newest turn.
    pages: usize,
    /// Wheel rows between pages, normalized after each frame.
    rows: usize,
    split: Split,
    drag: Option<Drag>,
    settings: Option<SettingsOverlay>,
}

impl Model {
    /// The state a fresh app starts in: the home row, else the first row.
    #[must_use]
    pub fn new(fleet: &Fleet) -> Self {
        let selected = fleet
            .home
            .clone()
            .or_else(|| fleet.rows.first().map(|row| row.name.clone()));
        Self {
            selected,
            ..Self::default()
        }
    }

    /// Take one key. `can_compose` says the app owns the home input;
    /// `agents_tab` says the Agents tab exists. An open overlay owns every
    /// key first; closed behavior is exactly what it always was.
    pub fn key(&mut self, key: Key, fleet: &Fleet, can_compose: bool, agents_tab: bool) -> Act {
        if self.settings.is_some() {
            return self.settings_key(key);
        }
        let at = self.position(fleet);
        match key {
            Key::Digit(index) => {
                let target = fleet
                    .rows
                    .iter()
                    .position(|row| row.index == usize::from(index));
                self.select(fleet, target)
            }
            Key::Up => self.select(fleet, at.and_then(|at| at.checked_sub(1))),
            Key::Down => self.select(fleet, at.map(|at| at + 1)),
            Key::NextNeed => {
                let after = at.map_or(0, |at| at + 1);
                let count = fleet.rows.len();
                let target = (0..count)
                    .map(|step| (after + step) % count)
                    .find(|candidate| fleet.rows[*candidate].needy);
                self.select(fleet, target)
            }
            Key::Tab if agents_tab => {
                self.tab = match self.tab {
                    Tab::Overview => Tab::Agents,
                    Tab::Agents => Tab::Overview,
                };
                Act::Redraw
            }
            Key::Compose if can_compose && self.on_home(fleet) => Act::Compose,
            Key::Esc => {
                let home = fleet
                    .home
                    .as_deref()
                    .and_then(|home| fleet.rows.iter().position(|row| row.name == home));
                self.select(fleet, home)
            }
            Key::PageUp => {
                self.pages = self.pages.saturating_add(1);
                Act::Redraw
            }
            Key::PageDown => {
                if self.pages == 0 {
                    self.rows = 0;
                }
                self.pages = self.pages.saturating_sub(1);
                Act::Redraw
            }
            Key::Quit => Act::Quit,
            Key::Tab | Key::Compose => Act::None,
        }
    }

    /// Select row `target`; selecting nothing, or what is already selected,
    /// answers nothing. A new selection reads its chat from the newest turn.
    fn select(&mut self, fleet: &Fleet, target: Option<usize>) -> Act {
        let Some(row) = target.and_then(|target| fleet.rows.get(target)) else {
            return Act::None;
        };
        if self.selected.as_deref() == Some(row.name.as_str()) {
            return Act::None;
        }
        self.selected = Some(row.name.clone());
        self.pages = 0;
        self.rows = 0;
        Act::Select(row.name.clone())
    }

    /// Where the selection sits in `fleet`.
    #[must_use]
    pub fn position(&self, fleet: &Fleet) -> Option<usize> {
        let selected = self.selected.as_deref()?;
        fleet.rows.iter().position(|row| row.name == selected)
    }

    /// Whether the home session is selected.
    fn on_home(&self, fleet: &Fleet) -> bool {
        fleet.home.is_some() && self.selected == fleet.home
    }

    /// Bound the chat scroll to what the lane holds: `max` pages back.
    pub fn clamp_pages(&mut self, max: usize) {
        if self.pages >= max {
            self.rows = 0;
        }
        self.pages = self.pages.min(max);
    }

    /// The selected session.
    #[must_use]
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// The tab showing.
    #[must_use]
    pub fn tab(&self) -> Tab {
        self.tab
    }

    /// Ceiling page count, including a partial wheel page. After a frame's
    /// clamp, the residual is smaller than that frame's page size.
    #[must_use]
    pub fn pages(&self) -> usize {
        self.pages.saturating_add(usize::from(self.rows > 0))
    }

    /// Whether one more notch, `up` or down, would land past `max`.
    pub(crate) fn wheel_passes(&self, up: bool, page_rows: usize, max: usize) -> bool {
        let scroll = self.scroll_rows(page_rows);
        let scroll = if up {
            scroll.saturating_add(WHEEL_ROWS)
        } else {
            scroll.saturating_sub(WHEEL_ROWS)
        };
        scroll > max
    }

    /// Total chat rows back, with keyboard pages sized by this frame.
    pub(crate) fn scroll_rows(&self, page_rows: usize) -> usize {
        self.pages
            .saturating_mul(page_rows)
            .saturating_add(self.rows)
    }

    /// Bound and normalize the scroll using the rows the frame actually drew.
    pub(crate) fn clamp_scroll(&mut self, max: usize, page_rows: usize) {
        let scroll = self.scroll_rows(page_rows).min(max);
        self.pages = scroll / page_rows.max(1);
        self.rows = scroll % page_rows.max(1);
    }

    /// One wheel notch, three rows, independent of the keyboard page size.
    pub(crate) fn wheel(&mut self, up: bool, page_rows: usize, max: usize) {
        let scroll = self.scroll_rows(page_rows);
        let scroll = if up {
            scroll.saturating_add(WHEEL_ROWS)
        } else {
            scroll.saturating_sub(WHEEL_ROWS)
        };
        self.pages = scroll.min(max) / page_rows.max(1);
        self.rows = scroll.min(max) % page_rows.max(1);
    }

    /// A target that departed since the frame was drawn is a full no-op.
    pub(crate) fn select_name(&mut self, fleet: &Fleet, name: &str) -> Option<Act> {
        let row = fleet.rows.iter().position(|row| row.name == name)?;
        Some(self.select(fleet, Some(row)))
    }

    pub(crate) fn show_tab(&mut self, tab: Tab) -> Act {
        if self.tab == tab {
            return Act::None;
        }
        self.tab = tab;
        Act::Redraw
    }

    /// Keep the selection when the fleet changes under it; a session that
    /// left the fleet hands the selection back to home, else the first row.
    pub fn reconcile(&mut self, fleet: &Fleet) {
        if self.position(fleet).is_none() {
            *self = Self {
                tab: self.tab,
                split: self.split,
                drag: self.drag,
                settings: self.settings,
                ..Self::new(fleet)
            };
        }
    }

    /// Start again on `fleet`, as [`Model::new`] does, keeping the sizes the
    /// borders were dragged to, a drag in progress and an open overlay.
    pub(crate) fn restart(&mut self, fleet: &Fleet) {
        *self = Self {
            split: self.split,
            drag: self.drag,
            settings: self.settings,
            ..Self::new(fleet)
        };
    }

    /// The sizes the borders were dragged to.
    pub(crate) fn split(&self) -> Split {
        self.split
    }

    /// The drag in progress.
    pub(crate) fn drag(&self) -> Option<Drag> {
        self.drag
    }

    /// Start dragging an edge.
    pub(crate) fn grab(&mut self, drag: Drag) {
        self.drag = Some(drag);
    }

    /// Set the dragged edge's size; whether it changed.
    pub(crate) fn drag_to(&mut self, size: u16) -> bool {
        let Some(drag) = self.drag else {
            return false;
        };
        let kept = match drag.edge {
            Edge::Sidebar => &mut self.split.sidebar,
            Edge::List => &mut self.split.list,
        };
        kept.replace(size) != Some(size)
    }

    /// End a drag; whether one was in progress.
    pub(crate) fn let_go(&mut self) -> bool {
        self.drag.take().is_some()
    }

    /// One key with the overlay open: tabs, scroll and close. `Key::Quit`
    /// here is the `q` that decoded to it — the App quits on a raw
    /// Interrupt before decoding, so it never reaches this branch.
    fn settings_key(&mut self, key: Key) -> Act {
        match key {
            Key::Tab => {
                let tab = self.settings.map_or(SettingsTab::Quota, |open| open.tab);
                self.show_settings_tab(match tab {
                    SettingsTab::Quota => SettingsTab::Config,
                    SettingsTab::Config => SettingsTab::About,
                    SettingsTab::About => SettingsTab::Quota,
                })
            }
            Key::Digit(1) => self.show_settings_tab(SettingsTab::Quota),
            Key::Digit(2) => self.show_settings_tab(SettingsTab::Config),
            Key::Digit(3) => self.show_settings_tab(SettingsTab::About),
            Key::Up => self.settings_scroll_by(-1),
            Key::Down => self.settings_scroll_by(1),
            Key::PageUp => self.settings_page_by(false),
            Key::PageDown => self.settings_page_by(true),
            Key::Esc | Key::Quit => {
                self.settings = None;
                Act::Redraw
            }
            Key::Digit(_) | Key::NextNeed | Key::Compose => Act::None,
        }
    }

    /// Open the overlay on the Quota tab, pinned to `generation`.
    pub(crate) fn open_settings(&mut self, generation: u64) {
        self.settings = Some(SettingsOverlay {
            tab: SettingsTab::Quota,
            scroll: 0,
            page_rows: 0,
            generation,
        });
    }

    /// Close the overlay.
    pub(crate) fn close_settings(&mut self) {
        self.settings = None;
    }

    /// Whether the overlay is open.
    pub(crate) fn settings_open(&self) -> bool {
        self.settings.is_some()
    }

    /// The open overlay, if any.
    pub(crate) fn settings(&self) -> Option<SettingsOverlay> {
        self.settings
    }

    /// Whether the overlay is open on `generation`.
    pub(crate) fn settings_generation(&self, generation: u64) -> bool {
        self.settings
            .is_some_and(|open| open.generation == generation)
    }

    /// Show `tab`, scrolled back to its first row; whether anything changed.
    fn show_settings_tab(&mut self, tab: SettingsTab) -> Act {
        let Some(open) = self.settings.as_mut() else {
            return Act::None;
        };
        if open.tab == tab && open.scroll == 0 {
            return Act::None;
        }
        open.tab = tab;
        open.scroll = 0;
        Act::Redraw
    }

    /// Scroll the open body by `rows`, down positive; always a redraw so a
    /// wheel notch past the end still settles the scroll it asked for.
    fn settings_scroll_by(&mut self, rows: isize) -> Act {
        let Some(open) = self.settings.as_mut() else {
            return Act::None;
        };
        open.scroll = open.scroll.saturating_add_signed(rows);
        Act::Redraw
    }

    /// Scroll the open body one drawn page, down when `down`: the frame's
    /// own body height, never a fixed count.
    fn settings_page_by(&mut self, down: bool) -> Act {
        let page = self.settings.map_or(1, |open| open.page_rows.max(1));
        let rows = page.cast_signed();
        self.settings_scroll_by(if down { rows } else { -rows })
    }

    /// The drawn body height one page key moves; the App sets it per frame.
    pub(crate) fn set_settings_page_rows(&mut self, page_rows: usize) {
        if let Some(open) = self.settings.as_mut() {
            open.page_rows = page_rows;
        }
    }

    /// Scroll the open body by `rows`, down positive, for the App's wheel.
    pub(crate) fn settings_wheel(&mut self, rows: isize) {
        let _ = self.settings_scroll_by(rows);
    }

    /// Bound the overlay scroll to the `max` rows the frame drew.
    pub(crate) fn clamp_settings_scroll(&mut self, max: usize) {
        if let Some(open) = self.settings.as_mut() {
            open.scroll = open.scroll.min(max);
        }
    }
}

#[cfg(test)]
mod tests {
    //! Mutation pins (pins-plan.md #101/#102). Oracles: docs/app.md (the
    //! selected session sits beside the sidebar; the tab stays as you move
    //! between sessions) and the start rule (home first, else the first row).

    use super::{Drag, Edge, Key, Model, SettingsTab, Tab};
    use crate::app::fleet::{Counts, Fleet, Line2, Row};
    use crate::theme::Mark;

    fn fleet(names: &[&str]) -> Fleet {
        let row = |(at, name): (usize, &&str)| Row {
            name: (*name).to_owned(),
            index: at + 1,
            mark: Mark::Working,
            needy: false,
            counts: Counts::Unknown,
            line2: Line2::NoGoal,
            home: *name == "api",
        };
        Fleet {
            rows: names.iter().enumerate().map(row).collect(),
            home: Some("api".to_owned()),
        }
    }

    #[test]
    fn a_selection_that_leaves_the_fleet_goes_home_on_the_same_tab() {
        let both = fleet(&["api", "web"]);
        let mut model = Model::new(&both);
        let _ = model.key(Key::Digit(2), &both, false, true);
        let _ = model.key(Key::Tab, &both, false, true);
        assert_eq!((model.selected(), model.tab()), (Some("web"), Tab::Agents));
        model.reconcile(&fleet(&["api"]));
        assert_eq!(
            model.selected(),
            Some("api"),
            "the selection is a sidebar row"
        );
        assert_eq!(model.tab(), Tab::Agents, "the tab stays");
    }

    /// The public page bound includes wheel rows between keyboard pages.
    #[test]
    fn a_zero_page_bound_also_clears_wheel_rows() {
        let mut model = Model::new(&fleet(&["api"]));
        model.wheel(true, 30, 100);
        assert_eq!(model.pages(), 1);
        model.clamp_pages(0);
        assert_eq!(model.pages(), 0);
        assert_eq!(model.scroll_rows(30), 0);
    }

    /// The chosen sizes last for the run: a selection leaving the fleet keeps
    /// them, and a drag says when it moved and when it ended.
    #[test]
    fn a_chosen_size_outlives_a_selection_that_leaves_the_fleet() {
        let both = fleet(&["api", "web"]);
        let mut model = Model::new(&both);
        assert!(!model.drag_to(50), "no drag, no size");
        assert!(!model.let_go(), "no drag to end");
        model.grab(Drag {
            edge: Edge::Sidebar,
            from: 44,
            at: 44,
        });
        assert!(model.drag_to(50));
        assert!(!model.drag_to(50), "the same size is no change");
        let _ = model.key(Key::Digit(2), &both, false, true);
        model.reconcile(&fleet(&["api"]));
        assert_eq!(model.split().sidebar, Some(50));
        assert!(model.let_go(), "the drag outlived the fleet change");
        assert_eq!(model.drag(), None);
        assert_eq!(model.split().list, None, "the other edge untouched");
    }

    /// Reconciling away the selected session keeps an open overlay whole:
    /// the selection goes home, every overlay field stays.
    #[test]
    fn reconcile_keeps_the_open_overlay_whole() {
        let both = fleet(&["api", "web"]);
        let mut model = Model::new(&both);
        let _ = model.key(Key::Digit(2), &both, false, true);
        assert_eq!(model.selected(), Some("web"));
        model.open_settings(42);
        let open = model.settings.as_mut().expect("open");
        (open.tab, open.scroll, open.page_rows) = (SettingsTab::Config, 7, 5);
        model.reconcile(&fleet(&["api"]));
        assert_eq!(model.selected(), Some("api"));
        let open = model.settings().expect("overlay kept");
        assert_eq!(
            (open.tab, open.scroll, open.page_rows, open.generation),
            (SettingsTab::Config, 7, 5, 42)
        );
    }

    /// Restarting onto a populated fleet keeps an open overlay whole.
    #[test]
    fn restart_keeps_the_open_overlay_whole() {
        let mut model = Model::new(&Fleet {
            rows: Vec::new(),
            home: None,
        });
        assert_eq!(model.selected(), None);
        model.open_settings(42);
        let open = model.settings.as_mut().expect("open");
        (open.tab, open.scroll, open.page_rows) = (SettingsTab::About, 3, 9);
        model.restart(&fleet(&["api"]));
        assert_eq!(model.selected(), Some("api"));
        let open = model.settings().expect("overlay kept");
        assert_eq!(
            (open.tab, open.scroll, open.page_rows, open.generation),
            (SettingsTab::About, 3, 9, 42)
        );
    }
}
