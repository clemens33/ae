//! The console lane as text, and the memory a follow needs so each tick prints
//! only what is new.
//!
//! One code path serves the one-shot and every follow tick: [`Printed::step`]
//! takes the WHOLE lane, prints what it has not printed yet, and says when a
//! card or prompt it printed no longer stands. Every field — actor, target,
//! reason, coverage, tag, body — reaches the terminal through the board's
//! terminal renderer, applied once over the finished text.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use super::input::{Effect, Size};
use super::lane::{Item, Kind, Lane, Seat};
use super::needs::{Cause, Row, Section, Source, Verdict};
use super::wrap::{clip, wrap};
use crate::board::{clock_text, terminal_text};
use crate::theme::{Look, Palette};
use crate::time::Timestamp;

/// Lane text a [`Style`] has already neutralised and dressed: the one text a
/// paint writes as it is. Only [`Printed::lane`] makes one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Styled(String);

impl Styled {
    /// The text, escapes included.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Who a row speaks for, which fixes the hue it wears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Voice {
    Human,
    Lead,
    Colead,
    System,
}

/// The viewer's clock zone, as tmux spelled it (`+0200`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Zone {
    label: String,
    secs: i64,
}

impl Zone {
    /// `[+-]HHMM` with a real hour and minute, else nothing: the answer is
    /// printed, so a malformed one is never kept.
    fn parse(answer: &str) -> Option<Self> {
        let digits = answer.strip_prefix(['+', '-'])?;
        if digits.len() != 4 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let hours = digits.get(..2)?.parse::<i64>().ok()?;
        let minutes = digits.get(2..)?.parse::<i64>().ok()?;
        let sign = if answer.starts_with('-') { -1 } else { 1 };
        (hours < 24 && minutes < 60).then(|| Self {
            label: answer.to_owned(),
            secs: sign * (hours * 3600 + minutes * 60),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Dress {
    palette: Palette,
    glyph: &'static str,
    zone: Option<Zone>,
    main: String,
}

/// How a console draws what it prints: [`Style::PLAIN`] is today's bytes, a
/// dressed style wears its session's palette. This type is the ONE place an
/// escape sequence is written, and it neutralises every field before it wraps
/// one, so no record can reach the terminal inside an escape ae did not write.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Style(Option<Dress>);

impl Style {
    /// No colour, no bar, UTC: the bytes of a pipe or a file.
    pub const PLAIN: Self = Self(None);

    /// Whether a console on a terminal (`tty`) drawn in `look` is dressed at
    /// all. A look nobody could read counts as the default one.
    #[must_use]
    pub fn wanted(tty: bool, look: Option<Look>) -> bool {
        tty && look.is_none_or(|look| look.drawn)
    }

    /// The style for a console on a terminal or not. `zone` is the raw answer
    /// tmux gave for the viewer's zone, kept only when it is `[+-]HHMM`; `main`
    /// is the lead seat's name, whose rows wear the lead hue.
    #[must_use]
    pub fn resolve(tty: bool, look: Option<Look>, zone: Option<&str>, main: &str) -> Self {
        if !Self::wanted(tty, look) {
            return Self::PLAIN;
        }
        let look = look.unwrap_or(Look::DEFAULT);
        Self(Some(Dress {
            palette: look.palette,
            glyph: if look.icons { "▌" } else { "|" },
            zone: zone.and_then(Zone::parse),
            main: main.to_owned(),
        }))
    }

    /// `micros` on the viewer's clock.
    #[must_use]
    pub fn shift(&self, micros: i64) -> i64 {
        let zone = self.0.as_ref().and_then(|dress| dress.zone.as_ref());
        micros.saturating_add(zone.map_or(0, |zone| zone.secs).saturating_mul(1_000_000))
    }

    /// The day divider, with its own newline.
    #[must_use]
    pub fn day(&self, day: &str) -> String {
        let zone = self.0.as_ref().and_then(|dress| dress.zone.as_ref());
        let text = format!("# {day} {}\n", zone.map_or("UTC", |zone| &zone.label));
        self.paint(|palette| palette.dim, false, &text)
    }

    /// A row's header, with its own newline.
    #[must_use]
    pub fn head(&self, kind: &Kind, time: &str) -> String {
        let Some(dress) = &self.0 else {
            return terminal_text(&format!("## {time} {}\n", tag(kind)));
        };
        let voice = dress.voice(kind);
        let bold = matches!(
            kind,
            Kind::Asked { .. } | Kind::NotDelivered { .. } | Kind::Closed { .. }
        );
        let (stamp, words) = (self.paint(|p| p.dim, false, time), tag(kind));
        let words = self.paint(|p| Dress::hue(p, voice), bold, &words);
        format!("{} {stamp} {words}\n", self.bar(kind))
    }

    /// [`Style::head`] for a pane `width` cells wide: every visual row keeps the bar,
    /// a dressed style wrapping the header like a body row.
    fn head_in(&self, kind: &Kind, time: &str, width: Option<usize>) -> String {
        let (Some(dress), Some(width)) = (&self.0, width) else {
            return self.head(kind, time);
        };
        let (voice, mut stamp) = (dress.voice(kind), time.len());
        let bold = matches!(
            kind,
            Kind::Asked { .. } | Kind::NotDelivered { .. } | Kind::Closed { .. }
        );
        let text = terminal_text(&format!("{time} {}", tag(kind)));
        let mut out = String::new();
        for (gap, piece) in wrap(&text, width, 1) {
            let (dim, words) = piece
                .split_at_checked(stamp.min(piece.len()))
                .unwrap_or(("", &piece));
            stamp -= dim.len();
            let (sep, words) = words.split_at(words.len() - words.trim_start().len());
            let words = self.paint(|p| Dress::hue(p, voice), bold, words);
            let dim = self.paint(|p| p.dim, false, dim);
            let _ = writeln!(
                out,
                "{}{}{dim}{sep}{words}",
                self.bar(kind),
                " ".repeat(gap)
            );
        }
        out
    }

    /// [`Style::line`] for a pane `width` cells wide: every visual row keeps the bar.
    fn line_in(&self, kind: &Kind, text: &str, width: Option<usize>) -> String {
        let (Some(_), Some(width)) = (&self.0, width) else {
            return self.line(kind, text);
        };
        self.rows(kind, &terminal_text(text), width, str::to_owned)
    }

    /// [`Style::under`] for a pane `width` cells wide: every visual row keeps the bar.
    fn under_in(&self, kind: &Kind, line: &str, width: Option<usize>) -> String {
        let (Some(_), Some(width)) = (&self.0, width) else {
            return self.under(kind, line);
        };
        self.rows(kind, &terminal_text(line), width, |piece| {
            self.status_as(line, piece)
        })
    }

    /// `text` wrapped, each row behind the bar of `kind`, its piece dressed by `dress`.
    fn rows(
        &self,
        kind: &Kind,
        text: &str,
        width: usize,
        dress: impl Fn(&str) -> String,
    ) -> String {
        let mut out = String::new();
        for (gap, piece) in wrap(text, width, 2) {
            let _ = writeln!(
                out,
                "{}{}{}",
                self.bar(kind),
                " ".repeat(gap),
                dress(&piece)
            );
        }
        out
    }

    /// One body line of a row, with its own newline.
    #[must_use]
    pub fn line(&self, kind: &Kind, text: &str) -> String {
        format!("{}{}\n", self.margin(kind), terminal_text(text))
    }

    /// A status line printed under its row, with its own newline.
    #[must_use]
    pub fn under(&self, kind: &Kind, line: &str) -> String {
        format!("{}{}\n", self.margin(kind), self.status(line))
    }

    /// ae's own status line, coloured by what it says (`sent`, `uncertain`, `not
    /// delivered`, `refused`), without a newline.
    #[must_use]
    pub fn status(&self, line: &str) -> String {
        self.status_as(line, line)
    }

    /// `piece` of a status `line`, in the hue the whole line says.
    fn status_as(&self, line: &str, piece: &str) -> String {
        self.paint(|palette| Dress::tone(palette, line), false, piece)
    }

    /// A quiet notice, its trailing newlines kept outside the colour.
    #[must_use]
    pub fn dim(&self, text: &str) -> String {
        self.paint(|palette| palette.dim, false, text)
    }

    /// `text` in the hue of the human, as the input prompt wears it.
    #[must_use]
    pub fn human(&self, text: &str) -> String {
        self.paint(|palette| Dress::hue(palette, Voice::Human), false, text)
    }

    fn bar(&self, kind: &Kind) -> String {
        let Some(dress) = &self.0 else {
            return String::new();
        };
        let voice = dress.voice(kind);
        self.paint(|palette| Dress::hue(palette, voice), false, dress.glyph)
    }

    /// What starts a body or status line: two spaces, behind the bar once dressed.
    fn margin(&self, kind: &Kind) -> String {
        format!("{}  ", self.bar(kind))
    }

    /// THE writer of escape sequences: neutralise `text`, then wrap it, the
    /// trailing newlines outside the wrap. 24-bit foreground, bold on request,
    /// one reset.
    fn paint(&self, hue: impl Fn(&Palette) -> &'static str, bold: bool, text: &str) -> String {
        let text = terminal_text(text);
        let Some(dress) = &self.0 else {
            return text;
        };
        let body = text.trim_end_matches('\n');
        if body.is_empty() {
            return text;
        }
        let hex = hue(&dress.palette);
        let channel = |at: usize| {
            let pair = hex.get(at..at + 2);
            pair.and_then(|pair| u8::from_str_radix(pair, 16).ok())
                .unwrap_or(0)
        };
        let (open, rgb) = (if bold { "\x1b[1m" } else { "" }, [1, 3, 5].map(channel));
        let tail = &text[body.len()..];
        format!(
            "{open}\x1b[38;2;{};{};{}m{body}\x1b[0m{tail}",
            rgb[0], rgb[1], rgb[2]
        )
    }
}

impl Dress {
    fn hue(palette: &Palette, voice: Voice) -> &'static str {
        match voice {
            Voice::Human => palette.title,
            Voice::Lead => palette.working,
            Voice::Colead => palette.stale,
            Voice::System => palette.dim,
        }
    }

    fn tone(palette: &Palette, line: &str) -> &'static str {
        // A word, not a prefix: `sentinel` is no outcome.
        let starts = |word: &str| {
            let rest = line.strip_prefix(word);
            rest.is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', ';', ':']))
        };
        if starts("sent") {
            palette.done
        } else if starts("uncertain") || starts("no record of") {
            palette.waiting_agent
        } else if starts("not delivered") || starts("refused") {
            palette.dead
        } else {
            palette.dim
        }
    }

    /// The voice a seat of this name speaks in: `ae` is the system, the main
    /// seat the lead, any other seat the colead.
    fn seat(&self, name: &str) -> Voice {
        match name {
            "ae" => Voice::System,
            name if name == self.main => Voice::Lead,
            _ => Voice::Colead,
        }
    }

    fn voice(&self, kind: &Kind) -> Voice {
        match kind {
            Kind::Pane { .. }
            | Kind::Inbound { .. }
            | Kind::Asked { .. }
            | Kind::NotDelivered { .. }
            | Kind::Closed { .. } => Voice::Human,
            Kind::Assistant { seat, .. }
            | Kind::Answer { seat, .. }
            | Kind::Card { seat }
            | Kind::NeedsYou { seat } => self.seat(seat),
            Kind::Reply { from, .. } | Kind::Unadmitted { from, .. } => self.seat(from),
            Kind::Said { who } => self.seat(who),
        }
    }
}

/// The first line of every console: what it is, and what it is not yet.
#[must_use]
pub fn header(session: &str, seats: &[Seat]) -> String {
    let names: Vec<&str> = seats.iter().map(|seat| seat.name.as_str()).collect();
    terminal_text(&format!(
        "chat: {session} — lead pair {} · preview of existing data: transcript turns are mirrors, chat-bridge replies are 600-character summaries, nothing here answers and only lines typed in the owner chat ask · scope: current conversations plus recorded predecessors, board limits; agent-to-agent turns are not shown\n",
        names.join(", ")
    ))
}

/// The tag a row wears after its time.
pub(super) fn tag(kind: &Kind) -> String {
    let prior = |generation: u8| match generation {
        0 => String::new(),
        n => format!(" · prior {n}"),
    };
    match kind {
        Kind::Pane { seat, generation } => {
            format!("{seat} pane (transcript){}", prior(*generation))
        }
        Kind::Assistant { seat, generation } => {
            format!("{seat} assistant (transcript){}", prior(*generation))
        }
        Kind::Inbound { from, to } => format!("{from} → {to}"),
        Kind::Reply { from, to } => format!("{from} → {to} · preview (600-char summary)"),
        Kind::Said { who } => format!("said {who}"),
        Kind::Card { seat } => format!("DECISION {seat} · waiting-user"),
        Kind::NeedsYou { seat } => format!("NEEDS YOU {seat} · needs you in its pane"),
        Kind::Asked { to, uncertain, .. } => match uncertain {
            true => format!("you → {to} · uncertain: check the {to} pane"),
            false => format!("you → {to}"),
        },
        Kind::NotDelivered { to, .. } => format!("you → {to} · not delivered"),
        Kind::Closed { .. } => "you closed an ask".to_owned(),
        Kind::Answer {
            seat,
            follow_up,
            late,
            gap,
            speaker,
            ..
        } => {
            let mut tag = format!("{seat} answers");
            if *follow_up > 0 {
                let _ = write!(tag, " · follow-up {follow_up}");
            }
            if *late {
                tag.push_str(" · late (closed)");
            }
            if let Some(profile) = speaker {
                let _ = write!(tag, " · speaker {seat} now {profile}");
            }
            if let Some(gap) = gap {
                let _ = write!(tag, " · {gap} · preview (600-char summary)");
            }
            tag
        }
        Kind::Unadmitted { from, why, .. } => {
            format!("{from} reply · not admitted: {why} · preview (600-char summary)")
        }
    }
}

/// The memory key of a console-thread row: its record's position in the read.
const CONSOLE_KEY: &str = "console record ";

/// What a console has already printed: rows by multiplicity (two records of
/// equal words in one second are two rows), the cards and prompts it printed
/// that may yet stop standing, the lane-derived gaps it named, and the day.
#[derive(Debug, Default)]
pub struct Printed {
    shown: BTreeMap<String, usize>,
    open: BTreeMap<String, String>,
    gaps: BTreeSet<String>,
    day: Option<String>,
    pending: Vec<Pending>,
    /// Submission ids survive a journal rebase; their outcomes print once.
    outcomes: BTreeSet<String>,
    style: Style,
    width: Option<usize>,
    /// The "needs you" section last printed from a settled read, by its rows.
    needs: Option<String>,
    /// The reason the section's current unsettled episode was named for.
    needs_gap: Option<String>,
    /// The wall clock of the last section read, epoch seconds: a gap past
    /// [`RESUME_GAP_SECS`] marks a host resume.
    needs_last: Option<i64>,
    /// The watchdog silence stays graced while the wall clock is before this.
    needs_grace_until: Option<i64>,
    /// The doubt classes the section last disclosed.
    needs_doubt: BTreeSet<&'static str>,
}

#[derive(Debug)]
struct Pending {
    id: String,
    line: String,
    passes: u8,
}

impl Printed {
    /// A console that draws what it prints in `style`; `default()` is plain.
    #[must_use]
    pub fn styled(style: Style) -> Self {
        Self {
            style,
            ..Self::default()
        }
    }

    /// How this console draws.
    #[must_use]
    pub fn style(&self) -> &Style {
        &self.style
    }

    /// Whether this console is dressed: only a dressed console asks tmux for its pane.
    #[must_use]
    pub fn dressed(&self) -> bool {
        self.style.0.is_some()
    }

    /// The pane width, in cells, the rows printed from now on are wrapped to.
    pub fn set_width(&mut self, width: Option<usize>) {
        self.width = width;
    }

    /// The first line of the console, drawn in this console's style.
    #[must_use]
    pub fn headline(&self, session: &str, seats: &[Seat]) -> String {
        self.style.dim(&header(session, seats))
    }

    /// `text`, which one of this console's own prints produced, as the effect
    /// that shows it. A dressed console's text carries its own escapes, so it
    /// travels as [`Styled`] and is written as it is; a plain one stays inert
    /// lane text.
    pub(super) fn lane(&self, text: String) -> Effect {
        match self.style {
            Style(None) => Effect::Lane(text),
            Style(Some(_)) => Effect::Styled(Styled(text)),
        }
    }

    /// Keep an outcome until its ask row prints, or two readable passes have
    /// found no row. Repeated submission ids never queue another outcome.
    pub fn outcome(&mut self, id: &str, line: String) {
        if self.outcomes.insert(id.to_owned()) {
            self.pending.push(Pending {
                id: id.to_owned(),
                line,
                passes: 2,
            });
        }
    }

    /// Flush held outcomes when this follow cannot continue. Their ids stay
    /// remembered, so a late row or another queue attempt cannot repeat them.
    pub fn flush_outcomes(&mut self) -> String {
        let mut out = String::new();
        for pending in std::mem::take(&mut self.pending) {
            let _ = writeln!(out, "{}", self.style.status(&pending.line));
        }
        out
    }

    /// Print what `lane` holds that this console has not: `board_gaps` is how
    /// many leading coverage rows are the board's own — its follow already
    /// names each once, so they always print — and the rest are named once.
    pub fn step(&mut self, lane: &Lane, board_gaps: usize, settled: bool) -> String {
        let mut out = String::new();
        for (index, gap) in lane.coverage.iter().enumerate() {
            if index < board_gaps || self.gaps.insert(gap.clone()) {
                out.push_str(&self.style.dim(&format!("coverage incomplete: {gap}\n")));
            }
        }
        let (mut in_lane, mut standing) = (BTreeMap::new(), BTreeMap::new());
        for item in &lane.items {
            let key = match &item.record {
                Some(record) => format!("{CONSOLE_KEY}{record}"),
                None => format!("{}|{:?}|{}", item.micros, item.kind, item.body),
            };
            let nth: &mut usize = in_lane.entry(key.clone()).or_default();
            *nth += 1;
            if matches!(item.kind, Kind::Card { .. } | Kind::NeedsYou { .. }) {
                standing.insert(key.clone(), tag(&item.kind));
            }
            let printed = self.shown.entry(key).or_default();
            if *nth > *printed {
                *printed += 1;
                self.row(item, &mut out);
            }
        }
        // Only a lane whose journal was READ can say a card stopped standing; a
        // card that closes and stands again is news, so it is forgotten as printed.
        if settled {
            for mut pending in std::mem::take(&mut self.pending) {
                pending.passes -= 1;
                if pending.passes == 0 {
                    let _ = writeln!(out, "{}", self.style.status(&pending.line));
                } else {
                    self.pending.push(pending);
                }
            }
            for (key, label) in &self.open {
                if !standing.contains_key(key) {
                    let closed = format!("-- closed: {label} is no longer standing\n\n");
                    out.push_str(&self.style.dim(&closed));
                    self.shown.remove(key);
                }
            }
            self.open = standing;
        }
        out
    }

    /// The "needs you" section of `read`, for a pane of `size` (`None`: a pipe
    /// or a file), as of `now`: what this console has not printed of it yet.
    /// A changed ACTIONABLE set reprints the section; a newly doubted cause
    /// prints one dim notice; recovery, new stamps and proof words print
    /// nothing. A wall-clock gap past [`RESUME_GAP_SECS`] marks a host resume
    /// and gives the watchdog [`GRACE_SECS`] of awake time before its
    /// silence is doubt again — off and unreadable beats are never graced.
    #[must_use]
    pub fn needs(
        &mut self,
        read: &Result<Section, String>,
        size: Option<Size>,
        now: Timestamp,
    ) -> String {
        let width = size.map(|size| size.width);
        let epoch = now.epoch();
        if self
            .needs_last
            .is_some_and(|last| epoch.saturating_sub(last) > RESUME_GAP_SECS)
        {
            self.needs_grace_until = Some(epoch.saturating_add(GRACE_SECS));
        }
        self.needs_last = Some(epoch);
        let section = match read {
            Err(why) => {
                if self.needs_gap.as_deref() == Some(why.as_str()) {
                    return String::new();
                }
                self.needs_gap = Some(why.clone());
                let line = format!("-- needs you: {why}: rows shown earlier stand, none cleared");
                return self.needs_line(&line, width, |palette| palette.dim);
            }
            Ok(section) => section,
        };
        let gap = self.needs_gap.take();
        let grace = self.needs_grace_until.is_some_and(|until| epoch < until);
        let owned;
        let rows: &[Row] = if grace {
            owned = graced(&section.rows);
            &owned
        } else {
            &section.rows
        };
        let key = need_key(rows);
        let doubt = doubt_set(rows);
        let cold = self.needs.is_none() && gap.is_none() && rows.is_empty();
        let same = self.needs.as_deref() == Some(key.as_str());
        let mut gained: Vec<&str> = doubt.difference(&self.needs_doubt).copied().collect();
        if cold || (same && gained.is_empty()) {
            self.needs = Some(key);
            self.needs_doubt = doubt;
            return String::new();
        }
        self.needs = Some(key);
        self.needs_doubt = doubt;
        if same {
            gained.sort_unstable();
            return self.notice(rows, &gained, width);
        }
        let (_, time) = clock_text(self.style.shift(micros_of(now)));
        let as_of = format!("as of {time}");
        if rows.is_empty() {
            let line = format!("-- needs you: nothing standing ({as_of})");
            return self.needs_line(&line, width, |palette| palette.dim);
        }
        let entries = entries(rows, &self.style, now);
        let budget = size.map_or(NEEDS_LINES, |size| (size.height / 3).max(1));
        let lines: usize = entries.iter().map(|entry| entry.lines.len()).sum();
        // The rows under the header go to the unverified causes first, then to
        // the reason rows in rank order, a row short of room keeping its seat
        // line before its detail.
        let mut room = if lines >= budget { budget - 1 } else { lines };
        let mut kept = vec![0; entries.len()];
        for verified in [false, true] {
            for (entry, keep) in entries.iter().zip(&mut kept) {
                if entry.verified == verified {
                    *keep = entry.lines.len().min(room);
                    room -= *keep;
                }
            }
        }
        let hidden: usize = entries
            .iter()
            .zip(&kept)
            .filter(|(_, keep)| **keep == 0)
            .map(|(entry, _)| entry.seats)
            .sum();
        let actionable = rows
            .iter()
            .filter(|row| matches!(row.verdict, Verdict::Reason(_)))
            .count();
        let mut head = format!("-- needs you: {}", seats(actionable));
        if hidden > 0 {
            let _ = write!(head, " · {hidden} more: ae list");
        }
        let _ = write!(head, " · {as_of}");
        let mut out = self.needs_line(&head, width, |palette| palette.dim);
        for (entry, keep) in entries.iter().zip(kept) {
            for line in &entry.lines[..keep] {
                out.push_str(&self.needs_line(line, width, |palette| {
                    if entry.verified {
                        palette.needs_you
                    } else {
                        palette.dim
                    }
                }));
            }
        }
        out
    }

    /// One dim notice for newly doubted `gained` classes: what is doubted
    /// and which seats carry it — never the action section again.
    fn notice(&self, rows: &[Row], gained: &[&str], width: Option<usize>) -> String {
        let names: Vec<&str> = rows
            .iter()
            .filter(|row| doubt_classes(row).any(|class| gained.contains(&class)))
            .map(|row| row.seat.name.as_str())
            .collect();
        let kinds = gained
            .iter()
            .map(|class| cause_words(class))
            .collect::<Vec<_>>()
            .join(", ");
        let line = format!(
            "  stale: {kinds} · {}: {}",
            seats(names.len()),
            names.join(", ")
        );
        self.needs_line(&line, width, |palette| palette.dim)
    }

    /// One section line: neutralised, flattened to one row of `width` cells
    /// when it is known, painted in `hue`, with its newline.
    fn needs_line(
        &self,
        line: &str,
        width: Option<usize>,
        hue: impl Fn(&Palette) -> &'static str,
    ) -> String {
        let text = terminal_text(&clip(line, usize::MAX));
        let text = width.map_or_else(|| text.clone(), |width| clip(&text, width));
        format!("{}\n", self.style.paint(hue, false, &text))
    }

    /// Forget the printed console-thread rows, whose positions a rewritten journal
    /// (`before` records, `now` now) no longer names; every other memory stays.
    pub fn rebase(&mut self, before: usize, now: usize) -> String {
        self.shown.retain(|key, _| !key.starts_with(CONSOLE_KEY));
        self.style.dim(&format!(
            "-- journal rewritten ({before} records before, {now} now): the chat thread is shown again as the journal stands\n\n"
        ))
    }

    fn row(&mut self, item: &Item, out: &mut String) {
        let (day, time) = clock_text(self.style.shift(item.micros));
        if self.day.as_deref() != Some(day.as_str()) {
            out.push_str(&self.style.day(&day));
            self.day = Some(day);
        }
        out.push_str(&self.style.head_in(&item.kind, &time, self.width));
        for line in item.body.lines() {
            out.push_str(&self.style.line_in(&item.kind, line, self.width));
        }
        if let Kind::Asked { id, .. } | Kind::NotDelivered { id, .. } = &item.kind
            && let Some(at) = self.pending.iter().position(|pending| pending.id == *id)
        {
            let pending = self.pending.remove(at);
            out.push_str(&self.style.under_in(&item.kind, &pending.line, self.width));
        }
        out.push('\n');
    }
}

/// The most lines the section takes where no pane height bounds it.
const NEEDS_LINES: usize = 12;

/// A wall-clock gap past this between section reads marks a host resume:
/// twelve missed polls, so the chat itself did not run either.
const RESUME_GAP_SECS: i64 = 60;

/// Awake time a resumed host gives the watchdog before its silence is doubt
/// again: three default verdict intervals, the staleness bound itself.
const GRACE_SECS: i64 = super::needs::STALE_SECS;

/// Rows with a resumed host's expected silence lifted: `WatchdogStale` rows
/// drop and stale flags lift. Off, unreadable and every other cause stands.
fn graced(rows: &[Row]) -> Vec<Row> {
    rows.iter()
        .filter_map(|row| match row.verdict {
            Verdict::Unknown(Cause::WatchdogStale { .. }) => None,
            Verdict::Unknown(_) => Some(row.clone()),
            Verdict::Reason(_) => Some(Row {
                stale: row
                    .stale
                    .filter(|stale| !matches!(stale, Cause::WatchdogStale { .. })),
                ..row.clone()
            }),
        })
        .collect()
}

/// The actionable set: one reason row's seat, verdict and source, sorted so
/// age order never replays. Stamps, words, record positions and doubt excluded.
fn need_key(rows: &[Row]) -> String {
    let mut parts: Vec<String> = rows
        .iter()
        .filter_map(|row| match row.verdict {
            Verdict::Reason(reason) => Some(format!(
                "{}\0{}\0{}\0{}",
                row.seat.slot,
                row.seat.name,
                reason as u8,
                source_token(&row.source)
            )),
            Verdict::Unknown(_) => None,
        })
        .collect();
    parts.sort_unstable();
    parts.join("\n")
}

/// A reason row's source, by kind and alert action.
fn source_token(source: &Source) -> String {
    match source {
        Source::Alert { action } => format!("alert:{action}"),
        Source::Declaration => "declared".to_owned(),
        Source::NoPane => "no-pane".to_owned(),
        Source::Unattributed => "unattributed".to_owned(),
        Source::Unverified => "unverified".to_owned(),
    }
}

/// Every doubt class `rows` carries, stale flags and unknown causes alike.
fn doubt_set(rows: &[Row]) -> BTreeSet<&'static str> {
    rows.iter().flat_map(doubt_classes).collect()
}

/// The doubt classes one row carries: none, its stale flag's, its cause's.
fn doubt_classes(row: &Row) -> impl Iterator<Item = &'static str> {
    match row.verdict {
        Verdict::Reason(_) => row.stale.map(cause_class),
        Verdict::Unknown(cause) => Some(cause_class(cause)),
    }
    .into_iter()
}

