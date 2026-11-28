//! The settings overlay's tab bodies.

use ratatui_core::style::{Modifier, Style};

use crate::app::model::SettingsTab;
use crate::app::settings::{
    self, AboutFacts, ConfigView, InstructionsView, RowTone, SettingsBodies,
};
use crate::console::wrap::wrap;
use crate::quota::view::Tone;
use crate::theme::Mark;
use crate::time::Timestamp;

use super::paint::{LEFT, Paint, drawn_cols, fit};
use super::{accounts_summary, stats_body};

/// The browse keys the Keys tab lists, in the order it shows them: the keys
/// and what they do.
const BROWSE_KEYS: [(&str, &str); 22] = [
    ("1-9", "select the Nth session"),
    ("j/k Up/Down", "previous / next session"),
    ("!", "next session needing you"),
    ("oo", "open seat; panel: orchestrator pane; twice in 2 s"),
    (
        "O F X",
        "orchestrator panel; F, X: zoom or unzoom; click pane",
    ),
    ("m", "session menu; right-click a card too"),
    ("s", "start / stop the selected session"),
    ("z", "show / hide stopped sessions"),
    ("Z", "show / hide the spawned seats"),
    ("v", "peek at the seat's screen"),
    ("r", "read the marked turn's first file link"),
    ("n/p", "step the seat cursor (▹ or -)"),
    ("< >", "narrow / widen the side panel, 4 cells"),
    ("PgUp/PgDn", "scroll a page"),
    ("[ ]", "mark older / newer turn"),
    ("y", "copy the marked turn"),
    ("Enter i", "write; a paste writes too"),
    ("Esc", "clear the cursor, then go home"),
    ("◂ or <", "marks the home session"),
    (",", "settings"),
    ("?", "this help"),
    ("qq", "quit: twice within 2 s"),
];

/// Plain-text links; no OSC 8 in this slice.
const SETTINGS_LINKS: [(&str, &str); 3] = [
    ("repo", "https://github.com/clemens33/ae"),
    ("releases", "https://github.com/clemens33/ae/releases"),
    (
        "docs",
        "https://github.com/clemens33/ae/blob/main/docs/app.md",
    ),
];

/// The Config tab's orchestrator profile block, toned: the rows `settings`
/// lays out for a body `room` cells wide. A row the writer cannot edit says why
/// and offers nothing.
fn orchestrator_rows(bodies: &SettingsBodies, paint: Paint, room: usize) -> Vec<(String, Style)> {
    let dim = paint.fg(|p| p.dim);
    let text = paint.fg(|p| p.text);
    let head = paint.fg(|p| p.text).add_modifier(Modifier::BOLD);
    settings::orchestrator_block(bodies, room)
        .rows
        .into_iter()
        .map(|(tone, row)| {
            let style = match tone {
                RowTone::Plain => text,
                RowTone::Dim => dim,
                RowTone::Head => head,
                RowTone::Critical => paint.fg(|p| p.needs_you),
            };
            (row, style)
        })
        .collect()
}

/// The Config tab's effective-value rows: the session, the files and every
/// configured key with its source.
pub(super) fn config_rows(bodies: &SettingsBodies, paint: Paint) -> Vec<(String, Style)> {
    let dim = paint.fg(|p| p.dim);
    let text = paint.fg(|p| p.text);
    let clean = crate::board::terminal_text;
    match &bodies.config {
        ConfigView::Loading => vec![("loading".to_owned(), dim)],
        ConfigView::Unreadable(why) => vec![(clean(why), text)],
        ConfigView::Rows { header, rows } => {
            let mut out = vec![
                (
                    format!(
                        "session: {}",
                        header
                            .session
                            .as_deref()
                            .map(clean)
                            .as_deref()
                            .unwrap_or("none")
                    ),
                    dim,
                ),
                (format!("global: {}", clean(&header.global)), dim),
            ];
            if let Some(current) = header.current_global.as_ref() {
                out.push((format!("current global: {}", clean(current)), dim));
            }
            out.extend(rows.iter().map(|row| {
                let mut line = format!(
                    "{} = {}  ({})",
                    row.key,
                    clean(&row.value),
                    row.source.word()
                );
                if row.global_only {
                    line.push_str("  global only");
                }
                (line, text)
            }));
            out
        }
    }
}

