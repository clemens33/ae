//! The SESSION-SCOPED look: the palette, the glyph set, and the tmux formats
//! ae draws its own sessions with.
//!
//! Everything here is PURE — colours, glyphs and format strings in, format
//! strings out. Who writes them and when is [`crate::session_launch`]'s and the
//! watchdog's business; what they mean is this module's.
//!
//! Three rules hold the whole design together:
//!
//! * **Session-scoped, never global.** `status`, `status-style` and the two
//!   `status-format` indices are SESSION options, so ae writes them on its own
//!   sessions and a non-ae session on the same server keeps the user's theme.
//!   The pane borders, the window entries and the menu/popup styles are WINDOW
//!   options — measured on tmux 3.7b, `set-option -t <session>` on one of those
//!   lands on the session's CURRENT window alone — so they are stamped per
//!   window, and never on the global-window table.
//! * **Three writers, one job each.** A LAUNCH writes the layout, the look
//!   facts and the attention seed. A RENAME rewrites the layout and the facts
//!   and leaves every verdict alone. The WATCHDOG owns the verdicts, refreshing
//!   them each cycle, and rewrites the layout only when the look stamp says the
//!   look changed under it. Every format references `@ae_*` rather than a fact
//!   of its own, so nothing here polls and no format ever runs `#()`.
//! * **Spaces, not commas, inside `#[…]`.** A `#{W:a,b}` or `#{?c,a,b}` splits
//!   on commas, so a comma-separated style list inside one would tear the
//!   format in half. tmux reads a space-separated style list identically, and
//!   its own default `status-format[0]` is written that way for this reason.

use std::fmt::Write as _;

use crate::attention::Reason;

// ---------------------------------------------------------------------------
// the palette
// ---------------------------------------------------------------------------

/// The colours one ae session is drawn in.
///
/// True colour by hex: tmux maps an RGB value down to the closest entry of the
/// 256-colour cube when the terminal does not report `RGB`, so one spelling
/// serves both and `terminal-features` — a SERVER option, and therefore the
/// user's — is never touched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// The name this palette is chosen by.
    pub name: &'static str,
    /// The status bar's ground.
    pub base: &'static str,
    /// The raised ground a segment sits on.
    pub panel: &'static str,
    /// Text on `base` / `panel`.
    pub text: &'static str,
    /// Text that is present but not the point.
    pub dim: &'static str,
    /// Text ON an accent — dark, because every accent is light.
    pub ink: &'static str,
    /// Rules: the pane borders, and the menu and popup frames.
    pub border: &'static str,
    /// The ground under the menu row the cursor is on.
    pub selected: &'static str,
    /// Text on `selected`.
    pub selected_ink: &'static str,
    /// The menu title, and nothing else — one bright word per surface.
    pub title: &'static str,
    /// A human is the next step.
    pub needs_you: &'static str,
    /// Movement.
    pub working: &'static str,
    /// A declared finish.
    pub done: &'static str,
    /// Silence, and what ae could not establish — never green.
    pub stale: &'static str,
    /// A process that is gone. The one alarm colour, spent on nothing else.
    pub dead: &'static str,
}

impl Palette {
    /// **Darcula** — the classic dark theme of the `JetBrains` IDEs, and the default.
    ///
    /// Every token is the IDE's own: the accents are its syntax colours, so a
    /// mark reads the way the code in the pane below it already does.
    pub const DARCULA: Self = Self {
        name: "darcula",
        base: "#313335",
        panel: "#3C3F41",
        text: "#A9B7C6",
        dim: "#808080",
        ink: "#2B2B2B",
        border: "#555555",
        selected: "#214283",
        selected_ink: "#A9B7C6",
        title: "#FFC66D",
        needs_you: "#CC7832",
        working: "#6897BB",
        done: "#6A8759",
        stale: "#9876AA",
        dead: "#FF6B68",
    };

    /// **A — neutral dark.** Grey-blue neutrals under ae's own accents.
    ///
    /// Amber for "needs you" rather than red: red is what a failure looks like,
    /// and three of the five reasons behind this mark are an agent waiting
    /// politely.
    pub const NEUTRAL: Self = Self {
        name: "a",
        base: "#16181d",
        panel: "#1e2128",
        text: "#c8ccd4",
        dim: "#6f7787",
        ink: "#0d0f12",
        border: "#6f7787",
        selected: "#57b6c2",
        selected_ink: "#0d0f12",
        title: "#e5a03c",
        needs_you: "#e5a03c",
        working: "#57b6c2",
        done: "#7fbf6a",
        stale: "#8a94a6",
        dead: "#e0605c",
    };

    /// **B — slightly warmer.** The same accents over brown-grey neutrals.
    pub const WARM: Self = Self {
        name: "b",
        base: "#1b1917",
        panel: "#262220",
        text: "#d4ccc2",
        dim: "#857a6e",
        ink: "#14110e",
        border: "#857a6e",
        selected: "#57b6c2",
        selected_ink: "#14110e",
        title: "#e5a03c",
        needs_you: "#e5a03c",
        working: "#57b6c2",
        done: "#7fbf6a",
        stale: "#8a94a6",
        dead: "#e0605c",
    };

    /// The palette a configured name selects, or the default.
    ///
    /// ```
    /// use ae::theme::Palette;
    /// assert_eq!(Palette::named("b"), Palette::WARM);
    /// assert_eq!(Palette::named("B"), Palette::WARM);
    /// assert_eq!(Palette::named(""), Palette::DARCULA);
    /// assert_eq!(Palette::named("chartreuse"), Palette::DARCULA);
    /// ```
    #[must_use]
    pub fn named(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "a" | "neutral" => Self::NEUTRAL,
            "b" | "warm" => Self::WARM,
            _ => Self::DARCULA,
        }
    }

    /// The accent for `mark`.
    #[must_use]
    pub const fn accent(&self, mark: Mark) -> &'static str {
        match mark {
            Mark::Dead => self.dead,
            Mark::NeedsYou => self.needs_you,
            Mark::Working => self.working,
            Mark::Done => self.done,
            Mark::Stale => self.stale,
            Mark::Idle => self.dim,
        }
    }
}

// ---------------------------------------------------------------------------
// the glyph set
// ---------------------------------------------------------------------------

/// What a session, a window, a pane or a picker row is saying.
///
/// SIX states and no more: the colour carries the nuance and the pane border
/// carries the word, so the glyph only has to be legible at one character wide.
/// Every state has its own glyph as well as its own accent — a reader who
/// cannot separate two of the colours still reads two different characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// The process behind the pane is gone. Not a state anything recovers from
    /// on its own, and never drawn like a session that finished.
    Dead,
    /// A human is the next step — waiting, blocked, throttled, unanswered.
    NeedsYou,
    /// Moving, or recently moved.
    Working,
    /// Declared complete or paused.
    Done,
    /// Silent past the window, or a fact ae could not establish. The two share
    /// a glyph and never a WORD: the reason beside the mark says which it was.
    Stale,
    /// Nothing to say: no agent, or no verdict yet.
    Idle,
}

impl Mark {
    /// Every mark, most actionable first.
    pub const BY_URGENCY: [Self; 6] = [
        Self::Dead,
        Self::NeedsYou,
        Self::Stale,
        Self::Working,
        Self::Done,
        Self::Idle,
    ];