/// One cause's class: the variant, never its stamps or payload counts.
fn cause_class(cause: Cause) -> &'static str {
    match cause {
        Cause::WatchdogOff => "watchdog-off",
        Cause::WatchdogUnreadable => "watchdog-unreadable",
        Cause::WatchdogStale { .. } => "watchdog-stale",
        Cause::RuntimeUnread => "runtime-unread",
        Cause::PaneUnproven => "pane-unproven",
        Cause::JournalPartial { .. } => "journal-partial",
    }
}

/// One doubt class in words, without stamps or counts.
fn cause_words(class: &str) -> &'static str {
    match class {
        "watchdog-off" => "watchdog off",
        "watchdog-unreadable" => "watchdog beat unreadable",
        "watchdog-stale" => "watchdog silent",
        "runtime-unread" => "tmux did not list the panes",
        "pane-unproven" => "pane unproven",
        _ => "journal partial",
    }
}

/// One entry of the section: a seat's row, its record's words on a line of
/// their own, or one cause's unverified seats.
struct Entry {
    lines: Vec<String>,
    /// How many seats the line stands for.
    seats: usize,
    /// A reason row, rather than a gap no verdict could be read for.
    verified: bool,
}

/// The section's lines, rows first in the fold's order, then ONE line per
/// cause no verdict could be trusted for, in the order the causes first appear.
fn entries(rows: &[Row], style: &Style, now: Timestamp) -> Vec<Entry> {
    let mut lines = Vec::new();
    let mut gaps: Vec<(Cause, Vec<&str>)> = Vec::new();
    for row in rows {
        match row.verdict {
            Verdict::Reason(reason) => lines.push(Entry {
                lines: reason_lines(row, reason, style, now),
                seats: 1,
                verified: true,
            }),
            Verdict::Unknown(cause) => match gaps.iter_mut().find(|(seen, _)| *seen == cause) {
                Some((_, names)) => names.push(&row.seat.name),
                None => gaps.push((cause, vec![&row.seat.name])),
            },
        }
    }
    for (cause, names) in gaps {
        lines.push(Entry {
            lines: vec![format!(
                "  unverified: {} · {}: {}",
                cause_text(cause, style),
                seats(names.len()),
                names.join(", ")
            )],
            seats: names.len(),
            verified: false,
        });
    }
    lines
}

