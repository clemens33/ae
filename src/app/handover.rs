//! The view a restarting `ae app` hands to the app that replaces it.
//!
//! A respawned chat window's old app publishes one session-scoped tmux option,
//! the new app in the same pane reads it once and unsets it, valid or not; its
//! `pane` is that pane. An app that execs passes the same line as the internal
//! argument [`FLAG`]; its `pane` is `%<pid>`, the process exec keeps. Any
//! client on the server can set the option and anyone can type the argument,
//! so it is hostile text: one canonical line, fixed field order, every field
//! bounded. Pure: no clock, no tmux, no app.
//!
//! `v1|pane|version|at|session|uuid|tab|focus|sidebar|list|pages|rows|list_top|body_top`
//!
//! Empty means `None`. Integers are canonical decimal. [`parse`] accepts only
//! text that [`Handover::render`] could have written, except that the
//! `overview` view token an older core wrote reads as `agents`, the one view.

use crate::archive::is_canonical_uuid;
use crate::config::is_agent_name;
use crate::session_launch::name::is_session_name;

/// The session option the handover rides.
pub const OPTION: &str = "@ae_app_view";
/// The internal `ae app` argument an exec restart hands the handover on by,
/// never shown in the usage text. An older core is never handed it, so every
/// later core must keep accepting it.
pub const FLAG: &str = "--app-view";
/// How old a handover may be, inclusive, whether read or still waiting.
pub const MAX_AGE_SECS: i64 = 60;
/// The whole text, in bytes.
pub const CAP: usize = 512;

const PAGES_MAX: usize = 1_000_000;
const ROWS_MAX: usize = 65_535;
const AT_MAX: u64 = 99_999_999_999;
const PANE_MAX: u64 = 9_999_999_999;
const VERSION_MAX: usize = 32;

/// What the human had on screen: preference only, with no authority over what
/// any act may reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub session: String,
    /// The selected session's canonical uuid: another incarnation of the name
    /// never inherits the view.
    pub id: String,
    pub focus: Option<String>,
    pub sidebar: Option<u16>,
    pub list: Option<u16>,
    pub pages: usize,
    pub rows: usize,
    pub list_top: Option<usize>,
    pub body_top: usize,
}

/// A [`View`] bound to the pane and version it is for, and to the instant it
/// was written, in epoch seconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handover {
    pub pane: String,
    pub version: String,
    pub at: i64,
    pub view: View,
}

/// What a fleet read makes of a waiting view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Apply,
    Discard,
    Defer,
}

impl Handover {
    /// The canonical text, each number clamped to its cap.
    #[must_use]
    pub fn render(&self) -> String {
        let view = &self.view;
        let small = |n: Option<u16>| n.map(|n| n.to_string()).unwrap_or_default();
        let fields = [
            "v1".to_owned(),
            self.pane.clone(),
            self.version.clone(),
            self.at.to_string(),
            view.session.clone(),
            view.id.clone(),
            "agents".to_owned(),
            view.focus.clone().unwrap_or_default(),
            small(view.sidebar),
            small(view.list),
            view.pages.min(PAGES_MAX).to_string(),
            view.rows.min(ROWS_MAX).to_string(),
            (view.list_top.map(|n| n.min(ROWS_MAX).to_string())).unwrap_or_default(),
            view.body_top.min(PAGES_MAX).to_string(),
        ];
        fields.join("|")
    }

    /// Whether this handover is for the app in `pane` running `version`, and
    /// still [`fresh`] at `now`.
    #[must_use]
    pub fn admits(&self, pane: &str, version: &str, now: i64) -> bool {
        self.pane == pane && self.version == version && fresh(self.at, now)
    }
}

/// Whether a handover written at `at` may still be used at `now`: not from the
/// future, and at most [`MAX_AGE_SECS`] old. The one age rule.
#[must_use]
pub fn fresh(at: i64, now: i64) -> bool {
    at <= now && now.saturating_sub(at) <= MAX_AGE_SECS
}

/// What a fleet read decides for the view written at `at`: `row` says the
/// carried session is a listed row, `id` is the uuid that read recorded for
/// it and `scanned` says the read was complete.
///
/// An expired view, or one whose name is now another incarnation, is
/// discarded; a read that cannot say, while it proves nothing, defers.
#[must_use]
pub fn decide(
    at: i64,
    now: i64,
    view: &View,
    row: bool,
    id: Option<&str>,
    scanned: bool,
) -> Verdict {
    if !fresh(at, now) {
        return Verdict::Discard;
    }
    match id.filter(|id| !id.is_empty()) {
        Some(id) if id != view.id => Verdict::Discard,
        Some(_) if row => Verdict::Apply,
        _ if scanned => Verdict::Discard,
        _ => Verdict::Defer,
    }
}