    /// The glyph, in the vocabulary `icons` selects.
    ///
    /// ```
    /// use ae::theme::Mark;
    /// assert_eq!(Mark::NeedsYou.glyph(true), "⚠");
    /// assert_eq!(Mark::NeedsYou.glyph(false), "!");
    /// ```
    #[must_use]
    pub const fn glyph(self, icons: bool) -> &'static str {
        match (self, icons) {
            (Self::Dead, true) => "✖",
            (Self::Dead, false) => "x",
            (Self::NeedsYou, true) => "⚠",
            (Self::NeedsYou, false) => "!",
            (Self::Working, true) => "●",
            (Self::Working, false) => "*",
            (Self::Done, true) => "✓",
            (Self::Done, false) => "+",
            (Self::Stale, true) => "◌",
            (Self::Stale, false) => "?",
            (Self::Idle, true) => "·",
            (Self::Idle, false) => "-",
        }
    }

    /// How urgently this mark wants a human — the fleet strip's sort key, and
    /// the number the watchdog publishes for other sessions to sort by.
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::Dead => 5,
            Self::NeedsYou => 4,
            Self::Stale => 3,
            Self::Working => 2,
            Self::Done => 1,
            Self::Idle => 0,
        }
    }

    /// The mark a published rank names, or [`Mark::Idle`] for anything else.
    #[must_use]
    pub fn from_rank(raw: &str) -> Self {
        let rank = raw.trim().parse::<u8>().unwrap_or(0);
        Self::BY_URGENCY
            .into_iter()
            .find(|mark| mark.rank() == rank)
            .unwrap_or(Self::Idle)
    }

    /// The mark an attention [`Reason`] shows as.
    ///
    /// `dead` and `stale` keep their own marks; the rest are a human's turn.
    ///
    /// ```
    /// use ae::attention::Reason;
    /// use ae::theme::Mark;
    /// assert_eq!(Mark::for_reason(Reason::WaitingUser), Mark::NeedsYou);
    /// assert_eq!(Mark::for_reason(Reason::Stale), Mark::Stale);
    /// ```
    #[must_use]
    pub const fn for_reason(reason: Reason) -> Self {
        match reason {
            Reason::Dead => Self::Dead,
            Reason::Stale => Self::Stale,
            Reason::WaitingUser | Reason::Blocked | Reason::Throttled | Reason::Unanswered => {
                Self::NeedsYou
            }
        }
    }
}

/// The spinner frames, shown in place of the working glyph while a pane
/// produced output since the watchdog's last capture.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The ASCII spinner, for the same movement without the braille block.
const ASCII_SPINNER: [&str; 4] = ["|", "/", "-", "\\"];

/// The spinner frame at `tick`, in the vocabulary `icons` selects.
///
/// ```
/// assert_eq!(ae::theme::spinner(0, true), "⠋");
/// assert_eq!(ae::theme::spinner(10, true), "⠋");
/// assert_eq!(ae::theme::spinner(1, false), "/");
/// ```
#[must_use]
pub fn spinner(tick: u64, icons: bool) -> &'static str {
    if icons {
        let index = usize::try_from(tick % SPINNER.len() as u64).unwrap_or(0);
        SPINNER.get(index).copied().unwrap_or("●")
    } else {
        let index = usize::try_from(tick % ASCII_SPINNER.len() as u64).unwrap_or(0);
        ASCII_SPINNER.get(index).copied().unwrap_or("*")
    }
}

/// Whether a look knob is ON.
///
/// UNSET means on: an unthemed session and a fresh one look the same, and every
/// one of these knobs is an OPT-OUT, so the quiet answer is the full look.
///
/// ```
/// assert!(ae::theme::wanted(""));
/// assert!(ae::theme::wanted("on"));
/// assert!(!ae::theme::wanted("off"));
/// assert!(!ae::theme::wanted("FALSE"));
/// ```
#[must_use]
pub fn wanted(value: &str) -> bool {
    !matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "off" | "false" | "no" | "0" | "none" | "ascii"
    )
}

/// Whether `@ae_icons` asks for the glyph set or the ASCII fallback.
///
/// ```
/// assert!(ae::theme::icons_wanted("on"));
/// assert!(!ae::theme::icons_wanted("ascii"));
/// ```
#[must_use]
pub fn icons_wanted(value: &str) -> bool {
    wanted(value)
}

/// The whole look of one session: what it is drawn in, and whether it is drawn
/// at all.
///
/// Three knobs and a palette, read from `[workspace]` at launch and from the
/// session's own options every watchdog cycle, so flipping one on a live
/// session takes effect on the next one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Look {
    /// The colours.
    pub palette: Palette,
    /// Whether the glyph set or its ASCII fallback is drawn.
    pub icons: bool,
    /// Whether ae DRAWS the session at all.
    ///
    /// `[workspace] theme = off` leaves `status-format`, the pane borders and
    /// the menu styles exactly as the user's own tmux configuration left them,
    /// and ae fills the `@ae_*` options anyway — so a hand-written
    /// `status-right` can show ae's facts in the user's own layout.
    pub drawn: bool,
    /// Whether anything animates. `[workspace] motion = off` freezes the
    /// spinner on its mark, for a reader who does not want movement in the
    /// corner of their eye.
    pub motion: bool,
}

impl Look {
    /// The look of a session nobody configured.
    pub const DEFAULT: Self = Self {
        palette: Palette::DARCULA,
        icons: true,
        drawn: true,
        motion: true,
    };

    /// The look four option or config VALUES describe. Empty is the default in
    /// every position, so an unset option and a fresh session agree.
    ///
    /// ```
    /// use ae::theme::{Look, Palette};
    /// assert_eq!(Look::read("", "", "", ""), Look::DEFAULT);
    /// assert!(!Look::read("", "", "off", "").drawn);
    /// assert_eq!(Look::read("", "b", "", "").palette, Palette::WARM);
    /// ```
    #[must_use]
    pub fn read(icons: &str, palette: &str, drawn: &str, motion: &str) -> Self {
        Self {
            palette: Palette::named(palette),
            icons: wanted(icons),
            drawn: wanted(drawn),
            motion: wanted(motion),
        }
    }

    /// The look, as ONE comparable value.
    ///
    /// What the layout depends on and nothing else: two looks with the same
    /// stamp draw the same bar, so a watchdog that finds a different stamp on a
    /// session knows the layout has to be rewritten. The FORMATS are part of
    /// that: [`FORMAT_VERSION`] leads the stamp, so a core whose formats
    /// changed repaints every session it inherits instead of leaving them on
    /// the layout an older core wrote.
    ///
    /// ```
    /// use ae::theme::{FORMAT_VERSION, Look};
    /// assert_eq!(Look::DEFAULT.stamp(), format!("{FORMAT_VERSION}:darcula:on:on"));
    /// assert_ne!(Look::read("off", "", "", "").stamp(), Look::DEFAULT.stamp());
    /// // Motion moves no pixel of the layout, so it is not in the stamp.
    /// assert_eq!(Look::read("", "", "", "off").stamp(), Look::DEFAULT.stamp());
    /// ```
    #[must_use]
    pub fn stamp(&self) -> String {
        format!(
            "{FORMAT_VERSION}:{}:{}:{}",
            self.palette.name,
            switch(self.icons),
            switch(self.drawn)
        )
    }

    /// The glyph for `mark`, or a spinner frame when `moving` and this look
    /// animates.
    ///
    /// The spinner is SUBORDINATE to the state: it stands in for the working
    /// glyph and for nothing else, so a pane that needs a human keeps saying so
    /// while it prints.
    #[must_use]
    pub fn glyph(&self, mark: Mark, moving: bool, tick: u64) -> &'static str {
        if moving && self.motion && mark == Mark::Working {
            spinner(tick, self.icons)
        } else {
            mark.glyph(self.icons)
        }
    }
}

// ---------------------------------------------------------------------------
// the @ae_* interface
// ---------------------------------------------------------------------------

/// SESSION — `on`/`off`, whether the glyph set or its ASCII fallback is drawn.
pub const ICONS_OPTION: &str = "@ae_icons";

/// SESSION — the session's own attention glyph, already in the chosen set.
pub const ATTENTION_GLYPH_OPTION: &str = "@ae_attn_glyph";

/// SESSION — that attention's rank, so another session's strip can sort on it.
pub const ATTENTION_RANK_OPTION: &str = "@ae_attn_rank";

/// SESSION — the `#[…]` style the session segment is drawn in.
pub const ATTENTION_STYLE_OPTION: &str = "@ae_attn_style";