/// A seat's reason row: who, what, how to open it, from which evidence since
/// when and how fresh that evidence is — the order a narrow pane cuts it in —
/// then the record's own words on their own line.
fn reason_lines(
    row: &Row,
    reason: crate::attention::Reason,
    style: &Style,
    now: Timestamp,
) -> Vec<String> {
    let mut text = format!("  {}", row.seat.name);
    if row.lead_pair {
        text.push_str(" (lead pair)");
    }
    let _ = write!(text, " · {reason} · /open {} · ", row.seat.name);
    match &row.source {
        Source::Alert { action } => text.push_str(&action.replace('-', " ")),
        Source::Declaration => text.push_str("declared"),
        Source::NoPane => {
            let _ = write!(text, "no pane carries {}", row.seat.slot);
        }
        Source::Unattributed | Source::Unverified => text.push_str("source unattributed"),
    }
    if let Some(since) = row.since_micros {
        let (_, time) = clock_text(style.shift(since));
        let age = crate::brief::age(Some(now.epoch() - since.div_euclid(1_000_000)));
        let _ = write!(text, " since {time} ({age})");
    }
    if let Some(stale) = row.stale {
        let _ = write!(text, " · stale: {}", cause_text(stale, style));
    }
    let mut lines = vec![text];
    if !row.detail.is_empty() {
        lines.push(format!("    {}", row.detail));
    }
    lines
}

