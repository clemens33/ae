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
//!
//! A zoomed main column uses `v3`: the same fields, the width field (always
//! there, possibly empty) and, with the panel open, the same `|mode,uuid,pages,rows`
//! tuple after it. The tag is the zoom, so a `v3` line has one spelling too, and
//! `v1` alone reads the `overview` token. The panel's own full-frame zoom rides
//! as the `full` mode, in `v2` or `v3`.
//!
//! The board's view preferences add `|desk=<parts>` after any of these
//! canonical lines. The parts that apply, comma-joined in this order, are the
//! dragged width `<width>`, `hidden` for a hidden board and `tab=<name>` for a
//! chosen tab (`|desk=44,hidden,tab=links`). The name is `coming`, `running`,
//! `waiting` or `links`: DECIDE is the default and is never written. With none
//! of the three, all three encodings keep their old bytes.

use crate::archive::is_canonical_uuid;
use crate::config::is_agent_name;
use crate::desk::Section;
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
/// The durable view's file name, directly under the state root. See
/// [`render_file`].
pub const FILE: &str = "app-view";
/// The most bytes the durable file may hold: the line and its one line feed.
pub const FILE_CAP: usize = CAP + 1;
/// The `pane` field of a durable line: the file belongs to no pane, and this
/// is the smallest id the one grammar accepts.
pub const FILE_PANE: &str = "%0";
/// The session a durable line names when no session is selected (an
/// orchestrator-only fleet draws none): the orchestrator role's own name,
/// which is never an ordinary session's. With [`NO_ID`] it is the one
/// spelling of "no selection", so the board, widths, zoom and panel still ride.
pub const NO_SESSION: &str = "orchestrator";
/// The nil uuid, the id of [`NO_SESSION`] in a durable line with no selection.
pub const NO_ID: &str = "00000000-0000-0000-0000-000000000000";

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
    /// The board's dragged width, including its left border.
    pub desk: Option<u16>,
    /// The human hid the board, whatever the width.
    pub desk_hidden: bool,
    /// The section the board shows: DECIDE until the human chose another.
    pub desk_tab: Section,
    /// The main column was zoomed: it holds the whole frame while it holds
    /// the keys, the sidebar not drawn. Independent of the panel's own zoom.
    pub zoom: bool,
    /// An open orchestrator panel, independent of the main view.
    pub orchestrator: Option<OrchestratorView>,
}

impl View {
    /// Whether this view names no selection: the one sentinel a durable line
    /// carries when no session is selected.
    #[must_use]
    pub fn unselected(&self) -> bool {
        self.session == NO_SESSION && self.id == NO_ID
    }

    /// This view without its scroll hints (rows, pages, list and body tops, and
    /// the panel's own): what a change to the view is judged on. Scroll rides
    /// along with the next write and never causes one.
    #[must_use]
    pub fn settled(&self) -> Self {
        let mut view = self.clone();
        (view.pages, view.rows, view.list_top, view.body_top) = (0, 0, None, 0);
        if let Some(panel) = view.orchestrator.as_mut() {
            (panel.pages, panel.rows) = (0, 0);
        }
        view
    }
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
        let tag = match (view.zoom, view.orchestrator.is_some()) {
            (true, _) => "v3",
            (false, true) => "v2",
            (false, false) => "v1",
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
        let line = match (&view.orchestrator, view.panel) {
            // The main column's zoom is the tag; its panel field is always there.
            (None, width) if view.zoom => format!("{line}|{}", small(width)),
            (Some(orch), width) => format!("{line}|{}|{}", small(width), orch.render()),
            (None, Some(width)) => format!("{line}|{width}"),
            (None, None) => line,
        };
        let mut desk = Vec::new();
        if let Some(width) = view.desk {
            desk.push(width.to_string());
        }
        if view.desk_hidden {
            desk.push("hidden".to_owned());
        }
        if let Some(name) = tab_name(view.desk_tab) {
            desk.push(format!("tab={name}"));
        }
        if desk.is_empty() {
            line
        } else {
            format!("{line}|desk={}", desk.join(","))
        }
    }

    /// Whether this handover is for the app in `pane` running `version`, and
    /// still [`fresh`] at `now`.
    #[must_use]
    pub fn admits(&self, pane: &str, version: &str, now: i64) -> bool {
        self.pane == pane && self.version == version && fresh(self.at, now)
    }

    /// `view` as the durable file holds it: bound to no pane, written by
    /// `version` at `at`.
    #[must_use]
    pub fn durable(version: &str, at: i64, view: View) -> Self {
        Self {
            pane: FILE_PANE.to_owned(),
            version: version.to_owned(),
            at,
            view,
        }
    }
}