/// SESSION — the whole fleet strip, ranges included.
pub const FLEET_STRIP_OPTION: &str = "@ae_fleet_strip";

/// SESSION — `on`/`off`, whether ae draws this session at all.
pub const LOOK_OPTION: &str = "@ae_look";

/// SESSION — `on`/`off`, whether anything animates.
pub const MOTION_OPTION: &str = "@ae_motion";

/// SESSION — the session goal, as the bar shows it.
pub const GOAL_OPTION: &str = "@ae_goal_status";

/// SESSION — the shortened work paths, as the bar shows them.
pub const PATHS_OPTION: &str = "@ae_paths";

/// SESSION — the ae core this session's watchdog runs on, as `ae <version>`.
///
/// Published by the WATCHDOG rather than the launch: an upgrade restarts the
/// watchdog on the new core, so the bar answers "did the upgrade reach this
/// session" without a relaunch.
pub const VERSION_OPTION: &str = "@ae_version";

/// SESSION — the look the LAYOUT was written for.
///
/// The watchdog compares it with the look the session's own options now
/// declare, and rewrites the layout when they have drifted apart — which is how
/// flipping `@ae_palette` or `@ae_look` on a live session takes effect rather
/// than leaving half the bar in the old look.
pub const LOOK_STAMP_OPTION: &str = "@ae_look_stamp";

/// PANE — the agent's name AS DRAWN.
///
/// Separate from `@ae_agent`, which is the pane's identity and is matched
/// against the roster, the monitor's own names and the meta. That one must stay
/// verbatim; this one is the same name with everything the drawer would read as
/// a style taken out of it.
pub const AGENT_LABEL_OPTION: &str = "@ae_agent_label";

/// The name a pane's border DRAWS for `agent`.
///
/// One owner, because two would disagree: the launch writes it when the pane is
/// created and the watchdog rewrites it every cycle, and a monitor pane whose
/// two writers spelled it differently would flicker between them.
///
/// The monitor's own panes carry a leading underscore in their IDENTITY, which
/// is how every lookup tells them from an agent. That underscore is a marker,
/// not a name, so it does not reach the border.
///
/// ```
/// assert_eq!(ae::theme::agent_label("_events"), "events");
/// assert_eq!(ae::theme::agent_label("lead"), "lead");
/// assert_eq!(ae::theme::agent_label("evil#[bg=red]"), "evil[bg=red]");
/// ```
#[must_use]
pub fn agent_label(agent: &str) -> String {
    bar_text(agent.strip_prefix('_').unwrap_or(agent), LABEL_WIDTH)
}

/// PANE — the seat's profile, for the border title.
pub const PROFILE_OPTION: &str = "@ae_profile";

/// PANE — `<styled glyph> <reason>`, for the border title.
pub const PANE_STATE_OPTION: &str = "@ae_pane_state";

/// PANE — the accent that pane's ACTIVE border is drawn in.
pub const PANE_ACCENT_OPTION: &str = "@ae_pane_accent";

/// WINDOW — the stamp saying this window already carries the theme.
pub const WINDOW_STAMP_OPTION: &str = "@ae_theme";

/// The version of every FORMAT this module draws — the session lines, the
/// window entries, the pane borders, the menu styles and terminal titles. Bump it when any of them
/// changes shape: the version leads both stamps, so a session or window carrying
/// an older one is rewritten by the next watchdog cycle rather than left on the
/// layout an older core wrote.
pub const FORMAT_VERSION: &str = "3";

/// What [`WINDOW_STAMP_OPTION`] is set to: the LOOK the window was dressed in,
/// formats version first.
///
/// The look is part of the stamp because the stamp is what tells a later cycle
/// whether the window still matches. A constant would say "dressed" about a
/// window dressed in another palette, or dressed by a caller whose look read
/// had failed, and hide the mismatch forever.
///
/// ```
/// use ae::theme::{FORMAT_VERSION, Look, window_stamp};
/// assert_eq!(window_stamp(&Look::DEFAULT), format!("{FORMAT_VERSION}:darcula:on:on"));
/// ```
#[must_use]
pub fn window_stamp(look: &Look) -> String {
    look.stamp()
}

// ---------------------------------------------------------------------------
// styles
// ---------------------------------------------------------------------------

/// A `#[…]` directive from a space-separated attribute list.
fn style(attributes: &[&str]) -> String {
    format!("#[{}]", attributes.join(" "))
}

/// The style the session's own attention BADGE is drawn in — the accent as
/// ink on the bar's ground.
///
/// Not the reverse: measured against the palettes here, dark text on an accent
/// clears 4.25:1 at best, which is fine for a glyph and too thin for a word. So
/// the accent carries the glyph and every essential WORD stays text on a
/// neutral ground.
#[must_use]
pub fn attention_style(palette: &Palette, mark: Mark) -> String {
    style(&[
        &format!("fg={}", palette.accent(mark)),
        &format!("bg={}", palette.base),
        "bold",
    ])
}

/// The style a glyph alone is drawn in: the accent as INK on the bar's ground,
/// which is what a strip of many sessions needs.
#[must_use]
pub fn mark_style(palette: &Palette, mark: Mark) -> String {
    style(&[
        &format!("fg={}", palette.accent(mark)),
        &format!("bg={}", palette.base),
    ])
}

/// The style a menu TITLE is drawn in — the one bright word on that surface.
///
/// tmux has no menu-title style option, so this is emitted into the `-T`
/// argument itself, which `display-menu` format-expands.
#[must_use]
pub fn menu_title_style(palette: &Palette) -> String {
    style(&[&format!("fg={} bold", palette.title)])
}

// ---------------------------------------------------------------------------
// status-format[0] — the session's own line
// ---------------------------------------------------------------------------

/// The window entry inside `#{W:…}`, drawn in `body`'s style.
///
/// `#F` is gone on purpose: the `*` / `-` / `Z` flag block is replaced by the
/// window's own `@ae_window_status`, which is the marks of the agents in it.
fn window_entry(palette: &Palette, current: bool) -> String {
    let text = if current { palette.text } else { palette.dim };
    let weight = if current { "bold" } else { "nobold" };
    format!(
        "#[range=window|#{{window_index}} fg={text} bg={panel} {weight}] \
         #{{window_index}}:#{{window_name}}#{{?window_zoomed_flag,Z,}}\
         #{{?#{{@ae_window_status}}, #{{@ae_window_status}},}} \
         #[norange nobold fg={dim} bg={base}]",
        panel = palette.panel,
        dim = palette.dim,
        base = palette.base,
    )
}

/// The client width below which the status bar drops its path segment.
///
/// The path is the FIRST thing to go: the branch, the goal and the marks all
/// say something the reader cannot reconstruct, and the path is the one fact
/// their own shell prompt already carries.
const NARROW: u16 = 100;

/// `status-format[0]`: this session's attention mark, its windows, and the
/// right-hand facts.
///
/// The session's NAME is not here: the fleet strip on the line below names
/// every session and raises this one, and a word the reader has just seen one
/// row down is a word this row can spend on the windows. The PATH is not baked
/// in either: it rides [`PATHS_OPTION`] like every other fact, so this format
/// depends on the look alone and the watchdog can rewrite it without knowing
/// where the session lives.
#[must_use]
pub fn status_line_zero(palette: &Palette) -> String {
    let windows = format!(
        "#{{W:{},{}}}",
        window_entry(palette, false),
        window_entry(palette, true)
    );
    format!(
        // The session's attention glyph in the accent, as the badge that opens
        // the line, then straight into the windows.
        "#[align=left]#{{{ATTENTION_STYLE_OPTION}}} #{{{ATTENTION_GLYPH_OPTION}}} \
         #[nobold fg={dim} bg={base}]\
         {windows}\
         #[align=right nobold fg={dim} bg={base}] \
         #{{@ae_branch_status}}#{{{GOAL_OPTION}}}\
         #{{?#{{e|>=:#{{client_width}},{NARROW}}}, #{{{PATHS_OPTION}}},}} \
         #{{@ae_watchdog_status}} ",
        dim = palette.dim,
        base = palette.base,
    )
}