/// Why a verdict cannot be trusted, in words.
fn cause_text(cause: Cause, style: &Style) -> String {
    match cause {
        Cause::WatchdogOff => "watchdog off".to_owned(),
        Cause::WatchdogUnreadable => "watchdog beat unreadable".to_owned(),
        Cause::WatchdogStale { last_micros } => {
            let (_, time) = clock_text(style.shift(last_micros));
            format!("watchdog silent since {time}")
        }
        Cause::RuntimeUnread => "tmux did not list the panes".to_owned(),
        Cause::PaneUnproven => "pane unproven".to_owned(),
        Cause::JournalPartial { skipped } => format!("journal partial, {skipped} lines unread"),
    }
}

/// `n seats`, singular for one.
fn seats(n: usize) -> String {
    if n == 1 {
        "1 seat".to_owned()
    } else {
        format!("{n} seats")
    }
}

/// `now` on the lane's clock.
fn micros_of(now: Timestamp) -> i64 {
    now.epoch().saturating_mul(1_000_000)
}

#[cfg(test)]
mod tests {
    use super::{Printed, Style, Zone, header};
    use crate::attention::Reason;
    use crate::console::input::Size;
    use crate::console::lane::{Item, Kind, Lane, Seat};
    use crate::console::needs::{Cause, Row, SeatRef, Section, Source, Verdict};
    use crate::theme::{Look, Palette};
    use crate::time::Timestamp;