/// What [`orchestrator_tail`] lays out: the rows, and while the picker is open the
/// body rows of its heading and its highlighted choice.
pub(super) struct OrchestratorTail {
    rows: Vec<(String, Style)>,
    pub(super) picker: Option<(usize, usize)>,
}

/// The orchestrator block under the `above` effective-value rows, set off by
/// one blank row, and the body rows of the picker's heading and highlight when
/// it is open. The block follows the effective values, so the tab keeps its top.
pub(super) fn orchestrator_tail(
    bodies: &SettingsBodies,
    paint: Paint,
    width: u16,
    above: usize,
) -> OrchestratorTail {
    let room = usize::from(width.saturating_sub(LEFT)) + 1;
    let block = settings::orchestrator_block(bodies, room);
    let gap = usize::from(above > 0);
    let picker = block
        .picker
        .map(|(top, at)| (above + gap + top, above + gap + at));
    let mut rows = Vec::new();
    if above > 0 {
        rows.push((String::new(), paint.fg(|p| p.text)));
    }
    rows.extend(orchestrator_rows(bodies, paint, room));
    OrchestratorTail { rows, picker }
}

/// The overlay body's styled rows: every field through the terminal-text
/// neutraliser, because paths and config values are hostile text.
pub(super) fn settings_body(
    tab: SettingsTab,
    bodies: &SettingsBodies,
    paint: Paint,
    width: u16,
    icons: bool,
) -> Vec<(String, Style)> {
    let dim = paint.fg(|p| p.dim);
    let text = paint.fg(|p| p.text);
    let head = paint.fg(|p| p.text).add_modifier(Modifier::BOLD);
    let clean = crate::board::terminal_text;
    let mut body = match tab {
        SettingsTab::Quota => match bodies.quota.as_ref() {
            None => vec![("loading".to_owned(), dim)],
            Some(scopes) if scopes.is_empty() => vec![("no quota scopes read".to_owned(), dim)],
            Some(scopes) => crate::quota::view::layout(
                scopes,
                crate::quota::view::Geometry {
                    width: usize::from(width.saturating_sub(LEFT)),
                    rows: None,
                    icons,
                },
            )
            .into_iter()
            .map(|line| {
                let style = match line.tone {
                    Tone::Plain => text,
                    Tone::Dim => dim,
                    Tone::Head => head,
                    Tone::Low => paint.fg(|p| p.waiting_agent),
                    Tone::Critical => paint.fg(|p| p.needs_you),
                    Tone::Stale => paint.fg(|p| p.stale),
                };
                (clean(&line.text), style)
            })
            .collect(),
        },
        SettingsTab::Config => {
            let mut out = config_rows(bodies, paint);
            let above = out.len();
            out.extend(orchestrator_tail(bodies, paint, width, above).rows);
            out
        }
        SettingsTab::About => match bodies.about.as_ref() {
            None => vec![("loading".to_owned(), dim)],
            Some(about) => about_body(about, text),
        },
        SettingsTab::Instructions => instructions_body(&bodies.instructions, width, paint),
        SettingsTab::Keys => {
            let mut rows = marks_legend(paint, icons);
            rows.extend(keys_body(paint));
            rows
        }
        SettingsTab::Accounts => bodies.live_seats.as_ref().map_or_else(
            || vec![("loading".to_owned(), dim)],
            |seats| live_seat_rows(seats, paint, width),
        ),
        SettingsTab::Statistics => stats_body::rows(&bodies.stats, paint, width),
    };
    // Both snapshots keep their refresh verdict; seats also carry fleet age.
    if matches!(tab, SettingsTab::Quota | SettingsTab::Accounts)
        && let Some(kept) = &bodies.quota_kept
    {
        body.insert(0, (clean(kept), paint.fg(|p| p.stale)));
    }
    body
}

