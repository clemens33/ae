//! The browse reducer: which session is selected, which tab shows, how far
//! the chat scrolls. Keys in, an act out; it draws nothing and reads nothing.

use super::fleet::Fleet;
use crate::console::input;

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

/// The browse state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Model {
    selected: Option<String>,
    tab: Tab,
    /// Chat pages scrolled back from the newest turn.
    pages: usize,
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
    /// `agents_tab` says the Agents tab exists.
    pub fn key(&mut self, key: Key, fleet: &Fleet, can_compose: bool, agents_tab: bool) -> Act {
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

    /// Chat pages scrolled back from the newest turn.
    #[must_use]
    pub fn pages(&self) -> usize {
        self.pages
    }

    /// Keep the selection when the fleet changes under it; a session that
    /// left the fleet hands the selection back to home, else the first row.
    pub fn reconcile(&mut self, fleet: &Fleet) {
        if self.position(fleet).is_none() {
            *self = Self {
                tab: self.tab,
                ..Self::new(fleet)
            };
        }
    }
}