    const T: i64 = 1_790_748_060_000_000;
    const UNDER_TEXT_BAR: [&str; 6] = [
        "darcula working",
        "darcula stale",
        "darcula dim",
        "darcula done",
        "a dim",
        "b dim",
    ];

    fn item(kind: Kind, micros: i64, body: &str) -> Item {
        Item {
            micros,
            kind,
            body: body.to_owned(),
            record: None,
        }
    }

    fn said(who: &str) -> Kind {
        Kind::Said {
            who: who.to_owned(),
        }
    }

    fn card(seat: &str) -> Kind {
        Kind::Card {
            seat: seat.to_owned(),
        }
    }

    fn lane(items: Vec<Item>, coverage: &[&str]) -> Lane {
        Lane {
            items,
            coverage: coverage.iter().map(|gap| (*gap).to_owned()).collect(),
        }
    }

    #[test]
    fn every_kind_wears_its_tag_and_a_body_keeps_its_tabs_and_indentation() {
        let kinds = [
            (
                Kind::Pane {
                    seat: "lead".to_owned(),
                    generation: 1,
                },
                "lead pane (transcript) · prior 1",
            ),
            (
                Kind::Assistant {
                    seat: "colead".to_owned(),
                    generation: 0,
                },
                "colead assistant (transcript)",
            ),
            (
                Kind::Inbound {
                    from: "telegram:42".to_owned(),
                    to: "lead".to_owned(),
                },
                "telegram:42 → lead",
            ),
            (
                Kind::Reply {
                    from: "lead".to_owned(),
                    to: "telegram:42".to_owned(),
                },
                "lead → telegram:42 · preview (600-char summary)",
            ),
            (said("ae"), "said ae"),
            (card("lead"), "DECISION lead · waiting-user"),
            (
                Kind::NeedsYou {
                    seat: "colead".to_owned(),
                },
                "NEEDS YOU colead · needs you in its pane",
            ),
        ];
        for (kind, tag) in kinds {
            let mut printed = Printed::default();
            let got = printed.step(
                &lane(vec![item(kind, T, "a\n\tb\n  fn c() {}")], &[]),
                0,
                true,
            );
            assert_eq!(
                got,
                format!("# 2026-09-30 UTC\n## 06:01:00 {tag}\n  a\n  \tb\n    fn c() {{}}\n\n")
            );
        }
    }

    #[test]
    fn a_hostile_byte_in_any_field_is_inert_and_only_whitespace_survives() {
        let hostile = "x\u{1b}]52;c;AAAA\u{7}y\rz\u{9b}w";
        let kind = Kind::Inbound {
            from: format!("telegram:{hostile}"),
            to: format!("lead{hostile}"),
        };
        let gap = format!("aedev:lead — {hostile}");
        let mut printed = Printed::default();
        let got = printed.step(
            &lane(vec![item(kind, T, hostile)], &[gap.as_str()]),
            1,
            true,
        );
        assert!(
            !got.chars().any(|ch| ch.is_control() && ch != '\n'),
            "{got:?}"
        );
        assert!(
            got.contains("x\u{fffd}]52;c;AAAA\u{fffd}y\u{fffd}z\u{fffd}w"),
            "{got}"
        );
        let head = header(
            &format!("s{hostile}"),
            &[Seat {
                slot: "main".to_owned(),
                name: format!("l{hostile}"),
                profile: None,
            }],
        );
        assert!(!head.trim_end().chars().any(char::is_control), "{head:?}");
    }

