//! What the Agents body and the chat header need beside the roster: what waits
//! on the human, and the selected session's working dir.
//!
//! PURE. Open is the needs Section the chat shows (so the sidebar agrees with
//! `ae list`); the dir is the one the session's record names, shortened by
//! `ae brief`'s own rule.

use std::path::Path;

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
    Overview { open, dir }
}
