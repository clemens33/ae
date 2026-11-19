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
//! `v1|pane|version|at|session|uuid|tab|focus|sidebar|list|pages|rows|list_top|body_top[|panel]`
//!
//! Empty means `None`. Integers are canonical decimal. The trailing `panel`
//! field is written only when the side panel's width was dragged, so a line
//! without one is the 14 fields an older core wrote. [`parse`] accepts only
//! text that [`Handover::render`] could have written, except that the
//! `overview` view token an older core wrote reads as `agents`, the one view,
//! and an empty trailing `panel` field reads as the line without it.
//!
//! An open panel on a proved seat uses `v2`: the same fields, the width
//! field (possibly empty), and `|mode,uuid,pages,rows`. This version has
//! one spelling and requires the panel tuple.

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
    /// The side panel's width as dragged, its border column included; the
    /// layout clamps it, never this preference.
    pub panel: Option<u16>,
    /// An open orchestrator panel, independent of the main view.
    pub orchestrator: Option<OrchestratorView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelMode {
    Side,
    SideMain,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchestratorView {
    pub id: String,
    pub mode: PanelMode,
    pub pages: usize,
    pub rows: usize,
}

impl OrchestratorView {
    fn render(&self) -> String {
        let mode = match self.mode {
            PanelMode::Side => "side",
            PanelMode::SideMain => "side-main",
            PanelMode::Full => "full",
        };
        let pages = self.pages.min(PAGES_MAX);
        let rows = self.rows.min(ROWS_MAX);
        format!("{mode},{},{pages},{rows}", self.id)
    }

    fn parse(text: &str) -> Option<Self> {
        let (mode, rest) = text.split_once(',')?;
        let (id, rest) = rest.split_once(',')?;
        let (pages, rows) = rest.split_once(',')?;
        Some(Self {
            mode: match mode {
                "side" => PanelMode::Side,
                "side-main" => PanelMode::SideMain,
                "full" => PanelMode::Full,
                _ => return None,
            },
            id: is_canonical_uuid(id).then(|| id.to_owned())?,
            pages: size(pages, PAGES_MAX)?,
            rows: size(rows, ROWS_MAX)?,
        })
    }
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
        let tag = if view.orchestrator.is_some() {
            "v2"
        } else {
            "v1"
        };
        let fields = [
            tag.to_owned(),
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
        let line = fields.join("|");
        match (&view.orchestrator, view.panel) {
            (Some(orch), width) => format!("{line}|{}|{}", small(width), orch.render()),
            (None, Some(width)) => format!("{line}|{width}"),
            (None, None) => line,
        }
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

fn number(text: &str, max: u64) -> Option<u64> {
    let lead = text.bytes().next()?;
    let canonical = lead.is_ascii_digit() && (lead != b'0' || text.len() == 1);
    let digits = text.len() <= 12 && text.bytes().all(|b| b.is_ascii_digit());
    if !(canonical && digits) {
        return None;
    }
    text.parse::<u64>().ok().filter(|n| *n <= max)
}

fn size(text: &str, max: usize) -> Option<usize> {
    usize::try_from(number(text, max as u64)?).ok()
}

/// The handover `text` spells, or `None` unless it is exactly canonical.
#[must_use]
pub fn parse(text: &str) -> Option<Handover> {
    if text.len() > CAP {
        return None;
    }
    let parts: Vec<&str> = text.split('|').collect();
    let (parts, panel, orchestrator) = match (parts.first().copied()?, parts.as_slice()) {
        ("v1", fields) if fields.len() == 14 => (fields, "", None),
        ("v1", [fields @ .., panel]) if fields.len() == 14 => (fields, *panel, None),
        ("v2", [fields @ .., panel, orch]) if fields.len() == 14 => {
            (fields, *panel, Some(OrchestratorView::parse(orch)?))
        }
        _ => return None,
    };
    let [
        _,
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
    ] = parts
    else {
        return None;
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
    if !(*tab == "agents" || *tab == "overview" && orchestrator.is_none()) {
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
        panel: small(panel)?,
        orchestrator,
    };
    let digits = pane.strip_prefix('%')?;
    let pane =
        (number(digits, PANE_MAX).is_some() && digits.len() <= 10).then(|| (*pane).to_owned())?;
    let version = (!version.is_empty()
        && version.len() <= VERSION_MAX
        && version.bytes().all(|b| b.is_ascii_digit() || b == b'.'))
    .then(|| (*version).to_owned())?;
    let at = number(at, AT_MAX).filter(|at| *at >= 1)?;
    Some(Handover {
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
    const ORCH_ID: &str = "0199c0de-bbbb-4890-abcd-ef0123456789";

    #[test]
    fn v2_round_trips_both_width_forms_and_clamps_panel_scroll() {
        let base = WIRE.replacen("v1|", "v2|", 1);
        for width in ["", "56"] {
            let text = format!("{base}|{width}|side,{ORCH_ID},1000000,65535");
            let mut handover = parse(&text).expect("both v2 width forms are canonical");
            assert_eq!(handover.render(), text);
            let panel = handover.view.orchestrator.as_mut().expect("open panel");
            (panel.pages, panel.rows) = (usize::MAX, usize::MAX);
            assert_eq!(
                handover.render(),
                text,
                "the writer caps both scroll values"
            );
        }
    }

    #[test]
    fn v2_requires_the_exact_open_panel_grammar() {
        let base = WIRE.replacen("v1|", "v2|", 1);
        let tuple = format!("full,{ORCH_ID},0,0");
        let canonical = format!("{base}||{tuple}");
        assert!(parse(&canonical).is_some());
        for bad in [
            base.clone(),
            format!("{base}|56"),
            format!("{base}||"),
            format!("{WIRE}||{tuple}"),
            canonical.replace("|agents|", "|overview|"),
            format!("{base}||full,{ORCH_ID},0"),
            format!("{base}||{tuple},0"),
            canonical.replace("full,", "unknown,"),
            canonical.replace(ORCH_ID, &ORCH_ID.to_ascii_uppercase()),
            canonical.replace(ORCH_ID, &ORCH_ID[..35]),
            format!("{base}||full,{ORCH_ID},1000001,0"),
            format!("{base}||full,{ORCH_ID},0,65536"),
        ] {
            assert!(parse(&bad).is_none(), "refuse {bad:?}");
        }
    }

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

    /// A dragged panel width rides as a trailing field written only when
    /// there is one; an empty field reads as the line without it.
    #[test]
    fn a_panel_width_is_a_trailing_field_written_only_when_dragged() {
        let wide = format!("{WIRE}|56");
        let read = parse(&wide).expect("a dragged width parses");
        assert_eq!(read.view.panel, Some(56));
        assert_eq!(read.render(), wide);
        let empty = parse(&format!("{WIRE}|")).expect("an empty width parses");
        assert_eq!((empty.view.panel, empty.render()), (None, WIRE.to_owned()));
        assert_eq!(parse(WIRE).map(|h| h.view.panel), Some(None));
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