    #[test]
    fn a_follow_tick_prints_only_what_is_new_and_equal_records_are_two_rows() {
        let mut printed = Printed::default();
        let one = lane(
            vec![item(said("lead"), T, "same")],
            &["say — 1 line from other seats not shown"],
        );
        assert!(
            printed
                .step(&one, 0, true)
                .contains("## 06:01:00 said lead")
        );
        assert_eq!(
            printed.step(&one, 0, true),
            "",
            "nothing new, nothing printed"
        );
        let two = lane(
            vec![
                item(said("lead"), T, "same"),
                item(said("lead"), T, "same"),
                item(said("lead"), T + 5_000_000, "later"),
            ],
            &["say — 1 line from other seats not shown"],
        );
        let got = printed.step(&two, 0, true);
        assert_eq!(got.matches("said lead").count(), 2, "{got}");
        assert!(got.contains("06:01:05 said lead\n  later"), "{got}");
        assert!(!got.contains("coverage"), "a named gap prints once: {got}");
    }

    #[test]
    fn the_boards_own_gaps_print_on_every_tick_and_a_lane_gap_only_once() {
        let mut printed = Printed::default();
        let both = lane(vec![], &["board gap", "lane gap"]);
        assert_eq!(
            printed.step(&both, 1, true),
            "coverage incomplete: board gap\ncoverage incomplete: lane gap\n"
        );
        assert_eq!(
            printed.step(&both, 1, true),
            "coverage incomplete: board gap\n",
            "only the board's own gap comes back"
        );
    }

    #[test]
    fn a_card_that_stops_standing_is_closed_out_loud_and_a_newer_one_prints() {
        let mut printed = Printed::default();
        let asks = lane(vec![item(card("lead"), T, "ship v2?")], &[]);
        assert!(printed.step(&asks, 0, true).contains("DECISION lead"));
        assert_eq!(printed.step(&asks, 0, true), "");
        let answered = lane(vec![], &[]);
        assert_eq!(
            printed.step(&answered, 0, true),
            "-- closed: DECISION lead · waiting-user is no longer standing\n\n"
        );
        assert_eq!(printed.step(&answered, 0, true), "", "closed once");
        let again = lane(vec![item(card("lead"), T + 60_000_000, "and now?")], &[]);
        assert!(
            printed
                .step(&again, 0, true)
                .contains("## 06:02:00 DECISION lead")
        );
    }

    #[test]
    fn an_unread_journal_closes_nothing_and_a_standing_item_that_closed_and_returns_prints_again() {
        let needs = Kind::NeedsYou {
            seat: "lead".to_owned(),
        };
        for (kind, tag) in [(card("lead"), "DECISION lead"), (needs, "NEEDS YOU lead")] {
            let (mut printed, empty) = (Printed::default(), lane(vec![], &[]));
            let asks = lane(vec![item(kind, T, "ship it?")], &[]);
            assert!(printed.step(&asks, 0, true).contains(tag));
            assert_eq!(printed.step(&empty, 0, false), "", "unread proves nothing");
            assert_eq!(
                printed.step(&asks, 0, true),
                "",
                "still standing, still printed"
            );
            assert!(printed.step(&empty, 0, true).contains("-- closed"));
            assert!(printed.step(&asks, 0, true).contains(tag), "back, so news");
        }
    }

    #[test]
    fn held_outcomes_age_only_on_readable_passes_and_flush_in_submission_order() {
        let mut printed = Printed::default();
        let empty = lane(vec![], &[]);
        printed.outcome("z", "sent z".to_owned());
        printed.outcome("a", "uncertain a".to_owned());
        for _ in 0..5 {
            assert_eq!(printed.step(&empty, 0, false), "");
        }
        assert_eq!(
            printed.step(&empty, 0, true),
            "",
            "first readable pass holds"
        );
        assert_eq!(printed.step(&empty, 0, false), "", "gap does not age");
        assert_eq!(printed.step(&empty, 0, true), "sent z\nuncertain a\n");
        printed.outcome("z", "sent z again".to_owned());
        assert_eq!(printed.step(&empty, 0, true), "", "one outcome per id");
    }

    #[test]
    fn held_outcomes_attach_to_their_own_rows_and_never_repeat_after_rebase() {
        for kind in [
            Kind::Asked {
                to: "lead".to_owned(),
                id: "id".to_owned(),
                uncertain: false,
            },
            Kind::NotDelivered {
                to: "lead".to_owned(),
                id: "id".to_owned(),
            },
        ] {
            let mut printed = Printed::default();
            printed.outcome("id", "outcome id".to_owned());
            let mut ask = item(kind, T, "question");
            ask.record = Some(0);
            let ask = lane(vec![ask], &[]);
            let text = printed.step(&ask, 0, true);
            assert!(text.contains("  question\n  outcome id\n\n"));
            assert_eq!(printed.step(&ask, 0, true), "");
            let _ = printed.rebase(1, 1);
            printed.outcome("id", "outcome id".to_owned());
            let again = printed.step(&ask, 0, true);
            assert!(again.contains("question"));
            assert!(!again.contains("outcome id"));
        }
    }

    fn dressed(look: &str, palette: &str) -> Style {
        let look = Look::read(look, palette, "", "");
        Style::resolve(true, Some(look), Some("+0200"), "lead")
    }

    fn asked(kind: &str) -> Kind {
        let (to, id) = ("lead".to_owned(), "id".to_owned());
        match kind {
            "asked" => Kind::Asked {
                to,
                id,
                uncertain: false,
            },
            "closed" => Kind::Closed { id },
            _ => Kind::NotDelivered { to, id },
        }
    }

    /// Every escape in `text` as the SGR parameters between `ESC [` and `m`,
    /// failing on any escape that is not one.
    fn escapes(text: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut rest = text;
        while let Some(at) = rest.find('\x1b') {
            let after = &rest[at + 1..];
            let end = after.find('m').expect("a terminated escape");
            let params = after[..end].strip_prefix('[').expect("only CSI");
            assert!(
                params.bytes().all(|b| b.is_ascii_digit() || b == b';'),
                "{params:?}"
            );
            found.push(params.to_owned());
            rest = &after[end + 1..];
        }
        found
    }

    #[test]
    fn a_zone_is_kept_only_as_a_real_signed_hour_and_minute() {
        for (answer, secs) in [
            ("+0200", 7200),
            ("-0330", -12_600),
            ("+0000", 0),
            ("+2359", 86_340),
        ] {
            assert_eq!(
                Zone::parse(answer).map(|zone| zone.secs),
                Some(secs),
                "{answer}"
            );
        }
        let bad = [
            "",
            "UTC",
            "0200",
            "+200",
            "+02000",
            "+2400",
            "+0260",
            "+0200\x1b[35m",
            "+02:00",
            "+٠٢٠٠",
            " +0200",
        ];
        for answer in bad {
            assert_eq!(Zone::parse(answer), None, "{answer:?}");
        }
    }