/// The durable file's exact bytes: the canonical line and one line feed. The
/// file has one spelling, so what [`parse_file`] accepts renders back to the
/// same bytes.
#[must_use]
pub fn render_file(held: &Handover) -> String {
    format!("{}\n", held.render())
}

/// The handover the durable file's `bytes` spell, or `None` unless they are
/// exactly what [`render_file`] writes: at most [`FILE_CAP`] bytes, valid
/// UTF-8, one canonical line (the live parser's two legacy spellings are not
/// canonical), one line feed and nothing after it. The file is
/// hostile input like the option and the argument. Its pane, version and age
/// are never judged: the file is for no pane and outlives every upgrade, and
/// the uuid binds what it carries.
#[must_use]
pub fn parse_file(bytes: &[u8]) -> Option<Handover> {
    if bytes.len() > FILE_CAP {
        return None;
    }
    let line = std::str::from_utf8(bytes).ok()?.strip_suffix('\n')?;
    // The live option still reads the older core's `overview` token and an
    // empty trailing panel width. No older core ever wrote this file, so it
    // has exactly one spelling: what does not render back is refused.
    parse(line).filter(|held| held.render() == line)
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
    decide_saved(view, row, id, scanned)
}

/// [`decide`] without the age rule, for the durable file's view: the same
/// identity and completeness rules, whatever its age. A view that names no
/// selection ([`View::unselected`]) has nothing to select, so it is never
/// applied.
#[must_use]
pub fn decide_saved(view: &View, row: bool, id: Option<&str>, scanned: bool) -> Verdict {
    if view.unselected() {
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

/// The spelling of the board tab in the wire; `None` for DECIDE, which is the
/// default and has no spelling.
fn tab_name(section: Section) -> Option<&'static str> {
    match section {
        Section::Decide => None,
        Section::Coming => Some("coming"),
        Section::Running => Some("running"),
        Section::Waiting => Some("waiting"),
        Section::Links => Some("links"),
    }
}

/// What the text after `|desk=` spells: the dragged width, whether the board
/// is hidden and the chosen tab, as the parts `<width>`, `hidden` and
/// `tab=<name>` in that order, comma-joined, at least one and nothing else.
fn desk_suffix(text: &str) -> Option<(Option<u16>, bool, Section)> {
    let mut parts = text.split(',').peekable();
    let width = match parts.next_if(|part| part.bytes().next().is_some_and(|b| b.is_ascii_digit()))
    {
        Some(part) => Some(u16::try_from(number(part, u64::from(u16::MAX))?).ok()?),
        None => None,
    };
    let hidden = parts.next_if_eq(&"hidden").is_some();
    let tab = match parts.next_if(|part| part.starts_with("tab=")) {
        Some(part) => Section::ALL
            .into_iter()
            .find(|section| tab_name(*section) == part.strip_prefix("tab="))?,
        None => Section::Decide,
    };
    parts.next().is_none().then_some((width, hidden, tab))
}

/// The handover `text` spells, or `None` unless it is exactly canonical.
#[must_use]
pub fn parse(text: &str) -> Option<Handover> {
    if text.len() > CAP {
        return None;
    }
    let (text, desk) = match text.rsplit_once("|desk=") {
        Some((text, width)) => (text, Some(width)),
        None => (text, None),
    };
    let parts: Vec<&str> = text.split('|').collect();
    let (parts, panel, orchestrator, zoom) = match (parts.first().copied()?, parts.as_slice()) {
        ("v1", fields) if fields.len() == 14 => (fields, "", None, false),
        ("v1", [fields @ .., panel])
            if fields.len() == 14 && (!panel.is_empty() || desk.is_none()) =>
        {
            (fields, *panel, None, false)
        }
        ("v2", [fields @ .., panel, orch]) if fields.len() == 14 => {
            (fields, *panel, Some(OrchestratorView::parse(orch)?), false)
        }
        ("v3", [fields @ .., panel]) if fields.len() == 14 => (fields, *panel, None, true),
        ("v3", [fields @ .., panel, orch]) if fields.len() == 14 => {
            (fields, *panel, Some(OrchestratorView::parse(orch)?), true)
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
    // `overview` is the one view token an older core wrote, in `v1` alone.
    let legacy = *tab == "overview" && orchestrator.is_none() && !zoom && desk.is_none();
    if !(*tab == "agents" || legacy) {
        return None;
    }
    let focus = match *focus {
        "" => None,
        name if is_agent_name(name) => Some(name.to_owned()),
        _ => return None,
    };
    let list_top = optional(list_top, ROWS_MAX as u64)?;
    let list_top = list_top.map(usize::try_from).transpose().ok()?;
    let (desk, desk_hidden, desk_tab) = match desk {
        Some(text) => desk_suffix(text)?,
        None => (None, false, Section::Decide),
    };
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
        desk,
        desk_hidden,
        desk_tab,
        zoom,
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
#[path = "appzoom_wire_spec.rs"]
mod appzoom_wire_spec;

#[cfg(test)]
mod tests {
    use super::*;

    const WIRE: &str = "v1|%7|2026.10.99|1760259000|web|6f1c2b7e-0d3a-4c58-9a2e-5b1d7c9e0a11|agents|colead|31|9|2|0|1|4";
    const ORCH_ID: &str = "0199c0de-bbbb-4890-abcd-ef0123456789";

    #[test]
    fn the_file_reads_one_spelling_where_the_live_option_reads_two_more() {
        let canonical = format!("{WIRE}\n");
        let held = parse_file(canonical.as_bytes()).expect("the canonical line");
        assert_eq!(render_file(&held), canonical, "byte for byte");
        let overview = WIRE.replacen("|agents|", "|overview|", 1);
        let empty_width = format!("{WIRE}|");
        for legacy in [overview, empty_width] {
            assert!(parse(&legacy).is_some(), "the option still reads {legacy}");
            assert_eq!(parse_file(format!("{legacy}\n").as_bytes()), None);
        }
        let open = format!("{}||full,{ORCH_ID},3,2", WIRE.replacen("v1|", "v2|", 1));
        let held = parse_file(format!("{open}\n").as_bytes()).expect("an open panel");
        assert_eq!(render_file(&held), format!("{open}\n"));
    }

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
    fn a_board_width_extends_each_canonical_view_without_changing_older_wires() {
        let open_panel = format!("side,{ORCH_ID},0,0");
        let side = WIRE.replacen("v1|", "v2|", 1);
        let zoom = WIRE.replacen("v1|", "v3|", 1);
        for old in [
            WIRE.to_owned(),
            format!("{WIRE}|56"),
            format!("{side}||{open_panel}"),
            format!("{zoom}|"),
            format!("{zoom}|56|{open_panel}"),
        ] {
            let old_view = parse(&old).expect("an old view");
            assert_eq!(old_view.view.desk, None);
            assert_eq!(old_view.render(), old);

            let extended = format!("{old}|desk=44");
            let new_view = parse(&extended).expect("a view with a board width");
            assert_eq!(new_view.view.desk, Some(44));
            assert_eq!(new_view.render(), extended);
        }
        let overview = WIRE.replace("|agents|", "|overview|");
        for bad in [
            format!("{WIRE}||desk=44"),
            format!("{overview}|desk=44"),
            format!("{WIRE}|desk=44|desk=45"),
        ] {
            assert!(parse(&bad).is_none(), "refuse {bad:?}");
        }
    }

    #[test]
    fn a_hidden_board_extends_each_canonical_view_with_or_without_a_width() {
        let open_panel = format!("side,{ORCH_ID},0,0");
        let side = WIRE.replacen("v1|", "v2|", 1);
        let zoom = WIRE.replacen("v1|", "v3|", 1);
        for old in [
            WIRE.to_owned(),
            format!("{WIRE}|56"),
            format!("{side}||{open_panel}"),
            format!("{zoom}|"),
            format!("{zoom}|56|{open_panel}"),
        ] {
            for (suffix, desk) in [("hidden", None), ("44,hidden", Some(44))] {
                let text = format!("{old}|desk={suffix}");
                let read = parse(&text).expect("a view with a hidden board");
                assert_eq!((read.view.desk, read.view.desk_hidden), (desk, true));
                assert_eq!(read.render(), text);
            }
            let shown = parse(&format!("{old}|desk=44")).expect("a view with a board width");
            assert!(!shown.view.desk_hidden);
            assert!(!parse(&old).expect("an old view").view.desk_hidden);
        }
    }

    #[test]
    fn a_hidden_board_has_one_spelling_and_the_overview_token_never_carries_it() {
        let overview = WIRE.replace("|agents|", "|overview|");
        for bad in [
            format!("{WIRE}|desk="),
            format!("{WIRE}|desk=,hidden"),
            format!("{WIRE}|desk=hidden,hidden"),
            format!("{WIRE}|desk=hidden,44"),
            format!("{WIRE}|desk=44,hidden,hidden"),
            format!("{WIRE}|desk=44,shown"),
            format!("{WIRE}|desk=44, hidden"),
            format!("{WIRE}|desk=Hidden"),
            format!("{WIRE}|desk=hidden|desk=44"),
            format!("{WIRE}||desk=hidden"),
            format!("{overview}|desk=hidden"),
            format!("{overview}|desk=44,hidden"),
        ] {
            assert!(parse(&bad).is_none(), "refuse {bad:?}");
        }
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
