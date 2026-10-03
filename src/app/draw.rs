//! The cells: the sidebar, its tabs and the chat column, into one buffer.

use ratatui_core::buffer::Buffer;

use super::fleet::{Facts, Fleet};
use super::model::Model;
use super::overview::Overview;
use crate::console::input::View;
use crate::console::lane::Lane;
use crate::digest::SessionEntry;
use crate::theme::Look;
use crate::time::Timestamp;

/// Everything one frame draws.
#[derive(Debug, Clone, Copy)]
pub struct Screen<'a> {
    pub fleet: &'a Fleet,
    pub model: &'a Model,
    pub overview: &'a Overview,
    pub selected: Option<&'a SessionEntry>,
    /// The home lead pair, main first.
    pub pair: &'a [String],
    pub agents: Option<&'a Facts>,
    pub lane: &'a Lane,
    pub composer: Composer<'a>,
    /// `None` draws no colour.
    pub look: Option<Look>,
    /// The viewer's zone, the raw `[+-]HHMM` tmux answered; read exactly as
    /// the chat reads it, so stamps stay UTC when nothing is drawn.
    pub zone: Option<&'a str>,
    pub now: Timestamp,
}

/// The composer's lines.
#[derive(Debug, Clone, Copy)]
pub enum Composer<'a> {
    /// The home session, owned: typing reaches its lead pair.
    Home {
        home: &'a str,
        speaker: &'a str,
        /// `Some` while composing.
        view: Option<&'a View>,
        draft: &'a str,
    },
    /// The home session, not owned.
    ReadOnly { why: &'a str },
    /// A foreign session is selected.
    Foreign { home: &'a str, speaker: &'a str },
    /// No home session.
    NoHome,
}

/// Draw `screen` into `buf`, over `buf.area`.
pub fn draw(_screen: &Screen<'_>, _buf: &mut Buffer) {}