    #[test]
    fn only_a_terminal_whose_look_is_drawn_is_dressed_and_the_look_picks_bar_and_hue() {
        let off = Look::read("", "", "off", "");
        assert_eq!(
            Style::resolve(false, None, Some("+0200"), "lead"),
            Style::PLAIN
        );
        assert_eq!(
            Style::resolve(true, Some(off), Some("+0200"), "lead"),
            Style::PLAIN
        );
        assert!(
            Style::wanted(true, None),
            "a look nobody could read is the default"
        );
        let kind = Kind::Said {
            who: "lead".to_owned(),
        };
        let ascii = dressed("off", "a").head(&kind, "01:00:00");
        assert!(
            ascii.starts_with("\x1b[38;2;") && ascii.contains("m|\x1b[0m "),
            "{ascii:?}"
        );
        assert!(
            dressed("on", "a")
                .head(&kind, "01:00:00")
                .contains("m▌\x1b[0m ")
        );
        assert_ne!(
            dressed("on", "").head(&kind, "t"),
            dressed("on", "a").head(&kind, "t")
        );
    }

    #[test]
    fn a_plain_style_writes_exactly_the_bytes_a_console_always_did() {
        let (plain, kind) = (Style::PLAIN, asked("asked"));
        assert_eq!(plain.day("2026-09-30"), "# 2026-09-30 UTC\n");
        assert_eq!(plain.head(&kind, "06:01:00"), "## 06:01:00 you → lead\n");
        assert_eq!(plain.line(&kind, "a\x1b[2J"), "  a\u{fffd}[2J\n");
        assert_eq!(plain.under(&kind, "sent id"), "  sent id\n");
        assert_eq!(plain.shift(5), 5);
        for text in ["-- closed: x\n\n", "coverage incomplete: y\n"] {
            assert_eq!(plain.dim(text), text);
        }
        assert_eq!(
            (plain.status("sent id"), plain.human("to lead> ")),
            ("sent id".to_owned(), "to lead> ".to_owned())
        );
    }

    #[test]
    fn a_dressed_row_wears_a_bar_a_dim_stamp_and_its_speakers_hue_and_only_you_is_bold() {
        let style = dressed("on", "");
        let p = Palette::DARCULA;
        let rgb = |hex: &str| {
            let at = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex");
            format!("\x1b[38;2;{};{};{}m", at(1), at(3), at(5))
        };
        let said = |who: &str| {
            style.head(
                &Kind::Said {
                    who: who.to_owned(),
                },
                "t",
            )
        };
        assert_eq!(
            said("lead"),
            format!(
                "{}▌\x1b[0m {}t\x1b[0m {}said lead\x1b[0m\n",
                rgb(p.working),
                rgb(p.dim),
                rgb(p.working)
            )
        );
        assert!(said("colead").contains(&rgb(p.stale)), "{}", said("colead"));
        assert!(said("ae").ends_with(&format!("{}said ae\x1b[0m\n", rgb(p.dim))));
        let you = style.head(&asked("asked"), "t");
        assert!(
            you.contains(&format!("\x1b[1m{}you → lead", rgb(p.title))),
            "{you:?}"
        );
        for kind in [
            Kind::Pane {
                seat: "lead".to_owned(),
                generation: 0,
            },
            Kind::Card {
                seat: "lead".to_owned(),
            },
        ] {
            assert!(!style.head(&kind, "t").contains("\x1b[1m"), "{kind:?}");
        }
        assert!(
            style
                .line(&asked("closed"), "body")
                .starts_with(&format!("{}▌\x1b[0m  body", rgb(p.title)))
        );
    }

    #[test]
    fn status_lines_wear_the_hue_of_what_they_say() {
        let (style, p) = (dressed("on", ""), Palette::DARCULA);
        for (line, hue) in [
            ("sent id", p.done),
            ("sent id; kept", p.done),
            ("uncertain id: check", p.waiting_agent),
            ("no record of id; maybe", p.waiting_agent),
            ("not delivered id", p.dead),
            ("refused: no", p.dead),
            ("closed id", p.dim),
            ("accepting input", p.dim),
            ("sentinel", p.dim),
        ] {
            let got = style.status(line);
            let at = |i: usize| u8::from_str_radix(&hue[i..i + 2], 16).expect("hex");
            let want = format!("\x1b[38;2;{};{};{}m{line}\x1b[0m", at(1), at(3), at(5));
            assert_eq!(got, want, "{line}");
        }
        assert!(
            style
                .under(&asked("asked"), "sent id")
                .ends_with("sent id\x1b[0m\n")
        );
    }

    #[test]
    fn a_record_can_reach_the_terminal_only_inside_text_ae_neutralised_before_it_wrapped() {
        let hostile = "x\u{1b}]52;c;AAAA\u{7}y\rz\u{9b}w\u{1b}[31m";
        let style = dressed("on", "b");
        let kind = Kind::Inbound {
            from: hostile.to_owned(),
            to: hostile.to_owned(),
        };
        let seat = Kind::Said {
            who: hostile.to_owned(),
        };
        for text in [
            style.head(&kind, hostile),
            style.head(&seat, "t"),
            style.line(&kind, hostile),
            style.under(&kind, hostile),
            style.status(hostile),
            style.dim(hostile),
            style.human(hostile),
            style.day(hostile),
        ] {
            let found = escapes(&text);
            assert!(
                found
                    .iter()
                    .all(|p| p == "0" || p == "1" || p.starts_with("38;2;")),
                "{found:?}"
            );
            assert!(
                !text.contains('\u{9b}') && !text.contains('\r') && !text.contains('\u{7}'),
                "{text:?}"
            );
        }
        let line = style.line(&kind, hostile);
        assert!(
            line.contains("x\u{fffd}]52;c;AAAA\u{fffd}y\u{fffd}z\u{fffd}w\u{fffd}[31m"),
            "{line:?}"
        );
    }

    #[test]
    fn the_viewers_zone_shifts_the_clock_and_names_itself_on_the_divider() {
        let (utc, local) = (Style::PLAIN, dressed("on", ""));
        assert_eq!(local.shift(T), T + 7_200_000_000);
        assert_eq!(utc.shift(T), T);
        assert!(
            local
                .day("2026-10-03")
                .contains("# 2026-10-03 +0200\x1b[0m\n")
        );
        let unzoned = Style::resolve(true, None, Some("bogus"), "lead");
        assert_eq!(unzoned.shift(T), T);
        assert!(unzoned.day("2026-10-02").contains("# 2026-10-02 UTC"));
        assert_eq!(
            Style::resolve(true, None, Some("-0100"), "lead").shift(T),
            T - 3_600_000_000
        );
    }

