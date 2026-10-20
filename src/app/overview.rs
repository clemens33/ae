//! What the Agents body and the chat header need beside the roster: what waits
//! on the human, and the selected session's working dir.
//!
//! PURE. Open is the needs Section the chat shows (so the sidebar agrees with
//! `ae list`); the dir is the one the session's record names, shortened by
//! `ae brief`'s own rule.

use std::path::Path;

use crate::console::link::Base;
use crate::console::needs::{Section, Verdict};
use crate::digest::SessionEntry;
use crate::time::Timestamp;

/// The facts of the selected session.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Overview {
    pub open: Vec<Open>,
    /// The recorded working dir, `~`-relative under the viewer's home; `None`
    /// when the record names none, so nothing is guessed.
    pub dir: Option<String>,
    /// Where a path in the lane resolves: the record's working dir as it is
    /// written, and the viewer's home. Nothing is guessed: no recorded dir, no
    /// base dir.
    pub base: Base,
    /// The session's unanswered requests — request debt, never something that
    /// waits on the human; `None` when not known exactly.
    pub unanswered: Option<usize>,
    /// How long the oldest of them has waited, in seconds.
    pub unanswered_oldest_secs: Option<i64>,
}

/// One thing waiting on the human.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Open {
    pub seat: String,
    pub text: String,
    pub age_secs: Option<i64>,
}

/// The facts of `entry`, every age judged at `now`.
#[must_use]
pub fn of(
    entry: &SessionEntry,
    needs: Option<&Section>,
    home: Option<&Path>,
    now: Timestamp,
) -> Overview {
    let open = needs
        .map(|section| section.rows.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|row| match row.verdict {
            Verdict::Reason(reason) => Some(Open {
                seat: row.seat.name.clone(),
                text: super::fleet::asked(&row.detail, reason),
                age_secs: super::fleet::micros_age(row.since_micros, now),
            }),
            Verdict::Unknown(_) => None,
        })
        .collect();
    let dir = (entry.work_dir.as_deref())
        .filter(|dir| !dir.is_empty())
        .map(|dir| crate::board::terminal_text(&crate::brief::short_path(dir, home)));
    let base = Base {
        dir: (entry.work_dir.clone()).filter(|dir| !dir.is_empty()),
        home: home.and_then(Path::to_str).map(str::to_owned),
    };
    Overview {
        open,
        dir,
        base,
        unanswered: entry.unanswered,
        unanswered_oldest_secs: entry
            .unanswered_oldest_epoch
            .map(|sent| now.epoch().saturating_sub(sent).max(0)),
    }
}