fn live_seat_rows(
    seats: &crate::quota::LiveSeats,
    paint: Paint,
    width: u16,
) -> Vec<(String, Style)> {
    let clean = crate::board::terminal_text;
    let dim = paint.fg(|p| p.dim);
    let text = paint.fg(|p| p.text);
    let head = text.add_modifier(Modifier::BOLD);
    let mut rows = Vec::new();
    let room = width.saturating_sub(LEFT);
    let line = |value: &str| fit(&clean(value).replace(['\n', '\r', '\t'], " "), "", room);
    let fields = |row: &crate::quota::LiveRow| {
        [
            row.session_seat.clone(),
            row.profile.clone(),
            row.model.clone(),
            row.effort.clone(),
            format!("activity={}", row.activity),
            format!("declared={}", row.declared),
        ]
        .map(|value| line(&value))
    };
    let labels = [
        "session:agent",
        "profile",
        "model",
        "effort",
        "activity",
        "declared",
    ];
    let mut columns = labels.map(drawn_cols);
    for row in seats.groups.iter().flat_map(|group| &group.rows) {
        for (column, value) in columns.iter_mut().zip(fields(row)) {
            *column = (*column).max(drawn_cols(&value));
        }
    }
    let aligned = |values: &[String; 6]| {
        let mut out = String::new();
        for (index, (value, column)) in values.iter().zip(columns).enumerate() {
            out.push_str(value);
            if index < columns.len() - 1 {
                out.push_str(
                    &" ".repeat(usize::from(column.saturating_sub(drawn_cols(value))) + 2),
                );
            }
        }
        line(&out)
    };
    if let Some(at) = seats.groups.first().and_then(|group| group.observed_at) {
        rows.push((
            line(&format!(
                "snapshot {} ago",
                crate::quota::age_label(Timestamp::now().epoch().saturating_sub(at))
            )),
            dim,
        ));
    }
    if seats.incomplete {
        rows.push((line("incomplete fleet snapshot"), paint.fg(|p| p.stale)));
    }
    rows.push((line("Live-seat summary"), head));
    for summary in accounts_summary::lines(seats) {
        rows.extend(
            crate::console::wrap::wrap_in(&summary, usize::from(room), 0, 0)
                .into_iter()
                .map(|(gap, value)| (line(&format!("{}{value}", " ".repeat(gap))), text)),
        );
    }
    rows.push((String::new(), dim));
    rows.push((line("Live seats by account"), head));
    if width >= 80 {
        rows.push((aligned(&labels.map(str::to_owned)), head));
    }
    if seats.groups.is_empty() {
        rows.push((line("no live seats"), dim));
    }
    for group in &seats.groups {
        rows.push((line(&group.heading), head));
        if let Some(gap) = &group.gap {
            rows.push((line(gap), paint.fg(|p| p.stale)));
        }
        if group.unknown_liveness > 0 {
            rows.push((
                line(&format!("{} liveness unknown", group.unknown_liveness)),
                dim,
            ));
        }
        for row in &group.rows {
            let tone = match row.activity.as_str() {
                "busy" => paint.fg(|p| p.working),
                "unknown" => dim,
                _ => text,
            };
            rows.push((aligned(&fields(row)), tone));
        }
    }
    rows
}

/// The write, held and settings keys the help lists beside the browse table.
const WRITE_KEYS: [(&str, &str); 23] = [
    ("Enter", "send the line"),
    ("Alt+Enter ^J", "new line in the draft"),
    ("^V", "attach a clipboard PNG"),
    ("Alt+y", "copy the whole draft"),
    ("Esc", "keep the draft, browse"),
    ("^C", "clear a draft; an empty one quits at once"),
    ("^C", "just after a clear: arms; again in 2 s quits"),
    ("Left Right", "move the cursor"),
    ("Home End", "start / end of line"),
    ("PgUp PgDn", "scroll the chat a page"),
    ("^A ^E", "start / end of line"),
    ("Backspace", "erase before the cursor"),
    ("Delete", "erase at the cursor"),
    ("^W", "erase the word before the cursor"),
    ("Alt+Bksp", "same, stopping at punctuation (Option as Meta)"),
    ("^K", "erase to the end of the line"),
    ("^U", "clear the whole draft, every line"),
    ("^Y", "put the cleared draft / last erase back, once"),
    ("paste", "literal text, never Enter; CR becomes LF"),
    ("@seat text", "ask that seat"),
    ("/close [id]", "withdraw your open ask"),
    ("/open seat", "open that seat's pane"),
    ("!command", "run it: a popup in tmux, else started"),
];

const HELD_KEYS: [(&str, &str); 4] = [
    ("Enter", "try to write again"),
    ("Esc", "browse"),
    ("^C", "quit; a kept draft clears first"),
    ("any other", "swallowed; a paste too"),
];

const MOUSE_KEYS: [(&str, &str); 5] = [
    ("click", "select a session, tab, seat or turn"),
    ("click mic", "panel: start or stop Claude dictation"),
    ("wheel", "scroll 3 rows (the list: 1 session)"),
    ("drag", "a border resizes a column or the list"),
    (
        "drag text",
        "select drawn text, copy on release; Esc clears",
    ),
];