// ---------------------------------------------------------------------------
// status-format[1] — the fleet strip
// ---------------------------------------------------------------------------

/// `status-format[1]`: every ae session on this server, then this session's
/// own agents, then the core they run on — dim, at the far right, where a
/// reader looks once after an upgrade and never otherwise.
#[must_use]
pub fn status_line_one(palette: &Palette) -> String {
    format!(
        "#[align=left fg={dim} bg={base}] #{{{FLEET_STRIP_OPTION}}}\
         #[align=right fg={dim} bg={base}]#{{@ae_agents_status}}  \
         #{{{VERSION_OPTION}}} ",
        dim = palette.dim,
        base = palette.base,
    )
}

/// One session as the fleet strip carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetRow {
    /// The session name, unescaped.
    pub name: String,
    /// Its tmux `$<n>` id — what the click target resolves through.
    pub id: String,
    /// What it is saying.
    pub mark: Mark,
    /// Whether it is the session the strip is being drawn for.
    pub current: bool,
}

impl FleetRow {
    /// Whether this row is the fleet's fixed orchestrator anchor.
    fn pinned(&self) -> bool {
        self.name == crate::orchestrator::ORCHESTRATOR_SESSION
    }
}

/// The fleet strip: `<glyph> <name>` per session, most actionable first, each
/// one a click that switches this client to it.
///
/// The range is tmux's OWN `session` range, so the default root binding
/// (`MouseDown1Status` → `switch-client -t =`) already does the jump: ae adds no
/// key binding, which would be a server-global write on the user's key table.
/// A session on another tmux server cannot appear here at all — the strip is
/// built from one server's own listing.
#[must_use]
pub fn fleet_strip(look: &Look, rows: &[FleetRow]) -> String {
    let palette = &look.palette;
    let icons = look.icons;
    let mut ordered: Vec<&FleetRow> = rows.iter().collect();
    ordered.sort_by(|left, right| {
        // The orchestrator is the fleet's fixed point of reference: keep it
        // first regardless of attention, and never let overflow shed it.
        let left_pinned = left.pinned();
        let right_pinned = right.pinned();
        right_pinned
            .cmp(&left_pinned)
            .then_with(|| right.mark.rank().cmp(&left.mark.rank()))
            .then_with(|| left.name.cmp(&right.name))
    });
    // OVERFLOW: the strip sheds its calmest rows first, because a session that
    // wants nothing is the one the reader loses least by not seeing. The
    // current session is never shed — a strip that cannot show you where you
    // are is not a map.
    let hidden = ordered.len().saturating_sub(STRIP_ROWS);
    if hidden > 0 {
        if let Some(here) = ordered.iter().position(|row| row.current)
            && here >= STRIP_ROWS
        {
            ordered.swap(STRIP_ROWS - 1, here);
        }
        ordered.truncate(STRIP_ROWS);
    }
    let mut strip = ordered.iter().fold(String::new(), |mut out, row| {
        // NOT escaped: the strip is published as an option VALUE, which tmux
        // interpolates literally, and a session name is an allowlist that
        // admits no `#`.
        let name = &row.name;
        // The CURRENT session is the raised tab: the panel ground the current
        // window entry stands on, bright bold text, and a space of ground on
        // either side so the segment reads as a shape. It is the only place the
        // bar names the session you are in, so it has to be found at a glance.
        let (ground, text) = if row.current {
            (palette.panel, format!("fg={} bold", palette.text))
        } else {
            (palette.base, format!("fg={} nobold", palette.dim))
        };
        let lead = if row.current { " " } else { "" };
        let pinned = row.pinned() && row.mark != Mark::Dead;
        let glyph = if pinned {
            if icons { "◆" } else { "o" }
        } else {
            row.mark.glyph(icons)
        };
        let accent = if pinned { palette.dim } else { palette.accent(row.mark) };
        let _ = write!(
            out,
            "#[range=session|{id} fg={accent} bg={ground}]{lead}{glyph}#[{text} bg={ground}] {name}{lead}\
             #[norange nobold fg={dim} bg={base}] ",
            id = row.id,
            accent = accent,
            glyph = glyph,
            base = palette.base,
            dim = palette.dim,
        );
        out
    });
    if hidden > 0 {
        let _ = write!(
            strip,
            "#[fg={dim} bg={base}]+{hidden} ",
            dim = palette.dim,
            base = palette.base,
        );
    }
    strip
}

/// How many sessions the fleet strip draws before it starts counting instead.
const STRIP_ROWS: usize = 8;

// ---------------------------------------------------------------------------
// the pane border
// ---------------------------------------------------------------------------

/// `pane-border-format`: `<name> · <profile> · <glyph> <reason>`, each part
/// dropped when the option behind it is unset rather than drawn as a gap.
#[must_use]
pub fn pane_border_format() -> String {
    format!(
        " #{{{AGENT_LABEL_OPTION}}}\
         #{{?#{{e|>=:#{{pane_width}},{NARROW_PANE}}},\
         #{{?#{{{PROFILE_OPTION}}}, · #{{{PROFILE_OPTION}}},}},}}\
         #{{?#{{{PANE_STATE_OPTION}}}, · #{{{PANE_STATE_OPTION}}},}} "
    )
}

/// The pane width below which the border title drops the profile.
///
/// Second in the same order the bar sheds things: the name and the state are
/// what the title is FOR, and the profile is a fact the roster also carries.
const NARROW_PANE: u16 = 60;

/// `pane-active-border-style`: the ACTIVE pane's own accent when the watchdog
/// has published one, and the working accent before it has.
///
/// A style option is format-expanded by tmux — its own default reads
/// `#{?pane_in_mode,…}` — so the colour follows the pane the border belongs to
/// without ae writing a style per pane.
#[must_use]
pub fn pane_active_border_style(palette: &Palette) -> String {
    format!(
        "#{{?#{{{PANE_ACCENT_OPTION}}},fg=#{{{PANE_ACCENT_OPTION}}},fg={}}}",
        palette.accent(Mark::Working)
    )
}

/// What the watchdog publishes into [`PANE_STATE_OPTION`]: `glyph` in the
/// mark's accent, then the reason word.
///
/// The GLYPH is passed rather than derived: a moving pane shows a spinner frame
/// where its mark would be, and only the caller counting cycles knows which.
///
/// Nothing here is escaped, and nothing may need to be: this is an option
/// VALUE, which tmux interpolates literally, and `reason` is one of a closed
/// set of words this crate wrote.
#[must_use]
pub fn pane_state(palette: &Palette, mark: Mark, glyph: &str, reason: &str) -> String {
    format!(
        "{}{glyph}#[default] {reason}",
        style(&[&format!("fg={}", palette.accent(mark))]),
    )
}