/// The handover `text` spells, or `None` unless it is exactly canonical.
#[must_use]
pub fn parse(text: &str) -> Option<Handover> {
    if text.len() > CAP {
        return None;
    }
    let parts: Vec<&str> = text.split('|').collect();
    let [
        tag,
        pane,
        version,
        at,
        session,
        id,
        tab,
        focus,
        sidebar,
        list,
        pages,
        rows,
        list_top,
        body_top,
    ] = parts.as_slice()
    else {
        return None;
    };
    let number = |text: &str, max: u64| -> Option<u64> {
        let lead = text.bytes().next()?;
        let canonical = lead.is_ascii_digit() && (lead != b'0' || text.len() == 1);
        let digits = text.len() <= 12 && text.bytes().all(|b| b.is_ascii_digit());
        if !(canonical && digits) {
            return None;
        }
        text.parse::<u64>().ok().filter(|n| *n <= max)
    };
    let optional = |text: &str, max: u64| match text {
        "" => Some(None),
        _ => number(text, max).map(Some),
    };
    let small = |text: &str| {
        optional(text, u64::from(u16::MAX))?
            .map(u16::try_from)
            .transpose()
            .ok()
    };
    let size = |text: &str, max: usize| usize::try_from(number(text, max as u64)?).ok();
    if !matches!(*tab, "overview" | "agents") {
        return None;
    }
    let focus = match *focus {
        "" => None,
        name if is_agent_name(name) => Some(name.to_owned()),
        _ => return None,
    };
    let list_top = optional(list_top, ROWS_MAX as u64)?;
    let list_top = list_top.map(usize::try_from).transpose().ok()?;
    let view = View {
        session: is_session_name(session).then(|| (*session).to_owned())?,
        id: is_canonical_uuid(id).then(|| (*id).to_owned())?,
        focus,
        sidebar: small(sidebar)?,
        list: small(list)?,
        pages: size(pages, PAGES_MAX)?,
        rows: size(rows, ROWS_MAX)?,
        list_top,
        body_top: size(body_top, PAGES_MAX)?,
    };
    let digits = pane.strip_prefix('%')?;
    let pane =
        (number(digits, PANE_MAX).is_some() && digits.len() <= 10).then(|| (*pane).to_owned())?;
    let version = (!version.is_empty()
        && version.len() <= VERSION_MAX
        && version.bytes().all(|b| b.is_ascii_digit() || b == b'.'))
    .then(|| (*version).to_owned())?;
    let at = number(at, AT_MAX).filter(|at| *at >= 1)?;
    (*tag == "v1").then_some(Handover {
        pane,
        version,
        at: i64::try_from(at).ok()?,
        view,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIRE: &str = "v1|%7|2026.10.99|1760259000|web|6f1c2b7e-0d3a-4c58-9a2e-5b1d7c9e0a11|agents|colead|31|9|2|0|1|4";

    #[test]
    fn what_parses_renders_back_to_the_same_text() {
        let bare = WIRE
            .replace("|agents|colead|31|9|", "|agents||||")
            .replace("|1|4", "||0");
        for text in [WIRE, bare.as_str()] {
            assert_eq!(parse(text).map(|h| h.render()).as_deref(), Some(text));
        }
    }

    /// R8: an older core wrote the Agents view as `agents` and the other as
    /// `overview`; a restart into this core reads both as the one view and
    /// writes `agents` back, never refusing the token.
    #[test]
    fn an_older_cores_overview_token_reads_as_the_agents_view() {
        let old = WIRE.replace("|agents|", "|overview|");
        assert_ne!(old, WIRE);
        assert_eq!(parse(&old).map(|h| h.render()).as_deref(), Some(WIRE));
    }

    #[test]
    fn a_matching_uuid_without_a_listed_row_is_never_applied() {
        let view = parse(WIRE).map(|h| h.view).expect("the wire parses");
        let id = Some(view.id.as_str());
        assert_eq!(decide(5, 5, &view, false, id, true), Verdict::Discard);
        assert_eq!(decide(5, 5, &view, false, id, false), Verdict::Defer);
        assert_eq!(decide(5, 5, &view, true, id, false), Verdict::Apply);
    }
}