const PEEK_KEYS: [(&str, &str); 6] = [
    ("j/k Up/Down", "scroll a line"),
    ("PgUp/PgDn", "scroll a page"),
    ("Esc q v", "back to chat; click the session too"),
    ("^C", "quit; a draft clears first"),
    ("wheel", "scroll 3 rows per notch"),
    ("any other", "swallowed; nothing reaches the pane"),
];

const READER_KEYS: [(&str, &str); 9] = [
    ("j/k Up/Down", "scroll a line"),
    ("PgUp/PgDn", "scroll a page"),
    ("g/G", "top / bottom of the file; Home/End too"),
    ("m", "Markdown: rendered / raw"),
    ("n/p", "next / previous file link"),
    ("Esc q r", "back to the lane"),
    ("^C", "quit; a draft clears first"),
    ("wheel", "scroll 3 rows per notch"),
    ("any other", "swallowed; nothing is typed or written"),
];

const SETTINGS_KEYS: [(&str, &str); 13] = [
    ("Tab 1-7", "show that tab"),
    ("Left/Right", "Statistics: previous or next inner tab"),
    ("m", "Statistics: all retained or this session"),
    ("o", "Config: pick the orchestrator profile; j/k Enter Esc"),
    ("j/k Up/Down", "scroll a line"),
    ("PgUp/PgDn", "scroll a page"),
    ("?", "show these keys"),
    ("Esc q ,", "close"),
    ("^C", "quit; a draft clears first"),
    ("click tab", "show that tab"),
    ("click gear", "or Esc close label: closes it"),
    ("wheel", "scroll 3 rows per notch"),
    ("any other", "swallowed; a paste too"),
];

/// What each of the eight marks means, in the glyph vocabulary being drawn.
/// The words are the ones a seat row prints ([`crate::theme::seat_word`]); the
/// rest says what the mark rests on, so a finished seat is only ever one that
/// declared it and an empty input box is never read as completion.
pub(super) fn marks_legend(paint: Paint, icons: bool) -> Vec<(String, Style)> {
    let (text, head) = (paint.fg(|p| p.text), paint.fg(|p| p.title));
    let mut rows = vec![(
        "Marks (the glyph before a name)".to_owned(),
        head.add_modifier(Modifier::BOLD),
    )];
    rows.extend(
        [
            (Mark::Dead, "dead — agent process gone"),
            (
                Mark::NeedsYou,
                "needs you — waiting on you / blocked / problem; the reason follows",
            ),
            (
                Mark::Stale,
                "unknown — state unestablished, or silent past the window",
            ),
            (Mark::Working, "working — a turn is running"),
            (
                Mark::Background,
                "background — a job of the seat runs while it is quiet",
            ),
            (
                Mark::WaitingAgent,
                "waiting on an agent — declared; its reason may name whom",
            ),
            (Mark::Done, "finished — only when the agent declared done"),
            (
                Mark::Idle,
                "no current status — alive, not working, no current done/waiting/blocked declaration",
            ),
        ]
        .map(|(mark, words)| (format!("{:<2}{words}", mark.glyph(icons)), text)),
    );
    rows
}

/// The Keys tab's rows: every key of browse, write, held and the overlay.
pub(super) fn keys_body(paint: Paint) -> Vec<(String, Style)> {
    let (text, head) = (paint.fg(|p| p.text), paint.fg(|p| p.title));
    let head = head.add_modifier(Modifier::BOLD);
    let mut rows = vec![("Browse".to_owned(), head)];
    rows.extend(
        BROWSE_KEYS
            .iter()
            .map(|(keys, does)| (format!("{keys:<12}{does}"), text)),
    );
    rows.push((
        format!("{:<12}quit at once; a kept draft clears first", "^C"),
        text,
    ));
    rows.push(("Any other key cancels qq / oo.".to_owned(), text));
    let sections = [
        ("Write (Enter or a paste starts it)", &WRITE_KEYS[..]),
        ("Held (writing was refused)", &HELD_KEYS[..]),
        ("Peek (v on a highlighted seat)", &PEEK_KEYS[..]),
        ("Reader (r on a marked turn)", &READER_KEYS[..]),
        ("Settings (the gear, or ,)", &SETTINGS_KEYS[..]),
        ("Mouse (outside Settings)", &MOUSE_KEYS[..]),
    ];
    for (title, keys) in sections {
        rows.push((title.to_owned(), head));
        rows.extend(
            keys.iter()
                .map(|(keys, does)| (format!("{keys:<12}{does}"), text)),
        );
    }
    rows
}