/// Free text as an option VALUE can carry it.
///
/// An option value is interpolated literally — `##` stays two characters — but
/// the DRAWER still reads `#[…]` out of it, so a `#` in text a human typed
/// could restyle the bar. There is no escape that survives both, so the
/// character is dropped. Control bytes go with it, and the result is cut to
/// `width` so one long line cannot push the rest of the bar off the screen.
///
/// ```
/// assert_eq!(ae::theme::bar_text("ship #[bg=red]P4", 40), "ship [bg=red]P4");
/// assert_eq!(ae::theme::bar_text("a\tb", 40), "a b");
/// assert_eq!(ae::theme::bar_text("abcdef", 4), "abc…");
/// ```
#[must_use]
pub fn bar_text(raw: &str, width: usize) -> String {
    let clean: String = raw
        .chars()
        .filter(|c| *c != '#')
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let clean = clean.trim();
    if clean.chars().count() <= width {
        return clean.to_owned();
    }
    let mut out: String = clean.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// How much of the session goal the bar carries.
pub const GOAL_WIDTH: usize = 40;

/// How much of a profile name a pane border title carries.
pub const PROFILE_WIDTH: usize = 24;

/// How much of an agent name the agent strip carries.
pub const LABEL_WIDTH: usize = 24;

// ---------------------------------------------------------------------------
// the paths segment
// ---------------------------------------------------------------------------

/// How many trailing components of a path survive into the status bar.
const PATH_TAIL: usize = 2;

/// `path` as the status bar shows it: `$HOME` as `~`, and at most the last
/// [`PATH_TAIL`] components, so a deep worktree does not push the watch count
/// off the end of the bar.
///
/// ```
/// use ae::theme::short_path;
/// assert_eq!(short_path("/Users/x", "/Users/x/projects/ae"), "~/projects/ae");
/// assert_eq!(short_path("/Users/x", "/Users/x/a/b/c/ae"), "~/…/c/ae");
/// assert_eq!(short_path("/Users/x", "/Users/x/ae"), "~/ae");
/// assert_eq!(short_path("/Users/x", "/Users/x"), "~");
/// assert_eq!(short_path("/Users/x", "/opt/tools/ae"), "…/tools/ae");
/// assert_eq!(short_path("", "/opt"), "/opt");
/// ```
#[must_use]
pub fn short_path(home: &str, path: &str) -> String {
    // The prefix must end on a COMPONENT boundary: `/home/x` under a home of
    // `/h` is not `~/ome/x`, it is a different directory that happens to share
    // two bytes.
    let under_home = (!home.is_empty())
        .then(|| path.strip_prefix(home))
        .flatten()
        .filter(|rest| rest.is_empty() || rest.starts_with('/'));
    let (prefix, rest) = match under_home {
        Some(rest) => ("~", rest.trim_start_matches('/')),
        None => ("", path.trim_start_matches('/')),
    };
    let parts: Vec<&str> = rest.split('/').filter(|part| !part.is_empty()).collect();
    let shown: Vec<&str> = parts.iter().rev().take(PATH_TAIL).rev().copied().collect();
    let elided = parts.len() > shown.len();
    let mut out = String::new();
    if prefix.is_empty() {
        if !elided {
            out.push('/');
        }
    } else {
        out.push_str(prefix);
        if !shown.is_empty() || elided {
            out.push('/');
        }
    }
    if elided {
        out.push_str("…/");
    }
    out.push_str(&shown.join("/"));
    out
}

// ---------------------------------------------------------------------------
// the option sets ae writes
// ---------------------------------------------------------------------------

/// SESSION — which palette this session is drawn in.
pub const PALETTE_OPTION: &str = "@ae_palette";

/// The SESSION options one ae session carries: the layout, and the facts.
///
/// The unpublished facts are seeded rather than left unset so a session looks
/// right in the seconds before the watchdog's first cycle, and so it says what
/// it actually knows then: nothing.
#[must_use]
pub fn session_options(look: &Look, paths: &str) -> Vec<(String, String)> {
    let mut options = redress_options(look, paths);
    options.extend(seed_options(look));
    options
}

/// Everything a session that is ALREADY RUNNING may have rewritten: the layout
/// and the facts, and none of the verdicts.
#[must_use]
pub fn redress_options(look: &Look, paths: &str) -> Vec<(String, String)> {
    let mut options = layout_options(look);
    options.extend(fact_options(look, paths));
    options
}

/// The LAYOUT half: the two status lines and their ground.
///
/// Empty when the look is undrawn. `[workspace] theme = off` leaves `status`,
/// `status-style`, titles and both `status-format` indices exactly as the user's own
/// tmux configuration left them, and ae fills the `@ae_*` facts anyway — so a
/// hand-written `status-right` can carry them in the user's own layout.
#[must_use]
pub fn layout_options(look: &Look) -> Vec<(String, String)> {
    if !look.drawn {
        return Vec::new();
    }
    let palette = &look.palette;
    vec![
        ("status".to_owned(), "2".to_owned()),
        ("status-interval".to_owned(), "5".to_owned()),
        (
            "status-style".to_owned(),
            format!("bg={},fg={}", palette.base, palette.text),
        ),
        ("set-titles".to_owned(), "on".to_owned()),
        (
            "set-titles-string".to_owned(),
            "#{?#{@ae_attn_glyph},#{@ae_attn_glyph} ,}#{session_name}".to_owned(),
        ),
        ("status-format[0]".to_owned(), status_line_zero(palette)),
        ("status-format[1]".to_owned(), status_line_one(palette)),
    ]
}

/// Every option name [`layout_options`] can write, for the retraction that
/// turns the look off.
///
/// UNSET, not written empty: a session option that is not set falls back to the
/// global one, which is the user's own — so this is how ae actually gives the
/// status line back rather than replacing it with a blank.
pub const LAYOUT_OPTIONS: [&str; 7] = [
    "status",
    "status-interval",
    "status-style",
    "set-titles",
    "set-titles-string",
    "status-format[0]",
    "status-format[1]",
];

/// The FACT half: what this session is drawn in, and where it lives.
///
/// Written whether or not ae draws the session, and safe to write AGAIN on a
/// live one — nothing here is a verdict, so a rename can re-render a session
/// without telling the bar anything it does not know.
#[must_use]
pub fn fact_options(look: &Look, paths: &str) -> Vec<(String, String)> {
    vec![
        (PALETTE_OPTION.to_owned(), look.palette.name.to_owned()),
        (ICONS_OPTION.to_owned(), switch(look.icons)),
        (LOOK_OPTION.to_owned(), switch(look.drawn)),
        (MOTION_OPTION.to_owned(), switch(look.motion)),
        (LOOK_STAMP_OPTION.to_owned(), look.stamp()),
        (PATHS_OPTION.to_owned(), bar_text(paths, PATHS_WIDTH)),
    ]
}

/// The attention SEED: what a session says before its watchdog's first cycle.
///
/// A LAUNCH writes it and nothing else may. These three are the watchdog's to
/// publish, and every other session on the server sorts its fleet strip on
/// them, so re-seeding a live session would make it claim to be stale — on its
/// own bar and on everyone else's — until the next cycle corrected it.
#[must_use]
pub fn seed_options(look: &Look) -> Vec<(String, String)> {
    vec![
        (
            ATTENTION_GLYPH_OPTION.to_owned(),
            Mark::Stale.glyph(look.icons).to_owned(),
        ),
        (
            ATTENTION_RANK_OPTION.to_owned(),
            Mark::Stale.rank().to_string(),
        ),
        (
            ATTENTION_STYLE_OPTION.to_owned(),
            attention_style(&look.palette, Mark::Stale),
        ),
    ]
}

/// How much of the work path the bar carries.
pub const PATHS_WIDTH: usize = 48;

/// How a knob is written back into the option that carries it.
fn switch(on: bool) -> String {
    if on { "on" } else { "off" }.to_owned()
}

/// Every option name [`window_options`] writes — the window half of
/// [`LAYOUT_OPTIONS`], for the retraction that turns the look off.
#[must_use]
pub fn window_option_names() -> Vec<String> {
    let mut names: Vec<String> = window_options(&Look::DEFAULT)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    names.push(WINDOW_STAMP_OPTION.to_owned());
    names
}

/// The WINDOW options every window of an ae session carries.
///
/// Window options, so each window is stamped individually: measured on tmux
/// 3.7b, `set-option -t <session>` on a window option resolves to that
/// session's CURRENT window and silently leaves the others on the global table.
///
/// `pane-border-lines` is `heavy`: measured on tmux 3.7b, `double` draws a
/// two-line rule that reads as a second status bar between panes, while `heavy`
/// is one weight up from the default and still one row tall.
#[must_use]
pub fn window_options(look: &Look) -> Vec<(String, String)> {
    let palette = &look.palette;
    vec![
        ("pane-border-status".to_owned(), "top".to_owned()),
        ("pane-border-format".to_owned(), pane_border_format()),
        ("pane-border-lines".to_owned(), "heavy".to_owned()),
        (
            "pane-border-style".to_owned(),
            format!("fg={}", palette.border),
        ),
        (
            "pane-active-border-style".to_owned(),
            pane_active_border_style(palette),
        ),
        // The menu and popup ae's own picker is drawn in — window options too.
        // A tmux that does not know one of these names simply refuses that
        // write, and the caller discards the result: an option ae cannot set is
        // a plainer surface, never a failed launch.
        (
            "menu-style".to_owned(),
            format!("bg={},fg={}", palette.panel, palette.text),
        ),
        (
            "menu-selected-style".to_owned(),
            format!("bg={},fg={},bold", palette.selected, palette.selected_ink),
        ),
        (
            "menu-border-style".to_owned(),
            format!("fg={}", palette.border),
        ),
        ("menu-border-lines".to_owned(), "rounded".to_owned()),
        (
            "popup-style".to_owned(),
            format!("bg={},fg={}", palette.panel, palette.text),
        ),
        (
            "popup-border-style".to_owned(),
            format!("fg={}", palette.border),
        ),
        ("popup-border-lines".to_owned(), "rounded".to_owned()),
    ]
}

#[cfg(test)]
mod tests {
    use super::{
        FleetRow, ICONS_OPTION, Look, Mark, PALETTE_OPTION, Palette, WINDOW_STAMP_OPTION,
        attention_style, fleet_strip, icons_wanted, mark_style, pane_border_format, pane_state,
        session_options, short_path, spinner, status_line_one, status_line_zero, window_options,
    };
    use crate::attention::Reason;

    /// Every palette a session can be drawn in.
    const PALETTES: [Palette; 3] = [Palette::DARCULA, Palette::NEUTRAL, Palette::WARM];

    /// Line two ends in the core version, so a reader can tell whether an
    /// upgrade reached this session without leaving the bar.
    #[test]
    fn line_one_carries_the_core_version_at_its_right_end() {
        for palette in PALETTES {
            let line = status_line_one(&palette);
            let reference = format!("#{{{}}}", super::VERSION_OPTION);
            assert!(line.ends_with(&format!("{reference} ")), "{line}");
        }
    }

    /// Every format literal this module hands tmux, for the guards below.
    fn every_format() -> Vec<String> {
        let mut out = vec![
            status_line_zero(&Palette::NEUTRAL),
            status_line_one(&Palette::NEUTRAL),
            pane_border_format(),
            super::pane_active_border_style(&Palette::NEUTRAL),
            fleet_strip(
                &Look::DEFAULT,
                &[FleetRow {
                    name: "demo".to_owned(),
                    id: "$0".to_owned(),
                    mark: Mark::NeedsYou,
                    current: true,
                }],
            ),
            pane_state(&Palette::NEUTRAL, Mark::Stale, "◌", "stale"),
        ];
        out.extend(
            session_options(&Look::DEFAULT, "~/p/ae")
                .into_iter()
                .map(|(_, value)| value),
        );
        out.extend(
            window_options(&Look::DEFAULT)
                .into_iter()
                .map(|(_, value)| value),
        );
        out
    }

    /// `#()` runs a SHELL COMMAND on every status redraw. Nothing ae writes may
    /// contain one, at any scope, ever — the status line is not a place to put
    /// a subprocess, and a format assembled from values is not a place to make
    /// one reachable.
    #[test]
    fn no_format_this_module_writes_can_run_a_command() {
        for format in every_format() {
            assert!(!format.contains("#("), "{format:?} carries a #() sink");
        }
    }

    /// The same rule the tmux module's formats live under: tmux 3.4 escapes
    /// control characters and 3.5+ does not, so a format that carries one is
    /// read back differently on different servers.
    #[test]
    fn no_format_this_module_writes_carries_a_control_character() {
        for format in every_format() {
            assert!(
                !format.chars().any(char::is_control),
                "{format:?} carries a control character"
            );
        }
    }

    /// Every colour is a six-digit hex, so tmux reads it as RGB and
    /// down-converts it itself where the terminal has no true colour — and so
    /// no value can begin with one of tmux's own `#<letter>` format aliases.
    #[test]
    fn every_palette_colour_is_a_six_digit_hex() {
        for palette in PALETTES {
            let mut colours = vec![
                palette.base,
                palette.panel,
                palette.text,
                palette.dim,
                palette.ink,
                palette.border,
                palette.selected,
                palette.selected_ink,
                palette.title,
            ];
            colours.extend(Mark::BY_URGENCY.map(|mark| palette.accent(mark)));
            for colour in colours {
                let digits = colour.strip_prefix('#').unwrap_or_default();
                assert_eq!(digits.len(), 6, "{colour}");
                assert!(digits.chars().all(|c| c.is_ascii_hexdigit()), "{colour}");
            }
        }
    }

    #[test]
    fn the_default_palette_is_darcula_and_carries_the_ide_tokens() {
        assert_eq!(Palette::named(""), Palette::DARCULA);
        assert_eq!(Palette::named("chartreuse"), Palette::DARCULA);
        assert_eq!(Palette::named("darcula"), Palette::DARCULA);
        assert_eq!(Palette::named("a"), Palette::NEUTRAL);
        assert_eq!(Palette::named("b"), Palette::WARM);
        // The tokens are the IDE's own, so they are pinned by value: a drift
        // here is a different theme wearing the name.
        let darcula = Palette::DARCULA;
        assert_eq!(darcula.base, "#313335");
        assert_eq!(darcula.panel, "#3C3F41");
        assert_eq!(darcula.text, "#A9B7C6");
        assert_eq!(darcula.dim, "#808080");
        assert_eq!(darcula.ink, "#2B2B2B");
        assert_eq!(darcula.border, "#555555");
        assert_eq!(darcula.selected, "#214283");
        assert_eq!(darcula.title, "#FFC66D");
        assert_eq!(darcula.accent(Mark::NeedsYou), "#CC7832");
        assert_eq!(darcula.accent(Mark::Working), "#6897BB");
        assert_eq!(darcula.accent(Mark::Done), "#6A8759");
        assert_eq!(darcula.accent(Mark::Stale), "#9876AA");
        assert_eq!(darcula.accent(Mark::Idle), darcula.dim);
    }

    #[test]
    fn every_palette_names_itself_and_answers_to_that_name() {
        for palette in PALETTES {
            assert_eq!(Palette::named(palette.name), palette, "{}", palette.name);
        }
    }

    #[test]
    fn the_menu_title_is_the_only_thing_drawn_in_the_title_colour() {
        for palette in PALETTES {
            let title = super::menu_title_style(&palette);
            assert_eq!(title, format!("#[fg={} bold]", palette.title));
        }
        // Nothing else on a Darcula surface reaches for it: the title is one
        // bright word, and a second would make it ordinary. The two ae variants
        // spend their amber twice, on the title and on the needs-you accent.
        let darcula = Palette::DARCULA;
        for drawn in [
            attention_style(&darcula, Mark::NeedsYou),
            mark_style(&darcula, Mark::NeedsYou),
            super::status_line_zero(&darcula),
            super::status_line_one(&darcula),
        ] {
            assert!(!drawn.contains(darcula.title), "{drawn}");
        }
    }

    /// The two palettes differ in their NEUTRALS and agree on their accents: a
    /// variant changes the room's light, not what the four states mean.
    #[test]
    fn the_two_ae_variants_differ_only_in_their_neutrals() {
        for mark in Mark::BY_URGENCY {
            if mark == Mark::Idle {
                continue; // idle IS the palette's dim, so it moves with it
            }
            assert_eq!(
                Palette::NEUTRAL.accent(mark),
                Palette::WARM.accent(mark),
                "{mark:?}"
            );
        }
        assert_ne!(Palette::NEUTRAL.base, Palette::WARM.base);
        assert_ne!(Palette::NEUTRAL.dim, Palette::WARM.dim);
        assert_eq!(Palette::NEUTRAL.accent(Mark::Idle), Palette::NEUTRAL.dim);
    }

    /// Every reason the attention module publishes maps onto a mark. `dead` and
    /// `stale` keep their own; the rest are a human's turn.
    #[test]
    fn every_attention_reason_has_a_mark() {
        for reason in Reason::BY_SEVERITY {
            let expected = match reason {
                Reason::Dead => Mark::Dead,
                Reason::Stale => Mark::Stale,
                _ => Mark::NeedsYou,
            };
            assert_eq!(Mark::for_reason(reason), expected, "{reason}");
        }
        // A gone process outranks everything, and reads as neither a finish nor
        // a polite wait.
        assert!(Mark::Dead.rank() > Mark::NeedsYou.rank());
        assert_ne!(Mark::Dead.glyph(true), Mark::Done.glyph(true));
        assert_ne!(Mark::Dead.glyph(false), Mark::Done.glyph(false));
    }

    /// The strip is ATTENTION-SORTED, then by name — the same order on every
    /// session's copy of it, whatever order tmux listed the sessions in.
    #[test]
    fn the_fleet_strip_is_attention_sorted_then_alphabetical() {
        let row = |name: &str, id: &str, mark| FleetRow {
            name: name.to_owned(),
            id: id.to_owned(),
            mark,
            current: false,
        };
        let strip = fleet_strip(
            &Look::DEFAULT,
            &[
                row("zeta", "$3", Mark::Done),
                row("alpha", "$1", Mark::Working),
                row("beta", "$2", Mark::NeedsYou),
                row("gamma", "$4", Mark::Working),
            ],
        );
        let order: Vec<&str> = ["beta", "alpha", "gamma", "zeta"]
            .into_iter()
            .filter(|name| strip.contains(name))
            .collect();
        assert_eq!(order.len(), 4, "{strip}");
        let at = |name: &str| strip.find(name).unwrap_or(usize::MAX);
        assert!(at("beta") < at("alpha"), "needs-you first: {strip}");
        assert!(at("alpha") < at("gamma"), "then by name: {strip}");
        assert!(at("gamma") < at("zeta"), "working before done: {strip}");
    }

    /// Each row is a tmux SESSION range, which is what makes it clickable with
    /// no key binding of ae's own: tmux's default `MouseDown1Status` is
    /// `switch-client -t =`, and `=` resolves through the range.
    #[test]
    fn every_fleet_row_is_a_session_range_that_closes_itself() {
        let strip = fleet_strip(
            &Look::DEFAULT,
            &[FleetRow {
                name: "demo".to_owned(),
                id: "$7".to_owned(),
                mark: Mark::Working,
                current: false,
            }],
        );
        assert!(strip.contains("#[range=session|$7 "), "{strip}");
        assert_eq!(strip.matches("#[range=session|").count(), 1, "{strip}");
        assert_eq!(
            strip.matches("norange").count(),
            1,
            "an unclosed range bleeds into the next row: {strip}"
        );
    }

    #[test]
    fn the_orchestrator_is_a_pinned_dim_fleet_anchor() {
        let row = |name: &str, mark| FleetRow {
            name: name.to_owned(),
            id: format!("${}", name.len()),
            mark,
            current: false,
        };
        let strip = fleet_strip(
            &Look::DEFAULT,
            &[
                row("zeta", Mark::Done),
                row("orchestrator", Mark::Working),
                row("alpha", Mark::NeedsYou),
            ],
        );
        assert!(
            strip.find("orchestrator").unwrap_or(usize::MAX)
                < strip.find("alpha").unwrap_or(usize::MAX),
            "the anchor stays first: {strip}"
        );
        assert_eq!(strip.matches("◆").count(), 1, "{strip}");
        assert!(strip.contains("fg=#808080"), "the pin is dim: {strip}");

        let ascii = fleet_strip(
            &Look {
                icons: false,
                ..Look::DEFAULT
            },
            &[row("orchestrator", Mark::Working)],
        );
        assert!(ascii.contains("]o#["), "ASCII pin: {ascii}");
        assert!(
            !ascii.contains("x#["),
            "the softer attention glyph is hidden: {ascii}"
        );

        let dead = fleet_strip(
            &Look {
                icons: false,
                ..Look::DEFAULT
            },
            &[row("orchestrator", Mark::Dead)],
        );
        assert!(dead.contains("]x#["), "dead keeps its alarm glyph: {dead}");
        assert!(
            dead.contains("fg=#FF6B68"),
            "dead keeps its alarm accent: {dead}"
        );
    }

    #[test]
    fn the_pinned_orchestrator_survives_strip_overflow() {
        let rows: Vec<FleetRow> = (0..super::STRIP_ROWS + 3)
            .map(|index| FleetRow {
                name: if index == super::STRIP_ROWS + 2 {
                    "orchestrator".to_owned()
                } else {
                    format!("session-{index}")
                },
                id: format!("${index}"),
                mark: Mark::Idle,
                current: false,
            })
            .collect();
        let strip = fleet_strip(&Look::DEFAULT, &rows);
        assert!(
            strip.contains("orchestrator"),
            "pinned row was shed: {strip}"
        );
        assert!(
            strip.contains("+3"),
            "overflow count remains exact: {strip}"
        );
    }

    #[test]
    fn terminal_titles_are_part_of_the_drawn_layout() {
        let options = super::layout_options(&Look::DEFAULT);
        assert_eq!(super::FORMAT_VERSION, "3");
        assert_eq!(
            options
                .iter()
                .find(|(name, _)| name == "set-titles")
                .map(|(_, value)| value.as_str()),
            Some("on")
        );
        assert_eq!(
            options
                .iter()
                .find(|(name, _)| name == "set-titles-string")
                .map(|(_, value)| value.as_str()),
            Some("#{?#{@ae_attn_glyph},#{@ae_attn_glyph} ,}#{session_name}")
        );
    }

    /// The session half is SESSION-scoped and the rest is WINDOW-scoped: tmux
    /// keeps the pane borders, the window entries and the menu styles in the
    /// window table, where a `set -t <session>` reaches only the current one.
    #[test]
    fn the_two_option_sets_do_not_overlap() {
        let session: Vec<String> = session_options(&Look::DEFAULT, "/w")
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        let window: Vec<String> = window_options(&Look::DEFAULT)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        for name in &session {
            assert!(!window.contains(name), "{name} is written at both scopes");
        }
        assert!(session.iter().any(|name| name == "status-format[0]"));
        assert!(session.iter().any(|name| name == PALETTE_OPTION));
        assert!(session.iter().any(|name| name == ICONS_OPTION));
        assert!(window.iter().any(|name| name == "pane-border-status"));
        assert!(window.iter().any(|name| name == "menu-style"));
        // The STAMP is not one of them: it is the claim that all of them
        // landed, so its writer adds it after the fact and its retraction list
        // carries it.
        assert!(!window.iter().any(|name| name == WINDOW_STAMP_OPTION));
        assert!(
            super::window_option_names()
                .iter()
                .any(|name| name == WINDOW_STAMP_OPTION),
            "the retraction still takes the stamp off"
        );
    }

    /// A session says NOTHING it has not been told: before the watchdog's first
    /// cycle its attention seed is the stale mark, never a working or done one.
    #[test]
    fn the_unpublished_attention_seed_is_stale_and_never_a_verdict() {
        let seeded = session_options(&Look::DEFAULT, "/w");
        let value = |name: &str| {
            seeded
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
                .unwrap_or_default()
        };
        assert_eq!(
            value(super::ATTENTION_GLYPH_OPTION),
            Mark::Stale.glyph(true)
        );
        assert_eq!(
            value(super::ATTENTION_RANK_OPTION),
            Mark::Stale.rank().to_string()
        );
        assert_eq!(
            value(super::ATTENTION_STYLE_OPTION),
            attention_style(&Look::DEFAULT.palette, Mark::Stale)
        );
        assert_eq!(value(ICONS_OPTION), "on");
        // And the ASCII session says so too.
        let ascii = session_options(
            &Look {
                icons: false,
                ..Look::DEFAULT
            },
            "/w",
        );
        assert!(
            ascii
                .iter()
                .any(|(key, value)| key == ICONS_OPTION && value == "off")
        );
    }

    /// `[workspace] theme = off` writes the FACTS and none of the layout: the
    /// user's own status line survives inside an ae session, and can read every
    /// `@ae_*` value ae publishes.
    #[test]
    fn a_session_with_the_theme_off_keeps_the_users_own_status_line() {
        let off = Look {
            drawn: false,
            ..Look::DEFAULT
        };
        let session = session_options(&off, "/w");
        let names: Vec<&str> = session.iter().map(|(name, _)| name.as_str()).collect();
        for layout in [
            "status",
            "status-interval",
            "status-style",
            "status-format[0]",
            "status-format[1]",
        ] {
            assert!(!names.contains(&layout), "{layout} was written anyway");
        }
        for fact in [
            super::LOOK_OPTION,
            super::PALETTE_OPTION,
            ICONS_OPTION,
            super::ATTENTION_GLYPH_OPTION,
            super::ATTENTION_RANK_OPTION,
        ] {
            assert!(names.contains(&fact), "{fact} is missing");
        }
        assert!(
            session
                .iter()
                .any(|(name, value)| name == super::LOOK_OPTION && value == "off"),
            "the session must say that it is undrawn, so the watchdog agrees"
        );
    }

    /// Motion is a knob, and it moves NOTHING but the working glyph.
    #[test]
    fn the_spinner_is_subordinate_to_the_state_and_to_the_motion_knob() {
        let still = Look {
            motion: false,
            ..Look::DEFAULT
        };
        assert_eq!(
            still.glyph(Mark::Working, true, 3),
            Mark::Working.glyph(true),
            "motion off freezes the working glyph on its mark"
        );
        assert_eq!(
            Look::DEFAULT.glyph(Mark::Working, true, 3),
            spinner(3, true),
            "motion on spins a working pane"
        );
        for mark in Mark::BY_URGENCY {
            if mark == Mark::Working {
                continue;
            }
            assert_eq!(
                Look::DEFAULT.glyph(mark, true, 3),
                mark.glyph(true),
                "{mark:?} keeps saying what it is while it prints"
            );
        }
        assert_eq!(
            Look::DEFAULT.glyph(Mark::Working, false, 3),
            Mark::Working.glyph(true),
            "a pane that did not move does not spin"
        );
    }

    /// Free text a human typed never reaches the drawer as a style.
    ///
    /// The two sinks that carry it — the session goal and a pane's profile —
    /// both come off a hand-editable file and both land in an option VALUE,
    /// which the drawer reads `#[…]` out of. There is no escape that survives
    /// that, so the character is dropped.
    #[test]
    fn no_free_text_sink_can_carry_a_style_directive() {
        for width in [super::GOAL_WIDTH, super::PROFILE_WIDTH] {
            let drawn = super::bar_text("#[bg=red]ship it#[default]", width);
            assert!(!drawn.contains('#'), "{drawn}");
            assert!(
                drawn.contains("ship it"),
                "the text itself survives: {drawn}"
            );
        }
        // And the cut is by CHARACTER, so a multi-byte name cannot be split
        // mid-character into a broken cell.
        assert_eq!(super::bar_text("übermäßig lang", 5), "über…");
        assert_eq!(super::bar_text("  padded  ", 40), "padded");
    }

    /// The drawn name has ONE owner, and it is not the identity.
    #[test]
    fn the_drawn_name_drops_the_marker_and_every_directive() {
        // The monitor's panes: the underscore is how a lookup tells them from
        // an agent, and it is not part of what a border says.
        assert_eq!(super::agent_label("_events"), "events");
        assert_eq!(super::agent_label("_watchdog"), "watchdog");
        // An ordinary seat is itself.
        assert_eq!(super::agent_label("colead"), "colead");
        // And a name off a hand-edited meta cannot carry a style.
        let hostile = super::agent_label("evil#[bg=red]");
        assert!(!hostile.contains('#'), "{hostile}");
        assert!(hostile.contains("evil"), "{hostile}");
        // The border reads the LABEL and never the identity.
        let format = super::pane_border_format();
        assert!(format.contains(super::AGENT_LABEL_OPTION), "{format}");
        assert!(!format.contains("#{@ae_agent}"), "{format}");
    }

    /// A rank published by one session's watchdog is read back as the same mark
    /// by every other session's strip — the whole reason the rank exists.
    #[test]
    fn a_published_rank_round_trips_through_the_strip() {
        for mark in Mark::BY_URGENCY {
            assert_eq!(Mark::from_rank(&mark.rank().to_string()), mark, "{mark:?}");
        }
        // A session with nothing published, or something ae cannot read, is
        // idle — never a verdict ae does not have.
        assert_eq!(Mark::from_rank(""), Mark::Idle);
        assert_eq!(Mark::from_rank("nonsense"), Mark::Idle);
        assert_eq!(Mark::from_rank("99"), Mark::Idle);
    }

    /// The window stamp is a VALUE, not a presence: a window dressed by an
    /// older option set — or in another LOOK — must be restamped, not skipped.
    #[test]
    fn the_window_stamp_names_the_look_the_window_was_dressed_in() {
        let stamped = super::window_stamp;
        assert!(!stamped(&Look::DEFAULT).is_empty(), "never set empty");
        // A window dressed in another palette does NOT read as dressed, which
        // is what makes a palette change reach windows that already exist.
        let warm = Look {
            palette: Palette::WARM,
            ..Look::DEFAULT
        };
        assert_ne!(stamped(&warm), stamped(&Look::DEFAULT));
        // The formats have their own version ahead of the look, so a change to
        // the FORMATS restamps every window even when the look has not moved.
        assert!(
            stamped(&Look::DEFAULT).starts_with(super::FORMAT_VERSION),
            "{}",
            stamped(&Look::DEFAULT)
        );
    }

    #[test]
    fn the_ascii_fallback_is_off_and_its_spellings() {
        for value in ["off", "OFF", "false", "no", "0", "ascii", " off "] {
            assert!(!icons_wanted(value), "{value}");
        }
        for value in ["", "on", "yes", "1", "true", "anything"] {
            assert!(icons_wanted(value), "{value}");
        }
    }

    #[test]
    fn the_spinner_cycles_and_never_leaves_its_frames() {
        let frames: Vec<&str> = (0..24).map(|tick| spinner(tick, true)).collect();
        assert_eq!(frames[0], frames[10], "ten frames, then it repeats");
        assert!(frames.iter().all(|frame| frame.chars().count() == 1));
        let ascii: Vec<&str> = (0..8).map(|tick| spinner(tick, false)).collect();
        assert_eq!(ascii[0], ascii[4], "four ASCII frames");
        assert!(ascii.iter().all(|frame| frame.is_ascii()));
    }

    #[test]
    fn a_path_under_home_is_drawn_against_a_tilde() {
        assert_eq!(short_path("/h", "/h/a/b/c/d"), "~/…/c/d");
        assert_eq!(short_path("/h", "/h"), "~");
        // A path OUTSIDE home keeps its own shape rather than borrowing one.
        assert_eq!(short_path("/h", "/var/tmp"), "/var/tmp");
        assert_eq!(short_path("/h", "/a/b/c"), "…/b/c");
        // A prefix that is not a component boundary is not a home match: this
        // is an absolute path outside home, drawn whole because it is short.
        assert_eq!(short_path("/h", "/home/x"), "/home/x");
        assert_eq!(short_path("/h", "/home/x/y/z"), "…/y/z");
    }
}
