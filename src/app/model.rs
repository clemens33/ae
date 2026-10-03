//! The browse reducer: which session is selected, which tab shows, how far
//! the lists scroll. Keys in, an act out; it draws nothing and reads nothing.

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

/// The browse keys one decoded key spells.
#[must_use]
pub fn browse_keys(_key: &input::Key) -> Vec<Key> {
    Vec::new()
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
}

impl Model {
    /// The state a fresh app starts in.
    #[must_use]
    pub fn new(_fleet: &Fleet) -> Self {
        Self::default()
    }

    /// Take one key.
    pub fn key(&mut self, _key: Key, _fleet: &Fleet, _can_compose: bool, _agents_tab: bool) -> Act {
        Act::None
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
}