/// The about tab's rows: versions, paths, the recorded server, plain links.
fn about_body(about: &AboutFacts, text: Style) -> Vec<(String, Style)> {
    let clean = crate::board::terminal_text;
    let mut rows = vec![
        (format!("ae   {}", crate::shape::version_line()), text),
        (
            format!(
                "tmux {} ({})",
                clean(&about.tmux_version),
                about.tmux_verdict
            ),
            text,
        ),
        (format!("state root   {}", clean(&about.state_root)), text),
        (format!("config   {}", clean(&about.config_file)), text),
        (format!("server   {}", clean(&about.server)), text),
    ];
    rows.extend(
        SETTINGS_LINKS
            .iter()
            .map(|(label, link)| (format!("{label}   {link}"), text)),
    );
    rows
}

/// The instructions tab's rows: the render-now line, the custom instructions,
/// each generic rule once under its audience and one line per seat naming the
/// rules it receives. Every field is neutralised first and then wrapped to the
/// panel, a continuation row hanging two cells; nothing is cut here, the panel
/// scrolls.
pub(super) fn instructions_body(
    view: &InstructionsView,
    width: u16,
    paint: Paint,
) -> Vec<(String, Style)> {
    let dim = paint.fg(|p| p.dim);
    let text = paint.fg(|p| p.text);
    let head = paint.fg(|p| p.text).add_modifier(Modifier::BOLD);
    // `wrap` keeps a one-cell bar free ahead of every row.
    let room = usize::from(width.saturating_sub(LEFT)) + 1;
    let rows = |body: &str, style: Style| -> Vec<(String, Style)> {
        wrap(&crate::board::terminal_text(body), room, 0)
            .into_iter()
            .map(|(gap, piece)| (format!("{}{piece}", " ".repeat(gap)), style))
            .collect()
    };
    let protocol = match view {
        InstructionsView::Loading => return vec![("loading".to_owned(), dim)],
        InstructionsView::Gap(why) => return rows(why, text),
        InstructionsView::Ready(protocol) => protocol,
    };
    let mut out = rows(&format!("session: {}", protocol.session), dim);
    out.extend(rows(
        &format!(
            "<session>, <helpers> and <owner line> stand for what a launch fills in per seat. Rendered now by {}: the text a launch injects today. A running seat holds the text of its own launch time.",
            crate::shape::version_line()
        ),
        dim,
    ));
    out.push((String::new(), text));
    out.extend(rows("Custom instructions ([prompt] instructions)", head));
    if let Some(custom) = &protocol.custom {
        out.extend(rows(
            &format!("source: {} {}", custom.source.word(), custom.file),
            dim,
        ));
        out.extend(rows(&custom.text, text));
    } else {
        let named = if protocol.files.is_empty() {
            "no config file".to_owned()
        } else {
            protocol.files.join(", ")
        };
        out.extend(rows(&format!("none in {named}"), dim));
    }
    // Drawn only when the key is in force, so a config without it reads as before.
    if let Some(spawn) = &protocol.spawn {
        out.push((String::new(), text));
        out.extend(rows(
            "Spawn instructions ([prompt] spawn_instructions)",
            head,
        ));
        out.extend(rows(
            &format!("source: {} {}", spawn.source.word(), spawn.file),
            dim,
        ));
        out.extend(rows(
            "The lead pair and a seat ae cannot classify get this text whole; every other seat gets one line pointing at the same section of workspace.md.",
            dim,
        ));
        out.extend(rows(&spawn.text, text));
    }
    for rule in &protocol.rules {
        out.push((String::new(), text));
        out.extend(rows(&format!("Rules for {}", rule.audience.word()), head));
        if let Some(owner) = &rule.owner {
            out.extend(rows(owner, dim));
        }
        out.extend(rows(&rule.text, text));
    }
    out.push((String::new(), text));
    out.extend(rows("Seats", head));
    for seat in &protocol.seats {
        let receives = match &seat.receives {
            Ok(audiences) => audiences
                .iter()
                .map(|audience| audience.word())
                .collect::<Vec<_>>()
                .join(" + "),
            Err(why) => format!("gap: {why}"),
        };
        let line = format!("{} · {} · {} · {receives}", seat.name, seat.slot, seat.role);
        out.extend(rows(&line, text));
    }
    out
}