    /// WCAG 2.1 contrast of two six-digit hex colours.
    fn contrast(one: &str, other: &str) -> f64 {
        let luminance = |hex: &str| {
            let channel = |at: usize| {
                let raw = f64::from(u8::from_str_radix(&hex[at..at + 2], 16).expect("hex")) / 255.0;
                if raw <= 0.03928 {
                    raw / 12.92
                } else {
                    ((raw + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5)
        };
        let (a, b) = (luminance(one), luminance(other));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// The chat draws its hues on the TERMINAL's own background, which ae does
    /// not know, so the proxy is each palette's `base`. Every hue clears the
    /// 3.0:1 bar of a mark (the bar, and a bold or large run of text); the roles
    /// under the 4.5:1 text bar are named here, so a palette edit that moves one
    /// is a decision and not a surprise.
    #[test]
    fn every_chat_hue_clears_the_mark_bar_on_the_palette_base_and_the_text_residual_is_named() {
        let mut under_text = Vec::new();
        for p in [Palette::DARCULA, Palette::NEUTRAL, Palette::WARM] {
            for (role, hue) in [
                ("title", p.title),
                ("working", p.working),
                ("stale", p.stale),
                ("dim", p.dim),
                ("done", p.done),
                ("waiting_agent", p.waiting_agent),
                ("dead", p.dead),
            ] {
                let ratio = contrast(hue, p.base);
                assert!(ratio >= 3.0, "{} {role} is {ratio:.2}:1 on base", p.name);
                if ratio < 4.5 {
                    under_text.push(format!("{} {role}", p.name));
                }
            }
        }
        assert_eq!(under_text, UNDER_TEXT_BAR);
    }

    fn unstyled(text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while let Some(at) = rest.find('\x1b') {
            out.push_str(&rest[..at]);
            let after = &rest[at..];
            rest = &after[after.find('m').expect("a terminated escape") + 1..];
        }
        out + rest
    }

    #[test]
    fn a_dressed_console_bars_every_line_dims_its_notices_and_routes_its_text_by_style() {
        use crate::console::input::Effect;
        let mut printed = Printed::styled(dressed("on", ""));
        printed.outcome("id", "sent id".to_owned());
        let kind = Kind::Asked {
            to: "lead".to_owned(),
            id: "id".to_owned(),
            uncertain: false,
        };
        let got = printed.step(&lane(vec![item(kind, T, "q\n  r")], &["gap"]), 0, true);
        let want = "coverage incomplete: gap\n# 2026-09-30 +0200\n▌ 08:01:00 you → lead\n▌  q\n▌    r\n▌  sent id\n\n";
        assert_eq!(unstyled(&got), want);
        let p = Palette::DARCULA;
        assert!(
            got.starts_with(&format!("{}coverage incomplete: gap\x1b[0m\n", sgr(p.dim))),
            "{got:?}"
        );
        assert!(
            got.contains(&format!("{}sent id\x1b[0m\n\n", sgr(p.done))),
            "{got:?}"
        );
        let seats = [Seat {
            slot: "main".to_owned(),
            name: "lead".to_owned(),
            profile: None,
        }];
        assert_eq!(
            unstyled(&printed.headline("s", &seats)),
            header("s", &seats)
        );
        assert_eq!(
            Printed::default().headline("s", &seats),
            header("s", &seats)
        );
        assert!(
            matches!(Printed::default().lane("x".to_owned()), Effect::Lane(text) if text == "x")
        );
        assert!(
            matches!(printed.lane("x".to_owned()), Effect::Styled(text) if text.as_str() == "x")
        );
        let closed = printed.step(&lane(vec![], &[]), 0, true);
        assert_eq!(
            closed, "",
            "an ask is not a card, so nothing stood to close"
        );
        let rebased = printed.rebase(2, 1);
        assert!(
            rebased.starts_with(&sgr(p.dim)) && rebased.ends_with("stands\x1b[0m\n\n"),
            "{rebased:?}"
        );
    }

    #[test]
    fn chat_wrap_spec_request_ids_are_fields_and_never_header_text() {
        let id = "ae-20261003T070000Z-12345678";
        let ask = Kind::Asked {
            to: "lead".to_owned(),
            id: id.to_owned(),
            uncertain: false,
        };
        assert_eq!(
            Style::PLAIN.head(&ask, "06:01:00"),
            "## 06:01:00 you → lead\n"
        );
        assert!(!dressed("on", "darcula").head(&ask, "06:01:00").contains(id));
        assert!(matches!(ask, Kind::Asked { id: stored, .. } if stored == id));
    }

    fn sgr(hex: &str) -> String {
        let at = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex");
        format!("\x1b[38;2;{};{};{}m", at(1), at(3), at(5))
    }
    fn seat_row(name: &str, slot: &str, verdict: Verdict, source: Source, detail: &str) -> Row {
        Row {
            seat: SeatRef {
                slot: slot.to_owned(),
                name: name.to_owned(),
            },
            lead_pair: slot == "main",
            verdict,
            source,
            stale: None,
            since_micros: Some(T),
            detail: detail.to_owned(),
            record: Some(0),
        }
    }

    fn unverified(name: &str, slot: &str) -> Row {
        let mut row = seat_row(
            name,
            slot,
            Verdict::Unknown(Cause::WatchdogOff),
            Source::Unverified,
            "",
        );
        row.since_micros = None;
        row.record = None;
        row
    }

    #[test]
    fn the_needs_section_prints_one_snapshot_with_its_source_age_and_open_hint() {
        let now = Timestamp::from_epoch(T / 1_000_000 + 600);
        let section = Section {
            rows: vec![
                seat_row(
                    "lead",
                    "main",
                    Verdict::Reason(Reason::WaitingUser),
                    Source::Declaration,
                    "which layout?",
                ),
                seat_row(
                    "scout",
                    "spawned.0",
                    Verdict::Reason(Reason::Blocked),
                    Source::Alert {
                        action: "human-prompt".to_owned(),
                    },
                    "",
                ),
                unverified("a", "spawned.1"),
                unverified("b", "spawned.2"),
            ],
        };
        let mut printed = Printed::default();
        assert_eq!(
            printed.needs(&Ok(section.clone()), None, now),
            "-- needs you: 2 seats · as of 06:11:00\n\
             \x20 lead (lead pair) · waiting-user · /open lead · declared since 06:01:00 (10m)\n\
             \x20   which layout?\n\
             \x20 scout · blocked · /open scout · human prompt since 06:01:00 (10m)\n\
             \x20 unverified: watchdog off · 2 seats: a, b\n"
        );
        let later = Timestamp::from_epoch(now.epoch() + 60);
        assert_eq!(
            printed.needs(&Ok(section), None, later),
            "",
            "age alone is no news"
        );
        assert_eq!(
            printed.needs(&Ok(Section::default()), None, later),
            "-- needs you: nothing standing (as of 06:12:00)\n"
        );
        assert_eq!(
            Printed::default().needs(&Ok(Section::default()), None, now),
            ""
        );
    }

    #[test]
    fn a_small_section_names_its_uncertainty_before_a_known_row() {
        let now = Timestamp::from_epoch(T / 1_000_000);
        let mut rows = vec![seat_row(
            "lead",
            "main",
            Verdict::Reason(Reason::Dead),
            Source::NoPane,
            "",
        )];
        rows.extend((0..3).map(|at| unverified(&format!("w{at}"), &format!("spawned.{at}"))));
        let small = Some(Size {
            width: 80,
            height: 6,
        });
        let text = Printed::default().needs(&Ok(Section { rows }), small, now);
        assert_eq!(text.lines().count(), 2, "{text}");
        assert!(
            text.contains("1 seat · 1 more: ae list")
                && text.contains("unverified: watchdog off · 3 seats: w0, w1, w2"),
            "{text}"
        );
        assert!(!text.contains("no pane carries main"), "{text}");
    }
}
